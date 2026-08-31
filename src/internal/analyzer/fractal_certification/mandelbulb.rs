//! Seam-free continuous-power Mandelbulb interval evaluation.

use std::fmt;

use super::jet::Jet2;
use super::{Atan2Chart, BigInterval, IntervalError, IntervalMath};

const DIMENSIONS: usize = 3;
const MIN_POWER: f64 = 2.0;
const MAX_POWER: f64 = 12.0;
const RADIUS_REGULARIZATION: f64 = 1e-6;

/// Axis-aligned spatial domain for one Mandelbulb map evaluation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IntervalBox3 {
    coordinates: [BigInterval; DIMENSIONS],
}

impl IntervalBox3 {
    pub(crate) const fn new(coordinates: [BigInterval; DIMENSIONS]) -> Self {
        Self { coordinates }
    }

    pub(crate) fn coordinates(&self) -> &[BigInterval; DIMENSIONS] {
        &self.coordinates
    }
}

/// Branch or backend reason that prevented a rigorous smooth-chart result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MandelbulbError {
    AzimuthSeam,
    PolarAxis,
    RadiusRegularization,
    OutOfRangePower,
    Backend(IntervalError),
}

impl fmt::Display for MandelbulbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AzimuthSeam => formatter.write_str("domain crosses the azimuth seam"),
            Self::PolarAxis => formatter.write_str("domain touches the polar axis"),
            Self::RadiusRegularization => {
                formatter.write_str("domain reaches the shader radius regularization")
            }
            Self::OutOfRangePower => formatter.write_str("effective power is outside [2, 12]"),
            Self::Backend(error) => write!(formatter, "interval backend: {error}"),
        }
    }
}

impl std::error::Error for MandelbulbError {}

impl From<IntervalError> for MandelbulbError {
    fn from(error: IntervalError) -> Self {
        Self::Backend(error)
    }
}

/// Reason a group-power spectral-norm enclosure could not be certified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroupPowerNormError {
    OutOfRangeParameter,
    RadiusRegularization,
    NegativeAxisSingularity,
    Backend(IntervalError),
}

impl fmt::Display for GroupPowerNormError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRangeParameter => {
                formatter.write_str("group-power parameters are outside the supported range")
            }
            Self::RadiusRegularization => {
                formatter.write_str("domain reaches the shader radius regularization")
            }
            Self::NegativeAxisSingularity => formatter
                .write_str("non-integer polar multiplier is unbounded at the negative polar axis"),
            Self::Backend(error) => write!(formatter, "interval backend: {error}"),
        }
    }
}

impl std::error::Error for GroupPowerNormError {}

impl From<IntervalError> for GroupPowerNormError {
    fn from(error: IntervalError) -> Self {
        Self::Backend(error)
    }
}

/// Value, Jacobian, and component Hessians of the smooth Mandelbulb chart.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MandelbulbEvaluation {
    components: [Jet2; DIMENSIONS],
}

impl MandelbulbEvaluation {
    pub(crate) fn components(&self) -> &[Jet2; DIMENSIONS] {
        &self.components
    }
}

/// Evaluates the shader-equivalent continuous-power map on one smooth chart.
pub(crate) fn evaluate(
    math: &IntervalMath,
    domain: &IntervalBox3,
    power: f64,
) -> Result<MandelbulbEvaluation, MandelbulbError> {
    evaluate_with_chart(math, domain, power, Atan2Chart::Principal)
}

