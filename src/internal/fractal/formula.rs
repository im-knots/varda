//! The formula ids a slot can hold, and one iteration of each.
//!
//! Every formula is a fixed sequence of primitives: plane reflections, box
//! folds, conditional swaps, radial folds, affine maps and seed adds, plus the
//! twist and sine warp of the two symmetry breakers. The shader
//! (`shaders/fractal_explorer.fs`) implements the same sequences; parameters
//! mean the same thing in both.

use super::vec3::{Mat3, Vec3};

/// Formula ids, as the `slotN_formula` parameter stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormulaId {
    Empty = 0,
    Box = 1,
    Menger = 2,
    Sierpinski = 3,
    Kifs = 4,
    PseudoKleinian = 5,
    Kaliset = 6,
    Mandelbulb = 7,
    Transform = 8,
    Helispiral = 9,
    Gnarl = 10,
    Bulbox = 11,
    Inversion = 12,
    Polyfold = 13,
    Sine = 14,
    Reciprocal = 15,
    Repeat = 16,
    KochCube = 17,
    JCube = 18,
    LinCombine = 19,
    Rotate4D = 20,
    ABoxMod2 = 21,
    MsltoeSym4 = 22,
}

impl FormulaId {
    pub const ALL: [Self; 22] = [
        Self::Box,
        Self::Menger,
        Self::Sierpinski,
        Self::Kifs,
        Self::PseudoKleinian,
        Self::Kaliset,
        Self::Mandelbulb,
        Self::Transform,
        Self::Helispiral,
        Self::Gnarl,
        Self::Bulbox,
        Self::Inversion,
        Self::Polyfold,
        Self::Sine,
        Self::Reciprocal,
        Self::Repeat,
        Self::KochCube,
        Self::JCube,
        Self::LinCombine,
        Self::Rotate4D,
        Self::ABoxMod2,
        Self::MsltoeSym4,
    ];

    /// The id a parameter value names. Unknown values are an empty slot.
    pub fn from_index(index: i32) -> Self {
        Self::ALL
            .into_iter()
            .find(|id| *id as i32 == index)
            .unwrap_or(Self::Empty)
    }

    /// Whether the formula adds the seed (the sample point, or the Julia
    /// constant) each iteration.
    pub fn adds_seed(self) -> bool {
        matches!(
            self,
            Self::Box | Self::Kaliset | Self::Mandelbulb | Self::Bulbox
        )
    }
}

/// `Box` fold and radial variants, as `slotN_mode` stores them.
pub mod box_mode {
    /// Tglad fold on all axes, sphere radial fold (Amazing Box, Mandelbox).
    pub const AMAZING: i32 = 0;
    /// Fold on x and y only, seed components x and y swapped (Amazing Surf).
    pub const SURF: i32 = 1;
    /// Amazing Surf with the radial fold on the xy cylinder radius.
    pub const SURF_CYLINDER: i32 = 2;
    /// `z = fold - |z|` (`ABoxModKali`).
    pub const MOD_KALI: i32 = 3;
    /// `z = |z| + fold` (Kalibox).
    pub const KALIBOX: i32 = 4;
}

/// `Bulbox` variants, as `slotN_mode` stores them.
pub mod bulbox_mode {
    /// Outside radius 1 the point is only scaled and seeded: the boxy part.
    pub const BOX: i32 = 0;
    /// The blend toward the negative power continues outside radius 1.
    pub const NO_BOX: i32 = 1;
}

/// `Inversion` variants, as `slotN_mode` stores them.
pub mod inversion_mode {
    /// Invert the point.
    pub const POINT: i32 = 0;
    /// Invert the point and the seed, which inverts the whole space the
    /// following slots see (MB3D `_SphereInvC`).
    pub const SPACE: i32 = 1;
}

/// The component `Sine` and `Reciprocal` act on, from `slotN_mode`.
/// `Sine` starts at y, so its first variant is MB3D's `_SinY`.
fn axis_of(formula: FormulaId, mode: i32) -> usize {
    let order: [usize; 3] = if formula == FormulaId::Sine {
        [1, 0, 2]
    } else {
        [0, 1, 2]
    };
    order[usize::try_from(mode).unwrap_or(0).min(2)]
}

fn component(v: Vec3, axis: usize) -> f64 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

fn with_component(v: Vec3, axis: usize, value: f64) -> Vec3 {
    match axis {
        0 => Vec3::new(value, v.y, v.z),
        1 => Vec3::new(v.x, value, v.z),
        _ => Vec3::new(v.x, v.y, value),
    }
}

/// `Repeat` variants, as `slotN_mode` stores them.
pub mod repeat_mode {
    /// Every other cell mirrored, so neighbors meet without a seam.
    pub const MIRROR: i32 = 0;
    /// Plain copies; cell borders show.
    pub const PLAIN: i32 = 1;
}

/// One axis of `Repeat`: the cell `a` falls in, clamped to `count` copies
/// each way when `count` is positive, and the position within it.
fn repeat_axis(a: f64, size: f64, count: f64, mirror: bool) -> f64 {
    (a - repeat_cell(a, size, count) * size) * repeat_sign(a, size, count, mirror)
}

fn repeat_cell(a: f64, size: f64, count: f64) -> f64 {
    let cell = (a / size + 0.5).floor();
    if count >= 1.0 {
        let limit = count.floor();
        cell.clamp(-limit, limit)
    } else {
        cell
    }
}

/// -1 in a mirrored cell, else 1: the derivative of [`repeat_axis`].
fn repeat_sign(a: f64, size: f64, count: f64, mirror: bool) -> f64 {
    // The cell is integer-valued; its parity decides the mirror.
    if mirror && (repeat_cell(a, size, count) as i64).rem_euclid(2) == 1 {
        -1.0
    } else {
        1.0
    }
}

/// `Repeat`'s warp: a smooth displacement by sines whose wavelengths, in
/// cells, are in irrational ratio (2 phi and 2 (1 + sqrt 2)), so the pattern
/// never repeats. Every cell is bent differently and no lattice line stays
/// straight, which hides the tiling's symmetry. Returns the moved point and
/// a bound on its stretch.
fn quasi_warp(v: Vec3, size: f64, amount: f64, phase: f64) -> (Vec3, f64) {
    if amount == 0.0 {
        return (v, 1.0);
    }
    let a = 0.5 * amount * size;
    let k1 = std::f64::consts::TAU / (size * 2.0 * PHI);
    let k2 = std::f64::consts::TAU / (size * 2.0 * (1.0 + std::f64::consts::SQRT_2));
    let p2 = 1.7 * phase;
    let moved = v + Vec3::new(
        a * ((k1 * v.y + phase).sin() + (k2 * v.z + p2).sin()),
        a * ((k1 * v.z + 2.3 + phase).sin() + (k2 * v.x + 0.6 + p2).sin()),
        a * ((k1 * v.x + 4.1 + phase).sin() + (k2 * v.y + 3.3 + p2).sin()),
    );
    (moved, quasi_warp_stretch(size, amount))
}

/// A bound on [`quasi_warp`]'s stretch, the same everywhere.
pub(crate) fn quasi_warp_stretch(size: f64, amount: f64) -> f64 {
    let a = 0.5 * amount * size;
    let k1 = std::f64::consts::TAU / (size * 2.0 * PHI);
    let k2 = std::f64::consts::TAU / (size * 2.0 * (1.0 + std::f64::consts::SQRT_2));
    1.0 + 2.0 * a.abs() * (k1 + k2)
}

/// The rows of [`quasi_warp`]'s Jacobian at `v`.
fn quasi_warp_rows(v: Vec3, size: f64, amount: f64, phase: f64) -> Columns {
    if amount == 0.0 {
        return IDENTITY;
    }
    let a = 0.5 * amount * size;
    let k1 = std::f64::consts::TAU / (size * 2.0 * PHI);
    let k2 = std::f64::consts::TAU / (size * 2.0 * (1.0 + std::f64::consts::SQRT_2));
    let p2 = 1.7 * phase;
    [
        Vec3::new(
            1.0,
            a * k1 * (k1 * v.y + phase).cos(),
            a * k2 * (k2 * v.z + p2).cos(),
        ),
        Vec3::new(
            a * k2 * (k2 * v.x + 0.6 + p2).cos(),
            1.0,
            a * k1 * (k1 * v.z + 2.3 + phase).cos(),
        ),
        Vec3::new(
            a * k1 * (k1 * v.x + 4.1 + phase).cos(),
            a * k2 * (k2 * v.y + 3.3 + p2).cos(),
            1.0,
        ),
    ]
}

/// Three vectors: Jacobian columns, or a matrix's rows.
pub type Columns = [Vec3; 3];

pub(crate) const IDENTITY: Columns = [
    Vec3::new(1.0, 0.0, 0.0),
    Vec3::new(0.0, 1.0, 0.0),
    Vec3::new(0.0, 0.0, 1.0),
];

fn times(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}

/// The slope of `abs`, taken as 1 at zero (and at -0), as the shader does.
fn slope_sign(a: f64) -> f64 {
    if a >= 0.0 { 1.0 } else { -1.0 }
}

fn signs(v: Vec3) -> Vec3 {
    Vec3::new(slope_sign(v.x), slope_sign(v.y), slope_sign(v.z))
}

fn by_rows(rows: &Columns, v: Vec3) -> Vec3 {
    Vec3::new(rows[0].dot(v), rows[1].dot(v), rows[2].dot(v))
}

fn map_columns(columns: &mut Columns, f: impl Fn(Vec3) -> Vec3) {
    for c in columns.iter_mut() {
        *c = f(*c);
    }
}

/// 1 where `clamp(a, -limit, limit) * 2 - a` passes `a` through, -1 where it
/// reflects it.
fn fold_sign(a: f64, limit: f64) -> f64 {
    if a.abs() <= limit { 1.0 } else { -1.0 }
}

fn fold_signs(v: Vec3, limit: f64) -> Vec3 {
    Vec3::new(
        fold_sign(v.x, limit),
        fold_sign(v.y, limit),
        fold_sign(v.z, limit),
    )
}

/// The derivative of `z * scale / |q|^2`-style radial scaling: `m` times `c`
/// minus its change of `m` along `c`, where `m` falls as `1 / rr` and `rr`
/// is `q . q` (`q` is `z`, or its xy part for cylinders).
fn radial(c: Vec3, z: Vec3, q: Vec3, rr: f64) -> Vec3 {
    c - z * (2.0 * q.dot(c) / rr)
}

