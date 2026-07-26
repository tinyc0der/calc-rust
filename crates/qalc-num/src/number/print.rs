//! Number printing — port of `Number::print` (Number.cc:10681-13169) and its
//! helpers `printMPZ`, `format_number_string`, `add_base_exponent`.
//!
//! This pass covers the paths exercised with default options: exact
//! integers, exact rationals (decimal + fraction formats), infinities,
//! complex join, and floats via their exact binary-rational value.
//! TODO(port): interval displays other than SIGNIFICANT_DIGITS/MIDPOINT,
//! preserve_format ellipses, indicate_infinite_series, two's complement,
//! sexagesimal/time/special bases, BCD, bijective-26, IEEE-float bases.

use super::{Number, RealValue};
use crate::context;
use crate::float::bigfloat_to_ratio;
use crate::options::IntervalDisplay;
use crate::options::{
    exp_mode, BaseDisplay, NumberFractionFormat, PrintOptions, RoundingMode,
};
use num_bigint::{BigInt, BigUint, Sign};
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

/// `get_rounding_mode(po)` — legacy `round_halfway_to_even` folds in.
pub(crate) fn get_rounding_mode(po: &PrintOptions) -> RoundingMode {
    if po.round_halfway_to_even {
        RoundingMode::HalfToEven
    } else {
        po.rounding
    }
}

/// `printMPZ`: integer magnitude to string in `base` (2-36). Roman numerals
/// for base -1 (up to |9999|).
pub fn print_bigint(z: &BigInt, base: i32, display_sign: bool, lower_case: bool) -> String {
    if base == crate::options::base::ROMAN_NUMERALS {
        if !z.is_zero() && z.magnitude() < &BigUint::from(10000u32) {
            return print_roman(z.to_i64().unwrap(), display_sign, lower_case);
        }
        return print_bigint(z, 10, display_sign, lower_case);
    }
    let base = base.clamp(2, 36) as u32;
    let mut s = String::new();
    if z.sign() == Sign::Minus && display_sign {
        s.push('-');
    }
    let digits = z.magnitude().to_str_radix(base);
    if base > 10 {
        if lower_case {
            s.push_str(&digits);
        } else {
            s.push_str(&digits.to_uppercase());
        }
    } else {
        s.push_str(&digits);
    }
    s
}

