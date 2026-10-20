//! Decide whether a finished word was typed in the wrong layout.
//!
//! Each interpretation of the keystrokes (en and ru) gets a log-probability
//! from a language model that interpolates a word frequency list with a
//! letter-trigram model, so unknown words and typos still get a sensible
//! score. The word flips if the other layout wins by more than a threshold.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use crate::keymap::{self, Lang, Stroke};

static EN_WORDS: &[u8] = include_bytes!("../data/en.txt.gz");
static RU_WORDS: &[u8] = include_bytes!("../data/ru.txt.gz");

/// One-letter words. A single letter flips when it's a word in the other
/// layout and not in the current one (`d` → `в`, but `a` stays).
const EN_LETTERS: &[&str] = &["a", "i"];
const RU_LETTERS: &[&str] = &["а", "в", "и", "к", "о", "с", "у", "я"];

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Minimum log-probability lead (nats) of the other layout to flip.
    pub threshold: f64,
    /// Weight of the word list vs the letter model.
    pub lambda: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self { threshold: 4.0, lambda: 0.9 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Flip,
}

pub struct Detector {
    en: Model,
    ru: Model,
    /// Executables in $PATH: typed in en, they are never flipped (`jq`, `gcc`).
    commands: HashSet<String>,
    pub params: Params,
}

impl Detector {
    pub fn new(params: Params) -> std::io::Result<Self> {
        Ok(Self {
            en: Model::load(EN_WORDS)?,
            ru: Model::load(RU_WORDS)?,
            commands: path_commands(),
            params,
        })
    }

    fn model(&self, lang: Lang) -> &Model {
        match lang {
            Lang::En => &self.en,
            Lang::Ru => &self.ru,
        }
    }

    /// `cur` is the layout the word was typed in.
    pub fn decide(&self, word: &[Stroke], cur: Lang) -> Verdict {
        if word.is_empty() || deliberate_case(word) {
            return Verdict::Keep;
        }
        let other = cur.other();
        let text = |lang| normalize(word, lang);
        let (cur_text, other_text) = (text(cur), text(other));
        if cur == Lang::En && self.commands.contains(core(Lang::En, &cur_text)) {
            return Verdict::Keep;
        }

        if word.len() == 1 {
            let is_word = |lang, t: &str| match lang {
                Lang::En => EN_LETTERS.contains(&t),
                Lang::Ru => RU_LETTERS.contains(&t),
            };
            let flip = is_word(other, &other_text) && !is_word(cur, &cur_text);
            return if flip { Verdict::Flip } else { Verdict::Keep };
        }

        let lead = self.score(other, &other_text) - self.score(cur, &cur_text);
        if lead > self.params.threshold { Verdict::Flip } else { Verdict::Keep }
    }

    /// Log-probability of the word in `lang`.
    pub fn score(&self, lang: Lang, text: &str) -> f64 {
        let m = self.model(lang);
        let core = core(lang, text);
        if core.is_empty() {
            return f64::NEG_INFINITY;
        }
        let p_word = m.words.get(core).map_or(0.0, |&c| c / m.total);
        let p_letters = m.letters_logp(core).exp();
        let lambda = self.params.lambda;
        // Punctuation around an en word (`hello.`) is fine, but it's less
        // likely than a clean word: charge one unseen letter per stripped char.
        let stripped = (text.chars().count() - core.chars().count()) as f64;
        (lambda * p_word + (1.0 - lambda) * p_letters).ln() - stripped * m.unseen_penalty
    }
}

/// Lowercased text of the keystrokes in `lang`, `ё` folded into `е`.
fn normalize(word: &[Stroke], lang: Lang) -> String {
    let (en, ru) = keymap::render(word);
    match lang {
        Lang::En => en.to_lowercase(),
        Lang::Ru => ru.to_lowercase().replace('ё', "е"),
    }
}

/// The word without surrounding punctuation. In ru every key is a letter;
/// in en the keys `[ ] ; ' , . \`` are punctuation.
fn core(lang: Lang, text: &str) -> &str {
    match lang {
        Lang::Ru => text,
        Lang::En => text.trim_matches(|c: char| !c.is_ascii_lowercase()),
    }
}

/// Capitals beyond the first letter mean the word was typed on purpose:
/// `iPhone`, `camelCase`, and ALL CAPS (`USA`, `ФРГ`, mostly abbreviations).
/// Only letter keys count: Shift on `;` or `'` is `:` or `"` in en.
fn deliberate_case(word: &[Stroke]) -> bool {
    let letters: Vec<&Stroke> = word.iter().filter(|s| keymap::is_letter_in_both(s.key)).collect();
    let shifted = letters.iter().filter(|s| s.shift).count();
    let capitalized = shifted == 1 && letters[0].shift;
    shifted != 0 && !capitalized
}

/// Names of the executables in $PATH, lowercased letters only.
fn path_commands() -> HashSet<String> {
    let Some(path) = std::env::var_os("PATH") else { return HashSet::new() };
    std::env::split_paths(&path)
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.len() >= 2 && n.chars().all(|c| c.is_ascii_lowercase()))
        .collect()
}

struct Model {
    words: HashMap<Box<str>, f64>,
    total: f64,
    trigrams: HashMap<[char; 3], f64>,
    contexts: HashMap<[char; 2], f64>,
    alphabet: f64,
    unseen_penalty: f64,
}

const BEGIN: char = '^';
const END: char = '$';
/// Add-k smoothing for the letter model.
const K: f64 = 0.01;

impl Model {
    fn load(gz: &[u8]) -> std::io::Result<Self> {
        let mut text = String::new();
        flate2::read::GzDecoder::new(gz).read_to_string(&mut text)?;

        let mut words = HashMap::new();
        let mut total = 0.0;
        let mut trigrams: HashMap<[char; 3], f64> = HashMap::new();
        let mut contexts: HashMap<[char; 2], f64> = HashMap::new();
        let mut letters = std::collections::HashSet::new();

        for line in text.lines() {
            let Some((word, count)) = line.split_once(' ') else { continue };
            let count: f64 = count.parse().map_err(std::io::Error::other)?;
            total += count;
            words.insert(word.into(), count);
            // Weight the letter model by log frequency: it scores rare and
            // unknown words, so it should learn word shapes, not just the
            // handful of most frequent words.
            let weight = (1.0 + count).ln();
            let chars = padded(word);
            letters.extend(word.chars());
            for w in chars.windows(3) {
                *trigrams.entry([w[0], w[1], w[2]]).or_default() += weight;
                *contexts.entry([w[0], w[1]]).or_default() += weight;
            }
        }
        let alphabet = letters.len() as f64 + 1.0; // + END
        Ok(Self { words, total, trigrams, contexts, alphabet, unseen_penalty: alphabet.ln() })
    }

    fn letters_logp(&self, word: &str) -> f64 {
        padded(word)
            .windows(3)
            .map(|w| {
                let tri = self.trigrams.get(&[w[0], w[1], w[2]]).copied().unwrap_or(0.0);
                let ctx = self.contexts.get(&[w[0], w[1]]).copied().unwrap_or(0.0);
                ((tri + K) / (ctx + K * self.alphabet)).ln()
            })
            .sum()
    }
}

fn padded(word: &str) -> Vec<char> {
    let mut chars = vec![BEGIN, BEGIN];
    chars.extend(word.chars());
    chars.push(END);
    chars
}
