//! Transcendental functions — port of the corresponding `Number.cc` methods
//! backed by astro-float (replacing mpfr_exp/log/trig...).
//!
//! Interval propagation: monotone functions map [l,u] → [f(l)↓, f(u)↑];
//! sin/cos locate contained extrema. Point mode uses round-to-nearest.
//! TODO(port): gamma/digamma/zeta/erf/bessel/polylog/expint/… (no astro-float
//! equivalents; need hand-rolled series — see special-functions plan).

use super::{Number, RealValue};
use crate::context;
use astro_float::{BigFloat, Consts, RoundingMode};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

type MonoFn = fn(&BigFloat, usize, RoundingMode, &mut Consts) -> BigFloat;

impl Number {
    /// Apply a monotone-increasing function to the real interval.
    /// Returns false if any resulting bound is NaN (domain error).
    fn apply_monotone(&mut self, f: MonoFn) -> bool {
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            context::with_consts(|cc| {
                (f(&al, p, RoundingMode::Down, cc), f(&au, p, RoundingMode::Up, cc))
            })
        } else {
            let v = context::with_consts(|cc| f(&al, p, RoundingMode::ToEven, cc));
            (v.clone(), v)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `ln()` — natural logarithm. Complex for negative reals.
    pub fn ln(&mut self) -> bool {
        if self.has_imaginary_part() {
            // ln(z) = ln|z| + i·arg(z)
            let mut re = self.clone();
            if !re.abs() || !re.ln() {
                return false;
            }
            let mut im = self.clone();
            if !im.arg() {
                return false;
            }
            *self = re;
            self.set_imaginary_part(&im);
            return true;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            return false;
        }
        if self.is_zero() {
            if self.is_imag_part {
                return false;
            }
            self.set_minus_infinity(true, false);
            self.approx = true;
            return true;
        }
        if self.is_one() {
            self.clear(true);
            return true;
        }
        if self.real_part_is_negative() {
            if self.is_imag_part {
                return false;
            }
            // ln(-x) = ln(x) + πi
            let mut pos = self.clone();
            if !pos.negate() || !pos.ln() {
                return false;
            }
            let mut pi = Number::new();
            pi.pi();
            *self = pos;
            self.set_imaginary_part(&pi);
            return true;
        }
        if !self.is_nonzero() {
            return false; // interval containing zero
        }
        self.apply_monotone(|x, p, rm, cc| x.ln(p, rm, cc))
    }

    /// `log(base)`.
    pub fn log(&mut self, base: &Number) -> bool {
        if base.is_zero() || base.is_one() || !base.is_real() {
            return false;
        }
        // ln(x)/ln(base)
        let mut lb = base.clone();
        if !lb.ln() || !self.ln() {
            return false;
        }
        self.divide(&lb)
    }

    /// `exp()`.
    pub fn exp(&mut self) -> bool {
        if self.has_imaginary_part() {
            // e^(a+bi) = e^a (cos b + i sin b)
            let mut ea = self.real_part();
            if !ea.exp() {
                return false;
            }
            let b = self.imaginary_part();
            let mut cb = b.clone();
            let mut sb = b;
            if !cb.cos() || !sb.sin() {
                return false;
            }
            let mut im = ea.clone();
            if !im.multiply(&sb) || !ea.multiply(&cb) {
                return false;
            }
            *self = ea;
            if !im.is_zero() {
                self.set_imaginary_part(&im);
            }
            return true;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            self.clear(true);
            self.approx = true;
            return true;
        }
        if self.is_zero() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.exp(p, rm, cc))
    }

    /// `sin()`.
    pub fn sin(&mut self) -> bool {
        if self.has_imaginary_part() {
            // sin(a+bi) = sin a cosh b + i cos a sinh b
            let a = self.real_part();
            let b = self.imaginary_part();
            let (mut sa, mut ca) = (a.clone(), a);
            let (mut chb, mut shb) = (b.clone(), b);
            if !sa.sin() || !ca.cos() || !chb.cosh() || !shb.sinh() {
                return false;
            }
            if !sa.multiply(&chb) || !ca.multiply(&shb) {
                return false;
            }
            *self = sa;
            if !ca.is_zero() {
                self.set_imaginary_part(&ca);
            }
            return true;
        }
        if self.is_infinite(true) {
            return false;
        }
        if self.is_zero() {
            return true;
        }
        self.apply_trig_bounded(|x, p, rm, cc| x.sin(p, rm, cc), TrigKind::Sin)
    }

