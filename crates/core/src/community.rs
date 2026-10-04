//! Community skills: browsing the shared index and installing skills from it
//! or from any web address.
//!
//! The index is a JSON file the community registry
//! (github.com/ahmadalshouly/brainwashed-skills) publishes on every merge. Each
//! entry points at a SKILL.md pinned to the commit it was reviewed at, with the
//! file's SHA-256, so what you install is exactly what was reviewed. An
//! installed skill remembers where it came from in `origin.json` next to its
//! SKILL.md, which is how updates are found.

use crate::{Engine, Error, Result};
use brainwashed_skills::{review, Skill, SKILL_FILE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Where the community registry publishes its index.
pub const DEFAULT_SKILL_INDEX: &str =
    "https://ahmadalshouly.github.io/brainwashed-skills/index.json";

/// Bigger files aren't skills.
const MAX_SKILL_BYTES: usize = 64 * 1024;
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;

/// Name of the file that records where an installed skill came from.
pub const ORIGIN_FILE: &str = "origin.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunitySkill {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// The SKILL.md, pinned to a commit.
    pub url: String,
    pub sha256: String,
    /// Page to read about it, e.g. on GitHub.
    #[serde(default)]
    pub page: Option<String>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
struct Index {
    skills: Vec<CommunitySkill>,
}

/// Where an installed skill came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillOrigin {
    pub url: String,
    pub sha256: String,
    /// Installed from the community index, rather than a link.
    #[serde(default)]
    pub community: bool,
    /// Seconds since 1970.
    pub installed_at: u64,
}

