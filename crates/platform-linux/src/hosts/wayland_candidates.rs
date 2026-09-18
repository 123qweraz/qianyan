//! 在 Wayland 输入法宿主的私有连接上自绘候选框（wl_shm + layer-shell）。
//!
//! v1 输入法协议拿不到光标矩形，因此候选框固定锚定在配置的角落；
//! 这里只做纯粹的 pango/cairo 光栅化，Wayland 对象由宿主持有。

use cairo::{Context, Format, ImageSurface};
use pango::{FontDescription, Layout, Weight, SCALE as PANGO_SCALE};
use qianyan_ime_core::Config;

const BORDER_W: f64 = 1.0;
const RADIUS: f64 = 4.0;
const OUTER_H: f64 = 8.0;
const OUTER_V: f64 = 4.0;
const ROW_H: f64 = 8.0;
const ROW_V: f64 = 2.0;
const LABEL_GAP: f64 = 4.0;
const CHUNK_GAP: f64 = 4.0;

#[derive(Clone)]
pub struct CandStyle {
    pub bg: (f64, f64, f64, f64),
    pub border: (f64, f64, f64, f64),
    pub text: (f64, f64, f64, f64),
    pub hint: (f64, f64, f64, f64),
    pub highlight: (f64, f64, f64, f64),
    pub highlight_text: (f64, f64, f64, f64),
    pub font_family: String,
    pub pinyin_size: f64,
    pub pinyin_weight: u32,
    pub candidate_size: f64,
    pub candidate_weight: u32,
    pub horizontal: bool,
}

impl CandStyle {
    pub fn from_config(c: &Config) -> Self {
        let a = &c.appearance;
        Self {
            bg: rgba(&a.window_bg_color),
            border: rgba(&a.window_border_color),
            text: rgba(&a.candidate_text.color),
            hint: rgba(&a.hint_text.color),
            highlight: rgba(&a.window_highlight_color),
            highlight_text: rgba(&a.window_highlight_text_color),
            font_family: first_family(&a.candidate_text.font_family),
            pinyin_size: a.pinyin_text.font_size.max(1) as f64,
            pinyin_weight: a.pinyin_text.font_weight,
            candidate_size: a.candidate_text.font_size.max(1) as f64,
            candidate_weight: a.candidate_text.font_weight,
            horizontal: a.candidate_layout == "horizontal",
        }
    }
}

#[derive(Clone)]
pub struct CandItem {
    pub label: String,
    pub text: String,
    pub hint: String,
}

#[derive(Clone, Default)]
pub struct CandContent {
    pub pinyin: String,
    pub candidates: Vec<CandItem>,
    pub selected: usize,
}

