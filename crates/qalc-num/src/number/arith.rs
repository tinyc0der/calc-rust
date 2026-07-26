//! Arithmetic on `Number` — port of the corresponding parts of `Number.cc`.
//!
//! Exact (rational × rational) paths stay exact; anything touching a float
//! goes through interval arithmetic with directed rounding (lower bound
//! rounded down, upper bound rounded up), or round-to-nearest point floats
//! when interval arithmetic is disabled.

use super::{Number, RealValue};
use crate::context;
use astro_float::{BigFloat, RoundingMode, Sign};
use num_rational::BigRational;
use num_traits::{Signed, Zero};

/// Compute one bound of an interval operation.
fn rnd(down: bool) -> RoundingMode {
    if down { RoundingMode::Down } else { RoundingMode::Up }
}

impl Number {
    /// Both operands as float bounds at working precision.
    fn float_bounds(&self, p: usize) -> (BigFloat, BigFloat) {
        (self.lower_bound_float(p), self.upper_bound_float(p))
    }

    // ------------------------------------------------------------------
    // Addition / subtraction
    // ------------------------------------------------------------------

    /// `add(o)`: self += o. Returns false when undefined (∞ + −∞).
    pub fn add(&mut self, o: &Number) -> bool {
        // Both partial derivatives of a sum are 1, so the operands'
        // uncertainties simply combine in quadrature.
        if self.either_uncertain(o) {
            let one = Number::from_i64(1);
            let (a, b) = (one.clone(), one);
            return self.uncertain_binary(
                o,
                move |_, _| Some(a),
                move |_, _| Some(b),
                Number::add_impl,
            );
        }
        self.add_impl(o)
    }

    fn add_impl(&mut self, o: &Number) -> bool {
        // Complex handling: add parts independently.
        if o.has_imaginary_part() || self.has_imaginary_part() {
            let mut re = self.real_part();
            let mut im = self.imaginary_part();
            if !re.add(&o.real_part()) || !im.add(&o.imaginary_part()) {
                return false;
            }
            *self = re;
            if !im.is_zero() {
                self.set_imaginary_part(&im);
            }
            return true;
        }
        // `∞ + (−∞)` is undefined however the two infinities arrive: as
        // infinities themselves, or as the open end of a half-infinite
        // interval. `[1:+infinity] + (-infinity)` has no enclosure — the
        // reference refuses it (Number.cc:3129) and so must the interval
        // arithmetic below, which would otherwise hand `+inf + -inf` to
        // astro-float and get a NaN bound.
        if self.includes_minus_infinity() && o.includes_plus_infinity() {
            return false;
        }
        if self.includes_plus_infinity() && o.includes_minus_infinity() {
            return false;
        }
        match (&self.value, &o.value) {
            (RealValue::PlusInfinity, RealValue::MinusInfinity)
            | (RealValue::MinusInfinity, RealValue::PlusInfinity) => false,
            (RealValue::PlusInfinity, _) | (_, RealValue::MinusInfinity)
            | (RealValue::MinusInfinity, _) | (_, RealValue::PlusInfinity) => {
                if matches!(o.value, RealValue::PlusInfinity | RealValue::MinusInfinity) {
                    self.value = o.value.clone();
                }
                self.set_precision_and_approximate_from(o);
                true
            }
            (RealValue::Rational(a), RealValue::Rational(b)) => {
                self.value = RealValue::Rational(a + b);
                self.set_precision_and_approximate_from(o);
                true
            }
            _ => {
                let p = context::bit_precision();
                let (al, au) = self.float_bounds(p);
                let (bl, bu) = o.float_bounds(p);
                let (lower, upper) = if context::create_interval() {
                    (al.add(&bl, p, rnd(true)), au.add(&bu, p, rnd(false)))
                } else {
                    let f = al.add(&bl, p, RoundingMode::ToEven);
                    (f.clone(), f)
                };
                self.value = RealValue::Float { lower, upper };
                self.approx = true;
                self.set_precision_and_approximate_from(o);
                self.test_float_result(true)
            }
        }
    }

