//! Empirical Stress Test Suite created by Challenger 2 for Milestone 2.

use qalc::cli::{evaluate_cli_line, new_session};

fn eval(expr: &str) -> String {
    let mut session = new_session();
    match evaluate_cli_line(&mut session, expr) {
        Ok(s) => s,
        Err(e) => format!("ERROR: {e}"),
    }
}

// ===========================================================================
// 1. UNIT VS FUNCTION DISAMBIGUATION EDGE CASES
// ===========================================================================

#[test]
fn test_unit_vs_function_psi() {
    // 50 psi should evaluate to pressure (344737.86 Pa)
    let res_unit = eval("50 psi");
    println!("'50 psi' => {}", res_unit);
    assert!(res_unit.contains("Pa") || res_unit.contains("psi"), "50 psi should be pressure unit: {}", res_unit);

    // psi(4) should evaluate to digamma(4) = 1.2561177 (function call)
    let res_fn_call = eval("psi(4)");
    println!("'psi(4)' => {}", res_fn_call);
    assert_eq!(res_fn_call, "1.2561177");

    // psi 4 - psi is a unit, so psi 4 should evaluate as 4 psi (27579.029 Pa), NOT psi(4)
    let res_implicit = eval("psi 4");
    println!("'psi 4' => {}", res_implicit);
    assert_ne!(res_implicit, "1.2561177", "psi 4 must not evaluate as digamma(4)");
    assert!(res_implicit.contains("Pa") || res_implicit.contains("psi"), "psi 4 should evaluate as unit 4 psi: {}", res_implicit);
}

#[test]
fn test_unit_vs_function_min() {
    // 10 min should evaluate to 10 minutes (600 s or 10 min)
    let res_unit = eval("10 min");
    println!("'10 min' => {}", res_unit);
    assert!(res_unit.contains("min") || res_unit.contains("s"), "10 min should be minutes: {}", res_unit);

    // min(1; 2) should evaluate to minimum function returning 1
    let res_fn_call = eval("min(1; 2)");
    println!("'min(1; 2)' => {}", res_fn_call);
    assert_eq!(res_fn_call, "1");

    // min 1 - min is a unit, so min 1 should evaluate as 1 min (60 s), NOT min(1)
    let res_implicit = eval("min 1");
    println!("'min 1' => {}", res_implicit);
    assert!(res_implicit.contains("min") || res_implicit.contains("s"), "min 1 should evaluate as unit 1 min: {}", res_implicit);
}

#[test]
fn test_unit_vs_function_m() {
    // 100 m should be 100 meters
    let res_unit = eval("100 m");
    println!("'100 m' => {}", res_unit);
    assert_eq!(res_unit, "100 m");

    // m(5) - parenthesized expression with m
    let res_paren = eval("m(5)");
    println!("'m(5)' => {}", res_paren);
    assert_eq!(res_paren, "5 m");

    // m 5 - m is unit meters, so m 5 should be 5 m
    let res_implicit = eval("m 5");
    println!("'m 5' => {}", res_implicit);
    assert_eq!(res_implicit, "5 m");
}

#[test]
fn test_implicit_functions_non_units() {
    // sqrt, ln, sin, abs are functions and NOT units
    assert_eq!(eval("sqrt 4"), "2");
    assert_eq!(eval("ln 25"), "3.2188758");
    assert_eq!(eval("sin 0"), "0");
    assert_eq!(eval("abs -5"), "5");

    // Chained implicit calls
    assert_eq!(eval("sqrt sqrt 16"), "2");
    assert_eq!(eval("2 sqrt 4"), "4");
}


// ===========================================================================
// 2. VECTOR DISTRIBUTION EDGE CASES
// ===========================================================================

#[test]
fn test_vector_distribution_delimiters_and_structures() {
    // Semicolon-separated multi-arg call
    assert_eq!(eval("sqrt(25; 16; 9; 4)"), "[5  4  3  2]");

    // Vector literal as single argument
    assert_eq!(eval("sqrt([25 16 9 4])"), "sqrt([25  16  9  4])");

    // Single element scalar vs single element vector
    assert_eq!(eval("sqrt(25)"), "5");
    assert_eq!(eval("sqrt([25])"), "5");

    // Empty vector
    let res_empty = eval("sqrt([])");
    println!("'sqrt([])' => {}", res_empty);
    assert_eq!(res_empty, "sqrt([])");

    // Abs and Ln distribution
    assert_eq!(eval("abs(-1; -2; 3)"), "[1  2  3]");
    assert_eq!(eval("ln(1; 1)"), "[0  0]");

    // Multi-argument functions MUST NOT distribute elementwise over arguments
    assert_eq!(eval("min(1; 2)"), "1");
    assert_eq!(eval("atan2(1; 1)"), "0.78539816");
}

#[test]
fn test_matrix_nested_vectors() {
    // Matrix / nested vector behavior
    let res_matrix = eval("sqrt([[25 16] [9 4]])");
    println!("'sqrt([[25 16] [9 4]])' => {}", res_matrix);
    // Unary functions currently bypass distribution for nested vectors
    assert_eq!(res_matrix, "sqrt([25  16; 9  4])");
}
