//! Physical key → character tables for us (QWERTY) and ru (ЙЦУКЕН).

use evdev::KeyCode;

/// One keystroke: physical key plus Shift state. Replaying strokes instead of
/// characters means the output side needs no character tables.
#[derive(Clone, Copy, Debug)]
pub struct Stroke {
    pub key: KeyCode,
    pub shift: bool,
}

/// (key, en, EN, ru, RU)
const TABLE: &[(KeyCode, char, char, char, char)] = &[
    (KeyCode::KEY_Q, 'q', 'Q', 'й', 'Й'),
    (KeyCode::KEY_W, 'w', 'W', 'ц', 'Ц'),
    (KeyCode::KEY_E, 'e', 'E', 'у', 'У'),
    (KeyCode::KEY_R, 'r', 'R', 'к', 'К'),
    (KeyCode::KEY_T, 't', 'T', 'е', 'Е'),
    (KeyCode::KEY_Y, 'y', 'Y', 'н', 'Н'),
    (KeyCode::KEY_U, 'u', 'U', 'г', 'Г'),
    (KeyCode::KEY_I, 'i', 'I', 'ш', 'Ш'),
    (KeyCode::KEY_O, 'o', 'O', 'щ', 'Щ'),
    (KeyCode::KEY_P, 'p', 'P', 'з', 'З'),
    (KeyCode::KEY_LEFTBRACE, '[', '{', 'х', 'Х'),
    (KeyCode::KEY_RIGHTBRACE, ']', '}', 'ъ', 'Ъ'),
    (KeyCode::KEY_A, 'a', 'A', 'ф', 'Ф'),
    (KeyCode::KEY_S, 's', 'S', 'ы', 'Ы'),
    (KeyCode::KEY_D, 'd', 'D', 'в', 'В'),
    (KeyCode::KEY_F, 'f', 'F', 'а', 'А'),
    (KeyCode::KEY_G, 'g', 'G', 'п', 'П'),
    (KeyCode::KEY_H, 'h', 'H', 'р', 'Р'),
    (KeyCode::KEY_J, 'j', 'J', 'о', 'О'),
    (KeyCode::KEY_K, 'k', 'K', 'л', 'Л'),
    (KeyCode::KEY_L, 'l', 'L', 'д', 'Д'),
    (KeyCode::KEY_SEMICOLON, ';', ':', 'ж', 'Ж'),
    (KeyCode::KEY_APOSTROPHE, '\'', '"', 'э', 'Э'),
    (KeyCode::KEY_Z, 'z', 'Z', 'я', 'Я'),
    (KeyCode::KEY_X, 'x', 'X', 'ч', 'Ч'),
    (KeyCode::KEY_C, 'c', 'C', 'с', 'С'),
    (KeyCode::KEY_V, 'v', 'V', 'м', 'М'),
    (KeyCode::KEY_B, 'b', 'B', 'и', 'И'),
    (KeyCode::KEY_N, 'n', 'N', 'т', 'Т'),
    (KeyCode::KEY_M, 'm', 'M', 'ь', 'Ь'),
    (KeyCode::KEY_COMMA, ',', '<', 'б', 'Б'),
    (KeyCode::KEY_DOT, '.', '>', 'ю', 'Ю'),
    (KeyCode::KEY_GRAVE, '`', '~', 'ё', 'Ё'),
];

/// Printable keys that are not letters in either layout. (key, en, EN, ru, RU)
const OTHER: &[(KeyCode, char, char, char, char)] = &[
    (KeyCode::KEY_1, '1', '!', '1', '!'),
    (KeyCode::KEY_2, '2', '@', '2', '"'),
    (KeyCode::KEY_3, '3', '#', '3', '№'),
    (KeyCode::KEY_4, '4', '$', '4', ';'),
    (KeyCode::KEY_5, '5', '%', '5', '%'),
    (KeyCode::KEY_6, '6', '^', '6', ':'),
    (KeyCode::KEY_7, '7', '&', '7', '?'),
    (KeyCode::KEY_8, '8', '*', '8', '*'),
    (KeyCode::KEY_9, '9', '(', '9', '('),
    (KeyCode::KEY_0, '0', ')', '0', ')'),
    (KeyCode::KEY_MINUS, '-', '_', '-', '_'),
    (KeyCode::KEY_EQUAL, '=', '+', '=', '+'),
    (KeyCode::KEY_SLASH, '/', '?', '.', ','),
    (KeyCode::KEY_BACKSLASH, '\\', '|', '\\', '/'),
];

/// What a keystroke does to the word being typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyClass {
    /// A letter in ru (and a letter or `[ ] ; ' , . \`` in en): part of the word.
    Word,
    /// Part of the word, but the word is never auto-corrected: digits, `-`, `=`
    /// (versions, paths, dates, `кто-то`).
    Taint,
    /// Ends the word, and the word may be corrected: Space, Shift+Enter, and
    /// punctuation that isn't a letter in either layout. Slash is `.` and `,`
    /// in ru; Shift+digits are `! ? ( ) " ;` and friends.
    Boundary,
    /// Anything else: the caret moved or the word is over without correction.
    Reset,
}

