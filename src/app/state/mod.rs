//! Engine state mutation methods on VardaApp.
//!
//! These methods encapsulate all mixer/modulation/sequence mutations.
//! They access self.mixer internally — callers never need &mut Mixer.
//!
//! Split into focused sub-modules:
//! - `presets` — deck/channel preset load + save
//! - `sequences` — transition sequence CRUD and step mutations
//! - `surfaces` — surface command state mutations
//! - `io` — external I/O deck creation and stream library mutations

mod io;
mod outputs;
mod presets;
mod sequences;
mod surfaces;

pub(crate) use outputs::resolve_output_audio;
