use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::config_manager::UserDictData;
use crate::trie::TrieResult;
use crate::Config;
use crate::Trie;

use super::{Candidate, CACHE_TTL_MS, MAX_LOOKUP_LIMIT};
use super::segmentation::fuzzy_variants_per_segment;

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
    fuzzy_cache: std::sync::RwLock<std::collections::HashMap<String, Vec<String>>>,
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
            fuzzy_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
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

        {
            if let (Ok(last_q_guard), Ok(cached)) =
                (self.last_query.read(), self.cached_candidates.read())
            {
                let (last_q, last_time) = &*last_q_guard;

                if query.starts_with(last_q)
                    && last_time.elapsed().as_millis() < CACHE_TTL_MS as u128
                {
                    let filtered: Vec<Candidate> = cached
                        .iter()
                        .filter(|c| c.simplified.starts_with(&query))
                        .cloned()
                        .collect();

                    if !filtered.is_empty() {
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
            Arc::from(match config.input.display_mode {
                crate::DisplayMode::CharacterOnly => String::new(),
                crate::DisplayMode::CharacterWithEnglish => {
                    if tr.en.is_empty() { String::new() } else { tr.en.to_string() }
                }
                crate::DisplayMode::CharacterWithStroke => {
                    if tr.stroke_aux.is_empty() { String::new() } else { tr.stroke_aux.to_string() }
                }
                crate::DisplayMode::CharacterWithTone => tr.tone.to_string(),
            })
        };

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

        if config.input.enable_fuzzy_pinyin {
            let fuzzy_cfg = &config.input.fuzzy_config;
            let per_seg: Vec<Vec<String>> = segments
                .iter()
                .map(|seg| {
                    if let Ok(cache) = self.fuzzy_cache.read() {
                        if let Some(cached) = cache.get(seg) {
                            return cached.clone();
                        }
                    }
                    let variants = fuzzy_variants_per_segment(seg, fuzzy_cfg);
                    if let Ok(mut cache) = self.fuzzy_cache.write() {
                        cache.insert(seg.clone(), variants.clone());
                    }
                    variants
                })
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
    pub trie: Option<Arc<Trie>>,
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
        log::trace!("UserDictTranslator: query={}, profile={}", query, self.profile);
        if let Some(profile_dict) = dict.get(&self.profile) {
            log::trace!("UserDictTranslator: has_query={}", profile_dict.contains_key(&query));
            if let Some(words) = profile_dict.get(&query) {
                for (word, weight) in words {
                    let (trad, en, stroke) = if let Some(ref trie) = self.trie {
                        if let Some(exacts) = trie.get_all_exact(&query) {
                            if let Some(tr) = exacts.iter().find(|tr| tr.word == *word) {
                                let trad = if tr.trad.is_empty() {
                                    Arc::from(word.as_str())
                                } else {
                                    Arc::from(tr.trad)
                                };
                                (trad, Arc::from(tr.en), Arc::from(tr.stroke_aux))
                            } else {
                                (Arc::from(word.as_str()), Arc::from(""), Arc::from(""))
                            }
                        } else {
                            (Arc::from(word.as_str()), Arc::from(""), Arc::from(""))
                        }
                    } else {
                        (Arc::from(word.as_str()), Arc::from(""), Arc::from(""))
                    };

                    results.push(Candidate {
                        text: Arc::from(word.as_str()),
                        simplified: Arc::from(word.as_str()),
                        traditional: trad,
                        hint: Arc::from("User"),
                        english_aux: en,
                        stroke_aux: stroke,
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
        let base = self.segment_base(input);
        if base.len() < 2 || base.len() > 12 {
            return vec![];
        }

        let mut all_partitions = Vec::new();
        self.backtrack_partitions(&base, 0, &mut Vec::new(), &mut all_partitions);

        if all_partitions.len() > 100 {
            all_partitions.truncate(100);
        }

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

        results.sort_by(|a, b| b.0.cmp(&a.0));
        results.dedup_by(|a, b| a.0 == b.0);

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
