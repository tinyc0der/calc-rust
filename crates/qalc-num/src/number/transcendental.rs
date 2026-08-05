//! Transcendental functions — port of the corresponding `Number.cc` methods
//! backed by astro-float (replacing mpfr_exp/log/trig...).
//!
//! Interval propagation: monotone functions map [l,u] → [f(l)↓, f(u)↑];
//! sin/cos locate contained extrema. Point mode uses round-to-nearest.
//! astro-float has no counterpart to MPFR's special functions, so gamma,
//! digamma, zeta, erf/erfc/erfi, Bernoulli numbers and the exponential,
//! logarithmic, sine and cosine integrals live next door in
//! [`super::special`] as hand-rolled precision-scaled series; `builtins.rs`
//! in qalc-core dispatches to them.
//!
//! TODO(port): the Bessel functions (`mpfr_jn`/`mpfr_yn`), Airy and the
//! polylogarithm (`mpfr_li2`) are still stubs — see the bottom of
//! `special.rs`.

use super::arith::{bf_abs_cmp, bf_sgn};
use super::{Number, RealValue};
use crate::context;
use astro_float::{BigFloat, Consts, RoundingMode};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

type MonoFn = fn(&BigFloat, usize, RoundingMode, &mut Consts) -> BigFloat;

/// `1/sqrt(1 − x²)` — the shared factor of the `asin`/`acos` derivatives.
fn one_over_sqrt_one_minus_square(x: &Number) -> Option<Number> {
    let mut d = x.clone();
    (d.square() && d.negate() && d.add(&Number::from_i64(1)) && d.sqrt() && d.recip())
        .then_some(d)
}

/// `mpfr_atan2(y, x)` at a point, with every limit MPFR defines at a zero or
/// an infinity. `atan(y/x)` plus a quadrant correction cannot stand in for it:
/// `infinity/infinity` is a NaN where `atan2(+infinity, +infinity)` is pi/4.
///
/// `rm` rounds the *result*, which is not the same as rounding every step that
/// way — a negated or subtracted term has to be rounded the other way for the
/// result to come out on the requested side.
fn atan2_bf(y: &BigFloat, x: &BigFloat, p: usize, rm: RoundingMode) -> BigFloat {
    if y.is_nan() || x.is_nan() {
        return BigFloat::nan(None);
    }
    // IEEE-754 `atan2` reads the sign of a zero, so does this: `atan2(0, -0)`
    // is pi, not 0.
    let x_neg = matches!(x.sign(), Some(astro_float::Sign::Neg));
    let y_neg = matches!(y.sign(), Some(astro_float::Sign::Neg));
    let opp = match rm {
        RoundingMode::Down => RoundingMode::Up,
        RoundingMode::Up => RoundingMode::Down,
        other => other,
    };
    // The magnitude of every closed-form case below is negated when y < 0, so
    // it has to be rounded the other way round.
    let mag = if y_neg { opp } else { rm };
    let signed = |v: BigFloat| if y_neg { v.neg() } else { v };
    let n = |i: i8| BigFloat::from_i8(i, p);
    if y.is_zero() {
        return if x_neg {
            signed(context::with_consts(|cc| cc.pi(p, mag)))
        } else {
            y.clone()
        };
    }
    if x.is_zero() {
        return signed(context::with_consts(|cc| cc.pi(p, mag)).div(&n(2), p, mag));
    }
    if y.is_inf() {
        if x.is_inf() {
            // The diagonals: ±pi/4 and ±3pi/4.
            let pi = context::with_consts(|cc| cc.pi(p, mag));
            return signed(if x_neg {
                pi.mul(&n(3), p, mag).div(&n(4), p, mag)
            } else {
                pi.div(&n(4), p, mag)
            });
        }
        return signed(context::with_consts(|cc| cc.pi(p, mag)).div(&n(2), p, mag));
    }
    if x.is_inf() {
        return if x_neg {
            signed(context::with_consts(|cc| cc.pi(p, mag)))
        } else {
            signed(n(0))
        };
    }
    // `atan` is increasing and `y/x` moves with the result, so both take `rm`;
    // only the pi that is *subtracted* in the third quadrant flips.
    let a = context::with_consts(|cc| y.div(x, p, rm).atan(p, rm, cc));
    if !x_neg {
        a
    } else if y_neg {
        a.sub(&context::with_consts(|cc| cc.pi(p, opp)), p, rm)
    } else {
        a.add(&context::with_consts(|cc| cc.pi(p, rm)), p, rm)
    }
}

fn canonical_base_exp(x: &num_bigint::BigUint) -> (num_bigint::BigUint, u32) {
    use num_traits::One;
    if x <= &num_bigint::BigUint::one() {
        return (x.clone(), 1);
    }
    let max_p = x.bits() as u32;
    for p in (2..=max_p).rev() {
        let root = x.nth_root(p);
        if root.pow(p) == *x {
            return (root, p);
        }
    }
    (x.clone(), 1)
}

