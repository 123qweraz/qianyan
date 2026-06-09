use crate::config_manager::UserDictData;
use arc_swap::ArcSwap;
use std::sync::Arc;

pub fn update_mru(
    history: &Arc<ArcSwap<UserDictData>>,
    profile: &str,
    key: &str,
    word: &str,
    sort_by_count: bool,
) -> Vec<(String, u32)> {
    let mut result = Vec::new();
    history.rcu(|hist| {
        let mut clone = (**hist).clone();
        let entries = clone
            .entry(profile.to_string())
            .or_default()
            .entry(key.to_string())
            .or_default();

        if let Some(pos) = entries.iter().position(|(w, _)| w == word) {
            if sort_by_count {
                entries[pos].1 += 1;
            } else {
                let old = entries[pos].1;
                entries.remove(pos);
                entries.insert(0, (word.to_string(), old + 1));
            }
        } else {
            if sort_by_count {
                entries.push((word.to_string(), 1));
            } else {
                entries.insert(0, (word.to_string(), 1));
            }
        }

        if sort_by_count {
            entries.sort_by_key(|b| std::cmp::Reverse(b.1));
        } else if entries.len() > 10 {
            entries.truncate(10);
        }

        result = entries.clone();
        Arc::new(clone)
    });
    result
}

fn is_boundary_stopword(word: &str) -> bool {
    let stopwords = "的了和是在有而及与或之为其于以到等说着也就都吧呢吗啊呀让把给被";
    let first = word.chars().next().unwrap_or(' ');
    let last = word.chars().last().unwrap_or(' ');
    stopwords.contains(first) || stopwords.contains(last)
}

pub fn record_usage(
    ctx: &mut crate::EngineContext,
    _pinyin: &str,
    word: &str,
    source: &std::sync::Arc<str>,
    context: Option<&str>,
) {
    if word.is_empty() {
        return;
    }
    // 过滤以停用词开头或结尾的碎片词（如"的学习"、"现在的"）
    if is_boundary_stopword(word) {
        return;
    }

    let profile = ctx.session_state.get_current_profile();
    let word_len = word.chars().count();

    if ctx.config.enable_auto_reorder() {
        // 按拼音记录用户选词顺序（无计数，纯 MRU 排序）
        if !_pinyin.is_empty() {
            ctx.config.insert_usage_order(&profile, word);
        }
        ctx.engine.clear_cache();

        // 上下文 ngram
        if let Some(ctx_str) = context {
            let updated =
                update_mru(&ctx.config.ngram_history, &profile, ctx_str, word, false);
            ctx.config.insert_ngram(&profile, ctx_str, &updated);
        }
    }

    // 反查系统词典获取拼音（用于新词发现）
    let correct_pinyin = match ctx.engine.get_or_load_trie(&profile) {
        Some(t) => {
            if let Some(py) = t.lookup_pinyin(word) {
                py.to_string()
            } else {
                let mut chars_pinyin = String::new();
                let mut ok = true;
                for ch in word.chars() {
                    let ch_str = ch.to_string();
                    match t.lookup_pinyin(&ch_str) {
                        Some(py) => chars_pinyin.push_str(py),
                        None => { ok = false; break; }
                    }
                }
                if ok && !chars_pinyin.is_empty() {
                    chars_pinyin
                } else {
                    _pinyin.to_string()
                }
            }
        }
        None => _pinyin.to_string(),
    };

    let is_valid_pinyin = correct_pinyin.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'v'));
    if ctx.config.master_config.input.enable_word_discovery
        && is_valid_pinyin
        && word_len > 1
        && !ctx.engine.has_word_in_dict(&profile, word)
        && source.as_ref() != "Compose"
        && source.as_ref() != "Table (Abbr)"
    {
        let updated = update_mru(&ctx.config.learned_words, &profile, &correct_pinyin, word, true);
        ctx.config.insert_learned(&profile, &correct_pinyin, &updated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_dict_data_type() {
        let data: UserDictData = UserDictData::new();
        assert!(data.is_empty());
    }

    #[test]
    fn test_user_dict_data_insert() {
        let mut data: UserDictData = UserDictData::new();
        data.entry("profile1".to_string())
            .or_default()
            .entry("ni".to_string())
            .or_default()
            .push(("你".to_string(), 1));

        assert_eq!(data.len(), 1);
        let profile_data = data.get("profile1").expect("profile1 should exist");
        assert_eq!(profile_data.len(), 1);
    }
}
