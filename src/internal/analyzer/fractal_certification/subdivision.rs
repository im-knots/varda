//! Bounded oriented parameter-cell subdivision for Mandelbulb exclusion.

use std::collections::VecDeque;

use dashu_float::{FBig, Repr};

use super::exclusion::{
    identity, matrix_add, matrix_mul, matrix_sub, represented_lower_endpoint,
    represented_point_matrix, represented_point_vector, taylor_nonlinear_bound,
    vector_distance_upper, vector_norm, Matrix3, Tensor3,
};
use super::jet::Jet2;
use super::mandelbulb::{
    evaluate_with_chart, hessian_tensor_upper_bound, matrix_spectral_upper_bound, IntervalBox3,
    MandelbulbError,
};
use super::stack::{
    cycle as stack_cycle, evaluate_slot, validate as validate_stack, StackEvaluationError,
};
use super::{Atan2Chart, BigInterval, IntervalError, IntervalMath};
use crate::internal::analyzer::fractal_reference_orbit::StackParams;

const DIMENSIONS: usize = 3;
const MAX_PRECISION: usize = 4_096;

/// Hard termination limits for one subdivision request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SubdivisionBudgets {
    pub(crate) max_depth: usize,
    pub(crate) max_leaves: usize,
    pub(crate) max_orbit_steps: usize,
    pub(crate) precision: usize,
}

/// Decimal-directed root cube and Mandelbulb parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SubdivisionRequest<'a> {
    pub(crate) center: [&'a str; DIMENSIONS],
    pub(crate) radius: &'a str,
    pub(crate) power: f64,
    pub(crate) bailout: &'a str,
    pub(crate) max_iterations: usize,
    pub(crate) max_enclosure_radius: &'a str,
    pub(crate) budgets: SubdivisionBudgets,
}

/// Published empty parameter cell. Lower-chart seam faces are half-open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CertifiedCell {
    pub(crate) bounds: [BigInterval; DIMENSIONS],
    pub(crate) center: [BigInterval; DIMENSIONS],
    pub(crate) half_extents: [BigInterval; DIMENSIONS],
    pub(crate) chart: Atan2Chart,
    pub(crate) includes_upper_seam: bool,
    pub(crate) escaped_at: usize,
}

/// Typed reason that prevented complete root-cell certification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubdivisionReason {
    MalformedInput,
    InvalidRadius,
    InvalidBailout,
    InvalidBudget,
    OutOfRangePower,
    SplitBudget,
    LeafBudget,
    OrbitStepBudget,
    IterationBudget,
    AzimuthSeam,
    PolarAxis,
    RadiusRegularization,
    InvalidStack,
    FormulaBranch,
    MengerTranslation,
    Vacuous,
    Backend(IntervalError),
}

/// Complete certification of every retained leaf, or no positive output.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SubdivisionResult {
    Excluded { cells: Vec<CertifiedCell> },
    Inconclusive(SubdivisionReason),
}

#[derive(Clone, Debug)]
struct ParameterCell {
    bounds: [BigInterval; DIMENSIONS],
    depth: usize,
    chart: Atan2Chart,
    includes_upper_seam: bool,
}

struct StackContract<'a> {
    params: Option<&'a StackParams>,
    cycle: Option<Vec<u8>>,
}

enum CellAttempt {
    Excluded { escaped_at: usize },
    Split,
    Inconclusive(SubdivisionReason),
}

/// Certifies a root cube by retaining every child created by finite subdivision.
pub(crate) fn certify_subdivided(input: &SubdivisionRequest<'_>) -> SubdivisionResult {
    match certify_subdivided_inner(input, None) {
        Ok(cells) => SubdivisionResult::Excluded { cells },
        Err(reason) => SubdivisionResult::Inconclusive(reason),
    }
}

/// Certifies the exact resolved four-slot stack represented by `StackParams`.
pub(crate) fn certify_stack_subdivided(
    input: &SubdivisionRequest<'_>,
    stack: &StackParams,
) -> SubdivisionResult {
    match certify_subdivided_inner(input, Some(stack)) {
        Ok(cells) => SubdivisionResult::Excluded { cells },
        Err(reason) => SubdivisionResult::Inconclusive(reason),
    }
}

