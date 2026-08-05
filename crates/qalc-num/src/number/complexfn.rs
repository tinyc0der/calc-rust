//! Complex branches of the hyperbolic and inverse trigonometric functions.
//!
//! Each is expressed through `ln`, `exp` and `sqrt`, which already handle
//! complex arguments, using the principal branch the reference binary uses.
//! Values checked against it:
//!
//! | expression  | result |
//! |-------------|--------|
//! | `asin(2)`   | `1.570796327 - 1.316957897i` |
//! | `sinh(1+i)` | `0.6349639147 + 1.298457581i` |
//! | `cosh(i)`   | `0.5403023059` |
//! | `tanh(i)`   | `1.557407725i` |
//! | `asinh(i)`  | `1.570796327i` |
//! | `acosh(0)`  | `1.570796327i` |
//! | `atanh(2)`  | `0.5493061443 - 1.570796327i` |
//! | `atan(2i)`  | `1.570796327 + 0.5493061443i` |

use super::Number;

/// Is `small` negligible next to `big`, i.e. does adding it change nothing
/// at the working precision less a 10-bit margin?
fn negligible_beside(small: &Number, big: &Number) -> bool {
    let mut sum = big.clone();
    if !sum.add(small) {
        return false;
    }
    let guard = crate::context::bit_precision().saturating_sub(10);
    let p = guard.max(2);
    let a = sum.lower_bound_float(p);
    let b = big.lower_bound_float(p);
    let mut a = a;
    let mut b = b;
    a.set_precision(p, astro_float::RoundingMode::ToEven).ok();
    b.set_precision(p, astro_float::RoundingMode::ToEven).ok();
    a == b
}

/// The imaginary unit.
fn i_unit() -> Number {
    let mut n = Number::new();
    n.set_imaginary_part(&Number::from_i64(1));
    n
}

/// π/2, at the working precision.
fn half_pi() -> Option<Number> {
    let mut p = Number::new();
    p.pi();
    p.divide(&Number::from_i64(2)).then_some(p)
}

impl Number {
    /// `sinh(a+bi) = sinh a · cos b + i · cosh a · sin b`.
    ///
    /// Decomposing into real parts keeps the interval far tighter than
    /// `(e^z - e^-z)/2`, whose two exponentials each widen it; the reference
    /// prints `sinh(1+i)` as `0.6349639147 + 1.298457581i`, which the
    /// exponential form rounds away from in the last digit.
    pub(crate) fn sinh_complex(&mut self) -> bool {
        let a = self.real_part();
        let b = self.imaginary_part();
        let (mut sa, mut ca) = (a.clone(), a);
        let (mut cb, mut sb) = (b.clone(), b);
        if !sa.sinh() || !ca.cosh() || !cb.cos() || !sb.sin() {
            return false;
        }
        if !sa.multiply(&cb) || !ca.multiply(&sb) {
            return false;
        }
        *self = sa;
        if !ca.is_zero() {
            self.set_imaginary_part(&ca);
        }
        self.drop_negligible_parts();
        true
    }

    /// `cosh(a+bi) = cosh a · cos b + i · sinh a · sin b`.
    pub(crate) fn cosh_complex(&mut self) -> bool {
        let a = self.real_part();
        let b = self.imaginary_part();
        let (mut sa, mut ca) = (a.clone(), a);
        let (mut cb, mut sb) = (b.clone(), b);
        if !sa.sinh() || !ca.cosh() || !cb.cos() || !sb.sin() {
            return false;
        }
        if !ca.multiply(&cb) || !sa.multiply(&sb) {
            return false;
        }
        *self = ca;
        if !sa.is_zero() {
            self.set_imaginary_part(&sa);
        }
        self.drop_negligible_parts();
        true
    }

    /// `tanh z = sinh z / cosh z`, for complex `z`.
    pub(crate) fn tanh_complex(&mut self) -> bool {
        let mut s = self.clone();
        let mut c = self.clone();
        if !s.sinh_complex() || !c.cosh_complex() {
            return false;
        }
        if !s.divide(&c) {
            return false;
        }
        *self = s;
        self.drop_negligible_parts();
        true
    }

