//! The symbolic half of `AbsFunction::calculate`
//! (BuiltinFunctions-number.cc:182).
//!
//! The numeric case lives in [`crate::builtins`]; what is here are the rules
//! that fire when the argument still contains unknowns:
//!
//! | expression | result |
//! |---|---|
//! | `abs(x - abs(x))` | `abs(x) - x` |
//! | `abs(-2x)` | `2 * abs(x)` |
//!
//! TODO(port): the reference also normalizes the *direction* of a difference,
//! so `abs(x - y)` and `abs(y - x)` both come out as `|y - x|` and cancel.
//! It decides that with `has_predominately_negative_sign` — negate when more
//! than half the terms carry a minus, the first term breaking an exact tie —
//! applied to the *evaluation-time* term order, which is not the print order
//! and which this port does not reproduce (`x - 1` stays but `1 - x` flips, so
//! the constant sorts last there while for two symbols the order is the
//! reverse of the printed one). Implementing the rule against our own order
//! flips the wrong cases, so it is left out rather than guessed at.

use crate::structure::MathStructure;
use qalc_num::Number;

/// `negate_struct` — negate each term of a sum, or the whole structure.
pub fn negate_struct(m: &MathStructure) -> MathStructure {
    match m {
        MathStructure::Addition(terms) => {
            MathStructure::Addition(terms.iter().map(negate_one).collect())
        }
        other => negate_one(other),
    }
}

fn negate_one(m: &MathStructure) -> MathStructure {
    match m {
        MathStructure::Number(n) => {
            let mut n = n.clone();
            n.negate();
            MathStructure::Number(n)
        }
        MathStructure::Multiplication(v) => {
            if let Some(MathStructure::Number(n)) = v.first() {
                let mut n = n.clone();
                n.negate();
                let mut out = v.clone();
                if n.is_one() && out.len() > 1 {
                    out.remove(0);
                    if out.len() == 1 {
                        return out.remove(0);
                    }
                } else {
                    out[0] = MathStructure::Number(n);
                }
                return MathStructure::Multiplication(out);
            }
            let mut out = vec![MathStructure::Number(Number::from_i64(-1))];
            out.extend(v.iter().cloned());
            MathStructure::Multiplication(out)
        }
        other => MathStructure::Multiplication(vec![
            MathStructure::Number(Number::from_i64(-1)),
            other.clone(),
        ]),
    }
}

/// `abs(v)`, as a structure.
fn abs_of(m: MathStructure) -> MathStructure {
    MathStructure::Function {
        id: crate::ids::FunctionId(crate::builtins::id::ABS),
        args: vec![m],
    }
}

/// The argument of an `abs(...)` call.
fn abs_argument(m: &MathStructure) -> Option<&MathStructure> {
    match m {
        MathStructure::Function { id, args }
            if id.0 == crate::builtins::id::ABS && args.len() == 1 =>
        {
            Some(&args[0])
        }
        _ => None,
    }
}

/// Rewrite an `abs` call whose argument is still symbolic. Returns true when
/// `m` was replaced.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let Some(arg) = abs_argument(m) else {
        return false;
    };
    // Numbers belong to the numeric path.
    if matches!(arg, MathStructure::Number(_)) {
        return false;
    }
    if let Some(v) = abs_minus_self(arg) {
        *m = v;
        return true;
    }
    // A product's absolute value is the product of the absolute values, which
    // is what pulls a numeric factor out of `abs(-2x)`.
    if let MathStructure::Multiplication(factors) = arg {
        *m = MathStructure::Multiplication(
            factors.iter().cloned().map(abs_of).collect(),
        );
        return true;
    }
    false
}

/// `abs(v - abs(v))` is `abs(v) - v`, because `v - abs(v)` is never positive
/// (BuiltinFunctions-number.cc:219). Returns the replacement, if the argument
/// has that shape.
fn abs_minus_self(arg: &MathStructure) -> Option<MathStructure> {
    let MathStructure::Addition(terms) = arg else {
        return None;
    };
    if terms.len() != 2 {
        return None;
    }
    for (i, term) in terms.iter().enumerate() {
        // The `abs` side has to be exactly `-abs(v)`; a different coefficient
        // leaves the sign of the sum genuinely unknown.
        let MathStructure::Multiplication(factors) = term else {
            continue;
        };
        if factors.len() != 2 {
            continue;
        }
        let MathStructure::Number(coefficient) = &factors[0] else {
            continue;
        };
        if !coefficient.is_minus_one() {
            continue;
        }
        let Some(inner) = abs_argument(&factors[1]) else {
            continue;
        };
        let other = &terms[1 - i];
        if inner.equals(other) {
            return Some(negate_struct(arg));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::session::Session;

    fn ev(s: &str) -> String {
        let mut session = Session::new();
        session.evaluate_line("/set approximation exact").ok();
        session.evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn abs_of_a_value_minus_its_own_absolute_value() {
        assert_eq!(ev("abs(x - abs(x)) - (abs(x) - x)"), "0");
    }

    #[test]
    fn a_numeric_factor_comes_out_of_the_bars() {
        assert_eq!(ev("abs(-2x)"), "2 |x|");
    }
}
