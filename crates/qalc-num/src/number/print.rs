//! Number printing — port of `Number::print` (Number.cc:10681-13169) and its
//! helpers `printMPZ`, `format_number_string`, `add_base_exponent`.
//!
//! This pass covers the paths exercised with default options: exact
//! integers, exact rationals (decimal + fraction formats), infinities,
//! complex join, and floats via their exact binary-rational value.
//! TODO(port): interval displays other than SIGNIFICANT_DIGITS/MIDPOINT,
//! preserve_format ellipses, indicate_infinite_series, two's complement,
//! special bases, BCD, bijective-26, IEEE-float bases.

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

/// `BASE_IS_SEXAGESIMAL(x)` (includes.h:337) — the degree/latitude/longitude
/// family, which shares a printer with `BASE_TIME` but not its formatting.
pub fn is_sexagesimal_base(b: i32) -> bool {
    use crate::options::base;
    (base::SEXAGESIMAL..=base::SEXAGESIMAL_3).contains(&b)
        || (base::LATITUDE..=base::LONGITUDE_2).contains(&b)
}

/// The base as a real number, for the bases `Number::print` handles by
/// repeated division rather than by `to_str_radix` (Number.cc:10840): the
/// named irrational bases and any custom base that is not an integer in
/// 2..=62.
fn real_base_of(po: &PrintOptions) -> Option<Number> {
    use crate::options::base;
    let mut b = match po.base {
        base::PI => {
            let mut n = Number::new();
            n.pi();
            n
        }
        base::E => {
            let mut n = Number::new();
            n.e();
            n
        }
        base::SQRT2 => {
            let mut n = Number::from_i64(2);
            n.sqrt();
            n
        }
        base::GOLDEN_RATIO => {
            let mut n = Number::from_i64(5);
            n.sqrt();
            n.add(&Number::from_i64(1));
            n.divide(&Number::from_i64(2));
            n
        }
        base::CUSTOM => po.custom_base.clone()?,
        _ => return None,
    };
    // An integer base in the ordinary range goes through the digit-string
    // path instead, which is both exact and much cheaper.
    if b.is_integer() && !b.is_less_than_i64(2) && b.is_less_than_i64(63) {
        return None;
    }
    b.set_approximate(b.is_approximate());
    Some(b)
}

/// The midpoint of a value's interval, as a *point* value
/// (`intervalToMidValue`).
///
/// Interval arithmetic has to be off while this is computed: adding the two
/// endpoints with directed rounding would hand back another interval, and
/// flooring `[1-e, 1+e]` gives `[0, 1]` — which then displays as 0.5 rather
/// than the 1 the caller is asking for.
fn interval_midpoint(n: &Number) -> Number {
    let saved = context::create_interval();
    context::set_create_interval(false);
    let lo = n.lower_end_point();
    let hi = n.upper_end_point();
    let mut mid = lo;
    let ok = mid.add(&hi) && mid.divide(&Number::from_i64(2));
    context::set_create_interval(saved);
    if ok {
        mid
    } else {
        n.clone()
    }
}

