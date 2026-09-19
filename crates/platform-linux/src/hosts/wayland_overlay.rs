//! 非 KDE Wayland 桌面（evdev 拦截 / IBus）的候选框自绘后端。
//!
//! evdev 宿主不注册 input-method 协议，没有合成器给的 popup 表面；这里在独立
//! 线程上用一条普通 Wayland 连接创建 Overlay layer-shell 表面，复用
//! `qianyan_ime_ui::offscreen`（Slint 离屏渲染）把候选内容画到 wl_shm。
//! 视觉定义与 v1/v2 宿主、X11/Windows 的 Slint 窗口完全一致（candidate.slint）。

use std::error::Error;
use std::ffi::CString;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use log::{error, info};
use qianyan_ime_core::Config;

use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use qianyan_ime_ui::GuiEvent;

use super::wayland_candidates::{render_slint, CandContent, CandItem};

/// layer-shell 表面事件循环轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(8);

/// Wayland 对象统一使用的 user data（该连接只服务候选框自身）。
struct OverlayUser;

struct OverlayState {
    config: Arc<RwLock<Config>>,
    qh: QueueHandle<OverlayState>,
    _compositor: WlCompositor,
    _layer_shell: ZwlrLayerShellV1,
    shm: WlShm,
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    configured: bool,
    surface_visible: bool,
    /// 尺寸变化时等待 configure 的待贴 buffer。
    pending: Option<(WlBuffer, i32, i32)>,
    /// 已 attach 且尚未收到 release 的 buffer。
    live: Vec<WlBuffer>,
    content: CandContent,
    dirty: bool,
    w: i32,
    h: i32,
}

impl OverlayState {
    fn candidates_enabled(&self) -> bool {
        let cfg = match self.config.read() {
            Ok(c) => c,
            Err(e) => e.into_inner(),
        };
        cfg.appearance.show_candidates && cfg.linux.show_slint_window
    }

