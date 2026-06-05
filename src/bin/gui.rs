use qianyan_ime_core::Config;
use qianyan_ime_ui::gui_slint;
use qianyan_ime_ui::ipc::transport::*;
use std::os::unix::net::UnixStream;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Ask kernel to kill us when the parent (qianyan-ime) dies.
    // This handles all exit scenarios: tray Exit, Ctrl+C, SIGTERM, crash, etc.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: qianyan-ime-gui <socket_path>");
        std::process::exit(1);
    }
    let socket_path = &args[1];

    // Connect to the main process
    let mut stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[GUI] Failed to connect to main process: {e}");
            std::process::exit(1);
        }
    };

    // Read initial config (main process sends it first)
    let initial_config: Config = match recv_main_to_gui(&mut stream) {
        Ok(Some(MainToGui::ApplyConfig(json))) => serde_json::from_str(&json).unwrap_or_else(|e| {
            log::error!("[GUI] Failed to parse initial config: {e}");
            Config::load()
        }),
        _ => {
            log::error!("[GUI] Failed to receive initial config");
            Config::load()
        }
    };

    log::info!("[GUI] Connected, starting Slint event loop");
    gui_slint::start_gui_ipc(stream, initial_config);
}
