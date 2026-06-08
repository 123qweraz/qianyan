use fst::{Automaton, IntoStreamer, Map, Streamer};
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, OnceLock};

const ABBREVIATION_SCAN_LIMIT: usize = 3000;
pub const TRIE_MAGIC: &[u8; 4] = b"QYTR";
pub const TRIE_VERSION: u32 = 2; // v2: 增加了 flags 字节用于生僻字过滤

#[derive(Clone, Copy)]
pub struct TrieResult<'a> {
    pub word: &'a str,
    pub trad: &'a str,
    pub tone: &'a str,
    pub en: &'a str,
    pub stroke_aux: &'a str,
    pub weight: u32,
    pub flags: u8, // bit0: 1 = level-4 (生僻字)
}

#[derive(Clone)]
pub enum TrieData {
    Mmap(Arc<Mmap>),
    Memory(Arc<Vec<u8>>),
}

impl AsRef<[u8]> for TrieData {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Mmap(m) => m.as_ref(),
            Self::Memory(v) => v.as_ref(),
        }
    }
}

#[derive(Clone)]
pub struct Trie {
    pub index: Map<TrieData>,
    data: TrieData,
    word_pinyin: Arc<OnceLock<HashMap<Box<str>, Box<str>>>>,
    rare_chars_cache: Arc<OnceLock<HashSet<String>>>,
}

impl Trie {
    pub fn load<P: AsRef<Path>>(
        index_path: P,
        data_path: P,
        force_memory: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let load_data = |path: &Path| -> Result<TrieData, Box<dyn std::error::Error>> {
            if force_memory {
                let buffer = std::fs::read(path)?;
                Ok(TrieData::Memory(Arc::new(buffer)))
            } else {
                let file = File::open(path)?;
                // SAFETY: The mapped file is a read-only dictionary (trie.data or trie.index)
                // that is never modified after initial creation. No other code writes to it.
                // Mmap::map is unsafe because the underlying file could be modified concurrently
                // by another process, but in practice these data files are static assets.
                let mmap = unsafe { Mmap::map(&file)? };
                Ok(TrieData::Mmap(Arc::new(mmap)))
            }
        };

        let index_data = load_data(index_path.as_ref())?;
        let data_data = load_data(data_path.as_ref())?;
        let index = Map::new(index_data)?;

        // 检查版本头
        let data = data_data.as_ref();
        if data.len() >= 8 && &data[0..4] == TRIE_MAGIC {
            let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
            if version != TRIE_VERSION {
                return Err(format!(
                    "Trie version mismatch: expected {}, found {}",
                    TRIE_VERSION, version
                )
                .into());
            }
        } else {
            // 如果没有魔法数字，视为旧版本 v1
            return Err("Trie format too old (v1), please recompile dictionaries.".into());
        }

        Ok(Self {
            index,
            data: data_data,
            word_pinyin: Arc::new(OnceLock::new()),
            rare_chars_cache: Arc::new(OnceLock::new()),
        })
    }

