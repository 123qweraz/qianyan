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
    static DISPLAY: RefCell<Option<Box<dyn CandidateDisplay>>> = const { RefCell::new(None) };
}

pub fn start_gui(
    rx: Receiver<GuiEvent>,
    config: Arc<RwLock<Config>>,
    _tray_tx: Sender<TrayEvent>,
) {
    {
        let cfg = config.read().expect("config lock poisoned");
        let initial = create_display(&cfg);
        DISPLAY.with(|d| {
            *d.borrow_mut() = Some(initial);
        });
    }

    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            let cfg = config.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    DISPLAY.with(|d| {
                        let mut display = d.borrow_mut();
                        if let Some(ref mut display) = *display {
                            handle_single_event(display, event, &cfg);
                        }
                    });
                }));
                if let Err(e) = result {
                    log::error!("GUI event handler panicked: {:?}", e);
                }
            });
        }
    });

    slint::run_event_loop().expect("slint event loop failed");
}

fn create_display(config: &Config) -> Box<dyn CandidateDisplay> {
    if cfg!(target_os = "linux") {
        if config.linux.display_mode == "notification" {
            Box::new(LinuxNotifyDisplay::new(config.clone()))
        } else {
            Box::new(SlintDisplay::new(config.clone()))
        }
    } else {
        Box::new(SlintDisplay::new(config.clone()))
    }
}

fn handle_single_event(
    display: &mut Box<dyn CandidateDisplay>,
    event: GuiEvent,
    config: &Arc<RwLock<Config>>,
) {
    match event {
        GuiEvent::Update {
            pinyin,
            candidates,
            selected,
            ..
        } => {
            display.update_candidates(&pinyin, candidates, selected);
        }
        GuiEvent::SyncState(state) => {
            display.update_status(&state.status_text, state.chinese_enabled);
            display.update_candidates(&state.pinyin, state.candidates, state.selected_index);
        }

        GuiEvent::ShowStatus(text, chinese_enabled) => {
            display.update_status(&text, chinese_enabled);
        }
        GuiEvent::MoveTo { x, y } => {
            display.move_to(x, y);
        }
        GuiEvent::SetVisible(visible) => {
            display.set_visible(visible);
        }
        GuiEvent::ApplyConfig(new_config) => {
            let old_mode = config
                .read()
                .map(|g| g.linux.display_mode.clone())
                .unwrap_or_default();
            let new_mode = new_config.linux.display_mode.clone();
            *config.write().expect("config lock poisoned") = *new_config;
            if new_mode != old_mode {
                display.close();
                *display = create_display(&config.read().expect("config lock poisoned"));
            } else {
                display.apply_config(&config.read().expect("config lock poisoned"));
            }
        }
        GuiEvent::UpdateStatusBarVisible(visible) => {
            display.update_status("", visible);
        }
        GuiEvent::ForceStatusVisible(visible) => {
            display.update_status("", visible);
        }
        GuiEvent::HideAndAck(ack_tx) => {
            display.set_visible(false);
            let _ = ack_tx.send(());
        }
        GuiEvent::Exit => {
            display.close();
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
}
