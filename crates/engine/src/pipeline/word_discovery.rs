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

pub fn discover_words(text: &str, config: &DiscoveryConfig, known_words: &std::collections::HashSet<String>) -> Vec<DiscoveredWord> {
    // 1. 文本预处理：保留汉字
    let sentences: Vec<String> = text
        .split(|c: char| !is_chinese(c))
        .filter(|s| s.chars().count() > 1)
        .map(|s| s.to_string())
        .collect();

    let total_chars: usize = sentences.iter().map(|s| s.chars().count()).sum();
    if total_chars == 0 {
        return vec![];
    }

    // 2. 统计 N-gram (1 to max_word_len + 1)
    let mut ngrams: HashMap<String, usize> = HashMap::new();
    for s in &sentences {
        let chars: Vec<char> = s.chars().collect();
        let slen = chars.len();
        for n in 1..=(config.max_word_len + 1) {
            for i in 0..=(slen.saturating_sub(n)) {
                let gram: String = chars[i..i + n].iter().collect();
                *ngrams.entry(gram).or_insert(0) += 1;
            }
        }
    }

    // 3. 计算左右邻居频率 (用于熵)
    let mut right_neighbors: HashMap<String, HashMap<char, usize>> = HashMap::new();
    let mut left_neighbors: HashMap<String, HashMap<char, usize>> = HashMap::new();

    for (gram, count) in &ngrams {
        let chars: Vec<char> = gram.chars().collect();
        if chars.len() < 2 { continue; }

        // gram = word + last_char
        let word: String = chars[..chars.len()-1].iter().collect();
        let last_char = chars[chars.len()-1];
        right_neighbors.entry(word).or_default().insert(last_char, *count);

        // gram = first_char + suffix
        let first_char = chars[0];
        let suffix: String = chars[1..].iter().collect();
        left_neighbors.entry(suffix).or_default().insert(first_char, *count);
    }

    // 4. 计算指标并筛选
    let mut results: Vec<DiscoveredWord> = ngrams
        .par_iter()
        .filter_map(|(word, &count)| {
            let wlen = word.chars().count();
            if wlen < 2 || wlen > config.max_word_len { return None; }
            if count < config.min_count { return None; }
            if known_words.contains(word) { return None; }

            // 过滤边界停用词
            if is_boundary_stopword(word) { return None; }

            // 计算 PMI (凝聚度)
            let mut min_pmi = f64::MAX;
            let p_word = count as f64 / total_chars as f64;
            let word_chars: Vec<char> = word.chars().collect();
            for k in 1..wlen {
                let part1: String = word_chars[..k].iter().collect();
                let part2: String = word_chars[k..].iter().collect();
                let c1 = *ngrams.get(&part1).unwrap_or(&0);
                let c2 = *ngrams.get(&part2).unwrap_or(&0);
                if c1 > 0 && c2 > 0 {
                    let p1 = c1 as f64 / total_chars as f64;
                    let p2 = c2 as f64 / total_chars as f64;
                    let pmi = (p_word / (p1 * p2)).log2();
                    if pmi < min_pmi { min_pmi = pmi; }
                }
            }
            if min_pmi < config.min_pmi { return None; }

            // 计算自由度 (信息熵)
            let r_entropy = compute_entropy(right_neighbors.get(word));
            let l_entropy = compute_entropy(left_neighbors.get(word));
            let entropy = r_entropy.min(l_entropy);
            if entropy < config.min_entropy { return None; }

            Some(DiscoveredWord {
                word: word.clone(),
                count,
                pmi: min_pmi,
                entropy,
            })
        })
        .collect();

    // 排序：词频优先
    results.sort_by(|a, b| b.count.cmp(&a.count));
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

fn compute_entropy(neighbors: Option<&HashMap<char, usize>>) -> f64 {
    let neighbors = match neighbors {
        Some(n) => n,
        None => return 0.0,
    };
    let total: usize = neighbors.values().sum();
    if total == 0 { return 0.0; }
    let mut entropy = 0.0;
    for &count in neighbors.values() {
        let p = count as f64 / total as f64;
        entropy -= p * p.log2();
    }
    entropy
}
