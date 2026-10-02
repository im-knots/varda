//! Moving the camera without a hand on the stick: recall flights to saved
//! locations, a tour through them, and an autopilot that steers toward open,
//! detailed space.

use super::camera::{MIN_DISTANCE, Pose, Quat, sphere_trace};
use super::stack::Stack;
use super::vec3::Vec3;

/// A saved camera pose and the scene scale it was filmed at.
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub name: String,
    pub pose: Pose,
    /// The flight's scene scale when saved; 0 when unknown.
    pub scale: f64,
}

/// Spherical interpolation between unit quaternions, the short way round.
pub fn slerp(a: Quat, b: Quat, t: f64) -> Quat {
    let mut b = b;
    let mut dot = a.w * b.w + a.x * b.x + a.y * b.y + a.z * b.z;
    if dot < 0.0 {
        b = Quat {
            w: -b.w,
            x: -b.x,
            y: -b.y,
            z: -b.z,
        };
        dot = -dot;
    }
    let (wa, wb) = if dot > 0.9995 {
        (1.0 - t, t)
    } else {
        let theta = dot.acos();
        let s = theta.sin();
        (((1.0 - t) * theta).sin() / s, (t * theta).sin() / s)
    };
    Quat {
        w: wa * a.w + wb * b.w,
        x: wa * a.x + wb * b.x,
        y: wa * a.y + wb * b.y,
        z: wa * a.z + wb * b.z,
    }
    .normalize()
}

/// A flight from one pose and scene scale to another, eased at both ends.
#[derive(Debug, Clone, PartialEq)]
pub struct Recall {
    from: Pose,
    to: Pose,
    from_scale: f64,
    to_scale: f64,
    duration: f64,
    elapsed: f64,
}

/// One frame of a recall flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecallFrame {
    pub pose: Pose,
    pub scale: f64,
    pub arrived: bool,
}

impl Recall {
    /// A scale of 0 at either end keeps the scale where it starts.
    pub fn new(from: (Pose, f64), to: (Pose, f64), duration: f64) -> Self {
        let to_scale = if to.1 > 0.0 { to.1 } else { from.1 };
        Self {
            from: from.0,
            to: to.0,
            from_scale: from.1,
            to_scale,
            duration: duration.max(0.0),
            elapsed: 0.0,
        }
    }

    /// Advance one frame. The scale moves geometrically, since scales
    /// differ by orders of magnitude.
    pub fn step(&mut self, dt: f64) -> RecallFrame {
        self.elapsed += dt.max(0.0);
        if self.duration <= 0.0 || self.elapsed >= self.duration {
            return RecallFrame {
                pose: self.to,
                scale: self.to_scale,
                arrived: true,
            };
        }
        let x = self.elapsed / self.duration;
        let eased = x * x * (3.0 - 2.0 * x);
        let scale = if self.from_scale > 0.0 && self.to_scale > 0.0 {
            (self.from_scale.ln() + (self.to_scale.ln() - self.from_scale.ln()) * eased).exp()
        } else {
            self.to_scale
        };
        RecallFrame {
            pose: Pose {
                position: self.from.position + (self.to.position - self.from.position) * eased,
                orientation: slerp(self.from.orientation, self.to.orientation, eased),
            },
            scale,
            arrived: false,
        }
    }
}

/// Points Find Inside scores.
const INSIDE_CANDIDATES: u32 = 256;
/// Probes reach this many distance estimates when testing enclosure.
const INSIDE_REACH: f64 = 8.0;
/// Enclosure a point needs to count as inside the structure.
const INSIDE_ENCLOSED: f64 = 0.6;

/// The 6 axis and 8 diagonal directions Find Inside probes.
pub fn probe_directions() -> [Vec3; 14] {
    let d = 1.0 / 3f64.sqrt();
    [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(d, d, d),
        Vec3::new(d, d, -d),
        Vec3::new(d, -d, d),
        Vec3::new(d, -d, -d),
        Vec3::new(-d, d, d),
        Vec3::new(-d, d, -d),
        Vec3::new(-d, -d, d),
        Vec3::new(-d, -d, -d),
    ]
}

fn radical_inverse(mut index: u32, base: u32) -> f64 {
    let mut fraction = 1.0;
    let mut result = 0.0;
    while index > 0 {
        fraction /= f64::from(base);
        result += fraction * f64::from(index % base);
        index /= base;
    }
    result
}

/// Where Find Inside puts the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inside {
    pub pose: Pose,
    /// The distance estimate there, the scene scale to use.
    pub scale: f64,
    /// Whether structure surrounds the place. False when the stack has no
    /// rooms, and this is only the most open point found.
    pub enclosed: bool,
}

