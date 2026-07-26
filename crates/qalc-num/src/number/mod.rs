//! The `Number` type — port of libqalculate's `Number` class
//! (`Number.h`/`Number.cc`).
//!
//! A `Number` is simultaneously capable of holding an exact rational, an
//! interval-arithmetic float (lower/upper `BigFloat` bounds), a complex
//! number (imaginary part is another boxed `Number`), or ±infinity.
//!
//! Methods keep the C++ mutate-and-return-`bool` shape: `false` means the
//! operation was not applicable and `self` was left unchanged.

mod arith;
mod compare;
mod complexfn;
mod convert;
mod parse;
mod pow;
mod print;
mod transcendental;
pub mod ieee;
mod integer;
mod lambertw;
mod special;

use crate::context;
use crate::float::{
    bigfloat_from_ratio, bigfloat_is_integer, bigfloat_to_bigint_trunc,
};
use astro_float::{BigFloat, RoundingMode};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};

/// The real-value payload: libqalculate's `n_type` + associated GMP/MPFR
/// storage collapsed into a tagged union.
#[derive(Debug, Clone)]
pub enum RealValue {
    /// `NUMBER_TYPE_RATIONAL`: exact rational (canonical, den > 0).
    Rational(BigRational),
    /// `NUMBER_TYPE_FLOAT`: interval [lower, upper]. In point mode the two
    /// bounds are equal.
    Float { lower: BigFloat, upper: BigFloat },
    /// `NUMBER_TYPE_PLUS_INFINITY`
    PlusInfinity,
    /// `NUMBER_TYPE_MINUS_INFINITY`
    MinusInfinity,
}

/// A number: rational, floating-point interval, complex or infinite.
#[derive(Debug, Clone)]
pub struct Number {
    pub(crate) value: RealValue,
    /// Imaginary part (`i_value`). `None` ≡ real. A present-but-zero
    /// imaginary part is still "real" for `isComplex` purposes.
    pub(crate) imag: Option<Box<Number>>,
    /// `b_approx`
    pub(crate) approx: bool,
    /// `b_imag`: this Number *is* the imaginary part of another Number.
    pub(crate) is_imag_part: bool,
    /// `i_precision`: significant decimal digits, -1 = exact/unset.
    pub(crate) precision: i32,
    /// Variance-formula uncertainty (`INTERVAL_CALCULATION_VARIANCE_FORMULA`,
    /// the default `/set ic 1`).
    ///
    /// The C++ implements the variance formula at the `MathStructure` level:
    /// uncertain values are replaced by variables, the expression is
    /// differentiated symbolically, and the result's uncertainty is
    /// `sqrt(Σ (∂f/∂xᵢ · uᵢ)²)` (`MathStructure-eval.cc:2732`). This port
    /// carries the same quantity forward through each `Number` operation
    /// instead — the chain rule makes the two agree — which is why the
    /// uncertainty is a field rather than a widened interval. `None` means
    /// "no uncertainty"; the value inside never has one of its own.
    ///
    /// A complex uncertainty means the real and imaginary parts have
    /// independent uncertainties, exactly as the C++ treats them (they come
    /// from separate replacement variables).
    pub(crate) unc: Option<Box<Number>>,
}

impl Default for Number {
    fn default() -> Self {
        Number::new()
    }
}

impl Number {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// `Number()` — zero.
    pub fn new() -> Self {
        Number {
            value: RealValue::Rational(BigRational::zero()),
            imag: None,
            approx: false,
            is_imag_part: false,
            precision: -1,
            unc: None,
        }
    }

    /// `Number(long numerator, long denominator, long exp_10)`.
    pub fn from_ints(numerator: i64, denominator: i64, exp_10: i64) -> Self {
        let mut n = Number::new();
        n.set_ints(numerator, denominator, exp_10);
        n
    }

    pub fn from_i64(i: i64) -> Self {
        Number::from_ints(i, 1, 0)
    }

    pub fn from_bigint(z: BigInt) -> Self {
        let mut n = Number::new();
        n.value = RealValue::Rational(BigRational::from_integer(z));
        n
    }

