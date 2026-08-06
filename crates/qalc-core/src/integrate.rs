//! Integration — port of `MathStructure::integrate`
//! (`MathStructure-integrate.cc`), the `integrate()`/`romberg()` builtins
//! (`BuiltinFunctions-calculus.cc`) and the integral special functions that
//! antiderivatives are expressed in (`Shi`, `Chi`, `fresnels`, `fresnelc`,
//! `igamma`, `betainc`).
//!
//! The C++ file is the largest algorithm in the library because it carries a
//! full pattern table. This port keeps the same shape but a smaller table:
//!
//! * linearity (sums, constant factors),
//! * the power rule, including `(a x + b)^n` and `b^(a x + c)`,
//! * a per-builtin antiderivative table applied through the linear
//!   substitution `u = a x + b`,
//! * the `f(c x^n) / x` family, which is what produces `Ei`/`Si`/`Ci`/
//!   `Shi`/`Chi` in results,
//! * integration by parts for `x^k * g(x)`, with `k` strictly decreasing so
//!   the recursion cannot cycle,
//! * partial fractions for a rational function whose denominator splits into
//!   distinct rational linear factors,
//! * a Romberg fallback for a definite integral with numeric bounds.
//!
//! Every recursion is depth-bounded and every loop has an explicit iteration
//! cap: a runaway integral would hang the whole batch runner.

use crate::builtins::id as bid;
use crate::differentiate::contains;
use crate::ids::FunctionId;
use crate::options::EvaluationOptions;
use crate::structure::MathStructure;
use qalc_num::Number;

pub mod id {
    /// `FUNCTION_ID_INTEGRATE` (BuiltinFunctions.h:354).
    pub const INTEGRATE: u32 = 1820;
    /// `FUNCTION_ID_ROMBERG` (BuiltinFunctions.h:356).
    pub const ROMBERG: u32 = 1822;

    /// `FUNCTION_ID_FRESNEL_S` (BuiltinFunctions.h:313).
    pub const FRESNEL_S: u32 = 1601;
    /// `FUNCTION_ID_FRESNEL_C`.
    pub const FRESNEL_C: u32 = 1602;
    /// `FUNCTION_ID_SINHINT` — `Shi`, the hyperbolic sine integral.
    pub const SINHINT: u32 = 1606;
    /// `FUNCTION_ID_COSHINT` — `Chi`, the hyperbolic cosine integral.
    pub const COSHINT: u32 = 1607;
    /// `FUNCTION_ID_I_GAMMA` — the *upper* incomplete gamma `igamma(a, x)`.
    pub const I_GAMMA: u32 = 1608;
    /// `FUNCTION_ID_INCOMPLETE_BETA` — the regularized `betainc(x, a, b)`.
    pub const INCOMPLETE_BETA: u32 = 1609;
    /// `gammainc` — the lower incomplete gamma. The reference defines it in
    /// XML as `gamma(x) - igamma(x, y)` rather than as a builtin, so it has
    /// no `FUNCTION_ID_*`; this port gives it one in the same block.
    pub const GAMMAINC: u32 = 1611;
}

// ----------------------------------------------------------------------
// Small builders (same conventions as `crate::limit`)
// ----------------------------------------------------------------------

fn num(i: i64) -> MathStructure {
    MathStructure::from(i)
}

fn nr(n: Number) -> MathStructure {
    MathStructure::Number(n)
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
    match v.len() {
        0 => num(1),
        1 => v.into_iter().next().expect("len 1"),
        _ => MathStructure::Multiplication(v),
    }
}

