use std::collections::HashMap;
use std::error::Error;
use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
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

use super::wayland_candidates::{CandContent, CandItem};

use xkbcommon::xkb;

use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::processor::Action;
use qianyan_ime_engine::processor::actor::{ProcessorHandle, diag_file};
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
    cand_compositor: Option<WlCompositor>,
    cand_surface: Option<WlSurface>,
    cand_layer: Option<ZwlrLayerSurfaceV1>,
    cand_shm: Option<WlShm>,
    cand_configured: bool,
    cand_visible: bool,
    cand_w: i32,
    cand_h: i32,
    cand_pending: Option<(WlBuffer, i32, i32)>,
    cand_live_buffers: Vec<WlBuffer>,
    /// 键盘自动重复（zwp_input_method_v1 不会替输入法做 key repeat，
    /// KWin 里按住退格要能连续删必须自己合成 repeat）
    repeat_rate: u32,
    repeat_delay_ms: u64,
    held_keys: HashMap<u32, HeldKey>,
    diag_stage: Arc<Mutex<&'static str>>,
    /// 最近一次分发的 Wayland 事件，用于卡死时定位是哪个处理器
    last_event: &'static str,
}

struct HeldKey {
    serial: u32,
    time: u32,
    next_repeat: Option<Instant>,
}

