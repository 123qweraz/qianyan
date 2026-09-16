use crate::scheme::InputScheme;
use std::collections::HashMap;

/// 根据方案定义实例化解析引擎。dict 传入方案 id（作为主词库挂载键）。
pub fn make_scheme(def: &qianyan_ime_core::scheme_def::SchemeDefinition) -> Option<Box<dyn InputScheme>> {
    let scheme: Box<dyn InputScheme> = match def.engine.as_str() {
        "pinyin" => Box::new(crate::schemes::ChineseScheme::with_dict(&def.id)),
        "english" => Box::new(crate::schemes::EnglishScheme::with_dict(&def.id)),
        "japanese" => Box::new(crate::schemes::JapaneseScheme::with_dict(&def.id)),
        "stroke" => Box::new(crate::schemes::StrokeScheme::with_dict(&def.id)),
        "bihua" => Box::new(crate::schemes::BihuaScheme::with_dict(&def.id)),
        "pybi" => Box::new(crate::schemes::PybiScheme::with_dicts(&def.id, &def.aux_dicts)),
        _ => return None,
    };
    Some(scheme)
}

/// 由方案清单构建 方案id → 引擎 映射（跳过错方案）。
pub fn build_schemes(
    defs: &[qianyan_ime_core::scheme_def::SchemeDefinition],
) -> HashMap<String, Box<dyn InputScheme>> {
    let mut m = HashMap::new();
    for def in defs {
        if def.id.is_empty() {
            continue;
        }
        if let Some(s) = make_scheme(def) {
            m.insert(def.id.clone(), s);
        }
    }
    m
}

/// 内置方案清单（用于无方案文件时的兜底，与 context::default_schemes 等价来源）。
pub fn builtin_defs() -> Vec<qianyan_ime_core::scheme_def::SchemeDefinition> {
    let mut defs = Vec::new();
    for (id, label, engine, dict, aux) in qianyan_ime_core::scheme_def::builtin_scheme_specs() {
        defs.push(qianyan_ime_core::scheme_def::SchemeDefinition {
            id: id.to_string(),
            label: label.to_string(),
            engine: engine.to_string(),
            dict: dict.to_string(),
            aux_dicts: aux.iter().map(|s| s.to_string()).collect(),
            ..qianyan_ime_core::scheme_def::SchemeDefinition::default()
        });
    }
    defs
}

/// 内置方案映射（无 configs/schemes 时的兜底，与历史 default_schemes 等价）。
pub fn default_schemes() -> HashMap<String, Box<dyn InputScheme>> {
    build_schemes(&builtin_defs())
}