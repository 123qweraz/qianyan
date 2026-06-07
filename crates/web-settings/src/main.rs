use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, mpsc};

use qianyan_ime_core::utils::find_project_root;
use qianyan_ime_core::Config;
use qianyan_ime_ui::tray::TrayEvent;
use qianyan_ime_ui::web::WebServer;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut port = 18765u16;
    let mut control_port = 0u16;
    let mut root: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" if i + 1 < args.len() => {
                port = args[i + 1].parse().unwrap_or(18765);
                i += 2;
            }
            "--control-port" if i + 1 < args.len() => {
                control_port = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--root" if i + 1 < args.len() => {
                root = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => i += 1,
        }
    }

    let root = root.unwrap_or_else(find_project_root);
    let config = Arc::new(RwLock::new(Config::load()));

    let (tray_tx, tray_rx) = mpsc::channel::<TrayEvent>();

    // Forward TrayEvents from web server to parent process via TCP
    if control_port > 0 {
        std::thread::spawn(move || {
            let addr = format!("127.0.0.1:{}", control_port);
            let stream = loop {
                match TcpStream::connect(&addr) {
                    Ok(s) => break s,
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                }
            };
            let mut writer = std::io::BufWriter::new(&stream);
            while let Ok(event) = tray_rx.recv() {
                let value = match event {
                    TrayEvent::ReloadConfig => serde_json::json!({"cmd": "reload_config"}),
                    TrayEvent::ShowNotification(body) => {
                        serde_json::json!({"cmd": "notify", "body": body})
                    }
                    TrayEvent::ClearUserDict(profile) => {
                        let mut v = serde_json::json!({"cmd": "clear_user_dict"});
                        if let Some(p) = profile {
                            v["profile"] = serde_json::json!(p);
                        }
                        v
                    }
                    TrayEvent::SendKey(key) => {
                        serde_json::json!({"cmd": "send_key", "key": key})
                    }
                    _ => continue,
                };
                let json = serde_json::to_string(&value).unwrap_or_default();
                let _ = writer.write_all(json.as_bytes());
                let _ = writer.write_all(b"\n");
                let _ = writer.flush();
            }
        });
    }

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        let server = WebServer::new(
            port,
            Arc::new(std::sync::atomic::AtomicU16::new(port)),
            config,
            Arc::new(RwLock::new(HashMap::new())),
            tray_tx,
            root,
        );
        server.start().await;
    });
}
