//! UI-specific notification helpers.
//!
//! Core notification types live in `crate::notifications`.
//! This module re-exports them and adds UI-only functionality (colors).

pub use crate::notifications::{Notification, NotificationLevel, NotificationSystem};

/// Get the accent color for a notification level (UI concern — egui Color32)
pub fn notification_color(level: NotificationLevel) -> egui::Color32 {
    match level {
        NotificationLevel::Info => egui::Color32::from_rgb(100, 160, 255), // Blue
        NotificationLevel::Warning => egui::Color32::from_rgb(255, 180, 50), // Orange
        NotificationLevel::Error => egui::Color32::from_rgb(255, 80, 80),  // Red
    }
}