fn certify_subdivided_inner(
    input: &SubdivisionRequest<'_>,
    stack: Option<&StackParams>,
) -> Result<Vec<CertifiedCell>, SubdivisionReason> {
    validate_budgets(input)?;
    if let Some(stack) = stack {
        validate_stack(stack).map_err(map_stack_error)?;
    }
    let cycle = stack
        .map(stack_cycle)
        .transpose()
        .map_err(map_stack_error)?;
    let contract = StackContract {
        params: stack,
        cycle,
    };
    let math =
        IntervalMath::new(input.budgets.precision).map_err(|_| SubdivisionReason::InvalidBudget)?;
    let requested_center = [
        parse_decimal(&math, input.center[0])?,
        parse_decimal(&math, input.center[1])?,
        parse_decimal(&math, input.center[2])?,
    ];
    let radius = math
        .point_decimal(input.radius)
        .map_err(|_| SubdivisionReason::InvalidRadius)?;
    if radius.lower().repr() < &Repr::zero() {
        return Err(SubdivisionReason::InvalidRadius);
    }
    let bailout = if let Some(stack) = stack {
        math.point_f64(stack.bailout)
    } else {
        math.point_decimal(input.bailout)
    }
    .map_err(|_| SubdivisionReason::InvalidBailout)?;
    if !bailout.strictly_positive() {
        return Err(SubdivisionReason::InvalidBailout);
    }
    let max_enclosure = math
        .point_decimal(input.max_enclosure_radius)
        .map_err(|_| SubdivisionReason::InvalidBudget)?;
    if !max_enclosure.strictly_positive() {
        return Err(SubdivisionReason::InvalidBudget);
    }
    if stack.is_none() && (!input.power.is_finite() || !(2.0..=12.0).contains(&input.power)) {
        return Err(SubdivisionReason::OutOfRangePower);
    }

    let root_bounds = [
        axis_bounds(&math, &requested_center[0], &radius)?,
        axis_bounds(&math, &requested_center[1], &radius)?,
        axis_bounds(&math, &requested_center[2], &radius)?,
    ];
    let root_widths = [
        math.width(&root_bounds[0])
            .map_err(SubdivisionReason::Backend)?,
        math.width(&root_bounds[1])
            .map_err(SubdivisionReason::Backend)?,
        math.width(&root_bounds[2])
            .map_err(SubdivisionReason::Backend)?,
    ];
    let mut pending = VecDeque::from([ParameterCell {
        bounds: root_bounds,
        depth: 0,
        chart: Atan2Chart::Principal,
        includes_upper_seam: true,
    }]);
    let mut certified = Vec::new();
    let mut orbit_steps = 0_usize;

    while let Some(cell) = pending.pop_front() {
        match certify_cell(
            &math,
            input,
            &contract,
            &cell,
            &bailout,
            &max_enclosure,
            &mut orbit_steps,
        ) {
            CellAttempt::Excluded { escaped_at } => {
                certified.push(publish_cell(&math, &cell, escaped_at)?);
            }
            CellAttempt::Inconclusive(reason) => return Err(reason),
            CellAttempt::Split => {
                if cell.depth >= input.budgets.max_depth {
                    return Err(SubdivisionReason::SplitBudget);
                }
                if pending.len() + certified.len() + 2 > input.budgets.max_leaves {
                    return Err(SubdivisionReason::LeafBudget);
                }
                let (lower, upper) = split_cell(&math, &cell, &root_widths)?;
                pending.push_front(upper);
                pending.push_front(lower);
            }
        }
    }
    Ok(certified)
}

fn validate_budgets(input: &SubdivisionRequest<'_>) -> Result<(), SubdivisionReason> {
    if input.budgets.precision == 0
        || input.budgets.precision > MAX_PRECISION
        || input.budgets.max_leaves == 0
    {
        return Err(SubdivisionReason::InvalidBudget);
    }
    Ok(())
}

fn parse_decimal(math: &IntervalMath, value: &str) -> Result<BigInterval, SubdivisionReason> {
    math.point_decimal(value)
        .map_err(|_| SubdivisionReason::MalformedInput)
}

fn axis_bounds(
    math: &IntervalMath,
    center: &BigInterval,
    radius: &BigInterval,
) -> Result<BigInterval, SubdivisionReason> {
    math.hull(
        &math
            .sub(center, radius)
            .map_err(SubdivisionReason::Backend)?,
        &math
            .add(center, radius)
            .map_err(SubdivisionReason::Backend)?,
    )
    .map_err(SubdivisionReason::Backend)
}

fn certify_cell(
    math: &IntervalMath,
    input: &SubdivisionRequest<'_>,
    contract: &StackContract<'_>,
    cell: &ParameterCell,
    bailout: &BigInterval,
    max_enclosure: &BigInterval,
    orbit_steps: &mut usize,
) -> CellAttempt {
    match certify_cell_inner(
        math,
        input,
        contract,
        cell,
        bailout,
        max_enclosure,
        orbit_steps,
    ) {
        Ok(result) => result,
        Err(reason) => CellAttempt::Inconclusive(reason),
    }
}

