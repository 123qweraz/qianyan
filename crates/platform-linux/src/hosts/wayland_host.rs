use std::error::Error;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use log::{error, info, warn};
use qianyan_ime_core::Rect;

use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::wl_keyboard::KeyState;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::ZwpInputMethodV2,
};

use xkbcommon::xkb;
use xkbcommon::xkb::keysyms;

use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::processor::Action;
use qianyan_ime_engine::processor::actor::ProcessorHandle;
use qianyan_ime_ui::GuiEvent;
use qianyan_ime_ui::tray::TrayEvent;

struct WlState {
    running: Arc<AtomicBool>,
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    serial: u32,
    active: bool,
    input_method: Option<ZwpInputMethodV2>,
    keyboard_grab: Option<ZwpInputMethodKeyboardGrabV2>,
    xkb_context: xkb::Context,
    xkb_state: Option<xkb::State>,
    surrounding_text: String,
    surrounding_cursor: u32,
    surrounding_anchor: u32,
    content_hint: u32,
    content_purpose: u32,
    prev_chinese_enabled: bool,
}

impl Dispatch<WlRegistry, GlobalListContents> for WlState {
    fn event(
        _state: &mut Self,
        _: &WlRegistry,
        _event: <WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

struct WlUser;

impl Dispatch<ZwpInputMethodManagerV2, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &ZwpInputMethodManagerV2,
        _event: <ZwpInputMethodManagerV2 as Proxy>::Event,
        _data: &WlUser,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodV2, WlUser> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwpInputMethodV2,
        event: <ZwpInputMethodV2 as Proxy>::Event,
        _data: &WlUser,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::Event;
        match event {
            Event::Activate => {
                info!("[WaylandIM] activated");
                state.active = true;
                state.serial = 0;
                if let Some(ref im) = state.input_method {
                    let grab = im.grab_keyboard(qh, WlUser);
                    state.keyboard_grab = Some(grab);
                    // Commit initial empty state so the compositor knows we are ready
                    im.set_preedit_string(String::new(), 0, 0);
                    im.commit(state.serial);
                    let _ = conn.flush();
                }
            }
            Event::Deactivate => {
                info!("[WaylandIM] deactivated");
                state.active = false;
                state.keyboard_grab = None;
                if let Some(ref im) = state.input_method {
                    im.set_preedit_string("".into(), 0, 0);
                    im.commit(state.serial);
                    let _ = conn.flush();
                }
                let _ = state.gui_tx.send(GuiEvent::SetVisible(false));
            }
            Event::Done => {
                state.serial += 1;
            }
            Event::SurroundingText { text, cursor, anchor } => {
                state.surrounding_text = text;
                state.surrounding_cursor = cursor;
                state.surrounding_anchor = anchor;
            }
            Event::ContentType { hint, purpose } => {
                state.content_hint = hint.into();
                state.content_purpose = purpose.into();
            }
            Event::Unavailable => {
                warn!("[WaylandIM] unavailable");
                state.active = false;
                state.keyboard_grab = None;
                state.running.store(false, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &WlSeat,
        _event: <WlSeat as Proxy>::Event,
        _data: &WlUser,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, WlUser> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwpInputMethodKeyboardGrabV2,
        event: <ZwpInputMethodKeyboardGrabV2 as Proxy>::Event,
        _data: &WlUser,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event;
        match event {
            Event::Keymap { format, fd, size } => {
                info!("[WaylandIM] received keymap (format={:?}, size={})", format, size);
                let raw_fd = fd.into_raw_fd();
                let mut file = unsafe { std::fs::File::from_raw_fd(raw_fd) };
                use std::io::Read;
                let mut keymap_str = String::new();
                if file.read_to_string(&mut keymap_str).is_ok() {
                    if let Some(keymap) = xkb::Keymap::new_from_string(
                        &state.xkb_context,
                        keymap_str,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    ) {
                        state.xkb_state = Some(xkb::State::new(&keymap));
                        info!("[WaylandIM] xkb keymap and state created successfully");
                    } else {
                        error!("[WaylandIM] failed to create xkb keymap from string");
                    }
                } else {
                    error!("[WaylandIM] failed to read keymap fd");
                }
            }
            Event::Key { serial: _, time, key, state: key_state } => {
                if !state.active {
                    return;
                }
                if key_state != WEnum::Value(KeyState::Pressed) {
                    return;
                }

                let (vk, utf8_text) = state.resolve_key(key);

                let prev_enabled = state.prev_chinese_enabled;

                // Use handle_key_sync to get action + gui + status atomically
                let (action, gui, status) = match state.processor.handle_key_sync(vk, 1, false, false, false) {
                    Some(tuple) => tuple,
                    None => return,
                };

                state.prev_chinese_enabled = status.chinese_enabled;
                if status.chinese_enabled != prev_enabled {
                    let text = if status.chinese_enabled { status.short_display.clone() } else { "英".into() };
                    let _ = state.gui_tx.send(GuiEvent::ShowStatus(text, status.chinese_enabled));
                    let _ = state.tray_tx.send(TrayEvent::SyncStatus {
                        chinese_enabled: status.chinese_enabled,
                        active_profile: status.active_profile.clone(),
                    });
                }

                // Build GUI update
                let update = if gui.pinyin.is_empty() || !gui.chinese_enabled {
                    GuiEvent::Update {
                        pinyin: "".into(),
                        candidates: vec![],
                        selected: 0, page: 0, total_pages: 0,
                        sentence: "".into(), cursor_pos: 0,
                        commit_mode: gui.commit_mode.clone(),
                    }
                } else {
                    let candidates: Vec<qianyan_ime_ui::DisplayCandidate> = gui.candidates.iter().map(|c| {
                        let full_display = if c.is_fuzzy {
                            format!("{}{}(模糊)", c.label, c.text)
                        } else if c.hint.is_empty() {
                            format!("{}{}", c.label, c.text)
                        } else {
                            format!("{}{}({})", c.label, c.text, c.hint)
                        };
                        qianyan_ime_ui::DisplayCandidate {
                            text: c.text.clone(),
                            label: c.label.clone(),
                            hint: c.hint.clone(),
                            full_display,
                            is_fuzzy: c.is_fuzzy,
                        }
                    }).collect();
                    GuiEvent::Update {
                        pinyin: gui.pinyin.clone(),
                        candidates,
                        selected: gui.selected,
                        page: gui.page,
                        total_pages: gui.total_pages,
                        sentence: gui.sentence.clone(),
                        cursor_pos: gui.cursor_pos,
                        commit_mode: gui.commit_mode,
                    }
                };
                let _ = state.gui_tx.send(update);

                // Apply action + preedit in a single commit
                Self::apply_state(state, &action, conn, utf8_text, key, time, &gui.pinyin);
                let _ = conn.flush();
            }
            Event::Modifiers {
                serial: _,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                if let Some(ref mut xkb_st) = state.xkb_state {
                    xkb_st.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            Event::RepeatInfo { rate: _, delay: _ } => {}
            _ => {}
        }
    }
}

impl WlState {
    #[inline]
    fn resolve_key(&self, keycode: u32) -> (VirtualKey, String) {
        if let Some(ref xkb_st) = self.xkb_state {
            // Wayland sends xkb keycodes (evdev scancode + 8)
            let xkb_keycode = xkb::Keycode::new(keycode + 8);
            let sym = xkb_st.key_get_one_sym(xkb_keycode);
            let utf8 = xkb_st.key_get_utf8(xkb_keycode);
            let vk = keysym_to_vk(sym);
            (vk, utf8)
        } else {
            // Fallback: raw evdev mapping (same as old code)
            (xkb_to_vk_raw(keycode), String::new())
        }
    }

    fn apply_state(
        state: &Self,
        action: &Action,
        conn: &Connection,
        utf8_text: String,
        _key: u32,
        _time: u32,
        preedit: &str,
    ) {
        let im = match state.input_method.as_ref() {
            Some(im) => im,
            None => return,
        };

        match action {
            Action::Emit(text) => {
                im.commit_string(text.clone());
            }
            Action::DeleteAndEmit { delete, insert } => {
                im.delete_surrounding_text(u32::try_from(*delete).unwrap_or(0), 0);
                im.commit_string(insert.clone());
            }
            Action::PassThrough if utf8_text.is_empty() => {
                // If utf8_text is empty (e.g. function keys), the key is consumed.
            }
            Action::PassThrough => {
                im.commit_string(utf8_text);
            }
            Action::Alert => {
                Self::play_beep();
            }
            _ => {}
        }

        // Set preedit string
        if preedit.is_empty() {
            im.set_preedit_string(String::new(), 0, 0);
        } else {
            let len = preedit.len() as i32;
            im.set_preedit_string(preedit.to_string(), len, len);
        }

        // Single commit to apply all pending state
        im.commit(state.serial);
        let _ = conn.flush();
    }

    fn play_beep() {
        let root = crate::find_project_root();
        let sound_path = root.join("sounds/beep.wav");
        if sound_path.exists() {
            let _ = std::process::Command::new("canberra-gtk-play")
                .arg("-f")
                .arg(sound_path)
                .spawn();
        } else {
            let _ = std::process::Command::new("canberra-gtk-play")
                .arg("--id=dialog-error")
                .spawn();
        }
    }
}

pub struct WaylandInputHost {
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    running: Arc<AtomicBool>,
}

impl InputMethodHost for WaylandInputHost {
    fn set_preedit(&self, _text: &str, _cursor_pos: usize) {}
    fn commit_text(&self, _text: &str) {}
    fn get_cursor_rect(&self) -> Option<Rect> {
        None
    }

    fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, mut event_queue): (GlobalList, EventQueue<WlState>) =
            registry_queue_init(&conn)?;
        let qh = event_queue.handle();

        let im_manager: ZwpInputMethodManagerV2 = match globals.bind(&qh, 1..=1, WlUser) {
            Ok(m) => m,
            Err(e) => {
                error!("[WaylandIM] failed to bind im_manager: {:?}", e);
                return Err("no zwp_input_method_manager_v2".into());
            }
        };

        let seat: WlSeat = match globals.bind(&qh, 1..=1, WlUser) {
            Ok(s) => s,
            Err(e) => {
                error!("[WaylandIM] failed to bind seat: {:?}", e);
                return Err("no wl_seat".into());
            }
        };

        let input_method: ZwpInputMethodV2 = im_manager.get_input_method(&seat, &qh, WlUser);
        self.running.store(true, Ordering::SeqCst);

        info!("[WaylandIM] connected, input_method obtained");

        let prev_chinese_enabled = self.processor.get_basic_status()
            .map(|s| s.chinese_enabled).unwrap_or(true);

        let mut state = WlState {
            running: self.running.clone(),
            processor: self.processor.clone(),
            gui_tx: self.gui_tx.clone(),
            tray_tx: self.tray_tx.clone(),
            serial: 0,
            active: false,
            input_method: Some(input_method),
            keyboard_grab: None,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_state: None,
            surrounding_text: String::new(),
            surrounding_cursor: 0,
            surrounding_anchor: 0,
            content_hint: 0,
            content_purpose: 0,
            prev_chinese_enabled,
        };

        let _ = event_queue.dispatch_pending(&mut state);
        let _ = conn.flush();

        loop {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            let _ = conn.flush();
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                error!("[WaylandIM] dispatch error: {e}");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }

        info!("[WaylandIM] event loop ended");
        Ok(())
    }
}

impl WaylandInputHost {
    pub fn new(
        processor: ProcessorHandle,
        gui_tx: Sender<GuiEvent>,
        tray_tx: Sender<TrayEvent>,
    ) -> Option<Self> {
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return None;
        }
        if Connection::connect_to_env().is_err() {
            return None;
        }
        Some(Self {
            processor,
            gui_tx,
            tray_tx,
            running: Arc::new(AtomicBool::new(false)),
        })
    }
}

// ---- Keysym → VirtualKey mapping (xkbcommon-based) ----

pub(crate) fn keysym_to_vk(sym: xkb::Keysym) -> VirtualKey {
    let s: u32 = sym.into();
    match s {
        // lowercase letters a-z: 0x61-0x7a → VirtualKey::A..VirtualKey::Z
        0x61..=0x7a => {
            VirtualKey::from_u32(s - 0x61).unwrap_or(VirtualKey::A)
        }
        // uppercase letters A-Z: 0x41-0x5a → VirtualKey::A..VirtualKey::Z
        0x41..=0x5a => {
            VirtualKey::from_u32(s - 0x41).unwrap_or(VirtualKey::A)
        }
        // digits 0-9: 0x30-0x39 → VirtualKey::Digit0..VirtualKey::Digit9
        0x30..=0x39 => {
            VirtualKey::from_u32(s - 0x30 + VirtualKey::Digit0.to_u32()).unwrap_or(VirtualKey::A)
        }

        // Control/function keysyms
        keysyms::KEY_space => VirtualKey::Space,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => VirtualKey::Enter,
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => VirtualKey::Tab,
        keysyms::KEY_BackSpace => VirtualKey::Backspace,
        keysyms::KEY_Escape => VirtualKey::Esc,
        keysyms::KEY_Caps_Lock => VirtualKey::CapsLock,
        keysyms::KEY_Shift_L | keysyms::KEY_Shift_R => VirtualKey::Shift,
        keysyms::KEY_Control_L | keysyms::KEY_Control_R => VirtualKey::Control,
        keysyms::KEY_Alt_L | keysyms::KEY_Alt_R => VirtualKey::Alt,
        keysyms::KEY_Left => VirtualKey::Left,
        keysyms::KEY_Right => VirtualKey::Right,
        keysyms::KEY_Up => VirtualKey::Up,
        keysyms::KEY_Down => VirtualKey::Down,
        keysyms::KEY_Page_Up => VirtualKey::PageUp,
        keysyms::KEY_Page_Down => VirtualKey::PageDown,
        keysyms::KEY_Home => VirtualKey::Home,
        keysyms::KEY_End => VirtualKey::End,
        keysyms::KEY_Delete => VirtualKey::Delete,

        // Punctuation
        keysyms::KEY_grave => VirtualKey::Grave,
        keysyms::KEY_minus => VirtualKey::Minus,
        keysyms::KEY_equal => VirtualKey::Equal,
        keysyms::KEY_bracketleft => VirtualKey::LeftBrace,
        keysyms::KEY_bracketright => VirtualKey::RightBrace,
        keysyms::KEY_backslash => VirtualKey::Backslash,
        keysyms::KEY_semicolon => VirtualKey::Semicolon,
        keysyms::KEY_apostrophe => VirtualKey::Apostrophe,
        keysyms::KEY_comma => VirtualKey::Comma,
        keysyms::KEY_period => VirtualKey::Dot,
        keysyms::KEY_slash => VirtualKey::Slash,

        // Also match shifted punctuation (some layouts might send these)
        keysyms::KEY_asciitilde => VirtualKey::Grave,
        keysyms::KEY_underscore => VirtualKey::Minus,
        keysyms::KEY_plus => VirtualKey::Equal,
        keysyms::KEY_braceleft => VirtualKey::LeftBrace,
        keysyms::KEY_braceright => VirtualKey::RightBrace,
        keysyms::KEY_bar => VirtualKey::Backslash,
        keysyms::KEY_colon => VirtualKey::Semicolon,
        keysyms::KEY_quotedbl => VirtualKey::Apostrophe,
        keysyms::KEY_less => VirtualKey::Comma,
        keysyms::KEY_greater => VirtualKey::Dot,
        keysyms::KEY_question => VirtualKey::Slash,

        // KP equivalents
        keysyms::KEY_KP_0 => VirtualKey::Digit0,
        keysyms::KEY_KP_1 => VirtualKey::Digit1,
        keysyms::KEY_KP_2 => VirtualKey::Digit2,
        keysyms::KEY_KP_3 => VirtualKey::Digit3,
        keysyms::KEY_KP_4 => VirtualKey::Digit4,
        keysyms::KEY_KP_5 => VirtualKey::Digit5,
        keysyms::KEY_KP_6 => VirtualKey::Digit6,
        keysyms::KEY_KP_7 => VirtualKey::Digit7,
        keysyms::KEY_KP_8 => VirtualKey::Digit8,
        keysyms::KEY_KP_9 => VirtualKey::Digit9,
        keysyms::KEY_KP_Space => VirtualKey::Space,
        keysyms::KEY_KP_Tab => VirtualKey::Tab,
        keysyms::KEY_KP_Equal => VirtualKey::Equal,
        keysyms::KEY_KP_Separator => VirtualKey::Comma,
        keysyms::KEY_KP_Decimal => VirtualKey::Dot,
        keysyms::KEY_KP_Divide => VirtualKey::Slash,
        keysyms::KEY_KP_Subtract => VirtualKey::Minus,
        keysyms::KEY_KP_Add => VirtualKey::Equal,
        keysyms::KEY_KP_Delete => VirtualKey::Delete,
        keysyms::KEY_KP_Home => VirtualKey::Home,
        keysyms::KEY_KP_End => VirtualKey::End,
        keysyms::KEY_KP_Left => VirtualKey::Left,
        keysyms::KEY_KP_Right => VirtualKey::Right,
        keysyms::KEY_KP_Up => VirtualKey::Up,
        keysyms::KEY_KP_Down => VirtualKey::Down,
        keysyms::KEY_KP_Page_Up => VirtualKey::PageUp,
        keysyms::KEY_KP_Page_Down => VirtualKey::PageDown,
        keysyms::KEY_KP_Begin => VirtualKey::Digit5,

        // Meta / Super -> treat as Control for now
        keysyms::KEY_Super_L | keysyms::KEY_Super_R => VirtualKey::Control,

        _ => {
            // Try keysym_to_utf8: if it's a Unicode keysym in range 0x01000100..=0x0110FFFF,
            // map back to plain ASCII range if applicable
            let utf8 = xkb::keysym_to_utf8(sym);
            if utf8.len() == 1 {
                if let Some(ch) = utf8.chars().next() {
                    match ch {
                        'a'..='z' => VirtualKey::from_u32(ch as u32 - 'a' as u32).unwrap_or(VirtualKey::A),
                        'A'..='Z' => VirtualKey::from_u32(ch as u32 - 'A' as u32).unwrap_or(VirtualKey::A),
                        '0'..='9' => VirtualKey::from_u32(ch as u32 - '0' as u32 + VirtualKey::Digit0.to_u32()).unwrap_or(VirtualKey::A),
                        _ => VirtualKey::A,
                    }
                } else {
                    VirtualKey::A
                }
            } else {
                VirtualKey::A
            }
        }
    }
}

// ---- Old raw xkb-to-VirtualKey mapping (fallback when no xkb state) ----
// This maps raw xkb keycodes (NOT keysyms) to VirtualKey.
// xkb keycode = evdev scancode + 8.
pub(crate) fn xkb_to_vk_raw(keycode: u32) -> VirtualKey {
    if keycode < 8 {
        return VirtualKey::A;
    }
    let e = keycode - 8;
    match e {
        1 => VirtualKey::Esc,
        2 => VirtualKey::Digit1,
        3 => VirtualKey::Digit2,
        4 => VirtualKey::Digit3,
        5 => VirtualKey::Digit4,
        6 => VirtualKey::Digit5,
        7 => VirtualKey::Digit6,
        8 => VirtualKey::Digit7,
        9 => VirtualKey::Digit8,
        10 => VirtualKey::Digit9,
        11 => VirtualKey::Digit0,
        12 => VirtualKey::Minus,
        13 => VirtualKey::Equal,
        14 => VirtualKey::Backspace,
        15 => VirtualKey::Tab,
        16 => VirtualKey::Q,
        17 => VirtualKey::W,
        18 => VirtualKey::E,
        19 => VirtualKey::R,
        20 => VirtualKey::T,
        21 => VirtualKey::Y,
        22 => VirtualKey::U,
        23 => VirtualKey::I,
        24 => VirtualKey::O,
        25 => VirtualKey::P,
        26 => VirtualKey::LeftBrace,
        27 => VirtualKey::RightBrace,
        28 => VirtualKey::Enter,
        29 => VirtualKey::Control,
        30 => VirtualKey::A,
        31 => VirtualKey::S,
        32 => VirtualKey::D,
        33 => VirtualKey::F,
        34 => VirtualKey::G,
        35 => VirtualKey::H,
        36 => VirtualKey::J,
        37 => VirtualKey::K,
        38 => VirtualKey::L,
        39 => VirtualKey::Semicolon,
        40 => VirtualKey::Apostrophe,
        41 => VirtualKey::Grave,
        42 => VirtualKey::Shift,
        43 => VirtualKey::Backslash,
        44 => VirtualKey::Z,
        45 => VirtualKey::X,
        46 => VirtualKey::C,
        47 => VirtualKey::V,
        48 => VirtualKey::B,
        49 => VirtualKey::N,
        50 => VirtualKey::M,
        51 => VirtualKey::Comma,
        52 => VirtualKey::Dot,
        53 => VirtualKey::Slash,
        54 => VirtualKey::Shift,
        56 => VirtualKey::Alt,
        57 => VirtualKey::Space,
        58 => VirtualKey::CapsLock,
        97 => VirtualKey::Control,
        100 => VirtualKey::Alt,
        103 => VirtualKey::Up,
        104 => VirtualKey::PageUp,
        105 => VirtualKey::Left,
        106 => VirtualKey::Right,
        108 => VirtualKey::Down,
        109 => VirtualKey::PageDown,
        110 => VirtualKey::Home,
        111 => VirtualKey::Delete,
        112 => VirtualKey::End,
        119 => VirtualKey::Esc,
        125 | 126 => VirtualKey::Control,
        _ => VirtualKey::A,
    }
}
