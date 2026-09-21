use super::*;
use encoding_rs::GBK;

#[derive(Deserialize)]
pub(crate) struct DiscoverConfigMsg {
    min_count: Option<usize>,
    min_pmi: Option<f64>,
    min_entropy: Option<f64>,
    max_word_len: Option<usize>,
}

#[derive(Serialize)]
pub(crate) struct DiscoverResult {
    word: String,
    pinyin: String,
    in_dict: bool,
    weight: u32,
}

/// 新词发现器：处理上传的文件
/// 从 multipart 中提取文本、配置和导出参数
pub(crate) struct DiscoverDownloadParams {
    text: String,
    config: qianyan_ime_engine::pipeline::DiscoveryConfig,
    format: String,
    only_new: bool,
}

pub(crate) async fn extract_download_params(
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
pub(crate) async fn extract_text_and_config(
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

pub(crate) async fn discover_words_file_handler(
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

pub(crate) async fn discover_download_handler(
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
pub(crate) struct DiscoveredWordWithPinyin {
    word: String,
    pinyin: String,
    in_dict: bool,
    count: usize,
    pmi: f64,
    entropy: f64,
}

#[derive(Deserialize)]
pub(crate) struct ExportDiscoveryRequest {
    words: Vec<String>,
    pinyins: Vec<String>,
    weights: Vec<u32>,
    format: String,
}

/// 保存发现的新词到服务器
pub(crate) async fn save_discovery_handler(
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
pub(crate) async fn export_discovery_handler(
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
