use std::collections::HashSet;
use std::path::PathBuf;
use axum::{
    extract::{State, Json},
    response::IntoResponse,
    http::StatusCode,
    http::header,
};
use qianyan_ime_core::event::TrayEvent;

use super::{WebState, atomic_write, user_dict_root, valid_profile_name};
use super::USER_DICT_FILES;

pub(crate) async fn export_user_data() -> impl IntoResponse {
    let mut result = serde_json::Map::new();
    let root = user_dict_root();

    if let Ok(entries) = std::fs::read_dir(&root) {
        for profile_entry in entries.flatten() {
            if !profile_entry.path().is_dir() { continue; }
            let profile = profile_entry.file_name().to_string_lossy().to_string();
            let mut profile_data = serde_json::Map::new();

            for file_name in USER_DICT_FILES {
                let path = profile_entry.path().join(file_name);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                        profile_data.insert(file_name.trim_end_matches(".json").to_string(), value);
                    }
                }
            }
            if !profile_data.is_empty() {
                result.insert(profile, serde_json::Value::Object(profile_data));
            }
        }
    }

    let export = serde_json::json!({
        "version": "1.0",
        "exported_at": timestamp_now(),
        "user_data": result,
    });

    let body = serde_json::to_string_pretty(&export).unwrap_or_default();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"qianyan_user_data_backup.json\""),
        ],
        body,
    )
}

pub(crate) async fn import_user_data(
    State((_, _, tray_tx)): State<WebState>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    let root = user_dict_root();
    let _ = std::fs::create_dir_all(&root);

    if let Some(user_data) = body.get("user_data").and_then(|v| v.as_object()) {
        for (profile, data) in user_data {
            if !valid_profile_name(profile) {
                continue;
            }
            let profile_dir = root.join(profile);
            let _ = std::fs::create_dir_all(&profile_dir);
            if let Some(data_obj) = data.as_object() {
                for (key, value) in data_obj {
                    let file_name = match key.as_str() {
                        "learned" => "learned.json",
                        "long_term" => "long_term.json",
                        "ngram" => "ngrams.json",
                        "order" => "order.json",
                        "usage" => "usage.json",
                        _ => continue,
                    };
                    let path = profile_dir.join(file_name);
                    let content = serde_json::to_string_pretty(value).unwrap_or_default();
                    let _ = atomic_write(&path, &content);
                }
            }
        }
    }

    // 如果是旧格式 {data: {profile: {pinyin: [...]}}} 直接合并到 learned.json
    if let Some(data) = body.get("data").and_then(|v| v.as_object()) {
        for (profile, pinyin_map) in data {
            if !valid_profile_name(profile) {
                continue;
            }
            let profile_dir = root.join(profile);
            let _ = std::fs::create_dir_all(&profile_dir);
            let learned_path = profile_dir.join("learned.json");
            let existing = std::fs::read_to_string(&learned_path).ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|v| v.get("data").cloned());
            let new_data = serde_json::json!({
                "version": "1.0",
                "updated_at": timestamp_now(),
                "data": pinyin_map,
            });
            let merged = if let Some(ref existing_data) = existing {
                merge_json_objects(existing_data.clone(), new_data.get("data").cloned().unwrap_or(serde_json::Value::Null))
            } else {
                serde_json::json!({
                    "version": "1.0",
                    "updated_at": timestamp_now(),
                    "data": pinyin_map,
                })
            };
            let _ = atomic_write(&learned_path, serde_json::to_string_pretty(&merged).unwrap_or_default());
        }
    }

    let _ = tray_tx.send(TrayEvent::ClearUserDict(None));
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

fn merge_json_objects(mut base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    if let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in overlay_obj {
            if let Some(base_v) = base_obj.get_mut(k) {
                if let (Some(base_arr), Some(overlay_arr)) = (base_v.as_array_mut(), v.as_array()) {
                    let mut seen: HashSet<String> = base_arr.iter()
                        .filter_map(|e| e.as_array().and_then(|a| a.first()?.as_str().map(String::from)))
                        .collect();
                    for item in overlay_arr {
                        if let Some(key) = item.as_array().and_then(|a| a.first()?.as_str()) {
                            if !seen.contains(key) {
                                base_arr.push(item.clone());
                                seen.insert(key.to_string());
                            }
                        }
                    }
                } else {
                    *base_v = v.clone();
                }
            } else {
                base_obj.insert(k.clone(), v.clone());
            }
        }
    }
    base
}

