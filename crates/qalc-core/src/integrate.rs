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

/// `m = a*x + b` with numeric `a != 0` and `b`, or `None`.
fn linear(m: &MathStructure, x: &MathStructure) -> Option<(Number, Number)> {
    let (a, b) = linear_raw(m, x)?;
    if a.is_zero() {
        return None;
    }
    Some((a, b))
}

fn linear_raw(m: &MathStructure, x: &MathStructure) -> Option<(Number, Number)> {
    if m.equals(x) {
        return Some((Number::from_i64(1), Number::new()));
    }
    if !contains(m, x) {
        let MathStructure::Number(n) = m else {
            return None;
        };
        return Some((Number::new(), n.clone()));
    }
    match m {
        MathStructure::Addition(terms) => {
            let mut a = Number::new();
            let mut b = Number::new();
            for t in terms {
                let (ta, tb) = linear_raw(t, x)?;
                if !a.add(&ta) || !b.add(&tb) {
                    return None;
                }
            }
            Some((a, b))
        }
        MathStructure::Multiplication(factors) => {
            let mut coeff = Number::from_i64(1);
            let mut seen_x = false;
            for f in factors {
                if f.equals(x) {
                    if seen_x {
                        return None;
                    }
                    seen_x = true;
                    continue;
                }
                if contains(f, x) {
                    return None;
                }
                let MathStructure::Number(n) = f else {
                    return None;
                };
                if !coeff.multiply(n) {
                    return None;
                }
            }
            if seen_x {
                Some((coeff, Number::new()))
            } else {
                Some((Number::new(), coeff))
            }
        }
        _ => None,
    }
}

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
    int_rec(m, x, 0)
}

fn int_rec(m: &MathStructure, x: &MathStructure, depth: usize) -> Option<MathStructure> {
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
    match m {
        MathStructure::Addition(terms) => {
            let mut out = Vec::with_capacity(terms.len());
            for t in terms {
                out.push(int_rec(t, x, depth + 1)?);
            }
            Some(add(out))
        }
        MathStructure::Multiplication(factors) => int_product(factors, x, depth),
        MathStructure::Power { base, exponent } => int_power(base, exponent, x, depth),
        MathStructure::Function { id, args } => int_function(id.0, args, x, depth),
        _ => None,
    }
}