    /// `cos()`.
    pub fn cos(&mut self) -> bool {
        if self.has_imaginary_part() {
            // cos(a+bi) = cos a cosh b − i sin a sinh b
            let a = self.real_part();
            let b = self.imaginary_part();
            let (mut sa, mut ca) = (a.clone(), a);
            let (mut chb, mut shb) = (b.clone(), b);
            if !sa.sin() || !ca.cos() || !chb.cosh() || !shb.sinh() {
                return false;
            }
            if !ca.multiply(&chb) || !sa.multiply(&shb) || !sa.negate() {
                return false;
            }
            *self = ca;
            if !sa.is_zero() {
                self.set_imaginary_part(&sa);
            }
            return true;
        }
        if self.is_infinite(true) {
            return false;
        }
        if self.is_zero() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        self.apply_trig_bounded(|x, p, rm, cc| x.cos(p, rm, cc), TrigKind::Cos)
    }

    /// `tan()`.
    pub fn tan(&mut self) -> bool {
        if self.has_imaginary_part() {
            let mut s = self.clone();
            let mut c = self.clone();
            if !s.sin() || !c.cos() {
                return false;
            }
            if !s.divide(&c) {
                return false;
            }
            *self = s;
            return true;
        }
        if self.is_infinite(true) {
            return false;
        }
        if self.is_zero() {
            return true;
        }
        // Reject intervals spanning an asymptote: tan monotone increasing on
        // each branch; if tan(l) > tan(u) the interval crossed a pole.
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            context::with_consts(|cc| {
                (al.tan(p, RoundingMode::Down, cc), au.tan(p, RoundingMode::Up, cc))
            })
        } else {
            let v = context::with_consts(|cc| al.tan(p, RoundingMode::ToEven, cc));
            (v.clone(), v)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        if lower.cmp(&upper) == Some(1) {
            return false; // pole inside interval
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `sinh()` — monotone increasing.
    pub fn sinh(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex sinh
        }
        if self.is_infinite(true) {
            return true;
        }
        if self.is_zero() {
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.sinh(p, rm, cc))
    }

    /// `cosh()` — decreasing on (−∞,0], increasing on [0,∞).
    pub fn cosh(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex cosh
        }
        if self.is_infinite(true) {
            self.value = RealValue::PlusInfinity;
            return true;
        }
        if self.is_zero() {
            let keep = (self.approx, self.precision);
            *self = Number::from_i64(1);
            self.approx = keep.0;
            self.precision = keep.1;
            return true;
        }
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let spans_zero = matches!(al.sign(), Some(astro_float::Sign::Neg))
            && matches!(au.sign(), Some(astro_float::Sign::Pos));
        if !context::create_interval() {
            return self.apply_monotone(|x, p, rm, cc| x.cosh(p, rm, cc));
        }
        let (lower, upper) = context::with_consts(|cc| {
            if spans_zero {
                let cl = al.cosh(p, RoundingMode::Up, cc);
                let cu = au.cosh(p, RoundingMode::Up, cc);
                let hi = if cl.cmp(&cu) == Some(1) { cl } else { cu };
                (BigFloat::from_i8(1, p), hi)
            } else if matches!(au.sign(), Some(astro_float::Sign::Neg)) {
                // negative interval: decreasing
                (au.cosh(p, RoundingMode::Down, cc), al.cosh(p, RoundingMode::Up, cc))
            } else {
                (al.cosh(p, RoundingMode::Down, cc), au.cosh(p, RoundingMode::Up, cc))
            }
        });
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `tanh()` — monotone increasing.
    pub fn tanh(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex tanh
        }
        if self.is_plus_infinity() {
            *self = Number::from_i64(1);
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            *self = Number::from_i64(-1);
            self.approx = true;
            return true;
        }
        if self.is_zero() {
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.tanh(p, rm, cc))
    }

    /// `asin()` — monotone increasing on [−1,1].
    pub fn asin(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex asin
        }
        if self.is_zero() {
            return true;
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if self.is_greater_than(&one) || self.is_less_than(&mone) {
            return false; // TODO(port): complex result
        }
        self.apply_monotone(|x, p, rm, cc| x.asin(p, rm, cc))
    }

