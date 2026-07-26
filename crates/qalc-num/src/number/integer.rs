//! Integer / number-theoretic / bitwise operations and rounding —
//! port of the corresponding `Number.cc` methods (gcd, lcm, factorial,
//! binomial, floor/ceil/trunc/round/frac, mod/rem, shifts, bit ops).

use super::{Number, RealValue};
use crate::options::RoundingMode;
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

impl Number {
    /// `gcd(o)` — integers only.
    pub fn gcd(&mut self, o: &Number) -> bool {
        let (Some(a), Some(b)) = (self.to_bigint(), o.to_bigint()) else {
            return false;
        };
        let g = a.gcd(b);
        self.value = RealValue::Rational(BigRational::from_integer(g));
        self.set_precision_and_approximate_from(o);
        true
    }

    /// `lcm(o)` — integers only.
    pub fn lcm(&mut self, o: &Number) -> bool {
        let (Some(a), Some(b)) = (self.to_bigint(), o.to_bigint()) else {
            return false;
        };
        let l = a.lcm(b);
        self.value = RealValue::Rational(BigRational::from_integer(l));
        self.set_precision_and_approximate_from(o);
        true
    }

    /// `factorial()` — non-negative integers.
    pub fn factorial(&mut self) -> bool {
        let Some(n) = self.to_i64() else {
            return false;
        };
        if n < 0 {
            return false;
        }
        if n > 1_000_000 {
            return false; // matches libqalculate's practical bound behaviour
        }
        let mut acc = BigInt::one();
        for i in 2..=n {
            acc *= i;
        }
        self.value = RealValue::Rational(BigRational::from_integer(acc));
        true
    }

    /// `doubleFactorial()`.
    pub fn double_factorial(&mut self) -> bool {
        let Some(n) = self.to_i64() else {
            return false;
        };
        if n < -1 {
            return false;
        }
        if n > 1_000_000 {
            return false;
        }
        let mut acc = BigInt::one();
        let mut i = n;
        while i > 1 {
            acc *= i;
            i -= 2;
        }
        self.value = RealValue::Rational(BigRational::from_integer(acc));
        true
    }

    /// `multiFactorial(o)` — n!^(k).
    pub fn multi_factorial(&mut self, o: &Number) -> bool {
        let (Some(n), Some(k)) = (self.to_i64(), o.to_i64()) else {
            return false;
        };
        if n < 0 || k <= 0 || n > 1_000_000 {
            return false;
        }
        let mut acc = BigInt::one();
        let mut i = n;
        while i > 1 {
            acc *= i;
            i -= k;
        }
        self.value = RealValue::Rational(BigRational::from_integer(acc));
        true
    }

    /// `binomial(m, k)` — self = C(m, k).
    pub fn binomial(&mut self, m: &Number, k: &Number) -> bool {
        let (Some(m), Some(k)) = (m.to_bigint(), k.to_i64()) else {
            return false;
        };
        if k < 0 {
            return false;
        }
        let mut acc = BigInt::one();
        let mut m_i = m.clone();
        for i in 1..=k {
            acc *= &m_i;
            acc /= i;
            m_i -= 1;
        }
        self.value = RealValue::Rational(BigRational::from_integer(acc));
        true
    }

    /// `floor()`.
    pub fn floor(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        match &self.value {
            RealValue::PlusInfinity | RealValue::MinusInfinity => true,
            RealValue::Rational(r) => {
                let f = r.numer().div_floor(r.denom());
                self.value = RealValue::Rational(BigRational::from_integer(f));
                true
            }
            RealValue::Float { lower, upper } => {
                // libqalculate: INTERVAL_FLOOR uses lower bound's floor.
                let (Some((ln, ld)), Some((un, ud))) = (
                    crate::float::bigfloat_to_ratio(lower),
                    crate::float::bigfloat_to_ratio(upper),
                ) else {
                    return false;
                };
                let fl = ln.div_floor(&ld);
                let fu = un.div_floor(&ud);
                if fl == fu {
                    self.value = RealValue::Rational(BigRational::from_integer(fl));
                    true
                } else {
                    // ambiguous interval floor: keep lower's floor (C++
                    // mpfr_floor on both bounds keeps an interval of integers)
                    let p = crate::context::bit_precision();
                    self.value = RealValue::Float {
                        lower: crate::float::bigfloat_from_bigint(
                            &fl,
                            p,
                            astro_float::RoundingMode::Down,
                        ),
                        upper: crate::float::bigfloat_from_bigint(
                            &fu,
                            p,
                            astro_float::RoundingMode::Up,
                        ),
                    };
                    true
                }
            }
        }
    }

