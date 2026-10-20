//! Physical key → character tables for us (QWERTY) and ru (ЙЦУКЕН).

use evdev::KeyCode;

use crate::Stroke;

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
