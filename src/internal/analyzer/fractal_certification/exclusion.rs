//! One-cell matrix Taylor exclusion for the continuous-power Mandelbulb.

use dashu_float::{FBig, Repr};

use super::mandelbulb::{
    evaluate, hessian_tensor_upper_bound, matrix_spectral_upper_bound, IntervalBox3,
    MandelbulbError, MandelbulbEvaluation,
};
use super::{BigInterval, IntervalError, IntervalMath};

const DIMENSIONS: usize = 3;
const MAX_ITERATION_BUDGET: usize = 4_096;
const MAX_PRECISION_BUDGET: usize = 4_096;
pub(super) type Matrix3 = [[BigInterval; DIMENSIONS]; DIMENSIONS];
pub(super) type Tensor3 = [[[BigInterval; DIMENSIONS]; DIMENSIONS]; DIMENSIONS];

/// Bounded inputs for one unsplit exclusion attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExclusionRequest<'a> {
    pub(crate) center: [&'a str; DIMENSIONS],
    pub(crate) radius: &'a str,
    pub(crate) power: f64,
    pub(crate) bailout: &'a str,
    pub(crate) max_iterations: usize,
    pub(crate) precision: usize,
    pub(crate) max_enclosure_radius: &'a str,
}

/// Typed reason why this cell did not produce a positive certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InconclusiveReason {
    MalformedInput,
    InvalidPrecision,
    InvalidRadius,
    InvalidBailout,
    InvalidBudget,
    OutOfRangePower,
    AzimuthSeam,
    PolarAxis,
    RadiusRegularization,
    Vacuous,
    IterationBudget,
    Backend(IntervalError),
}

/// Outcome of one bounded, unsplit parameter-cell attempt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExclusionResult {
    Excluded {
        radius: BigInterval,
        enclosure_radius: BigInterval,
        escaped_at: usize,
    },
    Inconclusive(InconclusiveReason),
}

impl ExclusionResult {
    pub(crate) const fn escaped_at(&self) -> Option<usize> {
        match self {
            Self::Excluded { escaped_at, .. } => Some(*escaped_at),
            Self::Inconclusive(_) => None,
        }
    }
}

/// Attempts a rigorous one-cell matrix Taylor exclusion certificate.
pub(crate) fn certify(input: &ExclusionRequest<'_>) -> ExclusionResult {
    match certify_inner(input) {
        Ok(result) => result,
        Err(reason) => ExclusionResult::Inconclusive(reason),
    }
}

