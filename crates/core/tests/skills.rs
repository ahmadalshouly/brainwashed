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
fn builtin_skills_are_always_installed() {
    let dir = tempfile::tempdir().unwrap();
    let names = |e: &Engine| -> Vec<String> {
        e.skills()
            .skills
            .into_iter()
            .map(|s| s.entry.skill.name)
            .collect()
    };
    let first = engine(dir.path());
    assert_eq!(names(&first), ["document-analyst", "writing-assistant"]);
    assert!(first.skills().skills.iter().all(|s| s.builtin));

    // They can't be deleted, and come back if removed by hand.
    assert!(first.delete_skill("writing-assistant").is_err());
    std::fs::remove_dir_all(dir.path().join("skills/writing-assistant")).unwrap();
    assert_eq!(
        names(&engine(dir.path())),
        ["document-analyst", "writing-assistant"]
    );
}

#[test]
fn old_example_skills_are_removed_unless_edited() {
    let dir = tempfile::tempdir().unwrap();
    let skills = dir.path().join("skills");
    let old = "---\nname: meal-planner\ndescription: Plans weekly meals from ingredients on hand and dietary goals.\ntriggers: [meal plan, what should I cook, groceries]\nversion: 1\n---\n\nWhen the user asks for help planning meals:\n\n1. If they have not listed ingredients they already have, ask for them in one short question.\n2. Ask about dietary restrictions only if none were mentioned.\n3. Propose meals for the requested days as a short list: day, dish, main ingredients.\n4. End with a grocery list of only the items they still need to buy.\n\nKeep the answer under 200 words unless the user asks for recipes.\n";
    std::fs::create_dir_all(skills.join("meal-planner")).unwrap();
    std::fs::write(skills.join("meal-planner/SKILL.md"), old).unwrap();
    let edited = "---\nname: explain-simply\ndescription: My own version.\n---\nMine.";
    std::fs::create_dir_all(skills.join("explain-simply")).unwrap();
    std::fs::write(skills.join("explain-simply/SKILL.md"), edited).unwrap();

    let names: Vec<String> = engine(dir.path())
        .skills()
        .skills
        .into_iter()
        .map(|s| s.entry.skill.name)
        .collect();
    assert_eq!(
        names,
        ["document-analyst", "explain-simply", "writing-assistant"]
    );
}

#[test]
fn builtin_skills_are_upgraded_to_newer_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("skills/writing-assistant/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "---\nname: writing-assistant\ndescription: old\nversion: 0\n---\nold",
    )
    .unwrap();
    engine(dir.path());
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("Tone guide"));
}

#[test]
fn relevant_skill_is_added_to_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let (prompt, used) =
        engine.build_prompt(&[user("Draft an email to my boss asking for Friday off")]);
    assert_eq!(used, ["writing-assistant"]);
    let system = &prompt[0].content;
    assert!(
        system.contains("- document-analyst: "),
        "index lists every enabled skill"
    );
    assert!(system.contains("## Skill: writing-assistant"));
    assert!(!system.contains("## Skill: document-analyst"));
}

#[test]
fn an_attached_document_brings_in_the_document_skill() {
    let dir = tempfile::tempdir().unwrap();
    let mut message = user("What are the payment terms?");
    message
        .attachments
        .push(brainwashed_core::Attachment::File {
            name: "lease.pdf".into(),
            text: "Rent is due on the first of each month.".into(),
        });
    let (_, used) = engine(dir.path()).build_prompt(&[message]);
    assert_eq!(used, ["document-analyst"]);
}

#[test]
fn disabled_skills_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    engine
        .set_skill_enabled("writing-assistant", false)
        .unwrap();
    let (prompt, used) = engine.build_prompt(&[user("Draft an email to my boss")]);
    assert!(used.is_empty());
    assert!(!prompt[0].content.contains("writing-assistant"));
    assert!(
        !engine
            .skills()
            .skills
            .iter()
            .find(|s| s.entry.skill.name == "writing-assistant")
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
    assert_eq!(list.skills.len(), 2);
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
