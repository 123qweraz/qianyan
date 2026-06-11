use crate::keys::VirtualKey;
use crate::user_data::UserDataManager;
use arc_swap::ArcSwap;
use qianyan_ime_core::config::{Config, PhantomType, ProfileLayout, PunctuationEntry};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type UserDictData = HashMap<String, HashMap<String, Vec<(String, u32)>>>;
pub type OrderData = HashMap<String, Vec<String>>;

const USAGE_SAVE_INTERVAL: Duration = Duration::from_secs(30);

pub struct ConfigManager {
    pub master_config: Config,
    pub learned_words: Arc<ArcSwap<UserDictData>>,
    pub long_term_words: Arc<ArcSwap<UserDictData>>,
    pub combined_dict: Arc<ArcSwap<UserDictData>>,
    pub ngram_history: Arc<ArcSwap<UserDictData>>,
    pub user_order: Arc<ArcSwap<OrderData>>,
    pub user_data: Option<Arc<UserDataManager>>,
    usage_last_save: Mutex<Instant>,
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigManager {
    pub fn new() -> Self {
        let master = Config::default_config();
        let data_dir = Self::get_data_dir();

        let user_data = UserDataManager::new(data_dir).ok();

        if user_data.is_some() {
            log::info!("[ConfigManager] 初始化用户数据管理器 (JSON 存储)");
        }

        Self {
            master_config: master,
            learned_words: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            long_term_words: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            combined_dict: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            ngram_history: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            user_order: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            user_data: user_data.map(Arc::new),
            usage_last_save: Mutex::new(Instant::now()),
        }
    }

    fn get_data_dir() -> PathBuf {
        if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(config_home)
                .join("qianyan-ime")
                .join("user_data")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join(".config")
                .join("qianyan-ime")
                .join("user_data")
        } else {
            PathBuf::from("data").join("user_data")
        }
    }

    pub fn master_config_write(&mut self) -> &mut Config {
        &mut self.master_config
    }

    pub fn apply_config(&mut self, conf: &Config) {
        self.master_config = conf.clone();
        log::info!("ConfigManager::apply_config: rare_char_mode={:?}", self.master_config.input.rare_char_mode);

        if (self.master_config.input.enable_word_discovery
            || self.master_config.input.enable_auto_reorder)
            && self.learned_words.load().is_empty()
        {
            self.load_user_dicts();
        }
    }

    pub fn load_user_dicts(&mut self) {
        let mut profiles: Vec<String> = self
            .master_config
            .input
            .profile_keys
            .iter()
            .map(|pk| pk.profile.to_lowercase())
            .collect();

        if profiles.is_empty() {
            profiles.push("chinese".to_string());
        }

        if let Some(ref user_data) = self.user_data {
            let (learned, ngrams, orders) = user_data.load_all(&profiles);
            self.learned_words.store(Arc::new(learned));

            // 加载长期记忆
            let mut long_term_all: UserDictData = UserDictData::new();
            for profile in &profiles {
                let lt = user_data.load(profile, crate::user_data::DataType::LongTerm);
                if !lt.is_empty() {
                    long_term_all.insert(profile.clone(), lt);
                }
            }
            self.long_term_words.store(Arc::new(long_term_all));

            self.ngram_history.store(Arc::new(ngrams));
            self.user_order.store(Arc::new(orders));
            self.rebuild_combined_dict();
        }
    }

    pub fn insert_learned(&self, profile: &str, pinyin: &str, entries: &[(String, u32)]) {
        let mut short = (**self.learned_words.load()).clone();
        let cap = self.master_config.input.short_term_capacity as usize;
        let threshold = self.master_config.input.long_term_threshold;

        // 更新短期池
        short
            .entry(profile.to_string())
            .or_default()
            .insert(pinyin.to_string(), entries.to_vec());

        // 淘汰：超过容量时移除 count 最低的条目
        if let Some(profile_data) = short.get_mut(profile) {
            let total: usize = profile_data.values().map(|v| v.len()).sum();
            if total > cap {
                // 找到全局最低 count 的条目
                let mut min_count = u32::MAX;
                let mut evict_py = String::new();
                let mut evict_idx = 0;
                for (py, words) in profile_data.iter() {
                    for (i, (_, cnt)) in words.iter().enumerate() {
                        if *cnt < min_count {
                            min_count = *cnt;
                            evict_py = py.clone();
                            evict_idx = i;
                        }
                    }
                }
                if !evict_py.is_empty() {
                    if let Some(words) = profile_data.get_mut(&evict_py) {
                        words.remove(evict_idx);
                        if words.is_empty() {
                            profile_data.remove(&evict_py);
                        }
                    }
                }
            }
        }

        // 晋升：count >= threshold 的条目移入长期记忆
        let mut long = (**self.long_term_words.load()).clone();
        let mut promoted = false;
        if let Some(profile_data) = short.get_mut(profile) {
            let mut to_remove: Vec<(String, String, u32)> = Vec::new();
            for (py, words) in profile_data.iter() {
                for (word, cnt) in words.iter() {
                    if *cnt >= threshold {
                        to_remove.push((py.clone(), word.clone(), *cnt));
                    }
                }
            }
            for (py, word, cnt) in &to_remove {
                if let Some(words) = profile_data.get_mut(py) {
                    words.retain(|(w, _)| w != word);
                    if words.is_empty() {
                        profile_data.remove(py);
                    }
                }
                let long_entries = long.entry(profile.to_string()).or_default()
                    .entry(py.clone()).or_default();
                if let Some(pos) = long_entries.iter().position(|(w, _)| w == word) {
                    long_entries[pos].1 = *cnt;
                } else {
                    long_entries.push((word.clone(), *cnt));
                }
                promoted = true;
            }
        }

        self.learned_words.store(Arc::new(short));
        self.long_term_words.store(Arc::new(long));
        self.rebuild_combined_dict();

        // 写盘
        if let Some(ref user_data) = self.user_data {
            let _ = user_data.save_user_dict(profile, crate::user_data::DataType::Learned,
                &self.learned_words.load());
            if promoted {
                let _ = user_data.save_user_dict(profile, crate::user_data::DataType::LongTerm,
                    &self.long_term_words.load());
            }
        }
    }