/// `Kifs` plane sets, as `slotN_mode` stores them.
pub mod kifs_mode {
    pub const TETRA: i32 = 0;
    pub const OCTA: i32 = 1;
    pub const ICOSA: i32 = 2;
}

/// One slot's formula and parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub formula: FormulaId,
    /// Formula variant (`box_mode`, `kifs_mode`); ignored by the others.
    pub mode: i32,
    /// Iterations this slot runs per visit.
    pub count: u32,
    /// `slotN_a` to `slotN_d`. Meaning per formula in [`apply`].
    pub params: [f64; 4],
    pub rotation: Mat3,
}

impl Slot {
    pub const EMPTY: Self = Self {
        formula: FormulaId::Empty,
        mode: 0,
        count: 0,
        params: [0.0; 4],
        rotation: Mat3::IDENTITY,
    };

    /// Whether this slot does anything.
    pub fn is_active(&self) -> bool {
        self.formula != FormulaId::Empty && self.count > 0
    }

    /// Whether the step scales every direction alike, so a scalar `dr`
    /// tracks it. The others need the Jacobian distance estimate.
    /// Whether the step moves the seed with the point: the steps that act on
    /// the whole space.
    pub fn moves_seed(&self) -> bool {
        self.formula == FormulaId::Repeat
            || (self.formula == FormulaId::Inversion && self.mode == inversion_mode::SPACE)
    }

    // Exact comparisons: any other value bends space.
    #[allow(clippy::float_cmp)]
    pub fn is_conformal(&self) -> bool {
        let [_, _, c, d] = self.params;
        let [a, b, _, _] = self.params;
        // A half turn through w maps two axes to minus themselves.
        let half_turns = |turns: f64| turns.fract() == 0.0;
        match self.formula {
            FormulaId::Helispiral
            | FormulaId::Gnarl
            | FormulaId::Sine
            | FormulaId::Reciprocal
            | FormulaId::Bulbox
            | FormulaId::ABoxMod2 => false,
            FormulaId::KochCube => b == 1.0,
            FormulaId::LinCombine => a.abs() == b.abs() && b.abs() == c.abs(),
            FormulaId::Rotate4D => half_turns(a) && half_turns(b) && half_turns(c),
            FormulaId::Kifs => d == 0.0 || d == 1.0,
            FormulaId::Repeat => c == 0.0,
            FormulaId::Inversion => self.mode != inversion_mode::SPACE,
            _ => true,
        }
    }
}

/// The iterated point and what the evaluator tracks along the orbit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    pub z: Vec3,
    /// Running derivative (scalar stretch).
    pub dr: f64,
    /// Scalar stretch of the seed, which steps that add the seed add to
    /// `dr`: 1, or 0 for a Julia seed, until a step moves the seed.
    pub seed_dr: f64,
    /// Minimum `|z|^2` over the orbit, for coloring.
    pub trap: f64,
    /// How `z` moves per unit move of the sample point along x, y and z.
    /// Not updated by Mandelbulb and msltoe Sym4, whose chains use the power
    /// DE.
    pub jac: Columns,
    /// The same for the seed: the identity, zero for a Julia seed, and
    /// changed by the steps that move the seed.
    pub seed_jac: Columns,
    /// A fourth coordinate, 0 at the start, that only Rotate 4D turns `z`
    /// through, and its gradient over the sample point.
    pub w: f64,
    pub jac_w: Vec3,
}

impl Orbit {
    pub fn start(p: Vec3) -> Self {
        Self {
            z: p,
            dr: 1.0,
            seed_dr: 1.0,
            trap: p.dot(p),
            jac: IDENTITY,
            seed_jac: IDENTITY,
            w: 0.0,
            jac_w: Vec3::ZERO,
        }
    }
}

/// Reflect `z` across the plane through the origin with unit normal `n` when
/// it lies on the negative side, scaled by `intensity`. Returns the stretch
/// bound of the step.
fn plane_reflect(z: &mut Vec3, n: Vec3, intensity: f64, jac: &mut Columns) -> f64 {
    let h = z.dot(n);
    if h < 0.0 {
        *z = *z - n * (2.0 * h * intensity);
        map_columns(jac, |c| c - n * (2.0 * c.dot(n) * intensity));
        (1.0 - 2.0 * intensity).abs().max(1.0)
    } else {
        1.0
    }
}

/// Swaps when `a < b`, and says whether it did.
fn swap_if_less(a: &mut f64, b: &mut f64) -> bool {
    let swap = *a < *b;
    if swap {
        std::mem::swap(a, b);
    }
    swap
}

/// `(a, b) -> (-b, -a)` when `a + b < 0`, and says whether it did.
fn reflect_pair(a: &mut f64, b: &mut f64) -> bool {
    let reflect = *a + *b < 0.0;
    if reflect {
        let (na, nb) = (-*b, -*a);
        *a = na;
        *b = nb;
    }
    reflect
}

/// Swap components `i` and `j`, negated when `negate`.
fn exchanged(v: Vec3, first: usize, second: usize, negate: bool) -> Vec3 {
    let sign = if negate { -1.0 } else { 1.0 };
    let (left, right) = (component(v, first), component(v, second));
    with_component(with_component(v, first, sign * right), second, sign * left)
}

/// `x - step * sin(x_from + sin(alpha * (x_from + sin(beta * x_from))))`.
fn gnarl_axis(x: f64, from: f64, step: f64, alpha: f64, beta: f64) -> f64 {
    x - step * (from + (alpha * (from + (beta * from).sin())).sin()).sin()
}

/// How much [`gnarl_axis`] falls per unit of `from`.
fn gnarl_slope(from: f64, step: f64, alpha: f64, beta: f64) -> f64 {
    let inner = alpha * (from + (beta * from).sin());
    let u = from + inner.sin();
    step * u.cos() * (1.0 + inner.cos() * alpha * (1.0 + beta * (beta * from).cos()))
}

/// Smallest squared radius a radial fold divides by.
pub const MIN_RADIUS2: f64 = 1e-8;

/// The golden ratio, for the icosahedral fold.
const PHI: f64 = 1.618_033_988_749_895;

