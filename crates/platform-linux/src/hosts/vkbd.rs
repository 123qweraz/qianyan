use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, Device, EventType, InputEvent, Key};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use arboard::Clipboard;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PasteMode {
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
}

enum VkbdTask {
    SendText(String, bool), // text, highlight
    Backspace(usize),
    EmitRaw(Key, i32),      // key, value
    Tap(Key),               // tap a single key (press + release)
}

pub struct Vkbd {
    pub dev: Arc<Mutex<VirtualDevice>>,
    pub paste_mode: Arc<Mutex<PasteMode>>,
    pub clipboard_delay_ms: Arc<Mutex<u64>>,
    pub backspace_delay_ms: Arc<Mutex<u64>>,
    task_tx: Sender<VkbdTask>,
}

impl Vkbd {
    pub fn new_wayland() -> Result<Self, Box<dyn std::error::Error>> {
        let mut keys = AttributeSet::new();
        for code in 0u16..=0x2ff {
            keys.insert(Key::new(code));
        }

        let dev_raw = VirtualDeviceBuilder::new()?
            .name("qianyan-ime-v2")
            .with_keys(&keys)?
            .with_msc(&{
                let mut misc = AttributeSet::<evdev::MiscType>::new();
                misc.insert(evdev::MiscType::MSC_SCAN);
                misc
            })?
            .build()?;

        let dev = Arc::new(Mutex::new(dev_raw));
        let paste_mode = Arc::new(Mutex::new(PasteMode::ShiftInsert));
        let clipboard_delay_ms = Arc::new(Mutex::new(50));
        let backspace_delay_ms = Arc::new(Mutex::new(10));

        let (task_tx, task_rx) = mpsc::channel::<VkbdTask>();
        let is_wayland = true;

        let dev_bg = dev.clone();
        let paste_mode_bg = paste_mode.clone();
        let delay_bg = clipboard_delay_ms.clone();
        let bs_delay_bg = backspace_delay_ms.clone();

        thread::spawn(move || {
            while let Ok(task) = task_rx.recv() {
                match task {
                    VkbdTask::SendText(text, highlight) => {
                        let p_mode = match paste_mode_bg.lock() {
                            Ok(m) => *m,
                            Err(_) => PasteMode::ShiftInsert,
                        };
                        let delay = match delay_bg.lock() {
                            Ok(d) => *d,
                            Err(_) => 50,
                        };
                        Self::do_send_text(
                            &dev_bg, is_wayland, p_mode, delay, &text, highlight,
                        );
                    }
                    VkbdTask::Backspace(count) => {
                        let bs_delay = match bs_delay_bg.lock() {
                            Ok(d) => *d,
                            Err(_) => 10,
                        };
                        Self::do_backspace(&dev_bg, count, bs_delay);
                    }
                    VkbdTask::EmitRaw(key, value) => {
                        Self::do_emit_raw(&dev_bg, key, value);
                    }
                    VkbdTask::Tap(key) => {
                        Self::do_tap(&dev_bg, key);
                    }
                }
            }
        });

        Ok(Self {
            dev,
            paste_mode,
            clipboard_delay_ms,
            backspace_delay_ms,
            task_tx,
        })
    }

