//! Engine state mutation methods on `VardaApp`.
//!
//! These methods encapsulate all mixer/modulation/sequence mutations.
//! They access self.mixer internally — callers never need &mut Mixer.
//!
//! Split into focused sub-modules:
//! - `arrangement` — lane and region CRUD, authority, and live override
//! - `clipboard` — copy, paste, and duplicate of decks, channels, and effects
//! - `recorder` — capturing live parameter writes as automation curves
//! - `presets` — deck/channel preset load + save
//! - `sequences` — transition sequence CRUD and step mutations
//! - `surfaces` — surface command state mutations
//! - `io` — external I/O deck creation and stream library mutations

mod arrangement;
pub(crate) mod clipboard;
mod io;
mod outputs;
mod presets;
pub(crate) mod recorder;
mod sequences;
mod surfaces;

pub(crate) use outputs::{encoder_fps, resolve_output_audio};