/// Run one iteration of `slot` on `orbit`, adding `seed` where the formula
/// adds one.
///
/// Parameters `[a, b, c, d]` per formula:
///
/// | Formula | a | b | c | d |
/// |---|---|---|---|---|
/// | Box | scale | min radius | fold limit | fixed radius |
/// | Menger | scale | center offset | | |
/// | Sierpinski | scale | offset | | |
/// | KIFS | scale | offset | abs first (> 0.5) | fold intensity |
/// | Pseudo-Kleinian | box size | inversion size | end plane z | |
/// | Kaliset | scale | radius offset | | |
/// | Mandelbulb | power | z multiplier | | |
/// | Transform | scale | offset x | offset y | offset z |
/// | Helispiral | twist per radius | twist per height | fixed twist | |
/// | Gnarl | step | alpha | beta | scale |
/// | Bulbox | scale | inner radius | inner scale | fold limit |
/// | Inversion | radius | center x | center y | center z |
/// | Polyfold | order | angle shift (degrees) | shift x | shift y |
/// | Sine | offset 1 | scale 1 | scale 2 | offset 2 |
/// | Reciprocal | limiter | | | |
/// | Repeat | cell size | copies each way (0: endless) | warp | warp phase |
///
/// `Inversion` in its `SPACE` variant and `Repeat` change `seed`, rotation
/// included: they act on the whole space.
pub fn apply(slot: &Slot, orbit: &mut Orbit, seed: &mut Vec3) {
    let seed_in = *seed;
    let [pa, pb, pc, pd] = slot.params;
    let rot = &slot.rotation;
    let z = &mut orbit.z;
    match slot.formula {
        FormulaId::Empty => {}
        FormulaId::Box => {
            let (scale, min_r, fold, fixed_r) = (pa, pb, pc, pd);
            let folds = match slot.mode {
                box_mode::SURF | box_mode::SURF_CYLINDER => {
                    let folds = Vec3::new(fold_sign(z.x, fold), fold_sign(z.y, fold), 1.0);
                    z.x = z.x.clamp(-fold, fold) * 2.0 - z.x;
                    z.y = z.y.clamp(-fold, fold) * 2.0 - z.y;
                    folds
                }
                box_mode::MOD_KALI => {
                    let folds = -signs(*z);
                    *z = Vec3::splat(fold) - z.abs();
                    folds
                }
                box_mode::KALIBOX => {
                    let folds = signs(*z);
                    *z = z.abs() + Vec3::splat(fold);
                    folds
                }
                _ => {
                    let folds = fold_signs(*z, fold);
                    *z = z.clamp_sym(fold) * 2.0 - *z;
                    folds
                }
            };
            let measured = if slot.mode == box_mode::SURF_CYLINDER {
                Vec3::new(z.x, z.y, 0.0)
            } else {
                *z
            };
            let rr = measured.dot(measured);
            // Floored so a zero min radius cannot divide by zero, and the
            // fixed radius never falls below the min radius.
            let min2 = (min_r * min_r).max(MIN_RADIUS2);
            let fixed2 = (fixed_r * fixed_r).max(min2);
            let m = scale * fixed2 / rr.clamp(min2, fixed2);
            let surf = matches!(slot.mode, box_mode::SURF | box_mode::SURF_CYLINDER);
            let seed = if surf {
                exchanged(seed_in, 0, 1, false)
            } else {
                seed_in
            };
            let inverting = rr > min2 && rr < fixed2;
            let folded = *z;
            for (column, seed_column) in orbit.jac.iter_mut().zip(orbit.seed_jac) {
                let mut moved = times(*column, folds);
                if inverting {
                    moved = radial(moved, folded, measured, rr);
                }
                let seed_column = if surf {
                    exchanged(seed_column, 0, 1, false)
                } else {
                    seed_column
                };
                *column = rot.apply(moved * m + seed_column);
            }
            *z = rot.apply(*z * m + seed);
            orbit.dr = orbit.dr * m.abs() + orbit.seed_dr;
        }
        FormulaId::Menger => {
            let (scale, offset) = (pa, pb);
            let abs = signs(*z);
            *z = z.abs();
            map_columns(&mut orbit.jac, |c| times(c, abs));
            for (i, j) in [(0, 1), (0, 2), (1, 2)] {
                let (mut a, mut b) = (component(*z, i), component(*z, j));
                if swap_if_less(&mut a, &mut b) {
                    *z = with_component(with_component(*z, i, a), j, b);
                    map_columns(&mut orbit.jac, |c| exchanged(c, i, j, false));
                }
            }
            *z = rot.apply(*z);
            let h = 0.5 * offset * (scale - 1.0) / scale;
            let fold_z = -slope_sign(z.z - h);
            z.z = h - (z.z - h).abs();
            z.x = scale * z.x - offset * (scale - 1.0);
            z.y = scale * z.y - offset * (scale - 1.0);
            z.z *= scale;
            map_columns(&mut orbit.jac, |c| {
                let c = rot.apply(c);
                Vec3::new(c.x, c.y, c.z * fold_z) * scale
            });
            orbit.dr *= scale.abs();
        }
        FormulaId::Sierpinski => {
            let (scale, offset) = (pa, pb);
            for (i, j) in [(0, 1), (0, 2), (1, 2)] {
                let (mut a, mut b) = (component(*z, i), component(*z, j));
                if reflect_pair(&mut a, &mut b) {
                    *z = with_component(with_component(*z, i, a), j, b);
                    map_columns(&mut orbit.jac, |c| exchanged(c, i, j, true));
                }
            }
            *z = rot.apply(*z);
            map_columns(&mut orbit.jac, |c| rot.apply(c) * scale);
            *z = *z * scale - Vec3::splat(offset * (scale - 1.0));
            orbit.dr *= scale.abs();
        }
        FormulaId::Kifs => {
            let (scale, offset, abs_first, intensity) = (pa, pb, pc, pd);
            let mut stretch = 1.0;
            match slot.mode {
                kifs_mode::OCTA => {
                    let abs = signs(*z);
                    *z = z.abs();
                    map_columns(&mut orbit.jac, |c| times(c, abs));
                    for n in [
                        Vec3::new(1.0, -1.0, 0.0),
                        Vec3::new(1.0, 0.0, -1.0),
                        Vec3::new(0.0, 1.0, -1.0),
                    ] {
                        stretch *= plane_reflect(z, n.normalize(), intensity, &mut orbit.jac);
                    }
                }
                kifs_mode::ICOSA => {
                    let abs = signs(*z);
                    *z = z.abs();
                    map_columns(&mut orbit.jac, |c| times(c, abs));
                    let n1 = Vec3::new(-1.0, PHI - 1.0, 1.0 / (PHI - 1.0)).normalize();
                    let n2 = Vec3::new(PHI - 1.0, 1.0 / (PHI - 1.0), -1.0).normalize();
                    let n3 = Vec3::new(1.0 / (PHI - 1.0), -1.0, PHI - 1.0).normalize();
                    for n in [n1, n2, n3, n2] {
                        stretch *= plane_reflect(z, n, intensity, &mut orbit.jac);
                    }
                }
                _ => {
                    if abs_first > 0.5 {
                        let abs = signs(*z);
                        *z = z.abs();
                        map_columns(&mut orbit.jac, |c| times(c, abs));
                    }
                    for n in [
                        Vec3::new(1.0, 1.0, 0.0),
                        Vec3::new(1.0, 0.0, 1.0),
                        Vec3::new(0.0, 1.0, 1.0),
                    ] {
                        stretch *= plane_reflect(z, n.normalize(), intensity, &mut orbit.jac);
                    }
                }
            }
            *z = rot.apply(*z);
            *z = *z * scale - Vec3::splat(offset * (scale - 1.0));
            map_columns(&mut orbit.jac, |c| rot.apply(c) * scale);
            orbit.dr *= scale.abs() * stretch;
        }
        FormulaId::PseudoKleinian => {
            let (box_size, size) = (pa, pb);
            let folds = fold_signs(*z, box_size);
            *z = z.clamp_sym(box_size) * 2.0 - *z;
            let rr = z.dot(*z);
            let k = (size / rr.max(MIN_RADIUS2)).max(1.0);
            let inverting = rr > MIN_RADIUS2 && size > rr;
            let folded = *z;
            map_columns(&mut orbit.jac, |c| {
                let d = times(c, folds);
                let d = if inverting {
                    radial(d, folded, folded, rr)
                } else {
                    d
                };
                rot.apply(d * k)
            });
            *z = rot.apply(*z * k);
            orbit.dr *= k;
        }
        FormulaId::Kaliset => {
            let (scale, offset) = (pa, pb);
            let abs = signs(*z);
            *z = z.abs();
            let denominator = z.dot(*z) + offset;
            let m = scale / denominator.max(MIN_RADIUS2);
            let folded = *z;
            for (column, seed_column) in orbit.jac.iter_mut().zip(orbit.seed_jac) {
                let moved = times(*column, abs);
                let moved = if denominator > MIN_RADIUS2 {
                    radial(moved, folded, folded, denominator)
                } else {
                    moved
                };
                *column = rot.apply(moved * m + seed_column);
            }
            *z = rot.apply(*z * m + seed_in);
            orbit.dr = orbit.dr * m.abs() + orbit.seed_dr;
        }
        FormulaId::KochCube => koch_cube(slot, orbit),
        FormulaId::JCube => jcube(slot, orbit),
        FormulaId::LinCombine => {
            let axes = Vec3::new(pa, pb, pc);
            *z = rot.apply(times(*z, axes));
            map_columns(&mut orbit.jac, |c| rot.apply(times(c, axes)));
            orbit.dr *= pa.abs().max(pb.abs()).max(pc.abs());
        }
        FormulaId::Rotate4D => rotate_4d(slot, orbit),
        FormulaId::ABoxMod2 => abox_mod2(slot, orbit, seed_in),
        FormulaId::MsltoeSym4 => msltoe_sym4(slot, orbit, seed_in),
        FormulaId::Mandelbulb => {
            let (power, z_mul) = (pa, pb);
            let r = z.length().max(1e-12);
            let theta = z.y.atan2(z.x) * power;
            let phi = (z.z / r).clamp(-1.0, 1.0).asin() * power;
            let rp = r.powf(power);
            orbit.dr = power * r.powf(power - 1.0) * orbit.dr + orbit.seed_dr;
            let bulb = Vec3::new(
                phi.cos() * theta.cos(),
                phi.cos() * theta.sin(),
                z_mul * phi.sin(),
            );
            *z = rot.apply(bulb * rp + seed_in);
        }
        FormulaId::Transform => {
            *z = rot.apply(*z) * pa + Vec3::new(pb, pc, pd);
            map_columns(&mut orbit.jac, |c| rot.apply(c) * pa);
            orbit.dr *= pa.abs();
        }
        FormulaId::Helispiral => {
            let (per_radius, per_height, fixed) = (pa, pb, pc);
            let rho = (z.x * z.x + z.y * z.y).sqrt();
            let angle = fixed + per_radius * rho + per_height * z.z;
            let (s, co) = angle.sin_cos();
            // The turn, plus the turn's own change along each column.
            let across = Vec3::new(-s * z.x - co * z.y, co * z.x - s * z.y, 0.0);
            let outward = if rho > 1e-12 {
                Vec3::new(z.x / rho, z.y / rho, 0.0)
            } else {
                Vec3::ZERO
            };
            let slope = outward * per_radius + Vec3::new(0.0, 0.0, per_height);
            map_columns(&mut orbit.jac, |c| {
                let turned = Vec3::new(co * c.x - s * c.y, s * c.x + co * c.y, c.z);
                rot.apply(turned + across * slope.dot(c))
            });
            *z = rot.apply(Vec3::new(co * z.x - s * z.y, s * z.x + co * z.y, z.z));
            orbit.dr *= 1.0 + rho * per_radius.hypot(per_height);
        }
        FormulaId::Bulbox => bulbox(slot, orbit, seed_in),
        FormulaId::Inversion => {
            let (radius, center) = (pa, Vec3::new(pb, pc, pd));
            let invert = |v: Vec3| {
                let d = v - center;
                center + d * (radius * radius / d.dot(d).max(MIN_RADIUS2))
            };
            let inverted = |columns: &mut Columns, v: Vec3| {
                let d = v - center;
                let dd = d.dot(d);
                let k = radius * radius / dd.max(MIN_RADIUS2);
                map_columns(columns, |c| {
                    if dd > MIN_RADIUS2 {
                        radial(c, d, d, dd) * k
                    } else {
                        c * k
                    }
                });
            };
            let d = *z - center;
            orbit.dr *= radius * radius / d.dot(d).max(MIN_RADIUS2);
            inverted(&mut orbit.jac, *z);
            map_columns(&mut orbit.jac, |c| rot.apply(c));
            *z = rot.apply(invert(*z));
            if slot.mode == inversion_mode::SPACE {
                let ds = seed_in - center;
                orbit.seed_dr *= radius * radius / ds.dot(ds).max(MIN_RADIUS2);
                inverted(&mut orbit.seed_jac, seed_in);
                map_columns(&mut orbit.seed_jac, |c| rot.apply(c));
                *seed = rot.apply(invert(seed_in));
            }
        }
        FormulaId::Polyfold => {
            let (folded, turn) = polyfold(*z, pa, pb, pc, pd);
            map_columns(&mut orbit.jac, |c| rot.apply(by_rows(&turn, c)));
            *z = rot.apply(folded);
        }
        FormulaId::Repeat => {
            // Machina's "mirror distance" and "chaos": the world tiled into
            // cells. The seed moves too, so every cell holds the same fractal.
            // Warping space first hides the tiling's symmetry.
            let size = pa.abs().max(1e-6);
            let mirror = slot.mode != repeat_mode::PLAIN;
            let tile = |v: Vec3| {
                let (v, stretch) = quasi_warp(v, size, pc, pd);
                let tiled = Vec3::new(
                    repeat_axis(v.x, size, pb, mirror),
                    repeat_axis(v.y, size, pb, mirror),
                    repeat_axis(v.z, size, pb, mirror),
                );
                (tiled, stretch)
            };
            // The warp's Jacobian, then each axis's mirror.
            let tile_columns = |columns: &mut Columns, v: Vec3| {
                let rows = quasi_warp_rows(v, size, pc, pd);
                let (w, _) = quasi_warp(v, size, pc, pd);
                let mirrors = Vec3::new(
                    repeat_sign(w.x, size, pb, mirror),
                    repeat_sign(w.y, size, pb, mirror),
                    repeat_sign(w.z, size, pb, mirror),
                );
                map_columns(columns, |c| times(by_rows(&rows, c), mirrors));
            };
            tile_columns(&mut orbit.jac, *z);
            map_columns(&mut orbit.jac, |c| rot.apply(c));
            tile_columns(&mut orbit.seed_jac, seed_in);
            map_columns(&mut orbit.seed_jac, |c| rot.apply(c));
            let (tiled, stretch) = tile(*z);
            *z = rot.apply(tiled);
            orbit.dr *= stretch;
            let (tiled_seed, seed_stretch) = tile(seed_in);
            orbit.seed_dr *= seed_stretch;
            *seed = rot.apply(tiled_seed);
        }
        FormulaId::Sine => {
            // MB3D `_SinY` and its siblings: one component through a sine.
            let axis = axis_of(slot.formula, slot.mode);
            let a = component(*z, axis);
            let slope = pb * pc * ((a - pa) * pb).cos();
            map_columns(&mut orbit.jac, |c| {
                rot.apply(with_component(c, axis, component(c, axis) * slope))
            });
            *z = rot.apply(with_component(*z, axis, ((a - pa) * pb).sin() * pc + pd));
            orbit.dr *= (pb * pc).abs().max(1.0);
        }
        FormulaId::Reciprocal => {
            // MB3D `_reciprocalX3`: continuous, and gentle on the DE.
            let axis = axis_of(slot.formula, slot.mode);
            let limiter = pa.abs().max(1e-3);
            let a = component(*z, axis);
            let bent = a.signum() * (1.0 / limiter - 1.0 / (a.abs() + limiter));
            let slope = 1.0 / ((a.abs() + limiter) * (a.abs() + limiter));
            map_columns(&mut orbit.jac, |c| {
                rot.apply(with_component(c, axis, component(c, axis) * slope))
            });
            *z = rot.apply(with_component(*z, axis, bent));
            orbit.dr *= (1.0 / ((a.abs() + limiter) * (a.abs() + limiter))).max(1.0);
        }
        FormulaId::Gnarl => {
            // A zero scale would collapse space to a point.
            let (step, alpha, beta) = (pa, pb, pc);
            let scale = if pd == 0.0 { 1.0 } else { pd };
            let p = *z;
            let warped = Vec3::new(
                gnarl_axis(p.x, p.z, step, alpha, beta),
                gnarl_axis(p.y, p.x, step, alpha, beta),
                gnarl_axis(p.z, p.y, step, alpha, beta),
            );
            let slopes = Vec3::new(
                gnarl_slope(p.z, step, alpha, beta),
                gnarl_slope(p.x, step, alpha, beta),
                gnarl_slope(p.y, step, alpha, beta),
            );
            map_columns(&mut orbit.jac, |c| {
                let d = c - times(slopes, Vec3::new(c.z, c.x, c.y));
                rot.apply(d * scale)
            });
            *z = rot.apply(warped * scale);
            orbit.dr *= scale.abs() * (1.0 + step.abs() * (1.0 + alpha.abs() * (1.0 + beta.abs())));
        }
    }
    orbit.trap = orbit.trap.min(orbit.z.dot(orbit.z));
}