    /// `acos()` — monotone decreasing on [−1,1].
    pub fn acos(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex acos
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if self.is_greater_than(&one) || self.is_less_than(&mone) {
            return false;
        }
        if self.is_one() {
            self.clear(true);
            return true;
        }
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        let (lower, upper) = if context::create_interval() {
            context::with_consts(|cc| {
                (au.acos(p, RoundingMode::Down, cc), al.acos(p, RoundingMode::Up, cc))
            })
        } else {
            let v = context::with_consts(|cc| al.acos(p, RoundingMode::ToEven, cc));
            (v.clone(), v)
        };
        if lower.is_nan() || upper.is_nan() {
            return false;
        }
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }

    /// `atan()` — monotone increasing.
    pub fn atan(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex atan
        }
        if self.is_zero() {
            return true;
        }
        if self.is_plus_infinity() || self.is_minus_infinity() {
            let neg = self.is_minus_infinity();
            let mut pi = Number::new();
            pi.pi();
            if !pi.divide(&Number::from_i64(2)) {
                return false;
            }
            if neg {
                pi.negate();
            }
            *self = pi;
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.atan(p, rm, cc))
    }

    /// `asinh()` — monotone increasing.
    pub fn asinh(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex asinh
        }
        if self.is_zero() || self.is_infinite(true) {
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.asinh(p, rm, cc))
    }

    /// `acosh()` — monotone increasing on [1,∞).
    pub fn acosh(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex acosh
        }
        if self.is_plus_infinity() {
            return true;
        }
        let one = Number::from_i64(1);
        if self.is_less_than(&one) {
            return false; // TODO(port): complex result
        }
        if self.is_one() {
            self.clear(true);
            return true;
        }
        self.apply_monotone(|x, p, rm, cc| x.acosh(p, rm, cc))
    }

    /// `atanh()` — monotone increasing on (−1,1).
    pub fn atanh(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex atanh
        }
        if self.is_zero() {
            return true;
        }
        let one = Number::from_i64(1);
        let mone = Number::from_i64(-1);
        if self.is_greater_than(&one) || self.is_less_than(&mone) {
            return false; // TODO(port): complex result
        }
        self.apply_monotone(|x, p, rm, cc| x.atanh(p, rm, cc))
    }

    /// `atan2(x)` — self = atan2(self, x) (self is y).
    pub fn atan2(&mut self, x: &Number, allow_zero: bool) -> bool {
        if self.has_imaginary_part() || x.has_imaginary_part() {
            return false;
        }
        if self.is_zero() && x.is_zero() {
            if allow_zero {
                self.clear(true);
                return true;
            }
            return false;
        }
        // Quadrant-aware: atan(y/x) adjusted by ±π.
        if x.real_part_is_positive() {
            let mut q = self.clone();
            if !q.divide(x) || !q.atan() {
                return false;
            }
            *self = q;
            return true;
        }
        if x.real_part_is_negative() {
            let y_neg = self.real_part_is_negative();
            let mut q = self.clone();
            if !q.divide(x) || !q.atan() {
                return false;
            }
            let mut pi = Number::new();
            pi.pi();
            if y_neg {
                pi.negate();
            }
            if !q.add(&pi) {
                return false;
            }
            *self = q;
            return true;
        }
        // x contains/equals zero: y must be sign-definite → ±π/2
        if self.real_part_is_positive() || self.real_part_is_negative() {
            let neg = self.real_part_is_negative();
            let mut pi = Number::new();
            pi.pi();
            if !pi.divide(&Number::from_i64(2)) {
                return false;
            }
            if neg {
                pi.negate();
            }
            *self = pi;
            return true;
        }
        false
    }

    /// `arg()` — argument of the complex number.
    pub fn arg(&mut self) -> bool {
        if !self.has_imaginary_part() {
            if self.is_zero() {
                return false;
            }
            if self.real_part_is_negative() {
                let mut pi = Number::new();
                pi.pi();
                *self = pi;
                return true;
            }
            if self.real_part_is_positive() {
                self.clear(true);
                return true;
            }
            return false;
        }
        let re = self.real_part();
        let mut im = self.imaginary_part();
        if !im.atan2(&re, false) {
            return false;
        }
        *self = im;
        true
    }
}

