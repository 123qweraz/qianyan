use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;


pub struct SoundManager {
    _handle: Option<OutputStreamHandle>,
    _sound_cache: HashMap<char, Vec<u8>>,
    enabled: bool,
    tx: Option<Sender<char>>,
}

impl Default for SoundManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundManager {
    pub fn new() -> Self {
        let (stream, handle) = match OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h)),
            Err(e) => {
                log::warn!("[Sound] 无法初始化音频输出: {}", e);
                (None, None)
            }
        };

        let (tx, rx) = mpsc::channel::<char>();

        let handle_clone = handle.clone();
        thread::spawn(move || {
            if let Some(h) = handle_clone {
                while let Ok(c) = rx.recv() {
                    play_letter_on_thread(&h, c);
                }
            }
        });

        drop(stream);

        Self {
            _handle: handle,
            _sound_cache: HashMap::new(),
            enabled: false,
            tx: Some(tx),
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
                    sink.play();
                }
            }
        }
        Err(e) => log::warn!("[Sound] 无法打开音频文件 {:?}: {}", sound_path, e),
    }
}

fn find_project_root() -> PathBuf {
    let mut curr = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    for _ in 0..5 {
        if curr.join("sounds").exists() {
            return curr;
        }
        if !curr.pop() {
            break;
        }
    }
    PathBuf::from(".")
}
