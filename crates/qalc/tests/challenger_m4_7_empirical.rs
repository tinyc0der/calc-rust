//! Challenger 7 Empirical Test Harness for Milestone 4 (Iteration 4 Gate Evaluation).
//! Stress-testing radical square factoring, perfect squares, nested radicals, and base conversions.

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
fn test_m4_7_square_factor_extraction() {
    assert_eq!(eval("sqrt(32)"), "4 * sqrt(2)");
    assert_eq!(eval("sqrt(18)"), "3 * sqrt(2)");
    assert_eq!(eval("sqrt(12)"), "2 * sqrt(3)");
    assert_eq!(eval("sqrt(50)"), "5 * sqrt(2)");
    assert_eq!(eval("sqrt(72)"), "6 * sqrt(2)");
    assert_eq!(eval("sqrt(200)"), "10 * sqrt(2)");
    assert_eq!(eval("sqrt(108)"), "6 * sqrt(3)");
}

#[test]
fn test_m4_7_perfect_squares() {
    assert_eq!(eval("sqrt(1)"), "1");
    assert_eq!(eval("sqrt(4)"), "2");
    assert_eq!(eval("sqrt(9)"), "3");
    assert_eq!(eval("sqrt(16)"), "4");
    assert_eq!(eval("sqrt(25)"), "5");
    assert_eq!(eval("sqrt(100)"), "10");
    assert_eq!(eval("sqrt(144)"), "12");
}

#[test]
fn test_m4_7_nested_radicals_and_subtraction() {
    // Test denesting in exact mode
    let res = eval_exact("sqrt(3 - 2*sqrt(2))");
    // Normalize unicode minus character (U+2212) or hyphen-minus (U+002D) for robust assertion
    let normalized = res.replace('−', "-");
    assert_eq!(normalized, "sqrt(2) - 1", "sqrt(3 - 2*sqrt(2)) should denest to 'sqrt(2) - 1'");

    // Test sqrt(2) - 1 in default mode
    let res_sub = eval("sqrt(2) - 1");
    assert_eq!(res_sub, "0.41421356");

    // Test sqrt(2) - sqrt(1) in exact mode
    let res_sub_exact = eval_exact("sqrt(2) - sqrt(1)");
    let normalized_sub_exact = res_sub_exact.replace('−', "-");
    assert_eq!(normalized_sub_exact, "sqrt(2) - 1");

    // Test product simplification with sqrt(1)
    let res_mult = eval_exact("2 * sqrt(1)");
    assert_eq!(res_mult, "2");
}

#[test]
fn test_m4_7_base_conversion() {
    assert_eq!(eval("sqrt(32) to base sqrt(2)"), "100000");
}