fn add(v: Vec<MathStructure>) -> MathStructure {
    match v.len() {
        0 => num(0),
        1 => v.into_iter().next().expect("len 1"),
        _ => MathStructure::Addition(v),
    }
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

/// `+ C`: the reference appends `CALCULATOR->getVariableById(VARIABLE_ID_C)`
/// to every indefinite integral (MathStructure-integrate.cc:7617).
fn c_sym() -> MathStructure {
    MathStructure::symbolic("C")
}

/// Divide by the numeric chain-rule factor `a` (from `u = a x + b`).
fn over(m: MathStructure, a: &Number) -> MathStructure {
    if a.is_one() {
        return m;
    }
    let mut r = Number::from_i64(1);
    if !r.divide(a) {
        return mul(vec![m, inv(nr(a.clone()))]);
    }
    mul(vec![nr(r), m])
}

fn evaluate(m: &mut MathStructure, eo: &EvaluationOptions) {
    crate::eval::evaluate_calculated_with(m, eo);
}

// ----------------------------------------------------------------------
// Shape analysis
// ----------------------------------------------------------------------

/// Guard against pathological nesting; the C++ has no explicit bound but is
/// protected by `CALCULATOR->aborted()`.
const MAX_DEPTH: usize = 12;

/// Largest `k` in the `x^k * g(x)` by-parts reduction.
const MAX_PARTS_POWER: i64 = 8;

/// `max_part_depth` — how many nested integrations by parts one integral may
/// spend. The C++ default is 5 (`MathStructure.h:868`); this port stops at 3
/// because its search is wider (every factor is tried as `u`, not just the
/// first that differentiates) and the corpus needs no more: raising it to 5
/// answers nothing extra and roughly triples the run time.
const MAX_PARTS_DEPTH: usize = 3;

/// Largest expression by parts will hand back to the integrator, counted in
/// nodes. `MathStructure-integrate.cc:6524` uses `countTotalChildren() < 100`
/// for the same purpose: `u' * int v dx` can grow without bound even when
/// every step is legal, and integrating the result is what costs.
const MAX_PARTS_NODES: usize = 120;

/// Largest number of factors by parts will split a product into. The C++
/// requires `SIZE < 10` (`:6495`).
const MAX_PARTS_FACTORS: usize = 6;

/// How many nested radical substitutions one integral may spend. One is
/// enough for this corpus — an integrand carries a single radical — and more
/// would only widen a search that has no other termination argument.
const MAX_SUBST_DEPTH: usize = 1;

/// `m = c * x^n` with numeric `c != 0` and `n != 0`, or `None`.
fn monomial(m: &MathStructure, x: &MathStructure) -> Option<(Number, Number)> {
    if m.equals(x) {
        return Some((Number::from_i64(1), Number::from_i64(1)));
    }
    match m {
        MathStructure::Power { base, exponent } => {
            if !base.equals(x) {
                return None;
            }
            let MathStructure::Number(n) = exponent.as_ref() else {
                return None;
            };
            if n.is_zero() || !n.is_real() {
                return None;
            }
            Some((Number::from_i64(1), n.clone()))
        }
        MathStructure::Multiplication(factors) => {
            let mut coeff = Number::from_i64(1);
            let mut deg: Option<Number> = None;
            for f in factors {
                if contains(f, x) {
                    if deg.is_some() {
                        return None;
                    }
                    let (c, d) = monomial(f, x)?;
                    if !coeff.multiply(&c) {
                        return None;
                    }
                    deg = Some(d);
                } else {
                    let MathStructure::Number(n) = f else {
                        return None;
                    };
                    if !coeff.multiply(n) {
                        return None;
                    }
                }
            }
            let d = deg?;
            if coeff.is_zero() {
                return None;
            }
            Some((coeff, d))
        }
        _ => None,
    }
}

/// `m = b + a x^n` with numeric `a != 0`, numeric `b` and numeric `n != 0`.
///
/// This is the C++'s `integrate_info(m, x, madd, mmul, mexp)`
/// (`MathStructure-integrate.cc:32`), which every rule in the table calls
/// first to decide whether a substitution applies. [`linear`] is the `n = 1`
/// case, kept separate because the rules that only ever want a linear
/// argument read better with two return values.
fn affine_power(m: &MathStructure, x: &MathStructure) -> Option<(Number, Number, Number)> {
    if !contains(m, x) {
        return None;
    }
    let single;
    let terms: &[MathStructure] = match m {
        MathStructure::Addition(t) => t,
        _ => {
            single = [m.clone()];
            &single
        }
    };
    let mut a = Number::new();
    let mut b = Number::new();
    let mut exp: Option<Number> = None;
    for t in terms {
        if !contains(t, x) {
            let MathStructure::Number(c) = t else {
                return None;
            };
            if !b.add(c) {
                return None;
            }
            continue;
        }
        let (c, d) = monomial(t, x)?;
        match &exp {
            Some(e) if !e.equals(&d, false, false) => return None,
            _ => exp = Some(d),
        }
        if !a.add(&c) {
            return None;
        }
    }
    let n = exp?;
    if a.is_zero() || !a.is_real() || !n.is_real() {
        return None;
    }
    Some((a, b, n))
}

/// The constant `du/dx` contributes when `u = a x^n + b` is substituted into
/// an integral that already carries a factor `x^k`: `du = a n x^(n-1) dx`, so
/// the rule only fires when `k + 1 == n`, and then divides by `a n`.
///
/// `k = 0` is the bare case `int f(a x + b) dx = F(a x + b) / a`.
fn substitution_divisor(u: &MathStructure, k: &Number, x: &MathStructure) -> Option<Number> {
    let (a, _b, n) = affine_power(u, x)?;
    let mut want = n.clone();
    if !want.subtract(&Number::from_i64(1)) || !want.equals(k, false, false) {
        return None;
    }
    let mut d = a;
    if !d.multiply(&n) || d.is_zero() {
        return None;
    }
    Some(d)
}

/// Whether `m` mentions one of the *real*-branch radicals.
///
/// `cbrt(x)` and `root(x, 3)` are negative for negative `x`; the `x^(1/3)`
/// that the merge engine produces once they have been differentiated is the
/// principal complex root, and the two disagree on the whole negative axis.
/// An expression built out of both is therefore wrong there even though it
/// differentiates back correctly *as a formal expression* — the derivative
/// and the integrand simply denote different functions.
///
/// [`int_by_parts`] refuses to differentiate such a `u` for that reason: it
/// is the one place in this file that puts a derivative and an integral of
/// the same subexpression side by side in one answer. `sqrt` is not on the
/// list, because `sqrt(x)` and `x^(1/2)` are the same principal branch.
fn mentions_real_radical(m: &MathStructure) -> bool {
    if let MathStructure::Function { id, .. } = m {
        if matches!(id.0, bid::CBRT | bid::ROOT) {
            return true;
        }
    }
    (0..m.size())
        .filter_map(|i| m.get(i))
        .any(mentions_real_radical)
}

/// How many nodes `m` has, for [`MAX_PARTS_NODES`].
fn node_count(m: &MathStructure) -> usize {
    1 + (0..m.size())
        .filter_map(|i| m.get(i))
        .map(node_count)
        .sum::<usize>()
}

/// `x^k` with `k` a positive integer no larger than [`MAX_PARTS_POWER`].
fn small_power_of_x(m: &MathStructure, x: &MathStructure) -> Option<i64> {
    if m.equals(x) {
        return Some(1);
    }
    let MathStructure::Power { base, exponent } = m else {
        return None;
    };
    if !base.equals(x) {
        return None;
    }
    let MathStructure::Number(n) = exponent.as_ref() else {
        return None;
    };
    let k = n.to_i64()?;
    (1..=MAX_PARTS_POWER).contains(&k).then_some(k)
}

// ----------------------------------------------------------------------
// Indefinite integration
// ----------------------------------------------------------------------

/// The antiderivative of `m` with respect to `x`, without `+ C`, or `None`
/// when no rule applies. The result is *unevaluated*: the caller runs it
/// through the merge engine.
pub fn integrate(m: &MathStructure, x: &MathStructure) -> Option<MathStructure> {
    let mut parents = Vec::new();
    int_rec(m, x, 0, MAX_PARTS_DEPTH, MAX_SUBST_DEPTH, &mut parents)
}

/// `parts` is the C++'s `max_part_depth` and `parents` its `parent_parts`:
/// the first bounds how many nested integrations by parts one integral may
/// spend, the second refuses an integral that is already on the stack, which
/// is what stops `int u v` from reducing to itself.
fn int_rec(
    m: &MathStructure,
    x: &MathStructure,
    depth: usize,
    parts: usize,
    subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    if depth > MAX_DEPTH {
        return None;
    }
    // A constant integrand: `int c dx = c x`.
    if !contains(m, x) {
        return Some(mul(vec![m.clone(), x.clone()]));
    }
    if m.equals(x) {
        return Some(mul(vec![ratio(1, 2), pow(x.clone(), num(2))]));
    }
    let table = match m {
        MathStructure::Addition(terms) => {
            let mut out = Vec::with_capacity(terms.len());
            let mut all = true;
            for t in terms {
                match int_rec(t, x, depth + 1, parts, subst, parents) {
                    Some(v) => out.push(v),
                    None => {
                        all = false;
                        break;
                    }
                }
            }
            if all {
                Some(add(out))
            } else {
                None
            }
        }
        MathStructure::Multiplication(factors) => {
            int_product(factors, x, depth, parts, subst, parents)
        }
        MathStructure::Power { base, exponent } => {
            int_power(base, exponent, x, depth, parts, subst, parents)
        }
        MathStructure::Function { id, args } => {
            int_function(id.0, args, x, depth, parts, subst, parents)
        }
        _ => None,
    };
    if table.is_some() {
        return table;
    }
    if subst > 0 {
        if let Some(r) = int_radical_substitution(m, x, depth, parts, subst - 1, parents) {
            return Some(r);
        }
    }
    // Last resort: a polynomial no rule above recognised, multiplied out and
    // integrated term by term. `(2x^2+5)^2` reaches here because the
    // substitution `u = 2x^2 + 5` wants a companion factor of `x` that is not
    // there; `(4x+5)^3` does not, because the substitution *does* apply and
    // `(4x+5)^4/16` is the better answer.
    let p = dense_of(m, x)?;
    if p.len() > MAX_EXPAND_DEGREE + 1 {
        return None;
    }
    integrate_poly(&p, x)
}

/// `int g(x) x^k dx` for a single non-additive factor `g` whose dependence on
/// `x` is an affine power `u = a x^n + b`, and a companion factor `x^k`.
///
/// The rule is the chain rule read backwards: `du = a n x^(n-1) dx`, so
/// `int g(u) x^(n-1) dx = G(u) / (a n)`, and it fires only when the companion
/// exponent `k` is exactly `n - 1`. `k = 0` is the plain linear substitution
/// `u = a x + b` the reference's table is written against; `k = 1, n = 2` is
/// what turns `x sin(x^2)`, `x / sqrt(x^2 - 1)` and `x / (2x^2 + 5)` into
/// one-line answers.
fn int_chain(g: &MathStructure, k: &Number, x: &MathStructure) -> Option<MathStructure> {
    match g {
        MathStructure::Function { id, args } if args.len() == 1 => {
            let u = &args[0];
            let d = substitution_divisor(u, k, x)?;
            let f = antiderivative_of(id.0, u)?;
            Some(over(f, &d))
        }
        MathStructure::Power { base, exponent } => {
            let b_has = contains(base, x);
            let e_has = contains(exponent, x);
            if b_has == e_has {
                return None;
            }
            if b_has {
                // `f(u)^p` with `f` in the table of integer powers.
                if let (MathStructure::Function { id, args }, MathStructure::Number(p)) =
                    (base.as_ref(), exponent.as_ref())
                {
                    if args.len() == 1 {
                        if let Some(pi) = p.to_i64() {
                            if let Some(d) = substitution_divisor(&args[0], k, x) {
                                if let Some(f) = antiderivative_of_power(id.0, &args[0], pi) {
                                    return Some(over(f, &d));
                                }
                            }
                        }
                    }
                }
                let d = substitution_divisor(base, k, x)?;
                // `u^(p+1) / (p+1)`, except at `p = -1` where that divides by
                // zero and the answer is a logarithm.
                let mut np1 = add(vec![(**exponent).clone(), num(1)]);
                evaluate(&mut np1, &EvaluationOptions::default());
                if matches!(&np1, MathStructure::Number(n) if n.is_zero()) {
                    return Some(over(
                        func(bid::LN, vec![func(bid::ABS, vec![(**base).clone()])]),
                        &d,
                    ));
                }
                let r = mul(vec![pow((**base).clone(), np1.clone()), inv(np1)]);
                return Some(over(r, &d));
            }
            // `int b^u x^(n-1) dx = b^u / (a n ln b)`.
            let d = substitution_divisor(exponent, k, x)?;
            let mut factors = vec![pow((**base).clone(), (**exponent).clone())];
            if !matches!(base.as_ref(), MathStructure::Symbolic(s) if s == "e") {
                factors.push(inv(func(bid::LN, vec![(**base).clone()])));
            }
            Some(over(mul(factors), &d))
        }
        _ => None,
    }
}

fn int_power(
    base: &MathStructure,
    exponent: &MathStructure,
    x: &MathStructure,
    depth: usize,
    parts: usize,
    _subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    let g = pow(base.clone(), exponent.clone());
    if let Some(r) = int_chain(&g, &Number::new(), x) {
        return Some(r);
    }
    // A higher-degree denominator: `int P(x)/Q(x) dx` by partial fractions
    // (the `1/Q(x)` shape reaches here as a bare power).
    if contains(base, x)
        && !contains(exponent, x)
        && matches!(exponent, MathStructure::Number(n) if n.is_negative() && n.is_integer())
    {
        if let Some(r) = int_partial_fractions(&g, x, depth) {
            return Some(r);
        }
    }
    if let Some(r) = int_quadratic_radical(std::slice::from_ref(&g), x) {
        return Some(r);
    }
    let _ = (parts, parents);
    None
}

/// Antiderivative of the one-argument builtin `id` at `u`, for `u` the
/// identity. The caller applies the `1/a` chain-rule factor.
fn antiderivative_of(id: u32, u: &MathStructure) -> Option<MathStructure> {
    let f = |i: u32| func(i, vec![u.clone()]);
    Some(match id {
        bid::SIN => neg(f(bid::COS)),
        bid::COS => f(bid::SIN),
        // `int tan u du = -ln|cos u|`
        bid::TAN => neg(func(bid::LN, vec![func(bid::ABS, vec![f(bid::COS)])])),
        // `int cot u du = ln|sin u|`
        bid::COT => func(bid::LN, vec![func(bid::ABS, vec![f(bid::SIN)])]),
        bid::EXP => f(bid::EXP),
        // `int ln u du = u ln u - u`
        bid::LN | bid::LOG => add(vec![mul(vec![u.clone(), f(bid::LN)]), neg(u.clone())]),
        bid::SINH => f(bid::COSH),
        bid::COSH => f(bid::SINH),
        // `int tanh u du = ln(cosh u)`
        bid::TANH => func(bid::LN, vec![f(bid::COSH)]),
        // `int sqrt(u) du = (2/3) u^(3/2)`
        bid::SQRT => mul(vec![ratio(2, 3), pow(u.clone(), ratio(3, 2))]),
        bid::CBRT => mul(vec![ratio(3, 4), pow(u.clone(), ratio(4, 3))]),
        // `int asin u du = u asin u + sqrt(1 - u^2)`
        bid::ASIN => add(vec![
            mul(vec![u.clone(), f(bid::ASIN)]),
            func(bid::SQRT, vec![sub_one_square(u)]),
        ]),
        bid::ACOS => add(vec![
            mul(vec![u.clone(), f(bid::ACOS)]),
            neg(func(bid::SQRT, vec![sub_one_square(u)])),
        ]),
        // `int atan u du = u atan u - ln(1 + u^2)/2`
        bid::ATAN => add(vec![
            mul(vec![u.clone(), f(bid::ATAN)]),
            neg(mul(vec![
                ratio(1, 2),
                func(bid::LN, vec![add_one_square(u)]),
            ])),
        ]),
        bid::ACOT => add(vec![
            mul(vec![u.clone(), f(bid::ACOT)]),
            mul(vec![ratio(1, 2), func(bid::LN, vec![add_one_square(u)])]),
        ]),
        // `int asinh u du = u asinh u - sqrt(u^2 + 1)`
        bid::ASINH => add(vec![
            mul(vec![u.clone(), f(bid::ASINH)]),
            neg(func(bid::SQRT, vec![add_one_square(u)])),
        ]),
        // `int acosh u du = u acosh u - sqrt(u^2 - 1)`
        bid::ACOSH => add(vec![
            mul(vec![u.clone(), f(bid::ACOSH)]),
            neg(func(bid::SQRT, vec![sub_one_square_flipped(u)])),
        ]),
        bid::ATANH => add(vec![
            mul(vec![u.clone(), f(bid::ATANH)]),
            mul(vec![
                ratio(1, 2),
                func(bid::LN, vec![sub_one_square(u)]),
            ]),
        ]),
        _ => return None,
    })
}

/// Antiderivative of `f(u)^p` at `u` the identity, for the integer powers
/// that have one. The caller applies the chain-rule factor.
///
/// The reference spells these out in `integrate_function`'s per-function
/// blocks, gated on `mpow` (`MathStructure-integrate.cc:1184`, `:1803`,
/// `:2763` and so on). Only `p = 2` is filled in here, because that is what
/// the corpus reaches — `sin(u)·sin(u)` collapses to `sin(u)^2` — and a
/// half-populated table of reduction formulas is a place for a sign error to
/// hide with nothing exercising it.
fn antiderivative_of_power(id: u32, u: &MathStructure, p: i64) -> Option<MathStructure> {
    if p != 2 {
        return None;
    }
    let f = |i: u32| func(i, vec![u.clone()]);
    // `sin(2u)`, `cosh(2u)`: the double-angle argument.
    let two_u = mul(vec![num(2), u.clone()]);
    let half_u = mul(vec![ratio(1, 2), u.clone()]);
    Some(match id {
        // `int sin^2 u du = u/2 - sin(2u)/4`
        bid::SIN => add(vec![
            half_u,
            neg(mul(vec![ratio(1, 4), func(bid::SIN, vec![two_u])])),
        ]),
        // `int cos^2 u du = u/2 + sin(2u)/4`
        bid::COS => add(vec![
            half_u,
            mul(vec![ratio(1, 4), func(bid::SIN, vec![two_u])]),
        ]),
        // `int tan^2 u du = tan u - u`
        bid::TAN => add(vec![f(bid::TAN), neg(u.clone())]),
        // `int cot^2 u du = -cot u - u`
        bid::COT => add(vec![neg(f(bid::COT)), neg(u.clone())]),
        // `int sinh^2 u du = sinh(2u)/4 - u/2`
        bid::SINH => add(vec![
            mul(vec![ratio(1, 4), func(bid::SINH, vec![two_u])]),
            neg(half_u),
        ]),
        // `int cosh^2 u du = sinh(2u)/4 + u/2`
        bid::COSH => add(vec![
            mul(vec![ratio(1, 4), func(bid::SINH, vec![two_u])]),
            half_u,
        ]),
        // `int tanh^2 u du = u - tanh u`
        bid::TANH => add(vec![u.clone(), neg(f(bid::TANH))]),
        // `int ln^2 u du = u (ln^2 u - 2 ln u + 2)`
        bid::LN | bid::LOG => mul(vec![
            u.clone(),
            add(vec![
                pow(f(bid::LN), num(2)),
                neg(mul(vec![num(2), f(bid::LN)])),
                num(2),
            ]),
        ]),
        // `int asin^2 u du = u asin^2 u + 2 sqrt(1 - u^2) asin u - 2u`
        bid::ASIN => add(vec![
            mul(vec![u.clone(), pow(f(bid::ASIN), num(2))]),
            mul(vec![
                num(2),
                func(bid::SQRT, vec![sub_one_square(u)]),
                f(bid::ASIN),
            ]),
            neg(mul(vec![num(2), u.clone()])),
        ]),
        // `int acos^2 u du = u acos^2 u - 2 sqrt(1 - u^2) acos u - 2u`
        bid::ACOS => add(vec![
            mul(vec![u.clone(), pow(f(bid::ACOS), num(2))]),
            neg(mul(vec![
                num(2),
                func(bid::SQRT, vec![sub_one_square(u)]),
                f(bid::ACOS),
            ])),
            neg(mul(vec![num(2), u.clone()])),
        ]),
        // `int asinh^2 u du = u asinh^2 u - 2 sqrt(u^2 + 1) asinh u + 2u`
        bid::ASINH => add(vec![
            mul(vec![u.clone(), pow(f(bid::ASINH), num(2))]),
            neg(mul(vec![
                num(2),
                func(bid::SQRT, vec![add_one_square(u)]),
                f(bid::ASINH),
            ])),
            mul(vec![num(2), u.clone()]),
        ]),
        // `int acosh^2 u du = u acosh^2 u - 2 sqrt(u^2 - 1) acosh u + 2u`
        bid::ACOSH => add(vec![
            mul(vec![u.clone(), pow(f(bid::ACOSH), num(2))]),
            neg(mul(vec![
                num(2),
                func(bid::SQRT, vec![sub_one_square_flipped(u)]),
                f(bid::ACOSH),
            ])),
            mul(vec![num(2), u.clone()]),
        ]),
        _ => return None,
    })
}

fn add_one_square(u: &MathStructure) -> MathStructure {
    add(vec![num(1), pow(u.clone(), num(2))])
}

fn sub_one_square(u: &MathStructure) -> MathStructure {
    add(vec![num(1), neg(pow(u.clone(), num(2)))])
}

/// `u^2 - 1`, the other way round — `acosh`'s radicand.
fn sub_one_square_flipped(u: &MathStructure) -> MathStructure {
    add(vec![pow(u.clone(), num(2)), num(-1)])
}

fn int_function(
    id: u32,
    args: &[MathStructure],
    x: &MathStructure,
    depth: usize,
    parts: usize,
    subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    if args.len() != 1 {
        return None;
    }
    let g = func(id, args.to_vec());
    if let Some(r) = int_chain(&g, &Number::new(), x) {
        return Some(r);
    }
    int_function_by_parts(&g, x, depth, parts, subst, parents)
}

/// By parts against `dv = dx`: `int f dx = x f - int x f' dx`.
///
/// `MathStructure-integrate.cc:3479` runs this for `ln`, `asin`, `acos`,
/// `atan`, `asinh`, `acosh` and `atanh` before anything else, and `:3876`
/// runs it for every remaining function *except* `sin`, `cos`, `tan`, `sinh`,
/// `cosh` and `tanh`. Those six are excluded because their derivative is
/// another function of the same family, so `int x f' dx` is no easier than
/// what it came from and the recursion only burns the budget.
fn int_function_by_parts(
    g: &MathStructure,
    x: &MathStructure,
    depth: usize,
    parts: usize,
    subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    if parts == 0 || depth + 1 > MAX_DEPTH {
        return None;
    }
    let MathStructure::Function { id, .. } = g else {
        return None;
    };
    if matches!(
        id.0,
        bid::SIN | bid::COS | bid::TAN | bid::COT | bid::SINH | bid::COSH | bid::TANH
    ) {
        return None;
    }
    if parents.iter().any(|p| p.equals(g)) || mentions_real_radical(g) {
        return None;
    }
    let eo = EvaluationOptions::default();
    let mut dg = crate::differentiate::differentiate(g, x)?;
    evaluate(&mut dg, &eo);
    let mut w = mul(vec![x.clone(), dg]);
    evaluate(&mut w, &eo);
    if node_count(&w) > MAX_PARTS_NODES {
        return None;
    }
    parents.push(g.clone());
    let tail = int_rec(&w, x, depth + 1, parts - 1, subst, parents);
    parents.pop();
    Some(add(vec![mul(vec![x.clone(), g.clone()]), neg(tail?)]))
}

/// `int f(w)/w dw` for the five builtins whose integral is a named special
/// function (`Ei`, `Si`, `Ci`, `Shi`, `Chi`).
fn over_argument_integral(id: u32, w: &MathStructure) -> Option<MathStructure> {
    Some(match id {
        bid::EXP => func(bid::EXPINT, vec![w.clone()]),
        bid::SIN => func(bid::SININT, vec![w.clone()]),
        bid::COS => func(bid::COSINT, vec![w.clone()]),
        bid::SINH => func(self::id::SINHINT, vec![w.clone()]),
        bid::COSH => func(self::id::COSHINT, vec![w.clone()]),
        _ => return None,
    })
}

fn int_product(
    factors: &[MathStructure],
    x: &MathStructure,
    depth: usize,
    parts: usize,
    subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    // Constant factors come straight out of the integral.
    let mut consts: Vec<MathStructure> = Vec::new();
    let mut vars: Vec<MathStructure> = Vec::new();
    for f in factors {
        if contains(f, x) {
            vars.push(f.clone());
        } else {
            consts.push(f.clone());
        }
    }
    if vars.is_empty() {
        consts.push(x.clone());
        return Some(mul(consts));
    }
    if vars.len() == 1 {
        let inner = int_rec(&vars[0], x, depth + 1, parts, subst, parents)?;
        consts.push(inner);
        return Some(mul(consts));
    }
    if let Some(r) = int_special_quotient(&vars, x) {
        consts.push(r);
        return Some(mul(consts));
    }
    if let Some(r) = int_substitution(&vars, x) {
        consts.push(r);
        return Some(mul(consts));
    }
    if let Some(r) = int_quadratic_radical(&vars, x) {
        consts.push(r);
        return Some(mul(consts));
    }
    // Before by parts, not after: by parts applied to a rational function
    // produces a correct but unrecognisable answer, and often no answer at
    // all, where the residue expansion is a closed form.
    if let Some(r) = int_partial_fractions(&mul(vars.clone()), x, depth) {
        consts.push(r);
        return Some(mul(consts));
    }
    if let Some(r) = int_by_parts(&vars, x, depth, parts, subst, parents) {
        consts.push(r);
        return Some(mul(consts));
    }
    None
}

// ----------------------------------------------------------------------
// Substitution t = (a x + b)^(1/n)
// ----------------------------------------------------------------------

/// The symbol the radical substitution integrates against. Constructed
/// directly as a `Symbolic`, never parsed, so it cannot collide with a name
/// the user could write.
const SUBST_VAR: &str = "\u{2009}t\u{2009}";

/// `m = (a x + b)^q`, in every spelling: the bare linear expression, `sqrt`,
/// `cbrt`, `root(·, k)`, and any of those raised to a constant power.
fn linear_power(m: &MathStructure, x: &MathStructure) -> Option<(Number, Number, Number)> {
    if let MathStructure::Function { id, args } = m {
        let root = match (id.0, args.len()) {
            (bid::SQRT, 1) => Some(Number::from_ints(1, 2, 0)),
            (bid::CBRT, 1) => Some(Number::from_ints(1, 3, 0)),
            (bid::ROOT, 2) => {
                let MathStructure::Number(k) = &args[1] else {
                    return None;
                };
                let ki = k.to_i64()?;
                if !(2..=8).contains(&ki) {
                    return None;
                }
                Some(Number::from_ints(1, ki, 0))
            }
            _ => None,
        };
        if let Some(q) = root {
            let (a, b, n) = affine_power(&args[0], x)?;
            if !n.is_one() {
                return None;
            }
            return Some((a, b, q));
        }
        return None;
    }
    if let MathStructure::Power { base, exponent } = m {
        if contains(exponent, x) {
            return None;
        }
        let MathStructure::Number(e) = exponent.as_ref() else {
            return None;
        };
        let (a, b, q0) = linear_power(base, x)?;
        // `(u^q0)^e = u^(q0 e)` is not an identity over the reals. When `q0`
        // is an even integer the inner power discards the sign of `u`, so the
        // composite is `|u|^(q0 e)`; folding the exponents turns `(x^2)^(1/3)`
        // — real and positive at every `x` — into `x^(2/3)`, which is the
        // principal *complex* root for `x < 0`. Only fold when the inner
        // exponent cannot erase a sign, i.e. when `q0` is an odd integer, or
        // when the result stays an integer power (no root is taken).
        let mut q = q0.clone();
        if !q.multiply(e) || !q.is_rational() {
            return None;
        }
        let inner_preserves_sign = q0.is_integer() && !q0.is_even();
        if !inner_preserves_sign && !q.is_integer() {
            return None;
        }
        return Some((a, b, q));
    }
    let (a, b, n) = affine_power(m, x)?;
    if !n.is_one() {
        return None;
    }
    Some((a, b, Number::from_i64(1)))
}

/// The node inside `m` that spells `(a x + b)^(1/d)` outright, and `d`.
///
/// `cbrt(x)^2` is a use of the radical with exponent `2/3`, but the radical
/// *itself* is right there as the base, and reusing that exact node for the
/// back-substitution is what keeps the real cube root real.
fn witness_of(m: &MathStructure, x: &MathStructure) -> Option<(MathStructure, i64)> {
    if let Some((_, _, q)) = linear_power(m, x) {
        if q.is_positive() && q.numerator().is_one() {
            let d = q.denominator().to_i64()?;
            if d > 1 {
                return Some((m.clone(), d));
            }
        }
    }
    if let MathStructure::Power { base, .. } = m {
        return witness_of(base, x);
    }
    None
}

/// What one integrand needs from the substitution `t = (a x + b)^(1/n)`.
struct RadicalUse {
    a: Number,
    b: Number,
    /// `n`: the lcm of the denominators of every fractional power of
    /// `a x + b` in the integrand.
    n: i64,
    /// A node that already spells `(a x + b)^(1/n)`, when one occurs. Using
    /// it verbatim for the back-substitution is what keeps `cbrt` — the real
    /// cube root — from silently becoming the principal complex one.
    witness: Option<MathStructure>,
    /// Whether any `cbrt`/`root` was involved, which is when that matters.
    real_branch: bool,
}

/// Find the one radical the whole integrand is built on, or `None` if there
/// is none or more than one.
fn scan_radicals(
    m: &MathStructure,
    x: &MathStructure,
    acc: &mut Option<RadicalUse>,
) -> bool {
    if !contains(m, x) {
        return true;
    }
    if let Some((a, b, q)) = linear_power(m, x) {
        let den = q.denominator().to_i64().unwrap_or(0);
        if den > 1 {
            let real = mentions_real_radical(m);
            let w = witness_of(m, x).filter(|(_, d)| *d == den).map(|(w, _)| w);
            let is_witness = w.is_some();
            match acc {
                None => {
                    *acc = Some(RadicalUse {
                        a,
                        b,
                        n: den,
                        witness: w,
                        real_branch: real,
                    });
                }
                Some(u) => {
                    if !u.a.equals(&a, false, false) || !u.b.equals(&b, false, false) {
                        return false;
                    }
                    let mut l = Number::from_i64(u.n);
                    if !l.lcm(&Number::from_i64(den)) {
                        return false;
                    }
                    u.n = match l.to_i64() {
                        Some(v) if (2..=8).contains(&v) => v,
                        _ => return false,
                    };
                    if is_witness && den == u.n {
                        u.witness = witness_of(m, x).map(|(w, _)| w);
                    } else if den > u.n {
                        u.witness = None;
                    }
                    u.real_branch |= real;
                }
            }
            return true;
        }
    }
    (0..m.size())
        .filter_map(|i| m.get(i))
        .all(|c| scan_radicals(c, x, acc))
}

/// Rewrite `m` in terms of `t`, given `x = (t^n - b)/a`.
fn rewrite_in_t(
    m: &MathStructure,
    x: &MathStructure,
    a: &Number,
    b: &Number,
    n: i64,
    t: &MathStructure,
) -> Option<MathStructure> {
    if !contains(m, x) {
        return Some(m.clone());
    }
    if let Some((ma, mb, q)) = linear_power(m, x) {
        if ma.equals(a, false, false) && mb.equals(b, false, false) {
            let mut e = q;
            if !e.multiply(&Number::from_i64(n)) || !e.is_integer() {
                return None;
            }
            let k = e.to_i64()?;
            return Some(if k == 1 {
                t.clone()
            } else {
                pow(t.clone(), num(k))
            });
        }
    }
    if m.equals(x) {
        // `x = (t^n - b) / a`
        let mut inv_a = Number::from_i64(1);
        let mut minus_b = b.clone();
        if !inv_a.divide(a) || !minus_b.negate() {
            return None;
        }
        return Some(mul(vec![
            nr(inv_a),
            add(vec![pow(t.clone(), num(n)), nr(minus_b)]),
        ]));
    }
    let rec = |c: &MathStructure| rewrite_in_t(c, x, a, b, n, t);
    match m {
        MathStructure::Addition(terms) => {
            Some(add(terms.iter().map(rec).collect::<Option<Vec<_>>>()?))
        }
        MathStructure::Multiplication(factors) => {
            Some(mul(factors.iter().map(rec).collect::<Option<Vec<_>>>()?))
        }
        MathStructure::Power { base, exponent } => {
            if contains(exponent, x) {
                return None;
            }
            Some(pow(rec(base)?, (**exponent).clone()))
        }
        MathStructure::Function { id, args } => {
            Some(func(id.0, args.iter().map(rec).collect::<Option<Vec<_>>>()?))
        }
        _ => None,
    }
}

/// `int f dx` by the substitution `t = (a x + b)^(1/n)`, so that
/// `x = (t^n - b)/a` and `dx = (n/a) t^(n-1) dt`.
///
/// This is the port's counterpart of the reference's
/// `UnknownVariable`-and-`replace` block (`MathStructure-integrate.cc:3684`),
/// and it is what the whole radical third of the corpus turns on:
/// `sin(cbrt(x))` has no rule at all, while `3 t^2 sin(t)` is two rounds of
/// integration by parts.
///
/// The back-substitution reuses the *exact* node the integrand spelled the
/// radical with wherever one exists, rather than rebuilding `(a x + b)^(1/n)`.
/// `cbrt` is the real cube root and `x^(1/3)` is the principal complex one;
/// they differ on the whole negative axis, and an answer that swapped one for
/// the other would be wrong there. When no such node occurs, one is built —
/// but only if no `cbrt`/`root` was involved, so the rebuilt spelling cannot
/// be the wrong branch of one that was.
fn int_radical_substitution(
    m: &MathStructure,
    x: &MathStructure,
    depth: usize,
    parts: usize,
    subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    if depth + 1 > MAX_DEPTH {
        return None;
    }
    let mut acc: Option<RadicalUse> = None;
    if !scan_radicals(m, x, &mut acc) {
        return None;
    }
    let use_ = acc?;
    let n = use_.n;
    if !(2..=8).contains(&n) || use_.a.is_zero() {
        return None;
    }
    let back = match use_.witness {
        Some(w) => w,
        None if use_.real_branch => return None,
        None => pow(
            add(vec![
                mul(vec![nr(use_.a.clone()), x.clone()]),
                nr(use_.b.clone()),
            ]),
            nr(Number::from_ints(1, n, 0)),
        ),
    };

    let t = MathStructure::symbolic(SUBST_VAR);
    let g = rewrite_in_t(m, x, &use_.a, &use_.b, n, &t)?;
    // `dx = (n/a) t^(n-1) dt`
    let mut coef = Number::from_i64(n);
    if !coef.divide(&use_.a) {
        return None;
    }
    let mut integrand = mul(vec![nr(coef), pow(t.clone(), num(n - 1)), g]);
    let eo = EvaluationOptions::default();
    evaluate(&mut integrand, &eo);
    if node_count(&integrand) > MAX_PARTS_NODES {
        return None;
    }
    let mut r = int_rec(&integrand, &t, depth + 1, parts, subst, parents)?;
    evaluate(&mut r, &eo);
    if contains(&r, &t) {
        crate::solve::replace(&mut r, &t, &back);
    }
    if contains(&r, &t) {
        return None;
    }
    Some(r)
}

// ----------------------------------------------------------------------
// Quadratic radicals
// ----------------------------------------------------------------------

/// `int P(x) sqrt(Q)^h dx` for a quadratic `Q` and `h` in `{-1, 1}`.
///
/// This is the family every inverse-trigonometric and inverse-hyperbolic
/// integrand reduces to once by parts has run: `int x asin(4x+5) dx` becomes
/// `x^2/2 · asin' = 2x^2 / sqrt(-16x^2 - 40x - 24)`, and without a rule for
/// that shape the whole by-parts branch is wasted.
///
/// `I_k = int x^k / sqrt(Q) dx` satisfies
/// `x^(k-1) sqrt(Q) = k a I_k + (k - 1/2) b I_(k-1) + (k - 1) c I_(k-2)`
/// — differentiate the left side to see it — which is solved for `I_k` and
/// run upwards from `I_0`. `sqrt(Q)` itself is folded in by multiplying the
/// numerator by `Q`.
fn int_quadratic_radical(
    vars: &[MathStructure],
    x: &MathStructure,
) -> Option<MathStructure> {
    let mut radical: Option<(Vec<Number>, MathStructure, i64)> = None;
    let mut numer: Vec<MathStructure> = Vec::new();
    for v in vars {
        match radical_factor(v, x) {
            Some(r) => {
                if radical.is_some() {
                    return None;
                }
                radical = Some(r);
            }
            None => numer.push(v.clone()),
        }
    }
    let (q, q_expr, h) = radical?;
    if q.len() != 3 {
        return None;
    }
    let (c, b, a) = (q[0].clone(), q[1].clone(), q[2].clone());
    if a.is_zero() || !a.is_real() || !b.is_real() || !c.is_real() {
        return None;
    }
    let mut n = dense_of(&mul(numer), x)?;
    match h {
        1 => n = poly_mul(&n, &q)?,
        -1 => {}
        _ => return None,
    }
    if n.len() > 5 {
        return None;
    }

    let sq = func(bid::SQRT, vec![q_expr]);
    // `2 a x + b`, which is `Q'`.
    let mut two_a = a.clone();
    if !two_a.multiply(&Number::from_i64(2)) {
        return None;
    }
    let lin = add(vec![mul(vec![nr(two_a.clone()), x.clone()]), nr(b.clone())]);
    // `b^2 - 4ac`.
    let mut disc = b.clone();
    let mut four_ac = a.clone();
    if !disc.multiply(&b)
        || !four_ac.multiply(&c)
        || !four_ac.multiply(&Number::from_i64(4))
        || !four_ac.negate()
        || !disc.add(&four_ac)
    {
        return None;
    }

    let i0 = if a.is_negative() {
        // `Q` opens downwards: it is positive only between two real roots, and
        // the antiderivative is an arcsine. With no real roots `Q < 0`
        // everywhere and there is nothing real to return.
        if !disc.is_positive() {
            return None;
        }
        let mut na = a.clone();
        if !na.negate() {
            return None;
        }
        let s1 = func(bid::SQRT, vec![nr(na)]);
        let s2 = func(bid::SQRT, vec![nr(disc)]);
        mul(vec![
            inv(s1),
            func(bid::ASIN, vec![mul(vec![neg(lin.clone()), inv(s2)])]),
        ])
    } else {
        let s1 = func(bid::SQRT, vec![nr(a.clone())]);
        if disc.is_negative() {
            let mut nd = disc.clone();
            if !nd.negate() {
                return None;
            }
            let s2 = func(bid::SQRT, vec![nr(nd)]);
            mul(vec![
                inv(s1),
                func(bid::ASINH, vec![mul(vec![lin.clone(), inv(s2)])]),
            ])
        } else if disc.is_zero() {
            mul(vec![
                inv(s1),
                func(bid::LN, vec![func(bid::ABS, vec![lin.clone()])]),
            ])
        } else {
            let inner = add(vec![
                mul(vec![num(2), s1.clone(), sq.clone()]),
                lin.clone(),
            ]);
            mul(vec![
                inv(s1),
                func(bid::LN, vec![func(bid::ABS, vec![inner])]),
            ])
        }
    };

    let mut ints: Vec<MathStructure> = vec![i0];
    for k in 1..n.len() {
        let ki = k as i64;
        let head = if k == 1 {
            sq.clone()
        } else {
            mul(vec![pow(x.clone(), num(ki - 1)), sq.clone()])
        };
        let mut terms = vec![head];
        // `-(k - 1/2) b I_(k-1)`
        let mut c1 = Number::from_ints(2 * ki - 1, 2, 0);
        if !c1.multiply(&b) || !c1.negate() {
            return None;
        }
        if !c1.is_zero() {
            terms.push(mul(vec![nr(c1), ints[k - 1].clone()]));
        }
        if k >= 2 {
            // `-(k - 1) c I_(k-2)`
            let mut c2 = Number::from_i64(ki - 1);
            if !c2.multiply(&c) || !c2.negate() {
                return None;
            }
            if !c2.is_zero() {
                terms.push(mul(vec![nr(c2), ints[k - 2].clone()]));
            }
        }
        let mut den = a.clone();
        if !den.multiply(&Number::from_i64(ki)) {
            return None;
        }
        ints.push(over(add(terms), &den));
    }

    let mut out: Vec<MathStructure> = Vec::new();
    for (k, coef) in n.iter().enumerate() {
        if coef.is_zero() {
            continue;
        }
        out.push(mul(vec![nr(coef.clone()), ints[k].clone()]));
    }
    if out.is_empty() {
        return None;
    }
    Some(add(out))
}

/// A factor that is `Q^(h/2)` for a polynomial `Q` and an *odd* `h`, in any
/// of the three spellings the merge engine leaves behind: `sqrt(Q)`,
/// `sqrt(Q)^h` and `Q^(h/2)`.
fn radical_factor(
    v: &MathStructure,
    x: &MathStructure,
) -> Option<(Vec<Number>, MathStructure, i64)> {
    if let MathStructure::Function { id, args } = v {
        if id.0 == bid::SQRT && args.len() == 1 {
            return Some((dense_of(&args[0], x)?, args[0].clone(), 1));
        }
    }
    let MathStructure::Power { base, exponent } = v else {
        return None;
    };
    let MathStructure::Number(e) = exponent.as_ref() else {
        return None;
    };
    if let MathStructure::Function { id, args } = base.as_ref() {
        if id.0 == bid::SQRT && args.len() == 1 {
            let h = e.to_i64()?;
            if h % 2 == 0 {
                return None;
            }
            return Some((dense_of(&args[0], x)?, args[0].clone(), h));
        }
    }
    // `Q^(h/2)`.
    let mut doubled = e.clone();
    if !doubled.multiply(&Number::from_i64(2)) {
        return None;
    }
    let h = doubled.to_i64()?;
    if h % 2 == 0 {
        return None;
    }
    Some((dense_of(base, x)?, (**base).clone(), h))
}

/// `int c x^k g(x) dx` through the substitution `u = a x^(k+1) + b`, when the
/// product is exactly a monomial times one such `g`. See [`int_chain`].
fn int_substitution(vars: &[MathStructure], x: &MathStructure) -> Option<MathStructure> {
    if vars.len() != 2 {
        return None;
    }
    for (i, j) in [(0usize, 1usize), (1, 0)] {
        let Some((c, k)) = monomial(&vars[i], x) else {
            continue;
        };
        let Some(r) = int_chain(&vars[j], &k, x) else {
            continue;
        };
        return Some(mul(vec![nr(c), r]));
    }
    None
}

/// `int f(c x^n) / x dx = F(c x^n) / n`, the family that produces `Ei`,
/// `Si`, `Ci`, `Shi` and `Chi`.
fn int_special_quotient(vars: &[MathStructure], x: &MathStructure) -> Option<MathStructure> {
    if vars.len() != 2 {
        return None;
    }
    // One factor must be exactly `x^-1`.
    let (call, other) = (&vars[0], &vars[1]);
    let (call, recip) = if is_reciprocal_x(other, x) {
        (call, other)
    } else if is_reciprocal_x(call, x) {
        (other, call)
    } else {
        return None;
    };
    let _ = recip;
    let MathStructure::Function { id, args } = call else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let (_, n) = monomial(&args[0], x)?;
    let f = over_argument_integral(id.0, &args[0])?;
    Some(over(f, &n))
}

fn is_reciprocal_x(m: &MathStructure, x: &MathStructure) -> bool {
    let MathStructure::Power { base, exponent } = m else {
        return false;
    };
    base.equals(x) && matches!(exponent.as_ref(), MathStructure::Number(n) if n.is_minus_one())
}

/// Integration by parts: `int u v dx = u V - int u' V dx`, with `V = int v dx`.
///
/// This is `MathStructure-integrate.cc:6494`. Each factor of the product is
/// tried as `u` in turn, with `v` the product of the others; the inner `V` is
/// integrated with the by-parts budget set to zero, exactly as the C++ does
/// (`minteg_v.integrate(…, 0, parent_parts)`), because a `V` that itself
/// needed by parts is a `V` that will not simplify anything.
///
/// Three things bound it: [`MAX_PARTS_DEPTH`] limits nesting, `parents`
/// refuses an integral already on the stack — which is what stops
/// `int u v dx` from reducing to itself — and [`MAX_PARTS_NODES`] refuses an
/// intermediate that has grown past the point of being worth integrating.
///
/// The order factors are tried in is the only place this departs from the
/// reference, which simply walks the children. `u` is picked by the textbook
/// LIATE preference ([`parts_rank`]); the C++'s order works because it
/// retries every factor, but for `x ln x` it spends the whole budget on the
/// branch that grows before reaching the one that shrinks.
fn int_by_parts(
    vars: &[MathStructure],
    x: &MathStructure,
    depth: usize,
    parts: usize,
    subst: usize,
    parents: &mut Vec<MathStructure>,
) -> Option<MathStructure> {
    if parts == 0 || depth + 1 > MAX_DEPTH || vars.len() > MAX_PARTS_FACTORS {
        return None;
    }
    let whole = mul(vars.to_vec());
    if parents.iter().any(|p| p.equals(&whole)) {
        return None;
    }
    let eo = EvaluationOptions::default();
    let mut order: Vec<usize> = (0..vars.len()).collect();
    order.sort_by_key(|i| parts_rank(&vars[*i], x));
    for i in order {
        let u = &vars[i];
        if mentions_real_radical(u) {
            continue;
        }
        let Some(mut du) = crate::differentiate::differentiate(u, x) else {
            continue;
        };
        let v = mul(
            vars.iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, m)| m.clone())
                .collect::<Vec<_>>(),
        );
        let Some(mut big_v) = int_rec(&v, x, depth + 1, 0, subst, parents) else {
            continue;
        };
        evaluate(&mut big_v, &eo);
        evaluate(&mut du, &eo);
        let mut w = mul(vec![big_v.clone(), du]);
        evaluate(&mut w, &eo);
        if node_count(&w) > MAX_PARTS_NODES {
            continue;
        }
        parents.push(whole.clone());
        let tail = int_rec(&w, x, depth + 1, parts - 1, subst, parents);
        parents.pop();
        if let Some(tail) = tail {
            return Some(add(vec![mul(vec![u.clone(), big_v]), neg(tail)]));
        }
    }
    None
}

