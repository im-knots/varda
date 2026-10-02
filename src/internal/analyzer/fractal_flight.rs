//! `fractal_flight`: the fractal explorer's camera, stepped once per frame.
//!
//! Reads the shader's flight controls and formula stack from its bound
//! parameters, flies the camera with speed set by the distance to the surface,
//! and publishes the camera, last frame's camera, the autofocus distance and
//! the slot rotations as one row of `rgba32float` texels.

use std::sync::Arc;

use crate::fractal::{
    Autopilot, Controls, DistanceKind, Flight, FlightMode, Location, Pose, Quat, Recall, Stack,
    Vec3, find_inside, resolution::Governor, stack_from_params,
};
use crate::params::ParamValue;

use super::traits::{
    AnalyzerSchema, AnalyzerSnapshot, AnalyzerStateSnapshot, HostFrame, HostInlinePreprocessor,
    ScalarOutputDef, TextureData, TextureOutputDef,
};

pub(crate) const PREPROCESSOR_TYPE: &str = "fractal_flight";

/// The output texture's name.
pub(crate) const TEXTURE: &str = "flight";

/// Bumped whenever the texel layout below changes. The shader checks it.
pub(crate) const LAYOUT_VERSION: f32 = 6.0;

/// The most time one frame advances the flight, in seconds.
const MAX_FLIGHT_STEP: f64 = 0.1;

/// Find Inside searches around the camera, this many distances to the
/// surface out, where the structure it is looking at is...
const INSIDE_REACH_FROM_CAMERA: f64 = 8.0;
/// ...and in balls of these radii around the origin: the default stack fits
/// in about 5.3, and a hybrid can fill a larger body with its open space
/// further out.
const INSIDE_RADII: [f64; 2] = [4.0, 16.0];
/// Find Inside's message when the stack has no rooms.
const NO_ROOMS: &str = "No enclosed space in this stack; showing the most open spot nearby.";

/// Texel layout, one `vec4` each:
///
/// | Texel | Content |
/// |---|---|
/// | 0 | camera position high part (f32), fov |
/// | 1 | camera position low part (f64 minus high), autofocus distance |
/// | 2, 3, 4 | camera right, up, forward |
/// | 5, 6 | last frame's position, high and low |
/// | 7, 8, 9 | last frame's right, up, forward |
/// | 10 | morph rate, distance at the camera, layout version, scene scale |
/// | 11 to 28 | each slot's rotation, three rows per slot |
/// | 29 | distance kind codes and Kleinian plane z: part 1 code, part 1 z, part 2 code, part 2 z |
/// | 30 | live render scale, last frame's live render scale |
///
/// The schedule is not here: the shader derives it from its specialized
/// structure inputs.
pub(crate) const ROTATION_TEXEL: usize = 11;
pub(crate) const KIND_TEXEL: usize = ROTATION_TEXEL + 3 * crate::fractal::stack::SLOTS;
pub(crate) const RESOLUTION_TEXEL: usize = KIND_TEXEL + 1;
pub(crate) const TEXELS: usize = RESOLUTION_TEXEL + 1;

/// A bound parameter's numeric value.
fn number(value: &ParamValue) -> Option<f64> {
    match value {
        ParamValue::Float(v) => Some(f64::from(*v)),
        ParamValue::Long(v) => Some(f64::from(*v)),
        ParamValue::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        ParamValue::Color(_) | ParamValue::Point2D(_) => None,
    }
}

/// Split an `f64` coordinate into an `f32` high part and the `f32` remainder.
fn split(v: f64) -> (f32, f32) {
    let high = v as f32;
    (high, (v - f64::from(high)) as f32)
}

/// A pose as saved: position and `[w, x, y, z]` orientation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SavedPose {
    position: [f64; 3],
    orientation: [f64; 4],
    /// Scene scale; absent in scenes saved before it existed.
    #[serde(default, skip_serializing_if = "is_zero")]
    scale: f64,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if passes a reference
fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

impl SavedPose {
    fn from_pose(pose: &Pose, scale: f64) -> Self {
        let q = pose.orientation;
        Self {
            position: [pose.position.x, pose.position.y, pose.position.z],
            orientation: [q.w, q.x, q.y, q.z],
            scale,
        }
    }

