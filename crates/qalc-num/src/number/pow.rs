//! Powers and roots — port of `Number::raise`, `sqrt`, `isqrt`, `root`
//! (exact paths + float-interval fallback).

use super::{Number, RealValue};
use crate::context;
use crate::float::{bigfloat_from_ratio, bigfloat_to_ratio};
use astro_float::{BigFloat, RoundingMode};
use num_bigint::{BigInt, BigUint};
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

impl Number {
    /// `raise(o, try_exact)`: self = self^o.
    pub fn raise(&mut self, o: &Number, try_exact: bool) -> bool {
        // d(x^y)/dx = y·x^(y−1), d(x^y)/dy = x^y·ln x.
        if self.either_uncertain(o) {
            return self.uncertain_binary(
                o,
                |x, y| {
                    let mut d = x.clone();
                    let mut e = y.clone();
                    (e.add(&Number::from_i64(-1)) && d.raise(&e, true) && d.multiply(y))
                        .then_some(d)
                },
                |x, y| {
                    let mut d = x.clone();
                    let mut l = x.clone();
                    (d.raise(y, true) && l.ln() && d.multiply(&l)).then_some(d)
                },
                |s, o| s.raise_impl(o, try_exact),
            );
        }
        self.raise_impl(o, try_exact)
    }

