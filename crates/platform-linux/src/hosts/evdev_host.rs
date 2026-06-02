use super::vkbd::Vkbd;
use evdev::{Device, InputEventKind, Key};
use qianyan_ime_core::{InputMethodHost, Rect};
use qianyan_ime_engine::compositor::Compositor;
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::pipeline::MAX_LOOKUP_LIMIT;
use qianyan_ime_engine::processor::Action;
use qianyan_ime_engine::Processor;
use qianyan_ime_ui::GuiEvent;
use std::collections::HashSet;
use std::os::unix::io::AsRawFd;
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};

fn evdev_to_virtual(key: Key) -> Option<VirtualKey> {
    match key {
        Key::KEY_A => Some(VirtualKey::A),
        Key::KEY_B => Some(VirtualKey::B),
        Key::KEY_C => Some(VirtualKey::C),
        Key::KEY_D => Some(VirtualKey::D),
        Key::KEY_E => Some(VirtualKey::E),
        Key::KEY_F => Some(VirtualKey::F),
        Key::KEY_G => Some(VirtualKey::G),
        Key::KEY_H => Some(VirtualKey::H),
        Key::KEY_I => Some(VirtualKey::I),
        Key::KEY_J => Some(VirtualKey::J),
        Key::KEY_K => Some(VirtualKey::K),
        Key::KEY_L => Some(VirtualKey::L),
        Key::KEY_M => Some(VirtualKey::M),
        Key::KEY_N => Some(VirtualKey::N),
        Key::KEY_O => Some(VirtualKey::O),
        Key::KEY_P => Some(VirtualKey::P),
        Key::KEY_Q => Some(VirtualKey::Q),
        Key::KEY_R => Some(VirtualKey::R),
        Key::KEY_S => Some(VirtualKey::S),
        Key::KEY_T => Some(VirtualKey::T),
        Key::KEY_U => Some(VirtualKey::U),
        Key::KEY_V => Some(VirtualKey::V),
        Key::KEY_W => Some(VirtualKey::W),
        Key::KEY_X => Some(VirtualKey::X),
        Key::KEY_Y => Some(VirtualKey::Y),
        Key::KEY_Z => Some(VirtualKey::Z),
        Key::KEY_0 => Some(VirtualKey::Digit0),
        Key::KEY_1 => Some(VirtualKey::Digit1),
        Key::KEY_2 => Some(VirtualKey::Digit2),
        Key::KEY_3 => Some(VirtualKey::Digit3),
        Key::KEY_4 => Some(VirtualKey::Digit4),
        Key::KEY_5 => Some(VirtualKey::Digit5),
        Key::KEY_6 => Some(VirtualKey::Digit6),
        Key::KEY_7 => Some(VirtualKey::Digit7),
        Key::KEY_8 => Some(VirtualKey::Digit8),
        Key::KEY_9 => Some(VirtualKey::Digit9),
        Key::KEY_SPACE => Some(VirtualKey::Space),
        Key::KEY_ENTER | Key::KEY_KPENTER => Some(VirtualKey::Enter),
        Key::KEY_TAB => Some(VirtualKey::Tab),
        Key::KEY_BACKSPACE => Some(VirtualKey::Backspace),
        Key::KEY_ESC => Some(VirtualKey::Esc),
        Key::KEY_CAPSLOCK => Some(VirtualKey::CapsLock),
        Key::KEY_LEFTSHIFT | Key::KEY_RIGHTSHIFT => Some(VirtualKey::Shift),
        Key::KEY_LEFTCTRL | Key::KEY_RIGHTCTRL => Some(VirtualKey::Control),
        Key::KEY_LEFTALT | Key::KEY_RIGHTALT => Some(VirtualKey::Alt),
        Key::KEY_LEFT => Some(VirtualKey::Left),
        Key::KEY_RIGHT => Some(VirtualKey::Right),
        Key::KEY_UP => Some(VirtualKey::Up),
        Key::KEY_DOWN => Some(VirtualKey::Down),
        Key::KEY_PAGEUP => Some(VirtualKey::PageUp),
        Key::KEY_PAGEDOWN => Some(VirtualKey::PageDown),
        Key::KEY_HOME => Some(VirtualKey::Home),
        Key::KEY_END => Some(VirtualKey::End),
        Key::KEY_DELETE => Some(VirtualKey::Delete),
        Key::KEY_GRAVE => Some(VirtualKey::Grave),
        Key::KEY_MINUS => Some(VirtualKey::Minus),
        Key::KEY_EQUAL => Some(VirtualKey::Equal),
        Key::KEY_LEFTBRACE => Some(VirtualKey::LeftBrace),
        Key::KEY_RIGHTBRACE => Some(VirtualKey::RightBrace),
        Key::KEY_BACKSLASH => Some(VirtualKey::Backslash),
        Key::KEY_SEMICOLON => Some(VirtualKey::Semicolon),
        Key::KEY_APOSTROPHE => Some(VirtualKey::Apostrophe),
        Key::KEY_COMMA => Some(VirtualKey::Comma),
        Key::KEY_DOT => Some(VirtualKey::Dot),
        Key::KEY_SLASH => Some(VirtualKey::Slash),
        _ => None,
    }
}

