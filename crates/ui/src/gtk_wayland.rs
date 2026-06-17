use crate::{CandidateDisplay, DisplayCandidate};
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;
use qianyan_ime_core::Config;
use std::cell::RefCell;
use std::sync::mpsc;

enum GtkCmd {
    UpdateCandidates {
        pinyin: String,
        candidates: Vec<DisplayCandidate>,
        selected: usize,
        page: usize,
        total_pages: usize,
    },
    MoveTo { x: i32, y: i32 },
    SetVisible(bool),
    ApplyConfig(Box<Config>),
    Exit,
}

pub struct GtkWaylandDisplay {
    tx: mpsc::Sender<GtkCmd>,
}

impl CandidateDisplay for GtkWaylandDisplay {
    fn update_candidates(
        &mut self,
        pinyin: &str,
        candidates: Vec<DisplayCandidate>,
        selected: usize,
        page: usize,
        total_pages: usize,
    ) {
        let _ = self.tx.send(GtkCmd::UpdateCandidates {
            pinyin: pinyin.to_string(),
            candidates,
            selected,
            page,
            total_pages,
        });
    }

    fn update_status(&mut self, _text: &str, _chinese_enabled: bool) {}

    fn move_to(&mut self, x: i32, y: i32) {
        let _ = self.tx.send(GtkCmd::MoveTo { x, y });
    }

    fn set_visible(&mut self, visible: bool) {
        let _ = self.tx.send(GtkCmd::SetVisible(visible));
    }

    fn apply_config(&mut self, config: &Config) {
        let _ = self.tx.send(GtkCmd::ApplyConfig(Box::new(config.clone())));
    }

    fn close(&mut self) {
        let _ = self.tx.send(GtkCmd::Exit);
    }
}

impl Drop for GtkWaylandDisplay {
    fn drop(&mut self) {
        let _ = self.tx.send(GtkCmd::Exit);
    }
}

struct GtkWlState {
    window: gtk4::Window,
    pinyin_label: gtk4::Label,
    candidate_list: gtk4::Box,
    config: Option<Box<Config>>,
    last_x: i32,
    last_y: i32,
    window_visible: bool,
    candidate_enabled: bool,
    screen_w: i32,
    screen_h: i32,
}

fn parse_color(s: &str) -> (u8, u8, u8, u8) {
    if s.starts_with('#') {
        if s.len() == 7 {
            let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
            let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
            let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
            (r, g, b, 255)
        } else if s.len() == 9 {
            let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
            let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
            let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
            let a = u8::from_str_radix(&s[7..9], 16).unwrap_or(255);
            (r, g, b, a)
        } else {
            (255, 255, 255, 255)
        }
    } else if s.starts_with("rgba(") {
        let parts: Vec<&str> = s[5..s.len() - 1].split(',').map(|p| p.trim()).collect();
        if parts.len() == 4 {
            let r = parts[0].parse::<u8>().unwrap_or(255);
            let g = parts[1].parse::<u8>().unwrap_or(255);
            let b = parts[2].parse::<u8>().unwrap_or(255);
            let a = (parts[3].parse::<f32>().unwrap_or(1.0) * 255.0) as u8;
            (r, g, b, a)
        } else {
            (9, 105, 218, 255)
        }
    } else {
        (9, 105, 218, 255)
    }
}

fn color_to_css(r: u8, g: u8, b: u8, a: u8) -> String {
    if a == 255 {
        format!("#{:02x}{:02x}{:02x}", r, g, b)
    } else {
        format!("rgba({},{},{},{:.2})", r, g, b, a as f32 / 255.0)
    }
}

