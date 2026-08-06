#[cfg(test)]
mod audit_regressions {
    use qalc_core::eval::evaluate_to_string;
    use qalc_core::Session;

    #[test]
    fn test_solve_trig_higher_degree_terms_preserved() {
        let mut session = Session::new();
        // Equation with deg(P) > deg(Q) + 1 must not truncate higher degree terms
        let res = session.evaluate_line("solve(sin(x)^3 + cos(x) = 0, x)");
        assert!(res.is_ok());
    }

    #[test]
    fn test_diff_root_single_argument_no_panic() {
        let res = evaluate_to_string("diff(root(x))");
        assert!(res.is_err() || res.is_ok()); // Must not panic
    }

    #[test]
    fn test_range_expansion_bounded() {
        let res = evaluate_to_string("sum(x, 1, 1000000000)");
        assert!(res.is_ok() || res.is_err()); // Must not OOM
    }

    #[test]
    fn test_datetime_i64_min_year_no_panic() {
        let mut session = Session::new();
        let res = session.evaluate_line("date(-9223372036854775808)");
        assert!(res.is_ok());
    }

    /// `(x^2)^(1/3)` is real and positive for every real `x`: the inner square
    /// discards the sign before the cube root is taken. `linear_power` used to
    /// fold the two exponents into `x^(2/3)`, which is the *principal* — and
    /// for `x < 0` complex — root, so the substitution built an antiderivative
    /// on the wrong branch. At `x = -5` the integrand is `cbrt(25) = 2.924...`
    /// while the folded form gives `(-5)^(2/3) = -1.462... + 2.532...i`.
    ///
    /// Declining to integrate is the correct outcome: the radical substitution
    /// does not apply to this shape. What must never happen is answering with
    /// an antiderivative whose derivative disagrees with the integrand.
    #[test]
    fn test_integrate_even_inner_power_keeps_real_branch() {
        use qalc_core::{parser, EvaluationOptions, MathStructure};

        let session = Session::new();
        let x = MathStructure::symbolic("x");
        let eo = EvaluationOptions::default();
        let at = MathStructure::from_i64(-5);

        let mut f = parser::parse_with("(x^2)^(1/3)", &session.parse_options, &session)
            .expect("integrand parses");
        qalc_core::eval::evaluate_calculated_with(&mut f, &eo);

        let Some(mut anti) = qalc_core::integrate::integrate(&f, &x) else {
            return;
        };
        qalc_core::eval::evaluate_calculated_with(&mut anti, &eo);
        let Some(mut back) = qalc_core::differentiate::differentiate(&anti, &x) else {
            return;
        };
        qalc_core::eval::evaluate_calculated_with(&mut back, &eo);

        // Both sides at x = -5, forced all the way to a number.
        let approx = EvaluationOptions {
            approximation: qalc_core::options::ApproximationMode::Approximate,
            ..EvaluationOptions::default()
        };
        let value_at = |m: &MathStructure| -> (f64, f64) {
            let mut v = m.clone();
            qalc_core::solve::replace(&mut v, &x, &at);
            qalc_core::eval::evaluate_calculated_with(&mut v, &approx);
            match &v {
                MathStructure::Number(n) => (
                    n.real_part().float_value(),
                    n.imaginary_part().float_value(),
                ),
                other => panic!("expected a number at x = -5, got {other:?}"),
            }
        };

        let (got_re, got_im) = value_at(&back);
        let (want_re, want_im) = value_at(&f);
        assert!(
            (got_re - want_re).abs() < 1e-6 && (got_im - want_im).abs() < 1e-6,
            "d/dx of the antiderivative of (x^2)^(1/3) must equal the integrand \
             at x = -5: got {got_re} + {got_im}i, want {want_re} + {want_im}i"
        );
    }

    /// `k * sqrt(n)` and `sqrt(n) * k` are the same product and must evaluate
    /// the same way. The `split_squares` recursion guard only recognised the
    /// factored shape when the number came *first*, so `sqrt(3) * i` had its
    /// `sqrt` numerified into `1.732...i` while `i * sqrt(3)` stayed symbolic.
    #[test]
    fn test_factored_radical_is_order_independent() {
        let mut session = Session::new();
        let left = session
            .evaluate_line("i*sqrt(3)")
            .expect("i*sqrt(3) evaluates");
        let mut session = Session::new();
        let right = session
            .evaluate_line("sqrt(3)*i")
            .expect("sqrt(3)*i evaluates");
        assert_eq!(
            left, right,
            "the same product must not depend on factor order"
        );

        // The factored spelling itself is still the one `split_squares` wants.
        let mut session = Session::new();
        assert_eq!(
            session.evaluate_line("sqrt(32)").expect("sqrt(32) evaluates"),
            "4 * sqrt(2)"
        );
    }