/// Which bounded trig function is being applied (for extremum handling).
enum TrigKind {
    Sin,
    Cos,
}

impl Number {
    /// sin/cos over an interval: if any extremum (odd multiples of π/2 for
    /// sin, multiples of π for cos) lies inside, the corresponding bound is
    /// ±1; otherwise evaluate endpoints. Falls back to [−1,1] for wide
    /// intervals.
    fn apply_trig_bounded(&mut self, f: MonoFn, kind: TrigKind) -> bool {
        let p = context::bit_precision();
        let (al, au) = (self.lower_bound_float(p), self.upper_bound_float(p));
        if !context::create_interval() || al == au {
            let v = context::with_consts(|cc| f(&al, p, RoundingMode::ToEven, cc));
            if v.is_nan() {
                return false;
            }
            let (lower, upper) = if context::create_interval() {
                // Point input still needs outward rounding of the result.
                context::with_consts(|cc| {
                    (f(&al, p, RoundingMode::Down, cc), f(&au, p, RoundingMode::Up, cc))
                })
            } else {
                (v.clone(), v)
            };
            self.value = RealValue::Float { lower, upper };
            self.approx = true;
            return self.test_float_result(true);
        }
        // Interval width ≥ 2π → full range.
        let width = au.sub(&al, p, RoundingMode::Up);
        let two_pi = context::with_consts(|cc| {
            cc.pi(p, RoundingMode::Up).mul(&BigFloat::from_i8(2, p), p, RoundingMode::Up)
        });
        let full_range = width.cmp(&two_pi) != Some(-1);
        let (mut has_max, mut has_min) = (full_range, full_range);
        if !full_range {
            // Count extremum points k in the interval: for sin the maxima are
            // at (4k+1)·π/2, minima at (4k+3)·π/2; for cos maxima at 2kπ,
            // minima at (2k+1)π. Work with t = x / (π/2).
            let half_pi = context::with_consts(|cc| {
                cc.pi(p, RoundingMode::ToEven)
                    .div(&BigFloat::from_i8(2, p), p, RoundingMode::ToEven)
            });
            let tl = al.div(&half_pi, p, RoundingMode::Down);
            let tu = au.div(&half_pi, p, RoundingMode::Up);
            // Integers in [tl, tu] with the right residue mod 4.
            let (kl, ku) = (bigfloat_floor_i(&tl), bigfloat_floor_i(&tu));
            let mut k = kl.clone();
            let one = BigInt::from(1);
            while k <= ku {
                let residue = ((&k % 4i32) + 4i32) % 4i32;
                let r = residue.to_string();
                match kind {
                    TrigKind::Sin => {
                        if r == "1" {
                            has_max = true;
                        } else if r == "3" {
                            has_min = true;
                        }
                    }
                    TrigKind::Cos => {
                        if r == "0" {
                            has_max = true;
                        } else if r == "2" {
                            has_min = true;
                        }
                    }
                }
                if has_max && has_min {
                    break;
                }
                k += &one;
            }
        }
        let (fl_l, fl_u, fu_l, fu_u) = context::with_consts(|cc| {
            (
                f(&al, p, RoundingMode::Down, cc),
                f(&al, p, RoundingMode::Up, cc),
                f(&au, p, RoundingMode::Down, cc),
                f(&au, p, RoundingMode::Up, cc),
            )
        });
        if fl_l.is_nan() || fu_l.is_nan() {
            return false;
        }
        let lower = if has_min {
            BigFloat::from_i8(-1, p)
        } else if fl_l.cmp(&fu_l) == Some(1) {
            fu_l
        } else {
            fl_l
        };
        let upper = if has_max {
            BigFloat::from_i8(1, p)
        } else if fl_u.cmp(&fu_u) == Some(-1) {
            fu_u
        } else {
            fl_u
        };
        self.value = RealValue::Float { lower, upper };
        self.approx = true;
        self.test_float_result(true)
    }
}

