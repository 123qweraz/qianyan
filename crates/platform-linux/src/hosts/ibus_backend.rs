//! IBus D-Bus backend for qianyan-ime.
//!
//! Implements a standalone IBus daemon (org.freedesktop.IBus) so that
//! GNOME, Chromium/CEF, and other IBus-aware apps can use qianyan as
//! their input method without requiring a separate ibus-daemon.

use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use log::{error, info, warn};
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::processor::actor::{GuiSnapshot, ProcessorHandle};
use qianyan_ime_engine::processor::Action;
use qianyan_ime_ui::tray::TrayEvent;
use qianyan_ime_ui::GuiEvent;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{self};

// ── Constants ────────────────────────────────────────────────────────────────

use super::wayland_host::keysym_to_vk;

const MOD_SHIFT: u32 = 0x0001;
const MOD_CTRL: u32 = 0x0004;
const MOD_ALT: u32 = 0x0008;
const RELEASE_MASK: u32 = 1 << 30;

fn should_bypass_empty_composition(keyval: u32, mods: u32, has_content: bool) -> bool {
    if !has_content {
        return true;
    }
    if mods & (MOD_CTRL | MOD_ALT) != 0 {
        return true;
    }
    matches!(
        keyval,
        0xff08 | 0xffff | 0xff09 | 0xff0d | 0xff1b | 0xff50..=0xff58 | 0xff8d
    )
}

fn is_enter_key(keyval: u32) -> bool {
    matches!(keyval, 0xff0d | 0xff8d)
}

fn is_shift_key(keyval: u32) -> bool {
    matches!(keyval, 0xffe1 | 0xffe2)
}

/// Build IBusText variant for D-Bus signals.
fn ibus_text_variant(text: &str) -> zvariant::Value<'static> {
    use zvariant::{Array, Dict, Signature, StructureBuilder, Value};

    let sig_s = Signature::try_from("s").unwrap();
    let sig_v = Signature::try_from("v").unwrap();
    let empty_dict = Dict::new(&sig_s, &sig_v);
    let empty_array = Array::new(&sig_v);

    let attr_list = StructureBuilder::new()
        .add_field("IBusAttrList".to_owned())
        .append_field(Value::Dict(empty_dict))
        .append_field(Value::Array(empty_array))
        .build()
        .unwrap();

    let empty_dict2 = Dict::new(&sig_s, &sig_v);
    let ibus_text = StructureBuilder::new()
        .add_field("IBusText".to_owned())
        .append_field(Value::Dict(empty_dict2))
        .add_field(text.to_owned())
        .append_field(Value::Value(Box::new(Value::Structure(attr_list))))
        .build()
        .unwrap();

    Value::Structure(ibus_text)
}

fn ibus_text_value(text: &str) -> zvariant::OwnedValue {
    zvariant::OwnedValue::try_from(ibus_text_variant(text)).expect("ibus_text_value")
}

fn ibus_as_variant(text: &str) -> zvariant::Value<'static> {
    zvariant::Value::Value(Box::new(ibus_text_variant(text)))
}

fn ibus_engine_desc_value() -> zvariant::OwnedValue {
    use zvariant::{Dict, Signature, StructureBuilder, Value};

    let sig_s = Signature::try_from("s").unwrap();
    let sig_v = Signature::try_from("v").unwrap();
    let empty_dict = Dict::new(&sig_s, &sig_v);

    let engine = StructureBuilder::new()
        .add_field("IBusEngineDesc".to_owned())
        .append_field(Value::Dict(empty_dict))
        .add_field("qianyan".to_owned())
        .add_field("Qianyan IME".to_owned())
        .add_field("Qianyan Input Method Engine".to_owned())
        .add_field("zh".to_owned())
        .add_field("MIT".to_owned())
        .add_field("Shian".to_owned())
        .add_field("".to_owned())
        .add_field("default".to_owned())
        .add_field(0u32)
        .add_field("".to_owned())
        .add_field("中".to_owned())
        .add_field("".to_owned())
        .add_field("".to_owned())
        .build()
        .unwrap();

    zvariant::OwnedValue::try_from(Value::Structure(engine)).expect("ibus_engine_desc_value")
}