    pub fn add_i64(&mut self, i: i64) -> bool {
        self.add(&Number::from_i64(i))
    }

    /// `subtract(o)`: self -= o.
    pub fn subtract(&mut self, o: &Number) -> bool {
        let mut neg = o.clone();
        if !neg.negate() {
            return false;
        }
        self.add(&neg)
    }

    /// `negate()`: self = -self. Always succeeds.
    pub fn negate(&mut self) -> bool {
        match &mut self.value {
            RealValue::Rational(r) => {
                let v = std::mem::replace(r, BigRational::zero());
                *r = -v;
            }
            RealValue::Float { lower, upper } => {
                let nl = upper.neg();
                let nu = lower.neg();
                *lower = nl;
                *upper = nu;
            }
            RealValue::PlusInfinity => self.value = RealValue::MinusInfinity,
            RealValue::MinusInfinity => self.value = RealValue::PlusInfinity,
        }
        if let Some(im) = &mut self.imag {
            im.negate();
        }
        true
    }

    // ------------------------------------------------------------------
    // Multiplication
    // ------------------------------------------------------------------

    /// `multiply(o)`: self *= o.
    pub fn multiply(&mut self, o: &Number) -> bool {
        // d(xy)/dx = y, d(xy)/dy = x.
        if self.either_uncertain(o) {
            return self.uncertain_binary(
                o,
                |_, y| Some(y.clone()),
                |x, _| Some(x.clone()),
                Number::multiply_impl,
            );
        }
        self.multiply_impl(o)
    }

    fn multiply_impl(&mut self, o: &Number) -> bool {
        // Only one side is complex: scale both of its parts and stop. The
        // general product below would multiply the other side's absent
        // imaginary part in as an exact zero and add it back — numerically the
        // same thing, but `0 × [1:+infinity]` is undefined, so the cross terms
        // turn `([1:+infinity]+(-infinity)i) × 1` into a failure. The reference
        // scales the parts separately for exactly this shape (Number.cc:3433).
        if self.has_imaginary_part() != o.has_imaginary_part() {
            let (complex, real) = if self.has_imaginary_part() {
                (&*self, o)
            } else {
                (o, &*self)
            };
            let mut re = complex.real_part();
            let mut im = complex.imaginary_part();
            if !re.multiply(real) || !im.multiply(real) {
                return false;
            }
            *self = re;
            if !im.is_zero() {
                self.set_imaginary_part(&im);
            }
            return true;
        }
        // Complex multiplication: (a+bi)(c+di) = (ac−bd) + (ad+bc)i
        if o.has_imaginary_part() || self.has_imaginary_part() {
            let a = self.real_part();
            let b = self.imaginary_part();
            let c = o.real_part();
            let d = o.imaginary_part();
            let mut ac = a.clone();
            let mut bd = b.clone();
            let mut ad = a;
            let mut bc = b;
            if !ac.multiply(&c) || !bd.multiply(&d) || !ad.multiply(&d) || !bc.multiply(&c) {
                return false;
            }
            let mut re = ac;
            if !re.subtract(&bd) {
                return false;
            }
            let mut im = ad;
            if !im.add(&bc) {
                return false;
            }
            *self = re;
            if !im.is_zero() {
                self.set_imaginary_part(&im);
            }
            return true;
        }
        // `0 × ∞` is undefined, and so is `[-0.5:0.5] × [1:+infinity]`: an
        // interval that is not *known* non-zero against one that reaches
        // infinity spans the whole line. The reference tests exactly this
        // (Number.cc:3379), and it is the half-infinite interval that makes it
        // matter — the arms below only recognise an infinity that is the whole
        // value.
        if o.includes_infinity() && !self.is_nonzero() {
            return false;
        }
        if self.includes_infinity() && !o.is_nonzero() {
            return false;
        }
        match (&self.value, &o.value) {
            (RealValue::PlusInfinity | RealValue::MinusInfinity, _)
            | (_, RealValue::PlusInfinity | RealValue::MinusInfinity) => {
                // 0 × ∞ is undefined.
                if self.is_zero() || o.is_zero() {
                    return false;
                }
                if !self.is_nonzero() || !o.is_nonzero() {
                    return false; // interval containing zero times infinity
                }
                let self_neg = self.real_part_is_negative();
                let o_neg = o.real_part_is_negative();
                let plus = matches!(
                    (&self.value, &o.value),
                    (RealValue::PlusInfinity, _) | (_, RealValue::PlusInfinity)
                );
                // Determine resulting sign: sign(self) × sign(o).
                let result_plus = match (&self.value, &o.value) {
                    (RealValue::PlusInfinity, RealValue::PlusInfinity) => true,
                    (RealValue::MinusInfinity, RealValue::MinusInfinity) => true,
                    (RealValue::PlusInfinity, RealValue::MinusInfinity)
                    | (RealValue::MinusInfinity, RealValue::PlusInfinity) => false,
                    (RealValue::PlusInfinity, _) => !o_neg,
                    (RealValue::MinusInfinity, _) => o_neg,
                    (_, RealValue::PlusInfinity) => !self_neg,
                    (_, RealValue::MinusInfinity) => self_neg,
                    _ => plus,
                };
                self.value = if result_plus { RealValue::PlusInfinity } else { RealValue::MinusInfinity };
                self.set_precision_and_approximate_from(o);
                true
            }
            (RealValue::Rational(a), RealValue::Rational(b)) => {
                self.value = RealValue::Rational(a * b);
                self.set_precision_and_approximate_from(o);
                true
            }
            _ => {
                let p = context::bit_precision();
                let (al, au) = self.float_bounds(p);
                let (bl, bu) = o.float_bounds(p);
                let (lower, upper) = if context::create_interval() {
                    interval_mul(&al, &au, &bl, &bu, p)
                } else {
                    let f = al.mul(&bl, p, RoundingMode::ToEven);
                    (f.clone(), f)
                };
                self.value = RealValue::Float { lower, upper };
                self.approx = true;
                self.set_precision_and_approximate_from(o);
                self.test_float_result(true)
            }
        }
    }

