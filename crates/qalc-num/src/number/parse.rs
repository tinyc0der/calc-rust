//! Number-from-string parsing — port of `Number::set(string, ParseOptions)`
//! (Number.cc:479) and the subset of `Number::setUncertainty` (Number.cc:1752)
//! it depends on.
//!
//! Scope of this pass (mirroring the C++ structure):
//! - integer bases 2..=36, with 0x/0o/0b/0d prefixes and leading-0 octal;
//! - '.' decimal point, exact-rational result (num / base^decimals);
//! - 'E'/'e' base-10 exponent (bases <= 10) and 'p' base-2 exponent (base 16);
//! - leading '-' signs (toggling), ' ' and '_' digit grouping;
//! - duodecimal X/E digits;
//! - 'i' imaginary suffix (`b_cplx`);
//! - "+/-"/"±" uncertainty and "(n)" parenthesized uncertainty;
//! - two's complement (`po.twos_complement` / `po.hexadecimal_twos_complement`
//!   with `po.binary_bits`);
//! - read_precision: value becomes a float interval of ±half the last digit.
//!
//! Skipped this pass (see TODO(port) markers below): roman numerals,
//! binary-coded decimal, bijective base-26, non-integer/negative/Unicode/
//! custom bases, sexagesimal ':' notation, and error reporting via
//! `CALCULATOR->error` (unrecognized characters are silently ignored).

use super::{Number, RealValue};
use crate::context;
use crate::float::bigfloat_from_ratio;
use crate::options::{base as base_const, ParseOptions, ReadPrecisionMode};
use astro_float::{BigFloat, RoundingMode};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Zero};

/// `s2i` (util.cc): leading optional-sign integer, `atol`-style.
fn s2i(s: &str) -> i64 {
    let bytes = s.trim_start().as_bytes();
    let mut i = 0;
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        neg = bytes[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add((bytes[i] - b'0') as i64);
        i += 1;
    }
    if neg {
        -v
    } else {
        v
    }
}

/// `mpz_ui_pow_ui` helper for exponent scaling.
fn pow_big(b: u32, e: u64) -> BigInt {
    // TODO(port): C++ raises "Too large exponent." above ULONG_MAX/10; we
    // additionally cap at u32::MAX for BigInt::pow.
    BigInt::from(b).pow(e.min(u32::MAX as u64) as u32)
}

/// `TEST_TWOS` macro (Number.cc:984): decide whether the number is a negative
/// two's-complement value based on its first digit (and `po.binary_bits`).
fn test_twos(bytes: &[u8], index: usize, po: &ParseOptions) -> bool {
    let c = bytes[index];
    let mut b_twos = (po.twos_complement && po.base == 2 && c == b'1')
        || (po.hexadecimal_twos_complement
            && po.base == 16
            && (c == b'8'
                || c == b'9'
                || (b'a'..=b'f').contains(&c)
                || (b'A'..=b'F').contains(&c))
            && !bytes.contains(&b'p'));
    if b_twos && po.base == 2 && po.binary_bits > 1 {
        let mut n: usize = 1;
        for &b in &bytes[index + 1..] {
            if b == b'0' || b == b'1' {
                n += 1;
            } else if b == b'E' || b == b'e' || b == b'.' {
                break;
            }
        }
        let mut bits = po.binary_bits as usize;
        while n > bits {
            bits *= 2;
        }
        if n != bits {
            b_twos = false;
        }
    }
    if b_twos && po.base == 16 && po.binary_bits >= 4 {
        let mut n: usize = 1;
        for &b in &bytes[index + 1..] {
            if b.is_ascii_alphanumeric() {
                n += 1;
            } else if b == b'.' || b == b'p' {
                break;
            }
        }
        let mut bits = po.binary_bits as usize;
        while n * 4 > bits {
            bits *= 2;
        }
        if n * 4 != bits {
            b_twos = false;
        }
    }
    b_twos
}

impl Number {
    /// `Number(string, ParseOptions)` constructor.
    pub fn parse(text: &str, po: &ParseOptions) -> Number {
        let mut n = Number::new();
        n.set_from_str(text, po);
        n
    }

