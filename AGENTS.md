# Qianyan IME — Anchored Summary

## Goal
Implement IBus D-Bus standalone daemon for GNOME/multi-desktop support in qianyan IME, referencing keytao project.

Current active task: Generate level4.json (rare CJK characters with stroke_aux + pinyin) from bihua.txt and Unihan database, then recompile FST dictionary.

## Constraints & Preferences
- Keep qianyan's own engine (don't switch to librime)
- Keep Slint GUI for candidate window, no X11 overlay panel
- Slint GUI should work in KWin mode (currently broken because GUI subprocess removes WAYLAND_SOCKET and WAYLAND_DISPLAY may not be set)
- In KWin virtual keyboard mode, skip Slint GUI entirely
- IBus backend runs as independent D-Bus daemon, not as GNOME ibus-engine plugin
- Communication in Chinese during task discussions

## Done
- Fixed `WAYLAND_DISPLAY` check in `wayland_host_v1.rs:433`, `wayland_host.rs:583` to also accept `WAYLAND_SOCKET` (KWin private socket)
- Fixed `vkbd.rs:139` same check for clipboard backend detection
- Added modifier forwarding (`ctx.modifiers()`) in `wayland_host_v1.rs`
- Added 180ms deactivate debounce in both `wayland_host_v1.rs` and `wayland_host.rs`
- Removed probe `Connection::connect_to_env()` from `WaylandInputHost::new()` and `WaylandInputHostV1::new()` (was creating extra Wayland connection)
- Skip Vkbd (uinput device) when `WAYLAND_SOCKET` is set (KWin mode)
- Set `GTK_IM_MODULE=wayland` and `QT_IM_MODULE=wayland` on KDE in `main.rs`
- Added zbus 5 + tokio + dirs dependency to `crates/platform-linux/Cargo.toml`
- Created `crates/platform-linux/src/hosts/ibus_backend.rs` implementing IBus daemon
- Registered `ibus_backend` module in `hosts/mod.rs`
- Added `start_ibus_backend()` to `runtime.rs`
- Updated `main.rs` to start IBus backend in background thread, skip Slint GUI in KWin mode
- **Fixed all zbus 5 API issues**
- Full release build succeeds
- **Decoupled GUI from main process**: GUI is now optional. Added `--no-gui` CLI flag; `show_slint_window=false` skips GUI subprocess entirely; GUI binary missing/crash no longer panics; `HideAndAck` handled gracefully in null handler; main thread no longer blocks on `child.wait()`
- **Moved system notifications from GUI subprocess to main process**: Created `crates/ui/src/local_notify.rs` (`LocalNotify`); inserted notification dispatcher layer between `gui_rx` and GUI forwarder/null handler; removed `LinuxNotifyDisplay` from GUI subprocess
- **方案A completed**: Fixed `recursive_mixed_match` to guard `is_initial=true` matches against `single_syllables` set. Added `load_single_syllables()` in `utils.rs`. Added `single_syllables` field to `SearchEngine` + `SchemeContext`.
- **方案B completed**: Added `enable_error_correction` config toggle (default true) in `InputConfig`, guard Strategy 4 in `chinese.rs`, toggle UI in `static/settings/pinyin.html`.
- **Generated `level4.json`**: 13,073 rare/CJK extension characters with pinyin + stroke_aux + tone, from `bihua.txt` × Unihan_Readings.txt (kMandarin).
- **Updated `load_single_syllables`**: now reads level1-4.json → 420+ valid single syllables.
- **Recompiled FST dictionary**: `cargo run --bin qianyan-ime -- --compile-only`.
- **Cached single syllables**: created `dicts/chinese/single_syllables.txt` (419 lines), `load_single_syllables` reads it first; falls back to JSON parsing if missing.
- **Added `enable_rare_chars` config toggle** (default false): hides level-4 characters from search results. Filter applied in `SearchEngine::do_search()` using rare chars set loaded from `level4.json`. Toggle UI in `static/settings/pinyin.html`.

## In Progress
- (none)

## Blocked
- (none)

## Key Decisions
- IBus standalone daemon (not ibus-engine plugin)

## Key Decisions
- IBus standalone daemon (not ibus-engine plugin)
- Slint GUI kept but fully decoupled — engine runs without it in any desktop mode
- KWin virtual keyboard mode skips Slint GUI
- `show_slint_window=false` now skips GUI subprocess entirely (previously only hid the window)
- System notifications (`LocalNotify`) run in main process regardless of GUI availability
- pinyin data for level4.json sourced from Unihan kMandarin (44K chars) instead of pypinyin

## Next Steps
1. Test on KDE KWin Wayland (ensure no regression)
2. Test on GNOME via IBus
3. Test on other compositors (niri, Hyprland)
4. Test `--no-gui` and `show_slint_window=false` modes

## Critical Context
- zbus 5 version: `zbus 5.15.0` pulled by Slint → accesskit_unix
- Signal API: `SignalEmitter` at `zbus::object_server::SignalEmitter`
- Signals defined with `#[zbus(signal)]` inside `#[interface]` impl block
- Signal emitter parameter: `#[zbus(signal_emitter)] ctxt: SignalEmitter<'_>` (owned, not `&`)
- Object server parameter: `#[zbus(object_server)] server: &zbus::ObjectServer`
- Signal call pattern: `InputContext::commit_text(&ctxt, val).await`
- `StructureBuilder::build()` returns `Result<Structure>` in zvariant 5
- `Dict::new(&key_sig, &value_sig)` takes `&Signature`
- `Connection::session().await` (not `connection::Builder::session()`)
- `conn.object_server().at(path, iface).await` to register interface
- `conn.request_name(name).await` returns `Result<()>`
- `gui_tx` type is `Sender<GuiEvent>` from `qianyan_ime_ui`
- `DisplayCandidate` is `qianyan_ime_ui::DisplayCandidate`

## Relevant Files
- `crates/platform-linux/Cargo.toml`
- `crates/platform-linux/src/hosts/ibus_backend.rs`
- `crates/platform-linux/src/hosts/mod.rs`
- `crates/platform-linux/src/runtime.rs`
- `src/main.rs`
- `crates/platform-linux/src/hosts/wayland_host_v1.rs`
- `crates/platform-linux/src/hosts/wayland_host.rs`
- `crates/platform-linux/src/hosts/vkbd.rs`
- `/release/keytao-app/crates/keytao-linux-ime/src/ibus_backend.rs`
- `dicts/chinese/chars/level4.json` (generated, 13,073 entries, 420 pinyin keys)
- `tools/gen_level4.py` (generation script)
- `crates/core/src/utils.rs` (load_single_syllables updated to include level4)
- `crates/engine/src/compiler.rs` (auto-discovers level4.json)
