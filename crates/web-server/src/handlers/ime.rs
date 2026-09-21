use super::*;

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

    // 输入方案：读取 configs/schemes/*.json；目录缺失/为空时回退内置 6 方案。
    let scheme_registry = qianyan_ime_core::scheme_def::SchemeRegistry::load(&root.join("configs/schemes"));
    let schemes_defs: Vec<qianyan_ime_core::scheme_def::SchemeDefinition> = if scheme_registry
        .schemes
        .is_empty()
    {
        qianyan_ime_engine::scheme_registry::builtin_defs()
    } else {
        scheme_registry.schemes
    };
    log::info!(
        "web-server: loaded {} schemes from configs/schemes",
        schemes_defs.len()
    );

    // 把 data/<dict> 挂到方案 id，并按方案文件装配解析引擎
    for def in &schemes_defs {
        if let Some(files) = trie_paths.get(&def.dict).cloned() {
            let id = def.id.clone();
            trie_paths.insert(id, files);
        }
    }
    let schemes_map: HashMap<String, Box<dyn InputScheme>> =
        qianyan_ime_engine::scheme_registry::build_schemes(&schemes_defs);
    let scheme_defs_map = Arc::new(
        schemes_defs
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect::<HashMap<_, _>>(),
    );

    let syllable_freq_arc = Arc::new(syllable_freq);
    let empty_user_dict = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())));

    let single_syllables = Arc::new(load_single_syllables(root));
    let mut engine = SearchEngine::new(
        trie_paths,
        syllable_freq_arc,
        empty_user_dict,
        Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
        Arc::new(ArcSwap::new(Arc::new(HashMap::<String, Vec<String>>::new()))),
        Arc::new(schemes_map),
    );
    engine.single_syllables = single_syllables;
    engine.scheme_defs = scheme_defs_map;
    Ok(engine)
}

#[derive(Deserialize)]
pub(crate) struct ImeSearchRequest {
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
pub(crate) struct ImeCandidateResponse {
    text: String,
    simplified: String,
    traditional: String,
    hint: String,
    weight: f64,
    match_level: u8,
    source: String,
}

#[derive(Serialize)]
pub(crate) struct ImeSearchResponse {
    candidates: Vec<ImeCandidateResponse>,
    segments: Vec<String>,
}

pub(crate) async fn ime_search_handler(
    State((config, _, _)): State<WebState>,
    Extension(ime_handle): Extension<Arc<ImeEngineHandle>>,
    Json(req): Json<ImeSearchRequest>,
) -> Json<ImeSearchResponse> {
    let engine = {
        let guard = ime_handle.engine.read()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(ref engine) = *guard {
            engine.clone()
        } else {
            drop(guard);
            match prepare_ime_engine(&ime_handle.root) {
                Ok(engine) => {
                    let mut w = ime_handle.engine.write()
                        .unwrap_or_else(|e| e.into_inner());
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

    let cfg = config.get();
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
        context_pair: None,
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
        .unwrap_or_else(|e| e.into_inner());
    if let Some(ref engine) = *guard {
        return Some(engine.clone());
    }
    drop(guard);
    match prepare_ime_engine(&handle.root) {
        Ok(engine) => {
            let mut w = handle.engine.write()
                .unwrap_or_else(|e| e.into_inner());
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
pub(crate) struct ImeSessionResponse {
    session_id: String,
    pinyin_display: String,
    candidates: Vec<ImeCandidateResponse>,
    segments: Vec<String>,
    filter_active: bool,
    chinese_enabled: bool,
    action: Option<ImeActionResponse>,
}

#[derive(Serialize)]
pub(crate) struct ImeActionResponse {
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
pub(crate) struct ImeKeyRequest {
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

pub(crate) async fn ime_session_handler(
    State((config, _, _)): State<WebState>,
    Extension(ime_handle): Extension<Arc<ImeEngineHandle>>,
) -> Result<Json<ImeSessionResponse>, StatusCode> {
    let shared_engine = ensure_engine(&ime_handle).ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let cfg = config.get();
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


pub(crate) async fn ime_key_handler(
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

fn uuid_v4() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::hash::{Hash, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let tid = std::thread::current().id();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    pid.hash(&mut hasher);
    tid.hash(&mut hasher);
    let thread_hash = hasher.finish();
    let cnt = COUNTER.fetch_add(1, Ordering::Relaxed);
    let high = (ts as u64).wrapping_mul(thread_hash).wrapping_add(cnt);
    let low = ((ts >> 64) as u64).wrapping_mul(pid as u64).wrapping_add(cnt.wrapping_mul(thread_hash));
    format!("{:016x}{:016x}", high, low)
}
