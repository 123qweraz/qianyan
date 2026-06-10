use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;

use crate::config_manager::{UserDictData, OrderData};
use crate::Config;
use crate::Trie;

use super::segmentation::{DefaultSegmentor, Segmentor};
use super::Candidate;

/// 搜索引擎：协调所有的方案
#[derive(Clone)]
pub struct SearchEngine {
    pub trie_paths: HashMap<String, (PathBuf, PathBuf)>,
    pub syllable_freq: Arc<HashMap<String, u64>>,
    pub base_syllables: Arc<HashSet<String>>,
    pub single_syllables: Arc<HashSet<String>>,
    pub(crate) learned_words: Arc<ArcSwap<UserDictData>>,
    pub(crate) ngram_history: Arc<ArcSwap<UserDictData>>,
    pub(crate) user_order: Arc<ArcSwap<OrderData>>,
    pub schemes: Arc<HashMap<String, Box<dyn crate::scheme::InputScheme>>>,
    pub(crate) trie_cache: Arc<RwLock<HashMap<String, Arc<Trie>>>>,
}

pub struct SearchQuery<'a> {
    pub buffer: &'a str,
    pub profile: &'a str,
    pub config: &'a Config,
    pub limit: usize,
    pub filter_mode: crate::processor::FilterMode,
    pub aux_filter: &'a str,
    pub context: Option<&'a str>,
    pub context_pair: Option<(&'a str, &'a str)>,
}

impl SearchEngine {
    pub fn new(
        trie_paths: HashMap<String, (PathBuf, PathBuf)>,
        syllable_freq: Arc<HashMap<String, u64>>,
        learned_words: Arc<ArcSwap<UserDictData>>,
        ngram_history: Arc<ArcSwap<UserDictData>>,
        user_order: Arc<ArcSwap<OrderData>>,
        schemes: Arc<HashMap<String, Box<dyn crate::scheme::InputScheme>>>,
    ) -> Self {
        let mut engine = Self {
            trie_paths,
            syllable_freq,
            base_syllables: Arc::new(HashSet::new()),
            single_syllables: Arc::new(HashSet::new()),
            learned_words,
            ngram_history,
            user_order,
            schemes,
            trie_cache: Arc::new(RwLock::new(HashMap::new())),
        };
        engine.load_base_syllables();
        engine
    }

    fn load_base_syllables(&mut self) {
        let paths = [
            std::path::PathBuf::from("dicts/chinese/single_syllables.txt"),
            std::path::PathBuf::from("../dicts/chinese/single_syllables.txt"),
            std::path::PathBuf::from("../../dicts/chinese/single_syllables.txt"),
        ];
        if let Some(content) = paths.iter().find_map(|p| std::fs::read_to_string(p).ok()) {
            let set: HashSet<String> = content.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect();
            if !set.is_empty() {
                self.base_syllables = Arc::new(set.clone());
                self.single_syllables = Arc::new(set);
            }
        }
    }

    pub fn search(&self, query: SearchQuery) -> (Vec<Candidate>, Vec<String>) {
        self.do_search(query)
    }

    fn do_search(&self, query: SearchQuery) -> (Vec<Candidate>, Vec<String>) {
        let config_ref = query.config;
        log::info!(
            "engine_search: profile={}, buffer={}, rare_char_mode={:?}",
            query.profile,
            query.buffer,
            config_ref.input.rare_char_mode
        );

        if let Some(scheme) = self.schemes.get(query.profile) {
            let mut tries_map = HashMap::with_capacity(1);
            if let Some(trie) = self.get_or_load_trie(query.profile) {
                tries_map.insert(query.profile.to_string(), trie);
            }
            let context = crate::scheme::SchemeContext {
                config: config_ref,
                tries: &tries_map,
                syllable_freq: &self.syllable_freq,
                base_syllables: &self.base_syllables,
                single_syllables: &self.single_syllables,
                user_dict: &self.learned_words,
                ngram_history: &self.ngram_history,
                user_order: &self.user_order,
                active_profiles: &[query.profile.to_string()],
                candidate_count: 0,
                last_word: query.context,
                last_two_words: query.context_pair,
            };

            let pre_processed = scheme.pre_process(query.buffer, &context);
            let mut scheme_candidates = scheme.lookup(&pre_processed, &context);
            scheme.post_process(&pre_processed, &mut scheme_candidates, &context);

            let mut results = Vec::new();
            for sc in scheme_candidates {
                let hint = Arc::from(match config_ref.input.display_mode {
                    crate::DisplayMode::CharacterOnly => "",
                    crate::DisplayMode::CharacterWithEnglish => {
                        sc.english.as_str()
                    }
                    crate::DisplayMode::CharacterWithStroke => {
                        sc.stroke_aux.as_str()
                    }
                    crate::DisplayMode::CharacterWithTone => sc.tone.as_str(),
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
            let delims = &query.config.input.segmentation_delimiters;
            let segs = DefaultSegmentor.segment(&pre_processed, delims, &self.syllable_freq, &self.base_syllables);
            let non_degen = !segs.is_empty() && !(segs.len() == pre_processed.len() && segs.iter().all(|s| s.len() == 1));
            return (results, if non_degen { segs } else { vec![] });
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

    pub fn get_trie(&self, profile: &str) -> Option<Arc<Trie>> {
        self.get_or_load_trie(profile)
    }

    #[inline]
    pub fn has_longer_match(&self, profile: &str, buffer: &str) -> bool {
        if let Some(trie) = self.get_or_load_trie(profile) {
            return trie.has_longer_match(buffer);
        }
        false
    }

    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.trie_cache.write() {
            cache.clear();
        }
    }

    /// 编译词库后调用，刷新 trie 缓存
    pub fn reload_tries(&self) {
        if let Ok(mut tc) = self.trie_cache.write() {
            tc.clear();
        }
        for profile in self.trie_paths.keys() {
            self.get_or_load_trie(profile);
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


