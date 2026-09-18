use std::error::Error;
use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use log::{error, info};
use qianyan_ime_core::{Config, Rect};

use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{self, KeyState, WlKeyboard};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child};

use wayland_protocols::wp::input_method::zv1::client::{
    zwp_input_method_context_v1::ZwpInputMethodContextV1,
    zwp_input_method_v1::ZwpInputMethodV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use super::wayland_candidates::{CandContent, CandItem, CandStyle};

use xkbcommon::xkb;

use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::processor::Action;
use qianyan_ime_engine::processor::actor::ProcessorHandle;
use qianyan_ime_ui::GuiEvent;
use qianyan_ime_ui::tray::TrayEvent;

use super::wayland_host::{keysym_to_vk, xkb_to_vk_raw};
use super::vkbd::Vkbd;

const DEACTIVATE_DEBOUNCE: Duration = Duration::from_millis(180);

struct WlState {
    _running: Arc<AtomicBool>,
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    active: bool,
    deactivate_deadline: Option<Instant>,
    _input_method: Option<ZwpInputMethodV1>,
    context: Option<ZwpInputMethodContextV1>,
    keyboard: Option<WlKeyboard>,
    context_serial: u32,
    xkb_context: xkb::Context,
    xkb_state: Option<xkb::State>,
    mods_depressed: u32,
    /// keycodes（evdev scancode）其 press 已透传给系统；
    /// 对应的 release 也必须转发，避免 keys 只按下不释放（如 Shift+CapsLock 卡大写）
    forwarded_keys: std::collections::HashSet<u32>,
    config: Arc<RwLock<Config>>,
    /// 自绘候选框（方案 B：绕过 GTK，直接在私有连接上用 layer-shell 渲染）
    _cand_layer_shell: Option<ZwlrLayerShellV1>,
    cand_surface: Option<WlSurface>,
    cand_layer: Option<ZwlrLayerSurfaceV1>,
    cand_shm: Option<WlShm>,
    cand_configured: bool,
    cand_visible: bool,
    cand_w: i32,
    cand_h: i32,
    cand_pending: Option<(WlBuffer, i32, i32)>,
    cand_live_buffers: Vec<WlBuffer>,
}

impl WlState {
    fn candidates_enabled(&self) -> bool {
        let cfg = match self.config.read() {
            Ok(c) => c,
            Err(e) => e.into_inner(),
        };
        cfg.appearance.show_candidates && cfg.linux.show_slint_window
    }

    /// 依据当前配置与候选内容刷新自绘候选框。
    fn refresh_candidates(&mut self, content: CandContent, qh: &QueueHandle<Self>) {
        if self.cand_surface.is_none() || !self.cand_configured {
            return;
        }
        let cfg = match self.config.read() {
            Ok(c) => c.clone(),
            Err(e) => e.into_inner().clone(),
        };
        let visible = !content.pinyin.is_empty() && self.candidates_enabled();

        if !visible {
            if let Some((pending, _, _)) = self.cand_pending.take() {
                pending.destroy();
            }
            if self.cand_visible {
                let surface = self.cand_surface.clone().unwrap();
                surface.attach(None, 0, 0);
                let _ = surface.commit();
                self.cand_visible = false;
            }
            return;
        }

        let style = CandStyle::from_config(&cfg);
        let (pixels, width, height) = match super::wayland_candidates::render(&style, &content) {
            Some(v) => v,
            None => return,
        };
        let shm = match self.cand_shm.clone() {
            Some(s) => s,
            None => return,
        };
        let buffer = match upload_shm(&shm, qh, &pixels, width, height) {
            Some(b) => b,
            None => return,
        };

        let surface = self.cand_surface.clone().unwrap();
        let layer = self.cand_layer.clone().unwrap();

        let mx = cfg.linux.fixed_x.max(0);
        let my = cfg.linux.fixed_y.max(0);
        let (anchor, mt, mr, mb, ml) = match cfg.linux.corner.as_str() {
            "top-right" => (Anchor::Top | Anchor::Right, my, mx, 0, 0),
            "top-left" => (Anchor::Top | Anchor::Left, my, 0, 0, mx),
            "bottom-left" => (Anchor::Bottom | Anchor::Left, 0, 0, my, mx),
            _ => (Anchor::Bottom | Anchor::Right, 0, mx, my, 0),
        };
        layer.set_anchor(anchor);
        layer.set_margin(mt, mr, mb, ml);
        if width != self.cand_w || height != self.cand_h {
            // layer-shell 要求先 ack 新的 configure 再贴对应尺寸的 buffer，
            // 因此尺寸变化时先提交空 buffer 触发 configure，待 configure 到达后再贴。
            if let Some((stale, _, _)) = self.cand_pending.take() {
                stale.destroy();
            }
            self.cand_w = width;
            self.cand_h = height;
            layer.set_size(width.max(0) as u32, height.max(0) as u32);
            self.cand_pending = Some((buffer, width, height));
            let _ = surface.commit();
            self.cand_visible = true;
            return;
        }

        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        let _ = surface.commit();
        self.cand_visible = true;
        self.cand_live_buffers.push(buffer);
    }
}

impl WlState {
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

    fn modifier_flags(&self) -> (bool, bool, bool) {
        match self.xkb_state.as_ref() {
            Some(xkb_st) => (
                xkb_st.mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE),
                xkb_st.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE),
                xkb_st.mod_name_is_active(xkb::MOD_NAME_ALT, xkb::STATE_MODS_EFFECTIVE),
            ),
            None => (
                self.mods_depressed & 0x0001 != 0,
                self.mods_depressed & 0x0004 != 0,
                self.mods_depressed & 0x0008 != 0,
            ),
        }
    }

    fn handle_action(
        &mut self,
        action: &Action,
        conn: &Connection,
        utf8_text: String,
        key_serial: u32,
        time: u32,
        key: u32,
        state: u32,
    ) {
        let ctx = match self.context.as_ref() {
            Some(c) => c.clone(),
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
                if utf8_text.is_empty()
                    || utf8_text.chars().next().is_some_and(|c| c.is_control())
                {
                    ctx.key(key_serial, time, key, state);
                    // 记录透传的 press，供后续 release 配对转发
                    self.forwarded_keys.insert(key);
                } else {
                    ctx.commit_string(self.context_serial, utf8_text);
                }
            }
            Action::Alert => {
                let root = crate::find_project_root();
                let sound_path = root.join("sounds/beep.wav");
                if sound_path.exists() {
                    let _ = std::process::Command::new("canberra-gtk-play")
                        .arg("-f")
                        .arg(sound_path)
                        .spawn()
                        .map(|mut c| c.wait());
                } else {
                    let _ = std::process::Command::new("canberra-gtk-play")
                        .arg("--id=dialog-error")
                        .spawn()
                        .map(|mut c| c.wait());
                }
            }
            _ => {}
        }
        let _ = conn.flush();
    }

    fn send_preedit(&self) {
        // Wayland v1 (KWin virtual keyboard) 下不做拼音预览：preedit 在部分应用里
        // 会残留在文本框中。这里仅持续清空 preedit，输入反馈交给候选窗口。
        let ctx = match self.context.as_ref() {
            Some(c) => c,
            None => return,
        };
        ctx.preedit_string(self.context_serial, String::new(), String::new());
    }

    fn forward_modifiers(
        &self,
        serial: u32,
        mods_depressed: u32,
        mods_latched: u32,
        mods_locked: u32,
        group: u32,
    ) {
        if let Some(ctx) = &self.context {
            ctx.modifiers(serial, mods_depressed, mods_latched, mods_locked, group);
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
                state.deactivate_deadline = None;
                state.context = Some(id);
                state.context_serial = 0;
                if let Some(ref ctx) = state.context {
                    let kbd = ctx.grab_keyboard(qh, WlUser);
                    state.keyboard = Some(kbd);
                    info!("[WaylandIM v1] keyboard grabbed");
                }
            }
            Event::Deactivate { context: _ } => {
                info!("[WaylandIM v1] deactivated (debouncing {}ms)", DEACTIVATE_DEBOUNCE.as_millis());
                state.active = false;
                if let Some(ref ctx) = state.context {
                    ctx.preedit_string(state.context_serial, String::new(), String::new());
                    let _ = conn.flush();
                }
                state.forwarded_keys.clear();
                state.deactivate_deadline = Some(Instant::now() + DEACTIVATE_DEBOUNCE);
                state.refresh_candidates(CandContent::default(), qh);
            }
            _ => {}
        }
    }

    event_created_child!(WlState, ZwpInputMethodV1, [
        0 => (ZwpInputMethodContextV1, WlUser)
    ]);
}

