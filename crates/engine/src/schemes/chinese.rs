use crate::scheme::{InputScheme, SchemeCandidate, SchemeContext};
use std::collections::{HashMap, HashSet};

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
        let buffer_normalized = if buffer.bytes().all(|b| b.is_ascii_lowercase() || b == b' ') {
            buffer
        } else {
            return Self::parse_buffer(self, &buffer.to_lowercase());
        };
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

        let pinyin_lower = if pinyin.bytes().all(|b| b.is_ascii_lowercase()) {
            pinyin.to_string()
        } else {
            pinyin.to_lowercase()
        };
        let mut new_variants = std::collections::HashSet::new();
        new_variants.insert(pinyin_lower);

        let cfg = &context.config.input.fuzzy_config;

        let mut to_process: Vec<String> = new_variants.iter().cloned().collect();
        while let Some(v) = to_process.pop() {
            if cfg.z_zh {
                if v.starts_with("zh") {
                    new_variants.insert(v.replace("zh", "z"));
                } else if v.starts_with("z") {
                    new_variants.insert(v.replace("z", "zh"));
                }
            }
            if cfg.c_ch {
                if v.starts_with("ch") {
                    new_variants.insert(v.replace("ch", "c"));
                } else if v.starts_with("c") {
                    new_variants.insert(v.replace("c", "ch"));
                }
            }
            if cfg.s_sh {
                if v.starts_with("sh") {
                    new_variants.insert(v.replace("sh", "s"));
                } else if v.starts_with("s") {
                    new_variants.insert(v.replace("s", "sh"));
                }
            }
            if cfg.n_l {
                if v.starts_with('n') {
                    new_variants.insert(v.replace('n', "l"));
                } else if v.starts_with('l') {
                    new_variants.insert(v.replace('l', "n"));
                }
            }
            if cfg.r_l {
                if v.starts_with('r') {
                    new_variants.insert(v.replace('r', "l"));
                } else if v.starts_with('l') {
                    new_variants.insert(v.replace('l', "r"));
                }
            }
            if cfg.f_h {
                if v.starts_with('f') {
                    new_variants.insert(v.replace('f', "h"));
                } else if v.starts_with('h') {
                    new_variants.insert(v.replace('h', "f"));
                }
            }
            if cfg.an_ang {
                if v.ends_with("ang") {
                    new_variants.insert(v.replace("ang", "an"));
                } else if v.ends_with("an") {
                    new_variants.insert(v.replace("an", "ang"));
                }
            }
            if cfg.en_eng {
                if v.ends_with("eng") {
                    new_variants.insert(v.replace("eng", "en"));
                } else if v.ends_with("en") {
                    new_variants.insert(v.replace("en", "eng"));
                }
            }
            if cfg.in_ing {
                if v.ends_with("ing") {
                    new_variants.insert(v.replace("ing", "in"));
                } else if v.ends_with("in") {
                    new_variants.insert(v.replace("in", "ing"));
                }
            }
            if cfg.ian_iang {
                if v.ends_with("iang") {
                    new_variants.insert(v.replace("iang", "ian"));
                } else if v.ends_with("ian") {
                    new_variants.insert(v.replace("ian", "iang"));
                }
            }
            if cfg.uan_uang {
                if v.ends_with("uang") {
                    new_variants.insert(v.replace("uang", "uan"));
                } else if v.ends_with("uan") {
                    new_variants.insert(v.replace("uan", "uang"));
                }
            }
            if cfg.u_v {
                if v.contains('u') {
                    new_variants.insert(v.replace('u', "v"));
                } else if v.contains('v') {
                    new_variants.insert(v.replace('v', "u"));
                }
            }
            for (from, to) in &cfg.custom_mappings {
                if v.contains(from) {
                    new_variants.insert(v.replace(from, to));
                }
            }
        }

        new_variants.into_iter().collect()
    }

    fn segment_buffer(&self, input: &str, delimiters: &str, context: &SchemeContext) -> Vec<String> {
        if !input
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return Self::segment_buffer(self, &input.to_lowercase(), delimiters, context);
        }
        // 第一遍：用基本音节（不在频率表中的=单音节）作贪心最长匹配
        let base_segments = Self::first_pass_segment(input, delimiters, context.base_syllables);
        // 第二遍：按频率表合并相邻音节
        Self::merge_segments(&base_segments, context.syllables, context.syllable_freq)
    }

    fn first_pass_segment(input: &str, delimiters: &str, base_syllables: &HashSet<String>) -> Vec<String> {
        let bytes = input.as_bytes();
        let mut segments = Vec::new();
        let mut pos = 0;

        while pos < bytes.len() {
            let remaining = &bytes[pos..];
            let max_len = 6.min(remaining.len());
            let mut matched = false;

            for len in (1..=max_len).rev() {
                let part = std::str::from_utf8(&remaining[..len]).unwrap_or("");
                if base_syllables.contains(part) {
                    segments.push(part.to_string());
                    pos += len;
                    matched = true;
                    break;
                }
            }

            if matched {
                continue;
            }

            let c = remaining[0] as char;
            if delimiters.contains(c) {
                pos += 1;
                continue;
            }
            let is_initial = "bpmfdtnlgkhjqxzcsryw".contains(c);
            if is_initial {
                let initial_len = if remaining.starts_with(b"zh")
                    || remaining.starts_with(b"ch")
                    || remaining.starts_with(b"sh")
                {
                    2
                } else {
                    1
                };
                let part = std::str::from_utf8(&remaining[..initial_len]).unwrap_or("");
                segments.push(part.to_string());
                pos += initial_len;
            } else {
                let part = std::str::from_utf8(&remaining[..1]).unwrap_or("");
                segments.push(part.to_string());
                pos += 1;
            }
        }
        segments
    }

    fn merge_segments(segments: &[String], full_syllables: &HashSet<String>, syllable_freq: &HashMap<String, u64>) -> Vec<String> {
        let n = segments.len();
        if n <= 1 {
            return segments.to_vec();
        }

        let mut dp = vec![(0u64, n); n + 1];
        dp[n] = (0, n);

        let mut combined = vec![vec![String::new(); n]; n];
        for i in 0..n {
            combined[i][i] = segments[i].clone();
            for j in i + 1..n.min(i + 4) {
                combined[i][j] = combined[i][j - 1].clone() + &segments[j];
            }
        }

        for i in (0..n).rev() {
            let mut best_freq = 0u64;
            let mut best_end = i + 1;

            for k in 1..=4.min(n - i) {
                let end = i + k;
                let freq = if k == 1 {
                    syllable_freq.get(&segments[i]).copied().unwrap_or(0)
                } else {
                    let c = &combined[i][end - 1];
                    if full_syllables.contains(c) {
                        syllable_freq.get(c).copied().unwrap_or(0)
                    } else {
                        0
                    }
                };
                let total = freq + dp[end].0;
                if total > best_freq {
                    best_freq = total;
                    best_end = end;
                }
            }

            dp[i] = (best_freq, best_end);
        }

        let mut result = Vec::new();
        let mut i = 0;
        while i < n {
            let end = dp[i].1;
            if end == i + 1 {
                result.push(segments[i].clone());
            } else {
                result.push(combined[i][end - 1].clone());
            }
            i = end;
        }
        result
    }
}