    fn raise_impl(&mut self, o: &Number, try_exact: bool) -> bool {
        // The reference's first two lines (Number.cc:3822): an exponent of 2
        // is `square()` and an exponent of -1 is `recip()`, both of which know
        // their operand appears twice where the general power does not.
        // `[-0.5:0.5]^2` is `[0:0.25]`, not the `[-0.25:0.25]` that combining
        // the corners of two independent factors gives.
        if o.is_two() {
            return self.square();
        }
        if o.is_minus_one() {
            if !self.recip() {
                return false;
            }
            self.set_precision_and_approximate_from(o);
            return true;
        }
        // Infinite base/exponent, *before* the `x^0 = 1` shortcut.
        //
        // The reference orders it this way (Number.cc:3841, whose
        // `!o.isNonZero() -> return false` runs long before its own
        // `o.isZero()` branch at :3925), which is what leaves `infinity^0`
        // unevaluated as `(+infinity)^0` rather than answering 1. Taking the
        // shortcut first would decide the indeterminate form.
        //
        // The test is `includesInfinity`, not "is an infinity": a half-infinite
        // interval decides the same cases a plain infinity does. `raise_infinite`
        // answers some of them outright and hands the rest back, sometimes with
        // `self` rewritten — see its documentation.
        if (self.includes_infinity() || o.includes_infinity())
            && !self.has_imaginary_part()
            && !o.has_imaginary_part()
        {
            if let Some(answer) = self.raise_infinite(o) {
                return answer;
            }
        }
        // Handle x^0 and x^1 quickly.
        if o.is_zero() {
            // `x^0` is 1 only for an `x` that is *known* non-zero: an interval
            // straddling zero contains the undefined `0^0`, and an infinite
            // imaginary part has no finite power either (Number.cc:3926).
            if !self.is_nonzero()
                || (self.has_imaginary_part() && self.imaginary_part().includes_infinity())
            {
                return false;
            }
            let keep = (self.approx || o.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        if o.is_one() {
            self.set_precision_and_approximate_from(o);
            return true;
        }
        if self.has_imaginary_part() || o.has_imaginary_part() {
            return self.raise_complex(o);
        }
        // A base that reaches zero raised to an exponent that may be negative
        // is a division by zero somewhere inside the interval, so there is no
        // enclosure to give. The reference refuses it twice over — once for a
        // floating-point exponent (Number.cc:4136) and once for a rational one
        // (:4208) — and both come to the same test.
        if !self.is_nonzero() && !o.is_non_negative() {
            return false;
        }
        // Exact integer exponent on rational base.
        if let (RealValue::Rational(r), Some(exp)) = (&self.value, o.to_i64()) {
            if try_exact && exact_power_in_range(r, exp, 1, true) {
                if exp < 0 && r.is_zero() {
                    return false;
                }
                let mag = r.pow(exp.unsigned_abs().min(i32::MAX as u64) as i32 * exp.signum() as i32);
                self.value = RealValue::Rational(mag);
                self.set_precision_and_approximate_from(o);
                return true;
            }
        }
        // Rational exponent num/den: try exact root then integer power.
        //
        // Only for a non-negative base. `^` takes the PRINCIPAL root, which
        // for a negative base is complex — the reference gives
        // `(-8)^(1/3)` = 1 + 1.732…i — whereas `cbrt(-8)` and `root(-8, 3)`
        // give the real root -2. Taking the exact real root here would
        // silently answer the wrong question.
        //
        // The exact root is taken even when `try_exact` is false: it only
        // applies to a rational base with a fractional exponent (root finding
        // works on floats, so it never pays for the test), and it is the only
        // way to avoid the float `pow` below, whose Ziv refinement never
        // terminates when the result is exactly representable (`4^(1/2)`).
        if let (RealValue::Rational(base_r), RealValue::Rational(oe)) = (&self.value, &o.value) {
            if !oe.denom().is_one() && !base_r.is_negative() {
                if let Some(den) = oe.denom().to_u32() {
                    let mut base = self.clone();
                    if base.exact_root(den) {
                        let int_exp = Number::from_bigint(oe.numer().clone());
                        if base.raise(&int_exp, true) {
                            *self = base;
                            self.set_precision_and_approximate_from(o);
                            return true;
                        }
                    }
                }
            }
        }
        // Square root of a negative rational: exact and *purely* imaginary
        // whenever the magnitude is a perfect square.
        //
        // Port of the `complex_result` branch of `Number::raise`
        // (Number.cc:4067). The reference raises to the exponent's numerator
        // first, negates, and only then takes the integer square root, so
        // `(-4)^(1/2)` is exactly `2i` and `(-4)^(3/2)` exactly `-8i`. Going
        // through `exp(w·ln z)` instead leaves ~1e-58 of rounding dust in the
        // real part, which then contaminates everything downstream
        // (`(-1)^(1/2)*(-1)^(1/2)` printed `-1.000000000 - 3.1E-58i`).
        //
        // Only a denominator of 2 is handled: the reference restricts its
        // negative-base exact path to `i_root <= 2` as well, which is what
        // keeps `(-8)^(1/3)` on the principal (complex) branch.
        if !self.is_imag_part {
            if let (RealValue::Rational(base_r), RealValue::Rational(oe)) =
                (&self.value, &o.value)
            {
                if base_r.is_negative() && oe.denom().to_u32() == Some(2) {
                    if let Some(num) = oe.numer().to_i64() {
                        if num != 0 && exact_power_in_range(base_r, num, 2, true) {
                            // The exponent is in lowest terms, so a
                            // denominator of 2 forces an odd numerator and
                            // `base^num` stays negative.
                            let powed = base_r.pow(num as i32);
                            let mut mag = Number::from_rational(-powed);
                            if mag.exact_root(2) {
                                // i^num cycles: the sign flips for
                                // num ≡ 3 (mod 4), and again for a negative
                                // numerator (1/i = -i).
                                let flip = (num < 0) != (num.unsigned_abs() % 4 == 3);
                                if flip && !mag.negate() {
                                    return false;
                                }
                                let mut result = Number::new();
                                result.set_imaginary_part(&mag);
                                *self = result;
                                self.set_precision_and_approximate_from(o);
                                return true;
                            }
                        }
                    }
                }
            }
        }
        // Negative rational base with non-integer exponent → complex result;
        // only even roots produce complex values. Odd denominators of the
        // reduced exponent keep it real for negative bases? In libqalculate
        // (-8)^(1/3) is complex (principal root). Defer complex results.
        if self.real_part_is_negative() {
            if let RealValue::Rational(oe) = &o.value {
                if !oe.denom().is_one() {
                    return self.raise_complex(o);
                }
            }
        }
        // An exact integer exponent is always done by repeated squaring,
        // never by the transcendental fallback below.
        //
        // Two reasons. It is far cheaper — exp(b·ln a) costs eight
        // directed-rounding pow calls per operation, which made numeric root
        // finding (thousands of evaluations of `x^3`) take minutes. And it
        // is the only correct route for a negative base: `ln` of a negative
        // number does not converge, so `(-6)^3` through the pow path hangs
        // rather than returning -216. The exact-rational path above is
        // skipped whenever `try_exact` is false, which is exactly what the
        // approximate evaluation mode used by root finding requests, so this
        // has to catch rationals too, not just floats.
        if let Some(exp) = o.to_i64() {
            // A rational base is held to the same size guard as the exact
            // path above — the repeated squaring here is the *same* exact
            // computation, so letting it run would undo the guard. A float
            // base costs a bounded amount per multiplication, so only the
            // exponent is capped there.
            let in_range = match &self.value {
                RealValue::Rational(r) => exact_power_in_range(r, exp, 1, true),
                _ => exp.unsigned_abs() <= 1_000_000,
            };
            if exp != 0 && in_range {
                let neg = exp < 0;
                let mut e = exp.unsigned_abs();
                let mut acc = Number::from_i64(1);
                let mut sq = self.clone();
                while e > 0 {
                    if e & 1 == 1 && !acc.multiply(&sq) {
                        return false;
                    }
                    e >>= 1;
                    if e > 0 {
                        let s = sq.clone();
                        if !sq.multiply(&s) {
                            return false;
                        }
                    }
                }
                if neg && !acc.recip() {
                    return false;
                }
                *self = acc;
                self.set_precision_and_approximate_from(o);
                return true;
            }
        }
        // `a^(n/2^k)` goes through repeated `sqrt`, never through `pow`.
        //
        // astro-float's `pow` refines until it can decide the rounding, and
        // an exactly representable result never settles that decision — the
        // refinement then runs forever. Only a *binary* exponent can produce
        // one (`4^(1/2)`, `16^(1/4)`, `4.0^(3/2)`), and those are exactly the
        // exponents this branch takes over.
        if !self.real_part_is_negative() {
            let mut e = o.clone();
            let mut base = self.clone();
            let mut steps = 0;
            while steps < 20 && !e.is_integer() && e.denominator_is_even() {
                if !base.sqrt() || !e.multiply(&Number::from_i64(2)) {
                    break;
                }
                steps += 1;
            }
            if steps > 0 && e.is_integer() {
                if let Some(n) = e.to_i64() {
                    if n.unsigned_abs() <= 1_000_000
                        && base.raise(&Number::from_i64(n), true)
                    {
                        *self = base;
                        self.set_precision_and_approximate_from(o);
                        return true;
                    }
                }
            }
        }
        // Float fallback: a^b = exp(b·ln(a)) computed by astro-float pow with
        // directed rounding on interval corners.
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (bl, bu) = (o.lower_bound_float(p), o.upper_bound_float(p));
        if matches!(al.sign(), Some(astro_float::Sign::Neg)) && !al.is_zero() {
            // Negative base with (effectively) integer float exponent only.
            if !o.is_integer() {
                // An interval base that straddles zero cannot go through
                // `exp(b·ln a)` — `ln` has no value on it. The C++ splits it
                // instead (`try_complex`, Number.cc:4374): raise each end
                // point, hull each with zero, and hull the two results.
                if !matches!(au.sign(), Some(astro_float::Sign::Neg)) {
                    return self.raise_straddling_zero(o);
                }
                return self.raise_complex(o);
            }
        }
        let (lower, upper) = if context::create_interval() {
            let c1 = pow_bound(&al, &bl, p, RoundingMode::Down);
            let c2 = pow_bound(&al, &bu, p, RoundingMode::Down);
            let c3 = pow_bound(&au, &bl, p, RoundingMode::Down);
            let c4 = pow_bound(&au, &bu, p, RoundingMode::Down);
            let d1 = pow_bound(&al, &bl, p, RoundingMode::Up);
            let d2 = pow_bound(&al, &bu, p, RoundingMode::Up);
            let d3 = pow_bound(&au, &bl, p, RoundingMode::Up);
            let d4 = pow_bound(&au, &bu, p, RoundingMode::Up);
            let mut lo = c1.clone();
            for c in [&c2, &c3, &c4] {
                if matches!(c.cmp(&lo), Some(c) if c < 0) || lo.is_nan() {
                    lo = (*c).clone();
                }
            }
            let mut hi = d1.clone();
            for c in [&d2, &d3, &d4] {
                if matches!(c.cmp(&hi), Some(c) if c > 0) || hi.is_nan() {
                    hi = (*c).clone();
                }
            }
            (lo, hi)
        } else {
            // Directed rounding, not `ToEven`: astro-float's `pow` refines
            // until it can decide the rounding, and an exactly representable
            // result (`4^(1/2)`, `16^(1/4)`, `4^(3/2)`) never settles the
            // to-even tie, so the loop never ends. The interval branch above
            // already only uses Down/Up for the same reason.
            let f = pow_bound(&al, &bl, p, RoundingMode::Down);
            (f.clone(), f)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        let bak = self.clone();
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.set_precision_and_approximate_from(o);
        // An infinity the operands did not contain is a pole, not an answer.
        // `0^(-0.5)` and `[0:0.5]^(-0.5)` both run off to +infinity at zero,
        // and the reference refuses both: its float path ends with
        // `includesInfinity() && !nr_bak.includesInfinity() &&
        // !o.includesInfinity() -> return false` (Number.cc:4394). Without it
        // `[-0.5:0]^(-2)` reports `[-4:+infinity]`, an "enclosure" of a
        // function that is unbounded on the interval.
        if !self.test_float_result(true)
            || (self.includes_infinity() && !bak.includes_infinity() && !o.includes_infinity())
        {
            *self = bak;
            return false;
        }
        true
    }

    /// `x^o` for a real interval `x` that contains zero and a non-integer
    /// exponent — the `try_complex` interval branch of `Number::raise`
    /// (Number.cc:4376).
    ///
    /// `x^o` sweeps a curve from the negative end (complex) through zero to
    /// the positive end (real), so the enclosure is the hull of the two end
    /// results *and* zero, taken componentwise.
    fn raise_straddling_zero(&mut self, o: &Number) -> bool {
        let mut neg_end = self.lower_end_point();
        let mut pos_end = self.upper_end_point();
        if !neg_end.raise(o, false) || !pos_end.raise(o, false) {
            return false;
        }
        let zero = Number::new();
        let Some(re) = hull_of(&[zero.clone(), neg_end.real_part(), pos_end.real_part()]) else {
            return false;
        };
        let Some(im) = hull_of(&[zero, neg_end.imaginary_part(), pos_end.imaginary_part()]) else {
            return false;
        };
        *self = re;
        if !im.is_zero() {
            self.set_imaginary_part(&im);
        }
        self.approx = true;
        self.set_precision_and_approximate_from(o);
        true
    }

    /// The infinity cases of `Number::raise` (Number.cc:3841).
    ///
    /// `Some(r)`: `raise` is finished, return `r`. `None`: keep going with the
    /// rest of `raise_impl` — and possibly with a *modified* `self`. That last
    /// part is the reference's design, not an accident of translation: when the
    /// exponent is an interval reaching to infinity rather than an infinity
    /// itself, the reference decides the limit here (`0.5^(-infinity)` is
    /// `+infinity`), writes it into the base, and lets the ordinary float path
    /// take the power of *that* against the whole exponent interval. It is what
    /// makes `0.5^[-infinity:-1]` come out as `0`.
    ///
    /// An exponent that is not *known* non-zero is indeterminate against an
    /// infinite base, and a base that is not known non-zero is indeterminate
    /// under an infinite exponent — both `!isNonZero()` tests in the reference.
    /// They are why `infinity^0`, `0^infinity` and `0^(-infinity)` are left as
    /// powers rather than answered.
    fn raise_infinite(&mut self, o: &Number) -> Option<bool> {
        if self.is_infinite(false) {
            if o.is_negative() {
                self.clear(true);
                return Some(true);
            }
            if !o.is_nonzero() {
                return Some(false);
            }
            if self.is_minus_infinity() {
                if o.is_even() {
                    self.value = RealValue::PlusInfinity;
                } else if !o.is_integer() {
                    return Some(false);
                }
                // an odd integer keeps minus infinity
            }
            self.set_precision_and_approximate_from(o);
            return Some(true);
        }
        // An unbounded base needs an exponent that is definitely not zero:
        // `[1:+infinity]^0` is anything from 1 to an indeterminate form.
        if self.includes_infinity() && !o.is_nonzero() {
            return Some(false);
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if o.includes_minus_infinity() {
            // An exponent interval that reaches both infinities decides
            // nothing: `x^[-infinity:+infinity]` is `[0:+infinity]` at best.
            if o.is_floating_point() && o.includes_plus_infinity() {
                return Some(false);
            }
            if !self.is_nonzero() {
                return Some(false);
            } else if self.is_negative() {
                if !self.is_less_than(&mone) {
                    return Some(false);
                }
                if !o.is_floating_point() {
                    self.clear(true);
                }
            } else if self.is_greater_than(&one) {
                if !o.is_floating_point() {
                    self.clear(true);
                }
            } else if self.is_positive() && self.is_less_than(&one) {
                self.set_plus_infinity(true, true);
            } else {
                return Some(false);
            }
            if !o.is_floating_point() {
                self.set_precision_and_approximate_from(o);
                return Some(true);
            }
        } else if o.includes_plus_infinity() {
            if !self.is_nonzero() {
                return Some(false);
            } else if self.is_negative() {
                if !self.is_greater_than(&mone) {
                    return Some(false);
                }
                if !o.is_floating_point() {
                    self.clear(true);
                }
            } else if self.is_greater_than(&one) {
                self.set_plus_infinity(true, true);
            } else if self.is_positive() && self.is_less_than(&one) {
                if !o.is_floating_point() {
                    self.clear(true);
                }
            } else {
                return Some(false);
            }
            if !o.is_floating_point() {
                self.set_precision_and_approximate_from(o);
                return Some(true);
            }
        }
        None
    }

    fn raise_complex(&mut self, o: &Number) -> bool {
        // Integer exponents on complex base: repeated squaring, exact.
        if let Some(mut exp) = o.to_i64() {
            if exp != 0 && exp.unsigned_abs() <= 1_000_000 {
                let neg = exp < 0;
                exp = exp.abs();
                let base = self.clone();
                let mut acc = Number::from_i64(1);
                let mut sq = base;
                let mut e = exp as u64;
                while e > 0 {
                    if e & 1 == 1 && !acc.multiply(&sq) {
                        return false;
                    }
                    e >>= 1;
                    if e > 0 {
                        let s = sq.clone();
                        if !sq.multiply(&s) {
                            return false;
                        }
                    }
                }
                if neg && !acc.recip() {
                    return false;
                }
                *self = acc;
                self.set_precision_and_approximate_from(o);
                return true;
            }
        }
        // `exp(w·ln z)` has nothing to say about a base with an unbounded
        // component: `ln z` is unbounded, `w·ln z` spreads that infinity
        // across both parts, and `exp` of it is whatever the compositions
        // happen to produce. The reference answers such a base only for the
        // exact integer powers handled above (`([-0.5:0.5]+(infinity)i)^2` is
        // `-infinity+([-infinity:+infinity])i`) and refuses the rest.
        if self.includes_infinity() {
            return false;
        }
        // General complex power: z^w = exp(w * ln z), with ln and exp
        // handling their own complex cases. The principal branch is used,
        // matching the reference.
        if self.is_zero() {
            // 0^w is zero for a positive real exponent, undefined otherwise.
            if o.has_imaginary_part() || !o.real_part_is_positive() {
                return false;
            }
            self.clear(true);
            return true;
        }
        let mut l = self.clone();
        if !l.ln() {
            return false;
        }
        if !l.multiply(o) {
            return false;
        }
        if !l.exp() {
            return false;
        }
        *self = l;
        // `exp(w·ln z)` leaves rounding dust in whichever part should have
        // cancelled: `(-2)^0.5` came out `-2.2E-58 + 1.414213562i`. The
        // reference clears it in `testComplex` (Number.cc:2309), reached from
        // the `testFloatResult` at the end of every float operation, and the
        // test is *relative* — a part is dropped only when adding it to the
        // other part changes nothing at `BIT_PRECISION - 10` — so a genuinely
        // small-but-significant part survives.
        self.drop_negligible_parts();
        self.set_precision_and_approximate_from(o);
        true
    }

    /// Try to take an exact `n`-th root of a rational; self unchanged on
    /// failure. Only succeeds for perfect powers (non-negative base, or odd
    /// roots of negative bases).
    pub(crate) fn exact_root(&mut self, n: u32) -> bool {
        if n == 0 {
            return false;
        }
        if n == 1 {
            return true;
        }
        let RealValue::Rational(r) = &self.value else {
            return false;
        };
        let neg = r.is_negative();
        if neg && n % 2 == 0 {
            return false;
        }
        let Some(num_root) = nth_root_exact(&r.numer().abs(), n) else {
            return false;
        };
        let Some(den_root) = nth_root_exact(r.denom(), n) else {
            return false;
        };
        let mut result = BigRational::new(num_root, den_root);
        if neg {
            result = -result;
        }
        self.value = RealValue::Rational(result);
        return true;

        /// Integer exact n-th root or None.
        fn nth_root_exact(z: &BigInt, n: u32) -> Option<BigInt> {
            if z.is_zero() || z.is_one() {
                return Some(z.clone());
            }
            let root = z.nth_root(n);
            if root.pow(n) == *z {
                Some(root)
            } else {
                None
            }
        }
    }

    /// `sqrt()`.
    pub fn sqrt(&mut self) -> bool {
        // `sqrt(f)' = f'/(2·sqrt(f))` (MathStructure-differentiate.cc:182).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.sqrt() && d.multiply(&Number::from_i64(2)) && d.recip()).then_some(d)
                },
                Number::sqrt_impl,
            );
        }
        self.sqrt_impl()
    }

    fn sqrt_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // handled by raise(1/2) at MathStructure level
        }
        if self.is_minus_infinity() {
            return false;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.real_part_is_negative() {
            // sqrt of negative real → imaginary.
            if self.is_imag_part {
                return false;
            }
            let mut pos = self.clone();
            if !pos.negate() || !pos.sqrt() {
                return false;
            }
            let mut result = Number::new();
            result.set_imaginary_part(&pos);
            result.approx = self.approx || pos.approx;
            *self = result;
            return true;
        }
        // Exact square root of perfect-square rational.
        if let RealValue::Rational(_) = &self.value {
            let mut copy = self.clone();
            if copy.exact_root(2) {
                *self = copy;
                return true;
            }
        }
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            (au.sqrt(p, RoundingMode::Up), al.sqrt(p, RoundingMode::Down))
        } else {
            let f = al.sqrt(p, RoundingMode::ToEven);
            (f.clone(), f)
        };
        // sqrt is monotone increasing: lower comes from al.
        let (lower, upper) = if context::create_interval() {
            (al.sqrt(p, RoundingMode::Down), au.sqrt(p, RoundingMode::Up))
        } else {
            (lower, upper)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `isqrt()` — integer square root (floor).
    pub fn isqrt(&mut self) -> bool {
        let Some(z) = self.to_bigint() else {
            return false;
        };
        if z.is_negative() {
            return false;
        }
        let r = z.sqrt();
        self.value = RealValue::Rational(BigRational::from_integer(r));
        true
    }

    /// `isPerfectSquare()`.
    pub fn is_perfect_square(&self) -> bool {
        match self.to_bigint() {
            Some(z) if !z.is_negative() => {
                let r = z.sqrt();
                &(&r * &r) == z
            }
            _ => false,
        }
    }

    /// `cbrt()`.
    pub fn cbrt(&mut self) -> bool {
        self.root_i(3)
    }

    /// `root(o)` — real n-th root (odd roots of negatives are real).
    pub fn root(&mut self, o: &Number) -> bool {
        let Some(n) = o.to_i64() else { return false };
        if n <= 0 || n > u32::MAX as i64 {
            return false;
        }
        self.root_i(n as u32)
    }

    fn root_i(&mut self, n: u32) -> bool {
        // `root(f,a)' = f'·root(f,a)^(1−a)/a`
        // (MathStructure-differentiate.cc:191; `cbrt` is the a = 3 case, whose
        // own rule at :201 is the same expression written out).
        if self.unc.is_some() {
            return self.uncertain_unary(
                move |x| {
                    let mut d = x.clone();
                    let e = Number::from_i64(1 - i64::from(n));
                    (d.root_i(n) && d.raise(&e, true) && d.divide(&Number::from_i64(i64::from(n))))
                        .then_some(d)
                },
                move |s| s.root_i_impl(n),
            );
        }
        self.root_i_impl(n)
    }

    fn root_i_impl(&mut self, n: u32) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        if n == 0 {
            return false;
        }
        if n == 1 {
            return true;
        }
        // An even root needs the *whole* interval non-negative, not merely a
        // non-negative upper bound — `root([-1:1], 2)` has no real value at
        // -1. The reference tests exactly this (`o.isEven() && !isNonNegative()`,
        // Number.cc:4485). A finite straddling interval used to be caught
        // further down by `pow` returning NaN on the negative bound; an
        // infinite one is not, since `(-infinity)^0.5` is `+infinity` rather
        // than NaN, and `root([-infinity:1], 2)` came back as `[1:+infinity]`.
        if n % 2 == 0 && !self.is_non_negative() {
            return false;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            return n % 2 == 1;
        }
        // The exact rational root of a point value.
        if let RealValue::Rational(r) = &self.value {
            let neg = r.is_negative();
            let mut copy = self.clone();
            if neg && !copy.negate() {
                return false;
            }
            if copy.exact_root(n) {
                if neg {
                    copy.negate();
                }
                *self = copy;
                return true;
            }
        }
        // Float: each bound independently, with its own sign taken out and put
        // back. Taking the absolute value of the *interval* only means
        // anything when the interval sits on one side of zero, which is why
        // an odd root of `[-1:0.5]` used to be refused outright; the reference
        // negates each bound on its own (Number.cc:4537) and so gets
        // `[-1:0.7937]`.
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let inv_lo = bigfloat_from_ratio(&BigInt::one(), &BigInt::from(n), p, RoundingMode::Down);
        let inv_hi = bigfloat_from_ratio(&BigInt::one(), &BigInt::from(n), p, RoundingMode::Up);
        let inv_n = bigfloat_from_ratio(&BigInt::one(), &BigInt::from(n), p, RoundingMode::ToEven);
        // `down` asks for the outward end of this bound's own rounding: a
        // negative bound is negated afterwards, so its magnitude rounds the
        // other way.
        let root_bound = |b: &BigFloat, down: bool| -> BigFloat {
            if b.is_inf() || b.is_zero() {
                return b.clone();
            }
            let neg = matches!(b.sign(), Some(astro_float::Sign::Neg));
            let mag = if neg { b.neg() } else { b.clone() };
            let rm = if neg == down { RoundingMode::Up } else { RoundingMode::Down };
            let a = context::with_consts(|cc| mag.pow(&inv_lo, p, rm, cc));
            let c = context::with_consts(|cc| mag.pow(&inv_hi, p, rm, cc));
            let pick_up = rm == RoundingMode::Up;
            let m = if matches!(a.cmp(&c), Some(k) if (k > 0) == pick_up) { a } else { c };
            if neg {
                m.neg()
            } else {
                m
            }
        };
        let (lower, upper) = if context::create_interval() {
            (root_bound(&al, true), root_bound(&au, false))
        } else {
            let neg = matches!(al.sign(), Some(astro_float::Sign::Neg));
            let mag = if neg { al.neg() } else { al.clone() };
            let mut f = context::with_consts(|cc| mag.pow(&inv_n, p, RoundingMode::ToEven, cc));
            if neg {
                f = f.neg();
            }
            (f.clone(), f)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        let mut result = Number::new();
        result.value = RealValue::Float { lower, upper };
        result.approx = true;
        result.set_precision_and_approximate_from(self);
        *self = result;
        self.approx = true;
        self.test_float_result(true)
    }
}

/// `mpz_sizeinbase(z, 10)` — the decimal length GMP reports.
///
/// GMP derives it from the bit count rather than from the digits, so the
/// answer is the digit count or one more; this reproduces that, including
/// `sizeinbase(0) == 1`. Nothing here needs the exact digit count — it feeds
/// a size guard — and matching GMP keeps the guard's boundary where the
/// reference puts it.
fn decimal_size(z: &BigInt) -> u64 {
    let bits = z.magnitude().bits();
    (bits as f64 * std::f64::consts::LOG10_2) as u64 + 1
}

/// The reference's exact-power guard (`Number::raise`, Number.cc:4053).
///
/// `i_pow` is the exponent's numerator, `i_root` its denominator, and
/// `length1` the larger decimal length of the base's numerator and
/// denominator. The result of `base^i_pow` is about `i_pow * length1` digits
/// long, so all three limits matter and the *product* is the one that bites:
/// `123456789^200000` has an exponent well inside the limit but would produce
/// 1.6 million digits, and `(1/3)^1000000` sits exactly on the exponent
/// bound. The reference declines both and answers from the MPFR float path
/// instead, in a few hundredths of a second.
fn exact_power_in_range(base: &BigRational, i_pow: i64, i_root: u64, try_exact: bool) -> bool {
    let length1 = decimal_size(base.numer()).max(decimal_size(base.denom()));
    let (limit, root_ok) = if try_exact {
        (1_000_000i64, i_root < 1_000_000)
    } else {
        (1_000i64, i_root <= 3)
    };
    root_ok
        && i_pow < limit
        && i_pow > -limit
        && length1 < limit as u64
        && (i_pow.unsigned_abs()).saturating_mul(length1) < limit as u64
}

/// `a^b` for two float bounds, rounded in direction `rm`.
///
/// astro-float's `pow` refines until it can decide which way to round, and an
/// exactly representable result never settles that decision — `0.5^(-2)` is
/// exactly 4, so no amount of extra precision tells `pow` whether the true
/// value is above or below 4, and the refinement runs forever. `raise_impl`
/// keeps *point* operands away from it by taking integer exponents through
/// repeated squaring and `n/2^k` exponents through repeated `sqrt`, but the
/// interval branch evaluates its four corners on the raw bounds, where neither
/// applies: `[-2:2]` is not an integer and not `n/2^k`, only its end points
/// are. This is the same treatment one level down — an exactly representable
/// corner is computed *as a rational*, exactly, and only the irrational ones
/// reach `pow`.
///
/// A corner is exactly representable only when the exponent is dyadic (`n/2^k`
/// — every finite `BigFloat` exponent is) and the base is an exact `2^k`-th
/// power of a dyadic rational. That is precisely the condition
/// [`Number::exact_root`] decides, so the test *is* the computation: when it
/// succeeds the exact value replaces `pow`'s answer, and when it fails the
/// result is irrational, `pow`'s Ziv loop terminates, and nothing changes.
fn pow_bound(a: &BigFloat, b: &BigFloat, p: usize, rm: RoundingMode) -> BigFloat {
    if let Some(f) = infinite_power(a, b, p) {
        return f;
    }
    if let Some(f) = exact_representable_integer_power(a, b, p, rm) {
        return f;
    }
    if let Some(f) = exact_dyadic_power(a, b, p, rm) {
        return f;
    }
    context::with_consts(|cc| a.pow(b, p, rm, cc))
}

/// `a^n` for an exact integer `n`, when the result is exactly representable in
/// `p` bits — the one shape `exact_dyadic_power` below is forbidden to reach.
///
/// The reference never comes here: past the exact-rational size guard it falls
/// through to MPFR (Number.cc:4127, `setToFloatingPoint`), and `mpfr_pow` with
/// an integer exponent is `mpfr_pow_z` — binary exponentiation on floats, which
/// takes ~log2(n) steps and stops. astro-float's `pow` is `exp(n·ln a)` refined
/// until the rounding is decided, and a result that lands *on* a representable
/// value never decides it: `2^10000000` is a one-bit mantissa times `2^10000000`,
/// so no working precision ever separates the true value from the candidate and
/// the Ziv loop runs forever. That is what hung `2^10000000`, `2^-10000000`,
/// `(-2)^10000001` and every other power-of-two base past the size guard;
/// `3^10000000` was never affected, because its result is irrational-shaped in
/// binary and `pow` settles in under a millisecond.
///
/// (`2^-10000000` still does not answer, but no longer here: `print_float`
/// spells a float out by converting it to an exact rational, and a rational
/// with a ten-million-bit denominator goes through `print_rational_decimal`'s
/// digit-at-a-time long division three million times. That is a separate,
/// quadratic problem in the printer, and it is why `2^10000000` costs 1.9s
/// where the reference costs 0.10s.)
///
/// `exact_dyadic_power` catches the small exact cases by building the power as a
/// rational, but it is held to the reference's guard (`|n| < 1000000`, and
/// `|n|·length1 < 1000000`) so that it cannot materialise a million-digit
/// integer. This function is the other half: it decides exactness *without*
/// building anything. Write `a = M·2^E` with `M` odd. Then `a^n = M^n·2^(E·n)`,
/// and the result fits in `p` bits exactly when `M^n` does. Since
/// `bits(M^n) ≥ n·(bits(M)−1)+1`, an `M > 1` needs `n ≲ p` — so the only way a
/// *large* exponent is exact at all is `M = 1`, a power-of-two base, where the
/// answer is a shift of the exponent field. Everything else returns `None`,
/// falls through unchanged, and `pow`'s loop terminates on its own.
///
/// Unlike `exact_dyadic_power` this accepts a negative base: `pow` returns NaN
/// only for a *non-integer* exponent there, and `b` is known to be an integer.
///
/// Overflowing astro-float's exponent range answers infinity rather than `None`
/// — `None` would hand the case back to the loop that cannot finish. It is also
/// what MPFR does, and the caller's "an infinity the operands did not contain is
/// a pole" check then leaves the expression unevaluated, which is the
/// reference's answer for `2^1000000000000`.
fn exact_representable_integer_power(
    a: &BigFloat,
    b: &BigFloat,
    p: usize,
    rm: RoundingMode,
) -> Option<BigFloat> {
    if a.is_zero() || a.is_inf() || a.is_nan() || b.is_inf() || b.is_nan() {
        return None;
    }
    // `|b| < 2^exponent`, so anything past 64 cannot be an `i64` — and must be
    // rejected *before* it is spelled out as an integer, or a float with a
    // billion-bit exponent turns into a hundred-megabyte `BigInt` on the way to
    // failing `to_i64`.
    if b.exponent()? > 64 || !crate::float::bigfloat_is_integer(b) {
        return None;
    }
    let n = crate::float::bigfloat_to_bigint_trunc(b)?.to_i64()?;
    if n == 0 {
        return None;
    }
    // `a = mantissa × 2^(e − p_a)`, the raw form; strip the trailing zeros to
    // get the odd part and fold them into the exponent.
    let (words, _n, sign, e, _inexact) = a.as_raw_parts()?;
    let mantissa = biguint_from_words(words);
    if mantissa.is_zero() {
        return None;
    }
    let tz = mantissa.trailing_zeros().unwrap_or(0);
    let odd = &mantissa >> tz;
    let exp2 = i128::from(e) - (words.len() * astro_float::WORD_BIT_SIZE) as i128 + tz as i128;
    let bits = odd.bits();
    let magnitude = n.unsigned_abs();
    let odd_powed = if bits == 1 {
        // `a` is ±2^exp2: exact for either sign of `n`.
        BigUint::one()
    } else {
        // `1/M^|n|` is not dyadic for `M > 1`, so a negative exponent is out.
        if n < 0 {
            return None;
        }
        if (bits - 1).checked_mul(magnitude)? >= p as u64 {
            return None;
        }
        let q = odd.pow(u32::try_from(magnitude).ok()?);
        if q.bits() > p as u64 {
            return None;
        }
        q
    };
    let negative = sign == astro_float::Sign::Neg && n % 2 != 0;
    let mut m = BigInt::from(odd_powed);
    if negative {
        m = -m;
    }
    let mut f = crate::float::bigfloat_from_bigint(&m, p, rm);
    let shifted = i128::from(f.exponent()?) + exp2 * i128::from(n);
    if shifted > i128::from(MPFR_EXPONENT_MAX) {
        return Some(BigFloat::from_f64(
            if negative { f64::NEG_INFINITY } else { f64::INFINITY },
            p,
        ));
    }
    if shifted < -i128::from(MPFR_EXPONENT_MAX) {
        // Underflow is a refusal, not a zero: `testFloatResult` opens with
        // `if(mpfr_underflow_p()) return false` (Number.cc:2387), which is why
        // `2^-2000000000` prints back as `1 / 2^2000000000`. NaN is the signal
        // the caller already reads that way.
        return Some(BigFloat::from_f64(f64::NAN, p));
    }
    f.set_exponent(shifted as astro_float::Exponent);
    Some(f)
}

/// MPFR's default exponent range, `±(2^30 − 1)`, on the same `0.m × 2^e`
/// convention astro-float uses — the reference does not move it
/// (no `mpfr_set_emax`/`mpfr_set_emin` anywhere in libqalculate), so this is
/// where its answers stop: `2^1073741822` is `1.049289358E323228496` and
/// `2^1073741823` prints back unevaluated, and likewise `2^-1073741822`
/// against `2^-2000000000`. astro-float's own range is four times wider
/// (`i32::MAX`), so without this the port would answer in a band the
/// reference refuses — and answer it by spending minutes in the decimal
/// printer, which materialises the exact value.
const MPFR_EXPONENT_MAX: astro_float::Exponent = (1 << 30) - 1;

/// astro-float hands its mantissa out as little-endian 64-bit words.
fn biguint_from_words(words: &[astro_float::Word]) -> BigUint {
    let mut v = Vec::with_capacity(words.len() * 2);
    for w in words {
        v.push(*w as u32);
        v.push((*w >> 32) as u32);
    }
    BigUint::new(v)
}

/// `a^b` when one of them is infinite: the IEEE limits, which is what MPFR
/// gives the reference and what a half-infinite interval bound needs.
///
/// astro-float computes `pow` as `exp(b·ln a)` throughout and gets two of these
/// wrong — `2^(-infinity)` comes back as `-infinity` and `0.5^(-infinity)` as
/// `0`, both sign errors from `ln a` — which turned `2^[-infinity:-1]` into the
/// enclosure `[-infinity:0.5]`. Only a positive base is decided here; a
/// negative one keeps whatever astro-float makes of it, including the NaN that
/// stands for "no real value".
fn infinite_power(a: &BigFloat, b: &BigFloat, p: usize) -> Option<BigFloat> {
    if !a.is_inf() && !b.is_inf() {
        return None;
    }
    if a.is_nan() || b.is_nan() || matches!(a.sign(), Some(astro_float::Sign::Neg)) && !a.is_zero() {
        return None;
    }
    let zero = BigFloat::from_i8(0, p);
    let one = BigFloat::from_i8(1, p);
    let inf = BigFloat::from_f64(f64::INFINITY, p);
    // `1^anything` is 1, infinite exponent included.
    if !a.is_inf() && matches!(a.cmp(&one), Some(0)) {
        return Some(one);
    }
    if b.is_inf() {
        let big = a.is_inf() || matches!(a.cmp(&one), Some(c) if c > 0);
        return Some(if big == b.is_inf_pos() { inf } else { zero });
    }
    // `a` is infinite and `b` is finite.
    if b.is_zero() {
        return Some(one);
    }
    Some(if matches!(b.sign(), Some(astro_float::Sign::Pos)) { inf } else { zero })
}

/// `a^b` when it is exactly a rational, rounded to `p` bits in direction `rm`.
/// `None` when the result is irrational, when either operand is not finite, or
/// when the exact form would be too large to be worth building — all cases
/// astro-float's `pow` handles by itself.
fn exact_dyadic_power(a: &BigFloat, b: &BigFloat, p: usize, rm: RoundingMode) -> Option<BigFloat> {
    // Positive bases only. Zero and infinity are astro-float's to answer, and
    // a *negative* base must stay its answer too: `pow` returns NaN there, and
    // the NaN is load-bearing — it is what makes `[-0.5:0]^[-2:-1]` fail
    // rather than quietly report the enclosure of the even-exponent corners.
    if !matches!(a.sign(), Some(astro_float::Sign::Pos)) || a.is_zero() || a.is_inf() {
        return None;
    }
    let (bn, bd) = bigfloat_to_ratio(b)?;
    let e = BigRational::new(bn, bd);
    // Every finite `BigFloat` is `n/2^k`, so the reduced denominator is a power
    // of two: the root is either exact or impossible, and the numerator is the
    // integer power that follows it.
    let root = e.denom().to_u32().filter(|r| *r <= (1 << 20))?;
    let i_pow = e.numer().to_i64()?;
    let (an, ad) = bigfloat_to_ratio(a)?;
    let base = BigRational::new(an, ad);
    if !exact_power_in_range(&base, i_pow, u64::from(root), true) {
        return None;
    }
    let mut rooted = Number::from_rational(base);
    if !rooted.exact_root(root) {
        return None;
    }
    let RealValue::Rational(r) = &rooted.value else {
        return None;
    };
    if i_pow < 0 && r.is_zero() {
        return None;
    }
    // `exact_power_in_range` has already bounded |i_pow| by 1 000 000.
    let r = r.pow(i_pow as i32);
    Some(bigfloat_from_ratio(r.numer(), r.denom(), p, rm))
}

/// The interval hull of a set of real numbers/intervals:
/// `[min lower bound, max upper bound]`.
fn hull_of(parts: &[Number]) -> Option<Number> {
    let mut lo = parts.first()?.lower_end_point();
    let mut hi = parts.first()?.upper_end_point();
    for p in &parts[1..] {
        let l = p.lower_end_point();
        let u = p.upper_end_point();
        if l.is_less_than(&lo) {
            lo = l;
        }
        if u.is_greater_than(&hi) {
            hi = u;
        }
    }
    if lo.equals(&hi, false, false) {
        return Some(lo);
    }
    let mut n = Number::new();
    n.set_interval(&lo, &hi, false).then_some(n)
}

#[cfg(test)]
mod uncertainty_tests {
    use crate::number::uncertainty_test_support::{plus_minus, uncertain};

