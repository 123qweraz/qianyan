use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use i_slint_renderer_skia::skia_safe;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::{slot::SlotPool, Shm, ShmHandler};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::seat::{SeatHandler, SeatState};
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_seat, delegate_shm,
    delegate_layer,
};
use wayland_client::backend::ObjectData;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use std::os::fd::AsRawFd;

use qianyan_ime_core::Config;

// ── Color parsing ──

fn parse_color(s: &str) -> skia_safe::Color {
    if s.starts_with('#') && s.len() == 7 {
        let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
        let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
        let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
        skia_safe::Color::from_rgb(r, g, b)
    } else if s.starts_with("rgba(") && s.ends_with(')') {
        let inner = &s[5..s.len() - 1];
        let parts: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() == 4 {
            skia_safe::Color::from_argb(
                (parts[3] * 255.0) as u8,
                (parts[0]) as u8,
                (parts[1]) as u8,
                (parts[2]) as u8,
            )
        } else if parts.len() == 3 {
            skia_safe::Color::from_rgb(parts[0] as u8, parts[1] as u8, parts[2] as u8)
        } else {
            skia_safe::Color::WHITE
        }
    } else {
        skia_safe::Color::WHITE
    }
}

// ── Position parse ──

fn parse_position(s: &str) -> Anchor {
    match s {
        "bottom" | "" => Anchor::BOTTOM,
        "top" => Anchor::TOP,
        "bottom-right" => Anchor::BOTTOM | Anchor::RIGHT,
        "bottom-left" => Anchor::BOTTOM | Anchor::LEFT,
        "top-right" => Anchor::TOP | Anchor::RIGHT,
        "top-left" => Anchor::TOP | Anchor::LEFT,
        _ => Anchor::BOTTOM,
    }
}

// ── Wayland keystroke renderer ──

struct KeystrokeWlState {
    registry_state: RegistryState,
    compositor_state: CompositorState,
    shm: Shm,
    layer_shell: Option<LayerShell>,
    _output_state: OutputState,
    _seat_state: SeatState,
    layer: Option<LayerSurface>,
    pool: Option<SlotPool>,
    exit: AtomicBool,
    keys: Vec<String>,
    mods: Vec<String>,
    hide_deadline: Option<Instant>,
    timeout_ms: u64,
    font_size: f32,
    bg_color: skia_safe::Color,
    text_color: skia_safe::Color,
}

delegate_registry!(KeystrokeWlState);
delegate_compositor!(KeystrokeWlState);
delegate_output!(KeystrokeWlState);
delegate_shm!(KeystrokeWlState);
delegate_seat!(KeystrokeWlState);
delegate_layer!(KeystrokeWlState);

struct ChildSurfaceData;
impl ObjectData for ChildSurfaceData {
    fn event(
        self: std::sync::Arc<Self>,
        _handle: &wayland_client::backend::Backend,
        _msg: wayland_client::backend::protocol::Message<wayland_client::backend::ObjectId, std::os::unix::io::OwnedFd>,
    ) -> Option<std::sync::Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_client::backend::ObjectId) {}
}

impl Dispatch<WlSurface, ()> for KeystrokeWlState {
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
    ) -> std::sync::Arc<dyn ObjectData> {
        std::sync::Arc::new(ChildSurfaceData)
    }
}

impl CompositorHandler for KeystrokeWlState {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: wayland_client::protocol::wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: &WlOutput) {}
}

impl OutputHandler for KeystrokeWlState {
    fn output_state(&mut self) -> &mut OutputState { &mut self._output_state }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl ShmHandler for KeystrokeWlState {
    fn shm_state(&mut self) -> &mut Shm { &mut self.shm }
}

impl SeatHandler for KeystrokeWlState {
    fn seat_state(&mut self) -> &mut SeatState { &mut self._seat_state }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: smithay_client_toolkit::seat::Capability) {}
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat, _: smithay_client_toolkit::seat::Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl LayerShellHandler for KeystrokeWlState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _layer: &LayerSurface) {
        log::warn!("[KSO] Keystroke layer surface closed by compositor");
        self.layer = None;
    }
    fn configure(&mut self, _: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface, _cfg: LayerSurfaceConfigure, _serial: u32) {
        layer.commit();
    }
}

