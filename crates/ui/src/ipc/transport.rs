use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const HEADER_SIZE: usize = 4;

/// Read one length-delimited frame from a UnixStream.
/// Returns None if the connection is closed.
pub fn read_frame(stream: &mut UnixStream) -> Result<Option<Vec<u8>>, String> {
    let mut header = [0u8; HEADER_SIZE];
    let mut read = 0;
    while read < HEADER_SIZE {
        match stream.read(&mut header[read..]) {
            Ok(0) => return Ok(None),
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("read header: {e}")),
        }
    }
    let len = u32::from_le_bytes(header) as usize;
    let mut buf = vec![0u8; len];
    let mut read = 0;
    while read < len {
        match stream.read(&mut buf[read..]) {
            Ok(0) => return Err("unexpected EOF in frame body".into()),
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("read body: {e}")),
        }
    }
    Ok(Some(buf))
}

/// Write one length-delimited frame to a UnixStream.
pub fn write_frame(stream: &mut UnixStream, data: &[u8]) -> Result<(), String> {
    let len = data.len();
    if len > u32::MAX as usize {
        return Err("frame too large".into());
    }
    let header = (len as u32).to_le_bytes();
    stream.write_all(&header).map_err(|e| format!("write header: {e}"))?;
    stream.write_all(data).map_err(|e| format!("write body: {e}"))?;
    stream.flush().map_err(|e| format!("flush: {e}"))
}

/// Read a frame with a per-operation timeout using `set_read_timeout`.
/// Returns None on clean EOF.
pub fn read_frame_timeout(
    stream: &mut UnixStream,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, String> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("set timeout: {e}"))?;
    let result = read_frame(stream);
    stream.set_read_timeout(None).ok();
    result
}

/// A serializable message from the main process to the GUI process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MainToGui {
    SyncState(AppStateMsg),
    Update {
        pinyin: String,
        candidates: Vec<DisplayCandidateMsg>,
        selected: usize,
        page: usize,
        total_pages: usize,
    },
    MoveTo { x: i32, y: i32 },
    SetVisible(bool),
    ShowStatus(String, bool),
    ApplyConfig(String),
    /// GUI must respond with GuiToMain::Ack after hiding the candidate window.
    HideCandidate,
    KeyEvent {
        keys: Vec<String>,
        modifiers: Vec<String>,
    },
    Exit,
}

/// A serializable message from the GUI process to the main process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GuiToMain {
    Ack,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateMsg {
    pub chinese_enabled: bool,
    pub active_profile: String,
    pub show_candidates_pref: bool,
    pub is_ime_active: bool,
    pub pinyin: String,
    pub candidates: Vec<DisplayCandidateMsg>,
    pub selected_index: usize,
    pub page: usize,
    pub total_pages: usize,
    pub status_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayCandidateMsg {
    pub text: String,
    pub label: String,
    pub hint: String,
    pub is_fuzzy: bool,
}

/// Send a MainToGui message over the stream.
pub fn send_main_to_gui(stream: &mut UnixStream, msg: &MainToGui) -> Result<(), String> {
    let data = serde_json::to_vec(msg).map_err(|e| format!("serialize: {e}"))?;
    write_frame(stream, &data)
}

/// Read a MainToGui message from the stream.
pub fn recv_main_to_gui(stream: &mut UnixStream) -> Result<Option<MainToGui>, String> {
    match read_frame(stream)? {
        None => Ok(None),
        Some(data) => {
            let msg: MainToGui =
                serde_json::from_slice(&data).map_err(|e| format!("deserialize: {e}"))?;
            Ok(Some(msg))
        }
    }
}

/// Send a GuiToMain message over the stream.
pub fn send_gui_to_main(stream: &mut UnixStream, msg: &GuiToMain) -> Result<(), String> {
    let data = serde_json::to_vec(msg).map_err(|e| format!("serialize: {e}"))?;
    write_frame(stream, &data)
}

/// Read a GuiToMain message with optional timeout.
pub fn recv_gui_to_main(
    stream: &mut UnixStream,
    timeout: Option<Duration>,
) -> Result<Option<GuiToMain>, String> {
    let data = match timeout {
        Some(t) => read_frame_timeout(stream, t)?,
        None => read_frame(stream)?,
    };
    match data {
        None => Ok(None),
        Some(buf) => {
            let msg: GuiToMain =
                serde_json::from_slice(&buf).map_err(|e| format!("deserialize: {e}"))?;
            Ok(Some(msg))
        }
    }
}