fn build_lookup_table(
    snapshot: &GuiSnapshot,
) -> Option<zvariant::OwnedValue> {
    if snapshot.candidates.is_empty() || !snapshot.chinese_enabled {
        return None;
    }

    use zvariant::{Array, Dict, Signature, StructureBuilder, Value};

    let sig_s = Signature::try_from("s").unwrap();
    let sig_v = Signature::try_from("v").unwrap();
    let empty_dict = Dict::new(&sig_s, &sig_v);

    let mut cands = Array::new(&sig_v);
    for c in &snapshot.candidates {
        cands.append(ibus_as_variant(&c.text)).expect("append candidate");
    }

    let mut labels = Array::new(&sig_v);
    for c in &snapshot.candidates {
        labels
            .append(ibus_as_variant(&c.label.trim_end_matches('.')))
            .expect("append label");
    }

    let page_size = snapshot.candidates.len().clamp(1, 16) as u32;
    let cursor_pos = snapshot.selected.min(snapshot.candidates.len() - 1) as u32;

    let table = StructureBuilder::new()
        .add_field("IBusLookupTable".to_owned())
        .append_field(Value::Dict(empty_dict))
        .add_field(page_size)
        .add_field(cursor_pos)
        .add_field(true)
        .add_field(false)
        .add_field(2i32) // IBUS_ORIENTATION_SYSTEM
        .append_field(Value::Array(cands))
        .append_field(Value::Array(labels))
        .build()
        .unwrap();

    Some(zvariant::OwnedValue::try_from(Value::Structure(table)).expect("lookup_table"))
}

fn keyval_to_vk(keyval: u32) -> Option<VirtualKey> {
    keysym_to_vk(xkbcommon::xkb::Keysym::from(keyval))
}

// ── InputContext D-Bus object ─────────────────────────────────────────────────

struct InputContext {
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    cursor_x: Arc<AtomicI32>,
    cursor_y: Arc<AtomicI32>,
}

// We share one processor across all input contexts.
// FocusIn/FocusOut events track which context should receive output.

impl InputContext {
    async fn send_clear(&self, ctxt: &SignalEmitter<'_>) {
        let _ = InputContext::hide_preedit_text(ctxt).await;
        let _ = InputContext::hide_lookup_table(ctxt).await;
        let _ = self.gui_tx.send(GuiEvent::SetVisible(false));
        let _ = self.processor.reset();
    }
}

#[interface(name = "org.freedesktop.IBus.InputContext")]
impl InputContext {
    async fn focus_in(&self) {
        info!("[IBus] InputContext FocusIn");
    }

    async fn focus_out(
        &self,
        #[zbus(signal_emitter)] ctxt: SignalEmitter<'_>,
    ) {
        info!("[IBus] InputContext FocusOut");
        self.send_clear(&ctxt).await;
    }

    async fn reset(
        &self,
        #[zbus(signal_emitter)] ctxt: SignalEmitter<'_>,
    ) {
        info!("[IBus] InputContext Reset");
        self.send_clear(&ctxt).await;
    }

    async fn set_cursor_location(&self, x: i32, y: i32, _w: i32, _h: i32) {
        self.cursor_x.store(x, Ordering::Relaxed);
        self.cursor_y.store(y, Ordering::Relaxed);
        let _ = self.gui_tx.send(GuiEvent::MoveTo { x, y });
    }

    async fn set_cursor_location_relative(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}

    async fn enable(&self) {
        info!("[IBus] InputContext Enable");
    }

    async fn disable(
        &self,
        #[zbus(signal_emitter)] ctxt: SignalEmitter<'_>,
    ) {
        info!("[IBus] InputContext Disable");
        self.send_clear(&ctxt).await;
    }

    async fn page_up(&self) {}
    async fn page_down(&self) {}
    async fn cursor_up(&self) {}
    async fn cursor_down(&self) {}

    async fn candidate_clicked(&self, _index: u32, _button: u32, _state: u32) {}

