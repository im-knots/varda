//! A free camera that flies through a formula stack at a speed set by its
//! distance to the surface.

use super::stack::Stack;
use super::vec3::{Mat3, Vec3};

/// A unit quaternion, `w + xi + yj + zk`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Rotation by `angle` radians about unit `axis`.
    pub fn from_axis_angle(axis: Vec3, angle: f64) -> Self {
        let (s, c) = (angle * 0.5).sin_cos();
        Self {
            w: c,
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
        }
    }

    /// Composition: rotating by `a.compose(b)` rotates by `b` first, then `a`.
    pub fn compose(self, o: Self) -> Self {
        Self {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }

    pub fn normalize(self) -> Self {
        let n = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        Self {
            w: self.w / n,
            x: self.x / n,
            y: self.y / n,
            z: self.z / n,
        }
    }

    /// The rotation matrix, rows in the world frame.
    pub fn to_mat3(self) -> Mat3 {
        let Self { w, x, y, z } = self;
        Mat3 {
            rows: [
                Vec3::new(
                    1.0 - 2.0 * (y * y + z * z),
                    2.0 * (x * y - w * z),
                    2.0 * (x * z + w * y),
                ),
                Vec3::new(
                    2.0 * (x * y + w * z),
                    1.0 - 2.0 * (x * x + z * z),
                    2.0 * (y * z - w * x),
                ),
                Vec3::new(
                    2.0 * (x * z - w * y),
                    2.0 * (y * z + w * x),
                    1.0 - 2.0 * (x * x + y * y),
                ),
            ],
        }
    }

    pub fn rotate(self, v: Vec3) -> Vec3 {
        self.to_mat3().apply(v)
    }
}

/// Where the camera is and which way it faces. Local axes: +x right, +y up,
/// +z forward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Pose {
    pub fn right(&self) -> Vec3 {
        self.orientation.rotate(Vec3::new(1.0, 0.0, 0.0))
    }

    pub fn up(&self) -> Vec3 {
        self.orientation.rotate(Vec3::new(0.0, 1.0, 0.0))
    }

    pub fn forward(&self) -> Vec3 {
        self.orientation.rotate(Vec3::new(0.0, 0.0, 1.0))
    }
}

impl Pose {
    /// At `position`, facing `forward`, with up as close to world +y as
    /// `forward` allows. Straight up or down, up falls back to world +z.
    pub fn looking(position: Vec3, forward: Vec3) -> Self {
        let f = forward.normalize();
        let world_up = if f.y.abs() > 0.999 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let r = world_up.cross(f).normalize();
        let u = f.cross(r);
        // Rotation matrix with columns right, up, forward, to a quaternion.
        let trace = r.x + u.y + f.z;
        let orientation = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Quat {
                w: 0.25 * s,
                x: (u.z - f.y) / s,
                y: (f.x - r.z) / s,
                z: (r.y - u.x) / s,
            }
        } else if r.x > u.y && r.x > f.z {
            let s = (1.0 + r.x - u.y - f.z).sqrt() * 2.0;
            Quat {
                w: (u.z - f.y) / s,
                x: 0.25 * s,
                y: (u.x + r.y) / s,
                z: (f.x + r.z) / s,
            }
        } else if u.y > f.z {
            let s = (1.0 + u.y - r.x - f.z).sqrt() * 2.0;
            Quat {
                w: (f.x - r.z) / s,
                x: (u.x + r.y) / s,
                y: 0.25 * s,
                z: (f.y + u.z) / s,
            }
        } else {
            let s = (1.0 + f.z - r.x - u.y).sqrt() * 2.0;
            Quat {
                w: (r.y - u.x) / s,
                x: (f.x + r.z) / s,
                y: (f.y + u.z) / s,
                z: 0.25 * s,
            }
        };
        Self {
            position,
            orientation: orientation.normalize(),
        }
    }
}

impl Default for Pose {
    /// Outside the default stack, looking at it. A scale 2.2 Mandelbox fits
    /// in a radius of `2 (|s| + 1) / (|s| - 1)`, about 5.3.
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, -12.0),
            orientation: Quat::IDENTITY,
        }
    }
}

/// How the camera's speed and the scene scale behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlightMode {
    /// Constant speed and scale, braking near surfaces: moving through a place.
    #[default]
    Walk,
    /// Speed and scale follow the distance to the surface: zooming into detail.
    Dive,
}

