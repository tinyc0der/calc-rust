//! The symbolic halves of `AbsFunction::calculate` and
//! `SignumFunction::calculate` (BuiltinFunctions-number.cc:182 and 788).
//!
//! The numeric case lives in [`crate::builtins`]; what is here are the rules
//! that fire when the argument still contains unknowns:
//!
//! | expression | result |
//! |---|---|
//! | `abs(x - abs(x))` | `abs(x) - x` |
//! | `abs(-2x)` | `2 * abs(x)` |
//! | `sgn(-2x)` | `-sgn(x)` |
//! | `abs(1 - x)` | `abs(x - 1)` |
//! | `sgn(1 - x)` | `-sgn(x - 1)` |
//!
//! The last two are the *direction* normalization both functions end with:
//! [`has_predominately_negative_sign`] negates the argument when more than
//! half of its terms carry a minus sign, with the first term breaking an
//! exact tie. "First" means first in the evaluator's own term order, which is
//! not the printed order — see [`crate::sort::eval_order`], where that order
//! is reconstructed.
//!
//! Known divergence: the reference compares two *unknown variables* by object
//! address, so it stores `x - y` as `[-y, x]` and prints both `abs(x - y)`
//! and `abs(y - x)` as `|y - x|`. This port has no unknown-variable type — an
//! unknown is a `Symbolic`, which the C++ orders by name — so both normalize
//! to `|x - y|` instead. The direction differs; the normalization, and hence
//! `abs(x - y) - abs(y - x) = 0`, does not.

use crate::sort::has_negative_sign;
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

/// `f(m)`, as a structure, for a one-argument builtin.
fn call(id: u32, m: MathStructure) -> MathStructure {
    MathStructure::Function {
        id: crate::ids::FunctionId(id),
        args: vec![m],
    }
}

/// `abs(v)`, as a structure.
fn abs_of(m: MathStructure) -> MathStructure {
    call(crate::builtins::id::ABS, m)
}

/// `sgn(v)`, as a structure.
fn signum_of(m: MathStructure) -> MathStructure {
    call(crate::builtins::id::SIGNUM, m)
}

/// The single argument of a call to the builtin `id`.
fn argument_of(m: &MathStructure, id: u32) -> Option<&MathStructure> {
    match m {
        MathStructure::Function { id: f, args } if f.0 == id && args.len() == 1 => Some(&args[0]),
        _ => None,
    }
}

/// The argument of an `abs(...)` call.
fn abs_argument(m: &MathStructure) -> Option<&MathStructure> {
    argument_of(m, crate::builtins::id::ABS)
}

/// Rewrite an `abs` or `sgn` call whose argument is still symbolic. Returns
/// true when `m` was replaced.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    if args.len() != 1 {
        return false;
    }
    // Numbers belong to the numeric path in `crate::builtins`.
    if matches!(args[0], MathStructure::Number(_)) {
        return false;
    }
    match id.0 {
        crate::builtins::id::ABS => calculate_abs(m),
        crate::builtins::id::SIGNUM => calculate_signum(m),
        _ => false,
    }
}

/// `AbsFunction::calculate` past the numeric cases
/// (BuiltinFunctions-number.cc:182).
fn calculate_abs(m: &mut MathStructure) -> bool {
    let arg = abs_argument(m).expect("checked by the caller");
    if let Some(v) = abs_minus_self(arg) {
        *m = v;
        return true;
    }
    // A product's absolute value is the product of the absolute values, which
    // is what pulls a numeric factor out of `abs(-2x)`.
    if let MathStructure::Multiplication(factors) = arg {
        *m = MathStructure::Multiplication(factors.iter().cloned().map(abs_of).collect());
        return true;
    }
    if let Some(v) = flip_sign(arg) {
        *m = abs_of(v);
        return true;
    }
    false
}

/// `SignumFunction::calculate` past the numeric cases
/// (BuiltinFunctions-number.cc:788).
fn calculate_signum(m: &mut MathStructure) -> bool {
    let arg = argument_of(m, crate::builtins::id::SIGNUM).expect("checked by the caller");
    // `sgn` distributes over a product, so `sgn(-2x)` is `sgn(-2) sgn(x)`.
    // (This is also what keeps the non-addition half of
    // `has_predominately_negative_sign` unreachable below, as it is in the
    // C++: everything else that carries a minus sign is a product.)
    if let MathStructure::Multiplication(factors) = arg {
        *m = MathStructure::Multiplication(factors.iter().cloned().map(signum_of).collect());
        return true;
    }
    if let Some(v) = flip_sign(arg) {
        *m = negate_one(&signum_of(v));
        return true;
    }
    false
}

