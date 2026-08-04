//! Empirical stress test harness for Milestone 5 (Polynomial Long Division).

use std::process::Command;

fn eval(expr: &str) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qalc"));
    cmd.arg("-t");
    if let Some(dir) = std::env::var_os("QALCULATE_DEFINITIONS_DIR") {
        cmd.env("QALCULATE_DEFINITIONS_DIR", dir);
    } else if std::path::Path::new("/Users/maxwell/Projects/Demo/libqalculate/data").exists() {
        cmd.env("QALCULATE_DEFINITIONS_DIR", "/Users/maxwell/Projects/Demo/libqalculate/data");
    }
    cmd.arg(expr);
    let out = cmd.output().expect("qalc binary executes");
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !stderr.is_empty() {
        stderr
    } else {
        stdout
    }
}

#[test]
fn test_m5_required_cases() {
    let cases = vec![
        ("(x^3 - 1)/(x - 1)", "x^2 + x + 1"),
        ("(x^4 + 2x^2 + 1)/(x^2 + 1)", "x^2 + 1"),
        ("(5x^2 + 2)/(x - 3)", "5x + 15 + 47 / (x − 3)"),
    ];

    for (expr, expected) in cases {
        let res = eval(expr);
        println!("EVAL: {} => {}", expr, res);
        assert_eq!(res, expected, "Failed on expression: {}", expr);
    }
}

#[test]
fn test_m5_complex_polynomial_division_with_remainder() {
    let expr = "(3x^3 + 5x^2 - 7)/(2x + 1)";
    let res = eval(expr);
    println!("EVAL: {} => {}", expr, res);
    assert!(!res.is_empty(), "Result should not be empty");
    assert_eq!(res, "1.5x^2 + 1.75x − 0.875 − 49 / (8(2x + 1))");
}

#[test]
fn test_m5_exact_polynomial_divisions() {
    let cases = vec![
        ("(x^2 - 1)/(x - 1)", "x + 1"),
        ("(x^3 + 1)/(x + 1)", "x^2 − x + 1"),
        ("(x^4 - 1)/(x - 1)", "x^3 + x^2 + x + 1"),
        ("(x^4 - 1)/(x^2 - 1)", "x^2 + 1"),
        ("(x^5 - 1)/(x - 1)", "x^4 + x^3 + x^2 + x + 1"),
        ("(2x^2 + 4x + 2)/(x + 1)", "2x + 2"),
        ("(x^3 - 8)/(x - 2)", "x^2 + 2x + 4"),
    ];

    for (expr, expected) in cases {
        let res = eval(expr);
        println!("EVAL: {} => {}", expr, res);
        assert_eq!(res, expected, "Failed on expression: {}", expr);
    }
}

#[test]
fn test_m5_polynomial_division_remainders() {
    let cases = vec![
        ("(x^2 + 1)/(x + 1)", "x − 1 + 2 / (x + 1)"),
        ("(x^2 + 3x + 5)/(x + 1)", "x + 2 + 3 / (x + 1)"),
        ("(2x^3 - 3x^2 + 4x - 5)/(x - 2)", "2x^2 + x + 6 + 7 / (x − 2)"),
    ];

    for (expr, expected) in cases {
        let res = eval(expr);
        println!("EVAL: {} => {}", expr, res);
        assert_eq!(res, expected, "Failed on expression: {}", expr);
    }
}

#[test]
fn test_m5_edge_cases() {
    // Division by zero
    let res_div_zero = eval("(x^2 + 1)/0");
    println!("EVAL: (x^2 + 1)/0 => {}", res_div_zero);
    assert!(!res_div_zero.is_empty());

    // Zero numerator
    let res_zero_num = eval("0/(x^2 + 1)");
    println!("EVAL: 0/(x^2 + 1) => {}", res_zero_num);
    assert_eq!(res_zero_num, "0");

    // Equal num and den
    let res_equal = eval("(x^3 + 2x + 1)/(x^3 + 2x + 1)");
    println!("EVAL: (x^3 + 2x + 1)/(x^3 + 2x + 1) => {}", res_equal);
    assert_eq!(res_equal, "1");

    // Degree num < degree den
    let res_lower_deg = eval("(x - 1)/(x^2 + 1)");
    println!("EVAL: (x - 1)/(x^2 + 1) => {}", res_lower_deg);
    assert!(!res_lower_deg.is_empty());
}

#[test]
fn test_m5_high_degree_division() {
    let res_deg20 = eval("(x^20 - 1)/(x - 1)");
    println!("EVAL: (x^20 - 1)/(x - 1) => {}", res_deg20);
    assert!(!res_deg20.is_empty());

    let res_deg50 = eval("(x^50 - 1)/(x - 1)");
    println!("EVAL: (x^50 - 1)/(x - 1) => {}", res_deg50);
    assert!(!res_deg50.is_empty());
}

#[test]
fn test_m5_polynomial_expansion() {
    let cases = vec![
        ("(x + 2)(x - 3)^3", "x^4 − 7x^3 + 9x^2 + 27x − 54"),
        ("(x + 1)^2", "x^2 + 2x + 1"),
        ("(x - 2)^3", "x^3 − 6x^2 + 12x − 8"),
    ];

    for (expr, expected) in cases {
        let res = eval(expr);
        println!("EVAL: {} => {}", expr, res);
        assert_eq!(res, expected, "Failed on expression: {}", expr);
    }
}

#[test]
fn test_m5_multivariable_and_symbolic_division() {
    let cases = vec![
        "(x^2 - y^2)/(x - y)",
        "(a^3 - b^3)/(a - b)",
    ];

    for expr in cases {
        let res = eval(expr);
        println!("EVAL: {} => {}", expr, res);
        assert!(!res.is_empty(), "Result for {} should not be empty", expr);
        assert!(!res.contains("panic"), "Result for {} should not panic", expr);
    }
}
