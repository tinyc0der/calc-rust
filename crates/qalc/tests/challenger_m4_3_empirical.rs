//! Empirical test suite for Challenger 3 M4 (Iteration 2 radical square factoring fixes).

use std::process::Command;
use qalc_core::{Session, ApproximationMode};

fn eval_cli(expr: &str) -> String {
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

fn eval_session_exact(expr: &str) -> String {
    let mut session = Session::new();
    session.eval_options.approximation = ApproximationMode::Exact;
    session.eval_options.split_squares = true;
    session.evaluate_line(expr).unwrap_or_else(|e| format!("ERROR: {e}"))
}

fn eval_session_try_exact(expr: &str) -> String {
    let mut session = Session::new();
    session.eval_options.approximation = ApproximationMode::TryExact;
    session.eval_options.split_squares = true;
    session.evaluate_line(expr).unwrap_or_else(|e| format!("ERROR: {e}"))
}

fn eval_session_approx(expr: &str) -> String {
    let mut session = Session::new();
    session.eval_options.approximation = ApproximationMode::Approximate;
    session.eval_options.split_squares = true;
    session.evaluate_line(expr).unwrap_or_else(|e| format!("ERROR: {e}"))
}

#[test]
fn test_m4_3_required_radical_edge_cases() {
    // 1. sqrt(8/9) -> 2/3 * sqrt(2) or 2 * sqrt(2) / 3 or (2 * sqrt(2)) / 3
    let res_8_9_exact = eval_session_exact("sqrt(8/9)");
    let res_8_9_try = eval_session_try_exact("sqrt(8/9)");
    println!("sqrt(8/9) [Exact] = {}", res_8_9_exact);
    println!("sqrt(8/9) [TryExact] = {}", res_8_9_try);

    // 2. sqrt(1/12) -> 1/6 * sqrt(3) or sqrt(3) / 6
    let res_1_12_exact = eval_session_exact("sqrt(1/12)");
    let res_1_12_try = eval_session_try_exact("sqrt(1/12)");
    println!("sqrt(1/12) [Exact] = {}", res_1_12_exact);
    println!("sqrt(1/12) [TryExact] = {}", res_1_12_try);

    // 3. sqrt(32) * sqrt(2) -> 8
    let res_32_2_exact = eval_session_exact("sqrt(32) * sqrt(2)");
    let res_32_2_try = eval_session_try_exact("sqrt(32) * sqrt(2)");
    let res_32_2_cli = eval_cli("sqrt(32) * sqrt(2)");
    println!("sqrt(32) * sqrt(2) [Exact] = {}", res_32_2_exact);
    println!("sqrt(32) * sqrt(2) [TryExact] = {}", res_32_2_try);
    println!("sqrt(32) * sqrt(2) [CLI] = {}", res_32_2_cli);

    let res_32_approx = eval_session_approx("sqrt(32)");
    let res_32_exact = eval_session_exact("sqrt(32)");
    let res_32_cli = eval_cli("sqrt(32)");
    println!("sqrt(32) [Approx] = {}", res_32_approx);
    println!("sqrt(32) [Exact] = {}", res_32_exact);
    println!("sqrt(32) [CLI] = {}", res_32_cli);

    // 4. sqrt(-12) -> 2i * sqrt(3)
    let res_neg12_exact = eval_session_exact("sqrt(-12)");
    let res_neg12_try = eval_session_try_exact("sqrt(-12)");
    let res_neg12_cli = eval_cli("sqrt(-12)");
    println!("sqrt(-12) [Exact] = {}", res_neg12_exact);
    println!("sqrt(-12) [TryExact] = {}", res_neg12_try);
    println!("sqrt(-12) [CLI] = {}", res_neg12_cli);
}

#[test]
fn test_m4_3_additional_radical_cases() {
    let cases = vec![
        ("sqrt(50/49)", "exact/try_exact radical rationalization"),
        ("sqrt(18/25)", "exact/try_exact radical rationalization"),
        ("sqrt(-32)", "negative radicand square extraction"),
        ("sqrt(-1/4)", "negative rational perfect square"),
        ("sqrt(0)", "zero radicand"),
        ("sqrt(1)", "unity radicand"),
        ("sqrt(4/9)", "rational perfect square"),
        ("sqrt(2) * sqrt(3)", "coprime square root product"),
        ("sqrt(8) / sqrt(2)", "radical division simplification"),
        ("sqrt(-1)", "imaginary unit"),
        ("sqrt(-4)", "negative integer square"),
        ("sqrt(-8)", "negative non-perfect square"),
        ("sqrt(1/27)", "odd power fraction denominator"),
        ("sqrt(3/4)", "half integer factor"),
        ("sqrt(125/18)", "large composite fraction"),
    ];

    for (expr, label) in cases {
        let exact = eval_session_exact(expr);
        let try_exact = eval_session_try_exact(expr);
        let cli = eval_cli(expr);
        println!("{expr} ({label}) => Exact: '{exact}' | TryExact: '{try_exact}' | CLI: '{cli}'");
    }
}