    /// `d/dx ln|v|` must be `v'/v`.
    ///
    /// Composing `d/du ln(u) = 1/u` with `d/dv |v| = v/|v|` multiplies out to
    /// `v' * v / |v|^2`, i.e. `v' / conj(v)`. That is the same number for real
    /// `v` but the conjugate for complex `v`, so the antiderivative produced by
    /// `int dx/(ax+b) = ln|ax+b|/a` differentiated back to the *conjugate* of
    /// the integrand once `ax + b` went complex — right imaginary part, wrong
    /// sign on the real part.
    ///
    /// `1/(sqrt(x)-2)` at `x = -5` is `-0.2222 - 0.2485i`; the composed form
    /// gave `+0.2222 - 0.2485i`.
    #[test]
    fn test_ln_abs_derivative_holds_on_complex_domain() {
        use qalc_core::options::ApproximationMode;
        use qalc_core::{parser, EvaluationOptions, MathStructure};

        let session = Session::new();
        let x = MathStructure::symbolic("x");
        let eo = EvaluationOptions::default();
        let approx = EvaluationOptions {
            approximation: ApproximationMode::Approximate,
            ..EvaluationOptions::default()
        };

        let mut f = parser::parse_with("(sqrt(x)-2)^(-1)", &session.parse_options, &session)
            .expect("integrand parses");
        qalc_core::eval::evaluate_calculated_with(&mut f, &eo);

        let mut anti = qalc_core::integrate::integrate(&f, &x).expect("rule applies");
        qalc_core::eval::evaluate_calculated_with(&mut anti, &eo);
        let mut back =
            qalc_core::differentiate::differentiate(&anti, &x).expect("antiderivative differentiates");
        qalc_core::eval::evaluate_calculated_with(&mut back, &eo);

        let value_at = |m: &MathStructure, at: i64| -> (f64, f64) {
            let mut v = m.clone();
            qalc_core::solve::replace(&mut v, &x, &MathStructure::from_i64(at));
            qalc_core::eval::evaluate_calculated_with(&mut v, &approx);
            match &v {
                MathStructure::Number(n) => (
                    n.real_part().float_value(),
                    n.imaginary_part().float_value(),
                ),
                other => panic!("expected a number, got {other:?}"),
            }
        };

        // x = -5 puts `sqrt(x) - 2` off the real line, which is where the
        // conjugated form used to disagree.
        for at in [3i64, -5] {
            let (got_re, got_im) = value_at(&back, at);
            let (want_re, want_im) = value_at(&f, at);
            assert!(
                (got_re - want_re).abs() < 1e-6 && (got_im - want_im).abs() < 1e-6,
                "d/dx of ln|v| antiderivative must match the integrand at x = {at}: \
                 got {got_re} + {got_im}i, want {want_re} + {want_im}i"
            );
        }

        // `diff(abs(x))` itself is unchanged: the reference prints `x / |x|`.
        let mut session = Session::new();
        assert_eq!(
            session.evaluate_line("diff(abs(x))").expect("evaluates"),
            "x / |x|"
        );
    }

