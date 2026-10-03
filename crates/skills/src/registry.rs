//! Loads every skill in a folder and keeps track of the ones that failed.

use crate::{Skill, SkillError};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const SKILL_FILE: &str = "SKILL.md";

#[derive(Debug, Clone, Serialize)]
pub struct SkillEntry {
    #[serde(flatten)]
    pub skill: SkillSummary,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

/// All skills found in a folder: each skill is `<folder>/<name>/SKILL.md`.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    skills: Vec<(Skill, PathBuf)>,
    errors: Vec<LoadError>,
}

impl Registry {
    pub fn load(dir: &Path) -> Registry {
        let mut reg = Registry::default();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return reg;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join(SKILL_FILE))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            match Skill::load(&path) {
                Ok(skill) if reg.get(&skill.meta.name).is_some() => reg.errors.push(LoadError {
                    message: format!("another skill is already named `{}`", skill.meta.name),
                    path,
                }),
                Ok(skill) => reg.skills.push((skill, path)),
                Err(e) => reg.errors.push(LoadError {
                    path,
                    message: e.to_string(),
                }),
            }
        }
        reg
    }

    pub fn skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.iter().map(|(s, _)| s)
    }

    pub fn entries(&self) -> Vec<SkillEntry> {
        self.skills
            .iter()
            .map(|(s, path)| SkillEntry {
                skill: SkillSummary {
                    name: s.meta.name.clone(),
                    description: s.meta.description.clone(),
                    triggers: s.meta.triggers.clone(),
                    version: s.meta.version,
                },
                path: path.clone(),
            })
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills().find(|s| s.meta.name == name)
    }

    pub fn errors(&self) -> &[LoadError] {
        &self.errors
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

/// A cheap summary of a skills folder that changes whenever a skill file is
/// added, removed or edited. Used to hot-reload without a file watcher.
pub fn fingerprint(dir: &Path) -> Vec<(PathBuf, u64, Option<SystemTime>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join(SKILL_FILE))
        .filter_map(|p| {
            let meta = std::fs::metadata(&p).ok()?;
            Some((p, meta.len(), meta.modified().ok()))
        })
        .collect();
    out.sort();
    out
}

/// Writes a skill to `<dir>/<name>/SKILL.md` after checking it parses.
pub fn save(dir: &Path, source: &str) -> Result<Skill, SkillError> {
    let skill = Skill::parse(source)?;
    let folder = dir.join(&skill.meta.name);
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join(SKILL_FILE), source)?;
    Ok(skill)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, folder: &str, src: &str) {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
        std::fs::write(dir.join(folder).join(SKILL_FILE), src).unwrap();
    }

    #[test]
    fn loads_good_skills_and_reports_bad_ones() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a", "---\nname: a\ndescription: A.\n---\nbody");
        write(dir.path(), "b", "no frontmatter");
        write(dir.path(), "c", "---\nname: a\ndescription: dup\n---\nbody");
        std::fs::write(dir.path().join("loose.md"), "ignored").unwrap();

        let reg = Registry::load(dir.path());
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("a").unwrap().body, "body");
        assert_eq!(reg.errors().len(), 2);
    }

    #[test]
    fn fingerprint_changes_on_edit() {
        let dir = tempfile::tempdir().unwrap();
        let before = fingerprint(dir.path());
        write(dir.path(), "a", "---\nname: a\ndescription: A.\n---\nbody");
        assert_ne!(before, fingerprint(dir.path()));
    }

    #[test]
    fn save_validates_first() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save(dir.path(), "nope").is_err());
        save(dir.path(), "---\nname: ok\ndescription: d\n---\nb").unwrap();
        assert!(dir.path().join("ok").join(SKILL_FILE).exists());
    }
}