impl InputScheme for ChineseScheme {
    fn lookup(&self, query: &str, context: &SchemeContext) -> Vec<SchemeCandidate> {
        let raw_parsed = self.parse_buffer(query);
        let mut final_results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let min_results_needed = 6;
        let max_results = 50;

        // 策略 1: 全量/简拼/前缀匹配
        let mut smart_segments = Vec::new();
        let delimiters = &context.config.input.segmentation_delimiters;
        if !query.contains(' ') {
            let pinyin_only: String = raw_parsed.iter().map(|p| p.pinyin.clone()).collect();
            smart_segments = self.segment_buffer(&pinyin_only, delimiters, context);
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
                            for tr in m {
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
                            if matches.len() >= min_results_needed {
                                break;
                            }
                        }
                        if context.config.input.enable_prefix_matching && !py.is_empty() {
                            let limit = if part.stroke_aux.is_some() || part.english_aux.is_some() {
                                50
                            } else if py.len() > 3 {
                                5
                            } else {
                                20
                            };
                            let m = d.search_bfs(py, limit);
                            for tr in m {
                                matches.push((
                                    tr.word.to_string(),
                                    tr.trad.to_string(),
                                    tr.tone.to_string(),
                                    tr.en.to_string(),
                                    tr.stroke_aux.to_string(),
                                    tr.weight,
                                    1,
                                ));
                            }
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

        // 策略 2: 简拼检索
        if final_results.len() < min_results_needed
            && context.config.input.enable_abbreviation_matching
            && !smart_segments.is_empty()
            && smart_segments.len() > 1
        {
            let first_seg_variants = self.get_fuzzy_variants(&smart_segments[0], context);
            for v1 in &first_seg_variants {
                let mut modified_segments = smart_segments.clone();
                modified_segments[0] = v1.clone();
                if let Some(d) = context.tries.get("chinese") {
                    let m = d.search_abbreviation(&modified_segments, context.syllables, 200);
                    for tr in m {
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

        candidates.sort_by(|a, b| {
            let get_score = |m: &SchemeCandidate| -> i64 {
                let level = m.match_level as i64;
                let weight = m.weight as i64;
                let char_count = m.text.chars().count() as i64;
                let mut score = if level == 3 {
                    40_000_000
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
    }
}
