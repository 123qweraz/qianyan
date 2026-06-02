use crate::CandidateDisplay;
use qianyan_ime_core::Config;
use slint::ComponentHandle;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

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

use i_slint_core::window::{WindowAdapter, WindowInner};

slint::include_modules!();

// ---- Offscreen window adapter ----

struct OffscreenWindow {
    window: slint::Window,
    renderer: slint::platform::software_renderer::SoftwareRenderer,
    size: std::cell::Cell<slint::PhysicalSize>,
    needs_redraw: std::cell::Cell<bool>,
}

impl OffscreenWindow {
    fn new() -> Rc<Self> {
        Rc::new_cyclic(|w: &std::rc::Weak<Self>| Self {
            window: slint::Window::new(w.clone()),
            renderer: slint::platform::software_renderer::SoftwareRenderer::new(),
            size: std::cell::Cell::new(slint::PhysicalSize::default()),
            needs_redraw: std::cell::Cell::new(false),
        })
    }
}

impl OffscreenWindow {
    fn software_renderer(&self) -> &slint::platform::software_renderer::SoftwareRenderer {
        &self.renderer
    }
}

impl WindowAdapter for OffscreenWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }
    fn size(&self) -> slint::PhysicalSize {
        let s = self.size.get();
        eprintln!("[WL_DEBUG] OffscreenWindow::size() = {}x{}", s.width, s.height);
        s
    }
    fn set_size(&self, size: slint::WindowSize) {
        let sf = self.window.scale_factor();
        let phys = size.to_physical(sf);
        eprintln!("[WL_DEBUG] OffscreenWindow::set_size({}x{})", phys.width, phys.height);
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
    static INIT: std::sync::Once = std::sync::Once::new();
    static mut RESULT: Option<()> = None;
    INIT.call_once(|| {
        let (tx, rx) = mpsc::channel::<EventCallback>();
        let platform = Box::new(SlintPlatform {
            running: Arc::new(AtomicBool::new(true)),
            cmd_tx: tx,
            cmd_rx: rx,
        });
        unsafe {
            RESULT = slint::platform::set_platform(platform).ok();
        }
    });
    unsafe { RESULT }
}


// ---- Wayland thread ----

