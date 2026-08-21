use crate::keys::VirtualKey;
use crate::processor::Action;
use crate::scheme::{InputScheme, SchemeCandidate, SchemeContext};

/// 逐笔笔画输入方案（bihua）
///
/// 键位映射（手机式，一键一画）：
///   左手 g=横 f=竖 d=撇 s=捺 a=折
///   右手 h=横 j=竖 k=撇 l=捺 p=折
///   z   = 通配符（匹配任意一笔）
///
/// 字典笔画数据以 12345 表示（1横 2竖 3撇 4捺 5折），
/// 输入时把按键翻译回数字序列后直接查询 data/bihua 索引。
pub struct BihuaScheme;

impl Default for BihuaScheme {
    fn default() -> Self {
        Self::new()
    }
}

impl BihuaScheme {
    pub fn new() -> Self {
        Self
    }

    /// 是否为有效的笔画按键（含通配符 z）
    fn is_stroke_key(key: VirtualKey) -> bool {
        matches!(
            key,
            VirtualKey::A
                | VirtualKey::D
                | VirtualKey::F
                | VirtualKey::G
                | VirtualKey::H
                | VirtualKey::J
                | VirtualKey::K
                | VirtualKey::L
                | VirtualKey::P
                | VirtualKey::S
                | VirtualKey::Z
        )
    }

    /// 键位字母序列 → 笔画数字序列（g/h→1, f/j→2, d/k→3, s/l→4, a/p→5, z→通配符）
    fn translate(buffer: &str) -> String {
        let mut res = String::with_capacity(buffer.len());
        for c in buffer.chars() {
            let mapped = match c {
                'g' | 'h' => '1',
                'f' | 'j' => '2',
                'd' | 'k' => '3',
                's' | 'l' => '4',
                'a' | 'p' => '5',
                'z' => 'z',
                _ => continue,
            };
            res.push(mapped);
        }
        res
    }
}

impl InputScheme for BihuaScheme {
    fn pre_process(&self, buffer: &str, _context: &SchemeContext) -> String {
        Self::translate(buffer)
    }

    fn lookup(&self, query: &str, context: &SchemeContext) -> Vec<SchemeCandidate> {
        let mut results = Vec::new();
        let has_wildcard = query.contains('z');

        if let Some(trie) = context.tries.get("bihua") {
            if has_wildcard {
                let matches = trie.search_wildcard(query, 50);
                for tr in matches {
                    let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                    cand.traditional = tr.trad.to_string();
                    cand.tone = tr.tone.to_string();
                    cand.english = tr.en.to_string();
                    cand.stroke_aux = tr.stroke_aux.to_string();
                    cand.flags = tr.flags;
                    cand.match_level = 2; // 通配匹配设为 2
                    results.push(cand);
                }
            } else {
                if let Some(matches) = trie.get_all_exact(query) {
                    for tr in matches {
                        let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                        cand.traditional = tr.trad.to_string();
                        cand.tone = tr.tone.to_string();
                        cand.english = tr.en.to_string();
                        cand.stroke_aux = tr.stroke_aux.to_string();
                        cand.flags = tr.flags;
                        cand.match_level = 3; // 精确匹配设为 3
                        results.push(cand);
                    }
                }

                // 前缀匹配：全局开启时始终启用；未精确命中时兜底前缀匹配
                let enable_prefix_fallback = results.is_empty();
                if context.config.input.enable_prefix_matching || enable_prefix_fallback {
                    let limit = context.config.input.prefix_matching_limit.min(50);
                    let matches = trie.search_bfs(query, limit);
                    for tr in matches {
                        let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                        cand.traditional = tr.trad.to_string();
                        cand.tone = tr.tone.to_string();
                        cand.english = tr.en.to_string();
                        cand.stroke_aux = tr.stroke_aux.to_string();
                        cand.flags = tr.flags;
                        cand.match_level = 1; // 前缀匹配设为 1
                        results.push(cand);
                    }
                }
            }
        }
        results
    }