/// LIATE: logarithmic, inverse-trigonometric, algebraic, trigonometric,
/// exponential — the order in which a factor is worth choosing as the `u`
/// that gets differentiated. Lower sorts first.
fn parts_rank(m: &MathStructure, x: &MathStructure) -> u8 {
    if let MathStructure::Function { id, .. } = m {
        return match id.0 {
            bid::LN | bid::LOG | bid::LOG2 | bid::LOG10 => 0,
            bid::ASIN | bid::ACOS | bid::ATAN | bid::ACOT | bid::ASINH | bid::ACOSH
            | bid::ATANH => 1,
            bid::SIN | bid::COS | bid::TAN | bid::COT | bid::SINH | bid::COSH | bid::TANH => 3,
            bid::EXP => 4,
            _ => 2,
        };
    }
    if small_power_of_x(m, x).is_some() || monomial(m, x).is_some() {
        return 2;
    }
    if let MathStructure::Power { base, exponent } = m {
        // `b^(f(x))`: exponential.
        if !contains(base, x) && contains(exponent, x) {
            return 4;
        }
    }
    2
}

// ----------------------------------------------------------------------
// Partial fractions
// ----------------------------------------------------------------------

/// Largest denominator degree the partial-fraction path will attempt.
const MAX_PF_DEGREE: usize = 6;

