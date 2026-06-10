use crate::trie::Trie;

#[derive(Debug, Clone)]
pub struct WordSpan {
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub pinyin: String,  // 该词对应的拼音，用于查 syllable_freq
}

#[derive(Debug, Clone)]
pub struct ComposePath {
    pub words: Vec<WordSpan>,
    pub score: f64,
}

fn load_syllables() -> std::collections::HashSet<String> {
    static SYLLABLES: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    let set = SYLLABLES.get_or_init(|| {
        let paths = [
            std::path::PathBuf::from("dicts/chinese/single_syllables.txt"),
            std::path::PathBuf::from("../dicts/chinese/single_syllables.txt"),
            std::path::PathBuf::from("../../dicts/chinese/single_syllables.txt"),
        ];
        match paths.iter().find_map(|p| std::fs::read_to_string(p).ok()) {
            Some(c) => c.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect(),
            None => std::collections::HashSet::new(),
        }
    });
    set.clone()
}

pub fn segment_syllables(pinyin: &str) -> Vec<String> {
    let sylls = load_syllables();
    let mut segs = Vec::new();
    let mut pos = 0;
    while pos < pinyin.len() {
        let max_len = (pinyin.len() - pos).min(6);
        let mut matched = false;
        for len in (1..=max_len).rev() {
            let end = pos + len;
            if pinyin.is_char_boundary(end) && sylls.contains(&pinyin[pos..end]) {
                segs.push(pinyin[pos..end].to_string());
                pos = end;
                matched = true;
                break;
            }
        }
        if !matched {
            segs.push(pinyin[pos..].to_string());
            break;
        }
    }
    segs
}

pub fn compose(
    pinyin: &str,
    trie: &Trie,
    ngram: &crate::config_manager::UserDictData,
    syllable_freq: &std::collections::HashMap<String, u64>,
    profile: &str,
) -> Vec<ComposePath> {
    let segs = segment_syllables(pinyin);
    if segs.len() < 2 { return Vec::new(); }
    let graph = build_word_graph(&segs, trie);
    viterbi_compose(&graph, segs.len(), syllable_freq, ngram, profile)
}

fn build_word_graph(segments: &[String], trie: &Trie) -> Vec<WordSpan> {
    let n = segments.len();
    let mut best: std::collections::HashMap<(usize, usize), (String, String)> = std::collections::HashMap::new();
    for start in 0..n {
        let max_k = 4.min(n - start);
        for k in 1..=max_k {
            let end = start + k;
            let py: String = segments[start..end].concat();
            if let Some(entries) = trie.get_all_exact(&py) {
                if let Some(tr) = entries.iter().max_by_key(|r| r.weight) {
                    let key = (start, end);
                    // 总是取最高频词
                    best.entry(key).or_insert_with(|| (tr.word.to_string(), py.clone()));
                }
            }
        }
    }
    // 也对每个key取最高频的：重新扫描一次确保是全局最高
    let mut graph = Vec::new();
    for ((start, end), (word, py)) in best {
        // 重新取一次确保是最高频
        let py_check = segments[start..end].concat();
        if let Some(entries) = trie.get_all_exact(&py_check) {
            if let Some(tr) = entries.iter().max_by_key(|r| r.weight) {
                graph.push(WordSpan { start, end, word: tr.word.to_string(), pinyin: py_check });
            }
        } else {
            graph.push(WordSpan { start, end, word, pinyin: py });
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

                // 核心：用 syllable_freq 作为主得分（不在表 = 0 = 惩罚单字）
                let freq = *syllable_freq.get(&span.pinyin).unwrap_or(&0) as f64;
                let ngram_bonus = if let (Some(last), Some(pn)) = (
                    prev.words.last().map(|w| w.word.as_str()),
                    ngram.get(profile),
                ) {
                    pn.get(last)
                        .and_then(|e| e.iter().find(|(w, _)| w == &span.word))
                        .map(|(_, c)| ((*c).min(10) as f64 + 1.0).ln() * 3.0)
                        .unwrap_or(0.0)
                } else {
                    0.0
                };

                let score = prev.score + freq * 0.001 + ngram_bonus;

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

    #[test]
    fn test_segment_syllables() {
        let segs = segment_syllables("wowangjichongdianle");
        assert_eq!(segs, vec!["wo","wang","ji","chong","dian","le"]);
    }

    #[test]
    fn test_pipeline_segmentor() {
        use std::path::PathBuf;
        use std::sync::Arc;
        use arc_swap::ArcSwap;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let mut trie_paths = std::collections::HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));
        let syllable_freq: std::collections::HashMap<String, u64> = {
            let path = root.join("dicts/chinese/syllable_freq.txt");
            let content = std::fs::read_to_string(path).unwrap();
            content.lines().filter_map(|l| {
                let mut parts = l.trim().split_whitespace();
                let key = parts.next()?.to_string();
                let val: u64 = parts.next()?.parse().ok()?;
                Some((key, val))
            }).collect()
        };
        let shared = Arc::new(ArcSwap::new(Arc::new(
            std::collections::HashMap::<String, std::collections::HashMap<String, Vec<(String, u32)>>>::new()
        )));
        let engine = crate::pipeline::SearchEngine::new(
            trie_paths,
            Arc::new(syllable_freq),
            shared.clone(),
            shared.clone(),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::new()))),
            Arc::new(std::collections::HashMap::new()),
        );
        assert!(!engine.base_syllables.is_empty(), "base_syllables should be loaded");
        println!("base_syllables loaded: {} entries", engine.base_syllables.len());

        // Test Viterbi segmentation via engine's pipeline segmentor
        let segmentor = crate::pipeline::segmentation::DefaultSegmentor;
        use crate::pipeline::segmentation::Segmentor;
        let segs = segmentor.segment("wowangjichongdianle", "", &engine.syllable_freq, &engine.base_syllables);
        println!("segmentor({:?}) -> {:?}", "wowangjichongdianle", segs);
        assert!(!segs.is_empty(), "segmentor should produce segments");
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

        // Load syllable_freq.txt
        let freq: std::collections::HashMap<String, u64> = {
            let content = std::fs::read_to_string(
                root.join("dicts/chinese/syllable_freq.txt")
            ).unwrap();
            content.lines().map(|l| {
                let mut parts = l.trim().split_whitespace();
                let key = parts.next().unwrap_or("").to_string();
                let val: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
                (key, val)
            }).collect()
        };

        let ngram = Arc::new(ArcSwap::new(Arc::new(
            std::collections::HashMap::<String, std::collections::HashMap<String, Vec<(String, u32)>>>::new()
        )));
        let paths = compose("wowangjichongdianle", &trie, &ngram.load(), &freq, "chinese");
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
