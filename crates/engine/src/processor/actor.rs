use std::sync::mpsc::{self, Sender};
use std::sync::Arc;

use crate::keys::VirtualKey;
use crate::pipeline::{Candidate, SearchQuery, MAX_LOOKUP_LIMIT};
use crate::processor::{Action, FilterMode, Processor};
use qianyan_ime_core::config::Config;

/// Snapshot of processor state for background search
#[derive(Clone)]
pub struct SearchSnapshot {
    pub buffer: String,
    pub profile: String,
    pub config: Config,
    pub aux_filter: String,
    pub filter_mode: FilterMode,
    pub fuzzy_activated: bool,
}

/// Result from background search
#[derive(Clone)]
pub struct SearchResult {
    pub candidates: Vec<Candidate>,
    pub best_segmentation: Vec<String>,
    pub has_dict_match: bool,
}

impl SearchResult {
    pub fn new(candidates: Vec<Candidate>, best_segmentation: Vec<String>, has_dict_match: bool) -> Self {
        Self { candidates, best_segmentation, has_dict_match }
    }
}

/// A single candidate's display info
#[derive(Clone)]
pub struct CandidateInfo {
    pub text: String,
    pub label: String,
    pub hint: String,
    pub is_fuzzy: bool,
}

/// Read-only snapshot for constructing GUI updates
#[derive(Clone)]
pub struct GuiSnapshot {
    pub pinyin: String,
    pub candidates: Vec<CandidateInfo>,
    pub selected: usize,
    pub page: usize,
    pub total_pages: usize,
    pub sentence: String,
    pub cursor_pos: usize,
    pub commit_mode: String,
    pub chinese_enabled: bool,
    pub short_display: String,
    pub active_profile: String,
}

/// Combined snapshot for tray handler responses
#[derive(Clone)]
pub struct TraySnapshot {
    pub chinese_enabled: bool,
    pub ime_enabled: bool,
    pub short_display: String,
    pub commit_mode: String,
    pub active_profile: String,
    pub enabled_profiles: Vec<String>,
}

/// Basic status info for the host loops
#[derive(Clone)]
pub struct BasicStatus {
    pub chinese_enabled: bool,
    pub ime_enabled: bool,
    pub short_display: String,
    pub active_profile: String,
}

pub enum ProcessorMsg {
    HandleKey {
        key: VirtualKey,
        val: i32,
        shift: bool,
        ctrl: bool,
        alt: bool,
        perform_lookup: bool,
        reply: Sender<Action>,
    },
    HandleKeySync {
        key: VirtualKey,
        val: i32,
        shift: bool,
        ctrl: bool,
        alt: bool,
        reply: Sender<(Action, GuiSnapshot, BasicStatus)>,
    },
    Toggle {
        reply: Sender<TraySnapshot>,
    },
    ToggleEnabled {
        reply: Sender<TraySnapshot>,
    },
    NextProfile {
        reply: Sender<TraySnapshot>,
    },
    SetProfile {
        profile: String,
        reply: Sender<Option<TraySnapshot>>,
    },
    ApplyConfig {
        config: Config,
        reply: Sender<()>,
    },
    ReloadTries {
        reply: Sender<()>,
    },
    Reset {
        reply: Sender<()>,
    },
    ClearUserData {
        profile: String,
    },
    ListProfiles {
        reply: Sender<Vec<String>>,
    },
    /// Triggers a search using the current processor state, writes results,
    /// checks auto-commit/phantom, and returns an action to execute if any.
    PerformSearch {
        reply: Sender<Option<Action>>,
    },
    GetGuiSnapshot {
        reply: Sender<GuiSnapshot>,
    },
    GetBasicStatus {
        reply: Sender<BasicStatus>,
    },
    GetConfig {
        reply: Sender<Config>,
    },
    Exit,
}

#[derive(Clone)]
pub struct ProcessorHandle {
    tx: Sender<ProcessorMsg>,
}

impl ProcessorHandle {
    pub fn new(tx: Sender<ProcessorMsg>) -> Self {
        Self { tx }
    }

    fn call<T: Send>(&self, msg: ProcessorMsg, rx: mpsc::Receiver<T>) -> Option<T> {
        self.tx.send(msg).ok()?;
        rx.recv().ok()
    }

    pub fn handle_key(
        &self,
        key: VirtualKey,
        val: i32,
        shift: bool,
        ctrl: bool,
        alt: bool,
        perform_lookup: bool,
    ) -> Option<Action> {
        let (tx, rx) = mpsc::channel();
        self.call(
            ProcessorMsg::HandleKey { key, val, shift, ctrl, alt, perform_lookup, reply: tx },
            rx,
        )
    }

