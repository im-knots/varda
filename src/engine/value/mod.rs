//! Engine-owned value vocabulary — plain data types shared between the engine
//! contract layer and the domain/implementation modules (`renderer`, `surface`,
//! `video`, …) that operate on them.
//!
//! This module is a **leaf**: it depends on nothing in `internal`. The
//! `internal` modules `pub use` types from here to keep their existing call
//! paths (e.g. the renderer config module's tonemap-mode type, the surface
//! module's path type) working, while `engine/{mod,types,traits}.rs` name
//! types from here directly rather than reaching into `internal`.
//!
//! See /spec/engine-value-types.md
pub mod detect;
pub mod dome;
pub mod render;
pub mod surface;
pub mod video;
pub mod warp;