impl ProvidesRegistryState for KeystrokeWlState {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry_state }
    fn runtime_add_global(&mut self, _: &Connection, _: &QueueHandle<Self>, _: u32, _: &str, _: u32) {}
    fn runtime_remove_global(&mut self, _: &Connection, _: &QueueHandle<Self>, _: u32, _: &str) {}
}

enum KSCmd {
    Show { keys: Vec<String>, mods: Vec<String> },
    Exit,
}

fn render_keystroke_pixels(
    keys: &[String],
    mods: &[String],
    typeface: &skia_safe::Typeface,
    font_size: f32,
    bg_color: skia_safe::Color,
    text_color: skia_safe::Color,
) -> (Vec<u8>, u32, u32) {
    let mod_font_size = font_size * 0.6;

    let font = skia_safe::Font::new(typeface, font_size);
    let mod_font = skia_safe::Font::new(typeface, mod_font_size);

    let padding_x = font_size * 0.8;
    let key_gap = font_size * 0.4;
    let mod_gap = font_size * 0.2;
    let mod_height = font_size * 1.1;
    let bg_radius = font_size * 0.4;
    let overlay_height = font_size * 2.8;

    let mut total_w = padding_x * 2.0;
    for m in mods {
        total_w += mod_font.measure_str(m, None).0 + mod_gap * 2.0 + 12.0;
    }
    if !mods.is_empty() && !keys.is_empty() {
        total_w += 4.0;
    }
    for k in keys {
        total_w += font.measure_str(k, None).0 + key_gap;
    }
    if total_w > padding_x * 2.0 {
        total_w -= key_gap;
    }

    let w = (total_w.ceil() as u32).max(20).min(2000);
    let h = (overlay_height.ceil() as u32).max(20);
    let pixel_count = (w * h) as usize;

    let mut pixels = vec![0u8; pixel_count * 4];
    let image_info = skia_safe::ImageInfo::new(
        (w as i32, h as i32),
        skia_safe::ColorType::BGRA8888,
        skia_safe::AlphaType::Premul,
        None,
    );
    let Some(mut surface) = skia_safe::surfaces::wrap_pixels(
        &image_info,
        pixels.as_mut_slice(),
        (w * 4) as usize,
        None,
    ) else {
        return (pixels, w, h);
    };

    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);

    let mut bg_paint = skia_safe::Paint::default();
    bg_paint.set_color(bg_color);
    bg_paint.set_anti_alias(true);
    canvas.draw_rrect(
        skia_safe::RRect::new_rect_radii(
            skia_safe::Rect::from_wh(w as f32, h as f32),
            &[skia_safe::Point::new(bg_radius, bg_radius); 4],
        ),
        &bg_paint,
    );

    let mut x = padding_x;

    let mut mod_bg_paint = skia_safe::Paint::default();
    mod_bg_paint.set_color(skia_safe::Color::from_argb(40, 255, 255, 255));
    mod_bg_paint.set_anti_alias(true);

    let mut text_paint = skia_safe::Paint::default();
    let mod_y = (overlay_height - mod_height) / 2.0 + 2.0;
    for m in mods {
        let mw = mod_font.measure_str(m, None).0 + 12.0;
        canvas.draw_rrect(
            skia_safe::RRect::new_rect_radii(
                skia_safe::Rect::from_xywh(x, mod_y, mw, mod_height),
                &[skia_safe::Point::new(4.0, 4.0); 4],
            ),
            &mod_bg_paint,
        );
        text_paint.set_color(skia_safe::Color::from_argb(200, 200, 200, 200));
        canvas.draw_str(m, (x + 6.0, mod_y + mod_height * 0.75), &mod_font, &text_paint);
        x += mw + mod_gap;
    }

    if !mods.is_empty() && !keys.is_empty() {
        x += 4.0;
    }

    let key_y = overlay_height * 0.68;
    text_paint.set_color(text_color);
    for k in keys {
        canvas.draw_str(k, (x + 1.0, key_y), &font, &text_paint);
        x += font.measure_str(k, None).0 + key_gap;
    }

    drop(surface);
    (pixels, w, h)
}