impl Dispatch<ZwpInputMethodContextV1, WlUser> for WlState {
    fn event(
        state: &mut Self,
        _proxy: &ZwpInputMethodContextV1,
        event: <ZwpInputMethodContextV1 as Proxy>::Event,
        _data: &WlUser,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
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
                info!("[WaylandIM v1] reset event received");
                state.processor.reset();
                state.refresh_candidates(CandContent::default(), qh);
                let _ = state.gui_tx.send(GuiEvent::Update {
                    pinyin: String::new(),
                    candidates: vec![],
                    selected: 0,
                    page: 0,
                    total_pages: 0,
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
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                info!("[WaylandIM v1] received keymap (format={:?}, size={})", format, size);
                let raw_fd = fd.into_raw_fd();
                let mut file = unsafe { std::fs::File::from_raw_fd(raw_fd) };
                use std::io::Read;
                let mut keymap_bytes = vec![0u8; size as usize];
                if file.read_exact(&mut keymap_bytes).is_ok() {
                    while keymap_bytes.last().is_some_and(|b| *b == 0) {
                        keymap_bytes.pop();
                    }
                    let keymap_str = String::from_utf8_lossy(&keymap_bytes).into_owned();
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
                if key_state == WEnum::Value(KeyState::Released) {
                    // 通知引擎按键释放，更新内部按键状态（如 tab_down、CapsLock 等）。
                    // 之前 release 完全不下发，会让 tab_down 在按过一次 Tab 后永久为真，
                    // 导致数字 1/2 被当成“以词定字”(word_to_char_shift) 而选字而非选词。
                    if let Some((vk, _)) = state.resolve_key(key) {
                        let (shift, ctrl, alt) = state.modifier_flags();
                        let _ = state.processor.handle_key(vk, 0, shift, ctrl, alt, false);
                    }
                    // 只转发与我们透传过的 press 配对的 release，
                    // 其余 release（如被 IME 吞掉的输入）一律丢弃
                    if state.forwarded_keys.remove(&key) {
                        if let Some(ctx) = &state.context {
                            ctx.key(serial, time, key, 0);
                            let _ = conn.flush();
                        }
                    }
                    return;
                }
                if key_state != WEnum::Value(KeyState::Pressed) {
                    return;
                }

                let (vk, utf8_text) = match state.resolve_key(key) {
                    Some(pair) => pair,
                    None => {
                        // 无法映射为 VirtualKey 的键（F 键、媒体键等）不得被 IME 吞掉，
                        // 直接以物理键转发给应用程序，并记录供 release 配对。
                        state.forwarded_keys.insert(key);
                        if let Some(ctx) = &state.context {
                            ctx.key(serial, time, key, key_state.into());
                            let _ = conn.flush();
                        }
                        return;
                    }
                };

                let (shift, ctrl, alt) = state.modifier_flags();
                let action = match state.processor.handle_key(vk, 1, shift, ctrl, alt, true) {
                    Some(a) => a,
                    None => return,
                };

                if let Some(status) = state.processor.get_basic_status() {
                    let _ = state.tray_tx.send(TrayEvent::SyncStatus {
                        chinese_enabled: status.chinese_enabled,
                        active_profile: status.active_profile.clone(),
                    });
                }

                let (update, _preedit, cand_content) = if let Some(gui) = state.processor.get_gui_snapshot() {
                    if gui.pinyin.is_empty() || !gui.chinese_enabled {
                        (GuiEvent::Update {
                            pinyin: "".into(),
                            candidates: vec![],
                            selected: 0, page: 0, total_pages: 0,
                            sentence: "".into(), cursor_pos: 0,
                            commit_mode: gui.commit_mode.clone(),
                        }, String::new(), CandContent::default())
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
                        let cand_items: Vec<CandItem> = gui.candidates.iter().map(|c| CandItem {
                            label: c.label.clone(),
                            text: c.text.clone(),
                            hint: c.hint.clone(),
                        }).collect();
                        let content = CandContent {
                            pinyin: gui.pinyin.clone(),
                            candidates: cand_items,
                            selected: gui.selected,
                        };
                        (GuiEvent::Update {
                            pinyin: gui.pinyin.clone(),
                            candidates,
                            selected: gui.selected,
                            page: gui.page,
                            total_pages: gui.total_pages,
                            sentence: gui.sentence.clone(),
                            cursor_pos: gui.cursor_pos,
                            commit_mode: gui.commit_mode,
                        }, gui.pinyin, content)
                    }
                } else {
                    return;
                };

                state.handle_action(&action, conn, utf8_text, serial, time, key, key_state.into());

                state.refresh_candidates(cand_content, qh);
                let _ = state.gui_tx.send(update);

                state.send_preedit();
                let _ = conn.flush();
            }
            wl_keyboard::Event::Modifiers {
                serial,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                state.mods_depressed = mods_depressed;
                if let Some(ref mut xkb_st) = state.xkb_state {
                    xkb_st.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
                state.forward_modifiers(serial, mods_depressed, mods_latched, mods_locked, group);
            }
            wl_keyboard::Event::RepeatInfo { rate: _, delay: _ } => {}
            _ => {}
        }
    }
}

impl Dispatch<WlCompositor, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &WlShm,
        _: <WlShm as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShmPool, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &WlShmPool,
        _: <WlShmPool as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, WlUser> for WlState {
    fn event(
        state: &mut Self,
        proxy: &WlBuffer,
        event: <WlBuffer as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            if let Some(pos) = state.cand_live_buffers.iter().position(|b| b.id() == proxy.id()) {
                let buf = state.cand_live_buffers.swap_remove(pos);
                buf.destroy();
            }
        }
    }
}

impl Dispatch<WlSurface, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &ZwlrLayerShellV1,
        _: <ZwlrLayerShellV1 as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, WlUser> for WlState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: <ZwlrLayerSurfaceV1 as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event;
        match event {
            Event::Configure { serial, width, height } => {
                proxy.ack_configure(serial);
                state.cand_configured = true;
                info!("[WaylandIM v1] candidate overlay configured {}x{}", width, height);
                if let Some((buffer, w, h)) = state.cand_pending.take() {
                    if let Some(surface) = state.cand_surface.clone() {
                        surface.attach(Some(&buffer), 0, 0);
                        surface.damage(0, 0, w, h);
                        let _ = surface.commit();
                        state.cand_visible = true;
                        state.cand_live_buffers.push(buffer);
                    }
                }
            }
            Event::Closed => {
                info!("[WaylandIM v1] candidate overlay closed");
                if let Some((pending, _, _)) = state.cand_pending.take() {
                    pending.destroy();
                }
                state.cand_surface = None;
                state.cand_layer = None;
                state.cand_visible = false;
            }
            _ => {}
        }
    }
}

