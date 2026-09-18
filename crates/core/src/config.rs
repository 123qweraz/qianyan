use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

use crate::file_lock;
use crate::scheme_def::SchemeDefinition;

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
    #[serde(default = "default_backspace_delay")]
    pub backspace_delay_ms: u64,
    #[serde(default = "default_true")]
    pub show_slint_window: bool,
    #[serde(default = "default_true")]
    pub show_preedit: bool,
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
    #[serde(default = "default_backend_type")]
    pub backend_type: String, // "auto", "evdev", "wayland"

    #[serde(default = "default_keystroke_enabled")]
    pub keystroke_enabled: bool,

    #[serde(default)]
    pub keystroke_position: String,

    #[serde(default = "default_keystroke_hide_ms")]
    pub keystroke_hide_ms: u64,

    #[serde(default = "default_keystroke_display_ms")]
    pub keystroke_display_ms: u64,

    #[serde(default = "default_keystroke_font_size")]
    pub keystroke_font_size: u32,

    #[serde(default = "default_keystroke_bg_color")]
    pub keystroke_bg_color: String,

    #[serde(default = "default_keystroke_text_color")]
    pub keystroke_text_color: String,
}

fn default_true() -> bool {
    true
}

fn default_mru_length() -> u32 {
    2000
}

fn default_word_learn_threshold() -> u32 {
    2
}

fn default_short_term_capacity() -> u32 {
    200
}

fn default_long_term_threshold() -> u32 {
    5
}

fn default_ngram_synthesis_threshold() -> u32 {
    2
}

fn default_match_level_exact() -> f64 {
    30_000_000.0
}

fn default_match_level_fuzzy() -> f64 {
    20_000_000.0
}

fn default_match_level_prefix() -> f64 {
    10_000_000.0
}

fn default_context_boost_bigram() -> f64 {
    50_000_000.0
}

fn default_context_boost_trigram() -> f64 {
    60_000_000.0
}

fn default_context_boost_cap() -> u32 {
    40
}

fn default_compose_freq_multiplier() -> f64 {
    100.0
}

fn default_compose_word_len_bonus() -> f64 {
    200.0
}

fn default_compose_abbr_bonus() -> f64 {
    5000.0
}

fn default_compose_dict_scale() -> f64 {
    0.01
}

fn default_compose_ngram_bonus() -> f64 {
    500.0
}

fn default_compose_ngram_cap() -> u32 {
    10
}

fn default_fixed_x() -> i32 {
    10
}

fn default_fixed_y() -> i32 {
    10
}

fn default_backend_type() -> String {
    "auto".to_string()
}

fn default_keystroke_enabled() -> bool {
    false
}

fn default_keystroke_hide_ms() -> u64 {
    2000
}

fn default_keystroke_display_ms() -> u64 {
    800
}

fn default_keystroke_font_size() -> u32 {
    22
}

fn default_keystroke_bg_color() -> String {
    "rgba(0, 0, 0, 0.78)".to_string()
}

fn default_keystroke_text_color() -> String {
    "#ffffff".to_string()
}

fn default_backspace_delay() -> u64 {
    2
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
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

/// 候选显示模板：主文本固定为候选字本身；fields 序列决定「副注解串」由哪些字段拼成。
/// 可用字段：char(候选字) / pinyin(拼音) / trad(繁体) / tone(声调) / stroke_aux(笔画辅助码)
/// / strokes(完整笔顺) / category(字级) / en(英注释) / weight(权重)。未知字段跳过。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CandidateDisplay {
    /// 全局默认模板：字段序列
    #[serde(default = "default_candidate_display_fields")]
    pub fields: Vec<String>,
    /// 按 profile 覆盖模板（key = profile 名）；空列表表示回落到全局 fields
    #[serde(default)]
    pub per_profile: std::collections::HashMap<String, Vec<String>>,
}

pub fn default_candidate_display_fields() -> Vec<String> {
    vec!["char".into(), "en".into()]
}

