use crate::hosts::ibus_backend;
use crate::hosts::vkbd::Vkbd;
use crate::hosts::evdev_host;
use crate::hosts::wayland_host::WaylandInputHost;
use crate::hosts::wayland_host_v1::WaylandInputHostV1;
use evdev::Device;
use qianyan_ime_core::Config;
use qianyan_ime_core::InputMethodHost;
use qianyan_ime_engine::processor::actor::ProcessorHandle;
use qianyan_ime_ui::GuiEvent;
use qianyan_ime_ui::tray::TrayEvent;
use std::error::Error;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

/// Minimal state used only to enumerate the registry globals of a connection
/// before deciding which input-method protocol version to speak.
struct ProbeState;

impl Dispatch<WlRegistry, GlobalListContents> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

/// Connect to the ambient Wayland socket (honoring `WAYLAND_SOCKET`, i.e. the
/// private socket KWin hands to its input method) and report which
/// input-method protocol versions the compositor advertises.
///
/// The returned `Connection` must be reused by the host: `WAYLAND_SOCKET` is a
/// connected fd that can only be consumed once.
fn connect_and_probe() -> Result<(Connection, bool, bool), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, _queue) =
        wayland_client::globals::registry_queue_init::<ProbeState>(&conn)?;
    let mut has_v1 = false;
    let mut has_v2 = false;
    globals.contents().with_list(|list| {
        for g in list {
            log::debug!("[Main] wayland global: {} v{}", g.interface, g.version);
            match g.interface.as_str() {
                "zwp_input_method_v1" => has_v1 = true,
                "zwp_input_method_manager_v2" => has_v2 = true,
                _ => {}
            }
        }
    });
    Ok((conn, has_v1, has_v2))
}

/// 选中的输入后端的运行句柄。
pub struct InputHost {
    pub vkbd: Option<Arc<Mutex<Vkbd>>>,
    pub run: Box<dyn FnOnce() + Send>,
    /// 宿主是否已在层表面自绘候选框（Wayland v1/v2）。为 false 时（evdev/none）
    /// 由主进程的 `wayland_overlay` 或 GUI 子进程负责候选框。
    pub self_draws_candidates: bool,
}

pub type InputHostResult = Result<InputHost, Box<dyn Error>>;

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
        BackendType::Wayland => start_prefer_wayland(processor, &dev_path, gui_tx, tray_tx, config),
        BackendType::Evdev => {
            try_start_evdev(processor, &dev_path, Some(gui_tx), tray_tx)
                .map_err(|e| format!("Evdev 启动失败: {:?}", e).into())
        }
        BackendType::Auto => {
            // 桌面感知自动选择：
            // - KDE 会话 → 优先 Wayland（KWin 6 仅提供 input-method v1，经
            //   WAYLAND_SOCKET 由 KWin 虚拟键盘拉起）
            // - 其余桌面（COSMIC/GNOME/Hyprland 等）→ 优先 evdev 拦截：evdev
            //   不注册 input-method（input-method-v2 keyboard grab 会被合成器
            //   拒绝应用 xdg_popup.grab，导致右键/下拉菜单失效，见 cosmic-comp#1763）
            if crate::kwin::is_kde_session() {
                println!("[Main] 检测到 KDE 会话，优先 Wayland 输入法...");
                start_prefer_wayland(processor, &dev_path, gui_tx, tray_tx, config)
            } else {
                println!("[Main] 检测到非 KDE 桌面，优先 evdev 拦截...");
                start_prefer_evdev(processor, &dev_path, gui_tx, tray_tx, config)
            }
        }
    }
}

/// 优先 Wayland（v1/v2）输入法协议，失败回退 evdev。
fn start_prefer_wayland(
    processor: ProcessorHandle,
    dev_path: &str,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    config: Arc<RwLock<Config>>,
) -> InputHostResult {
    // Try Wayland IM protocol first
    match try_start_wayland(processor.clone(), gui_tx.clone(), tray_tx.clone(), config.clone()) {
        Ok(result) => return Ok(result),
        Err(e) => println!("[Main] Wayland 不可用 ({}), 尝试 Evdev...", e),
    }
    // Fallback to Evdev
    match try_start_evdev(processor.clone(), dev_path, Some(gui_tx.clone()), tray_tx.clone()) {
        Ok(result) => return Ok(result),
        Err(e) => println!("[Main] Evdev 也不可用: {:?}", e),
    }
    // Last resort: try Wayland again (maybe a display issue)
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        match try_start_wayland(processor, gui_tx, tray_tx, config) {
            Ok(result) => return Ok(result),
            Err(e) => return Err(format!("所有后端均不可用: {}", e).into()),
        }
    }
    Err("所有后端均不可用".into())
}