    pub fn from_rational(r: BigRational) -> Self {
        let mut n = Number::new();
        n.value = RealValue::Rational(r);
        n
    }

    /// Construct an interval float from bounds (both must be ordinary).
    pub fn from_interval(lower: BigFloat, upper: BigFloat) -> Self {
        let mut n = Number::new();
        n.value = RealValue::Float { lower, upper };
        n.approx = true;
        n
    }

    /// `set(long numerator, long denominator, long exp_10, ...)`
    pub fn set_ints(&mut self, numerator: i64, denominator: i64, exp_10: i64) {
        let mut r = BigRational::new(BigInt::from(numerator), BigInt::from(denominator));
        if exp_10 != 0 {
            let ten_pow = BigInt::from(10).pow(exp_10.unsigned_abs() as u32);
            if exp_10 > 0 {
                r *= BigRational::from_integer(ten_pow);
            } else {
                r /= BigRational::from_integer(ten_pow);
            }
        }
        self.value = RealValue::Rational(r);
        self.imag = None;
        self.approx = false;
        self.precision = -1;
    }

    /// `setPlusInfinity()`
    pub fn set_plus_infinity(&mut self, keep_precision: bool, keep_imag: bool) {
        self.value = RealValue::PlusInfinity;
        if !keep_imag {
            self.imag = None;
        }
        if !keep_precision {
            self.approx = false;
            self.precision = -1;
        }
    }

    /// `setMinusInfinity()`
    pub fn set_minus_infinity(&mut self, keep_precision: bool, keep_imag: bool) {
        self.value = RealValue::MinusInfinity;
        if !keep_imag {
            self.imag = None;
        }
        if !keep_precision {
            self.approx = false;
            self.precision = -1;
        }
    }

    /// `setFloat(long double)`
    pub fn set_float(&mut self, d: f64) {
        let p = context::bit_precision();
        let f = BigFloat::from_f64(d, p);
        self.value = RealValue::Float { lower: f.clone(), upper: f };
        self.imag = None;
        self.approx = true;
        self.precision = -1;
        self.test_float_result(true);
    }

    /// `set(const Number &o, merge_precision, keep_imag)`
    pub fn set(&mut self, o: &Number, merge_precision: bool, keep_imag: bool) {
        self.value = o.value.clone();
        self.unc = o.unc.clone();
        if !keep_imag {
            self.imag = o.imag.clone();
        }
        if merge_precision {
            self.approx = self.approx || o.approx;
            if o.precision >= 0 && (self.precision < 0 || o.precision < self.precision) {
                self.precision = o.precision;
            }
        } else {
            self.approx = o.approx;
            self.precision = o.precision;
        }
    }

    /// `setInterval(lower, upper)` — set to an interval spanning both numbers.
    pub fn set_interval(&mut self, lo: &Number, hi: &Number, keep_precision: bool) -> bool {
        if !lo.is_real() || !hi.is_real() {
            return false;
        }
        let p = context::bit_precision();
        let l = lo.lower_bound_float(p);
        let u = hi.upper_bound_float(p);
        let (l, u) = if matches!(l.cmp(&u), Some(c) if c > 0) { (u, l) } else { (l, u) };
        self.value = RealValue::Float { lower: l, upper: u };
        self.imag = None;
        self.approx = true;
        if !keep_precision {
            self.precision = -1;
        }
        self.test_float_result(true);
        true
    }

    /// `clear()`
    pub fn clear(&mut self, keep_precision: bool) {
        self.value = RealValue::Rational(BigRational::zero());
        self.imag = None;
        self.unc = None;
        if !keep_precision {
            self.approx = false;
            self.precision = -1;
        }
    }

    /// `clearImaginary()`
    pub fn clear_imaginary(&mut self) {
        self.imag = None;
    }

    /// `setImaginaryPart(o)`
    pub fn set_imaginary_part(&mut self, o: &Number) {
        let mut im = o.clone();
        im.is_imag_part = true;
        self.imag = Some(Box::new(im));
        self.set_precision_and_approximate_from_clone();
    }

