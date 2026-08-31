//! Directed arbitrary-precision intervals for host-side fractal certification.
#![allow(
    dead_code,
    reason = "foundational certification API is integrated by the next certification stage"
)]

use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;

use dashu_float::round::mode::{Down, Up};
use dashu_float::{ConstCache, Context, FBig, FpError, Repr};

pub(crate) mod boundary;
pub(crate) mod exclusion;
pub(crate) mod jet;
pub(crate) mod mandelbulb;
pub(crate) mod segment;
pub(crate) mod stack;
pub(crate) mod subdivision;

type Lower = FBig<Down, 2>;
type Upper = FBig<Up, 2>;
type DecimalLower = FBig<Down, 10>;
type DecimalUpper = FBig<Up, 10>;

/// Keeps binary64 candidate spacing at most 1/4 before integer expansion.
const MAX_SAFE_QUOTIENT_MAGNITUDE: f64 = 1_125_899_906_842_624.0;

/// A closed binary interval whose endpoints carry independent rounding modes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BigInterval {
    lower: Lower,
    upper: Upper,
}

impl BigInterval {
    /// Builds an interval after checking finiteness and endpoint order.
    pub(crate) fn checked(lower: Lower, upper: Upper) -> Result<Self, IntervalError> {
        if !lower.repr().is_finite() || !upper.repr().is_finite() {
            return Err(IntervalError::NonFiniteInput);
        }
        if lower.repr() > upper.repr() {
            return Err(IntervalError::InvalidBounds);
        }
        Ok(Self { lower, upper })
    }

    pub(crate) fn lower(&self) -> &Lower {
        &self.lower
    }

    pub(crate) fn upper(&self) -> &Upper {
        &self.upper
    }

    pub(crate) fn contains_zero(&self) -> bool {
        self.lower.repr() <= &Repr::zero() && self.upper.repr() >= &Repr::zero()
    }

    pub(crate) fn strictly_positive(&self) -> bool {
        self.lower.repr() > &Repr::zero()
    }

    pub(crate) fn contains(&self, other: &Self) -> bool {
        self.lower.repr() <= other.lower.repr() && self.upper.repr() >= other.upper.repr()
    }

    pub(crate) fn strictly_before(&self, other: &Self) -> bool {
        self.upper.repr() < other.lower.repr()
    }

    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.lower.repr() <= other.upper.repr() && other.lower.repr() <= self.upper.repr()
    }
}

/// Explicit failures and certification states for interval operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntervalError {
    InvalidPrecision,
    NonFiniteInput,
    InvalidDecimal,
    InvalidBounds,
    EmptyIntersection,
    DivisionByZero,
    Domain,
    NeedsSplit,
    CacheUnavailable,
    Arithmetic(FpError),
}

/// Continuous branch used for the principal-angle `atan2` enclosure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Atan2Chart {
    Principal,
    Upper,
    Lower,
}

impl fmt::Display for IntervalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrecision => formatter.write_str("interval precision must be nonzero"),
            Self::NonFiniteInput => formatter.write_str("interval inputs must be finite"),
            Self::InvalidDecimal => formatter.write_str("invalid finite decimal"),
            Self::InvalidBounds => formatter.write_str("interval lower bound exceeds upper bound"),
            Self::EmptyIntersection => formatter.write_str("interval intersection is empty"),
            Self::DivisionByZero => formatter.write_str("divisor interval contains zero"),
            Self::Domain => formatter.write_str("interval is outside the operation domain"),
            Self::NeedsSplit => formatter.write_str("interval requires topology-aware splitting"),
            Self::CacheUnavailable => formatter.write_str("constant cache is unavailable"),
            Self::Arithmetic(error) => write!(formatter, "arbitrary-precision arithmetic: {error}"),
        }
    }
}

impl std::error::Error for IntervalError {}

impl From<FpError> for IntervalError {
    fn from(error: FpError) -> Self {
        Self::Arithmetic(error)
    }
}

/// Directed arithmetic contexts and shared transcendental constant state.
pub(crate) struct IntervalMath {
    precision: usize,
    down: Context<Down>,
    up: Context<Up>,
    constants: Mutex<ConstCache>,
}

impl IntervalMath {
    pub(crate) fn new(precision: usize) -> Result<Self, IntervalError> {
        if precision == 0 {
            return Err(IntervalError::InvalidPrecision);
        }
        Ok(Self {
            precision,
            down: Context::new(precision),
            up: Context::new(precision),
            constants: Mutex::new(ConstCache::new()),
        })
    }

    /// Encloses the exact finite binary value represented by an `f64`.
    pub(crate) fn point_f64(&self, value: f64) -> Result<BigInterval, IntervalError> {
        if !value.is_finite() {
            return Err(IntervalError::NonFiniteInput);
        }
        let lower = Lower::try_from(value)
            .map_err(|_| IntervalError::NonFiniteInput)?
            .with_precision(self.precision)
            .value();
        let upper = Upper::try_from(value)
            .map_err(|_| IntervalError::NonFiniteInput)?
            .with_precision(self.precision)
            .value();
        BigInterval::checked(lower, upper)
    }

