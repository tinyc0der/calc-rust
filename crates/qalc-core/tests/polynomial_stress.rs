//! Empirical stress tests for polynomial division, expansion, and factorization in qalc-core.

use qalc_core::options::EvaluationOptions;
use qalc_core::polynomial::{
    degree, polynomial_division_remainder, polynomial_quotient,
    dense_gcd, rational_roots,
};
use qalc_core::session::Session;
use qalc_core::structure::MathStructure;
use qalc_num::Number;

fn parse_and_eval(expr: &str) -> MathStructure {
    let mut m = qalc_core::eval::parse_expression(expr).expect("parse failed");
    qalc_core::eval::evaluate(&mut m);
    m
}

fn session_eval(expr: &str) -> String {
    let mut session = Session::new();
    session.evaluate_line(expr).expect("eval line failed")
}

// ===========================================================================
// 1. POLYNOMIAL DIVISION EDGE CASES & STRESS HARNESS
// ===========================================================================

#[test]
fn stress_poly_div_zero_denominator() {
    let num = parse_and_eval("5x^2 + 2");
    let den = parse_and_eval("0");
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    // Must return None, no panic or division by zero
    let res = polynomial_division_remainder(&num, &den, &xvar, &eo);
    assert!(res.is_none());

    let q_res = polynomial_quotient(&num, &den, &xvar, &eo);
    assert!(q_res.is_none());
}

#[test]
fn stress_poly_div_zero_numerator() {
    let num = parse_and_eval("0");
    let den = parse_and_eval("x - 3");
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    let (q, r) = polynomial_division_remainder(&num, &den, &xvar, &eo).expect("should succeed");
    assert!(q.is_zero(), "quotient of zero numerator should be zero");
    assert!(r.is_zero(), "remainder of zero numerator should be zero");
}

#[test]
fn stress_poly_div_equal_expressions() {
    let exprs = [
        "x - 3",
        "5x^2 + 2",
        "x^3 - 7x^2 + 9x + 27",
        "3x^4 - 2x^2 + 5x - 11",
    ];
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    for expr in exprs {
        let p = parse_and_eval(expr);
        let (q, r) = polynomial_division_remainder(&p, &p, &xvar, &eo).expect("equal division should succeed");
        assert!(q.is_one(), "P/P quotient should be 1 for {}", expr);
        assert!(r.is_zero(), "P/P remainder should be 0 for {}", expr);
    }
}

#[test]
fn stress_poly_div_lower_degree_numerator() {
    let num = parse_and_eval("x - 3");
    let den = parse_and_eval("5x^2 + 2");
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    let (q, r) = polynomial_division_remainder(&num, &den, &xvar, &eo).expect("should complete");
    assert!(q.is_zero(), "quotient should be zero when deg(num) < deg(den)");
    assert_eq!(
        qalc_core::print::print(&r, &qalc_core::eval::batch_print_options()),
        "x - 3"
    );
}

#[test]
fn stress_poly_div_high_degree() {
    // (x^10 - 1) / (x - 1) = x^9 + x^8 + ... + 1
    let num = parse_and_eval("x^10 - 1");
    let den = parse_and_eval("x - 1");
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    let (q, r) = polynomial_division_remainder(&num, &den, &xvar, &eo).expect("high degree division should succeed");
    assert!(r.is_zero(), "remainder of (x^10-1)/(x-1) should be 0");
    assert!(degree(&q, &xvar).equals_i64(9), "degree of quotient should be 9");
}

#[test]
fn stress_poly_div_exceed_guard_limit() {
    // Iteration guard limit is 1000 in polynomial_division_remainder.
    // Degree difference > 1000 should hit guard gracefully and return None.
    let num = parse_and_eval("x^1005 - 1");
    let den = parse_and_eval("x - 1");
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    let res = polynomial_division_remainder(&num, &den, &xvar, &eo);
    assert!(res.is_none(), "should hit guard limit and return None gracefully");
}