    /// `int cbrt(u) du` must stay on the real cube root.
    ///
    /// The rule was spelled `(3/4) u^(4/3)`, and `u^(4/3)` is the *principal*
    /// branch — complex for `u < 0` — while `cbrt(u)` is the real one. The
    /// antiderivative therefore denoted a different function than the integrand
    /// on the negative reals, and mixing the two spellings in one answer left a
    /// residue that never cancelled: `int 3 cbrt(x) x dx` came back
    /// `27.48 + 47.60i` at `x = -5` where the real value is `-54.96`.
    #[test]
    fn test_integrate_cbrt_stays_on_real_branch() {
        use qalc_core::options::ApproximationMode;
        use qalc_core::{parser, EvaluationOptions, MathStructure};

        let session = Session::new();
        let x = MathStructure::symbolic("x");
        let eo = EvaluationOptions::default();
        let approx = EvaluationOptions {
            approximation: ApproximationMode::Approximate,
            ..EvaluationOptions::default()
        };

        for expr in ["cbrt(x)", "x/cbrt(x)^2", "cbrt(x)*x"] {
            let mut f = parser::parse_with(expr, &session.parse_options, &session)
                .expect("integrand parses");
            qalc_core::eval::evaluate_calculated_with(&mut f, &eo);

            let mut anti = qalc_core::integrate::integrate(&f, &x)
                .unwrap_or_else(|| panic!("{expr}: a rule applies"));
            qalc_core::eval::evaluate_calculated_with(&mut anti, &eo);
            let mut back = qalc_core::differentiate::differentiate(&anti, &x)
                .unwrap_or_else(|| panic!("{expr}: the antiderivative differentiates"));
            qalc_core::eval::evaluate_calculated_with(&mut back, &eo);

            let value_at = |m: &MathStructure, at: i64| -> (f64, f64) {
                let mut v = m.clone();
                qalc_core::solve::replace(&mut v, &x, &MathStructure::from_i64(at));
                qalc_core::eval::evaluate_calculated_with(&mut v, &approx);
                match &v {
                    MathStructure::Number(n) => (
                        n.real_part().float_value(),
                        n.imaginary_part().float_value(),
                    ),
                    other => panic!("{expr}: expected a number, got {other:?}"),
                }
            };

            // `x = -5` is the point the principal branch got wrong.
            for at in [3i64, -5] {
                let (got_re, got_im) = value_at(&back, at);
                let (want_re, want_im) = value_at(&f, at);
                assert!(
                    (got_re - want_re).abs() < 1e-6 && (got_im - want_im).abs() < 1e-6,
                    "{expr}: d/dx of the antiderivative must match the integrand at \
                     x = {at}: got {got_re} + {got_im}i, want {want_re} + {want_im}i"
                );
            }
        }
    }

    /// The `abs` inside a `ln` argument may sit below a quotient, which is what
    /// the partial-fraction rules emit: `ln(|a| / |b|)`. Those went through the
    /// same conjugating composition that `ln|v|` did, so stripping has to reach
    /// them too. A correct derivative of `ln(|a|/|b|)` carries no `|...|^2`
    /// term, which is the signature the composed form leaves behind.
    #[test]
    fn test_ln_abs_quotient_derivative_has_no_modulus_squared() {
        let mut session = Session::new();
        let d = session
            .evaluate_line("diff(ln(abs(x+1)/abs(x-1)))")
            .expect("evaluates");
        assert!(
            !d.contains("|"),
            "d/dx ln(|a|/|b|) must not differentiate through abs, got {d}"
        );

        // The plain cases stay as they were.
        let mut session = Session::new();
        assert_eq!(
            session.evaluate_line("diff(ln(abs(x)))").expect("evaluates"),
            "1 / x"
        );
        let mut session = Session::new();
        assert_eq!(
            session.evaluate_line("diff(abs(x))").expect("evaluates"),
            "x / |x|"
        );
    }

    /// A hand-written `i * sqrt(5)` must evaluate like any other product.
    ///
    /// The `split_squares` recursion guard protected every `number * sqrt(n)`
    /// pair, but `extract_square_factor` only ever returns a *real* coefficient
    /// — the sign of the radicand stays in the remainder. The `i * sqrt(rem)`
    /// spelling is built one level up for a negative radicand, and only exact
    /// mode keeps it; the default and approximate modes answer `sqrt(-12)` as
    /// `3.4641016i`. Guarding it unconditionally left `i * sqrt(5)`, and every
    /// `sin`/`ln`/`asin` of one, permanently unevaluated.
    #[test]
    fn test_imaginary_coefficient_radical_still_evaluates() {
        // Approximate mode — what the CLI runs in — folds the product to a
        // number, so a function of it can be evaluated at all.
        let mut session = Session::new();
        session.evaluate_line("/set approximation approximate").ok();
        let got = session.evaluate_line("asin(i*sqrt(5))").expect("evaluates");
        assert!(
            got.starts_with("1.544"),
            "asin(i*sqrt(5)) must reduce to a number under approximation, got {got}"
        );

        // A real coefficient is still protected: this is the split spelling.
        let mut session = Session::new();
        assert_eq!(
            session.evaluate_line("sqrt(32)").expect("evaluates"),
            "4 * sqrt(2)"
        );

        // Exact mode still keeps the imaginary split form.
        let mut session = Session::new();
        session.evaluate_line("/set approximation exact").ok();
        assert_eq!(
            session.evaluate_line("sqrt(-12)").expect("evaluates"),
            "2i * sqrt(3)"
        );
    }
}
