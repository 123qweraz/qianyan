use axum::{
    routing::{get, post},
    extract::{State, Json},
    response::{IntoResponse, Html},
    http::{StatusCode, Uri, HeaderName, header},
    Router,
};
use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU16, Ordering};
use std::collections::HashMap;
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
}

type WebState = (
    Arc<RwLock<Config>>, 
    Arc<RwLock<HashMap<String, Trie>>>, 
    std::sync::mpsc::Sender<TrayEvent>
);

impl WebServer {
    pub fn new(
        port: u16, 
        actual_port: Arc<AtomicU16>,
        config: Arc<RwLock<Config>>, 
        tries: Arc<RwLock<HashMap<String, Trie>>>,
        tray_tx: std::sync::mpsc::Sender<TrayEvent>
    ) -> Self {
        Self { port, actual_port, config, tries, tray_tx }
    }

    pub async fn start(self) {
        let state: WebState = (self.config, self.tries, self.tray_tx);
        let app = Router::new()
            .route("/", get(index_handler))
            .route("/api/config", get(get_config).post(update_config))
            .route("/api/config/reset", post(reset_config))
            .route("/api/config/reset/{sections}", post(reset_config_section))
            .route("/api/fonts", get(list_fonts))
            .route("/api/dicts", get(list_dicts))
            .route("/api/dicts/compile", post(compile_dicts_handler))
            .route("/api/dicts/reload", post(reload_dicts))
            .route("/api/dicts/toggle", post(toggle_dict))
            .route("/api/dicts/create", post(create_dict_handler))
            .route("/api/dictionary/chars", get(get_chars_dict))
            .route("/api/dict/search", get(search_dict))
            .route("/api/dict/browse", get(browse_dict))
            .route("/api/dict/update", post(update_dict_entry))
            .route("/api/dict/entry/update", post(update_dict_entry_full))
            .route("/api/dict/entry/delete", post(delete_dict_entry))
            .route("/api/dict/add", post(add_dict_entry))
            .route("/api/dict/entry/add", post(add_dict_entry_full))
            .route("/api/dict/clear_user", post(clear_user_dict))
            .route("/api/keyboard/send", post(send_key_handler))
            .route("/static/*file", get(static_handler))
            .route("/dicts/*file", get(dicts_handler))
            .fallback(index_handler)
            .with_state(state);

        let mut current_port = self.port;
        loop {
            let addr = format!("127.0.0.1:{}", current_port);
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    self.actual_port.store(current_port, Ordering::SeqCst);
                    println!("[Web] 服务器启动在 http://{}", addr);
                    if let Err(e) = axum::serve(listener, app).await {
                        eprintln!("[Web] Server error: {}", e);
                    }
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    eprintln!("[Web] 端口 {} 已被占用，正在尝试 {}...", current_port, current_port + 1);
                    current_port += 1;
                    if current_port > self.port + 100 {
                        eprintln!("[Web] 已尝试 100 个端口均无法启动，退出。");
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
    Json(config.read().expect("config lock poisoned").clone()).into_response()
}

async fn update_config(
    State((config, _, tray_tx)): State<WebState>,
    Json(new_config): Json<Config>
) -> StatusCode {
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
    // 使用新重构的 save 方法保存到 configs/ 目录
    if let Err(_e) = new_config.save() { return StatusCode::INTERNAL_SERVER_ERROR; }
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
            "linux" => w.linux = default_conf.linux.clone(),
            "files" => w.files = default_conf.files.clone(),
            _ => return StatusCode::BAD_REQUEST,
        }
    }
    if let Err(_e) = w.save() { return StatusCode::INTERNAL_SERVER_ERROR; }
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
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
        .and_then(|f| serde_json::to_writer_pretty(f, &empty).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
    {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn compile_dicts_handler(State((_, _, tray_tx)): State<WebState>) -> StatusCode {
    let _ = tray_tx.send(TrayEvent::ShowNotification("正在编译词库...".into()));
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
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}

#[derive(Serialize)]
struct SearchResult {
    pinyin: String,
    word: String,
    hint: String,
    file: String,
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: String,
    ime: Option<String>,
}

async fn search_dict(axum::extract::Query(query): axum::extract::Query<SearchQuery>) -> Json<Vec<SearchResult>> {
    let mut results = Vec::new();
    let q = query.q.to_lowercase();
    let ime = query.ime.as_deref().unwrap_or("chinese");
    
    if q.is_empty() {
        return Json(results);
    }
    
    // 确定搜索路径（只允许已知语言）
    let search_root = match ime {
        "japanese" => "dicts/japanese",
        "stroke" => "dicts/stroke",
        "english" => "dicts/english",
        "chinese" => "dicts/chinese",
        _ => return Json(results),
    };
    
    // 遍历指定目录下的 json
    let entries = walkdir::WalkDir::new(search_root);
    for entry in entries.into_iter().filter_map(|e: Result<walkdir::DirEntry, walkdir::Error>| e.ok()) {
        if entry.path().extension().is_some_and(|ext: &std::ffi::OsStr| ext == "json") {
            let path_str = entry.path().to_string_lossy().to_string();
            if path_str.contains(".disabled") {
                continue;
            }
            if let Ok(f) = std::fs::File::open(entry.path()) {
                if let Ok(json) = serde_json::from_reader::<_, serde_json::Value>(std::io::BufReader::new(f)) {
                    if let Some(obj) = json.as_object() {
                        for (pinyin, val) in obj {
                            let pinyin_match = pinyin.to_lowercase().starts_with(&q) || pinyin.to_lowercase() == q;
                            if let Some(arr) = val.as_array() {
                                for v in arr {
                                    let word = v.get("char").and_then(|c| c.as_str()).unwrap_or("");
                                    let hint = v.get("en").and_then(|e| e.as_str()).unwrap_or("");
                                    // 支持按拼音、汉字、英文释义搜索
                                    if !pinyin_match && !word.contains(&query.q) && !hint.to_lowercase().contains(&q) {
                                        continue;
                                    }
                                    results.push(SearchResult {
                                        pinyin: pinyin.clone(),
                                        word: word.to_string(),
                                        hint: hint.to_string(),
                                        file: path_str.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        if results.len() > 100 { break; }
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
struct DictEntryView {
    pinyin: String,
    word: String,
    trad: String,
    en: String,
    tone: String,
    weight: i64,
    stroke_aux: String,
    category: String,
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

    let mut all_entries: Vec<DictEntryView> = Vec::new();
    // 记录搜索时的匹配类型，用于排序优先级：拼音 > 汉字 > 英文
    let mut match_kind: Vec<u8> = Vec::new();
    if let Ok(f) = std::fs::File::open(&path) {
        if let Ok(json) = serde_json::from_reader::<_, serde_json::Value>(std::io::BufReader::new(f)) {
            if let Some(obj) = json.as_object() {
                for (pinyin, val) in obj {
                    if let Some(arr) = val.as_array() {
                        for v in arr {
                            let word = v.get("char").and_then(|c| c.as_str()).unwrap_or("");
                            let en = v.get("en").and_then(|e| e.as_str()).unwrap_or("");
                            let trad = v.get("trad").and_then(|t| t.as_str()).unwrap_or("");
                            let tone = v.get("tone").and_then(|t| t.as_str()).unwrap_or("");
                            let weight = v.get("weight").and_then(|w| w.as_i64()).unwrap_or(0);
                            let stroke_aux = v.get("stroke_aux").and_then(|s| s.as_str()).unwrap_or("");
                            let category = v.get("category").and_then(|c| c.as_str()).unwrap_or("");

                            let mut kind: u8 = 0;
                            if !search.is_empty() {
                                let search_plain = strip_tone(&search);
                                let match_py = strip_tone(&pinyin.to_lowercase()).contains(&search_plain);
                                let match_word = word.to_lowercase().contains(&search);
                                let match_en = en.to_lowercase().contains(&search);
                                let matched = match search_by {
                                    "pinyin" => match_py,
                                    "word" => match_word,
                                    "en" => match_en,
                                    _ => {
                                        if match_py { kind = 0; true }
                                        else if match_word { kind = 1; true }
                                        else { false }
                                    },
                                };
                                if !matched {
                                    continue;
                                }
                            }
                            match_kind.push(kind);
                            all_entries.push(DictEntryView {
                                pinyin: pinyin.clone(),
                                word: word.to_string(),
                                trad: trad.to_string(),
                                en: en.to_string(),
                                tone: tone.to_string(),
                                weight,
                                stroke_aux: stroke_aux.to_string(),
                                category: category.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    let total = all_entries.len();
    let total_pages = total.div_ceil(page_size).max(1);

    // Sort：有搜索时按匹配优先级 拼音 > 汉字 > 英文，同优先级再按用户选择排序
    let cmp_ascii = |a: &str, b: &str, asc: bool| -> std::cmp::Ordering {
        let ord = a.to_lowercase().cmp(&b.to_lowercase());
        if asc { ord } else { ord.reverse() }
    };
    if !search.is_empty() && search_by == "all" {
        // 先按匹配优先级排序：拼音(0) > 汉字(1) > 英文(2)
        let mut paired: Vec<(DictEntryView, u8)> = all_entries.into_iter().zip(match_kind.into_iter()).collect();
        paired.sort_by(|(a, ka), (b, kb)| {
            if ka != kb { return ka.cmp(kb); }
            let asc = sort_order == "asc";
            match sort_by {
                "word" => cmp_ascii(&a.word, &b.word, asc),
                "en" => cmp_ascii(&a.en, &b.en, asc),
                "weight" => if asc { a.weight.cmp(&b.weight) } else { b.weight.cmp(&a.weight) },
                "stroke_aux" => cmp_ascii(&a.stroke_aux, &b.stroke_aux, asc),
                _ => cmp_ascii(&a.pinyin, &b.pinyin, asc),
            }
        });
        all_entries = paired.into_iter().map(|(e, _)| e).collect();
    } else {
        all_entries.sort_by(|a, b| {
            let asc = sort_order == "asc";
            match sort_by {
                "word" => cmp_ascii(&a.word, &b.word, asc),
                "en" => cmp_ascii(&a.en, &b.en, asc),
                "weight" => if asc { a.weight.cmp(&b.weight) } else { b.weight.cmp(&a.weight) },
                "stroke_aux" => cmp_ascii(&a.stroke_aux, &b.stroke_aux, asc),
                _ => cmp_ascii(&a.pinyin, &b.pinyin, asc),
            }
        });
    }

    let start = (page - 1) * page_size;
    let entries: Vec<DictEntryView> = all_entries.into_iter().skip(start).take(page_size).collect();

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

async fn clear_user_dict(State((_, _, tray_tx)): State<WebState>) -> StatusCode {
    let files = ["data/learned_words.json", "data/usage_history.json", "data/user_dict.json"];
    for f in files {
        let path = std::path::Path::new(f);
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
    // 通知主线程清空内存中的用户词典
    let _ = tray_tx.send(TrayEvent::ClearUserDict);
    StatusCode::OK
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
    Json(req): Json<SendKeyRequest>
) -> StatusCode {
    let key = req.key.to_lowercase();
    let _action = req.action.unwrap_or_else(|| "tap".to_string());
    
    let tray_tx = state.2.clone();
    let _ = tray_tx.send(TrayEvent::SendKey(key));
    
    StatusCode::OK
}
