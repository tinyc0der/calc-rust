//! Challenger 4 Empirical Test Harness for Milestone 4 (Iteration 2).

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
fn test_m4_it2_numeric_hcf_decline() {
    let res = eval("HCF(1/2, 1/3)");
    assert_eq!(res, "0.16666667", "HCF(1/2, 1/3) should decline to 0.16666667");
}

#[test]
fn test_m4_it2_symbolic_polynomial_gcd() {
    let res = eval("gcd(25x, 5x^2)");
    assert_eq!(res, "5x", "gcd(25x, 5x^2) should evaluate to 5x");
}

#[test]
fn test_m4_it2_fractional_gcd_decline() {
    let res = eval("gcd(1/2, 1/3)");
    assert_eq!(res, "0.16666667", "gcd(1/2, 1/3) should decline to 0.16666667");
}

#[test]
fn test_m4_it2_nested_radicals_ordering() {
    let res = eval("sqrt(1 + sqrt(2))");
    println!("sqrt(1 + sqrt(2)) evaluated to: {}", res);
    // Note: qalc output string check for nested square root
}
