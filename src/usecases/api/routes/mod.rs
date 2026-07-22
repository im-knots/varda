//! Route handlers for the HTTP API.
//!
//! Each sub-module groups routes by domain (mixer, channels, decks, etc.).
//! Route handlers are thin: validate input, read state or send commands,
//! map results to HTTP responses.

pub mod audio;
pub mod channels;
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
