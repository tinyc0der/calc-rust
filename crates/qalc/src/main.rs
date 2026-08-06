//! `qalc` CLI — pure-Rust port of libqalculate's `src/qalc.cc`.
//!
//! Argument handling and the output side. The evaluation path itself lives in
//! [`qalc::cli`] so the transcript parity test can drive the same code.
//!
//! `/set` (see [`qalc_core::Session`]) and `to` conversions (see
//! [`qalc_core::eval::apply_conversion`]) are implemented. [`repl`] is a bare
//! read-eval-print loop over stdin — no line editing, history or completion —
//! and `save` and RPN mode are not ported.

use qalc::batch;
use qalc::cli::{evaluate_cli_line, new_session};

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut test_file: Option<String> = None;
    let mut expression: Vec<String> = Vec::new();
    let mut terse = false;
    let mut args = std::env::args().skip(1);
    let mut options_enabled = true;

    while let Some(arg) = args.next() {
        if options_enabled && arg == "--" {
            options_enabled = false;
            continue;
        }
        if options_enabled && arg.starts_with("--test-file=") {
            let path = &arg["--test-file=".len()..];
            if path.is_empty() {
                return usage_error("--test-file requires a file path");
            }
            test_file = Some(path.to_string());
        } else if options_enabled && arg == "--test-file" {
            let Some(path) = args.next() else {
                return usage_error("--test-file requires a file path");
            };
            test_file = Some(path);
        } else if options_enabled && (arg == "-t" || arg == "--terse") {
            terse = true;
        } else if options_enabled && (arg == "-h" || arg == "--help") {
            write_usage(&mut io::stdout());
            return ExitCode::SUCCESS;
        } else if options_enabled && (arg == "-v" || arg == "--version") {
            println!("qalc (rust-calc) {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        } else if options_enabled && arg.starts_with('-') && arg != "-" {
            // "-5" and "-3.14" are expressions, not options. The reference
            // qalc accepts them as one-shot expressions (e.g. `qalc -t -- -5`
            // or `qalc -t -5` when not an option). Treat a leading '-' followed
            // by a digit or '.' as an expression rather than an unknown option.
            if arg.len() > 1 && arg.chars().nth(1).map_or(false, |c| c.is_ascii_digit() || c == '.') {
                expression.push(arg);
            } else {
                return usage_error(&format!("unknown option: {arg}"));
            }
        } else {
            expression.push(arg);
        }
    }
    if terse {
        qalc::cli::set_terse(true);
    }

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

fn write_usage(out: &mut impl Write) {
    let _ = writeln!(out, "Usage: qalc [options] [expression]");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  --test-file FILE   run a .batch transcript and report results"
    );
    let _ = writeln!(out, "  --test-file=FILE   same as above");
    let _ = writeln!(out, "  --                  stop processing options");
    let _ = writeln!(out, "  -t, --terse        print results only");
    let _ = writeln!(out, "  -v, --version      print version");
    let _ = writeln!(out, "  -h, --help         show this help");
}

fn usage_error(message: &str) -> ExitCode {
    let mut err = io::stderr();
    let _ = writeln!(err, "error: {message}");
    write_usage(&mut err);
    ExitCode::FAILURE
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
    let report = match qalc::cli::run_transcript_file(std::path::Path::new(path)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

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
