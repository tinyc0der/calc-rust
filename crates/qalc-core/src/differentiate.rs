//! Symbolic differentiation — the part of `MathStructure::differentiate`
//! (`BuiltinFunctions-calculus.cc`, `DeriveFunction`) the limit engine needs.
//!
//! The C++ `diff()` walks the structure applying the sum/product/quotient and
//! chain rules and consults a per-function derivative table. This port does
//! the same for the node types and builtins the transcripts exercise; a shape
//! with no rule returns `None` rather than an unevaluated `diff()` call, which
//! is what lets [`crate::limit`] fall back instead of producing nonsense.
//!
//! The result is an *unevaluated* structure: callers run it through the merge
//! engine themselves (the limit engine already has an evaluation helper with
//! the right options).

use crate::builtins::id as bid;
use crate::ids::FunctionId;
use crate::structure::MathStructure;
use qalc_num::Number;

pub mod id {
    /// `FUNCTION_ID_DIFFERENTIATE` (BuiltinFunctions.h:352).
    pub const DIFFERENTIATE: u32 = 1800;
}

fn num(i: i64) -> MathStructure {
    MathStructure::from(i)
}

fn ratio(n: i64, d: i64) -> MathStructure {
    MathStructure::Number(Number::from_ints(n, d, 0))
}

fn func(id: u32, args: Vec<MathStructure>) -> MathStructure {
    MathStructure::Function {
        id: FunctionId(id),
        args,
    }
}

fn mul(v: Vec<MathStructure>) -> MathStructure {
    MathStructure::Multiplication(v)
}

fn add(v: Vec<MathStructure>) -> MathStructure {
    MathStructure::Addition(v)
}

fn pow(b: MathStructure, e: MathStructure) -> MathStructure {
    MathStructure::Power {
        base: Box::new(b),
        exponent: Box::new(e),
    }
}

fn neg(m: MathStructure) -> MathStructure {
    mul(vec![num(-1), m])
}

fn inv(m: MathStructure) -> MathStructure {
    pow(m, num(-1))
}

/// True when `x` occurs anywhere in `m` (C++ `MathStructure::contains`).
pub fn contains(m: &MathStructure, x: &MathStructure) -> bool {
    if m.equals(x) {
        return true;
    }
    (0..m.size()).any(|i| m.get(i).is_some_and(|c| contains(c, x)))
}

/// Guard against pathological nesting.
const MAX_DEPTH: usize = 40;

/// `d(m)/d(x)`, or `None` when no rule applies.
pub fn differentiate(m: &MathStructure, x: &MathStructure) -> Option<MathStructure> {
    diff_depth(m, x, 0)
}

fn diff_depth(m: &MathStructure, x: &MathStructure, depth: usize) -> Option<MathStructure> {
    if depth > MAX_DEPTH {
        return None;
    }
    if !contains(m, x) {
        return Some(num(0));
    }
    if m.equals(x) {
        return Some(num(1));
    }
    match m {
        MathStructure::Addition(terms) => {
            let mut out = Vec::with_capacity(terms.len());
            for t in terms {
                out.push(diff_depth(t, x, depth + 1)?);
            }
            Some(add(out))
        }
        MathStructure::Multiplication(factors) => {
            // Product rule: sum_i f_i' * prod_{j != i} f_j.
            let mut out = Vec::with_capacity(factors.len());
            for (i, f) in factors.iter().enumerate() {
                if !contains(f, x) {
                    continue;
                }
                let d = diff_depth(f, x, depth + 1)?;
                let mut term = vec![d];
                for (j, g) in factors.iter().enumerate() {
                    if i != j {
                        term.push(g.clone());
                    }
                }
                out.push(mul(term));
            }
            match out.len() {
                0 => Some(num(0)),
                1 => Some(out.into_iter().next().expect("len 1")),
                _ => Some(add(out)),
            }
        }
        MathStructure::Power { base, exponent } => {
            diff_power(base, exponent, x, depth)
        }
        MathStructure::Function { id, args } => diff_function(id.0, args, x, depth),
        _ => None,
    }
}

fn diff_power(
    base: &MathStructure,
    exponent: &MathStructure,
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    let b_has = contains(base, x);
    let e_has = contains(exponent, x);
    if b_has && !e_has {
        // (b^e)' = e * b^(e-1) * b'
        let db = diff_depth(base, x, depth + 1)?;
        let e_minus_1 = add(vec![exponent.clone(), num(-1)]);
        return Some(mul(vec![
            exponent.clone(),
            pow(base.clone(), e_minus_1),
            db,
        ]));
    }
    if !b_has && e_has {
        // (b^e)' = b^e * ln(b) * e'. `e` is a plain symbol in this port, so
        // `ln(e)` would survive as an opaque call; drop it explicitly.
        let de = diff_depth(exponent, x, depth + 1)?;
        let is_e = matches!(base, MathStructure::Symbolic(s) if s == "e");
        let mut factors = vec![pow(base.clone(), exponent.clone())];
        if !is_e {
            factors.push(func(bid::LN, vec![base.clone()]));
        }
        factors.push(de);
        return Some(mul(factors));
    }
    // General: (b^e)' = b^e * (e' ln b + e b'/b)
    let db = diff_depth(base, x, depth + 1)?;
    let de = diff_depth(exponent, x, depth + 1)?;
    Some(mul(vec![
        pow(base.clone(), exponent.clone()),
        add(vec![
            mul(vec![de, func(bid::LN, vec![base.clone()])]),
            mul(vec![exponent.clone(), db, inv(base.clone())]),
        ]),
    ]))
}

