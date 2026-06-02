use crate::trie::Trie;
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
