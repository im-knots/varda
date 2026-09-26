//! Notification values: how serious a message to the performer is.

/// Severity levels for notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}
