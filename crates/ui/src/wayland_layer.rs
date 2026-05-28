use crate::CandidateDisplay;
use qianyan_ime_core::Config;
use slint::ComponentHandle;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
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

fn render_window_to_buffer(
    _window: &slint::Window,
    width: u32,
    height: u32,
    renderer: &slint::platform::software_renderer::SoftwareRenderer,
) -> Vec<u8> {
    eprintln!("[WL_DEBUG] render_window_to_buffer: {}x{}", width, height);

    let pixel_count = (width * height) as usize;
    let mut rgba_pixels = vec![0u8; pixel_count * 4];

    let buf: &mut [slint::platform::software_renderer::PremultipliedRgbaColor] =
        bytemuck::cast_slice_mut(&mut rgba_pixels);
    renderer.render(buf, width as usize);

    // RGBA -> BGRA for wl_shm Argb8888
    for pixel in rgba_pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    rgba_pixels
}

// ---- Wayland thread ----

struct WlState {
    registry_state: RegistryState,
    compositor_state: CompositorState,
    shm: Shm,
    layer_shell: Option<LayerShell>,
    _output_state: OutputState,
    _seat_state: SeatState,
    candidate_layer: Option<LayerSurface>,
    candidate_pool: Option<SlotPool>,
    status_layer: Option<LayerSurface>,
    status_pool: Option<SlotPool>,
    exit: AtomicBool,
}

delegate_registry!(WlState);
delegate_compositor!(WlState);
delegate_output!(WlState);
delegate_shm!(WlState);
delegate_seat!(WlState);
delegate_keyboard!(WlState);
delegate_layer!(WlState);

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
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit.store(true, Ordering::SeqCst);
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
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
    ShowCandidate { x: i32, y: i32, w: u32, h: u32, pixels: Vec<u8> },
    HideCandidate,
    ShowStatus { x: i32, y: i32, w: u32, h: u32, pixels: Vec<u8> },
    HideStatus,
    Exit,
}

fn wl_thread_main(rx: Receiver<WlCmd>) {
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
        status_layer: None,
        status_pool: None,
        exit: AtomicBool::new(false),
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

    // Create status layer surface
    {
        let surf = state.compositor_state.create_surface(&qh);
        let layer = state
            .layer_shell
            .as_ref()
            .unwrap()
            .create_layer_surface(&qh, surf, Layer::Overlay, Some("qianyan-ime-status"), None);
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(60, 28);
        layer.commit();
        state.status_layer = Some(layer);
        eprintln!("[WL_DEBUG] Status layer surface created");
    }
    if let Ok(pool) = SlotPool::new(4 * 1024 * 1024, &state.shm) {
        state.status_pool = Some(pool);
        eprintln!("[WL_DEBUG] Status pool created (4MB)");
    }

    let _ = event_queue.dispatch_pending(&mut state);
    let _ = event_queue.flush();
    eprintln!("[WL_DEBUG] Wayland init done, entering main loop");

    loop {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                WlCmd::ShowCandidate { x, y, w, h, pixels } => {
                    eprintln!("[WL_DEBUG] ShowCandidate: x={} y={} w={} h={} pixels={}", x, y, w, h, pixels.len());
                    if let Some(ref layer) = state.candidate_layer {
                        layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
                        layer.set_size(w.max(1), h.max(1));
                        // When cursor y is available, place window above the cursor.
                        // When y=0 (no cursor info), window sits at bottom-left.
                        layer.set_margin(0, 0, y.max(0) as i32, x.max(0) as i32);
                        if let Some(ref mut pool) = state.candidate_pool {
                            submit_to_layer(pool, layer, &pixels, w.max(1), h.max(1));
                        }
                    } else {
                        eprintln!("[WL_DEBUG] candidate_layer is None!");
                    }
                }
                WlCmd::HideCandidate => {
                    // Don't attach None (which unmaps the surface, causing issues on KDE).
                    // Use a 1x1 transparent buffer instead so the surface stays mapped.
                    if let Some(ref mut pool) = state.candidate_pool {
                        if let Some(ref layer) = state.candidate_layer {
                            if let Ok((buffer, canvas)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
                                canvas[0..4].copy_from_slice(&[0, 0, 0, 0]);
                                if buffer.attach_to(layer.wl_surface()).is_ok() {
                                    layer.wl_surface().damage_buffer(0, 0, 1, 1);
                                    layer.commit();
                                }
                            }
                        }
                    }
                }
                WlCmd::ShowStatus { x, y, w, h, pixels } => {
                    if let Some(ref layer) = state.status_layer {
                        layer.set_anchor(Anchor::BOTTOM | Anchor::LEFT);
                        layer.set_size(w.max(1), h.max(1));
                        layer.set_margin(0, 0, (y + 28).max(0) as i32, x.max(0) as i32);
                        if let Some(ref mut pool) = state.status_pool {
                            submit_to_layer(pool, layer, &pixels, w.max(1), h.max(1));
                        }
                    }
                }
                WlCmd::HideStatus => {
                    if let Some(ref mut pool) = state.status_pool {
                        if let Some(ref layer) = state.status_layer {
                            if let Ok((buffer, canvas)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
                                canvas[0..4].copy_from_slice(&[0, 0, 0, 0]);
                                if buffer.attach_to(layer.wl_surface()).is_ok() {
                                    layer.wl_surface().damage_buffer(0, 0, 1, 1);
                                    layer.commit();
                                }
                            }
                        }
                    }
                }
                WlCmd::Exit => return,
            }
        }

        if state.exit.load(Ordering::SeqCst) {
            break;
        }

        let _ = event_queue.dispatch_pending(&mut state);
        let _ = event_queue.flush();
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
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
    if needed > pool.len() {
        let new_size = needed.next_power_of_two().max(1024 * 1024);
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
    cmd_tx: Sender<WlCmd>,
}

