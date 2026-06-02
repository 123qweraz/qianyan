use crate::keys::VirtualKey;
use crate::pipeline::Candidate;
use crate::processor::{FilterMode, ImeState};

const MAX_BUFFER_LEN: usize = 64;

#[derive(Debug, Clone)]
pub struct InputSession {
    pub buffer: String,
    pub candidates: Vec<Candidate>,
    pub selected: usize,
    pub page: usize,
    pub cursor_pos: usize,
    pub joined_sentence: String,
    pub last_lookup_pinyin: String,
    pub state: ImeState,
    pub nav_mode: bool,
    pub switch_mode: bool,
    pub aux_filter: String,
    pub filter_mode: FilterMode,
    pub page_snapshot: Vec<Candidate>,
    pub shift_used_as_modifier: bool,

    pub best_segmentation: Vec<String>,
    pub phantom_text: String,
    pub preview_selected_candidate: bool,
    pub last_blocked_buffer: String,
    pub has_dict_match: bool,

    pub quote_open: bool,
    pub single_quote_open: bool,

    /// Non-modifier key whose press was consumed by the IME.
    /// Used to ensure the corresponding release is also consumed,
    /// preventing stray key events from being forwarded to the application
    /// after auto-commit or SPACE commit clears the composing buffer.
    pub consumed_press_key: Option<VirtualKey>,

    /// 模糊音是否已激活
    pub fuzzy_activated: bool,
    /// 当前输入会话的翻页次数
    pub fuzzy_page_turns: usize,

    /// Vim 编辑模式（Tab+I 切换），为 true 时在光标位置插入字符
    pub insert_mode: bool,
    /// 连按计数（用于 dd 清空缓冲区）
    pub tab_d_count: u8,
}

impl Default for InputSession {
    fn default() -> Self {
        Self::new()
    }
}

impl InputSession {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            candidates: Vec::new(),
            selected: 0,
            page: 0,
            cursor_pos: 0,
            joined_sentence: String::new(),
            last_lookup_pinyin: String::new(),
            state: ImeState::Idle,
            nav_mode: false,
            switch_mode: false,
            aux_filter: String::new(),
            filter_mode: FilterMode::None,
            page_snapshot: Vec::new(),
            shift_used_as_modifier: false,

            best_segmentation: Vec::new(),
            phantom_text: String::new(),
            preview_selected_candidate: false,
            last_blocked_buffer: String::new(),
            has_dict_match: false,

            quote_open: false,
            single_quote_open: false,

            consumed_press_key: None,

            fuzzy_activated: false,
            fuzzy_page_turns: 0,