/// A skill fetched for a person to read before installing it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreview {
    pub name: String,
    pub description: String,
    pub source: String,
    pub url: String,
    pub sha256: String,
    pub community: bool,
    /// Things worth a close look, from `brainwashed_skills::review`.
    pub warnings: Vec<String>,
    /// A skill with this name is already installed.
    pub installed: bool,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) fn read_origin(skill_file: &Path) -> Option<SkillOrigin> {
    let path = skill_file.parent()?.join(ORIGIN_FILE);
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Turns a GitHub page address into the raw file behind it, and a folder into
/// its SKILL.md. Other addresses are used as they are.
pub fn raw_skill_url(url: &str) -> Result<String> {
    let url = url.trim();
    let local = url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost")
        || url.starts_with("http://[::1]");
    if !url.starts_with("https://") && !local {
        return Err(Error::Invalid(
            "skill addresses must start with https://".into(),
        ));
    }
    let mut url = url.trim_end_matches('/').to_string();
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        // owner/repo/(blob|tree)/ref/path...
        let parts: Vec<&str> = rest.splitn(4, '/').collect();
        if let [owner, repo, "blob" | "tree", tail] = parts[..] {
            url = format!("https://raw.githubusercontent.com/{owner}/{repo}/{tail}");
        } else if parts.len() == 2 {
            return Err(Error::Invalid(
                "that's a whole repository; link to the skill's folder or SKILL.md".into(),
            ));
        }
    }
    let file = url.rsplit('/').next().unwrap_or("");
    if !file.to_ascii_lowercase().ends_with(".md") {
        url = format!("{url}/{SKILL_FILE}");
    }
    Ok(url)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Engine {
    fn skill_index_url(&self) -> String {
        self.settings()
            .skill_index
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SKILL_INDEX.to_string())
    }

    async fn fetch_text(&self, url: &str, limit: usize) -> Result<String> {
        let failed = |e: reqwest::Error| Error::Invalid(format!("couldn't download {url}: {e}"));
        let resp = self
            .inner
            .client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(failed)?;
        if !resp.status().is_success() {
            return Err(Error::Invalid(format!(
                "couldn't download {url}: {}",
                resp.status()
            )));
        }
        if resp.content_length().is_some_and(|n| n as usize > limit) {
            return Err(Error::Invalid(format!("{url} is too big")));
        }
        let bytes = resp.bytes().await.map_err(failed)?;
        if bytes.len() > limit {
            return Err(Error::Invalid(format!("{url} is too big")));
        }
        String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::Invalid(format!("{url} isn't a text file")))
    }

    /// Every skill in the community index.
    pub async fn community_skills(&self) -> Result<Vec<CommunitySkill>> {
        let url = self.skill_index_url();
        let text = self.fetch_text(&url, MAX_INDEX_BYTES).await?;
        let index: Index = serde_json::from_str(&text)
            .map_err(|e| Error::Invalid(format!("the skill index at {url} is broken: {e}")))?;
        Ok(index.skills)
    }

    /// Downloads a skill for a person to read: `spec` is a community skill's
    /// name or a web address.
    pub async fn preview_skill(&self, spec: &str) -> Result<SkillPreview> {
        let spec = spec.trim();
        let (url, expected, community) = if spec.contains("://") {
            (raw_skill_url(spec)?, None, false)
        } else {
            let found = self
                .community_skills()
                .await?
                .into_iter()
                .find(|s| s.name == spec)
                .ok_or_else(|| {
                    Error::Invalid(format!("there's no community skill named `{spec}`"))
                })?;
            (found.url, Some(found.sha256), true)
        };
        let source = self.fetch_text(&url, MAX_SKILL_BYTES).await?;
        let sha256 = sha256_hex(source.as_bytes());
        if expected.is_some_and(|e| !e.eq_ignore_ascii_case(&sha256)) {
            return Err(Error::Invalid(format!(
                "{url} doesn't match the community index; it may have been tampered with"
            )));
        }
        let skill = Skill::parse(&source)
            .map_err(|e| Error::Invalid(format!("{url} isn't a valid skill: {e}")))?;
        let installed = self
            .skills()
            .skills
            .iter()
            .any(|s| s.entry.skill.name == skill.meta.name);
        Ok(SkillPreview {
            warnings: review(&skill),
            name: skill.meta.name,
            description: skill.meta.description,
            source,
            url,
            sha256,
            community,
            installed,
        })
    }

    /// Installs a skill. `sha256` is the hash of the version the person read
    /// in `preview_skill`; if the file changed since, nothing is installed.
    /// A skill with the same name is only replaced when `replace` is set.
    pub async fn install_skill(
        &self,
        spec: &str,
        sha256: Option<&str>,
        replace: bool,
    ) -> Result<SkillPreview> {
        let preview = self.preview_skill(spec).await?;
        if sha256.is_some_and(|s| !s.eq_ignore_ascii_case(&preview.sha256)) {
            return Err(Error::Invalid(
                "the skill changed since you looked at it; look at it again before installing"
                    .into(),
            ));
        }
        if preview.installed && !replace {
            return Err(Error::Invalid(format!(
                "you already have a skill named `{}`",
                preview.name
            )));
        }
        if preview.installed {
            // Its folder may have another name; don't leave a duplicate.
            self.delete_skill(&preview.name)?;
        }
        let dir = self.skills_dir();
        brainwashed_skills::save(&dir, &preview.source)
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let origin = SkillOrigin {
            url: preview.url.clone(),
            sha256: preview.sha256.clone(),
            community: preview.community,
            installed_at: now(),
        };
        std::fs::write(
            dir.join(&preview.name).join(ORIGIN_FILE),
            serde_json::to_vec_pretty(&origin)?,
        )?;
        self.reload_skills(true);
        Ok(preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_pages_become_raw_files() {
        assert_eq!(
            raw_skill_url("https://github.com/a/b/blob/main/skills/x/SKILL.md").unwrap(),
            "https://raw.githubusercontent.com/a/b/main/skills/x/SKILL.md"
        );
        assert_eq!(
            raw_skill_url("https://github.com/a/b/tree/main/skills/x/").unwrap(),
            "https://raw.githubusercontent.com/a/b/main/skills/x/SKILL.md"
        );
        assert_eq!(
            raw_skill_url("https://example.org/my-skill.md").unwrap(),
            "https://example.org/my-skill.md"
        );
        assert!(raw_skill_url("http://example.org/x.md").is_err());
        assert!(raw_skill_url("https://github.com/a/b").is_err());
    }

    #[test]
    fn hashes_are_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
