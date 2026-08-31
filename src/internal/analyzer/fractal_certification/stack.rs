//! Interval evaluation of one slot from the authored four-slot fractal stack.

use dashu_float::Repr;

use super::jet::Jet2;
use super::mandelbulb::{evaluate_with_chart, IntervalBox3, MandelbulbError};
use super::{Atan2Chart, BigInterval, IntervalError, IntervalMath};
use crate::internal::analyzer::fractal_reference_orbit::StackParams;

const DIMENSIONS: usize = 3;

/// A slot map together with the parameter-seed derivative it contributes.
pub(super) struct SlotEvaluation {
    pub(super) components: [Jet2; DIMENSIONS],
    pub(super) seed_weight: f64,
}

/// A topology or input condition that cannot be enclosed as one smooth slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StackEvaluationError {
    InvalidFormula,
    InvalidParameter,
    ContinuousBranch,
    MengerTranslation,
    Mandelbulb(MandelbulbError),
    Backend(IntervalError),
}

impl From<IntervalError> for StackEvaluationError {
    fn from(error: IntervalError) -> Self {
        Self::Backend(error)
    }
}

impl From<MandelbulbError> for StackEvaluationError {
    fn from(error: MandelbulbError) -> Self {
        Self::Mandelbulb(error)
    }
}

pub(super) fn cycle(params: &StackParams) -> Result<Vec<u8>, StackEvaluationError> {
    let mut result = Vec::with_capacity(32);
    for (&formula, &rate) in params.formulas.iter().zip(&params.rates) {
        let formula = u8::try_from(formula)
            .ok()
            .filter(|formula| *formula <= 10)
            .ok_or(StackEvaluationError::InvalidFormula)?;
        for _ in 0..rate.clamp(0, 8) {
            result.push(formula);
        }
    }
    if result.is_empty() {
        return Err(StackEvaluationError::InvalidParameter);
    }
    Ok(result)
}

pub(super) fn validate(params: &StackParams) -> Result<(), StackEvaluationError> {
    let finite = params.scale.is_finite()
        && params.fold_limit.is_finite()
        && params.min_radius.is_finite()
        && params.fixed_radius.is_finite()
        && params.bailout.is_finite()
        && params.power.is_finite()
        && params.julia_amount.is_finite()
        && params.cocube.is_finite()
        && params.lin_mix.is_finite()
        && params.offset.iter().all(|value| value.is_finite())
        && params.julia.iter().all(|value| value.is_finite())
        && params.lin.iter().all(|value| value.is_finite())
        && params.rot.iter().all(|value| value.is_finite())
        && params.rot_w.iter().all(|value| value.is_finite());
    if !finite
        || params.fold_limit < 0.0
        || params.min_radius <= 0.0
        || params.fixed_radius <= 0.0
        || params.bailout <= 0.0
        || !(2.0..=12.0).contains(&params.power)
        || !(0.0..=1.0).contains(&params.julia_amount)
    {
        return Err(StackEvaluationError::InvalidParameter);
    }
    cycle(params).map(|_| ())
}

