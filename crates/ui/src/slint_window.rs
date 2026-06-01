use crate::CandidateDisplay;
use qianyan_ime_core::Config;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::ConnectionExt;
#[cfg(target_os = "linux")]
use x11rb::wrapper::ConnectionExt as _;

/// Find a child window whose `WM_NAME` or `_NET_WM_NAME` equals `target`.
#[cfg(target_os = "linux")]
fn find_windows_by_name(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    target: &str,
    net_wm_name: u32,
) -> Vec<u32> {
    let mut results = Vec::new();

    let check = |w: u32| {
        let name = conn
            .get_property(false, w, net_wm_name, x11rb::protocol::xproto::AtomEnum::ANY, 0, 1024)
            .ok()
            .and_then(|c| c.reply().ok())
            .filter(|r| r.format == 8 && !r.value.is_empty())
            .and_then(|r| {
                let s = String::from_utf8_lossy(&r.value);
                Some(s.trim_end_matches('\0').to_string())
            })
            .or_else(|| {
                conn.get_property(false, w, x11rb::protocol::xproto::AtomEnum::WM_NAME, x11rb::protocol::xproto::AtomEnum::ANY, 0, 1024)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .filter(|r| r.format == 8 && !r.value.is_empty())
                    .and_then(|r| {
                        let s = String::from_utf8_lossy(&r.value);
                        Some(s.trim_end_matches('\0').to_string())
                    })
            });
        name.filter(|n| n == target).is_some()
    };

    if check(root) {
        results.push(root);
    }
    if let Ok(cookie) = conn.query_tree(root) {
        if let Ok(reply) = cookie.reply() {
            for child in reply.children {
                results.append(&mut find_windows_by_name(conn, child, target, net_wm_name));
            }
        }
    }
    results
}

/// Set `_NET_WM_WINDOW_TYPE` to `UTILITY` and
/// `_NET_WM_STATE` to `SKIP_TASKBAR | SKIP_PAGER`
/// for every window whose name matches one of `window_names`.
#[cfg(target_os = "linux")]
fn set_skip_taskbar_and_hide() -> bool {
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let screen = &conn.setup().roots[screen_num];

    let intern = |name: &[u8]| -> Option<u32> {
        conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
    };

    let Some(net_wm_state) = intern(b"_NET_WM_STATE") else { return false };
    let Some(skip_taskbar) = intern(b"_NET_WM_STATE_SKIP_TASKBAR") else { return false };
    let Some(skip_pager) = intern(b"_NET_WM_STATE_SKIP_PAGER") else { return false };
    let Some(net_wm_window_type) = intern(b"_NET_WM_WINDOW_TYPE") else { return false };
    let Some(utility) = intern(b"_NET_WM_WINDOW_TYPE_UTILITY") else { return false };
    let Some(net_wm_name) = intern(b"_NET_WM_NAME") else { return false };

    let window_names = ["QianyanIMECandidateWindow", "QianyanIMEAnchor"];
    let mut found = false;

    for name in &window_names {
        let windows = find_windows_by_name(&conn, screen.root, name, net_wm_name);
        for &window_id in &windows {
            found = true;

            // Set _NET_WM_WINDOW_TYPE to UTILITY (more reliably removes from taskbar)
            use x11rb::protocol::xproto::PropMode;
            let _ = conn.change_property32(
                PropMode::REPLACE,
                window_id,
                net_wm_window_type,
                x11rb::protocol::xproto::AtomEnum::ATOM,
                &[utility],
            );

            // Send ClientMessage to add _NET_WM_STATE_SKIP_TASKBAR and SKIP_PAGER
            let event = x11rb::protocol::xproto::ClientMessageEvent {
                response_type: 33,
                format: 32,
                sequence: 0,
                window: window_id,
                type_: net_wm_state,
                data: x11rb::protocol::xproto::ClientMessageData::from([
                    1u32,       // _NET_WM_STATE_ADD
                    skip_taskbar,
                    skip_pager,
                    1,          // source: normal application
                    0,
                ]),
            };
            let _ = conn.send_event(
                false,
                screen.root,
                x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
                    | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            );
        }
    }
    let _ = conn.flush();
    found
}