    pub fn insert_long_term_direct(&self, profile: &str, pinyin: &str, entries: &[(String, u32)]) {
        let mut long = (**self.long_term_words.load()).clone();
        long
            .entry(profile.to_string())
            .or_default()
            .insert(pinyin.to_string(), entries.to_vec());
        self.long_term_words.store(Arc::new(long));
        self.rebuild_combined_dict();
        if let Some(ref user_data) = self.user_data {
            let _ = user_data.save_user_dict(profile, crate::user_data::DataType::LongTerm,
                &self.long_term_words.load());
        }
    }

    fn rebuild_combined_dict(&self) {
        let short = self.learned_words.load();
        let long = self.long_term_words.load();
        let mut combined = (**short).clone();
        for (profile, profile_data) in (**long).iter() {
            let target = combined.entry(profile.clone()).or_default();
            for (pinyin, words) in profile_data {
                let target_words = target.entry(pinyin.clone()).or_default();
                for (word, cnt) in words {
                    if !target_words.iter().any(|(w, _)| w == word) {
                        target_words.push((word.clone(), *cnt));
                    }
                }
            }
        }
        self.combined_dict.store(Arc::new(combined));
    }

    pub fn insert_usage_order(&self, profile: &str, word: &str) {
        let mut current = (**self.user_order.load()).clone();
        let entries = current.entry(profile.to_string()).or_default();
        entries.retain(|w| w != word);
        entries.insert(0, word.to_string());
        entries.truncate(self.master_config.input.mru_length as usize);
        self.user_order.store(Arc::new(current));
    }

    pub fn insert_ngram(&self, profile: &str, context: &str, entries: &[(String, u32)]) {
        self.ngram_history.rcu(|hist| {
            let mut clone = (**hist).clone();
            clone
                .entry(profile.to_string())
                .or_default()
                .insert(context.to_string(), entries.to_vec());
            Arc::new(clone)
        });
        self.save_usage_if_due(profile);
    }

    fn save_usage_if_due(&self, profile: &str) {
        let mut last = self.usage_last_save.lock()
            .unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= USAGE_SAVE_INTERVAL {
            *last = Instant::now();
            if let Some(ref user_data) = self.user_data {
                let _ = user_data.save_order(profile, &self.user_order.load());
                let _ = user_data.save_user_dict(profile, crate::user_data::DataType::Ngram,
                    &self.ngram_history.load());
            }
        }
    }

    /// 将所有用户数据批量写入磁盘（在退出或定时触发时调用）
    pub fn flush_all(&self) {
        if let Some(ref user_data) = self.user_data {
            let learned = (**self.learned_words.load()).clone();
            let long_term = (**self.long_term_words.load()).clone();
            let ngram = (**self.ngram_history.load()).clone();
            let order = (**self.user_order.load()).clone();

            let profiles: Vec<String> = self.master_config
                .input.profile_keys.iter()
                .map(|pk| pk.profile.to_lowercase())
                .collect();

            for profile in &profiles {
                if let Err(e) = user_data.save_user_dict(profile, crate::user_data::DataType::Learned, &learned) {
                    log::error!("[ConfigManager] 保存 learned 失败: {}", e);
                }
                if let Err(e) = user_data.save_user_dict(profile, crate::user_data::DataType::LongTerm, &long_term) {
                    log::error!("[ConfigManager] 保存 long_term 失败: {}", e);
                }
                if let Err(e) = user_data.save_order(profile, &order) {
                    log::error!("[ConfigManager] 保存 order 失败: {}", e);
                }
                if let Err(e) = user_data.save_user_dict(profile, crate::user_data::DataType::Ngram, &ngram) {
                    log::error!("[ConfigManager] 保存 ngram 失败: {}", e);
                }
            }
            log::info!("[ConfigManager] 用户数据已写入磁盘");
        }
    }