    /// `ceil()`.
    pub fn ceil(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        if !self.negate() {
            return false;
        }
        if !self.floor() {
            self.negate();
            return false;
        }
        self.negate()
    }

    /// `trunc()` — toward zero.
    pub fn trunc(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        if self.is_non_negative() {
            self.floor()
        } else {
            self.ceil()
        }
    }

    /// `frac()` — fractional part (x − trunc(x)).
    pub fn frac(&mut self) -> bool {
        if self.has_imaginary_part() || self.is_infinite(true) {
            return false;
        }
        let mut t = self.clone();
        if !t.trunc() {
            return false;
        }
        self.subtract(&t)
    }

    /// `round(mode)` — to nearest integer.
    pub fn round(&mut self, mode: RoundingMode) -> bool {
        if self.has_imaginary_part() {
            return false;
        }
        match &self.value {
            RealValue::PlusInfinity | RealValue::MinusInfinity => true,
            RealValue::Rational(r) => {
                if r.denom().is_one() {
                    return true;
                }
                let two = BigInt::from(2);
                let num2 = r.numer() * &two;
                let (q2, _r2) = num2.div_mod_floor(r.denom());
                // q2 = floor(2x); x rounds to floor((q2+1)/2) with tie logic
                let (q, rem) = q2.div_mod_floor(&two);
                let rounded = if rem.is_zero() {
                    // exactly q2/2 = q: check if x is exactly halfway
                    let is_half = (r.numer() * &two).mod_floor(r.denom()).is_zero()
                        && !(r.numer() % r.denom()).is_zero();
                    if is_half {
                        let lower = q.clone(); // floor(x)
                        let upper = &q + 1u32; // hmm: for exact half, floor(2x)=2·floor(x)+1
                        let _ = upper;
                        lower
                    } else {
                        q
                    }
                } else {
                    // floor(2x) odd ⇒ fractional part ≥ 0.5 in floor terms
                    let lower = q.clone();
                    let upper = &q + 1u32;
                    let frac_num = r.numer().mod_floor(r.denom());
                    let half_cmp = (&frac_num * &two).cmp(r.denom());
                    match half_cmp {
                        std::cmp::Ordering::Less => lower,
                        std::cmp::Ordering::Greater => upper,
                        std::cmp::Ordering::Equal => {
                            let neg = r.is_negative();
                            match mode {
                                RoundingMode::HalfAwayFromZero => {
                                    if neg { lower } else { upper }
                                }
                                RoundingMode::HalfTowardZero => {
                                    if neg { upper } else { lower }
                                }
                                RoundingMode::HalfToEven => {
                                    if lower.is_even() { lower } else { upper }
                                }
                                RoundingMode::HalfToOdd => {
                                    if lower.is_odd() { lower } else { upper }
                                }
                                RoundingMode::HalfUp => upper,
                                RoundingMode::HalfDown => lower,
                                RoundingMode::Up => upper,
                                RoundingMode::Down => lower,
                                RoundingMode::TowardZero => {
                                    if neg { upper } else { lower }
                                }
                                RoundingMode::AwayFromZero => {
                                    if neg { lower } else { upper }
                                }
                                RoundingMode::HalfRandom => lower,
                            }
                        }
                    }
                };
                self.value = RealValue::Rational(BigRational::from_integer(rounded));
                true
            }
            RealValue::Float { .. } => {
                // Round via the exact rational midpoint.
                let mut mid = self.clone();
                if let RealValue::Float { lower, upper } = &mid.value {
                    let (Some((ln, ld)), Some((un, ud))) = (
                        crate::float::bigfloat_to_ratio(lower),
                        crate::float::bigfloat_to_ratio(upper),
                    ) else {
                        return false;
                    };
                    let lo = BigRational::new(ln, ld);
                    let hi = BigRational::new(un, ud);
                    let m = (&lo + &hi) / BigRational::from_integer(2.into());
                    mid.value = RealValue::Rational(m);
                }
                if !mid.round(mode) {
                    return false;
                }
                *self = mid;
                true
            }
        }
    }