/// Steering inputs for one frame, each in `-1..=1` except `speed`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Controls {
    pub throttle: f64,
    pub strafe_x: f64,
    pub strafe_y: f64,
    pub yaw_rate: f64,
    pub pitch_rate: f64,
    pub roll_rate: f64,
    /// Speed in scene scales per second (Walk) or distances to the surface
    /// per second (Dive).
    pub speed: f64,
    pub mode: FlightMode,
}

/// Turn rate at full stick, radians per second.
pub const TURN_RATE: f64 = 1.0;
/// Seconds for the velocity to settle toward the stick.
pub const VELOCITY_SMOOTHING: f64 = 0.25;
/// The most of the distance to the surface one frame may cover.
pub const MAX_STEP_FRACTION: f64 = 0.5;
/// The fraction of the scene scale at which the camera touches the surface.
pub const CONTACT: f64 = 1e-3;
/// Steering turns along a surface closer ahead than this part of the scene
/// scale.
pub const SLIDE_AHEAD: f64 = 0.05;
/// Within this many contact distances of a surface, the camera slides.
pub const SLIDE_RANGE: f64 = 8.0;
/// Distance-estimate samples in one clearance march.
pub const CLEARANCE_STEPS: usize = 48;
/// Distance floor, so a camera touching the surface can still back away.
pub const MIN_DISTANCE: f64 = 1e-6;
/// Seconds for autofocus to settle on a new distance.
pub const FOCUS_SMOOTHING: f64 = 0.5;
/// Sphere-tracing steps for the autofocus ray.
const FOCUS_STEPS: usize = 96;
/// Seconds for the scene scale to follow the distance in Dive.
pub const SCALE_SMOOTHING: f64 = 0.5;
/// Walking slows once the surface is closer than `scale / WALK_BRAKE`.
pub const WALK_BRAKE: f64 = 2.0;

/// The camera between frames.
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub pose: Pose,
    /// Last frame's pose, for reprojection.
    pub previous: Pose,
    pub velocity: Vec3,
    /// Smoothed distance along the view axis to the surface.
    pub focus: f64,
    /// The length the look is measured in. Constant while walking; follows
    /// the distance while diving. 0 until the first frame sets it.
    pub scale: f64,
}

/// What one step measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepReport {
    /// Distance estimate at the camera after the step.
    pub distance: f64,
    /// World units per second actually flown.
    pub speed: f64,
    /// Whether the collision rule stopped the step.
    pub blocked: bool,
}

impl Flight {
    pub fn new(pose: Pose) -> Self {
        Self {
            pose,
            previous: pose,
            velocity: Vec3::ZERO,
            focus: 0.0,
            scale: 0.0,
        }
    }

    /// The scale the distance alone suggests: the distance at the camera, or
    /// a quarter of the traced distance ahead when that is larger.
    fn natural_scale(&self, distance: f64) -> f64 {
        distance.max(0.25 * self.focus).max(MIN_DISTANCE)
    }

    /// Distance to the surface at `p`, floored.
    fn distance(stack: &Stack, p: Vec3) -> f64 {
        stack.sample(p, 0.0).distance.max(MIN_DISTANCE)
    }

    /// Free distance from `p` along the unit vector `dir`, up to `reach`: the
    /// distance estimate marched forward, so a surface beside or behind the
    /// camera does not count. The estimate is a lower bound, so the march
    /// never passes a surface.
    fn clearance(&self, stack: &Stack, p: Vec3, dir: Vec3, reach: f64) -> f64 {
        let contact = self.contact();
        let mut t = 0.0;
        for _ in 0..CLEARANCE_STEPS {
            let d = Self::distance(stack, p + dir * t);
            if d < contact {
                break;
            }
            t += d;
            if t >= reach {
                return reach;
            }
        }
        t
    }

    /// The outward surface normal near `p`: the distance estimate's gradient
    /// by central differences `e` apart.
    fn normal(stack: &Stack, p: Vec3, e: f64) -> Vec3 {
        let d = |v: Vec3| Self::distance(stack, p + v * e) - Self::distance(stack, p - v * e);
        let g = Vec3::new(
            d(Vec3::new(1.0, 0.0, 0.0)),
            d(Vec3::new(0.0, 1.0, 0.0)),
            d(Vec3::new(0.0, 0.0, 1.0)),
        );
        if g.length() > 0.0 {
            g.normalize()
        } else {
            Vec3::ZERO
        }
    }