    pub fn clear_user_data(&mut self, profile: &str) -> std::io::Result<()> {
        if let Some(ref user_data) = self.user_data {
            user_data.clear(profile, None)?;
        }
        self.load_user_dicts();
        Ok(())
    }

    pub fn list_profiles(&self) -> Vec<String> {
        if let Some(ref user_data) = self.user_data {
            user_data.list_profiles()
        } else {
            Vec::new()
        }
    }

    // === Helper methods for computed values ===

    pub fn profile_keys(&self) -> Vec<(String, String)> {
        self.master_config
            .input
            .profile_keys
            .iter()
            .map(|pk| (pk.key.to_lowercase(), pk.profile.to_lowercase()))
            .collect()
    }

    pub fn page_up_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .page_up
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn page_down_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .page_down
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn prev_candidate_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .prev_candidate
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn next_candidate_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .next_candidate
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn nav_delete_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .nav_delete
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn nav_fuzzy_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .nav_fuzzy
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn nav_clear_keys(&self) -> std::collections::HashSet<VirtualKey> {
        self.master_config
            .hotkeys
            .nav_clear
            .iter()
            .filter_map(|s| s.parse::<VirtualKey>().ok())
            .collect()
    }

    pub fn word_to_char_keys(&self) -> Vec<(VirtualKey, usize)> {
        self.master_config
            .hotkeys
            .word_to_char
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.parse::<VirtualKey>().ok().map(|k| (k, i)))
            .collect()
    }

    pub fn word_to_char_shift_keys(&self) -> Vec<(VirtualKey, usize)> {
        self.master_config
            .hotkeys
            .word_to_char_shift
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.parse::<VirtualKey>().ok().map(|k| (k, i)))
            .collect()
    }

    pub fn quick_finals(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for qf in &self.master_config.quick_finals {
            m.insert(qf.key.to_lowercase(), qf.final_text.clone());
        }
        m
    }

    pub fn double_taps(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for dt in &self.master_config.input.double_taps {
            m.insert(dt.trigger_key.to_lowercase(), dt.insert_text.clone());
        }
        m
    }

    pub fn double_tap_uppercase_keys(&self) -> &[String] {
        &self.master_config.input.double_tap_uppercase_keys
    }

    pub fn long_press_uppercase_keys(&self) -> &[String] {
        &self.master_config.input.long_press_uppercase_keys
    }

    pub fn double_tap_timeout(&self) -> Duration {
        Duration::from_millis(self.master_config.input.double_tap_timeout_ms)
    }

    pub fn long_press_timeout(&self) -> Duration {
        Duration::from_millis(self.master_config.input.long_press_timeout_ms)
    }

    pub fn long_press_mappings(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for lm in &self.master_config.input.long_press_mappings {
            m.insert(lm.trigger_key.to_lowercase(), lm.insert_text.clone());
        }
        m
    }

    // === Shortcut accessors ===

    pub fn show_candidates(&self) -> bool {
        self.master_config.appearance.show_candidates
    }

    pub fn page_size(&self) -> usize {
        self.master_config.appearance.page_size
    }

    pub fn commit_mode(&self) -> &str {
        &self.master_config.input.commit_mode
    }

    pub fn enable_auto_reorder(&self) -> bool {
        self.master_config.input.enable_auto_reorder
    }

    pub fn auto_commit_unique_full_match(&self) -> bool {
        self.master_config.input.auto_commit_unique_full_match
    }

    pub fn auto_commit_stroke(&self) -> bool {
        self.master_config.input.auto_commit_stroke
    }

    pub fn firefox_space_interrupt(&self) -> bool {
        self.master_config.input.firefox_space_interrupt
    }

    pub fn swap_arrow_keys(&self) -> bool {
        self.master_config.input.swap_arrow_keys
    }

    pub fn enable_number_selection(&self) -> bool {
        self.master_config.input.enable_number_selection
    }

    pub fn enable_double_tap(&self) -> bool {
        self.master_config.input.enable_double_tap
    }

    pub fn enable_long_press(&self) -> bool {
        self.master_config.input.enable_long_press
    }

    pub fn enable_punctuation_long_press(&self) -> bool {
        self.master_config.input.enable_punctuation_long_press
    }

    pub fn punctuation_long_press_mappings(&self) -> &HashMap<String, String> {
        &self.master_config.input.punctuation_long_press_mappings
    }

    pub fn punctuations(&self) -> &HashMap<String, HashMap<String, Vec<PunctuationEntry>>> {
        &self.master_config.punctuations
    }

    pub fn layouts(&self) -> &HashMap<String, ProfileLayout> {
        &self.master_config.layouts
    }

    pub fn keyboard_layouts(&self) -> &HashMap<String, HashMap<String, String>> {
        &self.master_config.input.keyboard_layouts
    }

    pub fn phantom_type(&self) -> PhantomType {
        if cfg!(target_os = "windows") {
            PhantomType::None
        } else {
            self.master_config.input.phantom_type
        }
    }

    pub fn phantom_separator(&self) -> &str {
        &self.master_config.input.phantom_separator
    }
}