    /// Encloses an exact finite decimal, converting each endpoint independently.
    pub(crate) fn point_decimal(&self, value: &str) -> Result<BigInterval, IntervalError> {
        let source = value.trim();
        let lower = DecimalLower::from_str(source)
            .map_err(|_| IntervalError::InvalidDecimal)?
            .with_base_and_precision::<2>(self.precision)
            .value();
        let upper = DecimalUpper::from_str(source)
            .map_err(|_| IntervalError::InvalidDecimal)?
            .with_base_and_precision::<2>(self.precision)
            .value();
        BigInterval::checked(lower, upper).map_err(|error| match error {
            IntervalError::NonFiniteInput => IntervalError::InvalidDecimal,
            other => other,
        })
    }

    pub(crate) fn bounds_f64(&self, lower: f64, upper: f64) -> Result<BigInterval, IntervalError> {
        if !lower.is_finite() || !upper.is_finite() {
            return Err(IntervalError::NonFiniteInput);
        }
        if lower > upper {
            return Err(IntervalError::InvalidBounds);
        }
        let lower = Lower::try_from(lower)
            .map_err(|_| IntervalError::NonFiniteInput)?
            .with_precision(self.precision)
            .value();
        let upper = Upper::try_from(upper)
            .map_err(|_| IntervalError::NonFiniteInput)?
            .with_precision(self.precision)
            .value();
        BigInterval::checked(lower, upper)
    }

    pub(crate) fn add(
        &self,
        lhs: &BigInterval,
        rhs: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        Self::interval(
            self.down.add(lhs.lower.repr(), rhs.lower.repr())?.value(),
            self.up.add(lhs.upper.repr(), rhs.upper.repr())?.value(),
        )
    }

    pub(crate) fn sub(
        &self,
        lhs: &BigInterval,
        rhs: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        Self::interval(
            self.down.sub(lhs.lower.repr(), rhs.upper.repr())?.value(),
            self.up.sub(lhs.upper.repr(), rhs.lower.repr())?.value(),
        )
    }

    pub(crate) fn neg(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        Self::interval(
            self.down.sub(&Repr::zero(), value.upper.repr())?.value(),
            self.up.sub(&Repr::zero(), value.lower.repr())?.value(),
        )
    }

    pub(crate) fn mul(
        &self,
        lhs: &BigInterval,
        rhs: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        let lower = [
            self.down.mul(lhs.lower.repr(), rhs.lower.repr())?.value(),
            self.down.mul(lhs.lower.repr(), rhs.upper.repr())?.value(),
            self.down.mul(lhs.upper.repr(), rhs.lower.repr())?.value(),
            self.down.mul(lhs.upper.repr(), rhs.upper.repr())?.value(),
        ]
        .into_iter()
        .min_by(|a, b| a.repr().cmp(b.repr()))
        .ok_or(IntervalError::InvalidBounds)?;
        let upper = [
            self.up.mul(lhs.lower.repr(), rhs.lower.repr())?.value(),
            self.up.mul(lhs.lower.repr(), rhs.upper.repr())?.value(),
            self.up.mul(lhs.upper.repr(), rhs.lower.repr())?.value(),
            self.up.mul(lhs.upper.repr(), rhs.upper.repr())?.value(),
        ]
        .into_iter()
        .max_by(|a, b| a.repr().cmp(b.repr()))
        .ok_or(IntervalError::InvalidBounds)?;
        Self::interval(lower, upper)
    }

    pub(crate) fn square(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        if value.contains_zero() {
            let upper_source = if magnitude_cmp(value.lower.repr(), value.upper.repr()).is_gt() {
                value.lower.repr()
            } else {
                value.upper.repr()
            };
            return Self::interval(
                self.down.mul(&Repr::zero(), &Repr::zero())?.value(),
                self.up.mul(upper_source, upper_source)?.value(),
            );
        }

        let (lower_source, upper_source) = if value.strictly_positive() {
            (value.lower.repr(), value.upper.repr())
        } else {
            (value.upper.repr(), value.lower.repr())
        };
        Self::interval(
            self.down.mul(lower_source, lower_source)?.value(),
            self.up.mul(upper_source, upper_source)?.value(),
        )
    }

    pub(crate) fn div(
        &self,
        numerator: &BigInterval,
        denominator: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        if denominator.contains_zero() {
            return Err(IntervalError::DivisionByZero);
        }
        let lower = [
            self.down
                .div(numerator.lower.repr(), denominator.lower.repr())?
                .value(),
            self.down
                .div(numerator.lower.repr(), denominator.upper.repr())?
                .value(),
            self.down
                .div(numerator.upper.repr(), denominator.lower.repr())?
                .value(),
            self.down
                .div(numerator.upper.repr(), denominator.upper.repr())?
                .value(),
        ]
        .into_iter()
        .min_by(|a, b| a.repr().cmp(b.repr()))
        .ok_or(IntervalError::InvalidBounds)?;
        let upper = [
            self.up
                .div(numerator.lower.repr(), denominator.lower.repr())?
                .value(),
            self.up
                .div(numerator.lower.repr(), denominator.upper.repr())?
                .value(),
            self.up
                .div(numerator.upper.repr(), denominator.lower.repr())?
                .value(),
            self.up
                .div(numerator.upper.repr(), denominator.upper.repr())?
                .value(),
        ]
        .into_iter()
        .max_by(|a, b| a.repr().cmp(b.repr()))
        .ok_or(IntervalError::InvalidBounds)?;
        Self::interval(lower, upper)
    }

