use brainwashed_core::{ChatMessage, Engine, EngineConfig, Event, Role};
use std::time::Duration;

fn engine(dir: &std::path::Path) -> Engine {
    Engine::new(EngineConfig::new(dir, "0.1.0")).unwrap()
}

fn user(text: &str) -> ChatMessage {
    ChatMessage::new(Role::User, text)
}

const RECIPE: &str = "---\nname: haiku\ndescription: Writes haiku poems.\ntriggers: [haiku]\n---\nUse 5-7-5 syllables.";

#[test]
fn bundled_skills_are_installed_on_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let names: Vec<String> = engine(dir.path())
        .skills()
        .skills
        .into_iter()
        .map(|s| s.entry.skill.name)
        .collect();
    assert_eq!(names, ["email-writer", "explain-simply", "meal-planner"]);
}

#[test]
fn deleted_bundled_skills_stay_deleted() {
    let dir = tempfile::tempdir().unwrap();
    engine(dir.path()).delete_skill("meal-planner").unwrap();
    assert_eq!(engine(dir.path()).skills().skills.len(), 2);
}

#[test]
fn relevant_skill_is_added_to_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let (prompt, used) =
        engine.build_prompt(&[user("Draft an email to my boss asking for Friday off")]);
    assert_eq!(used, ["email-writer"]);
    let system = &prompt[0].content;
    assert!(
        system.contains("- meal-planner: "),
        "index lists every enabled skill"
    );
    assert!(system.contains("## Skill: email-writer"));
    assert!(!system.contains("## Skill: meal-planner"));
}

#[test]
fn disabled_skills_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    engine.set_skill_enabled("email-writer", false).unwrap();
    let (prompt, used) = engine.build_prompt(&[user("Draft an email to my boss")]);
    assert!(used.is_empty());
    assert!(!prompt[0].content.contains("email-writer"));
    assert!(
        !engine
            .skills()
            .skills
            .iter()
            .find(|s| s.entry.skill.name == "email-writer")
            .unwrap()
            .enabled
    );
}

#[test]
fn save_rename_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    assert!(engine.save_skill("not a skill", None).is_err());

    assert_eq!(engine.save_skill(RECIPE, None).unwrap(), "haiku");
    let (_, used) = engine.build_prompt(&[user("write a haiku about rain")]);
    assert_eq!(used, ["haiku"]);

    let renamed = RECIPE.replace("name: haiku", "name: poems");
    engine.save_skill(&renamed, Some("haiku")).unwrap();
    let names: Vec<_> = engine
        .skills()
        .skills
        .into_iter()
        .map(|s| s.entry.skill.name)
        .collect();
    assert!(names.contains(&"poems".to_string()) && !names.contains(&"haiku".to_string()));
    assert_eq!(engine.skill_source("poems").unwrap(), renamed);

    engine.delete_skill("poems").unwrap();
    assert!(engine.skill_source("poems").is_err());
}

#[tokio::test]
async fn skills_hot_reload_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let mut events = engine.subscribe();
    let watcher = engine.watch_skills(Duration::from_millis(50));

    // Edited outside the app, e.g. in a text editor.
    let folder = engine.skills_dir().join("haiku");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("SKILL.md"), RECIPE).unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Event::SkillsChanged) = events.recv().await {
                break;
            }
        }
    })
    .await
    .expect("skills should reload");
    assert!(engine.skill_source("haiku").is_ok());
    watcher.abort();
}

#[test]
fn broken_skills_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let folder = engine.skills_dir().join("broken");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("SKILL.md"), "# no frontmatter").unwrap();
    engine.reload_skills(true);
    let list = engine.skills();
    assert_eq!(list.errors.len(), 1);
    assert_eq!(list.skills.len(), 3);
}

#[test]
fn old_turns_are_dropped_to_fit_the_context() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let mut s = engine.settings();
    s.context_size = 2048;
    engine.update_settings(s).unwrap();

    let long = "word ".repeat(400); // ~670 estimated tokens each
    let mut convo = Vec::new();
    for i in 0..6 {
        convo.push(user(&format!("{i} {long}")));
        convo.push(ChatMessage::new(Role::Assistant, long.clone()));
    }
    convo.push(user("latest question"));

    let (prompt, _) = engine.build_prompt(&convo);
    assert!(prompt.len() < convo.len(), "history should be trimmed");
    assert_eq!(prompt.last().unwrap().content, "latest question");
    assert_eq!(prompt[0].role, Role::System);
    assert_eq!(
        prompt[1].role,
        Role::User,
        "conversation must start with the user"
    );
}