/// A place in open space enclosed by the fractal, searched in each of the
/// balls `(center, radius)`. The camera faces the longest way that still
/// ends in structure. Without an enclosed candidate, the most enclosed one,
/// marked as not enclosed. `None` when no candidate lies outside the solid.
///
/// Candidates follow a Halton (2, 3, 5) sequence, so the answer is the same
/// every time for the same fractal and balls.
pub fn find_inside(stack: &Stack, balls: &[(Vec3, f64)]) -> Option<Inside> {
    struct Candidate {
        position: Vec3,
        distance: f64,
        enclosure: f64,
        view: Vec3,
    }
    let probes = probe_directions();
    let mut best: Option<Candidate> = None;
    let better = |c: &Candidate, b: &Candidate| {
        let (ce, be) = (
            c.enclosure >= INSIDE_ENCLOSED,
            b.enclosure >= INSIDE_ENCLOSED,
        );
        match (ce, be) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => c.distance > b.distance,
            (false, false) => (c.enclosure, c.distance) > (b.enclosure, b.distance),
        }
    };
    let points = balls.iter().flat_map(|&(center, radius)| {
        (1..=INSIDE_CANDIDATES).filter_map(move |i| {
            let unit = Vec3::new(
                radical_inverse(i, 2) * 2.0 - 1.0,
                radical_inverse(i, 3) * 2.0 - 1.0,
                radical_inverse(i, 5) * 2.0 - 1.0,
            );
            (unit.dot(unit) <= 1.0).then_some(center + unit * radius)
        })
    });
    for position in points {
        let sample = stack.sample(position, 0.0);
        if sample.distance <= MIN_DISTANCE || stack.is_solid(position) {
            continue;
        }
        let reach = INSIDE_REACH * sample.distance;
        let mut hits = 0;
        let mut view = probes[0];
        let mut longest = -1.0;
        for dir in probes {
            let free = sphere_trace(stack, position, dir, 64, reach);
            if free < reach * 0.99 {
                hits += 1;
                if free > longest {
                    longest = free;
                    view = dir;
                }
            }
        }
        let candidate = Candidate {
            position,
            distance: sample.distance,
            enclosure: f64::from(hits) / probes.len() as f64,
            view,
        };
        if best.as_ref().is_none_or(|b| better(&candidate, b)) {
            best = Some(candidate);
        }
    }
    best.map(|c| Inside {
        pose: Pose::looking(c.position, c.view),
        scale: c.distance,
        enclosed: c.enclosure >= INSIDE_ENCLOSED,
    })
}

/// Probe directions, as `(right, up)` offsets from forward: forward, then
/// rings at 15 and 30 degrees.
const PROBES: usize = 17;
/// Probes traced per frame; the rest keep last frame's scores.
const PROBES_PER_FRAME: usize = 4;
/// Turn rate per radian between the heading and the goal.
const STEER_GAIN: f64 = 2.0;
/// How sharply the goal favors the best-scoring probes. Scores are 0 to 1;
/// a probe this much worse than the best counts `1/e` as much.
const SCORE_TEMPERATURE: f64 = 0.05;
/// Score bonus for a probe pointing along the current goal, at full
/// alignment. Between two equally good ways the autopilot keeps the one it
/// chose instead of switching back and forth.
const GOAL_LOYALTY: f64 = 0.08;
/// Probes further than this from the best one do not pull the goal (cosine
/// of about 25 degrees). Averaging good ways on both sides of a wall would
/// aim at the wall.
const MODE_COS: f64 = 0.9;
/// Seconds for the goal heading to follow a change in the scores.
const GOAL_SMOOTHING: f64 = 0.6;
/// The same when boxed in, where turning soon matters more than turning
/// smoothly.
const BOXED_GOAL_SMOOTHING: f64 = 0.15;
/// Seconds for the turn rates to follow the goal.
const RATE_SMOOTHING: f64 = 0.3;

fn probe_offset(i: usize) -> (f64, f64) {
    if i == 0 {
        return (0.0, 0.0);
    }
    let ring = if i <= 8 { 15f64 } else { 30f64 }.to_radians();
    let angle = (i - 1) as f64 * std::f64::consts::TAU / 8.0;
    (ring.tan() * angle.cos(), ring.tan() * angle.sin())
}

/// Steering from the autopilot for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Steering {
    pub yaw_rate: f64,
    pub pitch_rate: f64,
    /// How good the best direction looks, 0 to 1.
    pub confidence: f64,
}

