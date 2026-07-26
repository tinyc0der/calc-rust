//! Canonical ordering of terms and factors — port of the parts of
//! `sortCompare` / `MathStructure::sort` (MathStructure-print.cc:128) that
//! the transcripts exercise.
//!
//! Verified against the reference binary:
//!
//! | input        | output        | rule |
//! |--------------|---------------|------|
//! | `x*2`        | `2x`          | numbers first |
//! | `y*x`        | `xy`          | symbols alphabetical |
//! | `3*z*y*x`    | `3xyz`        | both together |
//! | `x*zeta(x)`  | `zeta(x) * x` | bare symbols last |
//! | `2*a*x^2`    | `2x^2 * a`    | a power is not a bare symbol |
//!
//! In a multiplication the C++ pushes units, percentages and *unknowns*
//! (bare symbols) to the end before consulting the general type order, which
//! is why `zeta(x)` precedes `x` despite z sorting after x.
//!
//! Addition ordering, also verified against the reference:
//!
//! | input        | output        | rule |
//! |--------------|---------------|------|
//! | `1 + x + x^2`| `x^2 + x + 1` | total degree, descending |
//! | `5 + x`      | `x + 5`       | numbers last in a sum |
//! | `6 - 5x^2 + 3x^2` | `6 - 2x^2` | `minus_last`, but only for two terms |
//! | `6 - 5x^2 + 3x^2 + 2y` | `2y - 2x^2 + 6` | three terms: degree order, then |
//! |              |               | the leading negative term is swapped back |
//!
//! TODO(port): unit parent/child ordering in sums, complex-number ordering,
//! and the `preserve_format` special cases.

use crate::structure::MathStructure;
use qalc_num::Number;

/// Sort rank inside a multiplication: lower sorts earlier.
fn multiplication_rank(m: &MathStructure) -> u8 {
    if crate::units::is_unit_exp(m) {
        // A unit, or a unit raised to a power, always sorts last.
        return 4;
    }
    match m {
        MathStructure::Number(_) => 0,
        // Bare symbols are "unknowns" and go last (before units).
        MathStructure::Symbolic(_) => 3,
        _ => 1,
    }
}

/// Units sort among themselves by reference name (`kg*m^2`, `A*s^3`).
fn unit_key(m: &MathStructure) -> Option<String> {
    let base = match m {
        MathStructure::Unit { .. } => m,
        MathStructure::Power { base, .. } => base,
        _ => return None,
    };
    let MathStructure::Unit { id, .. } = base else {
        return None;
    };
    let store = crate::units::store()?;
    Some(store.reference_name(*id).to_string())
}

