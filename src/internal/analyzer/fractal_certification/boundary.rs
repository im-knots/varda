//! Certified first boundary event on the shader's parked camera ray.
//!
//! The implementation deliberately treats a ray as an ordered collection of
//! intervals. It never infers the state between two point samples and therefore
//! does not assume membership is monotone along the complete ray.

use dashu_float::{FBig, Repr};
use std::fmt;

use super::jet::Jet2;
use super::mandelbulb::{IntervalBox3, MandelbulbError};
use super::stack::{cycle, evaluate_slot, validate, StackEvaluationError};
use super::{Atan2Chart, BigInterval, IntervalError, IntervalMath};
use crate::internal::analyzer::fractal_reference_orbit::StackParams;

const DIMENSIONS: usize = 3;
const MAX_PRECISION: usize = 4_096;

type Matrix3 = [[BigInterval; DIMENSIONS]; DIMENSIONS];

/// Camera controls used by the shader's animation-independent parked probe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParkedCamera {
    pub(crate) distance: f64,
    pub(crate) azimuth: f64,
    pub(crate) elevation: f64,
    pub(crate) look: [f64; DIMENSIONS],
}

/// Shader-matched parked ray in ordinary geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParkedRay {
    pub(crate) origin: [f64; DIMENSIONS],
    pub(crate) direction: [f64; DIMENSIONS],
    pub(crate) length: f64,
    pub(crate) right: [f64; DIMENSIONS],
    pub(crate) up: [f64; DIMENSIONS],
}

/// Bounded work for one boundary query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoundaryBudgets {
    pub(crate) precision: usize,
    pub(crate) max_depth: usize,
    pub(crate) max_segments: usize,
    pub(crate) max_newton_steps: usize,
}

/// A target whose finite-solid boundary event and camera-plane gate are proved.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CertifiedBoundary {
    pub(crate) target: [String; DIMENSIONS],
    pub(crate) event_iteration: usize,
    pub(crate) effective_iterations: usize,
    pub(crate) bracket_width: f64,
    pub(crate) sigma_min_lower: f64,
}

/// Why no target was published.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BoundaryReason {
    InvalidCamera,
    InvalidBudget,
    InvalidStack,
    NoTransition,
    SegmentBudget,
    TangentRoot,
    SimultaneousEvent,
    EarlierEvent,
    AzimuthSeam,
    PolarAxis,
    RadiusRegularization,
    FormulaBranch,
    MengerTranslation,
    RootNotUnique,
    NewtonBudget,
    ResolutionGate { sigma_min_lower: f64, required: f64 },
    Backend(IntervalError),
    LongLivedSearchExhausted,
    InsufficientSurvival { found: usize, required: usize },
}

impl BoundaryReason {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::InvalidCamera => 1,
            Self::InvalidBudget => 2,
            Self::InvalidStack => 3,
            Self::NoTransition => 4,
            Self::SegmentBudget => 5,
            Self::TangentRoot => 6,
            Self::SimultaneousEvent => 7,
            Self::EarlierEvent => 8,
            Self::AzimuthSeam => 9,
            Self::PolarAxis => 10,
            Self::RadiusRegularization => 11,
            Self::FormulaBranch => 12,
            Self::MengerTranslation => 13,
            Self::RootNotUnique => 14,
            Self::NewtonBudget => 15,
            Self::ResolutionGate { .. } => 16,
            Self::Backend(_) => 17,
            Self::LongLivedSearchExhausted => 18,
            Self::InsufficientSurvival { .. } => 19,
        }
    }
}

impl fmt::Display for BoundaryReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for BoundaryReason {}

