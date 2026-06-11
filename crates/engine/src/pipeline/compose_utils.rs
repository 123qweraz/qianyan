use std::collections::{HashMap, HashSet};

use crate::trie::Trie;
use crate::pipeline::segmentation::DefaultSegmentor;

/// 用 Viterbi DP 分割拼音串为音节序列
pub fn segment_base(
    input: &str,
    syllable_freq: &HashMap<String, u64>,
    base_syllables: &HashSet<String>,
) -> Vec<String> {
    if input.is_empty() {
        return vec![];
    }
    DefaultSegmentor::viterbi_segment(input, syllable_freq, base_syllables)
}

/// 回溯生成所有合法分割（每段 1~4 个 base 音节，且 pinyin 必须在 trie 有词）
pub fn backtrack_partitions(
    base: &[String],
    pos: usize,
    current: &mut Vec<(usize, usize)>,
    result: &mut Vec<Vec<(usize, usize)>>,
    trie: &Trie,
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
            backtrack_partitions(base, end, current, result, trie);
            current.pop();
        } else {
            let merged: String = base[pos..end].concat();
            if trie.get_all_exact(&merged).is_some() {
                current.push((pos, end));
                backtrack_partitions(base, end, current, result, trie);
                current.pop();
            }
        }
    }
}
