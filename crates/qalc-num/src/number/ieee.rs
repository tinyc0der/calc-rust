//! IEEE-754 bit-string conversion — port of `standard_expbits`, `to_float`
//! and `from_float` (Number.cc:10429-10600).
//!
//! `52.345 to float` yields the 32 bits of the single-precision encoding,
//! and `float(0100…)` reads them back. The C++ builds the mantissa by
//! printing the fraction in base 2 with a fixed decimal count; this port
//! computes the bits directly from the exact rational value, which is the
//! same result without a round trip through formatting.

use super::{Number, RealValue};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};

/// `standard_expbits(bits)`: the exponent width IEEE-754 pairs with a given
/// total width.
pub fn standard_expbits(bits: u32) -> u32 {
    if bits <= 16 {
        5
    } else if bits <= 32 {
        8
    } else if bits <= 64 {
        11
    } else if bits <= 128 {
        15
    } else {
        // ceil to a multiple of 32, then round(4*log2(bits)) - 13.
        let mut b = bits;
        if b % 32 != 0 {
            b = (b / 32 + 1) * 32;
        }
        let v = (4.0 * (b as f64).log2()).round() as i64 - 13;
        v.max(2) as u32
    }
}

/// Number of explicit mantissa bits. The 80-bit x87 format stores the
/// leading 1 explicitly, so it has one fewer implicit bit.
fn mantissa_bits(bits: u32, expbits: u32) -> u32 {
    bits - expbits - if bits == 80 { 2 } else { 1 }
}

/// `to_float`: encode `n` as a `bits`-wide IEEE-754 bit string.
///
/// Returns the raw bits (no grouping); the caller formats them. `None` if
/// the width is unusable.
pub fn to_float(n: &Number, bits: u32, expbits: u32) -> Option<String> {
    let expbits = if expbits == 0 {
        standard_expbits(bits)
    } else if expbits > bits - 2 {
        return None;
    } else {
        expbits
    };
    let expbias: i64 = (1i64 << (expbits - 1)) - 1;
    let mbits = mantissa_bits(bits, expbits);
    let explicit_leading = bits == 80;

    let mut v = n.clone();
    v.intervalToMidValue();
    let neg = v.real_part_is_negative();
    if neg {
        v.negate();
    }
    let mut s = String::with_capacity(bits as usize);
    s.push(if neg { '1' } else { '0' });

    // Zero, infinity and non-real values use the reserved exponent patterns.
    if v.is_zero() || v.is_infinite(true) || v.has_imaginary_part() {
        let one = !v.is_zero();
        for _ in 0..expbits {
            s.push(if one { '1' } else { '0' });
        }
        if explicit_leading && v.is_infinite(true) {
            s.push('1');
        } else {
            s.push('0');
        }
        while s.len() < bits as usize {
            s.push('0');
        }
        if v.has_imaginary_part() {
            // NaN: a non-zero payload distinguishes it from infinity.
            s.pop();
            s.push('1');
        }
        return Some(s);
    }

    // Exact rational magnitude of the value.
    let q = v.to_exact_rational()?;
    if q.is_zero() {
        return None;
    }

    // exp = floor(log2(q))
    let mut exp: i64 = ilog2_floor(&q);
    // 2^exp <= q < 2^(exp+1)
    if exp > expbias {
        // Overflow to infinity.
        for _ in 0..expbits {
            s.push('1');
        }
        s.push(if explicit_leading { '1' } else { '0' });
        while s.len() < bits as usize {
            s.push('0');
        }
        return Some(s);
    }

    let mut stored_exp = exp + expbias;
    let subnormal = stored_exp <= 0;
    if subnormal {
        // Subnormals share the smallest exponent and drop the implicit 1.
        exp = 1 - expbias;
        stored_exp = 0;
    }

    // significand = q / 2^exp, in [1,2) when normal.
    let sig = scale_pow2(&q, -exp);
    // Round the significand to `mbits` fractional bits, half to even.
    let total_bits = if explicit_leading { mbits + 1 } else { mbits };
    let scaled = scale_pow2(&sig, total_bits as i64);
    let mut int_val = round_half_even(&scaled);

    // Rounding can carry into the next binade (e.g. 1.111… -> 10.000…).
    let limit = BigInt::one() << (total_bits + 1) as usize;
    if !subnormal && int_val >= limit {
        int_val >>= 1u32;
        stored_exp += 1;
        if stored_exp > 2 * expbias + 1 {
            for _ in 0..expbits {
                s.push('1');
            }
            s.push(if explicit_leading { '1' } else { '0' });
            while s.len() < bits as usize {
                s.push('0');
            }
            return Some(s);
        }
    }

    // Exponent field.
    for i in (0..expbits).rev() {
        s.push(if (stored_exp >> i) & 1 == 1 { '1' } else { '0' });
    }
    // Mantissa field: drop the implicit leading bit unless it is explicit.
    let mant_field_bits = if explicit_leading { mbits + 1 } else { mbits };
    let mask = (BigInt::one() << mant_field_bits as usize) - BigInt::one();
    let field = if explicit_leading {
        int_val.clone() & ((BigInt::one() << (mbits + 1) as usize) - BigInt::one())
    } else {
        int_val.clone() & mask
    };
    for i in (0..mant_field_bits).rev() {
        let bit = (&field >> i as usize) & BigInt::one();
        s.push(if bit.is_one() { '1' } else { '0' });
    }
    s.truncate(bits as usize);
    Some(s)
}

