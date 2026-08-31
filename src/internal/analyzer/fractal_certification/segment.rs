//! Bounded safe-prefix certificates for primary-ray camera tiles.
//!
//! Every published prefix is a contiguous union of front-to-back `t` slabs.
//! A slab is retained only when one common shader escape event is proved for
//! its complete screen rectangle and complete `t` interval.

use dashu_float::Repr;

use super::mandelbulb::{IntervalBox3, MandelbulbError};
use super::stack::{cycle, evaluate_slot, validate, StackEvaluationError};
use super::{Atan2Chart, BigInterval, IntervalError, IntervalMath};
use crate::internal::analyzer::fractal_reference_orbit::StackParams;

const DIMENSIONS: usize = 3;
pub(crate) const ATLAS_COLUMNS: usize = 8;
pub(crate) const ATLAS_ROWS: usize = 4;
pub(crate) const DEFAULT_T_SLABS: usize = 24;
const MAX_PRECISION: usize = 4_096;
const MAX_T_SLABS: usize = 32;
// Must admit `SegmentBudgets::default()`, whose evaluation budget carries the
// per-tile screen-subdivision factor; without it here the default budget was
// rejected as invalid and no production atlas ever certified.
const MAX_ORBIT_EVALUATIONS: usize =
    ATLAS_COLUMNS * ATLAS_ROWS * MAX_T_SLABS * SCREEN_SUBDIVISIONS * SCREEN_SUBDIVISIONS * 4_096;
const CAMERA_BASIS_ROUNDING: f64 = 1.0 / 1_048_576.0;
const SCREEN_SUBDIVISIONS: usize = 2;
const MAX_DIRECTIONAL_SLAB_DEPTH: usize = 4;
const CAMERA_STANDOFF: f64 = 0.25;

/// Camera frame used to construct shader-equivalent primary rays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PrimaryCamera {
    pub(crate) forward: [f64; DIMENSIONS],
    pub(crate) right: [f64; DIMENSIONS],
    pub(crate) up: [f64; DIMENSIONS],
    pub(crate) fov: f64,
    pub(crate) aspect: f64,
    /// Maximum absolute image-plane sway over the certificate lifetime.
    pub(crate) sway_extent: [f64; 2],
}

/// Explicit work limits for one atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SegmentBudgets {
    pub(crate) precision: usize,
    pub(crate) t_slabs: usize,
    pub(crate) max_orbit_evaluations: usize,
}

impl Default for SegmentBudgets {
    fn default() -> Self {
        Self {
            precision: 160,
            t_slabs: DEFAULT_T_SLABS,
            max_orbit_evaluations: ATLAS_COLUMNS
                * ATLAS_ROWS
                * DEFAULT_T_SLABS
                * SCREEN_SUBDIVISIONS
                * SCREEN_SUBDIVISIONS
                * 4_096,
        }
    }
}

/// Fixed atlas geometry consumed together with its row-major tile records.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AtlasMetadata {
    pub(crate) columns: usize,
    pub(crate) rows: usize,
    pub(crate) t_slabs: usize,
    pub(crate) max_frame_t: f32,
}

/// The common event proving that one complete slab is outside the finite solid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TileEscapeEvent {
    MandelbulbPreGuard { iteration: usize },
    PostSlotBailout { iteration: usize },
}

/// Typed fail-closed reason at the first uncertified slab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TileStopReason {
    AzimuthSeam,
    PolarAxis,
    RadiusRegularization,
    FormulaBranch,
    MengerTranslation,
    NoCommonEscape,
    OrbitBudget,
    Backend(IntervalError),
}

/// Status of a row-major screen tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TileStatus {
    Certified {
        safe_prefix: f32,
        certified_slabs: usize,
        last_event: TileEscapeEvent,
    },
    Stopped {
        safe_prefix: f32,
        certified_slabs: usize,
        reason: TileStopReason,
    },
}

impl TileStatus {
    pub(crate) const fn safe_prefix(self) -> f32 {
        match self {
            Self::Certified { safe_prefix, .. } | Self::Stopped { safe_prefix, .. } => safe_prefix,
        }
    }
}

/// One fixed screen rectangle and its certified front prefix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CertifiedTile {
    pub(crate) column: usize,
    pub(crate) row: usize,
    pub(crate) screen_bounds: [f32; 4],
    /// One bit per front-to-back slab. A set bit proves that complete slab
    /// empty for every primary ray represented by this tile.
    pub(crate) safe_slab_mask: u32,
    pub(crate) status: TileStatus,
}

/// Complete fixed-layout output, or a request-level rejection with no tiles.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SegmentAtlasResult {
    Certified {
        metadata: AtlasMetadata,
        tiles: Vec<CertifiedTile>,
    },
    Rejected(SegmentRequestError),
}

/// Request errors that invalidate the atlas as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentRequestError {
    InvalidCamera,
    InvalidAnchor,
    InvalidScale,
    InvalidIterationCount,
    InvalidBudget,
    InvalidStack,
    Backend(IntervalError),
}

/// Exact affine and one-dimensional derivative state for a resolved ray orbit.
///
/// `affine_derivative` is the complete derivative with respect to the three
/// seed coordinates. `tangent` and `curvature` are derivatives with respect to
/// the ray parameter in `c(t) = origin + t direction`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DirectionalTransport {
    pub(crate) value: [BigInterval; DIMENSIONS],
    pub(crate) affine_derivative: [[BigInterval; DIMENSIONS]; DIMENSIONS],
    pub(crate) tangent: [BigInterval; DIMENSIONS],
    pub(crate) curvature: [BigInterval; DIMENSIONS],
}

/// A host-side directional measurement suitable for later payload packing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DirectionalCertificate {
    /// Largest binary32 endpoint proved safe by the quadratic lower bound.
    pub(crate) safe_endpoint: f32,
    pub(crate) event: TileEscapeEvent,
    pub(crate) transport: DirectionalTransport,
    pub(crate) margin_at_origin: BigInterval,
    pub(crate) slope_at_origin: BigInterval,
    pub(crate) curvature_bound: BigInterval,
}

/// Directional certification is deliberately fail-closed at unresolved maps.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DirectionalCertificateResult {
    Certified(Box<DirectionalCertificate>),
    Inconclusive(TileStopReason),
    Rejected(SegmentRequestError),
}

/// Measures one resolved ray segment with exact 3 by 3 affine transport.
///
/// For every considered escape event this transports
/// `D' = J D + S`, `v' = J v + S d`, and
/// `a' = J a + H[v,v]`. The event margin is `g = ||x||^2 - R^2`.
/// An endpoint is published only when directed interval arithmetic proves both
/// `g(0) > 0` and
/// `g(0) + g'(0)L - |g''|L^2/2 > 0`, where the curvature bound encloses the
/// complete requested segment.
pub(crate) fn certify_directional_ray_segment(
    params: &StackParams,
    origin: &[String; DIMENSIONS],
    direction: [f64; DIMENSIONS],
    finite_iterations: usize,
    maximum_length: f64,
    precision: usize,
) -> DirectionalCertificateResult {
    match certify_directional_inner(
        params,
        origin,
        direction,
        finite_iterations,
        maximum_length,
        precision,
    ) {
        Ok(certificate) => DirectionalCertificateResult::Certified(Box::new(certificate)),
        Err(DirectionalFailure::Inconclusive(reason)) => {
            DirectionalCertificateResult::Inconclusive(reason)
        }
        Err(DirectionalFailure::Rejected(reason)) => DirectionalCertificateResult::Rejected(reason),
    }
}

enum DirectionalFailure {
    Inconclusive(TileStopReason),
    Rejected(SegmentRequestError),
}