slint::include_modules!();

fn screen_size() -> (i32, i32) {
    if let Ok(out) = std::process::Command::new("xdotool")
        .arg("getdisplaygeometry")
        .output()
    {
        if let Ok(s) = String::from_utf8(out.stdout) {
            let parts: Vec<&str> = s.trim().split_whitespace().collect();
            if parts.len() == 2 {
                if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                    return (w, h);
                }
            }
        }
    }
    (1920, 1080)
}

pub struct SlintDisplay {
    window: CandidateWindow,
    anchor: AnchorWindow,
    config: Config,
    window_visible: bool,
    candidate_enabled: bool,
    last_x: i32,
    last_y: i32,
}

impl SlintDisplay {
    pub fn new(config: Config) -> Self {
        let window = CandidateWindow::new().expect("Failed to create CandidateWindow");
        let anchor = AnchorWindow::new().expect("Failed to create AnchorWindow");

        // 1x1 透明锚点窗口（始终 show），防止 hide() 候选窗口时
        // Slint 后端因无窗口而自动退出 event loop。
        let _ = anchor.window().show();
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            for i in 0..5 {
                std::thread::sleep(std::time::Duration::from_millis(500 * (i + 1)));
                if set_skip_taskbar_and_hide() {
                    break;
                }
            }
        });

        let candidate_enabled = cfg!(not(target_os = "linux")) || config.linux.show_slint_window;
        let mut display = Self {
            window,
            anchor,
            config: config.clone(),
            window_visible: false,
            candidate_enabled,
            last_x: 0,
            last_y: 0,
        };

        display.apply_style(&config);
        display
    }

    fn apply_style(&mut self, config: &Config) {
        let parse_color = |s: &str| -> slint::Color {
            if s.starts_with('#') {
                if s.len() == 7 {
                    let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
                    let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
                    let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
                    slint::Color::from_rgb_u8(r, g, b)
                } else if s.len() == 9 {
                    let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
                    let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
                    let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
                    let a = u8::from_str_radix(&s[7..9], 16).unwrap_or(255);
                    slint::Color::from_argb_u8(a, r, g, b)
                } else {
                    slint::Color::from_rgb_u8(255, 255, 255)
                }
            } else if s.starts_with("rgba(") {
                let parts: Vec<&str> = s[5..s.len() - 1].split(',').map(|p| p.trim()).collect();
                if parts.len() == 4 {
                    let r = parts[0].parse::<u8>().unwrap_or(255);
                    let g = parts[1].parse::<u8>().unwrap_or(255);
                    let b = parts[2].parse::<u8>().unwrap_or(255);
                    let a = (parts[3].parse::<f32>().unwrap_or(1.0) * 255.0) as u8;
                    slint::Color::from_argb_u8(a, r, g, b)
                } else {
                    slint::Color::from_rgb_u8(255, 255, 255)
                }
            } else {
                slint::Color::from_rgb_u8(9, 105, 218)
            }
        };

        self.window
            .set_is_horizontal(config.appearance.candidate_layout == "horizontal");

        self.window
            .set_bg_color(parse_color(&config.appearance.window_bg_color));
        self.window
            .set_accent_color(parse_color(&config.appearance.window_highlight_color));
        self.window
            .set_border_color(parse_color(&config.appearance.window_border_color));
        self.window
            .set_text_color(parse_color(&config.appearance.candidate_text.color));
        self.window
            .set_highlight_text_color(parse_color(&config.appearance.window_highlight_text_color));

        let font_stack = format!(
            "{}, Noto Color Emoji, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui",
            config.appearance.candidate_text.font_family
        );
        self.window
            .set_pinyin_font_family(SharedString::from(&font_stack));
        self.window
            .set_candidate_font_family(SharedString::from(&font_stack));

        self.window
            .set_pinyin_font_size(config.appearance.pinyin_text.font_size as f32);
        self.window
            .set_pinyin_font_weight(config.appearance.pinyin_text.font_weight as i32);
        self.window
            .set_candidate_font_size(config.appearance.candidate_text.font_size as f32);
        self.window
            .set_candidate_font_weight(config.appearance.candidate_text.font_weight as i32);
    }

    #[cfg(target_os = "linux")]
    fn apply_corner_position(&mut self) {
        let (tx, ty) = self.corner_position(&self.window.window());
        self.window.window().set_position(slint::WindowPosition::Physical(
            slint::PhysicalPosition::new(tx, ty),
        ));
    }

    /// 计算窗口在配置角落的位置，使用 config.linux.fixed_x/fixed_y 偏移
    #[cfg(target_os = "linux")]
    fn corner_position(&self, win: &slint::Window) -> (i32, i32) {
        let offset_x = self.config.linux.fixed_x;
        let offset_y = self.config.linux.fixed_y;
        let (sw, sh) = screen_size();
        let s = win.size();
        let w = s.width as i32;
        let h = s.height as i32;
        match self.config.linux.corner.as_str() {
            "top-right" => (sw - w - offset_x, offset_y),
            "bottom-left" => (offset_x, sh - h - offset_y),
            "bottom-right" => (sw - w - offset_x, sh - h - offset_y),
            _ => (offset_x, offset_y),
        }
    }
}