/// 光栅化为 `wl_shm` 需要的 ARGB8888（小端即 BGRA，预乘）像素。
pub fn render(style: &CandStyle, content: &CandContent) -> Option<(Vec<u8>, i32, i32)> {
    let scratch_surf = ImageSurface::create(Format::ARgb32, 1, 1).ok()?;
    let scratch_cr = Context::new(&scratch_surf).ok()?;
    let scratch_pctx = pangocairo::functions::create_context(&scratch_cr);

    let pinyin = make_layout(
        &scratch_pctx,
        &content.pinyin,
        &style.font_family,
        style.pinyin_size,
        style.pinyin_weight,
    );
    let (pw, ph) = pinyin.pixel_size();

    struct Chunk {
        label: Layout,
        text: Layout,
        hint: Option<Layout>,
        w: f64,
        h: f64,
    }
    let mut chunks = Vec::with_capacity(content.candidates.len());
    for item in &content.candidates {
        let label = make_layout(
            &scratch_pctx,
            item.label.trim_end_matches('.'),
            &style.font_family,
            style.candidate_size,
            style.candidate_weight,
        );
        let text = make_layout(
            &scratch_pctx,
            &item.text,
            &style.font_family,
            style.candidate_size,
            style.candidate_weight,
        );
        let (lw, lh) = label.pixel_size();
        let (tw, th) = text.pixel_size();
        let mut w = lw as f64 + LABEL_GAP + tw as f64;
        let mut h = lh.max(th) as f64;
        let hint = if item.hint.is_empty() {
            None
        } else {
            let lay = make_layout(
                &scratch_pctx,
                &item.hint,
                &style.font_family,
                style.candidate_size,
                style.candidate_weight,
            );
            let (hw, hh) = lay.pixel_size();
            w += CHUNK_GAP + hw as f64;
            h = h.max(hh as f64);
            Some(lay)
        };
        chunks.push(Chunk { label, text, hint, w, h });
    }

    let pinyin_cell_w = pw as f64 + OUTER_H * 2.0;
    let pinyin_cell_h = ph as f64 + OUTER_V * 2.0;

    let row_h: Vec<f64> = chunks.iter().map(|c| c.h + ROW_V * 2.0).collect();
    let row_w: Vec<f64> = chunks.iter().map(|c| c.w + ROW_H * 2.0).collect();

    let (cand_cell_w, cand_cell_h) = if style.horizontal {
        let w = row_w.iter().sum::<f64>() + CHUNK_GAP * (chunks.len().saturating_sub(1) as f64);
        let h = row_h.iter().cloned().fold(0.0, f64::max);
        (w, h)
    } else {
        let w = row_w.iter().cloned().fold(0.0, f64::max);
        let h = row_h.iter().sum::<f64>() + CHUNK_GAP * (chunks.len().saturating_sub(1) as f64);
        (w, h)
    };

    let content_w = pinyin_cell_w.max(cand_cell_w);
    let logical_w = content_w + BORDER_W * 2.0;
    let logical_h = pinyin_cell_h + cand_cell_h + BORDER_W * 2.0;

    let width = logical_w.round().max(1.0) as i32;
    let height = logical_h.round().max(1.0) as i32;

    let mut surface = ImageSurface::create(Format::ARgb32, width, height).ok()?;
    let cr = Context::new(&surface).ok()?;

    let (fg, fh) = (width as f64, height as f64);
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);

    rounded_rect(&cr, 0.5, 0.5, fg - 1.0, fh - 1.0, RADIUS);
    set_color(&cr, style.bg);
    let _ = cr.fill_preserve();
    set_color(&cr, style.border);
    cr.set_line_width(BORDER_W);
    let _ = cr.stroke();

    let pinyin = make_layout(
        &scratch_pctx,
        &content.pinyin,
        &style.font_family,
        style.pinyin_size,
        style.pinyin_weight,
    );
    set_color(&cr, style.text);
    cr.move_to(BORDER_W + OUTER_H, BORDER_W + OUTER_V);
    pangocairo::functions::show_layout(&cr, &pinyin);

    let draw_row = |cr: &Context,
                    x: f64,
                    y: f64,
                    row_w: f64,
                    row_h: f64,
                    chunk: &Chunk,
                    selected: bool| {
        if selected {
            rounded_rect(cr, x, y, row_w, row_h, RADIUS);
            set_color(cr, style.highlight);
            let _ = cr.fill();
        }
        let text_color = if selected { style.highlight_text } else { style.text };
        let hint_color = if selected { style.highlight_text } else { style.hint };
        let ty = y + (row_h - chunk.h) / 2.0;
        set_color(cr, text_color);
        let mut cx = x + ROW_H;
        cr.move_to(cx, ty);
        pangocairo::functions::show_layout(cr, &chunk.label);
        let (lw, _) = chunk.label.pixel_size();
        cx += lw as f64 + LABEL_GAP;
        cr.move_to(cx, ty);
        pangocairo::functions::show_layout(cr, &chunk.text);
        if let Some(hint) = &chunk.hint {
            let (tw, _) = chunk.text.pixel_size();
            cx += tw as f64 + CHUNK_GAP;
            set_color(cr, hint_color);
            cr.move_to(cx, ty);
            pangocairo::functions::show_layout(cr, hint);
        }
    };

    if style.horizontal {
        let mut x = BORDER_W;
        let y = BORDER_W + pinyin_cell_h;
        let h = row_h.iter().cloned().fold(0.0, f64::max);
        for (i, chunk) in chunks.iter().enumerate() {
            let w = row_w[i];
            draw_row(&cr, x, y, w, h, chunk, i == content.selected);
            x += w + CHUNK_GAP;
        }
    } else {
        let mut y = BORDER_W + pinyin_cell_h;
        for (i, chunk) in chunks.iter().enumerate() {
            let h = row_h[i];
            draw_row(&cr, BORDER_W, y, content_w, h, chunk, i == content.selected);
            y += h + CHUNK_GAP;
        }
    }

    let stride = surface.stride() as usize;
    let _ = surface.flush();
    let data = surface.data().ok()?;
    let row_bytes = (width as usize) * 4;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        let start = row * stride;
        pixels.extend_from_slice(&data[start..start + row_bytes]);
    }
    Some((pixels, width, height))
}

