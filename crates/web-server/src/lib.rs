pub mod handlers;
pub mod platform;
pub mod routes;
pub mod static_handler;
pub mod subproc;

use std::sync::{Arc, RwLock, Mutex as StdMutex};
use std::sync::atomic::{AtomicU16, AtomicU64, AtomicBool};
use std::collections::HashMap;
use std::path::PathBuf;

use qianyan_ime_engine::pipeline::SearchEngine;
use qianyan_ime_engine::trie::Trie;
use qianyan_ime_core::Config;
use qianyan_ime_core::event::TrayEvent;

pub struct WebServer {
    pub port: u16,
    pub actual_port: Arc<AtomicU16>,
    pub config: Arc<RwLock<Config>>,
    pub tries: Arc<RwLock<HashMap<String, Trie>>>,
    pub tray_tx: std::sync::mpsc::Sender<TrayEvent>,
    pub root: PathBuf,
}

pub type WebState = (
    Arc<RwLock<Config>>,
    Arc<RwLock<HashMap<String, Trie>>>,
    std::sync::mpsc::Sender<TrayEvent>,
);

pub struct ImeEngineHandle {
    pub engine: Arc<RwLock<Option<Arc<SearchEngine>>>>,
    pub root: PathBuf,
    pub(crate) sessions: StdMutex<HashMap<String, crate::ImeSession>>,
    pub shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub shutdown_pending: Arc<AtomicBool>,
    pub last_activity: Arc<AtomicU64>,
}

pub(crate) struct ImeSession {
    processor: qianyan_ime_engine::Processor,
    #[allow(dead_code)]
    created: std::time::Instant,
}

pub const MAX_IME_SESSIONS: usize = 1000;
pub const SESSION_TTL_SECS: u64 = 3600;

impl WebServer {
    pub fn new(
        port: u16,
        actual_port: Arc<AtomicU16>,
        config: Arc<RwLock<Config>>,
        tries: Arc<RwLock<HashMap<String, Trie>>>,
        tray_tx: std::sync::mpsc::Sender<TrayEvent>,
        root: PathBuf,
    ) -> Self {
        Self { port, actual_port, config, tries, tray_tx, root }
    }
}

pub use subproc::{launch_feature_center, stop_feature_center};