pub(super) fn evaluate_slot(
    math: &IntervalMath,
    domain: &IntervalBox3,
    formula: u8,
    params: &StackParams,
    chart: Atan2Chart,
) -> Result<SlotEvaluation, StackEvaluationError> {
    if formula == 5 {
        let bulb = evaluate_with_chart(math, domain, params.power, chart)?;
        return Ok(SlotEvaluation {
            components: bulb.components().clone(),
            seed_weight: 1.0 - params.julia_amount,
        });
    }

    let coordinates = domain.coordinates();
    let mut point = [
        Jet2::x(math, coordinates[0].clone())?,
        Jet2::y(math, coordinates[1].clone())?,
        Jet2::z(math, coordinates[2].clone())?,
    ];
    let scale = constant(math, params.scale)?;
    let one = constant(math, 1.0)?;
    let scale_offset = scale.sub(math, &one)?;
    let offset = [
        constant(math, params.offset[0])?,
        constant(math, params.offset[1])?,
        constant(math, params.offset[2])?,
    ];

    let seed_weight = match formula {
        0 => 0.0,
        1 => {
            point = box_fold(math, &point, params.fold_limit)?;
            let radius_squared = norm_squared(math, &point)?;
            let multiplier = radial_multiplier(
                math,
                &radius_squared,
                params.min_radius * params.min_radius,
                params.fixed_radius * params.fixed_radius,
                params.fixed_radius * params.fixed_radius,
            )?;
            point = scale_vector(math, &scale_vector(math, &point, &multiplier)?, &scale)?;
            1.0 - params.julia_amount
        }
        2 => {
            point[0] = fold_component(math, &point[0], params.fold_limit)?;
            point[1] = fold_component(math, &point[1], params.fold_limit)?;
            let radius_squared = norm_squared(math, &point)?;
            let multiplier = radial_multiplier(
                math,
                &radius_squared,
                params.min_radius * params.min_radius,
                params.fixed_radius * params.fixed_radius,
                params.scale,
            )?;
            point = add_vectors(math, &scale_vector(math, &point, &multiplier)?, &offset)?;
            0.0
        }
        3 => {
            point = [
                branch_abs(math, &point[0])?,
                branch_abs(math, &point[1])?,
                branch_abs(math, &point[2])?,
            ];
            conditional_swap(&mut point, 0, 1)?;
            conditional_swap(&mut point, 0, 2)?;
            conditional_swap(&mut point, 1, 2)?;
            point = subtract_vectors(
                math,
                &scale_vector(math, &point, &scale)?,
                &scale_vector(math, &offset, &scale_offset)?,
            )?;
            let threshold = constant(math, -0.5 * params.offset[2] * (params.scale - 1.0))?;
            let shift = constant(math, params.offset[2] * (params.scale - 1.0))?;
            if strictly_less(point[2].value(), threshold.value()) {
                point[2] = point[2].add(math, &shift)?;
            } else if !greater_or_equal(point[2].value(), threshold.value()) {
                return Err(StackEvaluationError::MengerTranslation);
            }
            0.0
        }
        4 => {
            conditional_reflect_pair(math, &mut point, 0, 1)?;
            conditional_reflect_pair(math, &mut point, 0, 2)?;
            conditional_reflect_pair(math, &mut point, 2, 1)?;
            point = subtract_vectors(
                math,
                &scale_vector(math, &point, &scale)?,
                &scale_vector(math, &offset, &scale_offset)?,
            )?;
            0.0
        }
        6 => {
            point = box_fold(math, &point, params.fold_limit)?;
            let radius_squared = norm_squared(math, &point)?;
            let multiplier = radial_multiplier(
                math,
                &radius_squared,
                params.min_radius * params.min_radius,
                f64::INFINITY,
                1.0,
            )?;
            point = subtract_vectors(math, &scale_vector(math, &point, &multiplier)?, &offset)?;
            0.0
        }
        7 => {
            let old = point.clone();
            point = [
                old[0]
                    .mul(math, &constant(math, params.lin[0])?)?
                    .add(math, &old[1].mul(math, &constant(math, params.lin_mix)?)?)?,
                old[1]
                    .mul(math, &constant(math, params.lin[1])?)?
                    .add(math, &old[2].mul(math, &constant(math, params.lin_mix)?)?)?,
                old[2]
                    .mul(math, &constant(math, params.lin[2])?)?
                    .add(math, &old[0].mul(math, &constant(math, params.lin_mix)?)?)?,
            ];
            0.0
        }
        8 => {
            rotate_pair(math, &mut point, 0, 1, params.rot[0])?;
            rotate_pair(math, &mut point, 1, 2, params.rot[1])?;
            rotate_pair(math, &mut point, 0, 2, params.rot[2])?;
            0.0
        }
        9 => {
            point = [
                branch_abs(math, &point[0])?,
                branch_abs(math, &point[1])?,
                branch_abs(math, &point[2])?,
            ];
            conditional_swap(&mut point, 0, 1)?;
            conditional_swap(&mut point, 1, 2)?;
            let corner = constant(math, params.cocube)?;
            point[2] = corner.sub(math, &branch_abs(math, &point[2].sub(math, &corner)?)?)?;
            point = subtract_vectors(
                math,
                &scale_vector(math, &point, &scale)?,
                &scale_vector(math, &offset, &scale_offset)?,
            )?;
            0.0
        }
        10 => {
            let zero = Jet2::constant(math, math.point_f64(0.0)?)?;
            let mut fourth = zero;
            for (axis, angle) in params.rot_w.iter().copied().enumerate() {
                let adjusted = w_plane_angle(angle);
                rotate_pair_fourth(math, &mut point[axis], &mut fourth, adjusted)?;
            }
            0.0
        }
        _ => return Err(StackEvaluationError::InvalidFormula),
    };
    Ok(SlotEvaluation {
        components: point,
        seed_weight,
    })
}

fn constant(math: &IntervalMath, value: f64) -> Result<Jet2, IntervalError> {
    Jet2::constant(math, math.point_f64(value)?)
}

