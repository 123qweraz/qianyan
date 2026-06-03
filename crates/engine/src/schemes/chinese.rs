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

    fn segment_buffer(&self, input: &str, _delimiters: &str, context: &SchemeContext) -> Vec<String> {
        if !input.is_ascii() {
            return Vec::new();
        }
        let normalized = if input.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()) {
            input.to_string()
        } else {
            input.to_ascii_lowercase()
        };
        if normalized.is_empty() {
            return vec![];
        }
        crate::pipeline::DefaultSegmentor::viterbi_segment(&normalized, context.syllable_freq, context.base_syllables)
    }

    /// 左到右贪心分段：先匹配最长全音节，否则匹配声母（zh/ch/sh 占 2 字符）
    /// 返回 (segment, is_initial) 对
    pub fn segment_for_abbreviation(input: &str, syllables: &std::collections::HashSet<String>) -> Vec<(String, bool)> {
        let two_initials: &[&str] = &["zh", "ch", "sh"];
        let single_initials: &[char] = &['b','p','m','f','d','t','n','l','g','k','h',
            'j','q','x','r','z','c','s','y','w'];

        let mut result = Vec::new();
        let n = input.len();
        let mut pos = 0;

        while pos < n {
            // 1. 尝试最长全音节（至少2字符，排除单字母如 m/z/n）
            let max_len = (n - pos).min(6);
            let mut matched = false;
            for len in (2..=max_len).rev() {
                if input.is_char_boundary(pos + len) {
                    let candidate = &input[pos..pos + len];
                    if syllables.contains(candidate) {
                        result.push((candidate.to_string(), false)); // full syllable
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

impl InputScheme for ChineseScheme {
    fn lookup(&self, query: &str, context: &SchemeContext) -> Vec<SchemeCandidate> {
        let raw_parsed = self.parse_buffer(query);
        let mut final_results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let min_results_needed = 500;
        let max_results = 500;

    // 用户词典精确匹配（最高优先级，排在最前）
    // 前缀匹配延后到所有策略之后，避免阻塞简拼/组句/纠错
    let mut user_prefix_matches: Vec<SchemeCandidate> = Vec::new();
    if let Some(profile) = context.active_profiles.first() {
        let pinyin_key: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
        let dict = context.user_dict.load();
        if let Some(profile_dict) = dict.get(profile) {
            let has_vowel = pinyin_key.chars().any(|c| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'v'));
            if has_vowel {
                // 精确匹配
                if let Some(words) = profile_dict.get(&pinyin_key) {
                    for (word, weight) in words {
                        if seen.insert(word.clone()) {
                            final_results.push(SchemeCandidate {
                                text: word.clone(),
                                simplified: word.clone(),
                                traditional: word.clone(),
                                tone: String::from("User"),
                                english: String::new(),
                                stroke_aux: String::new(),
                                weight: *weight,
                                match_level: 3,
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
                                    user_prefix_matches.push(SchemeCandidate {
                                        text: word.clone(),
                                        simplified: word.clone(),
                                        traditional: word.clone(),
                                        tone: key.clone(),
                                        english: String::new(),
                                        stroke_aux: String::new(),
                                        weight: (*weight as f64 * 0.8) as u32,
                                        match_level: 0, // below system prefix (level 1)
                                    });
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
                                comp_keys.sort_by(|a, b| a.len().cmp(&b.len()));
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
                final_results.push(cand);
            }
        }

        // 策略 2: 混拼/简拼检索 —— 仅在前缀补全无结果时触发
        if final_results.is_empty()
            && context.config.input.enable_abbreviation_matching
        {
            let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            let abbr_segs = ChineseScheme::segment_for_abbreviation(&pinyin_only, context.syllables);
            if abbr_segs.len() > 1 {
                if let Some(d) = context.tries.get("chinese") {
                    let mut abbr_results = d.search_abbreviation_mixed(&abbr_segs, context.syllables, 200);
                    abbr_results.sort_by(|a, b| b.weight.cmp(&a.weight));
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
                            final_results.push(cand);
                        }
                    }
                }
            }
        }

        // 策略 3: 长句组合（ComposeTranslator）
        if !query.contains(' ') && !raw_parsed.is_empty() {
            let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            if let Some(d) = context.tries.get("chinese") {
                let base = self.segment_base(&pinyin_only, context.base_syllables);
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
                        let mut compose_results: Vec<(String, usize, u64)> = Vec::new();
                        for part in &all_partitions {
                            let mut text = String::new();
                            let mut total_freq = 0u64;
                            let mut ok = true;
                            for &(s, e) in part {
                                let py: String = base[s..e].concat();
                                if let Some(entries) = d.get_all_exact(&py) {
                                    if let Some(best) = entries.iter().max_by_key(|r| r.weight) {
                                        text.push_str(best.word);
                                        total_freq += context.syllable_freq.get(&py).copied().unwrap_or(0);
                                        continue;
                                    }
                                }
                                ok = false;
                                break;
                            }
                            if ok {
                                compose_results.push((text, part.len(), total_freq));
                            }
                        }
                        compose_results.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
                        compose_results.truncate(6);
                        for (text, _, freq) in &compose_results {
                            if seen.insert(text.clone()) {
                                let weight = (*freq as f64 * 0.001 + 0.1) as u32;
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
        if final_results.is_empty() && !query.contains(' ') {
            let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            if let Some(d) = context.tries.get("chinese") {
                if let Ok(lev) = Levenshtein::new(&pinyin_only, 1u32) {
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
                    corr_keys.sort_by(|a, b| a.len().cmp(&b.len()));
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
        query: &str,
        candidates: &mut Vec<SchemeCandidate>,
        context: &SchemeContext,
    ) {
        let raw_parsed = self.parse_buffer(query);
        let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
        let delimiters = &context.config.input.segmentation_delimiters;
        let smart_segments = self.segment_buffer(&pinyin_only, delimiters, context);
        let input_syllables = if smart_segments.is_empty() {
            raw_parsed.len()
        } else {
            smart_segments.len()
        };

        // 自适应加权：基于 usage_history + ngram_history 调整权重
        if let Some(profile) = context.active_profiles.first() {
            let usage_guard = context.usage_history.load();
            let ngram_guard = context.ngram_history.load();

            // 用法历史（MRU 衰减 + 频率对数缩放）
            if let Some(profile_usage) = usage_guard.get(profile) {
                if let Some(entries) = profile_usage.get(&pinyin_only) {
                    let usage_map: std::collections::HashMap<String, (usize, u32)> = entries
                        .iter()
                        .enumerate()
                        .map(|(pos, (w, c))| (w.clone(), (pos, *c)))
                        .collect();
                    for c in &mut *candidates {
                        // level=0 (user prefix) never receives usage boosts —
                        // it must stay below system prefix (level 1) in all cases
                        if c.match_level == 0 {
                            continue;
                        }
                        if let Some(&(pos, count)) = usage_map.get(c.simplified.as_str()) {
                            c.weight += (crate::pipeline::compute_decay_boost(pos, count) as u32).max(1);
                        }
                    }

                    // 置顶首候选：最近常用词自动排第一
                    if context.config.input.enable_fixed_first_candidate {
                        if let Some((fixed_word, fixed_count)) = entries.first() {
                            if *fixed_count >= 3 {
                                for c in &mut *candidates {
                                    if c.match_level == 0 {
                                        continue;
                                    }
                                    if c.simplified.as_str() == fixed_word {
                                        c.weight += 20000000;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 上下文联想 (N-Gram) 加权
            if let Some(last_word) = context.last_word {
                if let Some(profile_ngram) = ngram_guard.get(profile) {
                    if let Some(entries) = profile_ngram.get(last_word) {
                        let ngram_map: std::collections::HashMap<String, u32> =
                            entries.iter().map(|(w, c)| (w.clone(), *c)).collect();
                        for c in &mut *candidates {
                            if c.match_level == 0 {
                                continue;
                            }
                            if let Some(&count) = ngram_map.get(c.simplified.as_str()) {
                                let effective = count.min(10);
                                let boost = (1.0 + (effective as f64).ln()).max(0.0)
                                    * crate::pipeline::NGRAM_BOOST_SCALE;
                                c.weight += (boost.min(crate::pipeline::MAX_NGRAM_BOOST) as u32).max(1);
                            }
                        }
                    }
                }
            }
        }

        candidates.sort_by(|a, b| {
            let get_score = |m: &SchemeCandidate| -> i64 {
                let level = m.match_level as i64;
                let weight = m.weight as i64;
                let char_count = m.text.chars().count() as i64;
            let mut score = if level == 3 {
                30_000_000 + context.config.input.ranking.exact_match_bonus as i64
            } else {
                    level * 10_000_000
                };
                if level == 2 && char_count == input_syllables as i64 {
                    score += 10_000_000;
                }
                score += weight;
                let len_diff = (char_count - input_syllables as i64).max(0);
                score -= len_diff * (if level == 2 { 10000 } else { 1000 });
                score
            };
            get_score(b).cmp(&get_score(a))
        });

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
