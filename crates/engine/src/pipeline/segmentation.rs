use std::collections::{HashMap, HashSet};

use crate::FuzzyPinyinConfig;

/// 切分器：将输入字符串切分为音节序列
pub trait Segmentor: Send + Sync {
    fn segment(
        &self,
        input: &str,
        syllables: &HashSet<String>,
        delimiters: &str,
        syllable_freq: &HashMap<String, u64>,
        base_syllables: &HashSet<String>,
    ) -> Vec<String>;
}

/// 默认切分器实现 (Viterbi DP)
pub struct DefaultSegmentor;
impl Segmentor for DefaultSegmentor {
    fn segment(
        &self,
        input: &str,
        syllables: &HashSet<String>,
        delimiters: &str,
        syllable_freq: &HashMap<String, u64>,
        base_syllables: &HashSet<String>,
    ) -> Vec<String> {
        let needs_lowercase = !input
            .bytes()
            .all(|b: u8| b.is_ascii_lowercase() || b.is_ascii_digit());

        if needs_lowercase {
            let input_lower = input.to_lowercase();
            return Self::segment_lowercase(
                &input_lower,
                syllables,
                delimiters,
                syllable_freq,
                base_syllables,
            );
        }

        Self::segment_lowercase(input, syllables, delimiters, syllable_freq, base_syllables)
    }
}

impl DefaultSegmentor {
    /// 尝试相邻字母换位纠错（处理 guna→guan、guagn→guang 等常见 finger slip）
    fn try_transpose(
        part: &str,
        syllable_freq: &HashMap<String, u64>,
        base_syllables: &HashSet<String>,
    ) -> Option<(u64, String)> {
        let bytes = part.as_bytes();
        for i in 0..bytes.len().saturating_sub(1) {
            let mut swapped = bytes.to_vec();
            swapped.swap(i, i + 1);
            if let Ok(candidate) = String::from_utf8(swapped) {
                if let Some(&freq) = syllable_freq.get(&candidate) {
                    return Some((freq / 2, candidate));
                }
                if base_syllables.contains(&candidate) {
                    return Some((0, candidate));
                }
            }
        }
        None
    }

    /// 单轮 Viterbi DP 切分：直接在原始字符串上做最优切分，避免两轮法中的贪心锁定问题
    pub(crate) fn viterbi_segment(
        input: &str,
        syllable_freq: &HashMap<String, u64>,
        base_syllables: &HashSet<String>,
    ) -> Vec<String> {
        let n = input.len();
        if n == 0 {
            return vec![];
        }

        let mut dp: Vec<Option<(u64, usize, usize)>> = vec![None; n + 1];
        dp[0] = Some((0, 0, 0));
        let mut corrected: Vec<String> = vec![String::new(); n + 1];

        for i in 0..n {
            let Some((cur_freq, cur_seg, _)) = dp[i] else {
                continue;
            };
            let max_len = 12.min(n - i);

            for len in 1..=max_len {
                if !input.is_char_boundary(i + len) {
                    continue;
                }
                let part = &input[i..i + len];

                let (freq, seg_text) = if syllable_freq.contains_key(part) {
                    (syllable_freq.get(part).copied().unwrap_or(0), None)
                } else if base_syllables.contains(part) || len == 1 {
                    (0, None)
                } else if let Some((xfreq, xtext)) =
                    Self::try_transpose(part, syllable_freq, base_syllables)
                {
                    (xfreq, Some(xtext))
                } else {
                    continue;
                };

                let total = cur_freq + freq;
                let seg_cnt = cur_seg + 1;
                let entry = &mut dp[i + len];

                let should_replace = match entry {
                    None => true,
                    Some((best_freq, best_seg, _)) => {
                        total > *best_freq || (total == *best_freq && seg_cnt < *best_seg)
                    }
                };

                if should_replace {
                    *entry = Some((total, seg_cnt, i));
                    if let Some(text) = seg_text {
                        corrected[i + len] = text;
                    }
                }
            }
        }

        let mut segments: Vec<String> = Vec::new();
        let mut pos = n;
        while pos > 0 {
            match dp[pos] {
                Some((_, _, prev)) if prev < pos => {
                    if !corrected[pos].is_empty() {
                        segments.push(corrected[pos].clone());
                    } else {
                        segments.push(input[prev..pos].to_string());
                    }
                    pos = prev;
                }
                _ => {
                    let prev = input[..pos]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(pos.saturating_sub(1));
                    segments.push(input[prev..pos].to_string());
                    pos = prev;
                }
            }
        }
        segments.reverse();
        segments
    }

