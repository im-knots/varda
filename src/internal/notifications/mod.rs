use std::time::{Duration, Instant};

/// Severity levels for notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

impl NotificationLevel {
    pub fn label(&self) -> &'static str {
        match self {
            NotificationLevel::Info => "INFO",
            NotificationLevel::Warning => "WARN",
            NotificationLevel::Error => "ERROR",
        }
    }
}

/// A single notification message
#[derive(Debug, Clone)]
pub struct Notification {
    pub level: NotificationLevel,
    pub message: String,
    pub created_at: Instant,
    pub duration: Duration,
}

impl Notification {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    /// Progress from 0.0 (just appeared) to 1.0 (about to expire)
    pub fn progress(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        (elapsed / total).min(1.0)
    }
}

/// Non-modal notification system for displaying messages to the user
pub struct NotificationSystem {
    /// Active notifications (newest first)
    active: Vec<Notification>,
    /// History of all notifications (for log panel)
    history: Vec<Notification>,
    /// Maximum number of visible notifications
    max_visible: usize,
}

impl NotificationSystem {
    pub fn new() -> Self {
        Self {
            active: Vec::new(),
            history: Vec::new(),
            max_visible: 5,
        }
    }

    /// Push a notification with default duration based on severity
    pub fn notify(&mut self, level: NotificationLevel, message: impl Into<String>) {
        let duration = match level {
            NotificationLevel::Info => Duration::from_secs(3),
            NotificationLevel::Warning => Duration::from_secs(5),
            NotificationLevel::Error => Duration::from_secs(8),
        };

        let notification = Notification {
            level,
            message: message.into(),
            created_at: Instant::now(),
            duration,
        };

        log::log!(
            match level {
                NotificationLevel::Info => log::Level::Info,
                NotificationLevel::Warning => log::Level::Warn,
                NotificationLevel::Error => log::Level::Error,
            },
            "[Notification] {}",
            notification.message
        );

        self.history.push(notification.clone());
        if self.history.len() > 1000 {
            self.history.drain(..self.history.len() - 1000);
        }
        self.active.insert(0, notification);
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.notify(NotificationLevel::Info, message);
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.notify(NotificationLevel::Warning, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.notify(NotificationLevel::Error, message);
    }

    /// Remove expired notifications
    pub fn update(&mut self) {
        self.active.retain(|n| !n.is_expired());
    }

    /// Get active notifications (up to max_visible)
    pub fn visible(&self) -> &[Notification] {
        let end = self.active.len().min(self.max_visible);
        &self.active[..end]
    }

    /// Dismiss a notification by index
    pub fn dismiss(&mut self, index: usize) {
        if index < self.active.len() {
            self.active.remove(index);
        }
    }
}

impl Default for NotificationSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_system_new_empty() {
        let ns = NotificationSystem::new();
        assert!(ns.visible().is_empty());
    }

    #[test]
    fn notification_system_notify_adds() {
        let mut ns = NotificationSystem::new();
        ns.notify(NotificationLevel::Info, "Test");
        assert_eq!(ns.visible().len(), 1);
        assert_eq!(ns.visible()[0].message, "Test");
    }

    #[test]
    fn notification_system_info_warn_error() {
        let mut ns = NotificationSystem::new();
        ns.info("Info");
        ns.warn("Warning");
        ns.error("Error");
        assert_eq!(ns.visible().len(), 3);
    }

    #[test]
    fn notification_system_dismiss() {
        let mut ns = NotificationSystem::new();
        ns.info("A");
        ns.info("B");
        ns.info("C");
        assert_eq!(ns.visible().len(), 3);
        ns.dismiss(0);
        assert_eq!(ns.visible().len(), 2);
    }

    #[test]
    fn notification_system_dismiss_out_of_bounds() {
        let mut ns = NotificationSystem::new();
        ns.info("A");
        ns.dismiss(10);
        assert_eq!(ns.visible().len(), 1);
    }

    #[test]
    fn notification_system_max_visible() {
        let mut ns = NotificationSystem::new();
        for i in 0..10 {
            ns.info(format!("Msg {}", i));
        }
        assert!(ns.visible().len() <= 5);
    }

    #[test]
    fn notification_newest_first() {
        let mut ns = NotificationSystem::new();
        ns.info("First");
        ns.info("Second");
        assert_eq!(ns.visible()[0].message, "Second");
    }

    #[test]
    fn notification_progress() {
        let n = Notification {
            level: NotificationLevel::Info,
            message: "Test".into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(10),
        };
        let p = n.progress();
        assert!((0.0..=1.0).contains(&p));
        assert!(p < 0.1);
    }

    #[test]
    fn notification_is_expired() {
        let n = Notification {
            level: NotificationLevel::Info,
            message: "Test".into(),
            created_at: Instant::now() - Duration::from_secs(100),
            duration: Duration::from_secs(3),
        };
        assert!(n.is_expired());
    }

    #[test]
    fn notification_not_expired() {
        let n = Notification {
            level: NotificationLevel::Info,
            message: "Test".into(),
            created_at: Instant::now(),
            duration: Duration::from_secs(100),
        };
        assert!(!n.is_expired());
    }

    #[test]
    fn notification_update_removes_expired() {
        let mut ns = NotificationSystem::new();
        ns.notify(NotificationLevel::Info, "Fresh");
        let _expired = Notification {
            level: NotificationLevel::Error,
            message: "Old".into(),
            created_at: Instant::now() - Duration::from_secs(100),
            duration: Duration::from_secs(1),
        };
        ns.update();
        assert_eq!(ns.visible().len(), 1);
        assert_eq!(ns.visible()[0].message, "Fresh");
    }

    #[test]
    fn notification_level_label() {
        assert_eq!(NotificationLevel::Info.label(), "INFO");
        assert_eq!(NotificationLevel::Warning.label(), "WARN");
        assert_eq!(NotificationLevel::Error.label(), "ERROR");
    }

    #[test]
    fn notification_system_default() {
        let ns = NotificationSystem::default();
        assert!(ns.visible().is_empty());
    }

    #[test]
    fn notification_durations_by_level() {
        let mut ns = NotificationSystem::new();
        ns.info("i");
        assert_eq!(ns.visible()[0].duration, Duration::from_secs(3));

        let mut ns2 = NotificationSystem::new();
        ns2.warn("w");
        assert_eq!(ns2.visible()[0].duration, Duration::from_secs(5));

        let mut ns3 = NotificationSystem::new();
        ns3.error("e");
        assert_eq!(ns3.visible()[0].duration, Duration::from_secs(8));
    }

    // ── Offensive: history must be capped at 1000 ────────────────────

    #[test]
    fn notification_history_capped_at_1000() {
        let mut ns = NotificationSystem::new();
        for i in 0..1100 {
            ns.info(format!("msg {}", i));
        }
        assert!(
            ns.history.len() <= 1000,
            "history should be capped at 1000, got {}",
            ns.history.len()
        );
        // Oldest messages should have been evicted
        assert!(
            ns.history[0].message != "msg 0",
            "oldest message should have been evicted"
        );
    }

    #[test]
    fn notification_history_at_boundary() {
        let mut ns = NotificationSystem::new();
        for i in 0..1000 {
            ns.info(format!("msg {}", i));
        }
        assert_eq!(ns.history.len(), 1000);
        // Adding one more should still cap at 1000
        ns.info("overflow");
        assert_eq!(ns.history.len(), 1000);
        assert_eq!(ns.history.last().unwrap().message, "overflow");
    }
}