    pub(crate) fn sqrt(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        if value.lower.repr() < &Repr::zero() {
            return Err(IntervalError::Domain);
        }
        Self::interval(
            self.down.sqrt(value.lower.repr())?.value(),
            self.up.sqrt(value.upper.repr())?.value(),
        )
    }

    pub(crate) fn exp(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        let mut constants = self
            .constants
            .lock()
            .map_err(|_| IntervalError::CacheUnavailable)?;
        Self::interval(
            self.down
                .exp(value.lower.repr(), Some(&mut constants))?
                .value(),
            self.up
                .exp(value.upper.repr(), Some(&mut constants))?
                .value(),
        )
    }

    pub(crate) fn ln(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        if value.lower.repr() <= &Repr::zero() {
            return Err(IntervalError::Domain);
        }
        let mut constants = self
            .constants
            .lock()
            .map_err(|_| IntervalError::CacheUnavailable)?;
        Self::interval(
            self.down
                .ln(value.lower.repr(), Some(&mut constants))?
                .value(),
            self.up
                .ln(value.upper.repr(), Some(&mut constants))?
                .value(),
        )
    }

    pub(crate) fn acos(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        if value.lower.repr() < &Repr::neg_one() || value.upper.repr() > &Repr::one() {
            return Err(IntervalError::Domain);
        }
        let mut constants = self
            .constants
            .lock()
            .map_err(|_| IntervalError::CacheUnavailable)?;
        Self::interval(
            self.down
                .acos(value.upper.repr(), Some(&mut constants))?
                .value(),
            self.up
                .acos(value.lower.repr(), Some(&mut constants))?
                .value(),
        )
    }

    pub(crate) fn sin(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        self.periodic_range(value, TrigFunction::Sin)
    }

    pub(crate) fn cos(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        self.periodic_range(value, TrigFunction::Cos)
    }

    pub(crate) fn atan2(
        &self,
        y: &BigInterval,
        x: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        self.atan2_chart(y, x, Atan2Chart::Principal)
    }

    pub(crate) fn atan2_chart(
        &self,
        y: &BigInterval,
        x: &BigInterval,
        chart: Atan2Chart,
    ) -> Result<BigInterval, IntervalError> {
        if x.contains_zero() && y.contains_zero() {
            return Err(IntervalError::Domain);
        }
        match chart {
            Atan2Chart::Principal if x.lower.repr() < &Repr::zero() && y.contains_zero() => {
                return Err(IntervalError::NeedsSplit);
            }
            Atan2Chart::Upper if y.lower.repr() < &Repr::zero() => {
                return Err(IntervalError::NeedsSplit);
            }
            Atan2Chart::Lower if y.upper.repr() > &Repr::zero() => {
                return Err(IntervalError::NeedsSplit);
            }
            _ => {}
        }

        let mut constants = self
            .constants
            .lock()
            .map_err(|_| IntervalError::CacheUnavailable)?;
        let pi_lower = self.down.pi::<2>(Some(&mut constants)).value();
        let pi_upper = self.up.pi::<2>(Some(&mut constants)).value();
        let two = Lower::from(2_u8);
        let corners = [
            (y.lower.repr(), x.lower.repr()),
            (y.lower.repr(), x.upper.repr()),
            (y.upper.repr(), x.lower.repr()),
            (y.upper.repr(), x.upper.repr()),
        ];
        let mut lower_candidates = Vec::with_capacity(corners.len());
        let mut upper_candidates = Vec::with_capacity(corners.len());
        for (y_corner, x_corner) in corners {
            if y_corner.significand().is_zero() {
                if x_corner < &Repr::zero() {
                    match chart {
                        Atan2Chart::Upper => {
                            lower_candidates.push(pi_lower.clone());
                            upper_candidates.push(pi_upper.clone());
                        }
                        Atan2Chart::Lower => {
                            lower_candidates
                                .push(self.down.sub(&Repr::zero(), pi_upper.repr())?.value());
                            upper_candidates
                                .push(self.up.sub(&Repr::zero(), pi_lower.repr())?.value());
                        }
                        Atan2Chart::Principal => return Err(IntervalError::NeedsSplit),
                    }
                } else {
                    lower_candidates.push(Lower::from_repr(y_corner.clone(), self.down));
                    upper_candidates.push(Upper::from_repr(y_corner.clone(), self.up));
                }
            } else if x_corner.significand().is_zero() {
                if y_corner < &Repr::zero() {
                    let negative_pi_lower = self.down.sub(&Repr::zero(), pi_upper.repr())?.value();
                    let negative_pi_upper = self.up.sub(&Repr::zero(), pi_lower.repr())?.value();
                    lower_candidates
                        .push(self.down.div(negative_pi_lower.repr(), two.repr())?.value());
                    upper_candidates
                        .push(self.up.div(negative_pi_upper.repr(), two.repr())?.value());
                } else {
                    lower_candidates.push(self.down.div(pi_lower.repr(), two.repr())?.value());
                    upper_candidates.push(self.up.div(pi_upper.repr(), two.repr())?.value());
                }
            } else {
                lower_candidates.push(
                    self.down
                        .atan2(y_corner, x_corner, Some(&mut constants))?
                        .value(),
                );
                upper_candidates.push(
                    self.up
                        .atan2(y_corner, x_corner, Some(&mut constants))?
                        .value(),
                );
            }
        }
        let lower = lower_candidates
            .into_iter()
            .min_by(|a, b| signed_zero_lower_cmp(a.repr(), b.repr()))
            .ok_or(IntervalError::InvalidBounds)?;
        let upper = upper_candidates
            .into_iter()
            .max_by(|a, b| signed_zero_upper_cmp(a.repr(), b.repr()))
            .ok_or(IntervalError::InvalidBounds)?;
        Self::interval(lower, upper)
    }