fn certify_inner(input: &ExclusionRequest<'_>) -> Result<ExclusionResult, InconclusiveReason> {
    if input.precision == 0 {
        return Err(InconclusiveReason::InvalidPrecision);
    }
    if input.precision > MAX_PRECISION_BUDGET || input.max_iterations > MAX_ITERATION_BUDGET {
        return Err(InconclusiveReason::InvalidBudget);
    }
    let math =
        IntervalMath::new(input.precision).map_err(|_| InconclusiveReason::InvalidPrecision)?;
    let parameter_center = [
        parse_center(&math, input.center[0])?,
        parse_center(&math, input.center[1])?,
        parse_center(&math, input.center[2])?,
    ];
    let radius = math
        .point_decimal(input.radius)
        .map_err(|_| InconclusiveReason::InvalidRadius)?;
    if radius.lower().repr() < &Repr::zero() {
        return Err(InconclusiveReason::InvalidRadius);
    }
    let bailout = math
        .point_decimal(input.bailout)
        .map_err(|_| InconclusiveReason::InvalidBailout)?;
    if !bailout.strictly_positive() {
        return Err(InconclusiveReason::InvalidBailout);
    }
    let max_enclosure_radius = math
        .point_decimal(input.max_enclosure_radius)
        .map_err(|_| InconclusiveReason::InvalidBudget)?;
    if !max_enclosure_radius.strictly_positive() {
        return Err(InconclusiveReason::InvalidBudget);
    }
    if !input.power.is_finite() || !(2.0..=12.0).contains(&input.power) {
        return Err(InconclusiveReason::OutOfRangePower);
    }

    let zero = math.point_f64(0.0).map_err(InconclusiveReason::Backend)?;
    let mut center = parameter_center.clone();
    let mut derivative = identity(&math).map_err(InconclusiveReason::Backend)?;
    let mut remainder = zero;

    for iteration in 1..=input.max_iterations {
        let derivative_norm =
            matrix_spectral_upper_bound(&math, &derivative).map_err(InconclusiveReason::Backend)?;
        let linear_radius = math
            .mul(&derivative_norm, &radius)
            .map_err(InconclusiveReason::Backend)?;
        let total_radius = math
            .add(&linear_radius, &remainder)
            .map_err(InconclusiveReason::Backend)?;
        if total_radius.upper().repr() > max_enclosure_radius.upper().repr() {
            return Err(InconclusiveReason::Vacuous);
        }

        let domain =
            ball_box(&math, &center, &total_radius).map_err(InconclusiveReason::Backend)?;
        let domain_evaluation =
            evaluate(&math, &domain, input.power).map_err(map_mandelbulb_error)?;
        let center_evaluation = evaluate(&math, &IntervalBox3::new(center.clone()), input.power)
            .map_err(map_mandelbulb_error)?;

        let center_jacobian = jacobian(&center_evaluation);
        let point_jacobian = represented_point_matrix(&math, &center_jacobian)
            .map_err(InconclusiveReason::Backend)?;
        let beta_difference = matrix_sub(&math, &center_jacobian, &point_jacobian)
            .map_err(InconclusiveReason::Backend)?;
        let beta = matrix_spectral_upper_bound(&math, &beta_difference)
            .map_err(InconclusiveReason::Backend)?;

        let domain_jacobian = jacobian(&domain_evaluation);
        let global_difference = matrix_sub(&math, &domain_jacobian, &point_jacobian)
            .map_err(InconclusiveReason::Backend)?;
        let global_variation = matrix_spectral_upper_bound(&math, &global_difference)
            .map_err(InconclusiveReason::Backend)?;
        let hessian = hessian_tensor_upper_bound(&math, &hessians(&domain_evaluation))
            .map_err(InconclusiveReason::Backend)?;
        let nonlinear =
            taylor_nonlinear_bound(&math, &beta, &global_variation, &hessian, &total_radius)
                .map_err(InconclusiveReason::Backend)?;

        let center_image = center_step_enclosure(&math, &center_evaluation, &parameter_center)
            .map_err(InconclusiveReason::Backend)?;
        let next_center =
            represented_point_vector(&math, &center_image).map_err(InconclusiveReason::Backend)?;
        let recentering_error = vector_distance_upper(&math, &center_image, &next_center)
            .map_err(InconclusiveReason::Backend)?;

        let point_jacobian_norm = matrix_spectral_upper_bound(&math, &point_jacobian)
            .map_err(InconclusiveReason::Backend)?;
        remainder = math
            .add(
                &math
                    .add(
                        &math
                            .mul(&point_jacobian_norm, &remainder)
                            .map_err(InconclusiveReason::Backend)?,
                        &nonlinear,
                    )
                    .map_err(InconclusiveReason::Backend)?,
                &recentering_error,
            )
            .map_err(InconclusiveReason::Backend)?;
        derivative = matrix_add(
            &math,
            &matrix_mul(&math, &point_jacobian, &derivative)
                .map_err(InconclusiveReason::Backend)?,
            &identity(&math).map_err(InconclusiveReason::Backend)?,
        )
        .map_err(InconclusiveReason::Backend)?;
        center = next_center;

        let derivative_norm =
            matrix_spectral_upper_bound(&math, &derivative).map_err(InconclusiveReason::Backend)?;
        let linear_radius = math
            .mul(&derivative_norm, &radius)
            .map_err(InconclusiveReason::Backend)?;
        let total_radius = math
            .add(&linear_radius, &remainder)
            .map_err(InconclusiveReason::Backend)?;
        if total_radius.upper().repr() > max_enclosure_radius.upper().repr() {
            return Err(InconclusiveReason::Vacuous);
        }
        let center_norm = vector_norm(&math, &center).map_err(InconclusiveReason::Backend)?;
        let separation = math
            .sub(&center_norm, &total_radius)
            .map_err(InconclusiveReason::Backend)?;
        if separation.lower().repr() > bailout.upper().repr() {
            return Ok(ExclusionResult::Excluded {
                radius,
                enclosure_radius: total_radius,
                escaped_at: iteration,
            });
        }
    }

    Ok(ExclusionResult::Inconclusive(
        InconclusiveReason::IterationBudget,
    ))
}

