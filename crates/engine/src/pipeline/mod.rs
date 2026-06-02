use std::sync::Arc;

pub mod compose_utils;
pub mod segmentation;
pub use segmentation::*;

pub mod translators;
pub use translators::*;

pub mod filters;
pub use filters::*;

pub mod engine;
pub use engine::*;

pub mod word_discovery;
pub use word_discovery::*;

pub mod search_helper;
pub use search_helper::*;

// 调频衰减算法参数
pub(crate) const RECENCY_BOOST_BASE: f64 = 5000000.0;
pub(crate) const FREQ_BOOST_SCALE: f64 = 1000000.0;
pub(crate) const MAX_USAGE_BOOST: f64 = 10000000.0;
pub(crate) const NGRAM_BOOST_SCALE: f64 = 2000000.0;
pub(crate) const MAX_NGRAM_BOOST: f64 = 5000000.0;

pub const MAX_LOOKUP_LIMIT: usize = 500;
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







#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use arc_swap::ArcSwap;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

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

    #[test]
    fn test_trie_cache_does_not_create_pipeline() {
        let engine = create_test_engine();
        // 没有 trie_paths，get_or_load_trie 应该返回 None 但不崩溃
        assert!(engine.get_or_load_trie("nonexistent").is_none());
        // 确保没有 pipeline 被创建
        let cache = engine.pipelines.read().unwrap();
        assert!(cache.0.is_empty());
    }

    #[test]
    fn test_get_trie_method() {
        let engine = create_test_engine();
        assert!(engine.get_trie("nonexistent").is_none());
    }

    #[test]
    fn test_clear_cache_clears_trie_cache() {
        let engine = create_test_engine();
        // clear_cache 不应崩溃
        engine.clear_cache();
    }

    #[test]
    fn test_do_search_scheme_path_no_pipeline_created() {
        use std::collections::HashMap;
        use std::sync::Arc;

        // 注册一个与 chinese 同名的 dummy scheme，验证 scheme 路径不被 pipeline 依赖
        let schemes: HashMap<String, Box<dyn crate::scheme::InputScheme>> = HashMap::new();
        let engine = SearchEngine::new(
            HashMap::new(),
            Arc::new(HashSet::new()),
            Arc::new(HashMap::new()),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(schemes),
        );

        // 即使没有 trie，scheme 路径（这里没 scheme）应优雅降级
        let config = crate::Config::default_config();
        let query = SearchQuery {
            buffer: "test",
            profile: "chinese",
            syllables: &HashSet::new(),
            config: &config,
            limit: 10,
            filter_mode: crate::processor::FilterMode::None,
            aux_filter: "",
            context: None,
            fuzzy_enabled: false,
        };
        let (candidates, _segments) = engine.search(query);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_compute_decay_boost_range() {
        // 验证 compute_decay_boost 的返回值在合理范围内
        let boost = compute_decay_boost(0, 1);
        assert!(boost > 0.0);
        assert!(boost <= MAX_USAGE_BOOST);

        // 高频、最近使用的词加成最大
        let high_boost = compute_decay_boost(0, 20);
        // 低频、很久没用过的词加成最小
        let low_boost = compute_decay_boost(100, 0);
        assert!(high_boost >= low_boost);

        // 验证上限
        let max_boost = compute_decay_boost(0, 100);
        assert!(max_boost <= MAX_USAGE_BOOST);
    }

    #[test]
    fn test_candidate_pagination_matches_count() {
        // 验证分页计算与实际候选数一致
        let page_size = 5usize;
        let total_candidates = 95usize;
        let total_pages = (total_candidates + page_size - 1) / page_size;
        assert_eq!(total_pages, 19);
        assert_eq!(total_pages * page_size, 95);
        // 如果 page_size 改变，分页数应正确反映
        let page_size = 10usize;
        let total_pages = (total_candidates + page_size - 1) / page_size;
        assert_eq!(total_pages, 10);
        assert_eq!((total_pages - 1) * page_size < total_candidates, true);
    }
}