fn exact_rational_log(
    self_rat: &num_rational::BigRational,
    base_rat: &num_rational::BigRational,
) -> Option<num_rational::BigRational> {
    use num_traits::{One, Zero};
    if self_rat.is_zero()
        || base_rat.is_zero()
        || base_rat.is_one()
        || self_rat <= &num_rational::BigRational::zero()
        || base_rat <= &num_rational::BigRational::zero()
    {
        return None;
    }
    if self_rat.is_one() {
        return Some(num_rational::BigRational::zero());
    }
    if self_rat == base_rat {
        return Some(num_rational::BigRational::one());
    }
    let a = self_rat.numer().to_biguint()?;
    let b = self_rat.denom().to_biguint()?;
    let m = base_rat.numer().to_biguint()?;
    let n = base_rat.denom().to_biguint()?;

    if b.is_one() && n.is_one() {
        let (c_base, q) = canonical_base_exp(&m);
        let (c_self, p) = canonical_base_exp(&a);
        if c_base > num_bigint::BigUint::one() && c_base == c_self {
            return Some(num_rational::BigRational::new(
                num_bigint::BigInt::from(p),
                num_bigint::BigInt::from(q),
            ));
        }
    } else if a.is_one() && n.is_one() {
        let (c_base, q) = canonical_base_exp(&m);
        let (c_self, p) = canonical_base_exp(&b);
        if c_base > num_bigint::BigUint::one() && c_base == c_self {
            return Some(num_rational::BigRational::new(
                -num_bigint::BigInt::from(p),
                num_bigint::BigInt::from(q),
            ));
        }
    } else if b.is_one() && m.is_one() {
        let (c_base, q) = canonical_base_exp(&n);
        let (c_self, p) = canonical_base_exp(&a);
        if c_base > num_bigint::BigUint::one() && c_base == c_self {
            return Some(num_rational::BigRational::new(
                -num_bigint::BigInt::from(p),
                num_bigint::BigInt::from(q),
            ));
        }
    } else if a.is_one() && m.is_one() {
        let (c_base, q) = canonical_base_exp(&n);
        let (c_self, p) = canonical_base_exp(&b);
        if c_base > num_bigint::BigUint::one() && c_base == c_self {
            return Some(num_rational::BigRational::new(
                num_bigint::BigInt::from(p),
                num_bigint::BigInt::from(q),
            ));
        }
    } else {
        let (c_m, q_m) = canonical_base_exp(&m);
        let (c_n, q_n) = canonical_base_exp(&n);
        let (c_a, p_a) = canonical_base_exp(&a);
        let (c_b, p_b) = canonical_base_exp(&b);

        if self_rat > &num_rational::BigRational::one()
            && base_rat > &num_rational::BigRational::one()
        {
            if c_m == c_a && c_n == c_b && q_m == q_n && p_a == p_b && c_m > num_bigint::BigUint::one() {
                return Some(num_rational::BigRational::new(
                    num_bigint::BigInt::from(p_a),
                    num_bigint::BigInt::from(q_m),
                ));
            }
        } else if self_rat < &num_rational::BigRational::one()
            && base_rat > &num_rational::BigRational::one()
        {
            if c_m == c_b && c_n == c_a && q_m == q_n && p_a == p_b && c_m > num_bigint::BigUint::one() {
                return Some(num_rational::BigRational::new(
                    -num_bigint::BigInt::from(p_a),
                    num_bigint::BigInt::from(q_m),
                ));
            }
        } else if self_rat > &num_rational::BigRational::one()
            && base_rat < &num_rational::BigRational::one()
        {
            if c_n == c_a && c_m == c_b && q_m == q_n && p_a == p_b && c_n > num_bigint::BigUint::one() {
                return Some(num_rational::BigRational::new(
                    -num_bigint::BigInt::from(p_a),
                    num_bigint::BigInt::from(q_m),
                ));
            }
        } else if self_rat < &num_rational::BigRational::one()
            && base_rat < &num_rational::BigRational::one()
        {
            if c_n == c_b && c_m == c_a && q_m == q_n && p_a == p_b && c_n > num_bigint::BigUint::one() {
                return Some(num_rational::BigRational::new(
                    num_bigint::BigInt::from(p_a),
                    num_bigint::BigInt::from(q_m),
                ));
            }
        }
    }
    None
}

