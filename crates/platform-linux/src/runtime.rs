use crate::hosts::vkbd::Vkbd;
use crate::hosts::evdev_host;
use qianyan_ime_core::config::LinuxConfig;
use qianyan_ime_core::Config;
use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::Processor;
use qianyan_ime_ui::GuiEvent;
use std::error::Error;
use std::sync::{Arc, Mutex, RwLock};

pub type InputHostResult = Result<(Option<Arc<Mutex<Vkbd>>>, Box<dyn FnOnce() + Send>), Box<dyn Error>>;

pub fn create_input_host(
    args: &[String],
    processor: Arc<Mutex<Processor>>,
    gui_tx: std::sync::mpsc::Sender<GuiEvent>,
    config: Arc<RwLock<Config>>,
    tray_tx: std::sync::mpsc::Sender<qianyan_ime_ui::tray::TrayEvent>,
    _app_state: Arc<Mutex<qianyan_ime_ui::AppState>>,
) -> InputHostResult {
    let linux_config = config
        .read()
        .map(|c| c.linux.clone())
        .unwrap_or(LinuxConfig {
            device_path: "/dev/input/event4".into(),
            paste_method: "shift_insert".into(),
            display_mode: "slint".into(),
            fixed_position: true,
            corner: "bottom-right".into(),
            fixed_x: 40,
            fixed_y: 40,
        });

    let dev_path = linux_config.device_path.clone();
    let backend = parse_backend(args);

    match backend {
        BackendType::Evdev => {
            let mut host = evdev_host::EvdevHost::new(processor, &dev_path, Some(gui_tx), tray_tx)?;
            let vkbd = host.vkbd.clone();
            Ok((
                Some(vkbd),
                Box::new(move || {
                    let _ = host.run();
                }),
            ))
        }
        BackendType::Auto => {
            match evdev_host::EvdevHost::new(
                processor.clone(),
                &dev_path,
                Some(gui_tx.clone()),
                tray_tx.clone(),
            ) {
                Ok(mut host) => {
                    println!("[Main] 成功启动 Evdev 拦截模式。");
                    let vkbd = host.vkbd.clone();
                    Ok((
                        Some(vkbd),
                        Box::new(move || {
                            let _ = host.run();
                        }),
                    ))
                }
                Err(e) => {
                    println!("[Main] Evdev 启动失败 ({:?})，请检查设备路径。", e);
                    Err(e)
                }
            }
        }
    }
}

enum BackendType {
    Auto,
    Evdev,
}

fn parse_backend(args: &[String]) -> BackendType {
    if args.iter().any(|a| a == "--backend=evdev" || a == "evdev") {
        BackendType::Evdev
    } else {
        BackendType::Auto
    }
}
