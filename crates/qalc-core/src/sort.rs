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
        // `pi` and `e` are `STRUCT_VARIABLE` in the C++, not unknowns, and
        // sort ahead of functions and of powers of unknowns (reference:
        // `pi * zeta(x)`, `pi * x^2`, `pi * ln(3)`, `pi * n`) — but after a
        // numeric radical (`sqrt(2) * pi`).
        MathStructure::Symbolic(s) if s == "pi" || s == "e" => 2,
        // Bare symbols are "unknowns" and go last (before units).
        MathStructure::Symbolic(_) => 4,
        // A power of a number is a plain value (`sqrt(2)`); a power of an
        // unknown is not.
        MathStructure::Power { base, .. } if base.is_number() => 1,
        _ => 3,
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

/// `MathStructure::hasNegativeSign` (MathStructure.cc:750). The C++
/// `STRUCT_NEGATE` is a formatting-only type this port does not have.
pub(crate) fn has_negative_sign(m: &MathStructure) -> bool {
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
    // "always place constant of definite integral last" (sortCompare,
    // MathStructure-print.cc): the `C` an indefinite integral appends is an
    // unknown variable, so the degree rules would otherwise pull it forward
    // past `ln(|x|)`.
    let is_c = |m: &MathStructure| matches!(m, MathStructure::Symbolic(s) if s == "C");
    if is_c(a) && !is_c(b) {
        return Ordering::Greater;
    }
    if is_c(b) && !is_c(a) {
        return Ordering::Less;
    }
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

// ----------------------------------------------------------------------
// Evaluation-time ordering
// ----------------------------------------------------------------------
//
// `MathStructure::evalSort` (MathStructure-calculate.cc:7656) is a *second*,
// completely separate ordering from the printing sort above: the evaluator
// keeps the terms of a sum in this order, and the printer reorders a copy on
// the way out (which is why `1 - x` prints with the constant first but is
// stored with it last). Nothing here feeds printing — the only caller is
// `crate::absolute::has_predominately_negative_sign`, which needs to know
// which term the reference considers *first*.

/// The number of leading numeric factors of a product that the addition
/// branch of `evalSortCompare` skips (`while(mstruct1[start].isNumber() &&
/// mstruct1.size() > start + 1) start++`), so that `-y` and `2y` both
/// compare as `y`.
fn numeric_prefix(v: &[MathStructure]) -> usize {
    let mut start = 0;
    while v[start].is_number() && v.len() > start + 1 {
        start += 1;
    }
    start
}

/// The ladder of `if(mstruct2.isX()) return -1; if(mstruct1.isX()) return 1;`
/// tests at MathStructure-calculate.cc:7495. The *first* type named there
/// sorts last, so these ranks run in the reverse of the C++ order.
///
/// `STRUCT_NUMBER` is not in the ladder at all. In a sum it never reaches
/// here — the test above the ladder sends it to the end — so the high rank
/// below only documents that. In a *product* it does reach here, and because
/// a number answers none of the ladder's tests the ladder always resolves in
/// its favour: whatever it is compared against is the type that fires, and
/// fires as "the other one sorts later". That is a rank below every named
/// type, which is why [`eval_type_rank_in`] moves it there.
fn eval_type_rank(m: &MathStructure) -> u8 {
    match m {
        MathStructure::DateTime(_) => 0,
        MathStructure::Variable(_) => 1,
        MathStructure::Symbolic(_) | MathStructure::Text(_) => 2,
        MathStructure::Unit { .. } => 3,
        MathStructure::Power { .. } => 4,
        MathStructure::Multiplication(_) => 5,
        MathStructure::Addition(_) => 6,
        MathStructure::Function { .. } => 7,
        MathStructure::Undefined => 8,
        MathStructure::BitwiseNot(_) => 9,
        MathStructure::BitwiseAnd(_) => 10,
        MathStructure::BitwiseXor(_) => 11,
        MathStructure::BitwiseOr(_) => 12,
        MathStructure::Comparison { .. } => 13,
        MathStructure::LogicalNot(_) => 14,
        MathStructure::LogicalXor(_) => 15,
        MathStructure::LogicalOr(_) => 16,
        MathStructure::LogicalAnd(_) => 17,
        MathStructure::Aborted => 18,
        MathStructure::Number(_) => 20,
        // `STRUCT_VECTOR` and this port's `Conversion` are not named in the
        // ladder; the C++ falls off its end with `return -1`.
        _ => 19,
    }
}

/// [`eval_type_rank`] with the parent's number rule applied.
fn eval_type_rank_in(m: &MathStructure, parent: EvalParent) -> i16 {
    if parent == EvalParent::Multiplication && m.is_number() {
        return -1;
    }
    i16::from(eval_type_rank(m))
}

/// Which n-ary structure the two operands are children of. `evalSortCompare`
/// takes the parent itself and only ever asks it `isAddition()` /
/// `isMultiplication()`, and it passes the *same* parent down every recursive
/// call, so one flag carries the whole comparison.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalParent {
    Addition,
    Multiplication,
}