/// Looks ahead in a cone, scores each direction by room to fly, whether it
/// reaches structure, and how much detail is where it lands, and steers
/// toward the good ones.
///
/// Picking the single best probe makes the turn rate jump whenever the
/// leader changes, and scores in a fractal change all the time. Instead the
/// goal is the score-weighted mean of the directions near the best (a soft
/// maximum around one mode), with a bonus for staying on the current goal;
/// the goal and then the turn rates each follow through a first-order lag,
/// so the camera's turning is smooth to the second order.
#[derive(Debug, Clone, PartialEq)]
pub struct Autopilot {
    scores: [f64; PROBES],
    /// Free distance along each probe, in distances to the surface.
    room: [f64; PROBES],
    /// World direction each probe was traced in. The scores stay attached
    /// to the directions they measured while the camera turns.
    directions: [Option<Vec3>; PROBES],
    next: usize,
    /// Smoothed goal heading, world space.
    goal: Option<Vec3>,
    /// Smoothed yaw and pitch rates.
    rates: (f64, f64),
}

impl Default for Autopilot {
    fn default() -> Self {
        Self {
            scores: [0.0; PROBES],
            room: [0.0; PROBES],
            directions: [None; PROBES],
            next: 0,
            goal: None,
            rates: (0.0, 0.0),
        }
    }
}

/// The fraction of the way a first-order lag of `seconds` moves in `dt`.
fn follow(dt: f64, seconds: f64) -> f64 {
    1.0 - (-dt.max(0.0) / seconds).exp()
}

/// With less free space than this along every probe (in distances to the
/// surface), the autopilot turns hard toward the most open one.
const BOXED_IN: f64 = 2.0;

