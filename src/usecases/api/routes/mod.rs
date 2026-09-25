//! Route handlers for the HTTP API.
//!
//! Each sub-module groups routes by domain (mixer, channels, decks, etc.).
//! Route handlers are thin: validate input, read state or send commands,
//! map results to HTTP responses.

pub mod arrangement;
pub mod audio;
pub mod channels;
pub mod clipboard;
pub mod decks;
pub mod effects;
pub mod library;
pub mod macros;
pub mod mixer;
pub mod modulation;
pub mod outputs;
pub mod scene;
pub mod sequences;
pub mod stage;
pub mod state;
pub mod surfaces;
pub mod system;
#[cfg(test)]
mod tests;
pub mod timecode;
pub mod transport;

use crate::app::publish::PublishedState;
use crate::usecases::api::SharedState;
use crate::usecases::api::projection::{self, StateReadError};
use axum::http::StatusCode;
use std::sync::Arc;

/// The published snapshot, or the HTTP error for its absence.
pub(crate) fn read_or_error(
    state: &SharedState,
) -> Result<Arc<PublishedState>, (StatusCode, &'static str)> {
    projection::read_state(&state.engine_state).map_err(|e| match e {
        StateReadError::NotInitialized => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Engine not yet initialized",
        ),
    })
}