impl CandidateDisplay for SlintDisplay {
    fn update_candidates(
        &mut self,
        pinyin: &str,
        candidates: Vec<crate::DisplayCandidate>,
        selected: usize,
        page: usize,
        total_pages: usize,
    ) {
        if !self.candidate_enabled || pinyin.is_empty() || !self.config.appearance.show_candidates {
            self.set_visible(false);
            return;
        }

        self.window.set_pinyin(SharedString::from(pinyin));
        self.window.set_selected_index(selected as i32);
        self.window.set_current_page(page as i32);
        self.window.set_total_pages(total_pages as i32);

        let mut cand_models = Vec::new();
        for c in candidates {
            cand_models.push(CandidateData {
                text: SharedString::from(c.text),
                label: SharedString::from(c.label),
                english_aux: SharedString::from(c.hint),
                stroke_aux: SharedString::from(""),
                is_fuzzy: c.is_fuzzy,
            });
        }
        self.window
            .set_candidates(ModelRc::from(std::rc::Rc::new(VecModel::from(cand_models))));

        self.set_visible(true);
    }

    fn update_status(&mut self, _text: &str, _chinese_enabled: bool) {
        // StatusBar 已移除，状态通过托盘图标显示
    }

    fn move_to(&mut self, x: i32, y: i32) {
        self.last_x = x;
        self.last_y = y;

        if !self.window_visible {
            return;
        }
        #[cfg(target_os = "linux")]
        if self.config.linux.fixed_position {
            self.apply_corner_position();
            return;
        }
        self.window.window().set_position(slint::WindowPosition::Physical(
            slint::PhysicalPosition::new(x, y),
        ));
    }

    fn set_visible(&mut self, visible: bool) {
        let effective = visible && self.candidate_enabled;
        if effective == self.window_visible {
            return;
        }
        self.window.set_is_visible(effective);
        if effective {
            #[cfg(target_os = "linux")]
            if self.config.linux.fixed_position {
                self.apply_corner_position();
            } else {
                self.window.window().set_position(slint::WindowPosition::Physical(
                    slint::PhysicalPosition::new(self.last_x, self.last_y),
                ));
            }
            let _ = self.window.window().show();
        } else {
            // 安全 hide(): 锚点窗口保持 event loop 运行
            let _ = self.window.window().hide();
        }
        self.window_visible = visible;
    }

    fn apply_config(&mut self, config: &Config) {
        self.config = config.clone();
        self.candidate_enabled = cfg!(not(target_os = "linux")) || config.linux.show_slint_window;
        self.apply_style(config);
    }

    fn close(&mut self) {
        let _ = self.window.window().hide();
        let _ = self.anchor.window().hide();
        self.window_visible = false;
    }
}
