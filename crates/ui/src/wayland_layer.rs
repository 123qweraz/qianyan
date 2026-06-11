use crate::CandidateDisplay;
use qianyan_ime_core::Config;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::{slot::SlotPool, Shm, ShmHandler};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::seat::keyboard::{KeyboardHandler, Modifiers, Keysym, KeyEvent};
use smithay_client_toolkit::seat::{SeatHandler, SeatState};
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_seat, delegate_shm,
    delegate_layer, delegate_keyboard,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, Dispatch, QueueHandle};
use std::os::fd::AsRawFd;

use i_slint_core::window::WindowAdapter;
use i_slint_renderer_skia::software_surface::{RenderBuffer, SoftwareSurface};
use i_slint_renderer_skia::{skia_safe, SkiaRenderer, SkiaSharedContext};

slint::include_modules!();

// ---- Offscreen window adapter ----
// Uses SkiaRenderer instead of SoftwareRenderer for higher-quality text
// and native BGRA pixel format (no swizzle needed for wl_shm).

thread_local! {
    static SKIA_RENDERER: RefCell<Option<Rc<SkiaRenderer>>> = const { RefCell::new(None) };
}

struct OffscreenWindow {
    window: slint::Window,
    renderer: Rc<SkiaRenderer>,
    size: std::cell::Cell<slint::PhysicalSize>,
    needs_redraw: std::cell::Cell<bool>,
}

impl OffscreenWindow {
    fn new() -> Rc<Self> {
        let renderer = SKIA_RENDERER.with(|s| s.borrow().as_ref()
            .expect("Skia renderer not initialized")
            .clone());
        Rc::new_cyclic(|w: &std::rc::Weak<Self>| Self {
            window: slint::Window::new(w.clone()),
            renderer,
            size: std::cell::Cell::new(slint::PhysicalSize::default()),
            needs_redraw: std::cell::Cell::new(false),
        })
    }
}

impl WindowAdapter for OffscreenWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn renderer(&self) -> &dyn slint::platform::Renderer {
        self.renderer.as_ref()
    }
    fn size(&self) -> slint::PhysicalSize {
        let s = self.size.get();
        log::debug!("[WL_DEBUG] OffscreenWindow::size() = {}x{}", s.width, s.height);
        s
    }
    fn set_size(&self, size: slint::WindowSize) {
        let sf = self.window.scale_factor();
        let phys = size.to_physical(sf);
        log::debug!("[WL_DEBUG] OffscreenWindow::set_size({}x{})", phys.width, phys.height);
        self.size.set(phys);
        let logical_size = size.to_logical(sf);
        self.window
            .dispatch_event(slint::platform::WindowEvent::Resized { size: logical_size });
    }
    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }
}

type EventCallback = Box<dyn FnOnce() + Send>;

struct SlintPlatform {
    running: Arc<AtomicBool>,
    cmd_tx: mpsc::Sender<EventCallback>,
    cmd_rx: mpsc::Receiver<EventCallback>,
}

impl slint::platform::Platform for SlintPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(OffscreenWindow::new())
    }
    fn run_event_loop(&self) -> Result<(), slint::PlatformError> {
        while self.running.load(Ordering::SeqCst) {
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                cmd();
            }
            slint::platform::update_timers_and_animations();
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        Ok(())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn slint::platform::EventLoopProxy>> {
        Some(Box::new(SlintProxy {
            running: self.running.clone(),
            cmd_tx: self.cmd_tx.clone(),
        }))
    }
}

struct SlintProxy {
    running: Arc<AtomicBool>,
    cmd_tx: mpsc::Sender<EventCallback>,
}

impl slint::platform::EventLoopProxy for SlintProxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.cmd_tx.send(event).map_err(|_| slint::EventLoopError::EventLoopTerminated)
    }
}

fn setup_slint_platform() -> Option<()> {
    use std::sync::OnceLock;
    static RESULT: OnceLock<Option<()>> = OnceLock::new();
    RESULT.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<EventCallback>();
        let platform = Box::new(SlintPlatform {
            running: Arc::new(AtomicBool::new(true)),
            cmd_tx: tx,
            cmd_rx: rx,
        });
        slint::platform::set_platform(platform).ok()
    });
    RESULT.get().copied().flatten()
}


// ---- Wayland thread ----

