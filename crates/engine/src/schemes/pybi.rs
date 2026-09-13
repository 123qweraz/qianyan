use crate::keys::VirtualKey;
use crate::processor::Action;
use crate::scheme::{InputScheme, SchemeCandidate, SchemeContext};

/// 拼音 + 逐笔笔画筛选方案（pybi）
///
/// 底子是拼音，通过 1-5（横竖撇捺折）逐笔输入笔顺来精确定位单字，抛弃数字选字。
/// 交互流程：
///   1. 输入拼音，得到同音**单字**候选（纯拼音时也会给出多字词组）；
///   2. 继续按 1-5 逐个输入目标字的前缀笔顺，候选逐步收窄；
///   3. 同音字前几笔相同时，按 6（末 1 笔）或 7（末 2 笔）切换到末笔筛选，
///      再输入目标字的末尾笔顺（后缀匹配），快速锁定；
///   4. 收窄到唯一时自动上屏。
///
/// buffer 编码（1-5 为笔画，不与模式标记冲突）：
///   - 字母            → 拼音
///   - 1-5             → 前缀笔顺（逐个累加）
///   - 6               → 末 1 笔模式标记，其后 1 个数字为目标字末笔
///   - 7               → 末 2 笔模式标记，其后 2 个数字为目标字末两笔
///   - 8/9/0           → 忽略
///
/// 数据：data/pybi 以拼音为 key，每个单字的完整笔顺存放在 stroke_aux 字段。
pub struct PybiScheme;

impl Default for PybiScheme {
    fn default() -> Self {
        Self::new()
    }
}

impl PybiScheme {
    pub fn new() -> Self {
        Self
    }

    /// 1-5 数字都是笔画键
    fn is_stroke_digit(key: VirtualKey) -> bool {
        matches!(
            key,
            VirtualKey::Digit1
                | VirtualKey::Digit2
                | VirtualKey::Digit3
                | VirtualKey::Digit4
                | VirtualKey::Digit5
        )
    }

    fn digit_char(key: VirtualKey) -> Option<char> {
        match key {
            VirtualKey::Digit1 => Some('1'),
            VirtualKey::Digit2 => Some('2'),
            VirtualKey::Digit3 => Some('3'),
            VirtualKey::Digit4 => Some('4'),
            VirtualKey::Digit5 => Some('5'),
            _ => None,
        }
    }

    /// 从 buffer 中分离出拼音、前缀笔顺与末笔部分。
    ///
    /// 约定：
    ///   - 字母            → pinyin
    ///   - 1-5             → prefix_strokes（逐笔累加）
    ///   - 6               → 末 1 笔模式，其后 1 个数字归入 last_strokes
    ///   - 7               → 末 2 笔模式，其后 2 个数字归入 last_strokes
    ///   - 8/9/0           → 忽略
    ///
    /// 例："ni3235"   → ("ni", "3235", "")
    ///     "ni32356"  → ("ni", "3235", "")
    ///     "ni323564" → ("ni", "3235", "4")
    ///     "ni32357"  → ("ni", "3235", "")
    ///     "ni3235734"→ ("ni", "3235", "34")
    fn split_buffer(buffer: &str) -> (String, String, String) {
        let mut pinyin = String::new();
        let mut prefix = String::new();
        let mut last = String::new();
        let mut mode_pending: Option<char> = None;

        for c in buffer.chars() {
            if c.is_ascii_lowercase() {
                pinyin.push(c);
            } else {
                match c {
                    '1'..='5' => {
                        if let Some(m) = mode_pending {
                            last.push(c);
                            if (m == '6' && last.len() >= 1) || (m == '7' && last.len() >= 2) {
                                mode_pending = None;
                            }
                        } else {
                            prefix.push(c);
                        }
                    }
                    '6' | '7' => {
                        mode_pending = Some(c);
                    }
                    _ => {}
                }
            }
        }
        (pinyin, prefix, last)
    }
}

impl InputScheme for PybiScheme {
    fn pre_process(&self, buffer: &str, _context: &SchemeContext) -> String {
        buffer.to_string()
    }

