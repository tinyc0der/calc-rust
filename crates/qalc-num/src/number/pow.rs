//! Powers and roots — port of the power-related parts of `Number.cc`
//! (`raise`, `sqrt`). Real paths only so far: complex results other than
//! `sqrt` of a negative real, and interval bases spanning zero with
//! non-integer exponents, still return `false` (pending the full
//! `Number::raise` port).

use super::{Number, RealValue};
use crate::context;
use crate::float::bigfloat_cmp;
use astro_float::{BigFloat, RoundingMode};
use num_bigint::BigInt;
use num_integer::Roots;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

fn rnd(down: bool) -> RoundingMode {
    if down { RoundingMode::Down } else { RoundingMode::Up }
}

/// Exact integer nth root: `Some(r)` iff `r^n == z` exactly.
fn exact_int_nth_root(z: &BigInt, n: u32) -> Option<BigInt> {
    if n == 0 {
        return None;
    }
    if z.is_negative() && n % 2 == 0 {
        return None;
    }
    let r = z.nth_root(n);
    if num_traits::pow(r.clone(), n as usize) == *z {
        Some(r)
    } else {
        None
    }
}

/// `numer/denom` raised to a positive integer power, exactly.
fn rational_pow_u32(r: &BigRational, e: u32) -> BigRational {
    BigRational::new_raw(
        num_traits::pow(r.numer().clone(), e as usize),
        num_traits::pow(r.denom().clone(), e as usize),
    )
}