    #[allow(
        clippy::unused_self,
        reason = "endpoint-preserving operation remains part of the context-owned API"
    )]
    pub(crate) fn hull(
        &self,
        lhs: &BigInterval,
        rhs: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        let lower = if lhs.lower <= rhs.lower {
            lhs.lower.clone()
        } else {
            rhs.lower.clone()
        };
        let upper = if lhs.upper >= rhs.upper {
            lhs.upper.clone()
        } else {
            rhs.upper.clone()
        };
        Self::interval(lower, upper)
    }

    #[allow(
        clippy::unused_self,
        reason = "endpoint-preserving operation remains part of the context-owned API"
    )]
    pub(crate) fn intersection(
        &self,
        lhs: &BigInterval,
        rhs: &BigInterval,
    ) -> Result<BigInterval, IntervalError> {
        let lower = if lhs.lower >= rhs.lower {
            lhs.lower.clone()
        } else {
            rhs.lower.clone()
        };
        let upper = if lhs.upper <= rhs.upper {
            lhs.upper.clone()
        } else {
            rhs.upper.clone()
        };
        if lower.repr() > upper.repr() {
            return Err(IntervalError::EmptyIntersection);
        }
        Self::interval(lower, upper)
    }

    pub(crate) fn width(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        Self::interval(
            self.down
                .sub(value.upper.repr(), value.lower.repr())?
                .value(),
            self.up.sub(value.upper.repr(), value.lower.repr())?.value(),
        )
    }

    /// Outward enclosure of the arithmetic midpoint of an interval.
    pub(crate) fn midpoint(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        let two = self.point_f64(2.0)?;
        Self::interval(
            self.down
                .div(
                    self.down
                        .add(value.lower.repr(), value.upper.repr())?
                        .value()
                        .repr(),
                    two.upper.repr(),
                )?
                .value(),
            self.up
                .div(
                    self.up
                        .add(value.lower.repr(), value.upper.repr())?
                        .value()
                        .repr(),
                    two.lower.repr(),
                )?
                .value(),
        )
    }

    pub(crate) fn magnitude(&self, value: &BigInterval) -> Result<BigInterval, IntervalError> {
        let source = if magnitude_cmp(value.lower.repr(), value.upper.repr()).is_gt() {
            value.lower.repr()
        } else {
            value.upper.repr()
        };
        if source < &Repr::zero() {
            Self::interval(
                self.down.sub(&Repr::zero(), source)?.value(),
                self.up.sub(&Repr::zero(), source)?.value(),
            )
        } else {
            Self::interval(
                self.down.add(source, &Repr::zero())?.value(),
                self.up.add(source, &Repr::zero())?.value(),
            )
        }
    }

    pub(crate) fn contains_f64(
        &self,
        interval: &BigInterval,
        value: f64,
    ) -> Result<bool, IntervalError> {
        let point = self.point_f64(value)?;
        Ok(interval.contains(&point))
    }

    pub(crate) fn contains_decimal(
        &self,
        interval: &BigInterval,
        value: &str,
    ) -> Result<bool, IntervalError> {
        let point = self.point_decimal(value)?;
        Ok(interval.contains(&point))
    }

    fn interval(lower: Lower, upper: Upper) -> Result<BigInterval, IntervalError> {
        BigInterval::checked(lower, upper)
    }

    fn periodic_range(
        &self,
        value: &BigInterval,
        function: TrigFunction,
    ) -> Result<BigInterval, IntervalError> {
        let mut constants = self
            .constants
            .lock()
            .map_err(|_| IntervalError::CacheUnavailable)?;
        let pi_lower = self.down.pi::<2>(Some(&mut constants)).value();
        let pi_upper = self.up.pi::<2>(Some(&mut constants)).value();
        let width_lower = self
            .down
            .sub(value.upper.repr(), value.lower.repr())?
            .value();
        let two = Lower::from(2_u8);
        let period_upper = self.up.mul(pi_upper.repr(), two.repr())?.value();
        if width_lower.repr() >= period_upper.repr() {
            return Self::unit_interval();
        }

        let mut lower_candidates = vec![
            function
                .eval_down(self.down, value.lower.repr(), &mut constants)?
                .value(),
            function
                .eval_down(self.down, value.upper.repr(), &mut constants)?
                .value(),
        ];
        let mut upper_candidates = vec![
            function
                .eval_up(self.up, value.lower.repr(), &mut constants)?
                .value(),
            function
                .eval_up(self.up, value.upper.repr(), &mut constants)?
                .value(),
        ];

        let (first, last) = critical_candidate_range(value, &pi_lower, &pi_upper)?;
        if last.saturating_sub(first) > 32 {
            return Err(IntervalError::NeedsSplit);
        }
        for index in first..=last {
            let critical = self.critical_point(index, &pi_lower, &pi_upper)?;
            if !value.overlaps(&critical) {
                continue;
            }
            match function.extremum(index) {
                Some(-1) => {
                    lower_candidates.push(Lower::from(-1_i8));
                    upper_candidates.push(Upper::from(-1_i8));
                }
                Some(1) => {
                    lower_candidates.push(Lower::from(1_i8));
                    upper_candidates.push(Upper::from(1_i8));
                }
                _ => {}
            }
        }

        let lower = lower_candidates
            .into_iter()
            .min_by(|a, b| a.repr().cmp(b.repr()))
            .ok_or(IntervalError::InvalidBounds)?;
        let upper = upper_candidates
            .into_iter()
            .max_by(|a, b| a.repr().cmp(b.repr()))
            .ok_or(IntervalError::InvalidBounds)?;
        Self::interval(lower, upper)
    }

    fn critical_point(
        &self,
        index: i64,
        pi_lower: &Lower,
        pi_upper: &Upper,
    ) -> Result<BigInterval, IntervalError> {
        let index_repr = Lower::from(index);
        let two = Lower::from(2_u8);
        let (lower_pi, upper_pi) = if index < 0 {
            (pi_upper.repr(), pi_lower.repr())
        } else {
            (pi_lower.repr(), pi_upper.repr())
        };
        let lower_product = self.down.mul(lower_pi, index_repr.repr())?.value();
        let upper_product = self.up.mul(upper_pi, index_repr.repr())?.value();
        Self::interval(
            self.down.div(lower_product.repr(), two.repr())?.value(),
            self.up.div(upper_product.repr(), two.repr())?.value(),
        )
    }

    fn unit_interval() -> Result<BigInterval, IntervalError> {
        Self::interval(Lower::from(-1_i8), Upper::from(1_i8))
    }
}

