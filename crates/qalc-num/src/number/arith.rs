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
        // Complex multiplication: (a+bi)(c+di) = (ac−bd) + (ad+bc)i.
        // Only when the *other* operand is complex too — a complex value
        // times a real one is scaled part by part below, because the cross
        // terms would otherwise multiply an absent imaginary part in as an
        // exact zero and turn `([1:+infinity]+(-infinity)i) × 1` into `0 ×
        // infinity`. The reference splits the same way (Number.cc:3331).
        if o.has_imaginary_part() {
            if o.has_real_part() {
                if self.has_imaginary_part() && self.has_real_part() {
                    // (a+bi)(c+di) = (ac−bd) + (ad+bc)i
                    let a = self.real_part();
                    let b = self.imaginary_part();
                    let c = o.real_part();
                    let d = o.imaginary_part();
                    let mut ac = a.clone();
                    let mut bd = b.clone();
                    let mut ad = a;
                    let mut bc = b;
                    if !ac.multiply(&c) || !bd.multiply(&d) || !ad.multiply(&d) || !bc.multiply(&c)
                    {
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
                // One of the two parts of `self` is absent, so the product is
                // `(self·i)·d + self·c` with the missing cross terms never
                // formed at all — a `0 × infinity` the general formula would
                // have to evaluate and refuse (Number.cc:3352).
                let mut copy = Number::new();
                if self.has_imaginary_part() {
                    copy = self.imaginary_part();
                    copy.negate();
                } else if self.has_real_part() {
                    copy.set_imaginary_part(&self.clone());
                }
                let bak = self.clone();
                if !copy.multiply(&o.imaginary_part()) || !self.multiply(&o.real_part()) {
                    *self = bak;
                    return false;
                }
                if !self.add(&copy) {
                    *self = bak;
                    return false;
                }
                return true;
            }
            // `o` is purely imaginary: multiplying by it is a quarter turn.
            if self.has_imaginary_part() {
                let mut copy = self.imaginary_part();
                copy.negate();
                if self.has_real_part() {
                    copy.set_imaginary_part(&self.real_part());
                }
                if !copy.multiply(&o.imaginary_part()) {
                    return false;
                }
                *self = copy;
                return true;
            }
            // A real value times a pure imaginary one: scale, then move the
            // whole thing into the imaginary slot. `infinity × (-i/pi)` is
            // `(-infinity)i`, where pairing the parts up would ask for
            // `0 × infinity` first.
            if !self.multiply(&o.imaginary_part()) {
                return false;
            }
            let moved = self.clone();
            let keep = (self.approx, self.precision);
            *self = Number::new();
            self.approx = keep.0;
            self.precision = keep.1;
            self.set_imaginary_part(&moved);
            return true;
        }
        // `0 × ∞` is undefined, and so is `[-0.5:0.5] × [1:+infinity]`: an
        // interval that is not *known* non-zero against one that reaches
        // infinity spans the whole line (Number.cc:3379).
        //
        // The test is on the *whole* value, imaginary part included, and it is
        // made exactly once. Re-asking it of the real part alone — which is
        // what scaling the two components through `multiply` would do —
        // rejects `([-0.5:0.5]+i) × [-infinity:-1]`, whose imaginary part is
        // what makes the operand non-zero.
        if o.includes_infinity() && !self.is_nonzero() {
            return false;
        }
        if self.includes_infinity() && !o.is_nonzero() {
            return false;
        }
        // From here `o` is real, and the imaginary part is scaled by a
        // recursive multiply exactly as the reference does it.
        let scaled_imag = if self.has_imaginary_part() {
            let mut im = self.imaginary_part();
            if !im.multiply(o) {
                return false;
            }
            Some(im)
        } else {
            None
        };
        let ok = self.multiply_real_unguarded(o);
        if ok {
            if let Some(im) = scaled_imag {
                if im.is_zero() {
                    self.imag = None;
                } else {
                    self.set_imaginary_part(&im);
                }
            }
        }
        ok
    }

    /// The real part of a product, once the zero-times-infinity question has
    /// been settled for the value as a whole. `o` is real.
    fn multiply_real_unguarded(&mut self, o: &Number) -> bool {
        // An infinite real part keeps its magnitude and takes `o`'s sign
        // (Number.cc:3384).
        if matches!(self.value, RealValue::PlusInfinity | RealValue::MinusInfinity) {
            if o.real_part_is_negative() {
                self.value = if matches!(self.value, RealValue::PlusInfinity) {
                    RealValue::MinusInfinity
                } else {
                    RealValue::PlusInfinity
                };
            }
            self.set_precision_and_approximate_from(o);
            return true;
        }
        if matches!(o.value, RealValue::PlusInfinity | RealValue::MinusInfinity) {
            // A real part that is present but may be zero has no sign to give
            // the infinity. A real part that is *absent* is simply left at
            // zero, which is what lets `0.5i × infinity` be `(+infinity)i`.
            if self.has_real_part() {
                if !self.real_part().is_nonzero() {
                    return false;
                }
                let neg = self.real_part_is_negative();
                let o_plus = matches!(o.value, RealValue::PlusInfinity);
                self.value = if neg == o_plus {
                    RealValue::MinusInfinity
                } else {
                    RealValue::PlusInfinity
                };
            }
            self.set_precision_and_approximate_from(o);
            return true;
        }
        if !self.has_real_part() {
            self.set_precision_and_approximate_from(o);
            return true;
        }
        if o.is_zero() {
            self.value = RealValue::Rational(BigRational::zero());
            self.set_precision_and_approximate_from(o);
            return true;
        }
        match (&self.value, &o.value) {
            (RealValue::Rational(a), RealValue::Rational(b)) => {
                self.value = RealValue::Rational(a * b);
                self.set_precision_and_approximate_from(o);
                true
            }
            // A float against an exact rational is `mpfr_mul_q`: the rational
            // is a single exact factor, not an interval of its own. Widening
            // it to `[1/3↓ : 1/3↑]` first is what left `1.5/3` as `[0.5:0.5]`
            // instead of the exact `0.5`.
            (RealValue::Float { lower, upper }, RealValue::Rational(r))
            | (RealValue::Rational(r), RealValue::Float { lower, upper }) => {
                let p = context::bit_precision();
                let (lower, upper) = if context::create_interval() {
                    if r.is_negative() {
                        (
                            mul_bf_rat(upper, r, p, rnd(true)),
                            mul_bf_rat(lower, r, p, rnd(false)),
                        )
                    } else {
                        (
                            mul_bf_rat(lower, r, p, rnd(true)),
                            mul_bf_rat(upper, r, p, rnd(false)),
                        )
                    }
                } else {
                    let f = mul_bf_rat(lower, r, p, RoundingMode::ToEven);
                    (f.clone(), f)
                };
                self.value = RealValue::Float { lower, upper };
                self.approx = true;
                self.set_precision_and_approximate_from(o);
                self.test_float_result(true)
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
                // The reference computes an infinite dividend as `x·(1/o)`
                // (Number.cc:3595) and its `multiply` refuses an infinity
                // against an operand that is not known non-zero
                // (Number.cc:3380) — which `1/o` is exactly when `o` reaches
                // infinity. `infinity / [1:+infinity]` is indeterminate, not
                // infinity.
                let mut oinv = o.clone();
                if !oinv.recip() {
                    return false;
                }
                self.multiply(&oinv)
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
            // A value that is not *known* non-zero has no reciprocal: the
            // reference's first line (Number.cc:3653), and for a complex
            // value it is the whole value that has to be non-zero, not each
            // part on its own.
            if !self.is_nonzero() {
                return false;
            }
            if !self.has_real_part() {
                // 1/(bi) = −(1/b)i.
                let mut im = self.imaginary_part();
                if !im.recip() || !im.negate() {
                    return false;
                }
                let keep = (self.approx, self.precision);
                *self = Number::new();
                self.approx = keep.0;
                self.precision = keep.1;
                self.set_imaginary_part(&im);
                return true;
            }
            // An interval operand goes through the dedicated interval
            // reciprocal below; a point one keeps the exact algebraic form.
            if self.is_interval(true)
                || self.imag.as_ref().is_some_and(|i| i.is_interval(true))
            {
                return self.recip_interval_complex();
            }
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

    /// The reference's dedicated interval reciprocal (Number.cc:3662).
    ///
    /// `1/(x+yi) = (x−yi)/(x²+y²)`, but evaluating that expression in interval
    /// arithmetic mentions `x` and `y` twice each and throws the dependency
    /// away — and squaring an unbounded part turns a perfectly finite
    /// reciprocal into a failure. Instead each component is optimised over the
    /// box directly: both are the same function `t ↦ t/(t²+s²)`, whose extrema
    /// over `|s| ∈ [smin, smax]` sit at `t = ±smin` (value `±1/(2·smin)`) and
    /// at the ends of `t`'s own range. The imaginary half is that function of
    /// `−y` with the two components' roles exchanged.
    fn recip_interval_complex(&mut self) -> bool {
        let p = context::bit_precision();
        let (xl, xu) = self.float_bounds(p);
        let (yl, yu) = self.imaginary_part().float_bounds(p);

        // min/max |·| over each component's range.
        let (abs_rl, abs_ru) = abs_range(&xl, &xu, p);
        let (abs_il, abs_iu) = abs_range(&yl, &yu, p);

        let Some((rl, ru)) = recip_component(&xl, &xu, &abs_rl, &abs_ru, &abs_il, &abs_iu, p)
        else {
            return false;
        };
        let Some((il, iu)) = recip_component(
            &yu.neg(),
            &yl.neg(),
            &abs_il,
            &abs_iu,
            &abs_rl,
            &abs_ru,
            p,
        ) else {
            return false;
        };

        let bak = self.clone();
        let mut im = Number::new();
        im.value = RealValue::Float { lower: il, upper: iu };
        im.approx = true;
        im.is_imag_part = true;
        self.value = RealValue::Float { lower: rl, upper: ru };
        self.approx = true;
        if !im.test_float_result(true) || !self.test_float_result(true) {
            *self = bak;
            return false;
        }
        self.imag = Some(Box::new(im));
        true
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
        if self.is_infinite(false) {
            self.value = RealValue::PlusInfinity;
            return true;
        }
        if self.has_imaginary_part() {
            if !self.has_real_part() {
                // (bi)^2 = -b^2, with `b` squared as one variable.
                let mut b = self.imaginary_part();
                if !b.square() || !b.negate() {
                    return false;
                }
                *self = b;
                return true;
            }
            if self.is_interval(true)
                || self.imag.as_ref().is_some_and(|i| i.is_interval(true))
            {
                return self.square_interval_complex();
            }
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

    /// `(x+yi)^2 = (x^2 - y^2) + 2xy i` over a box (Number.cc:4692).
    ///
    /// `multiply(self.clone())` would treat the two factors as independent and
    /// compute `y*y` as an interval product, which for a `y` straddling zero
    /// reaches below zero and widens the real part on both sides. Each part is
    /// squared here as the one variable it is.
    fn square_interval_complex(&mut self) -> bool {
        let p = context::bit_precision();
        let (xl, xu) = self.float_bounds(p);
        let (yl, yu) = self.imaginary_part().float_bounds(p);
        let (x2l, x2u) = sq_range(&xl, &xu, p);
        let (y2l, y2u) = sq_range(&yl, &yu, p);
        let re_l = x2l.sub(&y2u, p, rnd(true));
        let re_u = x2u.sub(&y2l, p, rnd(false));
        // 2xy, with the reference's flag semantics: a `0 * infinity` corner is
        // a NaN and fails the whole square rather than being skipped over.
        for a in [&xl, &xu] {
            for b in [&yl, &yu] {
                if a.mul(b, p, RoundingMode::ToEven).is_nan() {
                    return false;
                }
            }
        }
        let (ml, mu) = interval_mul(&xl, &xu, &yl, &yu, p);
        let two = BigFloat::from_i8(2, p);
        let im_l = ml.mul(&two, p, rnd(true));
        let im_u = mu.mul(&two, p, rnd(false));

        let bak = self.clone();
        let mut im = Number::new();
        im.value = RealValue::Float { lower: im_l, upper: im_u };
        im.approx = true;
        im.is_imag_part = true;
        self.value = RealValue::Float { lower: re_l, upper: re_u };
        self.approx = true;
        if !im.test_float_result(true) || !self.test_float_result(true) {
            *self = bak;
            return false;
        }
        self.imag = Some(Box::new(im));
        true
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

/// `mpfr_sgn`: 0 for either zero, ±1 otherwise (an infinity has a sign).
pub(super) fn bf_sgn(x: &BigFloat) -> i32 {
    if x.is_zero() {
        0
    } else if matches!(x.sign(), Some(Sign::Neg)) {
        -1
    } else {
        1
    }
}

fn bf_abs(x: &BigFloat) -> BigFloat {
    if matches!(x.sign(), Some(Sign::Neg)) {
        x.neg()
    } else {
        x.clone()
    }
}

/// `mpfr_cmpabs`: compares magnitudes. astro-float's own `abs_cmp` compares
/// signed values for two finite operands, so it cannot be used for this.
pub(super) fn bf_abs_cmp(a: &BigFloat, b: &BigFloat) -> i32 {
    bf_cmp(&bf_abs(a), &bf_abs(b))
}

/// `mpfr_cmp`, with `−0 == 0` — astro-float orders the two zeroes by sign.
pub(super) fn bf_cmp(a: &BigFloat, b: &BigFloat) -> i32 {
    if a.is_zero() && b.is_zero() {
        return 0;
    }
    match a.cmp(b) {
        Some(c) if c > 0 => 1,
        Some(c) if c < 0 => -1,
        _ => 0,
    }
}

/// min and max of `|t|` over `t ∈ [l, u]`. The minimum is zero exactly when
/// the interval straddles (or touches) zero.
fn abs_range(l: &BigFloat, u: &BigFloat, p: usize) -> (BigFloat, BigFloat) {
    let (al, au) = (bf_abs(l), bf_abs(u));
    let lo = if bf_sgn(l) != bf_sgn(u) {
        BigFloat::from_i8(0, p)
    } else if bf_cmp(&au, &al) < 0 {
        au.clone()
    } else {
        al.clone()
    };
    let hi = if bf_cmp(&al, &au) > 0 { al } else { au };
    (lo, hi)
}

/// One component of the interval reciprocal: the range of `t/(t²+s²)` for
/// `t ∈ [l, u]` and `s` the other component, whose `|s|` runs over
/// `[a_il, a_iu]`. `a_rl`/`a_ru` are the same bounds for `|t|`.
///
/// Statement for statement, the body of `Number::recip`'s `for(i = 0; i < 2;
/// i++)` loop (Number.cc:3697). The interval is first reflected into `u ≥ 0`,
/// because `t/(t²+s²)` is odd in `t`.
///
/// `None` where the reference gives up. A quotient of two infinities (or of
/// two zeroes) is a NaN, and the reference notices it not by inspecting the
/// result — the NaN lands in a temporary that is then thrown away — but
/// through MPFR's sticky NaN and range flags, which its `testFloatResult`
/// tests before anything else (Number.cc:2387). So a NaN *anywhere* in this
/// computation fails the whole reciprocal, even when both endpoints came out
/// finite: `1/([1:+infinity]+i)` is one of those.
#[allow(clippy::too_many_arguments)]
fn recip_component(
    l: &BigFloat,
    u: &BigFloat,
    a_rl: &BigFloat,
    a_ru: &BigFloat,
    a_il: &BigFloat,
    a_iu: &BigFloat,
    p: usize,
) -> Option<(BigFloat, BigFloat)> {
    // Every NaN in this routine comes from a division, so one checked
    // division is the whole of the reference's flag test.
    let mut nan = false;
    let mut div = |a: &BigFloat, b: &BigFloat, rm: RoundingMode| {
        let r = a.div(b, p, rm);
        if r.is_nan() {
            nan = true;
        }
        r
    };
    let two = BigFloat::from_i8(2, p);
    let (mut fl, mut fu) = (l.clone(), u.clone());
    let neg = bf_sgn(&fu) < 0;
    if neg {
        let (nl, nu) = (fu.neg(), fl.neg());
        fl = nl;
        fu = nu;
    }
    let absm_il = a_il.neg();
    let absm_iu = a_iu.neg();
    // |s|² at both rounding directions, reused below.
    let il2_d = a_il.mul(a_il, p, rnd(true));
    let il2_u = a_il.mul(a_il, p, rnd(false));
    let iu2_u = a_iu.mul(a_iu, p, rnd(false));
    // The maximum over `s` is always at |s| = a_il (smallest denominator);
    // `t \u21a6 t/(t\u00b2+a_il\u00b2)` then peaks at `t = a_il`.
    let ru = if bf_cmp(&fl, a_il) <= 0 {
        if bf_cmp(&fu, a_il) >= 0 {
            // the peak is inside the range: 1/(2\u00b7a_il)
            div(a_il, &il2_d.mul(&two, p, rnd(true)), rnd(false))
        } else {
            // still climbing at t = u
            div(&fu, &fu.mul(&fu, p, rnd(true)).add(&il2_d, p, rnd(true)), rnd(false))
        }
    } else {
        // past the peak already: largest at t = l
        div(&fl, &fl.mul(&fl, p, rnd(true)).add(&il2_d, p, rnd(true)), rnd(false))
    };
    let rl = if bf_sgn(&fl) < 0 {
        // the negative half mirrors the maximum
        if bf_cmp(&fl, &absm_il) <= 0 {
            div(&absm_il, &il2_u.mul(&two, p, rnd(false)), rnd(true))
        } else {
            div(&fl, &fl.mul(&fl, p, rnd(true)).add(&il2_d, p, rnd(true)), rnd(false))
        }
    } else if bf_cmp(&fl, &absm_iu) <= 0 {
        if bf_cmp(a_ru, &absm_iu) >= 0 {
            div(&absm_iu, &iu2_u.mul(&two, p, rnd(false)), rnd(true))
        } else {
            div(a_ru, &a_ru.mul(a_ru, p, rnd(false)).add(&iu2_u, p, rnd(false)), rnd(true))
        }
    } else if bf_cmp(&fl, a_iu) > 0 {
        // t is past the peak for every s: smallest at t = a_ru, |s| = a_iu
        div(a_ru, &a_ru.mul(a_ru, p, rnd(false)).add(&iu2_u, p, rnd(false)), rnd(true))
    } else {
        let mut v = div(
            &fl,
            &a_rl.mul(a_rl, p, rnd(false)).add(&iu2_u, p, rnd(false)),
            rnd(true),
        );
        if bf_cmp(a_ru, a_iu) > 0 {
            // the range crosses the peak, so the far end competes with it
            let c = div(
                a_ru,
                &a_ru.mul(a_ru, p, rnd(false)).add(&iu2_u, p, rnd(false)),
                rnd(true),
            );
            if bf_cmp(&c, &v) < 0 {
                v = c;
            }
        }
        v
    };
    if nan {
        return None;
    }
    Some(if neg {
        (ru.neg(), rl.neg())
    } else {
        (rl, ru)
    })
}

/// min and max of `t^2` over `t` in `[l, u]`, rounded outwards.
fn sq_range(l: &BigFloat, u: &BigFloat, p: usize) -> (BigFloat, BigFloat) {
    let bigger_l = bf_abs_cmp(l, u) > 0;
    let hi = if bigger_l {
        l.mul(l, p, rnd(false))
    } else {
        u.mul(u, p, rnd(false))
    };
    if bf_sgn(l) < 0 && bf_sgn(u) > 0 {
        return (BigFloat::from_i8(0, p), hi);
    }
    let lo = if bigger_l {
        u.mul(u, p, rnd(true))
    } else {
        l.mul(l, p, rnd(true))
    };
    (lo, hi)
}

/// `mpfr_mul_q`: a float times an exact rational, correctly rounded once.
/// The product with the numerator is computed at enough precision to be
/// exact, so only the division by the denominator rounds.
fn mul_bf_rat(f: &BigFloat, r: &BigRational, p: usize, rm: RoundingMode) -> BigFloat {
    let num = crate::float::bigfloat_from_bigint_exact(r.numer());
    let den = crate::float::bigfloat_from_bigint_exact(r.denom());
    let wide = p + r.numer().bits() as usize + 8;
    let prod = f.mul(&num, wide, RoundingMode::None);
    prod.div(&den, p, rm)
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