#[derive(Clone)]
struct PixelPool(Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

impl PixelPool {
    fn new() -> Self {
        Self(Arc::new(std::sync::Mutex::new(Vec::with_capacity(4))))
    }
    fn get(&self, size: usize) -> Vec<u8> {
        let mut pool = self.0.lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(mut v) = pool.pop() {
            if v.capacity() < size {
                v.reserve(size - v.capacity());
            }
            v.resize(size, 0);
            v
        } else {
            vec![0u8; size]
        }
    }
    fn put(&self, v: Vec<u8>) {
        let mut pool = self.0.lock()
            .unwrap_or_else(|e| e.into_inner());
        if pool.len() < 8 {
            pool.push(v);
        }
    }
}

// ---- Skia CPU offscreen render buffer ----

struct WaylandRenderBuffer {
    pixel_pool: PixelPool,
    wl_tx: Mutex<Option<mpsc::SyncSender<WlCmd>>>,
    last_x: Cell<i32>,
    last_y: Cell<i32>,
    window_visible: Cell<bool>,
    fixed_position: Cell<bool>,
    corner: RefCell<String>,
    fixed_x: Cell<i32>,
    fixed_y: Cell<i32>,
    screen_w: Arc<AtomicI32>,
    screen_h: Arc<AtomicI32>,
}

impl WaylandRenderBuffer {
    fn screen_size(&self) -> (i32, i32) {
        let w = self.screen_w.load(Ordering::Relaxed);
        let h = self.screen_h.load(Ordering::Relaxed);
        if w > 0 && h > 0 {
            return (w, h);
        }
        if let Ok(out) = std::process::Command::new("xdotool")
            .arg("getdisplaygeometry").output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.len() == 2 {
                    if let (Ok(w2), Ok(h2)) = (parts[0].parse(), parts[1].parse()) {
                        log::info!("[WL_POS] screen_size via xdotool: {}x{}", w2, h2);
                        return (w2, h2);
                    }
                }
            }
        }
        log::warn!("[WL_POS] screen_size fallback to 1920x1080");
        (1920, 1080)
    }

    fn compute_anchor(&self, w: u32, h: u32) -> (Anchor, i32, i32) {
        if self.fixed_position.get() {
            let corner = self.corner.borrow();
            let result = match corner.as_str() {
                "top-right" => (Anchor::TOP | Anchor::RIGHT, self.fixed_y.get(), self.fixed_x.get()),
                "bottom-left" => (Anchor::BOTTOM | Anchor::LEFT, self.fixed_y.get(), self.fixed_x.get()),
                "bottom-right" => (Anchor::BOTTOM | Anchor::RIGHT, self.fixed_y.get(), self.fixed_x.get()),
                _ => (Anchor::TOP | Anchor::LEFT, self.fixed_y.get(), self.fixed_x.get()),
            };
            log::info!("[WL_POS] compute_anchor(fixed, corner={}, fixed_x={}, fixed_y={}, w={}, h={}) = ({:?}, {} ,{})",
                corner, self.fixed_x.get(), self.fixed_y.get(), w, h, result.0, result.1, result.2);
            result
        } else {
            let (sw, sh) = self.screen_size();
            let result = calc_anchor(self.last_x.get(), self.last_y.get(), w, h, sw, sh);
            log::info!("[WL_POS] compute_anchor(cursor, last_x={}, last_y={}, w={}, h={}, sw={}, sh={}) = ({:?}, {}, {})",
                self.last_x.get(), self.last_y.get(), w, h, sw, sh, result.0, result.1, result.2);
            result
        }
    }
}

/// Calculate wayland layer anchor and margins based on cursor position.
fn calc_anchor(cx: i32, cy: i32, w: u32, h: u32, sw: i32, sh: i32) -> (Anchor, i32, i32) {
    let ow = 20i32;
    let use_bottom = cy + ow + h as i32 > sh;
    let use_right = cx + w as i32 > sw;
    (
        if use_bottom { Anchor::BOTTOM } else { Anchor::TOP }
        | if use_right { Anchor::RIGHT } else { Anchor::LEFT },
        if use_bottom { sh - cy } else { cy + ow },
        if use_right { sw - cx } else { cx },
    )
}

impl RenderBuffer for WaylandRenderBuffer {
    fn with_buffer(
        &self,
        _window: &slint::Window,
        size: slint::PhysicalSize,
        render_callback: &mut dyn FnMut(
            NonZeroU32, NonZeroU32,
            skia_safe::ColorType, u8,
            &mut [u8],
        ) -> Result<Option<i_slint_core::partial_renderer::DirtyRegion>, i_slint_core::platform::PlatformError>,
    ) -> Result<(), i_slint_core::platform::PlatformError> {
        let w = size.width;
        let h = size.height;
        let pixel_count = (w * h) as usize;
        let mut pixels = self.pixel_pool.get(pixel_count * 4);

        let Some(width) = NonZeroU32::new(w) else { return Ok(()); };
        let Some(height) = NonZeroU32::new(h) else { return Ok(()); };

        // Skia renders in BGRA8888 format — ready for wl_shm::Argb8888, no swizzle needed
        let _ = render_callback(width, height, skia_safe::ColorType::BGRA8888, 0, &mut pixels)?;

        if self.window_visible.get() {
            let (anchor, margin_a, margin_b) = self.compute_anchor(w, h);
            log::info!("[WL_POS] Sending ShowCandidate: x={} y={} w={} h={} anchor={:?} window_visible={}",
                margin_b, margin_a, w, h, anchor, self.window_visible.get());
            if let Some(ref tx) = *self.wl_tx.lock()
                .unwrap_or_else(|e| e.into_inner()) {
                let cmd = WlCmd::ShowCandidate {
                    x: margin_b, y: margin_a, w, h, anchor, pixels,
                };
                let _ = tx.send(cmd);
            }
            // pixels now owned by Wayland thread (returned to pool after use)
        } else {
            self.pixel_pool.put(pixels);
        }

        Ok(())
    }
}

struct WlState {
    registry_state: RegistryState,
    compositor_state: CompositorState,
    shm: Shm,
    layer_shell: Option<LayerShell>,
    _output_state: OutputState,
    _seat_state: SeatState,
    candidate_layer: Option<LayerSurface>,
    candidate_pool: Option<SlotPool>,
    pixel_pool: PixelPool,
    layer_closed: bool,
    configured_width: u32,
    configured_height: u32,
    last_anchor: Option<Anchor>,
    last_margin: Option<(i32, i32, i32, i32)>, // (top, right, bottom, left)
    screen_w: Arc<AtomicI32>,
    screen_h: Arc<AtomicI32>,
}

