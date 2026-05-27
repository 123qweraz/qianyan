use std::error::Error;
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use log::{error, info};
use qianyan_ime_core::Rect;

use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::wl_keyboard::{self, KeyState, WlKeyboard};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

use wayland_protocols::wp::input_method::zv1::client::{
    zwp_input_method_context_v1::ZwpInputMethodContextV1,
    zwp_input_method_v1::ZwpInputMethodV1,
};

use xkbcommon::xkb;

use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::processor::Action;
use qianyan_ime_engine::Processor;
use qianyan_ime_ui::GuiEvent;

use super::wayland_host::{keysym_to_vk, xkb_to_vk_raw};

struct WlState {
    _running: Arc<AtomicBool>,
    processor: Arc<Mutex<Processor>>,
    gui_tx: Sender<GuiEvent>,
    active: bool,
    _input_method: Option<ZwpInputMethodV1>,
    context: Option<ZwpInputMethodContextV1>,
    keyboard: Option<WlKeyboard>,
    context_serial: u32,
    xkb_context: xkb::Context,
    xkb_state: Option<xkb::State>,
}

impl WlState {
    fn resolve_key(&self, keycode: u32) -> (VirtualKey, String) {
        if let Some(ref xkb_st) = self.xkb_state {
            let xkb_keycode = xkb::Keycode::new(keycode + 8);
            let sym = xkb_st.key_get_one_sym(xkb_keycode);
            let utf8 = xkb_st.key_get_utf8(xkb_keycode);
            let vk = keysym_to_vk(sym);
            (vk, utf8)
        } else {
            (xkb_to_vk_raw(keycode), String::new())
        }
    }

