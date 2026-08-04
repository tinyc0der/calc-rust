//! Empirical stress test harness for Milestone 5 (Polynomial Power Expansion).

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
fn test_m5_required_expansion_cases() {
    let res1 = eval("(x + 2)(x - 3)^3");
    println!("EVAL: (x + 2)(x - 3)^3 => {}", res1);
    assert_eq!(res1, "x^4 − 7x^3 + 9x^2 + 27x − 54");

    let res2 = eval("(x - 1)^4");
    println!("EVAL: (x - 1)^4 => {}", res2);
    assert_eq!(res2, "x^4 − 4x^3 + 6x^2 − 4x + 1");
}

#[test]
fn test_m5_binomial_expansions() {
    let cases = vec![
        ("(x + 1)^2", "x^2 + 2x + 1"),
        ("(x + 1)^3", "x^3 + 3x^2 + 3x + 1"),
        ("(x - 1)^3", "x^3 − 3x^2 + 3x − 1"),
        ("(x + 1)^4", "x^4 + 4x^3 + 6x^2 + 4x + 1"),
        ("(2x + 1)^3", "8x^3 + 12x^2 + 6x + 1"),
        ("(x - 2)^4", "x^4 − 8x^3 + 24x^2 − 32x + 16"),
    ];

    for (expr, expected) in cases {
        let res = eval(expr);
        println!("EVAL: {} => {}", expr, res);
        assert_eq!(res, expected, "Failed on expression: {}", expr);
    }
}

#[test]
fn test_m5_product_of_powers() {
    let res1 = eval("(2x + 3)^3 * (x - 1)^2");
    println!("EVAL: (2x + 3)^3 * (x - 1)^2 => {}", res1);
    assert!(!res1.is_empty());
    assert!(!res1.contains("panic"));
    assert_eq!(res1, "8x^5 + 20x^4 − 10x^3 − 45x^2 + 27");

    let res2 = eval("(x + 1)^2 * (x - 1)^2");
    println!("EVAL: (x + 1)^2 * (x - 1)^2 => {}", res2);
    assert_eq!(res2, "x^4 − 2x^2 + 1");

    let res3 = eval("(x + 1)^3 * (x - 1)^3");
    println!("EVAL: (x + 1)^3 * (x - 1)^3 => {}", res3);
    assert_eq!(res3, "x^6 − 3x^4 + 3x^2 − 1");
}

#[test]
fn test_m5_multinomial_expansions() {
    let res1 = eval("(x^2 + x + 1)^2");
    println!("EVAL: (x^2 + x + 1)^2 => {}", res1);
    assert_eq!(res1, "x^4 + 2x^3 + 3x^2 + 2x + 1");

    let res2 = eval("(x^2 - 2x + 1)^2");
    println!("EVAL: (x^2 - 2x + 1)^2 => {}", res2);
    assert_eq!(res2, "x^4 − 4x^3 + 6x^2 − 4x + 1");
}

#[test]
fn test_m5_expansion_cancellations() {
    let res1 = eval("(x + 1)^3 - (x - 1)^3");
    println!("EVAL: (x + 1)^3 - (x - 1)^3 => {}", res1);
    assert_eq!(res1, "6x^2 + 2");

    let res2 = eval("(x + 1)^4 - (x - 1)^4");
    println!("EVAL: (x + 1)^4 - (x - 1)^4 => {}", res2);
    assert_eq!(res2, "8x^3 + 8x");
}

#[test]
fn test_m5_boundary_and_large_powers() {
    // Exponent 0
    let res0 = eval("(x + 1)^0");
    println!("EVAL: (x + 1)^0 => {}", res0);
    assert_eq!(res0, "1");

    // Exponent 1
    let res1 = eval("(x + 1)^1");
    println!("EVAL: (x + 1)^1 => {}", res1);
    assert_eq!(res1, "x + 1");

    // Large power within bounds (n=10)
    let res10 = eval("(x + 1)^10");
    println!("EVAL: (x + 1)^10 => {}", res10);
    assert!(!res10.is_empty());
    assert!(!res10.contains("panic"));
    assert_eq!(res10, "x^10 + 10x^9 + 45x^8 + 120x^7 + 210x^6 + 252x^5 + 210x^4 + 120x^3 + 45x^2 + 10x + 1");

    // Upper bound n=100
    let res100 = eval("(x + 1)^100");
    println!("EVAL: (x + 1)^100 => {}", res100);
    assert!(!res100.is_empty());
    assert!(!res100.contains("panic"));

    // Exceeding bound (n=101) - should not panic or hang
    let res101 = eval("(x + 1)^101");
    println!("EVAL: (x + 1)^101 => {}", res101);
    assert!(!res101.is_empty());
    assert!(!res101.contains("panic"));

    // Negative power - no expansion expected
    let res_neg = eval("(x + 1)^(-1)");
    println!("EVAL: (x + 1)^(-1) => {}", res_neg);
    assert!(!res_neg.is_empty());
    assert!(!res_neg.contains("panic"));

    // Fractional power
    let res_frac = eval("(x + 1)^(1/2)");
    println!("EVAL: (x + 1)^(1/2) => {}", res_frac);
    assert!(!res_frac.is_empty());
    assert!(!res_frac.contains("panic"));
}

#[test]
fn test_m5_nested_powers_and_multivariable() {
    let res_nested = eval("((x + 1)^2)^2");
    println!("EVAL: ((x + 1)^2)^2 => {}", res_nested);
    assert_eq!(res_nested, "x^4 + 4x^3 + 6x^2 + 4x + 1");

    let res_multi = eval("(x + y)^3");
    println!("EVAL: (x + y)^3 => {}", res_multi);
    assert!(!res_multi.is_empty());
    assert!(!res_multi.contains("panic"));
}
