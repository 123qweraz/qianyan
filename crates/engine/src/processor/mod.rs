pub mod actor;
pub mod commands;
pub mod fsm;
pub mod handlers;
pub mod intents;
mod learning;
pub mod punctuation;
pub mod session_state;
pub mod utils;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::compositor::Compositor;
use crate::keys::VirtualKey;
use crate::{Command, EngineContext, InputEvent};
use qianyan_ime_core::config::Config;

pub use fsm::ImeState;
pub use utils::*;

pub fn inject_text(ctx: &mut EngineContext, text: &str) -> Action {
    use crate::compositor::Compositor;
    use crate::pipeline::lookup;

    ctx.session.push_str(text);
    if ctx.session.state == ImeState::Idle && !ctx.session.buffer.is_empty() {
        ctx.session.state = ImeState::Composing;
    }
    ctx.session.preview_selected_candidate = false;
    if let Some(act) = lookup(ctx) {
        return act;
    }
    if let Some(act) = Compositor::check_auto_commit(ctx) {
        return act;
    }
    Compositor::update_phantom_action(ctx)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Emit(String),
    DeleteAndEmit { delete: usize, insert: String },
    PassThrough,
    Consume,
    Alert,
    Notify(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterMode {
    None,
    Global,
    Page,
}

pub struct Processor {
    pub ctx: EngineContext,
}

impl Processor {
    pub fn new(
        trie_paths: HashMap<String, (std::path::PathBuf, std::path::PathBuf)>,
        syllables: HashSet<String>,
        syllable_freq: HashMap<String, u64>,
    ) -> Self {
        Self {
            ctx: EngineContext::new(trie_paths, syllables, syllable_freq),
        }
    }

    pub fn new_with_engine(
        engine: crate::pipeline::SearchEngine,
        syllables: HashSet<String>,
    ) -> Self {
        Self {
            ctx: EngineContext::new_with_engine(engine, syllables),
        }
    }

    pub fn apply_config(&mut self, conf: &Config) {
        self.ctx.apply_config(conf);
    }

    pub fn handle_event(&mut self, event: InputEvent) -> Action {
        log::trace!("handle_event: {:?}", event);
        match event {
            InputEvent::Key {
                key,
                val,
                shift,
                ctrl,
                alt,
            } => self.handle_key_ext(key, val, shift, ctrl, alt, true),
            InputEvent::Voice(text) => {
                if !text.is_empty() {
                    self.reset();
                    return Action::Emit(text);
                }
                Action::Consume
            }
            InputEvent::CandidateSelect(idx) => self.execute_command(Command::Select(idx)),
        }
    }

    pub fn handle_key(
        &mut self,
        key: VirtualKey,
        val: i32,
        shift_pressed: bool,
        ctrl_pressed: bool,
        alt_pressed: bool,
    ) -> Action {
        self.handle_event(InputEvent::Key {
            key,
            val,
            shift: shift_pressed,
            ctrl: ctrl_pressed,
            alt: alt_pressed,
        })
    }

    pub fn toggle(&mut self) -> Action {
        self.ctx.session_state.chinese_enabled = !self.ctx.session_state.chinese_enabled;
        let enabled = self.ctx.session_state.chinese_enabled;
        let short = self.get_short_display();
        self.reset();

        if enabled {
            Action::Notify(short, "模式已开启".into())
        } else {
            Action::Notify("英".into(), "英文直通模式".into())
        }
    }

    pub fn toggle_enabled(&mut self) -> Action {
        self.ctx.session_state.ime_enabled = !self.ctx.session_state.ime_enabled;
        let enabled = self.ctx.session_state.ime_enabled;
        self.reset();
        if enabled {
            Action::Notify("中".into(), "输入法已激活".into())
        } else {
            Action::Notify("禁".into(), "输入法已停用".into())
        }
    }

    pub fn next_profile(&mut self) -> String {
        let enabled: Vec<String> = self
            .ctx
            .config
            .master_config
            .input
            .enabled_profiles
            .iter()
            .filter(|p| self.ctx.engine.trie_paths.contains_key(*p))
            .cloned()
            .collect();

        if enabled.is_empty() {
            return self
                .ctx
                .session_state
                .active_profiles
                .first()
                .cloned()
                .unwrap_or_else(|| "chinese".to_string());
        }

        let current = self
            .ctx
            .session_state
            .active_profiles
            .first()
            .cloned()
            .unwrap_or_else(|| enabled[0].clone());

        let idx = enabled.iter().position(|p| p == &current).unwrap_or(0);
        let next_idx = (idx + 1) % enabled.len();
        let next = enabled[next_idx].clone();

        self.ctx.session_state.active_profiles = vec![next.clone()];
        
        // Persist to config
        if let Ok(conf) = self.ctx.config.master_config_write() {
            conf.input.default_profile = next.clone();
            let _ = conf.save();
        }

        self.reset();
        next
    }

    pub fn handle_key_ext(
        &mut self,
        key: VirtualKey,
        val: i32,
        shift_pressed: bool,
        ctrl_pressed: bool,
        alt_pressed: bool,
        perform_lookup: bool,
    ) -> Action {
        let now = Instant::now();
        let is_press = val == 1;
        let is_release = val == 0;

        if !self.ctx.session_state.ime_enabled {
            return Action::PassThrough;
        }

        if is_press && is_letter(key) {
            if let Some(c) = key_to_char(key, shift_pressed, false) {
                self.ctx.sound_manager.play_letter(c);
            }
        }

        // CapsLock+F 组合键：繁简体切换
        if is_press && key == VirtualKey::F && self.ctx.session_state.capslock_combo_active {
            self.ctx.session_state.capslock_combo_active = false;
            // 撤销 CapsLock 按下的语言切换
            self.ctx.session_state.chinese_enabled = !self.ctx.session_state.chinese_enabled;
            self.ctx.session_state.traditional_enabled = !self.ctx.session_state.traditional_enabled;
            let mode = if self.ctx.session_state.traditional_enabled { "繁體" } else { "简体" };
            return Action::Notify(mode.into(), format!("切換至 {} 模式", mode));
        }

        // 跟踪 Tab 键状态，用于以词定字（Tab + 键）
        if key == VirtualKey::Tab {
            if is_press {
                self.ctx.session_state.tab_down = true;
            }
            if is_release {
                self.ctx.session_state.tab_down = false;
            }
        }

        if is_press {
            if let Some(action) = self.handle_global_hotkey(key, ctrl_pressed, shift_pressed) {
                return action;
            }

            // CapsLock + letter: quick final input (takes priority over nav_mode)
            if is_press
                && self.ctx.session_state.capslock_down
                && is_letter(key)
                && self.ctx.config.master_config.enable_quick_finals
            {
                if let Some(c) = key_to_char(key, false, false) {
                    let finals = self.ctx.config.quick_finals();
                    if let Some(final_text) = finals.get(&c.to_string()) {
                        return inject_text(&mut self.ctx, final_text);
                    }
                }
            }

            if self.ctx.session.nav_mode && !self.ctx.session.buffer.is_empty() {
                // 检查是否是导航编辑键（D/S/R 等），放行到 handle_composing 处理
                let is_nav_edit = self.ctx.config.nav_delete_keys().contains(&key)
                    || self.ctx.config.nav_fuzzy_keys().contains(&key)
                    || self.ctx.config.nav_clear_keys().contains(&key);
                if is_nav_edit {
                    // 交给 handle_composing 处理，不退出 nav_mode
                } else {
                    match key {
                        VirtualKey::H => return self.execute_command(Command::PrevCandidate),
                        VirtualKey::L => return self.execute_command(Command::NextCandidate),
                        VirtualKey::J => return self.execute_command(Command::NextPage),
                        VirtualKey::K => return self.execute_command(Command::PrevPage),
                        _ => {
                            self.ctx.session.nav_mode = false;
                        }
                    }
                }
            }
            if self.ctx.session_state.capslock_pending
                && self.ctx.session.buffer.is_empty()
                && is_letter(key)
            {
                if let Some(action) = self.handle_capslock_profile_switch(key) {
                    return action;
                }
            }
        }

        if is_release && key == VirtualKey::CapsLock {
            self.ctx.session_state.capslock_down = false;
            self.ctx.session_state.capslock_combo_active = false;
            if !self.ctx.session_state.chinese_enabled {
                return Action::PassThrough;
            }
            return Action::Consume;
        }

        if !self.ctx.session_state.chinese_enabled {
            return Action::PassThrough;
        }

        if ctrl_pressed
            && alt_pressed
            && get_punctuation_key(key, shift_pressed).is_none()
        {
            return Action::PassThrough;
        }

        if ctrl_pressed && !alt_pressed && is_letter(key) {
            return Action::PassThrough;
        }

        if is_press && ctrl_pressed && !alt_pressed {
            if let Some(action) = self.handle_ctrl_punctuation(key, shift_pressed) {
                return action;
            }
        }

        if let Some(action) = intents::process_modifiers(&mut self.ctx, key, is_press, is_release) {
            return action;
        }
        if let Some(action) = intents::process_intent(&mut self.ctx, key, val, shift_pressed, now) {
            return action;
        }
        if key == VirtualKey::Grave {
            return Action::PassThrough;
        }
        if let Some(action) = intents::process_switch_mode(&mut self.ctx, key, is_press, is_release)
        {
            return action;
        }

        // FSM 状态机转换（处理 字母、Shift、Backspace 等核心逻辑）
        let fsm_action = self.handle_fsm_transition(
            key,
            shift_pressed,
            ctrl_pressed,
            alt_pressed,
            is_press,
            perform_lookup,
        );

        // Track consumed press so that the release is also consumed
        // (prevents stray release events after background auto-commit or SPACE commit)
        if is_press && !matches!(key, VirtualKey::Control | VirtualKey::Alt | VirtualKey::Shift | VirtualKey::CapsLock) {
            self.ctx.session.consumed_press_key = if fsm_action != Action::PassThrough { Some(key) } else { None };
        }

        if is_press && is_letter(key) && perform_lookup {
            if fsm_action != Action::PassThrough {
                return fsm_action;
            }
            if let Some(c) = key_to_char(key, shift_pressed, false) {
                self.ctx.session.push_char(c);
                if let Some(action) = self.lookup() {
                    return action;
                }
            }
        }

        fsm_action
    }

    fn handle_fsm_transition(
        &mut self,
        key: VirtualKey,
        shift_pressed: bool,
        ctrl_pressed: bool,
        alt_pressed: bool,
        is_press: bool,
        perform_lookup: bool,
    ) -> Action {
        use crate::ModifierState;

        // 在 FSM 之前检查自定义导航热键，避免 FSM 拦截后无法执行
        if is_press {
            let has_candidates = !self.ctx.session.candidates.is_empty();
            if matches!(self.ctx.session.state, ImeState::Composing | ImeState::Selecting)
                && has_candidates
            {
                if self.ctx.config.page_up_keys().contains(&key) {
                    return self.execute_command(Command::PrevPage);
                }
                if self.ctx.config.page_down_keys().contains(&key) {
                    let action = self.execute_command(Command::NextPage);
                    let threshold = self.ctx.config.master_config.input.fuzzy_config.fuzzy_page_threshold;
                    if threshold > 0
                        && self.ctx.session.fuzzy_page_turns >= threshold
                        && !self.ctx.session.fuzzy_activated
                        && self.ctx.session.has_dict_match
                    {
                        self.ctx.session.fuzzy_activated = true;
                        self.ctx.session.page = 0;
                        self.ctx.session.selected = 0;
                        if let Some(relook) = self.lookup_with_limit(crate::pipeline::MAX_LOOKUP_LIMIT) {
                            return relook;
                        }
                    }
                    return action;
                }
                if self.ctx.config.prev_candidate_keys().contains(&key) {
                    return self.execute_command(Command::PrevCandidate);
                }
                if self.ctx.config.next_candidate_keys().contains(&key) {
                    return self.execute_command(Command::NextCandidate);
                }
            }
        }

        let input = fsm::FsmInput {
            key,
            mods: ModifierState {
                shift: shift_pressed,
                ctrl: ctrl_pressed,
                alt: alt_pressed,
                meta: false,
            },
            buffer_empty: self.ctx.session.buffer.is_empty(),
            has_candidates: !self.ctx.session.candidates.is_empty(),
            is_stroke_mode: self.ctx.session_state.is_stroke_mode(),
        };

        let (new_state, effect) = fsm::StateMachine::transition(self.ctx.session.state, &input);
        self.ctx.session.state = new_state;

        if is_press && is_letter(key) && !self.ctx.session.nav_mode {
            self.ctx.session_state.capslock_pending = false;
        }

        match effect {
            fsm::FsmEffect::PassThrough => {
                if self.ctx.session.state == ImeState::Idle {
                    handlers::handle_idle(&mut self.ctx, key, shift_pressed, perform_lookup)
                } else {
                    Action::PassThrough
                }
            }
            fsm::FsmEffect::UpdateLookup => {
                handlers::handle_composing(&mut self.ctx, key, shift_pressed, perform_lookup)
            }
            fsm::FsmEffect::Commit首选 => self.execute_command(Command::Commit),
            fsm::FsmEffect::CommitRaw => self.execute_command(Command::CommitRaw),
            fsm::FsmEffect::Clear => self.execute_command(Command::Clear),
            fsm::FsmEffect::Consume => Action::Consume,
            fsm::FsmEffect::Alert => Action::Alert,
        }
    }

    pub fn update_phantom_action(&mut self) -> Action {
        if self.ctx.config.phantom_type() == qianyan_ime_core::config::PhantomType::None {
            return Action::Consume;
        }
        let target = crate::compositor::Compositor::get_phantom_text(&mut self.ctx);
        if target == self.ctx.session.phantom_text {
            return Action::Consume;
        }
        let old_phantom = self.ctx.session.phantom_text.clone();
        let old_chars: Vec<char> = old_phantom.chars().collect();
        let target_chars: Vec<char> = target.chars().collect();
        let mut common_prefix_len = 0;
        for (c1, c2) in old_chars.iter().zip(target_chars.iter()) {
            if c1 == c2 {
                common_prefix_len += 1;
            } else {
                break;
            }
        }
        let delete_count = old_chars.len() - common_prefix_len;
        let insert_text: String = target_chars[common_prefix_len..].iter().collect();
        self.ctx.session.phantom_text = target;
        if delete_count == 0 && insert_text.is_empty() {
            Action::Consume
        } else if delete_count == 0 {
            Action::Emit(insert_text)
        } else {
            Action::DeleteAndEmit {
                delete: delete_count,
                insert: insert_text,
            }
        }
    }

    pub fn lookup(&mut self) -> Option<Action> {
        self.lookup_with_limit(crate::pipeline::MAX_LOOKUP_LIMIT)
    }

    pub fn lookup_with_limit(&mut self, limit: usize) -> Option<Action> {
        log::debug!("lookup: buffer={}, limit={}", self.ctx.session.buffer, limit);
        if self.ctx.session.buffer.is_empty() {
            self.reset();
            return None;
        }

        if self.ctx.session.filter_mode == FilterMode::Page
            && !self.ctx.session.page_snapshot.is_empty()
        {
            let mut filtered = Vec::new();
            for c in &self.ctx.session.page_snapshot {
                if self
                    .ctx
                    .engine
                    .matches_filter(c, &self.ctx.session.aux_filter, self.ctx.config.master_config.input.english_aux_mode)
                {
                    filtered.push(c.clone());
                }
            }
            if !filtered.is_empty() {
                self.ctx.session.candidates = filtered;
                if self.ctx.session.candidates.len() == 1 {
                    let word = self.ctx.session.candidates[0].text.clone();
                    return Some(commands::commit_candidate(&mut self.ctx, word, 0));
                }
            } else {
                self.ctx.session.candidates.clear();
            }
            self.ctx.session.update_state();
            return None;
        }

        let current_profile = self
            .ctx
            .session_state
            .active_profiles
            .first()
            .cloned()
            .unwrap_or_default();
        let last_word = self
            .ctx
            .session_state
            .commit_history
            .last()
            .map(|(_, word)| word.as_str());

        // Sync runtime toggle state to config before lookup so filters see it
        self.ctx.config.master_config.input.enable_traditional =
            self.ctx.session_state.traditional_enabled;

        // 首次查询：可能启用或禁用模糊音
        let fuzzy_enabled = self.ctx.session.fuzzy_activated;
        let query = crate::pipeline::SearchQuery {
            buffer: &self.ctx.session.buffer,
            profile: &current_profile,
            config: &self.ctx.config.master_config,
            limit,
            filter_mode: self.ctx.session.filter_mode.clone(),
            aux_filter: &self.ctx.session.aux_filter,
            context: last_word,
            fuzzy_enabled,
        };
        let (results, segments) = self.ctx.engine.search(query);
        self.ctx.session.candidates = results;
        self.ctx.session.best_segmentation = segments;
        self.ctx.session.has_dict_match = !self.ctx.session.candidates.is_empty();
        self.ctx.session.last_lookup_pinyin = self.ctx.session.buffer.clone();

        // 如果没有精确匹配结果且模糊音尚未激活，自动激活模糊音重查
        if !self.ctx.session.has_dict_match && !self.ctx.session.fuzzy_activated {
            self.ctx.session.fuzzy_activated = true;
            let fuzzy_query = crate::pipeline::SearchQuery {
                buffer: &self.ctx.session.buffer,
                profile: &current_profile,
                config: &self.ctx.config.master_config,
                limit,
                filter_mode: self.ctx.session.filter_mode.clone(),
                aux_filter: &self.ctx.session.aux_filter,
                context: last_word,
                fuzzy_enabled: true,
            };
            let (fuzzy_results, fuzzy_segments) = self.ctx.engine.search(fuzzy_query);
            if !fuzzy_results.is_empty() {
                self.ctx.session.candidates = fuzzy_results;
                self.ctx.session.best_segmentation = fuzzy_segments;
                self.ctx.session.has_dict_match = true;
            }
        }

        // Global 模式：用辅助码过滤候选词
        if self.ctx.session.filter_mode == FilterMode::Global && !self.ctx.session.aux_filter.is_empty() {
            let mode = self.ctx.config.master_config.input.english_aux_mode;
            let aux_filter = self.ctx.session.aux_filter.clone();
            self
                .ctx
                .session
                .candidates
                .retain(|c| self.ctx.engine.matches_filter(c, &aux_filter, mode));
            self.ctx.session.has_dict_match = !self.ctx.session.candidates.is_empty();
        }

        if self.ctx.session.candidates.len() == 1
            && self.ctx.session.filter_mode == FilterMode::Global
        {
            let word = self.ctx.session.candidates[0].text.clone();
            return Some(commands::commit_candidate(&mut self.ctx, word, 0));
        }

        if self.ctx.session.candidates.is_empty() {
            let buf_arc: Arc<str> = Arc::from(self.ctx.session.buffer.as_str());
            self.ctx
                .session
                .candidates
                .push(crate::pipeline::Candidate {
                    text: buf_arc.clone(),
                    simplified: buf_arc.clone(),
                    traditional: buf_arc.clone(),
                    hint: Arc::from(""),
                    english_aux: Arc::from(""),
                    stroke_aux: Arc::from(""),
                    source: Arc::from("Raw"),
                    weight: 0.0,
                    match_level: 0,
                });
        }
        self.ctx.session.update_state();
        Compositor::check_auto_commit(&mut self.ctx)
    }

    pub fn reset(&mut self) {
        self.ctx.reset();
    }

    pub fn clear_composing(&mut self) {
        self.ctx.session.clear_composing();
    }

    pub fn get_short_display(&self) -> String {
        let display = self.get_current_profile_display();
        match display.to_lowercase().as_str() {
            "chinese" => "中".to_string(),
            "english" => "英".to_string(),
            "japanese" => "日".to_string(),
            "stroke" => "笔".to_string(),
            "mixed" => "混".to_string(),
            _ => display
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_else(|| " ".to_string()),
        }
    }

    pub fn get_current_profile_display(&self) -> String {
        if self.ctx.session_state.active_profiles.is_empty() {
            return "None".to_string();
        }
        if self.ctx.session_state.active_profiles.len() == 1 {
            return self.ctx.session_state.active_profiles[0].clone();
        }
        "Mixed".to_string()
    }

    fn handle_global_hotkey(
        &mut self,
        key: VirtualKey,
        ctrl_pressed: bool,
        shift_pressed: bool,
    ) -> Option<Action> {
        if key == VirtualKey::Space
            && ctrl_pressed
            && self
                .ctx
                .config
                .master_config
                .hotkeys
                .enable_ctrl_space_toggle
        {
            self.ctx.session_state.chinese_enabled = !self.ctx.session_state.chinese_enabled;
            self.ctx.session.clear_composing();
            return Some(Action::Consume);
        }

        if key == VirtualKey::CapsLock
            && ctrl_pressed
            && self
                .ctx
                .config
                .master_config
                .hotkeys
                .enable_ctrl_capslock_commit
        {
            self.ctx.session_state.chinese_enabled = false;
            if !self.ctx.session.buffer.is_empty() {
                let out = Arc::from(self.ctx.session.buffer.as_str());
                return Some(commands::commit_candidate(&mut self.ctx, out, 99));
            }
            return Some(Action::Consume);
        }

        if key == VirtualKey::Tab
            && self.ctx.session.buffer.is_empty()
            && self.ctx.config.master_config.hotkeys.enable_tab_toggle
        {
            self.ctx.session_state.chinese_enabled = !self.ctx.session_state.chinese_enabled;
            return Some(Action::Consume);
        }

        if key == VirtualKey::CapsLock {
            // Shift + CapsLock -> 切换大写锁定状态
            if shift_pressed {
                self.ctx.session_state.caps_lock_enabled =
                    !self.ctx.session_state.caps_lock_enabled;
                return Some(Action::PassThrough);
            }

            self.ctx.session_state.capslock_combo_active = true;

            // 单击 CapsLock -> 切换中英文模式；有内容时只设置 capslock_down
            if self.ctx.session.buffer.is_empty() {
                self.ctx.session_state.chinese_enabled = !self.ctx.session_state.chinese_enabled;
                return Some(Action::Consume);
            } else {
                self.ctx.session_state.capslock_down = true;
                return Some(Action::Consume);
            }
        }

        if key == VirtualKey::Tab && !self.ctx.session.buffer.is_empty() {
            self.ctx.session.nav_mode = !self.ctx.session.nav_mode;
            return Some(Action::Consume);
        }

        None
    }

    fn handle_capslock_profile_switch(&mut self, key: VirtualKey) -> Option<Action> {
        let key_char = key_to_char(key, false, false)
            .unwrap_or('\0')
            .to_lowercase()
            .to_string();
        if let Some(profile) = self
            .ctx
            .config
            .profile_keys()
            .iter()
            .find(|(k, _)| k.to_lowercase() == key_char)
            .map(|(_, p)| p.clone())
        {
            self.ctx.session_state.active_profiles =
                profile.split(',').map(|s| s.to_string()).collect();
            self.reset();
            self.ctx.session_state.capslock_pending = false;
            return Some(Action::Notify(
                self.get_short_display(),
                format!("方案: {}", self.get_current_profile_display()),
            ));
        }
        self.ctx.session_state.capslock_pending = false;
        None
    }

    fn handle_ctrl_punctuation(&mut self, key: VirtualKey, shift_pressed: bool) -> Option<Action> {
        let p_key = get_punctuation_key(key, shift_pressed)?;
        let commit_text = if !self.ctx.session.joined_sentence.is_empty() {
            self.ctx.session.joined_sentence.trim_end().to_string()
        } else if !self.ctx.session.candidates.is_empty() {
            self.ctx.session.candidates[0].text.trim_end().to_string()
        } else {
            self.ctx.session.buffer.trim_end().to_string()
        };
        let del_len = self.ctx.session.phantom_text.chars().count();
        self.clear_composing();
        self.ctx.session_state.commit_history.clear();
        Some(Action::DeleteAndEmit {
            delete: del_len,
            insert: format!("{}{}", commit_text, p_key),
        })
    }

    pub fn execute_command(&mut self, cmd: Command) -> Action {
        commands::execute_command(&mut self.ctx, cmd)
    }
}