/// The argument, negated, when the reference would rather see it the other
/// way round — the `if(has_predominately_negative_sign(mstruct))` tail both
/// functions share.
///
/// The negated form is checked as well: the C++ relies on the negation being
/// a fixed point of the predicate, and a rewrite that was not would loop
/// forever here.
fn flip_sign(arg: &MathStructure) -> Option<MathStructure> {
    if !has_predominately_negative_sign(arg) {
        return None;
    }
    let flipped = negate_struct(arg);
    (!has_predominately_negative_sign(&flipped)).then_some(flipped)
}

/// `neg_sign_contains_addition` (BuiltinFunctions-trigonometry.cc:105): does
/// the leading minus sign sit on top of a sum, where it may be an artifact of
/// how the sum was written rather than a real sign?
fn neg_sign_contains_addition(m: &MathStructure) -> bool {
    match m {
        MathStructure::Addition(_) => true,
        MathStructure::Multiplication(_) => (0..m.size())
            .filter_map(|i| m.get(i))
            .any(neg_sign_contains_addition),
        MathStructure::Power { base, exponent } => {
            // A non-integer power is a root, which does not distribute.
            if matches!(exponent.as_ref(), MathStructure::Number(n) if !n.is_integer()) {
                return false;
            }
            neg_sign_contains_addition(base) || neg_sign_contains_addition(exponent)
        }
        _ => false,
    }
}

/// `MathStructure::containsInfinity(false, false, false)`.
fn contains_infinity(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => n.includes_infinity(),
        other => (0..other.size())
            .filter_map(|i| other.get(i))
            .any(contains_infinity),
    }
}

/// `has_predominately_negative_sign` (BuiltinFunctions-trigonometry.cc:114):
/// would the reference rather write this structure negated? A sum qualifies
/// when more than half of its terms carry a minus sign; on an exact tie the
/// *first* term decides, and first means first in the evaluator's term order
/// ([`crate::sort::eval_order`]), not in the printed one.
pub fn has_predominately_negative_sign(m: &MathStructure) -> bool {
    if has_negative_sign(m) && !neg_sign_contains_addition(m) {
        return true;
    }
    if contains_infinity(m) {
        return false;
    }
    let MathStructure::Addition(terms) = m else {
        return false;
    };
    if terms.is_empty() {
        return false;
    }
    let negative = terms.iter().filter(|t| has_negative_sign(t)).count();
    if negative > terms.len() / 2 {
        return true;
    }
    if terms.len() % 2 == 0 && negative == terms.len() / 2 {
        let first = crate::sort::eval_order(terms)[0];
        return has_negative_sign(&terms[first]);
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

    /// Both directions of a difference normalize to one, so they cancel.
    /// (Reference: `|y - x|` for both — see the module note on unknown
    /// variables.)
    #[test]
    fn the_direction_of_a_difference_is_normalized() {
        assert_eq!(ev("abs(x - y)"), "|x - y|");
        assert_eq!(ev("abs(y - x)"), "|x - y|");
        assert_eq!(ev("abs(x - y) - abs(y - x)"), "0");
        assert_eq!(ev("abs(2x - y)"), "|2x - y|");
        assert_eq!(ev("abs(y - 2x)"), "|2x - y|");
    }

    /// Verified against the reference binary: a constant term is stored last,
    /// so `1 - x` is the one that flips and both print `|x - 1|`.
    #[test]
    fn a_constant_term_is_the_last_one() {
        assert_eq!(ev("abs(x - 1)"), "|x - 1|");
        assert_eq!(ev("abs(1 - x)"), "|x - 1|");
    }

    /// Verified against the reference binary: one minus sign out of three
    /// terms is not a majority, and an odd count has no tie to break.
    #[test]
    fn a_single_minus_among_three_terms_does_not_flip() {
        assert_eq!(ev("abs(x + y - z)"), "|x + y - z|");
    }

    #[test]
    fn signum_normalizes_the_same_difference_and_keeps_the_sign() {
        assert_eq!(ev("sgn(x - y)"), "sgn(x - y)");
        assert_eq!(ev("sgn(y - x)"), "-sgn(x - y)");
        assert_eq!(ev("sgn(x - y) + sgn(y - x)"), "0");
    }

    #[test]
    fn signum_distributes_over_a_product() {
        assert_eq!(ev("sgn(-2x)"), "-sgn(x)");
        assert_eq!(ev("sgn(2x)"), "sgn(x)");
    }

    /// `x/abs(x)` and `sgn(x)` are the same value written two ways.
    #[test]
    fn a_quotient_by_the_absolute_value_cancels_a_signum() {
        assert_eq!(ev("x / abs(x) - sgn(x)"), "0");
        assert_eq!(ev("abs(x) / x - sgn(x)"), "0");
        // The quotient itself is left alone, as in the reference.
        assert_eq!(ev("x / abs(x)"), "x / |x|");
    }
}
