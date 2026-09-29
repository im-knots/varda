//! Plain data types shared by the engine contract and the `internal` modules
//! that operate on them.
//!
//! This module depends on nothing in `internal`. `internal` modules re-export
//! types from here; `engine/{mod,types,traits}.rs` name them directly.
pub mod detect;
pub mod dome;
pub mod editor;
pub mod entity;
pub mod keymap;
pub mod midi;
pub mod notification;
pub mod param;
pub mod provider;
pub mod render;
pub mod source;
pub mod surface;
pub mod video;
pub mod warp;
