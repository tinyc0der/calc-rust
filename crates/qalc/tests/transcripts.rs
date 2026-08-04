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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let allow_missing = std::env::var("QALC_ALLOW_MISSING_ORACLE").as_deref() == Ok("1");
    if let Ok(dir) = std::env::var("QALCULATE_TESTS_DIR") {
        let path = resolve_candidate(manifest_dir, Path::new(&dir));
        if is_transcripts_dir(&path) {
            return Some(path);
        }
        if allow_missing {
            return None;
        }
        panic!(
            "QALCULATE_TESTS_DIR does not contain operators.batch: {}",
            path.display()
        );
    }
    for candidate in [
        "/root/Project/libqalculate/tests",
        "../libqalculate/tests",
        "../../libqalculate/tests",
        "../../../libqalculate/tests",
        "../../../../libqalculate/tests",
        "../../../../../libqalculate/tests",
        "../Demo/libqalculate/tests",
        "../../Demo/libqalculate/tests",
        "../../../Demo/libqalculate/tests",
        "../../../../Demo/libqalculate/tests",
        "../../../../../Demo/libqalculate/tests",
    ] {
        let path = resolve_candidate(manifest_dir, Path::new(candidate));
        if is_transcripts_dir(&path) {
            return Some(path);
        }
    }
    if allow_missing {
        return None;
    }
    panic!(
        "reference transcripts not found. Set QALCULATE_TESTS_DIR to \
         libqalculate's tests/ directory, or QALC_ALLOW_MISSING_ORACLE=1 to skip."
    );
}

fn resolve_candidate(manifest_dir: &Path, candidate: &Path) -> PathBuf {
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        manifest_dir.join(candidate)
    }
}

fn is_transcripts_dir(path: &Path) -> bool {
    path.is_dir() && path.join("operators.batch").is_file()
}

#[test]
fn relative_candidates_are_resolved_from_the_manifest_directory() {
    let manifest_dir = Path::new("/workspace/calc-rust/crates/qalc");

    assert_eq!(
        resolve_candidate(manifest_dir, Path::new("../../../libqalculate/tests")),
        manifest_dir.join("../../../libqalculate/tests")
    );
    assert_eq!(
        resolve_candidate(manifest_dir, Path::new("/oracle/libqalculate/tests")),
        PathBuf::from("/oracle/libqalculate/tests")
    );
}

/// Run one transcript through the CLI's own evaluation path, returning the
/// 1-based line of every case that differs.
fn failing_lines(path: &Path) -> Result<Vec<(usize, String)>, String> {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_qalc"));
    cmd.arg("-t").arg(path);
    if let Some(dir) = std::env::var_os("QALCULATE_DEFINITIONS_DIR") {
        cmd.env("QALCULATE_DEFINITIONS_DIR", dir);
    }
    let output = cmd.output().map_err(|error| format!("cannot run qalc: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && !stdout.contains("Mismatch at line") && !stdout.contains("Error at line") {
        return Ok(Vec::new());
    }
    let mut failures = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("Mismatch at line ") || line.starts_with("Error at line ") {
            if let Some(num_str) = line.split("line ").nth(1) {
                if let Some(line_num) = num_str.trim_matches(':').parse::<usize>().ok() {
                    failures.push((line_num, line.to_string()));
                }
            }
        }
    }
    if failures.is_empty() && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("qalc failed on {}: {stdout}\n{stderr}", path.display()));
    }
    Ok(failures)
}

#[test]
fn every_reference_transcript_matches() {
    let Some(dir) = transcripts_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!("transcripts directory is not readable: {}", dir.display());
    };
    let mut batches: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "batch"))
        .collect();
    batches.sort();
    assert!(!batches.is_empty(), "no .batch files in {}", dir.display());

    let mut unexpected: Vec<String> = Vec::new();
    let mut fixed: Vec<String> = Vec::new();

    for path in &batches {
        let Some(file_name) = path.file_name() else {
            unexpected.push(format!("invalid transcript path: {}", path.display()));
            continue;
        };
        let name = file_name.to_string_lossy().into_owned();
        let failures = match failing_lines(path) {
            Ok(failures) => failures,
            Err(error) => {
                unexpected.push(error);
                continue;
            }
        };
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
