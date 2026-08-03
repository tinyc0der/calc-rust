use qalc::cli::{evaluate_cli_line, new_session};

fn eval(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression")
}

fn eval_exact(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, "/set approximation exact").ok();
    evaluate_cli_line(&mut s, "/set fr 2").ok();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression in exact mode")
}

fn eval_try_exact(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, "/set approximation try_exact").ok();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression in try_exact mode")
}

fn eval_approx(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, "/set approximation approximate").ok();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression in approximate mode")
}

// ----------------------------------------------------------------------
// Issue #3: Horsepower Conversions & XML Unit Relations Stress Tests
// ----------------------------------------------------------------------

#[test]
fn test_hp_conversions_standard() {
    // 100 lbf * 60 mph to hp -> 15.999998 hp
    assert_eq!(eval("100 lbf * 60 mph to hp"), "15.999998 hp");
    // 200 lbf * 120 mph to hp -> 63.999990 hp (due to SI conversion factors of lbf and mph)
    assert_eq!(eval("200 lbf * 120 mph to hp"), "63.999990 hp");
    assert_eq!(eval("745.69987158227022 W to hp"), "0.99999985 hp");
    assert_eq!(eval("745.699987158227022 W to hp"), "1 hp");
}

#[test]
fn test_hp_conversions_exact_mode() {
    assert_eq!(eval_exact("745.699987158227022 W to hp"), "1 hp");
}

#[test]
fn test_hp_conversions_wattage_and_energy() {
    assert_eq!(eval("1 hp to W"), "745.69999 W");
    assert_eq!(eval("745.699987158227022 W to hp"), "1 hp");
    assert_eq!(eval("10 hp * 1 h to J"), "26845200 J");
}

#[test]
fn test_hp_roundtrip_conversions() {
    let w = eval("100 hp to W");
    let back_to_hp = eval(&format!("{} to hp", w));
    assert_eq!(back_to_hp, "100.00000 hp");
}

// ----------------------------------------------------------------------
// Issue #4: Limits Evaluation Stress Tests
// ----------------------------------------------------------------------

#[test]
fn test_limit_default_approximation_mode() {
    assert_eq!(eval("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
    assert_eq!(eval("limit(sin(x)/x; 0)"), "1");
    assert_eq!(eval("limit((1 + x)^(1/x); 0)"), "e");
    assert_eq!(eval("limit((x^2 - 4)/(x - 2); 2)"), "4");
    assert_eq!(eval("limit((x - 2)/(x^2 - 3x + 2); 2)"), "1");
    assert_eq!(eval("limit((3^x - 1)/(6^x - 1); 0)"), "ln(3) / ln(6)");
    assert_eq!(eval("limit((5^x - 1)/x; 0)"), "ln(5)");
}

#[test]
fn test_limit_exact_approximation_mode() {
    assert_eq!(eval_exact("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
    assert_eq!(eval_exact("limit(sin(x)/x; 0)"), "1");
    assert_eq!(eval_exact("limit((1 + x)^(1/x); 0)"), "e");
    assert_eq!(eval_exact("limit((x^2 - 4)/(x - 2); 2)"), "4");
    assert_eq!(eval_exact("limit(x * sin(pi/x); infinity)"), "pi");
    assert_eq!(eval_exact("limit(acos(sqrt(x^2 + x) - x); infinity)"), "pi / 3");
}

#[test]
fn test_limit_try_exact_mode() {
    assert_eq!(eval_try_exact("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
    assert_eq!(eval_try_exact("limit(sin(x)/x; 0)"), "1");
    assert_eq!(eval_try_exact("limit((x^2 - 4)/(x - 2); 2)"), "4");
}

#[test]
fn test_limit_approximate_mode() {
    assert_eq!(eval_approx("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
    assert_eq!(eval_approx("limit(sin(x)/x; 0)"), "1");
    assert_eq!(eval_approx("limit((x^2 - 4)/(x - 2); 2)"), "4");
}

#[test]
fn test_limit_indeterminate_and_one_sided() {
    assert_eq!(eval("limit(1/x; 0)"), "limit(1 / x, 0)");
    assert_eq!(eval("limit(1/x; 0; x; 1)"), "+∞");
    assert_eq!(eval("limit(1/x; 0; x; -1)"), "−∞");
}

#[test]
fn test_limit_arithmetic_combinations() {
    assert_eq!(eval("1 + limit(sin(x)/x; 0)"), "2");
    assert_eq!(eval("2 * limit((1 + x)^(1/x); 0)"), "2e");
    assert_eq!(eval("limit(sin(x)/x; 0) + limit(cos(x); 0)"), "2");
}

#[test]
fn test_limit_complex_transcendental() {
    assert_eq!(eval_exact("limit((tan(x) - sin(x))/x^3; 0)"), "1/2");
    assert_eq!(eval_exact("limit(x * cot(2x); 0)"), "1/2");
    assert_eq!(eval_exact("limit((1 - sin(x)/x)^(1/ln(x)); 0)"), "e^2");
    assert_eq!(eval_exact("limit(x * (ln(x + 3) - ln(x)); infinity)"), "3");
}
