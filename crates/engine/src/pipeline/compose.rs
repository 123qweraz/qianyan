use std::collections::{HashMap, HashSet};

use crate::trie::Trie;
use crate::pipeline::segmentation::DefaultSegmentor;

#[derive(Debug, Clone)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub pinyin: String,
}

#[derive(Debug, Clone)]
pub struct ComposePath {
    pub words: Vec<WordSpan>,
    pub score: f64,
}

/// 用 Viterbi DP 分割拼音串为音节序列
pub fn segment_syllables(
    pinyin: &str,
    syllable_freq: &HashMap<String, u64>,
    base_syllables: &HashSet<String>,
) -> Vec<String> {
    if pinyin.is_empty() {
        return vec![];
    }
    DefaultSegmentor::viterbi_segment(pinyin, syllable_freq, base_syllables)
}

pub fn compose(
    pinyin: &str,
    trie: &Trie,
    ngram: &crate::config_manager::UserDictData,
    syllable_freq: &std::collections::HashMap<String, u64>,
    base_syllables: &std::collections::HashSet<String>,
    profile: &str,
) -> Vec<ComposePath> {
    let segs = segment_syllables(pinyin, syllable_freq, base_syllables);
    if segs.len() < 2 { return Vec::new(); }
    let graph = build_word_graph(&segs, trie);
    viterbi_compose(&graph, segs.len(), syllable_freq, ngram, profile)
}

fn build_word_graph(segments: &[String], trie: &Trie) -> Vec<WordSpan> {
    let n = segments.len();
    let mut graph = Vec::new();
    for start in 0..n {
        let max_k = 4.min(n - start);
        for k in 1..=max_k {
            let end = start + k;
            let py: String = segments[start..end].concat();
            if let Some(entries) = trie.get_all_exact(&py) {
                // 每个拼音段保留 top-3 词，增加 Viterbi 路径多样性
                let mut candidates: Vec<&crate::trie::TrieResult> = entries.iter().collect();
                candidates.sort_by_key(|r| std::cmp::Reverse(r.weight));
                for tr in candidates.iter().take(3) {
                    graph.push(WordSpan {
                        start,
                        end,
                        word: tr.word.to_string(),
                        pinyin: py.clone(),
                    });
                }
            }
        }
    }
    graph
}

fn viterbi_compose(
    graph: &[WordSpan],
    n_segments: usize,
    syllable_freq: &std::collections::HashMap<String, u64>,
    ngram: &crate::config_manager::UserDictData,
    profile: &str,
) -> Vec<ComposePath> {
    let n = n_segments + 1;
    let mut dp: Vec<Vec<ComposePath>> = vec![Vec::new(); n];
    dp[0].push(ComposePath { words: Vec::new(), score: 0.0 });

    for i in 0..n_segments {
        if dp[i].is_empty() { continue; }
        let dp_i = dp[i].clone();
        for span in graph.iter().filter(|s| s.start == i && s.end < n) {
            for prev in &dp_i {
                let mut words = prev.words.clone();
                words.push(span.clone());

                let freq = *syllable_freq.get(&span.pinyin).unwrap_or(&0) as f64;
                let freq_score = if freq > 0.0 { freq.log2() * 100.0 } else { 0.0 };

                let word_len_bonus = (span.word.chars().count().saturating_sub(1)) as f64 * 200.0;

                let ngram_bonus = if let (Some(last), Some(pn)) = (
                    prev.words.last().map(|w| w.word.as_str()),
                    ngram.get(profile),
                ) {
                    pn.get(last)
                        .and_then(|e| e.iter().find(|(w, _)| w == &span.word))
                        .map(|(_, c)| (*c).min(10) as f64 * 500.0)
                        .unwrap_or(0.0)
                } else {
                    0.0
                };

                let score = prev.score + freq_score + ngram_bonus + word_len_bonus;

                dp[span.end].push(ComposePath { words, score });
            }
        }
        dp[i + 1].sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        dp[i + 1].truncate(32);
    }

    let mut results = dp[n_segments].clone();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(8);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_syllable_freq(root: &std::path::Path) -> HashMap<String, u64> {
        let path = root.join("dicts/chinese/syllable_freq.txt");
        let content = std::fs::read_to_string(path).unwrap();
        content.lines().filter_map(|l| {
            let mut parts = l.trim().split_whitespace();
            let key = parts.next()?.to_string();
            let val: u64 = parts.next()?.parse().ok()?;
            Some((key, val))
        }).collect()
    }

    fn load_base_syllables(root: &std::path::Path) -> HashSet<String> {
        let paths = [
            root.join("dicts/chinese/single_syllables.txt"),
        ];
        paths.iter().find_map(|p| std::fs::read_to_string(p).ok())
            .map(|c| c.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn test_segment_syllables() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let freq = load_syllable_freq(&root);
        let base = load_base_syllables(&root);
        let segs = segment_syllables("wowangjichongdianle", &freq, &base);
        assert_eq!(segs, vec!["wo","wangji","chongdian","le"]);
    }

    #[test]
    fn test_compose_long_sentence() {
        use std::path::PathBuf;
        use arc_swap::ArcSwap;
        use std::sync::Arc;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let trie = crate::trie::Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");

        let freq = load_syllable_freq(&root);
        let base = load_base_syllables(&root);

        let ngram = Arc::new(ArcSwap::new(Arc::new(
            std::collections::HashMap::<String, std::collections::HashMap<String, Vec<(String, u32)>>>::new()
        )));
        let paths = compose("wowangjichongdianle", &trie, &ngram.load(), &freq, &base, "chinese");
        println!("Compose paths: {} found", paths.len());
        for p in &paths {
            let text: String = p.words.iter().map(|w| w.word.as_str()).collect();
            let pys: Vec<String> = p.words.iter().map(|w| w.pinyin.clone()).collect();
            println!("  path: {} (pinyin={:?}, score={:.3})", text, pys, p.score);
        }
        assert!(!paths.is_empty(), "Should find composed paths");

        let found = paths.iter().any(|p| {
            let text: String = p.words.iter().map(|w| w.word.as_str()).collect();
            text.contains("忘") && text.contains("充电")
        });
        assert!(found, "Should contain 忘记充电");
    }
}
