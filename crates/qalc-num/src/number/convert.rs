//! Conversions out of `Number` — `intValue`, `floatValue`, etc.

use super::{Number, RealValue};
use num_bigint::BigInt;
use num_traits::{One, ToPrimitive, Zero};

impl Number {
    /// Integer value if this is an exact integer that fits i64.
    pub fn to_i64(&self) -> Option<i64> {
        match &self.value {
            RealValue::Rational(r) if r.denom().is_one() && !self.has_imaginary_part() => {
                r.numer().to_i64()
            }
            _ => None,
        }
    }

    /// The exact integer value, if integer.
    pub fn to_bigint(&self) -> Option<&BigInt> {
        match &self.value {
            RealValue::Rational(r) if r.denom().is_one() && !self.has_imaginary_part() => {
                Some(r.numer())
            }
            _ => None,
        }
    }

    /// `floatValue()` — approximate f64 of the real part, the midpoint of an
    /// interval.
    ///
    /// The midpoint matters: callers use `float_value().abs()` to size the
    /// guard bits of a series whose largest term grows like e^|x|
    /// (`special.rs`, `integral_guard`). Returning the *lower* bound of an
    /// interval like [0.001, 900] would size the guard from 0.001, the series
    /// would lose every digit to cancellation, and the range rejection would
    /// never fire.
    pub fn float_value(&self) -> f64 {
        match &self.value {
            RealValue::Rational(r) => {
                let n_f = r.numer().to_f64().unwrap_or(f64::NAN);
                let d_f = r.denom().to_f64().unwrap_or(f64::NAN);
                let val = n_f / d_f;
                if val.is_nan() && !r.numer().is_zero() && !r.denom().is_zero() {
                    let n_bits = r.numer().bits() as i64;
                    let d_bits = r.denom().bits() as i64;
                    let shift = (n_bits.max(d_bits) - 1000).max(0) as u64;
                    let n_s = (r.numer() >> shift).to_f64().unwrap_or(0.0);
                    let d_s = (r.denom() >> shift).to_f64().unwrap_or(1.0);
                    n_s / d_s
                } else {
                    val
                }
            }
            RealValue::Float { lower, upper } => {
                let (l, u) = (bigfloat_to_f64(lower), bigfloat_to_f64(upper));
                if l == u {
                    l
                } else {
                    // Halve first: the endpoints of a wide interval can sum to
                    // infinity even when the midpoint is finite.
                    l / 2.0 + u / 2.0
                }
            }
            RealValue::PlusInfinity => f64::INFINITY,
            RealValue::MinusInfinity => f64::NEG_INFINITY,
        }
    }

    /// `integerLength()` — bit length of the integer.
    pub fn integer_length(&self) -> i32 {
        match self.to_bigint() {
            Some(z) if !z.is_zero() => z.magnitude().bits() as i32,
            _ => 0,
        }
    }

    /// `isEven`-adjacent helper used across ports: numerator magnitude digits.
    pub fn numerator_digits(&self) -> usize {
        match &self.value {
            RealValue::Rational(r) => r.numer().magnitude().to_string().len(),
            _ => 0,
        }
    }
}

fn bigfloat_to_f64(f: &astro_float::BigFloat) -> f64 {
    if f.is_inf_pos() {
        return f64::INFINITY;
    }
    if f.is_inf_neg() {
        return f64::NEG_INFINITY;
    }
    if f.is_nan() {
        return f64::NAN;
    }
    if f.is_zero() {
        return 0.0;
    }
    // Round-trip through a decimal string at 17 significant digits.
    let mut g = f.clone();
    g.set_precision(64, astro_float::RoundingMode::ToEven).ok();
    let (words, _n, s, e, _) = match g.as_raw_parts() {
        Some(t) => t,
        None => return f64::NAN,
    };
    let mant = words.last().copied().unwrap_or(0);
    let val = (mant as f64) * 2f64.powi(e - 64 * words.len() as i32 + (words.len() as i32 - 1) * 64);
    if s == astro_float::Sign::Neg { -val } else { val }
}