pub(crate) fn evaluate_with_chart(
    math: &IntervalMath,
    domain: &IntervalBox3,
    power: f64,
    azimuth_chart: Atan2Chart,
) -> Result<MandelbulbEvaluation, MandelbulbError> {
    if !power.is_finite() || !(MIN_POWER..=MAX_POWER).contains(&power) {
        return Err(MandelbulbError::OutOfRangePower);
    }

    let [x_domain, y_domain, z_domain] = domain.coordinates();
    let x = Jet2::x(math, x_domain.clone())?;
    let y = Jet2::y(math, y_domain.clone())?;
    let z = Jet2::z(math, z_domain.clone())?;

    let horizontal_squared = x.square(math)?.add(math, &y.square(math)?)?;
    if !horizontal_squared.value().strictly_positive() {
        return Err(MandelbulbError::PolarAxis);
    }
    let rho = horizontal_squared.sqrt(math)?;

    let radius_squared = horizontal_squared.add(math, &z.square(math)?)?;
    if !radius_squared.value().strictly_positive() {
        return Err(MandelbulbError::RadiusRegularization);
    }
    let radius = radius_squared.sqrt(math)?;
    let threshold = math.point_f64(RADIUS_REGULARIZATION)?;
    if radius.value().lower().repr() <= threshold.upper().repr() {
        return Err(MandelbulbError::RadiusRegularization);
    }

    let theta = Jet2::atan2(math, &rho, &z)?;
    let phi = match Jet2::atan2_chart(math, &y, &x, azimuth_chart) {
        Ok(value) => value,
        Err(IntervalError::NeedsSplit) => return Err(MandelbulbError::AzimuthSeam),
        Err(error) => return Err(MandelbulbError::Backend(error)),
    };
    let power = Jet2::constant(math, math.point_f64(power)?)?;
    let powered_radius = power.mul(math, &radius.ln(math)?)?.exp(math)?;
    let powered_theta = power.mul(math, &theta)?;
    let powered_phi = power.mul(math, &phi)?;
    let sin_theta = powered_theta.sin(math)?;
    let cos_theta = powered_theta.cos(math)?;
    let sin_phi = powered_phi.sin(math)?;
    let cos_phi = powered_phi.cos(math)?;
    let radial_sine = powered_radius.mul(math, &sin_theta)?;

    Ok(MandelbulbEvaluation {
        components: [
            radial_sine.mul(math, &cos_phi)?,
            radial_sine.mul(math, &sin_phi)?,
            powered_radius.mul(math, &cos_theta)?,
        ],
    })
}

/// Encloses the spectral norm of
/// `(r, theta, phi) -> (r^p, alpha theta, beta phi)`.
///
/// Away from the axis the singular values are `r^(p-1)` times
/// `{p, alpha, beta sin(alpha theta) / sin(theta)}`. For integer `alpha`, the
/// last quotient is evaluated as `U_(alpha-1)(cos(theta))` and capped by
/// `|U_(alpha-1)| <= alpha`; this bounds the norm across both axes without
/// claiming that the Cartesian Jacobian itself extends there.
pub(crate) fn group_power_spectral_norm_upper_bound(
    math: &IntervalMath,
    domain: &IntervalBox3,
    radial_exponent: f64,
    polar_multiplier: f64,
    azimuth_multiplier: f64,
) -> Result<BigInterval, GroupPowerNormError> {
    if !radial_exponent.is_finite()
        || !(MIN_POWER..=MAX_POWER).contains(&radial_exponent)
        || !polar_multiplier.is_finite()
        || !(MIN_POWER..=MAX_POWER).contains(&polar_multiplier)
        || !azimuth_multiplier.is_finite()
    {
        return Err(GroupPowerNormError::OutOfRangeParameter);
    }

    let [x, y, z] = domain.coordinates();
    let horizontal_squared = math.add(&math.square(x)?, &math.square(y)?)?;
    let radius_squared = math.add(&horizontal_squared, &math.square(z)?)?;
    let radius = math.sqrt(&radius_squared)?;
    let threshold = math.point_f64(RADIUS_REGULARIZATION)?;
    if radius.lower().repr() <= threshold.upper().repr() {
        return Err(GroupPowerNormError::RadiusRegularization);
    }
    let zero = math.point_f64(0.0)?;

    let integer_polar = polar_multiplier.fract() == 0.0;
    let kernel = if integer_polar {
        integer_chebyshev_kernel_bound(math, z, &radius, polar_multiplier as u32)?
    } else if horizontal_squared.strictly_positive() {
        off_axis_kernel_bound(math, z, &radius, &horizontal_squared, polar_multiplier)?
    } else if z.lower().repr() < zero.lower().repr() {
        return Err(GroupPowerNormError::NegativeAxisSingularity);
    } else {
        positive_axis_kernel_bound(math, polar_multiplier)?
    };

    let radial_factor = {
        let exponent = math.point_f64(radial_exponent - 1.0)?;
        math.exp(&math.mul(&exponent, &math.ln(&radius)?)?)?
    };
    let radial_scale = abs_upper(math, &math.point_f64(radial_exponent)?)?;
    let polar_scale = abs_upper(math, &math.point_f64(polar_multiplier)?)?;
    let azimuth_scale = math.mul(
        &abs_upper(math, &math.point_f64(azimuth_multiplier)?)?,
        &kernel,
    )?;
    let angular_scale = math.hull(&zero, &math.hull(&radial_scale, &polar_scale)?)?;
    let all_scales = math.hull(&angular_scale, &azimuth_scale)?;
    Ok(math.mul(&radial_factor, &all_scales)?)
}

