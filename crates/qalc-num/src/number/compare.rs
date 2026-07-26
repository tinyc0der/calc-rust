//! Comparison — port of `Number::compare`/`equals` and friends, including
//! the interval-aware `ComparisonResult` semantics.

use super::{Number, RealValue};
use crate::context;
use crate::float::bigfloat_cmp;
use crate::options::ComparisonResult;
use num_traits::Zero;

impl Number {
    /// `equals(o)` — exact equality (intervals equal only if identical points
    /// unless `allow_interval`).
    pub fn equals(&self, o: &Number, allow_interval: bool, allow_infinite: bool) -> bool {
        match (&self.imag, &o.imag) {
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
            (RealValue::Rational(a), RealValue::Rational(b)) => a == b,
            (RealValue::Float { lower: al, upper: au }, RealValue::Float { lower: bl, upper: bu }) => {
                if !allow_interval && (al != au || bl != bu) {
                    return false;
                }
                al == bl && au == bu
            }
            (RealValue::Rational(_), RealValue::Float { lower, upper })
            | (RealValue::Float { lower, upper }, RealValue::Rational(_)) => {
                if !allow_interval && lower != upper {
                    return false;
                }
                let p = context::bit_precision();
                let (rl, ru, fl, fu) = if matches!(self.value, RealValue::Rational(_)) {
                    (self.lower_bound_float(p), self.upper_bound_float(p), lower.clone(), upper.clone())
                } else {
                    (o.lower_bound_float(p), o.upper_bound_float(p), lower.clone(), upper.clone())
                };
                rl == fl && ru == fu
            }
            _ => false,
        }
    }

    pub fn equals_i64(&self, i: i64) -> bool {
        !self.has_imaginary_part()
            && matches!(&self.value, RealValue::Rational(r)
                if r.denom() == &num_bigint::BigInt::from(1) && r.numer() == &num_bigint::BigInt::from(i))
    }

    /// `compare(o)` — interval-aware three-way comparison of real parts.
    /// Returns Greater when self < o (libqalculate's convention: the result
    /// describes o relative to self... actually `COMPARISON_RESULT_LESS`
    /// means self > o is false — we mirror the C++: returns LESS when
    /// self < o).
    pub fn compare(&self, o: &Number) -> ComparisonResult {
        if self.has_imaginary_part() || o.has_imaginary_part() {
            if self.equals(o, false, true) {
                return ComparisonResult::Equal;
            }
            return ComparisonResult::NotEqual;
        }
        // Infinities first.
        match (&self.value, &o.value) {
            (RealValue::PlusInfinity, RealValue::PlusInfinity)
            | (RealValue::MinusInfinity, RealValue::MinusInfinity) => return ComparisonResult::Equal,
            (RealValue::PlusInfinity, _) => return ComparisonResult::Greater,
            (_, RealValue::PlusInfinity) => return ComparisonResult::Less,
            (RealValue::MinusInfinity, _) => return ComparisonResult::Less,
            (_, RealValue::MinusInfinity) => return ComparisonResult::Greater,
            (RealValue::Rational(a), RealValue::Rational(b)) => {
                return match a.cmp(b) {
                    std::cmp::Ordering::Less => ComparisonResult::Less,
                    std::cmp::Ordering::Equal => ComparisonResult::Equal,
                    std::cmp::Ordering::Greater => ComparisonResult::Greater,
                };
            }
            _ => {}
        }
        // Interval comparison via bounds.
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (bl, bu) = (o.lower_bound_float(p), o.upper_bound_float(p));
        let au_bl = bigfloat_cmp(&au, &bl);
        let al_bu = bigfloat_cmp(&al, &bu);
        if au_bl == Some(-1) {
            return ComparisonResult::Less;
        }
        if al_bu == Some(1) {
            return ComparisonResult::Greater;
        }
        if al == bl && au == bu {
            if al == au {
                return ComparisonResult::Equal;
            }
            return ComparisonResult::EqualLimits;
        }
        if au_bl == Some(0) && al_bu != Some(0) {
            return ComparisonResult::EqualOrLess;
        }
        if al_bu == Some(0) && au_bl != Some(0) {
            return ComparisonResult::EqualOrGreater;
        }
        let al_bl = bigfloat_cmp(&al, &bl);
        let au_bu = bigfloat_cmp(&au, &bu);
        match (al_bl, au_bu) {
            (Some(x), Some(y)) if x <= 0 && y >= 0 => ComparisonResult::Contains,
            (Some(x), Some(y)) if x >= 0 && y <= 0 => ComparisonResult::IsContained,
            (Some(x), _) if x < 0 => ComparisonResult::OverlappingLess,
            _ => ComparisonResult::OverlappingGreater,
        }
    }

    pub fn is_greater_than(&self, o: &Number) -> bool {
        self.compare(o) == ComparisonResult::Greater
    }

    pub fn is_less_than(&self, o: &Number) -> bool {
        self.compare(o) == ComparisonResult::Less
    }

    pub fn is_greater_than_or_equal_to(&self, o: &Number) -> bool {
        matches!(self.compare(o), ComparisonResult::Greater | ComparisonResult::Equal)
    }

    pub fn is_less_than_or_equal_to(&self, o: &Number) -> bool {
        matches!(self.compare(o), ComparisonResult::Less | ComparisonResult::Equal)
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
    fn rational_compare() {
        let a = Number::from_ints(1, 3, 0);
        let b = Number::from_ints(1, 2, 0);
        assert_eq!(a.compare(&b), ComparisonResult::Less);
        assert_eq!(b.compare(&a), ComparisonResult::Greater);
        assert_eq!(a.compare(&a.clone()), ComparisonResult::Equal);
    }

    #[test]
    fn infinity_compare() {
        let mut inf = Number::new();
        inf.set_plus_infinity(false, false);
        let x = Number::from_i64(1_000_000);
        assert_eq!(inf.compare(&x), ComparisonResult::Greater);
        assert_eq!(x.compare(&inf), ComparisonResult::Less);
    }

    #[test]
    fn interval_vs_rational() {
        let mut pi = Number::new();
        pi.pi();
        let three = Number::from_i64(3);
        let four = Number::from_i64(4);
        assert_eq!(pi.compare(&three), ComparisonResult::Greater);
        assert_eq!(pi.compare(&four), ComparisonResult::Less);
        assert!(pi.is_greater_than(&three) && pi.is_less_than(&four));
    }
}
