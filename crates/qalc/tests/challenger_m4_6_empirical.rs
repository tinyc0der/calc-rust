//! Challenger 6 Empirical Test Harness for Milestone 4 Iteration 3.
//! Stress-testing polynomial GCD, term ordering, radical simplification, and workspace regressions.

use qalc::cli::{evaluate_cli_line, new_session};

fn eval(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression")
}

fn eval_exact(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, "/set approximation exact").ok();
    evaluate_cli_line(&mut s, "/set fr 2").ok();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression in exact mode")
}

#[test]
fn test_m4_6_symbolic_gcd_required_case_1() {
    let res = eval("gcd(25x; 5x^2)");
    assert_eq!(res, "5x", "gcd(25x; 5x^2) should evaluate to 5x, got: {}", res);
}

#[test]
fn test_m4_6_symbolic_gcd_required_case_2() {
    let res = eval("gcd(6x^2; 9x)");
    assert_eq!(res, "3x", "gcd(6x^2; 9x) should evaluate to 3x, got: {}", res);
}

#[test]
fn test_m4_6_denest_and_sqrt1_simplification() {
    let res = eval_exact("sqrt(3 - 2*sqrt(2))");
    assert_eq!(res, "sqrt(2) − 1", "sqrt(3 - 2*sqrt(2)) should denest to 'sqrt(2) − 1', got: {}", res);
}

#[test]
fn test_m4_6_denest_term_ordering_left_alone() {
    let res = eval_exact("sqrt(1 + sqrt(2))");
    assert_eq!(res, "sqrt(1 + sqrt(2))", "sqrt(1 + sqrt(2)) should keep term ordering, got: {}", res);
}

#[test]
fn test_m4_6_polynomial_division_regression() {
    let res = eval("(5x^2 + 2)/(x - 3)");
    assert_eq!(res, "5x + 15 + 47 / (x − 3)", "Polynomial division regression, got: {}", res);
}

#[test]
fn test_m4_6_polynomial_expansion_regression() {
    let res = eval("(x + 2)(x - 3)^3");
    assert_eq!(res, "x^4 − 7x^3 + 9x^2 + 27x − 54", "Polynomial expansion regression, got: {}", res);
}

#[test]
fn test_m4_6_transcript_base_sqrt2_regression() {
    let res = eval("sqrt(32) to base sqrt(2)");
    assert_eq!(res, "100000", "sqrt(32) to base sqrt(2) regression, got: {}", res);
}