#[derive(Clone)]
struct PixelPool(Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

impl PixelPool {
    fn new() -> Self {
        Self(Arc::new(std::sync::Mutex::new(Vec::with_capacity(4))))
    }
    fn get(&self, size: usize) -> Vec<u8> {
        let mut pool = self.0.lock().unwrap();
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
        let mut pool = self.0.lock().unwrap();
        if pool.len() < 8 {
            pool.push(v);
        }
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
    exit: AtomicBool,
    pixel_pool: PixelPool,
    layer_closed: bool,
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
        layer.commit();
        self.candidate_layer = Some(layer.clone());
        self.layer_closed = false;
        eprintln!("[WL_DEBUG] Candidate layer surface recreated after compositor closed");
        Some(layer)
    }
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
        panic!("event_created_child for WlSurface")
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

impl OutputHandler for WlState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self._output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wayland_client::protocol::wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wayland_client::protocol::wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wayland_client::protocol::wl_output::WlOutput) {}
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
        eprintln!("[WL_DEBUG] Layer surface closed by compositor, marking for re-creation");
        self.candidate_layer = None;
        self.layer_closed = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        _cfg: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // Acknowledge configure by committing the surface
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

fn wl_thread_main(rx: Receiver<WlCmd>, pixel_pool: PixelPool) {
    eprintln!("[WL_DEBUG] Wayland thread started");
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
    eprintln!("[WL_DEBUG] Wayland globals obtained");

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
        exit: AtomicBool::new(false),
        pixel_pool: pixel_pool.clone(),
        layer_closed: false,
    };

    // Create candidate layer surface
    {
        let surf = state.compositor_state.create_surface(&qh);
        let layer = state
            .layer_shell
            .as_ref()
            .unwrap()
            .create_layer_surface(&qh, surf, Layer::Overlay, Some("qianyan-ime-candidate"), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(400, 200);
        layer.commit();
        state.candidate_layer = Some(layer);
        eprintln!("[WL_DEBUG] Candidate layer surface created");
    }
    if let Ok(pool) = SlotPool::new(4 * 1024 * 1024, &state.shm) {
        state.candidate_pool = Some(pool);
        eprintln!("[WL_DEBUG] Candidate pool created (4MB)");
    }

    let _ = event_queue.dispatch_pending(&mut state);
    let _ = event_queue.flush();
    eprintln!("[WL_DEBUG] Wayland init done, entering main loop");

    loop {
        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    WlCmd::ShowCandidate { x, y, w, h, anchor, pixels } => {
                        eprintln!("[WL_DEBUG] ShowCandidate: x={} y={} w={} h={} anchor={:?} pixels={}", x, y, w, h, anchor, pixels.len());
                        if let Some(layer) = state.ensure_layer(&qh) {
                            layer.set_anchor(anchor);
                            layer.set_size(w.max(1), h.max(1));
                            let has_top = anchor.contains(Anchor::TOP);
                            let has_bottom = anchor.contains(Anchor::BOTTOM);
                            let has_left = anchor.contains(Anchor::LEFT);
                            let has_right = anchor.contains(Anchor::RIGHT);
                            layer.set_margin(
                                if has_top { y.max(0) as i32 } else { 0 },
                                if has_right { x.max(0) as i32 } else { 0 },
                                if has_bottom { y.max(0) as i32 } else { 0 },
                                if has_left { x.max(0) as i32 } else { 0 },
                            );
                            if let Some(ref mut pool) = state.candidate_pool {
                                submit_to_layer(pool, &layer, &pixels, w.max(1), h.max(1));
                            }
                        } else {
                            eprintln!("[WL_DEBUG] candidate_layer is None!");
                        }
                        // Reuse pixel buffer
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
                        eprintln!("[WL_DEBUG] Wayland thread received Exit, terminating");
                        return;
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    eprintln!("[WL_DEBUG] Wayland thread channel disconnected, terminating");
                    return;
                }
            }
        }

        if state.exit.load(Ordering::SeqCst) {
            eprintln!("[WL_DEBUG] Wayland thread exit flag set, terminating");
            break;
        }

        if event_queue.dispatch_pending(&mut state).is_err() {
            log::error!("Wayland dispatch failed, exiting thread");
            break;
        }
        if event_queue.flush().is_err() {
            log::error!("Wayland flush failed, exiting thread");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    eprintln!("[WL_DEBUG] Wayland thread main loop exited");
}

fn submit_to_layer(
    pool: &mut SlotPool,
    layer: &LayerSurface,
    pixels: &[u8],
    width: u32,
    height: u32,
) {
    let stride = (width * 4) as i32;
    let needed = (stride * height as i32) as usize;
    const MAX_POOL_SIZE: usize = 32 * 1024 * 1024;
    if needed > pool.len() {
        let new_size = needed.next_power_of_two().max(1024 * 1024);
        if new_size > MAX_POOL_SIZE {
            log::error!("SHM pool would exceed max size {}MB, skipping render", MAX_POOL_SIZE / 1024 / 1024);
            return;
        }
        log::info!("SHM pool resize: {} -> {} (needed={})", pool.len(), new_size, needed);
        if pool.resize(new_size).is_err() {
            log::error!("Failed to resize SHM pool");
            return;
        }
    }
    if let Ok((buffer, canvas)) = pool.create_buffer(width as i32, height as i32, stride, wl_shm::Format::Argb8888) {
        let n = canvas.len().min(pixels.len());
        canvas[..n].copy_from_slice(&pixels[..n]);
        if buffer.attach_to(layer.wl_surface()).is_err() {
            log::error!("Failed to attach buffer");
        }
        layer.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
        layer.commit();
    }
}

// ---- WaylandLayerDisplay ----

struct WlThread {
    cmd_tx: mpsc::SyncSender<WlCmd>,
}

pub struct WaylandLayerDisplay {
    renderer_ptr: *const slint::platform::software_renderer::SoftwareRenderer,
    candidate_window: CandidateWindow,
    config: Config,
    window_visible: bool,
    candidate_enabled: bool,
    last_x: i32,
    last_y: i32,
    wl: Option<WlThread>,
    pixel_pool: PixelPool,
}

impl WaylandLayerDisplay {
    pub fn new(config: Config) -> Option<Self> {
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return None;
        }

        setup_slint_platform()?;

        let candidate_window = CandidateWindow::new().ok()?;

        // Set initial sizes for the layer surfaces before any content arrives.
        candidate_window.window().set_size(slint::WindowSize::Physical(slint::PhysicalSize::new(100, 100)));
        slint::platform::update_timers_and_animations();

        // Get the renderer from the offscreen window adapter.
        // WindowAdapter requires Any as supertrait, so transmute is valid.
        // SAFETY: adapter points to an OffscreenWindow (we created it in the platform).
        // Rc::as_ptr returns the data pointer of the trait object.
        let inner = WindowInner::from_pub(candidate_window.window());
        let adapter = inner.window_adapter();
        let ow: &OffscreenWindow = unsafe {
            let raw: *const dyn WindowAdapter = &*adapter;
            &*(raw as *const OffscreenWindow)
        };
        let renderer_ptr = ow.software_renderer() as *const slint::platform::software_renderer::SoftwareRenderer;

        let pixel_pool = PixelPool::new();
        let pixel_pool_clone = pixel_pool.clone();
        let (tx, rx) = mpsc::sync_channel(2);
        let join = std::thread::Builder::new()
            .name("wayland-layer".into())
            .spawn(move || wl_thread_main(rx, pixel_pool_clone));

        let candidate_enabled = config.linux.show_slint_window;

        let display = Self {
            renderer_ptr,
            candidate_window,
            config: config.clone(),
            window_visible: false,
            candidate_enabled,
            last_x: 0,
            last_y: 0,
            wl: join.ok().map(|_| WlThread { cmd_tx: tx }),
            pixel_pool,
        };

        display.apply_style(&config);
        Some(display)
    }

    fn renderer(&self) -> &slint::platform::software_renderer::SoftwareRenderer {
        unsafe { &*self.renderer_ptr }
    }

    fn screen_size() -> (i32, i32) {
        if let Ok(out) = std::process::Command::new("xdotool")
            .arg("getdisplaygeometry")
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let parts: Vec<&str> = s.trim().split_whitespace().collect();
                if parts.len() == 2 {
                    if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                        return (w, h);
                    }
                }
            }
        }
        (1920, 1080)
    }

    fn render_and_send_candidate(&self, w: u32, h: u32) {
        if self.window_visible && self.wl.is_some() {
            let _window = self.candidate_window.window();
            
            let pixel_count = (w * h) as usize;
            let mut pixels = self.pixel_pool.get(pixel_count * 4);
            
            let buf: &mut [slint::platform::software_renderer::PremultipliedRgbaColor] =
                bytemuck::cast_slice_mut(&mut pixels);
            self.renderer().render(buf, w as usize);

            // RGBA -> BGRA for wl_shm Argb8888
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }

            let (anchor, margin_a, margin_b) = if self.config.linux.fixed_position {
                match self.config.linux.corner.as_str() {
                    "top-right" => (Anchor::TOP | Anchor::RIGHT, self.config.linux.fixed_y, self.config.linux.fixed_x),
                    "bottom-left" => (Anchor::BOTTOM | Anchor::LEFT, self.config.linux.fixed_y, self.config.linux.fixed_x),
                    "bottom-right" => (Anchor::BOTTOM | Anchor::RIGHT, self.config.linux.fixed_y, self.config.linux.fixed_x),
                    _ => (Anchor::TOP | Anchor::LEFT, self.config.linux.fixed_y, self.config.linux.fixed_x),
                }
            } else {
                let cursor_x = self.last_x;
                let cursor_y = self.last_y;
                let (sw, sh) = Self::screen_size();
                let w32 = w as i32;
                let h32 = h as i32;
                let offset = 20i32;
                let use_bottom = cursor_y + offset + h32 > sh;
                let use_right = cursor_x + w32 > sw;
                let anchor_v = if use_bottom { Anchor::BOTTOM } else { Anchor::TOP };
                let anchor_h = if use_right { Anchor::RIGHT } else { Anchor::LEFT };
                let anchor = anchor_v | anchor_h;
                let margin_a = if use_bottom { sh - cursor_y } else { cursor_y + offset };
                let margin_b = if use_right { sw - cursor_x } else { cursor_x };
                (anchor, margin_a, margin_b)
            };

            let cmd = WlCmd::ShowCandidate { x: margin_b, y: margin_a, w, h, anchor, pixels };
            if let Err(e) = self.wl.as_ref().unwrap().cmd_tx.send(cmd) {
                log::error!("Wayland channel disconnected: {e:?}");
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

        let font_stack = format!(
            "{}, Noto Color Emoji, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui",
            config.appearance.candidate_text.font_family
        );
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
            self.candidate_window.set_is_visible(true);
        }

        // Offscreen window doesn't auto-resize via Slint's layout bindings,
        // so set a generous width to fit all candidates horizontally.
        let fs = self.config.appearance.candidate_text.font_size as u32;
        let max_chars = candidates.iter().map(|c| c.text.chars().count() + c.label.chars().count() + c.hint.chars().count()).max().unwrap_or(8) as u32;
        let per_cand_w = ((fs * max_chars) / 2 + 40).max(80);
        let total_w = (candidates.len() as u32 * per_cand_w + 80).min(1600).max(200);
        let total_h = 200u32;
        
        let current_size = self.candidate_window.window().size();
        if current_size.width as u32 != total_w || current_size.height as u32 != total_h {
            self.candidate_window.window().set_size(slint::WindowSize::Physical(
                slint::PhysicalSize::new(total_w, total_h),
            ));
        }
        slint::platform::update_timers_and_animations();
        let size = self.candidate_window.window().size();
        eprintln!("[WL_DEBUG] candidate window size: {}x{} (visible={})", size.width, size.height, self.window_visible);

        self.render_and_send_candidate(size.width.max(1), size.height.max(1));
    }

    fn update_status(&mut self, _text: &str, _chinese_enabled: bool) {
        // StatusBar 已移除，状态通过托盘图标显示
    }

    fn move_to(&mut self, x: i32, y: i32) {
        self.last_x = x;
        self.last_y = y;
        if self.window_visible {
            let size = self.candidate_window.window().size();
            self.render_and_send_candidate(size.width.max(1), size.height.max(1));
        }
    }

    fn set_visible(&mut self, visible: bool) {
        let effective = visible && self.candidate_enabled;
        if effective == self.window_visible {
            return;
        }
        self.window_visible = effective;
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
    }
}
