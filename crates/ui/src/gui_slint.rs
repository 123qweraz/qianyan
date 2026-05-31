use crate::ipc::transport::{self, MainToGui, GuiToMain};
use crate::linux_notify::LinuxNotifyDisplay;
use crate::slint_window::SlintDisplay;
use crate::tray::TrayEvent;
use crate::{CandidateDisplay, GuiEvent};
use qianyan_ime_core::Config;
use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::panic;
use std::time::Duration;

thread_local! {
    static DISPLAYS: RefCell<Vec<Box<dyn CandidateDisplay>>> = const { RefCell::new(Vec::new()) };
}

pub fn start_gui(
    rx: Receiver<GuiEvent>,
    config: Arc<RwLock<Config>>,
    _tray_tx: Sender<TrayEvent>,
) {
    {
        let cfg = config.read().expect("config lock poisoned");
        let initial = create_displays(&cfg);
        DISPLAYS.with(|d| {
            *d.borrow_mut() = initial;
        });
    }

    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            let cfg = config.clone();
            let event_type = match &event {
                GuiEvent::SyncState(_) => "SyncState",
                GuiEvent::ForceStatusVisible(_) => "ForceStatusVisible",
                GuiEvent::Update{..} => "Update",
                GuiEvent::MoveTo{..} => "MoveTo",
                GuiEvent::ApplyConfig(_) => "ApplyConfig",
                GuiEvent::ShowStatus(..) => "ShowStatus",
                GuiEvent::UpdateStatusBarVisible(_) => "UpdateStatusBarVisible",
                GuiEvent::SetVisible(_) => "SetVisible",
                GuiEvent::OpenTrayMenu{..} => "OpenTrayMenu",
                GuiEvent::HideAndAck(..) => "HideAndAck",
                GuiEvent::Exit => "Exit",
            };
            let invoke_result = slint::invoke_from_event_loop(move || {
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    DISPLAYS.with(|d| {
                        let mut displays = d.borrow_mut();
                        if !displays.is_empty() {
                            handle_event(&mut *displays, event, &cfg);
                        }
                    });
                }));
                if let Err(e) = result {
                    log::error!("GUI event handler panicked: {:?}", e);
                }
            });
            if invoke_result.is_err() {
                eprintln!("invoke_from_event_loop FAILED for event {}: {:?}", event_type, invoke_result);
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
    let current_config = std::sync::Arc::new(std::sync::Mutex::new(config));

    // Notify main process we're ready
    let _ = transport::send_gui_to_main(&mut stream, &GuiToMain::Ready);

    let stream_ref = std::sync::Mutex::new(Some(stream));

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
                    eprintln!("[GUI IPC] connection closed");
                    break;
                }
                Err(e) => {
                    eprintln!("[GUI IPC] error: {e}");
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
                        eprintln!("[GUI IPC] event loop not running");
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

                _ => {
                    let cfg = current_config.clone();
                    let r = slint::invoke_from_event_loop(move || {
                        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                            let mut guard = cfg.lock().unwrap();
                            handle_ipc_event(&msg, &mut *guard);
                        }));
                        if let Err(e) = result {
                            log::error!("GUI event handler panicked: {:?}", e);
                        }
                    });
                    if r.is_err() {
                        eprintln!("[GUI IPC] event loop not running");
                        break;
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
            MainToGui::ForceStatusVisible(visible) => {
                for display in displays.iter_mut() {
                    display.set_status_bar_visible(*visible);
                }
            }
            MainToGui::ShowStatus(text, chinese_enabled) => {
                for display in displays.iter_mut() {
                    display.update_status(text, *chinese_enabled);
                }
            }
            MainToGui::UpdateStatusBarVisible(visible) => {
                for display in displays.iter_mut() {
                    display.set_status_bar_visible(*visible);
                }
            }
            MainToGui::ApplyConfig(json) => {
                if let Ok(new_config) = serde_json::from_str::<Config>(json) {
                    let old_slint = config.linux.show_slint_window;
                    let old_notify = config.linux.show_notification;
                    let old_toggle_notify = config.linux.show_toggle_notification;
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
                } else {
                    eprintln!("[GUI IPC] invalid ApplyConfig JSON");
                }
            }
            _ => {}
        }
    });
}