/// Atomic success or typed fail-closed result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BoundaryResult {
    Certified(CertifiedBoundary),
    Inconclusive(BoundaryReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Membership {
    Inside,
    Outside { event: EscapeEvent },
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    MandelbulbPreGuard,
    PostSlotBailout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EscapeEvent {
    iteration: usize,
    kind: EventKind,
}

#[derive(Clone)]
struct Ray {
    origin: [BigInterval; DIMENSIONS],
    direction: [BigInterval; DIMENSIONS],
    length: BigInterval,
    right: [f64; DIMENSIONS],
    up: [f64; DIMENSIONS],
}

struct EventEvaluation {
    margin: BigInterval,
    derivative: BigInterval,
    earlier_positive: bool,
    simultaneous: bool,
    derivative_matrix: Matrix3,
}

/// Finds and certifies the nearest visible boundary of the exact finite solid.
pub(crate) fn certify_camera_boundary(
    params: &StackParams,
    camera: ParkedCamera,
    budgets: BoundaryBudgets,
) -> BoundaryResult {
    // `max_iters` is the transport ceiling, not automatically the finite-solid
    // count whose boundary is useful at the requested frame scale. Each
    // deterministic proposal below is rerun through every proof against that
    // exact S_N, so the proposals themselves carry no trust.
    let mut effective = params.clone();
    let mut last = BoundaryReason::InvalidBudget;
    for count in iteration_candidates(params) {
        effective.max_iters = count;
        match certify_camera_boundary_inner(&effective, camera, budgets) {
            Ok(boundary) => return BoundaryResult::Certified(boundary),
            Err(reason) => last = reason,
        }
    }
    BoundaryResult::Inconclusive(last)
}

/// Extra iterations requested beyond the count that merely resolves the frame.
const DETAIL_ITERATIONS: usize = 32;

/// Cost ceiling on the certified count, independent of the transport ceiling.
const ITERATION_CEILING: usize = 192;

/// Finite-solid counts to attempt, deepest first.
///
/// The count is not only a proof parameter. The renderer marches exactly `S_N`,
/// so `N` decides how much structure the frame contains: the smallest count that
/// resolves the frame leaves an almost smooth level set, which reads as one
/// featureless lobe however deep the camera has gone. Asking for the resolution
/// minimum was why a correctly certified, correctly consumed payload still
/// rendered a flat gradient. Aim well above the minimum and take the deepest
/// count the proofs actually close on, rather than presuming which one that is.
fn iteration_candidates(params: &StackParams) -> Vec<usize> {
    let minimum = (params.zoom_exp.max(0.0).ceil() as usize)
        .saturating_add(4)
        .clamp(1, params.max_iters);
    let desired = minimum
        .saturating_add(DETAIL_ITERATIONS)
        .min(params.max_iters)
        .min(ITERATION_CEILING.max(minimum));
    let mut counts = vec![desired, usize::midpoint(desired, minimum), minimum];
    counts.dedup();
    counts
}

fn certify_camera_boundary_inner(
    params: &StackParams,
    camera: ParkedCamera,
    budgets: BoundaryBudgets,
) -> Result<CertifiedBoundary, BoundaryReason> {
    validate_request(params, camera, budgets)?;
    let math = IntervalMath::new(budgets.precision).map_err(BoundaryReason::Backend)?;
    let formulas = cycle(params).map_err(map_stack_error)?;
    let ray = parked_ray(&math, camera)?;
    let zero = math.point_f64(0.0).map_err(BoundaryReason::Backend)?;
    let one = math.point_f64(1.0).map_err(BoundaryReason::Backend)?;
    let (mut bracket, mut event) =
        isolate_first_transition(&math, params, &formulas, &ray, &zero, &one, budgets)?;
    for _ in 0..budgets.max_newton_steps {
        let evaluation = match evaluate_event(&math, params, &formulas, &ray, &bracket, event) {
            Ok(evaluation) => evaluation,
            Err(reason) if is_subdividable_topology(reason) => {
                (bracket, event) =
                    bisect_transition(&math, params, &formulas, &ray, &bracket, event)?;
                continue;
            }
            Err(reason) => return Err(reason),
        };
        if evaluation.derivative.contains_zero()
            || !evaluation.earlier_positive
            || evaluation.simultaneous
        {
            (bracket, event) = bisect_transition(&math, params, &formulas, &ray, &bracket, event)?;
            continue;
        }
        let midpoint = midpoint(&math, &bracket)?;
        let point_evaluation = evaluate_event(&math, params, &formulas, &ray, &midpoint, event)?;
        let newton = math
            .sub(
                &midpoint,
                &math
                    .div(&point_evaluation.margin, &evaluation.derivative)
                    .map_err(BoundaryReason::Backend)?,
            )
            .map_err(BoundaryReason::Backend)?;
        let candidate = math.intersection(&bracket, &newton).ok();
        let (bisected, bisected_event) =
            bisect_transition(&math, params, &formulas, &ray, &bracket, event)?;
        if let Some(candidate) = candidate.filter(|value| strictly_inside(value, &bracket)) {
            bracket = candidate;
        } else {
            bracket = bisected;
            event = bisected_event;
        }
        let stopping_exponent = i32::try_from(budgets.precision.min(1_024))
            .map_err(|_| BoundaryReason::InvalidBudget)?
            / 2;
        if width_f64(&math, &bracket)? <= 2_f64.powi(-stopping_exponent) {
            break;
        }
    }

    let final_evaluation = evaluate_event(&math, params, &formulas, &ray, &bracket, event)?;
    if final_evaluation.derivative.contains_zero() {
        return Err(BoundaryReason::TangentRoot);
    }
    if !final_evaluation.earlier_positive {
        return Err(BoundaryReason::EarlierEvent);
    }
    if final_evaluation.simultaneous {
        return Err(BoundaryReason::SimultaneousEvent);
    }
    let uniqueness_evaluation = evaluate_event(&math, params, &formulas, &ray, &bracket, event)?;
    if uniqueness_evaluation.derivative.contains_zero() {
        return Err(BoundaryReason::TangentRoot);
    }
    let root_midpoint = midpoint(&math, &bracket)?;
    let midpoint_evaluation =
        evaluate_event(&math, params, &formulas, &ray, &root_midpoint, event)?;
    let newton_image = math
        .sub(
            &root_midpoint,
            &math
                .div(
                    &midpoint_evaluation.margin,
                    &uniqueness_evaluation.derivative,
                )
                .map_err(BoundaryReason::Backend)?,
        )
        .map_err(BoundaryReason::Backend)?;
    // The endpoint classifications establish existence, while the derivative
    // interval excluding zero establishes strict monotonicity. Interval Newton
    // containment therefore proves the one existing root is unique even when
    // the image touches a represented bracket endpoint.
    if !bracket.contains(&newton_image) {
        #[cfg(test)]
        eprintln!(
            "interval Newton miss: bracket [{:.17e}, {:.17e}], image [{:.17e}, {:.17e}], width {:.3e}",
            bracket.lower().to_f64().value(),
            bracket.upper().to_f64().value(),
            newton_image.lower().to_f64().value(),
            newton_image.upper().to_f64().value(),
            width_f64(&math, &bracket)?
        );
        return Err(BoundaryReason::RootNotUnique);
    }

    let sigma_min_lower = camera_plane_sigma_min_lower(
        &math,
        &final_evaluation.derivative_matrix,
        ray.right,
        ray.up,
    )?;
    let frame_radius = 10_f64.powf(-params.zoom_exp);
    if !sigma_min_lower.is_finite() || sigma_min_lower * frame_radius < 1.0 {
        return Err(BoundaryReason::ResolutionGate {
            sigma_min_lower,
            required: frame_radius.recip(),
        });
    }

    let outward_witness = represented_lower(&math, &bracket)?;
    let inward_witness = represented_upper(&math, &bracket)?;
    if !matches!(
        classify(&math, params, &formulas, &ray, &outward_witness)?,
        Membership::Outside { .. }
    ) || classify(&math, params, &formulas, &ray, &inward_witness)? != Membership::Inside
    {
        return Err(BoundaryReason::RootNotUnique);
    }
    let target = ray_point(&math, &ray, &inward_witness)?;
    let decimal_precision = budgets.precision.saturating_mul(31).div_ceil(100) + 8;
    Ok(CertifiedBoundary {
        target: target.map(|value| {
            value
                .lower()
                .clone()
                .with_base_and_precision::<10>(decimal_precision)
                .value()
                .to_string()
        }),
        event_iteration: event.iteration,
        effective_iterations: params.max_iters,
        bracket_width: width_f64(&math, &bracket)?,
        sigma_min_lower,
    })
}

fn validate_request(
    params: &StackParams,
    camera: ParkedCamera,
    budgets: BoundaryBudgets,
) -> Result<(), BoundaryReason> {
    validate(params).map_err(map_stack_error)?;
    if budgets.precision == 0
        || budgets.precision > MAX_PRECISION
        || budgets.max_segments == 0
        || budgets.max_newton_steps == 0
    {
        return Err(BoundaryReason::InvalidBudget);
    }
    if !camera.distance.is_finite()
        || camera.distance <= 0.0
        || !camera.azimuth.is_finite()
        || !camera.elevation.is_finite()
        || !camera.look.iter().all(|value| value.is_finite())
    {
        return Err(BoundaryReason::InvalidCamera);
    }
    Ok(())
}

/// Builds the animation-independent ray shared by targeting strategies.
pub(crate) fn parked_ray_geometry(camera: ParkedCamera) -> Result<ParkedRay, BoundaryReason> {
    if !camera.distance.is_finite()
        || camera.distance <= 0.0
        || !camera.azimuth.is_finite()
        || !camera.elevation.is_finite()
        || !camera.look.iter().all(|value| value.is_finite())
    {
        return Err(BoundaryReason::InvalidCamera);
    }
    let (sin_azimuth, cos_azimuth) = camera.azimuth.sin_cos();
    let (sin_elevation, cos_elevation) = camera.elevation.sin_cos();
    let orbit = [
        cos_azimuth * cos_elevation,
        sin_elevation,
        sin_azimuth * cos_elevation,
    ];
    let origin_f64 = orbit.map(|value| value * camera.distance);
    let aim = camera.look.map(|value| value * 0.5);
    let delta = std::array::from_fn(|axis| aim[axis] - origin_f64[axis]);
    let length = delta.iter().map(|value| value * value).sum::<f64>().sqrt();
    if !length.is_finite() || length < 1e-12 {
        return Err(BoundaryReason::InvalidCamera);
    }
    let direction_f64 = delta.map(|value| value / length);
    let forward = direction_f64;
    let mut right = cross([0.0, 1.0, 0.0], forward);
    let right_length = norm(right);
    right = if right_length > 1e-12 {
        right.map(|value| value / right_length)
    } else {
        [1.0, 0.0, 0.0]
    };
    let up = cross(forward, right);
    Ok(ParkedRay {
        origin: origin_f64,
        direction: direction_f64,
        length,
        right,
        up,
    })
}

fn parked_ray(math: &IntervalMath, camera: ParkedCamera) -> Result<Ray, BoundaryReason> {
    let geometry = parked_ray_geometry(camera)?;
    Ok(Ray {
        origin: geometry
            .origin
            .map(|value| math.point_f64(value).map_err(BoundaryReason::Backend))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| BoundaryReason::Backend(IntervalError::InvalidBounds))?,
        direction: geometry
            .direction
            .map(|value| math.point_f64(value).map_err(BoundaryReason::Backend))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| BoundaryReason::Backend(IntervalError::InvalidBounds))?,
        length: math
            .point_f64(geometry.length)
            .map_err(BoundaryReason::Backend)?,
        right: geometry.right,
        up: geometry.up,
    })
}

