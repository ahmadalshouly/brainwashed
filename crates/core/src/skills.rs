//! Skills on the engine: loading, hot reload, enabling, and adding the
//! relevant ones to each chat prompt.

use crate::engine::Event;
use crate::{Engine, Error, Result};
use brainwashed_runtime::chat::Delta;
use brainwashed_runtime::{ChatMessage, Role};
use brainwashed_skills::{fingerprint, LoadError, Registry, Router, SkillEntry, SKILL_FILE};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Longest skill body sent to the model. Small models lose track of long
/// instructions, so anything beyond this is cut.
pub const MAX_SKILL_CHARS: usize = 6000;

/// Skills that ship with BrainWashed and are copied in on first run.
const BUNDLED: &[(&str, &str)] = &[
    (
        "meal-planner",
        include_str!("../../../skills-examples/meal-planner/SKILL.md"),
    ),
    (
        "email-writer",
        include_str!("../../../skills-examples/email-writer/SKILL.md"),
    ),
    (
        "explain-simply",
        include_str!("../../../skills-examples/explain-simply/SKILL.md"),
    ),
];

/// What the UI and phones see while a reply streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ChatEvent {
    /// Skills added to the prompt for this reply. Always sent first.
    Skills {
        names: Vec<String>,
    },
    Content {
        text: String,
    },
    Reasoning {
        text: String,
    },
}

impl From<Delta> for ChatEvent {
    fn from(d: Delta) -> Self {
        match d {
            Delta::Content(text) => ChatEvent::Content { text },
            Delta::Reasoning(text) => ChatEvent::Reasoning { text },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    #[serde(flatten)]
    pub entry: SkillEntry,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillList {
    pub dir: PathBuf,
    pub skills: Vec<SkillInfo>,
    pub errors: Vec<LoadError>,
}

pub(crate) struct SkillState {
    dir: PathBuf,
    registry: Registry,
    fingerprint: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
}

impl SkillState {
    pub(crate) fn open(dir: &Path) -> Result<Self> {
        if !dir.exists() {
            for (name, source) in BUNDLED {
                std::fs::create_dir_all(dir.join(name))?;
                std::fs::write(dir.join(name).join(SKILL_FILE), source)?;
            }
        }
        Ok(SkillState {
            dir: dir.to_path_buf(),
            registry: Registry::load(dir),
            fingerprint: fingerprint(dir),
        })
    }

    /// Reloads if anything changed on disk. Returns whether it did.
    fn refresh(&mut self) -> bool {
        let now = fingerprint(&self.dir);
        if now == self.fingerprint {
            return false;
        }
        self.registry = Registry::load(&self.dir);
        self.fingerprint = now;
        true
    }
}

impl Engine {
    pub fn skills_dir(&self) -> PathBuf {
        self.inner.skills.read().unwrap().dir.clone()
    }

    pub fn skills(&self) -> SkillList {
        let disabled = self.settings().disabled_skills;
        let state = self.inner.skills.read().unwrap();
        SkillList {
            dir: state.dir.clone(),
            skills: state
                .registry
                .entries()
                .into_iter()
                .map(|entry| SkillInfo {
                    enabled: !disabled.contains(&entry.skill.name),
                    entry,
                })
                .collect(),
            errors: state.registry.errors().to_vec(),
        }
    }

    /// The raw SKILL.md text, for editing.
    pub fn skill_source(&self, name: &str) -> Result<String> {
        let path = self
            .skills()
            .skills
            .into_iter()
            .find(|s| s.entry.skill.name == name)
            .map(|s| s.entry.path)
            .ok_or_else(|| Error::Invalid(format!("no skill named `{name}`")))?;
        Ok(std::fs::read_to_string(path)?)
    }

    /// Creates or replaces a skill. When `previous_name` differs from the new
    /// name, the old skill is removed so a rename doesn't leave a copy behind.
    pub fn save_skill(&self, source: &str, previous_name: Option<&str>) -> Result<String> {
        let dir = self.skills_dir();
        let skill =
            brainwashed_skills::save(&dir, source).map_err(|e| Error::Invalid(e.to_string()))?;
        let name = skill.meta.name;
        if let Some(old) = previous_name.filter(|old| *old != name) {
            self.delete_skill(old)?;
        }
        self.reload_skills(true);
        Ok(name)
    }

    pub fn delete_skill(&self, name: &str) -> Result<()> {
        if let Some(info) = self
            .skills()
            .skills
            .into_iter()
            .find(|s| s.entry.skill.name == name)
        {
            if let Some(folder) = info.entry.path.parent() {
                std::fs::remove_dir_all(folder)?;
            }
        }
        self.reload_skills(true);
        Ok(())
    }

    pub fn set_skill_enabled(&self, name: &str, enabled: bool) -> Result<()> {
        let mut settings = self.settings();
        settings.disabled_skills.retain(|n| n != name);
        if !enabled {
            settings.disabled_skills.push(name.to_string());
        }
        self.update_settings(settings)?;
        self.emit(Event::SkillsChanged);
        Ok(())
    }

    /// Re-reads the skills folder if it changed (or always, with `force`).
    pub fn reload_skills(&self, force: bool) -> bool {
        let changed = {
            let mut state = self.inner.skills.write().unwrap();
            if force {
                state.fingerprint.clear();
            }
            state.refresh()
        };
        if changed {
            self.emit(Event::SkillsChanged);
        }
        changed
    }

    /// Polls the skills folder so edits made in any text editor apply without
    /// restarting. Must be called from inside a Tokio runtime.
    pub fn watch_skills(&self, every: Duration) -> tokio::task::JoinHandle<()> {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                tick.tick().await;
                if engine.reload_skills(false) {
                    tracing::info!("skills reloaded");
                }
            }
        })
    }

