use axum::{
    extract::{State, Json, Extension},
    response::IntoResponse,
};
use serde_json::json;
use std::path::PathBuf;
use std::io::BufRead;

use crate::WebState;
use qianyan_ime_core::event::TrayEvent;

static FEATURE_CENTER: std::sync::Mutex<Option<(std::process::Child, u16)>> = std::sync::Mutex::new(None);
static FEATURE_STARTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Launch the feature center subprocess.
/// Can be called from both the web handler and the tray handler.
pub fn launch_feature_center(root: PathBuf, tray_tx: std::sync::mpsc::Sender<TrayEvent>) -> Result<u16, String> {
    if FEATURE_STARTING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err("功能中心正在启动中".to_string());
    }
    let result = launch_feature_center_inner(root, tray_tx);
    FEATURE_STARTING.store(false, std::sync::atomic::Ordering::Release);
    result
}

fn launch_feature_center_inner(root: PathBuf, tray_tx: std::sync::mpsc::Sender<TrayEvent>) -> Result<u16, String> {
    if let Some(port) = check_feature_running() {
        return Ok(port);
    }

    let subprocess_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("qianyan-web-settings")))
        .unwrap_or_else(|| std::path::PathBuf::from("qianyan-web-settings"));

    let reserved = (18766..18866)
        .find_map(|p| {
            std::net::TcpListener::bind(format!("127.0.0.1:{}", p)).ok()
        })
        .ok_or_else(|| "无可用端口 (18766-18865)".to_string())?;
    let feature_port = reserved.local_addr().map_err(|e| format!("{}", e))?.port();

    let control_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("{}", e))?;
    let control_port = control_listener.local_addr()
        .map_err(|e| format!("{}", e))?.port();

    let root_str = root.to_string_lossy().to_string();

    let mut child = std::process::Command::new(&subprocess_path)
        .arg("--port").arg(feature_port.to_string())
        .arg("--control-port").arg(control_port.to_string())
        .arg("--root").arg(&root_str)
        .spawn()
        .map_err(|e| format!("{}", e))?;

    let stream = accept_with_timeout(&control_listener, std::time::Duration::from_secs(10))
        .map_err(|e| {
            let _ = child.kill();
            let _ = child.wait();
            format!("功能中心连接超时: {}", e)
        })?;

    drop(reserved);

    stream.set_read_timeout(Some(std::time::Duration::from_secs(60))).ok();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(&stream);
        for line in reader.lines().flatten() {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                let cmd = msg.get("cmd").and_then(|c| c.as_str());
                match cmd {
                    Some("reload_config") => { let _ = tray_tx.send(TrayEvent::ReloadConfig); }
                    Some("notify") => {
                        if let Some(body) = msg.get("body").and_then(|b| b.as_str()) {
                            let _ = tray_tx.send(TrayEvent::ShowNotification(body.to_string()));
                        }
                    }
                    Some("clear_user_dict") => {
                        let profile = msg.get("profile").and_then(|p| p.as_str()).map(|s| s.to_string());
                        let _ = tray_tx.send(TrayEvent::ClearUserDict(profile));
                    }
                    Some("send_key") => {
                        if let Some(key) = msg.get("key").and_then(|k| k.as_str()) {
                            let _ = tray_tx.send(TrayEvent::SendKey(key.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut guard = FEATURE_CENTER.lock().unwrap();
        if let Some((mut c, _)) = guard.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    });

    *FEATURE_CENTER.lock().unwrap() = Some((child, feature_port));
    Ok(feature_port)
}

fn check_feature_running() -> Option<u16> {
    let mut guard = FEATURE_CENTER.lock().unwrap();
    if let Some((ref mut child, port)) = *guard {
        match child.try_wait() {
            Ok(None) => return Some(port),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    *guard = None;
    None
}

fn accept_with_timeout(listener: &std::net::TcpListener, timeout: std::time::Duration) -> Result<std::net::TcpStream, String> {
    listener.set_nonblocking(true).map_err(|e| format!("非阻塞模式失败: {}", e))?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((s, _)) => return Ok(s),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() > deadline {
                    return Err("超时".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("监听错误: {}", e)),
        }
    }
}

pub async fn feature_start_handler(
    State(state): State<WebState>,
    Extension(root): Extension<PathBuf>,
) -> impl IntoResponse {
    match launch_feature_center(root, state.2.clone()) {
        Ok(port) => Json(json!({"ok": true, "port": port})),
        Err(e) => Json(json!({"ok": false, "error": e})),
    }
}

pub async fn feature_stop_handler() -> impl IntoResponse {
    let mut guard = FEATURE_CENTER.lock().unwrap();
    if let Some((mut child, _)) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
        Json(json!({"ok": true}))
    } else {
        Json(json!({"ok": true, "message": "not running"}))
    }
}

pub fn stop_feature_center() {
    let mut guard = FEATURE_CENTER.lock().unwrap();
    if let Some((mut child, _)) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
