use std::collections::HashSet;
use std::error::Error;
use std::io::{Seek, SeekFrom, Write};
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
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use xkbcommon::xkb;
use xkbcommon::xkb::keysyms;

use super::vkbd::Vkbd;

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
    virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1>,
    virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    virtual_keymap: Option<std::fs::File>,
    forwarded_keys: HashSet<u32>,
    last_key_time: u32,
    seat: Option<WlSeat>,
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
                    im.set_preedit_string(String::new(), 0, 0);
                    im.commit(state.serial);
                    let _ = conn.flush();
                }
                state.setup_virtual_keyboard(qh);
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
                let mut keymap_bytes = Vec::new();
                if file.read_to_end(&mut keymap_bytes).is_ok() {
                    let keymap_str = String::from_utf8_lossy(
                        keymap_bytes.strip_suffix(&[0]).unwrap_or(&keymap_bytes),
                    ).into_owned();
                    if let Some(keymap) = xkb::Keymap::new_from_string(
                        &state.xkb_context,
                        keymap_str,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    ) {
                        state.xkb_state = Some(xkb::State::new(&keymap));
                        info!("[WaylandIM] xkb keymap and state created successfully");
                        state.install_virtual_keymap(&keymap_bytes);
                    } else {
                        error!("[WaylandIM] failed to create xkb keymap from string");
                    }
                } else {
                    error!("[WaylandIM] failed to read keymap fd");
                }
            }
            Event::Key { serial: _, time, key, state: key_state } => {
                state.last_key_time = time;
                if !state.active {
                    return;
                }
                if key_state == WEnum::Value(KeyState::Released) {
                    if state.forwarded_keys.remove(&key) {
                        state.forward_physical_key(key, 0);
                    }
                    return;
                }
                if key_state != WEnum::Value(KeyState::Pressed) {
                    return;
                }

                let (vk, utf8_text) = match state.resolve_key(key) {
                    Some(pair) => pair,
                    None => return,
                };

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
                if let Some(vk) = &state.virtual_keyboard {
                    vk.modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            Event::RepeatInfo { rate: _, delay: _ } => {}
            _ => {}
        }
    }
}

impl Dispatch<ZwpVirtualKeyboardManagerV1, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &ZwpVirtualKeyboardManagerV1,
        _event: <ZwpVirtualKeyboardManagerV1 as Proxy>::Event,
        _data: &WlUser,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpVirtualKeyboardV1, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &ZwpVirtualKeyboardV1,
        _event: <ZwpVirtualKeyboardV1 as Proxy>::Event,
        _data: &WlUser,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

const WL_KEYMAP_FORMAT_XKB_V1: u32 = 1;

impl WlState {
    fn setup_virtual_keyboard(&mut self, qh: &QueueHandle<Self>) {
        let (Some(manager), Some(seat)) = (&self.virtual_keyboard_manager, &self.seat) else {
            return;
        };
        if self.virtual_keyboard.is_some() {
            return;
        }
        self.virtual_keyboard = Some(manager.create_virtual_keyboard(seat, qh, WlUser));
        info!("[WaylandIM] virtual keyboard created for key forwarding");
    }

