#[cfg(windows)]
pub use qianyan_ime_windows::{IME_ID, LANG_PROFILE_ID};

// 使用 crates/ 库替代本地模块
use qianyan_ime_core::config::Config;
use qianyan_ime_core::utils::{find_project_root, load_punctuation_dict, load_syllable_frequencies};
use qianyan_ime_engine::processor::Processor;
use qianyan_ime_engine::processor::actor::{ProcessorHandle, ProcessorActor};
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

    // 在 KDE Wayland 下设置 IM_MODULE，防止 GUI /Qt 应用走 IBus/XIM 冲突
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        if desktop.split(':').any(|s| s.eq_ignore_ascii_case("kde")) {
            std::env::set_var("GTK_IM_MODULE", "wayland");
            std::env::set_var("QT_IM_MODULE", "wayland");
        }
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

        let raw_name: Vec<u16> = "Global\\QianyanIMEUniqueMutex"
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .collect();
        let name = windows::core::PCWSTR(raw_name.as_ptr());
        let handle = CreateMutexW(None, true, name)?;
        let mutex_guard = MutexGuard(handle);
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
        mutex_guard
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

    // 在 Linux 上，在 gui_rx 下游插入通知分发器，
    // 使桌面通知始终由主进程处理（不依赖 GUI 子进程）。
    #[cfg(target_os = "linux")]
    let gui_rx = {
        let (gui_forward_tx, gui_forward_rx) = std::sync::mpsc::channel();
        let initial_cfg = config.read().unwrap_or_else(|e| e.into_inner()).clone();
        std::thread::spawn(move || {
            let mut notifier = qianyan_ime_ui::local_notify::LocalNotify::new(&initial_cfg);
            while let Ok(event) = gui_rx.recv() {
                notifier.handle(&event);
                if gui_forward_tx.send(event).is_err() {
                    break;
                }
            }
        });
        gui_forward_rx
    };

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

    let syllable_freq = load_syllable_frequencies(&root);
    let mut processor_obj = Processor::new(trie_paths, syllable_freq);
    if let Ok(conf) = config.read() {
        processor_obj.apply_config(&conf);
    }
    let (actor_tx, actor_rx) = std::sync::mpsc::channel();
    let processor_actor = ProcessorActor::new(processor_obj, actor_rx);
    std::thread::spawn(move || processor_actor.run());
    let processor_handle = ProcessorHandle::new(actor_tx);

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
        ime_enabled: true,
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

    let processor_clone = processor_handle.clone();
    let gui_tx_tray = gui_tx.clone();
    let tray_tx_for_main_loop = tray_tx.clone();
    let config_msg = config.clone();
    let app_state_tray = app_state.clone();
    let root_for_tray = root.clone();

    // Tray 事件处理线程（无锁：通过 ProcessorHandle 与 Actor 通信）
    std::thread::spawn(move || {
        while let Ok(event) = tray_rx.recv() {
            match event {
                qianyan_ime_ui::tray::TrayEvent::ToggleIme => {
                    let snap = match processor_clone.toggle() {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Some(ref handle) = tray_handle {
                        handle.update(move |t| t.chinese_enabled = snap.chinese_enabled);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.chinese_enabled = snap.chinese_enabled;
                        state.status_text = if snap.chinese_enabled { snap.short_display } else { "英".into() };
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::ToggleEnabled => {
                    let snap = match processor_clone.toggle_enabled() {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Some(ref handle) = tray_handle {
                        let ime_enabled = snap.ime_enabled;
                        handle.update(move |t| t.ime_enabled = ime_enabled);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.ime_enabled = snap.ime_enabled;
                        state.chinese_enabled = snap.chinese_enabled;
                        state.status_text = if snap.ime_enabled {
                            if snap.chinese_enabled { snap.short_display } else { "英".into() }
                        } else {
                            "禁".into()
                        };
                        let _ = gui_tx_tray.send(GuiEvent::SyncState(state.clone()));
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::NextProfile => {
                    let snap = match processor_clone.next_profile() {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Some(ref handle) = tray_handle {
                        let profile = snap.active_profile.clone();
                        handle.update(move |t| t.active_profile = profile);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = if snap.chinese_enabled { snap.short_display } else { "英".into() };
                        state.chinese_enabled = snap.chinese_enabled;
                        state.active_profile = snap.active_profile;
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
                            commit_mode: snap.commit_mode,
                        });
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::SetProfile(profile) => {
                    let snap = match processor_clone.set_profile(profile.clone()) {
                        Some(Some(s)) => s,
                        _ => continue,
                    };
                    if let Some(ref handle) = tray_handle {
                        let profile_for_tray = profile.clone();
                        handle.update(move |t| t.active_profile = profile_for_tray);
                    }
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = if snap.chinese_enabled { snap.short_display } else { "英".into() };
                        state.chinese_enabled = snap.chinese_enabled;
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
                            commit_mode: snap.commit_mode,
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
                        let root_web = root_for_tray.clone();
                        std::thread::spawn(move || {
                            if let Ok(rt) = tokio::runtime::Runtime::new() {
                                rt.block_on(async {
                                    let server = qianyan_ime_ui::web::WebServer::new(
                                        18765,
                                        Arc::new(std::sync::atomic::AtomicU16::new(18765)),
                                        config_web,
                                        Arc::new(RwLock::new(HashMap::new())),
                                        tray_tx_web,
                                        root_web,
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
                    processor_clone.apply_config(new_conf.clone());
                    processor_clone.reload_tries();
                    if let Ok(mut cfg) = config_msg.write() {
                        *cfg = new_conf.clone();
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
                qianyan_ime_ui::tray::TrayEvent::ClearUserDict(profile_opt) => {
                    if let Some(profile) = profile_opt {
                        processor_clone.clear_user_data(profile);
                    } else {
                        let profiles = processor_clone.list_profiles().unwrap_or_default();
                        for profile in profiles {
                            processor_clone.clear_user_data(profile);
                        }
                    }
                }
                qianyan_ime_ui::tray::TrayEvent::Exit => {
                    processor_clone.exit();
                    let _ = gui_tx_tray.send(GuiEvent::Exit);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    std::process::exit(0);
                }
                qianyan_ime_ui::tray::TrayEvent::SendKey(_) => {}
            }
        }
    });

    // 在 KDE 上自动注册虚拟键盘，以便 KWin 通过 WAYLAND_SOCKET 启动输入法
    #[cfg(target_os = "linux")]
    {
        if qianyan_ime_linux::kwin::is_kde_session()
            && !qianyan_ime_linux::kwin::is_kwin_virtual_keyboard()
        {
            match qianyan_ime_linux::kwin::configure_kde_virtual_keyboard() {
                Ok(()) => {
                    log::info!(
                        "[Main] KWin 虚拟键盘已注册。重新登入 Wayland 会话或执行 \
                         'kwin_x11 --replace && kwin_wayland --replace' 后生效。"
                    );
                }
                Err(e) => log::warn!("[Main] KWin 虚拟键盘注册失败: {e}"),
            }
        }
    }

    let (vkbd_option, host_run) = qianyan_ime_linux::runtime::create_input_host(
        &args,
        processor_handle.clone(),
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

    // 启动 IBus D-Bus 后端（在所有桌面环境下均可使用）
    #[cfg(target_os = "linux")]
    {
        qianyan_ime_linux::runtime::start_ibus_backend(
            processor_handle.clone(),
            gui_tx.clone(),
            tray_tx.clone(),
        );
    }

    #[cfg(not(target_os = "linux"))]
    {
        // Fallback: GUI in same process (Windows)
        qianyan_ime_ui::gui::start_gui(gui_rx, config, tray_tx);
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let is_kwin = qianyan_ime_linux::kwin::is_kwin_virtual_keyboard();
        let no_gui = args.iter().any(|a| a == "--no-gui");
        let show_gui = config.read().map(|c| c.linux.show_slint_window).unwrap_or(true);

        let launch_gui = !is_kwin && !no_gui && show_gui;

        if launch_gui {
            match try_start_gui_process(&config) {
                Ok((child, stream, socket_path)) => {
                    log::info!("[Main] GUI 子进程已启动");
                    spawn_gui_forwarder(gui_rx, stream, child, socket_path);
                }
                Err(e) => {
                    log::warn!("[Main] GUI 启动失败 (无 GUI 继续运行): {e}");
                    spawn_gui_null_handler(gui_rx);
                }
            }
        } else {
            if is_kwin {
                log::info!("[Main] KWin 虚拟键盘模式：跳过 Slint GUI");
            } else if no_gui {
                log::info!("[Main] --no-gui 参数：无 GUI 模式");
            } else {
                log::info!("[Main] show_slint_window=false：无 GUI 模式");
            }
            spawn_gui_null_handler(gui_rx);
        }

        // 保持主线程存活，让 IBus/Wayland 后端继续服务
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
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
            serde_json::to_string(&*config).ok().map(MainToGui::ApplyConfig)
        }
        _ => None,
    }
}

/// 尝试启动 GUI 子进程并建立 IPC 连接。
/// 返回 (child, stream, socket_path)，主线程不再阻塞等待 child。
#[cfg(target_os = "linux")]
fn try_start_gui_process(
    config: &std::sync::Arc<std::sync::RwLock<Config>>,
) -> Result<
    (
        std::process::Child,
        std::os::unix::net::UnixStream,
        String,
    ),
    Box<dyn std::error::Error>,
> {
    use qianyan_ime_ui::ipc::transport::*;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    let socket_path = format!("/tmp/qianyan-ime-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)?;
    listener.set_nonblocking(true)?;

    let exe_path = std::env::current_exe()?;
    let gui_exe = exe_path
        .parent()
        .ok_or("exe has no parent directory")?
        .join("qianyan-ime-gui");
    let mut child = std::process::Command::new(&gui_exe)
        .arg(&socket_path)
        .env_remove("WAYLAND_SOCKET")
        .spawn()?;

    // 轮询等待 GUI 连接，同时检查子进程存活
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if let Ok(Some(_)) = child.try_wait() {
                    let _ = std::fs::remove_file(&socket_path);
                    return Err("GUI exited before connecting".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&socket_path);
                return Err(e.into());
            }
        }
    };
    listener.set_nonblocking(false)?;

    // 发送初始配置
    let cfg = config.read().unwrap_or_else(|e| e.into_inner()).clone();
    send_main_to_gui(&mut stream, &MainToGui::ApplyConfig(serde_json::to_string(&cfg)?))?;

    match recv_gui_to_main(&mut stream, Some(Duration::from_secs(5))) {
        Ok(Some(GuiToMain::Ready)) => {}
        _ => log::warn!("[Main] GUI did not signal ready"),
    }

    Ok((child, stream, socket_path))
}

/// 将 gui_rx 事件通过 Unix socket 转发到 GUI 子进程。
/// GUI 子进程在单独线程中监控，退出时自动清理 socket 文件。
#[cfg(target_os = "linux")]
fn spawn_gui_forwarder(
    gui_rx: std::sync::mpsc::Receiver<GuiEvent>,
    stream: std::os::unix::net::UnixStream,
    mut child: std::process::Child,
    socket_path: String,
) {
    use qianyan_ime_ui::ipc::transport::*;
    use std::time::Duration;

    let stream = std::sync::Mutex::new(Some(stream));
    std::thread::spawn(move || {
        while let Ok(event) = gui_rx.recv() {
            let mut stream_guard = match stream.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            let stream_ref = match stream_guard.as_mut() {
                Some(s) => s,
                None => break,
            };
            match event {
                GuiEvent::HideAndAck(ack_tx) => {
                    if send_main_to_gui(stream_ref, &MainToGui::HideCandidate).is_err() {
                        stream_guard.take();
                        let _ = ack_tx.send(());
                        break;
                    }
                    if let Ok(Some(GuiToMain::Ack)) =
                        recv_gui_to_main(stream_ref, Some(Duration::from_millis(100)))
                    {}
                    let _ = ack_tx.send(());
                }
                GuiEvent::Exit => {
                    let _ = send_main_to_gui(stream_ref, &MainToGui::Exit);
                    break;
                }
                other => {
                    if let Some(ipc) = gui_event_to_ipc(other) {
                        if send_main_to_gui(stream_ref, &ipc).is_err() {
                            stream_guard.take();
                            break;
                        }
                    }
                }
            }
        }
    });

    // 监控线程：等待子进程退出后清理 socket
    std::thread::spawn(move || {
        child.wait().ok();
        let _ = std::fs::remove_file(&socket_path);
    });
}

/// GUI 不可用时接管 gui_rx。
/// HideAndAck 立即回复 ack，避免 evdev 100ms 超时等待。
#[cfg(target_os = "linux")]
fn spawn_gui_null_handler(gui_rx: std::sync::mpsc::Receiver<GuiEvent>) {
    std::thread::spawn(move || {
        while let Ok(event) = gui_rx.recv() {
            match event {
                GuiEvent::HideAndAck(ack) => {
                    let _ = ack.send(());
                }
                GuiEvent::Exit => break,
                _ => {}
            }
        }
    });
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
