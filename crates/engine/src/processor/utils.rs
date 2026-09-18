use crate::keys::VirtualKey;

pub fn is_letter(key: VirtualKey) -> bool {
    matches!(
        key,
        VirtualKey::Q
            | VirtualKey::W
            | VirtualKey::E
            | VirtualKey::R
            | VirtualKey::T
            | VirtualKey::Y
            | VirtualKey::U
            | VirtualKey::I
            | VirtualKey::O
            | VirtualKey::P
            | VirtualKey::A
            | VirtualKey::S
            | VirtualKey::D
            | VirtualKey::F
            | VirtualKey::G
            | VirtualKey::H
            | VirtualKey::J
            | VirtualKey::K
            | VirtualKey::L
            | VirtualKey::Z
            | VirtualKey::X
            | VirtualKey::C
            | VirtualKey::V
            | VirtualKey::B
            | VirtualKey::N
            | VirtualKey::M
    )
}

pub fn is_digit(key: VirtualKey) -> bool {
    matches!(
        key,
        VirtualKey::Digit1
            | VirtualKey::Digit2
            | VirtualKey::Digit3
            | VirtualKey::Digit4
            | VirtualKey::Digit5
            | VirtualKey::Digit6
            | VirtualKey::Digit7
            | VirtualKey::Digit8
            | VirtualKey::Digit9
            | VirtualKey::Digit0
    )
}

pub fn key_to_digit(key: VirtualKey) -> Option<usize> {
    match key {
        VirtualKey::Digit1 => Some(1),
        VirtualKey::Digit2 => Some(2),
        VirtualKey::Digit3 => Some(3),
        VirtualKey::Digit4 => Some(4),
        VirtualKey::Digit5 => Some(5),
        VirtualKey::Digit6 => Some(6),
        VirtualKey::Digit7 => Some(7),
        VirtualKey::Digit8 => Some(8),
        VirtualKey::Digit9 => Some(9),
        VirtualKey::Digit0 => Some(0),
        _ => None,
    }
}

pub fn key_to_char(key: VirtualKey, shift: bool, caps_lock: bool) -> Option<char> {
    let c = match key {
        VirtualKey::Q => Some('q'),
        VirtualKey::W => Some('w'),
        VirtualKey::E => Some('e'),
        VirtualKey::R => Some('r'),
        VirtualKey::T => Some('t'),
        VirtualKey::Y => Some('y'),
        VirtualKey::U => Some('u'),
        VirtualKey::I => Some('i'),
        VirtualKey::O => Some('o'),
        VirtualKey::P => Some('p'),
        VirtualKey::A => Some('a'),
        VirtualKey::S => Some('s'),
        VirtualKey::D => Some('d'),
        VirtualKey::F => Some('f'),
        VirtualKey::G => Some('g'),
        VirtualKey::H => Some('h'),
        VirtualKey::J => Some('j'),
        VirtualKey::K => Some('k'),
        VirtualKey::L => Some('l'),
        VirtualKey::Z => Some('z'),
        VirtualKey::X => Some('x'),
        VirtualKey::C => Some('c'),
        VirtualKey::V => Some('v'),
        VirtualKey::B => Some('b'),
        VirtualKey::N => Some('n'),
        VirtualKey::M => Some('m'),
        VirtualKey::Apostrophe => Some('\''),
        VirtualKey::Slash => Some('/'),
        VirtualKey::Minus => Some('-'),
        VirtualKey::Equal => Some('='),
        VirtualKey::Comma => Some(','),
        VirtualKey::Dot => Some('.'),
        VirtualKey::Digit1 => Some('1'),
        VirtualKey::Digit2 => Some('2'),
        VirtualKey::Digit3 => Some('3'),
        VirtualKey::Digit4 => Some('4'),
        VirtualKey::Digit5 => Some('5'),
        VirtualKey::Digit6 => Some('6'),
        VirtualKey::Digit7 => Some('7'),
        VirtualKey::Digit8 => Some('8'),
        VirtualKey::Digit9 => Some('9'),
        VirtualKey::Digit0 => Some('0'),
        VirtualKey::Grave => Some('`'),
        VirtualKey::LeftBrace => Some('['),
        VirtualKey::RightBrace => Some(']'),
        VirtualKey::Backslash => Some('\\'),
        VirtualKey::Semicolon => Some(';'),
        _ => None,
    };

    // 逻辑：如果是字母，(Shift XOR CapsLock) 为真则大写；
    // 如果不是字母，仅取决于 Shift。
    if is_letter(key) {
        if shift ^ caps_lock {
            c.map(|ch| ch.to_ascii_uppercase())
        } else {
            c
        }
    } else if shift {
        match key {
            VirtualKey::Digit1 => Some('!'),
            VirtualKey::Digit2 => Some('@'),
            VirtualKey::Digit3 => Some('#'),
            VirtualKey::Digit4 => Some('$'),
            VirtualKey::Digit5 => Some('%'),
            VirtualKey::Digit6 => Some('^'),
            VirtualKey::Digit7 => Some('&'),
            VirtualKey::Digit8 => Some('*'),
            VirtualKey::Digit9 => Some('('),
            VirtualKey::Digit0 => Some(')'),
            VirtualKey::Minus => Some('_'),
            VirtualKey::Equal => Some('+'),
            VirtualKey::Comma => Some('<'),
            VirtualKey::Dot => Some('>'),
            VirtualKey::Slash => Some('?'),
            VirtualKey::Grave => Some('~'),
            VirtualKey::LeftBrace => Some('{'),
            VirtualKey::RightBrace => Some('}'),
            VirtualKey::Backslash => Some('|'),
            VirtualKey::Semicolon => Some(':'),
            VirtualKey::Apostrophe => Some('"'),
            _ => c,
        }
    } else {
        c
    }
}

