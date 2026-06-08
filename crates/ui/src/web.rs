use axum::{
    routing::{get, post},
    extract::{State, Json, DefaultBodyLimit, Extension},
    response::{IntoResponse, Html, Response},
    http::{StatusCode, Uri, HeaderName, header},
    body::Body,
    middleware,
    Router,
};
use fst::Streamer;
use qianyan_ime_engine::pipeline::{SearchEngine, SearchQuery as EngineSearchQuery};
use qianyan_ime_engine::processor::FilterMode;
use qianyan_ime_engine::schemes::{ChineseScheme, EnglishScheme, JapaneseScheme, StrokeScheme};
use qianyan_ime_engine::scheme::InputScheme;
use qianyan_ime_core::utils::{load_single_syllables, load_syllable_frequencies};
use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock, OnceLock, Mutex as StdMutex};
use std::sync::atomic::{AtomicU16, AtomicU64, AtomicBool, Ordering};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use qianyan_ime_core::Config;
use qianyan_ime_engine::trie::Trie;
use rust_embed::RustEmbed;
use crate::tray::TrayEvent;

#[derive(RustEmbed)]
#[folder = "../../static/"]
struct Assets;

// Web server implementation for IME configuration
pub struct WebServer {
    pub port: u16,
    pub actual_port: Arc<AtomicU16>,
    pub config: Arc<RwLock<Config>>,
    pub tries: Arc<RwLock<HashMap<String, Trie>>>,
    pub tray_tx: std::sync::mpsc::Sender<TrayEvent>,
    pub root: PathBuf,
}

type WebState = (
    Arc<RwLock<Config>>, 
    Arc<RwLock<HashMap<String, Trie>>>, 
    std::sync::mpsc::Sender<TrayEvent>
);

pub struct ImeEngineHandle {
    pub engine: Arc<RwLock<Option<Arc<SearchEngine>>>>,
    pub root: PathBuf,
    pub(super) sessions: StdMutex<HashMap<String, ImeSession>>,
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub shutdown_pending: Arc<AtomicBool>,
    pub last_activity: Arc<AtomicU64>,
}

pub(super) struct ImeSession {
    processor: qianyan_ime_engine::Processor,
    #[allow(dead_code)]
    created: std::time::Instant,
}

const MAX_IME_SESSIONS: usize = 1000;
const SESSION_TTL_SECS: u64 = 3600;

impl WebServer {
    pub fn new(
        port: u16, 
        actual_port: Arc<AtomicU16>,
        config: Arc<RwLock<Config>>, 
        tries: Arc<RwLock<HashMap<String, Trie>>>,
        tray_tx: std::sync::mpsc::Sender<TrayEvent>,
        root: PathBuf,
    ) -> Self {
        Self { port, actual_port, config, tries, tray_tx, root }
    }

    pub async fn start(self) {
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
            .route("/", get(index_handler))
            .route("/api/config", get(get_config).post(update_config))
            .route("/api/config/reset", post(reset_config))
            .route("/api/config/reset/{sections}", post(reset_config_section))
            .route("/api/shutdown", post(shutdown_handler))
            .route("/api/fonts", get(list_fonts))
            .route("/api/dicts", get(list_dicts))
            .route("/api/dicts/compile", post(compile_dicts_handler))
            .route("/api/dicts/reload", post(reload_dicts))
            .route("/api/dicts/toggle", post(toggle_dict))
            .route("/api/dicts/create", post(create_dict_handler))
            .route("/api/dicts/open", post(open_dicts_dir))
            .route("/api/dict/user/browse", get(browse_user_dict))
            .route("/api/dict/user/delete", post(delete_user_dict_entry))
            .route("/api/dictionary/chars", get(get_chars_dict))
            .route("/api/dict/search", get(search_dict))
            .route("/api/dict/browse", get(browse_dict))
            .route("/api/dict/update", post(update_dict_entry))
            .route("/api/dict/entry/update", post(update_dict_entry_full))
            .route("/api/dict/entry/delete", post(delete_dict_entry))
            .route("/api/dict/add", post(add_dict_entry))
            .route("/api/dict/entry/add", post(add_dict_entry_full))
            .route("/api/dict/clear_user", post(clear_user_dict))
            .route("/api/dict/user/cleanup", post(cleanup_user_dict))
            .route("/api/dict/user/promote", post(promote_to_system_dict))
            .route("/api/keyboard/send", post(send_key_handler))
            .route("/api/pinyin/convert", post(pinyin_convert_handler))
            .route("/api/convert", post(convert_handler))
            .route("/api/tools/discover", post(discover_words_file_handler))
            .route("/api/tools/discover/export", post(export_discovery_handler))
            .route("/api/tools/discover/save", post(save_discovery_handler))
            .route("/api/tools/discover/download", post(discover_download_handler))
            .route("/api/ime/search", post(ime_search_handler))
            .route("/api/ime/session", post(ime_session_handler))
            .route("/api/ime/key", post(ime_key_handler))
            .route("/api/user/export", get(export_user_data))
            .route("/api/user/import", post(import_user_data))
            .route("/api/backup/full", get(export_full_backup))
            .route("/api/backup/restore", post(restore_full_backup))
            .route("/static/*file", get(static_handler))
            .route("/dicts/*file", get(dicts_handler))
            .fallback(index_handler)
            .layer(middleware::from_fn(activity_layer))
            .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
            .layer(Extension(ime_handle))
            .with_state(state);

        let mut current_port = self.port;
        loop {
            let addr = format!("127.0.0.1:{}", current_port);
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    self.actual_port.store(current_port, Ordering::SeqCst);
                    println!("[Web] 服务器启动在 http://{}", addr);
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
}

async fn index_handler() -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => Html(String::from_utf8_lossy(&content.data).to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches("/static/").trim_start_matches("/");

    // 开发模式：优先从磁盘读取，修改后无需重新编译
    let dev_root = find_static_root();
    if let Some(ref dev_root) = dev_root {
        if let Some(safe_path) = safe_join(dev_root, path) {
            if let Ok(content) = std::fs::read(&safe_path) {
                let mime = mime_guess::from_path(&safe_path).first_or_octet_stream();
                let headers = [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (HeaderName::from_static("cache-control"), "no-cache"),
                ];
                return (headers, content).into_response();
            }
        }
    }

    // 生产模式：使用编译时嵌入的资源
    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let headers = [
                (header::CONTENT_TYPE, mime.as_ref()),
                (HeaderName::from_static("cache-control"), "no-cache"),
            ];
            (headers, content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

fn find_static_root() -> Option<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from("static"),
        std::env::current_exe().ok()?.parent()?.join("static"),
    ];
    for p in &candidates {
        if p.is_dir() {
            return Some(p.clone());
        }
    }
    // 从工作目录向上查找最多3层
    if let Ok(mut cwd) = std::env::current_dir() {
        for _ in 0..3 {
            let check = cwd.join("static");
            if check.is_dir() {
                return Some(check);
            }
            cwd.pop();
        }
    }
    None
}

fn find_dicts_root() -> std::path::PathBuf {
    let base_path = std::path::PathBuf::from("dicts");
    if base_path.exists() {
        return base_path;
    }
    if let Ok(mut exe_path) = std::env::current_exe() {
        exe_path.pop();
        for _ in 0..3 {
            let p = exe_path.join("dicts");
            if p.exists() {
                return p;
            }
            exe_path.pop();
        }
    }
    std::path::PathBuf::from("dicts")
}

/// 安全地将用户提供的路径解析到基准目录下。
/// 拒绝绝对路径、`..` 穿越，并通过 canonicalize 验证最终路径在 base 内。
fn safe_join(base: &std::path::Path, user_path: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let p = std::path::Path::new(user_path);
    let base = base.canonicalize().ok()?;

    // 绝对路径：规范化后检查是否在 base 目录下
    if p.is_absolute() {
        let canonical = p.canonicalize().ok()?;
        return if canonical.starts_with(&base) { Some(canonical) } else { None };
    }
    // 相对路径：拒绝 .. 穿越
    for c in p.components() {
        if matches!(c, Component::ParentDir) {
            return None;
        }
    }
    let joined = base.join(p);
    if joined.exists() {
        let canonical = joined.canonicalize().ok()?;
        if canonical.starts_with(&base) {
            return Some(canonical);
        }
        return None;
    }
    // 文件不存在时，验证父目录在 base 内（允许新建/写入）
    let parent = joined.parent()?;
    let parent_canonical = parent.canonicalize().ok()?;
    if parent_canonical.starts_with(&base) {
        Some(joined)
    } else {
        None
    }
}

async fn dicts_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches("/dicts/").trim_start_matches("/");
    
    let base_path = find_dicts_root();
    if let Some(safe_path) = safe_join(&base_path, path) {
        if let Ok(content) = std::fs::read(&safe_path) {
            let mime = mime_guess::from_path(&safe_path).first_or_octet_stream();
            return ([(axum::http::header::CONTENT_TYPE, mime.as_ref())], content).into_response();
        }
    }
    eprintln!("[Web] Dictionary file not found: {} in {:?}", path, base_path);
    (StatusCode::NOT_FOUND, "Dictionary Not Found").into_response()
}

async fn get_config(State((config, _, _)): State<WebState>) -> impl IntoResponse {
    match config.read() {
        Ok(cfg) => Json(cfg.clone()).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "config lock poisoned").into_response(),
    }
}

