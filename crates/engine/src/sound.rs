use qianyan_ime_core::utils::find_project_root;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};


pub struct SoundManager {
    _sound_cache: HashMap<char, Vec<u8>>,
    enabled: bool,
    tx: Option<Sender<char>>,
    _thread: Option<JoinHandle<()>>,
}

impl Default for SoundManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SoundManager {
    fn drop(&mut self) {
        drop(self.tx.take());
    }
}

impl SoundManager {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<char>();

        let handle = thread::spawn(move || {
            let (_stream, handle) = match OutputStream::try_default() {
                Ok((s, h)) => (Some(s), Some(h)),
                Err(e) => {
                    log::warn!("[Sound] 无法初始化音频输出: {}", e);
                    (None, None)
                }
            };

            if let Some(h) = handle {
                while let Ok(c) = rx.recv() {
                    play_letter_on_thread(&h, c);
                }
            }
        });

        Self {
            _sound_cache: HashMap::new(),
            enabled: false,
            tx: Some(tx),
            _thread: Some(handle),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn play_letter(&self, c: char) {
        if !self.enabled {
            return;
        }

        let c = c.to_ascii_lowercase();
        if !c.is_ascii_alphabetic() {
            return;
        }

        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }
}

fn play_letter_on_thread(handle: &OutputStreamHandle, c: char) {
    let root = find_project_root();
    let sound_path = root
        .join("sounds")
        .join("letters")
        .join(format!("{}.mp3", c));

    if !sound_path.exists() {
        return;
    }

    match File::open(&sound_path) {
        Ok(file) => {
            if let Ok(sink) = Sink::try_new(handle) {
                if let Ok(source) = Decoder::new(BufReader::new(file)) {
                    sink.append(source);
                    sink.sleep_until_end();
                }
            }
        }
        Err(e) => log::warn!("[Sound] 无法打开音频文件 {:?}: {}", sound_path, e),
    }
}