fn isolate_first_transition(
    math: &IntervalMath,
    params: &StackParams,
    formulas: &[u8],
    ray: &Ray,
    lower: &BigInterval,
    upper: &BigInterval,
    budgets: BoundaryBudgets,
) -> Result<(BigInterval, EscapeEvent), BoundaryReason> {
    let mut pending = vec![(lower.clone(), upper.clone(), 0_usize)];
    let mut visited = 0_usize;
    while let Some((left, right, depth)) = pending.pop() {
        visited += 1;
        if visited > budgets.max_segments {
            return Err(BoundaryReason::SegmentBudget);
        }
        let segment = math.hull(&left, &right).map_err(BoundaryReason::Backend)?;
        if !matches!(
            classify_for_isolation(math, params, formulas, ray, &segment)?,
            Membership::Unresolved
        ) {
            continue;
        }
        let left_state = classify_for_isolation(math, params, formulas, ray, &left)?;
        let right_state = classify_for_isolation(math, params, formulas, ray, &right)?;
        if let (Membership::Outside { event }, Membership::Inside) = (left_state, right_state) {
            return Ok((
                math.hull(&left, &right).map_err(BoundaryReason::Backend)?,
                event,
            ));
        }
        if depth >= budgets.max_depth {
            continue;
        }
        let middle = midpoint_between(math, &left, &right)?;
        // LIFO, so the front half is always exhausted before the back half.
        pending.push((middle.clone(), right, depth + 1));
        pending.push((left, middle, depth + 1));
    }
    Err(BoundaryReason::NoTransition)
}