async fn update_config(
    State((config, _, tray_tx)): State<WebState>,
    Json(new_config): Json<Config>
) -> StatusCode {
    log::info!("update_config: rare_char_mode={:?}", new_config.input.rare_char_mode);
    // 先保存到磁盘，再更新内存（磁盘失败时内存不受影响）
    if let Err(e) = new_config.save() {
        log::error!("Config save failed: {}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    {
        let mut w = match config.write() {
            Ok(w) => w,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
        };

        // 处理自启逻辑变化
        if w.input.autostart != new_config.input.autostart {
            if new_config.input.autostart {
                let _ = crate::platform::setup_autostart();
            } else {
                let _ = crate::platform::remove_autostart();
            }
        }

        *w = new_config.clone();
    }
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

async fn reset_config(
    State((config, _, tray_tx)): State<WebState>,
) -> StatusCode {
    let default_conf = Config::default_config();
    {
        let mut w = match config.write() {
            Ok(w) => w,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
        };
        *w = default_conf.clone();
    }
    if let Err(_e) = default_conf.save() { return StatusCode::INTERNAL_SERVER_ERROR; }
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

async fn reset_config_section(
    State((config, _, tray_tx)): State<WebState>,
    axum::extract::Path(sections): axum::extract::Path<String>,
) -> StatusCode {
    let default_conf = Config::default_config();
    let mut w = match config.write() {
        Ok(w) => w,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };
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
            _ => return StatusCode::BAD_REQUEST,
        }
    }
    if let Err(_e) = w.save() { return StatusCode::INTERNAL_SERVER_ERROR; }
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

async fn activity_layer(
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

async fn shutdown_handler(
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

#[derive(Serialize)]
struct DictFile {
    name: String,
    path: String,
    group: String,
    size: u64,
    entry_count: u64,
    enabled: bool,
}

async fn list_dicts() -> Json<Vec<DictFile>> {
    let mut list = Vec::new();
    let root = "dicts";
    let walker = walkdir::WalkDir::new(root).into_iter();
    
    for entry in walker.filter_map(|e: Result<walkdir::DirEntry, walkdir::Error>| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if filename.ends_with(".json") || filename.ends_with(".json.disabled") {
                // 计算分组名：取 dicts/ 下的一级目录名
                let relative = path.strip_prefix(root).unwrap_or(path);
                let group = relative.components().next()
                    .map(|c: std::path::Component| c.as_os_str().to_string_lossy().to_string())
                    .unwrap_or_else(|| "other".to_string());
                
                let mut dict = process_dict_entry(path.to_path_buf());
                dict.group = group;
                dict.path = relative.to_string_lossy().to_string();
                list.push(dict);
            }
        }
    }
    Json(list)
}

fn process_dict_entry(path: std::path::PathBuf) -> DictFile {
    let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let enabled = !filename.contains(".disabled");
    let metadata = path.metadata();
    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
    
    let mut entry_count = 0;
    if let Ok(f) = std::fs::File::open(&path) {
        if let Ok(json) = serde_json::from_reader::<_, serde_json::Value>(std::io::BufReader::new(f)) {
            if let Some(obj) = json.as_object() {
                for val in obj.values() {
                    if let Some(arr) = val.as_array() { entry_count += arr.len() as u64; } else { entry_count += 1; }
                }
            } else if let Some(arr) = json.as_array() { entry_count = arr.len() as u64; }
        }
    }

    DictFile {
        name: filename,
        path: path.to_string_lossy().to_string(),
        group: String::new(),
        size,
        entry_count,
        enabled,
    }
}

#[derive(serde::Deserialize)]
struct ToggleRequest {
    path: String,
}

async fn toggle_dict(Json(req): Json<ToggleRequest>) -> StatusCode {
    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &req.path) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN,
    };

    if !path.exists() { return StatusCode::NOT_FOUND; }

    let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let new_path = if filename.ends_with(".json") {
        path.with_file_name(format!("{}.disabled", filename))
    } else if filename.ends_with(".json.disabled") {
        path.with_file_name(filename.replace(".json.disabled", ".json"))
    } else {
        return StatusCode::BAD_REQUEST;
    };

    if std::fs::rename(&path, &new_path).is_ok() {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[derive(serde::Deserialize)]
struct CreateDictRequest {
    name: String,
    group: Option<String>,
}

async fn create_dict_handler(Json(req): Json<CreateDictRequest>) -> StatusCode {
    let group = req.group.unwrap_or_else(|| "user".to_string());
    let base = find_dicts_root();
    let dir = base.join(&group);
    if std::fs::create_dir_all(&dir).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    let filename = format!("{}.json", req.name);
    let file_path = dir.join(&filename);
    if file_path.exists() {
        return StatusCode::CONFLICT;
    }
    let empty = serde_json::Value::Object(serde_json::Map::new());
    match std::fs::File::create(&file_path)
        .and_then(|f| serde_json::to_writer_pretty(f, &empty).map_err(std::io::Error::other))
    {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn compile_dicts_handler(State((_, _, tray_tx)): State<WebState>) -> StatusCode {
    let _ = tray_tx.send(TrayEvent::ShowNotification("正在编译词库...".into()));
    if let Ok(mut cache) = get_known_words_cache().lock() {
        *cache = None;
    }
    match qianyan_ime_engine::compiler::check_and_compile_all() {
        Ok(_) => {
            let _ = tray_tx.send(TrayEvent::ShowNotification("词库编译完成".into()));
            // 编译完成后自动重载
            let _ = tray_tx.send(TrayEvent::ReloadConfig);
            StatusCode::OK
        },
        Err(e) => {
            eprintln!("[Web] 词库编译失败: {}", e);
            let _ = tray_tx.send(TrayEvent::ShowNotification(format!("编译失败: {}", e)));
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn reload_dicts(State((_, _, tray_tx)): State<WebState>) -> StatusCode {
    if let Ok(mut cache) = get_known_words_cache().lock() {
        *cache = None;
    }
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

#[derive(Serialize)]
struct SearchResult {
    pinyin: String,
    word: String,
    hint: String,
    stroke_aux: String,
    strokes: String,
    file: String,
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: String,
    ime: Option<String>,
}

async fn search_dict(
    State((_, tries, _)): State<WebState>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Json<Vec<SearchResult>> {
    let mut results = Vec::new();
    let q = query.q.to_lowercase();
    let ime = query.ime.as_deref().unwrap_or("chinese");

    if q.is_empty() {
        return Json(results);
    }

    // 懒加载 trie（mmap，零额外内存）
    let trie = {
        let mut cache = tries.write().unwrap_or_else(|e| e.into_inner());
        if !cache.contains_key(ime) {
            let project_root = qianyan_ime_core::utils::find_project_root();
            let idx = project_root.join(format!("data/{}/trie.index", ime));
            let dat = project_root.join(format!("data/{}/trie.data", ime));
            if idx.exists() && dat.exists() {
                if let Ok(t) = Trie::load(idx, dat, false) {
                    cache.insert(ime.to_string(), t);
                }
            }
        }
        cache.get(ime).cloned()
    };

    let Some(trie) = trie else {
        return Json(results);
    };

    // FST 前缀匹配（拼音搜索），毫秒级
    let hits = trie.search_bfs(&q, 100);
    for tr in hits {
        results.push(SearchResult {
            pinyin: tr.tone.to_string(),
            word: tr.word.to_string(),
            hint: tr.en.to_string(),
            stroke_aux: tr.stroke_aux.to_string(),
            strokes: String::new(),
            file: ime.to_string(),
        });
    }

    Json(results)
}

#[derive(Serialize)]
struct BrowseResult {
    entries: Vec<DictEntryView>,
    total: usize,
    page: usize,
    page_size: usize,
    total_pages: usize,
}

#[derive(Serialize)]
#[derive(Clone)]
struct DictEntryView {
    id: String,
    pinyin: String,
    word: String,
    trad: String,
    en: String,
    tone: String,
    weight: i64,
    stroke_aux: String,
    category: String,
    strokes: String,
}

#[derive(serde::Deserialize)]
struct BrowseQuery {
    file: String,
    page: Option<usize>,
    page_size: Option<usize>,
    search: Option<String>,
    sort_by: Option<String>,
    sort_order: Option<String>,
    search_by: Option<String>,
}

fn strip_tone(s: &str) -> String {
    s.chars().map(|c| match c {
        'ā' | 'á' | 'ǎ' | 'à' => 'a',
        'ē' | 'é' | 'ě' | 'è' => 'e',
        'ī' | 'í' | 'ǐ' | 'ì' => 'i',
        'ō' | 'ó' | 'ǒ' | 'ò' => 'o',
        'ū' | 'ú' | 'ǔ' | 'ù' => 'u',
        'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' => 'ü',
        'Ā' | 'Á' | 'Ǎ' | 'À' => 'A',
        'Ē' | 'É' | 'Ě' | 'È' => 'E',
        'Ī' | 'Í' | 'Ǐ' | 'Ì' => 'I',
        'Ō' | 'Ó' | 'Ǒ' | 'Ò' => 'O',
        'Ū' | 'Ú' | 'Ǔ' | 'Ù' => 'U',
        'Ǖ' | 'Ǘ' | 'Ǚ' | 'Ǜ' => 'Ü',
        _ => c,
    }).collect()
}

static DICT_CACHE: OnceLock<StdMutex<HashMap<String, (std::time::SystemTime, std::sync::Arc<Vec<DictEntryView>>)>>> = OnceLock::new();

fn get_dict_cache() -> &'static StdMutex<HashMap<String, (std::time::SystemTime, std::sync::Arc<Vec<DictEntryView>>)>> {
    DICT_CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn get_cached_dict_entries(path: &std::path::Path) -> std::sync::Arc<Vec<DictEntryView>> {
    let mtime = path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let key = path.to_string_lossy().to_string();
    let mut cache = get_dict_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached_mtime, entries)) = cache.get(&key) {
        if *cached_mtime >= mtime {
            return std::sync::Arc::clone(entries);
        }
    }
    let entries = load_dict_entries(path);
    let arc = std::sync::Arc::new(entries);
    cache.insert(key, (mtime, std::sync::Arc::clone(&arc)));
    arc
}

fn load_dict_entries(path: &std::path::Path) -> Vec<DictEntryView> {
    let mut all = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        if let Ok(json) = serde_json::from_reader::<_, serde_json::Value>(std::io::BufReader::new(f)) {
            if let Some(obj) = json.as_object() {
                for (pinyin, val) in obj {
                    if let Some(arr) = val.as_array() {
                        for v in arr {
                            let word = v.get("char").and_then(|c| c.as_str()).unwrap_or("").to_string();
                            if word.is_empty() { continue; }
                            all.push(DictEntryView {
                                id: format!("{}::{}", pinyin, word),
                                pinyin: pinyin.clone(),
                                word,
                                trad: v.get("trad").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                                en: v.get("en").and_then(|e| e.as_str()).unwrap_or("").to_string(),
                                tone: v.get("tone").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                                weight: v.get("weight").and_then(|w| w.as_i64()).unwrap_or(0),
                                stroke_aux: v.get("stroke_aux").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                                category: v.get("category").and_then(|c| c.as_str()).unwrap_or("").to_string(),
                                strokes: v.get("strokes").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    all
}

async fn browse_dict(axum::extract::Query(query): axum::extract::Query<BrowseQuery>) -> impl IntoResponse {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(10, 500);
    let search = query.search.as_deref().unwrap_or("").to_lowercase();
    let search_by = query.search_by.as_deref().unwrap_or("all");
    let sort_by = query.sort_by.as_deref().unwrap_or("pinyin");
    let sort_order = query.sort_order.as_deref().unwrap_or("asc");

    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &query.file) {
        Some(p) => p,
        None => return (StatusCode::FORBIDDEN, "{}").into_response(),
    };
    if !path.exists() {
        return (StatusCode::NOT_FOUND, "{}").into_response();
    }

    // 全局缓存：每个文件只解析一次 JSON
    let entries_arc = get_cached_dict_entries(&path);
    let all = entries_arc.as_ref();

    // 过滤
    let filtered: Vec<&DictEntryView> = if search.is_empty() {
        all.iter().collect()
    } else {
        let search_plain = strip_tone(&search);
        all.iter().filter(|e| {
            match search_by {
                "pinyin" => strip_tone(&e.pinyin.to_lowercase()).contains(&search_plain),
                "word" => e.word.to_lowercase().contains(&search),
                "en" => e.en.to_lowercase().contains(&search),
                "stroke_aux" => e.stroke_aux.to_lowercase().contains(&search),
                _ => {
                    strip_tone(&e.pinyin.to_lowercase()).contains(&search_plain)
                        || e.word.to_lowercase().contains(&search)
                        || e.en.to_lowercase().contains(&search)
                        || e.stroke_aux.to_lowercase().contains(&search)
                }
            }
        }).collect()
    };

    let total = filtered.len();
    let total_pages = total.div_ceil(page_size).max(1);

    // Clone + sort + paginate
    let mut entries: Vec<DictEntryView> = filtered.into_iter().cloned().collect();
    let asc = sort_order == "asc";
    entries.sort_by(|a, b| {
        let ord = |x: &str, y: &str| x.to_lowercase().cmp(&y.to_lowercase());
        match sort_by {
            "word" => { let o = ord(&a.word, &b.word); if asc { o } else { o.reverse() } }
            "en" => { let o = ord(&a.en, &b.en); if asc { o } else { o.reverse() } }
            "weight" => if asc { a.weight.cmp(&b.weight) } else { b.weight.cmp(&a.weight) },
            "stroke_aux" => { let o = ord(&a.stroke_aux, &b.stroke_aux); if asc { o } else { o.reverse() } }
            _ => { let o = ord(&a.pinyin, &b.pinyin); if asc { o } else { o.reverse() } }
        }
    });

    let start = (page - 1) * page_size;
    let entries: Vec<DictEntryView> = entries.into_iter().skip(start).take(page_size).collect();

    Json(BrowseResult { entries, total, page, page_size, total_pages }).into_response()
}

#[derive(serde::Deserialize)]
struct DeleteEntryRequest {
    pinyin: String,
    word: String,
    file: String,
}

async fn delete_dict_entry(Json(req): Json<DeleteEntryRequest>) -> StatusCode {
    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &req.file) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN,
    };
    if !path.exists() { return StatusCode::NOT_FOUND; }

    let mut data: serde_json::Value = match std::fs::File::open(&path) {
        Ok(f) => serde_json::from_reader(std::io::BufReader::new(f)).unwrap_or(serde_json::Value::Null),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    let mut found = false;
    if let Some(obj) = data.as_object_mut() {
        if let Some(entries) = obj.get_mut(&req.pinyin).and_then(|v| v.as_array_mut()) {
            entries.retain(|entry| {
                if entry.get("char").and_then(|c| c.as_str()) == Some(&req.word) {
                    found = true;
                    false
                } else {
                    true
                }
            });
            if entries.is_empty() {
                obj.remove(&req.pinyin);
            }
        }
    }

    if found {
        if let Ok(f) = std::fs::File::create(path) {
            if serde_json::to_writer_pretty(f, &data).is_ok() {
                return StatusCode::OK;
            }
        }
    }
    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(serde::Deserialize)]
struct UpdateEntryFullRequest {
    pinyin: String,
    word: String,
    file: String,
    en: Option<String>,
    trad: Option<String>,
    tone: Option<String>,
    weight: Option<i64>,
    stroke_aux: Option<String>,
    category: Option<String>,
}

async fn update_dict_entry_full(Json(req): Json<UpdateEntryFullRequest>) -> StatusCode {
    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &req.file) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN,
    };
    if !path.exists() { return StatusCode::NOT_FOUND; }

    let mut data: serde_json::Value = match std::fs::File::open(&path) {
        Ok(f) => serde_json::from_reader(std::io::BufReader::new(f)).unwrap_or(serde_json::Value::Null),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    let mut found = false;
    if let Some(obj) = data.as_object_mut() {
        if let Some(entries) = obj.get_mut(&req.pinyin).and_then(|v| v.as_array_mut()) {
            for entry in entries {
                if entry.get("char").and_then(|c| c.as_str()) == Some(&req.word) {
                    if let Some(en) = &req.en {
                        entry["en"] = serde_json::Value::String(en.clone());
                    }
                    if let Some(trad) = &req.trad {
                        entry["trad"] = serde_json::Value::String(trad.clone());
                    }
                    if let Some(tone) = &req.tone {
                        entry["tone"] = serde_json::Value::String(tone.clone());
                    }
                    if let Some(weight) = req.weight {
                        entry["weight"] = serde_json::Value::Number(weight.into());
                    }
                    if let Some(stroke_aux) = &req.stroke_aux {
                        entry["stroke_aux"] = serde_json::Value::String(stroke_aux.clone());
                    }
                    if let Some(category) = &req.category {
                        entry["category"] = serde_json::Value::String(category.clone());
                    }
                    found = true;
                    break;
                }
            }
        }
    }

    if found {
        if let Ok(f) = std::fs::File::create(path) {
            if serde_json::to_writer_pretty(f, &data).is_ok() {
                return StatusCode::OK;
            }
        }
    }
    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(serde::Deserialize)]
struct AddEntryFullRequest {
    pinyin: String,
    word: String,
    file: String,
    en: Option<String>,
    trad: Option<String>,
    tone: Option<String>,
    weight: Option<i64>,
    stroke_aux: Option<String>,
    category: Option<String>,
}

async fn add_dict_entry_full(Json(req): Json<AddEntryFullRequest>) -> StatusCode {
    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &req.file) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN,
    };
    if !path.exists() { return StatusCode::NOT_FOUND; }

    let mut data: serde_json::Value = match std::fs::File::open(&path) {
        Ok(f) => serde_json::from_reader(std::io::BufReader::new(f)).unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    if let Some(obj) = data.as_object_mut() {
        let entries = obj.entry(req.pinyin.clone()).or_insert(serde_json::Value::Array(Vec::new()));
        if let Some(arr) = entries.as_array_mut() {
            for item in arr.iter() {
                if item.get("char").and_then(|c| c.as_str()) == Some(&req.word) {
                    return StatusCode::CONFLICT;
                }
            }
            let mut new_entry = serde_json::Map::new();
            new_entry.insert("char".to_string(), serde_json::Value::String(req.word));
            if let Some(en) = &req.en { new_entry.insert("en".to_string(), serde_json::Value::String(en.clone())); }
            if let Some(trad) = &req.trad { new_entry.insert("trad".to_string(), serde_json::Value::String(trad.clone())); }
            if let Some(tone) = &req.tone { new_entry.insert("tone".to_string(), serde_json::Value::String(tone.clone())); }
            if let Some(weight) = req.weight { new_entry.insert("weight".to_string(), serde_json::Value::Number(weight.into())); }
            if let Some(stroke_aux) = &req.stroke_aux { new_entry.insert("stroke_aux".to_string(), serde_json::Value::String(stroke_aux.clone())); }
            if let Some(category) = &req.category { new_entry.insert("category".to_string(), serde_json::Value::String(category.clone())); }
            arr.push(serde_json::Value::Object(new_entry));
        }
    }

    if let Ok(f) = std::fs::File::create(path) {
        if serde_json::to_writer_pretty(f, &data).is_ok() {
            return StatusCode::OK;
        }
    }
    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(serde::Deserialize)]
struct UpdateEntryRequest {
    pinyin: String,
    word: String,
    new_hint: String,
    file: String,
}

async fn update_dict_entry(Json(req): Json<UpdateEntryRequest>) -> StatusCode {
    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &req.file) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN,
    };
    if !path.exists() { return StatusCode::NOT_FOUND; }

    let mut data: serde_json::Value = match std::fs::File::open(&path) {
        Ok(f) => serde_json::from_reader(std::io::BufReader::new(f)).unwrap_or(serde_json::Value::Null),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    let mut success = false;
    if let Some(obj) = data.as_object_mut() {
        if let Some(entries) = obj.get_mut(&req.pinyin).and_then(|v| v.as_array_mut()) {
            for entry in entries {
                if entry.get("char").and_then(|c| c.as_str()) == Some(&req.word) {
                    entry["en"] = serde_json::Value::String(req.new_hint.clone());
                    success = true;
                    break;
                }
            }
        }
    }

    if success {
        if let Ok(f) = std::fs::File::create(path) {
            if serde_json::to_writer_pretty(f, &data).is_ok() {
                return StatusCode::OK;
            }
        }
    }

    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(serde::Deserialize)]
struct AddEntryRequest {
    pinyin: String,
    word: String,
    hint: String,
    file: String,
}

async fn add_dict_entry(Json(req): Json<AddEntryRequest>) -> StatusCode {
    let base_path = find_dicts_root();
    let path = match safe_join(&base_path, &req.file) {
        Some(p) => p,
        None => return StatusCode::FORBIDDEN,
    };
    if !path.exists() { return StatusCode::NOT_FOUND; }

    let mut data: serde_json::Value = match std::fs::File::open(&path) {
        Ok(f) => serde_json::from_reader(std::io::BufReader::new(f)).unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    if let Some(obj) = data.as_object_mut() {
        let entries = obj.entry(req.pinyin).or_insert(serde_json::Value::Array(Vec::new()));
        if let Some(arr) = entries.as_array_mut() {
            // 检查是否已存在
            for item in arr.iter() {
                if item.get("char").and_then(|c| c.as_str()) == Some(&req.word) {
                    return StatusCode::CONFLICT;
                }
            }
            let mut new_entry = serde_json::Map::new();
            new_entry.insert("char".to_string(), serde_json::Value::String(req.word));
            new_entry.insert("en".to_string(), serde_json::Value::String(req.hint));
            arr.push(serde_json::Value::Object(new_entry));
        }
    }

    if let Ok(f) = std::fs::File::create(path) {
        if serde_json::to_writer_pretty(f, &data).is_ok() {
            return StatusCode::OK;
        }
    }

    StatusCode::INTERNAL_SERVER_ERROR
}

#[derive(Deserialize)]
struct ClearUserDictRequest {
    profile: Option<String>,
    all: bool,
}

async fn clear_user_dict(
    State((_, _, tray_tx)): State<WebState>,
    Json(req): Json<ClearUserDictRequest>,
) -> StatusCode {
    let root = user_dict_root();

    if req.all && req.profile.is_none() {
        // 清空所有方案的词典
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    for file_name in &["learned.json", "usage.json", "ngrams.json"] {
                        let f = entry.path().join(file_name);
                        if f.exists() {
                            let _ = std::fs::remove_file(&f);
                        }
                    }
                }
            }
        }
        let _ = tray_tx.send(TrayEvent::ClearUserDict(None));
    } else if let Some(ref profile) = req.profile {
        let profile_dir = root.join(profile);
        if profile_dir.exists() {
            for file_name in &["learned.json", "usage.json", "ngrams.json"] {
                let f = profile_dir.join(file_name);
                if f.exists() {
                    let _ = std::fs::remove_file(&f);
                }
            }
        }
        let _ = tray_tx.send(TrayEvent::ClearUserDict(req.profile));
    }

    StatusCode::OK
}

#[derive(Serialize)]
struct CleanupResult {
    profile: String,
    removed: usize,
}

/// 扫描用户词典，移除系统词典已有的词
async fn cleanup_user_dict() -> Json<Vec<CleanupResult>> {
    let root = user_dict_root();
    let mut results = Vec::new();

    let trie = {
        let project_root = qianyan_ime_core::utils::find_project_root();
        let idx = project_root.join("data/chinese/trie.index");
        let dat = project_root.join("data/chinese/trie.data");
        Trie::load(&idx, &dat, false).ok()
    };

    if let (Some(trie), Ok(entries)) = (trie, std::fs::read_dir(&root)) {
        for profile_entry in entries.flatten() {
            if !profile_entry.path().is_dir() { continue; }
            let profile = profile_entry.file_name().to_string_lossy().to_string();
            let learned_path = profile_entry.path().join("learned.json");
            if !learned_path.exists() { continue; }

            let content = match std::fs::read_to_string(&learned_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut data: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let mut removed = 0usize;
            if let Some(obj) = data.as_object_mut() {
                if let Some(data_obj) = obj.get_mut("data").and_then(|d| d.as_object_mut()) {
                    let pinyins: Vec<String> = data_obj.keys().cloned().collect();
                    for pinyin in pinyins {
                        if let Some(entries) = data_obj.get_mut(&pinyin).and_then(|a| a.as_array_mut()) {
                            let before = entries.len();
                            entries.retain(|entry| {
                                let word = entry.as_array()
                                    .and_then(|a| a.first())
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                word.is_empty() || !trie.has_word_in_dict(word)
                            });
                            removed += before - entries.len();
                            if entries.is_empty() {
                                data_obj.remove(&pinyin);
                            }
                        }
                    }
                }
            }

            if removed > 0 {
                let _ = std::fs::write(&learned_path, serde_json::to_string_pretty(&data).unwrap_or_default());
            }
            results.push(CleanupResult { profile, removed });
        }
    }

    Json(results)
}

#[derive(Deserialize)]
struct PromoteRequest {
    words: Vec<PromoteWord>,
}
#[derive(Deserialize)]
struct PromoteWord {
    profile: String,
    pinyin: String,
    word: String,
}
#[derive(Serialize)]
struct PromoteResult {
    added: usize,
    skipped: usize,
}

async fn promote_to_system_dict(
    State((_, _, tray_tx)): State<WebState>,
    Json(req): Json<PromoteRequest>,
) -> Json<PromoteResult> {
    let project_root = qianyan_ime_core::utils::find_project_root();
    let dict_file = project_root.join("dicts/chinese/words/user_promoted.json");

    let mut dict: serde_json::Value = if dict_file.exists() {
        std::fs::read_to_string(&dict_file).ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        if let Some(parent) = dict_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        serde_json::json!({})
    };

    let mut added = 0usize;
    let mut skipped = 0usize;

    for w in &req.words {
        let pinyin = w.pinyin.to_lowercase();
        let obj = match dict.as_object_mut() {
            Some(o) => o,
            None => break,
        };
        let entries = obj.entry(pinyin.clone())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let arr = match entries.as_array_mut() {
            Some(a) => a,
            None => continue,
        };
        if arr.iter().any(|e| e.get("char").and_then(|v| v.as_str()) == Some(&w.word)) {
            skipped += 1; continue;
        }
        arr.push(serde_json::json!({ "char": w.word, "weight": 50000 }));
        added += 1;
    }

    if added > 0 {
        let _ = std::fs::write(&dict_file, serde_json::to_string_pretty(&dict).unwrap_or_default());
    }

    // 从用户词典中删除已提升的词
    for w in &req.words {
        let learned_path = user_dict_root().join(&w.profile).join("learned.json");
        if !learned_path.exists() { continue; }
        if let Ok(content) = std::fs::read_to_string(&learned_path) {
            if let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&content) {
                let pinyin = w.pinyin.to_lowercase();
                let modified = if let Some(data_obj) = data.get_mut("data").and_then(|d| d.as_object_mut()) {
                    if let Some(entries) = data_obj.get_mut(&pinyin).and_then(|a| a.as_array_mut()) {
                        entries.retain(|e| {
                            e.as_array().and_then(|a| a.first().and_then(|v| v.as_str())) != Some(&w.word)
                        });
                        if entries.is_empty() { data_obj.remove(&pinyin); }
                        true
                    } else { false }
                } else { false };
                if modified {
                    let _ = std::fs::write(&learned_path, serde_json::to_string_pretty(&data).unwrap_or_default());
                }
            }
        }
    }

    if added > 0 {
        let _ = tray_tx.send(TrayEvent::ClearUserDict(None));
    }
    Json(PromoteResult { added, skipped })
}

#[derive(Serialize)]
struct UserDictEntryView {
    profile: String,
    pinyin: String,
    word: String,
    weight: u32,
    data_type: String,
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
async fn browse_user_dict() -> Json<Vec<UserDictEntryView>> {
    let mut results = Vec::new();
    let root = user_dict_root();

    fn read_json_file(path: &std::path::Path) -> Option<HashMap<String, Vec<(String, u32)>>> {
        let content = std::fs::read_to_string(path).ok()?;
        // 新版格式带 version/data 字段
        if let Ok(data_file) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&content) {
            if let Some(data) = data_file.get("data") {
                let mut map = HashMap::new();
                if let Some(obj) = data.as_object() {
                    for (key, arr) in obj {
                        let mut words = Vec::new();
                        if let Some(entries) = arr.as_array() {
                            for entry in entries {
                                if let Some(arr) = entry.as_array() {
                                    if arr.len() >= 2 {
                                        let word = arr[0].as_str().unwrap_or("").to_string();
                                        let weight = arr[1].as_u64().unwrap_or(0) as u32;
                                        words.push((word, weight));
                                    }
                                }
                            }
                        }
                        map.insert(key.clone(), words);
                    }
                }
                return Some(map);
            }
        }
        // 旧版格式: {pinyin: [[word, weight]]}
        serde_json::from_str::<HashMap<String, Vec<(String, u32)>>>(&content).ok()
    }

    fn push_results(
        results: &mut Vec<UserDictEntryView>,
        profile: &str,
        map: &HashMap<String, Vec<(String, u32)>>,
        data_type: &str,
    ) {
        for (pinyin, words) in map {
            for (word, weight) in words {
                results.push(UserDictEntryView {
                    profile: profile.to_string(),
                    pinyin: pinyin.clone(),
                    word: word.clone(),
                    weight: *weight,
                    data_type: data_type.to_string(),
                });
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(&root) {
        for profile_entry in entries.flatten() {
            if !profile_entry.path().is_dir() {
                continue;
            }
            let profile = profile_entry.file_name().to_string_lossy().to_string();

            for (file_name, data_type) in &[
                ("learned.json", "learned"),
                ("usage.json", "usage"),
                ("ngrams.json", "ngram"),
            ] {
                let path = profile_entry.path().join(file_name);
                if path.exists() {
                    if let Some(map) = read_json_file(&path) {
                        push_results(&mut results, &profile, &map, data_type);
                    }
                }
            }
        }
    }
    Json(results)
}

#[derive(serde::Deserialize)]
struct DeleteUserDictRequest {
    profile: String,
    pinyin: String,
    word: String,
}

/// 删除用户词典中的一条记录
async fn delete_user_dict_entry(
    State((_, _, tray_tx)): State<WebState>,
    Json(req): Json<DeleteUserDictRequest>,
) -> StatusCode {
    let root = user_dict_root();
    let profile_dir = root.join(&req.profile);

    // 同时从 learned, usage, ngram 三个文件中删除该词条
    let mut deleted = false;
    for file_name in &["learned.json", "usage.json", "ngrams.json"] {
        let path = profile_dir.join(file_name);
        if !path.exists() { continue; }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut data: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(obj) = data.as_object_mut() {
            if let Some(data_obj) = obj.get_mut("data").and_then(|d| d.as_object_mut()) {
                if let Some(entries) = data_obj.get_mut(&req.pinyin).and_then(|a| a.as_array_mut()) {
                    let before = entries.len();
                    entries.retain(|entry| {
                        entry.as_array().map(|a| a.first().and_then(|v| v.as_str())) != Some(Some(&req.word))
                    });
                    if entries.len() < before { deleted = true; }
                    if entries.is_empty() {
                        data_obj.remove(&req.pinyin);
                    }
                }
            }
        }
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&data).unwrap_or_default());
    }

    if !deleted {
        return StatusCode::NOT_FOUND;
    }
    let _ = tray_tx.send(TrayEvent::ClearUserDict(Some(req.profile)));
    StatusCode::OK
}

/// 在文件管理器中打开词典目录
async fn open_dicts_dir() -> StatusCode {
    let path = std::fs::canonicalize("dicts").unwrap_or_else(|_| std::path::PathBuf::from("dicts"));
    if cfg!(target_os = "linux") {
        match std::process::Command::new("xdg-open")
            .arg(path.to_string_lossy().as_ref())
            .spawn()
        {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    } else if cfg!(target_os = "windows") {
        match std::process::Command::new("explorer")
            .arg(path.to_string_lossy().as_ref())
            .spawn()
        {
            Ok(_) => StatusCode::OK,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    } else {
        StatusCode::NOT_IMPLEMENTED
    }
}

async fn list_fonts() -> Json<Vec<crate::platform::fonts::FontInfo>> {
    Json(crate::platform::fonts::list_system_fonts())
}

#[derive(Serialize)]
struct CharEntryView {
    pinyin: String,
    #[serde(rename = "char")]
    character: String,
    en_meaning: String,
    en_aux: String,
    stroke_aux: String,
    group: u32,
}

#[derive(serde::Deserialize)]
struct DictViewQuery {
    file: Option<String>,
}

async fn get_chars_dict(axum::extract::Query(query): axum::extract::Query<DictViewQuery>) -> impl IntoResponse {
    let base_path = find_dicts_root();
    let path = match &query.file {
        Some(f) => match safe_join(&base_path, f) {
            Some(p) => p,
            None => return (StatusCode::FORBIDDEN, "{}").into_response(),
        },
        None => base_path.join("chinese/chars/chars.json"),
    };
    let mut results = Vec::new();
    
    if let Ok(f) = std::fs::File::open(&path) {
        if let Ok(json) = serde_json::from_reader::<_, serde_json::Value>(std::io::BufReader::new(f)) {
            if let Some(obj) = json.as_object() {
                let mut pinyin_sorted: Vec<_> = obj.keys().collect();
                pinyin_sorted.sort();
                
                let mut group_toggle = 0;
                let mut last_pinyin = String::new();
                
                for pinyin in pinyin_sorted {
                    if pinyin != &last_pinyin && !last_pinyin.is_empty() {
                        group_toggle = 1 - group_toggle;
                    }
                    last_pinyin = pinyin.clone();
                    
                    if let Some(entries) = obj.get(pinyin).and_then(|v| v.as_array()) {
                        for entry in entries {
                            let character = entry.get("char").and_then(|v| v.as_str()).unwrap_or("");
                            let en_meaning = entry.get("en").and_then(|v| v.as_str()).unwrap_or("");
                            let stroke_code = entry.get("stroke_aux").and_then(|v| v.as_str()).unwrap_or("");
                            
                            // 英文辅助码：拼音 + 英文前3位 (如果en存在)
                            let en_aux = if !en_meaning.is_empty() {
                                format!("{}{}", pinyin, en_meaning.chars().take(3).collect::<String>())
                            } else {
                                pinyin.clone()
                            };
                            
                            // 笔画辅助码：拼音 + 笔画码 (如果笔画存在)
                            let stroke_aux = if !stroke_code.is_empty() {
                                format!("{}{}", pinyin, stroke_code)
                            } else {
                                pinyin.clone()
                            };
                            
                            results.push(CharEntryView {
                                pinyin: pinyin.clone(),
                                character: character.to_string(),
                                en_meaning: if en_meaning.is_empty() { "-".to_string() } else { en_meaning.to_string() },
                                en_aux,
                                stroke_aux: if stroke_code.is_empty() { "-".to_string() } else { stroke_aux },
                                group: group_toggle,
                            });
                        }
                    }
                }
            }
        }
    }
    Json(results).into_response()
}

#[derive(serde::Deserialize)]
struct SendKeyRequest {
    key: String,
    action: Option<String>,
}

async fn send_key_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SendKeyRequest>
) -> StatusCode {
    // CSRF protection: reject requests with external Origin/Referer
    if let Some(origin) = headers.get("origin")
        .or_else(|| headers.get("referer"))
    {
        if let Ok(host) = origin.to_str() {
            if !host.starts_with("http://127.0.0.1")
                && !host.starts_with("http://localhost")
            {
                return StatusCode::FORBIDDEN;
            }
        }
    }

    let key = req.key.to_lowercase();
    let _action = req.action.unwrap_or_else(|| "tap".to_string());
    
    let tray_tx = state.2.clone();
    let _ = tray_tx.send(TrayEvent::SendKey(key));
    
    StatusCode::OK
}

#[derive(serde::Deserialize)]
struct PinyinConvertRequest {
    text: String,
}

#[derive(Serialize)]
struct PinyinConvertResponse {
    segmented: String,
    pinyin: String,
}

type WordMap = HashMap<String, (String, u32)>;

static CHINESE_WORD_MAP: OnceLock<WordMap> = OnceLock::new();

fn load_chinese_word_map() -> WordMap {
    let root = qianyan_ime_core::utils::find_project_root();
    let data_dir = root.join("data");
    let index_path = data_dir.join("chinese/trie.index");
    let data_path = data_dir.join("chinese/trie.data");

    let trie = match Trie::load(&index_path, &data_path, false) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[pinyin] Failed to load chinese trie: {}", e);
            return HashMap::new();
        }
    };

    let mut map = WordMap::new();
    let mut stream = trie.index.stream();
    while let Some((pinyin_bytes, offset)) = stream.next() {
        let pinyin = String::from_utf8_lossy(pinyin_bytes).to_string();
        trie.read_block(offset as usize, |tr| {
            let word = tr.word.to_string();
            map.entry(word).or_insert_with(|| (pinyin.clone(), tr.weight));
        });
    }
    map
}

fn get_chinese_word_map() -> &'static WordMap {
    CHINESE_WORD_MAP.get_or_init(load_chinese_word_map)
}

fn is_word_in_map(word: &str, map: &WordMap) -> bool {
    map.contains_key(word)
}

fn word_weight(word: &str, map: &WordMap) -> u32 {
    map.get(word).map_or(0, |e| e.1)
}

fn word_pinyin(word: &str, map: &WordMap) -> String {
    map.get(word).map_or_else(|| word.to_string(), |e| e.0.clone())
}

fn max_word_len(map: &WordMap) -> usize {
    map.keys().map(|k| k.chars().count()).max().unwrap_or(6).min(8)
}

fn segment_forward(chars: &[char], map: &WordMap, max_len: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !is_chinese(chars[i]) {
            words.push(chars[i].to_string());
            i += 1;
            continue;
        }
        let lookahead = max_len.min(chars.len() - i);
        let mut found = false;
        for len in (1..=lookahead).rev() {
            let word: String = chars[i..i + len].iter().collect();
            if is_word_in_map(&word, map) {
                words.push(word);
                i += len;
                found = true;
                break;
            }
        }
        if !found {
            words.push(chars[i].to_string());
            i += 1;
        }
    }
    words
}

fn segment_backward(chars: &[char], map: &WordMap, max_len: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = chars.len();
    while i > 0 {
        if !is_chinese(chars[i - 1]) {
            words.push(chars[i - 1].to_string());
            i -= 1;
            continue;
        }
        let lookahead = max_len.min(i);
        let mut found = false;
        for len in (1..=lookahead).rev() {
            let word: String = chars[i - len..i].iter().collect();
            if is_word_in_map(&word, map) {
                words.push(word);
                i -= len;
                found = true;
                break;
            }
        }
        if !found {
            words.push(chars[i - 1].to_string());
            i -= 1;
        }
    }
    words.reverse();
    words
}

fn single_char_count(words: &[String]) -> usize {
    words.iter().filter(|w| w.chars().count() == 1).count()
}

fn total_weight(words: &[String], map: &WordMap) -> u64 {
    words.iter().map(|w| word_weight(w, map) as u64).sum()
}

fn segment_and_convert(
    text: &str,
    map: &WordMap,
) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let max_len = max_word_len(map);

    let forward = segment_forward(&chars, map, max_len);
    let backward = segment_backward(&chars, map, max_len);

    let words = if forward == backward {
        &forward
    } else {
        let f1 = single_char_count(&forward);
        let f2 = single_char_count(&backward);
        if f1 != f2 {
            if f1 < f2 { &forward } else { &backward }
        } else {
            let fw = total_weight(&forward, map);
            let bw = total_weight(&backward, map);
            if fw >= bw { &forward } else { &backward }
        }
    };

    let mut segmented = Vec::new();
    let mut pinyin = Vec::new();
    for word in words {
        segmented.push(word.clone());
        pinyin.push(word_pinyin(word, map));
    }

    (segmented.join(" "), pinyin.join(" "))
}

async fn pinyin_convert_handler(
    State(_): State<WebState>,
    Json(req): Json<PinyinConvertRequest>,
) -> Json<PinyinConvertResponse> {
    let map = get_chinese_word_map();
    if map.is_empty() {
        return Json(PinyinConvertResponse {
            segmented: req.text.clone(),
            pinyin: req.text,
        });
    }
    let (segmented, pinyin) = segment_and_convert(&req.text, map);
    Json(PinyinConvertResponse { segmented, pinyin })
}

fn is_chinese(c: char) -> bool {
    ('\u{4e00}'..='\u{9fa5}').contains(&c) || ('\u{3400}'..='\u{4dbf}').contains(&c)
}

// ===== Simplified ⇄ Traditional conversion =====

type S2TMap = HashMap<String, String>;

static S2T_MAP: OnceLock<S2TMap> = OnceLock::new();
static T2S_MAP: OnceLock<S2TMap> = OnceLock::new();
static MAX_S2T_WORD_LEN: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
static MAX_T2S_WORD_LEN: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

fn load_s2t_t2s_maps() -> (S2TMap, S2TMap, usize, usize) {
    let root = qianyan_ime_core::utils::find_project_root();
    let data_dir = root.join("data");
    let index_path = data_dir.join("chinese/trie.index");
    let data_path = data_dir.join("chinese/trie.data");

    let trie = match Trie::load(&index_path, &data_path, false) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[s2t] Failed to load chinese trie: {}", e);
            return (HashMap::new(), HashMap::new(), 0, 0);
        }
    };

    let mut s2t: S2TMap = HashMap::new();
    let mut t2s: S2TMap = HashMap::new();
    let mut s2t_max = 0usize;
    let mut t2s_max = 0usize;

    let mut stream = trie.index.stream();
    while let Some((_pinyin_bytes, offset)) = stream.next() {
        trie.read_block(offset as usize, |tr| {
            let word = tr.word.to_string();
            let trad = tr.trad.to_string();
            if !trad.is_empty() && word != trad {
                let wc = word.chars().count();
                if wc > s2t_max { s2t_max = wc; }
                // For s2t: prefer longer match, then higher weight
                s2t.entry(word.clone())
                    .and_modify(|existing| {
                        let existing_len = existing.chars().count();
                        if wc > existing_len {
                            *existing = trad.clone();
                        }
                    })
                    .or_insert_with(|| trad.clone());

                let tc = trad.chars().count();
                if tc > t2s_max { t2s_max = tc; }
                t2s.entry(trad)
                    .and_modify(|existing| {
                        let existing_len = existing.chars().count();
                        if tc > existing_len {
                            *existing = word.clone();
                        }
                    })
                    .or_insert_with(|| word.clone());
            }
        });
    }

    (s2t, t2s, s2t_max, t2s_max)
}

fn get_s2t_map() -> &'static S2TMap { S2T_MAP.get_or_init(|| load_s2t_t2s_maps().0) }
fn get_t2s_map() -> &'static S2TMap { T2S_MAP.get_or_init(|| load_s2t_t2s_maps().1) }
fn get_s2t_max_len() -> usize { *MAX_S2T_WORD_LEN.get_or_init(|| load_s2t_t2s_maps().2) }
fn get_t2s_max_len() -> usize { *MAX_T2S_WORD_LEN.get_or_init(|| load_s2t_t2s_maps().3) }
fn ensure_s2t_loaded() { let _ = get_s2t_map(); let _ = get_t2s_map(); }

fn convert_longest_match(text: &str, map: &S2TMap, max_len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len() * 2);
    let mut i = 0;
    while i < chars.len() {
        let mut best_len = 0usize;
        let mut best_word = None;
        let max_check = max_len.min(chars.len() - i);
        for len in (1..=max_check).rev() {
            let slice: String = chars[i..i + len].iter().collect();
            if let Some(converted) = map.get(&slice) {
                best_len = len;
                best_word = Some(converted.clone());
                break;
            }
        }
        if best_len > 0 {
            result.push_str(&best_word.expect("best_word must be Some when best_len > 0"));
            i += best_len;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[derive(Deserialize)]
struct ConvertRequest {
    text: String,
    #[serde(default = "default_convert_mode")]
    mode: String,
}

fn default_convert_mode() -> String { "pinyin".into() }

#[derive(Serialize)]
struct ConvertResponse {
    result: String,
    segmented: Option<String>,
    pinyin: Option<String>,
}

fn cn_punct_to_en(c: char) -> Option<char> {
    match c {
        '，' => Some(','),
        '。' => Some('.'),
        '！' => Some('!'),
        '？' => Some('?'),
        '；' => Some(';'),
        '：' => Some(':'),
        '\u{201c}' | '\u{201d}' => Some('"'),
        '\u{2018}' | '\u{2019}' => Some('\''),
        '（' => Some('('),
        '）' => Some(')'),
        '【' => Some('['),
        '】' => Some(']'),
        '《' => Some('<'),
        '》' => Some('>'),
        '、' => Some(','),
        '～' => Some('~'),
        '…' => Some('.'),
        '—' => Some('-'),
        _ => None,
    }
}

fn segment_and_convert_pinyin(text: &str, map: &WordMap) -> (String, String) {
    let chars: Vec<char> = text.chars().collect();
    let max_len = max_word_len(map);
    let mut segmented = Vec::new();
    let mut pinyin_parts = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        if !is_chinese(chars[i]) {
            // Punctuation / non-Chinese: keep in segmented, convert for pinyin
            segmented.push(chars[i].to_string());
            if let Some(en) = cn_punct_to_en(chars[i]) {
                pinyin_parts.push(en.to_string());
            } else if chars[i].is_whitespace() {
                pinyin_parts.push(" ".to_string());
            } else {
                pinyin_parts.push(chars[i].to_string());
            }
            i += 1;
            continue;
        }

        let mut best_len = 0usize;
        let max_check = max_len.min(chars.len() - i);
        for len in (1..=max_check).rev() {
            let slice: String = chars[i..i + len].iter().collect();
            if is_word_in_map(&slice, map) {
                best_len = len;
                break;
            }
        }
        if best_len == 0 {
            best_len = 1;
        }

        let word: String = chars[i..i + best_len].iter().collect();
        let py = word_pinyin(&word, map);
        segmented.push(word);
        pinyin_parts.push(py);
        i += best_len;
    }

    // Compact: no space before punctuation, space after/between
    let mut pinyin_compact = String::new();
    for (i, part) in pinyin_parts.iter().enumerate() {
        if i > 0 {
            let curr_is_punct = part.chars().all(|c| !c.is_ascii_alphanumeric());
            if !curr_is_punct {
                pinyin_compact.push(' ');
            }
        }
        pinyin_compact.push_str(part);
    }

    (segmented.join(" "), pinyin_compact)
}

async fn convert_handler(
    State(_): State<WebState>,
    Json(req): Json<ConvertRequest>,
) -> Json<ConvertResponse> {
    match req.mode.as_str() {
        "s2t" => {
            ensure_s2t_loaded();
            let map = get_s2t_map();
            let max_len = get_s2t_max_len();
            Json(ConvertResponse { result: convert_longest_match(&req.text, map, max_len), segmented: None, pinyin: None })
        }
        "t2s" => {
            ensure_s2t_loaded();
            let map = get_t2s_map();
            let max_len = get_t2s_max_len();
            Json(ConvertResponse { result: convert_longest_match(&req.text, map, max_len), segmented: None, pinyin: None })
        }
        _ => {
            let map = get_chinese_word_map();
            if map.is_empty() {
                return Json(ConvertResponse { result: req.text.clone(), segmented: None, pinyin: None });
            }
            let (segmented, pinyin) = segment_and_convert_pinyin(&req.text, map);
            Json(ConvertResponse { result: String::new(), segmented: Some(segmented), pinyin: Some(pinyin) })
        }
    }
}

use axum::extract::Multipart;
use encoding_rs::GBK;

#[derive(Deserialize)]
struct DiscoverConfigMsg {
    min_count: Option<usize>,
    min_pmi: Option<f64>,
    min_entropy: Option<f64>,
    max_word_len: Option<usize>,
}

#[derive(Serialize)]
struct DiscoverResult {
    word: String,
    pinyin: String,
    in_dict: bool,
    weight: u32,
}

/// 新词发现器：处理上传的文件
/// 从 multipart 中提取文本、配置和导出参数
struct DiscoverDownloadParams {
    text: String,
    config: qianyan_ime_engine::pipeline::DiscoveryConfig,
    format: String,
    only_new: bool,
}

async fn extract_download_params(
    mut multipart: Multipart,
) -> Result<DiscoverDownloadParams, (StatusCode, String)> {
    let mut text = String::new();
    let mut config = qianyan_ime_engine::pipeline::DiscoveryConfig::default();
    let mut format = String::from("txt");
    let mut only_new = false;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, format!("Read file error: {e}")))?;
            if let Ok(s) = String::from_utf8(data.to_vec()) {
                text = s;
            } else {
                let (res, _, has_errors) = GBK.decode(&data);
                if !has_errors {
                    text = res.into_owned();
                } else {
                    text = String::from_utf8_lossy(&data).into_owned();
                }
            }
        } else if name == "config" {
            if let Ok(config_bytes) = field.bytes().await {
                if let Ok(cfg_msg) = serde_json::from_slice::<DiscoverConfigMsg>(&config_bytes) {
                    if let Some(v) = cfg_msg.min_count { config.min_count = v; }
                    if let Some(v) = cfg_msg.min_pmi { config.min_pmi = v; }
                    if let Some(v) = cfg_msg.min_entropy { config.min_entropy = v; }
                    if let Some(v) = cfg_msg.max_word_len { config.max_word_len = v; }
                }
            }
        } else if name == "format" {
            if let Ok(b) = field.bytes().await {
                let s = String::from_utf8_lossy(&b).to_string();
                if s == "json" { format = s; }
            }
        } else if name == "only_new" {
            if let Ok(b) = field.bytes().await {
                let s = String::from_utf8_lossy(&b).to_string();
                only_new = s == "true";
            }
        }
    }

    if text.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No text provided".into()));
    }
    Ok(DiscoverDownloadParams { text, config, format, only_new })
}