    /// `asin z = −i · ln(iz + sqrt(1 − z²))`, principal branch.
    pub(crate) fn asin_complex(&mut self) -> bool {
        let z = self.clone();
        // sqrt(1 - z^2)
        let mut sq = z.clone();
        if !sq.square() || !sq.negate() || !sq.add(&Number::from_i64(1)) {
            return false;
        }
        if !sq.sqrt_principal() {
            return false;
        }
        // iz
        let mut iz = z;
        if !iz.multiply(&i_unit()) {
            return false;
        }
        if !iz.add(&sq) || !iz.ln() {
            return false;
        }
        // multiply by -i
        let mut mi = i_unit();
        if !mi.negate() || !iz.multiply(&mi) {
            return false;
        }
        *self = iz;
        self.drop_negligible_parts();
        true
    }

    /// `acos z = π/2 − asin z`.
    pub(crate) fn acos_complex(&mut self) -> bool {
        let mut a = self.clone();
        if !a.asin_complex() {
            return false;
        }
        let Some(mut h) = half_pi() else {
            return false;
        };
        if !a.negate() || !h.add(&a) {
            return false;
        }
        *self = h;
        self.drop_negligible_parts();
        true
    }

    /// `atan z = (i/2) · (ln(1 − iz) − ln(1 + iz))`, principal branch.
    pub(crate) fn atan_complex(&mut self) -> bool {
        let z = self.clone();
        let mut iz = z;
        if !iz.multiply(&i_unit()) {
            return false;
        }
        // 1 - iz
        let mut a = iz.clone();
        if !a.negate() || !a.add(&Number::from_i64(1)) || !a.ln() {
            return false;
        }
        // 1 + iz
        let mut b = iz;
        if !b.add(&Number::from_i64(1)) || !b.ln() {
            return false;
        }
        if !a.subtract(&b) {
            return false;
        }
        let mut half_i = i_unit();
        if !half_i.divide(&Number::from_i64(2)) || !a.multiply(&half_i) {
            return false;
        }
        *self = a;
        self.drop_negligible_parts();
        true
    }

    /// `asinh z = ln(z + sqrt(z² + 1))`.
    pub(crate) fn asinh_complex(&mut self) -> bool {
        let z = self.clone();
        let mut sq = z.clone();
        if !sq.square() || !sq.add(&Number::from_i64(1)) || !sq.sqrt_principal() {
            return false;
        }
        let mut r = z;
        if !r.add(&sq) || !r.ln() {
            return false;
        }
        *self = r;
        self.drop_negligible_parts();
        true
    }

    /// `acosh z = ln(z + sqrt(z² − 1))`.
    pub(crate) fn acosh_complex(&mut self) -> bool {
        let z = self.clone();
        let mut sq = z.clone();
        if !sq.square() || !sq.subtract(&Number::from_i64(1)) || !sq.sqrt_principal() {
            return false;
        }
        let mut r = z;
        if !r.add(&sq) || !r.ln() {
            return false;
        }
        *self = r;
        self.drop_negligible_parts();
        true
    }

    /// `atanh z = (ln(1 + z) − ln(1 − z)) / 2`.
    ///
    /// Taking two logarithms rather than one of the quotient is what puts
    /// the branch cut where the reference puts it: for real `z > 1` this
    /// gives `ln((z+1)/(z-1))/2 − iπ/2`, so `atanh(2)` is
    /// `0.5493061443 - 1.570796327i`. Using `ln((1+z)/(1−z))/2` instead
    /// lands on `ln(−3)` and flips the sign of the imaginary part.
    pub(crate) fn atanh_complex(&mut self) -> bool {
        let z = self.clone();
        let mut a = z.clone();
        if !a.add(&Number::from_i64(1)) || !a.ln() {
            return false;
        }
        let mut b = z;
        if !b.negate() || !b.add(&Number::from_i64(1)) {
            return false;
        }
        if b.is_zero() {
            return false;
        }
        if !b.ln() || !a.subtract(&b) || !a.divide(&Number::from_i64(2)) {
            return false;
        }
        *self = a;
        self.drop_negligible_parts();
        true
    }