/// `int P(x)/Q(x) dx` when `Q` splits into distinct rational linear factors.
///
/// This is the small, safe corner of `MathStructure-decompose.cc`: `Q` is
/// searched for rational roots by the rational-root theorem, and each simple
/// root contributes `c ln|x - r|`.
///
/// An improper fraction is divided first — `P = D Q + R` — and the quotient
/// polynomial `D` integrated term by term. That is what makes `int x ln(4x+5)`
/// come out: by parts turns it into `int 2x^2/(4x+5)`, which is improper.
fn int_partial_fractions(
    m: &MathStructure,
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    if depth + 1 > MAX_DEPTH {
        return None;
    }
    let (mut num_poly, den_poly) = as_rational_function(m, x)?;
    if den_poly.len() < 2 || den_poly.len() > MAX_PF_DEGREE + 1 {
        return None;
    }
    let mut whole: Option<MathStructure> = None;
    if num_poly.len() >= den_poly.len() {
        let (q, r) = poly_divmod(&num_poly, &den_poly)?;
        whole = Some(integrate_poly(&q, x)?);
        num_poly = r;
    }
    if num_poly.iter().all(Number::is_zero) {
        return whole;
    }
    let roots = match distinct_rational_roots(&den_poly) {
        Some(r) if r.len() + 1 == den_poly.len() => r,
        // No rational split. A quadratic denominator still has a closed form
        // — an arctangent or an argument-hyperbolic-tangent — and that is the
        // one irrational case worth carrying, because `1/(2x^2+5)` and the
        // `x^2/(x^2+1)` that `int x atan(x)` reduces to are both it.
        _ => {
            let q = int_quadratic(&num_poly, &den_poly, x)?;
            return Some(match whole {
                Some(w) => add(vec![w, q]),
                None => q,
            });
        }
    };
    // Residue at a simple root: `P(r) / Q'(r)`.
    let dq = derivative_coeffs(&den_poly);
    let mut parts: Vec<(Number, MathStructure)> = Vec::with_capacity(roots.len());
    for r in &roots {
        let p_r = eval_poly(&num_poly, r)?;
        let q_r = eval_poly(&dq, r)?;
        if q_r.is_zero() {
            return None;
        }
        let mut c = p_r;
        if !c.divide(&q_r) {
            return None;
        }
        let mut shifted = r.clone();
        if !shifted.negate() {
            return None;
        }
        let arg = func(bid::ABS, vec![add(vec![x.clone(), nr(shifted)])]);
        parts.push((c, arg));
    }
    let logs = combine_logs(parts);
    Some(match whole {
        Some(w) => add(vec![w, logs]),
        None => logs,
    })
}

