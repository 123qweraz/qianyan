#[cfg(target_os = "windows")]
pub mod registry;

mod constants;

#[cfg(windows)]
pub use crate::constants::{IME_ID, LANG_PROFILE_ID};

// 使用 crates/ 库替代本地模块
use shian_ime_core::config::Config;
use shian_ime_core::utils::{find_project_root, load_punctuation_dict, load_syllables};
use shian_ime_engine::processor::Processor;
use shian_ime_engine::compiler;
use shian_ime_ui::GuiEvent;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex, RwLock};

static WEB_SERVER_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 强制使用 Skia 渲染后端以支持彩色 Emoji 和高质量文字渲染
    std::env::set_var("SLINT_BACKEND", "skia");

    let args: Vec<String> = env::args().collect();
    let should_daemonize = match shian_ime_linux::cli::handle_startup(&args)? {
        shian_ime_linux::cli::StartupAction::Exit => return Ok(()),
        shian_ime_linux::cli::StartupAction::Continue { should_daemonize } => should_daemonize,
    };

    #[cfg(target_os = "windows")]
    let _mutex_handle = unsafe {
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
        handle
    };

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::HiDpi::*;
        let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
    }

    let mut root = find_project_root();
    let mut current_config = Config::load();
    
    // 如果配置中指定了数据目录，则优先使用
    if let Some(ref custom_root) = current_config.files.data_dir {
        let p = std::path::PathBuf::from(custom_root);
        if p.exists() {
            root = p;
        }
    }

    if should_daemonize {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::System::Console::GetConsoleWindow;
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
            let window = unsafe { GetConsoleWindow() };
            if window.0 != 0 {
                unsafe {
                    ShowWindow(window, SW_HIDE);
                }
            }
        }
    }

    if !root.join("data/chinese/trie.index").exists() {
        let _ = compiler::check_and_compile_all();
    }

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

    let mut trie_paths = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(root.join("data")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let dir_name = entry
                    .file_name()
                    .to_string_lossy()
                    .to_string()
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

    let tray_handle = if let Ok(conf) = config.read() {
        shian_ime_ui::tray::start_tray(shian_ime_ui::tray::TrayParams {
            active_profile: conf.input.default_profile.clone(),
            show_status_bar: conf.appearance.show_status_bar,
            tx: tray_tx.clone(),
        })
    } else {
        shian_ime_ui::tray::start_tray(shian_ime_ui::tray::TrayParams {
            active_profile: "chinese".into(),
            show_status_bar: true,
            tx: tray_tx.clone(),
        })
    };

    // 全局状态维护
    let app_state = Arc::new(Mutex::new(shian_ime_ui::AppState {
        chinese_enabled: true,
        active_profile: "".into(),
        show_status_bar_pref: config.read().map_or(true, |c| c.appearance.show_status_bar),
        show_candidates_pref: config.read().map_or(true, |c| c.appearance.show_candidates),
        is_ime_active: true,
        pinyin: "".into(),
        candidates: vec![],
        selected_index: 0,
        status_text: "中".into(),
    }));

    let processor_clone = processor.clone();
    let gui_tx_tray = gui_tx.clone();
    let tray_tx_for_main_loop = tray_tx.clone();
    let config_msg = config.clone();
    let app_state_tray = app_state.clone();

    std::thread::spawn(move || {
        while let Ok(event) = tray_rx.recv() {
            match event {
                shian_ime_ui::tray::TrayEvent::ToggleIme => {
                    if let Ok(mut p) = processor_clone.lock() {
                        p.toggle();
                        let enabled = p.ctx.session_state.chinese_enabled;
                        let short = p.get_short_display();
                        tray_handle.update(move |t| t.chinese_enabled = enabled);

                        if let Ok(mut state) = app_state_tray.lock() {
                            state.chinese_enabled = enabled;
                            state.status_text = if enabled { short } else { "英".into() };
                            let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                        }
                    }
                }
                shian_ime_ui::tray::TrayEvent::NextProfile => {
                    if let Ok(mut p) = processor_clone.lock() {
                        let profile = p.next_profile();
                        let enabled = p.ctx.session_state.chinese_enabled;
                        let short = p.get_short_display();
                        tray_handle.update(move |t| t.active_profile = profile);

                        if let Ok(mut state) = app_state_tray.lock() {
                            state.status_text = if enabled { short } else { "英".into() };
                            state.chinese_enabled = enabled;
                            let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                        }
                    }
                }
                shian_ime_ui::tray::TrayEvent::ToggleStatusBar => {
                    let mut new_show = false;
                    if let Ok(mut w) = config_msg.write() {
                        w.appearance.show_status_bar = !w.appearance.show_status_bar;
                        new_show = w.appearance.show_status_bar;
                        let _ = w.save();
                    }
                    tray_handle.update(move |t| t.show_status_bar = new_show);

                    if let Ok(mut state) = app_state_tray.lock() {
                        state.show_status_bar_pref = new_show;
                        let _ = gui_tx_tray.send(GuiEvent::ForceStatusVisible(new_show));
                    }
                }
                shian_ime_ui::tray::TrayEvent::SyncStatus {
                    chinese_enabled,
                    active_profile,
                } => {
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.chinese_enabled = chinese_enabled;
                        state.active_profile = active_profile;
                    }
                }
                shian_ime_ui::tray::TrayEvent::OpenConfig => {
                    if !WEB_SERVER_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
                        WEB_SERVER_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);
                        let config_web = config_msg.clone();
                        let tray_tx_web = tray_tx_for_main_loop.clone();
                        std::thread::spawn(move || {
                            if let Ok(rt) = tokio::runtime::Runtime::new() {
                                rt.block_on(async {
                                    let server = shian_ime_ui::web::WebServer::new(
                                        18765,
                                        Arc::new(std::sync::atomic::AtomicU16::new(18765)),
                                        config_web,
                                        Arc::new(RwLock::new(HashMap::new())),
                                        tray_tx_web,
                                    );
                                    server.start().await;
                                });
                            }
                        });
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = open::that("http://127.0.0.1:18765");
                    }
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd")
                        .arg("/c")
                        .arg("start")
                        .arg("http://localhost:18765")
                        .spawn();
                }
                shian_ime_ui::tray::TrayEvent::ReloadConfig => {
                    let new_conf = Config::load();
                    if let Ok(mut p) = processor_clone.lock() {
                        p.apply_config(&new_conf);
                    }
                    let _ = gui_tx_tray.send(GuiEvent::ApplyConfig(Box::new(new_conf)));
                }
                shian_ime_ui::tray::TrayEvent::ShowNotification(msg) => {
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = msg;
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                    }
                }
                shian_ime_ui::tray::TrayEvent::ClearUserDict => {
                    if let Ok(mut p) = processor_clone.lock() {
                        let profiles = p.ctx.config.list_profiles();
                        for profile in profiles {
                            if let Err(e) = p.ctx.config.clear_user_data(&profile) {
                                eprintln!("清除用户数据失败: {}", e);
                            }
                        }
                    }
                }
                shian_ime_ui::tray::TrayEvent::Exit => std::process::exit(0),
                shian_ime_ui::tray::TrayEvent::SendKey(_) => {
                    // 暂不处理 SendKey 事件
                }
            }
        }
    });

    let (vkbd_option, host_run) = shian_ime_linux::runtime::create_input_host(
        &args,
        processor.clone(),
        gui_tx.clone(),
        config.clone(),
        tray_tx.clone(),
        app_state.clone(),
    )?;

    // 如果有 vkbd（Evdev 模式），可以在这里使用
    let _ = vkbd_option;

    // 在新线程中运行输入主机
    std::thread::spawn(move || {
        host_run();
    });

    // GUI 在主线程运行（Slint 事件循环需要主线程）
    let gui_config = config
        .read()
        .map_or_else(|_| Config::default_config(), |c| c.clone());
    shian_ime_ui::gui::start_gui(gui_rx, gui_config, tray_tx);

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn setup_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_str().ok_or("Invalid path")?;
    let _ = std::process::Command::new("reg")
        .arg("add")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg("RustIME")
        .arg("/t")
        .arg("REG_SZ")
        .arg("/d")
        .arg(exe_path)
        .arg("/f")
        .status();
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn remove_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let _ = std::process::Command::new("reg")
        .arg("delete")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg("RustIME")
        .arg("/f")
        .status();
    Ok(())
}
