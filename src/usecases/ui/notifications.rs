//! UI-only notification helpers. Core types live in `crate::notifications`.

pub use crate::notifications::{Notification, NotificationLevel, NotificationSystem};

/// Accent color for a notification level.
pub fn notification_color(level: NotificationLevel) -> egui::Color32 {
    match level {
        NotificationLevel::Info => egui::Color32::from_rgb(100, 160, 255), // Blue
        NotificationLevel::Warning => egui::Color32::from_rgb(255, 180, 50), // Orange
        NotificationLevel::Error => egui::Color32::from_rgb(255, 80, 80),  // Red
    }
}