impl Autopilot {
    /// Score some probes and steer toward the good ones, over a frame of `dt`
    /// seconds. `distance` is the distance from the camera to the surface.
    pub fn step(&mut self, stack: &Stack, pose: &Pose, distance: f64, dt: f64) -> Steering {
        let reach = 16.0 * distance.max(MIN_DISTANCE);
        for _ in 0..PROBES_PER_FRAME {
            let i = self.next;
            self.next = (self.next + 1) % PROBES;
            let dir = direction(pose, probe_offset(i));
            let free = sphere_trace(stack, pose.position, dir, 32, reach);
            // Room: a few distances to the surface of free space ahead, so
            // the camera does not fly into a wall.
            let ratio = free / distance.max(MIN_DISTANCE);
            self.room[i] = ratio;
            self.directions[i] = Some(dir);
            let room = ((ratio - 1.5) / 4.5).clamp(0.0, 1.0);
            // Structure: the probe reached a surface rather than empty sky.
            let structure = if free < reach * 0.99 { 1.0 } else { 0.3 };
            // Detail: how much the escape count varies around where the
            // probe stopped. A smooth blob varies little.
            let hit = pose.position + dir * free;
            let spread = 0.25 * free.max(distance);
            let centre = stack.sample(hit, 0.0).smooth_iteration;
            let variation = [
                Vec3::new(spread, 0.0, 0.0),
                Vec3::new(0.0, spread, 0.0),
                Vec3::new(0.0, 0.0, spread),
            ]
            .iter()
            .map(|e| (stack.sample(hit + *e, 0.0).smooth_iteration - centre).abs())
            .sum::<f64>()
                / 3.0;
            let detail = (variation / 4.0).min(1.0);
            self.scores[i] = 0.4 * room + 0.3 * structure + 0.3 * detail;
        }
        let confidence = self.scores.iter().copied().fold(f64::MIN, f64::max);
        let most_room = self
            .room
            .iter()
            .copied()
            .enumerate()
            .fold(
                (0, f64::MIN),
                |acc, (i, r)| if r > acc.1 { (i, r) } else { acc },
            );
        let boxed_in = most_room.1 < BOXED_IN;
        let wanted = if boxed_in {
            // Boxed in: head for the most open probe, or, if that is
            // straight ahead, turn anyway.
            let i = if most_room.0 == 0 { 1 } else { most_room.0 };
            self.directions[i].unwrap_or_else(|| direction(pose, probe_offset(i)))
        } else {
            self.soft_best(pose)
        };
        let smoothing = if boxed_in {
            BOXED_GOAL_SMOOTHING
        } else {
            GOAL_SMOOTHING
        };
        let goal = match self.goal {
            Some(goal) => {
                let moved = goal + (wanted - goal) * follow(dt, smoothing);
                if moved.length() > 1e-9 {
                    moved.normalize()
                } else {
                    wanted
                }
            }
            None => wanted,
        };
        self.goal = Some(goal);

        // Angles to the goal in the camera's frame. Pitching positive turns
        // the view down.
        let ahead = goal.dot(pose.forward());
        let yaw = goal.dot(pose.right()).atan2(ahead);
        let pitch = -goal.dot(pose.up()).atan2(ahead);
        let gain = if boxed_in {
            4.0 * STEER_GAIN
        } else {
            STEER_GAIN
        };
        let target = (
            (yaw * gain).clamp(-1.0, 1.0),
            (pitch * gain).clamp(-1.0, 1.0),
        );
        let k = follow(dt, RATE_SMOOTHING);
        self.rates.0 += (target.0 - self.rates.0) * k;
        self.rates.1 += (target.1 - self.rates.1) * k;
        Steering {
            yaw_rate: self.rates.0,
            pitch_rate: self.rates.1,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    /// The score-weighted mean direction of the probes near the best one,
    /// with scores favoring the current goal.
    fn soft_best(&self, pose: &Pose) -> Vec3 {
        let biased: Vec<(f64, Vec3)> = self
            .scores
            .iter()
            .zip(&self.directions)
            .filter_map(|(score, dir)| {
                let dir = (*dir)?;
                let loyalty = self
                    .goal
                    .map_or(0.0, |g| GOAL_LOYALTY * g.dot(dir).max(0.0));
                Some((score + loyalty, dir))
            })
            .collect();
        let Some(&(top, mode)) = biased.iter().max_by(|a, b| a.0.total_cmp(&b.0)) else {
            return pose.forward();
        };
        let mut sum = Vec3::ZERO;
        for (score, dir) in &biased {
            if dir.dot(mode) >= MODE_COS {
                sum = sum + *dir * ((score - top) / SCORE_TEMPERATURE).exp();
            }
        }
        sum.normalize()
    }
}

fn direction(pose: &Pose, (right, up): (f64, f64)) -> Vec3 {
    (pose.forward() + pose.right() * right + pose.up() * up).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::camera::Pose;
    use crate::fractal::formula::{FormulaId, Slot};
    use crate::fractal::stack::{CombineOp, HybridMode, SLOTS, Stack};
    use crate::fractal::vec3::Mat3;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn slerp_ends_at_both_poses_and_keeps_unit_length() {
        let a = Quat::IDENTITY;
        let b = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), 1.2);
        assert_eq!(slerp(a, b, 0.0), a);
        let end = slerp(a, b, 1.0);
        assert!(close(
            end.rotate(Vec3::new(0.0, 0.0, 1.0)),
            b.rotate(Vec3::new(0.0, 0.0, 1.0))
        ));
        let mid = slerp(a, b, 0.5);
        let expect = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.6);
        assert!(close(
            mid.rotate(Vec3::new(0.0, 0.0, 1.0)),
            expect.rotate(Vec3::new(0.0, 0.0, 1.0))
        ));
    }

    #[test]
    fn recall_arrives_exactly_and_eases() {
        let from = Pose::default();
        let to = Pose {
            position: Vec3::new(1.0, 2.0, 3.0),
            orientation: Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), 0.5),
        };
        let mut recall = Recall::new((from, 1.0), (to, 100.0), 1.0);
        let early = recall.step(0.1);
        assert!(!early.arrived);
        let covered =
            (early.pose.position - from.position).length() / (to.position - from.position).length();
        assert!(covered < 0.1, "eased start covered {covered}");
        let halfway = recall.step(0.4);
        assert!(
            (halfway.scale - 10.0).abs() < 1e-9,
            "geometric midpoint {}",
            halfway.scale
        );
        let last = recall.step(1.0);
        assert!(last.arrived);
        assert_eq!(last.pose, to);
        assert_eq!(last.scale, 100.0);
    }

    #[test]
    fn a_location_without_a_scale_keeps_the_current_one() {
        let mut recall = Recall::new((Pose::default(), 3.0), (Pose::default(), 0.0), 0.0);
        assert_eq!(recall.step(0.1).scale, 3.0);
    }

    fn default_box() -> Stack {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = Slot {
            formula: FormulaId::Box,
            mode: 0,
            count: 1,
            params: [2.2, 0.5, 1.0, 1.0],
            rotation: Mat3::IDENTITY,
        };
        Stack {
            slots,
            hybrid: HybridMode::Alternate,
            repeat_from: 0,
            max_iterations: 12,
            bailout: 100.0,
            julia: None,
            combine_split: 3,
            combine_op: CombineOp::Min,
            combine_width: 0.0,
        }
    }

    fn enclosure(stack: &Stack, p: Vec3) -> f64 {
        let d = stack.sample(p, 0.0).distance;
        let hits = probe_directions()
            .iter()
            .filter(|dir| {
                crate::fractal::camera::sphere_trace(stack, p, **dir, 64, 8.0 * d) < 8.0 * d * 0.99
            })
            .count();
        hits as f64 / 14.0
    }

    #[test]
    fn find_inside_lands_in_open_space_enclosed_by_structure() {
        let stack = default_box();
        let Inside {
            pose,
            scale,
            enclosed: found,
        } = find_inside(&stack, &[(Vec3::ZERO, 4.0)]).expect("a place inside");
        assert!(found, "enclosed");
        let sample = stack.sample(pose.position, 0.0);
        assert!(sample.escaped, "inside the solid at {:?}", pose.position);
        assert!(sample.distance > 0.0 && (scale - sample.distance).abs() < 1e-12);
        let enclosed = enclosure(&stack, pose.position);
        assert!(enclosed >= 0.6, "enclosure {enclosed}");
        // It looks down a way that ends in structure.
        let ahead = crate::fractal::camera::sphere_trace(
            &stack,
            pose.position,
            pose.forward(),
            64,
            8.0 * scale,
        );
        assert!(ahead < 8.0 * scale * 0.99, "looks into the void");
        assert!(pose.up().y > 0.0, "upside down");
    }

    /// Pseudo-Kleinian orbits never escape; the space between the spheres
    /// is still open space to find.
    #[test]
    fn find_inside_works_for_a_kleinian_stack() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = Slot {
            formula: FormulaId::PseudoKleinian,
            mode: 0,
            count: 1,
            params: [0.92, 1.0, 0.0, 0.0],
            rotation: Mat3::IDENTITY,
        };
        let stack = Stack {
            slots,
            max_iterations: 10,
            ..default_box()
        };
        let inside = find_inside(&stack, &[(Vec3::ZERO, 4.0)]).expect("a place inside");
        assert!(!stack.is_solid(inside.pose.position));
        assert!(inside.scale > 1e-6);
        assert!(inside.enclosed);
    }

    /// A Mandelbulb is a solid lump with no rooms: Find Inside still offers
    /// the most open point it found, but says nothing encloses it.
    #[test]
    fn find_inside_says_when_nothing_is_enclosed() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = Slot {
            formula: FormulaId::Mandelbulb,
            mode: 0,
            count: 1,
            params: [8.0, 1.0, 0.0, 0.0],
            rotation: Mat3::IDENTITY,
        };
        let stack = Stack {
            slots,
            max_iterations: 10,
            ..default_box()
        };
        let inside = find_inside(&stack, &[(Vec3::ZERO, 4.0)]).expect("an open point");
        assert!(!inside.enclosed);
    }

    #[test]
    fn find_inside_is_repeatable() {
        let stack = default_box();
        let balls = [(Vec3::ZERO, 4.0)];
        assert_eq!(find_inside(&stack, &balls), find_inside(&stack, &balls));
    }

    fn settle(stack: &Stack, pose: &Pose) -> Steering {
        let distance = stack.sample(pose.position, 0.0).distance;
        let mut autopilot = Autopilot::default();
        let mut steering = Steering::default();
        // Every probe scored, then long enough for the lags to settle.
        for _ in 0..PROBES + 240 {
            steering = autopilot.step(stack, pose, distance, 1.0 / 60.0);
        }
        steering
    }

    /// Nose against the box's face: every probe straight ahead meets the
    /// wall at once, so the autopilot turns.
    #[test]
    fn autopilot_turns_away_from_a_wall() {
        let stack = default_box();
        let near = crate::fractal::camera::sphere_trace(
            &stack,
            Vec3::new(0.0, 0.0, -12.0),
            Vec3::new(0.0, 0.0, 1.0),
            200,
            20.0,
        );
        let pose = Pose {
            position: Vec3::new(0.0, 0.0, -12.0 + near * 0.99),
            orientation: Quat::IDENTITY,
        };
        let steering = settle(&stack, &pose);
        assert!(
            steering.yaw_rate.abs() + steering.pitch_rate.abs() > 0.1,
            "{steering:?}"
        );
    }

    /// Far out, with the box ahead and sky all round: structure wins over
    /// empty sky, so it keeps heading in.
    #[test]
    fn autopilot_keeps_structure_in_view() {
        let stack = default_box();
        let pose = Pose {
            position: Vec3::new(0.0, 0.0, -30.0),
            orientation: Quat::IDENTITY,
        };
        let steering = settle(&stack, &pose);
        assert!(steering.confidence > 0.3);
        assert!(
            steering.yaw_rate.abs() < 0.6 && steering.pitch_rate.abs() < 0.6,
            "{steering:?}"
        );
    }
}
