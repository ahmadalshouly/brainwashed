//! `brainwashed`: the BrainWashed host. It runs models on this computer and
//! serves the web app: chat for everyone, plus admin pages for models, skills,
//! devices, remote access and settings. It opens the admin page when it starts
//! and makes the computer reachable from anywhere through a free Cloudflare
//! tunnel, with no router setup.

mod browser;
mod service;

use brainwashed_core::{
    CatalogItem, ChatEvent, ChatMessage, Engine, EngineConfig, EngineState, Event, InstalledModel,
    RemoteAccess, Role, SamplingOptions,
};
use brainwashed_gateway::{DeviceRole, Gateway, PairingOffer};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

const HELP: &str = "\
BrainWashed runs open source AI models on this computer and lets you and your
team use them from any browser or phone, at home or away.

Usage:
  brainwashed [serve]        Start BrainWashed and open the admin page. Downloads
                             a small model the first time. If it's already
                             running, just opens the admin page.
  brainwashed open           Open the admin page of the running BrainWashed.
  brainwashed status         Show whether BrainWashed is running and its addresses.
  brainwashed stop           Stop the running BrainWashed.
  brainwashed chat           Chat in this terminal.
  brainwashed models         List installed and suggested models.
  brainwashed pull <model>   Download a model: a name from `models`, or any
                             Hugging Face GGUF repo like `Qwen/Qwen3-4B-GGUF`.
  brainwashed use <model>    Load this model from now on.
  brainwashed skills         Show your skills and the folder they live in.
  brainwashed skills search [words]
                             Find skills other people shared.
  brainwashed skills install <name or link>
                             Install a shared skill, or one from a link to a
                             SKILL.md. Shows it to you first.
  brainwashed skills update  Update skills installed from the community.
  brainwashed remote <how>   Choose how devices reach this computer from anywhere:
                               quick                 free Cloudflare address, no
                                                     account (default; it changes
                                                     each time BrainWashed starts)
                               cloudflare <token> <https://your.domain>
                                                     your own Cloudflare tunnel,
                                                     a stable address
                               url <https://...>     an address you set up
                                                     yourself (Tailscale Funnel,
                                                     a reverse proxy)
                               relay <https://...>   your own BrainWashed relay
                               off                   local network only
  brainwashed service install    Start BrainWashed in the background at login.
  brainwashed service uninstall  Stop doing that.

Options:
  --port <port>              Port to serve on (default 47860).
  --no-browser               Don't open the admin page.
  --local-only               Don't open a tunnel this time.
  --data-dir <dir>           Keep models and settings here instead.
  -y, --yes                  Don't ask before installing.
  -V, --version              Print the version.
";

struct Args {
    command: String,
    operands: Vec<String>,
    port: Option<u16>,
    browser: bool,
    local_only: bool,
    data_dir: Option<PathBuf>,
    yes: bool,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut args = Args {
        command: "serve".into(),
        operands: Vec::new(),
        port: None,
        browser: true,
        local_only: false,
        data_dir: None,
        yes: false,
    };
    let mut positional = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => args.command = "help".into(),
            "-V" | "--version" => args.command = "version".into(),
            "--no-browser" => args.browser = false,
            "--local-only" => args.local_only = true,
            "-y" | "--yes" => args.yes = true,
            "--port" => {
                let p = it.next().ok_or("--port needs a number")?;
                args.port = Some(
                    p.parse()
                        .map_err(|_| format!("`{p}` isn't a port number"))?,
                );
            }
            "--data-dir" => {
                args.data_dir = Some(it.next().ok_or("--data-dir needs a folder")?.into())
            }
            a if a.starts_with('-') => return Err(format!("unknown option `{a}`")),
            _ => positional.push(arg),
        }
    }
    let mut positional = positional.into_iter();
    if let Some(cmd) = positional.next() {
        if args.command == "serve" {
            args.command = cmd;
        }
    }
    args.operands = positional.collect();
    Ok(args)
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\nError: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn data_dir(args: &Args) -> Result<PathBuf> {
    match args
        .data_dir
        .clone()
        .or_else(|| std::env::var_os("BRAINWASHED_DATA_DIR").map(PathBuf::from))
    {
        Some(d) => Ok(d),
        // Kept from the desktop app days, so existing models and devices stay.
        None => Ok(dirs::data_dir()
            .ok_or("can't find your app data folder; pass --data-dir")?
            .join("org.brainwashed.host")),
    }
}