    fn post_process(
        &self,
        _query: &str,
        candidates: &mut Vec<SchemeCandidate>,
        _context: &SchemeContext,
    ) {
        // 按综合得分排序：级别基础分 + 匹配级别分 + 词频权重
        candidates.sort_by(|a, b| {
            let get_score = |c: &SchemeCandidate| -> i64 {
                let cat_score = match c.stroke_aux.as_str() {
                    "level-1" => 1_000_000_000,
                    "level-2" => 500_000_000,
                    "level-3" => 200_000_000,
                    _ => 0,
                };

                let level_score = match c.match_level {
                    3 => 50_000_000, // 精确匹配
                    1 => 10_000_000, // 前缀匹配
                    _ => 0,          // 通配匹配或其他
                };

                let weight_score = c.weight as i64;

                cat_score + level_score + weight_score
            };
            get_score(b).cmp(&get_score(a))
        });

        // 去重（保留权重最高的）
        let mut seen = std::collections::HashMap::new();
        candidates.retain(|c| {
            let entry = seen.entry(c.text.clone());
            match entry {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(c.weight);
                    true
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if c.weight > *e.get() {
                        e.insert(c.weight);
                        true
                    } else {
                        false
                    }
                }
            }
        });
    }

    fn handle_special_key(
        &self,
        key: VirtualKey,
        _buffer: &mut String,
        _context: &SchemeContext,
    ) -> Option<Action> {
        // 笔画模式下只接受笔画键，其余字母键静默忽略（不进入 buffer）
        if crate::processor::is_letter(key) && !Self::is_stroke_key(key) {
            return Some(Action::Consume);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate_left_hand() {
        assert_eq!(BihuaScheme::translate("gfd"), "123");
        assert_eq!(BihuaScheme::translate("gfd sa"), "12345");
        assert_eq!(BihuaScheme::translate("g"), "1");
        assert_eq!(BihuaScheme::translate("f"), "2");
        assert_eq!(BihuaScheme::translate("d"), "3");
        assert_eq!(BihuaScheme::translate("s"), "4");
        assert_eq!(BihuaScheme::translate("a"), "5");
    }

    #[test]
    fn test_translate_right_hand() {
        assert_eq!(BihuaScheme::translate("hjk"), "123");
        assert_eq!(BihuaScheme::translate("hjk lp"), "12345");
        assert_eq!(BihuaScheme::translate("h"), "1");
        assert_eq!(BihuaScheme::translate("j"), "2");
        assert_eq!(BihuaScheme::translate("k"), "3");
        assert_eq!(BihuaScheme::translate("l"), "4");
        assert_eq!(BihuaScheme::translate("p"), "5");
    }

    #[test]
    fn test_translate_mixed_hands() {
        assert_eq!(BihuaScheme::translate("ghjk"), "1123");
        assert_eq!(BihuaScheme::translate("ghjk lp"), "112345");
        assert_eq!(BihuaScheme::translate("sak"), "453");
    }

    #[test]
    fn test_translate_wildcard() {
        assert_eq!(BihuaScheme::translate("gzd"), "1z3");
        assert_eq!(BihuaScheme::translate("z"), "z");
        assert_eq!(BihuaScheme::translate("ggzz"), "11zz");
    }

    #[test]
    fn test_translate_invalid_chars_dropped() {
        assert_eq!(BihuaScheme::translate("qwergfd"), "123");
        assert_eq!(BihuaScheme::translate("tuvb"), "");
    }

    #[test]
    fn test_stroke_key_validation() {
        assert!(BihuaScheme::is_stroke_key(VirtualKey::G));
        assert!(BihuaScheme::is_stroke_key(VirtualKey::H));
        assert!(BihuaScheme::is_stroke_key(VirtualKey::Z));
        assert!(!BihuaScheme::is_stroke_key(VirtualKey::Q));
        assert!(!BihuaScheme::is_stroke_key(VirtualKey::E));
        assert!(!BihuaScheme::is_stroke_key(VirtualKey::Digit1));
    }

    #[test]
    fn test_handle_special_key_consumes_invalid_letters() {
        let scheme = BihuaScheme::new();
        let mut buffer = String::from("gf");
        assert!(scheme.handle_special_key(VirtualKey::Q, &mut buffer, ctx_stub()) == Some(Action::Consume));
        // 合法笔画键放行
        assert!(scheme.handle_special_key(VirtualKey::G, &mut buffer, ctx_stub()).is_none());
        // 数字键放行（用于选词）
        assert!(scheme.handle_special_key(VirtualKey::Digit1, &mut buffer, ctx_stub()).is_none());
    }

    #[test]
    fn test_lookup_against_real_trie() {
        use crate::config_manager::{OrderData, UserDictData};
        use arc_swap::ArcSwap;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Arc;

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let trie = crate::Trie::load(
            root.join("data/bihua/trie.index"),
            root.join("data/bihua/trie.data"),
            true,
        )
        .expect("Failed to load bihua trie");

        let mut tries = HashMap::new();
        tries.insert("bihua".to_string(), Arc::new(trie));

        let config = crate::Config::default_config();
        let syllable_freq = HashMap::new();
        let base = std::collections::HashSet::new();
        let single = std::collections::HashSet::new();
        let user_dict = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())));
        let ngram = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())));
        let order = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, Vec<String>>::new())));
        let active = vec!["bihua".to_string()];

        let context = SchemeContext {
            config: &config,
            tries: &tries,
            syllable_freq: &syllable_freq,
            base_syllables: &base,
            single_syllables: &single,
            user_dict: &user_dict,
            ngram_history: &ngram,
            user_order: &order,
            active_profiles: &active,
            candidate_count: 0,
            last_word: None,
            last_two_words: None,
        };

        let scheme = BihuaScheme::new();

        // 精确：啊 = 2515212512，左手编码 fagafgfagf
        let query = BihuaScheme::translate("fagafgfagf");
        assert_eq!(query, "2515212512");
        let results = scheme.lookup(&query, &context);
        assert!(
            results.iter().any(|c| c.text == "啊"),
            "啊 not found for exact stroke seq: {:?}",
            results.iter().map(|c| c.text.clone()).collect::<Vec<_>>()
        );

        // 通配符：251521251z
        let results = scheme.lookup("251521251z", &context);
        assert!(!results.is_empty(), "wildcard query should return results");

        // 前缀：25152
        let results = scheme.lookup("25152", &context);
        assert!(!results.is_empty(), "prefix query should return results");

        // 双手混输等价性：左手 g/f/d/s/a 与右手 h/j/k/l/p 各自编码同一字都应命中
        // 啊 = 2515212512：左手 fagafgfagf，右手 jphpjhjphj
        let left = BihuaScheme::translate("fagafgfagf");
        let right = BihuaScheme::translate("jphpjhjphj");
        assert_eq!(left, "2515212512");
        assert_eq!(right, "2515212512");
        let results_l = scheme.lookup(&left, &context);
        let results_r = scheme.lookup(&right, &context);
        assert!(results_l.iter().any(|c| c.text == "啊"));
        assert!(results_r.iter().any(|c| c.text == "啊"));
    }

    fn ctx_stub() -> &'static SchemeContext<'static> {
        use crate::config_manager::{OrderData, UserDictData};
        use arc_swap::ArcSwap;
        use std::collections::HashMap;
        use std::sync::{Arc, OnceLock};

        static STUB: OnceLock<SchemeContext<'static>> = OnceLock::new();
        STUB.get_or_init(|| {
            static CONFIG: OnceLock<crate::Config> = OnceLock::new();
            static TRIES: OnceLock<std::collections::HashMap<String, Arc<crate::Trie>>> = OnceLock::new();
            static SYLLABLE_FREQ: OnceLock<HashMap<String, u64>> = OnceLock::new();
            static BASE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
            static SINGLE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
            static USER_DICT: OnceLock<Arc<ArcSwap<UserDictData>>> = OnceLock::new();
            static NGRAM: OnceLock<Arc<ArcSwap<UserDictData>>> = OnceLock::new();
            static ORDER: OnceLock<Arc<ArcSwap<OrderData>>> = OnceLock::new();
            static ACTIVE: OnceLock<Vec<String>> = OnceLock::new();

            SchemeContext {
                config: CONFIG.get_or_init(crate::Config::default_config),
                tries: TRIES.get_or_init(std::collections::HashMap::new),
                syllable_freq: SYLLABLE_FREQ.get_or_init(HashMap::new),
                base_syllables: BASE.get_or_init(std::collections::HashSet::new),
                single_syllables: SINGLE.get_or_init(std::collections::HashSet::new),
                user_dict: USER_DICT.get_or_init(|| {
                    Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())))
                }),
                ngram_history: NGRAM.get_or_init(|| {
                    Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())))
                }),
                user_order: ORDER.get_or_init(|| {
                    Arc::new(ArcSwap::new(Arc::new(HashMap::<String, Vec<String>>::new())))
                }),
                active_profiles: ACTIVE.get_or_init(Vec::new),
                candidate_count: 0,
                last_word: None,
                last_two_words: None,
            }
        })
    }
}