fn certify_cell_inner(
    math: &IntervalMath,
    input: &SubdivisionRequest<'_>,
    contract: &StackContract<'_>,
    cell: &ParameterCell,
    bailout: &BigInterval,
    max_enclosure: &BigInterval,
    orbit_steps: &mut usize,
) -> Result<CellAttempt, SubdivisionReason> {
    let parameter_center = cell_center(math, &cell.bounds)?;
    let parameter_offset = [
        math.sub(&cell.bounds[0], &parameter_center[0])
            .map_err(SubdivisionReason::Backend)?,
        math.sub(&cell.bounds[1], &parameter_center[1])
            .map_err(SubdivisionReason::Backend)?,
        math.sub(&cell.bounds[2], &parameter_center[2])
            .map_err(SubdivisionReason::Backend)?,
    ];
    let mut center = parameter_center.clone();
    let mut derivative = identity(math).map_err(SubdivisionReason::Backend)?;
    let mut remainder = math.point_f64(0.0).map_err(SubdivisionReason::Backend)?;

    for iteration in 1..=input.max_iterations {
        if *orbit_steps >= input.budgets.max_orbit_steps {
            return Err(SubdivisionReason::OrbitStepBudget);
        }
        *orbit_steps += 1;

        let state_offset = propagated_offset(math, &derivative, &parameter_offset, &remainder)?;
        let total_radius = offset_radius(math, &state_offset)?;
        if total_radius.upper().repr() > max_enclosure.upper().repr() {
            return Ok(CellAttempt::Inconclusive(SubdivisionReason::Vacuous));
        }
        let domain = IntervalBox3::new([
            math.add(&center[0], &state_offset[0])
                .map_err(SubdivisionReason::Backend)?,
            math.add(&center[1], &state_offset[1])
                .map_err(SubdivisionReason::Backend)?,
            math.add(&center[2], &state_offset[2])
                .map_err(SubdivisionReason::Backend)?,
        ]);
        let formula = contract
            .cycle
            .as_deref()
            .map_or(5, |cycle| cycle[(iteration - 1) % cycle.len()]);
        if contract.params.is_some() && formula == 5 {
            let radius =
                vector_norm(math, domain.coordinates()).map_err(SubdivisionReason::Backend)?;
            let bulb_escape = math.point_f64(2.0).map_err(SubdivisionReason::Backend)?;
            if radius.lower().repr() > bulb_escape.upper().repr() {
                return Ok(CellAttempt::Excluded {
                    escaped_at: iteration,
                });
            }
            if radius.upper().repr() > bulb_escape.lower().repr() {
                return Ok(CellAttempt::Split);
            }
        }
        let step_chart = if formula == 5 {
            let Ok(chart) = chart_for_domain(&domain) else {
                return Ok(CellAttempt::Split);
            };
            chart
        } else {
            Atan2Chart::Principal
        };
        let (domain_components, center_components, seed_weight) = if let Some(stack) =
            contract.params
        {
            let domain_evaluation = match evaluate_slot(math, &domain, formula, stack, step_chart) {
                Ok(value) => value,
                Err(
                    StackEvaluationError::ContinuousBranch
                    | StackEvaluationError::MengerTranslation
                    | StackEvaluationError::Mandelbulb(MandelbulbError::AzimuthSeam),
                ) => {
                    return Ok(CellAttempt::Split);
                }
                Err(error) => {
                    return Ok(CellAttempt::Inconclusive(map_stack_error(error)));
                }
            };
            let center_evaluation = evaluate_slot(
                math,
                &IntervalBox3::new(center.clone()),
                formula,
                stack,
                step_chart,
            )
            .map_err(map_stack_error)?;
            (
                domain_evaluation.components,
                center_evaluation.components,
                domain_evaluation.seed_weight,
            )
        } else {
            let domain_evaluation =
                match evaluate_with_chart(math, &domain, input.power, step_chart) {
                    Ok(value) => value,
                    Err(MandelbulbError::AzimuthSeam) => return Ok(CellAttempt::Split),
                    Err(error) => {
                        return Ok(CellAttempt::Inconclusive(map_mandelbulb_error(error)));
                    }
                };
            let center_evaluation = evaluate_with_chart(
                math,
                &IntervalBox3::new(center.clone()),
                input.power,
                step_chart,
            )
            .map_err(map_mandelbulb_error)?;
            (
                domain_evaluation.components().clone(),
                center_evaluation.components().clone(),
                1.0,
            )
        };

        let center_jacobian = components_jacobian(&center_components);
        let point_jacobian =
            represented_point_matrix(math, &center_jacobian).map_err(SubdivisionReason::Backend)?;
        let beta = matrix_spectral_upper_bound(
            math,
            &matrix_sub(math, &center_jacobian, &point_jacobian)
                .map_err(SubdivisionReason::Backend)?,
        )
        .map_err(SubdivisionReason::Backend)?;
        let global_variation = matrix_spectral_upper_bound(
            math,
            &matrix_sub(
                math,
                &components_jacobian(&domain_components),
                &point_jacobian,
            )
            .map_err(SubdivisionReason::Backend)?,
        )
        .map_err(SubdivisionReason::Backend)?;
        let hessian = hessian_tensor_upper_bound(math, &components_hessians(&domain_components))
            .map_err(SubdivisionReason::Backend)?;
        let nonlinear =
            taylor_nonlinear_bound(math, &beta, &global_variation, &hessian, &total_radius)
                .map_err(SubdivisionReason::Backend)?;

        let center_image = stack_center_step(
            math,
            &center_components,
            &parameter_center,
            contract.params,
            seed_weight,
        )
        .map_err(SubdivisionReason::Backend)?;
        let next_center =
            represented_point_vector(math, &center_image).map_err(SubdivisionReason::Backend)?;
        let recentering_error = vector_distance_upper(math, &center_image, &next_center)
            .map_err(SubdivisionReason::Backend)?;
        let point_norm = matrix_spectral_upper_bound(math, &point_jacobian)
            .map_err(SubdivisionReason::Backend)?;
        remainder = math
            .add(
                &math
                    .add(
                        &math
                            .mul(&point_norm, &remainder)
                            .map_err(SubdivisionReason::Backend)?,
                        &nonlinear,
                    )
                    .map_err(SubdivisionReason::Backend)?,
                &recentering_error,
            )
            .map_err(SubdivisionReason::Backend)?;
        let seed_derivative =
            scaled_identity(math, seed_weight).map_err(SubdivisionReason::Backend)?;
        derivative = matrix_add(
            math,
            &matrix_mul(math, &point_jacobian, &derivative).map_err(SubdivisionReason::Backend)?,
            &seed_derivative,
        )
        .map_err(SubdivisionReason::Backend)?;
        center = next_center;

        let next_offset = propagated_offset(math, &derivative, &parameter_offset, &remainder)?;
        let next_radius = offset_radius(math, &next_offset)?;
        if next_radius.upper().repr() > max_enclosure.upper().repr() {
            return Ok(CellAttempt::Inconclusive(SubdivisionReason::Vacuous));
        }
        let center_norm = vector_norm(math, &center).map_err(SubdivisionReason::Backend)?;
        let separation = math
            .sub(&center_norm, &next_radius)
            .map_err(SubdivisionReason::Backend)?;
        if separation.lower().repr() > bailout.upper().repr() {
            return Ok(CellAttempt::Excluded {
                escaped_at: iteration,
            });
        }
    }
    Ok(CellAttempt::Inconclusive(
        SubdivisionReason::IterationBudget,
    ))
}