    pub fn multiply_i64(&mut self, i: i64) -> bool {
        self.multiply(&Number::from_i64(i))
    }

    // ------------------------------------------------------------------
    // Division / reciprocal
    // ------------------------------------------------------------------

    /// `divide(o)`: self /= o. Fails on division by zero.
    pub fn divide(&mut self, o: &Number) -> bool {
        // d(x/y)/dx = 1/y, d(x/y)/dy = -x/y².
        if self.either_uncertain(o) {
            return self.uncertain_binary(
                o,
                |_, y| {
                    let mut d = y.clone();
                    d.recip().then_some(d)
                },
                |x, y| {
                    let mut d = y.clone();
                    (d.square() && d.recip() && d.multiply(x) && d.negate()).then_some(d)
                },
                Number::divide_impl,
            );
        }
        self.divide_impl(o)
    }

    fn divide_impl(&mut self, o: &Number) -> bool {
        if o.has_imaginary_part() || self.has_imaginary_part() {
            // z/w = z * conj(w) / |w|^2
            let mut recip = o.clone();
            if !recip.recip() {
                return false;
            }
            return self.multiply(&recip);
        }
        if o.is_zero() {
            return false;
        }
        if !o.is_nonzero() && o.is_floating_point() {
            return false; // interval containing zero
        }
        match (&self.value, &o.value) {
            (RealValue::PlusInfinity | RealValue::MinusInfinity,
             RealValue::PlusInfinity | RealValue::MinusInfinity) => false,
            (RealValue::PlusInfinity | RealValue::MinusInfinity, _) => {
                let flip = o.real_part_is_negative();
                if flip {
                    self.negate();
                }
                self.set_precision_and_approximate_from(o);
                true
            }
            (_, RealValue::PlusInfinity | RealValue::MinusInfinity) => {
                // finite / ∞ = 0 — but an *unbounded* numerator is not
                // finite. The reference reaches this case as `x · (1/∞)`,
                // i.e. `x · 0`, which its `multiply` refuses whenever `x`
                // includes an infinity (Number.cc:3380): `[-infinity:-1] /
                // infinity` is indeterminate, not zero.
                if self.includes_infinity() {
                    return false;
                }
                self.clear(true);
                self.set_precision_and_approximate_from(o);
                true
            }
            (RealValue::Rational(a), RealValue::Rational(b)) => {
                self.value = RealValue::Rational(a / b);
                self.set_precision_and_approximate_from(o);
                true
            }
            _ => {
                let mut recip = o.clone();
                if !recip.recip() {
                    return false;
                }
                self.multiply(&recip)
            }
        }
    }