    fn refresh(&mut self) {
        let cfg = match self.config.read() {
            Ok(c) => c.clone(),
            Err(e) => e.into_inner().clone(),
        };
        let visible = !self.content.pinyin.is_empty() && self.candidates_enabled();

        if !visible {
            if let Some((pending, _, _)) = self.pending.take() {
                pending.destroy();
            }
            if self.surface_visible {
                self.surface.attach(None, 0, 0);
                let _ = self.surface.commit();
                self.surface_visible = false;
            }
            self.dirty = false;
            return;
        }

        // 尚未收到首次 configure，或有一块等待 configure 的 buffer 在途：
        // 保留 dirty，等 configure 后再渲染最新内容。
        if !self.configured || self.pending.is_some() {
            return;
        }

        let (pixels, width, height) = match render_slint(&cfg, &self.content) {
            Some(v) => v,
            None => return,
        };
        let buffer = match upload_shm(&self.shm, &self.qh.clone(), &pixels, width, height) {
            Some(b) => b,
            None => return,
        };

        let mx = cfg.linux.fixed_x.max(0);
        let my = cfg.linux.fixed_y.max(0);
        let (anchor, mt, mr, mb, ml) = match cfg.linux.corner.as_str() {
            "top-right" => (Anchor::Top | Anchor::Right, my, mx, 0, 0),
            "top-left" => (Anchor::Top | Anchor::Left, my, 0, 0, mx),
            "bottom-left" => (Anchor::Bottom | Anchor::Left, 0, 0, my, mx),
            _ => (Anchor::Bottom | Anchor::Right, 0, mx, my, 0),
        };
        self.layer.set_anchor(anchor);
        self.layer.set_margin(mt, mr, mb, ml);

        if width != self.w || height != self.h {
            // layer-shell 要求先 ack 新的 configure 再贴对应尺寸的 buffer：
            // 尺寸变化时先提交空 buffer 触发 configure，待 configure 到达后再贴。
            if let Some((stale, _, _)) = self.pending.take() {
                stale.destroy();
            }
            self.w = width;
            self.h = height;
            self.layer.set_size(width.max(0) as u32, height.max(0) as u32);
            self.pending = Some((buffer, width, height));
            let _ = self.surface.commit();
            return;
        }

        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage(0, 0, width, height);
        let _ = self.surface.commit();
        self.surface_visible = true;
        self.live.push(buffer);
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for OverlayState {
    fn event(
        _state: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, OverlayUser> for OverlayState {
    fn event(
        _state: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, OverlayUser> for OverlayState {
    fn event(
        _state: &mut Self,
        _: &WlShm,
        _: <WlShm as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShmPool, OverlayUser> for OverlayState {
    fn event(
        _state: &mut Self,
        _: &WlShmPool,
        _: <WlShmPool as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, OverlayUser> for OverlayState {
    fn event(
        _state: &mut Self,
        _: &WlSurface,
        _: <WlSurface as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, OverlayUser> for OverlayState {
    fn event(
        state: &mut Self,
        proxy: &WlBuffer,
        event: <WlBuffer as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            if let Some(pos) = state.live.iter().position(|b| b.id() == proxy.id()) {
                let buf = state.live.swap_remove(pos);
                buf.destroy();
            }
        }
    }
}

impl Dispatch<ZwlrLayerShellV1, OverlayUser> for OverlayState {
    fn event(
        _state: &mut Self,
        _: &ZwlrLayerShellV1,
        _: <ZwlrLayerShellV1 as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, OverlayUser> for OverlayState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: <ZwlrLayerSurfaceV1 as Proxy>::Event,
        _: &OverlayUser,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event;
        match event {
            Event::Configure { serial, width, height } => {
                proxy.ack_configure(serial);
                info!(
                    "[Overlay] candidate surface configured {}x{}",
                    width, height
                );
                state.configured = true;
                if let Some((buffer, w, h)) = state.pending.take() {
                    state.surface.attach(Some(&buffer), 0, 0);
                    state.surface.damage(0, 0, w, h);
                    let _ = state.surface.commit();
                    state.surface_visible = true;
                    state.live.push(buffer);
                }
            }
            Event::Closed => {
                info!("[Overlay] candidate surface closed");
                if let Some((pending, _, _)) = state.pending.take() {
                    pending.destroy();
                }
                state.surface_visible = false;
            }
            _ => {}
        }
    }
}

fn upload_shm(
    shm: &WlShm,
    qh: &QueueHandle<OverlayState>,
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

    let pool = shm.create_pool(owned.as_fd(), size as i32, qh, OverlayUser);
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, OverlayUser);
    pool.destroy();
    Some(buffer)
}

fn apply_event(state: &mut OverlayState, ev: GuiEvent) {
    match ev {
        GuiEvent::Update { pinyin, candidates, selected, .. } => {
            state.content = CandContent {
                pinyin,
                candidates: candidates
                    .into_iter()
                    .map(|c| CandItem {
                        label: c.label,
                        text: c.text,
                        hint: c.hint,
                        is_fuzzy: c.is_fuzzy,
                    })
                    .collect(),
                selected,
            };
            state.dirty = true;
        }
        GuiEvent::SyncState(app) => {
            state.content = CandContent {
                pinyin: app.pinyin,
                candidates: app
                    .candidates
                    .into_iter()
                    .map(|c| CandItem {
                        label: c.label,
                        text: c.text,
                        hint: c.hint,
                        is_fuzzy: c.is_fuzzy,
                    })
                    .collect(),
                selected: app.selected_index,
            };
            state.dirty = true;
        }
        GuiEvent::SetVisible(false) => {
            state.content = CandContent::default();
            state.dirty = true;
        }
        GuiEvent::HideAndAck(ack) => {
            // 与 GUI 子进程一致：立即隐藏并回 ack，避免 uinput 注入被候选窗拦截。
            state.content = CandContent::default();
            state.dirty = true;
            let _ = ack.send(());
        }
        _ => {}
    }
}

fn conn_fd_readable(conn: &Connection, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd: conn.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    r > 0 && (pfd.revents & libc::POLLIN) != 0
}

fn run_loop(
    conn: Connection,
    mut queue: EventQueue<OverlayState>,
    mut state: OverlayState,
    rx: Receiver<GuiEvent>,
    running: &Arc<AtomicBool>,
) -> Result<(), Box<dyn Error>> {
    while running.load(Ordering::SeqCst) {
        let mut active = false;
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    apply_event(&mut state, ev);
                    active = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    running.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }

        if state.dirty && state.configured {
            state.refresh();
            // 若尺寸变化后有一块等待 configure 的 buffer 在途，保留 dirty，
            // 待 configure 贴出后再以最新内容补渲染一次；期间允许睡眠轮询。
            state.dirty = state.pending.is_some();
        }

        let _ = conn.flush();
        queue.dispatch_pending(&mut state).map_err(|e| -> Box<dyn Error> { e.into() })?;

        if conn_fd_readable(&conn, 0) {
            if let Some(guard) = conn.prepare_read() {
                let _ = guard.read();
            }
            queue.dispatch_pending(&mut state).map_err(|e| -> Box<dyn Error> { e.into() })?;
        }

        if !active {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
    Ok(())
}

/// 主进程内的 Wayland 候选框自绘后端。
pub struct WaylandOverlay {
    tx: Sender<GuiEvent>,
    running: Arc<AtomicBool>,
}

impl WaylandOverlay {
    /// 建立一条独立 Wayland 连接并启动候选框事件循环线程。
    /// 失败（无 Wayland / 无 layer-shell）时返回 Err，调用方回退 GUI 子进程。
    pub fn start(config: Arc<RwLock<Config>>) -> Result<Self, Box<dyn Error>> {
        let conn = Connection::connect_to_env()?;
        let (globals, queue): (GlobalList, EventQueue<OverlayState>) = registry_queue_init(&conn)?;
        let qh = queue.handle();

        let compositor: WlCompositor = globals.bind(&qh, 1..=6, OverlayUser)?;
        let shm: WlShm = globals.bind(&qh, 1..=1, OverlayUser)?;
        let layer_shell: ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, OverlayUser)?;

        let surface = compositor.create_surface(&qh, OverlayUser);
        let layer = layer_shell.get_layer_surface(
            &surface,
            None,
            Layer::Overlay,
            "qianyan-candidates".to_string(),
            &qh,
            OverlayUser,
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);
        layer.set_size(1, 1);
        layer.set_anchor(Anchor::Bottom | Anchor::Right);
        layer.set_margin(0, 10, 10, 0);
        let _ = surface.commit();
        let _ = conn.flush();

        let (tx, rx) = mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let state = OverlayState {
            config,
            qh,
            _compositor: compositor,
            _layer_shell: layer_shell,
            shm,
            surface,
            layer,
            configured: false,
            surface_visible: false,
            pending: None,
            live: Vec::new(),
            content: CandContent::default(),
            dirty: false,
            w: 0,
            h: 0,
        };
        let running_loop = running.clone();
        std::thread::Builder::new()
            .name("qianyan-overlay".to_string())
            .spawn(move || {
                if let Err(e) = run_loop(conn, queue, state, rx, &running_loop) {
                    error!("[Overlay] event loop error: {e}");
                }
            })?;
        info!("[Overlay] Wayland 候选框 layer-shell 表面已就绪");
        Ok(Self { tx, running })
    }

    pub fn sender(&self) -> Sender<GuiEvent> {
        self.tx.clone()
    }
}

impl Drop for WaylandOverlay {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}