pub struct WaylandLayerDisplay {
    // Raw pointer to the SoftwareRenderer owned by OffscreenWindow.
    // Safe because OffscreenWindow lives as long as WaylandLayerDisplay
    // (candidate_window owns the adapter chain).
    renderer_ptr: *const slint::platform::software_renderer::SoftwareRenderer,
    candidate_window: CandidateWindow,
    status_bar: StatusBar,
    config: Config,
    window_visible: bool,
    candidate_enabled: bool,
    last_x: i32,
    last_y: i32,
    status_bar_visible: bool,
    wl: Option<WlThread>,
}

impl WaylandLayerDisplay {
    pub fn new(config: Config) -> Option<Self> {
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return None;
        }

        setup_slint_platform()?;

        let candidate_window = CandidateWindow::new().ok()?;
        let status_bar = StatusBar::new().ok()?;

        // Set initial sizes for the layer surfaces before any content arrives.
        // Candidate window: fixed 600px wide, height auto.
        candidate_window.window().set_size(slint::WindowSize::Physical(slint::PhysicalSize::new(600, 100)));
        // Status bar: will be sized by its binding.
        status_bar.window().set_size(slint::WindowSize::Physical(slint::PhysicalSize::new(60, 28)));
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

        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("wayland-layer".into())
            .spawn(move || wl_thread_main(rx));

        let candidate_enabled = config.linux.show_slint_window;

        let display = Self {
            renderer_ptr,
            candidate_window,
            status_bar,
            config: config.clone(),
            window_visible: false,
            candidate_enabled,
            last_x: 0,
            last_y: 0,
            status_bar_visible: false,
            wl: join.ok().map(|_| WlThread { cmd_tx: tx }),
        };

        display.apply_style(&config);
        Some(display)
    }

    fn renderer(&self) -> &slint::platform::software_renderer::SoftwareRenderer {
        unsafe { &*self.renderer_ptr }
    }

    fn render_and_send_candidate(&self, w: u32, h: u32) {
        if self.window_visible && self.wl.is_some() {
            let window = self.candidate_window.window();
            let pixels = render_window_to_buffer(window, w, h, self.renderer());
            let _ = self.wl.as_ref().unwrap().cmd_tx.send(WlCmd::ShowCandidate {
                x: self.last_x,
                y: self.last_y + 20,
                w,
                h,
                pixels,
            });
        } else if let Some(ref wl) = self.wl {
            let _ = wl.cmd_tx.send(WlCmd::HideCandidate);
        }
    }

    fn render_and_send_status(&self) {
        if self.status_bar_visible {
            if let Some(ref wl) = self.wl {
                let window = self.status_bar.window();
                let pixels = render_window_to_buffer(window, 60, 28, self.renderer());
                let _ = wl.cmd_tx.send(WlCmd::ShowStatus {
                    x: self.last_x,
                    y: self.last_y,
                    w: 60,
                    h: 28,
                    pixels,
                });
            }
        } else if let Some(ref wl) = self.wl {
            let _ = wl.cmd_tx.send(WlCmd::HideStatus);
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
            .set_show_english_aux(config.appearance.show_english_aux);
        self.candidate_window
            .set_show_stroke_aux(config.appearance.show_stroke_aux);
        self.candidate_window
            .set_show_translation(config.appearance.show_english_translation);
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
            "{}, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui",
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
        for c in candidates {
            cand_models.push(CandidateData {
                text: slint::SharedString::from(c.text),
                label: slint::SharedString::from(c.label),
                english_aux: slint::SharedString::from(c.hint),
                stroke_aux: slint::SharedString::from(""),
                is_fuzzy: c.is_fuzzy,
            });
        }
        self.candidate_window.set_candidates(slint::ModelRc::from(
            std::rc::Rc::new(slint::VecModel::from(cand_models)),
        ));

        // Let Slint compute the preferred size from content, twice to ensure
        // the binding evaluation propagates through to adapter set_size.
        slint::platform::update_timers_and_animations();
        slint::platform::update_timers_and_animations();
        let size = self.candidate_window.window().size();
        eprintln!("[WL_DEBUG] candidate window size: {}x{} (visible={})", size.width, size.height, self.window_visible);

        // Always render content regardless of visibility state.
        if !self.window_visible {
            self.window_visible = true;
            self.candidate_window.set_is_visible(true);
        }
        self.render_and_send_candidate(size.width.max(1), size.height.max(1));
    }

    fn update_status(&mut self, text: &str, chinese_enabled: bool) {
        if !text.is_empty() {
            self.status_bar
                .set_status_text(slint::SharedString::from(text));
        }
        self.status_bar.set_chinese_enabled(chinese_enabled);
        slint::platform::update_timers_and_animations();
        self.render_and_send_status();
    }

    fn set_status_bar_visible(&mut self, visible: bool) {
        self.status_bar_visible = visible;
        self.config.appearance.show_status_bar = visible;
        self.status_bar.set_bar_visible(visible);
        self.render_and_send_status();
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
