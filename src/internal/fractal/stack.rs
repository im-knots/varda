//! A formula stack: six slots, the schedule they run in, and the distance
//! estimate at a point.

use super::formula::{self, FormulaId, Orbit, Slot};
use super::vec3::Vec3;

/// Slots in a stack, matching MB3D's six formula tabs.
pub const SLOTS: usize = 6;

/// How the slots combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HybridMode {
    /// One chain: slots run in order, each `count` times, then repeat from
    /// `repeat_from`.
    Alternate = 0,
    /// Two chains, split at `combine_split`, evaluated at the same point and
    /// combined by `combine_op`.
    Combine = 1,
}

/// How two chains' distances combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombineOp {
    Min = 0,
    Max = 1,
    /// Part 2 with part 1 carved out.
    Subtract = 2,
    /// Union with a linear bevel of `combine_width`.
    Chamfer = 3,
    /// Union with a rounded fillet of `combine_width`.
    Fillet = 4,
}

impl CombineOp {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Max,
            2 => Self::Subtract,
            3 => Self::Chamfer,
            4 => Self::Fillet,
            _ => Self::Min,
        }
    }
}

/// The order slots run in: `prefix` once, then `cycle` repeated. Slot indices.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schedule {
    pub prefix: Vec<u8>,
    pub cycle: Vec<u8>,
}

impl Schedule {
    /// Expand the slots in `range`: every active slot once (`count` times),
    /// then from `repeat_from` on, forever.
    pub fn expand(
        slots: &[Slot; SLOTS],
        range: std::ops::Range<usize>,
        repeat_from: usize,
    ) -> Self {
        let run = |from: usize| -> Vec<u8> {
            (from..range.end)
                .filter(|&i| slots[i].is_active())
                .flat_map(|i| std::iter::repeat_n(i as u8, slots[i].count as usize))
                .collect()
        };
        let prefix = run(range.start);
        let mut cycle = run(repeat_from.clamp(range.start, range.end));
        if cycle.is_empty() {
            // Repeat-from past every active slot: keep cycling the whole part.
            cycle.clone_from(&prefix);
        }
        Self { prefix, cycle }
    }

    pub fn is_empty(&self) -> bool {
        self.prefix.is_empty()
    }

    /// The slot iteration `i` runs.
    pub fn slot_at(&self, i: usize) -> usize {
        let index = if i < self.prefix.len() {
            self.prefix[i]
        } else {
            self.cycle[(i - self.prefix.len()) % self.cycle.len()]
        };
        usize::from(index)
    }
}

/// How a chain's end state becomes a distance. The shader reads the
/// discriminant as a code: 0 linear, 1 power, 2 Kleinian, 3 Jacobian.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DistanceKind {
    /// `|z| / dr`, for conformal fold stacks.
    Linear,
    /// `0.5 ln|z| |z| / dr`, when a Mandelbulb slot runs.
    Power,
    /// Distance to the plane `z = c` over `dr`, when the chain ends in a
    /// Pseudo-Kleinian slot.
    Kleinian { plane_z: f64 },
    /// `|z|` over the Frobenius norm of the orbit's Jacobian, when a linear
    /// chain has a step that is not conformal. The norm bounds the stretch
    /// in every direction, so the estimate stays below the distance where
    /// `|z| / |grad |z||` would overshoot.
    Jacobian,
}

impl DistanceKind {
    /// `(code, Kleinian plane z)` as the shader reads them.
    pub fn code(self) -> (f32, f32) {
        match self {
            Self::Linear => (0.0, 0.0),
            Self::Power => (1.0, 0.0),
            Self::Kleinian { plane_z } => (2.0, plane_z as f32),
            Self::Jacobian => (3.0, 0.0),
        }
    }
}

/// A whole stack's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    pub slots: [Slot; SLOTS],
    pub hybrid: HybridMode,
    /// First slot of the repeating cycle (0-based). Slots before it run once.
    pub repeat_from: usize,
    /// Slot visits before the orbit counts as bounded.
    pub max_iterations: u32,
    pub bailout: f64,
    /// The seed when set, instead of the sample point.
    pub julia: Option<Vec3>,
    /// First slot of part 2 in `Combine` mode.
    pub combine_split: usize,
    pub combine_op: CombineOp,
    /// Chamfer and fillet width, in pixel footprints at the sample.
    pub combine_width: f64,
}

