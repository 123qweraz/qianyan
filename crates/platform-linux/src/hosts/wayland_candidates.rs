//! Wayland 输入法宿主的候选框内容模型与 Slint 离屏渲染桥接。
//!
//! v1/v2 宿主都通过 `render_slint` 把候选内容交给 `qianyan_ime_ui::offscreen`
//! 渲染成 wl_shm 像素，视觉定义统一在 `candidate.slint`；Wayland 对象
//! （layer-shell / input-popup）仍由宿主持有。

use qianyan_ime_core::Config;

#[derive(Clone)]
pub struct CandItem {
    pub label: String,
    pub text: String,
    pub hint: String,
    pub is_fuzzy: bool,
}

#[derive(Clone, Default)]
pub struct CandContent {
    pub pinyin: String,
    pub candidates: Vec<CandItem>,
    pub selected: usize,
}

/// 用 Slint 离屏渲染候选框，与 X11/Windows 的 Slint 窗口共用同一份
/// `candidate.slint` 视觉定义。
pub fn render_slint(config: &Config, content: &CandContent) -> Option<(Vec<u8>, i32, i32)> {
    let candidates: Vec<qianyan_ime_ui::DisplayCandidate> = content
        .candidates
        .iter()
        .map(|c| qianyan_ime_ui::DisplayCandidate {
            text: c.text.clone(),
            label: c.label.clone(),
            hint: c.hint.clone(),
            full_display: String::new(),
            is_fuzzy: c.is_fuzzy,
        })
        .collect();
    qianyan_ime_ui::offscreen::render_candidates(
        config,
        &content.pinyin,
        &candidates,
        content.selected,
        0,
        0,
    )
}
