use qianyan_ime_engine::pipeline::{SearchEngine, SearchQuery};
use qianyan_ime_engine::scheme::InputScheme;
use qianyan_ime_engine::schemes::{BihuaScheme, ChineseScheme, EnglishScheme, JapaneseScheme, PybiScheme, StrokeScheme};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

// 临时探针：验证运行期注释装配（青简式 join）
#[test]
fn probe_annotation_join() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let data_dir = root.join("data");
    let mut trie_paths = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let idx = path.join("trie.index");
                let dat = path.join("trie.data");
                if idx.exists() && dat.exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        trie_paths.insert(name.to_string(), (idx, dat));
                    }
                }
            }
        }
    }
    assert!(trie_paths.contains_key("chinese"), "chinese trie missing");
    assert!(data_dir.join("annotations/zh-en.index").exists(), "zh-en annotation missing");
    assert!(data_dir.join("annotations/en-zh.index").exists(), "en-zh annotation missing");
    assert!(data_dir.join("annotations/jp-en.index").exists(), "jp-en annotation missing");

    let mut schemes: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
    schemes.insert("chinese".into(), Box::new(ChineseScheme::new()));
    schemes.insert("english".into(), Box::new(EnglishScheme::new()));
    schemes.insert("japanese".into(), Box::new(JapaneseScheme::new()));
    schemes.insert("stroke".into(), Box::new(StrokeScheme::new()));
    schemes.insert("bihua".into(), Box::new(BihuaScheme::new()));
    schemes.insert("pybi".into(), Box::new(PybiScheme::new()));

    let engine = SearchEngine::new(
        trie_paths,
        Arc::new(HashMap::new()),
        Arc::new(arc_swap::ArcSwap::from_pointee(HashMap::new())),
        Arc::new(arc_swap::ArcSwap::from_pointee(HashMap::new())),
        Arc::new(arc_swap::ArcSwap::from_pointee(HashMap::new())),
        Arc::new(schemes),
    );

    let config = qianyan_ime_core::Config::default_config();

    // 中文方案："pingguo" 应装配出 english_aux=Apple
    let q = SearchQuery {
        buffer: "pingguo",
        profile: "chinese",
        config: &config,
        limit: 20,
        filter_mode: qianyan_ime_engine::processor::FilterMode::None,
        aux_filter: "",
        context: None,
        context_pair: None,
    };
    let (cands, _) = engine.search(q);
    let pingguo = cands.iter().find(|c| c.simplified.as_ref() == "苹果");
    assert!(pingguo.is_some(), "pingguo 未命中苹果");
    let pc = pingguo.unwrap();
    println!("[chinese] 苹果 english_aux={:?} hint={:?}", pc.english_aux, pc.hint);
    // annotation_lang 默认 en → 应 join 出 Apple
    assert_eq!(pc.english_aux.as_ref(), "Apple");

    // english 方案强制 zh：'apple' 应装配出中文注释
    let q2 = SearchQuery {
        buffer: "apple",
        profile: "english",
        config: &config,
        limit: 20,
        filter_mode: qianyan_ime_engine::processor::FilterMode::None,
        aux_filter: "",
        context: None,
        context_pair: None,
    };
    let (cands2, _) = engine.search(q2);
    let apple = cands2.iter().find(|c| c.simplified.as_ref() == "apple");
    assert!(apple.is_some(), "apple 未命中");
    println!("[english] apple english_aux={:?} hint={:?}", apple.unwrap().english_aux, apple.unwrap().hint);

    // 英文辅助码过滤在引擎层生效：aux_filter=ap 应保留 苹果
    let q3 = SearchQuery {
        buffer: "pingguo",
        profile: "chinese",
        config: &config,
        limit: 20,
        filter_mode: qianyan_ime_engine::processor::FilterMode::Global,
        aux_filter: "app",
        context: None,
        context_pair: None,
    };
    let (cands3, _) = engine.search(q3);
    assert!(cands3.iter().any(|c| c.simplified.as_ref() == "苹果"), "辅码过滤丢失 苹果");
}