    /// Builds the prompt the model actually sees, and names the skills used.
    pub fn build_prompt(&self, conversation: &[ChatMessage]) -> (Vec<ChatMessage>, Vec<String>) {
        let settings = self.settings();
        let mut system = settings.system_prompt;

        let mut user_turns = conversation.iter().rev().filter(|m| m.role == Role::User);
        let last = user_turns.next().map(|m| m.content.as_str()).unwrap_or("");
        let previous = user_turns.next().map(|m| m.content.as_str());

        let state = self.inner.skills.read().unwrap();
        let enabled: Vec<_> = state
            .registry
            .skills()
            .filter(|s| !settings.disabled_skills.contains(&s.meta.name))
            .collect();
        let mut used = Vec::new();
        if !enabled.is_empty() {
            system.push_str(
                "\n\n# Skills\nYou have these skills. When one fits the request, its instructions appear below and you must follow them.\n",
            );
            for s in &enabled {
                system.push_str(&format!("- {}: {}\n", s.meta.name, s.meta.description));
            }
            for skill in Router::new(enabled.iter().copied()).route(last, previous) {
                system.push_str(&format!(
                    "\n## Skill: {}\n{}\n",
                    skill.meta.name,
                    truncate(&skill.body)
                ));
                used.push(skill.meta.name.clone());
            }
        }
        drop(state);

        // A client-supplied system message is appended to ours, never replaces it.
        let mut messages = Vec::with_capacity(conversation.len() + 1);
        for m in conversation {
            if m.role == Role::System {
                system.push_str("\n\n");
                system.push_str(&m.content);
            } else {
                messages.push(m.clone());
            }
        }
        let context = self
            .inner
            .loaded_context
            .read()
            .unwrap()
            .map_or(settings.context_size, |n| n.min(settings.context_size));
        fit_to_context(&mut messages, &system, context);
        messages.insert(0, ChatMessage::new(Role::System, system));
        (messages, used)
    }
}

/// Rough token count. Real tokenizers average 3-4 characters per token for
/// English, so this errs on the side of dropping history early.
fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 3 + 4
}

/// Drops the oldest turns until the prompt leaves room for a reply. The
/// latest message is always kept.
fn fit_to_context(messages: &mut Vec<ChatMessage>, system: &str, context_size: u32) {
    let reply_reserve = (context_size as usize / 4).max(256);
    let budget = (context_size as usize).saturating_sub(reply_reserve);
    let mut total: usize = estimate_tokens(system)
        + messages
            .iter()
            .map(|m| estimate_tokens(&m.content))
            .sum::<usize>();
    while total > budget && messages.len() > 1 {
        let dropped = messages.remove(0);
        total -= estimate_tokens(&dropped.content);
    }
    // Chat templates expect the conversation to start with the user.
    while messages.len() > 1 && messages[0].role != Role::User {
        messages.remove(0);
    }
}

fn truncate(body: &str) -> &str {
    match body.char_indices().nth(MAX_SKILL_CHARS) {
        Some((i, _)) => &body[..i],
        None => body,
    }
}