/// State snapshot for background search, read under Processor lock but used outside.
struct BgState {
    buffer: String,
    profile: String,
    config: qianyan_ime_core::config::Config,
    aux_filter: String,
    filter_mode: qianyan_ime_engine::processor::FilterMode,
    fuzzy_activated: bool,
}

pub struct EvdevHost {
    processor: Arc<Mutex<Processor>>,
    pub vkbd: Arc<Mutex<Vkbd>>,
    dev: Arc<Mutex<Device>>,
    gui_tx: Option<Sender<GuiEvent>>,
    tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
    should_exit: Arc<AtomicBool>,
    tab_held_and_not_used: bool,
    lookup_tx: std::sync::mpsc::Sender<()>,
    lookup_completion: Arc<(Mutex<bool>, Condvar)>,
    is_grabbed: bool,
    meta_was_pressed: bool,
    epoll_fd: std::os::unix::io::RawFd,
}

struct GrabGuard {
    device: Arc<Mutex<Device>>,
    is_grabbed: bool,
}

impl GrabGuard {
    fn new(device: Arc<Mutex<Device>>) -> Self {
        let is_grabbed = if let Ok(mut dev) = device.lock() {
            if let Err(e) = dev.grab() {
                eprintln!("[EvdevHost] 警告: 无法锁定键盘设备: {e}");
                false
            } else {
                println!("[EvdevHost] 已成功锁定键盘硬件拦截。");
                true
            }
        } else {
            false
        };
        Self { device, is_grabbed }
    }

    fn ungrab(&mut self) {
        if self.is_grabbed {
            if let Ok(mut dev) = self.device.lock() {
                if dev.ungrab().is_ok() {
                    self.is_grabbed = false;
                    println!("[EvdevHost] 已临时释放键盘拦截");
                }
            }
        }
    }

    fn re_grab(&mut self) -> bool {
        if !self.is_grabbed {
            if let Ok(mut dev) = self.device.lock() {
                if dev.grab().is_ok() {
                    self.is_grabbed = true;
                    println!("[EvdevHost] 已重新获取键盘拦截");
                    return true;
                }
            }
        }
        false
    }
}

impl Drop for GrabGuard {
    fn drop(&mut self) {
        if self.is_grabbed {
            if let Ok(mut dev) = self.device.lock() {
                let _ = dev.ungrab();
                println!("[EvdevHost] 键盘硬件拦截已安全释放。");
            }
        }
    }
}

impl Drop for EvdevHost {
    fn drop(&mut self) {
        if self.epoll_fd >= 0 {
            unsafe { libc::close(self.epoll_fd); }
        }
    }
}