impl WlState {
    /// 依据当前配置与候选内容刷新自绘候选框。
    ///
    /// 关键时序：wlr-layer-shell 的表面一旦用空 buffer commit 取消映射，再次显示
    /// 必须重新走“set_size → configure → ack → 贴同尺寸 buffer”流程；直接往旧表面
    /// 再贴 buffer 会被 KWin 忽略（表现就是“只有第一次置顶，之后再打字都被普通窗口
    /// 挡着”）。因此这里隐藏时直接拆掉表面，显示时重建一个全新表面（保证 KWin 始终
    /// 按 layer-shell OVERLAY 规则压在所有普通窗口之上）。
    fn refresh_candidates(&mut self, content: CandContent, qh: &QueueHandle<Self>) {
        let cfg = {
            let g = self.config.read();
            match g {
                Ok(c) => c.clone(),
                Err(e) => e.into_inner().clone(),
            }
        };
        let visible = !content.pinyin.is_empty() && cfg.appearance.show_candidates;

        if !visible {
            self.hide_candidates();
            return;
        }

        // 显示前确保表面上存在（首次 / 隐藏后重建）
        if self.cand_surface.is_none() {
            self.ensure_candidate_surface(&cfg, qh);
        }
        let surface = match self.cand_surface.clone() {
            Some(s) => s,
            None => return,
        };
        let layer = match self.cand_layer.clone() {
            Some(l) => l,
            None => return,
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

        if self.cand_pending.is_some() {
            // 上一个尺寸变更的 configure 还没到达：此时直接 attach 违反 layer-shell
            // 时序，且会与先前 pending 的 buffer 竞态（configure 到达后会把旧的贴出来）。
            // 统一丢弃旧的 pending，用本次最新渲染结果替换，等 configure 到达后再贴。
            if let Some((stale, _, _)) = self.cand_pending.take() {
                stale.destroy();
            }
            layer.set_size(width.max(0) as u32, height.max(0) as u32);
            self.cand_w = width;
            self.cand_h = height;
            self.cand_pending = Some((buffer, width, height));
            let _ = surface.commit();
            self.cand_visible = true;
            return;
        }

        if width != self.cand_w || height != self.cand_h {
            // layer-shell 要求先 ack 新的 configure 再贴对应尺寸的 buffer，
            // 因此尺寸变化时先提交空 buffer 触发 configure(实为 set_size 请求)，
            // 待 configure 到达后再贴。
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

    /// 隐藏候选框：拆掉 layer surface。再次显示时会重建全新表面，
    /// 按 OVERLAY 层级重新置顶，避免“贴回旧表面被忽略”的问题。
    fn hide_candidates(&mut self) {
        if self.cand_surface.is_none() && self.cand_layer.is_none() {
            if let Some((pending, _, _)) = self.cand_pending.take() {
                pending.destroy();
            }
            return;
        }
        if let Some((pending, _, _)) = self.cand_pending.take() {
            pending.destroy();
        }
        if let Some(layer) = self.cand_layer.take() {
            layer.destroy();
        }
        if let Some(surface) = self.cand_surface.take() {
            surface.destroy();
        }
        self.cand_configured = false;
        self.cand_visible = false;
        self.cand_w = 0;
        self.cand_h = 0;
    }

    /// 重建候选框 layer surface（OVERLAY，压住所有普通窗口）。重新走到
    /// “set_size → configure → ack → 贴图”即首屏置顶流程。
    fn ensure_candidate_surface(&mut self, cfg: &Config, qh: &QueueHandle<Self>) {
        let (comp, shell) = match (self.cand_compositor.as_ref(), self._cand_layer_shell.as_ref()) {
            (Some(c), Some(s)) => (c.clone(), s.clone()),
            _ => return,
        };
        let surface = comp.create_surface(qh, WlUser);
        let layer = shell.get_layer_surface(
            &surface,
            None,
            Layer::Overlay,
            "qianyan-candidates".to_string(),
            qh,
            WlUser,
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);
        layer.set_size(1, 1);

        let (anchor, mt, mr, mb, ml) = match cfg.linux.corner.as_str() {
            "top-right" => (Anchor::Top | Anchor::Right, cfg.linux.fixed_y.max(0), cfg.linux.fixed_x.max(0), 0, 0),
            "top-left" => (Anchor::Top | Anchor::Left, cfg.linux.fixed_y.max(0), 0, 0, cfg.linux.fixed_x.max(0)),
            "bottom-left" => (Anchor::Bottom | Anchor::Left, 0, 0, cfg.linux.fixed_y.max(0), cfg.linux.fixed_x.max(0)),
            _ => (Anchor::Bottom | Anchor::Right, 0, cfg.linux.fixed_x.max(0), cfg.linux.fixed_y.max(0), 0),
        };
        layer.set_anchor(anchor);
        layer.set_margin(mt, mr, mb, ml);
        let _ = surface.commit();

        self.cand_surface = Some(surface);
        self.cand_layer = Some(layer);
        self.cand_configured = false;
        self.cand_visible = true;
        info!("[WaylandIM v1] candidate overlay (re)created on top");
        diag_file("[V1] candidate overlay (re)created on top");
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

    fn set_stage(&self, stage: &'static str) {
        if let Ok(mut s) = self.diag_stage.lock() {
            *s = stage;
        }
    }

    fn set_last_event(&mut self, event: &'static str) {
        self.last_event = event;
    }

    fn arm_key_repeat(&mut self, key: u32, serial: u32, time: u32) {
        if self.repeat_rate == 0 {
            return;
        }
        let repeatable = match self.resolve_key(key) {
            Some((vk, _)) => {
                !vk.is_modifier()
                    && vk != VirtualKey::CapsLock
            }
            None => false,
        };
        if !repeatable {
            self.held_keys.remove(&key);
            return;
        }
        self.held_keys.insert(
            key,
            HeldKey {
                serial,
                time,
                next_repeat: Some(Instant::now() + Duration::from_millis(self.repeat_delay_ms)),
            },
        );
    }

    fn next_repeat_deadline(&self, now: Instant) -> Option<Instant> {
        self.held_keys
            .values()
            .filter_map(|h| h.next_repeat)
            .filter(|d| *d > now)
            .min()
    }

    fn synthesize_repeats(&mut self, conn: &Connection, qh: &QueueHandle<Self>, now: Instant) {
        if self.repeat_rate == 0 {
            self.held_keys.clear();
            return;
        }
        let interval = Duration::from_millis((1000u64 / (self.repeat_rate as u64).max(1)).max(1));
        let due: Vec<(u32, u32, u32)> = self
            .held_keys
            .iter_mut()
            .filter(|(_, h)| h.next_repeat.is_some_and(|d| d <= now))
            .map(|(k, h)| {
                h.next_repeat = Some(now + interval);
                (*k, h.serial, h.time)
            })
            .collect();
        for (key, serial, time) in due {
            // 合成期间该键可能已被释放，跳过
            if !self.held_keys.contains_key(&key) {
                continue;
            }
            self.set_stage("synthesize_repeat");
            self.process_key_press(conn, qh, key, serial, time);
        }
        self.set_stage("idle");
    }

    /// 统一的按键按下处理路径：真实 press 与合成 repeat 都走这里。
    fn process_key_press(&mut self, conn: &Connection, qh: &QueueHandle<Self>, key: u32, serial: u32, time: u32) {
        if !self.active {
            self.held_keys.remove(&key);
            return;
        }
        self.set_stage("resolve_key");
        let (vk, utf8_text) = match self.resolve_key(key) {
            Some(pair) => pair,
            None => {
                self.set_stage("forward_unmapped");
                self.forwarded_keys.insert(key);
                if let Some(ctx) = &self.context {
                    ctx.key(serial, time, key, KeyState::Pressed.into());
                    let _ = conn.flush();
                }
                self.set_stage("idle");
                return;
            }
        };

        let (shift, ctrl, alt) = self.modifier_flags();
        self.set_stage("engine.handle_key");
        let action = match self.processor.handle_key(vk, 1, shift, ctrl, alt, true) {
            Some(a) => a,
            None => {
                self.set_stage("idle");
                return;
            }
        };

        self.set_stage("engine.get_basic_status");
        if let Some(status) = self.processor.get_basic_status() {
            let _ = self.tray_tx.send(TrayEvent::SyncStatus {
                chinese_enabled: status.chinese_enabled,
                active_profile: status.active_profile.clone(),
            });
        }

        // 先执行动作（可能已上屏并清空缓冲区），再取快照，
        // 否则 commit 后会拿到上屏前的旧候选，导致候选框不消失。
        self.set_stage("handle_action");
        self.handle_action(&action, conn, utf8_text, serial, time, key, KeyState::Pressed.into());

        self.set_stage("engine.get_gui_snapshot");
        let (update, preedit, cand_content) = if let Some(gui) = self.processor.get_gui_snapshot() {
            if gui.pinyin.is_empty() || !gui.chinese_enabled {
                (
                    GuiEvent::Update {
                        pinyin: "".into(),
                        candidates: vec![],
                        selected: 0,
                        page: 0,
                        total_pages: 0,
                        sentence: "".into(),
                        cursor_pos: 0,
                        commit_mode: gui.commit_mode.clone(),
                    },
                    String::new(),
                    CandContent::default(),
                )
            } else {
                let candidates: Vec<qianyan_ime_ui::DisplayCandidate> = gui
                    .candidates
                    .iter()
                    .map(|c| {
                        let full_display = if c.hint.is_empty() {
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
                    })
                    .collect();
                let cand_items: Vec<CandItem> = gui.candidates.iter().map(|c| CandItem {
                    label: c.label.clone(),
                    text: c.text.clone(),
                    hint: c.hint.clone(),
                    is_fuzzy: c.is_fuzzy,
                }).collect();
                let content = CandContent {
                    pinyin: gui.pinyin.clone(),
                    candidates: cand_items,
                    selected: gui.selected,
                };
                (
                    GuiEvent::Update {
                        pinyin: gui.pinyin.clone(),
                        candidates,
                        selected: gui.selected,
                        page: gui.page,
                        total_pages: gui.total_pages,
                        sentence: gui.sentence.clone(),
                        cursor_pos: gui.cursor_pos,
                        commit_mode: gui.commit_mode,
                    },
                    gui.pinyin,
                    content,
                )
            }
        } else {
            (
                GuiEvent::Update {
                    pinyin: "".into(),
                    candidates: vec![],
                    selected: 0,
                    page: 0,
                    total_pages: 0,
                    sentence: "".into(),
                    cursor_pos: 0,
                    commit_mode: String::new(),
                },
                String::new(),
                CandContent::default(),
            )
        };

        self.set_stage("refresh_candidates");
        self.refresh_candidates(cand_content, qh);
        let _ = self.gui_tx.send(update);

        self.set_stage("send_preedit");
        self.send_preedit(&preedit);
        let _ = conn.flush();
        self.set_stage("idle");
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
                if is_chinese_commit(text) {
                    let _ = self.gui_tx.send(GuiEvent::Committed(text.clone()));
                }
            }
            Action::DeleteAndEmit { delete, insert } => {
                ctx.delete_surrounding_text(-(i32::try_from(*delete).unwrap_or(0)), 0);
                ctx.commit_string(self.context_serial, insert.clone());
                if is_chinese_commit(insert) {
                    let _ = self.gui_tx.send(GuiEvent::Committed(insert.clone()));
                }
            }
            Action::PassThrough => {
                if utf8_text.is_empty()
                    || utf8_text.chars().next().is_some_and(|c| c.is_control())
                {
                    ctx.key(key_serial, time, key, state);
                    // 记录透传的 press，供后续 release 配对转发
                    self.forwarded_keys.insert(key);
                } else {
                    ctx.commit_string(self.context_serial, utf8_text.clone());
                    if is_chinese_commit(&utf8_text) {
                        let _ = self.gui_tx.send(GuiEvent::Committed(utf8_text.clone()));
                    }
                }
            }
            Action::Alert => {
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

    fn send_preedit(&self, text: &str) {
        // 在目标文本框内联显示拼音预览（preedit）。部分应用对 preedit 支持不佳，
        // 因此提供 linux.show_preedit 开关，关闭时始终发送空串以清空。
        let ctx = match self.context.as_ref() {
            Some(c) => c,
            None => return,
        };
        let enabled = match self.config.read() {
            Ok(c) => c.linux.show_preedit,
            Err(e) => e.into_inner().linux.show_preedit,
        };
        let preedit = if enabled { text.to_string() } else { String::new() };
        ctx.preedit_string(self.context_serial, preedit, String::new());
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
                state.set_last_event("IME-Activate");
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
                state.set_last_event("IME-Deactivate");
                info!("[WaylandIM v1] deactivated (debouncing {}ms)", DEACTIVATE_DEBOUNCE.as_millis());
                state.active = false;
                if let Some(ref ctx) = state.context {
                    ctx.preedit_string(state.context_serial, String::new(), String::new());
                    let _ = conn.flush();
                }
                state.forwarded_keys.clear();
                state.held_keys.clear();
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
                state.set_last_event("Ctx-SurroundingText");
                info!("[WaylandIM v1] surrounding_text: cursor={} anchor={}", cursor, anchor);
            }
            Event::ContentType { hint, purpose } => {
                state.set_last_event("Ctx-ContentType");
                info!("[WaylandIM v1] content_type: hint={:?} purpose={:?}", hint, purpose);
            }
            Event::CommitState { serial } => {
                state.set_last_event("Ctx-CommitState");
                state.context_serial = serial;
            }
            Event::Reset => {
                state.set_last_event("Ctx-Reset");
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
                state.set_last_event("Kbd-Keymap");
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
                state.set_last_event("Kbd-Key");
                // 释放必须先于 active 判断处理：某些应用会触发 activate/deactivate
                // 抖动，若释放恰好落在 180ms 防抖窗口内（active=false）被直接丢弃，
                // held_keys 里就会残留“假按住”，导致这个键被无限合成 repeat——
                // 即“只按了一次 c 却一直 cccc”。
                if key_state == WEnum::Value(KeyState::Released) {
                    state.held_keys.remove(&key);
                    // 通知引擎按键释放，更新内部按键状态（如 tab_down、CapsLock 等）。
                    // 之前 release 完全不下发，会让 tab_down 在按过一次 Tab 后永久为真，
                    // 导致数字 1/2 被当成“以词定字”(word_to_char_shift) 而选字而非选词。
                    if state.active {
                        if let Some((vk, _)) = state.resolve_key(key) {
                            let (shift, ctrl, alt) = state.modifier_flags();
                            let _ = state.processor.handle_key(vk, 0, shift, ctrl, alt, false);
                        }
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
                if !state.active {
                    return;
                }
                if key_state != WEnum::Value(KeyState::Pressed) {
                    return;
                }

                // 若该键已处于 held（上一次 release 丢失导致的“假按住”），先清除
                // 旧条目，阻断旧 repeat 续命；再重新登记并走统一按键处理。
                state.held_keys.remove(&key);
                // 这次新的物理按键说明此前所有按键的按压周期都已结束，
                // 顺手清掉其它残留的假按住，防止任一键跑成无限连发。
                state.held_keys.clear();

                // KWin(input-method v1)不会替输入法发送重复按键。
                // 先登记“按住中”，再走统一按键处理；合成 repeat 时也会走这条路径。
                state.arm_key_repeat(key, serial, time);
                state.process_key_press(conn, qh, key, serial, time);
            }
            wl_keyboard::Event::Modifiers {
                serial,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                state.set_last_event("Kbd-Modifiers");
                state.mods_depressed = mods_depressed;
                if let Some(ref mut xkb_st) = state.xkb_state {
                    xkb_st.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
                state.forward_modifiers(serial, mods_depressed, mods_latched, mods_locked, group);
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.set_last_event("Kbd-RepeatInfo");
                state.repeat_rate = rate.max(0) as u32;
                state.repeat_delay_ms = if rate <= 0 { 0 } else { delay.max(1) as u64 };
            }
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
        diag_file("[V1] connected to KWin private socket, input_method_v1 bound");

        let last_tick = Arc::new(Mutex::new(Instant::now()));
        let diag_stage = Arc::new(Mutex::new("startup"));

        // 自绘候选框：延迟到首次有候选时再创建(ensure_candidate_surface)，
        // 隐藏时拆掉、再显示时重建全新 OVERLAY 表面，保证每次都压得住普通窗口。
        let cand_layer_shell: Option<ZwlrLayerShellV1> = globals.bind(&qh, 1..=4, WlUser).ok();
        let cand_shm: Option<WlShm> = globals.bind(&qh, 1..=1, WlUser).ok();
        let cand_compositor: Option<WlCompositor> = globals.bind(&qh, 1..=6, WlUser).ok();
        let cand_surface: Option<WlSurface> = None;
        let cand_layer: Option<ZwlrLayerSurfaceV1> = None;
        diag_file(&format!(
            "[V1] globals: layer_shell={} compositor={} shm={}",
            cand_layer_shell.is_some(),
            cand_compositor.is_some(),
            cand_shm.is_some()
        ));

        let cand_layer_shell_present = cand_layer_shell.is_some();
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
            cand_compositor,
            cand_surface,
            cand_layer,
            cand_shm,
            cand_configured: false,
            cand_visible: false,
            cand_w: 0,
            cand_h: 0,
            cand_pending: None,
            cand_live_buffers: Vec::new(),
            // KWin 不给输入法下发 RepeatInfo 时 fallback 到通用参数
            repeat_rate: 25,
            repeat_delay_ms: 500,
            held_keys: HashMap::new(),
            diag_stage: diag_stage.clone(),
            last_event: "startup",
        };

        diag_file(&format!(
            "[V1] startup ok: repeat={}/{}s layer_shell={}",
            state.repeat_rate,
            state.repeat_delay_ms,
            cand_layer_shell_present
        ));

        // 看门狗：Wayland 事件循环必须每 ~8s 至少醒一次，
        // 一旦超过就认为“键盘卡死”，把停滞时长、现场阶段与最近事件写入诊断日志。
        {
            let w_last = last_tick.clone();
            let w_stage = diag_stage.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(5));
                let stale = Instant::now().duration_since(*w_last.lock().unwrap());
                if stale >= Duration::from_secs(8) {
                    let stage = *w_stage.lock().unwrap();
                    diag_file(&format!(
                        "[WATCHDOG] Wayland 主机循环已停滞 {:.1}s，阶段: {stage}",
                        stale.as_secs_f64()
                    ));
                }
            });
        }

        // 事件循环期间我们的写操作（flush）绝不能阻塞：
        // 若 KWin 端输入法协议卡住不再读 socket，阻塞式 send 会让键盘线程
        // 永久卡死（此前正是“打字后鼠标动一下光标 → 打不出字”的元凶候选）。
        // 这里把连接 fd 设为 O_NONBLOCK，写不出去就记录，而不是挂死。
        {
            use std::os::fd::AsRawFd;
            let fd = conn.as_fd().as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags >= 0 {
                unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK); }
            }
            diag_file(&format!("[V1] conn fd O_NONBLOCK: flags={flags}"));
        }

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

            // 事件分发在共享键盘线程里执行，任意处理器 panic 都会
            // 直接退出该线程导致“键盘卡死”。这里 catch_unwind 兜住 panic：记录到日志文件
            // 后继续事件循环，而不是让整个 Wayland 端退出。
            // 另外用 poll 替换 blocking_dispatch：带超时等待，循环不会被 compositor 一直没事件
            // 而卡死，同时留出时间片做 KWin 不做合成、需要自己做的 key repeat。
            let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let now = Instant::now();
                *last_tick.lock().unwrap() = now;

                // 等待超时 = min(1s 兜底, deactivate 截止, 下一次 repeat 合成时间点)
                let mut timeout_ms: i64 = 1_000;
                if let Some(deadline) = state.deactivate_deadline {
                    if deadline > now {
                        timeout_ms = timeout_ms.min(deadline.duration_since(now).as_millis() as i64);
                    }
                }
                if let Some(next_repeat) = state.next_repeat_deadline(now) {
                    if next_repeat > now {
                        timeout_ms = timeout_ms.min(next_repeat.duration_since(now).as_millis() as i64);
                    }
                }

                state.set_stage("loop.flush");
                if let Err(e) = conn.flush() {
                    diag_file(&format!("[V1] flush error: {e} (last_event={})", state.last_event));
                }
                state.set_stage("loop.poll");
                let readable = poll_readable(conn, timeout_ms.max(1).min(i32::MAX as i64) as i32);
                state.set_stage("loop.read");
                if readable {
                    if let Some(guard) = conn.prepare_read() {
                        if let Err(e) = guard.read() {
                            return Err(format!("read error: {e}"));
                        }
                    }
                }
                state.set_stage("loop.dispatch");
                if let Err(e) = event_queue.dispatch_pending(&mut state) {
                    return Err(format!("dispatch error: {e}"));
                }
                if self.running.load(Ordering::SeqCst) {
                    state.set_stage("loop.synth");
                    state.synthesize_repeats(conn, &qh, Instant::now());
                }
                state.set_stage("idle");
                Ok(())
            }));

