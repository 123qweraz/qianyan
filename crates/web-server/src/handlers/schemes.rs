use super::*;

pub(crate) async fn compile_dicts_handler(State((config, _, tray_tx)): State<WebState>) -> impl IntoResponse {
    let _ = tray_tx.send(TrayEvent::ShowNotification("正在编译词库...".into()));
    if let Ok(mut cache) = get_known_words_cache().lock() {
        *cache = None;
    }
    if let Ok(mut cache) = get_dict_cache().lock() {
        cache.clear();
    }
    match qianyan_ime_engine::compiler::check_and_compile_all() {
        Ok(_) => {
            let cfg = config.get();
            let (annotation_sources, annotation_files): (
                std::collections::BTreeMap<String, String>,
                std::collections::BTreeMap<String, String>,
            ) = (
                cfg.input
                    .annotation_sources
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                cfg.input
                    .annotation_files
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            );
            let _ = qianyan_ime_engine::compiler::check_and_compile_annotation_sources(&annotation_sources);
            let _ = qianyan_ime_engine::compiler::check_and_compile_annotation_files(&annotation_files, &annotation_sources);
            let _ = tray_tx.send(TrayEvent::ShowNotification("词库编译完成".into()));
            let _ = tray_tx.send(TrayEvent::ReloadConfig);
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        },
        Err(e) => {
            eprintln!("[Web] 词库编译失败: {}", e);
            let _ = tray_tx.send(TrayEvent::ShowNotification(format!("编译失败: {}", e)));
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("{}", e)}))).into_response()
        }
    }
}

fn schemes_dir() -> std::path::PathBuf {
    qianyan_ime_core::utils::find_project_root().join("configs/schemes")
}

pub(crate) fn scheme_def_for(profile: &str) -> Option<qianyan_ime_core::scheme_def::SchemeDefinition> {
    let reg = qianyan_ime_core::scheme_def::SchemeRegistry::load(&schemes_dir());
    reg.get(profile).cloned().or_else(|| {
        qianyan_ime_core::scheme_def::builtin_scheme_specs()
            .iter()
            .find(|(id, _, _, _, _)| *id == profile)
            .map(|(id, label, engine, dict, aux)| qianyan_ime_core::scheme_def::SchemeDefinition {
                id: id.to_string(),
                label: label.to_string(),
                engine: engine.to_string(),
                dict: dict.to_string(),
                aux_dicts: aux.iter().map(|s| s.to_string()).collect(),
                ..qianyan_ime_core::scheme_def::SchemeDefinition::default()
            })
    })
}

pub(crate) fn scheme_dict_for(profile: &str) -> String {
    scheme_def_for(profile)
        .map(|d| d.dict)
        .unwrap_or_else(|| profile.to_string())
}

fn engine_descriptions() -> Vec<serde_json::Value> {
    use qianyan_ime_core::scheme_def::KNOWN_ENGINES;
    let descs: &[(&str, &str)] = &[
        ("pinyin", "拼音（含简拼/模糊/纠错/联想）"),
        ("english", "英文（词表/纠错/首字母冲突）"),
        ("japanese", "日语（假名/词汇）"),
        ("stroke", "笔顺码（1-5 转双笔一键编码查字）"),
        ("bihua", "逐笔笔画（一键一画）"),
        ("pybi", "拼音+逐笔笔画筛选"),
    ];
    KNOWN_ENGINES
        .iter()
        .map(|id| {
            let desc = descs
                .iter()
                .find(|(e, _)| *e == *id)
                .map(|(_, d)| *d)
                .unwrap_or("");
            json!({"id": id, "desc": desc})
        })
        .collect()
}

