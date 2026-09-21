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

#[derive(Serialize)]
pub(crate) struct DictFile {
    name: String,
    path: String,
    group: String,
    size: u64,
    entry_count: u64,
    enabled: bool,
}

pub(crate) async fn list_dicts() -> Json<Vec<DictFile>> {
    let mut list = Vec::new();
    let root = find_dicts_root();
    let walker = walkdir::WalkDir::new(&root).into_iter();
    
    for entry in walker.filter_map(|e: Result<walkdir::DirEntry, walkdir::Error>| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let filename = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if filename.ends_with(".json") || filename.ends_with(".json.disabled") {
                // 计算分组名：取 root 下的一级目录名
                let relative = path.strip_prefix(&root).unwrap_or(path);
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
pub(crate) struct ToggleRequest {
    path: String,
}

pub(crate) async fn toggle_dict(Json(req): Json<ToggleRequest>) -> StatusCode {
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
pub(crate) struct CreateDictRequest {
    name: String,
    group: Option<String>,
}

pub(crate) async fn create_dict_handler(Json(req): Json<CreateDictRequest>) -> StatusCode {
    let group = req.group.unwrap_or_else(|| "user".to_string());
    let base = find_dicts_root();
    let dir = match safe_join(&base, &group) {
        Some(d) => d,
        None => return StatusCode::BAD_REQUEST,
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    let filename = format!("{}.json", req.name);
    let file_path = match safe_join(&dir, &filename) {
        Some(p) => p,
        None => return StatusCode::BAD_REQUEST,
    };
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

#[derive(Serialize)]
pub(crate) struct SearchResult {
    pinyin: String,
    word: String,
    hint: String,
    stroke_aux: String,
    strokes: String,
    file: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct SearchQuery {
    q: String,
    ime: Option<String>,
}

pub(crate) async fn search_dict(
    State((config, tries, _)): State<WebState>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> Json<Vec<SearchResult>> {
    let mut results = Vec::new();
    let q = query.q.to_lowercase();
    let ime = query.ime.as_deref().unwrap_or("chinese");

    if q.is_empty() {
        return Json(results);
    }

    // 懒加载 trie（mmap，零额外内存）；自定义方案用其声明的码表目录
    let dict = scheme_dict_for(ime);
    let trie = {
        let mut cache = tries.write().unwrap_or_else(|e| e.into_inner());
        if !cache.contains_key(ime) {
            let project_root = qianyan_ime_core::utils::find_project_root();
            let idx = project_root.join(format!("data/{}/trie.index", dict));
            let dat = project_root.join(format!("data/{}/trie.data", dict));
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

    // 注释表懒加载：data/annotations/<source>-<target>.{index,data}（word→gloss，青简式）
    let (ann_lang, display_template) = {
        let config_guard = config.get();
        let scheme_def = scheme_def_for(ime);
        let template = config_guard
            .input
            .candidate_display_for_scheme(ime, scheme_def.as_ref());
        let (table, _) = config_guard.input.annotation_info_for_scheme(ime, scheme_def.as_ref());
        (table, template)
    };
    let ann_key = format!("annotation_{}", ann_lang);
    let annotation = {
        let mut cache = tries.write().unwrap_or_else(|e| e.into_inner());
        if !cache.contains_key(&ann_key) {
            let project_root = qianyan_ime_core::utils::find_project_root();
            let idx = project_root.join(format!("data/annotations/{}.index", ann_lang));
            let dat = project_root.join(format!("data/annotations/{}.data", ann_lang));
            if idx.exists() && dat.exists() {
                if let Ok(t) = Trie::load(idx, dat, false) {
                    cache.insert(ann_key.clone(), t);
                }
            }
        }
        cache.get(&ann_key).cloned()
    };

    // FST 前缀匹配（拼音搜索），毫秒级
    let hits = trie.search_bfs(&q, 100);
    for tr in hits {
        let gloss: String = annotation
            .as_ref()
            .and_then(|t| t.get_all_exact(&tr.word))
            .and_then(|es| es.first().map(|e| e.word.to_string()))
            .unwrap_or_default();
        let hint = {
            let mut values = HashMap::with_capacity(8);
            values.insert("char".to_string(), tr.word.clone());
            values.insert("pinyin".to_string(), tr.tone.clone());
            values.insert("tone".to_string(), tr.tone.clone());
            values.insert("trad".to_string(), tr.trad.clone());
            values.insert("stroke_aux".to_string(), tr.stroke_aux.clone());
            values.insert("strokes".to_string(), tr.strokes.clone());
            values.insert("category".to_string(), tr.category.clone());
            values.insert("en".to_string(), gloss.clone());
            values.insert("weight".to_string(), tr.weight.to_string());
            render_annotation(&display_template, &values)
        };
        results.push(SearchResult {
            pinyin: tr.tone.to_string(),
            word: tr.word.to_string(),
            hint,
            stroke_aux: tr.stroke_aux.to_string(),
            strokes: tr.strokes.to_string(),
            file: ime.to_string(),
        });
    }

    Json(results)
}

#[derive(Serialize)]
pub(crate) struct BrowseResult {
    entries: Vec<DictEntryView>,
    total: usize,
    page: usize,
    page_size: usize,
    total_pages: usize,
}

#[derive(Serialize)]
#[derive(Clone)]
pub(crate) struct DictEntryView {
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
pub(crate) struct BrowseQuery {
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

const DICT_CACHE_MAX: usize = 20;

/// 注释表（青简式 TSV）：dicts/annotations/<source>-<target>.tsv 懒加载缓存
/// 每行 "词\t[词性.]译词\t[词性.]译词…"，"#" 开头为注释；多义列以 " · " 拼接
fn get_annotation_map(
    lang: &str,
) -> std::sync::Arc<std::collections::HashMap<String, String>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<
        std::sync::RwLock<std::collections::HashMap<String, std::sync::Arc<std::collections::HashMap<String, String>>>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    {
        if let Ok(guard) = cache.read() {
            if let Some(m) = guard.get(lang) {
                return m.clone();
            }
        }
    }
    let mut loaded = std::collections::HashMap::new();
    let ann_path = find_dicts_root().join("annotations").join(format!("{}.tsv", lang));
    if let Ok(content) = std::fs::read_to_string(ann_path) {
        for line in content.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, rest)) = line.split_once('\t') else { continue };
            if key.is_empty() {
                continue;
            }
            let gloss: Vec<&str> = rest
                .split('\t')
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
                .collect();
            if gloss.is_empty() {
                continue;
            }
            loaded.insert(key.to_string(), gloss.join(" · "));
        }
    }
    let arc = std::sync::Arc::new(loaded);
    if let Ok(mut guard) = cache.write() {
        guard.insert(lang.to_string(), arc.clone());
    }
    arc
}

/// 按词典文件选择对应注释表：dicts/english → en-zh、dicts/japanese → jp-en、其余 → zh-en
fn annotation_table_for_path(path: &std::path::Path, base: &std::path::Path) -> &'static str {
    if path.starts_with(base.join("english")) {
        "en-zh"
    } else if path.starts_with(base.join("japanese")) {
        "jp-en"
    } else {
        "zh-en"
    }
}

fn get_cached_dict_entries(path: &std::path::Path) -> std::sync::Arc<Vec<DictEntryView>> {
    let mtime = path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let base = find_dicts_root();
    let table = annotation_table_for_path(path, &base);
    let ann = get_annotation_map(table);
    let ann_mtime = base
        .join("annotations")
        .join(format!("{}.tsv", table))
        .metadata()
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let key = format!("{}|{}", path.to_string_lossy(), ann_mtime.duration_since(std::time::SystemTime::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let mut cache = get_dict_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached_mtime, entries)) = cache.get(&key) {
        if *cached_mtime >= mtime {
            return std::sync::Arc::clone(entries);
        }
    }
    let entries = load_dict_entries(path, &ann);
    let arc = std::sync::Arc::new(entries);
    cache.insert(key, (mtime, std::sync::Arc::clone(&arc)));
    if cache.len() > DICT_CACHE_MAX {
        // 淘汰最早缓存的条目（按 mtime 排序）
        if let Some(oldest) = cache.iter().min_by_key(|(_, (t, _))| t).map(|(k, _)| k.clone()) {
            cache.remove(&oldest);
        }
    }
    arc
}

fn load_dict_entries(
    path: &std::path::Path,
    ann: &std::collections::HashMap<String, String>,
) -> Vec<DictEntryView> {
    let mut all = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        if let Ok(json) = serde_json::from_reader::<_, serde_json::Value>(std::io::BufReader::new(f)) {
            if let Some(obj) = json.as_object() {
                for (pinyin, val) in obj {
                    if let Some(arr) = val.as_array() {
                        for v in arr {
                            let word = v.get("char").and_then(|c| c.as_str()).unwrap_or("").to_string();
                            if word.is_empty() { continue; }
                            let raw_en = v.get("en").and_then(|e| e.as_str()).unwrap_or("");
                            let en = if raw_en.is_empty() {
                                ann.get(&word).cloned().unwrap_or_default()
                            } else {
                                raw_en.to_string()
                            };
                            all.push(DictEntryView {
                                id: format!("{}::{}", pinyin, word),
                                pinyin: pinyin.clone(),
                                word,
                                trad: v.get("trad").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                                en,
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

pub(crate) async fn browse_dict(axum::extract::Query(query): axum::extract::Query<BrowseQuery>) -> impl IntoResponse {
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
pub(crate) struct DeleteEntryRequest {
    pinyin: String,
    word: String,
    file: String,
}

pub(crate) async fn delete_dict_entry(Json(req): Json<DeleteEntryRequest>) -> StatusCode {
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
pub(crate) struct UpdateEntryFullRequest {
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

pub(crate) async fn update_dict_entry_full(Json(req): Json<UpdateEntryFullRequest>) -> StatusCode {
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
pub(crate) struct AddEntryFullRequest {
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

pub(crate) async fn add_dict_entry_full(Json(req): Json<AddEntryFullRequest>) -> StatusCode {
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
pub(crate) struct UpdateEntryRequest {
    pinyin: String,
    word: String,
    new_hint: String,
    file: String,
}

pub(crate) async fn update_dict_entry(Json(req): Json<UpdateEntryRequest>) -> StatusCode {
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
pub(crate) struct AddEntryRequest {
    pinyin: String,
    word: String,
    hint: String,
    file: String,
}

pub(crate) async fn add_dict_entry(Json(req): Json<AddEntryRequest>) -> StatusCode {
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
pub(crate) struct ClearUserDictRequest {
    profile: Option<String>,
    all: bool,
}

pub(crate) async fn clear_user_dict(
    State((_, _, tray_tx)): State<WebState>,
    Json(req): Json<ClearUserDictRequest>,
) -> StatusCode {
    let root = user_dict_root();

    if req.all && req.profile.is_none() {
        // 清空所有方案的词典
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    for file_name in USER_DICT_FILES {
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
        if !valid_profile_name(profile) {
            return StatusCode::BAD_REQUEST;
        }
        let profile_dir = root.join(profile);
        if profile_dir.exists() {
            for file_name in USER_DICT_FILES {
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
pub(crate) struct CleanupResult {
    profile: String,
    removed: usize,
}

/// 扫描用户词典，移除系统词典已有的词
pub(crate) async fn cleanup_user_dict() -> Json<Vec<CleanupResult>> {
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
                let _ = atomic_write(&learned_path, serde_json::to_string_pretty(&data).unwrap_or_default());
            }
            results.push(CleanupResult { profile, removed });
        }
    }

    Json(results)
}

#[derive(Deserialize)]
pub(crate) struct PromoteRequest {
    words: Vec<PromoteWord>,
}
#[derive(Deserialize)]
pub(crate) struct PromoteWord {
    profile: String,
    pinyin: String,
    word: String,
}
#[derive(Serialize)]
pub(crate) struct PromoteResult {
    added: usize,
    skipped: usize,
}

pub(crate) async fn promote_to_system_dict(
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
        let _ = atomic_write(&dict_file, serde_json::to_string_pretty(&dict).unwrap_or_default());
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
                    let _ = atomic_write(&learned_path, serde_json::to_string_pretty(&data).unwrap_or_default());
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
pub(crate) struct UserDictEntryView {
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
pub(crate) async fn browse_user_dict() -> Json<Vec<UserDictEntryView>> {
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

            for (file_name, data_type) in USER_DICT_TYPES {
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
pub(crate) struct DeleteUserDictRequest {
    profile: String,
    pinyin: String,
    word: String,
}

/// 删除用户词典中的一条记录
pub(crate) async fn delete_user_dict_entry(
    State((_, _, tray_tx)): State<WebState>,
    Json(req): Json<DeleteUserDictRequest>,
) -> StatusCode {
    if !valid_profile_name(&req.profile) {
        return StatusCode::BAD_REQUEST;
    }
    let root = user_dict_root();
    let profile_dir = root.join(&req.profile);

    // 同时从 learned, usage, ngram 三个文件中删除该词条
    let mut deleted = false;
    for file_name in USER_DICT_FILES {
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
        let _ = atomic_write(&path, serde_json::to_string_pretty(&data).unwrap_or_default());
    }

    if !deleted {
        return StatusCode::NOT_FOUND;
    }
    let _ = tray_tx.send(TrayEvent::ClearUserDict(Some(req.profile)));
    StatusCode::OK
}

/// 在文件管理器中打开词典目录
pub(crate) async fn open_dicts_dir() -> StatusCode {
    let path = find_dicts_root();
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

pub(crate) async fn list_fonts() -> Json<Vec<crate::platform::fonts::FontInfo>> {
    Json(crate::platform::fonts::list_system_fonts())
}

#[derive(Serialize)]
pub(crate) struct CharEntryView {
    pinyin: String,
    #[serde(rename = "char")]
    character: String,
    en_meaning: String,
    en_aux: String,
    stroke_aux: String,
    group: u32,
}

#[derive(serde::Deserialize)]
pub(crate) struct DictViewQuery {
    file: Option<String>,
}

pub(crate) async fn get_chars_dict(axum::extract::Query(query): axum::extract::Query<DictViewQuery>) -> impl IntoResponse {
    let base_path = find_dicts_root();
    let path = match &query.file {
        Some(f) => match safe_join(&base_path, f) {
            Some(p) => p,
            None => return (StatusCode::FORBIDDEN, "{}").into_response(),
        },
        None => base_path.join("chinese/chars/chars.json"),
    };
    let mut results = Vec::new();
    let ann = get_annotation_map("zh-en");

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
                            let raw_en = entry.get("en").and_then(|v| v.as_str()).unwrap_or("");
                            let en_meaning = if raw_en.is_empty() {
                                ann.get(character).map(|s| s.as_str()).unwrap_or("")
                            } else {
                                raw_en
                            };
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
pub(crate) struct SendKeyRequest {
    key: String,
    action: Option<String>,
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
