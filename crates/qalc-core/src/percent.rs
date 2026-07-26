//! Simplified percentage handling.
//!
//! With `simplified_percentage` enabled (libqalculate's default), a percent
//! term inside a sum means "of the running total" rather than a bare
//! fraction. Verified against the reference binary:
//!
//! | expression            | result |
//! |-----------------------|--------|
//! | `100 + 10%`           | 110    |
//! | `100 + 10% + 10%`     | 121    |
//! | `100 - 10% - 10%`     | 81     |
//! | `100 + (10 + 10)%`    | 120    |
//! | `50% + 2`             | 2.5    |
//! | `123% - 3% + 10%`     | 1.3    |
//! | `100 * 10%`           | 10     |
//!
//! Two rules follow from those: the accumulation is strictly left to right
//! (the second `10%` in `100 + 10% + 10%` is 10% of 110, not of 100), and it
//! only applies when the running total did not itself start as a percentage
//! — a sum whose first term is a percent treats every term as a plain
//! fraction. Outside a sum, `x%` is just `x/100`.

use crate::builtins::id;
use crate::structure::MathStructure;
use qalc_num::Number;

/// Is this node a percent marker?
fn as_percent(m: &MathStructure) -> Option<&MathStructure> {
    match m {
        MathStructure::Function { id: f, args } if f.0 == id::PERCENT && args.len() == 1 => {
            Some(&args[0])
        }
        _ => None,
    }
}

/// Is this node a percent marker, or a negated one (`- 10%`)?
/// Returns the inner value and whether it was negated.
fn as_signed_percent(m: &MathStructure) -> Option<(&MathStructure, bool)> {
    if let Some(inner) = as_percent(m) {
        return Some((inner, false));
    }
    // Subtraction is `Multiplication[-1, Percent(x)]`.
    if let MathStructure::Multiplication(factors) = m {
        if factors.len() == 2 {
            if let MathStructure::Number(n) = &factors[0] {
                if n.is_minus_one() {
                    if let Some(inner) = as_percent(&factors[1]) {
                        return Some((inner, true));
                    }
                }
            }
        }
    }
    None
}

/// Does this expression begin with a percentage, making the whole sum a
/// plain-fraction sum?
fn starts_as_percentage(m: &MathStructure) -> bool {
    as_signed_percent(m).is_some()
}

/// `x%` as a plain fraction: `x * (1/100)`.
fn to_fraction(inner: &MathStructure) -> MathStructure {
    MathStructure::Multiplication(vec![
        inner.clone(),
        MathStructure::Number(Number::from_ints(1, 100, 0)),
    ])
}

/// Rewrite percent markers throughout `m`, applying the simplified rule
/// inside sums.
pub fn apply(m: &mut MathStructure) {
    // A sum must claim its percent terms before the recursion reaches them,
    // otherwise they would already have collapsed into plain fractions.
    if let MathStructure::Addition(terms) = m {
        rewrite_sum(terms);
    }
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            apply(child);
        }
    }
    // Any percent marker not consumed by a sum is a plain fraction.
    if let Some(inner) = as_percent(m) {
        let f = to_fraction(inner);
        *m = f;
    }
}

/// Apply the running-total rule across the terms of one sum.
fn rewrite_sum(terms: &mut Vec<MathStructure>) {
    if terms.is_empty() {
        return;
    }
    // A sum that opens with a percentage uses plain fractions throughout.
    if starts_as_percentage(&terms[0]) {
        for t in terms.iter_mut() {
            if let Some((inner, negated)) = as_signed_percent(t) {
                let frac = to_fraction(inner);
                *t = if negated {
                    MathStructure::Multiplication(vec![
                        MathStructure::Number(Number::from_i64(-1)),
                        frac,
                    ])
                } else {
                    frac
                };
            }
        }
        return;
    }
    // Otherwise fold left, replacing each percent term with a fraction of
    // everything accumulated so far.
    let mut acc: Vec<MathStructure> = vec![terms[0].clone()];
    for t in terms.iter().skip(1) {
        let next = match as_signed_percent(t) {
            Some((inner, negated)) => {
                let running = if acc.len() == 1 {
                    acc[0].clone()
                } else {
                    MathStructure::Addition(acc.clone())
                };
                let mut factors = vec![
                    running,
                    inner.clone(),
                    MathStructure::Number(Number::from_ints(1, 100, 0)),
                ];
                if negated {
                    factors.insert(0, MathStructure::Number(Number::from_i64(-1)));
                }
                MathStructure::Multiplication(factors)
            }
            None => t.clone(),
        };
        acc.push(next);
    }
    *terms = acc;
}

#[cfg(test)]
mod tests {
    use crate::eval::evaluate_to_string;

    fn ev(s: &str) -> String {
        evaluate_to_string(s).expect("evaluates")
    }

    #[test]
    fn plain_percent_outside_a_sum() {
        assert_eq!(ev("50%"), "0.5");
        assert_eq!(ev("100 * 10%"), "10");
        assert_eq!(ev("100 / 10%"), "1000");
    }

    #[test]
    fn percent_of_running_total() {
        assert_eq!(ev("100 + 10%"), "110");
        assert_eq!(ev("2 + 50%"), "3");
        assert_eq!(ev("100 - 10%"), "90");
    }

    #[test]
    fn accumulation_is_left_to_right() {
        // The second 10% is of 110, not of 100.
        assert_eq!(ev("100 + 10% + 10%"), "121");
        assert_eq!(ev("100 - 10% - 10%"), "81");
    }

    #[test]
    fn percent_first_means_plain_fractions() {
        assert_eq!(ev("50% + 2"), "2.5");
        assert_eq!(ev("123% - 3% + 10%"), "1.3");
        assert_eq!(ev("10% - 20%"), "-0.1");
    }

    #[test]
    fn parenthesized_percent_argument() {
        assert_eq!(ev("100 + (10 + 10)%"), "120");
    }

    #[test]
    fn percent_before_minus_is_postfix() {
        // `10 % -3` is 0.1 - 3, not a modulo.
        assert_eq!(ev("10 % -3"), "-2.9");
    }
}
