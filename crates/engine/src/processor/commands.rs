use crate::processor::{Action, Command};
use crate::EngineContext;
use std::sync::Arc;

pub fn execute_command(ctx: &mut EngineContext, cmd: Command) -> Action {
    let page_size = ctx.config.page_size();
    match cmd {
        Command::NextPage => {
            ctx.session.next_page(page_size);
            Action::Consume
        }
        Command::PrevPage => {
            ctx.session.prev_page(page_size);
            Action::Consume
        }
        Command::NextCandidate => {
            ctx.session.next_candidate(page_size);
            crate::compositor::Compositor::update_phantom_action(ctx)
        }
        Command::PrevCandidate => {
            ctx.session.prev_candidate(page_size);
            crate::compositor::Compositor::update_phantom_action(ctx)
        }
        Command::Select(idx) => {
            let abs_idx = ctx.session.page + idx;
            if let Some(cand) = ctx.session.candidates.get(abs_idx) {
                let word = cand.text.clone();
                return commit_candidate(ctx, word, abs_idx);
            }
            Action::Consume
        }
        Command::SelectChar(char_idx) => {
            if let Some(cand) = ctx.session.candidates.first() {
                let word = cand.text.as_ref();
                let chars: Vec<char> = word.chars().collect();
                if char_idx < chars.len() {
                    let ch: String = chars[char_idx].to_string();
                    let out = Arc::from(ch.as_str());
                    return commit_candidate(ctx, out, 99);
                }
            }
            Action::Consume
        }
        Command::Commit => {
            if ctx.session.buffer.is_empty() {
                return Action::PassThrough;
            }

            if ctx.config.firefox_space_interrupt() {
                let out = ctx.session.buffer.clone();
                let delete_count = out.chars().count() + 1;
                ctx.reset();
                return Action::DeleteAndEmit {
                    delete: delete_count,
                    insert: out,
                };
            }

            if !ctx.session.candidates.is_empty() {
                let idx = ctx.session.selected;
                if let Some(cand) = ctx.session.candidates.get(idx) {
                    let word = cand.text.clone();
                    return commit_candidate(ctx, word, idx);
                }
            }

            let out = Arc::from(ctx.session.buffer.as_str());
            commit_candidate(ctx, out, 99)
        }
        Command::CommitRaw => {
            if ctx.session.buffer.is_empty() {
                return Action::PassThrough;
            }
            let out = Arc::from(ctx.session.buffer.as_str());
            commit_candidate(ctx, out, 99)
        }
        Command::Clear => {
            ctx.session_state.commit_history.clear();
            let del = ctx.session.phantom_text.chars().count();
            ctx.reset();
            if del > 0 {
                Action::DeleteAndEmit {
                    delete: del,
                    insert: "".into(),
                }
            } else {
                Action::Consume
            }
        }
    }
}

pub(crate) fn commit_candidate(
    ctx: &mut EngineContext,
    mut cand: Arc<str>,
    index: usize,
) -> Action {
    use std::time::{Duration, Instant};

    let now = Instant::now();
    let py = ctx.session.last_lookup_pinyin.clone();

    if !py.is_empty() && index != 99 {
        if now.duration_since(ctx.session_state.last_commit_time) > Duration::from_secs(10) {
            ctx.session_state.commit_history.clear();
        }

        let source = ctx
            .session
            .candidates
            .get(index)
            .map(|c| c.source.clone())
            .unwrap_or_default();
        let last_word_opt = ctx.session_state.get_last_word().map(|s| s.to_string());
        let last_two_opt = ctx.session_state.get_last_two_words()
            .map(|(a, b)| (a.to_string(), b.to_string()));
        crate::processor::learning::record_usage(
            ctx, &py, &cand, &source,
            last_word_opt.as_deref(),
            last_two_opt.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
        );
        ctx.session_state
            .add_to_history(py.clone(), cand.to_string());

        ctx.session_state.update_commit_time();
    }

    if ctx.session_state.is_english_mode()
        && !cand.is_empty()
        && cand.chars().next_back().unwrap_or(' ').is_alphanumeric()
    {
        let mut s = cand.to_string();
        s.push(' ');
        cand = Arc::from(s);
    }

    let del = ctx.session.phantom_text.chars().count();
    ctx.session.clear_composing();
    Action::DeleteAndEmit {
        delete: del,
        insert: cand.to_string(),
    }
}


