//! `/assume <sign>` — the CLI command that tells the evaluator what it may
//! take for granted about unknown symbols (src/qalc.cc, `Assumptions`).
//!
//! The reference carries a full `Assumptions` object per variable (sign and
//! number type, with `AssumptionSign` and `AssumptionType`). Only the
//! session-wide *sign* is ported, because that is what the transcripts turn
//! on: under `/assume positive` an unknown may be pulled out of a square
//! root, `sqrt(xy)` splits into `sqrt(x)*sqrt(y)`, and `ln(u^2)` becomes
//! `2 ln(u)`.

use std::cell::Cell;

/// `AssumptionSign` (includes.h), reduced to the values `/assume` sets here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sign {
    #[default]
    Unknown,
    NonZero,
    Positive,
    NonNegative,
    Negative,
    NonPositive,
}

thread_local! {
    static SIGN: Cell<Sign> = const { Cell::new(Sign::Unknown) };
}

pub fn sign() -> Sign {
    SIGN.with(|s| s.get())
}

pub fn set_sign(s: Sign) {
    SIGN.with(|c| c.set(s));
}

/// May an unknown symbol be treated as positive?
pub fn unknowns_are_positive() -> bool {
    sign() == Sign::Positive
}

/// Parse the argument of `/assume`.
pub fn parse_sign(word: &str) -> Option<Sign> {
    Some(match word.to_ascii_lowercase().as_str() {
        "positive" | "pos" => Sign::Positive,
        "non-negative" | "nonnegative" | "nonneg" => Sign::NonNegative,
        "negative" | "neg" => Sign::Negative,
        "non-positive" | "nonpositive" | "nonpos" => Sign::NonPositive,
        "non-zero" | "nonzero" => Sign::NonZero,
        "unknown" | "none" => Sign::Unknown,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::session::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.evaluate_line("/set approximation exact").ok();
        s
    }

    #[test]
    fn positivity_unlocks_roots_and_logarithms() {
        let mut s = session();
        s.evaluate_line("/assume positive").unwrap();
        assert_eq!(s.evaluate_line("sqrt(x*y)").unwrap(), "sqrt(x) * sqrt(y)");
        assert_eq!(s.evaluate_line("sqrt(x + 2*sqrt(x) + 1)").unwrap(), "sqrt(x) + 1");
        assert_eq!(s.evaluate_line("ln(x^2 + 2*x + 1)").unwrap(), "2 * ln(x + 1)");
        s.evaluate_line("/assume unknown").unwrap();
    }

    #[test]
    fn without_the_assumption_nothing_moves() {
        let mut s = session();
        crate::assumptions::set_sign(crate::assumptions::Sign::Unknown);
        assert_eq!(s.evaluate_line("sqrt(x*y)").unwrap(), "sqrt(xy)");
        assert_eq!(
            s.evaluate_line("ln(x^2 + 2*x + 1)").unwrap(),
            "ln(x^2 + 2x + 1)"
        );
    }
}
