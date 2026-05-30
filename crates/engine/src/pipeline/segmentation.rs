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
                    (*syllable_freq.get(part).unwrap(), None)
                } else if base_syllables.contains(part) {
                    (0, None)
                } else if len == 1 {
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

/// 对每个音节段生成模糊音变体（迭代式：新变体也继续应用规则，与 chinese.rs 保持一致）
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

    let mut to_process: Vec<String> = new_variants.iter().cloned().collect();
    while let Some(v) = to_process.pop() {
        if fuzzy.z_zh {
            if v.starts_with("zh") {
                let replaced = v.replacen("zh", "z", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with("z") {
                let replaced = v.replacen("z", "zh", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.c_ch {
            if v.starts_with("ch") {
                let replaced = v.replacen("ch", "c", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with("c") {
                let replaced = v.replacen("c", "ch", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.s_sh {
            if v.starts_with("sh") {
                let replaced = v.replacen("sh", "s", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with("s") {
                let replaced = v.replacen("s", "sh", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.n_l {
            if v.starts_with('n') {
                let replaced = v.replacen('n', "l", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with('l') {
                let replaced = v.replacen('l', "n", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.r_l {
            if v.starts_with('r') {
                let replaced = v.replacen('r', "l", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with('l') {
                let replaced = v.replacen('l', "r", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.f_h {
            if v.starts_with('f') {
                let replaced = v.replacen('f', "h", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with('h') {
                let replaced = v.replacen('h', "f", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.an_ang {
            if v.ends_with("ang") {
                let replaced = format!("{}an", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("an") {
                let replaced = format!("{}ang", &v[..v.len() - 2]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.en_eng {
            if v.ends_with("eng") {
                let replaced = format!("{}en", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("en") {
                let replaced = format!("{}eng", &v[..v.len() - 2]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.in_ing {
            if v.ends_with("ing") {
                let replaced = format!("{}in", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("in") {
                let replaced = format!("{}ing", &v[..v.len() - 2]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.ian_iang {
            if v.ends_with("iang") {
                let replaced = format!("{}ian", &v[..v.len() - 4]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("ian") {
                let replaced = format!("{}iang", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.uan_uang {
            if v.ends_with("uang") {
                let replaced = format!("{}uan", &v[..v.len() - 4]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("uan") {
                let replaced = format!("{}uang", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.u_v {
            if v.contains('u') {
                let replaced = v.replace('u', "v");
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.contains('v') {
                let replaced = v.replace('v', "u");
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        for (from, to) in &fuzzy.custom_mappings {
            if v.contains(from) {
                let replaced = v.replace(from, to);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
    }

    let mut result: Vec<String> = new_variants.into_iter().collect();
    result.sort();
    result
}