#[derive(Clone, Copy)]
enum TrigFunction {
    Sin,
    Cos,
}

impl TrigFunction {
    fn eval_down(
        self,
        context: Context<Down>,
        value: &Repr<2>,
        constants: &mut ConstCache,
    ) -> dashu_float::FpResult<Lower> {
        match self {
            Self::Sin => context.sin(value, Some(constants)),
            Self::Cos => context.cos(value, Some(constants)),
        }
    }

    fn eval_up(
        self,
        context: Context<Up>,
        value: &Repr<2>,
        constants: &mut ConstCache,
    ) -> dashu_float::FpResult<Upper> {
        match self {
            Self::Sin => context.sin(value, Some(constants)),
            Self::Cos => context.cos(value, Some(constants)),
        }
    }

    fn extremum(self, index: i64) -> Option<i8> {
        match (self, index.rem_euclid(4)) {
            (Self::Sin, 1) | (Self::Cos, 0) => Some(1),
            (Self::Sin, 3) | (Self::Cos, 2) => Some(-1),
            _ => None,
        }
    }
}

fn critical_candidate_range(
    value: &BigInterval,
    pi_lower: &Lower,
    pi_upper: &Upper,
) -> Result<(i64, i64), IntervalError> {
    let lower = value.lower.to_f64().value();
    let upper = value.upper.to_f64().value();
    if !lower.is_finite() || !upper.is_finite() {
        return Err(IntervalError::NeedsSplit);
    }

    let half_pi_lower = pi_lower.to_f64().value() / 2.0;
    let half_pi_upper = pi_upper.to_f64().value() / 2.0;
    let lower_quotient = if lower.is_sign_negative() {
        lower / half_pi_lower
    } else {
        lower / half_pi_upper
    };
    let upper_quotient = if upper.is_sign_negative() {
        upper / half_pi_upper
    } else {
        upper / half_pi_lower
    };
    // At 2^50, binary64 spacing is at most 1/4 (and remains below one after
    // the directed quotient step). One adjacent-float expansion plus two whole
    // candidate indices therefore strictly over-encloses conversion, division,
    // floor, and ceil uncertainty. Beyond this guard, integer spacing can exceed
    // one and an i64 range inferred from binary64 is not a proof.
    if !lower_quotient.is_finite()
        || !upper_quotient.is_finite()
        || lower_quotient.abs() > MAX_SAFE_QUOTIENT_MAGNITUDE
        || upper_quotient.abs() > MAX_SAFE_QUOTIENT_MAGNITUDE
    {
        return Err(IntervalError::NeedsSplit);
    }
    let first = next_down_f64(lower_quotient).floor() - 2.0;
    let last = next_up_f64(upper_quotient).ceil() + 2.0;
    if first < i64::MIN as f64 || last > i64::MAX as f64 {
        return Err(IntervalError::NeedsSplit);
    }
    Ok((first as i64, last as i64))
}

fn next_down_f64(value: f64) -> f64 {
    if value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = if value.is_sign_positive() {
        value.to_bits() - 1
    } else {
        value.to_bits() + 1
    };
    f64::from_bits(bits)
}

fn next_up_f64(value: f64) -> f64 {
    if value == f64::INFINITY {
        return value;
    }
    if value == 0.0 {
        return f64::from_bits(1);
    }
    let bits = if value.is_sign_positive() {
        value.to_bits() + 1
    } else {
        value.to_bits() - 1
    };
    f64::from_bits(bits)
}

