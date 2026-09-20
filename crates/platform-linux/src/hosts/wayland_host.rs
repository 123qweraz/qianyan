use std::collections::HashSet;
use std::error::Error;
use std::ffi::CString;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use log::{error, info, warn};
use qianyan_ime_core::{Config, Rect};

use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::KeyState;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::ZwpInputMethodV2,
    zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use xkbcommon::xkb;
use xkbcommon::xkb::keysyms;

use super::vkbd::Vkbd;
use super::wayland_candidates::{CandContent, CandItem};

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
    deactivate_deadline: Option<Instant>,
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
    keymap_installed: bool,
    forwarded_keys: HashSet<u32>,
    last_key_time: u32,
    seat: Option<WlSeat>,
    held_keys: HashSet<u32>,
    mods_depressed: u32,
    config: Arc<RwLock<Config>>,
    /// 自绘候选框：v2 下用 `zwp_input_popup_surface_v2`，由合成器跟随光标定位。
    cand_surface: Option<WlSurface>,
    cand_popup: Option<ZwpInputPopupSurfaceV2>,
    cand_shm: Option<WlShm>,
    cand_visible: bool,
    cand_w: i32,
    cand_h: i32,
    cand_live_buffers: Vec<WlBuffer>,
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
                state.deactivate_deadline = None;
                // 激活即 grab 键盘以接收按键（无 grab 合成器不会把按键交给 IME）。
                // 不再在 activate 时提交空 preedit+commit：对齐 fcitx5 行为，
                // 激活瞬间的额外提交会触发 COSMIC 焦点抖动/上下文菜单关闭。
                if let Some(ref im) = state.input_method {
                    let grab = im.grab_keyboard(qh, WlUser);
                    state.keyboard_grab = Some(grab);
                }
                state.setup_virtual_keyboard(qh);
                state.refresh_candidates(CandContent::default(), qh);
            }
            Event::Deactivate => {
                info!("[WaylandIM] deactivated (debouncing {}ms)", DEACTIVATE_DEBOUNCE.as_millis());
                if let Some(ref im) = state.input_method {
                    im.set_preedit_string("".into(), 0, 0);
                    im.commit(state.serial);
                    let _ = conn.flush();
                }
                state.deactivate_deadline = Some(Instant::now() + DEACTIVATE_DEBOUNCE);
                state.refresh_candidates(CandContent::default(), qh);
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
                state.refresh_candidates(CandContent::default(), qh);
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
        qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_keyboard_grab_v2::Event;
        match event {
            Event::Keymap { format, fd, size } => {
                info!("[WaylandIM] received keymap (format={:?}, size={})", format, size);
                let raw_fd = fd.into_raw_fd();
                let mut file = unsafe { std::fs::File::from_raw_fd(raw_fd) };
                use std::io::{Read, Seek, SeekFrom};
                // 合成器（尤其 COSMIC）传回的 fd 偏移可能不在 0，read_to_end 会因此
                // 读到空；先 seek 回文件头再读，与 v1 的实现保持一致。
                let mut keymap_bytes = Vec::new();
                if file.seek(SeekFrom::Start(0)).is_err()
                    || file.read_to_end(&mut keymap_bytes).is_err()
                    || keymap_bytes.is_empty()
                {
                    error!("[WaylandIM] failed to read keymap fd (size={size}, read={})", keymap_bytes.len());
                    return;
                }
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
                    info!("[WaylandIM] xkb keymap and state created successfully");
                    state.install_virtual_keymap(&keymap_bytes);
                } else {
                    error!("[WaylandIM] failed to create xkb keymap from string (size={})", keymap_bytes.len());
                }
            }
            Event::Key { serial: _, time, key, state: key_state } => {
                state.last_key_time = time;
                if !state.active {
                    return;
                }

                // Track held keys for keystroke visualization
                if key_state == WEnum::Value(KeyState::Pressed) {
                    state.held_keys.insert(key);
                } else if key_state == WEnum::Value(KeyState::Released) {
                    state.held_keys.remove(&key);
                }
                state.send_keystroke_event();

                if key_state == WEnum::Value(KeyState::Released) {
                    // 通知引擎按键释放，更新内部按键状态（Shift 组合态、tab_down 等）。
                    // 与 v1/evdev 保持一致：release 不下发会让 `shift_used_as_modifier`
                    // 等状态残留，之后会误判 Shift 单击/组合，表现为异常切换。
                    if let Some((vk, _)) = state.resolve_key(key) {
                        let (shift, ctrl, alt) = state.modifier_flags();
                        let _ = state.processor.handle_key(vk, 0, shift, ctrl, alt, false);
                    }
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

                let (shift, ctrl, alt) = state.modifier_flags();
                // Use handle_key_sync to get action + gui + status atomically
                let (action, gui, status) = match state.processor.handle_key_sync(vk, 1, shift, ctrl, alt) {
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

                // Build GUI update + 自绘候选内容
                let (update, cand_content) = if gui.pinyin.is_empty() || !gui.chinese_enabled {
                    (
                        GuiEvent::Update {
                            pinyin: "".into(),
                            candidates: vec![],
                            selected: 0, page: 0, total_pages: 0,
                            sentence: "".into(), cursor_pos: 0,
                            commit_mode: gui.commit_mode.clone(),
                        },
                        CandContent::default(),
                    )
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
                        is_fuzzy: c.is_fuzzy,
                    }).collect();
                    (
                        GuiEvent::Update {
                            pinyin: gui.pinyin.clone(),
                            candidates,
                            selected: gui.selected,
                            page: gui.page,
                            total_pages: gui.total_pages,
                            sentence: gui.sentence.clone(),
                            cursor_pos: gui.cursor_pos,
                            commit_mode: gui.commit_mode.clone(),
                        },
                        CandContent {
                            pinyin: gui.pinyin.clone(),
                            candidates: cand_items,
                            selected: gui.selected,
                        },
                    )
                };
                state.refresh_candidates(cand_content, qh);
                if state.cand_popup.is_some() {
                    // 自绘候选框已接管显示，给 GUI 子进程送空更新避免出现重复候选窗
                    let _ = state.gui_tx.send(GuiEvent::Update {
                        pinyin: String::new(),
                        candidates: vec![],
                        selected: 0, page: 0, total_pages: 0,
                        sentence: String::new(), cursor_pos: 0,
                        commit_mode: gui.commit_mode.clone(),
                    });
                } else {
                    let _ = state.gui_tx.send(update);
                }

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
                state.mods_depressed = mods_depressed;
                if let Some(ref mut xkb_st) = state.xkb_state {
                    xkb_st.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
                // 只有已安装 keymap 之后才能给虚拟键盘下发 modifiers，
                // 否则某些合成器（如 COSMIC）会报 "modifiers sent before keymap"。
                if state.keymap_installed {
                    if let Some(vk) = &state.virtual_keyboard {
                        vk.modifiers(mods_depressed, mods_latched, mods_locked, group);
                    }
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

impl Dispatch<ZwpInputPopupSurfaceV2, WlUser> for WlState {
    fn event(
        _state: &mut Self,
        _: &ZwpInputPopupSurfaceV2,
        event: <ZwpInputPopupSurfaceV2 as Proxy>::Event,
        _: &WlUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_popup_surface_v2::Event;
        if let Event::TextInputRectangle { x, y, width, height } = event {
            log::debug!(
                "[WaylandIM v2] text_input_rectangle: x={x} y={y} {}x{}",
                width,
                height
            );
        }
    }
}

const WL_KEYMAP_FORMAT_XKB_V1: u32 = 1;
const DEACTIVATE_DEBOUNCE: Duration = Duration::from_millis(180);

impl WlState {
    fn candidates_enabled(&self) -> bool {
        let cfg = match self.config.read() {
            Ok(c) => c,
            Err(e) => e.into_inner(),
        };
        // 自绘候选框（zwp_input_popup_surface_v2）只受候选开关控制，
        // 与 show_slint_window（旧的 GUI 候选窗口）无关：用户关掉 Slint 窗口后，
        // 若还要求该开关为 true，自绘候选框也会一并消失。
        cfg.appearance.show_candidates
    }

    /// 依据当前配置与候选内容刷新自绘候选框。
    ///
    /// v2 的 `zwp_input_popup_surface_v2` 由合成器锚定到文本光标处，
    /// 因此这里只需贴图/清空，无需自己计算坐标。
    fn refresh_candidates(&mut self, content: CandContent, qh: &QueueHandle<Self>) {
        let surface = match self.cand_surface.clone() {
            Some(s) => s,
            None => return,
        };

        let visible = self.active && !content.pinyin.is_empty() && self.candidates_enabled();

        if !visible {
            if self.cand_visible {
                surface.attach(None, 0, 0);
                let _ = surface.commit();
                self.cand_visible = false;
            }
            self.cand_w = 0;
            self.cand_h = 0;
            return;
        }

        let cfg = match self.config.read() {
            Ok(c) => c.clone(),
            Err(e) => e.into_inner().clone(),
        };
        let (pixels, width, height) = match super::wayland_candidates::render_slint(&cfg, &content) {
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

        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        let _ = surface.commit();
        self.cand_w = width;
        self.cand_h = height;
        self.cand_visible = true;
        self.cand_live_buffers.push(buffer);
    }

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
        self.keymap_installed = true;
        info!("[WaylandIM] virtual keyboard keymap installed ({} bytes)", keymap.len());
    }

    fn forward_physical_key(&self, evdev_keycode: u32, state: u32) {
        let Some(vk) = &self.virtual_keyboard else {
            return;
        };
        if !self.keymap_installed {
            warn!("[WaylandIM] virtual keyboard keymap not installed; cannot forward key {}", evdev_keycode);
            return;
        }
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

    fn send_keystroke_event(&self) {
        let mut mods = Vec::new();
        if self.mods_depressed & 0x0001 != 0 { mods.push("Shift".into()); }
        if self.mods_depressed & 0x0004 != 0 { mods.push("Ctrl".into()); }
        if self.mods_depressed & 0x0008 != 0 { mods.push("Alt".into()); }
        if self.mods_depressed & 0x0040 != 0 { mods.push("Super".into()); }

        let is_mod = |k: u32| -> bool {
            matches!(k, 29 | 42 | 56 | 97 | 100 | 125 | 126 | 127)
        };

        let keys: Vec<String> = self.held_keys.iter()
            .filter(|k| !is_mod(**k))
            .filter_map(|k| self.resolve_key(*k))
            .map(|(vk, _)| vk.display_name().to_string())
            .collect();

        let _ = self.gui_tx.send(GuiEvent::KeyEvent { keys, modifiers: mods });
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
        let show_preedit = match state.config.read() {
            Ok(c) => c.linux.show_preedit,
            Err(e) => e.into_inner().linux.show_preedit,
        };
        let im = match state.input_method.as_ref() {
            Some(im) => im,
            None => return,
        };

        match action {
            Action::Emit(text) => {
                im.commit_string(text.clone());
                if Self::is_chinese_commit(text) {
                    let _ = state.gui_tx.send(GuiEvent::Committed(text.clone()));
                }
            }
            Action::DeleteAndEmit { delete, insert } => {
                im.delete_surrounding_text(u32::try_from(*delete).unwrap_or(0), 0);
                im.commit_string(insert.clone());
                if Self::is_chinese_commit(insert) {
                    let _ = state.gui_tx.send(GuiEvent::Committed(insert.clone()));
                }
            }
            Action::PassThrough if utf8_text.is_empty()
                || utf8_text.chars().next().is_some_and(|c| c.is_control()) =>
            {
                if let Some(vk) = &state.virtual_keyboard {
                    vk.key(state.last_key_time, key, 1);
                    state.forwarded_keys.insert(key);
                } else if !utf8_text.is_empty() {
                    // 无虚拟键盘时至少把文本转发出去，避免按键被静默吞掉
                    im.commit_string(utf8_text);
                }
            }
            Action::PassThrough => {
                im.commit_string(utf8_text.clone());
                if Self::is_chinese_commit(&utf8_text) {
                    let _ = state.gui_tx.send(GuiEvent::Committed(utf8_text.clone()));
                }
            }
            Action::Alert => {
                Self::play_beep();
            }
            _ => {}
        }

        // Set preedit string
        let preedit = if show_preedit { preedit } else { "" };
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

    /// 上屏文本里含非 ASCII（中文）才算中文上屏，避免每个英文字母都弹通知。
    fn is_chinese_commit(s: &str) -> bool {
        s.chars().any(|c| c > '\u{7f}')
    }

    fn play_beep() {
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

pub struct WaylandInputHost {
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    running: Arc<AtomicBool>,
    vkbd: Option<Arc<std::sync::Mutex<Vkbd>>>,
    conn: Connection,
    config: Arc<RwLock<Config>>,
}

impl InputMethodHost for WaylandInputHost {
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

        // 自绘 candidate input popup (input_popup_surface_v2) 在 COSMIC 下不会被映射，
        // 在 KWin 下行为也不稳定；统一不创建，改由 GUI 子进程（gtk layer-shell）
        // 显示候选窗，KWin/COSMIC 均可用。若后续需要跟随光标的窗口，
        // 参考 yinxing 用 wlr-layer-shell 自绘自锚定实现。
        let (cand_surface, cand_popup): (Option<WlSurface>, Option<ZwpInputPopupSurfaceV2>) = (None, None);

        self.running.store(true, Ordering::SeqCst);
        info!("[WaylandIM] connected, input_method obtained");

        let prev_chinese_enabled = self.processor.get_basic_status()
            .map(|s| s.chinese_enabled).unwrap_or(false);

        let mut state = WlState {
            running: self.running.clone(),
            processor: self.processor.clone(),
            gui_tx: self.gui_tx.clone(),
            tray_tx: self.tray_tx.clone(),
            serial: 0,
            active: false,
            deactivate_deadline: None,
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
            keymap_installed: false,
            forwarded_keys: HashSet::new(),
            last_key_time: 0,
            seat: Some(seat),
            held_keys: HashSet::new(),
            mods_depressed: 0,
            config: self.config.clone(),
            cand_surface,
            cand_popup,
            cand_shm: None,
            cand_visible: false,
            cand_w: 0,
            cand_h: 0,
            cand_live_buffers: Vec::new(),
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

            if state.deactivate_deadline.is_some_and(|d| Instant::now() >= d) {
                info!("[WaylandIM] deactivated after debounce");
                state.active = false;
                state.deactivate_deadline = None;
                state.keyboard_grab = None;
                let _ = state.gui_tx.send(GuiEvent::SetVisible(false));
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
        conn: Connection,
        enable_vkbd: bool,
        config: Arc<RwLock<Config>>,
    ) -> Self {
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

        keysyms::KEY_Super_L | keysyms::KEY_Super_R => None,

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
// xkb keycode = evdev scancode + 8；具体映射统一收敛到 keymap::scancode_to_vk。
pub(crate) fn xkb_to_vk_raw(keycode: u32) -> Option<VirtualKey> {
    keycode.checked_sub(8).and_then(super::keymap::scancode_to_vk)
}
