//! Expression printing — port of `MathStructure::print` and its helpers
//! `needsParenthesis` / `neededMultiplicationSign` (MathStructure-print.cc).
//!
//! Output is validated against the reference binary in non-Unicode mode
//! (`qalc +u8`, which is what `--test-file` uses):
//!
//! | expression   | output          |
//! |--------------|-----------------|
//! | `x+y`        | `x + y`         |
//! | `x*y`        | `xy`            |
//! | `x^2*y^3`    | `x^2 * y^3`     |
//! | `x-y`        | `x - y`         |
//! | `x/y`        | `x / y`         |
//! | `(x+1)/(y+2)`| `(x + 1) / (y + 2)` |
//! | `[1,2,3]`    | `[1  2  3]`     |
//!
//! TODO(port): unit placement (`place_units_separately`), HTML/LaTeX tag
//! output, colorization, `preserve_format` round-tripping, prefix selection.

use crate::structure::{ComparisonType, MathStructure};
use qalc_num::{Number, PrintOptions};

/// What separates two adjacent factors — `MULTIPLICATION_SIGN_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MulSign {
    /// Juxtaposition: `xy`, `2x`.
    None,
    /// A plain space: `(a) u`.
    Space,
    /// The multiplication operator: ` * `.
    Operator,
}

/// Print `m` using `po`.
pub fn print(m: &MathStructure, po: &PrintOptions) -> String {
    print_sub(m, po, 0)
}

fn print_sub(m: &MathStructure, po: &PrintOptions, depth: usize) -> String {
    match m {
        MathStructure::Number(n) => n.print(po),
        MathStructure::Symbolic(s) => s.clone(),
        MathStructure::Variable(id) => format!("var{}", id.0),
        MathStructure::Unit(id) => format!("unit{}", id.0),
        MathStructure::Undefined => "undefined".to_string(),
        MathStructure::Aborted => "aborted".to_string(),
        MathStructure::DateTime(dt) => format!("{dt:?}"),
        // A conversion that survived evaluation prints as its value; the
        // target has already been folded into the print options.
        MathStructure::Conversion { value, .. } => print_sub(value, po, depth),
        MathStructure::Vector(items) => print_vector(items, po, depth),
        MathStructure::Addition(terms) => print_addition(terms, po, depth),
        MathStructure::Multiplication(factors) => print_multiplication(factors, po, depth),
        MathStructure::Power { base, exponent } => print_power(base, exponent, po, depth),
        MathStructure::Function { id, args } => {
            let inner: Vec<String> = args.iter().map(|a| print_sub(a, po, depth + 1)).collect();
            format!("{}({})", function_name(*id), inner.join(", "))
        }
        MathStructure::Comparison { left, op, right } => {
            let l = print_operand(left, po, depth, m);
            let r = print_operand(right, po, depth, m);
            format!("{l} {} {r}", comparison_sign(*op, po))
        }
        MathStructure::BitwiseAnd(v) => print_infix(v, " & ", po, depth, m),
        MathStructure::BitwiseOr(v) => print_infix(v, " | ", po, depth, m),
        MathStructure::BitwiseXor(v) => print_infix(v, " xor ", po, depth, m),
        MathStructure::BitwiseNot(x) => format!("~{}", print_operand(x, po, depth, m)),
        MathStructure::LogicalAnd(v) => print_infix(v, " && ", po, depth, m),
        MathStructure::LogicalOr(v) => print_infix(v, " || ", po, depth, m),
        MathStructure::LogicalXor(v) => print_infix(v, " xor ", po, depth, m),
        MathStructure::LogicalNot(x) => format!("!{}", print_operand(x, po, depth, m)),
    }
}

fn print_infix(
    items: &[MathStructure],
    sep: &str,
    po: &PrintOptions,
    depth: usize,
    parent: &MathStructure,
) -> String {
    items
        .iter()
        .map(|i| print_operand(i, po, depth, parent))
        .collect::<Vec<_>>()
        .join(sep)
}

/// Print `m` as an operand of `parent`, adding parentheses if needed.
fn print_operand(
    m: &MathStructure,
    po: &PrintOptions,
    depth: usize,
    parent: &MathStructure,
) -> String {
    let s = print_sub(m, po, depth + 1);
    if needs_parenthesis(m, parent) {
        format!("({s})")
    } else {
        s
    }
}

