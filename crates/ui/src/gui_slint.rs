use crate::linux_notify::LinuxNotifyDisplay;
use crate::slint_window::SlintDisplay;
use crate::tray::TrayEvent;
use crate::{CandidateDisplay, GuiEvent};
use qianyan_ime_core::Config;
use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::panic;

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

fn create_displays(config: &Config) -> Vec<Box<dyn CandidateDisplay>> {
    let mut displays: Vec<Box<dyn CandidateDisplay>> = Vec::new();
    eprintln!("[GUI_DEBUG] create_displays: show_slint={} show_notify={}", config.linux.show_slint_window, config.linux.show_notification);
    displays.push(Box::new(SlintDisplay::new(config.clone())));
    if cfg!(target_os = "linux") && config.linux.show_notification {
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
        GuiEvent::Update { pinyin, candidates, selected, .. } => {
            for d in displays.iter_mut() {
                d.update_candidates(&pinyin, candidates.clone(), selected);
            }
        }
        GuiEvent::SyncState(state) => {
            for d in displays.iter_mut() {
                d.update_status(&state.status_text, state.chinese_enabled);
                d.update_candidates(&state.pinyin, state.candidates.clone(), state.selected_index);
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
            let new = &*new_config;
            let new_slint = new.linux.show_slint_window;
            let new_notify = new.linux.show_notification;
            drop(old);
            eprintln!("[GUI_DEBUG] ApplyConfig: old(slint={},notify={}) new(slint={},notify={})",
                old_slint, old_notify, new_slint, new_notify);
            *config.write().expect("config lock poisoned") = *new_config;

            if new_slint != old_slint || new_notify != old_notify {
                eprintln!("[GUI_DEBUG] ApplyConfig: display config changed, recreating");
                for d in displays.iter_mut() {
                    d.close();
                }
                displays.clear();
                *displays = create_displays(&config.read().expect("config lock poisoned"));
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
                d.update_status("", visible);
            }
        }
        GuiEvent::ForceStatusVisible(visible) => {
            for d in displays.iter_mut() {
                d.update_status("", visible);
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