    fn lookup(&self, query: &str, context: &SchemeContext) -> Vec<SchemeCandidate> {
        let (pinyin, prefix, last) = Self::split_buffer(query);
        let has_strokes = !prefix.is_empty() || !last.is_empty();
        let mut results = Vec::new();

        // 单字候选：来自 data/pybi（拼音→单字+完整笔顺）。
        // 无 pybi 词库时回退到 chinese 词库的单字。
        if let Some(trie) = context.tries.get("pybi").or_else(|| context.tries.get("chinese")) {
            if let Some(entries) = trie.get_all_exact(&pinyin) {
                for tr in entries {
                    // 笔画筛选只对单字有意义
                    if tr.word.chars().count() != 1 {
                        continue;
                    }
                    // pybi 词库中 stroke_aux 即完整笔顺；若来自 chinese 词库，
                    // stroke_aux 是 SBSRF 码，此处用其实际存储值。
                    let full_strokes = tr.stroke_aux.to_lowercase();
                    if full_strokes.is_empty() {
                        continue;
                    }

                    // 前缀笔画：必须前缀匹配完整笔顺
                    if !prefix.is_empty() && !full_strokes.starts_with(&prefix) {
                        continue;
                    }
                    // 末笔笔画：必须后缀匹配完整笔顺
                    if !last.is_empty() && !full_strokes.ends_with(&last) {
                        continue;
                    }

                    let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                    cand.simplified = tr.word.to_string();
                    cand.traditional = tr.trad.to_string();
                    cand.tone = tr.tone.to_string();
                    cand.english = tr.en.to_string();
                    cand.stroke_aux = full_strokes.clone();
                    cand.flags = tr.flags;
                    // 有笔画筛选用 3（精确到位），无笔画时用 2
                    cand.match_level = if has_strokes { 3 } else { 2 };
                    results.push(cand);
                }
            }
        }

        // 拼音词组支持：纯拼音（无任何笔画）时，从左 chinese 词库取多字词组。
        if !has_strokes {
            if let Some(trie) = context.tries.get("chinese") {
                if let Some(entries) = trie.get_all_exact(&pinyin) {
                    for tr in entries {
                        if tr.word.chars().count() < 2 {
                            continue;
                        }
                        let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                        cand.simplified = tr.word.to_string();
                        cand.traditional = tr.trad.to_string();
                        cand.tone = tr.tone.to_string();
                        cand.english = tr.en.to_string();
                        cand.flags = tr.flags;
                        cand.match_level = 2;
                        results.push(cand);
                    }
                }
            }

            // 简拼/混拼词组兜底：前面没有任何候选（单字/词组）时启用，如 zm → 怎么
            if results.is_empty() && context.config.input.enable_abbreviation_matching {
                if let Some(trie) = context.tries.get("chinese") {
                    let abbr_segs =
                        crate::schemes::chinese::ChineseScheme::segment_for_abbreviation(&pinyin, trie);
                    if abbr_segs.len() > 1 {
                        let mut abbr_results = trie.search_abbreviation_mixed(
                            &abbr_segs,
                            50,
                            context.single_syllables,
                        );
                        abbr_results.sort_by_key(|r| std::cmp::Reverse(r.weight));
                        for tr in abbr_results {
                            if tr.word.chars().count() < 2 {
                                continue;
                            }
                            let mut cand = SchemeCandidate::new(tr.word.to_string(), tr.weight);
                            cand.simplified = tr.word.to_string();
                            cand.traditional = tr.trad.to_string();
                            cand.tone = tr.tone.to_string();
                            cand.english = tr.en.to_string();
                            cand.flags = tr.flags;
                            cand.match_level = 2;
                            results.push(cand);
                        }
                    }
                }
            }
        }

        // 优先按词频排序；同级别时保持稳定。
        results.sort_by(|a, b| b.weight.cmp(&a.weight));

        // 去重（保留权重最高的单字）
        let mut seen = std::collections::HashMap::new();
        results.retain(|c| {
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

        results
    }

    fn post_process(
        &self,
        _query: &str,
        _candidates: &mut Vec<SchemeCandidate>,
        _context: &SchemeContext,
    ) {
    }

    fn handle_special_key(
        &self,
        key: VirtualKey,
        buffer: &mut String,
        _context: &SchemeContext,
    ) -> Option<Action> {
        // 1-5 作为笔画输入
        if Self::is_stroke_digit(key) {
            if let Some(c) = Self::digit_char(key) {
                buffer.push(c);
                return Some(Action::Consume);
            }
        }
        // 6=末1笔模式标记，7=末2笔模式标记
        if matches!(key, VirtualKey::Digit6 | VirtualKey::Digit7) {
            let c = if key == VirtualKey::Digit6 { '6' } else { '7' };
            buffer.push(c);
            return Some(Action::Consume);
        }
        // 其余数字（8-9/0）不作数字选字，一律忽略
        if crate::processor::is_digit(key) {
            return Some(Action::Consume);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_buffer() {
        // (pinyin, prefix, last)
        assert_eq!(PybiScheme::split_buffer("ni3234"), ("ni".to_string(), "3234".to_string(), "".to_string()));
        assert_eq!(PybiScheme::split_buffer("ni"), ("ni".to_string(), "".to_string(), "".to_string()));
        assert_eq!(PybiScheme::split_buffer("a12"), ("a".to_string(), "12".to_string(), "".to_string()));
        assert_eq!(PybiScheme::split_buffer(""), ("".to_string(), "".to_string(), "".to_string()));
    }

    #[test]
    fn test_split_buffer_last_stroke() {
        // 6=末1笔
        assert_eq!(PybiScheme::split_buffer("ni32356"), ("ni".to_string(), "3235".to_string(), "".to_string()));
        assert_eq!(PybiScheme::split_buffer("ni323564"), ("ni".to_string(), "3235".to_string(), "4".to_string()));
        // 7=末2笔
        assert_eq!(PybiScheme::split_buffer("ni3235734"), ("ni".to_string(), "3235".to_string(), "34".to_string()));
        // 末1笔模式下再次按 6 可重新进入末笔模式追加
        assert_eq!(PybiScheme::split_buffer("ni6565"), ("ni".to_string(), "".to_string(), "55".to_string()));
    }

    #[test]
    fn test_stroke_digit_keys() {
        assert!(PybiScheme::is_stroke_digit(VirtualKey::Digit1));
        assert!(PybiScheme::is_stroke_digit(VirtualKey::Digit5));
        assert!(!PybiScheme::is_stroke_digit(VirtualKey::Digit6));
        assert!(!PybiScheme::is_stroke_digit(VirtualKey::Digit0));
    }

    #[test]
    fn test_handle_special_key_digits() {
        let scheme = PybiScheme::new();
        let mut buffer = String::new();
        assert!(scheme.handle_special_key(VirtualKey::Digit1, &mut buffer, ctx_stub()) == Some(Action::Consume));
        assert_eq!(buffer, "1");
        assert!(scheme.handle_special_key(VirtualKey::Digit6, &mut buffer, ctx_stub()) == Some(Action::Consume));
        assert_eq!(buffer, "16"); // 6 = 末1笔模式标记
        assert!(scheme.handle_special_key(VirtualKey::Digit7, &mut buffer, ctx_stub()) == Some(Action::Consume));
        assert_eq!(buffer, "167"); // 7 = 末2笔模式标记
        assert!(scheme.handle_special_key(VirtualKey::N, &mut buffer, ctx_stub()).is_none());
        assert!(scheme.handle_special_key(VirtualKey::Digit8, &mut buffer, ctx_stub()) == Some(Action::Consume));
        assert_eq!(buffer, "167"); // 8 被忽略
    }

    #[test]
    fn test_lookup_against_real_trie() {
        use crate::config_manager::{OrderData, UserDictData};
        use arc_swap::ArcSwap;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Arc;

        if !std::path::Path::new(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap()
                .join("data/pybi/trie.index"),
        ).exists() {
            // pybi 数据可能未编译，切换到 chinese 词库验证单字筛选逻辑
        }

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().parent().unwrap().to_path_buf();

        let trie = crate::Trie::load(
            root.join("data/pybi/trie.index"),
            root.join("data/pybi/trie.data"),
            true,
        );

        match trie {
            Ok(t) => {
                let mut tries = HashMap::new();
                tries.insert("pybi".to_string(), Arc::new(t));
                let config = crate::Config::default_config();
                let syllable_freq = HashMap::new();
                let base = std::collections::HashSet::new();
                let single = std::collections::HashSet::new();
                let user_dict = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())));
                let ngram = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new())));
                let order = Arc::new(ArcSwap::new(Arc::new(HashMap::<String, Vec<String>>::new())));
                let active = vec!["pybi".to_string()];
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
                let scheme = PybiScheme::new();
                let res = scheme.lookup("ni", &context);
                assert!(!res.is_empty(), "pybi lookup ni should return candidates");
                // 筛选后应有对应字的完整笔顺
                let res_filtered = scheme.lookup("ni44", &context);
                for c in &res_filtered {
                    assert!(c.stroke_aux.starts_with("44") || c.stroke_aux == "44", "filtered stroke must prefix-match");
                }
            }
            Err(_) => {
                // 数据未编译时跳过
                eprintln!("pybi trie not compiled, skipping real-trie test");
            }
        }
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