/// [`eval_compare_in`] under an addition parent — the ordering
/// [`eval_order`] and [`crate::absolute::has_predominately_negative_sign`]
/// want.
fn eval_compare(a: &MathStructure, b: &MathStructure) -> std::cmp::Ordering {
    eval_compare_in(a, b, EvalParent::Addition)
}

/// `evalSortCompare` (MathStructure-calculate.cc:7415). `Less` means `a` is
/// stored before `b`.
///
/// The parent decides three things. Under an *addition* parent a product is
/// compared by what is left after its numeric coefficient (so `2x` and `-x`
/// land next to each other) and a bare number sorts last; under a
/// *multiplication* parent neither happens — products are compared as plain
/// structures, and the number sorts first, which is the position
/// [`crate::calculate`]'s `merge_addition` reads a coefficient from.
///
/// Two identity comparisons of the C++ have no faithful analogue here and are
/// approximated by this port's own ids: `STRUCT_UNIT` and `STRUCT_VARIABLE`
/// compare by object address there. So do the C++ *unknown variables* — which
/// this port spells `Symbolic` — because they all carry id 0; the reference's
/// order for `x`, `y` and `z` is therefore the order those objects happen to
/// sit in memory (empirically `y`, `z`, `x`). `Symbolic` compares by name
/// here, which is what the C++ `STRUCT_SYMBOLIC` case does for the symbols it
/// really does hold.
fn eval_compare_in(
    a: &MathStructure,
    b: &MathStructure,
    parent: EvalParent,
) -> std::cmp::Ordering {
    use qalc_num::ComparisonResult as CR;
    use std::cmp::Ordering;

    let in_mul = parent == EvalParent::Multiplication;
    let eval_compare = |x: &MathStructure, y: &MathStructure| eval_compare_in(x, y, parent);

    // Matrix factors do not commute, so a multiplication leaves them where
    // they are (`if((!m1.representsNonMatrix() && !m2.representsScalar()) ||
    // …) return 0;`).
    if in_mul && (!crate::calculate::represents::non_matrix(a) || !crate::calculate::represents::non_matrix(b)) {
        return Ordering::Equal;
    }

    // A product is compared by what is left after its numeric coefficient,
    // so `2x` and `-x` land next to each other. Addition parent only.
    if !in_mul {
        if let MathStructure::Multiplication(va) = a {
            if !va.is_empty() {
                let start = numeric_prefix(va);
                if let MathStructure::Multiplication(vb) = b {
                    if vb.is_empty() {
                        return Ordering::Less;
                    }
                    let start2 = numeric_prefix(vb);
                    let mut i = 0;
                    loop {
                        if i + start2 >= vb.len() {
                            if i + start >= va.len() {
                                if start2 == start {
                                    for i3 in 0..start {
                                        let c = eval_compare(&va[i3], &vb[i3]);
                                        if c != Ordering::Equal {
                                            return c;
                                        }
                                    }
                                    return Ordering::Equal;
                                }
                                if start2 > start {
                                    return Ordering::Less;
                                }
                            }
                            return Ordering::Greater;
                        }
                        if i + start >= va.len() {
                            return Ordering::Less;
                        }
                        let c = eval_compare(&va[i + start], &vb[i + start2]);
                        if c != Ordering::Equal {
                            return c;
                        }
                        i += 1;
                    }
                }
                let c = eval_compare(&va[start], b);
                if c != Ordering::Equal {
                    return c;
                }
                return Ordering::Greater;
            }
        }
        if let MathStructure::Multiplication(vb) = b {
            if !vb.is_empty() {
                let start2 = numeric_prefix(vb);
                let c = eval_compare(a, &vb[start2]);
                if c != Ordering::Equal {
                    return c;
                }
                return Ordering::Less;
            }
        }
    }

    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        // A number is the last term of a sum, and the first factor of a
        // product — but in a product the ladder below already gets it there,
        // because a number answers none of the ladder's type tests.
        if !in_mul {
            if b.is_number() {
                return Ordering::Less;
            }
            if a.is_number() {
                return Ordering::Greater;
            }
        }
        // `x` against `x^2` compares the bases first, then 1 against the
        // exponent. Skipped in a product when one side is the coefficient.
        if !in_mul || (!a.is_number() && !b.is_number()) {
            if let MathStructure::Power { base, exponent } = b {
                let c = eval_compare(a, base);
                if c != Ordering::Equal {
                    return c;
                }
                return eval_compare(&MathStructure::from(1), exponent);
            }
            if let MathStructure::Power { base, exponent } = a {
                let c = eval_compare(base, b);
                if c != Ordering::Equal {
                    return c;
                }
                return eval_compare(exponent, &MathStructure::from(1));
            }
        }
        return eval_type_rank_in(a, parent).cmp(&eval_type_rank_in(b, parent));
    }

    match (a, b) {
        (MathStructure::Number(x), MathStructure::Number(y)) => {
            // An inexact number sorts after an exact one; otherwise the
            // larger value comes first (`compare` describes the *argument*
            // relative to the receiver).
            if x.is_floating_point() != y.is_floating_point() {
                return if x.is_floating_point() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            match x.compare(y) {
                CR::Less | CR::EqualOrLess | CR::OverlappingLess | CR::Contains => Ordering::Less,
                CR::Greater | CR::EqualOrGreater | CR::OverlappingGreater | CR::IsContained => {
                    Ordering::Greater
                }
                _ => Ordering::Equal,
            }
        }
        (MathStructure::Symbolic(x), MathStructure::Symbolic(y)) => x.cmp(y),
        (MathStructure::Text(x), MathStructure::Text(y)) => x.cmp(y),
        (MathStructure::Unit { id: x, .. }, MathStructure::Unit { id: y, .. }) => x.0.cmp(&y.0),
        (MathStructure::Variable(x), MathStructure::Variable(y)) => x.0.cmp(&y.0),
        (
            MathStructure::Function { id: x, args: ax },
            MathStructure::Function { id: y, args: ay },
        ) => {
            if x != y {
                return x.0.cmp(&y.0);
            }
            for (i, arg) in ay.iter().enumerate() {
                let Some(mine) = ax.get(i) else {
                    return Ordering::Less;
                };
                let c = eval_compare(mine, arg);
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        }
        (
            MathStructure::Power {
                base: b1,
                exponent: e1,
            },
            MathStructure::Power {
                base: b2,
                exponent: e2,
            },
        ) => {
            let c = eval_compare(b1, b2);
            if c != Ordering::Equal {
                return c;
            }
            eval_compare(e1, e2)
        }
        _ => {
            // The C++ `default` case: the wider structure sorts first, then
            // child by child.
            match b.size().cmp(&a.size()) {
                Ordering::Equal => {}
                Ordering::Less => return Ordering::Less,
                Ordering::Greater => return Ordering::Greater,
            }
            for i in 0..a.size() {
                let (Some(x), Some(y)) = (a.get(i), b.get(i)) else {
                    break;
                };
                let c = eval_compare(x, y);
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        }
    }
}

/// The order `MathStructure::evalSort` would leave the terms of an addition
/// in, as indices into `terms`. The C++ walks the input left to right and
/// scans backwards from the tail for the first element the new one does not
/// compare less than, which keeps indistinguishable elements in input order.
pub(crate) fn eval_order(terms: &[MathStructure]) -> Vec<usize> {
    eval_order_in(terms, EvalParent::Addition)
}

/// [`eval_order`] for either kind of parent.
fn eval_order_in(terms: &[MathStructure], parent: EvalParent) -> Vec<usize> {
    let mut sorted: Vec<usize> = Vec::with_capacity(terms.len());
    for (i, term) in terms.iter().enumerate() {
        let pos = sorted
            .iter()
            .rposition(|&j| eval_compare_in(term, &terms[j], parent) != std::cmp::Ordering::Less)
            .map_or(0, |p| p + 1);
        sorted.insert(pos, i);
    }
    sorted
}

/// `MathStructure::evalSort(recursive = false)`
/// (MathStructure-calculate.cc:7656), the *evaluation* ordering — not the
/// print ordering [`sort`] applies.
///
/// This is the `else` arm of the C++ `MERGE_ALL2` macro
/// (MathStructure-calculate.cc:5311): every `calculatesub` of an addition or
/// a multiplication leaves its children in `evalSort` order, so that the next
/// pairwise merge compares canonical spellings. Without it two sums with the
/// same terms in different order — `x - y` from the source and `-y + x` from
/// `abs`'s argument negation — are structurally distinct and never cancel.
///
/// Products need it just as much: `merge_addition` compares two products
/// factor by factor from a fixed offset, so `x·(1-x²)^-½` and
/// `-1·(1-x²)^-½·x` — the two halves of `d/dx ∫acos(x) dx` — only cancel once
/// their factors are in one order.
pub(crate) fn eval_sort(m: &mut MathStructure) {
    let (terms, parent) = match m {
        MathStructure::Addition(v) => (v, EvalParent::Addition),
        MathStructure::Multiplication(v) => (v, EvalParent::Multiplication),
        _ => return,
    };
    if terms.len() < 2 {
        return;
    }
    // `if(m_type == STRUCT_ADDITION && containsType(STRUCT_DATETIME, false,
    // true, false) > 0) return;` — a sum containing a date keeps its written
    // order, because `date - date` is read positionally.
    if parent == EvalParent::Addition && terms.iter().any(contains_date) {
        return;
    }
    let order = eval_order_in(terms, parent);
    if order.iter().enumerate().all(|(i, &j)| i == j) {
        return;
    }
    let mut taken: Vec<Option<MathStructure>> = terms.drain(..).map(Some).collect();
    *terms = order
        .into_iter()
        .map(|i| taken[i].take().expect("each index appears once"))
        .collect();
}

/// `containsType(STRUCT_DATETIME, false, true, false)`, widened to the text
/// values this port still spells a date with.
///
/// The C++ parser produces a `STRUCT_DATETIME` for `"2020-11-05"` before
/// `calculatesub` ever sees it, so its bare type test is enough. Here
/// `datetime::apply` runs *after* the merge loop, so at sort time a date is
/// still a `Text` leaf — and `datetime::merge_addition` reads `date - date`
/// positionally, taking the first date term as the later one. Reordering
/// `"2020-11-05" - "2020-10-05"` would turn its answer into a negative
/// duration, or lose it entirely.
fn contains_date(m: &MathStructure) -> bool {
    match m {
        MathStructure::DateTime(_) => true,
        MathStructure::Text(t) => crate::datetime::parse_date(t).is_some(),
        _ => (0..m.size()).filter_map(|i| m.get(i)).any(contains_date),
    }
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
                .then_with(|| sums_in_a_product(a, b))
        });
    }
}