#[test]
fn stress_poly_div_multivariable() {
    // (x^2 - y^2) / (x - y) = x + y
    let num = parse_and_eval("x^2 - y^2");
    let den = parse_and_eval("x - y");
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    let (q, r) = polynomial_division_remainder(&num, &den, &xvar, &eo).expect("multivariable division");
    let q_str = qalc_core::print::print(&q, &qalc_core::eval::batch_print_options());
    assert!(q_str == "x + y" || q_str == "y + x", "expected x + y, got {}", q_str);
    assert!(r.is_zero(), "remainder should be zero for (x^2-y^2)/(x-y)");
}

#[test]
fn stress_poly_div_randomized_algebraic_identity() {
    // Property check: for generated P(x) = Q(x)*D(x) + R(x),
    // verifying division produces correct (Q, R) matching numerical evaluation.
    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    // Test cases: (Num, Den)
    let cases = [
        ("3x^3 + 5x^2 - 7x + 4", "x + 2"),
        ("x^4 - 16", "x^2 + 4"),
        ("2x^5 - 3x^3 + x^2 - 5", "x^2 - 1"),
        ("7x^3 - 2x^2 + 4x - 1", "2x - 3"),
    ];

    for (num_str, den_str) in cases {
        let num = parse_and_eval(num_str);
        let den = parse_and_eval(den_str);
        let (q, r) = polynomial_division_remainder(&num, &den, &xvar, &eo)
            .unwrap_or_else(|| panic!("division failed for ({}) / ({})", num_str, den_str));

        // Evaluate P(x) and Q(x)*D(x) + R(x) at sample points x = 3, 5, -2
        for test_val in [3i64, 5, -2] {
            let val_num = eval_at_num(&num, &xvar, test_val);
            let val_den = eval_at_num(&den, &xvar, test_val);
            let val_q = eval_at_num(&q, &xvar, test_val);
            let val_r = eval_at_num(&r, &xvar, test_val);

            // reconstructed = val_q * val_den + val_r
            let mut reconstructed = val_q;
            reconstructed.calculate_multiply(val_den, &eo);
            reconstructed.calculate_add(val_r, &eo);
            reconstructed.calculatesub(&eo);

            let mut diff = val_num.clone();
            diff.calculate_subtract(reconstructed, &eo);
            diff.calculatesub(&eo);

            assert!(
                diff.is_zero(),
                "Failed reconstruction for ({}) / ({}) at x={}: expected {:?}, got diff {:?}",
                num_str, den_str, test_val, val_num, diff
            );
        }
    }
}

fn eval_at_num(m: &MathStructure, var: &MathStructure, val: i64) -> MathStructure {
    let mut substituted = m.clone();
    qalc_core::matrix::replace(&mut substituted, var, &MathStructure::from(val));
    let eo = EvaluationOptions::default();
    substituted.calculatesub(&eo);
    qalc_core::sort::sort(&mut substituted);
    qalc_core::eval::evaluate(&mut substituted);
    substituted
}

fn eval_at_xy(m: &MathStructure, x_val: i64, y_val: i64) -> MathStructure {
    let mut substituted = m.clone();
    let xvar = MathStructure::symbolic("x");
    let yvar = MathStructure::symbolic("y");
    qalc_core::matrix::replace(&mut substituted, &xvar, &MathStructure::from(x_val));
    qalc_core::matrix::replace(&mut substituted, &yvar, &MathStructure::from(y_val));
    let eo = EvaluationOptions::default();
    substituted.calculatesub(&eo);
    qalc_core::sort::sort(&mut substituted);
    qalc_core::eval::evaluate(&mut substituted);
    substituted
}

// ===========================================================================
// 2. POLYNOMIAL EXPANSION & FACTORIZATION STRESS TESTS
// ===========================================================================

#[test]
fn stress_poly_expansion_readme_and_variants() {
    let clean = |s: String| s.replace('−', "-");
    assert_eq!(clean(session_eval("(5x^2 + 2)/(x - 3)")), "5x + 15 + 47/(x - 3)");
    assert_eq!(clean(session_eval("(x + 2)(x - 3)^3")), "x^4 - 7x^3 + 9x^2 + 27x - 54");
    assert_eq!(clean(session_eval("expand((x + 1)^4)")), "x^4 + 4x^3 + 6x^2 + 4x + 1");
    assert_eq!(clean(session_eval("expand((x - 2)^3)")), "x^3 - 6x^2 + 12x - 8");
}

