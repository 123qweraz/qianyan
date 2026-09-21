use super::*;

pub(crate) async fn get_config(State((config, _, _)): State<WebState>) -> impl IntoResponse {
    Json((*config.get()).clone()).into_response()
}

pub(crate) async fn update_config(
    State((config, _, tray_tx)): State<WebState>,
    Json(new_config): Json<Config>
) -> StatusCode {
    log::info!("update_config: shown_char_categories={:?}", new_config.input.shown_char_categories);
    // 先保存到磁盘，再更新内存（磁盘失败时内存不受影响）
    if let Err(e) = new_config.save() {
        log::error!("Config save failed: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    // 处理自启逻辑变化（基于替换前的内存副本判断）
    let prev = config.get();
    if prev.input.autostart != new_config.input.autostart {
        if new_config.input.autostart {
            let _ = crate::platform::setup_autostart();
        } else {
            let _ = crate::platform::remove_autostart();
        }
    }

    config.replace(new_config);
    let _ = tray_tx.send(TrayEvent::ReloadConfigFromWeb);
    StatusCode::OK
}

pub(crate) async fn reset_config(
    State((config, _, tray_tx)): State<WebState>,
) -> StatusCode {
    let default_conf = Config::default_config();
    config.replace(default_conf.clone());
    if let Err(_e) = default_conf.save() { return StatusCode::INTERNAL_SERVER_ERROR; }
    let _ = tray_tx.send(TrayEvent::ReloadConfigFromWeb);
    StatusCode::OK
}

pub(crate) async fn reset_config_section(
    State((config, _, tray_tx)): State<WebState>,
    axum::extract::Path(sections): axum::extract::Path<String>,
) -> StatusCode {
    let default_conf = Config::default_config();
    let (valid, _) = config.update(|w| {
        for section in sections.split(',') {
            match section.trim() {
                "appearance" => w.appearance = default_conf.appearance.clone(),
                "hotkeys" => w.hotkeys = default_conf.hotkeys.clone(),
                "input" => w.input = default_conf.input.clone(),
                "quickfinals" => {
                    w.enable_quick_finals = default_conf.enable_quick_finals;
                    w.quick_finals = default_conf.quick_finals.clone();
                },
                "punctuations" => {
                    w.punctuations = default_conf.punctuations.clone();
                },
                "layouts" => {
                    w.layouts = default_conf.layouts.clone();
                },
                "linux" => w.linux = default_conf.linux.clone(),
                "files" => w.files = default_conf.files.clone(),
                _ => return false,
            }
        }
        true
    });
    if !valid {
        return StatusCode::BAD_REQUEST;
    }
    let conf = config.get();
    if let Err(_e) = conf.save() { return StatusCode::INTERNAL_SERVER_ERROR; }
    let _ = tray_tx.send(TrayEvent::ReloadConfigFromWeb);
    StatusCode::OK
}

pub(crate) async fn activity_layer(
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if let Some(handle) = request.extensions().get::<Arc<ImeEngineHandle>>() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        handle.last_activity.store(now, Ordering::Relaxed);

        let is_shutdown = request.uri().path() == "/api/shutdown";
        if !is_shutdown {
            handle.shutdown_pending.store(false, Ordering::Relaxed);
        }
    }
    next.run(request).await
}

pub(crate) async fn shutdown_handler(
    Extension(handle): Extension<Arc<ImeEngineHandle>>,
) -> impl IntoResponse {
    handle.shutdown_pending.store(true, Ordering::Relaxed);
    let pending = handle.shutdown_pending.clone();
    let tx = handle.shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if pending.load(Ordering::Relaxed) {
            tx.send(true).ok();
        }
    });
    (StatusCode::OK, "服务器正在关闭...")
}