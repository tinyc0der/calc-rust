//! The evaluation path the `qalc` binary uses.
//!
//! This lives in the library, not the binary, because the transcript parity
//! test has to drive *exactly* what `--test-file` drives. When it drove
//! `Session::evaluate_line` directly instead, six cases differed — the CLI
//! owns `/set` options that `Calculator` does not, and the adaptive interval
//! display, and both change printed output.

use qalc_core::parser::TargetConversionOption;
use qalc_core::{MathStructure, Number, Session};
use qalc_num::options::{IntervalDisplay, NumberFractionFormat};

thread_local! {
    /// `adaptive_interval_display` (declared src/qalc.cc:82): on until
    /// `/set ivdisp` picks a display explicitly — the CLI clears it at
    /// src/qalc.cc:2211 and restores it at :2203 for the `0`/adaptive value.
    static ADAPTIVE_INTERVAL_DISPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };

    /// Terse output formatting flag (set by `-t` / `--terse` or `/set terse`).
    static TERSE_OUTPUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set terse output formatting in the CLI context.
pub fn set_terse(terse: bool) {
    TERSE_OUTPUT.with(|t| t.set(terse));
}

/// Check if terse output formatting is enabled in the CLI context.
pub fn is_terse() -> bool {
    TERSE_OUTPUT.with(|t| t.get())
}

/// A fresh session with the calculator's own predefined variables installed.
///
/// The C++ registers the imaginary unit as `VARIABLE_ID_I`, a builtin
/// `KnownVariable` holding `Number(0, 1, 0, 1)` (`Calculator.cc`). The port's
/// definition registry does not carry it, so the CLI installs it here — the
/// session's variable table is consulted before every other name source, so
/// `2i - 3` parses to the complex number it does in the reference.
pub fn new_session() -> Session {
    let mut session = Session::new();
    session.eval_options.approximation = qalc_core::ApproximationMode::Approximate;
    session.print_options.use_unicode_signs = true;
    // libqalculate's `DEFAULT_PRECISION` is 8, but `qalc` itself raises the
    // precision to 10 before evaluating anything (`src/qalc.cc` calls
    // `setPrecision(10)`), so 10 — not 8 — is the reference program's
    // user-facing default. Verified against qalc 5.5.2: `sqrt(2)` prints
    // `1.414213562` and `1/3` prints `0.3333333333`, both 10 significant
    // digits. The transcript runner already overrode this back to
    // `DEFAULT_PRECISION`, which is why batch parity passed while the
    // interactive and one-shot paths printed 8 digits.
    qalc_num::context::set_precision(qalc_num::context::DEFAULT_PRECISION);
    qalc_num::context::set_interval_calculation(
        qalc_num::context::IntervalCalculation::VarianceFormula,
    );
    session
}

/// Evaluate one CLI line, applying the options `src/qalc.cc` handles outside
/// `Calculator` — the `/set` commands the session does not know about, and
/// the adaptive interval display.
pub fn evaluate_cli_line(session: &mut Session, line: &str) -> Result<String, String> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix('/') {
        if let Some(out) = set_cli_option(session, rest)? {
            return Ok(out);
        }
    }
    // `adaptive_interval_display` (src/qalc.cc:7566): an expression that
    // states an uncertainty is shown as `value ± uncertainty` rather than
    // rounded to its significant digits.
    if ADAPTIVE_INTERVAL_DISPLAY.with(|a| a.get()) {
        session.print_options.interval_display = if trimmed.contains("+/-")
            || trimmed.contains('\u{00B1}')
            || trimmed.contains("uncertainty(")
        {
            IntervalDisplay::PlusMinus
        } else {
            IntervalDisplay::SignificantDigits
        };
    }
    let res = match qalc_core::parser::split_target_conversion_option(trimmed) {
        Some((expr, target)) => evaluate_target_conversion(session, expr, target)?,
        None => session.evaluate_line(trimmed)?,
    };
    if is_terse() {
        let trimmed = res.trim();
        if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
            Ok(trimmed[1..trimmed.len() - 1].to_string())
        } else {
            Ok(trimmed.to_string())
        }
    } else {
        Ok(res)
    }
}

