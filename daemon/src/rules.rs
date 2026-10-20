//! Learned rules: words the user keeps flipping by hand.
//!
//! A manual flip (Alt+Space) votes for the layout the word was flipped to;
//! undoing an auto-correction votes for the layout it was typed in. At
//! `learn_threshold` votes (config) the word becomes a rule, which beats the
//! detector.
//! Words are keyed by their keys (`keymap::key_string`), so `next` and
//! `туче` are the same rule.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::keymap::{self, Lang, Stroke};

/// Votes that never became a rule are forgotten after this long.
const VOTE_TTL_SECS: u64 = 30 * 24 * 3600;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub keys: String,
    /// The layout the word belongs to.
    pub lang: Lang,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Vote {
    keys: String,
    lang: Lang,
    count: u32,
    updated_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    rules: Vec<Rule>,
    #[serde(default)]
    votes: Vec<Vote>,
}

pub struct Rules {
    path: Option<PathBuf>,
    store: Store,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("layoutless").join("rules.json"))
}

impl Rules {
    /// Read the rules file. Missing gives no rules; broken is reported and
    /// left untouched until the next change overwrites it.
    pub fn load() -> Self {
        let path = path();
        let store = path
            .as_ref()
            .and_then(|p| match std::fs::read_to_string(p) {
                Ok(text) => serde_json::from_str(&text)
                    .map_err(|e| eprintln!("{}: {e}; starting with no rules", p.display()))
                    .ok(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    eprintln!("{}: {e}", p.display());
                    None
                }
            })
            .unwrap_or_default();
        let mut rules = Self { path, store };
        rules.expire_votes();
        rules
    }

    pub fn list(&self) -> &[Rule] {
        &self.store.rules
    }

    pub fn lookup(&self, word: &[Stroke]) -> Option<Lang> {
        let keys = keymap::key_string(word)?;
        self.store.rules.iter().find(|r| r.keys == keys).map(|r| r.lang)
    }

    /// Count a vote for `word` belonging to `lang`. Returns the rule if this
    /// vote reached `threshold` and created or changed one.
    pub fn vote(&mut self, word: &[Stroke], lang: Lang, threshold: u32) -> Option<Rule> {
        let keys = keymap::key_string(word)?;
        if self.store.rules.iter().any(|r| r.keys == keys && r.lang == lang) {
            return None;
        }
        let count = match self.store.votes.iter_mut().find(|v| v.keys == keys && v.lang == lang) {
            Some(v) => {
                v.count += 1;
                v.updated_at = now();
                v.count
            }
            None => {
                self.store.votes.push(Vote { keys: keys.clone(), lang, count: 1, updated_at: now() });
                1
            }
        };
        let learned = (count >= threshold.max(1)).then(|| self.set(&keys, lang));
        self.save();
        learned
    }

    /// Take back a vote (the flip was toggled back right away).
    pub fn unvote(&mut self, word: &[Stroke], lang: Lang) {
        let Some(keys) = keymap::key_string(word) else { return };
        if let Some(i) = self.store.votes.iter().position(|v| v.keys == keys && v.lang == lang) {
            self.store.votes[i].count -= 1;
            if self.store.votes[i].count == 0 {
                self.store.votes.remove(i);
            }
            self.save();
        }
    }

    /// Create or replace the rule for `keys` and forget its votes.
    pub fn set(&mut self, keys: &str, lang: Lang) -> Rule {
        self.store.rules.retain(|r| r.keys != keys);
        self.store.votes.retain(|v| v.keys != keys);
        let rule = Rule { keys: keys.to_string(), lang, created_at: now() };
        self.store.rules.push(rule.clone());
        self.save();
        rule
    }

    pub fn remove(&mut self, keys: &str) -> bool {
        let before = self.store.rules.len();
        self.store.rules.retain(|r| r.keys != keys);
        self.store.votes.retain(|v| v.keys != keys);
        let removed = self.store.rules.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    fn expire_votes(&mut self) {
        let cutoff = now().saturating_sub(VOTE_TTL_SECS);
        self.store.votes.retain(|v| v.updated_at >= cutoff);
    }

    /// Write atomically: a crash mid-write never leaves a broken file.
    fn save(&mut self) {
        self.expire_votes();
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            let json = serde_json::to_string_pretty(&self.store).map_err(std::io::Error::other)?;
            std::fs::write(&tmp, json + "\n")?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            eprintln!("{}: {e}", path.display());
        }
    }
}

/// The rule as shown to people: both spellings and the layout it belongs to.
pub fn describe(rule: &Rule) -> serde_json::Value {
    let strokes = keymap::from_key_string(&rule.keys).unwrap_or_default();
    let (en, ru) = keymap::render(&strokes);
    serde_json::json!({ "keys": rule.keys, "en": en, "ru": ru, "lang": rule.lang, "created_at": rule.created_at })
}