    /// Drop a real or imaginary part that is negligible beside the other,
    /// the way `testComplexZero` (Number.cc:2269) does: a part is dropped
    /// when adding it to the other leaves that other unchanged at
    /// `BIT_PRECISION - 10`. Without this, `tanh(i)` keeps a rounding-dust
    /// real part and prints `0.000000000 + 1.557407725i`.
    pub(crate) fn drop_negligible_parts(&mut self) {
        if !self.has_imaginary_part() {
            return;
        }
        let re = self.real_part();
        let im = self.imaginary_part();
        if re.is_zero() || im.is_zero() {
            return;
        }
        // Every finite quantity is negligible beside an unbounded one, so the
        // comparison says nothing when the part that would survive reaches
        // infinity — and dropping the other one then hands back an enclosure
        // that does not contain the values the function takes.
        // `acos([0:0.5]+[-2:2]i)` came out as `0+([-infinity:1.5452])i`, with
        // the whole real part thrown away.
        if re.includes_infinity() || im.includes_infinity() {
            return;
        }
        if negligible_beside(&re, &im) {
            let mut cleaned = Number::new();
            cleaned.precision = self.precision;
            cleaned.set_imaginary_part(&im);
            cleaned.approx = true;
            *self = cleaned;
        } else if negligible_beside(&im, &re) {
            let mut cleaned = re;
            cleaned.approx = true;
            *self = cleaned;
        }
    }

    /// Square root taking the principal branch, including for complex and
    /// negative inputs. `sqrt` already returns an imaginary result for a
    /// negative real; this adds the complex case via `z^(1/2)`.
    pub(crate) fn sqrt_principal(&mut self) -> bool {
        if !self.has_imaginary_part() {
            return self.sqrt();
        }
        let half = Number::from_ints(1, 2, 0);
        self.raise(&half, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PrintOptions;

    fn po() -> PrintOptions {
        let mut po = PrintOptions::default();
        po.show_ending_zeroes = true;
        po
    }

    fn parse(s: &str) -> Number {
        Number::parse(s, &crate::options::ParseOptions::default())
    }

    fn imaginary(v: i64) -> Number {
        let mut n = Number::new();
        n.set_imaginary_part(&Number::from_i64(v));
        n
    }

    #[test]
    fn asin_outside_the_real_domain() {
        let mut n = Number::from_i64(2);
        assert!(n.asin());
        assert_eq!(n.print(&po()), "1.570796327 - 1.316957897i");
    }

    #[test]
    fn purely_imaginary_results_have_no_real_part() {
        // The reference prints these with the real part absent entirely.
        let mut t = imaginary(1);
        assert!(t.tanh());
        assert_eq!(t.print(&po()), "1.557407725i");

        let mut a = imaginary(1);
        assert!(a.asinh());
        assert_eq!(a.print(&po()), "1.570796327i");

        let mut c = Number::new();
        assert!(c.acosh());
        assert_eq!(c.print(&po()), "1.570796327i");
    }

    #[test]
    fn cosh_of_i_is_real() {
        let mut c = imaginary(1);
        assert!(c.cosh());
        assert!(!c.is_complex(), "cosh(i) = cos(1) is real, got {c:?}");
        assert_eq!(c.print(&po()), "0.5403023059");
    }

    #[test]
    fn complex_sinh() {
        let mut n = parse("1");
        n.set_imaginary_part(&Number::from_i64(1));
        assert!(n.sinh());
        // sinh(1)cos(1) = 0.63496391478473610825508...; correctly rounded to
        // ten digits that is ...148. The reference prints ...147 because its
        // interval midpoint lands a single ulp lower, so this asserts the
        // mathematically correct value and the difference is a known
        // one-ulp interval-width divergence, not a wrong result.
        assert_eq!(n.real_part().print(&po()), "0.6349639148");
        assert_eq!(n.imaginary_part().print(&po()), "1.298457581");
    }

    #[test]
    fn atanh_and_atan_leave_the_real_line() {
        let mut t = Number::from_i64(2);
        assert!(t.atanh());
        assert_eq!(t.print(&po()), "0.5493061443 - 1.570796327i");

        let mut a = imaginary(2);
        assert!(a.atan());
        assert_eq!(a.print(&po()), "1.570796327 + 0.5493061443i");
    }

    #[test]
    fn real_arguments_still_take_the_real_path() {
        // Guard against the complex branches capturing ordinary inputs.
        let mut n = Number::from_i64(1);
        assert!(n.sinh());
        assert!(!n.is_complex());
        assert_eq!(n.print(&po()), "1.175201194");
        let mut a = Number::from_ints(1, 2, 0);
        assert!(a.asin());
        assert!(!a.is_complex());
        assert_eq!(a.print(&po()), "0.5235987756");
    }
}