fn norm_squared(math: &IntervalMath, point: &[Jet2; 3]) -> Result<Jet2, IntervalError> {
    point[0]
        .square(math)?
        .add(math, &point[1].square(math)?)?
        .add(math, &point[2].square(math)?)
}

fn add_vectors(
    math: &IntervalMath,
    lhs: &[Jet2; 3],
    rhs: &[Jet2; 3],
) -> Result<[Jet2; 3], IntervalError> {
    Ok([
        lhs[0].add(math, &rhs[0])?,
        lhs[1].add(math, &rhs[1])?,
        lhs[2].add(math, &rhs[2])?,
    ])
}

fn subtract_vectors(
    math: &IntervalMath,
    lhs: &[Jet2; 3],
    rhs: &[Jet2; 3],
) -> Result<[Jet2; 3], IntervalError> {
    Ok([
        lhs[0].sub(math, &rhs[0])?,
        lhs[1].sub(math, &rhs[1])?,
        lhs[2].sub(math, &rhs[2])?,
    ])
}

fn scale_vector(
    math: &IntervalMath,
    point: &[Jet2; 3],
    scale: &Jet2,
) -> Result<[Jet2; 3], IntervalError> {
    Ok([
        point[0].mul(math, scale)?,
        point[1].mul(math, scale)?,
        point[2].mul(math, scale)?,
    ])
}

fn box_fold(
    math: &IntervalMath,
    point: &[Jet2; 3],
    limit: f64,
) -> Result<[Jet2; 3], StackEvaluationError> {
    Ok([
        fold_component(math, &point[0], limit)?,
        fold_component(math, &point[1], limit)?,
        fold_component(math, &point[2], limit)?,
    ])
}

fn fold_component(
    math: &IntervalMath,
    value: &Jet2,
    limit: f64,
) -> Result<Jet2, StackEvaluationError> {
    let upper = math.point_f64(limit)?;
    let lower = math.point_f64(-limit)?;
    if greater_or_equal(value.value(), &upper) {
        constant(math, 2.0 * limit)?
            .sub(math, value)
            .map_err(Into::into)
    } else if less_or_equal(value.value(), &lower) {
        constant(math, -2.0 * limit)?
            .sub(math, value)
            .map_err(Into::into)
    } else if strictly_between(value.value(), &lower, &upper) {
        Ok(value.clone())
    } else {
        Err(StackEvaluationError::ContinuousBranch)
    }
}

fn radial_multiplier(
    math: &IntervalMath,
    radius_squared: &Jet2,
    lower: f64,
    upper: f64,
    numerator: f64,
) -> Result<Jet2, StackEvaluationError> {
    let lower_bound = constant(math, lower)?;
    let numerator = constant(math, numerator)?;
    if strictly_less(radius_squared.value(), lower_bound.value()) {
        return numerator
            .div(math, &lower_bound)
            .map_err(StackEvaluationError::from);
    }
    if !greater_or_equal(radius_squared.value(), lower_bound.value()) {
        return Err(StackEvaluationError::ContinuousBranch);
    }
    if upper.is_infinite() {
        return numerator
            .div(math, radius_squared)
            .map_err(StackEvaluationError::from);
    }
    let upper_bound = constant(math, upper)?;
    if strictly_less(radius_squared.value(), upper_bound.value()) {
        numerator
            .div(math, radius_squared)
            .map_err(StackEvaluationError::from)
    } else if greater_or_equal(radius_squared.value(), upper_bound.value()) {
        numerator
            .div(math, &upper_bound)
            .map_err(StackEvaluationError::from)
    } else {
        Err(StackEvaluationError::ContinuousBranch)
    }
}

fn branch_abs(math: &IntervalMath, value: &Jet2) -> Result<Jet2, StackEvaluationError> {
    if value.value().lower().repr() >= &Repr::zero() {
        Ok(value.clone())
    } else if value.value().upper().repr() <= &Repr::zero() {
        value.neg(math).map_err(Into::into)
    } else {
        Err(StackEvaluationError::ContinuousBranch)
    }
}

fn conditional_swap(
    point: &mut [Jet2; 3],
    first: usize,
    second: usize,
) -> Result<(), StackEvaluationError> {
    if strictly_less(point[first].value(), point[second].value()) {
        point.swap(first, second);
        Ok(())
    } else if greater_or_equal(point[first].value(), point[second].value()) {
        Ok(())
    } else {
        Err(StackEvaluationError::ContinuousBranch)
    }
}