    fn set_precision_and_approximate_from_clone(&mut self) {
        if let Some(im) = &self.imag {
            let (a, p) = (im.approx, im.precision);
            if a {
                self.approx = true;
            }
            if p >= 0 && (self.precision < 0 || p < self.precision) {
                self.precision = p;
            }
        }
    }

    /// `markAsImaginaryPart()`
    pub fn mark_as_imaginary_part(&mut self, is_imag: bool) {
        self.is_imag_part = is_imag;
    }

    // ------------------------------------------------------------------
    // Variance-formula uncertainty (see the `unc` field)
    // ------------------------------------------------------------------

    /// The carried uncertainty, if any.
    pub fn variance_uncertainty(&self) -> Option<&Number> {
        self.unc.as_deref()
    }

    /// Attach `u` as the carried uncertainty, combining it in quadrature with
    /// any uncertainty already present (independent contributions add as
    /// `sqrt(u₁² + u₂²)`, per-component for a complex uncertainty).
    pub fn add_variance_uncertainty(&mut self, u: &Number) {
        if u.is_zero() && !u.has_imaginary_part() {
            return;
        }
        let mut u = u.clone();
        u.unc = None;
        u.imag = u.imag.take().map(|mut im| {
            im.unc = None;
            im
        });
        let combined = match self.unc.take() {
            None => u,
            Some(prev) => match quadrature(&prev, &u) {
                Some(c) => c,
                None => *prev,
            },
        };
        if combined.is_zero() && !combined.has_imaginary_part() {
            return;
        }
        self.approx = true;
        self.unc = Some(Box::new(combined));
    }

    /// Drop the carried uncertainty and return it.
    pub(crate) fn take_variance_uncertainty(&mut self) -> Option<Number> {
        self.unc.take().map(|b| *b)
    }

    /// Fold the carried uncertainty into the value as a symmetric interval,
    /// which is what the printer and any interval-mode consumer expects.
    pub fn resolve_variance_uncertainty(&mut self) {
        let Some(u) = self.unc.take() else { return };
        self.set_uncertainty(&u);
    }

    // ------------------------------------------------------------------
    // Internal access
    // ------------------------------------------------------------------

    pub fn internal_rational(&self) -> Option<&BigRational> {
        match &self.value {
            RealValue::Rational(r) => Some(r),
            _ => None,
        }
    }

    pub fn internal_imaginary(&self) -> Option<&Number> {
        self.imag.as_deref()
    }