fn parse_center(math: &IntervalMath, value: &str) -> Result<BigInterval, InconclusiveReason> {
    math.point_decimal(value)
        .map_err(|_| InconclusiveReason::MalformedInput)
}

fn map_mandelbulb_error(error: MandelbulbError) -> InconclusiveReason {
    match error {
        MandelbulbError::AzimuthSeam => InconclusiveReason::AzimuthSeam,
        MandelbulbError::PolarAxis => InconclusiveReason::PolarAxis,
        MandelbulbError::RadiusRegularization => InconclusiveReason::RadiusRegularization,
        MandelbulbError::OutOfRangePower => InconclusiveReason::OutOfRangePower,
        MandelbulbError::Backend(error) => InconclusiveReason::Backend(error),
    }
}

pub(super) fn jacobian(evaluation: &MandelbulbEvaluation) -> Matrix3 {
    std::array::from_fn(|row| evaluation.components()[row].gradient().clone())
}

pub(super) fn hessians(evaluation: &MandelbulbEvaluation) -> Tensor3 {
    std::array::from_fn(|component| evaluation.components()[component].hessian().clone())
}

pub(super) fn center_step_enclosure(
    math: &IntervalMath,
    evaluation: &MandelbulbEvaluation,
    parameter_center: &[BigInterval; DIMENSIONS],
) -> Result<[BigInterval; DIMENSIONS], IntervalError> {
    Ok([
        math.add(evaluation.components()[0].value(), &parameter_center[0])?,
        math.add(evaluation.components()[1].value(), &parameter_center[1])?,
        math.add(evaluation.components()[2].value(), &parameter_center[2])?,
    ])
}

fn ball_box(
    math: &IntervalMath,
    center: &[BigInterval; DIMENSIONS],
    radius: &BigInterval,
) -> Result<IntervalBox3, IntervalError> {
    Ok(IntervalBox3::new([
        math.hull(
            &math.sub(&center[0], radius)?,
            &math.add(&center[0], radius)?,
        )?,
        math.hull(
            &math.sub(&center[1], radius)?,
            &math.add(&center[1], radius)?,
        )?,
        math.hull(
            &math.sub(&center[2], radius)?,
            &math.add(&center[2], radius)?,
        )?,
    ]))
}

pub(super) fn represented_point_vector(
    math: &IntervalMath,
    values: &[BigInterval; DIMENSIONS],
) -> Result<[BigInterval; DIMENSIONS], IntervalError> {
    Ok([
        represented_lower_endpoint(math, &values[0])?,
        represented_lower_endpoint(math, &values[1])?,
        represented_lower_endpoint(math, &values[2])?,
    ])
}

pub(super) fn represented_point_matrix(
    math: &IntervalMath,
    values: &Matrix3,
) -> Result<Matrix3, IntervalError> {
    matrix_map(values, |value| represented_lower_endpoint(math, value))
}

pub(super) fn represented_lower_endpoint(
    math: &IntervalMath,
    value: &BigInterval,
) -> Result<BigInterval, IntervalError> {
    let representation = value.lower().repr().clone();
    BigInterval::checked(
        FBig::<_, 2>::from_repr(representation.clone(), math.down),
        FBig::<_, 2>::from_repr(representation, math.up),
    )
}

pub(super) fn vector_norm(
    math: &IntervalMath,
    vector: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let squares = [
        math.square(&vector[0])?,
        math.square(&vector[1])?,
        math.square(&vector[2])?,
    ];
    math.sqrt(&sum_three(math, &squares)?)
}

pub(super) fn vector_distance_upper(
    math: &IntervalMath,
    enclosure: &[BigInterval; DIMENSIONS],
    point: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let distances = [
        math.magnitude(&math.sub(&enclosure[0], &point[0])?)?,
        math.magnitude(&math.sub(&enclosure[1], &point[1])?)?,
        math.magnitude(&math.sub(&enclosure[2], &point[2])?)?,
    ];
    let squared = [
        math.square(&distances[0])?,
        math.square(&distances[1])?,
        math.square(&distances[2])?,
    ];
    math.sqrt(&sum_three(math, &squared)?)
}