    #[inline]
    fn segment_lowercase(
        input: &str,
        _syllables: &HashSet<String>,
        delimiters: &str,
        syllable_freq: &HashMap<String, u64>,
        base_syllables: &HashSet<String>,
    ) -> Vec<String> {
        if input.is_empty() {
            return vec![];
        }

        let mut result = Vec::new();
        for chunk in input.split(|c: char| delimiters.contains(c)) {
            if chunk.is_empty() {
                continue;
            }
            result.extend(Self::viterbi_segment(chunk, syllable_freq, base_syllables));
        }
        result
    }
}

/// 对每个音节段生成模糊音变体（单轮快照扫描，不链式迭代）
pub(crate) fn fuzzy_variants_per_segment(
    seg: &str,
    fuzzy: &FuzzyPinyinConfig,
) -> Vec<String> {
    let pinyin_lower = if seg.bytes().all(|b| b.is_ascii_lowercase()) {
        seg.to_string()
    } else {
        seg.to_lowercase()
    };
    let mut new_variants = std::collections::HashSet::new();
    new_variants.insert(pinyin_lower);

    // 声母替换（单轮快照）
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

    // 韵母替换（单轮快照）
    let final_list: Vec<String> = new_variants.iter().cloned().collect();
    for v in final_list {
        if fuzzy.an_ang {
            if v.ends_with("ang") {
                let mut s = String::with_capacity(v.len());
                s.push_str(&v[..v.len() - 3]);
                s.push_str("an");
                new_variants.insert(s);
            } else if v.ends_with("an") {
                let mut s = String::with_capacity(v.len() + 1);
                s.push_str(&v[..v.len() - 2]);
                s.push_str("ang");
                new_variants.insert(s);
            }
        }
        if fuzzy.en_eng {
            if v.ends_with("eng") {
                let mut s = String::with_capacity(v.len());
                s.push_str(&v[..v.len() - 3]);
                s.push_str("en");
                new_variants.insert(s);
            } else if v.ends_with("en") {
                let mut s = String::with_capacity(v.len() + 1);
                s.push_str(&v[..v.len() - 2]);
                s.push_str("eng");
                new_variants.insert(s);
            }
        }
        if fuzzy.in_ing {
            if v.ends_with("ing") {
                let mut s = String::with_capacity(v.len());
                s.push_str(&v[..v.len() - 3]);
                s.push_str("in");
                new_variants.insert(s);
            } else if v.ends_with("in") {
                let mut s = String::with_capacity(v.len() + 1);
                s.push_str(&v[..v.len() - 2]);
                s.push_str("ing");
                new_variants.insert(s);
            }
        }
        if fuzzy.ian_iang {
            if v.ends_with("iang") {
                let mut s = String::with_capacity(v.len());
                s.push_str(&v[..v.len() - 4]);
                s.push_str("ian");
                new_variants.insert(s);
            } else if v.ends_with("ian") {
                let mut s = String::with_capacity(v.len() + 1);
                s.push_str(&v[..v.len() - 3]);
                s.push_str("iang");
                new_variants.insert(s);
            }
        }
        if fuzzy.uan_uang {
            if v.ends_with("uang") {
                let mut s = String::with_capacity(v.len());
                s.push_str(&v[..v.len() - 4]);
                s.push_str("uan");
                new_variants.insert(s);
            } else if v.ends_with("uan") {
                let mut s = String::with_capacity(v.len() + 1);
                s.push_str(&v[..v.len() - 3]);
                s.push_str("uang");
                new_variants.insert(s);
            }
        }
        if fuzzy.u_v {
            let replaced = v.replace('u', "v");
            if replaced != v {
                new_variants.insert(replaced);
            } else {
                let replaced = v.replace('v', "u");
                if replaced != v {
                    new_variants.insert(replaced);
                }
            }
        }
    }

    // 自定义映射（单轮快照，避免 contains+replace 重复扫描）
    let custom_list: Vec<String> = new_variants.iter().cloned().collect();
    for v in custom_list {
        for (from, to) in &fuzzy.custom_mappings {
            let replaced = v.replace(from, to);
            if replaced != v {
                new_variants.insert(replaced);
            }
        }
    }

    let mut result: Vec<String> = new_variants.into_iter().collect();
    result.sort();
    result
}