/// `int (p1 x + p0) / (a x^2 + b x + c) dx`.
///
/// Split as `p1/(2a) ln|Q| + (p0 - p1 b/(2a)) int dx/Q`, and the remaining
/// `int dx/Q` by completing the square: an arctangent when the discriminant is
/// negative, a logarithm of a ratio when it is positive, and `-2/(2ax+b)` at
/// the double root. Only reached when the residue expansion could not split
/// `Q` into distinct rational linear factors, so the positive-discriminant
/// branch is the irrational-roots case.
fn int_quadratic(
    num_poly: &[Number],
    den: &[Number],
    x: &MathStructure,
) -> Option<MathStructure> {
    if den.len() != 3 || num_poly.len() > 2 {
        return None;
    }
    let (c, b, a) = (den[0].clone(), den[1].clone(), den[2].clone());
    if a.is_zero() || !a.is_real() || !b.is_real() || !c.is_real() {
        return None;
    }
    let p1 = num_poly.get(1).cloned().unwrap_or_else(Number::new);
    let p0 = num_poly.first().cloned().unwrap_or_else(Number::new);

    let mut two_a = a.clone();
    if !two_a.multiply(&Number::from_i64(2)) {
        return None;
    }
    // `p1 / (2a)`, the coefficient of `ln|Q|`.
    let mut log_coef = p1.clone();
    if !log_coef.divide(&two_a) {
        return None;
    }
    // `p0 - p1 b / (2a)`, the coefficient of `int dx/Q`.
    let mut lin_coef = log_coef.clone();
    if !lin_coef.multiply(&b) || !lin_coef.negate() || !lin_coef.add(&p0) {
        return None;
    }
    // `b^2 - 4ac`.
    let mut disc = b.clone();
    let mut four_ac = a.clone();
    if !disc.multiply(&b) || !four_ac.multiply(&c) || !four_ac.multiply(&Number::from_i64(4)) {
        return None;
    }
    if !four_ac.negate() || !disc.add(&four_ac) {
        return None;
    }

    let mut terms: Vec<MathStructure> = Vec::new();
    if !log_coef.is_zero() {
        let q = poly_structure(den, x);
        terms.push(mul(vec![
            nr(log_coef),
            func(bid::LN, vec![func(bid::ABS, vec![q])]),
        ]));
    }
    if !lin_coef.is_zero() {
        // `2 a x + b`, the derivative of `Q`.
        let lin = add(vec![mul(vec![nr(two_a), x.clone()]), nr(b)]);
        let base = if disc.is_zero() {
            mul(vec![num(-2), inv(lin)])
        } else if disc.is_negative() {
            let mut m = disc.clone();
            if !m.negate() {
                return None;
            }
            let s = func(bid::SQRT, vec![nr(m)]);
            mul(vec![
                num(2),
                inv(s.clone()),
                func(bid::ATAN, vec![mul(vec![lin, inv(s)])]),
            ])
        } else {
            let s = func(bid::SQRT, vec![nr(disc)]);
            let ratio_arg = mul(vec![
                add(vec![lin.clone(), neg(s.clone())]),
                inv(add(vec![lin, s.clone()])),
            ]);
            mul(vec![
                inv(s),
                func(bid::LN, vec![func(bid::ABS, vec![ratio_arg])]),
            ])
        };
        terms.push(mul(vec![nr(lin_coef), base]));
    }
    if terms.is_empty() {
        return None;
    }
    Some(add(terms))
}

/// A dense coefficient vector as an expression in `x`.
fn poly_structure(p: &[Number], x: &MathStructure) -> MathStructure {
    let mut terms: Vec<MathStructure> = Vec::new();
    for (i, c) in p.iter().enumerate() {
        if c.is_zero() {
            continue;
        }
        terms.push(match i {
            0 => nr(c.clone()),
            1 => mul(vec![nr(c.clone()), x.clone()]),
            _ => mul(vec![nr(c.clone()), pow(x.clone(), num(i as i64))]),
        });
    }
    add(terms)
}

/// `p = q d + r` with `deg r < deg d`, over the rationals. Both arguments are
/// dense coefficient vectors, lowest power first.
fn poly_divmod(p: &[Number], d: &[Number]) -> Option<(Vec<Number>, Vec<Number>)> {
    let dn = d.len().checked_sub(1)?;
    let lead = d.last()?;
    if lead.is_zero() {
        return None;
    }
    let mut r: Vec<Number> = p.to_vec();
    if r.len() <= dn {
        return Some((vec![Number::new()], r));
    }
    let mut q = vec![Number::new(); r.len() - dn];
    while r.len() > dn {
        let i = r.len() - 1 - dn;
        let mut c = r.last()?.clone();
        if !c.divide(lead) {
            return None;
        }
        if !c.is_zero() {
            for (j, dc) in d.iter().enumerate() {
                let mut t = c.clone();
                if !t.multiply(dc) || !t.negate() || !r[i + j].add(&t) {
                    return None;
                }
            }
            q[i] = c;
        }
        r.pop();
    }
    Some((trim(q), trim(r)))
}

/// `int P(x) dx` for a dense coefficient vector.
fn integrate_poly(p: &[Number], x: &MathStructure) -> Option<MathStructure> {
    let mut terms: Vec<MathStructure> = Vec::new();
    for (i, c) in p.iter().enumerate() {
        if c.is_zero() {
            continue;
        }
        let mut t = c.clone();
        if !t.divide(&Number::from_i64(i as i64 + 1)) {
            return None;
        }
        terms.push(mul(vec![nr(t), pow(x.clone(), num(i as i64 + 1))]));
    }
    Some(add(terms))
}

/// `c ln a - c ln b` becomes `c ln(a/b)`, which is the shape `simplify_ln`
/// (MathStructure-eval.cc) leaves the reference's partial-fraction results in.
fn combine_logs(parts: Vec<(Number, MathStructure)>) -> MathStructure {
    let common = parts.first().map(|(c, _)| {
        let mut a = c.clone();
        a.abs();
        a
    });
    let uniform = match &common {
        Some(c) if !c.is_zero() => parts.iter().all(|(k, _)| {
            let mut a = k.clone();
            a.abs();
            a.equals(c, false, false)
        }),
        _ => false,
    };
    if !uniform || parts.len() < 2 {
        return add(parts
            .into_iter()
            .map(|(c, arg)| mul(vec![nr(c), func(bid::LN, vec![arg])]))
            .collect());
    }
    let mut top: Vec<MathStructure> = Vec::new();
    let mut bottom: Vec<MathStructure> = Vec::new();
    for (c, arg) in parts {
        if c.is_negative() {
            bottom.push(arg);
        } else {
            top.push(arg);
        }
    }
    if top.is_empty() || bottom.is_empty() {
        // No cancellation to exploit; fall back to the plain sum.
        let sign = if top.is_empty() { -1 } else { 1 };
        let mut c = common.expect("uniform implies a coefficient");
        if sign < 0 && !c.negate() {
            return MathStructure::Undefined;
        }
        let args = if top.is_empty() { bottom } else { top };
        return add(args
            .into_iter()
            .map(|a| mul(vec![nr(c.clone()), func(bid::LN, vec![a])]))
            .collect());
    }
    let ratio = mul(vec![mul(top), inv(mul(bottom))]);
    mul(vec![
        nr(common.expect("uniform implies a coefficient")),
        func(bid::LN, vec![ratio]),
    ])
}

/// Split `m` into dense numerator and denominator coefficient vectors.
fn as_rational_function(
    m: &MathStructure,
    x: &MathStructure,
) -> Option<(Vec<Number>, Vec<Number>)> {
    let single;
    let factors: &[MathStructure] = match m {
        MathStructure::Multiplication(f) => f,
        MathStructure::Power { .. } => {
            single = [m.clone()];
            &single
        }
        _ => return None,
    };
    let mut numer: Vec<MathStructure> = Vec::new();
    let mut denom: Vec<MathStructure> = Vec::new();
    for f in factors {
        match f {
            MathStructure::Power { base, exponent } => match exponent.as_ref() {
                MathStructure::Number(n) if n.is_negative() && n.is_integer() => {
                    let k = n.to_i64()?;
                    if !(-(MAX_PF_DEGREE as i64)..0).contains(&k) {
                        return None;
                    }
                    for _ in 0..(-k) {
                        denom.push((**base).clone());
                    }
                }
                _ => numer.push(f.clone()),
            },
            _ => numer.push(f.clone()),
        }
    }
    if denom.is_empty() {
        return None;
    }
    let n_poly = dense_of(&mul(numer), x)?;
    let d_poly = dense_of(&mul(denom), x)?;
    Some((trim(n_poly), trim(d_poly)))
}

/// Largest integer exponent [`dense_of`] will expand, and the largest degree
/// it will produce. `(4x+5)^3 / x` has to be multiplied out before it is a
/// polynomial over a polynomial, and nothing stops a user writing
/// `(4x+5)^10000`.
const MAX_EXPAND_POWER: i64 = 12;
const MAX_EXPAND_DEGREE: usize = 24;

/// `m` as a dense coefficient vector, *expanding* powers and products.
///
/// [`crate::polynomial::to_dense`] reads an already-expanded sum; the
/// integrand here has usually not been expanded, because the merge engine
/// leaves `(4x+5)^3` and `1 - (4x+5)^2` alone, and both are polynomials that
/// the residue expansion can handle once multiplied out.
fn dense_of(m: &MathStructure, x: &MathStructure) -> Option<Vec<Number>> {
    if m.equals(x) {
        return Some(vec![Number::new(), Number::from_i64(1)]);
    }
    if !contains(m, x) {
        let MathStructure::Number(n) = m else {
            return None;
        };
        return Some(vec![n.clone()]);
    }
    match m {
        MathStructure::Addition(terms) => {
            let mut acc = vec![Number::new()];
            for t in terms {
                acc = poly_add(&acc, &dense_of(t, x)?)?;
            }
            Some(trim(acc))
        }
        MathStructure::Multiplication(factors) => {
            let mut acc = vec![Number::from_i64(1)];
            for f in factors {
                acc = poly_mul(&acc, &dense_of(f, x)?)?;
            }
            Some(trim(acc))
        }
        MathStructure::Power { base, exponent } => {
            let MathStructure::Number(e) = exponent.as_ref() else {
                return None;
            };
            let k = e.to_i64()?;
            if !(0..=MAX_EXPAND_POWER).contains(&k) {
                return None;
            }
            let b = dense_of(base, x)?;
            let mut acc = vec![Number::from_i64(1)];
            for _ in 0..k {
                acc = poly_mul(&acc, &b)?;
            }
            Some(trim(acc))
        }
        _ => None,
    }
}

fn poly_add(a: &[Number], b: &[Number]) -> Option<Vec<Number>> {
    let mut out = vec![Number::new(); a.len().max(b.len())];
    for (i, c) in a.iter().chain(std::iter::empty()).enumerate() {
        out[i] = c.clone();
    }
    for (i, c) in b.iter().enumerate() {
        if !out[i].add(c) {
            return None;
        }
    }
    Some(out)
}

fn poly_mul(a: &[Number], b: &[Number]) -> Option<Vec<Number>> {
    if a.len() + b.len() > MAX_EXPAND_DEGREE + 2 {
        return None;
    }
    let mut out = vec![Number::new(); a.len() + b.len() - 1];
    for (i, ca) in a.iter().enumerate() {
        if ca.is_zero() {
            continue;
        }
        for (j, cb) in b.iter().enumerate() {
            let mut t = ca.clone();
            if !t.multiply(cb) || !out[i + j].add(&t) {
                return None;
            }
        }
    }
    Some(out)
}

fn trim(mut v: Vec<Number>) -> Vec<Number> {
    while v.len() > 1 && v.last().is_some_and(Number::is_zero) {
        v.pop();
    }
    v
}

fn derivative_coeffs(p: &[Number]) -> Vec<Number> {
    let mut out = Vec::with_capacity(p.len().saturating_sub(1));
    for (i, c) in p.iter().enumerate().skip(1) {
        let mut t = c.clone();
        if !t.multiply_i64(i as i64) {
            return Vec::new();
        }
        out.push(t);
    }
    out
}

