//! Fixed-size interval automatic differentiation for three spatial variables.

use super::{Atan2Chart, BigInterval, IntervalError, IntervalMath};

const VARIABLES: usize = 3;

/// Value and first derivatives with respect to exactly `(x, y, z)`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Jet1 {
    value: BigInterval,
    gradient: [BigInterval; VARIABLES],
}

impl Jet1 {
    pub(crate) fn constant(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        let zero = math.point_f64(0.0)?;
        Ok(Self {
            value,
            gradient: std::array::from_fn(|_| zero.clone()),
        })
    }

    pub(crate) fn x(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        Self::variable(math, value, 0)
    }

    pub(crate) fn y(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        Self::variable(math, value, 1)
    }

    pub(crate) fn z(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        Self::variable(math, value, 2)
    }

    pub(crate) fn value(&self) -> &BigInterval {
        &self.value
    }

    pub(crate) fn gradient(&self) -> &[BigInterval; VARIABLES] {
        &self.gradient
    }

    pub(crate) fn add(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.add(&self.value, &rhs.value)?,
            gradient: array3(|index| math.add(&self.gradient[index], &rhs.gradient[index]))?,
        })
    }

    pub(crate) fn sub(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.sub(&self.value, &rhs.value)?,
            gradient: array3(|index| math.sub(&self.gradient[index], &rhs.gradient[index]))?,
        })
    }

    pub(crate) fn neg(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.neg(&self.value)?,
            gradient: array3(|index| math.neg(&self.gradient[index]))?,
        })
    }

    pub(crate) fn mul(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.mul(&self.value, &rhs.value)?,
            gradient: array3(|index| {
                let lhs_term = math.mul(&self.gradient[index], &rhs.value)?;
                let rhs_term = math.mul(&self.value, &rhs.gradient[index])?;
                math.add(&lhs_term, &rhs_term)
            })?,
        })
    }

    pub(crate) fn square(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.square(&self.value)?;
        let first = math.add(&self.value, &self.value)?;
        self.unary(math, value, &first)
    }

    pub(crate) fn div(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        self.mul(math, &rhs.reciprocal(math)?)
    }

    pub(crate) fn sqrt(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        if !self.value.strictly_positive() {
            return Err(IntervalError::Domain);
        }
        let value = math.sqrt(&self.value)?;
        let denominator = math.add(&value, &value)?;
        let first = math.div(&math.point_f64(1.0)?, &denominator)?;
        self.unary(math, value, &first)
    }

    pub(crate) fn ln(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.ln(&self.value)?;
        let first = math.div(&math.point_f64(1.0)?, &self.value)?;
        self.unary(math, value, &first)
    }

    pub(crate) fn exp(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.exp(&self.value)?;
        self.unary(math, value.clone(), &value)
    }

    pub(crate) fn sin(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        self.unary(math, math.sin(&self.value)?, &math.cos(&self.value)?)
    }

    pub(crate) fn cos(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let first = math.neg(&math.sin(&self.value)?)?;
        self.unary(math, math.cos(&self.value)?, &first)
    }

    pub(crate) fn atan2(math: &IntervalMath, y: &Self, x: &Self) -> Result<Self, IntervalError> {
        let value = math.atan2(&y.value, &x.value)?;
        let radius_squared = math.add(&math.square(&x.value)?, &math.square(&y.value)?)?;
        let horizontal_weight = math.neg(&math.div(&y.value, &radius_squared)?)?;
        let vertical_weight = math.div(&x.value, &radius_squared)?;
        let gradient = array3(|index| {
            let x_term = math.mul(&horizontal_weight, &x.gradient[index])?;
            let y_term = math.mul(&vertical_weight, &y.gradient[index])?;
            math.add(&x_term, &y_term)
        })?;
        Ok(Self { value, gradient })
    }

    fn variable(
        math: &IntervalMath,
        value: BigInterval,
        axis: usize,
    ) -> Result<Self, IntervalError> {
        let mut result = Self::constant(math, value)?;
        result.gradient[axis] = math.point_f64(1.0)?;
        Ok(result)
    }

    fn reciprocal(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let one = math.point_f64(1.0)?;
        let value = math.div(&one, &self.value)?;
        let first = math.neg(&math.div(&one, &math.square(&self.value)?)?)?;
        self.unary(math, value, &first)
    }

    fn unary(
        &self,
        math: &IntervalMath,
        value: BigInterval,
        first: &BigInterval,
    ) -> Result<Self, IntervalError> {
        Ok(Self {
            value,
            gradient: array3(|index| math.mul(first, &self.gradient[index]))?,
        })
    }
}

