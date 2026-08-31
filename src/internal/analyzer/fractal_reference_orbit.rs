//! Reference orbit for perturbed fractal distance estimation.
//!
//! See `/spec/fractal-reference-orbit-preprocessor.md`.
//!
//! `shaders/fractal_explorer.fs` can evaluate its distance estimator by
//! perturbation: carry a reference point in high precision and the sample's
//! exact offset from it in low precision, never forming a difference of two
//! order-one quantities. That is verified on the GPU. What the shader cannot
//! produce for itself is the reference, for two reasons, and a false-colour
//! diagnostic says which of them actually bites.
//!
//! The one that bites is **orbit lifetime**. Instrumenting the shader shows the
//! fold count frozen at 38 and `log10(dr)` frozen at 17.2 from a zoom of `1e-6`
//! all the way to `1e-16`, with the resolution ratio saturating at about `5e5`:
//! the finest feature the estimate can distinguish stays half a million times
//! larger than the window. Thirty-eight is not the fold ceiling and not the
//! level-of-detail cutoff. It is the bailout. The shader's reference is its dive
//! target, found by a probe that stops within `8.4e-3` of the surface, so the
//! reference is nowhere near the set and its orbit escapes after 38 iterations.
//! Every fold after that is unavailable at any zoom.
//!
//! So a merely *precise* anchor buys nothing. The anchor has to be one whose
//! orbit survives substantially longer than the records the renderer consumes.
//! For fold stacks, Newton refinement can solve for a pre-periodic
//! (Misiurewicz) point satisfying
//!
//! ```text
//!     F^(n+p)(c) = F^n(c)
//! ```
//!
//! so its orbit is bounded for every iteration count by construction. The
//! continuous-power Mandelbulb instead uses a camera-ray search in arbitrary
//! precision. It targets survival several times beyond the transported fold
//! count, avoiding the smooth finite-level set that previously became the
//! anchor while keeping the view tied to the parked camera.
//!
//! The second reason, precision, still matters and is handled by computing the
//! orbit in double-double arithmetic and only then reducing to `f32` for
//! transport. That asymmetry is the load-bearing one: rounding *the orbit* to
//! `f32` at every step destroys the anchor's accuracy immediately, while
//! rounding *the transported values* is harmless, because each is order one and
//! an error of eps there perturbs the computed increment by order eps relative,
//! which is already `f32` noise.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::str::FromStr;

use dashu_float::ops::Abs;
use dashu_float::round::mode::HalfEven;
use dashu_float::{Context, FBig};
use serde::Deserialize;

use super::fractal_certification::boundary::{parked_ray_geometry, BoundaryReason, ParkedCamera};
use super::fractal_certification::segment::{
    certify_primary_ray_segments, PrimaryCamera, SegmentAtlasResult, SegmentBudgets, ATLAS_COLUMNS,
    ATLAS_ROWS,
};
use super::traits::{
    next_texture_generation, Analyzer, AnalyzerInput, AnalyzerSchema, AnalyzerSnapshot,
    AnalyzerStateSnapshot, ScalarOutputDef, TextureData, TextureOutputDef,
};

/// Iterations the orbit texture can hold.
const MAX_ITERS_LIMIT: usize = 4096;

/// Version of the host-to-shader record contract.
const PAYLOAD_VERSION: u32 = 7;

/// Records of header in front of the orbit.
///
/// Two since payload v7: the first carries version, anchor, signature and the
/// certificate atlas, and the second carries the measured shell-fold table.
const HEADER_RECORDS: usize = 2;

/// Floats stored per iteration.
///
/// Three groups of four. The shader must recompute *nothing*, which is the whole
/// point of the layout: an earlier version transported only the end-of-iteration
/// point and let the shader re-derive each fold's intermediates in `f32`, so the
/// offset increment was computed against `f32`-derived values while the reference
/// came from double-double, and the pair stopped describing the same sample by
/// about `1e-7` per iteration. Amplified by the derivative that dominates within
/// a dozen folds.
///
/// Group 0: the point entering the iteration, and its squared radius.
/// Group 1: the point after the fold or sort stage, and its squared radius. This
///          is what the radial identity reads, and it is the value that was
///          previously recomputed.
/// Group 2: the point after any scale applied to the branch result, and the
///          radial multiplier. Menger branches again *after* its scale, on the
///          trailing conditional shift, and this is the reference that branch
///          reads. Slots with no post-scale branch repeat the leaving point.
/// Group 3: the point leaving the iteration and the authoritative branch code.
/// Group 4: Mandelbulb radius, theta, phi, and radius-to-power (zero otherwise).
/// Groups 5-7: twelve native-precision decision margins. Margin 11 is always
///             `bailout^2 - |post|^2`. Slot-specific indices are documented by
///             `transport_margins_dd` and `ApResolved::transport_margins`.
/// Group 8: authoritative branch code, Mandelbulb principal-seam side,
///          principal winding (zero for `atan2`), and a Mandelbulb-state flag.
/// Groups 9-11: reserved and zero.
///
/// Slots with no branch stage (the rotations, the linear recombination) repeat
/// the entering point in group 1, so the layout is uniform and the shader needs
/// no per-slot indexing.
const FLOATS_PER_ITER: usize = 48;

/// Groups of four floats per iteration, i.e. `FLOATS_PER_ITER / 4`.
const GROUPS_PER_ITER: usize = FLOATS_PER_ITER / 4;

const MARGIN_COUNT: usize = 12;
const BAILOUT_MARGIN_INDEX: usize = 11;

/// Portable bounded row width for the byte-packed two-dimensional payload.
const PAYLOAD_WIDTH: usize = 1024;

// ── Double-double arithmetic ────────────────────────────────────────────────

/// A number held as an unevaluated sum of two `f64`s, giving about 32 decimal
/// digits.
///
/// Enough for zoom depths to roughly `1e-30`, which is far past anything the
/// shader can currently use. If more is ever needed this type is the only thing
/// that changes; nothing downstream sees it, because the orbit is reduced to
/// `f32` before it leaves.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Dd {
    hi: f64,
    lo: f64,
}

impl Dd {
    const fn new(hi: f64) -> Self {
        Self { hi, lo: 0.0 }
    }

    /// Knuth's two-sum: exact, and the reason this type works at all.
    fn two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        let bb = s - a;
        (s, (a - (s - bb)) + (b - bb))
    }

    fn quick_two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        (s, b - (s - a))
    }

    fn renorm(hi: f64, lo: f64) -> Self {
        let (h, l) = Self::quick_two_sum(hi, lo);
        Self { hi: h, lo: l }
    }

    fn add(self, o: Self) -> Self {
        let (s1, s2) = Self::two_sum(self.hi, o.hi);
        let (t1, t2) = Self::two_sum(self.lo, o.lo);
        let (s1b, s2b) = Self::quick_two_sum(s1, s2 + t1);
        Self::renorm(s1b, s2b + t2)
    }

    fn sub(self, o: Self) -> Self {
        self.add(o.neg())
    }

    const fn neg(self) -> Self {
        Self {
            hi: -self.hi,
            lo: -self.lo,
        }
    }

    /// Two-product by Dekker splitting. `f64::mul_add` would be shorter but is
    /// not guaranteed to be a single fused instruction everywhere.
    fn two_prod(a: f64, b: f64) -> (f64, f64) {
        let p = a * b;
        let e = a.mul_add(b, -p);
        (p, e)
    }

    fn mul(self, o: Self) -> Self {
        let (p1, p2) = Self::two_prod(self.hi, o.hi);
        Self::renorm(p1, p2 + (self.hi * o.lo + self.lo * o.hi))
    }

    fn mul_f(self, k: f64) -> Self {
        let (p1, p2) = Self::two_prod(self.hi, k);
        Self::renorm(p1, p2 + self.lo * k)
    }

    fn div(self, o: Self) -> Self {
        let q1 = self.hi / o.hi;
        let r = self.sub(o.mul_f(q1));
        let q2 = r.hi / o.hi;
        let r2 = r.sub(o.mul_f(q2));
        let q3 = r2.hi / o.hi;
        let (a, b) = Self::quick_two_sum(q1, q2);
        Self::renorm(a, b + q3)
    }

    fn abs(self) -> Self {
        if self.hi < 0.0 {
            self.neg()
        } else {
            self
        }
    }

    fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    #[allow(clippy::cast_possible_truncation)]
    fn to_f32(self) -> f32 {
        self.to_f64() as f32
    }

    fn cmp(self, other: Self) -> Ordering {
        match self.hi.partial_cmp(&other.hi).unwrap_or(Ordering::Equal) {
            Ordering::Equal => self.lo.partial_cmp(&other.lo).unwrap_or(Ordering::Equal),
            ordering => ordering,
        }
    }
}

type V3 = [Dd; 3];

/// The reference values one iteration produces, all of which some identity reads.
#[derive(Clone, Copy)]
struct StepRecord {
    /// After the fold or sort stage, before any radial multiplier.
    mid: V3,
    /// After the scale, before any branch that follows it. Equal to `post`
    /// except where a slot branches after scaling, which is Menger alone.
    mid2: V3,
    /// Leaving the iteration.
    post: V3,
    /// The radial multiplier applied to the reference.
    m_ref: Dd,
    /// Decisions made from the native double-double values.
    branches: u32,
    /// Signed distances to every branch surface, reduced only during packing.
    margins: [Dd; MARGIN_COUNT],
}

fn pack_branches(fold: [u32; 3], radial: u32) -> u32 {
    fold[0] + 3 * fold[1] + 9 * fold[2] + 27 * radial
}

fn v_add(a: V3, b: V3) -> V3 {
    [a[0].add(b[0]), a[1].add(b[1]), a[2].add(b[2])]
}

fn v_sub(a: V3, b: V3) -> V3 {
    [a[0].sub(b[0]), a[1].sub(b[1]), a[2].sub(b[2])]
}

fn v_scale(a: V3, k: Dd) -> V3 {
    [a[0].mul(k), a[1].mul(k), a[2].mul(k)]
}

fn v_dot(a: V3, b: V3) -> Dd {
    a[0].mul(b[0]).add(a[1].mul(b[1])).add(a[2].mul(b[2]))
}

// ── Stack description ───────────────────────────────────────────────────────

/// Which formula a slot runs. Mirrors the shader's numbering, and covers only
/// the conformal slots: the Mandelbulb's estimator is separately known not to be
/// a distance bound, so the shader routes stacks containing it to the direct
/// path and no reference orbit is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Slot {
    Off,
    Mandelbox,
    AmazingBox,
    Menger,
    Sierpinski,
    Mandelbulb,
    PseudoKleinian,
    LinCombine,
    Rotate,
    CoCube,
    Rotate4d,
}

impl Slot {
    const fn from_shader_index(i: i64) -> Option<Self> {
        Some(match i {
            0 => Self::Off,
            1 => Self::Mandelbox,
            2 => Self::AmazingBox,
            3 => Self::Menger,
            4 => Self::Sierpinski,
            5 => Self::Mandelbulb,
            6 => Self::PseudoKleinian,
            7 => Self::LinCombine,
            8 => Self::Rotate,
            9 => Self::CoCube,
            10 => Self::Rotate4d,
            _ => return None,
        })
    }
}

/// Everything the orbit depends on, as the shader sees it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct StackParams {
    #[serde(default = "default_formulas")]
    pub formulas: [i64; 4],
    #[serde(default = "default_rates")]
    pub rates: [i64; 4],
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default = "default_one")]
    pub fold_limit: f64,
    #[serde(default = "default_min_radius")]
    pub min_radius: f64,
    #[serde(default = "default_one")]
    pub fixed_radius: f64,
    #[serde(default = "default_offset")]
    pub offset: [f64; 3],
    #[serde(default = "default_bailout")]
    pub bailout: f64,
    #[serde(default = "default_power")]
    pub power: f64,
    /// Decimal zoom depth requested by the shader. The arbitrary-precision
    /// backend derives its mantissa budget from this value.
    #[serde(default)]
    pub zoom_exp: f64,
    /// Fixed parked-camera distance shared with directional certification.
    ///
    /// This is not deserialized: the host reference payload deliberately uses
    /// the shipped camera rather than authored or live camera controls.
    #[serde(skip, default = "default_cam_dist")]
    pub cam_dist: f64,
    #[serde(skip)]
    pub cam_azim: f64,
    #[serde(skip, default = "default_cam_elev")]
    pub cam_elev: f64,
    #[serde(skip)]
    pub look: [f64; 3],
    #[serde(default = "default_render_aspect")]
    pub render_aspect: f64,
    /// Directional certificate work is enabled only for an explicit static zoom.
    #[serde(default)]
    pub certificate_enabled: bool,
    /// Authored opt-out for certificate work (`"certificates": false` in the
    /// shader's OPTIONS). Atlas certification costs minutes of worker time at
    /// production budgets, so a shader (or a debugging session) that does not
    /// consume certified prefixes can decline the work entirely.
    #[serde(default = "default_true", rename = "certificates")]
    pub certificates_allowed: bool,
    #[serde(default)]
    pub julia_amount: f64,
    #[serde(default = "default_julia")]
    pub julia: [f64; 3],
    /// Co-cube corner. Distinct from the offset, which an earlier version
    /// wrongly reused.
    #[serde(default = "default_cocube")]
    pub cocube: f64,
    /// Lin-combine diagonal and cross-axis bleed.
    #[serde(default = "default_lin")]
    pub lin: [f64; 3],
    #[serde(default = "default_lin_mix")]
    pub lin_mix: f64,
    /// Plane rotation angles: xy, yz, xz for the 3D slot.
    #[serde(default)]
    pub rot: [f64; 3],
    /// Plane rotation angles through the hidden fourth axis: xw, yw, zw.
    #[serde(default)]
    pub rot_w: [f64; 3],
    /// The zoom target, as a decimal string per axis so it can carry more digits
    /// than an `f64` literal in JSON would.
    #[serde(default)]
    pub anchor: Option<[String; 3]>,
    #[serde(default = "default_max_iters")]
    pub max_iters: usize,
    /// The renderer's authored fold cutoff, before it adds the depth-driven
    /// boost. Transporting fewer records than the shader will march silently
    /// truncates the march, so this has to be known here rather than guessed.
    #[serde(default = "default_stack_cap")]
    pub stack_cap: f64,
    /// Pre-period and period for the Newton condition. Small values are the
    /// shallowest pre-periodic points and the easiest to reach from a cold start.
    #[serde(default = "default_pre_period")]
    pub newton_pre_period: usize,
    #[serde(default = "default_period")]
    pub newton_period: usize,
    /// Set false to use the supplied anchor verbatim, which is what the
    /// diagnostic mode wants.
    #[serde(default = "default_true")]
    pub refine: bool,
}

const fn default_formulas() -> [i64; 4] {
    [5, 0, 0, 0]
}
const fn default_rates() -> [i64; 4] {
    [1, 0, 0, 0]
}
const fn default_scale() -> f64 {
    2.0
}
const fn default_one() -> f64 {
    1.0
}
const fn default_min_radius() -> f64 {
    0.5
}
const fn default_offset() -> [f64; 3] {
    [1.0, 1.0, 1.0]
}
const fn default_bailout() -> f64 {
    32.0
}
const fn default_power() -> f64 {
    8.0
}
const fn default_cam_dist() -> f64 {
    4.2
}
const fn default_cam_elev() -> f64 {
    PARKED_ELEVATION
}
const PARKED_AZIMUTH: f64 = 0.0;
const PARKED_ELEVATION: f64 = 0.2;
const PARKED_LOOK: [f64; 3] = [0.05, -0.02, 0.03];
const PARKED_FOV: f64 = 0.85;
const fn default_render_aspect() -> f64 {
    16.0 / 9.0
}
const fn default_julia() -> [f64; 3] {
    [0.35, -0.15, 0.2]
}
const fn default_cocube() -> f64 {
    0.6
}
const fn default_lin() -> [f64; 3] {
    [1.0, 1.0, 1.0]
}
const fn default_lin_mix() -> f64 {
    0.2
}
const fn default_max_iters() -> usize {
    512
}
/// Mirrors the shader's `stack_cap` default.
const fn default_stack_cap() -> f64 {
    14.0
}
const fn default_pre_period() -> usize {
    2
}
const fn default_period() -> usize {
    1
}
const fn default_true() -> bool {
    true
}

impl Default for StackParams {
    fn default() -> Self {
        Self {
            formulas: default_formulas(),
            rates: default_rates(),
            scale: default_scale(),
            fold_limit: default_one(),
            min_radius: default_min_radius(),
            fixed_radius: default_one(),
            offset: default_offset(),
            bailout: default_bailout(),
            power: default_power(),
            zoom_exp: 0.0,
            cam_dist: default_cam_dist(),
            cam_azim: PARKED_AZIMUTH,
            cam_elev: default_cam_elev(),
            look: [0.0; 3],
            render_aspect: default_render_aspect(),
            certificate_enabled: false,
            certificates_allowed: true,
            julia_amount: 0.0,
            julia: default_julia(),
            cocube: default_cocube(),
            lin: default_lin(),
            lin_mix: default_lin_mix(),
            rot: [0.0; 3],
            rot_w: [0.0; 3],
            anchor: None,
            max_iters: default_max_iters(),
            stack_cap: default_stack_cap(),
            newton_pre_period: default_pre_period(),
            newton_period: default_period(),
            refine: default_true(),
        }
    }
}

#[allow(dead_code)] // Called by the standalone Criterion bench, not the library target.
pub(crate) fn benchmark_reference_orbit(iterations: usize) -> usize {
    let params = StackParams {
        max_iters: iterations.clamp(1, MAX_ITERS_LIMIT),
        // A fold stack, not the default. This measures the double-double
        // kernel, and that backend does not implement the Mandelbulb: the
        // continuous-power orbit is arbitrary-precision only, so leaving the
        // default here drove `step_detailed` into its `unreachable!` and the
        // whole benchmark binary panicked before taking a sample. The stack
        // is the subject under test rather than the shipped configuration.
        formulas: [1, 0, 0, 0],
        ..StackParams::default()
    };
    let resolved = Resolved::new(&params).expect("default reference-orbit parameters are valid");
    let anchor = [Dd::new(0.35), Dd::new(-0.21), Dd::new(0.14)];
    resolved.orbit_records(anchor, params.max_iters).0.len()
}

impl StackParams {
    #[allow(clippy::approx_constant, clippy::float_cmp)] // Mirrors shader literals and its exact zero sentinel.
    fn with_live_state(&self, state: &AnalyzerStateSnapshot) -> Self {
        let mut next = self.clone();
        let mut formulas = next.formulas;
        let mut rates = next.rates;
        for index in 0..4 {
            if let Some(value) = state.long(&format!("formula{index}")) {
                formulas[index] = i64::from(value);
            }
            if let Some(value) = state.float(&format!("rate{index}")) {
                rates[index] = value.clamp(0.0, 8.0) as i64;
            }
        }
        let order = match state.long("stack_order").unwrap_or(0) {
            1 => [1, 0, 2, 3],
            2 => [0, 1, 3, 2],
            3 => [1, 0, 3, 2],
            4 => [2, 3, 0, 1],
            5 => [1, 2, 3, 0],
            6 => [3, 0, 1, 2],
            7 => [3, 2, 1, 0],
            _ => [0, 1, 2, 3],
        };
        next.formulas = order.map(|index| formulas[index]);
        next.rates = order.map(|index| rates[index]);

        let float = |name: &str, fallback: f64| state.float(name).map_or(fallback, f64::from);
        next.scale = float("scale", next.scale);
        next.fold_limit = float("fold_limit", next.fold_limit);
        next.min_radius = float("min_radius", next.min_radius);
        next.fixed_radius = float("fixed_radius", next.fixed_radius);
        next.power = float("power", next.power).clamp(2.0, 12.0);
        let explicit_zoom_exp = float("zoom_exp", 0.0).clamp(0.0, 10_000.0);
        let flight_max_exp = float("flight_max_exp", next.zoom_exp).clamp(0.0, 10_000.0);
        next.zoom_exp = if explicit_zoom_exp > 0.0 {
            explicit_zoom_exp
        } else {
            flight_max_exp
        };
        next.stack_cap = float("stack_cap", next.stack_cap).clamp(1.0, SHADER_FOLD_CEILING as f64);
        // A phase-driven flight changes depth only in the shader. Until the
        // payload carries swept or multi-depth certificates, only an explicit
        // static depth may enable the certificate atlas.
        next.certificate_enabled = explicit_zoom_exp > 0.0 && next.certificates_allowed;
        next.offset = [
            float("offset_x", next.offset[0]),
            float("offset_y", next.offset[1]),
            float("offset_z", next.offset[2]),
        ];
        next.lin = [
            float("lin_x", next.lin[0]),
            float("lin_y", next.lin[1]),
            float("lin_z", next.lin[2]),
        ];
        next.lin_mix = float("lin_mix", next.lin_mix);
        next.rot = [
            float("rot_a", next.rot[0]),
            float("rot_b", next.rot[1]),
            float("rot_c", next.rot[2]),
        ];
        next.rot_w = [
            float("rot_xw", next.rot_w[0]),
            float("rot_yw", next.rot_w[1]),
            float("rot_zw", next.rot_w[2]),
        ];
        next.cocube = float("cocube", next.cocube);
        next.bailout = float("bailout", next.bailout);
        next.julia_amount = float("julia_amount", next.julia_amount).clamp(0.0, 1.0);
        next.julia = [
            float("julia_x", next.julia[0]),
            float("julia_y", next.julia[1]),
            float("julia_z", next.julia[2]),
        ];
        next.max_iters =
            float("max_iters", next.max_iters as f64).clamp(1.0, MAX_ITERS_LIMIT as f64) as usize;

        next
    }
}

/// Resolved constants, so the inner loop does no parsing or conversion.
struct Resolved {
    cycle: Vec<Slot>,
    scale: Dd,
    fold_limit: Dd,
    min_r2: Dd,
    fix_r2: Dd,
    offset: V3,
    bail2: Dd,
    seed_sample_weight: Dd,
    julia_seed: V3,
    cocube: Dd,
    lin: V3,
    lin_mix: Dd,
    /// Cosine and sine per plane, precomputed.
    rot: [(Dd, Dd); 3],
    rot_w: [(Dd, Dd); 3],
}

/// A w-plane angle at exactly +/- pi/2 collapses its spatial axis, so the shader
/// nudges it clear of that value. Mirrored here exactly, because a reference
/// orbit that disagrees with the shader by any amount is not a reference orbit.
#[allow(clippy::approx_constant)] // This is the shader's rounded HALF_PI literal.
fn w_plane_angle(a: f64) -> f64 {
    const HALF_PI: f64 = 1.570_796_3;
    const MARGIN: f64 = 0.06;
    let mut m = a.abs();
    if m > HALF_PI - MARGIN && m < HALF_PI + MARGIN {
        m = if m < HALF_PI {
            HALF_PI - MARGIN
        } else {
            HALF_PI + MARGIN
        };
    }
    if a < 0.0 {
        -m
    } else {
        m
    }
}

/// `mat2(c, -s, s, c)` applied to a pair, matching the shader's `rot`.
fn rot2(x: Dd, y: Dd, cs: (Dd, Dd)) -> (Dd, Dd) {
    let (c, sn) = cs;
    (x.mul(c).sub(y.mul(sn)), x.mul(sn).add(y.mul(c)))
}

impl Resolved {
    fn new(p: &StackParams) -> anyhow::Result<Self> {
        // Round-robin exactly as the shader does: each slot contributes `rate`
        // consecutive iterations, and the pattern repeats.
        let mut cycle = Vec::new();
        for (idx, &f) in p.formulas.iter().enumerate() {
            let slot = Slot::from_shader_index(f).ok_or_else(|| {
                anyhow::anyhow!(
                    "slot formula {f} is not supported by the reference orbit preprocessor"
                )
            })?;
            let rate = p.rates[idx].clamp(0, 8);
            for _ in 0..rate {
                cycle.push(slot);
            }
        }
        if cycle.is_empty() {
            anyhow::bail!("every slot is silenced, so there is no orbit to compute");
        }
        Ok(Self {
            cycle,
            scale: Dd::new(p.scale),
            fold_limit: Dd::new(p.fold_limit),
            min_r2: Dd::new(p.min_radius * p.min_radius),
            fix_r2: Dd::new(p.fixed_radius * p.fixed_radius),
            offset: [
                Dd::new(p.offset[0]),
                Dd::new(p.offset[1]),
                Dd::new(p.offset[2]),
            ],
            bail2: Dd::new(p.bailout * p.bailout),
            seed_sample_weight: Dd::new(1.0 - p.julia_amount),
            julia_seed: p.julia.map(Dd::new),
            cocube: Dd::new(p.cocube),
            lin: [Dd::new(p.lin[0]), Dd::new(p.lin[1]), Dd::new(p.lin[2])],
            lin_mix: Dd::new(p.lin_mix),
            rot: [0, 1, 2].map(|k| (Dd::new(p.rot[k].cos()), Dd::new(p.rot[k].sin()))),
            rot_w: [0, 1, 2].map(|k| {
                let a = w_plane_angle(p.rot_w[k]);
                (Dd::new(a.cos()), Dd::new(a.sin()))
            }),
        })
    }

