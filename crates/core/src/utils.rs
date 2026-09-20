use std::path::{Path, PathBuf};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use serde_json::Value;
use crate::config::PunctuationEntry;

/// 进程级“数据根目录”覆盖。启动早期由 main 依据配置里的
/// `files.data_dir` 覆盖设置一次；此后所有 `find_project_root()` 调用
/// 统一走这里，子模块无需各自传参，天然保持一致。
static PROJECT_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// 记录进程级数据根目录（仅首次生效，重复调用忽略）
pub fn set_project_root(root: PathBuf) {
    let _ = PROJECT_ROOT.set(root);
}

/// 返回已设置的数据根目录覆盖（未设置则为 None）
pub(crate) fn project_root_override() -> Option<&'static PathBuf> {
    PROJECT_ROOT.get()
}

/// 从注释源文件路径解析表名（稳定、可跨会话）：
/// 1) 若该路径出现在 named（annotation_sources 的“表名→文件”映射）的值中 → 直接用其键名；
/// 2) 文件名主干（不含扩展名）为 ASCII 的 `词-词` 形式（如 zh-jp.tsv）→ 主干即表名；
/// 3) 其余情况（如中文名文件）→ 用路径 FNV-1a 稳定哈希 `ext_<16位hex>`。
pub fn annotation_table_name_for_path<K, V, I>(path: &str, named: I) -> String
where
    K: AsRef<str>,
    V: AsRef<str>,
    I: IntoIterator<Item = (K, V)>,
{
    for (table, src) in named {
        if src.as_ref() == path {
            return table.as_ref().to_string();
        }
    }
    if let Some(stem) = Path::new(path).file_stem().and_then(|s| s.to_str()) {
        let ok = !stem.is_empty()
            && stem.contains('-')
            && stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if ok {
            return stem.to_ascii_lowercase();
        }
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("ext_{:016x}", h)
}

pub fn find_project_root() -> PathBuf {
    // 已覆盖数据根目录（如配置 files.data_dir）时直接返回
    if let Some(p) = PROJECT_ROOT.get() {
        return p.clone();
    }
    // Helper: 检查路径是否为合法的项目根目录（必须有编译后的 trie 词典）
    fn is_project_root(p: &Path) -> bool {
        // 优先以源词典目录为标记（全新检出 data/ 尚不存在时也能定位），
        // 兼容老的 data 标记（打包产物）。
        p.join("dicts").exists() || p.join("data/chinese/trie.index").exists()
    }

    // 1. 检查可执行文件同级目录 (适用于绿色版/便携版)
    if let Ok(mut exe_path) = env::current_exe() {
        exe_path.pop();
        if is_project_root(&exe_path) {
            return exe_path;
        }
        // 1.5 从可执行文件目录向上查找 (适用于 cargo build 的 target/release 布局)
        for _ in 0..4 {
            if !exe_path.pop() {
                break;
            }
            if is_project_root(&exe_path) {
                return exe_path;
            }
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
        if is_project_root(&curr) {
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