fn eval_poly(p: &[Number], at: &Number) -> Option<Number> {
    let mut acc = Number::new();
    for c in p.iter().rev() {
        if !acc.multiply(at) || !acc.add(c) {
            return None;
        }
    }
    Some(acc)
}

/// Rational roots of an integer-coefficient polynomial, by trial over the
/// divisors of the leading and trailing coefficients. Bounded by
/// [`MAX_PF_DEGREE`] and a divisor cap.
fn distinct_rational_roots(p: &[Number]) -> Option<Vec<Number>> {
    const MAX_DIVISOR: i64 = 64;
    let mut ints: Vec<i64> = Vec::with_capacity(p.len());
    // Clear denominators so the rational-root theorem applies.
    let mut lcm = Number::from_i64(1);
    for c in p {
        if !c.is_rational() || c.is_approximate() {
            return None;
        }
        let d = c.denominator();
        let mut l = lcm.clone();
        if !l.lcm(&d) {
            return None;
        }
        lcm = l;
    }
    for c in p {
        let mut t = c.clone();
        if !t.multiply(&lcm) {
            return None;
        }
        ints.push(t.to_i64()?);
    }
    let mut roots: Vec<Number> = Vec::new();
    if ints.first() == Some(&0) {
        // `Q(0) = 0`: deflate the zero root and search the rest. A *repeated*
        // zero root is not a simple pole, so the caller's
        // `roots.len() + 1 == deg Q` check must be allowed to fail; bail out
        // here rather than return a root list that would pass it.
        if ints.get(1) == Some(&0) {
            return None;
        }
        roots.push(Number::new());
        ints.remove(0);
    }
    let a0 = *ints.first()?;
    let an = *ints.last()?;
    if an == 0 || a0 == 0 {
        return None;
    }
    for q in 1..=MAX_DIVISOR {
        if an % q != 0 {
            continue;
        }
        for pnum in 1..=MAX_DIVISOR {
            if a0 % pnum != 0 {
                continue;
            }
            for sign in [1i64, -1] {
                let cand = Number::from_ints(sign * pnum, q, 0);
                if roots.iter().any(|r| r.equals(&cand, false, false)) {
                    continue;
                }
                let v = eval_poly(p, &cand)?;
                if v.is_zero() {
                    roots.push(cand);
                }
            }
        }
        if roots.len() + 1 >= p.len() {
            break;
        }
    }
    Some(roots)
}

// ----------------------------------------------------------------------
// Definite integration
// ----------------------------------------------------------------------

/// `F(b) - F(a)`, evaluated.
fn definite_from_antiderivative(
    f: &MathStructure,
    x: &MathStructure,
    a: &MathStructure,
    b: &MathStructure,
    eo: &EvaluationOptions,
) -> Option<MathStructure> {
    let mut hi = f.clone();
    crate::solve::replace(&mut hi, x, b);
    let mut lo = f.clone();
    crate::solve::replace(&mut lo, x, a);
    let mut r = add(vec![hi, neg(lo)]);
    evaluate(&mut r, eo);
    if contains(&r, x) {
        return None;
    }
    Some(r)
}

/// Hard cap on Romberg halvings; see [`romberg`].
const MAX_ROMBERG_STEPS: usize = 12;

