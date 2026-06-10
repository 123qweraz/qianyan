use crate::ipc::transport::{self, MainToGui, GuiToMain};
use crate::keystroke_overlay::KeystrokeOverlay;
use crate::slint_window::SlintDisplay;
use crate::tray::TrayEvent;
use crate::{CandidateDisplay, GuiEvent};
use qianyan_ime_core::Config;
use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::panic;
use std::time::Duration;

thread_local! {
    static DISPLAYS: RefCell<Vec<Box<dyn CandidateDisplay>>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    static KEYSTROKE: RefCell<Option<KeystrokeOverlay>> = const { RefCell::new(None) };
}

pub fn start_gui(
    rx: Receiver<GuiEvent>,
    config: Arc<RwLock<Config>>,
    _tray_tx: Sender<TrayEvent>,
) {
    {
        let cfg = config.read().unwrap_or_else(|e| e.into_inner());
        let initial = create_displays(&cfg);
        DISPLAYS.with(|d| {
            *d.borrow_mut() = initial;
        });
        if cfg.linux.keystroke_enabled {
            KEYSTROKE.with(|k| {
                *k.borrow_mut() = KeystrokeOverlay::new(&cfg);
            });
        }
    }

    // Coalesce rapid events (SyncState, Update, MoveTo, etc.) to avoid
    // redundant per-keystroke renders in the single-process path.
    let coalesced_event = Arc::new(Mutex::new(None::<GuiEvent>));
    let pending_update = Arc::new(AtomicBool::new(false));

    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            match event {
                GuiEvent::HideAndAck(tx) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        DISPLAYS.with(|d| {
                            for display in d.borrow_mut().iter_mut() {
                                display.set_visible(false);
                            }
                        });
                    });
                    let _ = tx.send(());
                }
                GuiEvent::Exit => {
                    let r = slint::invoke_from_event_loop(|| {
                        DISPLAYS.with(|d| {
                            for display in d.borrow_mut().iter_mut() {
                                display.set_visible(false);
                            }
                        });
                    });
                    if r.is_err() { break; }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    break;
                }
                GuiEvent::ApplyConfig(_) => {
                    let r = slint::invoke_from_event_loop(move || {
                        if let GuiEvent::ApplyConfig(cfg) = event {
                            DISPLAYS.with(|d| {
                                for display in d.borrow_mut().iter_mut() {
                                    display.apply_config(&cfg);
                                }
                            });
                        }
                    });
                    if r.is_err() { break; }
                }
                // Coalesceable: rapid typing events — only latest matters
                _ => {
                    *coalesced_event.lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(event);
                    if !pending_update.swap(true, Ordering::SeqCst) {
                        let cfg = config.clone();
                        let c_evt = coalesced_event.clone();
                        let p_upd = pending_update.clone();
                        let r = slint::invoke_from_event_loop(move || {
                            p_upd.store(false, Ordering::SeqCst);
                            let evt = c_evt.lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .take();
                            if let Some(e) = evt {
                                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                                    DISPLAYS.with(|d| {
                                        let mut displays = d.borrow_mut();
                                        if !displays.is_empty() {
                                            handle_event(&mut displays, e, &cfg);
                                        }
                                    });
                                }));
                                if let Err(err) = result {
                                    log::error!("GUI coalesced handler panicked: {:?}", err);
                                }
                            }
                        });
                        if r.is_err() { break; }
                    }
                }
            }
        }
    });

    slint::run_event_loop().expect("slint event loop failed");
}

