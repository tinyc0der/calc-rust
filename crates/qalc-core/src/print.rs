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

// `MULTIPLICATION_SIGN_OPERATOR_SHORT` (the operator without spaces, used
// between unit factors) is not a variant here: units are printed as one group
// by `print_units_group`, which emits the `*` itself.

/// Print `m` using `po`.
pub fn print(m: &MathStructure, po: &PrintOptions) -> String {
    // `MathStructure::format` runs before printing; the only formatting this
    // port needs so far is the vector collapse (`[[1  2]]` → `[1  2]`,
    // `[1]` → `1`).
    let mut formatted = m.clone();
    crate::matrix::format_for_print(&mut formatted, false);
    // A result that is nothing but a unit still shows its quantity: the
    // reference prints `1 m`, never a bare `m`.
    if crate::units::is_unit_exp(&formatted) {
        return format!(
            "1 {}",
            print_units_group(std::slice::from_ref(&formatted), po)
        );
    }
    print_sub(&formatted, po, 0)
}

/// A unit factor's printed name, prefix included.
fn print_unit(
    id: crate::ids::UnitId,
    prefix: Option<crate::defs::PrefixId>,
    po: &PrintOptions,
) -> String {
    match crate::units::store() {
        Some(s) => s.unit_name(id, prefix, po.abbreviate_names),
        None => format!("unit{}", id.0),
    }
}

/// Print a run of `unit`/`unit^n` factors the way the reference does: factors
/// joined with `*`, a `/` before the negative-exponent ones (parenthesized
/// when there is more than one), and plain negative exponents when there is
/// no numerator at all (`1 s^-1`, not `1/s`).
fn print_units_group(units: &[MathStructure], po: &PrintOptions) -> String {
    type Part = (crate::ids::UnitId, Option<crate::defs::PrefixId>, i32);
    let parts: Vec<Part> = units
        .iter()
        .filter_map(crate::units::unit_exp_parts)
        .collect();
    let one = |(id, p, e): &Part| {
        let name = print_unit(*id, *p, po);
        if *e == 1 {
            name
        } else {
            format!("{name}^{e}")
        }
    };
    let pos: Vec<Part> = parts.iter().filter(|(_, _, e)| *e > 0).cloned().collect();
    let neg: Vec<Part> = parts.iter().filter(|(_, _, e)| *e < 0).cloned().collect();
    if pos.is_empty() {
        return parts.iter().map(one).collect::<Vec<_>>().join("*");
    }
    let n_str = pos.iter().map(one).collect::<Vec<_>>().join("*");
    if neg.is_empty() {
        return n_str;
    }
    let flipped: Vec<Part> = neg.iter().map(|(u, p, e)| (*u, *p, -*e)).collect();
    let d_str = flipped.iter().map(one).collect::<Vec<_>>().join("*");
    if flipped.len() > 1 {
        format!("{n_str}/({d_str})")
    } else {
        format!("{n_str}/{d_str}")
    }
}

/// A rational printed in fraction format never uses the spacious form:
/// the reference writes `x = 3/2 - sqrt(5) / 2`, with the *number* tight and
/// the division structure spaced.
fn print_number(n: &Number, po: &PrintOptions) -> String {
    use qalc_num::options::NumberFractionFormat as F;
    // `restrict_fraction_length` (PrintOptions, set by `/set fractions on`):
    // a fraction whose numerator or denominator is longer than the display
    // precision allows is shown as a decimal instead — `(3/2)^30` prints as
    // `191751.0592`, not `205891132094649/1073741824`
    // (Number.cc:13127-13161: the C++ prints numerator and denominator with
    // `is_approximate` armed and reprints the whole number as
    // `FRACTION_DECIMAL` if either came out approximate).
    if let Some(s) = print_long_fraction_as_decimal(n, po) {
        return s;
    }
    if po.spacious
        && matches!(po.number_fraction_format, F::Fractional | F::Combined)
        && n.is_rational()
        && !n.is_integer()
    {
        let mut po2 = po.clone();
        po2.spacious = false;
        return n.print(&po2);
    }
    n.print(po)
}