    /// `setToFloatingPoint()` — convert exact rational to a float interval.
    pub fn set_to_floating_point(&mut self) -> bool {
        if let RealValue::Rational(r) = &self.value {
            let p = context::bit_precision();
            let (lower, upper) = if context::create_interval() {
                (
                    bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::Down),
                    bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::Up),
                )
            } else {
                let f = bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::ToEven);
                (f.clone(), f)
            };
            self.value = RealValue::Float { lower, upper };
        }
        matches!(self.value, RealValue::Float { .. })
    }

    /// Lower interval bound of the real part as BigFloat at precision `p`.
    pub(crate) fn lower_bound_float(&self, p: usize) -> BigFloat {
        match &self.value {
            RealValue::Rational(r) => bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::Down),
            RealValue::Float { lower, .. } => lower.clone(),
            RealValue::PlusInfinity => BigFloat::from_f64(f64::INFINITY, p),
            RealValue::MinusInfinity => BigFloat::from_f64(f64::NEG_INFINITY, p),
        }
    }

    /// Upper interval bound of the real part as BigFloat at precision `p`.
    pub(crate) fn upper_bound_float(&self, p: usize) -> BigFloat {
        match &self.value {
            RealValue::Rational(r) => bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::Up),
            RealValue::Float { upper, .. } => upper.clone(),
            RealValue::PlusInfinity => BigFloat::from_f64(f64::INFINITY, p),
            RealValue::MinusInfinity => BigFloat::from_f64(f64::NEG_INFINITY, p),
        }
    }

    // ------------------------------------------------------------------
    // Result validation (`testFloatResult` / `testInteger`)
    // ------------------------------------------------------------------

    /// Port of `testFloatResult`: normalize a float result — collapse
    /// infinities, order bounds, demote exact integers back to rational.
    /// Returns false when the result is invalid (NaN).
    pub(crate) fn test_float_result(&mut self, allow_infinite_result: bool) -> bool {
        if let RealValue::Float { lower, upper } = &mut self.value {
            if lower.is_nan() || upper.is_nan() {
                return false;
            }
            if let Some(c) = lower.cmp(upper) {
                if c > 0 {
                    std::mem::swap(lower, upper);
                }
            }
            let li = lower.is_inf();
            let ui = upper.is_inf();
            if li && ui && lower.is_inf_pos() == upper.is_inf_pos() {
                if !allow_infinite_result {
                    return false;
                }
                let plus = lower.is_inf_pos();
                self.value = if plus { RealValue::PlusInfinity } else { RealValue::MinusInfinity };
                self.approx = true;
                return true;
            }
            self.test_integer();
        }
        true
    }

    /// Port of `testInteger`: if the float interval is a point on an exact
    /// integer, demote back to rational (load-bearing for exactness).
    pub(crate) fn test_integer(&mut self) {
        if let RealValue::Float { lower, upper } = &self.value {
            if !lower.is_inf()
                && !upper.is_inf()
                && lower == upper
                && bigfloat_is_integer(lower)
            {
                if let Some(z) = bigfloat_to_bigint_trunc(lower) {
                    self.value = RealValue::Rational(BigRational::from_integer(z));
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Approximation / precision
    // ------------------------------------------------------------------

    pub fn is_approximate(&self) -> bool {
        self.approx
            || self.imag.as_ref().is_some_and(|i| i.is_approximate())
    }

    pub fn is_floating_point(&self) -> bool {
        matches!(self.value, RealValue::Float { .. })
    }

    pub fn set_approximate(&mut self, is_approx: bool) {
        if is_approx != self.is_approximate() {
            if is_approx {
                self.approx = true;
            } else {
                self.approx = false;
                self.precision = -1;
                if let Some(im) = &mut self.imag {
                    im.set_approximate(false);
                }
            }
        }
    }

    pub fn precision(&self) -> i32 {
        self.precision
    }

    pub fn set_precision(&mut self, prec: i32) {
        self.precision = prec;
        if prec >= 0 {
            self.approx = true;
        }
    }

    /// `setPrecisionAndApproximateFrom(o)`
    pub fn set_precision_and_approximate_from(&mut self, o: &Number) {
        if o.precision >= 0 && (self.precision < 0 || o.precision < self.precision) {
            self.precision = o.precision;
        }
        if o.approx {
            self.approx = true;
        }
    }

    pub fn is_interval(&self, _ignore_imag: bool) -> bool {
        match &self.value {
            RealValue::Float { lower, upper } => lower != upper,
            _ => false,
        }
    }

    // ------------------------------------------------------------------
    // Type predicates (real-part unless stated)
    // ------------------------------------------------------------------

    pub fn is_infinite(&self, _ignore_imag: bool) -> bool {
        matches!(self.value, RealValue::PlusInfinity | RealValue::MinusInfinity)
    }

    pub fn is_plus_infinity(&self) -> bool {
        matches!(self.value, RealValue::PlusInfinity) && !self.has_imaginary_part()
    }

    pub fn is_minus_infinity(&self) -> bool {
        matches!(self.value, RealValue::MinusInfinity) && !self.has_imaginary_part()
    }

    pub fn includes_infinity(&self) -> bool {
        let re = match &self.value {
            RealValue::PlusInfinity | RealValue::MinusInfinity => true,
            RealValue::Float { lower, upper } => lower.is_inf() || upper.is_inf(),
            RealValue::Rational(_) => false,
        };
        re || self.imag.as_ref().is_some_and(|i| i.includes_infinity())
    }

    pub fn has_imaginary_part(&self) -> bool {
        self.imag.as_ref().is_some_and(|i| !i.is_zero())
    }

    pub fn has_real_part(&self) -> bool {
        !self.real_part_is_zero_internal()
    }

    fn real_part_is_zero_internal(&self) -> bool {
        match &self.value {
            RealValue::Rational(r) => r.is_zero(),
            RealValue::Float { lower, upper } => lower.is_zero() && upper.is_zero(),
            _ => false,
        }
    }

    pub fn is_complex(&self) -> bool {
        self.has_imaginary_part()
    }

    pub fn is_integer(&self) -> bool {
        !self.has_imaginary_part()
            && matches!(&self.value, RealValue::Rational(r) if r.denom().is_one())
    }

    pub fn is_rational(&self) -> bool {
        !self.has_imaginary_part() && matches!(self.value, RealValue::Rational(_))
    }

    pub fn is_real(&self) -> bool {
        !self.has_imaginary_part() && !matches!(self.value, RealValue::PlusInfinity | RealValue::MinusInfinity)
    }

    pub fn is_fraction(&self) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        match &self.value {
            RealValue::Rational(r) => r.numer().magnitude() < r.denom().magnitude(),
            _ => false,
        }
    }

    pub fn is_zero(&self) -> bool {
        if self.imag.as_ref().is_some_and(|i| !i.is_zero()) {
            return false;
        }
        self.real_part_is_zero_internal()
    }

    /// True if zero is not contained in this number.
    pub fn is_nonzero(&self) -> bool {
        if self.imag.as_ref().is_some_and(|i| i.is_nonzero()) {
            return true;
        }
        match &self.value {
            RealValue::Rational(r) => !r.is_zero(),
            RealValue::Float { lower, upper } => {
                matches!(lower.sign(), Some(astro_float::Sign::Pos)) && !lower.is_zero()
                    || matches!(upper.sign(), Some(astro_float::Sign::Neg)) && !upper.is_zero()
            }
            RealValue::PlusInfinity | RealValue::MinusInfinity => true,
        }
    }

    pub fn is_one(&self) -> bool {
        !self.has_imaginary_part()
            && matches!(&self.value, RealValue::Rational(r) if r.is_one())
    }

    pub fn is_two(&self) -> bool {
        !self.has_imaginary_part()
            && matches!(&self.value, RealValue::Rational(r) if r.denom().is_one() && *r.numer() == BigInt::from(2))
    }

    pub fn is_minus_one(&self) -> bool {
        !self.has_imaginary_part()
            && matches!(&self.value, RealValue::Rational(r) if r.denom().is_one() && *r.numer() == BigInt::from(-1))
    }

    pub fn is_i(&self) -> bool {
        self.real_part_is_zero_internal()
            && self.imag.as_ref().is_some_and(|i| i.is_one())
    }

    pub fn is_minus_i(&self) -> bool {
        self.real_part_is_zero_internal()
            && self.imag.as_ref().is_some_and(|i| i.is_minus_one())
    }

    /// Real part < 0 (whole interval below zero for floats).
    pub fn is_negative(&self) -> bool {
        !self.has_imaginary_part() && self.real_part_is_negative()
    }

    pub fn is_positive(&self) -> bool {
        !self.has_imaginary_part() && self.real_part_is_positive()
    }

    pub fn is_non_negative(&self) -> bool {
        !self.has_imaginary_part()
            && match &self.value {
                RealValue::Rational(r) => !r.is_negative(),
                RealValue::Float { lower, .. } => {
                    lower.is_zero() || matches!(lower.sign(), Some(astro_float::Sign::Pos))
                }
                RealValue::PlusInfinity => true,
                RealValue::MinusInfinity => false,
            }
    }

    pub fn is_non_positive(&self) -> bool {
        !self.has_imaginary_part()
            && match &self.value {
                RealValue::Rational(r) => !r.is_positive(),
                RealValue::Float { upper, .. } => {
                    upper.is_zero() || matches!(upper.sign(), Some(astro_float::Sign::Neg))
                }
                RealValue::MinusInfinity => true,
                RealValue::PlusInfinity => false,
            }
    }

    pub(crate) fn real_part_is_negative(&self) -> bool {
        match &self.value {
            RealValue::Rational(r) => r.is_negative(),
            RealValue::Float { upper, .. } => {
                !upper.is_zero() && matches!(upper.sign(), Some(astro_float::Sign::Neg))
            }
            RealValue::MinusInfinity => true,
            RealValue::PlusInfinity => false,
        }
    }

    pub(crate) fn real_part_is_positive(&self) -> bool {
        match &self.value {
            RealValue::Rational(r) => r.is_positive(),
            RealValue::Float { lower, .. } => {
                !lower.is_zero() && matches!(lower.sign(), Some(astro_float::Sign::Pos))
            }
            RealValue::PlusInfinity => true,
            RealValue::MinusInfinity => false,
        }
    }

    pub fn is_even(&self) -> bool {
        self.is_integer()
            && matches!(&self.value, RealValue::Rational(r) if (r.numer() % 2i32).is_zero())
    }

    pub fn is_odd(&self) -> bool {
        self.is_integer()
            && matches!(&self.value, RealValue::Rational(r) if !(r.numer() % 2i32).is_zero())
    }

    pub fn numerator_is_one(&self) -> bool {
        matches!(&self.value, RealValue::Rational(r) if r.numer().is_one())
    }

    pub fn numerator_is_minus_one(&self) -> bool {
        matches!(&self.value, RealValue::Rational(r) if *r.numer() == BigInt::from(-1))
    }

    pub fn numerator_is_even(&self) -> bool {
        matches!(&self.value, RealValue::Rational(r) if (r.numer() % 2i32).is_zero())
    }

    pub fn denominator_is_even(&self) -> bool {
        matches!(&self.value, RealValue::Rational(r) if (r.denom() % 2i32).is_zero())
    }

    pub fn denominator_is_two(&self) -> bool {
        matches!(&self.value, RealValue::Rational(r) if *r.denom() == BigInt::from(2))
    }

    // ------------------------------------------------------------------
    // Parts
    // ------------------------------------------------------------------

    pub fn real_part(&self) -> Number {
        let mut n = self.clone();
        n.imag = None;
        n.is_imag_part = false;
        // A complex uncertainty holds one component per part.
        n.unc = self.unc.as_ref().and_then(|u| {
            let r = u.real_part();
            (!r.is_zero()).then(|| Box::new(r))
        });
        n
    }

    pub fn imaginary_part(&self) -> Number {
        match &self.imag {
            Some(im) => {
                let mut n = (**im).clone();
                n.is_imag_part = false;
                n.unc = self.unc.as_ref().and_then(|u| {
                    let i = u.imaginary_part();
                    (!i.is_zero()).then(|| Box::new(i))
                });
                n
            }
            None => Number::new(),
        }
    }

    pub fn numerator(&self) -> Number {
        match &self.value {
            RealValue::Rational(r) => Number::from_bigint(r.numer().clone()),
            _ => self.clone(),
        }
    }

    pub fn denominator(&self) -> Number {
        match &self.value {
            RealValue::Rational(r) => Number::from_bigint(r.denom().clone()),
            _ => Number::from_i64(1),
        }
    }

    pub fn lower_end_point(&self) -> Number {
        match &self.value {
            RealValue::Float { lower, .. } => {
                let mut n = Number::new();
                n.value = RealValue::Float { lower: lower.clone(), upper: lower.clone() };
                n.approx = true;
                n.test_integer();
                n
            }
            _ => self.clone(),
        }
    }

    pub fn upper_end_point(&self) -> Number {
        match &self.value {
            RealValue::Float { upper, .. } => {
                let mut n = Number::new();
                n.value = RealValue::Float { lower: upper.clone(), upper: upper.clone() };
                n.approx = true;
                n.test_integer();
                n
            }
            _ => self.clone(),
        }
    }

    // ------------------------------------------------------------------
    // Boolean
    // ------------------------------------------------------------------

    pub fn get_boolean(&self) -> i32 {
        if self.is_nonzero() {
            1
        } else if self.is_zero() {
            0
        } else {
            -1
        }
    }

    pub fn set_true(&mut self, is_true: bool) {
        self.clear(false);
        if is_true {
            self.value = RealValue::Rational(BigRational::one());
        }
    }

    pub fn set_false(&mut self) {
        self.set_true(false);
    }

    pub fn set_logical_not(&mut self) {
        let b = self.get_boolean();
        self.set_true(b == 0);
    }

    // ------------------------------------------------------------------
    // Constants
    // ------------------------------------------------------------------

    /// Set to pi at current working precision.
    pub fn pi(&mut self) {
        let p = context::bit_precision();
        let (lo, hi) = context::with_consts(|cc| {
            (cc.pi(p, RoundingMode::Down), cc.pi(p, RoundingMode::Up))
        });
        self.value = if context::create_interval() {
            RealValue::Float { lower: lo, upper: hi }
        } else {
            let f = context::with_consts(|cc| cc.pi(p, RoundingMode::ToEven));
            RealValue::Float { lower: f.clone(), upper: f }
        };
        self.imag = None;
        self.approx = true;
        self.precision = -1;
    }

    /// Set to e at current working precision.
    pub fn e(&mut self) {
        let p = context::bit_precision();
        self.value = if context::create_interval() {
            let (lo, hi) = context::with_consts(|cc| {
                (cc.e(p, RoundingMode::Down), cc.e(p, RoundingMode::Up))
            });
            RealValue::Float { lower: lo, upper: hi }
        } else {
            let f = context::with_consts(|cc| cc.e(p, RoundingMode::ToEven));
            RealValue::Float { lower: f.clone(), upper: f }
        };
        self.imag = None;
        self.approx = true;
        self.precision = -1;
    }
}

/// Componentwise `sqrt(a² + b²)` of two uncertainties.
///
/// The real and imaginary components stand for independent variables, so
/// they combine separately — the same thing the C++ variance formula does
/// when it sums the squared partial derivatives.
pub(crate) fn quadrature(a: &Number, b: &Number) -> Option<Number> {
    fn one(x: Number, y: Number) -> Option<Number> {
        if x.is_zero() {
            return Some(y);
        }
        if y.is_zero() {
            return Some(x);
        }
        let mut s = x;
        let mut t = y;
        if !s.square() || !t.square() || !s.add(&t) || !s.sqrt() {
            return None;
        }
        Some(s)
    }
    let re = one(a.real_part(), b.real_part())?;
    let im = one(a.imaginary_part(), b.imaginary_part())?;
    let mut r = re;
    if !im.is_zero() {
        r.set_imaginary_part(&im);
    }
    Some(r)
}

impl Number {
    /// Propagate an argument's uncertainty through a function whose complex
    /// derivative at that argument is `d`, and attach the result to `self`.
    ///
    /// With `d = p + qi` and an argument uncertainty of `uₓ` on the real
    /// component and `u_y` on the imaginary one,
    /// `δf = (p + qi)(δx + i·δy)`, so the result's components carry
    /// `sqrt((p·uₓ)² + (q·u_y)²)` and `sqrt((q·uₓ)² + (p·u_y)²)`.
    pub(crate) fn propagate_uncertainty(&mut self, d: &Number, u: &Number) -> bool {
        let (mut p, mut q) = (d.real_part(), d.imaginary_part());
        p.unc = None;
        q.unc = None;
        let (ux, uy) = (u.real_part(), u.imaginary_part());
        let term = |a: &Number, b: &Number| -> Option<Number> {
            let mut t = a.clone();
            if !t.multiply(b) || !t.abs() {
                return None;
            }
            Some(t)
        };
        let (Some(pux), Some(quy), Some(qux), Some(puy)) = (
            term(&p, &ux),
            term(&q, &uy),
            term(&q, &ux),
            term(&p, &uy),
        ) else {
            return false;
        };
        let (Some(re), Some(im)) = (quadrature(&pux, &quy), quadrature(&qux, &puy)) else {
            return false;
        };
        let mut total = re;
        if !im.is_zero() {
            total.set_imaginary_part(&im);
        }
        self.add_variance_uncertainty(&total);
        true
    }

    /// Apply the unary operation `op`, carrying any uncertainty forward by
    /// the operation's derivative `deriv` evaluated at the original value.
    ///
    /// The uncertainty is detached before `op` runs, so nested `Number`
    /// operations inside the implementation (and inside `deriv`) never see
    /// it and cannot double-count.
    pub(crate) fn uncertain_unary(
        &mut self,
        deriv: impl FnOnce(&Number) -> Option<Number>,
        op: impl FnOnce(&mut Number) -> bool,
    ) -> bool {
        let Some(u) = self.take_variance_uncertainty() else {
            return op(self);
        };
        let x = self.clone();
        if !op(self) {
            self.unc = Some(Box::new(u));
            return false;
        }
        self.unc = None;
        if let Some(d) = deriv(&x) {
            self.propagate_uncertainty(&d, &u);
        }
        true
    }

    /// Two-operand form of [`Number::uncertain_unary`]; each operand's
    /// uncertainty is pushed through its own partial derivative and the two
    /// contributions combine in quadrature.
    pub(crate) fn uncertain_binary(
        &mut self,
        o: &Number,
        dfdx: impl FnOnce(&Number, &Number) -> Option<Number>,
        dfdy: impl FnOnce(&Number, &Number) -> Option<Number>,
        op: impl FnOnce(&mut Number, &Number) -> bool,
    ) -> bool {
        let ux = self.take_variance_uncertainty();
        let mut y = o.clone();
        let uy = y.take_variance_uncertainty();
        let x = self.clone();
        if !op(self, &y) {
            self.unc = ux.map(Box::new);
            return false;
        }
        self.unc = None;
        if let Some(u) = &ux {
            if let Some(d) = dfdx(&x, &y) {
                self.propagate_uncertainty(&d, u);
            }
        }
        if let Some(u) = &uy {
            if let Some(d) = dfdy(&x, &y) {
                self.propagate_uncertainty(&d, u);
            }
        }
        true
    }

    /// True when either operand carries a variance-formula uncertainty.
    pub(crate) fn either_uncertain(&self, o: &Number) -> bool {
        self.unc.is_some() || o.unc.is_some()
    }
}

/// Helper: construct a `Number` from integer literal in expressions/tests.
impl From<i64> for Number {
    fn from(i: i64) -> Self {
        Number::from_i64(i)
    }
}

impl From<BigInt> for Number {
    fn from(z: BigInt) -> Self {
        Number::from_bigint(z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_ints() {
        let z = Number::new();
        assert!(z.is_zero() && z.is_integer() && z.is_rational() && z.is_real());
        let n = Number::from_ints(5, 2, 0);
        assert!(n.is_fraction() == false && n.is_rational());
        let f = Number::from_ints(1, 2, 0);
        assert!(f.is_fraction());
        let e = Number::from_ints(25, 1, -1); // 2.5
        assert!(e.is_rational() && !e.is_integer());
        let g = Number::from_ints(25, 1, 2); // 2500
        assert!(g.is_integer());
    }

    #[test]
    fn infinity_flags() {
        let mut n = Number::new();
        n.set_plus_infinity(false, false);
        assert!(n.is_plus_infinity() && n.is_infinite(true) && !n.is_real());
    }

    #[test]
    fn float_demotion() {
        // 5.0 as float must demote to exact integer 5 via testInteger.
        let mut n = Number::new();
        n.set_float(5.0);
        assert!(n.is_integer(), "float 5.0 demotes to integer: {n:?}");
    }

    #[test]
    fn complex_parts() {
        let mut n = Number::from_i64(3);
        n.set_imaginary_part(&Number::from_i64(4));
        assert!(n.is_complex() && n.has_real_part() && n.has_imaginary_part());
        assert!(n.real_part().is_integer());
        assert!(n.imaginary_part().is_integer());
        let mut i = Number::new();
        i.set_imaginary_part(&Number::from_i64(1));
        assert!(i.is_i() && !i.has_real_part());
    }

    #[test]
    fn pi_is_interval() {
        let mut n = Number::new();
        n.pi();
        assert!(n.is_floating_point() && n.is_approximate());
        assert!(n.is_interval(true), "pi should be a proper interval in interval mode");
        assert!(n.is_positive());
    }
}
