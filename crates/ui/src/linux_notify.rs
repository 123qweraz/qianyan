use crate::{CandidateDisplay, DisplayCandidate};
use notify_rust::{Hint, Notification, NotificationHandle};
use qianyan_ime_core::Config;

/// 把“上屏的文本”渲染成系统通知气泡的后端（linux.show_notification 时启用）。
///
/// 不在每个按键时弹通知（太吵），只在文本真正上屏（commit）时弹一条，
/// 短暂显示后自动消失。
pub struct LinuxNotifyDisplay {
    active_notification: Option<NotificationHandle>,
    config: Config,
}

impl LinuxNotifyDisplay {
    pub fn new(config: Config) -> Self {
        Self {
            active_notification: None,
            config,
        }
    }

    /// 一条上屏文本 → 一条通知。
    pub fn on_commit(&mut self, text: &str) {
        #[cfg(target_os = "linux")]
        let show = self.config.linux.show_notification;
        #[cfg(not(target_os = "linux"))]
        let show = false;
        if !show || text.is_empty() {
            return;
        }
        let _ = self.active_notification.take().map(|h| h.close());
        let result = Notification::new()
            .summary(text)
            .body("")
            .appname("qianyan-ime")
            .hint(Hint::Transient(true))
            .timeout(2000)
            .show();
        match result {
            Ok(h) => {
                self.active_notification = Some(h);
            }
            Err(e) => {
                log::debug!("[NOTIFY_DEBUG] notification FAILED: {:?}", e);
            }
        }
    }
}

impl CandidateDisplay for LinuxNotifyDisplay {
    fn update_candidates(
        &mut self,
        _pinyin: &str,
        _candidates: Vec<DisplayCandidate>,
        _selected: usize,
        _page: usize,
        _total_pages: usize,
    ) {
        // 候选是实时滚动的，不在这里弹气泡，只在 on_commit 时弹一条。
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
            if let Some(h) = self.active_notification.take() {
                h.close();
            }
        }
    }

    fn apply_config(&mut self, config: &Config) {
        self.config = config.clone();
    }

    fn close(&mut self) {
        if let Some(h) = self.active_notification.take() {
            h.close();
        }
    }
}