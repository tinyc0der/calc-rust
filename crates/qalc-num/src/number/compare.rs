//! Comparison — faithful port of `Number::compare`/`equals` and friends
//! (Number.cc:2607-3050), including interval-aware `ComparisonResult`
//! semantics.
//!
//! Convention (matches C++): `self.compare(o)` describes **o relative to
//! self** — `Greater` means o > self, `Less` means o < self.

use super::{Number, RealValue};
use crate::float::bigfloat_to_ratio;
use crate::options::ComparisonResult;
use astro_float::BigFloat;
use num_rational::BigRational;
use num_traits::Zero;

/// Exact comparison of a BigFloat with a BigRational (mpfr_cmp_q):
/// returns sign of (f − r). None if f is NaN.
fn cmp_f_rat(f: &BigFloat, r: &BigRational) -> Option<i32> {
    if f.is_nan() {
        return None;
    }
    if f.is_inf_pos() {
        return Some(1);
    }
    if f.is_inf_neg() {
        return Some(-1);
    }
    let (n, d) = bigfloat_to_ratio(f)?;
    let fr = BigRational::new(n, d);
    Some(match fr.cmp(r) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

/// mpfr_cmp: sign of (a − b). None on NaN.
fn cmp_ff(a: &BigFloat, b: &BigFloat) -> Option<i32> {
    a.cmp(b).map(|c| c.signum() as i32)
}

impl Number {
    /// `equals(o, allow_interval, allow_infinite)` — Number.cc:2610.
    pub fn equals(&self, o: &Number, allow_interval: bool, allow_infinite: bool) -> bool {
        // Imaginary parts must match.
        let ia = self.imag.as_deref();
        let ib = o.imag.as_deref();
        match (ia, ib) {
            (Some(a), Some(b)) => {
                if !a.equals(b, allow_interval, allow_infinite) {
                    return false;
                }
            }
            (Some(a), None) => {
                if !a.is_zero() {
                    return false;
                }
            }
            (None, Some(b)) => {
                if !b.is_zero() {
                    return false;
                }
            }
            (None, None) => {}
        }
        match (&self.value, &o.value) {
            (RealValue::PlusInfinity, RealValue::PlusInfinity)
            | (RealValue::MinusInfinity, RealValue::MinusInfinity) => allow_infinite,
            (RealValue::PlusInfinity | RealValue::MinusInfinity, _)
            | (_, RealValue::PlusInfinity | RealValue::MinusInfinity) => false,
            (RealValue::Rational(a), RealValue::Rational(b)) => a == b,
            (RealValue::Float { lower: al, upper: au }, RealValue::Float { lower: bl, upper: bu }) => {
                if !allow_interval && (al != au || bl != bu) {
                    return false;
                }
                cmp_ff(al, bl) == Some(0) && cmp_ff(au, bu) == Some(0)
            }
            (RealValue::Rational(r), RealValue::Float { lower, upper })
            | (RealValue::Float { lower, upper }, RealValue::Rational(r)) => {
                if !allow_interval && lower != upper {
                    return false;
                }
                cmp_f_rat(lower, r) == Some(0) && cmp_f_rat(upper, r) == Some(0)
            }
        }
    }

    pub fn equals_i64(&self, i: i64) -> bool {
        !self.has_imaginary_part()
            && matches!(&self.value, RealValue::Rational(r)
                if r.denom() == &num_bigint::BigInt::from(1) && r.numer() == &num_bigint::BigInt::from(i))
    }

    pub fn equals_zero(&self) -> bool {
        self.is_zero()
    }

    /// `compare(o)` — Number.cc:2750. Result describes o relative to self.
    pub fn compare(&self, o: &Number) -> ComparisonResult {
        if matches!(self.value, RealValue::PlusInfinity) && !self.has_imaginary_part() {
            if o.has_imaginary_part() || o.includes_plus_infinity() {
                return ComparisonResult::Unknown;
            }
            return ComparisonResult::Less;
        }
        if matches!(self.value, RealValue::MinusInfinity) && !self.has_imaginary_part() {
            if o.has_imaginary_part() || o.includes_minus_infinity() {
                return ComparisonResult::Unknown;
            }
            return ComparisonResult::Greater;
        }
        if matches!(o.value, RealValue::PlusInfinity) && !o.has_imaginary_part() {
            if self.has_imaginary_part() || self.includes_plus_infinity() {
                return ComparisonResult::Unknown;
            }
            return ComparisonResult::Greater;
        }
        if matches!(o.value, RealValue::MinusInfinity) && !o.has_imaginary_part() {
            if self.has_imaginary_part() || self.includes_minus_infinity() {
                return ComparisonResult::Unknown;
            }
            return ComparisonResult::Less;
        }
        if self.equals(o, false, false) {
            return ComparisonResult::Equal;
        }
        if !self.has_imaginary_part() && !o.has_imaginary_part() {
            let (i, i2): (i32, i32);
            match (&self.value, &o.value) {
                (RealValue::Rational(r), RealValue::Float { lower, upper }) => {
                    let a = cmp_f_rat(lower, r).unwrap_or(0);
                    let b = cmp_f_rat(upper, r).unwrap_or(0);
                    if a != b {
                        return ComparisonResult::Contains;
                    }
                    i = a;
                    i2 = b;
                }
                (RealValue::Float { lower: sl, upper: su }, RealValue::Float { lower: ol, upper: ou }) => {
                    let a = cmp_ff(ou, sl).unwrap_or(0);
                    let b = cmp_ff(ol, su).unwrap_or(0);
                    if a != b && b <= 0 && a >= 0 {
                        let c = cmp_ff(ol, sl).unwrap_or(0);
                        let d = cmp_ff(ou, su).unwrap_or(0);
                        return if c > 0 {
                            if d <= 0 {
                                ComparisonResult::IsContained
                            } else {
                                ComparisonResult::OverlappingGreater
                            }
                        } else if c < 0 {
                            if d >= 0 {
                                ComparisonResult::Contains
                            } else {
                                ComparisonResult::OverlappingLess
                            }
                        } else if d == 0 {
                            ComparisonResult::EqualLimits
                        } else if d > 0 {
                            ComparisonResult::Contains
                        } else {
                            ComparisonResult::IsContained
                        };
                    }
                    i = a;
                    i2 = b;
                }
                (RealValue::Float { lower, upper }, RealValue::Rational(r)) => {
                    let a = -cmp_f_rat(lower, r).unwrap_or(0);
                    let b = -cmp_f_rat(upper, r).unwrap_or(0);
                    if a != b {
                        return ComparisonResult::IsContained;
                    }
                    i = a;
                    i2 = b;
                }
                (RealValue::Rational(a), RealValue::Rational(b)) => {
                    let c = match b.cmp(a) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    };
                    i = c;
                    i2 = c;
                }
                _ => unreachable!("infinities handled above"),
            }
            let mut i = i;
            if i2 == 0 || i == 0 {
                if i == 0 {
                    i = i2;
                }
                if i > 0 {
                    return ComparisonResult::EqualOrGreater;
                } else if i < 0 {
                    return ComparisonResult::EqualOrLess;
                }
            } else if i2 != i {
                return ComparisonResult::Unknown;
            }
            if i == 0 {
                ComparisonResult::Equal
            } else if i > 0 {
                ComparisonResult::Greater
            } else {
                ComparisonResult::Less
            }
        } else {
            // Complex comparison — Number.cc:2812.
            let cr = self.real_part().compare(&o.real_part());
            if !cr.is_equal_or_might_be() {
                return ComparisonResult::NotEqual;
            }
            if self.has_imaginary_part() && o.has_imaginary_part() {
                let ci = self.imaginary_part().compare(&o.imaginary_part());
                if !ci.is_equal_or_might_be() {
                    return ComparisonResult::NotEqual;
                }
                return ComparisonResult::Unknown;
            }
            if self.has_imaginary_part() != o.has_imaginary_part() {
                // one is definitely complex, the other real
                return ComparisonResult::NotEqual;
            }
            ComparisonResult::Unknown
        }
    }

    pub fn includes_plus_infinity(&self) -> bool {
        match &self.value {
            RealValue::PlusInfinity => true,
            RealValue::Float { upper, .. } => upper.is_inf_pos(),
            _ => false,
        }
    }

    pub fn includes_minus_infinity(&self) -> bool {
        match &self.value {
            RealValue::MinusInfinity => true,
            RealValue::Float { lower, .. } => lower.is_inf_neg(),
            _ => false,
        }
    }

    /// `isGreaterThan(o)` — Number.cc:2958 (direct, not via compare).
    pub fn is_greater_than(&self, o: &Number) -> bool {
        if matches!(self.value, RealValue::MinusInfinity) || o.is_plus_infinity() {
            return false;
        }
        if o.is_minus_infinity() {
            return true;
        }
        if matches!(self.value, RealValue::PlusInfinity) {
            return true;
        }
        if self.has_imaginary_part() || o.has_imaginary_part() {
            return false;
        }
        match (&self.value, &o.value) {
            (RealValue::Rational(r), RealValue::Float { upper, .. }) => {
                cmp_f_rat(upper, r) == Some(-1)
            }
            (RealValue::Float { lower, .. }, RealValue::Float { upper, .. }) => {
                cmp_ff(lower, upper) == Some(1)
            }
            (RealValue::Float { lower, .. }, RealValue::Rational(r)) => {
                cmp_f_rat(lower, r) == Some(1)
            }
            (RealValue::Rational(a), RealValue::Rational(b)) => a > b,
            _ => false,
        }
    }

    /// `isLessThan(o)`.
    pub fn is_less_than(&self, o: &Number) -> bool {
        o.is_greater_than(self)
    }

    /// `isGreaterThanOrEqualTo(o)`.
    pub fn is_greater_than_or_equal_to(&self, o: &Number) -> bool {
        if matches!(self.value, RealValue::MinusInfinity) {
            return o.is_minus_infinity();
        }
        if o.is_plus_infinity() {
            return matches!(self.value, RealValue::PlusInfinity);
        }
        if o.is_minus_infinity() || matches!(self.value, RealValue::PlusInfinity) {
            return true;
        }
        if self.has_imaginary_part() || o.has_imaginary_part() {
            return false;
        }
        match (&self.value, &o.value) {
            (RealValue::Rational(r), RealValue::Float { upper, .. }) => {
                matches!(cmp_f_rat(upper, r), Some(c) if c <= 0)
            }
            (RealValue::Float { lower, .. }, RealValue::Float { upper, .. }) => {
                matches!(cmp_ff(lower, upper), Some(c) if c >= 0)
            }
            (RealValue::Float { lower, .. }, RealValue::Rational(r)) => {
                matches!(cmp_f_rat(lower, r), Some(c) if c >= 0)
            }
            (RealValue::Rational(a), RealValue::Rational(b)) => a >= b,
            _ => false,
        }
    }

    pub fn is_less_than_or_equal_to(&self, o: &Number) -> bool {
        o.is_greater_than_or_equal_to(self)
    }

    pub fn is_greater_than_i64(&self, i: i64) -> bool {
        self.is_greater_than(&Number::from_i64(i))
    }

    pub fn is_less_than_i64(&self, i: i64) -> bool {
        self.is_less_than(&Number::from_i64(i))
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other, true, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_compare_convention() {
        let a = Number::from_ints(1, 3, 0);
        let b = Number::from_ints(1, 2, 0);
        // compare describes o relative to self: b > a → Greater.
        assert_eq!(a.compare(&b), ComparisonResult::Greater);
        assert_eq!(b.compare(&a), ComparisonResult::Less);
        assert_eq!(a.compare(&a.clone()), ComparisonResult::Equal);
        assert!(b.is_greater_than(&a));
        assert!(a.is_less_than(&b));
        assert!(a.is_less_than_or_equal_to(&a.clone()));
    }

    #[test]
    fn infinity_compare() {
        let mut inf = Number::new();
        inf.set_plus_infinity(false, false);
        let x = Number::from_i64(1_000_000);
        assert_eq!(inf.compare(&x), ComparisonResult::Less, "x is less than +inf");
        assert_eq!(x.compare(&inf), ComparisonResult::Greater);
        assert!(inf.is_greater_than(&x));
    }

    #[test]
    fn interval_vs_rational() {
        let mut pi = Number::new();
        pi.pi();
        let three = Number::from_i64(3);
        let four = Number::from_i64(4);
        assert!(pi.is_greater_than(&three));
        assert!(pi.is_less_than(&four));
        assert_eq!(three.compare(&pi), ComparisonResult::Greater, "pi greater than 3");
        assert_eq!(pi.compare(&three), ComparisonResult::Less);
    }

    #[test]
    fn exact_float_rational_equality() {
        // 0.5 as float equals 1/2 exactly.
        let mut h = Number::new();
        h.set_float(0.5);
        let half = Number::from_ints(1, 2, 0);
        assert!(h.equals(&half, false, false), "binary 0.5 == 1/2 exactly");
        // 0.1 as binary float does NOT equal 1/10.
        let mut t = Number::new();
        t.set_float(0.1);
        let tenth = Number::from_ints(1, 10, 0);
        assert!(!t.equals(&tenth, false, false), "binary 0.1 != 1/10");
    }

    #[test]
    fn complex_not_equal() {
        let mut a = Number::from_i64(1);
        a.set_imaginary_part(&Number::from_i64(1));
        let b = Number::from_i64(1);
        assert_eq!(a.compare(&b), ComparisonResult::NotEqual);
    }
}
