use crate::{CandidateDisplay, DisplayCandidate};
use notify_rust::{Hint, Notification, NotificationHandle};
use qianyan_ime_core::Config;

/// “系统通知候选框”显示后端（linux.show_notification 时启用）。
///
/// 忠实恢复“未被误删”的旧版行为：在通知气泡里实时显示候选词列表，
/// 【】标出当前选中项；候选清空（上屏）时气泡自动关闭消失。
///
/// 针对 Plasma/KDE 修正原版两个问题：
/// 1. 原版用 `x-canonical-private-synchronous`（仅 GNOME 三件套支持）做
///    原位替换，update 失败时直接新建却不关闭旧气泡 → 每打一个字母就叠
///    一条通知。这里改为“更新失败/重新创建前先 close 旧气泡”，保证同时
///    最多只有一条候选气泡。
/// 2. 原版 timeout(0) 永不消失；改为 2s 自动消失，停顿/上屏后气泡自己
///    收起，不再挂壁。
pub struct LinuxNotifyDisplay {
    active_notification: Option<NotificationHandle>,
    config: Config,
    last_content: String,
}

impl LinuxNotifyDisplay {
    pub fn new(config: Config) -> Self {
        Self {
            active_notification: None,
            config,
            last_content: String::new(),
        }
    }

    fn close_bubble(&mut self) {
        if let Some(h) = self.active_notification.take() {
            h.close();
        }
    }
}

impl CandidateDisplay for LinuxNotifyDisplay {
    fn update_candidates(
        &mut self,
        pinyin: &str,
        candidates: Vec<DisplayCandidate>,
        selected: usize,
        _page: usize,
        _total_pages: usize,
    ) {
        #[cfg(target_os = "linux")]
        let show = self.config.linux.show_notification;
        #[cfg(not(target_os = "linux"))]
        let show = false;
        if !show {
            return;
        }
        log::debug!(
            "[NOTIFY_DEBUG] update_candidates: pinyin='{}' candidates={}",
            pinyin,
            candidates.len()
        );
        if pinyin.is_empty() {
            // 上屏 / 清空 → 候选框自己消失
            self.close_bubble();
            self.last_content.clear();
            return;
        }

        let mut notify_body = String::new();
        for (i, c) in candidates.iter().enumerate() {
            if i == selected {
                notify_body.push_str(&format!("【{}】 ", c.full_display));
            } else {
                notify_body.push_str(&format!("{} ", c.full_display));
            }
        }

        let current_content = format!("{}:{}", pinyin, notify_body);
        if current_content == self.last_content {
            log::debug!("[NOTIFY_DEBUG] content unchanged, skipping");
            return;
        }
        self.last_content = current_content;

        // 优先原位更新，避免在支持替换的桌面（GNOME）上反复弹窗。
        let updated = if let Some(h) = self.active_notification.as_mut() {
            h.summary(pinyin);
            h.body(&notify_body);
            h.hint(Hint::Transient(true));
            h.hint(Hint::Custom(
                "x-canonical-private-synchronous".to_string(),
                "true".to_string(),
            ));
            h.update().is_ok()
        } else {
            false
        };

        if !updated {
            // 原位更新失败（Plasma 等）：先收掉旧气泡，再弹一条新的，
            // 保证任一瞬间最多一条候选通知，不会每打一个字母叠一条。
            self.close_bubble();
            let result = Notification::new()
                .summary(pinyin)
                .body(&notify_body)
                .appname("qianyan-ime")
                .hint(Hint::Transient(true))
                .hint(Hint::Custom(
                    "x-canonical-private-synchronous".to_string(),
                    "true".to_string(),
                ))
                .timeout(2000)
                .show();
            match result {
                Ok(h) => {
                    log::debug!("[NOTIFY_DEBUG] notification shown successfully");
                    self.active_notification = Some(h);
                }
                Err(e) => {
                    log::debug!("[NOTIFY_DEBUG] notification FAILED: {:?}", e);
                }
            }
        }
    }

    fn update_status(&mut self, text: &str, chinese_enabled: bool) {
        #[cfg(target_os = "linux")]
        let show = self.config.linux.show_toggle_notification;
        #[cfg(not(target_os = "linux"))]
        let show = false;
        if !show || text.is_empty() {
            return;
        }
        let _ = Notification::new()
            .summary(if chinese_enabled { "中" } else { "英" })
            .body("")
            .appname("qianyan-ime")
            .hint(Hint::Transient(true))
            .timeout(1500)
            .show();
    }

    fn move_to(&mut self, _x: i32, _y: i32) {}

    fn set_visible(&mut self, visible: bool) {
        if !visible {
            self.close_bubble();
            self.last_content.clear();
        }
    }

    fn apply_config(&mut self, config: &Config) {
        self.config = config.clone();
    }

    fn close(&mut self) {
        self.close_bubble();
    }
}