fn classify_for_isolation(
    math: &IntervalMath,
    params: &StackParams,
    formulas: &[u8],
    ray: &Ray,
    t: &BigInterval,
) -> Result<Membership, BoundaryReason> {
    match classify(math, params, formulas, ray, t) {
        Ok(membership) => Ok(membership),
        // These errors mean the current box crosses a chart or formula
        // singularity. They are not evidence that every narrower child does,
        // so isolation must subdivide. Certification still calls `classify`
        // and `evaluate_event` directly and therefore fails closed if the
        // selected root box itself retains any of these ambiguities.
        Err(reason) if is_subdividable_topology(reason) => Ok(Membership::Unresolved),
        Err(reason) => Err(reason),
    }
}

fn is_subdividable_topology(reason: BoundaryReason) -> bool {
    matches!(
        reason,
        BoundaryReason::AzimuthSeam
            | BoundaryReason::PolarAxis
            | BoundaryReason::RadiusRegularization
            | BoundaryReason::FormulaBranch
            | BoundaryReason::MengerTranslation
    )
}

fn bisect_transition(
    math: &IntervalMath,
    params: &StackParams,
    formulas: &[u8],
    ray: &Ray,
    bracket: &BigInterval,
    event: EscapeEvent,
) -> Result<(BigInterval, EscapeEvent), BoundaryReason> {
    let left = represented_lower(math, bracket)?;
    let right = represented_upper(math, bracket)?;
    let middle = midpoint(math, bracket)?;
    match classify(math, params, formulas, ray, &middle)? {
        Membership::Outside { event: found } => Ok((
            math.hull(&middle, &right)
                .map_err(BoundaryReason::Backend)?,
            found,
        )),
        Membership::Inside => Ok((
            math.hull(&left, &middle).map_err(BoundaryReason::Backend)?,
            event,
        )),
        Membership::Unresolved => Ok((bracket.clone(), event)),
    }
}

fn classify(
    math: &IntervalMath,
    params: &StackParams,
    formulas: &[u8],
    ray: &Ray,
    t: &BigInterval,
) -> Result<Membership, BoundaryReason> {
    let parameter = ray_point(math, ray, t)?;
    let mut state = parameter.clone();
    let bailout_squared = math
        .point_f64(params.bailout * params.bailout)
        .map_err(BoundaryReason::Backend)?;
    let bulb_guard = math.point_f64(4.0).map_err(BoundaryReason::Backend)?;
    let mut crossed_escape_surface = false;
    for iteration in 1..=params.max_iters {
        let formula = formulas[(iteration - 1) % formulas.len()];
        if formula == 5 {
            let norm_squared = norm_squared_intervals(math, &state)?;
            if norm_squared.lower().repr() > bulb_guard.upper().repr() {
                return Ok(Membership::Outside {
                    event: EscapeEvent {
                        iteration,
                        kind: EventKind::MandelbulbPreGuard,
                    },
                });
            }
            if norm_squared.upper().repr() >= bulb_guard.lower().repr() {
                crossed_escape_surface = true;
            }
        }
        let chart = if formula == 5 {
            chart_for(&state)?
        } else {
            Atan2Chart::Principal
        };
        let evaluation = evaluate_slot(math, &IntervalBox3::new(state), formula, params, chart)
            .map_err(map_stack_error)?;
        state = add_seed(
            math,
            &evaluation.components,
            &parameter,
            params,
            evaluation.seed_weight,
        )?;
        let norm_squared = norm_squared_intervals(math, &state)?;
        if norm_squared.lower().repr() > bailout_squared.upper().repr() {
            return Ok(Membership::Outside {
                event: EscapeEvent {
                    iteration,
                    kind: EventKind::PostSlotBailout,
                },
            });
        }
        if norm_squared.upper().repr() >= bailout_squared.lower().repr() {
            crossed_escape_surface = true;
        }
    }
    if crossed_escape_surface {
        Ok(Membership::Unresolved)
    } else {
        Ok(Membership::Inside)
    }
}

