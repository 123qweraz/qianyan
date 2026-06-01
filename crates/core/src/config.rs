use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
    pub files: Files,
    pub appearance: Appearance,
    pub input: Input,
    pub hotkeys: Hotkeys,
    pub enable_quick_finals: bool,
    pub quick_finals: Vec<QuickFinal>,
    pub punctuations: std::collections::HashMap<
        String,
        std::collections::HashMap<String, Vec<PunctuationEntry>>,
    >,
    pub layouts: std::collections::HashMap<String, ProfileLayout>,
    #[cfg(target_os = "linux")]
    pub linux: LinuxConfig,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LinuxConfig {
    pub device_path: String,
    pub paste_method: String,
    pub clipboard_delay_ms: u64,
    #[serde(default = "default_true")]
    pub show_slint_window: bool,
    #[serde(default)]
    pub show_notification: bool,
    #[serde(default)]
    pub show_toggle_notification: bool,
    pub fixed_position: bool,
    pub corner: String, // "top-left", "top-right", "bottom-left", "bottom-right"
    #[serde(default = "default_fixed_x")]
    pub fixed_x: i32,
    #[serde(default = "default_fixed_y")]
    pub fixed_y: i32,
}

fn default_true() -> bool {
    true
}

fn default_fixed_x() -> i32 {
    40
}