delegate_registry!(WlState);
delegate_compositor!(WlState);
delegate_output!(WlState);
delegate_shm!(WlState);
delegate_seat!(WlState);
delegate_keyboard!(WlState);
delegate_layer!(WlState);

impl WlState {
    fn ensure_layer(&mut self, qh: &QueueHandle<Self>) -> Option<LayerSurface> {
        if let Some(layer) = &self.candidate_layer {
            return Some(layer.clone());
        }
        if !self.layer_closed {
            return None;
        }
        let ls = self.layer_shell.as_ref()?;
        let surf = self.compositor_state.create_surface(qh);
        let layer = ls.create_layer_surface(qh, surf, Layer::Overlay, Some("qianyan-ime-candidate"), None);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        log::info!("[WL_POS] Candidate layer surface recreated after compositor closed");
        Some(layer)
    }
}

/// Placeholder ObjectData for child surfaces created by the compositor
struct ChildSurfaceData;
impl wayland_client::backend::ObjectData for ChildSurfaceData {
    fn event(
        self: Arc<Self>,
        _handle: &wayland_client::backend::Backend,
        _msg: wayland_client::backend::protocol::Message<wayland_client::backend::ObjectId, std::os::unix::io::OwnedFd>,
    ) -> Option<Arc<dyn wayland_client::backend::ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_client::backend::ObjectId) {}
}

impl Dispatch<WlSurface, ()> for WlState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSurface,
        _event: <WlSurface as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
    fn event_created_child(
        _opcode: u16,
        _qh: &QueueHandle<Self>,
    ) -> Arc<dyn wayland_client::backend::ObjectData> {
        Arc::new(ChildSurfaceData)
    }
}

impl CompositorHandler for WlState {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wayland_client::protocol::wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
}

impl WlState {
    fn update_screen_size(&mut self) {
        for output in self._output_state.outputs() {
            if let Some(info) = self._output_state.info(&output) {
                for mode in &info.modes {
                    if mode.current {
                        let (w, h) = mode.dimensions;
                        self.screen_w.store(w, Ordering::Relaxed);
                        self.screen_h.store(h, Ordering::Relaxed);
                        log::info!("[WL_POS] wl_output screen size: {}x{}", w, h);
                        return;
                    }
                }
            }
        }
    }
}

impl OutputHandler for WlState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self._output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _output: wayland_client::protocol::wl_output::WlOutput) {
        self.update_screen_size();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _output: wayland_client::protocol::wl_output::WlOutput) {
        self.update_screen_size();
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _output: wayland_client::protocol::wl_output::WlOutput) {}
}

impl ShmHandler for WlState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl SeatHandler for WlState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self._seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: smithay_client_toolkit::seat::Capability) {}
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: smithay_client_toolkit::seat::Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for WlState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: &WlSurface, _: u32) {}
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: u32, _: KeyEvent) {}
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _raw_modifiers: smithay_client_toolkit::seat::keyboard::RawModifiers,
        _layout: u32,
    ) {
    }
    fn update_repeat_info(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlKeyboard, _: smithay_client_toolkit::seat::keyboard::RepeatInfo) {}
    fn update_keymap(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: smithay_client_toolkit::seat::keyboard::Keymap<'_>,
    ) {
    }
}

impl LayerShellHandler for WlState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _layer: &LayerSurface) {
        log::warn!("Layer surface closed by compositor, will re-create on next update");
        self.candidate_layer = None;
        self.layer_closed = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.configured_width = cfg.new_size.0;
        self.configured_height = cfg.new_size.1;
        let (a_dbg, m_dbg) = (self.last_anchor, self.last_margin);
        log::info!("[WL_POS] configure event: new_size=({},{}), last_anchor={:?}, last_margin={:?}",
            cfg.new_size.0, cfg.new_size.1, a_dbg, m_dbg);
        // Re-apply cached anchor/margin before commit, preventing
        // the compositor from locking in an anchor-less default position
        if let Some(a) = self.last_anchor {
            layer.set_anchor(a);
            if let Some((t, r, b, l)) = self.last_margin {
                layer.set_margin(t, r, b, l);
            }
        }
        layer.commit();
    }
}

impl ProvidesRegistryState for WlState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    fn runtime_add_global(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: u32,
        _: &str,
        _: u32,
    ) {
    }
    fn runtime_remove_global(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: u32,
        _: &str,
    ) {
    }
}

enum WlCmd {
    ShowCandidate { x: i32, y: i32, w: u32, h: u32, anchor: Anchor, pixels: Vec<u8> },
    HideCandidate,
    Exit,
}

fn wl_thread_main(rx: Receiver<WlCmd>, pixel_pool: PixelPool,
    initial_anchor: Anchor, initial_mt: i32, initial_mr: i32, initial_mb: i32, initial_ml: i32,
    screen_w: Arc<AtomicI32>, screen_h: Arc<AtomicI32>)
{
    log::info!("Wayland thread started");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wl_thread_main_inner(rx, pixel_pool, initial_anchor, initial_mt, initial_mr, initial_mb, initial_ml, screen_w, screen_h);
    }));
    if let Err(e) = result {
        let msg = if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else {
            "unknown panic".to_string()
        };
        log::error!("Wayland thread PANICKED: {msg}");
    }
    log::info!("Wayland thread exited");
}