    pub fn new(phys_dev: &Device) -> Result<Self, Box<dyn std::error::Error>> {
        let mut keys = AttributeSet::new();

        if let Some(supported) = phys_dev.supported_keys() {
            for k in supported.iter() {
                keys.insert(k);
            }
        }

        keys.insert(Key::KEY_LEFTCTRL);
        keys.insert(Key::KEY_RIGHTCTRL);
        keys.insert(Key::KEY_LEFTSHIFT);
        keys.insert(Key::KEY_RIGHTSHIFT);
        keys.insert(Key::KEY_LEFTALT);
        keys.insert(Key::KEY_RIGHTALT);
        keys.insert(Key::KEY_LEFTMETA);
        keys.insert(Key::KEY_RIGHTMETA);
        keys.insert(Key::KEY_ENTER);
        keys.insert(Key::KEY_KPENTER);

        let dev_raw = VirtualDeviceBuilder::new()?
            .name("qianyan-ime-v2")
            .with_keys(&keys)?
            .with_msc(&{
                let mut misc = AttributeSet::<evdev::MiscType>::new();
                misc.insert(evdev::MiscType::MSC_SCAN);
                misc
            })?
            .build()?;

        let dev = Arc::new(Mutex::new(dev_raw));
        let paste_mode = Arc::new(Mutex::new(PasteMode::ShiftInsert));
        let clipboard_delay_ms = Arc::new(Mutex::new(50));
        let backspace_delay_ms = Arc::new(Mutex::new(10));

        let (task_tx, task_rx) = mpsc::channel::<VkbdTask>();
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var("WAYLAND_SOCKET").is_ok();

        // 启动后台工作线程
        let dev_bg = dev.clone();
        let paste_mode_bg = paste_mode.clone();
        let delay_bg = clipboard_delay_ms.clone();
        let bs_delay_bg = backspace_delay_ms.clone();

        thread::spawn(move || {
            while let Ok(task) = task_rx.recv() {
                match task {
                    VkbdTask::SendText(text, highlight) => {
                        let p_mode = match paste_mode_bg.lock() {
                            Ok(m) => *m,
                            Err(_) => PasteMode::ShiftInsert,
                        };
                        let delay = match delay_bg.lock() {
                            Ok(d) => *d,
                            Err(_) => 50,
                        };
                        Self::do_send_text(
                            &dev_bg, is_wayland, p_mode, delay, &text, highlight,
                        );
                    }
                    VkbdTask::Backspace(count) => {
                        let bs_delay = match bs_delay_bg.lock() {
                            Ok(d) => *d,
                            Err(_) => 10,
                        };
                        Self::do_backspace(&dev_bg, count, bs_delay);
                    }
                    VkbdTask::EmitRaw(key, value) => {
                        Self::do_emit_raw(&dev_bg, key, value);
                    }
                    VkbdTask::Tap(key) => {
                        Self::do_tap(&dev_bg, key);
                    }
                }
            }
        });

        Ok(Self {
            dev,
            paste_mode,
            clipboard_delay_ms,
            backspace_delay_ms,
            task_tx,
        })
    }

    pub fn send_key(&self, key_name: &str) {
        if let Some(key) = key_name_to_key(key_name) {
            let _ = self.task_tx.send(VkbdTask::Tap(key));
        }
    }

    pub fn send_text(&self, text: &str, highlight: bool) {
        let _ = self.task_tx.send(VkbdTask::SendText(text.to_string(), highlight));
    }

    pub fn backspace(&self, count: usize) {
        let _ = self.task_tx.send(VkbdTask::Backspace(count));
    }

    pub fn emit_raw(&self, key: Key, value: i32) {
        let _ = self.task_tx.send(VkbdTask::EmitRaw(key, value));
    }

    // --- 同步工作逻辑 (由后台线程调用) ---

    fn do_send_text(
        dev: &Arc<Mutex<VirtualDevice>>,
        is_wayland: bool,
        mode: PasteMode,
        delay: u64,
        text: &str,
        highlight: bool,
    ) {
        if text.is_empty() {
            return;
        }

        // FAST PATH: 只要是能通过 uinput 直接模拟的 ASCII 字符，都不走剪贴板
        if !highlight
            && text.chars().all(|c| char_to_key_with_shift(c).is_some())
        {
            for c in text.chars() {
                if let Some((key, needs_shift)) = char_to_key_with_shift(c) {
                    if needs_shift {
                        Self::do_emit(dev, Key::KEY_LEFTSHIFT, true);
                    }
                    Self::do_tap(dev, key);
                    if needs_shift {
                        Self::do_emit(dev, Key::KEY_LEFTSHIFT, false);
                    }
                    thread::sleep(Duration::from_micros(200));
                }
            }
            return;
        }

        println!("[Vkbd BG] 正在通过剪贴板路径发送文字: {text} (模式={mode:?})");

        // Wayland: 使用 wl-clipboard-rs 直接操作剪贴板协议 (无外部进程)
        if is_wayland {
            if Self::do_send_via_wl_clipboard(dev, mode, delay, text) {
                return;
            }
            log::warn!("[Vkbd] wl-clipboard-rs 失败, 降级到 arboard");
        }

        // X11 或降级: 使用 arboard
        let _ = Self::do_send_via_arboard(dev, mode, delay, text);
    }