    fn install_virtual_keymap(&mut self, keymap: &[u8]) {
        let Some(vk) = &self.virtual_keyboard else {
            return;
        };

        let mut file = match Self::tempfile() {
            Ok(file) => file,
            Err(e) => {
                error!("[WaylandIM] failed to create virtual keyboard keymap fd: {e}");
                return;
            }
        };
        if file.set_len(keymap.len() as u64)
            .and_then(|_| file.write_all(keymap))
            .and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()))
            .is_err()
        {
            error!("[WaylandIM] failed to write virtual keyboard keymap");
            return;
        }

        use std::os::fd::AsFd;
        vk.keymap(WL_KEYMAP_FORMAT_XKB_V1, file.as_fd(), keymap.len() as u32);
        self.virtual_keymap = Some(file);
        info!("[WaylandIM] virtual keyboard keymap installed ({} bytes)", keymap.len());
    }

    fn forward_physical_key(&self, evdev_keycode: u32, state: u32) {
        let Some(vk) = &self.virtual_keyboard else {
            return;
        };
        vk.key(self.last_key_time, evdev_keycode, state);
    }

    fn tempfile() -> std::io::Result<std::fs::File> {
        let name = std::ffi::CString::new("qianyan-vk")
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad name"))?;
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    #[inline]
    fn resolve_key(&self, keycode: u32) -> Option<(VirtualKey, String)> {
        if let Some(ref xkb_st) = self.xkb_state {
            let xkb_keycode = xkb::Keycode::new(keycode + 8);
            let sym = xkb_st.key_get_one_sym(xkb_keycode);
            let utf8 = xkb_st.key_get_utf8(xkb_keycode);
            keysym_to_vk(sym).map(|vk| (vk, utf8))
        } else {
            xkb_to_vk_raw(keycode).map(|vk| (vk, String::new()))
        }
    }

    fn apply_state(
        state: &mut Self,
        action: &Action,
        conn: &Connection,
        utf8_text: String,
        key: u32,
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
                if state.virtual_keyboard.is_some() {
                    state.forward_physical_key(key, 1);
                    state.forwarded_keys.insert(key);
                }
            }
            Action::PassThrough => {
                if state.virtual_keyboard.is_some() && utf8_text.is_empty() {
                    state.forward_physical_key(key, 1);
                    state.forwarded_keys.insert(key);
                } else {
                    im.commit_string(utf8_text);
                }
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
    vkbd: Option<Arc<std::sync::Mutex<Vkbd>>>,
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

        let virtual_keyboard_manager: Option<ZwpVirtualKeyboardManagerV1> =
            globals.bind(&qh, 1..=1, WlUser).ok().inspect(|_| {
                info!("[WaylandIM] zwp_virtual_keyboard_manager_v1 available");
            });
        if virtual_keyboard_manager.is_none() {
            warn!("[WaylandIM] zwp_virtual_keyboard_manager_v1 not available; key forwarding will fall back to uinput/clipboard");
        }

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
            virtual_keyboard_manager,
            virtual_keyboard: None,
            virtual_keymap: None,
            forwarded_keys: HashSet::new(),
            last_key_time: 0,
            seat: Some(seat),
        };

        if let Err(e) = event_queue.roundtrip(&mut state) {
            error!("[WaylandIM] initial roundtrip failed: {e}");
            return Err(e.into());
        }

        info!("[WaylandIM] event loop started");

        loop {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            let _ = conn.flush();
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                error!("[WaylandIM] dispatch error: {e}");
                break;
            }
            if self.running.load(Ordering::SeqCst) {
                if let Some(guard) = conn.prepare_read() {
                    if let Err(e) = guard.read() {
                        error!("[WaylandIM] read error: {e}");
                        break;
                    }
                }
                if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                    error!("[WaylandIM] dispatch error: {e}");
                    break;
                }
            }
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
        let has_conn = std::env::var("WAYLAND_DISPLAY").is_ok()
            || std::env::var("WAYLAND_SOCKET").is_ok();
        if !has_conn {
            return None;
        }
        if Connection::connect_to_env().is_err() {
            return None;
        }
        let vkbd = Vkbd::new_wayland().ok().map(|v| Arc::new(std::sync::Mutex::new(v)));
        Some(Self {
            processor,
            gui_tx,
            tray_tx,
            running: Arc::new(AtomicBool::new(false)),
            vkbd,
        })
    }

    pub fn vkbd(&self) -> Option<Arc<std::sync::Mutex<Vkbd>>> {
        self.vkbd.clone()
    }
}

// ---- Keysym → VirtualKey mapping (xkbcommon-based) ----

