use std::sync::Arc;

use crate::processor::Action;
use crate::EngineContext;

use super::{Candidate, MAX_LOOKUP_LIMIT};
use super::engine::SearchQuery;

/// 主查找入口：执行拼音→候选词搜索
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
    let last_two = ctx
        .session_state
        .get_last_two_words();

    let query = SearchQuery {
        buffer: &ctx.session.buffer,
        profile: &current_profile,
        config: &ctx.config.master_config,
        limit: MAX_LOOKUP_LIMIT,
        filter_mode: ctx.session.filter_mode.clone(),
        aux_filter: &ctx.session.aux_filter,
        context: last_word,
        context_pair: last_two,
    };
    let (results, segments) = ctx.engine.search(query);
    ctx.session.candidates = results;
    ctx.session.best_segmentation = segments;
    ctx.session.has_dict_match = !ctx.session.candidates.is_empty();
    ctx.session.last_lookup_pinyin = ctx.session.buffer.clone();

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
            source: Arc::from("Syllable"),
            weight: 1.0,
            match_level: 3,
            flags: 0,
            });

    }
    ctx.session.update_state();
    crate::compositor::Compositor::check_auto_commit(ctx)
}
