//! Tells the user when a newer BrainWashed release is out. Only asks GitHub
//! for the list of releases; nothing about this computer is sent.

use serde::{Deserialize, Serialize};

pub const RELEASES_URL: &str =
    "https://api.github.com/repos/ahmadalshouly/brainwashed/releases?per_page=20";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    /// Release page with the installers.
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
}

/// `1.2.3` or `1.2.3-beta.1` as comparable numbers; a pre-release sorts
/// before its release.
fn parse(version: &str) -> Option<(u64, u64, u64, bool)> {
    let version = version.trim_start_matches('v');
    let (core, pre) = match version.split_once('-') {
        Some((core, _)) => (core, true),
        None => (version, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    Some((major, minor, patch, !pre))
}

/// The newest published release, if it is newer than `current`. Dev builds
/// (version 0.0.0) never report updates.
pub fn newer_release(current: &str, releases_json: &str) -> Option<UpdateInfo> {
    let current = parse(current)?;
    if current.0 == 0 && current.1 == 0 && current.2 == 0 {
        return None;
    }
    let releases: Vec<Release> = serde_json::from_str(releases_json).ok()?;
    releases
        .into_iter()
        .filter(|r| !r.draft)
        .filter_map(|r| Some((parse(&r.tag_name)?, r)))
        .filter(|(v, _)| *v > current)
        .max_by_key(|(v, _)| *v)
        .map(|(_, r)| UpdateInfo {
            version: r.tag_name.trim_start_matches('v').to_string(),
            url: r.html_url,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASES: &str = r#"[
        {"tag_name":"v0.3.0","html_url":"https://x/v0.3.0","draft":true},
        {"tag_name":"v0.2.1-beta.1","html_url":"https://x/v0.2.1-beta.1"},
        {"tag_name":"v0.2.0","html_url":"https://x/v0.2.0"},
        {"tag_name":"not-a-version","html_url":"https://x/n"},
        {"tag_name":"v0.1.0","html_url":"https://x/v0.1.0"}
    ]"#;

    #[test]
    fn finds_the_newest_published_release() {
        let update = newer_release("0.1.0", RELEASES).unwrap();
        assert_eq!(update.version, "0.2.1-beta.1");
        assert_eq!(update.url, "https://x/v0.2.1-beta.1");
        // Drafts are not out yet.
        assert_eq!(newer_release("0.2.1", RELEASES), None);
        assert_eq!(
            newer_release("0.2.0", RELEASES).unwrap().version,
            "0.2.1-beta.1"
        );
    }

    #[test]
    fn dev_builds_and_garbage_report_nothing() {
        assert_eq!(newer_release("0.0.0", RELEASES), None);
        assert_eq!(newer_release("0.1.0", "not json"), None);
        assert!(parse("1.2.3-rc.1").unwrap() < parse("1.2.3").unwrap());
        assert!(parse("1.10.0").unwrap() > parse("1.9.9").unwrap());
    }
}
