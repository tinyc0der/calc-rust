//! Batch transcript runner — the Rust equivalent of `qalc --test-file=`
//! (src/qalc.cc:4715) and the `unittest` driver.
//!
//! Transcript format: a line starting at column 0 is an expression; a line
//! starting with a TAB is the expected result for the preceding expression.
//! Blank lines and lines starting with `#` or `//` are ignored. **Expected
//! lines must be tab-indented** — the C++ harness silently skips
//! space-indented ones, so we report them as an error instead.

use std::fmt;

/// One expression and, when the transcript states one, its expected output.
///
/// Lines *without* an expectation still run: they carry state the later
/// assertions depend on (`alpha := 5`, `v = load(data.csv)`, `/set unicode 1`,
/// `delete v`). The C++ harness executes every line and only compares the
/// ones followed by a tab-indented expectation, so this does too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub expression: String,
    pub expected: Option<String>,
    /// 1-based line number of the expression in the source file.
    pub line: usize,
}

/// A transcript parsed from a `.batch` file.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub cases: Vec<Case>,
    /// Lines that look like expected results but use spaces instead of a tab.
    pub space_indented: Vec<usize>,
}

/// Parse a `.batch` transcript.
pub fn parse_transcript(src: &str) -> Transcript {
    let mut t = Transcript::default();
    let mut pending: Option<(String, usize)> = None;
    for (idx, raw) in src.lines().enumerate() {
        let line_no = idx + 1;
        if raw.starts_with('\t') {
            let expected = raw.trim_matches(|c: char| c.is_whitespace()).to_string();
            if let Some((expression, line)) = pending.take() {
                t.cases.push(Case {
                    expression,
                    expected: Some(expected),
                    line,
                });
            }
            continue;
        }
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            if let Some((expression, line)) = pending.take() {
                t.cases.push(Case {
                    expression,
                    expected: None,
                    line,
                });
            }
            continue;
        }
        // A space-indented line following an expression was meant to be an
        // expectation; the C++ harness drops it silently.
        if raw.starts_with(' ') && pending.is_some() {
            t.space_indented.push(line_no);
        }
        // The previous expression had no expectation: keep it as a setup
        // line rather than dropping it.
        if let Some((expression, line)) = pending.take() {
            t.cases.push(Case {
                expression,
                expected: None,
                line,
            });
        }
        pending = Some((trimmed.to_string(), line_no));
    }
    if let Some((expression, line)) = pending.take() {
        t.cases.push(Case {
            expression,
            expected: None,
            line,
        });
    }
    t
}

/// Outcome of one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    /// Produced output differing from the expectation.
    Mismatch { got: String },
    /// Evaluation returned an error.
    Error { message: String },
}

/// Result of running a whole transcript.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub name: String,
    pub results: Vec<(Case, Outcome)>,
}

impl Report {
    pub fn passed(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, o)| *o == Outcome::Pass)
            .count()
    }
    pub fn total(&self) -> usize {
        self.results.len()
    }
    pub fn failures(&self) -> impl Iterator<Item = &(Case, Outcome)> {
        self.results.iter().filter(|(_, o)| *o != Outcome::Pass)
    }
    pub fn all_passed(&self) -> bool {
        self.passed() == self.total()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} - {}/{} passed", self.name, self.passed(), self.total())
    }
}

