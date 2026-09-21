use std::collections::VecDeque;
use std::time::Instant;

/// 记录一次上屏里 record_usage 实际写入的学习效果，供「退格重打撤销误学习」精确回退。
#[derive(Debug, Clone, Default)]
pub struct LearnEffects {
    /// 是否写了 user_order（MRU 排序）
    pub order: bool,
    /// bigram ngram key（上一个词）
    pub ngram_bigram: Option<String>,
    /// trigram ngram key（"prev2|prev1"）
    pub ngram_trigram: Option<String>,
    /// 新词发现写入 learned_words 的 (拼音, 词)
    pub discovered: Option<(String, String)>,
    /// ngram 驱动新词合成写入 learned_words 的 (拼音, 合成词) 列表
    pub synthesized: Vec<(String, String)>,
}

/// 一次上屏的记录，用于「退格重打撤销误学习」。
/// 对应青简的 recent_commits：上屏后组句外按退格会从最近一次往前消耗这些记录，
/// 重新输入相同（或其前缀）拼音选不同词时，撤销该次上屏产生的学习。
#[derive(Debug, Clone)]
pub struct CommitRecord {
    /// 是否参与了学习（learned / long-term / ngram / order）。
    /// false 表示标点、英文、原样上屏这类“没学习”的上屏，只留长度好让退格数过去。
    pub learned: bool,
    /// 上屏时的方案名
    pub profile: String,
    /// 上屏时的拼音（last_lookup_pinyin）
    pub pinyin: String,
    /// 上屏的文本
    pub word: String,
    /// 上屏文本的字符数（退格计数用）
    pub word_chars: usize,
    /// 该记录是否已被退格完全删掉（等待重打）
    pub deleted: bool,
    /// 本次上屏产生的学习效果（回退用）
    pub effects: LearnEffects,
}

const MAX_RECENT_COMMITS: usize = 4;