fn wl_keystroke_thread(
    rx: Receiver<KSCmd>,
    timeout_ms: u64,
    position: Anchor,
    font_size: f32,
    bg_color: skia_safe::Color,
    text_color: skia_safe::Color,
) {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            log::error!("[KSO] cannot connect to Wayland: {e}");
            return;
        }
    };

    let (globals, mut event_queue) = match registry_queue_init(&conn) {
        Ok(g) => g,
        Err(e) => {
            log::error!("[KSO] registry init failed: {e}");
            return;
        }
    };
    let qh = event_queue.handle();

    let compositor = match CompositorState::bind(&globals, &qh) {
        Ok(c) => c,
        Err(e) => {
            log::error!("[KSO] no wl_compositor: {e}");
            return;
        }
    };
    let shm = match Shm::bind(&globals, &qh) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[KSO] no wl_shm: {e}");
            return;
        }
    };
    let ls = match LayerShell::bind(&globals, &qh) {
        Ok(ls) => ls,
        Err(e) => {
            log::error!("[KSO] no zwlr_layer_shell_v1: {e}");
            return;
        }
    };

    let mut state = KeystrokeWlState {
        registry_state: RegistryState::new(&globals),
        compositor_state: compositor,
        shm,
        layer_shell: Some(ls),
        _output_state: OutputState::new(&globals, &qh),
        _seat_state: SeatState::new(&globals, &qh),
        layer: None,
        pool: None,
        exit: AtomicBool::new(false),
        keys: Vec::new(),
        mods: Vec::new(),
        hide_deadline: None,
        timeout_ms,
        font_size,
        bg_color,
        text_color,
    };

    {
        let surf = state.compositor_state.create_surface(&qh);
        let layer = state.layer_shell.as_ref().unwrap()
            .create_layer_surface(&qh, surf, Layer::Overlay, Some("qianyan-ime-keystroke"), None);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_anchor(position);
        layer.set_size(1, 1);
        layer.commit();
        state.layer = Some(layer);
    }
    if let Ok(pool) = SlotPool::new(1_024_000, &state.shm) {
        state.pool = Some(pool);
    }

    let _ = event_queue.dispatch_pending(&mut state);
    let _ = event_queue.flush();

    let fm = skia_safe::FontMgr::default();
    let typeface = fm.legacy_make_typeface(None, skia_safe::FontStyle::default())
        .expect("no default typeface available");

    loop {
        loop {
            match rx.try_recv() {
                Ok(KSCmd::Show { keys, mods }) => {
                    log::debug!("[KSO] Show keys={:?} mods={:?}", keys, mods);
                    state.keys = keys;
                    state.mods = mods;
                    state.hide_deadline = Some(Instant::now() + std::time::Duration::from_millis(state.timeout_ms));

                    if state.keys.is_empty() && state.mods.is_empty() {
                        if let (Some(ref layer), Some(ref mut pool)) = (&state.layer, &mut state.pool) {
                            if let Ok((buffer, canvas)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
                                canvas[0..4].copy_from_slice(&[0, 0, 0, 0]);
                                if buffer.attach_to(layer.wl_surface()).is_ok() {
                                    layer.wl_surface().damage_buffer(0, 0, 1, 1);
                                    layer.commit();
                                }
                            }
                        }
                        continue;
                    }

                    let (pixels, rw, rh) = render_keystroke_pixels(
                        &state.keys, &state.mods, &typeface,
                        state.font_size, state.bg_color, state.text_color,
                    );

                    if let (Some(ref layer), Some(ref mut pool)) = (&state.layer, &mut state.pool) {
                        let w = rw.max(1);
                        let h = rh.max(1);
                        let stride = (w * 4) as i32;
                        let needed = (stride * h as i32) as usize;
                        if needed > pool.len() {
                            let new_size = needed.next_power_of_two().max(1024);
                            if pool.resize(new_size).is_err() {
                                log::error!("[KSO] SHM pool resize failed");
                                continue;
                            }
                        }
                        if let Ok((buffer, canvas)) = pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888) {
                            canvas.copy_from_slice(&pixels);
                            layer.set_size(w, h);
                            if buffer.attach_to(layer.wl_surface()).is_ok() {
                                layer.wl_surface().damage_buffer(0, 0, w as i32, h as i32);
                                layer.commit();
                            }
                        }
                    }
                }
                Ok(KSCmd::Exit) => return,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }

        if let Some(deadline) = state.hide_deadline {
            if Instant::now() >= deadline {
                state.keys.clear();
                state.mods.clear();
                state.hide_deadline = None;
                if let (Some(ref layer), Some(ref mut pool)) = (&state.layer, &mut state.pool) {
                    if let Ok((buffer, canvas)) = pool.create_buffer(1, 1, 4, wl_shm::Format::Argb8888) {
                        canvas[0..4].copy_from_slice(&[0, 0, 0, 0]);
                        if buffer.attach_to(layer.wl_surface()).is_ok() {
                            layer.set_size(1, 1);
                            layer.wl_surface().damage_buffer(0, 0, 1, 1);
                            layer.commit();
                        }
                    }
                }
            }
        }

        if state.exit.load(Ordering::SeqCst) {
            break;
        }

        if event_queue.flush().is_err() { break; }
        if event_queue.dispatch_pending(&mut state).is_err() { break; }

        if let Some(read_guard) = event_queue.prepare_read() {
            let fd = read_guard.connection_fd().as_raw_fd();
            let mut fds = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }];
            if unsafe { libc::poll(fds.as_mut_ptr(), 1, 100) } > 0 {
                if read_guard.read().is_err() { break; }
                if event_queue.dispatch_pending(&mut state).is_err() { break; }
            }
        }
    }
}

