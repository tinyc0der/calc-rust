use qalc::cli::{evaluate_cli_line, new_session};

fn eval(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression")
}

fn eval_exact(expr: &str) -> String {
    let mut s = new_session();
    evaluate_cli_line(&mut s, "/set approximation exact").ok();
    evaluate_cli_line(&mut s, expr).expect("evaluate expression in exact mode")
}

#[test]
fn test_hp_conversions_detailed() {
    assert_eq!(eval("100 lbf * 60 mph to hp"), "15.999998 hp");
    assert_eq!(eval("200 lbf * 120 mph to hp"), "63.999990 hp");
    assert_eq!(eval("550 ft * lbf / s to hp"), "0.99999985 hp");
    assert_eq!(eval("1100 ft * lbf / s to hp"), "1.9999997 hp");
    assert_eq!(eval("745.69987158227022 W to hp"), "0.99999985 hp");
}

#[test]
fn test_exact_log_ratios_detailed() {
    assert_eq!(eval("log2(4)/log10(100)"), "1");
    assert_eq!(eval("log(27; 3)/log(8; 2)"), "1");
    assert_eq!(eval("log2(16)/log(16; 4)"), "2");
    assert_eq!(eval("log10(1000)/log10(10)"), "3");
    assert_eq!(eval("log2(1/4)/log10(100)"), "−1");
    assert_eq!(eval("log2(4)/log10(1/100)"), "−1");
    assert_eq!(eval("log2(1/8)"), "−3");
    assert_eq!(eval("log(8; 1/2)"), "−3");
    assert_eq!(eval("log(1/8; 1/2)"), "3");
}

#[test]
fn test_trig_pi_evaluations_detailed() {
    assert_eq!(eval("sin(pi/2) - cos(pi)"), "2");
    assert_eq!(eval("sin(pi) + cos(pi/2)"), "0");
    assert_eq!(eval("tan(pi/4)"), "1");
    assert_eq!(eval("sin(pi/6) + cos(pi/3)"), "1");
    assert_eq!(eval("sin(-pi/2)"), "−1");
    assert_eq!(eval("cos(-pi)"), "−1");
    assert_eq!(eval("cot(pi/4)"), "1");
}

#[test]
fn test_limit_evaluations_default_mode_detailed() {
    assert_eq!(eval("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
    assert_eq!(eval("limit(sin(x)/x; 0)"), "1");
    assert_eq!(eval("limit((1 + x)^(1/x); 0)"), "e");
    assert_eq!(eval("limit((x^2 - 4)/(x - 2); 2)"), "4");
}

#[test]
fn test_composite_limit_expressions() {
    assert_eq!(eval("1 + limit(sin(x)/x; 0)"), "2");
    assert_eq!(eval("2 * limit((1 + x)^(1/x); 0)"), "2e");
}

#[test]
fn test_limit_evaluations_exact_mode_detailed() {
    assert_eq!(eval_exact("limit(ln(1 + 4x)/(3^x - 1); 0)"), "4 / ln(3)");
    assert_eq!(eval_exact("limit(sin(x)/x; 0)"), "1");
    assert_eq!(eval_exact("limit((x^2 - 4)/(x - 2); 2)"), "4");
}