    #[test]
    fn sqrt_of_four() {
        // Reference: `sqrt(4+/-0.5)` = `2.00±0.13` — 0.5/(2·2).
        let mut n = uncertain("4", "0.5");
        assert!(n.sqrt());
        assert_eq!(plus_minus(&n), "2.00±0.13");
    }

    #[test]
    fn sqrt_of_nine() {
        // Reference: `sqrt(9+/-1)` = `3.00±0.17` — 1/(2·3).
        let mut n = uncertain("9", "1");
        assert!(n.sqrt());
        assert_eq!(plus_minus(&n), "3.00±0.17");
    }

    #[test]
    fn nested_sqrt_chains_the_derivatives() {
        // Reference: `sqrt(sqrt(16+/-1))` = `2.000±0.031` — 1/8 then /(2·2).
        let mut n = uncertain("16", "1");
        assert!(n.sqrt() && n.sqrt());
        assert_eq!(plus_minus(&n), "2.000±0.031");
    }

    #[test]
    fn cbrt_of_eight() {
        // Reference: `cbrt(8+/-0.6)` = `2.000±0.050` — 0.6/(3·2²).
        let mut n = uncertain("8", "0.6");
        assert!(n.cbrt());
        assert_eq!(plus_minus(&n), "2.000±0.050");
    }