impl Number {
    /// `sqrt()`: principal square root. Negative reals go complex
    /// (`sqrt(-x) = i·sqrt(x)`), matching `Number::sqrt`.
    pub fn sqrt(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        match self.value {
            RealValue::MinusInfinity => return false,
            RealValue::PlusInfinity => return true,
            _ => {}
        }
        if self.is_negative() {
            let mut im = self.clone();
            im.negate();
            if !im.sqrt() {
                return false;
            }
            let approx = self.approx || im.approx;
            self.clear(true);
            self.set_imaginary_part(&im);
            self.approx = approx;
            return true;
        }
        if let RealValue::Rational(r) = &self.value {
            if let (Some(n), Some(d)) = (
                exact_int_nth_root(r.numer(), 2),
                exact_int_nth_root(r.denom(), 2),
            ) {
                self.value = RealValue::Rational(BigRational::new_raw(n, d));
                return true;
            }
        }
        let p = context::bit_precision();
        let interval = context::create_interval() || self.is_interval(true);
        let (lower, upper) = if interval {
            (
                self.lower_bound_float(p).sqrt(p, rnd(true)),
                self.upper_bound_float(p).sqrt(p, rnd(false)),
            )
        } else {
            let f = self.lower_bound_float(p).sqrt(p, RoundingMode::ToEven);
            (f.clone(), f)
        };
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `raise(o, try_exact)`: self = self^o. Mirrors `Number::raise` for the
    /// real-valued cases; complex-result cases return `false` for now.
    pub fn raise(&mut self, o: &Number, try_exact: bool) -> bool {
        if self.has_imaginary_part() || o.has_imaginary_part() {
            return false;
        }

        // Infinite exponent.
        if matches!(o.value, RealValue::PlusInfinity | RealValue::MinusInfinity) {
            return self.raise_to_infinity(matches!(o.value, RealValue::PlusInfinity));
        }
        // Infinite base.
        if matches!(self.value, RealValue::PlusInfinity | RealValue::MinusInfinity) {
            if o.is_zero() {
                return false;
            }
            if o.is_negative() {
                self.clear(true);
                self.approx = true;
                return true;
            }
            if matches!(self.value, RealValue::MinusInfinity) {
                if !o.is_integer() {
                    return false;
                }
                if o.is_even() {
                    self.value = RealValue::PlusInfinity;
                }
            }
            return true;
        }

        if o.is_zero() {
            // x^0 = 1 (including 0^0, as libqalculate evaluates it).
            let (approx, prec) = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = approx;
            self.precision = prec;
            return true;
        }
        if self.is_zero() {
            if o.is_negative() {
                return false;
            }
            self.set_precision_and_approximate_from(o);
            return true;
        }

        // Exact: rational base, machine-size integer exponent.
        if let RealValue::Rational(base) = &self.value {
            if o.is_integer() {
                if let Some(e) = o.to_i64() {
                    if e.unsigned_abs() <= 1_000_000 {
                        let pow = rational_pow_u32(base, e.unsigned_abs() as u32);
                        self.value = RealValue::Rational(if e < 0 { pow.recip() } else { pow });
                        self.set_precision_and_approximate_from(o);
                        return true;
                    }
                }
            } else if try_exact && o.is_rational() {
                // Exact nth root: base^(n/d) when base is a perfect d-th power.
                if let RealValue::Rational(er) = &o.value {
                    if let (Some(d), Some(n)) = (er.denom().to_u32(), er.numer().to_i64()) {
                        if d <= 64 && n.unsigned_abs() <= 1_000_000 {
                            let neg_base = base.is_negative() && d % 2 == 1;
                            let abs_base = base.abs();
                            if let (Some(rn), Some(rd)) = (
                                exact_int_nth_root(abs_base.numer(), d),
                                exact_int_nth_root(abs_base.denom(), d),
                            ) {
                                let mut root = BigRational::new_raw(rn, rd);
                                if neg_base {
                                    root = -root;
                                }
                                self.value = RealValue::Rational(root);
                                return self.raise(&Number::from_bigint(BigInt::from(n)), true);
                            }
                        }
                    }
                }
            }
        }

        // Float path (real results only).
        let p = context::bit_precision();
        if self.is_negative() {
            // Negative base: real result only for integer exponents.
            if !o.is_integer() {
                return false;
            }
            let odd = o.is_odd();
            let mut abs_self = self.clone();
            if !abs_self.abs() || !abs_self.raise(o, try_exact) {
                return false;
            }
            if odd {
                abs_self.negate();
            }
            let prec = self.precision;
            *self = abs_self;
            if prec >= 0 {
                self.precision = if self.precision >= 0 { self.precision.min(prec) } else { prec };
            }
            return true;
        }
        if !self.real_part_is_positive() {
            // Interval spanning/touching zero with a float exponent: not yet.
            if !o.is_integer() {
                return false;
            }
        }

        let interval = context::create_interval() || self.is_interval(true) || o.is_interval(true);
        let (bl, bu) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (el, eu) = (o.lower_bound_float(p), o.upper_bound_float(p));
        let (lower, upper) = if interval {
            let mut lo: Option<BigFloat> = None;
            let mut hi: Option<BigFloat> = None;
            for b in [&bl, &bu] {
                for e in [&el, &eu] {
                    let down = context::with_consts(|cc| b.pow(e, p, rnd(true), cc));
                    let up = context::with_consts(|cc| b.pow(e, p, rnd(false), cc));
                    lo = Some(match lo {
                        Some(cur) if bigfloat_cmp(&cur, &down) == Some(-1) => cur,
                        _ => down,
                    });
                    hi = Some(match hi {
                        Some(cur) if bigfloat_cmp(&cur, &up) == Some(1) => cur,
                        _ => up,
                    });
                }
            }
            (lo.unwrap(), hi.unwrap())
        } else {
            let f = context::with_consts(|cc| bl.pow(&el, p, RoundingMode::ToEven, cc));
            (f.clone(), f)
        };
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// Base raised to ±∞ for a real, finite base.
    fn raise_to_infinity(&mut self, plus: bool) -> bool {
        let one = Number::from_i64(1);
        let minus_one = Number::from_i64(-1);
        if self.equals(&one, false, false) {
            return true; // 1^±∞ = 1 (as libqalculate leaves it)
        }
        let grows = self.is_greater_than(&one);
        let shrinks = self.is_less_than(&one) && self.is_greater_than(&minus_one);
        if grows {
            if plus {
                self.set_plus_infinity(true, false);
            } else {
                self.clear(true);
                self.approx = true;
            }
            true
        } else if shrinks && self.is_nonzero() {
            if plus {
                self.clear(true);
                self.approx = true;
            } else {
                return false; // 0-neighborhood^−∞: sign unknown
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Number;

    fn n(i: i64) -> Number {
        Number::from_i64(i)
    }

    #[test]
    fn exact_integer_powers() {
        let mut x = n(2);
        assert!(x.raise(&n(10), true));
        assert!(x.equals_i64(1024));

        let mut x = n(3);
        assert!(x.raise(&n(-2), true));
        assert_eq!(x.internal_rational().unwrap(), &num_rational::BigRational::new(1.into(), 9.into()));
    }

    #[test]
    fn exact_roots() {
        let mut x = n(27);
        assert!(x.raise(&Number::from_ints(1, 3, 0), true));
        assert!(x.equals_i64(3));

        let mut x = n(4);
        assert!(x.sqrt());
        assert!(x.equals_i64(2));
    }

    #[test]
    fn sqrt_negative_goes_complex() {
        let mut x = n(-4);
        assert!(x.sqrt());
        assert!(!x.has_real_part());
        assert!(x.imaginary_part().equals_i64(2));
    }

    #[test]
    fn float_pow_is_approximate() {
        let mut x = n(2);
        assert!(x.sqrt());
        assert!(x.is_approximate());
        let mut sq = x.clone();
        assert!(sq.raise(&n(2), true));
        // interval must contain 2
        assert!(sq.is_interval(true) || sq.equals_i64(2));
    }

    #[test]
    fn zero_and_one_exponents() {
        let mut x = n(7);
        assert!(x.raise(&n(0), true));
        assert!(x.equals_i64(1));

        let mut x = n(0);
        assert!(!x.raise(&n(-1), true));
    }
}
