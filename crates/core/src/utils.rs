use std::path::{Path, PathBuf};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::collections::{HashMap, HashSet};
use serde_json::Value;
use crate::config::PunctuationEntry;

pub fn find_project_root() -> PathBuf {
    // 1. 检查可执行文件同级目录 (适用于绿色版/便携版)
    if let Ok(mut exe_path) = env::current_exe() {
        exe_path.pop();
        if exe_path.join("data").exists() || exe_path.join("dicts").exists() {
            return exe_path;
        }
    }

    // 2. 检查 Linux 系统标准路径 (适用于 .deb 安装版)
    #[cfg(target_os = "linux")]
    {
        let share_path = PathBuf::from("/usr/share/qianyan-ime");
        if share_path.exists() {
            return share_path;
        }
    }

    // 3. 检查当前工作目录及其父目录 (适用于开发环境)
    let mut curr = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for _ in 0..4 {
        if curr.join("dicts").exists() || curr.join("data").exists() {
            return curr;
        }
        if !curr.pop() {
            break;
        }
    }
    
    // 默认兜底到当前目录
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn load_punctuation_dict(p: &str) -> HashMap<String, Vec<PunctuationEntry>> {
    let mut m = HashMap::new();
    if let Ok(f) = File::open(p) {
        if let Ok(v) = serde_json::from_reader::<_, Value>(BufReader::new(f)) {
            if let Some(obj) = v.as_object() {
                for (k, val) in obj {
                    if let Some(arr) = val.as_array() {
                        let entries = arr
                            .iter()
                            .filter_map(|item| {
                                let c = item.get("char")?.as_str()?;
                                let d = item.get("desc").and_then(|d| d.as_str()).unwrap_or("");
                                Some(PunctuationEntry {
                                    char: c.to_string(),
                                    desc: d.to_string(),
                                })
                            })
                            .collect();
                        m.insert(k.clone(), entries);
                    }
                }
            }
        }
    }
    m
}

pub fn load_syllable_frequencies(root: &Path) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    let path = root.join("dicts/chinese/syllable_freq.txt");
    if let Ok(f) = File::open(&path) {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(f);
        for line in reader.lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((pinyin, count_str)) = line.split_once(' ') {
                if let Ok(count) = count_str.parse::<u64>() {
                    map.insert(pinyin.to_lowercase(), count);
                }
            }
        }
    }
    map
}

/// 从 level4.json 和 level5.json 提取生僻字集合
pub fn load_rare_chars(root: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    for filename in &["level4.json", "level5.json"] {
        let path = root.join("dicts/chinese/chars").join(filename);
        if let Ok(f) = File::open(&path) {
            if let Ok(v) = serde_json::from_reader::<_, Value>(BufReader::new(f)) {
                if let Some(obj) = v.as_object() {
                    for entries in obj.values() {
                        if let Some(arr) = entries.as_array() {
                            for entry in arr {
                                if let Some(ch) = entry.get("char").and_then(|c| c.as_str()) {
                                    set.insert(ch.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    set
}

pub fn load_single_syllables(root: &Path) -> HashSet<String> {
    let mut set = HashSet::new();

    // 优先读取缓存的 single_syllables.txt（免去解析 5 个 JSON）
    let cached = root.join("dicts/chinese/single_syllables.txt");
    if let Ok(f) = File::open(&cached) {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(f);
        for line in reader.lines().map_while(Result::ok) {
            let s = line.trim().to_lowercase();
            if !s.is_empty() {
                set.insert(s);
            }
        }
        if !set.is_empty() {
            return set;
        }
    }

    // 兜底：逐层解析 JSON
    for filename in &["level1.json", "level2.json", "level3.json", "level4.json", "level5.json"] {
        let path = root.join("dicts/chinese/chars").join(filename);
        if let Ok(f) = File::open(&path) {
            if let Ok(v) = serde_json::from_reader::<_, Value>(BufReader::new(f)) {
                if let Some(obj) = v.as_object() {
                    for key in obj.keys() {
                        set.insert(key.to_lowercase());
                    }
                }
            }
        }
    }
    set
}
