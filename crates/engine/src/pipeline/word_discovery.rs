use std::collections::HashMap;
use rayon::prelude::*;

#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub min_count: usize,
    pub min_pmi: f64,
    pub min_entropy: f64,
    pub max_word_len: usize,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            min_count: 5,
            min_pmi: 3.0,
            min_entropy: 0.8,
            max_word_len: 4,
        }
    }
}

pub struct DiscoveredWord {
    pub word: String,
    pub count: usize,
    pub pmi: f64,
    pub entropy: f64,
}

type Counter = HashMap<String, usize>;

/// 对一段纯汉字文本统计 n-gram（n = 1..=max_n）
fn count_ngrams(text: &str, max_n: usize) -> Counter {
    let chars: Vec<char> = text.chars().collect();
    let slen = chars.len();
    let mut map = Counter::new();
    for n in 1..=max_n {
        if n > slen { continue; }
        for win in chars.windows(n) {
            let s: String = win.iter().collect();
            *map.entry(s).or_insert(0) += 1;
        }
    }
    map
}

/// 计算信息熵 H(X) = -Σ p(x) log2 p(x)
fn entropy(dist: &HashMap<char, usize>) -> f64 {
    let total: usize = dist.values().sum();
    if total == 0 { return 0.0 }
    let mut e = 0.0;
    for &c in dist.values() {
        let p = c as f64 / total as f64;
        e -= p * p.log2();
    }
    e
}

/// 从纯汉字文本中统计每个候选词（长度 2..=max_word_len）的
/// - 频次
/// - 左邻字分布、右邻字分布
struct WordStats {
    count: usize,
    left: HashMap<char, usize>,
    right: HashMap<char, usize>,
}

fn collect_word_stats(text: &str, max_word_len: usize) -> HashMap<String, WordStats> {
    let chars: Vec<char> = text.chars().collect();
    let slen = chars.len();
    let mut stats: HashMap<String, WordStats> = HashMap::new();

    // 滑动窗口，对每个位置提取候选词及邻字
    for start in 0..slen {
        // 候选词长度 2..=max_word_len
        let max_end = (start + max_word_len).min(slen);
        for end in (start + 2)..=max_end {
            let word: String = chars[start..end].iter().collect();

            let left_char = if start > 0 { Some(chars[start - 1]) } else { None };
            let right_char = if end < slen { Some(chars[end]) } else { None };

            let entry = stats.entry(word).or_insert(WordStats {
                count: 0,
                left: HashMap::new(),
                right: HashMap::new(),
            });
            entry.count += 1;
            if let Some(c) = left_char {
                *entry.left.entry(c).or_insert(0) += 1;
            }
            if let Some(c) = right_char {
                *entry.right.entry(c).or_insert(0) += 1;
            }
        }
    }
    stats
}

pub fn discover_words(text: &str, config: &DiscoveryConfig, known_words: &std::collections::HashSet<String>) -> Vec<DiscoveredWord> {
    const MAX_CHARS: usize = 300_000;
    let text = if text.chars().count() > MAX_CHARS {
        log::warn!("[word_discovery] text too long, truncating to {} chars", MAX_CHARS);
        let end = text.char_indices().nth(MAX_CHARS).map(|(i, _)| i).unwrap_or(text.len());
        &text[..end]
    } else {
        text
    };

    // 1. 提取纯汉字段落（长度 > 1）
    let sentences: Vec<&str> = text
        .split(|c: char| !is_chinese(c))
        .filter(|s| s.chars().count() > 1)
        .collect();

    if sentences.is_empty() { return vec![]; }

    // 2. 统计全量 n-gram（1..=max_word_len+1，用于 PMI 计算）
    let total_ngram_count: usize = sentences.iter()
        .map(|s| {
            let len = s.chars().count();
            let max_n = config.max_word_len + 1;
            (1..=max_n.min(len)).map(|n| len - n + 1).sum::<usize>()
        })
        .sum();
    if total_ngram_count == 0 { return vec![]; }

    let ngrams: Counter = sentences.iter()
        .map(|s| count_ngrams(s, config.max_word_len + 1))
        .reduce(|mut a, b| {
            for (k, v) in b { *a.entry(k).or_insert(0) += v; }
            a
        })
        .unwrap_or_default();

    // 3. 收集候选词统计（频次 + 邻字）
    let concat: String = sentences.concat();
    let word_stats = collect_word_stats(&concat, config.max_word_len);

    // 4. 并行筛选
    let mut results: Vec<DiscoveredWord> = word_stats
        .par_iter()
        .filter_map(|(word, ws)| {
            if ws.count < config.min_count { return None; }
            if known_words.contains(word) { return None; }
            if is_boundary_stopword(word) { return None; }

            // PMI（凝聚度）：所有切分位置取最小值
            let p_word = ws.count as f64 / total_ngram_count as f64;
            let wlen = word.chars().count();
            let word_chars: Vec<char> = word.chars().collect();
            let mut min_pmi = f64::MAX;
            for k in 1..wlen {
                let part1: String = word_chars[..k].iter().collect();
                let part2: String = word_chars[k..].iter().collect();
                let c1 = ngrams.get(&part1).copied().unwrap_or(0);
                let c2 = ngrams.get(&part2).copied().unwrap_or(0);
                if c1 > 0 && c2 > 0 {
                    let p1 = c1 as f64 / total_ngram_count as f64;
                    let p2 = c2 as f64 / total_ngram_count as f64;
                    let pmi = (p_word / (p1 * p2)).log2();
                    if pmi < min_pmi { min_pmi = pmi; }
                }
            }
            if min_pmi >= f64::MAX || min_pmi < config.min_pmi { return None; }

            // 自由度（信息熵）：取左右熵的较小值
            let r_entropy = entropy(&ws.right);
            let l_entropy = entropy(&ws.left);
            let ent = r_entropy.min(l_entropy);
            if ent < config.min_entropy { return None; }

            Some(DiscoveredWord {
                word: word.clone(),
                count: ws.count,
                pmi: min_pmi,
                entropy: ent,
            })
        })
        .collect();

    results.par_sort_unstable_by(|a, b| b.count.cmp(&a.count));
    results
}

fn is_chinese(c: char) -> bool {
    (c >= '\u{4e00}' && c <= '\u{9fa5}') || (c >= '\u{3400}' && c <= '\u{4dbf}')
}

fn is_boundary_stopword(word: &str) -> bool {
    let stopwords = "的了和是在有而及与或之为其于以到等说着也就都吧呢吗啊呀让把给被";
    let first = word.chars().next().unwrap_or(' ');
    let last = word.chars().last().unwrap_or(' ');
    stopwords.contains(first) || stopwords.contains(last)
}
