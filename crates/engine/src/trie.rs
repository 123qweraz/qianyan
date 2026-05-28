use fst::{Automaton, IntoStreamer, Map, Streamer};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

const ABBREVIATION_SCAN_LIMIT: usize = 3000;

#[derive(Clone, Copy)]
pub struct TrieResult<'a> {
    pub word: &'a str,
    pub trad: &'a str,
    pub tone: &'a str,
    pub en: &'a str,
    pub stroke_aux: &'a str,
    pub weight: u32,
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
                let mmap = unsafe { Mmap::map(&file)? };
                Ok(TrieData::Mmap(Arc::new(mmap)))
            }
        };

        let index_data = load_data(index_path.as_ref())?;
        let data_data = load_data(data_path.as_ref())?;
        let index = Map::new(index_data)?;

        Ok(Self {
            index,
            data: data_data,
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

        // 按 weight 降序排列，取前 limit 条（避免存储顺序导致高权重词被遗漏）
        results.sort_by(|a, b| b.weight.cmp(&a.weight));
        results.truncate(limit);
        results
    }

    /// 通配符搜索实现：z 匹配任意单个 a-y 字母
    pub fn search_wildcard(&self, pattern: &str, limit: usize) -> Vec<TrieResult<'_>> {
        self.search_wildcard_with_level_filter(pattern, limit, None)
    }

    /// 带等级过滤的通配符搜索
    pub fn search_wildcard_with_level_filter(
        &self,
        pattern: &str,
        limit: usize,
        level_filter: Option<&str>,
    ) -> Vec<TrieResult<'_>> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // 简单的 DFS 实现通配符匹配
        let mut stream = self.index.stream();
        while let Some((key_bytes, offset)) = stream.next() {
            let key = String::from_utf8_lossy(key_bytes);
            if self.wildcard_match(pattern, &key) {
                let mut stop = false;
                self.read_block(offset as usize, |pair| {
                    if !stop && seen.insert(pair.word) {
                        // 应用等级过滤
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

    fn wildcard_match(&self, pattern: &str, key: &str) -> bool {
        let p_chars: Vec<char> = pattern.chars().collect();
        let k_chars: Vec<char> = key.chars().collect();

        // 如果 pattern 不包含通配符且不是 key 的前缀，快速失败
        if !pattern.contains('z') {
            return key.starts_with(pattern);
        }

        // 简易正则逻辑：z 匹配任意 1 个字符
        if p_chars.len() > k_chars.len() {
            return false;
        }

        for i in 0..p_chars.len() {
            if p_chars[i] != 'z' && p_chars[i] != k_chars[i] {
                return false;
            }
        }
        true
    }

    pub fn search_abbreviation(
        &self,
        segments: &[String],
        syllables: &std::collections::HashSet<String>,
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

            // 严格匹配：
            // 1. 每一个 segment 必须匹配一个音节的开头
            // 2. 必须刚好匹配完所有 segment 且 耗尽 key 中的所有音节
            if self.matches_strict_jianpin(&key, segments, syllables) {
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
        syllables: &std::collections::HashSet<String>,
    ) -> bool {
        self.recursive_strict_match(key, segments, syllables)
    }

    fn recursive_strict_match(
        &self,
        key: &str,
        segments: &[String],
        syllables: &std::collections::HashSet<String>,
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
                if syllables.contains(syl) {
                    // 声母必须匹配
                    if syl.starts_with(first_seg) {
                        // 递归匹配剩余音节
                        if self.recursive_strict_match(&key[len..], &segments[1..], syllables) {
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
        if syllables.contains(key) && key.starts_with(first_seg) && segments.len() == 1 {
            return true;
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
                .expect("read_block: failed to read count at offset"),
        );
        let mut cursor = offset + 4;

        for _ in 0..count {
            if cursor + 2 > data.len() {
                log::warn!("[Trie] read_block: truncated length field at cursor {}", cursor);
                break;
            }
            let w_len = u16::from_le_bytes(
                data[cursor..cursor + 2]
                    .try_into()
                    .expect("read_block: failed to read word length"),
            ) as usize;
            cursor += 2;
            if cursor + w_len > data.len() {
                log::warn!("[Trie] read_block: truncated word data at cursor {}", cursor);
                break;
            }
            let word = match std::str::from_utf8(&data[cursor..cursor + w_len]) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Trie] read_block: invalid utf8 for word: {}", e);
                    break;
                }
            };
            cursor += w_len;

            if cursor + 2 > data.len() {
                log::warn!("[Trie] read_block: truncated trad length at cursor {}", cursor);
                break;
            }
            let tr_len = u16::from_le_bytes(
                data[cursor..cursor + 2]
                    .try_into()
                    .expect("read_block: failed to read trad length"),
            ) as usize;
            cursor += 2;
            if cursor + tr_len > data.len() {
                log::warn!("[Trie] read_block: truncated trad data at cursor {}", cursor);
                break;
            }
            let trad = match std::str::from_utf8(&data[cursor..cursor + tr_len]) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Trie] read_block: invalid utf8 for trad: {}", e);
                    break;
                }
            };
            cursor += tr_len;

            if cursor + 2 > data.len() {
                log::warn!("[Trie] read_block: truncated tone length at cursor {}", cursor);
                break;
            }
            let t_len = u16::from_le_bytes(
                data[cursor..cursor + 2]
                    .try_into()
                    .expect("read_block: failed to read tone length"),
            ) as usize;
            cursor += 2;
            if cursor + t_len > data.len() {
                log::warn!("[Trie] read_block: truncated tone data at cursor {}", cursor);
                break;
            }
            let tone = match std::str::from_utf8(&data[cursor..cursor + t_len]) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Trie] read_block: invalid utf8 for tone: {}", e);
                    break;
                }
            };
            cursor += t_len;

            if cursor + 2 > data.len() {
                log::warn!("[Trie] read_block: truncated en length at cursor {}", cursor);
                break;
            }
            let e_len = u16::from_le_bytes(
                data[cursor..cursor + 2]
                    .try_into()
                    .expect("read_block: failed to read en length"),
            ) as usize;
            cursor += 2;
            if cursor + e_len > data.len() {
                log::warn!("[Trie] read_block: truncated en data at cursor {}", cursor);
                break;
            }
            let en = match std::str::from_utf8(&data[cursor..cursor + e_len]) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Trie] read_block: invalid utf8 for en: {}", e);
                    break;
                }
            };
            cursor += e_len;

            if cursor + 2 > data.len() {
                log::warn!("[Trie] read_block: truncated stroke_aux length at cursor {}", cursor);
                break;
            }
            let s_len = u16::from_le_bytes(
                data[cursor..cursor + 2]
                    .try_into()
                    .expect("read_block: failed to read stroke_aux length"),
            ) as usize;
            cursor += 2;
            if cursor + s_len > data.len() {
                log::warn!("[Trie] read_block: truncated stroke_aux data at cursor {}", cursor);
                break;
            }
            let stroke_aux = match std::str::from_utf8(&data[cursor..cursor + s_len]) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Trie] read_block: invalid utf8 for stroke_aux: {}", e);
                    break;
                }
            };
            cursor += s_len;

            if cursor + 4 > data.len() {
                log::warn!("[Trie] read_block: truncated weight at cursor {}", cursor);
                break;
            }
            let weight = u32::from_le_bytes(
                data[cursor..cursor + 4]
                    .try_into()
                    .expect("read_block: failed to read weight"),
            );
            cursor += 4;

            f(TrieResult {
                word,
                trad,
                tone,
                en,
                stroke_aux,
                weight,
            });
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

        let syllables: std::collections::HashSet<String> = {
            let content = std::fs::read_to_string(root.join("dicts/chinese/syllables.txt")).unwrap();
            content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        };

        let engine = SearchEngine::new(
            trie_paths,
            Arc::new(syllables),
            Arc::new(std::collections::HashMap::new()),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
            >::new()))),
            Arc::new(ArcSwap::new(Arc::new(std::collections::HashMap::<
                String, std::collections::HashMap<String, Vec<(String, u32)>>
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
            syllables: &std::collections::HashSet::new(),
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
        assert!(count > 40,
            "Only {} candidates for 'li' — expected >40! The result limit is still too aggressive.",
            count);

        // Also verify the trie itself has 80+ entries for "li"
        let trie = Trie::load(
            root.join("data/chinese/trie.index"),
            root.join("data/chinese/trie.data"),
            true,
        ).expect("Failed to load trie");
        let trie_count = trie.get_all_exact("li").map(|v| v.len()).unwrap_or(0);
        println!("'li' exact trie entries: {}", trie_count);
        assert!(trie_count > 50, "Only {} entries in trie for 'li'!", trie_count);
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
        };
        assert_eq!(result.word, "hello");
        assert_eq!(result.weight, 0);
    }
}