#[test]
fn stress_poly_expansion_multivariable() {
    let res = session_eval("expand((x + y)^3)");
    assert!(res.contains("x^3") && res.contains("y^3"), "got {}", res);

    let res2 = session_eval("expand((x + y + z)^2)");
    assert!(res2.contains("x^2") && res2.contains("y^2") && res2.contains("z^2"), "got {}", res2);
}

#[test]
fn stress_poly_expansion_power_expansion_bounds() {
    // Power expansion in merge_power is bounded by MAX_POWER_EXPANSION = 5 and base terms <= 4
    let res5 = session_eval("expand((x + 1)^5)");
    assert!(res5.contains("x^5"), "expansion of (x+1)^5 should expand power, got {}", res5);

    let res8 = session_eval("expand((x + 1)^8)");
    assert_eq!(res8, "(x + 1)^8", "power > 5 is intentionally kept unexpanded");
}

#[test]
fn stress_poly_expansion_numerical_invariants() {
    let single_var_cases = [
        "(x + 2)(x - 3)^3",
        "(2x - 1)^4",
        "(x^2 - 3x + 2)(x + 4)",
    ];

    let xvar = MathStructure::symbolic("x");
    let eo = EvaluationOptions::default();

    for expr in single_var_cases {
        let unexpanded = parse_and_eval(expr);
        let expanded = parse_and_eval(&format!("expand({})", expr));

        for x_val in [1i64, 2, -3, 7] {
            let v1 = eval_at_num(&unexpanded, &xvar, x_val);
            let v2 = eval_at_num(&expanded, &xvar, x_val);
            let mut diff = v1.clone();
            diff.calculate_subtract(v2.clone(), &eo);
            diff.calculatesub(&eo);
            assert!(diff.is_zero(), "Expansion mismatch for {} at x={}: unexpanded={:?}, expanded={:?}", expr, x_val, v1, v2);
        }
    }

    // Multivariable case
    let unexp_xy = parse_and_eval("(x + y)^4");
    let exp_xy = parse_and_eval("expand((x + y)^4)");
    for (x_val, y_val) in [(1i64, 2i64), (3, -1), (0, 5)] {
        let v1 = eval_at_xy(&unexp_xy, x_val, y_val);
        let v2 = eval_at_xy(&exp_xy, x_val, y_val);
        let mut diff = v1.clone();
        diff.calculate_subtract(v2.clone(), &eo);
        diff.calculatesub(&eo);
        assert!(diff.is_zero(), "Multivariable expansion mismatch for (x+y)^4 at x={}, y={}", x_val, y_val);
    }
}

// ===========================================================================
// 3. DENSE POLYNOMIAL ALGEBRA & GCD STRESS TESTS
// ===========================================================================

#[test]
fn stress_dense_gcd_and_roots() {
    // GCD of (x-1)(x-2)(x-3) and (x-2)(x-3)(x-4) should be (x-2)(x-3) = x^2 - 5x + 6
    let p1 = vec![
        Number::from_i64(-6),
        Number::from_i64(11),
        Number::from_i64(-6),
        Number::from_i64(1),
    ];
    let p2 = vec![
        Number::from_i64(-24),
        Number::from_i64(26),
        Number::from_i64(-9),
        Number::from_i64(1),
    ];

    let gcd_res = dense_gcd(&p1, &p2).expect("dense_gcd");
    assert_eq!(gcd_res.len(), 3);
    // x^2 - 5x + 6
    assert!(gcd_res[0].equals_i64(6));
    assert!(gcd_res[1].equals_i64(-5));
    assert!(gcd_res[2].equals_i64(1));

    let roots = rational_roots(&p1);
    let mut root_ints: Vec<i64> = roots.iter().filter_map(|r| r.to_i64()).collect();
    root_ints.sort_unstable();
    assert_eq!(root_ints, vec![1, 2, 3]);
}