    pub fn divide_i64(&mut self, i: i64) -> bool {
        self.divide(&Number::from_i64(i))
    }

    /// `recip()`: self = 1/self.
    pub fn recip(&mut self) -> bool {
        if self.has_imaginary_part() {
            // 1/(a+bi) = (a−bi)/(a²+b²)
            let a = self.real_part();
            let b = self.imaginary_part();
            let mut a2 = a.clone();
            let mut b2 = b.clone();
            if !a2.square() || !b2.square() {
                return false;
            }
            let mut den = a2;
            if !den.add(&b2) {
                return false;
            }
            if den.is_zero() {
                return false;
            }
            let mut re = a;
            let mut im = b;
            if !re.divide(&den) || !im.divide(&den) || !im.negate() {
                return false;
            }
            *self = re;
            if !im.is_zero() {
                self.set_imaginary_part(&im);
            }
            return true;
        }
        match &self.value {
            RealValue::Rational(r) => {
                if r.is_zero() {
                    return false;
                }
                self.value = RealValue::Rational(r.recip());
                true
            }
            RealValue::Float { lower, upper } => {
                if !self.is_nonzero() {
                    return false;
                }
                let p = context::bit_precision();
                let one = BigFloat::from_i8(1, p);
                let (lower, upper) = if context::create_interval() {
                    // 1/[l,u] = [1/u, 1/l] for intervals not containing 0.
                    (one.div(upper, p, rnd(true)), one.div(lower, p, rnd(false)))
                } else {
                    let f = one.div(lower, p, RoundingMode::ToEven);
                    (f.clone(), f)
                };
                self.value = RealValue::Float { lower, upper };
                self.approx = true;
                self.test_float_result(true)
            }
            RealValue::PlusInfinity | RealValue::MinusInfinity => {
                self.clear(true);
                true
            }
        }
    }

