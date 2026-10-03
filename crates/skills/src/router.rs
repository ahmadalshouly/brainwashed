//! Picks which skills are relevant to a message.
//!
//! A small model gets confused by long prompts, so only the one or two most
//! relevant skills are added in full. Matching is lexical: an explicit trigger
//! phrase wins, otherwise skills are scored by the distinctive words they share
//! with the message.

use crate::Skill;
use std::collections::{HashMap, HashSet};

pub const MAX_ACTIVE_SKILLS: usize = 2;

const TRIGGER_SCORE: f32 = 3.0;
const MIN_KEYWORD_SCORE: f32 = 1.0;
const MIN_KEYWORD_MATCHES: usize = 2;
/// How much the previous user message counts, so follow-ups keep their skill.
const PREVIOUS_WEIGHT: f32 = 0.5;

pub struct Router<'a> {
    skills: Vec<Indexed<'a>>,
    /// How many skills each word appears in.
    doc_freq: HashMap<String, usize>,
}

struct Indexed<'a> {
    skill: &'a Skill,
    words: HashSet<String>,
    triggers: Vec<Vec<String>>,
}

impl<'a> Router<'a> {
    pub fn new(skills: impl IntoIterator<Item = &'a Skill>) -> Self {
        let skills: Vec<Indexed> = skills
            .into_iter()
            .map(|skill| {
                let m = &skill.meta;
                let text = format!(
                    "{} {} {}",
                    m.name.replace('-', " "),
                    m.description,
                    m.triggers.join(" ")
                );
                Indexed {
                    skill,
                    words: words(&text).into_iter().collect(),
                    triggers: m
                        .triggers
                        .iter()
                        .map(|t| words(t))
                        .filter(|t| !t.is_empty())
                        .collect(),
                }
            })
            .collect();
        let mut doc_freq = HashMap::new();
        for s in &skills {
            for w in &s.words {
                *doc_freq.entry(w.clone()).or_insert(0) += 1;
            }
        }
        Router { skills, doc_freq }
    }

    /// Returns up to [`MAX_ACTIVE_SKILLS`] skills for `message`, best first.
    pub fn route(&self, message: &str, previous: Option<&str>) -> Vec<&'a Skill> {
        let current = words(message);
        let previous = previous.map(words).unwrap_or_default();
        let mut scored: Vec<(f32, &'a Skill)> = self
            .skills
            .iter()
            .filter_map(|s| {
                let now = self.score(s, &current);
                let before = self.score(s, &previous);
                let total = now.unwrap_or(0.0) + PREVIOUS_WEIGHT * before.unwrap_or(0.0);
                (now.is_some() || before.is_some()).then_some((total, s.skill))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.meta.name.cmp(&b.1.meta.name))
        });
        scored
            .into_iter()
            .take(MAX_ACTIVE_SKILLS)
            .map(|(_, s)| s)
            .collect()
    }

    /// `None` when the skill isn't relevant enough to include.
    fn score(&self, s: &Indexed, message: &[String]) -> Option<f32> {
        if message.is_empty() {
            return None;
        }
        if s.triggers.iter().any(|t| contains_phrase(message, t)) {
            return Some(TRIGGER_SCORE + self.keyword_score(s, message).0);
        }
        let (score, matches) = self.keyword_score(s, message);
        (matches >= MIN_KEYWORD_MATCHES && score >= MIN_KEYWORD_SCORE).then_some(score)
    }

    fn keyword_score(&self, s: &Indexed, message: &[String]) -> (f32, usize) {
        let n = self.skills.len() as f32;
        let unique: HashSet<&String> = message.iter().collect();
        let mut score = 0.0;
        let mut matches = 0;
        for w in unique {
            if s.words.contains(w) {
                let df = *self.doc_freq.get(w).unwrap_or(&1) as f32;
                score += (1.0 + n / df).ln();
                matches += 1;
            }
        }
        (score, matches)
    }
}

fn contains_phrase(haystack: &[String], phrase: &[String]) -> bool {
    haystack.windows(phrase.len()).any(|w| w == phrase)
}

const STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "do", "for",
    "from", "how", "i", "if", "in", "is", "it", "its", "me", "my", "of", "on", "or", "please",
    "so", "that", "the", "this", "to", "was", "we", "what", "when", "which", "who", "will", "with",
    "would", "you", "your",
];

/// Lowercased content words with a light plural/verb-ending trim.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\''))
        .filter(|w| !w.is_empty() && !STOPWORDS.contains(w))
        .map(stem)
        .collect()
}

fn stem(word: &str) -> String {
    for suffix in ["ing", "ies", "es", "ed", "s"] {
        if let Some(root) = word.strip_suffix(suffix) {
            if root.chars().count() >= 3 {
                return if suffix == "ies" {
                    format!("{root}y")
                } else {
                    root.to_string()
                };
            }
        }
    }
    word.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, description: &str, triggers: &[&str]) -> Skill {
        let src = format!(
            "---\nname: {name}\ndescription: {description}\ntriggers: [{}]\n---\nbody",
            triggers.join(", ")
        );
        Skill::parse(&src).unwrap()
    }

    fn skills() -> Vec<Skill> {
        vec![
            skill(
                "meal-planner",
                "Plans weekly meals from ingredients on hand.",
                &["meal plan", "what should I cook"],
            ),
            skill(
                "email-writer",
                "Drafts and rewrites emails in the right tone.",
                &["write an email", "draft an email"],
            ),
            skill(
                "explain-simply",
                "Explains hard topics in plain words.",
                &["eli5", "explain simply"],
            ),
        ]
    }

    fn names(picked: Vec<&Skill>) -> Vec<&str> {
        picked.iter().map(|s| s.meta.name.as_str()).collect()
    }

    #[test]
    fn trigger_phrases_select_skills() {
        let s = skills();
        let r = Router::new(&s);
        assert_eq!(
            names(r.route("Can you make a meal plan for next week?", None)),
            ["meal-planner"]
        );
        assert_eq!(
            names(r.route("Please write an email to my landlord", None)),
            ["email-writer"]
        );
        assert_eq!(
            names(r.route("ELI5 quantum computing", None)),
            ["explain-simply"]
        );
    }

    #[test]
    fn keywords_select_without_triggers() {
        let s = skills();
        let r = Router::new(&s);
        assert_eq!(
            names(r.route("I have rice and eggs, plan my meals", None)),
            ["meal-planner"]
        );
        assert_eq!(
            names(r.route("rewrite these emails so the tone is friendlier", None)),
            ["email-writer"]
        );
    }

    #[test]
    fn unrelated_messages_select_nothing() {
        let s = skills();
        let r = Router::new(&s);
        assert!(r.route("What's the capital of France?", None).is_empty());
        assert!(r.route("hi", None).is_empty());
        assert!(r.route("", None).is_empty());
    }

    #[test]
    fn follow_ups_keep_the_previous_skill() {
        let s = skills();
        let r = Router::new(&s);
        let picked = r.route("Make it vegetarian", Some("meal plan for 3 days please"));
        assert_eq!(names(picked), ["meal-planner"]);
    }

    #[test]
    fn caps_the_number_of_skills() {
        let s = skills();
        let r = Router::new(&s);
        let picked = r.route("eli5 then write an email with a meal plan", None);
        assert_eq!(picked.len(), MAX_ACTIVE_SKILLS);
    }

    #[test]
    fn stems_simple_endings() {
        assert_eq!(
            words("Planning meals, cooked stories"),
            ["plann", "meal", "cook", "story"]
        );
    }
}
