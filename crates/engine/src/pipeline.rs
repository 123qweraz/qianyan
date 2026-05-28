use crate::config_manager::UserDictData;
use crate::processor::Action;
use crate::trie::TrieResult;
use crate::Config;
use crate::EngineContext;
use crate::FuzzyPinyinConfig;
use crate::Trie;
use arc_swap::ArcSwap;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

// 调频衰减算法参数
pub(crate) const RECENCY_BOOST_BASE: f64 = 6000.0;
pub(crate) const FREQ_BOOST_SCALE: f64 = 2000.0;
pub(crate) const MAX_USAGE_BOOST: f64 = 15000.0;
pub(crate) const NGRAM_BOOST_SCALE: f64 = 3000.0;
pub(crate) const MAX_NGRAM_BOOST: f64 = 10000.0;

const PREWARM_ENTRIES: usize = 1000;
const MAX_LOOKUP_LIMIT: usize = 500;
const CACHE_TTL_MS: u64 = 300;

/// 候选项
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub text: Arc<str>,
    pub simplified: Arc<str>,
    pub traditional: Arc<str>,
    pub hint: Arc<str>,
    pub english_aux: Arc<str>,
    pub stroke_aux: Arc<str>,
    pub source: Arc<str>, // 来源：如 "User", "Table", "Script"
    pub weight: f64,
    pub match_level: u8, // 0: unknown, 1: prefix, 2: abbreviation/wildcard, 3: exact
}

/* 核心接口定义 */

pub trait Segmentor: Send + Sync {
    fn segment(&self, input: &str, syllables: &HashSet<String>, delimiters: &str, syllable_freq: &HashMap<String, u64>, base_syllables: &HashSet<String>) -> Vec<String>;
}

pub trait Translator: Send + Sync + 'static {
    fn translate(
        &self,
        input: &str,
        segments: &[String],
        config: &Config,
        limit: usize,
    ) -> Vec<Candidate>;
    fn as_any(&self) -> &dyn std::any::Any;
}

pub trait Filter: Send + Sync {
    fn filter(
        &self,
        input: &str,
        candidates: Vec<Candidate>,
        config: &Config,
        context: Option<&str>,
    ) -> Vec<Candidate>;
}

/* 具体实现 */

/// 默认切分器实现 (Viterbi DP)
pub struct DefaultSegmentor;
impl Segmentor for DefaultSegmentor {
    fn segment(&self, input: &str, syllables: &HashSet<String>, delimiters: &str, syllable_freq: &HashMap<String, u64>, base_syllables: &HashSet<String>) -> Vec<String> {
        // 快速路径：如果输入已经是小写字母/数字，直接使用（避免 to_lowercase 分配）
        let needs_lowercase = !input
            .bytes()
            .all(|b: u8| b.is_ascii_lowercase() || b.is_ascii_digit());

        if needs_lowercase {
            let input_lower = input.to_lowercase();
            return Self::segment_lowercase(&input_lower, syllables, delimiters, syllable_freq, base_syllables);
        }

        Self::segment_lowercase(input, syllables, delimiters, syllable_freq, base_syllables)
    }
}

impl DefaultSegmentor {
    /// 尝试相邻字母换位纠错（处理 guna→guan、guagn→guang 等常见 finger slip）
    fn try_transpose(part: &str, syllable_freq: &HashMap<String, u64>, base_syllables: &HashSet<String>) -> Option<(u64, String)> {
        let bytes = part.as_bytes();
        for i in 0..bytes.len().saturating_sub(1) {
            let mut swapped = bytes.to_vec();
            swapped.swap(i, i + 1);
            if let Ok(candidate) = String::from_utf8(swapped) {
                if let Some(&freq) = syllable_freq.get(&candidate) {
                    return Some((freq / 2, candidate));
                }
                if base_syllables.contains(&candidate) {
                    return Some((0, candidate));
                }
            }
        }
        None
    }

    /// 单轮 Viterbi DP 切分：直接在原始字符串上做最优切分，避免两轮法中的贪心锁定问题
    pub(crate) fn viterbi_segment(input: &str, syllable_freq: &HashMap<String, u64>, base_syllables: &HashSet<String>) -> Vec<String> {
        let n = input.len();
        if n == 0 {
            return vec![];
        }

        // dp[i] = (best_total_freq, segment_count, prev_pos)
        let mut dp: Vec<Option<(u64, usize, usize)>> = vec![None; n + 1];
        dp[0] = Some((0, 0, 0));
        // 换位修正后的文本（非空时优先于 input[prev..pos]）
        let mut corrected: Vec<String> = vec![String::new(); n + 1];

        for i in 0..n {
            let Some((cur_freq, cur_seg, _)) = dp[i] else { continue };
            let max_len = 12.min(n - i);

            for len in 1..=max_len {
                if !input.is_char_boundary(i + len) { continue; }
                let part = &input[i..i + len];

                let (freq, seg_text) = if syllable_freq.contains_key(part) {
                    (*syllable_freq.get(part).unwrap(), None)
                } else if base_syllables.contains(part) {
                    (0, None)
                } else if len == 1 {
                    (0, None) // 单字符兜底
                } else if let Some((xfreq, xtext)) = Self::try_transpose(part, syllable_freq, base_syllables) {
                    (xfreq, Some(xtext))
                } else {
                    continue;
                };

                let total = cur_freq + freq;
                let seg_cnt = cur_seg + 1;
                let entry = &mut dp[i + len];

                let should_replace = match entry {
                    None => true,
                    Some((best_freq, best_seg, _)) =>
                        total > *best_freq || (total == *best_freq && seg_cnt < *best_seg),
                };

                if should_replace {
                    *entry = Some((total, seg_cnt, i));
                    if let Some(text) = seg_text {
                        corrected[i + len] = text;
                    }
                }
            }
        }

        // 回溯重建
        let mut segments: Vec<String> = Vec::new();
        let mut pos = n;
        while pos > 0 {
            match dp[pos] {
                Some((_, _, prev)) if prev < pos => {
                    if !corrected[pos].is_empty() {
                        segments.push(corrected[pos].clone());
                    } else {
                        segments.push(input[prev..pos].to_string());
                    }
                    pos = prev;
                }
                _ => {
                    // 不可达位置：逐字符兜底
                    let prev = input[..pos].char_indices().next_back().map(|(i, _)| i).unwrap_or(pos.saturating_sub(1));
                    segments.push(input[prev..pos].to_string());
                    pos = prev;
                }
            }
        }
        segments.reverse();
        segments
    }

    #[inline]
    fn segment_lowercase(input: &str, _syllables: &HashSet<String>, delimiters: &str, syllable_freq: &HashMap<String, u64>, base_syllables: &HashSet<String>) -> Vec<String> {
        if input.is_empty() {
            return vec![];
        }

        let mut result = Vec::new();
        for chunk in input.split(|c: char| delimiters.contains(c)) {
            if chunk.is_empty() { continue; }
            result.extend(Self::viterbi_segment(chunk, syllable_freq, base_syllables));
        }
        result
    }
}

