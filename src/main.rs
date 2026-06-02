#[cfg(windows)]
pub use qianyan_ime_windows::{IME_ID, LANG_PROFILE_ID};

// 使用 crates/ 库替代本地模块
use qianyan_ime_core::config::Config;
use qianyan_ime_core::utils::{find_project_root, load_punctuation_dict, load_syllable_frequencies, load_syllables};
use qianyan_ime_engine::processor::Processor;
use qianyan_ime_engine::compiler;
use qianyan_ime_ui::GuiEvent;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex, RwLock};

static WEB_SERVER_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("RUST_BACKTRACE", "1");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        eprintln!("\n======= PANIC =======");
        default_hook(info);
    }));
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 在 Linux 下默认使用 software 渲染后端以提高兼容性，除非用户显式设置了 SLINT_BACKEND
    if cfg!(target_os = "linux") {
        if std::env::var("SLINT_BACKEND").is_err() {
            std::env::set_var("SLINT_BACKEND", "software");
        }
    } else {
        std::env::set_var("SLINT_BACKEND", "skia");
    }

    let args: Vec<String> = env::args().collect();
    let should_daemonize = match qianyan_ime_linux::cli::handle_startup(&args)? {
        qianyan_ime_linux::cli::StartupAction::Exit => return Ok(()),
        qianyan_ime_linux::cli::StartupAction::Continue { should_daemonize } => should_daemonize,
    };

    #[cfg(target_os = "windows")]
    let _mutex_guard = unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::*;

        struct MutexGuard(windows::Win32::Foundation::HANDLE);
        impl Drop for MutexGuard {
            fn drop(&mut self) {
                let _ = unsafe { CloseHandle(self.0) };
            }
        }

        let raw_name = r"Global\QianyanIMEUniqueMutex\0"
            .encode_utf16()
            .collect::<Vec<u16>>();
        let name = windows::core::PCWSTR(raw_name.as_ptr());
        let handle = CreateMutexW(None, true, name)?;
        if windows::Win32::Foundation::GetLastError()
            .is_err_and(|e| e.code() == windows::Win32::Foundation::ERROR_ALREADY_EXISTS.to_hresult())
        {
            let _ = notify_rust::Notification::new()
                .summary("Qianyan IME")
                .body("程序已经在运行中。")
                .appname("Qianyan IME")
                .timeout(notify_rust::Timeout::Milliseconds(3000))
                .show();
            return Ok(());
        }
        MutexGuard(handle)
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
        current_config.punctuations = punctuations;
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
    let syllable_freq = load_syllable_frequencies(&root);
    let mut processor_obj = Processor::new(trie_paths, syllables, syllable_freq);
    if let Ok(conf) = config.read() {
        processor_obj.apply_config(&conf);
    }
    let processor = Arc::new(Mutex::new(processor_obj));

    let tray_handle = if let Ok(conf) = config.read() {
        if conf.appearance.show_tray {
            Some(qianyan_ime_ui::tray::start_tray(qianyan_ime_ui::tray::TrayParams {
                active_profile: conf.input.default_profile.clone(),
                enabled_profiles: conf.input.enabled_profiles.clone(),
                tx: tray_tx.clone(),
            }))
        } else {
            None
        }
    } else {
        Some(qianyan_ime_ui::tray::start_tray(qianyan_ime_ui::tray::TrayParams {
            active_profile: "chinese".into(),
            enabled_profiles: vec!["chinese".into()],
            tx: tray_tx.clone(),
        }))
    };

    // 全局状态维护
    let app_state = Arc::new(Mutex::new(qianyan_ime_ui::AppState {
        chinese_enabled: true,
        active_profile: "".into(),
        show_candidates_pref: config.read().map_or(true, |c| c.appearance.show_candidates),
        is_ime_active: true,
        pinyin: "".into(),
        candidates: vec![],
        selected_index: 0,
        page: 0,
        total_pages: 0,
        status_text: "中".into(),
    }));

    let processor_clone = processor.clone();
    let gui_tx_tray = gui_tx.clone();
    let tray_tx_for_main_loop = tray_tx.clone();
    let config_msg = config.clone();
    let app_state_tray = app_state.clone();

    // Tray 事件处理线程。
    // 架构约定：Processor 锁只用于极短的临界区（读取/写入处理器的状态），
    // GUI 更新、托盘图标更新等操作在锁外执行，避免阻塞其他线程（evdev/背景检索）。
    std::thread::spawn(move || {
        while let Ok(event) = tray_rx.recv() {
            match event {
                qianyan_ime_ui::tray::TrayEvent::ToggleIme => {
                    // 临界区：仅读取处理器状态，不做 GUI 操作
                    let (enabled, short) = {
                        let mut p = match processor_clone.lock() {
                            Ok(guard) => guard,
                            Err(_) => continue,
                        };
                        p.toggle();
                        (p.ctx.session_state.chinese_enabled, p.get_short_display())
                    };
                    // GUI/托盘更新在锁外执行
                    if let Some(ref handle) = tray_handle {
                        handle.update(move |t| t.chinese_enabled = enabled);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.chinese_enabled = enabled;
                        state.status_text = if enabled { short } else { "英".into() };
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::NextProfile => {
                    let (profile, enabled, short, commit_mode) = {
                        let mut p = match processor_clone.lock() {
                            Ok(guard) => guard,
                            Err(_) => continue,
                        };
                        let profile = p.next_profile();
                        let enabled = p.ctx.session_state.chinese_enabled;
                        let short = p.get_short_display();
                        let commit_mode = p.ctx.config.commit_mode().to_string();
                        (profile, enabled, short, commit_mode)
                    };
                    let profile_for_tray = profile.clone();
                    if let Some(ref handle) = tray_handle {
                        handle.update(move |t| t.active_profile = profile_for_tray);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = if enabled { short } else { "英".into() };
                        state.chinese_enabled = enabled;
                        state.active_profile = profile;
                        state.pinyin = "".into();
                        state.candidates = vec![];
                        state.selected_index = 0;
                        state.page = 0;
                        state.total_pages = 0;
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                        let _ = gui_tx_tray.send(GuiEvent::Update {
                            pinyin: "".into(),
                            candidates: vec![],
                            selected: 0,
                            page: 0,
                            total_pages: 0,
                            sentence: "".into(),
                            cursor_pos: 0,
                            commit_mode,
                        });
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::SetProfile(profile) => {
                    let result = {
                        let mut p = match processor_clone.lock() {
                            Ok(guard) => guard,
                            Err(_) => continue,
                        };
                        let profiles: Vec<String> = profile.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| p.ctx.engine.trie_paths.contains_key(s))
                            .collect();
                        if profiles.is_empty() {
                            None
                        } else {
                            p.ctx.session_state.active_profiles = profiles;
                            if let Ok(conf) = p.ctx.config.master_config_write() {
                                conf.input.default_profile = profile.clone();
                                let _ = conf.save();
                            }
                            p.reset();
                            Some((p.ctx.session_state.chinese_enabled, p.get_short_display(), p.ctx.config.commit_mode().to_string()))
                        }
                    };
                    let Some((enabled, short, commit_mode)) = result else {
                        continue;
                    };
                    if let Some(ref handle) = tray_handle {
                        let profile_for_tray = profile.clone();
                        handle.update(move |t| t.active_profile = profile_for_tray);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = if enabled { short } else { "英".into() };
                        state.chinese_enabled = enabled;
                        state.active_profile = profile;
                        state.pinyin = "".into();
                        state.candidates = vec![];
                        state.selected_index = 0;
                        state.page = 0;
                        state.total_pages = 0;
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                        let _ = gui_tx_tray.send(GuiEvent::Update {
                            pinyin: "".into(),
                            candidates: vec![],
                            selected: 0,
                            page: 0,
                            total_pages: 0,
                            sentence: "".into(),
                            cursor_pos: 0,
                            commit_mode,
                        });
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::SyncStatus {
                    chinese_enabled,
                    active_profile,
                } => {
                    if let Some(ref handle) = tray_handle {
                        let ce = chinese_enabled;
                        let ap = active_profile.clone();
                        handle.update(move |t| {
                            t.chinese_enabled = ce;
                            t.active_profile = ap;
                        });
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.chinese_enabled = chinese_enabled;
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::OpenConfig => {
                    if !WEB_SERVER_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
                        WEB_SERVER_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);
                        let config_web = config_msg.clone();
                        let tray_tx_web = tray_tx_for_main_loop.clone();
                        std::thread::spawn(move || {
                            if let Ok(rt) = tokio::runtime::Runtime::new() {
                                rt.block_on(async {
                                    let server = qianyan_ime_ui::web::WebServer::new(
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
                qianyan_ime_ui::tray::TrayEvent::ReloadConfig => {
                    let new_conf = Config::load();
                    {
                        let mut p = match processor_clone.lock() {
                            Ok(guard) => guard,
                            Err(_) => continue,
                        };
                        p.apply_config(&new_conf);
                    }
                    if let Some(ref handle) = tray_handle {
                        let enabled = new_conf.input.enabled_profiles.clone();
                        handle.update(move |t| {
                            t.enabled_profiles = enabled;
                        });
                    }
                    let _ = gui_tx_tray.send(GuiEvent::ApplyConfig(Box::new(new_conf)));
                }
                qianyan_ime_ui::tray::TrayEvent::ShowNotification(msg) => {
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = msg;
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::ClearUserDict => {
                    let profiles = {
                        let p = match processor_clone.lock() {
                            Ok(guard) => guard,
                            Err(_) => continue,
                        };
                        p.ctx.config.list_profiles()
                    };
                    for profile in profiles {
                        let mut p = match processor_clone.lock() {
                            Ok(guard) => guard,
                            Err(_) => break,
                        };
                        if let Err(e) = p.ctx.config.clear_user_data(&profile) {
                            eprintln!("清除用户数据失败: {}", e);
                        }
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::Exit => {
                    let _ = gui_tx_tray.send(GuiEvent::Exit);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    std::process::exit(0);
                }
                qianyan_ime_ui::tray::TrayEvent::SendKey(_) => {}
            }
        }
    });

    let (vkbd_option, host_run) = qianyan_ime_linux::runtime::create_input_host(
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

    #[cfg(target_os = "linux")]
    {
        use qianyan_ime_ui::ipc::transport::*;
        use std::os::unix::net::UnixListener;
        use std::time::Duration;

        // GUI 作为独立进程运行，通过 Unix socket IPC 通信
        let socket_path = format!("/tmp/qianyan-ime-{}.sock", std::process::id());
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)
            .expect("Failed to bind IPC socket");

        let exe_path = std::env::current_exe().unwrap();
        let gui_exe = exe_path.parent().unwrap().join("qianyan-ime-gui");
        let mut child = std::process::Command::new(&gui_exe)
            .arg(&socket_path)
            .spawn()
            .expect("Failed to spawn GUI process");

        // Wait for GUI to connect
        let (mut stream, _) = listener.accept()
            .expect("Failed to accept GUI connection");

        // Send initial config
        let cfg = config.read().expect("config lock poisoned").clone();
        let _ = send_main_to_gui(&mut stream, &MainToGui::ApplyConfig(
            serde_json::to_string(&cfg).unwrap(),
        ));

        // Wait for GUI to signal ready
        match recv_gui_to_main(&mut stream, Some(Duration::from_secs(5))) {
            Ok(Some(GuiToMain::Ready)) => {},
            _ => eprintln!("[Main] GUI did not signal ready"),
        }

        // Forward events from gui_rx to IPC
        let stream = std::sync::Mutex::new(Some(stream));
        std::thread::spawn(move || {
            while let Ok(event) = gui_rx.recv() {
                // Take the stream, if it was already consumed (GUI died) -> skip
                let mut stream_guard = match stream.lock() {
                    Ok(g) => g,
                    Err(_) => break,
                };
                let stream_ref = match stream_guard.as_mut() {
                    Some(s) => s,
                    None => break, // already closed
                };
                match event {
                    GuiEvent::HideAndAck(ack_tx) => {
                        if send_main_to_gui(stream_ref, &MainToGui::HideCandidate).is_err() {
                            stream_guard.take(); // GUI died
                            let _ = ack_tx.send(());
                            break;
                        }
                        match recv_gui_to_main(stream_ref, Some(Duration::from_millis(100))) {
                            Ok(Some(GuiToMain::Ack)) => {},
                            _ => {},
                        }
                        let _ = ack_tx.send(());
                    }
                    GuiEvent::Exit => {
                        let _ = send_main_to_gui(stream_ref, &MainToGui::Exit);
                        break;
                    }
                    other => {
                        if let Some(ipc) = gui_event_to_ipc(other) {
                            if send_main_to_gui(stream_ref, &ipc).is_err() {
                                stream_guard.take(); // GUI died
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Wait for GUI process to exit
        child.wait().ok();
        let _ = std::fs::remove_file(&socket_path);
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Fallback: GUI in same process (Windows)
        qianyan_ime_ui::gui::start_gui(gui_rx, config, tray_tx);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn gui_event_to_ipc(event: GuiEvent) -> Option<qianyan_ime_ui::ipc::transport::MainToGui> {
    use qianyan_ime_ui::ipc::transport::{self, MainToGui};
    match event {
        GuiEvent::Update { pinyin, candidates, selected, page, total_pages, .. } => {
            Some(MainToGui::Update {
                pinyin,
                candidates: candidates.into_iter().map(|c| transport::DisplayCandidateMsg {
                    text: c.text,
                    label: c.label,
                    hint: c.hint,
                    is_fuzzy: c.is_fuzzy,
                }).collect(),
                selected,
                page,
                total_pages,
            })
        }
        GuiEvent::SyncState(state) => {
            Some(MainToGui::SyncState(transport::AppStateMsg {
                chinese_enabled: state.chinese_enabled,
                active_profile: state.active_profile,
                show_candidates_pref: state.show_candidates_pref,
                is_ime_active: state.is_ime_active,
                pinyin: state.pinyin,
                candidates: state.candidates.into_iter().map(|c| transport::DisplayCandidateMsg {
                    text: c.text,
                    label: c.label,
                    hint: c.hint,
                    is_fuzzy: c.is_fuzzy,
                }).collect(),
                selected_index: state.selected_index,
                page: state.page,
                total_pages: state.total_pages,
                status_text: state.status_text,
            }))
        }
        GuiEvent::MoveTo { x, y } => Some(MainToGui::MoveTo { x, y }),
        GuiEvent::SetVisible(v) => Some(MainToGui::SetVisible(v)),
        GuiEvent::ShowStatus(text, chinese) => Some(MainToGui::ShowStatus(text, chinese)),
        GuiEvent::ApplyConfig(config) => {
            serde_json::to_string(&*config).ok().map(|json| MainToGui::ApplyConfig(json))
        }
        _ => None,
    }
}

#[cfg(target_os = "windows")]
pub fn setup_autostart() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_str().ok_or("Invalid path")?;
    let _ = std::process::Command::new("reg")
        .arg("add")
        .arg("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .arg("/v")
        .arg("QianyanIME")
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
        .arg("QianyanIME")
        .arg("/f")
        .status();
    Ok(())
}