/// Port of `MathStructure::needsParenthesis`, reduced to the precedence
/// relation between a child and its parent node type.
fn needs_parenthesis(child: &MathStructure, parent: &MathStructure) -> bool {
    use MathStructure::*;
    match parent {
        Addition(_) => matches!(child, Comparison { .. } | LogicalAnd(_) | LogicalOr(_)),
        Multiplication(_) => matches!(
            child,
            Addition(_)
                | Comparison { .. }
                | BitwiseAnd(_)
                | BitwiseOr(_)
                | BitwiseXor(_)
                | LogicalAnd(_)
                | LogicalOr(_)
                | LogicalXor(_)
        ),
        Power { .. } => matches!(
            child,
            Addition(_)
                | Multiplication(_)
                | Power { .. }
                | Comparison { .. }
                | BitwiseAnd(_)
                | BitwiseOr(_)
                | BitwiseXor(_)
                | LogicalAnd(_)
                | LogicalOr(_)
                | LogicalXor(_)
        ) || is_negative_number(child),
        Comparison { .. } => matches!(child, Comparison { .. }),
        BitwiseAnd(_) | BitwiseOr(_) | BitwiseXor(_) | BitwiseNot(_) => {
            matches!(child, Addition(_) | Comparison { .. } | LogicalAnd(_) | LogicalOr(_))
        }
        LogicalAnd(_) | LogicalOr(_) | LogicalXor(_) | LogicalNot(_) => {
            matches!(child, LogicalAnd(_) | LogicalOr(_) | LogicalXor(_))
        }
        _ => false,
    }
}

fn is_negative_number(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => n.is_negative(),
        _ => false,
    }
}

fn print_vector(items: &[MathStructure], po: &PrintOptions, depth: usize) -> String {
    // The reference prints `[1  2  3]` — two spaces between elements.
    let inner: Vec<String> = items.iter().map(|i| print_sub(i, po, depth + 1)).collect();
    format!("[{}]", inner.join("  "))
}

/// Addition, rendering negative terms as subtraction (`x - y`, not
/// `x + -1 * y`).
fn print_addition(terms: &[MathStructure], po: &PrintOptions, depth: usize) -> String {
    let mut out = String::new();
    for (i, term) in terms.iter().enumerate() {
        let (negated, body) = split_negation(term);
        let text = {
            let s = print_sub(&body, po, depth + 1);
            if needs_parenthesis(&body, &MathStructure::Addition(Vec::new())) {
                format!("({s})")
            } else {
                s
            }
        };
        if i == 0 {
            if negated {
                out.push('-');
            }
            out.push_str(&text);
        } else {
            let sign = if negated { '-' } else { '+' };
            if po.spacious {
                out.push(' ');
                out.push(sign);
                out.push(' ');
            } else {
                out.push(sign);
            }
            out.push_str(&text);
        }
    }
    out
}

/// If `m` is `Multiplication[-1, rest…]` or a negative number, return the
/// positive form plus a flag. This is how `x - y` is recovered from the
/// parser's `Addition[x, Multiplication[-1, y]]`.
fn split_negation(m: &MathStructure) -> (bool, MathStructure) {
    match m {
        MathStructure::Number(n) if n.is_negative() => {
            let mut p = n.clone();
            p.negate();
            (true, MathStructure::Number(p))
        }
        MathStructure::Multiplication(factors) if !factors.is_empty() => {
            let first_is_minus_one = matches!(&factors[0], MathStructure::Number(n) if n.is_minus_one());
            if first_is_minus_one {
                let rest: Vec<MathStructure> = factors[1..].to_vec();
                let inner = if rest.len() == 1 {
                    rest.into_iter().next().unwrap()
                } else {
                    MathStructure::Multiplication(rest)
                };
                (true, inner)
            } else if let MathStructure::Number(n) = &factors[0] {
                if n.is_negative() {
                    let mut p = n.clone();
                    p.negate();
                    let mut rest = factors.clone();
                    rest[0] = MathStructure::Number(p);
                    (true, MathStructure::Multiplication(rest))
                } else {
                    (false, m.clone())
                }
            } else {
                (false, m.clone())
            }
        }
        _ => (false, m.clone()),
    }
}