/// Mandelbulb specialization of [`group_power_spectral_norm_upper_bound`].
pub(crate) fn mandelbulb_spectral_norm_upper_bound(
    math: &IntervalMath,
    domain: &IntervalBox3,
    power: f64,
) -> Result<BigInterval, GroupPowerNormError> {
    group_power_spectral_norm_upper_bound(math, domain, power, power, power)
}

fn integer_chebyshev_kernel_bound(
    math: &IntervalMath,
    z: &BigInterval,
    radius: &BigInterval,
    degree_plus_one: u32,
) -> Result<BigInterval, GroupPowerNormError> {
    if !(2..=12).contains(&degree_plus_one) {
        return Err(GroupPowerNormError::OutOfRangeParameter);
    }

    let unit = math.bounds_f64(-1.0, 1.0)?;
    let cosine = math.intersection(&math.div(z, radius)?, &unit)?;
    let two_cosine = math.mul(&math.point_f64(2.0)?, &cosine)?;
    let mut previous = math.point_f64(1.0)?;
    let mut current = two_cosine.clone();
    for _ in 2..degree_plus_one {
        let next = math.sub(&math.mul(&two_cosine, &current)?, &previous)?;
        previous = current;
        current = next;
    }

    let recurrence_bound = abs_upper(math, &current)?;
    let global_bound = math.point_f64(f64::from(degree_plus_one))?;
    if recurrence_bound.upper().repr() <= global_bound.upper().repr() {
        Ok(recurrence_bound)
    } else {
        Ok(global_bound)
    }
}

fn off_axis_kernel_bound(
    math: &IntervalMath,
    z: &BigInterval,
    radius: &BigInterval,
    horizontal_squared: &BigInterval,
    polar_multiplier: f64,
) -> Result<BigInterval, IntervalError> {
    let unit = math.bounds_f64(-1.0, 1.0)?;
    let cosine = math.intersection(&math.div(z, radius)?, &unit)?;
    let theta = math.acos(&cosine)?;
    let scaled_theta = math.mul(&math.point_f64(polar_multiplier)?, &theta)?;
    let numerator = abs_upper(math, &math.sin(&scaled_theta)?)?;
    let sine_theta = math.div(&math.sqrt(horizontal_squared)?, radius)?;
    math.div(&numerator, &sine_theta)
}

fn positive_axis_kernel_bound(
    math: &IntervalMath,
    polar_multiplier: f64,
) -> Result<BigInterval, IntervalError> {
    // For 0 <= theta <= pi/2, |sin(alpha theta)| <= |alpha| theta and
    // theta/sin(theta) <= pi/2.
    let half_pi = math.acos(&math.point_f64(0.0)?)?;
    math.mul(
        &abs_upper(math, &math.point_f64(polar_multiplier)?)?,
        &half_pi,
    )
}

/// Encloses the maximum absolute value represented by a scalar interval.
pub(crate) fn abs_upper(
    math: &IntervalMath,
    value: &BigInterval,
) -> Result<BigInterval, IntervalError> {
    math.magnitude(value)
}

