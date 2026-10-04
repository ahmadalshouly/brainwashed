//! `brainwashed service install`: starts BrainWashed in the background when
//! you log in, so the computer stays reachable without a terminal open.
//! A systemd user service on Linux, a launch agent on macOS, and a scheduled
//! task on Windows.

use std::path::Path;
use std::process::Command;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const NAME: &str = "brainwashed";

pub fn command(operands: &[String], data_dir: &Path) -> Result {
    let exe = std::env::current_exe()?;
    match operands.first().map(String::as_str) {
        Some("install") => install(&exe, data_dir),
        Some("uninstall") => uninstall(),
        // Windows' scheduled task runs this to start the server with no window.
        Some("run") => run_detached(&exe, data_dir),
        _ => Err("use `brainwashed service install` or `brainwashed service uninstall`".into()),
    }
}

fn run(cmd: &mut Command) -> Result {
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(format!(
            "`{:?}` failed: {}",
            cmd,
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(())
}

fn data_dir_args(data_dir: &Path) -> Vec<String> {
    vec![
        "serve".into(),
        "--no-browser".into(),
        "--data-dir".into(),
        data_dir.display().to_string(),
    ]
}

#[cfg(target_os = "linux")]
fn install(exe: &Path, data_dir: &Path) -> Result {
    let dir = dirs::config_dir()
        .ok_or("can't find your config folder")?
        .join("systemd/user");
    std::fs::create_dir_all(&dir)?;
    let args = data_dir_args(data_dir)
        .iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let unit = format!(
        "[Unit]\n\
         Description=BrainWashed local AI\n\
         After=network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\n\
         ExecStart=\"{}\" {args}\n\
         Restart=on-failure\n\
         RestartSec=5\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display()
    );
    std::fs::write(dir.join(format!("{NAME}.service")), unit)?;
    run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run(Command::new("systemctl").args(["--user", "enable", "--now", NAME]))?;
    println!("BrainWashed now starts when you log in, and is running.");
    println!("Logs: journalctl --user -u {NAME} -f");
    println!("To keep it running when you're logged out: sudo loginctl enable-linger $USER");
    println!("Open the admin page with `brainwashed open`.");
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall() -> Result {
    let _ = run(Command::new("systemctl").args(["--user", "disable", "--now", NAME]));
    if let Some(dir) = dirs::config_dir() {
        let _ = std::fs::remove_file(dir.join(format!("systemd/user/{NAME}.service")));
    }
    let _ = run(Command::new("systemctl").args(["--user", "daemon-reload"]));
    println!("BrainWashed no longer starts at login.");
    Ok(())
}

#[cfg(target_os = "macos")]
const LABEL: &str = "org.brainwashed.host";

#[cfg(target_os = "macos")]
fn plist_path() -> Result<std::path::PathBuf> {
    Ok(dirs::home_dir()
        .ok_or("can't find your home folder")?
        .join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

#[cfg(target_os = "macos")]
fn install(exe: &Path, data_dir: &Path) -> Result {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut args = vec![exe.display().to_string()];
    args.extend(data_dir_args(data_dir));
    let args = args
        .iter()
        .map(|a| format!("    <string>{}</string>", escape(a)))
        .collect::<Vec<_>>()
        .join("\n");
    let log = dirs::home_dir()
        .ok_or("can't find your home folder")?
        .join("Library/Logs/BrainWashed.log");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        log = escape(&log.display().to_string())
    );
    let path = plist_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let _ = run(Command::new("launchctl").arg("unload").arg(&path));
    std::fs::write(&path, plist)?;
    run(Command::new("launchctl").arg("load").arg("-w").arg(&path))?;
    println!("BrainWashed now starts when you log in, and is running.");
    println!("Logs: {}", log.display());
    println!("Open the admin page with `brainwashed open`.");
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall() -> Result {
    let path = plist_path()?;
    let _ = run(Command::new("launchctl").arg("unload").arg("-w").arg(&path));
    let _ = std::fs::remove_file(&path);
    println!("BrainWashed no longer starts at login.");
    Ok(())
}

#[cfg(windows)]
fn install(exe: &Path, data_dir: &Path) -> Result {
    let task = format!(
        "\"{}\" service run --data-dir \"{}\"",
        exe.display(),
        data_dir.display()
    );
    run(Command::new("schtasks").args([
        "/Create",
        "/F",
        "/SC",
        "ONLOGON",
        "/RL",
        "LIMITED",
        "/TN",
        "BrainWashed",
        "/TR",
        &task,
    ]))?;
    run(Command::new("schtasks").args(["/Run", "/TN", "BrainWashed"]))?;
    println!("BrainWashed now starts when you log in, and is running.");
    println!("Logs: {}", data_dir.join("brainwashed.log").display());
    println!("Open the admin page with `brainwashed open`.");
    Ok(())
}

#[cfg(windows)]
fn uninstall() -> Result {
    let _ = run(Command::new("schtasks").args(["/Delete", "/F", "/TN", "BrainWashed"]));
    println!(
        "BrainWashed no longer starts at login. `brainwashed stop` stops the one running now."
    );
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn install(_: &Path, _: &Path) -> Result {
    Err("starting at login isn't supported on this system".into())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn uninstall() -> Result {
    Err("starting at login isn't supported on this system".into())
}

/// Starts the server with no console window, logging to a file, and returns.
fn run_detached(exe: &Path, data_dir: &Path) -> Result {
    std::fs::create_dir_all(data_dir)?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("brainwashed.log"))?;
    let mut cmd = Command::new(exe);
    cmd.args(data_dir_args(data_dir))
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | DETACHED_PROCESS
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
    cmd.spawn()?;
    Ok(())
}