// ── KeystrokeOverlay ──

pub struct KeystrokeOverlay {
    wl_tx: Option<mpsc::Sender<KSCmd>>,
    wl_join: Option<std::thread::JoinHandle<()>>,
}

impl KeystrokeOverlay {
    pub fn new(config: &Config) -> Option<Self> {
        let timeout_ms = config.linux.keystroke_timeout_ms;
        let position = parse_position(&config.linux.keystroke_position);
        let font_size = config.linux.keystroke_font_size as f32;
        let bg_color = parse_color(&config.linux.keystroke_bg_color);
        let text_color = parse_color(&config.linux.keystroke_text_color);

        let (tx, rx) = mpsc::channel();
        let join = std::thread::Builder::new()
            .name("keystroke-wl".into())
            .spawn(move || wl_keystroke_thread(rx, timeout_ms, position, font_size, bg_color, text_color))
            .ok()?;

        Some(Self {
            wl_tx: Some(tx),
            wl_join: Some(join),
        })
    }

    pub fn update_keys(&mut self, keys: &[String], modifiers: &[String]) {
        if let Some(ref tx) = self.wl_tx {
            let _ = tx.send(KSCmd::Show {
                keys: keys.to_vec(),
                mods: modifiers.to_vec(),
            });
        }
    }
}

impl Drop for KeystrokeOverlay {
    fn drop(&mut self) {
        if let Some(ref tx) = self.wl_tx {
            let _ = tx.send(KSCmd::Exit);
        }
        if let Some(join) = self.wl_join.take() {
            let _ = join.join();
        }
    }
}