    /// 使用 wl-clipboard-rs (Wayland 原生, 不产生外部进程)
    fn do_send_via_wl_clipboard(
        dev: &Arc<Mutex<VirtualDevice>>,
        mode: PasteMode,
        delay: u64,
        text: &str,
    ) -> bool {
        use wl_clipboard_rs::copy::{self, ClipboardType, MimeType, Options, Source};

        let clipboard_type = match mode {
            PasteMode::ShiftInsert => ClipboardType::Both,
            _ => ClipboardType::Regular,
        };

        let mut opts = Options::new();
        opts.clipboard(clipboard_type);
        // foreground(false) 默认: copy() 后台线程注册数据源 + serve 剪贴板,
        // 前台线程立即返回, delay 后粘贴时数据已就绪
        let opts = opts.clone();

        match copy::copy(opts, Source::Bytes(text.as_bytes().to_vec().into()), MimeType::Text) {
            Ok(()) => {
                thread::sleep(Duration::from_millis(delay));
                Self::perform_paste(dev, mode);
                true
            }
            Err(e) => {
                log::warn!("[Vkbd] wl-clipboard-rs copy 失败: {e:?}");
                false
            }
        }
    }

    /// 使用 arboard 库 (X11 主力, Wayland 降级)
    fn do_send_via_arboard(
        dev: &Arc<Mutex<VirtualDevice>>,
        mode: PasteMode,
        delay: u64,
        text: &str,
    ) -> bool {
        static CLIPBOARD_INSTANCE: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();

        let cb_mutex = CLIPBOARD_INSTANCE.get_or_init(|| Mutex::new(None));

        if let Ok(mut guard) = cb_mutex.lock() {
            if guard.is_none() {
                match Clipboard::new() {
                    Ok(c) => *guard = Some(c),
                    Err(e) => {
                        log::warn!("[Vkbd] 无法初始化 arboard: {e}");
                        return false;
                    }
                }
            }
            if let Some(cb) = guard.as_mut() {
                if cb.set_text(text.to_string()).is_ok() {
                    drop(guard);
                    thread::sleep(Duration::from_millis(delay));
                    Self::perform_paste(dev, mode);
                    return true;
                }
            }
        }
        false
    }

    fn perform_paste(dev: &Arc<Mutex<VirtualDevice>>, mode: PasteMode) {
        match mode {
            PasteMode::CtrlV => {
                Self::do_emit(dev, Key::KEY_LEFTCTRL, true);
                thread::sleep(Duration::from_millis(5));
                Self::do_tap(dev, Key::KEY_V);
                thread::sleep(Duration::from_millis(5));
                Self::do_emit(dev, Key::KEY_LEFTCTRL, false);
            }
            PasteMode::ShiftInsert => {
                Self::do_emit(dev, Key::KEY_LEFTSHIFT, true);
                thread::sleep(Duration::from_millis(5));
                Self::do_tap(dev, Key::KEY_INSERT);
                thread::sleep(Duration::from_millis(5));
                Self::do_emit(dev, Key::KEY_LEFTSHIFT, false);
            }
            PasteMode::CtrlShiftV => {
                Self::do_emit(dev, Key::KEY_LEFTCTRL, true);
                Self::do_emit(dev, Key::KEY_LEFTSHIFT, true);
                thread::sleep(Duration::from_millis(5));
                Self::do_tap(dev, Key::KEY_V);
                thread::sleep(Duration::from_millis(5));
                Self::do_emit(dev, Key::KEY_LEFTSHIFT, false);
                Self::do_emit(dev, Key::KEY_LEFTCTRL, false);
            }
        }
    }

    fn do_backspace(dev: &Arc<Mutex<VirtualDevice>>, count: usize, delay_ms: u64) {
        if count == 0 {
            return;
        }
        // 使用空格+回删技巧来强制中断应用程序（如 Firefox 地址栏）的联想功能
        Self::do_tap(dev, Key::KEY_SPACE);
        Self::do_tap(dev, Key::KEY_BACKSPACE);

        for _ in 0..count {
            Self::do_emit_raw(dev, Key::KEY_BACKSPACE, 1);
            Self::do_emit_raw(dev, Key::KEY_BACKSPACE, 0);
            if delay_ms > 0 {
                thread::sleep(Duration::from_millis(delay_ms));
            }
        }
        eprintln!("[Vkbd BG] do_backspace({}) done (sent 1 SPACE + 1 BS trick + {} BS)", count, count);
    }

    fn do_tap(dev: &Arc<Mutex<VirtualDevice>>, key: Key) {
        Self::do_emit(dev, key, true);
        thread::sleep(Duration::from_millis(2));
        Self::do_emit(dev, key, false);
    }

