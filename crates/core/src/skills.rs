//! Skills on the engine: loading, hot reload, enabling, and adding the
//! relevant ones to each chat prompt.

use crate::community::{read_origin, sha256_hex, SkillOrigin};
use crate::engine::Event;
use crate::{Engine, Error, Result};
use brainwashed_runtime::chat::Delta;
use brainwashed_runtime::toolcalls::ToolCall;
use brainwashed_runtime::{Attachment, ChatMessage, ReplyStats, Role};
use brainwashed_skills::{fingerprint, LoadError, Registry, Router, SkillEntry, SKILL_FILE};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Longest skill body sent to the model. Small models lose track of long
/// instructions, so anything beyond this is cut.
pub const MAX_SKILL_CHARS: usize = brainwashed_skills::MAX_BODY_CHARS;

/// Skills that ship with BrainWashed. They are installed on every start
/// when missing, and replaced when this version of BrainWashed carries a
/// newer `version`. They can be turned off but not deleted.
pub const BUILTIN_SKILLS: &[(&str, &str)] = &[
    (
        "document-analyst",
        include_str!("../../../builtin-skills/document-analyst/SKILL.md"),
    ),
    (
        "writing-assistant",
        include_str!("../../../builtin-skills/writing-assistant/SKILL.md"),
    ),
];

/// Example skills earlier versions installed. They are removed unless
/// someone edited them.
const RETIRED: &[(&str, &str)] = &[
    (
        "meal-planner",
        include_str!("retired_skills/meal-planner.md"),
    ),
    (
        "email-writer",
        include_str!("retired_skills/email-writer.md"),
    ),
    (
        "explain-simply",
        include_str!("retired_skills/explain-simply.md"),
    ),
];

/// Puts the built-in skills in place and clears out the old examples.
fn install_builtin(dir: &Path) -> Result<()> {
    let same = |a: &str, b: &str| a.replace("\r\n", "\n").trim() == b.replace("\r\n", "\n").trim();
    for (name, source) in RETIRED {
        let folder = dir.join(name);
        if std::fs::read_to_string(folder.join(SKILL_FILE))
            .is_ok_and(|on_disk| same(&on_disk, source))
        {
            std::fs::remove_dir_all(&folder)?;
        }
    }
    for (name, source) in BUILTIN_SKILLS {
        let path = dir.join(name).join(SKILL_FILE);
        let shipped = brainwashed_skills::Skill::parse(source)
            .map(|s| s.meta.version)
            .unwrap_or(1);
        let installed = std::fs::read_to_string(&path)
            .ok()
            .map(|text| brainwashed_skills::Skill::parse(&text).map_or(0, |s| s.meta.version));
        if installed.map_or(true, |v| v < shipped) {
            std::fs::create_dir_all(dir.join(name))?;
            std::fs::write(&path, source)?;
        }
    }
    Ok(())
}

/// What the UI and phones see while a reply streams.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
    /// A tool the model called. Calls of MCP tools are run and their
    /// result follows as `tool_result`; `ask_user` carries a question for
    /// the person; anything else is only shown.
    #[serde(rename = "tool_call")]
    ToolCall(ToolCall),
    /// What a tool returned, after its `tool_call`.
    #[serde(rename = "tool_result")]
    ToolResult(crate::ToolResult),
    /// Sent once, after the answer.
    Stats(ReplyStats),
}

impl From<Delta> for ChatEvent {
    fn from(d: Delta) -> Self {
        match d {
            Delta::Content(text) => ChatEvent::Content { text },
            Delta::Reasoning(text) => ChatEvent::Reasoning { text },
            Delta::ToolCall(call) => ChatEvent::ToolCall(call),
            Delta::Stats(stats) => ChatEvent::Stats(stats),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    #[serde(flatten)]
    pub entry: SkillEntry,
    pub enabled: bool,
    /// Ships with BrainWashed: it can be turned off but not deleted.
    pub builtin: bool,
    /// Where it was installed from, for skills that weren't written here.
    pub origin: Option<SkillOrigin>,
    /// Changed on this computer since it was installed.
    pub modified: bool,
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
        std::fs::create_dir_all(dir)?;
        install_builtin(dir)?;
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

/// Whether a skill is one of the built-in ones, in its own folder.
fn is_builtin(entry: &SkillEntry) -> bool {
    BUILTIN_SKILLS.iter().any(|(name, _)| {
        entry.skill.name == *name
            && entry
                .path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|folder| folder == *name)
    })
}

/// What skills are matched against: the message, plus a note of what is
/// attached, so a document brings in skills that read documents.
fn routing_text(m: &ChatMessage) -> String {
    let mut text = m.content.clone();
    for a in &m.attachments {
        text.push_str(match a {
            Attachment::File { .. } => " (attached document)",
            Attachment::Image { .. } => " (attached picture)",
        });
    }
    text
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
                .map(|entry| {
                    let origin = read_origin(&entry.path);
                    let modified = origin.as_ref().is_some_and(|o| {
                        std::fs::read(&entry.path).is_ok_and(|b| sha256_hex(&b) != o.sha256)
                    });
                    SkillInfo {
                        enabled: !disabled.contains(&entry.skill.name),
                        builtin: is_builtin(&entry),
                        entry,
                        origin,
                        modified,
                    }
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
            if info.builtin {
                return Err(Error::Invalid(format!(
                    "{name} comes with BrainWashed and can't be deleted. Turn it off instead."
                )));
            }
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
        self.build_prompt_with(conversation, None)
    }

    /// `context` overrides the local model's context size, for cloud models.
    pub(crate) fn build_prompt_with(
        &self,
        conversation: &[ChatMessage],
        context: Option<u32>,
    ) -> (Vec<ChatMessage>, Vec<String>) {
        let settings = self.settings();
        let mut system = settings.system_prompt;

        let mut user_turns = conversation.iter().rev().filter(|m| m.role == Role::User);
        let last = user_turns.next().map(routing_text).unwrap_or_default();
        let previous = user_turns.next().map(routing_text);

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
            for skill in Router::new(enabled.iter().copied()).route(&last, previous.as_deref()) {
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
        let context = context.unwrap_or_else(|| {
            self.inner
                .loaded_context
                .read()
                .unwrap()
                .map_or(settings.context_size, |n| n.min(settings.context_size))
        });
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

/// Tokens a picture takes. Vision projectors use 256 to about 1000.
const IMAGE_TOKENS: usize = 768;

fn message_tokens(m: &ChatMessage) -> usize {
    estimate_tokens(&m.text()) + m.images().count() * IMAGE_TOKENS
}

/// Drops the oldest turns until the prompt leaves room for a reply. The
/// latest message is always kept.
fn fit_to_context(messages: &mut Vec<ChatMessage>, system: &str, context_size: u32) {
    let reply_reserve = (context_size as usize / 4).max(256);
    let budget = (context_size as usize).saturating_sub(reply_reserve);
    let mut total: usize =
        estimate_tokens(system) + messages.iter().map(message_tokens).sum::<usize>();
    while total > budget && messages.len() > 1 {
        let dropped = messages.remove(0);
        total -= message_tokens(&dropped);
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