fn upload_shm(
    shm: &WlShm,
    qh: &QueueHandle<WlState>,
    pixels: &[u8],
    width: i32,
    height: i32,
) -> Option<WlBuffer> {
    let stride = width * 4;
    let size = stride as usize * height as usize;
    if pixels.len() < size {
        return None;
    }
    let name = CString::new("qianyan-cand").ok()?;
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::ftruncate(owned.as_raw_fd(), size as libc::off_t) } != 0 {
        return None;
    }
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            owned.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return None;
    }
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), ptr as *mut u8, size) };
    unsafe { libc::munmap(ptr, size) };

    let pool = shm.create_pool(owned.as_fd(), size as i32, qh, WlUser);
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, WlUser);
    pool.destroy();
    Some(buffer)
}

pub struct WaylandInputHostV1 {
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    running: Arc<AtomicBool>,
    vkbd: Option<Arc<std::sync::Mutex<Vkbd>>>,
    conn: Connection,
    config: Arc<RwLock<Config>>,
}

impl InputMethodHost for WaylandInputHostV1 {
    fn set_preedit(&self, _text: &str, _cursor_pos: usize) {}
    fn commit_text(&self, _text: &str) {}
    fn get_cursor_rect(&self) -> Option<Rect> {
        None
    }

    fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let conn = &self.conn;
        let (globals, mut event_queue): (GlobalList, EventQueue<WlState>) =
            registry_queue_init(conn)?;