    /// `void Number::set(string number, const ParseOptions &po)` — Number.cc:479.
    pub fn set_from_str(&mut self, text: &str, po: &ParseOptions) {
        // look for +/- for specified uncertainty of number
        // TODO(port): C++ also allows BASE_CUSTOM when the calculator's custom
        // input base is within [-62, 62]; we have no Calculator singleton.
        if po.base != base_const::UNICODE && po.base != base_const::CUSTOM {
            let pm = text
                .find('\u{00B1}')
                .map(|i| (i, '\u{00B1}'.len_utf8()))
                .or_else(|| text.find("+/-").map(|i| (i, "+/-".len())));
            if let Some((pm_index, pm_len)) = pm {
                // +/- overrides read precision option
                let mut po2 = po.clone();
                po2.read_precision = ReadPrecisionMode::DontRead;
                // read number without uncertainty
                self.set_from_str(&text[..pm_index], &po2);
                let rest = &text[pm_index + pm_len..];
                if !rest.is_empty() {
                    // read number after +/- and set uncertainty
                    let pm_nr = Number::parse(rest, &po2);
                    self.set_uncertainty(&pm_nr);
                }
                return;
            }
        }
        match po.base {
            // TODO(port): binary-coded decimal (BASE_BINARY_DECIMAL) — Number.cc:500.
            // TODO(port): bijective base-26 (BASE_BIJECTIVE_26) — Number.cc:531.
            // TODO(port): roman numerals (BASE_ROMAN_NUMERALS) — Number.cc:754.
            // TODO(port): non-integer/negative/Unicode/custom bases (golden
            // ratio, pi, e, sqrt2, BASE_UNICODE, BASE_CUSTOM, ...) — Number.cc:560.
            base_const::BINARY_DECIMAL
            | base_const::BIJECTIVE_26
            | base_const::ROMAN_NUMERALS
            | base_const::UNICODE
            | base_const::CUSTOM
            | base_const::GOLDEN_RATIO
            | base_const::SUPER_GOLDEN_RATIO
            | base_const::PI
            | base_const::E
            | base_const::SQRT2 => {
                self.clear(false);
                return;
            }
            _ => {}
        }

        // read numbers with positive integer bases >= 2 and <= 36
        let mut base: i32 = if (2..=36).contains(&po.base) { po.base } else { 10 };

        let mut i_unc: i64 = 0;
        let mut num = BigInt::zero();
        let mut den = BigInt::one();
        let mut unc_num = BigInt::zero();
        let mut unc_den = BigInt::one();

        // remove_blank_ends
        let mut number: &str = text.trim_matches(|c: char| c.is_ascii_whitespace());

        // Remove base prefixes. A prefix also *selects* the base: the
        // reference binary reads `0xFF` as 255, `0b1011` as 11 and `0o17` as
        // 15 regardless of the configured input base.
        let nb = number.as_bytes();
        if nb.len() >= 2 && nb[0] == b'0' && (nb[1] == b'x' || nb[1] == b'X') {
            number = &number[2..];
            base = 16;
        } else if nb.len() >= 2 && nb[0] == b'0' && (nb[1] == b'o' || nb[1] == b'O') {
            number = &number[2..];
            base = 8;
        } else if nb.len() >= 2 && nb[0] == b'0' && (nb[1] == b'b' || nb[1] == b'B') {
            number = &number[2..];
            base = 2;
        } else if nb.len() >= 2 && nb[0] == b'0' && (nb[1] == b'd' || nb[1] == b'D') {
            number = &number[2..];
            base = 12;
        } else if po.base == 8 && nb.len() > 1 && nb[0] == b'0' && nb[1] != b'.' {
            number = &number[1..];
        }
        let bytes = number.as_bytes();

        // determine if value is negative for numbers using binary or
        // hexadecimal complement representation
        let mut b_twos = false;

        let mut numbers_started = false;
        let mut minus = false;
        let mut in_decimals = false;
        let mut b_cplx = false;
        let mut exp_minus = false;
        let mut exp: u64 = 0;

        let mut index = 0usize;
        while index < bytes.len() {
            let c = bytes[index];
            if c >= b'0' && ((base >= 10 && c <= b'9') || (base < 10 && c < b'0' + base as u8)) {
                if !numbers_started && !in_decimals {
                    b_twos = test_twos(bytes, index, po);
                }
                // multiply previous value with base
                num *= base;
                // for negative numbers using complement representation,
                // digit value = base - digit - 1 (e.g. 0=1, 1=0 in binary base)
                let d = (c - b'0') as i64;
                let v = if b_twos { (base as i64 - 1) - d } else { d };
                if v != 0 {
                    num += v;
                }
                if in_decimals {
                    // if after decimal separator: multiply denominator by base
                    den *= base;
                }
                numbers_started = true;
            } else if po.base == base_const::DUODECIMAL
                && (c == b'X' || c == b'E' || c == b'x' || c == b'e')
            {
                // duodecimal numbers use X and E instead of A and B
                num *= base;
                num += if c == b'E' || c == b'e' { 11 } else { 10 };
                if in_decimals {
                    den *= base;
                }
                numbers_started = true;
            } else if base > 10 && c >= b'a' && c < b'a' + (base as u8 - 10) {
                // (base > 36 case-sensitive digits do not apply: base <= 36 here)
                if !numbers_started && !in_decimals {
                    b_twos = test_twos(bytes, index, po);
                }
                num *= base;
                let d = (c - b'a') as i64 + 10;
                let v = if b_twos { (base as i64 - 1) - d } else { d };
                if v != 0 {
                    num += v;
                }
                if in_decimals {
                    den *= base;
                }
                numbers_started = true;
            } else if base > 10 && c >= b'A' && c < b'A' + (base as u8 - 10) {
                if !numbers_started && !in_decimals {
                    b_twos = test_twos(bytes, index, po);
                }
                num *= base;
                let d = (c - b'A') as i64 + 10;
                let v = if b_twos { (base as i64 - 1) - d } else { d };
                if v != 0 {
                    num += v;
                }
                if in_decimals {
                    den *= base;
                }
                numbers_started = true;
            } else if numbers_started
                && (((c == b'E' || c == b'e') && base <= 10 && index + 1 < bytes.len())
                    || (base == 16 && c == b'p'))
            {
                // scientific e-notation: read base-10 exponent after E
                // (in base 16, 'p' introduces a base-2 exponent)
                index += 1;
                numbers_started = false;
                let max_exp = u64::MAX / 10;
                while index < bytes.len() {
                    let c2 = bytes[index];
                    if c2.is_ascii_digit() {
                        if exp > max_exp {
                            // TODO(port): CALCULATOR->error "Too large exponent."
                        } else {
                            exp = exp * 10 + (c2 - b'0') as u64;
                            numbers_started = true;
                        }
                    } else if !numbers_started && c2 == b'-' {
                        exp_minus = !exp_minus;
                    }
                    index += 1;
                }
                break;
            } else if c == b'.' {
                if in_decimals {
                    // TODO(port): CALCULATOR->error "Misplaced decimal separator ignored"
                } else {
                    in_decimals = true;
                }
            } else if c == b':' {
                // TODO(port): sexagesimal ':' notation (recursive parse with
                // successive division by 60) — Number.cc:1076. Ignored for now.
            } else if !numbers_started && c == b'-' {
                minus = !minus;
            } else if c == b'i' {
                // i found: number is imaginary
                // TODO(port): 'j' alias when the imaginary unit variable is
                // named "j" (needs Calculator singleton).
                b_cplx = true;
            } else if base == 10 && c == b'(' && index + 2 <= bytes.len() {
                // digits in parentheses at the end of a number specify the
                // uncertainty of the preceding digits
                let par_i = bytes[index + 1..]
                    .iter()
                    .position(|&b| b == b')')
                    .map(|p| p + index + 1);
                match par_i {
                    None => {
                        i_unc = s2i(&number[index + 1..]);
                        index = bytes.len() - 1;
                    }
                    Some(p) if p > index + 1 => {
                        i_unc = s2i(&number[index + 1..p]);
                        index = p;
                    }
                    _ => {}
                }
                if i_unc > 0 {
                    unc_num = BigInt::from(i_unc);
                    unc_den = den.clone();
                }
            } else if c != b' ' && (c != b'_' || !numbers_started) {
                // TODO(port): CALCULATOR->error "Character ... was ignored";
                // unrecognized characters (incl. non-ASCII bytes) are skipped.
            }
            index += 1;
        }

        if b_twos {
            num += 1;
            minus = !minus;
        }

        self.clear(false);

        let exp_base: u32 = if base == 16 { 2 } else { 10 };
        if exp_minus && exp > 0 {
            // if negative exponent multiply denominator
            let e_den = pow_big(exp_base, exp);
            den *= &e_den;
            if i_unc > 0 {
                unc_den *= &e_den;
            }
        }

        if i_unc <= 0
            && (po.read_precision == ReadPrecisionMode::Always
                || (in_decimals && po.read_precision == ReadPrecisionMode::WhenDecimals))
        {
            // read precision: uncertainty = value of last digit / 2
            // (e.g. 22.0 = 22.0 +/- 0.05)
            // upper end point = ((num * 2) + 1)/(den * 2)
            // lower end point = ((num * 2) - 1)/(den * 2)
            num *= 2;
            den *= 2;
            num += 1;
            let mut rv1 = if minus { -num.clone() } else { num.clone() };
            num -= 2;
            let mut rv2 = if minus { -num.clone() } else { num.clone() };

            if !exp_minus && exp > 0 {
                // if positive exponent multiply numerator
                let e_num = pow_big(exp_base, exp);
                rv1 *= &e_num;
                rv2 *= &e_num;
            }

            // numbers with uncertainty/interval are always floating point;
            // bounds are rounded inward as in the C++ (fu with RNDD, fl with RNDU)
            let p = context::bit_precision();
            let (upper_num, lower_num) = if minus { (rv2, rv1) } else { (rv1, rv2) };
            let mut upper = bigfloat_from_ratio(&upper_num, &den, p, RoundingMode::Down);
            let mut lower = bigfloat_from_ratio(&lower_num, &den, p, RoundingMode::Up);
            // TODO(port): C++ additionally nudges each bound 3 ulps inward
            // (mpfr_nextbelow/mpfr_nextabove) to avoid rounding issues when
            // displaying significant digits; astro-float has no next-ulp op.
            if matches!(lower.cmp(&upper), Some(c) if c > 0) {
                std::mem::swap(&mut lower, &mut upper);
            }
            self.value = RealValue::Float { lower, upper };
            self.approx = true;
            self.test_float_result(true);

            if b_cplx {
                // i was found: this is an imaginary number
                let re = Number {
                    value: std::mem::replace(
                        &mut self.value,
                        RealValue::Rational(BigRational::zero()),
                    ),
                    imag: None,
                    approx: self.approx,
                    is_imag_part: false,
                    precision: self.precision,
                };
                self.set_imaginary_part(&re);
            }
        } else {
            if !exp_minus && exp > 0 {
                // if positive exponent multiply numerator
                let e_num = pow_big(exp_base, exp);
                num *= &e_num;
                if i_unc > 0 {
                    unc_num *= &e_num;
                }
            }
            if minus {
                num = -num;
            }
            // set numerator and denominator of rational value (canonicalized)
            let r = BigRational::new(num, den);
            if b_cplx {
                // i was found: this is an imaginary number
                self.set_imaginary_part(&Number::from_rational(r));
            } else {
                self.value = RealValue::Rational(r);
            }
            if i_unc > 0 {
                // set uncertainty specified in parentheses at end of number string
                let nr_unc = Number::from_rational(BigRational::new(unc_num, unc_den));
                self.set_uncertainty(&nr_unc);
            }
        }
    }

