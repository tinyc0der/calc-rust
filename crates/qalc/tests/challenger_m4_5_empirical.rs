//! Empirical test suite for Challenger M4_5 (Stress-testing Milestone 4 Iteration 3 radical square factor extraction).

use std::process::Command;
use std::time::Instant;
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

fn eval_session(expr: &str, approx: ApproximationMode, split_sq: bool) -> String {
    let mut session = Session::new();
    session.eval_options.approximation = approx;
    session.eval_options.split_squares = split_sq;
    session.evaluate_line(expr).unwrap_or_else(|e| format!("ERROR: {e}"))
}

#[test]
fn print_all_empirical_results() {
    println!("\n========================================================");
    println!("   M4_5 EMPIRICAL STRESS TEST RESULTS");
    println!("========================================================\n");

    let core_cases = vec![
        "sqrt(32)",
        "sqrt(12)",
        "sqrt(50)",
        "sqrt(9)",
        "sqrt(2)",
    ];

    println!("--- 1. Core Required Cases ---");
    for expr in core_cases {
        let cli = eval_cli(expr);
        let try_exact = eval_session(expr, ApproximationMode::TryExact, true);
        let exact = eval_session(expr, ApproximationMode::Exact, true);
        let approx = eval_session(expr, ApproximationMode::Approximate, true);
        println!("Expr: {:<12} | CLI: {:<16} | TryExact: {:<16} | Exact: {:<12} | Approx: {:<12}",
            expr, cli, try_exact, exact, approx);
    }

    let fraction_cases = vec![
        "sqrt(8/9)",
        "sqrt(12/25)",
        "sqrt(1/12)",
        "sqrt(50/49)",
        "sqrt(18/25)",
        "sqrt(4/9)",
        "sqrt(1/4)",
        "sqrt(72/98)",
    ];

    println!("\n--- 2. Fraction Radicand Cases ---");
    for expr in fraction_cases {
        let cli = eval_cli(expr);
        let try_exact = eval_session(expr, ApproximationMode::TryExact, true);
        let exact = eval_session(expr, ApproximationMode::Exact, true);
        println!("Expr: {:<14} | CLI: {:<20} | TryExact: {:<20} | Exact: {:<16}",
            expr, cli, try_exact, exact);
    }

    let composite_cases = vec![
        "sqrt(32) * sqrt(2)",
        "sqrt(12) + sqrt(27)",
        "sqrt(50) - sqrt(18)",
        "sqrt(12) * sqrt(3)",
        "sqrt(-12)",
        "sqrt(-32)",
        "sqrt(-50)",
        "sqrt(-9)",
        "sqrt(-2)",
    ];

    println!("\n--- 3. Composite & Complex Radical Cases ---");
    for expr in composite_cases {
        let cli = eval_cli(expr);
        let try_exact = eval_session(expr, ApproximationMode::TryExact, true);
        let exact = eval_session(expr, ApproximationMode::Exact, true);
        println!("Expr: {:<20} | CLI: {:<20} | TryExact: {:<20} | Exact: {:<16}",
            expr, cli, try_exact, exact);
    }

    let perf_cases = vec![
        "sqrt(100000000)",
        "sqrt(32000000)",
        "sqrt(999999999999)",
        "sqrt(12345678901234567890)",
        "sqrt(2^30)",
        "sqrt(3^20 * 5)",
    ];

    println!("\n--- 4. Performance & Large Radicand Cases ---");
    for expr in perf_cases {
        let start = Instant::now();
        let cli = eval_cli(expr);
        let dur = start.elapsed();
        println!("Expr: {:<24} | CLI: {:<24} | Time: {:?}", expr, cli, dur);
    }
}