    /// `square()`: self = self².
    pub fn square(&mut self) -> bool {
        // d(x²)/dx = 2x, taken here rather than left to the `multiply` below.
        // `x·x` has one variable, but `uncertain_binary` assumes its two
        // operands are independent, so routing an uncertain value through it
        // would combine the same contribution with itself in quadrature and
        // return |2x|·u/√2 instead of |2x|·u.
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    d.multiply(&Number::from_i64(2)).then_some(d)
                },
                Number::square_impl,
            );
        }
        self.square_impl()
    }

    fn square_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            let o = self.clone();
            return self.multiply(&o);
        }
        match &self.value {
            RealValue::PlusInfinity | RealValue::MinusInfinity => {
                self.value = RealValue::PlusInfinity;
                true
            }
            RealValue::Rational(r) => {
                self.value = RealValue::Rational(r * r);
                true
            }
            RealValue::Float { lower, upper } => {
                let p = context::bit_precision();
                let (lower, upper) = if context::create_interval() {
                    let ln = lower.sign() == Some(Sign::Neg);
                    let un = upper.sign() == Some(Sign::Neg);
                    if !ln {
                        // both non-negative
                        (lower.mul(lower, p, rnd(true)), upper.mul(upper, p, rnd(false)))
                    } else if un {
                        // both negative: [u², l²]
                        (upper.mul(upper, p, rnd(true)), lower.mul(lower, p, rnd(false)))
                    } else {
                        // spans zero: [0, max(l², u²)]
                        let l2 = lower.mul(lower, p, rnd(false));
                        let u2 = upper.mul(upper, p, rnd(false));
                        let m = if matches!(l2.cmp(&u2), Some(c) if c > 0) { l2 } else { u2 };
                        (BigFloat::from_i8(0, p), m)
                    }
                } else {
                    let f = lower.mul(lower, p, RoundingMode::ToEven);
                    (f.clone(), f)
                };
                self.value = RealValue::Float { lower, upper };
                self.approx = true;
                self.test_float_result(true)
            }
        }
    }

    /// `abs()`.
    pub fn abs(&mut self) -> bool {
        if self.has_imaginary_part() {
            // |a+bi| = sqrt(a²+b²) — needs sqrt; deferred to the power module.
            let a = self.real_part();
            let b = self.imaginary_part();
            let mut a2 = a;
            let mut b2 = b;
            if !a2.square() || !b2.square() || !a2.add(&b2) {
                return false;
            }
            if !a2.sqrt() {
                return false;
            }
            *self = a2;
            return true;
        }
        match &mut self.value {
            RealValue::Rational(r) => {
                let v = std::mem::replace(r, BigRational::zero());
                *r = v.abs();
                true
            }
            RealValue::Float { .. } => {
                if self.real_part_is_negative() {
                    self.negate();
                } else if !self.real_part_is_positive() {
                    // interval spanning zero: [0, max(−l, u)]
                    let p = context::bit_precision();
                    if let RealValue::Float { lower, upper } = &self.value {
                        let nl = lower.neg();
                        let m = if matches!(nl.cmp(upper), Some(c) if c > 0) { nl } else { upper.clone() };
                        self.value = RealValue::Float { lower: BigFloat::from_i8(0, p), upper: m };
                    }
                }
                true
            }
            RealValue::PlusInfinity => true,
            RealValue::MinusInfinity => {
                self.value = RealValue::PlusInfinity;
                true
            }
        }
    }

    /// `signum()`: self = sign(self) ∈ {−1, 0, 1} (real only here).
    pub fn signum(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        if self.is_zero() {
            self.clear(true);
            return true;
        }
        if self.real_part_is_positive() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            true
        } else if self.real_part_is_negative() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(-1);
            self.approx = keep.0;
            self.precision = keep.1;
            true
        } else {
            false // interval spanning zero: sign unknown
        }
    }

    /// `exp10(o)`: self = self × 10^o (exact for integer o).
    pub fn exp10_mul(&mut self, o: &Number) -> bool {
        if o.is_integer() {
            if let (RealValue::Rational(r), Some(exp)) = (&self.value, o.to_i64()) {
                if exp.unsigned_abs() <= 1_000_000 {
                    let ten_pow = num_bigint::BigInt::from(10).pow(exp.unsigned_abs() as u32);
                    let f = BigRational::from_integer(ten_pow);
                    self.value = RealValue::Rational(if exp >= 0 { r * &f } else { r / &f });
                    self.set_precision_and_approximate_from(o);
                    return true;
                }
            }
        }
        let mut ten = Number::from_i64(10);
        if !ten.raise(o, true) {
            return false;
        }
        self.multiply(&ten)
    }

    /// `exp2(o)`: self = self × 2^o (exact for integer o).
    pub fn exp2_mul(&mut self, o: &Number) -> bool {
        if o.is_integer() {
            if let (RealValue::Rational(r), Some(exp)) = (&self.value, o.to_i64()) {
                if exp.unsigned_abs() <= 10_000_000 {
                    let two_pow = num_bigint::BigInt::from(2).pow(exp.unsigned_abs() as u32);
                    let f = BigRational::from_integer(two_pow);
                    self.value = RealValue::Rational(if exp >= 0 { r * &f } else { r / &f });
                    self.set_precision_and_approximate_from(o);
                    return true;
                }
            }
        }
        let mut two = Number::from_i64(2);
        if !two.raise(o, true) {
            return false;
        }
        self.multiply(&two)
    }
}