fn wl_thread_main_inner(rx: Receiver<WlCmd>, pixel_pool: PixelPool,
    initial_anchor: Anchor, initial_mt: i32, initial_mr: i32, initial_mb: i32, initial_ml: i32,
    screen_w: Arc<AtomicI32>, screen_h: Arc<AtomicI32>)
{
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    log::info!("Wayland compositor: desktop={desktop}, session={session}");
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Wayland: cannot connect: {e}");
            return;
        }
    };

    let (globals, mut event_queue): (_, wayland_client::EventQueue<WlState>) = match registry_queue_init(&conn) {
        Ok(g) => g,
        Err(e) => {
            log::error!("Wayland: registry init failed: {e}");
            return;
        }
    };
    let qh: wayland_client::QueueHandle<WlState> = event_queue.handle();
    log::debug!("[WL_DEBUG] Wayland globals obtained");

    let compositor = match CompositorState::bind(&globals, &qh) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Wayland: no wl_compositor: {e}");
            return;
        }
    };
    let shm = match Shm::bind(&globals, &qh) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Wayland: no wl_shm: {e}");
            return;
        }
    };
    let ls = match LayerShell::bind(&globals, &qh) {
        Ok(ls) => ls,
        Err(e) => {
            log::error!("Wayland: no zwlr_layer_shell_v1: {e}");
            return;
        }
    };

    let mut state = WlState {
        registry_state: RegistryState::new(&globals),
        compositor_state: compositor,
        shm,
        layer_shell: Some(ls),
        _output_state: OutputState::new(&globals, &qh),
        _seat_state: SeatState::new(&globals, &qh),
        candidate_layer: None,
        candidate_pool: None,
        pixel_pool: pixel_pool.clone(),
        layer_closed: false,
        configured_width: 0,
        configured_height: 0,
        last_anchor: None,
        last_margin: None,
        screen_w: screen_w.clone(),
        screen_h: screen_h.clone(),
    };

    // Create candidate layer surface
    {
        let surf = state.compositor_state.create_surface(&qh);
        let layer = state
            .layer_shell
            .as_ref()
            .expect("layer_shell not initialized")
            .create_layer_surface(&qh, surf, Layer::Overlay, Some("qianyan-ime-candidate"), None);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_anchor(initial_anchor);
        layer.set_size(400, 200);
        layer.set_margin(initial_mt, initial_mr, initial_mb, initial_ml);
        state.last_anchor = Some(initial_anchor);
        state.last_margin = Some((initial_mt, initial_mr, initial_mb, initial_ml));
        log::info!("[WL_POS] Initial commit: anchor={:?} size=(400,200) margin=({},{},{},{})",
            initial_anchor, initial_mt, initial_mr, initial_mb, initial_ml);
        layer.commit();
        state.candidate_layer = Some(layer);
    }
    if let Ok(pool) = SlotPool::new(4 * 1024 * 1024, &state.shm) {
        state.candidate_pool = Some(pool);
        log::debug!("[WL_DEBUG] Candidate pool created (4MB)");
    }

    let _ = event_queue.dispatch_pending(&mut state);
    let _ = event_queue.flush();
    log::debug!("[WL_DEBUG] Wayland init done, entering main loop");

    loop {
        // Process all pending GUI commands
        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    WlCmd::ShowCandidate { x, y, w, h, anchor, pixels } => {
                        log::info!("[WL_POS] ShowCandidate handler: x={} y={} w={} h={} anchor={:?}", x, y, w, h, anchor);
                        if let Some(layer) = state.ensure_layer(&qh) {
                            layer.set_anchor(anchor);
                            layer.set_size(w.max(1), h.max(1));
                            let has_top = anchor.contains(Anchor::TOP);
                            let has_bottom = anchor.contains(Anchor::BOTTOM);
                            let has_left = anchor.contains(Anchor::LEFT);
                            let has_right = anchor.contains(Anchor::RIGHT);
                            let mt = if has_top { y.max(0) } else { 0 };
                            let mr = if has_right { x.max(0) } else { 0 };
                            let mb = if has_bottom { y.max(0) } else { 0 };
                            let ml = if has_left { x.max(0) } else { 0 };
                            layer.set_margin(mt, mr, mb, ml);
                            state.last_anchor = Some(anchor);
                            state.last_margin = Some((mt, mr, mb, ml));
                            log::info!("[WL_POS] ShowCandidate applied: anchor={:?} margin=({},{},{},{}) size=({},{})",
                                anchor, mt, mr, mb, ml, w, h);
                            if let Some(ref mut pool) = state.candidate_pool {
                                submit_to_layer(pool, &layer, &pixels, w.max(1), h.max(1));
                            }
                        } else {
                            log::info!("[WL_POS] candidate_layer is None!");
                        }
                        state.pixel_pool.put(pixels);
                    }
                    WlCmd::HideCandidate => {
                        let layer = state.candidate_layer.clone();
                        let pool = &mut state.candidate_pool;
                        if let (Some(layer), Some(ref mut pool)) = (layer, pool) {
                            if let Ok((buffer, canvas)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
                                canvas[0..4].copy_from_slice(&[0, 0, 0, 0]);
                                if buffer.attach_to(layer.wl_surface()).is_ok() {
                                    layer.wl_surface().damage_buffer(0, 0, 1, 1);
                                    layer.commit();
                                }
                            }
                        }
                    }
                    WlCmd::Exit => {
                        log::debug!("[WL_DEBUG] Wayland thread received Exit, terminating");
                        return;
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::debug!("[WL_DEBUG] Wayland thread channel disconnected, terminating");
                    return;
                }
            }
        }

        // Flush outgoing requests to compositor
        if event_queue.flush().is_err() {
            log::error!("Wayland flush failed, exiting thread");
            break;
        }

        // Dispatch any events already in the internal buffer
        if event_queue.dispatch_pending(&mut state).is_err() {
            log::error!("Wayland dispatch failed, exiting thread");
            break;
        }

        // Read new events from socket with 20ms timeout.
        // This is where wl_buffer.release events are received,
        // allowing SlotPool to recycle SHM memory.
        if let Some(read_guard) = event_queue.prepare_read() {
            let fd = read_guard.connection_fd().as_raw_fd();
            let mut fds = [libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            }];
            if unsafe { libc::poll(fds.as_mut_ptr(), 1, 20) } > 0 {
                if read_guard.read().is_err() {
                    log::error!("Wayland read failed, exiting thread");
                    break;
                }
                // Dispatch newly read events (includes wl_buffer.release)
                if event_queue.dispatch_pending(&mut state).is_err() {
                    log::error!("Wayland dispatch after read failed, exiting thread");
                    break;
                }
            }
        }
    }
    log::debug!("[WL_DEBUG] Wayland thread main loop exited");
}