            insert_mode: false,
            tab_d_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.clear_composing();
        self.switch_mode = false;
        self.quote_open = false;
        self.single_quote_open = false;
        self.fuzzy_activated = false;
        self.fuzzy_page_turns = 0;
    }

    pub fn clear_composing(&mut self) {
        self.buffer.clear();
        self.candidates.clear();
        self.best_segmentation.clear();
        self.joined_sentence.clear();
        self.selected = 0;
        self.page = 0;
        self.state = ImeState::Idle;

        self.phantom_text.clear();
        self.preview_selected_candidate = false;
        self.cursor_pos = 0;
        self.aux_filter.clear();
        self.filter_mode = FilterMode::None;
        self.page_snapshot.clear();
        self.nav_mode = false;
        self.consumed_press_key = None;
        self.fuzzy_activated = false;
        self.fuzzy_page_turns = 0;
    }

    pub fn push_char(&mut self, c: char) {
        if self.buffer.len() >= MAX_BUFFER_LEN {
            return;
        }
        self.buffer.push(c);
        if self.state == ImeState::Idle {
            self.state = ImeState::Composing;
        }
        self.preview_selected_candidate = false;
        self.fuzzy_activated = false;
        self.fuzzy_page_turns = 0;
    }

    pub fn push_str(&mut self, s: &str) -> bool {
        let available = MAX_BUFFER_LEN.saturating_sub(self.buffer.len());
        let to_push = &s[..s.len().min(available)];
        if to_push.is_empty() {
            return false;
        }
        self.buffer.push_str(to_push);
        if self.state == ImeState::Idle {
            self.state = ImeState::Composing;
        }
        self.preview_selected_candidate = false;
        self.fuzzy_activated = false;
        self.fuzzy_page_turns = 0;
        true
    }

    pub fn pop_char(&mut self) -> bool {
        if self.buffer.is_empty() {
            return false;
        }
        self.buffer.pop();
        if self.buffer.is_empty() {
            self.reset();
        }
        true
    }

    // ── Vim 编辑模式：光标移动 ──

    pub fn move_cursor_left(&mut self) -> bool {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            true
        } else {
            false
        }
    }

    pub fn move_cursor_right(&mut self) -> bool {
        if self.cursor_pos < self.buffer.len() {
            self.cursor_pos += 1;
            true
        } else {
            false
        }
    }

    pub fn move_cursor_start(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.buffer.len();
    }

    /// 按音节跳转（W 前跳 / B 后跳）
    pub fn move_cursor_by_syllable(&mut self, forward: bool) -> bool {
        if self.buffer.is_empty() || self.best_segmentation.is_empty() {
            // Fallback: jump by whole buffer
            if forward {
                return self.cursor_pos < self.buffer.len() && {
                    self.move_cursor_end();
                    true
                };
            } else {
                return self.cursor_pos > 0 && {
                    self.move_cursor_start();
                    true
                };
            }
        }

        // 使用 best_segmentation 计算音节边界位置
        let boundaries = syllable_boundaries(&self.best_segmentation, self.buffer.len());
        if boundaries.is_empty() {
            return false;
        }

        if forward {
            // 跳到下一个音节边界（光标后的第一个边界）
            for &b in &boundaries {
                if b > self.cursor_pos {
                    self.cursor_pos = b;
                    return true;
                }
            }
            // Already at or past last boundary, jump to end
            if self.cursor_pos < self.buffer.len() {
                self.move_cursor_end();
                return true;
            }
        } else {
            // 跳到上一个音节边界（光标前的最后一个边界）
            for &b in boundaries.iter().rev() {
                if b < self.cursor_pos {
                    self.cursor_pos = b;
                    return true;
                }
            }
            if self.cursor_pos > 0 {
                self.move_cursor_start();
                return true;
            }
        }
        false
    }

    // ── Vim 编辑模式：插入/删除 ──

    /// 在光标处插入字符（insert_mode）或替换（非 insert_mode）
    pub fn insert_char_at_cursor(&mut self, c: char) {
        if self.buffer.len() >= MAX_BUFFER_LEN {
            return;
        }
        if self.insert_mode {
            self.buffer.insert(self.cursor_pos, c);
        } else {
            if self.cursor_pos < self.buffer.len() {
                let chars: Vec<char> = self.buffer.chars().collect();
                let mut new_buf = String::new();
                for (i, ch) in chars.iter().enumerate() {
                    if i == self.cursor_pos {
                        new_buf.push(c);
                    } else {
                        new_buf.push(*ch);
                    }
                }
                self.buffer = new_buf;
            } else {
                self.buffer.push(c);
            }
        }
        self.cursor_pos += 1;
        if self.state == ImeState::Idle {
            self.state = ImeState::Composing;
        }
        self.preview_selected_candidate = false;
    }

    /// 删除光标处的一个字符
    pub fn delete_at_cursor(&mut self) -> bool {
        if self.buffer.is_empty() || self.cursor_pos == 0 {
            return false;
        }
        let remove_idx = self.cursor_pos - 1;
        if remove_idx < self.buffer.len() {
            self.buffer.remove(remove_idx);
            self.cursor_pos = self.cursor_pos.saturating_sub(1);
            if self.buffer.is_empty() {
                self.reset();
            }
            true
        } else {
            false
        }
    }

    /// 按音节删除（Word delete）
    pub fn delete_syllable_at_cursor(&mut self) -> bool {
        if self.buffer.is_empty() {
            return false;
        }
        if self.best_segmentation.is_empty() {
            return self.pop_char();
        }

        let boundaries = syllable_boundaries(&self.best_segmentation, self.buffer.len());
        if boundaries.is_empty() {
            return self.pop_char();
        }

        // 找到光标所在的音节区间，删除它
        let mut start = 0usize;
        for &b in &boundaries {
            if self.cursor_pos <= b {
                let chars: Vec<char> = self.buffer.chars().collect();
                let mut new_buf = String::new();
                for (i, ch) in chars.iter().enumerate() {
                    if i < start || i >= b {
                        new_buf.push(*ch);
                    }
                }
                self.buffer = new_buf;
                self.cursor_pos = start;
                if self.buffer.is_empty() {
                    self.reset();
                }
                return true;
            }
            start = b;
        }
        // 在最后一个音节后，删除末尾音节
        if start < self.buffer.len() {
            let chars: Vec<char> = self.buffer.chars().collect();
            let mut new_buf = String::new();
            for (i, ch) in chars.iter().enumerate() {
                if i < start {
                    new_buf.push(*ch);
                }
            }
            self.buffer = new_buf;
            self.cursor_pos = start;
            if self.buffer.is_empty() {
                self.reset();
            }
            return true;
        }
        false
    }

    /// dd / Tab+DD: 清空缓冲区
    pub fn clear_buffer(&mut self) {
        self.reset();
    }

    // ── Vim 编辑模式：模糊音切换 ──

    /// Tab+S: 切换光标所在音节的模糊音（sh↔s, ch↔c, zh↔z）
    pub fn toggle_syllable_fuzzy(&mut self) -> bool {
        if self.buffer.is_empty() {
            return false;
        }
        // 找到光标所在的音节范围
        let mut start = 0usize;
        let mut end = self.buffer.len();
        if !self.best_segmentation.is_empty() {
            let boundaries = syllable_boundaries(&self.best_segmentation, self.buffer.len());
            for &b in &boundaries {
                if self.cursor_pos <= b {
                    end = b;
                    break;
                }
                start = b;
            }
        }

        if start >= self.buffer.len() {
            return false;
        }
        let syllable: String = self.buffer[start..end.min(self.buffer.len())].to_string();
        if syllable.is_empty() {
            return false;
        }

        // Toggle: zh↔z, ch↔c, sh↔s (以及反向)
        let toggled: String;
        if syllable.starts_with("zh") {
            toggled = syllable.replacen("zh", "z", 1);
        } else if syllable.starts_with("ch") {
            toggled = syllable.replacen("ch", "c", 1);
        } else if syllable.starts_with("sh") {
            toggled = syllable.replacen("sh", "s", 1);
        } else if syllable.starts_with('z') && syllable.len() >= 2 {
            if syllable.as_bytes().get(1) != Some(&b'h') {
                toggled = "zh".to_string() + &syllable[1..];
            } else {
                return false;
            }
        } else if syllable.starts_with('c') && syllable.len() >= 2 {
            if syllable.as_bytes().get(1) != Some(&b'h') {
                toggled = "ch".to_string() + &syllable[1..];
            } else {
                return false;
            }
        } else if syllable.starts_with('s') && syllable.len() >= 2 {
            if syllable.as_bytes().get(1) != Some(&b'h') {
                toggled = "sh".to_string() + &syllable[1..];
            } else {
                return false;
            }
        } else {
            return false;
        }

        if toggled.is_empty() || toggled == syllable {
            return false;
        }

        let chars: Vec<char> = self.buffer.chars().collect();
        let mut new_buf = String::new();
        for (i, ch) in chars.iter().enumerate() {
            if i < start {
                new_buf.push(*ch);
            } else if i >= end.min(self.buffer.len()) {
                new_buf.push(*ch);
            }
        }
        new_buf.insert_str(start, &toggled);
        self.buffer = new_buf;
        self.preview_selected_candidate = false;
        true
    }

    pub fn next_candidate(&mut self, page_size: usize) {
        if !self.candidates.is_empty() {
            self.preview_selected_candidate = true;
            if self.selected + 1 < self.candidates.len() {
                self.selected += 1;
            }
            self.page = (self.selected / page_size) * page_size;
        }
    }

    pub fn prev_candidate(&mut self, page_size: usize) {
        if !self.candidates.is_empty() {
            self.preview_selected_candidate = true;
            if self.selected > 0 {
                self.selected -= 1;
            }
            self.page = (self.selected / page_size) * page_size;
        }
    }

    pub fn next_page(&mut self, page_size: usize) {
        if !self.candidates.is_empty() && self.page + page_size < self.candidates.len() {
            self.page += page_size;
            self.selected = self.page;
        }
        self.fuzzy_page_turns += 1;
    }

    pub fn prev_page(&mut self, page_size: usize) {
        self.page = self.page.saturating_sub(page_size);
        self.selected = self.page;
    }

    pub fn handle_filter_char(&mut self, c: char) {
        if self.filter_mode == FilterMode::None {
            self.filter_mode = FilterMode::Page;
            self.page_snapshot = self.candidates.clone();
            self.aux_filter = c.to_string();
        } else {
            self.aux_filter.push(c);
        }
        self.selected = 0;
        if self.filter_mode == FilterMode::Global {
            self.page = 0;
        }
    }

    pub fn pop_filter(&mut self) {
        if !self.aux_filter.is_empty() {
            self.aux_filter.pop();
            if self.aux_filter.is_empty() {
                self.filter_mode = FilterMode::None;
                self.page_snapshot.clear();
                self.page = 0;
            } else {
                self.selected = 0;
                if self.filter_mode == FilterMode::Global {
                    self.page = 0;
                }
            }
        }
    }

    pub fn update_state(&mut self) {
        if self.buffer.is_empty() {
            self.state = ImeState::Idle;
        } else if self.state == ImeState::Idle {
            self.state = ImeState::Composing;
        }
    }
}

