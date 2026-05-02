#![cfg(target_os = "windows")]

use shian_ime_core::Config;
use shian_ime_core::utils::{find_project_root, load_punctuation_dict, load_syllables};
use shian_ime_engine::Processor;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, RwLock};

pub mod tray;
pub mod runtime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_target(false)
        .with_thread_ids(true)
        .init();

    std::env::set_var("SLINT_BACKEND", "skia");

    unsafe {
        use windows::Win32::UI::HiDpi::*;
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwareness(
            windows::Win32::UI::HiDpi::PROCESS_PER_MONITOR_DPI_AWARE,
        );
    }

    let args: Vec<String> = env::args().collect();

    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
        use windows::Win32::System::Threading::*;

        let name = PCWSTR(
            r"Global\RustImeUniqueMutex\0"
                .encode_utf16()
                .collect::<Vec<u16>>()
                .as_ptr(),
        );
        let handle = CreateMutexW(None, true, name)?;
        if windows::Win32::Foundation::GetLastError()
            .is_err_and(|e| e.code() == ERROR_ALREADY_EXISTS.to_hresult())
        {
            let _ = notify_rust::Notification::new()
                .summary("Rust IME")
                .body("程序已经在运行中。")
                .appname("Rust IME")
                .timeout(notify_rust::Timeout::Milliseconds(3000))
                .show();
            return Ok(());
        }
        let _ = handle;
    }

    let root = find_project_root();

    if !root.join("data/chinese/trie.index").exists() {
        let _ = shian_ime_engine::compiler::check_and_compile_all();
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
        current_config.input.punctuations = punctuations;
    }

    let config = Arc::new(RwLock::new(current_config));
    let (gui_tx, gui_rx) = std::sync::mpsc::channel();
    let (tray_tx, tray_rx) = std::sync::mpsc::channel();

    let gui_config = config
        .read()
        .map_or_else(|_| Config::default_config(), |c| c.clone());
    let tray_tx_for_gui = tray_tx.clone();
    std::thread::spawn(move || {
        ui::gui::start_gui(gui_rx, gui_config, tray_tx_for_gui);
    });

    let mut trie_paths = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(root.join("data")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry
                    .file_name()
                    .to_string_lossy()
                    .to_lowercase();
                let trie_idx = entry.path().join("trie.index");
                let trie_dat = entry.path().join("trie.data");
                if trie_idx.exists() && trie_dat.exists() {
                    trie_paths.insert(dir_name, (trie_idx, trie_dat));
                }
            }
        }
    }

    let syllables = load_syllables(&root);
    let mut processor_obj = Processor::new(trie_paths, syllables);
    if let Ok(conf) = config.read() {
        processor_obj.apply_config(&conf);
    }
    let processor = Arc::new(Mutex::new(processor_obj));

    let _tray_handle = tray::start_tray(tray::TrayParams {
        active_profile: config.read().map(|c| c.input.default_profile.clone()).unwrap_or_else(|_| "chinese".into()),
        show_status_bar: config.read().map(|c| c.appearance.show_status_bar).unwrap_or(true),
        tx: tray_tx.clone(),
    });

    let app_state = Arc::new(Mutex::new(ui::AppState {
        chinese_enabled: true,
        active_profile: "".into(),
        show_status_bar_pref: true,
        show_candidates_pref: true,
        is_ime_active: true,
        pinyin: "".into(),
        candidates: vec![],
        selected_index: 0,
        status_text: "中".into(),
    }));

    runtime::run_input_host(
        processor,
        Some(gui_tx),
        config.clone(),
        tray_tx,
        app_state,
    )?;

    Ok(())
}