    pub fn handle_key_sync(
        &self,
        key: VirtualKey,
        val: i32,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> Option<(Action, GuiSnapshot, BasicStatus)> {
        let (tx, rx) = mpsc::channel();
        self.call(
            ProcessorMsg::HandleKeySync { key, val, shift, ctrl, alt, reply: tx },
            rx,
        )
    }

    pub fn toggle(&self) -> Option<TraySnapshot> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::Toggle { reply: tx }, rx)
    }

    pub fn toggle_enabled(&self) -> Option<TraySnapshot> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::ToggleEnabled { reply: tx }, rx)
    }

    pub fn next_profile(&self) -> Option<TraySnapshot> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::NextProfile { reply: tx }, rx)
    }

    pub fn set_profile(&self, profile: String) -> Option<Option<TraySnapshot>> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::SetProfile { profile, reply: tx }, rx)
    }

    pub fn apply_config(&self, config: Config) -> Option<()> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::ApplyConfig { config, reply: tx }, rx)
    }

    pub fn reload_tries(&self) -> Option<()> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::ReloadTries { reply: tx }, rx)
    }

    pub fn reset(&self) -> Option<()> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::Reset { reply: tx }, rx)
    }

    pub fn clear_user_data(&self, profile: String) {
        let _ = self.tx.send(ProcessorMsg::ClearUserData { profile });
    }

    pub fn list_profiles(&self) -> Option<Vec<String>> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::ListProfiles { reply: tx }, rx)
    }

    pub fn perform_search(&self) -> Option<Option<Action>> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::PerformSearch { reply: tx }, rx)
    }

    pub fn get_gui_snapshot(&self) -> Option<GuiSnapshot> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::GetGuiSnapshot { reply: tx }, rx)
    }

    pub fn get_basic_status(&self) -> Option<BasicStatus> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::GetBasicStatus { reply: tx }, rx)
    }

    pub fn get_config(&self) -> Option<Config> {
        let (tx, rx) = mpsc::channel();
        self.call(ProcessorMsg::GetConfig { reply: tx }, rx)
    }

    pub fn exit(&self) {
        let _ = self.tx.send(ProcessorMsg::Exit);
    }
}

pub struct ProcessorActor {
    processor: Processor,
    rx: mpsc::Receiver<ProcessorMsg>,
}

impl ProcessorActor {
    pub fn new(processor: Processor, rx: mpsc::Receiver<ProcessorMsg>) -> Self {
        Self { processor, rx }
    }

    fn build_candidate_infos(&self, start: usize, end: usize) -> Vec<CandidateInfo> {
        let ctx = &self.processor.ctx;
        ctx.session.candidates[start..end].iter().enumerate().map(|(i, c)| {
            let is_fuzzy = c.match_level < 3 && c.source.as_ref() == "Table (Fuzzy)";
            let label = format!("{}.", i + 1);
            CandidateInfo {
                text: c.text.to_string(),
                label,
                hint: c.hint.to_string(),
                is_fuzzy,
            }
        }).collect()
    }

    fn build_gui_snapshot(&self) -> GuiSnapshot {
        let ctx = &self.processor.ctx;
        let pinyin = crate::compositor::Compositor::get_preedit(ctx);
        let short_display = self.processor.get_short_display();
        let active_profile = self.processor.get_current_profile_display();
        let page_size = ctx.config.page_size();
        let start = ctx.session.page.min(ctx.session.candidates.len());
        let end = (start + page_size).min(ctx.session.candidates.len());

        let candidates = self.build_candidate_infos(start, end);
        let relative_selected = ctx.session.selected.saturating_sub(start);
        let current_page = start.checked_div(page_size).unwrap_or(0);
        let total_pages = ctx.session.candidates.len().div_ceil(page_size);

        GuiSnapshot {
            pinyin,
            candidates,
            selected: relative_selected,
            page: current_page,
            total_pages,
            sentence: ctx.session.joined_sentence.clone(),
            cursor_pos: ctx.session.cursor_pos,
            commit_mode: ctx.config.commit_mode().to_string(),
            chinese_enabled: ctx.session_state.chinese_enabled,
            short_display,
            active_profile,
        }
    }

    fn build_basic_status(&self) -> BasicStatus {
        BasicStatus {
            chinese_enabled: self.processor.ctx.session_state.chinese_enabled,
            ime_enabled: self.processor.ctx.session_state.ime_enabled,
            short_display: self.processor.get_short_display(),
            active_profile: self.processor.get_current_profile_display(),
        }
    }

    fn build_tray_snapshot(&self) -> TraySnapshot {
        TraySnapshot {
            chinese_enabled: self.processor.ctx.session_state.chinese_enabled,
            ime_enabled: self.processor.ctx.session_state.ime_enabled,
            short_display: self.processor.get_short_display(),
            commit_mode: self.processor.ctx.config.commit_mode().to_string(),
            active_profile: self.processor.get_current_profile_display(),
            enabled_profiles: self.processor.ctx.config.master_config.input.enabled_profiles.clone(),
        }
    }

