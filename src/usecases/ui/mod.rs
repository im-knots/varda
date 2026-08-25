//! egui delivery layer.
//!
//! Thin orchestrator: declares the view-model modules and re-exports their types
//! at `usecases::ui::*`, so panels keep importing from one place. The split is by
//! concern — layout/selection state, the per-frame view model, session state, and
//! the outbound action bucket. See /spec/ui-engine-boundary.md.

mod actions;
mod data;
pub(crate) mod keyboard;
pub mod notifications;
pub mod panels;
pub mod runner;
mod session;
mod snapshot;
mod state;
pub mod widgets;

#[cfg(any(test, feature = "test-fixtures"))]
mod fixtures;

pub(crate) use snapshot::build_ui_data;

pub use actions::*;
pub use data::*;
pub use session::*;
pub use state::*;

// Re-export default render resolution constants from the engine layer
pub use crate::app::{DEFAULT_RENDER_HEIGHT, DEFAULT_RENDER_WIDTH};