    /// `round()` default: half away from zero (libqalculate default) unless
    /// `halfway_to_even`.
    pub fn round_default(&mut self, halfway_to_even: bool) -> bool {
        self.round(if halfway_to_even {
            RoundingMode::HalfToEven
        } else {
            RoundingMode::HalfAwayFromZero
        })
    }

    /// `mod(o)` — floored modulo (sign follows divisor).
    pub fn mod_floor(&mut self, o: &Number) -> bool {
        if o.is_zero() {
            return false;
        }
        if let (RealValue::Rational(a), RealValue::Rational(b)) = (&self.value, &o.value) {
            if a.denom().is_one() && b.denom().is_one() {
                let m = a.numer().mod_floor(b.numer());
                self.value = RealValue::Rational(BigRational::from_integer(m));
                self.set_precision_and_approximate_from(o);
                return true;
            }
            // rational mod: a − floor(a/b)·b
            let q = a / b;
            let fq = q.numer().div_floor(q.denom());
            let m = a - b * BigRational::from_integer(fq);
            self.value = RealValue::Rational(m);
            self.set_precision_and_approximate_from(o);
            return true;
        }
        // float path: a − floor(a/b)·b
        let mut q = self.clone();
        if !q.divide(o) || !q.floor() {
            return false;
        }
        if !q.multiply(o) {
            return false;
        }
        self.subtract(&q)
    }

    /// `rem(o)` — truncated remainder (sign follows dividend).
    pub fn rem(&mut self, o: &Number) -> bool {
        if o.is_zero() {
            return false;
        }
        if let (RealValue::Rational(a), RealValue::Rational(b)) = (&self.value, &o.value) {
            if a.denom().is_one() && b.denom().is_one() {
                let m = a.numer() % b.numer();
                self.value = RealValue::Rational(BigRational::from_integer(m));
                self.set_precision_and_approximate_from(o);
                return true;
            }
        }
        let mut q = self.clone();
        if !q.divide(o) || !q.trunc() {
            return false;
        }
        if !q.multiply(o) {
            return false;
        }
        self.subtract(&q)
    }

    /// `iquo(o)` — truncated integer quotient.
    pub fn iquo(&mut self, o: &Number) -> bool {
        if o.is_zero() {
            return false;
        }
        if !self.divide(o) {
            return false;
        }
        self.trunc()
    }

    /// `isIntegerDivisible(o)`.
    pub fn is_integer_divisible(&self, o: &Number) -> bool {
        match (self.to_bigint(), o.to_bigint()) {
            (Some(a), Some(b)) if !b.is_zero() => (a % b).is_zero(),
            _ => false,
        }
    }

    // ------------------------------------------------------------------
    // Bitwise (integers; semantics of GMP mpz two's-complement ops)
    // ------------------------------------------------------------------

    pub fn bit_and(&mut self, o: &Number) -> bool {
        let (Some(a), Some(b)) = (self.to_bigint(), o.to_bigint()) else {
            return false;
        };
        let v = a & b;
        self.value = RealValue::Rational(BigRational::from_integer(v));
        self.set_precision_and_approximate_from(o);
        true
    }

    pub fn bit_or(&mut self, o: &Number) -> bool {
        let (Some(a), Some(b)) = (self.to_bigint(), o.to_bigint()) else {
            return false;
        };
        let v = a | b;
        self.value = RealValue::Rational(BigRational::from_integer(v));
        self.set_precision_and_approximate_from(o);
        true
    }

    pub fn bit_xor(&mut self, o: &Number) -> bool {
        let (Some(a), Some(b)) = (self.to_bigint(), o.to_bigint()) else {
            return false;
        };
        let v = a ^ b;
        self.value = RealValue::Rational(BigRational::from_integer(v));
        self.set_precision_and_approximate_from(o);
        true
    }

    /// `bitNot()` — one's complement (mpz_com): ~x = −x − 1.
    pub fn bit_not(&mut self) -> bool {
        let Some(a) = self.to_bigint() else {
            return false;
        };
        let v = -(a + BigInt::one());
        self.value = RealValue::Rational(BigRational::from_integer(v));
        true
    }