/// 从 multipart 中提取文本和配置（用于 JSON 端点）
async fn extract_text_and_config(
    mut multipart: Multipart,
) -> Result<(String, qianyan_ime_engine::pipeline::DiscoveryConfig), (StatusCode, String)> {
    let mut text = String::new();
    let mut config = qianyan_ime_engine::pipeline::DiscoveryConfig::default();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            let data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, format!("Read file error: {e}")))?;
            if let Ok(s) = String::from_utf8(data.to_vec()) {
                text = s;
            } else {
                let (res, _, has_errors) = GBK.decode(&data);
                if !has_errors {
                    text = res.into_owned();
                } else {
                    text = String::from_utf8_lossy(&data).into_owned();
                }
            }
        } else if name == "config" {
            if let Ok(config_bytes) = field.bytes().await {
                if let Ok(cfg_msg) = serde_json::from_slice::<DiscoverConfigMsg>(&config_bytes) {
                    if let Some(v) = cfg_msg.min_count { config.min_count = v; }
                    if let Some(v) = cfg_msg.min_pmi { config.min_pmi = v; }
                    if let Some(v) = cfg_msg.min_entropy { config.min_entropy = v; }
                    if let Some(v) = cfg_msg.max_word_len { config.max_word_len = v; }
                }
            }
        }
    }

    if text.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No text provided".into()));
    }
    Ok((text, config))
}

