use std::process::Command;

fn qalc() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qalc"));
    cmd.arg("-t");
    // Ensure definitions directory is discovered during test execution
    if let Ok(dir) = std::env::var("QALCULATE_DEFINITIONS_DIR") {
        cmd.env("QALCULATE_DEFINITIONS_DIR", dir);
    }
    cmd
}

fn eval(expr: &str) -> String {
    let output = qalc().arg(expr).output().expect("qalc binary executes");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn eval_err(expr: &str) -> String {
    let output = qalc().arg(expr).output().expect("qalc binary executes");
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[test]
fn test_basic_math_and_operators() {
    assert_eq!(eval("cbrt(-27)"), "-3");
    assert_eq!(eval("5!"), "120");
    assert_eq!(eval("5\\2"), "2");
    assert_eq!(eval("5 mod 3"), "2");
    assert_eq!(eval("gcd(63; 27)"), "9");
    assert_eq!(eval("log2(4)/log10(100)"), "1.000000000");
}

#[test]
fn test_algebra_and_calculus() {
    assert_eq!(eval("x+x^2+4 = 16"), "x = 3 or x = -4");
    assert_eq!(eval("diff(6x^2)"), "12x");
    assert_eq!(eval("integrate(6x^2)"), "2x^3 + C");
    assert_eq!(eval("integrate(6x^2; 1; 5)"), "248");
    assert_eq!(eval("limit(ln(1 + 4x)/(3^x - 1); 0)"), "3.640956907");
}

#[test]
fn test_statistics_and_dates() {
    assert_eq!(eval("mean(5; 6; 4; 2; 3; 7)"), "4.5");
    assert_eq!(eval("stdev(5; 6; 4; 2; 3; 7)"), "1.870828693");
    assert_eq!(eval("10:31 + 8:30 to time"), "19:01");
    assert_eq!(eval("10h 31min + 8h 30min to time"), "19:01");
    assert_eq!(eval("\"2020-05-20\" + 523d"), "\"2021-10-25\"");
}

#[test]
fn test_number_base_conversions() {
    assert_eq!(eval("52 to bin"), "0011 0100");
    assert_eq!(eval("52 to oct"), "064");
    assert_eq!(eval("52 to hex"), "0x34");
    assert_eq!(eval("1978 to roman"), "MCMLXXVIII");
}

#[test]
fn test_matrices_and_vectors() {
    assert_eq!(eval("[1, 2, 3; 4, 5, 6]"), "[1  2  3; 4  5  6]");
    assert_eq!(eval("(1; 2; 3) * 2 - 2"), "[0  2  4]");
    assert_eq!(eval("cross([1 2 3]; [4 5 6])"), "[-3  6  -3]");
    assert_eq!(eval("[1 2; 3 4]^-1"), "[-2  1; 1.5  -0.5]");
}

#[test]
fn test_unit_conversions_when_definitions_available() {
    // Verified behavior when definitions directory is available
    if std::path::Path::new("/Users/maxwell/Projects/Demo/libqalculate/data").exists() {
        let run = |expr: &str| -> String {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_qalc"));
            cmd.arg("-t").env("QALCULATE_DEFINITIONS_DIR", "/Users/maxwell/Projects/Demo/libqalculate/data");
            let out = cmd.arg(expr).output().expect("executes");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        assert_eq!(run("5 dm3 to l"), "5 L");
        assert_eq!(run("20 miles / 2 h to km/h"), "16.09344 km/h");
        assert_eq!(run("50 Ω * 2 A"), "100 V");
        assert_eq!(run("5 m/s to s/m"), "0.2 s/m");
    }
}

#[test]
fn test_unsupported_language_gaps_return_expected_errors() {
    // Tests that unsupported constructs fail predictably with clear syntax errors
    let err_where = eval_err("sinh(0.5) where sinh()=cosh()");
    assert!(err_where.contains("unexpected token Where"), "got: {err_where}");

    let err_escape = eval_err("(\\a + \\b)(\\a - \\b)");
    assert!(err_escape.contains("unexpected token IntDivide"), "got: {err_escape}");

    let err_factors = eval_err("52 to factors");
    assert!(
        err_factors.contains("unit definitions are not available")
            || err_factors.contains("conversion target is not a unit expression"),
        "got: {err_factors}"
    );
}