fn evaluate_event(
    math: &IntervalMath,
    params: &StackParams,
    formulas: &[u8],
    ray: &Ray,
    t: &BigInterval,
    event: EscapeEvent,
) -> Result<EventEvaluation, BoundaryReason> {
    let parameter = ray_point(math, ray, t)?;
    let mut state = parameter.clone();
    let mut derivative = identity(math);
    let direction = [
        math.mul(&ray.direction[0], &ray.length)
            .map_err(BoundaryReason::Backend)?,
        math.mul(&ray.direction[1], &ray.length)
            .map_err(BoundaryReason::Backend)?,
        math.mul(&ray.direction[2], &ray.length)
            .map_err(BoundaryReason::Backend)?,
    ];
    let bailout_squared = math
        .point_f64(params.bailout * params.bailout)
        .map_err(BoundaryReason::Backend)?;
    let bulb_guard = math.point_f64(4.0).map_err(BoundaryReason::Backend)?;
    let mut earlier_positive = true;
    let mut simultaneous = false;

    for iteration in 1..=event.iteration {
        let formula = formulas[(iteration - 1) % formulas.len()];
        if formula == 5 {
            let pre_margin = math
                .sub(&bulb_guard, &norm_squared_intervals(math, &state)?)
                .map_err(BoundaryReason::Backend)?;
            if iteration == event.iteration && event.kind == EventKind::MandelbulbPreGuard {
                return event_from_state(
                    math,
                    pre_margin,
                    &state,
                    &derivative,
                    &direction,
                    earlier_positive,
                    simultaneous,
                );
            }
            earlier_positive &= pre_margin.strictly_positive();
        }
        let chart = if formula == 5 {
            chart_for(&state)?
        } else {
            Atan2Chart::Principal
        };
        let evaluation = evaluate_slot(math, &IntervalBox3::new(state), formula, params, chart)
            .map_err(map_stack_error)?;
        let jacobian = std::array::from_fn(|row| evaluation.components[row].gradient().clone());
        derivative = matrix_add(
            math,
            &matrix_mul(math, &jacobian, &derivative)?,
            &scaled_identity(math, evaluation.seed_weight)?,
        )?;
        state = add_seed(
            math,
            &evaluation.components,
            &parameter,
            params,
            evaluation.seed_weight,
        )?;
        let post_margin = math
            .sub(&bailout_squared, &norm_squared_intervals(math, &state)?)
            .map_err(BoundaryReason::Backend)?;
        if iteration == event.iteration && event.kind == EventKind::PostSlotBailout {
            return event_from_state(
                math,
                post_margin,
                &state,
                &derivative,
                &direction,
                earlier_positive,
                simultaneous,
            );
        }
        if !post_margin.strictly_positive() {
            earlier_positive = false;
        }
        simultaneous |= post_margin.contains_zero();
    }
    Err(BoundaryReason::RootNotUnique)
}

fn event_from_state(
    math: &IntervalMath,
    margin: BigInterval,
    state: &[BigInterval; DIMENSIONS],
    derivative_matrix: &Matrix3,
    direction: &[BigInterval; DIMENSIONS],
    earlier_positive: bool,
    simultaneous: bool,
) -> Result<EventEvaluation, BoundaryReason> {
    let tangent = matrix_vector(math, derivative_matrix, direction)?;
    let mut dot = math.point_f64(0.0).map_err(BoundaryReason::Backend)?;
    for axis in 0..DIMENSIONS {
        dot = math
            .add(
                &dot,
                &math
                    .mul(&state[axis], &tangent[axis])
                    .map_err(BoundaryReason::Backend)?,
            )
            .map_err(BoundaryReason::Backend)?;
    }
    let derivative = math
        .mul(
            &math.point_f64(-2.0).map_err(BoundaryReason::Backend)?,
            &dot,
        )
        .map_err(BoundaryReason::Backend)?;
    Ok(EventEvaluation {
        margin,
        derivative,
        earlier_positive,
        simultaneous,
        derivative_matrix: derivative_matrix.clone(),
    })
}

fn camera_plane_sigma_min_lower(
    math: &IntervalMath,
    derivative: &Matrix3,
    right: [f64; DIMENSIONS],
    up: [f64; DIMENSIONS],
) -> Result<f64, BoundaryReason> {
    let right = right.map(|value| math.point_f64(value).map_err(BoundaryReason::Backend));
    let up = up.map(|value| math.point_f64(value).map_err(BoundaryReason::Backend));
    let right: [BigInterval; DIMENSIONS] = right
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| BoundaryReason::Backend(IntervalError::InvalidBounds))?;
    let up: [BigInterval; DIMENSIONS] =
        up.into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| BoundaryReason::Backend(IntervalError::InvalidBounds))?;
    let a = matrix_vector(math, derivative, &right)?;
    let b = matrix_vector(math, derivative, &up)?;
    let aa = dot_intervals(math, &a, &a)?;
    let bb = dot_intervals(math, &b, &b)?;
    let ab = dot_intervals(math, &a, &b)?;
    let trace = math.add(&aa, &bb).map_err(BoundaryReason::Backend)?;
    let difference = math.sub(&aa, &bb).map_err(BoundaryReason::Backend)?;
    let discriminant = math
        .sqrt(
            &math
                .add(
                    &math.square(&difference).map_err(BoundaryReason::Backend)?,
                    &math
                        .mul(
                            &math.point_f64(4.0).map_err(BoundaryReason::Backend)?,
                            &math.square(&ab).map_err(BoundaryReason::Backend)?,
                        )
                        .map_err(BoundaryReason::Backend)?,
                )
                .map_err(BoundaryReason::Backend)?,
        )
        .map_err(BoundaryReason::Backend)?;
    let lambda_min = math
        .div(
            &math
                .sub(&trace, &discriminant)
                .map_err(BoundaryReason::Backend)?,
            &math.point_f64(2.0).map_err(BoundaryReason::Backend)?,
        )
        .map_err(BoundaryReason::Backend)?;
    if lambda_min.lower().repr() <= &Repr::zero() {
        return Ok(0.0);
    }
    Ok(lambda_min.lower().to_f64().value().sqrt())
}