/// Multiplication, splitting inverse factors (`x^-1`) into a division and
/// choosing juxtaposition vs. an explicit `*` per
/// `neededMultiplicationSign`.
fn print_multiplication(factors: &[MathStructure], po: &PrintOptions, depth: usize) -> String {
    // A leading -1 is a negation, not a factor: `-x`, not `-1x`.
    if factors.len() >= 2 {
        if let MathStructure::Number(n) = &factors[0] {
            if n.is_minus_one() {
                let rest: Vec<MathStructure> = factors[1..].to_vec();
                let inner = if rest.len() == 1 {
                    print_operand(&rest[0], po, depth, &MathStructure::Multiplication(Vec::new()))
                } else {
                    print_multiplication(&rest, po, depth)
                };
                return format!("-{inner}");
            }
        }
    }
    // Partition into numerator and denominator factors.
    let mut numer: Vec<MathStructure> = Vec::new();
    let mut denom: Vec<MathStructure> = Vec::new();
    for f in factors {
        match as_inverse(f) {
            Some(base) => denom.push(base),
            None => numer.push(f.clone()),
        }
    }
    if numer.is_empty() {
        numer.push(MathStructure::Number(Number::from_i64(1)));
    }
    let n_str = join_factors(&numer, po, depth);
    if denom.is_empty() {
        return n_str;
    }
    let d_str = join_factors(&denom, po, depth);
    // Parenthesize a compound denominator: `1 / (2x)`. A single additive
    // factor is already parenthesized by `join_factors`, so only wrap when
    // that has not happened.
    let d_str = if denom.len() > 1 || (is_compound(&denom[0]) && !d_str.starts_with('(')) {
        format!("({d_str})")
    } else {
        d_str
    };
    let n_str = if numer.len() > 1 && numer.iter().any(is_additive) {
        format!("({n_str})")
    } else {
        n_str
    };
    if po.spacious {
        format!("{n_str} / {d_str}")
    } else {
        format!("{n_str}/{d_str}")
    }
}

/// `x^-1` → `Some(x)`; used to recover division from the multiplication form.
fn as_inverse(m: &MathStructure) -> Option<MathStructure> {
    if let MathStructure::Power { base, exponent } = m {
        if let MathStructure::Number(n) = exponent.as_ref() {
            if n.is_minus_one() {
                return Some((**base).clone());
            }
        }
    }
    None
}

fn is_compound(m: &MathStructure) -> bool {
    matches!(
        m,
        MathStructure::Addition(_) | MathStructure::Multiplication(_)
    )
}

fn is_additive(m: &MathStructure) -> bool {
    matches!(m, MathStructure::Addition(_))
}

fn join_factors(factors: &[MathStructure], po: &PrintOptions, depth: usize) -> String {
    let mut out = String::new();
    for (i, f) in factors.iter().enumerate() {
        let par = needs_parenthesis(f, &MathStructure::Multiplication(Vec::new()));
        let text = {
            let s = print_sub(f, po, depth + 1);
            if par {
                format!("({s})")
            } else {
                s
            }
        };
        if i > 0 {
            match multiplication_sign(&factors[i - 1], f, par, po) {
                MulSign::None => {}
                MulSign::Space => out.push(' '),
                MulSign::Operator => {
                    if po.spacious {
                        out.push_str(" * ");
                    } else {
                        out.push('*');
                    }
                }
            }
        }
        out.push_str(&text);
    }
    out
}

/// Port of `neededMultiplicationSign` for the node types this pass supports.
fn multiplication_sign(
    prev: &MathStructure,
    this: &MathStructure,
    this_par: bool,
    po: &PrintOptions,
) -> MulSign {
    if !po.short_multiplication {
        return MulSign::Operator;
    }
    // Bases other than 2..=10 may use letter digits, which would make
    // juxtaposition ambiguous.
    if !(2..=10).contains(&po.base) {
        return MulSign::Operator;
    }
    // `a^b*(c)` = `a^b (c)`; `a*(b)` = `a(b)`.
    if this_par {
        return if matches!(prev, MathStructure::Power { .. }) {
            MulSign::Space
        } else {
            MulSign::None
        };
    }
    match prev {
        // `a^b*c` needs the operator when the power prints flat.
        MathStructure::Power { .. } => MulSign::Operator,
        MathStructure::Addition(_)
        | MathStructure::Comparison { .. }
        | MathStructure::BitwiseAnd(_)
        | MathStructure::BitwiseOr(_)
        | MathStructure::BitwiseXor(_)
        | MathStructure::BitwiseNot(_)
        | MathStructure::LogicalAnd(_)
        | MathStructure::LogicalOr(_)
        | MathStructure::LogicalXor(_)
        | MathStructure::LogicalNot(_)
        | MathStructure::Function { .. } => MulSign::Operator,
        // A number followed by another number would run together.
        MathStructure::Number(_) if matches!(this, MathStructure::Number(_)) => MulSign::Operator,
        _ => MulSign::None,
    }
}