/// Start the GUI with events coming from a Unix socket (IPC with main process).
/// The socket should already be connected; the first message must be
/// MainToGui::ApplyConfig(json) carrying the initial config.
pub fn start_gui_ipc(mut stream: UnixStream, config: Config) {
    let displays = create_displays(&config);
    DISPLAYS.with(|d| {
        *d.borrow_mut() = displays;
    });
    if config.linux.keystroke_enabled {
        KEYSTROKE.with(|k| {
            *k.borrow_mut() = KeystrokeOverlay::new(&config);
        });
    }
    let current_config = std::sync::Arc::new(std::sync::Mutex::new(config));

    // Notify main process we're ready
    let _ = transport::send_gui_to_main(&mut stream, &GuiToMain::Ready);

    let stream_ref = std::sync::Mutex::new(Some(stream));
    let coalesced_msg = std::sync::Arc::new(std::sync::Mutex::new(None::<MainToGui>));
    let pending_update = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    std::thread::spawn(move || {
        // Take ownership of the stream for the IPC thread
        let mut stream = match stream_ref.lock() {
            Ok(mut s) => s.take().expect("stream taken twice"),
            Err(_) => return,
        };

        loop {
            let msg = match transport::recv_main_to_gui(&mut stream) {
                Ok(Some(msg)) => msg,
                Ok(None) => {
                    log::warn!("[GUI IPC] connection closed");
                    break;
                }
                Err(e) => {
                    log::warn!("[GUI IPC] error: {e}");
                    break;
                }
            };

            match msg {
                MainToGui::HideCandidate => {
                    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
                    let r = slint::invoke_from_event_loop(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            DISPLAYS.with(|d| {
                                for display in d.borrow_mut().iter_mut() {
                                    display.set_visible(false);
                                }
                            });
                        }));
                        if let Err(e) = result {
                            log::error!("HideCandidate callback panicked: {:?}", e);
                        }
                        let _ = ack_tx.send(());
                    });
                    if r.is_err() {
                        log::warn!("[GUI IPC] event loop not running");
                        break;
                    }
                    let _ = ack_rx.recv_timeout(Duration::from_millis(100));
                    let _ = transport::send_gui_to_main(&mut stream, &GuiToMain::Ack);
                }

                MainToGui::Exit => {
                    let _ = slint::invoke_from_event_loop(|| {
                        DISPLAYS.with(|d| {
                            for display in d.borrow_mut().iter_mut() {
                                display.close();
                            }
                        });
                        let _ = slint::quit_event_loop();
                    });
                    break;
                }

                MainToGui::ApplyConfig(_) => {
                    // ApplyConfig should not be coalesced as it may recreate displays
                    let cfg = current_config.clone();
                    let r = slint::invoke_from_event_loop(move || {
                        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                                    let mut guard = cfg.lock()
                                        .unwrap_or_else(|e| e.into_inner());
                            handle_ipc_event(&msg, &mut guard);
                        }));
                        if let Err(e) = result {
                            log::error!("GUI ApplyConfig handler panicked: {:?}", e);
                        }
                    });
                    if r.is_err() { break; }
                }

                _ => {
                    // Coalesce frequent updates (SyncState, Update, MoveTo, etc.)
                    // only the latest one in the queue matters for an IME UI.
                    *coalesced_msg.lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(msg);
                    if !pending_update.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        let cfg = current_config.clone();
                        let c_msg = coalesced_msg.clone();
                        let p_upd = pending_update.clone();
                        let r = slint::invoke_from_event_loop(move || {
                            p_upd.store(false, std::sync::atomic::Ordering::SeqCst);
                            let msg_to_process = c_msg.lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .take();
                            if let Some(m) = msg_to_process {
                                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                            let mut guard = cfg.lock()
                                .unwrap_or_else(|e| e.into_inner());
                                    handle_ipc_event(&m, &mut guard);
                                }));
                                if let Err(e) = result {
                                    log::error!("GUI coalesced event handler panicked: {:?}", e);
                                }
                            }
                        });
                        if r.is_err() {
                            log::warn!("[GUI IPC] event loop not running");
                            break;
                        }
                    }
                }
            }
        }
    });

    slint::run_event_loop().expect("slint event loop failed");
}