fn create_displays(config: &Config) -> Vec<Box<dyn CandidateDisplay>> {
    let mut displays: Vec<Box<dyn CandidateDisplay>> = Vec::new();
    eprintln!("[GUI_DEBUG] create_displays: show_slint={} show_notify={}", config.linux.show_slint_window, config.linux.show_notification);

    // On Wayland, use layer-shell overlay instead of Slint winit windows
    // to avoid taskbar icons. Never fall back to SlintDisplay on Wayland,
    // which would create real windows with taskbar icons.
    #[cfg(target_os = "linux")]
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        if let Some(wl_display) = crate::wayland_layer::WaylandLayerDisplay::new(config.clone()) {
            eprintln!("[GUI_DEBUG] Using WaylandLayerDisplay");
            displays.push(Box::new(wl_display));
        } else {
            eprintln!("[GUI_DEBUG] WaylandLayerDisplay init failed, no window display available");
        }
    } else {
        eprintln!("[GUI_DEBUG] No WAYLAND_DISPLAY, using SlintDisplay (X11)");
        displays.push(Box::new(SlintDisplay::new(config.clone())));
    }

    #[cfg(not(target_os = "linux"))]
    displays.push(Box::new(SlintDisplay::new(config.clone())));

    if cfg!(target_os = "linux") && (config.linux.show_notification || config.linux.show_toggle_notification) {
        eprintln!("[GUI_DEBUG] create_displays: adding LinuxNotifyDisplay");
        displays.push(Box::new(LinuxNotifyDisplay::new(config.clone())));
    }
    eprintln!("[GUI_DEBUG] create_displays: total {} displays", displays.len());
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
            eprintln!("[GUI_DEBUG] SetVisible({}), {} displays", visible, displays.len());
            for d in displays.iter_mut() {
                d.set_visible(visible);
            }
        }
        GuiEvent::ApplyConfig(new_config) => {
            let old = config.read().expect("config lock poisoned");
            let old_slint = old.linux.show_slint_window;
            let old_notify = old.linux.show_notification;
            let old_toggle_notify = old.linux.show_toggle_notification;
            let new = &*new_config;
            let new_slint = new.linux.show_slint_window;
            let new_notify = new.linux.show_notification;
            let new_toggle_notify = new.linux.show_toggle_notification;
            drop(old);
            eprintln!("[GUI_DEBUG] ApplyConfig: old(slint={},notify={},toggle_notify={}) new(slint={},notify={},toggle_notify={})",
                old_slint, old_notify, old_toggle_notify, new_slint, new_notify, new_toggle_notify);
            *config.write().expect("config lock poisoned") = *new_config;

            if new_slint != old_slint || new_notify != old_notify || new_toggle_notify != old_toggle_notify {
                eprintln!("[GUI_DEBUG] ApplyConfig: display config changed, recreating");
                let new_displays = create_displays(&config.read().expect("config lock poisoned"));
                for d in displays.iter_mut() {
                    d.close();
                }
                *displays = new_displays;
            } else {
                eprintln!("[GUI_DEBUG] ApplyConfig: config unchanged, applying to existing");
                let cfg = config.read().expect("config lock poisoned");
                for d in displays.iter_mut() {
                    d.apply_config(&cfg);
                }
            }
        }
        GuiEvent::UpdateStatusBarVisible(visible) => {
            for d in displays.iter_mut() {
                d.set_status_bar_visible(visible);
            }
        }
        GuiEvent::ForceStatusVisible(visible) => {
            for d in displays.iter_mut() {
                d.set_status_bar_visible(visible);
            }
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