    fn to_pose(&self) -> Pose {
        let [x, y, z] = self.position;
        let [w, qx, qy, qz] = self.orientation;
        Pose {
            position: Vec3::new(x, y, z),
            orientation: Quat {
                w,
                x: qx,
                y: qy,
                z: qz,
            }
            .normalize(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SavedLocation {
    name: String,
    #[serde(flatten)]
    pose: SavedPose,
}

/// Saved state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Saved {
    version: u32,
    #[serde(flatten)]
    pose: SavedPose,
    #[serde(default)]
    locations: Vec<SavedLocation>,
}

pub(crate) struct FractalFlight {
    flight: Flight,
    /// Formula parameters last frame, for the morph rate.
    last_stack: Option<Stack>,
    locations: Vec<Location>,
    /// A flight to a location in progress.
    recall: Option<Recall>,
    /// The location the camera was last sent to.
    location_index: Option<usize>,
    /// Last frame's `location` parameter, to notice a change.
    last_location_param: Option<i64>,
    /// Last frame's `save_location`, to catch the rising edge.
    was_saving: bool,
    /// Last frame's `find_inside`, to catch the rising edge.
    was_finding: bool,
    /// A message for the performer, until taken.
    message: Option<String>,
    /// Seconds spent at the current tour stop.
    tour_clock: f64,
    autopilot: Autopilot,
    /// Last frame's Heading, Pitch and Roll sliders in radians. `None` until
    /// the first frame, so values already set when the flight starts do not
    /// turn the camera.
    last_look: Option<[f64; 3]>,
    /// Picks the render scale that holds `target_fps`.
    governor: Governor,
    /// Last frame's live render scale.
    last_live: Option<f64>,
    /// Last frame's look inputs, for the change rate.
    last_look_values: Option<Vec<f64>>,
}

impl FractalFlight {
    pub(crate) fn new() -> Self {
        Self {
            flight: Flight::new(Pose::default()),
            last_stack: None,
            locations: Vec::new(),
            recall: None,
            location_index: None,
            last_location_param: None,
            was_saving: false,
            was_finding: false,
            message: None,
            tour_clock: 0.0,
            autopilot: Autopilot::default(),
            last_look: None,
            governor: Governor::default(),
            last_live: None,
            last_look_values: None,
        }
    }

    /// Turn the camera by however far the Heading, Pitch and Roll sliders
    /// moved since last frame. The sliders cannot hold the orientation
    /// itself: the autopilot, locations and the speed sliders turn it too.
    fn look(&mut self, get: &impl Fn(&str) -> Option<f64>) {
        let look = ["look_heading", "look_pitch", "look_roll"]
            .map(|name| get(name).unwrap_or(0.0).to_radians());
        if let Some(last) = self.last_look {
            let [heading, pitch, roll] = [0, 1, 2].map(|i| look[i] - last[i]);
            if heading != 0.0 || pitch != 0.0 || roll != 0.0 {
                // Heading right and pitch up. The camera turns in its own
                // frame, where pitching positive turns the view down.
                let pose = &mut self.flight.pose;
                pose.orientation = pose
                    .orientation
                    .compose(Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), heading))
                    .compose(Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -pitch))
                    .compose(Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), roll))
                    .normalize();
            }
        }
        self.last_look = Some(look);
    }

    /// Where Find Inside looks: near what the camera sees, and around the
    /// origin.
    fn inside_balls(&self, stack: &Stack) -> Vec<(Vec3, f64)> {
        let camera = self.flight.pose.position;
        let distance = stack.sample(camera, 0.0).distance.max(1e-6);
        let mut balls = vec![(camera, INSIDE_REACH_FROM_CAMERA * distance)];
        balls.extend(INSIDE_RADII.map(|r| (Vec3::ZERO, r)));
        balls
    }

    fn start_recall(&mut self, index: usize, duration: f64) {
        if let Some(location) = self.locations.get(index) {
            self.recall = Some(Recall::new(
                (self.flight.pose, self.flight.scale),
                (location.pose, location.scale),
                duration,
            ));
            self.location_index = Some(index);
            self.tour_clock = 0.0;
        }
    }

    /// Saving, recalling and touring, from this frame's parameters.
    fn navigate(&mut self, get: &impl Fn(&str) -> Option<f64>, dt: f64) {
        let saving = get("save_location").is_some_and(|v| v > 0.5);
        if saving && !self.was_saving {
            self.locations.push(Location {
                name: format!("Location {}", self.locations.len() + 1),
                pose: self.flight.pose,
                scale: self.flight.scale,
            });
        }
        self.was_saving = saving;

        let recall_time = get("recall_time").unwrap_or(4.0).max(0.0);
        let wanted = get("location").map(|v| v.round() as i64);
        if wanted != self.last_location_param {
            if let (Some(index), Some(_)) = (wanted, self.last_location_param)
                && let Ok(index) = usize::try_from(index)
            {
                self.start_recall(index, recall_time);
            }
            self.last_location_param = wanted;
        }

        if get("tour").is_some_and(|v| v > 0.5) && !self.locations.is_empty() {
            self.tour_clock += dt;
            let stop = get("tour_seconds").unwrap_or(8.0).max(recall_time + 0.1);
            if self.tour_clock >= stop || self.location_index.is_none() {
                let next = self
                    .location_index
                    .map_or(0, |i| (i + 1) % self.locations.len());
                self.start_recall(next, recall_time);
            }
        }
    }
}