/// Interval multiplication: all four corner products with directed rounding.
fn interval_mul(
    al: &BigFloat,
    au: &BigFloat,
    bl: &BigFloat,
    bu: &BigFloat,
    p: usize,
) -> (BigFloat, BigFloat) {
    let candidates_lo = [
        al.mul(bl, p, RoundingMode::Down),
        al.mul(bu, p, RoundingMode::Down),
        au.mul(bl, p, RoundingMode::Down),
        au.mul(bu, p, RoundingMode::Down),
    ];
    let candidates_hi = [
        al.mul(bl, p, RoundingMode::Up),
        al.mul(bu, p, RoundingMode::Up),
        au.mul(bl, p, RoundingMode::Up),
        au.mul(bu, p, RoundingMode::Up),
    ];
    let mut lo = candidates_lo[0].clone();
    for c in &candidates_lo[1..] {
        if matches!(c.cmp(&lo), Some(c) if c < 0) || lo.is_nan() {
            lo = c.clone();
        }
    }
    let mut hi = candidates_hi[0].clone();
    for c in &candidates_hi[1..] {
        if matches!(c.cmp(&hi), Some(c) if c > 0) || hi.is_nan() {
            hi = c.clone();
        }
    }
    (lo, hi)
}

#[cfg(test)]
mod uncertainty_tests {
    use crate::number::uncertainty_test_support::{plus_minus, uncertain};

    #[test]
    fn cancelling_subtraction_still_adds_in_quadrature() {
        // Reference: `(1+/-0.1)-(1+/-0.1)` = `0.00±0.14`. Both operands are
        // independent, so the uncertainties combine as sqrt(0.1²+0.1²) even
        // though the values cancel — and a zero midpoint has to take its
        // digits from the uncertainty, which is what used to print `0±0.1`.
        let mut a = uncertain("1", "0.1");
        let b = uncertain("1", "0.1");
        assert!(a.subtract(&b));
        assert_eq!(plus_minus(&a), "0.00±0.14");
    }