    async fn destroy(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(signal_emitter)] ctxt: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        info!("[IBus] InputContext Destroy");
        self.send_clear(&ctxt).await;
        server
            .remove::<InputContext, _>(ctxt.path().to_owned())
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn process_key_event(
        &self,
        keyval: u32,
        _keycode: u32,
        state: u32,
        #[zbus(signal_emitter)] ctxt: SignalEmitter<'_>,
    ) -> bool {
        // Handle key release (only for shift mode toggle)
        if state & RELEASE_MASK != 0 {
            if is_shift_key(keyval) {
                return false;
            }
            return false;
        }

        // Send keystroke visualization event
        {
            let mut mods = Vec::new();
            if state & MOD_SHIFT != 0 { mods.push("Shift".into()); }
            if state & MOD_CTRL != 0 { mods.push("Ctrl".into()); }
            if state & MOD_ALT != 0 { mods.push("Alt".into()); }
            let key_name = keyval_to_vk(keyval).map(|vk| vk.display_name().to_string()).unwrap_or_default();
            let keys = if key_name.is_empty() { vec![] } else { vec![key_name] };
            let _ = self.gui_tx.send(GuiEvent::KeyEvent { keys, modifiers: mods });
        }

        // Get current gui snapshot to check state
        let gui = match self.processor.get_gui_snapshot() {
            Some(g) => g,
            None => return false,
        };

        let has_content = !gui.pinyin.is_empty() || !gui.candidates.is_empty();

        // Bypass keys that should not go through the IME
        if should_bypass_empty_composition(keyval, state, has_content) {
            if has_content {
                let _ = InputContext::hide_preedit_text(&ctxt).await;
                let _ = InputContext::hide_lookup_table(&ctxt).await;
                let _ = self.gui_tx.send(GuiEvent::SetVisible(false));
            }
            return false;
        }

        // Enter key with active composition -> commit preedit
        if is_enter_key(keyval) && !gui.pinyin.is_empty() {
            let text = gui.sentence.clone();
            if !text.is_empty() {
                let ov = ibus_text_value(&text);
                if let Ok(v) = zvariant::Value::try_from(&ov) {
                    let _ = InputContext::commit_text(&ctxt, v).await;
                }
            }
            let _ = self.processor.reset();
            let _ = InputContext::hide_preedit_text(&ctxt).await;
            let _ = InputContext::hide_lookup_table(&ctxt).await;
            let _ = self.gui_tx.send(GuiEvent::SetVisible(false));
            return true;
        }

        // Convert keyval to VirtualKey
        let vk = match keyval_to_vk(keyval) {
            Some(v) => v,
            None => return false,
        };

        let shift = state & MOD_SHIFT != 0;
        let ctrl = state & MOD_CTRL != 0;
        let alt = state & MOD_ALT != 0;

        // Process the key through qianyan's engine
        let (action, new_gui, status) = match self.processor.handle_key_sync(vk, 1, shift, ctrl, alt)
        {
            Some(tuple) => tuple,
            None => return false,
        };

        // Sync status with tray
        let _ = self.tray_tx.send(TrayEvent::SyncStatus {
            chinese_enabled: status.chinese_enabled,
            active_profile: status.active_profile,
        });

        // Handle action
        let consumed = match &action {
            Action::Emit(text) => {
                info!("[IBus] Emit: {:?}", text);
                let _ = InputContext::hide_preedit_text(&ctxt).await;
                let ov = ibus_text_value(text);
                if let Ok(v) = zvariant::Value::try_from(&ov) {
                    let _ = InputContext::commit_text(&ctxt, v).await;
                }
                true
            }
            Action::DeleteAndEmit { delete: _, insert } => {
                info!("[IBus] DeleteAndEmit: {:?}", insert);
                let ov = ibus_text_value(insert);
                if let Ok(v) = zvariant::Value::try_from(&ov) {
                    let _ = InputContext::commit_text(&ctxt, v).await;
                }
                true
            }
            Action::PassThrough => false,
            Action::Consume => true,
            Action::Alert => false,
            Action::Notify(_, _) => true,
        };

        // Update UI
        self.update_ui(&action, &new_gui, &ctxt);

        // Send gui events to Slint window
        if new_gui.chinese_enabled && (!new_gui.pinyin.is_empty() || !new_gui.candidates.is_empty()) {
            let gui_update = GuiEvent::Update {
                pinyin: new_gui.pinyin.clone(),
                candidates: new_gui.candidates.iter().map(|c| {
                    qianyan_ime_ui::DisplayCandidate {
                        text: c.text.clone(),
                        label: c.label.clone(),
                        hint: c.hint.clone(),
                        full_display: format!("{}{}", c.label, c.text),
                        is_fuzzy: c.is_fuzzy,
                    }
                }).collect(),
                selected: new_gui.selected,
                page: new_gui.page,
                total_pages: new_gui.total_pages,
                sentence: new_gui.sentence.clone(),
                cursor_pos: new_gui.cursor_pos,
                commit_mode: new_gui.commit_mode.clone(),
            };
            let _ = self.gui_tx.send(gui_update);
            let _ = self.gui_tx.send(GuiEvent::SetVisible(true));
        } else {
            let _ = self.gui_tx.send(GuiEvent::SetVisible(false));
        }

        consumed
    }

