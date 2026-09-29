//! egui delivery layer.
//!
//! Declares the view-model modules and re-exports their types at `usecases::ui::*`.

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

pub(crate) use snapshot::{LutCatalog, PreviewTextures, build_ui_data};

pub use actions::*;
pub use data::*;
pub use session::*;
pub use state::*;

pub use crate::app::{DEFAULT_RENDER_HEIGHT, DEFAULT_RENDER_WIDTH};