/// 判断热键字符串（如 `Ctrl+Shift+F`）是否与当前按键状态匹配。
/// 修饰键需要严格一致；目标键支持字母、数字、符号以及 Space/Tab/CapsLock。
/// 非修饰键只能出现在末尾，因此像旧的 `CapsLock+F` 这类配置不会被误判成单独的 F。
pub fn hotkey_matches(binding: &str, key: VirtualKey, ctrl: bool, alt: bool, shift: bool) -> bool {
    let parts: Vec<&str> = binding
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return false;
    }

    let mut req_ctrl = false;
    let mut req_alt = false;
    let mut req_shift = false;
    let mut target: Option<&str> = None;
    let last = parts.len() - 1;

    for (i, part) in parts.iter().enumerate() {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => {
                if i == last {
                    return false;
                }
                req_ctrl = true;
            }
            "alt" | "option" => {
                if i == last {
                    return false;
                }
                req_alt = true;
            }
            "shift" => {
                if i == last {
                    return false;
                }
                req_shift = true;
            }
            _ => {
                if i != last {
                    return false;
                }
                target = Some(part);
            }
        }
    }

    if req_ctrl != ctrl || req_alt != alt || req_shift != shift {
        return false;
    }

    let Some(target) = target else {
        return false;
    };

    if target.eq_ignore_ascii_case("space") {
        return key == VirtualKey::Space;
    }
    if target.eq_ignore_ascii_case("tab") {
        return key == VirtualKey::Tab;
    }
    if target.eq_ignore_ascii_case("capslock") {
        return key == VirtualKey::CapsLock;
    }

    if let Some(c) = target.chars().next() {
        if let Some(k) = key_to_char(key, false, false) {
            return k.eq_ignore_ascii_case(&c);
        }
    }
    false
}