    /// `bitEqv(o)` — ~(a ^ b).
    pub fn bit_eqv(&mut self, o: &Number) -> bool {
        if !self.bit_xor(o) {
            return false;
        }
        self.bit_not()
    }

    /// `shiftLeft(o)`.
    pub fn shift_left(&mut self, o: &Number) -> bool {
        let (Some(a), Some(s)) = (self.to_bigint(), o.to_i64()) else {
            return false;
        };
        if s < 0 || s > 1_000_000 {
            return false;
        }
        let v = a << (s as usize);
        self.value = RealValue::Rational(BigRational::from_integer(v));
        self.set_precision_and_approximate_from(o);
        true
    }

    /// `shiftRight(o)` — arithmetic shift (mpz_fdiv_q_2exp).
    pub fn shift_right(&mut self, o: &Number) -> bool {
        let (Some(a), Some(s)) = (self.to_bigint(), o.to_i64()) else {
            return false;
        };
        if s < 0 || s > 1_000_000 {
            return false;
        }
        let v = a.div_floor(&(BigInt::one() << (s as usize)));
        self.value = RealValue::Rational(BigRational::from_integer(v));
        self.set_precision_and_approximate_from(o);
        true
    }

    /// `shift(o)` — left for positive o, right for negative.
    pub fn shift(&mut self, o: &Number) -> bool {
        let Some(s) = o.to_i64() else {
            return false;
        };
        if s >= 0 {
            self.shift_left(&Number::from_i64(s))
        } else {
            self.shift_right(&Number::from_i64(-s))
        }
    }

    /// `bitGet(bit)` — 1-indexed in libqalculate usage? (mpz_tstbit is
    /// 0-indexed; Number::bitGet passes the raw index).
    pub fn bit_get(&self, bit: u64) -> i32 {
        match self.to_bigint() {
            Some(z) => {
                if z.bit(bit) {
                    1
                } else {
                    0
                }
            }
            None => -1,
        }
    }

    /// `bitSet(bit, set)`.
    pub fn bit_set(&mut self, bit: u64, set: bool) -> bool {
        let Some(z) = self.to_bigint() else {
            return false;
        };
        let mut z = z.clone();
        z.set_bit(bit, set);
        self.value = RealValue::Rational(BigRational::from_integer(z));
        true
    }

