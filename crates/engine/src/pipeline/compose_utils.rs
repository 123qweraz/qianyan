use crate::trie::Trie;
use qianyan_ime_core::config::FuzzyPinyinConfig;
use std::collections::HashSet;

/// 只用 base_syllables 做最长贪心匹配（第一遍，不做 DP 合并）
pub fn segment_base(input: &str, base_syllables: &HashSet<String>) -> Vec<String> {
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
/// 支持模糊音：当 fuzzy 参数不为 None 时，也会检查模糊变体组合
pub fn backtrack_partitions(
    base: &[String],
    pos: usize,
    current: &mut Vec<(usize, usize)>,
    result: &mut Vec<Vec<(usize, usize)>>,
    trie: &Trie,
    fuzzy: Option<&FuzzyPinyinConfig>,
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
            backtrack_partitions(base, end, current, result, trie, fuzzy);
            current.pop();
        } else {
            let merged: String = base[pos..end].concat();
            // 先查原始拼音
            if trie.get_all_exact(&merged).is_some() {
                current.push((pos, end));
                backtrack_partitions(base, end, current, result, trie, fuzzy);
                current.pop();
            } else if let Some(fz) = fuzzy {
                // 原始不命中，查模糊变体组合
                let segs: Vec<&str> = base[pos..end].iter().map(|s| s.as_str()).collect();
                let variant_sets: Vec<Vec<String>> = segs.iter()
                    .map(|seg| {
                        let mut vars = crate::pipeline::segmentation::fuzzy_variants_per_segment(seg, fz);
                        if vars.is_empty() { vars = vec![seg.to_string()]; }
                        vars
                    })
                    .collect();
                let mut idxs = vec![0usize; variant_sets.len()];
                let mut found = false;
                loop {
                    let py: String = idxs.iter().enumerate()
                        .map(|(i, &idx)| variant_sets[i][idx].as_str())
                        .collect::<Vec<&str>>().concat();
                    if py != merged && trie.get_all_exact(&py).is_some() {
                        found = true;
                        break;
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
                if found {
                    current.push((pos, end));
                    backtrack_partitions(base, end, current, result, trie, fuzzy);
                    current.pop();
                }
            }
        }
    }
}