/// Koch Cube (Luca G.N. 2011, MB3D `koch_cube.m3f`): three times the folded
/// point, sorted descending, a Koch-curve step in the XY plane with an XY
/// stretch, and a fold in z. The seed is never added. Params: post-scale,
/// XY stretch, Z fold, X add; Y and Z add are 0. The slot rotation runs
/// mid-map, where MB3D's own rotation does.
fn koch_cube(slot: &Slot, orbit: &mut Orbit) {
    let [post, stretch, fold, x_add] = slot.params;
    let stretch = if stretch == 0.0 { 1.0 } else { stretch };
    let z = &mut orbit.z;
    let abs = signs(*z) * 3.0;
    *z = z.abs() * 3.0;
    map_columns(&mut orbit.jac, |c| times(c, abs));
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let (mut a, mut b) = (component(*z, i), component(*z, j));
        if swap_if_less(&mut a, &mut b) {
            *z = with_component(with_component(*z, i, a), j, b);
            map_columns(&mut orbit.jac, |c| exchanged(c, i, j, false));
        }
    }
    z.x += x_add;
    *z = slot.rotation.apply(*z);
    map_columns(&mut orbit.jac, |c| slot.rotation.apply(c));
    let (near, far) = (3.0 - stretch, 3.0 + stretch);
    let fold_z = slope_sign(fold - z.z);
    z.z = fold - (fold - z.z).abs();
    let (x, y) = (z.x, z.y);
    let swapped = if x - near < y {
        z.x = x - near;
        z.y = y - near;
        false
    } else if x - far > y {
        z.x = x - far;
        false
    } else {
        z.x = y;
        z.y = x - far;
        true
    };
    z.x /= stretch;
    z.y /= stretch;
    *z = *z * post;
    map_columns(&mut orbit.jac, |c| {
        let c = Vec3::new(c.x, c.y, c.z * fold_z);
        let c = if swapped {
            exchanged(c, 0, 1, false)
        } else {
            c
        };
        Vec3::new(c.x / stretch, c.y / stretch, c.z) * post
    });
    orbit.dr *= 3.0 * post.abs() * (1.0 / stretch.abs()).max(1.0);
}

/// `JCube` (MB3D `JCube3.m3f`), a brute-force IFS for the Jerusalem cube,
/// decoded from its machine code: fold to the first octant, take the
/// smallest component first, then scale toward an edge cube by `s0 /
/// alpha` or toward a corner cube by `s0 = G - 1 + alpha`. Its author calls
/// it discontinuous. The seed is never added. Params: alpha, `GScale` (G),
/// edge center (0, c, c), corner center (d, d, d).
fn jcube(slot: &Slot, orbit: &mut Orbit) {
    let [alpha, g, edge_c, corner_c] = slot.params;
    let alpha = if alpha.abs() < 1e-6 { 1e-6 } else { alpha };
    let abs = signs(orbit.z);
    let p = orbit.z.abs();
    map_columns(&mut orbit.jac, |c| times(c, abs));
    // MB3D's partial sort: x' the smallest, y' = max(min(x, y), z),
    // z' = max(x, y).
    let lower = usize::from(p.x >= p.y);
    let upper = 1 - lower;
    let a = component(p, lower);
    let order = if a >= p.z {
        [2, lower, upper]
    } else {
        [lower, 2, upper]
    };
    let sorted = Vec3::new(
        component(p, order[0]),
        component(p, order[1]),
        component(p, order[2]),
    );
    map_columns(&mut orbit.jac, |c| {
        Vec3::new(
            component(c, order[0]),
            component(c, order[1]),
            component(c, order[2]),
        )
    });
    let corner_scale = g - 1.0 + alpha;
    let edge_scale = corner_scale / alpha;
    let (scale, center) =
        if sorted.x <= 1.0 / edge_scale && sorted.y >= sorted.x + 1.0 - g / edge_scale {
            (edge_scale, Vec3::new(0.0, edge_c, edge_c))
        } else {
            (corner_scale, Vec3::splat(corner_c))
        };
    orbit.z = slot.rotation.apply((sorted - center) * scale + center);
    map_columns(&mut orbit.jac, |c| slot.rotation.apply(c * scale));
    orbit.dr *= scale.abs();
}

/// Rotate 4D (MB3D `_Rotate4d.m3f`): `(x, y, z, w)` turned in the XW, YW and
/// ZW planes by the first three params in half turns (1 is 180 degrees),
/// then by the slot
/// rotation in the YZ, XZ and XY planes. An isometry on `(z, w)`.
fn rotate_4d(slot: &Slot, orbit: &mut Orbit) {
    let [xw, yw, zw, _] = slot.params;
    let mut point = [orbit.z.x, orbit.z.y, orbit.z.z, orbit.w];
    let mut columns: [[f64; 4]; 3] = std::array::from_fn(|j| {
        let c = orbit.jac[j];
        [c.x, c.y, c.z, component(orbit.jac_w, j)]
    });
    for (axis, turns) in [(0, xw), (1, yw), (2, zw)] {
        let (sin, cos) = (turns * std::f64::consts::PI).sin_cos();
        let turn = |v: &mut [f64; 4]| {
            let (a, w) = (v[axis], v[3]);
            v[axis] = a * cos - w * sin;
            v[3] = a * sin + w * cos;
        };
        turn(&mut point);
        for column in &mut columns {
            turn(column);
        }
    }
    orbit.z = slot.rotation.apply(Vec3::new(point[0], point[1], point[2]));
    orbit.w = point[3];
    for (j, column) in columns.iter().enumerate() {
        orbit.jac[j] = slot
            .rotation
            .apply(Vec3::new(column[0], column[1], column[2]));
    }
    orbit.jac_w = Vec3::new(columns[0][3], columns[1][3], columns[2][3]);
}

/// The cylinder half size `ABoxMod2` keeps at its author's default.
const ABOX_MOD2_HALF_SIZE: f64 = 0.5;

/// `ABoxMod2` (MB3D `ABoxMod2.m3f`): a box fold with its own fold for z, then
/// an inversion in a capped cylinder of half size 0.5 instead of a sphere,
/// so not conformal. Params: scale, min R, fold XY, fold Z.
fn abox_mod2(slot: &Slot, orbit: &mut Orbit, seed: Vec3) {
    let [scale, min_r, fold, fold_z] = slot.params;
    let z = orbit.z;
    let folds = Vec3::new(
        fold_sign(z.x, fold),
        fold_sign(z.y, fold),
        fold_sign(z.z, fold_z),
    );
    let folded = Vec3::new(
        z.x.clamp(-fold, fold) * 2.0 - z.x,
        z.y.clamp(-fold, fold) * 2.0 - z.y,
        z.z.clamp(-fold_z, fold_z) * 2.0 - z.z,
    );
    let cap = folded.z.abs() - ABOX_MOD2_HALF_SIZE;
    let rr = folded.x * folded.x + folded.y * folded.y + if cap > 0.0 { cap * cap } else { 0.0 };
    let min2 = (min_r * min_r).max(MIN_RADIUS2);
    let m = scale / rr.clamp(min2, 1.0);
    let inverting = rr > min2 && rr < 1.0;
    // The gradient of rr, which m falls with between min R and 1.
    let grad = Vec3::new(
        2.0 * folded.x,
        2.0 * folded.y,
        if cap > 0.0 {
            2.0 * cap * slope_sign(folded.z)
        } else {
            0.0
        },
    );
    for (column, seed_column) in orbit.jac.iter_mut().zip(orbit.seed_jac) {
        let moved = times(*column, folds);
        let moved = if inverting {
            moved - folded * (grad.dot(moved) / rr)
        } else {
            moved
        };
        *column = slot.rotation.apply(moved * m + seed_column);
    }
    orbit.z = slot.rotation.apply(folded * m + seed);
    orbit.dr = orbit.dr * m.abs() + orbit.seed_dr;
}