/// Evaluate the CLI-only `to factors` and `to fraction` targets.
///
/// They are split before the ordinary conversion path because neither names
/// a unit: factorization restructures the evaluated value, while fraction
/// changes only the number printer.
fn evaluate_target_conversion(
    session: &Session,
    expr: &str,
    target: TargetConversionOption,
) -> Result<String, String> {
    let mut value = qalc_core::parser::parse_with(expr, &session.parse_options, session)
        .map_err(|error| error.to_string())?;
    qalc_core::percent::apply(&mut value);
    qalc_core::eval::evaluate_calculated_with(&mut value, &session.eval_options);

    let mut print_options = session.print_options.clone();
    match target {
        TargetConversionOption::Factors => {
            value = factor_result(&value, &session.eval_options);
            print_options.use_unicode_signs = true;
        }
        TargetConversionOption::Fraction => {
            value = mixed_fraction(value);
            print_options.number_fraction_format = NumberFractionFormat::Fractional;
            print_options.restrict_fraction_length = false;
        }
    }
    let rendered = qalc_core::print::print(&value, &print_options);
    Ok(match target {
        // Factored polynomial groups conventionally juxtapose, as in
        // `(x + 2)(x - 3)^3`, rather than carrying the generic product
        // printer's explicit sign between parenthesized factors.
        TargetConversionOption::Factors => rendered.replace(") * (", ")(").replace(" - ", " − "),
        TargetConversionOption::Fraction => rendered,
    })
}

fn mixed_fraction(value: MathStructure) -> MathStructure {
    let MathStructure::Number(number) = &value else {
        return value;
    };
    if !number.is_rational() || number.is_integer() || number.has_imaginary_part() {
        return value;
    }
    let mut whole = number.clone();
    whole.trunc();
    if whole.is_zero() {
        return value;
    }
    let mut remainder = number.clone();
    remainder.subtract(&whole);
    MathStructure::Addition(vec![
        MathStructure::Number(whole),
        MathStructure::Number(remainder),
    ])
}

fn factor_result(value: &MathStructure, options: &qalc_core::EvaluationOptions) -> MathStructure {
    match value {
        MathStructure::Number(number) => factor_number(number).unwrap_or_else(|| value.clone()),
        _ => qalc_core::polynomial::factor(value, options),
    }
}

/// Express an exact rational as prime factors, using negative powers for its
/// denominator. Symbolic values are handled by the polynomial factorizer.
fn factor_number(number: &Number) -> Option<MathStructure> {
    if !number.is_rational() || number.has_imaginary_part() {
        return None;
    }
    let mut numerator = Vec::new();
    if !number.numerator().factorize(&mut numerator) {
        return None;
    }
    let mut denominator = Vec::new();
    let denominator_number = number.denominator();
    if !denominator_number.is_one() && !denominator_number.factorize(&mut denominator) {
        return None;
    }

    let mut factors = grouped_prime_factors(&numerator, 1);
    factors.extend(grouped_prime_factors(&denominator, -1));
    match factors.len() {
        0 => None,
        1 => factors.pop(),
        _ => Some(MathStructure::Multiplication(factors)),
    }
}

fn grouped_prime_factors(factors: &[Number], exponent_sign: i64) -> Vec<MathStructure> {
    let mut grouped = Vec::new();
    let mut index = 0;
    while index < factors.len() {
        let mut end = index + 1;
        while end < factors.len() && factors[end].equals(&factors[index], false, false) {
            end += 1;
        }
        let exponent = exponent_sign * (end - index) as i64;
        let base = MathStructure::Number(factors[index].clone());
        grouped.push(if exponent == 1 {
            base
        } else {
            MathStructure::Power {
                base: Box::new(base),
                exponent: Box::new(MathStructure::from(exponent)),
            }
        });
        index = end;
    }
    grouped
}

