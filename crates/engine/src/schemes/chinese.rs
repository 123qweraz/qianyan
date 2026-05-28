use crate::scheme::{InputScheme, SchemeCandidate, SchemeContext};
use crate::FuzzyPinyinConfig;
use fst::automaton::Levenshtein;
use fst::{Automaton, IntoStreamer, Streamer};

fn lev_distance(a: &str, b: &str) -> u32 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let m = ac.len();
    let n = bc.len();
    let mut dp = vec![0u32; (m + 1) * (n + 1)];
    for i in 0..=m { dp[i * (n + 1)] = i as u32; }
    for j in 0..=n { dp[j] = j as u32; }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if ac[i - 1] == bc[j - 1] { 0 } else { 1 };
            let idx = i * (n + 1) + j;
            dp[idx] = (dp[(i - 1) * (n + 1) + j] + 1)
                .min(dp[i * (n + 1) + (j - 1)] + 1)
                .min(dp[(i - 1) * (n + 1) + (j - 1)] + cost);
        }
    }
    dp[m * (n + 1) + n]
}

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
        if !context.config.input.enable_fuzzy_pinyin {
            if pinyin.bytes().all(|b| b.is_ascii_lowercase()) {
                return vec![pinyin.to_string()];
            }
            return vec![pinyin.to_lowercase()];
        }
        Self::fuzzy_variants_per_segment(pinyin, &context.config.input.fuzzy_config)
    }

    fn fuzzy_variants_per_segment(seg: &str, fuzzy: &FuzzyPinyinConfig) -> Vec<String> {
        let pinyin_lower = if seg.bytes().all(|b| b.is_ascii_lowercase()) {
            seg.to_string()
        } else {
            seg.to_lowercase()
        };
        let mut new_variants = std::collections::HashSet::new();
        new_variants.insert(pinyin_lower);

        // Initial-consonant substitutions (one pass over snapshot, no chaining)
        let initial_list: Vec<String> = new_variants.iter().cloned().collect();
        for v in initial_list {
            if fuzzy.z_zh {
                if v.starts_with("zh") { new_variants.insert(v.replacen("zh", "z", 1)); }
                else if v.starts_with("z") { new_variants.insert(v.replacen("z", "zh", 1)); }
            }
            if fuzzy.c_ch {
                if v.starts_with("ch") { new_variants.insert(v.replacen("ch", "c", 1)); }
                else if v.starts_with("c") { new_variants.insert(v.replacen("c", "ch", 1)); }
            }
            if fuzzy.s_sh {
                if v.starts_with("sh") { new_variants.insert(v.replacen("sh", "s", 1)); }
                else if v.starts_with("s") { new_variants.insert(v.replacen("s", "sh", 1)); }
            }
            if fuzzy.n_l {
                if v.starts_with('n') { new_variants.insert(v.replacen('n', "l", 1)); }
                else if v.starts_with('l') { new_variants.insert(v.replacen('l', "n", 1)); }
            }
            if fuzzy.r_l {
                if v.starts_with('r') { new_variants.insert(v.replacen('r', "l", 1)); }
                else if v.starts_with('l') { new_variants.insert(v.replacen('l', "r", 1)); }
            }
            if fuzzy.f_h {
                if v.starts_with('f') { new_variants.insert(v.replacen('f', "h", 1)); }
                else if v.starts_with('h') { new_variants.insert(v.replacen('h', "f", 1)); }
            }
        }

        // Final-substitution variants (one pass over snapshot)
        let final_list: Vec<String> = new_variants.iter().cloned().collect();
        for v in final_list {
            if fuzzy.an_ang {
                if v.ends_with("ang") { new_variants.insert(format!("{}an", &v[..v.len() - 3])); }
                else if v.ends_with("an") { new_variants.insert(format!("{}ang", &v[..v.len() - 2])); }
            }
            if fuzzy.en_eng {
                if v.ends_with("eng") { new_variants.insert(format!("{}en", &v[..v.len() - 3])); }
                else if v.ends_with("en") { new_variants.insert(format!("{}eng", &v[..v.len() - 2])); }
            }
            if fuzzy.in_ing {
                if v.ends_with("ing") { new_variants.insert(format!("{}in", &v[..v.len() - 3])); }
                else if v.ends_with("in") { new_variants.insert(format!("{}ing", &v[..v.len() - 2])); }
            }
            if fuzzy.ian_iang {
                if v.ends_with("iang") { new_variants.insert(format!("{}ian", &v[..v.len() - 4])); }
                else if v.ends_with("ian") { new_variants.insert(format!("{}iang", &v[..v.len() - 3])); }
            }
            if fuzzy.uan_uang {
                if v.ends_with("uang") { new_variants.insert(format!("{}uan", &v[..v.len() - 4])); }
                else if v.ends_with("uan") { new_variants.insert(format!("{}uang", &v[..v.len() - 3])); }
            }
            if fuzzy.u_v {
                if v.contains('u') { new_variants.insert(v.replace('u', "v")); }
                else if v.contains('v') { new_variants.insert(v.replace('v', "u")); }
            }
        }

        // Custom mappings (one pass over snapshot)
        let custom_list: Vec<String> = new_variants.iter().cloned().collect();
        for v in custom_list {
            for (from, to) in &fuzzy.custom_mappings {
                if v.contains(from) {
                    new_variants.insert(v.replace(from, to));
                }
            }
        }

        let mut result: Vec<String> = new_variants.into_iter().collect();
        result.sort();
        result
    }

    /// 只用 base_syllables 做最长贪心匹配（第一遍，不做 DP 合并）
    fn segment_base(&self, input: &str, base_syllables: &std::collections::HashSet<String>) -> Vec<String> {
        let mut segs = Vec::new();
        let mut pos = 0;
        while pos < input.len() {
            let max_len = 12.min(input.len() - pos);
            let mut matched = false;
            for len in (1..=max_len).rev() {
                let end = pos + len;
                if input.is_char_boundary(end) {
                    let part = &input[pos..end];
                    if base_syllables.contains(part) {
                        segs.push(part.to_string());
                        pos = end;
                        matched = true;
                        break;
                    }
                }
            }
            if !matched {
                break;
            }
        }
        segs
    }

    /// 回溯生成所有合法分割（每段 1~4 个 base 音节，且 pinyin 必须在 trie 有词）
    fn backtrack_partitions(
        &self,
        base: &[String],
        pos: usize,
        current: &mut Vec<(usize, usize)>,
        result: &mut Vec<Vec<(usize, usize)>>,
        trie: &crate::trie::Trie,
    ) {
        if pos >= base.len() {
            result.push(current.clone());
            return;
        }
        let max_k = 4.min(base.len() - pos);
        for k in 1..=max_k {
            let end = pos + k;
            if k == 1 {
                current.push((pos, end));
                self.backtrack_partitions(base, end, current, result, trie);
                current.pop();
            } else {
                let merged: String = base[pos..end].concat();
                if trie.get_all_exact(&merged).is_some() {
                    current.push((pos, end));
                    self.backtrack_partitions(base, end, current, result, trie);
                    current.pop();
                }
            }
        }
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
        let segments = crate::pipeline::DefaultSegmentor::viterbi_segment(&normalized, context.syllable_freq, context.base_syllables);
        if segments.iter().all(|s: &String| s.len() == 1) {
            return vec![];
        }
        segments
    }
}