impl Number {
    /// Apply a monotone-increasing function to the real interval.
    /// Returns false if any resulting bound is NaN (domain error).
    fn apply_monotone(&mut self, f: MonoFn) -> bool {
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            context::with_consts(|cc| {
                (f(&al, p, RoundingMode::Down, cc), f(&au, p, RoundingMode::Up, cc))
            })
        } else {
            let v = context::with_consts(|cc| f(&al, p, RoundingMode::ToEven, cc));
            (v.clone(), v)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `ln()` — natural logarithm. Complex for negative reals.
    pub fn ln(&mut self) -> bool {
        // d(ln x)/dx = 1/x.
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    d.recip().then_some(d)
                },
                Number::ln_impl,
            );
        }
        self.ln_impl()
    }

    fn ln_impl(&mut self) -> bool {
        // Order is the reference's (Number.cc:7580): the infinities and the
        // two exact values are decided before anything looks at the
        // imaginary part.
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            // ln(-infinity) = +infinity + pi·i, not a failure.
            let mut pi = Number::new();
            pi.pi();
            self.set_plus_infinity(true, false);
            self.set_imaginary_part(&pi);
            return true;
        }
        if self.is_one() {
            self.clear(true);
            return true;
        }
        if self.is_zero() {
            if self.is_imag_part {
                return false;
            }
            self.set_minus_infinity(true, false);
            self.approx = true;
            return true;
        }
        if self.has_imaginary_part() {
            // ln(z) = ln|z| + i·arg(z), with `arg` as the `atan2` of the two
            // parts — `allow_zero`, because a real part that straddles zero is
            // still a perfectly good angle here.
            let re = self.real_part();
            let mut new_i = self.imaginary_part();
            if !new_i.atan2(&re, true) || new_i.has_imaginary_part() {
                return false;
            }
            let mut new_r = self.clone();
            if !new_r.abs() || new_r.has_imaginary_part() || !new_r.ln() {
                return false;
            }
            *self = new_r;
            self.set_imaginary_part(&new_i);
            return true;
        }
        if self.is_non_positive() {
            if self.is_imag_part {
                return false;
            }
            // ln(-x) = ln(x) + pi·i
            let mut new_r = self.clone();
            if !new_r.abs() || !new_r.ln() {
                return false;
            }
            let mut pi = Number::new();
            pi.pi();
            *self = new_r;
            self.set_imaginary_part(&pi);
            return true;
        }

        let p = context::bit_precision();
        let bak = self.clone();
        let (fl, mut fu) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let ln = |x: &BigFloat, rm: RoundingMode| context::with_consts(|cc| x.ln(p, rm, cc));
        let mut straddles = false;
        let (lower, upper) = if !context::create_interval() && !self.is_interval(true) {
            let v = ln(&fl, RoundingMode::ToEven);
            (v.clone(), v)
        } else if bf_sgn(&fl) < 0 {
            // The interval straddles zero (it is not non-positive, so the
            // upper bound is above it): the image runs down to -infinity, and
            // the argument sweeps [0, pi].
            if bf_abs_cmp(&fl, &fu) > 0 {
                fu = fl.neg();
            }
            straddles = true;
            (
                BigFloat::from_f64(f64::NEG_INFINITY, p),
                ln(&fu, RoundingMode::Up),
            )
        } else {
            // ln(0) is -infinity rather than the NaN the bare call gives.
            let lo = if fl.is_zero() {
                BigFloat::from_f64(f64::NEG_INFINITY, p)
            } else {
                ln(&fl, RoundingMode::Down)
            };
            (lo, ln(&fu, RoundingMode::Up))
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.imag = None;
        self.approx = true;
        if !self.test_float_result(true) {
            *self = bak;
            return false;
        }
        if straddles {
            let mut pi = Number::new();
            pi.pi();
            let zero = Number::new();
            let mut arc = Number::new();
            if !arc.set_interval(&zero, &pi, false) {
                *self = bak;
                return false;
            }
            self.set_imaginary_part(&arc);
        }
        true
    }

    /// `log(base)` — a transcription of Number.cc:7655.
    ///
    /// The quotient `ln(x)/ln(base)` is the general case and the *only* case
    /// this used to have, which is why a base of zero, a negative base or one
    /// straddling zero was refused outright: none of them survives a division.
    /// The reference reaches them anyway, because `ln(0)` is an infinity it is
    /// happy to carry and because the quotient is formed as `ln(x)·(1/ln b)`.
    pub fn log(&mut self, base: &Number) -> bool {
        // `log_b(1) = 0` for every base that is definitely not 1.
        let one = Number::from_i64(1);
        if self.is_one()
            && (base.is_greater_than(&one)
                || base.is_less_than(&one)
                || base.imaginary_part().is_nonzero())
        {
            self.clear(true);
            self.set_precision_and_approximate_from(base);
            return true;
        }
        if base.is_one() || base.is_zero() {
            return false;
        }
        // `log_x(x)` is an exact 1 — recognised before the floats get a
        // chance to leave a hair of interval width behind.
        if self.equals(base, false, false) {
            let keep = self.approx;
            *self = Number::from_i64(1);
            self.approx = keep;
            self.set_precision_and_approximate_from(base);
            return true;
        }
        if let (Some(self_rat), Some(base_rat)) = (self.internal_rational(), base.internal_rational()) {
            if !self.has_imaginary_part() && !base.has_imaginary_part() && !self.is_approximate() && !base.is_approximate() {
                if let Some(res_rat) = exact_rational_log(self_rat, base_rat) {
                    let mut res = Number::from_rational(res_rat);
                    res.approx = false;
                    res.set_precision_and_approximate_from(base);
                    *self = res;
                    return true;
                }
            }
        }
        let mut num = self.clone();
        let mut den = base.clone();
        if !num.ln() || !den.ln() || !den.recip() || !num.multiply(&den) {
            return false;
        }
        if self.is_imag_part && num.has_imaginary_part() {
            return false;
        }
        *self = num;
        true
    }

    /// `exp()`.
    pub fn exp(&mut self) -> bool {
        // d(exp x)/dx = exp x.
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    d.exp().then_some(d)
                },
                Number::exp_impl,
            );
        }
        self.exp_impl()
    }

    fn exp_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            // e^(a+bi) = e^a (cos b + i sin b)
            let mut ea = self.real_part();
            if !ea.exp() {
                return false;
            }
            let b = self.imaginary_part();
            let mut cb = b.clone();
            let mut sb = b;
            if !cb.cos() || !sb.sin() {
                return false;
            }
            let mut im = ea.clone();
            if !im.multiply(&sb) || !ea.multiply(&cb) {
                return false;
            }
            *self = ea;
            if !im.is_zero() {
                self.set_imaginary_part(&im);
            }
            return true;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            self.clear(true);
            self.approx = true;
            return true;
        }
        if self.is_zero() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.exp(p, rm, cc))
    }

    /// `sin()`.
    pub fn sin(&mut self) -> bool {
        // `sin(f)' = f'·cos(f)` (MathStructure-differentiate.cc:480).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    d.cos().then_some(d)
                },
                Number::sin_impl,
            );
        }
        self.sin_impl()
    }

    fn sin_impl(&mut self) -> bool {
        // An unbounded real interval is the one infinite argument whose sine
        // is known without evaluating anything: the whole of `[-1:1]`. The
        // reference answers it here (Number.cc:6522) and refuses every other
        // infinite argument.
        if self.includes_infinity() {
            if !self.has_imaginary_part() && self.is_interval(true) {
                return self.set_interval(&Number::from_i64(-1), &Number::from_i64(1), true);
            }
            return false;
        }
        if self.has_imaginary_part() {
            // sin(a+bi) = sin a cosh b + i cos a sinh b
            let a = self.real_part();
            let b = self.imaginary_part();
            let (mut sa, mut ca) = (a.clone(), a);
            let (mut chb, mut shb) = (b.clone(), b);
            if !sa.sin() || !ca.cos() || !chb.cosh() || !shb.sinh() {
                return false;
            }
            if !sa.multiply(&chb) || !ca.multiply(&shb) {
                return false;
            }
            *self = sa;
            if !ca.is_zero() {
                self.set_imaginary_part(&ca);
            }
            return true;
        }
        if self.is_infinite(true) {
            return false;
        }
        if self.is_zero() {
            return true;
        }
        self.apply_trig_bounded(|x, p, rm, cc| x.sin(p, rm, cc), TrigKind::Sin)
    }

    /// `cos()`.
    pub fn cos(&mut self) -> bool {
        // `cos(f)' = −f'·sin(f)` (MathStructure-differentiate.cc:488).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.sin() && d.negate()).then_some(d)
                },
                Number::cos_impl,
            );
        }
        self.cos_impl()
    }

    fn cos_impl(&mut self) -> bool {
        // As in `sin_impl` (Number.cc:6852).
        if self.includes_infinity() {
            if !self.has_imaginary_part() && self.is_interval(true) {
                return self.set_interval(&Number::from_i64(-1), &Number::from_i64(1), true);
            }
            return false;
        }
        if self.has_imaginary_part() {
            // cos(a+bi) = cos a cosh b − i sin a sinh b
            let a = self.real_part();
            let b = self.imaginary_part();
            let (mut sa, mut ca) = (a.clone(), a);
            let (mut chb, mut shb) = (b.clone(), b);
            if !sa.sin() || !ca.cos() || !chb.cosh() || !shb.sinh() {
                return false;
            }
            if !ca.multiply(&chb) || !sa.multiply(&shb) || !sa.negate() {
                return false;
            }
            *self = ca;
            if !sa.is_zero() {
                self.set_imaginary_part(&sa);
            }
            return true;
        }
        if self.is_infinite(true) {
            return false;
        }
        if self.is_zero() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        self.apply_trig_bounded(|x, p, rm, cc| x.cos(p, rm, cc), TrigKind::Cos)
    }

    /// `tan()`.
    pub fn tan(&mut self) -> bool {
        // `tan(f)' = f'·(1 + tan(f)²)` (MathStructure-differentiate.cc:498).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.tan() && d.square() && d.add(&Number::from_i64(1))).then_some(d)
                },
                Number::tan_impl,
            );
        }
        self.tan_impl()
    }

    fn tan_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            let mut s = self.clone();
            let mut c = self.clone();
            if !s.sin() || !c.cos() {
                return false;
            }
            if !s.divide(&c) {
                return false;
            }
            *self = s;
            return true;
        }
        if self.is_infinite(true) {
            return false;
        }
        if self.is_zero() {
            return true;
        }
        // Reject intervals spanning an asymptote: tan monotone increasing on
        // each branch; if tan(l) > tan(u) the interval crossed a pole.
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            context::with_consts(|cc| {
                (al.tan(p, RoundingMode::Down, cc), au.tan(p, RoundingMode::Up, cc))
            })
        } else {
            let v = context::with_consts(|cc| al.tan(p, RoundingMode::ToEven, cc));
            (v.clone(), v)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        if matches!(lower.cmp(&upper), Some(c) if c > 0) {
            return false; // pole inside interval
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `sinh()` — monotone increasing.
    pub fn sinh(&mut self) -> bool {
        // `sinh(f)' = f'·cosh(f)` (MathStructure-differentiate.cc:506).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    d.cosh().then_some(d)
                },
                Number::sinh_impl,
            );
        }
        self.sinh_impl()
    }

    fn sinh_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.sinh_complex();
        }
        if self.is_infinite(true) {
            return true;
        }
        if self.is_zero() {
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.sinh(p, rm, cc))
    }

    /// `cosh()` — decreasing on (−∞,0], increasing on [0,∞).
    pub fn cosh(&mut self) -> bool {
        // `cosh(f)' = f'·sinh(f)` (MathStructure-differentiate.cc:512).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    d.sinh().then_some(d)
                },
                Number::cosh_impl,
            );
        }
        self.cosh_impl()
    }

    fn cosh_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.cosh_complex();
        }
        if self.is_infinite(true) {
            self.value = RealValue::PlusInfinity;
            return true;
        }
        if self.is_zero() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let spans_zero = matches!(al.sign(), Some(astro_float::Sign::Neg))
            && matches!(au.sign(), Some(astro_float::Sign::Pos));
        if !context::create_interval() {
            return self.apply_monotone(|x, p, rm, cc| x.cosh(p, rm, cc));
        }
        let (lower, upper) = context::with_consts(|cc| {
            if spans_zero {
                let cl = al.cosh(p, RoundingMode::Up, cc);
                let cu = au.cosh(p, RoundingMode::Up, cc);
                let hi = if matches!(cl.cmp(&cu), Some(c) if c > 0) { cl } else { cu };
                (BigFloat::from_i8(1, p), hi)
            } else if matches!(au.sign(), Some(astro_float::Sign::Neg)) {
                // negative interval: decreasing
                (au.cosh(p, RoundingMode::Down, cc), al.cosh(p, RoundingMode::Up, cc))
            } else {
                (al.cosh(p, RoundingMode::Down, cc), au.cosh(p, RoundingMode::Up, cc))
            }
        });
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `tanh()` — monotone increasing.
    pub fn tanh(&mut self) -> bool {
        // `tanh(f)' = f'·(1 − tanh(f)²)` (MathStructure-differentiate.cc:518).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.tanh() && d.square() && d.negate() && d.add(&Number::from_i64(1)))
                        .then_some(d)
                },
                Number::tanh_impl,
            );
        }
        self.tanh_impl()
    }

    fn tanh_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.tanh_complex();
        }
        if self.is_plus_infinity() {
            *self = Number::from_i64(1);
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            *self = Number::from_i64(-1);
            self.approx = true;
            return true;
        }
        if self.is_zero() {
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.tanh(p, rm, cc))
    }

    /// `asin()` — monotone increasing on [−1,1].
    pub fn asin(&mut self) -> bool {
        // `asin(f)' = f'/sqrt(1 − f²)` (MathStructure-differentiate.cc:526).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| one_over_sqrt_one_minus_square(x),
                Number::asin_impl,
            );
        }
        self.asin_impl()
    }

    fn asin_impl(&mut self) -> bool {
        // `Number::asin` opens with this test (Number.cc:6650), and it is not
        // redundant with the range check below: an *unbounded* argument — a
        // half-infinite interval, or one whose imaginary part is — has no
        // enclosure, and the complex composition below would invent one out of
        // whatever `ln` and `sqrt` make of an infinite bound.
        if self.includes_infinity() {
            return false;
        }
        if self.has_imaginary_part() {
            return self.asin_complex();
        }
        if self.is_zero() {
            return true;
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if self.is_greater_than(&one) || self.is_less_than(&mone) {
            // Outside [-1, 1] the result is complex.
            return self.asin_complex();
        }
        self.apply_monotone(|x, p, rm, cc| x.asin(p, rm, cc))
    }

    /// `acos()` — monotone decreasing on [−1,1].
    pub fn acos(&mut self) -> bool {
        // `acos(f)' = −f'/sqrt(1 − f²)` (MathStructure-differentiate.cc:535).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = one_over_sqrt_one_minus_square(x)?;
                    d.negate().then_some(d)
                },
                Number::acos_impl,
            );
        }
        self.acos_impl()
    }

    fn acos_impl(&mut self) -> bool {
        // As in `asin_impl`; `Number::acos` guards the same way (:6984).
        if self.includes_infinity() {
            return false;
        }
        if self.has_imaginary_part() {
            return self.acos_complex();
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if self.is_greater_than(&one) || self.is_less_than(&mone) {
            return self.acos_complex();
        }
        if self.is_one() {
            self.clear(true);
            return true;
        }
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            context::with_consts(|cc| {
                (au.acos(p, RoundingMode::Down, cc), al.acos(p, RoundingMode::Up, cc))
            })
        } else {
            let v = context::with_consts(|cc| al.acos(p, RoundingMode::ToEven, cc));
            (v.clone(), v)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `atan()` — monotone increasing.
    pub fn atan(&mut self) -> bool {
        // `atan(f)' = f'/(1 + f²)` (MathStructure-differentiate.cc:546).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.square() && d.add(&Number::from_i64(1)) && d.recip()).then_some(d)
                },
                Number::atan_impl,
            );
        }
        self.atan_impl()
    }

    fn atan_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.atan_complex();
        }
        if self.is_zero() {
            return true;
        }
        if self.is_plus_infinity() || self.is_minus_infinity() {
            let neg = self.is_minus_infinity();
            let mut pi = Number::new();
            pi.pi();
            if !pi.divide(&Number::from_i64(2)) {
                return false;
            }
            if neg {
                pi.negate();
            }
            *self = pi;
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.atan(p, rm, cc))
    }

    /// `asinh()` — monotone increasing.
    pub fn asinh(&mut self) -> bool {
        // `asinh(f)' = f'/sqrt(1 + f²)` (MathStructure-differentiate.cc:560).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.square() && d.add(&Number::from_i64(1)) && d.sqrt() && d.recip())
                        .then_some(d)
                },
                Number::asinh_impl,
            );
        }
        self.asinh_impl()
    }

    fn asinh_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.asinh_complex();
        }
        if self.is_zero() || self.is_infinite(true) {
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.asinh(p, rm, cc))
    }

    /// `acosh()` — monotone increasing on [1,∞).
    pub fn acosh(&mut self) -> bool {
        // `acosh(f)' = f'/(sqrt(f − 1)·sqrt(f + 1))`
        // (MathStructure-differentiate.cc:568) — kept as two square roots
        // rather than `sqrt(f² − 1)`, which differs on `f < −1`.
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut lo = x.clone();
                    let mut hi = x.clone();
                    (lo.add(&Number::from_i64(-1))
                        && hi.add(&Number::from_i64(1))
                        && lo.sqrt()
                        && hi.sqrt()
                        && lo.multiply(&hi)
                        && lo.recip())
                    .then_some(lo)
                },
                Number::acosh_impl,
            );
        }
        self.acosh_impl()
    }

    fn acosh_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.acosh_complex();
        }
        if self.is_plus_infinity() {
            return true;
        }
        let one = Number::from_i64(1);
        if self.is_less_than(&one) {
            // Below 1, acosh leaves the reals.
            return self.acosh_complex();
        }
        if self.is_one() {
            self.clear(true);
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.acosh(p, rm, cc))
    }

    /// `atanh()` — monotone increasing on (−1,1).
    pub fn atanh(&mut self) -> bool {
        // `atanh(f)' = f'/(1 − f²)` (MathStructure-differentiate.cc:580).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.square() && d.negate() && d.add(&Number::from_i64(1)) && d.recip())
                        .then_some(d)
                },
                Number::atanh_impl,
            );
        }
        self.atanh_impl()
    }

    fn atanh_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.atanh_complex();
        }
        if self.is_zero() {
            return true;
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if self.is_greater_than(&one) || self.is_less_than(&mone) {
            // Outside (-1, 1) the result is complex.
            return self.atanh_complex();
        }
        self.apply_monotone(|x, p, rm, cc| x.atanh(p, rm, cc))
    }

    /// `atan2(x)` — self = atan2(self, x) (self is y).
    ///
    /// A transcription of Number.cc:7319. The result has to lie in (-pi, pi],
    /// which composing `atan(y/x)` and a quadrant correction does not
    /// guarantee once either operand is an interval: the branch cut runs
    /// straight through the box whenever `x` can be negative and `y` straddles
    /// zero, and the composed form walks past +pi instead of widening to the
    /// whole circle. So each bound is a corner evaluation of atan2 itself,
    /// picked by the sign pattern of the four endpoints — which also gives an
    /// infinite operand a limit instead of an `infinity/infinity`.
    pub fn atan2(&mut self, x: &Number, allow_zero: bool) -> bool {
        if self.has_imaginary_part() || x.has_imaginary_part() {
            return false;
        }
        if self.is_zero() {
            if allow_zero && x.is_non_negative() {
                self.clear(true);
                self.set_precision_and_approximate_from(x);
                return true;
            }
            if x.is_zero() {
                return false;
            }
            if x.is_positive() {
                self.clear(true);
                self.set_precision_and_approximate_from(x);
                return true;
            }
        }
        let p = context::bit_precision();
        let (yl, yu) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (xl, xu) = (x.lower_bound_float(p), x.upper_bound_float(p));

        let (lower, upper) = if !context::create_interval()
            && !self.is_interval(true)
            && !x.is_interval(true)
        {
            let v = atan2_bf(&yl, &xl, p, RoundingMode::ToEven);
            (v.clone(), v)
        } else {
            let (sgn_l, sgn_u) = (bf_sgn(&yl), bf_sgn(&yu));
            // `atan2` of an interval that surrounds the origin is the whole
            // circle, and no interval can say which end of the cut it is on.
            if !allow_zero && !x.is_nonzero() && (sgn_l != sgn_u || sgn_l == 0) {
                return false;
            }
            let (sgn_lo, sgn_uo) = (bf_sgn(&xl), bf_sgn(&xu));
            let pi_d = context::with_consts(|cc| cc.pi(p, RoundingMode::Down));
            let pi_u = context::with_consts(|cc| cc.pi(p, RoundingMode::Up));
            let zero = BigFloat::from_i8(0, p);
            if sgn_lo < 0 {
                if sgn_u >= 0 {
                    // The cut is inside the box unless `y` keeps one sign.
                    let lo = if sgn_l < 0 {
                        pi_d.neg()
                    } else if sgn_uo >= 0 {
                        if sgn_l == 0 {
                            zero.clone()
                        } else {
                            atan2_bf(&yl, &xu, p, RoundingMode::Down)
                        }
                    } else {
                        atan2_bf(&yu, &xu, p, RoundingMode::Down)
                    };
                    let hi = if sgn_l <= 0 {
                        pi_u
                    } else {
                        atan2_bf(&yl, &xl, p, RoundingMode::Up)
                    };
                    (lo, hi)
                } else {
                    (
                        atan2_bf(&yu, &xl, p, RoundingMode::Down),
                        atan2_bf(&yl, &xu, p, RoundingMode::Up),
                    )
                }
            } else if sgn_u >= 0 {
                let hi = if sgn_u == 0 {
                    zero.clone()
                } else {
                    atan2_bf(&yu, &xl, p, RoundingMode::Up)
                };
                let lo = if sgn_l == 0 {
                    zero
                } else if sgn_l < 0 {
                    atan2_bf(&yl, &xl, p, RoundingMode::Down)
                } else {
                    atan2_bf(&yl, &xu, p, RoundingMode::Down)
                };
                (lo, hi)
            } else {
                (
                    atan2_bf(&yl, &xl, p, RoundingMode::Down),
                    atan2_bf(&yu, &xu, p, RoundingMode::Up),
                )
            }
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        let bak = self.clone();
        self.value = RealValue::Float { lower, upper };
        self.imag = None;
        self.approx = true;
        // `testFloatResult()` — the reference passes no arguments here, so an
        // infinite bound is a failure rather than a result.
        if self.lower_bound_float(p).is_inf() || self.upper_bound_float(p).is_inf() {
            *self = bak;
            return false;
        }
        if !self.test_float_result(false) {
            *self = bak;
            return false;
        }
        self.set_precision_and_approximate_from(x);
        true
    }

    /// `arg()` — argument of the complex number.
    pub fn arg(&mut self) -> bool {
        // A value that may be zero has no argument (Number.cc:7402).
        if !self.is_nonzero() {
            return false;
        }
        if !self.has_imaginary_part() {
            if self.real_part_is_negative() {
                let mut pi = Number::new();
                pi.pi();
                *self = pi;
                return true;
            }
            self.clear(true);
            return true;
        }
        if !self.has_real_part() {
            // Purely imaginary: ±pi/2, without asking `atan2` about it.
            let neg = self.imaginary_part().real_part_is_negative();
            let mut pi = Number::new();
            pi.pi();
            if !pi.divide(&Number::from_i64(2)) {
                return false;
            }
            if neg {
                pi.negate();
            }
            *self = pi;
            return true;
        }
        let re = self.real_part();
        let mut im = self.imaginary_part();
        if !im.atan2(&re, false) {
            return false;
        }
        *self = im;
        true
    }
}