pub(crate) fn keysym_to_vk(sym: xkb::Keysym) -> Option<VirtualKey> {
    let s: u32 = sym.into();
    match s {
        0x61..=0x7a => VirtualKey::from_u32(s - 0x61),
        0x41..=0x5a => VirtualKey::from_u32(s - 0x41),
        0x30..=0x39 => VirtualKey::from_u32(s - 0x30 + VirtualKey::Digit0.to_u32()),

        keysyms::KEY_space => Some(VirtualKey::Space),
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(VirtualKey::Enter),
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Some(VirtualKey::Tab),
        keysyms::KEY_BackSpace => Some(VirtualKey::Backspace),
        keysyms::KEY_Escape => Some(VirtualKey::Esc),
        keysyms::KEY_Caps_Lock => Some(VirtualKey::CapsLock),
        keysyms::KEY_Shift_L | keysyms::KEY_Shift_R => Some(VirtualKey::Shift),
        keysyms::KEY_Control_L | keysyms::KEY_Control_R => Some(VirtualKey::Control),
        keysyms::KEY_Alt_L | keysyms::KEY_Alt_R => Some(VirtualKey::Alt),
        keysyms::KEY_Left => Some(VirtualKey::Left),
        keysyms::KEY_Right => Some(VirtualKey::Right),
        keysyms::KEY_Up => Some(VirtualKey::Up),
        keysyms::KEY_Down => Some(VirtualKey::Down),
        keysyms::KEY_Page_Up => Some(VirtualKey::PageUp),
        keysyms::KEY_Page_Down => Some(VirtualKey::PageDown),
        keysyms::KEY_Home => Some(VirtualKey::Home),
        keysyms::KEY_End => Some(VirtualKey::End),
        keysyms::KEY_Delete => Some(VirtualKey::Delete),

        keysyms::KEY_grave => Some(VirtualKey::Grave),
        keysyms::KEY_minus => Some(VirtualKey::Minus),
        keysyms::KEY_equal => Some(VirtualKey::Equal),
        keysyms::KEY_bracketleft => Some(VirtualKey::LeftBrace),
        keysyms::KEY_bracketright => Some(VirtualKey::RightBrace),
        keysyms::KEY_backslash => Some(VirtualKey::Backslash),
        keysyms::KEY_semicolon => Some(VirtualKey::Semicolon),
        keysyms::KEY_apostrophe => Some(VirtualKey::Apostrophe),
        keysyms::KEY_comma => Some(VirtualKey::Comma),
        keysyms::KEY_period => Some(VirtualKey::Dot),
        keysyms::KEY_slash => Some(VirtualKey::Slash),

        keysyms::KEY_asciitilde => Some(VirtualKey::Grave),
        keysyms::KEY_underscore => Some(VirtualKey::Minus),
        keysyms::KEY_plus => Some(VirtualKey::Equal),
        keysyms::KEY_braceleft => Some(VirtualKey::LeftBrace),
        keysyms::KEY_braceright => Some(VirtualKey::RightBrace),
        keysyms::KEY_bar => Some(VirtualKey::Backslash),
        keysyms::KEY_colon => Some(VirtualKey::Semicolon),
        keysyms::KEY_quotedbl => Some(VirtualKey::Apostrophe),
        keysyms::KEY_less => Some(VirtualKey::Comma),
        keysyms::KEY_greater => Some(VirtualKey::Dot),
        keysyms::KEY_question => Some(VirtualKey::Slash),

        keysyms::KEY_KP_0 => Some(VirtualKey::Digit0),
        keysyms::KEY_KP_1 => Some(VirtualKey::Digit1),
        keysyms::KEY_KP_2 => Some(VirtualKey::Digit2),
        keysyms::KEY_KP_3 => Some(VirtualKey::Digit3),
        keysyms::KEY_KP_4 => Some(VirtualKey::Digit4),
        keysyms::KEY_KP_5 => Some(VirtualKey::Digit5),
        keysyms::KEY_KP_6 => Some(VirtualKey::Digit6),
        keysyms::KEY_KP_7 => Some(VirtualKey::Digit7),
        keysyms::KEY_KP_8 => Some(VirtualKey::Digit8),
        keysyms::KEY_KP_9 => Some(VirtualKey::Digit9),
        keysyms::KEY_KP_Space => Some(VirtualKey::Space),
        keysyms::KEY_KP_Tab => Some(VirtualKey::Tab),
        keysyms::KEY_KP_Equal => Some(VirtualKey::Equal),
        keysyms::KEY_KP_Separator => Some(VirtualKey::Comma),
        keysyms::KEY_KP_Decimal => Some(VirtualKey::Dot),
        keysyms::KEY_KP_Divide => Some(VirtualKey::Slash),
        keysyms::KEY_KP_Subtract => Some(VirtualKey::Minus),
        keysyms::KEY_KP_Add => Some(VirtualKey::Equal),
        keysyms::KEY_KP_Delete => Some(VirtualKey::Delete),
        keysyms::KEY_KP_Home => Some(VirtualKey::Home),
        keysyms::KEY_KP_End => Some(VirtualKey::End),
        keysyms::KEY_KP_Left => Some(VirtualKey::Left),
        keysyms::KEY_KP_Right => Some(VirtualKey::Right),
        keysyms::KEY_KP_Up => Some(VirtualKey::Up),
        keysyms::KEY_KP_Down => Some(VirtualKey::Down),
        keysyms::KEY_KP_Page_Up => Some(VirtualKey::PageUp),
        keysyms::KEY_KP_Page_Down => Some(VirtualKey::PageDown),
        keysyms::KEY_KP_Begin => Some(VirtualKey::Digit5),

        keysyms::KEY_Super_L | keysyms::KEY_Super_R => Some(VirtualKey::Control),

        _ => {
            let utf8 = xkb::keysym_to_utf8(sym);
            if utf8.len() == 1 {
                if let Some(ch) = utf8.chars().next() {
                    match ch {
                        'a'..='z' => VirtualKey::from_u32(ch as u32 - 'a' as u32),
                        'A'..='Z' => VirtualKey::from_u32(ch as u32 - 'A' as u32),
                        '0'..='9' => VirtualKey::from_u32(ch as u32 - '0' as u32 + VirtualKey::Digit0.to_u32()),
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

// ---- Old raw xkb-to-VirtualKey mapping (fallback when no xkb state) ----
// This maps raw xkb keycodes (NOT keysyms) to VirtualKey.
// xkb keycode = evdev scancode + 8.
pub(crate) fn xkb_to_vk_raw(keycode: u32) -> Option<VirtualKey> {
    if keycode < 8 {
        return None;
    }
    let e = keycode - 8;
    match e {
        1 => Some(VirtualKey::Esc),
        2 => Some(VirtualKey::Digit1),
        3 => Some(VirtualKey::Digit2),
        4 => Some(VirtualKey::Digit3),
        5 => Some(VirtualKey::Digit4),
        6 => Some(VirtualKey::Digit5),
        7 => Some(VirtualKey::Digit6),
        8 => Some(VirtualKey::Digit7),
        9 => Some(VirtualKey::Digit8),
        10 => Some(VirtualKey::Digit9),
        11 => Some(VirtualKey::Digit0),
        12 => Some(VirtualKey::Minus),
        13 => Some(VirtualKey::Equal),
        14 => Some(VirtualKey::Backspace),
        15 => Some(VirtualKey::Tab),
        16 => Some(VirtualKey::Q),
        17 => Some(VirtualKey::W),
        18 => Some(VirtualKey::E),
        19 => Some(VirtualKey::R),
        20 => Some(VirtualKey::T),
        21 => Some(VirtualKey::Y),
        22 => Some(VirtualKey::U),
        23 => Some(VirtualKey::I),
        24 => Some(VirtualKey::O),
        25 => Some(VirtualKey::P),
        26 => Some(VirtualKey::LeftBrace),
        27 => Some(VirtualKey::RightBrace),
        28 => Some(VirtualKey::Enter),
        29 => Some(VirtualKey::Control),
        30 => Some(VirtualKey::A),
        31 => Some(VirtualKey::S),
        32 => Some(VirtualKey::D),
        33 => Some(VirtualKey::F),
        34 => Some(VirtualKey::G),
        35 => Some(VirtualKey::H),
        36 => Some(VirtualKey::J),
        37 => Some(VirtualKey::K),
        38 => Some(VirtualKey::L),
        39 => Some(VirtualKey::Semicolon),
        40 => Some(VirtualKey::Apostrophe),
        41 => Some(VirtualKey::Grave),
        42 => Some(VirtualKey::Shift),
        43 => Some(VirtualKey::Backslash),
        44 => Some(VirtualKey::Z),
        45 => Some(VirtualKey::X),
        46 => Some(VirtualKey::C),
        47 => Some(VirtualKey::V),
        48 => Some(VirtualKey::B),
        49 => Some(VirtualKey::N),
        50 => Some(VirtualKey::M),
        51 => Some(VirtualKey::Comma),
        52 => Some(VirtualKey::Dot),
        53 => Some(VirtualKey::Slash),
        54 => Some(VirtualKey::Shift),
        56 => Some(VirtualKey::Alt),
        57 => Some(VirtualKey::Space),
        58 => Some(VirtualKey::CapsLock),
        97 => Some(VirtualKey::Control),
        100 => Some(VirtualKey::Alt),
        103 => Some(VirtualKey::Up),
        104 => Some(VirtualKey::PageUp),
        105 => Some(VirtualKey::Left),
        106 => Some(VirtualKey::Right),
        108 => Some(VirtualKey::Down),
        109 => Some(VirtualKey::PageDown),
        110 => Some(VirtualKey::Home),
        111 => Some(VirtualKey::Delete),
        112 => Some(VirtualKey::End),
        119 => Some(VirtualKey::Esc),
        125 | 126 => Some(VirtualKey::Control),
        _ => None,
    }
}