static KNOWN_WORDS_CACHE: OnceLock<StdMutex<Option<HashSet<String>>>> = OnceLock::new();

fn get_known_words_cache() -> &'static StdMutex<Option<HashSet<String>>> {
    KNOWN_WORDS_CACHE.get_or_init(|| StdMutex::new(None))
}

/// 执行发现算法 + 拼音标注
fn do_discovery(
    text: &str,
    config: &qianyan_ime_engine::pipeline::DiscoveryConfig,
    tries: &HashMap<String, Trie>,
    word_map: &WordMap,
) -> Vec<DiscoveredWordWithPinyin> {
    let cache_mutex = get_known_words_cache();
    let mut cache_guard = cache_mutex.lock()
        .unwrap_or_else(|e| e.into_inner());
    
    if cache_guard.is_none() {
        log::info!("[discover] Building global known_words index...");
        let mut set = HashSet::new();
        for trie in tries.values() {
            let mut stream = trie.index.stream();
            while let Some((_, offset)) = stream.next() {
                trie.read_block(offset as usize, |tr| {
                    set.insert(tr.word.to_string());
                });
            }
        }
        log::info!("[discover] Index built with {} words", set.len());
        *cache_guard = Some(set);
    }
    
    let known_words = cache_guard.as_ref()
        .expect("known_words cache should be initialized");
    let results = qianyan_ime_engine::pipeline::discover_words(text, config, known_words);

    results.into_iter().map(|dw| {
        let (pinyin, in_dict) = if let Some((py, _)) = word_map.get(&dw.word) {
            (py.clone(), true)
        } else {
            let mut chars_py = Vec::new();
            for c in dw.word.chars() {
                let s = c.to_string();
                if let Some((py, _)) = word_map.get(&s) {
                    chars_py.push(py.clone());
                } else {
                    chars_py.push(s);
                }
            }
            (chars_py.join(" "), false)
        };
        DiscoveredWordWithPinyin {
            word: dw.word,
            pinyin,
            in_dict,
            count: dw.count,
            pmi: dw.pmi,
            entropy: dw.entropy,
        }
    }).collect()
}