/// `m` with every `abs()` call replaced by its argument, or `None` when there
/// was no `abs` to strip.
///
/// Used only for `ln` arguments: `ln|v|` and `ln(v)` have the same derivative
/// wherever both are defined (they differ by a constant on any region where
/// `arg v` is fixed), and the unwrapped form is the one that stays correct off
/// the real line. See the comment at the call site.
fn strip_abs(m: &MathStructure) -> Option<MathStructure> {
    if let MathStructure::Function { id, args } = m {
        if id.0 == bid::ABS && args.len() == 1 {
            let inner = &args[0];
            return Some(strip_abs(inner).unwrap_or_else(|| inner.clone()));
        }
    }
    let mut out = m.clone();
    let mut found = false;
    for i in 0..out.size() {
        let Some(child) = out.get_mut(i) else { continue };
        if let Some(rebuilt) = strip_abs(child) {
            *child = rebuilt;
            found = true;
        }
    }
    found.then_some(out)
}


fn diff_function(
    id: u32,
    args: &[MathStructure],
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    // `root(u, n)` and `log(u, b)` are the only two-argument cases; both
    // require the second argument to be constant.
    if args.len() == 2 {
        if contains(&args[1], x) {
            return None;
        }
    } else if args.len() != 1 {
        return None;
    }
    let u = &args[0];

    // `d/dx ln|v| = v'/v`.
    //
    // Taking this as a composition instead — `d/du ln(u) = 1/u` chained with
    // `d/dv |v| = v/|v|` — multiplies out to `v' * v / |v|^2`, which is
    // `v' / conj(v)`. For a real `v` that is the same number, but for a
    // complex one it conjugates the result: the `int dx/(ax+b) = ln|ax+b|/a`
    // rule's answer then differentiated back to the conjugate of the
    // integrand whenever `ax + b` went complex, flipping the sign of the real
    // part while leaving the imaginary part right.
    //
    // Differentiating `ln|v|` as `ln(v)` is what the antiderivative means: the
    // two differ by a constant (`i*pi`) on any region where `arg v` is fixed,
    // so the derivative is the same, and unlike the composed form it is
    // correct on both domains.
    // The `abs` may also sit below a product or quotient, which is the shape
    // the partial-fraction rules emit: `ln(|a| / |b|)`. Stripping every `abs`
    // in the argument handles those the same way, since `|a|/|b|` and `a/b`
    // differ only by a sign.
    if id == bid::LN && args.len() == 1 {
        if let Some(stripped) = strip_abs(u) {
            let dv = diff_depth(&stripped, x, depth + 1)?;
            return Some(mul(vec![dv, inv(stripped)]));
        }
    }

    let du = diff_depth(u, x, depth + 1)?;
    // The chain-rule factor `f'(u)`; the caller multiplies by `u'`.
    let outer = match id {
        // d/du sqrt(u) = 1 / (2 u^(1/2))
        bid::SQRT => inv(mul(vec![num(2), pow(u.clone(), ratio(1, 2))])),
        // d/du cbrt(u) = 1 / (3 cbrt(u)^2)
        bid::CBRT => inv(mul(vec![num(3), pow(func(bid::CBRT, vec![u.clone()]), num(2))])),
        // d/du root(u, n) = (1/n) u^(1/n - 1)
        bid::ROOT => {
            if args.len() < 2 {
                return None;
            }
            let n = args[1].clone();
            mul(vec![
                inv(n.clone()),
                pow(u.clone(), add(vec![inv(n), num(-1)])),
            ])
        }
        // d/du |u| = u / |u|. That is the reference's own answer — `diff(abs(x))`
        // prints `x / |x|` — and it is why it is spelled this way rather than
        // as `sgn(u)`: `x / |x|` is built out of functions this port already
        // has, and it cancels against the `1/|u|` that `d/du ln|u|` puts next
        // to it, which is exactly the shape `integrate.rs`'s
        // `int dx/(ax+b) = ln|ax+b|/a` rule produces.
        //
        // Undefined at `u = 0`, as in the C++ (which does not special-case it
        // either).
        bid::ABS => mul(vec![u.clone(), inv(func(bid::ABS, vec![u.clone()]))]),
        bid::EXP => func(bid::EXP, vec![u.clone()]),
        bid::LN => inv(u.clone()),
        bid::LOG if args.len() == 1 => inv(u.clone()),
        bid::LOG => inv(mul(vec![u.clone(), func(bid::LN, vec![args[1].clone()])])),
        bid::LOG2 => inv(mul(vec![u.clone(), func(bid::LN, vec![num(2)])])),
        bid::LOG10 => inv(mul(vec![u.clone(), func(bid::LN, vec![num(10)])])),
        bid::SIN => func(bid::COS, vec![u.clone()]),
        bid::COS => neg(func(bid::SIN, vec![u.clone()])),
        bid::TAN => inv(pow(func(bid::COS, vec![u.clone()]), num(2))),
        bid::COT => neg(inv(pow(func(bid::SIN, vec![u.clone()]), num(2)))),
        bid::ASIN => inv(pow(
            add(vec![num(1), neg(pow(u.clone(), num(2)))]),
            ratio(1, 2),
        )),
        bid::ACOS => neg(inv(pow(
            add(vec![num(1), neg(pow(u.clone(), num(2)))]),
            ratio(1, 2),
        ))),
        bid::ATAN => inv(add(vec![num(1), pow(u.clone(), num(2))])),
        bid::ACOT => neg(inv(add(vec![num(1), pow(u.clone(), num(2))]))),
        bid::SINH => func(bid::COSH, vec![u.clone()]),
        bid::COSH => func(bid::SINH, vec![u.clone()]),
        bid::TANH => inv(pow(func(bid::COSH, vec![u.clone()]), num(2))),
        bid::ASINH => inv(pow(
            add(vec![pow(u.clone(), num(2)), num(1)]),
            ratio(1, 2),
        )),
        bid::ACOSH => inv(pow(
            add(vec![pow(u.clone(), num(2)), num(-1)]),
            ratio(1, 2),
        )),
        bid::ATANH => inv(add(vec![num(1), neg(pow(u.clone(), num(2)))])),
        _ => return None,
    };
    Some(mul(vec![outer, du]))
}

