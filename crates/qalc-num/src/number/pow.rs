//! Powers and roots — port of `Number::raise`, `sqrt`, `isqrt`, `root`
//! (exact paths + float-interval fallback).

use super::{Number, RealValue};
use crate::context;
use crate::float::bigfloat_from_ratio;
use astro_float::RoundingMode;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

impl Number {
    /// `raise(o, try_exact)`: self = self^o.
    pub fn raise(&mut self, o: &Number, try_exact: bool) -> bool {
        // Handle x^0 and x^1 quickly.
        if o.is_zero() {
            if self.is_zero() {
                return false; // 0^0 handled upstream (evaluates to 1 in qalc via function, but Number fails)
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
        // Infinite base/exponent.
        if self.is_infinite(true) || o.is_infinite(true) {
            return self.raise_infinite(o);
        }
        // Exact integer exponent on rational base.
        if let (RealValue::Rational(r), Some(exp)) = (&self.value, o.to_i64()) {
            if try_exact && exp.unsigned_abs() <= 1_000_000 {
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
        if let (RealValue::Rational(_), RealValue::Rational(oe)) = (&self.value, &o.value) {
            if try_exact && !oe.denom().is_one() {
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
        // Float fallback: a^b = exp(b·ln(a)) computed by astro-float pow with
        // directed rounding on interval corners.
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (bl, bu) = (o.lower_bound_float(p), o.upper_bound_float(p));
        if matches!(al.sign(), Some(astro_float::Sign::Neg)) && !al.is_zero() {
            // Negative base with (effectively) integer float exponent only.
            if !o.is_integer() {
                return self.raise_complex(o);
            }
        }
        let (lower, upper) = if context::create_interval() {
            let c1 = context::with_consts(|cc| al.pow(&bl, p, RoundingMode::Down, cc));
            let c2 = context::with_consts(|cc| al.pow(&bu, p, RoundingMode::Down, cc));
            let c3 = context::with_consts(|cc| au.pow(&bl, p, RoundingMode::Down, cc));
            let c4 = context::with_consts(|cc| au.pow(&bu, p, RoundingMode::Down, cc));
            let d1 = context::with_consts(|cc| al.pow(&bl, p, RoundingMode::Up, cc));
            let d2 = context::with_consts(|cc| al.pow(&bu, p, RoundingMode::Up, cc));
            let d3 = context::with_consts(|cc| au.pow(&bl, p, RoundingMode::Up, cc));
            let d4 = context::with_consts(|cc| au.pow(&bu, p, RoundingMode::Up, cc));
            let mut lo = c1.clone();
            for c in [&c2, &c3, &c4] {
                if c.cmp(&lo) == Some(-1) || lo.is_nan() {
                    lo = (*c).clone();
                }
            }
            let mut hi = d1.clone();
            for c in [&d2, &d3, &d4] {
                if c.cmp(&hi) == Some(1) || hi.is_nan() {
                    hi = (*c).clone();
                }
            }
            (lo, hi)
        } else {
            let f = context::with_consts(|cc| al.pow(&bl, p, RoundingMode::ToEven, cc));
            (f.clone(), f)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.set_precision_and_approximate_from(o);
        self.test_float_result(true)
    }

    fn raise_infinite(&mut self, o: &Number) -> bool {
        // Port of the infinity cases of Number::raise.
        if self.is_plus_infinity() {
            if o.real_part_is_negative() {
                self.clear(true);
                return true;
            }
            if o.real_part_is_positive() {
                return true;
            }
            return false;
        }
        if self.is_minus_infinity() {
            if o.real_part_is_negative() {
                self.clear(true);
                return true;
            }
            if o.is_integer() {
                if o.is_even() {
                    self.value = RealValue::PlusInfinity;
                } // odd keeps minus infinity
                return true;
            }
            return false;
        }
        if o.is_plus_infinity() {
            // x^∞: |x|>1 → ∞; |x|<1 → 0; else undefined.
            let one = Number::from_i64(1);
            let mut a = self.clone();
            if !a.abs() {
                return false;
            }
            if a.is_greater_than(&one) {
                if self.real_part_is_negative() {
                    return false; // oscillates in sign
                }
                self.set_plus_infinity(true, false);
                return true;
            }
            if a.is_less_than(&one) {
                self.clear(true);
                return true;
            }
            return false;
        }
        if o.is_minus_infinity() {
            let one = Number::from_i64(1);
            let mut a = self.clone();
            if !a.abs() {
                return false;
            }
            if a.is_greater_than(&one) {
                self.clear(true);
                return true;
            }
            return false;
        }
        false
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
        // General complex power — needs ln/exp; wired up once the
        // transcendental module lands.
        false
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
        if self.has_imaginary_part() {
            return false;
        }
        if n == 0 {
            return false;
        }
        if n == 1 {
            return true;
        }
        if self.real_part_is_negative() && n % 2 == 0 {
            return false;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            return n % 2 == 1;
        }
        let neg = self.real_part_is_negative();
        let mut abs_self = self.clone();
        if neg && !abs_self.negate() {
            return false;
        }
        if let RealValue::Rational(_) = &abs_self.value {
            let mut copy = abs_self.clone();
            if copy.exact_root(n) {
                if neg {
                    copy.negate();
                }
                *self = copy;
                return true;
            }
        }
        // Float: x^(1/n) via pow with rational exponent 1/n.
        let p = context::bit_precision();
        let inv_n = bigfloat_from_ratio(&BigInt::one(), &BigInt::from(n), p, RoundingMode::ToEven);
        let (al, au) = (abs_self.lower_bound_float(p), abs_self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            // Use slightly widened exponent rounding to keep enclosure sound:
            let inv_lo = bigfloat_from_ratio(&BigInt::one(), &BigInt::from(n), p, RoundingMode::Down);
            let inv_hi = bigfloat_from_ratio(&BigInt::one(), &BigInt::from(n), p, RoundingMode::Up);
            let lo = context::with_consts(|cc| al.pow(&inv_lo, p, RoundingMode::Down, cc));
            let lo2 = context::with_consts(|cc| al.pow(&inv_hi, p, RoundingMode::Down, cc));
            let hi = context::with_consts(|cc| au.pow(&inv_lo, p, RoundingMode::Up, cc));
            let hi2 = context::with_consts(|cc| au.pow(&inv_hi, p, RoundingMode::Up, cc));
            let lo = if lo2.cmp(&lo) == Some(-1) { lo2 } else { lo };
            let hi = if hi2.cmp(&hi) == Some(1) { hi2 } else { hi };
            (lo, hi)
        } else {
            let f = context::with_consts(|cc| al.pow(&inv_n, p, RoundingMode::ToEven, cc));
            (f.clone(), f)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        let mut result = Number::new();
        result.value = RealValue::Float { lower, upper };
        result.approx = true;
        if neg {
            result.negate();
        }
        result.set_precision_and_approximate_from(self);
        *self = result;
        self.approx = true;
        self.test_float_result(true)
    }
}

#[cfg(test)]
mod tests {
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
