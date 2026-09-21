use axum::{
    extract::{State, Json, Extension, Multipart},
    response::{IntoResponse, Response},
    http::{StatusCode, header},
    body::Body,
    middleware,
};
use fst::Streamer;
use qianyan_ime_engine::pipeline::{SearchEngine, SearchQuery as EngineSearchQuery, render_annotation};
use qianyan_ime_engine::processor::FilterMode;
use qianyan_ime_engine::scheme::InputScheme;
use qianyan_ime_core::utils::{load_single_syllables, load_syllable_frequencies};
use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use std::sync::{Arc, OnceLock, Mutex as StdMutex};
use std::sync::atomic::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use qianyan_ime_core::Config;
use qianyan_ime_engine::trie::Trie;
use qianyan_ime_core::event::TrayEvent;
use crate::{WebState, ImeEngineHandle, ImeSession, MAX_IME_SESSIONS, SESSION_TTL_SECS};
use crate::static_handler::{find_dicts_root, safe_join, valid_profile_name};

pub mod epub;
pub(crate) use epub::*;

pub mod transfer;
pub(crate) use transfer::*;

pub mod config;
pub(crate) use config::*;

pub mod schemes;
pub(crate) use schemes::*;

pub mod convert;
pub(crate) use convert::*;

pub mod discovery;
pub(crate) use discovery::*;

pub mod ime;
pub(crate) use ime::*;

pub mod dicts;
pub(crate) use dicts::*;

/// 所有 profile 下的用户词典文件（清空/导出/备份/导入等整组扫描共用）
const USER_DICT_FILES: &[&str] = &[
    "learned.json",
    "long_term.json",
    "ngrams.json",
    "order.json",
    "usage.json",
];

/// 浏览接口中识别的词典类型（键=文件名，值=接口呈现类型）
const USER_DICT_TYPES: &[(&str, &str)] = &[
    ("learned.json", "learned"),
    ("long_term.json", "long_term"),
    ("ngrams.json", "ngram"),
    ("usage.json", "usage"),
];

/// 原子写：临时文件 + fsync + rename 覆盖，进程被杀不会留下半个文件的损坏数据。
/// 供用户词典 / 配置 / 备份恢复等落盘使用。
pub(crate) fn atomic_write(path: &std::path::Path, content: impl AsRef<str>) -> std::io::Result<()> {
    use std::io::Write;
    let content = content.as_ref();
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp_path = PathBuf::from(tmp_os);
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

/// 获取用户词典根目录（兼容新旧路径）
fn user_dict_root() -> std::path::PathBuf {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        std::path::PathBuf::from(config_home)
            .join("qianyan-ime")
            .join("user_data")
    } else if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".config")
            .join("qianyan-ime")
            .join("user_data")
    } else {
        std::path::PathBuf::from("data").join("user_data")
    }
}

/// 读取所有 profile 下的 learned.json，返回扁平化的用户词列表
pub(crate) async fn list_fonts() -> Json<Vec<crate::platform::fonts::FontInfo>> {
    Json(crate::platform::fonts::list_system_fonts())
}



static KNOWN_WORDS_CACHE: OnceLock<StdMutex<Option<HashSet<String>>>> = OnceLock::new();

fn get_known_words_cache() -> &'static StdMutex<Option<HashSet<String>>> {
    KNOWN_WORDS_CACHE.get_or_init(|| StdMutex::new(None))
}

/// 执行发现算法 + 拼音标注
async fn extract_file_bytes(
    mut multipart: Multipart,
) -> Result<Vec<u8>, (StatusCode, String)> {
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let data = field.bytes().await
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("读取文件失败: {e}")))?;
            if !data.is_empty() {
                return Ok(data.to_vec());
            }
        }
    }
    Err((StatusCode::BAD_REQUEST, "未提供文件".into()))
}

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static std::sync::Mutex<()> {
    &TEST_ENV_LOCK
}

#[cfg(test)]
mod restore_backup_tests {
    use super::*;
    use std::sync::{mpsc, RwLock};
    use qianyan_ime_core::ConfigStore;

    fn temp_roots(prefix: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "qianyan_restore_{}_{}",
            prefix,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("configs")).unwrap();
        std::fs::create_dir_all(base.join("qianyan-ime")).unwrap();
        base
    }

    fn state() -> WebState {
        let (tx, _rx) = mpsc::channel();
        (
            Arc::new(ConfigStore::new(Config::default_config())),
            Arc::new(RwLock::new(HashMap::new())),
            tx,
        )
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    /// 恶意 name/profile 不能逃逸 configs/ 与 user_data/；env 覆盖机制一并验证。
    /// 全部放在同一测试内避免并行测试对进程级 env 的竞态。
    #[test]
    fn restore_rejects_path_traversal_names() {
        let _guard = test_env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let base = temp_roots("traversal");
        let configs = base.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", &configs);
        std::env::set_var("XDG_CONFIG_HOME", &base);

        assert_eq!(configs_root(), configs);

        let unique = format!("evil_{}", std::process::id());
        let config_key = format!("../../{}", unique);
        let user_key = format!("../../{}", unique);

        let body = serde_json::json!({
            "config": {
                config_key: {"placeholder": true},
                "appearance": {"key": "value"},
            },
            "user_data": {
                user_key: {
                    "learned": {"ni": [["你", 1]]}
                },
                "normal": {
                    "learned": {"ni": [["你好", 1]]}
                },
            },
        });

        let code = block_on(restore_full_backup(State(state()), Json(body)));
        assert_eq!(code, StatusCode::OK);

        // 恶意 key 不应逃逸：configs/../../{unique}.json 与 user_data_root/../../{unique}
        assert!(!configs.join("..").join("..").join(format!("{unique}.json")).exists());
        let real_user_root = base.join("qianyan-ime").join("user_data");
        assert!(!real_user_root.join("..").join("..").join(&unique).exists());

        // 合法条目应正常恢复
        assert!(configs.join("appearance.json").exists());
        assert!(real_user_root.join("normal").join("learned.json").exists());
    }

    #[test]
    fn valid_profile_name_rejects_traversal() {
        assert!(valid_profile_name("default"));
        assert!(valid_profile_name("my-profile_2"));
        assert!(!valid_profile_name("../evil"));
        assert!(!valid_profile_name("../../evil"));
        assert!(!valid_profile_name("a/b"));
        assert!(!valid_profile_name(""));
        assert!(!valid_profile_name(".."));
    }
}
