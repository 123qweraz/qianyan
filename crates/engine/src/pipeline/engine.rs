use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;

use crate::config_manager::{UsageData, UserDictData};
use crate::Config;
use crate::Trie;

use super::filters::{AdaptiveFilter, Filter, MatchLevelScoringFilter, TraditionalFilter};
use super::segmentation::{DefaultSegmentor, Segmentor};
use super::translators::{ComposeTranslator, TableTranslator, Translator, UserDictTranslator};
use super::{Candidate, MAX_LOOKUP_LIMIT};

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
        config: &Config,
        limit: usize,
        context: Option<&str>,
    ) -> Vec<Candidate> {
        let segments = {
            if let Ok(guard) = self.segment_cache.read() {
                if let Some(cached) = guard.get(input) {
                    cached.clone()
                } else {
                    drop(guard);
                    let segments = self.segmentor.segment(
                        input,
                        &config.input.segmentation_delimiters,
                        &self.syllable_freq,
                        &self.base_syllables,
                    );
                    if let Ok(mut guard) = self.segment_cache.write() {
                        if guard.len() < 100 {
                            guard.insert(input.to_string(), segments.clone());
                        }
                    }
                    segments
                }
            } else {
                self.segmentor.segment(
                    input,
                    &config.input.segmentation_delimiters,
                    &self.syllable_freq,
                    &self.base_syllables,
                )
            }
        };

        let mut candidates = Vec::new();
        for t in &self.translators {
            candidates.extend(t.translate(input, &segments, config, limit));
        }

        {
            let mut seen = std::collections::HashSet::new();
            candidates.retain(|c| seen.insert(c.text.clone()));
        }
        for f in &self.filters {
            candidates = f.filter(input, candidates, config, context);
        }
        candidates.truncate(limit);
        candidates
    }
}

type PipelineCache = (HashMap<String, Arc<Pipeline>>, std::collections::VecDeque<String>);

/// 搜索引擎：协调所有的 Pipeline
#[derive(Clone)]
pub struct SearchEngine {
    pub trie_paths: HashMap<String, (PathBuf, PathBuf)>,
    pub syllable_freq: Arc<HashMap<String, u64>>,
    pub base_syllables: Arc<HashSet<String>>,
    pub single_syllables: Arc<HashSet<String>>,
    pub(crate) learned_words: Arc<ArcSwap<UserDictData>>,
    pub(crate) usage_history: Arc<ArcSwap<UsageData>>,
    pub(crate) ngram_history: Arc<ArcSwap<UserDictData>>,
    pub schemes: Arc<HashMap<String, Box<dyn crate::scheme::InputScheme>>>,
    pub(crate) pipelines: Arc<RwLock<PipelineCache>>,
    pub(crate) trie_cache: Arc<RwLock<HashMap<String, Arc<Trie>>>>,
}

const MAX_CACHED_PIPELINES: usize = 10;