async fn discover_words_file_handler(
    State((_, tries, _)): State<WebState>,
    multipart: Multipart,
) -> impl IntoResponse {
    log::info!("[discover] JSON discovery request received");
    let (text, config) = match extract_text_and_config(multipart).await {
        Ok(v) => v,
        Err((code, msg)) => return (code, msg).into_response(),
    };

    let tries = match tries.read() {
        Ok(m) => m.clone(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read tries").into_response(),
    };
    let tries_clone = tries.clone();
    let text_clone = text.clone();
    let config_clone = config.clone();
    let word_map = get_chinese_word_map();

    log::info!("[discover] starting discovery (text length: {} chars)", text.chars().count());
    let results = match tokio::task::spawn_blocking(move || {
        do_discovery(&text_clone, &config_clone, &tries_clone, word_map)
    }).await {
        Ok(res) => res,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Discovery task panicked").into_response(),
    };
    log::info!("[discover] found {} candidate words", results.len());

    let response: Vec<DiscoverResult> = results.into_iter().map(|dw| DiscoverResult {
        word: dw.word,
        pinyin: dw.pinyin,
        in_dict: dw.in_dict,
        weight: dw.count as u32,
    }).collect();

    log::info!("[discover] returning {} results", response.len());
    Json(response).into_response()
}

async fn discover_download_handler(
    State((_, tries, _)): State<WebState>,
    multipart: Multipart,
) -> impl IntoResponse {
    log::info!("[discover] download discovery request received");
    let params = match extract_download_params(multipart).await {
        Ok(v) => v,
        Err((_code, msg)) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };

    let tries = match tries.read() {
        Ok(m) => m.clone(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read tries").into_response(),
    };
    let tries_clone = tries.clone();
    let text = params.text.clone();
    let config = params.config.clone();
    let word_map = get_chinese_word_map();

    let mut results = match tokio::task::spawn_blocking(move || {
        do_discovery(&text, &config, &tries_clone, word_map)
    }).await {
        Ok(res) => res,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Discovery task panicked").into_response(),
    };

    log::info!("[discover] found {} candidate words, generating download", results.len());

    if params.only_new {
        results.retain(|w| !w.in_dict);
    }

    let (content_type, filename, body) = if params.format == "json" {
        let json_body = serde_json::to_string_pretty(&results).unwrap_or_default();
        ("application/json; charset=utf-8".to_string(), "discovered_words.json".to_string(), json_body)
    } else {
        let mut lines = Vec::with_capacity(results.len() + 1);
        lines.push("词\t拼音\t词频\t凝聚度\t自由度".to_string());
        for dw in &results {
            lines.push(format!("{}\t{}\t{}\t{:.2}\t{:.2}",
                dw.word, dw.pinyin, dw.count, dw.pmi, dw.entropy));
        }
        ("text/plain; charset=utf-8".to_string(), "discovered_words.txt".to_string(), lines.join("\n"))
    };

    let headers = [
        (header::CONTENT_TYPE, content_type.as_str()),
        (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{}\"", filename)),
    ];
    (StatusCode::OK, headers, body).into_response()
}

#[derive(Serialize)]
struct DiscoveredWordWithPinyin {
    word: String,
    pinyin: String,
    in_dict: bool,
    count: usize,
    pmi: f64,
    entropy: f64,
}

#[derive(Deserialize)]
struct ExportDiscoveryRequest {
    words: Vec<String>,
    pinyins: Vec<String>,
    weights: Vec<u32>,
    format: String,
}

/// 保存发现的新词到服务器
async fn save_discovery_handler(
    State(_): State<WebState>,
    Json(req): Json<ExportDiscoveryRequest>,
) -> impl IntoResponse {
    let dict_root = find_dicts_root();
    let save_path = dict_root.join("chinese").join("words").join("discovered.json");
    
    // 确保目录存在
    if let Some(parent) = save_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut data: HashMap<String, Vec<serde_json::Value>> = if save_path.exists() {
        match std::fs::File::open(&save_path) {
            Ok(f) => serde_json::from_reader(std::io::BufReader::new(f)).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    } else {
        HashMap::new()
    };

    for ((word, pinyin), weight) in req.words.iter().zip(req.pinyins.iter()).zip(req.weights.iter()) {
        let entries = data.entry(pinyin.clone()).or_default();
        // 检查是否已存在
        let mut exists = false;
        for entry in entries.iter() {
            if entry.get("char").and_then(|c| c.as_str()) == Some(word) {
                exists = true;
                break;
            }
        }
        if !exists {
            entries.push(serde_json::json!({
                "char": word,
                "weight": weight
            }));
        }
    }

    match std::fs::File::create(&save_path) {
        Ok(f) => {
            if let Err(e) = serde_json::to_writer_pretty(std::io::BufWriter::new(f), &data) {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to write JSON: {}", e)).into_response();
            }
            StatusCode::OK.into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create file: {}", e)).into_response(),
    }
}

/// 导出发现的新词
async fn export_discovery_handler(
    State(_): State<WebState>,
    Json(req): Json<ExportDiscoveryRequest>,
) -> impl IntoResponse {
    if req.format == "txt" {
        let lines: Vec<String> = req.words.iter().zip(req.pinyins.iter())
            .map(|(w, p)| format!("{}\t{}", w, p))
            .collect();
        let body = lines.join("\n");
        let headers = [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"discovered_words.txt\""),
        ];
        (StatusCode::OK, headers, body).into_response()
    } else {
        let mut data: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        for ((word, pinyin), weight) in req.words.iter().zip(req.pinyins.iter()).zip(req.weights.iter()) {
            data.entry(pinyin.clone())
                .or_default()
                .push(serde_json::json!({
                    "char": word,
                    "weight": weight
                }));
        }
        
        let body = serde_json::to_string_pretty(&data).unwrap_or_default();
        let headers = [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"discovered_words.json\""),
        ];
        (StatusCode::OK, headers, body).into_response()
    }
}

fn prepare_ime_engine(root: &std::path::Path) -> Result<SearchEngine, String> {
    let syllable_freq = load_syllable_frequencies(root);

    let data_dir = root.join("data");
    let mut trie_paths: HashMap<String, (PathBuf, PathBuf)> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                    let trie_idx = path.join("trie.index");
                    let trie_dat = path.join("trie.data");
                    if trie_idx.exists() && trie_dat.exists() {
                        trie_paths.insert(dir_name.to_string(), (trie_idx, trie_dat));
                    }
                }
            }
        }
    }

    if trie_paths.is_empty() {
        return Err("No trie files found in data/ directory".into());
    }

    let syllable_freq_arc = Arc::new(syllable_freq);
    let empty_user_dict = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())));

    let mut schemes_map: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
    schemes_map.insert("chinese".into(), Box::new(ChineseScheme::new()));
    schemes_map.insert("english".into(), Box::new(EnglishScheme::new()));
    schemes_map.insert("japanese".into(), Box::new(JapaneseScheme::new()));
    schemes_map.insert("stroke".into(), Box::new(StrokeScheme::new()));

    let single_syllables = Arc::new(load_single_syllables(root));
    let mut engine = SearchEngine::new(
        trie_paths,
        syllable_freq_arc,
        empty_user_dict,
        Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
        Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<String>>>::new()))),
        Arc::new(schemes_map),
    );
    engine.single_syllables = single_syllables;
    Ok(engine)
}

