//! Makes pictures and videos with stable-diffusion.cpp's `sd-cli`: finds the
//! right prebuilt release for this machine and runs one generation at a time.
//!
//! stable-diffusion.cpp runs Stable Diffusion 1.x/2.x/XL/3.x, FLUX, Z-Image,
//! Qwen-Image and Wan video models from GGUF or safetensors files, on the same
//! ggml backends as llama.cpp (Metal, Vulkan, CUDA, CPU).

use crate::release::{Asset, Release};
use crate::{Backend, Error, Hardware, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Recent stable-diffusion.cpp releases, newest first. Each `master-…` build
/// is a full release with binaries.
pub const RELEASES_URL: &str =
    "https://api.github.com/repos/leejet/stable-diffusion.cpp/releases?per_page=10";

/// The command-line program in a release, newest name first.
pub const BINARIES: &[&str] = if cfg!(windows) {
    &["sd-cli.exe", "sd.exe"]
} else {
    &["sd-cli", "sd"]
};

/// What a model makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
}

/// The part a file plays in a model. Image and video models are often split
/// into a diffusion model, text encoders and a VAE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileRole {
    /// A full checkpoint with everything inside, like most SD 1.5 and SDXL files.
    Model,
    DiffusionModel,
    /// Wan 2.2 A14B's second expert.
    HighNoiseDiffusionModel,
    Vae,
    ClipL,
    ClipG,
    T5xxl,
    /// A language model used as the text encoder (Qwen-Image, Z-Image, FLUX.2).
    Llm,
    ClipVision,
    /// A tiny VAE: much faster decoding, lower quality.
    Taesd,
}

impl FileRole {
    pub fn flag(self) -> &'static str {
        match self {
            FileRole::Model => "--model",
            FileRole::DiffusionModel => "--diffusion-model",
            FileRole::HighNoiseDiffusionModel => "--high-noise-diffusion-model",
            FileRole::Vae => "--vae",
            FileRole::ClipL => "--clip_l",
            FileRole::ClipG => "--clip_g",
            FileRole::T5xxl => "--t5xxl",
            FileRole::Llm => "--llm",
            FileRole::ClipVision => "--clip_vision",
            FileRole::Taesd => "--taesd",
        }
    }
}

/// One generation.
#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub binary: PathBuf,
    pub kind: MediaKind,
    pub files: Vec<(FileRole, PathBuf)>,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg_scale: f32,
    pub seed: i64,
    pub sampler: Option<String>,
    pub flow_shift: Option<f32>,
    /// Videos only.
    pub frames: Option<u32>,
    pub fps: Option<u32>,
    /// A picture to start from: changed into a new picture, or set in motion.
    pub init_image: Option<PathBuf>,
    /// How much of the starting picture to change, 0 to 1. Pictures only.
    pub strength: Option<f32>,
    /// Extra flags the model's recipe asks for, e.g. `--diffusion-fa`.
    pub extra_args: Vec<String>,
    /// `.png` for pictures; `.webp` (animated), `.webm` or `.avi` for videos.
    pub output: PathBuf,
}

/// The command line for one generation.
pub fn args(o: &GenerateOptions) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |k: &str, v: OsString| {
        a.push(k.into());
        a.push(v);
    };
    if o.kind == MediaKind::Video {
        push("-M", "vid_gen".into());
    }
    for (role, path) in &o.files {
        push(role.flag(), path.into());
    }
    push("--prompt", o.prompt.clone().into());
    if let Some(n) = o.negative_prompt.as_ref().filter(|n| !n.is_empty()) {
        push("--negative-prompt", n.into());
    }
    push("--width", o.width.to_string().into());
    push("--height", o.height.to_string().into());
    push("--steps", o.steps.to_string().into());
    push("--cfg-scale", o.cfg_scale.to_string().into());
    push("--seed", o.seed.to_string().into());
    if let Some(s) = &o.sampler {
        push("--sampling-method", s.into());
    }
    if let Some(f) = o.flow_shift {
        push("--flow-shift", f.to_string().into());
    }
    if o.kind == MediaKind::Video {
        if let Some(f) = o.frames {
            push("--video-frames", f.to_string().into());
        }
        if let Some(f) = o.fps {
            push("--fps", f.to_string().into());
        }
    }
    if let Some(img) = &o.init_image {
        push("--init-img", img.into());
        if o.kind == MediaKind::Image {
            if let Some(s) = o.strength {
                push("--strength", s.to_string().into());
            }
        }
    }
    push("--output", o.output.clone().into());
    a.extend(o.extra_args.iter().map(OsString::from));
    a
}