/// msltoe Sym4 (Msltoe, MB3D `MsltoeSym4.m3f`): conditional swaps and sign
/// flips that give the power-2 bulb a 4-fold symmetry, then the bulb plus
/// the seed. Params: XZ, XY and YZ sym-mul; one at or below 0 turns its
/// swap off. Chains with it use the power DE, so `J` is left alone.
fn msltoe_sym4(slot: &Slot, orbit: &mut Orbit, seed: Vec3) {
    let [xz, xy, yz, _] = slot.params;
    let mut v = orbit.z;
    let r = v.length();
    if v.x.abs() < v.z.abs() * xz {
        std::mem::swap(&mut v.x, &mut v.z);
    }
    if v.x.abs() < v.y.abs() * xy {
        std::mem::swap(&mut v.x, &mut v.y);
    }
    if v.y.abs() < v.z.abs() * yz {
        std::mem::swap(&mut v.y, &mut v.z);
    }
    if v.x * v.z < 0.0 {
        v.z = -v.z;
    }
    if v.x * v.y < 0.0 {
        v.y = -v.y;
    }
    let squared = Vec3::new(
        v.x * v.x - v.y * v.y - v.z * v.z,
        2.0 * v.x * v.y,
        2.0 * v.x * v.z,
    );
    orbit.z = slot.rotation.apply(squared + seed);
    orbit.dr = 2.0 * r * orbit.dr + orbit.seed_dr;
}