    /// `factorize(factors)` — prime factorization by trial division.
    ///
    /// TODO(port): the cofactor left after trial division is pushed as-is,
    /// with no primality test. For a semiprime whose factors both exceed the
    /// bound below, the "factor" list therefore contains a *composite*, and
    /// nothing signals it. Closing this needs a primality test (deterministic
    /// Miller-Rabin over the usual base set) and a splitter (Pollard rho) for
    /// the composite case.
    pub fn factorize(&self, factors: &mut Vec<Number>) -> bool {
        let Some(z) = self.to_bigint() else {
            return false;
        };
        if z.is_zero() {
            return false;
        }
        let mut n = z.magnitude().clone();
        if z.is_negative() {
            factors.push(Number::from_i64(-1));
        }
        if n.is_one() {
            if factors.is_empty() {
                factors.push(Number::from_i64(1));
            }
            return true;
        }
        let mut d = num_bigint::BigUint::from(2u32);
        // Trial division up to a bound. Past it the loop gives up and the
        // remainder is pushed unfactored — see the TODO above.
        let bound = num_bigint::BigUint::from(1_000_000u64);
        while &(&d * &d) <= &n {
            if d > bound {
                break;
            }
            while (&n % &d).is_zero() {
                factors.push(Number::from_bigint(BigInt::from(d.clone())));
                n /= &d;
            }
            d += if d == num_bigint::BigUint::from(2u32) { 1u32 } else { 2u32 };
        }
        if !n.is_one() {
            factors.push(Number::from_bigint(BigInt::from(n)));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcd_lcm() {
        let mut a = Number::from_i64(12);
        assert!(a.gcd(&Number::from_i64(18)));
        assert_eq!(a.to_i64(), Some(6));
        let mut b = Number::from_i64(4);
        assert!(b.lcm(&Number::from_i64(6)));
        assert_eq!(b.to_i64(), Some(12));
    }

    #[test]
    fn factorials() {
        let mut n = Number::from_i64(10);
        assert!(n.factorial());
        assert_eq!(n.to_i64(), Some(3628800));
        let mut d = Number::from_i64(9);
        assert!(d.double_factorial());
        assert_eq!(d.to_i64(), Some(945));
        let mut b = Number::new();
        assert!(b.binomial(&Number::from_i64(10), &Number::from_i64(3)));
        assert_eq!(b.to_i64(), Some(120));
    }

    #[test]
    fn rounding_ops() {
        let mut n = Number::from_ints(7, 2, 0); // 3.5
        assert!(n.round(RoundingMode::HalfAwayFromZero));
        assert_eq!(n.to_i64(), Some(4));
        let mut m = Number::from_ints(7, 2, 0);
        assert!(m.round(RoundingMode::HalfToEven));
        assert_eq!(m.to_i64(), Some(4), "3.5 → 4 (even)");
        let mut e = Number::from_ints(5, 2, 0); // 2.5
        assert!(e.round(RoundingMode::HalfToEven));
        assert_eq!(e.to_i64(), Some(2), "2.5 → 2 (even)");
        let mut k = Number::from_ints(-5, 2, 0);
        assert!(k.round(RoundingMode::HalfAwayFromZero));
        assert_eq!(k.to_i64(), Some(-3));
        let mut f = Number::from_ints(-7, 4, 0); // −1.75
        assert!(f.floor());
        assert_eq!(f.to_i64(), Some(-2));
        let mut c = Number::from_ints(-7, 4, 0);
        assert!(c.ceil());
        assert_eq!(c.to_i64(), Some(-1));
        let mut t = Number::from_ints(-7, 4, 0);
        assert!(t.trunc());
        assert_eq!(t.to_i64(), Some(-1));
        let mut fr = Number::from_ints(-7, 4, 0);
        assert!(fr.frac());
        assert!(fr.internal_rational().unwrap() == &BigRational::new((-3).into(), 4.into()));
    }

    #[test]
    fn modulo_semantics() {
        // mod: sign follows divisor; rem: sign follows dividend.
        let mut a = Number::from_i64(-11);
        assert!(a.mod_floor(&Number::from_i64(3)));
        assert_eq!(a.to_i64(), Some(1), "-11 mod 3 = 1");
        let mut b = Number::from_i64(-11);
        assert!(b.rem(&Number::from_i64(3)));
        assert_eq!(b.to_i64(), Some(-2), "-11 rem 3 = -2");
        let mut q = Number::from_i64(-11);
        assert!(q.iquo(&Number::from_i64(3)));
        assert_eq!(q.to_i64(), Some(-3));
    }

    #[test]
    fn bitwise() {
        // Values from tests/bitwise.batch
        let mut n = Number::from_i64(0);
        assert!(n.bit_not());
        assert_eq!(n.to_i64(), Some(-1), "~0 = -1");
        let mut m = Number::from_i64(-1);
        assert!(m.bit_not());
        assert_eq!(m.to_i64(), Some(0));
        let mut k = Number::from_i64(-812);
        assert!(k.bit_not());
        assert_eq!(k.to_i64(), Some(811));
        let mut s = Number::from_i64(18);
        assert!(s.shift_right(&Number::from_i64(2)));
        assert_eq!(s.to_i64(), Some(4), "18 >> 2 = 4");
        let mut t = Number::from_i64(-18);
        assert!(t.shift_right(&Number::from_i64(1)));
        assert_eq!(t.to_i64(), Some(-9), "-18 >> 1 = -9");
        let mut u = Number::from_i64(-18);
        assert!(u.shift_left(&Number::from_i64(2)));
        assert_eq!(u.to_i64(), Some(-72));
        let mut x = Number::from_i64(0b1011_0010);
        assert!(x.bit_or(&Number::from_i64(0b0111_0001)));
        assert_eq!(x.to_i64(), Some(0b1111_0011));
        let mut y = Number::from_i64(0b0101);
        assert!(y.bit_and(&Number::from_i64(0b1001)));
        assert_eq!(y.to_i64(), Some(0b0001));
    }

    #[test]
    fn factorize_basic() {
        let mut factors = Vec::new();
        assert!(Number::from_i64(360).factorize(&mut factors));
        let vals: Vec<i64> = factors.iter().map(|f| f.to_i64().unwrap()).collect();
        assert_eq!(vals, vec![2, 2, 2, 3, 3, 5]);
    }
}
