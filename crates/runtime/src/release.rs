//! Finds the right prebuilt llama.cpp release for this machine.

use crate::{Backend, Error, Hardware, Result};
use serde::Deserialize;

/// Recent releases, newest first. Not `/releases/latest`: llama.cpp marks its
/// binary builds (`b1234`) as pre-releases, and its plain `vX.Y.Z` releases
/// carry no binaries, so "latest" can be a release with nothing to download.
pub const RELEASES_URL: &str =
    "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=20";

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

pub async fn fetch_releases(client: &reqwest::Client, url: &str) -> Result<Vec<Release>> {
    let releases = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(releases)
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
    find_asset(release, token).ok_or_else(|| {
        Error::other(format!(
            "llama.cpp {} has no `{token}` build",
            release.tag_name
        ))
    })
}

/// Picks the newest published release that has a build for this machine.
pub fn select_newest<'a>(
    releases: &'a [Release],
    hw: &Hardware,
    backend: Backend,
) -> Result<(&'a Release, &'a Asset)> {
    let token = platform_token(hw, backend)?;
    releases
        .iter()
        .filter(|r| !r.draft)
        .find_map(|r| find_asset(r, token).map(|a| (r, a)))
        .ok_or_else(|| {
            Error::other(format!(
                "no recent llama.cpp release has a `{token}` build; set a custom llama-server path in settings"
            ))
        })
}

fn find_asset<'a>(release: &'a Release, token: &str) -> Option<&'a Asset> {
    let needle = format!("-bin-{token}.");
    release
        .assets
        .iter()
        .filter(|a| a.name.contains(&needle))
        .find(|a| a.name.ends_with(".zip") || a.name.ends_with(".tar.gz"))
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
    fn skips_releases_without_binaries() {
        let releases: Vec<Release> = serde_json::from_value(serde_json::json!([
            {"tag_name": "b7001", "draft": true, "assets": [
                {"name": "llama-b7001-bin-win-vulkan-x64.zip", "browser_download_url": "x"}]},
            {"tag_name": "v0.5.0", "assets": [{"name": "llama-v0.5.0-ui.tar.gz", "browser_download_url": "x"}]},
            {"tag_name": "b7000", "assets": [
                {"name": "llama-b7000-bin-win-cpu-x64.zip", "browser_download_url": "x"},
                {"name": "llama-b7000-bin-win-vulkan-x64.zip", "browser_download_url": "x"}]}
        ]))
        .unwrap();
        let (rel, asset) =
            select_newest(&releases, &hw("windows", "x86_64"), Backend::Auto).unwrap();
        assert_eq!(rel.tag_name, "b7000");
        assert_eq!(asset.name, "llama-b7000-bin-win-vulkan-x64.zip");
        assert!(select_newest(&releases[..2], &hw("windows", "x86_64"), Backend::Auto).is_err());
    }

    #[test]
    fn missing_build_is_an_error() {
        let r = release();
        assert!(select_asset(&r, &hw("linux", "aarch64"), Backend::Auto).is_err());
        assert!(platform_token(&hw("freebsd", "x86_64"), Backend::Auto).is_err());
    }
}
