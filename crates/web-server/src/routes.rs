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
            .route("/api/schemes", get(crate::handlers::list_schemes))
            .route("/api/scheme", post(crate::handlers::upsert_scheme))
            .route("/api/scheme/preview", post(crate::handlers::preview_annotation))
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
            .layer(middleware::from_fn(crate::csrf::csrf_layer))
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
        let app = config_app(state, self.root.clone());

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

/// 构造控制中心的内嵌路由（配置读写 + 方案/词典/数据导入导出 + 静态资源）。
/// 与 start_config 共用，便于测试直接以 tower oneshot 走完整的 HTTP 请求管线。
pub(crate) fn config_app(state: WebState, root: std::path::PathBuf) -> axum::Router {
    Router::new()
        .route("/", get(crate::static_handler::config_index_handler))
        .route("/api/config", get(crate::handlers::get_config).post(crate::handlers::update_config))
        .route("/api/config/reset", post(crate::handlers::reset_config))
        .route("/api/config/reset/{sections}", post(crate::handlers::reset_config_section))
        .route("/api/fonts", get(crate::handlers::list_fonts))
        .route("/api/feature/start", post(crate::subproc::feature_start_handler))
        .route("/api/feature/stop", post(crate::subproc::feature_stop_handler))
        .route("/api/reader/parse", post(crate::handlers::parse_epub_handler))
        // 输入方案：控制中心内嵌服务器需提供方案列表/保存/预览，否则页面无法显示现有方案
        .route("/api/schemes", get(crate::handlers::list_schemes))
        .route("/api/scheme", post(crate::handlers::upsert_scheme))
        .route("/api/scheme/preview", post(crate::handlers::preview_annotation))
        // 词典工具（随控制中心展示）：系统词典 / 用户词典 / 新词发现器所用的端点
        .route("/api/dicts", get(crate::handlers::list_dicts))
        .route("/api/dicts/compile", post(crate::handlers::compile_dicts_handler))
        .route("/api/dicts/reload", post(crate::handlers::reload_dicts))
        .route("/api/dicts/toggle", post(crate::handlers::toggle_dict))
        .route("/api/dicts/create", post(crate::handlers::create_dict_handler))
        .route("/api/dicts/open", post(crate::handlers::open_dicts_dir))
        .route("/api/dict/user/browse", get(crate::handlers::browse_user_dict))
        .route("/api/dict/user/delete", post(crate::handlers::delete_user_dict_entry))
        .route("/api/dict/user/cleanup", post(crate::handlers::cleanup_user_dict))
        .route("/api/dict/user/promote", post(crate::handlers::promote_to_system_dict))
        .route("/api/dict/clear_user", post(crate::handlers::clear_user_dict))
        .route("/api/dict/search", get(crate::handlers::search_dict))
        .route("/api/dict/browse", get(crate::handlers::browse_dict))
        .route("/api/dict/update", post(crate::handlers::update_dict_entry))
        .route("/api/dict/entry/update", post(crate::handlers::update_dict_entry_full))
        .route("/api/dict/entry/delete", post(crate::handlers::delete_dict_entry))
        .route("/api/dict/add", post(crate::handlers::add_dict_entry))
        .route("/api/dict/entry/add", post(crate::handlers::add_dict_entry_full))
        .route("/api/dictionary/chars", get(crate::handlers::get_chars_dict))
        .route("/api/user/export", get(crate::handlers::export_user_data))
        .route("/api/user/import", post(crate::handlers::import_user_data))
        .route("/api/tools/discover", post(crate::handlers::discover_words_file_handler))
        .route("/api/tools/discover/export", post(crate::handlers::export_discovery_handler))
        .route("/api/tools/discover/save", post(crate::handlers::save_discovery_handler))
        .route("/api/tools/discover/download", post(crate::handlers::discover_download_handler))
        .route("/static/*file", get(crate::static_handler::static_handler))
        .fallback(crate::static_handler::config_index_handler)
        .layer(middleware::from_fn(crate::csrf::csrf_layer))
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .layer(Extension(root))
        .with_state(state)
}

#[cfg(test)]
mod config_flow_tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use qianyan_ime_core::{Config, ConfigStore};
    use tower::ServiceExt;

    /// 与 handlers::restore_backup_tests 共享进程级 env 锁；测试体持锁期间串行化。
    fn block_env<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    fn state(config: Arc<ConfigStore>) -> (WebState, std_mpsc::Receiver<TrayEvent>) {
        let (tx, rx) = std_mpsc::channel();
        ((config, Arc::new(std::sync::RwLock::new(HashMap::new())), tx), rx)
    }

    /// 起 web 服务（tower oneshot 走完整 HTTP 管线）→ PUT 配置 → 验证：
    /// 版本号每次变更仅自增一次、控制中心收到 ReloadConfigFromWeb、磁盘落盘同步更新。
    #[test]
    fn update_config_persists_and_bumps_version_once() {
        // 进程级 env 覆盖统一由 TEST_ENV_LOCK 串行化（本测试整个流程持锁）
        let _guard = crate::handlers::test_env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("qianyan_web_flow_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let configs = base.join("configs");
        std::fs::create_dir_all(&configs).unwrap();
        // 进程级 env 覆盖统一交给 block_env 里的 TEST_ENV_LOCK 串行化
        std::env::set_var("QIANYAN_CONFIG_DIR", &configs);

        let mut cfg = Config::default_config();
        cfg.appearance.page_size = 7;
        let (state, rx) = state(Arc::new(ConfigStore::new(cfg.clone())));
        let store = state.0.clone();
        let app = config_app(state, base.clone());
        let initial = store.version();

        // 第一次修改：page_size 7 -> 5
        cfg.appearance.page_size = 5;
        let body = serde_json::to_string(&cfg).unwrap();
        let resp = block_env(async {
            app.clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/api/config")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        });
        assert_eq!(resp.status(), 200);

        // 版本号恰好自增一次（消除双写后的关键断言）
        assert_eq!(store.version(), initial + 1);
        let _ = block_env(async {
            app.clone()
                .oneshot(axum::http::Request::builder().uri("/api/config").body(axum::body::Body::empty()).unwrap())
                .await
                .unwrap()
        });
        // 内存即真相：replace 后 GET 返回新值
        assert_eq!(store.get().appearance.page_size, 5);

        // 控制中心（主进程代理）收到通知事件
        let evt = rx.recv_timeout(std::time::Duration::from_secs(2)).expect("应收到 ReloadConfigFromWeb");
        assert!(matches!(evt, TrayEvent::ReloadConfigFromWeb));

        // 磁盘落盘已更新（appearance.json 内 page_size=5）
        let disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(configs.join("appearance.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(disk["appearance"]["page_size"], 5);

        // 第二次修改仍只自增一次
        cfg.appearance.page_size = 9;
        let body = serde_json::to_string(&cfg).unwrap();
        block_env(async {
            app.oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/config")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
        });
        assert_eq!(store.version(), initial + 2);
        assert_eq!(store.get().appearance.page_size, 9);
    }
}