impl Default for CandidateDisplay {
    fn default() -> Self {
        Self {
            fields: default_candidate_display_fields(),
            per_profile: std::collections::HashMap::new(),
        }
    }
}

/// 选词（候选过滤）辅助码类型：与「候选显示模板」完全分离，可分别设置。
/// auto = 英文与笔画辅助码均可过滤（默认，兼容旧行为）；
/// english = 仅用英注释（en）过滤；stroke = 仅用笔画辅助码过滤；none = 关闭辅助码过滤。
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxFilterMode {
    #[default]
    Auto,
    English,
    Stroke,
    None,
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

fn default_phantom_separator() -> String {
    ">".to_string()
}

fn default_shown_char_categories() -> Vec<String> {
    vec!["level-1".into(), "level-2".into(), "level-3".into()]
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Input {
    pub autostart: bool,
    pub commit_mode: String,
    pub default_profile: String,
    pub phantom_type: PhantomType,
    #[serde(default = "default_phantom_separator")]
    pub phantom_separator: String,
    pub enable_double_tap: bool,
    pub double_tap_timeout_ms: u64,
    pub double_taps: Vec<DoubleTap>,
    #[serde(default)]
    pub double_tap_uppercase_keys: Vec<String>,
    pub enable_long_press: bool,
    pub long_press_timeout_ms: u64,
    pub long_press_mappings: Vec<LongPressMapping>,
    pub enable_punctuation_long_press: bool,
    pub punctuation_long_press_mappings: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub long_press_uppercase_keys: Vec<String>,
    pub keyboard_layouts:
        std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    pub auto_commit_unique_en_fuzhuma: bool,
    pub auto_commit_unique_full_match: bool,
    pub auto_commit_stroke: bool,
    #[serde(default = "default_auto_sentence_min_syllables")]
    pub auto_sentence_min_syllables: u32,
    pub enable_prefix_matching: bool,
    pub prefix_matching_limit: usize,
    pub enable_abbreviation_matching: bool,
    pub enable_error_correction: bool,
    #[serde(default = "default_shown_char_categories")]
    pub shown_char_categories: Vec<String>,
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
    #[serde(default = "default_word_learn_threshold")]
    pub word_learn_threshold: u32,
    #[serde(default = "default_short_term_capacity")]
    pub short_term_capacity: u32,
    #[serde(default = "default_long_term_threshold")]
    pub long_term_threshold: u32,
    pub enable_auto_reorder: bool,
    #[serde(default)]
    pub enable_pin_first_word: bool,
    #[serde(default = "default_mru_length")]
    pub mru_length: u32,
    #[serde(default = "default_ngram_synthesis_threshold")]
    pub ngram_synthesis_threshold: u32,
    pub enable_smart_backspace: bool,
    pub enable_double_pinyin: bool,
    pub double_pinyin_scheme: DoublePinyinScheme,
    pub fuzzy_config: FuzzyPinyinConfig,
    pub enable_traditional: bool,
    #[serde(default = "default_english_aux_mode")]
    pub english_aux_mode: EnglishAuxMode,
    #[serde(default)]
    pub candidate_display: CandidateDisplay,
    /// 选词辅助码来源：全局默认，按方案可用 filter_aux_per_profile 覆盖
    #[serde(default)]
    pub filter_aux_mode: AuxFilterMode,
    /// 按方案覆盖选词辅助码（key = profile 名；缺省用全局 filter_aux_mode）
    #[serde(default)]
    pub filter_aux_per_profile: std::collections::HashMap<String, AuxFilterMode>,
    pub ranking: RankingConfig,
    #[serde(default = "default_true")]
    pub enable_context_sorting: bool,
    #[serde(default = "default_segmentation_delimiters")]
    pub segmentation_delimiters: String,
    #[serde(default = "default_enable_auto_english")]
    pub enable_auto_english: bool,
    #[serde(default = "default_auto_english_timeout")]
    pub auto_english_timeout: u64,
    #[serde(default = "default_annotation_lang")]
    pub annotation_lang: String,
    /// 外部注释表源映射：表名（如 "zh-jp"）→ 源 TSV 文件路径（绝对路径或相对项目根）。
    /// 程序启动/编译时按此表名把任意位置的 TSV 编译进 data/annotations/<表名>.{index,data}，
    /// 使 annotation_lang 等选表规则指向的词典可放在桌面等任意目录。
    #[serde(default)]
    pub annotation_sources: std::collections::HashMap<String, String>,
    /// 按方案直接指定注释源文件（key = profile 名，value = TSV/JSON 文件路径，
    /// 如 "/home/xiao/Desktop/zh-jp.tsv"）。表名自动推导（文件名主干或稳定哈希），
    /// 启动时自动编译。值为空串表示该方案不显示注释。
    #[serde(default)]
    pub annotation_files: std::collections::HashMap<String, String>,
}

fn default_enable_auto_english() -> bool { true }
fn default_auto_english_timeout() -> u64 { 10 }
fn default_annotation_lang() -> String { "en".to_string() }

impl Input {
    /// 解析某个 scheme 生效的候选显示模板：per_profile 覆盖 → 全局 fields。
    /// 模板为空（fields 置空）时返回空模板 = 不显示副注解。
    pub fn candidate_display_for(&self, profile: &str) -> Vec<String> {
        if let Some(profile_fields) = self.candidate_display.per_profile.get(profile) {
            if !profile_fields.is_empty() {
                return profile_fields.clone();
            }
        }
        self.candidate_display.fields.clone()
    }

    /// 解析某个 scheme 生效的选词辅助码类型：per_profile 覆盖 → 全局。
    pub fn filter_aux_for(&self, profile: &str) -> AuxFilterMode {
        self.filter_aux_per_profile
            .get(profile)
            .copied()
            .unwrap_or(self.filter_aux_mode)
    }

    /// 解析某个 scheme 生效的注释信息：(表名, 目标语言字段键)。
    /// 1) annotation_files[profile] 直接指定源文件 → 表名自动推导；
    ///    语言键取表名目标段（zh-jp → "jp"），供候选模板字段使用；
    /// 2) 否则默认规则：english→en-zh / japanese→jp-en / 其余→zh-{annotation_lang}。
    ///    表名为空 = 该方案不显示注释。
    pub fn annotation_info_for(&self, profile: &str) -> (String, String) {
        if let Some(path) = self.annotation_files.get(profile) {
            if path.is_empty() {
                return (String::new(), String::new());
            }
            let table =
                crate::utils::annotation_table_name_for_path(path, &self.annotation_sources);
            let lang = table
                .rsplit_once('-')
                .map(|(_, l)| l.to_string())
                .unwrap_or_default();
            return (table, lang);
        }
        if profile.eq_ignore_ascii_case("english") {
            return ("en-zh".to_string(), "zh".to_string());
        }
        if profile.eq_ignore_ascii_case("japanese") {
            return ("jp-en".to_string(), "en".to_string());
        }
        if self.annotation_lang.is_empty() {
            (String::new(), String::new())
        } else {
            (
                format!("zh-{}", self.annotation_lang),
                self.annotation_lang.clone(),
            )
        }
    }

    /// 解析某个 scheme 生效的候选显示模板：方案定义优先（display），否则回到全局/遗留覆盖。
    pub fn candidate_display_for_scheme(
        &self,
        profile: &str,
        def: Option<&SchemeDefinition>,
    ) -> Vec<String> {
        if let Some(def) = def {
            if let Some(fields) = &def.display {
                return fields.clone();
            }
        }
        self.candidate_display_for(profile)
    }

    /// 解析某个 scheme 生效的选词辅助码：方案定义优先，否则回到全局/遗留覆盖。
    pub fn filter_aux_for_scheme(
        &self,
        profile: &str,
        def: Option<&SchemeDefinition>,
    ) -> AuxFilterMode {
        if let Some(def) = def {
            if let Some(mode) = def.aux_filter {
                return mode;
            }
        }
        self.filter_aux_for(profile)
    }

    /// 解析某个 scheme 生效的注释信息：方案定义（显式表名/文件）优先，
    /// 否则回到遗留 annotation_files 与默认规则。
    pub fn annotation_info_for_scheme(
        &self,
        profile: &str,
        def: Option<&SchemeDefinition>,
    ) -> (String, String) {
        if let Some(def) = def {
            let (table, lang) = def.annotation_table(&self.annotation_sources);
            if def.annotation.disabled || !table.is_empty() {
                return (table, lang);
            }
        }
        self.annotation_info_for(profile)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RankingConfig {
    pub length_penalty: f64,
    pub user_dict_bonus: f64,
    pub exact_match_bonus: f64,
    pub single_char_bonus: f64,

    #[serde(default = "default_match_level_exact")]
    pub match_level_exact: f64,
    #[serde(default = "default_match_level_fuzzy")]
    pub match_level_fuzzy: f64,
    #[serde(default = "default_match_level_prefix")]
    pub match_level_prefix: f64,

    #[serde(default = "default_context_boost_bigram")]
    pub context_boost_bigram: f64,
    #[serde(default = "default_context_boost_trigram")]
    pub context_boost_trigram: f64,
    #[serde(default = "default_context_boost_cap")]
    pub context_boost_cap: u32,

    #[serde(default = "default_compose_freq_multiplier")]
    pub compose_freq_multiplier: f64,
    #[serde(default = "default_compose_word_len_bonus")]
    pub compose_word_len_bonus: f64,
    #[serde(default = "default_compose_abbr_bonus")]
    pub compose_abbr_bonus: f64,
    #[serde(default = "default_compose_dict_scale")]
    pub compose_dict_scale: f64,
    #[serde(default = "default_compose_ngram_bonus")]
    pub compose_ngram_bonus: f64,
    #[serde(default = "default_compose_ngram_cap")]
    pub compose_ngram_cap: u32,
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
    #[serde(default = "default_enable_shift_toggle")]
    pub enable_shift_toggle: bool,
    pub enable_ctrl_capslock_commit: bool,
    #[serde(default = "default_toggle_traditional")]
    pub toggle_traditional: Hotkey,
    #[serde(default = "default_word_to_char")]
    pub word_to_char: Vec<String>,
    #[serde(default = "default_word_to_char_shift")]
    pub word_to_char_shift: Vec<String>,
    #[serde(default = "default_nav_delete")]
    pub nav_delete: Vec<String>,
    #[serde(default = "default_nav_fuzzy")]
    pub nav_fuzzy: Vec<String>,
    #[serde(default = "default_nav_clear")]
    pub nav_clear: Vec<String>,
}

fn default_nav_delete() -> Vec<String> {
    vec!["D".into()]
}
fn default_nav_fuzzy() -> Vec<String> {
    vec!["S".into()]
}
fn default_nav_clear() -> Vec<String> {
    vec!["R".into()]
}

fn default_word_to_char() -> Vec<String> {
    vec!["9".into(), "0".into()]
}

fn default_word_to_char_shift() -> Vec<String> {
    vec!["1".into(), "2".into()]
}

fn default_toggle_traditional() -> Hotkey {
    Hotkey {
        key: "Ctrl+Shift+F".to_string(),
        description: "繁简体切换".to_string(),
    }
}

fn default_enable_shift_toggle() -> bool {
    true
}

fn default_segmentation_delimiters() -> String {
    "'".to_string()
}

fn default_auto_sentence_min_syllables() -> u32 {
    4
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
    chinese.insert("[".to_string(), action("【", "{", None, None, "中文左方括号"));
    chinese.insert("]".to_string(), action("】", "}", None, None, "中文右方括号"));
    chinese.insert("'".to_string(), action("'", "\"", None, None, "中文引号"));
    chinese.insert("/".to_string(), action("、", "?", None, None, "中文顿号"));
    chinese.insert("\\".to_string(), action("、", "|", None, None, "中文顿号"));
    layouts.insert(
        "chinese".to_string(),
        ProfileLayout {
            name: "中文默认布局".into(),
            mappings: chinese,
        },
    );

    let english = std::collections::HashMap::new();
    layouts.insert(
        "english".to_string(),
        ProfileLayout {
            name: "English (Default)".into(),
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
        .entry("bihua".to_string())
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
        if let Some(dir) = std::env::var_os("QIANYAN_CONFIG_DIR") {
            return std::path::PathBuf::from(dir);
        }

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
        ("system", &["files", "linux"], &["autostart", "enabled_profiles", "phantom_type", "phantom_separator"], &["show_candidates", "show_tray"]),
        ("fuzzy", &[], &["fuzzy_config"], &[]),
        ("doublepinyin", &[], &["enable_double_pinyin", "double_pinyin_scheme"], &[]),
        ("punctuation", &["punctuations"], &["enable_punctuation_long_press", "punctuation_long_press_mappings"], &[]),
        ("hotkeys", &["hotkeys"], &[], &[]),
        ("algorithms", &[],
            &["enable_auto_reorder", "mru_length", "enable_pin_first_word",
              "enable_context_sorting", "ngram_synthesis_threshold", "auto_sentence_min_syllables",
              "enable_word_discovery", "word_learn_threshold", "short_term_capacity", "long_term_threshold",
              "ranking.length_penalty", "ranking.user_dict_bonus", "ranking.exact_match_bonus",
              "ranking.single_char_bonus", "ranking.match_level_exact", "ranking.match_level_fuzzy",
              "ranking.match_level_prefix", "ranking.context_boost_bigram", "ranking.context_boost_trigram",
              "ranking.context_boost_cap", "ranking.compose_freq_multiplier", "ranking.compose_word_len_bonus",
              "ranking.compose_abbr_bonus", "ranking.compose_dict_scale", "ranking.compose_ngram_bonus",
              "ranking.compose_ngram_cap"],
            &[]),
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
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("[Config] 无法打开配置文件 {:?}: {}", path, e);
                return None;
            }
        };
        match serde_json::from_reader(std::io::BufReader::new(file)) {
            Ok(val) => Some(val),
            Err(e) => {
                log::warn!("[Config] 配置文件损坏 {:?}: {}", path, e);
                None
            }
        }
    }

    /// 保存单个 JSON 文件（临时文件 + 原子 rename，避免并发读读到半截文件）
    fn save_json(path: &std::path::Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
        let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let file_name = path
            .file_name()
            .ok_or("invalid config path")?
            .to_string_lossy();
        let tmp = dir.join(format!(".{}.{}.tmp", file_name, std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        {
            let f = std::fs::File::create(&tmp)?;
            serde_json::to_writer_pretty(&f, value)?;
            // 确保数据落盘后再 rename，避免断电/崩溃后得到空文件
            f.sync_all().ok();
        }
        // Windows 上 rename 不允许覆盖已存在目标，先删旧文件
        #[cfg(target_os = "windows")]
        {
            let _ = std::fs::remove_file(path);
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load() -> Self {
        let config_dir = Self::get_config_dir();
        if !config_dir.exists() {
            let _ = std::fs::create_dir_all(&config_dir);
        }

        // 跨进程读锁：避免在 save() 分页写入过程中读到混合/半截配置。
        // 锁文件不存在时（首次运行）跳过并放行。
        let _config_lock = file_lock::lock_shared(&config_dir.join(".config.lock")).ok();

        // 从默认值开始，然后被页面文件覆盖
        let mut merged = serde_json::to_value(Self::default_config())
            .unwrap_or(Value::Null);

        // 加载新格式页面文件
        // PAGE_FILES 中的子字段归属是 save 方向用的；load 时全文件合并即可
        for &(file, ..) in Self::PAGE_FILES {
            let path = config_dir.join(format!("{}.json", file));
            if let Some(page_val) = Self::load_json(&path) {
                Self::deep_merge(&mut merged, page_val);
            }
        }

        // pinyin.json —— 包含 input（减去其他页的子字段；hotkeys 已独立）
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
        {
            let old = &"input.json";
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

        // 兼容旧版：繁简切换默认快捷键由 CapsLock+F 调整为 Ctrl+Shift+F
        if let Some(key) = merged
            .get_mut("hotkeys")
            .and_then(|h| h.get_mut("toggle_traditional"))
            .and_then(|t| t.get_mut("key"))
        {
            if key.as_str() == Some("CapsLock+F") {
                *key = Value::String("Ctrl+Shift+F".to_string());
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
        log::info!("Config::save to {:?}, shown_char_categories={:?}", config_dir, self.input.shown_char_categories);
        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir)?;
        }

        // 跨进程互斥：web-settings 子进程与引擎进程同时 save 时串行化，
        // 配合 load() 的读锁使并发读看到一致快照
        let _config_lock = file_lock::lock_exclusive(&config_dir.join(".config.lock"))
            .map_err(|e| format!("config lock failed: {}", e))?;

        let full = serde_json::to_value(self)?;

        // 保存各页面文件
        for &(file, top_keys, input_keys, appear_keys) in Self::PAGE_FILES {
            let mut page = Value::Object(serde_json::Map::new());

            // 顶级键
            for k in top_keys {
                if let Some(v) = full.get(*k) {
                    page.as_object_mut().ok_or_else(|| format!("page is not a JSON object"))?.insert(k.to_string(), v.clone());
                }
            }

            // input 子字段
            if !input_keys.is_empty() {
                if let Some(input_val) = full.get("input") {
                    let picked = Self::pick_keys(input_val, input_keys);
                    if picked != Value::Object(serde_json::Map::new()) {
                        page.as_object_mut().ok_or_else(|| format!("page is not a JSON object"))?.insert("input".into(), picked);
                    }
                }
            }

            // appearance 子字段
            if !appear_keys.is_empty() {
                if let Some(appear_val) = full.get("appearance") {
                    let picked = Self::pick_keys(appear_val, appear_keys);
                    if picked != Value::Object(serde_json::Map::new()) {
                        page.as_object_mut().ok_or_else(|| format!("page is not a JSON object"))?.insert("appearance".into(), picked);
                    }
                }
            }

            Self::save_json(&config_dir.join(format!("{}.json", file)), &page)?;
        }

        // pinyin.json: input（减去其他页拥有字段；hotkeys 已独立到 hotkeys.json）
        {
            let mut page = Value::Object(serde_json::Map::new());
            if let Some(input_val) = full.get("input") {
                let mut input_owned = input_val.clone();
                // 移除其他页拥有的 input 子字段
                for &(_, _, input_keys, _) in Self::PAGE_FILES {
                    Self::remove_keys(&mut input_owned, input_keys);
                }
                page.as_object_mut().ok_or_else(|| format!("page is not a JSON object"))?.insert("input".into(), input_owned);
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
                page.as_object_mut().ok_or_else(|| format!("page is not a JSON object"))?.insert("appearance".into(), appear_owned);
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
                page.as_object_mut().ok_or_else(|| format!("page is not a JSON object"))?.insert("layouts".into(), layouts.clone());
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
                    Profile {
                        name: "bihua".to_string(),
                        path: "data/bihua/trie".to_string(),
                    },
                    Profile {
                        name: "pybi".to_string(),
                        path: "data/pybi/trie".to_string(),
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
                autostart: false,
                commit_mode: "single".to_string(),
                default_profile: "chinese".to_string(),
                phantom_type: PhantomType::Pinyin,
                phantom_separator: ">".to_string(),
                enable_double_tap: false,
                double_tap_timeout_ms: 250,
                double_taps: vec![],
                double_tap_uppercase_keys: vec![],
                enable_long_press: false,
                long_press_timeout_ms: 400,
                long_press_mappings: vec![],
                enable_punctuation_long_press: true,
                punctuation_long_press_mappings: std::collections::HashMap::new(),
                long_press_uppercase_keys: vec![],
                keyboard_layouts: std::collections::HashMap::new(),
                auto_commit_unique_en_fuzhuma: false,
                auto_commit_unique_full_match: false,
                auto_commit_stroke: true,
                auto_sentence_min_syllables: 4,
                enable_prefix_matching: true,
                prefix_matching_limit: 20,
                enable_abbreviation_matching: true,
                enable_error_correction: true,
                shown_char_categories: default_shown_char_categories(),
                filter_proper_nouns_by_case: true,
                enabled_profiles: vec!["chinese".to_string(), "pybi".to_string()],
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
                        key: "v".into(),
                        profile: "bihua".into(),
                    },
                    ProfileKey {
                        key: "x".into(),
                        profile: "pybi".into(),
                    },
                    ProfileKey {
                        key: "m".into(),
                        profile: "chinese,english,japanese".into(),
                    },
                ],
                swap_arrow_keys: false,
                enable_error_sound: true,
                enable_keyboard_voice: false,
                enable_english_filter: true,
                enable_caps_selection: true,
                enable_number_selection: true,
                enable_word_discovery: true,
                word_learn_threshold: 2,
                ngram_synthesis_threshold: 2,
                short_term_capacity: 200,
                long_term_threshold: 5,
                enable_auto_reorder: true,
                enable_pin_first_word: false,
                mru_length: 2000,
                enable_smart_backspace: false,
                enable_context_sorting: true,
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
                },
                enable_traditional: false,
                english_aux_mode: EnglishAuxMode::Prefix,
                candidate_display: CandidateDisplay::default(),
                filter_aux_mode: AuxFilterMode::Auto,
                filter_aux_per_profile: std::collections::HashMap::new(),
                ranking: RankingConfig {
                    length_penalty: 50000.0,
                    user_dict_bonus: 10000000.0,
                    exact_match_bonus: 10000000.0,
                    single_char_bonus: 1000000.0,
                    match_level_exact: default_match_level_exact(),
                    match_level_fuzzy: default_match_level_fuzzy(),
                    match_level_prefix: default_match_level_prefix(),
                    context_boost_bigram: default_context_boost_bigram(),
                    context_boost_trigram: default_context_boost_trigram(),
                    context_boost_cap: default_context_boost_cap(),
                    compose_freq_multiplier: default_compose_freq_multiplier(),
                    compose_word_len_bonus: default_compose_word_len_bonus(),
                    compose_abbr_bonus: default_compose_abbr_bonus(),
                    compose_dict_scale: default_compose_dict_scale(),
                    compose_ngram_bonus: default_compose_ngram_bonus(),
                    compose_ngram_cap: default_compose_ngram_cap(),
                },
                segmentation_delimiters: "'".to_string(),
                enable_auto_english: true,
                auto_english_timeout: 10,
                annotation_lang: "en".to_string(),
                annotation_sources: std::collections::HashMap::new(),
                annotation_files: std::collections::HashMap::new(),
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
                enable_shift_toggle: true,
                enable_ctrl_capslock_commit: true,
                toggle_traditional: Hotkey {
                    key: "Ctrl+Shift+F".to_string(),
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
                nav_delete: vec!["D".into()],
                nav_fuzzy: vec!["S".into()],
                nav_clear: vec!["R".into()],
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
                backspace_delay_ms: 10,
                show_slint_window: true,
                show_preedit: true,
                show_notification: false,
                show_toggle_notification: false,
                fixed_position: true,
                corner: "bottom-right".to_string(),
                fixed_x: 10,
                fixed_y: 10,
                backend_type: "auto".to_string(),
                keystroke_enabled: false,
                keystroke_position: String::new(),
                keystroke_hide_ms: 2000,
                keystroke_display_ms: 800,
                keystroke_font_size: 22,
                keystroke_bg_color: "rgba(0, 0, 0, 0.78)".to_string(),
                keystroke_text_color: "#ffffff".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn with_temp_config_dir<F: FnOnce()>(label: &str, f: F) {
        let dir = std::env::temp_dir().join(format!(
            "qianyan_config_test_{}_{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("QIANYAN_CONFIG_DIR", &dir);
        f();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip_no_tmp_leftovers() {
        with_temp_config_dir("roundtrip", || {
            let cfg = Config::default_config();
            cfg.save().unwrap();

            // 页面文件应真实写出
            assert!(Config::get_config_dir().join("pinyin.json").exists());

            // 不应残留临时文件
            let leftovers = std::fs::read_dir(Config::get_config_dir())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map_or(false, |n| n.ends_with(".tmp"))
                })
                .count();
            assert_eq!(leftovers, 0);

            // 关键字段往返后保持一致
            let loaded = Config::load();
            assert_eq!(loaded.input, cfg.input);
            assert_eq!(loaded.appearance, cfg.appearance);
            assert_eq!(loaded.hotkeys, cfg.hotkeys);
            assert_eq!(loaded.files, cfg.files);
        });
    }

    /// 原子写：确认 save_json 使用临时文件 + rename，不会留下半成品
    #[test]
    fn save_json_is_atomic() {
        with_temp_config_dir("atomic", || {
            let cfg = Config::default_config();
            cfg.save().unwrap();
            let path = Config::get_config_dir().join("pinyin.json");
            // 模拟一次新的 save：应先清理旧 tmp 再原子替换
            let value = serde_json::json!({"input": {"enable_traditional": false}});
            Config::save_json(&path, &value).unwrap();
            let loaded: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(loaded["input"]["enable_traditional"], false);
        });
    }

    #[test]
    fn annotation_info_resolution() {
        let mut cfg = Config::default_config();
        // 默认规则
        assert_eq!(cfg.input.annotation_info_for("chinese"), ("zh-en".into(), "en".into()));
        assert_eq!(cfg.input.annotation_info_for("english"), ("en-zh".into(), "zh".into()));
        assert_eq!(cfg.input.annotation_info_for("japanese"), ("jp-en".into(), "en".into()));
        assert_eq!(cfg.input.annotation_info_for("pybi"), ("zh-en".into(), "en".into()));

        // annotation_lang 全局兜底：chinese → zh-jp
        cfg.input.annotation_lang = "jp".into();
        assert_eq!(cfg.input.annotation_info_for("chinese"), ("zh-jp".into(), "jp".into()));
        assert_eq!(cfg.input.annotation_info_for("japanese"), ("jp-en".into(), "en".into()));

        // 按方案直接指定源文件：文件名主干推导表名 + 语言键
        let mut named = std::collections::HashMap::new();
        named.insert("zh-fr".to_string(), "/data/mydict.tsv".to_string());
        cfg.input.annotation_sources = named.clone();
        cfg.input.annotation_files.insert(
            "chinese".to_string(),
            "/home/xiao/Desktop/zh-jp.tsv".to_string(),
        );
        assert_eq!(cfg.input.annotation_info_for("chinese"), ("zh-jp".into(), "jp".into()));

        // 源路径命中 annotation_sources 的值 → 使用其键名为表名
        cfg.input.annotation_files.insert(
            "pybi".to_string(),
            "/data/mydict.tsv".to_string(),
        );
        assert_eq!(cfg.input.annotation_info_for("pybi"), ("zh-fr".into(), "fr".into()));

        // 非 ASCII 文件名 → 稳定哈希表名，无语言键
        cfg.input.annotation_files.insert(
            "stroke".to_string(),
            "/home/xiao/Desktop/鸟汉字典.tsv".to_string(),
        );
        let (table, lang) = cfg.input.annotation_info_for("stroke");
        assert!(table.starts_with("ext_"), "got {}", table);
        assert_eq!(lang, "");

        // 空串 = 不显示注释
        cfg.input.annotation_files.insert("japanese".to_string(), "".to_string());
        assert_eq!(cfg.input.annotation_info_for("japanese").0, "");
    }
}
