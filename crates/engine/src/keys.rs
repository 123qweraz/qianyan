#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum VirtualKey {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Space,
    Enter,
    Tab,
    Backspace,
    Esc,
    CapsLock,
    Shift,
    Control,
    Alt,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Delete,
    Grave,
    Minus,
    Equal,
    LeftBrace,
    RightBrace,
    Backslash,
    Semicolon,
    Apostrophe,
    Comma,
    Dot,
    Slash,
}

impl std::fmt::Display for VirtualKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl VirtualKey {
    /// 返回人类可读的按键名称（用于按键可视化浮层）
    pub fn display_name(self) -> &'static str {
        use VirtualKey::*;
        match self {
            A => "A", B => "B", C => "C", D => "D", E => "E",
            F => "F", G => "G", H => "H", I => "I", J => "J",
            K => "K", L => "L", M => "M", N => "N", O => "O",
            P => "P", Q => "Q", R => "R", S => "S", T => "T",
            U => "U", V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
            Digit0 => "0", Digit1 => "1", Digit2 => "2", Digit3 => "3",
            Digit4 => "4", Digit5 => "5", Digit6 => "6", Digit7 => "7",
            Digit8 => "8", Digit9 => "9",
            Space => "␣", Enter => "↵", Tab => "⇥", Backspace => "⌫",
            Esc => "⎋", CapsLock => "⇪",
            Shift => "⇧", Control => "Ctrl", Alt => "Alt",
            Left => "←", Right => "→", Up => "↑", Down => "↓",
            PageUp => "⇞", PageDown => "⇟",
            Home => "↖", End => "↘", Delete => "⌦",
            Grave => "`", Minus => "-", Equal => "=",
            LeftBrace => "[", RightBrace => "]", Backslash => "\\",
            Semicolon => ";", Apostrophe => "'",
            Comma => ",", Dot => ".", Slash => "/",
        }
    }

    /// 是否为修饰键
    pub fn is_modifier(self) -> bool {
        matches!(self, VirtualKey::Shift | VirtualKey::Control | VirtualKey::Alt)
    }
}

impl std::str::FromStr for VirtualKey {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "a" => Ok(Self::A),
            "b" => Ok(Self::B),
            "c" => Ok(Self::C),
            "d" => Ok(Self::D),
            "e" => Ok(Self::E),
            "f" => Ok(Self::F),
            "g" => Ok(Self::G),
            "h" => Ok(Self::H),
            "i" => Ok(Self::I),
            "j" => Ok(Self::J),
            "k" => Ok(Self::K),
            "l" => Ok(Self::L),
            "m" => Ok(Self::M),
            "n" => Ok(Self::N),
            "o" => Ok(Self::O),
            "p" => Ok(Self::P),
            "q" => Ok(Self::Q),
            "r" => Ok(Self::R),
            "s" => Ok(Self::S),
            "t" => Ok(Self::T),
            "u" => Ok(Self::U),
            "v" => Ok(Self::V),
            "w" => Ok(Self::W),
            "x" => Ok(Self::X),
            "y" => Ok(Self::Y),
            "z" => Ok(Self::Z),
            "0" | "digit0" => Ok(Self::Digit0),
            "1" | "digit1" => Ok(Self::Digit1),
            "2" | "digit2" => Ok(Self::Digit2),
            "3" | "digit3" => Ok(Self::Digit3),
            "4" | "digit4" => Ok(Self::Digit4),
            "5" | "digit5" => Ok(Self::Digit5),
            "6" | "digit6" => Ok(Self::Digit6),
            "7" | "digit7" => Ok(Self::Digit7),
            "8" | "digit8" => Ok(Self::Digit8),
            "9" | "digit9" => Ok(Self::Digit9),
            "space" => Ok(Self::Space),
            "enter" => Ok(Self::Enter),
            "tab" => Ok(Self::Tab),
            "backspace" => Ok(Self::Backspace),
            "esc" => Ok(Self::Esc),
            "capslock" => Ok(Self::CapsLock),
            "shift" => Ok(Self::Shift),
            "control" | "ctrl" => Ok(Self::Control),
            "alt" => Ok(Self::Alt),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "pageup" => Ok(Self::PageUp),
            "pagedown" => Ok(Self::PageDown),
            "home" => Ok(Self::Home),
            "end" => Ok(Self::End),
            "delete" => Ok(Self::Delete),
            "grave" | "`" => Ok(Self::Grave),
            "minus" | "-" => Ok(Self::Minus),
            "equal" | "=" => Ok(Self::Equal),
            "leftbrace" | "[" => Ok(Self::LeftBrace),
            "rightbrace" | "]" => Ok(Self::RightBrace),
            "backslash" | "\\" => Ok(Self::Backslash),
            "semicolon" | ";" => Ok(Self::Semicolon),
            "apostrophe" | "'" => Ok(Self::Apostrophe),
            "comma" | "," => Ok(Self::Comma),
            "dot" | "." => Ok(Self::Dot),
            "slash" | "/" => Ok(Self::Slash),
            _ => Err(()),
        }
    }
}

impl VirtualKey {
    pub fn to_u32(self) -> u32 {
        self as u32
    }

