use std::collections::HashSet;
use std::sync::Arc;

use crate::processor::Action;
use crate::EngineContext;

use super::{Candidate, MAX_LOOKUP_LIMIT};
use super::engine::SearchQuery;

/// 主查找入口：执行拼音→候选词搜索，包含智能辅码逻辑
pub fn lookup(ctx: &mut EngineContext) -> Option<Action> {
    use crate::processor::FilterMode;

    if ctx.session.buffer.is_empty() {
        ctx.reset();
        return None;
    }

    if ctx.session.filter_mode == FilterMode::Page && !ctx.session.page_snapshot.is_empty() {
        let mut filtered = Vec::new();
        for c in &ctx.session.page_snapshot {
            if ctx.engine.matches_filter(
                c,
                &ctx.session.aux_filter,
                ctx.config.master_config.input.english_aux_mode,
            ) {
                filtered.push(c.clone());
            }
        }
        if !filtered.is_empty() {
            ctx.session.candidates = filtered;
            if ctx.session.candidates.len() == 1 {
                let word = ctx.session.candidates[0].text.clone();
                return Some(crate::processor::commands::commit_candidate(ctx, word, 0));
            }
        } else {
            ctx.session.candidates.clear();
        }
        ctx.session.update_state();
        return None;
    }

    let current_profile = ctx
        .session_state
        .active_profiles
        .first()
        .cloned()
        .unwrap_or_default();
    let last_word = ctx
        .session_state
        .commit_history
        .last()
        .map(|(_, word)| word.as_str());

    let fuzzy_enabled = ctx.session.fuzzy_activated;
    let query = SearchQuery {
        buffer: &ctx.session.buffer,
        profile: &current_profile,
        syllables: &ctx.syllables,
        config: &ctx.config.master_config,
        limit: MAX_LOOKUP_LIMIT,
        filter_mode: ctx.session.filter_mode.clone(),
        aux_filter: &ctx.session.aux_filter,
        context: last_word,
        fuzzy_enabled,
    };
    let (results, segments) = ctx.engine.search(query);
    ctx.session.candidates = results;
    ctx.session.best_segmentation = segments;
    ctx.session.has_dict_match = !ctx.session.candidates.is_empty();
    ctx.session.last_lookup_pinyin = ctx.session.buffer.clone();

    if ctx.config.master_config.input.enable_smart_aux
        && ctx.session.filter_mode == FilterMode::None
    {
        let buffer = &ctx.session.buffer;
        if let Some((pinyin_base, aux_chars)) =
            detect_smart_aux(buffer, &ctx.syllables, ctx.config.master_config.input.smart_aux_mode)
        {
            let aux_query = SearchQuery {
                buffer: &pinyin_base,
                profile: &current_profile,
                syllables: &ctx.syllables,
                config: &ctx.config.master_config,
                limit: MAX_LOOKUP_LIMIT,
                filter_mode: FilterMode::Global,
                aux_filter: &aux_chars,
                context: last_word,
                fuzzy_enabled,
            };
            let (aux_results, _) = ctx.engine.search(aux_query);
            if !aux_results.is_empty() {
                let mut merged = aux_results;
                for c in &ctx.session.candidates {
                    if !merged.iter().any(|r| r.text == c.text) {
                        merged.push(c.clone());
                    }
                }
                ctx.session.candidates = merged;
                ctx.session.has_dict_match = true;
            }
        }
    }

    if ctx.session.candidates.len() == 1 && ctx.session.filter_mode == FilterMode::Global {
        let word = ctx.session.candidates[0].text.clone();
        return Some(crate::processor::commands::commit_candidate(ctx, word, 0));
    }

    if ctx.session.candidates.is_empty() {
        let buf_arc: Arc<str> = Arc::from(ctx.session.buffer.as_str());
        ctx.session.candidates.push(Candidate {
            text: buf_arc.clone(),
            simplified: buf_arc.clone(),
            traditional: buf_arc.clone(),
            hint: Arc::from(""),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from("Raw"),
            weight: 0.0,
            match_level: 0,
        });
    }
    ctx.session.update_state();
    crate::compositor::Compositor::check_auto_commit(ctx)
}

/// 检测「完整拼音 + 辅码字母」模式。
/// 如果 buffer 的最长有效拼音前缀之后有额外字母，且整体不是有效拼音，则返回 (拼音前缀, 辅码后缀)。
pub fn detect_smart_aux(
    buffer: &str,
    syllables: &HashSet<String>,
    mode: qianyan_ime_core::config::SmartAuxMode,
) -> Option<(String, String)> {
    if buffer.len() < 3 {
        return None;
    }
    let bytes = buffer.as_bytes();
    if !bytes.iter().all(|b| b.is_ascii_lowercase()) {
        return None;
    }

    if syllables.contains(buffer) || is_fully_syllabic(buffer, syllables) {
        return None;
    }

    let split_points: Vec<usize> = match mode {
        qianyan_ime_core::config::SmartAuxMode::Greedy => (1..buffer.len()).rev().collect(),
        qianyan_ime_core::config::SmartAuxMode::Minimal => (1..buffer.len()).collect(),
    };

    for split in split_points {
        let prefix = &buffer[..split];
        let suffix = &buffer[split..];

        if is_fully_syllabic(prefix, syllables) {
            return Some((prefix.to_string(), suffix.to_string()));
        }
    }

    None
}

/// 检查字符串是否由一个或多个有效音节组成（无剩余字符）
fn is_fully_syllabic(s: &str, syllables: &HashSet<String>) -> bool {
    if s.is_empty() {
        return true;
    }
    is_fully_syllabic_depth(s, syllables, 0)
}

fn is_fully_syllabic_depth(s: &str, syllables: &HashSet<String>, depth: usize) -> bool {
    if depth > 30 {
        return false;
    }
    if s.is_empty() {
        return true;
    }
    for len in (1..=s.len()).rev() {
        if syllables.contains(&s[..len]) {
            if is_fully_syllabic_depth(&s[len..], syllables, depth + 1) {
                return true;
            }
        }
    }
    false
}
