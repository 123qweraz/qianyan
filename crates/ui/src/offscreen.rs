//! 离屏 Slint 渲染：把 `CandidateWindow` 光栅化为 wl_shm 可用的像素。
//!
//! 原生 Wayland 下 winit/Slint 无法给顶层窗口定位，因此候选框的位置由
//! Wayland 宿主（layer-shell / input-popup）负责。本模块只负责把候选窗
//! 渲染成像素，宿主再把像素贴到自己的 wl_shm 表面上，从而在原生 Wayland
//! 下也复用同一份 `candidate.slint` 视觉定义。
//!
//! 渲染器每个线程独立（Slint 的全局上下文是 thread-local），因此这里用
//! `thread_local!` 缓存组件与缓冲；Wayland 宿主在自己的线程内调用即可。

use crate::slint_window::{apply_candidate_style, CandidateData, CandidateWindow};
use crate::DisplayCandidate;
use qianyan_ime_core::Config;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static PLATFORM_SET: Cell<bool> = const { Cell::new(false) };
    static WINDOW: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
    static STATE: RefCell<Option<OffscreenState>> = const { RefCell::new(None) };
}

struct OffscreenPlatform;

impl Platform for OffscreenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        WINDOW.with(|c| *c.borrow_mut() = Some(window.clone()));
        Ok(window)
    }
}

struct OffscreenState {
    window: Rc<MinimalSoftwareWindow>,
    component: CandidateWindow,
    buffer: Vec<PremultipliedRgbaColor>,
}

fn init_state() -> Option<OffscreenState> {
    if !PLATFORM_SET.with(|c| c.get()) {
        slint::platform::set_platform(Box::new(OffscreenPlatform)).ok()?;
        PLATFORM_SET.with(|c| c.set(true));
    }
    let component = CandidateWindow::new().ok()?;
    let window = WINDOW.with(|c| c.borrow().clone())?;
    Some(OffscreenState {
        window,
        component,
        buffer: Vec::new(),
    })
}

fn with_state<R>(f: impl FnOnce(&mut OffscreenState) -> R) -> Option<R> {
    STATE.with(|cell| {
        let is_none = cell.borrow().is_none();
        if is_none {
            *cell.borrow_mut() = Some(init_state()?);
        }
        let mut slot = cell.borrow_mut();
        slot.as_mut().map(f)
    })
}

/// 把候选内容离屏渲染为 `wl_shm` 需要的 ARGB8888（小端即 BGRA，预乘）像素。
///
/// 返回 `(pixels, width, height)`；尺寸取 Slint 内容的紧凑首选尺寸（逻辑像素）。
pub fn render_candidates(
    config: &Config,
    pinyin: &str,
    candidates: &[DisplayCandidate],
    selected: usize,
    page: usize,
    total_pages: usize,
) -> Option<(Vec<u8>, i32, i32)> {
    with_state(|state| -> Option<(Vec<u8>, i32, i32)> {
        apply_candidate_style(&state.component, config);
        state.component.set_pinyin(SharedString::from(pinyin));
        state.component.set_selected_index(selected as i32);
        state.component.set_current_page(page as i32);
        state.component.set_total_pages(total_pages as i32);
        state.component.set_is_visible(true);

        let models: Vec<CandidateData> = candidates
            .iter()
            .map(|c| CandidateData {
                text: SharedString::from(&c.text),
                label: SharedString::from(&c.label),
                english_aux: SharedString::from(&c.hint),
                stroke_aux: SharedString::from(""),
                is_fuzzy: c.is_fuzzy,
            })
            .collect();
        state
            .component
            .set_candidates(ModelRc::from(Rc::new(VecModel::from(models))));

        let logical_w = state.component.get_content_width();
        let logical_h = state.component.get_content_height();
        let width = logical_w.round().max(1.0) as i32;
        let height = logical_h.round().max(1.0) as i32;

        state
            .window
            .set_size(slint::LogicalSize::new(logical_w, logical_h));
        state.window.request_redraw();

        let needed = width as usize * height as usize;
        if state.buffer.len() < needed {
            state.buffer.resize(needed, PremultipliedRgbaColor::default());
        }

        let rendered = {
            let buffer = &mut state.buffer[..needed];
            state
                .window
                .draw_if_needed(|renderer| {
                    renderer.render(buffer, width as usize);
                })
        };
        if !rendered {
            return None;
        }

        let mut pixels = Vec::with_capacity(needed * 4);
        for px in &state.buffer[..needed] {
            pixels.extend_from_slice(&[px.blue, px.green, px.red, px.alpha]);
        }
        Some((pixels, width, height))
    })
    .flatten()
}
