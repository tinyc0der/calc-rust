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
fn print_all_m4_requested_inputs() {
    let inputs = vec![
        "sqrt(0)",
        "sqrt(1)",
        "sqrt(4)",
        "sqrt(12)",
        "sqrt(32)",
        "sqrt(75)",
        "sqrt(1/4)",
        "sqrt(8/9)",
        "sqrt(5)",
    ];
    for input in inputs {
        println!("EVAL: {} => {}", input, eval(input));
    }
}

#[test]
fn print_rational_radicands() {
    let inputs = vec![
        "sqrt(9/16)",
        "sqrt(1/12)",
        "sqrt(18/25)",
        "sqrt(50/49)",
        "sqrt(12/25)",
    ];
    for input in inputs {
        println!("EVAL: {} => {}", input, eval(input));
    }
}

#[test]
fn print_negative_radicands() {
    let inputs = vec![
        "sqrt(-1)",
        "sqrt(-4)",
        "sqrt(-12)",
        "sqrt(-32)",
    ];
    for input in inputs {
        println!("EVAL: {} => {}", input, eval(input));
    }
}

#[test]
fn print_large_integers() {
    let inputs = vec![
        "sqrt(1000000)",
        "sqrt(4000000)",
        "sqrt(3000000)",
        "sqrt(1200000000)",
    ];
    for input in inputs {
        println!("EVAL: {} => {}", input, eval(input));
    }
}

#[test]
fn print_algebraic_operations() {
    let inputs = vec![
        "sqrt(32) + sqrt(12)",
        "sqrt(32) * sqrt(2)",
        "sqrt(32) / 2",
        "sqrt(32) / sqrt(2)",
    ];
    for input in inputs {
        println!("EVAL: {} => {}", input, eval(input));
    }
}
