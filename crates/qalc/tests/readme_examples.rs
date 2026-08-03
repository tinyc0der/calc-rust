//! Individual integration test cases for every example in libqalculate's README.
//!
//! Each example from libqalculate's README is a dedicated `#[test]` function.
//! Examples that match libqalculate pass (`... ok`).
//! Examples with known gaps fail (`... FAILED`), giving an exact count of passing
//! vs failing examples under `cargo test --test readme_examples`.

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

// ===========================================================================
// 1. BASIC FUNCTIONS & OPERATORS
// ===========================================================================

#[test]
fn example_basic_01_sqrt_implicit_parens() {
    assert_eq!(eval("sqrt 4"), "2");
}

#[test]
fn example_basic_02_sqrt_vector_distrib() {
    assert_eq!(eval("sqrt(25; 16; 9; 4)"), "[5 4 3 2]");
}

#[test]
fn example_basic_03_sqrt_exact_symbolic() {
    assert_eq!(eval("sqrt(32)"), "4 * sqrt(2)");
}

#[test]
fn example_basic_04_cbrt_negative() {
    assert_eq!(eval("cbrt(-27)"), "−3");
}

#[test]
fn example_basic_05_principal_root_complex() {
    assert_eq!(eval("(-27)^(1/3)"), "1.5 + 2.5980762i");
}

#[test]
fn example_basic_06_ln_implicit() {
    assert_eq!(eval("ln 25"), "3.2188758");
}

#[test]
fn example_basic_07_log_ratio() {
    assert_eq!(eval("log2(4)/log10(100)"), "1");
}

#[test]
fn example_basic_08_factorial() {
    assert_eq!(eval("5!"), "120");
}

#[test]
fn example_basic_09_integer_division() {
    assert_eq!(eval("5\\2"), "2");
}

#[test]
fn example_basic_10_modulus() {
    assert_eq!(eval("5 mod 3"), "2");
}

#[test]
fn example_basic_11_to_factors() {
    assert_eq!(eval("52 to factors"), "2^2 * 13");
}

#[test]
fn example_basic_12_to_fraction() {
    assert_eq!(eval("25/4 * 3/5 to fraction"), "3 + 3/4");
}

#[test]
fn example_basic_13_gcd() {
    assert_eq!(eval("gcd(63; 27)"), "9");
}

#[test]
fn example_basic_14_trig_expression() {
    assert_eq!(eval("sin(pi/2) - cos(pi)"), "2");
}

#[test]
fn example_basic_15_sum_range() {
    assert_eq!(eval("sum(x; 1; 5)"), "15");
}

#[test]
fn example_basic_16_product_range() {
    assert_eq!(eval("product(x; 1; 5)"), "120");
}

#[test]
fn example_basic_17_where_clause() {
    assert_eq!(eval("sinh(0.5) where sinh()=cosh()"), "1.1276260");
}

// ===========================================================================
// 2. UNITS
// ===========================================================================

#[test]
fn example_unit_01_volume_conversion() {
    assert_eq!(eval("5 dm3 to l"), "5 L");
}

#[test]
fn example_unit_02_speed_conversion() {
    assert_eq!(eval("20 miles / 2 h to km/h"), "16.09344 km/h");
}

#[test]
fn example_unit_03_mixed_ft_in() {
    assert_eq!(eval("1.74 to ft"), "5 ft + 8.5039370 in");
}

#[test]
fn example_unit_04_negative_target_ft() {
    assert_eq!(eval("1.74 m to -ft"), "5.7086614 ft");
}

#[test]
fn example_unit_05_power_hp_conversion() {
    assert_eq!(eval("100 lbf * 60 mph to hp"), "15.999998 hp");
}

#[test]
fn example_unit_06_ohm_amp_to_volts() {
    assert_eq!(eval("50 Ω * 2 A"), "100 V");
}

#[test]
fn example_unit_07_pressure_division() {
    assert_eq!(eval("10 N / 5 Pa"), "2 m²");
}

#[test]
fn example_unit_08_reciprocal_speed() {
    assert_eq!(eval("5 m/s to s/m"), "0.2 s/m");
}

