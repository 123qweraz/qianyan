use std::path::{Path, PathBuf};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::collections::{HashMap, HashSet};
use serde_json::Value;
use crate::config::PunctuationEntry;

pub fn find_project_root() -> PathBuf {
    if let Ok(mut exe_path) = env::current_exe() {
        exe_path.pop();
        if exe_path.join("dicts").exists() {
            return exe_path;
        }
    }

    let mut curr = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for _ in 0..3 {
        if curr.join("dicts").exists() {
            return curr;
        }
        if !curr.pop() {
            break;
        }
    }
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

pub fn load_syllables(root: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    let path = root.join("dicts/chinese/syllables.txt");
    if let Ok(f) = File::open(&path) {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(f);
        for line in reader.lines().map_while(Result::ok) {
            let s = line.trim().to_lowercase();
            if !s.is_empty() {
                set.insert(s);
            }
        }
    }
    set
}