/// 对每个音节段生成模糊音变体（迭代式：新变体也继续应用规则，与 chinese.rs 保持一致）
pub(crate) fn fuzzy_variants_per_segment(seg: &str, fuzzy: &FuzzyPinyinConfig) -> Vec<String> {
    let pinyin_lower = if seg.bytes().all(|b| b.is_ascii_lowercase()) {
        seg.to_string()
    } else {
        seg.to_lowercase()
    };
    let mut new_variants = std::collections::HashSet::new();
    new_variants.insert(pinyin_lower);

    let mut to_process: Vec<String> = new_variants.iter().cloned().collect();
    while let Some(v) = to_process.pop() {
        if fuzzy.z_zh {
            if v.starts_with("zh") {
                let replaced = v.replacen("zh", "z", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with("z") {
                let replaced = v.replacen("z", "zh", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.c_ch {
            if v.starts_with("ch") {
                let replaced = v.replacen("ch", "c", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with("c") {
                let replaced = v.replacen("c", "ch", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.s_sh {
            if v.starts_with("sh") {
                let replaced = v.replacen("sh", "s", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with("s") {
                let replaced = v.replacen("s", "sh", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.n_l {
            if v.starts_with('n') {
                let replaced = v.replacen('n', "l", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with('l') {
                let replaced = v.replacen('l', "n", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.r_l {
            if v.starts_with('r') {
                let replaced = v.replacen('r', "l", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with('l') {
                let replaced = v.replacen('l', "r", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.f_h {
            if v.starts_with('f') {
                let replaced = v.replacen('f', "h", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.starts_with('h') {
                let replaced = v.replacen('h', "f", 1);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.an_ang {
            if v.ends_with("ang") {
                let replaced = format!("{}an", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("an") {
                let replaced = format!("{}ang", &v[..v.len() - 2]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.en_eng {
            if v.ends_with("eng") {
                let replaced = format!("{}en", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("en") {
                let replaced = format!("{}eng", &v[..v.len() - 2]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.in_ing {
            if v.ends_with("ing") {
                let replaced = format!("{}in", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("in") {
                let replaced = format!("{}ing", &v[..v.len() - 2]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.ian_iang {
            if v.ends_with("iang") {
                let replaced = format!("{}ian", &v[..v.len() - 4]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("ian") {
                let replaced = format!("{}iang", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.uan_uang {
            if v.ends_with("uang") {
                let replaced = format!("{}uan", &v[..v.len() - 4]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.ends_with("uan") {
                let replaced = format!("{}uang", &v[..v.len() - 3]);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        if fuzzy.u_v {
            if v.contains('u') {
                let replaced = v.replace('u', "v");
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            } else if v.contains('v') {
                let replaced = v.replace('v', "u");
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
        for (from, to) in &fuzzy.custom_mappings {
            if v.contains(from) {
                let replaced = v.replace(from, to);
                if new_variants.insert(replaced.clone()) {
                    to_process.push(replaced);
                }
            }
        }
    }

    let mut result: Vec<String> = new_variants.into_iter().collect();
    result.sort();
    result
}

/// 带前缀剪枝的 DFS 模糊音搜索：只遍历在词典中有前缀匹配的分支，
/// 避免对不存在的拼音组合做无用的精确查询。
struct FuzzyPinyinSearcher<'a, 'b> {
    trie: &'a Trie,
    per_seg: &'b [Vec<String>],
    candidates: &'b mut Vec<Candidate>,
    seen: &'b mut HashSet<&'a str>,
    query: &'b str,
    build_hint: &'b dyn Fn(&TrieResult<'a>) -> Arc<str>,
    limit: usize,
}

impl<'a, 'b> FuzzyPinyinSearcher<'a, 'b> {
    fn search(&mut self) {
        let mut buf = String::new();
        self.dfs(0, &mut buf);
    }

    fn dfs(&mut self, depth: usize, buf: &mut String) {
        if self.candidates.len() >= self.limit {
            return;
        }
        if depth == self.per_seg.len() {
            if buf == self.query {
                return;
            }
            if let Some(exact_results) = self.trie.get_all_exact(buf) {
                for tr in exact_results {
                    if self.seen.insert(tr.word) {
                        self.candidates.push(Candidate {
                            simplified: Arc::from(tr.word),
                            traditional: if tr.trad.is_empty() {
                                Arc::from(tr.word)
                            } else {
                                Arc::from(tr.trad)
                            },
                            text: Arc::from(tr.word),
                            hint: (self.build_hint)(&tr),
                            english_aux: Arc::from(tr.en),
                            stroke_aux: Arc::from(tr.stroke_aux),
                            source: Arc::from("Table (Fuzzy)"),
                            weight: tr.weight as f64,
                            match_level: 2,
                        });
                        if self.candidates.len() >= self.limit {
                            return;
                        }
                    }
                }
            }
            return;
        }

        for variant in &self.per_seg[depth] {
            let start = buf.len();
            buf.push_str(variant);

            if depth + 1 == self.per_seg.len() || self.trie.has_prefix(buf) {
                self.dfs(depth + 1, buf);
                if self.candidates.len() >= self.limit {
                    buf.truncate(start);
                    return;
                }
            }

            buf.truncate(start);
        }
    }
}

pub struct TableTranslator {
    pub trie: Arc<Trie>,
    pub syllables: Arc<HashSet<String>>,
    pub enable_abbreviation: bool,
    last_query: std::sync::RwLock<(String, std::time::Instant)>,
    cached_candidates: std::sync::RwLock<Vec<Candidate>>,
}

impl TableTranslator {
    pub fn new(
        trie: Arc<Trie>,
        syllables: Arc<HashSet<String>>,
        enable_abbreviation: bool,
    ) -> Self {
        Self {
            trie,
            syllables,
            enable_abbreviation,
            last_query: std::sync::RwLock::new((String::new(), std::time::Instant::now())),
            cached_candidates: std::sync::RwLock::new(Vec::new()),
        }
    }
}

impl Translator for TableTranslator {
    fn translate(
        &self,
        _input: &str,
        segments: &[String],
        config: &Config,
        limit: usize,
    ) -> Vec<Candidate> {
        if segments.is_empty() {
            return vec![];
        }
        let query = segments.join("");
        let internal_limit = limit.max(MAX_LOOKUP_LIMIT);

        // 检查缓存是否可以复用（增量搜索优化）
        {
            if let (Ok(last_q_guard), Ok(cached)) =
                (self.last_query.read(), self.cached_candidates.read())
            {
                let (last_q, last_time) = &*last_q_guard;

                if query.starts_with(last_q)
                    && last_time.elapsed().as_millis() < CACHE_TTL_MS as u128
                {
                    // 新的查询是之前查询的延伸，从缓存中进一步过滤
                    let filtered: Vec<Candidate> = cached
                        .iter()
                        .filter(|c| c.simplified.starts_with(&query))
                        .cloned()
                        .collect();

                    if !filtered.is_empty() {
                        // 如果结果太多，这里还是可以 truncate 到 internal_limit
                        let mut result = filtered;
                        result.truncate(internal_limit);
                        return result;
                    }
                }
            }
        }

        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        let build_hint = |tr: &TrieResult| -> Arc<str> {
            let mut hint = String::new();
            if config.appearance.show_english_aux && !tr.en.is_empty() {
                hint.push_str(tr.en);
            }
            if config.appearance.show_stroke_aux && !tr.stroke_aux.is_empty() {
                if !hint.is_empty() {
                    hint.push(' ');
                }
                hint.push_str(tr.stroke_aux);
            }
            if hint.is_empty() {
                Arc::from(tr.tone)
            } else {
                Arc::from(hint.as_str())
            }
        };

        // 1. 尝试全拼精确匹配（权重由 MatchLevelScoringFilter 统一评分）
        if let Some(exact_results) = self.trie.get_all_exact(&query) {
            for tr in exact_results {
                if seen.insert(tr.word) {
                    candidates.push(Candidate {
                        simplified: Arc::from(tr.word),
                        traditional: if tr.trad.is_empty() {
                            Arc::from(tr.word)
                        } else {
                            Arc::from(tr.trad)
                        },
                        text: Arc::from(tr.word),
                        hint: build_hint(&tr),
                        english_aux: Arc::from(tr.en),
                        stroke_aux: Arc::from(tr.stroke_aux),
                        source: Arc::from("Table (Exact)"),
                        weight: tr.weight as f64,
                        match_level: 3,
                    });
                }
            }
        }

        // 2. Fuzzy exact match: DFS + 前缀剪枝，避免笛卡尔积爆炸
        if config.input.enable_fuzzy_pinyin {
            let fuzzy_cfg = &config.input.fuzzy_config;
            let per_seg: Vec<Vec<String>> = segments
                .iter()
                .map(|seg| fuzzy_variants_per_segment(seg, fuzzy_cfg))
                .collect();

            let mut searcher = FuzzyPinyinSearcher {
                trie: &self.trie,
                per_seg: &per_seg,
                candidates: &mut candidates,
                seen: &mut seen,
                query: &query,
                build_hint: &build_hint,
                limit: internal_limit,
            };
            searcher.search();
        }

        let is_abbreviation =
            self.enable_abbreviation && segments.len() > 1 && segments.iter().any(|s| s.len() == 1);

        if is_abbreviation && config.input.enable_abbreviation_matching {
            let abbr_results =
                self.trie
                    .search_abbreviation(segments, &self.syllables, internal_limit);
            for ar in abbr_results {
                if seen.insert(ar.word) {
                    candidates.push(Candidate {
                        simplified: Arc::from(ar.word),
                        traditional: if ar.trad.is_empty() {
                            Arc::from(ar.word)
                        } else {
                            Arc::from(ar.trad)
                        },
                        text: Arc::from(ar.word),
                        hint: build_hint(&ar),
                        english_aux: Arc::from(ar.en),
                        stroke_aux: Arc::from(ar.stroke_aux),
                        source: Arc::from("Table (Abbr)"),
                        weight: ar.weight as f64,
                        match_level: 2,
                    });
                }
                if candidates.len() >= internal_limit {
                    break;
                }
            }
        } else {
            let results = self.trie.search_bfs(&query, internal_limit);
            for tr in results {
                if seen.insert(tr.word) {
                    candidates.push(Candidate {
                        simplified: Arc::from(tr.word),
                        traditional: if tr.trad.is_empty() {
                            Arc::from(tr.word)
                        } else {
                            Arc::from(tr.trad)
                        },
                        text: Arc::from(tr.word),
                        hint: build_hint(&tr),
                        english_aux: Arc::from(tr.en),
                        stroke_aux: Arc::from(tr.stroke_aux),
                        source: Arc::from("Table"),
                        weight: tr.weight as f64,
                        match_level: 1,
                    });
                }
                if candidates.len() >= internal_limit {
                    break;
                }
            }
        }

        // 更新缓存
        if let (Ok(mut last_q), Ok(mut cached)) =
            (self.last_query.write(), self.cached_candidates.write())
        {
            *last_q = (query, std::time::Instant::now());
            *cached = candidates.clone();
        }

        candidates
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// 用户词库翻译器 (仅处理用户自造词)
pub struct UserDictTranslator {
    pub user_dict: Arc<ArcSwap<UserDictData>>,
    pub profile: String,
}
impl Translator for UserDictTranslator {
    fn translate(
        &self,
        _input: &str,
        segments: &[String],
        _config: &Config,
        _limit: usize,
    ) -> Vec<Candidate> {
        let query = segments.join("");
        let mut results = Vec::new();
        let dict = self.user_dict.load();
        log::info!("UserDictTranslator: query={}, profile={}, dict_keys={:?}, has_profile={}", 
            query, self.profile, 
            dict.keys().collect::<Vec<_>>(),
            dict.contains_key(&self.profile));
        if let Some(profile_dict) = dict.get(&self.profile) {
            log::info!("UserDictTranslator: profile_dict keys={:?}, has_query={}",
                profile_dict.keys().collect::<Vec<_>>(),
                profile_dict.contains_key(&query));
            if let Some(words) = profile_dict.get(&query) {
                for (word, weight) in words {
                    results.push(Candidate {
                        text: Arc::from(word.as_str()),
                        simplified: Arc::from(word.as_str()),
                        traditional: Arc::from(word.as_str()),
                        hint: Arc::from("User"),
                        english_aux: Arc::from(""),
                        stroke_aux: Arc::from(""),
                        source: Arc::from("User"),
                        weight: *weight as f64,
                        match_level: 3,
                    });
                }
            }
        }
        results
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// 长句组合器：遍历所有可能的分割，逐段查最高频词，返回多个候选
pub struct ComposeTranslator {
    pub trie: Arc<Trie>,
    pub base_syllables: Arc<HashSet<String>>,
    pub syllable_freq: Arc<HashMap<String, u64>>,
}

impl ComposeTranslator {
    pub fn new(
        trie: Arc<Trie>,
        base_syllables: Arc<HashSet<String>>,
        syllable_freq: Arc<HashMap<String, u64>>,
    ) -> Self {
        Self { trie, base_syllables, syllable_freq }
    }

    /// 只用 base_syllables 做最长贪心匹配（第一遍，不做 DP 合并）
    fn segment_base(&self, input: &str) -> Vec<String> {
        let mut segs = Vec::new();
        let mut pos = 0;
        while pos < input.len() {
            let max_len = 12.min(input.len() - pos);
            let mut matched = false;
            for len in (1..=max_len).rev() {
                let end = pos + len;
                if input.is_char_boundary(end) {
                    let part = &input[pos..end];
                    if self.base_syllables.contains(part) {
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
    fn backtrack_partitions(
        &self,
        base: &[String],
        pos: usize,
        current: &mut Vec<(usize, usize)>,
        result: &mut Vec<Vec<(usize, usize)>>,
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
                self.backtrack_partitions(base, end, current, result);
                current.pop();
            } else {
                let merged: String = base[pos..end].concat();
                if self.trie.get_all_exact(&merged).is_some() {
                    current.push((pos, end));
                    self.backtrack_partitions(base, end, current, result);
                    current.pop();
                }
            }
        }
    }
}

impl Translator for ComposeTranslator {
    fn translate(
        &self,
        input: &str,
        _segments: &[String],
        _config: &Config,
        _limit: usize,
    ) -> Vec<Candidate> {
        // 第一遍：用 base_syllables 分词（不做 DP 合并）
        let base = self.segment_base(input);
        // 限制长度以避免回溯爆炸
        if base.len() < 2 || base.len() > 12 {
            return vec![];
        }

        // 生成所有合法分割
        let mut all_partitions = Vec::new();
        self.backtrack_partitions(&base, 0, &mut Vec::new(), &mut all_partitions);
        
        // 进一步限制分割数量
        if all_partitions.len() > 100 {
            all_partitions.truncate(100);
        }

        // 对每个分割，逐段取 trie 最高频词，计算 syllable_freq 总和
        let mut results: Vec<(String, usize, u64)> = Vec::new();
        for part in &all_partitions {
            let mut text = String::new();
            let mut total_freq = 0u64;
            let mut ok = true;

            for &(s, e) in part {
                let py: String = base[s..e].concat();
                if let Some(entries) = self.trie.get_all_exact(&py) {
                    if let Some(best) = entries.iter().max_by_key(|r| r.weight) {
                        text.push_str(best.word);
                        total_freq += self.syllable_freq.get(&py).copied().unwrap_or(0);
                        continue;
                    }
                }
                ok = false;
                break;
            }

            if ok {
                results.push((text, part.len(), total_freq));
            }
        }

        // 去重
        results.sort_by(|a, b| b.0.cmp(&a.0));
        results.dedup_by(|a, b| a.0 == b.0);

        // 排序：段数少优先 → freq 高优先
        results.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
        results.truncate(6);

        results
            .into_iter()
            .map(|(text, _, freq)| Candidate {
                text: Arc::from(text.clone()),
                simplified: Arc::from(text.clone()),
                traditional: Arc::from(text),
                hint: Arc::from(""),
                english_aux: Arc::from(""),
                stroke_aux: Arc::from(""),
                source: Arc::from("Compose"),
                weight: freq as f64 * 0.001 + 0.1,
                match_level: 0,
            })
            .collect()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// 匹配层级评分过滤器：统一 match_level 评分，替代 ChineseScheme::post_process 的独立评分
/// 排序规则：精确匹配 (level 3) > 模糊/简拼 (level 2) > 前缀 (level 1)，同层内按 weight 降序
pub struct MatchLevelScoringFilter;
impl Filter for MatchLevelScoringFilter {
    fn filter(
        &self,
        input: &str,
        mut candidates: Vec<Candidate>,
        config: &Config,
        _context: Option<&str>,
    ) -> Vec<Candidate> {
        let input_syllables = estimate_syllables(input);

        for c in &mut candidates {
            let base = match c.match_level {
                3 => 30_000_000.0 + config.input.ranking.exact_match_bonus,
                2 => 20_000_000.0,
                1 => 10_000_000.0,
                _ => 0.0,
            };
            let char_count = c.simplified.chars().count() as f64;
            let len_diff = (char_count - input_syllables as f64).max(0.0);
            let penalty = if c.match_level == 2 {
                len_diff * 10000.0
            } else {
                len_diff * 1000.0
            };
            c.weight = base + c.weight - penalty;
        }

        // 硬性分层排序：匹配层级优先，同层内按 weight 降序
        candidates.sort_by(|a, b| {
            b.match_level
                .cmp(&a.match_level)
                .then(b.weight.partial_cmp(&a.weight).unwrap_or(std::cmp::Ordering::Equal))
        });
        candidates
    }
}

fn estimate_syllables(input: &str) -> usize {
    if input.is_empty() {
        return 0;
    }
    input.chars().filter(|&c| c == ' ' || c == '\'' || c == ';').count() + 1
}

/// 繁简转换过滤器
pub struct TraditionalFilter;
impl Filter for TraditionalFilter {
    fn filter(
        &self,
        _input: &str,
        mut candidates: Vec<Candidate>,
        config: &Config,
        _context: Option<&str>,
    ) -> Vec<Candidate> {
        if config.input.enable_traditional {
            for c in &mut candidates {
                c.text = c.traditional.clone();
            }
        } else {
            for c in &mut candidates {
                c.text = c.simplified.clone();
            }
        }
        candidates
    }
}

/// 动态自适应过滤器 (调频与上下文联想)
pub struct AdaptiveFilter {
    pub usage_history: Arc<ArcSwap<UserDictData>>,
    pub ngram_history: Arc<ArcSwap<UserDictData>>,
    pub profile: String,
    last_input: std::sync::RwLock<Option<(String, std::time::Instant)>>,
    /// 缓存的 MRU 条目: word → (position, count)，O(1) 查找
    cached_usage_map: std::sync::RwLock<Option<std::collections::HashMap<String, (usize, u32)>>>,
    cached_ngram_map: std::sync::RwLock<Option<std::collections::HashMap<String, u32>>>,
}

impl AdaptiveFilter {
    pub fn new(
        usage_history: Arc<ArcSwap<UserDictData>>,
        ngram_history: Arc<ArcSwap<UserDictData>>,
        profile: String,
    ) -> Self {
        Self {
            usage_history,
            ngram_history,
            profile,
            last_input: std::sync::RwLock::new(None),
            cached_usage_map: std::sync::RwLock::new(None),
            cached_ngram_map: std::sync::RwLock::new(None),
        }
    }
}

impl Filter for AdaptiveFilter {
    fn filter(
        &self,
        input: &str,
        mut candidates: Vec<Candidate>,
        _config: &Config,
        context: Option<&str>,
    ) -> Vec<Candidate> {
        let usage_guard = self.usage_history.load();
        let ngram_guard = self.ngram_history.load();

        // === 用法历史加权（带 MRU 位置衰减 + 对数容量上限） ===
        if let Some(profile_usage) = usage_guard.get(&self.profile) {
            if let Some(entries) = profile_usage.get(input) {
                // 检查缓存是否有效
                let use_cached = {
                    if let Ok(guard) = self.last_input.read() {
                        if let Some((ref last_input, ref time)) = *guard {
                            *last_input == input && time.elapsed().as_millis() < 100
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if use_cached {
                    if let Ok(guard) = self.cached_usage_map.read() {
                        if let Some(ref cache_map) = *guard {
                            for c in &mut candidates {
                                if let Some(&(pos, count)) = cache_map.get(c.simplified.as_ref()) {
                                    c.weight += compute_decay_boost(pos, count);
                                }
                            }
                        }
                    }
                } else {
                    // 构建 HashMap: word → (position, count)，O(1) 查找
                    let usage_map: std::collections::HashMap<String, (usize, u32)> = entries
                        .iter()
                        .enumerate()
                        .map(|(pos, (w, c))| (w.clone(), (pos, *c)))
                        .collect();

                    for c in &mut candidates {
                        if let Some(&(pos, count)) = usage_map.get(c.simplified.as_ref()) {
                            c.weight += compute_decay_boost(pos, count);
                        }
                    }

                    // 更新缓存
                    if let Ok(mut guard) = self.cached_usage_map.write() {
                        *guard = Some(usage_map);
                    }
                    if let Ok(mut guard) = self.last_input.write() {
                        *guard = Some((input.to_string(), std::time::Instant::now()));
                    }
                }
            }
        }

        // === 上下文联想 (N-Gram) 加权（对数缩放） ===
        if let Some(ctx) = context {
            if let Some(profile_ngram) = ngram_guard.get(&self.profile) {
                if let Some(entries) = profile_ngram.get(ctx) {
                    let ngram_map: std::collections::HashMap<String, u32> =
                        entries.iter().map(|(w, c)| (w.clone(), *c)).collect();
                    for c in &mut candidates {
                        if let Some(&count) = ngram_map.get(c.simplified.as_ref()) {
                            let effective = count.min(10);
                            let boost =
                                (1.0 + (effective as f64).ln()).max(0.0) * NGRAM_BOOST_SCALE;
                            c.weight += boost.min(MAX_NGRAM_BOOST);
                        }
                    }

                    if let Ok(mut guard) = self.cached_ngram_map.write() {
                        *guard = Some(ngram_map);
                    }
                }
            }
        }

        // 再次根据新权重排序
        candidates.sort_by(|a, b| {
            b.weight
                .partial_cmp(&a.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates
    }
}

/// 衰减算法：位置越靠后（越久没用），加成越小；次数用对数缩放，避免旧使用无限叠加
pub(crate) fn compute_decay_boost(pos: usize, count: u32) -> f64 {
    // 位置衰减：最近（pos=0）拿满，越远衰减越慢（sqrt）
    let recency = RECENCY_BOOST_BASE / (1.0 + (pos as f64).sqrt());
    // 频率衰减：对数缩放，次数越多收益越小，上限 20 次
    let effective = count.min(20);
    let freq = (1.0 + (effective as f64).ln()).max(0.0) * FREQ_BOOST_SCALE;
    // 总加成上限，防止单个词永久霸榜
    (recency + freq).min(MAX_USAGE_BOOST)
}

/// 核心管道定义
pub struct Pipeline {
    pub segmentor: Box<dyn Segmentor>,
    pub translators: Vec<Box<dyn Translator>>,
    pub filters: Vec<Box<dyn Filter>>,
    pub syllable_freq: Arc<HashMap<String, u64>>,
    pub base_syllables: Arc<HashSet<String>>,
    segment_cache: std::sync::RwLock<std::collections::HashMap<String, Vec<String>>>,
}

impl Pipeline {
    pub fn new(segmentor: Box<dyn Segmentor>) -> Self {
        Self {
            segmentor,
            translators: Vec::new(),
            filters: Vec::new(),
            syllable_freq: Arc::new(HashMap::new()),
            base_syllables: Arc::new(HashSet::new()),
            segment_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub fn add_translator(&mut self, t: Box<dyn Translator>) {
        self.translators.push(t);
    }

    pub fn add_filter(&mut self, f: Box<dyn Filter>) {
        self.filters.push(f);
    }

    #[inline]
    pub fn run(
        &self,
        input: &str,
        syllables: &HashSet<String>,
        config: &Config,
        limit: usize,
        context: Option<&str>,
    ) -> Vec<Candidate> {
        // 使用缓存的分段结果
        let segments = {
            if let Ok(guard) = self.segment_cache.read() {
                if let Some(cached) = guard.get(input) {
                    cached.clone()
                } else {
                    drop(guard);
                    let segments = self.segmentor.segment(input, syllables, &config.input.segmentation_delimiters, &self.syllable_freq, &self.base_syllables);
                    if let Ok(mut guard) = self.segment_cache.write() {
                        if guard.len() < 100 {
                            guard.insert(input.to_string(), segments.clone());
                        }
                    }
                    segments
                }
            } else {
                self.segmentor.segment(input, syllables, &config.input.segmentation_delimiters, &self.syllable_freq, &self.base_syllables)
            }
        };

        let mut candidates = Vec::new();
        for t in &self.translators {
            candidates.extend(t.translate(input, &segments, config, limit));
        }

        // Dedup by text: keep the first occurrence (dictionary entries from
        // UserDict/Table translators come before Compose/auto-sentence entries)
        {
            let mut seen = std::collections::HashSet::new();
            candidates.retain(|c| seen.insert(c.text.clone()));
        }
        candidates.truncate(200);
        for f in &self.filters {
            candidates = f.filter(input, candidates, config, context);
        }
        candidates
    }
}

type PipelineCache = (HashMap<String, Arc<Pipeline>>, Vec<String>);

/// 搜索引擎：协调所有的 Pipeline
#[derive(Clone)]
pub struct SearchEngine {
    pub trie_paths: HashMap<String, (PathBuf, PathBuf)>,
    pub syllables: Arc<HashSet<String>>,
    pub syllable_freq: Arc<HashMap<String, u64>>,
    pub base_syllables: Arc<HashSet<String>>,
    pub(crate) learned_words: Arc<ArcSwap<UserDictData>>,
    pub(crate) usage_history: Arc<ArcSwap<UserDictData>>,
    pub(crate) ngram_history: Arc<ArcSwap<UserDictData>>,
    pub schemes: Arc<HashMap<String, Box<dyn crate::scheme::InputScheme>>>,
    pipelines: Arc<RwLock<PipelineCache>>,
}

const MAX_CACHED_PIPELINES: usize = 10;

pub struct SearchQuery<'a> {
    pub buffer: &'a str,
    pub profile: &'a str,
    pub syllables: &'a HashSet<String>,
    pub config: &'a Config,
    pub limit: usize,
    pub filter_mode: crate::processor::FilterMode,
    pub aux_filter: &'a str,
    pub context: Option<&'a str>,
    pub fuzzy_enabled: bool,
}

impl SearchEngine {
    pub fn new(
        trie_paths: HashMap<String, (PathBuf, PathBuf)>,
        syllables: Arc<HashSet<String>>,
        syllable_freq: Arc<HashMap<String, u64>>,
        learned_words: Arc<ArcSwap<UserDictData>>,
        usage_history: Arc<ArcSwap<UserDictData>>,
        ngram_history: Arc<ArcSwap<UserDictData>>,
        schemes: Arc<HashMap<String, Box<dyn crate::scheme::InputScheme>>>,
    ) -> Self {
        let base_syllables: HashSet<String> = syllables.iter()
            .filter(|s| !syllable_freq.contains_key(*s))
            .cloned()
            .collect();
        Self {
            trie_paths,
            syllables,
            syllable_freq,
            base_syllables: Arc::new(base_syllables),
            learned_words,
            usage_history,
            ngram_history,
            schemes,
            pipelines: Arc::new(RwLock::new((HashMap::new(), Vec::new()))),
        }
    }

    pub fn search(&self, query: SearchQuery) -> (Vec<Candidate>, Vec<String>) {
        self.do_search(query)
    }

    fn do_search(&self, query: SearchQuery) -> (Vec<Candidate>, Vec<String>) {
        log::info!("engine_search: profile={}, buffer={}, fuzzy_enabled={}", query.profile, query.buffer, query.fuzzy_enabled);

        // 根据 fuzzy_enabled 决定是否启用模糊音
        let config_ref;
        let mut cloned_config;
        if query.fuzzy_enabled {
            config_ref = query.config;
        } else if query.config.input.enable_fuzzy_pinyin {
            cloned_config = query.config.clone();
            cloned_config.input.enable_fuzzy_pinyin = false;
            config_ref = &cloned_config;
        } else {
            config_ref = query.config;
        }

        // 优先走方案路径（ChineseScheme 等语言特定逻辑）
        if let Some(scheme) = self.schemes.get(query.profile) {
            let mut tries_map = HashMap::new();
            if let Some(pipeline) = self.get_or_create_pipeline(query.profile) {
                if let Some(trie) = self.get_trie_from_pipeline(pipeline.as_ref()) {
                    tries_map.insert(query.profile.to_string(), trie.clone());
                }
            }
            let context = crate::scheme::SchemeContext {
                config: config_ref,
                tries: &tries_map,
                syllables: query.syllables,
                syllable_freq: &self.syllable_freq,
                base_syllables: &self.base_syllables,
                user_dict: &self.learned_words,
                usage_history: &self.usage_history,
                ngram_history: &self.ngram_history,
                active_profiles: &[query.profile.to_string()],
                candidate_count: 0,
                last_word: query.context,
                _filter_mode: query.filter_mode.clone(),
                _aux_filter: query.aux_filter,
            };

            let pre_processed = scheme.pre_process(query.buffer, &context);
            let mut scheme_candidates = scheme.lookup(&pre_processed, &context);
            scheme.post_process(&pre_processed, &mut scheme_candidates, &context);

            let mut results = Vec::new();
            for sc in scheme_candidates {
                let hint = {
                    let mut h = String::new();
                    if config_ref.appearance.show_english_aux && !sc.english.is_empty() {
                        h.push_str(&sc.english);
                    }
                    if config_ref.appearance.show_stroke_aux && !sc.stroke_aux.is_empty() {
                        if !h.is_empty() { h.push(' '); }
                        h.push_str(&sc.stroke_aux);
                    }
                    if h.is_empty() { Arc::from(sc.tone.as_str()) } else { Arc::from(h) }
                };
                results.push(Candidate {
                    text: if config_ref.input.enable_traditional {
                        Arc::from(sc.traditional.as_str())
                    } else {
                        Arc::from(sc.simplified.as_str())
                    },
                    simplified: Arc::from(sc.simplified.as_str()),
                    traditional: Arc::from(sc.traditional.as_str()),
                    hint,
                    english_aux: Arc::from(sc.english.as_str()),
                    stroke_aux: Arc::from(sc.stroke_aux.as_str()),
                    source: Arc::from("Engine"),
                    weight: sc.weight as f64,
                    match_level: sc.match_level,
                });
            }

            // Global 模式辅码过滤
            if query.filter_mode == crate::processor::FilterMode::Global
                && !query.aux_filter.is_empty()
            {
                results.retain(|c| self.matches_filter(c, query.aux_filter, config_ref.input.english_aux_mode));
            }

            results.truncate(query.limit);
            return (results, vec![]);
        }

        // 没有方案时回退到旧的 pipeline 路径
        if let Some(pipeline) = self.get_or_create_pipeline(query.profile) {
            let search_limit = if !query.aux_filter.is_empty() {
                MAX_LOOKUP_LIMIT
            } else {
                query.limit
            };

            let results = pipeline.run(
                query.buffer,
                query.syllables,
                config_ref,
                search_limit,
                query.context,
            );
            let segments = pipeline.segmentor.segment(query.buffer, query.syllables, &config_ref.input.segmentation_delimiters, &pipeline.syllable_freq, &pipeline.base_syllables);

            let mut final_results = results;
            if query.filter_mode == crate::processor::FilterMode::Global
                && !query.aux_filter.is_empty()
            {
                final_results.retain(|c| self.matches_filter(c, query.aux_filter, config_ref.input.english_aux_mode));
            }
            
            final_results.truncate(query.limit);
            return (final_results, segments);
        }

        (vec![], vec![])
    }

    #[inline]
    pub fn has_exact_match(&self, profile: &str, pinyin: &str, word: &str) -> bool {
        if let Some(pipeline) = self.get_or_create_pipeline(profile) {
            if let Some(trie) = self.get_trie_from_pipeline(pipeline.as_ref()) {
                if let Some(exacts) = trie.get_all_exact(pinyin) {
                    return exacts.iter().any(|tr| tr.word == word);
                }
            }
        }
        false
    }

    pub fn get_trie_from_pipeline<'a>(&self, pipeline: &'a Pipeline) -> Option<&'a Trie> {
        for t in &pipeline.translators {
            if let Some(table) = t.as_any().downcast_ref::<TableTranslator>() {
                return Some(&table.trie);
            }
        }
        None
    }

    pub fn get_or_create_pipeline(&self, profile: &str) -> Option<Arc<Pipeline>> {
        // 1. 快速路径：读锁查询
        if let Ok(cache) = self.pipelines.read() {
            let (p_map, _access_order) = &*cache;
            if let Some(p) = p_map.get(profile) {
                let result = Some(p.clone());
                drop(cache);
                // 写锁仅在更新访问顺序时短暂持有
                if let Ok(mut cache) = self.pipelines.write() {
                    let (_, access_order) = &mut *cache;
                    if let Some(pos) = access_order.iter().position(|p| p == profile) {
                        access_order.remove(pos);
                    }
                    access_order.push(profile.to_string());
                }
                return result;
            }
        }

        // 2. 如果不存在，尝试创建
        let paths = self.trie_paths.get(profile)?;
        log::info!("Lazy loading dictionary: profile={}", profile);
        let trie = Trie::load(&paths.0, &paths.1, true).ok()?;
        let trie_arc = Arc::new(trie);

        let mut pipeline = Pipeline::new(Box::new(DefaultSegmentor));
        pipeline.syllable_freq = self.syllable_freq.clone();
        pipeline.base_syllables = self.base_syllables.clone();
        pipeline.add_translator(Box::new(UserDictTranslator {
            user_dict: self.learned_words.clone(),
            profile: profile.to_string(),
        }));
        pipeline.add_translator(Box::new(TableTranslator::new(
            trie_arc.clone(),
            self.syllables.clone(),
            profile == "chinese",
        )));
        pipeline.add_translator(Box::new(ComposeTranslator::new(
            trie_arc.clone(),
            self.base_syllables.clone(),
            self.syllable_freq.clone(),
        )));
        pipeline.add_filter(Box::new(MatchLevelScoringFilter));
        pipeline.add_filter(Box::new(AdaptiveFilter::new(
            self.usage_history.clone(),
            self.ngram_history.clone(),
            profile.to_string(),
        )));
        pipeline.add_filter(Box::new(TraditionalFilter));

        let arc_p = Arc::new(pipeline);

        // 在写锁内再次检查（防止另一个线程在我们构建 pipeline 时已插入）
        {
            let mut cache = self.pipelines.write().ok()?;
            let (p_map, access_order) = &mut *cache;
            if let Some(p) = p_map.get(profile) {
                return Some(p.clone());
            }
            if access_order.len() >= MAX_CACHED_PIPELINES {
                if let Some(oldest) = access_order.first().cloned() {
                    p_map.remove(&oldest);
                    access_order.remove(0);
                    log::debug!("Evicted pipeline from cache: profile={}", oldest);
                }
            }
            p_map.insert(profile.to_string(), arc_p.clone());
            access_order.push(profile.to_string());
        }

        Some(arc_p)
    }

    #[inline]
    pub fn has_longer_match(&self, profile: &str, buffer: &str) -> bool {
        if let Some(pipeline) = self.get_or_create_pipeline(profile) {
            if let Some(trie) = self.get_trie_from_pipeline(pipeline.as_ref()) {
                return trie.has_longer_match(buffer);
            }
        }
        false
    }

    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.pipelines.write() {
            cache.0.clear();
            cache.1.clear();
        }
    }

    /// 预加载并初始化指定方案的 Pipeline
    pub fn prewarm_profile(&self, profile: &str) {
        log::info!("prewarm_profile: profile={}", profile);

        // 直接调用 get_or_create_pipeline，这将触发完整的加载和缓存流程
        if let Some(_pipeline) = self.get_or_create_pipeline(profile) {
            log::info!("Pipeline eagerly initialized and cached: profile={}", profile);
            // 顺便触发一次内部 trie 的预热（如果是 Mmap 模式）
            // 虽然目前默认是全内存加载，但保留此逻辑以增强兼容性
            if let Some(paths) = self.trie_paths.get(profile) {
                if let Ok(trie) = Trie::load(&paths.0, &paths.1, true) {
                    trie.prewarm(PREWARM_ENTRIES);
                }
            }
        }
    }

    #[inline]
    pub fn matches_filter(&self, candidate: &Candidate, filter: &str, mode: qianyan_ime_core::config::EnglishAuxMode) -> bool {
        if filter.is_empty() {
            return true;
        }
        let filter_lower = filter.to_lowercase();

        // 1. 优先检查 English Aux (支持多含义分割)
        if !candidate.english_aux.is_empty() {
            let en_lower = candidate.english_aux.to_lowercase();
            // 扩充符号集，支持更多类型的词典分隔符
            let parts: Vec<&str> = en_lower.split([' ', '/', '(', ')', ',', ';', '|', '.', ':', '!', '?', '[', ']', '{', '}']).collect();
            
            if mode == qianyan_ime_core::config::EnglishAuxMode::FirstLetter {
                // 仅首字母模式：匹配每个单词的首字母
                if parts.iter().any(|p| p.starts_with(&filter_lower)) {
                    return true;
                }
            } else {
                // 前缀匹配模式
                if parts.iter().any(|p| p.starts_with(&filter_lower)) || en_lower.starts_with(&filter_lower) {
                    return true;
                }
            }
        }

        // 2. 检查 Stroke Aux (始终使用前缀匹配)
        if !candidate.stroke_aux.is_empty() {
            if candidate.stroke_aux.to_lowercase().starts_with(&filter_lower) {
                return true;
            }
        }

        // 3. 后备：检查 Hint (兼容性)
        let hint_lower = candidate.hint.to_lowercase();
        let parts: Vec<&str> = hint_lower.split([' ', '/', '(', ')', ',', ';', '|', '.']).collect();
        parts.iter().any(|p| p.starts_with(&filter_lower)) || hint_lower.starts_with(&filter_lower)
    }
}

pub fn lookup(ctx: &mut EngineContext) -> Option<Action> {
    use crate::processor::FilterMode;
    use std::sync::Arc;

    if ctx.session.buffer.is_empty() {
        ctx.reset();
        return None;
    }

    if ctx.session.filter_mode == FilterMode::Page && !ctx.session.page_snapshot.is_empty() {
        let mut filtered = Vec::new();
        for c in &ctx.session.page_snapshot {
            if ctx.engine.matches_filter(c, &ctx.session.aux_filter, ctx.config.master_config.input.english_aux_mode) {
                filtered.push(c.clone());
            }
        }
        if !filtered.is_empty() {
            ctx.session.candidates = filtered;
            if ctx.session.candidates.len() == 1 {
                let word = ctx.session.candidates[0].text.clone();
                return Some(crate::processor::commands::commit_candidate(ctx, word, 0));
            }
        } else {
            ctx.session.candidates.clear();
        }
        ctx.session.update_state();
        return None;
    }

    let current_profile = ctx
        .session_state
        .active_profiles
        .first()
        .cloned()
        .unwrap_or_default();
    let last_word = ctx
        .session_state
        .commit_history
        .last()
        .map(|(_, word)| word.as_str());

    let fuzzy_enabled = ctx.session.fuzzy_activated;
    let query = SearchQuery {
        buffer: &ctx.session.buffer,
        profile: &current_profile,
        syllables: &ctx.syllables,
        config: &ctx.config.master_config,
        limit: 20,
        filter_mode: ctx.session.filter_mode.clone(),
        aux_filter: &ctx.session.aux_filter,
        context: last_word,
        fuzzy_enabled,
    };
    let (results, segments) = ctx.engine.search(query);
    ctx.session.candidates = results;
    ctx.session.best_segmentation = segments;
    ctx.session.has_dict_match = !ctx.session.candidates.is_empty();
    ctx.session.last_lookup_pinyin = ctx.session.buffer.clone();

    // 智能辅码：当输入为「完整拼音 + 辅码字母」时自动进入辅码过滤
    if ctx.config.master_config.input.enable_smart_aux
        && ctx.session.filter_mode == FilterMode::None
    {
        let buffer = &ctx.session.buffer;
        if let Some((pinyin_base, aux_chars)) = detect_smart_aux(buffer, &ctx.syllables, ctx.config.master_config.input.smart_aux_mode) {
            let aux_query = SearchQuery {
                buffer: &pinyin_base,
                profile: &current_profile,
                syllables: &ctx.syllables,
                config: &ctx.config.master_config,
                limit: 20,
                filter_mode: FilterMode::Global,
                aux_filter: &aux_chars,
                context: last_word,
                fuzzy_enabled,
            };
            let (aux_results, _) = ctx.engine.search(aux_query);
            if !aux_results.is_empty() {
                let mut merged = aux_results;
                for c in &ctx.session.candidates {
                    if !merged.iter().any(|r| r.text == c.text) {
                        merged.push(c.clone());
                    }
                }
                ctx.session.candidates = merged;
                ctx.session.has_dict_match = true;
            }
        }
    }

    if ctx.session.candidates.len() == 1 && ctx.session.filter_mode == FilterMode::Global {
        let word = ctx.session.candidates[0].text.clone();
        return Some(crate::processor::commands::commit_candidate(ctx, word, 0));
    }

    if ctx.session.candidates.is_empty() {
        let buf_arc: Arc<str> = Arc::from(ctx.session.buffer.as_str());
        ctx.session.candidates.push(Candidate {
            text: buf_arc.clone(),
            simplified: buf_arc.clone(),
            traditional: buf_arc.clone(),
            hint: Arc::from(""),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from("Raw"),
            weight: 0.0,
            match_level: 0,
        });
    }
    ctx.session.update_state();
    crate::compositor::Compositor::check_auto_commit(ctx)
}

/// 检测「完整拼音 + 辅码字母」模式。
/// 如果 buffer 的最长有效拼音前缀之后有额外字母，且整体不是有效拼音，则返回 (拼音前缀, 辅码后缀)。
pub fn detect_smart_aux(buffer: &str, syllables: &HashSet<String>, mode: qianyan_ime_core::config::SmartAuxMode) -> Option<(String, String)> {
    if buffer.len() < 3 {
        return None;
    }
    let bytes = buffer.as_bytes();
    if !bytes.iter().all(|b| b.is_ascii_lowercase()) {
        return None;
    }

    // 如果整体就是一个有效音节或可以被完整切分为音节，不要触发智能辅码
    if syllables.contains(buffer) || is_fully_syllabic(buffer, syllables) {
        return None;
    }

    // 根据模式决定切分策略
    let split_points: Vec<usize> = match mode {
        qianyan_ime_core::config::SmartAuxMode::Greedy => (1..buffer.len()).rev().collect(),
        qianyan_ime_core::config::SmartAuxMode::Minimal => (1..buffer.len()).collect(),
    };

    for split in split_points {
        let prefix = &buffer[..split];
        let suffix = &buffer[split..];

        // 检查 prefix 是否可以被完整切分
        if is_fully_syllabic(prefix, syllables) {
            return Some((prefix.to_string(), suffix.to_string()));
        }
    }

    None
}

/// 检查字符串是否由一个或多个有效音节组成（无剩余字符）
fn is_fully_syllabic(s: &str, syllables: &HashSet<String>) -> bool {
    if s.is_empty() {
        return true;
    }
    is_fully_syllabic_depth(s, syllables, 0)
}

fn is_fully_syllabic_depth(s: &str, syllables: &HashSet<String>, depth: usize) -> bool {
    if depth > 30 {
        return false;
    }
    if s.is_empty() {
        return true;
    }
    for len in (1..=s.len()).rev() {
        if syllables.contains(&s[..len]) {
            if is_fully_syllabic_depth(&s[len..], syllables, depth + 1) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn test_detect_smart_aux_logic() {
        let syllables: HashSet<String> = ["ni", "hao", "wo", "hen"].iter().map(|s| s.to_string()).collect();
        let mode = qianyan_ime_core::config::SmartAuxMode::Greedy;
        
        // 单个音节 + 辅码
        assert_eq!(detect_smart_aux("nih", &syllables, mode), Some(("ni".to_string(), "h".to_string())));
        
        // 多个音节 + 辅码
        assert_eq!(detect_smart_aux("nihaoh", &syllables, mode), Some(("nihao".to_string(), "h".to_string())));
        
        // 整体是音节，不应触发
        assert_eq!(detect_smart_aux("nihao", &syllables, mode), None);
        
        // 无效音节序列
        assert_eq!(detect_smart_aux("xyz", &syllables, mode), None);
    }

    #[test]
    fn test_detect_smart_aux_modes() {
        let syllables: HashSet<String> = ["na", "nan", "hai", "nanhai", "ha"].iter().map(|s| s.to_string()).collect();
        
        // Greedy 模式：取最长拼音
        let greedy = qianyan_ime_core::config::SmartAuxMode::Greedy;
        assert_eq!(detect_smart_aux("nanhaix", &syllables, greedy), Some(("nanhai".to_string(), "x".to_string())));
        
        // Minimal 模式：取最短拼音
        let minimal = qianyan_ime_core::config::SmartAuxMode::Minimal;
        assert_eq!(detect_smart_aux("nanhaix", &syllables, minimal), Some(("na".to_string(), "nhaix".to_string())));
    }

    #[test]
    fn test_default_segmentor_basic() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["ni", "hao", "zhong", "guo"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let result = segmentor.segment("nihao", &syllables, "", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["ni", "hao"]);
    }

    #[test]
    fn test_default_segmentor_longer_match() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["zhong", "guo", "zhongguo"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let result = segmentor.segment("zhongguo", &syllables, "", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["zhongguo"]);
    }

    #[test]
    fn test_default_segmentor_partial_match() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["zhong", "guo"].iter().map(|s| s.to_string()).collect();

        let result = segmentor.segment("zhongguo", &syllables, "", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["zhong", "guo"]);
    }

    #[test]
    fn test_default_segmentor_unknown_chars() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["ni"].iter().map(|s| s.to_string()).collect();

        let result = segmentor.segment("nixyz", &syllables, "", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["ni", "x", "y", "z"]);
    }

    #[test]
    fn test_default_segmentor_empty_input() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["ni", "hao"].iter().map(|s| s.to_string()).collect();

        let result = segmentor.segment("", &syllables, "", &HashMap::new(), &syllables);
        assert!(result.is_empty());
    }

    #[test]
    fn test_default_segmentor_delimiter_apostrophe() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["xi", "an"].iter().map(|s| s.to_string()).collect();
        let result = segmentor.segment("xi'an", &syllables, "'", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["xi", "an"]);
    }

    #[test]
    fn test_default_segmentor_delimiter_semicolon() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["ni", "hao"].iter().map(|s| s.to_string()).collect();
        let result = segmentor.segment("ni;hao", &syllables, ";", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["ni", "hao"]);
    }

    #[test]
    fn test_default_segmentor_delimiter_edge_cases() {
        let segmentor = DefaultSegmentor;
        let syllables: HashSet<String> = ["ti"].iter().map(|s| s.to_string()).collect();
        // delimiter at end: skipped
        let result = segmentor.segment("ti'", &syllables, "'", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["ti"]);
        // delimiter at start: skipped
        let result = segmentor.segment("'ti", &syllables, "'", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["ti"]);
        // empty delimiters: no change, individual chars (no "xi" in syllables)
        let result = segmentor.segment("xi'an", &syllables, "", &HashMap::new(), &syllables);
        assert_eq!(result, vec!["x", "i", "'", "a", "n"]);
    }

    #[test]
    fn test_default_segmentor_two_pass_merge() {
        let segmentor = DefaultSegmentor;
        // 模拟真实场景：基本音节（不在 freq 表中）+ 复合词（在 freq 表中）
        let all: HashSet<String> = ["fan", "gan", "fang", "an", "fangan"]
            .iter().map(|s| s.to_string()).collect();
        let base: HashSet<String> = ["fan", "gan", "fang", "an"]
            .iter().map(|s| s.to_string()).collect();
        let mut freqs = HashMap::new();
        freqs.insert("fangan".to_string(), 1);

        // "fangan" → first pass: "fang"+"an", second pass: merge to "fangan"
        let result = segmentor.segment("fangan", &all, "", &freqs, &base);
        assert_eq!(result, vec!["fangan"]);

        // 无 freq 时不合并；两条路径 "fan"+"gan" 和 "fang"+"an" 均有效
        let result2 = segmentor.segment("fangan", &all, "", &HashMap::new(), &base);
        assert!(result2 == vec!["fan", "gan"] || result2 == vec!["fang", "an"],
            "expected either fan+gan or fang+an, got {:?}", result2);
    }

    #[test]
    fn test_default_segmentor_wowangjile() {
        let segmentor = DefaultSegmentor;
        // 关键测试：wowangjile 不应出现 "wan g" 分割
        let all: HashSet<String> = ["wo", "wang", "wan", "ji", "le", "wowang", "wowan", "wangji", "jile", "g"]
            .iter().map(|s| s.to_string()).collect();
        let base: HashSet<String> = ["wo", "wang", "wan", "ji", "le", "g"]
            .iter().map(|s| s.to_string()).collect();
        let mut freqs = HashMap::new();
        freqs.insert("wowang".to_string(), 14628);
        freqs.insert("wowan".to_string(), 22290);
        freqs.insert("wangji".to_string(), 482559);
        freqs.insert("jile".to_string(), 11073);

        // DP should pick "wo"+"wangji"+"le" (highest total freq = 482559)
        let result = segmentor.segment("wowangjile", &all, "", &freqs, &base);
        assert_eq!(result, vec!["wo", "wangji", "le"]);
    }

    #[test]
    fn test_default_segmentor_mana_ambiguity() {
        let segmentor = DefaultSegmentor;
        // "mana" 即可以是 "man a" 也可以是 "ma na"
        // Viterbi DP 应选择 "ma na"（优先较短音节路径）
        let all: HashSet<String> = ["ma", "man", "na", "a"]
            .iter().map(|s| s.to_string()).collect();
        let result = segmentor.segment("mana", &all, "", &HashMap::new(), &all);
        assert_eq!(result, vec!["ma", "na"]);
    }

    #[test]
    fn test_default_segmentor_transpose_guna() {
        let segmentor = DefaultSegmentor;
        // "guna" → 换位纠错 -> "guan" ; 比不纠错 (gu+na, 2段) 段数少
        let all: HashSet<String> = ["gu", "na", "guan"]
            .iter().map(|s| s.to_string()).collect();
        let mut freqs = HashMap::new();
        freqs.insert("guan".to_string(), 100);
        let result = segmentor.segment("guna", &all, "", &freqs, &all);
        assert_eq!(result, vec!["guan"]);
    }

    #[test]
    fn test_default_segmentor_transpose_guagn() {
        let segmentor = DefaultSegmentor;
        // "guagn" → 换位纠错 -> "guang"
        let all: HashSet<String> = ["guang"]
            .iter().map(|s| s.to_string()).collect();
        let mut freqs = HashMap::new();
        freqs.insert("guang".to_string(), 200);
        let result = segmentor.segment("guagn", &all, "", &freqs, &all);
        assert_eq!(result, vec!["guang"]);
    }

    #[test]
    fn test_default_segmentor_transpose_correct_input_untouched() {
        let segmentor = DefaultSegmentor;
        // 正确输入 "guan" 直接匹配，不触发换位
        let all: HashSet<String> = ["guan", "gu", "an"]
            .iter().map(|s| s.to_string()).collect();
        let result = segmentor.segment("guan", &all, "", &HashMap::new(), &all);
        assert_eq!(result, vec!["guan"]);
    }

    #[test]
    fn test_default_segmentor_transpose_no_false_positive() {
        let segmentor = DefaultSegmentor;
        // "mana" 不应被换位影响（ma 和 man 都在，且不是换位场景）
        let all: HashSet<String> = ["ma", "man", "na", "a"]
            .iter().map(|s| s.to_string()).collect();
        let result = segmentor.segment("mana", &all, "", &HashMap::new(), &all);
        assert_eq!(result, vec!["ma", "na"]);
    }

    #[test]
    fn test_candidate_clone() {
        let candidate = Candidate {
            text: Arc::from("test"),
            simplified: Arc::from("test"),
            traditional: Arc::from("test"),
            hint: Arc::from("hint"),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from("test"),
            weight: 1.0,
            match_level: 3,
        };

        let cloned = candidate.clone();
        assert_eq!(candidate.text, cloned.text);
        assert_eq!(candidate.weight, cloned.weight);
    }

    #[test]
    fn test_match_level_scoring_filter() {
        let filter = MatchLevelScoringFilter;
        let candidates = vec![
            Candidate {
                text: Arc::from("prefix"),
                simplified: Arc::from("prefix"),
                traditional: Arc::from("prefix"),
                hint: Arc::from(""),
                english_aux: Arc::from(""),
                stroke_aux: Arc::from(""),
                source: Arc::from(""),
                weight: 5000.0,
                match_level: 1,
            },
            Candidate {
                text: Arc::from("exact"),
                simplified: Arc::from("exact"),
                traditional: Arc::from("exact"),
                hint: Arc::from(""),
                english_aux: Arc::from(""),
                stroke_aux: Arc::from(""),
                source: Arc::from(""),
                weight: 100.0,
                match_level: 3,
            },
            Candidate {
                text: Arc::from("fuzzy"),
                simplified: Arc::from("fuzzy"),
                traditional: Arc::from("fuzzy"),
                hint: Arc::from(""),
                english_aux: Arc::from(""),
                stroke_aux: Arc::from(""),
                source: Arc::from(""),
                weight: 3000.0,
                match_level: 2,
            },
        ];

        let mut config = Config::default_config();
        config.input.ranking.exact_match_bonus = 10_000_000.0;
        // "test" → 1 syllable estimate
        let result = filter.filter("test", candidates, &config, None);
        // exact (30M+10M+100 ≈ 40,000,100) > fuzzy (20M+3000 ≈ 20,003,000) > prefix (10M+5000 ≈ 10,005,000)
        assert_eq!(result[0].text.as_ref(), "exact");
        assert_eq!(result[1].text.as_ref(), "fuzzy");
        assert_eq!(result[2].text.as_ref(), "prefix");
    }

    #[test]
    fn test_traditional_filter_simplified() {
        let filter = TraditionalFilter;
        let candidates = vec![Candidate {
            text: Arc::from("简化"),
            simplified: Arc::from("简化"),
            traditional: Arc::from("簡化"),
            hint: Arc::from(""),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from(""),
            weight: 1.0,
            match_level: 1,
        }];

        let config = Config::default_config();
        let result = filter.filter("test", candidates, &config, None);
        assert_eq!(result[0].text.as_ref(), "简化");
    }

    #[test]
    fn test_traditional_filter_traditional() {
        let filter = TraditionalFilter;
        let mut config = Config::default_config();
        config.input.enable_traditional = true;

        let candidates = vec![Candidate {
            text: Arc::from("简化"),
            simplified: Arc::from("简化"),
            traditional: Arc::from("簡化"),
            hint: Arc::from(""),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from(""),
            weight: 1.0,
            match_level: 1,
        }];

        let result = filter.filter("test", candidates, &config, None);
        assert_eq!(result[0].text.as_ref(), "簡化");
    }

    #[test]
    fn test_matches_filter_comprehensive() {
        let engine = create_test_engine();
        let candidate = Candidate {
            text: Arc::from("你好"),
            simplified: Arc::from("你好"),
            traditional: Arc::from("你好"),
            hint: Arc::from("nihao"),
            english_aux: Arc::from("Hello/Hi"),
            stroke_aux: Arc::from("HSP"),
            source: Arc::from(""),
            weight: 1.0,
            match_level: 1,
        };

        let mode = qianyan_ime_core::config::EnglishAuxMode::Prefix;

        // 匹配 English Aux
        assert!(engine.matches_filter(&candidate, "h", mode));
        assert!(engine.matches_filter(&candidate, "he", mode));
        assert!(engine.matches_filter(&candidate, "hello", mode));
        assert!(engine.matches_filter(&candidate, "hi", mode));
        
        // 匹配 Stroke Aux
        assert!(engine.matches_filter(&candidate, "hsp", mode));
        
        // 匹配 Hint (后备)
        assert!(engine.matches_filter(&candidate, "nih", mode));
        
        // 不匹配
        assert!(!engine.matches_filter(&candidate, "xyz", mode));
    }

    #[test]
    fn test_matches_filter_empty() {
        let engine = create_test_engine();
        let mode = qianyan_ime_core::config::EnglishAuxMode::Prefix;
        let candidate = Candidate {
            text: Arc::from("测试"),
            simplified: Arc::from("测试"),
            traditional: Arc::from("测试"),
            hint: Arc::from("ceshi"),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from(""),
            weight: 1.0,
            match_level: 1,
        };

        assert!(engine.matches_filter(&candidate, "", mode));
        assert!(engine.matches_filter(&candidate, "ces", mode));
        assert!(!engine.matches_filter(&candidate, "xyz", mode));
    }

    fn create_test_engine() -> SearchEngine {
        SearchEngine::new(
            HashMap::new(),
            Arc::new(HashSet::new()),
            Arc::new(HashMap::new()),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(HashMap::new()),
        )
    }

    pub fn reverse_lookup(&self, text: &str) -> Vec<String> {
        let mut results = Vec::new();
        let pipelines = self.pipelines.read().unwrap();
        if let Some(pipeline) = pipelines.0.get("chinese") {
            for t in &pipeline.translators {
                if let Some(table) = t.as_any().downcast_ref::<TableTranslator>() {
                    let index = &table.trie.index;
                    let mut stream = index.stream();
                    while let Some((pinyin_bytes, offset)) = stream.next() {
                        let pinyin = String::from_utf8_lossy(pinyin_bytes);
                        let mut found = false;
                        table.trie.read_block(offset as usize, |tr| {
                            if !found && tr.word == text {
                                results.push(pinyin.to_string());
                                found = true;
                            }
                        });
                        if found {
                            break;
                        }
                    }
                }
            }
        }
        results
    }

    /// 简单的句子转拼音（带分词）。
    pub fn convert_sentence_to_pinyin(&self, text: &str) -> String {
        let mut result = String::new();
        let mut last_was_chinese = false;

        for c in text.chars() {
            if is_chinese(c) {
                let py_list = self.reverse_lookup(&c.to_string());
                if let Some(py) = py_list.first() {
                    if !result.is_empty() && last_was_chinese {
                        result.push(' ');
                    }
                    result.push_str(py);
                    last_was_chinese = true;
                } else {
                    result.push(c);
                    last_was_chinese = false;
                }
            } else {
                if !result.is_empty() && last_was_chinese && !c.is_whitespace() && !c.is_ascii_punctuation() {
                    result.push(' ');
                }
                result.push(c);
                last_was_chinese = false;
            }
        }
        result
    }
}

fn is_chinese(c: char) -> bool {
    (c >= '\u{4e00}' && c <= '\u{9fa5}') || (c >= '\u{3400}' && c <= '\u{4dbf}')
}
