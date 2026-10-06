//! The fractal explorer's formula stack in `f64`: formulas, the schedule of a
//! six-slot hybrid, and the distance estimate at a point.
//!
//! The shader `shaders/fractal_explorer.fs` renders the same stack on the GPU.
//! This host copy serves the flight preprocessor (distance at the camera,
//! autopilot probes, autofocus) and tests that hold the shader to the same
//! math.

mod camera;
mod formula;
mod navigation;
mod params;
mod resolution;
mod stack;
mod vec3;

pub use camera::{Controls, Flight, FlightMode, Pose, Quat, StepReport};
pub use formula::{FormulaId, Slot};
pub use navigation::{Autopilot, Inside, Location, Recall, RecallFrame, Steering, find_inside};
pub use params::{SLOT_FIELDS, STACK_PARAMS, default_param, slot_param, stack_from_params};
pub use resolution::Governor;
pub use stack::{CombineOp, DistanceKind, HybridMode, SLOTS, Sample, Schedule, Stack};
pub use vec3::{Mat3, Vec3};