/// What the stack says about one point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub distance: f64,
    /// Escape iteration with a fractional part, or the iteration count when
    /// the orbit stayed bounded.
    pub smooth_iteration: f64,
    /// Minimum `|z|^2` over the orbit.
    pub trap: f64,
    pub log2_dr: f64,
    /// Which chain decided the distance: 0, or 1 for part 2 of a combine.
    pub part: u8,
    pub escaped: bool,
}

/// A Kleinian chain's distance estimate below which a point counts as solid.
const KLEINIAN_SOLID: f64 = 1e-6;

impl Stack {
    /// The schedules of part 1 and part 2. Part 2 is empty in `Alternate`
    /// mode. In `Combine` mode, `repeat_from` applies to part 1, and part 2
    /// cycles all of its slots.
    pub fn schedules(&self) -> [Schedule; 2] {
        match self.hybrid {
            HybridMode::Alternate => [
                Schedule::expand(&self.slots, 0..SLOTS, self.repeat_from),
                Schedule::default(),
            ],
            HybridMode::Combine => {
                let split = self.combine_split.clamp(1, SLOTS - 1);
                [
                    Schedule::expand(&self.slots, 0..split, self.repeat_from),
                    Schedule::expand(&self.slots, split..SLOTS, split),
                ]
            }
        }
    }

    /// How each part's end state becomes a distance, for the shader.
    pub fn distance_kinds(&self) -> [DistanceKind; 2] {
        let [first, second] = self.schedules();
        [self.distance_kind(&first), self.distance_kind(&second)]
    }

    fn distance_kind(&self, schedule: &Schedule) -> DistanceKind {
        let uses = |id: FormulaId| {
            schedule
                .prefix
                .iter()
                .any(|&i| self.slots[usize::from(i)].formula == id)
        };
        if uses(FormulaId::Mandelbulb) || uses(FormulaId::MsltoeSym4) {
            return DistanceKind::Power;
        }
        match schedule.cycle.last().map(|&i| &self.slots[usize::from(i)]) {
            Some(last) if last.formula == FormulaId::PseudoKleinian => DistanceKind::Kleinian {
                plane_z: last.params[2],
            },
            _ => {
                // A bend in a slot that runs once cannot compound, so its
                // scalar bound serves.
                let bends = schedule
                    .cycle
                    .iter()
                    .any(|&i| !self.slots[usize::from(i)].is_conformal());
                if bends {
                    DistanceKind::Jacobian
                } else {
                    DistanceKind::Linear
                }
            }
        }
    }