/// Encloses `sqrt(||M||_1 ||M||_inf)`, a certified spectral norm upper bound.
pub(crate) fn matrix_spectral_upper_bound(
    math: &IntervalMath,
    matrix: &[[BigInterval; DIMENSIONS]; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let absolute = matrix_map(matrix, |entry| abs_upper(math, entry))?;
    let row_sums = [
        sum_three(math, &absolute[0])?,
        sum_three(math, &absolute[1])?,
        sum_three(math, &absolute[2])?,
    ];
    let column_sums = [
        sum_three(
            math,
            &[
                absolute[0][0].clone(),
                absolute[1][0].clone(),
                absolute[2][0].clone(),
            ],
        )?,
        sum_three(
            math,
            &[
                absolute[0][1].clone(),
                absolute[1][1].clone(),
                absolute[2][1].clone(),
            ],
        )?,
        sum_three(
            math,
            &[
                absolute[0][2].clone(),
                absolute[1][2].clone(),
                absolute[2][2].clone(),
            ],
        )?,
    ];
    let infinity_norm = hull_three(math, &row_sums)?;
    let one_norm = hull_three(math, &column_sums)?;
    math.sqrt(&math.mul(&one_norm, &infinity_norm)?)
}

/// Encloses `sqrt(sum_i ||H_i||_2^2)` for a three-component Hessian tensor.
pub(crate) fn hessian_tensor_upper_bound(
    math: &IntervalMath,
    tensor: &[[[BigInterval; DIMENSIONS]; DIMENSIONS]; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    let norms = [
        matrix_spectral_upper_bound(math, &tensor[0])?,
        matrix_spectral_upper_bound(math, &tensor[1])?,
        matrix_spectral_upper_bound(math, &tensor[2])?,
    ];
    let squared = [
        math.square(&norms[0])?,
        math.square(&norms[1])?,
        math.square(&norms[2])?,
    ];
    math.sqrt(&sum_three(math, &squared)?)
}

fn sum_three(
    math: &IntervalMath,
    values: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    math.add(&math.add(&values[0], &values[1])?, &values[2])
}

fn hull_three(
    math: &IntervalMath,
    values: &[BigInterval; DIMENSIONS],
) -> Result<BigInterval, IntervalError> {
    math.hull(&math.hull(&values[0], &values[1])?, &values[2])
}

fn matrix_map<F>(
    matrix: &[[BigInterval; DIMENSIONS]; DIMENSIONS],
    mut operation: F,
) -> Result<[[BigInterval; DIMENSIONS]; DIMENSIONS], IntervalError>
where
    F: FnMut(&BigInterval) -> Result<BigInterval, IntervalError>,
{
    Ok([
        [
            operation(&matrix[0][0])?,
            operation(&matrix[0][1])?,
            operation(&matrix[0][2])?,
        ],
        [
            operation(&matrix[1][0])?,
            operation(&matrix[1][1])?,
            operation(&matrix[1][2])?,
        ],
        [
            operation(&matrix[2][0])?,
            operation(&matrix[2][1])?,
            operation(&matrix[2][2])?,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::analyzer::fractal_certification::{BigInterval, IntervalMath};

    fn math() -> IntervalMath {
        IntervalMath::new(128).unwrap()
    }

    fn box_around(math: &IntervalMath, center: [f64; 3], half_width: f64) -> IntervalBox3 {
        IntervalBox3::new([
            math.bounds_f64(center[0] - half_width, center[0] + half_width)
                .unwrap(),
            math.bounds_f64(center[1] - half_width, center[1] + half_width)
                .unwrap(),
            math.bounds_f64(center[2] - half_width, center[2] + half_width)
                .unwrap(),
        ])
    }

    fn point_box(math: &IntervalMath, point: [f64; 3]) -> IntervalBox3 {
        IntervalBox3::new([
            math.point_f64(point[0]).unwrap(),
            math.point_f64(point[1]).unwrap(),
            math.point_f64(point[2]).unwrap(),
        ])
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

    fn group_power_map(
        point: [f64; 3],
        radial_exponent: f64,
        polar_multiplier: f64,
        azimuth_multiplier: f64,
    ) -> [f64; 3] {
        let [x, y, z] = point;
        let rho = x.hypot(y);
        let radius = rho.hypot(z);
        let theta = rho.atan2(z);
        let phi = y.atan2(x);
        let radial = radius.powf(radial_exponent);
        let polar = polar_multiplier * theta;
        let azimuth = azimuth_multiplier * phi;
        [
            radial * polar.sin() * azimuth.cos(),
            radial * polar.sin() * azimuth.sin(),
            radial * polar.cos(),
        ]
    }

    fn finite_difference_jacobian(
        point: [f64; 3],
        radial_exponent: f64,
        polar_multiplier: f64,
        azimuth_multiplier: f64,
    ) -> [[f64; 3]; 3] {
        let step = 1e-6;
        let mut jacobian = [[0.0; 3]; 3];
        for column in 0..3 {
            let mut plus = point;
            let mut minus = point;
            plus[column] += step;
            minus[column] -= step;
            let plus_value =
                group_power_map(plus, radial_exponent, polar_multiplier, azimuth_multiplier);
            let minus_value =
                group_power_map(minus, radial_exponent, polar_multiplier, azimuth_multiplier);
            for row in 0..3 {
                jacobian[row][column] = (plus_value[row] - minus_value[row]) / (2.0 * step);
            }
        }
        jacobian
    }

    fn assert_contains(math: &IntervalMath, interval: &BigInterval, expected: f64) {
        assert!(
            math.contains_f64(interval, expected).unwrap(),
            "{interval:?} did not contain {expected}"
        );
    }

    #[test]
    fn values_enclose_all_supported_power_classes_in_all_quadrants() {
        let math = math();
        let powers = [2.0, 2.5, 7.2, 8.0, 8.5, 12.0];
        let points = [
            [1.0, 0.75, 0.5],
            [-1.0, 0.75, 0.5],
            [-1.0, -0.75, 0.5],
            [1.0, -0.75, 0.5],
        ];
        for power in powers {
            for point in points {
                let evaluation =
                    evaluate(&math, &box_around(&math, point, 1.0 / 1024.0), power).unwrap();
                let expected = reference_map(point, power);
                for (component, expected) in evaluation.components().iter().zip(expected) {
                    assert_contains(&math, component.value(), expected);
                }
            }
        }
    }

    #[test]
    fn sampled_images_jacobians_and_hessians_are_contained() {
        let math = math();
        let center = [1.0, 0.75, 0.5];
        let power = 2.5;
        let evaluation = evaluate(&math, &box_around(&math, center, 1.0 / 128.0), power).unwrap();
        let step = 1e-4;

        for output in 0..3 {
            assert_contains(
                &math,
                evaluation.components()[output].value(),
                reference_map(center, power)[output],
            );
            for axis in 0..3 {
                let mut plus = center;
                let mut minus = center;
                plus[axis] += step;
                minus[axis] -= step;
                let derivative = (reference_map(plus, power)[output]
                    - reference_map(minus, power)[output])
                    / (2.0 * step);
                assert_contains(
                    &math,
                    &evaluation.components()[output].gradient()[axis],
                    derivative,
                );

                let diagonal = (reference_map(plus, power)[output]
                    - 2.0 * reference_map(center, power)[output]
                    + reference_map(minus, power)[output])
                    / step.powi(2);
                assert_contains(
                    &math,
                    &evaluation.components()[output].hessian()[axis][axis],
                    diagonal,
                );
            }
            for row in 0..3 {
                for column in (row + 1)..3 {
                    let mut plus_plus = center;
                    let mut plus_minus = center;
                    let mut minus_plus = center;
                    let mut minus_minus = center;
                    plus_plus[row] += step;
                    plus_plus[column] += step;
                    plus_minus[row] += step;
                    plus_minus[column] -= step;
                    minus_plus[row] -= step;
                    minus_plus[column] += step;
                    minus_minus[row] -= step;
                    minus_minus[column] -= step;
                    let mixed = (reference_map(plus_plus, power)[output]
                        - reference_map(plus_minus, power)[output]
                        - reference_map(minus_plus, power)[output]
                        + reference_map(minus_minus, power)[output])
                        / (4.0 * step.powi(2));
                    assert_contains(
                        &math,
                        &evaluation.components()[output].hessian()[row][column],
                        mixed,
                    );
                }
            }
        }
    }

    fn chebyshev_t8(value: f64) -> f64 {
        let squared = value * value;
        128.0 * squared.powi(4) - 256.0 * squared.powi(3) + 160.0 * squared * squared
            - 32.0 * squared
            + 1.0
    }

    fn chebyshev_u7(value: f64) -> f64 {
        let squared = value * value;
        value * (128.0 * squared.powi(3) - 192.0 * squared * squared + 80.0 * squared - 8.0)
    }

    fn power_eight_oracle(point: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = point;
        let rho = x.hypot(y);
        let radius = rho.hypot(z);
        let cos_theta = z / radius;
        let sin_theta = rho / radius;
        let cos_phi = x / rho;
        let sin_phi = y / rho;
        let cos_polar = chebyshev_t8(cos_theta);
        let sin_polar = sin_theta * chebyshev_u7(cos_theta);
        let cos_azimuth = chebyshev_t8(cos_phi);
        let sin_azimuth = sin_phi * chebyshev_u7(cos_phi);
        let radial = radius.powi(8);
        [
            radial * sin_polar * cos_azimuth,
            radial * sin_polar * sin_azimuth,
            radial * cos_polar,
        ]
    }

    #[test]
    fn power_eight_agrees_with_independent_chebyshev_oracle() {
        let math = math();
        for point in [
            [1.0, 0.5, 0.25],
            [-1.0, 0.5, -0.25],
            [-1.0, -0.5, 0.25],
            [1.0, -0.5, -0.25],
        ] {
            let evaluation = evaluate(&math, &box_around(&math, point, 1.0 / 4096.0), 8.0).unwrap();
            for (component, expected) in evaluation
                .components()
                .iter()
                .zip(power_eight_oracle(point))
            {
                assert_contains(&math, component.value(), expected);
            }
        }
    }

    #[test]
    fn branch_surfaces_have_distinct_inconclusive_reasons() {
        let math = math();
        let seam = IntervalBox3::new([
            math.bounds_f64(-2.0, -1.0).unwrap(),
            math.bounds_f64(-0.25, 0.25).unwrap(),
            math.bounds_f64(0.5, 0.75).unwrap(),
        ]);
        assert_eq!(
            evaluate(&math, &seam, 2.5),
            Err(MandelbulbError::AzimuthSeam)
        );

        let axis = IntervalBox3::new([
            math.bounds_f64(-0.1, 0.1).unwrap(),
            math.bounds_f64(-0.1, 0.1).unwrap(),
            math.bounds_f64(1.0, 1.1).unwrap(),
        ]);
        assert_eq!(evaluate(&math, &axis, 8.0), Err(MandelbulbError::PolarAxis));

        let tiny = box_around(&math, [2e-7, 3e-7, 4e-7], 1e-8);
        assert_eq!(
            evaluate(&math, &tiny, 8.0),
            Err(MandelbulbError::RadiusRegularization)
        );
        assert_eq!(
            evaluate(
                &math,
                &point_box(&math, [RADIUS_REGULARIZATION, 0.0, 0.0]),
                8.0
            ),
            Err(MandelbulbError::RadiusRegularization),
            "the max branch is not differentiable at its exact threshold"
        );
        assert_eq!(
            evaluate(&math, &point_box(&math, [1.0, 0.5, 0.25]), 1.99),
            Err(MandelbulbError::OutOfRangePower)
        );
    }

    #[test]
    fn explicit_seam_charts_preserve_noninteger_winding() {
        let math = math();
        let upper_domain = point_box(&math, [-1.0, 0.0, 0.5]);
        let lower_domain = point_box(&math, [-1.0, -0.0, 0.5]);
        let upper = evaluate_with_chart(&math, &upper_domain, 2.5, Atan2Chart::Upper).unwrap();
        let lower = evaluate_with_chart(&math, &lower_domain, 2.5, Atan2Chart::Lower).unwrap();
        let upper_vertical = upper.components()[1].value();
        let reflected_lower = math.neg(lower.components()[1].value()).unwrap();
        assert!(upper_vertical.strictly_positive());
        assert!(reflected_lower.strictly_positive());
        assert!(upper_vertical.overlaps(&reflected_lower));

        let upper_integer =
            evaluate_with_chart(&math, &upper_domain, 8.0, Atan2Chart::Upper).unwrap();
        let lower_integer =
            evaluate_with_chart(&math, &lower_domain, 8.0, Atan2Chart::Lower).unwrap();
        for (above, below) in upper_integer
            .components()
            .iter()
            .zip(lower_integer.components())
        {
            assert!(above.value().overlaps(below.value()));
        }
    }

    fn frame_jacobian(point: [f64; 3], power: f64) -> [[f64; 3]; 3] {
        let [x, y, z] = point;
        let rho = x.hypot(y);
        let radius = rho.hypot(z);
        let theta = rho.atan2(z);
        let phi = y.atan2(x);
        let polar = power * theta;
        let azimuth = power * phi;
        let input_radial = [x / radius, y / radius, z / radius];
        let input_polar = [
            z * x / (radius * rho),
            z * y / (radius * rho),
            -rho / radius,
        ];
        let input_azimuth = [-y / rho, x / rho, 0.0];
        let output_radial = [
            polar.sin() * azimuth.cos(),
            polar.sin() * azimuth.sin(),
            polar.cos(),
        ];
        let output_polar = [
            polar.cos() * azimuth.cos(),
            polar.cos() * azimuth.sin(),
            -polar.sin(),
        ];
        let output_azimuth = [-azimuth.sin(), azimuth.cos(), 0.0];
        let radial_scale = power * radius.powf(power - 1.0);
        let azimuth_scale = radius * polar.sin() / rho;
        std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                radial_scale
                    * (output_radial[row] * input_radial[column]
                        + output_polar[row] * input_polar[column]
                        + azimuth_scale * output_azimuth[row] * input_azimuth[column])
            })
        })
    }

    #[test]
    fn frame_factorized_point_jacobian_is_contained() {
        let math = math();
        let point = [1.0, -0.75, 0.5];
        let evaluation = evaluate(&math, &box_around(&math, point, 1.0 / 2048.0), 7.2).unwrap();
        for (component, expected_row) in evaluation
            .components()
            .iter()
            .zip(frame_jacobian(point, 7.2))
        {
            for (actual, expected) in component.gradient().iter().zip(expected_row) {
                assert_contains(&math, actual, expected);
            }
        }
    }

    #[test]
    fn integer_power_axis_cell_has_finite_near_global_norm_bound() {
        let math = math();
        let domain = IntervalBox3::new([
            math.bounds_f64(-1e-3, 1e-3).unwrap(),
            math.bounds_f64(-1e-3, 1e-3).unwrap(),
            math.bounds_f64(0.99, 1.01).unwrap(),
        ]);
        let bound = mandelbulb_spectral_norm_upper_bound(&math, &domain, 8.0).unwrap();
        let upper = bound.upper().to_f64().value();
        let global = 64.0 * (1.01_f64.hypot(1e-3_f64.hypot(1e-3))).powi(7);
        assert!((upper / global - 1.0).abs() <= 1e-12);
        assert!(upper <= global * 1.000_001);

        for point in [[1e-4, 2e-4, 1.0], [-2e-4, 1e-4, 1.005]] {
            let sampled = spectral_norm(finite_difference_jacobian(point, 8.0, 8.0, 8.0));
            assert!(upper >= sampled, "{upper} did not contain {sampled}");
        }
    }

    #[test]
    fn noninteger_negative_axis_cell_is_typed_inconclusive() {
        let math = math();
        let domain = IntervalBox3::new([
            math.bounds_f64(-1e-3, 1e-3).unwrap(),
            math.bounds_f64(-1e-3, 1e-3).unwrap(),
            math.bounds_f64(-1.01, -0.99).unwrap(),
        ]);
        assert_eq!(
            group_power_spectral_norm_upper_bound(&math, &domain, 2.5, 2.5, 2.5),
            Err(GroupPowerNormError::NegativeAxisSingularity)
        );
    }

    #[test]
    fn off_axis_group_power_bound_contains_sampled_singular_values() {
        let math = math();
        let domain = box_around(&math, [0.8, -0.6, 0.4], 1.0 / 128.0);
        let bound = group_power_spectral_norm_upper_bound(&math, &domain, 3.0, 2.0, 5.0).unwrap();
        let upper = bound.upper().to_f64().value();

        for point in [[0.8, -0.6, 0.4], [0.807, -0.593, 0.393]] {
            let sampled = spectral_norm(finite_difference_jacobian(point, 3.0, 2.0, 5.0));
            assert!(upper >= sampled, "{upper} did not contain {sampled}");
        }
    }

    fn sampled_matrix(parameter: f64) -> [[f64; 3]; 3] {
        [
            [1.0 + parameter, -0.5, 0.25 * parameter],
            [0.75, 2.0 - parameter, -1.0],
            [0.5 * parameter, 0.125, -0.5 + parameter],
        ]
    }

    fn spectral_norm(matrix: [[f64; 3]; 3]) -> f64 {
        let mut vector = [1.0_f64, 0.5, -0.25];
        for _ in 0..32 {
            let image: [f64; 3] = std::array::from_fn(|row| {
                (0..3)
                    .map(|column| matrix[column][row] * vector[column])
                    .sum::<f64>()
            });
            let normal: [f64; 3] = std::array::from_fn(|row| {
                (0..3)
                    .map(|column| matrix[row][column] * image[column])
                    .sum::<f64>()
            });
            let length = normal.iter().map(|entry| entry * entry).sum::<f64>().sqrt();
            vector = normal.map(|entry| entry / length);
        }
        let image: [f64; 3] = std::array::from_fn(|row| {
            (0..3)
                .map(|column| matrix[column][row] * vector[column])
                .sum::<f64>()
        });
        image.iter().map(|entry| entry * entry).sum::<f64>().sqrt()
    }

    fn interval_matrix(math: &IntervalMath) -> [[BigInterval; 3]; 3] {
        let low = sampled_matrix(-0.5);
        let high = sampled_matrix(0.5);
        std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                math.bounds_f64(
                    low[row][column].min(high[row][column]),
                    low[row][column].max(high[row][column]),
                )
                .unwrap()
            })
        })
    }

    #[test]
    fn norm_helpers_dominate_dense_matrix_and_tensor_samples() {
        let math = math();
        let matrix = interval_matrix(&math);
        let matrix_bound = matrix_spectral_upper_bound(&math, &matrix).unwrap();
        let tensor = [matrix.clone(), matrix.clone(), matrix.clone()];
        let tensor_bound = hessian_tensor_upper_bound(&math, &tensor).unwrap();
        let matrix_upper = matrix_bound.upper().to_f64().value();
        let tensor_upper = tensor_bound.upper().to_f64().value();

        for sample in 0..=100 {
            let parameter = -0.5 + f64::from(sample) / 100.0;
            let norm = spectral_norm(sampled_matrix(parameter));
            assert!(matrix_upper >= norm);
            assert!(tensor_upper >= 3.0_f64.sqrt() * norm);
        }

        let scalar = math.bounds_f64(-3.0, 2.0).unwrap();
        assert_contains(&math, &abs_upper(&math, &scalar).unwrap(), 3.0);
    }
}