#[derive(Deserialize)]
struct ImeSearchRequest {
    buffer: String,
    #[serde(default = "default_profile")]
    profile: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    aux_filter: String,
    #[serde(default = "default_filter_mode")]
    filter_mode: String,
}

fn default_profile() -> String { "chinese".into() }
fn default_limit() -> usize { 500 }
fn default_filter_mode() -> String { "none".into() }

#[derive(Serialize)]
struct ImeCandidateResponse {
    text: String,
    simplified: String,
    traditional: String,
    hint: String,
    weight: f64,
    match_level: u8,
    source: String,
}

#[derive(Serialize)]
struct ImeSearchResponse {
    candidates: Vec<ImeCandidateResponse>,
    segments: Vec<String>,
}

async fn ime_search_handler(
    State((config, _, _)): State<WebState>,
    Extension(ime_handle): Extension<Arc<ImeEngineHandle>>,
    Json(req): Json<ImeSearchRequest>,
) -> Json<ImeSearchResponse> {
    let engine = {
        let guard = ime_handle.engine.read()
            .expect("ime engine RwLock poisoned");
        if let Some(ref engine) = *guard {
            engine.clone()
        } else {
            drop(guard);
            match prepare_ime_engine(&ime_handle.root) {
                Ok(engine) => {
                    let mut w = ime_handle.engine.write()
                        .expect("ime engine RwLock poisoned");
                    if w.is_none() {
                        *w = Some(Arc::new(engine.clone()));
                    }
                    Arc::new(engine)
                }
                Err(e) => {
                    log::error!("[Web] Failed to init IME engine: {}", e);
                    return Json(ImeSearchResponse { candidates: vec![], segments: vec![] });
                }
            }
        }
    };

    let cfg = config.read()
        .expect("config RwLock poisoned")
        .clone();
    let fm = match req.filter_mode.as_str() {
        "global" => FilterMode::Global,
        "page" => FilterMode::Page,
        _ => FilterMode::None,
    };
    let (candidates, segments) = engine.search(EngineSearchQuery {
        buffer: &req.buffer,
        profile: &req.profile,
        config: &cfg,
        limit: req.limit,
        filter_mode: fm,
        aux_filter: &req.aux_filter,
        context: None,
        fuzzy_enabled: cfg.input.enable_fuzzy_pinyin,
    });

    let response_candidates: Vec<ImeCandidateResponse> = candidates
        .into_iter()
        .map(|c| ImeCandidateResponse {
            text: c.text.to_string(),
            simplified: c.simplified.to_string(),
            traditional: c.traditional.to_string(),
            hint: c.hint.to_string(),
            weight: c.weight,
            match_level: c.match_level,
            source: c.source.to_string(),
        })
        .collect();

    Json(ImeSearchResponse {
        candidates: response_candidates,
        segments,
    })
}