/// The `/set` options `src/qalc.cc` owns rather than `Calculator`.
/// Returns `Ok(None)` when the option is not one of them, so the session can
/// try it. Recognized but malformed CLI options return an error instead of
/// being reinterpreted by the expression/session path.
fn set_cli_option(session: &mut Session, cmd: &str) -> Result<Option<String>, String> {
    let mut words = cmd.split_whitespace();
    if words.next() != Some("set") {
        return Ok(None);
    }
    let Some(raw_option) = words.next() else {
        return Err("missing /set option".to_string());
    };
    let (option, value) = match raw_option {
        "interval" => match words.next() {
            Some("display") => ("ivdisp", words.next().unwrap_or("1")),
            Some("calculation") => ("ic", words.next().unwrap_or("1")),
            Some(name) => return Err(format!("unknown /set interval option: {name}")),
            None => return Err("missing /set interval option".to_string()),
        },
        "uncertainty" => match words.next() {
            Some("propagation") => ("up", words.next().unwrap_or("1")),
            Some(name) => return Err(format!("unknown /set uncertainty option: {name}")),
            None => return Err("missing /set uncertainty option".to_string()),
        },
        _ => (raw_option, words.next().unwrap_or("1")),
    };
    match option {
        // `/set interval calculation | ic | uncertainty propagation | up`
        // (src/qalc.cc:1967).
        "ic" | "up" => {
            let v = match value {
                "none" => 0,
                "variance" | "variance formula" => 1,
                "iv" | "interval" | "interval arithmetic" => 2,
                "simple" | "simple interval arithmetic" => 3,
                _ => value
                    .parse::<i32>()
                    .map_err(|_| format!("invalid value for /set {raw_option}: {value}"))?,
            };
            let mode = qalc_num::context::IntervalCalculation::from_i32(v)
                .ok_or_else(|| format!("invalid value for /set {raw_option}: {value}"))?;
            qalc_num::context::set_interval_calculation(mode);
            Ok(Some(String::new()))
        }
        // `/set approximation`: "try exact" means "an exact pass, then an
        // approximate one" in the C++ (`MathStructure::eval`,
        // MathStructure-eval.cc:2937). This port's evaluator has a single
        // pass, so the approximate one is the one to run — otherwise every
        // irrational result stays unevaluated. `exact` still reaches the
        // session unchanged.
        "approximation" | "appr" | "approx" => match value {
            "exact" | "0" => Ok(None),
            "approximate" | "2" | "try" | "1" => {
                session.eval_options.approximation = qalc_core::ApproximationMode::Approximate;
                Ok(Some(String::new()))
            }
            _ => Err(format!("invalid value for /set {raw_option}: {value}")),
        },
        // `/set interval display | ivdisp` (src/qalc.cc).
        "ivdisp" => {
            if matches!(value, "0" | "adaptive") {
                ADAPTIVE_INTERVAL_DISPLAY.with(|a| a.set(true));
                return Ok(Some(String::new()));
            }
            session.print_options.interval_display = match value {
                "1" | "significant" => IntervalDisplay::SignificantDigits,
                "2" | "interval" => IntervalDisplay::Interval,
                "3" | "plusminus" | "+/-" => IntervalDisplay::PlusMinus,
                "4" | "midpoint" => IntervalDisplay::Midpoint,
                "5" | "lower" => IntervalDisplay::Lower,
                "6" | "upper" => IntervalDisplay::Upper,
                "7" | "concise" => IntervalDisplay::Concise,
                "8" | "relative" => IntervalDisplay::Relative,
                _ => return Err(format!("invalid value for /set {raw_option}: {value}")),
            };
            ADAPTIVE_INTERVAL_DISPLAY.with(|a| a.set(false));
            Ok(Some(String::new()))
        }
        // `/set terse | t`
        "terse" | "t" => {
            let enabled = match value {
                "1" | "on" | "true" | "yes" => true,
                "0" | "off" | "false" | "no" => false,
                _ => return Err(format!("invalid value for /set {raw_option}: {value}")),
            };
            set_terse(enabled);
            Ok(Some(String::new()))
        }
        _ => Ok(None),
    }
}

