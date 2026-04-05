use shian_ime_ui as ui;

use shian_ime_core::Config;
use shian_ime_engine::Processor;
use shian_ime_linux::{cli, runtime, find_project_root, load_syllables};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::sync::mpsc;
use daemonize::Daemonize;

fn load_punctuation_dict(p: &str) -> HashMap<String, Vec<shian_ime_core::config::PunctuationEntry>> {
// ... (remove load_syllables later in the file)
    let mut m = HashMap::new();
    if let Ok(f) = File::open(p) {
        if let Ok(v) = serde_json::from_reader::<_, Value>(BufReader::new(f)) {
            if let Some(obj) = v.as_object() {
                for (k, val) in obj {
                    if let Some(arr) = val.as_array() {
                        let entries = arr
                            .iter()
                            .filter_map(|item| {
                                let c = item.get("char")?.as_str()?;
                                let d = item.get("desc").and_then(|d| d.as_str()).unwrap_or("");
                                Some(shian_ime_core::config::PunctuationEntry {
                                    char: c.to_string(),
                                    desc: d.to_string(),
                                })
                            })
                            .collect();
                        m.insert(k.clone(), entries);
                    }
                }
            }
        }
    }
    m
}

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

    let args: Vec<String> = env::args().collect();
    let should_daemonize = match cli::handle_startup(&args)? {
        cli::StartupAction::Exit => return Ok(()),
        cli::StartupAction::Continue { should_daemonize } => should_daemonize,
    };

    let root = find_project_root();

    if should_daemonize {
        if let Ok(mut pid_file) = std::fs::File::create("/tmp/rust-ime.pid") {
            use std::io::Write;
            let _ = writeln!(pid_file, "{}", std::process::id());
        }
        let daemonize = Daemonize::new().pid_file("/tmp/rust-ime.pid").start();
        if let Err(e) = daemonize {
            eprintln!("Failed to daemonize: {}", e);
        }
    }

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

    let tray_handle = ui::tray::start_tray(ui::tray::TrayParams {
        active_profile: config.read().map(|c| c.input.default_profile.clone()).unwrap_or_else(|_| "chinese".into()),
        show_status_bar: config.read().map(|c| c.appearance.show_status_bar).unwrap_or(true),
        tx: tray_tx.clone(),
    });

    let app_state = Arc::new(Mutex::new(ui::AppState {
        chinese_enabled: true,
        active_profile: "".into(),
        show_status_bar_pref: config.read().map(|c| c.appearance.show_status_bar).unwrap_or(true),
        show_candidates_pref: config.read().map(|c| c.appearance.show_candidates).unwrap_or(true),
        is_ime_active: true,
        pinyin: "".into(),
        candidates: vec![],
        selected_index: 0,
        status_text: "中".into(),
    }));

    // 创建输入主机，获取 vkbd 引用
    let vkbd_ref = match runtime::create_input_host(
        &args,
        processor.clone(),
        gui_tx.clone(),
        config.clone(),
        tray_tx.clone(),
        app_state.clone(),
    ) {
        Ok((vkbd, run_fn)) => {
            // 保存 run_fn 以便后续执行
            (vkbd, Some(run_fn))
        }
        Err(e) => {
            eprintln!("创建输入主机失败: {}", e);
            (None, None)
        }
    };

    let processor_clone = processor.clone();
    let gui_tx_tray = gui_tx.clone();
    let tray_tx_for_main_loop = tray_tx.clone();
    let config_msg = config.clone();
    let app_state_tray = app_state.clone();
    let vkbd_for_event = vkbd_ref.0;

    std::thread::spawn(move || {
        while let Ok(event) = tray_rx.recv() {
            match event {
                ui::tray::TrayEvent::ToggleIme => {
                    if let Ok(mut p) = processor_clone.lock() {
                        p.toggle();
                        let enabled = p.ctx.session_state.chinese_enabled;
                        let short = p.get_short_display();
                        tray_handle.update(move |t| t.chinese_enabled = enabled);

                        if let Ok(mut state) = app_state_tray.lock() {
                            state.chinese_enabled = enabled;
                            state.status_text = if enabled { short } else { "英".into() };
                            let _ = gui_tx_tray.send(ui::GuiEvent::SyncState(state.clone()));
                        }
                    }
                }
                ui::tray::TrayEvent::NextProfile => {
                    if let Ok(mut p) = processor_clone.lock() {
                        let profile = p.next_profile();
                        let enabled = p.ctx.session_state.chinese_enabled;
                        let short = p.get_short_display();
                        tray_handle.update(move |t| t.active_profile = profile);

                        if let Ok(mut state) = app_state_tray.lock() {
                            state.status_text = if enabled { short } else { "英".into() };
                            state.chinese_enabled = enabled;
                            let _ = gui_tx_tray.send(ui::GuiEvent::SyncState(state.clone()));
                        }
                    }
                }
                ui::tray::TrayEvent::ToggleStatusBar => {
                    let mut new_show = false;
                    if let Ok(mut w) = config_msg.write() {
                        w.appearance.show_status_bar = !w.appearance.show_status_bar;
                        new_show = w.appearance.show_status_bar;
                        let _ = w.save();
                    }
                    tray_handle.update(move |t| t.show_status_bar = new_show);

                    if let Ok(mut state) = app_state_tray.lock() {
                        state.show_status_bar_pref = new_show;
                        let _ = gui_tx_tray.send(ui::GuiEvent::ForceStatusVisible(new_show));
                    }
                }
                ui::tray::TrayEvent::SyncStatus {
                    chinese_enabled,
                    active_profile,
                } => {
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.chinese_enabled = chinese_enabled;
                        state.active_profile = active_profile;
                    }
                }
                ui::tray::TrayEvent::OpenConfig => {
                    let config_web = config_msg.clone();
                    let tray_tx_web = tray_tx_for_main_loop.clone();
                    std::thread::spawn(move || {
                        if let Ok(rt) = tokio::runtime::Runtime::new() {
                            rt.block_on(async {
                                let server = ui::web::WebServer::new(
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
                    let _ = open::that("http://127.0.0.1:18765");
                }
                ui::tray::TrayEvent::ReloadConfig => {
                    let new_conf = Config::load();
                    if let Ok(mut p) = processor_clone.lock() {
                        p.apply_config(&new_conf);
                    }
                    let _ = gui_tx_tray.send(ui::GuiEvent::ApplyConfig(Box::new(new_conf)));
                }
                ui::tray::TrayEvent::ShowNotification(msg) => {
                    if let Ok(mut state) = app_state_tray.lock() {
                        state.status_text = msg;
                        let _ = gui_tx_tray.send(ui::GuiEvent::SyncState(state.clone()));
                    }
                }
                ui::tray::TrayEvent::ClearUserDict => {
                    if let Ok(mut p) = processor_clone.lock() {
                        let profiles = p.ctx.config.list_profiles();
                        for profile in profiles {
                            if let Err(e) = p.ctx.config.clear_user_data(&profile) {
                                eprintln!("清除用户数据失败: {}", e);
                            }
                        }
                    }
                }
                ui::tray::TrayEvent::Exit => std::process::exit(0),
                ui::tray::TrayEvent::SendKey(key) => {
                    if let Some(ref vkbd) = vkbd_for_event {
                        if let Ok(mut vk) = vkbd.lock() {
                            vk.send_key(&key);
                        }
                    }
                }
            }
        }
    });

    // 执行输入主机的 run 函数
    if let Some(run_fn) = vkbd_ref.1 {
        run_fn();
    }

    Ok(())
}