pub fn get_punctuation_key(key: VirtualKey, shift: bool) -> Option<&'static str> {
    match (key, shift) {
        (VirtualKey::Space, _) => Some(" "),
        (VirtualKey::Grave, false) => Some("`"),
        (VirtualKey::Grave, true) => Some("~"),
        (VirtualKey::Minus, false) => Some("-"),
        (VirtualKey::Minus, true) => Some("_"),
        (VirtualKey::Equal, false) => Some("="),
        (VirtualKey::Equal, true) => Some("+"),
        (VirtualKey::LeftBrace, false) => Some("["),
        (VirtualKey::LeftBrace, true) => Some("{"),
        (VirtualKey::RightBrace, false) => Some("]"),
        (VirtualKey::RightBrace, true) => Some("}"),
        (VirtualKey::Backslash, false) => Some("\\"),
        (VirtualKey::Backslash, true) => Some("|"),
        (VirtualKey::Semicolon, false) => Some(";"),
        (VirtualKey::Semicolon, true) => Some(":"),
        (VirtualKey::Apostrophe, false) => Some("'"),
        (VirtualKey::Apostrophe, true) => Some("\""),
        (VirtualKey::Comma, false) => Some(","),
        (VirtualKey::Comma, true) => Some("<"),
        (VirtualKey::Dot, false) => Some("."),
        (VirtualKey::Dot, true) => Some(">"),
        (VirtualKey::Slash, false) => Some("/"),
        (VirtualKey::Slash, true) => Some("?"),
        (VirtualKey::Digit1, true) => Some("!"),
        (VirtualKey::Digit2, true) => Some("@"),
        (VirtualKey::Digit3, true) => Some("#"),
        (VirtualKey::Digit4, true) => Some("$"),
        (VirtualKey::Digit5, true) => Some("%"),
        (VirtualKey::Digit6, true) => Some("^"),
        (VirtualKey::Digit7, true) => Some("&"),
        (VirtualKey::Digit8, true) => Some("*"),
        (VirtualKey::Digit9, true) => Some("("),
        (VirtualKey::Digit0, true) => Some(")"),
        _ => None,
    }
}

pub fn strip_tones(s: &str) -> String {
    let mut res = String::new();
    for c in s.chars() {
        match c {
            'ā' | 'á' | 'ǎ' | 'à' => res.push('a'),
            'ē' | 'é' | 'ě' | 'è' => res.push('e'),
            'ī' | 'í' | 'ǐ' | 'ì' => res.push('i'),
            'ō' | 'ó' | 'ǒ' | 'ò' => res.push('o'),
            'ū' | 'ú' | 'ǔ' | 'ù' => res.push('u'),
            'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' => res.push('v'),
            'Ā' | 'Á' | 'Ǎ' | 'À' => res.push('a'),
            'Ē' | 'É' | 'Ě' | 'È' => res.push('e'),
            'Ī' | 'Í' | 'Ǐ' | 'Ì' => res.push('i'),
            'Ō' | 'Ó' | 'Ǒ' | 'Ò' => res.push('o'),
            'Ū' | 'Ú' | 'Ǔ' | 'Ù' => res.push('u'),
            'Ǖ' | 'Ǘ' | 'Ǚ' | 'Ǜ' => res.push('v'),
            _ => res.push(c),
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_matches_modifiers_strictly() {
        assert!(hotkey_matches("Ctrl+Shift+F", VirtualKey::F, true, false, true));
        // 缺少或多余的修饰键都不匹配
        assert!(!hotkey_matches("Ctrl+Shift+F", VirtualKey::F, true, false, false));
        assert!(!hotkey_matches("Ctrl+Shift+F", VirtualKey::F, false, false, true));
        assert!(!hotkey_matches("Ctrl+Shift+F", VirtualKey::G, true, false, true));
    }

    #[test]
    fn hotkey_matches_named_keys() {
        assert!(hotkey_matches("Ctrl+Space", VirtualKey::Space, true, false, false));
        assert!(hotkey_matches("Tab", VirtualKey::Tab, false, false, false));
        assert!(hotkey_matches("CapsLock", VirtualKey::CapsLock, false, false, false));
    }

    #[test]
    fn legacy_capslock_plus_f_binding_never_matches_plain_f() {
        // 旧的 "CapsLock+F" 配置非修饰键不在末尾，绝不能退化成普通 F 触发
        assert!(!hotkey_matches("CapsLock+F", VirtualKey::F, false, false, false));
    }
}
