//! `layoutless eval`: run the detector over text files and count mistakes.
//!
//! The text is "typed" key by key in each layout and cut into words exactly
//! like the daemon does (see `keymap::classify`): `,` and `.` stick to en
//! words, the ru `.` and `,` end the word, digits disable correction. Chars
//! the layout doesn't have (Latin in ru, `«`, `—`) reset the word.
//!
//! Every word is tested twice: typed in its own layout (it must be kept) and
//! typed in the other layout (it must be flipped).

use std::collections::HashMap;
use std::time::Instant;

use crate::detect::{Detector, Params, Verdict};
use crate::keymap::{self, KeyClass, Lang, Stroke};

const USAGE: &str = "usage: layoutless eval [--threshold N] [--lambda N] [--show N] FILE...";

#[derive(Default)]
struct Tally {
    words: usize,
    false_flips: usize,
    misses: usize,
    false_flip_words: HashMap<String, usize>,
    missed_words: HashMap<String, usize>,
}

pub fn run(args: &[String]) -> std::io::Result<()> {
    let mut params = Params::default();
    let mut show = 15;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut num = |name: &str| -> std::io::Result<f64> {
            it.next()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| std::io::Error::other(format!("{name} needs a number\n{USAGE}")))
        };
        match a.as_str() {
            "--threshold" => params.threshold = num("--threshold")?,
            "--lambda" => params.lambda = num("--lambda")?,
            "--show" => show = num("--show")? as usize,
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() {
        return Err(std::io::Error::other(USAGE));
    }

    let t0 = Instant::now();
    let detector = Detector::new(params)?;
    eprintln!("models loaded in {} ms; {params:?}", t0.elapsed().as_millis());

    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file)?;
        for lang in [Lang::En, Lang::Ru] {
            let t = tallies.entry(if lang == Lang::En { "en" } else { "ru" }).or_default();
            for word in words(&text, lang) {
                check(&detector, &word, lang, t);
            }
        }
    }

    for lang in ["en", "ru"] {
        let Some(t) = tallies.get(lang) else { continue };
        let pct = |n: usize| 100.0 * n as f64 / t.words.max(1) as f64;
        println!(
            "{lang}: {} words | false flips {} ({:.2}%) | misses {} ({:.2}%)",
            t.words,
            t.false_flips,
            pct(t.false_flips),
            t.misses,
            pct(t.misses)
        );
        print_top("  false flips (typed right, flipped):", &t.false_flip_words, show);
        print_top("  misses (typed wrong, kept):", &t.missed_words, show);
    }
    Ok(())
}

fn print_top(title: &str, words: &HashMap<String, usize>, show: usize) {
    if words.is_empty() || show == 0 {
        return;
    }
    let mut v: Vec<_> = words.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let list: Vec<String> = v.iter().take(show).map(|(w, n)| format!("{w}×{n}")).collect();
    println!("{title} {}", list.join(" "));
}

fn check(detector: &Detector, word: &[Stroke], lang: Lang, t: &mut Tally) {
    let (en, ru) = keymap::render(word);
    let label = if lang == Lang::En { en } else { ru }.to_lowercase();
    t.words += 1;
    if detector.decide(word, lang) == Verdict::Flip {
        t.false_flips += 1;
        *t.false_flip_words.entry(label.clone()).or_default() += 1;
    }
    if detector.decide(word, lang.other()) == Verdict::Keep {
        t.misses += 1;
        *t.missed_words.entry(label).or_default() += 1;
    }
}

/// The words the daemon would evaluate if `text` were typed in `lang`.
fn words(text: &str, lang: Lang) -> Vec<Vec<Stroke>> {
    let mut out = Vec::new();
    let mut word = Vec::new();
    let mut tainted = false;
    let mut finish = |word: &mut Vec<Stroke>, tainted: &mut bool, evaluate: bool| {
        // Skip "words" without a single letter in this layout (`...`, `--`).
        let (en, ru) = keymap::render(word);
        let letters = if lang == Lang::En { en } else { ru };
        if evaluate && !*tainted && letters.chars().any(char::is_alphabetic) {
            out.push(std::mem::take(word));
        }
        word.clear();
        *tainted = false;
    };
    for c in text.chars().chain([' ']) {
        let c = if c.is_whitespace() { ' ' } else { c };
        let Some(s) = keymap::stroke_for(c, lang) else {
            finish(&mut word, &mut tainted, false);
            continue;
        };
        match keymap::classify(s) {
            KeyClass::Word => word.push(s),
            KeyClass::Taint => {
                word.push(s);
                tainted = true;
            }
            KeyClass::Boundary => finish(&mut word, &mut tainted, true),
            KeyClass::Reset => finish(&mut word, &mut tainted, false),
        }
    }
    out
}