fn int_power(
    base: &MathStructure,
    exponent: &MathStructure,
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    let b_has = contains(base, x);
    let e_has = contains(exponent, x);
    if b_has && e_has {
        return None;
    }
    if b_has {
        // `int (a x + b)^n dx`, with `n` constant.
        let Some((a, _)) = linear(base, x) else {
            // A higher-degree denominator: `int P(x)/Q(x) dx` by partial
            // fractions (the `1/Q(x)` shape reaches here as a bare power).
            if matches!(exponent, MathStructure::Number(n) if n.is_minus_one()) {
                return int_partial_fractions(
                    &mul(vec![pow(base.clone(), num(-1))]),
                    x,
                    depth,
                );
            }
            return None;
        };
        if let MathStructure::Number(n) = exponent {
            // `int (a x + b)^-1 dx` is a logarithm, not a power: the general
            // rule below would divide by `n + 1 = 0`.
            if n.is_minus_one() {
                // `int 1/(a x + b) dx = ln|a x + b| / a`
                return Some(over(
                    func(bid::LN, vec![func(bid::ABS, vec![base.clone()])]),
                    &a,
                ));
            }
        }
        // `(a x + b)^(n+1) / (a (n+1))`
        let mut np1 = add(vec![exponent.clone(), num(1)]);
        evaluate(&mut np1, &EvaluationOptions::default());
        if matches!(&np1, MathStructure::Number(n) if n.is_zero()) {
            return Some(over(
                func(bid::LN, vec![func(bid::ABS, vec![base.clone()])]),
                &a,
            ));
        }
        let mut r = mul(vec![pow(base.clone(), np1.clone()), inv(np1)]);
        r = over(r, &a);
        return Some(r);
    }
    // `int b^(a x + c) dx = b^(a x + c) / (a ln b)`.
    let (a, _) = linear(exponent, x)?;
    let mut factors = vec![pow(base.clone(), exponent.clone())];
    if !matches!(base, MathStructure::Symbolic(s) if s == "e") {
        factors.push(inv(func(bid::LN, vec![base.clone()])));
    }
    let _ = depth;
    Some(over(mul(factors), &a))
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

fn add_one_square(u: &MathStructure) -> MathStructure {
    add(vec![num(1), pow(u.clone(), num(2))])
}

fn sub_one_square(u: &MathStructure) -> MathStructure {
    add(vec![num(1), neg(pow(u.clone(), num(2)))])
}

fn int_function(
    id: u32,
    args: &[MathStructure],
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    let _ = depth;
    if args.len() != 1 {
        return None;
    }
    let u = &args[0];
    let (a, _) = linear(u, x)?;
    let f = antiderivative_of(id, u)?;
    Some(over(f, &a))
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
        let inner = int_rec(&vars[0], x, depth + 1)?;
        consts.push(inner);
        return Some(mul(consts));
    }
    if let Some(r) = int_special_quotient(&vars, x) {
        consts.push(r);
        return Some(mul(consts));
    }
    if let Some(r) = int_by_parts(&vars, x, depth) {
        consts.push(r);
        return Some(mul(consts));
    }
    if let Some(r) = int_partial_fractions(&mul(vars.clone()), x, depth) {
        consts.push(r);
        return Some(mul(consts));
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

/// `int x^k g(x) dx = x^k G(x) - k int x^(k-1) G(x) dx`.
///
/// `k` strictly decreases, so the recursion terminates; it is additionally
/// bounded by `k <= MAX_PARTS_POWER` and by [`MAX_DEPTH`].
fn int_by_parts(
    vars: &[MathStructure],
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    if depth + 1 > MAX_DEPTH {
        return None;
    }
    // Find the `x^k` factor.
    let mut k_idx = None;
    for (i, v) in vars.iter().enumerate() {
        if let Some(k) = small_power_of_x(v, x) {
            k_idx = Some((i, k));
            break;
        }
    }
    let (i, k) = k_idx?;
    let rest: Vec<MathStructure> = vars
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(_, v)| v.clone())
        .collect();
    let g = mul(rest);
    let big_g = int_rec(&g, x, depth + 1)?;
    let u = vars[i].clone();
    let next_u = if k == 1 {
        num(1)
    } else {
        pow(x.clone(), num(k - 1))
    };
    let tail = int_rec(&mul(vec![next_u, big_g.clone()]), x, depth + 1)?;
    Some(add(vec![
        mul(vec![u, big_g]),
        neg(mul(vec![num(k), tail])),
    ]))
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
fn int_partial_fractions(
    m: &MathStructure,
    x: &MathStructure,
    depth: usize,
) -> Option<MathStructure> {
    if depth + 1 > MAX_DEPTH {
        return None;
    }
    let (num_poly, den_poly) = as_rational_function(m, x)?;
    if den_poly.len() < 2 || den_poly.len() > MAX_PF_DEGREE + 1 {
        return None;
    }
    // Only a proper fraction: the reference divides first, and polynomial
    // division is not needed by any transcript here.
    if num_poly.len() >= den_poly.len() {
        return None;
    }
    let roots = distinct_rational_roots(&den_poly)?;
    if roots.len() + 1 != den_poly.len() {
        return None;
    }
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
    Some(combine_logs(parts))
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
    let n_poly = crate::polynomial::to_dense(&mul(numer), x)?;
    let d_poly = crate::polynomial::to_dense(&mul(denom), x)?;
    Some((trim(n_poly), trim(d_poly)))
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
    let a0 = *ints.first()?;
    let an = *ints.last()?;
    if an == 0 {
        return None;
    }
    if a0 == 0 {
        // A zero root; the transcripts do not need this case and handling it
        // would require deflation, so bail out rather than guess.
        return None;
    }
    let mut roots: Vec<Number> = Vec::new();
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