impl EvdevHost {
    pub fn new(
        processor: Arc<Mutex<Processor>>,
        device_path: &str,
        gui_tx: Option<Sender<GuiEvent>>,
        tray_tx: Sender<qianyan_ime_ui::tray::TrayEvent>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let dev = Device::open(device_path)?;
        // 使用 epoll 阻塞等待键盘事件，避免 1ms 轮询空转 CPU
        let epoll_fd = unsafe {
            let fd = libc::epoll_create1(0);
            if fd < 0 {
                return Err("epoll_create1 failed".into());
            }
            let mut event = libc::epoll_event {
                events: (libc::EPOLLIN | libc::EPOLLET) as u32,
                u64: 0,
            };
            let ret = libc::epoll_ctl(fd, libc::EPOLL_CTL_ADD, dev.as_raw_fd(), &mut event);
            if ret < 0 {
                let _ = libc::close(fd);
                return Err("epoll_ctl add failed".into());
            }
            fd
        };
        let vkbd_raw = Vkbd::new(&dev)?;
        let vkbd = Arc::new(Mutex::new(vkbd_raw));
        {
            if let Ok(p) = processor.lock() {
                if let Ok(mut vk) = vkbd.lock() {
                    vk.apply_config(&p.ctx.config.master_config);
                }
            }
        }
        let (lookup_tx, lookup_rx) = std::sync::mpsc::channel::<()>();
        let lookup_completion = Arc::new((Mutex::new(false), Condvar::new()));

        // 启动后台检索线程（克隆 SearchEngine 避免 engine.search() 占用 Processor 锁）
        let p_bg = processor.clone();
        let v_bg = vkbd.clone();
        let g_bg = gui_tx.clone();
        let pending_bg = lookup_completion.clone();
        let engine_bg = {
            let p = processor.lock().expect("processor lock poisoned");
            p.ctx.engine.clone()
        };

        std::thread::spawn(move || {
            while lookup_rx.recv().is_ok() {
                struct PendingGuard(Arc<(Mutex<bool>, Condvar)>);
                impl Drop for PendingGuard {
                    fn drop(&mut self) {
                        let (lock, cvar) = &*self.0;
                        *lock.lock().expect("lookup_completion lock poisoned") = false;
                        cvar.notify_all();
                    }
                }
                let _guard = PendingGuard(pending_bg.clone());
                while lookup_rx.try_recv().is_ok() {}

                // Step 1: 读取 buffer、配置和辅助码状态（单次获取锁）
                let state = {
                    let p = match p_bg.lock() {
                        Ok(guard) => guard,
                        Err(_) => continue,
                    };
                    if p.ctx.session.buffer.is_empty() {
                        update_gui_internal(&p, &g_bg);
                        continue;
                    }
                    BgState {
                        buffer: p.ctx.session.buffer.clone(),
                        profile: p.ctx.session_state.active_profiles.first().cloned().unwrap_or_default(),
                        config: p.ctx.config.master_config.clone(),
                        aux_filter: p.ctx.session.aux_filter.clone(),
                        filter_mode: p.ctx.session.filter_mode.clone(),
                        fuzzy_activated: p.ctx.session.fuzzy_activated,
                    }
                };

                // Step 2: 在不持有 Processor 锁的情况下执行检索
                let syllables = engine_bg.syllables.clone();
                let query = qianyan_ime_engine::pipeline::SearchQuery {
                    buffer: &state.buffer,
                    profile: &state.profile,
                    syllables: &syllables,
                    config: &state.config,
                    limit: MAX_LOOKUP_LIMIT,
                    filter_mode: state.filter_mode,
                    aux_filter: &state.aux_filter,
                    context: None,
                    fuzzy_enabled: state.fuzzy_activated,
                };
                let (results, segments) = engine_bg.search(query);

                // Step 3: 更新状态、检查自动上屏（单次获取锁）
                {
                    let mut p = match p_bg.lock() {
                        Ok(guard) => guard,
                        Err(_) => continue,
                    };
                    p.ctx.session.candidates = results;
                    p.ctx.session.best_segmentation = segments;
                    p.ctx.session.has_dict_match = !p.ctx.session.candidates.is_empty();
                    p.ctx.session.last_lookup_pinyin = p.ctx.session.buffer.clone();

                    if p.ctx.session.candidates.is_empty() {
                        let buf_arc: std::sync::Arc<str> = std::sync::Arc::from(
                            p.ctx.session.buffer.as_str(),
                        );
                        p.ctx.session.candidates.push(
                            qianyan_ime_engine::pipeline::Candidate {
                                text: buf_arc.clone(),
                                simplified: buf_arc.clone(),
                                traditional: buf_arc.clone(),
                                hint: std::sync::Arc::from(""),
                                english_aux: std::sync::Arc::from(""),
                                stroke_aux: std::sync::Arc::from(""),
                                source: std::sync::Arc::from("Raw"),
                                weight: 0.0,
                                match_level: 0,
                            },
                        );
                    }
                    p.ctx.session.update_state();

                    if let Some(commit_action) = Compositor::check_auto_commit(&mut p.ctx) {
                        drop(p);
                        if let Ok(vkbd) = v_bg.lock() {
                            execute_action(&vkbd, &g_bg, commit_action, None);
                        }
                        let _ = p_bg.lock().map(|p| update_gui_internal(&p, &g_bg));
                        continue;
                    }

                    let phantom_action = p.update_phantom_action();
                    drop(p);
                    if phantom_action != Action::Consume {
                        if let Ok(vkbd) = v_bg.lock() {
                            execute_action(&vkbd, &g_bg, phantom_action, None);
                        }
                    }
                    let _ = p_bg.lock().map(|p| update_gui_internal(&p, &g_bg));
                }
            }
        });

        Ok(Self {
            processor,
            vkbd,
            dev: Arc::new(Mutex::new(dev)),
            gui_tx,
            tray_tx,
            should_exit: Arc::new(AtomicBool::new(false)),
            tab_held_and_not_used: false,
            lookup_tx,
            lookup_completion,
            is_grabbed: true,
            meta_was_pressed: false,
            epoll_fd,
        })
    }
}