    /// Run one chain from `p`.
    fn run_chain(&self, schedule: &Schedule, p: Vec3) -> Sample {
        let mut seed = self.julia.unwrap_or(p);
        let mut orbit = Orbit::start(p);
        if self.julia.is_some() {
            orbit.seed_dr = 0.0;
            orbit.seed_jac = [Vec3::ZERO; 3];
        }
        let bail2 = self.bailout * self.bailout;
        let mut previous_r2 = orbit.z.dot(orbit.z);
        let mut escaped = false;
        let mut iterations = 0u32;
        let kind = self.distance_kind(schedule);
        // Leading space steps move the point and the seed alike, so after them
        // the Jacobian restarts from the identity and their stretch bound, the
        // `dr` so far, becomes a factor. The shader then need not carry the
        // seed's Jacobian.
        let leading = if kind == DistanceKind::Jacobian && self.julia.is_none() {
            schedule
                .prefix
                .iter()
                .take_while(|&&i| self.slots[usize::from(i)].moves_seed())
                .count()
        } else {
            0
        };
        let mut lead_stretch = 1.0;
        if !schedule.is_empty() {
            while iterations < self.max_iterations {
                let slot = &self.slots[schedule.slot_at(iterations as usize)];
                previous_r2 = orbit.z.dot(orbit.z);
                formula::apply(slot, &mut orbit, &mut seed);
                iterations += 1;
                if iterations as usize == leading {
                    lead_stretch = orbit.dr.abs();
                    orbit.jac = formula::IDENTITY;
                    orbit.seed_jac = formula::IDENTITY;
                }
                if orbit.z.dot(orbit.z) > bail2 {
                    escaped = true;
                    break;
                }
            }
        }
        let r = orbit.z.length();
        let dr = if kind == DistanceKind::Jacobian {
            frobenius(&orbit.jac) * lead_stretch
        } else {
            orbit.dr.abs()
        }
        .max(1e-300);
        let distance = match kind {
            DistanceKind::Linear | DistanceKind::Jacobian => r / dr,
            DistanceKind::Power => 0.5 * r.max(1e-300).ln() * r / dr,
            DistanceKind::Kleinian { plane_z } => 0.5 * (orbit.z.z - plane_z).abs() / dr,
        };
        Sample {
            distance,
            smooth_iteration: smooth_iteration(
                iterations,
                escaped,
                previous_r2,
                r * r,
                self.bailout,
            ),
            trap: orbit.trap,
            log2_dr: dr.log2(),
            part: 0,
            escaped,
        }
    }

    /// The distance estimate and coloring values at `p`. `footprint` is the
    /// world size of one pixel at `p`, which scales the combine width.
    /// Whether `p` lies in the solid. An escape-time chain is solid where
    /// the orbit stays bounded. A Kleinian chain never escapes, so its solid
    /// is where the distance estimate vanishes.
    pub fn is_solid(&self, p: Vec3) -> bool {
        let sample = self.sample(p, 0.0);
        match self.distance_kinds()[usize::from(sample.part)] {
            DistanceKind::Kleinian { .. } => sample.distance <= KLEINIAN_SOLID,
            DistanceKind::Linear | DistanceKind::Power | DistanceKind::Jacobian => !sample.escaped,
        }
    }

    pub fn sample(&self, p: Vec3, footprint: f64) -> Sample {
        let [first, second] = self.schedules();
        let a = self.run_chain(&first, p);
        if self.hybrid == HybridMode::Alternate {
            return a;
        }
        let mut b = self.run_chain(&second, p);
        b.part = 1;
        let width = self.combine_width * footprint;
        let distance = combine(self.combine_op, a.distance, b.distance, width);
        // Coloring follows the chain whose surface is nearer.
        let mut out = if b.distance.abs() < a.distance.abs() {
            b
        } else {
            a
        };
        out.distance = distance;
        out
    }
}

fn frobenius(columns: &formula::Columns) -> f64 {
    columns.iter().map(|c| c.dot(*c)).sum::<f64>().sqrt()
}

/// MB3D's power-free smooth iteration value, which works for hybrids.
fn smooth_iteration(
    iterations: u32,
    escaped: bool,
    previous_r2: f64,
    r2: f64,
    bailout: f64,
) -> f64 {
    let n = f64::from(iterations);
    if !escaped || previous_r2 <= 1.0 || bailout <= 1.0 {
        return n;
    }
    let d = (0.5 * r2.ln()).ln();
    let d_prev = (0.5 * previous_r2.ln()).ln();
    if (d - d_prev).abs() < 1e-12 {
        return n;
    }
    n + (bailout.ln().ln() - d) / (d - d_prev)
}

