//! The evaluation path the `qalc` binary uses.
//!
//! This lives in the library, not the binary, because the transcript parity
//! test has to drive *exactly* what `--test-file` drives. When it drove
//! `Session::evaluate_line` directly instead, six cases differed — the CLI
//! owns `/set` options that `Calculator` does not, and the adaptive interval
//! display, and both change printed output.

use qalc_core::Session;
use qalc_num::options::IntervalDisplay;


thread_local! {
    /// `adaptive_interval_display` (declared src/qalc.cc:82): on until
    /// `/set ivdisp` picks a display explicitly — the CLI clears it at
    /// src/qalc.cc:2211 and restores it at :2203 for the `0`/adaptive value.
    static ADAPTIVE_INTERVAL_DISPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
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
        if let Some(out) = set_cli_option(session, rest) {
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
    session.evaluate_line(trimmed)
}

/// The `/set` options `src/qalc.cc` owns rather than `Calculator`.
/// Returns `None` when the option is not one of them, so the session can try.
fn set_cli_option(session: &mut Session, cmd: &str) -> Option<String> {
    let mut words = cmd.split_whitespace();
    if words.next()? != "set" {
        return None;
    }
    let option = words.next()?;
    let value = words.next().unwrap_or("1");
    match option {
        // `/set interval calculation | ic | uncertainty propagation | up`
        // (src/qalc.cc:1967).
        "ic" | "up" => {
            let v = match value {
                "variance" | "variance formula" => 1,
                "iv" | "interval" | "interval arithmetic" => 2,
                _ => value.parse::<i32>().ok()?,
            };
            let mode = qalc_num::context::IntervalCalculation::from_i32(v)?;
            qalc_num::context::set_interval_calculation(mode);
            Some(String::new())
        }
        // `/set approximation`: "try exact" means "an exact pass, then an
        // approximate one" in the C++ (`MathStructure::eval`,
        // MathStructure-eval.cc:2937). This port's evaluator has a single
        // pass, so the approximate one is the one to run — otherwise every
        // irrational result stays unevaluated. `exact` still reaches the
        // session unchanged.
        "approximation" | "appr" | "approx"
            if !matches!(value, "exact" | "0") =>
        {
            session.eval_options.approximation = qalc_core::ApproximationMode::Approximate;
            Some(String::new())
        }
        // `/set interval display | ivdisp` (src/qalc.cc).
        "ivdisp" => {
            session.print_options.interval_display = match value {
                "1" | "significant" => IntervalDisplay::SignificantDigits,
                "2" | "interval" => IntervalDisplay::Interval,
                "3" | "plusminus" | "+/-" => IntervalDisplay::PlusMinus,
                "4" | "midpoint" => IntervalDisplay::Midpoint,
                "5" | "lower" => IntervalDisplay::Lower,
                "6" | "upper" => IntervalDisplay::Upper,
                "7" | "concise" => IntervalDisplay::Concise,
                "8" | "relative" => IntervalDisplay::Relative,
                _ => return None,
            };
            ADAPTIVE_INTERVAL_DISPLAY.with(|a| a.set(false));
            Some(String::new())
        }
        _ => None,
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
    Ok(crate::batch::run_transcript(
        &path.display().to_string(),
        &transcript,
        |expression| evaluate_cli_line(&mut session, expression),
    ))
}