    /// Port of `Number::setUncertainty(o, to_precision = false)` — Number.cc:1752.
    /// Turns the value into a float interval `[self - o, self + o]`.
    pub fn set_uncertainty(&mut self, o: &Number) {
        if o.is_zero() {
            return;
        }
        if o.has_imaginary_part() {
            let mut im = match self.imag.take() {
                Some(im) => im,
                None => {
                    let mut n = Box::new(Number::new());
                    n.mark_as_imaginary_part(true);
                    n
                }
            };
            im.set_uncertainty(&o.imaginary_part());
            self.set_precision_and_approximate_from(&im);
            self.imag = Some(im);
            if o.has_real_part() {
                self.set_uncertainty(&o.real_part());
            }
            return;
        }
        if o.is_infinite(true) {
            let p = context::bit_precision();
            self.value = RealValue::Float {
                lower: BigFloat::from_f64(f64::NEG_INFINITY, p),
                upper: BigFloat::from_f64(f64::INFINITY, p),
            };
            return;
        }
        if self.is_infinite(true) {
            return;
        }
        self.approx = true;
        // (to_precision branch omitted: Number::set always passes false)
        if o.is_negative() {
            let mut o_abs = o.clone();
            o_abs.negate();
            self.set_uncertainty(&o_abs);
            return;
        }
        let p = context::bit_precision();
        let new_val = match (&self.value, &o.value) {
            (RealValue::Rational(r), RealValue::Rational(ro)) => {
                let lo = r - ro;
                let hi = r + ro;
                RealValue::Float {
                    lower: bigfloat_from_ratio(lo.numer(), lo.denom(), p, RoundingMode::Down),
                    upper: bigfloat_from_ratio(hi.numer(), hi.denom(), p, RoundingMode::Up),
                }
            }
            (RealValue::Rational(r), RealValue::Float { upper: ou, .. }) => RealValue::Float {
                lower: bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::Down)
                    .sub(ou, p, RoundingMode::Down),
                upper: bigfloat_from_ratio(r.numer(), r.denom(), p, RoundingMode::Up)
                    .add(ou, p, RoundingMode::Up),
            },
            (RealValue::Float { lower: sl, upper: su }, RealValue::Rational(ro)) => {
                let of = bigfloat_from_ratio(ro.numer(), ro.denom(), p, RoundingMode::Up);
                RealValue::Float {
                    lower: sl.sub(&of, p, RoundingMode::Down),
                    upper: su.add(&of, p, RoundingMode::Up),
                }
            }
            (RealValue::Float { lower: sl, upper: su }, RealValue::Float { upper: ou, .. }) => {
                RealValue::Float {
                    lower: sl.sub(ou, p, RoundingMode::Down),
                    upper: su.add(ou, p, RoundingMode::Up),
                }
            }
            _ => return,
        };
        self.value = new_val;
        self.test_float_result(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(n: i64, d: i64) -> BigRational {
        BigRational::new(BigInt::from(n), BigInt::from(d))
    }

    fn parse10(s: &str) -> Number {
        Number::parse(s, &ParseOptions::default())
    }

    fn parse_base(s: &str, base: i32) -> Number {
        let po = ParseOptions { base, ..ParseOptions::default() };
        Number::parse(s, &po)
    }

    fn assert_exact(n: &Number, r: BigRational, what: &str) {
        assert!(!n.is_approximate(), "{what}: expected exact, got {n:?}");
        assert_eq!(n.internal_rational(), Some(&r), "{what}");
    }

    #[test]
    fn integers() {
        assert_exact(&parse10("0"), q(0, 1), "0");
        assert_exact(&parse10("42"), q(42, 1), "42");
        assert_exact(&parse10("-17"), q(-17, 1), "-17");
        assert_exact(&parse10("--17"), q(17, 1), "--17 double minus toggles");
        assert_exact(&parse10("  42  "), q(42, 1), "surrounding whitespace");
        assert!(parse10("42").is_integer());
    }

    #[test]
    fn exact_decimals() {
        assert_exact(&parse10("3.25"), q(13, 4), "3.25 == 13/4");
        assert_exact(&parse10("0.1"), q(1, 10), "0.1 == 1/10");
        assert_exact(&parse10(".5"), q(1, 2), ".5 == 1/2");
        assert_exact(&parse10("-3.5"), q(-7, 2), "-3.5");
        assert_exact(&parse10("2.50"), q(5, 2), "trailing zero");
    }

    #[test]
    fn exponents() {
        assert_exact(&parse10("1e3"), q(1000, 1), "1e3");
        assert_exact(&parse10("1.5e-2"), q(3, 200), "1.5e-2");
        assert_exact(&parse10("2.5E2"), q(250, 1), "2.5E2");
        assert!(parse10("2.5E2").is_integer());
        assert_exact(&parse10("-2e2"), q(-200, 1), "-2e2");
        // '-' anywhere before exponent digits toggles the exponent sign
        assert_exact(&parse10("1e--2"), q(100, 1), "1e--2 double minus");
        // trailing 'e' with nothing after is not an exponent (ignored)
        assert_exact(&parse10("5e"), q(5, 1), "trailing e ignored");
    }

    #[test]
    fn digit_grouping() {
        assert_exact(&parse10("1 000"), q(1000, 1), "space grouping");
        assert_exact(&parse10("1_000"), q(1000, 1), "underscore grouping");
    }

    #[test]
    fn hexadecimal() {
        assert_exact(&parse_base("ff", 16), q(255, 1), "hex ff");
        assert_exact(&parse_base("DEAD", 16), q(57005, 1), "hex DEAD");
        assert_exact(&parse_base("0xff", 16), q(255, 1), "hex 0x prefix");
        assert_exact(&parse_base("a.8", 16), q(21, 2), "hex a.8 == 10.5");
        // 'p' introduces a base-2 exponent in base 16: a.8p4 = 10.5 * 16
        assert_exact(&parse_base("a.8p4", 16), q(168, 1), "hex a.8p4");
        assert_exact(&parse_base("1p-1", 16), q(1, 2), "hex 1p-1 == 1/2");
    }

    #[test]
    fn binary_and_octal() {
        assert_exact(&parse_base("1011", 2), q(11, 1), "binary 1011");
        assert_exact(&parse_base("0b101", 2), q(5, 1), "binary 0b prefix");
        assert_exact(&parse_base("017", 8), q(15, 1), "octal leading zero");
        assert_exact(&parse_base("0o17", 8), q(15, 1), "octal 0o prefix");
        // binary 'e' exponent is a power of ten (base <= 10), as in C++
        assert_exact(&parse_base("1e2", 2), q(100, 1), "binary 1e2 == 100");
    }

    #[test]
    fn duodecimal_and_base36() {
        assert_exact(&parse_base("X", 12), q(10, 1), "duodecimal X");
        assert_exact(&parse_base("1E", 12), q(23, 1), "duodecimal 1E");
        assert_exact(&parse_base("b", 12), q(11, 1), "duodecimal b");
        assert_exact(&parse_base("z", 36), q(35, 1), "base 36 z");
        assert_exact(&parse_base("10", 36), q(36, 1), "base 36 10");
    }

    #[test]
    fn imaginary_suffix() {
        let n = parse10("5i");
        assert!(n.has_imaginary_part() && !n.has_real_part(), "{n:?}");
        assert_eq!(n.imaginary_part().internal_rational(), Some(&q(5, 1)));
        let m = parse10("-2.5i");
        assert_eq!(m.imaginary_part().internal_rational(), Some(&q(-5, 2)));
    }

    #[test]
    fn read_precision_always() {
        let po = ParseOptions {
            read_precision: ReadPrecisionMode::Always,
            ..ParseOptions::default()
        };
        // 1.2 -> interval [1.15, 1.25] (± half of last digit)
        let n = Number::parse("1.2", &po);
        assert!(n.is_approximate(), "{n:?}");
        assert!(n.is_floating_point() && n.is_interval(true), "{n:?}");
        let lo = n.lower_end_point();
        let hi = n.upper_end_point();
        assert!(lo.is_greater_than(&Number::from_ints(114, 100, 0)), "{lo:?}");
        assert!(lo.is_less_than(&Number::from_ints(116, 100, 0)), "{lo:?}");
        assert!(hi.is_greater_than(&Number::from_ints(124, 100, 0)), "{hi:?}");
        assert!(hi.is_less_than(&Number::from_ints(126, 100, 0)), "{hi:?}");
        // integers become intervals too with ALWAYS
        let m = Number::parse("12", &po);
        assert!(m.is_approximate() && m.is_interval(true), "{m:?}");
    }

    #[test]
    fn read_precision_when_decimals() {
        let po = ParseOptions {
            read_precision: ReadPrecisionMode::WhenDecimals,
            ..ParseOptions::default()
        };
        let n = Number::parse("12", &po);
        assert!(!n.is_approximate(), "integer stays exact: {n:?}");
        assert_eq!(n.internal_rational(), Some(&q(12, 1)));
        let m = Number::parse("1.2", &po);
        assert!(m.is_approximate() && m.is_interval(true), "{m:?}");
    }

    #[test]
    fn plus_minus_uncertainty() {
        for s in ["1.5+/-0.1", "1.5\u{00B1}0.1"] {
            let n = parse10(s);
            assert!(n.is_approximate() && n.is_interval(true), "{s}: {n:?}");
            let lo = n.lower_end_point();
            let hi = n.upper_end_point();
            assert!(lo.is_greater_than(&Number::from_ints(139, 100, 0)), "{s}");
            assert!(lo.is_less_than(&Number::from_ints(141, 100, 0)), "{s}");
            assert!(hi.is_greater_than(&Number::from_ints(159, 100, 0)), "{s}");
            assert!(hi.is_less_than(&Number::from_ints(161, 100, 0)), "{s}");
        }
    }

    #[test]
    fn parenthesized_uncertainty() {
        // 1.234(5) = 1.234 ± 0.005 -> interval [1.229, 1.239]
        let n = parse10("1.234(5)");
        assert!(n.is_approximate() && n.is_interval(true), "{n:?}");
        let lo = n.lower_end_point();
        let hi = n.upper_end_point();
        assert!(lo.is_greater_than(&Number::from_ints(1228, 1000, 0)), "{lo:?}");
        assert!(lo.is_less_than(&Number::from_ints(1230, 1000, 0)), "{lo:?}");
        assert!(hi.is_greater_than(&Number::from_ints(1238, 1000, 0)), "{hi:?}");
        assert!(hi.is_less_than(&Number::from_ints(1240, 1000, 0)), "{hi:?}");
    }

    #[test]
    fn twos_complement_binary() {
        let po = ParseOptions {
            base: 2,
            twos_complement: true,
            ..ParseOptions::default()
        };
        assert_exact(&Number::parse("1111", &po), q(-1, 1), "binary 1111 twos");
        assert_exact(&Number::parse("1110", &po), q(-2, 1), "binary 1110 twos");
        assert_exact(&Number::parse("0111", &po), q(7, 1), "binary 0111 positive");
        // with binary_bits set, only matching widths are complemented
        let po8 = ParseOptions { binary_bits: 8, ..po.clone() };
        assert_exact(&Number::parse("1111", &po8), q(15, 1), "4 digits != 8 bits");
        assert_exact(
            &Number::parse("11111111", &po8),
            q(-1, 1),
            "8 digits == 8 bits",
        );
    }

    #[test]
    fn twos_complement_hex() {
        let po = ParseOptions {
            base: 16,
            hexadecimal_twos_complement: true,
            ..ParseOptions::default()
        };
        assert_exact(&Number::parse("ff", &po), q(-1, 1), "hex ff twos");
        assert_exact(&Number::parse("FE", &po), q(-2, 1), "hex FE twos");
        assert_exact(&Number::parse("7f", &po), q(127, 1), "hex 7f positive");
    }

    #[test]
    fn reuse_resets_state() {
        let po_always = ParseOptions {
            read_precision: ReadPrecisionMode::Always,
            ..ParseOptions::default()
        };
        let mut n = Number::parse("1.2", &po_always);
        assert!(n.is_approximate());
        n.set_from_str("3", &ParseOptions::default());
        assert_exact(&n, q(3, 1), "reparse resets approx");
        assert!(!n.has_imaginary_part());
    }

    #[test]
    fn empty_and_garbage() {
        assert_exact(&parse10(""), q(0, 1), "empty string is zero");
        // unrecognized characters are ignored (C++ raises non-fatal errors)
        assert_exact(&parse10("4$2"), q(42, 1), "garbage ignored");
    }
}