/// How much the formula changed since last frame, from 0 (still) to 1.
/// Inputs that change the lit image apart from the camera and the formula
/// slots: when they change, last frame's image is out of date even where the
/// camera holds still. Camera, motion, lens, grade and upscaler inputs act
/// through reprojection or after TAA, so they are not here.
const LOOK_INPUTS: [&str; 57] = [
    "look",
    "fov",
    "max_iterations",
    "bailout",
    "julia_mode",
    "julia_x",
    "julia_y",
    "julia_z",
    "combine_op",
    "combine_width",
    "detail",
    "lod",
    "max_steps",
    "step_mult",
    "sun_elev",
    "sun_azim",
    "sun_color",
    "sun_intensity",
    "shadow_softness",
    "shadow_length",
    "fill_elev",
    "fill_azim",
    "fill_color",
    "fill_intensity",
    "headlight_intensity",
    "headlight_color",
    "amb_top",
    "amb_bottom",
    "ao_strength",
    "bounce",
    "color_source",
    "palette_offset",
    "palette_scale",
    "color1",
    "color2",
    "color3",
    "color4",
    "roughness",
    "metallic",
    "specular",
    "emit_band",
    "emit_width",
    "emit_gain",
    "fog_density",
    "fog_color",
    "dyn_fog",
    "dyn_fog_color",
    "fog_on_iteration",
    "sky_brightness",
    "shafts",
    "shaft_anisotropy",
    "surface",
    "surface_mapping",
    "surface_scale",
    "surface_amount",
    "surface_bump",
    "surface_depth",
];

/// The look inputs as numbers, colors as their four channels.
fn look_values(state: &AnalyzerStateSnapshot) -> Vec<f64> {
    let mut values = Vec::with_capacity(LOOK_INPUTS.len() + 24);
    for name in LOOK_INPUTS {
        match state.values.get(name) {
            Some(ParamValue::Color(c)) => values.extend(c.iter().map(|v| f64::from(*v))),
            Some(value) => values.push(number(value).unwrap_or(0.0)),
            None => values.push(0.0),
        }
    }
    values
}

/// How fast the look is changing, 0 to 1, on the morph rate's scale.
fn look_rate(previous: Option<&[f64]>, current: &[f64]) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };
    let change = previous
        .iter()
        .zip(current)
        .map(|(x, y)| (x - y).abs() / (1.0 + x.abs()))
        .fold(0.0, f64::max);
    (change * 100.0).min(1.0) as f32
}

fn morph_rate(previous: Option<&Stack>, current: &Stack) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };
    let mut change: f64 = 0.0;
    for (a, b) in previous.slots.iter().zip(&current.slots) {
        if a.formula != b.formula || a.mode != b.mode || a.count != b.count {
            return 1.0;
        }
        for (x, y) in a.params.iter().zip(&b.params) {
            change = change.max((x - y).abs() / (1.0 + x.abs()));
        }
        for (ra, rb) in a.rotation.rows.iter().zip(&b.rotation.rows) {
            change = change.max((*ra - *rb).length());
        }
    }
    if previous.hybrid != current.hybrid
        || previous.repeat_from != current.repeat_from
        || previous.combine_split != current.combine_split
    {
        return 1.0;
    }
    // A 1% change per frame is already a fast morph.
    (change * 100.0).min(1.0) as f32
}

fn push_pose(texels: &mut Vec<[f32; 4]>, pose: &Pose, w_high: f32, w_low: f32) {
    let p = pose.position;
    let (xh, xl) = split(p.x);
    let (yh, yl) = split(p.y);
    let (zh, zl) = split(p.z);
    texels.push([xh, yh, zh, w_high]);
    texels.push([xl, yl, zl, w_low]);
    for axis in [pose.right(), pose.up(), pose.forward()] {
        texels.push([axis.x as f32, axis.y as f32, axis.z as f32, 0.0]);
    }
}

fn pack(
    flight: &Flight,
    fov: f32,
    morph: f32,
    distance: f64,
    stack: &Stack,
    [live, previous]: [f64; 2],
) -> Vec<u8> {
    let mut texels = Vec::with_capacity(TEXELS);
    push_pose(&mut texels, &flight.pose, fov, flight.focus as f32);
    push_pose(&mut texels, &flight.previous, 0.0, 0.0);
    texels.push([morph, distance as f32, LAYOUT_VERSION, flight.scale as f32]);
    texels.resize(TEXELS, [0.0; 4]);
    for (i, slot) in stack.slots.iter().enumerate() {
        for (r, row) in slot.rotation.rows.iter().enumerate() {
            texels[ROTATION_TEXEL + 3 * i + r] = [row.x as f32, row.y as f32, row.z as f32, 0.0];
        }
    }
    let [(code1, plane1), (code2, plane2)] = stack.distance_kinds().map(DistanceKind::code);
    texels[KIND_TEXEL] = [code1, plane1, code2, plane2];
    texels[RESOLUTION_TEXEL] = [live as f32, previous as f32, 0.0, 0.0];
    texels
        .iter()
        .flat_map(|t| t.iter().flat_map(|v| v.to_le_bytes()))
        .collect()
}

impl HostInlinePreprocessor for FractalFlight {
    fn output_schema(&self) -> AnalyzerSchema {
        let scalar = |name: &str, description: &str, range: (f32, f32)| ScalarOutputDef {
            name: name.into(),
            description: description.into(),
            range,
            default: 0.0,
            default_smoothing: 0.0,
        };
        AnalyzerSchema {
            scalars: vec![
                scalar(
                    "camera_distance",
                    "Distance from the camera to the nearest surface",
                    (0.0, 10.0),
                ),
                scalar("speed_actual", "World units per second flown", (0.0, 10.0)),
                scalar("focus_distance", "Autofocus distance", (0.0, 10.0)),
                scalar(
                    "scene_scale",
                    "The length fog and speed are measured in",
                    (0.0, 10.0),
                ),
                scalar(
                    "render_scale_live",
                    "The render scale this frame, set to hold Target FPS",
                    (0.0, 1.0),
                ),
                scalar("location_count", "Saved locations", (0.0, 64.0)),
                scalar(
                    "location_index",
                    "Location last flown to, or -1",
                    (-1.0, 64.0),
                ),
                scalar(
                    "autopilot_confidence",
                    "How open and detailed the autopilot's chosen way looks",
                    (0.0, 1.0),
                ),
            ],
            textures: vec![TextureOutputDef {
                name: TEXTURE.into(),
                description: "Camera, last frame's camera, focus and slot rotations".into(),
                format: "rgba32float".into(),
            }],
        }
    }

