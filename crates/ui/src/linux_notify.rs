use crate::{CandidateDisplay, DisplayCandidate};
use notify_rust::{Hint, Notification, NotificationHandle};
use qianyan_ime_core::Config;

pub struct LinuxNotifyDisplay {
    active_notification: Option<NotificationHandle>,
    config: Config,
    last_content: String, // 缓存内容，避免重复发送完全相同的内容
}

impl LinuxNotifyDisplay {
    pub fn new(config: Config) -> Self {
        Self {
            active_notification: None,
            config,
            last_content: String::new(),
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
        log::debug!("[NOTIFY_DEBUG] update_candidates: pinyin='{}' candidates={}", pinyin, candidates.len());
        if pinyin.is_empty() {
            if let Some(h) = self.active_notification.take() {
                h.close();
            }
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

        if let Some(ref mut h) = self.active_notification {
            log::debug!("[NOTIFY_DEBUG] updating existing notification");
            h.summary(pinyin);
            h.body(&notify_body);
            h.hint(Hint::Transient(true));
            h.hint(Hint::Custom(
                "x-canonical-private-synchronous".to_string(),
                "true".to_string(),
            ));
            if let Err(e) = h.update() {
                log::debug!("[NOTIFY_DEBUG] update failed: {:?}", e);
            }
        } else {
            log::debug!("[NOTIFY_DEBUG] creating new notification");
            let result = Notification::new()
                .summary(pinyin)
                .body(&notify_body)
                .appname("qianyan-ime")
                .hint(Hint::Transient(true))
                .hint(Hint::Custom(
                    "x-canonical-private-synchronous".to_string(),
                    "true".to_string(),
                ))
                .timeout(0)
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
            if let Some(h) = self.active_notification.take() {
                h.close();
            }
            self.last_content.clear();
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