async fn run(args: Args) -> Result {
    match args.command.as_str() {
        "help" => {
            print!("{HELP}");
            return Ok(());
        }
        "version" => {
            println!("brainwashed {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }

    let data_dir = data_dir(&args)?;
    // Commands that talk to a running server don't need the engine.
    match args.command.as_str() {
        "open" => {
            let control = Control::find(&data_dir, &args)
                .await
                .ok_or("BrainWashed isn't running. Start it with `brainwashed`.")?;
            return open_admin(&control, args.browser).await;
        }
        "status" => return status(&data_dir, &args).await,
        "stop" => {
            let control = Control::find(&data_dir, &args)
                .await
                .ok_or("BrainWashed isn't running.")?;
            control.post("/control/stop").await?;
            println!("BrainWashed is stopping.");
            return Ok(());
        }
        "service" => return service::command(&args.operands, &data_dir),
        _ => {}
    }

    let engine = Engine::new(EngineConfig::new(&data_dir, env!("CARGO_PKG_VERSION")))?;
    match args.command.as_str() {
        "serve" => {
            if let Some(control) = Control::find(&data_dir, &args).await {
                println!("BrainWashed is already running.");
                return open_admin(&control, args.browser).await;
            }
            serve(engine, &data_dir, &args).await
        }
        "chat" => chat(engine).await,
        "models" => {
            list_models(&engine);
            Ok(())
        }
        "pull" => {
            let wanted = args
                .operands
                .first()
                .ok_or("say which model, e.g. `brainwashed pull qwen3-4b`")?;
            pull(&engine, wanted).await.map(|_| ())
        }
        "use" => {
            let wanted = args
                .operands
                .first()
                .ok_or("say which model, e.g. `brainwashed use qwen3-4b`")?;
            let model = find_installed(&engine, wanted)
                .ok_or_else(|| format!("`{wanted}` isn't installed; see `brainwashed models`"))?;
            let mut settings = engine.settings();
            settings.active_model = Some(model.id.clone());
            engine.update_settings(settings)?;
            println!("BrainWashed will use {} from now on.", model.name);
            Ok(())
        }
        "skills" => skills(&engine, &args).await,
        "remote" => remote(
            &engine,
            &args.operands,
            Control::find(&data_dir, &args).await.is_some(),
        ),
        other => Err(format!("unknown command `{other}`; run `brainwashed --help`").into()),
    }
}

// ----- skills -----

async fn skills(engine: &Engine, args: &Args) -> Result {
    let ops = &args.operands;
    match ops.first().map(String::as_str) {
        None => {
            let list = engine.skills();
            println!("Skills folder: {}\n", list.dir.display());
            for s in &list.skills {
                let off = if s.enabled { "" } else { " (off)" };
                let from = match &s.origin {
                    Some(o) if o.community => " [community]",
                    Some(_) => " [installed from a link]",
                    None => "",
                };
                println!(
                    "  {}{off}{from}: {}",
                    s.entry.skill.name, s.entry.skill.description
                );
            }
            for e in &list.errors {
                println!("  ! {}: {}", e.path.display(), e.message);
            }
            println!("\nAdd a skill by creating a folder there with a SKILL.md file in it, or in the admin page. Changes apply right away.");
            println!("Find skills other people shared with `brainwashed skills search`.");
            Ok(())
        }
        Some("search") => {
            let words: Vec<String> = ops[1..].iter().map(|w| w.to_lowercase()).collect();
            let found: Vec<_> = engine
                .community_skills()
                .await?
                .into_iter()
                .filter(|s| {
                    let text = format!(
                        "{} {} {} {}",
                        s.name,
                        s.description,
                        s.triggers.join(" "),
                        s.category.as_deref().unwrap_or("")
                    )
                    .to_lowercase();
                    words.iter().all(|w| text.contains(w))
                })
                .collect();
            if found.is_empty() {
                println!("No community skills match.");
            }
            for s in &found {
                let by = s
                    .author
                    .as_deref()
                    .map(|a| format!(" (by {a})"))
                    .unwrap_or_default();
                println!("  {}{by}: {}", s.name, s.description);
            }
            if !found.is_empty() {
                println!("\nInstall one with `brainwashed skills install <name>`.");
            }
            Ok(())
        }
        Some("install") => {
            let spec = ops
                .get(1)
                .ok_or("say which skill, e.g. `brainwashed skills install meal-planner`")?;
            let preview = engine.preview_skill(spec).await?;
            println!("{}\n", preview.source.trim_end());
            println!("--- from {}", preview.url);
            for w in &preview.warnings {
                println!("  ! {w}");
            }
            if preview.installed {
                println!("This replaces your skill named `{}`.", preview.name);
            }
            if !args.yes && !confirm(&format!("Install {}?", preview.name))? {
                println!("Not installed.");
                return Ok(());
            }
            let done = engine
                .install_skill(spec, Some(&preview.sha256), true)
                .await?;
            println!("Installed {}. It applies right away.", done.name);
            Ok(())
        }
        Some("update") => {
            let index = engine.community_skills().await?;
            let mut updated = 0;
            for s in engine.skills().skills {
                let Some(origin) = s.origin.filter(|o| o.community) else {
                    continue;
                };
                let name = &s.entry.skill.name;
                let Some(latest) = index.iter().find(|c| &c.name == name) else {
                    continue;
                };
                if latest.sha256.eq_ignore_ascii_case(&origin.sha256) {
                    continue;
                }
                if s.modified {
                    println!("  {name}: an update is out, but you changed this skill, so it was left alone. Run `brainwashed skills install {name}` to replace it.");
                    continue;
                }
                engine
                    .install_skill(name, Some(&latest.sha256), true)
                    .await?;
                println!("  {name}: updated");
                updated += 1;
            }
            println!(
                "{updated} skill{} updated.",
                if updated == 1 { "" } else { "s" }
            );
            Ok(())
        }
        Some(other) => {
            Err(format!("unknown skills command `{other}`; try search, install or update").into())
        }
    }
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

// ----- serve -----

async fn serve(engine: Engine, data_dir: &Path, args: &Args) -> Result {
    println!("BrainWashed {}", env!("CARGO_PKG_VERSION"));

    let gateway = Gateway::new(engine.clone(), data_dir)?;
    if let Err(e) = gateway.apply_settings() {
        eprintln!("Check your remote access settings: {e}");
    }
    if args.local_only {
        gateway.set_tunnel(None);
    }
    let port = args.port.unwrap_or(engine.settings().phone_port);
    gateway.start(port).await.map_err(|e| {
        format!("couldn't serve on port {port} ({e}). Is another program using it? Pass --port.")
    })?;
    let watcher = engine.watch_skills(Duration::from_secs(2));

    // The model loads in the background so the admin page opens right away
    // and shows the progress.
    let loader = tokio::spawn(load_model_in_background(engine.clone()));

    let local = gateway.create_pairing_offer(DeviceRole::Admin)?;
    let admin_url = local_link(&local);
    println!("\nAdmin page on this computer:\n  {admin_url}");
    if args.browser && !browser::open_app_window(&admin_url) {
        println!("  (couldn't open a browser; copy the link above)");
    }
    if let Some(lan) = &local.lan_url {
        let base = lan.split_once("/#").map_or(lan.as_str(), |(b, _)| b);
        println!("\nOn your local network: {base}");
    }
    if args.local_only {
        println!("Remote access is off for this run (--local-only).");
    } else {
        print_remote_status(&gateway).await;
    }
    println!(
        "\nSkills folder: {}\nPress Enter for a phone QR code, Ctrl+C to stop.",
        engine.skills_dir().display()
    );

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = true;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = gateway.stop_requested() => break,
            line = lines.next_line(), if stdin_open => match line {
                Ok(Some(_)) => show_phone_code(&gateway)?,
                // No terminal input (started at login, or from a script): keep serving.
                _ => stdin_open = false,
            },
        }
    }
    println!("\nStopping...");
    loader.abort();
    watcher.abort();
    gateway.stop();
    engine.unload().await?;
    Ok(())
}

/// The admin link for this computer's own browser, on localhost so it works
/// even with no network.
fn local_link(offer: &PairingOffer) -> String {
    format!("http://localhost:{}/#pair?{}", offer.port, offer.query)
}

async fn load_model_in_background(engine: Engine) {
    let progress = spawn_progress_printer(&engine);
    let result = async {
        let model = ensure_model(&engine).await?;
        println!("\nLoading {}...", model.name);
        engine.load_model(&model.id).await?;
        Ok::<_, Box<dyn std::error::Error>>(model)
    }
    .await;
    progress.abort();
    clear_line();
    match result {
        Ok(model) => println!("\nReady: {}", model.name),
        Err(e) => println!("\nCouldn't load a model: {e}\nPick or download one in the admin page."),
    }
}

/// Waits briefly for the tunnel, then says where devices reach this computer
/// from anywhere. Keeps watching in the background and reports changes.
async fn print_remote_status(gateway: &Gateway) {
    let tunnel_on = gateway.status().tunnel.is_some();
    if !tunnel_on {
        match gateway.public_url() {
            Some(url) => println!("From anywhere: {url}"),
            None if gateway.status().relay.is_some() => {
                println!("From anywhere: through your relay, in the BrainWashed app")
            }
            None => println!(
                "Remote access is off, so only devices on your network can connect. Turn it on in the admin page or with `brainwashed remote quick`."
            ),
        }
        return;
    }
    println!("Opening a secure tunnel so devices can connect from anywhere...");
    for _ in 0..60 {
        if gateway.public_url().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let gw = gateway.clone();
    let mut last = gw.public_url();
    match &last {
        Some(url) => println!("From anywhere: {url}"),
        None => {
            let why = gw
                .status()
                .tunnel
                .and_then(|t| t.error)
                .unwrap_or_else(|| "no answer yet".into());
            println!("The tunnel isn't up yet ({why}). BrainWashed keeps trying; the admin page shows when it's ready.");
        }
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let now = gw.public_url();
            if now != last {
                match &now {
                    Some(url) => {
                        println!("\nFrom anywhere: {url}  (press Enter for a new phone code)")
                    }
                    None => println!("\nThe tunnel dropped; reconnecting..."),
                }
                last = now;
            }
        }
    });
}

fn show_phone_code(gateway: &Gateway) -> Result {
    let offer = gateway.create_pairing_offer(DeviceRole::Admin)?;
    let code = qrcode::QrCode::new(offer.web_url.as_bytes())?;
    let qr = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    let where_ = if offer.public_url.is_some() {
        "anywhere"
    } else {
        "the same Wi-Fi"
    };
    println!("\nOn your phone ({where_}), scan this with the camera:\n\n{qr}\n");
    println!("Or open: {}", offer.web_url);
    println!("This code makes the phone an admin, works once and expires in 10 minutes.");
    println!("To add people who can only chat, use Devices in the admin page.");
    Ok(())
}

// ----- talking to a running server -----

/// The running server, reached with the token it wrote for this user.
struct Control {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Control {
    async fn find(data_dir: &Path, args: &Args) -> Option<Control> {
        let token = std::fs::read_to_string(Gateway::control_token_path(data_dir)).ok()?;
        let port = args.port.unwrap_or_else(|| {
            Engine::new(EngineConfig::new(data_dir, env!("CARGO_PKG_VERSION")))
                .map(|e| e.settings().phone_port)
                .unwrap_or(brainwashed_gateway::DEFAULT_PORT)
        });
        let control = Control {
            base: format!("http://127.0.0.1:{port}"),
            token: token.trim().to_string(),
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .ok()?,
        };
        control.get("/control/status").await.ok()?;
        Some(control)
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }

    async fn post(&self, path: &str) -> Result<Value> {
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?;
        Ok(res.json().await?)
    }
}

async fn open_admin(control: &Control, browser: bool) -> Result {
    let offer = control.post("/control/admin-link").await?;
    let url = format!(
        "http://localhost:{}/#pair?{}",
        offer["port"],
        offer["query"].as_str().unwrap_or_default()
    );
    if browser && browser::open_app_window(&url) {
        println!("Opened the admin page.");
    } else {
        println!("Admin page (works once, for 10 minutes):\n  {url}");
    }
    Ok(())
}

async fn status(data_dir: &Path, args: &Args) -> Result {
    let Some(control) = Control::find(data_dir, args).await else {
        println!("BrainWashed isn't running. Start it with `brainwashed`.");
        return Ok(());
    };
    let s = control.get("/control/status").await?;
    let state = &s["state"];
    let model = match state["state"].as_str() {
        Some("ready") => format!("ready ({})", state["model"].as_str().unwrap_or_default()),
        Some("loading") => "loading a model".into(),
        Some("installingRuntime") => "downloading llama.cpp".into(),
        Some("error") => format!("error: {}", state["message"].as_str().unwrap_or_default()),
        _ => "no model loaded".into(),
    };
    let access = &s["access"];
    println!(
        "BrainWashed {} on {}",
        s["version"].as_str().unwrap_or("?"),
        s["hostName"].as_str().unwrap_or("?")
    );
    println!("  Model:     {model}");
    println!("  Port:      {}", access["port"]);
    println!("  Devices:   {}", s["devices"]);
    match access["publicUrl"].as_str() {
        Some(url) => println!("  Anywhere:  {url}"),
        None => match access["tunnel"]["error"].as_str() {
            Some(e) => println!("  Anywhere:  not yet ({e})"),
            None => println!("  Anywhere:  off"),
        },
    }
    if let Some(relay) = access["relay"]["url"].as_str() {
        let up = if access["relay"]["connected"] == true {
            "connected"
        } else {
            "not connected"
        };
        println!("  Relay:     {relay} ({up})");
    }
    Ok(())
}

// ----- remote access settings -----

fn remote(engine: &Engine, operands: &[String], running: bool) -> Result {
    let mut s = engine.settings();
    let arg = |i: usize, what: &str| -> Result<String> {
        operands
            .get(i)
            .cloned()
            .ok_or_else(|| format!("missing {what}; see `brainwashed --help`").into())
    };
    let check = |url: String| -> Result<String> {
        let url = url.trim().trim_end_matches('/').to_string();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err("the address must start with https://".into());
        }
        Ok(url)
    };
    match operands.first().map(String::as_str) {
        None => {
            println!("Remote access: {:?}", s.remote_access);
            if let Some(u) = &s.public_url {
                println!("Public address: {u}");
            }
            if let Some(r) = &s.relay_url {
                println!("Relay: {r}");
            }
            return Ok(());
        }
        Some("off") => {
            s.remote_access = RemoteAccess::Off;
            s.public_url = None;
        }
        Some("quick") => {
            s.remote_access = RemoteAccess::Quick;
            s.public_url = None;
        }
        Some("cloudflare") => {
            s.remote_access = RemoteAccess::Cloudflare;
            s.tunnel_token = Some(arg(1, "the tunnel token")?);
            s.public_url = Some(check(arg(2, "the tunnel's https:// address")?)?);
        }
        Some("url") => {
            s.remote_access = RemoteAccess::Off;
            s.public_url = Some(check(arg(1, "the https:// address")?)?);
        }
        Some("relay") => match arg(1, "the relay address or `off`")?.as_str() {
            "off" => s.relay_url = None,
            url => s.relay_url = Some(check(url.to_string())?),
        },
        Some(other) => {
            return Err(format!("unknown remote access `{other}`; see `brainwashed --help`").into())
        }
    }
    engine.update_settings(s)?;
    println!("Saved.");
    if running {
        println!("Restart BrainWashed to apply it (`brainwashed stop`, then `brainwashed`), or change it in the admin page, which applies right away.");
    }
    Ok(())
}

// ----- models -----

/// The model to load: the last one used, any installed one, or a download of
/// the first suggested model that fits this computer.
async fn ensure_model(engine: &Engine) -> Result<InstalledModel> {
    let models = engine.models();
    if let Some(m) = engine
        .settings()
        .active_model
        .and_then(|id| models.iter().find(|m| m.id == id).cloned())
        .or_else(|| models.first().cloned())
    {
        return Ok(m);
    }
    let pick = engine
        .catalog()
        .into_iter()
        .find(|c| c.fits)
        .or_else(|| engine.catalog().into_iter().next())
        .ok_or("no suggested models")?;
    println!(
        "No model yet, so downloading {} ({}). Get others in the admin page or with `brainwashed pull`.",
        pick.entry.name, pick.entry.description
    );
    pull(engine, &pick.entry.repo).await
}

async fn pull(engine: &Engine, wanted: &str) -> Result<InstalledModel> {
    let repo = match find_in_catalog(engine, wanted) {
        Some(c) => c.entry.repo,
        None if wanted.contains('/') => wanted.to_string(),
        None => {
            return Err(
                format!("no suggested model called `{wanted}`; see `brainwashed models`").into(),
            )
        }
    };
    let progress = spawn_progress_printer(engine);
    let result = engine.download_model(&repo, None).await;
    progress.abort();
    clear_line();
    let model = result?;
    println!("Downloaded {}.", model.name);
    Ok(model)
}

fn list_models(engine: &Engine) {
    let models = engine.models();
    let active = engine.settings().active_model;
    println!("Installed:");
    if models.is_empty() {
        println!("  none yet");
    }
    for m in &models {
        let mark = if active.as_deref() == Some(&m.id) {
            " (in use)"
        } else {
            ""
        };
        println!("  {} {}{mark}  [{}]", m.name, gb(m.size), slug(&m.name));
    }
    println!("\nSuggested (download with `brainwashed pull <name>`):");
    for c in engine.catalog() {
        let note = match (c.installed, c.fits) {
            (true, _) => " (installed)",
            (false, false) => " (needs more memory)",
            _ => "",
        };
        println!(
            "  {:<22} {}: {}{note}",
            slug(&c.entry.name),
            c.entry.name,
            c.entry.description
        );
    }
}

fn find_in_catalog(engine: &Engine, wanted: &str) -> Option<CatalogItem> {
    let w = wanted.to_lowercase();
    engine.catalog().into_iter().find(|c| {
        slug(&c.entry.name) == w
            || c.entry.name.to_lowercase() == w
            || c.entry.repo.to_lowercase() == w
    })
}

fn find_installed(engine: &Engine, wanted: &str) -> Option<InstalledModel> {
    let w = wanted.to_lowercase();
    let repo = find_in_catalog(engine, wanted).map(|c| c.entry.repo);
    engine.models().into_iter().find(|m| {
        m.id == wanted
            || slug(&m.name) == w
            || m.name.to_lowercase() == w
            || (repo.is_some() && m.repo == repo)
    })
}

/// "Qwen3 1.7B" -> "qwen3-1.7b", a name that's easy to type.
fn slug(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Prints download and runtime install progress on one line.
fn spawn_progress_printer(engine: &Engine) -> tokio::task::JoinHandle<()> {
    let mut events = engine.subscribe();
    tokio::spawn(async move {
        loop {
            let (what, done, total) = match events.recv().await {
                Ok(Event::DownloadProgress { done, total, .. }) => {
                    ("Downloading model", done, total)
                }
                Ok(Event::State(EngineState::InstallingRuntime { done, total })) => {
                    ("Downloading llama.cpp", done, total)
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let line = match total {
                Some(t) if t > 0 => {
                    format!("{what}: {} of {} ({}%)", gb(done), gb(t), done * 100 / t)
                }
                _ => format!("{what}: {}", gb(done)),
            };
            print!("\r{line:<60}");
            let _ = std::io::stdout().flush();
        }
    })
}

fn clear_line() {
    print!("\r{:<60}\r", "");
    let _ = std::io::stdout().flush();
}

// ----- chat -----

async fn chat(engine: Engine) -> Result {
    let progress = spawn_progress_printer(&engine);
    let model = ensure_model(&engine).await?;
    println!("Loading {}...", model.name);
    engine.load_model(&model.id).await?;
    progress.abort();
    clear_line();
    println!(
        "Chatting with {}. Type a message, /new to start over, or /quit. Ctrl+C stops an answer.\n",
        model.name
    );

    let mut conversation: Vec<ChatMessage> = Vec::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        // Once Ctrl+C has stopped an answer it no longer quits by itself.
        let line = tokio::select! {
            line = lines.next_line() => line?,
            _ = tokio::signal::ctrl_c() => None,
        };
        let Some(line) = line else {
            println!();
            break;
        };
        let line = line.trim();
        match line {
            "" => continue,
            "/quit" | "/exit" => break,
            "/new" => {
                conversation.clear();
                println!("Started a new conversation.\n");
                continue;
            }
            _ => {}
        }
        conversation.push(ChatMessage::new(Role::User, line));
        let mut thinking = false;
        let mut asked = None;
        let written = std::cell::RefCell::new(String::new());
        let options = SamplingOptions::default();
        let reply = engine.chat(&conversation, &options, |event| {
            match event {
                ChatEvent::Skills { names } if !names.is_empty() => {
                    println!("(using skill: {})", names.join(", "));
                }
                ChatEvent::Reasoning { .. } if !thinking => {
                    thinking = true;
                    print!("(thinking...) ");
                }
                ChatEvent::Content { text } => {
                    print!("{text}");
                    written.borrow_mut().push_str(&text);
                }
                ChatEvent::ToolCall(call) if call.name == "ask_user" => {
                    let ask = call.ask_user().unwrap_or_default();
                    print!("\n{}", ask.question);
                    for (i, o) in ask.options.iter().enumerate() {
                        print!("\n  {}. {o}", i + 1);
                    }
                    asked = Some(ask.text());
                }
                ChatEvent::ToolCall(call) if call.name.is_empty() => {
                    print!("\n(the model tried to use a tool BrainWashed doesn't have)")
                }
                ChatEvent::ToolCall(call) => print!(
                    "\n(the model tried to use the tool {}, which BrainWashed doesn't have)",
                    call.name
                ),
                _ => {}
            }
            let _ = std::io::stdout().flush();
        });
        // Ctrl+C stops this answer instead of quitting.
        let mut answer = tokio::select! {
            answer = reply => answer?,
            _ = tokio::signal::ctrl_c() => {
                print!(" (stopped)");
                written.take()
            }
        };
        println!("\n");
        // The question stays in the history, so the reply makes sense.
        if let Some(ask) = asked {
            if !answer.is_empty() {
                answer.push_str("\n\n");
            }
            answer.push_str(&ask);
        }
        conversation.push(ChatMessage::new(Role::Assistant, answer));
    }
    engine.unload().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_easy_to_type() {
        assert_eq!(slug("Qwen3 1.7B"), "qwen3-1.7b");
        assert_eq!(slug("Llama 3.2 3B Instruct"), "llama-3.2-3b-instruct");
    }
}