fn submit_to_layer(
    pool: &mut SlotPool,
    layer: &LayerSurface,
    pixels_rgba: &[u8],   // input is RGBA, will be converted to BGRA on copy
    width: u32,
    height: u32,
) {
    let stride = (width * 4) as i32;
    let needed = (stride * height as i32) as usize;
    const MAX_POOL_SIZE: usize = 32 * 1024 * 1024;
    if needed > pool.len() {
        let new_size = needed.next_power_of_two().max(1024 * 1024);
        if new_size > MAX_POOL_SIZE {
            log::error!("SHM pool would exceed {}MB, halving render size", MAX_POOL_SIZE / 1024 / 1024);
            let hw = (width / 2).max(1);
            let hh = (height / 2).max(1);
            submit_to_layer(pool, layer, pixels_rgba, hw, hh);
            return;
        }
        log::info!("SHM pool resize: {} -> {} (needed={})", pool.len(), new_size, needed);
        if pool.resize(new_size).is_err() {
            log::error!("Failed to resize SHM pool, trying with smaller buffer");
            let hw = (width / 2).max(1);
            let hh = (height / 2).max(1);
            submit_to_layer(pool, layer, pixels_rgba, hw, hh);
            return;
        }
    }
    if let Ok((buffer, canvas)) = pool.create_buffer(width as i32, height as i32, stride, wl_shm::Format::Argb8888) {
        // Merge swizzle (RGBA→BGRA) into SHM copy — single pass
        swizzle_rgba_to_bgra(pixels_rgba, canvas);
        if buffer.attach_to(layer.wl_surface()).is_err() {
            log::error!("Failed to attach buffer to layer surface");
            return;
        }
        layer.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
        layer.commit();
    }
}

/// Copy RGBA pixels to BGRA canvas (wl_shm::Argb8888 is BGRA in memory).
fn swizzle_rgba_to_bgra(src: &[u8], dst: &mut [u8]) {
    let n = dst.len().min(src.len());
    for (s, d) in src[..n].chunks_exact(4).zip(dst[..n].chunks_exact_mut(4)) {
        d[0] = s[2]; d[1] = s[1]; d[2] = s[0]; d[3] = s[3];
    }
}

// ---- WaylandLayerDisplay ----

struct WlThread {
    cmd_tx: mpsc::SyncSender<WlCmd>,
}

pub struct WaylandLayerDisplay {
    skia_renderer: Rc<SkiaRenderer>,
    render_buffer: Rc<WaylandRenderBuffer>,
    candidate_window: CandidateWindow,
    config: Config,
    window_visible: bool,
    candidate_enabled: bool,
    last_x: i32,
    last_y: i32,
    wl: Option<WlThread>,
    wl_join: Option<std::thread::JoinHandle<()>>,
}

impl WaylandLayerDisplay {
    pub fn new(config: Config) -> Option<Self> {
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return None;
        }

        setup_slint_platform()?;

        let pixel_pool = PixelPool::new();
        let (tx, rx) = mpsc::sync_channel(2);
        let screen_w = Arc::new(AtomicI32::new(0));
        let screen_h = Arc::new(AtomicI32::new(0));

        // Create Skia CPU renderer BEFORE creating the window,
        // so OffscreenWindow::new() can find it via the SKIA_RENDERER static
        let render_buffer = Rc::new(WaylandRenderBuffer {
            pixel_pool: pixel_pool.clone(),
            wl_tx: Mutex::new(Some(tx.clone())),
            last_x: Cell::new(0),
            last_y: Cell::new(0),
            window_visible: Cell::new(false),
            fixed_position: Cell::new(config.linux.fixed_position),
            corner: RefCell::new(config.linux.corner.clone()),
            fixed_x: Cell::new(config.linux.fixed_x),
            fixed_y: Cell::new(config.linux.fixed_y),
            screen_w: screen_w.clone(),
            screen_h: screen_h.clone(),
        });

        let skia_context = SkiaSharedContext::default();
        let surface = SoftwareSurface::from(render_buffer.clone());
        let skia_renderer = Rc::new(SkiaRenderer::new_with_surface(&skia_context, Box::new(surface)));