fn print_power(
    base: &MathStructure,
    exponent: &MathStructure,
    po: &PrintOptions,
    depth: usize,
) -> String {
    let parent = MathStructure::Power {
        base: Box::new(MathStructure::Undefined),
        exponent: Box::new(MathStructure::Undefined),
    };
    let b = print_operand(base, po, depth, &parent);
    let e = print_operand(exponent, po, depth, &parent);
    format!("{b}^{e}")
}

fn comparison_sign(op: ComparisonType, po: &PrintOptions) -> &'static str {
    match op {
        ComparisonType::Equals => "=",
        ComparisonType::NotEquals => {
            if po.use_unicode_signs {
                "≠"
            } else {
                "!="
            }
        }
        ComparisonType::Less => "<",
        ComparisonType::Greater => ">",
        ComparisonType::EqualsLess => {
            if po.use_unicode_signs {
                "≤"
            } else {
                "<="
            }
        }
        ComparisonType::EqualsGreater => {
            if po.use_unicode_signs {
                "≥"
            } else {
                ">="
            }
        }
    }
}

/// Names for the builtin function ids the parser produces. The full table
/// arrives with the function registry.
fn function_name(id: crate::ids::FunctionId) -> &'static str {
    match id.0 {
        1400 => "abs",
        1500 => "factorial",
        1501 => "factorial2",
        1700 => "mod",
        1701 => "rem",
        1702 => "idiv",
        1703 => "shift",
        1704 => "shift",
        1705 => "uncertainty",
        _ => "f",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use qalc_num::ParseOptions;

    /// Print options matching `qalc +u8` (what `--test-file` uses).
    fn batch_po() -> PrintOptions {
        let mut po = PrintOptions::default();
        po.use_unicode_signs = false;
        po.spacious = true;
        po.short_multiplication = true;
        po
    }

    fn roundtrip(expr: &str) -> String {
        let m = parse(expr, &ParseOptions::default()).expect("parse");
        print(&m, &batch_po())
    }

    #[test]
    fn addition_is_spacious() {
        assert_eq!(roundtrip("x+y"), "x + y");
    }

    #[test]
    fn subtraction_renders_as_minus() {
        // Parser builds Addition[x, Multiplication[-1, y]]; the printer must
        // recover `x - y` (reference output).
        assert_eq!(roundtrip("x-y"), "x - y");
    }

    #[test]
    fn simple_product_is_juxtaposed() {
        assert_eq!(roundtrip("x*y"), "xy");
        assert_eq!(roundtrip("2x"), "2x");
    }

    #[test]
    fn powers_force_explicit_operator() {
        // Reference: `x^2 * y^3`, not `x^2y^3`.
        assert_eq!(roundtrip("x^2*y^3"), "x^2 * y^3");
    }

    #[test]
    fn power_formatting() {
        assert_eq!(roundtrip("x^2"), "x^2");
    }

    #[test]
    fn division_is_spacious() {
        assert_eq!(roundtrip("x/y"), "x / y");
    }

    #[test]
    fn compound_denominator_is_parenthesized() {
        // Reference: `1 / (2x)`.
        assert_eq!(roundtrip("1/(2x)"), "1 / (2x)");
    }

    #[test]
    fn parenthesized_sums_in_division() {
        assert_eq!(roundtrip("(x+1)/(y+2)"), "(x + 1) / (y + 2)");
    }

    #[test]
    fn negation() {
        assert_eq!(roundtrip("-x"), "-x");
    }

    #[test]
    fn vectors_use_double_space() {
        assert_eq!(roundtrip("[1,2,3]"), "[1  2  3]");
    }

    #[test]
    fn sums_inside_products_are_parenthesized() {
        let s = roundtrip("2*(x+1)");
        assert_eq!(s, "2(x + 1)");
    }

    #[test]
    fn comparisons() {
        assert_eq!(roundtrip("x=5"), "x = 5");
        assert_eq!(roundtrip("x<=5"), "x <= 5");
    }

    #[test]
    fn numbers_use_number_print() {
        assert_eq!(roundtrip("42"), "42");
        // A parsed `1/2` is still a division tree; only evaluation turns it
        // into the number 0.5.
        assert_eq!(roundtrip("1/2"), "1 / 2");
        let half = MathStructure::Number(Number::from_ints(1, 2, 0));
        assert_eq!(print(&half, &batch_po()), "0.5");
    }

    #[test]
    fn non_spacious_mode() {
        let mut po = batch_po();
        po.spacious = false;
        let m = parse("x+y", &ParseOptions::default()).unwrap();
        assert_eq!(print(&m, &po), "x+y");
    }
}