// ===== Session-based IME (thin client: key events in, state out) =====

fn ensure_engine(handle: &ImeEngineHandle) -> Option<Arc<SearchEngine>> {
    let guard = handle.engine.read()
        .expect("ime engine RwLock poisoned");
    if let Some(ref engine) = *guard {
        return Some(engine.clone());
    }
    drop(guard);
    match prepare_ime_engine(&handle.root) {
        Ok(engine) => {
            let mut w = handle.engine.write()
                .expect("ime engine RwLock poisoned");
            if w.is_none() {
                *w = Some(Arc::new(engine.clone()));
            }
            Some(Arc::new(engine))
        }
        Err(e) => {
            log::error!("[Web] Failed to init IME engine: {}", e);
            None
        }
    }
}

fn create_processor(
    _root: &std::path::Path,
    engine: Arc<SearchEngine>,
) -> Option<qianyan_ime_engine::Processor> {
    Some(qianyan_ime_engine::Processor::new_with_engine(
        (*engine).clone(),
    ))
}

#[derive(Serialize)]
struct ImeSessionResponse {
    session_id: String,
    pinyin_display: String,
    candidates: Vec<ImeCandidateResponse>,
    segments: Vec<String>,
    filter_active: bool,
    chinese_enabled: bool,
    action: Option<ImeActionResponse>,
}