/// Value, gradient, and symmetric Hessian for exactly `(x, y, z)`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Jet2 {
    value: BigInterval,
    gradient: [BigInterval; VARIABLES],
    hessian: [[BigInterval; VARIABLES]; VARIABLES],
}

impl Jet2 {
    pub(crate) fn constant(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        let zero = math.point_f64(0.0)?;
        Ok(Self {
            value,
            gradient: std::array::from_fn(|_| zero.clone()),
            hessian: std::array::from_fn(|_| std::array::from_fn(|_| zero.clone())),
        })
    }

    pub(crate) fn x(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        Self::variable(math, value, 0)
    }

    pub(crate) fn y(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        Self::variable(math, value, 1)
    }

    pub(crate) fn z(math: &IntervalMath, value: BigInterval) -> Result<Self, IntervalError> {
        Self::variable(math, value, 2)
    }

    pub(crate) fn value(&self) -> &BigInterval {
        &self.value
    }

    pub(crate) fn gradient(&self) -> &[BigInterval; VARIABLES] {
        &self.gradient
    }

    pub(crate) fn hessian(&self) -> &[[BigInterval; VARIABLES]; VARIABLES] {
        &self.hessian
    }

    pub(crate) fn add(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.add(&self.value, &rhs.value)?,
            gradient: array3(|index| math.add(&self.gradient[index], &rhs.gradient[index]))?,
            hessian: symmetric_matrix3(|row, column| {
                math.add(&self.hessian[row][column], &rhs.hessian[row][column])
            })?,
        })
    }

    pub(crate) fn sub(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.sub(&self.value, &rhs.value)?,
            gradient: array3(|index| math.sub(&self.gradient[index], &rhs.gradient[index]))?,
            hessian: symmetric_matrix3(|row, column| {
                math.sub(&self.hessian[row][column], &rhs.hessian[row][column])
            })?,
        })
    }

    pub(crate) fn neg(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        Ok(Self {
            value: math.neg(&self.value)?,
            gradient: array3(|index| math.neg(&self.gradient[index]))?,
            hessian: symmetric_matrix3(|row, column| math.neg(&self.hessian[row][column]))?,
        })
    }

    pub(crate) fn mul(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        let value = math.mul(&self.value, &rhs.value)?;
        let gradient = array3(|index| {
            let lhs_term = math.mul(&self.gradient[index], &rhs.value)?;
            let rhs_term = math.mul(&self.value, &rhs.gradient[index])?;
            math.add(&lhs_term, &rhs_term)
        })?;
        let hessian = symmetric_matrix3(|row, column| {
            let lhs_hessian = math.mul(&self.hessian[row][column], &rhs.value)?;
            let outer_forward = math.mul(&self.gradient[row], &rhs.gradient[column])?;
            let outer_reverse = math.mul(&self.gradient[column], &rhs.gradient[row])?;
            let rhs_hessian = math.mul(&self.value, &rhs.hessian[row][column])?;
            let lhs_terms = math.add(&lhs_hessian, &outer_forward)?;
            let rhs_terms = math.add(&outer_reverse, &rhs_hessian)?;
            math.add(&lhs_terms, &rhs_terms)
        })?;
        Ok(Self {
            value,
            gradient,
            hessian,
        })
    }

    pub(crate) fn square(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.square(&self.value)?;
        let first = math.add(&self.value, &self.value)?;
        let second = math.point_f64(2.0)?;
        self.unary(math, value, &first, &second)
    }

    pub(crate) fn div(&self, math: &IntervalMath, rhs: &Self) -> Result<Self, IntervalError> {
        self.mul(math, &rhs.reciprocal(math)?)
    }

    pub(crate) fn sqrt(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        if !self.value.strictly_positive() {
            return Err(IntervalError::Domain);
        }
        let value = math.sqrt(&self.value)?;
        let one = math.point_f64(1.0)?;
        let two_root = math.add(&value, &value)?;
        let first = math.div(&one, &two_root)?;
        let four = math.point_f64(4.0)?;
        let second_denominator = math.mul(&four, &math.mul(&self.value, &value)?)?;
        let second = math.neg(&math.div(&one, &second_denominator)?)?;
        self.unary(math, value, &first, &second)
    }

    pub(crate) fn ln(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.ln(&self.value)?;
        let one = math.point_f64(1.0)?;
        let first = math.div(&one, &self.value)?;
        let second = math.neg(&math.div(&one, &math.square(&self.value)?)?)?;
        self.unary(math, value, &first, &second)
    }

    pub(crate) fn exp(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.exp(&self.value)?;
        self.unary(math, value.clone(), &value, &value)
    }

    pub(crate) fn sin(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.sin(&self.value)?;
        let first = math.cos(&self.value)?;
        let second = math.neg(&value)?;
        self.unary(math, value, &first, &second)
    }

    pub(crate) fn cos(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let value = math.cos(&self.value)?;
        let first = math.neg(&math.sin(&self.value)?)?;
        let second = math.neg(&value)?;
        self.unary(math, value, &first, &second)
    }

    pub(crate) fn atan2(math: &IntervalMath, y: &Self, x: &Self) -> Result<Self, IntervalError> {
        Self::atan2_chart(math, y, x, Atan2Chart::Principal)
    }

    pub(crate) fn atan2_chart(
        math: &IntervalMath,
        y: &Self,
        x: &Self,
        chart: Atan2Chart,
    ) -> Result<Self, IntervalError> {
        let value = math.atan2_chart(&y.value, &x.value, chart)?;
        let x_squared = math.square(&x.value)?;
        let y_squared = math.square(&y.value)?;
        let radius_squared = math.add(&x_squared, &y_squared)?;
        let radius_fourth = math.square(&radius_squared)?;
        let horizontal_weight = math.neg(&math.div(&y.value, &radius_squared)?)?;
        let vertical_weight = math.div(&x.value, &radius_squared)?;
        let two_xy = math.add(
            &math.mul(&x.value, &y.value)?,
            &math.mul(&x.value, &y.value)?,
        )?;
        let horizontal_curvature = math.div(&two_xy, &radius_fourth)?;
        let mixed_curvature = math.div(&math.sub(&y_squared, &x_squared)?, &radius_fourth)?;
        let vertical_curvature = math.neg(&horizontal_curvature)?;

        let gradient = array3(|index| {
            let x_term = math.mul(&horizontal_weight, &x.gradient[index])?;
            let y_term = math.mul(&vertical_weight, &y.gradient[index])?;
            math.add(&x_term, &y_term)
        })?;
        let hessian = symmetric_matrix3(|row, column| {
            let linear_x = math.mul(&horizontal_weight, &x.hessian[row][column])?;
            let linear_y = math.mul(&vertical_weight, &y.hessian[row][column])?;
            let quadratic_x = math.mul(
                &horizontal_curvature,
                &math.mul(&x.gradient[row], &x.gradient[column])?,
            )?;
            let cross_gradient = math.add(
                &math.mul(&x.gradient[row], &y.gradient[column])?,
                &math.mul(&y.gradient[row], &x.gradient[column])?,
            )?;
            let quadratic_cross = math.mul(&mixed_curvature, &cross_gradient)?;
            let quadratic_y = math.mul(
                &vertical_curvature,
                &math.mul(&y.gradient[row], &y.gradient[column])?,
            )?;
            let linear = math.add(&linear_x, &linear_y)?;
            let quadratic = math.add(&math.add(&quadratic_x, &quadratic_cross)?, &quadratic_y)?;
            math.add(&linear, &quadratic)
        })?;
        Ok(Self {
            value,
            gradient,
            hessian,
        })
    }

    fn variable(
        math: &IntervalMath,
        value: BigInterval,
        axis: usize,
    ) -> Result<Self, IntervalError> {
        let mut result = Self::constant(math, value)?;
        result.gradient[axis] = math.point_f64(1.0)?;
        Ok(result)
    }

    fn reciprocal(&self, math: &IntervalMath) -> Result<Self, IntervalError> {
        let one = math.point_f64(1.0)?;
        let two = math.point_f64(2.0)?;
        let squared = math.square(&self.value)?;
        let cubed = math.mul(&squared, &self.value)?;
        let value = math.div(&one, &self.value)?;
        let first = math.neg(&math.div(&one, &squared)?)?;
        let second = math.div(&two, &cubed)?;
        self.unary(math, value, &first, &second)
    }

    fn unary(
        &self,
        math: &IntervalMath,
        value: BigInterval,
        first: &BigInterval,
        second: &BigInterval,
    ) -> Result<Self, IntervalError> {
        let gradient = array3(|index| math.mul(first, &self.gradient[index]))?;
        let hessian = symmetric_matrix3(|row, column| {
            let outer = math.mul(&self.gradient[row], &self.gradient[column])?;
            let curvature = math.mul(second, &outer)?;
            let transported = math.mul(first, &self.hessian[row][column])?;
            math.add(&curvature, &transported)
        })?;
        Ok(Self {
            value,
            gradient,
            hessian,
        })
    }
}

