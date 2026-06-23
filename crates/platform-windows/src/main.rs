#![cfg(target_os = "windows")]

use qianyan_ime_core::Config;
use qianyan_ime_core::utils::{find_project_root, load_punctuation_dict};
use qianyan_ime_ui::tray;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    std::env::set_var("SLINT_BACKEND", "skia");

    unsafe {
        use windows::Win32::UI::HiDpi::*;
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwareness(
            windows::Win32::UI::HiDpi::PROCESS_PER_MONITOR_DPI_AWARE,
        );
    }

    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::*;

        struct MutexGuard(windows::Win32::Foundation::HANDLE);
        impl Drop for MutexGuard {
            fn drop(&mut self) {
                let _ = unsafe { CloseHandle(self.0) };
            }
        }

        let name_buf: Vec<u16> = "Global\\QianyanIMEUniqueMutex"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name = PCWSTR(name_buf.as_ptr());
        let handle = CreateMutexW(None, true, name)?;
        let mutex_guard = MutexGuard(handle);
        if windows::Win32::Foundation::GetLastError()
            .is_err_and(|e| e.code() == ERROR_ALREADY_EXISTS.to_hresult())
        {
            let _ = notify_rust::Notification::new()
                .summary("Qianyan IME")
                .body("程序已经在运行中。")
                .appname("Qianyan IME")
                .timeout(notify_rust::Timeout::Milliseconds(3000))
                .show();
            return Ok(());
        }
        let _mutex_guard = mutex_guard;
    }

    let root = find_project_root();

    if !root.join("data/chinese/trie.index").exists() {
        let _ = qianyan_ime_engine::compiler::check_and_compile_all();
    }

    let mut current_config = Config::load();
    {
        let mut punctuations = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(root.join("dicts")) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let lang = entry.file_name().to_string_lossy().to_string();
                    let punc_file = entry.path().join("punctuation.json");
                    if punc_file.exists() {
                        punctuations
                            .insert(lang, load_punctuation_dict(&punc_file.to_string_lossy()));
                    }
                }
            }
        }
        current_config.punctuations = punctuations;
    }

    let config = Arc::new(RwLock::new(current_config));
    let (gui_tx, gui_rx) = std::sync::mpsc::channel();
    let (tray_tx, _tray_rx) = std::sync::mpsc::channel();

    let gui_config = config
        .read()
        .map_or_else(|_| Config::default_config(), |c| c.clone());
    let tray_tx_for_gui = tray_tx.clone();
    std::thread::spawn(move || {
        ui::gui::start_gui(gui_rx, gui_config, tray_tx_for_gui);
    });

    let _tray_handle = tray::start_tray(tray::TrayParams {
        active_profile: config.read().map(|c| c.input.default_profile.clone()).unwrap_or_else(|_| "chinese".into()),
        enabled_profiles: vec![],
        tx: tray_tx.clone(),
    });

    // On Windows, input is handled via TSF COM callbacks (registered in lib.rs).
    // No polling loop is needed; block the main thread to keep the process alive.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }

    Ok(())
}