    fn do_emit_raw(dev: &Arc<Mutex<VirtualDevice>>, key: Key, value: i32) {
        if let Ok(mut d) = dev.lock() {
            let ev = InputEvent::new(EventType::KEY, key.code(), value);
            let syn = InputEvent::new(EventType::SYNCHRONIZATION, 0, 0); // SYN_REPORT
            let _ = d.emit(&[ev, syn]);
        }
    }

    fn do_emit(dev: &Arc<Mutex<VirtualDevice>>, key: Key, down: bool) {
        Self::do_emit_raw(dev, key, if down { 1 } else { 0 });
    }

    pub fn apply_config(&mut self, config: &qianyan_ime_core::Config) {
        if let Ok(mut delay) = self.clipboard_delay_ms.lock() {
            *delay = config.linux.clipboard_delay_ms;
        }
        if let Ok(mut delay) = self.backspace_delay_ms.lock() {
            *delay = config.linux.backspace_delay_ms;
        }
        if let Ok(mut mode) = self.paste_mode.lock() {
            *mode = match config.linux.paste_method.as_str() {
                "ctrl_v" => PasteMode::CtrlV,
                "ctrl_shift_v" => PasteMode::CtrlShiftV,
                _ => PasteMode::ShiftInsert,
            };
        }
    }
}

fn char_to_key_with_shift(c: char) -> Option<(Key, bool)> {
    // 返回 (按键, 是否需要 Shift)
    match c {
        'a'..='z' => char_to_key(c).map(|k| (k, false)),
        'A'..='Z' => {
            let lower = c.to_lowercase().next()?;
            char_to_key(lower).map(|k| (k, true))
        }
        '0'..='9' => char_to_key(c).map(|k| (k, false)),
        '!' => Some((Key::KEY_1, true)),
        '@' => Some((Key::KEY_2, true)),
        '#' => Some((Key::KEY_3, true)),
        '$' => Some((Key::KEY_4, true)),
        '%' => Some((Key::KEY_5, true)),
        '^' => Some((Key::KEY_6, true)),
        '&' => Some((Key::KEY_7, true)),
        '*' => Some((Key::KEY_8, true)),
        '(' => Some((Key::KEY_9, true)),
        ')' => Some((Key::KEY_0, true)),
        '_' => Some((Key::KEY_MINUS, true)),
        '+' => Some((Key::KEY_EQUAL, true)),
        '{' => Some((Key::KEY_LEFTBRACE, true)),
        '}' => Some((Key::KEY_RIGHTBRACE, true)),
        '|' => Some((Key::KEY_BACKSLASH, true)),
        ':' => Some((Key::KEY_SEMICOLON, true)),
        '"' => Some((Key::KEY_APOSTROPHE, true)),
        '<' => Some((Key::KEY_COMMA, true)),
        '>' => Some((Key::KEY_DOT, true)),
        '?' => Some((Key::KEY_SLASH, true)),
        '~' => Some((Key::KEY_GRAVE, true)),
        ' ' => Some((Key::KEY_SPACE, false)),
        '\'' => Some((Key::KEY_APOSTROPHE, false)),
        ',' => Some((Key::KEY_COMMA, false)),
        '.' => Some((Key::KEY_DOT, false)),
        '/' => Some((Key::KEY_SLASH, false)),
        ';' => Some((Key::KEY_SEMICOLON, false)),
        '[' => Some((Key::KEY_LEFTBRACE, false)),
        ']' => Some((Key::KEY_RIGHTBRACE, false)),
        '\\' => Some((Key::KEY_BACKSLASH, false)),
        '-' => Some((Key::KEY_MINUS, false)),
        '=' => Some((Key::KEY_EQUAL, false)),
        '`' => Some((Key::KEY_GRAVE, false)),
        '\n' => Some((Key::KEY_ENTER, false)),
        '\t' => Some((Key::KEY_TAB, false)),
        _ => None,
    }
}