    /// One iteration, returning every reference value an identity reads.
    ///
    /// `mid` is the point after the fold or sort stage and `m_ref` the radial
    /// multiplier applied to the reference; both were previously recomputed in
    /// the shader from `post`, which is what broke the scheme. Slots with no
    /// branch stage report `mid == p` and a unit multiplier.
    #[allow(clippy::many_single_char_names)] // Names mirror the compact shader recurrence.
    fn step_detailed(&self, p: V3, seed: V3, n: usize) -> StepRecord {
        let slot = self.cycle[n % self.cycle.len()];
        let sc = self.scale;
        let l = self.fold_limit;
        let one = Dd::new(1.0);
        let mut record = match slot {
            Slot::Mandelbulb => {
                unreachable!("Mandelbulb records use the arbitrary-precision backend")
            }
            Slot::Mandelbox => {
                let mid = box_fold(p, l);
                let q = v_dot(mid, mid);
                let m = radial_multiplier(q, self.min_r2, self.fix_r2, self.fix_r2);
                let post = v_add(v_scale(v_scale(mid, m), sc), seed);
                StepRecord {
                    mid,
                    mid2: post,
                    post,
                    m_ref: m,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
            Slot::AmazingBox => {
                let mut mid = p;
                let two = Dd::new(2.0);
                for component in mid.iter_mut().take(2) {
                    *component = fold_component(*component, l, two);
                }
                let q = v_dot(mid, mid);
                let m = radial_multiplier(q, self.min_r2, self.fix_r2, sc);
                let post = v_add(v_scale(mid, m), self.offset);
                StepRecord {
                    mid,
                    mid2: post,
                    post,
                    m_ref: m,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
            Slot::PseudoKleinian => {
                let mid = box_fold(p, l);
                let q = v_dot(mid, mid);
                let clamped = if q.cmp(self.min_r2) == Ordering::Less {
                    self.min_r2
                } else {
                    q
                };
                let m = one.div(clamped);
                let post = v_sub(v_scale(mid, m), self.offset);
                StepRecord {
                    mid,
                    mid2: post,
                    post,
                    m_ref: m,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
            Slot::Menger => {
                let mut mid = [p[0].abs(), p[1].abs(), p[2].abs()];
                sort_desc(&mut mid, 0, 1);
                sort_desc(&mut mid, 0, 2);
                sort_desc(&mut mid, 1, 2);
                let k = sc.sub(one);
                let shift = self.offset[2].mul(k);
                let mid2 = [
                    mid[0].mul(sc).sub(self.offset[0].mul(k)),
                    mid[1].mul(sc).sub(self.offset[1].mul(k)),
                    mid[2].mul(sc).sub(shift),
                ];
                let mut post = mid2;
                if post[2].cmp(shift.mul_f(-0.5)) == Ordering::Less {
                    post[2] = post[2].add(shift);
                }
                StepRecord {
                    mid,
                    mid2,
                    post,
                    m_ref: one,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
            Slot::Sierpinski => {
                // Three symmetry planes, then a uniform scale. Nothing branches
                // after the scale, so the post-reflect point is the whole of
                // what an identity needs.
                let mut mid = p;
                cond_reflect_pair(&mut mid, 0, 1);
                cond_reflect_pair(&mut mid, 0, 2);
                cond_reflect_pair(&mut mid, 2, 1);
                let k = sc.sub(one);
                let post = [
                    mid[0].mul(sc).sub(self.offset[0].mul(k)),
                    mid[1].mul(sc).sub(self.offset[1].mul(k)),
                    mid[2].mul(sc).sub(self.offset[2].mul(k)),
                ];
                StepRecord {
                    mid,
                    mid2: post,
                    post,
                    m_ref: one,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
            Slot::CoCube => {
                // Partial sort, then a reflection about the corner, then a
                // uniform scale. Again nothing branches after the scale.
                let mut mid = [p[0].abs(), p[1].abs(), p[2].abs()];
                sort_desc(&mut mid, 0, 1);
                sort_desc(&mut mid, 1, 2);
                let corner = self.cocube;
                mid[2] = corner.sub(mid[2].sub(corner).abs());
                let k = sc.sub(one);
                let post = [
                    mid[0].mul(sc).sub(self.offset[0].mul(k)),
                    mid[1].mul(sc).sub(self.offset[1].mul(k)),
                    mid[2].mul(sc).sub(self.offset[2].mul(k)),
                ];
                StepRecord {
                    mid,
                    mid2: post,
                    post,
                    m_ref: one,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
            _ => {
                // Menger branches *after* its scale, on the trailing
                // conditional shift, so the post-sort point alone is not enough
                // and it needs a fourth record group. The rotations and the
                // linear recombination have no branch at all and need no
                // reference beyond the endpoint. Both report `mid == post`.
                let post = self.step(p, seed, n);
                StepRecord {
                    mid: post,
                    mid2: post,
                    post,
                    m_ref: one,
                    branches: 0,
                    margins: [Dd::default(); MARGIN_COUNT],
                }
            }
        };
        let (margins, branches) =
            transport_margins_dd(slot, p, record.mid, record.mid2, record.post, self);
        record.margins = margins;
        record.branches = branches;
        record
    }

    /// One iteration of the round-robin stack, in double-double.
    ///
    /// Mirrors the shader's `stackDE` body for the conformal slots. Kept in one
    /// function rather than one per slot so the correspondence stays checkable
    /// by reading the two side by side.
    #[allow(clippy::many_single_char_names)] // Names mirror the compact shader recurrence.
    fn step(&self, p: V3, seed: V3, n: usize) -> V3 {
        let slot = self.cycle[n % self.cycle.len()];
        let sc = self.scale;
        let l = self.fold_limit;
        match slot {
            Slot::Mandelbulb => {
                unreachable!("Mandelbulb records use the arbitrary-precision backend")
            }
            Slot::Off => p,
            Slot::Rotate => {
                // Ordered plane rotations, xy then yz then xz, applied in that
                // order because each sees the result of the last.
                let mut a = p;
                let (x, y) = rot2(a[0], a[1], self.rot[0]);
                a[0] = x;
                a[1] = y;
                let (y2, z) = rot2(a[1], a[2], self.rot[1]);
                a[1] = y2;
                a[2] = z;
                let (x2, z2) = rot2(a[0], a[2], self.rot[2]);
                a[0] = x2;
                a[2] = z2;
                a
            }
            Slot::Rotate4d => {
                // Embed as (p, 0), rotate in the three planes through the
                // hidden axis, discard the fourth component.
                let mut q = [p[0], p[1], p[2], Dd::default()];
                for (k, cs) in self.rot_w.iter().enumerate() {
                    let (a, w) = rot2(q[k], q[3], *cs);
                    q[k] = a;
                    q[3] = w;
                }
                [q[0], q[1], q[2]]
            }
            Slot::Mandelbox => {
                let folded = box_fold(p, l);
                let q = v_dot(folded, folded);
                let m = radial_multiplier(q, self.min_r2, self.fix_r2, self.fix_r2);
                v_add(v_scale(v_scale(folded, m), sc), seed)
            }
            Slot::AmazingBox => {
                let mut f = p;
                let two = Dd::new(2.0);
                for component in f.iter_mut().take(2) {
                    *component = fold_component(*component, l, two);
                }
                let q = v_dot(f, f);
                let m = radial_multiplier(q, self.min_r2, self.fix_r2, sc);
                v_add(v_scale(f, m), self.offset)
            }
            Slot::PseudoKleinian => {
                let folded = box_fold(p, l);
                let q = v_dot(folded, folded);
                let clamped = if q.cmp(self.min_r2) == Ordering::Less {
                    self.min_r2
                } else {
                    q
                };
                let m = Dd::new(1.0).div(clamped);
                v_sub(v_scale(folded, m), self.offset)
            }
            Slot::Menger => {
                let mut a = [p[0].abs(), p[1].abs(), p[2].abs()];
                sort_desc(&mut a, 0, 1);
                sort_desc(&mut a, 0, 2);
                sort_desc(&mut a, 1, 2);
                let shift = self.offset[2].mul(sc.sub(Dd::new(1.0)));
                let mut out = [
                    a[0].mul(sc).sub(self.offset[0].mul(sc.sub(Dd::new(1.0)))),
                    a[1].mul(sc).sub(self.offset[1].mul(sc.sub(Dd::new(1.0)))),
                    a[2].mul(sc).sub(shift),
                ];
                if out[2].cmp(shift.mul_f(-0.5)) == Ordering::Less {
                    out[2] = out[2].add(shift);
                }
                out
            }
            Slot::Sierpinski => {
                let mut a = p;
                cond_reflect_pair(&mut a, 0, 1);
                cond_reflect_pair(&mut a, 0, 2);
                cond_reflect_pair(&mut a, 2, 1);
                let k = sc.sub(Dd::new(1.0));
                [
                    a[0].mul(sc).sub(self.offset[0].mul(k)),
                    a[1].mul(sc).sub(self.offset[1].mul(k)),
                    a[2].mul(sc).sub(self.offset[2].mul(k)),
                ]
            }
            Slot::CoCube => {
                let mut a = [p[0].abs(), p[1].abs(), p[2].abs()];
                sort_desc(&mut a, 0, 1);
                sort_desc(&mut a, 1, 2);
                // p.z = corner - |p.z - corner|. Its own parameter: an earlier
                // version reused the offset's z, which is a different number.
                let corner = self.cocube;
                a[2] = corner.sub(a[2].sub(corner).abs());
                let k = sc.sub(Dd::new(1.0));
                [
                    a[0].mul(sc).sub(self.offset[0].mul(k)),
                    a[1].mul(sc).sub(self.offset[1].mul(k)),
                    a[2].mul(sc).sub(self.offset[2].mul(k)),
                ]
            }
            Slot::LinCombine => [
                p[0].mul(self.lin[0]).add(p[1].mul(self.lin_mix)),
                p[1].mul(self.lin[1]).add(p[2].mul(self.lin_mix)),
                p[2].mul(self.lin[2]).add(p[0].mul(self.lin_mix)),
            ],
        }
    }

    /// Iterate from `c`, returning the orbit and whether it survived.
    fn orbit(&self, c: V3, iters: usize) -> (Vec<V3>, bool) {
        let (recs, alive) = self.orbit_records(c, iters);
        (recs.into_iter().map(|r| r.post).collect(), alive)
    }

    /// The orbit with every intermediate an identity reads.
    fn orbit_records(&self, c: V3, iters: usize) -> (Vec<StepRecord>, bool) {
        let mut p = c;
        let seed = v_add(
            v_scale(c, self.seed_sample_weight),
            v_scale(self.julia_seed, Dd::new(1.0).sub(self.seed_sample_weight)),
        );
        let mut out = Vec::with_capacity(iters);
        for n in 0..iters {
            let rec = self.step_detailed(p, seed, n);
            p = rec.post;
            out.push(rec);
            if v_dot(p, p).cmp(self.bail2) == Ordering::Greater {
                return (out, false);
            }
        }
        (out, true)
    }
}

fn fold_component(v: Dd, l: Dd, two: Dd) -> Dd {
    // clamp(v, -l, l) * 2 - v
    let c = if v.cmp(l) == Ordering::Greater {
        l
    } else if v.cmp(l.neg()) == Ordering::Less {
        l.neg()
    } else {
        v
    };
    c.mul(two).sub(v)
}

fn box_fold(p: V3, l: Dd) -> V3 {
    let two = Dd::new(2.0);
    [
        fold_component(p[0], l, two),
        fold_component(p[1], l, two),
        fold_component(p[2], l, two),
    ]
}

/// `k / clamp(q, lo, hi)`, the one form all three radial folds share.
fn radial_multiplier(q: Dd, lo: Dd, hi: Dd, k: Dd) -> Dd {
    let qc = if q.cmp(lo) == Ordering::Less {
        lo
    } else if q.cmp(hi) == Ordering::Greater {
        hi
    } else {
        q
    };
    k.div(qc)
}

fn sort_desc(a: &mut V3, i: usize, j: usize) {
    if a[i].cmp(a[j]) == Ordering::Less {
        a.swap(i, j);
    }
}

#[allow(clippy::many_single_char_names)] // Axis indices keep the reflection identity readable.
fn cond_reflect_pair(a: &mut V3, i: usize, j: usize) {
    if a[i].add(a[j]).cmp(Dd::default()) == Ordering::Less {
        let (x, y) = (a[i], a[j]);
        a[i] = y.neg();
        a[j] = x.neg();
    }
}

// ── Arbitrary-precision continuous-power orbit ─────────────────────────────

type Hp = FBig<HalfEven, 2>;
type ApV3 = [Hp; 3];

#[derive(Clone)]
struct ApBulbRecord {
    radius: Hp,
    theta: Hp,
    phi: Hp,
    radius_power: Hp,
    /// -1 below the principal seam, +1 above it, zero on the seam/axis.
    seam_side: i32,
    /// `atan2` returns the principal branch, so the reference winding is zero.
    principal_winding: i32,
}

#[derive(Clone)]
struct ApStepRecord {
    pre: ApV3,
    q_pre: Hp,
    mid: ApV3,
    q_mid: Hp,
    mid2: ApV3,
    post: ApV3,
    m_ref: Hp,
    branches: u32,
    bulb: Option<ApBulbRecord>,
    margins: [Hp; MARGIN_COUNT],
}

/// Arithmetic context for the reference path. Every operation rounds to the
/// same requested precision. Transcendentals therefore gain precision with the
/// requested zoom rather than silently falling back to binary64.
struct ApMath {
    precision: usize,
    context: Context<HalfEven>,
}

impl ApMath {
    fn with_precision(precision: usize) -> Self {
        Self {
            precision,
            context: Context::new(precision),
        }
    }

    fn for_zoom(zoom_exp: f64) -> Self {
        // Decimal depth converted to bits, plus enough guard space for angle
        // reduction and the accumulated error of a long mixed orbit.
        let precision = ((zoom_exp.max(0.0) + 24.0) * std::f64::consts::LOG2_10)
            .ceil()
            .max(256.0) as usize;
        let precision = precision.min(65_536);
        Self::with_precision(precision)
    }

    fn for_anchor_search(zoom_exp: f64, survival: usize, power: f64) -> Self {
        let depth_bits = (zoom_exp.max(0.0) * std::f64::consts::LOG2_10).ceil();
        let orbit_bits = survival as f64 * power.max(2.0).log2();
        let precision = (depth_bits.max(orbit_bits) + 256.0)
            .ceil()
            .clamp(256.0, 65_536.0) as usize;
        Self::with_precision(precision)
    }

    fn f(&self, value: f64) -> Hp {
        Hp::try_from(value)
            .expect("finite shader parameters convert to arbitrary precision")
            .with_precision(self.precision)
            .value()
    }

    fn decimal(&self, value: &str) -> anyhow::Result<Hp> {
        type Decimal = FBig<HalfEven, 10>;
        let decimal = Decimal::from_str(value.trim()).map_err(|error| {
            anyhow::anyhow!("invalid arbitrary-precision decimal {value:?}: {error}")
        })?;
        Ok(decimal.with_base_and_precision::<2>(self.precision).value())
    }

    fn to_decimal(&self, value: &Hp) -> String {
        let precision = self.precision.saturating_mul(31).div_ceil(100) + 8;
        value
            .clone()
            .with_base_and_precision::<10>(precision)
            .value()
            .to_string()
    }

    fn zero(&self) -> Hp {
        self.f(0.0)
    }

    fn one(&self) -> Hp {
        self.f(1.0)
    }

    fn add(&self, a: &Hp, b: &Hp) -> Hp {
        self.context
            .add(a.repr(), b.repr())
            .expect("finite arbitrary-precision addition succeeds")
            .value()
    }

    fn sub(&self, a: &Hp, b: &Hp) -> Hp {
        self.context
            .sub(a.repr(), b.repr())
            .expect("finite arbitrary-precision subtraction succeeds")
            .value()
    }

    fn mul(&self, a: &Hp, b: &Hp) -> Hp {
        self.context
            .mul(a.repr(), b.repr())
            .expect("finite arbitrary-precision multiplication succeeds")
            .value()
    }

    fn div(&self, a: &Hp, b: &Hp) -> Hp {
        self.context
            .div(a.repr(), b.repr())
            .expect("guarded arbitrary-precision division succeeds")
            .value()
    }

    fn sqrt(&self, value: &Hp) -> Hp {
        self.context
            .sqrt(value.repr())
            .expect("non-negative arbitrary-precision square root succeeds")
            .value()
    }

    fn pow(&self, base: &Hp, exponent: &Hp) -> Hp {
        self.context
            .powf(base.repr(), exponent.repr(), None)
            .expect("non-negative arbitrary-precision power succeeds")
            .value()
    }

    fn sin(&self, value: &Hp) -> Hp {
        self.context
            .sin(value.repr(), None)
            .expect("finite arbitrary-precision sine succeeds")
            .value()
    }

    fn cos(&self, value: &Hp) -> Hp {
        self.context
            .cos(value.repr(), None)
            .expect("finite arbitrary-precision cosine succeeds")
            .value()
    }

    fn acos(&self, value: &Hp) -> Hp {
        self.context
            .acos(value.repr(), None)
            .expect("clamped arbitrary-precision arccosine succeeds")
            .value()
    }

    fn atan2(&self, y: &Hp, x: &Hp) -> Hp {
        if Self::is_zero(y) && Self::is_zero(x) {
            return self.zero();
        }
        self.context
            .atan2(y.repr(), x.repr(), None)
            .expect("guarded arbitrary-precision atan2 succeeds")
            .value()
    }

    fn dot(&self, a: &ApV3, b: &ApV3) -> Hp {
        let xy = self.add(&self.mul(&a[0], &b[0]), &self.mul(&a[1], &b[1]));
        self.add(&xy, &self.mul(&a[2], &b[2]))
    }

    fn add_v(&self, a: &ApV3, b: &ApV3) -> ApV3 {
        std::array::from_fn(|index| self.add(&a[index], &b[index]))
    }

    fn sub_v(&self, a: &ApV3, b: &ApV3) -> ApV3 {
        std::array::from_fn(|index| self.sub(&a[index], &b[index]))
    }

    fn scale_v(&self, value: &ApV3, scale: &Hp) -> ApV3 {
        std::array::from_fn(|index| self.mul(&value[index], scale))
    }

    fn to_f64(value: &Hp) -> f64 {
        value.to_f64().value()
    }

    fn to_f32(value: &Hp) -> f32 {
        value.to_f32().value()
    }

    fn branch(value: &Hp, boundary: &Hp) -> Ordering {
        value.partial_cmp(boundary).unwrap_or(Ordering::Equal)
    }

    fn abs(value: &Hp) -> Hp {
        value.clone().abs()
    }

    fn is_zero(value: &Hp) -> bool {
        value.repr().significand().is_zero()
    }
}

struct ApResolved {
    cycle: Vec<Slot>,
    scale: Hp,
    fold_limit: Hp,
    min_r2: Hp,
    fix_r2: Hp,
    offset: ApV3,
    bail2: Hp,
    power: Hp,
    sample_weight: Hp,
    julia_seed: ApV3,
    cocube: Hp,
    lin: ApV3,
    lin_mix: Hp,
    rot: [(Hp, Hp); 3],
    rot_w: [(Hp, Hp); 3],
}

impl ApResolved {
    fn new(params: &StackParams, math: &mut ApMath) -> anyhow::Result<Self> {
        let mut cycle = Vec::new();
        for (index, formula) in params.formulas.iter().enumerate() {
            let slot = Slot::from_shader_index(*formula)
                .ok_or_else(|| anyhow::anyhow!("unsupported formula index {formula}"))?;
            for _ in 0..params.rates[index].clamp(0, 8) {
                cycle.push(slot);
            }
        }
        if cycle.is_empty() {
            anyhow::bail!("every slot is silenced, so there is no orbit to compute");
        }

        let make_rotation = |angle: f64| {
            let angle = math.f(angle);
            let cosine = math.cos(&angle);
            let sine = math.sin(&angle);
            (cosine, sine)
        };
        let rot = std::array::from_fn(|index| make_rotation(params.rot[index]));
        let rot_w = std::array::from_fn(|index| make_rotation(w_plane_angle(params.rot_w[index])));
        Ok(Self {
            cycle,
            scale: math.f(params.scale),
            fold_limit: math.f(params.fold_limit),
            min_r2: math.f(params.min_radius * params.min_radius),
            fix_r2: math.f(params.fixed_radius * params.fixed_radius),
            offset: params.offset.map(|value| math.f(value)),
            bail2: math.f(params.bailout * params.bailout),
            power: math.f(params.power),
            sample_weight: math.f(1.0 - params.julia_amount),
            julia_seed: params.julia.map(|value| math.f(value)),
            cocube: math.f(params.cocube),
            lin: params.lin.map(|value| math.f(value)),
            lin_mix: math.f(params.lin_mix),
            rot,
            rot_w,
        })
    }

    fn fold_component(math: &ApMath, value: &Hp, limit: &Hp) -> Hp {
        let clamped = match ApMath::branch(value, limit) {
            Ordering::Greater => limit.clone(),
            _ => match ApMath::branch(value, &(-limit.clone())) {
                Ordering::Less => -limit.clone(),
                _ => value.clone(),
            },
        };
        math.sub(&math.mul(&clamped, &math.f(2.0)), value)
    }

    fn box_fold(&self, math: &ApMath, point: &ApV3) -> ApV3 {
        std::array::from_fn(|index| Self::fold_component(math, &point[index], &self.fold_limit))
    }

    fn radial_multiplier(&self, math: &ApMath, radius2: &Hp, numerator: &Hp) -> (Hp, u32) {
        if ApMath::branch(radius2, &self.min_r2) == Ordering::Less {
            (math.div(numerator, &self.min_r2), 0)
        } else if ApMath::branch(radius2, &self.fix_r2) == Ordering::Less {
            (math.div(numerator, radius2), 1)
        } else {
            (math.div(numerator, &self.fix_r2), 2)
        }
    }

    fn rotate_pair(math: &ApMath, x: &Hp, y: &Hp, rotation: &(Hp, Hp)) -> (Hp, Hp) {
        let (cosine, sine) = rotation;
        (
            math.sub(&math.mul(x, cosine), &math.mul(y, sine)),
            math.add(&math.mul(x, sine), &math.mul(y, cosine)),
        )
    }

    fn transport_margins(
        &self,
        math: &ApMath,
        slot: Slot,
        record: &ApStepRecord,
    ) -> [Hp; MARGIN_COUNT] {
        let zero = math.zero();
        let mut margins = std::array::from_fn(|_| zero.clone());
        match slot {
            Slot::Mandelbox | Slot::PseudoKleinian => {
                for component in 0..3 {
                    margins[2 * component] = math.add(&record.pre[component], &self.fold_limit);
                    margins[2 * component + 1] = math.sub(&self.fold_limit, &record.pre[component]);
                }
                margins[6] = math.sub(&record.q_mid, &self.min_r2);
                margins[7] = math.sub(&self.fix_r2, &record.q_mid);
            }
            Slot::AmazingBox => {
                for component in 0..2 {
                    margins[2 * component] = math.add(&record.pre[component], &self.fold_limit);
                    margins[2 * component + 1] = math.sub(&self.fold_limit, &record.pre[component]);
                }
                margins[4] = math.sub(&record.q_mid, &self.min_r2);
                margins[5] = math.sub(&self.fix_r2, &record.q_mid);
            }
            Slot::Menger | Slot::CoCube => {
                let mut value: ApV3 = std::array::from_fn(|index| ApMath::abs(&record.pre[index]));
                margins[..3].clone_from_slice(&record.pre);
                let pairs: &[(usize, usize)] = if slot == Slot::Menger {
                    &[(0, 1), (0, 2), (1, 2)]
                } else {
                    &[(0, 1), (1, 2)]
                };
                for (index, &(left, right)) in pairs.iter().enumerate() {
                    margins[3 + index] = math.sub(&value[left], &value[right]);
                    if ApMath::branch(&margins[3 + index], &zero) == Ordering::Less {
                        value.swap(left, right);
                    }
                }
                if slot == Slot::Menger {
                    let shift = math.mul(&self.offset[2], &math.sub(&self.scale, &math.one()));
                    margins[6] = math.add(&record.mid2[2], &math.mul(&shift, &math.f(0.5)));
                } else {
                    margins[5] = math.sub(&value[2], &self.cocube);
                }
            }
            Slot::Sierpinski => {
                let mut value = record.pre.clone();
                for (index, (left, right)) in [(0, 1), (0, 2), (2, 1)].into_iter().enumerate() {
                    margins[index] = math.add(&value[left], &value[right]);
                    if ApMath::branch(&margins[index], &zero) == Ordering::Less {
                        let old = value[left].clone();
                        value[left] = -value[right].clone();
                        value[right] = -old;
                    }
                }
            }
            Slot::Mandelbulb => {
                let bulb = record
                    .bulb
                    .as_ref()
                    .expect("Mandelbulb record carries polar state");
                // Bulb margin indices:
                // 0: `2 - radius`, the pre-step radius-2 escape decision.
                // 1: `radius - 1e-6`, the safe-radius max decision.
                // 2: `rho = sqrt(x^2 + y^2)`, distance to the polar axis.
                // 3: x, distance to the negative-half-plane gate.
                // 4: y, signed distance to the principal azimuth seam.
                // 5-10: reserved. 11: the universal post-slot bailout margin.
                margins[0] = math.sub(&math.f(2.0), &bulb.radius);
                margins[1] = math.sub(&bulb.radius, &math.f(1.0e-6));
                let rho2 = math.add(
                    &math.mul(&record.pre[0], &record.pre[0]),
                    &math.mul(&record.pre[1], &record.pre[1]),
                );
                margins[2] = math.sqrt(&rho2);
                margins[3].clone_from(&record.pre[0]);
                margins[4].clone_from(&record.pre[1]);
            }
            Slot::Off | Slot::LinCombine | Slot::Rotate | Slot::Rotate4d => {}
        }
        margins[BAILOUT_MARGIN_INDEX] =
            math.sub(&self.bail2, &math.dot(&record.post, &record.post));
        margins
    }

    fn step(&self, math: &mut ApMath, point: &ApV3, seed: &ApV3, iteration: usize) -> ApStepRecord {
        let slot = self.cycle[iteration % self.cycle.len()];
        let one = math.one();
        let mut record = ApStepRecord {
            pre: point.clone(),
            q_pre: math.dot(point, point),
            mid: point.clone(),
            q_mid: math.dot(point, point),
            mid2: point.clone(),
            post: point.clone(),
            m_ref: one.clone(),
            branches: 0,
            bulb: None,
            margins: std::array::from_fn(|_| math.zero()),
        };
        match slot {
            Slot::Off => {}
            Slot::Mandelbox => {
                let fold = std::array::from_fn(|index| {
                    match ApMath::branch(&point[index], &self.fold_limit) {
                        Ordering::Less
                            if ApMath::branch(&point[index], &(-self.fold_limit.clone()))
                                == Ordering::Less =>
                        {
                            0
                        }
                        Ordering::Greater => 2,
                        _ => 1,
                    }
                });
                let mid = self.box_fold(math, point);
                let radius2 = math.dot(&mid, &mid);
                let (multiplier, radial) = self.radial_multiplier(math, &radius2, &self.fix_r2);
                let post = math.add_v(
                    &math.scale_v(&math.scale_v(&mid, &multiplier), &self.scale),
                    seed,
                );
                record.mid = mid;
                record.mid2.clone_from(&post);
                record.post = post;
                record.m_ref = multiplier;
                record.branches = pack_branches(fold, radial);
            }
            Slot::AmazingBox => {
                let fold = std::array::from_fn(|index| {
                    if index >= 2 {
                        return 0;
                    }
                    match ApMath::branch(&point[index], &self.fold_limit) {
                        Ordering::Less
                            if ApMath::branch(&point[index], &(-self.fold_limit.clone()))
                                == Ordering::Less =>
                        {
                            0
                        }
                        Ordering::Greater => 2,
                        _ => 1,
                    }
                });
                let mut mid = point.clone();
                for component in mid.iter_mut().take(2) {
                    *component = Self::fold_component(math, component, &self.fold_limit);
                }
                let radius2 = math.dot(&mid, &mid);
                let (multiplier, radial) = self.radial_multiplier(math, &radius2, &self.scale);
                let post = math.add_v(&math.scale_v(&mid, &multiplier), &self.offset);
                record.mid = mid;
                record.mid2.clone_from(&post);
                record.post = post;
                record.m_ref = multiplier;
                record.branches = pack_branches(fold, radial);
            }
            Slot::Menger | Slot::Sierpinski | Slot::CoCube => {
                let mut mid = if slot == Slot::Sierpinski {
                    point.clone()
                } else {
                    std::array::from_fn(|index| ApMath::abs(&point[index]))
                };
                if slot == Slot::Sierpinski {
                    for (index, (left, right)) in [(0, 1), (0, 2), (2, 1)].into_iter().enumerate() {
                        if ApMath::branch(&math.add(&mid[left], &mid[right]), &math.zero())
                            == Ordering::Less
                        {
                            let x = mid[left].clone();
                            mid[left] = -mid[right].clone();
                            mid[right] = -x;
                            record.branches |= 1 << index;
                        }
                    }
                } else {
                    for (index, component) in point.iter().enumerate() {
                        if ApMath::branch(component, &math.zero()) == Ordering::Less {
                            record.branches |= 1 << index;
                        }
                    }
                    let pairs: &[(usize, usize)] = if slot == Slot::Menger {
                        &[(0, 1), (0, 2), (1, 2)]
                    } else {
                        &[(0, 1), (1, 2)]
                    };
                    for (index, &(left, right)) in pairs.iter().enumerate() {
                        if ApMath::branch(&mid[left], &mid[right]) == Ordering::Less {
                            mid.swap(left, right);
                            record.branches |= 1 << (3 + index);
                        }
                    }
                    if slot == Slot::CoCube {
                        if ApMath::branch(&mid[2], &self.cocube) == Ordering::Less {
                            record.branches |= 1 << 5;
                        }
                        mid[2] =
                            math.sub(&self.cocube, &ApMath::abs(&math.sub(&mid[2], &self.cocube)));
                    }
                }
                let scale_minus_one = math.sub(&self.scale, &one);
                let mut post = std::array::from_fn(|index| {
                    math.sub(
                        &math.mul(&mid[index], &self.scale),
                        &math.mul(&self.offset[index], &scale_minus_one),
                    )
                });
                record.mid2.clone_from(&post);
                if slot == Slot::Menger {
                    let shift = math.mul(&self.offset[2], &scale_minus_one);
                    let threshold = math.mul(&shift, &math.f(-0.5));
                    if ApMath::branch(&post[2], &threshold) == Ordering::Less {
                        post[2] = math.add(&post[2], &shift);
                        record.branches |= 1 << 6;
                    }
                }
                record.mid = mid;
                record.post = post;
            }
            Slot::Mandelbulb => {
                let radius2 = math.dot(point, point);
                let radius = math.sqrt(&radius2);
                let safe_radius = if ApMath::branch(&radius, &math.f(1.0e-6)) == Ordering::Less {
                    math.f(1.0e-6)
                } else {
                    radius.clone()
                };
                let ratio = math.div(&point[2], &safe_radius);
                let ratio = if ApMath::branch(&ratio, &one) == Ordering::Greater {
                    one.clone()
                } else if ApMath::branch(&ratio, &(-one.clone())) == Ordering::Less {
                    -one.clone()
                } else {
                    ratio
                };
                let theta = math.acos(&ratio);
                let phi = math.atan2(&point[1], &point[0]);
                let radius_power = math.pow(&radius, &self.power);
                let powered_theta = math.mul(&theta, &self.power);
                let powered_phi = math.mul(&phi, &self.power);
                let sin_theta = math.sin(&powered_theta);
                let cos_phi = math.cos(&powered_phi);
                let sin_phi = math.sin(&powered_phi);
                let cos_theta = math.cos(&powered_theta);
                let direction = [
                    math.mul(&sin_theta, &cos_phi),
                    math.mul(&sin_theta, &sin_phi),
                    cos_theta,
                ];
                let post = math.add_v(&math.scale_v(&direction, &radius_power), seed);
                let seam_side = if ApMath::branch(&point[0], &math.zero()) == Ordering::Less {
                    match ApMath::branch(&point[1], &math.zero()) {
                        Ordering::Less => -1,
                        Ordering::Greater => 1,
                        Ordering::Equal => 0,
                    }
                } else {
                    0
                };
                record.mid.clone_from(point);
                record.mid2 = direction;
                record.post = post;
                record.m_ref.clone_from(&radius_power);
                record.branches = u32::try_from(seam_side + 1).unwrap_or(1);
                record.bulb = Some(ApBulbRecord {
                    radius,
                    theta,
                    phi,
                    radius_power,
                    seam_side,
                    principal_winding: 0,
                });
            }
            Slot::PseudoKleinian => {
                let mid = self.box_fold(math, point);
                let radius2 = math.dot(&mid, &mid);
                let clamped = if ApMath::branch(&radius2, &self.min_r2) == Ordering::Less {
                    self.min_r2.clone()
                } else {
                    radius2.clone()
                };
                let multiplier = math.div(&one, &clamped);
                let post = math.sub_v(&math.scale_v(&mid, &multiplier), &self.offset);
                record.mid = mid;
                record.mid2.clone_from(&post);
                record.post = post;
                record.m_ref = multiplier;
                let radial = u32::from(ApMath::branch(&radius2, &self.min_r2) != Ordering::Less);
                let fold = std::array::from_fn(|index| {
                    match ApMath::branch(&point[index], &self.fold_limit) {
                        Ordering::Less
                            if ApMath::branch(&point[index], &(-self.fold_limit.clone()))
                                == Ordering::Less =>
                        {
                            0
                        }
                        Ordering::Greater => 2,
                        _ => 1,
                    }
                });
                record.branches = pack_branches(fold, radial);
            }
            Slot::LinCombine => {
                record.post = [
                    math.add(
                        &math.mul(&point[0], &self.lin[0]),
                        &math.mul(&point[1], &self.lin_mix),
                    ),
                    math.add(
                        &math.mul(&point[1], &self.lin[1]),
                        &math.mul(&point[2], &self.lin_mix),
                    ),
                    math.add(
                        &math.mul(&point[2], &self.lin[2]),
                        &math.mul(&point[0], &self.lin_mix),
                    ),
                ];
                record.mid.clone_from(&record.post);
                record.mid2.clone_from(&record.post);
            }
            Slot::Rotate => {
                let mut post = point.clone();
                let (x, y) = Self::rotate_pair(math, &post[0], &post[1], &self.rot[0]);
                post[0] = x;
                post[1] = y;
                let (y, z) = Self::rotate_pair(math, &post[1], &post[2], &self.rot[1]);
                post[1] = y;
                post[2] = z;
                let (x, z) = Self::rotate_pair(math, &post[0], &post[2], &self.rot[2]);
                post[0] = x;
                post[2] = z;
                record.mid.clone_from(&post);
                record.mid2.clone_from(&post);
                record.post = post;
            }
            Slot::Rotate4d => {
                let mut q = [
                    point[0].clone(),
                    point[1].clone(),
                    point[2].clone(),
                    math.zero(),
                ];
                for (index, rotation) in self.rot_w.iter().enumerate() {
                    let (axis, hidden) = Self::rotate_pair(math, &q[index], &q[3], rotation);
                    q[index] = axis;
                    q[3] = hidden;
                }
                let post = [q[0].clone(), q[1].clone(), q[2].clone()];
                record.mid.clone_from(&post);
                record.mid2.clone_from(&post);
                record.post = post;
            }
        }
        record.q_mid = math.dot(&record.mid, &record.mid);
        record.margins = self.transport_margins(math, slot, &record);
        record
    }

    /// The seed an orbit through `point` iterates with.
    ///
    /// Named rather than repeated: the seed is not the point, it is the point
    /// blended toward the Julia constant, and a caller that reconstructs it by
    /// hand will get a Julia stack subtly wrong.
    fn sample_seed(&self, math: &ApMath, point: &ApV3) -> ApV3 {
        math.add_v(
            &math.scale_v(point, &self.sample_weight),
            &math.scale_v(
                &self.julia_seed,
                &math.sub(&math.one(), &self.sample_weight),
            ),
        )
    }

    /// How much of the sample point the seed carries.
    fn weight(&self) -> Hp {
        self.sample_weight.clone()
    }

    fn records(
        &self,
        math: &mut ApMath,
        anchor: &ApV3,
        iterations: usize,
    ) -> (Vec<ApStepRecord>, bool) {
        let seed = math.add_v(
            &math.scale_v(anchor, &self.sample_weight),
            &math.scale_v(
                &self.julia_seed,
                &math.sub(&math.one(), &self.sample_weight),
            ),
        );
        let mut point = anchor.clone();
        let mut records = Vec::with_capacity(iterations);
        for iteration in 0..iterations {
            if self.cycle[iteration % self.cycle.len()] == Slot::Mandelbulb {
                let radius2 = math.dot(&point, &point);
                if ApMath::branch(&radius2, &math.f(4.0)) == Ordering::Greater {
                    return (records, false);
                }
            }
            let record = self.step(math, &point, &seed, iteration);
            point.clone_from(&record.post);
            records.push(record);
            if ApMath::branch(&math.dot(&point, &point), &self.bail2) == Ordering::Greater {
                return (records, false);
            }
        }
        (records, true)
    }

    /// The fold at which this point escapes, or `None` if it is still bounded
    /// after `iterations`.
    ///
    /// `survives` answers a yes/no question and that has been enough to place an
    /// anchor, but not to judge one. A frame is interesting exactly when this
    /// number *varies* across it: constant means a solid, radially monotone
    /// means a smooth shell rendered as concentric contours, and varying in
    /// every direction is the structure the renderer exists to draw.
    #[cfg(test)]
    fn escape_fold(&self, math: &mut ApMath, point: &ApV3, iterations: usize) -> Option<usize> {
        let seed = math.add_v(
            &math.scale_v(point, &self.sample_weight),
            &math.scale_v(
                &self.julia_seed,
                &math.sub(&math.one(), &self.sample_weight),
            ),
        );
        let mut current = point.clone();
        for iteration in 0..iterations {
            if self.cycle[iteration % self.cycle.len()] == Slot::Mandelbulb
                && ApMath::branch(&math.dot(&current, &current), &math.f(4.0)) == Ordering::Greater
            {
                return Some(iteration);
            }
            current = self.step(math, &current, &seed, iteration).post;
            if ApMath::branch(&math.dot(&current, &current), &self.bail2) == Ordering::Greater {
                return Some(iteration + 1);
            }
        }
        None
    }

    fn survives(&self, math: &mut ApMath, anchor: &ApV3, iterations: usize) -> bool {
        let seed = math.add_v(
            &math.scale_v(anchor, &self.sample_weight),
            &math.scale_v(
                &self.julia_seed,
                &math.sub(&math.one(), &self.sample_weight),
            ),
        );
        let mut point = anchor.clone();
        for iteration in 0..iterations {
            if self.cycle[iteration % self.cycle.len()] == Slot::Mandelbulb
                && ApMath::branch(&math.dot(&point, &point), &math.f(4.0)) == Ordering::Greater
            {
                return false;
            }
            point = self.step(math, &point, &seed, iteration).post;
            if ApMath::branch(&math.dot(&point, &point), &self.bail2) == Ordering::Greater {
                return false;
            }
        }
        true
    }
}

// ── Newton refinement ───────────────────────────────────────────────────────

/// Move `c` onto a nearby pre-periodic point by Newton on
/// `F^(n+p)(c) - F^n(c) = 0`.
///
/// Three equations in the three components of `c`, with the Jacobian taken by
/// central differences. Differences rather than an analytic derivative because
/// the stack is a user-selected composition of piecewise maps and the branch
/// structure is constant in a neighbourhood, so a difference is exact enough and
/// cannot fall out of step with the `step` function above.
fn refine(res: &Resolved, c0: V3, pre: usize, period: usize, steps: usize) -> (V3, f64, usize) {
    let residual = |c: V3| -> Option<V3> {
        let (orb, alive) = res.orbit(c, pre + period);
        if !alive || orb.len() < pre + period {
            return None;
        }
        Some(v_sub(orb[pre + period - 1], orb[pre - 1]))
    };

    let mut c = c0;
    let mut best = c0;
    let mut best_norm = f64::INFINITY;
    let h = 1e-11_f64;

    for it in 0..steps {
        let Some(r) = residual(c) else {
            return (best, best_norm, it);
        };
        let norm = r.iter().map(|x| x.to_f64().abs()).fold(0.0, f64::max);
        if norm < best_norm {
            best_norm = norm;
            best = c;
        }
        if norm < 1e-28 {
            break;
        }
        // Jacobian columns.
        let mut jac = [[0.0_f64; 3]; 3];
        for k in 0..3 {
            let mut cp = c;
            let mut cm = c;
            cp[k] = cp[k].add(Dd::new(h));
            cm[k] = cm[k].sub(Dd::new(h));
            let (Some(rp), Some(rm)) = (residual(cp), residual(cm)) else {
                return (best, best_norm, it);
            };
            for row in 0..3 {
                jac[row][k] = (rp[row].to_f64() - rm[row].to_f64()) / (2.0 * h);
            }
        }
        let rhs = [-r[0].to_f64(), -r[1].to_f64(), -r[2].to_f64()];
        let Some(delta) = solve3(jac, rhs) else {
            return (best, best_norm, it);
        };
        for k in 0..3 {
            c[k] = c[k].add(Dd::new(delta[k]));
        }
    }
    (best, best_norm, steps)
}

/// Gaussian elimination with partial pivoting.
fn solve3(m: [[f64; 3]; 3], rhs: [f64; 3]) -> Option<[f64; 3]> {
    let mut a = [
        [m[0][0], m[0][1], m[0][2], rhs[0]],
        [m[1][0], m[1][1], m[1][2], rhs[1]],
        [m[2][0], m[2][1], m[2][2], rhs[2]],
    ];
    for col in 0..3 {
        let piv = (col..3).max_by(|&x, &y| {
            a[x][col]
                .abs()
                .partial_cmp(&a[y][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if a[piv][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, piv);
        for row in 0..3 {
            if row == col {
                continue;
            }
            let f = a[row][col] / a[col][col];
            let pivot = a[col];
            for (value, pivot_value) in a[row][col..].iter_mut().zip(&pivot[col..]) {
                *value -= f * pivot_value;
            }
        }
    }
    Some([a[0][3] / a[0][0], a[1][3] / a[1][1], a[2][3] / a[2][2]])
}

/// Build the twelve-margin payload without narrowing any decision operand.
///
/// Mandelbox/Pseudo-Kleinian use 0-5 for lower/upper fold planes, 6 for
/// `q - minR2`, and 7 for `fixR2 - q` (7 is informational for Pseudo).
/// Amazing Box uses 0-3 for its two fold pairs and 4-5 for the radial pair.
/// Menger uses 0-2 for abs signs, 3-5 for sort planes, and 6 for its trailing
/// translate. Co-cube uses 0-2 for abs, 3-4 for sort, and 5 for its corner abs.
/// Sierpinski uses 0-2 for its reflection planes. Index 11 is always the
/// post-slot bailout margin; unused indices are zero.
fn transport_margins_dd(
    slot: Slot,
    pre: V3,
    mid: V3,
    mid2: V3,
    post: V3,
    resolved: &Resolved,
) -> ([Dd; MARGIN_COUNT], u32) {
    let zero = Dd::default();
    let mut margins = [zero; MARGIN_COUNT];
    let mut code = 0_u32;
    let limit = resolved.fold_limit;
    match slot {
        Slot::Mandelbox | Slot::PseudoKleinian => {
            for component in 0..3 {
                margins[2 * component] = pre[component].add(limit);
                margins[2 * component + 1] = limit.sub(pre[component]);
                let branch = if margins[2 * component].cmp(zero) == Ordering::Less {
                    0
                } else if margins[2 * component + 1].cmp(zero) == Ordering::Less {
                    2
                } else {
                    1
                };
                code += branch * 3_u32.pow(component as u32);
            }
            let q = v_dot(mid, mid);
            margins[6] = q.sub(resolved.min_r2);
            margins[7] = resolved.fix_r2.sub(q);
            let radial = if margins[6].cmp(zero) == Ordering::Less {
                0
            } else if slot == Slot::PseudoKleinian || margins[7].cmp(zero) != Ordering::Less {
                1
            } else {
                2
            };
            code += 27 * radial;
        }
        Slot::AmazingBox => {
            for component in 0..2 {
                margins[2 * component] = pre[component].add(limit);
                margins[2 * component + 1] = limit.sub(pre[component]);
                let branch = if margins[2 * component].cmp(zero) == Ordering::Less {
                    0
                } else if margins[2 * component + 1].cmp(zero) == Ordering::Less {
                    2
                } else {
                    1
                };
                code += branch * 3_u32.pow(component as u32);
            }
            let q = v_dot(mid, mid);
            margins[4] = q.sub(resolved.min_r2);
            margins[5] = resolved.fix_r2.sub(q);
            let radial = if margins[4].cmp(zero) == Ordering::Less {
                0
            } else if margins[5].cmp(zero) != Ordering::Less {
                1
            } else {
                2
            };
            code += 27 * radial;
        }
        Slot::Menger | Slot::CoCube => {
            let mut value = pre.map(Dd::abs);
            for component in 0..3 {
                margins[component] = pre[component];
                if pre[component].cmp(zero) == Ordering::Less {
                    code |= 1 << component;
                }
            }
            let pairs: &[(usize, usize)] = if slot == Slot::Menger {
                &[(0, 1), (0, 2), (1, 2)]
            } else {
                &[(0, 1), (1, 2)]
            };
            for (index, &(left, right)) in pairs.iter().enumerate() {
                margins[3 + index] = value[left].sub(value[right]);
                if margins[3 + index].cmp(zero) == Ordering::Less {
                    value.swap(left, right);
                    code |= 1 << (3 + index);
                }
            }
            if slot == Slot::Menger {
                let shift = resolved.offset[2].mul(resolved.scale.sub(Dd::new(1.0)));
                margins[6] = mid2[2].add(shift.mul_f(0.5));
                if margins[6].cmp(zero) == Ordering::Less {
                    code |= 1 << 6;
                }
            } else {
                margins[5] = value[2].sub(resolved.cocube);
                if margins[5].cmp(zero) == Ordering::Less {
                    code |= 1 << 5;
                }
            }
        }
        Slot::Sierpinski => {
            let mut value = pre;
            for (index, (left, right)) in [(0, 1), (0, 2), (2, 1)].into_iter().enumerate() {
                margins[index] = value[left].add(value[right]);
                if margins[index].cmp(zero) == Ordering::Less {
                    let old = value[left];
                    value[left] = value[right].neg();
                    value[right] = old.neg();
                    code |= 1 << index;
                }
            }
        }
        Slot::Mandelbulb => unreachable!("Mandelbulb margins use arbitrary precision"),
        Slot::Off | Slot::LinCombine | Slot::Rotate | Slot::Rotate4d => {}
    }
    margins[BAILOUT_MARGIN_INDEX] = resolved.bail2.sub(v_dot(post, post));
    (margins, code)
}

const ANCHOR_COARSE_SAMPLES: usize = 48;
const ANCHOR_SURVIVAL_CEILING: usize = 1_024;
const ANCHOR_SURVIVAL_HEADROOM: usize = 24;
/// `MAX_STACK_ITERS` in `fractal_explorer.fs`. The shader cannot march past it,
/// so transporting more records than this buys nothing.
const SHADER_FOLD_CEILING: usize = 256;

/// Folds the anchor search bisects for, the records transported, and the shell
/// the renderer draws - one number, because they are the same number. See
/// `anchor_search_work`.
///
/// The scaling law is one fold per magnification by the stack's own expansion
/// rate, which is the shader's `stack_cap + g_zoomLog2 / foldOctaves()` written
/// on this side of the wire. Getting the *units* wrong here is what produced
/// every empty deep frame: `bucketed_depth_folds` counts the bits of precision
/// the reference orbit needs, which is log2(10) per decade, while a fold buys
/// log2(power) of magnification. Using bits as folds overstated the target
/// threefold at power eight, put the anchor on the boundary of a level set the
/// renderer could not reach, and left every sample in frame still bounded at
/// the fold the march stopped at - a solid wall.
///
/// Too large fails as surely as too small, by the opposite road. The level set
/// K_S has surface features at scale 1/dr_S, and dr compounds by the expansion
/// rate every fold; past the fold where that drops below a pixel footprint the
/// shell is finer than the frame can hold, the distance estimate collapses
/// toward zero everywhere, every ray hits at the camera and the frame renders
/// as flat grey. Measured at 3.2 decades with a target of 96: distance estimate
/// 2^-22 frame radii at the camera, mean Laplacian 0.00.
fn shell_fold_target(params: &StackParams) -> usize {
    // `foldOctaves()` on the shader side: the largest magnification any active
    // slot applies in one fold, floored at two so a stack with no expanding
    // slot still advances.
    let mut expansion: f64 = 2.0;
    for slot in 0..4 {
        if params.rates[slot].clamp(0, 8) <= 0 {
            continue;
        }
        let rate = if params.formulas[slot] == 5 {
            params.power
        } else {
            params.scale.abs()
        };
        expansion = expansion.max(rate);
    }
    let expansion = expansion.max(2.0).log2().max(1.0);
    let magnification_folds = params.zoom_exp.max(0.0) * std::f64::consts::LOG2_10 / expansion;
    // TWO THINGS TRIED HERE AND REJECTED, both recorded because they look
    // obviously right and are not.
    //
    // A second pass - search at the derived target, measure the shell around
    // the anchor it found, re-search at the measurement - is degenerate. The
    // measurement is taken in the neighbourhood of an anchor the same target
    // chose, so every target is its own fixed point; the ladder came back empty
    // at every depth. The scale has to come from outside that loop.
    //
    // A floor ramping to 75 folds, fitted to a sweep in which raising
    // `stack_cap` to 70 rendered all five depths, does not transfer: the same
    // fold target reached by raising the floor instead of the cap renders
    // nothing at six decades. The two are not the same experiment, because
    // `stack_cap` is also the shader's authored budget and its LOD reference.
    // The ladder is not monotone in the target either - six decades fails at 53
    // folds and works at 33 and 77 - which is the self-similar repeat showing
    // through, and a linear rule against a self-similar object is in phase only
    // once per repeat. See spec/fractal-fold-coherence.md.
    ((params.stack_cap.max(1.0) + magnification_folds).ceil() as usize)
        .clamp(1, SHADER_FOLD_CEILING)
}

/// Depth-driven folds are rounded up to multiples of this before they size the
/// transported orbit or the search precision.
///
/// Ungated, a continuous zoom gesture changed the required record count every
/// few frames, and each change re-ran the arbitrary-precision anchor search —
/// seconds of six-thread work per frame of a slider drag, which is what ground
/// the app down during a zoom. Bucketed, a gesture crosses a regeneration
/// boundary once every `24 / log2(10) ≈ 7.2` decades, and the orbit generated
/// at a bucket's top is valid everywhere inside it. An unmarched record only
/// costs transport, so erring seven decades deep is nearly free.
const DEPTH_FOLD_BUCKET: f64 = 24.0;

fn bucketed_depth_folds(zoom_exp: f64) -> f64 {
    let folds = (zoom_exp.max(0.0) * std::f64::consts::LOG2_10).ceil();
    (folds / DEPTH_FOLD_BUCKET).ceil() * DEPTH_FOLD_BUCKET
}

/// The depth the orbit is actually generated for: the top of the current
/// bucket, so every frame inside the bucket is covered by the anchor's
/// precision as well as by the record count.
fn orbit_zoom_exp(zoom_exp: f64) -> f64 {
    bucketed_depth_folds(zoom_exp) / std::f64::consts::LOG2_10
}

#[derive(Debug)]
struct LongLivedAnchor {
    target: [String; 3],
    transported_iterations: usize,
    survival: usize,
}

/// Entries in the transported shell-fold table, one per equal slice of the
/// rated depth range.
///
/// The renderer draws an iso-escape-fold shell, and where to put that shell is
/// not derivable from the depth. The natural rate argument gives about 1.1
/// folds per decade for a generic point at `10^-d` from the set, and the shader
/// used exactly that; but the anchor is not generic, it is *selected* for
/// survival, so its whole neighbourhood stays bounded far longer than the
/// argument predicts. Measured, frame samples escape between folds 64 and 256
/// at nine decades while the formula asked for seventeen, which put the shell
/// deep inside the solid and rendered the dive as a wall the camera was already
/// within. The host can simply measure where the frame decides, so it does.
const SHELL_TABLE_LEN: usize = 12;

/// Samples per side of the square grid laid across the frame at each depth.
const SHELL_GRID: i32 = 1;

/// Share of the frame the shell should leave bounded, so that a frame has an
/// object in it rather than being all surface or all solid.
const SHELL_INTERIOR_TARGET: f64 = 0.45;

/// Decades below one used as the finite-difference step for `D_k`.
///
/// It has to stay far below `1 / |D_k|` for the whole transported orbit or the
/// quotient stops being a derivative, and `|D_k|` reaches `10^100` and beyond
/// within a hundred folds.
const JACOBIAN_DELTA_DECADES: i32 = 250;

/// Mirrors `DZ_CAMERA_STANDOFF` in the shader: the camera sits this fraction of
/// the frame radius back along the view direction.
const DZ_CAMERA_STANDOFF: f64 = 0.25;

/// Standoffs the host will consider putting the eye at, in frame radii, nearest
/// first.
///
/// A fixed standoff is a coin toss against a fractal. The camera sits a quarter
/// of a frame radius back from an anchor that is on the boundary of the solid
/// the frame draws, and whether that lands inside or outside the solid is not a
/// property anything in the renderer controls - it changes with depth, with the
/// stack, and with which fold the shell ends up on. Inside, every ray hits at
/// the camera and the frame is a flat lit field; the depths that render and the
/// depths that do not have looked like a knife edge for exactly this reason,
/// and the host's own camera-fold table has been reporting `CAMERA INSIDE` at
/// two of twelve slices all along.
///
/// Nearest first because the standoff is also the shot: the closer the eye, the
/// more of the frame the structure fills.
const CAMERA_STANDOFF_LADDER: [f64; 9] =
    [0.25, 0.35, 0.5, 0.7, 1.0, 1.4, 2.0, 2.8, 4.0];

struct GeneratedArbitraryRecords {
    records: Vec<ApStepRecord>,
    alive: bool,
    anchor_survival: usize,
    segment_atlas: Option<SegmentAtlasResult>,
    /// Escape fold of the frame's own samples, per depth slice.
    shell_folds: [f32; SHELL_TABLE_LEN],
    /// Escape fold of the camera position itself, per depth slice.
    camera_folds: [f32; SHELL_TABLE_LEN],
    /// Standoff in frame radii that puts the eye outside this slice's shell.
    camera_standoffs: [f32; SHELL_TABLE_LEN],
    /// Parameter Jacobian per fold: unit matrix and `log2` of its magnitude.
    jacobians: Vec<([f32; 9], f32)>,
    /// The decimal anchor the records iterate, kept so a later depth change
    /// can re-certify the atlas without re-running the anchor search.
    anchor_target: Option<[String; 3]>,
}

/// How many records the shader will actually consume, and how long the anchor
/// must therefore outlive them.
///
/// The count has to be the renderer's fold budget, not the requested depth. The
/// shader clamps `total` to the transported length, so a short payload silently
/// shortens the march instead of reporting anything: at a frame radius of 1e-6
/// the renderer asks for `stack_cap` plus twenty depth-driven folds and was
/// handed ten, which is far too few folds for a surface to develop, so nearly
/// every ray returned the same near-smooth field. That reads as a flat gradient
/// with isolated speckle where a stray ray did converge, and it looks like a
/// broken estimator rather than a truncated one.
///
/// Survival is a fixed margin above that count rather than a multiple of it.
/// The anchor's only obligation is not to escape before the last fold the shader
/// marches, and at power eight a margin of twenty-four folds is already some
/// seventy octaves of separation from the level set the renderer consumes. A
/// multiple instead made the search cost grow with the square of the depth: at
/// zoom 30 and 100 it demanded 228 and 256 surviving iterations at roughly
/// thousand-bit precision, which is what made the depth ladder unable to finish.
fn anchor_search_work(params: &StackParams) -> Result<(usize, usize), BoundaryReason> {
    // Mirrors `g_foldBoost = floor(g_zoomLog2 + 0.5)` and the `stack_cap + boost`
    // budget in both estimators. Erring high is free here, since an unmarched
    // record only costs transport.
    // Escape times near a boundary anchor are set by the anchor's local
    // dynamics, not the decade count.
    //
    // A factor of four used to sit here, justified by ground truth showing
    // frame samples escaping between folds 64 and 256. That reasoning was
    // circular: those escape folds were measured in the neighbourhood of an
    // anchor the same factor had already driven deep into the set. Survival
    // is what buries the anchor, burial is what makes the surrounding frame
    // need hundreds of folds to show a silhouette, and hundreds of folds per
    // march step is not renderable - measured, it cost twenty-four times the
    // per-pixel budget and tripped the GPU watchdog. The anchor only has to
    // outlive the folds the renderer actually marches, which is what this
    // asks for.
    // The record count is set by where the frame's silhouette sits, and that is
    // measured rather than derived. `measure_shell_folds` puts it at 65 to 108
    // folds between 2.7 and 8 decades while this formula asks for 15 to 23, and
    // the shortfall does not degrade gracefully: the shader clamps `total` to
    // the transported length, runs out of records mid-march, and rebases the
    // sample onto the reference in binary32. At depth the offset is far below
    // an f32 ulp of the reference, so the rebase *is* the anchor - and the
    // anchor was selected to survive. Every sample then survives, the whole
    // frame reports interior, and the dive renders as a solid wall. That is
    // exactly what nine decades did: 100% bounded-at-budget, luma flat, zero
    // detail, with 44 records against a shell at 86.
    //
    // The measurement needs an anchor, so it cannot inform the search that
    // finds one. The ceiling is the honest resolution: transport every record
    // the shader can consume and let the shell placement decide how many are
    // marched. An unmarched record costs transport and nothing else.
    // Three numbers have to agree and never have: the fold at which the shader
    // draws its shell, the number of records transported to it, and the level
    // set whose boundary the anchor search bisects onto. Every empty deep frame
    // this renderer has produced is one of those three disagreeing.
    //
    // The anchor is bisected to the boundary of K_S, where S is the survival
    // target below. Its neighbourhood therefore escapes at folds near S. March
    // fewer than S and every sample in frame is still bounded, so the whole
    // frame reports interior and renders as a solid wall; transport fewer than
    // the march wants and the shader runs out of records mid-fold, rebases the
    // sample onto the reference in binary32 - which at depth *is* the anchor,
    // an anchor selected to survive - and reaches the same solid wall by the
    // other road. Both are what nine decades did.
    //
    // The fold count is not derivable from the depth. `bucketed_depth_folds` is
    // a count of *bits*, and using it as a fold count is a units error that put
    // the anchor at level 44 while the shader marched 24. What sets it is where
    // this frame's silhouette actually sits, which `measure_shell_folds` puts at
    // 65 to 108 folds between 2.7 and 8 decades - near constant with depth,
    // because it is a property of the anchor's local dynamics rather than of the
    // magnification. That measurement needs an anchor and so cannot inform the
    // search that finds one; this constant is its fixed point, and the shell
    // measurement transported alongside is how a frame reports disagreement.
    let budget = shell_fold_target(params);

    let transported = budget.min(SHADER_FOLD_CEILING);
    if transported == 0 {
        return Err(BoundaryReason::InsufficientSurvival {
            found: params.max_iters,
            required: 1,
        });
    }
    let survival = transported.saturating_add(ANCHOR_SURVIVAL_HEADROOM);
    let ceiling = params.max_iters.min(ANCHOR_SURVIVAL_CEILING);
    if survival > ceiling {
        return Err(BoundaryReason::InsufficientSurvival {
            found: ceiling,
            required: survival,
        });
    }
    Ok((transported, survival))
}

fn point_on_parked_ray(
    math: &ApMath,
    ray: &super::fractal_certification::boundary::ParkedRay,
    t: &Hp,
) -> ApV3 {
    let distance = math.mul(&math.f(ray.length), t);
    std::array::from_fn(|axis| {
        math.add(
            &math.f(ray.origin[axis]),
            &math.mul(&math.f(ray.direction[axis]), &distance),
        )
    })
}

/// Interior points evaluated per refinement round.
///
/// Bisection is sequential by construction and dominates the deep rungs: at zoom
/// 100 the bracket needs some four hundred halvings and each one pays a full
/// survival test. Splitting the bracket `w` ways and testing the interior points
/// together narrows it by a factor of `w + 1` per round, so the number of rounds
/// falls by `log2(w + 1)` while a round still costs one batch of wall time.
fn search_workers() -> usize {
    // More lanes reduce the number of refinement rounds but duplicate a full
    // arbitrary-precision context and orbit for every interior point. Above six
    // lanes the work per acquired bit rises sharply and simultaneous analyzer
    // or test jobs contend for all cores.
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(6)
}

/// Whether each offered ray parameter survives, evaluated concurrently.
///
/// Each worker builds its own context and resolved stack: the arbitrary-precision
/// context carries mutable rounding state and caches, so it cannot be shared, and
/// the resolved constants are precision-bound and have to match their own context.
/// That construction is paid per point, which is worth it only because a survival
/// test is hundreds of transcendental evaluations at several hundred bits.
fn survives_batch(
    params: &StackParams,
    ray: &super::fractal_certification::boundary::ParkedRay,
    precision: usize,
    survival: usize,
    offered: &[Hp],
) -> anyhow::Result<Vec<bool>> {
    let evaluate = |t: &Hp| -> anyhow::Result<bool> {
        let mut math = ApMath::with_precision(precision);
        let resolved = ApResolved::new(params, &mut math)?;
        let point = point_on_parked_ray(&math, ray, t);
        Ok(resolved.survives(&mut math, &point, survival))
    };

    // One point is the common case in the last rounds, where the bracket is
    // already narrow enough that a batch would spawn threads to idle.
    if offered.len() == 1 {
        return Ok(vec![evaluate(&offered[0])?]);
    }

    std::thread::scope(|scope| {
        let handles: Vec<_> = offered
            .iter()
            .map(|t| scope.spawn(move || evaluate(t)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("anchor search worker panicked"))?
            })
            .collect()
    })
}

/// The sub-interval of `[outside, inside]` that still brackets the transition.
///
/// `verdicts` are ordered from the outside end inward. The first surviving point
/// becomes the new inner end and its predecessor the new outer end; if none
/// survive, the transition lies beyond the last point tested. This is the same
/// single-transition assumption bisection already makes, applied to more than one
/// interior point at a time.
fn narrow_bracket(interior: &[Hp], verdicts: &[bool], outside: Hp, inside: Hp) -> (Hp, Hp) {
    match verdicts.iter().position(|&survived| survived) {
        Some(0) => (outside, interior[0].clone()),
        Some(index) => (interior[index - 1].clone(), interior[index].clone()),
        None => (interior.last().cloned().unwrap_or(outside), inside),
    }
}

fn search_long_lived_anchor(params: &StackParams) -> anyhow::Result<LongLivedAnchor> {
    let (transported_iterations, target_survival) = anchor_search_work(params)?;
    let ray = parked_ray_geometry(ParkedCamera {
        distance: default_cam_dist(),
        azimuth: PARKED_AZIMUTH,
        elevation: PARKED_ELEVATION,
        look: PARKED_LOOK,
    })?;
    let full_precision =
        ApMath::for_anchor_search(orbit_zoom_exp(params.zoom_exp), target_survival, params.power)
            .precision;
    let workers = search_workers();
    let scout_precision = full_precision.min(256);
    let mut math = ApMath::with_precision(scout_precision);

    // The coarse scan walks outward in worker-sized batches. Testing a batch at
    // once cannot change which sample is chosen, because the first surviving
    // sample in ray order is still the one taken.
    let mut outside = math.zero();
    let mut inside = None;
    let mut sample = 0;
    'scout: while sample <= ANCHOR_COARSE_SAMPLES {
        let batch: Vec<Hp> = (sample..=(sample + workers - 1).min(ANCHOR_COARSE_SAMPLES))
            .map(|index| math.div(&math.f(index as f64), &math.f(ANCHOR_COARSE_SAMPLES as f64)))
            .collect();
        let verdicts = survives_batch(params, &ray, scout_precision, target_survival, &batch)?;
        for (offset, survived) in verdicts.iter().enumerate() {
            if *survived {
                inside = Some(batch[offset].clone());
                break 'scout;
            }
            outside.clone_from(&batch[offset]);
        }
        sample += batch.len();
    }
    let Some(mut inside) = inside else {
        return Err(anyhow::Error::new(BoundaryReason::LongLivedSearchExhausted));
    };

    // Bits of bracket the refinement has to buy, and how many a round delivers.
    let wanted_bits = bucketed_depth_folds(params.zoom_exp) as usize + 64;
    let bits_per_round = ((workers + 1) as f64).log2();
    let mut gained_bits = 0.0_f64;
    while gained_bits < wanted_bits as f64 {
        let stage_precision = if (wanted_bits as f64 - gained_bits) <= 64.0 {
            full_precision
        } else {
            (gained_bits as usize + 128).clamp(256, full_precision)
        };
        math = ApMath::with_precision(stage_precision);
        outside = outside.with_precision(stage_precision).value();
        inside = inside.with_precision(stage_precision).value();

        let span = math.sub(&inside, &outside);
        let divisor = math.f((workers + 1) as f64);
        let interior: Vec<Hp> = (1..=workers)
            .map(|step| {
                let fraction = math.div(&math.f(step as f64), &divisor);
                math.add(&outside, &math.mul(&span, &fraction))
            })
            .collect();
        let verdicts = survives_batch(params, &ray, stage_precision, target_survival, &interior)?;
        (outside, inside) = narrow_bracket(&interior, &verdicts, outside, inside);
        gained_bits += bits_per_round;
    }

    math = ApMath::with_precision(full_precision);
    inside = inside.with_precision(full_precision).value();
    let resolved = ApResolved::new(params, &mut math)?;
    let anchor = point_on_parked_ray(&math, &ray, &inside);
    if !resolved.survives(&mut math, &anchor, target_survival) {
        return Err(anyhow::Error::new(BoundaryReason::InsufficientSurvival {
            found: transported_iterations,
            required: target_survival,
        }));
    }
    Ok(LongLivedAnchor {
        target: anchor.map(|component| math.to_decimal(&component)),
        transported_iterations,
        survival: target_survival,
    })
}


// ── Segment bilinear table ──────────────────────────────────────────────────

/// Levels in the table. Level `l` holds segments of `2^(l+1)` folds starting at
/// every multiple of that length, so the marcher can take the longest jump its
/// current offset allows and fall back to shorter ones as the offset grows.
///
/// A single stride does not work, and the reason is arithmetic. A segment
/// amplifies the offset by roughly two octaves per fold, so an eight-fold
/// segment multiplies it by about sixteen octaves, and its radius has to be
/// small enough that even the offset at the *end* is still inside the branch
/// structure. Measured with one stride of eight: a sample entering at 2^-20
/// took one jump and ran fifty-six folds by hand. The chain has to shorten as
/// the offset grows, which is what levels are for.
const BLA_LEVELS: usize = 6;

/// How far the linear prediction may drift from the true offset before a
/// segment is declared invalid at that radius.
const BLA_TOLERANCE: f64 = 0.02;

/// One segment of the bilinear table: `e_{j+L} = A e_j + B w`, valid while
/// `|e_j|` stays under `2^radius_log2`.
///
/// The matrices are transported the way the parameter Jacobian already is - a
/// unit matrix and the `log2` of its magnitude - because a segment deep in the
/// orbit has entries far outside binary32 while its *shape* is order one.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SegmentBla {
    pub a_unit: [f32; 9],
    pub a_log2: f32,
    pub b_unit: [f32; 9],
    pub b_log2: f32,
    pub radius_log2: f32,
    pub length: f32,
}

/// A 3x3 matrix carried as a unit matrix and a base-two exponent.
///
/// The composition never leaves `f64` even though the matrices it produces run
/// to thousands of octaves, because each factor `J_k` is a *local* derivative -
/// one fold, bounded by the formula's own scale - and only the accumulated
/// magnitude is large. Renormalising after every fold keeps the mantissas order
/// one and puts the growth where it belongs, in the exponent.
#[derive(Clone, Copy, Debug)]
struct ScaledMatrix {
    unit: [[f64; 3]; 3],
    log2: f64,
}

impl ScaledMatrix {
    fn identity() -> Self {
        Self {
            unit: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            log2: 0.0,
        }
    }

    fn zero() -> Self {
        Self {
            unit: [[0.0; 3]; 3],
            log2: f64::NEG_INFINITY,
        }
    }

    fn peak(unit: &[[f64; 3]; 3]) -> f64 {
        unit.iter()
            .flat_map(|row| row.iter())
            .fold(0.0_f64, |peak, value| peak.max(value.abs()))
    }

    fn renormalized(unit: [[f64; 3]; 3], log2: f64) -> Self {
        let peak = Self::peak(&unit);
        if peak <= 0.0 || !peak.is_finite() {
            return Self::zero();
        }
        Self {
            unit: std::array::from_fn(|row| std::array::from_fn(|col| unit[row][col] / peak)),
            log2: log2 + peak.log2(),
        }
    }

    /// `J * self`, with the magnitude staying in the exponent.
    fn premultiplied(&self, j: &[[f64; 3]; 3]) -> Self {
        if !self.log2.is_finite() {
            return Self::zero();
        }
        let unit = std::array::from_fn(|row| {
            std::array::from_fn(|col| (0..3).map(|k| j[row][k] * self.unit[k][col]).sum::<f64>())
        });
        Self::renormalized(unit, self.log2)
    }

    /// `self + other`, both carried scaled.
    fn added(&self, other: &Self) -> Self {
        if !self.log2.is_finite() {
            return *other;
        }
        if !other.log2.is_finite() {
            return *self;
        }
        let top = self.log2.max(other.log2);
        // A term more than a thousand octaves below the other contributes
        // nothing an f64 mantissa could hold, and asking for its scale factor
        // would underflow to zero anyway.
        let mine = if self.log2 - top < -1000.0 {
            0.0
        } else {
            (self.log2 - top).exp2()
        };
        let theirs = if other.log2 - top < -1000.0 {
            0.0
        } else {
            (other.log2 - top).exp2()
        };
        let unit = std::array::from_fn(|row| {
            std::array::from_fn(|col| {
                self.unit[row][col] * mine + other.unit[row][col] * theirs
            })
        });
        Self::renormalized(unit, top)
    }

    fn plain(m: [[f64; 3]; 3]) -> Self {
        Self::renormalized(m, 0.0)
    }

    fn packed(&self) -> ([f32; 9], f32) {
        if !self.log2.is_finite() {
            return ([0.0; 9], 0.0);
        }
        let mut unit = [0.0_f32; 9];
        for row in 0..3 {
            for col in 0..3 {
                #[allow(clippy::cast_possible_truncation)]
                {
                    unit[row * 3 + col] = self.unit[row][col] as f32;
                }
            }
        }
        #[allow(clippy::cast_possible_truncation)]
        (unit, self.log2 as f32)
    }
}


/// A vector carried the same way, so a prediction can be compared against an
/// offset thousands of octaves from one.
#[derive(Clone, Copy, Debug)]
struct ScaledVec {
    unit: [f64; 3],
    log2: f64,
}

impl ScaledVec {
    fn from_plain(v: [f64; 3]) -> Self {
        let peak = v.iter().fold(0.0_f64, |p, value| p.max(value.abs()));
        if peak <= 0.0 || !peak.is_finite() {
            return Self {
                unit: [0.0; 3],
                log2: f64::NEG_INFINITY,
            };
        }
        Self {
            unit: std::array::from_fn(|k| v[k] / peak),
            log2: peak.log2(),
        }
    }

    fn magnitude_log2(&self) -> f64 {
        if !self.log2.is_finite() {
            return f64::NEG_INFINITY;
        }
        let norm = (self.unit.iter().map(|v| v * v).sum::<f64>()).sqrt();
        if norm <= 0.0 {
            return f64::NEG_INFINITY;
        }
        self.log2 + norm.log2()
    }

    fn transformed(&self, m: &ScaledMatrix) -> Self {
        if !self.log2.is_finite() || !m.log2.is_finite() {
            return Self {
                unit: [0.0; 3],
                log2: f64::NEG_INFINITY,
            };
        }
        let raw: [f64; 3] = std::array::from_fn(|row| {
            (0..3).map(|k| m.unit[row][k] * self.unit[k]).sum::<f64>()
        });
        let scaled = Self::from_plain(raw);
        if !scaled.log2.is_finite() {
            return scaled;
        }
        Self {
            unit: scaled.unit,
            log2: scaled.log2 + self.log2 + m.log2,
        }
    }

    fn added(&self, other: &Self) -> Self {
        if !self.log2.is_finite() {
            return *other;
        }
        if !other.log2.is_finite() {
            return *self;
        }
        let top = self.log2.max(other.log2);
        let mine = if self.log2 - top < -1000.0 { 0.0 } else { (self.log2 - top).exp2() };
        let theirs = if other.log2 - top < -1000.0 { 0.0 } else { (other.log2 - top).exp2() };
        let raw: [f64; 3] =
            std::array::from_fn(|k| self.unit[k] * mine + other.unit[k] * theirs);
        let scaled = Self::from_plain(raw);
        if !scaled.log2.is_finite() {
            return scaled;
        }
        Self {
            unit: scaled.unit,
            log2: scaled.log2 + top,
        }
    }

    fn negated(&self) -> Self {
        Self {
            unit: std::array::from_fn(|k| -self.unit[k]),
            log2: self.log2,
        }
    }

    /// `|self - other| / |other|`, which is the only comparison this table needs
    /// and the only one that survives the dynamic range involved.
    fn relative_error(&self, other: &Self) -> f64 {
        let reference = other.magnitude_log2();
        if !reference.is_finite() {
            return f64::INFINITY;
        }
        let difference = self.added(&other.negated()).magnitude_log2();
        if !difference.is_finite() {
            return 0.0;
        }
        (difference - reference).exp2()
    }
}


/// Build the bilinear table for one reference orbit.
///
/// Two measurements, both slot-agnostic. `J_k` and `S_k` are *local*
/// derivatives - three central differences of a single fold each, at the stored
/// reference point - so the whole table costs `6N` fold evaluations rather than
/// `6N` orbits, and no per-formula derivative is written down for any of the ten
/// slots. The validity radius is measured rather than derived: probe orbits at a
/// ladder of offset magnitudes are run against the true dynamics, and a segment
/// keeps the largest magnitude at which its prediction still tracks. That
/// catches the two failure modes together - the dropped quadratic term, and the
/// sample taking a different fold branch than the reference, which is not an
/// approximation error at all but a different function.
fn measure_segment_bla(
    params: &StackParams,
    anchor: &[String; 3],
    folds: usize,
) -> anyhow::Result<Vec<Vec<SegmentBla>>> {
    if folds == 0 {
        return Ok(Vec::new());
    }
    let mut math = ApMath::for_zoom(orbit_zoom_exp(params.zoom_exp));
    let resolved = ApResolved::new(params, &mut math)?;
    let mut centre: ApV3 = std::array::from_fn(|_| math.zero());
    for (index, part) in anchor.iter().enumerate() {
        centre[index] = math.decimal(part)?;
    }
    let seed = resolved.sample_seed(&math, &centre);

    // The reference orbit, kept as points so a single fold can be re-applied at
    // any of them.
    let mut reference = Vec::with_capacity(folds + 1);
    reference.push(centre.clone());
    let mut point = centre.clone();
    for iteration in 0..folds {
        point = resolved.step(&mut math, &point, &seed, iteration).post;
        reference.push(point.clone());
    }

    // Central differences of one fold. The step is far below the reference and
    // far above the working precision.
    let delta = math.f(1e-24);
    let two_delta = math.add(&delta, &delta);
    let mut state_jac = Vec::with_capacity(folds);
    let mut seed_jac = Vec::with_capacity(folds);
    for iteration in 0..folds {
        let base = reference[iteration].clone();
        let mut j = [[0.0_f64; 3]; 3];
        let mut sj = [[0.0_f64; 3]; 3];
        for axis in 0..3 {
            let mut offset: ApV3 = std::array::from_fn(|_| math.zero());
            offset[axis] = delta.clone();
            let plus = math.add_v(&base, &offset);
            let minus = math.sub_v(&base, &offset);
            let hi = resolved.step(&mut math, &plus, &seed, iteration).post;
            let lo = resolved.step(&mut math, &minus, &seed, iteration).post;
            for row in 0..3 {
                let diff = math.sub(&hi[row], &lo[row]);
                j[row][axis] = ApMath::to_f64(&math.div(&diff, &two_delta));
            }
            // Through the seed, and scaled by how much of the sample point the
            // seed carries, so this is the map from `w` rather than from the
            // seed itself.
            let seed_offset = math.scale_v(&offset, &resolved.weight());
            let seed_plus = math.add_v(&seed, &seed_offset);
            let seed_minus = math.sub_v(&seed, &seed_offset);
            let hi = resolved.step(&mut math, &base, &seed_plus, iteration).post;
            let lo = resolved.step(&mut math, &base, &seed_minus, iteration).post;
            for row in 0..3 {
                let diff = math.sub(&hi[row], &lo[row]);
                sj[row][axis] = ApMath::to_f64(&math.div(&diff, &two_delta));
            }
        }
        state_jac.push(j);
        seed_jac.push(sj);
    }

    // Compose every level. Level zero is two folds; each level after it is the
    // merge of two neighbours, which is the composition rule read as an
    // algorithm: A = A_y A_x and B = A_y B_x + B_y.
    let mut composed: Vec<Vec<(ScaledMatrix, ScaledMatrix, usize, usize)>> =
        Vec::with_capacity(BLA_LEVELS);
    for level in 0..BLA_LEVELS {
        let span = 1_usize << (level + 1);
        let count = folds.div_ceil(span);
        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let start = index * span;
            let end = (start + span).min(folds);
            let (mut a, mut b) = (ScaledMatrix::identity(), ScaledMatrix::zero());
            for fold in start..end {
                a = a.premultiplied(&state_jac[fold]);
                b = b
                    .premultiplied(&state_jac[fold])
                    .added(&ScaledMatrix::plain(seed_jac[fold]));
            }
            entries.push((a, b, start, end));
        }
        composed.push(entries);
    }

    // Probe the true dynamics for the radius at which each entry still holds.
    //
    // One orbit answers for every entry at every level, because a probe's offset
    // at fold `j` is exactly the `e_j` a marching sample would arrive with. The
    // ladder walks downward, so the first magnitude that survives every
    // direction is the largest one that does.
    let directions: [[f64; 3]; 4] = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.577, -0.577, 0.577],
    ];
    let ladder: Vec<f64> = (0..64).map(|step| -2.0 - 3.0 * f64::from(step)).collect();
    let mut radius_log2: Vec<Vec<f64>> = composed
        .iter()
        .map(|entries| vec![f64::NEG_INFINITY; entries.len()])
        .collect();
    let mut settled: Vec<Vec<bool>> = composed
        .iter()
        .map(|entries| vec![false; entries.len()])
        .collect();
    for &magnitude_log2 in &ladder {
        if settled.iter().flatten().all(|done| *done) {
            break;
        }
        let magnitude = magnitude_log2.exp2();
        let mut holds: Vec<Vec<bool>> = composed
            .iter()
            .map(|entries| vec![true; entries.len()])
            .collect();
        // The radius has to be quoted in the variable the marcher actually
        // holds, which is the offset at the segment's *start*, not the probe's
        // initial offset. Recording the probe's `|w|` instead makes every radius
        // look like it shrinks along the orbit at exactly the rate the offset
        // grows - the same quantity written in two different units - and a
        // marcher reading it that way finds the table valid at the beginning and
        // never again.
        let mut start_magnitude: Vec<Vec<f64>> = composed
            .iter()
            .map(|entries| vec![f64::NEG_INFINITY; entries.len()])
            .collect();
        for direction in &directions {
            let offset: [f64; 3] = std::array::from_fn(|k| direction[k] * magnitude);
            let sample: ApV3 =
                std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));
            let sample_seed = resolved.sample_seed(&math, &sample);
            let mut offsets = Vec::with_capacity(folds + 1);
            offsets.push(ScaledVec::from_plain(offset));
            let mut walker = sample.clone();
            for iteration in 0..folds {
                walker = resolved
                    .step(&mut math, &walker, &sample_seed, iteration)
                    .post;
                let difference: [f64; 3] = std::array::from_fn(|axis| {
                    ApMath::to_f64(&math.sub(&walker[axis], &reference[iteration + 1][axis]))
                });
                offsets.push(ScaledVec::from_plain(difference));
            }
            let w = ScaledVec::from_plain(offset);
            for (level, entries) in composed.iter().enumerate() {
                for (index, (a, b, start, end)) in entries.iter().enumerate() {
                    if settled[level][index] || !holds[level][index] {
                        continue;
                    }
                    let predicted = offsets[*start].transformed(a).added(&w.transformed(b));
                    if predicted.relative_error(&offsets[*end]) < BLA_TOLERANCE {
                        start_magnitude[level][index] =
                            start_magnitude[level][index].max(offsets[*start].magnitude_log2());
                    } else {
                        holds[level][index] = false;
                    }
                }
            }
        }
        for (level, entries) in composed.iter().enumerate() {
            for index in 0..entries.len() {
                if !settled[level][index]
                    && holds[level][index]
                    && start_magnitude[level][index].is_finite()
                {
                    radius_log2[level][index] = start_magnitude[level][index];
                    settled[level][index] = true;
                }
            }
        }
    }

    let mut table = Vec::with_capacity(BLA_LEVELS);
    for (level, entries) in composed.into_iter().enumerate() {
        let mut packed = Vec::with_capacity(entries.len());
        for (index, (a, b, start, end)) in entries.into_iter().enumerate() {
            let (a_unit, a_log2) = a.packed();
            let (b_unit, b_log2) = b.packed();
            #[allow(clippy::cast_possible_truncation)]
            packed.push(SegmentBla {
                a_unit,
                a_log2,
                b_unit,
                b_log2,
                radius_log2: if radius_log2[level][index].is_finite() {
                    radius_log2[level][index] as f32
                } else {
                    // Never valid: the marcher must fold this stretch by hand.
                    -1.0e30
                },
                length: (end - start) as f32,
            });
        }
        table.push(packed);
    }
    Ok(table)
}


// ── Analyzer ────────────────────────────────────────────────────────────────

pub(crate) struct FractalReferenceOrbitAnalyzer {
    base_params: StackParams,
    params: StackParams,
    resolved: Option<Resolved>,
    arbitrary_records: Option<Vec<ApStepRecord>>,
    segment_atlas: Option<SegmentAtlasResult>,
    anchor: V3,
    survived: usize,
    residual: f64,
    boundary_ready: bool,
    boundary_reason: Option<BoundaryReason>,
    boundary_runtime_ms: f32,
    payload_generation: u64,
    cached_texture: Option<TextureData>,
    /// Anchor behind `arbitrary_records`, for atlas-only re-certification.
    anchor_target: Option<[String; 3]>,
    /// Measured escape fold of the frame per depth slice, transported so the
    /// shader places its shell where the frame actually decides.
    shell_folds: [f32; SHELL_TABLE_LEN],
    /// Measured escape fold of the camera itself, per depth slice.
    camera_folds: [f32; SHELL_TABLE_LEN],
    /// Standoff, in frame radii, at which the eye is outside the shell this
    /// depth slice draws. See `CAMERA_STANDOFF_LADDER`.
    camera_standoffs: [f32; SHELL_TABLE_LEN],
    /// Parameter Jacobian per fold, transported so the shader can reach any
    /// fold in one matrix-vector product instead of running the loop.
    jacobians: Vec<([f32; 9], f32)>,
    /// Orbit key a provisional payload was published for, and whether the
    /// full-depth payload still owes a refinement.
    ///
    /// A full-depth anchor search costs seconds, and it runs on a state
    /// change, which is exactly when a shader has just been loaded and has
    /// nothing to draw. Publishing a shallow payload first costs a fraction
    /// of that and gives the renderer a valid orbit immediately: the frames
    /// are shallower than requested, not wrong, because the anchor is a
    /// genuine long-lived one and every perturbation identity still holds.
    /// The deep payload replaces it a moment later.
    provisional_key: Option<StackParams>,
    needs_full_orbit: bool,
    /// Recently searched anchors by orbit key. Re-testing a stored anchor's
    /// survival is one orbit evaluation where a fresh bisection is dozens, so
    /// returning to a look already visited costs almost nothing.
    anchor_cache: Vec<(StackParams, [String; 3])>,
    /// Exact depth the current atlas was certified at. A certificate is a
    /// statement about one frame radius; consuming it at any other depth is
    /// unsound, so a depth change drops the atlas until this matches again.
    atlas_zoom: Option<f64>,
}

impl FractalReferenceOrbitAnalyzer {
    pub(crate) fn new() -> Self {
        Self {
            base_params: StackParams::default(),
            params: StackParams::default(),
            resolved: None,
            arbitrary_records: None,
            segment_atlas: None,
            anchor: [Dd::default(); 3],
            survived: 0,
            residual: f64::INFINITY,
            boundary_ready: false,
            boundary_reason: None,
            boundary_runtime_ms: 0.0,
            payload_generation: 0,
            cached_texture: None,
            anchor_target: None,
            shell_folds: [0.0; SHELL_TABLE_LEN],
            camera_folds: [0.0; SHELL_TABLE_LEN],
            camera_standoffs: [0.0; SHELL_TABLE_LEN],
            jacobians: Vec::new(),
            atlas_zoom: None,
            provisional_key: None,
            needs_full_orbit: false,
            anchor_cache: Vec::new(),
        }
    }

    /// Longest orbit key history worth keeping. Anchors are a few hundred
    /// bytes of decimal digits, and a session revisits a handful of looks.
    const ANCHOR_CACHE_LIMIT: usize = 8;

    fn cached_anchor(&self, key: &StackParams) -> Option<[String; 3]> {
        self.anchor_cache
            .iter()
            .find(|(cached, _)| cached == key)
            .map(|(_, anchor)| anchor.clone())
    }

    fn remember_anchor(&mut self, key: StackParams, anchor: [String; 3]) {
        self.anchor_cache.retain(|(cached, _)| *cached != key);
        self.anchor_cache.push((key, anchor));
        if self.anchor_cache.len() > Self::ANCHOR_CACHE_LIMIT {
            self.anchor_cache.remove(0);
        }
    }

    /// The shallow stand-in published while the full-depth search runs.
    ///
    /// Depth zero makes `anchor_search_work` ask only for the authored fold
    /// cutoff, which is the cheap end of the search, and clears the
    /// certificate work. Everything the payload gate hashes is left alone, so
    /// the provisional payload is accepted by the same shader state that will
    /// accept the deep one.
    fn provisional_params(params: &StackParams) -> StackParams {
        let mut provisional = params.clone();
        provisional.zoom_exp = 0.0;
        provisional.certificate_enabled = false;
        provisional
    }

    fn uses_mandelbulb(params: &StackParams) -> bool {
        params
            .formulas
            .iter()
            .zip(params.rates)
            .any(|(formula, rate)| *formula == 5 && rate > 0)
    }

    /// Whether this stack needs the arbitrary-precision path: a searched anchor,
    /// a transported record orbit, and the shell measurement.
    ///
    /// This used to be `uses_mandelbulb`, which had the support backwards. The
    /// paper's exact-increment result covers the nine non-Mandelbulb operations
    /// and it is the *bulb* that needs matrix certification, yet the bulb was
    /// the only stack the deep-zoom machinery would run for. Everything else
    /// fell through to a double-double orbit taken from a fixed anchor, which
    /// is a shallow renderer: at six decades the Mandelbox published fourteen
    /// records from an unsearched point, every sample crossed the footprint
    /// gate within a fold or two, and the frame came back 100% LOD-truncated -
    /// empty sky, at every depth, for every fold-stack formula.
    ///
    /// It matters for the look rather than for coverage. The Mandelbox is
    /// self-similar by construction, so a dive keeps finding structure, while
    /// the Mandelbulb's boundary is a smooth surface almost everywhere and a
    /// deep frame on it is a smooth shell drawn as concentric contours.
    /// Measured with `parked_ray_structure_profile` at six decades: along the
    /// parked ray the bulb reads zero alternations at every sample, and the box
    /// reads four to nine out of sixteen over most of the ray.
    fn needs_arbitrary_precision(params: &StackParams) -> bool {
        Self::uses_mandelbulb(params) || params.zoom_exp > 0.0
    }

    /// Camera geometry used by frame-local perturbed rendering.
    fn primary_camera(params: &StackParams) -> PrimaryCamera {
        let (sin_azimuth, cos_azimuth) = PARKED_AZIMUTH.sin_cos();
        let (sin_elevation, cos_elevation) = PARKED_ELEVATION.sin_cos();
        let orbit = [
            cos_azimuth * cos_elevation,
            sin_elevation,
            sin_azimuth * cos_elevation,
        ];
        let eye = orbit.map(|component| component * default_cam_dist());
        let aim = PARKED_LOOK.map(|component| component * 0.5);
        let forward = {
            let delta = std::array::from_fn(|axis| aim[axis] - eye[axis]);
            let length = delta
                .iter()
                .map(|component| component * component)
                .sum::<f64>()
                .sqrt();
            delta.map(|component| component / length)
        };
        let cross = |left: [f64; 3], right: [f64; 3]| {
            [
                left[1] * right[2] - left[2] * right[1],
                left[2] * right[0] - left[0] * right[2],
                left[0] * right[1] - left[1] * right[0],
            ]
        };
        let normalize = |vector: [f64; 3]| {
            let length = vector
                .iter()
                .map(|component| component * component)
                .sum::<f64>()
                .sqrt();
            vector.map(|component| component / length)
        };
        let candidate_right = cross([0.0, 1.0, 0.0], forward);
        let right = if candidate_right
            .iter()
            .map(|component| component * component)
            .sum::<f64>()
            .sqrt()
            > 1.0e-4
        {
            normalize(candidate_right)
        } else {
            [1.0, 0.0, 0.0]
        };
        let up = cross(forward, right);
        PrimaryCamera {
            forward,
            right,
            up,
            fov: PARKED_FOV,
            aspect: params.render_aspect,
            sway_extent: [0.0; 2],
        }
    }

    fn generate_arbitrary_records(
        params: &StackParams,
    ) -> anyhow::Result<GeneratedArbitraryRecords> {
        Self::generate_arbitrary_records_hinted(params, None)
    }

    /// Whether a stored anchor still meets this configuration's survival
    /// requirement, which is what makes reusing it sound.
    ///
    /// One orbit evaluation against dozens for a fresh bisection. A stale hint
    /// simply fails the test and the search runs as before, so the cache can
    /// never move the anchor somewhere the search would not have put it.
    fn hinted_anchor_survives(params: &StackParams, hint: &[String; 3]) -> Option<LongLivedAnchor> {
        let (transported_iterations, survival) = anchor_search_work(params).ok()?;
        let precision =
            ApMath::for_anchor_search(orbit_zoom_exp(params.zoom_exp), survival, params.power)
                .precision;
        let mut math = ApMath::with_precision(precision);
        let resolved = ApResolved::new(params, &mut math).ok()?;
        let mut point = std::array::from_fn(|_| math.zero());
        for (index, part) in hint.iter().enumerate() {
            point[index] = math.decimal(part).ok()?;
        }
        resolved
            .survives(&mut math, &point, survival)
            .then(|| LongLivedAnchor {
                target: hint.clone(),
                transported_iterations,
                survival,
            })
    }

    /// The parameter Jacobian `D_k = d x_k / d c` along the reference orbit.
    ///
    /// This is the object that lets the shader skip the fold loop. The paper's
    /// stratified perturbation theorem gives `D_0 = I`, `D_{k+1} = J_k D_k +
    /// S_k` and `e_k = D_k w + O(|w|^2)`, so a sample's offset at fold `k` is
    /// one matrix-vector product rather than `k` folds, and the escape fold
    /// becomes a monotone predicate that can be bisected.
    ///
    /// Computed by finite differences rather than by an analytic Jacobian per
    /// slot. Three extra orbits is nothing beside the anchor search, and the
    /// difference quotient is slot-agnostic: it covers the whole ten-formula
    /// stack, folds and branches included, with no new per-slot derivation to
    /// get wrong.
    ///
    /// The difference is kept *unscaled* on purpose. Orbits are bounded until
    /// they escape, so `x_k(c + delta) - x_k(c)` stays order one however large
    /// `D_k` becomes, and the magnitude is carried separately as
    /// `log2|D_k| = log2|difference| + log2(1/delta)`. That is what keeps a
    /// derivative of `10^100` inside binary32 on the way to the GPU. `delta`
    /// has to stay far below `1/|D_k|` for the quotient to be a derivative at
    /// all, which is why it is `1e-250` and the working precision is set from
    /// it rather than from the zoom depth.
    fn measure_parameter_jacobians(
        params: &StackParams,
        anchor: &[String; 3],
        iterations: usize,
    ) -> anyhow::Result<Vec<([f32; 9], f32)>> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let precision =
            (f64::from(JACOBIAN_DELTA_DECADES) * std::f64::consts::LOG2_10) as usize + 192;
        let mut math = ApMath::with_precision(precision);
        let resolved = ApResolved::new(params, &mut math)?;
        let mut centre: ApV3 = std::array::from_fn(|_| math.zero());
        for (index, part) in anchor.iter().enumerate() {
            centre[index] = math.decimal(part)?;
        }
        let delta = math.decimal(&format!("1e-{JACOBIAN_DELTA_DECADES}"))?;

        let (base, _) = resolved.records(&mut math, &centre, iterations);
        let mut probes = Vec::with_capacity(3);
        for axis in 0..3 {
            let mut probe = centre.clone();
            probe[axis] = math.add(&probe[axis], &delta);
            probes.push(resolved.records(&mut math, &probe, iterations).0);
        }

        let length = probes
            .iter()
            .map(Vec::len)
            .min()
            .unwrap_or(0)
            .min(base.len());
        let log2_inverse_delta = f64::from(JACOBIAN_DELTA_DECADES) * std::f64::consts::LOG2_10;

        let mut out = Vec::with_capacity(length);
        for fold in 0..length {
            let mut entries = [[0.0_f64; 3]; 3];
            for (column, probe) in probes.iter().enumerate() {
                for row in 0..3 {
                    let difference =
                        math.sub(&probe[fold].post[row], &base[fold].post[row]);
                    entries[row][column] = ApMath::to_f64(&difference);
                }
            }
            let peak = entries
                .iter()
                .flatten()
                .fold(0.0_f64, |peak, value| peak.max(value.abs()));
            if peak <= 0.0 || !peak.is_finite() {
                out.push(([0.0; 9], 0.0));
                continue;
            }
            let mut unit = [0.0_f32; 9];
            for row in 0..3 {
                for column in 0..3 {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        unit[row * 3 + column] = (entries[row][column] / peak) as f32;
                    }
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            let scale = (peak.log2() + log2_inverse_delta) as f32;
            out.push((unit, scale));
        }
        Ok(out)
    }

    /// Where the frame's own samples stop being undecided, per depth slice.
    ///
    /// At each of `SHELL_TABLE_LEN` depths spanning the rated range, a small
    /// grid is laid across the frame in the camera's own right and up
    /// directions at that depth's radius, and each sample's orbit is run until
    /// it escapes. The entry is the low order statistic of those escape folds,
    /// so a shell placed there has most of the frame escaping in front of it
    /// and reading as surface, rather than bounded behind it and reading as
    /// solid. A sample that never escapes contributes the iteration ceiling,
    /// which pushes the shell deeper exactly where the frame really is solid.
    fn measure_shell_folds(
        params: &StackParams,
        anchor: &[String; 3],
    ) -> anyhow::Result<(
        [f32; SHELL_TABLE_LEN],
        [f32; SHELL_TABLE_LEN],
        [f32; SHELL_TABLE_LEN],
    )> {
        let camera = Self::primary_camera(params);
        let rated = params.zoom_exp.max(1.0);
        let mut math = ApMath::for_zoom(orbit_zoom_exp(params.zoom_exp));
        let resolved = ApResolved::new(params, &mut math)?;
        let mut centre: ApV3 = std::array::from_fn(|_| math.zero());
        for (index, part) in anchor.iter().enumerate() {
            centre[index] = math.decimal(part)?;
        }

        // Folds past what the shader can march tell the shell nothing, and the
        // measurement is on the path to the first frame, so it stops there.
        let budget = params.max_iters.min(SHADER_FOLD_CEILING);
        // Folds past the shader's ceiling cannot be marched, so measuring them
        // buys nothing and costs the whole payload's latency: the sweep runs
        // `SHELL_TABLE_LEN` depths times a grid of orbits, and every iteration
        // past the ceiling is time the renderer spends showing nothing.
        let mut table = [0.0_f32; SHELL_TABLE_LEN];
        // The camera's own escape fold at the same depth. The shell has to sit
        // beyond it or the eye is inside the solid the frame draws, which is
        // the wall the dive kept hitting: measured, frames render exactly when
        // the camera decides before the shell does.
        let mut camera_table = [0.0_f32; SHELL_TABLE_LEN];
        // Where the eye has to stand to be outside what the frame draws.
        let mut standoff_table = [0.0_f32; SHELL_TABLE_LEN];
        let mut folds = Vec::with_capacity(25);
        for slot in 0..SHELL_TABLE_LEN {
            #[allow(clippy::cast_precision_loss)]
            let depth = rated * (slot as f64 + 1.0) / SHELL_TABLE_LEN as f64;
            let radius = params.cam_dist * 10.0_f64.powf(-depth);
            folds.clear();
            let mut sampled = 0_usize;
            for gx in -SHELL_GRID..=SHELL_GRID {
                for gy in -SHELL_GRID..=SHELL_GRID {
                    let offset: [f64; 3] = std::array::from_fn(|axis| {
                        radius
                            * (f64::from(gx) * camera.right[axis]
                                + f64::from(gy) * camera.up[axis])
                    });
                    let point: ApV3 = std::array::from_fn(|axis| {
                        math.add(&centre[axis], &math.f(offset[axis]))
                    });
                    let (orbit, alive) = resolved.records(&mut math, &point, budget);
                    // Every sample counts, deciding or not. A sample that never
                    // escapes is the object's own interior and enters the
                    // distribution at the ceiling, which is what lets the
                    // quantile below express how much of this frame is solid.
                    sampled += 1;
                    folds.push(if alive { budget } else { orbit.len() });
                }
            }
            log::debug!(
                "shell slot {slot}: {} of {sampled} frame samples escape",
                folds.len()
            );
            if folds.is_empty() {
                table[slot] = 0.0;
                continue;
            }
            // The camera sits a fixed fraction of the frame back along the
            // view direction, so its distance from the anchor shrinks with the
            // frame and its escape fold has to be measured per depth too.
            let camera_point: ApV3 = std::array::from_fn(|axis| {
                math.add(
                    &centre[axis],
                    &math.f(-DZ_CAMERA_STANDOFF * radius * camera.forward[axis]),
                )
            });
            let (camera_orbit, camera_alive) =
                resolved.records(&mut math, &camera_point, budget);
            #[allow(clippy::cast_precision_loss)]
            {
                camera_table[slot] = if camera_alive {
                    budget as f32
                } else {
                    camera_orbit.len() as f32
                };
            }

            folds.sort_unstable();
            // Place the shell where a target share of the frame is still
            // bounded, which is the silhouette criterion stated directly.
            //
            // Every other statistic tried here answered a different question.
            // The escape rate alone says where the surface resolves but not how
            // much solid is left behind it, and a shell at that fold left
            // almost nothing interior, so rays missed into sky. What a frame
            // needs is an object to look at: some share of it bounded and the
            // rest open. Reading the fold off the distribution at
            // `1 - SHELL_INTERIOR_TARGET` gives that share at every depth, and
            // it self-corrects against the structure's own self-similar repeat
            // instead of assuming a rate.
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let index = ((folds.len() as f64) * (1.0 - SHELL_INTERIOR_TARGET)) as usize;
            let index = index.min(folds.len() - 1);
            #[allow(clippy::cast_precision_loss)]
            {
                table[slot] = folds[index] as f32;
            }
            // The eye goes where it is outside the shell this slice draws.
            //
            // "Outside" is a statement about the same surface the march stops
            // on, so it is the same test: the camera point has to escape
            // *before* the shell fold. Walking outward and taking the first
            // standoff that passes keeps the structure as large in frame as
            // being outside it allows.
            let shell_fold = folds[index];
            let mut chosen = *CAMERA_STANDOFF_LADDER
                .last()
                .expect("the standoff ladder is not empty");
            for candidate in CAMERA_STANDOFF_LADDER {
                let point: ApV3 = std::array::from_fn(|axis| {
                    math.add(
                        &centre[axis],
                        &math.f(-candidate * radius * camera.forward[axis]),
                    )
                });
                let (orbit, alive) = resolved.records(&mut math, &point, budget);
                let escape = if alive { budget } else { orbit.len() };
                if escape < shell_fold {
                    chosen = candidate;
                    break;
                }
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                standoff_table[slot] = chosen as f32;
            }
            log::debug!(
                "shell slot {slot}: {} of {sampled} samples escape, shell at fold {}, \
                 camera standoff {chosen}",
                folds.iter().filter(|fold| **fold < budget).count(),
                folds[index]
            );
        }
        Ok((table, camera_table, standoff_table))
    }

    fn generate_arbitrary_records_hinted(
        params: &StackParams,
        hint: Option<&[String; 3]>,
    ) -> anyhow::Result<GeneratedArbitraryRecords> {
        let effective = if params.refine {
            let anchor = match hint.and_then(|hint| Self::hinted_anchor_survives(params, hint)) {
                Some(reused) => {
                    log::debug!(
                        "reused a cached Mandelbulb anchor for {} transported records",
                        reused.transported_iterations
                    );
                    reused
                }
                None => search_long_lived_anchor(params)?,
            };
            log::debug!(
                "found camera-ray Mandelbulb anchor surviving {} iterations for {} transported records",
                anchor.survival,
                anchor.transported_iterations
            );
            let mut effective = params.clone();
            effective.anchor = Some(anchor.target);
            effective.refine = false;
            effective.max_iters = anchor.transported_iterations;
            effective
        } else {
            params.clone()
        };
        let mut math = ApMath::for_zoom(orbit_zoom_exp(effective.zoom_exp));
        let resolved = ApResolved::new(&effective, &mut math)?;
        let mut anchor = std::array::from_fn(|_| math.zero());
        if let Some(parts) = effective.anchor.as_ref() {
            for (index, part) in parts.iter().enumerate() {
                anchor[index] = math.decimal(part)?;
            }
        }
        let (records, alive) = resolved.records(&mut math, &anchor, effective.max_iters);
        let segment_atlas = if params.certificate_enabled {
            effective.anchor.as_ref().map(|anchor| {
                certify_primary_ray_segments(
                    &effective,
                    anchor,
                    Self::primary_camera(&effective),
                    records.len(),
                    24.0,
                    SegmentBudgets::default(),
                )
            })
        } else {
            None
        };
        let anchor_survival = if params.refine {
            anchor_search_work(params)?.1
        } else {
            records.len()
        };
        // Measured against the live parameters, not the effective ones: the
        // shell is a statement about the frames the renderer will draw, and
        // `effective` carries the search's own iteration budget.
        let (shell_folds, camera_folds, camera_standoffs) = match effective.anchor.as_ref() {
            Some(anchor) => Self::measure_shell_folds(params, anchor).unwrap_or_else(|error| {
                log::warn!("shell-fold measurement failed, falling back to the derived rate: {error}");
                ([0.0; SHELL_TABLE_LEN], [0.0; SHELL_TABLE_LEN], [0.0; SHELL_TABLE_LEN])
            }),
            None => (
                [0.0; SHELL_TABLE_LEN],
                [0.0; SHELL_TABLE_LEN],
                [0.0; SHELL_TABLE_LEN],
            ),
        };

        let jacobians = match effective.anchor.as_ref() {
            Some(anchor) => {
                Self::measure_parameter_jacobians(&effective, anchor, records.len())
                    .unwrap_or_else(|error| {
                        log::warn!("parameter Jacobian measurement failed: {error}");
                        Vec::new()
                    })
            }
            None => Vec::new(),
        };

        Ok(GeneratedArbitraryRecords {
            records,
            alive,
            anchor_survival,
            segment_atlas,
            anchor_target: effective.anchor,
            shell_folds,
            camera_folds,
            camera_standoffs,
            jacobians,
        })
    }

    /// The live state the reference orbit actually depends on.
    ///
    /// Depth is bucketed out: the orbit is the iterated image of one anchor
    /// and does not depend on the frame radius, only the transported record
    /// count and the search precision do, and both are quantized by
    /// `orbit_zoom_exp`. The certificate toggle is excluded because it selects
    /// atlas work, not orbit work. Everything else regenerates the orbit as
    /// before.
    fn orbit_key(params: &StackParams) -> StackParams {
        let mut key = params.clone();
        key.zoom_exp = orbit_zoom_exp(params.zoom_exp);
        key.certificate_enabled = false;
        key
    }

    /// Re-certify the primary-ray atlas for the current exact depth, reusing
    /// the stored anchor and records.
    fn rebuild_atlas(&mut self) {
        let Some(anchor) = self.anchor_target.clone() else {
            return;
        };
        let count = self.arbitrary_records.as_ref().map_or(0, Vec::len);
        if count == 0 {
            return;
        }
        let mut effective = self.params.clone();
        effective.anchor = Some(anchor.clone());
        effective.refine = false;
        effective.max_iters = count;
        self.segment_atlas = Some(certify_primary_ray_segments(
            &effective,
            &anchor,
            Self::primary_camera(&effective),
            count,
            24.0,
            SegmentBudgets::default(),
        ));
        self.atlas_zoom = Some(self.params.zoom_exp);
        self.refresh_texture_cache();
    }

    fn state_signature(params: &StackParams) -> u32 {
        let mut hash = 2_166_136_261_u32;
        let mut mix = |word: u32| {
            // The transported signature is restricted to 24 exactly
            // representable integer bits. Fold the high byte down before FNV,
            // otherwise powers of two such as 0.125 differ only above bit 23
            // and disappear when the final hash is masked.
            hash ^= word ^ (word >> 24);
            hash = hash.wrapping_mul(16_777_619);
        };
        for value in params.formulas {
            mix(value as u32);
        }
        for value in params.rates {
            mix(value as u32);
        }
        for value in [
            params.scale,
            params.fold_limit,
            params.min_radius,
            params.fixed_radius,
            params.bailout,
            params.power,
            // Depth is deliberately not hashed (this slot mirrors the
            // shader's seventh scalar, once `dzOrbitBudgetExp()`). The orbit
            // is the iterated image of one anchor and does not depend on the
            // frame radius; hashing depth invalidated a sound payload on
            // every frame of a zoom gesture, faster than any regeneration
            // could answer, so the whole gesture rendered the alarm pattern.
            0.0,
            default_cam_dist(),
            PARKED_AZIMUTH,
            PARKED_ELEVATION,
            PARKED_FOV,
            0.0,
            params.render_aspect,
            params.julia_amount,
            params.cocube,
            params.lin_mix,
            // Decides how many records get transported, so a payload built for
            // one cutoff must not be accepted under another.
            params.stack_cap,
        ]
        .into_iter()
        .chain(params.offset)
        .chain(params.julia)
        .chain(params.lin)
        .chain(params.rot)
        .chain(params.rot_w)
        .chain(PARKED_LOOK)
        {
            mix((value as f32).to_bits());
        }
        mix(params.max_iters as u32);
        // Exactly representable as an f32 numeric value for shader comparison.
        hash & 0x00ff_ffff
    }

    fn snapshot(&self) -> AnalyzerSnapshot {
        let (segment_ready, segment_coverage) = match self.segment_atlas.as_ref() {
            Some(SegmentAtlasResult::Certified { tiles, .. }) => {
                let covered = tiles.iter().filter(|tile| tile.safe_slab_mask != 0).count();
                (1.0, covered as f32 / (ATLAS_COLUMNS * ATLAS_ROWS) as f32)
            }
            _ => (0.0, 0.0),
        };
        let mut scalars = HashMap::from([
            ("boundary_ready".to_string(), f32::from(self.boundary_ready)),
            (
                "boundary_reason".to_string(),
                self.boundary_reason
                    .map_or(0.0, |reason| f32::from(reason.code())),
            ),
            ("boundary_runtime_ms".to_string(), self.boundary_runtime_ms),
            ("segment_ready".to_string(), segment_ready),
            ("segment_coverage".to_string(), segment_coverage),
            // 1 while the published payload is the shallow stand-in and the
            // full-depth orbit is still owed. Readiness alone cannot say this:
            // a provisional payload is a real, ready payload, it is simply the
            // wrong depth, and a capture taken against it is indistinguishable
            // from a capture of the requested depth that happens to look wrong.
            (
                "payload_provisional".to_string(),
                f32::from(self.needs_full_orbit),
            ),
        ]);
        if !Self::uses_mandelbulb(&self.params) {
            scalars.insert("boundary_ready".to_string(), 1.0);
        }
        let mut textures = HashMap::new();
        if let Some(texture) = self.cached_texture.clone() {
            textures.insert("refOrbit".to_string(), texture);
        }
        AnalyzerSnapshot {
            scalars,
            textures,
            timestamp: std::time::Instant::now(),
        }
    }

    fn refresh_texture_cache(&mut self) {
        self.payload_generation = next_texture_generation();
        self.cached_texture = self.orbit_texture();
    }

    fn target_digest(anchor_high: [f64; 3], anchor_low: [f64; 3], count: usize) -> u32 {
        let mut hash = 2_166_136_261_u32;
        for value in anchor_high.into_iter().chain(anchor_low) {
            let word = (value as f32).to_bits();
            hash = (hash ^ word ^ (word >> 24)).wrapping_mul(16_777_619);
        }
        let word = count as u32;
        hash = (hash ^ word ^ (word >> 24)).wrapping_mul(16_777_619);
        hash & 0x00ff_ffff
    }

    /// The orbit as a texture: one `rgba32float` texel per four-float group,
    /// twelve groups per iteration.
    ///
    /// **Reduced to `f32`.** The anchor needed thirty-two digits; these values
    /// do not. Each is order one, so an error of eps here perturbs a computed
    /// increment by order eps relative, which is already `f32` noise. What would
    /// be fatal is computing the orbit itself in `f32`, which is what this
    /// exists to avoid.
    ///
    /// **Sent as raw floats rather than byte-packed.** An earlier version
    /// packed each `f32` into one `rgba8unorm` texel because the renderer
    /// declared every preprocessor texture filterable and `Rgba32Float` is
    /// not. That cost four texel fetches plus byte reassembly per group,
    /// inside a recurrence the march runs tens of thousands of times per
    /// pixel — a measured multiple of the whole fold arithmetic. The layout
    /// is format-aware now (`FORMAT: "rgba32float"` in the shader's
    /// `PREPROCESSORS` block binds the slot non-filterable), so a group is
    /// one fetch, exact by construction.
    fn orbit_texture(&self) -> Option<TextureData> {
        let iters = self.params.max_iters.min(MAX_ITERS_LIMIT);
        let mut floats: Vec<f32> = Vec::with_capacity((iters + HEADER_RECORDS) * FLOATS_PER_ITER);
        let push = |v: [f64; 4], out: &mut Vec<f32>| {
            #[allow(clippy::cast_possible_truncation)]
            for x in v {
                out.push(x as f32);
            }
        };
        let push_f32 = |v: [f32; 4], out: &mut Vec<f32>| out.extend_from_slice(&v);

        if let Some(records) = self.arbitrary_records.as_ref() {
            let anchor = records.first().map_or([0.0; 3], |record| {
                std::array::from_fn(|index| ApMath::to_f64(&record.pre[index]))
            });
            let anchor_hi = anchor.map(|value| f64::from(value as f32));
            let anchor_lo = records.first().map_or([0.0; 3], |record| {
                std::array::from_fn(|index| {
                    let precision = record.pre[index].precision();
                    let high = Hp::try_from(anchor_hi[index])
                        .expect("finite anchor converts to arbitrary precision")
                        .with_precision(precision)
                        .value();
                    ApMath::to_f64(&(&record.pre[index] - &high))
                })
            });
            push(
                [
                    f64::from(PAYLOAD_VERSION),
                    GROUPS_PER_ITER as f64,
                    records.len() as f64,
                    f64::from(Self::state_signature(&self.params)),
                ],
                &mut floats,
            );
            let target_digest = Self::target_digest(anchor_hi, anchor_lo, records.len());
            push(
                [
                    anchor_hi[0],
                    anchor_hi[1],
                    anchor_hi[2],
                    records
                        .first()
                        .map_or(0, |record| record.pre[0].precision()) as f64,
                ],
                &mut floats,
            );
            push(
                [
                    anchor_lo[0],
                    anchor_lo[1],
                    anchor_lo[2],
                    f64::from(target_digest),
                ],
                &mut floats,
            );
            let atlas = match self.segment_atlas.as_ref() {
                Some(SegmentAtlasResult::Certified { metadata, tiles })
                    if metadata.columns == ATLAS_COLUMNS
                        && metadata.rows == ATLAS_ROWS
                        && tiles.len() == ATLAS_COLUMNS * ATLAS_ROWS =>
                {
                    Some((metadata, tiles))
                }
                _ => None,
            };
            if let Some((metadata, tiles)) = atlas {
                for group in 0..8 {
                    let base = group * 4;
                    push_f32(
                        [
                            (tiles[base].safe_slab_mask & 0x00ff_ffff) as f32,
                            (tiles[base + 1].safe_slab_mask & 0x00ff_ffff) as f32,
                            (tiles[base + 2].safe_slab_mask & 0x00ff_ffff) as f32,
                            (tiles[base + 3].safe_slab_mask & 0x00ff_ffff) as f32,
                        ],
                        &mut floats,
                    );
                }
                push(
                    [
                        metadata.columns as f64,
                        metadata.rows as f64,
                        f64::from(metadata.max_frame_t),
                        records.len() as f64,
                    ],
                    &mut floats,
                );
            } else {
                for _ in 3..GROUPS_PER_ITER {
                    push([0.0; 4], &mut floats);
                }
            }
            // Header record one: the measured shell-fold table, three groups
            // of four, then the camera-fold table, then reserved zeroes.
            // Record two is where the orbit begins, which `dzRec` accounts for.
            for group in 0..GROUPS_PER_ITER {
                let base = group * 4;
                if base < SHELL_TABLE_LEN {
                    push_f32(
                        [
                            self.shell_folds[base],
                            self.shell_folds[base + 1],
                            self.shell_folds[base + 2],
                            self.shell_folds[base + 3],
                        ],
                        &mut floats,
                    );
                } else if base < 2 * SHELL_TABLE_LEN {
                    let camera = base - SHELL_TABLE_LEN;
                    push_f32(
                        [
                            self.camera_folds[camera],
                            self.camera_folds[camera + 1],
                            self.camera_folds[camera + 2],
                            self.camera_folds[camera + 3],
                        ],
                        &mut floats,
                    );
                } else if base < 3 * SHELL_TABLE_LEN {
                    let standoff = base - 2 * SHELL_TABLE_LEN;
                    push_f32(
                        [
                            self.camera_standoffs[standoff],
                            self.camera_standoffs[standoff + 1],
                            self.camera_standoffs[standoff + 2],
                            self.camera_standoffs[standoff + 3],
                        ],
                        &mut floats,
                    );
                } else {
                    push([0.0; 4], &mut floats);
                }
            }
            for index in 0..iters {
                let record = records.get(index);
                let Some(record) = record else {
                    for _ in 0..GROUPS_PER_ITER {
                        push([0.0; 4], &mut floats);
                    }
                    continue;
                };
                let vec3 = |value: &ApV3| -> [f64; 3] {
                    std::array::from_fn(|component| ApMath::to_f64(&value[component]))
                };
                let pre = vec3(&record.pre);
                let mid = vec3(&record.mid);
                let mid2 = vec3(&record.mid2);
                let post = vec3(&record.post);
                push(
                    [pre[0], pre[1], pre[2], ApMath::to_f64(&record.q_pre)],
                    &mut floats,
                );
                push(
                    [mid[0], mid[1], mid[2], ApMath::to_f64(&record.q_mid)],
                    &mut floats,
                );
                push(
                    [mid2[0], mid2[1], mid2[2], ApMath::to_f64(&record.m_ref)],
                    &mut floats,
                );
                push(
                    [post[0], post[1], post[2], f64::from(record.branches)],
                    &mut floats,
                );
                if let Some(bulb) = record.bulb.as_ref() {
                    push(
                        [
                            ApMath::to_f64(&bulb.radius),
                            ApMath::to_f64(&bulb.theta),
                            ApMath::to_f64(&bulb.phi),
                            ApMath::to_f64(&bulb.radius_power),
                        ],
                        &mut floats,
                    );
                } else {
                    push([0.0; 4], &mut floats);
                }
                for margin_group in 0..3 {
                    let base = margin_group * 4;
                    push_f32(
                        [
                            ApMath::to_f32(&record.margins[base]),
                            ApMath::to_f32(&record.margins[base + 1]),
                            ApMath::to_f32(&record.margins[base + 2]),
                            ApMath::to_f32(&record.margins[base + 3]),
                        ],
                        &mut floats,
                    );
                }
                push(
                    [
                        f64::from(record.branches),
                        record
                            .bulb
                            .as_ref()
                            .map_or(0.0, |bulb| f64::from(bulb.seam_side)),
                        record
                            .bulb
                            .as_ref()
                            .map_or(0.0, |bulb| f64::from(bulb.principal_winding)),
                        f64::from(record.bulb.is_some()),
                    ],
                    &mut floats,
                );
                // Groups nine through eleven: the parameter Jacobian, as a
                // unit matrix plus the base-two logarithm of its magnitude.
                // These were reserved zeroes, so the payload does not grow.
                let (unit, scale) = self
                    .jacobians
                    .get(index)
                    .copied()
                    .unwrap_or(([0.0; 9], 0.0));
                push_f32([unit[0], unit[1], unit[2], scale], &mut floats);
                push_f32([unit[3], unit[4], unit[5], 0.0], &mut floats);
                push_f32([unit[6], unit[7], unit[8], 0.0], &mut floats);
            }
        } else {
            let res = self.resolved.as_ref()?;
            let (records, _) = res.orbit_records(self.anchor, self.params.max_iters);
            push(
                [
                    f64::from(PAYLOAD_VERSION),
                    GROUPS_PER_ITER as f64,
                    records.len() as f64,
                    f64::from(Self::state_signature(&self.params)),
                ],
                &mut floats,
            );
            let anchor_hi = self.anchor.map(|value| f64::from(value.to_f64() as f32));
            let anchor_lo: [f64; 3] = std::array::from_fn(|index| {
                self.anchor[index].sub(Dd::new(anchor_hi[index])).to_f64()
            });
            let target_digest = Self::target_digest(anchor_hi, anchor_lo, records.len());
            push(
                [anchor_hi[0], anchor_hi[1], anchor_hi[2], 106.0],
                &mut floats,
            );
            push(
                [
                    anchor_lo[0],
                    anchor_lo[1],
                    anchor_lo[2],
                    f64::from(target_digest),
                ],
                &mut floats,
            );
            for _ in 3..GROUPS_PER_ITER {
                push([0.0; 4], &mut floats);
            }
            // Header record one: the measured shell-fold table, three groups
            // of four, then reserved zeroes. Record two is where the orbit
            // begins, which `dzRec` accounts for.
            for group in 0..GROUPS_PER_ITER {
                let base = group * 4;
                if base < SHELL_TABLE_LEN {
                    push_f32(
                        [
                            self.shell_folds[base],
                            self.shell_folds[base + 1],
                            self.shell_folds[base + 2],
                            self.shell_folds[base + 3],
                        ],
                        &mut floats,
                    );
                } else if base < 2 * SHELL_TABLE_LEN {
                    let camera = base - SHELL_TABLE_LEN;
                    push_f32(
                        [
                            self.camera_folds[camera],
                            self.camera_folds[camera + 1],
                            self.camera_folds[camera + 2],
                            self.camera_folds[camera + 3],
                        ],
                        &mut floats,
                    );
                } else if base < 3 * SHELL_TABLE_LEN {
                    let standoff = base - 2 * SHELL_TABLE_LEN;
                    push_f32(
                        [
                            self.camera_standoffs[standoff],
                            self.camera_standoffs[standoff + 1],
                            self.camera_standoffs[standoff + 2],
                            self.camera_standoffs[standoff + 3],
                        ],
                        &mut floats,
                    );
                } else {
                    push([0.0; 4], &mut floats);
                }
            }

            let mut previous_post = self.anchor;
            for index in 0..iters {
                let record = records.get(index).copied();
                let (pre, mid, mid2, post, multiplier) = match record {
                    Some(record) => (
                        previous_post,
                        record.mid,
                        record.mid2,
                        record.post,
                        record.m_ref.to_f64(),
                    ),
                    None => (self.anchor, self.anchor, self.anchor, self.anchor, 1.0),
                };
                let pre_f64 = pre.map(Dd::to_f64);
                let mid_f64 = mid.map(Dd::to_f64);
                let post_scale_f64 = mid2.map(Dd::to_f64);
                let post_f64 = post.map(Dd::to_f64);
                let branches = record.map_or(0, |record| record.branches);
                push(
                    [pre_f64[0], pre_f64[1], pre_f64[2], v_dot(pre, pre).to_f64()],
                    &mut floats,
                );
                push(
                    [mid_f64[0], mid_f64[1], mid_f64[2], v_dot(mid, mid).to_f64()],
                    &mut floats,
                );
                push(
                    [
                        post_scale_f64[0],
                        post_scale_f64[1],
                        post_scale_f64[2],
                        multiplier,
                    ],
                    &mut floats,
                );
                push(
                    [post_f64[0], post_f64[1], post_f64[2], f64::from(branches)],
                    &mut floats,
                );
                push([0.0; 4], &mut floats);
                let margins = record.map_or([Dd::default(); MARGIN_COUNT], |record| record.margins);
                for margin_group in 0..3 {
                    let base = margin_group * 4;
                    push_f32(
                        [
                            margins[base].to_f32(),
                            margins[base + 1].to_f32(),
                            margins[base + 2].to_f32(),
                            margins[base + 3].to_f32(),
                        ],
                        &mut floats,
                    );
                }
                push([f64::from(branches), 0.0, 0.0, 0.0], &mut floats);
                for _ in 9..GROUPS_PER_ITER {
                    push([0.0; 4], &mut floats);
                }
                if record.is_some() && index < records.len() {
                    previous_post = records[index].post;
                }
            }
        }

        // One rgba32float texel per group of four floats: the shader's
        // `dzGroup` is a single `texelFetch` with no byte unpacking, which
        // matters because the march reads tens of thousands of groups per
        // pixel.
        let texels = floats.len().div_ceil(4).max(1);
        let width = texels.clamp(1, PAYLOAD_WIDTH);
        let height = texels.div_ceil(width);
        floats.resize(width * height * 4, 0.0);
        let mut data = Vec::with_capacity(floats.len() * 4);
        for v in &floats {
            data.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        Some(TextureData {
            generation: self.payload_generation,
            width: u32::try_from(width).unwrap_or(1),
            height: u32::try_from(height).unwrap_or(1),
            format: "rgba32float".into(),
            data: data.into(),
        })
    }
}

impl Analyzer for FractalReferenceOrbitAnalyzer {
    fn analyzer_type(&self) -> &'static str {
        "fractal_reference_orbit"
    }

    fn output_schema(&self) -> AnalyzerSchema {
        AnalyzerSchema {
            scalars: vec![
                ScalarOutputDef {
                    name: "boundary_ready".into(),
                    description: "1 only when a sufficiently long-lived camera-ray anchor exists"
                        .into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.0,
                },
                ScalarOutputDef {
                    name: "boundary_reason".into(),
                    description: "Stable typed inconclusive reason code; 0 means none".into(),
                    range: (0.0, 255.0),
                    default: 0.0,
                    default_smoothing: 0.0,
                },
                ScalarOutputDef {
                    name: "boundary_runtime_ms".into(),
                    description: "Host anchor query runtime in milliseconds".into(),
                    range: (0.0, f32::MAX),
                    default: 0.0,
                    default_smoothing: 0.0,
                },
                ScalarOutputDef {
                    name: "segment_ready".into(),
                    description: "1 when a matching primary-ray certificate atlas is packed".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.0,
                },
                ScalarOutputDef {
                    name: "payload_provisional".into(),
                    description: "1 while the published orbit is the shallow stand-in and the \
                                  full-depth one is still owed"
                        .into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.0,
                },
                ScalarOutputDef {
                    name: "segment_coverage".into(),
                    description: "Fraction of primary-ray tiles with a positive safe prefix".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.0,
                },
            ],
            textures: vec![TextureOutputDef {
                name: "refOrbit".into(),
                description: "Versioned two-dimensional reference orbit, forty-eight \
                              byte-packed f32 texels per iteration. Records carry all \
                              intermediate points, continuous-power Mandelbulb polar \
                              values, branch decisions, and branch margins. The header \
                              carries version, stride, state signature, split anchor, \
                              precision, valid iteration count, and the optional \
                              eight-by-four primary-ray safe-slab table."
                    .into(),
                format: "rgba32float".into(),
            }],
        }
    }

    /// The orbit depends on the stack parameters and the anchor, never on the
    /// deck's pixels, so the per-frame readback would be pure cost.
    fn needs_frame_input(&self) -> bool {
        false
    }

    fn init(&mut self, options: &serde_json::Value) -> anyhow::Result<()> {
        let params: StackParams = if options.is_null() {
            StackParams::default()
        } else {
            serde_json::from_value(options.clone())?
        };
        if params.max_iters == 0 || params.max_iters > MAX_ITERS_LIMIT {
            anyhow::bail!(
                "max_iters must be between 1 and {MAX_ITERS_LIMIT}, got {}",
                params.max_iters
            );
        }
        if params.newton_pre_period == 0 {
            anyhow::bail!("newton_pre_period must be at least 1");
        }
        if Self::uses_mandelbulb(&params) {
            let GeneratedArbitraryRecords {
                records,
                alive,
                segment_atlas,
                anchor_target,
                shell_folds,
                camera_folds,
                jacobians,
                ..
            } = Self::generate_arbitrary_records(&params)?;
            self.survived = records.len();
            if let Some(first) = records.first() {
                self.anchor =
                    std::array::from_fn(|index| Dd::new(ApMath::to_f64(&first.pre[index])));
            }
            self.residual = f64::NAN;
            if !alive {
                log::debug!(
                    "arbitrary-precision Mandelbulb reference escaped after {} of {} iterations",
                    self.survived,
                    params.max_iters
                );
            }
            self.atlas_zoom = segment_atlas.as_ref().map(|_| params.zoom_exp);
            self.arbitrary_records = Some(records);
            self.segment_atlas = segment_atlas;
            self.anchor_target = anchor_target;
            self.shell_folds = shell_folds;
            self.camera_folds = camera_folds;
            self.jacobians = jacobians;
            self.resolved = None;
            self.params = params;
            self.base_params = self.params.clone();
            self.refresh_texture_cache();
            return Ok(());
        }

        let resolved = Resolved::new(&params)?;

        // Parse the anchor as decimal strings so it can carry more digits than a
        // JSON float literal survives.
        let mut anchor = [Dd::default(); 3];
        if let Some(parts) = params.anchor.as_ref() {
            for (k, s) in parts.iter().enumerate() {
                let v: f64 = s
                    .trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("anchor component {k} is not a number: {s:?}"))?;
                anchor[k] = Dd::new(v);
            }
        }

        // A stationary start defeats Newton. The origin is a fixed point of
        // several of these formulas with a zero seed, so the pre-periodic
        // residual is already zero there and Newton returns immediately with a
        // degenerate anchor whose orbit never moves. Nudge off it first.
        if params.refine && anchor.iter().all(|c| c.to_f64() == 0.0) {
            anchor = [Dd::new(0.31), Dd::new(-0.17), Dd::new(0.23)];
            log::info!(
                "reference orbit: anchor was the origin, which is a fixed point \
                 for these maps; starting Newton from a nudged point instead"
            );
        }

        if params.refine {
            let (refined, residual, _) = refine(
                &resolved,
                anchor,
                params.newton_pre_period,
                params.newton_period,
                48,
            );
            self.anchor = refined;
            self.residual = residual;
        } else {
            self.anchor = anchor;
            self.residual = f64::NAN;
        }

        let (_, alive) = resolved.orbit(self.anchor, params.max_iters);
        let (orbit, _) = resolved.orbit(self.anchor, params.max_iters);
        self.survived = orbit.len();
        if !alive {
            // Not fatal: a short orbit still renders, just shallower. Worth
            // saying loudly, because it is the exact failure the diagnostic
            // found in the shader and the whole reason this analyzer exists.
            log::warn!(
                "reference orbit escaped after {} of {} iterations (Newton \
                 residual {:.3e}); zoom depth will be limited to about 1/dr at \
                 that count",
                self.survived,
                params.max_iters,
                self.residual
            );
        }

        self.params = params;
        self.base_params = self.params.clone();
        self.arbitrary_records = None;
        self.resolved = Some(resolved);
        self.refresh_texture_cache();
        Ok(())
    }

    fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot> {
        let mut next = self.base_params.with_live_state(&input.state);
        if input.width > 0 && input.height > 0 {
            next.render_aspect = f64::from(input.width) / f64::from(input.height);
        }
        // Whether this frame arrived with the state already settled, taken
        // before any mutation below. Atlas certification keys off this: it is
        // expensive, its result is only sound at one exact depth, and running
        // it while a slider is still moving is exactly the per-frame
        // arbitrary-precision grind that used to stall the app.
        let params_stable = next == self.params;
        if !params_stable {
            if Self::needs_arbitrary_precision(&next) {
                let orbit_stale = self.arbitrary_records.is_none()
                    || Self::orbit_key(&next) != Self::orbit_key(&self.params);
                if !orbit_stale {
                    // Only depth or the certificate toggle moved. The orbit is
                    // depth-independent and stays published; the atlas is a
                    // statement about one exact depth, so a stale one is
                    // dropped rather than consumed at the wrong radius. It is
                    // re-certified below once the depth stops moving.
                    self.params = next;
                    if self.segment_atlas.is_some()
                        && self.atlas_zoom != Some(self.params.zoom_exp)
                    {
                        self.segment_atlas = None;
                        self.atlas_zoom = None;
                        self.refresh_texture_cache();
                    }
                    return Ok(self.snapshot());
                }
                // Publish shallow first, deep second. The full search costs
                // seconds and a state change is exactly the moment the
                // renderer has nothing to draw, so the first pass builds the
                // cheap payload and returns; the refinement runs on the next
                // call, by which time a slider still in motion will have
                // superseded it anyway.
                let key = Self::orbit_key(&next);
                let provisional = self.provisional_key.as_ref() != Some(&key);
                let build = if provisional {
                    Self::provisional_params(&next)
                } else {
                    next.clone()
                };
                let hint = self.cached_anchor(&key);
                let started = std::time::Instant::now();
                let generated =
                    Self::generate_arbitrary_records_hinted(&build, hint.as_ref());
                self.boundary_runtime_ms = started.elapsed().as_secs_f32() * 1_000.0;
                let GeneratedArbitraryRecords {
                    records,
                    alive,
                    anchor_survival,
                    segment_atlas,
                    anchor_target,
                    shell_folds,
                    camera_folds,
                    camera_standoffs,
                    jacobians,
                } = match generated {
                    Ok(generated) => {
                        self.boundary_ready = true;
                        self.boundary_reason = None;
                        generated
                    }
                    Err(error) => {
                        self.boundary_ready = false;
                        self.boundary_reason = error.downcast_ref::<BoundaryReason>().copied();
                        self.survived = 0;
                        self.params = next;
                        self.resolved = None;
                        self.arbitrary_records = None;
                        self.segment_atlas = None;
                        self.anchor_target = None;
                        self.atlas_zoom = None;
                        self.cached_texture = None;
                        log::error!(
                            "live long-lived Mandelbulb anchor rejected after {:.1} ms: {}",
                            self.boundary_runtime_ms,
                            error
                        );
                        return Ok(self.snapshot());
                    }
                };
                self.survived = records.len();
                log::debug!(
                    "live Mandelbulb anchor survived at least {} iterations; publishing {} records",
                    anchor_survival,
                    self.survived
                );
                if !alive {
                    log::debug!(
                        "live arbitrary-precision Mandelbulb reference escaped after {} of {} iterations",
                        self.survived,
                        next.max_iters
                    );
                }
                self.atlas_zoom = segment_atlas.as_ref().map(|_| next.zoom_exp);
                // The live state is what the payload must be signed against
                // even on the provisional pass, or the gate would reject the
                // very payload published to fill the gap.
                self.params = next;
                self.resolved = None;
                self.arbitrary_records = Some(records);
                self.segment_atlas = segment_atlas;
                if let Some(anchor) = anchor_target.clone() {
                    if !provisional {
                        self.remember_anchor(key.clone(), anchor);
                    }
                }
                self.anchor_target = anchor_target;
                self.shell_folds = shell_folds;
                self.camera_folds = camera_folds;
                self.camera_standoffs = camera_standoffs;
                self.jacobians = jacobians;
                self.provisional_key = Some(key);
                self.needs_full_orbit = provisional;
                self.refresh_texture_cache();
                return Ok(self.snapshot());
            }
            let resolved = Resolved::new(&next)?;
            let (orbit, alive) = resolved.orbit(self.anchor, next.max_iters);
            self.survived = orbit.len();
            if !alive {
                log::debug!(
                    "live reference orbit escaped after {} of {} iterations",
                    self.survived,
                    next.max_iters
                );
            }
            self.params = next;
            self.arbitrary_records = None;
            self.segment_atlas = None;
            self.anchor_target = None;
            self.atlas_zoom = None;
            self.resolved = Some(resolved);
            self.refresh_texture_cache();
            return Ok(self.snapshot());
        }

        // The state has settled. Anything a provisional payload deferred is
        // owed now: the shallow stand-in kept the renderer drawing, and this
        // is where it becomes the full-depth orbit that was asked for.
        if self.needs_full_orbit && Self::needs_arbitrary_precision(&self.params) {
            self.needs_full_orbit = false;
            let key = Self::orbit_key(&self.params);
            let hint = self.cached_anchor(&key);
            let started = std::time::Instant::now();
            let refined =
                Self::generate_arbitrary_records_hinted(&self.params, hint.as_ref());
            self.boundary_runtime_ms = started.elapsed().as_secs_f32() * 1_000.0;
            match refined {
                Ok(generated) => {
                    self.boundary_ready = true;
                    self.boundary_reason = None;
                    self.survived = generated.records.len();
                    self.atlas_zoom = generated
                        .segment_atlas
                        .as_ref()
                        .map(|_| self.params.zoom_exp);
                    self.arbitrary_records = Some(generated.records);
                    self.segment_atlas = generated.segment_atlas;
                    if let Some(anchor) = generated.anchor_target.clone() {
                        self.remember_anchor(key, anchor);
                    }
                    self.anchor_target = generated.anchor_target;
                    self.shell_folds = generated.shell_folds;
                    self.camera_folds = generated.camera_folds;
                    self.camera_standoffs = generated.camera_standoffs;
                    self.jacobians = generated.jacobians;
                    self.refresh_texture_cache();
                    log::debug!(
                        "refined the provisional payload to {} records in {:.1} ms",
                        self.survived,
                        self.boundary_runtime_ms
                    );
                }
                Err(error) => {
                    // The shallow payload stays published rather than being
                    // cleared: it is a valid orbit, just not the requested
                    // depth, and a renderer drawing shallow beats one that
                    // has been handed nothing.
                    self.boundary_reason = error.downcast_ref::<BoundaryReason>().copied();
                    log::warn!(
                        "full-depth refinement rejected after {:.1} ms, keeping the \
                         provisional payload: {error}",
                        self.boundary_runtime_ms
                    );
                }
            }
            return Ok(self.snapshot());
        }

        // A parked explicit depth wants its certificate atlas, certified for
        // exactly this depth from the stored anchor.
        if self.params.certificate_enabled
            && Self::uses_mandelbulb(&self.params)
            && self.arbitrary_records.is_some()
            && self.atlas_zoom != Some(self.params.zoom_exp)
        {
            let started = std::time::Instant::now();
            self.rebuild_atlas();
            self.boundary_runtime_ms = started.elapsed().as_secs_f32() * 1_000.0;
        }

        Ok(self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_always_enabled_without_mode_state() {
        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [1, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "refine": false,
                "max_iters": 4
            }))
            .unwrap();

        assert!(analyzer.cached_texture.is_some());
    }

    #[test]
    fn removed_camera_controls_cannot_change_the_fixed_certificate_camera() {
        let defaults = StackParams::default();
        let expected = FractalReferenceOrbitAnalyzer::primary_camera(&defaults);
        let mut state = AnalyzerStateSnapshot::default();
        for (name, value) in [
            ("cam_dist", 9.0),
            ("cam_azim", 1.0),
            ("cam_elev", -0.5),
            ("fov", 1.6),
            ("sway_amount", 0.8),
            ("look_x", 0.7),
            ("look_y", -0.4),
            ("look_z", 0.9),
        ] {
            state
                .values
                .insert(name.into(), crate::params::ParamValue::Float(value));
        }

        let live = defaults.with_live_state(&state);
        assert_eq!(
            FractalReferenceOrbitAnalyzer::primary_camera(&live),
            expected
        );
    }

    #[test]
    fn unchanged_state_reuses_the_packed_texture_generation() {
        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [1, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "refine": false,
                "max_iters": 4
            }))
            .unwrap();
        let first = analyzer.cached_texture.as_ref().unwrap().clone();
        let second = analyzer.cached_texture.as_ref().unwrap().clone();

        assert_ne!(first.generation, 0);
        assert_eq!(first.generation, second.generation);
        assert!(std::sync::Arc::ptr_eq(&first.data, &second.data));
    }

    #[test]
    fn failed_live_boundary_clears_unrelated_previous_snapshot() {
        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [1, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "refine": false,
                "max_iters": 4
            }))
            .unwrap();
        assert!(analyzer.orbit_texture().is_some());

        analyzer.base_params = StackParams {
            formulas: [5, 5, 8, 9],
            rates: [1, 0, 0, 0],
            max_iters: 1,
            ..StackParams::default()
        };
        let snapshot = analyzer
            .analyze(&AnalyzerInput {
                frame: Vec::new(),
                width: 0,
                height: 0,
                timestamp: std::time::Instant::now(),
                state: AnalyzerStateSnapshot::default(),
            })
            .unwrap();

        assert!(snapshot.textures.is_empty());
        assert_eq!(snapshot.scalar("boundary_ready"), 0.0);
        assert_eq!(
            snapshot.scalar("boundary_reason"),
            f32::from(
                BoundaryReason::InsufficientSurvival {
                    found: 1,
                    required: 1 + ANCHOR_SURVIVAL_HEADROOM,
                }
                .code()
            )
        );
    }

    /// Ground truth for the deep-frame wall: what the true map says about the
    /// frame the shader renders at depth nine, with no shader in the loop.
    ///
    /// Grids the physical frame around the searched anchor and reports true
    /// escape-fold counts from the arbitrary-precision evaluator. Run with
    /// `--ignored --nocapture` when diagnosing; the numbers say whether a
    /// mostly-interior deep frame is real geometry (escape times beyond the
    /// renderer's fold ceiling) or a shader recurrence defect (samples the
    /// true map escapes quickly that the shader keeps bounded).
    /// Ground truth for the parked camera itself: the marcher starts at the
    /// fixed standoff on the parked ray, and if that point's true escape fold
    /// exceeds every budget we can march, no shell placement can save the
    /// frame - the camera must move onto the host-proven escaping segment.
    /// Does `x_k(c + w) = P_k + D_k w` actually hold, and out to which fold?
    ///
    /// The linearized escape rests entirely on this identity, so it is checked
    /// against the arbitrary-precision orbit rather than assumed. Prints the
    /// relative error of the linear prediction at each fold for an offset the
    /// size of a real frame radius; the fold where the error stops being small
    /// is the validity horizon the shader must not bisect past.
    #[test]
    #[ignore = "diagnostic: validates the parameter Jacobian against the true orbit"]
    fn linear_model_tracks_the_true_orbit() {
        let params = StackParams {
            zoom_exp: 6.0,
            ..StackParams::default()
        };
        let anchor = search_long_lived_anchor(&params).expect("anchor search");
        let folds = 64;
        let jacobians =
            FractalReferenceOrbitAnalyzer::measure_parameter_jacobians(&params, &anchor.target, folds)
                .expect("jacobians");

        let mut math = ApMath::with_precision(512);
        let resolved = ApResolved::new(&params, &mut math).expect("stack");
        let mut centre: ApV3 = std::array::from_fn(|_| math.zero());
        for (index, part) in anchor.target.iter().enumerate() {
            centre[index] = math.decimal(part).expect("anchor digits");
        }
        // An offset the size of the frame at this depth.
        let radius = params.cam_dist * 10.0_f64.powf(-params.zoom_exp);
        let offset = [radius * 0.5, radius * -0.25, radius * 0.125];
        let sample: ApV3 =
            std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));

        let (base, _) = resolved.records(&mut math, &centre, folds);
        let (truth, _) = resolved.records(&mut math, &sample, folds);
        let length = base.len().min(truth.len()).min(jacobians.len());
        println!("offset magnitude {radius:.3e}, {length} folds");
        for fold in (0..length).step_by(4) {
            let (unit, scale) = jacobians[fold];
            let magnitude = f64::from(scale).exp2();
            // Predicted offset: D_k w, reassembled from unit matrix and scale.
            let predicted: [f64; 3] = std::array::from_fn(|row| {
                (0..3)
                    .map(|column| f64::from(unit[row * 3 + column]) * offset[column])
                    .sum::<f64>()
                    * magnitude
            });
            let actual: [f64; 3] = std::array::from_fn(|row| {
                ApMath::to_f64(&math.sub(&truth[fold].post[row], &base[fold].post[row]))
            });
            let norm = actual.iter().map(|v| v * v).sum::<f64>().sqrt();
            let error = (0..3)
                .map(|axis| (predicted[axis] - actual[axis]).powi(2))
                .sum::<f64>()
                .sqrt();
            let relative = if norm > 0.0 { error / norm } else { 0.0 };
            println!(
                "fold {fold:3}: |offset| {norm:.3e}  relative error {relative:.3e}"
            );
        }
    }

    /// What the host measures for the shell, printed for inspection.
    #[test]
    #[ignore = "diagnostic: prints the measured shell-fold table; AP work"]
    fn measured_shell_fold_table() {
        let params = StackParams {
            zoom_exp: 8.0,
            ..StackParams::default()
        };
        let anchor = search_long_lived_anchor(&params).expect("anchor search");
        let (table, camera, standoff) =
            FractalReferenceOrbitAnalyzer::measure_shell_folds(&params, &anchor.target)
                .expect("shell measurement");
        for (slot, fold) in table.iter().enumerate() {
            let depth = 8.0 * (slot as f64 + 1.0) / SHELL_TABLE_LEN as f64;
            let cam = camera[slot];
            let verdict = if cam < *fold { "camera outside" } else { "CAMERA INSIDE" };
            println!(
                "depth {depth:5.2} -> shell {fold:6.0}  camera {cam:6.0}  {verdict}  \
                 standoff {:.2}",
                standoff[slot]
            );
        }
        let derived: Vec<String> = (0..SHELL_TABLE_LEN)
            .map(|slot| {
                let depth = 8.0 * (slot as f64 + 1.0) / SHELL_TABLE_LEN as f64;
                format!("{:.0}", 14.0 + 1.107 * depth)
            })
            .collect();
        println!("the formula would have asked for: {}", derived.join(", "));
    }

    #[test]
    #[ignore = "diagnostic: prints the camera point's true escape fold; AP work"]
    fn parked_camera_ground_truth_escape_fold() {
        for zoom_exp in [6.0, 9.0] {
            let params = StackParams {
                zoom_exp,
                ..StackParams::default()
            };
            let anchor = search_long_lived_anchor(&params).expect("anchor search");
            let mut math = ApMath::with_precision(256);
            let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
            let center: ApV3 = std::array::from_fn(|axis| {
                math.decimal(&anchor.target[axis]).expect("anchor digits")
            });
            let forward = FractalReferenceOrbitAnalyzer::primary_camera(&params).forward;
            let rho = default_cam_dist() * 10.0_f64.powf(-zoom_exp);
            let camera: ApV3 = std::array::from_fn(|axis| {
                math.add(&center[axis], &math.f(-0.25 * rho * forward[axis]))
            });
            let mut verdicts = Vec::new();
            for folds in [64_usize, 113, 128, 256, 512, 1024] {
                verdicts.push((folds, resolved.survives(&mut math, &camera, folds)));
            }
            println!("zoom {zoom_exp}: camera survives {verdicts:?}");
        }
    }

    /// The measured table, checked at the radius it claims.
    ///
    /// The radii are measured by probing, so a test that probes the same
    /// directions would only confirm arithmetic. This one probes a direction the
    /// measurement never used, at exactly the magnitude each segment reported,
    /// and asks whether the prediction still tracks the true dynamics there. It
    /// also insists the table is worth transporting: a table whose segments are
    /// all invalid is a payload cost with no march saving.
    #[test]
    fn segment_table_holds_at_the_radius_it_reports() {
        let params = StackParams {
            formulas: [1, 0, 0, 0],
            rates: [1, 0, 0, 0],
            refine: false,
            zoom_exp: 0.0,
            ..StackParams::default()
        };
        let anchor = [
            "0.31".to_string(),
            "-0.17".to_string(),
            "0.11".to_string(),
        ];
        let folds = 32_usize;
        let table = measure_segment_bla(&params, &anchor, folds).expect("segment table");
        assert_eq!(table.len(), BLA_LEVELS);
        for (level, entries) in table.iter().enumerate() {
            assert_eq!(entries.len(), folds.div_ceil(1 << (level + 1)));
        }

        let usable: usize = table
            .iter()
            .flatten()
            .filter(|segment| segment.radius_log2 > -1.0e29)
            .count();
        let total: usize = table.iter().map(Vec::len).sum();
        assert!(
            usable * 2 >= total,
            "only {usable} of {total} entries are ever valid; the table would not \
             pay for its transport"
        );

        let mut math = ApMath::with_precision(256);
        let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
        let centre: ApV3 =
            std::array::from_fn(|axis| math.decimal(&anchor[axis]).expect("anchor digits"));
        let seed = resolved.sample_seed(&math, &centre);
        let mut reference = Vec::with_capacity(folds + 1);
        reference.push(centre.clone());
        let mut point = centre.clone();
        for iteration in 0..folds {
            point = resolved.step(&mut math, &point, &seed, iteration).post;
            reference.push(point.clone());
        }

        // A direction the measurement did not use, and a ladder of offsets, so
        // each segment can be tested at the largest offset it actually claims.
        //
        // The radius is quoted in the offset at the segment's *start*, which is
        // not something a probe can be given directly - it is whatever the
        // dynamics produce from an initial `w`. So the ladder is walked and each
        // segment is checked at the first magnitude whose arrival at that fold
        // is inside the radius, which is exactly the largest offset a marcher
        // could present it with.
        let direction = [0.4694, 0.8138, -0.3419];
        let ladder: Vec<f64> = (0..64).map(|step| -2.0 - 3.0 * f64::from(step)).collect();
        let mut checked = 0_usize;
        let mut done: Vec<Vec<bool>> = table.iter().map(|e| vec![false; e.len()]).collect();
        for &magnitude_log2 in &ladder {
            if done.iter().flatten().all(|entry| *entry) {
                break;
            }
            let magnitude = magnitude_log2.exp2();
            let offset: [f64; 3] = std::array::from_fn(|k| direction[k] * magnitude);
            let sample: ApV3 =
                std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));
            let sample_seed = resolved.sample_seed(&math, &sample);
            let mut offsets = Vec::with_capacity(folds + 1);
            offsets.push(ScaledVec::from_plain(offset));
            let mut walker = sample.clone();
            for iteration in 0..folds {
                walker = resolved
                    .step(&mut math, &walker, &sample_seed, iteration)
                    .post;
                let difference: [f64; 3] = std::array::from_fn(|axis| {
                    ApMath::to_f64(&math.sub(&walker[axis], &reference[iteration + 1][axis]))
                });
                offsets.push(ScaledVec::from_plain(difference));
            }
            let w = ScaledVec::from_plain(offset);

            for (level, entries) in table.iter().enumerate() {
                let span = 1_usize << (level + 1);
                for (index, segment) in entries.iter().enumerate() {
                    if done[level][index] || segment.radius_log2 <= -1.0e29 {
                        continue;
                    }
                    let start = index * span;
                    let end = start + segment.length as usize;
                    if offsets[start].magnitude_log2() > f64::from(segment.radius_log2) {
                        continue;
                    }
                    done[level][index] = true;
                    checked += 1;
                    let a = ScaledMatrix {
                        unit: std::array::from_fn(|row| {
                            std::array::from_fn(|col| f64::from(segment.a_unit[row * 3 + col]))
                        }),
                        log2: f64::from(segment.a_log2),
                    };
                    let b = ScaledMatrix {
                        unit: std::array::from_fn(|row| {
                            std::array::from_fn(|col| f64::from(segment.b_unit[row * 3 + col]))
                        }),
                        log2: f64::from(segment.b_log2),
                    };
                    let predicted = offsets[start].transformed(&a).added(&w.transformed(&b));
                    let error = predicted.relative_error(&offsets[end]);
                    assert!(
                        error < BLA_TOLERANCE * 4.0,
                        "level {level} segment {index} (folds {start}..{end}) reported a \
                         radius of 2^{} and predicts with relative error {error:.4} at an \
                         arrival of 2^{:.1}",
                        segment.radius_log2,
                        offsets[start].magnitude_log2()
                    );
                }
            }
        }
        assert!(checked > 0, "no segment was ever reached at its own radius");
    }

    /// What the table is worth, in the only currency that matters: folds the
    /// march does not have to run.
    ///
    /// The renderer's cost is folds executed per estimator call, measured at
    /// thirty-four on the Mandelbox at six decades with the existing prefix skip
    /// already applied. This walks the same rule the shader will - at fold `j`,
    /// take the segment if the offset is inside its radius, otherwise run one
    /// fold - and counts. A sample arrives with an offset the size of the frame,
    /// which at depth is many octaves below every radius in the table, so the
    /// early orbit should go in single jumps and the exact folds should be the
    /// few at the end where the offset has grown into the branch structure.
    #[test]
    fn segment_table_skips_most_of_the_orbit() {
        let params = StackParams {
            formulas: [1, 0, 0, 0],
            rates: [1, 0, 0, 0],
            refine: false,
            zoom_exp: 0.0,
            ..StackParams::default()
        };
        let anchor = [
            "0.31".to_string(),
            "-0.17".to_string(),
            "0.11".to_string(),
        ];
        let folds = 64_usize;
        let table = measure_segment_bla(&params, &anchor, folds).expect("segment table");

        let mut math = ApMath::with_precision(512);
        let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
        let centre: ApV3 =
            std::array::from_fn(|axis| math.decimal(&anchor[axis]).expect("anchor digits"));
        let seed = resolved.sample_seed(&math, &centre);
        let mut reference = Vec::with_capacity(folds + 1);
        reference.push(centre.clone());
        let mut point = centre.clone();
        for iteration in 0..folds {
            point = resolved.step(&mut math, &point, &seed, iteration).post;
            reference.push(point.clone());
        }

        for (level, entries) in table.iter().enumerate() {
            let radii: Vec<String> = entries
                .iter()
                .map(|segment| {
                    if segment.radius_log2 > -1.0e29 {
                        format!("{:.0}", segment.radius_log2)
                    } else {
                        "-".to_string()
                    }
                })
                .collect();
            eprintln!("level {level} ({} folds): {}", 1 << (level + 1), radii.join(" "));
        }

        let mut worst_ratio = 0.0_f64;
        let mut deepest = 1.0_f64;
        for offset_log2 in [-20.0_f64, -40.0, -80.0, -160.0] {
            let magnitude = offset_log2.exp2();
            let direction = [0.4694, 0.8138, -0.3419];
            let offset: [f64; 3] = std::array::from_fn(|k| direction[k] * magnitude);
            let sample: ApV3 =
                std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));
            let sample_seed = resolved.sample_seed(&math, &sample);
            let mut offsets = Vec::with_capacity(folds + 1);
            offsets.push(ScaledVec::from_plain(offset));
            let mut walker = sample.clone();
            for iteration in 0..folds {
                walker = resolved
                    .step(&mut math, &walker, &sample_seed, iteration)
                    .post;
                let difference: [f64; 3] = std::array::from_fn(|axis| {
                    ApMath::to_f64(&math.sub(&walker[axis], &reference[iteration + 1][axis]))
                });
                offsets.push(ScaledVec::from_plain(difference));
            }

            let mut fold = 0_usize;
            let mut executed = 0_usize;
            let mut jumps = 0_usize;
            while fold < folds {
                let magnitude = offsets[fold].magnitude_log2();
                // The longest jump this offset allows, which is what levels are
                // for: long early, shortening as the offset grows.
                let mut taken = 0_usize;
                for level in (0..table.len()).rev() {
                    let span = 1_usize << (level + 1);
                    if fold % span != 0 {
                        continue;
                    }
                    let index = fold / span;
                    let Some(segment) = table[level].get(index) else {
                        continue;
                    };
                    if segment.length as usize == 0 {
                        continue;
                    }
                    if f64::from(segment.radius_log2) >= magnitude {
                        taken = segment.length as usize;
                        break;
                    }
                }
                if taken > 0 {
                    fold += taken;
                    jumps += 1;
                } else {
                    fold += 1;
                    executed += 1;
                }
            }
            let ratio = executed as f64 / folds as f64;
            worst_ratio = worst_ratio.max(ratio);
            deepest = ratio;
            eprintln!(
                "offset 2^{offset_log2}: {executed} folds executed and {jumps} jumps \
                 against {folds} folds ({:.0}% of the work)",
                ratio * 100.0
            );
        }
        // The shallow end has nothing to give: a sample twenty octaves from the
        // reference reaches the branch structure within a dozen folds and the
        // table can only cover the first jump. The claim is about depth, which
        // is where this renderer spends its life and where the headroom is.
        assert!(
            deepest < 0.25,
            "at the deepest offset the table still ran {:.0}% of the folds by hand",
            deepest * 100.0
        );
        assert!(worst_ratio <= 1.0);
    }

    /// The segment form of the paper's parameter Jacobian, checked against the
    /// orbit it claims to predict.
    ///
    /// Statement two of the stratified perturbation theorem gives the prefix
    /// Jacobian, `D_0 = I`, `D_{k+1} = J_k D_k + S_k`. Written over an interval
    /// rather than from zero it is the bilinear form that replaced series
    /// approximation in two-dimensional deep zoom:
    ///
    ///     e_k = A_{j→k} e_j + B_{j→k} w
    ///     A_{j→k+1} = J_k A_{j→k},    B_{j→k+1} = J_k B_{j→k} + S_k
    ///
    /// and adjacent segments compose as `A = A_y A_x`, `B = A_y B_x + B_y`.
    /// The renderer skips only the prefix today; the value of the segment form
    /// is that it skips repeatedly, which is what makes a deep shell cost what
    /// a shallow one does. This test is the part that has to be true before any
    /// of that is worth building: that `J_k` and `S_k` measured as *local*
    /// derivatives — three central differences of a single fold each, not of a
    /// whole orbit — compose into a map that reproduces the real perturbed
    /// orbit.
    #[test]
    fn segment_jacobians_compose_into_the_perturbed_orbit() {
        let params = StackParams {
            formulas: [1, 0, 0, 0],
            rates: [1, 0, 0, 0],
            refine: false,
            zoom_exp: 0.0,
            ..StackParams::default()
        };
        let mut math = ApMath::with_precision(192);
        let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
        // An ordinary interior point rather than the origin, which is a fixed
        // point of the box fold and would make every Jacobian the same matrix.
        let centre: ApV3 = [math.f(0.31), math.f(-0.17), math.f(0.11)];
        let folds = 12_usize;

        // The reference orbit, and the local derivatives at each of its points.
        let mut reference = Vec::with_capacity(folds + 1);
        reference.push(centre.clone());
        let mut point = centre.clone();
        for iteration in 0..folds {
            point = resolved.step(&mut math, &point, &centre, iteration).post;
            reference.push(point.clone());
        }

        // Central differences of ONE fold. The step is far above the working
        // precision and far below the reference, which is what makes this both
        // accurate and slot-agnostic: no per-formula derivative is written down.
        let delta = math.f(1e-20);
        let two_delta = math.add(&delta, &delta);
        let basis = |axis: usize, sign: f64, math: &ApMath| -> ApV3 {
            std::array::from_fn(|k| {
                if k == axis {
                    math.mul(&delta, &math.f(sign))
                } else {
                    math.zero()
                }
            })
        };
        let mut jac = Vec::with_capacity(folds);
        let mut seed_jac = Vec::with_capacity(folds);
        for iteration in 0..folds {
            let base = reference[iteration].clone();
            let mut j = [[0.0_f64; 3]; 3];
            let mut sj = [[0.0_f64; 3]; 3];
            for axis in 0..3 {
                let plus = math.add_v(&base, &basis(axis, 1.0, &math));
                let minus = math.add_v(&base, &basis(axis, -1.0, &math));
                let hi = resolved.step(&mut math, &plus, &centre, iteration).post;
                let lo = resolved.step(&mut math, &minus, &centre, iteration).post;
                for row in 0..3 {
                    let diff = math.sub(&hi[row], &lo[row]);
                    j[row][axis] = ApMath::to_f64(&math.div(&diff, &two_delta));
                }
                let seed_plus = math.add_v(&centre, &basis(axis, 1.0, &math));
                let seed_minus = math.add_v(&centre, &basis(axis, -1.0, &math));
                let hi = resolved.step(&mut math, &base, &seed_plus, iteration).post;
                let lo = resolved.step(&mut math, &base, &seed_minus, iteration).post;
                for row in 0..3 {
                    let diff = math.sub(&hi[row], &lo[row]);
                    sj[row][axis] = ApMath::to_f64(&math.div(&diff, &two_delta));
                }
            }
            jac.push(j);
            seed_jac.push(sj);
        }

        // Compose the prefix segment fold by fold, exactly as a merge tree's
        // level zero would, then compare against the orbit of a real offset.
        let mul = |a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]| -> [[f64; 3]; 3] {
            let mut out = [[0.0; 3]; 3];
            for row in 0..3 {
                for col in 0..3 {
                    out[row][col] =
                        (0..3).map(|k| a[row][k] * b[k][col]).sum::<f64>();
                }
            }
            out
        };
        let apply = |m: &[[f64; 3]; 3], v: &[f64; 3]| -> [f64; 3] {
            std::array::from_fn(|row| (0..3).map(|k| m[row][k] * v[k]).sum())
        };

        let offset = [3e-9_f64, -1.7e-9, 0.9e-9];
        let sample: ApV3 =
            std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));
        let mut truth = sample.clone();

        let mut a = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut b = [[0.0_f64; 3]; 3];
        let mut worst = 0.0_f64;
        for iteration in 0..folds {
            truth = resolved.step(&mut math, &truth, &sample, iteration).post;
            // A_{j→k+1} = J_k A_{j→k};  B_{j→k+1} = J_k B_{j→k} + S_k
            a = mul(&jac[iteration], &a);
            b = mul(&jac[iteration], &b);
            for row in 0..3 {
                for col in 0..3 {
                    b[row][col] += seed_jac[iteration][row][col];
                }
            }
            // e_0 is the offset itself, so both blocks act on the same vector.
            let linear = {
                let from_state = apply(&a, &offset);
                let from_seed = apply(&b, &offset);
                [
                    from_state[0] + from_seed[0],
                    from_state[1] + from_seed[1],
                    from_state[2] + from_seed[2],
                ]
            };
            let exact: [f64; 3] = std::array::from_fn(|axis| {
                ApMath::to_f64(&math.sub(&truth[axis], &reference[iteration + 1][axis]))
            });
            let magnitude = exact.iter().map(|v| v * v).sum::<f64>().sqrt();
            if magnitude < 1e-30 {
                continue;
            }
            let error = (0..3)
                .map(|axis| (linear[axis] - exact[axis]).powi(2))
                .sum::<f64>()
                .sqrt()
                / magnitude;
            worst = worst.max(error);
            assert!(
                error < 0.05,
                "fold {iteration}: composed segment predicts {linear:?} against {exact:?} \
                 (relative error {error:.3}, offset magnitude {magnitude:.3e})"
            );
        }
        assert!(worst > 0.0, "the offset never grew, so nothing was tested");
    }

    /// Prints the escape fold across the frame as a map, which is the one
    /// diagnostic that says whether a frame is worth rendering at all.
    ///
    /// Three failures look identical in a rendered still and are trivially
    /// distinguishable here: a constant map is a solid (the march reports
    /// interior everywhere); a map that varies only with distance from the
    /// centre is a smooth patch of boundary, and the renderer draws its
    /// iso-escape contours as concentric rings; a map that varies in every
    /// direction is fractal structure at frame scale.
    /// Walks the parked camera ray and scores each point by how much the escape
    /// fold *varies* around it at frame scale.
    ///
    /// Placing the anchor on the boundary is necessary and nowhere near
    /// sufficient. A boundary point on a smooth patch of the surface has a
    /// neighbourhood whose escape fold depends only on the distance from the
    /// surface, and the renderer draws its iso-fold contours as concentric
    /// rings - which is exactly what the deep frames look like. Structure worth
    /// flying into is where the fold varies *around* the point as well as away
    /// from it, so that is what this measures: a ring of samples at the frame
    /// radius, and the spread and the number of alternations around it.
    #[test]
    #[ignore = "diagnostic: scores structure along the parked ray; AP work"]
    fn parked_ray_structure_profile() {
        let zoom_exp: f64 = std::env::var("VARDA_ZOOM")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(6.0);
        let formula: i64 = std::env::var("VARDA_FORMULA")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5);
        let stack_cap: f64 = std::env::var("VARDA_CAP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(14.0);
        let params = StackParams {
            zoom_exp,
            formulas: [formula, 0, 0, 0],
            stack_cap,
            ..StackParams::default()
        };
        let horizon = shell_fold_target(&params);
        let ray = parked_ray_geometry(ParkedCamera {
            distance: default_cam_dist(),
            azimuth: PARKED_AZIMUTH,
            elevation: PARKED_ELEVATION,
            look: PARKED_LOOK,
        })
        .expect("parked ray");
        let precision =
            ApMath::for_anchor_search(orbit_zoom_exp(zoom_exp), horizon, params.power).precision;
        let mut math = ApMath::with_precision(precision);
        let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
        let camera = FractalReferenceOrbitAnalyzer::primary_camera(&params);
        let rho = default_cam_dist() * 10.0_f64.powf(-zoom_exp);
        let ring = 16;

        println!("zoom {zoom_exp}, horizon {horizon} folds, frame radius {rho:.3e}");
        println!("     t   centre   ring folds (# = bounded)        spread  alternations");
        let steps = 48;
        for step in 0..=steps {
            let fraction = step as f64 / steps as f64;
            let t = math.f(fraction);
            let centre = point_on_parked_ray(&math, &ray, &t);
            let centre_fold = resolved.escape_fold(&mut math, &centre, horizon);
            let mut folds = Vec::new();
            let mut glyphs = String::new();
            for index in 0..ring {
                let angle = std::f64::consts::TAU * index as f64 / ring as f64;
                let offset: [f64; 3] = std::array::from_fn(|axis| {
                    rho * (angle.cos() * camera.right[axis] + angle.sin() * camera.up[axis])
                });
                let point: ApV3 =
                    std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));
                match resolved.escape_fold(&mut math, &point, horizon) {
                    Some(fold) => {
                        folds.push(fold as f64);
                        glyphs.push(char::from(
                            b'0' + (fold * 10 / horizon.max(1)).min(9) as u8,
                        ));
                    }
                    None => {
                        folds.push(f64::NAN);
                        glyphs.push('#');
                    }
                }
            }
            // Alternations: how often the ring changes between bounded and
            // escaped, or jumps a decile. A smooth shell scores zero.
            let mut alternations = 0;
            for index in 0..ring {
                let a = folds[index];
                let b = folds[(index + 1) % ring];
                let changed = match (a.is_nan(), b.is_nan()) {
                    (true, true) => false,
                    (false, false) => (a - b).abs() > horizon as f64 / 10.0,
                    _ => true,
                };
                if changed {
                    alternations += 1;
                }
            }
            let finite: Vec<f64> = folds.iter().copied().filter(|v| !v.is_nan()).collect();
            let spread = if finite.len() < 2 {
                0.0
            } else {
                let mean = finite.iter().sum::<f64>() / finite.len() as f64;
                (finite.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / finite.len() as f64)
                    .sqrt()
            };
            let centre_text = match centre_fold {
                Some(fold) => format!("{fold:6}"),
                None => "     #".to_string(),
            };
            println!("{fraction:6.3}  {centre_text}   {glyphs}   {spread:6.1}   {alternations:3}");
        }
    }

    #[test]
    #[ignore = "diagnostic: prints the frame's escape-fold map; AP work"]
    fn frame_escape_fold_map() {
        let zoom_exp: f64 = std::env::var("VARDA_ZOOM")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(6.0);
        let params = StackParams {
            zoom_exp,
            ..StackParams::default()
        };
        let anchor = search_long_lived_anchor(&params).expect("anchor search");
        println!(
            "zoom {zoom_exp}: anchor survives >= {}, transported {}",
            anchor.survival, anchor.transported_iterations
        );
        let mut math = ApMath::with_precision(
            ApMath::for_anchor_search(orbit_zoom_exp(zoom_exp), anchor.survival, params.power)
                .precision,
        );
        let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
        let centre: ApV3 =
            std::array::from_fn(|axis| math.decimal(&anchor.target[axis]).expect("anchor digits"));
        let camera = FractalReferenceOrbitAnalyzer::primary_camera(&params);
        let rho = default_cam_dist() * 10.0_f64.powf(-zoom_exp);
        let horizon = anchor.transported_iterations;

        let grid = 12_i64;
        let mut rows = Vec::new();
        let mut folds = Vec::new();
        for gy in -grid..=grid {
            let mut row = String::new();
            for gx in -grid..=grid {
                let offset: [f64; 3] = std::array::from_fn(|axis| {
                    rho * ((gx as f64) * camera.right[axis] + (gy as f64) * camera.up[axis])
                        / (grid as f64)
                });
                let point: ApV3 =
                    std::array::from_fn(|axis| math.add(&centre[axis], &math.f(offset[axis])));
                match resolved.escape_fold(&mut math, &point, horizon) {
                    Some(fold) => {
                        folds.push(fold);
                        // One character per octave of escape fold, so the map
                        // stays readable whatever the horizon is.
                        let digit = (fold * 10 / horizon.max(1)).min(9);
                        row.push(char::from(b'0' + digit as u8));
                    }
                    None => row.push('#'),
                }
            }
            rows.push(row);
        }
        for row in &rows {
            println!("{row}");
        }
        let bounded = (grid * 2 + 1).pow(2) as usize - folds.len();
        let mean = if folds.is_empty() {
            0.0
        } else {
            folds.iter().sum::<usize>() as f64 / folds.len() as f64
        };
        println!(
            "bounded {bounded} of {}, escaped mean fold {mean:.1}, spread {}..{}",
            (grid * 2 + 1).pow(2),
            folds.iter().min().copied().unwrap_or(0),
            folds.iter().max().copied().unwrap_or(0)
        );
    }

    #[test]
    #[ignore = "diagnostic: prints ground-truth escape histograms; minutes of AP work"]
    fn deep_frame_ground_truth_escape_histogram() {
        let zoom_exp = 9.0;
        let params = StackParams {
            zoom_exp,
            ..StackParams::default()
        };
        let anchor = search_long_lived_anchor(&params).expect("anchor search");
        println!(
            "anchor survives >= {} folds, transported {}",
            anchor.survival, anchor.transported_iterations
        );

        let mut math = ApMath::with_precision(256);
        let resolved = ApResolved::new(&params, &mut math).expect("resolved stack");
        let center: ApV3 =
            std::array::from_fn(|axis| math.decimal(&anchor.target[axis]).expect("anchor digits"));
        // Physical frame radius at this depth, matching `dzFrameLog2`.
        let rho = default_cam_dist() * 10.0_f64.powf(-zoom_exp);

        let thresholds = [64_usize, 128, 256, 512];
        let mut histogram = [0_usize; 5];
        let grid = 4_i64;
        for gx in -grid..=grid {
            for gy in -grid..=grid {
                let offset = [
                    rho * (gx as f64) / (grid as f64),
                    rho * (gy as f64) / (grid as f64),
                    0.0,
                ];
                let point: ApV3 = std::array::from_fn(|axis| {
                    math.add(&center[axis], &math.f(offset[axis]))
                });
                let mut bucket = thresholds.len();
                for (index, folds) in thresholds.iter().enumerate() {
                    if !resolved.survives(&mut math, &point, *folds) {
                        bucket = index;
                        break;
                    }
                }
                histogram[bucket] += 1;
            }
        }
        let labels = ["<64", "<128", "<256", "<512", ">=512"];
        for (label, count) in labels.iter().zip(histogram) {
            println!("escape folds {label}: {count}");
        }
    }

    /// `"certificates": false` in the authored options declines atlas work
    /// even at an explicit static depth; the default keeps it on.
    #[test]
    fn authored_certificate_opt_out_disables_atlas_work() {
        let mut state = AnalyzerStateSnapshot::default();
        state
            .values
            .insert("zoom_exp".into(), crate::params::ParamValue::Float(6.0));

        let declined: StackParams = serde_json::from_value(serde_json::json!({
            "formulas": [5, 0, 0, 0],
            "rates": [1, 0, 0, 0],
            "certificates": false
        }))
        .unwrap();
        assert!(!declined.with_live_state(&state).certificate_enabled);

        let default_on: StackParams = serde_json::from_value(serde_json::json!({
            "formulas": [5, 0, 0, 0],
            "rates": [1, 0, 0, 0]
        }))
        .unwrap();
        assert!(default_on.with_live_state(&state).certificate_enabled);
    }

    /// A state change publishes a shallow payload first and owes a refinement.
    ///
    /// The renderer has nothing to draw during a full-depth anchor search, so
    /// the first pass after a change publishes the cheap orbit and the second
    /// replaces it. The provisional payload must be signed against the live
    /// state, or the gate would reject the very thing published to fill the
    /// gap, so everything the signature hashes has to survive the swap.
    #[test]
    fn a_state_change_publishes_provisionally_then_owes_a_refinement() {
        let live = StackParams {
            zoom_exp: 9.0,
            certificate_enabled: true,
            ..StackParams::default()
        };
        let provisional = FractalReferenceOrbitAnalyzer::provisional_params(&live);
        assert_eq!(provisional.zoom_exp, 0.0);
        assert!(!provisional.certificate_enabled);
        assert_eq!(
            FractalReferenceOrbitAnalyzer::state_signature(&provisional),
            FractalReferenceOrbitAnalyzer::state_signature(&live),
            "the provisional payload must carry the live state's signature"
        );

        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [5, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "refine": false,
                "max_iters": 4,
                "anchor": ["0.31", "-0.17", "0.23"]
            }))
            .unwrap();

        let mut state = AnalyzerStateSnapshot::default();
        state
            .values
            .insert("power".into(), crate::params::ParamValue::Float(7.0));
        let input = AnalyzerInput {
            frame: Vec::new(),
            width: 0,
            height: 0,
            timestamp: std::time::Instant::now(),
            state,
        };

        analyzer.analyze(&input).unwrap();
        assert!(
            analyzer.needs_full_orbit,
            "the first pass after a change owes a full-depth refinement"
        );
        assert!(analyzer.cached_texture.is_some(), "nothing was published");

        analyzer.analyze(&input).unwrap();
        assert!(
            !analyzer.needs_full_orbit,
            "the settled pass must discharge the refinement"
        );
    }

    /// A depth change inside one bucket must not regenerate the orbit.
    ///
    /// This is the analyzer half of surviving a live zoom gesture: the shader
    /// no longer hashes depth into the payload gate, and the host no longer
    /// re-runs the arbitrary-precision pipeline for every frame of a slider
    /// drag. Only crossing a depth bucket (or changing the stack) regenerates.
    #[test]
    fn zoom_changes_inside_a_bucket_reuse_the_published_orbit() {
        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [5, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "refine": false,
                "max_iters": 4,
                "anchor": ["0.31", "-0.17", "0.23"]
            }))
            .unwrap();

        let analyze_at = |analyzer: &mut FractalReferenceOrbitAnalyzer, flight_max: f64| {
            let mut state = AnalyzerStateSnapshot::default();
            state.values.insert(
                "flight_max_exp".into(),
                crate::params::ParamValue::Float(flight_max as f32),
            );
            analyzer
                .analyze(&AnalyzerInput {
                    frame: Vec::new(),
                    width: 0,
                    height: 0,
                    timestamp: std::time::Instant::now(),
                    state,
                })
                .unwrap();
        };

        // Crossing from the zero bucket into the first one regenerates.
        analyze_at(&mut analyzer, 2.0);
        let crossed = analyzer.cached_texture.as_ref().unwrap().generation;

        // Any further depth inside the same bucket reuses that payload.
        analyze_at(&mut analyzer, 3.0);
        assert_eq!(
            analyzer.cached_texture.as_ref().unwrap().generation,
            crossed,
            "zoom change inside one bucket regenerated the orbit"
        );
        analyze_at(&mut analyzer, 6.5);
        assert_eq!(
            analyzer.cached_texture.as_ref().unwrap().generation,
            crossed,
            "zoom change inside one bucket regenerated the orbit"
        );
    }

    /// A depth change must drop a certificate atlas certified at another depth.
    ///
    /// The payload gate no longer includes depth, so nothing shader-side
    /// prevents consuming stale certified prefixes; the host is responsible
    /// for never publishing an atlas at a depth other than the one it was
    /// certified for.
    #[test]
    fn zoom_change_drops_an_atlas_certified_at_another_depth() {
        use crate::internal::analyzer::fractal_certification::segment::AtlasMetadata;

        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [5, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "refine": false,
                "max_iters": 4,
                "anchor": ["0.31", "-0.17", "0.23"]
            }))
            .unwrap();
        analyzer.params.zoom_exp = 6.0;
        analyzer.params.certificate_enabled = true;
        analyzer.segment_atlas = Some(SegmentAtlasResult::Certified {
            metadata: AtlasMetadata {
                columns: ATLAS_COLUMNS,
                rows: ATLAS_ROWS,
                t_slabs: 24,
                max_frame_t: 24.0,
            },
            tiles: Vec::new(),
        });
        analyzer.atlas_zoom = Some(6.0);
        // Keep the settled-state hook from immediately re-certifying, so the
        // drop is observable on its own.
        analyzer.anchor_target = None;

        let mut state = AnalyzerStateSnapshot::default();
        state
            .values
            .insert("zoom_exp".into(), crate::params::ParamValue::Float(7.0));
        analyzer
            .analyze(&AnalyzerInput {
                frame: Vec::new(),
                width: 0,
                height: 0,
                timestamp: std::time::Instant::now(),
                state,
            })
            .unwrap();

        assert!(
            analyzer.arbitrary_records.is_some(),
            "records must survive a same-bucket depth change"
        );
        assert!(
            analyzer.segment_atlas.is_none(),
            "an atlas certified at depth 6 must not be published at depth 7"
        );
        assert_eq!(analyzer.atlas_zoom, None);
    }

    /// The transported length is the renderer's fold budget, not the zoom depth.
    ///
    /// Deriving it from `zoom_exp` alone sent ten records for a frame radius of
    /// 1e-6, where the shader's own budget is `stack_cap` plus twenty
    /// depth-driven folds. Because the shader clamps `total` to the payload
    /// length, the extra thirty-odd folds were dropped without a word and the
    /// frame lost its geometry rather than its depth.
    #[test]
    fn transported_length_covers_the_fold_budget_the_shader_will_march() {
        let base = StackParams {
            max_iters: 512,
            ..StackParams::default()
        };

        for (zoom_exp, cutoff) in [(0.0, 14.0), (6.0, 14.0), (20.0, 14.0), (6.0, 40.0)] {
            let params = StackParams {
                zoom_exp,
                stack_cap: cutoff,
                ..base.clone()
            };
            let (transported, survival) = anchor_search_work(&params).unwrap();
            // One fold per magnification by the stack's expansion rate. The
            // depth term is NOT `bucketed_depth_folds`, which counts bits of
            // precision: a fold buys log2(power) of magnification, and using
            // bits as folds overstated the target threefold at power eight,
            // which put the anchor on the boundary of a level set the shader
            // could not reach and rendered the dive as a solid wall.
            let expansion = params.power.log2();
            let budget =
                (cutoff + zoom_exp * std::f64::consts::LOG2_10 / expansion).ceil() as usize;
            assert_eq!(
                transported,
                budget.min(SHADER_FOLD_CEILING),
                "zoom {zoom_exp} cutoff {cutoff} transported {transported} for budget {budget}"
            );
            assert_eq!(survival, transported + ANCHOR_SURVIVAL_HEADROOM);
        }

        // The oldest rule is what has to stay gone: at 1e-6 it produced ten,
        // which is fewer folds than the authored cutoff alone.
        let (transported, _) = anchor_search_work(&StackParams {
            zoom_exp: 6.0,
            ..base.clone()
        })
        .unwrap();
        assert!(
            transported > default_stack_cap() as usize,
            "fold budget collapsed back to {transported}"
        );

        // Nothing past the shader's own ceiling is worth transporting, and the
        // authored orbit length still caps the payload below it.
        let (deep, _) = anchor_search_work(&StackParams {
            zoom_exp: 1_000.0,
            ..base.clone()
        })
        .unwrap();
        assert_eq!(deep, SHADER_FOLD_CEILING);
        // A short authored orbit cannot prove enough lifetime for the renderer's
        // requested fold budget, so publication fails closed.
        assert!(anchor_search_work(&StackParams {
            zoom_exp: 1_000.0,
            max_iters: 40,
            ..base
        })
        .is_err());

        assert!(anchor_search_work(&StackParams {
            zoom_exp: 6.0,
            max_iters: ANCHOR_SURVIVAL_HEADROOM,
            ..StackParams::default()
        })
        .is_err());
    }

    #[test]
    fn anchor_search_caps_parallel_context_duplication() {
        assert!((1..=6).contains(&search_workers()));
    }

    /// A many-way round keeps the same bracket a halving would have kept.
    ///
    /// The refinement narrows by testing several interior points at once, which
    /// is only sound if it preserves bisection's invariant: the outer end never
    /// survives, the inner end always does. Getting the off-by-one wrong here
    /// would move the anchor to a point on the wrong side of the transition, and
    /// the resulting frame would look like a bad estimator rather than a bad
    /// bracket, which is the failure this whole area keeps producing.
    #[test]
    fn many_way_refinement_keeps_the_transition_bracketed() {
        let math = ApMath::with_precision(128);
        let interior: Vec<Hp> = (1..=3).map(|step| math.f(f64::from(step) / 4.0)).collect();
        let outside = math.zero();
        let inside = math.f(1.0);
        let at = |value: &Hp| value.to_f64().value();

        // The transition sits inside the first gap, so the bracket keeps the
        // original outer end.
        let (lo, hi) = narrow_bracket(
            &interior,
            &[true, true, true],
            outside.clone(),
            inside.clone(),
        );
        assert!((at(&lo) - 0.0).abs() < 1e-12);
        assert!((at(&hi) - 0.25).abs() < 1e-12);

        // A transition in the middle takes the straddling pair, not the ends.
        let (lo, hi) = narrow_bracket(
            &interior,
            &[false, true, true],
            outside.clone(),
            inside.clone(),
        );
        assert!((at(&lo) - 0.25).abs() < 1e-12);
        assert!((at(&hi) - 0.5).abs() < 1e-12);

        let (lo, hi) = narrow_bracket(
            &interior,
            &[false, false, true],
            outside.clone(),
            inside.clone(),
        );
        assert!((at(&lo) - 0.5).abs() < 1e-12);
        assert!((at(&hi) - 0.75).abs() < 1e-12);

        // Nothing interior survives, so the transition is past the last point and
        // the inner end is still the only witness.
        let (lo, hi) = narrow_bracket(&interior, &[false, false, false], outside, inside);
        assert!((at(&lo) - 0.75).abs() < 1e-12);
        assert!((at(&hi) - 1.0).abs() < 1e-12);
    }

    /// One rung of the transport ladder: the anchor outlives everything the
    /// shader will march, and an independent context agrees that it does.
    fn assert_transport_rung(zoom_exp: f64, budget_per_rung: std::time::Duration) {
        let params = StackParams {
            formulas: [5, 5, 8, 9],
            rates: [1, 0, 0, 0],
            power: 8.0,
            zoom_exp,
            max_iters: 512,
            refine: true,
            ..StackParams::default()
        };
        let started = std::time::Instant::now();
        let generated = FractalReferenceOrbitAnalyzer::generate_arbitrary_records(&params).unwrap();
        assert!(generated.alive, "zoom {zoom_exp} transport escaped");
        assert!(
            generated.anchor_survival >= generated.records.len() + ANCHOR_SURVIVAL_HEADROOM,
            "zoom {zoom_exp} anchor lifetime {} left no headroom over {} records",
            generated.anchor_survival,
            generated.records.len()
        );
        // The shader clamps its fold budget to this length, so it decides how
        // much structure the frame can develop rather than merely how deep
        // the reference reaches. Taken from `shell_fold_target` rather than
        // restated: the whole point of that function is that the anchor, the
        // records and the shader's shell are one number, and a test that spells
        // the formula out a second time is a second place for them to disagree.
        let budget = shell_fold_target(&params);
        assert!(
            generated.records.len() >= budget.min(SHADER_FOLD_CEILING),
            "zoom {zoom_exp} transported {} records against a fold budget of {budget}",
            generated.records.len()
        );

        let anchor = generated.records[0].pre.clone();
        let mut math = ApMath::for_anchor_search(zoom_exp, generated.anchor_survival, params.power);
        let resolved = ApResolved::new(&params, &mut math).unwrap();
        assert!(
            resolved
                .records(&mut math, &anchor, generated.anchor_survival)
                .1,
            "zoom {zoom_exp} anchor did not independently survive {} iterations",
            generated.anchor_survival
        );
        eprintln!(
            "zoom {zoom_exp}: transported {}, survived {}, runtime {:?}",
            generated.records.len(),
            generated.anchor_survival,
            started.elapsed()
        );
        assert!(
            started.elapsed() < budget_per_rung,
            "zoom {zoom_exp} anchor search took {:?}, over its {budget_per_rung:?} budget",
            started.elapsed()
        );
    }

    /// The contract rungs, which are the ones every run has to check.
    ///
    /// Refinement is sequential bisection whose round count grows linearly in
    /// `zoom_exp`, so the deep rungs cost several times what these do and say
    /// nothing extra about the contract: the transport rule, the headroom rule
    /// and the independent survival check are all exercised here. Depth itself
    /// is a measurement, so it lives in the ignored test below.
    #[test]
    fn long_lived_preview_bulb_anchor_outlives_transport_depth_ladder() {
        for zoom_exp in [6.0, 12.0] {
            assert_transport_rung(zoom_exp, std::time::Duration::from_secs(30));
        }
    }

    /// The measured depth ladder.
    ///
    /// Ignored by default because it is a measurement rather than a contract and
    /// its cost is dominated by bisection rounds that scale with the requested
    /// depth. Run it when the anchor search or the arbitrary-precision backend
    /// changes:
    ///
    /// ```sh
    /// cargo test --lib preview_bulb_anchor_reaches_measured_depth -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measurement: minutes of arbitrary-precision bisection"]
    fn preview_bulb_anchor_reaches_measured_depth() {
        for zoom_exp in [30.0, 100.0] {
            assert_transport_rung(zoom_exp, std::time::Duration::from_secs(120));
        }
    }

    #[test]
    fn live_state_matches_static_shader_order_and_ignores_evolution() {
        let base = StackParams::default();
        let mut state = AnalyzerStateSnapshot::default();
        state
            .values
            .insert("formula0".into(), crate::params::ParamValue::Long(1));
        state
            .values
            .insert("formula1".into(), crate::params::ParamValue::Long(5));
        state
            .values
            .insert("rate0".into(), crate::params::ParamValue::Float(2.9));
        state
            .values
            .insert("rate1".into(), crate::params::ParamValue::Float(3.1));
        state
            .values
            .insert("stack_order".into(), crate::params::ParamValue::Long(1));
        state.values.insert(
            "evolve_amount".into(),
            crate::params::ParamValue::Float(0.25),
        );
        state.values.insert(
            "evolve_phase".into(),
            crate::params::ParamValue::Float(std::f32::consts::FRAC_PI_2),
        );
        state
            .values
            .insert("evolve_target".into(), crate::params::ParamValue::Long(0));

        let live = base.with_live_state(&state);
        assert_eq!(live.formulas[..2], [5, 1]);
        assert_eq!(live.rates[..2], [3, 2]);
        assert_eq!(live.scale, 2.0);
    }

    #[test]
    fn animated_flight_budgets_maximum_depth_without_enabling_static_certificate() {
        let base = StackParams::default();
        let mut state = AnalyzerStateSnapshot::default();
        state.values.insert(
            "flight_max_exp".into(),
            crate::params::ParamValue::Float(12.0),
        );

        let flight = base.with_live_state(&state);
        assert_eq!(flight.zoom_exp, 12.0);
        assert!(!flight.certificate_enabled);

        state
            .values
            .insert("zoom_exp".into(), crate::params::ParamValue::Float(6.0));
        let fixed = base.with_live_state(&state);
        assert_eq!(fixed.zoom_exp, 6.0);
        assert!(fixed.certificate_enabled);
    }

    fn mandelbox_only() -> StackParams {
        StackParams {
            formulas: [1, 0, 0, 0],
            rates: [1, 0, 0, 0],
            ..StackParams::default()
        }
    }

    #[test]
    fn double_double_beats_f64_on_a_cancelling_sum() {
        // (1 + e) - 1 recovers e exactly in double-double and loses it in f64.
        let e = 1e-25_f64;
        let dd = Dd::new(1.0).add(Dd::new(e)).sub(Dd::new(1.0));
        assert!(
            (dd.to_f64() - e).abs() < e * 1e-10,
            "double-double lost the small term: {}",
            dd.to_f64()
        );
        assert_eq!((1.0_f64 + e) - 1.0, 0.0, "f64 was expected to lose it");
    }

    #[test]
    fn mandelbulb_slot_selects_arbitrary_precision_backend() {
        let params = StackParams {
            formulas: [5, 0, 0, 0], // Mandelbulb
            rates: [1, 0, 0, 0],
            refine: false,
            max_iters: 4,
            ..StackParams::default()
        };
        let records = FractalReferenceOrbitAnalyzer::generate_arbitrary_records(&params)
            .unwrap()
            .records;
        assert!(!records.is_empty());
        assert!(records[0].bulb.is_some());
    }

    #[test]
    fn arbitrary_power_bulb_matches_shader_formula() {
        let point = [0.35_f64, -0.21, 0.14];
        for power in [2.0_f64, 7.5, 8.0, 12.0] {
            let params = StackParams {
                formulas: [5, 0, 0, 0],
                rates: [1, 0, 0, 0],
                power,
                julia_amount: 0.35,
                julia: [-0.2, 0.11, 0.07],
                refine: false,
                max_iters: 1,
                ..StackParams::default()
            };
            let mut math = ApMath::for_zoom(80.0);
            let resolved = ApResolved::new(&params, &mut math).unwrap();
            let anchor = point.map(|value| math.f(value));
            let seed = std::array::from_fn(|index| {
                point[index] * (1.0 - params.julia_amount)
                    + params.julia[index] * params.julia_amount
            });
            let arbitrary_seed = seed.map(|value| math.f(value));
            let record = resolved.step(&mut math, &anchor, &arbitrary_seed, 0);

            let radius = point.iter().map(|value| value * value).sum::<f64>().sqrt();
            let theta = (point[2] / radius.max(1.0e-6)).clamp(-1.0, 1.0).acos();
            let phi = point[1].atan2(point[0]);
            let radius_power = radius.powf(power);
            let expected = [
                radius_power * (theta * power).sin() * (phi * power).cos() + seed[0],
                radius_power * (theta * power).sin() * (phi * power).sin() + seed[1],
                radius_power * (theta * power).cos() + seed[2],
            ];
            for (component, expected) in record.post.iter().zip(expected) {
                let actual = ApMath::to_f64(component);
                assert!(
                    (actual - expected).abs() <= 4.0e-14,
                    "power {power}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn non_integer_bulb_records_both_sides_of_principal_seam() {
        let params = StackParams {
            formulas: [5, 0, 0, 0],
            rates: [1, 0, 0, 0],
            power: 7.5,
            refine: false,
            max_iters: 1,
            ..StackParams::default()
        };
        let mut math = ApMath::for_zoom(120.0);
        let resolved = ApResolved::new(&params, &mut math).unwrap();
        for (y, expected_side) in [(1.0e-40, 1), (-1.0e-40, -1)] {
            let point = [math.f(-0.5), math.f(y), math.f(0.2)];
            let record = resolved.step(&mut math, &point, &point, 0);
            assert_eq!(record.bulb.as_ref().unwrap().seam_side, expected_side);
        }
    }

    #[test]
    fn arbitrary_precision_budget_grows_with_zoom_depth() {
        let shallow = ApMath::for_zoom(0.0);
        let deep = ApMath::for_zoom(300.0);
        assert!(deep.precision > shallow.precision);
        assert!(deep.precision >= 1_000);
    }

    #[test]
    fn payload_signature_changes_with_live_bulb_state() {
        let base = StackParams::default();
        let mut changed = base.clone();
        changed.power = 7.5;
        assert_ne!(
            FractalReferenceOrbitAnalyzer::state_signature(&base),
            FractalReferenceOrbitAnalyzer::state_signature(&changed)
        );
        changed = base.clone();
        changed.formulas.swap(0, 1);
        assert_ne!(
            FractalReferenceOrbitAnalyzer::state_signature(&base),
            FractalReferenceOrbitAnalyzer::state_signature(&changed)
        );
    }

    #[test]
    fn silenced_stack_is_rejected() {
        let params = StackParams {
            rates: [0, 0, 0, 0],
            ..StackParams::default()
        };
        assert!(Resolved::new(&params).is_err());
    }

    #[test]
    fn round_robin_matches_the_shaders_slot_order() {
        let params = StackParams {
            formulas: [1, 3, 0, 0],
            rates: [2, 1, 0, 0],
            ..StackParams::default()
        };
        let res = Resolved::new(&params).unwrap();
        assert_eq!(
            res.cycle,
            vec![Slot::Mandelbox, Slot::Mandelbox, Slot::Menger]
        );
    }

    /// A faithful `f64` transcription of the shader's own fold slots.
    ///
    /// Written from `stackDE` line by line rather than from `Resolved::step`, so
    /// that agreement between the two means something. The GPU comparison can
    /// only report that two images differ; this reports which iteration and
    /// which component, which is what was actually needed.
    #[allow(clippy::many_single_char_names)] // This helper is a line-for-line shader transcription.
    fn shader_step(slot: Slot, p: [f64; 3], seed: [f64; 3], k: &ShaderConsts) -> [f64; 3] {
        let (sc, l, min_r2, fix_r2, off) = (k.sc, k.fold_limit, k.min_r2, k.fix_r2, k.offset);
        match slot {
            Slot::Mandelbox => {
                let mut q = [0.0; 3];
                for i in 0..3 {
                    q[i] = p[i].clamp(-l, l) * 2.0 - p[i];
                }
                let r2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2];
                let m = if r2 < min_r2 {
                    fix_r2 / min_r2
                } else if r2 < fix_r2 {
                    fix_r2 / r2
                } else {
                    1.0
                };
                [
                    q[0] * m * sc + seed[0],
                    q[1] * m * sc + seed[1],
                    q[2] * m * sc + seed[2],
                ]
            }
            Slot::Menger => {
                let mut a = [p[0].abs(), p[1].abs(), p[2].abs()];
                if a[0] < a[1] {
                    a.swap(0, 1);
                }
                if a[0] < a[2] {
                    a.swap(0, 2);
                }
                if a[1] < a[2] {
                    a.swap(1, 2);
                }
                let mut o = [
                    a[0] * sc - off[0] * (sc - 1.0),
                    a[1] * sc - off[1] * (sc - 1.0),
                    a[2] * sc - off[2] * (sc - 1.0),
                ];
                if o[2] < -0.5 * off[2] * (sc - 1.0) {
                    o[2] += off[2] * (sc - 1.0);
                }
                o
            }
            Slot::CoCube => {
                let mut a = [p[0].abs(), p[1].abs(), p[2].abs()];
                if a[0] < a[1] {
                    a.swap(0, 1);
                }
                if a[1] < a[2] {
                    a.swap(1, 2);
                }
                a[2] = k.cocube - (a[2] - k.cocube).abs();
                [
                    a[0] * sc - off[0] * (sc - 1.0),
                    a[1] * sc - off[1] * (sc - 1.0),
                    a[2] * sc - off[2] * (sc - 1.0),
                ]
            }
            _ => p,
        }
    }

    struct ShaderConsts {
        sc: f64,
        fold_limit: f64,
        min_r2: f64,
        fix_r2: f64,
        offset: [f64; 3],
        cocube: f64,
    }

    fn consts_from(p: &StackParams) -> ShaderConsts {
        ShaderConsts {
            sc: p.scale,
            fold_limit: p.fold_limit,
            min_r2: p.min_radius * p.min_radius,
            fix_r2: p.fixed_radius * p.fixed_radius,
            offset: p.offset,
            cocube: p.cocube,
        }
    }

    #[test]
    fn each_fold_slot_matches_the_shaders_own_arithmetic() {
        let anchor = [0.35_f64, -0.21, 0.14];
        for (slot, formula) in [
            (Slot::Mandelbox, 1_i64),
            (Slot::Menger, 3),
            (Slot::CoCube, 9),
        ] {
            let params = StackParams {
                formulas: [formula, 0, 0, 0],
                rates: [1, 0, 0, 0],
                ..StackParams::default()
            };
            let res = Resolved::new(&params).unwrap();
            let k = consts_from(&params);

            let mut mine = [Dd::new(anchor[0]), Dd::new(anchor[1]), Dd::new(anchor[2])];
            let seed_dd = mine;
            let mut theirs = anchor;

            for n in 0..24 {
                mine = res.step(mine, seed_dd, n);
                theirs = shader_step(slot, theirs, anchor, &k);
                let gap = (0..3)
                    .map(|i| (mine[i].to_f64() - theirs[i]).abs())
                    .fold(0.0, f64::max);
                let scale = (0..3).map(|i| theirs[i].abs()).fold(1.0, f64::max);
                // The two differ only by double-double against f64, and that
                // difference is amplified by the derivative, so the tolerance
                // has to grow the same way. A fixed one fails at iteration
                // twelve on agreement to eleven significant digits. The rate is
                // per-formula: a scale-two fold alone doubles, and the
                // Mandelbox's sphere fold contributes about 1.6 on top of that,
                // which is the 0.4998 decades per iteration measured elsewhere.
                let rate: f64 = match slot {
                    Slot::Mandelbox => 3.4,
                    _ => 2.1,
                };
                let tol =
                    4.0 * f64::EPSILON * rate.powi(i32::try_from(n).unwrap_or(i32::MAX)) * scale;
                assert!(
                    gap <= tol.max(1e-14),
                    "{slot:?} diverged from the shader's arithmetic at iteration \
                     {n}: mine {:?} theirs {theirs:?} (gap {gap:.3e}, tol {tol:.3e})",
                    [mine[0].to_f64(), mine[1].to_f64(), mine[2].to_f64()],
                );
                if theirs.iter().any(|v| v.abs() > 1e6) {
                    break; // escaped; nothing further to compare
                }
            }
        }
    }

    #[test]
    fn newton_lengthens_the_orbit() {
        // The point of the analyzer: an unrefined anchor escapes early, and a
        // refined one survives. This is the shader's 38-fold ceiling in
        // miniature.
        let params = mandelbox_only();
        let res = Resolved::new(&params).unwrap();
        let start = [Dd::new(0.6), Dd::new(-0.42), Dd::new(0.31)];
        let (before, _) = res.orbit(start, 256);
        let (refined, _residual, _) = refine(&res, start, 2, 1, 48);
        let (after, _) = res.orbit(refined, 256);
        assert!(
            after.len() >= before.len(),
            "refinement shortened the orbit: {} -> {}",
            before.len(),
            after.len()
        );
    }

    /// The shader's `dzUnpackF32`, in Rust, so the contract is tested rather
    /// than assumed.
    ///
    /// `rgba8unorm` presents a byte to the shader as `b / 255`, which is what
    /// the division models; the shader recovers the byte with `round(v * 255)`.
    /// This is the whole interface between the analyzer and the shader, and it
    /// cannot be checked from the preview harness because that harness has no
    /// preprocessor wiring, so it is checked here.
    fn unpack_as_shader_does(texel: [u8; 4]) -> f32 {
        let norm = texel.map(|b| f32::from(b) / 255.0);
        let bytes = norm.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u32);
        let bits = bytes[0] | (bytes[1] << 8) | (bytes[2] << 16) | (bytes[3] << 24);
        f32::from_bits(bits)
    }

    fn unpack_texture(texture: &TextureData) -> Vec<f32> {
        texture
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| unpack_as_shader_does([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect()
    }

    fn first_record(texture: &TextureData) -> Vec<f32> {
        let floats = unpack_texture(texture);
        floats
            .get(
                HEADER_RECORDS * FLOATS_PER_ITER
                    ..(HEADER_RECORDS + 1) * FLOATS_PER_ITER,
            )
            .expect("first payload record")
            .to_vec()
    }

    fn assert_dd_row(row: &[f32], record: &StepRecord, pre: V3, formula: i64) {
        for component in 0..3 {
            assert_eq!(row[component].to_bits(), pre[component].to_f32().to_bits());
            assert_eq!(
                row[4 + component].to_bits(),
                record.mid[component].to_f32().to_bits()
            );
            assert_eq!(
                row[8 + component].to_bits(),
                record.mid2[component].to_f32().to_bits()
            );
            assert_eq!(
                row[12 + component].to_bits(),
                record.post[component].to_f32().to_bits()
            );
        }
        assert_eq!(row[3].to_bits(), v_dot(pre, pre).to_f32().to_bits());
        assert_eq!(
            row[7].to_bits(),
            v_dot(record.mid, record.mid).to_f32().to_bits()
        );
        assert_eq!(row[11].to_bits(), record.m_ref.to_f32().to_bits());
        assert_eq!(row[15], record.branches as f32);
        assert!(row[16..20].iter().all(|value| *value == 0.0));
        for (index, margin) in record.margins.iter().enumerate() {
            assert_eq!(
                row[20 + index].to_bits(),
                margin.to_f32().to_bits(),
                "formula {formula} margin {index}"
            );
        }
        assert_eq!(&row[32..36], &[record.branches as f32, 0.0, 0.0, 0.0]);
    }

    fn assert_ap_row(row: &[f32], record: &ApStepRecord, formula: i64) {
        for component in 0..3 {
            assert_eq!(
                row[component].to_bits(),
                ApMath::to_f32(&record.pre[component]).to_bits()
            );
            assert_eq!(
                row[4 + component].to_bits(),
                ApMath::to_f32(&record.mid[component]).to_bits()
            );
            assert_eq!(
                row[8 + component].to_bits(),
                ApMath::to_f32(&record.mid2[component]).to_bits()
            );
            assert_eq!(
                row[12 + component].to_bits(),
                ApMath::to_f32(&record.post[component]).to_bits()
            );
        }
        assert_eq!(row[3].to_bits(), ApMath::to_f32(&record.q_pre).to_bits());
        assert_eq!(row[7].to_bits(), ApMath::to_f32(&record.q_mid).to_bits());
        assert_eq!(row[11].to_bits(), ApMath::to_f32(&record.m_ref).to_bits());
        assert_eq!(row[15], record.branches as f32);
        let bulb = record.bulb.as_ref().expect("formula 5 bulb state");
        for (actual, expected) in
            row[16..20]
                .iter()
                .zip([&bulb.radius, &bulb.theta, &bulb.phi, &bulb.radius_power])
        {
            assert_eq!(actual.to_bits(), ApMath::to_f32(expected).to_bits());
        }
        for (index, margin) in record.margins.iter().enumerate() {
            assert_eq!(
                row[20 + index].to_bits(),
                ApMath::to_f32(margin).to_bits(),
                "formula {formula} margin {index}"
            );
        }
        assert_eq!(
            &row[32..36],
            &[
                record.branches as f32,
                bulb.seam_side as f32,
                bulb.principal_winding as f32,
                1.0,
            ]
        );
    }

    #[test]
    fn packed_floats_round_trip_exactly_through_rgba8unorm() {
        for v in [
            0.0_f32,
            1.0,
            -1.0,
            0.5,
            -0.123_456_79,
            1e-30,
            -1e30,
            std::f32::consts::PI,
            f32::MIN_POSITIVE,
        ] {
            let packed = v.to_bits().to_le_bytes();
            let back = unpack_as_shader_does(packed);
            assert_eq!(
                back.to_bits(),
                v.to_bits(),
                "round trip changed {v} to {back}"
            );
        }
    }

    #[test]
    fn first_orbit_entry_is_nonzero_so_the_shaders_presence_check_works() {
        // The shader treats an all-zero first entry as "no orbit bound" and
        // falls back rather than rendering with a reference pinned at the
        // origin. That guard is only sound if a real orbit never starts at
        // exactly zero.
        let mut a = FractalReferenceOrbitAnalyzer::new();
        a.init(&serde_json::json!({
            "formulas": [1, 0, 0, 0],
            "rates": [1, 0, 0, 0],
            "max_iters": 32,
            "refine": false,
            "anchor": ["0.31", "-0.17", "0.44"]
        }))
        .unwrap();
        let tex = a.orbit_texture().expect("texture");
        let first: Vec<f32> = (0..4)
            .map(|i| {
                let b = &tex.data[i * 4..i * 4 + 4];
                unpack_as_shader_does([b[0], b[1], b[2], b[3]])
            })
            .collect();
        assert!(
            first.iter().any(|v| *v != 0.0),
            "first orbit entry is all zero, which the shader reads as absent"
        );
    }

    /// The record has to be *self-consistent*: applying the shader's own fold to
    /// the transported `pre` must give the transported `mid`, and the radial
    /// multiplier must carry `mid` to `post`. If either fails, the shader is
    /// handed values it cannot reconcile, which is the failure this layout
    /// exists to remove.
    #[test]
    fn transported_intermediates_are_self_consistent() {
        let params = StackParams {
            formulas: [1, 0, 0, 0],
            rates: [1, 0, 0, 0],
            ..StackParams::default()
        };
        let resolved = Resolved::new(&params).unwrap();
        let k = consts_from(&params);
        let anchor = [Dd::new(0.35), Dd::new(-0.21), Dd::new(0.14)];
        let (records, _) = resolved.orbit_records(anchor, 24);

        let mut pre = anchor;
        for (n, rec) in records.iter().enumerate() {
            // `mid` must be the box fold of `pre`, by the shader's arithmetic.
            let folded: Vec<f64> = (0..3)
                .map(|i| pre[i].to_f64().clamp(-k.fold_limit, k.fold_limit) * 2.0 - pre[i].to_f64())
                .collect();
            for (i, folded_component) in folded.iter().enumerate() {
                let gap = (rec.mid[i].to_f64() - folded_component).abs();
                assert!(
                    gap < 1e-14,
                    "iteration {n}: transported mid disagrees with the fold of                      the transported pre on component {i} by {gap:.3e}"
                );
            }
            // `post` must be `mid * m_ref * sc + seed`.
            for (i, anchor_component) in anchor.iter().enumerate() {
                let expect =
                    rec.mid[i].to_f64() * rec.m_ref.to_f64() * k.sc + anchor_component.to_f64();
                let gap = (rec.post[i].to_f64() - expect).abs();
                assert!(
                    gap < 1e-13,
                    "iteration {n}: transported post disagrees with mid * m * sc                      + seed on component {i} by {gap:.3e}"
                );
            }
            pre = rec.post;
        }
    }

    #[test]
    fn orbit_texture_carries_a_full_record_per_iteration() {
        let mut a = FractalReferenceOrbitAnalyzer::new();
        let opts = serde_json::json!({
            "formulas": [1, 0, 0, 0],
            "rates": [1, 0, 0, 0],
            "max_iters": 64,
            "refine": false,
            "anchor": ["0.1", "0.2", "0.3"]
        });
        a.init(&opts).unwrap();
        let tex = a.orbit_texture().expect("texture");
        let needed = (64 + HEADER_RECORDS) * FLOATS_PER_ITER;
        // One rgba32float texel per four-float group.
        let texels = needed.div_ceil(4);
        assert_eq!(tex.width, texels.min(PAYLOAD_WIDTH) as u32);
        assert_eq!(
            tex.height,
            texels.div_ceil(texels.min(PAYLOAD_WIDTH)) as u32
        );
        assert_eq!(tex.format, "rgba32float");
        assert!(tex.data.len() >= needed * 4, "four f32 bytes per float");
    }

    #[test]
    fn payload_v6_packs_safe_slab_masks_in_header_groups_three_through_eleven() {
        use crate::internal::analyzer::fractal_certification::segment::{
            AtlasMetadata, CertifiedTile, TileStatus,
        };

        let mut analyzer = FractalReferenceOrbitAnalyzer::new();
        analyzer
            .init(&serde_json::json!({
                "formulas": [5, 0, 0, 0],
                "rates": [1, 0, 0, 0],
                "max_iters": 2,
                "refine": false,
                "anchor": ["0.31", "-0.17", "0.23"]
            }))
            .unwrap();
        let tiles = (0..ATLAS_COLUMNS * ATLAS_ROWS)
            .map(|index| CertifiedTile {
                column: index % ATLAS_COLUMNS,
                row: index / ATLAS_COLUMNS,
                screen_bounds: [0.0; 4],
                safe_slab_mask: 1_u32 << (index % 24),
                status: TileStatus::Certified {
                    safe_prefix: (index + 1) as f32 * 0.125,
                    certified_slabs: 1,
                    last_event: crate::internal::analyzer::fractal_certification::segment::TileEscapeEvent::MandelbulbPreGuard {
                        iteration: 1,
                    },
                },
            })
            .collect();
        analyzer.segment_atlas = Some(SegmentAtlasResult::Certified {
            metadata: AtlasMetadata {
                columns: ATLAS_COLUMNS,
                rows: ATLAS_ROWS,
                t_slabs: 24,
                max_frame_t: 24.0,
            },
            tiles,
        });

        let texture = analyzer.orbit_texture().expect("payload texture");
        let header = &unpack_texture(&texture)[..FLOATS_PER_ITER];
        assert_eq!(header[0], PAYLOAD_VERSION as f32);
        for index in 0..ATLAS_COLUMNS * ATLAS_ROWS {
            assert_eq!(header[12 + index], (1_u32 << (index % 24)) as f32);
        }
        assert_eq!(
            &header[44..48],
            &[ATLAS_COLUMNS as f32, ATLAS_ROWS as f32, 24.0, 2.0]
        );
    }

    #[test]
    fn native_dd_margins_keep_representable_boundary_signs() {
        let params = mandelbox_only();
        let resolved = Resolved::new(&params).unwrap();
        let epsilon = Dd::new(1.0e-40);
        let point = [
            Dd::new(params.fold_limit).add(epsilon),
            Dd::new(0.0),
            Dd::new(0.0),
        ];
        let record = resolved.step_detailed(point, point, 0);

        assert_eq!(record.margins[1].cmp(Dd::new(0.0)), Ordering::Less);
        assert!(record.margins[1].to_f32() < 0.0);
        assert_eq!(record.margins[7].cmp(Dd::new(0.0)), Ordering::Greater);
        assert!(record.margins[7].to_f32() > 0.0);
        assert_eq!(record.branches % 3, 2);
    }

    #[test]
    fn native_ap_margins_keep_representable_boundary_signs() {
        let params = mandelbox_only();
        let mut math = ApMath::for_zoom(120.0);
        let resolved = ApResolved::new(&params, &mut math).unwrap();
        let point = [
            math.decimal("1.0000000000000000000000000000000000000001")
                .unwrap(),
            math.zero(),
            math.zero(),
        ];
        let record = resolved.step(&mut math, &point, &point, 0);

        assert_eq!(
            ApMath::branch(&record.margins[1], &math.zero()),
            Ordering::Less
        );
        assert!(ApMath::to_f32(&record.margins[1]) < 0.0);
        assert_eq!(
            ApMath::branch(&record.margins[7], &math.zero()),
            Ordering::Greater
        );
        assert!(ApMath::to_f32(&record.margins[7]) > 0.0);
        assert_eq!(record.branches % 3, 2);
    }

    #[test]
    fn pseudo_kleinian_has_only_inner_and_inversion_radial_codes() {
        let params = StackParams {
            formulas: [6, 0, 0, 0],
            rates: [1, 0, 0, 0],
            min_radius: 0.5,
            ..StackParams::default()
        };
        let mut math = ApMath::for_zoom(80.0);
        let resolved = ApResolved::new(&params, &mut math).unwrap();

        for (coordinate, expected_radial) in [("0.1", 0), ("0.5", 1), ("3.0", 1)] {
            let point = [math.decimal(coordinate).unwrap(), math.zero(), math.zero()];
            let record = resolved.step(&mut math, &point, &point, 0);
            assert_eq!(record.branches / 27, expected_radial);
            assert_ne!(record.branches / 27, 2);
        }
    }

    #[test]
    fn arbitrary_record_branches_are_the_payload_source_of_truth() {
        let params = StackParams {
            formulas: [5, 0, 0, 0],
            rates: [1, 0, 0, 0],
            refine: false,
            max_iters: 1,
            anchor: Some(["-0.5".into(), "1e-40".into(), "0.2".into()]),
            ..StackParams::default()
        };
        let mut records = FractalReferenceOrbitAnalyzer::generate_arbitrary_records(&params)
            .unwrap()
            .records;
        records[0].branches = 73;
        let analyzer = FractalReferenceOrbitAnalyzer {
            params,
            arbitrary_records: Some(records),
            ..FractalReferenceOrbitAnalyzer::new()
        };
        let texture = analyzer.orbit_texture().unwrap();
        let row = first_record(&texture);

        assert_eq!(row[3 * 4 + 3], 73.0, "group 3 branch code");
        assert_eq!(row[8 * 4], 73.0, "group 8 authoritative branch code");
        assert_eq!(row[8 * 4 + 1], 1.0, "group 8 bulb seam side");
        assert_eq!(row[8 * 4 + 2], 0.0, "group 8 principal winding");
        assert_eq!(row[8 * 4 + 3], 1.0, "group 8 bulb-state flag");
    }

    #[test]
    fn state_signature_and_payload_target_digest_cover_distinct_identities() {
        let base = StackParams::default();
        let mut removed_controls = AnalyzerStateSnapshot::default();
        for (name, value) in [
            ("cam_dist", 9.0),
            ("cam_azim", 1.0),
            ("cam_elev", -0.5),
            ("fov", 1.6),
            ("sway_amount", 0.8),
            ("look_x", 0.7),
            ("look_y", -0.4),
            ("look_z", 0.9),
            ("evolve_amount", 0.25),
            ("evolve_phase", std::f64::consts::FRAC_PI_2),
            ("anchor_x", 0.1),
            ("anchor_y", 0.2),
            ("anchor_z", 0.3),
        ] {
            removed_controls
                .values
                .insert(name.into(), crate::params::ParamValue::Float(value as f32));
        }
        assert_eq!(
            FractalReferenceOrbitAnalyzer::state_signature(&base),
            FractalReferenceOrbitAnalyzer::state_signature(
                &base.with_live_state(&removed_controls)
            )
        );
        let mut state = base.clone();
        state.formulas.swap(0, 1);
        assert_ne!(
            FractalReferenceOrbitAnalyzer::state_signature(&base),
            FractalReferenceOrbitAnalyzer::state_signature(&state)
        );

        let mut same_effective = base.clone();
        same_effective.anchor = Some(["0.1250000001".into(), "0".into(), "0".into()]);
        let mut rounded = base.clone();
        rounded.anchor = Some(["0.125".into(), "0".into(), "0".into()]);
        assert_eq!(
            FractalReferenceOrbitAnalyzer::state_signature(&same_effective),
            FractalReferenceOrbitAnalyzer::state_signature(&rounded),
            "target identity belongs to the payload digest, not live f32 controls"
        );
        let precise_records =
            FractalReferenceOrbitAnalyzer::generate_arbitrary_records(&StackParams {
                formulas: [5, 0, 0, 0],
                rates: [1, 0, 0, 0],
                refine: false,
                max_iters: 1,
                anchor: same_effective.anchor,
                ..StackParams::default()
            })
            .unwrap()
            .records;
        assert_ne!(
            precise_records[0].pre[0],
            Hp::try_from(0.125_f64)
                .unwrap()
                .with_precision(precise_records[0].pre[0].precision())
                .value(),
            "orbit generation must retain the exact decimal anchor"
        );
        let precise = FractalReferenceOrbitAnalyzer::target_digest(
            [0.125_f64, 0.0, 0.0],
            [1e-10, 0.0, 0.0],
            1,
        );
        let rounded = FractalReferenceOrbitAnalyzer::target_digest([0.125, 0.0, 0.0], [0.0; 3], 1);
        assert_ne!(
            precise, rounded,
            "sub-f32 target changes must alter the shader-reproducible payload digest"
        );
    }

    #[test]
    fn full_payload_row_layout_and_bailout_margin_cover_all_slots() {
        for formula in 1..=10 {
            let mut analyzer = FractalReferenceOrbitAnalyzer::new();
            analyzer
                .init(&serde_json::json!({
                    "formulas": [formula, 0, 0, 0],
                    "rates": [1, 0, 0, 0],
                    "max_iters": 1,
                    "refine": false,
                    "anchor": ["0.31", "-0.17", "0.23"]
                }))
                .unwrap();
            let texture = analyzer.orbit_texture().unwrap();
            let row = first_record(&texture);
            assert_eq!(row.len(), FLOATS_PER_ITER);
            if let Some(records) = analyzer.arbitrary_records.as_ref() {
                assert_ap_row(&row, &records[0], formula);
            } else {
                let (records, _) = analyzer
                    .resolved
                    .as_ref()
                    .unwrap()
                    .orbit_records(analyzer.anchor, 1);
                assert_dd_row(&row, &records[0], analyzer.anchor, formula);
            }
            assert_eq!(
                row[8 * 4 + 3],
                if formula == 5 { 1.0 } else { 0.0 },
                "group 8 state flag for formula {formula}"
            );
            // Groups nine through eleven carry the parameter Jacobian since
            // payload v7: a unit matrix in the first three components of each
            // group, the base-two magnitude in group nine's fourth, and the
            // two remaining components reserved. The double-double backend
            // publishes no Jacobian, so its rows stay zero; what must hold in
            // both cases is that the reserved components are untouched.
            assert_eq!(row[10 * 4 + 3], 0.0, "group 10 reserved component");
            assert_eq!(row[11 * 4 + 3], 0.0, "group 11 reserved component");
            for entry in 0..3 {
                assert!(
                    row[9 * 4 + entry].abs() <= 1.0,
                    "Jacobian row is normalised, got {}",
                    row[9 * 4 + entry]
                );
            }
        }
    }
}