/// `from_float`: decode a `bits`-wide IEEE-754 bit string.
pub fn from_float(sbin: &str, bits: u32, expbits: u32) -> Option<Number> {
    let expbits = if expbits == 0 {
        standard_expbits(bits)
    } else if expbits > bits - 2 {
        return None;
    } else {
        expbits
    };
    let mut s: String = sbin.chars().filter(|c| *c == '0' || *c == '1').collect();
    if s.len() < bits as usize {
        let pad = "0".repeat(bits as usize - s.len());
        s = pad + &s;
    }
    if s.len() > bits as usize {
        return None;
    }
    let b: Vec<u8> = s.bytes().collect();
    let neg = b[0] == b'1';

    // Exponent field.
    let mut exp: i64 = 0;
    let mut all_ones = true;
    for i in 1..=expbits as usize {
        exp = exp * 2 + if b[i] == b'1' { 1 } else { 0 };
        if b[i] != b'1' {
            all_ones = false;
        }
    }
    if all_ones {
        // Infinity when the significand is empty, otherwise NaN.
        let sig_start = expbits as usize + 1;
        let has_payload = b[sig_start..].iter().any(|c| *c == b'1');
        let is_inf = if bits == 80 {
            // The 80-bit format's explicit leading bit is set for infinity.
            b[sig_start] == b'1' && !b[sig_start + 1..].iter().any(|c| *c == b'1')
        } else {
            !has_payload
        };
        if !is_inf {
            return None;
        }
        let mut n = Number::new();
        if neg {
            n.set_minus_infinity(false, false);
        } else {
            n.set_plus_infinity(false, false);
        }
        return Some(n);
    }

    let subnormal = exp == 0;
    let expbias: i64 = (1i64 << (expbits - 1)) - 1;
    let mut e = exp - expbias;
    if subnormal {
        e += 1;
    }

    // Significand: implicit leading 1 unless subnormal or 80-bit explicit.
    let mut frac = if subnormal || bits == 80 {
        BigRational::zero()
    } else {
        BigRational::one()
    };
    let mut step = if bits == 80 {
        BigRational::one()
    } else {
        BigRational::new(BigInt::one(), BigInt::from(2))
    };
    for i in (expbits as usize + 1)..bits as usize {
        if b[i] == b'1' {
            frac += &step;
        }
        step /= BigInt::from(2);
    }

    // value = 2^e * frac
    let mut value = scale_pow2(&frac, e);
    if neg {
        value = -value;
    }
    let mut n = Number::from_rational(value);
    n.set_approximate(true);
    Some(n)
}

/// The difference between `n` and its `bits`-wide float encoding
/// (`floatError`).
pub fn float_error(n: &Number, bits: u32, expbits: u32) -> Option<Number> {
    let encoded = to_float(n, bits, expbits)?;
    let decoded = from_float(&encoded, bits, expbits)?;
    let mut d = decoded;
    d.set_approximate(false);
    let mut diff = n.clone();
    if !diff.subtract(&d) || !diff.abs() {
        return None;
    }
    Some(diff)
}

/// `q * 2^k` for positive or negative `k`.
fn scale_pow2(q: &BigRational, k: i64) -> BigRational {
    if k >= 0 {
        q * BigRational::from_integer(BigInt::one() << k as usize)
    } else {
        q / BigRational::from_integer(BigInt::one() << (-k) as usize)
    }
}

/// floor(log2(q)) for a positive rational.
fn ilog2_floor(q: &BigRational) -> i64 {
    let n = q.numer().magnitude().bits() as i64;
    let d = q.denom().magnitude().bits() as i64;
    // n-1 <= log2(numer) < n, likewise for the denominator.
    let mut e = n - d;
    // Correct the off-by-one by comparing against 2^e.
    while scale_pow2(q, -e) < BigRational::one() {
        e -= 1;
    }
    while scale_pow2(q, -(e + 1)) >= BigRational::one() {
        e += 1;
    }
    e
}

