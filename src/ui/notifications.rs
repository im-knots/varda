use std::time::{Duration, Instant};

/// Severity levels for notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

impl NotificationLevel {
    /// Get the accent color for this level (egui Color32)
    pub fn color(&self) -> egui::Color32 {
        match self {
            NotificationLevel::Info => egui::Color32::from_rgb(100, 160, 255),    // Blue
            NotificationLevel::Warning => egui::Color32::from_rgb(255, 180, 50),  // Orange
            NotificationLevel::Error => egui::Color32::from_rgb(255, 80, 80),     // Red
        }
    }

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

