//! Empirical stress test harness for Milestone 3 Issue #1 (Log Ratios) and Issue #2 (Trig Pi Evaluation).

use qalc_core::session::Session;

fn eval(expr: &str) -> String {
    let mut s = Session::new();
    s.evaluate_line(expr).expect("evaluate expression")
}

fn eval_exact(expr: &str) -> String {
    let mut s = Session::new();
    s.evaluate_line("/set approximation exact").ok();
    s.evaluate_line(expr).expect("evaluate expression in exact mode")
}

// ===========================================================================
// Issue #1: Log Ratio & Exact Logarithm Stress Tests
// ===========================================================================

#[test]
fn test_m3_1_log_ratio_basic() {
    assert_eq!(eval("log2(4)/log10(100)"), "1");
    assert_eq!(eval("log(4; 2) / log(100; 10)"), "1");
    assert_eq!(eval("log2(8) / log(27; 3)"), "1");
    assert_eq!(eval("log2(16) / log2(4)"), "2");
}

#[test]
fn test_m3_1_log_ratio_signs_and_reciprocals() {
    assert_eq!(eval("log2(1/4) / log10(100)"), "-1");
    assert_eq!(eval("log2(4) / log10(1/100)"), "-1");
    assert_eq!(eval("log2(1/4) / log10(1/100)"), "1");
    assert_eq!(eval("log2(1/8) / log(27; 3)"), "-1");
}

#[test]
fn test_m3_1_log_rational_bases_and_arguments() {
    assert_eq!(eval("log(8/27; 4/9)"), "1.5");
    assert_eq!(eval("log(27/8; 4/9)"), "-1.5");
    assert_eq!(eval("log(4/9; 8/27)"), "0.6666666667");
    assert_eq!(eval("log(1/1000; 1/10)"), "3");
    assert_eq!(eval("log(1/1000; 10)"), "-3");
    assert_eq!(eval("log(100; 1/10)"), "-2");
}

#[test]
fn test_m3_1_log_large_powers() {
    assert_eq!(eval("log2(2^50)"), "50");
    assert_eq!(eval("log10(10^50)"), "50");
    assert_eq!(eval("log(3^20; 3)"), "20");
}

#[test]
fn test_m3_1_log_power_matrix_exhaustive() {
    // Test base^m / base^n for bases 2, 3, 5, 10 and exponents m, n in -4..4 (m, n != 0)
    let bases = [2, 3, 5, 10];
    let exps = [-4, -3, -2, -1, 1, 2, 3, 4];
    for &b in &bases {
        for &m in &exps {
            for &n in &exps {
                let expr = format!("log({b}^{m}; {b}^{n})");
                let expected_ratio = (m as f64) / (n as f64);
                let res_str = eval(&expr);
                let res_val: f64 = res_str.parse().unwrap_or_else(|_| panic!("failed to parse result '{res_str}' for '{expr}'"));
                assert!((res_val - expected_ratio).abs() < 1e-6, "Failed for {expr}: expected {expected_ratio}, got {res_str}");
            }
        }
    }
}

#[test]
fn test_m3_1_log_edge_cases_no_panic() {
    // Evaluating invalid base or argument should not panic
    let _ = eval("log(5; 1)");
    let _ = eval("log(5; 0)");
    let _ = eval("log(0; 5)");
    let _ = eval("log(1; 5)");
    assert_eq!(eval("log(1; 5)"), "0");
    let _ = eval("log(4; -2)");
    let _ = eval("log(-4; 2)");
    let _ = eval("log(1; 1)");
    let _ = eval("log(0; 0)");
}

#[test]
fn test_m3_1_log_exact_mode() {
    assert_eq!(eval_exact("log2(4)/log10(100)"), "1");
    assert_eq!(eval_exact("log2(8)"), "3");
    assert_eq!(eval_exact("log(10; 100)"), "0.5");
    assert_eq!(eval_exact("log(100; 10)"), "2");
}

// ===========================================================================
// Issue #2: Trig Pi Angle Evaluation & Phase Shift Stress Tests
// ===========================================================================

#[test]
fn test_m3_2_trig_pi_cardinal_angles() {
    assert_eq!(eval("sin(0)"), "0");
    assert_eq!(eval("cos(0)"), "1");
    assert_eq!(eval("tan(0)"), "0");
    assert_eq!(eval("sin(pi/2)"), "1");
    assert_eq!(eval("cos(pi/2)"), "0");
    assert_eq!(eval("sin(pi)"), "0");
    assert_eq!(eval("cos(pi)"), "-1");
    assert_eq!(eval("tan(pi)"), "0");
}

#[test]
fn test_m3_2_trig_pi_special_angles() {
    assert_eq!(eval("sin(pi/6)"), "0.5");
    assert_eq!(eval("cos(pi/3)"), "0.5");
    assert_eq!(eval("tan(pi/4)"), "1");
    assert_eq!(eval("cot(pi/4)"), "1");
    assert_eq!(eval("sin(pi/4)"), "1 / sqrt(2)");
    assert_eq!(eval("cos(pi/4)"), "1 / sqrt(2)");
}

