use crate::hosts::ibus_backend;
use crate::hosts::vkbd::Vkbd;
use crate::hosts::evdev_host;
use crate::hosts::wayland_host::WaylandInputHost;
use crate::hosts::wayland_host_v1::WaylandInputHostV1;
use qianyan_ime_core::Config;
use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::processor::actor::ProcessorHandle;
use qianyan_ime_ui::GuiEvent;
use qianyan_ime_ui::tray::TrayEvent;
use std::error::Error;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};

pub type InputHostResult = Result<(Option<Arc<Mutex<Vkbd>>>, Box<dyn FnOnce() + Send>), Box<dyn Error>>;

pub fn create_input_host(
    args: &[String],
    processor: ProcessorHandle,
    gui_tx: std::sync::mpsc::Sender<GuiEvent>,
    config: Arc<RwLock<Config>>,
    tray_tx: std::sync::mpsc::Sender<qianyan_ime_ui::tray::TrayEvent>,
    _app_state: Arc<Mutex<qianyan_ime_ui::AppState>>,
) -> InputHostResult {
    let linux_config = match config.read() {
        Ok(c) => c.linux.clone(),
        Err(e) => {
            log::warn!("[Runtime] Config lock poisoned, using fallback device path");
            e.into_inner().linux.clone()
        }
    };


    let dev_path = linux_config.device_path.clone();
    let backend = parse_backend(args, &linux_config.backend_type);

    // Try the primary backend, falling back through alternatives
    match backend {
        BackendType::Wayland => {
            // Try Wayland IM protocol first
            match try_start_wayland(processor.clone(), gui_tx.clone(), tray_tx.clone()) {
                Ok(result) => return Ok(result),
                Err(e) => println!("[Main] Wayland 不可用 ({}), 尝试 Evdev...", e),
            }
            // Fallback to Evdev
            match try_start_evdev(processor.clone(), &dev_path, Some(gui_tx.clone()), tray_tx.clone()) {
                Ok(result) => return Ok(result),
                Err(e) => println!("[Main] Evdev 也不可用: {:?}", e),
            }
            // Last resort: try Wayland again (maybe a display issue)
            if std::env::var("WAYLAND_DISPLAY").is_ok() {
                match try_start_wayland(processor, gui_tx, tray_tx) {
                    Ok(result) => return Ok(result),
                    Err(e) => return Err(format!("所有后端均不可用: {}", e).into()),
                }
            }
            Err("所有后端均不可用".into())
        }
        BackendType::Evdev => {
            try_start_evdev(processor, &dev_path, Some(gui_tx), tray_tx)
                .map_err(|e| format!("Evdev 启动失败: {:?}", e).into())
        }
        BackendType::Auto => {
            // Default: evdev first (reliable, no compositor dependency)
            match try_start_evdev(processor.clone(), &dev_path, Some(gui_tx.clone()), tray_tx.clone()) {
                Ok(result) => return Ok(result),
                Err(evdev_err) => println!("[Main] Evdev 启动失败 ({:?})，尝试 Wayland。", evdev_err),
            }

            // Fallback: try Wayland input method protocol
            if std::env::var("WAYLAND_DISPLAY").is_ok() {
                match try_start_wayland(processor.clone(), gui_tx.clone(), tray_tx.clone()) {
                    Ok(result) => return Ok(result),
                    Err(e) => println!("[Main] Wayland 输入法不可用: {}", e),
                }
            }

            // Final fallback: try Evdev again with different device path
            Err("No input backend available".into())
        }
    }
}

fn try_start_evdev(
    processor: ProcessorHandle,
    dev_path: &str,
    gui_tx: Option<Sender<GuiEvent>>,
    tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
) -> Result<(Option<Arc<Mutex<Vkbd>>>, Box<dyn FnOnce() + Send>), Box<dyn Error>> {
    let mut host = evdev_host::EvdevHost::new(processor, dev_path, gui_tx, tray_tx)?;
    println!("[Main] 成功启动 Evdev 拦截模式。");
    let vkbd = host.vkbd.clone();
    Ok((Some(vkbd), Box::new(move || {
        let _ = host.run();
    })))
}

fn try_start_wayland(
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
) -> Result<(Option<Arc<Mutex<Vkbd>>>, Box<dyn FnOnce() + Send>), Box<dyn Error>> {
    let (mut host, vkbd_opt, desc) = create_wayland_host(processor, gui_tx, tray_tx)?;
    println!("[Main] 成功启动{}输入法模式。", desc);
    Ok((vkbd_opt, Box::new(move || {
        let _ = host.run();
    })))
}

fn create_wayland_host(
    processor: ProcessorHandle,
    gui_tx: std::sync::mpsc::Sender<GuiEvent>,
    tray_tx: std::sync::mpsc::Sender<TrayEvent>,
) -> Result<(Box<dyn InputMethodHost>, Option<Arc<Mutex<Vkbd>>>, &'static str), Box<dyn Error>> {
    // When launched by KWin as a virtual keyboard, WAYLAND_SOCKET is set.
    // On this private socket only zwp_input_method_v1 is available — skip the probe.
    let kwin_socket = crate::kwin::is_kwin_virtual_keyboard();
    if kwin_socket {
        log::info!("[Main] KWin Virtual Keyboard mode detected (WAYLAND_SOCKET)");
        let host = WaylandInputHostV1::new(processor, gui_tx, tray_tx)
            .ok_or("Wayland v1 host init failed (KWin mode)")?;
        let vkbd = host.vkbd();
        return Ok((Box::new(host), vkbd, " Wayland v1 (KWin)"));
    }

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

    let has_v2 = globals_list.iter().any(|g| g.interface == "zwp_input_method_manager_v2");
    let has_v1 = globals_list.iter().any(|g| g.interface == "zwp_input_method_v1");

    if has_v2 {
        let host = WaylandInputHost::new(processor.clone(), gui_tx.clone(), tray_tx.clone())
            .ok_or("Wayland v2 host init failed")?;
        let vkbd = host.vkbd();
        Ok((Box::new(host), vkbd, " Wayland v2"))
    } else if has_v1 {
        let host = WaylandInputHostV1::new(processor, gui_tx, tray_tx)
            .ok_or("Wayland v1 host init failed")?;
        let vkbd = host.vkbd();
        Ok((Box::new(host), vkbd, " Wayland v1"))
    } else {
        Err("No zwp_input_method (v1 or v2) global available".into())
    }
}

enum BackendType {
    Auto,
    Evdev,
    Wayland,
}

/// Start the IBus D-Bus backend in a background thread.
/// Should be called alongside the main input host.
pub fn start_ibus_backend(
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
) {
    ibus_backend::start_ibus_backend(processor, gui_tx, tray_tx);
}

fn parse_backend(args: &[String], config_backend: &str) -> BackendType {
    // CLI args take priority
    if args.iter().any(|a| a == "--backend=wayland" || a == "wayland") {
        BackendType::Wayland
    } else if args.iter().any(|a| a == "--backend=evdev" || a == "evdev") {
        BackendType::Evdev
    } else if args.iter().any(|a| a == "--backend=auto" || a == "auto") {
        BackendType::Auto
    } else {
        // Fall back to config value
        match config_backend {
            "wayland" => BackendType::Wayland,
            "evdev" => BackendType::Evdev,
            _ => BackendType::Auto,
        }
    }
}