fn publish_cell(
    math: &IntervalMath,
    cell: &ParameterCell,
    escaped_at: usize,
) -> Result<CertifiedCell, SubdivisionReason> {
    let center = cell_center(math, &cell.bounds)?;
    let offset = [
        math.sub(&cell.bounds[0], &center[0])
            .map_err(SubdivisionReason::Backend)?,
        math.sub(&cell.bounds[1], &center[1])
            .map_err(SubdivisionReason::Backend)?,
        math.sub(&cell.bounds[2], &center[2])
            .map_err(SubdivisionReason::Backend)?,
    ];
    let half_extents = [
        math.magnitude(&offset[0])
            .map_err(SubdivisionReason::Backend)?,
        math.magnitude(&offset[1])
            .map_err(SubdivisionReason::Backend)?,
        math.magnitude(&offset[2])
            .map_err(SubdivisionReason::Backend)?,
    ];
    Ok(CertifiedCell {
        bounds: cell.bounds.clone(),
        center,
        half_extents,
        chart: cell.chart,
        includes_upper_seam: cell.includes_upper_seam,
        escaped_at,
    })
}

fn propagated_offset(
    math: &IntervalMath,
    derivative: &Matrix3,
    parameter_offset: &[BigInterval; DIMENSIONS],
    remainder: &BigInterval,
) -> Result<[BigInterval; DIMENSIONS], SubdivisionReason> {
    let linear = matrix_vector_mul(math, derivative, parameter_offset)?;
    let symmetric_remainder = math
        .hull(
            &math.neg(remainder).map_err(SubdivisionReason::Backend)?,
            remainder,
        )
        .map_err(SubdivisionReason::Backend)?;
    Ok([
        math.add(&linear[0], &symmetric_remainder)
            .map_err(SubdivisionReason::Backend)?,
        math.add(&linear[1], &symmetric_remainder)
            .map_err(SubdivisionReason::Backend)?,
        math.add(&linear[2], &symmetric_remainder)
            .map_err(SubdivisionReason::Backend)?,
    ])
}

fn matrix_vector_mul(
    math: &IntervalMath,
    matrix: &Matrix3,
    vector: &[BigInterval; DIMENSIONS],
) -> Result<[BigInterval; DIMENSIONS], SubdivisionReason> {
    let row = |index: usize| -> Result<BigInterval, SubdivisionReason> {
        let terms = [
            math.mul(&matrix[index][0], &vector[0])
                .map_err(SubdivisionReason::Backend)?,
            math.mul(&matrix[index][1], &vector[1])
                .map_err(SubdivisionReason::Backend)?,
            math.mul(&matrix[index][2], &vector[2])
                .map_err(SubdivisionReason::Backend)?,
        ];
        math.add(
            &math
                .add(&terms[0], &terms[1])
                .map_err(SubdivisionReason::Backend)?,
            &terms[2],
        )
        .map_err(SubdivisionReason::Backend)
    };
    Ok([row(0)?, row(1)?, row(2)?])
}