        // Store in thread-local for OffscreenWindow::new()
        // Slint auto-calls set_window_adapter() on our SkiaRenderer during window init
        SKIA_RENDERER.with(|s| *s.borrow_mut() = Some(skia_renderer.clone()));

        let candidate_window = CandidateWindow::new().ok()?;
        candidate_window.window().set_size(slint::WindowSize::Physical(slint::PhysicalSize::new(100, 100)));
        slint::platform::update_timers_and_animations();

        let pixel_pool_clone = pixel_pool.clone();
        let (initial_anchor, initial_mt, initial_mr, initial_mb, initial_ml) = {
            let mx = config.linux.fixed_x.max(0);
            let my = config.linux.fixed_y.max(0);
            if config.linux.fixed_position {
                let (a, mt2, mr2, mb2, ml2) = match config.linux.corner.as_str() {
                    "bottom-right" => (Anchor::BOTTOM | Anchor::RIGHT, 0, mx, my, 0),
                    "bottom-left"  => (Anchor::BOTTOM | Anchor::LEFT,  0, 0, my, mx),
                    "top-right"    => (Anchor::TOP | Anchor::RIGHT,    my, mx, 0, 0),
                    "top-left"     => (Anchor::TOP | Anchor::LEFT,     my, 0, 0, mx),
                    _ => (Anchor::BOTTOM | Anchor::RIGHT, 0, mx, my, 0),
                };
                (a, mt2, mr2, mb2, ml2)
            } else {
                (Anchor::TOP | Anchor::LEFT, 0, 0, 0, 0)
            }
        };
        log::info!("[WL_POS] Config: fixed_position={}, corner={}, fixed_x={}, fixed_y={}",
            config.linux.fixed_position, config.linux.corner, config.linux.fixed_x, config.linux.fixed_y);
        log::info!("[WL_POS] Initial layer anchor={:?} margin=({},{},{},{})",
            initial_anchor, initial_mt, initial_mr, initial_mb, initial_ml);
        let join = std::thread::Builder::new()
            .name("wayland-layer".into())
            .spawn(move || wl_thread_main(rx, pixel_pool_clone, initial_anchor, initial_mt, initial_mr, initial_mb, initial_ml, screen_w, screen_h));

        let candidate_enabled = config.linux.show_slint_window;

        let display = Self {
            skia_renderer,
            render_buffer,
            candidate_window,
            config: config.clone(),
            window_visible: false,
            candidate_enabled,
            last_x: 0,
            last_y: 0,
            wl: join.as_ref().ok().map(|_| WlThread { cmd_tx: tx }),
            wl_join: join.ok(),
        };

        display.apply_style(&config);
        Some(display)
    }

    fn render_and_send_candidate(&self, _w: u32, _h: u32) {
        if self.window_visible && self.wl.is_some() {
            // Skia CPU renders into our WaylandRenderBuffer, which sends pixels
            // directly to the Wayland thread — no manual pixel handling needed
            if let Err(e) = self.skia_renderer.render() {
                log::error!("Skia render failed: {e}");
            }
        } else if let Some(ref wl) = self.wl {
            let _ = wl.cmd_tx.send(WlCmd::HideCandidate);
        }
    }

    fn apply_style(&self, config: &Config) {
        let parse_color = |s: &str| -> slint::Color {
            if s.starts_with('#') {
                if s.len() == 7 {
                    let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
                    let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
                    let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
                    slint::Color::from_rgb_u8(r, g, b)
                } else if s.len() == 9 {
                    let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
                    let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
                    let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
                    let a = u8::from_str_radix(&s[7..9], 16).unwrap_or(255);
                    slint::Color::from_argb_u8(a, r, g, b)
                } else {
                    slint::Color::from_rgb_u8(255, 255, 255)
                }
            } else if s.starts_with("rgba(") {
                let parts: Vec<&str> = s[5..s.len() - 1].split(',').map(|p| p.trim()).collect();
                if parts.len() == 4 {
                    let r = parts[0].parse::<u8>().unwrap_or(255);
                    let g = parts[1].parse::<u8>().unwrap_or(255);
                    let b = parts[2].parse::<u8>().unwrap_or(255);
                    let a = (parts[3].parse::<f32>().unwrap_or(1.0) * 255.0) as u8;
                    slint::Color::from_argb_u8(a, r, g, b)
                } else {
                    slint::Color::from_rgb_u8(255, 255, 255)
                }
            } else {
                slint::Color::from_rgb_u8(9, 105, 218)
            }
        };

        self.candidate_window
            .set_is_horizontal(config.appearance.candidate_layout == "horizontal");

        self.candidate_window
            .set_bg_color(parse_color(&config.appearance.window_bg_color));
        self.candidate_window
            .set_accent_color(parse_color(&config.appearance.window_highlight_color));
        self.candidate_window
            .set_border_color(parse_color(&config.appearance.window_border_color));
        self.candidate_window
            .set_text_color(parse_color(&config.appearance.candidate_text.color));
        self.candidate_window
            .set_highlight_text_color(parse_color(&config.appearance.window_highlight_text_color));

        let font_family = config.appearance.candidate_text.font_family.as_str();
        let font_stack = if font_family.is_empty() {
            "Noto Color Emoji, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui".to_string()
        } else {
            format!("{font_family}, Noto Color Emoji, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui")
        };
        self.candidate_window
            .set_pinyin_font_family(slint::SharedString::from(&font_stack));
        self.candidate_window
            .set_candidate_font_family(slint::SharedString::from(&font_stack));
        self.candidate_window
            .set_pinyin_font_size(config.appearance.pinyin_text.font_size as f32);
        self.candidate_window
            .set_pinyin_font_weight(config.appearance.pinyin_text.font_weight as i32);
        self.candidate_window
            .set_candidate_font_size(config.appearance.candidate_text.font_size as f32);
        self.candidate_window
            .set_candidate_font_weight(config.appearance.candidate_text.font_weight as i32);
    }
}

