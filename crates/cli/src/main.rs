//! `brainwashed`: runs the BrainWashed host from a terminal, with no desktop
//! window. It shares its models, skills, settings and paired devices with the
//! desktop app, so use one or the other at a time.

use brainwashed_core::{
    CatalogItem, ChatEvent, ChatMessage, Engine, EngineConfig, EngineState, Event, InstalledModel,
    Role, SamplingOptions,
};
use brainwashed_gateway::Gateway;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::io::{AsyncBufReadExt, BufReader};

const HELP: &str = "\
BrainWashed runs open source AI models on this computer.

Usage:
  brainwashed [serve]        Load a model and serve the web chat to this computer
                             and to phones on your Wi-Fi. Downloads a small model
                             the first time.
  brainwashed chat           Chat in this terminal.
  brainwashed models         List installed and suggested models.
  brainwashed pull <model>   Download a model: a name from `models`, or any
                             Hugging Face GGUF repo like `Qwen/Qwen3-4B-GGUF`.
  brainwashed use <model>    Load this model from now on.
  brainwashed skills         Show your skills and the folder they live in.

Options:
  --port <port>              Port for the web chat (default 47860).
  --no-browser               Don't open the web chat in your browser.
  --data-dir <dir>           Keep models and settings here instead.
  -V, --version              Print the version.
";

struct Args {
    command: String,
    operand: Option<String>,
    port: Option<u16>,
    browser: bool,
    data_dir: Option<PathBuf>,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut args = Args {
        command: "serve".into(),
        operand: None,
        port: None,
        browser: true,
        data_dir: None,
    };
    let mut positional = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => args.command = "help".into(),
            "-V" | "--version" => args.command = "version".into(),
            "--no-browser" => args.browser = false,
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
    args.operand = positional.next();
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

    let data_dir = match args
        .data_dir
        .clone()
        .or_else(|| std::env::var_os("BRAINWASHED_DATA_DIR").map(PathBuf::from))
    {
        Some(d) => d,
        // The same folder the desktop app uses.
        None => dirs::data_dir()
            .ok_or("can't find your app data folder; pass --data-dir")?
            .join("org.brainwashed.host"),
    };
    let engine = Engine::new(EngineConfig::new(&data_dir, env!("CARGO_PKG_VERSION")))?;

    match args.command.as_str() {
        "serve" => serve(engine, &data_dir, &args).await,
        "chat" => chat(engine).await,
        "models" => {
            list_models(&engine);
            Ok(())
        }
        "pull" => {
            let wanted = args
                .operand
                .as_deref()
                .ok_or("say which model, e.g. `brainwashed pull qwen3-4b`")?;
            pull(&engine, wanted).await.map(|_| ())
        }
        "use" => {
            let wanted = args
                .operand
                .as_deref()
                .ok_or("say which model, e.g. `brainwashed use qwen3-4b`")?;
            let model = find_installed(&engine, wanted)
                .ok_or_else(|| format!("`{wanted}` isn't installed; see `brainwashed models`"))?;
            let mut settings = engine.settings();
            settings.active_model = Some(model.id.clone());
            engine.update_settings(settings)?;
            println!("BrainWashed will use {} from now on.", model.name);
            Ok(())
        }
        "skills" => {
            let list = engine.skills();
            println!("Skills folder: {}\n", list.dir.display());
            for s in &list.skills {
                let off = if s.enabled { "" } else { " (off)" };
                println!(
                    "  {}{off}: {}",
                    s.entry.skill.name, s.entry.skill.description
                );
            }
            for e in &list.errors {
                println!("  ! {}: {}", e.path.display(), e.message);
            }
            println!("\nAdd a skill by creating a folder there with a SKILL.md file in it. Changes apply right away.");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`; run `brainwashed --help`").into()),
    }
}

// ----- serve -----

async fn serve(engine: Engine, data_dir: &std::path::Path, args: &Args) -> Result {
    println!("BrainWashed {}", env!("CARGO_PKG_VERSION"));
    let progress = spawn_progress_printer(&engine);
    let model = ensure_model(&engine).await?;
    println!(
        "Loading {} (the first start also downloads the llama.cpp runtime)...",
        model.name
    );
    engine.load_model(&model.id).await?;
    progress.abort();
    clear_line();
    println!("Ready: {}", model.name);

    let gateway = Gateway::new(engine.clone(), data_dir)?;
    if let Err(e) = gateway.set_relay_url(engine.settings().relay_url.as_deref()) {
        eprintln!("Ignoring the saved relay address: {e}");
    }
    let port = args.port.unwrap_or(engine.settings().phone_port);
    let addr = gateway.start(port).await.map_err(|e| {
        format!("couldn't serve on port {port} ({e}). Is the BrainWashed app already running? Close it, or pass --port.")
    })?;
    let watcher = engine.watch_skills(std::time::Duration::from_secs(2));

    // A pairing code works once, so this computer's browser gets its own.
    let local = gateway.create_pairing_offer()?;
    let local_url = local.web_url.replacen(
        &format!("//{}:", host_of(&local.web_url)),
        "//localhost:",
        1,
    );
    println!("\nOn this computer, open:\n  {local_url}");
    if args.browser && open::that(&local_url).is_err() {
        println!("  (couldn't open your browser; copy the link above)");
    }

    if gateway.status().addresses.is_empty() {
        println!("\nThis computer isn't on a local network, so phones can't reach it.");
    } else {
        show_phone_code(&gateway)?;
    }
    println!(
        "\nServing on port {}. Skills: {}\nPress Enter for a new phone code, Ctrl+C to stop.",
        addr.port(),
        engine.skills_dir().display()
    );

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            line = lines.next_line() => match line {
                Ok(Some(_)) => show_phone_code(&gateway)?,
                // No terminal input (e.g. started from a script): keep serving.
                _ => {
                    tokio::signal::ctrl_c().await.ok();
                    break;
                }
            },
        }
    }
    println!("\nStopping...");
    watcher.abort();
    engine.unload().await?;
    Ok(())
}

fn show_phone_code(gateway: &Gateway) -> Result {
    let offer = gateway.create_pairing_offer()?;
    let code = qrcode::QrCode::new(offer.web_url.as_bytes())?;
    let qr = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("\nOn your phone (same Wi-Fi), scan this with the camera:\n\n{qr}\n");
    println!("Or open: {}", offer.web_url);
    println!("This code works once and expires in 10 minutes.");
    Ok(())
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("//").map_or(url, |(_, r)| r);
    rest.split([':', '/']).next().unwrap_or(rest)
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
        "No model yet, so downloading {} ({}). Get others with `brainwashed pull`.",
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
        "Chatting with {}. Type a message, /new to start over, or /quit.\n",
        model.name
    );

    let mut conversation: Vec<ChatMessage> = Vec::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let Some(line) = lines.next_line().await? else {
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
        let answer = engine
            .chat(&conversation, &SamplingOptions::default(), |event| {
                match event {
                    ChatEvent::Skills { names } if !names.is_empty() => {
                        println!("(using skill: {})", names.join(", "));
                    }
                    ChatEvent::Reasoning { .. } if !thinking => {
                        thinking = true;
                        print!("(thinking...) ");
                    }
                    ChatEvent::Content { text } => print!("{text}"),
                    _ => {}
                }
                let _ = std::io::stdout().flush();
            })
            .await?;
        println!("\n");
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

    #[test]
    fn finds_the_host_in_a_link() {
        assert_eq!(host_of("http://192.168.1.5:47860/#pair?v=1"), "192.168.1.5");
        assert_eq!(host_of("http://localhost:1/"), "localhost");
    }
}