/// GET /api/schemes：方案列表 + 引擎清单 + 可用码表目录 + 注释默认信息
pub(crate) async fn list_schemes(
    State((config, _, _)): State<WebState>,
) -> Json<serde_json::Value> {
    let directory = schemes_dir();
    let registry = qianyan_ime_core::scheme_def::SchemeRegistry::load(&directory);
    // 配置文件目录为空（首次尚未启动守护进程生成默认方案）时，
    // 直接展示内置 6 方案，保证页面始终有现有方案可选。
    let mut schemes = registry.schemes.clone();
    if schemes.is_empty() {
        schemes = qianyan_ime_core::scheme_def::builtin_scheme_specs()
            .iter()
            .map(|(id, label, engine, dict, aux)| {
                qianyan_ime_core::scheme_def::SchemeDefinition {
                    id: id.to_string(),
                    label: label.to_string(),
                    engine: engine.to_string(),
                    dict: dict.to_string(),
                    aux_dicts: aux.iter().map(|s| s.to_string()).collect(),
                    ..qianyan_ime_core::scheme_def::SchemeDefinition::default()
                }
            })
            .collect();
    }
    let root = qianyan_ime_core::utils::find_project_root();
    let mut dicts: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.join("dicts")) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(n) = entry.file_name().to_str() {
                    if n != "annotations" {
                        dicts.push(n.to_string());
                    }
                }
            }
        }
    }
    dicts.sort();
    let config_guard = config.get();
    let global = json!({
        "annotation_lang": config_guard.input.annotation_lang,
        "default_candidate_fields": config_guard.input.candidate_display.fields,
        "default_aux_filter": config_guard.input.filter_aux_mode,
        "annotation_sources": config_guard.input.annotation_sources,
        "enabled_profiles": config_guard.input.enabled_profiles,
        "default_profile": config_guard.input.default_profile,
    });
    Json(json!({
        "ok": true,
        "engines": engine_descriptions(),
        "dicts": dicts,
        "schemes": schemes,
        "errors": registry.errors,
        "global": global,
    }))
}

#[derive(Deserialize)]
pub(crate) struct SchemeSaveRequest {
    action: String,
    def: Option<qianyan_ime_core::scheme_def::SchemeDefinition>,
    id: Option<String>,
}

/// POST /api/scheme：保存 / 删除 / 恢复默认方案，保存后增量编译并触发重载。
pub(crate) async fn upsert_scheme(
    State((config, _, tray_tx)): State<WebState>,
    Json(req): Json<SchemeSaveRequest>,
) -> Response {
    let directory = schemes_dir();
    match req.action.as_str() {
        "delete" => {
            let Some(id) = req.id.as_deref() else {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": "缺少 id"}))).into_response();
            };
            if !qianyan_ime_core::scheme_def::delete_scheme(id, &directory) {
                return (StatusCode::NOT_FOUND, Json(json!({"error": "方案不存在"}))).into_response();
            }
            config.update(|w| {
                w.input.enabled_profiles.retain(|p| p != id);
                w.input.profile_keys.retain(|k| k.profile != id);
                if w.input.default_profile == id {
                    w.input.default_profile = String::new();
                }
                let _ = w.save();
            });
            trigger_compile(&config, tray_tx);
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }
        "restore" => {
            if let Ok(entries) = std::fs::read_dir(&directory) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("json") {
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
            let input = config.get().input.clone();
            if let Err(e) = qianyan_ime_core::scheme_def::ensure_default_schemes(&directory, &input) {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("{}", e)}))).into_response();
            }
            trigger_compile(&config, tray_tx);
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }
        "save" => {
            let def = req.def;
            let Some(def) = def else {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": "缺少方案内容"}))).into_response();
            };
            let id = def.id.clone();
            let valid_id = !id.is_empty()
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid_id {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": "id 只能包含字母/数字/_/-"}))).into_response();
            }
            if def.dict.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": "未选择码表"}))).into_response();
            }
            if !qianyan_ime_core::scheme_def::KNOWN_ENGINES.contains(&def.engine.as_str()) {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": "未知引擎"}))).into_response();
            }
            let root = qianyan_ime_core::utils::find_project_root();
            if !root.join("dicts").join(&def.dict).is_dir() {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": format!("码表目录不存在: {}", def.dict)}))).into_response();
            }
            if let Err(e) = qianyan_ime_core::scheme_def::save_scheme(&def, &directory) {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("{}", e)}))).into_response();
            }
            trigger_compile(&config, tray_tx);
            (StatusCode::OK, Json(json!({"ok": true, "id": id}))).into_response()
        }
        _ => (StatusCode::BAD_REQUEST, Json(json!({"error": "未知 action"}))).into_response(),
    }
}