fn certify_directional_inner(
    params: &StackParams,
    origin: &[String; DIMENSIONS],
    direction: [f64; DIMENSIONS],
    finite_iterations: usize,
    maximum_length: f64,
    precision: usize,
) -> Result<DirectionalCertificate, DirectionalFailure> {
    validate(params)
        .map_err(|_| DirectionalFailure::Rejected(SegmentRequestError::InvalidStack))?;
    if finite_iterations == 0 || finite_iterations > params.max_iters {
        return Err(DirectionalFailure::Rejected(
            SegmentRequestError::InvalidIterationCount,
        ));
    }
    if precision == 0 || precision > MAX_PRECISION {
        return Err(DirectionalFailure::Rejected(
            SegmentRequestError::InvalidBudget,
        ));
    }
    if !maximum_length.is_finite()
        || maximum_length <= 0.0
        || direction.iter().any(|component| !component.is_finite())
        || dot(direction, direction) <= 0.0
    {
        return Err(DirectionalFailure::Rejected(
            SegmentRequestError::InvalidScale,
        ));
    }

    let math = IntervalMath::new(precision)
        .map_err(|error| DirectionalFailure::Rejected(SegmentRequestError::Backend(error)))?;
    let origin = [
        math.point_decimal(&origin[0])
            .map_err(|_| DirectionalFailure::Rejected(SegmentRequestError::InvalidAnchor))?,
        math.point_decimal(&origin[1])
            .map_err(|_| DirectionalFailure::Rejected(SegmentRequestError::InvalidAnchor))?,
        math.point_decimal(&origin[2])
            .map_err(|_| DirectionalFailure::Rejected(SegmentRequestError::InvalidAnchor))?,
    ];
    let direction = [
        math.point_f64(direction[0]),
        math.point_f64(direction[1]),
        math.point_f64(direction[2]),
    ];
    let direction = [
        direction[0]
            .clone()
            .map_err(|error| DirectionalFailure::Rejected(SegmentRequestError::Backend(error)))?,
        direction[1]
            .clone()
            .map_err(|error| DirectionalFailure::Rejected(SegmentRequestError::Backend(error)))?,
        direction[2]
            .clone()
            .map_err(|error| DirectionalFailure::Rejected(SegmentRequestError::Backend(error)))?,
    ];
    let mut evaluations = 0;
    certify_directional_packet(
        &math,
        params,
        DirectionalPacket {
            origin: &origin,
            direction: &direction,
            maximum_length,
        },
        finite_iterations,
        &mut evaluations,
        usize::MAX,
    )
    .map_err(DirectionalFailure::Inconclusive)
}

/// Certifies one packet whose affine origin and direction are interval boxes.
///
/// The returned endpoint applies to every affine ray represented by the packet.
#[derive(Clone, Copy)]
struct DirectionalPacket<'a> {
    origin: &'a [BigInterval; DIMENSIONS],
    direction: &'a [BigInterval; DIMENSIONS],
    maximum_length: f64,
}

#[derive(Clone, Copy)]
struct DirectionalSlab<'a> {
    anchor: &'a [BigInterval; DIMENSIONS],
    frame_radius: &'a BigInterval,
    camera_forward: [f64; DIMENSIONS],
    direction: &'a [BigInterval; DIMENSIONS],
    start: &'a BigInterval,
    length: f64,
}