pub(super) fn taylor_nonlinear_bound(
    math: &IntervalMath,
    beta: &BigInterval,
    global_variation: &BigInterval,
    hessian: &BigInterval,
    radius: &BigInterval,
) -> Result<BigInterval, IntervalError> {
    let global = math.mul(global_variation, radius)?;
    let beta_term = math.mul(beta, radius)?;
    let half = math.point_f64(0.5)?;
    let quadratic = math.mul(&half, &math.mul(hessian, &math.square(radius)?)?)?;
    let local = math.add(&beta_term, &quadratic)?;
    Ok(if global.upper().repr() <= local.upper().repr() {
        global
    } else {
        local
    })
}

pub(super) fn identity(math: &IntervalMath) -> Result<Matrix3, IntervalError> {
    let zero = math.point_f64(0.0)?;
    let one = math.point_f64(1.0)?;
    Ok([
        [one.clone(), zero.clone(), zero.clone()],
        [zero.clone(), one.clone(), zero.clone()],
        [zero.clone(), zero, one],
    ])
}

pub(crate) fn matrix_add(
    math: &IntervalMath,
    lhs: &Matrix3,
    rhs: &Matrix3,
) -> Result<Matrix3, IntervalError> {
    matrix_zip(lhs, rhs, |left, right| math.add(left, right))
}

pub(crate) fn matrix_sub(
    math: &IntervalMath,
    lhs: &Matrix3,
    rhs: &Matrix3,
) -> Result<Matrix3, IntervalError> {
    matrix_zip(lhs, rhs, |left, right| math.sub(left, right))
}

pub(crate) fn matrix_mul(
    math: &IntervalMath,
    lhs: &Matrix3,
    rhs: &Matrix3,
) -> Result<Matrix3, IntervalError> {
    matrix_generate(|row, column| {
        let products = [
            math.mul(&lhs[row][0], &rhs[0][column])?,
            math.mul(&lhs[row][1], &rhs[1][column])?,
            math.mul(&lhs[row][2], &rhs[2][column])?,
        ];
        sum_three(math, &products)
    })
}

fn sum_three(
    math: &IntervalMath,
    values: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    math.add(&math.add(&values[0], &values[1])?, &values[2])
}

fn matrix_map<F>(matrix: &Matrix3, mut operation: F) -> Result<Matrix3, IntervalError>
where
    F: FnMut(&BigInterval) -> Result<BigInterval, IntervalError>,
{
    matrix_generate(|row, column| operation(&matrix[row][column]))
}

fn matrix_zip<F>(lhs: &Matrix3, rhs: &Matrix3, mut operation: F) -> Result<Matrix3, IntervalError>
where
    F: FnMut(&BigInterval, &BigInterval) -> Result<BigInterval, IntervalError>,
{
    matrix_generate(|row, column| operation(&lhs[row][column], &rhs[row][column]))
}