/// How far a generation has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// Reading the model files into memory.
    Loading,
    /// Denoising, one step at a time.
    Drawing,
    /// Turning the result into pixels and writing the file.
    Finishing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: Stage,
    pub step: u32,
    pub steps: u32,
}

/// Reads `12/20` out of one of sd-cli's progress lines, e.g.
/// `|=====>      | 12/20 - 1.52s/it`.
pub fn parse_progress(line: &str) -> Option<(u32, u32)> {
    let bar_end = line.rfind('|')?;
    let rest = line[bar_end + 1..].trim_start();
    let (counts, _) = rest.split_once(" - ")?;
    let (step, steps) = counts.trim().split_once('/')?;
    let step = step.parse().ok()?;
    let steps: u32 = steps.parse().ok()?;
    (steps > 0 && step <= steps).then_some((step, steps))
}

/// Follows sd-cli's output and works out the stage. Loading files and
/// decoding in tiles print progress bars too; only bars with as many steps as
/// asked for are drawing.
#[derive(Debug)]
pub struct ProgressTracker {
    steps: Vec<u32>,
    total: u32,
    /// Steps of the passes already finished.
    done_before: u32,
    pass_step: u32,
    pass_steps: u32,
    last: Progress,
}

impl ProgressTracker {
    /// `steps`: the step counts a run uses (Wan 2.2 A14B runs two passes).
    pub fn new(steps: Vec<u32>) -> Self {
        let total = steps.iter().sum();
        ProgressTracker {
            steps,
            total,
            done_before: 0,
            pass_step: 0,
            pass_steps: 0,
            last: Progress {
                stage: Stage::Loading,
                step: 0,
                steps: total,
            },
        }
    }

    /// Feeds one line; returns the new progress if it changed.
    pub fn line(&mut self, line: &str) -> Option<Progress> {
        let next = match parse_progress(line) {
            Some((step, steps)) if self.steps.contains(&steps) => {
                // A bar starting over is the next pass.
                if step < self.pass_step {
                    self.done_before += self.pass_steps;
                }
                self.pass_step = step;
                self.pass_steps = steps;
                let done = (self.done_before + step).min(self.total);
                Progress {
                    stage: if done >= self.total {
                        Stage::Finishing
                    } else {
                        Stage::Drawing
                    },
                    step: done,
                    steps: self.total,
                }
            }
            _ if self.last.stage == Stage::Drawing
                && line.to_ascii_lowercase().contains("decod") =>
            {
                Progress {
                    stage: Stage::Finishing,
                    ..self.last
                }
            }
            _ => return None,
        };
        if next == self.last {
            return None;
        }
        self.last = next;
        Some(next)
    }
}

const LOG_LINES: usize = 60;

