//! Helpers bridging `num-bigint` integers/rationals and `astro-float`
//! arbitrary-precision floats — the glue GMP/MPFR provided for free
//! (`mpfr_set_q`, `mpfr_get_z`, `mpfr_cmp_q`, ...).

use astro_float::{BigFloat, Exponent, RoundingMode, Sign, WORD_BIT_SIZE};
use num_bigint::{BigInt, BigUint, Sign as IntSign};
use num_traits::Zero;

/// Construct a `BigFloat` holding `z` exactly (no rounding).
pub fn bigfloat_from_bigint_exact(z: &BigInt) -> BigFloat {
    if z.is_zero() {
        return BigFloat::from_i8(0, WORD_BIT_SIZE);
    }
    let (sign, mag) = (z.sign(), z.magnitude());
    let digits = mag.to_u64_digits();
    let e = (digits.len() * WORD_BIT_SIZE) as Exponent;
    let s = if sign == IntSign::Minus { Sign::Neg } else { Sign::Pos };
    BigFloat::from_words(&digits, s, e)
}

/// `mpfr_set_z` with rounding: `z` rounded to precision `p` bits.
pub fn bigfloat_from_bigint(z: &BigInt, p: usize, rm: RoundingMode) -> BigFloat {
    let mut f = bigfloat_from_bigint_exact(z);
    f.set_precision(p, rm).ok();
    f
}

/// `mpfr_set_q`: numerator/denominator correctly rounded to `p` bits.
/// Exact conversion of both parts followed by one correctly-rounded division.
pub fn bigfloat_from_ratio(num: &BigInt, den: &BigInt, p: usize, rm: RoundingMode) -> BigFloat {
    let fnum = bigfloat_from_bigint_exact(num);
    if den.is_zero() {
        // Caller is responsible for not passing zero denominators; mirror
        // MPFR's div-by-zero → inf behaviour for robustness.
        return if fnum.is_negative() {
            BigFloat::from_f64(f64::NEG_INFINITY, p)
        } else {
            BigFloat::from_f64(f64::INFINITY, p)
        };
    }
    let fden = bigfloat_from_bigint_exact(den);
    fnum.div(&fden, p, rm)
}

/// `mpfr_get_z(..., MPFR_RNDZ)`-style exact extraction: returns the value of
/// `f` as a `BigInt` if `f` is finite, truncating toward zero.
pub fn bigfloat_to_bigint_trunc(f: &BigFloat) -> Option<BigInt> {
    if f.is_inf() || f.is_nan() {
        return None;
    }
    if f.is_zero() {
        return Some(BigInt::zero());
    }
    let (words, n, s, e, _inexact) = f.as_raw_parts()?;
    let p = words.len() * WORD_BIT_SIZE;
    let _ = n;
    // value = W × 2^(e − p), truncate toward zero.
    let w = BigUint::from_slice_u64(words);
    let mag = if (e as isize) >= p as isize {
        w << ((e as isize - p as isize) as usize)
    } else {
        let shift = (p as isize - e as isize) as usize;
        if shift >= p { BigUint::zero() } else { w >> shift }
    };
    let mut z = BigInt::from(mag);
    if s == Sign::Neg {
        z = -z;
    }
    Some(z)
}

/// True if `f` represents an exact integer value.
pub fn bigfloat_is_integer(f: &BigFloat) -> bool {
    if f.is_inf() || f.is_nan() {
        return false;
    }
    if f.is_zero() {
        return true;
    }
    match f.as_raw_parts() {
        Some((words, _n, _s, e, _)) => {
            let p = (words.len() * WORD_BIT_SIZE) as isize;
            let e = e as isize;
            if e >= p {
                return true;
            }
            if e < 0 {
                return false;
            }
            // Fractional part is the low (p − e) bits.
            let frac_bits = (p - e) as usize;
            let mut checked = 0usize;
            for w in words {
                if checked >= frac_bits {
                    break;
                }
                let remaining = frac_bits - checked;
                let mask: u64 = if remaining >= WORD_BIT_SIZE {
                    u64::MAX
                } else {
                    (1u64 << remaining) - 1
                };
                if w & mask != 0 {
                    return false;
                }
                checked += WORD_BIT_SIZE;
            }
            true
        }
        None => false,
    }
}

/// Exact `BigFloat` → rational (numerator, denominator 2^k). None for inf/nan.
pub fn bigfloat_to_ratio(f: &BigFloat) -> Option<(BigInt, BigInt)> {
    if f.is_inf() || f.is_nan() {
        return None;
    }
    if f.is_zero() {
        return Some((BigInt::zero(), BigInt::from(1)));
    }
    let (words, _n, s, e, _) = f.as_raw_parts()?;
    let p = (words.len() * WORD_BIT_SIZE) as isize;
    let e = e as isize;
    let w = BigUint::from_slice_u64(words);
    let mut num = BigInt::from(w);
    if s == Sign::Neg {
        num = -num;
    }
    // value = num × 2^(e − p)
    if e >= p {
        Some((num << ((e - p) as usize), BigInt::from(1)))
    } else {
        Some((num, BigInt::from(1) << ((p - e) as usize)))
    }
}

/// Extension trait: BigUint construction from u64 slice (num-bigint exposes
/// u32 digits; convert).
pub trait FromSliceU64 {
    fn from_slice_u64(words: &[u64]) -> BigUint;
}

impl FromSliceU64 for BigUint {
    fn from_slice_u64(words: &[u64]) -> BigUint {
        let mut u32s = Vec::with_capacity(words.len() * 2);
        for w in words {
            u32s.push(*w as u32);
            u32s.push((*w >> 32) as u32);
        }
        BigUint::from_slice(&u32s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn roundtrip_bigint() {
        for s in ["0", "1", "-1", "123456789012345678901234567890", "-987654321"] {
            let z = BigInt::from_str(s).unwrap();
            let f = bigfloat_from_bigint_exact(&z);
            assert_eq!(bigfloat_to_bigint_trunc(&f).unwrap(), z, "roundtrip {s}");
            assert!(bigfloat_is_integer(&f), "is_integer {s}");
        }
    }

    #[test]
    fn ratio_rounding_directions() {
        let one = BigInt::from(1);
        let three = BigInt::from(3);
        let lo = bigfloat_from_ratio(&one, &three, 128, RoundingMode::Down);
        let hi = bigfloat_from_ratio(&one, &three, 128, RoundingMode::Up);
        assert_eq!(lo.cmp(&hi), Some(-1));
        assert!(!bigfloat_is_integer(&lo));
    }

    #[test]
    fn to_ratio_exact() {
        let z = BigInt::from(625);
        let f = bigfloat_from_bigint_exact(&z).div(
            &bigfloat_from_bigint_exact(&BigInt::from(100)),
            128,
            RoundingMode::ToEven,
        );
        // 6.25 is exactly representable in binary.
        let (n, d) = bigfloat_to_ratio(&f).unwrap();
        assert_eq!(n.clone() * 4, d.clone() * 25, "6.25 = {n}/{d}");
    }
}
