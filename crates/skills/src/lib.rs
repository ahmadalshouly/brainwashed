//! Skills are markdown files with YAML frontmatter that the host injects into
//! the model's prompt at inference time. See `builtin-skills/` for the format.

mod registry;
mod router;

pub use registry::{fingerprint, save, LoadError, Registry, SkillEntry, SkillSummary, SKILL_FILE};
pub use router::{Router, MAX_ACTIVE_SKILLS};

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub meta: SkillMeta,
    /// Markdown instructions after the frontmatter.
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("skill file must start with a `---` frontmatter block")]
    MissingFrontmatter,
    #[error("invalid frontmatter: {0}")]
    InvalidFrontmatter(#[from] serde_norway::Error),
    #[error("skill `name` must be lowercase letters, digits and dashes, got `{0}`")]
    InvalidName(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Skill {
    pub fn parse(source: &str) -> Result<Self, SkillError> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let rest = source
            .strip_prefix("---\n")
            .or_else(|| source.strip_prefix("---\r\n"))
            .ok_or(SkillError::MissingFrontmatter)?;
        let end = rest.find("\n---").ok_or(SkillError::MissingFrontmatter)?;
        let meta: SkillMeta = serde_norway::from_str(&rest[..end])?;
        if meta.name.is_empty()
            || !meta
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(SkillError::InvalidName(meta.name));
        }
        let after = &rest[end + "\n---".len()..];
        let body = after
            .trim_start_matches(['\r', '\n'])
            .trim_end()
            .to_string();
        Ok(Skill { meta, body })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SkillError> {
        Self::parse(&std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let skill = Skill::parse(
            "---\nname: meal-planner\ndescription: Plans meals.\ntriggers: [meal plan]\n---\n\nStep one.\n",
        )
        .unwrap();
        assert_eq!(skill.meta.name, "meal-planner");
        assert_eq!(skill.meta.triggers, vec!["meal plan"]);
        assert_eq!(skill.meta.version, 1);
        assert_eq!(skill.body, "Step one.");
    }

    #[test]
    fn rejects_missing_frontmatter() {
        assert!(matches!(
            Skill::parse("# Just markdown"),
            Err(SkillError::MissingFrontmatter)
        ));
    }

    #[test]
    fn rejects_bad_names() {
        let err = Skill::parse("---\nname: Meal Planner\ndescription: x\n---\nbody").unwrap_err();
        assert!(matches!(err, SkillError::InvalidName(_)));
    }

    #[test]
    fn builtin_skills_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../builtin-skills");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path().join("SKILL.md");
            if path.exists() {
                Skill::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                count += 1;
            }
        }
        assert!(count > 0, "no built-in skills found");
    }
}
