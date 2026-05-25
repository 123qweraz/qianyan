use crate::CandidateDisplay;
use qianyan_ime_core::Config;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::ConnectionExt;

const OFFSCREEN_POS: i32 = 30_000;

/// Send EWMH ClientMessage to add _NET_WM_STATE_SKIP_TASKBAR and SKIP_PAGER.
/// Uses x11rb for proper protocol compliance (xprop -set doesn't work for atom arrays).
#[cfg(target_os = "linux")]
fn set_skip_taskbar_and_hide() {
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(v) => v,
        Err(_) => return,
    };
    let screen = &conn.setup().roots[screen_num];

    let intern = |name: &[u8]| -> Option<u32> {
        conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
    };

    let Some(net_wm_state) = intern(b"_NET_WM_STATE") else { return };
    let Some(skip_taskbar) = intern(b"_NET_WM_STATE_SKIP_TASKBAR") else { return };
    let Some(skip_pager) = intern(b"_NET_WM_STATE_SKIP_PAGER") else { return };

    // Find the status bar window by name (xdotool is simpler than tree walking)
    let out = match std::process::Command::new("xdotool")
        .args(["search", "--name", "RustImeStatusBar"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return,
    };
    let ids = String::from_utf8_lossy(&out.stdout);
    for id_str in ids.lines() {
        let id_str = id_str.trim();
        if id_str.is_empty() { continue; }
        let window_id = match u32::from_str_radix(id_str, 10) {
            Ok(id) => id,
            Err(_) => continue,
        };

        // _NET_WM_STATE ClientMessage: data[0]=1(ADD), data[1]=atom1, data[2]=atom2, data[3]=1(source), data[4]=0
        let event = x11rb::protocol::xproto::ClientMessageEvent {
            response_type: 33,  // ClientMessage
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
    let _ = conn.flush();
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
    status_bar: StatusBar,
    config: Config,
    window_visible: bool,
    candidate_enabled: bool,
    last_x: i32,
    last_y: i32,
    status_bar_visible: bool,
}

impl SlintDisplay {
    pub fn new(config: Config) -> Self {
        let window = CandidateWindow::new().expect("Failed to create CandidateWindow");
        let status_bar = StatusBar::new().expect("Failed to create StatusBar");

        // 状态栏作为永久锚点窗口（始终 show，移出屏幕），防止 hide() 候选窗口时
        // Slint 1.9 winit software 后端因无窗口而自动退出 event loop
        let _ = status_bar.window().show();
        status_bar.window().set_position(slint::WindowPosition::Physical(
            slint::PhysicalPosition::new(OFFSCREEN_POS, OFFSCREEN_POS),
        ));
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(500));
            set_skip_taskbar_and_hide();
        });

        let candidate_enabled = cfg!(not(target_os = "linux")) || config.linux.show_slint_window;
        let mut display = Self {
            window,
            status_bar,
            config: config.clone(),
            window_visible: false,
            candidate_enabled,
            last_x: 0,
            last_y: 0,
            status_bar_visible: false,
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
            .set_show_english_aux(config.appearance.show_english_aux);
        self.window
            .set_show_stroke_aux(config.appearance.show_stroke_aux);
        self.window
            .set_show_translation(config.appearance.show_english_translation);
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
            "{}, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui",
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

    fn apply_corner_position(&mut self) {
        let offset_x = self.config.linux.fixed_x;
        let offset_y = self.config.linux.fixed_y;
        let (sw, sh) = screen_size();
        let s = self.window.window().size();
        let w = s.width as i32;
        let h = s.height as i32;
        let (tx, ty) = match self.config.linux.corner.as_str() {
            "top-right" => (sw - w - offset_x, offset_y),
            "bottom-left" => (offset_x, sh - h - offset_y),
            "bottom-right" => (sw - w - offset_x, sh - h - offset_y),
            _ => (offset_x, offset_y), // top-left default
        };
        self.window.window().set_position(slint::WindowPosition::Physical(
            slint::PhysicalPosition::new(tx, ty),
        ));
    }
}

impl CandidateDisplay for SlintDisplay {
    fn update_candidates(
        &mut self,
        pinyin: &str,
        candidates: Vec<crate::DisplayCandidate>,
        selected: usize,
    ) {
        if !self.candidate_enabled || pinyin.is_empty() || !self.config.appearance.show_candidates {
            self.set_visible(false);
            return;
        }

        self.window.set_pinyin(SharedString::from(pinyin));
        self.window.set_selected_index(selected as i32);

        let mut cand_models = Vec::new();
        for c in candidates {
            cand_models.push(CandidateData {
                text: SharedString::from(c.text),
                label: SharedString::from(c.label),
                english_aux: SharedString::from(c.hint),
                stroke_aux: SharedString::from(""),
            });
        }
        self.window
            .set_candidates(ModelRc::from(std::rc::Rc::new(VecModel::from(cand_models))));

        self.set_visible(true);
    }

    fn update_status(&mut self, text: &str, chinese_enabled: bool) {
        if !text.is_empty() {
            self.status_bar.set_status_text(SharedString::from(text));
        }
        self.status_bar.set_chinese_enabled(chinese_enabled);
    }

    fn set_status_bar_visible(&mut self, visible: bool) {
        self.status_bar_visible = visible;
        self.config.appearance.show_status_bar = visible;
        if visible {
            let _ = self.status_bar.window().show();
            self.status_bar.window().set_position(
                slint::WindowPosition::Physical(slint::PhysicalPosition::new(self.last_x, self.last_y)),
            );
        } else {
            self.status_bar.window().set_position(
                slint::WindowPosition::Physical(slint::PhysicalPosition::new(OFFSCREEN_POS, OFFSCREEN_POS)),
            );
        }
    }

    fn move_to(&mut self, x: i32, y: i32) {
        self.last_x = x;
        self.last_y = y;

        // 如果状态栏可见，跟随光标移动
        if self.status_bar_visible {
            self.status_bar.window().set_position(slint::WindowPosition::Physical(
                slint::PhysicalPosition::new(x, y),
            ));
        }

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
            let _ = self.window.window().show();
            #[cfg(target_os = "linux")]
            if self.config.linux.fixed_position {
                self.apply_corner_position();
            } else {
                self.window.window().set_position(slint::WindowPosition::Physical(
                    slint::PhysicalPosition::new(self.last_x, self.last_y),
                ));
            }
        } else {
            // 安全 hide(): 状态栏作为锚点窗口保持 event loop 运行
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
        let _ = self.status_bar.window().hide();
        self.window_visible = false;
    }
}