fn matrix_generate<F>(mut operation: F) -> Result<Matrix3, IntervalError>
where
    F: FnMut(usize, usize) -> Result<BigInterval, IntervalError>,
{
    Ok([
        [operation(0, 0)?, operation(0, 1)?, operation(0, 2)?],
        [operation(1, 0)?, operation(1, 1)?, operation(1, 2)?],
        [operation(2, 0)?, operation(2, 1)?, operation(2, 2)?],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTER: [&str; 3] = ["1.1", "0.2", "-0.1"];

    fn request<'a>(
        center: [&'a str; 3],
        radius: &'a str,
        power: f64,
        bailout: &'a str,
        max_iterations: usize,
        precision: usize,
    ) -> ExclusionRequest<'a> {
        ExclusionRequest {
            center,
            radius,
            power,
            bailout,
            max_iterations,
            precision,
            max_enclosure_radius: "1e6",
        }
    }

    #[test]
    fn immediate_escape_precedes_chart_rejection() {
        let result = certify(&request(["2.1", "0", "0"], "0.05", 8.0, "2", 16, 128));
        assert!(matches!(
            result,
            ExclusionResult::Excluded { escaped_at: 1, .. }
        ));
    }

    #[test]
    fn bailout_is_tested_after_the_shader_step() {
        // At power two this point starts outside R=1.5 but maps back inside it.
        // Testing x_0 would therefore certify a cell the shader does not reject.
        let center = [
            "-1.3855713646001475",
            "0.013856175521597914",
            "0.8000000000000003",
        ];
        assert_eq!(
            certify(&request(center, "0", 2.0, "1.5", 1, 128)),
            ExclusionResult::Inconclusive(InconclusiveReason::IterationBudget)
        );
    }

    #[test]
    fn origin_axis_and_strict_escape_equality_are_inconclusive() {
        assert_eq!(
            certify(&request(["0", "0", "0"], "1e-6", 8.0, "2", 16, 128)),
            ExclusionResult::Inconclusive(InconclusiveReason::PolarAxis)
        );
        assert!(matches!(
            certify(&request(["2", "0", "0"], "0", 8.0, "2", 0, 128)),
            ExclusionResult::Inconclusive(InconclusiveReason::IterationBudget)
        ));
    }

    #[test]
    fn one_cell_chart_and_regularization_crossings_are_typed() {
        assert_eq!(
            certify(&request(["-1", "0", "0.5"], "0.1", 2.5, "2", 16, 128)),
            ExclusionResult::Inconclusive(InconclusiveReason::AzimuthSeam)
        );
        assert_eq!(
            certify(&request(
                ["2e-7", "3e-7", "4e-7"],
                "1e-8",
                8.0,
                "2",
                16,
                128
            )),
            ExclusionResult::Inconclusive(InconclusiveReason::RadiusRegularization)
        );
    }

    #[test]
    fn parameter_seed_dependence_is_propagated() {
        let result = certify(&request(CENTER, "1e-8", 8.0, "2", 16, 128));
        match result {
            ExclusionResult::Excluded {
                escaped_at,
                enclosure_radius,
                ..
            } => {
                assert_eq!(escaped_at, 1);
                let math = IntervalMath::new(128).unwrap();
                let seed_radius = math.point_decimal("1e-8").unwrap();
                assert!(enclosure_radius.upper().repr() > seed_radius.upper().repr());
            }
            other @ ExclusionResult::Inconclusive(_) => {
                panic!("expected exclusion, got {other:?}")
            }
        }
    }

    fn reference_map(point: [f64; 3], power: f64) -> [f64; 3] {
        let [x, y, z] = point;
        let rho = x.hypot(y);
        let radius = rho.hypot(z);
        let theta = rho.atan2(z);
        let phi = y.atan2(x);
        let radial = radius.powf(power);
        let polar = power * theta;
        let azimuth = power * phi;
        [
            radial * polar.sin() * azimuth.cos(),
            radial * polar.sin() * azimuth.sin(),
            radial * polar.cos(),
        ]
    }

    fn escapes_by(mut point: [f64; 3], power: f64, bailout: f64, iteration: usize) -> bool {
        let parameter = point;
        for step in 0..=iteration {
            if point.iter().map(|value| value * value).sum::<f64>().sqrt() > bailout {
                return true;
            }
            if step < iteration {
                let image = reference_map(point, power);
                point = std::array::from_fn(|axis| image[axis] + parameter[axis]);
            }
        }
        false
    }

    #[test]
    fn every_deterministic_and_adversarial_sample_in_certified_ball_escapes() {
        let radius = 1e-8;
        let result = certify(&request(CENTER, "1e-8", 8.0, "2", 16, 128));
        let ExclusionResult::Excluded { escaped_at, .. } = result else {
            panic!("fixture should certify");
        };
        let center = [1.1, 0.2, -0.1];
        let mut directions = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        for direction in &mut directions {
            let point = std::array::from_fn(|axis| center[axis] + 0.999 * radius * direction[axis]);
            assert!(escapes_by(point, 8.0, 2.0, escaped_at));
        }

        let mut state = 7_u64;
        for _ in 0..250 {
            let mut direction: [f64; 3] = std::array::from_fn(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                2.0 * (state as f64 / u64::MAX as f64) - 1.0
            });
            let length = direction
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            if length > 1.0 {
                direction = direction.map(|value| value / length);
            }
            let point = std::array::from_fn(|axis| center[axis] + 0.999 * radius * direction[axis]);
            assert!(escapes_by(point, 8.0, 2.0, escaped_at));
        }
    }

    #[test]
    fn hard_decimal_anchor_is_never_routed_through_f64() {
        let center = [
            "0.11852213395765483217036972973801312036812305450439453125",
            "0.2402522627527517162793202487591770477592945098876953125",
            "0.8812425012552556058409436445799656212329864501953125",
        ];
        let math = IntervalMath::new(192).unwrap();
        for coordinate in center {
            let parsed = parse_center(&math, coordinate).unwrap();
            assert!(math.contains_decimal(&parsed, coordinate).unwrap());
        }
        let beyond_binary64 = "1.1000000000000000000000000000000000001";
        let parsed = parse_center(&math, beyond_binary64).unwrap();
        let binary64 = math
            .point_f64(beyond_binary64.parse::<f64>().unwrap())
            .unwrap();
        assert!(math.contains_decimal(&parsed, beyond_binary64).unwrap());
        assert_ne!(parsed, binary64);
        let result = certify(&request(center, "1e-14", 8.0, "2", 64, 192));
        assert!(matches!(
            result,
            ExclusionResult::Excluded { escaped_at: 35, .. }
        ));
    }

    #[test]
    fn larger_failed_ball_is_never_reported_as_excluded() {
        assert!(matches!(
            certify(&request(CENTER, "1e-8", 8.0, "2", 16, 128)),
            ExclusionResult::Excluded { .. }
        ));
        assert!(matches!(
            certify(&request(CENTER, "0.5", 8.0, "2", 16, 128)),
            ExclusionResult::Inconclusive(_)
        ));
    }

    #[test]
    fn hessian_alternative_includes_center_jacobian_mismatch() {
        let math = IntervalMath::new(128).unwrap();
        let beta = math.point_f64(3.0).unwrap();
        let global_variation = math.point_f64(20.0).unwrap();
        let hessian = math.point_f64(0.0).unwrap();
        let radius = math.point_f64(2.0).unwrap();
        let bound =
            taylor_nonlinear_bound(&math, &beta, &global_variation, &hessian, &radius).unwrap();
        assert!(math.contains_f64(&bound, 6.0).unwrap());
        assert!(bound.upper().to_f64().value() >= 6.0);
    }

    #[test]
    fn classification_is_stable_when_precision_increases() {
        let low = certify(&request(CENTER, "1e-8", 8.0, "2", 16, 96));
        let high = certify(&request(CENTER, "1e-8", 8.0, "2", 16, 192));
        assert_eq!(low.escaped_at(), high.escaped_at());
        assert!(low.escaped_at().is_some());
    }

    #[test]
    fn malformed_inputs_return_typed_reasons_without_panicking() {
        let cases = [
            (
                request(["not", "0", "0"], "0.1", 8.0, "2", 16, 128),
                InconclusiveReason::MalformedInput,
            ),
            (
                request(CENTER, "-0.1", 8.0, "2", 16, 128),
                InconclusiveReason::InvalidRadius,
            ),
            (
                request(CENTER, "0.1", 1.0, "2", 16, 128),
                InconclusiveReason::OutOfRangePower,
            ),
            (
                request(CENTER, "0.1", 8.0, "0", 16, 128),
                InconclusiveReason::InvalidBailout,
            ),
            (
                request(CENTER, "0.1", 8.0, "2", 16, 0),
                InconclusiveReason::InvalidPrecision,
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(certify(&input), ExclusionResult::Inconclusive(expected));
        }

        let mut oversized_budget = request(CENTER, "0.1", 8.0, "2", 4_097, 128);
        assert_eq!(
            certify(&oversized_budget),
            ExclusionResult::Inconclusive(InconclusiveReason::InvalidBudget)
        );
        oversized_budget.max_iterations = 16;
        oversized_budget.max_enclosure_radius = "1e-30";
        assert_eq!(
            certify(&oversized_budget),
            ExclusionResult::Inconclusive(InconclusiveReason::Vacuous)
        );
    }
}
