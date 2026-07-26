//! Transcript parity against the reference binary, as a test.
//!
//! This is the project's primary verification criterion: every transcript in
//! libqalculate's `tests/` must produce byte-identical output. It was measured
//! only by `scripts/parity.sh`, which is not run by `cargo test` and not run by
//! CI — so the suite could pass green while parity regressed. It cannot now.
//!
//! [`KNOWN_FAILURES`] pins the cases that do not yet pass. The count can only
//! go down: a new failure fails the test, and fixing a listed one fails it too,
//! so the list cannot rot.

use std::path::{Path, PathBuf};

use qalc::batch::Outcome;

/// Cases that do not yet match the reference, as `(file, line)`.
///
/// Empty: all 656 transcript assertions pass. Keep it that way — a new failure
/// fails this test, and so does listing one that has started passing, so the
/// count cannot creep back up unnoticed.
const KNOWN_FAILURES: &[(&str, usize)] = &[];

/// The reference checkout's `tests/` directory.
///
/// Deliberately fails rather than skipping: 39 tests in this workspace already
/// turn green by silently returning when the reference is absent, which is how
/// a suite comes to test nothing on a machine that is not the author's. Set
/// `QALC_ALLOW_MISSING_ORACLE=1` to opt into skipping.
fn transcripts_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QALCULATE_TESTS_DIR") {
        return Some(PathBuf::from(dir));
    }
    for candidate in [
        "/root/Project/libqalculate/tests",
        "../libqalculate/tests",
        "../../libqalculate/tests",
        "../../../libqalculate/tests",
    ] {
        let path = PathBuf::from(candidate);
        if path.join("operators.batch").is_file() {
            return Some(path);
        }
    }
    if std::env::var("QALC_ALLOW_MISSING_ORACLE").is_ok() {
        return None;
    }
    panic!(
        "reference transcripts not found. Set QALCULATE_TESTS_DIR to \
         libqalculate's tests/ directory, or QALC_ALLOW_MISSING_ORACLE=1 to skip."
    );
}

/// Run one transcript through the CLI's own evaluation path, returning the
/// 1-based line of every case that differs.
fn failing_lines(path: &Path) -> Vec<(usize, String)> {
    let report = qalc::cli::run_transcript_file(path).expect("transcript is readable");
    report
        .results
        .iter()
        .filter(|(_, outcome)| *outcome != Outcome::Pass)
        .map(|(case, outcome)| {
            let detail = match outcome {
                Outcome::Mismatch { got } => format!(
                    "{}\n    expected: {}\n    got:      {}",
                    case.expression,
                    case.expected.as_deref().unwrap_or(""),
                    got
                ),
                Outcome::Error { message } => {
                    format!("{}\n    error: {message}", case.expression)
                }
                Outcome::Pass => unreachable!(),
            };
            (case.line, detail)
        })
        .collect()
}

#[test]
fn every_reference_transcript_matches() {
    let Some(dir) = transcripts_dir() else {
        return;
    };
    let mut batches: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("transcripts directory is readable")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "batch"))
        .collect();
    batches.sort();
    assert!(!batches.is_empty(), "no .batch files in {}", dir.display());

    let mut unexpected: Vec<String> = Vec::new();
    let mut fixed: Vec<String> = Vec::new();

    for path in &batches {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let failures = failing_lines(path);
        let failed_lines: Vec<usize> = failures.iter().map(|(line, _)| *line).collect();

        for (line, detail) in &failures {
            if !KNOWN_FAILURES.contains(&(name.as_str(), *line)) {
                unexpected.push(format!("{name}:{line}  {detail}"));
            }
        }
        for (known_file, known_line) in KNOWN_FAILURES {
            if *known_file == name && !failed_lines.contains(known_line) {
                fixed.push(format!("{known_file}:{known_line}"));
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "transcript parity regressed:\n{}",
        unexpected.join("\n")
    );
    assert!(
        fixed.is_empty(),
        "these cases now pass — remove them from KNOWN_FAILURES so the count \
         cannot creep back up:\n{}",
        fixed.join("\n")
    );
}