        let qh = event_queue.handle();

        globals.contents().with_list(|list| {
            for g in list {
                info!("[WaylandIM v1] private-socket global: {} v{}", g.interface, g.version);
            }
        });

        let input_method: ZwpInputMethodV1 = globals.bind(&qh, 1..=1, WlUser)
            .map_err(|e| {
                let msg = format!("no zwp_input_method_v1: {:?}", e);
                Box::<dyn Error>::from(msg)
            })?;

        self.running.store(true, Ordering::SeqCst);
        info!("[WaylandIM v1] connected, waiting for activate...");

        // 自绘候选框：在同一个私有连接上创建 Overlay layer 表面。
        let cand_layer_shell: Option<ZwlrLayerShellV1> = globals.bind(&qh, 1..=4, WlUser).ok();
        let cand_shm: Option<WlShm> = globals.bind(&qh, 1..=1, WlUser).ok();
        let cand_compositor: Option<WlCompositor> = globals.bind(&qh, 1..=6, WlUser).ok();
        let (cand_surface, cand_layer) =
            match (cand_compositor.as_ref(), cand_layer_shell.as_ref()) {
                (Some(comp), Some(shell)) => {
                    let surface = comp.create_surface(&qh, WlUser);
                    let layer = shell.get_layer_surface(
                        &surface,
                        None,
                        Layer::Overlay,
                        "qianyan-candidates".to_string(),
                        &qh,
                        WlUser,
                    );
                    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                    layer.set_exclusive_zone(0);
                    layer.set_size(1, 1);
                    layer.set_anchor(Anchor::Bottom | Anchor::Right);
                    layer.set_margin(0, 10, 10, 0);
                    let _ = surface.commit();
                    info!("[WaylandIM v1] candidate overlay created");
                    (Some(surface), Some(layer))
                }
                _ => {
                    info!("[WaylandIM v1] candidate overlay unavailable");
                    (None, None)
                }
            };