/// Which bounded trig function is being applied (for extremum handling).
enum TrigKind {
    Sin,
    Cos,
}

impl Number {
    /// sin/cos over an interval: if any extremum (odd multiples of π/2 for
    /// sin, multiples of π for cos) lies inside, the corresponding bound is
    /// ±1; otherwise evaluate endpoints. Falls back to [−1,1] for wide
    /// intervals.
    fn apply_trig_bounded(&mut self, f: MonoFn, kind: TrigKind) -> bool {
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        if !context::create_interval() || al == au {
            let v = context::with_consts(|cc| f(&al, p, RoundingMode::ToEven, cc));
            if v.is_nan() {
                return false;
            }
            let (lower, upper) = if context::create_interval() {
                // Point input still needs outward rounding of the result.
                context::with_consts(|cc| {
                    (f(&al, p, RoundingMode::Down, cc), f(&au, p, RoundingMode::Up, cc))
                })
            } else {
                (v.clone(), v)
            };
            self.value = RealValue::Float { lower, upper };
            self.approx = true;
            return self.test_float_result(true);
        }
        // Interval width ≥ 2π → full range.
        let width = au.sub(&al, p, RoundingMode::Up);
        let two_pi = context::with_consts(|cc| {
            cc.pi(p, RoundingMode::Up).mul(&BigFloat::from_i8(2, p), p, RoundingMode::Up)
        });
        let full_range = !matches!(width.cmp(&two_pi), Some(c) if c < 0);
        let (mut has_max, mut has_min) = (full_range, full_range);
        if !full_range {
            // Count extremum points k in the interval: for sin the maxima are
            // at (4k+1)·π/2, minima at (4k+3)·π/2; for cos maxima at 2kπ,
            // minima at (2k+1)π. Work with t = x / (π/2).
            let half_pi = context::with_consts(|cc| {
                cc.pi(p, RoundingMode::ToEven)
                    .div(&BigFloat::from_i8(2, p), p, RoundingMode::ToEven)
            });
            let tl = al.div(&half_pi, p, RoundingMode::Down);
            let tu = au.div(&half_pi, p, RoundingMode::Up);
            // Integers in [tl, tu] with the right residue mod 4. The lower
            // end must be *rounded up*: `floor(tl)` sits below the interval
            // and would report an extremum that is not in it — which is what
            // made `cos(0.8976)` widen to `[cos(0.8976), 1]`.
            let (kl, ku) = (bigfloat_ceil_i(&tl), bigfloat_floor_i(&tu));
            let mut k = kl.clone();
            let one = BigInt::from(1);
            while k <= ku {
                let residue = ((&k % 4i32) + 4i32) % 4i32;
                let r = residue.to_string();
                match kind {
                    TrigKind::Sin => {
                        if r == "1" {
                            has_max = true;
                        } else if r == "3" {
                            has_min = true;
                        }
                    }
                    TrigKind::Cos => {
                        if r == "0" {
                            has_max = true;
                        } else if r == "2" {
                            has_min = true;
                        }
                    }
                }
                if has_max && has_min {
                    break;
                }
                k += &one;
            }
        }
        let (fl_l, fl_u, fu_l, fu_u) = context::with_consts(|cc| {
            (
                f(&al, p, RoundingMode::Down, cc),
                f(&al, p, RoundingMode::Up, cc),
                f(&au, p, RoundingMode::Down, cc),
                f(&au, p, RoundingMode::Up, cc),
            )
        });
        if fl_l.is_nan() || fu_l.is_nan() {
            return false;
        }
        let lower = if has_min {
            BigFloat::from_i8(-1, p)
        } else if matches!(fl_l.cmp(&fu_l), Some(c) if c > 0) {
            fu_l
        } else {
            fl_l
        };
        let upper = if has_max {
            BigFloat::from_i8(1, p)
        } else if matches!(fl_u.cmp(&fu_u), Some(c) if c < 0) {
            fu_u
        } else {
            fl_u
        };
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }
}