/// A stable tiebreaker for factors of the same rank: symbols and power
/// bases compare by name.
fn factor_key(m: &MathStructure) -> Option<&str> {
    match m {
        MathStructure::Symbolic(s) => Some(s),
        MathStructure::Power { base, .. } => match base.as_ref() {
            MathStructure::Symbolic(s) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

// ----------------------------------------------------------------------
// Addition ordering
// ----------------------------------------------------------------------

/// `MathStructure::hasNegativeSign` (`MathStructure.cc`).
fn has_negative_sign(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => n.is_negative(),
        MathStructure::Multiplication(v) => v.first().is_some_and(has_negative_sign),
        _ => false,
    }
}

/// `isUnknown()` — a symbolic leaf (this port has no unknown-variable type).
fn is_unknown(m: &MathStructure) -> bool {
    m.is_symbolic()
}

/// `get_total_degree` (MathStructure-print.cc:107): the sum of the exponents
/// of the unknown factors of a term.
fn total_degree(m: &MathStructure, top: bool) -> Number {
    let mut deg = Number::new();
    match m {
        MathStructure::Multiplication(v) if top => {
            for c in v {
                let d = total_degree(c, false);
                deg.add(&d);
            }
        }
        MathStructure::Power { base, exponent } if is_unknown(base) => {
            if let MathStructure::Number(n) = exponent.as_ref() {
                deg.add(n);
            }
        }
        other if is_unknown(other) => {
            deg.add_i64(1);
        }
        _ => {}
    }
    deg
}

/// `a*b^-1` detection — the `isdiv` flags of `sortCompare`.
fn is_division(m: &MathStructure) -> bool {
    let neg_exp = |p: &MathStructure| {
        matches!(p, MathStructure::Power { exponent, .. } if has_negative_sign(exponent))
    };
    match m {
        MathStructure::Multiplication(v) => v.iter().any(neg_exp),
        other => neg_exp(other),
    }
}

/// The unknown factors of a term, as `(name, exponent)` in tree order.
fn unknown_factors(m: &MathStructure) -> Vec<(String, Number)> {
    let one = Number::from_i64(1);
    let of = |f: &MathStructure| -> Option<(String, Number)> {
        match f {
            MathStructure::Symbolic(s) => Some((s.clone(), one.clone())),
            MathStructure::Power { base, exponent } => match (base.as_ref(), exponent.as_ref()) {
                (MathStructure::Symbolic(s), MathStructure::Number(n)) => {
                    Some((s.clone(), n.clone()))
                }
                _ => None,
            },
            _ => None,
        }
    };
    match m {
        MathStructure::Multiplication(v) => v.iter().filter_map(of).collect(),
        other => of(other).into_iter().collect(),
    }
}

/// The type rank used as the last resort inside an addition. Lower is
/// earlier; the values follow the C++ check order in `sortCompare`
/// (`if(mstruct2.isX()) return -1` means "X sorts later").
fn addition_type_rank(m: &MathStructure) -> u8 {
    if crate::units::is_unit_exp(m) {
        return 3;
    }
    match m {
        MathStructure::DateTime(_) => 0,
        MathStructure::Variable(_) => 1,
        MathStructure::Symbolic(_) => 2,
        MathStructure::Power { .. } => 4,
        MathStructure::Multiplication(_) => {
            if is_division(m) {
                7
            } else {
                5
            }
        }
        MathStructure::Number(_) => 6,
        MathStructure::Addition(_) => 8,
        MathStructure::Function { .. } => 9,
        MathStructure::Undefined => 10,
        MathStructure::Aborted => 15,
        _ => 11,
    }
}

/// `sortCompare` restricted to an addition parent: `Less` when `a` should be
/// placed before `b`.
fn addition_compare(a: &MathStructure, b: &MathStructure, minus_last: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if minus_last {
        let (m1, m2) = (has_negative_sign(a), has_negative_sign(b));
        if m1 && !m2 {
            return Ordering::Greater;
        }
        if m2 && !m1 {
            return Ordering::Less;
        }
    }
    let (d1, d2) = (is_division(a), is_division(b));
    if d1 == d2 {
        let (deg1, deg2) = (total_degree(a, true), total_degree(b, true));
        if deg1.is_greater_than(&deg2) {
            return Ordering::Less;
        }
        if deg2.is_greater_than(&deg1) {
            return Ordering::Greater;
        }
        if !deg1.is_zero() {
            let (u1, u2) = (unknown_factors(a), unknown_factors(b));
            for (x, y) in u1.iter().zip(u2.iter()) {
                match x.0.cmp(&y.0) {
                    Ordering::Equal => {}
                    other => return other,
                }
                // Equal bases: the larger exponent sorts first.
                if x.1.is_greater_than(&y.1) {
                    return Ordering::Less;
                }
                if y.1.is_greater_than(&x.1) {
                    return Ordering::Greater;
                }
            }
            match u1.len().cmp(&u2.len()) {
                Ordering::Equal => {}
                other => return other,
            }
        }
    }
    addition_type_rank(a).cmp(&addition_type_rank(b))
}

/// The insertion sort of `MathStructure::sort` (MathStructure-print.cc:556):
/// each element is inserted before the first element it compares less than.
/// The comparator is deliberately not a total order, so a general-purpose
/// sort would give different results.
fn insertion_sort_addition(items: &mut Vec<MathStructure>, minus_last: bool) {
    let mut sorted: Vec<MathStructure> = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        let pos = (0..sorted.len())
            .find(|&i| addition_compare(&item, &sorted[i], minus_last) == std::cmp::Ordering::Less);
        match pos {
            Some(i) => sorted.insert(i, item),
            None => sorted.push(item),
        }
    }
    *items = sorted;
}

/// Sort the terms of every addition and the factors of every multiplication.
pub fn sort(m: &mut MathStructure) {
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            sort(child);
        }
    }
    if let MathStructure::Addition(terms) = m {
        // `po2.sort_options.minus_last = po.sort_options.minus_last &&
        // SIZE == 2` — the flag only applies to a two-term sum.
        let minus_last = terms.len() == 2;
        insertion_sort_addition(terms, minus_last);
        // With more than two terms the C++ instead pulls the first
        // non-negative term to the front when the sum would otherwise start
        // with a minus sign.
        if terms.len() > 2 && has_negative_sign(&terms[0]) {
            if let Some(i) = terms.iter().skip(1).position(|t| !has_negative_sign(t)) {
                let t = terms.remove(i + 1);
                terms.insert(0, t);
            }
        }
    }
    if let MathStructure::Multiplication(factors) = m {
        // A stable sort keeps the relative order of factors the rules do not
        // distinguish, matching sortCompare's "0 means preserve order".
        factors.sort_by(|a, b| {
            multiplication_rank(a)
                .cmp(&multiplication_rank(b))
                .then_with(|| match (unit_key(a), unit_key(b)) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    _ => std::cmp::Ordering::Equal,
                })
                .then_with(|| match (factor_key(a), factor_key(b)) {
                    (Some(x), Some(y)) => x.cmp(y),
                    _ => std::cmp::Ordering::Equal,
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::session::Session;

    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn numeric_coefficient_comes_first() {
        assert_eq!(ev("x*2"), "2x");
        assert_eq!(ev("2*x"), "2x");
        assert_eq!(ev("x+x"), "2x");
    }

    #[test]
    fn symbols_sort_alphabetically() {
        assert_eq!(ev("y*x"), "xy");
        assert_eq!(ev("3*z*y*x"), "3xyz");
    }

    #[test]
    fn bare_symbols_sort_after_other_factors() {
        // `zeta` sorts after `x` alphabetically, but a bare symbol still
        // goes last.
        assert_eq!(ev("x*zeta(x)"), "zeta(x) * x");
        assert_eq!(ev("2*a*x^2"), "2x^2 * a");
    }

    #[test]
    fn powers_keep_base_ordering() {
        assert_eq!(ev("x^2*y^3"), "x^2 * y^3");
    }
}