/// 规范化拼音：去空格与音节分隔符、小写，用于退格重打的匹配比较。
fn normalize_pinyin(py: &str) -> String {
    py.chars()
        .filter(|c| !c.is_whitespace() && *c != '\'' && *c != '`')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 重打的拼音与已删记录的拼音匹配规则（对应青简的“其中一段拼音（或其前缀）”）。
fn normalized_pinyin_matches(record_py: &str, new_py: &str) -> bool {
    let a = normalize_pinyin(record_py);
    let b = normalize_pinyin(new_py);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.starts_with(&b) || b.starts_with(&a)
}

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
    /// 上一次 CapsLock 按下是否已透传给系统（用于对称转发与之配对的 release）
    pub capslock_passed_press: bool,
    pub capslock_down: bool,
    /// 组句外退格累计删掉的字符数（自最近一次上屏后），用于退格撤销误学习
    pub pending_backspaces: usize,
    /// 最近几次上屏（最近的在前），用于退格撤销误学习
    pub recent_commits: VecDeque<CommitRecord>,
    /// 最后一次上屏的 (拼音, 词)，用于打错检测（同拼音换词则衰减旧词）
    last_committed: Option<(String, String)>,
    last_committed_time: Instant,
    /// 中文模式下最后一次按键的时间，用于自动切回英文
    pub last_ime_activity: Instant,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            active_profiles: Vec::new(),
            chinese_enabled: false,
            ime_enabled: true,
            traditional_enabled: false,
            commit_history: Vec::new(),
            last_commit_time: Instant::now(),
            capslock_pending: false,
            caps_lock_enabled: false,
            capslock_passed_press: false,
            capslock_down: false,
            pending_backspaces: 0,
            recent_commits: VecDeque::new(),
            last_committed: None,
            last_committed_time: Instant::now(),
            last_ime_activity: Instant::now(),
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

    /// 记录一次上屏（最近的在前，最多 MAX_RECENT_COMMITS 条），用于退格撤销误学习。
    pub fn push_commit_record(&mut self, record: CommitRecord) {
        self.recent_commits.push_front(record);
        if self.recent_commits.len() > MAX_RECENT_COMMITS {
            self.recent_commits.truncate(MAX_RECENT_COMMITS);
        }
    }

    /// 标点、英文、原样上屏这些“没学习”的上屏也留一条只有长度的记录，
    /// 让退格能数过它们，够到更早的、有学习的记录。
    pub fn push_no_learning_record(&mut self, text: &str) {
        self.push_commit_record(CommitRecord {
            learned: false,
            profile: "".to_string(),
            pinyin: String::new(),
            word: text.to_string(),
            word_chars: text.chars().count(),
            deleted: false,
            effects: LearnEffects::default(),
        });
    }

    /// 组句外的退格，会删掉最近一次上屏的一个字符。
    /// 从最近一次往前累计，一条记录被删满 word_chars 个字符就标记 deleted（候着重打）。
    pub fn note_backspace(&mut self) {
        self.pending_backspaces = self.pending_backspaces.saturating_add(1);
        self.recompute_deleted_flags();
    }

    /// 重打后选定了一个新词：在“已被退格完全删掉”的记录里，找拼音匹配的那条。
    /// 返回较旧的一条匹配记录（matched 语义：重打其中一段拼音，或它的前缀）。
    /// 同时处理三类收尾：
    /// - 同词重打：只把记录丢掉，不撤销学习；
    /// - 拼音谁都对不上：当作在改别处，全忘掉；
    /// - 删得比记着的加起来还多：全忘掉。
    pub fn resolve_backspace_undo(
        &mut self,
        new_pinyin: &str,
        new_word: &str,
    ) -> Option<CommitRecord> {
        if self.pending_backspaces == 0 {
            return None;
        }

        // 删得比记录的总字符还多：当作完全改了别处，全忘掉。
        let total: usize = self.recent_commits.iter().map(|r| r.word_chars).sum();
        if self.pending_backspaces > total {
            self.forget_backspace_tracking();
            return None;
        }

        let matches = |rec: &CommitRecord| {
            rec.deleted
                && !rec.pinyin.is_empty()
                && normalized_pinyin_matches(&rec.pinyin, new_pinyin)
        };

        // 从最近的一条开始找（recent_commits 队首是最近的一次上屏），
        // 优先撤销离光标最近的那条 deleted 且拼音匹配的记录。
        for i in 0..self.recent_commits.len() {
            if matches(&self.recent_commits[i]) {
                let record = self.recent_commits.remove(i).unwrap();
                self.pending_backspaces =
                    self.pending_backspaces.saturating_sub(record.word_chars);
                self.recompute_deleted_flags();
                if record.word == new_word {
                    // 同词重打：记录丢掉即可，不撤销学习
                    return None;
                }
                return Some(record);
            }
        }

        // 有退格但谁也对不上：当作改别处
        self.forget_backspace_tracking();
        None
    }

    /// 全忘掉退格撤销的状态（当作在改别处）。
    pub fn forget_backspace_tracking(&mut self) {
        self.pending_backspaces = 0;
        for rec in self.recent_commits.iter_mut() {
            rec.deleted = false;
        }
    }

    /// 根据当前的 pending_backspaces 重新标记各记录是否被退格完全删掉。
    fn recompute_deleted_flags(&mut self) {
        let mut remaining = self.pending_backspaces;
        for rec in self.recent_commits.iter_mut() {
            rec.deleted = false;
            if remaining == 0 {
                break;
            }
            if remaining >= rec.word_chars {
                rec.deleted = true;
            }
            remaining = remaining.saturating_sub(rec.word_chars);
        }
        // 被退格完全删掉的词不再算作 ngram 上下文（avoid_history 与
        // recent_commits 是同一状态的两个视图：词被删掉后从历史里同步剔除）。
        self.commit_history.retain(|(py, w)| {
            !self.recent_commits.iter().any(|r| r.deleted && r.pinyin == *py && r.word == *w)
        });
    }

    pub fn get_last_word(&self) -> Option<&str> {
        self.commit_history.last().map(|(_, w)| w.as_str())
    }

    pub fn get_last_two_words(&self) -> Option<(&str, &str)> {
        if self.commit_history.len() >= 2 {
            let n2 = &self.commit_history[self.commit_history.len() - 2];
            let n1 = &self.commit_history[self.commit_history.len() - 1];
            Some((n2.1.as_str(), n1.1.as_str()))
        } else {
            None
        }
    }

    pub fn update_commit_time(&mut self) {
        self.last_commit_time = Instant::now();
    }

    pub fn get_current_profile(&self) -> String {
        self.active_profiles
            .first()
            .cloned()
            .unwrap_or_else(|| "chinese".to_string())
    }

    pub fn is_stroke_mode(&self) -> bool {
        self.active_profiles.iter().any(|p| p == "stroke" || p == "bihua")
    }

    pub fn is_pybi_mode(&self) -> bool {
        self.active_profiles.iter().any(|p| p == "pybi")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_learned(py: &str, word: &str) -> CommitRecord {
        CommitRecord {
            learned: true,
            profile: "chinese".to_string(),
            pinyin: py.to_string(),
            word: word.to_string(),
            word_chars: word.chars().count(),
            deleted: false,
            effects: LearnEffects::default(),
        }
    }

    fn make_plain(word: &str) -> CommitRecord {
        CommitRecord {
            learned: false,
            profile: String::new(),
            pinyin: String::new(),
            word: word.to_string(),
            word_chars: word.chars().count(),
            deleted: false,
            effects: LearnEffects::default(),
        }
    }

    #[test]
    fn test_normalize_pinyin_matches_prefix() {
        assert!(normalized_pinyin_matches("wo'de", "wode"));
        assert!(normalized_pinyin_matches("WO DE", "wode"));
        assert!(normalized_pinyin_matches("wode", "wo"));
        assert!(normalized_pinyin_matches("wo", "wode"));
        assert!(!normalized_pinyin_matches("wode", "niha"));
        assert!(!normalized_pinyin_matches("wo", "hao"));
    }

    #[test]
    fn test_same_word_recommit_no_undo() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wode", "我的"));
        s.note_backspace();
        s.note_backspace();
        let rec = s.resolve_backspace_undo("wode", "我的");
        assert!(rec.is_none(), "同词重打不撤销学习");
        assert_eq!(s.pending_backspaces, 0);
    }

    #[test]
    fn test_different_word_recommit_undoes() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wode", "我的"));
        s.note_backspace();
        s.note_backspace();
        let rec = s.resolve_backspace_undo("wode", "沃德");
        let rec = rec.expect("拼音匹配且选不同词应返回记录");
        assert_eq!(rec.word, "我的");
        assert_eq!(s.pending_backspaces, 0);
    }

    #[test]
    fn test_recommit_prefix_matches() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wodeshu", "我的书"));
        s.note_backspace();
        s.note_backspace();
        s.note_backspace();
        // 重打部分拼音（前缀）
        let rec = s.resolve_backspace_undo("wode", "沃德");
        let rec = rec.expect("重打前缀应匹配");
        assert_eq!(rec.word, "我的书");
    }

    #[test]
    fn test_pinyin_mismatch_forgets_all() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wode", "我的"));
        s.note_backspace();
        s.note_backspace();
        let rec = s.resolve_backspace_undo("niha", "你好");
        assert!(rec.is_none());
        assert_eq!(s.pending_backspaces, 0, "对不上就全忘掉");
        assert!(!s.recent_commits[0].deleted);
    }

    #[test]
    fn test_delete_more_than_total_forgets() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wode", "我的"));
        for _ in 0..5 {
            s.note_backspace();
        }
        let rec = s.resolve_backspace_undo("wode", "我的");
        assert!(rec.is_none());
        assert_eq!(s.pending_backspaces, 0);
    }

    #[test]
    fn test_deleted_commit_no_longer_ngram_context() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wodeshu", "我的书"));
        s.add_to_history("wodeshu".to_string(), "我的书".to_string());
        assert_eq!(s.get_last_word(), Some("我的书"));
        // 退格完全删掉
        s.note_backspace();
        s.note_backspace();
        s.note_backspace();
        // 删除的词不能再作为 ngram 上文（否则撤销/重打会用错上下文学习）
        assert_eq!(s.get_last_word(), None);
        // 重打别的词后，新的词才是上下文
        s.push_commit_record(make_learned("wode", "我的"));
        s.add_to_history("wode".to_string(), "我的".to_string());
        assert_eq!(s.get_last_word(), Some("我的"));
    }

    #[test]
    fn test_plain_record_length_crossing() {
        let mut s = SessionState::new();
        s.push_commit_record(make_plain("，"));
        s.push_commit_record(make_learned("wode", "我的"));
        // 删掉“我的”+标点：退格三次，标点那条先被数过去
        s.note_backspace();
        s.note_backspace();
        s.note_backspace();
        // 重打 wode 选不同词，应命中“我的”
        let rec = s.resolve_backspace_undo("wode", "沃德");
        let rec = rec.expect("能越过标点记录命中更早的词");
        assert_eq!(rec.word, "我的");
    }

    #[test]
    fn test_newest_matching_record_priority() {
        let mut s = SessionState::new();
        // 最近一次(队首) my 记录 + 更早一条 wode 记录，两条都被删光
        s.push_commit_record(make_learned("wode", "我的"));
        s.push_commit_record(make_learned("woshu", "我书"));
        for _ in 0..2 {
            s.note_backspace();
        }
        for _ in 0..2 {
            s.note_backspace();
        }
        // 重打 woshu 选同词：应优先命中最新那条（"我书"），同词只丢记录不撤销
        let rec = s.resolve_backspace_undo("woshu", "我书");
        assert!(rec.is_none(), "同词重打只丢记录");
        assert_eq!(s.recent_commits.len(), 1);
        assert_eq!(s.recent_commits[0].word, "我的");
    }

    #[test]
    fn test_partial_deletion_not_undo() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wode", "我的"));
        s.note_backspace();
        // 只删了一个字，不足以撤销整个词的学习
        assert!(s.resolve_backspace_undo("wode", "我的").is_none());
        assert_eq!(s.recent_commits[0].word_chars, 2);
        assert!(!s.recent_commits[0].deleted);
    }

    #[test]
    fn test_pending_reset_on_commit_record_push() {
        let mut s = SessionState::new();
        s.push_commit_record(make_learned("wode", "我的"));
        s.note_backspace();
        let rec = s.resolve_backspace_undo("wode", "我的");
        assert!(rec.is_none());
        assert_eq!(s.pending_backspaces, 0);
    }
}
