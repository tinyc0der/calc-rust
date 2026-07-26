//! The Lambert W function — port of `Number::lambertW` (`Number.cc:7748`).

use super::Number;
use crate::context;
use crate::options::ParseOptions;

/// Hard cap on the Halley iteration. The C++ allows `PRECISION * 100` steps;
/// the iteration is cubically convergent from every starting point used here,
/// so anything beyond a few dozen means it is not going to converge and the
/// function must fail rather than spin.
const MAX_ITERATIONS: usize = 100;

fn lit(s: &str) -> Number {
    Number::parse(s, &ParseOptions::default())
}

fn cplx(re: &str, im: &str) -> Number {
    let mut n = lit(re);
    n.set_imaginary_part(&lit(im));
    n
}

impl Number {
    /// `W_k(self)`, replacing `self`. Returns false when the value is
    /// outside the branch's domain or the iteration does not converge.
    ///
    /// The principal branch (`k = 0`) and `k = -1` get the C++'s dedicated
    /// starting approximations; other integer branches fall back to the
    /// asymptotic `L₁ − ln L₁` start, as in the C++.
    pub fn lambert_w(&mut self, k: i64) -> bool {
        // dW/dz = W / (z(1 + W)) — evaluated before the value is overwritten.
        if self.unc.is_some() {
            return self.uncertain_unary(
                move |z| {
                    let mut w = z.clone();
                    if !w.lambert_w(k) {
                        return None;
                    }
                    let mut den = w.clone();
                    (den.add(&Number::from_i64(1)) && den.multiply(z) && w.divide(&den))
                        .then_some(w)
                },
                move |s| s.lambert_w_impl(k),
            );
        }
        self.lambert_w_impl(k)
    }