fn print_roman(mut value: i64, display_sign: bool, lower_case: bool) -> String {
    let mut s = String::new();
    if value < 0 {
        value = -value;
        if display_sign {
            s.push('-');
        }
    }
    let table: &[(i64, &str)] = &[
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"),
        (100, "C"), (90, "XC"), (50, "L"), (40, "XL"),
        (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    for (v, sym) in table {
        while value >= *v {
            value -= v;
            if lower_case {
                s.push_str(&sym.to_lowercase());
            } else {
                s.push_str(sym);
            }
        }
    }
    s
}

/// `format_number_string`: binary/hex bit padding+grouping, base
/// prefixes/suffixes, minus sign.
fn format_number_string(
    mut cl_str: String,
    base: i32,
    base_display: BaseDisplay,
    show_neg: bool,
    format_base_two: bool,
    po: &PrintOptions,
) -> String {
    if format_base_two
        && (base == 2 || (base == 16 && po.binary_bits >= 8))
        && base_display != BaseDisplay::None
    {
        let mut bits = po.binary_bits as usize;
        let l0 = cl_str.find(po.decimalpoint()).unwrap_or(cl_str.len());
        let mut l = l0;
        if bits == 0 {
            bits = l;
            if bits % 4 != 0 {
                bits += 4 - bits % 4;
            }
        }
        if base == 16 {
            bits /= 4;
        }
        if l < bits {
            let pad: String = "0".repeat(bits - l);
            cl_str = pad + &cl_str;
            l = bits;
        }
        if base == 2 && matches!(base_display, BaseDisplay::Normal | BaseDisplay::Suffix) {
            let mut i = l as i64 - 4;
            while i > 0 {
                cl_str.insert(i as usize, ' ');
                i -= 4;
            }
        }
    }
    let mut str = String::new();
    if show_neg {
        if po.use_unicode_signs {
            str.push('−');
        } else {
            str.push('-');
        }
    }
    match base_display {
        BaseDisplay::Normal => {
            if base == 16 {
                str.push_str("0x");
            } else if base == 8 {
                str.push('0');
            }
        }
        BaseDisplay::Alternative => {
            if base == 16 {
                str.push_str("0x0");
            } else if base == 8 {
                str.push('0');
            } else if base == 2 {
                str.push_str("0b00");
            }
        }
        _ => {}
    }
    str.push_str(&cl_str);
    str
}

/// `add_base_exponent`: append scientific-notation exponent.
fn add_base_exponent(str: &mut String, expo: i64, base: i32, po: &PrintOptions) {
    if expo == 0 {
        return;
    }
    if base == 10 && !matches!(po.exp_display, crate::options::ExpDisplay::PowerOf10) {
        match po.exp_display {
            crate::options::ExpDisplay::LowercaseE => str.push('e'),
            _ => str.push('E'),
        }
        if expo < 0 && po.use_unicode_signs {
            str.push('−');
            str.push_str(&(-expo).to_string());
        } else {
            str.push_str(&expo.to_string());
        }
    } else {
        if str == "1" {
            str.clear();
        } else {
            if po.spacious {
                str.push(' ');
            }
            if po.use_unicode_signs {
                str.push('∙');
            } else {
                str.push('*');
            }
            if po.spacious {
                str.push(' ');
            }
        }
        str.push_str(&if base == 10 { "10".to_string() } else { base.to_string() });
        str.push('^');
        str.push_str(&expo.to_string());
    }
}

/// State shared by the integer/rational printing paths.
struct PrintPrecision {
    precision: i64,
    precision_base: i64,
    i_precision_base: i64,
    approx: bool,
}

impl Number {
    fn compute_print_precision(&self, po: &PrintOptions) -> PrintPrecision {
        // PRECISION_DIGITS macro
        let global_prec = context::precision() as i64;
        let mut precision = if po.use_max_decimals && po.max_decimals < -1 && (-po.max_decimals as i64) < global_prec {
            -po.max_decimals as i64
        } else {
            global_prec
        };
        let full_prec = context::from_bit_precision(context::bit_precision()) as i64;
        if self.approx && self.precision >= 0
            && (po.preserve_precision || po.preserve_format || (self.precision as i64) < precision)
        {
            precision = self.precision as i64;
        } else if self.precision < 0 && po.preserve_precision && full_prec > precision {
            precision = full_prec;
        } else if self.approx && self.precision < 0 && po.preserve_format && full_prec - 1 > precision {
            precision = full_prec - 1;
        }
        if po.preserve_precision && !self.approx && self.precision < 0 && precision < 10000 {
            precision = 10000;
        }
        let approx = self.is_approximate();
        // precision_base: digits allowed in output base
        let base = po.base as i64;
        let mut precision_base = precision;
        let mut i_precision_base = precision_base;
        if base != 10 && (2..=36).contains(&base) {
            precision_base = digits_in_base(precision, base);
        }
        if (self.precision < 0 && full_prec > precision) || (self.precision as i64) > precision {
            i_precision_base = if self.precision < 0 { full_prec } else { self.precision as i64 };
            if base != 10 && (2..=36).contains(&base) {
                i_precision_base = digits_in_base(i_precision_base, base);
            }
        }
        PrintPrecision { precision, precision_base, i_precision_base, approx }
    }

    /// `Number::print(po)`.
    pub fn print(&self, po: &PrintOptions) -> String {
        // A variance-formula uncertainty is only a display concern once the
        // calculation is over: fold it into the value as a symmetric interval
        // and print that, so every interval display mode sees it.
        if self.unc.is_some() {
            let mut n = self.clone();
            n.resolve_variance_uncertainty();
            return n.print(po);
        }
        // Complex numbers: join real and imaginary parts.
        if self.has_imaginary_part() {
            return self.print_complex(po);
        }
        match &self.value {
            RealValue::PlusInfinity => {
                if po.use_unicode_signs {
                    "+∞".to_string()
                } else {
                    "+infinity".to_string()
                }
            }
            RealValue::MinusInfinity => {
                if po.use_unicode_signs {
                    "−∞".to_string()
                } else {
                    "-infinity".to_string()
                }
            }
            RealValue::Float { .. } => self.print_float(po),
            RealValue::Rational(r) => {
                if r.denom().is_one() {
                    self.print_integer(r.numer(), po)
                } else {
                    self.print_rational(r, po)
                }
            }
        }
    }

    fn print_complex(&self, po: &PrintOptions) -> String {
        let re = self.real_part();
        let im = self.imaginary_part();
        let has_re = !re.is_zero();
        let mut str = String::new();
        let mut im_neg = im.real_part_is_negative();
        let mut im_abs = im.clone();
        if im_neg {
            im_abs.negate();
        }
        let mut im_str = if im_abs.is_one() {
            String::new()
        } else {
            im_abs.print(po)
        };
        // An interval that straddles zero is neither positive nor negative,
        // but the value it *displays* still has a sign — `86±87 - 0.29±0.30i`
        // in the reference. Take the sign from the rendering.
        if !im_neg && (im_str.starts_with('-') || im_str.starts_with('\u{2212}')) {
            im_neg = true;
            im_abs = im;
            im_abs.negate();
            im_str = if im_abs.is_one() {
                String::new()
            } else {
                im_abs.print(po)
            };
        }
        if has_re {
            str.push_str(&re.print(po));
            if im_neg {
                str.push_str(if po.spacious { " - " } else { "-" });
            } else {
                str.push_str(if po.spacious { " + " } else { "+" });
            }
        } else if im_neg {
            str.push('-');
        }
        str.push_str(&im_str);
        str.push('i');
        str
    }

    /// Float printing: the float value is an exact binary rational; print it
    /// through the rational path with precision limited by the value's
    /// interval width (or working precision for point values).

    /// `midpoint ± half-width`, or `None` when the bounds are unusable.
    fn print_plus_minus(
        &self,
        po: &PrintOptions,
        lower: &astro_float::BigFloat,
        upper: &astro_float::BigFloat,
    ) -> Option<String> {
        let (ln_, ld) = bigfloat_to_ratio(lower)?;
        let (un, ud) = bigfloat_to_ratio(upper)?;
        let lo = BigRational::new(ln_, ld);
        let hi = BigRational::new(un, ud);
        let two = BigRational::from_integer(BigInt::from(2));
        let mid = (&lo + &hi) / &two;
        if lo == hi {
            return None;
        }
        plus_minus_string(&lo, &hi, &mid, po)
    }

    fn print_float(&self, po: &PrintOptions) -> String {
        let RealValue::Float { lower, upper } = &self.value else {
            unreachable!()
        };
        // Half-infinite intervals print as the infinite bound for now.
        if lower.is_inf_neg() || upper.is_inf_pos() {
            let mut n = Number::new();
            if upper.is_inf_pos() {
                n.set_plus_infinity(true, false);
            } else {
                n.set_minus_infinity(true, false);
            }
            return n.print(po);
        }
        // `interval_display = PlusMinus`: `midpoint ± half-width`.
        //
        // The reference prints the uncertainty to two significant digits and
        // gives the value the same number of decimals — `5+/-1` is `5.0±1.0`
        // and `Ei(3+/-0.3)` is `9.9±2.0`.
        if po.interval_display == IntervalDisplay::PlusMinus && lower != upper {
            if let Some(s) = self.print_plus_minus(po, lower, upper) {
                return s;
            }
        }
        let (mid, prec_from_interval) = if lower == upper {
            let (n, d) = match bigfloat_to_ratio(lower) {
                Some(t) => t,
                None => return "(floating point error)".to_string(),
            };
            (BigRational::new(n, d), None)
        } else {
            // Midpoint and precision from interval width:
            // i_prec = integer_log(|mid/(u−l)|, 10) + 1
            let (ln_, ld) = match bigfloat_to_ratio(lower) {
                Some(t) => t,
                None => return "(floating point error)".to_string(),
            };
            let (un, ud) = match bigfloat_to_ratio(upper) {
                Some(t) => t,
                None => return "(floating point error)".to_string(),
            };
            let lo = BigRational::new(ln_, ld);
            let hi = BigRational::new(un, ud);
            let mid = (&lo + &hi) / BigRational::from_integer(2.into());
            let diff = &hi - &lo;
            let prec = if diff.is_zero() || mid.is_zero() {
                None
            } else {
                let ratio = (&mid / &diff).abs();
                // integer log10
                let mut p: i64 = 0;
                let ten = BigRational::from_integer(10.into());
                let mut v = ratio;
                let one = BigRational::one();
                while v >= ten {
                    v /= &ten;
                    p += 1;
                }
                if v >= one {
                    p += 1;
                }
                Some(p.max(2))
            };
            (mid, prec)
        };
        // The C++ reaches the `FRACTION_FRACTIONAL`/`FRACTION_COMBINED`
        // branches of `Number::print` only for `isRational()` values, so a
        // float is never spelled as a fraction: under `/set fr 2` the
        // reference prints `sqrt(2)` as `1.414213562`, not as the binary
        // fraction its float happens to equal. The midpoint is handed to the
        // rational printer here, so the fractional formats are dropped.
        let po = &match po.number_fraction_format {
            NumberFractionFormat::Fractional
            | NumberFractionFormat::Combined
            | NumberFractionFormat::FractionalFixedDenominator
            | NumberFractionFormat::CombinedFixedDenominator => {
                let mut po2 = po.clone();
                po2.number_fraction_format = NumberFractionFormat::Decimal;
                po2
            }
            _ => po.clone(),
        };
        let mut n = Number::from_rational(mid);
        n.approx = true;
        n.precision = match prec_from_interval {
            Some(p) => {
                if self.precision >= 0 {
                    p.min(self.precision as i64) as i32
                } else {
                    p as i32
                }
            }
            None => self.precision,
        };
        n.print(po)
    }

    /// Integer printing path — Number.cc:11555-11901.
    fn print_integer(&self, z: &BigInt, po: &PrintOptions) -> String {
        let pp = self.compute_print_precision(po);
        let base = po.base.clamp(-1, 36);
        let min_decimals: i64 = if po.use_min_decimals && po.min_decimals > 0 {
            po.min_decimals as i64
        } else {
            0
        };
        let neg = z.sign() == Sign::Minus;
        let mut ivalue = z.magnitude().clone();
        let mut rerun = false;
        let mut exact = true;
        let (mut mpz_str, mut expo, mut decimals): (String, i64, i64);
        let mut precision_base = pp.precision_base;
        let precision = pp.precision;
        loop {
            mpz_str = print_bigint(
                &BigInt::from(ivalue.clone()),
                base,
                false,
                base != crate::options::base::ROMAN_NUMERALS && po.lower_case_numbers,
            );
            let length = mpz_str.len() as i64;
            expo = 0;
            if base == 10 && !po.preserve_format {
                if length == 1 && mpz_str == "0" {
                    expo = 0;
                } else if length > 0
                    && (po.restrict_fraction_length
                        || matches!(
                            po.number_fraction_format,
                            NumberFractionFormat::Decimal | NumberFractionFormat::DecimalExact
                        ))
                {
                    expo = length - 1;
                } else if length > 0 {
                    // unrestricted fraction format: exponent strips trailing zeros
                    for c in mpz_str.bytes().rev() {
                        if c != b'0' {
                            break;
                        }
                        expo += 1;
                    }
                }
                if po.min_exp == exp_mode::PRECISION
                    || (po.min_exp == exp_mode::NONE && (expo > 100_000 || expo < -100_000))
                {
                    let mut precexp = pp.i_precision_base;
                    let prec_add: i64 = if po.use_max_decimals && po.max_decimals < -1 {
                        0
                    } else if precision < 8 {
                        2
                    } else {
                        3
                    };
                    if precexp > precision + prec_add {
                        precexp = precision + prec_add;
                    }
                    if exact && ((expo >= 0 && length - 1 < precexp) || (expo < 0 && expo > -precision)) {
                        if precision_base < length {
                            precision_base = length;
                        }
                        expo = 0;
                    }
                } else if po.min_exp < -1 {
                    expo -= expo % (-po.min_exp as i64);
                    if expo < 0 {
                        expo = 0;
                    }
                } else if po.min_exp != 0 {
                    if expo > -(po.min_exp as i64) && expo < po.min_exp as i64 {
                        expo = 0;
                    }
                } else {
                    expo = 0;
                }
            }
            decimals = expo;
            let nondecimals = length - decimals;

            if !rerun && !ivalue.is_zero() {
                let mut precision2 = precision_base;
                if min_decimals > 0 && min_decimals + nondecimals > precision_base {
                    precision2 = min_decimals + nondecimals;
                    if pp.approx && precision2 > pp.i_precision_base {
                        precision2 = pp.i_precision_base;
                    }
                }
                let decimal_fraction = po.restrict_fraction_length
                    || matches!(
                        po.number_fraction_format,
                        NumberFractionFormat::Decimal | NumberFractionFormat::DecimalExact
                    );
                if po.use_max_decimals
                    && po.max_decimals >= 0
                    && decimals > po.max_decimals as i64
                    && (!pp.approx || (po.max_decimals as i64) + nondecimals < precision2)
                    && base == 10
                    && decimal_fraction
                {
                    let shift = (decimals - po.max_decimals as i64) as u32;
                    let divisor = BigUint::from(10u32).pow(shift);
                    let (quo, rem) = num_integer::Integer::div_rem(&ivalue, &divisor);
                    if !rem.is_zero() {
                        ivalue = round_quotient(quo, &rem, &divisor, neg, po);
                        ivalue *= divisor;
                        exact = false;
                        rerun = true;
                        continue;
                    }
                } else if precision2 < length
                    && (pp.approx || (base == 10 && expo != 0 && decimal_fraction))
                {
                    let shift = (length - precision2) as u32;
                    let divisor = BigUint::from(base as u32).pow(shift);
                    let (quo, rem) = num_integer::Integer::div_rem(&ivalue, &divisor);
                    if !rem.is_zero() {
                        ivalue = round_quotient(quo, &rem, &divisor, neg, po);
                        ivalue *= divisor;
                        exact = false;
                        rerun = true;
                        continue;
                    }
                }
            }
            break;
        }

        let mut dp_added = false;
        decimals = 0;
        if expo > 0 {
            let decimal_fraction = po.restrict_fraction_length
                || matches!(
                    po.number_fraction_format,
                    NumberFractionFormat::Decimal | NumberFractionFormat::DecimalExact
                );
            if decimal_fraction {
                mpz_str.insert(mpz_str.len() - expo as usize, '.');
                dp_added = true;
                decimals = expo;
            } else {
                mpz_str.truncate(mpz_str.len() - expo as usize);
            }
        }

        let decimal_fraction = po.restrict_fraction_length
            || matches!(
                po.number_fraction_format,
                NumberFractionFormat::Decimal | NumberFractionFormat::DecimalExact
            );
        if base != crate::options::base::ROMAN_NUMERALS && decimal_fraction {
            // strip trailing zeroes (respecting min_decimals)
            let bytes = mpz_str.as_bytes();
            let mut pos = bytes.len() as i64 - 1;
            let limit = bytes.len() as i64 + min_decimals - decimals;
            while pos >= limit {
                if bytes[pos as usize] != b'0' {
                    break;
                }
                pos -= 1;
            }
            if pos + 1 < mpz_str.len() as i64 {
                decimals -= mpz_str.len() as i64 - (pos + 1);
                mpz_str.truncate((pos + 1) as usize);
            }
            if exact && min_decimals > decimals {
                if decimals <= 0 {
                    mpz_str.push('.');
                    dp_added = true;
                }
                while min_decimals > decimals {
                    decimals += 1;
                    mpz_str.push('0');
                }
            }
            if mpz_str.ends_with('.') {
                mpz_str.pop();
                dp_added = false;
            }
        }

        if base != crate::options::base::ROMAN_NUMERALS
            && po.show_ending_zeroes
            && (mpz_str.len() > 1 || po.preserve_precision || mpz_str == "0")
            && (!exact || pp.approx)
            && (!po.use_max_decimals || po.max_decimals < 0 || (po.max_decimals as i64) > decimals)
        {
            let mut prec = precision_base;
            prec -= mpz_str.len() as i64;
            if dp_added {
                prec += 1;
            } else if prec > 0 {
                mpz_str.push('.');
            }
            while prec > 0
                && (!po.use_max_decimals || po.max_decimals < 0 || (po.max_decimals as i64) > decimals)
            {
                decimals += 1;
                mpz_str.push('0');
                prec -= 1;
            }
        }

        let mut str = format_number_string(mpz_str, base, po.base_display, neg, true, po);
        add_base_exponent(&mut str, expo, base, po);
        str
    }

    /// Non-integer rational printing — Number.cc:12656-13166.
    fn print_rational(&self, r: &BigRational, po: &PrintOptions) -> String {
        let base = po.base.clamp(-1, 36);
        if base != crate::options::base::ROMAN_NUMERALS
            && matches!(
                po.number_fraction_format,
                NumberFractionFormat::Decimal | NumberFractionFormat::DecimalExact
            )
        {
            self.print_rational_decimal(r, po)
        } else {
            // Fraction display: numerator / denominator.
            let num = Number::from_bigint(r.numer().clone());
            let den = Number::from_bigint(r.denom().clone());
            let mut po2 = po.clone();
            po2.indicate_infinite_series = false;
            let mut str = num.print(&po2);
            if po.spacious {
                str.push(' ');
            }
            str.push('/');
            if po.spacious {
                str.push(' ');
            }
            str.push_str(&den.print(&po2));
            str
        }
    }

    fn print_rational_decimal(&self, r: &BigRational, po: &PrintOptions) -> String {
        let pp = self.compute_print_precision(po);
        let base = po.base.clamp(2, 36);
        let base_big = BigUint::from(base as u32);
        let mut min_decimals: i64 = if po.use_min_decimals && po.min_decimals > 0 {
            po.min_decimals as i64
        } else {
            0
        };
        let precision = pp.precision;
        let mut precision_base = pp.precision_base;
        let d = r.denom().magnitude().clone();
        let mut num = r.numer().magnitude().clone();
        let neg = r.numer().sign() == Sign::Minus;
        let (quo, mut remainder) = num_integer::Integer::div_rem(&num, &d);
        num = quo;
        let mut exact = remainder.is_zero();
        let mut started = false;
        let mut expo: i64 = 0;
        let mut precision2 = precision_base;
        let num_sign = if num.is_zero() { 0 } else { 1 };
        let applied_expo = false;

        if num_sign != 0 {
            let str = num.to_str_radix(base as u32);
            let length = str.len() as i64;
            if base != 10 || po.preserve_format {
                expo = 0;
            } else {
                expo = length - 1;
                if po.min_exp == exp_mode::PRECISION {
                    let mut precexp = pp.i_precision_base;
                    let prec_add: i64 = if po.use_max_decimals && po.max_decimals < -1 {
                        0
                    } else if precision < 8 {
                        2
                    } else {
                        3
                    };
                    if precexp > precision + prec_add {
                        precexp = precision + prec_add;
                    }
                    if (expo > 0 && expo < precexp) || (expo < 0 && expo > -precision) {
                        if expo >= precision_base {
                            precision_base = expo + 1;
                        }
                        if expo >= precision2 {
                            precision2 = expo + 1;
                        }
                        expo = 0;
                    }
                } else if po.min_exp < -1 {
                    expo -= expo % (-po.min_exp as i64);
                    if expo < 0 {
                        expo = 0;
                    }
                } else if po.min_exp != 0 {
                    if expo > -(po.min_exp as i64) && expo < po.min_exp as i64 {
                        expo = 0;
                    }
                } else {
                    expo = 0;
                }
            }
            let decimals = expo;
            let nondecimals = length - decimals;

            if pp.approx && min_decimals + nondecimals > pp.i_precision_base {
                min_decimals = pp.i_precision_base - nondecimals;
            }
            precision2 -= length;
            if min_decimals > 0 {
                let min_l10 = (min_decimals + nondecimals) - length;
                if min_l10 > precision2 {
                    precision2 = min_l10;
                }
            }

            let mut do_div = 0;
            if po.use_max_decimals
                && po.max_decimals >= 0
                && decimals > po.max_decimals as i64
                && (!pp.approx || (po.max_decimals as i64) - decimals < precision2)
            {
                do_div = 1;
            } else if precision2 < 0 && (pp.approx || decimals > min_decimals) {
                do_div = 2;
            }
            if do_div != 0 {
                let shift = if do_div == 1 {
                    (decimals - po.max_decimals as i64) as u32
                } else {
                    (-precision2) as u32
                };
                let div_pre = base_big.pow(shift);
                let i_div = &div_pre * r.denom().magnitude();
                let (i_quo, i_rem) = num_integer::Integer::div_rem(r.numer().magnitude(), &i_div);
                if !i_rem.is_zero() {
                    let rounded = round_quotient_frac(i_quo, &i_rem, &i_div, base, neg, po);
                    num = rounded * &div_pre;
                    exact = false;
                }
                remainder = BigUint::zero();
            }
            started = true;
            if !applied_expo
                && po.use_max_decimals
                && po.max_decimals >= 0
                && precision2 > po.max_decimals as i64 - decimals
            {
                precision2 = po.max_decimals as i64 - decimals;
            }
        }

        // Backup for rerun paths.
        let remainder_bak = remainder.clone();
        let num_bak = num.clone();
        let mut rerun = false;
        let min_decimals_bak = min_decimals;

        let mut str;
        'rational_rerun: loop {
            let mut l10: i64 = 0;
            if rerun {
                num = num_bak.clone();
                remainder = remainder_bak.clone();
            }
            // Long division digit generation.
            while !exact && precision2 > 0 {
                remainder *= &base_big;
                let (q, rem2) = num_integer::Integer::div_rem(&remainder, &d);
                exact = rem2.is_zero();
                if !started {
                    started = !q.is_zero();
                }
                if started {
                    num *= &base_big;
                    num += &q;
                }
                l10 += 1;
                remainder = rem2;
                if started {
                    precision2 -= 1;
                }
            }
            if !exact {
                // Round using the next digit.
                remainder *= &base_big;
                let (q, rem2) = num_integer::Integer::div_rem(&remainder, &d);
                let rounding = get_rounding_mode(po);
                let round_up = if matches!(
                    rounding,
                    RoundingMode::HalfAwayFromZero
                        | RoundingMode::HalfToEven
                        | RoundingMode::HalfToOdd
                        | RoundingMode::HalfTowardZero
                        | RoundingMode::HalfRandom
                        | RoundingMode::HalfUp
                        | RoundingMode::HalfDown
                ) {
                    // compare 2·q_digit against base
                    let two_q = &q * 2u32;
                    let cmp = two_q.cmp(&base_big);
                    cmp == std::cmp::Ordering::Greater
                        || (cmp == std::cmp::Ordering::Equal
                            && (matches!(rounding, RoundingMode::HalfAwayFromZero)
                                || !rem2.is_zero()
                                || (matches!(rounding, RoundingMode::HalfToEven) && num_integer::Integer::is_odd(&num))
                                || (matches!(rounding, RoundingMode::HalfToOdd) && num_integer::Integer::is_even(&num))
                                || (!neg && matches!(rounding, RoundingMode::HalfUp))
                                || (neg && matches!(rounding, RoundingMode::HalfDown))))
                } else {
                    (!neg && matches!(rounding, RoundingMode::Up))
                        || (neg && matches!(rounding, RoundingMode::Down))
                        || matches!(rounding, RoundingMode::AwayFromZero)
                };
                if round_up {
                    num += 1u32;
                }
            }

            if !exact && matches!(po.number_fraction_format, NumberFractionFormat::DecimalExact) && !pp.approx {
                let mut po2 = po.clone();
                po2.number_fraction_format = NumberFractionFormat::Fractional;
                po2.restrict_fraction_length = true;
                return self.print(&po2);
            }

            str = num.to_str_radix(base as u32);
            if base > 10 && !po.lower_case_numbers {
                str = str.to_uppercase();
            }
            if base == 10 && !rerun && !po.preserve_format && !applied_expo {
                expo = str.len() as i64 - l10 - 1;
                if po.min_exp == exp_mode::PRECISION
                    || (po.min_exp == exp_mode::NONE && (expo > 100_000 || expo < -100_000))
                {
                    let mut precexp = pp.i_precision_base;
                    if precision < 8 {
                        if precexp > precision + 2 {
                            precexp = precision + 2;
                        }
                    } else if precexp > precision + 3 {
                        precexp = precision + 3;
                    }
                    if (expo > 0 && expo < precexp) || (expo < 0 && expo > -precision) {
                        if expo >= precision2 {
                            precision2 = expo + 1;
                        }
                        expo = 0;
                    }
                } else if po.min_exp < -1 {
                    if expo < 0 {
                        let mut expo_rem = (-expo) % (-po.min_exp as i64);
                        if expo_rem > 0 {
                            expo_rem = (-po.min_exp as i64) - expo_rem;
                        }
                        expo -= expo_rem;
                        if expo > 0 {
                            expo = 0;
                        }
                    } else if expo > 0 {
                        expo -= expo % (-po.min_exp as i64);
                        if expo < 0 {
                            expo = 0;
                        }
                    }
                } else if po.min_exp != 0 {
                    if expo > -(po.min_exp as i64) && expo < po.min_exp as i64 {
                        expo = 0;
                    }
                } else {
                    expo = 0;
                }
            }
            // max_decimals rerun for pure fractions (num_sign == 0)
            if !rerun
                && num_sign == 0
                && expo <= 0
                && po.use_max_decimals
                && po.max_decimals >= 0
                && l10 + expo > po.max_decimals as i64
            {
                precision2 = po.max_decimals as i64 + (str.len() as i64 - l10 - expo);
                rerun = true;
                exact = false;
                started = false;
                continue 'rational_rerun;
            }
            // min_decimals rerun
            if !rerun
                && !exact
                && num_sign == 0
                && expo <= 0
                && min_decimals_bak > 0
                && l10 + expo < min_decimals_bak
                && (!pp.approx || (str.len() as i64) < pp.i_precision_base)
            {
                min_decimals = min_decimals_bak;
                precision2 = min_decimals + (str.len() as i64 - l10 - expo);
                if pp.approx && precision2 > pp.i_precision_base {
                    precision2 = pp.i_precision_base;
                }
                rerun = true;
                started = false;
                continue 'rational_rerun;
            }

            let mut l10 = l10;
            if expo != 0 && !applied_expo {
                l10 += expo;
            }
            while l10 < 0 {
                str.push('0');
                l10 += 1;
            }
            let show_ending_zeroes = po.show_ending_zeroes;
            if l10 > 0 {
                let mut l10 = str.len() as i64 - l10;
                let mut padd_begin: i64 = 0;
                if l10 < 1 {
                    padd_begin = -l10 + 1;
                    let pad: String = "0".repeat((1 - l10) as usize);
                    str = pad + &str;
                    l10 = 1;
                }
                str.insert(l10 as usize, '.');
                let mut l2: i64 = 0;
                {
                    let bytes = str.as_bytes();
                    while bytes[bytes.len() - 1 - l2 as usize] == b'0' {
                        l2 += 1;
                    }
                }
                let decimals = str.len() as i64 - l10 - 1;
                if (!exact || pp.approx)
                    && show_ending_zeroes
                    && (str.len() as i64) - precision_base - 1 - padd_begin < l2
                {
                    l2 = str.len() as i64 - precision_base - 1 - padd_begin;
                    if po.use_max_decimals
                        && po.max_decimals >= 0
                        && decimals - l2 > po.max_decimals as i64
                    {
                        l2 = decimals - po.max_decimals as i64;
                    }
                    while l2 < 0 {
                        l2 += 1;
                        str.push('0');
                    }
                }
                if l2 > 0 {
                    if min_decimals > 0
                        && (!pp.approx
                            || (!show_ending_zeroes
                                && (str.len() as i64) - pp.i_precision_base - 1 < l2))
                    {
                        if decimals - min_decimals < l2 {
                            l2 = decimals - min_decimals;
                        }
                        if pp.approx && (str.len() as i64) - pp.i_precision_base - 1 > l2 {
                            l2 = str.len() as i64 - pp.i_precision_base - 1;
                        }
                    }
                    if l2 > 0 {
                        str.truncate(str.len() - l2 as usize);
                    }
                }
                if str.ends_with('.') {
                    str.pop();
                }
            }

            let mut decimals: i64 = 0;
            if l10 > 0 {
                let dp = str.find('.');
                if let Some(dp) = dp {
                    decimals = (str.len() - dp - 1) as i64;
                }
            }
            if str.is_empty() {
                str = "0".to_string();
            }
            if !exact
                && str == "0"
                && show_ending_zeroes
                && po.use_max_decimals
                && po.max_decimals >= 0
                && (po.max_decimals as i64) < precision_base
            {
                str.push('.');
                while decimals < po.max_decimals as i64 {
                    str.push('0');
                    decimals += 1;
                }
            }
            if exact && min_decimals > decimals {
                if decimals <= 0 {
                    str.push('.');
                    decimals = 0;
                }
                while decimals < min_decimals {
                    str.push('0');
                    decimals += 1;
                }
            }
            if str.ends_with('.') {
                str.pop();
            }
            break;
        }

        let mut out = format_number_string(str, base, po.base_display, neg, true, po);
        add_base_exponent(&mut out, expo, base, po);
        out
    }
}


/// Decimals needed to show `v` with `sig` significant digits.
///
/// `0.000043` with two significant digits needs six decimals; `2.0` needs
/// one.
/// `floor(log10(x))` for `x > 0` (`integer_log`, Number.cc).
fn ilog10_rat(x: &BigRational) -> i64 {
    let ten = BigRational::from_integer(BigInt::from(10));
    let one = BigRational::one();
    let mut v = x.abs();
    let mut e = 0i64;
    // Bounded: a value outside 10^±20000 cannot come out of the printer.
    let mut guard = 0;
    while v >= ten && guard < 40_000 {
        v /= &ten;
        e += 1;
        guard += 1;
    }
    while v < one && guard < 40_000 {
        v *= &ten;
        e -= 1;
        guard += 1;
    }
    e
}

/// Round a rational to the nearest integer (ties away from zero), the effect
/// of `MPFR_RNDN` followed by `MPFR_ROUND_PO` for the default rounding mode.
fn round_rat(x: &BigRational) -> BigInt {
    let neg = x.is_negative();
    let a = x.abs();
    let half = BigRational::new(BigInt::from(1), BigInt::from(2));
    let r = (a + half).floor().to_integer();
    if neg {
        -r
    } else {
        r
    }
}

/// Insert the decimal point into a digit string that represents
/// `digits · 10^(-decimals)`, padding with zeroes on either side.
fn place_decimal_point(digits: &str, decimals: i64, po: &PrintOptions) -> String {
    if decimals <= 0 {
        let mut s = digits.to_string();
        for _ in 0..(-decimals) {
            s.push('0');
        }
        return s;
    }
    let mut s = digits.to_string();
    let int_len = s.len() as i64 - decimals;
    if int_len < 1 {
        let pad = (1 - int_len) as usize;
        s.insert_str(0, &"0".repeat(pad));
    }
    let at = s.len() - decimals as usize;
    s.insert_str(at, &po.decimalpoint);
    s
}

/// `value ± uncertainty` rendering — the `INTERVAL_DISPLAY_PLUSMINUS` branch
/// of `Number::print` (Number.cc:12470).
///
/// The value is printed to `precision` significant digits and the
/// uncertainty gets the *same* decimal count, measured as the larger
/// distance from the rounded value to either interval end. `precision` is
/// then re-derived from what those two strings turned out to be, which is
/// why the C++ runs the body at most twice ("float_rerun").
fn plus_minus_string(
    lo: &BigRational,
    hi: &BigRational,
    mid: &BigRational,
    po: &PrintOptions,
) -> Option<String> {
    let ten = BigRational::from_integer(BigInt::from(10));
    let neg = mid.is_negative();
    let (lo, hi, mid) = if neg {
        (-hi.clone(), -lo.clone(), mid.abs())
    } else {
        (lo.clone(), hi.clone(), mid.clone())
    };
    let mut precision = crate::context::precision() as i64;
    if precision < 2 {
        precision = 2;
    }
    let mut rerun = false;
    loop {
        let i_log = if mid.is_zero() {
            ilog10_rat(&(&hi - &lo))
        } else {
            ilog10_rat(&mid)
        };
        let decimals = precision - 1 - i_log;
        // scale = 10^decimals
        let scale = if decimals >= 0 {
            ten.pow(decimals.min(10_000) as i32)
        } else {
            BigRational::one() / ten.pow((-decimals).min(10_000) as i32)
        };
        let v = round_rat(&(&mid * &scale));
        let vstr = v.magnitude().to_str_radix(10);
        // Distance from the *rounded* value to each end, larger one wins.
        let vr = BigRational::from_integer(v.clone());
        let d_lo = &vr - &lo * &scale;
        let d_hi = &hi * &scale - &vr;
        let d = if d_lo > d_hi { d_lo } else { d_hi };
        let u = round_rat(&d);
        let ustr = if u.is_zero() {
            String::new()
        } else {
            u.magnitude().to_str_radix(10)
        };
        if !rerun {
            if ustr.len() > vstr.len() {
                let drop = (ustr.len() - vstr.len()) as i64;
                precision -= drop;
                if precision <= 0 {
                    return None;
                }
                rerun = true;
                continue;
            } else if decimals > 0 && ustr.len() > 2 {
                precision = vstr.len() as i64 - decimals;
                let floor = vstr.len() as i64 - ustr.len() as i64 + 2;
                if precision < floor {
                    precision = floor;
                }
                if precision <= 0 {
                    return None;
                }
                rerun = true;
                continue;
            }
        }
        if ustr.is_empty() {
            return None;
        }
        let show_ending_zeroes = vstr.len() > ustr.len() || precision == 2;
        let mut mid_s = place_decimal_point(&vstr, decimals, po);
        let mut unc_s = place_decimal_point(&ustr, decimals, po);
        if !show_ending_zeroes {
            trim_trailing_zeroes(&mut mid_s, &po.decimalpoint);
            trim_trailing_zeroes(&mut unc_s, &po.decimalpoint);
        }
        if neg {
            mid_s.insert_str(0, if po.use_unicode_signs { "\u{2212}" } else { "-" });
        }
        return Some(format!("{mid_s}\u{00B1}{unc_s}"));
    }
}

fn trim_trailing_zeroes(s: &mut String, point: &str) {
    if !s.contains(point) {
        return;
    }
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with(point) {
        s.truncate(s.len() - point.len());
    }
}

/// Round an integer quotient per PrintOptions rounding (used by the integer
/// path). `rem`/`div` describe the discarded fraction rem/div.
fn round_quotient(quo: BigUint, rem: &BigUint, div: &BigUint, neg: bool, po: &PrintOptions) -> BigUint {
    let rounding = get_rounding_mode(po);
    let mut quo = quo;
    let round_up = if matches!(
        rounding,
        RoundingMode::HalfAwayFromZero
            | RoundingMode::HalfToEven
            | RoundingMode::HalfToOdd
            | RoundingMode::HalfTowardZero
            | RoundingMode::HalfRandom
            | RoundingMode::HalfUp
            | RoundingMode::HalfDown
    ) {
        let two_rem = rem * 2u32;
        let cmp = two_rem.cmp(div);
        cmp == std::cmp::Ordering::Greater
            || (cmp == std::cmp::Ordering::Equal
                && (matches!(rounding, RoundingMode::HalfAwayFromZero)
                    || (matches!(rounding, RoundingMode::HalfToEven) && num_integer::Integer::is_odd(&quo))
                    || (matches!(rounding, RoundingMode::HalfToOdd) && num_integer::Integer::is_even(&quo))
                    || (!neg && matches!(rounding, RoundingMode::HalfUp))
                    || (neg && matches!(rounding, RoundingMode::HalfDown))))
    } else {
        (!neg && matches!(rounding, RoundingMode::Up))
            || (neg && matches!(rounding, RoundingMode::Down))
            || matches!(rounding, RoundingMode::AwayFromZero)
    };
    if round_up {
        quo += 1u32;
    }
    quo
}

/// Same as `round_quotient` but the comparison multiplies by the base first
/// (rational path variant, Number.cc:12792).
fn round_quotient_frac(
    quo: BigUint,
    rem: &BigUint,
    div: &BigUint,
    base: i32,
    neg: bool,
    po: &PrintOptions,
) -> BigUint {
    let rounding = get_rounding_mode(po);
    let mut quo = quo;
    let round_up = if matches!(
        rounding,
        RoundingMode::HalfAwayFromZero
            | RoundingMode::HalfToEven
            | RoundingMode::HalfToOdd
            | RoundingMode::HalfTowardZero
            | RoundingMode::HalfRandom
            | RoundingMode::HalfUp
            | RoundingMode::HalfDown
    ) {
        // rem/div × base vs 1/2  ⟺  2·rem·base vs div... careful: original
        // compares (rem/div)·base against base/2 ⟺ rem·2 against div.
        let two_rem_base = rem * (2 * base as u32);
        let cmp = two_rem_base.cmp(&(div * (base as u32)));
        cmp == std::cmp::Ordering::Greater
            || (cmp == std::cmp::Ordering::Equal
                && (matches!(rounding, RoundingMode::HalfAwayFromZero)
                    || (matches!(rounding, RoundingMode::HalfToEven) && num_integer::Integer::is_odd(&quo))
                    || (matches!(rounding, RoundingMode::HalfToOdd) && num_integer::Integer::is_even(&quo))
                    || (!neg && matches!(rounding, RoundingMode::HalfUp))
                    || (neg && matches!(rounding, RoundingMode::HalfDown))))
    } else {
        (!neg && matches!(rounding, RoundingMode::Up))
            || (neg && matches!(rounding, RoundingMode::Down))
            || matches!(rounding, RoundingMode::AwayFromZero)
    };
    if round_up {
        quo += 1u32;
    }
    quo
}

/// floor(log_base(10^prec − 1)): digits available in `base` for `prec`
/// decimal digits.
fn digits_in_base(prec: i64, base: i64) -> i64 {
    if prec <= 0 {
        return prec;
    }
    // 10^prec − 1 has prec×log(10)/log(base) digits; compute via floats with
    // a safety check (values here are small).
    let v = (prec as f64) * (10f64.ln() / (base as f64).ln());
    let f = v.floor();
    if (v - f).abs() < 1e-9 && f > 0.0 {
        // borderline: 10^prec−1 slightly below a power → floor(v) may
        // overcount by one; err on the C++ side by exact check for small prec
        let candidate = f as i64;
        let mut pow = BigUint::one();
        for _ in 0..candidate {
            pow *= base as u32;
        }
        let mut ten = BigUint::one();
        for _ in 0..prec {
            ten *= 10u32;
        }
        ten -= 1u32;
        if pow > ten {
            return candidate - 1;
        }
        return candidate;
    }
    f as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PrintOptions;

    fn p(n: &Number) -> String {
        n.print(&PrintOptions::default())
    }

    #[test]
    fn integers() {
        assert_eq!(p(&Number::from_i64(0)), "0");
        assert_eq!(p(&Number::from_i64(42)), "42");
        assert_eq!(p(&Number::from_i64(-17)), "-17");
        assert_eq!(p(&Number::from_i64(30000)), "30000");
        assert_eq!(p(&Number::from_i64(1234567890)), "1234567890");
    }

    #[test]
    fn big_integer_scientific() {
        // 2^100 = 1267650600228229401496703205376 → rounded to 10 digits
        let mut n = Number::from_i64(2);
        assert!(n.raise(&Number::from_i64(100), true));
        let s = p(&n);
        assert_eq!(s, "1.2676506E30", "2^100 default print, got {s}");
        // qalc CLI enables show_ending_zeroes → 1.267650600E30 (oracle-checked)
        let mut po = PrintOptions::default();
        po.show_ending_zeroes = true;
        assert_eq!(n.print(&po), "1.267650600E30");
    }

    #[test]
    fn simple_fractions_decimal() {
        assert_eq!(p(&Number::from_ints(1, 2, 0)), "0.5");
        assert_eq!(p(&Number::from_ints(1, 4, 0)), "0.25");
        assert_eq!(p(&Number::from_ints(-3, 4, 0)), "-0.75");
        assert_eq!(p(&Number::from_ints(1, 3, 0)), "0.3333333333");
        assert_eq!(p(&Number::from_ints(2, 3, 0)), "0.6666666667");
        assert_eq!(p(&Number::from_ints(1, 7, 0)), "0.1428571429");
    }

    #[test]
    fn small_fractions() {
        assert_eq!(p(&Number::from_ints(1, 1000, 0)), "0.001");
        // 1/10^12 → scientific
        assert_eq!(p(&Number::from_ints(1, 1, -12)), "1E-12");
    }

    #[test]
    fn fraction_format() {
        let mut po = PrintOptions::default();
        po.number_fraction_format = NumberFractionFormat::Fractional;
        let n = Number::from_ints(1, 3, 0);
        assert_eq!(n.print(&po), "1 / 3");
        po.spacious = false;
        assert_eq!(n.print(&po), "1/3");
    }

    #[test]
    fn hex_and_binary() {
        let mut po = PrintOptions::default();
        po.base = 16;
        assert_eq!(Number::from_i64(255).print(&po), "0xFF");
        po.base = 8;
        assert_eq!(Number::from_i64(8).print(&po), "010");
        po.base = 2;
        // 11 = 1011 padded to 8 bits in groups of four
        assert_eq!(Number::from_i64(11).print(&po), "1011");
    }

    #[test]
    fn roman() {
        let mut po = PrintOptions::default();
        po.base = crate::options::base::ROMAN_NUMERALS;
        assert_eq!(Number::from_i64(1974).print(&po), "MCMLXXIV");
        assert_eq!(Number::from_i64(4).print(&po), "IV");
        assert_eq!(Number::from_i64(9).print(&po), "IX");
    }

    #[test]
    fn infinities() {
        let mut n = Number::new();
        n.set_plus_infinity(false, false);
        assert_eq!(p(&n), "+infinity");
        n.set_minus_infinity(false, false);
        assert_eq!(p(&n), "-infinity");
    }

    #[test]
    fn complex_print() {
        let mut n = Number::from_i64(3);
        n.set_imaginary_part(&Number::from_i64(4));
        assert_eq!(p(&n), "3 + 4i");
        let mut m = Number::new();
        m.set_imaginary_part(&Number::from_i64(1));
        assert_eq!(p(&m), "i");
        let mut k = Number::from_i64(1);
        k.set_imaginary_part(&Number::from_i64(-1));
        assert_eq!(p(&k), "1 - i");
    }

    #[test]
    fn float_print_sqrt2() {
        let mut n = Number::from_i64(2);
        assert!(n.sqrt());
        let s = p(&n);
        assert!(
            s.starts_with("1.41421356"),
            "sqrt(2) prints as 1.414213562, got {s}"
        );
    }

    #[test]
    fn pi_print() {
        let mut n = Number::new();
        n.pi();
        let s = p(&n);
        assert!(s.starts_with("3.14159265"), "pi prints as 3.141592654, got {s}");
    }
}

#[cfg(test)]
mod plusminus_tests {
    use crate::options::{IntervalDisplay, ParseOptions, PrintOptions};
    use crate::Number;

    fn pm_options() -> PrintOptions {
        let mut po = PrintOptions::default();
        po.interval_display = IntervalDisplay::PlusMinus;
        po
    }

    fn uncertain(value: &str, unc: &str) -> Number {
        let po = ParseOptions::default();
        let mut n = Number::parse(value, &po);
        n.set_uncertainty(&Number::parse(unc, &po));
        n
    }

    #[test]
    fn value_matches_the_uncertainty_decimals() {
        // Reference: `5+/-1` prints `5.0±1.0` — the uncertainty carries two
        // significant digits and the value takes the same decimal count.
        assert_eq!(uncertain("5", "1").print(&pm_options()), "5.0±1.0");
    }

    #[test]
    fn small_uncertainties_get_more_decimals() {
        assert_eq!(
            uncertain("0.389008", "0.000043").print(&pm_options()),
            "0.389008±0.000043"
        );
    }

    #[test]
    fn an_exact_value_is_unaffected() {
        // No interval, so no ± regardless of the display mode.
        let n = Number::from_i64(5);
        assert_eq!(n.print(&pm_options()), "5");
    }

    #[test]
    fn uncertainty_decimals_follow_the_reference() {
        // Oracle (`qalc -t`): each of these round-trips through `a+/-b`.
        for (v, u, want) in [
            ("2", "3", "2.0±3.0"),
            ("1", "0.5", "1.00±0.50"),
            ("123.456", "0.0007", "123.45600±0.00070"),
            ("9.933832571", "2.008553836", "9.9±2.0"),
        ] {
            assert_eq!(uncertain(v, u).print(&pm_options()), want, "{v}+/-{u}");
        }
    }

    #[test]
    fn a_large_uncertainty_keeps_the_value_digits() {
        // Oracle: `9.18958684+/-44.11001683` prints back unchanged — the
        // uncertainty being wider than the value drops the value's precision
        // by one digit rather than collapsing it to two significant digits.
        assert_eq!(
            uncertain("9.18958684", "44.11001683").print(&pm_options()),
            "9.18958684±44.11001683"
        );
    }

    #[test]
    fn uncertainty_is_measured_from_the_rounded_value() {
        // Oracle: `Ei(3+/-0.3)` under `/set ic 2` is `10.1±2.1`, not
        // `10.1±2.0`: the reference measures the uncertainty as the larger
        // distance from the *displayed* value to either interval end.
        let po = ParseOptions::default();
        let mut n = Number::new();
        assert!(n.set_interval(
            &Number::parse("8.110347415", &po),
            &Number::parse("12.16104137", &po),
            false
        ));
        assert_eq!(n.print(&pm_options()), "10.1±2.1");
    }

    #[test]
    fn a_straddling_imaginary_part_still_shows_its_sign() {
        // Oracle: `(2+/-3)^3.2` under `/set ic 2` is
        // `86±87 - 0.29±0.30i`; the imaginary interval [-0.59, 0.01] is
        // neither positive nor negative but displays as negative.
        let po = ParseOptions::default();
        let mut re = Number::new();
        assert!(re.set_interval(&Number::from_i64(-1), &Number::parse("173", &po), false));
        let mut im = Number::new();
        assert!(im.set_interval(
            &Number::parse("-0.59", &po),
            &Number::parse("0.01", &po),
            false
        ));
        re.set_imaginary_part(&im);
        let mut po2 = pm_options();
        po2.spacious = true;
        assert!(re.print(&po2).contains(" - "), "got {}", re.print(&po2));
    }
}