    fn init(&mut self, _options: &serde_json::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn step(&mut self, frame: &HostFrame<'_>) -> AnalyzerSnapshot {
        let get = |name: &str| frame.state.values.get(name).and_then(number);
        let stack = stack_from_params(get);
        let control = |name: &str| get(name).unwrap_or(0.0);
        let controls = Controls {
            throttle: control("throttle"),
            strafe_x: control("strafe_x"),
            strafe_y: control("strafe_y"),
            yaw_rate: control("yaw_rate"),
            pitch_rate: control("pitch_rate"),
            roll_rate: control("roll_rate"),
            speed: get("speed").unwrap_or(0.5),
            mode: if get("flight_mode").is_some_and(|v| v > 0.5) {
                FlightMode::Dive
            } else {
                FlightMode::Walk
            },
        };
        if get("reset_camera").is_some_and(|v| v > 0.5) {
            self.flight = Flight::new(Pose::default());
            self.recall = None;
        }
        let finding = get("find_inside").is_some_and(|v| v > 0.5);
        if finding
            && !self.was_finding
            && let Some(inside) = find_inside(&stack, &self.inside_balls(&stack))
        {
            self.flight = Flight::new(inside.pose);
            self.flight.scale = inside.scale;
            self.recall = None;
            if !inside.enclosed {
                self.message = Some(NO_ROOMS.to_owned());
            }
        }
        self.was_finding = finding;
        // Flight moves in wall-clock time, so a slow frame still covers its
        // share of a second. Capped so a hitch cannot throw the camera.
        let dt = f64::from(frame.frame_seconds).min(MAX_FLIGHT_STEP);
        // Last frame's pose, before the look sliders turn the camera: the
        // step below records the pose it starts from, which would already
        // include the turn, and TAA would reproject as if the camera had not
        // turned.
        let before = self.flight.pose;
        self.look(&get);
        self.navigate(&get, dt);

        let autopilot = get("autopilot").unwrap_or(0.0).clamp(0.0, 1.0);
        let mut confidence = 0.0;
        let report = if let Some(recall) = &mut self.recall {
            let frame = recall.step(dt);
            if frame.arrived {
                self.recall = None;
            }
            self.flight.scale = frame.scale;
            self.flight.place(&stack, frame.pose, dt)
        } else {
            let mut controls = controls;
            if autopilot > 0.0 {
                let distance = stack.sample(self.flight.pose.position, 0.0).distance;
                let steering = self.autopilot.step(&stack, &self.flight.pose, distance, dt);
                confidence = steering.confidence;
                controls.yaw_rate += (steering.yaw_rate - controls.yaw_rate) * autopilot;
                controls.pitch_rate += (steering.pitch_rate - controls.pitch_rate) * autopilot;
                controls.throttle += (1.0 - controls.throttle) * autopilot;
            }
            self.flight.step(&stack, &controls, dt)
        };
        self.flight.previous = before;
        // Last frame's image is out of date where the formula or the look
        // changed; TAA trusts its history less by this rate.
        let look = look_values(frame.state);
        let morph = morph_rate(self.last_stack.as_ref(), &stack)
            .max(look_rate(self.last_look_values.as_deref(), &look));
        self.last_look_values = Some(look);
        let fov = get("fov").unwrap_or(1.0) as f32;
        let live = self.governor.step(
            f64::from(frame.frame_seconds),
            get("target_fps").unwrap_or(0.0),
            get("render_scale").unwrap_or(1.0),
        );
        let previous = self.last_live.replace(live).unwrap_or(live);
        let data = pack(
            &self.flight,
            fov,
            morph,
            report.distance,
            &stack,
            [live, previous],
        );
        self.last_stack = Some(stack);

        let mut snapshot = AnalyzerSnapshot::from_defaults(&self.output_schema());
        snapshot
            .scalars
            .insert("camera_distance".into(), report.distance as f32);
        snapshot
            .scalars
            .insert("speed_actual".into(), report.speed as f32);
        snapshot
            .scalars
            .insert("focus_distance".into(), self.flight.focus as f32);
        snapshot
            .scalars
            .insert("scene_scale".into(), self.flight.scale as f32);
        snapshot
            .scalars
            .insert("render_scale_live".into(), live as f32);
        snapshot
            .scalars
            .insert("location_count".into(), self.locations.len() as f32);
        snapshot.scalars.insert(
            "location_index".into(),
            self.location_index.map_or(-1.0, |i| i as f32),
        );
        snapshot
            .scalars
            .insert("autopilot_confidence".into(), confidence as f32);
        snapshot.textures.insert(
            TEXTURE.into(),
            TextureData {
                generation: 0,
                width: TEXELS as u32,
                height: 1,
                format: "rgba32float".into(),
                data: Arc::from(data),
            },
        );
        snapshot
    }

    fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }

    fn persisted_state(&self) -> Option<serde_json::Value> {
        serde_json::to_value(Saved {
            version: 1,
            pose: SavedPose::from_pose(&self.flight.pose, self.flight.scale),
            locations: self
                .locations
                .iter()
                .map(|l| SavedLocation {
                    name: l.name.clone(),
                    pose: SavedPose::from_pose(&l.pose, l.scale),
                })
                .collect(),
        })
        .ok()
    }

    fn restore_state(&mut self, state: &serde_json::Value) -> anyhow::Result<()> {
        let saved: Saved = serde_json::from_value(state.clone())?;
        anyhow::ensure!(
            saved.version == 1,
            "unknown flight state version {}",
            saved.version
        );
        self.flight = Flight::new(saved.pose.to_pose());
        self.flight.scale = saved.pose.scale;
        // The saved pose already includes the sliders' turns.
        self.last_look = None;
        self.locations = saved
            .locations
            .iter()
            .map(|l| Location {
                name: l.name.clone(),
                pose: l.pose.to_pose(),
                scale: l.pose.scale,
            })
            .collect();
        self.recall = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(flight: &mut FractalFlight, values: &[(&str, ParamValue)]) -> AnalyzerSnapshot {
        step_taking(flight, 1.0 / 60.0, values)
    }

    /// One frame after a previous frame that took `seconds`.
    fn step_taking(
        flight: &mut FractalFlight,
        seconds: f32,
        values: &[(&str, ParamValue)],
    ) -> AnalyzerSnapshot {
        let mut state = AnalyzerStateSnapshot::default();
        for (name, value) in values {
            state.values.insert((*name).to_owned(), *value);
        }
        flight.step(&HostFrame {
            frame_seconds: seconds,
            state: &state,
        })
    }

    #[test]
    fn walking_forward_through_open_space_does_not_stall() {
        // From Find Inside the way ahead is open; a surface beside or below
        // the camera must not brake it to a crawl.
        let mut flight = FractalFlight::new();
        step(&mut flight, &[("find_inside", ParamValue::Bool(true))]);
        let forward = [("throttle", ParamValue::Float(1.0))];
        for _ in 0..180 {
            step(&mut flight, &forward);
        }
        let at =
            |t: &[[f32; 4]]| Vec3::new(f64::from(t[0][0]), f64::from(t[0][1]), f64::from(t[0][2]));
        let before = at(&texels(&step(&mut flight, &forward)));
        let mut after = before;
        for _ in 0..180 {
            after = at(&texels(&step(&mut flight, &forward)));
        }
        let moved = (after - before).length();
        let scale = flight.flight.scale;
        assert!(
            moved > 0.25 * scale,
            "moved {moved} in 3 s at scale {scale}"
        );
    }

    #[test]
    fn a_look_slider_turn_is_reprojected_from_the_unturned_camera() {
        let mut flight = FractalFlight::new();
        let before = texels(&step(
            &mut flight,
            &[("look_heading", ParamValue::Float(0.0))],
        ));
        let after = texels(&step(
            &mut flight,
            &[("look_heading", ParamValue::Float(30.0))],
        ));
        assert_ne!(after[4], before[4], "the camera turned");
        assert_eq!(
            after[9], before[4],
            "last frame's forward is the unturned one"
        );
    }

    #[test]
    fn the_camera_flies_in_wall_clock_time() {
        // A 60 fps deck steps its animation 1/60 s however long frames
        // take; flight must still cover its speed per real second.
        let travelled = |frame_seconds: f32| {
            let mut flight = FractalFlight::new();
            let values = [
                ("throttle", ParamValue::Float(1.0)),
                ("speed", ParamValue::Float(1.0)),
            ];
            let start = texels(&step_taking(&mut flight, frame_seconds, &values))[0];
            let mut end = start;
            for _ in 0..30 {
                end = texels(&step_taking(&mut flight, frame_seconds, &values))[0];
            }
            (end[2] - start[2]).abs()
        };
        let (fast, slow) = (travelled(1.0 / 15.0), travelled(1.0 / 60.0));
        assert!(fast > 3.0 * slow, "at 15 fps {fast}, at 60 fps {slow}");
    }

    #[test]
    fn a_changing_look_lowers_history_trust_and_the_grade_does_not() {
        let rate = |name: &str, a: ParamValue, b: ParamValue| {
            let mut flight = FractalFlight::new();
            step(&mut flight, &[(name, a)]);
            texels(&step(&mut flight, &[(name, b)]))[10][0]
        };
        let sun = |v: f32| ParamValue::Float(v);
        assert!(rate("sun_intensity", sun(4.0), sun(4.4)) > 0.5);
        assert!(
            rate(
                "fog_color",
                ParamValue::Color([0.3, 0.3, 0.4, 1.0]),
                ParamValue::Color([0.3, 0.5, 0.4, 1.0]),
            ) > 0.5
        );
        assert_eq!(rate("sun_intensity", sun(4.0), sun(4.0)), 0.0);
        assert_eq!(
            rate("exposure", sun(0.0), sun(1.0)),
            0.0,
            "applied after TAA"
        );
    }

    #[test]
    fn the_governor_follows_the_wall_clock_not_the_animation_step() {
        // A 60 fps deck steps its animation 1/60 s however long frames take.
        let mut flight = FractalFlight::new();
        let values = [
            ("render_scale", ParamValue::Float(1.0)),
            ("target_fps", ParamValue::Float(50.0)),
        ];
        let mut live = 1.0;
        for _ in 0..60 {
            live = texels(&step_taking(&mut flight, 0.15, &values))[RESOLUTION_TEXEL][0];
        }
        assert!(live < 0.5, "{live}");
    }

    #[test]
    fn without_a_target_the_live_scale_is_the_ceiling() {
        let mut flight = FractalFlight::new();
        let values = [
            ("render_scale", ParamValue::Float(0.8)),
            ("target_fps", ParamValue::Float(0.0)),
        ];
        let snapshot = step_taking(&mut flight, 0.5, &values);
        assert_eq!(texels(&snapshot)[RESOLUTION_TEXEL][..2], [0.8, 0.8]);
        assert!((snapshot.scalar("render_scale_live") - 0.8).abs() < 1e-6);
    }

    #[test]
    fn slow_frames_lower_the_live_scale_and_report_the_last_one() {
        let mut flight = FractalFlight::new();
        let values = [
            ("render_scale", ParamValue::Float(1.0)),
            ("target_fps", ParamValue::Float(50.0)),
        ];
        let first = texels(&step_taking(&mut flight, 0.1, &values))[RESOLUTION_TEXEL];
        assert_eq!(
            first[0], first[1],
            "no earlier frame: the previous is the live scale"
        );
        let mut last = [first[0], first[1]];
        for _ in 0..60 {
            let t = texels(&step_taking(&mut flight, 0.1, &values));
            let live = t[RESOLUTION_TEXEL];
            assert_eq!(live[1], last[0], "the previous scale is last frame's");
            last = [live[0], live[1]];
        }
        assert!(last[0] < 0.5, "{}", last[0]);
    }

    fn texels(snapshot: &AnalyzerSnapshot) -> Vec<[f32; 4]> {
        let data = &snapshot.textures[TEXTURE].data;
        data.as_chunks::<16>()
            .0
            .iter()
            .map(|c| {
                std::array::from_fn(|i| {
                    f32::from_le_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]])
                })
            })
            .collect()
    }

    #[test]
    fn publishes_the_camera_and_the_layout_version() {
        let mut flight = FractalFlight::new();
        let snapshot = step(&mut flight, &[("fov", ParamValue::Float(1.3))]);
        let t = texels(&snapshot);
        assert_eq!(t.len(), TEXELS);
        assert_eq!(t[0], [0.0, 0.0, -12.0, 1.3]);
        assert_eq!(t[4][..3], [0.0, 0.0, 1.0], "forward");
        assert_eq!(t[10][2], LAYOUT_VERSION);
        assert!(snapshot.scalar("camera_distance") > 0.0);
    }

    #[test]
    fn the_high_and_low_parts_sum_to_the_f64_position() {
        let (high, low) = split(1.0 + 1e-9);
        assert!(((f64::from(high) + f64::from(low)) - (1.0 + 1e-9)).abs() < 1e-15);
    }

    #[test]
    fn packs_the_default_rotations() {
        let mut flight = FractalFlight::new();
        let t = texels(&step(&mut flight, &[]));
        assert_eq!(t.len(), TEXELS);
        // Slot 1 is turned: unit rows, not the identity.
        let row = t[ROTATION_TEXEL];
        let length = (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt();
        assert!(
            (length - 1.0).abs() < 1e-6 && row[0] < 0.999,
            "turned slot 1"
        );
        assert_eq!(
            t[ROTATION_TEXEL + 3],
            [1.0, 0.0, 0.0, 0.0],
            "unrotated slot 2"
        );
        assert_eq!(t[ROTATION_TEXEL + 5], [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_formula_change_reports_full_morph() {
        let mut flight = FractalFlight::new();
        step(&mut flight, &[]);
        let t = texels(&step(&mut flight, &[("slot1_a", ParamValue::Float(-2.0))]));
        assert_eq!(t[10][0], 1.0);
        let t = texels(&step(&mut flight, &[("slot1_a", ParamValue::Float(-2.0))]));
        assert_eq!(t[10][0], 0.0, "a still formula has no morph");
    }

    #[test]
    fn a_saved_location_is_recalled_by_changing_location() {
        let mut flight = FractalFlight::new();
        step(&mut flight, &[("location", ParamValue::Long(0))]);
        step(&mut flight, &[("save_location", ParamValue::Bool(true))]);
        let saved_pose = flight.flight.pose;
        for _ in 0..60 {
            step(
                &mut flight,
                &[
                    ("throttle", ParamValue::Float(1.0)),
                    ("yaw_rate", ParamValue::Float(0.5)),
                    ("location", ParamValue::Long(0)),
                ],
            );
        }
        assert_ne!(flight.flight.pose, saved_pose);
        // Changing the parameter away and back recalls location 0.
        step(
            &mut flight,
            &[
                ("location", ParamValue::Long(1)),
                ("recall_time", ParamValue::Float(0.0)),
            ],
        );
        step(
            &mut flight,
            &[
                ("location", ParamValue::Long(0)),
                ("recall_time", ParamValue::Float(0.0)),
            ],
        );
        assert_eq!(flight.flight.pose, saved_pose);
    }

    #[test]
    fn a_tour_visits_every_location_in_turn() {
        let mut flight = FractalFlight::new();
        let mut poses = Vec::new();
        for i in 0..2 {
            for _ in 0..30 {
                step(
                    &mut flight,
                    &[
                        ("throttle", ParamValue::Float(1.0)),
                        ("yaw_rate", ParamValue::Float(0.3 * f32::from(i as u8))),
                    ],
                );
            }
            step(&mut flight, &[("save_location", ParamValue::Bool(true))]);
            step(&mut flight, &[]);
            poses.push(flight.flight.pose);
        }
        let tour = [
            ("tour", ParamValue::Bool(true)),
            ("tour_seconds", ParamValue::Float(0.5)),
            ("recall_time", ParamValue::Float(0.2)),
        ];
        let mut visited = Vec::new();
        for _ in 0..120 {
            step(&mut flight, &tour);
            if let Some(i) = flight.location_index
                && visited.last() != Some(&i)
            {
                visited.push(i);
            }
        }
        assert!(visited.starts_with(&[0, 1, 0]), "{visited:?}");
    }

    #[test]
    fn the_autopilot_steers() {
        let mut flight = FractalFlight::new();
        let start = flight.flight.pose.forward();
        for _ in 0..120 {
            step(&mut flight, &[("autopilot", ParamValue::Float(1.0))]);
        }
        let end = flight.flight.pose.forward();
        assert!((end - start).length() > 0.05, "forward unchanged: {end:?}");
    }

    /// Steady flight turns smoothly: the camera's angular velocity changes
    /// little from one frame to the next, relative to how much it turns.
    #[test]
    fn the_autopilot_turns_smoothly() {
        let mut flight = FractalFlight::new();
        let mut forwards = Vec::new();
        for _ in 0..600 {
            step(&mut flight, &[("autopilot", ParamValue::Float(1.0))]);
            forwards.push(flight.flight.pose.forward());
        }
        let spins: Vec<Vec3> = forwards.windows(2).map(|f| f[0].cross(f[1])).collect();
        let turned: f64 = spins.iter().map(|w| w.length()).sum();
        let jerk: f64 = spins.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        println!(
            "turned {turned:.4} rad, jerk {jerk:.4}, ratio {:.4}",
            jerk / turned.max(1e-9)
        );
        assert!(turned > 0.05, "the autopilot did not turn: {turned}");
        assert!(
            jerk < 0.1 * turned,
            "turn rate jumps: total change {jerk:.4} against {turned:.4} turned"
        );
    }

    fn heading(flight: &FractalFlight) -> f64 {
        let f = flight.flight.pose.forward();
        f.x.atan2(f.z).to_degrees()
    }

    /// Moving the Heading slider turns the camera by as much as it moved,
    /// and the view stays there.
    #[test]
    fn the_heading_slider_turns_the_camera_by_its_change() {
        let mut flight = FractalFlight::new();
        step(&mut flight, &[("look_heading", ParamValue::Float(0.0))]);
        let start = heading(&flight);
        step(&mut flight, &[("look_heading", ParamValue::Float(30.0))]);
        assert!(
            (heading(&flight) - start - 30.0).abs() < 1e-6,
            "{}",
            heading(&flight)
        );
        for _ in 0..10 {
            step(&mut flight, &[("look_heading", ParamValue::Float(30.0))]);
        }
        assert!((heading(&flight) - start - 30.0).abs() < 1e-6, "held still");
        step(&mut flight, &[("look_heading", ParamValue::Float(0.0))]);
        assert!((heading(&flight) - start).abs() < 1e-6, "back again");
    }

    /// Pitch up looks up.
    #[test]
    fn the_pitch_slider_looks_up() {
        let mut flight = FractalFlight::new();
        step(&mut flight, &[]);
        step(&mut flight, &[("look_pitch", ParamValue::Float(20.0))]);
        let up = flight.flight.pose.forward().y.asin().to_degrees();
        assert!((up - 20.0).abs() < 1e-6, "{up}");
    }

    /// A value already set when the flight starts, as in a loaded scene, is
    /// where the view is, not a turn to make.
    #[test]
    fn a_starting_slider_value_does_not_turn() {
        let mut flight = FractalFlight::new();
        let start = heading(&flight);
        step(&mut flight, &[("look_heading", ParamValue::Float(45.0))]);
        assert!((heading(&flight) - start).abs() < 1e-9);
    }

    #[test]
    fn find_inside_moves_the_camera_into_the_structure_once_per_press() {
        let mut flight = FractalFlight::new();
        step(&mut flight, &[]);
        let outside = flight.flight.pose.position;
        step(&mut flight, &[("find_inside", ParamValue::Bool(true))]);
        let inside = flight.flight.pose.position;
        assert!((inside - outside).length() > 1.0, "did not move");
        assert!(inside.length() < INSIDE_RADII[1]);
        assert!(flight.flight.scale > 0.0);
        // Held down, it does not search again and pull the camera back.
        let mut held = flight.flight.pose;
        held.position = held.position + Vec3::new(1e-3, 0.0, 0.0);
        flight.flight.pose = held;
        step(&mut flight, &[("find_inside", ParamValue::Bool(true))]);
        assert!((flight.flight.pose.position - inside).length() > 5e-4);
    }

    /// Find Inside says when the stack has no rooms, and only then.
    #[test]
    fn find_inside_reports_a_stack_without_rooms() {
        let press = |stack: &[(&str, f64)]| {
            let mut flight = FractalFlight::new();
            let mut values: Vec<(&str, ParamValue)> = stack
                .iter()
                .map(|&(name, v)| (name, ParamValue::Float(v as f32)))
                .collect();
            step(&mut flight, &values);
            values.push(("find_inside", ParamValue::Bool(true)));
            step(&mut flight, &values);
            flight.take_message()
        };
        let bulb = press(&[
            ("slot1_formula", 7.0),
            ("slot1_a", 8.0),
            ("slot1_b", 1.0),
            ("slot2_formula", 0.0),
            ("max_iterations", 10.0),
        ]);
        assert!(
            bulb.is_some_and(|m| m.starts_with("No enclosed space")),
            "a Mandelbulb has no rooms"
        );
        let caves = press(&[
            ("slot1_a", 2.2),
            ("slot2_formula", 0.0),
            ("max_iterations", 12.0),
        ]);
        assert_eq!(caves, None, "Box 2.2 has rooms");
    }

    /// A hybrid that fills the ball around the origin with solid still has a
    /// place to find: further out, near what the camera sees.
    #[test]
    fn find_inside_searches_beyond_a_solid_core() {
        let forest = [
            ("slot1_formula", 13.0),
            ("slot1_a", 14.0),
            ("slot1_b", 0.0),
            ("slot1_c", 0.1),
            ("slot1_d", 0.0),
            ("slot2_formula", 1.0),
            ("slot2_count", 1.0),
            ("slot2_a", 2.0),
            ("slot2_b", 0.5),
            ("slot2_c", 1.0),
            ("slot2_d", 1.0),
            ("slot3_formula", 14.0),
            ("slot3_count", 1.0),
            ("slot3_a", 0.3),
            ("slot3_b", 1.0),
            ("slot3_c", 1.0),
            ("slot3_d", 0.0),
            ("slot4_formula", 15.0),
            ("slot4_count", 1.0),
            ("slot4_a", 0.5),
            ("repeat_from", 1.0),
            ("julia_mode", 1.0),
            ("julia_x", 0.333),
            ("julia_y", 0.0),
            ("julia_z", 0.1),
            ("max_iterations", 14.0),
        ];
        let values: Vec<(&str, ParamValue)> = forest
            .iter()
            .map(|(k, v)| (*k, ParamValue::Float(*v as f32)))
            .collect();
        let mut flight = FractalFlight::new();
        step(&mut flight, &values);
        let before = flight.flight.pose.position;
        let mut pressed = values.clone();
        pressed.push(("find_inside", ParamValue::Bool(true)));
        step(&mut flight, &pressed);
        assert!(
            (flight.flight.pose.position - before).length() > 1e-3,
            "did not move"
        );
    }

    #[test]
    fn the_scene_scale_is_published_and_saved() {
        let mut flight = FractalFlight::new();
        let snapshot = step(&mut flight, &[]);
        let scale = flight.flight.scale;
        assert!(scale > 0.0);
        assert!((f64::from(texels(&snapshot)[10][3]) - scale).abs() < 1e-5 * scale);
        let saved = flight.persisted_state().unwrap();
        let mut restored = FractalFlight::new();
        restored.restore_state(&saved).unwrap();
        assert_eq!(restored.flight.scale, scale);
    }

    #[test]
    fn locations_survive_a_save() {
        let mut flight = FractalFlight::new();
        step(&mut flight, &[("save_location", ParamValue::Bool(true))]);
        let saved = flight.persisted_state().unwrap();
        let mut restored = FractalFlight::new();
        restored.restore_state(&saved).unwrap();
        assert_eq!(restored.locations.len(), 1);
        assert_eq!(restored.persisted_state().unwrap(), saved);
    }

    #[test]
    fn state_round_trips() {
        let mut flight = FractalFlight::new();
        for _ in 0..30 {
            step(
                &mut flight,
                &[
                    ("throttle", ParamValue::Float(1.0)),
                    ("yaw_rate", ParamValue::Float(0.5)),
                ],
            );
        }
        let saved = flight.persisted_state().unwrap();
        let mut restored = FractalFlight::new();
        restored.restore_state(&saved).unwrap();
        assert_eq!(restored.persisted_state().unwrap(), saved);
        assert!(
            restored
                .restore_state(&serde_json::json!({"version": 9}))
                .is_err()
        );
    }
}