/// Run a transcript file the way `--test-file` does, returning its report.
///
/// One session per file: transcripts carry state across lines (`alpha := 5`
/// then `alpha`), which is why variables.batch works at all.
pub fn run_transcript_file(path: &std::path::Path) -> std::io::Result<crate::batch::Report> {
    let source = std::fs::read_to_string(path)?;
    // `load(tests/data.csv)` in a transcript is relative to the reference
    // project root, i.e. the parent of the directory holding the transcript.
    if let Some(root) = path
        .parent()
        .and_then(|p| p.parent())
        .filter(|p| !p.as_os_str().is_empty())
    {
        qalc_core::stats::set_data_dir(root.to_path_buf());
    }
    let transcript = crate::batch::parse_transcript(&source);
    let mut session = new_session();
    session.print_options = qalc_core::eval::batch_print_options();
    qalc_core::assumptions::set_sign(qalc_core::assumptions::Sign::Unknown);
    Ok(crate::batch::run_transcript(
        &path.display().to_string(),
        &transcript,
        |expression| evaluate_cli_line(&mut session, expression),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qalc_core::ApproximationMode;
    use qalc_num::context::{interval_calculation, IntervalCalculation};
    use qalc_num::options::IntervalDisplay;

    #[test]
    fn test_set_ic_and_up() {
        let mut session = new_session();

        // /set ic variance
        let res = evaluate_cli_line(&mut session, "/set ic variance");
        assert!(res.is_ok());
        assert_eq!(interval_calculation(), IntervalCalculation::VarianceFormula);

        // /set ic interval
        let res = evaluate_cli_line(&mut session, "/set ic interval");
        assert!(res.is_ok());
        assert_eq!(interval_calculation(), IntervalCalculation::IntervalArithmetic);

        // /set up 1 (alias for ic)
        let res = evaluate_cli_line(&mut session, "/set up 1");
        assert!(res.is_ok());
        assert_eq!(interval_calculation(), IntervalCalculation::VarianceFormula);

        // /set interval calculation 2 (multi-word alias)
        let res = evaluate_cli_line(&mut session, "/set interval calculation 2");
        assert!(res.is_ok());
        assert_eq!(interval_calculation(), IntervalCalculation::IntervalArithmetic);

        // /set uncertainty propagation variance
        let res = evaluate_cli_line(&mut session, "/set uncertainty propagation variance");
        assert!(res.is_ok());
        assert_eq!(interval_calculation(), IntervalCalculation::VarianceFormula);
    }

    #[test]
    fn test_set_appr() {
        let mut session = new_session();
        assert_eq!(session.eval_options.approximation, ApproximationMode::Approximate);

        // /set appr exact
        let res = evaluate_cli_line(&mut session, "/set appr exact");
        assert!(res.is_ok());
        assert_eq!(session.eval_options.approximation, ApproximationMode::Exact);

        // /set appr approximate
        let res = evaluate_cli_line(&mut session, "/set appr approximate");
        assert!(res.is_ok());
        assert_eq!(session.eval_options.approximation, ApproximationMode::Approximate);

        // /set approximation exact
        let res = evaluate_cli_line(&mut session, "/set approximation exact");
        assert!(res.is_ok());
        assert_eq!(session.eval_options.approximation, ApproximationMode::Exact);

        // /set approx 2
        let res = evaluate_cli_line(&mut session, "/set approx 2");
        assert!(res.is_ok());
        assert_eq!(session.eval_options.approximation, ApproximationMode::Approximate);
    }

    #[test]
    fn test_set_ivdisp() {
        let mut session = new_session();

        let cases = [
            ("1", IntervalDisplay::SignificantDigits),
            ("significant", IntervalDisplay::SignificantDigits),
            ("2", IntervalDisplay::Interval),
            ("interval", IntervalDisplay::Interval),
            ("3", IntervalDisplay::PlusMinus),
            ("plusminus", IntervalDisplay::PlusMinus),
            ("+/-", IntervalDisplay::PlusMinus),
            ("4", IntervalDisplay::Midpoint),
            ("midpoint", IntervalDisplay::Midpoint),
            ("5", IntervalDisplay::Lower),
            ("lower", IntervalDisplay::Lower),
            ("6", IntervalDisplay::Upper),
            ("upper", IntervalDisplay::Upper),
            ("7", IntervalDisplay::Concise),
            ("concise", IntervalDisplay::Concise),
            ("8", IntervalDisplay::Relative),
            ("relative", IntervalDisplay::Relative),
        ];

        for (val, expected) in cases {
            let cmd = format!("/set ivdisp {val}");
            let res = evaluate_cli_line(&mut session, &cmd);
            assert!(res.is_ok(), "failed for command: {cmd}");
            assert_eq!(session.print_options.interval_display, expected, "failed for: {cmd}");
        }

        // Test multi-word option `/set interval display 4`
        let res = evaluate_cli_line(&mut session, "/set interval display 4");
        assert!(res.is_ok());
        assert_eq!(session.print_options.interval_display, IntervalDisplay::Midpoint);
    }

    #[test]
    fn test_adaptive_interval_display_behavior() {
        let mut session = new_session();

        // Enable adaptive display explicitly
        evaluate_cli_line(&mut session, "/set ivdisp 0").unwrap();

        // Expression without +/- should default to SignificantDigits
        let res = evaluate_cli_line(&mut session, "2 + 2");
        assert!(res.is_ok());
        assert_eq!(session.print_options.interval_display, IntervalDisplay::SignificantDigits);

        // Expression with +/- should trigger PlusMinus adaptive display
        let _ = evaluate_cli_line(&mut session, "5 +/- 1");
        assert_eq!(session.print_options.interval_display, IntervalDisplay::PlusMinus);

        // Expression with unicode ±
        let _ = evaluate_cli_line(&mut session, "5 \u{00B1} 1");
        assert_eq!(session.print_options.interval_display, IntervalDisplay::PlusMinus);

        // Expression with uncertainty(...)
        let _ = evaluate_cli_line(&mut session, "uncertainty(5)");
        assert_eq!(session.print_options.interval_display, IntervalDisplay::PlusMinus);

        // Disabling adaptive mode via explicit ivdisp selection (e.g., Interval = 2)
        evaluate_cli_line(&mut session, "/set ivdisp 2").unwrap();
        assert_eq!(session.print_options.interval_display, IntervalDisplay::Interval);

        // Subsequent +/- expression should NOT override explicit setting
        let _ = evaluate_cli_line(&mut session, "5 +/- 1");
        assert_eq!(session.print_options.interval_display, IntervalDisplay::Interval);

        // Restoring adaptive display via /set ivdisp 0 or /set ivdisp adaptive
        evaluate_cli_line(&mut session, "/set ivdisp adaptive").unwrap();
        let _ = evaluate_cli_line(&mut session, "2 + 2");
        assert_eq!(session.print_options.interval_display, IntervalDisplay::SignificantDigits);
        let _ = evaluate_cli_line(&mut session, "5 +/- 1");
        assert_eq!(session.print_options.interval_display, IntervalDisplay::PlusMinus);
    }

    #[test]
    fn test_terse_mode() {
        let mut session = new_session();

        set_terse(false);
        assert!(!is_terse());

        set_terse(true);
        assert!(is_terse());

        let res = evaluate_cli_line(&mut session, " 10 + 20 \n");
        assert_eq!(res.unwrap(), "30");

        // Test /set terse command
        evaluate_cli_line(&mut session, "/set terse 0").unwrap();
        assert!(!is_terse());

        evaluate_cli_line(&mut session, "/set terse on").unwrap();
        assert!(is_terse());

        evaluate_cli_line(&mut session, "/set t off").unwrap();
        assert!(!is_terse());

        set_terse(false);
    }

    #[test]
    fn malformed_cli_set_options_return_errors() {
        let mut session = new_session();

        for command in [
            "/set",
            "/set ic invalid",
            "/set up 9",
            "/set approx invalid",
            "/set ivdisp invalid",
            "/set terse invalid",
            "/set interval display invalid",
            "/set interval invalid",
            "/set uncertainty propagation invalid",
            "/set uncertainty invalid",
        ] {
            assert!(
                evaluate_cli_line(&mut session, command).is_err(),
                "{command} should be rejected"
            );
        }
    }
}