#[test]
fn test_m3_2_trig_pi_quadrants_and_multiples() {
    assert_eq!(eval("sin(3*pi/2)"), "-1");
    assert_eq!(eval("cos(3*pi/2)"), "0");
    assert_eq!(eval("sin(2*pi)"), "0");
    assert_eq!(eval("cos(2*pi)"), "1");
    assert_eq!(eval("tan(2*pi)"), "0");
    assert_eq!(eval("sin(5*pi/2)"), "1");
    assert_eq!(eval("cos(5*pi/2)"), "0");
    assert_eq!(eval("sin(100*pi)"), "0");
    assert_eq!(eval("cos(100*pi)"), "1");
    assert_eq!(eval("sin(101*pi)"), "0");
    assert_eq!(eval("cos(101*pi)"), "-1");
}

#[test]
fn test_m3_2_trig_pi_exhaustive_integer_multiples() {
    for k in -20..=20 {
        let sin_expr = format!("sin({k} * pi)");
        let cos_expr = format!("cos({k} * pi)");
        let tan_expr = format!("tan({k} * pi)");

        let expected_sin = "0";
        let expected_cos = if k % 2 == 0 { "1" } else { "-1" };
        let expected_tan = "0";

        assert_eq!(eval(&sin_expr), expected_sin, "Failed for {sin_expr}");
        assert_eq!(eval(&cos_expr), expected_cos, "Failed for {cos_expr}");
        assert_eq!(eval(&tan_expr), expected_tan, "Failed for {tan_expr}");
    }
}

#[test]
fn test_m3_2_trig_pi_exhaustive_half_multiples() {
    for k in -20..=20 {
        // (2k + 1) * pi / 2
        let n: i64 = 2 * k + 1;
        let sin_expr = format!("sin({n} * pi / 2)");
        let cos_expr = format!("cos({n} * pi / 2)");

        // n mod 4:
        // n = 1 mod 4 -> sin = 1, cos = 0
        // n = 3 mod 4 -> sin = -1, cos = 0
        let rem = n.rem_euclid(4);
        let expected_sin = if rem == 1 { "1" } else { "-1" };
        let expected_cos = "0";

        assert_eq!(eval(&sin_expr), expected_sin, "Failed for {sin_expr}");
        assert_eq!(eval(&cos_expr), expected_cos, "Failed for {cos_expr}");
    }
}

#[test]
fn test_m3_2_trig_pi_negative_angles() {
    assert_eq!(eval("sin(-pi/6)"), "-0.5");
    assert_eq!(eval("cos(-pi/3)"), "0.5");
    assert_eq!(eval("tan(-pi/4)"), "-1");
    assert_eq!(eval("sin(-pi/2)"), "-1");
    assert_eq!(eval("cos(-pi/2)"), "0");
    assert_eq!(eval("sin(-pi)"), "0");
    assert_eq!(eval("cos(-pi)"), "-1");
    assert_eq!(eval("sin(-3*pi/2)"), "1");
    assert_eq!(eval("cos(-3*pi/2)"), "0");
    assert_eq!(eval("sin(-2*pi)"), "0");
    assert_eq!(eval("cos(-2*pi)"), "1");
}

#[test]
fn test_m3_2_trig_pi_undefined_cases_no_panic() {
    // Asymptote values should not panic
    let _ = eval("tan(pi/2)");
    let _ = eval("tan(3*pi/2)");
    let _ = eval("tan(-pi/2)");
    let _ = eval("cot(0)");
    let _ = eval("cot(pi)");
}

#[test]
fn test_m3_2_trig_pi_identities_and_combinations() {
    assert_eq!(eval("sin(pi/2) - cos(pi)"), "2");
    assert_eq!(eval("sin(pi/6) + cos(pi/3)"), "1");
    assert_eq!(eval("sin(pi/2)^2 + cos(pi/2)^2"), "1");
    assert_eq!(eval("sin(pi/4)^2 + cos(pi/4)^2"), "1");
    assert_eq!(eval("sin(pi/3)^2 + cos(pi/3)^2"), "1");
    assert_eq!(eval("tan(pi/4) * cot(pi/4)"), "1");
    assert_eq!(eval("sin(pi/2) * cos(0) - cos(pi/2) * sin(0)"), "1");
}

#[test]
fn test_m3_2_trig_pi_exact_mode() {
    assert_eq!(eval_exact("sin(pi/2) - cos(pi)"), "2");
    assert_eq!(eval_exact("sin(pi/4)"), "1 / sqrt(2)");
    assert_eq!(eval_exact("cos(pi/3)"), "0.5");
    assert_eq!(eval_exact("tan(pi/4)"), "1");
}
