//! Engine state mutation methods on VardaApp.
//!
//! These methods encapsulate all mixer/modulation/sequence mutations.
//! They access self.mixer internally — callers never need &mut Mixer.
//!
//! Split into focused sub-modules:
//! - `video` — playback and auto-transition actions
//! - `params` — parameter value updates (generator, effect, master effect)
//! - `modulation` — modulation source CRUD and assignment
//! - `decks` — deck/effect add/remove/move and transition sequences

mod video;
mod params;
mod modulation;
mod decks;