        let mut state = WlState {
            _running: self.running.clone(),
            processor: self.processor.clone(),
            gui_tx: self.gui_tx.clone(),
            tray_tx: self.tray_tx.clone(),
            active: false,
            deactivate_deadline: None,
            _input_method: Some(input_method),
            context: None,
            keyboard: None,
            context_serial: 0,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_state: None,
            mods_depressed: 0,
            forwarded_keys: std::collections::HashSet::new(),
            config: self.config.clone(),
            _cand_layer_shell: cand_layer_shell,
            cand_surface,
            cand_layer,
            cand_shm,
            cand_configured: false,
            cand_visible: false,
            cand_w: 0,
            cand_h: 0,
            cand_pending: None,
            cand_live_buffers: Vec::new(),
        };

        let _ = event_queue.roundtrip(&mut state)?;
        let _ = conn.flush();

        loop {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }

            // Check deactivate debounce deadline
            if state.deactivate_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                info!("[WaylandIM v1] deactivated after debounce");
                state.deactivate_deadline = None;
                if let Some(ctx) = state.context.take() {
                    ctx.destroy();
                }
                state.keyboard = None;
                state.forwarded_keys.clear();
                let _ = state.gui_tx.send(GuiEvent::SetVisible(false));
            }

            let _ = conn.flush();
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                error!("[WaylandIM v1] dispatch error: {e}");
                break;
            }
            if self.running.load(Ordering::SeqCst) {
                if let Some(guard) = conn.prepare_read() {
                    if let Err(e) = guard.read() {
                        error!("[WaylandIM v1] read error: {e}");
                        break;
                    }
                }
                if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                    error!("[WaylandIM v1] dispatch error: {e}");
                    break;
                }
            }
        }

        info!("[WaylandIM v1] event loop ended");
        Ok(())
    }
}

impl WaylandInputHostV1 {
    pub fn new(
        processor: ProcessorHandle,
        gui_tx: Sender<GuiEvent>,
        tray_tx: Sender<TrayEvent>,
        conn: Connection,
        enable_vkbd: bool,
        config: Arc<RwLock<Config>>,
    ) -> Self {
        // In KWin virtual keyboard mode (WAYLAND_SOCKET), avoid creating evdev uinput
        // device — all text goes through the Wayland input-method protocol.
        let vkbd = if enable_vkbd {
            Vkbd::new_wayland().ok().map(|v| Arc::new(std::sync::Mutex::new(v)))
        } else {
            None
        };
        Self {
            processor,
            gui_tx,
            tray_tx,
            running: Arc::new(AtomicBool::new(false)),
            vkbd,
            conn,
            config,
        }
    }

    pub fn vkbd(&self) -> Option<Arc<std::sync::Mutex<Vkbd>>> {
        self.vkbd.clone()
    }
}