#[derive(Serialize)]
struct ImeActionResponse {
    #[serde(rename = "type")]
    action_type: String,
    text: Option<String>,
    delete: Option<usize>,
}

fn build_state_response(
    processor: &qianyan_ime_engine::Processor,
    action: &qianyan_ime_engine::processor::Action,
) -> ImeSessionResponse {
    let pinyin_display = qianyan_ime_engine::compositor::Compositor::get_preedit(&processor.ctx);
    let filter_active = processor.ctx.session.filter_mode != qianyan_ime_engine::processor::FilterMode::None;
    let chinese_enabled = processor.ctx.session_state.chinese_enabled;

    let candidates: Vec<ImeCandidateResponse> = processor.ctx.session.candidates
        .iter()
        .map(|c| ImeCandidateResponse {
            text: c.text.to_string(),
            simplified: c.simplified.to_string(),
            traditional: c.traditional.to_string(),
            hint: c.hint.to_string(),
            weight: c.weight,
            match_level: c.match_level,
            source: c.source.to_string(),
        })
        .collect();

    let segments = processor.ctx.session.best_segmentation.clone();

    let action_resp = match action {
        qianyan_ime_engine::processor::Action::Emit(text) => Some(ImeActionResponse {
            action_type: "commit".into(),
            text: Some(text.clone()),
            delete: None,
        }),
        qianyan_ime_engine::processor::Action::DeleteAndEmit { delete, insert } => Some(ImeActionResponse {
            action_type: "delete_and_emit".into(),
            text: Some(insert.clone()),
            delete: Some(*delete),
        }),
        _ => None,
    };

    ImeSessionResponse {
        session_id: String::new(), // filled in by caller
        pinyin_display,
        candidates,
        segments,
        filter_active,
        chinese_enabled,
        action: action_resp,
    }
}

#[derive(Deserialize)]
struct ImeKeyRequest {
    session_id: String,
    key: String,
    #[serde(default)]
    val: i32,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    candidate_index: Option<usize>,
    #[serde(default)]
    profile: Option<String>,
}

async fn ime_session_handler(
    State((config, _, _)): State<WebState>,
    Extension(ime_handle): Extension<Arc<ImeEngineHandle>>,
) -> Result<Json<ImeSessionResponse>, StatusCode> {
    let shared_engine = ensure_engine(&ime_handle).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let cfg = config.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.clone();
    let mut processor = create_processor(&ime_handle.root, shared_engine)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    processor.apply_config(&cfg);
    let _ = processor.handle_event(qianyan_ime_engine::InputEvent::CandidateSelect(0)); // warmup first lookup

    let session_id = uuid_v4();
    let action = qianyan_ime_engine::processor::Action::Consume;
    let mut resp = build_state_response(&processor, &action);
    resp.session_id = session_id.clone();

    let mut sessions = ime_handle.sessions.lock()
        .unwrap_or_else(|e| e.into_inner());
    if sessions.len() >= MAX_IME_SESSIONS {
        let now = std::time::Instant::now();
        let ttl = std::time::Duration::from_secs(SESSION_TTL_SECS);
        sessions.retain(|_, s| now.duration_since(s.created) < ttl);
        if sessions.len() >= MAX_IME_SESSIONS {
            let oldest_key = sessions.iter().min_by_key(|(_, s)| s.created).map(|(k, _)| k.clone());
            if let Some(key) = oldest_key {
                sessions.remove(&key);
            }
        }
    }
    sessions.insert(session_id, ImeSession { processor, created: Instant::now() });

    Ok(Json(resp))
}


async fn ime_key_handler(
    State(_): State<WebState>,
    Extension(ime_handle): Extension<Arc<ImeEngineHandle>>,
    Json(req): Json<ImeKeyRequest>,
) -> Result<Json<ImeSessionResponse>, StatusCode> {
    let mut sessions = ime_handle.sessions.lock()
        .unwrap_or_else(|e| e.into_inner());
    let session = sessions.get_mut(&req.session_id).ok_or(StatusCode::NOT_FOUND)?;

    if let Some(ref profile) = req.profile {
        session.processor.ctx.session_state.active_profiles = vec![profile.clone()];
        session.processor.ctx.engine.clear_cache();
    }

    let action = if let Some(idx) = req.candidate_index {
        session.processor.handle_event(qianyan_ime_engine::InputEvent::CandidateSelect(idx))
    } else {
        let key: qianyan_ime_engine::keys::VirtualKey = req.key.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
        session.processor.handle_event(qianyan_ime_engine::InputEvent::Key {
            key,
            val: req.val,
            shift: req.shift,
            ctrl: req.ctrl,
            alt: req.alt,
        })
    };

    let mut resp = build_state_response(&session.processor, &action);
    resp.session_id = req.session_id;

    Ok(Json(resp))
}

// ===== User Data Export / Import =====

async fn export_user_data() -> impl IntoResponse {
    let mut result = serde_json::Map::new();
    let root = user_dict_root();

    if let Ok(entries) = std::fs::read_dir(&root) {
        for profile_entry in entries.flatten() {
            if !profile_entry.path().is_dir() { continue; }
            let profile = profile_entry.file_name().to_string_lossy().to_string();
            let mut profile_data = serde_json::Map::new();

            for file_name in &["learned.json", "usage.json", "ngrams.json"] {
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

async fn import_user_data(
    State((_, _, tray_tx)): State<WebState>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    let root = user_dict_root();
    let _ = std::fs::create_dir_all(&root);

    if let Some(user_data) = body.get("user_data").and_then(|v| v.as_object()) {
        for (profile, data) in user_data {
            let profile_dir = root.join(profile);
            let _ = std::fs::create_dir_all(&profile_dir);
            if let Some(data_obj) = data.as_object() {
                for (key, value) in data_obj {
                    let file_name = match key.as_str() {
                        "learned" => "learned.json",
                        "usage" => "usage.json",
                        "ngram" => "ngrams.json",
                        _ => continue,
                    };
                    let path = profile_dir.join(file_name);
                    let content = serde_json::to_string_pretty(value).unwrap_or_default();
                    let _ = std::fs::write(&path, content);
                }
            }
        }
    }

    // 如果是旧格式 {data: {profile: {pinyin: [...]}}} 直接合并到 learned.json
    if let Some(data) = body.get("data").and_then(|v| v.as_object()) {
        for (profile, pinyin_map) in data {
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
            let _ = std::fs::write(&learned_path, serde_json::to_string_pretty(&merged).unwrap_or_default());
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

fn configs_root() -> PathBuf {
    // 尝试找到 configs/ 目录
    if let Ok(cwd) = std::env::current_dir() {
        let configs = cwd.join("configs");
        if configs.exists() { return configs; }
    }
    PathBuf::from("configs")
}

async fn export_full_backup() -> impl IntoResponse {
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
            for file_name in &["learned.json", "usage.json", "ngrams.json"] {
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

async fn restore_full_backup(
    State((_, _, tray_tx)): State<WebState>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    // 1. 恢复配置文件
    if let Some(config_data) = body.get("config").and_then(|v| v.as_object()) {
        let configs = configs_root();
        let _ = std::fs::create_dir_all(&configs);
        for (name, value) in config_data {
            let path = configs.join(format!("{}.json", name));
            let content = serde_json::to_string_pretty(value).unwrap_or_default();
            let _ = std::fs::write(&path, content);
        }
    }

    // 2. 恢复用户数据
    if let Some(user_data) = body.get("user_data").and_then(|v| v.as_object()) {
        let root = user_dict_root();
        let _ = std::fs::create_dir_all(&root);
        for (profile, data) in user_data {
            let profile_dir = root.join(profile);
            let _ = std::fs::create_dir_all(&profile_dir);
            if let Some(data_obj) = data.as_object() {
                for (key, value) in data_obj {
                    let file_name = match key.as_str() {
                        "learned" => "learned.json",
                        "usage" => "usage.json",
                        "ngram" => "ngrams.json",
                        _ => continue,
                    };
                    let path = profile_dir.join(file_name);
                    let content = serde_json::to_string_pretty(value).unwrap_or_default();
                    let _ = std::fs::write(&path, content);
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
    let remaining = secs % 86400;
    let h = remaining / 3600;
    let min = (remaining % 3600) / 60;
    let s = remaining % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, min, s)
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let d = days;
    let era = if d >= 0 { d } else { d - 146096 } / 146097;
    let doe = d - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { (mp + 3) as u32 } else { (mp - 9) as u32 };
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

fn uuid_v4() -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(32);
    for _ in 0..8 { write!(s, "{:x}", rand_u8()).expect("write to String never fails"); }
    s
}

fn rand_u8() -> u8 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    ((nanos >> 16) ^ (nanos >> 8) ^ nanos) as u8
}
