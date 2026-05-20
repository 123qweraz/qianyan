use crate::linux_notify::LinuxNotifyDisplay;
use crate::slint_window::SlintDisplay;
use crate::tray::TrayEvent;
use crate::{CandidateDisplay, GuiEvent};
use qianyan_ime_core::Config;
use std::sync::mpsc::{Receiver, Sender};

pub fn start_gui(rx: Receiver<GuiEvent>, mut config: Config, _tray_tx: Sender<TrayEvent>) {
    let mut display: Box<dyn CandidateDisplay> = create_display(&config);

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(5),
        move || {
            while let Ok(event) = rx.try_recv() {
                handle_single_event(&mut display, event, &mut config);
            }
        },
    );

    slint::run_event_loop().unwrap();
}

fn create_display(config: &Config) -> Box<dyn CandidateDisplay> {
    if cfg!(target_os = "linux") {
        if config.linux.enable_notification_candidates {
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
    config: &mut Config,
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
            let old_notify = config.linux.enable_notification_candidates;
            *config = *new_config.clone();
            if config.linux.enable_notification_candidates != old_notify {
                display.close();
                *display = create_display(config);
            } else {
                display.apply_config(config);
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
            slint::quit_event_loop().unwrap();
        }
        _ => {}
    }
}
