//! End-to-end evaluation facade: parse → calculate → print.
//!
//! Mirrors `Calculator::calculateAndPrint` (Calculator-calculate.cc).

use crate::builtins;
use crate::options::EvaluationOptions;
use crate::parser::{self, ParseError};
use crate::print;
use crate::structure::{ConversionTarget, MathStructure};
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
            let is_time = *base == qalc_num::options::base::TIME;
            let v = (**value).clone();
            *m = v;
            if is_time {
                convert_for_time_format(m);
            }
        }
        ConversionTarget::Base(expr) => {
            let mut b = (**expr).clone();
            let mut eo2 = crate::options::EvaluationOptions::default();
            eo2.approximation = crate::options::ApproximationMode::Approximate;
            eo2.split_squares = false;
            evaluate_calculated_with(&mut b, &eo2);
            match &b {
                MathStructure::Number(n) => match n.to_i64() {
                    Some(v) if (2..=36).contains(&v) => po.base = v as i32,
                    _ if n.is_real() && n.is_greater_than(&qalc_num::Number::from_i64(1)) => {
                        po.base = qalc_num::options::base::CUSTOM;
                        po.custom_base = Some(n.clone());
                    }
                    _ => return Err("unsupported number base".to_string()),
                },
                _ => return Err("number base must evaluate to a number".to_string()),
            }
            let mut v = (**value).clone();
            if po.base == qalc_num::options::base::CUSTOM {
                evaluate_calculated_with(&mut v, &eo2);
            }
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
        ConversionTarget::TimeZone { offset_minutes } => {
            match offset_minutes {
                None => po.time_zone = qalc_num::options::TimeZoneMode::Utc,
                Some(n) => {
                    po.time_zone = qalc_num::options::TimeZoneMode::Custom;
                    po.custom_time_zone = *n;
                }
            }
            let v = (**value).clone();
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

/// `convert_for_time_format` (Calculator-calculate.cc:929).
///
/// Time format counts in hours, so a time-valued result is divided by `h`
/// before it is printed: `10h 31min + 8h 30min to time` reaches the printer as
/// the bare number 19.01666…, which the sexagesimal printer renders as `19:01`.
/// A value that is not a plain duration is left alone.
fn convert_for_time_format(m: &mut MathStructure) -> bool {
    let Some(store) = crate::units::store() else {
        return false;
    };
    let is_seconds = |x: &MathStructure| -> bool {
        let MathStructure::Unit { id, .. } = x else {
            return false;
        };
        store
            .base_form(*id)
            .is_some_and(|f| f.sig.len() == 1 && f.sig.values().all(|e| *e == 1) && !f.nonlinear)
            && store
                .base_form(*id)
                .and_then(|f| f.sig.keys().next().copied())
                .is_some_and(|b| store.reference_name(b) == "s")
    };
    let is_duration = |x: &MathStructure| -> bool {
        match x {
            MathStructure::Multiplication(f) => {
                f.len() == 2 && matches!(f[0], MathStructure::Number(_)) && is_seconds(&f[1])
            }
            _ => is_seconds(x),
        }
    };
    let convertible = match &*m {
        MathStructure::Addition(terms) => terms.iter().all(is_duration),
        other => is_duration(other),
    };
    if !convertible {
        return false;
    }
    let Some(hour) = store.resolve_name("h") else {
        return false;
    };
    let mut divided = MathStructure::Multiplication(vec![
        m.clone(),
        MathStructure::Power {
            base: Box::new(hour),
            exponent: Box::new(MathStructure::Number(qalc_num::Number::from_i64(-1))),
        },
    ]);
    // `eo2.sync_units` in the C++: the terms may be in different time units
    // (`18 h + 61 min`), which only cancel against `h` once everything is in
    // seconds.
    crate::units::convert_to_base_units(store, &mut divided);
    evaluate_calculated(&mut divided);
    if crate::units::contains_unit(&divided) {
        return false;
    }
    *m = divided;
    true
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
    // `limit()` is evaluated before anything else gets to numerify its
    // argument. The C++ `LimitFunction::calculate` forces
    // `APPROXIMATION_EXACT` for the whole call, and the limit machinery needs
    // it: it recognises `0/0` structurally, so an argument whose `sqrt(3)` has
    // already become `1.732050808` no longer looks indeterminate. See
    // `limit::resolve_exactly`. Under `Exact` the ordinary loop below already
    // does exactly this, so the pre-pass only runs when it would differ.
    let is_limit = crate::limit::is_limit_call(m);
    if eo.approximation != crate::options::ApproximationMode::Exact {
        crate::limit::resolve_exactly(m);
        if is_limit && !crate::limit::is_limit_call(m) {
            crate::sort::sort(m);
            return;
        }
    }
    for _ in 0..MAX_EVAL_PASSES {
        let ranges_changed = evaluate_ranges(m);
        let functions_changed = builtins::calculate_functions_eo(m, eo);
        let merged = m.calculatesub(eo);
        if !ranges_changed && !functions_changed && !merged {
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
    //
    // The C++ solves inside `MathStructure::calculatesub`, so a solution it
    // produces is merged by the same loop that produced it. Here the solver is
    // a separate top-level step (`SOLVING` guards against the re-entry that
    // would otherwise occur), so its output has to be offered to the merge
    // engine explicitly — otherwise `x^3 = 2` answers `x = cbrt(2)` even under
    // `Approximate`, and `sin(3x) = 1/3` answers `pi / 6` where a second pass
    // would have given the decimal.
    //
    // Restricted to the non-`Exact` modes on purpose. Under `Exact` the extra
    // pass changes nothing about the *value* — there is nothing left to
    // numerify — but it does re-associate the surds the closed forms are
    // written in (`(sqrt(5) + 3) / 2` becomes `sqrt(5) / 2 + 3/2`), and the
    // reference's spelling of those is what `polynomial.batch:6`,
    // `solver.batch:7` and `solver.batch:19` pin.
    let solved = crate::solve::isolate_x_toplevel(m, eo);
    if solved && eo.approximation != crate::options::ApproximationMode::Exact {
        for _ in 0..MAX_EVAL_PASSES {
            let ranges_changed = evaluate_ranges(m);
            let functions_changed = builtins::calculate_functions_eo(m, eo);
            let merged = m.calculatesub(eo);
            if !ranges_changed && !functions_changed && !merged {
                break;
            }
        }
    }
    // Exact `ln(e)` identities, mirroring the C++ `ln(e^x) -> x` and `ln(e) -> 1`.
    // These fire before numerification so `ln(e)` stays exact `1` rather than
    // approximate `ln(2.718...) -> 1.000…`.
    eval_ln_exact(m);

    // `KnownVariable` numerification (`pi`, `e`, `phi`): the C++ keeps them
    // symbolic through the exact pass (so `sin(pi/2)` collapses to exact `1`)
    // and numerifies the survivors on the second pass — but only when the
    // result is not still symbolic (e.g. `e^x` must stay `e^x`, not
    // `2.718^x`). With pi installed as a variable this never happens —
    // `sin(pi/2)` would see numeric `1.570…` and return approximate `1.000…`
    // — and without it lone `pi` would stay symbolic. Do it here, after the
    // exact identities have had their chance.
    if eo.approximation != crate::options::ApproximationMode::Exact {
        if numerify_known_constants(m) {
            for _ in 0..MAX_EVAL_PASSES {
                let functions_changed = builtins::calculate_functions_eo(m, eo);
                let merged = m.calculatesub(eo);
                if !functions_changed && !merged {
                    break;
                }
            }
        }
    }
    // Canonical ordering, as the C++ does in evalSort before printing.
    crate::sort::sort(m);
}

fn numerify_known_constants(m: &mut MathStructure) -> bool {
    let mut changed = false;
    match m {
        MathStructure::Symbolic(s) => {
            let repl = match s.as_str() {
                "pi" => {
                    let mut n = qalc_num::Number::new();
                    n.pi();
                    Some(n)
                }
                "e" => {
                    let mut n = qalc_num::Number::new();
                    n.e();
                    Some(n)
                }
                "phi" => {
                    let mut n = qalc_num::Number::from_i64(5);
                    n.sqrt();
                    n.add_i64(1);
                    n.divide_i64(2);
                    Some(n)
                }
                _ => None,
            };
            if let Some(n) = repl {
                *m = MathStructure::Number(n);
                changed = true;
            }
        }
        _ => {
            for i in 0..m.size() {
                if let Some(child) = m.get_mut(i) {
                    if numerify_known_constants(child) {
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

fn eval_ln_exact(m: &mut MathStructure) -> bool {
    let mut changed = false;
    // Recurse first.
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            if eval_ln_exact(child) {
                changed = true;
            }
        }
    }
    // Check for ln(e) and ln(e^n) after children are processed.
    if let MathStructure::Function { id, args } = m {
        if id.0 == crate::builtins::id::LN && args.len() == 1 {
            match &args[0] {
                MathStructure::Symbolic(s) if s == "e" => {
                    *m = MathStructure::Number(qalc_num::Number::from_i64(1));
                    return true;
                }
                MathStructure::Power { base, exponent } => {
                    if let MathStructure::Symbolic(s) = base.as_ref() {
                        if s == "e" {
                            *m = (**exponent).clone();
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    changed
}

enum RangeOperation {
    Sum,
    Product,
}

/// Expand `sum(term, lower, upper[, index])` and `product(...)` over an
/// inclusive integer range before ordinary function and arithmetic evaluation.
fn evaluate_ranges(m: &mut MathStructure) -> bool {
    let mut changed = false;
    for index in 0..m.size() {
        if let Some(child) = m.get_mut(index) {
            changed |= evaluate_ranges(child);
        }
    }

    let MathStructure::Function { id, args } = m else {
        return changed;
    };
    let Some(store) = crate::units::store_if_ready() else {
        return changed;
    };
    let registry = store.registry();
    let operation = if registry.find_function_id("sum") == Some(*id) {
        RangeOperation::Sum
    } else if registry.find_function_id("product") == Some(*id) {
        RangeOperation::Product
    } else {
        return changed;
    };
    if !(3..=4).contains(&args.len()) {
        return changed;
    }

    let Some(lower) = args[1].number().and_then(|n| n.to_i64()) else {
        return changed;
    };
    let Some(upper) = args[2].number().and_then(|n| n.to_i64()) else {
        return changed;
    };
    if lower > upper || upper.saturating_sub(lower) > 100_000 {
        return changed;
    }

    let term = args[0].clone();
    let index = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| MathStructure::symbolic("x"));
    let terms = (lower..=upper)
        .map(|value| {
            let mut term = term.clone();
            crate::matrix::replace(&mut term, &index, &MathStructure::from_i64(value));
            term
        })
        .collect();
    *m = match operation {
        RangeOperation::Sum => MathStructure::Addition(terms),
        RangeOperation::Product => MathStructure::Multiplication(terms),
    };
    true
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