impl CandidateDisplay for WaylandLayerDisplay {
    fn update_candidates(
        &mut self,
        pinyin: &str,
        candidates: Vec<crate::DisplayCandidate>,
        selected: usize,
        page: usize,
        total_pages: usize,
    ) {
        if !self.candidate_enabled || pinyin.is_empty() || !self.config.appearance.show_candidates {
            if self.window_visible {
                log::info!("Hiding candidate window (enabled={} pinyin_len={} show={})",
                    self.candidate_enabled, pinyin.len(), self.config.appearance.show_candidates);
            }
            self.set_visible(false);
            return;
        }

        self.candidate_window
            .set_pinyin(slint::SharedString::from(pinyin));
        self.candidate_window.set_selected_index(selected as i32);
        self.candidate_window.set_current_page(page as i32);
        self.candidate_window.set_total_pages(total_pages as i32);

        let mut cand_models = Vec::new();
        for c in &candidates {
            cand_models.push(CandidateData {
                text: slint::SharedString::from(c.text.clone()),
                label: slint::SharedString::from(c.label.clone()),
                english_aux: slint::SharedString::from(c.hint.clone()),
                stroke_aux: slint::SharedString::from(""),
                is_fuzzy: c.is_fuzzy,
            });
        }
        self.candidate_window.set_candidates(slint::ModelRc::from(
            std::rc::Rc::new(slint::VecModel::from(cand_models)),
        ));

        if !self.window_visible {
            self.window_visible = true;
            self.render_buffer.window_visible.set(true);
            self.candidate_window.set_is_visible(true);
            log::info!("[WL_POS] WaylandLayerDisplay set_visible=true (first show)");
        }

        // Estimate window size based on candidate count and font size.
        // CJK characters are roughly fs pixels wide, ASCII ~ fs * 0.55.
        let fs = self.config.appearance.candidate_text.font_size;
        let cand_count = candidates.len().max(1) as u32;
        let max_chars = candidates.iter()
            .map(|c| c.text.chars().count() + c.label.chars().count() + c.hint.chars().count())
            .max().unwrap_or(8) as u32;

        let is_horizontal = self.config.appearance.candidate_layout == "horizontal";
        let line_height = (fs as f32 * 1.6) as u32;
        let pinyin_height = (fs as f32 * 1.4) as u32;
        let padding = 40u32;

        // Compute max width based on actual screen size
        let (sw, _sh) = self.render_buffer.screen_size();
        let max_w = if self.config.linux.fixed_position {
            // Leave at least the configured margin + 10px on the anchor side
            (sw - self.config.linux.fixed_x - 10).max(200) as u32
        } else {
            // Cursor-follow: use 85% of screen width
            (sw as f32 * 0.85) as u32
        };
        let mut total_w = if is_horizontal {
            (cand_count * (fs * max_chars + 30) + padding).min(max_w)
        } else {
            (fs * max_chars + 120).min(max_w)
        };
        let mut total_h = (pinyin_height + line_height * cand_count + padding).min(1200);

        // In fixed position mode, never shrink — only grow.
        // Combined with right-alignment (set below), this keeps the
        // window visually stable while content stays flush with the anchor edge.
        if self.config.linux.fixed_position {
            let cs = self.candidate_window.window().size();
            total_w = total_w.max(cs.width).min(max_w);
            total_h = total_h.max(cs.height).min(1200);
        }
        
        let current_size = self.candidate_window.window().size();
        if current_size.width != total_w || current_size.height != total_h {
            self.candidate_window.window().set_size(slint::WindowSize::Physical(
                slint::PhysicalSize::new(total_w, total_h),
            ));
        }
        slint::platform::update_timers_and_animations();
        let size = self.candidate_window.window().size();
        log::debug!("[WL_DEBUG] candidate window size: {}x{} (visible={})", size.width, size.height, self.window_visible);

        self.render_and_send_candidate(size.width.max(1), size.height.max(1));
    }

    fn update_status(&mut self, _text: &str, _chinese_enabled: bool) {
        // StatusBar 已移除，状态通过托盘图标显示
    }

    fn move_to(&mut self, x: i32, y: i32) {
        self.last_x = x;
        self.last_y = y;
        self.render_buffer.last_x.set(x);
        self.render_buffer.last_y.set(y);
        // Position update is deferred to the next render call
        // (update_candidates or set_visible). This avoids double-rendering
        // when move_to()+update_candidates() are called in sequence.
    }

    fn set_visible(&mut self, visible: bool) {
        let effective = visible && self.candidate_enabled;
        if effective == self.window_visible {
            return;
        }
        log::info!("Candidate window visibility: {} -> {}", self.window_visible, effective);
        self.window_visible = effective;
        self.render_buffer.window_visible.set(effective);
        self.candidate_window.set_is_visible(effective);
        if effective {
            let size = self.candidate_window.window().size();
            self.render_and_send_candidate(size.width.max(1), size.height.max(1));
        } else if let Some(ref wl) = self.wl {
            let _ = wl.cmd_tx.send(WlCmd::HideCandidate);
        }
    }