impl InputScheme for ChineseScheme {
    fn lookup(&self, query: &str, context: &SchemeContext) -> Vec<SchemeCandidate> {
        let raw_parsed = self.parse_buffer(query);
        let mut final_results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let min_results_needed = 500;
        let max_results = 500;

        // 用户词典检索
        if let Some(profile) = context.active_profiles.first() {
            let pinyin_key: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            let dict = context.user_dict.load();
            if let Some(profile_dict) = dict.get(profile) {
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
                }
            }
        }

        // 策略 1: 全量/简拼/前缀匹配
        let mut smart_segments = Vec::new();
        let delimiters = &context.config.input.segmentation_delimiters;
        if !query.contains(' ') {
            let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            smart_segments = self.segment_buffer(&pinyin_only, delimiters, context);
            log::info!("lookup: query={}, segments={:?}", query, smart_segments);
        }
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
                            exact.sort_by(|a, b| b.weight.cmp(&a.weight));
                            for tr in exact.iter().take(min_results_needed) {
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
                                if let Ok(lev) = Levenshtein::new(py, 1u32) {
                                    let mut corr_keys: Vec<(String, u32)> = Vec::new();
                                    let mut stream2 = d.index.search(lev).into_stream();
                                    while let Some((key_bytes, _)) = stream2.next() {
                                        if let Ok(key) = std::str::from_utf8(key_bytes) {
                                            if key != py {
                                                let dist = lev_distance(py, key);
                                                corr_keys.push((key.to_string(), dist));
                                                if corr_keys.len() >= 20 { break; }
                                            }
                                        }
                                    }
                                    corr_keys.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.len().cmp(&b.0.len())));
                                    comp_keys = corr_keys.into_iter().map(|(k, _)| k).collect();
                                }
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

        log::info!("lookup: after s1, len={}, top3=[{}, {}, {}]",
            final_results.len(),
            final_results.get(0).map(|c| c.text.as_str()).unwrap_or(""),
            final_results.get(1).map(|c| c.text.as_str()).unwrap_or(""),
            final_results.get(2).map(|c| c.text.as_str()).unwrap_or(""));

        // 策略 2: 简拼检索
        if final_results.len() < min_results_needed
            && context.config.input.enable_abbreviation_matching
            && !smart_segments.is_empty()
            && smart_segments.len() > 1
        {
            if let Some(d) = context.tries.get("chinese") {
                let mut abbr_results = d.search_abbreviation(&smart_segments, context.syllables, 200);
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

        // 策略 3: 长句组合（ComposeTranslator）
        if !query.contains(' ') && !raw_parsed.is_empty() {
            let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            if let Some(d) = context.tries.get("chinese") {
                let base = self.segment_base(&pinyin_only, context.base_syllables);
                if base.len() >= 2 && base.len() <= 12 {
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
                        if let Some(&(pos, count)) = usage_map.get(c.simplified.as_str()) {
                            c.weight += (crate::pipeline::compute_decay_boost(pos, count) as u32).max(1);
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