fn handle_ipc_event(msg: &MainToGui, config: &mut Config) {
    DISPLAYS.with(|d| {
        let mut displays = d.borrow_mut();
        if displays.is_empty() {
            return;
        }
        match msg {
            MainToGui::SyncState(state) => {
                for display in displays.iter_mut() {
                    display.update_status(&state.status_text, state.chinese_enabled);
                    let candidates: Vec<_> = state.candidates.iter().map(|c| {
                        let full_display = if c.is_fuzzy {
                            format!("{}.{}(模糊)", c.label, c.text)
                        } else {
                            format!("{}.{}({})", c.label, c.text, c.hint)
                        };
                        crate::DisplayCandidate {
                            text: c.text.clone(),
                            label: c.label.clone(),
                            hint: c.hint.clone(),
                            full_display,
                            is_fuzzy: c.is_fuzzy,
                        }
                    }).collect();
                    display.update_candidates(&state.pinyin, candidates, state.selected_index, state.page, state.total_pages);
                }
            }
            MainToGui::Update { pinyin, candidates, selected, page, total_pages } => {
                let cands: Vec<_> = candidates.iter().map(|c| {
                    let full_display = if c.is_fuzzy {
                        format!("{}.{}(模糊)", c.label, c.text)
                    } else {
                        format!("{}.{}({})", c.label, c.text, c.hint)
                    };
                    crate::DisplayCandidate {
                        text: c.text.clone(),
                        label: c.label.clone(),
                        hint: c.hint.clone(),
                        full_display,
                        is_fuzzy: c.is_fuzzy,
                    }
                }).collect();
                for display in displays.iter_mut() {
                    display.update_candidates(pinyin, cands.clone(), *selected, *page, *total_pages);
                }
            }
            MainToGui::MoveTo { x, y } => {
                for display in displays.iter_mut() {
                    display.move_to(*x, *y);
                }
            }
            MainToGui::SetVisible(visible) => {
                for display in displays.iter_mut() {
                    display.set_visible(*visible);
                }
            }
            MainToGui::ShowStatus(text, chinese_enabled) => {
                for display in displays.iter_mut() {
                    display.update_status(text, *chinese_enabled);
                }
            }
            MainToGui::ApplyConfig(json) => {
                if let Ok(new_config) = serde_json::from_str::<Config>(json) {
                    let old_slint = config.linux.show_slint_window;
                    let old_notify = config.linux.show_notification;
                    let old_toggle_notify = config.linux.show_toggle_notification;
                    let ks_changed = config.linux.keystroke_enabled != new_config.linux.keystroke_enabled
                        || config.linux.keystroke_position != new_config.linux.keystroke_position
                        || config.linux.keystroke_timeout_ms != new_config.linux.keystroke_timeout_ms
                        || config.linux.keystroke_font_size != new_config.linux.keystroke_font_size
                        || config.linux.keystroke_bg_color != new_config.linux.keystroke_bg_color
                        || config.linux.keystroke_text_color != new_config.linux.keystroke_text_color;
                    let new_slint = new_config.linux.show_slint_window;
                    let new_notify = new_config.linux.show_notification;
                    let new_toggle_notify = new_config.linux.show_toggle_notification;
                    *config = new_config;

                    if new_slint != old_slint || new_notify != old_notify || new_toggle_notify != old_toggle_notify {
                        let new_displays = create_displays(config);
                        for display in displays.iter_mut() {
                            display.close();
                        }
                        *displays = new_displays;
                    } else {
                        for display in displays.iter_mut() {
                            display.apply_config(config);
                        }
                    }

                    if ks_changed {
                        KEYSTROKE.with(|k| {
                            *k.borrow_mut() = if config.linux.keystroke_enabled {
                                KeystrokeOverlay::new(config)
                            } else {
                                None
                            };
                        });
                    }
                } else {
                    log::warn!("[GUI IPC] invalid ApplyConfig JSON");
                }
            }
            MainToGui::KeyEvent { keys, modifiers } => {
                KEYSTROKE.with(|k| {
                    if let Some(ref mut ko) = *k.borrow_mut() {
                        ko.update_keys(keys, modifiers);
                    }
                });
            }
            _ => {}
        }
    });
}

fn create_displays(config: &Config) -> Vec<Box<dyn CandidateDisplay>> {
    let mut displays: Vec<Box<dyn CandidateDisplay>> = Vec::new();
    log::debug!("[GUI_DEBUG] create_displays: show_slint={} show_notify={}", config.linux.show_slint_window, config.linux.show_notification);

    // On Wayland, use layer-shell overlay instead of Slint winit windows
    // to avoid taskbar icons. Fall back to SlintDisplay if layer shell is
    // unavailable (e.g. compositor like niri doesn't support zwlr_layer_shell_v1).
    #[cfg(target_os = "linux")]
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        if let Some(wl_display) = crate::wayland_layer::WaylandLayerDisplay::new(config.clone()) {
            log::debug!("[GUI_DEBUG] Using WaylandLayerDisplay");
            displays.push(Box::new(wl_display));
        } else {
            log::warn!("WaylandLayerDisplay failed, falling back to Slint window (XWayland)");
            displays.push(Box::new(SlintDisplay::new(config.clone())));
        }
    } else {
        log::debug!("[GUI_DEBUG] No WAYLAND_DISPLAY, using SlintDisplay (X11)");
        displays.push(Box::new(SlintDisplay::new(config.clone())));
    }

    #[cfg(not(target_os = "linux"))]
    displays.push(Box::new(SlintDisplay::new(config.clone())));

    log::debug!("[GUI_DEBUG] create_displays: total {} displays (notifications moved to main process)", displays.len());
    displays
}

