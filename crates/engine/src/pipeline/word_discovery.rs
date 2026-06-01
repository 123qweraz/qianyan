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

struct WordStats {
    count: usize,
    left: HashMap<char, usize>,
    right: HashMap<char, usize>,
}

/// 对一段纯汉字文本做单遍扫描：
/// - 统计 n-gram（1..=max_word_len-1，只保留 PMI 需要的部分）
/// - 收集候选词（2..=max_word_len）的频次和左右邻字
fn scan_sentence(s: &str, max_word_len: usize) -> (Counter, HashMap<String, WordStats>) {
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len();
    let mut ngrams: Counter = Counter::new();
    let mut stats: HashMap<String, WordStats> = HashMap::new();

    for start in 0..slen {
        let max_win_end = (start + max_word_len).min(slen);

        // n-gram 1..=max_word_len-1（PMI 子部件需要）
        let max_ng_end = (start + max_word_len.saturating_sub(1)).min(slen);
        for end in (start + 1)..=max_ng_end {
            let gram: String = chars[start..end].iter().collect();
            *ngrams.entry(gram).or_insert(0) += 1;
        }

        // 候选词 2..=max_word_len + 邻字
        for end in (start + 2)..=max_win_end {
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

    (ngrams, stats)
}

/// 将 (ng2, st2) 合并入 (ng1, st1)，释放 ng2/st2
fn merge_into(ng1: &mut Counter, st1: &mut HashMap<String, WordStats>, ng2: Counter, st2: HashMap<String, WordStats>) {
    for (k, v) in ng2 {
        *ng1.entry(k).or_insert(0) += v;
    }
    for (word, ws2) in st2 {
        let ws1 = st1.entry(word).or_insert(WordStats {
            count: 0,
            left: HashMap::new(),
            right: HashMap::new(),
        });
        ws1.count += ws2.count;
        for (c, cnt) in ws2.left {
            *ws1.left.entry(c).or_insert(0) += cnt;
        }
        for (c, cnt) in ws2.right {
            *ws1.right.entry(c).or_insert(0) += cnt;
        }
    }
}

/// 计算信息熵 H(X) = -Σ p(x) log2 p(x)
fn entropy(dist: &HashMap<char, usize>) -> f64 {
    let total: usize = dist.values().sum();
    if total == 0 {
        return 0.0;
    }
    let mut e = 0.0;
    for &c in dist.values() {
        let p = c as f64 / total as f64;
        e -= p * p.log2();
    }
    e
}

pub fn discover_words(
    text: &str,
    config: &DiscoveryConfig,
    known_words: &std::collections::HashSet<String>,
) -> Vec<DiscoveredWord> {
    // 1. 提取纯汉字段落
    let sentences: Vec<&str> = text
        .split(|c: char| !is_chinese(c))
        .filter(|s| s.chars().count() > 1)
        .collect();

    if sentences.is_empty() {
        return vec![];
    }

    // 2. 分块 + 逐块合并（内存 O(全局 HashMap + 一个块)）
    const CHUNK_CHARS: usize = 50_000;
    let mut global_ngrams: Counter = Counter::new();
    let mut global_stats: HashMap<String, WordStats> = HashMap::new();

    let mut i = 0;
    while i < sentences.len() {
        let mut chunk_chars = 0usize;
        let chunk_start = i;
        while i < sentences.len() && chunk_chars < CHUNK_CHARS {
            chunk_chars += sentences[i].chars().count();
            i += 1;
        }
        let chunk = &sentences[chunk_start..i];

        // 块内句子级并行扫描 + fold 减少中间 HashMap 数量
        let chunk_result: (Counter, HashMap<String, WordStats>) = chunk
            .par_iter()
            .fold(
                || (Counter::new(), HashMap::new()),
                |mut acc, s| {
                    let (ng, st) = scan_sentence(s, config.max_word_len);
                    merge_into(&mut acc.0, &mut acc.1, ng, st);
                    acc
                },
            )
            .reduce(
                || (Counter::new(), HashMap::new()),
                |mut a, b| {
                    merge_into(&mut a.0, &mut a.1, b.0, b.1);
                    a
                },
            );

        merge_into(&mut global_ngrams, &mut global_stats, chunk_result.0, chunk_result.1);
    }

    if global_ngrams.is_empty() {
        return vec![];
    }

    let total_ngram_count: usize = global_ngrams.values().sum();

    // 3. 并行筛选
    let mut discovered: Vec<DiscoveredWord> = global_stats
        .par_iter()
        .filter_map(|(word, ws)| {
            if ws.count < config.min_count {
                return None;
            }
            if known_words.contains(word) {
                return None;
            }
            if is_boundary_stopword(word) {
                return None;
            }

            // PMI（凝聚度）
            let p_word = ws.count as f64 / total_ngram_count as f64;
            let wlen = word.chars().count();
            let word_chars: Vec<char> = word.chars().collect();
            let mut min_pmi = f64::MAX;
            for k in 1..wlen {
                let part1: String = word_chars[..k].iter().collect();
                let part2: String = word_chars[k..].iter().collect();
                let c1 = global_ngrams.get(&part1).copied().unwrap_or(0);
                let c2 = global_ngrams.get(&part2).copied().unwrap_or(0);
                if c1 > 0 && c2 > 0 {
                    let p1 = c1 as f64 / total_ngram_count as f64;
                    let p2 = c2 as f64 / total_ngram_count as f64;
                    let pmi = (p_word / (p1 * p2)).log2();
                    if pmi < min_pmi {
                        min_pmi = pmi;
                    }
                }
            }
            if min_pmi >= f64::MAX || min_pmi < config.min_pmi {
                return None;
            }

            // 自由度（信息熵）
            let r_entropy = entropy(&ws.right);
            let l_entropy = entropy(&ws.left);
            let ent = r_entropy.min(l_entropy);
            if ent < config.min_entropy {
                return None;
            }

            Some(DiscoveredWord {
                word: word.clone(),
                count: ws.count,
                pmi: min_pmi,
                entropy: ent,
            })
        })
        .collect();

    discovered.par_sort_unstable_by(|a, b| b.count.cmp(&a.count));
    discovered
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