// ===========================================================================
// 3. ALGEBRA
// ===========================================================================

#[test]
fn example_algebra_01_polynomial_division() {
    assert_eq!(eval("(5x^2 + 2)/(x - 3)"), "5x + 15 + 47/(x − 3)");
}

#[test]
fn example_algebra_02_backslash_variable_escape() {
    assert_eq!(eval("(\\a + \\b)(\\a - \\b)"), "'a'^2 − 'b'^2");
}

#[test]
fn example_algebra_03_polynomial_expansion() {
    assert_eq!(eval("(x + 2)(x - 3)^3"), "x^4 − 7x^3 + 9x^2 + 27x − 54");
}

#[test]
fn example_algebra_04_factoring_target() {
    assert_eq!(eval("x^4 - 7x^3 + 9x^2 + 27x - 54 to factors"), "(x + 2)(x − 3)^3");
}

#[test]
fn example_algebra_05_where_clause() {
    assert_eq!(eval("cos(x)+3y^2 where x=pi; y=2"), "11");
}

#[test]
fn example_algebra_06_symbolic_gcd() {
    assert_eq!(eval("gcd(25x; 5x^2)"), "5x");
}

#[test]
fn example_algebra_07_quadratic_solver() {
    assert_eq!(eval("x+x^2+4 = 16"), "x = 3 or x = −4");
}

// ===========================================================================
// 4. CALCULUS
// ===========================================================================

#[test]
fn example_calculus_01_derivative() {
    assert_eq!(eval("diff(6x^2)"), "12x");
}

#[test]
fn example_calculus_02_indefinite_integral() {
    assert_eq!(eval("integrate(6x^2)"), "2x^3 + C");
}

#[test]
fn example_calculus_03_definite_integral() {
    assert_eq!(eval("integrate(6x^2; 1; 5)"), "248");
}

#[test]
fn example_calculus_04_limit_evaluation() {
    assert_eq!(eval("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
}

// ===========================================================================
// 5. MATRICES & VECTORS
// ===========================================================================

#[test]
fn example_matrix_01_literal_formatting() {
    assert_eq!(eval("[1, 2, 3; 4, 5, 6]"), "[1  2  3; 4  5  6]");
}

#[test]
fn example_matrix_02_vector_elementwise() {
    assert_eq!(eval("(1; 2; 3) * 2 - 2"), "[0  2  4]");
}

#[test]
fn example_matrix_03_vector_cross_product() {
    assert_eq!(eval("cross([1 2 3]; [4 5 6])"), "[−3  6  −3]");
}

#[test]
fn example_matrix_04_inverse() {
    assert_eq!(eval("[1 2; 3 4]^-1"), "[−2  1; 1.5  −0.5]");
}

// ===========================================================================
// 6. STATISTICS & TIME/DATE
// ===========================================================================

#[test]
fn example_stats_01_mean() {
    assert_eq!(eval("mean(5; 6; 4; 2; 3; 7)"), "4.5");
}

#[test]
fn example_stats_02_stdev() {
    assert_eq!(eval("stdev(5; 6; 4; 2; 3; 7)"), "1.8708287");
}

#[test]
fn example_time_01_addition_hhmm() {
    assert_eq!(eval("10:31 + 8:30 to time"), "19:01");
}

#[test]
fn example_time_02_addition_unit_names() {
    assert_eq!(eval("10h 31min + 8h 30min to time"), "19:01");
}

#[test]
fn example_date_01_addition_days() {
    assert_eq!(eval("\"2020-05-20\" + 523d"), "2021-10-25");
}

// ===========================================================================
// 7. NUMBER BASES
// ===========================================================================

#[test]
fn example_base_01_binary() {
    assert_eq!(eval("52 to bin"), "0011 0100");
}

#[test]
fn example_base_02_octal() {
    assert_eq!(eval("52 to oct"), "064");
}

#[test]
fn example_base_03_hex() {
    assert_eq!(eval("52 to hex"), "0x34");
}

#[test]
fn example_base_04_roman() {
    assert_eq!(eval("1978 to roman"), "MCMLXXVIII");
}
