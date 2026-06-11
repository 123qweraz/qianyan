use std::collections::{HashMap, HashSet};

use crate::FuzzyPinyinConfig;

/// 切分器：将输入字符串切分为音节序列
pub trait Segmentor: Send + Sync {
    fn segment(
        &self,
        input: &str,
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
                delimiters,
                syllable_freq,
                base_syllables,
            );
        }

        Self::segment_lowercase(input, delimiters, syllable_freq, base_syllables)
    }
}

impl DefaultSegmentor {
    /// Pass1: 只用 base_syllables 贪心最长匹配，切成纯单音节
    fn segment_single(input: &str, base_syllables: &HashSet<String>) -> Vec<String> {
        let mut segs = Vec::new();
        let mut pos = 0;
        while pos < input.len() {
            let max_len = 6.min(input.len() - pos);
            let mut matched = false;
            for len in (1..=max_len).rev() {
                let end = pos + len;
                if input.is_char_boundary(end) && base_syllables.contains(&input[pos..end]) {
                    segs.push(input[pos..end].to_string());
                    pos = end;
                    matched = true;
                    break;
                }
            }
            if !matched {
                break;
            }
        }
        segs
    }

    /// Pass2: DP 合并相邻单音节，用 syllable_freq 最大化总频次
    pub(crate) fn merge_by_freq(segs: &[String], syll_freq: &HashMap<String, u64>) -> Vec<String> {
        let n = segs.len();
        if n == 0 { return vec![]; }
        let mut dp = vec![(0u64, 0usize); n + 1];

        for i in 0..n {
            let cur_score = dp[i].0;
            // 不合并：保留单音节
            if cur_score >= dp[i + 1].0 {
                dp[i + 1] = (cur_score, i);
            }
            // 合并 k 个相邻音节
            for k in 2..=4.min(n - i) {
                let py: String = segs[i..i + k].concat();
                if let Some(&freq) = syll_freq.get(&py) {
                    let score = cur_score + freq;
                    if score > dp[i + k].0 {
                        dp[i + k] = (score, i);
                    }
                }
            }
        }

        let mut result = Vec::new();
        let mut pos = n;
        while pos > 0 {
            let prev = dp[pos].1;
            let py: String = segs[prev..pos].concat();
            result.push(py);
            pos = prev;
        }
        result.reverse();
        result
    }

    /// 两趟法切分：Pass1 切单音节 → Pass2 合并多音节
    pub(crate) fn viterbi_segment(
        input: &str,
        syllable_freq: &HashMap<String, u64>,
        base_syllables: &HashSet<String>,
    ) -> Vec<String> {
        let singles = Self::segment_single(input, base_syllables);
        if singles.is_empty() {
            return vec![];
        }
        Self::merge_by_freq(&singles, syllable_freq)
    }

    #[inline]
    fn segment_lowercase(
        input: &str,
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