#[allow(
    clippy::float_cmp,
    reason = "zero is an exact slot sentinel for formulas with no seed"
)]
fn add_seed(
    math: &IntervalMath,
    components: &[Jet2; DIMENSIONS],
    parameter: &[BigInterval; DIMENSIONS],
    params: &StackParams,
    seed_weight: f64,
) -> Result<[BigInterval; DIMENSIONS], BoundaryReason> {
    if seed_weight == 0.0 {
        return Ok(std::array::from_fn(|axis| components[axis].value().clone()));
    }
    let parameter_weight = math
        .point_f64(seed_weight)
        .map_err(BoundaryReason::Backend)?;
    let julia_weight = math
        .point_f64(1.0 - seed_weight)
        .map_err(BoundaryReason::Backend)?;
    let component = |axis: usize| {
        let seed = math
            .add(
                &math
                    .mul(&parameter[axis], &parameter_weight)
                    .map_err(BoundaryReason::Backend)?,
                &math
                    .mul(
                        &math
                            .point_f64(params.julia[axis])
                            .map_err(BoundaryReason::Backend)?,
                        &julia_weight,
                    )
                    .map_err(BoundaryReason::Backend)?,
            )
            .map_err(BoundaryReason::Backend)?;
        math.add(components[axis].value(), &seed)
            .map_err(BoundaryReason::Backend)
    };
    Ok([component(0)?, component(1)?, component(2)?])
}

fn ray_point(
    math: &IntervalMath,
    ray: &Ray,
    t: &BigInterval,
) -> Result<[BigInterval; DIMENSIONS], BoundaryReason> {
    let distance = math.mul(t, &ray.length).map_err(BoundaryReason::Backend)?;
    let component = |axis: usize| {
        math.add(
            &ray.origin[axis],
            &math
                .mul(&ray.direction[axis], &distance)
                .map_err(BoundaryReason::Backend)?,
        )
        .map_err(BoundaryReason::Backend)
    };
    Ok([component(0)?, component(1)?, component(2)?])
}

fn norm_squared_intervals(
    math: &IntervalMath,
    point: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, BoundaryReason> {
    dot_intervals(math, point, point)
}

fn dot_intervals(
    math: &IntervalMath,
    left: &[BigInterval; DIMENSIONS],
    right: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, BoundaryReason> {
    let mut sum = math.point_f64(0.0).map_err(BoundaryReason::Backend)?;
    for axis in 0..DIMENSIONS {
        sum = math
            .add(
                &sum,
                &math
                    .mul(&left[axis], &right[axis])
                    .map_err(BoundaryReason::Backend)?,
            )
            .map_err(BoundaryReason::Backend)?;
    }
    Ok(sum)
}

fn identity(math: &IntervalMath) -> Matrix3 {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            math.point_f64(f64::from(row == column))
                .expect("zero and one are finite")
        })
    })
}

fn scaled_identity(math: &IntervalMath, scale: f64) -> Result<Matrix3, BoundaryReason> {
    let zero = math.point_f64(0.0).map_err(BoundaryReason::Backend)?;
    let scale = math.point_f64(scale).map_err(BoundaryReason::Backend)?;
    Ok(std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            if row == column {
                scale.clone()
            } else {
                zero.clone()
            }
        })
    }))
}

fn matrix_add(
    math: &IntervalMath,
    left: &Matrix3,
    right: &Matrix3,
) -> Result<Matrix3, BoundaryReason> {
    let entry = |row: usize, column: usize| {
        math.add(&left[row][column], &right[row][column])
            .map_err(BoundaryReason::Backend)
    };
    Ok([
        [entry(0, 0)?, entry(0, 1)?, entry(0, 2)?],
        [entry(1, 0)?, entry(1, 1)?, entry(1, 2)?],
        [entry(2, 0)?, entry(2, 1)?, entry(2, 2)?],
    ])
}

fn matrix_mul(
    math: &IntervalMath,
    left: &Matrix3,
    right: &Matrix3,
) -> Result<Matrix3, BoundaryReason> {
    let entry = |row: usize, column: usize| -> Result<BigInterval, BoundaryReason> {
        let products = [
            math.mul(&left[row][0], &right[0][column])
                .map_err(BoundaryReason::Backend)?,
            math.mul(&left[row][1], &right[1][column])
                .map_err(BoundaryReason::Backend)?,
            math.mul(&left[row][2], &right[2][column])
                .map_err(BoundaryReason::Backend)?,
        ];
        math.add(
            &math
                .add(&products[0], &products[1])
                .map_err(BoundaryReason::Backend)?,
            &products[2],
        )
        .map_err(BoundaryReason::Backend)
    };
    Ok([
        [entry(0, 0)?, entry(0, 1)?, entry(0, 2)?],
        [entry(1, 0)?, entry(1, 1)?, entry(1, 2)?],
        [entry(2, 0)?, entry(2, 1)?, entry(2, 2)?],
    ])
}

fn matrix_vector(
    math: &IntervalMath,
    matrix: &Matrix3,
    vector: &[BigInterval; DIMENSIONS],
) -> Result<[BigInterval; DIMENSIONS], BoundaryReason> {
    let row = |row: usize| {
        let mut sum = math.point_f64(0.0).map_err(BoundaryReason::Backend)?;
        for column in 0..DIMENSIONS {
            sum = math
                .add(
                    &sum,
                    &math
                        .mul(&matrix[row][column], &vector[column])
                        .map_err(BoundaryReason::Backend)?,
                )
                .map_err(BoundaryReason::Backend)?;
        }
        Ok(sum)
    };
    Ok([row(0)?, row(1)?, row(2)?])
}

fn chart_for(point: &[BigInterval; DIMENSIONS]) -> Result<Atan2Chart, BoundaryReason> {
    let [x, y, _] = point;
    if x.contains_zero() && y.contains_zero() {
        return Err(BoundaryReason::PolarAxis);
    }
    if x.lower().repr() < &Repr::zero() && y.contains_zero() {
        if y.lower().repr() >= &Repr::zero() {
            Ok(Atan2Chart::Upper)
        } else if y.upper().repr() <= &Repr::zero() {
            Ok(Atan2Chart::Lower)
        } else {
            Err(BoundaryReason::AzimuthSeam)
        }
    } else {
        Ok(Atan2Chart::Principal)
    }
}

