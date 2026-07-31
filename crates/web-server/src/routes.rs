use axum::{
    routing::{get, post},
    extract::{DefaultBodyLimit, Extension},
    middleware,
    Router,
};
use std::sync::{Arc, RwLock, Mutex as StdMutex};
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::collections::HashMap;

use crate::{WebServer, WebState, ImeEngineHandle, SESSION_TTL_SECS};
use qianyan_ime_core::event::TrayEvent;

impl WebServer {
    pub async fn start(self) {
        let tray_tx = self.tray_tx.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let state: WebState = (self.config, self.tries, self.tray_tx);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ime_handle = Arc::new(ImeEngineHandle {
            engine: Arc::new(RwLock::new(None)),
            root: self.root,
            sessions: StdMutex::new(HashMap::new()),
            shutdown_tx,
            shutdown_pending: Arc::new(AtomicBool::new(false)),
            last_activity: Arc::new(AtomicU64::new(now)),
        });
        let app = Router::new()
            .route("/", get(crate::static_handler::feature_index_handler))
            .route("/api/config", get(crate::handlers::get_config).post(crate::handlers::update_config))
            .route("/api/config/reset", post(crate::handlers::reset_config))
            .route("/api/config/reset/{sections}", post(crate::handlers::reset_config_section))
            .route("/api/shutdown", post(crate::handlers::shutdown_handler))
            .route("/api/fonts", get(crate::handlers::list_fonts))
            .route("/api/dicts", get(crate::handlers::list_dicts))
            .route("/api/dicts/compile", post(crate::handlers::compile_dicts_handler))
            .route("/api/dicts/reload", post(crate::handlers::reload_dicts))
            .route("/api/dicts/toggle", post(crate::handlers::toggle_dict))
            .route("/api/dicts/create", post(crate::handlers::create_dict_handler))
            .route("/api/dicts/open", post(crate::handlers::open_dicts_dir))
            .route("/api/dict/user/browse", get(crate::handlers::browse_user_dict))
            .route("/api/dict/user/delete", post(crate::handlers::delete_user_dict_entry))
            .route("/api/dictionary/chars", get(crate::handlers::get_chars_dict))
            .route("/api/dict/search", get(crate::handlers::search_dict))
            .route("/api/dict/browse", get(crate::handlers::browse_dict))
            .route("/api/dict/update", post(crate::handlers::update_dict_entry))
            .route("/api/dict/entry/update", post(crate::handlers::update_dict_entry_full))
            .route("/api/dict/entry/delete", post(crate::handlers::delete_dict_entry))
            .route("/api/dict/add", post(crate::handlers::add_dict_entry))
            .route("/api/dict/entry/add", post(crate::handlers::add_dict_entry_full))
            .route("/api/dict/clear_user", post(crate::handlers::clear_user_dict))
            .route("/api/dict/user/cleanup", post(crate::handlers::cleanup_user_dict))
            .route("/api/dict/user/promote", post(crate::handlers::promote_to_system_dict))
            .route("/api/keyboard/send", post(crate::handlers::send_key_handler))
            .route("/api/pinyin/convert", post(crate::handlers::pinyin_convert_handler))
            .route("/api/convert", post(crate::handlers::convert_handler))
            .route("/api/tools/discover", post(crate::handlers::discover_words_file_handler))
            .route("/api/tools/discover/export", post(crate::handlers::export_discovery_handler))
            .route("/api/tools/discover/save", post(crate::handlers::save_discovery_handler))
            .route("/api/tools/discover/download", post(crate::handlers::discover_download_handler))
            .route("/api/ime/search", post(crate::handlers::ime_search_handler))
            .route("/api/ime/session", post(crate::handlers::ime_session_handler))
            .route("/api/ime/key", post(crate::handlers::ime_key_handler))
            .route("/api/user/export", get(crate::handlers::export_user_data))
            .route("/api/user/import", post(crate::handlers::import_user_data))
            .route("/api/backup/full", get(crate::handlers::export_full_backup))
            .route("/api/backup/restore", post(crate::handlers::restore_full_backup))
            .route("/api/reader/parse", post(crate::handlers::parse_epub_handler))
            .route("/static/*file", get(crate::static_handler::static_handler))
            .route("/dicts/*file", get(crate::static_handler::dicts_handler))
            .fallback(crate::static_handler::feature_index_handler)
            .layer(middleware::from_fn(crate::handlers::activity_layer))
            .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
            .layer(Extension(ime_handle.clone()))
            .with_state(state);

        let last_activity = ime_handle.last_activity.clone();
        let shutdown_tx_idle = ime_handle.shutdown_tx.clone();
        let idle_timeout = std::time::Duration::from_secs(300);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let last = last_activity.load(std::sync::atomic::Ordering::Relaxed);
                if last > 0 && now - last > idle_timeout.as_secs() {
                    let _ = shutdown_tx_idle.send(true);
                    break;
                }
            }
        });

        let cleanup_handle = ime_handle.clone();
        let mut cleanup_shutdown_rx = ime_handle.shutdown_tx.subscribe();
        tokio::spawn(async move {
            let interval = std::time::Duration::from_secs(SESSION_TTL_SECS / 2);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        let now = std::time::Instant::now();
                        let ttl = std::time::Duration::from_secs(SESSION_TTL_SECS);
                        if let Ok(mut sessions) = cleanup_handle.sessions.lock() {
                            sessions.retain(|_, s| now.duration_since(s.created) < ttl);
                        }
                    }
                    _ = cleanup_shutdown_rx.changed() => break,
                }
            }
        });

        let mut current_port = self.port;
        loop {
            let addr = format!("127.0.0.1:{}", current_port);
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    self.actual_port.store(current_port, Ordering::SeqCst);
                    println!("[Web] 服务器启动在 http://{}", addr);
                    let _ = tray_tx.send(TrayEvent::FeatureReady(current_port));
                    if let Err(e) = axum::serve(listener, app)
                        .with_graceful_shutdown(async move {
                            shutdown_rx.changed().await.ok();
                        })
                        .await {
                        eprintln!("[Web] Server error: {}", e);
                    }
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    log::warn!("[Web] 端口 {} 已被占用，正在尝试 {}...", current_port, current_port + 1);
                    current_port += 1;
                    if current_port > self.port + 100 {
                        log::error!("[Web] 已尝试 100 个端口均无法启动，退出。");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("[Web] Failed to bind to {}: {}", addr, e);
                    break;
                }
            }
        }
    }

    /// Config-only web server, embedded in main process (no separate binary).
    /// Serves config routes + static assets + feature-center lifecycle endpoints.
    pub async fn start_config(self) {
        let state: WebState = (self.config, self.tries, self.tray_tx);
        let root = self.root.clone();
        let app = Router::new()
            .route("/", get(crate::static_handler::config_index_handler))
            .route("/api/config", get(crate::handlers::get_config).post(crate::handlers::update_config))
            .route("/api/config/reset", post(crate::handlers::reset_config))
            .route("/api/config/reset/{sections}", post(crate::handlers::reset_config_section))
            .route("/api/fonts", get(crate::handlers::list_fonts))
            .route("/api/feature/start", post(crate::subproc::feature_start_handler))
            .route("/api/feature/stop", post(crate::subproc::feature_stop_handler))
            .route("/api/reader/parse", post(crate::handlers::parse_epub_handler))
            .route("/static/*file", get(crate::static_handler::static_handler))
            .fallback(crate::static_handler::config_index_handler)
            .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
            .layer(Extension(root))
            .with_state(state);

        let mut current_port = self.port;
        loop {
            let addr = format!("127.0.0.1:{}", current_port);
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    self.actual_port.store(current_port, Ordering::SeqCst);
                    println!("[Config] 控制中心启动在 http://{}", addr);
                    if let Err(e) = axum::serve(listener, app).await {
                        eprintln!("[Config] Server error: {}", e);
                    }
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    current_port += 1;
                    if current_port > self.port + 100 {
                        log::error!("[Config] 已尝试 100 个端口均无法启动，退出。");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("[Config] Failed to bind to {}: {}", addr, e);
                    break;
                }
            }
        }
    }
}