pub fn classify(s: Stroke) -> KeyClass {
    use KeyCode as K;
    match s.key {
        k if is_word_key(k) => KeyClass::Word,
        K::KEY_SPACE | K::KEY_SLASH => KeyClass::Boundary,
        K::KEY_ENTER | K::KEY_KPENTER if s.shift => KeyClass::Boundary,
        K::KEY_1 | K::KEY_2 | K::KEY_3 | K::KEY_4 | K::KEY_5 | K::KEY_6 | K::KEY_7 | K::KEY_8
        | K::KEY_9 | K::KEY_0 => {
            if s.shift { KeyClass::Boundary } else { KeyClass::Taint }
        }
        K::KEY_MINUS | K::KEY_EQUAL => KeyClass::Taint,
        _ => KeyClass::Reset,
    }
}

pub const WORD_KEYS: &[KeyCode] = &[
    KeyCode::KEY_Q, KeyCode::KEY_W, KeyCode::KEY_E, KeyCode::KEY_R, KeyCode::KEY_T,
    KeyCode::KEY_Y, KeyCode::KEY_U, KeyCode::KEY_I, KeyCode::KEY_O, KeyCode::KEY_P,
    KeyCode::KEY_LEFTBRACE, KeyCode::KEY_RIGHTBRACE,
    KeyCode::KEY_A, KeyCode::KEY_S, KeyCode::KEY_D, KeyCode::KEY_F, KeyCode::KEY_G,
    KeyCode::KEY_H, KeyCode::KEY_J, KeyCode::KEY_K, KeyCode::KEY_L,
    KeyCode::KEY_SEMICOLON, KeyCode::KEY_APOSTROPHE,
    KeyCode::KEY_Z, KeyCode::KEY_X, KeyCode::KEY_C, KeyCode::KEY_V, KeyCode::KEY_B,
    KeyCode::KEY_N, KeyCode::KEY_M, KeyCode::KEY_COMMA, KeyCode::KEY_DOT, KeyCode::KEY_GRAVE,
];

/// Keys that are letters in en too (not `[ ] ; ' , . \``).
pub fn is_letter_in_both(key: KeyCode) -> bool {
    TABLE.iter().any(|(k, e, ..)| *k == key && e.is_ascii_lowercase())
}

pub fn is_word_key(key: KeyCode) -> bool {
    TABLE.iter().any(|(k, ..)| *k == key)
}

/// Render the keystrokes as typed in us and in ru.
pub fn render(word: &[Stroke]) -> (String, String) {
    let mut en = String::new();
    let mut ru = String::new();
    for s in word {
        if let Some(&(_, e, eu, r, ru_up)) = TABLE.iter().find(|(k, ..)| *k == s.key) {
            en.push(if s.shift { eu } else { e });
            ru.push(if s.shift { ru_up } else { r });
        }
    }
    (en, ru)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    /// Index in Hyprland's `kb_layout` ("us,ru").
    pub fn index(self) -> usize {
        match self {
            Lang::En => 0,
            Lang::Ru => 1,
        }
    }

    pub fn from_index(i: usize) -> Lang {
        if i == 1 { Lang::Ru } else { Lang::En }
    }

    pub fn other(self) -> Lang {
        match self {
            Lang::En => Lang::Ru,
            Lang::Ru => Lang::En,
        }
    }
}

/// The keystroke that types `c` in `lang`, if the layout has it.
pub fn stroke_for(c: char, lang: Lang) -> Option<Stroke> {
    if c == ' ' {
        return Some(Stroke { key: KeyCode::KEY_SPACE, shift: false });
    }
    TABLE.iter().chain(OTHER).find_map(|&(key, e, eu, r, ru)| {
        let (lower, upper) = match lang {
            Lang::En => (e, eu),
            Lang::Ru => (r, ru),
        };
        // Check upper first: for non-letters both can be the same char.
        if c == upper && c != lower {
            Some(Stroke { key, shift: true })
        } else {
            (c == lower).then_some(Stroke { key, shift: false })
        }
    })
}

/// The layout-independent name of a word: its keys as unshifted en chars
/// (`next` and `туче` are both `next`). None if the word has keys that
/// aren't letters in ru (digits, `-`, ...): such words are never learned.
pub fn key_string(word: &[Stroke]) -> Option<String> {
    word.iter()
        .map(|s| TABLE.iter().find(|(k, ..)| *k == s.key).map(|&(_, e, ..)| e))
        .collect()
}

/// Inverse of `key_string`.
pub fn from_key_string(keys: &str) -> Option<Vec<Stroke>> {
    keys.chars()
        .map(|c| TABLE.iter().find(|&&(_, e, ..)| e == c).map(|&(key, ..)| Stroke { key, shift: false }))
        .collect()
}

/// The keystrokes that type `word` in `lang`, or None if a char isn't a
/// letter key there (or isn't on the layout at all).
pub fn strokes_for_word(word: &str, lang: Lang) -> Option<Vec<Stroke>> {
    word.chars()
        .map(|c| stroke_for(c, lang).filter(|s| is_word_key(s.key)))
        .collect()
}
