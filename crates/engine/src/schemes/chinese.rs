use crate::scheme::{InputScheme, SchemeCandidate, SchemeContext};
use fst::{Automaton, IntoStreamer, Streamer};

pub struct ChineseScheme;

#[derive(Debug, Clone)]
struct ParsedPart {
    pinyin: String,
    stroke_aux: Option<String>,
    english_aux: Option<String>,
}

impl Default for ChineseScheme {
    fn default() -> Self {
        Self::new()
    }
}

impl ChineseScheme {
    pub fn new() -> Self {
        Self
    }

    fn parse_buffer(&self, buffer: &str) -> Vec<ParsedPart> {
        let buffer_normalized = buffer.to_lowercase();
        let parts: Vec<&str> = buffer_normalized
            .split(' ')
            .filter(|s| !s.is_empty())
            .collect();
        let mut result = Vec::new();

        for part in parts {
            let mut stroke_aux = None;
            let mut english_aux = None;

            let pinyin_end = part
                .char_indices()
                .find(|(i, c)| {
                    *c == ';' || c.is_ascii_digit() || (*i > 0 && c.is_ascii_uppercase())
                })
                .map(|(i, _)| i)
                .unwrap_or(part.len());

            let pinyin = part[..pinyin_end].to_string();
            let mut rest = &part[pinyin_end..];

            if rest.starts_with(';') {
                rest = &rest[1..];
                let stroke_end = rest
                    .find(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase())
                    .unwrap_or(rest.len());
                let s = &rest[..stroke_end];
                if !s.is_empty() {
                    stroke_aux = Some(s.to_string());
                }
                rest = &rest[stroke_end..];
            }

            if !rest.is_empty() && rest.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                let english_end = rest
                    .find(|c: char| c.is_ascii_digit())
                    .unwrap_or(rest.len());
                let e = &rest[..english_end];
                if !e.is_empty() {
                    english_aux = Some(e.to_string());
                }
            }

            result.push(ParsedPart {
                pinyin,
                stroke_aux,
                english_aux,
            });
        }
        result
    }

    /// 左到右贪心分段：先匹配最长全音节，否则匹配声母（zh/ch/sh 占 2 字符）
    /// 返回 (segment, is_initial) 对
    pub fn segment_for_abbreviation(input: &str, trie: &crate::trie::Trie) -> Vec<(String, bool)> {
        crate::pipeline::compose_utils::segment_for_abbreviation(input, trie)
    }
}

/// 递归检查用户词典词条的拼音是否匹配简拼段（如 "fzm" → "fuzhuma"）
fn match_user_dict_abbreviation(
    pinyin: &str,
    segments: &[(String, bool)],
    trie: &crate::trie::Trie,
    single_syllables: &std::collections::HashSet<String>,
) -> bool {
    fn recursive(
        pinyin: &str,
        segments: &[(String, bool)],
        trie: &crate::trie::Trie,
        single_syllables: &std::collections::HashSet<String>,
    ) -> bool {
        if segments.is_empty() {
            return pinyin.is_empty();
        }
        let first_seg = &segments[0].0;
        let is_initial = segments[0].1;
        let max_len = pinyin.len().min(6);
        for len in 1..=max_len {
            if !pinyin.is_char_boundary(len) {
                continue;
            }
            let syl = &pinyin[..len];
            if !trie.index.contains_key(syl) {
                continue;
            }
            let matched = if is_initial {
                (single_syllables.is_empty() || single_syllables.contains(syl))
                    && syl.starts_with(first_seg.as_str())
            } else {
                syl == first_seg.as_str()
            };
            if matched && recursive(&pinyin[len..], &segments[1..], trie, single_syllables) {
                return true;
            }
        }
        false
    }
    recursive(pinyin, segments, trie, single_syllables)
}

