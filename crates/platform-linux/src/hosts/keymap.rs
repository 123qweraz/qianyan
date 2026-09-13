//! 统一的 evdev 扫描码 → VirtualKey 映射表。
//!
//! 原维护在三处：
//! - evdev_host.rs 的 `evdev_to_virtual`（evdev::Key 常量匹配）
//! - wayland_host.rs 的 `xkb_to_vk_raw`（xkb keycode = 扫描码 + 8 后匹配数值）
//!
//! 两者本质是同一映射（evdev 键盘事件码 = libinput/wayland 上报的扫描码），
//! 现合并为单一真值来源，另两处改为薄包装调用，避免反复维护导致不一致。

use qianyan_ime_engine::keys::VirtualKey;

/// evdev 扫描码 → VirtualKey。扫描码值即 evdev KEY_* 常量（如 KEY_A=30）。
pub(crate) fn scancode_to_vk(code: u32) -> Option<VirtualKey> {
    use VirtualKey::*;
    Some(match code {
        1 => Esc,           // KEY_ESC
        2 => Digit1,        // KEY_1
        3 => Digit2,        // KEY_2
        4 => Digit3,        // KEY_3
        5 => Digit4,        // KEY_4
        6 => Digit5,        // KEY_5
        7 => Digit6,        // KEY_6
        8 => Digit7,        // KEY_7
        9 => Digit8,        // KEY_8
        10 => Digit9,       // KEY_9
        11 => Digit0,       // KEY_0
        12 => Minus,        // KEY_MINUS
        13 => Equal,        // KEY_EQUAL
        14 => Backspace,    // KEY_BACKSPACE
        15 => Tab,          // KEY_TAB
        16 => Q,
        17 => W,
        18 => E,
        19 => R,
        20 => T,
        21 => Y,
        22 => U,
        23 => I,
        24 => O,
        25 => P,
        26 => LeftBrace,    // KEY_LEFTBRACE
        27 => RightBrace,   // KEY_RIGHTBRACE
        28 => Enter,        // KEY_ENTER
        29 => Control,      // KEY_LEFTCTRL
        30 => A,
        31 => S,
        32 => D,
        33 => F,
        34 => G,
        35 => H,
        36 => J,
        37 => K,
        38 => L,
        39 => Semicolon,    // KEY_SEMICOLON
        40 => Apostrophe,   // KEY_APOSTROPHE
        41 => Grave,        // KEY_GRAVE
        42 => Shift,        // KEY_LEFTSHIFT
        43 => Backslash,    // KEY_BACKSLASH
        44 => Z,
        45 => X,
        46 => C,
        47 => V,
        48 => B,
        49 => N,
        50 => M,
        51 => Comma,        // KEY_COMMA
        52 => Dot,          // KEY_DOT
        53 => Slash,        // KEY_SLASH
        54 => Shift,        // KEY_RIGHTSHIFT
        56 => Alt,          // KEY_LEFTALT
        57 => Space,        // KEY_SPACE
        58 => CapsLock,     // KEY_CAPSLOCK
        96 => Enter,        // KEY_KPENTER
        97 => Control,      // KEY_RIGHTCTRL
        100 => Alt,         // KEY_RIGHTALT
        102 => Home,        // KEY_HOME
        103 => Up,          // KEY_UP
        104 => PageUp,      // KEY_PAGEUP
        105 => Left,        // KEY_LEFT
        106 => Right,       // KEY_RIGHT
        107 => End,         // KEY_END
        108 => Down,        // KEY_DOWN
        109 => PageDown,    // KEY_PAGEDOWN
        111 => Delete,      // KEY_DELETE
        // 以下为历史 fallback 表保留的映射（与 evdev 语义不符，但维持旧行为）：
        119 => Esc,         // 实为 KEY_PAUSE
        125 | 126 => Control, // 实为 KEY_LEFTMETA / KEY_RIGHTMETA
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::Key;

    fn pairs() -> Vec<(u16, VirtualKey)> {
        vec![
            (Key::KEY_ESC.code(), VirtualKey::Esc),
            (Key::KEY_1.code(), VirtualKey::Digit1),
            (Key::KEY_2.code(), VirtualKey::Digit2),
            (Key::KEY_3.code(), VirtualKey::Digit3),
            (Key::KEY_4.code(), VirtualKey::Digit4),
            (Key::KEY_5.code(), VirtualKey::Digit5),
            (Key::KEY_6.code(), VirtualKey::Digit6),
            (Key::KEY_7.code(), VirtualKey::Digit7),
            (Key::KEY_8.code(), VirtualKey::Digit8),
            (Key::KEY_9.code(), VirtualKey::Digit9),
            (Key::KEY_0.code(), VirtualKey::Digit0),
            (Key::KEY_MINUS.code(), VirtualKey::Minus),
            (Key::KEY_EQUAL.code(), VirtualKey::Equal),
            (Key::KEY_BACKSPACE.code(), VirtualKey::Backspace),
            (Key::KEY_TAB.code(), VirtualKey::Tab),
            (Key::KEY_Q.code(), VirtualKey::Q),
            (Key::KEY_W.code(), VirtualKey::W),
            (Key::KEY_E.code(), VirtualKey::E),
            (Key::KEY_R.code(), VirtualKey::R),
            (Key::KEY_T.code(), VirtualKey::T),
            (Key::KEY_Y.code(), VirtualKey::Y),
            (Key::KEY_U.code(), VirtualKey::U),
            (Key::KEY_I.code(), VirtualKey::I),
            (Key::KEY_O.code(), VirtualKey::O),
            (Key::KEY_P.code(), VirtualKey::P),
            (Key::KEY_LEFTBRACE.code(), VirtualKey::LeftBrace),
            (Key::KEY_RIGHTBRACE.code(), VirtualKey::RightBrace),
            (Key::KEY_ENTER.code(), VirtualKey::Enter),
            (Key::KEY_KPENTER.code(), VirtualKey::Enter),
            (Key::KEY_LEFTCTRL.code(), VirtualKey::Control),
            (Key::KEY_A.code(), VirtualKey::A),
            (Key::KEY_S.code(), VirtualKey::S),
            (Key::KEY_D.code(), VirtualKey::D),
            (Key::KEY_F.code(), VirtualKey::F),
            (Key::KEY_G.code(), VirtualKey::G),
            (Key::KEY_H.code(), VirtualKey::H),
            (Key::KEY_J.code(), VirtualKey::J),
            (Key::KEY_K.code(), VirtualKey::K),
            (Key::KEY_L.code(), VirtualKey::L),
            (Key::KEY_SEMICOLON.code(), VirtualKey::Semicolon),
            (Key::KEY_APOSTROPHE.code(), VirtualKey::Apostrophe),
            (Key::KEY_GRAVE.code(), VirtualKey::Grave),
            (Key::KEY_LEFTSHIFT.code(), VirtualKey::Shift),
            (Key::KEY_BACKSLASH.code(), VirtualKey::Backslash),
            (Key::KEY_Z.code(), VirtualKey::Z),
            (Key::KEY_X.code(), VirtualKey::X),
            (Key::KEY_C.code(), VirtualKey::C),
            (Key::KEY_V.code(), VirtualKey::V),
            (Key::KEY_B.code(), VirtualKey::B),
            (Key::KEY_N.code(), VirtualKey::N),
            (Key::KEY_M.code(), VirtualKey::M),
            (Key::KEY_COMMA.code(), VirtualKey::Comma),
            (Key::KEY_DOT.code(), VirtualKey::Dot),
            (Key::KEY_SLASH.code(), VirtualKey::Slash),
            (Key::KEY_RIGHTSHIFT.code(), VirtualKey::Shift),
            (Key::KEY_LEFTALT.code(), VirtualKey::Alt),
            (Key::KEY_SPACE.code(), VirtualKey::Space),
            (Key::KEY_CAPSLOCK.code(), VirtualKey::CapsLock),
            (Key::KEY_RIGHTCTRL.code(), VirtualKey::Control),
            (Key::KEY_RIGHTALT.code(), VirtualKey::Alt),
            (Key::KEY_UP.code(), VirtualKey::Up),
            (Key::KEY_PAGEUP.code(), VirtualKey::PageUp),
            (Key::KEY_LEFT.code(), VirtualKey::Left),
            (Key::KEY_RIGHT.code(), VirtualKey::Right),
            (Key::KEY_DOWN.code(), VirtualKey::Down),
            (Key::KEY_PAGEDOWN.code(), VirtualKey::PageDown),
            (Key::KEY_HOME.code(), VirtualKey::Home),
            (Key::KEY_DELETE.code(), VirtualKey::Delete),
            (Key::KEY_END.code(), VirtualKey::End),
        ]
    }

    /// 统一表必须与 evdev 常量逐一吻合
    #[test]
    fn unified_table_matches_evdev_constants() {
        for (code, vk) in pairs() {
            let got = scancode_to_vk(code as u32);
            assert_eq!(got, Some(vk), "scancode {} 映射不一致", code);
        }
    }

    /// 旧 wayland fallback 路径（xkb keycode = 扫描码 + 8）与新表结果一致
    #[cfg(target_os = "linux")]
    #[test]
    fn raw_xkb_fallback_consistent_with_unified_table() {
        for (code, vk) in pairs() {
            let got = super::super::wayland_host::xkb_to_vk_raw(code as u32 + 8);
            assert_eq!(got, Some(vk), "xkb keycode {} 解析不一致", code + 8);
        }
    }

    /// 历史 quirk：保留不被 evdev 常量验证的映射
    #[test]
    fn legacy_quirk_mappings_preserved() {
        assert_eq!(scancode_to_vk(119), Some(VirtualKey::Esc));
        assert_eq!(scancode_to_vk(125), Some(VirtualKey::Control));
        assert_eq!(scancode_to_vk(126), Some(VirtualKey::Control));
    }

    #[test]
    fn unknown_scancode_is_none() {
        assert_eq!(scancode_to_vk(0), None);
        assert_eq!(scancode_to_vk(200), None);
    }
}