// ===== Full Backup / Restore =====

pub(crate) fn configs_root() -> PathBuf {
    // 与引擎 Config::get_config_dir 保持一致的显式覆盖机制（测试/独立部署可用）
    if let Ok(dir) = std::env::var("QIANYAN_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    // 尝试找到 configs/ 目录
    if let Ok(cwd) = std::env::current_dir() {
        let configs = cwd.join("configs");
        if configs.exists() { return configs; }
    }
    PathBuf::from("configs")
}

pub(crate) async fn export_full_backup() -> impl IntoResponse {
    let mut result = serde_json::Map::new();

    // 1. 导出所有配置文件
    let configs = configs_root();
    let mut config_data = serde_json::Map::new();
    if configs.exists() {
        if let Ok(entries) = std::fs::read_dir(&configs) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                            config_data.insert(name, value);
                        }
                    }
                }
            }
        }
    }
    result.insert("config".to_string(), serde_json::Value::Object(config_data));

    // 2. 导出所有用户数据
    let mut user_data = serde_json::Map::new();
    let root = user_dict_root();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for profile_entry in entries.flatten() {
            if !profile_entry.path().is_dir() { continue; }
            let profile = profile_entry.file_name().to_string_lossy().to_string();
            let mut profile_data = serde_json::Map::new();
            for file_name in USER_DICT_FILES {
                let path = profile_entry.path().join(file_name);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                        profile_data.insert(file_name.trim_end_matches(".json").to_string(), value);
                    }
                }
            }
            if !profile_data.is_empty() {
                user_data.insert(profile, serde_json::Value::Object(profile_data));
            }
        }
    }
    result.insert("user_data".to_string(), serde_json::Value::Object(user_data));

    let export = serde_json::json!({
        "version": "1.0",
        "exported_at": timestamp_now(),
        "config": result.get("config").cloned().unwrap_or_default(),
        "user_data": result.get("user_data").cloned().unwrap_or_default(),
    });

    let body = serde_json::to_string_pretty(&export).unwrap_or_default();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"qianyan_full_backup.json\""),
        ],
        body,
    )
}

pub(crate) async fn restore_full_backup(
    State((_, _, tray_tx)): State<WebState>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    // 1. 恢复配置文件（name 走白名单校验，防止 "../../xxx" 逃逸 configs/ 目录）
    if let Some(config_data) = body.get("config").and_then(|v| v.as_object()) {
        let configs = configs_root();
        let _ = std::fs::create_dir_all(&configs);
        for (name, value) in config_data {
            if !valid_profile_name(name) {
                log::warn!("[Restore] 跳过非法配置文件名: {:?}", name);
                continue;
            }
            let path = configs.join(format!("{}.json", name));
            let content = serde_json::to_string_pretty(value).unwrap_or_default();
            let _ = atomic_write(&path, &content);
        }
    }

    // 2. 恢复用户数据（profile 名白名单校验，与 import_user_data 一致）
    if let Some(user_data) = body.get("user_data").and_then(|v| v.as_object()) {
        let root = user_dict_root();
        let _ = std::fs::create_dir_all(&root);
        for (profile, data) in user_data {
            if !valid_profile_name(profile) {
                log::warn!("[Restore] 跳过非法 profile 名: {:?}", profile);
                continue;
            }
            let profile_dir = root.join(profile);
            let _ = std::fs::create_dir_all(&profile_dir);
            if let Some(data_obj) = data.as_object() {
                for (key, value) in data_obj {
                    let file_name = match key.as_str() {
                        "learned" => "learned.json",
                        "long_term" => "long_term.json",
                        "ngram" => "ngrams.json",
                        "order" => "order.json",
                        "usage" => "usage.json",
                        _ => continue,
                    };
                    let path = profile_dir.join(file_name);
                    let content = serde_json::to_string_pretty(value).unwrap_or_default();
                    let _ = atomic_write(&path, &content);
                }
            }
        }
    }

    let _ = tray_tx.send(TrayEvent::ClearUserDict(None));
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

fn timestamp_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = secs / 86400;
    let (y, m, d) = days_to_ymd(days as i64 + 719468);
    let (hh, mm, ss) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let days = days - 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}