/// floor of a BigFloat as BigInt (assumes finite).
fn bigfloat_floor_i(f: &BigFloat) -> BigInt {
    match crate::float::bigfloat_to_ratio(f) {
        Some((n, d)) => num_integer::Integer::div_floor(&n, &d),
        None => BigInt::zero(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PrintOptions;

    fn po_ez() -> PrintOptions {
        let mut po = PrintOptions::default();
        po.show_ending_zeroes = true;
        po
    }

    #[test]
    fn ln_exp_roundtrip() {
        let mut n = Number::from_i64(10);
        assert!(n.ln());
        assert_eq!(n.print(&po_ez()), "2.302585093");
        let mut e = Number::from_i64(1);
        assert!(e.exp());
        assert_eq!(e.print(&po_ez()), "2.718281828");
        let mut one = Number::from_i64(1);
        assert!(one.ln());
        assert!(one.is_zero(), "ln(1) = 0 exact");
    }

    #[test]
    fn ln_negative_complex() {
        let mut n = Number::from_i64(-1);
        assert!(n.ln());
        assert!(n.is_complex(), "ln(-1) = πi");
        assert!(!n.has_real_part());
        assert_eq!(n.imaginary_part().print(&po_ez()), "3.141592654");
    }

    #[test]
    fn trig_basics() {
        let mut n = Number::from_i64(1);
        assert!(n.sin());
        assert_eq!(n.print(&po_ez()), "0.8414709848");
        let mut c = Number::from_i64(1);
        assert!(c.cos());
        assert_eq!(c.print(&po_ez()), "0.5403023059");
        let mut t = Number::from_i64(1);
        assert!(t.tan());
        assert_eq!(t.print(&po_ez()), "1.557407725");
    }

    #[test]
    fn sin_pi_contains_zero() {
        let mut pi = Number::new();
        pi.pi();
        assert!(pi.sin());
        // sin(π-interval) is a tiny interval containing 0 — must not claim
        // to be nonzero.
        assert!(!pi.is_nonzero() || pi.is_zero());
    }

    #[test]
    fn sin_interval_spanning_max() {
        // Interval [1, 2.5] contains π/2 → upper bound must be exactly 1.
        let mut n = Number::new();
        assert!(n.set_interval(&Number::from_i64(1), &Number::from_ints(25, 10, 0), false));
        assert!(n.sin());
        let hi = n.upper_end_point();
        assert!(hi.equals_i64(1), "sin over [1,2.5] has max exactly 1, got {hi:?}");
    }

    #[test]
    fn atan_infinity() {
        let mut n = Number::new();
        n.set_plus_infinity(false, false);
        assert!(n.atan());
        assert_eq!(n.print(&po_ez()), "1.570796327");
    }

    #[test]
    fn inverse_trig() {
        let mut n = Number::from_ints(1, 2, 0);
        assert!(n.asin());
        assert_eq!(n.print(&po_ez()), "0.5235987756");
        let mut c = Number::from_ints(1, 2, 0);
        assert!(c.acos());
        assert_eq!(c.print(&po_ez()), "1.047197551");
        let mut bad = Number::from_i64(2);
        assert!(!bad.asin(), "asin(2) real fails (complex TODO)");
    }

    #[test]
    fn hyperbolic() {
        let mut n = Number::from_i64(1);
        assert!(n.sinh());
        assert_eq!(n.print(&po_ez()), "1.175201194");
        let mut c = Number::from_i64(1);
        assert!(c.cosh());
        assert_eq!(c.print(&po_ez()), "1.543080635");
        let mut t = Number::from_i64(1);
        assert!(t.tanh());
        assert_eq!(t.print(&po_ez()), "0.7615941560");
    }

    #[test]
    fn exp_complex_euler() {
        // e^(iπ) = −1 (within interval tolerance the imaginary part vanishes
        // or is negligible; real part ≈ −1)
        let mut z = Number::new();
        let mut pi = Number::new();
        pi.pi();
        z.set_imaginary_part(&pi);
        assert!(z.exp());
        let re = z.real_part();
        assert!(re.is_less_than(&Number::from_ints(-99, 100, 0)));
        assert!(re.is_greater_than(&Number::from_ints(-101, 100, 0)));
    }

    #[test]
    fn log_base() {
        let mut n = Number::from_i64(8);
        assert!(n.log(&Number::from_i64(2)));
        // log2(8) = 3 — float path, should be very close to 3
        assert!(n.is_greater_than(&Number::from_ints(299, 100, 0)));
        assert!(n.is_less_than(&Number::from_ints(301, 100, 0)));
    }
}
