//! End-to-end evaluation facade: parse → calculate → print.
//!
//! Mirrors `Calculator::calculateAndPrint` (Calculator-calculate.cc).

use crate::builtins;
use crate::parser::{self, ParseError};
use crate::print;
use crate::structure::{ConversionTarget, MathStructure};
use crate::options::EvaluationOptions;
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
    let mut po = batch_print_options();
    apply_conversion(&mut m, &mut po)?;
    Ok(print::print(&m, &po))
}

/// Fold an outer `to <base>` conversion into the print options, mirroring
/// how the C++ CLI applies a parsed "to" expression to `printops` rather
/// than to the value.
fn apply_conversion(m: &mut MathStructure, po: &mut PrintOptions) -> Result<(), String> {
    let MathStructure::Conversion { value, target } = m else {
        return Ok(());
    };
    match target {
        ConversionTarget::NumberBase { base, bits } => {
            po.base = *base;
            po.binary_bits = *bits;
        }
        ConversionTarget::Base(expr) => {
            let mut b = (**expr).clone();
            evaluate(&mut b);
            match &b {
                MathStructure::Number(n) => match n.to_i64() {
                    Some(v) if (2..=36).contains(&v) => po.base = v as i32,
                    _ => return Err("unsupported number base".to_string()),
                },
                _ => return Err("number base must evaluate to a number".to_string()),
            }
        }
        ConversionTarget::Unit(_) => {
            // TODO(port): unit conversion needs the unit registry.
            return Err("unit conversion is not implemented yet".to_string());
        }
    }
    *m = (**value).clone();
    Ok(())
}

/// Evaluate a structure in place using default evaluation options.
///
/// Mirrors `MathStructure::eval`: evaluate function calls, then run the
/// arithmetic merge engine, repeating while either makes progress (a
/// resolved function can expose a new merge, and a merge can complete a
/// function's arguments).
pub fn evaluate(m: &mut MathStructure) {
    // Percent markers become concrete arithmetic before any merging, since
    // a percent inside a sum depends on the sum's term order.
    crate::percent::apply(m);
    evaluate_calculated(m);
}

/// The evaluation loop proper, with percent rewriting already done.
pub fn evaluate_calculated(m: &mut MathStructure) {
    let eo = EvaluationOptions::default();
    for _ in 0..MAX_EVAL_PASSES {
        let functions_changed = builtins::calculate_functions(m);
        let merged = m.calculatesub(&eo);
        if !functions_changed && !merged {
            break;
        }
    }
    // Canonical ordering, as the C++ does in evalSort before printing.
    crate::sort::sort(m);
}

/// Guard against a pathological rewrite cycle.
const MAX_EVAL_PASSES: usize = 16;

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