    #[zbus(signal)]
    async fn commit_text(ctxt: &SignalEmitter<'_>, text: zvariant::Value<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_preedit_text(
        ctxt: &SignalEmitter<'_>,
        text: zvariant::Value<'_>,
        cursor_pos: u32,
        visible: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn hide_preedit_text(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn show_preedit_text(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_lookup_table(
        ctxt: &SignalEmitter<'_>,
        table: zvariant::Value<'_>,
        visible: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn hide_lookup_table(ctxt: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl InputContext {
    fn update_ui(&self, _action: &Action, gui: &GuiSnapshot, ctxt: &SignalEmitter<'_>) {
        // Preedit
        if gui.chinese_enabled && !gui.pinyin.is_empty() {
            let _ = InputContext::hide_preedit_text(ctxt); // clear first
            let ov = ibus_text_value(&gui.pinyin);
            if let Ok(v) = zvariant::Value::try_from(&ov) {
                let _ = InputContext::update_preedit_text(ctxt, v, gui.pinyin.len() as u32, true);
            }
        } else {
            let _ = InputContext::hide_preedit_text(ctxt);
        }

        // Lookup table
        if let Some(table) = build_lookup_table(gui) {
            if let Ok(v) = zvariant::Value::try_from(&table) {
                let _ = InputContext::update_lookup_table(ctxt, v, true);
            }
        } else {
            let _ = InputContext::hide_lookup_table(ctxt);
        }
    }
}

// ── IBusBus D-Bus object ─────────────────────────────────────────────────────

struct IBusBus {
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
    ctx_counter: Arc<AtomicU32>,
}

#[interface(name = "org.freedesktop.IBus")]
impl IBusBus {
    async fn create_input_context(
        &self,
        client_name: &str,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        let n = self.ctx_counter.fetch_add(1, Ordering::SeqCst);
        let path_str = format!("/org/freedesktop/IBus/InputContext_{n}");
        let path = zbus::zvariant::OwnedObjectPath::try_from(path_str.clone())
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

        info!("[IBus] CreateInputContext client={:?} -> {}", client_name, path_str);

        let context = InputContext {
            processor: self.processor.clone(),
            gui_tx: self.gui_tx.clone(),
            tray_tx: self.tray_tx.clone(),
            cursor_x: Arc::new(AtomicI32::new(0)),
            cursor_y: Arc::new(AtomicI32::new(0)),
        };
        server
            .at(path.clone(), context)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

        Ok(path)
    }

    async fn current_input_context(&self) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        Ok(zbus::zvariant::OwnedObjectPath::try_from(
            "/org/freedesktop/IBus/InputContext_1",
        )
        .expect("hardcoded path"))
    }

    async fn is_global_engine(&self) -> bool {
        true
    }

    async fn get_engines(&self) -> Vec<zvariant::OwnedValue> {
        vec![ibus_engine_desc_value()]
    }

    async fn list_active_engines(&self) -> Vec<zvariant::OwnedValue> {
        vec![ibus_engine_desc_value()]
    }

    async fn get_global_engine(&self) -> zbus::fdo::Result<zvariant::OwnedValue> {
        Ok(ibus_engine_desc_value())
    }

    async fn set_global_engine(&self, name: &str) -> zbus::fdo::Result<()> {
        info!("[IBus] SetGlobalEngine: {}", name);
        Ok(())
    }

    async fn register_component(&self, _component: zvariant::Value<'_>) -> zbus::fdo::Result<()> {
        info!("[IBus] RegisterComponent");
        Ok(())
    }

    async fn exit(&self, _restart: bool) {
        info!("[IBus] Exit requested");
    }

    async fn name_owner_changed(&self, _old: &str, _new: &str) {}

    #[zbus(signal)]
    async fn global_engine_changed(ctxt: &SignalEmitter<'_>, name: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn registry_changed(
        ctxt: &SignalEmitter<'_>,
        engine_descs: zvariant::Value<'_>,
    ) -> zbus::Result<()>;
}

// ── Address file helpers ─────────────────────────────────────────────────────

fn write_ibus_address_files() {
    let dbus_address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .unwrap_or_else(|_| format!("unix:path=/run/user/{}/bus", unsafe { libc::getuid() }));

    let machine_id = read_machine_id();
    let pid = std::process::id();

    let bus_dir = match dirs::config_dir() {
        Some(d) => d.join("ibus").join("bus"),
        None => {
            warn!("[IBus] cannot determine config dir; skipping address files");
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&bus_dir) {
        warn!("[IBus] failed to create {}: {}", bus_dir.display(), e);
        return;
    }

    let content = format!(
        "# This file is created by qianyan-ime (IBus compatible)\n\
         IBUS_ADDRESS={dbus_address}\n\
         IBUS_DAEMON_PID={pid}\n"
    );

    let display_num = display_number();
    let wayland_num = wayland_display_number();

    let mut names = vec![
        format!("{machine_id}-unix-{display_num}"),
        format!("{machine_id}-unix-wayland-0"),
        format!("{machine_id}-unix-wayland-1"),
    ];
    if let Some(wn) = wayland_num {
        names.push(format!("{machine_id}-unix-wayland-{wn}"));
    }
    names.sort();
    names.dedup();

    for name in names {
        let path = bus_dir.join(&name);
        if let Err(e) = std::fs::write(&path, &content) {
            warn!("[IBus] failed to write {}: {}", path.display(), e);
        } else {
            info!("[IBus] wrote address file: {}", path.display());
        }
    }
}

fn read_machine_id() -> String {
    for path in &["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(s) = std::fs::read_to_string(path) {
            let id = s.trim().to_owned();
            if !id.is_empty() {
                return id;
            }
        }
    }
    "unknown".to_owned()
}

fn display_number() -> u32 {
    std::env::var("DISPLAY")
        .ok()
        .and_then(|d| {
            d.rsplit(':')
                .next()
                .and_then(|s| s.split('.').next())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0)
}

fn wayland_display_number() -> Option<u32> {
    std::env::var("WAYLAND_DISPLAY")
        .ok()
        .and_then(|d| d.rsplit('-').next().and_then(|s| s.parse().ok()))
}

// ── Public entry point ────────────────────────────────────────────────────────

pub fn start_ibus_backend(
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                error!("[IBus] Failed to create tokio runtime: {e}");
                return;
            }
        };
        rt.block_on(run_ibus(processor, gui_tx, tray_tx));
    });
}

async fn run_ibus(
    processor: ProcessorHandle,
    gui_tx: Sender<GuiEvent>,
    tray_tx: Sender<TrayEvent>,
) {
    info!("[IBus] Starting IBus D-Bus backend");

    let bus = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            error!("[IBus] Failed to connect to session bus: {e}");
            return;
        }
    };

    // Serve the IBus bus object
    if let Err(e) = bus
        .object_server()
        .at(
            "/org/freedesktop/IBus",
            IBusBus {
                processor: processor.clone(),
                gui_tx: gui_tx.clone(),
                tray_tx: tray_tx.clone(),
                ctx_counter: Arc::new(AtomicU32::new(1)),
            },
        )
        .await
    {
        error!("[IBus] Failed to serve at /org/freedesktop/IBus: {e}");
        return;
    }

    // Request the org.freedesktop.IBus bus name
    if let Err(e) = bus.request_name("org.freedesktop.IBus").await {
        warn!("[IBus] Failed to request org.freedesktop.IBus name (another ibus-daemon running?): {e}");
        return;
    }
    info!("[IBus] Acquired org.freedesktop.IBus");

    write_ibus_address_files();

    // Notify that our engine is available
    if let Ok(signal_ctx) = SignalEmitter::new(&bus, "/org/freedesktop/IBus") {
        let _ = IBusBus::global_engine_changed(&signal_ctx, "qianyan").await;
    }

    info!("[IBus] IBus D-Bus backend ready");

    // Keep alive
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