// ----------------------------------------------------------------------
// Builtin dispatch
// ----------------------------------------------------------------------

/// `diff(expr, x)` — the `DeriveFunction` builtin, restricted to the first
/// derivative (the C++ third argument selects a higher order).
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    if id.0 != id::DIFFERENTIATE || args.is_empty() || args.len() > 3 {
        return false;
    }
    let expr = args[0].clone();
    let xvar = match args.get(1) {
        Some(v) if v.is_symbolic() => v.clone(),
        _ => crate::polynomial::find_x_var(&expr).unwrap_or_else(|| MathStructure::symbolic("x")),
    };
    let order = match args.get(2) {
        Some(MathStructure::Number(n)) => match n.to_i64() {
            Some(k) if (1..=8).contains(&k) => k as usize,
            _ => return false,
        },
        None => 1,
        _ => return false,
    };
    let eo = crate::options::EvaluationOptions::default();
    let mut cur = expr;
    for _ in 0..order {
        let Some(d) = differentiate(&cur, &xvar) else {
            return false;
        };
        cur = d;
        crate::eval::evaluate_calculated_with(&mut cur, &eo);
    }
    *m = cur;
    true
}

pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    match name {
        "diff" | "derivative" => Some(FunctionId(id::DIFFERENTIATE)),
        _ => None,
    }
}

pub fn function_name(id: u32) -> Option<&'static str> {
    match id {
        self::id::DIFFERENTIATE => Some("diff"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::evaluate_to_string;

    fn ev(s: &str) -> String {
        evaluate_to_string(s).expect("evaluates")
    }

    #[test]
    fn polynomial_derivative() {
        assert_eq!(ev("diff(6x^2)"), "12x");
        assert_eq!(ev("diff(x^3 + 2x)"), "3x^2 + 2");
    }

    #[test]
    fn chain_rule_on_builtins() {
        // d/dx sin(2x) = 2 cos(2x)
        assert_eq!(ev("diff(sin(2x))"), "2 * cos(2x)");
    }

    #[test]
    fn exponential_and_logarithm() {
        assert_eq!(ev("diff(ln(x))"), "1 / x");
        // `e` numerifies to `2.718281828` in Approximate mode, so the
        // derivative is `2.718281828^x`, matching `qalc -t "diff(e^x, x)"`.
        assert_eq!(ev("diff(e^x, x)"), "2.718281828^x");
    }

    #[test]
    fn contains_detects_the_variable() {
        let x = MathStructure::symbolic("x");
        let m = crate::eval::parse_expression("sin(x) + 1").expect("parse");
        assert!(contains(&m, &x));
        let m2 = crate::eval::parse_expression("sin(y) + 1").expect("parse");
        assert!(!contains(&m2, &x));
    }
}
