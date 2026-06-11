use std::collections::{HashMap, HashSet};

use crate::trie::Trie;
use fst::{IntoStreamer, Streamer};
use qianyan_ime_core::config::FuzzyPinyinConfig;

/// 拼音相似候选，带 cost（越小越相似）
#[derive(Debug, Clone)]
pub struct PinyinMatch {
    pub pinyin: String,
    pub cost: f64,
}

/// 统一引擎：给定输入拼音，返回所有有效相似拼音（已过滤 trie 存在性），按 cost 升序。
pub fn find_similar_pinyin(
    input: &str,
    trie: &Trie,
    fuzzy: Option<&FuzzyPinyinConfig>,
    syllable_freq: &HashMap<String, u64>,
    base_syllables: &HashSet<String>,
) -> Vec<PinyinMatch> {
    // 长输入只做 L0+L1（精确+模糊），跳过昂贵的 L2+L3
    let long_input = input.len() > 12;
    let mut seen = std::collections::HashSet::new();
    let mut matches: Vec<PinyinMatch> = Vec::new();

    // L0: exact match — cost 0.0
    try_add(input.to_string(), 0.0, trie, &mut seen, &mut matches);

    // L1: segment-based fuzzy variants — cost 0.2 per fuzzy segment
    l1_fuzzy_variants(input, trie, fuzzy, syllable_freq, base_syllables, &mut seen, &mut matches);

    // L2: adjacent transposition — cost 0.5 per swap
    if !long_input {
        l2_transpositions(input, trie, &mut seen, &mut matches);
    }

    // L3: Levenshtein distance 1 — cost 1.0
    if !long_input {
        l3_levenshtein(input, trie, &mut seen, &mut matches);
    }

    // Sort by cost ascending, then by length (shorter = better)
    matches.sort_by(|a, b| {
        a.cost.partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.pinyin.len().cmp(&b.pinyin.len()))
    });

    matches
}

fn try_add(
    py: String,
    cost: f64,
    trie: &Trie,
    seen: &mut std::collections::HashSet<String>,
    matches: &mut Vec<PinyinMatch>,
) {
    if seen.insert(py.clone()) && trie.get_all_exact(&py).is_some() {
        matches.push(PinyinMatch { pinyin: py, cost });
    }
}

fn l1_fuzzy_variants(
    input: &str,
    trie: &Trie,
    fuzzy: Option<&FuzzyPinyinConfig>,
    syllable_freq: &HashMap<String, u64>,
    base_syllables: &HashSet<String>,
    seen: &mut std::collections::HashSet<String>,
    matches: &mut Vec<PinyinMatch>,
) {
    let fz = match fuzzy {
        Some(f) => f,
        None => return,
    };
    let segs = crate::pipeline::compose::segment_syllables(input, syllable_freq, base_syllables);
    if segs.is_empty() || (segs.len() == 1 && segs[0].len() >= input.len()) {
        return;
    }
    let variant_sets: Vec<Vec<String>> = segs.iter()
        .map(|seg| {
            let mut vars = crate::pipeline::segmentation::fuzzy_variants_per_segment(seg, fz);
            if vars.is_empty() { vars = vec![seg.to_string()]; }
            vars
        })
        .collect();

    let total_combos: usize = variant_sets.iter().map(|v| v.len()).product();
    if total_combos == 0 || total_combos > 256 {
        return;
    }

    let mut idxs = vec![0usize; variant_sets.len()];
    loop {
        let py: String = idxs.iter().enumerate()
            .map(|(i, &idx)| variant_sets[i][idx].as_str())
            .collect::<Vec<&str>>().concat();
        let fuzzy_count: usize = idxs.iter().enumerate()
            .filter(|(i, &idx)| variant_sets[*i][idx] != segs[*i])
            .count();
        if fuzzy_count > 0 {
            try_add(py, 0.2 * fuzzy_count as f64, trie, seen, matches);
        }
        let mut carry = true;
        for i in (0..idxs.len()).rev() {
            if carry {
                idxs[i] += 1;
                if idxs[i] >= variant_sets[i].len() {
                    idxs[i] = 0;
                } else {
                    carry = false;
                }
            }
        }
        if carry { break; }
    }
}

fn l2_transpositions(
    input: &str,
    trie: &Trie,
    seen: &mut std::collections::HashSet<String>,
    matches: &mut Vec<PinyinMatch>,
) {
    let chars: Vec<char> = input.chars().collect();
    if chars.len() < 2 { return; }
    for i in 0..chars.len() - 1 {
        let mut swapped = chars.clone();
        swapped.swap(i, i + 1);
        let candidate: String = swapped.into_iter().collect();
        if candidate != input {
            try_add(candidate, 0.5, trie, seen, matches);
        }
    }
}

fn l3_levenshtein(
    input: &str,
    trie: &Trie,
    seen: &mut std::collections::HashSet<String>,
    matches: &mut Vec<PinyinMatch>,
) {
    if let Ok(lev) = fst::automaton::Levenshtein::new(input, 1u32) {
        let mut stream = trie.index.search(lev).into_stream();
        while let Some((key_bytes, _)) = stream.next() {
            if let Ok(key) = std::str::from_utf8(key_bytes) {
                try_add(key.to_string(), 1.0, trie, seen, matches);
                if matches.len() >= 50 { break; }
            }
        }
    }
}