fn midpoint(math: &IntervalMath, interval: &BigInterval) -> Result<BigInterval, BoundaryReason> {
    midpoint_between(
        math,
        &represented_lower(math, interval)?,
        &represented_upper(math, interval)?,
    )
}

fn midpoint_between(
    math: &IntervalMath,
    lower: &BigInterval,
    upper: &BigInterval,
) -> Result<BigInterval, BoundaryReason> {
    let sum = math.add(lower, upper).map_err(BoundaryReason::Backend)?;
    let half = math.point_f64(0.5).map_err(BoundaryReason::Backend)?;
    let enclosure = math.mul(&sum, &half).map_err(BoundaryReason::Backend)?;
    represented_lower(math, &enclosure)
}

fn represented_lower(
    math: &IntervalMath,
    value: &BigInterval,
) -> Result<BigInterval, BoundaryReason> {
    let repr = value.lower().repr().clone();
    BigInterval::checked(
        FBig::<_, 2>::from_repr(repr.clone(), math.down),
        FBig::<_, 2>::from_repr(repr, math.up),
    )
    .map_err(BoundaryReason::Backend)
}

fn represented_upper(
    math: &IntervalMath,
    value: &BigInterval,
) -> Result<BigInterval, BoundaryReason> {
    let repr = value.upper().repr().clone();
    BigInterval::checked(
        FBig::<_, 2>::from_repr(repr.clone(), math.down),
        FBig::<_, 2>::from_repr(repr, math.up),
    )
    .map_err(BoundaryReason::Backend)
}

fn strictly_inside(inner: &BigInterval, outer: &BigInterval) -> bool {
    inner.lower().repr() > outer.lower().repr() && inner.upper().repr() < outer.upper().repr()
}

