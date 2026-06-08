use std::time::Instant;

#[derive(Debug, Clone)]
pub struct SessionState {
    pub active_profiles: Vec<String>,
    pub chinese_enabled: bool,
    pub ime_enabled: bool,
    pub traditional_enabled: bool,
    pub commit_history: Vec<(String, String)>,
    pub last_commit_time: Instant,
    pub capslock_pending: bool,
    pub caps_lock_enabled: bool,
    pub capslock_down: bool,
    pub capslock_combo_active: bool,
    /// Tab 键是否被按下，用于以词定字（Tab + 键）
    pub tab_down: bool,
    /// 最后一次上屏的 (拼音, 词)，用于打错检测（同拼音换词则衰减旧词）
    last_committed: Option<(String, String)>,
    last_committed_time: Instant,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            active_profiles: Vec::new(),
            chinese_enabled: true,
            ime_enabled: true,
            traditional_enabled: false,
            commit_history: Vec::new(),
            last_commit_time: Instant::now(),
            capslock_pending: false,
            caps_lock_enabled: false,
            capslock_down: false,
            capslock_combo_active: false,
            tab_down: false,
            last_committed: None,
            last_committed_time: Instant::now(),
        }
    }

    pub fn set_last_committed(&mut self, pinyin: String, word: String) {
        self.last_committed = Some((pinyin, word));
        self.last_committed_time = Instant::now();
    }

    /// 获取最后一次上屏的拼音和词（如果在超时窗口内，用于打错检测）
    pub fn last_commit(&self, timeout_secs: u64) -> Option<(&str, &str)> {
        if self.last_committed_time.elapsed().as_secs() < timeout_secs {
            self.last_committed
                .as_ref()
                .map(|(py, w)| (py.as_str(), w.as_str()))
        } else {
            None
        }
    }

    pub fn add_to_history(&mut self, pinyin: String, word: String) {
        self.commit_history.push((pinyin, word));
        if self.commit_history.len() > 10 {
            self.commit_history.remove(0);
        }
    }

    pub fn get_last_word(&self) -> Option<&str> {
        self.commit_history.last().map(|(_, w)| w.as_str())
    }

    pub fn update_commit_time(&mut self) {
        self.last_commit_time = Instant::now();
    }

    pub fn get_combination_candidates(&self, max_len: usize) -> Vec<(String, String)> {
        let mut results = Vec::new();
        if self.commit_history.len() < 2 {
            return results;
        }
        let prev = &self.commit_history[self.commit_history.len() - 2];
        let last = self.commit_history.last().unwrap();
        let combined_py = format!("{}{}", prev.0, last.0);
        let combined_word = format!("{}{}", prev.1, last.1);
        if combined_word.chars().count() <= max_len {
            results.push((combined_py, combined_word));
        }
        results
    }

    pub fn get_current_profile(&self) -> String {
        self.active_profiles
            .first()
            .cloned()
            .unwrap_or_else(|| "chinese".to_string())
    }

    pub fn is_stroke_mode(&self) -> bool {
        self.active_profiles.contains(&"stroke".to_string())
    }

    pub fn is_english_mode(&self) -> bool {
        self.active_profiles.len() == 1 && self.active_profiles[0] == "english"
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}
