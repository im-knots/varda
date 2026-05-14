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

mod video;
mod params;
mod modulation;
mod decks;
mod outputs;
mod sequences;
mod surfaces;
mod io;