    #[test]
    fn squaring_is_one_variable_not_two() {
        // `x²` is not `x·y`: the reference's `2x` derivative gives
        // `square(4+/-0.1)` = `16.00±0.80`, where multiplying a clone in
        // would treat the two factors as independent and return 0.57.
        let mut n = uncertain("4", "0.1");
        assert!(n.square());
        assert_eq!(plus_minus(&n), "16.00±0.80");
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn interval_multiplication_keeps_both_bounds() {
        // [4.997, 5.003]² = [24.970009, 25.030009] — the corner search has to
        // compare bound *signs*, not `cmp(..) == Some(1)`; astro-float's
        // `cmp` returns a signed magnitude, so the old test never fired and
        // the product collapsed onto its lower corner.
        let po = crate::options::ParseOptions::default();
        let mut n = Number::new();
        assert!(n.set_interval(
            &Number::parse("4.997", &po),
            &Number::parse("5.003", &po),
            false
        ));
        let m = n.clone();
        assert!(n.multiply(&m));
        assert!(n.lower_end_point().is_less_than(&Number::parse("24.9701", &po)));
        assert!(n.upper_end_point().is_greater_than(&Number::parse("25.03", &po)));
    }

    #[test]
    fn variance_uncertainty_moves_with_multiplication_by_i() {
        // `(5+/-0.003)i` puts the uncertainty on the imaginary component:
        // d(iz)/dz = i, so the real and imaginary uncertainties swap.
        let po = crate::options::ParseOptions::default();
        let mut n = Number::from_i64(5);
        n.add_variance_uncertainty(&Number::parse("0.003", &po));
        let mut i = Number::new();
        i.set_imaginary_part(&Number::from_i64(1));
        assert!(n.multiply(&i));
        let u = n.variance_uncertainty().expect("uncertainty survives");
        assert!(u.real_part().is_zero(), "real component: {u:?}");
        assert!(!u.imaginary_part().is_zero(), "imaginary component: {u:?}");
    }

    #[test]
    fn independent_uncertainties_add_in_quadrature() {
        // d(x+y)/dx = d(x+y)/dy = 1, so 0.3 ⊕ 0.4 = 0.5.
        let po = crate::options::ParseOptions::default();
        let mut a = Number::from_i64(1);
        a.add_variance_uncertainty(&Number::parse("0.3", &po));
        let mut b = Number::from_i64(2);
        b.add_variance_uncertainty(&Number::parse("0.4", &po));
        assert!(a.add(&b));
        let u = a.variance_uncertainty().expect("uncertainty survives");
        assert!(u.is_greater_than(&Number::parse("0.4999", &po)), "{u:?}");
        assert!(u.is_less_than(&Number::parse("0.5001", &po)), "{u:?}");
    }
    use super::*;

    #[test]
    fn exact_rational_arithmetic() {
        let mut n = Number::from_ints(1, 3, 0);
        assert!(n.add(&Number::from_ints(1, 6, 0)));
        // 1/3 + 1/6 = 1/2 exactly
        assert!(n.is_rational() && !n.is_approximate());
        assert!(n.internal_rational().unwrap() == &BigRational::new(1.into(), 2.into()));
    }

    #[test]
    fn division_exact() {
        let mut n = Number::from_i64(1);
        assert!(n.divide(&Number::from_i64(3)));
        assert!(n.is_rational(), "1/3 stays exact rational");
        assert!(!n.divide(&Number::new()), "division by zero fails");
    }

    #[test]
    fn interval_add_contains_true_value() {
        let mut pi = Number::new();
        pi.pi();
        let mut n = pi.clone();
        assert!(n.add(&pi));
        // 2π ∈ [lower, upper] and interval is proper
        assert!(n.is_interval(true));
        assert!(n.is_positive());
    }

    #[test]
    fn complex_mul() {
        // (3+4i)(3−4i) = 25
        let mut a = Number::from_i64(3);
        a.set_imaginary_part(&Number::from_i64(4));
        let mut b = Number::from_i64(3);
        b.set_imaginary_part(&Number::from_i64(-4));
        assert!(a.multiply(&b));
        assert!(a.is_integer() && !a.is_complex());
        assert!(a.internal_rational().unwrap() == &BigRational::from_integer(25.into()));
    }

    #[test]
    fn infinity_rules() {
        let mut inf = Number::new();
        inf.set_plus_infinity(false, false);
        let mut minf = Number::new();
        minf.set_minus_infinity(false, false);
        let mut x = inf.clone();
        assert!(!x.add(&minf), "∞ + −∞ undefined");
        let mut y = inf.clone();
        assert!(y.multiply(&Number::from_i64(-2)));
        assert!(y.is_minus_infinity());
        let mut z = Number::from_i64(0);
        assert!(!z.multiply(&inf), "0 × ∞ undefined");
        let mut w = Number::from_i64(5);
        assert!(w.divide(&inf) && w.is_zero(), "5/∞ = 0");
    }

    #[test]
    fn negate_interval_flips_bounds() {
        let mut pi = Number::new();
        pi.pi();
        let lo = pi.lower_end_point();
        pi.negate();
        assert!(pi.is_negative());
        let hi = pi.upper_end_point();
        let mut sum = lo;
        assert!(sum.add(&hi));
        // −upper ≤ −lower ⇒ lower + (−lower) ... just sanity: result near zero
        assert!(sum.is_interval(true) || sum.is_zero());
    }

    #[test]
    fn abs_and_signum() {
        let mut n = Number::from_i64(-7);
        assert!(n.abs());
        assert!(n.internal_rational().unwrap() == &BigRational::from_integer(7.into()));
        let mut s = Number::from_ints(-3, 2, 0);
        assert!(s.signum());
        assert!(s.internal_rational().unwrap() == &BigRational::from_integer((-1).into()));
    }

    #[test]
    fn complex_abs() {
        let mut a = Number::from_i64(3);
        a.set_imaginary_part(&Number::from_i64(4));
        assert!(a.abs());
        assert!(a.is_integer(), "|3+4i| = 5 exactly, got {a:?}");
        assert!(a.internal_rational().unwrap() == &BigRational::from_integer(5.into()));
    }

    #[test]
    fn exp10_exact() {
        let mut n = Number::from_i64(3);
        assert!(n.exp10_mul(&Number::from_i64(4)));
        assert!(n.internal_rational().unwrap() == &BigRational::from_integer(30000.into()));
    }
}