fn offset_radius(
    math: &IntervalMath,
    offset: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, SubdivisionReason> {
    let magnitudes = [
        math.magnitude(&offset[0])
            .map_err(SubdivisionReason::Backend)?,
        math.magnitude(&offset[1])
            .map_err(SubdivisionReason::Backend)?,
        math.magnitude(&offset[2])
            .map_err(SubdivisionReason::Backend)?,
    ];
    vector_norm(math, &magnitudes).map_err(SubdivisionReason::Backend)
}

fn chart_for_domain(domain: &IntervalBox3) -> Result<Atan2Chart, ()> {
    let [x, y, _] = domain.coordinates();
    if x.lower().repr() < &Repr::zero() && y.contains_zero() {
        if y.lower().repr() >= &Repr::zero() {
            Ok(Atan2Chart::Upper)
        } else if y.upper().repr() <= &Repr::zero() {
            Ok(Atan2Chart::Lower)
        } else {
            Err(())
        }
    } else {
        Ok(Atan2Chart::Principal)
    }
}

fn map_mandelbulb_error(error: MandelbulbError) -> SubdivisionReason {
    match error {
        MandelbulbError::AzimuthSeam => SubdivisionReason::AzimuthSeam,
        MandelbulbError::PolarAxis => SubdivisionReason::PolarAxis,
        MandelbulbError::RadiusRegularization => SubdivisionReason::RadiusRegularization,
        MandelbulbError::OutOfRangePower => SubdivisionReason::OutOfRangePower,
        MandelbulbError::Backend(error) => SubdivisionReason::Backend(error),
    }
}

fn map_stack_error(error: StackEvaluationError) -> SubdivisionReason {
    match error {
        StackEvaluationError::InvalidFormula | StackEvaluationError::InvalidParameter => {
            SubdivisionReason::InvalidStack
        }
        StackEvaluationError::ContinuousBranch => SubdivisionReason::FormulaBranch,
        StackEvaluationError::MengerTranslation => SubdivisionReason::MengerTranslation,
        StackEvaluationError::Mandelbulb(error) => map_mandelbulb_error(error),
        StackEvaluationError::Backend(error) => SubdivisionReason::Backend(error),
    }
}

fn components_jacobian(components: &[Jet2; DIMENSIONS]) -> Matrix3 {
    std::array::from_fn(|row| components[row].gradient().clone())
}

fn components_hessians(components: &[Jet2; DIMENSIONS]) -> Tensor3 {
    std::array::from_fn(|component| components[component].hessian().clone())
}

fn stack_center_step(
    math: &IntervalMath,
    components: &[Jet2; DIMENSIONS],
    parameter_center: &[BigInterval; DIMENSIONS],
    stack: Option<&StackParams>,
    seed_weight: f64,
) -> Result<[BigInterval; DIMENSIONS], IntervalError> {
    let mut image = std::array::from_fn(|axis| components[axis].value().clone());
    if seed_weight == 0.0 {
        return Ok(image);
    }
    let weight = math.point_f64(seed_weight)?;
    let julia_weight = math.point_f64(1.0 - seed_weight)?;
    for axis in 0..DIMENSIONS {
        let parameter_term = math.mul(&parameter_center[axis], &weight)?;
        let julia_term = if let Some(stack) = stack {
            math.mul(&math.point_f64(stack.julia[axis])?, &julia_weight)?
        } else {
            math.point_f64(0.0)?
        };
        image[axis] = math.add(&image[axis], &math.add(&parameter_term, &julia_term)?)?;
    }
    Ok(image)
}

fn scaled_identity(math: &IntervalMath, scale: f64) -> Result<Matrix3, IntervalError> {
    let zero = math.point_f64(0.0)?;
    let scale = math.point_f64(scale)?;
    Ok([
        [scale.clone(), zero.clone(), zero.clone()],
        [zero.clone(), scale.clone(), zero.clone()],
        [zero.clone(), zero, scale],
    ])
}

fn split_cell(
    math: &IntervalMath,
    cell: &ParameterCell,
    root_widths: &[BigInterval; DIMENSIONS],
) -> Result<(ParameterCell, ParameterCell), SubdivisionReason> {
    let zero = Repr::zero();
    let split_at_seam =
        cell.bounds[1].lower().repr() < &zero && cell.bounds[1].upper().repr() > &zero;
    let axis = if split_at_seam {
        1
    } else {
        longest_normalized_axis(math, &cell.bounds, root_widths)?
    };
    let split = if split_at_seam {
        math.point_f64(0.0).map_err(SubdivisionReason::Backend)?
    } else {
        midpoint(math, &cell.bounds[axis])?
    };
    let mut lower_bounds = cell.bounds.clone();
    let mut upper_bounds = cell.bounds.clone();
    lower_bounds[axis] = interval_from_endpoints(
        math,
        &represented_lower_endpoint(math, &cell.bounds[axis])
            .map_err(SubdivisionReason::Backend)?,
        &split,
    )?;
    upper_bounds[axis] = interval_from_endpoints(
        math,
        &split,
        &represented_upper_endpoint(math, &cell.bounds[axis])?,
    )?;
    let (lower_chart, upper_chart, lower_owns, upper_owns) = if split_at_seam {
        (Atan2Chart::Lower, Atan2Chart::Upper, false, true)
    } else {
        (
            cell.chart,
            cell.chart,
            cell.includes_upper_seam,
            cell.includes_upper_seam,
        )
    };
    Ok((
        ParameterCell {
            bounds: lower_bounds,
            depth: cell.depth + 1,
            chart: lower_chart,
            includes_upper_seam: lower_owns,
        },
        ParameterCell {
            bounds: upper_bounds,
            depth: cell.depth + 1,
            chart: upper_chart,
            includes_upper_seam: upper_owns,
        },
    ))
}

fn longest_normalized_axis(
    math: &IntervalMath,
    bounds: &[BigInterval; DIMENSIONS],
    root_widths: &[BigInterval; DIMENSIONS],
) -> Result<usize, SubdivisionReason> {
    let normalized = [
        normalized_width(math, &bounds[0], &root_widths[0])?,
        normalized_width(math, &bounds[1], &root_widths[1])?,
        normalized_width(math, &bounds[2], &root_widths[2])?,
    ];
    Ok((1..DIMENSIONS).fold(0, |largest, axis| {
        if normalized[axis].upper().repr() > normalized[largest].upper().repr() {
            axis
        } else {
            largest
        }
    }))
}

fn normalized_width(
    math: &IntervalMath,
    bounds: &BigInterval,
    root_width: &BigInterval,
) -> Result<BigInterval, SubdivisionReason> {
    if root_width.contains_zero() {
        return math.point_f64(0.0).map_err(SubdivisionReason::Backend);
    }
    math.div(
        &math.width(bounds).map_err(SubdivisionReason::Backend)?,
        root_width,
    )
    .map_err(SubdivisionReason::Backend)
}

fn cell_center(
    math: &IntervalMath,
    bounds: &[BigInterval; DIMENSIONS],
) -> Result<[BigInterval; DIMENSIONS], SubdivisionReason> {
    Ok([
        midpoint(math, &bounds[0])?,
        midpoint(math, &bounds[1])?,
        midpoint(math, &bounds[2])?,
    ])
}

fn midpoint(math: &IntervalMath, bounds: &BigInterval) -> Result<BigInterval, SubdivisionReason> {
    let lower = represented_lower_endpoint(math, bounds).map_err(SubdivisionReason::Backend)?;
    let upper = represented_upper_endpoint(math, bounds)?;
    let two = math.point_f64(2.0).map_err(SubdivisionReason::Backend)?;
    let enclosure = math
        .div(
            &math
                .add(&lower, &upper)
                .map_err(SubdivisionReason::Backend)?,
            &two,
        )
        .map_err(SubdivisionReason::Backend)?;
    represented_lower_endpoint(math, &enclosure).map_err(SubdivisionReason::Backend)
}

fn represented_upper_endpoint(
    math: &IntervalMath,
    value: &BigInterval,
) -> Result<BigInterval, SubdivisionReason> {
    let representation = value.upper().repr().clone();
    BigInterval::checked(
        FBig::<_, 2>::from_repr(representation.clone(), math.down),
        FBig::<_, 2>::from_repr(representation, math.up),
    )
    .map_err(SubdivisionReason::Backend)
}

fn interval_from_endpoints(
    math: &IntervalMath,
    lower: &BigInterval,
    upper: &BigInterval,
) -> Result<BigInterval, SubdivisionReason> {
    math.hull(lower, upper).map_err(SubdivisionReason::Backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashu_float::Repr;

    fn request<'a>(center: [&'a str; 3], radius: &'a str, power: f64) -> SubdivisionRequest<'a> {
        SubdivisionRequest {
            center,
            radius,
            power,
            bailout: "2",
            max_iterations: 16,
            max_enclosure_radius: "1e6",
            budgets: SubdivisionBudgets {
                max_depth: 8,
                max_leaves: 64,
                max_orbit_steps: 1_024,
                precision: 128,
            },
        }
    }

    #[test]
    fn noninteger_root_seam_splits_into_owned_side_charts() {
        let result = certify_subdivided(&request(["-2.1", "0", "0.5"], "0.05", 2.5));
        let SubdivisionResult::Excluded { cells } = result else {
            panic!("seam fixture should certify");
        };
        assert_eq!(cells.len(), 2);
        assert!(cells.iter().any(|cell| cell.chart == Atan2Chart::Upper));
        assert!(cells.iter().any(|cell| cell.chart == Atan2Chart::Lower));
        assert!(cells.iter().all(|cell| cell.escaped_at == 1));
        let math = IntervalMath::new(128).unwrap();
        for cell in &cells {
            for (extent, expected) in cell.half_extents.iter().zip(["0.05", "0.025", "0.05"]) {
                assert!(
                    extent.upper().repr() >= math.point_decimal(expected).unwrap().upper().repr()
                );
            }
        }

        let zero = Repr::zero();
        let upper = cells
            .iter()
            .find(|cell| cell.chart == Atan2Chart::Upper)
            .unwrap();
        let lower = cells
            .iter()
            .find(|cell| cell.chart == Atan2Chart::Lower)
            .unwrap();
        assert_eq!(upper.bounds[1].lower().repr(), &zero);
        assert_eq!(lower.bounds[1].upper().repr(), &zero);
        assert!(upper.includes_upper_seam);
        assert!(!lower.includes_upper_seam);
    }

    #[test]
    fn leaf_union_covers_the_root_cube_without_repackaging() {
        let result = certify_subdivided(&request(["-2.1", "0", "0.5"], "0.05", 2.5));
        let SubdivisionResult::Excluded { cells } = result else {
            panic!("seam fixture should certify");
        };
        let math = IntervalMath::new(128).unwrap();
        let root_low = [
            math.point_decimal("-2.15").unwrap(),
            math.point_decimal("-0.05").unwrap(),
            math.point_decimal("0.45").unwrap(),
        ];
        let root_high = [
            math.point_decimal("-2.05").unwrap(),
            math.point_decimal("0.05").unwrap(),
            math.point_decimal("0.55").unwrap(),
        ];
        for axis in [0, 2] {
            assert!(cells
                .iter()
                .all(|cell| cell.bounds[axis].contains(&root_low[axis])
                    && cell.bounds[axis].contains(&root_high[axis])));
        }
        assert!(cells
            .iter()
            .any(|cell| cell.bounds[1].contains(&root_low[1])));
        assert!(cells
            .iter()
            .any(|cell| cell.bounds[1].contains(&root_high[1])));
    }

    fn reference_map(point: [f64; 3], power: f64, chart: Atan2Chart) -> [f64; 3] {
        let [x, y, z] = point;
        let rho = x.hypot(y);
        let radius = rho.hypot(z);
        let theta = rho.atan2(z);
        let phi = if y == 0.0 && x < 0.0 {
            match chart {
                Atan2Chart::Upper | Atan2Chart::Principal => std::f64::consts::PI,
                Atan2Chart::Lower => -std::f64::consts::PI,
            }
        } else {
            y.atan2(x)
        };
        let radial = radius.powf(power);
        let polar = power * theta;
        let azimuth = power * phi;
        [
            radial * polar.sin() * azimuth.cos(),
            radial * polar.sin() * azimuth.sin(),
            radial * polar.cos(),
        ]
    }

    fn escapes_by(parameter: [f64; 3], power: f64, chart: Atan2Chart, escaped_at: usize) -> bool {
        let mut state = parameter;
        for _ in 1..=escaped_at {
            let image = reference_map(state, power, chart);
            state = std::array::from_fn(|axis| image[axis] + parameter[axis]);
            if state.iter().map(|value| value * value).sum::<f64>().sqrt() > 2.0 {
                return true;
            }
        }
        false
    }

    #[test]
    fn deterministic_samples_in_every_published_cell_escape() {
        let result = certify_subdivided(&request(["-2.1", "0", "0.5"], "0.05", 2.5));
        let SubdivisionResult::Excluded { cells } = result else {
            panic!("seam fixture should certify");
        };
        for cell in cells {
            let lower = cell
                .bounds
                .each_ref()
                .map(|bound| bound.lower().to_f64().value());
            let upper = cell
                .bounds
                .each_ref()
                .map(|bound| bound.upper().to_f64().value());
            for selector in [[0.0, 0.0, 0.0], [0.5, 0.5, 0.5], [1.0, 1.0, 1.0]] {
                let point = std::array::from_fn(|axis| {
                    lower[axis] + selector[axis] * (upper[axis] - lower[axis])
                });
                if cell.chart == Atan2Chart::Lower && point[1] == 0.0 {
                    continue;
                }
                assert!(escapes_by(point, 2.5, cell.chart, cell.escaped_at));
            }
        }
    }

    #[test]
    fn all_supported_power_classes_certify_away_from_branches() {
        for power in [2.0, 2.5, 7.2, 8.0, 8.5, 12.0] {
            assert!(matches!(
                certify_subdivided(&request(["2.1", "0.3", "0.5"], "1e-6", power)),
                SubdivisionResult::Excluded { .. }
            ));
        }
    }

    #[test]
    fn split_and_step_budget_exhaustion_are_inconclusive() {
        let mut split_limited = request(["-2.1", "0", "0.5"], "0.05", 2.5);
        split_limited.budgets.max_depth = 0;
        assert_eq!(
            certify_subdivided(&split_limited),
            SubdivisionResult::Inconclusive(SubdivisionReason::SplitBudget)
        );

        let mut step_limited = request(["2.1", "0.3", "0.5"], "1e-6", 8.0);
        step_limited.budgets.max_orbit_steps = 0;
        assert_eq!(
            certify_subdivided(&step_limited),
            SubdivisionResult::Inconclusive(SubdivisionReason::OrbitStepBudget)
        );
    }

    #[test]
    fn axis_regularization_and_bailout_order_never_publish_cells() {
        for input in [
            request(["0", "0", "0"], "1e-6", 8.0),
            request(["2e-7", "3e-7", "4e-7"], "1e-8", 8.0),
        ] {
            assert!(matches!(
                certify_subdivided(&input),
                SubdivisionResult::Inconclusive(_)
            ));
        }

        let mut ordering = request(
            [
                "-1.3855713646001475",
                "0.013856175521597914",
                "0.8000000000000003",
            ],
            "0",
            2.0,
        );
        ordering.bailout = "1.5";
        ordering.max_iterations = 1;
        assert_eq!(
            certify_subdivided(&ordering),
            SubdivisionResult::Inconclusive(SubdivisionReason::IterationBudget)
        );
    }

    fn stack_with(formulas: [i64; 4], rates: [i64; 4]) -> StackParams {
        StackParams {
            formulas,
            rates,
            bailout: 0.25,
            julia_amount: 0.35,
            julia: [0.2, -0.1, 0.3],
            refine: false,
            ..StackParams::default()
        }
    }

    #[test]
    fn every_authored_formula_id_has_a_certifying_host_path() {
        for formula in 0..=10 {
            let stack = stack_with([formula, 0, 0, 0], [1, 0, 0, 0]);
            let result =
                certify_stack_subdivided(&request(["3.25", "2.5", "1.75"], "0", 2.5), &stack);
            assert!(
                matches!(result, SubdivisionResult::Excluded { .. }),
                "formula {formula}: {result:?}"
            );
        }
    }

    #[test]
    fn mixed_stack_uses_authored_order_rates_seed_and_post_slot_bailout() {
        let weighted = stack_with([8, 7, 3, 5], [2, 1, 1, 1]);
        let result =
            certify_stack_subdivided(&request(["3.25", "2.5", "1.75"], "0", 8.0), &weighted);
        let SubdivisionResult::Excluded { cells } = result else {
            panic!("weighted mixed stack should certify");
        };
        assert_eq!(
            cells[0].escaped_at, 1,
            "the first weighted rotate runs first"
        );

        let reordered = stack_with([7, 8, 3, 5], [1, 2, 1, 1]);
        assert!(matches!(
            certify_stack_subdivided(&request(["3.25", "2.5", "1.75"], "0", 8.0), &reordered),
            SubdivisionResult::Excluded { .. }
        ));

        let mut pre_bulb = stack_with([0, 5, 0, 0], [1, 1, 0, 0]);
        pre_bulb.bailout = 100.0;
        let SubdivisionResult::Excluded { cells } =
            certify_stack_subdivided(&request(["3.25", "2.5", "1.75"], "0", 8.0), &pre_bulb)
        else {
            panic!("the authored pre-bulb escape should certify");
        };
        assert_eq!(cells[0].escaped_at, 2);
    }

    #[test]
    fn malformed_stack_and_budget_publish_no_cells() {
        let mut invalid_formula = stack_with([11, 0, 0, 0], [1, 0, 0, 0]);
        assert_eq!(
            certify_stack_subdivided(&request(["3", "2", "1"], "0", 8.0), &invalid_formula),
            SubdivisionResult::Inconclusive(SubdivisionReason::InvalidStack)
        );

        invalid_formula.formulas = [1, 0, 0, 0];
        invalid_formula.rates = [0; 4];
        assert_eq!(
            certify_stack_subdivided(&request(["3", "2", "1"], "0", 8.0), &invalid_formula),
            SubdivisionResult::Inconclusive(SubdivisionReason::InvalidStack)
        );

        let mut bad_budget = request(["3", "2", "1"], "0", 8.0);
        bad_budget.budgets.max_leaves = 0;
        assert_eq!(
            certify_stack_subdivided(&bad_budget, &stack_with([1, 0, 0, 0], [1, 0, 0, 0])),
            SubdivisionResult::Inconclusive(SubdivisionReason::InvalidBudget)
        );
    }

    #[test]
    fn menger_discontinuity_never_silently_selects_a_crossing_branch() {
        let mut stack = stack_with([3, 0, 0, 0], [1, 0, 0, 0]);
        stack.scale = 2.0;
        stack.offset = [1.0; 3];
        stack.bailout = 100.0;
        let mut crossing = request(["0.75", "0.75", "0.25"], "0.5", 8.0);
        crossing.budgets.max_depth = 0;
        assert_eq!(
            certify_stack_subdivided(&crossing, &stack),
            SubdivisionResult::Inconclusive(SubdivisionReason::SplitBudget)
        );
    }
}