impl InputScheme for ChineseScheme {
    fn lookup(&self, query: &str, context: &SchemeContext) -> Vec<SchemeCandidate> {
        let raw_parsed = self.parse_buffer(query);
        let mut final_results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let max_results = match context.config.input.rare_char_mode {
            qianyan_ime_core::config::RareCharMode::CommonOnly => 500,
            _ => 2000,
        };
        let min_results_needed = max_results;

        let pinyin_key: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();

        // 辅助函数：从系统词典查词
        let lookup_aux = |word: &str, py: &str| -> (String, String, String, String, u8) {
            if let Some(trie) = context.tries.get("chinese") {
                if let Some(entries) = trie.get_all_exact(py) {
                    if let Some(tr) = entries.iter().find(|t| t.word == word) {
                        return (tr.trad.to_string(), tr.tone.to_string(),
                                tr.en.to_string(), tr.stroke_aux.to_string(), tr.flags);
                    }
                }
            }
            (word.to_string(), String::new(), String::new(), String::new(), 0)
        };

        fn build_cand(
            word: String, trad: String, tone: String, en: String, sa: String,
            weight: u32, match_level: u8, flags: u8,
        ) -> SchemeCandidate {
            let mut cand = SchemeCandidate::new(word.clone(), weight);
            cand.simplified = word;
            cand.traditional = trad;
            cand.tone = tone;
            cand.english = en;
            cand.stroke_aux = sa;
            cand.match_level = match_level;
            cand.flags = flags;
            cand
        }

        // ── 用户词典匹配 ──
        let mut user_prefix_matches: Vec<SchemeCandidate> = Vec::new();
        if let Some(profile) = context.active_profiles.first() {
            let dict = context.user_dict.load();
            if let Some(profile_dict) = dict.get(profile) {
                let has_vowel = pinyin_key.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'v'));
                let threshold = context.config.input.word_learn_threshold;
                if has_vowel {
                    // 精确匹配
                    if let Some(words) = profile_dict.get(&pinyin_key) {
                        for (word, weight) in words {
                            if *weight < threshold { continue; }
                            if seen.insert(word.clone()) {
                                let (trad, tone, en, sa, flags) = lookup_aux(word, &pinyin_key);
                                final_results.push(build_cand(
                                    word.clone(), trad, tone, en, sa, *weight, 3, flags,
                                ));
                            }
                        }
                    }
                    // 前缀匹配 → match_level=1，参与正常排序
                    {
                        let max_extra = pinyin_key.len() + 5;
                        let mut prefix_keys: Vec<&String> = profile_dict
                            .keys()
                            .filter(|k| k.starts_with(&pinyin_key) && k.len() <= max_extra && *k != &pinyin_key)
                            .collect();
                        prefix_keys.sort_by_key(|k| k.len());
                        for key in prefix_keys {
                            if let Some(words) = profile_dict.get(key) {
                                for (word, weight) in words {
                                    if *weight < threshold { continue; }
                                    if seen.insert(word.clone()) {
                                        let (trad, tone, en, sa, flags) = lookup_aux(word, key);
                                        user_prefix_matches.push(build_cand(
                                            word.clone(), trad, tone, en, sa,
                                            ((*weight as f64 * 0.8).max(1.0)) as u32, 1, flags,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                } else if context.config.input.enable_abbreviation_matching {
                    if let Some(trie) = context.tries.get("chinese") {
                        let abbr_segs = ChineseScheme::segment_for_abbreviation(&pinyin_key, trie);
                        if !abbr_segs.is_empty() {
                            for (pinyin, words) in profile_dict.iter() {
                                if match_user_dict_abbreviation(pinyin, &abbr_segs, trie, context.single_syllables) {
                                    for (word, weight) in words {
                                        if *weight < threshold { continue; }
                                        if seen.insert(word.clone()) {
                                            let (trad, tone, en, sa, flags) = lookup_aux(word, pinyin);
                                            user_prefix_matches.push(build_cand(
                                                word.clone(), trad, tone, en, sa,
                                                ((*weight as f64 * 0.8).max(1.0)) as u32, 1, flags,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── 策略 1: 逐段精确 / 前缀匹配 ──
        let mut last_matches_raw = Vec::new();
        for (i, part) in raw_parsed.iter().enumerate() {
            let mut matches = Vec::new();
            for profile in context.active_profiles {
                if let Some(d) = context.tries.get(profile) {
                    let py = &part.pinyin;
                    if let Some(m) = d.get_all_exact(py) {
                        let mut exact: Vec<_> = m.iter().collect();
                        let n = exact.len().min(min_results_needed);
                        if n > 0 {
                            exact.select_nth_unstable_by_key(n.saturating_sub(1), |r| std::cmp::Reverse(r.weight));
                        }
                        for tr in exact.iter().take(n) {
                            matches.push((
                                tr.word.to_string(), tr.trad.to_string(), tr.tone.to_string(),
                                tr.en.to_string(), tr.stroke_aux.to_string(), tr.weight, 3u8, tr.flags,
                            ));
                        }
                    }
                    if i == raw_parsed.len() - 1 && context.config.input.enable_prefix_matching && !py.is_empty() {
                        let matcher = fst::automaton::Str::new(py).starts_with();
                        let mut comp_keys: Vec<String> = Vec::new();
                        let max_key_len = if py.len() <= 3 { 6usize } else { py.len() + 2 };
                        let mut stream = d.index.search(matcher).into_stream();
                        while let Some((key_bytes, _)) = stream.next() {
                            if let Ok(key) = std::str::from_utf8(key_bytes) {
                                let klen = key.len();
                                if klen > py.len() && klen <= max_key_len {
                                    comp_keys.push(key.to_string());
                                    if comp_keys.len() >= 30 { break; }
                                }
                            }
                        }
                        comp_keys.sort_by_key(|a| a.len());
                        comp_keys.truncate(10);
                        for key in comp_keys {
                            if let Some(entries) = d.get_all_exact(&key) {
                                if py.len() <= 3 && !entries.iter().any(|tr| tr.word.chars().count() == 1) {
                                    continue;
                                }
                                for tr in entries.iter() {
                                    matches.push((
                                        tr.word.to_string(), tr.trad.to_string(), tr.tone.to_string(),
                                        tr.en.to_string(), tr.stroke_aux.to_string(), tr.weight, 1u8, tr.flags,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            if i == raw_parsed.len() - 1 {
                last_matches_raw = matches;
            }
        }

        // 辅码过滤
        for m in last_matches_raw {
            let last_part = raw_parsed.last();
            if let Some(aux) = last_part.and_then(|p| p.stroke_aux.as_ref()) {
                let aux_lower = aux.to_lowercase();
                if !m.4.to_lowercase().starts_with(&aux_lower) { continue; }
            }
            if let Some(aux) = last_part.and_then(|p| p.english_aux.as_ref()) {
                let aux_lower = aux.to_lowercase();
                if !m.3.to_lowercase().split(',').any(|part: &str| part.trim().starts_with(&aux_lower)) {
                    continue;
                }
            }
            if seen.insert(m.0.clone()) {
                final_results.push(build_cand(m.0, m.1, m.2, m.3, m.4, m.5, m.6, m.7));
            }
        }

        // ── 策略 2: 混拼/简拼检索 —— 仅策略 1 无结果时触发 ──
        if final_results.is_empty()
            && context.config.input.enable_abbreviation_matching
        {
            if let Some(d) = context.tries.get("chinese") {
                let abbr_segs = ChineseScheme::segment_for_abbreviation(&pinyin_key, d);
                if abbr_segs.len() > 1 {
                    let mut abbr_results = d.search_abbreviation_mixed(&abbr_segs, 200, context.single_syllables);
                    abbr_results.sort_by_key(|r| std::cmp::Reverse(r.weight));
                    for tr in abbr_results {
                        if final_results.len() >= max_results { break; }
                        let last_part = raw_parsed.last();
                        if let Some(aux) = last_part.and_then(|p| p.stroke_aux.as_ref()) {
                            let aux_lower = aux.to_lowercase();
                            if !tr.stroke_aux.to_lowercase().starts_with(&aux_lower) { continue; }
                        }
                        if let Some(aux) = last_part.and_then(|p| p.english_aux.as_ref()) {
                            let aux_lower = aux.to_lowercase();
                            if !tr.en.to_lowercase().split(',').any(|part: &str| part.trim().starts_with(&aux_lower)) {
                                continue;
                            }
                        }
                        if seen.insert(tr.word.to_string()) {
                            final_results.push(build_cand(
                                tr.word.to_string(), tr.trad.to_string(), tr.tone.to_string(),
                                tr.en.to_string(), tr.stroke_aux.to_string(), tr.weight, 2, tr.flags,
                            ));
                        }
                    }
                }
            }
        }

        // ── 策略 3: 相似拼音纠错 —— 仅前面策略都无结果时触发 ──
        if final_results.is_empty() && !query.contains(' ')
            && context.config.input.enable_error_correction
        {
            if let Some(d) = context.tries.get("chinese") {
                let similar = crate::pipeline::find_similar_pinyin(
                    &pinyin_key, d, Some(&context.config.input.fuzzy_config),
                    context.syllable_freq, context.base_syllables,
                );
                for m in &similar {
                    if let Some(entries) = d.get_all_exact(&m.pinyin) {
                        for tr in entries.iter() {
                            if seen.insert(tr.word.to_string()) {
                                final_results.push(build_cand(
                                    tr.word.to_string(), tr.trad.to_string(), tr.tone.to_string(),
                                    tr.en.to_string(), tr.stroke_aux.to_string(), tr.weight, 1, tr.flags,
                                ));
                                if final_results.len() >= max_results { break; }
                            }
                        }
                    }
                    if final_results.len() >= max_results { break; }
                }
            }
        }

        // ── 策略 4: 词图 Viterbi 组句 —— 仅前面策略都无结果时触发 ──
        if final_results.is_empty() && !query.contains(' ') && !raw_parsed.is_empty() {
            if let Some(d) = context.tries.get("chinese") {
                if pinyin_key.len() <= 24 {
                    let ngram_guard = context.ngram_history.load();
                    let profile = context.active_profiles.first().cloned().unwrap_or_default();
                    let paths = crate::pipeline::compose::compose(
                        &pinyin_key, d, &ngram_guard, context.syllable_freq, context.base_syllables, &profile,
                    );
                    for path in &paths {
                        let text: String = path.words.iter().map(|w| w.word.as_str()).collect();
                        if seen.insert(text.clone()) {
                            let weight = (path.score * 1000.0) as u32;
                            final_results.push(build_cand(
                                text.clone(), text, String::new(), String::new(), String::new(),
                                weight.max(1), 3, 0,
                            ));
                            if final_results.len() >= max_results { break; }
                        }
                    }
                }
            }
        }

        // ── 追加用户词前缀匹配（不阻塞其他策略，但比系统结果优先级低）──
        final_results.append(&mut user_prefix_matches);

        final_results
    }

    fn post_process(
        &self,
        _query: &str,
        candidates: &mut Vec<SchemeCandidate>,
        context: &SchemeContext,
    ) {
        if let Some(profile) = context.active_profiles.first() {
            if context.config.input.enable_context_sorting {
                let ngram_guard = context.ngram_history.load();
                if let Some(profile_ngram) = ngram_guard.get(profile) {
                    // bigram: key=last_word
                    if let Some(last_word) = context.last_word {
                        if let Some(entries) = profile_ngram.get(last_word) {
                            let ngram_map: std::collections::HashMap<String, u32> =
                                entries.iter().map(|(w, c)| (w.clone(), *c)).collect();
                            for c in &mut *candidates {
                                if let Some(&count) = ngram_map.get(c.simplified.as_str()) {
                                    let effective = count.min(40) as u32;
                                    let boost = effective.saturating_mul(50_000_000);
                                    c.weight = c.weight.saturating_add(boost);
                                }
                            }
                        }
                    }

                    // trigram: key="prev2|prev1"
                    if let Some((prev2, prev1)) = context.last_two_words {
                        let trigram_key = format!("{}|{}", prev2, prev1);
                        if let Some(entries) = profile_ngram.get(&trigram_key) {
                            for c in &mut *candidates {
                                if let Some(&count) = entries.iter()
                                    .find(|(w, _)| w == c.simplified.as_str())
                                    .map(|(_, c)| c)
                                {
                                    let boost = count.min(40).saturating_mul(60_000_000);
                                    c.weight = c.weight.saturating_add(boost);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 添加 match_level 基础分，确保不同策略结果可比较
        for c in &mut *candidates {
            let level_base = match c.match_level {
                3 => 30_000_000u32,
                2 => 20_000_000,
                1 => 10_000_000,
                _ => 0,
            };
            c.weight = c.weight.saturating_add(level_base);
        }

        // 纯 weight 降序排序
        candidates.sort_by(|a, b| b.weight.cmp(&a.weight));

        // 全局 MRU：最近选的词排最前（不按拼音分组，同词不同拼音也能置顶）
        if context.config.input.enable_auto_reorder {
            if let Some(profile) = context.active_profiles.first() {
                let order_guard = context.user_order.load();
                if let Some(word_list) = order_guard.get(profile) {
                    for word in word_list.iter().rev() {
                        if let Some(idx) = candidates.iter().position(|c| c.text == *word || c.simplified == *word) {
                            let cand = candidates.remove(idx);
                            candidates.insert(0, cand);
                        }
                    }
                }
            }
        }

        // 繁体开关
        if context.config.input.enable_traditional {
            for c in &mut *candidates {
                if !c.traditional.is_empty() {
                    c.text = c.traditional.clone();
                }
            }
        }
    }
}
