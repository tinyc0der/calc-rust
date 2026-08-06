//! Empirical Stress Test Suite created by Challenger 2 for Milestone 2 Iteration 2.

use qalc::cli::{evaluate_cli_line, new_session};
use std::thread;

fn eval(expr: &str) -> String {
    let mut session = new_session();
    match evaluate_cli_line(&mut session, expr) {
        Ok(s) => s,
        Err(e) => format!("ERROR: {e}"),
    }
}

// ===========================================================================
// 1. THREAD SAFETY & RE-ENTRANCY DEADLOCK PREVENTION
// ===========================================================================

#[test]
fn test_parallel_session_initialization_deadlock_free() {
    let handles: Vec<_> = (0..16)
        .map(|i| {
            thread::spawn(move || {
                let res = eval("sqrt 4 + 5");
                assert_eq!(res, "7", "Thread {} failed evaluation", i);
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

// ===========================================================================
// 2. DISAMBIGUATION & UNIT MULTIPLICATION
// ===========================================================================

#[test]
fn test_unit_vs_function_disambiguation_full() {
    // psi: unit pressure (psi) vs function digamma(x)
    assert_eq!(eval("psi(4)"), "1.256117668");
    let res_psi_unit = eval("50 psi");
    assert!(res_psi_unit.contains("Pa") || res_psi_unit.contains("psi"));
    let res_psi_impl = eval("psi 4");
    assert_ne!(res_psi_impl, "1.256117668");

    // min: unit minute vs function min(a; b)
    assert_eq!(eval("min(1; 2)"), "1");
    let res_min_unit = eval("10 min");
    assert!(res_min_unit.contains("min") || res_min_unit.contains("s"));
    let res_min_impl = eval("min 1");
    assert!(res_min_impl.contains("min") || res_min_impl.contains("s"));

    // m: unit meters vs function/variable m
    assert_eq!(eval("100 m"), "100 m");
    assert_eq!(eval("m(5)"), "5 m");
    assert_eq!(eval("m 5"), "5 m");
}

// ===========================================================================
// 3. IMPLICIT PARSING & NESTED FUNCTIONS
// ===========================================================================

#[test]
fn test_implicit_parsing_and_nesting() {
    assert_eq!(eval("sqrt 4"), "2");
    assert_eq!(eval("ln 25"), "3.218875825");
    assert_eq!(eval("sin 0"), "0");
    assert_eq!(eval("abs -5"), "5");

    // Chained/nested implicit function calls
    assert_eq!(eval("sqrt sqrt 16"), "2");
    assert_eq!(eval("2 sqrt 4"), "4");
    assert_eq!(eval("sqrt(sqrt(16))"), "2");
    assert_eq!(eval("ln(exp(5))"), "5");
}

// ===========================================================================
// 4. VECTOR DISTRIBUTION & EDGE CASES
// ===========================================================================

#[test]
fn test_vector_distribution_edge_cases() {
    // Multi-arg distribution via semicolon
    assert_eq!(eval("sqrt(25; 16; 9; 4)"), "[5  4  3  2]");

    // Vector argument
    assert_eq!(eval("sqrt([25 16 9 4])"), "sqrt([25  16  9  4])");

    // Empty vector
    let res_empty = eval("sqrt([])");
    assert_eq!(res_empty, "sqrt([])");

    // Single element vector
    assert_eq!(eval("sqrt([25])"), "5");

    // Unary functions elementwise
    assert_eq!(eval("abs(-1; -2; 3)"), "[1  2  3]");
    assert_eq!(eval("ln(1; 1)"), "[0  0]");

    // Multi-parameter functions (must NOT distribute elementwise)
    assert_eq!(eval("min(1; 2)"), "1");
    assert_eq!(eval("atan2(1; 1)"), "0.7853981634");
}