/// Run every case through `eval`, collecting outcomes.
///
/// Unlike the C++ harness, which exits on the first mismatch, this runs the
/// whole file so progress can be measured while the port is incomplete.
pub fn run_transcript(
    name: &str,
    t: &Transcript,
    mut eval: impl FnMut(&str) -> Result<String, String>,
) -> Report {
    let mut report = Report {
        name: name.to_string(),
        results: Vec::new(),
    };
    for case in &t.cases {
        let result = eval(&case.expression);
        // A line without an expectation is setup (an assignment, a `/set`
        // command, a `delete`): run it for its side effects, assert nothing,
        // and let a failure surface on the assertions that depend on it.
        let Some(expected) = case.expected.as_deref() else {
            continue;
        };
        let outcome = match result {
            Ok(got) if got == expected => Outcome::Pass,
            Ok(got) => Outcome::Mismatch { got },
            Err(message) => Outcome::Error { message },
        };
        report.results.push((case.clone(), outcome));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_expression_and_expectation() {
        let src = "1+1\n\t2\n\n2*3\n\t6\n";
        let t = parse_transcript(src);
        assert_eq!(t.cases.len(), 2);
        assert_eq!(t.cases[0].expression, "1+1");
        assert_eq!(t.cases[0].expected.as_deref(), Some("2"));
        assert_eq!(t.cases[0].line, 1);
        assert_eq!(t.cases[1].expression, "2*3");
        assert_eq!(t.cases[1].expected.as_deref(), Some("6"));
    }

    #[test]
    fn skips_comments_and_blanks() {
        let src = "# a comment\n// another\n\n1+1\n\t2\n";
        let t = parse_transcript(src);
        assert_eq!(t.cases.len(), 1);
    }

    #[test]
    fn expressions_without_expectations_are_kept_but_unasserted() {
        // Commands like `/set ...` and assignments carry state but assert
        // nothing; they must still run, in order.
        let src = "alpha := 5\nalpha + 1\n\t6\n";
        let t = parse_transcript(src);
        assert_eq!(t.cases.len(), 2);
        assert_eq!(t.cases[0].expression, "alpha := 5");
        assert_eq!(t.cases[0].expected, None);
        assert_eq!(t.cases[1].expression, "alpha + 1");
        assert_eq!(t.cases[1].expected.as_deref(), Some("6"));

        let mut seen = Vec::new();
        let r = run_transcript("t", &t, |e| {
            seen.push(e.to_string());
            Ok("6".to_string())
        });
        assert_eq!(seen, vec!["alpha := 5", "alpha + 1"]);
        // Only the line with an expectation is scored.
        assert_eq!(r.total(), 1);
    }

    #[test]
    fn flags_space_indented_expectations() {
        let src = "1+1\n  2\n";
        let t = parse_transcript(src);
        assert_eq!(t.space_indented, vec![2]);
        assert_eq!(
            t.cases.iter().filter(|c| c.expected.is_some()).count(),
            0,
            "space-indented lines are not expectations"
        );
    }

    #[test]
    fn expectation_whitespace_is_trimmed() {
        let src = "1+1\n\t  2  \n";
        let t = parse_transcript(src);
        assert_eq!(t.cases[0].expected.as_deref(), Some("2"));
    }

    #[test]
    fn reports_count_outcomes() {
        let src = "1+1\n\t2\n2+2\n\t4\n";
        let t = parse_transcript(src);
        let r = run_transcript("t", &t, |e| {
            if e == "1+1" {
                Ok("2".to_string())
            } else {
                Ok("5".to_string())
            }
        });
        assert_eq!(r.passed(), 1);
        assert_eq!(r.total(), 2);
        assert!(!r.all_passed());
        assert_eq!(r.failures().count(), 1);
    }

    #[test]
    fn errors_are_recorded_not_fatal() {
        let src = "boom\n\t1\nok\n\t2\n";
        let t = parse_transcript(src);
        let r = run_transcript("t", &t, |e| {
            if e == "boom" {
                Err("nope".to_string())
            } else {
                Ok("2".to_string())
            }
        });
        assert_eq!(r.passed(), 1);
        assert!(matches!(r.results[0].1, Outcome::Error { .. }));
    }

    #[test]
    fn parses_a_real_transcript() {
        let path = "/root/Project/libqalculate/tests/operators.batch";
        let Ok(src) = std::fs::read_to_string(path) else {
            return; // reference checkout not present
        };
        let t = parse_transcript(&src);
        assert!(t.cases.len() > 5, "operators.batch has cases");
        assert!(
            t.space_indented.is_empty(),
            "reference transcripts are tab-indented"
        );
    }
}