fn trigger_compile(
    config: &Arc<qianyan_ime_core::ConfigStore>,
    tray_tx: std::sync::mpsc::Sender<qianyan_ime_core::event::TrayEvent>,
) {
    // data 已有编译产物时跳过自动全量编译（发行包自带），仅 data 为空时首次编译；
    // 需要强制重编请用「编译词库」按钮（POST /api/dicts/compile）。
    let _ = qianyan_ime_engine::compiler::check_and_compile_all_if_needed();
    let c = config.get();
    let (annotation_sources, annotation_files, schemes) = (
        c.input
            .annotation_sources
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<std::collections::BTreeMap<_, _>>(),
        c.input
            .annotation_files
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<std::collections::BTreeMap<_, _>>(),
        qianyan_ime_core::scheme_def::SchemeRegistry::load(&schemes_dir()).schemes,
    );
    let _ = qianyan_ime_engine::compiler::check_and_compile_annotation_sources(&annotation_sources);
    let _ = qianyan_ime_engine::compiler::check_and_compile_annotation_files(&annotation_files, &annotation_sources);
    let _ = qianyan_ime_engine::compiler::check_and_compile_scheme_annotations(&schemes, &annotation_sources);
    let _ = tray_tx.send(qianyan_ime_core::event::TrayEvent::ReloadConfig);
}

#[derive(Deserialize)]
pub(crate) struct AnnotationPreviewRequest {
    file: String,
}

/// POST /api/scheme_preview：预览任意注释 TSV 文件（表名/语言键/前几行释义）。
pub(crate) async fn preview_annotation(
    State((config, _, _)): State<WebState>,
    Json(req): Json<AnnotationPreviewRequest>,
) -> Json<serde_json::Value> {
    let file = req.file.trim().to_string();
    if file.is_empty() {
        return Json(json!({"exists": false, "error": "路径为空"}));
    }
    let path = std::path::PathBuf::from(&file);
    let exists = path.is_file();
    if !exists {
        return Json(json!({"exists": false, "table": "", "lang": "", "error": "文件不存在"}));
    }
    let annotation_sources: std::collections::BTreeMap<String, String> = config
        .get()
        .input
        .annotation_sources
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let table =
        qianyan_ime_core::utils::annotation_table_name_for_path(&file, &annotation_sources);
    let lang = table
        .rsplit_once('-')
        .map(|(_, l)| l.to_string())
        .unwrap_or_default();
    let mut preview: Vec<[String; 2]> = Vec::new();
    let mut error: String = String::new();
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if let Ok(text) = std::fs::read_to_string(&path) {
        if is_json {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => {
                    if let Some(obj) = v.as_object() {
                        for (word, val) in obj.iter().take(8) {
                            let gloss = match val {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Array(a) => a
                                    .iter()
                                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                                _ => val.to_string(),
                            };
                            preview.push([word.clone(), gloss]);
                        }
                    }
                }
                Err(e) => error = format!("JSON 解析失败: {}", e),
            }
        } else {
            for line in text.lines().take(16) {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut parts = line.split('\t');
                if let Some(word) = parts.next() {
                    let gloss = parts.collect::<Vec<_>>().join(" · ");
                    preview.push([word.to_string(), gloss]);
                    if preview.len() >= 8 {
                        break;
                    }
                }
            }
        }
    } else {
        error = "无法读取文件".to_string();
    }
    Json(json!({
        "exists": true,
        "table": table,
        "lang": lang,
        "preview": preview,
        "error": error,
    }))
}

pub(crate) async fn reload_dicts(State((_, _, tray_tx)): State<WebState>) -> StatusCode {
    if let Ok(mut cache) = get_known_words_cache().lock() {
        *cache = None;
    }
    if let Ok(mut cache) = get_dict_cache().lock() {
        cache.clear();
    }
    let _ = tray_tx.send(TrayEvent::ReloadConfig);
    StatusCode::OK
}