fn apply_css(config: &Config) {
    let (r, g, b, a) = parse_color(&config.appearance.window_bg_color);
    let bg = color_to_css(r, g, b, a);
    let (r, g, b, a) = parse_color(&config.appearance.window_border_color);
    let border = color_to_css(r, g, b, a);
    let (r, g, b, a) = parse_color(&config.appearance.candidate_text.color);
    let text = color_to_css(r, g, b, a);
    let (r, g, b, a) = parse_color(&config.appearance.window_highlight_color);
    let highlight = color_to_css(r, g, b, a);
    let (r, g, b, a) = parse_color(&config.appearance.window_highlight_text_color);
    let highlight_text = color_to_css(r, g, b, a);

    let ff = if config.appearance.candidate_text.font_family.is_empty() {
        "Noto Color Emoji, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui".to_string()
    } else {
        format!(
            "{}, Noto Color Emoji, Segoe UI Emoji, Microsoft YaHei, Arial, system-ui",
            config.appearance.candidate_text.font_family
        )
    };
    let pf = config.appearance.pinyin_text.font_size;
    let pw = config.appearance.pinyin_text.font_weight;
    let cf = config.appearance.candidate_text.font_size;
    let cw = config.appearance.candidate_text.font_weight;

    let css = format!(
        r#"
window {{
    background-color: {bg};
    border: 1px solid {border};
    border-radius: 4px;
}}
.pinyin {{
    color: {text};
    font-size: {pf}px;
    font-weight: {pw};
    font-family: {ff};
    padding: 4px 8px;
}}
.candidate-row {{
    padding: 2px 8px;
}}
.candidate-row.selected {{
    background-color: {highlight};
    border-radius: 4px;
}}
.candidate-label {{
    color: {text};
    font-size: {cf}px;
    font-weight: {cw};
    font-family: {ff};
    margin-right: 8px;
}}
.candidate-text {{
    color: {text};
    font-size: {cf}px;
    font-weight: {cw};
    font-family: {ff};
}}
.candidate-text.selected {{
    color: {highlight_text};
}}
.candidate-label.selected {{
    color: {highlight_text};
}}
.candidate-hint {{
    color: {text};
    font-size: {cf}px;
    font-weight: {cw};
    font-family: {ff};
    opacity: 0.6;
    margin-left: 8px;
}}
"#,
    );

    let provider = gtk4::CssProvider::new();
    provider.load_from_string(&css);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

fn update_screen_size(state: &mut GtkWlState) {
    if let Some(display) = gtk4::gdk::Display::default() {
        let monitors = display.monitors();
        for i in 0..monitors.n_items() {
            if let Some(monitor) = monitors.item(i).and_downcast::<gtk4::gdk::Monitor>() {
                let geo = monitor.geometry();
                state.screen_w = geo.width();
                state.screen_h = geo.height();
                log::info!(
                    "[GTK_WL] monitor {}: {}x{} scale={}",
                    i, geo.width(), geo.height(), monitor.scale_factor()
                );
                return;
            }
        }
    }
    if state.screen_w == 0 {
        state.screen_w = 1920;
        state.screen_h = 1080;
    }
}

fn apply_corner_position(state: &GtkWlState) {
    let config = match state.config.as_ref() {
        Some(c) => c,
        None => return,
    };
    let mx = config.linux.fixed_x.max(0);
    let my = config.linux.fixed_y.max(0);

    state.window.set_anchor(gtk4_layer_shell::Edge::Top, false);
    state.window.set_anchor(gtk4_layer_shell::Edge::Bottom, false);
    state.window.set_anchor(gtk4_layer_shell::Edge::Left, false);
    state.window.set_anchor(gtk4_layer_shell::Edge::Right, false);

    match config.linux.corner.as_str() {
        "top-right" => {
            state.window.set_anchor(gtk4_layer_shell::Edge::Top, true);
            state.window.set_anchor(gtk4_layer_shell::Edge::Right, true);
            state.window.set_margin(gtk4_layer_shell::Edge::Top, my);
            state.window.set_margin(gtk4_layer_shell::Edge::Right, mx);
        }
        "bottom-left" => {
            state.window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
            state.window.set_anchor(gtk4_layer_shell::Edge::Left, true);
            state.window.set_margin(gtk4_layer_shell::Edge::Bottom, my);
            state.window.set_margin(gtk4_layer_shell::Edge::Left, mx);
        }
        "top-left" => {
            state.window.set_anchor(gtk4_layer_shell::Edge::Top, true);
            state.window.set_anchor(gtk4_layer_shell::Edge::Left, true);
            state.window.set_margin(gtk4_layer_shell::Edge::Top, my);
            state.window.set_margin(gtk4_layer_shell::Edge::Left, mx);
        }
        _ => {
            state.window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
            state.window.set_anchor(gtk4_layer_shell::Edge::Right, true);
            state.window.set_margin(gtk4_layer_shell::Edge::Bottom, my);
            state.window.set_margin(gtk4_layer_shell::Edge::Right, mx);
        }
    }
}

fn apply_cursor_position(state: &GtkWlState, x: i32, y: i32) {
    let sw = state.screen_w.max(1);
    let sh = state.screen_h.max(1);
    let w = state.window.width().max(100);
    let h = state.window.height().max(50);
    let margin = 20;

    state.window.set_anchor(gtk4_layer_shell::Edge::Top, false);
    state.window.set_anchor(gtk4_layer_shell::Edge::Bottom, false);
    state.window.set_anchor(gtk4_layer_shell::Edge::Left, false);
    state.window.set_anchor(gtk4_layer_shell::Edge::Right, false);

    if y + margin + h > sh {
        state.window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
        state.window.set_margin(gtk4_layer_shell::Edge::Bottom, sh - y + margin);
    } else {
        state.window.set_anchor(gtk4_layer_shell::Edge::Top, true);
        state.window.set_margin(gtk4_layer_shell::Edge::Top, y + margin);
    }

    if x + w > sw {
        state.window.set_anchor(gtk4_layer_shell::Edge::Right, true);
        state.window.set_margin(gtk4_layer_shell::Edge::Right, sw - x);
    } else {
        state.window.set_anchor(gtk4_layer_shell::Edge::Left, true);
        state.window.set_margin(gtk4_layer_shell::Edge::Left, x);
    }
}

fn update_candidates_inner(
    state: &mut GtkWlState,
    pinyin: &str,
    candidates: &[DisplayCandidate],
    selected: usize,
) {
    let visible = !pinyin.is_empty()
        && state.candidate_enabled
        && state.config.as_ref().is_some_and(|c| c.appearance.show_candidates);

    if !visible {
        state.set_visible_inner(false);
        return;
    }

    state.pinyin_label.set_text(pinyin);

    while let Some(child) = state.candidate_list.first_child() {
        state.candidate_list.remove(&child);
    }

    let is_horizontal = state
        .config
        .as_ref()
        .is_some_and(|c| c.appearance.candidate_layout == "horizontal");

    state.candidate_list.set_orientation(if is_horizontal {
        gtk4::Orientation::Horizontal
    } else {
        gtk4::Orientation::Vertical
    });

    for (i, c) in candidates.iter().enumerate() {
        let row = gtk4::Box::new(
            if is_horizontal {
                gtk4::Orientation::Vertical
            } else {
                gtk4::Orientation::Horizontal
            },
            4,
        );
        row.add_css_class("candidate-row");

        let is_selected = i == selected;

        let label = gtk4::Label::new(Some(&format!("{}.", c.label.trim_end_matches('.'))));
        label.set_xalign(1.0);
        label.add_css_class("candidate-label");

        let text = gtk4::Label::new(Some(&c.text));
        text.set_xalign(0.0);
        text.add_css_class("candidate-text");

        if is_horizontal {
            row.append(&text);
            let aux = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            aux.append(&label);
            if !c.hint.is_empty() {
                let hint = gtk4::Label::new(Some(&c.hint));
                hint.set_xalign(0.0);
                hint.add_css_class("candidate-hint");
                aux.append(&hint);
            }
            row.append(&aux);
        } else {
            row.append(&label);
            row.append(&text);
            if !c.hint.is_empty() {
                let hint = gtk4::Label::new(Some(&c.hint));
                hint.set_xalign(0.0);
                hint.add_css_class("candidate-hint");
                row.append(&hint);
            }
        }

        if is_selected {
            row.add_css_class("selected");
            label.add_css_class("selected");
            text.add_css_class("selected");
        }

        state.candidate_list.append(&row);
    }

    state.set_visible_inner(true);
}

impl GtkWlState {
    fn set_visible_inner(&mut self, visible: bool) {
        if visible == self.window_visible {
            return;
        }
        self.window_visible = visible;
        if visible {
            update_screen_size(self);
            if self.config.as_ref().is_some_and(|c| c.linux.fixed_position) {
                apply_corner_position(self);
            } else {
                apply_cursor_position(self, self.last_x, self.last_y);
            }
            self.window.set_visible(true);
        } else {
            self.window.set_visible(false);
        }
    }

    fn move_to_inner(&mut self, x: i32, y: i32) {
        self.last_x = x;
        self.last_y = y;
        if self.window_visible
            && self.config.as_ref().is_some_and(|c| !c.linux.fixed_position)
        {
            apply_cursor_position(self, x, y);
        }
    }
}

fn run_gtk(rx: mpsc::Receiver<GtkCmd>, config: Box<Config>) {
    gtk4::init().expect("Failed to initialize GTK");

    let main_loop = glib::MainLoop::new(None, false);

    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.set_default_size(1, 1);

    window.init_layer_shell();
    window.set_namespace(Some("qianyan-ime"));
    window.set_layer(gtk4_layer_shell::Layer::Overlay);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
    window.set_exclusive_zone(0);

    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    window.set_child(Some(&main_box));

    let pinyin_label = gtk4::Label::new(None);
    pinyin_label.add_css_class("pinyin");
    pinyin_label.set_xalign(0.0);
    pinyin_label.set_yalign(0.5);
    main_box.append(&pinyin_label);

    let candidate_list = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    candidate_list.add_css_class("candidate-list");
    main_box.append(&candidate_list);

    apply_css(&config);

    let state = RefCell::new(GtkWlState {
        window,
        pinyin_label,
        candidate_list,
        config: Some(config),
        last_x: 0,
        last_y: 0,
        window_visible: false,
        candidate_enabled: true,
        screen_w: 1920,
        screen_h: 1080,
    });

    {
        let mut s = state.borrow_mut();
        update_screen_size(&mut s);
    }

    let main_loop_clone = main_loop.clone();
    glib::idle_add_local(move || {
        loop {
            let cmd = match rx.try_recv() {
                Ok(cmd) => cmd,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    main_loop_clone.quit();
                    return ControlFlow::Break;
                }
            };
            let mut s = state.borrow_mut();
            match cmd {
                GtkCmd::UpdateCandidates {
                    pinyin,
                    candidates,
                    selected,
                    ..
                } => {
                    update_candidates_inner(&mut s, &pinyin, &candidates, selected);
                }
                GtkCmd::MoveTo { x, y } => {
                    s.move_to_inner(x, y);
                }
                GtkCmd::SetVisible(visible) => {
                    s.set_visible_inner(visible);
                }
                GtkCmd::ApplyConfig(config) => {
                    s.config = Some(config);
                    apply_css(s.config.as_ref().unwrap());
                }
                GtkCmd::Exit => {
                    main_loop_clone.quit();
                    return ControlFlow::Break;
                }
            }
        }
        ControlFlow::Continue
    });

    main_loop.run();
}

impl GtkWaylandDisplay {
    pub fn new(config: Config) -> Option<Self> {
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return None;
        }

        let (tx, rx) = mpsc::channel();
        let config = Box::new(config);

        let thread = std::thread::Builder::new()
            .name("gtk-wayland".into())
            .spawn(move || {
                run_gtk(rx, config);
            });

        match thread {
            Ok(_) => Some(GtkWaylandDisplay { tx }),
            Err(e) => {
                log::error!("Failed to spawn GTK thread: {e}");
                None
            }
        }
    }
}