fn conditional_reflect_pair(
    math: &IntervalMath,
    point: &mut [Jet2; 3],
    first: usize,
    second: usize,
) -> Result<(), StackEvaluationError> {
    let sum = point[first].add(math, &point[second])?;
    if sum.value().upper().repr() < &Repr::zero() {
        let old_first = point[first].clone();
        point[first] = point[second].neg(math)?;
        point[second] = old_first.neg(math)?;
        Ok(())
    } else if sum.value().lower().repr() >= &Repr::zero() {
        Ok(())
    } else {
        Err(StackEvaluationError::ContinuousBranch)
    }
}

fn rotate_pair(
    math: &IntervalMath,
    point: &mut [Jet2; 3],
    first: usize,
    second: usize,
    angle: f64,
) -> Result<(), IntervalError> {
    let mut first_value = point[first].clone();
    let mut second_value = point[second].clone();
    rotate_pair_fourth(math, &mut first_value, &mut second_value, angle)?;
    point[first] = first_value;
    point[second] = second_value;
    Ok(())
}

fn rotate_pair_fourth(
    math: &IntervalMath,
    first: &mut Jet2,
    second: &mut Jet2,
    angle: f64,
) -> Result<(), IntervalError> {
    let cosine = constant(math, angle.cos())?;
    let sine = constant(math, angle.sin())?;
    let old_first = first.clone();
    let old_second = second.clone();
    *first = old_first
        .mul(math, &cosine)?
        .sub(math, &old_second.mul(math, &sine)?)?;
    *second = old_first
        .mul(math, &sine)?
        .add(math, &old_second.mul(math, &cosine)?)?;
    Ok(())
}

#[allow(clippy::approx_constant)]
fn w_plane_angle(angle: f64) -> f64 {
    const HALF_PI: f64 = 1.570_796_3;
    const MARGIN: f64 = 0.06;
    let magnitude = angle.abs();
    let adjusted = if magnitude > HALF_PI - MARGIN && magnitude < HALF_PI + MARGIN {
        if magnitude < HALF_PI {
            HALF_PI - MARGIN
        } else {
            HALF_PI + MARGIN
        }
    } else {
        magnitude
    };
    adjusted.copysign(angle)
}

fn strictly_less(lhs: &BigInterval, rhs: &BigInterval) -> bool {
    lhs.upper().repr() < rhs.lower().repr()
}

fn less_or_equal(lhs: &BigInterval, rhs: &BigInterval) -> bool {
    lhs.upper().repr() <= rhs.lower().repr()
}

fn greater_or_equal(lhs: &BigInterval, rhs: &BigInterval) -> bool {
    lhs.lower().repr() >= rhs.upper().repr()
}

