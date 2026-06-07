use crate::scheme::{InputScheme, SchemeCandidate, SchemeContext};
use crate::FuzzyPinyinConfig;
use fst::automaton::Levenshtein;
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

    fn get_fuzzy_variants(&self, pinyin: &str, context: &SchemeContext) -> Vec<String> {
        if !context.effective_fuzzy {
            if pinyin.bytes().all(|b| b.is_ascii_lowercase()) {
                return vec![pinyin.to_string()];
            }
            return vec![pinyin.to_lowercase()];
        }
        Self::fuzzy_variants_per_segment(pinyin, &context.config.input.fuzzy_config)
    }

    fn fuzzy_variants_per_segment(seg: &str, fuzzy: &FuzzyPinyinConfig) -> Vec<String> {
        crate::pipeline::fuzzy_variants_per_segment(seg, fuzzy)
    }

    fn segment_base(&self, input: &str, base_syllables: &std::collections::HashSet<String>) -> Vec<String> {
        crate::pipeline::compose_utils::segment_base(input, base_syllables)
    }

    fn backtrack_partitions(
        &self,
        base: &[String],
        pos: usize,
        current: &mut Vec<(usize, usize)>,
        result: &mut Vec<Vec<(usize, usize)>>,
        trie: &crate::trie::Trie,
    ) {
        crate::pipeline::compose_utils::backtrack_partitions(base, pos, current, result, trie)
    }

    /// 左到右贪心分段：先匹配最长全音节，否则匹配声母（zh/ch/sh 占 2 字符）
    /// 返回 (segment, is_initial) 对
    pub fn segment_for_abbreviation(input: &str, trie: &crate::trie::Trie) -> Vec<(String, bool)> {
        let two_initials: &[&str] = &["zh", "ch", "sh"];
        let single_initials: &[char] = &['b','p','m','f','d','t','n','l','g','k','h',
            'j','q','x','r','z','c','s','y','w'];

        let mut result = Vec::new();
        let n = input.len();
        let mut pos = 0;

        while pos < n {
            let max_len = (n - pos).min(6);
            let mut matched = false;
            for len in (2..=max_len).rev() {
                if input.is_char_boundary(pos + len) {
                    let candidate = &input[pos..pos + len];
                    if trie.index.contains_key(candidate) {
                        result.push((candidate.to_string(), false));
                        pos += len;
                        matched = true;
                        break;
                    }
                }
            }
            if matched { continue; }

            // 2. 尝试双字符声母 zh ch sh
            if n - pos >= 2 {
                let candidate = &input[pos..pos + 2];
                if two_initials.contains(&candidate) {
                    result.push((candidate.to_string(), true));
                    pos += 2;
                    continue;
                }
            }

            // 3. 单字符声母
            let ch = match input[pos..].chars().next() {
                Some(c) => c,
                None => break,
            };
            if single_initials.contains(&ch) {
                result.push((ch.to_string(), true));
                pos += ch.len_utf8();
            } else {
                break;
            }
        }

        result
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

        let min_results_needed = 500;
        let max_results = 500;

        let pinyin_key: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();

    // 用户词典精确匹配（最高优先级，排在最前）
    // 前缀匹配延后到所有策略之后，避免阻塞简拼/组句/纠错
    let mut user_prefix_matches: Vec<SchemeCandidate> = Vec::new();
    if let Some(profile) = context.active_profiles.first() {
        let dict = context.user_dict.load();
        // 辅助函数：从系统词典查词，返回 (trad, tone, en, stroke_aux, flags)
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
        if let Some(profile_dict) = dict.get(profile) {
            let has_vowel = pinyin_key.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'v'));
            if has_vowel {
                // 精确匹配
                if let Some(words) = profile_dict.get(&pinyin_key) {
                    for (word, weight) in words {
                        if seen.insert(word.clone()) {
                            let (trad, tone, en, sa, flags) = lookup_aux(word, &pinyin_key);
                            final_results.push(SchemeCandidate {
                                text: word.clone(),
                                simplified: word.clone(),
                                traditional: trad,
                                tone,
                                english: en,
                                stroke_aux: sa,
                                weight: *weight,
                                match_level: 3,
                                flags,
                            });
                        }
                    }
                } else {
                    // 前缀匹配 → 暂存到末尾再推入，不阻塞其他策略
                    let mut prefix_keys: Vec<&String> = profile_dict
                        .keys()
                        .filter(|k| k.starts_with(&pinyin_key))
                        .collect();
                    prefix_keys.sort_by_key(|k| k.len());
                    for key in prefix_keys {
                        if let Some(words) = profile_dict.get(key) {
                            for (word, weight) in words {
                                if seen.insert(word.clone()) {
                                    let (trad, tone, en, sa, flags) = lookup_aux(word, key);
                                    user_prefix_matches.push(SchemeCandidate {
                                        text: word.clone(),
                                        simplified: word.clone(),
                                        traditional: trad,
                                        tone,
                                        english: en,
                                        stroke_aux: sa,
                                        weight: (*weight as f64 * 0.8) as u32,
                                        match_level: 0,
                                        flags,
                                    });
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
                                    if seen.insert(word.clone()) {
                                        let (trad, tone, en, sa, flags) = lookup_aux(word, pinyin);
                                        user_prefix_matches.push(SchemeCandidate {
                                            text: word.clone(),
                                            simplified: word.clone(),
                                            traditional: trad,
                                            tone,
                                            english: en,
                                            stroke_aux: sa,
                                            weight: (*weight as f64 * 0.8) as u32,
                                            match_level: 0,
                                            flags,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

        // 策略 1: 全量/简拼/前缀匹配
        // 原始切分检索
        let mut last_matches_raw = Vec::new();
        for (i, part) in raw_parsed.iter().enumerate() {
            let mut matches = Vec::new();
            let pinyin_variants = self.get_fuzzy_variants(&part.pinyin, context);
            for profile in context.active_profiles {
                if let Some(d) = context.tries.get(profile) {
                    for py in &pinyin_variants {
                        if let Some(m) = d.get_all_exact(py) {
                            let mut exact: Vec<_> = m.iter().collect();
                            let n = exact.len().min(min_results_needed);
                            if n > 0 {
                                exact.select_nth_unstable_by_key(n.saturating_sub(1), |r| std::cmp::Reverse(r.weight));
                            }
                            for tr in exact.iter().take(n) {
                                matches.push((
                                    tr.word.to_string(),
                                    tr.trad.to_string(),
                                    tr.tone.to_string(),
                                    tr.en.to_string(),
                                    tr.stroke_aux.to_string(),
                                    tr.weight,
                                    3,
                                    tr.flags,
                                ));
                            }
                        } else if context.config.input.enable_prefix_matching && !py.is_empty() {
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
                            if comp_keys.is_empty() {
                                // 无前缀补全匹配，留空让简拼分支处理
                            } else {
                                comp_keys.sort_by_key(|a| a.len());
                            }
                            comp_keys.truncate(10);
                            for key in comp_keys {
                                if let Some(entries) = d.get_all_exact(&key) {
                                    if py.len() <= 3 && !entries.iter().any(|tr| tr.word.chars().count() == 1) {
                                        continue;
                                    }
                                    for tr in entries.iter() {
                                        matches.push((
                                            tr.word.to_string(),
                                            tr.trad.to_string(),
                                            tr.tone.to_string(),
                                            tr.en.to_string(),
                                            tr.stroke_aux.to_string(),
                                            tr.weight,
                                            1,
                                            tr.flags,
                                        ));
                                        if matches.len() >= max_results {
                                            break;
                                        }
                                    }
                                }
                                if matches.len() >= max_results {
                                    break;
                                }
                            }
                        }
                    }
                    if matches.len() >= max_results {
                        break;
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
                let stroke_aux_lower = m.4.to_lowercase();
                if !stroke_aux_lower.starts_with(&aux_lower) {
                    continue;
                }
            }
            if let Some(aux) = last_part.and_then(|p| p.english_aux.as_ref()) {
                let aux_lower = aux.to_lowercase();
                if !m
                    .3
                    .to_lowercase()
                    .split(',')
                    .any(|part: &str| part.trim().starts_with(&aux_lower))
                {
                    continue;
                }
            }

            if seen.insert(m.0.clone()) {
                let mut cand = SchemeCandidate::new(m.0, m.5);
                cand.traditional = m.1;
                cand.tone = m.2;
                cand.english = m.3;
                cand.stroke_aux = m.4;
                cand.match_level = m.6;
                cand.flags = m.7;
                final_results.push(cand);
            }
        }

        // 策略 2: 混拼/简拼检索 —— 仅在前缀补全无结果时触发
        if final_results.is_empty()
            && context.config.input.enable_abbreviation_matching
        {
            if let Some(d) = context.tries.get("chinese") {
                let abbr_segs = ChineseScheme::segment_for_abbreviation(&pinyin_key, d);
                if abbr_segs.len() > 1 {
                    let mut abbr_results = d.search_abbreviation_mixed(&abbr_segs, 200, context.single_syllables);
                    abbr_results.sort_by_key(|r| std::cmp::Reverse(r.weight));
                    for tr in abbr_results {
                        if final_results.len() >= max_results {
                            break;
                        }
                        let last_part = raw_parsed.last();
                        if let Some(aux) = last_part.and_then(|p| p.stroke_aux.as_ref()) {
                            let aux_lower = aux.to_lowercase();
                            if !tr.stroke_aux.to_lowercase().starts_with(&aux_lower) {
                                continue;
                            }
                        }
                        if let Some(aux) = last_part.and_then(|p| p.english_aux.as_ref()) {
                            let aux_lower = aux.to_lowercase();
                            if !tr
                                .en
                                .to_lowercase()
                                .split(',')
                                .any(|part: &str| part.trim().starts_with(&aux_lower))
                            {
                                continue;
                            }
                        }
                        if seen.insert(tr.word.to_string()) {
                            let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                            cand.traditional = tr.trad.to_string();
                            cand.tone = tr.tone.to_string();
                            cand.english = tr.en.to_string();
                            cand.stroke_aux = tr.stroke_aux.to_string();
                            cand.match_level = 2;
                            cand.flags = tr.flags;
                            final_results.push(cand);
                        }
                    }
                }
            }
        }

        // 策略 3: 长句组合（ComposeTranslator）
        if !query.contains(' ') && !raw_parsed.is_empty() {
            if let Some(d) = context.tries.get("chinese") {
                let base = self.segment_base(&pinyin_key, context.base_syllables);
                let min_syllables = context.config.input.auto_sentence_min_syllables as usize;
                let min_syllables = min_syllables.max(2);
                if base.len() >= 2 && base.len() <= 12
                    && (base.len() >= min_syllables || final_results.is_empty()) {
                    let prefix_without_last: String = base[..base.len() - 1].concat();
                    if d.has_longer_match(&prefix_without_last) {
                        // 去掉最后一个音节的前缀仍在词典中有更长的匹配，
                        // 说明用户可能正在输入一个较长的已知词，跳过组句
                    } else {
                        let mut all_partitions = Vec::new();
                        self.backtrack_partitions(&base, 0, &mut Vec::new(), &mut all_partitions, d);
                        if all_partitions.len() > 100 {
                            all_partitions.truncate(100);
                        }

                        // 加载 ngram 数据，用于评估切分方案中相邻词的衔接概率
                        let ngram_guard = context.ngram_history.load();
                        let profile = context.active_profiles.first();

                        let mut compose_results: Vec<(String, usize, u64)> = Vec::new();
                        for part in &all_partitions {
                            let mut text = String::new();
                            let mut words: Vec<String> = Vec::new();
                            let mut total_freq = 0u64;
                            let mut ok = true;
                            for &(s, e) in part {
                                let py: String = base[s..e].concat();
                                if let Some(entries) = d.get_all_exact(&py) {
                                    if let Some(best) = entries.iter().max_by_key(|r| r.weight) {
                                        words.push(best.word.to_string());
                                        text.push_str(best.word);
                                        total_freq += context.syllable_freq.get(&py).copied().unwrap_or(0);
                                        continue;
                                    }
                                }
                                ok = false;
                                break;
                            }
                            if !ok { continue; }

                            // N-Gram 衔接评分：相邻词对 log(共现次数+1) 累加
                            let mut ngram_bonus = 0u64;
                            if let (Some(_profile_str), Some(profile_ngram)) =
                                (profile.map(|s| s.as_str()), profile.and_then(|p| ngram_guard.get(p.as_str())))
                            {
                                for pair in words.windows(2) {
                                    if let Some(entries) = profile_ngram.get(&pair[0]) {
                                        if let Some((_, count)) = entries.iter().find(|(w, _)| w == &pair[1])
                                        {
                                            let effective = (*count).min(10) as f64;
                                            ngram_bonus +=
                                                ((effective + 1.0).ln() * 500_000.0) as u64;
                                        }
                                    }
                                }
                            }
                            let total_score = total_freq + ngram_bonus;
                            compose_results.push((text, part.len(), total_score));
                        }
                        compose_results.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
                        compose_results.truncate(6);
                        for (text, _, score) in &compose_results {
                            if seen.insert(text.clone()) {
                                let weight = (*score as f64 * 0.001 + 0.1) as u32;
                                let mut cand = SchemeCandidate::new(text.clone(), weight);
                                cand.match_level = 3;
                                final_results.push(cand);
                            }
                        }
                    }
                }
            }
        }

        // 策略 4: 纠错 — 所有策略无结果时，用编辑距离找最近的 key
        if final_results.is_empty() && !query.contains(' ')
            && context.config.input.enable_error_correction {
            let pinyin_only = &pinyin_key;
            if let Some(d) = context.tries.get("chinese") {
                if let Ok(lev) = Levenshtein::new(pinyin_only, 1u32) {
                    // FST Levenshtein 自动机已保证所有返回 key 的距离 ≤ 1
                    // 按 key 长度排序（更短更优），取前 5 个候选 key
                    let mut corr_keys: Vec<String> = Vec::new();
                    let mut stream = d.index.search(lev).into_stream();
                    while let Some((key_bytes, _)) = stream.next() {
                        if let Ok(key) = std::str::from_utf8(key_bytes) {
                            corr_keys.push(key.to_string());
                            if corr_keys.len() >= 20 { break; }
                        }
                    }
                    corr_keys.sort_by_key(|a| a.len());
                    corr_keys.truncate(5);
                    for key in corr_keys {
                        if let Some(entries) = d.get_all_exact(&key) {
                            for tr in entries.iter() {
                                if seen.insert(tr.word.to_string()) {
                                    let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                                    cand.traditional = tr.trad.to_string();
                                    cand.tone = tr.tone.to_string();
                                    cand.english = tr.en.to_string();
                                    cand.stroke_aux = tr.stroke_aux.to_string();
                                    cand.match_level = 1;
                                    cand.flags = tr.flags;
                                    final_results.push(cand);
                                    if final_results.len() >= max_results { break; }
                                }
                            }
                        }
                        if final_results.len() >= max_results { break; }
                    }
                }
            }
        }

        // 方案 4 结束 → 追加用户词前缀匹配（最低优先级，不阻塞其他策略）
        final_results.append(&mut user_prefix_matches);

        final_results
    }

    fn post_process(
        &self,
        _query: &str,
        candidates: &mut Vec<SchemeCandidate>,
        context: &SchemeContext,
    ) {
        const USAGE_BOOST_SCALE: u32 = 50000;   // 5万/次使用，50次封顶≈+283K
        const NGRAM_BOOST_SCALE: u32 = 50000;

        // 自适应加权：基于 usage_history + ngram_history 调整权重
        if let Some(profile) = context.active_profiles.first() {
            // 开关1: 使用频率排序（受历史输入影响）
            if context.config.input.enable_usage_sorting {
                let usage_guard = context.usage_history.load();
                if let Some(profile_usage) = usage_guard.get(profile) {
                    for c in &mut *candidates {
                        if let Some(count) = profile_usage.get(c.simplified.as_str()) {
                            let effective = (*count).min(50);
                            let boost = ((effective as f64 + 1.0).log2() * USAGE_BOOST_SCALE as f64) as u32;
                            c.weight += boost;
                        }
                    }
                }
            }

            // 开关2: 上下文联想（受上文词影响）
            if context.config.input.enable_context_sorting {
                if let Some(last_word) = context.last_word {
                    let ngram_guard = context.ngram_history.load();
                    if let Some(profile_ngram) = ngram_guard.get(profile) {
                        if let Some(entries) = profile_ngram.get(last_word) {
                            let ngram_map: std::collections::HashMap<String, u32> =
                                entries.iter().map(|(w, c)| (w.clone(), *c)).collect();
                            for c in &mut *candidates {
                                if let Some(&count) = ngram_map.get(c.simplified.as_str()) {
                                    let effective = count.min(50);
                                    let boost = ((effective as f64 + 1.0).log2() * NGRAM_BOOST_SCALE as f64) as u32;
                                    c.weight += boost;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 纯 weight 降序排序
        candidates.sort_by(|a, b| b.weight.cmp(&a.weight));

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
