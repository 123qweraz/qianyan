use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

// 模拟项目环境
mod engine {
    pub mod trie;
    pub mod pipeline;
    pub mod config_manager;
    pub mod processor;
    pub mod keys;
    pub mod scheme;
    pub mod schemes;
    pub mod compositor;
}
mod config;

use crate::engine::pipeline::{SearchEngine, SearchQuery};
use crate::engine::processor::FilterMode;

fn main() {
    let trie_index = PathBuf::from("dicts/chinese/chars/index.fst");
    let trie_data = PathBuf::from("dicts/chinese/chars/data.bin");
    
    // 实际项目中可能有多个词库，我们尝试加载最主要的
    let mut trie_paths = HashMap::new();
    trie_paths.insert("chinese".to_string(), (
        PathBuf::from("dicts/chinese/words/index.fst"),
        PathBuf::from("dicts/chinese/words/data.bin"),
    ));

    let syllables_content = std::fs::read_to_string("dicts/chinese/syllables.txt").expect("Failed to read syllables");
    let syllables: HashSet<String> = syllables_content.lines().map(|s| s.trim().to_string()).collect();

    let config = crate::config::Config::default();
    let user_dict = Arc::new(arc_swap::ArcSwap::from_pointee(HashMap::new()));
    
    let engine = SearchEngine::new(
        trie_paths,
        Arc::new(syllables.clone()),
        user_dict,
        HashMap::new(),
    );

    println!("--- 正在调试 'sm' 的搜索逻辑 ---");
    let query = SearchQuery {
        buffer: "sm",
        profile: "chinese",
        syllables: &syllables,
        config: &config,
        limit: 10,
        filter_mode: FilterMode::None,
        aux_filter: "",
    };

    let (candidates, _) = engine.search(query);

    for (i, cand) in candidates.iter().enumerate() {
        println!("{}. [{}] weight: {:.1}, source: {}, hint: {}", 
            i + 1, cand.text, cand.weight, cand.source, cand.hint);
    }
}
