//! Finds the right prebuilt llama.cpp release for this machine.

use crate::{Backend, Error, Hardware, Result};
use serde::Deserialize;

pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

pub async fn fetch_latest(client: &reqwest::Client, url: &str) -> Result<Release> {
    let release = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(release)
}

/// The platform part of llama.cpp asset names, e.g. `ubuntu-vulkan-x64` in
/// `llama-b6500-bin-ubuntu-vulkan-x64.zip`.
pub fn platform_token(hw: &Hardware, backend: Backend) -> Result<&'static str> {
    let token = match (hw.os, hw.arch, hw.resolve(backend)) {
        ("macos", "aarch64", _) => "macos-arm64",
        ("macos", "x86_64", _) => "macos-x64",
        ("linux", "x86_64", Backend::Vulkan) => "ubuntu-vulkan-x64",
        ("linux", "x86_64", _) => "ubuntu-x64",
        ("linux", "aarch64", _) => "ubuntu-arm64",
        ("windows", "x86_64", Backend::Vulkan) => "win-vulkan-x64",
        ("windows", "x86_64", _) => "win-cpu-x64",
        ("windows", "aarch64", _) => "win-cpu-arm64",
        (os, arch, b) => {
            return Err(Error::other(format!(
                "no prebuilt llama.cpp for {os}/{arch} with {b:?}; set a custom llama-server path in settings"
            )))
        }
    };
    Ok(token)
}

/// Picks the asset for this machine from a release.
pub fn select_asset<'a>(
    release: &'a Release,
    hw: &Hardware,
    backend: Backend,
) -> Result<&'a Asset> {
    let token = platform_token(hw, backend)?;
    let needle = format!("-bin-{token}.");
    release
        .assets
        .iter()
        .filter(|a| a.name.contains(&needle))
        .find(|a| a.name.ends_with(".zip") || a.name.ends_with(".tar.gz"))
        .ok_or_else(|| {
            Error::other(format!(
                "llama.cpp {} has no `{token}` build",
                release.tag_name
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> Release {
        serde_json::from_str(include_str!("../tests/fixtures/release.json")).unwrap()
    }

    fn hw(os: &'static str, arch: &'static str) -> Hardware {
        Hardware {
            os,
            arch,
            total_memory_bytes: 0,
            cpu_cores: 1,
        }
    }

    #[test]
    fn selects_per_platform() {
        let r = release();
        let pick = |os, arch, b| select_asset(&r, &hw(os, arch), b).unwrap().name.clone();
        assert_eq!(
            pick("macos", "aarch64", Backend::Auto),
            "llama-b6600-bin-macos-arm64.zip"
        );
        assert_eq!(
            pick("linux", "x86_64", Backend::Auto),
            "llama-b6600-bin-ubuntu-vulkan-x64.zip"
        );
        assert_eq!(
            pick("linux", "x86_64", Backend::Cpu),
            "llama-b6600-bin-ubuntu-x64.zip"
        );
        assert_eq!(
            pick("windows", "x86_64", Backend::Auto),
            "llama-b6600-bin-win-vulkan-x64.zip"
        );
        assert_eq!(
            pick("windows", "x86_64", Backend::Cpu),
            "llama-b6600-bin-win-cpu-x64.zip"
        );
    }

    #[test]
    fn missing_build_is_an_error() {
        let r = release();
        assert!(select_asset(&r, &hw("linux", "aarch64"), Backend::Auto).is_err());
        assert!(platform_token(&hw("freebsd", "x86_64"), Backend::Auto).is_err());
    }
}