    fn run_search(&mut self) -> Option<Action> {
        let ctx = &mut self.processor.ctx;
        if ctx.session.buffer.is_empty() {
            return None;
        }

        let current_profile = ctx.session_state.active_profiles.first().cloned().unwrap_or_default();
        let last_word = ctx.session_state.commit_history.last().map(|(_, w)| w.as_str());

        let query = SearchQuery {
            buffer: &ctx.session.buffer,
            profile: &current_profile,
            config: &ctx.config.master_config,
            limit: MAX_LOOKUP_LIMIT,
            filter_mode: ctx.session.filter_mode.clone(),
            aux_filter: &ctx.session.aux_filter,
            context: last_word,
            fuzzy_enabled: ctx.session.fuzzy_activated,
        };

        let (candidates, segments) = ctx.engine.search(query);
        ctx.session.candidates = candidates;
        ctx.session.best_segmentation = segments;
        ctx.session.has_dict_match = !ctx.session.candidates.is_empty();
        ctx.session.last_lookup_pinyin = ctx.session.buffer.clone();

        if ctx.session.candidates.is_empty() {
            let buf_arc: Arc<str> = Arc::from(ctx.session.buffer.as_str());
            ctx.session.candidates.push(Candidate {
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
        ctx.session.update_state();

        if let Some(commit_action) = crate::compositor::Compositor::check_auto_commit(ctx) {
            return Some(commit_action);
        }

        let phantom_action = self.processor.update_phantom_action();
        if phantom_action != Action::Consume {
            return Some(phantom_action);
        }

        None
    }

    pub fn run(mut self) {
        loop {
            match self.rx.recv() {
                Ok(msg) => self.handle(msg),
                Err(_) => break,
            }
        }
        log::info!("ProcessorActor: exiting");
    }

    fn handle(&mut self, msg: ProcessorMsg) {
        match msg {
            ProcessorMsg::HandleKey { key, val, shift, ctrl, alt, perform_lookup, reply } => {
                let action = self.processor.handle_key_ext(key, val, shift, ctrl, alt, perform_lookup);
                let _ = reply.send(action);
            }
            ProcessorMsg::HandleKeySync { key, val, shift, ctrl, alt, reply } => {
                let action = self.processor.handle_key_ext(key, val, shift, ctrl, alt, true);
                let snapshot = self.build_gui_snapshot();
                let status = self.build_basic_status();
                let _ = reply.send((action, snapshot, status));
            }
            ProcessorMsg::Toggle { reply } => {
                self.processor.toggle();
                let snap = self.build_tray_snapshot();
                let _ = reply.send(snap);
            }
            ProcessorMsg::ToggleEnabled { reply } => {
                self.processor.toggle_enabled();
                let snap = self.build_tray_snapshot();
                let _ = reply.send(snap);
            }
            ProcessorMsg::NextProfile { reply } => {
                self.processor.next_profile();
                let snap = self.build_tray_snapshot();
                let _ = reply.send(snap);
            }
            ProcessorMsg::SetProfile { profile, reply } => {
                let profiles: Vec<String> = profile.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| self.processor.ctx.engine.trie_paths.contains_key(s))
                    .collect();
                if profiles.is_empty() {
                    let _ = reply.send(None);
                    return;
                }
                self.processor.ctx.session_state.active_profiles = profiles;
                if let Ok(conf) = self.processor.ctx.config.master_config_write() {
                    conf.input.default_profile = profile.clone();
                    let _ = conf.save();
                }
                self.processor.reset();
                let snap = self.build_tray_snapshot();
                let _ = reply.send(Some(snap));
            }
            ProcessorMsg::ApplyConfig { config, reply } => {
                self.processor.apply_config(&config);
                let _ = reply.send(());
            }
            ProcessorMsg::ReloadTries { reply } => {
                let engine = self.processor.ctx.engine.clone();
                std::thread::spawn(move || {
                    engine.reload_tries();
                });
                let _ = reply.send(());
            }
            ProcessorMsg::Reset { reply } => {
                self.processor.reset();
                let _ = reply.send(());
            }
            ProcessorMsg::ClearUserData { profile } => {
                let _ = self.processor.ctx.config.clear_user_data(&profile);
            }
            ProcessorMsg::ListProfiles { reply } => {
                let profiles = self.processor.ctx.config.list_profiles();
                let _ = reply.send(profiles);
            }
            ProcessorMsg::PerformSearch { reply } => {
                let action = self.run_search();
                let _ = reply.send(action);
            }
            ProcessorMsg::GetGuiSnapshot { reply } => {
                let snap = self.build_gui_snapshot();
                let _ = reply.send(snap);
            }
            ProcessorMsg::GetBasicStatus { reply } => {
                let status = self.build_basic_status();
                let _ = reply.send(status);
            }
            ProcessorMsg::GetConfig { reply } => {
                let config = self.processor.ctx.config.master_config.clone();
                let _ = reply.send(config);
            }
            ProcessorMsg::Exit => {}
        }
    }
}
