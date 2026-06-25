//! Engine state mutation methods on VardaApp.
//!
//! These methods encapsulate all mixer/modulation/sequence mutations.
//! They access self.mixer internally — callers never need &mut Mixer.
//!
//! Split into focused sub-modules:
//! - `video` — playback and auto-transition actions
//! - `params` — parameter value updates (generator, effect, master effect)
//! - `modulation` — modulation source CRUD and assignment
//! - `decks` — deck/effect add/remove/move
//! - `sequences` — transition sequence CRUD and step mutations
//! - `surfaces` — surface command state mutations
//! - `io` — external I/O deck creation and stream library mutations

mod decks;
mod io;
mod modulation;
mod outputs;
mod params;
mod sequences;
mod surfaces;
mod video;

pub(crate) use outputs::resolve_output_audio;
