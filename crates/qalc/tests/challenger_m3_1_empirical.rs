//! Empirical stress test harness for Milestone 3 (Log Ratios & Trig Pi Evaluation).

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

// ---------------------------------------------------------------------------
// Issue #1: Log Ratios & Exact Logarithms
// ---------------------------------------------------------------------------

#[test]
fn test_m3_issue1_log_ratio_requested_cases() {
    assert_eq!(eval("log2(8)/log2(2)"), "3");
    assert_eq!(eval("log10(1000)/log10(10)"), "3");
    assert_eq!(eval("log2(4)/log10(100)"), "1");
}

#[test]
fn test_m3_issue1_exact_logarithms_integers() {
    assert_eq!(eval("log2(8)"), "3");
    assert_eq!(eval("log2(1/8)"), "−3");
    assert_eq!(eval("log10(100)"), "2");
    assert_eq!(eval("log10(1/100)"), "−2");
    assert_eq!(eval("log2(1024)"), "10");
    assert_eq!(eval("log2(2^30)"), "30");
}

#[test]
fn test_m3_issue1_exact_logarithms_large_exponents() {
    assert_eq!(eval("log2(2^100)"), "100");
    assert_eq!(eval("log10(10^100)"), "100");
    assert_eq!(eval("log(1/10^50; 10)"), "−50");
}

#[test]
fn test_m3_issue1_exact_logarithms_bases() {
    assert_eq!(eval("log(8; 4)"), "1.5");
    assert_eq!(eval("log(4; 8)"), "0.6666666667");
    assert_eq!(eval("log(1/8; 1/2)"), "3");
    assert_eq!(eval("log(1/2; 1/8)"), "0.3333333333");
    assert_eq!(eval("log(8; 1/2)"), "−3");
    assert_eq!(eval("log(1/2; 8)"), "−0.3333333333");
    assert_eq!(eval("log(10; 10)"), "1");
    assert_eq!(eval("log(1; 10)"), "0");
}

#[test]
fn test_m3_issue1_exact_logarithms_rational_fractions() {
    assert_eq!(eval("log(27/8; 3/2)"), "3");
    assert_eq!(eval("log(3/2; 27/8)"), "0.3333333333");
    assert_eq!(eval("log(27/8; 9/4)"), "1.5");
}

// ---------------------------------------------------------------------------
// Issue #2: Trig Pi Evaluation
// ---------------------------------------------------------------------------

#[test]
fn test_m3_issue2_trig_pi_requested_cases() {
    assert_eq!(eval("sin(3 pi / 2)"), "−1");
    assert_eq!(eval("cos(2 pi)"), "1");
    assert_eq!(eval("tan(pi/4)"), "1");
    assert_eq!(eval("sin(pi/2) - cos(pi)"), "2");
}

#[test]
fn test_m3_issue2_trig_pi_cardinal_angles() {
    assert_eq!(eval("sin(0)"), "0");
    assert_eq!(eval("cos(0)"), "1");
    assert_eq!(eval("tan(0)"), "0");
    assert_eq!(eval("sin(pi)"), "0");
    assert_eq!(eval("cos(pi)"), "−1");
    assert_eq!(eval("tan(pi)"), "0");
    assert_eq!(eval("sin(2 pi)"), "0");
    assert_eq!(eval("cos(3 pi / 2)"), "0");
    assert_eq!(eval("cot(pi / 2)"), "0");
}

#[test]
fn test_m3_issue2_trig_pi_large_multiples() {
    assert_eq!(eval("sin(100 pi)"), "0");
    assert_eq!(eval("cos(100 pi)"), "1");
    assert_eq!(eval("sin(101 pi)"), "0");
    assert_eq!(eval("cos(101 pi)"), "−1");
}

#[test]
fn test_m3_issue2_trig_pi_special_angles() {
    assert_eq!(eval("sin(pi / 6)"), "0.5");
    assert_eq!(eval("cos(pi / 3)"), "0.5");
    assert_eq!(eval("sin(pi / 3)"), "0.8660254038");
    assert_eq!(eval("cos(pi / 6)"), "0.8660254038");
    assert_eq!(eval("sin(pi / 4)"), "0.7071067812");
    assert_eq!(eval("cos(pi / 4)"), "0.7071067812");
    assert_eq!(eval("cot(pi / 4)"), "1");
    assert_eq!(eval("tan(3 pi / 4)"), "−1");
    assert_eq!(eval("tan(7 pi / 4)"), "−1");
}

#[test]
fn test_m3_issue2_trig_pi_negative_and_periodicity() {
    assert_eq!(eval("sin(-pi / 2)"), "−1");
    assert_eq!(eval("cos(-pi)"), "−1");
    assert_eq!(eval("sin(5 pi / 2)"), "1");
    assert_eq!(eval("cos(5 pi)"), "−1");
}

#[test]
fn test_m3_issue2_trig_pi_factor_orderings() {
    assert_eq!(eval("sin(pi * 3 / 2)"), "−1");
    assert_eq!(eval("sin(1.5 * pi)"), "−1");
    assert_eq!(eval("sin(3/2 * pi)"), "−1");
    assert_eq!(eval("sin(1/2 * pi)"), "1");
    assert_eq!(eval("cos(1/2 * pi)"), "0");
    assert_eq!(eval("sin(pi * 1/2)"), "1");
    assert_eq!(eval("cos(pi * 1/2)"), "0");
}

#[test]
fn test_m3_issue2_trig_identity() {
    assert_eq!(eval("sin(pi/4)^2 + cos(pi/4)^2"), "1");
}

// ---------------------------------------------------------------------------
// Combined / Compound Expressions
// ---------------------------------------------------------------------------

#[test]
fn test_m3_combined_expressions() {
    assert_eq!(eval("log2(8) / log2(2) + log10(1000) / log10(10)"), "6");
    assert_eq!(eval("(log2(8) / log2(2)) * sin(pi/2)"), "3");
    assert_eq!(eval("(log10(100) / log2(4)) ^ cos(2 pi)"), "1");
}

// ---------------------------------------------------------------------------
// Issues #3 & #4 (Regression checks)
// ---------------------------------------------------------------------------

#[test]
fn test_m3_issue3_hp_conversion() {
    assert_eq!(eval("100 lbf * 60 mph to hp"), "15.99999752 hp");
}

#[test]
fn test_m3_issue4_limit_evaluation() {
    assert_eq!(eval("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
}