/// floor of a BigFloat as BigInt (assumes finite).
fn bigfloat_floor_i(f: &BigFloat) -> BigInt {
    match crate::float::bigfloat_to_ratio(f) {
        Some((n, d)) => num_integer::Integer::div_floor(&n, &d),
        None => BigInt::zero(),
    }
}

/// ceil of a BigFloat as BigInt (assumes finite).
fn bigfloat_ceil_i(f: &BigFloat) -> BigInt {
    match crate::float::bigfloat_to_ratio(f) {
        Some((n, d)) => num_integer::Integer::div_ceil(&n, &d),
        None => BigInt::zero(),
    }
}

#[cfg(test)]
mod uncertainty_tests {
    use crate::number::uncertainty_test_support::{plus_minus, uncertain};

    #[test]
    fn sine_carries_the_cosine() {
        // Reference: `sin(1+/-0.1)` = `0.841±0.055` — |cos 1|·0.1.
        let mut n = uncertain("1", "0.1");
        assert!(n.sin());
        assert_eq!(plus_minus(&n), "0.841±0.055");
    }

    #[test]
    fn cosine_carries_the_sine() {
        // Reference: `cos(1+/-0.1)` = `0.540±0.084` — |−sin 1|·0.1.
        let mut n = uncertain("1", "0.1");
        assert!(n.cos());
        assert_eq!(plus_minus(&n), "0.540±0.084");
    }