fn make_layout(
    pctx: &pango::Context,
    text: &str,
    family: &str,
    size: f64,
    weight: u32,
) -> Layout {
    let layout = Layout::new(pctx);
    let mut desc = FontDescription::from_string(family);
    desc.set_absolute_size(size * PANGO_SCALE as f64);
    desc.set_weight(weight_of(weight));
    layout.set_font_description(Some(&desc));
    layout.set_text(text);
    layout
}

fn weight_of(weight: u32) -> Weight {
    match weight {
        0..=349 => Weight::Light,
        350..=549 => Weight::Normal,
        550..=649 => Weight::Semibold,
        650..=749 => Weight::Bold,
        _ => Weight::Heavy,
    }
}

fn set_color(cr: &Context, c: (f64, f64, f64, f64)) {
    cr.set_source_rgba(c.0, c.1, c.2, c.3);
}

fn rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn rgba(s: &str) -> (f64, f64, f64, f64) {
    let (r, g, b, a) = parse_color(s);
    (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0)
}

fn parse_color(s: &str) -> (u8, u8, u8, u8) {
    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            6 => (
                u8::from_str_radix(&hex[0..2], 16).unwrap_or(255),
                u8::from_str_radix(&hex[2..4], 16).unwrap_or(255),
                u8::from_str_radix(&hex[4..6], 16).unwrap_or(255),
                255,
            ),
            8 => (
                u8::from_str_radix(&hex[0..2], 16).unwrap_or(255),
                u8::from_str_radix(&hex[2..4], 16).unwrap_or(255),
                u8::from_str_radix(&hex[4..6], 16).unwrap_or(255),
                u8::from_str_radix(&hex[6..8], 16).unwrap_or(255),
            ),
            _ => (255, 255, 255, 255),
        }
    } else if let Some(inner) = s.strip_prefix("rgba(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() == 4 {
            (
                parts[0].parse().unwrap_or(255),
                parts[1].parse().unwrap_or(255),
                parts[2].parse().unwrap_or(255),
                (parts[3].parse::<f32>().unwrap_or(1.0) * 255.0) as u8,
            )
        } else {
            (9, 105, 218, 255)
        }
    } else if let Some(inner) = s.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() == 3 {
            (
                parts[0].parse().unwrap_or(0),
                parts[1].parse().unwrap_or(0),
                parts[2].parse().unwrap_or(0),
                255,
            )
        } else {
            (9, 105, 218, 255)
        }
    } else {
        (9, 105, 218, 255)
    }
}

fn first_family(family: &str) -> String {
    family
        .split(',')
        .map(str::trim)
        .find(|f| !f.is_empty())
        .unwrap_or("Noto Sans CJK SC")
        .to_string()
}