/// Combine two chains' distances. `width` is in world units.
pub fn combine(op: CombineOp, d1: f64, d2: f64, width: f64) -> f64 {
    match op {
        CombineOp::Min => d1.min(d2),
        CombineOp::Max => d1.max(d2),
        CombineOp::Subtract => (-d1).max(d2),
        CombineOp::Chamfer => (d2 - (width - d1).max(0.0)).min(d1 - (width - d2).max(0.0)),
        CombineOp::Fillet => {
            if width <= 0.0 {
                return d1.min(d2);
            }
            let r1 = (width - d1).max(0.0);
            let r2 = (width - d2).max(0.0);
            (d2 - r1 * (width - r2) / width).min(d1 - r2 * (width - r1) / width)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::formula::box_mode;
    use crate::fractal::vec3::Mat3;

    fn slot(formula: FormulaId, count: u32, params: [f64; 4]) -> Slot {
        Slot {
            formula,
            mode: 0,
            count,
            params,
            rotation: Mat3::IDENTITY,
        }
    }

    fn stack(slots: &[Slot; SLOTS]) -> Stack {
        Stack {
            slots: *slots,
            hybrid: HybridMode::Alternate,
            repeat_from: 0,
            max_iterations: 16,
            bailout: 100.0,
            julia: None,
            combine_split: 3,
            combine_op: CombineOp::Min,
            combine_width: 0.0,
        }
    }

    fn mandelbox() -> Slot {
        Slot {
            mode: box_mode::AMAZING,
            ..slot(FormulaId::Box, 1, [-1.5, 0.5, 1.0, 1.0])
        }
    }

    #[test]
    fn schedule_runs_every_slot_once_then_cycles_from_repeat_from() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = slot(FormulaId::Transform, 1, [1.0, 0.0, 0.0, 0.0]);
        slots[1] = slot(FormulaId::Box, 2, [2.0, 0.5, 1.0, 1.0]);
        slots[3] = slot(FormulaId::Menger, 1, [3.0, 1.0, 0.0, 0.0]);
        let schedule = Schedule::expand(&slots, 0..SLOTS, 1);
        assert_eq!(schedule.prefix, [0, 1, 1, 3]);
        assert_eq!(schedule.cycle, [1, 1, 3]);
        let visited: Vec<usize> = (0..10).map(|i| schedule.slot_at(i)).collect();
        assert_eq!(visited, [0, 1, 1, 3, 1, 1, 3, 1, 1, 3]);
    }

    #[test]
    fn repeat_from_past_every_active_slot_cycles_the_whole_part() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = slot(FormulaId::Box, 1, [2.0, 0.5, 1.0, 1.0]);
        let schedule = Schedule::expand(&slots, 0..SLOTS, 4);
        assert_eq!(schedule.cycle, [0]);
    }

    #[test]
    fn combine_splits_into_two_schedules() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = mandelbox();
        slots[4] = slot(FormulaId::Menger, 1, [3.0, 1.0, 0.0, 0.0]);
        let mut s = stack(&slots);
        s.hybrid = HybridMode::Combine;
        let [a, b] = s.schedules();
        assert_eq!(a.prefix, [0]);
        assert_eq!(b.prefix, [4]);
    }

    #[test]
    fn far_points_escape_and_estimate_a_sane_distance() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = mandelbox();
        let s = stack(&slots);
        let far = s.sample(Vec3::new(20.0, 0.0, 0.0), 0.0);
        assert!(far.escaped);
        assert!(
            far.distance > 5.0 && far.distance < 25.0,
            "distance {}",
            far.distance
        );
    }

    #[test]
    fn every_formula_gives_finite_values_across_space() {
        for id in FormulaId::ALL {
            let mut slots = [Slot::EMPTY; SLOTS];
            slots[0] = slot(
                id,
                1,
                match id {
                    FormulaId::Box => [-1.5, 0.5, 1.0, 1.0],
                    FormulaId::Menger => [3.0, 1.0, 0.0, 0.0],
                    FormulaId::Sierpinski | FormulaId::Kifs => [2.0, 1.0, 0.0, 1.0],
                    FormulaId::PseudoKleinian => [0.92, 1.0, 0.0, 0.0],
                    FormulaId::Kaliset => [1.0, 0.1, 0.0, 0.0],
                    FormulaId::Mandelbulb => [8.0, 1.0, 0.0, 0.0],
                    FormulaId::Transform => [1.1, 0.0, 0.0, 0.0],
                    FormulaId::Helispiral => [0.3, 0.2, 0.0, 0.0],
                    FormulaId::Gnarl => [0.1, 1.0, 1.0, 1.0],
                    FormulaId::Bulbox => [2.0, 0.6, -0.5, 1.0],
                    FormulaId::Inversion => [1.0, 0.3, 0.0, 0.0],
                    FormulaId::Polyfold => [5.0, 10.0, 0.1, 0.0],
                    FormulaId::Sine => [0.0, 1.5, 1.0, 0.0],
                    FormulaId::Reciprocal => [0.5, 0.0, 0.0, 0.0],
                    FormulaId::Repeat => [3.0, 0.0, 0.0, 0.0],
                    FormulaId::KochCube => [1.0, 1.0, 1.0, 0.0],
                    FormulaId::JCube => [0.414_213_56, 3.0, 1.0, 1.0],
                    FormulaId::LinCombine => [1.2, 0.9, 1.0, 0.0],
                    FormulaId::Rotate4D => [0.3, 0.6, 0.9, 0.0],
                    FormulaId::ABoxMod2 => [2.0, 0.5, 1.0, 1.5],
                    FormulaId::MsltoeSym4 => [1.0, 0.8, 1.0, 0.0],
                    FormulaId::Empty => [0.0; 4],
                },
            );
            let s = stack(&slots);
            for i in 0..64 {
                let t = f64::from(i) * 0.37;
                let p = Vec3::new(t.sin() * 2.0, (t * 1.3).cos() * 2.0, (t * 0.7).sin() * 2.0);
                let sample = s.sample(p, 0.001);
                assert!(
                    sample.distance.is_finite()
                        && sample.smooth_iteration.is_finite()
                        && sample.log2_dr.is_finite(),
                    "{id:?} at {p:?}: {sample:?}"
                );
            }
        }
    }

    #[test]
    fn julia_mode_uses_the_constant_seed() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = mandelbox();
        let mut s = stack(&slots);
        s.max_iterations = 1;
        s.bailout = 1e9;
        let julia = Vec3::new(0.3, -0.2, 0.1);
        s.julia = Some(julia);
        let p = Vec3::new(0.2, 0.1, 0.0);
        let seeded = s.sample(p, 0.0);
        s.julia = None;
        let mandel = s.sample(p, 0.0);
        assert!((seeded.distance - mandel.distance).abs() > 1e-6);
    }

    #[test]
    fn combine_min_is_the_nearer_chain() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = mandelbox();
        slots[3] = slot(FormulaId::Menger, 1, [3.0, 1.0, 0.0, 0.0]);
        let mut combined = stack(&slots);
        combined.hybrid = HybridMode::Combine;
        let mut box_only = stack(&[
            mandelbox(),
            Slot::EMPTY,
            Slot::EMPTY,
            Slot::EMPTY,
            Slot::EMPTY,
            Slot::EMPTY,
        ]);
        box_only.hybrid = HybridMode::Alternate;
        let mut menger_slots = [Slot::EMPTY; SLOTS];
        menger_slots[0] = slots[3];
        let menger_only = stack(&menger_slots);
        for p in [Vec3::new(1.5, 0.3, 0.2), Vec3::new(-0.4, 2.0, 0.9)] {
            let expect = box_only
                .sample(p, 0.0)
                .distance
                .min(menger_only.sample(p, 0.0).distance);
            let got = combined.sample(p, 0.0).distance;
            assert!((got - expect).abs() < 1e-12, "{got} vs {expect}");
        }
    }

    #[test]
    fn chamfer_and_fillet_reduce_to_min_at_zero_width_and_bite_deeper_with_width() {
        for op in [CombineOp::Chamfer, CombineOp::Fillet] {
            assert_eq!(combine(op, 0.3, 0.5, 0.0), 0.3);
            assert!(combine(op, 0.3, 0.35, 1.0) < 0.3);
        }
        assert_eq!(combine(CombineOp::Subtract, 0.2, 0.5, 0.0), 0.5);
        assert_eq!(combine(CombineOp::Subtract, -0.2, 0.1, 0.0), 0.2);
    }

    #[test]
    fn a_bulb_slot_switches_to_the_power_estimate() {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = slot(FormulaId::Mandelbulb, 1, [8.0, 1.0, 0.0, 0.0]);
        let s = stack(&slots);
        let [schedule, _] = s.schedules();
        assert_eq!(s.distance_kind(&schedule), DistanceKind::Power);
    }

    fn twisted_temple() -> Stack {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = Slot {
            mode: box_mode::AMAZING,
            ..slot(FormulaId::Box, 1, [-1.8, 0.5, 1.0, 1.0])
        };
        slots[1] = slot(FormulaId::Helispiral, 1, [0.1, 0.2, 0.0, 0.0]);
        let mut s = stack(&slots);
        s.max_iterations = 14;
        s
    }

    #[test]
    fn non_conformal_chains_take_the_jacobian_distance() {
        let s = twisted_temple();
        assert_eq!(s.distance_kinds()[0], DistanceKind::Jacobian);
        assert_eq!(DistanceKind::Jacobian.code(), (3.0, 0.0));
        let mut plain = s.clone();
        plain.slots[1] = slot(FormulaId::Repeat, 1, [6.0, 0.0, 0.0, 0.0]);
        assert_eq!(plain.distance_kinds()[0], DistanceKind::Linear);
        plain.slots[1].params[2] = 0.2;
        assert_eq!(plain.distance_kinds()[0], DistanceKind::Jacobian);
    }

    /// `|z|` over the Frobenius norm of the Jacobian of `z`, both by central
    /// differences over the iterations `p` takes to escape.
    fn numeric_jacobian_distance(s: &Stack, p: Vec3) -> f64 {
        let [schedule, _] = s.schedules();
        let bail2 = s.bailout * s.bailout;
        let run = |q: Vec3, limit: Option<u32>| {
            let mut orbit = Orbit::start(q);
            let mut seed = q;
            let mut i = 0;
            while i < limit.unwrap_or(s.max_iterations) {
                formula::apply(
                    &s.slots[schedule.slot_at(i as usize)],
                    &mut orbit,
                    &mut seed,
                );
                i += 1;
                if limit.is_none() && orbit.z.dot(orbit.z) > bail2 {
                    break;
                }
            }
            (orbit.z, i)
        };
        let (z, iterations) = run(p, None);
        let h = 1e-7;
        let column = |a: Vec3| {
            (run(p + a * h, Some(iterations)).0 - run(p - a * h, Some(iterations)).0) * (0.5 / h)
        };
        let jac = [
            column(Vec3::new(1.0, 0.0, 0.0)),
            column(Vec3::new(0.0, 1.0, 0.0)),
            column(Vec3::new(0.0, 0.0, 1.0)),
        ];
        z.length() / frobenius(&jac)
    }

    #[test]
    fn the_jacobian_distance_matches_finite_differences() {
        let s = twisted_temple();
        let mut checked = 0;
        for i in 0..1000 {
            let t = f64::from(i);
            let p = Vec3::new((t * 0.37).sin(), (t * 0.61).cos(), (t * 0.23).sin()) * 2.5;
            let sample = s.sample(p, 0.0);
            if !sample.escaped {
                continue;
            }
            let numeric = numeric_jacobian_distance(&s, p);
            if !numeric.is_finite() {
                continue;
            }
            assert!(
                (sample.distance - numeric).abs() < 1e-3 * numeric.abs().max(1e-9),
                "at {p:?}: {} vs {numeric}",
                sample.distance
            );
            checked += 1;
        }
        assert!(checked > 50, "only {checked} points checked");
    }

    /// Repeat with a warp in slot 1, run once, then the -1.8 Box cycle.
    fn warped_temple() -> Stack {
        let mut slots = [Slot::EMPTY; SLOTS];
        slots[0] = slot(FormulaId::Repeat, 1, [6.0, 0.0, 0.3, 0.5]);
        slots[1] = Slot {
            mode: box_mode::AMAZING,
            ..slot(FormulaId::Box, 1, [-1.8, 0.5, 1.0, 1.0])
        };
        let mut s = stack(&slots);
        s.repeat_from = 1;
        s.max_iterations = 14;
        s
    }

    #[test]
    fn a_bend_that_runs_once_keeps_the_scalar_distance() {
        let mut s = warped_temple();
        assert_eq!(s.distance_kinds()[0], DistanceKind::Linear);
        s.repeat_from = 0;
        assert_eq!(s.distance_kinds()[0], DistanceKind::Jacobian);
    }

    /// `|z| / |grad |z||` by central differences over the iterations `p`
    /// takes to escape.
    fn numeric_gradient_distance(s: &Stack, p: Vec3) -> f64 {
        let [schedule, _] = s.schedules();
        let bail2 = s.bailout * s.bailout;
        let run = |q: Vec3, limit: Option<u32>| {
            let mut orbit = Orbit::start(q);
            let mut seed = q;
            let mut i = 0;
            while i < limit.unwrap_or(s.max_iterations) {
                formula::apply(
                    &s.slots[schedule.slot_at(i as usize)],
                    &mut orbit,
                    &mut seed,
                );
                i += 1;
                if limit.is_none() && orbit.z.dot(orbit.z) > bail2 {
                    break;
                }
            }
            (orbit.z.length(), i)
        };
        let (r, iterations) = run(p, None);
        let h = 1e-7;
        let axis = |a: Vec3| {
            (run(p + a * h, Some(iterations)).0 - run(p - a * h, Some(iterations)).0) / (2.0 * h)
        };
        let grad = Vec3::new(
            axis(Vec3::new(1.0, 0.0, 0.0)),
            axis(Vec3::new(0.0, 1.0, 0.0)),
            axis(Vec3::new(0.0, 0.0, 1.0)),
        );
        r / grad.length()
    }

    #[test]
    fn the_scalar_distance_counts_the_seeds_stretch() {
        // `dr` bounds the stretch, so `|z| / dr` never exceeds the first-order
        // distance. That fails when a warped seed is counted as unstretched.
        let s = warped_temple();
        let mut checked = 0;
        for i in 0..3000 {
            let t = f64::from(i);
            let p = Vec3::new((t * 0.37).sin(), (t * 0.61).cos(), (t * 0.23).sin()) * 9.0;
            let sample = s.sample(p, 0.0);
            if !sample.escaped {
                continue;
            }
            let numeric = numeric_gradient_distance(&s, p);
            if !numeric.is_finite() {
                continue;
            }
            assert!(
                sample.distance <= numeric * 1.001,
                "at {p:?}: scalar {} over first-order {numeric}",
                sample.distance
            );
            checked += 1;
        }
        assert!(checked > 50, "only {checked} points checked");
    }

    #[test]
    fn a_leading_space_step_folds_into_a_scalar_and_stays_safe() {
        // Warped Repeat once, then Box and Gnarl repeating. The Jacobian is
        // restarted after the Repeat and its stretch bound carried as a
        // factor, so the distance never exceeds the exact Frobenius distance,
        // and typically loses little more than that bound.
        let mut s = warped_temple();
        s.slots[2] = slot(FormulaId::Gnarl, 1, [0.15, 1.0, 1.0, 1.0]);
        assert_eq!(s.distance_kinds()[0], DistanceKind::Jacobian);
        let bound = formula::quasi_warp_stretch(6.0, 0.3);
        let mut ratios = Vec::new();
        for i in 0..3000 {
            let t = f64::from(i);
            let p = Vec3::new((t * 0.37).sin(), (t * 0.61).cos(), (t * 0.23).sin()) * 9.0;
            let sample = s.sample(p, 0.0);
            if !sample.escaped {
                continue;
            }
            let exact = numeric_jacobian_distance(&s, p);
            if !exact.is_finite() {
                continue;
            }
            assert!(
                sample.distance <= exact * 1.001,
                "at {p:?}: {} over exact {exact}",
                sample.distance
            );
            ratios.push(sample.distance / exact);
        }
        assert!(ratios.len() > 50, "only {} points checked", ratios.len());
        ratios.sort_by(f64::total_cmp);
        let median = ratios[ratios.len() / 2];
        assert!(median * bound > 0.8, "median {median} with bound {bound}");
    }
}