/// The digit an interval determines, or `None` when it does not determine one.
///
/// For a point value this is plain truncation. For an interval the C++ floors
/// the midpoint, adds one, and decrements again unless the upper endpoint has
/// really reached that next integer — which pins `[1-e, 1+e]` to 1 rather than
/// letting it truncate to 0.
fn interval_digit(q: &Number) -> Option<i64> {
    let lo = q.lower_end_point();
    let hi = q.upper_end_point();
    let mut width = hi.clone();
    width.subtract(&lo);
    if !width.is_less_than(&Number::from_i64(1)) {
        return None;
    }
    let mut d = interval_midpoint(q);
    d.floor();
    let mut next = d.clone();
    next.add(&Number::from_i64(1));
    if hi.is_less_than(&next) {
        d.to_i64()
    } else {
        next.to_i64()
    }
}

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
        // IEEE-754 bit-string bases print the encoding, in groups of four
        // (`52.345 to float` is `0100 0010 0101 0001 0110 0001 0100 1000`).
        if let Some(bits) = ieee_width(po.base) {
            if let Some(s) = crate::number::ieee::to_float(self, bits, 0) {
                return group_bits(&s);
            }
            return "(floating point error)".to_string();
        }
        // Complex numbers: join real and imaginary parts.
        if self.has_imaginary_part() {
            return self.print_complex(po);
        }
        if (is_sexagesimal_base(po.base) || po.base == crate::options::base::TIME)
            && !self.is_infinite(false)
        {
            return self.print_sexagesimal(po);
        }
        if let Some(base) = real_base_of(po) {
            if self.is_real() && !self.is_infinite(false) {
                return self.print_real_base(po, &base);
            }
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

    /// The non-integer-base branch of `Number::print` (Number.cc:10840).
    ///
    /// Digits are produced most-significant first by repeatedly dividing by
    /// the largest remaining power of the base, so `sqrt(32) to base sqrt(2)`
    /// is `100000` — five factors of sqrt(2) and nothing left over.
    ///
    /// The values involved are intervals, and a digit right on a boundary
    /// (`5 / sqrt(2)^0` is *exactly* 1, but only to within the interval) would
    /// truncate to the digit below. The C++ handles that by flooring the
    /// midpoint, adding one, and backing off only when the upper endpoint is
    /// genuinely below; this does the same.
    ///
    /// TODO(port): negative and complex bases, the escaped-digit form for
    /// bases above 62, and `show_ending_zeroes` padding.
    fn print_real_base(&self, po: &PrintOptions, base: &Number) -> String {
        let decimal = |n: &Number| {
            let mut po2 = po.clone();
            po2.base = 10;
            po2.custom_base = None;
            n.print(&po2)
        };
        // Negative, complex, and shallow bases are not supported.
        if base.has_imaginary_part() || !base.is_greater_than(&Number::from_i64(1)) {
            return decimal(self);
        }
        if self.is_zero() {
            return "0".to_string();
        }
        let precision = context::precision() as i64;
        // How many digits the working precision is worth in this base:
        // floor(log_base(10^precision - 1)).
        let precision_base = {
            let mut p = Number::from_i64(10);
            p.raise(&Number::from_i64(precision), true);
            p.subtract(&Number::from_i64(1));
            p.log(base);
            p.floor();
            p.to_i64().unwrap_or(precision).max(1)
        };
        // Digits above 36 need a case distinction to stay unambiguous.
        let b_case = base.is_greater_than(&Number::from_i64(36));

        let neg = self.is_negative();
        let mut nr = self.clone();
        if neg {
            nr.negate();
        }
        let mut exponent = {
            let mut l = nr.clone();
            l.log(base);
            let mut l = interval_midpoint(&l);
            l.floor();
            match l.to_i64() {
                Some(v) if (-1000..=1000).contains(&v) => v,
                _ => return decimal(self),
            }
        };

        let mut digits: Vec<i64> = Vec::new();
        let mut point_at = 0usize;
        let mut have_point = false;
        let mut exact = false;
        if exponent < 0 {
            point_at = 0;
            have_point = true;
        }
        loop {
            let mut base_pow = base.clone();
            base_pow.raise(&Number::from_i64(exponent), true);
            if !have_point && exponent < 0 {
                if digits.len() as i64 >= precision_base {
                    break;
                }
                point_at = digits.len();
                have_point = true;
            }
            if have_point && digits.len() as i64 >= precision_base {
                break;
            }
            let mut quotient = nr.clone();
            if !quotient.divide(&base_pow) {
                return decimal(self);
            }
            let Some(digit) = interval_digit(&quotient) else {
                // The interval no longer pins the digit down; every further
                // digit would be noise.
                break;
            };
            digits.push(digit);
            let mut taken = Number::from_i64(digit);
            taken.multiply(&base_pow);
            nr.subtract(&taken);
            if nr.is_zero() {
                while exponent > 0 {
                    digits.push(0);
                    exponent -= 1;
                }
                exact = true;
                break;
            }
            exponent -= 1;
        }
        let _ = exact;
        // Trailing zeroes after the point carry no information.
        if have_point {
            while digits.len() > point_at && digits.last() == Some(&0) {
                digits.pop();
            }
            if digits.len() <= point_at {
                have_point = false;
            }
        }

        let mut str = String::new();
        if have_point && point_at == 0 {
            str.push('0');
        }
        for (index, digit) in digits.iter().enumerate() {
            if have_point && index == point_at {
                str.push_str(po.decimalpoint());
            }
            let c = *digit;
            if c <= 9 {
                str.push((b'0' + c as u8) as char);
            } else if b_case {
                if c < 36 {
                    str.push((b'A' + (c - 10) as u8) as char);
                } else {
                    str.push((b'a' + (c - 36) as u8) as char);
                }
            } else if po.lower_case_numbers {
                str.push((b'a' + (c - 10) as u8) as char);
            } else {
                str.push((b'A' + (c - 10) as u8) as char);
            }
        }
        if str.is_empty() {
            str.push('0');
        }
        if neg {
            str.insert_str(0, if po.use_unicode_signs { "\u{2212}" } else { "-" });
        }
        str
    }

    /// The `BASE_IS_SEXAGESIMAL || BASE_TIME` branch of `Number::print`
    /// (Number.cc:11251).
    ///
    /// The value is split left to right into three sections — whole units,
    /// sixtieths, and thirty-six-hundredths — and each section is printed in
    /// base 10. `52.34 to sexa` is `52°20′24″`; `(19+1/60) to time` is
    /// `19:01`, the seconds omitted because time format hides a zero third
    /// section while degree formats always show it.
    fn print_sexagesimal(&self, po: &PrintOptions) -> String {
        use crate::options::base;
        let is_time = po.base == base::TIME;
        let two_part = matches!(
            po.base,
            base::SEXAGESIMAL_2 | base::LATITUDE_2 | base::LONGITUDE_2
        );
        // Time and the one-letter latitude/longitude forms pad each section
        // to two digits; the plain degree form does not.
        let pad_sections =
            matches!(po.base, base::TIME | base::LATITUDE | base::LONGITUDE);
        // `PRECISION_DIGITS` (Number.cc:10679).
        let precision_digits = if po.use_max_decimals && po.max_decimals < -1 {
            (-po.max_decimals).min(crate::context::precision())
        } else {
            crate::context::precision()
        };

        let mut nr = self.clone();
        match po.interval_display {
            IntervalDisplay::Lower => nr = nr.lower_end_point(),
            IntervalDisplay::Upper => nr = nr.upper_end_point(),
            _ => {}
        }
        let neg = nr.is_negative();
        if neg {
            nr.negate();
        }

        let mut po2 = po.clone();
        po2.base = 10;
        po2.number_fraction_format = NumberFractionFormat::Decimal;
        po2.show_ending_zeroes = false;

        let mut nr1 = nr.clone();
        nr1.trunc();
        let mut nr2 = nr.clone();
        nr2.frac();

        let mut str3 = String::new();
        if two_part {
            nr2.multiply_i64(60);
        } else {
            nr2.set_approximate(false);
            nr2.multiply_i64(60);
            nr2.trunc();

            let mut nr3 = nr.clone();
            nr3.frac();
            nr3.multiply_i64(60);
            nr3.frac();
            nr3.multiply_i64(60);
            if po.base == base::SEXAGESIMAL_3 && !nr3.is_integer() {
                nr3.round(po.rounding);
            }
            // A zero third section is dropped in time format but kept in the
            // degree formats.
            if !nr3.is_zero() || is_sexagesimal_base(po.base) {
                let mut po3 = po2.clone();
                if nr1.is_zero() && nr2.is_zero() {
                    po3.min_exp = precision_digits;
                } else {
                    po3.min_exp = crate::options::exp_mode::NONE;
                    if po3.max_decimals < 0 || !po3.use_max_decimals {
                        po3.max_decimals = precision_digits;
                        po3.use_max_decimals = true;
                    }
                }
                str3 = nr3.print(&po3);
                // Rounding can push the third section to a full 60; carry it.
                if str3.starts_with("60") {
                    str3.replace_range(0..2, "0");
                    nr2.add_i64(1);
                    if nr2.equals_i64(60) {
                        nr2 = Number::new();
                        nr1.add_i64(1);
                    }
                }
            }
        }

        if (po.min_exp > 0 && po.min_exp < precision_digits)
            || (po.min_exp < 0 && -po.min_exp < precision_digits)
        {
            po2.min_exp = crate::options::exp_mode::PRECISION;
        } else {
            po2.min_exp = po.min_exp;
        }
        let mut str = nr1.print(&po2);
        // An exponent in the first section means the value is too large for
        // sexagesimal notation to be readable; fall back to base 10.
        if str.contains('E') || str.contains('^') {
            let mut po_dec = po.clone();
            po_dec.base = 10;
            po_dec.number_fraction_format = NumberFractionFormat::Decimal;
            return self.print(&po_dec);
        }
        if !is_time {
            str.push(if po.use_unicode_signs { '\u{b0}' } else { 'o' });
        } else {
            str.push(':');
        }
        if pad_sections && nr2.is_less_than_i64(10) {
            str.push('0');
        }
        if two_part {
            let mut po3 = po2.clone();
            if nr1.is_zero() {
                po3.min_exp = precision_digits;
            } else {
                po3.min_exp = crate::options::exp_mode::NONE;
                if po3.max_decimals < 0 || !po3.use_max_decimals {
                    po3.max_decimals = precision_digits;
                    po3.use_max_decimals = true;
                }
            }
            str.push_str(&nr2.print(&po3));
        } else {
            po2.min_exp = crate::options::exp_mode::NONE;
            str.push_str(&nr2.numerator().print(&po2));
        }
        if !is_time {
            str.push_str(if po.use_unicode_signs { "\u{2032}" } else { "'" });
        }
        if !str3.is_empty() {
            if is_time {
                str.push(':');
            }
            if pad_sections && (str3.chars().count() == 1 || str3.find(&po.decimalpoint) == Some(1))
            {
                str.push('0');
            }
            str.push_str(&str3);
            if !is_time {
                str.push_str(if po.use_unicode_signs { "\u{2033}" } else { "\"" });
            }
        }
        match po.base {
            base::LONGITUDE | base::LONGITUDE_2 => str.push(if neg { 'W' } else { 'E' }),
            base::LATITUDE | base::LATITUDE_2 => str.push(if neg { 'S' } else { 'N' }),
            _ if neg => {
                str.insert_str(0, if po.use_unicode_signs { "\u{2212}" } else { "-" });
            }
            _ => {}
        }
        str
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

    /// Fix the bit width of a binary (or hexadecimal-two's-complement)
    /// integer — Number.cc:11594-11657.
    ///
    /// Two jobs, both of which re-enter `print_integer` with `binary_bits`
    /// pinned so that `format_number_string` zero-pads to a whole width:
    ///
    /// * a negative value is replaced by `value + 2^bits`, its two's
    ///   complement, and printed unsigned — `-5 to bin8` is `1111 1011`,
    ///   not the sign-magnitude `-0000 0101` (which is not a bit pattern
    ///   at all, and would read back as `-5`'s complement of itself);
    /// * a value too wide for the requested width, or one printed with no
    ///   width at all, gets the next power-of-two width of at least 8, so
    ///   `5 to bin` is `0000 0101` and `256 to bin8` widens to
    ///   `0000 0001 0000 0000`.
    ///
    /// Returns `None` when the value needs no such treatment.
    fn print_integer_binary_bits(
        &self,
        z: &BigInt,
        po: &PrintOptions,
        neg: bool,
    ) -> Option<String> {
        if po.base != 2 && !(po.base == 16 && po.hexadecimal_twos_complement) {
            return None;
        }
        if (po.base == 16 || po.twos_complement) && neg {
            // The width has to hold `value + 1`: -128 fits in 8 bits
            // (`1000 0000`) because -128+1 = -127 is 7 bits wide.
            let mut bits = po.binary_bits as u64;
            let needed = integer_length(&(z + 1)) + 1;
            if bits == 0 || bits < needed {
                bits = round_up_bits(needed);
            }
            let twos = z + (BigInt::one() << bits);
            let mut po2 = po.clone();
            po2.twos_complement = false;
            po2.binary_bits = bits.min(u32::MAX as u64) as u32;
            let mut nr = self.clone();
            nr.value = RealValue::Rational(BigRational::from_integer(twos.clone()));
            return Some(nr.print_integer(&twos, &po2));
        }
        let len = integer_length(z);
        if po.binary_bits == 0 || (po.binary_bits as u64) < len {
            let mut po2 = po.clone();
            po2.binary_bits = round_up_bits(len + 1).min(u32::MAX as u64) as u32;
            return Some(self.print_integer(z, &po2));
        }
        None
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
        if let Some(s) = self.print_integer_binary_bits(z, po, neg) {
            return s;
        }
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


/// Total width for the IEEE-754 bases, or `None` for an ordinary base.
fn ieee_width(base: i32) -> Option<u32> {
    use crate::options::base as b;
    Some(match base {
        b::FP16 => 16,
        b::FP32 => 32,
        b::FP64 => 64,
        b::FP80 => 80,
        b::FP128 => 128,
        _ => return None,
    })
}

/// `Number::integerLength()` (Number.cc:3106) — `mpz_sizeinbase(_, 2)`,
/// which GMP defines as 1 for zero, not 0.
fn integer_length(z: &BigInt) -> u64 {
    z.magnitude().bits().max(1)
}

/// Round a bit count up to a printable width: the next power of two, never
/// below 8 (Number.cc:11604).
fn round_up_bits(bits: u64) -> u64 {
    bits.max(8).next_power_of_two()
}

/// Space-separated groups of four bits.
fn group_bits(bits: &str) -> String {
    let mut out = String::with_capacity(bits.len() + bits.len() / 4);
    for (i, c) in bits.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
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
    fn irrational_number_bases() {
        use crate::options::base;
        let mut root2 = Number::from_i64(2);
        assert!(root2.sqrt());
        let mut po = PrintOptions::default();
        po.base = base::CUSTOM;
        po.custom_base = Some(root2.clone());

        // Oracle values.
        assert_eq!(Number::from_i64(1).print(&po), "1");
        assert_eq!(Number::from_i64(2).print(&po), "100");
        assert_eq!(Number::from_i64(5).print(&po), "10001");
        assert_eq!(Number::from_i64(8).print(&po), "1000000");
        // The value *is* the base: the leading digit sits right on the
        // interval boundary, which plain truncation would round down to 0.
        assert_eq!(root2.print(&po), "10");
        let mut root32 = Number::from_i64(32);
        assert!(root32.sqrt());
        assert_eq!(root32.print(&po), "100000");
    }

    #[test]
    fn sexagesimal_and_time_sections() {
        use crate::options::base;
        let mut po = PrintOptions::default();

        // 52.34° = 52°20′24″ (oracle, after `/set unicode 1`).
        po.base = base::SEXAGESIMAL;
        po.use_unicode_signs = true;
        let n = Number::parse("52.34", &Default::default());
        assert_eq!(n.print(&po), "52°20′24″");
        po.use_unicode_signs = false;
        assert_eq!(n.print(&po), "52o20'24\"");

        // Time format hides a zero third section and pads to two digits.
        po.base = base::TIME;
        let mut t = Number::from_i64(1);
        assert!(t.divide(&Number::from_i64(60)));
        assert!(t.add(&Number::from_i64(19)));
        assert_eq!(t.print(&po), "19:01");

        // ...but shows it when it is non-zero, padded and unrounded.
        let n2 = Number::parse("19.0166666667", &Default::default());
        assert_eq!(n2.print(&po), "19:01:00.00000012");

        // The sign goes in front for time and degrees, and becomes a compass
        // letter for the latitude/longitude bases.
        let mut neg = n.clone();
        neg.negate();
        po.base = base::SEXAGESIMAL;
        assert_eq!(neg.print(&po), "-52o20'24\"");
        po.base = base::LATITUDE;
        assert_eq!(neg.print(&po), "52o20'24\"S");
        po.base = base::LONGITUDE;
        assert_eq!(n.print(&po), "52o20'24\"E");
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
        assert_eq!(Number::from_i64(11).print(&po), "0000 1011");
    }

    /// Every value here is `printf 'EXPR\n' | qalc -t +u8`'s.
    #[test]
    fn binary_widths_are_whole_bytes() {
        let mut po = PrintOptions::default();
        po.base = 2;
        let b = |n: i64, po: &PrintOptions| Number::from_i64(n).print(po);
        assert_eq!(b(5, &po), "0000 0101");
        assert_eq!(b(255, &po), "0000 0000 1111 1111");
        assert_eq!(b(0, &po), "0000 0000");
        assert_eq!(b(11, &po), "0000 1011");
        // A requested width too narrow for the value is widened, not honoured.
        po.binary_bits = 8;
        assert_eq!(b(256, &po), "0000 0001 0000 0000");
        assert_eq!(b(5, &po), "0000 0101");
        po.binary_bits = 4;
        assert_eq!(b(15, &po), "1111");
        po.binary_bits = 2;
        assert_eq!(b(7, &po), "0000 0111");
    }

    /// Negative binary integers are two's complement, not sign-magnitude:
    /// `-0000 0001` is not a bit pattern at all. Oracle values again.
    #[test]
    fn negative_binaries_are_twos_complement() {
        let mut po = PrintOptions::default();
        po.base = 2;
        let b = |n: i64, po: &PrintOptions| Number::from_i64(n).print(po);
        po.binary_bits = 8;
        assert_eq!(b(-1, &po), "1111 1111");
        assert_eq!(b(-5, &po), "1111 1011");
        // -128 still fits in 8 bits; -129 and -256 do not, and widen to 16.
        assert_eq!(b(-128, &po), "1000 0000");
        assert_eq!(b(-129, &po), "1111 1111 0111 1111");
        assert_eq!(b(-256, &po), "1111 1111 0000 0000");
        po.binary_bits = 16;
        assert_eq!(b(-255, &po), "1111 1111 0000 0001");
        po.binary_bits = 32;
        assert_eq!(b(-1, &po), "1111 1111 1111 1111 1111 1111 1111 1111");
        po.binary_bits = 4;
        assert_eq!(b(-3, &po), "1101");
        po.binary_bits = 0;
        assert_eq!(b(-1, &po), "1111 1111");
        assert_eq!(b(-5, &po), "1111 1011");
        assert_eq!(
            b(-65536, &po),
            "1111 1111 1111 1111 0000 0000 0000 0000"
        );
        // With two's complement off the sign-magnitude form is kept.
        po.twos_complement = false;
        assert_eq!(b(-5, &po), "-0000 0101");
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
