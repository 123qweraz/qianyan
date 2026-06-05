use crate::{DisplayCandidate, GuiEvent};
use notify_rust::{Hint, Notification, NotificationHandle};
use qianyan_ime_core::Config;

/// 在主进程中运行的系统通知处理器。
/// 接收 GuiEvent 并选择性地发出桌面通知（受 config.linux.show_notification 等控制）。
/// GUI 子进程不再包含通知逻辑。
pub struct LocalNotify {
    active_notification: Option<NotificationHandle>,
    config: Config,
    last_content: String,
}

impl LocalNotify {
    pub fn new(initial_config: &Config) -> Self {
        Self {
            active_notification: None,
            config: initial_config.clone(),
            last_content: String::new(),
        }
    }

    pub fn handle(&mut self, event: &GuiEvent) {
        match event {
            GuiEvent::Update {
                pinyin,
                candidates,
                selected,
                ..
            } => self.update_candidates(pinyin, candidates, *selected),
            GuiEvent::ShowStatus(text, chinese_enabled) => {
                self.update_status(text, *chinese_enabled)
            }
            GuiEvent::SetVisible(visible) => {
                if !visible {
                    self.close_active();
                }
            }
            GuiEvent::ApplyConfig(cfg) => {
                self.config = (**cfg).clone();
            }
            GuiEvent::Exit => self.close_active(),
            _ => {}
        }
    }

    fn update_candidates(
        &mut self,
        pinyin: &str,
        candidates: &[DisplayCandidate],
        selected: usize,
    ) {
        if !self.config.linux.show_notification {
            return;
        }
        if pinyin.is_empty() {
            self.close_active();
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
            return;
        }
        self.last_content = current_content;

        if let Some(ref mut h) = self.active_notification {
            h.summary(pinyin);
            h.body(&notify_body);
            h.hint(Hint::Transient(true));
            h.hint(Hint::Custom(
                "x-canonical-private-synchronous".to_string(),
                "true".to_string(),
            ));
            if let Err(e) = h.update() {
                log::debug!("[LocalNotify] update failed: {:?}", e);
            }
        } else {
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
                Ok(h) => self.active_notification = Some(h),
                Err(e) => log::debug!("[LocalNotify] notification FAILED: {:?}", e),
            }
        }
    }

    fn update_status(&mut self, text: &str, chinese_enabled: bool) {
        if !self.config.linux.show_toggle_notification || text.is_empty() {
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

    fn close_active(&mut self) {
        if let Some(h) = self.active_notification.take() {
            h.close();
        }
        self.last_content.clear();
    }
}