    #[test]
    fn tangent_carries_one_plus_its_square() {
        // Reference: `tan(1+/-0.1)` = `1.56±0.35` — (1 + tan²1)·0.1.
        let mut n = uncertain("1", "0.1");
        assert!(n.tan());
        assert_eq!(plus_minus(&n), "1.56±0.35");
    }

    #[test]
    fn arctangent_carries_the_rational_derivative() {
        // Reference: `atan(1+/-0.1)` = `0.785±0.050` — 0.1/(1 + 1²).
        let mut n = uncertain("1", "0.1");
        assert!(n.atan());
        assert_eq!(plus_minus(&n), "0.785±0.050");
    }

    #[test]
    fn arcsine_carries_the_inverse_root() {
        // Reference: `asin(0.5+/-0.1)` = `0.52±0.12` — 0.1/sqrt(1 − 0.25).
        let mut n = uncertain("0.5", "0.1");
        assert!(n.asin());
        assert_eq!(plus_minus(&n), "0.52±0.12");
    }

    #[test]
    fn hyperbolic_sine_carries_the_hyperbolic_cosine() {
        // Reference: `sinh(1+/-0.1)` = `1.18±0.16` — cosh(1)·0.1.
        let mut n = uncertain("1", "0.1");
        assert!(n.sinh());
        assert_eq!(plus_minus(&n), "1.18±0.16");
    }

