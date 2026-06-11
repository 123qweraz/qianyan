use std::collections::{HashMap, HashSet};

use crate::trie::Trie;
use crate::pipeline::segmentation::DefaultSegmentor;

/// 合法拼音声母（含双字符 zh/ch/sh）
pub const TWO_INITIALS: &[&str] = &["zh", "ch", "sh"];
pub const SINGLE_INITIALS: &[char] = &[
    'b', 'p', 'm', 'f', 'd', 't', 'n', 'l', 'g', 'k', 'h',
    'j', 'q', 'x', 'r', 'z', 'c', 's', 'y', 'w',
];

/// 判断字符串是否为合法拼音声母
pub fn is_initial(s: &str) -> bool {
    match s.len() {
        2 => TWO_INITIALS.contains(&s),
        1 => s.chars().next().is_some_and(|c| SINGLE_INITIALS.contains(&c)),
        _ => false,
    }
}

/// 两趟法切分：Pass1 切单音节 → Pass2 合并多音节
pub fn segment_base(
    input: &str,
    syllable_freq: &HashMap<String, u64>,
    base_syllables: &HashSet<String>,
) -> Vec<String> {
    DefaultSegmentor::viterbi_segment(input, syllable_freq, base_syllables)
}

/// 左到右贪心分段：先匹配最长全音节，否则匹配声母
/// 返回 (segment, is_initial) 对
pub fn segment_for_abbreviation(input: &str, trie: &Trie) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    let n = input.len();
    let mut pos = 0;

    while pos < n {
        let max_len = (n - pos).min(6);
        let mut matched = false;

        // 1. 尝试匹配完整拼音音节（2~6字符）
        for len in (2..=max_len).rev() {
            if input.is_char_boundary(pos + len) {
                let candidate = &input[pos..pos + len];
                if trie.index.contains_key(candidate) {
                    result.push((candidate.to_string(), false));
                    pos += len;
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }

        // 2. 尝试双字符声母 zh ch sh
        if n - pos >= 2 {
            let candidate = &input[pos..pos + 2];
            if TWO_INITIALS.contains(&candidate) {
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
        if SINGLE_INITIALS.contains(&ch) {
            result.push((ch.to_string(), true));
            pos += ch.len_utf8();
        } else {
            break;
        }
    }

    result
}

/// 简拼切分：只用 base_syllables 检测音节，不含 trie 复合词
pub fn segment_by_syllables(input: &str, base_syllables: &HashSet<String>) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    let n = input.len();
    let mut pos = 0;

    while pos < n {
        let max_len = 6.min(n - pos);
        let mut matched = false;

        for len in (2..=max_len).rev() {
            if input.is_char_boundary(pos + len) && base_syllables.contains(&input[pos..pos + len]) {
                result.push((input[pos..pos + len].to_string(), false));
                pos += len;
                matched = true;
                break;
            }
        }
        if matched { continue; }

        if n - pos >= 2 && is_initial(&input[pos..pos + 2]) {
            result.push((input[pos..pos + 2].to_string(), true));
            pos += 2;
            continue;
        }

        if let Some(ch) = input[pos..].chars().next() {
            let s = ch.to_string();
            if is_initial(&s) {
                result.push((s, true));
                pos += ch.len_utf8();
                continue;
            }
        }

        break;
    }

    result
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