fn default_fixed_y() -> i32 {
    40
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Files {
    pub data_dir: Option<String>,
    pub punctuation_file: String,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    CharacterWithEnglish,
    CharacterOnly,
    CharacterWithStroke,
    CharacterWithTone,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxFilterMode {
    #[default]
    English,
    None,
    Stroke,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Appearance {
    pub show_candidates: bool,
    pub page_size: usize,
    pub candidate_anchor: String,
    pub candidate_layout: String,
    pub corner_radius: f32,
    pub window_bg_color: String,
    pub window_highlight_color: String,
    pub window_highlight_text_color: String,
    pub window_border_color: String,
    pub window_padding_x: i32,
    pub window_padding_y: i32,
    pub item_spacing: f32,
    pub row_spacing: f32,
    pub theme_mode: String,
    pub pinyin_text: TextStyle,
    pub candidate_text: TextStyle,
    pub hint_text: TextStyle,
    pub comment_text: TextStyle,
    pub enable_random_highlight: bool,
    pub show_learning_stroke_hint: bool,
    pub show_learning_english_hint: bool,
    pub auto_pronounce: bool,
    #[serde(default = "default_show_tray")]
    pub show_tray: bool,
}

fn default_show_tray() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct TextStyle {
    pub font_family: String,
    pub font_size: u32,
    pub font_weight: u32,
    pub color: String,
    pub alpha: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum AntiTypoMode {
    None,
    Strict,
    Smart,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum PhantomType {
    None,
    Hanzi,
    Pinyin,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct KeyAction {
    pub tap: String,
    #[serde(default)]
    pub shift: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_tap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_press: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ProfileLayout {
    pub name: String,
    pub mappings: std::collections::HashMap<String, KeyAction>,
}



#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum EnglishAuxMode {
    Prefix,      // 前缀匹配
    FirstLetter, // 仅首字母
}

fn default_english_aux_mode() -> EnglishAuxMode {
    EnglishAuxMode::Prefix
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Input {
    pub autostart: bool,
    pub commit_mode: String,
    pub default_profile: String,
    pub phantom_type: PhantomType,
    pub anti_typo_mode: AntiTypoMode,
    pub enable_double_tap: bool,
    pub double_tap_timeout_ms: u64,
    pub double_taps: Vec<DoubleTap>,
    pub enable_long_press: bool,
    pub long_press_timeout_ms: u64,
    pub long_press_mappings: Vec<LongPressMapping>,
    pub enable_punctuation_long_press: bool,
    pub punctuation_long_press_mappings: std::collections::HashMap<String, String>,
    pub keyboard_layouts:
        std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    pub auto_commit_unique_en_fuzhuma: bool,
    pub auto_commit_unique_full_match: bool,
    pub auto_commit_stroke: bool,
    pub enable_prefix_matching: bool,
    pub prefix_matching_limit: usize,
    pub enable_abbreviation_matching: bool,
    pub filter_proper_nouns_by_case: bool,
    pub enabled_profiles: Vec<String>,
    pub profile_keys: Vec<ProfileKey>,
    pub swap_arrow_keys: bool,
    pub enable_error_sound: bool,
    pub enable_keyboard_voice: bool,
    pub enable_english_filter: bool,
    pub enable_caps_selection: bool,
    pub enable_number_selection: bool,
    pub enable_word_discovery: bool,
    pub enable_auto_reorder: bool,
    pub enable_fixed_first_candidate: bool,
    pub enable_smart_backspace: bool,
    pub enable_double_pinyin: bool,
    pub double_pinyin_scheme: DoublePinyinScheme,
    pub enable_fuzzy_pinyin: bool,
    pub fuzzy_config: FuzzyPinyinConfig,
    pub enable_traditional: bool,
    #[serde(default = "default_english_aux_mode")]
    pub english_aux_mode: EnglishAuxMode,
    #[serde(default)]
    pub display_mode: DisplayMode,
    pub ranking: RankingConfig,
    pub firefox_space_interrupt: bool,
    #[serde(default = "default_segmentation_delimiters")]
    pub segmentation_delimiters: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RankingConfig {
    pub length_penalty: f64,
    pub user_dict_bonus: f64,
    pub exact_match_bonus: f64,
    pub single_char_bonus: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct PunctuationEntry {
    pub char: String,
    pub desc: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FuzzyPinyinConfig {
    pub z_zh: bool,
    pub c_ch: bool,
    pub s_sh: bool,
    pub n_l: bool,
    pub r_l: bool,
    pub f_h: bool,
    pub an_ang: bool,
    pub en_eng: bool,
    pub in_ing: bool,
    pub ian_iang: bool,
    pub uan_uang: bool,
    pub u_v: bool,
    pub custom_mappings: Vec<(String, String)>,
    /// 翻页多少次后激活模糊音（0=始终激活，N=翻页N次后激活）
    pub fuzzy_page_threshold: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct DoublePinyinScheme {
    pub name: String,
    pub initials: std::collections::HashMap<String, String>,
    pub rimes: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct DoubleTap {
    pub trigger_key: String,
    pub insert_text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LongPressMapping {
    pub trigger_key: String,
    pub insert_text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ProfileKey {
    pub key: String,
    pub profile: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct QuickFinal {
    pub key: String,
    pub final_text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct QuickFinalsFile {
    pub enable_quick_finals: bool,
    pub quick_finals: Vec<QuickFinal>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Hotkeys {
    pub switch_language: Hotkey,
    #[serde(default = "default_page_up")]
    pub page_up: Vec<String>,
    #[serde(default = "default_page_down")]
    pub page_down: Vec<String>,
    #[serde(default = "default_prev_candidate")]
    pub prev_candidate: Vec<String>,
    #[serde(default = "default_next_candidate")]
    pub next_candidate: Vec<String>,
    pub enable_tab_toggle: bool,
    pub enable_ctrl_space_toggle: bool,
    pub enable_ctrl_capslock_commit: bool,
    #[serde(default = "default_toggle_traditional")]
    pub toggle_traditional: Hotkey,
    #[serde(default = "default_word_to_char")]
    pub word_to_char: Vec<String>,
    #[serde(default = "default_word_to_char_shift")]
    pub word_to_char_shift: Vec<String>,
}

fn default_word_to_char() -> Vec<String> {
    vec!["9".into(), "0".into()]
}

fn default_word_to_char_shift() -> Vec<String> {
    vec!["1".into(), "2".into()]
}

fn default_toggle_traditional() -> Hotkey {
    Hotkey {
        key: "CapsLock+F".to_string(),
        description: "繁简体切换".to_string(),
    }
}

fn default_segmentation_delimiters() -> String {
    "'".to_string()
}

fn default_page_up() -> Vec<String> {
    vec![
        "Up".into(),
        "PageUp".into(),
        "-".into(),
        ",".into(),
        "[".into(),
    ]
}
fn default_page_down() -> Vec<String> {
    vec![
        "Down".into(),
        "PageDown".into(),
        "=".into(),
        ".".into(),
        "]".into(),
    ]
}
fn default_prev_candidate() -> Vec<String> {
    vec!["Left".into()]
}
fn default_next_candidate() -> Vec<String> {
    vec!["Right".into()]
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Hotkey {
    pub key: String,
    pub description: String,
}

fn action(
    tap: &str,
    shift: &str,
    double_tap: Option<&str>,
    long_press: Option<&str>,
    description: &str,
) -> KeyAction {
    KeyAction {
        tap: tap.to_string(),
        shift: shift.to_string(),
        double_tap: double_tap.map(ToString::to_string),
        long_press: long_press.map(ToString::to_string),
        description: Some(description.to_string()),
    }
}

pub fn default_profile_layouts() -> std::collections::HashMap<String, ProfileLayout> {
    let mut layouts = std::collections::HashMap::new();

    let mut chinese = std::collections::HashMap::new();
    chinese.insert(
        ";".to_string(),
        action("；", "：", Some(";"), Some("……"), "中文分号"),
    );
    chinese.insert(
        ".".to_string(),
        action("。", "》", Some("..."), Some("·"), "中文句号"),
    );
    chinese.insert(
        ",".to_string(),
        action("，", "《", None, Some("、"), "中文逗号"),
    );
    chinese.insert("?".to_string(), action("？", "?", None, None, "中文问号"));
    chinese.insert("!".to_string(), action("！", "!", None, None, "中文叹号"));
    layouts.insert(
        "chinese".to_string(),
        ProfileLayout {
            name: "中文默认布局".into(),
            mappings: chinese,
        },
    );

    let mut english = std::collections::HashMap::new();
    english.insert(";".to_string(), action(";", ":", None, None, "英文分号"));
    english.insert(".".to_string(), action(".", ">", None, None, "英文句号"));
    english.insert(",".to_string(), action(",", "<", None, None, "英文逗号"));
    layouts.insert(
        "english".to_string(),
        ProfileLayout {
            name: "English Default Layout".into(),
            mappings: english,
        },
    );

    let mut japanese = std::collections::HashMap::new();
    japanese.insert(
        ".".to_string(),
        action("。", ">", None, Some("・"), "日文句号"),
    );
    japanese.insert(",".to_string(), action("、", "<", None, None, "日文顿号"));
    japanese.insert("/".to_string(), action("・", "?", None, None, "日文中点"));
    japanese.insert("[".to_string(), action("「", "{", None, None, "日文左引号"));
    japanese.insert("]".to_string(), action("」", "}", None, None, "日文右引号"));
    layouts.insert(
        "japanese".to_string(),
        ProfileLayout {
            name: "日本語デフォルト配列".into(),
            mappings: japanese,
        },
    );

    layouts
        .entry("stroke".to_string())
        .or_insert(ProfileLayout {
            name: "笔画默认布局".into(),
            mappings: std::collections::HashMap::new(),
        });
    layouts
}

impl Config {
    pub fn apply_theme(&mut self, dark: bool) {
        if dark {
            self.appearance.window_bg_color = "#1e1e1e".to_string();
            self.appearance.window_highlight_color = "#0078d4".to_string();
            self.appearance.window_highlight_text_color = "#ffffff".to_string();
            self.appearance.window_border_color = "rgba(255, 255, 255, 0.15)".to_string();
            self.appearance.pinyin_text.color = "#bbbbbb".to_string();
            self.appearance.candidate_text.color = "#eeeeee".to_string();
            self.appearance.hint_text.color = "#888888".to_string();
        } else {
            self.appearance.window_bg_color = "#ffffff".to_string();
            self.appearance.window_highlight_color = "#0969da".to_string();
            self.appearance.window_highlight_text_color = "#ffffff".to_string();
            self.appearance.window_border_color = "rgba(0, 0, 0, 0.1)".to_string();
            self.appearance.pinyin_text.color = "#586069".to_string();
            self.appearance.candidate_text.color = "#24292e".to_string();
            self.appearance.hint_text.color = "#6e7781".to_string();
        }
    }

    pub fn get_config_dir() -> std::path::PathBuf {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let root = if exe_dir.join("dicts").exists() {
            exe_dir.clone()
        } else {
            let mut curr = exe_dir.clone();
            for _ in 0..4 {
                if curr.join("dicts").exists() {
                    break;
                }
                if !curr.pop() {
                    break;
                }
            }
            if curr.join("dicts").exists() {
                curr
            } else {
                exe_dir
            }
        };

        root.join("configs")
    }

    // ─── Page-based JSON file helpers ───

    /// 定义每个页面 JSON 文件归属哪些顶级字段和 input/appearance 子字段。
    /// (文件名, 顶级字段, input子字段, appearance子字段)
    const PAGE_FILES: &'static [(
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        &'static [&'static str],
    )] = &[
        ("system", &["files", "linux"], &["autostart", "enabled_profiles", "phantom_type"], &["show_candidates", "show_tray"]),
        ("fuzzy", &[], &["enable_fuzzy_pinyin", "fuzzy_config"], &[]),
        ("doublepinyin", &[], &["enable_double_pinyin", "double_pinyin_scheme"], &[]),
        ("punctuation", &["punctuations"], &["enable_punctuation_long_press", "punctuation_long_press_mappings"], &[]),
    ];

    /// 深合并：将 patch 合并到 base 中（递归合并对象）
    fn deep_merge(base: &mut Value, patch: Value) {
        match (base, patch) {
            (Value::Object(base_map), Value::Object(patch_map)) => {
                for (k, v) in patch_map {
                    if let Some(existing) = base_map.get_mut(&k) {
                        if existing.is_object() && v.is_object() {
                            Self::deep_merge(existing, v);
                        } else {
                            base_map.insert(k, v);
                        }
                    } else {
                        base_map.insert(k, v);
                    }
                }
            }
            (base, patch) => *base = patch,
        }
    }

    /// 从 Value 中提取指定键的子集（用于 save 时拆分）
    fn pick_keys(value: &Value, keys: &[&str]) -> Value {
        match value {
            Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for k in keys {
                    if let Some(v) = map.get(*k) {
                        out.insert(k.to_string(), v.clone());
                    }
                }
                Value::Object(out)
            }
            _ => Value::Null,
        }
    }

    /// 从 Value 中删除指定键
    fn remove_keys(value: &mut Value, keys: &[&str]) {
        if let Value::Object(map) = value {
            for k in keys {
                map.remove(*k);
            }
        }
    }

    /// 加载单个 JSON 文件到 Value
    fn load_json(path: &std::path::Path) -> Option<Value> {
        std::fs::File::open(path).ok()
            .and_then(|f| serde_json::from_reader(std::io::BufReader::new(f)).ok())
    }

    /// 保存单个 JSON 文件
    fn save_json(path: &std::path::Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
        let f = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(f, value)?;
        Ok(())
    }

    pub fn load() -> Self {
        let config_dir = Self::get_config_dir();
        if !config_dir.exists() {
            let _ = std::fs::create_dir_all(&config_dir);
        }

        // 从默认值开始，然后被页面文件覆盖
        let mut merged = serde_json::to_value(Self::default_config())
            .unwrap_or_else(|_| Value::Null);

        // 加载新格式页面文件
        // PAGE_FILES 中的子字段归属是 save 方向用的；load 时全文件合并即可
        for &(file, ..) in Self::PAGE_FILES {
            let path = config_dir.join(format!("{}.json", file));
            if let Some(page_val) = Self::load_json(&path) {
                Self::deep_merge(&mut merged, page_val);
            }
        }

        // pinyin.json —— 包含 hotkeys + input（减去其他页的子字段）
        {
            let path = config_dir.join("pinyin.json");
            if let Some(val) = Self::load_json(&path) {
                Self::deep_merge(&mut merged, val);
            }
        }

        // appearance.json —— 包含 appearance（减去 system 页的子字段）
        {
            let path = config_dir.join("appearance.json");
            if let Some(val) = Self::load_json(&path) {
                Self::deep_merge(&mut merged, val);
            }
        }

        // quickfinals.json —— 包装格式 { enable_quick_finals, quick_finals }
        {
            let path = config_dir.join("quickfinals.json");
            if let Some(val) = Self::load_json(&path) {
                if let Ok(qf) = serde_json::from_value::<QuickFinalsFile>(val) {
                    if let Value::Object(ref mut map) = merged {
                        map.insert("enable_quick_finals".into(), Value::Bool(qf.enable_quick_finals));
                        map.insert("quick_finals".into(), serde_json::to_value(qf.quick_finals).unwrap_or_default());
                    }
                }
            }
        }

        // layout.json —— 包含 layouts
        {
            let path = config_dir.join("layout.json");
            if let Some(val) = Self::load_json(&path) {
                Self::deep_merge(&mut merged, val);
            }
        }

        // ── 向后兼容：尝试加载旧格式文件 ──
        for old in &["input.json"] {
            let path = config_dir.join(old);
            if path.exists() {
                if let Some(val) = Self::load_json(&path) {
                    Self::deep_merge(&mut merged, val);
                }
            }
        }
        // 旧文件 system 相关（已合并进 system.json）
        for old in &["files.json", "linux.json"] {
            let path = config_dir.join(old);
            if path.exists() {
                if let Some(val) = Self::load_json(&path) {
                    Self::deep_merge(&mut merged, val);
                }
            }
        }
        // 旧 layouts.json（已更名为 layout.json）
        {
            let path = config_dir.join("layouts.json");
            if path.exists() {
                if let Some(val) = Self::load_json(&path) {
                    Self::deep_merge(&mut merged, val);
                }
            }
        }
        // 旧 hotkeys.json
        {
            let path = config_dir.join("hotkeys.json");
            if path.exists() {
                if let Some(val) = Self::load_json(&path) {
                    Self::deep_merge(&mut merged, val);
                }
            }
        }

        serde_json::from_value(merged).unwrap_or_else(|e| {
            log::warn!("Config::load: 配置合并反序列化失败: {:?}，使用默认配置", e);
            Self::default_config()
        })
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        static SAVE_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        let lock = SAVE_LOCK.get_or_init(|| std::sync::Mutex::new(()));
        let _guard = lock.lock().map_err(|e| format!("Lock poisoned: {}", e))?;

        let config_dir = Self::get_config_dir();
        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir)?;
        }

        let full = serde_json::to_value(self)?;

        // 保存各页面文件
        for &(file, top_keys, input_keys, appear_keys) in Self::PAGE_FILES {
            let mut page = Value::Object(serde_json::Map::new());

            // 顶级键
            for k in top_keys {
                if let Some(v) = full.get(*k) {
                    page.as_object_mut().unwrap().insert(k.to_string(), v.clone());
                }
            }

            // input 子字段
            if !input_keys.is_empty() {
                if let Some(input_val) = full.get("input") {
                    let picked = Self::pick_keys(input_val, input_keys);
                    if picked != Value::Object(serde_json::Map::new()) {
                        page.as_object_mut().unwrap().insert("input".into(), picked);
                    }
                }
            }

            // appearance 子字段
            if !appear_keys.is_empty() {
                if let Some(appear_val) = full.get("appearance") {
                    let picked = Self::pick_keys(appear_val, appear_keys);
                    if picked != Value::Object(serde_json::Map::new()) {
                        page.as_object_mut().unwrap().insert("appearance".into(), picked);
                    }
                }
            }

            Self::save_json(&config_dir.join(format!("{}.json", file)), &page)?;
        }

        // pinyin.json: hotkeys + input（减去其他页拥有字段）
        {
            let mut page = Value::Object(serde_json::Map::new());
            if let Some(hotkeys) = full.get("hotkeys") {
                page.as_object_mut().unwrap().insert("hotkeys".into(), hotkeys.clone());
            }
            if let Some(input_val) = full.get("input") {
                let mut input_owned = input_val.clone();
                // 移除其他页拥有的 input 子字段
                for &(_, _, input_keys, _) in Self::PAGE_FILES {
                    Self::remove_keys(&mut input_owned, input_keys);
                }
                page.as_object_mut().unwrap().insert("input".into(), input_owned);
            }
            Self::save_json(&config_dir.join("pinyin.json"), &page)?;
        }

        // appearance.json: appearance（减去 system 页拥有字段）
        {
            let mut page = Value::Object(serde_json::Map::new());
            if let Some(appear_val) = full.get("appearance") {
                let mut appear_owned = appear_val.clone();
                for &(_, _, _, appear_keys) in Self::PAGE_FILES {
                    Self::remove_keys(&mut appear_owned, appear_keys);
                }
                page.as_object_mut().unwrap().insert("appearance".into(), appear_owned);
            }
            Self::save_json(&config_dir.join("appearance.json"), &page)?;
        }

        // quickfinals.json: 包装格式
        {
            let qf = QuickFinalsFile {
                enable_quick_finals: self.enable_quick_finals,
                quick_finals: self.quick_finals.clone(),
            };
            Self::save_json(&config_dir.join("quickfinals.json"), &serde_json::to_value(&qf)?)?;
        }

        // layout.json: 只包含 layouts
        {
            let mut page = Value::Object(serde_json::Map::new());
            if let Some(layouts) = full.get("layouts") {
                page.as_object_mut().unwrap().insert("layouts".into(), layouts.clone());
            }
            Self::save_json(&config_dir.join("layout.json"), &page)?;
        }

        // ── 清理旧格式文件，避免混淆 ──
        for old in &["input.json", "files.json", "linux.json", "hotkeys.json", "layouts.json", "punctuations.json"] {
            let path = config_dir.join(old);
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }

        Ok(())
    }

    pub fn default_config() -> Self {
        Config {
            files: Files {
                data_dir: None,
                punctuation_file: "dicts/chinese/punctuation.json".to_string(),
                profiles: vec![
                    Profile {
                        name: "chinese".to_string(),
                        path: "data/chinese/trie".to_string(),
                    },
                    Profile {
                        name: "english".to_string(),
                        path: "data/english/trie".to_string(),
                    },
                    Profile {
                        name: "japanese".to_string(),
                        path: "data/japanese/trie".to_string(),
                    },
                    Profile {
                        name: "stroke".to_string(),
                        path: "data/stroke/trie".to_string(),
                    },
                ],
            },
            appearance: Appearance {
                show_candidates: true,
                page_size: 5,
                candidate_anchor: "bottom".to_string(),
                candidate_layout: "horizontal".to_string(),
                corner_radius: 10.0,
                window_bg_color: "#ffffff".to_string(),
                window_highlight_color: "#0969da".to_string(),
                window_highlight_text_color: "#ffffff".to_string(),
                window_border_color: "rgba(0, 0, 0, 0.1)".to_string(),
                window_padding_x: 18,
                window_padding_y: 14,
                item_spacing: 16.0,
                row_spacing: 8.0,
                theme_mode: "auto".to_string(),
                pinyin_text: TextStyle {
                    font_family: "".to_string(),
                    font_size: 18,
                    font_weight: 400,
                    color: "#586069".to_string(),
                    alpha: 1.0,
                },
                candidate_text: TextStyle {
                    font_family: "".to_string(),
                    font_size: 18,
                    font_weight: 600,
                    color: "#24292e".to_string(),
                    alpha: 1.0,
                },
                hint_text: TextStyle {
                    font_family: "".to_string(),
                    font_size: 14,
                    font_weight: 400,
                    color: "#6e7781".to_string(),
                    alpha: 0.8,
                },
                comment_text: TextStyle {
                    font_family: "".to_string(),
                    font_size: 12,
                    font_weight: 400,
                    color: "#0969da".to_string(),
                    alpha: 0.7,
                },
                enable_random_highlight: false,
                show_learning_stroke_hint: true,
                show_learning_english_hint: true,
                auto_pronounce: true,
                show_tray: true,
            },
            input: Input {
                autostart: true,
                commit_mode: "single".to_string(),
                default_profile: "chinese".to_string(),
                phantom_type: PhantomType::Pinyin,
                anti_typo_mode: AntiTypoMode::None,
                enable_double_tap: false,
                double_tap_timeout_ms: 250,
                double_taps: vec![],
                enable_long_press: false,
                long_press_timeout_ms: 400,
                long_press_mappings: vec![],
                enable_punctuation_long_press: true,
                punctuation_long_press_mappings: std::collections::HashMap::new(),
                keyboard_layouts: std::collections::HashMap::new(),
                auto_commit_unique_en_fuzhuma: false,
                auto_commit_unique_full_match: false,
                auto_commit_stroke: true,
                enable_prefix_matching: true,
                prefix_matching_limit: 20,
                enable_abbreviation_matching: true,
                filter_proper_nouns_by_case: true,
                enabled_profiles: vec!["chinese".to_string()],
                profile_keys: vec![
                    ProfileKey {
                        key: "c".into(),
                        profile: "chinese".into(),
                    },
                    ProfileKey {
                        key: "e".into(),
                        profile: "english".into(),
                    },
                    ProfileKey {
                        key: "j".into(),
                        profile: "japanese".into(),
                    },
                    ProfileKey {
                        key: "b".into(),
                        profile: "stroke".into(),
                    },
                    ProfileKey {
                        key: "m".into(),
                        profile: "chinese,english,japanese".into(),
                    },
                    ProfileKey {
                        key: "s".into(),
                        profile: "shengpizi".into(),
                    },
                ],
                swap_arrow_keys: false,
                enable_error_sound: true,
                enable_keyboard_voice: false,
                enable_english_filter: true,
                enable_caps_selection: true,
                enable_number_selection: true,
                enable_word_discovery: true,
                enable_auto_reorder: true,
                enable_fixed_first_candidate: false,
                enable_smart_backspace: false,
                enable_double_pinyin: false,
                double_pinyin_scheme: DoublePinyinScheme {
                    name: "小鹤双拼".to_string(),
                    initials: [("v", "zh"), ("u", "sh"), ("i", "ch")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                    rimes: [
                        ("p", "ie"),
                        ("b", "in"),
                        ("m", "ian"),
                        ("q", "iu"),
                        ("r", "uan"),
                        ("x", "ia"),
                        ("k", "ao"),
                        ("f", "en"),
                        ("d", "ai"),
                        ("j", "an"),
                        ("t", "ue"),
                        ("c", "ao"),
                        ("s", "ong"),
                        ("z", "ou"),
                        ("y", "un"),
                        ("w", "ei"),
                        ("l", "iang"),
                    ]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                },
                enable_fuzzy_pinyin: false,
                fuzzy_config: FuzzyPinyinConfig {
                    z_zh: true,
                    c_ch: true,
                    s_sh: true,
                    n_l: false,
                    r_l: false,
                    f_h: false,
                    an_ang: false,
                    en_eng: false,
                    in_ing: false,
                    ian_iang: false,
                    uan_uang: false,
                    u_v: false,
                    custom_mappings: vec![],
                    fuzzy_page_threshold: 5,
                },
                enable_traditional: false,
                english_aux_mode: EnglishAuxMode::Prefix,
                display_mode: DisplayMode::CharacterWithEnglish,
                ranking: RankingConfig {
                    length_penalty: 50000.0,
                    user_dict_bonus: 10000000.0,
                    exact_match_bonus: 10000000.0,
                    single_char_bonus: 1000000.0,
                },
                firefox_space_interrupt: false,
                segmentation_delimiters: "'".to_string(),
            },
            hotkeys: Hotkeys {
                switch_language: Hotkey {
                    key: "CapsLock".to_string(),
                    description: "核心: 切换中/英文模式".to_string(),
                },
                page_up: vec![
                    "Up".into(),
                    "PageUp".into(),
                    "-".into(),
                    ",".into(),
                    "[".into(),
                ],
                page_down: vec![
                    "Down".into(),
                    "PageDown".into(),
                    "=".into(),
                    ".".into(),
                    "]".into(),
                ],
                prev_candidate: vec!["Left".into()],
                next_candidate: vec!["Right".into()],
                enable_tab_toggle: false,
                enable_ctrl_space_toggle: false,
                enable_ctrl_capslock_commit: true,
                toggle_traditional: Hotkey {
                    key: "CapsLock+F".to_string(),
                    description: "繁简体切换".to_string(),
                },
                word_to_char: vec![
                    "9".into(),
                    "0".into(),
                ],
                word_to_char_shift: vec![
                    "1".into(),
                    "2".into(),
                ],
            },
            enable_quick_finals: false,
            quick_finals: vec![
                QuickFinal { key: "u".into(), final_text: "uang".into() },
                QuickFinal { key: "i".into(), final_text: "ing".into() },
                QuickFinal { key: "o".into(), final_text: "ong".into() },
                QuickFinal { key: "p".into(), final_text: "iong".into() },
                QuickFinal { key: "h".into(), final_text: "ang".into() },
                QuickFinal { key: "j".into(), final_text: "eng".into() },
                QuickFinal { key: "k".into(), final_text: "uai".into() },
                QuickFinal { key: "l".into(), final_text: "iang".into() },
                QuickFinal { key: "n".into(), final_text: "iao".into() },
                QuickFinal { key: "m".into(), final_text: "ian".into() },
                QuickFinal { key: "y".into(), final_text: "uan".into() },
            ],
            punctuations: std::collections::HashMap::new(),
            layouts: default_profile_layouts(),
            #[cfg(target_os = "linux")]
            linux: LinuxConfig {
                device_path: "/dev/input/event4".to_string(),
                paste_method: "shift_insert".to_string(),
                clipboard_delay_ms: 50,
                show_slint_window: true,
                show_notification: false,
                show_toggle_notification: false,
                fixed_position: true,
                corner: "bottom-right".to_string(),
                fixed_x: 40,
                fixed_y: 40,
            },
        }
    }
}