/// Two *sums* multiplied together, ordered as the C++ print `sortCompare`
/// orders them: its `default:` arm (MathStructure-print.cc:539) walks the two
/// structures child by child, and its `STRUCT_NUMBER` arm sorts numbers
/// *increasing* under a multiplication parent (`bool inc_order =
/// parent.isMultiplication()`) where a sum sorts them decreasing. That is why
/// `factor x^2+3x+2` prints `(x + 1)(x + 2)`.
///
/// This only became visible once [`eval_sort`] started ordering products:
/// `evalSortCompare` has no `inc_order`, so it leaves the factors as
/// `(x + 2)(x + 1)` and the print pass is what turns them back round. Before,
/// nothing reordered them and the input order happened to be right.
///
/// Restricted to a pair of additions on purpose — every other pair of factors
/// is already decided by the rank, unit and name keys above, and widening the
/// child walk to them would re-order products this port prints correctly
/// today.
fn sums_in_a_product(a: &MathStructure, b: &MathStructure) -> std::cmp::Ordering {
    let (MathStructure::Addition(va), MathStructure::Addition(vb)) = (a, b) else {
        return std::cmp::Ordering::Equal;
    };
    for (x, y) in va.iter().zip(vb.iter()) {
        let c = match (x, y) {
            // `inc_order`: the smaller constant term names the first factor.
            (MathStructure::Number(p), MathStructure::Number(q)) => match q.compare(p) {
                qalc_num::ComparisonResult::Less => std::cmp::Ordering::Less,
                qalc_num::ComparisonResult::Greater => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            },
            (MathStructure::Symbolic(p), MathStructure::Symbolic(q)) => p.cmp(q),
            _ => std::cmp::Ordering::Equal,
        };
        if c != std::cmp::Ordering::Equal {
            return c;
        }
    }
    // "place MathStructure with less children first".
    va.len().cmp(&vb.len())
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

    // The evaluation-time order never reaches printed output, so the cases
    // below drive `eval_order` directly. Each was read out of the reference's
    // own `evalSort` by dumping `Calculator::calculate`'s result tree from a
    // program linked against libqalculate; the expression that produced it is
    // named in each test. `p`/`q`/`r` there are genuine `STRUCT_SYMBOLIC`,
    // which is what this port's `Symbolic` is — the reference's `x`/`y`/`z`
    // are unknown *variables*, ordered by object address (see
    // `eval_compare`), and are the one case this port cannot reproduce.

    use super::eval_order;
    use crate::structure::MathStructure;

    fn neg(m: MathStructure) -> MathStructure {
        crate::absolute::negate_struct(&m)
    }

    fn sym(s: &str) -> MathStructure {
        MathStructure::symbolic(s)
    }

    /// `p - 1` is stored as `[p, -1]` and `1 - p` as `[-p, 1]`: a number is
    /// the last term of a sum, whichever side it was written on.
    #[test]
    fn a_number_is_the_last_term() {
        let terms = vec![MathStructure::from(1), neg(sym("p"))];
        assert_eq!(eval_order(&terms), vec![1, 0]);
        let terms = vec![sym("p"), MathStructure::from(-1)];
        assert_eq!(eval_order(&terms), vec![0, 1]);
    }

    /// `p*2 - q` is stored as `[2p, -q]` and `q - p*2` as `[-2p, q]`: the
    /// numeric coefficient of a term is skipped, so both order `p` before
    /// `q` by name.
    #[test]
    fn a_coefficient_does_not_change_a_terms_place() {
        let two_p = MathStructure::Multiplication(vec![MathStructure::from(2), sym("p")]);
        let terms = vec![two_p.clone(), neg(sym("q"))];
        assert_eq!(eval_order(&terms), vec![0, 1]);
        let terms = vec![neg(sym("q")), two_p];
        assert_eq!(eval_order(&terms), vec![1, 0]);
    }

    /// `p^2 + p` and `p + p^2` are both stored as `[p^2, p]`: the C++
    /// compares the lone structure against the power's base and then 1
    /// against its exponent, and the larger exponent comes first.
    #[test]
    fn a_power_sorts_by_base_then_exponent() {
        let p2 = MathStructure::Power {
            base: Box::new(sym("p")),
            exponent: Box::new(MathStructure::from(2)),
        };
        let terms = vec![p2.clone(), sym("p")];
        assert_eq!(eval_order(&terms), vec![0, 1]);
        let terms = vec![sym("p"), p2];
        assert_eq!(eval_order(&terms), vec![1, 0]);
    }

    /// `p + q - r` is stored in the order it was written.
    #[test]
    fn symbols_keep_their_alphabetical_order() {
        let terms = vec![sym("p"), sym("q"), neg(sym("r"))];
        assert_eq!(eval_order(&terms), vec![0, 1, 2]);
        let terms = vec![neg(sym("r")), sym("q"), sym("p")];
        assert_eq!(eval_order(&terms), vec![2, 1, 0]);
    }
}