fn handle_event(
    displays: &mut Vec<Box<dyn CandidateDisplay>>,
    event: GuiEvent,
    config: &Arc<RwLock<Config>>,
) {
    match event {
        GuiEvent::Update { pinyin, candidates, selected, page, total_pages, .. } => {
            for d in displays.iter_mut() {
                d.update_candidates(&pinyin, candidates.clone(), selected, page, total_pages);
            }
        }
        GuiEvent::SyncState(state) => {
            for d in displays.iter_mut() {
                d.update_status(&state.status_text, state.chinese_enabled);
                d.update_candidates(&state.pinyin, state.candidates.clone(), state.selected_index, state.page, state.total_pages);
            }
        }
        GuiEvent::ShowStatus(text, chinese_enabled) => {
            for d in displays.iter_mut() {
                d.update_status(&text, chinese_enabled);
            }
        }
        GuiEvent::MoveTo { x, y } => {
            for d in displays.iter_mut() {
                d.move_to(x, y);
            }
        }
        GuiEvent::SetVisible(visible) => {
            log::debug!("[GUI_DEBUG] SetVisible({}), {} displays", visible, displays.len());
            for d in displays.iter_mut() {
                d.set_visible(visible);
            }
        }
        GuiEvent::ApplyConfig(new_config) => {
            let old = config.read().unwrap_or_else(|e| e.into_inner());
            let old_slint = old.linux.show_slint_window;
            let old_notify = old.linux.show_notification;
            let old_toggle_notify = old.linux.show_toggle_notification;
            let ks_changed = old.linux.keystroke_enabled != new_config.linux.keystroke_enabled
                || old.linux.keystroke_position != new_config.linux.keystroke_position
                || old.linux.keystroke_timeout_ms != new_config.linux.keystroke_timeout_ms
                || old.linux.keystroke_font_size != new_config.linux.keystroke_font_size
                || old.linux.keystroke_bg_color != new_config.linux.keystroke_bg_color
                || old.linux.keystroke_text_color != new_config.linux.keystroke_text_color;
            let new_slint = new_config.linux.show_slint_window;
            let new_notify = new_config.linux.show_notification;
            let new_toggle_notify = new_config.linux.show_toggle_notification;
            drop(old);
            *config.write().unwrap_or_else(|e| e.into_inner()) = *new_config;

            if new_slint != old_slint || new_notify != old_notify || new_toggle_notify != old_toggle_notify {
                log::debug!("[GUI_DEBUG] ApplyConfig: display config changed, recreating");
                let new_displays = create_displays(&config.read().unwrap_or_else(|e| e.into_inner()));
                for d in displays.iter_mut() {
                    d.close();
                }
                *displays = new_displays;
            } else {
                log::debug!("[GUI_DEBUG] ApplyConfig: config unchanged, applying to existing");
                let cfg = config.read().unwrap_or_else(|e| e.into_inner());
                for d in displays.iter_mut() {
                    d.apply_config(&cfg);
                }
            }

            if ks_changed {
                let cfg = config.read().unwrap_or_else(|e| e.into_inner());
                KEYSTROKE.with(|k| {
                    *k.borrow_mut() = if cfg.linux.keystroke_enabled {
                        KeystrokeOverlay::new(&cfg)
                    } else {
                        None
                    };
                });
            }
        }
        GuiEvent::KeyEvent { keys, modifiers } => {
            KEYSTROKE.with(|k| {
                if let Some(ref mut ko) = *k.borrow_mut() {
                    ko.update_keys(&keys, &modifiers);
                }
            });
        }
        GuiEvent::HideAndAck(ack_tx) => {
            for d in displays.iter_mut() {
                d.set_visible(false);
            }
            let _ = ack_tx.send(());
        }
        GuiEvent::Exit => {
            for d in displays.iter_mut() {
                d.close();
            }
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
}