fn width_f64(math: &IntervalMath, interval: &BigInterval) -> Result<f64, BoundaryReason> {
    Ok(math
        .width(interval)
        .map_err(BoundaryReason::Backend)?
        .upper()
        .to_f64()
        .value())
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn norm(value: [f64; 3]) -> f64 {
    value
        .iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt()
}

fn map_stack_error(error: StackEvaluationError) -> BoundaryReason {
    match error {
        StackEvaluationError::InvalidFormula
        | StackEvaluationError::InvalidParameter
        | StackEvaluationError::Mandelbulb(MandelbulbError::OutOfRangePower) => {
            BoundaryReason::InvalidStack
        }
        StackEvaluationError::ContinuousBranch => BoundaryReason::FormulaBranch,
        StackEvaluationError::MengerTranslation => BoundaryReason::MengerTranslation,
        StackEvaluationError::Mandelbulb(MandelbulbError::AzimuthSeam) => {
            BoundaryReason::AzimuthSeam
        }
        StackEvaluationError::Mandelbulb(MandelbulbError::PolarAxis) => BoundaryReason::PolarAxis,
        StackEvaluationError::Mandelbulb(MandelbulbError::RadiusRegularization) => {
            BoundaryReason::RadiusRegularization
        }
        StackEvaluationError::Mandelbulb(MandelbulbError::Backend(error))
        | StackEvaluationError::Backend(error) => BoundaryReason::Backend(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> ParkedCamera {
        ParkedCamera {
            distance: 4.2,
            azimuth: 0.0,
            elevation: 0.2,
            look: [0.0; 3],
        }
    }

    fn budgets() -> BoundaryBudgets {
        BoundaryBudgets {
            precision: 128,
            max_depth: 18,
            max_segments: 1 << 19,
            max_newton_steps: 32,
        }
    }

    #[test]
    fn parked_camera_ignores_animation_by_construction() {
        let math = IntervalMath::new(128).unwrap();
        let ray = parked_ray(&math, camera()).unwrap();
        assert!(ray.origin[0].lower().to_f64().value() > 4.0);
        assert_eq!(ray.right, [0.0, -0.0, 1.0]);
    }

    #[test]
    fn tangent_fixture_is_typed_inconclusive() {
        let params = StackParams {
            formulas: [0, 0, 0, 0],
            rates: [1, 0, 0, 0],
            bailout: 4.2,
            max_iters: 1,
            ..StackParams::default()
        };
        assert!(matches!(
            certify_camera_boundary(&params, camera(), budgets()),
            BoundaryResult::Inconclusive(_)
        ));
    }

    #[test]
    fn regularization_and_both_seam_charts_fail_closed() {
        let math = IntervalMath::new(128).unwrap();
        let negative_x = math.bounds_f64(-2.0, -1.0).unwrap();
        let z = math.point_f64(0.5).unwrap();
        assert_eq!(
            chart_for(&[
                negative_x.clone(),
                math.bounds_f64(-1e-20, 1e-20).unwrap(),
                z.clone(),
            ]),
            Err(BoundaryReason::AzimuthSeam)
        );
        assert_eq!(
            chart_for(&[
                negative_x.clone(),
                math.bounds_f64(0.0, 1e-20).unwrap(),
                z.clone(),
            ]),
            Ok(Atan2Chart::Upper)
        );
        assert_eq!(
            chart_for(&[negative_x, math.bounds_f64(-1e-20, -0.0).unwrap(), z,]),
            Ok(Atan2Chart::Lower)
        );
        assert_eq!(
            map_stack_error(StackEvaluationError::Mandelbulb(
                MandelbulbError::RadiusRegularization
            )),
            BoundaryReason::RadiusRegularization
        );
    }

    #[test]
    fn mixed_cycle_preserves_phase_and_order() {
        let params = StackParams {
            formulas: [8, 7, 5, 3],
            rates: [2, 1, 1, 1],
            ..StackParams::default()
        };
        assert_eq!(cycle(&params).unwrap(), [8, 8, 7, 5, 3]);
    }

    #[test]
    fn nearest_transition_wins_on_a_multi_crossing_ray() {
        let params = StackParams {
            formulas: [0, 0, 0, 0],
            rates: [1, 0, 0, 0],
            bailout: 1.0,
            max_iters: 1,
            zoom_exp: 0.0,
            ..StackParams::default()
        };
        let crossing_camera = ParkedCamera {
            distance: 4.2,
            azimuth: 0.0,
            elevation: 0.0,
            look: [-8.4, 0.0, 0.0],
        };
        let BoundaryResult::Certified(boundary) =
            certify_camera_boundary(&params, crossing_camera, budgets())
        else {
            panic!("nearest crossing was not certified");
        };
        let x: f64 = boundary.target[0].parse().unwrap();
        assert!(
            x > 0.999 && x < 1.001,
            "selected the farther crossing at x={x}"
        );
        assert_eq!(boundary.event_iteration, 1);
    }

    fn preview_bulb(zoom_exp: f64) -> StackParams {
        StackParams {
            formulas: [5, 5, 8, 9],
            rates: [1, 0, 0, 0],
            power: 8.0,
            zoom_exp,
            max_iters: 512,
            refine: true,
            cam_dist: 4.2,
            cam_azim: 0.0,
            cam_elev: 0.2,
            look: [0.0; 3],
            ..StackParams::default()
        }
    }

    /// Depth measurement for the finite-solid boundary at zoom exponent 6.
    ///
    /// Ignored because it costs about 78 seconds alone and around 169 under suite
    /// contention, which made it the entire wall time of `cargo test --lib`, and
    /// because what it measures is depth rather than behaviour. The behaviour of
    /// `certify_camera_boundary` stays covered in the default run by
    /// `tangent_fixture_is_typed_inconclusive`,
    /// `regularization_and_both_seam_charts_fail_closed`, and
    /// `nearest_transition_wins_on_a_multi_crossing_ray`. Production does not call
    /// this function at all: the Mandelbulb path uses `search_long_lived_anchor`,
    /// and the paper lists "a finite-$S_N$ boundary witness is automatically a
    /// useful zoom anchor" as disproved.
    ///
    /// ```sh
    /// cargo test --lib finite_solid_boundary_remains_available -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measurement: ~78s of arbitrary-precision subdivision"]
    fn finite_solid_boundary_remains_available_for_safe_step_work_at_zoom_6() {
        let started = std::time::Instant::now();
        let params = preview_bulb(6.0);
        let result = certify_camera_boundary(
            &params,
            camera(),
            BoundaryBudgets {
                precision: 152,
                max_depth: 152,
                max_segments: 1_024,
                max_newton_steps: 64,
            },
        );
        eprintln!("zoom 6: {result:?} in {:?}", started.elapsed());
        let BoundaryResult::Certified(boundary) = result else {
            panic!("zoom 6 fixture was not certified");
        };
        // This finite-level boundary remains useful to the later safe-step
        // certificate path. It is deliberately no longer the zoom anchor.
        assert!(
            boundary.effective_iterations > 10,
            "settled for the resolution minimum: {}",
            boundary.effective_iterations
        );
        assert_eq!(boundary.event_iteration, boundary.effective_iterations);
        assert!(boundary.sigma_min_lower >= 1.0e6);
        assert!(started.elapsed() < std::time::Duration::from_secs(120));
    }

    /// Zoom 100 currently reports a typed inconclusive rather than certifying.
    ///
    /// Ignored for two reasons. It costs about 173 seconds, which was the entire
    /// wall time of `cargo test --lib`, and it pins a verdict we now know to be
    /// removable rather than fundamental. `PolarAxis` is raised whenever a cell's
    /// horizontal radius encloses zero, because the azimuth chart and the
    /// `sin(theta)` denominators degenerate there. The map's singular values stay
    /// bounded by `p^2 r^(p-1)` across the axis at integer power, since the
    /// anisotropy kernel `sin(p theta)/sin(theta)` is the Chebyshev polynomial
    /// `U_{p-1}(cos theta)` with `|U_{p-1}| <= p`, so a uniform norm bound is
    /// available where this evaluator refuses. Certification needs that bound,
    /// not a derivative limit, and the individual Jacobian entries genuinely do
    /// not converge on the axis. See `notes/prototypes/generalized_group_power.py`
    /// and the generalization section of the paper.
    ///
    /// Re-enable once the evaluator carries the Chebyshev kernel; the expected
    /// verdict should then change rather than merely get faster.
    ///
    /// ```sh
    /// cargo test --lib finite_solid_boundary_zoom_100 -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measurement: ~173s, and pins a removable polar-axis artifact"]
    fn finite_solid_boundary_zoom_100_stays_typed_inconclusive() {
        let started = std::time::Instant::now();
        let params = preview_bulb(100.0);
        let result = certify_camera_boundary(
            &params,
            camera(),
            BoundaryBudgets {
                precision: 528,
                max_depth: 528,
                max_segments: 1_024,
                max_newton_steps: 16,
            },
        );
        eprintln!("zoom 100: {result:?} in {:?}", started.elapsed());
        assert_eq!(
            result,
            BoundaryResult::Inconclusive(BoundaryReason::PolarAxis)
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(120));
    }
}
