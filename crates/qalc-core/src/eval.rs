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

/// Apply an outer `to <target>` conversion.
///
/// The C++ CLI splits the "to" expression off before parsing
/// (`Calculator::separateToExpression`) and then either folds it into the
/// print options (number bases) or runs `Calculator::convert` on the value.
/// This port keeps the conversion in the tree and unwraps it here.
///
/// When there is no explicit conversion the automatic post-conversion runs
/// instead, which is what turns `50 ohm * 2 A` into `100 V`
/// (`POST_CONVERSION_OPTIMAL_SI`, Calculator-calculate.cc:4043).
pub fn apply_conversion(m: &mut MathStructure, po: &mut PrintOptions) -> Result<(), String> {
    let MathStructure::Conversion { value, target } = m else {
        // A duration that came from subtracting two dates keeps its days;
        // see `datetime::took_date_duration`.
        let from_dates = crate::datetime::took_date_duration();
        if !from_dates {
            if let Some(store) = crate::units::store() {
                crate::units::convert_to_optimal(store, m);
            }
        }
        return Ok(());
    };
    match target {
        ConversionTarget::NumberBase { base, bits } => {
            po.base = *base;
            po.binary_bits = *bits;
            let v = (**value).clone();
            *m = v;
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
            let v = (**value).clone();
            *m = v;
        }
        ConversionTarget::BaseUnits => {
            let store = crate::units::store()
                .ok_or_else(|| "unit definitions are not available".to_string())?;
            let mut v = (**value).clone();
            crate::units::convert_to_base_units(store, &mut v);
            evaluate_calculated(&mut v);
            *m = v;
        }
        ConversionTarget::Unit { expr, mix, prefix } => {
            let store = crate::units::store()
                .ok_or_else(|| "unit definitions are not available".to_string())?;
            let mut r = crate::units::convert_to(store, value, expr, *mix)?;
            crate::units::apply_prefix_mode(store, &mut r, *prefix);
            *m = r;
        }
    }
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
    evaluate_calculated_with(m, &EvaluationOptions::default());
}

/// [`evaluate_calculated`] with explicit evaluation options — the session
/// passes its own so `/set approximation exact` reaches the merge engine.
pub fn evaluate_calculated_with(m: &mut MathStructure, eo: &EvaluationOptions) {
    for _ in 0..MAX_EVAL_PASSES {
        let functions_changed = builtins::calculate_functions_eo(m, eo);
        let merged = m.calculatesub(eo);
        if !functions_changed && !merged {
            break;
        }
    }
    // Dates: text that denotes a date becomes a date value, and date
    // arithmetic folds. This runs after unit reduction, because a duration
    // like `523d` only becomes a plain count of seconds once units resolve.
    // This runs once, after unit reduction and with no re-evaluation
    // afterwards: a duration like `523d` only becomes a plain count of
    // seconds once units resolve, and the day count this produces for a
    // date difference must not be reduced back to seconds.
    crate::datetime::apply(m);
    // `eo.isolate_x` (default on for the CLI): an equation in one unknown is
    // solved rather than merely simplified.
    crate::solve::isolate_x_toplevel(m, eo);
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