/// 根据 best_segmentation 计算音节边界位置（每个音节的结束位置）
fn syllable_boundaries(segments: &[String], buffer_len: usize) -> Vec<usize> {
    let mut pos = 0usize;
    let mut boundaries = Vec::new();
    for seg in segments {
        pos += seg.len();
        if pos <= buffer_len {
            boundaries.push(pos);
        }
    }
    boundaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::ImeState;

    #[test]
    fn test_session_basic_ops() {
        let mut session = InputSession::new();
        assert_eq!(session.state, ImeState::Idle);

        session.push_char('n');
        assert_eq!(session.buffer, "n");
        assert_eq!(session.state, ImeState::Composing);

        session.pop_char();
        assert_eq!(session.buffer, "");
        assert_eq!(session.state, ImeState::Idle);
    }

    #[test]
    fn test_session_state_updates() {
        let mut session = InputSession::new();
        session.buffer = "nh".to_string();
        session.update_state();
        assert_eq!(session.state, ImeState::Composing);

        session.buffer.clear();
        session.update_state();
        assert_eq!(session.state, ImeState::Idle);
    }

    #[test]
    fn test_session_reset() {
        let mut session = InputSession::new();
        session.push_char('a');
        session.nav_mode = true;
        session.reset();
        assert!(session.buffer.is_empty());
        assert!(!session.nav_mode);
        assert_eq!(session.state, ImeState::Idle);
    }

    #[test]
    fn test_session_push_pop() {
        let mut session = InputSession::new();
        session.push_char('a');
        session.push_char('b');
        session.push_char('c');
        assert_eq!(session.buffer, "abc");

        session.pop_char();
        assert_eq!(session.buffer, "ab");

        session.pop_char();
        session.pop_char();
        assert_eq!(session.buffer, "");
    }

    #[test]
    fn test_session_page_navigation() {
        let mut session = InputSession::new();
        session.candidates = vec![
            create_candidate("a"),
            create_candidate("b"),
            create_candidate("c"),
            create_candidate("d"),
            create_candidate("e"),
        ];

        session.selected = 2;
        session.next_candidate(3);
        assert_eq!(session.selected, 3);

        session.next_candidate(3);
        assert_eq!(session.selected, 4);

        session.prev_candidate(3);
        assert_eq!(session.selected, 3);
    }

    #[test]
    fn test_session_clear_composing() {
        let mut session = InputSession::new();
        session.buffer = "test".to_string();
        session.phantom_text = "best".to_string();
        session.clear_composing();
        assert!(session.buffer.is_empty());
        assert!(session.phantom_text.is_empty());
        assert_eq!(session.state, ImeState::Idle);
    }

    fn create_candidate(text: &str) -> crate::pipeline::Candidate {
        use std::sync::Arc;
        crate::pipeline::Candidate {
            text: Arc::from(text),
            simplified: Arc::from(text),
            traditional: Arc::from(text),
            hint: Arc::from(""),
            english_aux: Arc::from(""),
            stroke_aux: Arc::from(""),
            source: Arc::from("test"),
            weight: 1.0,
            match_level: 3,
        }
    }
}
