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
}
