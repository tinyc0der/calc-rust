//! `qalc` CLI — pure-Rust port of libqalculate's `src/qalc.cc`.
//!
//! Argument handling and the output side. The evaluation path itself lives in
//! [`qalc::cli`] so the transcript parity test can drive the same code.
//!
//! `/set` and `to` are implemented; the interactive REPL, `save`, and RPN mode
//! land with the Calculator port.

use qalc::batch;
use qalc::cli::{evaluate_cli_line, new_session};

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

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
