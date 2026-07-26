//! `qalc` CLI — pure-Rust port of libqalculate's `src/qalc.cc`.
//!
//! Currently supports expression evaluation from arguments, stdin, and
//! `--test-file=` batch transcripts. The interactive REPL, command set
//! (`/set`, `to`, `save`), and RPN mode land with the Calculator port.

mod batch;

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use qalc_core::Session;
use qalc_num::options::IntervalDisplay;

thread_local! {
    /// `adaptive_interval_display` (src/qalc.cc:116): on until `/set ivdisp`
    /// picks a display explicitly.
    static ADAPTIVE_INTERVAL_DISPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// A fresh session with the calculator's own predefined variables installed.
///
/// The C++ registers the imaginary unit as `VARIABLE_ID_I`, a builtin
/// `KnownVariable` holding `Number(0, 1, 0, 1)` (`Calculator.cc`). The port's
/// definition registry does not carry it, so the CLI installs it here — the
/// session's variable table is consulted before every other name source, so
/// `2i - 3` parses to the complex number it does in the reference.
fn new_session() -> Session {
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
fn evaluate_cli_line(session: &mut Session, line: &str) -> Result<String, String> {
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut test_file: Option<String> = None;
    let mut expression: Vec<String> = Vec::new();
    let mut terse = false;

    for arg in &args {
        if let Some(path) = arg.strip_prefix("--test-file=") {
            test_file = Some(path.to_string());
        } else if arg == "-t" || arg == "--terse" {
            terse = true;
        } else if arg == "-h" || arg == "--help" {
            print_usage();
            return ExitCode::SUCCESS;
        } else if arg == "-v" || arg == "--version" {
            println!("qalc (rust-calc) {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        } else {
            expression.push(arg.clone());
        }
    }
    let _ = terse;

    if let Some(path) = test_file {
        return run_test_file(&path);
    }

    if !expression.is_empty() {
        let expr = expression.join(" ");
        let mut session = new_session();
        match evaluate_cli_line(&mut session, &expr) {
            Ok(s) => {
                println!("{s}");
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    repl()
}

fn print_usage() {
    println!("Usage: qalc [options] [expression]");
    println!();
    println!("  --test-file=FILE   run a .batch transcript and report results");
    println!("  -t, --terse        print results only");
    println!("  -v, --version      print version");
    println!("  -h, --help         show this help");
}

/// Read-eval-print loop over stdin.
fn repl() -> ExitCode {
    let mut session = new_session();
    let stdin = io::stdin();
    let mut out = io::stdout();
    let interactive = std::io::IsTerminal::is_terminal(&stdin);
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "quit" || line == "exit" {
            break;
        }
        match evaluate_cli_line(&mut session, line) {
            Ok(s) => {
                let _ = writeln!(out, "{s}");
            }
            Err(e) => {
                let _ = writeln!(out, "error: {e}");
            }
        }
        let _ = out.flush();
        if !interactive {
            continue;
        }
    }
    ExitCode::SUCCESS
}

/// Run a `.batch` transcript, printing a per-file summary and every failure.
fn run_test_file(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // `load(tests/data.csv)` in a transcript is relative to the reference
    // project root, i.e. the parent of the directory holding the transcript.
    if let Some(root) = std::path::Path::new(path)
        .parent()
        .and_then(|p| p.parent())
        .filter(|p| !p.as_os_str().is_empty())
    {
        qalc_core::stats::set_data_dir(root.to_path_buf());
    }
    let transcript = batch::parse_transcript(&src);
    if !transcript.cases.iter().any(|c| c.expected.is_some()) {
        println!("WARNING: 0 tests were run (indentation needs to be tab-based)");
        return ExitCode::FAILURE;
    }
    // One session per file: transcripts carry state across lines (`alpha := 5`
    // then `alpha`), which is why variables.batch works at all.
    let mut session = new_session();
    let report = batch::run_transcript(path, &transcript, |expr| evaluate_cli_line(&mut session, expr));

    for (case, outcome) in report.failures() {
        match outcome {
            batch::Outcome::Mismatch { got } => {
                println!("Mismatch at line {}:", case.line);
                println!("  expression: {}", case.expression);
                println!("  expected:   {}", case.expected.as_deref().unwrap_or(""));
                println!("  received:   {got}");
            }
            batch::Outcome::Error { message } => {
                println!("Error at line {}:", case.line);
                println!("  expression: {}", case.expression);
                println!("  expected:   {}", case.expected.as_deref().unwrap_or(""));
                println!("  error:      {message}");
            }
            batch::Outcome::Pass => {}
        }
    }
    println!("{report}");
    if report.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
