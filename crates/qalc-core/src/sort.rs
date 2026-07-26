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
//! TODO(port): addition ordering by total degree, `minus_last`, unit
//! parent/child ordering, and the full type-rank tail of `sortCompare`.

use crate::structure::MathStructure;

/// Sort rank inside a multiplication: lower sorts earlier.
fn multiplication_rank(m: &MathStructure) -> u8 {
    match m {
        MathStructure::Number(_) => 0,
        // Bare symbols are "unknowns" and go last (before units, which the
        // unit port will add as rank 4).
        MathStructure::Symbolic(_) => 3,
        MathStructure::Unit(_) => 4,
        _ => 1,
    }
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

/// Sort the factors of every multiplication in the tree.
pub fn sort(m: &mut MathStructure) {
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            sort(child);
        }
    }
    if let MathStructure::Multiplication(factors) = m {
        // A stable sort keeps the relative order of factors the rules do not
        // distinguish, matching sortCompare's "0 means preserve order".
        factors.sort_by(|a, b| {
            multiplication_rank(a)
                .cmp(&multiplication_rank(b))
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
