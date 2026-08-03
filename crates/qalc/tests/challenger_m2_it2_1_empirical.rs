//! Empirical Stress Test Suite created by Challenger 1 for M2 Iteration 2.

use qalc::cli::{evaluate_cli_line, new_session};
use std::sync::Arc;
use std::thread;

fn eval(expr: &str) -> String {
    let mut session = new_session();
    match evaluate_cli_line(&mut session, expr) {
        Ok(s) => s,
        Err(e) => format!("ERROR: {e}"),
    }
}

// ===========================================================================
// 1. RE-ENTRANCY AND DEADLOCK FREEDOM STRESS TESTS
// ===========================================================================

#[test]
fn test_session_new_deadlock_freedom() {
    // Test sequential session creations
    for _ in 0..10 {
        let mut session = new_session();
        let res = evaluate_cli_line(&mut session, "sqrt 4");
        assert_eq!(res.unwrap(), "2");
    }
}

#[test]
fn test_concurrent_session_initialization() {
    // Test multi-threaded concurrent Session::new() initialization
    let mut handles = vec![];
    for i in 0..10 {
        let handle = thread::spawn(move || {
            let mut session = new_session();
            let expr = if i % 2 == 0 { "sqrt 16" } else { "50 psi" };
            evaluate_cli_line(&mut session, expr)
        });
        handles.push(handle);
    }

    for handle in handles {
        let res = handle.join().expect("Thread panicked");
        assert!(res.is_ok(), "Concurrent evaluation failed: {:?}", res);
    }
}

// ===========================================================================
// 2. IMPLICIT FUNCTION PARSING STRESS TESTS
// ===========================================================================

#[test]
fn test_implicit_function_parsing_variations() {
    assert_eq!(eval("sqrt 4"), "2");
    assert_eq!(eval("ln 25"), "3.2188758");
    assert_eq!(eval("sqrt 0"), "0");
    assert_eq!(eval("sqrt 1"), "1");
    assert_eq!(eval("sqrt 100"), "10");
    assert_eq!(eval("sin 0"), "0");
    assert_eq!(eval("abs -5"), "5");

    // Nested / chained implicit functions
    assert_eq!(eval("sqrt sqrt 16"), "2");
    assert_eq!(eval("sqrt sqrt 81"), "3");
    assert_eq!(eval("2 sqrt 4"), "4");
    assert_eq!(eval("3 sqrt 9"), "9");

    // Parenthesized vs unparenthesized parity
    assert_eq!(eval("sqrt 4"), eval("sqrt(4)"));
    assert_eq!(eval("ln 25"), eval("ln(25)"));
}

// ===========================================================================
// 3. VECTOR DISTRIBUTION STRESS TESTS
// ===========================================================================

#[test]
fn test_vector_distribution_stress() {
    // Semicolon-separated elementwise distribution
    assert_eq!(eval("sqrt(25; 16; 9; 4)"), "[5  4  3  2]");
    assert_eq!(eval("sqrt(100; 81; 64)"), "[10  9  8]");
    assert_eq!(eval("abs(-10; -20; -30)"), "[10  20  30]");

    // Multi-argument non-unary functions should NOT distribute
    assert_eq!(eval("min(1; 2)"), "1");
    assert_eq!(eval("atan2(1; 1)"), "0.78539816");
}

// ===========================================================================
// 4. UNIT VS FUNCTION DISAMBIGUATION & UNIT EXPRESSIONS
// ===========================================================================

#[test]
fn test_unit_expressions_stress() {
    // Unit expressions
    let res_psi = eval("50 psi");
    assert!(res_psi.contains("Pa") || res_psi.contains("psi"), "50 psi: {}", res_psi);

    let res_min = eval("10 min");
    assert!(res_min.contains("min") || res_min.contains("s"), "10 min: {}", res_min);

    let res_m = eval("100 m");
    assert_eq!(res_m, "100 m");

    // Disambiguation between units and function calls
    assert_eq!(eval("psi(4)"), "1.2561177");
    assert_ne!(eval("psi 4"), "1.2561177");

    assert_eq!(eval("min(1; 2)"), "1");
    assert!(eval("min 1").contains("min") || eval("min 1").contains("s"));

    // Unit arithmetic / conversion expressions
    let res_add_min = eval("10 min + 5 min");
    assert!(res_add_min.contains("15 min") || res_add_min.contains("900 s"), "10 min + 5 min: {}", res_add_min);

    let res_conv_min = eval("10 min to s");
    assert_eq!(res_conv_min, "600 s");
}

// ===========================================================================
// 5. BOUNDARY & CORNER CASES
// ===========================================================================

#[test]
fn test_boundary_cases_no_panic() {
    // Test that unusual syntax does not panic
    let _ = eval("sqrt");
    let _ = eval("sqrt ;");
    let _ = eval("sqrt()");
    let _ = eval("sqrt sqrt");
    let _ = eval("psi");
}