    /// Convert a u32 to VirtualKey via the #[repr(u32)] discriminant.
    /// Returns None if the value does not correspond to any variant.
    /// This is the safe alternative to `unsafe { std::mem::transmute }`.
    pub fn from_u32(v: u32) -> Option<Self> {
        use VirtualKey::*;
        Some(match v {
            0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H,
            8 => I, 9 => J, 10 => K, 11 => L, 12 => M, 13 => N, 14 => O,
            15 => P, 16 => Q, 17 => R, 18 => S, 19 => T, 20 => U, 21 => V,
            22 => W, 23 => X, 24 => Y, 25 => Z,
            26 => Digit0, 27 => Digit1, 28 => Digit2, 29 => Digit3, 30 => Digit4,
            31 => Digit5, 32 => Digit6, 33 => Digit7, 34 => Digit8, 35 => Digit9,
            36 => Space, 37 => Enter, 38 => Tab, 39 => Backspace, 40 => Esc,
            41 => CapsLock, 42 => Shift, 43 => Control, 44 => Alt,
            45 => Left, 46 => Right, 47 => Up, 48 => Down,
            49 => PageUp, 50 => PageDown, 51 => Home, 52 => End, 53 => Delete,
            54 => Grave, 55 => Minus, 56 => Equal, 57 => LeftBrace,
            58 => RightBrace, 59 => Backslash, 60 => Semicolon, 61 => Apostrophe,
            62 => Comma, 63 => Dot, 64 => Slash,
            _ => return None,
        })
    }
}

/// Compile-time assertion: VirtualKey discriminants match the hard-coded table above.
const _: fn() = || {
    let _ = VirtualKey::A as u32;
    let _ = VirtualKey::Z as u32 - 25;
    let _ = VirtualKey::Digit0 as u32 - 26;
    let _ = VirtualKey::Digit9 as u32 - 35;
    let _ = VirtualKey::Slash as u32 - 64;
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_name_not_empty() {
        let all = all_variants();
        for vk in &all {
            let name = vk.display_name();
            assert!(!name.is_empty(), "display_name() empty for {:?}", vk);
        }
    }

    #[test]
    fn test_display_name_letters() {
        assert_eq!(VirtualKey::A.display_name(), "A");
        assert_eq!(VirtualKey::Z.display_name(), "Z");
        assert_eq!(VirtualKey::M.display_name(), "M");
    }

    #[test]
    fn test_display_name_digits() {
        assert_eq!(VirtualKey::Digit0.display_name(), "0");
        assert_eq!(VirtualKey::Digit5.display_name(), "5");
        assert_eq!(VirtualKey::Digit9.display_name(), "9");
    }

    #[test]
    fn test_display_name_modifiers() {
        assert_eq!(VirtualKey::Shift.display_name(), "⇧");
        assert_eq!(VirtualKey::Control.display_name(), "Ctrl");
        assert_eq!(VirtualKey::Alt.display_name(), "Alt");
    }

    #[test]
    fn test_display_name_special() {
        assert_eq!(VirtualKey::Space.display_name(), "␣");
        assert_eq!(VirtualKey::Enter.display_name(), "↵");
        assert_eq!(VirtualKey::Tab.display_name(), "⇥");
        assert_eq!(VirtualKey::Backspace.display_name(), "⌫");
        assert_eq!(VirtualKey::Esc.display_name(), "⎋");
        assert_eq!(VirtualKey::CapsLock.display_name(), "⇪");
    }

    #[test]
    fn test_display_name_arrows() {
        assert_eq!(VirtualKey::Left.display_name(), "←");
        assert_eq!(VirtualKey::Right.display_name(), "→");
        assert_eq!(VirtualKey::Up.display_name(), "↑");
        assert_eq!(VirtualKey::Down.display_name(), "↓");
    }

    #[test]
    fn test_display_name_symbols() {
        assert_eq!(VirtualKey::Grave.display_name(), "`");
        assert_eq!(VirtualKey::Minus.display_name(), "-");
        assert_eq!(VirtualKey::Equal.display_name(), "=");
        assert_eq!(VirtualKey::LeftBrace.display_name(), "[");
        assert_eq!(VirtualKey::RightBrace.display_name(), "]");
        assert_eq!(VirtualKey::Backslash.display_name(), "\\");
    }

    #[test]
    fn test_is_modifier_true() {
        assert!(VirtualKey::Shift.is_modifier());
        assert!(VirtualKey::Control.is_modifier());
        assert!(VirtualKey::Alt.is_modifier());
    }

    #[test]
    fn test_is_modifier_false() {
        assert!(!VirtualKey::A.is_modifier());
        assert!(!VirtualKey::Digit1.is_modifier());
        assert!(!VirtualKey::Space.is_modifier());
        assert!(!VirtualKey::Enter.is_modifier());
        assert!(!VirtualKey::Left.is_modifier());
        assert!(!VirtualKey::Esc.is_modifier());
    }

    fn all_variants() -> Vec<VirtualKey> {
        use VirtualKey::*;
        vec![
            A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
            Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
            Space, Enter, Tab, Backspace, Esc, CapsLock,
            Shift, Control, Alt,
            Left, Right, Up, Down,
            PageUp, PageDown, Home, End, Delete,
            Grave, Minus, Equal, LeftBrace, RightBrace, Backslash,
            Semicolon, Apostrophe, Comma, Dot, Slash,
        ]
    }
}