    fn apply_config(&mut self, config: &Config) {
        self.config = config.clone();
        self.candidate_enabled = config.linux.show_slint_window;
        self.render_buffer.fixed_position.set(config.linux.fixed_position);
        *self.render_buffer.corner.borrow_mut() = config.linux.corner.clone();
        self.render_buffer.fixed_x.set(config.linux.fixed_x);
        self.render_buffer.fixed_y.set(config.linux.fixed_y);
        self.apply_style(config);
        if self.window_visible {
            let size = self.candidate_window.window().size();
            self.render_and_send_candidate(size.width.max(1), size.height.max(1));
        }
    }

    fn close(&mut self) {
        self.window_visible = false;
        if let Some(ref wl) = self.wl {
            let _ = wl.cmd_tx.send(WlCmd::Exit);
        }
    }
}

impl Drop for WaylandLayerDisplay {
    fn drop(&mut self) {
        self.close();
        if let Some(handle) = self.wl_join.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── calc_anchor tests ──

    #[test]
    fn anchor_top_left_when_room() {
        let (anchor, margin_y, margin_x) = calc_anchor(200, 200, 300, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::TOP | Anchor::LEFT);
        assert_eq!(margin_y, 220);
        assert_eq!(margin_x, 200);
    }

    #[test]
    fn anchor_bottom_when_overflow() {
        // 1000 + 20 + 100 = 1120 > 1080 → bottom
        let (anchor, margin_y, margin_x) = calc_anchor(100, 1000, 200, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::BOTTOM | Anchor::LEFT);
        assert_eq!(margin_y, 80); // 1080 - 1000
        assert_eq!(margin_x, 100);
    }

    #[test]
    fn anchor_right_when_overflow() {
        // 1800 + 300 = 2100 > 1920 → right
        let (anchor, margin_y, margin_x) = calc_anchor(1800, 200, 300, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::TOP | Anchor::RIGHT);
        assert_eq!(margin_y, 220);
        assert_eq!(margin_x, 120); // 1920 - 1800
    }

    #[test]
    fn anchor_bottom_right_both_overflow() {
        let (anchor, margin_y, margin_x) = calc_anchor(1800, 1000, 300, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::BOTTOM | Anchor::RIGHT);
        assert_eq!(margin_y, 80);
        assert_eq!(margin_x, 120);
    }

    #[test]
    fn anchor_no_overflow_at_boundary() {
        // 1620 + 300 = 1920 → exact boundary (no overflow)
        // 960 + 20 + 100 = 1080 → exact boundary (no overflow)
        let (anchor, margin_y, margin_x) = calc_anchor(1620, 960, 300, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::TOP | Anchor::LEFT);
        assert_eq!(margin_y, 980);
        assert_eq!(margin_x, 1620);
    }

    #[test]
    fn anchor_zero_zero_cursor() {
        let (anchor, margin_y, margin_x) = calc_anchor(0, 0, 300, 100, 1920, 1080);
        assert_eq!(anchor, Anchor::TOP | Anchor::LEFT);
        assert_eq!(margin_y, 20);
        assert_eq!(margin_x, 0);
    }

    // ── swizzle_rgba_to_bgra tests ──

    #[test]
    fn swizzle_single_pixel() {
        let src = [0xAA, 0xBB, 0xCC, 0xDD]; // R=A, G=B, B=C, A=D
        let mut dst = [0u8; 4];
        swizzle_rgba_to_bgra(&src, &mut dst);
        assert_eq!(dst, [0xCC, 0xBB, 0xAA, 0xDD]); // B=C, G=B, R=A, A=D
    }

    #[test]
    fn swizzle_two_pixels() {
        let src = [0x01, 0x02, 0x03, 0x04, 0x10, 0x20, 0x30, 0x40];
        let mut dst = [0u8; 8];
        swizzle_rgba_to_bgra(&src, &mut dst);
        assert_eq!(dst, [0x03, 0x02, 0x01, 0x04, 0x30, 0x20, 0x10, 0x40]);
    }

    #[test]
    fn swizzle_dst_longer_than_src() {
        let src = [0x01, 0x02, 0x03, 0x04];
        let mut dst = [0xFF; 8];
        swizzle_rgba_to_bgra(&src, &mut dst);
        // Only first 4 bytes swizzled, rest untouched
        assert_eq!(dst, [0x03, 0x02, 0x01, 0x04, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn swizzle_src_longer_than_dst() {
        let src = [0x01, 0x02, 0x03, 0x04, 0x10, 0x20, 0x30, 0x40];
        let mut dst = [0u8; 4];
        swizzle_rgba_to_bgra(&src, &mut dst);
        assert_eq!(dst, [0x03, 0x02, 0x01, 0x04]);
    }

    #[test]
    fn swizzle_empty() {
        swizzle_rgba_to_bgra(&[], &mut []);
    }

    #[test]
    fn swizzle_transparent_pixel() {
        let src = [0xFF, 0x00, 0x00, 0x00]; // Red, fully transparent
        let mut dst = [0u8; 4];
        swizzle_rgba_to_bgra(&src, &mut dst);
        assert_eq!(dst, [0x00, 0x00, 0xFF, 0x00]); // alpha preserved
    }
}