/// Runs one generation and waits for its file. Dropping the future stops it.
pub async fn generate(opts: &GenerateOptions, mut on_progress: impl FnMut(Progress)) -> Result<()> {
    let bin_dir = opts.binary.parent().unwrap_or(Path::new("."));
    if let Some(dir) = opts.output.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let _ = tokio::fs::remove_file(&opts.output).await;
    let mut cmd = Command::new(&opts.binary);
    cmd.args(args(opts))
        .current_dir(bin_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if cfg!(target_os = "linux") {
        cmd.env(
            "LD_LIBRARY_PATH",
            crate::llama::prepend_env("LD_LIBRARY_PATH", bin_dir),
        );
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::other(format!("could not start {}: {e}", opts.binary.display())))?;

    let mut steps = vec![opts.steps];
    if opts
        .files
        .iter()
        .any(|(r, _)| *r == FileRole::HighNoiseDiffusionModel)
    {
        // The high-noise pass uses the same count unless the recipe says otherwise.
        let high = opts
            .extra_args
            .iter()
            .position(|a| a == "--high-noise-steps")
            .and_then(|i| opts.extra_args.get(i + 1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(opts.steps);
        steps.push(high);
    }
    let mut tracker = ProgressTracker::new(steps);
    let mut log: VecDeque<String> = VecDeque::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout.take() {
        tokio::spawn(read_lines(out, tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(read_lines(err, tx));
    }
    while let Some(line) = rx.recv().await {
        tracing::debug!(target: "sd_cli", "{line}");
        if let Some(p) = tracker.line(&line) {
            on_progress(p);
        }
        if parse_progress(&line).is_none() {
            if log.len() == LOG_LINES {
                log.pop_front();
            }
            log.push_back(line);
        }
    }
    let status = child.wait().await?;
    let tail = || {
        let skip = log.len().saturating_sub(15);
        log.iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    };
    if !status.success() {
        return Err(Error::other(format!(
            "stable-diffusion.cpp exited with {status}.{}\n{}",
            status.code().and_then(explain).unwrap_or(""),
            tail()
        )));
    }
    if !tokio::fs::try_exists(&opts.output).await.unwrap_or(false) {
        return Err(Error::other(format!(
            "stable-diffusion.cpp finished without writing a file.\n{}",
            tail()
        )));
    }
    Ok(())
}

/// Splits output on newlines and on the carriage returns progress bars use.
async fn read_lines(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let n = match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if !pending.is_empty() {
                    let line = strip_ansi(&String::from_utf8_lossy(&pending));
                    pending.clear();
                    if !line.trim().is_empty() && tx.send(line).is_err() {
                        return;
                    }
                }
            } else {
                pending.push(b);
            }
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(strip_ansi(&String::from_utf8_lossy(&pending)));
    }
}

/// Removes terminal escape codes, like the "clear line" after progress bars.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn explain(code: i32) -> Option<&'static str> {
    match code as u32 {
        0xC0E9_0002 => Some(
            " Windows blocked stable-diffusion.cpp's files because they aren't signed (Smart App \
             Control or a company policy).",
        ),
        0xC000_0135 => Some(
            " A file stable-diffusion.cpp needs is missing. Install the Microsoft Visual C++ \
             Redistributable, or delete the runtime folder so it downloads again.",
        ),
        0xC000_001D => Some(" This processor lacks an instruction the build uses. Try the CPU backend in Settings."),
        // Killed by the system, usually for running out of memory.
        137 | 0xC000_0017 => Some(
            " The computer ran out of memory. Try a smaller size, fewer frames, or a smaller model.",
        ),
        _ => None,
    }
}

// ----- releases -----

/// Whether a release asset is the build for this machine.
fn matches(name: &str, hw: &Hardware, backend: Backend) -> bool {
    if !name.ends_with(".zip") {
        return false;
    }
    match (hw.os, hw.arch, backend) {
        // The macOS build is universal and uses Metal.
        ("macos", _, _) => name.contains("-bin-Darwin-"),
        ("linux", "x86_64", Backend::Vulkan) => {
            name.contains("-bin-Linux-") && name.ends_with("-x86_64-vulkan.zip")
        }
        ("linux", "x86_64", _) => name.contains("-bin-Linux-") && name.ends_with("-x86_64.zip"),
        ("windows", "x86_64", Backend::Vulkan) => name.ends_with("-bin-win-vulkan-x64.zip"),
        // Older releases called the CPU build avx2.
        ("windows", "x86_64", _) => {
            name.ends_with("-bin-win-cpu-x64.zip") || name.ends_with("-bin-win-avx2-x64.zip")
        }
        _ => false,
    }
}

/// Picks the newest release with a build for this machine, falling back to
/// the CPU build when there is no GPU one.
pub fn select_newest<'a>(
    releases: &'a [Release],
    hw: &Hardware,
    backend: Backend,
) -> Result<(&'a Release, &'a Asset)> {
    let wanted = hw.resolve(backend);
    let find = |b: Backend| {
        releases.iter().filter(|r| !r.draft).find_map(|r| {
            r.assets
                .iter()
                .find(|a| matches(&a.name, hw, b))
                .map(|a| (r, a))
        })
    };
    find(wanted)
        .or_else(|| (wanted != Backend::Cpu).then(|| find(Backend::Cpu)).flatten())
        .ok_or_else(|| {
            Error::other(format!(
                "no recent stable-diffusion.cpp release has a build for {}/{}; set a custom sd-cli path in settings",
                hw.os, hw.arch
            ))
        })
}

/// A short name for the build, used to tell installed runtimes apart.
pub fn platform_label(hw: &Hardware, backend: Backend) -> String {
    format!("{}-{}-{:?}", hw.os, hw.arch, hw.resolve(backend)).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(os: &'static str, arch: &'static str) -> Hardware {
        Hardware {
            os,
            arch,
            total_memory_bytes: 0,
            cpu_cores: 1,
        }
    }

    fn releases() -> Vec<Release> {
        let names = [
            "sd-master-2f88688-bin-Darwin-macOS-15.6-arm64.zip",
            "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64.zip",
            "sd-master-2f88688-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip",
            "sd-master-2f88688-bin-win-cpu-x64.zip",
            "sd-master-2f88688-bin-win-cuda12-x64.zip",
            "sd-master-2f88688-bin-win-vulkan-x64.zip",
            "sd-master-2f88688-bin-win-rocm-7.1-x64.zip",
            "cudart-sd-bin-win-cu12-x64.zip",
        ];
        let assets: Vec<_> = names
            .iter()
            .map(|n| serde_json::json!({"name": n, "browser_download_url": format!("https://x/{n}")}))
            .collect();
        serde_json::from_value(serde_json::json!([
            {"tag_name": "master-921-aaaaaaa", "draft": true, "assets": []},
            {"tag_name": "master-920-2f88688", "assets": assets},
        ]))
        .unwrap()
    }

    #[test]
    fn picks_the_build_for_each_platform() {
        let r = releases();
        let pick = |os, arch, b| select_newest(&r, &hw(os, arch), b).unwrap().1.name.clone();
        assert!(pick("macos", "aarch64", Backend::Auto).contains("Darwin"));
        assert!(pick("macos", "x86_64", Backend::Auto).contains("Darwin"));
        assert!(pick("linux", "x86_64", Backend::Auto).ends_with("x86_64-vulkan.zip"));
        assert!(pick("linux", "x86_64", Backend::Cpu).ends_with("24.04-x86_64.zip"));
        assert!(pick("windows", "x86_64", Backend::Auto).ends_with("win-vulkan-x64.zip"));
        assert!(pick("windows", "x86_64", Backend::Cpu).ends_with("win-cpu-x64.zip"));
        assert!(select_newest(&r, &hw("linux", "aarch64"), Backend::Auto).is_err());
    }

    #[test]
    fn falls_back_to_cpu() {
        let r: Vec<Release> = serde_json::from_value(serde_json::json!([
            {"tag_name": "master-1", "assets": [
                {"name": "sd-master-1-bin-win-avx2-x64.zip", "browser_download_url": "x"}]}
        ]))
        .unwrap();
        let (_, a) = select_newest(&r, &hw("windows", "x86_64"), Backend::Auto).unwrap();
        assert_eq!(a.name, "sd-master-1-bin-win-avx2-x64.zip");
    }

    #[test]
    fn reads_progress_lines() {
        assert_eq!(
            parse_progress("  |=====>     | 3/20 - 1.52s/it"),
            Some((3, 20))
        );
        assert_eq!(
            parse_progress("|==================================================| 20/20 - 2.10it/s"),
            Some((20, 20))
        );
        assert_eq!(
            parse_progress("[INFO ] stable-diffusion.cpp:123 - loading model"),
            None
        );
        assert_eq!(parse_progress("| 0/0 - x"), None);
        assert_eq!(strip_ansi("| 1/4 - 1.0s/it\u{1b}[K"), "| 1/4 - 1.0s/it");
    }

    #[test]
    fn tracks_stages() {
        let mut t = ProgressTracker::new(vec![4]);
        // Loading tensors prints its own bar.
        assert_eq!(t.line("|====| 900/1200 - 300.00it/s"), None);
        let p = t.line("|=>   | 1/4 - 1.00s/it").unwrap();
        assert_eq!((p.stage, p.step, p.steps), (Stage::Drawing, 1, 4));
        let p = t.line("|====| 4/4 - 1.00s/it").unwrap();
        assert_eq!((p.stage, p.step), (Stage::Finishing, 4));

        // Two passes: 2 high-noise steps, then 2 low-noise ones.
        let mut t = ProgressTracker::new(vec![2, 2]);
        t.line("| 1/2 - 1s/it");
        t.line("| 2/2 - 1s/it");
        let p = t.line("| 1/2 - 1s/it").unwrap();
        assert_eq!((p.stage, p.step, p.steps), (Stage::Drawing, 3, 4));
    }

    #[test]
    fn builds_the_command_line() {
        let o = GenerateOptions {
            binary: "sd-cli".into(),
            kind: MediaKind::Video,
            files: vec![
                (FileRole::DiffusionModel, "wan.gguf".into()),
                (FileRole::Vae, "vae.safetensors".into()),
            ],
            prompt: "a cat".into(),
            negative_prompt: Some("blurry".into()),
            width: 832,
            height: 480,
            steps: 20,
            cfg_scale: 6.0,
            seed: 7,
            sampler: Some("euler".into()),
            flow_shift: Some(3.0),
            frames: Some(33),
            fps: Some(16),
            init_image: None,
            strength: Some(0.5),
            extra_args: vec!["--diffusion-fa".into()],
            output: "out.webp".into(),
        };
        let a: Vec<String> = args(&o)
            .into_iter()
            .map(|s| s.into_string().unwrap())
            .collect();
        let joined = a.join(" ");
        assert!(joined.starts_with("-M vid_gen --diffusion-model wan.gguf --vae vae.safetensors"));
        assert!(joined.contains("--video-frames 33 --fps 16"));
        assert!(joined.contains("--flow-shift 3"));
        assert!(joined.ends_with("--output out.webp --diffusion-fa"));
        assert!(!joined.contains("--strength"));
    }
}