/// 优先 evdev 硬件拦截（不依赖合成器 input-method），失败回退 Wayland。
fn start_prefer_evdev(
    processor: ProcessorHandle,
    dev_path: &str,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    config: Arc<RwLock<Config>>,
) -> InputHostResult {
    // Default: evdev first (reliable, no compositor dependency)
    match try_start_evdev(processor.clone(), dev_path, Some(gui_tx.clone()), tray_tx.clone()) {
        Ok(result) => return Ok(result),
        Err(evdev_err) => println!("[Main] Evdev 启动失败 ({:?})，尝试 Wayland。", evdev_err),
    }

    // Fallback: try Wayland input method protocol
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        match try_start_wayland(processor.clone(), gui_tx.clone(), tray_tx.clone(), config) {
            Ok(result) => return Ok(result),
            Err(e) => println!("[Main] Wayland 输入法不可用: {}", e),
        }
    }

    // Final fallback: try Evdev again with different device path
    Err("No input backend available".into())
}

/// evdev 设备路径解析：优先用配置的设备；打不开时自动扫描探测物理键盘。
fn resolve_evdev_device_path(configured: &str) -> String {
    if !configured.is_empty() && Device::open(configured).is_ok() {
        return configured.to_string();
    }
    match evdev_host::find_keyboard_device() {
        Some(p) => {
            println!("[Main] 配置的 evdev 设备不可用，自动探测到物理键盘: {p}");
            p
        }
        None => configured.to_string(),
    }
}

fn try_start_evdev(
    processor: ProcessorHandle,
    dev_path: &str,
    gui_tx: Option<Sender<GuiEvent>>,
    tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
) -> Result<InputHost, Box<dyn Error>> {
    let dev_path = resolve_evdev_device_path(dev_path);
    let mut host = evdev_host::EvdevHost::new(processor, &dev_path, gui_tx, tray_tx)?;
    println!("[Main] 成功启动 Evdev 拦截模式(设备 {dev_path})。");
    let vkbd = host.vkbd.clone();
    Ok(InputHost {
        vkbd: Some(vkbd),
        self_draws_candidates: false,
        run: Box::new(move || {
            let _ = host.run();
        }),
    })
}

fn try_start_wayland(
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
    config: Arc<RwLock<Config>>,
) -> Result<InputHost, Box<dyn Error>> {
    let (mut host, vkbd_opt, desc) = create_wayland_host(processor, gui_tx, tray_tx, config)?;
    println!("[Main] 成功启动{}输入法模式。", desc);
    Ok(InputHost {
        vkbd: vkbd_opt,
        self_draws_candidates: true,
        run: Box::new(move || {
            let _ = host.run();
        }),
    })
}

fn create_wayland_host(
    processor: ProcessorHandle,
    gui_tx: std::sync::mpsc::Sender<GuiEvent>,
    tray_tx: std::sync::mpsc::Sender<TrayEvent>,
    config: Arc<RwLock<Config>>,
) -> Result<(Box<dyn InputMethodHost>, Option<Arc<Mutex<Vkbd>>>, &'static str), Box<dyn Error>> {
    // When launched by KWin as a virtual keyboard, WAYLAND_SOCKET is set and
    // points at a private socket exposing the input-method protocol. KWin
    // (through 6.7) only implements input-method v1; v2 (with popup-surface
    // cursor follow) is found on wlroots-based compositors and others, where
    // the IME connects to the regular Wayland display.
    let kwin_socket = crate::kwin::is_kwin_virtual_keyboard();

    let (conn, has_v1, has_v2) = connect_and_probe()?;
    log::info!(
        "[Main] input-method protocols: v1={} v2={} (kwin_socket={})",
        has_v1, has_v2, kwin_socket
    );

    if has_v2 {
        let host = WaylandInputHost::new(processor, gui_tx, tray_tx, conn, !kwin_socket, config.clone());
        let vkbd = host.vkbd();
        let desc = if kwin_socket { " Wayland v2 (KWin)" } else { " Wayland v2" };
        return Ok((Box::new(host), vkbd, desc));
    }

    if has_v1 {
        log::warn!("[Main] input-method v2 unavailable, falling back to v1 (compat mode)");
        let host = WaylandInputHostV1::new(processor, gui_tx, tray_tx, conn, !kwin_socket, config);
        let vkbd = host.vkbd();
        return Ok((Box::new(host), vkbd, " Wayland v1 (compat)"));
    }

    Err("no input-method protocol (v1/v2) on this Wayland connection".into())
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