    #[test]
    fn cube_root_of_eight() {
        // Reference: `root(8+/-0.6,3)` = `2.000±0.050`, same as `cbrt`.
        let mut n = uncertain("8", "0.6");
        assert!(n.root(&crate::Number::from_i64(3)));
        assert_eq!(plus_minus(&n), "2.000±0.050");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// astro-float's `pow` cannot decide the rounding of an exactly
    /// representable result, so a binary exponent must not reach it.
    /// Every value here is the reference binary's.
    fn raised(base: &str, exp: (i64, i64)) -> String {
        let po = crate::options::PrintOptions::default();
        let mut n = Number::parse(base, &crate::options::ParseOptions::default());
        assert!(n.raise(&Number::from_ints(exp.0, exp.1, 0), false));
        n.print(&po)
    }

    #[test]
    fn binary_exponents_with_exact_results_terminate() {
        assert_eq!(raised("4", (1, 2)), "2");
        assert_eq!(raised("4", (3, 2)), "8");
        assert_eq!(raised("16", (1, 4)), "2");
        assert_eq!(raised("0.25", (1, 2)), "0.5");
    }

    /// The interval branch has the same trap as the point branch one level
    /// down: `0.5^[-2:2]` evaluates the corner `0.5^(-2)`, which is exactly 4,
    /// and astro-float's `pow` never settles the rounding of an exactly
    /// representable result. This used to run forever. Every value here is the
    /// reference's, via the C++ shim of `tests/interval_closure.rs`.
    #[test]
    fn interval_exponents_with_exactly_representable_corners_terminate() {
        let po = crate::options::PrintOptions::default();
        let raised = |base: Number, lo: i64, hi: i64| {
            let mut e = Number::new();
            assert!(e.set_interval(&Number::from_i64(lo), &Number::from_i64(hi), false));
            let mut n = base;
            assert!(n.raise(&e, true));
            format!(
                "[{}:{}]",
                n.lower_end_point().print(&po),
                n.upper_end_point().print(&po)
            )
        };
        assert_eq!(raised(Number::from_ints(1, 2, 0), -2, 2), "[0.25:4]");
        assert_eq!(raised(Number::from_ints(1, 2, 0), 0, 2), "[0.25:1]");
        assert_eq!(raised(Number::from_i64(2), -2, 2), "[0.25:4]");
        assert_eq!(raised(Number::from_i64(2), 2, 3), "[4:8]");
    }

    /// astro-float computes `pow` as `exp(b·ln a)` and loses the sign of
    /// `ln a` at an infinite exponent: `2^(-infinity)` came back as
    /// `-infinity`, which made `2^[-infinity:-1]` the enclosure
    /// `[-infinity:0.5]`. The reference (and MPFR) give `[0:0.5]`.
    #[test]
    fn infinite_exponents_take_the_ieee_limit() {
        let po = crate::options::PrintOptions::default();
        let mut minf = Number::new();
        minf.set_minus_infinity(false, false);
        let mut e = Number::new();
        assert!(e.set_interval(&minf, &Number::from_i64(-1), false));
        let mut n = Number::from_i64(2);
        assert!(n.raise(&e, true));
        assert_eq!(
            format!(
                "[{}:{}]",
                n.lower_end_point().print(&po),
                n.upper_end_point().print(&po)
            ),
            "[0:0.5]"
        );
    }

    /// Raise an integer base to an integer exponent the way the evaluator
    /// does, and print the result at the default precision.
    fn raise_int(base: i64, exp: i64) -> Number {
        let mut n = Number::from_i64(base);
        assert!(n.raise(&Number::from_i64(exp), true), "{base}^{exp} failed");
        n
    }

    /// `show_ending_zeroes` so the strings are the reference CLI's verbatim.
    fn shown(base: i64, exp: i64) -> String {
        let mut po = crate::options::PrintOptions::default();
        po.show_ending_zeroes = true;
        raise_int(base, exp).print(&po)
    }

    /// `2^10000000` ran forever. Past the reference's exact-power size guard
    /// the port had nothing left but astro-float's `pow`, which is
    /// `exp(n·ln a)` refined until the rounding is decided — and `2^10000000`
    /// is a one-bit mantissa times `2^10000000`, an exactly representable
    /// result, which never decides it. The reference answers from
    /// `mpfr_pow_z`, binary exponentiation on floats, in 0.10s.
    ///
    /// The value is pinned against the exact integer rather than against the
    /// reference's `9.049817306E3010299`, which says the same thing about ten
    /// significant digits instead of ten million: spelling the answer out in
    /// decimal costs seconds (see `print_float`, which goes through the exact
    /// rational), and the shift is both cheaper and stricter.
    #[test]
    fn a_power_of_two_base_past_the_size_guard_terminates() {
        let n = raise_int(2, 10000000);
        assert!(n.is_approximate(), "the size guard declines the exact path");
        let exact = Number::from_bigint(BigInt::from(1) << 10000000u32);
        assert!(n.equals(&exact, false, false), "2^10000000 is not 2^10000000");
    }

    /// Either side of the exact-power size guard, which is the threshold that
    /// decides whether the answer is built as a rational or as a float.
    ///
    /// `|i_pow|·length1 < 1000000` (Number.cc:4053) with `length1 = 1` for
    /// base 2, so `2^999999` is the last exponent the exact-rational path
    /// takes and `2^1000000` the first it declines — and the declined side is
    /// where the hang was. Both values are the reference binary's, and so is
    /// the split between an exact and an approximate answer.
    #[test]
    fn the_exact_power_size_guard_hands_over_without_a_gap() {
        assert_eq!(shown(2, 999999), "4.950328115E301029");
        assert_eq!(shown(2, 1000000), "9.900656229E301029");
        assert!(!raise_int(2, 999999).is_approximate());
        assert!(raise_int(2, 1000000).is_approximate());
    }

    /// The other side of the new branch's own threshold: it fires only when
    /// the result is exactly representable, which for a large exponent means
    /// an odd mantissa of 1. `3^1000000` has the same exponent and the same
    /// declined size guard but an irrational-shaped result, so it stays on
    /// `pow` — where it always terminated, in under a millisecond. Negative
    /// bases take the branch too; `pow` returns NaN there only for a
    /// non-integer exponent. Both values are the reference binary's.
    #[test]
    fn only_exactly_representable_results_leave_the_pow_path() {
        assert_eq!(shown(3, 1000000), "1.797710117E477121");
        assert_eq!(shown(-2, 1000001), "-1.980131246E301030");
        assert_eq!(shown(-3, 1000001), "-5.393130350E477121");
    }

    /// Either side of MPFR's default exponent range, which is where the
    /// reference's answers stop: `2^1073741822` has exponent `2^30 − 1` and is
    /// answered, `2^1073741823` needs one more and prints back unevaluated.
    /// Overflow answers infinity rather than `None` so that the case is never
    /// handed back to the loop that cannot finish; the caller's pole check
    /// turns that into the refusal.
    ///
    /// The accepted side is only checked for *acceptance*: printing a
    /// 323-million-digit value is minutes of work in this port's decimal
    /// printer.
    #[test]
    fn the_exponent_range_ends_where_mpfrs_does() {
        let mut n = Number::from_i64(2);
        assert!(n.raise(&Number::from_i64(1_073_741_822), true));
        let mut n = Number::from_i64(2);
        assert!(!n.raise(&Number::from_i64(1_073_741_823), true));
        let mut n = Number::from_i64(2);
        assert!(!n.raise(&Number::from_i64(-2_000_000_000), true));
        let mut n = Number::from_i64(2);
        assert!(!n.raise(&Number::from_i64(1_000_000_000_000), true));
    }

    #[test]
    fn binary_exponents_without_exact_results_are_unchanged() {
        assert_eq!(raised("2", (1, 2)), "1.414213562");
        assert_eq!(raised("2", (3, 2)), "2.828427125");
        assert_eq!(raised("5", (1, 2)), "2.236067977");
    }


    #[test]
    fn interval_base_straddling_zero_goes_complex() {
        // Oracle: `(2+/-3)^3.2` under `/set ic 2` is `86±87 - 0.29±0.30i`.
        // The base interval [-1, 5] contains zero, so `exp(b·ln a)` has no
        // value on it and the end points have to be hulled with zero.
        let po = crate::options::ParseOptions::default();
        let mut n = Number::new();
        assert!(n.set_interval(&Number::from_i64(-1), &Number::from_i64(5), false));
        assert!(n.raise(&Number::parse("3.2", &po), true));
        let mut pm = crate::options::PrintOptions::default();
        pm.interval_display = crate::options::IntervalDisplay::PlusMinus;
        pm.spacious = true;
        assert_eq!(n.print(&pm), "86±87 - 0.29±0.30i");
    }

    #[test]
    fn variance_uncertainty_scales_by_the_derivative() {
        // Oracle: `(2+/-3)^3.2` = `9.18958684±44.11001683`; the uncertainty is
        // |f'(2)|·3 = 3.2·2^2.2·3, not the original 3.
        let po = crate::options::ParseOptions::default();
        let mut n = Number::from_i64(2);
        n.add_variance_uncertainty(&Number::from_i64(3));
        assert!(n.raise(&Number::parse("3.2", &po), true));
        let mut pm = crate::options::PrintOptions::default();
        pm.interval_display = crate::options::IntervalDisplay::PlusMinus;
        assert_eq!(n.print(&pm), "9.18958684±44.11001683");
    }
    use super::*;

    #[test]
    fn integer_powers_exact() {
        let mut n = Number::from_i64(2);
        assert!(n.raise(&Number::from_i64(10), true));
        assert_eq!(n.to_i64(), Some(1024));
        let mut m = Number::from_ints(2, 3, 0);
        assert!(m.raise(&Number::from_i64(-2), true));
        assert!(m.internal_rational().unwrap() == &BigRational::new(9.into(), 4.into()));
    }

    #[test]
    fn perfect_roots_exact() {
        let mut n = Number::from_i64(16);
        assert!(n.raise(&Number::from_ints(1, 2, 0), true));
        assert_eq!(n.to_i64(), Some(4), "16^(1/2) = 4 exact");
        let mut c = Number::from_i64(27);
        assert!(c.cbrt());
        assert_eq!(c.to_i64(), Some(3), "cbrt(27) = 3 exact");
        let mut f = Number::from_ints(4, 9, 0);
        assert!(f.sqrt());
        assert!(f.internal_rational().unwrap() == &BigRational::new(2.into(), 3.into()));
    }

    /// An even root of a negative real is *purely* imaginary — no ~1e-58
    /// rounding dust in the real part. Every value here is
    /// `printf 'EXPR\n' | qalc -t +u8`'s.
    #[test]
    fn even_roots_of_negatives_have_no_real_part() {
        let po = crate::options::PrintOptions::default();
        let pow = |base: &str, num: i64, den: i64| {
            let mut n = Number::parse(base, &crate::options::ParseOptions::default());
            assert!(n.raise(&Number::from_ints(num, den, 0), true));
            n.print(&po)
        };
        // The exact cases: perfect squares stay exact and rational.
        assert_eq!(pow("-4", 1, 2), "2i");
        assert_eq!(pow("-9", 1, 2), "3i");
        assert_eq!(pow("-1", 1, 2), "i");
        assert_eq!(pow("-1", 2, 4), "i");
        assert_eq!(pow("-4", 3, 2), "-8i");
        assert_eq!(pow("-4", -1, 2), "-0.5i");
        assert_eq!(pow("-1", 3, 2), "-i");
        assert_eq!(pow("-1", -1, 2), "-i");
        // The inexact ones go through `exp(w·ln z)`, where the real part has
        // to be dropped as negligible rather than printed as dust.
        assert_eq!(pow("-2", 1, 2), "1.414213562i");
        assert_eq!(pow("-4.5", 1, 2), "2.121320344i");
        // Genuine two-part results are untouched.
        assert_eq!(pow("-2", 1, 4), "0.8408964153 + 0.8408964153i");
    }

    /// `sqrt(-4)` is `2i`, so `sqrt(-1)^2` is exactly `-1` — dust in the
    /// real part used to propagate as `-1.000000000 - 3.1E-58i`.
    #[test]
    fn imaginary_roots_multiply_back_exactly() {
        let po = crate::options::PrintOptions::default();
        let half = Number::from_ints(1, 2, 0);
        let mut a = Number::from_i64(-1);
        assert!(a.raise(&half, true));
        let b = a.clone();
        assert!(a.multiply(&b));
        assert_eq!(a.print(&po), "-1");
    }

    #[test]
    fn sqrt_negative_gives_imaginary() {
        let mut n = Number::from_i64(-4);
        assert!(n.sqrt());
        assert!(n.is_complex());
        assert!(n.imaginary_part().to_i64() == Some(2), "sqrt(-4) = 2i");
    }

    #[test]
    fn sqrt2_is_interval() {
        let mut n = Number::from_i64(2);
        assert!(n.sqrt());
        assert!(n.is_floating_point() && n.is_approximate());
        // 1.414... between 1.4 and 1.5
        assert!(n.is_greater_than(&Number::from_ints(14, 10, 0)));
        assert!(n.is_less_than(&Number::from_ints(15, 10, 0)));
    }

    #[test]
    fn general_complex_power() {
        // i^i = e^(-pi/2) = 0.2078795764, a real value.
        let mut z = Number::new();
        z.set_imaginary_part(&Number::from_i64(1));
        let w = z.clone();
        assert!(z.raise(&w, true));
        assert!(z.is_greater_than(&Number::from_ints(2078, 10000, 0)));
        assert!(z.is_less_than(&Number::from_ints(2079, 10000, 0)));

        // A real base with a complex exponent stays finite.
        let mut b = Number::from_i64(2);
        let mut e = Number::from_i64(1);
        e.set_imaginary_part(&Number::from_i64(1));
        assert!(b.raise(&e, true));
        assert!(b.is_complex());
    }

    #[test]
    fn negative_base_fractional_exponent_is_complex() {
        // (-8)^(1/3) takes the principal root, which is complex.
        let mut n = Number::from_i64(-8);
        assert!(n.raise(&Number::from_ints(1, 3, 0), true));
        assert!(n.is_complex(), "principal cube root of -8 is complex");
    }

    #[test]
    fn complex_integer_power() {
        // (1+i)^4 = -4
        let mut z = Number::from_i64(1);
        z.set_imaginary_part(&Number::from_i64(1));
        assert!(z.raise(&Number::from_i64(4), true));
        assert_eq!(z.to_i64(), Some(-4));
        assert!(!z.is_complex());
    }

    #[test]
    fn isqrt_and_perfect_square() {
        let mut n = Number::from_i64(17);
        assert!(n.isqrt());
        assert_eq!(n.to_i64(), Some(4));
        assert!(Number::from_i64(49).is_perfect_square());
        assert!(!Number::from_i64(50).is_perfect_square());
    }

    #[test]
    fn infinity_powers() {
        let mut inf = Number::new();
        inf.set_plus_infinity(false, false);
        let mut x = inf.clone();
        assert!(x.raise(&Number::from_i64(-1), true));
        assert!(x.is_zero(), "inf^-1 = 0");
        let mut y = Number::from_i64(2);
        assert!(y.raise(&inf, true));
        assert!(y.is_plus_infinity(), "2^inf = inf");
        let mut h = Number::from_ints(1, 2, 0);
        assert!(h.raise(&inf, true));
        assert!(h.is_zero(), "(1/2)^inf = 0");
    }
}