    pub fn get_all_exact(&self, pinyin: &str) -> Option<Vec<TrieResult<'_>>> {
        self.get_all_exact_with_level_filter(pinyin, None)
    }

    /// 带等级过滤的精确匹配
    pub fn get_all_exact_with_level_filter(
        &self,
        pinyin: &str,
        level_filter: Option<&str>,
    ) -> Option<Vec<TrieResult<'_>>> {
        log::debug!("trie_exact: pinyin={}, level_filter={:?}", pinyin, level_filter);
        let offset = self.index.get(pinyin)? as usize;
        let mut results = Vec::new();
        self.read_block(offset, |tr| {
            // 应用等级过滤
            if let Some(filter_level) = level_filter {
                if tr.stroke_aux == filter_level {
                    results.push(tr);
                }
            } else {
                results.push(tr);
            }
        });
        // 如果过滤后没有结果，返回 None
        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }

    /// 获取生僻字集合（flags & 1），带内部缓存
    pub fn get_rare_chars(&self) -> &HashSet<String> {
        self.rare_chars_cache
            .get_or_init(|| self.build_rare_chars())
    }

    /// 从二进制数据中提取生僻字集合（level-4, level-5, flags & 1）
    pub fn build_rare_chars(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        let mut stream = self.index.stream();
        while let Some((_, offset)) = fst::Streamer::next(&mut stream) {
            self.read_block(offset as usize, |tr| {
                if tr.flags & 1 != 0 {
                    set.insert(tr.word.to_string());
                }
            });
        }
        set
    }

    /// 预热词库：读取前 limit 条记录以填充 Page Cache
    pub fn prewarm(&self, limit: usize) {
        if matches!(self.data, TrieData::Memory(_)) {
            return;
        }
        let mut stream = self.index.stream();
        let mut count = 0;
        while let Some((_, offset)) = fst::Streamer::next(&mut stream) {
            self.read_block(offset as usize, |_| {});
            count += 1;
            if count >= limit {
                break;
            }
        }
    }

    /// 预热 word_pinyin 索引，避免首次 has_word_in_dict 触发的全量扫描
    pub fn ensure_word_index(&self) {
        self.word_pinyin.get_or_init(|| self.build_word_pinyin());
    }

    /// 快速前缀检查：FST 中是否有任何 key 以 prefix 开头（不读数据块）
    pub fn has_prefix(&self, prefix: &str) -> bool {
        let matcher = fst::automaton::Str::new(prefix).starts_with();
        let mut stream = self.index.search(matcher).into_stream();
        stream.next().is_some()
    }

    pub fn has_longer_match(&self, prefix: &str) -> bool {
        let matcher = fst::automaton::Str::new(prefix).starts_with();
        let mut stream = self.index.search(matcher).into_stream();
        while let Some((key, _)) = stream.next() {
            if key.len() > prefix.len() {
                return true;
            }
        }
        false
    }

    /// 检查词典中是否存在该词（不限拼音），用于防止系统词典词被重复加入用户词典
    /// 使用 word_index 实现 O(1) 查找（首次调用时惰性构建索引，之后 O(1)）
    pub fn has_word_in_dict(&self, word: &str) -> bool {
        let index = self.word_pinyin.get_or_init(|| self.build_word_pinyin());
        index.contains_key(word)
    }

    fn build_word_pinyin(&self) -> HashMap<Box<str>, Box<str>> {
        let mut map = HashMap::with_capacity(50000);
        let mut stream = self.index.stream();
        while let Some((key, offset)) = fst::Streamer::next(&mut stream) {
            let mut first = true;
            self.read_block(offset as usize, |tr| {
                let word: Box<str> = Box::from(tr.word);
                if first {
                    if let Ok(key_str) = std::str::from_utf8(key) {
                        map.entry(word).or_insert_with(|| Box::<str>::from(key_str));
                    }
                    first = false;
                }
            });
        }
        map
    }

    pub fn lookup_pinyin(&self, word: &str) -> Option<&str> {
        let index = self.word_pinyin.get_or_init(|| self.build_word_pinyin());
        index.get(word).map(|s| s.as_ref())
    }

    pub fn search_bfs(&self, prefix: &str, limit: usize) -> Vec<TrieResult<'_>> {
        self.search_bfs_with_level_filter(prefix, limit, None)
    }

    /// 带等级过滤的前缀搜索
    /// level_filter: Some("level-1") 表示只返回 level-1 的结果，None 表示返回所有结果
    pub fn search_bfs_with_level_filter(
        &self,
        prefix: &str,
        limit: usize,
        level_filter: Option<&str>,
    ) -> Vec<TrieResult<'_>> {
        log::debug!("trie_bfs: prefix={}, limit={}, level_filter={:?}", prefix, limit, level_filter);
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let matcher = fst::automaton::Str::new(prefix).starts_with();
        let mut stream = self.index.search(matcher).into_stream();

        // 收集所有匹配 key 的完整数据块
        while let Some((_, offset)) = stream.next() {
            self.read_block(offset as usize, |pair| {
                if seen.insert(pair.word) {
                    if let Some(filter_level) = level_filter {
                        if pair.stroke_aux == filter_level {
                            results.push(pair);
                        }
                    } else {
                        results.push(pair);
                    }
                }
            });
        }

        // 部分排序仅取 Top-K，避免对全部结果全排序
        let n = results.len().min(limit);
        if n > 0 {
            results.select_nth_unstable_by_key(n.saturating_sub(1), |r| std::cmp::Reverse(r.weight));
            results.truncate(n);
            results.sort_by_key(|r| std::cmp::Reverse(r.weight));
        }
        results
    }

    /// 通配符搜索实现：z 匹配任意单个 a-y 字母
    pub fn search_wildcard(&self, pattern: &str, limit: usize) -> Vec<TrieResult<'_>> {
        self.search_wildcard_with_level_filter(pattern, limit, None)
    }

    /// 带等级过滤的通配符搜索
    /// 优化：提取非通配符前缀利用 FST prefix search 剪枝，避免全表扫描
    pub fn search_wildcard_with_level_filter(
        &self,
        pattern: &str,
        limit: usize,
        level_filter: Option<&str>,
    ) -> Vec<TrieResult<'_>> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let prefix = self.wildcard_prefix(pattern);
        let matcher = fst::automaton::Str::new(prefix).starts_with();
        let mut stream = self.index.search(matcher).into_stream();

        while let Some((key_bytes, offset)) = stream.next() {
            let key = match std::str::from_utf8(key_bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if Self::wildcard_match(pattern, key) {
                let mut stop = false;
                self.read_block(offset as usize, |pair| {
                    if !stop && seen.insert(pair.word) {
                        if let Some(filter_level) = level_filter {
                            if pair.stroke_aux == filter_level {
                                results.push(pair);
                                if results.len() >= limit {
                                    stop = true;
                                }
                            }
                        } else {
                            results.push(pair);
                            if results.len() >= limit {
                                stop = true;
        }
    }
}
                });
                if stop {
                    break;
                }
            }
        }
        results
    }

    /// 提取 pattern 中第一个 'z' 之前的前缀，用于 FST prefix 搜索剪枝
    fn wildcard_prefix<'a>(&self, pattern: &'a str) -> &'a str {
        match pattern.find('z') {
            Some(pos) if pos > 0 => &pattern[..pos],
            _ => "",
        }
    }

    fn wildcard_match(pattern: &str, key: &str) -> bool {
        // 如果 pattern 不包含通配符，就是简单前缀匹配
        if !pattern.contains('z') {
            return key.starts_with(pattern);
        }

        // 逐字节比较（拼音只含 ASCII 字符），z 匹配任意单个字母
        let p = pattern.as_bytes();
        let k = key.as_bytes();
        if p.len() > k.len() {
            return false;
        }
        for i in 0..p.len() {
            if p[i] != b'z' && p[i] != k[i] {
                return false;
            }
        }
        true
    }

    pub fn search_abbreviation(
        &self,
        segments: &[String],
        limit: usize,
    ) -> Vec<TrieResult<'_>> {
        if segments.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::with_capacity(limit);
        let mut seen = std::collections::HashSet::new();

        let first_seg = &segments[0];
        let matcher = fst::automaton::Str::new(first_seg).starts_with();
        let mut stream = self.index.search(matcher).into_stream();

        while let Some((key_bytes, offset)) = stream.next() {
            let key = String::from_utf8_lossy(key_bytes);

            if self.matches_strict_jianpin(&key, segments) {
                let mut stop = false;
                self.read_block(offset as usize, |pair| {
                    if !stop && seen.insert(pair.word) {
                        results.push(pair);
                        if results.len() >= ABBREVIATION_SCAN_LIMIT {
                            stop = true;
                        }
                    }
                });
                if stop {
                    break;
                }
            }
            if results.len() >= ABBREVIATION_SCAN_LIMIT {
                break;
            }
        }
        results
    }

    /// 严格简拼匹配：输入 segments 数量必须等于词组音节数
    fn matches_strict_jianpin(
        &self,
        key: &str,
        segments: &[String],
    ) -> bool {
        self.recursive_strict_match(key, segments)
    }

    fn recursive_strict_match(
        &self,
        key: &str,
        segments: &[String],
    ) -> bool {
        // 如果 segments 耗尽，则 key 也必须耗尽（确保音节数一致）
        if segments.is_empty() {
            return key.is_empty();
        }

        if key.is_empty() {
            return false;
        }

        let first_seg = &segments[0];

        // 安全地按字符边界尝试切分
        // 拼音音节最长通常为 6 字节（如 chuang），
        // 但为了 Unicode 安全，我们遍历实际的字符索引
        for (char_count, (byte_idx, _)) in key.char_indices().enumerate() {
            let len = byte_idx;
            if len > 0 && len <= 10 {
                // 适当放宽长度限制以处理带声调的 Unicode
                let syl = &key[..len];
                if self.index.contains_key(syl) {
                    // 声母必须匹配
                    if syl.starts_with(first_seg) {
                        // 递归匹配剩余音节
                        if self.recursive_strict_match(&key[len..], &segments[1..]) {
                            return true;
                        }
                    }
                }
            }
            if char_count > 8 {
                break;
            } // 一个音节不可能超过 8 个字符
        }

        // 兜底：尝试全量匹配最后一个或唯一一个音节
        if self.index.contains_key(key) && key.starts_with(first_seg) && segments.len() == 1 {
            return true;
        }

        false
    }

    /// 混拼检索：segments 为 (segment, is_initial) 对
    /// is_initial=true → 声母简拼 (starts_with)，false → 全音节精确匹配
    /// single_syllables: 合法单音节集合，用于防止多音节 key 冒充单音节匹配缩写
    pub fn search_abbreviation_mixed(
        &self,
        segments: &[(String, bool)],
        limit: usize,
        single_syllables: &std::collections::HashSet<String>,
    ) -> Vec<TrieResult<'_>> {
        if segments.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::with_capacity(limit);
        let mut seen = std::collections::HashSet::new();

        let first_seg = &segments[0].0;
        let matcher = fst::automaton::Str::new(first_seg).starts_with();
        let mut stream = self.index.search(matcher).into_stream();

        while let Some((key_bytes, offset)) = stream.next() {
            let key = String::from_utf8_lossy(key_bytes);
            if self.recursive_mixed_match(&key, segments, single_syllables) {
                let mut stop = false;
                self.read_block(offset as usize, |pair| {
                    if !stop && seen.insert(pair.word) {
                        results.push(pair);
                        if results.len() >= ABBREVIATION_SCAN_LIMIT {
                            stop = true;
                        }
                    }
                });
                if stop {
                    break;
                }
            }
            if results.len() >= ABBREVIATION_SCAN_LIMIT {
                break;
            }
        }
        results
    }

    fn recursive_mixed_match(
        &self,
        key: &str,
        segments: &[(String, bool)],
        single_syllables: &std::collections::HashSet<String>,
    ) -> bool {
        if segments.is_empty() {
            return key.is_empty();
        }
        if key.is_empty() {
            return false;
        }

        let (first_seg, is_initial) = &segments[0];

        for (char_count, (byte_idx, _)) in key.char_indices().enumerate() {
            let len = byte_idx;
            if len > 0 && len <= 10 {
                let syl = &key[..len];
                if self.index.contains_key(syl) {
                    let matches = if *is_initial {
                        // 声母缩写时，如果 single_syllables 已加载则只接受合法单音节，
                        // 防止多音节 key 冒充单音节；未加载时向后兼容
                        (single_syllables.is_empty() || single_syllables.contains(syl))
                            && syl.starts_with(first_seg.as_str())
                    } else {
                        syl == first_seg.as_str()
                    };
                    if matches
                        && self.recursive_mixed_match(&key[len..], &segments[1..], single_syllables) {
                            return true;
                        }
                }
            }
            if char_count > 8 {
                break;
            }
        }

        if segments.len() == 1 && self.index.contains_key(key) {
            let matches = if *is_initial {
                // 同样需要检查单音节
                (single_syllables.is_empty() || single_syllables.contains(key))
                    && key.starts_with(first_seg.as_str())
            } else {
                key == first_seg.as_str()
            };
            if matches {
                return true;
            }
        }

        false
    }


    pub fn read_block<'a>(&'a self, offset: usize, mut f: impl FnMut(TrieResult<'a>)) {
        let data = self.data.as_ref();
        if offset + 4 > data.len() {
            log::warn!("[Trie] read_block: offset {} beyond data length {}", offset, data.len());
            return;
        }

        let count = u32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .unwrap_or_default(),
        );
        let mut cursor = offset + 4;

        for _ in 0..count {
            let word = match Self::read_str(data, &mut cursor, "word") {
                Some(s) => s,
                None => break,
            };
            let trad = match Self::read_str(data, &mut cursor, "trad") {
                Some(s) => s,
                None => break,
            };
            let tone = match Self::read_str(data, &mut cursor, "tone") {
                Some(s) => s,
                None => break,
            };
            let en = match Self::read_str(data, &mut cursor, "en") {
                Some(s) => s,
                None => break,
            };
            let stroke_aux = match Self::read_str(data, &mut cursor, "stroke_aux") {
                Some(s) => s,
                None => break,
            };

            if cursor + 4 > data.len() {
                log::warn!("[Trie] read_block: truncated weight at cursor {}", cursor);
                break;
            }
            let weight = u32::from_le_bytes(
                data[cursor..cursor + 4]
                    .try_into()
                    .unwrap_or_default(),
            );
            cursor += 4;

            let flags = if cursor < data.len() { data[cursor] } else { 0 };
            cursor += 1;

            f(TrieResult {
                word,
                trad,
                tone,
                en,
                stroke_aux,
                weight,
                flags,
            });
        }
    }

    fn read_str<'a>(data: &'a [u8], cursor: &mut usize, field_name: &str) -> Option<&'a str> {
        if *cursor + 2 > data.len() {
            log::warn!("[Trie] read_block: truncated {} length at cursor {}", field_name, *cursor);
            return None;
        }
        let len = u16::from_le_bytes(
            data[*cursor..*cursor + 2]
                .try_into()
                .unwrap_or_default(),
        ) as usize;
        *cursor += 2;
        if *cursor + len > data.len() {
            log::warn!("[Trie] read_block: truncated {} data at cursor {}", field_name, *cursor);
            return None;
        }
        match std::str::from_utf8(&data[*cursor..*cursor + len]) {
            Ok(s) => {
                *cursor += len;
                Some(s)
            }
            Err(e) => {
                log::warn!("[Trie] read_block: invalid utf8 for {}: {}", field_name, e);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_qi_exact_lookup() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let trie = Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");

        assert!(trie.index.get("qi").is_some(), "Key 'qi' not found in trie index!");
        let results = trie.get_all_exact("qi").expect("No results for 'qi'!");
        assert!(results.iter().any(|r| r.word == "器"), "器 not found in trie for 'qi'");
    }

    #[test]
    fn test_candidate_count_chinese() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        // Load a real Config to get enable_prefix_matching=true etc.
        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let config = qianyan_ime_core::config::Config::load();

        let mut trie_paths = std::collections::HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllable_freq: std::collections::HashMap<String, u64> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().filter_map(|s| { let s = s.trim(); if s.is_empty() { None } else { let (p,c) = s.split_once(' ')?; Some((p.to_string(), c.parse().unwrap_or(1))) } }).collect()
        };

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(std::collections::HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, u32>
            >::new()))),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            {
                let mut m: std::collections::HashMap<String, Box<dyn InputScheme>> = std::collections::HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        // Test "li" — there are MANY characters for li (里力立利理李离例 etc.)
        // Should return way more than 20 candidates
        let query = SearchQuery {
            buffer: "li",
            profile: "chinese",
            config: &config,
            limit: crate::pipeline::MAX_LOOKUP_LIMIT,
            filter_mode: crate::processor::FilterMode::None,
            aux_filter: "",
            context: None,
            fuzzy_enabled: false,
        };
        let (candidates, _) = engine.search(query);
        let count = candidates.len();

        // Print top 30 for inspection
        println!("=== 'li' search: {} total candidates ===", count);
        for (i, c) in candidates.iter().enumerate().take(30) {
            println!("  [{}] {} weight={}", i, c.text, c.weight);
        }

        // There should be WAY more than 20 — at least 50+ characters for common pinyin "li"
        // Note: source dicts have 121 total entries for "li" across chars/level2/level3,
        // but the compiler deduplicates by word (same word in multiple levels), yielding 95 unique.
        assert_eq!(count, 95,
            "Expected exactly 95 unique candidates for 'li' (121 total across dict files - 26 duplicates)");

        // Also verify the trie itself has 80+ entries for "li"
        let trie = Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");
        let trie_count = trie.get_all_exact("li").map(|v| v.len()).unwrap_or(0);
        println!("'li' exact trie entries: {}", trie_count);
        assert!(trie_count >= 95, "Expected at least 95 entries in trie for 'li', got {}", trie_count);
    }

    #[test]
    fn debug_xianzai_weights() {
        use crate::pipeline::{SearchEngine, SearchQuery, MAX_LOOKUP_LIMIT};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::{HashMap, HashSet};
        use std::path::PathBuf;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let config = qianyan_ime_core::config::Config::load();

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let usage_history = std::sync::Arc::new(ArcSwap::new(std::sync::Arc::new(HashMap::<
            String, HashMap<String, u32>
        >::new())));

        let engine = SearchEngine::new(
            trie_paths,
            std::sync::Arc::new(HashMap::new()),
            std::sync::Arc::new(ArcSwap::new(std::sync::Arc::new(HashMap::<
                String, HashMap<String, Vec<(String, u32)>>
            >::new()))),
            usage_history.clone(),
            std::sync::Arc::new(ArcSwap::new(std::sync::Arc::new(HashMap::<
                String, HashMap<String, Vec<(String, u32)>>
            >::new()))),
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                std::sync::Arc::new(m)
            },
        );

        let query = SearchQuery {
            buffer: "xianzai",
            profile: "chinese",
            config: &config,
            limit: MAX_LOOKUP_LIMIT,
            filter_mode: crate::processor::FilterMode::None,
            aux_filter: "",
            context: None,
            fuzzy_enabled: false,
        };
        let (candidates, _) = engine.search(query);
        println!("=== xianzai debug ===");
        for (i, c) in candidates.iter().enumerate().take(20) {
            println!("  [{}] {} match_level={} weight={}", i, c.text, c.match_level, c.weight);
        }
    }

    #[test]
    fn test_has_word_in_dict() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let trie = Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");
        // "我们" is a known system dictionary word → should return true
        assert!(trie.has_word_in_dict("我们"));
        // a random nonsense word should not exist
        assert!(!trie.has_word_in_dict("qqqnotexist"));
    }

    #[test]
    fn test_comprehensive_lookup() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let mut config = qianyan_ime_core::config::Config::load();
        config.input.enable_abbreviation_matching = true;
        config.input.enable_prefix_matching = true;
        config.input.enable_fuzzy_pinyin = false;

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, u32>>::new()))),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search(engine: &SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &HashSet<String>) -> Vec<String> {
            let query = SearchQuery {
                buffer,
                profile: "chinese",
                config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().take(20).map(|c| c.text.to_string()).collect()
        }

        // zho: prefix completion → zhong → 中 first
        let r = search(&engine, &config, "zho", &syllables);
        println!("zho top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "zho should not be empty");
        assert!(r[0] == "中", "zho[0] expected 中, got {}", r[0]);

        // guor: prefix completion → guoran → 果然
        let r = search(&engine, &config, "guor", &syllables);
        println!("guor top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "guor should not be empty");

        // zm: abbreviation → 怎么/咱们 first
        let r = search(&engine, &config, "zm", &syllables);
        println!("zm top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "zm should not be empty");
        assert!(r[0] == "怎么", "zm[0] expected 怎么, got {}", r[0]);

        // sm: abbreviation → 什么 first
        let r = search(&engine, &config, "sm", &syllables);
        println!("sm top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "sm should not be empty");
        assert!(r[0] == "什么", "sm[0] expected 什么, got {}", r[0]);

        // qkun: mixed abbreviation
        let r = search(&engine, &config, "qkun", &syllables);
        println!("qkun top10: {:?}", &r[..10.min(r.len())]);

        // qmian: mixed abbreviation → 前面/全面
        let r = search(&engine, &config, "qmian", &syllables);
        println!("qmian top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "qmian should not be empty");
        assert!(
            r[0] == "全面" || r[0] == "前面",
            "qmian[0] expected 全面 or 前面, got {}", r[0]
        );

        // zhonf: typo correction → zhong → 中
        let r = search(&engine, &config, "zhonf", &syllables);
        println!("zhonf top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "zhonf should not be empty (typo correction)");
        assert!(r[0] == "中", "zhonf[0] expected 中 (corrected to zhong), got {}", r[0]);
    }

    #[test]
    fn test_rerank() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let mut config = qianyan_ime_core::config::Config::load();
        config.input.enable_abbreviation_matching = false;
        config.input.enable_prefix_matching = false;
        config.input.enable_fuzzy_pinyin = false;
        config.input.enable_fixed_first_candidate = true;

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        let usage_history = Arc::new(ArcSwap::new(Arc::new(HashMap::<
            String, HashMap<String, u32>
        >::new())));

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
            usage_history.clone(),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search_weight(engine: &SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &HashSet<String>) -> Vec<(String, f64)> {
            let query = SearchQuery {
                buffer,
                profile: "chinese",
                config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().take(20).map(|c| (c.text.to_string(), c.weight)).collect()
        }

        // Search "da" without any usage history
        let r1 = search_weight(&engine, &config, "da", &syllables);
        println!("da (no history) top5: {:?}", &r1[..5.min(r1.len())]);
        assert!(!r1.is_empty(), "da should have results");
        let da_weight_before = r1.iter().find(|(w, _)| w == "打").map(|(_, w)| *w).unwrap_or(0.0);
        println!("da '打' weight (no history): {}", da_weight_before);
        assert!(da_weight_before > 0.0, "打 should exist in results");
        assert!(r1[0].0 == "大", "Without history, 大 should be first");

        // 模拟 "打" 用了 5 次 → 权重应增加
        let mut usage: HashMap<String, HashMap<String, u32>> = HashMap::new();
        let mut da_usage: HashMap<String, u32> = HashMap::new();
        da_usage.insert("打".to_string(), 5);
        usage.insert("chinese".to_string(), da_usage);
        usage_history.store(Arc::new(usage));

        let r2 = search_weight(&engine, &config, "da", &syllables);
        let da_weight_after = r2.iter().find(|(w, _)| w == "打").map(|(_, w)| *w).unwrap_or(0.0);
        println!("da (打x5) top5: {:?}", &r2[..5.min(r2.len())]);
        println!("打 weight before={}, after={}", da_weight_before, da_weight_after);
        // Verify weight increased by approximately log2(6) * 50000 ≈ 129283
        let expected_min_boost = 129000.0;
        assert!(da_weight_after >= da_weight_before + expected_min_boost,
            "打 weight should increase by at least {} after 5 uses. Before: {}, After: {}",
            expected_min_boost, da_weight_before, da_weight_after);

        // 再给 "大" 更多使用次数
        let mut usage2: HashMap<String, HashMap<String, u32>> = HashMap::new();
        let mut da_usage2: HashMap<String, u32> = HashMap::new();
        da_usage2.insert("大".to_string(), 20);
        da_usage2.insert("打".to_string(), 3);
        usage2.insert("chinese".to_string(), da_usage2);
        usage_history.store(Arc::new(usage2));

        let r4 = search_weight(&engine, &config, "da", &syllables);
        println!("da (大x20, 打x3) top5: {:?}", &r4[..5.min(r4.len())]);
        assert!(r4[0].0 == "大", "大 (20 uses) should be first. Got: {}", r4[0].0);

        // Clean up test data
        usage_history.store(Arc::new(HashMap::new()));
    }

    #[test]
    fn test_rerank_da_da_10_times() {
        use arc_swap::ArcSwap;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let mut config = qianyan_ime_core::config::Config::load();
        config.input.enable_auto_reorder = true;
        config.input.enable_fixed_first_candidate = false;

        let mut trie_paths = std::collections::HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: std::collections::HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        let usage_history: Arc<ArcSwap<std::collections::HashMap<String, std::collections::HashMap<String, u32>>>> =
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::new())));

        let engine = crate::pipeline::SearchEngine::new(
            trie_paths,
            Arc::new(std::collections::HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            usage_history.clone(),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            {
                let mut m: std::collections::HashMap<String, Box<dyn crate::scheme::InputScheme>> = std::collections::HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search_weight(engine: &crate::pipeline::SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &std::collections::HashSet<String>) -> Vec<(String, f64)> {
            let query = crate::pipeline::SearchQuery {
                buffer,
                profile: "chinese",
                config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().take(20).map(|c| (c.text.to_string(), c.weight)).collect()
        }

        // 初始状态
        let r1 = search_weight(&engine, &config, "da", &syllables);
        println!("da (no history) top5: {:?}", &r1[..5.min(r1.len())]);
        let da_weight_before = r1.iter().find(|(w, _)| w == "打").map(|(_, w)| *w).unwrap_or(0.0);
        println!("da '打' initial weight: {}", da_weight_before);
        assert!(da_weight_before > 0.0, "打 should exist in results");

        // 模拟选中 "打" 10 次
        let mut usage: std::collections::HashMap<String, std::collections::HashMap<String, u32>> = std::collections::HashMap::new();
        let mut da_usage: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        da_usage.insert("打".to_string(), 10);
        usage.insert("chinese".to_string(), da_usage);
        usage_history.store(Arc::new(usage));

        // 验证 usage 已正确记录
        let stored = usage_history.load();
        let count = stored.get("chinese")
            .and_then(|p| p.get("打"))
            .copied()
            .unwrap_or(0);
        assert_eq!(count, 10, "打 should have usage count 10, got {}", count);

        // 重新搜索 — 验证 weight 增加
        let r2 = search_weight(&engine, &config, "da", &syllables);
        println!("da (打x10) top5: {:?}", &r2[..5.min(r2.len())]);
        let da_weight_after = r2.iter().find(|(w, _)| w == "打").map(|(_, w)| *w).unwrap_or(0.0);
        println!("da '打' weight after 10 uses: {}", da_weight_after);

        // 验证 weight 增加了 log2(11) * 50000 ≈ 172,891
        let expected_min_boost = 170000.0;
        assert!(da_weight_after >= da_weight_before + expected_min_boost,
            "打 weight should increase by at least {} after 10 uses. Before: {}, After: {}",
            expected_min_boost, da_weight_before, da_weight_after);

        // 清理
        usage_history.store(Arc::new(std::collections::HashMap::new()));
    }

    #[test]
    fn test_zho_completion() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let config = qianyan_ime_core::config::Config::load();

        let mut trie_paths = std::collections::HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: std::collections::HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(std::collections::HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, u32>
            >::new()))),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            {
                let mut m: std::collections::HashMap<String, Box<dyn InputScheme>> = std::collections::HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search(engine: &SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &std::collections::HashSet<String>) -> Vec<String> {
            let query = SearchQuery {
                buffer,
                profile: "chinese",
                config: &config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().take(20).map(|c| c.text.to_string()).collect()
        }

        let r = search(&engine, &config, "zho", &syllables);
        assert!(r[0] == "中", "First result for 'zho' should be 中, got {}", r[0]);
    }

    #[test]
    fn test_trie_result_clone() {
        let result = TrieResult {
            word: "test",
            trad: "測試",
            tone: "tēst",
            en: "test",
            stroke_aux: "",
            weight: 100,
            flags: 0,
        };
        let cloned = result;
        assert_eq!(result.word, cloned.word);
        assert_eq!(result.weight, cloned.weight);
    }

    #[test]
    fn test_trie_data_memory() {
        let data = vec![1u8, 2, 3, 4];
        let trie_data = TrieData::Memory(Arc::new(data.clone()));
        assert_eq!(trie_data.as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_trie_result_copy() {
        let result = TrieResult {
            word: "hello",
            trad: "hello",
            tone: "",
            en: "hello",
            stroke_aux: "",
            weight: 0,
            flags: 0,
        };
        assert_eq!(result.word, "hello");
        assert_eq!(result.weight, 0);
    }

    #[test]
    fn test_abbreviation_user_dict_priority() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let mut config = qianyan_ime_core::config::Config::load();
        config.input.enable_abbreviation_matching = true;
        config.input.enable_prefix_matching = true;
        config.input.enable_fuzzy_pinyin = false;

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        // 在用户词典中注入全拼词 "shenme" → "什么可以"
        // (无元音的简拼键如 "sm" 不会被查到——那是给简拼策略处理的)
        let learned_words: Arc<ArcSwap<HashMap<String, HashMap<String, Vec<(String, u32)>>>>> =
            Arc::new(ArcSwap::new(Arc::new({
                let mut m = HashMap::new();
                let mut sm = HashMap::new();
                sm.insert("shenme".to_string(), vec![("什么可以".to_string(), 10)]);
                m.insert("chinese".to_string(), sm);
                m
            })));

        let usage_history = Arc::new(ArcSwap::new(Arc::new(HashMap::new())));
        let ngram_history = Arc::new(ArcSwap::new(Arc::new(HashMap::new())));

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(HashMap::new()),
            learned_words,
            usage_history,
            ngram_history,
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search(engine: &SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &HashSet<String>) -> Vec<String> {
            let query = SearchQuery {
                buffer,
                profile: "chinese",
                config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().map(|c| c.text.to_string()).collect()
        }

        // "sm" 简拼（无元音）→ 什么 排第一（简拼策略），用户词不会被查到
        let r = search(&engine, &config, "sm", &syllables);
        println!("sm top10: {:?}", &r[..10.min(r.len())]);
        assert!(!r.is_empty(), "sm should not be empty");
        assert_eq!(r[0], "什么", "sm[0] = 什么 (abbreviation), got {}", r[0]);

        // "shenme" 全拼 → 用户词 "什么可以" 精确匹配
        let r2 = search(&engine, &config, "shenme", &syllables);
        println!("shenme top5: {:?}", &r2[..5.min(r2.len())]);
        assert!(r2.iter().any(|w| w == "什么可以"), "什么可以 (exact user dict match) should appear");

        // "zm" 简拼 → 怎么 正常匹配
        let r3 = search(&engine, &config, "zm", &syllables);
        println!("zm top10: {:?}", &r3[..10.min(r3.len())]);
        assert!(!r3.is_empty());
        assert_eq!(r3[0], "怎么", "zm[0] = 怎么, got {}", r3[0]);
    }

    #[test]
    fn test_user_dict_prefix_matching() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let mut config = qianyan_ime_core::config::Config::load();
        config.input.enable_abbreviation_matching = true;
        config.input.enable_prefix_matching = true;
        config.input.enable_fuzzy_pinyin = false;

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        // 用户词典里加一个全拼词 "houxuanchuang" → "候选窗"
        let learned_words: Arc<ArcSwap<HashMap<String, HashMap<String, Vec<(String, u32)>>>>> =
            Arc::new(ArcSwap::new(Arc::new({
                let mut m = HashMap::new();
                let mut hxc = HashMap::new();
                hxc.insert("houxuanchuang".to_string(), vec![("候选窗".to_string(), 5)]);
                m.insert("chinese".to_string(), hxc);
                m
            })));

        let usage_history = Arc::new(ArcSwap::new(Arc::new(HashMap::new())));
        let ngram_history = Arc::new(ArcSwap::new(Arc::new(HashMap::new())));

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(HashMap::new()),
            learned_words,
            usage_history,
            ngram_history,
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search(engine: &SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &HashSet<String>) -> Vec<String> {
            let query = SearchQuery {
                buffer,
                profile: "chinese",
                config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().map(|c| c.text.to_string()).collect()
        }

        // 全拼前缀 "houxuanch" (含元音) → 应该前缀匹配 "houxuanchuang" → 候选窗
        let r = search(&engine, &config, "houxuanch", &syllables);
        println!("houxuanch results: {:?}", &r[..10.min(r.len())]);
        assert!(r.iter().any(|w| w == "候选窗"), "候选窗 should appear via user dict prefix match for 'houxuanch'");

        // 纯声母/简拼 "hx" (无元音) → 不应该走用户词前缀匹配
        let r2 = search(&engine, &config, "hx", &syllables);
        println!("hx results: {:?}", &r2[..10.min(r2.len())]);
        // "hx" 可能匹配到系统词，但不应该匹配到用户词 "候选窗"
        // (因为 "hx" 不含元音，不会触发用户词典查询)
    }

    #[test]
    fn test_user_prefix_can_compete_with_system_prefix() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::{HashMap, HashSet};
        use std::sync::Arc;

        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());
        let mut config = qianyan_ime_core::config::Config::load();
        config.input.enable_abbreviation_matching = true;
        config.input.enable_prefix_matching = true;
        config.input.enable_fuzzy_pinyin = false;
        config.input.enable_auto_reorder = false;
        config.input.enable_fixed_first_candidate = false;

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let syllables: HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllable_freq.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        // 用户词典注入 erqie→而去切（通过 erq 前缀匹配到）
        let learned_words: Arc<ArcSwap<HashMap<String, HashMap<String, Vec<(String, u32)>>>>> =
            Arc::new(ArcSwap::new(Arc::new({
                let mut m = HashMap::new();
                let mut erq = HashMap::new();
                erq.insert("erqie".to_string(), vec![("而去切".to_string(), 9999)]);
                m.insert("chinese".to_string(), erq);
                m
            })));

        // 用法历史（以词为单位）
        let usage_history: Arc<ArcSwap<HashMap<String, HashMap<String, u32>>>> =
            Arc::new(ArcSwap::new(Arc::new({
                let mut m = HashMap::new();
                let mut profile_usage = HashMap::new();
                profile_usage.insert("而去切".to_string(), 100);
                m.insert("chinese".to_string(), profile_usage);
                m
            })));

        let ngram_history = Arc::new(ArcSwap::new(Arc::new(HashMap::new())));

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(HashMap::new()),
            learned_words,
            usage_history,
            ngram_history,
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        fn search(engine: &SearchEngine, config: &qianyan_ime_core::Config, buffer: &str, syl: &HashSet<String>) -> Vec<(String, u8)> {
            let query = SearchQuery {
                buffer,
                profile: "chinese",
                config,
                limit: crate::pipeline::MAX_LOOKUP_LIMIT,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            candidates.iter().map(|c| (c.text.to_string(), c.match_level)).collect()
        }

        // "erq" → 两个词都应出现，且用户词可能排在系统词前面
        let r = search(&engine, &config, "erq", &syllables);
        println!("erq results: {:?}", &r[..10.min(r.len())]);
        assert!(r.iter().any(|(w, _)| w == "而且"), "而且 (system prefix) should appear");
        assert!(r.iter().any(|(w, _)| w == "而去切"), "而去切 (user prefix) should appear");
    }

    #[test]
    fn test_rare_char_flags_in_trie() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();
        let trie = Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");

        // 检查 li 拼音的条目，统计 flag=1 的生僻字
        let results = trie.get_all_exact("li").expect("li should exist");
        let total = results.len();
        let rare_count = results.iter().filter(|r| r.flags & 1 != 0).count();
        let common_count = results.iter().filter(|r| r.flags & 1 == 0).count();
        println!("li: total={}, rare={}, common={}", total, rare_count, common_count);

        // 检查几个已知的生僻字
        let rare_examples = vec!["孋", "悧", "蒚", "唎"];
        for ex in &rare_examples {
            if let Some(r) = results.iter().find(|r| r.word == *ex) {
                println!("  {}: weight={}, flags={:#04x}", ex, r.weight, r.flags);
                assert_eq!(r.flags & 1, 1, "{} should have rare flag set", ex);
            }
        }

        // 确保生僻字数量合理（至少有 level4/level5 的条目）
        assert!(rare_count > 0, "Should have at least some rare characters with flags=1 for 'li'");
    }

    #[test]
    fn test_rare_char_search_engine_filter() {
        use crate::pipeline::{SearchEngine, SearchQuery};
        use crate::scheme::InputScheme;
        use arc_swap::ArcSwap;
        use std::collections::HashMap;
        use std::sync::Arc;

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().unwrap().parent().unwrap();

        let config_path = root.join("configs");
        std::env::set_var("QIANYAN_CONFIG_DIR", config_path.to_str().unwrap());

        let mut trie_paths = HashMap::new();
        trie_paths.insert("chinese".to_string(), (
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
        ));

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, u32>>::new()))),
            Arc::new(ArcSwap::new(Arc::new(HashMap::<String, HashMap<String, Vec<(String, u32)>>>::new()))),
            {
                let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
                m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
                Arc::new(m)
            },
        );

        let base_cfg = qianyan_ime_core::config::Config::load();
        println!(
            "base_cfg.rare_char_mode={:?}",
            base_cfg.input.rare_char_mode,
        );

        // Test 3 modes
        for (label, mode) in [
            ("CommonOnly", qianyan_ime_core::config::RareCharMode::CommonOnly),
            ("IncludeRare", qianyan_ime_core::config::RareCharMode::IncludeRare),
            ("OnlyRare", qianyan_ime_core::config::RareCharMode::OnlyRare),
        ] {
            let mut cfg = base_cfg.clone();
            cfg.input.rare_char_mode = mode;
            cfg.input.enable_prefix_matching = false;
            cfg.input.enable_abbreviation_matching = false;
            cfg.input.enable_error_correction = false;

            let query = SearchQuery {
                buffer: "li",
                profile: "chinese",
                config: &cfg,
                limit: 3000,
                filter_mode: crate::processor::FilterMode::None,
                aux_filter: "",
                context: None,
                fuzzy_enabled: false,
            };
            let (candidates, _) = engine.search(query);
            let total = candidates.len();
            let rare = candidates.iter().filter(|c| c.flags & 1 != 0).count();
            let common = candidates.iter().filter(|c| c.flags & 1 == 0).count();
            println!("{}: total={} rare={} common={}", label, total, rare, common);
        }
    }
}
