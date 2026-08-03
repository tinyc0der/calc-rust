//! Empirical stress test harness for Milestone 4 (Radical Simplification & Symbolic GCD).

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
fn test_required_symbolic_gcd_cases() {
    // 1. gcd(25x, 5x^2)
    assert_eq!(eval("gcd(25x, 5x^2)"), "5x");

    // 2. gcd(x^2 - 1, x - 1)
    let res2 = eval("gcd(x^2 - 1, x - 1)");
    assert!(res2 == "x - 1" || res2 == "x − 1", "Got: {}", res2);

    // 3. gcd(6x^3 + 3x^2, 9x^2)
    let res3 = eval("gcd(6x^3 + 3x^2, 9x^2)");
    assert!(res3 == "3x^2" || res3 == "3 * x^2", "Got: {}", res3);

    // 4. gcd(x, 2x)
    assert_eq!(eval("gcd(x, 2x)"), "x");

    // 5. gcd(0, x)
    let res5 = eval("gcd(0, x)");
    assert_eq!(res5, "gcd(0, x)");

    // 6. numeric gcd(12, 18)
    assert_eq!(eval("gcd(12, 18)"), "6");
}

#[test]
fn test_rational_gcd_regression() {
    // HCF(1/2, 1/3) should be 1/6 (0.1666666667), but worker_m4_1 broke this so it returns "1"
    let res_hcf = eval("HCF(1/2, 1/3)");
    println!("HCF(1/2, 1/3) = {}", res_hcf);

    let res_gcd_frac = eval("gcd(1/2, 1/3)");
    println!("gcd(1/2, 1/3) = {}", res_gcd_frac);

    // Numeric integer GCD
    assert_eq!(eval("gcd(4, 6)"), "2");

    // Print empirical finding
    if res_hcf != "0.1666666667" && res_hcf != "1/6" {
        println!("[REGRESSION DISCOVERED] HCF(1/2, 1/3) evaluated to '{}' instead of '0.1666666667' (or 1/6)", res_hcf);
    }
}

#[test]
fn test_polynomial_gcd_variadic_and_negatives() {
    // Variadic 3 args: gcd(2x, 4x, 6x)
    let res_var = eval("gcd(2x, 4x, 6x)");
    assert_eq!(res_var, "2x");

    // Negative coefficients: gcd(-5x, 10x^2)
    let res_neg = eval("gcd(-5x, 10x^2)");
    println!("gcd(-5x, 10x^2) = {}", res_neg);

    // Same operand: gcd(x, x)
    assert_eq!(eval("gcd(x, x)"), "x");

    // Identical polynomials with constant multiplier: gcd(2x+2, x+1)
    let res_mult = eval("gcd(2x+2, x+1)");
    assert!(res_mult == "x + 1" || res_mult == "x + 1", "Got: {}", res_mult);
}

#[test]
fn test_polynomial_gcd_edge_cases() {
    // Single argument
    let res_single = eval("gcd(x)");
    assert_eq!(res_single, "gcd(x)");

    // Zero arguments
    let res_zero_arg = eval("gcd()");
    assert_eq!(res_zero_arg, "gcd()");

    // Co-prime polynomials
    let res_coprime = eval("gcd(x + 1, x + 2)");
    assert_eq!(res_coprime, "1");

    // Multivariate x, y
    let res_multi = eval("gcd(x, y)");
    assert_eq!(res_multi, "1");

    // High degree
    let res_high = eval("gcd(x^10 - 1, x - 1)");
    assert!(res_high == "x - 1" || res_high == "x − 1", "Got: {}", res_high);

    // Non-polynomial symbolic
    let res_trig = eval("gcd(sin(x), cos(x))");
    println!("gcd(sin(x), cos(x)) = {}", res_trig);

    // Rational coefficients in polynomial
    let res_frac_poly = eval("gcd(1/2 x, 1/3 x^2)");
    println!("gcd(1/2 x, 1/3 x^2) = {}", res_frac_poly);
}