fn array3<F>(mut operation: F) -> Result<[BigInterval; VARIABLES], IntervalError>
where
    F: FnMut(usize) -> Result<BigInterval, IntervalError>,
{
    Ok([operation(0)?, operation(1)?, operation(2)?])
}

fn symmetric_matrix3<F>(
    mut operation: F,
) -> Result<[[BigInterval; VARIABLES]; VARIABLES], IntervalError>
where
    F: FnMut(usize, usize) -> Result<BigInterval, IntervalError>,
{
    let diagonal = (operation(0, 0)?, operation(1, 1)?, operation(2, 2)?);
    let mixed = (operation(0, 1)?, operation(0, 2)?, operation(1, 2)?);
    Ok([
        [diagonal.0, mixed.0.clone(), mixed.1.clone()],
        [mixed.0, diagonal.1, mixed.2.clone()],
        [mixed.1, mixed.2, diagonal.2],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::analyzer::fractal_certification::{
        BigInterval, IntervalError, IntervalMath,
    };

    fn math() -> IntervalMath {
        IntervalMath::new(128).unwrap()
    }

    fn point(math: &IntervalMath, value: f64) -> BigInterval {
        math.point_f64(value).unwrap()
    }

    fn assert_contains(math: &IntervalMath, interval: &BigInterval, expected: f64) {
        assert!(
            math.contains_f64(interval, expected).unwrap(),
            "{interval:?} did not contain {expected}"
        );
    }

    #[test]
    fn jet1_exact_polynomial_derivatives() {
        let math = math();
        let x = Jet1::x(&math, point(&math, 2.0)).unwrap();
        let y = Jet1::y(&math, point(&math, 3.0)).unwrap();
        let z = Jet1::z(&math, point(&math, -1.0)).unwrap();
        let result = x
            .mul(&math, &y)
            .unwrap()
            .add(&math, &z.square(&math).unwrap())
            .unwrap();

        assert_contains(&math, result.value(), 7.0);
        for (actual, expected) in result.gradient().iter().zip([3.0, 2.0, -2.0]) {
            assert_contains(&math, actual, expected);
        }
    }

    #[test]
    fn jet1_required_unary_and_quotient_operations() {
        let math = math();
        let x = Jet1::x(&math, point(&math, 4.0)).unwrap();
        let two = Jet1::constant(&math, point(&math, 2.0)).unwrap();
        let quotient = x.div(&math, &two).unwrap();
        assert_contains(&math, quotient.value(), 2.0);
        assert_contains(&math, &quotient.gradient()[0], 0.5);

        let log_root = x.sqrt(&math).unwrap().ln(&math).unwrap();
        assert_contains(&math, &log_root.gradient()[0], 0.125);

        let zero = x.sub(&math, &x).unwrap();
        let transformed = zero
            .neg(&math)
            .unwrap()
            .sin(&math)
            .unwrap()
            .exp(&math)
            .unwrap()
            .cos(&math)
            .unwrap();
        assert_contains(&math, &transformed.gradient()[0], 0.0);
    }

    #[test]
    fn jet2_exact_polynomial_hessian() {
        let math = math();
        let x = Jet2::x(&math, point(&math, 2.0)).unwrap();
        let y = Jet2::y(&math, point(&math, 3.0)).unwrap();
        let z = Jet2::z(&math, point(&math, -1.0)).unwrap();
        let result = x
            .square(&math)
            .unwrap()
            .mul(&math, &y)
            .unwrap()
            .add(&math, &z)
            .unwrap();

        assert_contains(&math, result.value(), 11.0);
        for (actual, expected) in result.gradient().iter().zip([12.0, 4.0, 1.0]) {
            assert_contains(&math, actual, expected);
        }
        let expected = [[6.0, 4.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        for (row, expected_row) in result.hessian().iter().zip(expected) {
            for (actual, expected) in row.iter().zip(expected_row) {
                assert_contains(&math, actual, expected);
            }
        }
    }

    #[test]
    fn jet2_quotient_has_full_second_derivatives() {
        let math = math();
        let x = Jet2::x(&math, point(&math, 2.0)).unwrap();
        let y = Jet2::y(&math, point(&math, 4.0)).unwrap();
        let result = x.div(&math, &y).unwrap();

        for (actual, expected) in result.gradient().iter().zip([0.25, -0.125, 0.0]) {
            assert_contains(&math, actual, expected);
        }
        let expected = [[0.0, -0.0625, 0.0], [-0.0625, 0.0625, 0.0], [0.0, 0.0, 0.0]];
        for (row, expected_row) in result.hessian().iter().zip(expected) {
            for (actual, expected) in row.iter().zip(expected_row) {
                assert_contains(&math, actual, expected);
            }
        }
    }

    #[test]
    fn jet2_transcendental_derivatives_enclose_exact_cases() {
        let math = math();
        let zero = Jet2::x(&math, point(&math, 0.0)).unwrap();
        let one = Jet2::x(&math, point(&math, 1.0)).unwrap();
        let four = Jet2::x(&math, point(&math, 4.0)).unwrap();
        let cases = [
            (zero.exp(&math).unwrap(), 1.0, 1.0, 1.0),
            (one.ln(&math).unwrap(), 0.0, 1.0, -1.0),
            (four.sqrt(&math).unwrap(), 2.0, 0.25, -0.03125),
            (zero.sin(&math).unwrap(), 0.0, 1.0, 0.0),
            (zero.cos(&math).unwrap(), 1.0, 0.0, -1.0),
        ];

        for (jet, value, first, second) in cases {
            assert_contains(&math, jet.value(), value);
            assert_contains(&math, &jet.gradient()[0], first);
            assert_contains(&math, &jet.hessian()[0][0], second);
        }
    }

    #[test]
    fn jet_atan2_handles_quadrants_seams_and_origin() {
        let math = math();
        for (x_value, y_value) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
            let x1 = Jet1::x(&math, point(&math, x_value)).unwrap();
            let y1 = Jet1::y(&math, point(&math, y_value)).unwrap();
            assert!(Jet1::atan2(&math, &y1, &x1).is_ok());

            let x2 = Jet2::x(&math, point(&math, x_value)).unwrap();
            let y2 = Jet2::y(&math, point(&math, y_value)).unwrap();
            let angle = Jet2::atan2(&math, &y2, &x2).unwrap();
            assert_contains(&math, &angle.gradient()[0], -y_value / 2.0);
            assert_contains(&math, &angle.gradient()[1], x_value / 2.0);
        }

        for (x_value, y_value, expected_x, expected_y) in [
            (0.0, 1.0, -1.0, 0.0),
            (0.0, -1.0, 1.0, 0.0),
            (1.0, 0.0, 0.0, 1.0),
        ] {
            let x = Jet2::x(&math, point(&math, x_value)).unwrap();
            let y = Jet2::y(&math, point(&math, y_value)).unwrap();
            let angle = Jet2::atan2(&math, &y, &x).unwrap();
            assert_contains(&math, &angle.gradient()[0], expected_x);
            assert_contains(&math, &angle.gradient()[1], expected_y);
        }

        let seam_x = Jet2::x(&math, math.bounds_f64(-2.0, -1.0).unwrap()).unwrap();
        let seam_y = Jet2::y(&math, math.bounds_f64(-0.25, 0.25).unwrap()).unwrap();
        assert_eq!(
            Jet2::atan2(&math, &seam_y, &seam_x),
            Err(IntervalError::NeedsSplit)
        );
        let origin_x = Jet2::x(&math, math.bounds_f64(-1.0, 1.0).unwrap()).unwrap();
        let origin_y = Jet2::y(&math, math.bounds_f64(-1.0, 1.0).unwrap()).unwrap();
        assert_eq!(
            Jet2::atan2(&math, &origin_y, &origin_x),
            Err(IntervalError::Domain)
        );
    }

    #[test]
    fn jet2_hessian_is_bitwise_symmetric_after_composition() {
        let math = math();
        let x = Jet2::x(&math, point(&math, 1.0)).unwrap();
        let y = Jet2::y(&math, point(&math, 0.5)).unwrap();
        let z = Jet2::z(&math, point(&math, 2.0)).unwrap();
        let result = x
            .mul(&math, &y)
            .unwrap()
            .sin(&math)
            .unwrap()
            .add(&math, &Jet2::atan2(&math, &y, &x).unwrap())
            .unwrap()
            .add(&math, &z.ln(&math).unwrap())
            .unwrap();

        for row in 0..3 {
            for column in 0..3 {
                assert_eq!(result.hessian()[row][column], result.hessian()[column][row]);
            }
        }
    }

    fn scalar_function(x: f64, y: f64, z: f64) -> f64 {
        (x * y).exp() + z.sin() + y.atan2(x)
    }

    fn midpoint(value: &BigInterval) -> f64 {
        f64::midpoint(
            value.lower().to_f64().value(),
            value.upper().to_f64().value(),
        )
    }

    #[test]
    fn finite_difference_diagnostics_match_jet2() {
        let math = math();
        let coordinates = [1.0, 0.5, 0.25];
        let x = Jet2::x(&math, point(&math, coordinates[0])).unwrap();
        let y = Jet2::y(&math, point(&math, coordinates[1])).unwrap();
        let z = Jet2::z(&math, point(&math, coordinates[2])).unwrap();
        let result = x
            .mul(&math, &y)
            .unwrap()
            .exp(&math)
            .unwrap()
            .add(&math, &z.sin(&math).unwrap())
            .unwrap()
            .add(&math, &Jet2::atan2(&math, &y, &x).unwrap())
            .unwrap();

        let step = 1e-4;
        let center = scalar_function(coordinates[0], coordinates[1], coordinates[2]);
        for axis in 0..3 {
            let mut plus = coordinates;
            let mut minus = coordinates;
            plus[axis] += step;
            minus[axis] -= step;
            let finite_gradient = (scalar_function(plus[0], plus[1], plus[2])
                - scalar_function(minus[0], minus[1], minus[2]))
                / (2.0 * step);
            assert!((midpoint(&result.gradient()[axis]) - finite_gradient).abs() < 1e-6);

            let finite_diagonal = (scalar_function(plus[0], plus[1], plus[2]) - 2.0 * center
                + scalar_function(minus[0], minus[1], minus[2]))
                / step.powi(2);
            assert!((midpoint(&result.hessian()[axis][axis]) - finite_diagonal).abs() < 1e-5);
        }
        for row in 0..3 {
            for column in (row + 1)..3 {
                let mut plus_plus = coordinates;
                let mut plus_minus = coordinates;
                let mut minus_plus = coordinates;
                let mut minus_minus = coordinates;
                plus_plus[row] += step;
                plus_plus[column] += step;
                plus_minus[row] += step;
                plus_minus[column] -= step;
                minus_plus[row] -= step;
                minus_plus[column] += step;
                minus_minus[row] -= step;
                minus_minus[column] -= step;
                let evaluate = |values: [f64; 3]| scalar_function(values[0], values[1], values[2]);
                let finite_mixed =
                    (evaluate(plus_plus) - evaluate(plus_minus) - evaluate(minus_plus)
                        + evaluate(minus_minus))
                        / (4.0 * step.powi(2));
                assert!((midpoint(&result.hessian()[row][column]) - finite_mixed).abs() < 1e-6);
            }
        }
    }
}