fn signed_zero_lower_cmp(lhs: &Repr<2>, rhs: &Repr<2>) -> std::cmp::Ordering {
    let ordering = lhs.cmp(rhs);
    if ordering.is_eq() && lhs.is_neg_zero() != rhs.is_neg_zero() {
        if lhs.is_neg_zero() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    } else {
        ordering
    }
}

fn signed_zero_upper_cmp(lhs: &Repr<2>, rhs: &Repr<2>) -> std::cmp::Ordering {
    let ordering = lhs.cmp(rhs);
    if ordering.is_eq() && lhs.is_pos_zero() != rhs.is_pos_zero() {
        if lhs.is_pos_zero() {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        }
    } else {
        ordering
    }
}

fn magnitude_cmp(lhs: &Repr<2>, rhs: &Repr<2>) -> std::cmp::Ordering {
    let lhs = if lhs < &Repr::zero() {
        -Lower::from_repr_const(lhs.clone())
    } else {
        Lower::from_repr_const(lhs.clone())
    };
    let rhs = if rhs < &Repr::zero() {
        -Lower::from_repr_const(rhs.clone())
    } else {
        Lower::from_repr_const(rhs.clone())
    };
    lhs.cmp(&rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn math() -> IntervalMath {
        IntervalMath::new(128).expect("valid precision")
    }

    fn interval(math: &IntervalMath, lower: f64, upper: f64) -> BigInterval {
        math.bounds_f64(lower, upper).expect("valid bounds")
    }

    #[test]
    fn arithmetic_covers_all_multiplication_sign_classes() {
        let math = math();
        let cases = [
            ((2.0, 3.0), (4.0, 5.0), (8.0, 15.0)),
            ((-3.0, -2.0), (4.0, 5.0), (-15.0, -8.0)),
            ((-3.0, 2.0), (4.0, 5.0), (-15.0, 10.0)),
            ((-3.0, -2.0), (-5.0, -4.0), (8.0, 15.0)),
            ((-3.0, 2.0), (-5.0, 4.0), (-12.0, 15.0)),
        ];

        for (lhs, rhs, expected) in cases {
            let product = math
                .mul(
                    &interval(&math, lhs.0, lhs.1),
                    &interval(&math, rhs.0, rhs.1),
                )
                .expect("finite product");
            assert!(math.contains_f64(&product, expected.0).unwrap());
            assert!(math.contains_f64(&product, expected.1).unwrap());
        }
    }

    #[test]
    fn basic_arithmetic_and_square_are_enclosures() {
        let math = math();
        let a = interval(&math, -2.0, 3.0);
        let b = interval(&math, 4.0, 5.0);

        assert!(math.contains_f64(&math.add(&a, &b).unwrap(), 2.0).unwrap());
        assert!(math.contains_f64(&math.add(&a, &b).unwrap(), 8.0).unwrap());
        assert!(math.contains_f64(&math.sub(&a, &b).unwrap(), -7.0).unwrap());
        assert!(math.contains_f64(&math.neg(&a).unwrap(), -3.0).unwrap());
        let square = math.square(&a).unwrap();
        assert!(math.contains_f64(&square, 0.0).unwrap());
        assert!(math.contains_f64(&square, 9.0).unwrap());
    }

    #[test]
    fn division_through_zero_is_explicitly_inconclusive() {
        let math = math();
        let result = math.div(&interval(&math, 1.0, 2.0), &interval(&math, -1.0, 1.0));
        assert_eq!(result, Err(IntervalError::DivisionByZero));
    }

    #[test]
    fn domains_and_invalid_inputs_are_rejected_without_panics() {
        let math = math();
        let attempts = std::panic::catch_unwind(|| {
            (
                math.sqrt(&interval(&math, -2.0, -1.0)),
                math.ln(&interval(&math, -1.0, 2.0)),
                math.acos(&interval(&math, -2.0, 0.0)),
                math.point_f64(f64::NAN),
                math.point_f64(f64::INFINITY),
                math.bounds_f64(2.0, 1.0),
                IntervalMath::new(0),
            )
        });
        assert!(attempts.is_ok());
        let (sqrt, ln, acos, nan, infinity, bounds, precision) = attempts.unwrap();
        assert_eq!(sqrt, Err(IntervalError::Domain));
        assert_eq!(ln, Err(IntervalError::Domain));
        assert_eq!(acos, Err(IntervalError::Domain));
        assert_eq!(nan, Err(IntervalError::NonFiniteInput));
        assert_eq!(infinity, Err(IntervalError::NonFiniteInput));
        assert_eq!(bounds, Err(IntervalError::InvalidBounds));
        assert!(matches!(precision, Err(IntervalError::InvalidPrecision)));
    }

    #[test]
    fn decimal_conversion_contains_the_exact_decimal() {
        let math = IntervalMath::new(32).unwrap();
        for source in ["0.1", "-0.1", "1.234567890123456789", "1e-100"] {
            let value = math.point_decimal(source).unwrap();
            assert!(math.contains_decimal(&value, source).unwrap(), "{source}");
        }
        assert_eq!(
            math.point_decimal("not a number"),
            Err(IntervalError::InvalidDecimal)
        );
    }

    #[test]
    fn finite_f64_conversion_is_independently_directed() {
        let math = IntervalMath::new(8).unwrap();
        let value = math.point_f64(0.1).unwrap();
        assert!(math.contains_f64(&value, 0.1).unwrap());
        assert!(
            value.lower().repr() < value.upper().repr(),
            "a 53-bit f64 must widen at 8-bit precision"
        );
    }

    #[test]
    fn monotone_transcendentals_contain_reference_values() {
        let math = math();
        let exp_one = math.exp(&interval(&math, 0.999, 1.001)).unwrap();
        assert!(math
            .contains_decimal(&exp_one, "2.718281828459045")
            .unwrap());

        let ln_two = math.ln(&interval(&math, 1.999, 2.001)).unwrap();
        assert!(math
            .contains_decimal(&ln_two, "0.6931471805599453")
            .unwrap());

        let acos_half = math.acos(&interval(&math, 0.499, 0.501)).unwrap();
        assert!(math
            .contains_decimal(&acos_half, "1.0471975511965977")
            .unwrap());
    }

    #[test]
    fn interval_queries_hull_and_intersection_work() {
        let math = math();
        let a = interval(&math, -2.0, 1.0);
        let b = interval(&math, 0.5, 3.0);
        assert!(a.contains_zero());
        assert!(!a.strictly_positive());
        assert!(b.strictly_positive());

        let hull = math.hull(&a, &b).unwrap();
        assert!(math.contains_f64(&hull, -2.0).unwrap());
        assert!(math.contains_f64(&hull, 3.0).unwrap());
        assert!(hull.contains(&a));
        assert!(a.overlaps(&b));
        assert!(a.strictly_before(&interval(&math, 2.0, 3.0)));
        let overlap = math.intersection(&a, &b).unwrap();
        assert!(math.contains_f64(&overlap, 0.5).unwrap());
        assert!(math.contains_f64(&overlap, 1.0).unwrap());
        assert_eq!(
            math.intersection(&a, &interval(&math, 2.0, 3.0)),
            Err(IntervalError::EmptyIntersection)
        );
        assert!(math.contains_f64(&math.width(&a).unwrap(), 3.0).unwrap());
        assert!(math
            .contains_f64(&math.magnitude(&a).unwrap(), 2.0)
            .unwrap());
    }

    fn next_down(value: f64) -> f64 {
        if value == f64::NEG_INFINITY {
            return value;
        }
        if value == 0.0 {
            return -f64::from_bits(1);
        }
        let bits = if value.is_sign_positive() {
            value.to_bits() - 1
        } else {
            value.to_bits() + 1
        };
        f64::from_bits(bits)
    }

    fn next_up(value: f64) -> f64 {
        if value == f64::INFINITY {
            return value;
        }
        if value == 0.0 {
            return f64::from_bits(1);
        }
        let bits = if value.is_sign_positive() {
            value.to_bits() + 1
        } else {
            value.to_bits() - 1
        };
        f64::from_bits(bits)
    }

    #[test]
    fn sin_and_cos_include_each_kind_of_extremum() {
        let math = math();
        let cases = [
            (std::f64::consts::FRAC_PI_2, true, 1.0),
            (-std::f64::consts::FRAC_PI_2, true, -1.0),
            (0.0, false, 1.0),
            (std::f64::consts::PI, false, -1.0),
        ];

        for (critical, sine, extremum) in cases {
            let input = interval(&math, next_down(critical), next_up(critical));
            let output = if sine {
                math.sin(&input).unwrap()
            } else {
                math.cos(&input).unwrap()
            };
            assert!(
                math.contains_f64(&output, extremum).unwrap(),
                "critical point {critical}"
            );
        }
    }

    #[test]
    fn intervals_one_endpoint_ulp_from_extrema_are_conservative() {
        let math = math();
        let half_pi = std::f64::consts::FRAC_PI_2;
        let left = interval(&math, 1.0, next_down(half_pi));
        let right = interval(&math, next_up(half_pi), 2.0);
        let left_sin = math.sin(&left).unwrap();
        let right_sin = math.sin(&right).unwrap();
        assert!(left_sin.upper().repr() <= &Repr::one());
        assert!(right_sin.upper().repr() <= &Repr::one());

        let pi = std::f64::consts::PI;
        let left = interval(&math, 3.0, next_down(pi));
        let right = interval(&math, next_up(pi), 3.3);
        let left_cos = math.cos(&left).unwrap();
        let right_cos = math.cos(&right).unwrap();
        assert!(left_cos.lower().repr() >= &Repr::neg_one());
        assert!(right_cos.lower().repr() >= &Repr::neg_one());
    }

    #[test]
    fn full_period_returns_the_exact_unit_interval() {
        let math = math();
        for output in [
            math.sin(&interval(&math, -4.0, 4.0)).unwrap(),
            math.cos(&interval(&math, -4.0, 4.0)).unwrap(),
        ] {
            assert_eq!(output.lower().repr(), &Repr::neg_one());
            assert_eq!(output.upper().repr(), &Repr::one());
        }
    }

    #[test]
    fn huge_finite_angles_are_explicitly_inconclusive() {
        let math = math();
        let huge = math.point_f64(2_f64.powi(54)).unwrap();
        assert_eq!(math.sin(&huge), Err(IntervalError::NeedsSplit));
        assert_eq!(math.cos(&huge), Err(IntervalError::NeedsSplit));
    }

    #[test]
    fn candidate_reduction_remains_available_through_mandelbulb_angles() {
        let math = math();
        let pi = std::f64::consts::PI;
        for turn in -12..12 {
            let lower = f64::from(turn) * pi;
            let upper = (f64::from(turn) + 0.25) * pi;
            let input = interval(&math, lower, upper);
            assert!(math.sin(&input).is_ok(), "sin turn {turn}");
            assert!(math.cos(&input).is_ok(), "cos turn {turn}");
        }
    }

    #[test]
    fn trig_ranges_contain_sampled_dyadic_interior_points() {
        let math = math();
        let input = interval(&math, -2.0, 2.0);
        let sine = math.sin(&input).unwrap();
        let cosine = math.cos(&input).unwrap();
        for sample in [-1.75_f64, -1.0, -0.25, 0.25, 1.0, 1.75] {
            assert!(math.contains_f64(&sine, sample.sin()).unwrap());
            assert!(math.contains_f64(&cosine, sample.cos()).unwrap());
        }
    }

    #[test]
    fn atan2_handles_all_quadrants_and_axis_aligned_boxes() {
        let math = math();
        let boxes = [
            ((1.0, 2.0), (1.0, 2.0)),
            ((1.0, 2.0), (-2.0, -1.0)),
            ((-2.0, -1.0), (-2.0, -1.0)),
            ((-2.0, -1.0), (1.0, 2.0)),
            ((1.0, 2.0), (-0.5, 0.5)),
            ((1.0, 2.0), (0.0, 0.0)),
            ((-2.0, -1.0), (-0.5, 0.5)),
            ((-2.0, -1.0), (0.0, 0.0)),
            ((-0.5, 0.5), (1.0, 2.0)),
        ];

        for (y_bounds, x_bounds) in boxes {
            let y = interval(&math, y_bounds.0, y_bounds.1);
            let x = interval(&math, x_bounds.0, x_bounds.1);
            assert!(math.atan2(&y, &x).is_ok(), "{y_bounds:?}, {x_bounds:?}");
        }

        let signed_zero = math
            .atan2(&interval(&math, -0.0, 0.0), &interval(&math, 1.0, 2.0))
            .unwrap();
        assert!(signed_zero.lower().repr().is_neg_zero());
        assert!(signed_zero.upper().repr().is_pos_zero());
    }

    #[test]
    fn atan2_rejects_origin_and_splits_the_negative_x_seam() {
        let math = math();
        assert_eq!(
            math.atan2(&interval(&math, -1.0, 1.0), &interval(&math, -1.0, 1.0)),
            Err(IntervalError::Domain)
        );

        let negative_x = interval(&math, -2.0, -1.0);
        let above = interval(&math, 0.25, 1.0);
        let below = interval(&math, -1.0, -0.25);
        assert!(math.atan2(&above, &negative_x).is_ok());
        assert!(math.atan2(&below, &negative_x).is_ok());
        assert_eq!(
            math.atan2(&interval(&math, -0.25, 0.25), &negative_x),
            Err(IntervalError::NeedsSplit)
        );
    }

    #[test]
    fn atan2_explicit_seam_charts_validate_sign_and_signed_zero() {
        let math = math();
        let negative_x = interval(&math, -2.0, -1.0);
        let positive_zero = interval(&math, 0.0, 0.0);
        let negative_zero = interval(&math, -0.0, -0.0);
        let upper = math
            .atan2_chart(&positive_zero, &negative_x, Atan2Chart::Upper)
            .unwrap();
        let lower = math
            .atan2_chart(&negative_zero, &negative_x, Atan2Chart::Lower)
            .unwrap();
        let reflected_lower = math.neg(&lower).unwrap();
        assert!(upper.strictly_positive());
        assert!(reflected_lower.strictly_positive());
        assert!(upper.overlaps(&reflected_lower));
        assert_eq!(
            math.atan2_chart(
                &interval(&math, -0.25, 0.25),
                &negative_x,
                Atan2Chart::Upper
            ),
            Err(IntervalError::NeedsSplit)
        );
        assert_eq!(
            math.atan2_chart(
                &interval(&math, -0.25, 0.25),
                &negative_x,
                Atan2Chart::Lower
            ),
            Err(IntervalError::NeedsSplit)
        );
    }

    #[test]
    fn atan2_ranges_contain_sampled_dyadic_points() {
        let math = math();
        let cases = [
            ((0.5, 2.0), (0.5, 2.0)),
            ((0.5, 2.0), (-2.0, -0.5)),
            ((-2.0, -0.5), (-2.0, -0.5)),
            ((-2.0, -0.5), (0.5, 2.0)),
            ((1.0, 2.0), (-0.5, 0.5)),
        ];
        for (y_bounds, x_bounds) in cases {
            let output = math
                .atan2(
                    &interval(&math, y_bounds.0, y_bounds.1),
                    &interval(&math, x_bounds.0, x_bounds.1),
                )
                .unwrap();
            for y in [
                0.75 * y_bounds.0 + 0.25 * y_bounds.1,
                f64::midpoint(y_bounds.0, y_bounds.1),
            ] {
                for x in [
                    0.75 * x_bounds.0 + 0.25 * x_bounds.1,
                    f64::midpoint(x_bounds.0, x_bounds.1),
                ] {
                    assert!(math.contains_f64(&output, y.atan2(x)).unwrap());
                }
            }
        }
    }
}