pub struct SearchQuery<'a> {
    pub buffer: &'a str,
    pub profile: &'a str,
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
        syllable_freq: Arc<HashMap<String, u64>>,
        learned_words: Arc<ArcSwap<UserDictData>>,
        usage_history: Arc<ArcSwap<UsageData>>,
        ngram_history: Arc<ArcSwap<UserDictData>>,
        schemes: Arc<HashMap<String, Box<dyn crate::scheme::InputScheme>>>,
    ) -> Self {
        Self {
            trie_paths,
            syllable_freq,
            base_syllables: Arc::new(HashSet::new()),
            single_syllables: Arc::new(HashSet::new()),
            learned_words,
            usage_history,
            ngram_history,
            schemes,
            pipelines: Arc::new(RwLock::new((HashMap::new(), std::collections::VecDeque::new()))),
            trie_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn search(&self, query: SearchQuery) -> (Vec<Candidate>, Vec<String>) {
        self.do_search(query)
    }

    fn do_search(&self, query: SearchQuery) -> (Vec<Candidate>, Vec<String>) {
        let config_ref = query.config;
        log::info!(
            "engine_search: profile={}, buffer={}, fuzzy_enabled={}, rare_char_mode={:?}",
            query.profile,
            query.buffer,
            query.fuzzy_enabled,
            config_ref.input.rare_char_mode
        );

        let effective_fuzzy = query.fuzzy_enabled && query.config.input.enable_fuzzy_pinyin;

        if let Some(scheme) = self.schemes.get(query.profile) {
            let mut tries_map = HashMap::with_capacity(1);
            if let Some(trie) = self.get_or_load_trie(query.profile) {
                tries_map.insert(query.profile.to_string(), (*trie).clone());
            }
            let context = crate::scheme::SchemeContext {
                config: config_ref,
                tries: &tries_map,
                syllable_freq: &self.syllable_freq,
                base_syllables: &self.base_syllables,
                single_syllables: &self.single_syllables,
                user_dict: &self.learned_words,
                usage_history: &self.usage_history,
                ngram_history: &self.ngram_history,
                active_profiles: &[query.profile.to_string()],
                candidate_count: 0,
                last_word: query.context,
                _filter_mode: query.filter_mode.clone(),
                _aux_filter: query.aux_filter,
                effective_fuzzy,
            };

            let pre_processed = scheme.pre_process(query.buffer, &context);
            let mut scheme_candidates = scheme.lookup(&pre_processed, &context);
            scheme.post_process(&pre_processed, &mut scheme_candidates, &context);

            let mut results = Vec::new();
            for sc in scheme_candidates {
                let hint = Arc::from(match config_ref.input.display_mode {
                    crate::DisplayMode::CharacterOnly => String::new(),
                    crate::DisplayMode::CharacterWithEnglish => {
                        if sc.english.is_empty() {
                            String::new()
                        } else {
                            sc.english.clone()
                        }
                    }
                    crate::DisplayMode::CharacterWithStroke => {
                        if sc.stroke_aux.is_empty() {
                            String::new()
                        } else {
                            sc.stroke_aux.clone()
                        }
                    }
                    crate::DisplayMode::CharacterWithTone => sc.tone.clone(),
                });
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
                    flags: sc.flags,
                });
            }

            if query.filter_mode == crate::processor::FilterMode::Global
                && !query.aux_filter.is_empty()
            {
                results.retain(|c| {
                    self.matches_filter(c, query.aux_filter, config_ref.input.english_aux_mode)
                });
            }

            match config_ref.input.rare_char_mode {
                qianyan_ime_core::config::RareCharMode::CommonOnly => {
                    results.retain(|c| c.flags & 1 == 0);
                }
                qianyan_ime_core::config::RareCharMode::OnlyRare => {
                    results.retain(|c| c.flags & 1 != 0);
                }
                qianyan_ime_core::config::RareCharMode::IncludeRare => {}
            }

            // CommonOnly 保持原有上限，IncludeRare/OnlyRare 放宽到更大值以显示全部生僻字
            let effective_limit = match config_ref.input.rare_char_mode {
                qianyan_ime_core::config::RareCharMode::CommonOnly => query.limit,
                _ => query.limit.max(2000),
            };
            results.truncate(effective_limit);
            log::info!(
                "engine_search: scheme results total={}, rare={}, common={}, mode={:?}",
                results.len(),
                results.iter().filter(|c| c.flags & 1 != 0).count(),
                results.iter().filter(|c| c.flags & 1 == 0).count(),
                config_ref.input.rare_char_mode,
            );
            return (results, vec![]);
        }

        if let Some(pipeline) = self.get_or_create_pipeline(query.profile) {
            let search_limit = if !query.aux_filter.is_empty() {
                MAX_LOOKUP_LIMIT
            } else {
                query.limit
            };

            let results = pipeline.run(
                query.buffer,
                config_ref,
                search_limit,
                query.context,
            );
            let segments = pipeline.segmentor.segment(
                query.buffer,
                &config_ref.input.segmentation_delimiters,
                &pipeline.syllable_freq,
                &pipeline.base_syllables,
            );

            let mut final_results = results;
            if query.filter_mode == crate::processor::FilterMode::Global
                && !query.aux_filter.is_empty()
            {
                final_results.retain(|c| {
                    self.matches_filter(c, query.aux_filter, config_ref.input.english_aux_mode)
                });
            }

            match config_ref.input.rare_char_mode {
                qianyan_ime_core::config::RareCharMode::CommonOnly => {
                    final_results.retain(|c| c.flags & 1 == 0);
                }
                qianyan_ime_core::config::RareCharMode::OnlyRare => {
                    final_results.retain(|c| c.flags & 1 != 0);
                }
                qianyan_ime_core::config::RareCharMode::IncludeRare => {}
            }

            let effective_limit = match config_ref.input.rare_char_mode {
                qianyan_ime_core::config::RareCharMode::CommonOnly => query.limit,
                _ => query.limit.max(2000),
            };
            final_results.truncate(effective_limit);
            log::info!(
                "engine_search: pipeline results total={}, rare={}, common={}, mode={:?}",
                final_results.len(),
                final_results.iter().filter(|c| c.flags & 1 != 0).count(),
                final_results.iter().filter(|c| c.flags & 1 == 0).count(),
                config_ref.input.rare_char_mode,
            );
            return (final_results, segments);
        }

        (vec![], vec![])
    }

    #[inline]
    pub fn get_or_load_trie(&self, profile: &str) -> Option<Arc<Trie>> {
        {
            if let Ok(cache) = self.trie_cache.read() {
                if let Some(trie) = cache.get(profile) {
                    return Some(trie.clone());
                }
            }
        }

        let paths = self.trie_paths.get(profile)?;
        log::info!("Lazy loading trie: profile={}", profile);
        let trie = Trie::load(&paths.0, &paths.1, false).ok()?;
        let trie_arc = Arc::new(trie);

        if let Ok(mut cache) = self.trie_cache.write() {
            cache.entry(profile.to_string()).or_insert(trie_arc.clone());
        }
        Some(trie_arc)
    }

    #[inline]
    pub fn has_exact_match(&self, profile: &str, pinyin: &str, word: &str) -> bool {
        if let Some(trie) = self.get_or_load_trie(profile) {
            if let Some(exacts) = trie.get_all_exact(pinyin) {
                return exacts.iter().any(|tr| tr.word == word);
            }
        }
        false
    }

    #[inline]
    pub fn has_word_in_dict(&self, profile: &str, word: &str) -> bool {
        if let Some(trie) = self.get_or_load_trie(profile) {
            return trie.has_word_in_dict(word);
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

    pub fn get_trie(&self, profile: &str) -> Option<Arc<Trie>> {
        self.get_or_load_trie(profile)
    }

    pub fn get_or_create_pipeline(&self, profile: &str) -> Option<Arc<Pipeline>> {
        // Fast path: read lock only — no write-lock LRU update on every lookup
        if let Ok(cache) = self.pipelines.read() {
            if let Some(p) = cache.0.get(profile) {
                return Some(p.clone());
            }
        }

        let trie_arc = self.get_or_load_trie(profile)?;

        let mut pipeline = Pipeline::new(Box::new(DefaultSegmentor));
        pipeline.syllable_freq = self.syllable_freq.clone();
        pipeline.base_syllables = self.base_syllables.clone();
        pipeline.add_translator(Box::new(UserDictTranslator {
            user_dict: self.learned_words.clone(),
            profile: profile.to_string(),
            trie: Some(trie_arc.clone()),
        }));
        pipeline.add_translator(Box::new(TableTranslator::new(
            trie_arc.clone(),
            self.syllable_freq.clone(),
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

        let mut cache = self.pipelines.write().ok()?;
        if let Some(p) = cache.0.get(profile) {
            return Some(p.clone());
        }
        if cache.0.len() >= MAX_CACHED_PIPELINES {
            if let Some(k) = cache.1.pop_front() {
                cache.0.remove(&k);
            }
        }
        cache.0.insert(profile.to_string(), arc_p.clone());
        cache.1.push_back(profile.to_string());

        Some(arc_p)
    }

    #[inline]
    pub fn has_longer_match(&self, profile: &str, buffer: &str) -> bool {
        if let Some(trie) = self.get_or_load_trie(profile) {
            return trie.has_longer_match(buffer);
        }
        false
    }

    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.pipelines.write() {
            cache.0.clear();
            cache.1.clear();
        }
    }

    /// 编译词库后调用，刷新 trie 缓存 + 预热 word 索引
    pub fn reload_tries(&self) {
        if let Ok(mut tc) = self.trie_cache.write() {
            tc.clear();
        }
        // 立即重载并预热，避免首次查询卡顿
        for profile in self.trie_paths.keys() {
            if let Some(trie) = self.get_or_load_trie(profile) {
                trie.ensure_word_index();
            }
        }
    }

    pub fn prewarm_profile(&self, profile: &str) {
        log::info!("prewarm_profile: profile={}", profile);

        if let Some(trie) = self.get_or_load_trie(profile) {
            trie.prewarm(PREWARM_ENTRIES);
        }
    }

    #[inline]
    pub fn matches_filter(
        &self,
        candidate: &Candidate,
        filter: &str,
        mode: qianyan_ime_core::config::EnglishAuxMode,
    ) -> bool {
        if filter.is_empty() {
            return true;
        }
        let filter_lower = filter.to_lowercase();

        if !candidate.english_aux.is_empty() {
            let en_lower = candidate.english_aux.to_lowercase();
            let parts: Vec<&str> = en_lower
                .split([' ', '/', '(', ')', ',', ';', '|', '.', ':', '!', '?', '[', ']', '{', '}'])
                .collect();

            if mode == qianyan_ime_core::config::EnglishAuxMode::FirstLetter {
                if parts.iter().any(|p| p.starts_with(&filter_lower)) {
                    return true;
                }
            } else {
                if parts.iter().any(|p| p.starts_with(&filter_lower))
                    || en_lower.starts_with(&filter_lower)
                {
                    return true;
                }
            }
        }

        if !candidate.stroke_aux.is_empty()
            && candidate.stroke_aux.to_lowercase().starts_with(&filter_lower) {
                return true;
            }

        let hint_lower = candidate.hint.to_lowercase();
        let parts: Vec<&str> = hint_lower
            .split([' ', '/', '(', ')', ',', ';', '|', '.'])
            .collect();
        parts.iter().any(|p| p.starts_with(&filter_lower))
            || hint_lower.starts_with(&filter_lower)
    }
}

/// 预加载条目数
const PREWARM_ENTRIES: usize = 1000;
