use crate::hosts::vkbd::Vkbd;
use crate::hosts::evdev_host;
use crate::hosts::wayland_host::WaylandInputHost;
use crate::hosts::wayland_host_v1::WaylandInputHostV1;
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
            clipboard_delay_ms: 50,
            show_slint_window: true,
            show_notification: false,
            show_toggle_notification: false,
            fixed_position: true,
            corner: "bottom-right".into(),
            fixed_x: 40,
            fixed_y: 40,
        });

    let dev_path = linux_config.device_path.clone();
    let backend = parse_backend(args);

    match backend {
        BackendType::Wayland => {
            let (mut host, desc) = create_wayland_host(processor, gui_tx)?;
            println!("[Main] 成功启动{}输入法模式。", desc);
            Ok((
                None,
                Box::new(move || {
                    let _ = host.run();
                }),
            ))
        }
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
            // Default: evdev first (reliable across X11/Wayland, no compositor dependency)
            match evdev_host::EvdevHost::new(
                processor.clone(),
                &dev_path,
                Some(gui_tx.clone()),
                tray_tx.clone(),
            ) {
                Ok(mut host) => {
                    println!("[Main] 成功启动 Evdev 拦截模式。");
                    let vkbd = host.vkbd.clone();
                    return Ok((
                        Some(vkbd),
                        Box::new(move || {
                            let _ = host.run();
                        }),
                    ));
                }
                Err(evdev_err) => {
                    println!("[Main] Evdev 启动失败 ({:?})，尝试 Wayland。", evdev_err);
                }
            }

            // Fallback: try Wayland input method protocol
            #[cfg(target_os = "linux")]
            if std::env::var("WAYLAND_DISPLAY").is_ok() {
                match create_wayland_host(processor.clone(), gui_tx.clone()) {
                    Ok((mut host, desc)) => {
                        println!("[Main] 成功启动{}输入法模式。", desc);
                        return Ok((
                            None,
                            Box::new(move || {
                                let _ = host.run();
                            }),
                        ));
                    }
                    Err(e) => {
                        println!("[Main] Wayland 输入法不可用: {}", e);
                    }
                }
            }

            Err("No input backend available".into())
        }
    }
}

fn create_wayland_host(
    processor: Arc<Mutex<Processor>>,
    gui_tx: std::sync::mpsc::Sender<GuiEvent>,
) -> Result<(Box<dyn InputMethodHost>, &'static str), Box<dyn Error>> {
    // Minimal state just to query globals; we don't need a real dispatch loop
    struct DummyState;
    impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for DummyState {
        fn event(
            _state: &mut Self,
            _: &wayland_client::protocol::wl_registry::WlRegistry,
            _event: <wayland_client::protocol::wl_registry::WlRegistry as wayland_client::Proxy>::Event,
            _data: &wayland_client::globals::GlobalListContents,
            _conn: &wayland_client::Connection,
            _qh: &wayland_client::QueueHandle<Self>,
        ) {
        }
    }

    let conn = wayland_client::Connection::connect_to_env()
        .map_err(|_| "Cannot connect to Wayland compositor")?;
    let (globals, _event_queue) = wayland_client::globals::registry_queue_init::<DummyState>(&conn)
        .map_err(|_| "Cannot initialize Wayland registry")?;

    let globals_list = globals.contents().clone_list();

    // Check for v2 (zwp_input_method_manager_v2)
    let has_v2 = globals_list.iter().any(|g| g.interface == "zwp_input_method_manager_v2");
    // Check for v1 (zwp_input_method_v1)
    let has_v1 = globals_list.iter().any(|g| g.interface == "zwp_input_method_v1");

    if has_v2 {
        Ok((
            Box::new(WaylandInputHost::new(processor, gui_tx)
                .ok_or("Wayland v2 host init failed")?),
            " Wayland v2",
        ))
    } else if has_v1 {
        Ok((
            Box::new(WaylandInputHostV1::new(processor, gui_tx)
                .ok_or("Wayland v1 host init failed")?),
            " Wayland v1",
        ))
    } else {
        Err("No zwp_input_method (v1 or v2) global available".into())
    }
}

enum BackendType {
    Auto,
    Evdev,
    Wayland,
}

fn parse_backend(args: &[String]) -> BackendType {
    if args.iter().any(|a| a == "--backend=wayland" || a == "wayland") {
        BackendType::Wayland
    } else if args.iter().any(|a| a == "--backend=evdev" || a == "evdev") {
        BackendType::Evdev
    } else {
        BackendType::Auto
    }
}
