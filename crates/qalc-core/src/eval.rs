//! End-to-end evaluation facade: parse → calculate → print.
//!
//! Mirrors `Calculator::calculateAndPrint` (Calculator-calculate.cc).

use crate::parser::{self, ParseError};
use crate::print;
use crate::structure::MathStructure;
use qalc_num::{ParseOptions, PrintOptions};

/// Print options matching `qalc +u8` — the mode `--test-file` runs in.
pub fn batch_print_options() -> PrintOptions {
    let mut po = PrintOptions::default();
    po.use_unicode_signs = false;
    po.spacious = true;
    po.short_multiplication = true;
    po.show_ending_zeroes = true;
    po
}

/// Parse `expr` into a structure without evaluating it.
pub fn parse_expression(expr: &str) -> Result<MathStructure, ParseError> {
    parser::parse(expr, &ParseOptions::default())
}

/// Parse, evaluate, and print `expr`.
pub fn evaluate_to_string(expr: &str) -> Result<String, String> {
    let mut m = parse_expression(expr).map_err(|e| e.to_string())?;
    evaluate(&mut m);
    Ok(print::print(&m, &batch_print_options()))
}

/// Evaluate a structure in place.
///
/// TODO(port): delegate to the `calculate` module's `calculatesub` once the
/// merge engine lands; today this is a no-op so the pipeline is exercisable
/// end to end.
pub fn evaluate(_m: &mut MathStructure) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_runs_end_to_end() {
        // Without the merge engine this only round-trips; the assertions
        // tighten as evaluation lands.
        assert_eq!(evaluate_to_string("42").unwrap(), "42");
        assert_eq!(evaluate_to_string("x + y").unwrap(), "x + y");
    }

    #[test]
    fn parse_errors_surface() {
        assert!(evaluate_to_string("1+").is_err());
    }
}