/// Round a non-negative rational to an integer, ties to even.
fn round_half_even(q: &BigRational) -> BigInt {
    let floor = q.numer().div_floor(q.denom());
    let frac = q - BigRational::from_integer(floor.clone());
    let half = BigRational::new(BigInt::one(), BigInt::from(2));
    match frac.cmp(&half) {
        std::cmp::Ordering::Less => floor,
        std::cmp::Ordering::Greater => floor + 1,
        std::cmp::Ordering::Equal => {
            if floor.is_even() {
                floor
            } else {
                floor + 1
            }
        }
    }
}

use num_integer::Integer as _;

impl Number {
    /// The exact rational value of a real number, if it has one.
    pub(crate) fn to_exact_rational(&self) -> Option<BigRational> {
        match &self.value {
            RealValue::Rational(r) => Some(r.clone()),
            RealValue::Float { lower, upper } if lower == upper => {
                let (n, d) = crate::float::bigfloat_to_ratio(lower)?;
                Some(BigRational::new(n, d))
            }
            _ => None,
        }
    }

    /// Collapse an interval to its midpoint (`intervalToMidValue`).
    #[allow(non_snake_case)]
    pub(crate) fn intervalToMidValue(&mut self) {
        if let RealValue::Float { lower, upper } = &self.value {
            if lower != upper {
                let (Some((ln, ld)), Some((un, ud))) = (
                    crate::float::bigfloat_to_ratio(lower),
                    crate::float::bigfloat_to_ratio(upper),
                ) else {
                    return;
                };
                let mid = (BigRational::new(ln, ld) + BigRational::new(un, ud))
                    / BigRational::from_integer(BigInt::from(2));
                self.value = RealValue::Rational(mid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ParseOptions;

    fn parse(s: &str) -> Number {
        Number::parse(s, &ParseOptions::default())
    }

    #[test]
    fn expbits_table() {
        assert_eq!(standard_expbits(16), 5);
        assert_eq!(standard_expbits(32), 8);
        assert_eq!(standard_expbits(64), 11);
        assert_eq!(standard_expbits(128), 15);
    }

    #[test]
    fn encodes_single_precision() {
        // numberbase.batch: `52.345 to float`
        let bits = to_float(&parse("52.345"), 32, 0).unwrap();
        assert_eq!(bits, "01000010010100010110000101001000");
    }

    #[test]
    fn decodes_single_precision() {
        // numberbase.batch: `float(01000010010100010110000101001000)`
        let n = from_float("01000010010100010110000101001000", 32, 0).unwrap();
        let mut po = crate::options::PrintOptions::default();
        po.show_ending_zeroes = true;
        assert_eq!(n.print(&po), "52.34500122");
    }

    #[test]
    fn round_trips_exact_values() {
        // Powers of two and simple fractions encode exactly.
        for s in ["1", "2", "0.5", "-4", "0.25", "3"] {
            let bits = to_float(&parse(s), 32, 0).unwrap();
            let back = from_float(&bits, 32, 0).unwrap();
            let mut expect = parse(s);
            expect.set_approximate(true);
            assert!(
                back.equals(&expect, true, false),
                "{s} round trip: {bits} -> {back:?}"
            );
        }
    }

    #[test]
    fn float_error_matches_reference() {
        // numberbase.batch: `floatError(52.345)`
        let e = float_error(&parse("52.345"), 32, 0).unwrap();
        let mut po = crate::options::PrintOptions::default();
        po.show_ending_zeroes = true;
        assert_eq!(e.print(&po), "0.000001220703125");
    }

    #[test]
    fn zero_and_infinity() {
        let z = to_float(&Number::new(), 32, 0).unwrap();
        assert_eq!(z, "0".repeat(32));
        let mut inf = Number::new();
        inf.set_plus_infinity(false, false);
        let b = to_float(&inf, 32, 0).unwrap();
        assert_eq!(b, format!("0{}{}", "1".repeat(8), "0".repeat(23)));
        let back = from_float(&b, 32, 0).unwrap();
        assert!(back.is_plus_infinity());
    }

    #[test]
    fn double_precision() {
        let bits = to_float(&parse("1"), 64, 0).unwrap();
        assert_eq!(bits.len(), 64);
        // 1.0 = sign 0, exponent 1023, mantissa 0
        assert_eq!(&bits[..12], "001111111111");
        let back = from_float(&bits, 64, 0).unwrap();
        assert!(back.equals(&parse("1"), true, false));
    }

    #[test]
    fn negative_values_set_the_sign_bit() {
        let bits = to_float(&parse("-52.345"), 32, 0).unwrap();
        assert!(bits.starts_with('1'));
        let back = from_float(&bits, 32, 0).unwrap();
        assert!(back.is_negative());
    }
}