    /// Whether `p` touches the drawn surface: its distance estimate is
    /// under `CONTACT` of the scene scale, as the renderer's hit test is a
    /// small distance, not an orbit that stays bounded. Bounded orbits reach
    /// far past what is drawn (Gnarl leaves wide bands of them in what renders
    /// as open space), and colliding with them stopped the camera at
    /// invisible walls.
    fn inside(&self, stack: &Stack, p: Vec3) -> bool {
        Self::distance(stack, p) < self.contact()
    }

    /// The distance the camera counts as touching the surface.
    fn contact(&self) -> f64 {
        (CONTACT * self.scale).max(4.0 * MIN_DISTANCE)
    }

    /// Advance one frame of `dt` seconds.
    pub fn step(&mut self, stack: &Stack, controls: &Controls, dt: f64) -> StepReport {
        self.previous = self.pose;
        let dt = dt.max(0.0);

        // Turn in the camera's own frame.
        let turn = |axis: Vec3, rate: f64| {
            Quat::from_axis_angle(axis, rate.clamp(-1.0, 1.0) * TURN_RATE * dt)
        };
        self.pose.orientation = self
            .pose
            .orientation
            .compose(turn(Vec3::new(0.0, 1.0, 0.0), controls.yaw_rate))
            .compose(turn(Vec3::new(1.0, 0.0, 0.0), controls.pitch_rate))
            .compose(turn(Vec3::new(0.0, 0.0, 1.0), controls.roll_rate))
            .normalize();

        let distance = Self::distance(stack, self.pose.position);
        if self.scale <= 0.0 {
            self.scale = self.natural_scale(distance);
        }
        let mut heading = self.pose.forward() * controls.throttle.clamp(-1.0, 1.0)
            + self.pose.right() * controls.strafe_x.clamp(-1.0, 1.0)
            + self.pose.up() * controls.strafe_y.clamp(-1.0, 1.0);
        // Steering into a surface close ahead turns along it, so the brake
        // below measures the way the camera will actually go.
        if heading.length() > 1e-9 {
            let ahead = self.clearance(stack, self.pose.position, heading.normalize(), self.scale);
            if ahead < SLIDE_AHEAD * self.scale {
                let n = Self::normal(stack, self.pose.position, distance.max(self.contact()));
                let into = heading.dot(n);
                if into < 0.0 {
                    heading = heading - n * into;
                }
            }
        }
        let reach = match controls.mode {
            // Walking brakes for what is ahead, not the nearest surface in
            // any direction, which a floor or wall alongside would be.
            FlightMode::Walk => {
                let ahead = if heading.length() > 1e-9 {
                    self.clearance(stack, self.pose.position, heading.normalize(), self.scale)
                } else {
                    distance
                };
                self.scale.min(WALK_BRAKE * ahead)
            }
            FlightMode::Dive => distance,
        };
        let speed = controls.speed.max(0.0) * reach;
        let wanted = heading * speed;
        let blend = if dt > 0.0 {
            1.0 - (-dt / VELOCITY_SMOOTHING).exp()
        } else {
            0.0
        };
        self.velocity = self.velocity + (wanted - self.velocity) * blend;

        let mut step = self.velocity * dt;
        // Near a surface, motion into it turns into motion along it: the
        // camera slides along walls and around specks instead of stopping.
        if step.length() > 0.0 {
            let ahead = self.clearance(
                stack,
                self.pose.position,
                step.normalize(),
                2.0 * step.length(),
            );
            if ahead < 2.0 * step.length() || distance < SLIDE_RANGE * self.contact() {
                let n = Self::normal(stack, self.pose.position, distance.max(self.contact()));
                let into = step.dot(n);
                if into < 0.0 {
                    step = step - n * into;
                }
                let pushing = self.velocity.dot(n);
                if pushing < 0.0 {
                    self.velocity = self.velocity - n * pushing;
                }
            }
        }
        let length = step.length();
        if length > 0.0 {
            // At most half the free distance along the step itself.
            let free = self.clearance(
                stack,
                self.pose.position,
                step * (1.0 / length),
                2.0 * length,
            );
            let limit = MAX_STEP_FRACTION * free;
            if length > limit {
                step = step * (limit / length);
            }
        }
        let was_inside = self.inside(stack, self.pose.position);
        let target = self.pose.position + step;
        let blocked = !was_inside && self.inside(stack, target);
        if blocked {
            self.velocity = Vec3::ZERO;
        } else {
            self.pose.position = target;
        }

        self.update_focus(stack, dt);
        let distance = Self::distance(stack, self.pose.position);
        if controls.mode == FlightMode::Dive {
            let target = self.natural_scale(distance);
            self.scale += (target - self.scale) * (1.0 - (-dt / SCALE_SMOOTHING).exp());
        }

        StepReport {
            distance,
            speed: if dt > 0.0 && !blocked {
                step.length() / dt
            } else {
                0.0
            },
            blocked,
        }
    }