            match stepped {
                Ok(Err(msg)) => {
                    error!("[WaylandIM v1] {msg}");
                    break;
                }
                Err(payload) => {
                    let bt = std::backtrace::Backtrace::force_capture();
                    let dump = format!(
                        "[WaylandIM v1] panic in event dispatch: {payload:?}\n{bt}"
                    );
                    error!("{dump}");
                    log_panic_file(&dump);
                    diag_file(&format!("[V1 PANIC] {dump}"));
                }
                Ok(Ok(())) => {}
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

fn poll_readable(conn: &Connection, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd: conn.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    r > 0 && (pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0
}

/// 上屏文本里含非 ASCII（中文）才算中文上屏，避免每个英文字母都弹通知。
fn is_chinese_commit(s: &str) -> bool {
    s.chars().any(|c| c > '\u{7f}')
}

/// KWin 从桌面启动本进程时不会保留终端 stderr，panic 信息可能直接丢失。
/// 把 panic 现场落盘到配置目录，便于复现排查随机卡死。
fn log_panic_file(dump: &str) {
    use std::io::Write;
    let path = qianyan_ime_core::Config::get_config_dir().join("wayland_v1_panic.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut stamp = String::new();
    use std::fmt::Write as _;
    let _ = write!(stamp, "{:?}", std::time::SystemTime::now());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "\n===== {} =====\n{}", stamp, dump);
    }
}