fn print_long_fraction_as_decimal(n: &Number, po: &PrintOptions) -> Option<String> {
    use qalc_num::options::NumberFractionFormat as F;
    if !matches!(po.number_fraction_format, F::Fractional | F::Combined) {
        return None;
    }
    if !n.is_rational() || n.is_integer() || n.is_approximate() || n.has_imaginary_part() {
        return None;
    }
    // `precexp` in Number::print: the precision plus three digits.
    let limit = (qalc_num::context::precision() + 3).max(4) as usize;
    let len = n
        .numerator()
        .numerator_digits()
        .max(n.denominator().numerator_digits());
    if len <= limit {
        return None;
    }
    let mut approx = n.clone();
    if !approx.set_to_floating_point() {
        return None;
    }
    let mut po2 = po.clone();
    po2.number_fraction_format = F::Decimal;
    Some(approx.print(&po2))
}

fn print_sub(m: &MathStructure, po: &PrintOptions, depth: usize) -> String {
    match m {
        // `to unicode` selects a base qalc-num does not print; the code
        // points are rendered here instead (Number.cc:11185).
        MathStructure::Number(n) if po.base == qalc_num::options::base::UNICODE => {
            crate::strings::unicode_digits(n, po).unwrap_or_else(|| {
                let mut po2 = po.clone();
                po2.base = 10;
                n.print(&po2)
            })
        }
        MathStructure::Number(n) => print_number(n, po),
        MathStructure::Symbolic(s) => s.clone(),
        MathStructure::Text(s) => crate::strings::quote_text(s),
        MathStructure::Variable(id) => format!("var{}", id.0),
        MathStructure::Unit { id, prefix } => print_unit(*id, *prefix, po),
        MathStructure::Undefined => "undefined".to_string(),
        MathStructure::Aborted => "aborted".to_string(),
        // The reference prints a date as a quoted ISO string.
        MathStructure::DateTime(dt) => format!("\"{}\"", dt.print(po)),
        // A conversion that survived evaluation prints as its value; the
        // target has already been folded into the print options.
        MathStructure::Conversion { value, .. } => print_sub(value, po, depth),
        MathStructure::Vector(items) => print_vector(items, po, depth),
        MathStructure::Addition(terms) => print_addition(terms, po, depth),
        MathStructure::Multiplication(factors) => print_multiplication(factors, po, depth),
        MathStructure::Power { base, exponent } => print_power(base, exponent, po, depth),
        MathStructure::Function { id, args } => {
            // `MathStructure::print` writes `abs(x)` as `|x|`
            // (MathStructure-print.cc, FUNCTION_ID_ABS).
            if id.0 == crate::builtins::id::ABS && args.len() == 1 {
                return format!("|{}|", print_sub(&args[0], po, depth + 1));
            }
            let inner: Vec<String> = args.iter().map(|a| print_sub(a, po, depth + 1)).collect();
            format!("{}({})", function_name(*id), inner.join(", "))
        }
        MathStructure::Comparison { left, op, right } => {
            let l = print_operand(left, po, depth, m);
            let r = print_operand(right, po, depth, m);
            let sign = if *op == ComparisonType::Equals
                && po.use_unicode_signs
                && is_approximate(right, 0)
            {
                // MathStructure-print.cc:4666 — an equality whose value is
                // approximate prints `≈` (`SIGN_ALMOST_EQUAL`), which is how
                // the numeric solutions of `x^(5x) = 5` are shown.
                "≈"
            } else {
                comparison_sign(*op, po)
            };
            format!("{l} {sign} {r}")
        }
        MathStructure::BitwiseAnd(v) => print_infix(v, " & ", po, depth, m),
        MathStructure::BitwiseOr(v) => print_infix(v, " | ", po, depth, m),
        MathStructure::BitwiseXor(v) => print_infix(v, " xor ", po, depth, m),
        MathStructure::BitwiseNot(x) => format!("~{}", print_operand(x, po, depth, m)),
        // `qalc` runs with `spell_out_logical_operators`, so the logical
        // operators print as words (`x = 1 or x = 2`).
        MathStructure::LogicalAnd(v) => print_infix(v, " and ", po, depth, m),
        MathStructure::LogicalOr(v) => print_infix(v, " or ", po, depth, m),
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

/// Port of the `STRUCT_VECTOR` case of `MathStructure::print`
/// (MathStructure-print.cc:5273).
///
/// In matlab-matrix mode a flat vector prints as `[1  2  3]` and a matrix as
/// `[1  2; 4  5]`. A vector whose children are vectors of *differing*
/// lengths is not a matrix and falls back to the tuple form
/// `([1  2  3], [4  5])`.
fn print_vector(items: &[MathStructure], po: &PrintOptions, depth: usize) -> String {
    let sep = if po.spacious { "  " } else { " " };
    let row_sep = if po.spacious { "; " } else { ";" };
    // Decide between the bracket ("new") style and the tuple style.
    let mut newstyle = !items.is_empty();
    let mut matrix = true;
    let mut cols = 0usize;
    for item in items {
        if item.is_vector() {
            if cols == 0 {
                cols = item.size();
                if cols == 0 {
                    newstyle = false;
                    break;
                }
            } else if cols != item.size() {
                newstyle = false;
                break;
            }
            if crate::matrix::is_matrix(item) {
                newstyle = false;
                break;
            }
        } else if cols > 1 {
            newstyle = false;
            break;
        } else {
            cols = 1;
            matrix = false;
        }
    }
    if newstyle {
        let cell = |m: &MathStructure| wrap_vector_element(m, po, depth);
        if matrix {
            let rows: Vec<String> = items
                .iter()
                .map(|r| r.children().map(cell).collect::<Vec<_>>().join(sep))
                .collect();
            return format!("[{}]", rows.join(row_sep));
        }
        let cells: Vec<String> = items.iter().map(cell).collect();
        return format!("[{}]", cells.join(sep));
    }
    let inner: Vec<String> = items.iter().map(|i| print_sub(i, po, depth + 1)).collect();
    if items.len() <= 1 {
        format!("[{}]", inner.join(", "))
    } else {
        format!("({})", inner.join(if po.spacious { ", " } else { "," }))
    }
}

/// `needsParenthesis` for a vector element (MathStructure-print.cc:3319): an
/// element is parenthesized when its printed form contains a top-level
/// space, comma or semicolon, which would otherwise read as a separator.
fn wrap_vector_element(m: &MathStructure, po: &PrintOptions, depth: usize) -> String {
    let s = print_sub(m, po, depth + 1);
    let mut brackets = 0i32;
    let mut pars = 0i32;
    for c in s.chars() {
        match c {
            '[' => brackets += 1,
            ']' => brackets = (brackets - 1).max(0),
            '(' if brackets == 0 => pars += 1,
            ')' if brackets == 0 && pars > 0 => pars -= 1,
            ' ' | ';' | ',' if brackets == 0 && pars == 0 => return format!("({s})"),
            _ => {}
        }
    }
    s
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
    // `place_units_separately`: the units of a product are printed as one
    // group at the end, with their own multiplication and division signs.
    if factors.iter().any(crate::units::is_unit_exp) {
        let others: Vec<MathStructure> = factors
            .iter()
            .filter(|f| !crate::units::is_unit_exp(f))
            .cloned()
            .collect();
        let units: Vec<MathStructure> = factors
            .iter()
            .filter(|f| crate::units::is_unit_exp(f))
            .cloned()
            .collect();
        let u_str = print_units_group(&units, po);
        if others.is_empty() {
            return format!("1 {u_str}");
        }
        let mut o_str = if others.len() == 1 {
            print_operand(
                &others[0],
                po,
                depth,
                &MathStructure::Multiplication(Vec::new()),
            )
        } else {
            print_multiplication(&others, po, depth)
        };
        let wrapped = others.len() > 1 && others.iter().any(|f| as_inverse(f).is_some());
        if wrapped {
            o_str = format!("({o_str})");
        }
        let sep = if wrapped {
            MulSign::Space
        } else {
            match others.last() {
                Some(MathStructure::Number(_))
                | Some(MathStructure::Symbolic(_))
                | Some(MathStructure::Variable(_))
                | Some(MathStructure::Vector(_)) => MulSign::Space,
                _ => MulSign::Operator,
            }
        };
        return match sep {
            MulSign::None => format!("{o_str}{u_str}"),
            MulSign::Space => format!("{o_str} {u_str}"),
            MulSign::Operator => {
                if po.spacious {
                    format!("{o_str} * {u_str}")
                } else {
                    format!("{o_str}*{u_str}")
                }
            }
        };
    }
    // `MathStructure::formatsub` turns a leading `1/d` coefficient into a
    // division when fractions are displayed as fractions:
    //   `1/2 * x * y` -> `(xy) / 2`, `1/2 * x` -> `x / 2`.
    // A coefficient with a numerator other than one stays a parenthesized
    // factor instead (`3/2 * x` -> `(3/2)x`).
    if let Some(s) = print_unit_fraction_multiplication(factors, po, depth) {
        return s;
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
    if denom.is_empty() {
        if numer.is_empty() {
            numer.push(MathStructure::Number(Number::from_i64(1)));
        }
        return join_factors(&numer, po, depth);
    }
    // `formatsub`'s multiplication split (MathStructure-print.cc:2540) walks
    // the factors into a numerator and a denominator, and a *rational*
    // coefficient contributes to both: `1.5 y / sqrt(x)` prints as
    // `(3y) / (2 * sqrt(x))`, while the same coefficient without a
    // denominator factor stays decimal (`0.4 * cosh(x^2)`).
    if let Some(MathStructure::Number(n)) = numer.first() {
        if n.is_rational() && !n.is_integer() && !n.is_approximate() {
            let mut p = n.numerator();
            let q = n.denominator();
            let negative = p.is_negative();
            if negative {
                p.negate();
            }
            if p.is_one() {
                numer.remove(0);
            } else {
                numer[0] = MathStructure::Number(p);
            }
            if negative {
                numer.insert(0, MathStructure::Number(Number::from_i64(-1)));
            }
            denom.insert(0, MathStructure::Number(q));
        }
    }
    // A leading -1 left by the split above is a sign, not a factor.
    let mut sign = "";
    if let Some(MathStructure::Number(n)) = numer.first() {
        if n.is_minus_one() && numer.len() > 1 {
            numer.remove(0);
            sign = "-";
        }
    }
    if numer.is_empty() {
        numer.push(MathStructure::Number(Number::from_i64(1)));
    }
    let n_str = join_factors(&numer, po, depth);
    let d_str = join_factors(&denom, po, depth);
    // Parenthesize a compound denominator: `1 / (2x)`. A single additive
    // factor is already parenthesized by `join_factors`, so only wrap when
    // that has not happened.
    let d_str = if denom.len() > 1 || (is_compound(&denom[0]) && !d_str.starts_with('(')) {
        format!("({d_str})")
    } else {
        d_str
    };
    // The reference always parenthesizes a numerator built from more than
    // one factor: `(2x) / y`, `(xy) / z`, `(3 * sin(x)) / (2z)`.
    let n_str = if numer.len() > 1 {
        format!("({n_str})")
    } else {
        n_str
    };
    let n_str = format!("{sign}{n_str}");
    if po.spacious {
        format!("{n_str} / {d_str}")
    } else {
        format!("{n_str}/{d_str}")
    }
}

/// A multiplication whose first factor is the rational `±1/d` (with fraction
/// display enabled) prints as `rest / d` — `1/2 * x` is `x / 2` and
/// `-1/2 * e` is `-e / 2`, matching `MathStructure::formatsub`.
///
/// When the rest of the product also contributes denominator factors the two
/// denominators merge into one: `1/2 * e^-1` is `1 / (2e)`.
fn print_unit_fraction_multiplication(
    factors: &[MathStructure],
    po: &PrintOptions,
    depth: usize,
) -> Option<String> {
    use qalc_num::options::NumberFractionFormat as F;
    if !matches!(po.number_fraction_format, F::Fractional | F::Combined) || factors.len() < 2 {
        return None;
    }
    let MathStructure::Number(n) = &factors[0] else {
        return None;
    };
    if !n.is_rational() || n.is_integer() || n.is_approximate() {
        return None;
    }
    let mut numerator = n.numerator();
    let negative = numerator.is_negative();
    if negative && !numerator.negate() {
        return None;
    }
    if !numerator.is_one() {
        return None;
    }
    let coeff_den = MathStructure::Number(n.denominator());
    let mut numer: Vec<MathStructure> = Vec::new();
    let mut denom: Vec<MathStructure> = Vec::new();
    for f in &factors[1..] {
        match as_inverse(f) {
            Some(base) => denom.push(base),
            None => numer.push(f.clone()),
        }
    }
    // The coefficient's denominator leads a plain product (`1 / (2e)`) but
    // trails one that already starts with a call (`1 / (sqrt(2) * 4)`).
    let trailing = denom.iter().any(|d| renders_as_call(d));
    if trailing {
        denom.push(coeff_den);
    } else {
        denom.insert(0, coeff_den);
    }
    if numer.is_empty() {
        numer.push(MathStructure::Number(Number::from_i64(1)));
    }
    // A multi-factor numerator keeps its parentheses (`(xy) / 2`).
    let mut n_str = join_factors(&numer, po, depth);
    if numer.len() > 1
        || needs_parenthesis(&numer[0], &MathStructure::Multiplication(Vec::new()))
    {
        n_str = format!("({n_str})");
    }
    let mut d_str = join_factors(&denom, po, depth);
    if denom.len() > 1 || (is_compound(&denom[0]) && !d_str.starts_with('(')) {
        d_str = format!("({d_str})");
    }
    let sign = if negative { "-" } else { "" };
    Some(if po.spacious {
        format!("{sign}{n_str} / {d_str}")
    } else {
        format!("{sign}{n_str}/{d_str}")
    })
}

/// True when `m` prints as a function call (`sqrt(2)`, `ln(3)`, `cbrt(e)`),
/// which the reference always separates from a neighbouring factor with an
/// explicit `*`.
fn renders_as_call(m: &MathStructure) -> bool {
    match m {
        // `abs` prints as `|x|`, a bracketed group rather than a call.
        MathStructure::Function { id, args } if id.0 == crate::builtins::id::ABS
            && args.len() == 1 =>
        {
            false
        }
        MathStructure::Function { .. } => true,
        MathStructure::Power { base, exponent } => {
            is_one_half(exponent) || (is_one_third(exponent) && represents_non_negative(base))
        }
        _ => false,
    }
}

/// `x^(1/3)` only prints as `cbrt(x)` when the base is known non-negative
/// (the reference keeps `x^(1/3)` but writes `cbrt(2)` and `cbrt(e)`).
fn is_abs_call(m: &MathStructure) -> bool {
    matches!(m, MathStructure::Function { id, args }
        if id.0 == crate::builtins::id::ABS && args.len() == 1)
}

fn is_one_third(m: &MathStructure) -> bool {
    matches!(m, MathStructure::Number(n)
        if n.is_rational() && !n.is_approximate()
            && n.numerator().is_one() && n.denominator().equals(&Number::from_i64(3), false, false))
}

fn represents_non_negative(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => !n.is_negative(),
        MathStructure::Symbolic(s) => s == "e" || s == "pi",
        _ => false,
    }
}

/// `x^-n` → `Some(x^n)`; used to recover division from the multiplication
/// form. `MathStructure::formatsub` does the same for *any* negative
/// exponent, which is why the reference prints `x^-2` as `1 / x^2`.
fn as_inverse(m: &MathStructure) -> Option<MathStructure> {
    let MathStructure::Power { base, exponent } = m else {
        return None;
    };
    let MathStructure::Number(n) = exponent.as_ref() else {
        return None;
    };
    if !n.is_negative() {
        return None;
    }
    if n.is_minus_one() {
        return Some((**base).clone());
    }
    let mut p = n.clone();
    if !p.negate() {
        return None;
    }
    Some(MathStructure::Power {
        base: base.clone(),
        exponent: Box::new(MathStructure::Number(p)),
    })
}

fn is_compound(m: &MathStructure) -> bool {
    matches!(
        m,
        MathStructure::Addition(_) | MathStructure::Multiplication(_)
    )
}

/// A rational shown as `n/d` needs parentheses as a factor (`(3/2)x`).
fn is_displayed_fraction(m: &MathStructure, po: &PrintOptions) -> bool {
    use qalc_num::options::NumberFractionFormat as F;
    matches!(po.number_fraction_format, F::Fractional | F::Combined)
        && matches!(m, MathStructure::Number(n)
            if n.is_rational() && !n.is_integer() && !n.is_approximate())
}

fn join_factors(factors: &[MathStructure], po: &PrintOptions, depth: usize) -> String {
    let mut out = String::new();
    let mut prev_par = false;
    for (i, f) in factors.iter().enumerate() {
        let par = needs_parenthesis(f, &MathStructure::Multiplication(Vec::new()))
            || (factors.len() > 1 && is_displayed_fraction(f, po));
        let text = {
            let s = print_sub(f, po, depth + 1);
            if par {
                format!("({s})")
            } else {
                s
            }
        };
        if i > 0 {
            match multiplication_sign(&factors[i - 1], f, par, prev_par, po) {
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
        prev_par = par;
    }
    out
}


/// Would this factor's printed form begin with a digit? Juxtaposing it after
/// a number would merge the two into one unreadable literal.
fn starts_with_digit(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(_) => true,
        MathStructure::Power { base, .. } => starts_with_digit(base),
        MathStructure::Multiplication(v) => v.first().is_some_and(starts_with_digit),
        _ => false,
    }
}

/// Port of `neededMultiplicationSign` for the node types this pass supports.
fn multiplication_sign(
    prev: &MathStructure,
    this: &MathStructure,
    this_par: bool,
    prev_par: bool,
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
    // `|x|` is a bracketed group: `2 |x - 1|`, `|x| |y|`.
    if is_abs_call(this) {
        return MulSign::Space;
    }
    // A factor that prints as a call is always separated by `*`
    // (`4 * sqrt(3)`, `3 * ln(2)`, `e * sqrt(e)`).
    if renders_as_call(this) {
        return MulSign::Operator;
    }
    // `(a)*b`: after a parenthesized factor only an *unknown* juxtaposes
    // (`(3/2)x`); a known constant keeps the operator (`(2/3) * pi`).
    if prev_par {
        if is_constant_symbol(this) {
            return MulSign::Operator;
        }
        return match name_len(this) {
            Some(l) if l > 1 => MulSign::Space,
            Some(_) => MulSign::None,
            None => MulSign::Operator,
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
        // A number followed by anything that starts with a digit would run
        // together: `0.5 * 2^x`, not `0.52^x`. The reference uses an
        // explicit operator for both a bare number and a power over a
        // numeric base.
        MathStructure::Number(_) if starts_with_digit(this) => MulSign::Operator,
        // `neededMultiplicationSign`, the `STRUCT_SYMBOLIC`/`STRUCT_VARIABLE`
        // branch (MathStructure-print.cc:3516): a name longer than one
        // character keeps a separator — `2 pi`, `pi * n`, `pi * x^2` — while
        // single-letter names still juxtapose (`2x`, `xy`, `ex`).
        MathStructure::Number(_) => match name_len(this) {
            Some(l) if l > 1 => MulSign::Space,
            _ => MulSign::None,
        },
        _ => match (name_len(prev), name_len(this)) {
            (Some(p), Some(t)) => {
                if p > 1 || t > 1 || prev.equals(this) {
                    MulSign::Operator
                } else {
                    MulSign::None
                }
            }
            _ => MulSign::None,
        },
    }
}

/// `pi` and `e` are `STRUCT_VARIABLE` in the C++, not unknowns.
fn is_constant_symbol(m: &MathStructure) -> bool {
    matches!(m, MathStructure::Symbolic(s) if s == "pi" || s == "e")
}

/// `namelen` (`MathStructure-print.cc:2992-3024`) for the node types that
/// carry a name: a bare symbol, or a power of one — `neededMultiplicationSign`
/// delegates a `STRUCT_POWER` to its base at `MathStructure-print.cc:3492`.
fn name_len(m: &MathStructure) -> Option<usize> {
    match m {
        MathStructure::Symbolic(s) => Some(s.chars().count()),
        MathStructure::Power { base, .. } => name_len(base),
        _ => None,
    }
}

/// `MathStructure::formatsub` rewrites `x^(1/2)` as `sqrt(x)` before
/// printing (reference: `x^(1/2)` prints as `sqrt(x)`).
fn is_one_half(m: &MathStructure) -> bool {
    matches!(m, MathStructure::Number(n)
        if n.is_rational() && !n.is_approximate()
            && n.numerator().is_one() && n.denominator().is_two())
}

fn print_power(
    base: &MathStructure,
    exponent: &MathStructure,
    po: &PrintOptions,
    depth: usize,
) -> String {
    if is_one_half(exponent) {
        return format!("sqrt({})", print_sub(base, po, depth + 1));
    }
    if is_one_third(exponent) && represents_non_negative(base) {
        return format!("cbrt({})", print_sub(base, po, depth + 1));
    }
    // `formatsub` turns a negative exponent into a division: `x^-2` prints
    // as `1 / x^2` and `e^(-3/2)` as `1 / (e * sqrt(e))`.
    if let Some(inner) = as_inverse(&MathStructure::Power {
        base: Box::new(base.clone()),
        exponent: Box::new(exponent.clone()),
    }) {
        let body = print_sub(&inner, po, depth + 1);
        let body = if needs_denominator_parens(&body) {
            format!("({body})")
        } else {
            body
        };
        return if po.spacious {
            format!("1 / {body}")
        } else {
            format!("1/{body}")
        };
    }
    // `e^(7/3)` prints as `e^2 * cbrt(e)`: a rational exponent whose
    // denominator is 2 or 3 is split into its integer part and a root
    // (Number::print does the same for a numeric base, giving `4 * cbrt(2)`).
    if let Some(s) = print_split_root_power(base, exponent, po, depth) {
        return s;
    }
    if is_displayed_fraction(exponent, po) {
        let parent = MathStructure::Power {
            base: Box::new(MathStructure::Undefined),
            exponent: Box::new(MathStructure::Undefined),
        };
        return format!(
            "{}^({})",
            print_operand(base, po, depth, &parent),
            print_sub(exponent, po, depth + 1)
        );
    }
    let parent = MathStructure::Power {
        base: Box::new(MathStructure::Undefined),
        exponent: Box::new(MathStructure::Undefined),
    };
    let b = print_operand(base, po, depth, &parent);
    let e = print_operand(exponent, po, depth, &parent);
    format!("{b}^{e}")
}

/// `base^(p/q)` with `q` in `{2, 3}` and `|p/q| > 1` → `base^i * root(base)`.
///
/// `formatsub`'s `halfexp_to_sqrt` branch (MathStructure-print.cc:2654)
/// applies the halving split to any *leaf* or function base, with no sign
/// condition: `x^(3/2)` prints as `x * sqrt(x)` and `x^(5/2)` as
/// `x^2 * sqrt(x)`. The cube-root split below it does carry a
/// non-negativity condition, which is why `x^(7/3)` stays intact and only
/// the constant `e` takes that path.
fn print_split_root_power(
    base: &MathStructure,
    exponent: &MathStructure,
    po: &PrintOptions,
    depth: usize,
) -> Option<String> {
    let is_e = matches!(base, MathStructure::Symbolic(s) if s == "e");
    let leaf_base = base.size() == 0 || matches!(base, MathStructure::Function { .. });
    let MathStructure::Number(n) = exponent else {
        return None;
    };
    if !n.is_rational() || n.is_integer() || n.is_approximate() || n.is_negative() {
        return None;
    }
    let den = n.denominator();
    let half = den.equals(&Number::from_i64(2), false, false);
    let third = den.equals(&Number::from_i64(3), false, false);
    if !(half && leaf_base) && !(third && is_e) {
        return None;
    }
    let mut whole = n.clone();
    whole.floor();
    if whole.is_zero() {
        return None;
    }
    let mut frac = n.clone();
    if !frac.subtract(&whole) {
        return None;
    }
    let int_part = if whole.is_one() {
        base.clone()
    } else {
        MathStructure::Power {
            base: Box::new(base.clone()),
            exponent: Box::new(MathStructure::Number(whole)),
        }
    };
    let root_part = MathStructure::Power {
        base: Box::new(base.clone()),
        exponent: Box::new(MathStructure::Number(frac)),
    };
    Some(join_factors(&[int_part, root_part], po, depth))
}

/// A printed denominator needs parentheses when it is a compound expression:
/// any operator or space outside brackets would otherwise re-associate
/// (`1 / e * sqrt(e)` must be `1 / (e * sqrt(e))`).
fn needs_denominator_parens(s: &str) -> bool {
    let mut pars = 0i32;
    let mut brackets = 0i32;
    for c in s.chars() {
        match c {
            '[' => brackets += 1,
            ']' => brackets = (brackets - 1).max(0),
            '(' => pars += 1,
            ')' => pars = (pars - 1).max(0),
            ' ' if pars == 0 && brackets == 0 => return true,
            _ => {}
        }
    }
    false
}

/// `MathStructure::isApproximate()` — true when any number below `m` carries
/// the approximate flag. The depth cap mirrors the printer's own recursion
/// limit and keeps a cyclic structure from looping.
fn is_approximate(m: &MathStructure, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    if let MathStructure::Number(n) = m {
        return n.is_approximate();
    }
    (0..m.size()).any(|i| m.get(i).is_some_and(|c| is_approximate(c, depth + 1)))
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

/// Names for the builtin function ids the parser produces. The arms here are
/// the ids `builtins` owns; everything else falls through to the owning
/// module's own `function_name`, so each module keeps its ids and their
/// printed names together.
fn function_name(id: crate::ids::FunctionId) -> &'static str {
    use crate::builtins::id as f;
    match id.0 {
        f::ABS => "abs",
        f::SIGNUM => "sgn",
        f::SQRT => "sqrt",
        f::CBRT => "cbrt",
        f::ROOT => "root",
        f::EXP => "exp",
        f::LN => "ln",
        f::LOG => "log",
        f::LOG2 => "log2",
        f::LOG10 => "log10",
        f::SIN => "sin",
        f::COS => "cos",
        f::TAN => "tan",
        f::ASIN => "asin",
        f::ACOS => "acos",
        f::ATAN => "atan",
        f::SINH => "sinh",
        f::COSH => "cosh",
        f::TANH => "tanh",
        f::ASINH => "asinh",
        f::ACOSH => "acosh",
        f::ATANH => "atanh",
        f::ATAN2 => "atan2",
        f::ARG => "arg",
        f::COT => "cot",
        f::ACOT => "acot",
        f::SINC => "sinc",
        f::SQ => "sq",
        f::CIS => "cis",
        f::GAMMA => "gamma",
        f::ERF => "erf",
        f::ERFC => "erfc",
        f::ZETA => "zeta",
        f::DIGAMMA => "digamma",
        f::ERFI => "erfi",
        f::BERNOULLI => "bernoulli",
        f::EXPINT => "Ei",
        f::LOGINT => "li",
        f::SININT => "Si",
        f::COSINT => "Ci",
        f::FACTORIAL => "factorial",
        f::DOUBLE_FACTORIAL => "factorial2",
        f::BINOMIAL => "binomial",
        f::MULTI_FACTORIAL => "multifactorial",
        f::ISPRIME => "isprime",
        f::NEXTPRIME => "nextprime",
        f::PREVPRIME => "prevprime",
        f::NTHPRIME => "nthprime",
        f::PRIME_PI => "primePi",
        f::PRIMES => "primes",
        f::DIVISORS => "divisors",
        f::POWMOD => "powmod",
        f::POPCOUNT => "popCount",
        f::MOD => "mod",
        f::REM => "rem",
        f::IDIV => "idiv",
        f::SHIFT_LEFT | f::SHIFT_RIGHT => "shift",
        f::UNCERTAINTY => "uncertainty",
        f::GCD => "gcd",
        f::LCM => "lcm",
        f::FLOOR => "floor",
        f::CEIL => "ceil",
        f::TRUNC => "trunc",
        f::ROUND => "round",
        f::FRAC => "frac",
        f::INT => "int",
        f::BITWISE_NOT => "bitnot",
        f::PERCENT => "percent",
        other => crate::polynomial::function_name(other)
            .or_else(|| crate::differentiate::function_name(other))
            .or_else(|| crate::limit::function_name(other))
            .or_else(|| crate::integrate::function_name(other))
            .or_else(|| crate::solve::function_name(other))
            .or_else(|| crate::explog::function_name(other))
            .or_else(|| crate::matrix::function_name(other))
            .or_else(|| crate::geometry::function_name(other))
            .or_else(|| crate::strings::function_name(other))
            .or_else(|| crate::datetime::function_name(other))
            .or_else(|| crate::stats::function_name(other))
            .or_else(|| registry_function_name(id))
            .unwrap_or("f"),
    }
}

/// The name of a function that lives only in the definition registry.
///
/// The parser builds these for calls it recognises but cannot evaluate (see
/// `parser::unimplemented_function`), and they survive evaluation untouched,
/// so printing them by name is what makes `airy(0)` come back as `airy(0)`.
fn registry_function_name(id: crate::ids::FunctionId) -> Option<&'static str> {
    let index = id.registry_index()?;
    // The store outlives the process, so its names are `'static`.
    let store = crate::units::store_if_ready()?;
    let name = store.registry().functions().get(index)?.reference_name();
    (!name.is_empty()).then_some(name)
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