    fn handle_action(
        &self,
        action: &Action,
        conn: &Connection,
        utf8_text: String,
        key_serial: u32,
        time: u32,
        key: u32,
        state: u32,
    ) {
        let ctx = match self.context.as_ref() {
            Some(c) => c,
            None => return,
        };
        match action {
            Action::Emit(text) => {
                ctx.commit_string(self.context_serial, text.clone());
            }
            Action::DeleteAndEmit { delete, insert } => {
                ctx.delete_surrounding_text(-(i32::try_from(*delete).unwrap_or(0)), 0);
                ctx.commit_string(self.context_serial, insert.clone());
            }
            Action::PassThrough => {
                if !utf8_text.is_empty() {
                    ctx.commit_string(self.context_serial, utf8_text);
                } else {
                    // Forward non-text key to the application
                    ctx.key(key_serial, time, key, state);
                }
            }
            Action::Alert => {
                // Sound alert
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
            _ => {}
        }
        let _ = conn.flush();
    }

    fn send_preedit(&self) {
        let ctx = match self.context.as_ref() {
            Some(c) => c,
            None => return,
        };
        let preedit = {
            let guard = match self.processor.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            qianyan_ime_engine::compositor::Compositor::get_preedit(&guard.ctx)
        };
        if preedit.is_empty() {
            ctx.preedit_string(self.context_serial, String::new(), String::new());
        } else {
            ctx.preedit_string(self.context_serial, preedit, String::new());
        }
    }
}

struct WlUser;

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

impl Dispatch<ZwpInputMethodV1, WlUser> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwpInputMethodV1,
        event: <ZwpInputMethodV1 as Proxy>::Event,
        _data: &WlUser,
        conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols::wp::input_method::zv1::client::zwp_input_method_v1::Event;
        match event {
            Event::Activate { id } => {
                info!("[WaylandIM v1] activated");
                state.active = true;
                state.context = Some(id);
                state.context_serial = 0;
                if let Some(ref ctx) = state.context {
                    let kbd = ctx.grab_keyboard(qh, WlUser);
                    state.keyboard = Some(kbd);
                    info!("[WaylandIM v1] keyboard grabbed");
                }
            }
            Event::Deactivate { context } => {
                info!("[WaylandIM v1] deactivated");
                state.active = false;
                if let Some(ref ctx) = state.context {
                    ctx.preedit_string(state.context_serial, String::new(), String::new());
                    let _ = conn.flush();
                }
                state.keyboard = None;
                context.destroy();
                state.context = None;
                let _ = state.gui_tx.send(GuiEvent::SetVisible(false));
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodContextV1, WlUser> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwpInputMethodContextV1,
        event: <ZwpInputMethodContextV1 as Proxy>::Event,
        _data: &WlUser,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols::wp::input_method::zv1::client::zwp_input_method_context_v1::Event;
        match event {
            Event::SurroundingText { text: _, cursor, anchor } => {
                info!("[WaylandIM v1] surrounding_text: cursor={} anchor={}", cursor, anchor);
            }
            Event::ContentType { hint, purpose } => {
                info!("[WaylandIM v1] content_type: hint={:?} purpose={:?}", hint, purpose);
            }
            Event::CommitState { serial } => {
                state.context_serial = serial;
            }
            Event::Reset => {
                // Text input was reset, clear preedit
                if let Ok(mut p) = state.processor.lock() {
                    p.reset();
                }
                let _ = state.gui_tx.send(GuiEvent::Update {
                    pinyin: String::new(),
                    candidates: vec![],
                    selected: 0,
                    sentence: String::new(),
                    cursor_pos: 0,
                    commit_mode: String::new(),
                });
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

impl Dispatch<WlKeyboard, WlUser> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &WlKeyboard,
        event: <WlKeyboard as Proxy>::Event,
        _data: &WlUser,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                info!("[WaylandIM v1] received keymap (format={:?}, size={})", format, size);
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
                        info!("[WaylandIM v1] xkb keymap and state created");
                    } else {
                        error!("[WaylandIM v1] failed to create xkb keymap");
                    }
                } else {
                    error!("[WaylandIM v1] failed to read keymap fd");
                }
            }
            wl_keyboard::Event::Key { serial, time, key, state: key_state } => {
                if !state.active {
                    return;
                }
                if key_state != WEnum::Value(KeyState::Pressed) {
                    return;
                }

                let (vk, utf8_text) = state.resolve_key(key);

                let mut guard = match state.processor.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };

                let action = guard.handle_key(vk, 1, false, false, false);
                let buffer = guard.ctx.session.buffer.clone();
                let candidates: Vec<qianyan_ime_ui::DisplayCandidate> =
                    guard.ctx.session.candidates.iter().take(10).enumerate().map(|(i, c)| {
                        qianyan_ime_ui::DisplayCandidate {
                            text: c.text.to_string(),
                            label: format!("{}.", i + 1),
                            hint: c.hint.to_string(),
                            full_display: format!("{}.{}({})", i + 1, c.text, c.hint),
                        }
                    }).collect();
                let selected = guard.ctx.session.selected;
                let _preedit = qianyan_ime_engine::compositor::Compositor::get_preedit(&guard.ctx);
                drop(guard);

                state.handle_action(&action, conn, utf8_text, serial, time, key, key_state.into());

                let _ = state.gui_tx.send(GuiEvent::MoveTo { x: 0, y: 0 });
                let _ = state.gui_tx.send(GuiEvent::Update {
                    pinyin: buffer,
                    candidates,
                    selected,
                    sentence: String::new(),
                    cursor_pos: 0,
                    commit_mode: String::new(),
                });

                state.send_preedit();
                let _ = conn.flush();
            }
            wl_keyboard::Event::Modifiers {
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
            wl_keyboard::Event::RepeatInfo { rate: _, delay: _ } => {}
            _ => {}
        }
    }
}

pub struct WaylandInputHostV1 {
    processor: Arc<Mutex<Processor>>,
    gui_tx: Sender<GuiEvent>,
    running: Arc<AtomicBool>,
}

impl InputMethodHost for WaylandInputHostV1 {
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

        let input_method: ZwpInputMethodV1 = globals.bind(&qh, 1..=1, WlUser)
            .map_err(|e| {
                let msg = format!("no zwp_input_method_v1: {:?}", e);
                Box::<dyn Error>::from(msg)
            })?;

        self.running.store(true, Ordering::SeqCst);
        info!("[WaylandIM v1] connected, waiting for activate...");

        let mut state = WlState {
            _running: self.running.clone(),
            processor: self.processor.clone(),
            gui_tx: self.gui_tx.clone(),
            active: false,
            _input_method: Some(input_method),
            context: None,
            keyboard: None,
            context_serial: 0,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_state: None,
        };

        let _ = event_queue.dispatch_pending(&mut state);
        let _ = conn.flush();

        loop {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                error!("[WaylandIM v1] dispatch error: {e}");
                break;
            }
            let _ = conn.flush();
            std::thread::sleep(std::time::Duration::from_millis(4));
        }

        info!("[WaylandIM v1] event loop ended");
        Ok(())
    }
}

impl WaylandInputHostV1 {
    pub fn new(
        processor: Arc<Mutex<Processor>>,
        gui_tx: Sender<GuiEvent>,
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
            running: Arc::new(AtomicBool::new(false)),
        })
    }
}