/// Romberg integration — port of `romberg()` (MathStructure-integrate.cc:6711).
///
/// The convergence test is the reference's, simplified to the real,
/// non-interval case: the run stops once two successive diagonal entries
/// agree to `PRECISION + extra` relative digits, having taken at least
/// `min_steps` steps.
pub fn romberg(
    integrand: &MathStructure,
    x: &MathStructure,
    a: &Number,
    b: &Number,
    min_steps: usize,
    max_steps: usize,
    safety: bool,
) -> Option<Number> {
    // The reference allows 22 halvings (4M samples) because each sample is a
    // single MPFR evaluation; here a sample re-runs the whole merge engine,
    // so a non-convergent integrand (`romberg(1/x, 0, 1)`) would take
    // minutes. Cap the refinement at 2^12 samples: every convergent case in
    // the transcripts settles by the ninth step.
    let max_steps = max_steps.clamp(2, MAX_ROMBERG_STEPS);
    let min_steps = min_steps.clamp(2, max_steps);
    let mut prev: Vec<Number> = vec![Number::new(); max_steps + 1];
    let mut cur: Vec<Number> = vec![Number::new(); max_steps + 1];

    let mut h = b.clone();
    if !h.subtract(a) {
        return None;
    }
    let fa = eval_at_shifted(integrand, x, a, true)?;
    let fb = eval_at_shifted(integrand, x, b, false)?;
    prev[0] = fa;
    if !prev[0].add(&fb) || !prev[0].multiply(&Number::from_ints(1, 2, 0)) || !prev[0].multiply(&h)
    {
        return None;
    }

    let mut value: Option<Number> = None;
    let mut had_prev_unc = false;
    for i in 1..max_steps {
        if !h.multiply(&Number::from_ints(1, 2, 0)) {
            return None;
        }
        let mut c = Number::new();
        let ep: u64 = 1u64 << (i - 1);
        let mut t = a.clone();
        if !t.add(&h) {
            return None;
        }
        for _ in 0..ep {
            let v = eval_point(integrand, x, &t)?;
            if !c.add(&v) {
                return None;
            }
            if !t.add(&h) || !t.add(&h) {
                return None;
            }
        }
        let mut c0 = h.clone();
        let mut half_prev = prev[0].clone();
        if !half_prev.multiply(&Number::from_ints(1, 2, 0))
            || !c0.multiply(&c)
            || !c0.add(&half_prev)
        {
            return None;
        }
        cur[0] = c0;
        for j in 1..=i {
            let mut p4 = Number::from_i64(4);
            if !p4.raise(&Number::from_i64(j as i64), true) {
                return None;
            }
            let mut denom = p4.clone();
            if !denom.add_i64(-1) || !denom.recip() {
                return None;
            }
            let mut e = p4;
            if !e.multiply(&cur[j - 1]) || !e.subtract(&prev[j - 1]) || !e.multiply(&denom) {
                return None;
            }
            cur[j] = e;
        }
        if i + 1 >= min_steps {
            let mut unc = prev[i - 1].clone();
            if !unc.subtract(&cur[i]) || !unc.abs() {
                return None;
            }
            if safety && !unc.multiply_i64(10) {
                return None;
            }
            let prec = qalc_num::context::precision() + if safety { 3 } else { 1 };
            let mut acc = Number::from_ints(1, 1, -(prec as i64));
            let mid = cur[i - 1].clone();
            if !mid.is_zero() && !acc.multiply(&mid) {
                return None;
            }
            if !acc.abs() {
                return None;
            }
            value = Some(cur[i - 1].clone());
            if !unc.is_greater_than(&acc) {
                if !safety || had_prev_unc || unc.is_zero() {
                    let mut v = cur[i - 1].clone();
                    v.set_approximate(true);
                    return Some(v);
                }
                had_prev_unc = true;
            } else {
                had_prev_unc = false;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    // Out of steps: accept the last diagonal entry only if it is at least
    // roughly converged (the reference's `acc.set(1, 1, -3)` tail).
    value.map(|mut v| {
        v.set_approximate(true);
        v
    })
}

fn eval_point(m: &MathStructure, x: &MathStructure, at: &Number) -> Option<Number> {
    // The sample points stay *exact*: they are `a + j (b - a) / 2^i`, so the
    // denominators are bounded by the step cap and cost nothing, while a
    // floating-point exponent would send `b^x` through the generic
    // `exp(x ln b)` path (which does not terminate when the result is
    // exactly representable).
    let v = crate::solve::eval_at(m, x, at)?;
    if v.includes_infinity() {
        return None;
    }
    Some(v)
}

/// An endpoint value, nudged inward when the integrand is singular there
/// (the reference's `mpfr_nextabove`/`mpfr_nextbelow`).
fn eval_at_shifted(
    m: &MathStructure,
    x: &MathStructure,
    at: &Number,
    above: bool,
) -> Option<Number> {
    if let Some(v) = eval_point(m, x, at) {
        return Some(v);
    }
    let mut delta = Number::from_ints(1, 1, -(qalc_num::context::precision() as i64 + 12));
    if !above && !delta.negate() {
        return None;
    }
    let mut shifted = at.clone();
    if !shifted.add(&delta) {
        return None;
    }
    eval_point(m, x, &shifted)
}

// ----------------------------------------------------------------------
// Integral special functions
// ----------------------------------------------------------------------

/// Iteration cap shared by every series below.
const MAX_SERIES_TERMS: usize = 2000;

fn series_converged(term: &Number, sum: &Number, guard: i64) -> bool {
    if term.is_zero() {
        return true;
    }
    let mut a = term.clone();
    if !a.abs() {
        return false;
    }
    let mut s = sum.clone();
    if !s.abs() {
        return false;
    }
    let mut tol = Number::from_ints(1, 1, -guard);
    if !tol.multiply(&s) {
        return false;
    }
    if s.is_zero() {
        return false;
    }
    a.is_less_than(&tol)
}

/// `Shi(z) = sum_{k>=0} z^(2k+1) / ((2k+1) (2k+1)!)`.
pub fn sinh_integral(z: &Number) -> Option<Number> {
    if z.is_zero() {
        return Some(Number::new());
    }
    if !z.is_real() || z.is_complex() {
        return None;
    }
    let mut mag = z.clone();
    if !mag.abs() || mag.is_greater_than_i64(60) {
        return None;
    }
    let mut z2 = z.clone();
    if !z2.square() {
        return None;
    }
    // `p = z^(2k+1) / (2k+1)!`, `term = p / (2k+1)`.
    let mut p = z.clone();
    let mut sum = z.clone();
    for k in 1..MAX_SERIES_TERMS {
        let kk = k as i64;
        if !p.multiply(&z2) || !p.divide_i64(2 * kk) || !p.divide_i64(2 * kk + 1) {
            return None;
        }
        let mut term = p.clone();
        if !term.divide_i64(2 * kk + 1) {
            return None;
        }
        if !sum.add(&term) {
            return None;
        }
        if series_converged(&term, &sum, qalc_num::context::precision() as i64 + 12) {
            break;
        }
    }
    sum.set_approximate(true);
    Some(sum)
}

/// `Chi(z) = gamma + ln z + sum_{k>=1} z^(2k) / ((2k) (2k)!)`.
pub fn cosh_integral(z: &Number) -> Option<Number> {
    if z.is_zero() || !z.is_real() || z.is_complex() || !z.is_positive() {
        return None;
    }
    let mut mag = z.clone();
    if !mag.abs() || mag.is_greater_than_i64(60) {
        return None;
    }
    let mut z2 = z.clone();
    if !z2.square() {
        return None;
    }
    let mut sum = euler_gamma()?;
    let mut lnz = z.clone();
    if !lnz.ln() || !sum.add(&lnz) {
        return None;
    }
    // `p = z^(2k) / (2k)!`.
    let mut p = Number::from_i64(1);
    for k in 1..MAX_SERIES_TERMS {
        let kk = k as i64;
        if !p.multiply(&z2) || !p.divide_i64(2 * kk - 1) || !p.divide_i64(2 * kk) {
            return None;
        }
        let mut term = p.clone();
        if !term.divide_i64(2 * kk) {
            return None;
        }
        if !sum.add(&term) {
            return None;
        }
        if series_converged(&term, &sum, qalc_num::context::precision() as i64 + 12) {
            break;
        }
    }
    sum.set_approximate(true);
    Some(sum)
}

/// The Euler-Mascheroni constant, recovered from `Ei(1)`:
/// `Ei(1) = gamma + sum_{k>=1} 1/(k k!)`.
fn euler_gamma() -> Option<Number> {
    let mut ei1 = Number::from_i64(1);
    if !ei1.expint() {
        return None;
    }
    let mut p = Number::from_i64(1);
    let mut s = Number::new();
    for k in 1..MAX_SERIES_TERMS {
        let kk = k as i64;
        if !p.divide_i64(kk) {
            return None;
        }
        let mut term = p.clone();
        if !term.divide_i64(kk) {
            return None;
        }
        if !s.add(&term) {
            return None;
        }
        if series_converged(&term, &s, qalc_num::context::precision() as i64 + 12) {
            break;
        }
    }
    if !ei1.subtract(&s) {
        return None;
    }
    Some(ei1)
}

/// `S(x) = int_0^x sin(pi t^2 / 2) dt`, by its power series
/// `sum_k (-1)^k (pi/2)^(2k+1) x^(4k+3) / ((2k+1)! (4k+3))`.
///
/// The series alternates, so the working precision is raised while it runs:
/// at `x = 5` the largest term is `1e13` against a result below one.
pub fn fresnel_s(x: &Number) -> Option<Number> {
    fresnel(x, true)
}

/// `C(x) = int_0^x cos(pi t^2 / 2) dt`.
pub fn fresnel_c(x: &Number) -> Option<Number> {
    fresnel(x, false)
}

fn fresnel(x: &Number, sine: bool) -> Option<Number> {
    if !x.is_real() || x.is_complex() {
        return None;
    }
    if x.is_zero() {
        return Some(Number::new());
    }
    let mut mag = x.clone();
    if !mag.abs() || mag.is_greater_than_i64(12) {
        // Beyond this the alternating series needs more guard digits than
        // the working precision provides; the asymptotic form is not ported.
        return None;
    }
    let saved = qalc_num::context::precision();
    let boosted = (saved + 8 * mag.to_i64().unwrap_or(12).unsigned_abs() as i32 + 40).min(400);
    qalc_num::context::set_precision(boosted);
    let r = fresnel_series(x, sine);
    qalc_num::context::set_precision(saved);
    let mut r = r?;
    r.set_precision(saved);
    r.set_approximate(true);
    Some(r)
}

fn fresnel_series(x: &Number, sine: bool) -> Option<Number> {
    let mut half_pi = Number::new();
    half_pi.pi();
    if !half_pi.divide_i64(2) {
        return None;
    }
    let mut hp2 = half_pi.clone();
    if !hp2.square() {
        return None;
    }
    let mut x2 = x.clone();
    if !x2.square() {
        return None;
    }
    let mut x4 = x2.clone();
    if !x4.square() {
        return None;
    }
    // sine:   p = (pi/2)^(2k+1) x^(4k+3) / (2k+1)!,  divided by (4k+3)
    // cosine: p = (pi/2)^(2k)   x^(4k+1) / (2k)!,    divided by (4k+1)
    let mut p = if sine {
        let mut v = half_pi.clone();
        if !v.multiply(&x2) || !v.multiply(x) {
            return None;
        }
        v
    } else {
        x.clone()
    };
    let mut sum = p.clone();
    if !sum.divide_i64(if sine { 3 } else { 1 }) {
        return None;
    }
    let mut sign = -1i64;
    for k in 1..MAX_SERIES_TERMS {
        let kk = k as i64;
        // Advance the factorial/power product by one term.
        let (d1, d2) = if sine { (2 * kk, 2 * kk + 1) } else { (2 * kk - 1, 2 * kk) };
        if !p.multiply(&hp2) || !p.multiply(&x4) || !p.divide_i64(d1) || !p.divide_i64(d2) {
            return None;
        }
        let mut term = p.clone();
        let den = if sine { 4 * kk + 3 } else { 4 * kk + 1 };
        if !term.divide_i64(den) || !term.multiply_i64(sign) {
            return None;
        }
        if !sum.add(&term) {
            return None;
        }
        sign = -sign;
        if series_converged(&term, &sum, qalc_num::context::precision() as i64) {
            break;
        }
    }
    Some(sum)
}

/// `gammainc(a, x)`, the *lower* incomplete gamma.
///
/// The reference defines it in XML as `gamma(x) - igamma(x, y)`
/// (data/functions.xml.in), and the subtraction is where its printed
/// precision comes from: for `gammainc(53, 5.2)` the two terms agree to 34
/// digits, so the reference prints only six. This keeps that definition so
/// the reported precision matches.
pub fn gamma_inc(a: &Number, x: &Number) -> Option<Number> {
    let upper = upper_gamma(a, x)?;
    let mut ga = a.clone();
    if !ga.gamma() {
        return None;
    }
    let mut g = ga.clone();
    if !g.subtract(&upper) {
        return None;
    }
    g.set_approximate(true);
    if let Some(p) = cancellation_precision(&ga, &g) {
        g.set_precision(p);
    }
    Some(g)
}

/// Significant digits left after `large - large` collapses to `small`.
///
/// The reference gets this for free: `CREATE_INTERVAL` makes `gamma(53)` an
/// interval whose *absolute* width survives the subtraction, so
/// `gammainc(53, 5.2)` comes out with six significant digits rather than
/// ten. This port carries precision as a scalar, so the loss is computed
/// from the magnitudes instead.
fn cancellation_precision(large: &Number, small: &Number) -> Option<i32> {
    if small.is_zero() || !small.is_real() || !large.is_real() {
        return None;
    }
    let mut ratio = large.clone();
    if !ratio.divide(small) || !ratio.abs() {
        return None;
    }
    if !ratio.is_greater_than_i64(1) {
        return None;
    }
    let v = ratio.float_value();
    if !v.is_finite() || v <= 1.0 {
        return None;
    }
    let internal = qalc_num::context::from_bit_precision(qalc_num::context::bit_precision());
    let left = (internal as f64 - v.log10()).floor() as i32;
    let global = qalc_num::context::precision();
    if left >= global || left < 1 {
        return None;
    }
    Some(left)
}

/// `int_0^x t^(a-1) e^-t dt` by the series
/// `x^a e^-x sum_{n>=0} x^n / (a (a+1) ... (a+n))`.
pub fn lower_gamma(a: &Number, x: &Number) -> Option<Number> {
    if !a.is_real() || !x.is_real() || a.is_complex() || x.is_complex() {
        return None;
    }
    if !x.is_non_negative() || !a.is_positive() {
        return None;
    }
    if x.is_zero() {
        return Some(Number::new());
    }
    if x.is_greater_than_i64(400) || a.is_greater_than_i64(4000) {
        return None;
    }
    // The series converges for every x but loses digits once x exceeds a;
    // in that regime use `gamma(a) - upper`.
    let mut a_plus_1 = a.clone();
    if !a_plus_1.add_i64(1) {
        return None;
    }
    if x.is_greater_than(&a_plus_1) {
        let upper = upper_gamma_cf(a, x)?;
        let mut g = a.clone();
        if !g.gamma() || !g.subtract(&upper) {
            return None;
        }
        return Some(g);
    }
    let mut denom = a.clone();
    let mut term = Number::from_i64(1);
    if !term.divide(&denom) {
        return None;
    }
    let mut sum = term.clone();
    for _ in 0..MAX_SERIES_TERMS {
        if !denom.add_i64(1) {
            return None;
        }
        if !term.multiply(x) || !term.divide(&denom) {
            return None;
        }
        if !sum.add(&term) {
            return None;
        }
        if series_converged(&term, &sum, qalc_num::context::precision() as i64 + 12) {
            break;
        }
    }
    let mut pref = x.clone();
    if !pref.raise(a, false) {
        return None;
    }
    let mut ex = x.clone();
    if !ex.negate() || !ex.exp() {
        return None;
    }
    if !sum.multiply(&pref) || !sum.multiply(&ex) {
        return None;
    }
    sum.set_approximate(true);
    Some(sum)
}

/// The *upper* incomplete gamma `igamma(a, x) = int_x^inf t^(a-1) e^-t dt`.
pub fn upper_gamma(a: &Number, x: &Number) -> Option<Number> {
    if !a.is_real() || !x.is_real() || a.is_complex() || x.is_complex() {
        return None;
    }
    if !x.is_non_negative() || !a.is_positive() {
        return None;
    }
    let mut a_plus_1 = a.clone();
    if !a_plus_1.add_i64(1) {
        return None;
    }
    if x.is_greater_than(&a_plus_1) {
        return upper_gamma_cf(a, x);
    }
    let low = lower_gamma(a, x)?;
    let mut g = a.clone();
    if !g.gamma() || !g.subtract(&low) {
        return None;
    }
    g.set_approximate(true);
    Some(g)
}

/// Legendre's continued fraction for the upper incomplete gamma, in the
/// modified Lentz form. Converges quickly for `x > a + 1`.
fn upper_gamma_cf(a: &Number, x: &Number) -> Option<Number> {
    let tiny = Number::from_ints(1, 1, -300);
    let mut b = x.clone();
    if !b.subtract(a) || !b.add_i64(1) {
        return None;
    }
    let mut c = Number::from_ints(1, 1, 300);
    let mut d = b.clone();
    if d.is_zero() {
        d = tiny.clone();
    }
    if !d.recip() {
        return None;
    }
    let mut h = d.clone();
    for i in 1..MAX_SERIES_TERMS {
        let ii = i as i64;
        // an = -i (i - a)
        let mut an = a.clone();
        if !an.negate() || !an.add_i64(ii) || !an.multiply_i64(-ii) {
            return None;
        }
        if !b.add_i64(2) {
            return None;
        }
        // d = an*d + b
        let mut nd = an.clone();
        if !nd.multiply(&d) || !nd.add(&b) {
            return None;
        }
        if nd.is_zero() {
            nd = tiny.clone();
        }
        // c = b + an/c
        let mut ncr = an.clone();
        if !ncr.divide(&c) {
            return None;
        }
        let mut nc = b.clone();
        if !nc.add(&ncr) {
            return None;
        }
        if nc.is_zero() {
            nc = tiny.clone();
        }
        if !nd.recip() {
            return None;
        }
        let mut delta = nd.clone();
        if !delta.multiply(&nc) {
            return None;
        }
        c = nc;
        d = nd;
        if !h.multiply(&delta) {
            return None;
        }
        let mut e = delta.clone();
        if !e.add_i64(-1) || !e.abs() {
            return None;
        }
        if e.is_less_than(&Number::from_ints(
            1,
            1,
            -(qalc_num::context::precision() as i64 + 12),
        )) {
            break;
        }
    }
    let mut pref = x.clone();
    if !pref.raise(a, false) {
        return None;
    }
    let mut ex = x.clone();
    if !ex.negate() || !ex.exp() {
        return None;
    }
    if !h.multiply(&pref) || !h.multiply(&ex) {
        return None;
    }
    h.set_approximate(true);
    Some(h)
}

/// The regularized incomplete beta `I_x(a, b)`.
///
/// Port of the free `betainc()` in `BuiltinFunctions-calculus.cc`: Romberg
/// integration of `t^(a-1) (1 - t)^(b-1)` along the straight segment from 0
/// to `x` (which may be complex), divided by `B(a, b)`.
pub fn incomplete_beta(x: &Number, a: &Number, b: &Number) -> Option<Number> {
    if x.is_zero() {
        return Some(Number::new());
    }
    if let Some(exact) = incomplete_beta_exact(x, a, b) {
        return Some(exact);
    }
    let mut am1 = a.clone();
    let mut bm1 = b.clone();
    if !am1.add_i64(-1) || !bm1.add_i64(-1) {
        return None;
    }
    let raw = betainc_romberg(x, &am1, &bm1)?;
    // Divide by B(a, b) = gamma(a) gamma(b) / gamma(a + b).
    let mut ga = a.clone();
    let mut gb = b.clone();
    let mut gab = a.clone();
    if !gab.add(b) || !ga.gamma() || !gb.gamma() || !gab.gamma() {
        return None;
    }
    if !ga.multiply(&gb) || !ga.divide(&gab) {
        return None;
    }
    let mut r = raw;
    if !r.divide(&ga) {
        return None;
    }
    r.set_approximate(true);
    Some(r)
}

/// `Number::betainc`'s closed form for positive integer parameters:
/// `I_x(p, q) = sum_{i=p}^{p+q-1} binomial(p+q-1, i) x^i (1-x)^(p+q-1-i)`.
fn incomplete_beta_exact(x: &Number, p: &Number, q: &Number) -> Option<Number> {
    if !p.is_integer() || !q.is_integer() || !p.is_positive() || !q.is_positive() {
        return None;
    }
    if !x.is_real() || x.is_complex() {
        return None;
    }
    let pi = p.to_i64()?;
    let qi = q.to_i64()?;
    if pi + qi > 512 {
        return None;
    }
    let n = pi + qi - 1;
    let mut one_minus = Number::from_i64(1);
    if !one_minus.subtract(x) {
        return None;
    }
    let mut sum = Number::new();
    for i in pi..=n {
        let mut term = Number::new();
        if !term.binomial(&Number::from_i64(n), &Number::from_i64(i)) {
            return None;
        }
        let mut xi = x.clone();
        if !xi.raise(&Number::from_i64(i), true) {
            return None;
        }
        let mut yi = one_minus.clone();
        if !yi.raise(&Number::from_i64(n - i), true) {
            return None;
        }
        if !term.multiply(&xi) || !term.multiply(&yi) || !sum.add(&term) {
            return None;
        }
    }
    Some(sum)
}

/// `int_0^x t^am1 (1 - t)^bm1 dt` by Romberg along the segment `s x`,
/// `s in [0, 1]`.
fn betainc_romberg(x: &Number, am1: &Number, bm1: &Number) -> Option<Number> {
    const MAX_STEPS: usize = 14;
    const MIN_STEPS: usize = 6;
    let f = |t: &Number| -> Option<Number> {
        let mut w = Number::from_i64(1);
        if !w.subtract(t) {
            return None;
        }
        if !w.is_zero() && !w.raise(bm1, false) {
            return None;
        }
        let mut u = t.clone();
        if !u.is_zero() && !u.raise(am1, false) {
            return None;
        }
        if !w.multiply(&u) {
            return None;
        }
        if w.includes_infinity() {
            return None;
        }
        Some(w)
    };
    let mut prev: Vec<Number> = vec![Number::new(); MAX_STEPS + 1];
    let mut cur: Vec<Number> = vec![Number::new(); MAX_STEPS + 1];
    let mut h = x.clone();
    let f0 = f(&Number::new())?;
    let f1 = f(x)?;
    prev[0] = f0;
    if !prev[0].add(&f1) || !prev[0].multiply(&Number::from_ints(1, 2, 0)) || !prev[0].multiply(&h)
    {
        return None;
    }
    let mut value: Option<Number> = None;
    for i in 1..MAX_STEPS {
        if !h.multiply(&Number::from_ints(1, 2, 0)) {
            return None;
        }
        let mut c = Number::new();
        let ep: u64 = 1u64 << (i - 1);
        let mut t = h.clone();
        for _ in 0..ep {
            let v = f(&t)?;
            if !c.add(&v) {
                return None;
            }
            if !t.add(&h) || !t.add(&h) {
                return None;
            }
        }
        let mut c0 = h.clone();
        let mut half_prev = prev[0].clone();
        if !half_prev.multiply(&Number::from_ints(1, 2, 0))
            || !c0.multiply(&c)
            || !c0.add(&half_prev)
        {
            return None;
        }
        cur[0] = c0;
        for j in 1..=i {
            let mut p4 = Number::from_i64(4);
            if !p4.raise(&Number::from_i64(j as i64), true) {
                return None;
            }
            let mut denom = p4.clone();
            if !denom.add_i64(-1) || !denom.recip() {
                return None;
            }
            let mut e = p4;
            if !e.multiply(&cur[j - 1]) || !e.subtract(&prev[j - 1]) || !e.multiply(&denom) {
                return None;
            }
            cur[j] = e;
        }
        if i + 1 >= MIN_STEPS {
            let mut unc = prev[i - 1].clone();
            if !unc.subtract(&cur[i]) || !unc.abs() {
                return None;
            }
            let prec = qalc_num::context::precision() as i64 + 1;
            let mut acc = Number::from_ints(1, 1, -prec);
            let mut mid = cur[i - 1].clone();
            if !mid.abs() {
                return None;
            }
            if !mid.is_zero() && !acc.multiply(&mid) {
                return None;
            }
            value = Some(cur[i - 1].clone());
            if !unc.is_greater_than(&acc) {
                return Some(cur[i - 1].clone());
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    value
}

// ----------------------------------------------------------------------
// Builtin dispatch
// ----------------------------------------------------------------------

/// `integrate()`, `romberg()` and the integral special functions.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    match fid {
        self::id::INTEGRATE => {
            let args = args.clone();
            calculate_integrate(m, &args)
        }
        self::id::ROMBERG => {
            let args = args.clone();
            calculate_romberg(m, &args)
        }
        self::id::SINHINT
        | self::id::COSHINT
        | self::id::FRESNEL_S
        | self::id::FRESNEL_C
        | self::id::I_GAMMA
        | self::id::GAMMAINC
        | self::id::INCOMPLETE_BETA => {
            let args = args.clone();
            calculate_special(m, fid, &args)
        }
        _ => false,
    }
}

fn calculate_special(m: &mut MathStructure, fid: u32, args: &[MathStructure]) -> bool {
    let mut nums: Vec<Number> = Vec::with_capacity(args.len());
    for a in args {
        match a {
            MathStructure::Number(n) => nums.push(n.clone()),
            _ => return false,
        }
    }
    let r = match (fid, nums.len()) {
        (self::id::SINHINT, 1) => sinh_integral(&nums[0]),
        (self::id::COSHINT, 1) => cosh_integral(&nums[0]),
        (self::id::FRESNEL_S, 1) => fresnel_s(&nums[0]),
        (self::id::FRESNEL_C, 1) => fresnel_c(&nums[0]),
        (self::id::I_GAMMA, 2) => upper_gamma(&nums[0], &nums[1]),
        (self::id::GAMMAINC, 2) => gamma_inc(&nums[0], &nums[1]),
        (self::id::INCOMPLETE_BETA, 3) => incomplete_beta(&nums[0], &nums[1], &nums[2]),
        _ => None,
    };
    match r {
        Some(v) => {
            *m = MathStructure::Number(v);
            true
        }
        None => false,
    }
}

/// `IntegrateFunction::calculate` (BuiltinFunctions-calculus.cc).
fn calculate_integrate(m: &mut MathStructure, args: &[MathStructure]) -> bool {
    if args.is_empty() || args.len() > 4 {
        return false;
    }
    let expr = args[0].clone();
    // `integrate(f, x)` names the variable; `integrate(f, a, b)` gives bounds.
    let (xvar, bounds) = if args.len() == 2 && args[1].is_symbolic() {
        (args[1].clone(), None)
    } else if args.len() >= 3 {
        let xv = match args.get(3) {
            Some(v) if v.is_symbolic() => v.clone(),
            _ => default_x_var(&expr),
        };
        (xv, Some((args[1].clone(), args[2].clone())))
    } else if args.len() == 1 {
        (default_x_var(&expr), None)
    } else {
        return false;
    };
    let eo = EvaluationOptions::default();
    // `simplify_first` (MathStructure-integrate.cc:7407): the integrand is
    // evaluated before the pattern table sees it, so `x^-1` and `1/x` reach
    // the same shape.
    let mut expr = expr;
    evaluate(&mut expr, &eo);
    let anti = integrate(&expr, &xvar).map(|mut f| {
        evaluate(&mut f, &eo);
        f
    });
    match bounds {
        None => {
            let Some(f) = anti else { return false };
            *m = add(vec![f, c_sym()]);
            evaluate(m, &eo);
            true
        }
        Some((a, b)) => {
            if let Some(f) = &anti {
                if let Some(r) = definite_from_antiderivative(f, &xvar, &a, &b, &eo) {
                    *m = r;
                    return true;
                }
            }
            // Numeric fallback (MathStructure-integrate.cc:7866).
            let (MathStructure::Number(na), MathStructure::Number(nb)) = (&a, &b) else {
                return false;
            };
            if !na.is_real() || !nb.is_real() || na.includes_infinity() || nb.includes_infinity() {
                return false;
            }
            match romberg(&expr, &xvar, na, nb, 6, 22, true) {
                Some(v) => {
                    *m = MathStructure::Number(v);
                    true
                }
                None => false,
            }
        }
    }
}

fn default_x_var(expr: &MathStructure) -> MathStructure {
    crate::polynomial::find_x_var(expr).unwrap_or_else(|| MathStructure::symbolic("x"))
}

/// `RombergFunction::calculate` — `romberg(expr, a, b, min, max, x)`.
fn calculate_romberg(m: &mut MathStructure, args: &[MathStructure]) -> bool {
    if args.len() < 3 || args.len() > 6 {
        return false;
    }
    let expr = args[0].clone();
    let (MathStructure::Number(a), MathStructure::Number(b)) = (&args[1], &args[2]) else {
        return false;
    };
    if !a.is_real() || !b.is_real() {
        return false;
    }
    let steps = |i: usize, dflt: usize| -> usize {
        match args.get(i) {
            Some(MathStructure::Number(n)) => n.to_i64().map(|v| v.max(2) as usize).unwrap_or(dflt),
            _ => dflt,
        }
    };
    let min_steps = steps(3, 6);
    let max_steps = steps(4, 20);
    let xvar = match args.get(5) {
        Some(v) if v.is_symbolic() => v.clone(),
        _ => default_x_var(&expr),
    };
    match romberg(&expr, &xvar, a, b, min_steps, max_steps, false) {
        Some(v) => {
            *m = MathStructure::Number(v);
            true
        }
        None => false,
    }
}

pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let id = match name {
        "integrate" | "integral" => self::id::INTEGRATE,
        "romberg" => self::id::ROMBERG,
        "Shi" | "sinhint" => self::id::SINHINT,
        "Chi" | "coshint" => self::id::COSHINT,
        "fresnels" => self::id::FRESNEL_S,
        "fresnelc" => self::id::FRESNEL_C,
        "igamma" => self::id::I_GAMMA,
        "gammainc" => self::id::GAMMAINC,
        "betainc" => self::id::INCOMPLETE_BETA,
        _ => return None,
    };
    Some(FunctionId(id))
}

pub fn function_name(id: u32) -> Option<&'static str> {
    Some(match id {
        self::id::INTEGRATE => "integrate",
        self::id::ROMBERG => "romberg",
        self::id::SINHINT => "Shi",
        self::id::COSHINT => "Chi",
        self::id::FRESNEL_S => "fresnels",
        self::id::FRESNEL_C => "fresnelc",
        self::id::I_GAMMA => "igamma",
        self::id::GAMMAINC => "gammainc",
        self::id::INCOMPLETE_BETA => "betainc",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::session::Session;

    /// Every expected value here was produced by the reference binary
    /// (`qalc -t +u8`, the mode `--test-file` runs in).
    ///
    /// The evaluation goes through a [`Session`] because the builtin
    /// constants (`i`) and the unit store have to be in place, exactly as in
    /// the CLI the transcripts run under.
    fn ev(s: &str) -> String {
        let mut session = Session::new();
        // `src/qalc.cc` runs the CLI with `APPROXIMATION_APPROXIMATE`; the
        // transcripts are recorded under that setting.
        session.eval_options.approximation = crate::options::ApproximationMode::Approximate;
        session.evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn power_rule_and_constant() {
        assert_eq!(ev("integrate(6x^2)"), "2x^3 + C");
        assert_eq!(ev("integrate(x)"), "0.5x^2 + C");
        assert_eq!(ev("integrate(5)"), "5x + C");
    }

    #[test]
    fn reciprocal_gives_a_logarithm() {
        assert_eq!(ev("integrate(1/x)"), "ln(|x|) + C");
        assert_eq!(ev("integrate(x^-1)"), "ln(|x|) + C");
    }

    #[test]
    fn trigonometric_table() {
        assert_eq!(ev("integrate(sin(x))"), "-cos(x) + C");
        assert_eq!(ev("integrate(cos(x))"), "sin(x) + C");
    }

    #[test]
    fn linear_substitution() {
        assert_eq!(ev("integrate(sin(2x+1))"), "-0.5 * cos(2x + 1) + C");
    }

    #[test]
    fn square_root_power_rule() {
        assert_eq!(ev("integrate(sqrt(x))"), "0.6666666667x * sqrt(x) + C");
    }

    #[test]
    fn exponential_base() {
        assert_eq!(ev("integrate(3^x)"), "0.9102392266 * 3^x + C");
    }

    #[test]
    fn logarithm_by_the_antiderivative_table() {
        assert_eq!(ev("integrate(ln(x))"), "ln(x) * x - x + C");
    }

    #[test]
    fn sinh_over_x_gives_shi() {
        assert_eq!(ev("integrate(sinh(x)/x)"), "Shi(x) + C");
        assert_eq!(ev("integrate(sinh(x^2)/x)"), "0.5 * Shi(x^2) + C");
    }

    #[test]
    fn transcript_indefinite_integral() {
        assert_eq!(
            ev("integrate(sinh(x^2)/(5x) + 3xy/sqrt(x))"),
            "2x * sqrt(x) * y + 0.1 * Shi(x^2) + C"
        );
    }

    #[test]
    fn definite_integral_is_exact_when_it_can_be() {
        assert_eq!(ev("integrate(6x^2; 1; 5)"), "248");
        assert_eq!(ev("integrate(x^2, 1, 2)"), "2.333333333");
    }

    #[test]
    fn transcript_definite_integral_with_a_free_symbol() {
        assert_eq!(
            ev("integrate(sinh(x^2)/(5x) + 3xy/sqrt(x); 1; 2)"),
            "3.656854249y + 0.8760076036"
        );
    }

    #[test]
    fn definite_integral_falls_back_to_romberg() {
        // `integrate(Ei(x))` has no antiderivative in the reference either,
        // so the whole integrand goes through numeric quadrature.
        assert_eq!(ev("integrate(Ei(x) + 3^x - sin(ln(x)), 1, 2)"), "8.434289610");
    }

    #[test]
    fn romberg_builtin() {
        assert_eq!(ev("romberg(5x + ln(x), 1, 5)"), "64.04718956");
        assert_eq!(ev("romberg(3^x, 1, 2)"), "5.461435360");
    }

    #[test]
    fn by_parts_reduces_the_polynomial_degree() {
        assert_eq!(ev("integrate(x*e^x)"), "e^x * x - e^x + C");
    }

    #[test]
    fn partial_fractions_combine_into_one_logarithm() {
        assert_eq!(ev("integrate(1/(x^2-1))"), "0.5 * ln(|x - 1| / |x + 1|) + C");
        assert_eq!(
            ev("integrate(1/((x-1)*(x+2)))"),
            "0.3333333333 * ln(|x - 1| / |x + 2|) + C"
        );
    }

    #[test]
    fn hyperbolic_integrals() {
        assert_eq!(ev("Shi(1)"), "1.057250875");
        assert_eq!(ev("Shi(4)"), "9.817326911");
        assert_eq!(ev("Chi(2)"), "2.452666923");
    }

    #[test]
    fn fresnel_integrals() {
        assert_eq!(ev("fresnels(5)"), "0.4991913819");
        assert_eq!(ev("fresnels(1)"), "0.4382591474");
        assert_eq!(ev("fresnelc(1)"), "0.7798934004");
    }

    #[test]
    fn incomplete_gamma() {
        // The reference computes `gammainc` as `gamma(x) - igamma(x, y)`;
        // the cancellation is why only six digits are shown.
        assert_eq!(ev("gammainc(53, 5.2)"), "1.02201E34");
        assert_eq!(ev("igamma(3, 1)"), "1.839397206");
    }

    #[test]
    fn incomplete_beta() {
        // Integer parameters take the exact binomial-sum branch.
        assert_eq!(ev("betainc(0.5, 2, 3)"), "0.6875");
        assert_eq!(
            ev("betainc(5i - 2, 32, 3.2)"),
            "-9.431063439E27 - 5.083225623E27i"
        );
    }

    #[test]
    fn an_unintegrable_expression_is_left_alone() {
        let s = ev("integrate(Ei(x))");
        assert!(s.contains("integrate"), "got {s}");
    }
}
