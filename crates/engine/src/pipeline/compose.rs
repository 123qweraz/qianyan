use std::collections::{HashMap, HashSet};

use crate::trie::Trie;
use crate::pipeline::segmentation::DefaultSegmentor;
use crate::pipeline::compose_utils;

#[derive(Debug, Clone)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub pinyin: String,
    pub initial_count: u8,
}

#[derive(Debug, Clone)]
pub struct ComposePath {
    pub words: Vec<WordSpan>,
    pub score: f64,
}

/// 用 Viterbi DP 分割拼音串为音节序列（给外部简单调用用）
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

/// 智能切分：Viterbi DP 切分后检测是否含简拼声母段，标记 is_initial
fn segment_for_compose(
    input: &str,
    trie: &Trie,
    syllable_freq: &HashMap<String, u64>,
    base_syllables: &HashSet<String>,
) -> Vec<(String, bool)> {
    let viterbi_segs = DefaultSegmentor::viterbi_segment(input, syllable_freq, base_syllables);

    if viterbi_segs.is_empty() {
        return compose_utils::segment_for_abbreviation(input, trie);
    }

    // Viterbi 切分结果上直接标记声母段
    viterbi_segs.into_iter().map(|s| {
        let is_init = s.len() <= 2
            && !base_syllables.contains(s.as_str())
            && compose_utils::is_initial(s.as_str());
        (s, is_init)
    }).collect()
}

pub fn compose(
    pinyin: &str,
    trie: &Trie,
    ngram: &crate::config_manager::UserDictData,
    syllable_freq: &std::collections::HashMap<String, u64>,
    base_syllables: &std::collections::HashSet<String>,
    profile: &str,
) -> Vec<ComposePath> {
    let segs = segment_for_compose(pinyin, trie, syllable_freq, base_syllables);
    if segs.len() < 2 {
        return Vec::new();
    }
    let graph = build_word_graph(&segs, trie, base_syllables);
    viterbi_compose(&graph, segs.len(), syllable_freq, ngram, profile)
}

fn build_word_graph(
    segments: &[(String, bool)],
    trie: &Trie,
    base_syllables: &HashSet<String>,
) -> Vec<WordSpan> {
    let n = segments.len();
    let mut graph = Vec::new();

    for start in 0..n {
        let max_k = 4.min(n - start);
        for k in 1..=max_k {
            let end = start + k;
            let sub = &segments[start..end];
            let has_initial = sub.iter().any(|(_, is_init)| *is_init);
            // 跳过单声母段：多声母合并段有 abbr_bonus 永远胜出，无需浪费 FST 扫描
            if k == 1 && has_initial {
                continue;
            }
            let py: String = sub.iter().map(|(s, _)| s.as_str()).collect();
            let initial_count = sub.iter().filter(|(_, is_init)| *is_init).count() as u8;

            if has_initial {
                let sub_owned: Vec<(String, bool)> = sub.to_vec();
                let mut results = trie.search_abbreviation_mixed(&sub_owned, 200, base_syllables);
                let top_n = results.len().min(3);
                if top_n > 0 {
                    results.select_nth_unstable_by_key(top_n - 1, |r| std::cmp::Reverse(r.weight));
                }
                for tr in results.into_iter().take(top_n) {
                    graph.push(WordSpan {
                        start,
                        end,
                        word: tr.word.to_string(),
                        pinyin: py.clone(),
                        initial_count,
                    });
                }
            } else {
                if let Some(entries) = trie.get_all_exact(&py) {
                    let mut candidates: Vec<&crate::trie::TrieResult> = entries.iter().collect();
                    let top_n = candidates.len().min(3);
                    if top_n > 0 {
                        candidates.select_nth_unstable_by_key(top_n - 1, |r| std::cmp::Reverse(r.weight));
                    }
                    for tr in candidates.into_iter().take(top_n) {
                        graph.push(WordSpan {
                            start,
                            end,
                            word: tr.word.to_string(),
                            pinyin: py.clone(),
                            initial_count,
                        });
                    }
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

                // 多声母合并成词奖励：有词组词，没词才退成字
                let abbr_bonus = if span.initial_count >= 2 {
                    span.initial_count as f64 * 5000.0
                } else {
                    0.0
                };

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

                let score = prev.score + freq_score + ngram_bonus + word_len_bonus + abbr_bonus;

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
        assert_eq!(segs, vec!["wo","wang","ji","chong","dian","le"]);
    }

    #[test]
    fn test_segment_for_compose_mixed_pinyin() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let trie = crate::trie::Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");
        let freq = load_syllable_freq(&root);
        let base = load_base_syllables(&root);

        // 混拼: jtdayouxi = j(beginning) t(beginning) da(full) youxi(full)
        let segs = segment_for_compose("jtdayouxi", &trie, &freq, &base);
        let pinyins: Vec<&str> = segs.iter().map(|(s, _)| s.as_str()).collect();
        let initials: Vec<bool> = segs.iter().map(|(_, is_init)| *is_init).collect();
        println!("segs: {:?}, initials: {:?}", pinyins, initials);
        // 应该能分出声母段和全拼段
        assert_eq!(pinyins, vec!["j", "t", "da", "youxi"]);
        assert_eq!(initials, vec![true, true, false, false]);

        // 纯全拼不应该被误判为混拼
        let segs2 = segment_for_compose("wowangjichongdianle", &trie, &freq, &base);
        let initials2: Vec<bool> = segs2.iter().map(|(_, is_init)| *is_init).collect();
        assert!(initials2.iter().all(|b| !b), "full pinyin should not have initials");
    }

    #[test]
    fn test_compose_mixed_abbreviation() {
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
            HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()
        )));

        // 混拼: jtdayouxi → "有词组词"，今天(2声母)胜于就/太(单声母)
        let paths = compose("jtdayouxi", &trie, &ngram.load(), &freq, &base, "chinese");
        println!("Mixed compose paths: {} found", paths.len());
        for p in &paths {
            let text: String = p.words.iter().map(|w| w.word.as_str()).collect();
            println!("  path: {} (score={:.3})", text, p.score);
        }
        assert!(!paths.is_empty(), "Mixed abbreviation should produce compose paths");

        // "今天打游戏" 应该出现在前 5 条路径中
        let top_5: Vec<String> = paths.iter().take(5)
            .map(|p| p.words.iter().map(|w| w.word.as_str()).collect::<String>())
            .collect();
        assert!(
            top_5.iter().any(|t| t == "今天打游戏"),
            "今天打游戏 should be in top-5, got {:?}", top_5
        );
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