fn strictly_between(value: &BigInterval, lower: &BigInterval, upper: &BigInterval) -> bool {
    value.lower().repr() > lower.upper().repr() && value.upper().repr() < upper.lower().repr()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(formula: i64) -> StackParams {
        StackParams {
            formulas: [formula, 0, 0, 0],
            rates: [1, 0, 0, 0],
            scale: 2.0,
            fold_limit: 1.0,
            min_radius: 0.5,
            fixed_radius: 1.0,
            offset: [1.0, 0.75, 0.5],
            power: 2.5,
            cocube: 0.6,
            lin: [1.2, 0.8, -0.7],
            lin_mix: 0.15,
            rot: [0.2, -0.3, 0.4],
            rot_w: [0.25, -0.35, 0.45],
            ..StackParams::default()
        }
    }

    fn point_box(math: &IntervalMath, point: [f64; 3]) -> IntervalBox3 {
        IntervalBox3::new(point.map(|value| math.point_f64(value).unwrap()))
    }

    fn rotate(first: f64, second: f64, angle: f64) -> (f64, f64) {
        let (sine, cosine) = angle.sin_cos();
        (
            first * cosine - second * sine,
            first * sine + second * cosine,
        )
    }

    fn fold(value: f64, limit: f64) -> f64 {
        value.clamp(-limit, limit) * 2.0 - value
    }

    #[allow(clippy::many_single_char_names)]
    fn reference(mut p: [f64; 3], formula: i64, params: &StackParams) -> [f64; 3] {
        let sc = params.scale;
        let off = params.offset;
        match formula {
            0 => p,
            1 => {
                p = p.map(|value| fold(value, params.fold_limit));
                let q = p.iter().map(|value| value * value).sum::<f64>();
                let m = params.fixed_radius.powi(2)
                    / q.clamp(params.min_radius.powi(2), params.fixed_radius.powi(2));
                p.map(|value| value * m * sc)
            }
            2 => {
                p[0] = fold(p[0], params.fold_limit);
                p[1] = fold(p[1], params.fold_limit);
                let q = p.iter().map(|value| value * value).sum::<f64>();
                let m = sc / q.clamp(params.min_radius.powi(2), params.fixed_radius.powi(2));
                std::array::from_fn(|axis| p[axis] * m + off[axis])
            }
            3 => {
                p = p.map(f64::abs);
                if p[0] < p[1] {
                    p.swap(0, 1);
                }
                if p[0] < p[2] {
                    p.swap(0, 2);
                }
                if p[1] < p[2] {
                    p.swap(1, 2);
                }
                p = std::array::from_fn(|axis| p[axis] * sc - off[axis] * (sc - 1.0));
                let shift = off[2] * (sc - 1.0);
                if p[2] < -0.5 * shift {
                    p[2] += shift;
                }
                p
            }
            4 => {
                for (first, second) in [(0, 1), (0, 2), (2, 1)] {
                    if p[first] + p[second] < 0.0 {
                        let old = p[first];
                        p[first] = -p[second];
                        p[second] = -old;
                    }
                }
                std::array::from_fn(|axis| p[axis] * sc - off[axis] * (sc - 1.0))
            }
            5 => {
                let rho = p[0].hypot(p[1]);
                let r = rho.hypot(p[2]);
                let theta = rho.atan2(p[2]) * params.power;
                let phi = p[1].atan2(p[0]) * params.power;
                let radial = r.powf(params.power);
                [
                    radial * theta.sin() * phi.cos(),
                    radial * theta.sin() * phi.sin(),
                    radial * theta.cos(),
                ]
            }
            6 => {
                p = p.map(|value| fold(value, params.fold_limit));
                let q = p.iter().map(|value| value * value).sum::<f64>();
                std::array::from_fn(|axis| p[axis] / q.max(params.min_radius.powi(2)) - off[axis])
            }
            7 => [
                p[0] * params.lin[0] + p[1] * params.lin_mix,
                p[1] * params.lin[1] + p[2] * params.lin_mix,
                p[2] * params.lin[2] + p[0] * params.lin_mix,
            ],
            8 => {
                (p[0], p[1]) = rotate(p[0], p[1], params.rot[0]);
                (p[1], p[2]) = rotate(p[1], p[2], params.rot[1]);
                (p[0], p[2]) = rotate(p[0], p[2], params.rot[2]);
                p
            }
            9 => {
                p = p.map(f64::abs);
                if p[0] < p[1] {
                    p.swap(0, 1);
                }
                if p[1] < p[2] {
                    p.swap(1, 2);
                }
                p[2] = params.cocube - (p[2] - params.cocube).abs();
                std::array::from_fn(|axis| p[axis] * sc - off[axis] * (sc - 1.0))
            }
            10 => {
                let mut w = 0.0;
                for (axis, angle) in params.rot_w.iter().copied().enumerate() {
                    (p[axis], w) = rotate(p[axis], w, w_plane_angle(angle));
                }
                p
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn every_formula_interval_contains_its_exact_authored_point_map() {
        let math = IntervalMath::new(128).unwrap();
        let point = [0.8, 0.4, 0.2];
        for formula in 0..=10 {
            let params = params(formula);
            let evaluation = evaluate_slot(
                &math,
                &point_box(&math, point),
                formula as u8,
                &params,
                Atan2Chart::Principal,
            )
            .unwrap();
            for (component, expected) in evaluation
                .components
                .iter()
                .zip(reference(point, formula, &params))
            {
                let lower = component.value().lower().to_f64().value();
                let upper = component.value().upper().to_f64().value();
                let tolerance = expected.abs().max(1.0) * 1e-14;
                assert!(
                    lower <= expected + tolerance && upper >= expected - tolerance,
                    "formula {formula} interval [{lower}, {upper}] missed the independently rounded oracle {expected}"
                );
            }
        }
    }

    #[test]
    fn exact_menger_translation_boundary_belongs_to_untranslated_branch() {
        let math = IntervalMath::new(128).unwrap();
        let params = params(3);
        // After sort and scale, z is exactly the strict translation threshold.
        let point = [0.8, 0.6, 0.125];
        let evaluation = evaluate_slot(
            &math,
            &point_box(&math, point),
            3,
            &params,
            Atan2Chart::Principal,
        )
        .unwrap();
        let expected = reference(point, 3, &params);
        assert!(math
            .contains_f64(evaluation.components[2].value(), expected[2])
            .unwrap());
    }

    #[test]
    fn cycle_preserves_four_slot_order_and_integer_rates() {
        let mut params = params(1);
        params.formulas = [8, 3, 5, 10];
        params.rates = [2, 1, 3, 1];
        assert_eq!(cycle(&params).unwrap(), [8, 8, 3, 5, 5, 5, 10]);
    }
}