fn char_to_key(c: char) -> Option<Key> {
    match c {
        'a' => Some(Key::KEY_A),
        'b' => Some(Key::KEY_B),
        'c' => Some(Key::KEY_C),
        'd' => Some(Key::KEY_D),
        'e' => Some(Key::KEY_E),
        'f' => Some(Key::KEY_F),
        'g' => Some(Key::KEY_G),
        'h' => Some(Key::KEY_H),
        'i' => Some(Key::KEY_I),
        'j' => Some(Key::KEY_J),
        'k' => Some(Key::KEY_K),
        'l' => Some(Key::KEY_L),
        'm' => Some(Key::KEY_M),
        'n' => Some(Key::KEY_N),
        'o' => Some(Key::KEY_O),
        'p' => Some(Key::KEY_P),
        'q' => Some(Key::KEY_Q),
        'r' => Some(Key::KEY_R),
        's' => Some(Key::KEY_S),
        't' => Some(Key::KEY_T),
        'u' => Some(Key::KEY_U),
        'v' => Some(Key::KEY_V),
        'w' => Some(Key::KEY_W),
        'x' => Some(Key::KEY_X),
        'y' => Some(Key::KEY_Y),
        'z' => Some(Key::KEY_Z),
        '0' => Some(Key::KEY_0),
        '1' => Some(Key::KEY_1),
        '2' => Some(Key::KEY_2),
        '3' => Some(Key::KEY_3),
        '4' => Some(Key::KEY_4),
        '5' => Some(Key::KEY_5),
        '6' => Some(Key::KEY_6),
        '7' => Some(Key::KEY_7),
        '8' => Some(Key::KEY_8),
        '9' => Some(Key::KEY_9),
        '\'' => Some(Key::KEY_APOSTROPHE),
        ' ' => Some(Key::KEY_SPACE),
        ',' => Some(Key::KEY_COMMA),
        '.' => Some(Key::KEY_DOT),
        '/' => Some(Key::KEY_SLASH),
        ';' => Some(Key::KEY_SEMICOLON),
        '[' => Some(Key::KEY_LEFTBRACE),
        ']' => Some(Key::KEY_RIGHTBRACE),
        '\\' => Some(Key::KEY_BACKSLASH),
        '-' => Some(Key::KEY_MINUS),
        '=' => Some(Key::KEY_EQUAL),
        '`' => Some(Key::KEY_GRAVE),
        '\n' => Some(Key::KEY_ENTER),
        '\t' => Some(Key::KEY_TAB),
        _ => None,
    }
}

fn key_name_to_key(name: &str) -> Option<Key> {
    match name.to_lowercase().as_str() {
        "a" => Some(Key::KEY_A),
        "b" => Some(Key::KEY_B),
        "c" => Some(Key::KEY_C),
        "d" => Some(Key::KEY_D),
        "e" => Some(Key::KEY_E),
        "f" => Some(Key::KEY_F),
        "g" => Some(Key::KEY_G),
        "h" => Some(Key::KEY_H),
        "i" => Some(Key::KEY_I),
        "j" => Some(Key::KEY_J),
        "k" => Some(Key::KEY_K),
        "l" => Some(Key::KEY_L),
        "m" => Some(Key::KEY_M),
        "n" => Some(Key::KEY_N),
        "o" => Some(Key::KEY_O),
        "p" => Some(Key::KEY_P),
        "q" => Some(Key::KEY_Q),
        "r" => Some(Key::KEY_R),
        "s" => Some(Key::KEY_S),
        "t" => Some(Key::KEY_T),
        "u" => Some(Key::KEY_U),
        "v" => Some(Key::KEY_V),
        "w" => Some(Key::KEY_W),
        "x" => Some(Key::KEY_X),
        "y" => Some(Key::KEY_Y),
        "z" => Some(Key::KEY_Z),
        "0" => Some(Key::KEY_0),
        "1" => Some(Key::KEY_1),
        "2" => Some(Key::KEY_2),
        "3" => Some(Key::KEY_3),
        "4" => Some(Key::KEY_4),
        "5" => Some(Key::KEY_5),
        "6" => Some(Key::KEY_6),
        "7" => Some(Key::KEY_7),
        "8" => Some(Key::KEY_8),
        "9" => Some(Key::KEY_9),
        "enter" => Some(Key::KEY_ENTER),
        "esc" => Some(Key::KEY_ESC),
        "backspace" => Some(Key::KEY_BACKSPACE),
        "tab" => Some(Key::KEY_TAB),
        "space" => Some(Key::KEY_SPACE),
        "left" => Some(Key::KEY_LEFT),
        "right" => Some(Key::KEY_RIGHT),
        "up" => Some(Key::KEY_UP),
        "down" => Some(Key::KEY_DOWN),
        "home" => Some(Key::KEY_HOME),
        "end" => Some(Key::KEY_END),
        "pageup" => Some(Key::KEY_PAGEUP),
        "pagedown" => Some(Key::KEY_PAGEDOWN),
        "insert" => Some(Key::KEY_INSERT),
        "delete" => Some(Key::KEY_DELETE),
        _ => None,
    }
}