    fn lambert_w_impl(&mut self, k: i64) -> bool {
        if self.is_zero() {
            if k == 0 {
                return true;
            }
            self.set_minus_infinity(true, false);
            return true;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.includes_infinity() {
            return false;
        }

        // The result stays real on the real stretches of the two principal
        // branches; rounding noise in the iteration must not leave a spurious
        // imaginary part behind (`b_real` in the C++).
        let b_real = !self.has_imaginary_part() && {
            let mut lower = Number::from_i64(-1);
            lower.exp()
                && lower.negate()
                && self.is_greater_than_or_equal_to(&lower)
                && (k == 0 || (k == -1 && self.is_negative()))
        };

        // The C++ raises the precision and forces interval arithmetic on so
        // that it can read the achieved precision off the interval width.
        // This port iterates in point mode instead — an interval would grow
        // with every Halley step and drown the result — and stamps the final
        // precision on by hand, which is what the non-interval branch of
        // `setRelativeUncertainty` amounts to.
        let prec_bak = context::precision();
        let interval_bak = context::create_interval();
        context::set_precision(prec_bak * 2 + 20);
        context::set_create_interval(false);
        let outcome = self.lambert_w_iterate(k, b_real, prec_bak);
        context::set_precision(prec_bak);
        context::set_create_interval(interval_bak);
        let Some(mut v) = outcome else {
            return false;
        };
        v.set_to_floating_point();
        v.approx = true;
        v.precision = prec_bak;
        if let Some(im) = &mut v.imag {
            im.set_to_floating_point();
            im.approx = true;
            im.precision = prec_bak;
        }
        *self = v;
        true
    }

    /// Starting approximation plus the Halley iteration, at raised precision.
    fn lambert_w_iterate(&self, k: i64, b_real: bool, prec_bak: i32) -> Option<Number> {
        let z = self.clone();
        let mut v = lambert_w_start(&z, k)?;
        if b_real {
            v.clear_imaginary();
        }

        // Convergence threshold: a relative step below 10^-(prec+5).
        let mut eps = Number::from_i64(10);
        if !eps.raise(&Number::from_i64(-(prec_bak as i64) - 5), true) {
            return None;
        }

        let two = Number::from_i64(2);
        let minus_two = Number::from_i64(-2);
        for _ in 0..MAX_ITERATIONS {
            // w -= 2(we^w − z)·d / (2d² − (we^w − z)·dd)
            // with d = (1+w)e^w and dd = (2+w)e^w.
            let mut wexp = v.clone();
            if !wexp.exp() {
                return None;
            }
            let mut wexpw = wexp.clone();
            if !wexpw.multiply(&v) {
                return None;
            }
            let mut d = wexpw.clone();
            if !d.add(&wexp) {
                return None;
            }
            let mut dd = wexp;
            if !dd.multiply(&two) || !dd.add(&wexpw) {
                return None;
            }
            let mut num = wexpw;
            if !num.subtract(&z) {
                return None;
            }
            let mut den = d.clone();
            if !den.square() || !den.multiply(&two) {
                return None;
            }
            let mut corr = num.clone();
            if !corr.multiply(&dd) || !den.subtract(&corr) {
                return None;
            }
            let mut step = num;
            if !step.multiply(&d) || !step.multiply(&minus_two) {
                return None;
            }
            if !den.is_nonzero() || !step.divide(&den) {
                return None;
            }
            let mut rel = step.clone();
            if !v.add(&step) || v.includes_infinity() {
                return None;
            }
            if b_real {
                v.clear_imaginary();
            }
            // Relative step size.
            if !rel.abs() {
                return None;
            }
            let mut scale = v.clone();
            if !scale.abs() {
                return None;
            }
            if scale.is_nonzero() && !rel.divide(&scale) {
                return None;
            }
            if rel.is_less_than(&eps) {
                return Some(v);
            }
        }
        None
    }
}

/// The starting approximations of `Number::lambertW` (`Number.cc:7834`).
fn lambert_w_start(z: &Number, k: i64) -> Option<Number> {
    if k == 0 || k == -1 {
        // Padé-style approximant on the disc around ½.
        let mut near_half = z.clone();
        if !near_half.add(&Number::from_ints(-1, 2, 0)) || !near_half.abs() {
            return None;
        }
        let half = Number::from_ints(1, 2, 0);
        let five_eighths = Number::from_ints(5, 8, 0);
        if near_half.is_less_than_or_equal_to(&half)
            || (k == 0 && near_half.is_less_than_or_equal_to(&five_eighths))
        {
            if k == 0 {
                let mut ndiv = z.clone();
                let mut v = z.clone();
                if !ndiv.multiply(&Number::from_i64(2))
                    || !ndiv.add(&Number::from_i64(1))
                    || !ndiv.multiply(&lit("0.827184"))
                    || !ndiv.add(&Number::from_i64(2))
                    || !v.multiply(&lit("7.061302897"))
                    || !v.add(&lit("0.1237166"))
                    || !v.multiply(&lit("0.35173371"))
                    || !v.divide(&ndiv)
                {
                    return None;
                }
                return Some(v);
            }
            let i1 = cplx("2.2591588985", "4.22096");
            let i2 = cplx("-14.073271", "-33.767687754");
            let mut i3 = cplx("-12.7127", "19.071643");
            let mut i4 = cplx("-17.23103", "10.629721");
            let mut n1p2z = z.clone();
            if !n1p2z.multiply(&Number::from_i64(2))
                || !n1p2z.add(&Number::from_i64(1))
                || !i4.multiply(&n1p2z)
                || !i4.add(&Number::from_i64(2))
                || !i3.multiply(&n1p2z)
            {
                return None;
            }
            let mut v = i2;
            if !v.multiply(z) || !v.add(&i3) || !v.multiply(&i1) || !v.divide(&i4) || !v.negate() {
                return None;
            }
            return Some(v);
        }
        // Branch-point series around z = −1/e.
        if k != -1 || z.imaginary_part().is_positive() {
            let mut near_bp = Number::from_i64(-1);
            if !near_bp.exp() || !near_bp.add(z) || !near_bp.abs() {
                return None;
            }
            if near_bp.is_less_than_or_equal_to(&Number::from_i64(1)) {
                let mut p = Number::new();
                p.e();
                if !p.multiply(z)
                    || !p.add(&Number::from_i64(1))
                    || !p.multiply(&Number::from_i64(2))
                    || !p.raise(&Number::from_ints(1, 2, 0), false)
                {
                    return None;
                }
                let mut p2 = p.clone();
                if !p2.square() || !p2.multiply(&Number::from_ints(-1, 3, 0)) {
                    return None;
                }
                let mut p3 = p.clone();
                if !p3.raise(&Number::from_i64(3), true)
                    || !p3.multiply(&Number::from_ints(if k == 0 { 11 } else { -11 }, 72, 0))
                {
                    return None;
                }
                let mut v = p;
                if (k != 0 && !v.negate())
                    || !v.add(&Number::from_i64(-1))
                    || !v.add(&p2)
                    || !v.add(&p3)
                {
                    return None;
                }
                return Some(v);
            }
        }
    }
    // Asymptotic start: L₁ = ln z + 2πki, w ≈ L₁ − ln L₁.
    let mut logz = z.clone();
    if !logz.ln() {
        return None;
    }
    if k != 0 {
        let mut two_pi_ki = Number::new();
        two_pi_ki.pi();
        let mut i_unit = Number::new();
        i_unit.set_imaginary_part(&Number::from_i64(1));
        if !two_pi_ki.multiply(&Number::from_i64(k))
            || !two_pi_ki.multiply(&Number::from_i64(2))
            || !two_pi_ki.multiply(&i_unit)
            || !logz.add(&two_pi_ki)
        {
            return None;
        }
    }
    let mut v = logz.clone();
    if !v.ln() || !v.negate() || !v.add(&logz) {
        return None;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PrintOptions;

    fn po() -> PrintOptions {
        PrintOptions::default()
    }

    #[test]
    fn principal_branch_inverts_w_exp_w() {
        // W(3e³) = 3 exactly, the defining identity.
        let mut n = Number::from_i64(3);
        assert!(n.exp() && n.multiply(&Number::from_i64(3)));
        assert!(n.lambert_w(0));
        assert_eq!(n.print(&po()), "3");
    }

    #[test]
    fn principal_branch_real_values() {
        // Oracle: qalc -t 'lambertw(1)' / 'lambertw(5)' / 'lambertw(0.5)'.
        for (arg, want) in [
            ("1", "0.5671432904"),
            ("5", "1.326724665"),
            ("0.5", "0.3517337112"),
        ] {
            let mut n = Number::parse(arg, &Default::default());
            assert!(n.lambert_w(0), "lambertw({arg})");
            assert_eq!(n.print(&po()), want, "lambertw({arg})");
        }
    }

    #[test]
    fn zero_and_branch_point() {
        let mut n = Number::from_i64(0);
        assert!(n.lambert_w(0) && n.is_zero());
        let mut n = Number::from_i64(0);
        assert!(n.lambert_w(-1) && n.is_minus_infinity());
    }

    #[test]
    fn negative_argument_on_both_branches() {
        // Oracle: qalc -t 'lambertw(-0.2)' and 'lambertw(-0.2, -1)'.
        let mut n = Number::parse("-0.2", &Default::default());
        assert!(n.lambert_w(0));
        assert_eq!(n.print(&po()), "-0.2591711018");
        let mut n = Number::parse("-0.2", &Default::default());
        assert!(n.lambert_w(-1));
        assert_eq!(n.print(&po()), "-2.542641358");
    }

    #[test]
    fn complex_argument_second_branch() {
        // Oracle: qalc -t 'lambertw(2 + 5i, -1)'.
        let mut n = Number::from_i64(2);
        n.set_imaginary_part(&Number::from_i64(5));
        assert!(n.lambert_w(-1));
        assert_eq!(n.print(&po()), "0.3890084896 - 3.628888908i");
    }

    #[test]
    fn iteration_is_bounded() {
        // Oracle: qalc -t 'lambertw(-1)' leaves the principal branch's real
        // domain and returns a complex value; it must not spin.
        let mut n = Number::from_i64(-1);
        assert!(n.lambert_w(0));
        assert_eq!(n.print(&po()), "-0.3181315052 + 1.337235701i");
    }
}