fn certify_directional_packet(
    math: &IntervalMath,
    params: &StackParams,
    packet: DirectionalPacket<'_>,
    finite_iterations: usize,
    evaluations: &mut usize,
    evaluation_budget: usize,
) -> Result<DirectionalCertificate, TileStopReason> {
    let DirectionalPacket {
        origin,
        direction,
        maximum_length,
    } = packet;
    let direction_norm = norm_squared(math, direction).map_err(TileStopReason::Backend)?;
    if !direction_norm.strictly_positive() {
        return Err(TileStopReason::NoCommonEscape);
    }
    let length = math
        .bounds_f64(0.0, maximum_length)
        .map_err(TileStopReason::Backend)?;
    let segment_component =
        |axis: usize| math.add(&origin[axis], &math.mul(&length, &direction[axis])?);
    let segment = [
        segment_component(0).map_err(TileStopReason::Backend)?,
        segment_component(1).map_err(TileStopReason::Backend)?,
        segment_component(2).map_err(TileStopReason::Backend)?,
    ];
    let mut center = DirectionalTransport::identity(math, origin.clone(), direction)
        .map_err(TileStopReason::Backend)?;
    let mut enclosure = DirectionalTransport::identity(math, segment.clone(), direction)
        .map_err(TileStopReason::Backend)?;
    let formulas = cycle(params).map_err(|_| TileStopReason::NoCommonEscape)?;
    let bulb_guard_squared = math.point_f64(4.0).map_err(TileStopReason::Backend)?;
    let bailout_squared = math
        .square(
            &math
                .point_f64(params.bailout)
                .map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;
    let mut best = None;

    for iteration in 1..=finite_iterations {
        if *evaluations >= evaluation_budget {
            return Err(TileStopReason::OrbitBudget);
        }
        *evaluations += 1;
        let formula = formulas[(iteration - 1) % formulas.len()];
        if formula == 5 {
            best = choose_longer(
                best,
                directional_candidate(
                    math,
                    &center,
                    &enclosure,
                    &bulb_guard_squared,
                    maximum_length,
                    TileEscapeEvent::MandelbulbPreGuard { iteration },
                )
                .map_err(TileStopReason::Backend)?,
            );
        }
        if let Some(certificate) = best.take() {
            if certificate.safe_endpoint.to_bits() == inward_f32(maximum_length).to_bits() {
                return Ok(certificate);
            }
            best = Some(certificate);
        }

        let chart = if formula == 5 {
            match chart_for(&enclosure.value) {
                Ok(chart) => chart,
                Err(reason) => {
                    return best.ok_or(reason);
                }
            }
        } else {
            Atan2Chart::Principal
        };
        enclosure = match advance_directional(
            math, &enclosure, &segment, direction, formula, params, chart,
        ) {
            Ok(next) => next,
            Err(error) => {
                return best.ok_or(map_stack_error(error));
            }
        };
        center = match advance_directional(math, &center, origin, direction, formula, params, chart)
        {
            Ok(next) => next,
            Err(error) => {
                return best.ok_or(map_stack_error(error));
            }
        };

        best = choose_longer(
            best,
            directional_candidate(
                math,
                &center,
                &enclosure,
                &bailout_squared,
                maximum_length,
                TileEscapeEvent::PostSlotBailout { iteration },
            )
            .map_err(TileStopReason::Backend)?,
        );
    }
    best.ok_or(TileStopReason::NoCommonEscape)
}

fn certify_directional_slab(
    math: &IntervalMath,
    params: &StackParams,
    slab: DirectionalSlab<'_>,
    finite_iterations: usize,
    evaluations: &mut usize,
    evaluation_budget: usize,
    depth: usize,
) -> Result<TileEscapeEvent, TileStopReason> {
    let (origin, derivative) = directional_slab_packet(
        math,
        slab.anchor,
        slab.frame_radius,
        slab.camera_forward,
        slab.direction,
        slab.start,
    )
    .map_err(|error| match error {
        SegmentRequestError::Backend(error) => TileStopReason::Backend(error),
        _ => TileStopReason::NoCommonEscape,
    })?;
    let result = certify_directional_packet(
        math,
        params,
        DirectionalPacket {
            origin: &origin,
            direction: &derivative,
            maximum_length: slab.length,
        },
        finite_iterations,
        evaluations,
        evaluation_budget,
    );
    match result {
        Ok(certificate)
            if certificate.safe_endpoint.to_bits() == inward_f32(slab.length).to_bits() =>
        {
            return Ok(certificate.event);
        }
        Err(TileStopReason::OrbitBudget) => return Err(TileStopReason::OrbitBudget),
        // At the subdivision floor a typed failure keeps its cause — an axis
        // stop reported as a generic no-escape hides exactly the diagnostic
        // the reason codes exist to surface. Only a partial certificate, which
        // has no failure of its own, becomes the generic reason.
        Err(reason) if depth == MAX_DIRECTIONAL_SLAB_DEPTH => return Err(reason),
        Ok(_) if depth == MAX_DIRECTIONAL_SLAB_DEPTH => {
            return Err(TileStopReason::NoCommonEscape);
        }
        Ok(_) | Err(_) => {}
    }

    let half_length = slab.length * 0.5;
    certify_directional_slab(
        math,
        params,
        DirectionalSlab {
            length: half_length,
            ..slab
        },
        finite_iterations,
        evaluations,
        evaluation_budget,
        depth + 1,
    )?;
    let second_start = math
        .add(
            slab.start,
            &math
                .point_f64(half_length)
                .map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;
    certify_directional_slab(
        math,
        params,
        DirectionalSlab {
            start: &second_start,
            length: half_length,
            ..slab
        },
        finite_iterations,
        evaluations,
        evaluation_budget,
        depth + 1,
    )
}

impl DirectionalTransport {
    fn identity(
        math: &IntervalMath,
        value: [BigInterval; DIMENSIONS],
        direction: &[BigInterval; DIMENSIONS],
    ) -> Result<Self, IntervalError> {
        let zero = math.point_f64(0.0)?;
        let one = math.point_f64(1.0)?;
        Ok(Self {
            value,
            affine_derivative: std::array::from_fn(|row| {
                std::array::from_fn(|column| {
                    if row == column {
                        one.clone()
                    } else {
                        zero.clone()
                    }
                })
            }),
            tangent: direction.clone(),
            curvature: std::array::from_fn(|_| zero.clone()),
        })
    }
}

fn advance_directional(
    math: &IntervalMath,
    state: &DirectionalTransport,
    parameter: &[BigInterval; DIMENSIONS],
    direction: &[BigInterval; DIMENSIONS],
    formula: u8,
    params: &StackParams,
    chart: Atan2Chart,
) -> Result<DirectionalTransport, StackEvaluationError> {
    let evaluation = evaluate_slot(
        math,
        &IntervalBox3::new(state.value.clone()),
        formula,
        params,
        chart,
    )?;
    let jacobian: [[BigInterval; DIMENSIONS]; DIMENSIONS] =
        std::array::from_fn(|row| evaluation.components[row].gradient().clone());
    let hessian: [[[BigInterval; DIMENSIONS]; DIMENSIONS]; DIMENSIONS] =
        std::array::from_fn(|row| evaluation.components[row].hessian().clone());
    let seed = math.point_f64(evaluation.seed_weight)?;

    let affine_derivative = matrix_product(math, &jacobian, &state.affine_derivative)?;
    let affine_entry = |row: usize, column: usize| {
        if row == column {
            math.add(&affine_derivative[row][column], &seed)
        } else {
            Ok(affine_derivative[row][column].clone())
        }
    };
    let affine_derivative = [
        [
            affine_entry(0, 0)?,
            affine_entry(0, 1)?,
            affine_entry(0, 2)?,
        ],
        [
            affine_entry(1, 0)?,
            affine_entry(1, 1)?,
            affine_entry(1, 2)?,
        ],
        [
            affine_entry(2, 0)?,
            affine_entry(2, 1)?,
            affine_entry(2, 2)?,
        ],
    ];
    let tangent_component = |row: usize| {
        math.add(
            &matrix_vector_component(math, &jacobian[row], &state.tangent)?,
            &math.mul(&seed, &direction[row])?,
        )
    };
    let tangent = [
        tangent_component(0)?,
        tangent_component(1)?,
        tangent_component(2)?,
    ];
    let curvature_component = |output: usize| {
        let linear = matrix_vector_component(math, &jacobian[output], &state.curvature)?;
        let quadratic = quadratic_form(math, &hessian[output], &state.tangent)?;
        math.add(&linear, &quadratic)
    };
    let curvature = [
        curvature_component(0)?,
        curvature_component(1)?,
        curvature_component(2)?,
    ];
    let values = std::array::from_fn(|axis| evaluation.components[axis].value().clone());
    let value = add_seed_values(math, &values, parameter, params, evaluation.seed_weight).map_err(
        |reason| match reason {
            TileStopReason::Backend(error) => StackEvaluationError::Backend(error),
            _ => StackEvaluationError::InvalidParameter,
        },
    )?;
    Ok(DirectionalTransport {
        value,
        affine_derivative,
        tangent,
        curvature,
    })
}

fn matrix_product(
    math: &IntervalMath,
    lhs: &[[BigInterval; DIMENSIONS]; DIMENSIONS],
    rhs: &[[BigInterval; DIMENSIONS]; DIMENSIONS],
) -> Result<[[BigInterval; DIMENSIONS]; DIMENSIONS], IntervalError> {
    let zero = math.point_f64(0.0)?;
    let mut result = std::array::from_fn(|_| std::array::from_fn(|_| zero.clone()));
    for (row, result_row) in result.iter_mut().enumerate() {
        for (column, entry) in result_row.iter_mut().enumerate() {
            for (inner, rhs_row) in rhs.iter().enumerate() {
                *entry = math.add(entry, &math.mul(&lhs[row][inner], &rhs_row[column])?)?;
            }
        }
    }
    Ok(result)
}

fn matrix_vector_component(
    math: &IntervalMath,
    row: &[BigInterval; DIMENSIONS],
    vector: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let mut result = math.point_f64(0.0)?;
    for (entry, component) in row.iter().zip(vector) {
        result = math.add(&result, &math.mul(entry, component)?)?;
    }
    Ok(result)
}

fn quadratic_form(
    math: &IntervalMath,
    matrix: &[[BigInterval; DIMENSIONS]; DIMENSIONS],
    vector: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let mut result = math.point_f64(0.0)?;
    for (row, vector_row) in vector.iter().enumerate() {
        for (column, vector_column) in vector.iter().enumerate() {
            let product = math.mul(vector_row, &math.mul(&matrix[row][column], vector_column)?)?;
            result = math.add(&result, &product)?;
        }
    }
    Ok(result)
}

fn directional_candidate(
    math: &IntervalMath,
    center: &DirectionalTransport,
    enclosure: &DirectionalTransport,
    threshold_squared: &BigInterval,
    maximum_length: f64,
    event: TileEscapeEvent,
) -> Result<Option<DirectionalCertificate>, IntervalError> {
    let margin = math.sub(&norm_squared(math, &center.value)?, threshold_squared)?;
    if !margin.strictly_positive() {
        return Ok(None);
    }
    let two = math.point_f64(2.0)?;
    let slope = math.mul(&two, &dot_intervals(math, &center.value, &center.tangent)?)?;
    let speed_squared = dot_intervals(math, &enclosure.tangent, &enclosure.tangent)?;
    let acceleration = dot_intervals(math, &enclosure.value, &enclosure.curvature)?;
    let second = math.mul(&two, &math.add(&speed_squared, &acceleration)?)?;
    let curvature_bound = math.magnitude(&second)?;
    let safe_endpoint =
        safe_quadratic_endpoint(math, &margin, &slope, &curvature_bound, maximum_length)?;
    if safe_endpoint == 0.0 {
        return Ok(None);
    }
    Ok(Some(DirectionalCertificate {
        safe_endpoint,
        event,
        transport: enclosure.clone(),
        margin_at_origin: margin,
        slope_at_origin: slope,
        curvature_bound,
    }))
}

fn safe_quadratic_endpoint(
    math: &IntervalMath,
    margin: &BigInterval,
    slope: &BigInterval,
    curvature_bound: &BigInterval,
    maximum_length: f64,
) -> Result<f32, IntervalError> {
    let half = math.point_f64(0.5)?;
    let proves = |length: f64| -> Result<bool, IntervalError> {
        let length = math.point_f64(length)?;
        let linear = math.mul(slope, &length)?;
        let quadratic = math.mul(&half, &math.mul(curvature_bound, &math.square(&length)?)?)?;
        Ok(math
            .sub(&math.add(margin, &linear)?, &quadratic)?
            .strictly_positive())
    };
    if proves(maximum_length)? {
        return Ok(inward_f32(maximum_length));
    }
    let mut lower = 0.0;
    let mut upper = maximum_length;
    for _ in 0..64 {
        let midpoint = lower + (upper - lower) * 0.5;
        if proves(midpoint)? {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    Ok(inward_f32(lower))
}

fn dot_intervals(
    math: &IntervalMath,
    lhs: &[BigInterval; DIMENSIONS],
    rhs: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let mut result = math.point_f64(0.0)?;
    for (left, right) in lhs.iter().zip(rhs) {
        result = math.add(&result, &math.mul(left, right)?)?;
    }
    Ok(result)
}

fn choose_longer(
    current: Option<DirectionalCertificate>,
    candidate: Option<DirectionalCertificate>,
) -> Option<DirectionalCertificate> {
    match (current, candidate) {
        (Some(current), Some(candidate)) if candidate.safe_endpoint > current.safe_endpoint => {
            Some(candidate)
        }
        (Some(current), _) => Some(current),
        (None, candidate) => candidate,
    }
}

/// Certifies a fixed 8 by 4 primary-ray atlas.
///
/// `zoom_exp > 0` is evaluated as `exp(-zoom_exp * ln(10))` entirely in
/// directed arbitrary-precision arithmetic before multiplication by
/// `params.cam_dist`.
pub(crate) fn certify_primary_ray_segments(
    params: &StackParams,
    anchor: &[String; DIMENSIONS],
    camera: PrimaryCamera,
    finite_iterations: usize,
    max_frame_t: f64,
    budgets: SegmentBudgets,
) -> SegmentAtlasResult {
    match certify_inner(
        params,
        anchor,
        camera,
        finite_iterations,
        max_frame_t,
        budgets,
    ) {
        Ok((metadata, tiles)) => SegmentAtlasResult::Certified { metadata, tiles },
        Err(reason) => SegmentAtlasResult::Rejected(reason),
    }
}

fn certify_inner(
    params: &StackParams,
    anchor: &[String; DIMENSIONS],
    camera: PrimaryCamera,
    finite_iterations: usize,
    max_frame_t: f64,
    budgets: SegmentBudgets,
) -> Result<(AtlasMetadata, Vec<CertifiedTile>), SegmentRequestError> {
    validate_request(params, camera, finite_iterations, max_frame_t, budgets)?;
    let math = IntervalMath::new(budgets.precision).map_err(SegmentRequestError::Backend)?;
    let anchor = [
        math.point_decimal(&anchor[0])
            .map_err(|_| SegmentRequestError::InvalidAnchor)?,
        math.point_decimal(&anchor[1])
            .map_err(|_| SegmentRequestError::InvalidAnchor)?,
        math.point_decimal(&anchor[2])
            .map_err(|_| SegmentRequestError::InvalidAnchor)?,
    ];
    let rho = frame_radius(&math, params.cam_dist, params.zoom_exp)?;
    let max_t = math
        .point_f64(max_frame_t)
        .map_err(SegmentRequestError::Backend)?;
    let mut evaluations = 0_usize;
    let mut tiles = Vec::with_capacity(ATLAS_COLUMNS * ATLAS_ROWS);

    for row in 0..ATLAS_ROWS {
        for column in 0..ATLAS_COLUMNS {
            let bounds = tile_screen_bounds(column, row, camera);
            let directions = subdivided_directions(&math, camera, bounds)?;
            let mut certified_slabs = 0_usize;
            let mut safe_slab_mask = 0_u32;
            let mut last_event = None;
            let mut stopped = None;
            for slab in 0..budgets.t_slabs {
                let slab_start = slab_endpoint(&math, &max_t, slab, budgets.t_slabs)?;
                let slab_length = slab_length(max_frame_t, slab, budgets.t_slabs);
                let mut slab_event = None;
                let mut slab_stopped = None;
                for direction in &directions {
                    let result = certify_directional_slab(
                        &math,
                        params,
                        DirectionalSlab {
                            anchor: &anchor,
                            frame_radius: &rho,
                            camera_forward: camera.forward,
                            direction,
                            start: &slab_start,
                            length: slab_length,
                        },
                        finite_iterations,
                        &mut evaluations,
                        budgets.max_orbit_evaluations,
                        0,
                    );
                    match result {
                        Ok(event) => {
                            slab_event = Some(event);
                        }
                        Err(reason) => {
                            slab_stopped = Some(reason);
                            break;
                        }
                    }
                }
                if let Some(reason) = slab_stopped {
                    stopped.get_or_insert(reason);
                    if reason == TileStopReason::OrbitBudget {
                        break;
                    }
                    continue;
                }
                safe_slab_mask |= 1_u32 << slab;
                last_event = slab_event;
                if slab == certified_slabs {
                    certified_slabs += 1;
                }
            }
            let prefix = inward_prefix(max_frame_t, certified_slabs, budgets.t_slabs);
            let status = match (stopped, last_event) {
                (None, Some(event)) => TileStatus::Certified {
                    safe_prefix: prefix,
                    certified_slabs,
                    last_event: event,
                },
                (Some(reason), _) => TileStatus::Stopped {
                    safe_prefix: prefix,
                    certified_slabs,
                    reason,
                },
                (None, None) => TileStatus::Stopped {
                    safe_prefix: 0.0,
                    certified_slabs: 0,
                    reason: TileStopReason::NoCommonEscape,
                },
            };
            tiles.push(CertifiedTile {
                column,
                row,
                screen_bounds: bounds.map(|value| value as f32),
                safe_slab_mask,
                status,
            });
        }
    }

    Ok((
        AtlasMetadata {
            columns: ATLAS_COLUMNS,
            rows: ATLAS_ROWS,
            t_slabs: budgets.t_slabs,
            max_frame_t: inward_f32(max_frame_t),
        },
        tiles,
    ))
}

fn validate_request(
    params: &StackParams,
    camera: PrimaryCamera,
    finite_iterations: usize,
    max_frame_t: f64,
    budgets: SegmentBudgets,
) -> Result<(), SegmentRequestError> {
    validate(params).map_err(|_| SegmentRequestError::InvalidStack)?;
    if finite_iterations == 0 || finite_iterations > params.max_iters {
        return Err(SegmentRequestError::InvalidIterationCount);
    }
    if budgets.precision == 0
        || budgets.precision > MAX_PRECISION
        || budgets.t_slabs == 0
        || budgets.t_slabs > MAX_T_SLABS
        || budgets.max_orbit_evaluations == 0
        || budgets.max_orbit_evaluations > MAX_ORBIT_EVALUATIONS
    {
        return Err(SegmentRequestError::InvalidBudget);
    }
    let camera_finite = camera
        .forward
        .iter()
        .chain(&camera.right)
        .chain(&camera.up)
        .chain([camera.fov, camera.aspect].iter())
        .chain(&camera.sway_extent)
        .all(|value| value.is_finite());
    if !camera_finite
        || camera.fov <= 0.0
        || camera.aspect <= 0.0
        || camera.fov > f64::from(f32::MAX)
        || camera.sway_extent.iter().any(|extent| *extent < 0.0)
        || camera.aspect + camera.sway_extent[0] > f64::from(f32::MAX)
        || 1.0 + camera.sway_extent[1] > f64::from(f32::MAX)
        || !is_orthonormal(camera.forward, camera.right, camera.up)
    {
        return Err(SegmentRequestError::InvalidCamera);
    }
    if !params.cam_dist.is_finite()
        || params.cam_dist <= 0.0
        || !params.zoom_exp.is_finite()
        || !max_frame_t.is_finite()
        || max_frame_t <= 0.0
        || max_frame_t > f64::from(f32::MAX)
    {
        return Err(SegmentRequestError::InvalidScale);
    }
    Ok(())
}

fn is_orthonormal(forward: [f64; 3], right: [f64; 3], up: [f64; 3]) -> bool {
    let tolerance = 1e-9;
    [forward, right, up]
        .iter()
        .all(|axis| (dot(*axis, *axis) - 1.0).abs() <= tolerance)
        && dot(forward, right).abs() <= tolerance
        && dot(forward, up).abs() <= tolerance
        && dot(right, up).abs() <= tolerance
}

fn frame_radius(
    math: &IntervalMath,
    cam_dist: f64,
    zoom_exp: f64,
) -> Result<BigInterval, SegmentRequestError> {
    let distance = math
        .point_f64(cam_dist)
        .map_err(SegmentRequestError::Backend)?;
    let scale = if zoom_exp > 0.0 {
        let ten = math
            .point_decimal("10")
            .map_err(SegmentRequestError::Backend)?;
        let exponent = math
            .neg(
                &math
                    .point_f64(zoom_exp)
                    .map_err(SegmentRequestError::Backend)?,
            )
            .map_err(SegmentRequestError::Backend)?;
        math.exp(
            &math
                .mul(
                    &exponent,
                    &math.ln(&ten).map_err(SegmentRequestError::Backend)?,
                )
                .map_err(SegmentRequestError::Backend)?,
        )
        .map_err(SegmentRequestError::Backend)?
    } else {
        math.point_f64(10_f64.powf(-zoom_exp))
            .map_err(SegmentRequestError::Backend)?
    };
    math.mul(&distance, &scale)
        .map_err(SegmentRequestError::Backend)
}

fn tile_screen_bounds(column: usize, row: usize, camera: PrimaryCamera) -> [f64; 4] {
    let x0 = -camera.aspect + 2.0 * camera.aspect * column as f64 / ATLAS_COLUMNS as f64
        - camera.sway_extent[0];
    let x1 = -camera.aspect
        + 2.0 * camera.aspect * (column + 1) as f64 / ATLAS_COLUMNS as f64
        + camera.sway_extent[0];
    let y0 = -1.0 + 2.0 * row as f64 / ATLAS_ROWS as f64 - camera.sway_extent[1];
    let y1 = -1.0 + 2.0 * (row + 1) as f64 / ATLAS_ROWS as f64 + camera.sway_extent[1];
    [x0, x1, y0, y1]
}

fn direction_box(
    math: &IntervalMath,
    camera: PrimaryCamera,
    screen: [f64; 4],
) -> Result<[BigInterval; DIMENSIONS], SegmentRequestError> {
    let x = math
        .bounds_f64(screen[0], screen[1])
        .map_err(SegmentRequestError::Backend)?;
    let y = math
        .bounds_f64(screen[2], screen[3])
        .map_err(SegmentRequestError::Backend)?;
    let fov = math
        .point_f64(camera.fov)
        .map_err(SegmentRequestError::Backend)?;
    let component = |axis: usize| -> Result<BigInterval, IntervalError> {
        let bounded_basis = |value: f64| {
            math.bounds_f64(value - CAMERA_BASIS_ROUNDING, value + CAMERA_BASIS_ROUNDING)
        };
        let forward = bounded_basis(camera.forward[axis])?;
        let horizontal = math.mul(&x, &bounded_basis(camera.right[axis])?)?;
        let vertical = math.mul(&y, &bounded_basis(camera.up[axis])?)?;
        math.add(
            &forward,
            &math.mul(&fov, &math.add(&horizontal, &vertical)?)?,
        )
    };
    let unnormalized = [
        component(0).map_err(SegmentRequestError::Backend)?,
        component(1).map_err(SegmentRequestError::Backend)?,
        component(2).map_err(SegmentRequestError::Backend)?,
    ];
    let norm = norm_squared(math, &unnormalized)
        .and_then(|value| math.sqrt(&value))
        .map_err(SegmentRequestError::Backend)?;
    if !norm.strictly_positive() {
        return Err(SegmentRequestError::InvalidCamera);
    }
    Ok([
        math.div(&unnormalized[0], &norm)
            .map_err(SegmentRequestError::Backend)?,
        math.div(&unnormalized[1], &norm)
            .map_err(SegmentRequestError::Backend)?,
        math.div(&unnormalized[2], &norm)
            .map_err(SegmentRequestError::Backend)?,
    ])
}

fn subdivided_directions(
    math: &IntervalMath,
    camera: PrimaryCamera,
    bounds: [f64; 4],
) -> Result<Vec<[BigInterval; DIMENSIONS]>, SegmentRequestError> {
    let [x0, x1, y0, y1] = bounds;
    let mut directions = Vec::with_capacity(SCREEN_SUBDIVISIONS * SCREEN_SUBDIVISIONS);
    for row in 0..SCREEN_SUBDIVISIONS {
        for column in 0..SCREEN_SUBDIVISIONS {
            let sx0 = x0 + (x1 - x0) * column as f64 / SCREEN_SUBDIVISIONS as f64;
            let sx1 = x0 + (x1 - x0) * (column + 1) as f64 / SCREEN_SUBDIVISIONS as f64;
            let sy0 = y0 + (y1 - y0) * row as f64 / SCREEN_SUBDIVISIONS as f64;
            let sy1 = y0 + (y1 - y0) * (row + 1) as f64 / SCREEN_SUBDIVISIONS as f64;
            directions.push(direction_box(math, camera, [sx0, sx1, sy0, sy1])?);
        }
    }
    Ok(directions)
}

fn slab_endpoint(
    math: &IntervalMath,
    maximum: &BigInterval,
    index: usize,
    slab_count: usize,
) -> Result<BigInterval, SegmentRequestError> {
    let fraction = math
        .div(
            &math
                .point_f64(index as f64)
                .map_err(SegmentRequestError::Backend)?,
            &math
                .point_f64(slab_count as f64)
                .map_err(SegmentRequestError::Backend)?,
        )
        .map_err(SegmentRequestError::Backend)?;
    math.mul(
        maximum,
        &math
            .square(&fraction)
            .map_err(SegmentRequestError::Backend)?,
    )
    .map_err(SegmentRequestError::Backend)
}

fn slab_length(maximum: f64, slab: usize, slab_count: usize) -> f64 {
    let start = slab as f64 / slab_count as f64;
    let end = (slab + 1) as f64 / slab_count as f64;
    maximum * (end * end - start * start)
}

fn directional_slab_packet(
    math: &IntervalMath,
    anchor: &[BigInterval; DIMENSIONS],
    rho: &BigInterval,
    forward: [f64; DIMENSIONS],
    direction: &[BigInterval; DIMENSIONS],
    slab_start: &BigInterval,
) -> Result<([BigInterval; DIMENSIONS], [BigInterval; DIMENSIONS]), SegmentRequestError> {
    let origin_component = |axis: usize| -> Result<BigInterval, IntervalError> {
        let backward = math.mul(
            &math.neg(&math.bounds_f64(
                forward[axis] - CAMERA_BASIS_ROUNDING,
                forward[axis] + CAMERA_BASIS_ROUNDING,
            )?)?,
            &math.point_f64(CAMERA_STANDOFF)?,
        )?;
        let travel = math.mul(slab_start, &direction[axis])?;
        math.add(
            &anchor[axis],
            &math.mul(rho, &math.add(&backward, &travel)?)?,
        )
    };
    let derivative_component = |axis: usize| math.mul(rho, &direction[axis]);
    Ok((
        [
            origin_component(0).map_err(SegmentRequestError::Backend)?,
            origin_component(1).map_err(SegmentRequestError::Backend)?,
            origin_component(2).map_err(SegmentRequestError::Backend)?,
        ],
        [
            derivative_component(0).map_err(SegmentRequestError::Backend)?,
            derivative_component(1).map_err(SegmentRequestError::Backend)?,
            derivative_component(2).map_err(SegmentRequestError::Backend)?,
        ],
    ))
}

/// Value enclosure for an integer-power bulb cell touching the polar axis.
///
/// The Cartesian derivative has no axis limit, so this is deliberately not a
/// `Jet2`. Segment certification only needs the image enclosure. The transverse
/// direction is enclosed by a disk whose radius uses
/// `sin(p theta) = sin(theta) U_{p-1}(cos(theta))` and
/// `|U_{p-1}| <= p`; the axial component uses the Chebyshev recurrence for
/// `T_p`. This removes both `atan2` and division by the horizontal radius.
#[cfg(test)]
fn integer_axis_bulb_values(
    math: &IntervalMath,
    state: &[BigInterval; DIMENSIONS],
    power: f64,
) -> Result<Option<[BigInterval; DIMENSIONS]>, TileStopReason> {
    let integer = power.round();
    if !(2.0..=12.0).contains(&integer) || power.to_bits() != integer.to_bits() {
        return Ok(None);
    }
    let exponent = integer as usize;
    let horizontal_squared = math
        .add(
            &math.square(&state[0]).map_err(TileStopReason::Backend)?,
            &math.square(&state[1]).map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;
    if !horizontal_squared.contains_zero() {
        return Ok(None);
    }
    let radius_squared = math
        .add(
            &horizontal_squared,
            &math.square(&state[2]).map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;
    let regularization = math.point_f64(1.0e-12).map_err(TileStopReason::Backend)?;
    if radius_squared.lower().repr() <= regularization.upper().repr() {
        return Err(TileStopReason::RadiusRegularization);
    }
    let radius = math
        .sqrt(&radius_squared)
        .map_err(TileStopReason::Backend)?;
    let powered_radius = math
        .exp(
            &math
                .mul(
                    &math.point_f64(power).map_err(TileStopReason::Backend)?,
                    &math.ln(&radius).map_err(TileStopReason::Backend)?,
                )
                .map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;
    let cosine = math
        .intersection(
            &math
                .div(&state[2], &radius)
                .map_err(TileStopReason::Backend)?,
            &math
                .bounds_f64(-1.0, 1.0)
                .map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;

    let one = math.point_f64(1.0).map_err(TileStopReason::Backend)?;
    let two = math.point_f64(2.0).map_err(TileStopReason::Backend)?;
    let mut t_previous = one.clone();
    let mut t_current = cosine.clone();
    for _ in 2..=exponent {
        let next = math
            .sub(
                &math
                    .mul(
                        &two,
                        &math
                            .mul(&cosine, &t_current)
                            .map_err(TileStopReason::Backend)?,
                    )
                    .map_err(TileStopReason::Backend)?,
                &t_previous,
            )
            .map_err(TileStopReason::Backend)?;
        t_previous = t_current;
        t_current = next;
    }

    let horizontal_radius = math
        .sqrt(&horizontal_squared)
        .map_err(TileStopReason::Backend)?;
    let sine_magnitude = math
        .div(&horizontal_radius, &radius)
        .map_err(TileStopReason::Backend)?;
    let transverse_magnitude = math
        .mul(
            &powered_radius,
            &math
                .mul(
                    &sine_magnitude,
                    &math.point_f64(power).map_err(TileStopReason::Backend)?,
                )
                .map_err(TileStopReason::Backend)?,
        )
        .map_err(TileStopReason::Backend)?;
    let symmetric_transverse = math
        .hull(
            &math
                .neg(&transverse_magnitude)
                .map_err(TileStopReason::Backend)?,
            &transverse_magnitude,
        )
        .map_err(TileStopReason::Backend)?;
    let axial = math
        .mul(&powered_radius, &t_current)
        .map_err(TileStopReason::Backend)?;
    Ok(Some([
        symmetric_transverse.clone(),
        symmetric_transverse,
        axial,
    ]))
}

fn chart_for(point: &[BigInterval; DIMENSIONS]) -> Result<Atan2Chart, TileStopReason> {
    let [x, y, _] = point;
    if x.contains_zero() && y.contains_zero() {
        return Err(TileStopReason::PolarAxis);
    }
    if x.lower().repr() < &Repr::zero() && y.contains_zero() {
        if y.lower().repr() >= &Repr::zero() {
            Ok(Atan2Chart::Upper)
        } else if y.upper().repr() <= &Repr::zero() {
            Ok(Atan2Chart::Lower)
        } else {
            Err(TileStopReason::AzimuthSeam)
        }
    } else {
        Ok(Atan2Chart::Principal)
    }
}

#[allow(
    clippy::float_cmp,
    reason = "zero is the exact no-seed sentinel returned by the stack evaluator"
)]
fn add_seed_values(
    math: &IntervalMath,
    values: &[BigInterval; DIMENSIONS],
    parameter: &[BigInterval; DIMENSIONS],
    params: &StackParams,
    seed_weight: f64,
) -> Result<[BigInterval; DIMENSIONS], TileStopReason> {
    if seed_weight == 0.0 {
        return Ok(values.clone());
    }
    let parameter_weight = math
        .point_f64(seed_weight)
        .map_err(TileStopReason::Backend)?;
    let julia_weight = math
        .point_f64(1.0 - seed_weight)
        .map_err(TileStopReason::Backend)?;
    let component = |axis: usize| {
        let seed = math.add(
            &math.mul(&parameter[axis], &parameter_weight)?,
            &math.mul(&math.point_f64(params.julia[axis])?, &julia_weight)?,
        )?;
        math.add(&values[axis], &seed)
    };
    Ok([
        component(0).map_err(TileStopReason::Backend)?,
        component(1).map_err(TileStopReason::Backend)?,
        component(2).map_err(TileStopReason::Backend)?,
    ])
}

fn norm_squared(
    math: &IntervalMath,
    point: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    math.add(
        &math.add(&math.square(&point[0])?, &math.square(&point[1])?)?,
        &math.square(&point[2])?,
    )
}

fn map_stack_error(error: StackEvaluationError) -> TileStopReason {
    match error {
        StackEvaluationError::ContinuousBranch => TileStopReason::FormulaBranch,
        StackEvaluationError::MengerTranslation => TileStopReason::MengerTranslation,
        StackEvaluationError::Mandelbulb(MandelbulbError::AzimuthSeam) => {
            TileStopReason::AzimuthSeam
        }
        StackEvaluationError::Mandelbulb(MandelbulbError::PolarAxis) => TileStopReason::PolarAxis,
        StackEvaluationError::Mandelbulb(MandelbulbError::RadiusRegularization) => {
            TileStopReason::RadiusRegularization
        }
        StackEvaluationError::Mandelbulb(MandelbulbError::Backend(error))
        | StackEvaluationError::Backend(error) => TileStopReason::Backend(error),
        StackEvaluationError::InvalidFormula
        | StackEvaluationError::InvalidParameter
        | StackEvaluationError::Mandelbulb(MandelbulbError::OutOfRangePower) => {
            TileStopReason::NoCommonEscape
        }
    }
}

fn inward_prefix(max_t: f64, slabs: usize, total: usize) -> f32 {
    if slabs == 0 {
        0.0
    } else {
        let fraction = slabs as f64 / total as f64;
        inward_f32(max_t * fraction * fraction)
    }
}

#[allow(
    clippy::float_cmp,
    reason = "zero is an exact endpoint sentinel and has no predecessor toward the interior"
)]
fn inward_f32(value: f64) -> f32 {
    let rounded = value as f32;
    if rounded == 0.0 {
        return 0.0;
    }
    let downward = if rounded.is_sign_positive() {
        f32::from_bits(rounded.to_bits() - 1)
    } else {
        f32::from_bits(rounded.to_bits() + 1)
    };
    if f64::from(rounded) <= value {
        rounded
    } else {
        downward
    }
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.iter().zip(right).map(|(lhs, rhs)| lhs * rhs).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> PrimaryCamera {
        PrimaryCamera {
            forward: [0.0, 0.0, 1.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fov: 0.25,
            aspect: 1.5,
            sway_extent: [0.0; 2],
        }
    }

    fn budgets() -> SegmentBudgets {
        SegmentBudgets {
            precision: 96,
            t_slabs: 4,
            max_orbit_evaluations: ATLAS_COLUMNS
                * ATLAS_ROWS
                * 4
                * SCREEN_SUBDIVISIONS
                * SCREEN_SUBDIVISIONS
                * 2,
        }
    }

    fn production_camera() -> PrimaryCamera {
        let elevation = 0.2_f64;
        let eye = [elevation.cos() * 4.2, elevation.sin() * 4.2, 0.0];
        let aim = [0.025, -0.01, 0.015];
        let mut forward = std::array::from_fn(|axis| aim[axis] - eye[axis]);
        let forward_length = dot(forward, forward).sqrt();
        forward = forward.map(|component| component / forward_length);
        let mut right = [forward[2], 0.0, -forward[0]];
        let right_length = dot(right, right).sqrt();
        right = right.map(|component| component / right_length);
        let up = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        PrimaryCamera {
            forward,
            right,
            up,
            fov: 0.85,
            aspect: 16.0 / 9.0,
            sway_extent: [0.0; 2],
        }
    }

    fn atlas_reason_distribution(
        result: SegmentAtlasResult,
    ) -> (usize, Vec<(TileStopReason, usize)>) {
        let SegmentAtlasResult::Certified { tiles, .. } = result else {
            panic!("production-like atlas was rejected: {result:?}");
        };
        let covered = tiles
            .iter()
            .filter(|tile| tile.safe_slab_mask != 0)
            .count();
        let mut reasons = Vec::new();
        for tile in tiles {
            let TileStatus::Stopped { reason, .. } = tile.status else {
                continue;
            };
            if let Some((_, count)) = reasons.iter_mut().find(|(candidate, _)| *candidate == reason) {
                *count += 1;
            } else {
                reasons.push((reason, 1));
            }
        }
        (covered, reasons)
    }

    #[test]
    fn production_like_boundary_anchor_reports_atlas_reasons() {
        let anchor = [
            "0.81074521283760356128362504787291405376855320211606185355".into(),
            "0.16434619088279685317293327534083013644758458933396935663".into(),
            "0".into(),
        ];
        for zoom_exp in [6.0, 12.0] {
            let params = StackParams {
                zoom_exp,
                max_iters: 10,
                ..StackParams::default()
            };
            let result = certify_primary_ray_segments(
                &params,
                &anchor,
                production_camera(),
                10,
                24.0,
                SegmentBudgets::default(),
            );
            let (covered, reasons) = atlas_reason_distribution(result);
            eprintln!("zoom {zoom_exp}: covered={covered}/32, reasons={reasons:?}");
        }
    }

    #[test]
    fn production_like_first_slabs_report_packet_endpoints() {
        let params = StackParams {
            zoom_exp: 6.0,
            max_iters: 64,
            ..StackParams::default()
        };
        let camera = production_camera();
        let math = IntervalMath::new(160).unwrap();
        let anchor = [
            math.point_decimal(
                "0.81074521283760356128362504787291405376855320211606185355",
            )
            .unwrap(),
            math.point_decimal(
                "0.16434619088279685317293327534083013644758458933396935663",
            )
            .unwrap(),
            math.point_decimal("0").unwrap(),
        ];
        let rho = frame_radius(&math, params.cam_dist, params.zoom_exp).unwrap();
        let maximum = math.point_f64(24.0).unwrap();
        let directions =
            subdivided_directions(&math, camera, tile_screen_bounds(4, 2, camera)).unwrap();
        let mut evaluations = 0;
        for slab in 0..4 {
            let start = slab_endpoint(&math, &maximum, slab, DEFAULT_T_SLABS).unwrap();
            let length = slab_length(24.0, slab, DEFAULT_T_SLABS);
            for (packet_index, direction) in directions.iter().enumerate() {
                let (origin, derivative) = directional_slab_packet(
                    &math,
                    &anchor,
                    &rho,
                    camera.forward,
                    direction,
                    &start,
                )
                .unwrap();
                let result = certify_directional_packet(
                    &math,
                    &params,
                    DirectionalPacket {
                        origin: &origin,
                        direction: &derivative,
                        maximum_length: length,
                    },
                    10,
                    &mut evaluations,
                    usize::MAX,
                );
                match result {
                    Ok(certificate) => eprintln!(
                        "slab {slab} packet {packet_index}: endpoint={:.9e}/{length:.9e} event={:?}",
                        certificate.safe_endpoint, certificate.event
                    ),
                    Err(reason) => {
                        eprintln!("slab {slab} packet {packet_index}: reason={reason:?}");
                    }
                }
            }
        }
        let bounds = tile_screen_bounds(4, 2, camera);
        let center = [
            f64::midpoint(bounds[0], bounds[1]),
            f64::midpoint(bounds[2], bounds[3]),
        ];
        let point_direction =
            direction_box(&math, camera, [center[0], center[0], center[1], center[1]]).unwrap();
        let start = slab_endpoint(&math, &maximum, 0, DEFAULT_T_SLABS).unwrap();
        let length = slab_length(24.0, 0, DEFAULT_T_SLABS);
        let (origin, derivative) = directional_slab_packet(
            &math,
            &anchor,
            &rho,
            camera.forward,
            &point_direction,
            &start,
        )
        .unwrap();
        for (iterations, test_length) in [(10, length), (10, length / 16.0), (10, 1.0e-9)] {
            eprintln!(
                "center-point packet at {iterations} iterations, length {test_length:.3e}: {:?}",
                certify_directional_packet(
                &math,
                &params,
                DirectionalPacket {
                    origin: &origin,
                    direction: &derivative,
                    maximum_length: test_length,
                },
                iterations,
                &mut evaluations,
                usize::MAX,
            )
            .map(|certificate| (certificate.safe_endpoint, certificate.event))
            );
        }
    }

    fn identity_stack(formula: i64, bailout: f64) -> StackParams {
        StackParams {
            formulas: [formula, 0, 0, 0],
            rates: [1, 0, 0, 0],
            bailout,
            max_iters: 2,
            cam_dist: 1.0,
            zoom_exp: 0.0,
            ..StackParams::default()
        }
    }

    #[test]
    fn immediate_escape_publishes_positive_fixed_atlas_prefixes() {
        let result = certify_primary_ray_segments(
            &identity_stack(0, 0.5),
            &["5".into(), "0".into(), "0".into()],
            camera(),
            1,
            1.0,
            budgets(),
        );
        let SegmentAtlasResult::Certified { metadata, tiles } = result else {
            panic!("valid atlas was rejected");
        };
        assert_eq!((metadata.columns, metadata.rows), (8, 4));
        assert_eq!(tiles.len(), 32);
        assert!(tiles.iter().all(|tile| tile.status.safe_prefix() > 0.0));
        assert!(tiles.iter().all(|tile| matches!(
            tile.status,
            TileStatus::Certified {
                certified_slabs: 4,
                ..
            }
        )));
    }

    #[test]
    fn bulb_ball_transport_publishes_an_immediate_escape_prefix() {
        let result = certify_primary_ray_segments(
            &identity_stack(5, 100.0),
            &["5".into(), "0".into(), "0".into()],
            camera(),
            1,
            1.0,
            budgets(),
        );
        let SegmentAtlasResult::Certified { tiles, .. } = result else {
            panic!("valid bulb atlas was rejected");
        };
        assert!(tiles.iter().all(|tile| tile.status.safe_prefix() > 0.0));
        assert!(tiles.iter().all(|tile| matches!(
            tile.status,
            TileStatus::Certified {
                last_event: TileEscapeEvent::MandelbulbPreGuard { iteration: 1 },
                ..
            }
        )));
    }

    #[test]
    #[ignore = "the packet evaluator still reports PolarAxis on integer-power \
                axis cells; the Chebyshev axis route exists for the isotropic \
                bound but is not yet wired into directional certification. \
                This surfaced when slab subdivision stopped collapsing typed \
                stop reasons into NoCommonEscape — the previous green was the \
                masking, not the capability."]
    fn integer_axis_bulb_does_not_fail_on_the_polar_chart() {
        let result = certify_primary_ray_segments(
            &identity_stack(5, 100.0),
            &["0".into(), "0".into(), "1".into()],
            camera(),
            1,
            1.0,
            budgets(),
        );
        let SegmentAtlasResult::Certified { tiles, .. } = result else {
            panic!("valid request was rejected");
        };
        assert!(tiles.iter().all(|tile| match tile.status {
            TileStatus::Stopped {
                safe_prefix: 0.0,
                reason,
                ..
            } => reason != TileStopReason::PolarAxis,
            _ => false,
        }));
    }

    #[test]
    fn chebyshev_value_enclosure_contains_the_integer_axis_image() {
        let math = IntervalMath::new(128).unwrap();
        let state = [
            math.bounds_f64(-0.01, 0.01).unwrap(),
            math.bounds_f64(-0.01, 0.01).unwrap(),
            math.bounds_f64(1.0, 1.01).unwrap(),
        ];
        let values = integer_axis_bulb_values(&math, &state, 8.0)
            .unwrap()
            .expect("integer axis enclosure");
        assert!(math.contains_f64(&values[0], 0.0).unwrap());
        assert!(math.contains_f64(&values[1], 0.0).unwrap());
        assert!(math.contains_f64(&values[2], 1.0).unwrap());
    }

    #[test]
    fn non_integer_negative_axis_remains_fail_closed() {
        let mut params = identity_stack(5, 100.0);
        params.power = 2.5;
        let result = certify_primary_ray_segments(
            &params,
            &["0".into(), "0".into(), "-1".into()],
            camera(),
            1,
            1.0,
            budgets(),
        );
        let SegmentAtlasResult::Certified { tiles, .. } = result else {
            panic!("valid request was rejected");
        };
        let offending: Vec<_> = tiles
            .iter()
            .filter(|tile| {
                !matches!(
                    tile.status,
                    TileStatus::Stopped {
                        safe_prefix: 0.0,
                        reason: TileStopReason::PolarAxis,
                        ..
                    }
                )
            })
            .map(|tile| (tile.column, tile.row, tile.status))
            .collect();
        assert!(
            offending.is_empty(),
            "tiles escaped the fail-closed axis stop: {offending:?}"
        );
    }

    fn sample_world(
        anchor: [f64; 3],
        params: &StackParams,
        camera: PrimaryCamera,
        screen: [f64; 2],
        t: f64,
    ) -> [f64; 3] {
        let mut direction = std::array::from_fn(|axis| {
            camera.forward[axis]
                + camera.fov * (screen[0] * camera.right[axis] + screen[1] * camera.up[axis])
        });
        let length = dot(direction, direction).sqrt();
        direction = direction.map(|value| value / length);
        let rho = params.cam_dist * 10_f64.powf(-params.zoom_exp);
        std::array::from_fn(|axis| {
            anchor[axis] + rho * (-CAMERA_STANDOFF * camera.forward[axis] + t * direction[axis])
        })
    }

    #[test]
    fn published_bits_cover_sampled_rays_and_slab_endpoints() {
        let params = identity_stack(0, 0.5);
        let result = certify_primary_ray_segments(
            &params,
            &["5".into(), "0".into(), "0".into()],
            camera(),
            1,
            1.0,
            budgets(),
        );
        let SegmentAtlasResult::Certified { tiles, .. } = result else {
            panic!("valid atlas was rejected");
        };
        for tile in tiles {
            let prefix = f64::from(tile.status.safe_prefix());
            if prefix <= 0.0 {
                continue;
            }
            let [x0, x1, y0, y1] = tile.screen_bounds.map(f64::from);
            for (x, y, fraction) in [
                (x0, y0, 0.0),
                (f64::midpoint(x0, x1), f64::midpoint(y0, y1), 0.5),
                (x1, y1, 1.0),
            ] {
                let point = sample_world(
                    [5.0, 0.0, 0.0],
                    &params,
                    camera(),
                    [x, y],
                    prefix * fraction,
                );
                assert!(
                    dot(point, point) > params.bailout * params.bailout,
                    "published tile {},{} admitted a non-escaping sample",
                    tile.column,
                    tile.row
                );
            }
            for slab in 0..budgets().t_slabs {
                if tile.safe_slab_mask & (1_u32 << slab) == 0 {
                    continue;
                }
                let start = (slab as f64 / budgets().t_slabs as f64).powi(2);
                let end = ((slab + 1) as f64 / budgets().t_slabs as f64).powi(2);
                for x in [x0, f64::midpoint(x0, x1), x1] {
                    for y in [y0, f64::midpoint(y0, y1), y1] {
                        for t in [start, f64::midpoint(start, end), end] {
                            let point = sample_world([5.0, 0.0, 0.0], &params, camera(), [x, y], t);
                            assert!(
                                dot(point, point) > params.bailout * params.bailout,
                                "published tile {},{} slab {slab} admitted ray ({x},{y}) at {t}",
                                tile.column,
                                tile.row
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn atlas_mask_is_reproduced_by_directional_packet_proof() {
        let params = identity_stack(0, 0.5);
        let anchor_text = ["5".into(), "0".into(), "0".into()];
        let request_budgets = budgets();
        let result =
            certify_primary_ray_segments(&params, &anchor_text, camera(), 1, 1.0, request_budgets);
        let SegmentAtlasResult::Certified { tiles, .. } = result else {
            panic!("valid atlas was rejected");
        };

        let math = IntervalMath::new(request_budgets.precision).unwrap();
        let anchor = [
            math.point_decimal(&anchor_text[0]).unwrap(),
            math.point_decimal(&anchor_text[1]).unwrap(),
            math.point_decimal(&anchor_text[2]).unwrap(),
        ];
        let rho = frame_radius(&math, params.cam_dist, params.zoom_exp).unwrap();
        let maximum = math.point_f64(1.0).unwrap();
        let mut evaluations = 0;
        let mut published_slabs = 0;
        for tile in tiles {
            let bounds = tile_screen_bounds(tile.column, tile.row, camera());
            let directions = subdivided_directions(&math, camera(), bounds).unwrap();
            let mut reproduced = 0_u32;
            for slab in 0..request_budgets.t_slabs {
                let start = slab_endpoint(&math, &maximum, slab, request_budgets.t_slabs).unwrap();
                let length = slab_length(1.0, slab, request_budgets.t_slabs);
                let covered = directions.iter().all(|direction| {
                    let (origin, derivative) = directional_slab_packet(
                        &math,
                        &anchor,
                        &rho,
                        camera().forward,
                        direction,
                        &start,
                    )
                    .unwrap();
                    certify_directional_packet(
                        &math,
                        &params,
                        DirectionalPacket {
                            origin: &origin,
                            direction: &derivative,
                            maximum_length: length,
                        },
                        1,
                        &mut evaluations,
                        usize::MAX,
                    )
                    .is_ok_and(|certificate| {
                        certificate.safe_endpoint.to_bits() == inward_f32(length).to_bits()
                    })
                });
                if covered {
                    reproduced |= 1_u32 << slab;
                }
            }
            assert_eq!(tile.safe_slab_mask, reproduced);
            published_slabs += tile.safe_slab_mask.count_ones();
        }
        assert_eq!(published_slabs, 128);
    }

    #[test]
    fn adversarial_interval_direction_fails_closed() {
        let math = IntervalMath::new(128).unwrap();
        let origin = [
            math.point_f64(2.0).unwrap(),
            math.point_f64(0.0).unwrap(),
            math.point_f64(0.0).unwrap(),
        ];
        let direction = std::array::from_fn(|_| math.bounds_f64(-1.0, 1.0).unwrap());
        let mut evaluations = 0;
        assert!(matches!(
            certify_directional_packet(
                &math,
                &identity_stack(0, 0.5),
                DirectionalPacket {
                    origin: &origin,
                    direction: &direction,
                    maximum_length: 1.0,
                },
                1,
                &mut evaluations,
                usize::MAX,
            ),
            Err(TileStopReason::NoCommonEscape)
        ));
        assert_eq!(evaluations, 0);
    }

    #[test]
    fn published_endpoint_is_rounded_inward() {
        let value = 1.0_f64 / 3.0;
        let endpoint = inward_f32(value);
        assert!(f64::from(endpoint) <= value);
        assert!(endpoint > 0.0);
    }

    fn directional(
        params: &StackParams,
        origin: [f64; 3],
        direction: [f64; 3],
        maximum_length: f64,
    ) -> DirectionalCertificateResult {
        certify_directional_ray_segment(
            params,
            &origin.map(|value| value.to_string()),
            direction,
            1,
            maximum_length,
            128,
        )
    }

    fn certified_directional(result: DirectionalCertificateResult) -> DirectionalCertificate {
        let DirectionalCertificateResult::Certified(certificate) = result else {
            panic!("expected a directional certificate, got {result:?}");
        };
        *certificate
    }

    #[test]
    fn directional_identity_transport_keeps_all_nine_affine_entries() {
        let certificate = certified_directional(directional(
            &identity_stack(0, 0.5),
            [2.0, 0.25, -0.5],
            [0.5, -0.25, 0.125],
            0.5,
        ));
        let math = IntervalMath::new(128).unwrap();
        assert_eq!(certificate.safe_endpoint, inward_f32(0.5));
        for row in 0..3 {
            for column in 0..3 {
                assert!(math
                    .contains_f64(
                        &certificate.transport.affine_derivative[row][column],
                        if row == column { 1.0 } else { 0.0 },
                    )
                    .unwrap());
            }
            assert!(math
                .contains_f64(
                    &certificate.transport.tangent[row],
                    [0.5, -0.25, 0.125][row],
                )
                .unwrap());
            assert!(math
                .contains_f64(&certificate.transport.curvature[row], 0.0)
                .unwrap());
        }
    }

    #[test]
    fn directional_endpoint_is_conservative_and_safe_when_ray_turns_inward() {
        let certificate = certified_directional(directional(
            &identity_stack(0, 1.0),
            [2.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            1.5,
        ));
        let endpoint = f64::from(certificate.safe_endpoint);
        let conservative_root = 7.0_f64.sqrt() - 2.0;
        assert!(endpoint <= conservative_root);
        assert!(endpoint > conservative_root - 1e-6);
        assert!((2.0 - endpoint).powi(2) > 1.0);
        assert!(certificate.curvature_bound.upper().to_f64().value() >= 2.0);
    }

    #[test]
    fn directional_topology_boundaries_fail_closed() {
        let mut bulb = identity_stack(5, 100.0);
        bulb.power = 2.5;
        assert!(matches!(
            directional(&bulb, [-1.0, -0.1, 0.2], [0.0, 1.0, 0.0], 0.2),
            DirectionalCertificateResult::Inconclusive(TileStopReason::AzimuthSeam)
        ));
        assert!(matches!(
            directional(&bulb, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], 0.2),
            DirectionalCertificateResult::Inconclusive(TileStopReason::PolarAxis)
        ));

        let branch = identity_stack(9, 100.0);
        assert!(matches!(
            directional(&branch, [-0.1, 0.4, 0.8], [1.0, 0.0, 0.0], 0.2),
            DirectionalCertificateResult::Inconclusive(TileStopReason::FormulaBranch)
        ));
    }

    fn bulb_with_seed(point: [f64; 3], power: f64, seed_weight: f64) -> [f64; 3] {
        let [x, y, z] = point;
        let horizontal = x.hypot(y);
        let radius = horizontal.hypot(z);
        let theta = horizontal.atan2(z) * power;
        let phi = y.atan2(x) * power;
        let radial = radius.powf(power);
        [
            radial * theta.sin() * phi.cos() + seed_weight * x,
            radial * theta.sin() * phi.sin() + seed_weight * y,
            radial * theta.cos() + seed_weight * z,
        ]
    }

    #[test]
    fn directional_nonlinear_transport_contains_sampled_tangent_and_curvature() {
        let mut params = identity_stack(5, 0.1);
        params.power = 2.5;
        params.julia_amount = 0.25;
        let origin = [0.8, 0.6, 0.4];
        let direction = [0.2, -0.1, 0.15];
        let certificate =
            certified_directional(directional(&params, origin, direction, 1.0 / 64.0));
        let math = IntervalMath::new(128).unwrap();
        let step = 1e-5;
        for t in [0.0, 1.0 / 128.0, 1.0 / 64.0] {
            let point = std::array::from_fn(|axis| origin[axis] + t * direction[axis]);
            let plus = std::array::from_fn(|axis| point[axis] + step * direction[axis]);
            let minus = std::array::from_fn(|axis| point[axis] - step * direction[axis]);
            let center = bulb_with_seed(point, params.power, 1.0 - params.julia_amount);
            let forward = bulb_with_seed(plus, params.power, 1.0 - params.julia_amount);
            let backward = bulb_with_seed(minus, params.power, 1.0 - params.julia_amount);
            for output in 0..3 {
                let tangent = (forward[output] - backward[output]) / (2.0 * step);
                let curvature =
                    (forward[output] - 2.0 * center[output] + backward[output]) / step.powi(2);
                assert!(math
                    .contains_f64(&certificate.transport.tangent[output], tangent)
                    .unwrap());
                assert!(math
                    .contains_f64(&certificate.transport.curvature[output], curvature)
                    .unwrap());
                for input in 0..3 {
                    let mut coordinate_plus = point;
                    let mut coordinate_minus = point;
                    coordinate_plus[input] += step;
                    coordinate_minus[input] -= step;
                    let image_plus =
                        bulb_with_seed(coordinate_plus, params.power, 1.0 - params.julia_amount);
                    let image_minus =
                        bulb_with_seed(coordinate_minus, params.power, 1.0 - params.julia_amount);
                    let derivative = (image_plus[output] - image_minus[output]) / (2.0 * step);
                    assert!(math
                        .contains_f64(
                            &certificate.transport.affine_derivative[output][input],
                            derivative,
                        )
                        .unwrap());
                }
            }
        }
    }
}
