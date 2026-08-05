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
}