    #[test]
    fn hyperbolic_tangent_carries_one_minus_its_square() {
        // Reference: `tanh(1+/-0.1)` = `0.762±0.042` — (1 − tanh²1)·0.1.
        let mut n = uncertain("1", "0.1");
        assert!(n.tanh());
        assert_eq!(plus_minus(&n), "0.762±0.042");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PrintOptions;

    fn po_ez() -> PrintOptions {
        let mut po = PrintOptions::default();
        po.show_ending_zeroes = true;
        po
    }

    #[test]
    fn cos_does_not_invent_an_extremum() {
        // Oracle: `cos(0.8975979)` = 0.6234898027. The extremum scan must
        // only count multiples of π/2 that are actually inside the interval;
        // counting `floor(l)` widened the result to `[cos x, 1]`, whose
        // midpoint printed as 0.81.
        let mut n = Number::parse("0.8975979", &Default::default());
        assert!(n.cos());
        assert_eq!(n.print(&po_ez()), "0.6234898027");
    }

    #[test]
    fn sin_and_cos_still_clamp_a_contained_extremum() {
        // An interval wider than 2π must map onto the whole range.
        let mut n = Number::new();
        assert!(n.set_interval(&Number::from_i64(0), &Number::from_i64(10), false));
        assert!(n.cos());
        assert!(n.lower_end_point().is_less_than_or_equal_to(&Number::from_i64(-1)));
        assert!(n.upper_end_point().is_greater_than_or_equal_to(&Number::from_i64(1)));
    }

    #[test]
    fn atan2_point_values() {
        // Oracle: `atan2(5, 0.2)` = 1.530817640.
        let mut y = Number::from_i64(5);
        assert!(y.atan2(&Number::parse("0.2", &Default::default()), false));
        assert_eq!(y.print(&po_ez()), "1.530817640");
    }

    #[test]
    fn atan2_over_an_x_interval_straddling_zero() {
        // `arg((5±0.003)i - 0±0.2)` is `1.571±0.040` in the reference: with x
        // straddling zero the quadrant formula is useless and `±π/2 −
        // atan(x/y)` has to take over, keeping the interval width.
        let po = crate::options::ParseOptions::default();
        let mut x = Number::new();
        assert!(x.set_interval(&Number::parse("-0.2", &po), &Number::parse("0.2", &po), false));
        let mut y = Number::new();
        assert!(y.set_interval(&Number::parse("4.997", &po), &Number::parse("5.003", &po), false));
        assert!(y.atan2(&x, false));
        let mut pm = crate::options::PrintOptions::default();
        pm.interval_display = crate::options::IntervalDisplay::PlusMinus;
        assert_eq!(y.print(&pm), "1.571±0.040");
    }

    #[test]
    fn ln_exp_roundtrip() {
        let mut n = Number::from_i64(10);
        assert!(n.ln());
        assert_eq!(n.print(&po_ez()), "2.302585093");
        let mut e = Number::from_i64(1);
        assert!(e.exp());
        assert_eq!(e.print(&po_ez()), "2.718281828");
        let mut one = Number::from_i64(1);
        assert!(one.ln());
        assert!(one.is_zero(), "ln(1) = 0 exact");
    }

    #[test]
    fn ln_negative_complex() {
        let mut n = Number::from_i64(-1);
        assert!(n.ln());
        assert!(n.is_complex(), "ln(-1) = πi");
        assert!(!n.has_real_part());
        assert_eq!(n.imaginary_part().print(&po_ez()), "3.141592654");
    }

    #[test]
    fn trig_basics() {
        let mut n = Number::from_i64(1);
        assert!(n.sin());
        assert_eq!(n.print(&po_ez()), "0.8414709848");
        let mut c = Number::from_i64(1);
        assert!(c.cos());
        assert_eq!(c.print(&po_ez()), "0.5403023059");
        let mut t = Number::from_i64(1);
        assert!(t.tan());
        assert_eq!(t.print(&po_ez()), "1.557407725");
    }

    #[test]
    fn sin_pi_contains_zero() {
        let mut pi = Number::new();
        pi.pi();
        assert!(pi.sin());
        // sin(π-interval) is a tiny interval containing 0 — must not claim
        // to be nonzero.
        assert!(!pi.is_nonzero() || pi.is_zero());
    }

    #[test]
    fn sin_interval_spanning_max() {
        // Interval [1, 2.5] contains π/2 → upper bound must be exactly 1.
        let mut n = Number::new();
        assert!(n.set_interval(&Number::from_i64(1), &Number::from_ints(25, 10, 0), false));
        assert!(n.sin());
        let hi = n.upper_end_point();
        assert!(hi.equals_i64(1), "sin over [1,2.5] has max exactly 1, got {hi:?}");
    }

    #[test]
    fn atan_infinity() {
        let mut n = Number::new();
        n.set_plus_infinity(false, false);
        assert!(n.atan());
        assert_eq!(n.print(&po_ez()), "1.570796327");
    }

    #[test]
    fn inverse_trig() {
        let mut n = Number::from_ints(1, 2, 0);
        assert!(n.asin());
        assert_eq!(n.print(&po_ez()), "0.5235987756");
        let mut c = Number::from_ints(1, 2, 0);
        assert!(c.acos());
        assert_eq!(c.print(&po_ez()), "1.047197551");
        // asin(2) now leaves the real domain instead of failing.
        let mut out = Number::from_i64(2);
        assert!(out.asin());
        assert!(out.is_complex());
    }

    #[test]
    fn hyperbolic() {
        let mut n = Number::from_i64(1);
        assert!(n.sinh());
        assert_eq!(n.print(&po_ez()), "1.175201194");
        let mut c = Number::from_i64(1);
        assert!(c.cosh());
        assert_eq!(c.print(&po_ez()), "1.543080635");
        let mut t = Number::from_i64(1);
        assert!(t.tanh());
        assert_eq!(t.print(&po_ez()), "0.7615941560");
    }

    #[test]
    fn exp_complex_euler() {
        // e^(iπ) = −1 (within interval tolerance the imaginary part vanishes
        // or is negligible; real part ≈ −1)
        let mut z = Number::new();
        let mut pi = Number::new();
        pi.pi();
        z.set_imaginary_part(&pi);
        assert!(z.exp());
        let re = z.real_part();
        assert!(re.is_less_than(&Number::from_ints(-99, 100, 0)));
        assert!(re.is_greater_than(&Number::from_ints(-101, 100, 0)));
    }

    #[test]
    fn log_base() {
        let mut n = Number::from_i64(8);
        assert!(n.log(&Number::from_i64(2)));
        assert_eq!(n, Number::from_i64(3));
        assert!(!n.is_approximate());
    }

    #[test]
    fn exact_rational_log_test() {
        let mut n = Number::from_i64(4);
        assert!(n.log(&Number::from_i64(2)));
        assert_eq!(n, Number::from_i64(2));
        assert!(!n.is_approximate());

        let mut n2 = Number::from_i64(100);
        assert!(n2.log(&Number::from_i64(10)));
        assert_eq!(n2, Number::from_i64(2));
        assert!(!n2.is_approximate());

        let mut n3 = Number::from_rational(num_rational::BigRational::new(1.into(), 100.into()));
        assert!(n3.log(&Number::from_i64(10)));
        assert_eq!(n3, Number::from_i64(-2));
        assert!(!n3.is_approximate());
    }
}