/// Polyfold Sym (Luca GN, MB3D `_PolyFold-sym.m3f`), as its machine code
/// runs: each of `order` sectors around the z axis is turned back onto the
/// first, rotated when its index is even and mirrored when odd, which keeps
/// even orders continuous. An isometry, so `dr` is untouched. Returns the
/// point and the rows of the step's Jacobian.
fn polyfold(
    point: Vec3,
    order: f64,
    shift_deg: f64,
    shift_x: f64,
    shift_y: f64,
) -> (Vec3, Columns) {
    let (px, py) = (point.x + shift_x, point.y + shift_y);
    let order = if order.abs() < 1e-9 { 1.0 } else { order };
    let shift = shift_deg.to_radians();
    let theta = py.atan2(px);
    let sector = ((theta + shift) * order / std::f64::consts::TAU).round_ties_even();
    let turn = shift - sector * std::f64::consts::TAU / order;
    let (sin, cos) = turn.sin_cos();
    let new_x = py * sin - px * cos;
    let turned_y = px * sin + py * cos;
    // `sector` is an integer-valued float; its parity decides the mirror.
    let mirror = if sector.rem_euclid(2.0) == 0.0 {
        -1.0
    } else {
        1.0
    };
    let rows = [
        Vec3::new(-cos, sin, 0.0),
        Vec3::new(mirror * sin, mirror * cos, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    (
        Vec3::new(new_x - shift_x, mirror * turned_y - shift_y, point.z),
        rows,
    )
}

/// Bulbox P-2 (Luca GN, MB3D `BulboxP-2.m3f`), as its machine code runs:
/// Tglad fold; the radius of the folded point; scale; then plain seeding
/// outside radius 1, a W/N power -2 inside the inner radius, and a linear
/// blend between. The blend removes the branch cuts; what is left is
/// discontinuous on purpose. The file's description says `-2 x y` for the
/// second component; the code computes `-x y`.
fn bulbox(slot: &Slot, orbit: &mut Orbit, seed: Vec3) {
    let [scale, inner_r, inner_scale, fold] = slot.params;
    let start = orbit.z;
    let fold_slope = |a: f64| slope_sign(a + fold) - slope_sign(a - fold) - 1.0;
    let folds = Vec3::new(
        fold_slope(start.x),
        fold_slope(start.y),
        fold_slope(start.z),
    );
    let folded = Vec3::new(
        (start.x + fold).abs() - (start.x - fold).abs() - start.x,
        (start.y + fold).abs() - (start.y - fold).abs() - start.y,
        (start.z + fold).abs() - (start.z - fold).abs() - start.z,
    );
    let r2 = folded.dot(folded).max(MIN_RADIUS2);
    let radius = r2.sqrt();
    let scaled = folded * scale;
    let outer = scale.abs();
    // Columns of the scaled fold, before the branches.
    let mut scaled_jac = orbit.jac;
    map_columns(&mut scaled_jac, |c| times(c, folds) * scale);
    let (next, stretch) = if slot.mode == bulbox_mode::BOX && radius >= 1.0 {
        for ((c, d), s) in orbit.jac.iter_mut().zip(scaled_jac).zip(orbit.seed_jac) {
            *c = d + s;
        }
        (scaled + seed, outer)
    } else {
        let rxy = (scaled.x * scaled.x + scaled.y * scaled.y)
            .sqrt()
            .max(1e-12);
        let power_scale = inner_scale / (r2 * r2);
        let flatten = 1.0 - (scaled.z / rxy) * (scaled.z / rxy);
        let power = Vec3::new(
            (scaled.x * scaled.x - scaled.y * scaled.y) * flatten * power_scale,
            -(scaled.x * scaled.y) * flatten * power_scale,
            -2.0 * scaled.z * rxy * power_scale,
        );
        // The power -2 map stretches by 2 |inner scale| scale^2 / radius^3.
        let inner = 2.0 * inner_scale.abs() * scale * scale / (r2 * radius);
        let power_rows = bulbox_power_rows(scaled, rxy, flatten, power_scale);
        let blend = if radius <= inner_r {
            0.0
        } else {
            (radius - inner_r) / (1.0 - inner_r)
        };
        // `blend` grows with the radius, which is `|scaled| / scale`.
        let blend_slope = if radius <= inner_r {
            Vec3::ZERO
        } else {
            folded * (1.0 / (radius * scale * (1.0 - inner_r)))
        };
        for ((c, d), s) in orbit.jac.iter_mut().zip(scaled_jac).zip(orbit.seed_jac) {
            let dp = by_rows(&power_rows, d);
            *c = dp * (1.0 - blend) + d * blend + (scaled - power) * blend_slope.dot(d) + s;
        }
        if radius <= inner_r {
            (power + seed, inner)
        } else {
            (
                power * (1.0 - blend) + scaled * blend + seed,
                inner * (1.0 - blend).abs() + outer * blend.abs(),
            )
        }
    };
    map_columns(&mut orbit.jac, |c| slot.rotation.apply(c));
    orbit.z = slot.rotation.apply(next);
    orbit.dr = orbit.dr * stretch + orbit.seed_dr;
}

/// Rows of the Jacobian of Bulbox's power -2 map at the scaled point `v`,
/// whose terms are `flatten = 1 - v.z^2 / rxy^2` and `q = inner scale / r^4`.
fn bulbox_power_rows(v: Vec3, rxy: f64, flatten: f64, q: f64) -> Columns {
    let p = rxy * rxy;
    let vv = v.dot(v);
    let grad_flatten = Vec3::new(
        2.0 * v.x * v.z * v.z / (p * p),
        2.0 * v.y * v.z * v.z / (p * p),
        -2.0 * v.z / p,
    );
    // q falls as |v|^-4, since r is |v| over the scale.
    let grad_q = v * (-4.0 * q / vv);
    let grad_product = grad_flatten * q + grad_q * flatten;
    let xy = Vec3::new(v.x, v.y, 0.0) * (1.0 / rxy);
    [
        Vec3::new(2.0 * v.x, -2.0 * v.y, 0.0) * (flatten * q)
            + grad_product * (v.x * v.x - v.y * v.y),
        Vec3::new(-v.y, -v.x, 0.0) * (flatten * q) - grad_product * (v.x * v.y),
        (Vec3::new(0.0, 0.0, rxy * q) + xy * (v.z * q) + grad_q * (v.z * rxy)) * -2.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(formula: FormulaId, mode: i32, params: [f64; 4]) -> Slot {
        Slot {
            formula,
            mode,
            count: 1,
            params,
            rotation: Mat3::IDENTITY,
        }
    }

    fn run(slot: &Slot, p: Vec3, seed: Vec3) -> Orbit {
        let mut orbit = Orbit::start(p);
        let mut seed = seed;
        apply(slot, &mut orbit, &mut seed);
        orbit
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-12
    }

    #[test]
    fn ids_round_trip_and_unknown_is_empty() {
        for id in FormulaId::ALL {
            assert_eq!(FormulaId::from_index(id as i32), id);
        }
        assert_eq!(FormulaId::from_index(99), FormulaId::Empty);
    }

    #[test]
    fn amazing_box_folds_then_inverts() {
        let s = slot(FormulaId::Box, box_mode::AMAZING, [-1.5, 0.5, 1.0, 1.0]);
        // x = 1.5 folds to 0.5; |z|^2 = 0.25 + 0.04 is between min r^2 and 1.
        let orbit = run(&s, Vec3::new(1.5, 0.2, 0.0), Vec3::ZERO);
        let rr = 0.25 + 0.04;
        let m = -1.5 / rr;
        assert!(close(orbit.z, Vec3::new(0.5 * m, 0.2 * m, 0.0)));
        assert!((orbit.dr - (m.abs() + 1.0)).abs() < 1e-12);
    }

    #[test]
    fn box_inner_radius_clamps_the_inversion() {
        let s = slot(FormulaId::Box, box_mode::AMAZING, [2.0, 0.5, 1.0, 1.0]);
        let orbit = run(&s, Vec3::new(0.1, 0.0, 0.0), Vec3::new(1.0, 2.0, 3.0));
        // |z|^2 = 0.01 < 0.25, so m = 2 / 0.25 = 8.
        assert!(close(orbit.z, Vec3::new(1.8, 2.0, 3.0)));
    }

    #[test]
    fn surf_swaps_seed_components() {
        let s = slot(FormulaId::Box, box_mode::SURF, [1.0, 0.5, 10.0, 10.0]);
        let orbit = run(&s, Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0));
        assert!(close(orbit.z, Vec3::new(2.0, 1.0, 3.0)));
    }

    #[test]
    fn degenerate_radii_do_not_panic_or_divide_by_zero() {
        for (min_r, fixed_r) in [(0.0, 1.0), (2.0, 1.0), (0.0, 0.0)] {
            let s = slot(
                FormulaId::Box,
                box_mode::AMAZING,
                [2.0, min_r, 1.0, fixed_r],
            );
            let orbit = run(&s, Vec3::ZERO, Vec3::new(0.1, 0.0, 0.0));
            assert!(orbit.z.length().is_finite() && orbit.dr.is_finite());
        }
    }

    #[test]
    fn menger_sorts_and_scales() {
        let s = slot(FormulaId::Menger, 0, [3.0, 1.0, 0.0, 0.0]);
        let orbit = run(&s, Vec3::new(-0.1, 0.9, 0.5), Vec3::ZERO);
        // abs, sorted descending: (0.9, 0.5, 0.1). h = 1/3.
        let h = 1.0 / 3.0;
        let expect_z = (h - (0.1f64 - h).abs()) * 3.0;
        assert!(close(
            orbit.z,
            Vec3::new(0.9 * 3.0 - 2.0, 0.5 * 3.0 - 2.0, expect_z)
        ));
        assert!((orbit.dr - 3.0).abs() < 1e-12);
    }

    #[test]
    fn sierpinski_reflects_pairs_with_negative_sum() {
        let s = slot(FormulaId::Sierpinski, 0, [2.0, 1.0, 0.0, 0.0]);
        let orbit = run(&s, Vec3::new(-0.5, 0.2, 0.3), Vec3::ZERO);
        // (x, y) = (-0.5, 0.2) sums negative -> (-0.2, 0.5); then (x, z) = (-0.2, 0.3) does not.
        assert!(close(orbit.z, Vec3::new(-0.4 - 1.0, 1.0 - 1.0, 0.6 - 1.0)));
    }

    #[test]
    fn tetra_kifs_at_unit_intensity_matches_sierpinski() {
        let kifs = slot(FormulaId::Kifs, kifs_mode::TETRA, [2.0, 1.0, 0.0, 1.0]);
        let sierp = slot(FormulaId::Sierpinski, 0, [2.0, 1.0, 0.0, 0.0]);
        for p in [
            Vec3::new(-0.5, 0.2, 0.3),
            Vec3::new(0.1, -0.7, -0.2),
            Vec3::new(-0.3, -0.4, 0.9),
        ] {
            let a = run(&kifs, p, Vec3::ZERO);
            let b = run(&sierp, p, Vec3::ZERO);
            assert!(close(a.z, b.z), "{p:?}: {:?} vs {:?}", a.z, b.z);
        }
    }

    #[test]
    fn mandelbulb_power_two_on_the_equator_squares_the_plane() {
        // On z = 0 the MB3D convention is complex squaring in x, y.
        let s = slot(FormulaId::Mandelbulb, 0, [2.0, 1.0, 0.0, 0.0]);
        let orbit = run(&s, Vec3::new(0.3, 0.4, 0.0), Vec3::new(0.1, 0.0, 0.0));
        assert!(close(orbit.z, Vec3::new(0.09 - 0.16 + 0.1, 0.24, 0.0)));
    }

    #[test]
    fn helispiral_rotates_about_z_by_the_twist() {
        let s = slot(
            FormulaId::Helispiral,
            0,
            [0.0, 0.0, std::f64::consts::FRAC_PI_2, 0.0],
        );
        let orbit = run(&s, Vec3::new(1.0, 0.0, 0.5), Vec3::ZERO);
        assert!(close(orbit.z, Vec3::new(0.0, 1.0, 0.5)));
    }

    #[test]
    fn gnarl_with_zero_step_is_a_scale() {
        let s = slot(FormulaId::Gnarl, 0, [0.0, 1.0, 1.0, 2.0]);
        let orbit = run(&s, Vec3::new(0.3, -0.2, 0.1), Vec3::ZERO);
        assert!(close(orbit.z, Vec3::new(0.6, -0.4, 0.2)));
    }

    fn bulbox() -> Slot {
        slot(FormulaId::Bulbox, 0, [2.0, 0.6, -0.5, 1.0])
    }

    #[test]
    fn bulbox_outside_radius_one_scales_and_adds_the_seed() {
        // Inside the fold limit, r = 1.27 >= 1: plain scale plus seed.
        let orbit = run(
            &bulbox(),
            Vec3::new(0.9, 0.9, 0.0),
            Vec3::new(0.1, 0.2, 0.3),
        );
        assert!(close(orbit.z, Vec3::new(1.9, 2.0, 0.3)), "{:?}", orbit.z);
    }

    #[test]
    fn bulbox_inside_the_inner_radius_takes_the_negative_power() {
        // r = 0.5 <= 0.6. Scaled v = (0.6, 0.8, 0), rxy = 1, a = 1,
        // q = -0.5 / 0.5^4 = -8: T = (-0.28 * -8, -0.48 * -8, 0).
        let orbit = run(&bulbox(), Vec3::new(0.3, 0.4, 0.0), Vec3::ZERO);
        assert!(close(orbit.z, Vec3::new(2.24, 3.84, 0.0)), "{:?}", orbit.z);
    }

    #[test]
    fn bulbox_blends_between_the_radii() {
        // r = 0.8: k = 0.5 between T and the scaled vector (1.6, 0, 0).
        let orbit = run(&bulbox(), Vec3::new(0.8, 0.0, 0.0), Vec3::ZERO);
        let q = -0.5 / 0.8f64.powi(4);
        let tx = 1.6 * 1.6 * q;
        assert!(
            close(orbit.z, Vec3::new(0.5 * tx + 0.5 * 1.6, 0.0, 0.0)),
            "{:?}",
            orbit.z
        );
    }

    #[test]
    fn bulbox_without_the_box_keeps_blending_outside_radius_one() {
        let boxed = run(&bulbox(), Vec3::new(0.9, 0.9, 0.0), Vec3::ZERO);
        let mut open = bulbox();
        open.mode = 1;
        let unboxed = run(&open, Vec3::new(0.9, 0.9, 0.0), Vec3::ZERO);
        assert!(!close(boxed.z, unboxed.z));
        assert!(unboxed.z.length().is_finite());
    }

    #[test]
    fn inversion_maps_distance_r_to_radius_squared_over_r() {
        // Radius 1 about (1, 0, 0): a point 2 away lands 0.5 away, same side.
        let s = slot(FormulaId::Inversion, 0, [1.0, 1.0, 0.0, 0.0]);
        let orbit = run(&s, Vec3::new(3.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));
        assert!(close(orbit.z, Vec3::new(1.5, 0.0, 0.0)), "{:?}", orbit.z);
        assert!((orbit.dr - 0.25).abs() < 1e-12);
    }

    #[test]
    fn inversion_of_the_whole_space_inverts_the_seed_too() {
        let s = slot(FormulaId::Inversion, 1, [1.0, 0.0, 0.0, 0.0]);
        let mut orbit = Orbit::start(Vec3::new(2.0, 0.0, 0.0));
        let mut seed = Vec3::new(0.0, 4.0, 0.0);
        apply(&s, &mut orbit, &mut seed);
        assert!(close(orbit.z, Vec3::new(0.5, 0.0, 0.0)));
        assert!(close(seed, Vec3::new(0.0, 0.25, 0.0)));
    }

    #[test]
    fn polyfold_maps_each_sector_onto_the_first() {
        // Order 4: the point at 100 degrees lies in sector n = 1 (odd), so it
        // is rotated back by 90 degrees and mirrored in x.
        let s = slot(FormulaId::Polyfold, 0, [4.0, 0.0, 0.0, 0.0]);
        let angle = 100f64.to_radians();
        let orbit = run(&s, Vec3::new(angle.cos(), angle.sin(), 0.3), Vec3::ZERO);
        let back = 10f64.to_radians();
        assert!(
            close(orbit.z, Vec3::new(-back.cos(), back.sin(), 0.3)),
            "{:?}",
            orbit.z
        );
        assert!((orbit.dr - 1.0).abs() < 1e-12, "an isometry");
    }

    #[test]
    fn polyfold_rotates_even_sectors_by_a_half_turn() {
        // At 10 degrees, n = 0 (even): both components negate.
        let s = slot(FormulaId::Polyfold, 0, [4.0, 0.0, 0.0, 0.0]);
        let angle = 10f64.to_radians();
        let orbit = run(&s, Vec3::new(angle.cos(), angle.sin(), 0.0), Vec3::ZERO);
        assert!(
            close(orbit.z, Vec3::new(-angle.cos(), -angle.sin(), 0.0)),
            "{:?}",
            orbit.z
        );
    }

    #[test]
    fn sine_warps_one_component() {
        // Variant 0 is SinY: y = sin((y - 0.1) * 2) * 3 + 0.5.
        let s = slot(FormulaId::Sine, 0, [0.1, 2.0, 3.0, 0.5]);
        let orbit = run(&s, Vec3::new(0.4, 0.7, -0.2), Vec3::ZERO);
        let y = ((0.7f64 - 0.1) * 2.0).sin() * 3.0 + 0.5;
        assert!(close(orbit.z, Vec3::new(0.4, y, -0.2)), "{:?}", orbit.z);
        assert!(
            (orbit.dr - 6.0).abs() < 1e-12,
            "stretch bound |scale1 scale2|"
        );
    }

    #[test]
    fn reciprocal_is_continuous_through_zero() {
        // Variant 0 is x: x' = sign(x) (1/L - 1/(|x| + L)).
        let s = slot(FormulaId::Reciprocal, 0, [0.5, 0.0, 0.0, 0.0]);
        let at = |x: f64| run(&s, Vec3::new(x, 0.2, 0.3), Vec3::ZERO);
        let orbit = at(1.5);
        assert!(
            close(orbit.z, Vec3::new(2.0 - 0.5, 0.2, 0.3)),
            "{:?}",
            orbit.z
        );
        assert!((at(1e-9).z.x - at(-1e-9).z.x).abs() < 1e-6, "jump at zero");
    }

    fn repeated(mode: i32, params: [f64; 4], p: Vec3) -> (Vec3, Vec3, f64) {
        let s = slot(FormulaId::Repeat, mode, params);
        let mut orbit = Orbit::start(p);
        let mut seed = p;
        apply(&s, &mut orbit, &mut seed);
        (orbit.z, seed, orbit.dr)
    }

    #[test]
    fn repeat_leaves_the_center_cell_alone() {
        let p = Vec3::new(0.3, -0.4, 0.1);
        let (z, seed, dr) = repeated(0, [2.0, 0.0, 0.0, 0.0], p);
        assert!(close(z, p) && close(seed, p));
        assert!((dr - 1.0).abs() < 1e-12, "an isometry");
    }

    #[test]
    fn plain_repeat_copies_each_cell_and_mirror_repeat_flips_odd_ones() {
        let p = Vec3::new(0.3 + 2.0, -0.4, 0.1 - 4.0);
        let (plain, seed, _) = repeated(1, [2.0, 0.0, 0.0, 0.0], p);
        assert!(close(plain, Vec3::new(0.3, -0.4, 0.1)), "{plain:?}");
        assert!(close(seed, plain), "the seed moves with the point");
        // Mirror: one cell over in x flips x; two cells over in z keeps z.
        let (mirror, _, _) = repeated(0, [2.0, 0.0, 0.0, 0.0], p);
        assert!(close(mirror, Vec3::new(-0.3, -0.4, 0.1)), "{mirror:?}");
    }

    #[test]
    fn mirror_repeat_is_continuous_across_a_cell_border() {
        let at = |x: f64| {
            repeated(0, [2.0, 0.0, 0.0, 0.0], Vec3::new(x, 0.0, 0.0))
                .0
                .x
        };
        assert!((at(1.0 - 1e-9) - at(1.0 + 1e-9)).abs() < 1e-6);
    }

    #[test]
    fn warped_repeat_makes_every_cell_different() {
        let p = Vec3::new(0.3, -0.4, 0.1);
        let far = p + Vec3::new(8.0, 0.0, 0.0);
        let (plain_a, _, _) = repeated(1, [2.0, 0.0, 0.0, 0.0], p);
        let (plain_b, _, _) = repeated(1, [2.0, 0.0, 0.0, 0.0], far);
        assert!(close(plain_a, plain_b), "unwarped copies are identical");
        let (warped_a, _, _) = repeated(1, [2.0, 0.0, 0.3, 0.0], p);
        let (warped_b, _, _) = repeated(1, [2.0, 0.0, 0.3, 0.0], far);
        assert!(
            (warped_a - warped_b).length() > 1e-3,
            "warped copies differ"
        );
    }

    #[test]
    fn warped_repeat_stays_continuous_and_bounds_its_stretch() {
        let at = |x: f64| repeated(0, [2.0, 0.0, 0.3, 0.7], Vec3::new(x, 0.2, -0.1));
        let (a, _, dr) = at(1.0 - 1e-9);
        let (b, _, _) = at(1.0 + 1e-9);
        assert!((a - b).length() < 1e-6, "{a:?} vs {b:?}");
        assert!(dr > 1.0, "the warp stretches space, so dr grows");
    }

    #[test]
    fn repeat_count_limits_the_copies() {
        // One copy each way: the cell three over is the last copy moved out.
        let (z, _, _) = repeated(1, [2.0, 1.0, 0.0, 0.0], Vec3::new(6.3, 0.0, 0.0));
        assert!(close(z, Vec3::new(4.3, 0.0, 0.0)), "{z:?}");
    }

    #[test]
    fn only_box_kaliset_bulb_and_bulbox_add_the_seed() {
        let seeded: Vec<FormulaId> = FormulaId::ALL
            .into_iter()
            .filter(|id| id.adds_seed())
            .collect();
        assert_eq!(
            seeded,
            [
                FormulaId::Box,
                FormulaId::Kaliset,
                FormulaId::Mandelbulb,
                FormulaId::Bulbox
            ]
        );
    }

    /// Formulas and parameters for the Jacobian checks. Mandelbulb is left
    /// out: chains with it use the power DE, which does not read `J`.
    fn jacobian_cases() -> Vec<(FormulaId, [f64; 4])> {
        vec![
            (FormulaId::Box, [-1.8, 0.5, 1.0, 1.0]),
            (FormulaId::Menger, [3.0, 1.0, 0.0, 0.0]),
            (FormulaId::Sierpinski, [2.0, 1.0, 0.0, 0.0]),
            (FormulaId::Kifs, [2.0, 1.0, 1.0, 0.7]),
            (FormulaId::PseudoKleinian, [0.9, 1.2, 0.0, 0.0]),
            (FormulaId::Kaliset, [1.3, 0.2, 0.0, 0.0]),
            (FormulaId::Transform, [1.5, 0.1, -0.2, 0.3]),
            (FormulaId::Helispiral, [0.25, 0.4, 0.3, 0.0]),
            (FormulaId::Gnarl, [0.15, 1.0, 1.0, 1.0]),
            (FormulaId::Bulbox, [1.7, 0.4, 1.2, 1.0]),
            (FormulaId::Inversion, [1.1, 0.3, -0.2, 0.4]),
            (FormulaId::Polyfold, [5.0, 12.0, 0.2, -0.1]),
            (FormulaId::Sine, [0.2, 1.3, 0.8, 0.1]),
            (FormulaId::Reciprocal, [0.5, 0.0, 0.0, 0.0]),
            (FormulaId::Repeat, [1.5, 0.0, 0.3, 0.5]),
            (FormulaId::Repeat, [1.5, 2.0, 0.0, 0.0]),
            (FormulaId::KochCube, [1.0, 1.2, 1.0, 0.1]),
            (FormulaId::JCube, [0.414_213_56, 3.0, 1.0, 1.0]),
            (FormulaId::LinCombine, [1.5, 0.7, 1.2, 0.0]),
            (FormulaId::Rotate4D, [0.17, 0.28, 0.39, 0.0]),
            (FormulaId::ABoxMod2, [2.0, 0.5, 1.0, 1.5]),
        ]
    }

    fn lcg(state: &mut u64) -> f64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }

    /// `z` after one step from `p`, with the seed at `p`.
    fn step_from(slot: &Slot, p: Vec3) -> Orbit {
        run(slot, p, p)
    }

    #[test]
    fn every_step_jacobian_matches_finite_differences() {
        let h = 1e-6;
        let axes = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let mut state = 7u64;
        for (formula, params) in jacobian_cases() {
            for mode in 0..5 {
                for rotated in [false, true] {
                    let mut s = slot(formula, mode, params);
                    if rotated {
                        s.rotation = Mat3::from_euler(0.3, -0.5, 0.9);
                    }
                    let mut checked = 0;
                    for _ in 0..200 {
                        let p = Vec3::new(lcg(&mut state), lcg(&mut state), lcg(&mut state)) * 1.6;
                        let orbit = step_from(&s, p);
                        for (j, axis) in axes.iter().enumerate() {
                            let z0 = orbit.z;
                            let ahead = step_from(&s, p + *axis * h).z;
                            let behind = step_from(&s, p - *axis * h).z;
                            let forward = (ahead - z0) * (1.0 / h);
                            let backward = (z0 - behind) * (1.0 / h);
                            // A branch edge within h: no derivative to compare.
                            if (forward - backward).length() > 1e-3 * (1.0 + forward.length()) {
                                continue;
                            }
                            let numeric = (ahead - behind) * (0.5 / h);
                            let analytic = orbit.jac[j];
                            assert!(
                                (numeric - analytic).length() < 1e-5 * (1.0 + numeric.length()),
                                "{formula:?} mode {mode} rotated {rotated} at {p:?} column {j}: \
                                 analytic {analytic:?} numeric {numeric:?}"
                            );
                            checked += 1;
                        }
                    }
                    assert!(
                        checked > 100,
                        "{formula:?} mode {mode}: only {checked} checks"
                    );
                }
            }
        }
    }

    #[test]
    fn a_julia_seed_adds_nothing_to_the_jacobian() {
        let s = slot(FormulaId::Box, box_mode::AMAZING, [2.0, 0.5, 1.0, 1.0]);
        let mut orbit = Orbit::start(Vec3::new(3.0, 0.0, 0.0));
        orbit.seed_jac = [Vec3::ZERO; 3];
        let mut seed = Vec3::new(1.0, 2.0, 3.0);
        apply(&s, &mut orbit, &mut seed);
        // x folds (-1), |z| = 1 leaves m = 2: only the fold and scale remain.
        assert!(close(orbit.jac[0], Vec3::new(-2.0, 0.0, 0.0)));
        assert!(close(orbit.jac[1], Vec3::new(0.0, 2.0, 0.0)));
    }

    #[test]
    fn gnarl_scale_zero_counts_as_one() {
        let zero = run(
            &slot(FormulaId::Gnarl, 0, [0.1, 1.0, 1.0, 0.0]),
            Vec3::new(0.3, -0.2, 0.5),
            Vec3::ZERO,
        );
        let one = run(
            &slot(FormulaId::Gnarl, 0, [0.1, 1.0, 1.0, 1.0]),
            Vec3::new(0.3, -0.2, 0.5),
            Vec3::ZERO,
        );
        assert!(close(zero.z, one.z));
    }

    #[test]
    fn non_conformal_steps_are_named() {
        let conformal = |f, p| slot(f, 0, p).is_conformal();
        assert!(conformal(FormulaId::Box, [2.0, 0.5, 1.0, 1.0]));
        assert!(conformal(FormulaId::Kifs, [2.0, 1.0, 1.0, 1.0]));
        assert!(!conformal(FormulaId::Kifs, [2.0, 1.0, 1.0, 0.7]));
        assert!(conformal(FormulaId::Repeat, [6.0, 0.0, 0.0, 0.0]));
        assert!(!conformal(FormulaId::Repeat, [6.0, 0.0, 0.2, 0.0]));
        assert!(!conformal(FormulaId::Helispiral, [0.1, 0.2, 0.0, 0.0]));
        assert!(!conformal(FormulaId::Gnarl, [0.1, 1.0, 1.0, 1.0]));
        assert!(!conformal(FormulaId::Bulbox, [1.7, 0.4, 1.2, 1.0]));
        assert!(conformal(FormulaId::Inversion, [1.0, 0.0, 0.0, 0.0]));
        assert!(
            !slot(
                FormulaId::Inversion,
                inversion_mode::SPACE,
                [1.0, 0.0, 0.0, 0.0]
            )
            .is_conformal()
        );
    }

    #[test]
    fn seeded_steps_add_the_seeds_stretch() {
        let warp = slot(FormulaId::Repeat, 0, [6.0, 0.0, 0.3, 0.5]);
        let boxed = slot(FormulaId::Box, box_mode::AMAZING, [2.0, 0.5, 1.0, 1.0]);
        let p = Vec3::new(3.0, 0.0, 0.0);
        let mut orbit = Orbit::start(p);
        let mut seed = p;
        apply(&warp, &mut orbit, &mut seed);
        let (_, stretch) = quasi_warp(p, 6.0, 0.3, 0.5);
        assert!((orbit.seed_dr - stretch).abs() < 1e-12);
        let before = orbit.dr;
        let z = orbit.z;
        apply(&boxed, &mut orbit, &mut seed);
        let rr = z.clamp_sym(1.0) * 2.0 - z;
        let m = 2.0 / rr.dot(rr).clamp(0.25, 1.0);
        assert!((orbit.dr - (before * m + stretch)).abs() < 1e-9);
    }

    #[test]
    fn a_rotated_space_step_turns_the_seed_too() {
        let mut repeat = slot(FormulaId::Repeat, 0, [6.0, 0.0, 0.2, 0.5]);
        repeat.rotation = Mat3::from_euler(0.3, -0.5, 0.9);
        let p = Vec3::new(4.0, -1.0, 2.5);
        let orbit = run(&repeat, p, p);
        let mut seed = p;
        apply(&repeat, &mut Orbit::start(p), &mut seed);
        assert!(close(orbit.z, seed), "the lattice turns as a whole");
        let mut inversion = slot(
            FormulaId::Inversion,
            inversion_mode::SPACE,
            [1.1, 0.3, -0.2, 0.4],
        );
        inversion.rotation = repeat.rotation;
        let orbit = run(&inversion, p, p);
        let mut seed = p;
        apply(&inversion, &mut Orbit::start(p), &mut seed);
        assert!(close(orbit.z, seed));
    }

    #[test]
    fn space_steps_are_the_ones_that_move_the_seed() {
        assert!(slot(FormulaId::Repeat, 0, [6.0, 0.0, 0.0, 0.0]).moves_seed());
        assert!(
            slot(
                FormulaId::Inversion,
                inversion_mode::SPACE,
                [1.0, 0.0, 0.0, 0.0]
            )
            .moves_seed()
        );
        assert!(
            !slot(
                FormulaId::Inversion,
                inversion_mode::POINT,
                [1.0, 0.0, 0.0, 0.0]
            )
            .moves_seed()
        );
        assert!(!slot(FormulaId::Box, 0, [2.0, 0.5, 1.0, 1.0]).moves_seed());
    }

    fn close_to(a: Vec3, b: Vec3, tolerance: f64) -> bool {
        (a - b).length() < tolerance
    }

    #[test]
    fn koch_cube_takes_each_branch_as_described() {
        let koch = slot(FormulaId::KochCube, 0, [1.0, 1.0, 1.0, 0.0]);
        // 3|z| sorted (0.9, 0.6, 0.3); x - 2 < y: x -= 2, y -= 2.
        let a = run(&koch, Vec3::new(0.3, -0.2, 0.1), Vec3::ZERO);
        assert!(close(a.z, Vec3::new(-1.1, -1.4, 0.3)), "{:?}", a.z);
        // (6, 0.3, 0.15): x - 4 > y: x -= 4.
        let b = run(&koch, Vec3::new(2.0, 0.1, 0.05), Vec3::ZERO);
        assert!(close(b.z, Vec3::new(2.0, 0.3, 0.15)), "{:?}", b.z);
        // (3.3, 0.6, 0.3): neither: (x, y) = (y, x - 4).
        let c = run(&koch, Vec3::new(1.1, 0.2, 0.1), Vec3::ZERO);
        assert!(close(c.z, Vec3::new(0.6, -0.7, 0.3)), "{:?}", c.z);
    }

    #[test]
    fn jcube_scales_edge_and_corner_cubes() {
        let jcube = slot(
            FormulaId::JCube,
            0,
            [std::f64::consts::SQRT_2 - 1.0, 3.0, 1.0, 1.0],
        );
        let s0 = std::f64::consts::SQRT_2 + 1.0;
        let s8 = s0 * s0;
        // Edge: x' = 0.1 <= 1/s8 and y' = 0.8 >= 0.1 + 1 - 3/s8.
        let edge = run(&jcube, Vec3::new(0.1, 0.9, 0.8), Vec3::ZERO);
        let want = Vec3::new(0.1 * s8, (0.8 - 1.0) * s8 + 1.0, (0.9 - 1.0) * s8 + 1.0);
        assert!(close_to(edge.z, want, 1e-9), "{:?} vs {want:?}", edge.z);
        assert!((edge.dr - s8).abs() < 1e-9);
        // Corner: x' = 0.5 > 1/s8; sorted (0.5, 0.7, 0.6).
        let corner = run(&jcube, Vec3::new(0.5, -0.6, 0.7), Vec3::ZERO);
        let want = Vec3::new(-0.5 * s0 + 1.0, -0.3 * s0 + 1.0, -0.4 * s0 + 1.0);
        assert!(close_to(corner.z, want, 1e-9), "{:?} vs {want:?}", corner.z);
    }

    #[test]
    fn lin_combine_scales_each_axis() {
        let lin = slot(FormulaId::LinCombine, 0, [2.0, 0.5, 1.0, 0.0]);
        let o = run(&lin, Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO);
        assert!(close(o.z, Vec3::new(2.0, 1.0, 3.0)));
    }

    #[test]
    fn rotate_4d_turns_through_w_and_back() {
        // XW and YW at a half turn: x and y negate, w stays 0.
        let both = slot(FormulaId::Rotate4D, 0, [1.0, 1.0, 0.0, 0.0]);
        let o = run(&both, Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO);
        assert!(close_to(o.z, Vec3::new(-1.0, -2.0, 3.0), 1e-12) && o.w.abs() < 1e-12);
        // XW at a quarter turn moves x into w; a second brings it back negated.
        let quarter = slot(FormulaId::Rotate4D, 0, [0.5, 0.0, 0.0, 0.0]);
        let mut orbit = Orbit::start(Vec3::new(1.0, 2.0, 3.0));
        let mut seed = Vec3::ZERO;
        apply(&quarter, &mut orbit, &mut seed);
        assert!(
            close_to(orbit.z, Vec3::new(0.0, 2.0, 3.0), 1e-12) && (orbit.w - 1.0).abs() < 1e-12
        );
        apply(&quarter, &mut orbit, &mut seed);
        assert!(close_to(orbit.z, Vec3::new(-1.0, 2.0, 3.0), 1e-12) && orbit.w.abs() < 1e-12);
    }

    #[test]
    fn abox_mod2_inverts_in_a_capped_cylinder() {
        let abox = slot(FormulaId::ABoxMod2, 0, [2.0, 0.5, 1.0, 1.5]);
        // Inside the cylinder body below min R: m = 2 / 0.25.
        let a = run(&abox, Vec3::new(0.3, 0.2, 0.4), Vec3::ZERO);
        assert!(close(a.z, Vec3::new(2.4, 1.6, 3.2)), "{:?}", a.z);
        // Past the half size the cap counts: rr = 0.36 + 0.25 + 0.16.
        let b = run(&abox, Vec3::new(0.6, 0.5, 0.9), Vec3::ZERO);
        let m = 2.0 / 0.77;
        assert!(
            close_to(b.z, Vec3::new(0.6, 0.5, 0.9) * m, 1e-12),
            "{:?}",
            b.z
        );
        // The XY fold: 1.5 folds to 0.5.
        let c = run(&abox, Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO);
        assert!(
            close_to(c.z, Vec3::new(0.5 * 8.0, 0.0, 0.0), 1e-12),
            "{:?}",
            c.z
        );
    }

    #[test]
    fn msltoe_sym4_swaps_then_squares() {
        let sym4 = slot(FormulaId::MsltoeSym4, 0, [1.0, 1.0, 1.0, 0.0]);
        // Swaps to (0.5, 0.3, 0.2), then (x^2 - y^2 - z^2, 2xy, 2xz).
        let o = run(&sym4, Vec3::new(0.2, 0.5, 0.3), Vec3::ZERO);
        assert!(close(o.z, Vec3::new(0.12, 0.3, 0.2)), "{:?}", o.z);
        let r = (0.04_f64 + 0.25 + 0.09).sqrt();
        assert!((o.dr - (2.0 * r + 1.0)).abs() < 1e-12);
    }

    #[test]
    fn the_new_formulas_name_their_conformality() {
        let conformal = |f, p| slot(f, 0, p).is_conformal();
        assert!(conformal(FormulaId::JCube, [0.41, 3.0, 1.0, 1.0]));
        assert!(conformal(FormulaId::KochCube, [1.0, 1.0, 1.0, 0.0]));
        assert!(!conformal(FormulaId::KochCube, [1.0, 1.3, 1.0, 0.0]));
        assert!(conformal(FormulaId::LinCombine, [2.0, -2.0, 2.0, 0.0]));
        assert!(!conformal(FormulaId::LinCombine, [2.0, 1.0, 2.0, 0.0]));
        assert!(conformal(FormulaId::Rotate4D, [1.0, 0.0, 2.0, 0.0]));
        assert!(!conformal(FormulaId::Rotate4D, [0.3, 0.0, 0.0, 0.0]));
        assert!(!conformal(FormulaId::ABoxMod2, [2.0, 0.5, 1.0, 1.5]));
    }
}