    /// Move straight to `pose`, as a recall flight does. Last frame's pose
    /// stays available for reprojection.
    pub fn place(&mut self, stack: &Stack, pose: Pose, dt: f64) -> StepReport {
        self.previous = self.pose;
        let speed = if dt > 0.0 {
            (pose.position - self.pose.position).length() / dt
        } else {
            0.0
        };
        self.pose = pose;
        self.velocity = Vec3::ZERO;
        self.update_focus(stack, dt.max(0.0));
        StepReport {
            distance: Self::distance(stack, pose.position),
            speed,
            blocked: false,
        }
    }

    fn update_focus(&mut self, stack: &Stack, dt: f64) {
        let focus = sphere_trace(
            stack,
            self.pose.position,
            self.pose.forward(),
            FOCUS_STEPS,
            f64::MAX,
        );
        self.focus = if self.focus <= 0.0 || dt <= 0.0 {
            focus
        } else {
            self.focus + (focus - self.focus) * (1.0 - (-dt / FOCUS_SMOOTHING).exp())
        };
    }
}

/// Distance along `dir` to the surface, by sphere tracing: at most `steps`
/// steps and `max_t`. The last distance reached when nothing is hit.
pub fn sphere_trace(stack: &Stack, origin: Vec3, dir: Vec3, steps: usize, max_t: f64) -> f64 {
    let mut t = 0.0;
    for _ in 0..steps {
        let d = stack
            .sample(origin + dir * t, 0.0)
            .distance
            .max(MIN_DISTANCE);
        if d < 1e-4 * t.max(MIN_DISTANCE) || t > max_t {
            break;
        }
        t += d;
    }
    t.min(max_t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::stack_from_params;

    fn default_stack() -> Stack {
        stack_from_params(|_| None)
    }

    /// A Menger sponge: a cube from -1 to 1 with flat faces.
    fn menger() -> Stack {
        stack_from_params(|name| match name {
            "slot1_formula" => Some(2.0),
            "slot1_a" => Some(3.0),
            "slot1_b" => Some(1.0),
            "slot2_formula" => Some(0.0),
            "max_iterations" => Some(8.0),
            _ => None,
        })
    }

    #[test]
    fn walking_into_a_wall_at_an_angle_slides_along_it() {
        // Heading at the sponge's z = -1 face at 45 degrees: on reaching it,
        // the camera keeps the along-wall part of its motion.
        let stack = menger();
        let start = Vec3::new(-0.3, 0.05, -1.6);
        let mut flight = Flight::new(Pose::looking(start, Vec3::new(1.0, 0.0, 1.0)));
        flight.scale = 0.5;
        let walk = Controls {
            throttle: 1.0,
            speed: 1.0,
            mode: FlightMode::Walk,
            ..Controls::default()
        };
        for _ in 0..240 {
            flight.step(&stack, &walk, 1.0 / 60.0);
        }
        let reached = flight.pose.position;
        for _ in 0..120 {
            flight.step(&stack, &walk, 1.0 / 60.0);
        }
        let slid = flight.pose.position.x - reached.x;
        assert!(
            slid > 0.1,
            "stuck at the wall: moved {slid} along it in 2 s"
        );
        assert!(
            stack.sample(flight.pose.position, 0.0).distance >= flight.contact(),
            "touching at {:?}",
            flight.pose.position
        );
    }

    fn forward(speed: f64) -> Controls {
        Controls {
            throttle: 1.0,
            speed,
            mode: FlightMode::Dive,
            ..Controls::default()
        }
    }

    fn walk(speed: f64) -> Controls {
        Controls {
            mode: FlightMode::Walk,
            ..forward(speed)
        }
    }

    #[test]
    fn walking_keeps_a_constant_speed_in_open_space() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        flight.scale = 0.5;
        let mut speeds = Vec::new();
        for _ in 0..120 {
            speeds.push(flight.step(&stack, &walk(1.0), 1.0 / 60.0).speed);
        }
        // Past the velocity smoothing, the camera covers one scale per second
        // although the distance to the surface keeps shrinking.
        for s in &speeds[90..] {
            assert!((s - 0.5).abs() < 0.01, "speed {s}");
        }
        assert!(
            (flight.scale - 0.5).abs() < 1e-12,
            "scale moved: {}",
            flight.scale
        );
    }

    #[test]
    fn walking_brakes_near_a_surface() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        flight.scale = 100.0;
        let mut speeds = Vec::new();
        for _ in 0..600 {
            speeds.push(flight.step(&stack, &walk(1.0), 1.0 / 60.0).speed);
            assert!(stack.sample(flight.pose.position, 0.0).distance >= CONTACT * flight.scale);
        }
        let peak = speeds.iter().copied().fold(0.0, f64::max);
        let last = *speeds.last().unwrap();
        assert!(last < peak * 0.5, "peak {peak}, last {last}");
    }

    #[test]
    fn diving_scale_follows_the_distance() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        flight.step(&stack, &forward(1.0), 1.0 / 60.0);
        let start = flight.scale;
        assert!(start > 0.0);
        for _ in 0..600 {
            flight.step(&stack, &forward(1.0), 1.0 / 60.0);
        }
        assert!(flight.scale < start * 0.5, "{start} -> {}", flight.scale);
    }

    #[test]
    fn the_first_frame_sets_the_scale() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        let report = flight.step(&stack, &walk(0.0), 1.0 / 60.0);
        assert!(
            (flight.scale - report.distance).abs() < 1e-9 * report.distance.max(1.0)
                || flight.scale >= report.distance
        );
    }

    #[test]
    fn looking_faces_the_direction_with_up_toward_the_sky() {
        for f in [
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.3, -0.8, 0.5),
            Vec3::new(-0.6, 0.7, -0.2),
            Vec3::new(0.0, -1.0, 0.0),
        ] {
            let pose = Pose::looking(Vec3::ZERO, f);
            assert!(
                (pose.forward() - f.normalize()).length() < 1e-9,
                "{f:?}: {:?}",
                pose.forward()
            );
            assert!(pose.right().y.abs() < 1e-9, "{f:?}: rolled");
            if f.y.abs() < 0.999 {
                assert!(pose.up().y > 0.0, "{f:?}: upside down");
            }
        }
    }

    #[test]
    fn quaternion_rotation_matches_axis_angle() {
        let q = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), std::f64::consts::FRAC_PI_2);
        let v = q.rotate(Vec3::new(0.0, 0.0, 1.0));
        assert!((v - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-12);
    }

    #[test]
    fn diving_toward_the_surface_slows_with_distance() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        let mut speeds = Vec::new();
        for _ in 0..600 {
            speeds.push(flight.step(&stack, &forward(1.0), 1.0 / 60.0).speed);
        }
        let peak = speeds.iter().copied().fold(0.0, f64::max);
        let last = *speeds.last().unwrap();
        assert!(peak > 0.0);
        assert!(last < peak * 0.5, "peak {peak}, last {last}");
    }

    #[test]
    fn the_camera_never_touches_the_drawn_surface() {
        // Contact is a small distance estimate, as the renderer's hit is; a
        // bounded orbit with a large distance is open space on screen.
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        for _ in 0..2000 {
            flight.step(&stack, &forward(4.0), 1.0 / 30.0);
            assert!(
                stack.sample(flight.pose.position, 0.0).distance >= flight.contact(),
                "camera touching at {:?}",
                flight.pose.position
            );
        }
    }

    #[test]
    fn previous_pose_is_last_frames_pose() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        flight.step(&stack, &forward(1.0), 1.0 / 60.0);
        let after_first = flight.pose;
        flight.step(&stack, &forward(1.0), 1.0 / 60.0);
        assert_eq!(flight.previous, after_first);
    }

    #[test]
    fn yaw_turns_about_the_camera_up_axis() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        let controls = Controls {
            yaw_rate: 1.0,
            ..Controls::default()
        };
        for _ in 0..60 {
            flight.step(&stack, &controls, 1.0 / 60.0);
        }
        let expect = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), TURN_RATE)
            .rotate(Vec3::new(0.0, 0.0, 1.0));
        assert!((flight.pose.forward() - expect).length() < 1e-9);
        assert_eq!(flight.pose.position, Pose::default().position);
    }

    #[test]
    fn autofocus_finds_the_surface_ahead() {
        let stack = default_stack();
        let mut flight = Flight::new(Pose::default());
        flight.step(&stack, &Controls::default(), 0.0);
        // The default stack fits in radius 5.3; the camera sits at z = -12.
        assert!(
            flight.focus > 1.0 && flight.focus < 12.0,
            "focus {}",
            flight.focus
        );
    }
}