impl InputMethodHost for EvdevHost {
    fn set_preedit(&self, _text: &str, _cursor_pos: usize) {}
    fn commit_text(&self, text: &str) {
        if let Ok(vkbd) = self.vkbd.lock() {
            vkbd.send_text(text, false);
        }
    }

    fn get_cursor_rect(&self) -> Option<Rect> {
        None
    }

    fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // 使用 RAII Guard 自动管理 grab 生命周期
        let mut grab_guard = GrabGuard::new(self.dev.clone());
        let mut held_keys = HashSet::new();
        println!("[EvdevHost] 正在运行硬件拦截循环...");

        while !self.should_exit.load(Ordering::Relaxed) {
            let events: Vec<_> = if let Ok(mut dev) = self.dev.lock() {
                match dev.fetch_events() {
                    Ok(evs) => evs.collect(),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // 用 epoll 阻塞等待键盘事件，不空转 CPU
                        // 200ms 超时确保 exit 信号能及时响应
                        let mut event =
                            std::mem::MaybeUninit::<libc::epoll_event>::uninit();
                        let ret = unsafe {
                            libc::epoll_wait(self.epoll_fd, event.as_mut_ptr(), 1, 200)
                        };
                        if ret < 0 {
                            let err = std::io::Error::last_os_error();
                            if err.kind() != std::io::ErrorKind::Interrupted {
                                return Err(err.into());
                            }
                        }
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            } else {
                break;
            };

            for ev in events {
                if let InputEventKind::Key(key) = ev.kind() {
                    let val = ev.value();

                    // 1. 基础状态维护 (必须在任何可能 continue 的逻辑之前更新状态，确保 held_keys 始终同步)
                    if val == 1 {
                        held_keys.insert(key);
                        if key != Key::KEY_TAB {
                            self.tab_held_and_not_used = false;
                        }
                    } else if val == 0 {
                        held_keys.remove(&key);
                    }

                    // 2. 检测修饰键状态
                    let is_meta_key = matches!(
                        key,
                        Key::KEY_LEFTMETA | Key::KEY_RIGHTMETA | Key::KEY_COMPOSE
                    );
                    let meta_is_held = held_keys.contains(&Key::KEY_LEFTMETA)
                        || held_keys.contains(&Key::KEY_RIGHTMETA);

                    // 3. Meta (Win) 键特殊处理：自动释放 grab 以支持系统快捷键
                    if is_meta_key && val == 1 {
                        self.meta_was_pressed = true;
                        // 强制清除所有修饰键状态，防止 Meta 弹窗时 Shift 等被锁定在按下状态
                        for mod_key in [
                            Key::KEY_LEFTSHIFT,
                            Key::KEY_RIGHTSHIFT,
                            Key::KEY_CAPSLOCK,
                            Key::KEY_LEFTCTRL,
                            Key::KEY_RIGHTCTRL,
                            Key::KEY_LEFTALT,
                            Key::KEY_RIGHTALT,
                        ] {
                            held_keys.remove(&mod_key);
                            if let Ok(vkbd) = self.vkbd.lock() {
                                vkbd.emit_raw(mod_key, 0);
                            }
                        }
                        if self.is_grabbed {
                            grab_guard.ungrab();
                            self.is_grabbed = false;
                        }
                    }

                    // Meta 键释放：检测是否重新获取拦截
                    // 只有当 meta_was_pressed 为 true 且所有按键都释放后，才重新 grab
                    if is_meta_key && val == 0 && !meta_is_held && self.meta_was_pressed
                        && !self.is_grabbed && held_keys.is_empty()
                        && grab_guard.re_grab() {
                            self.is_grabbed = true;
                            self.meta_was_pressed = false;
                    }

                    // 4. 快捷键透传判断
                    // 如果是 Meta 组合键、或者 Meta 键被按住、或者已释放 grab (系统正在处理)，则直接透传并跳过 IME
                    if is_meta_key || meta_is_held || !self.is_grabbed {
                        // 当 meta_was_pressed 为 true 且所有按键都释放后，重新 grab 键盘
                        if self.meta_was_pressed && !self.is_grabbed && held_keys.is_empty()
                            && grab_guard.re_grab() {
                                self.is_grabbed = true;
                                self.meta_was_pressed = false;
                        }
                        if let Ok(vkbd) = self.vkbd.lock() {
                            vkbd.emit_raw(key, val);
                        }
                        continue;
                    }

                    let shift = held_keys.contains(&Key::KEY_LEFTSHIFT)
                        || held_keys.contains(&Key::KEY_RIGHTSHIFT);
                    let ctrl = held_keys.contains(&Key::KEY_LEFTCTRL)
                        || held_keys.contains(&Key::KEY_RIGHTCTRL);
                    let alt = held_keys.contains(&Key::KEY_LEFTALT)
                        || held_keys.contains(&Key::KEY_RIGHTALT);

                    if let Ok(mut p) = self.processor.lock() {
                        if let Some(vk) = evdev_to_virtual(key) {
                            // 所有的按键（包含组合键）现在都交给 Processor 统一处理
                            let is_sync_key = vk == VirtualKey::Space
                                || vk == VirtualKey::Enter
                                || vk == VirtualKey::CapsLock
                                || vk == VirtualKey::Tab
                                || (vk.to_u32() >= VirtualKey::Digit0.to_u32()
                                    && vk.to_u32() <= VirtualKey::Digit9.to_u32())
                                || matches!(
                                    vk,
                                    VirtualKey::PageUp
                                        | VirtualKey::PageDown
                                        | VirtualKey::Up
                                        | VirtualKey::Down
                                        | VirtualKey::Left
                                        | VirtualKey::Right
                                        | VirtualKey::Minus
                                        | VirtualKey::Equal
                                        | VirtualKey::Comma
                                        | VirtualKey::Dot
                                );

                            if is_sync_key {
                                drop(p);
                                let (lock, cvar) = &*self.lookup_completion;
                                let mut pending = lock.lock().expect("lookup_completion lock poisoned");
                                while *pending {
                                    pending = cvar.wait(pending).expect("lookup_completion cvar wait failed");
                                }
                                drop(pending);
                                if let Ok(mut p_locked) = self.processor.lock() {
                                    eprintln!("[DEBUG] sync key handler: key={:?} val={}", vk, val);
                                    let prev_enabled = p_locked.ctx.session_state.chinese_enabled;
                                    let action =
                                        p_locked.handle_key_ext(vk, val, shift, ctrl, alt, true);
                                    eprintln!("[DEBUG] sync key action type: {}",
                                        match &action {
                                            qianyan_ime_engine::processor::Action::Emit(_) => "Emit",
                                            qianyan_ime_engine::processor::Action::DeleteAndEmit{..} => "DeleteAndEmit",
                                            qianyan_ime_engine::processor::Action::PassThrough => "PassThrough",
                                            qianyan_ime_engine::processor::Action::Consume => "Consume",
                                            qianyan_ime_engine::processor::Action::Alert => "Alert",
                                            qianyan_ime_engine::processor::Action::Notify(..) => "Notify",
                                        });

                                    let enabled = p_locked.ctx.session_state.chinese_enabled;
                                    if prev_enabled != enabled {
                                        let short = p_locked.get_short_display();
                                        let text = if enabled { short } else { "英".into() };
                                        if let Some(ref gui_tx) = self.gui_tx {
                                            let _ = gui_tx.send(qianyan_ime_ui::GuiEvent::ShowStatus(text, enabled));
                                        }
                                        let profile = p_locked.get_current_profile_display();
                                        let _ = self.tray_tx.send(
                                            qianyan_ime_ui::tray::TrayEvent::SyncStatus {
                                                chinese_enabled: enabled,
                                                active_profile: profile,
                                            },
                                        );
                                    }

                                    if let Ok(vkbd) = self.vkbd.lock() {
                                        execute_action(&vkbd, &self.gui_tx, action, Some((key, val)));
                                    } else {
                                        eprintln!("[DEBUG] FAILED to lock vkbd");
                                    }
                                    if val != 0 {
                                        drop(p_locked);
                                        eprintln!("[DEBUG] calling update_gui after sync key");
                                        self.update_gui();
                                        eprintln!("[DEBUG] update_gui done");
                                    }
                                } else {
                                    eprintln!("[DEBUG] FAILED to lock processor for sync key");
                                }
                            } else {
                                let fast_action =
                                    p.handle_key_ext(vk, val, shift, ctrl, alt, false);
                                if let Ok(vkbd) = self.vkbd.lock() {
                                    execute_action(&vkbd, &self.gui_tx, fast_action, Some((key, val)));
                                }

                                if val != 0 {
                                    {
                                        let (lock, _) = &*self.lookup_completion;
                                        *lock.lock().expect("lookup_completion lock poisoned") = true;
                                    }
                                    let _ = self.lookup_tx.send(());
                                    drop(p);
                                    self.update_gui();
                                }
                            }
                        } else {
                            if let Ok(vkbd) = self.vkbd.lock() {
                                vkbd.emit_raw(key, val);
                            }
                            drop(p);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl EvdevHost {
    fn update_gui(&self) {
        if let Ok(p) = self.processor.lock() {
            update_gui_internal(&p, &self.gui_tx);
        }
    }
}

fn update_gui_internal(p: &Processor, gui_tx: &Option<Sender<GuiEvent>>) {
    if let Some(ref tx) = gui_tx {
        let pinyin = qianyan_ime_engine::compositor::Compositor::get_preedit(&p.ctx);

        if pinyin.is_empty() || !p.ctx.session_state.chinese_enabled {
            let _ = tx.send(GuiEvent::Update {
                pinyin: "".into(),
                candidates: vec![],
                selected: 0,
                page: 0,
                total_pages: 0,
                sentence: "".into(),
                cursor_pos: 0,
                commit_mode: p.ctx.config.commit_mode().to_string(),
            });
            return;
        }

        let page_size = p.ctx.config.page_size();
        let start = p.ctx.session.page.min(p.ctx.session.candidates.len());
        let end = (start + page_size).min(p.ctx.session.candidates.len());

        let mut display_candidates = Vec::new();
        for (i, c) in p.ctx.session.candidates[start..end].iter().enumerate() {
            let is_fuzzy = c.match_level < 3 && c.source.as_ref() == "Table (Fuzzy)";
            let label = format!("{}.", i + 1);
            let full_display = if is_fuzzy {
                format!("{label}{}(模糊)", c.text)
            } else if c.hint.is_empty() {
                format!("{label}{}", c.text)
            } else {
                format!("{label}{}({})", c.text, c.hint)
            };
            display_candidates.push(qianyan_ime_ui::DisplayCandidate {
                text: c.text.to_string(),
                label,
                hint: c.hint.to_string(),
                full_display,
                is_fuzzy,
            });
        }

        let relative_selected = p.ctx.session.selected.saturating_sub(start);
        let current_page = if page_size > 0 { start / page_size } else { 0 };
        let total_pages = if page_size > 0 { (p.ctx.session.candidates.len() + page_size - 1) / page_size } else { 0 };

        let _ = tx.send(GuiEvent::Update {
            pinyin,
            candidates: display_candidates,
            selected: relative_selected,
            page: current_page,
            total_pages,
            sentence: p.ctx.session.joined_sentence.clone(),
            cursor_pos: p.ctx.session.cursor_pos,
            commit_mode: p.ctx.config.commit_mode().to_string(),
        });
    }
}

fn execute_action(
    vkbd: &Vkbd,
    gui_tx: &Option<Sender<GuiEvent>>,
    action: Action,
    raw_key: Option<(Key, i32)>,
) {
    match action {
        Action::Emit(s) => {
            eprintln!("[DEBUG] execute_action: Emit text='{}'", s);
            vkbd.send_text(&s, false);
            eprintln!("[DEBUG] execute_action: Emit done");
        }
        Action::DeleteAndEmit { delete, insert } => {
            eprintln!("[DEBUG] execute_action: DeleteAndEmit delete={} insert='{}'", delete, insert);
            // 注入前必须先隐藏候选窗口，避免 uinput 发出的 SPACE 被 Slint 候选窗口截获
            if let Some(ref tx) = gui_tx {
                let (ack_tx, ack_rx) = std::sync::mpsc::channel();
                eprintln!("[DEBUG] sending HideAndAck...");
                let send_result = tx.send(GuiEvent::HideAndAck(ack_tx));
                eprintln!("[DEBUG] HideAndAck send result: {:?}", send_result);
                let recv_result = ack_rx.recv_timeout(std::time::Duration::from_millis(100));
                eprintln!("[DEBUG] HideAndAck recv result: {:?}", recv_result);
            } else {
                eprintln!("[DEBUG] gui_tx is None, skipping HideAndAck");
            }
            if delete > 0 {
                eprintln!("[DEBUG] vkbd.backspace({})", delete);
                vkbd.backspace(delete);
            }
            if !insert.is_empty() {
                eprintln!("[DEBUG] vkbd.send_text('{}')", insert);
                vkbd.send_text(&insert, false);
                eprintln!("[DEBUG] vkbd.send_text done");
            }
        }
        Action::PassThrough => {
            if let Some((k, v)) = raw_key {
                vkbd.emit_raw(k, v);
            }
        }
        Action::Alert => {
            let root = crate::find_project_root();
            let sound_path = root.join("sounds/beep.wav");
            if sound_path.exists() {
                let _ = std::process::Command::new("canberra-gtk-play")
                    .arg("-f")
                    .arg(sound_path)
                    .spawn();
            } else {
                let _ = std::process::Command::new("canberra-gtk-play")
                    .arg("--id=dialog-error")
                    .spawn();
            }
        }
        _ => {}
    }
}
