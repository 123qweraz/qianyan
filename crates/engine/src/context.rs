use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::keys::VirtualKey;
use crate::scheme::InputScheme;
use qianyan_ime_core::Config;

pub struct EngineContext {
    pub session: crate::InputSession,
    pub session_state: crate::processor::session_state::SessionState,
    pub config: crate::ConfigManager,
    pub engine: crate::pipeline::SearchEngine,
    pub syllables: HashSet<String>,
    pub dispatcher: crate::KeyDispatcher,

    pub sound_manager: crate::sound::SoundManager,
}

impl EngineContext {
    pub fn new(
        trie_paths: HashMap<String, (std::path::PathBuf, std::path::PathBuf)>,
        syllables: HashSet<String>,
        syllable_freq: HashMap<String, u64>,
    ) -> Self {
        let config = crate::ConfigManager::new();
        let syllables_arc = Arc::new(syllables.clone());
        let syllable_freq_arc = Arc::new(syllable_freq);

        let engine = crate::pipeline::SearchEngine::new(
            trie_paths,
            syllables_arc,
            syllable_freq_arc,
            config.learned_words.clone(),
            config.usage_history.clone(),
            config.ngram_history.clone(),
            Self::default_schemes(),
        );

        Self {
            session: crate::InputSession::new(),
            session_state: crate::processor::session_state::SessionState::new(),
            config,
            engine,
            syllables,
            dispatcher: crate::KeyDispatcher::new(),
            sound_manager: crate::sound::SoundManager::new(),
        }
    }

    pub fn new_with_engine(
        engine: crate::pipeline::SearchEngine,
        syllables: HashSet<String>,
    ) -> Self {
        let config = crate::ConfigManager::new();
        Self {
            session: crate::InputSession::new(),
            session_state: crate::processor::session_state::SessionState::new(),
            config,
            engine,
            syllables,
            dispatcher: crate::KeyDispatcher::new(),
            sound_manager: crate::sound::SoundManager::new(),
        }
    }

    fn default_schemes() -> Arc<HashMap<String, Box<dyn InputScheme>>> {
        let mut m: HashMap<String, Box<dyn InputScheme>> = HashMap::new();
        m.insert("stroke".to_string(), Box::new(crate::schemes::StrokeScheme::new()));
        m.insert("english".to_string(), Box::new(crate::schemes::EnglishScheme::new()));
        m.insert("japanese".to_string(), Box::new(crate::schemes::JapaneseScheme::new()));
        m.insert("chinese".to_string(), Box::new(crate::schemes::ChineseScheme::new()));
        Arc::new(m)
    }

    pub fn reset(&mut self) {
        self.session.reset();
        self.session_state.commit_history.clear();
    }

    pub fn apply_config(&mut self, conf: &Config) {
        self.config.apply_config(conf);
        self.engine.clear_cache();
        self.sound_manager.set_enabled(conf.input.enable_keyboard_voice);

        // Prioritize default_profile if it's in trie_paths
        let default_profile = conf.input.default_profile.to_lowercase();
        if !default_profile.is_empty() && self.engine.trie_paths.contains_key(&default_profile) {
            self.session_state.active_profiles = vec![default_profile];
        } else if !conf.input.enabled_profiles.is_empty() {
            let enabled: Vec<String> = conf
                .input
                .enabled_profiles
                .iter()
                .map(|p| p.to_lowercase())
                .filter(|p| self.engine.trie_paths.contains_key(p))
                .collect();
            if !enabled.is_empty() {
                self.session_state.active_profiles = vec![enabled[0].clone()];
            }
        } else {
            self.session_state.active_profiles = vec!["chinese".to_string()];
        }

        let enabled_list: Vec<String> = conf
            .input
            .enabled_profiles
            .iter()
            .filter(|p| self.engine.trie_paths.contains_key(&p.to_lowercase()))
            .map(|p| p.to_lowercase())
            .collect();
        for profile in enabled_list {
            let engine = self.engine.clone();
            std::thread::spawn(move || {
                engine.prewarm_profile(&profile);
            });
        }

        self.setup_default_keymap();
    }

    fn setup_default_keymap(&mut self) {
        use crate::Command;
        use crate::ModifierState;

        self.dispatcher.key_map.clear();
        let none = ModifierState {
            shift: false,
            ctrl: false,
            alt: false,
            meta: false,
        };

        self.dispatcher
            .key_map
            .insert((VirtualKey::Left, none), Command::PrevCandidate);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Right, none), Command::NextCandidate);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Up, none), Command::PrevPage);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Down, none), Command::NextPage);
        self.dispatcher
            .key_map
            .insert((VirtualKey::PageUp, none), Command::PrevPage);
        self.dispatcher
            .key_map
            .insert((VirtualKey::PageDown, none), Command::NextPage);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Space, none), Command::Commit);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Enter, none), Command::CommitRaw);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Esc, none), Command::Clear);
        self.dispatcher
            .key_map
            .insert((VirtualKey::Delete, none), Command::Clear);
    }
}

impl Drop for EngineContext {
    fn drop(&mut self) {
        self.config.flush_all();
    }
}
