//! Equation solving — port of `MathStructure::isolate_x`
//! (`MathStructure-isolatex.cc`) and the `solve()` builtin
//! (`BuiltinFunctions-algebra.cc`).
//!
//! The C++ isolates `x` by repeatedly moving everything else to the other
//! side of a `STRUCT_COMPARISON`, falling back to polynomial factorization
//! for higher degrees. This port implements:
//!
//! - linear and quadratic equations (the quadratic formula, kept exact),
//! - polynomials of any degree that factor into rational roots,
//! - cubics that do not, via Cardano kept in radicals (`cubic_roots`):
//!   `x^3 + x^2 + x = 5` → `2 / (3 * cbrt(3 * sqrt(561) - 71)) - 0.3333333333
//!   - cbrt(3 * sqrt(561) - 71) / 3`,
//! - `a^x = b` → `x = ln(b)/ln(a)`,
//! - the trigonometric general solutions: `1/3*sin(3x) - 1/3 = 0` →
//!   `x = 0.6666666667 pi * n + pi / 6`,
//! - Lambert-W inversion ([`solve_lambert`]), covering both `ln(x) + x = b`
//!   (`ln(x) + x = 3` → `x = lambertw(e^3)`) and `x^(a x) = b`, which uses
//!   both W branches (`x^(-3x) = 2` → two roots),
//! - numeric root finding for `newtonsolve`/`secantsolve` and for the
//!   approximate cases.
//!
//! TODO(port): quartic radicals via the resolvent cubic — every quartic in the
//! transcripts factors into rational roots, so `x^4 + x + 1 = 0` comes back
//! unsolved (see [`solve_polynomial`]) — and interval/assumption filtering of
//! the solution set, which is what would drop the roots the C++ rejects
//! against `CALCULATOR->defaultAssumptions()`.

use crate::builtins::id as bid;
use crate::ids::FunctionId;
use crate::options::EvaluationOptions;
use crate::polynomial as poly;
use crate::structure::{ComparisonType, MathStructure};
use qalc_num::Number;

pub mod id {
    pub const SOLVE: u32 = 1840;
    pub const SOLVE_MULTIPLE: u32 = 1841;
    pub const NEWTON_RAPHSON: u32 = 1850;
    pub const SECANT_METHOD: u32 = 1851;
}

fn eo() -> EvaluationOptions {
    current_eo()
}

// The options in force for the current top-level evaluation. The C++ passes
// `EvaluationOptions` explicitly through `isolate_x`; this port stashes them
// so the (recursive, structure-shaped) helpers stay simple.
thread_local! {
    static CURRENT_EO: std::cell::RefCell<EvaluationOptions> =
        std::cell::RefCell::new(EvaluationOptions::default());
}

fn current_eo() -> EvaluationOptions {
    CURRENT_EO.with(|c| c.borrow().clone())
}

fn num(i: i64) -> MathStructure {
    MathStructure::from(i)
}

fn func(id: u32, args: Vec<MathStructure>) -> MathStructure {
    MathStructure::Function {
        id: FunctionId(id),
        args,
    }
}

/// Evaluate `m` with the standard pipeline (functions + merge + sort),
/// honouring the options of the enclosing evaluation.
fn ev(m: &mut MathStructure) {
    let eo = current_eo();
    for _ in 0..8 {
        let changed = crate::builtins::calculate_functions_eo(m, &eo);
        let merged = m.calculatesub(&eo);
        if !changed && !merged {
            break;
        }
    }
    crate::sort::sort(m);
}

/// Solve `left = right` for `xvar`, returning the solution structures.
///
/// The comparison is first normalized to `lhs - rhs = 0`.
pub fn solve_equation(
    left: &MathStructure,
    right: &MathStructure,
    xvar: &MathStructure,
) -> Option<Vec<MathStructure>> {
    let mut expr = left.clone();
    expr.calculate_subtract(right.clone(), &eo());
    ev(&mut expr);
    solve_zero(&expr, xvar)
}

/// Solve `expr = 0` for `xvar`.
pub fn solve_zero(expr: &MathStructure, xvar: &MathStructure) -> Option<Vec<MathStructure>> {
    let dense = poly::to_dense(expr, xvar);
    if let Some(d) = &dense {
        if let Some(sols) = solve_polynomial(d) {
            return Some(sols);
        }
    } else if let Some(sols) = solve_exponential(expr, xvar) {
        return Some(sols);
    } else if let Some(sols) = solve_power_substitution(expr, xvar) {
        return Some(sols);
    } else if let Some(sols) = solve_trig(expr, xvar) {
        return Some(sols);
    } else if eo().approximation == crate::options::ApproximationMode::Exact {
        // The Lambert-W forms are only useful while the answer stays
        // symbolic; in approximate mode the numeric fallback below already
        // finds every branch.
        if let Some(sols) = solve_lambert(expr, xvar) {
            return Some(sols);
        }
    }
    // Nothing closed-form: fall back to numeric root finding unless the
    // result is required to be exact.
    if eo().approximation == crate::options::ApproximationMode::Exact {
        return None;
    }
    let (lo, hi) = match &dense {
        Some(d) => {
            let b = cauchy_bound(d);
            (-b, b)
        }
        None => (-10.0, 10.0),
    };
    let roots = numeric_roots(expr, xvar, lo, hi);
    if roots.is_empty() {
        None
    } else {
        Some(roots.into_iter().map(MathStructure::Number).collect())
    }
}

/// Cauchy's bound `1 + max|a_i / a_n|` on the modulus of the roots.
fn cauchy_bound(c: &[Number]) -> f64 {
    let n = c.len() - 1;
    let lead = c[n].float_value().abs();
    if lead == 0.0 {
        return 10.0;
    }
    let mut m: f64 = 0.0;
    for x in &c[..n] {
        m = m.max(x.float_value().abs() / lead);
    }
    (1.0 + m).min(1.0e6)
}

/// Sample `expr` over `[lo, hi]`, bisecting every sign change.
///
/// This replaces the C++ `MathStructure::calculateFunctions` +
/// `find_interval_precision` machinery, which is not ported. Roots are
/// returned in descending order, matching how the reference lists them.
pub fn numeric_roots(
    expr: &MathStructure,
    xvar: &MathStructure,
    lo: f64,
    hi: f64,
) -> Vec<Number> {
    const SAMPLES: usize = 1200;
    let mut roots: Vec<Number> = Vec::new();
    let step = (hi - lo) / SAMPLES as f64;
    if !step.is_finite() || step <= 0.0 {
        return roots;
    }
    let mut prev: Option<(f64, Number)> = None;
    for i in 0..=SAMPLES {
        let x = lo + step * i as f64;
        let mut xn = Number::new();
        xn.set_float(x);
        let Some(fx) = eval_at(expr, xvar, &xn) else {
            prev = None;
            continue;
        };
        if !fx.is_real() {
            prev = None;
            continue;
        }
        if fx.is_zero() {
            push_root(&mut roots, xn.clone());
            prev = Some((x, fx));
            continue;
        }
        if let Some((px, pf)) = &prev {
            if pf.is_negative() != fx.is_negative() {
                if let Some(r) = bisect(expr, xvar, *px, x) {
                    push_root(&mut roots, r);
                }
            }
        }
        prev = Some((x, fx));
    }
    roots.sort_by(|a, b| {
        if a.is_greater_than(b) {
            std::cmp::Ordering::Less
        } else if b.is_greater_than(a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    roots
}

fn push_root(roots: &mut Vec<Number>, r: Number) {
    let close = roots.iter().any(|x| {
        let mut d = x.clone();
        d.subtract(&r);
        d.abs();
        d.is_less_than(&Number::from_ints(1, 1, -9))
    });
    if !close {
        roots.push(r);
    }
}

/// Bisection on a bracketing interval, refined by a few secant steps.
fn bisect(expr: &MathStructure, xvar: &MathStructure, a: f64, b: f64) -> Option<Number> {
    let mut lo = Number::new();
    lo.set_float(a);
    let mut hi = Number::new();
    hi.set_float(b);
    let flo = eval_at(expr, xvar, &lo)?;
    let mut lo_neg = flo.is_negative();
    for _ in 0..200 {
        let mut mid = lo.clone();
        mid.add(&hi);
        mid.divide(&Number::from_i64(2));
        if mid.equals(&lo, false, false) || mid.equals(&hi, false, false) {
            break;
        }
        let Some(fm) = eval_at(expr, xvar, &mid) else {
            break;
        };
        if fm.is_zero() {
            return Some(mid);
        }
        if fm.is_negative() == lo_neg {
            lo = mid;
        } else {
            hi = mid;
        }
        let _ = &mut lo_neg;
    }
    let mut r = lo.clone();
    r.add(&hi);
    r.divide(&Number::from_i64(2));
    r.set_approximate(true);
    Some(r)
}

/// `a^x * c + d = 0` → `x = ln(-d/c) / ln(a)`.
fn solve_exponential(expr: &MathStructure, xvar: &MathStructure) -> Option<Vec<MathStructure>> {
    // Shape after normalization: Addition[ Multiplication[c, a^x], d ] or
    // Addition[ a^x, d ].
    let (terms, constant) = match expr {
        MathStructure::Addition(v) if v.len() == 2 => {
            if v[1].is_number() {
                (&v[0], v[1].clone())
            } else if v[0].is_number() {
                (&v[1], v[0].clone())
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let (coeff, powpart) = match terms {
        MathStructure::Multiplication(v) if v.len() == 2 && v[0].is_number() => {
            (v[0].clone(), &v[1])
        }
        other => (num(1), other),
    };
    let MathStructure::Power { base, exponent } = powpart else {
        return None;
    };
    if !exponent.equals(xvar) || !base.is_number() {
        return None;
    }
    // x = ln(-constant / coeff) / ln(base)
    let mut rhs = constant;
    rhs.calculate_negate_eo(&eo());
    rhs.calculate_divide(coeff, &eo());
    let mut sol = func(bid::LN, vec![rhs]);
    sol.calculate_divide(func(bid::LN, vec![(**base).clone()]), &eo());
    ev(&mut sol);
    Some(vec![sol])
}

/// Solve a dense polynomial equation exactly.
///
/// Rational roots are peeled off first (which is what makes the cubic
/// transcripts work); a leftover quadratic is solved with the quadratic
/// formula, keeping the discriminant as an exact `sqrt`.
///
/// The reference lists solutions in *descending* order (`x = 5 or x = 2 or
/// x = -2`), so the rational roots are sorted and the leading coefficient is
/// normalized positive before the quadratic formula runs.
pub fn solve_polynomial(dense: &[Number]) -> Option<Vec<MathStructure>> {
    let deg = dense.len().saturating_sub(1);
    if deg == 0 {
        return None;
    }
    let mut out: Vec<MathStructure> = Vec::new();
    let mut rest = dense.to_vec();
    if rest[deg].is_negative() {
        for c in rest.iter_mut() {
            c.negate();
        }
    }
    let mut rational: Vec<Number> = Vec::new();
    // Peel rational roots.
    loop {
        let d = rest.len().saturating_sub(1);
        if d == 0 {
            break;
        }
        let roots = poly::rational_roots(&rest);
        let Some(r) = roots.into_iter().next() else {
            break;
        };
        let mut neg = r.clone();
        neg.negate();
        let lin = vec![neg, Number::from_i64(1)];
        let Some(q) = poly::dense_divide(&rest, &lin) else {
            break;
        };
        if !rational.iter().any(|x: &Number| x.equals(&r, false, false)) {
            rational.push(r);
        }
        rest = q;
    }
    rational.sort_by(|a, b| {
        if a.is_greater_than(b) {
            std::cmp::Ordering::Less
        } else if b.is_greater_than(a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    out.extend(rational.into_iter().map(MathStructure::Number));
    let d = rest.len().saturating_sub(1);
    match d {
        0 => {}
        1 => {
            // a x + b = 0
            let mut r = rest[0].clone();
            if !r.divide(&rest[1]) || !r.negate() {
                return None;
            }
            out.push(MathStructure::Number(r));
        }
        2 => {
            out.extend(quadratic_roots(&rest[2], &rest[1], &rest[0])?);
        }
        3 => {
            match cubic_roots(&rest[3], &rest[2], &rest[1], &rest[0]) {
                Some(r) => out.extend(r),
                None if out.is_empty() => return None,
                None => {}
            }
        }
        _ => {
            // TODO(port): quartic radicals (the resolvent cubic) — the
            // transcripts' quartics all factor into rational roots.
            if out.is_empty() {
                return None;
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The exact quadratic formula for `a x^2 + b x + c = 0`, in the shape the
/// reference prints: `(sqrt(D) - b) / (2a)` and `-b/(2a) - sqrt(D)/(2a)`.
fn quadratic_roots(a: &Number, b: &Number, c: &Number) -> Option<Vec<MathStructure>> {
    // D = b^2 - 4ac
    let mut disc = b.clone();
    if !disc.square() {
        return None;
    }
    let mut ac = a.clone();
    if !ac.multiply(c) || !ac.multiply(&Number::from_i64(4)) {
        return None;
    }
    if !disc.subtract(&ac) {
        return None;
    }
    let mut two_a = a.clone();
    if !two_a.multiply(&Number::from_i64(2)) {
        return None;
    }
    let mut minus_b = b.clone();
    if !minus_b.negate() {
        return None;
    }

    let exact_sqrt = {
        let mut n = disc.clone();
        (n.sqrt() && !n.is_approximate()).then_some(n)
    };
    if let Some(n) = exact_sqrt {
        // A rational discriminant: both roots are plain numbers.
        let mut roots = Vec::new();
        for sign in [1i64, -1] {
            let mut r = n.clone();
            if sign < 0 && !r.negate() {
                return None;
            }
            if !r.add(&minus_b) || !r.divide(&two_a) {
                return None;
            }
            roots.push(MathStructure::Number(r));
        }
        return Some(roots);
    }

    // The C++ keeps the radical as `D^(1/2)` (a power, which is why it sorts
    // before the constant term); the printer renders it as `sqrt(D)`.
    let sqrt_d = MathStructure::Power {
        base: Box::new(MathStructure::Number(disc.clone())),
        exponent: Box::new(MathStructure::Number(Number::from_ints(1, 2, 0))),
    };
    // The reference prints the two roots in different shapes — the first as
    // one fraction, the second distributed:
    //     x = (sqrt(5) + 3) / 2 or x = 3/2 - sqrt(5) / 2
    // so they are assembled directly instead of going back through the merge
    // engine, which would expand both.
    let inv_two_a = MathStructure::Power {
        base: Box::new(MathStructure::Number(two_a.clone())),
        exponent: Box::new(num(-1)),
    };
    let first = if minus_b.is_zero() {
        MathStructure::Multiplication(vec![sqrt_d.clone(), inv_two_a.clone()])
    } else {
        MathStructure::Multiplication(vec![
            MathStructure::Addition(vec![sqrt_d.clone(), MathStructure::Number(minus_b.clone())]),
            inv_two_a.clone(),
        ])
    };
    let neg_sqrt_over = MathStructure::Multiplication(vec![num(-1), sqrt_d, inv_two_a]);
    let second = if minus_b.is_zero() {
        neg_sqrt_over
    } else {
        let mut offset = minus_b.clone();
        if !offset.divide(&two_a) {
            return None;
        }
        MathStructure::Addition(vec![MathStructure::Number(offset), neg_sqrt_over])
    };
    Some(vec![first, second])
}

// ----------------------------------------------------------------------
// Equations that become polynomial after `u = x^(1/k)`
// ----------------------------------------------------------------------

/// `x^(1/3) + x^(2/3) = 3` is a quadratic in `u = x^(1/3)`; the reference
/// solves it and raises the root back, printing `x = 2 * sqrt(13) - 5`.
///
/// The roots live in `Q(sqrt D)`, so `u^k` is computed there exactly and the
/// candidates are checked by substitution — the second root of the quadratic
/// is negative, and its cube's principal cube root is complex, so it is not a
/// solution (which is why the reference prints only one).
pub fn solve_power_substitution(
    expr: &MathStructure,
    xvar: &MathStructure,
) -> Option<Vec<MathStructure>> {
    let mut exps: Vec<Number> = Vec::new();
    collect_x_exponents(expr, xvar, &mut exps, 0)?;
    if exps.is_empty() {
        return None;
    }
    // k = lcm of the exponent denominators; the substitution is x = u^k.
    let mut k = Number::from_i64(1);
    for e in &exps {
        if !k.lcm(&e.denominator()) {
            return None;
        }
    }
    let k_i = k.to_i64().filter(|k| (2..=12).contains(k))?;
    let dense = dense_in_u(expr, xvar, k_i, 0)?;
    let roots = surd_roots(&dense)?;
    let mut out: Vec<MathStructure> = Vec::new();
    for (p, q, d) in roots {
        let (mut pp, mut qq) = (Number::from_i64(1), Number::new());
        for _ in 0..k_i {
            // (pp + qq√d)(p + q√d)
            let mut np = pp.clone();
            let mut t = qq.clone();
            if !np.multiply(&p) || !t.multiply(&q) || !t.multiply(&d) || !np.add(&t) {
                return None;
            }
            let mut nq = pp.clone();
            let mut t2 = qq.clone();
            if !nq.multiply(&q) || !t2.multiply(&p) || !nq.add(&t2) {
                return None;
            }
            pp = np;
            qq = nq;
        }
        let sol = surd_structure(&pp, &qq, &d)?;
        // Reject the roots that the principal branch does not satisfy.
        let value = {
            let mut v = d.clone();
            if !v.sqrt() {
                continue;
            }
            let mut v2 = qq.clone();
            if !v2.multiply(&v) || !v2.add(&pp) {
                continue;
            }
            v2
        };
        match eval_at(expr, xvar, &value) {
            Some(r) if r.is_real() && r.float_value().abs() < 1e-6 => out.push(sol),
            _ => {}
        }
    }
    (!out.is_empty()).then_some(out)
}

fn collect_x_exponents(
    m: &MathStructure,
    xvar: &MathStructure,
    out: &mut Vec<Number>,
    depth: usize,
) -> Option<()> {
    if depth > 24 || !contains_var(m, xvar) {
        return Some(());
    }
    if m.equals(xvar) {
        out.push(Number::from_i64(1));
        return Some(());
    }
    if let MathStructure::Power { base, exponent } = m {
        if base.equals(xvar) {
            let MathStructure::Number(e) = &**exponent else {
                return None;
            };
            if !e.is_rational() || e.is_negative() {
                return None;
            }
            out.push(e.clone());
            return Some(());
        }
    }
    if !matches!(m, MathStructure::Addition(_) | MathStructure::Multiplication(_)) {
        return None;
    }
    for i in 0..m.size() {
        collect_x_exponents(m.get(i)?, xvar, out, depth + 1)?;
    }
    Some(())
}

/// `expr` as a dense polynomial in `u`, where `x = u^k`.
fn dense_in_u(
    m: &MathStructure,
    xvar: &MathStructure,
    k: i64,
    depth: usize,
) -> Option<Vec<Number>> {
    if depth > 24 {
        return None;
    }
    if !contains_var(m, xvar) {
        return Some(vec![const_number(m)?]);
    }
    let monomial = |deg: usize| -> Option<Vec<Number>> {
        (deg <= 32).then(|| {
            let mut v = vec![Number::new(); deg + 1];
            v[deg] = Number::from_i64(1);
            v
        })
    };
    if m.equals(xvar) {
        return monomial(k as usize);
    }
    match m {
        MathStructure::Addition(v) => {
            let mut acc: Vec<Number> = Vec::new();
            for t in v {
                let p = dense_in_u(t, xvar, k, depth + 1)?;
                if acc.len() < p.len() {
                    acc.resize(p.len(), Number::new());
                }
                for (i, c) in p.iter().enumerate() {
                    if !acc[i].add(c) {
                        return None;
                    }
                }
            }
            Some(acc)
        }
        MathStructure::Multiplication(v) => {
            let mut acc = vec![Number::from_i64(1)];
            for f in v {
                let p = dense_in_u(f, xvar, k, depth + 1)?;
                if acc.len() + p.len() > 34 {
                    return None;
                }
                let mut r = vec![Number::new(); acc.len() + p.len() - 1];
                for (i, x) in acc.iter().enumerate() {
                    for (j, y) in p.iter().enumerate() {
                        let mut t = x.clone();
                        if !t.multiply(y) || !r[i + j].add(&t) {
                            return None;
                        }
                    }
                }
                acc = r;
            }
            Some(acc)
        }
        MathStructure::Power { base, exponent } if base.equals(xvar) => {
            let MathStructure::Number(e) = &**exponent else {
                return None;
            };
            let mut deg = e.clone();
            if !deg.multiply(&Number::from_i64(k)) {
                return None;
            }
            monomial(deg.to_i64().filter(|d| (0..=32).contains(d))? as usize)
        }
        _ => None,
    }
}

/// Roots of a degree 1 or 2 dense polynomial as `(p, q, d)` meaning
/// `p + q sqrt(d)`.
fn surd_roots(dense: &[Number]) -> Option<Vec<(Number, Number, Number)>> {
    let deg = dense.len().checked_sub(1)?;
    match deg {
        1 => {
            let mut r = dense[0].clone();
            if !r.divide(&dense[1]) || !r.negate() {
                return None;
            }
            Some(vec![(r, Number::new(), Number::new())])
        }
        2 => {
            let (a, b, c) = (&dense[2], &dense[1], &dense[0]);
            let mut disc = b.clone();
            let mut ac = a.clone();
            if !disc.square()
                || !ac.multiply(c)
                || !ac.multiply(&Number::from_i64(4))
                || !disc.subtract(&ac)
                || disc.is_negative()
            {
                return None;
            }
            let mut two_a = a.clone();
            let mut p = b.clone();
            if !two_a.multiply(&Number::from_i64(2)) || !p.negate() || !p.divide(&two_a) {
                return None;
            }
            let mut q = Number::from_i64(1);
            if !q.divide(&two_a) {
                return None;
            }
            let mut nq = q.clone();
            if !nq.negate() {
                return None;
            }
            Some(vec![(p.clone(), q, disc.clone()), (p, nq, disc)])
        }
        _ => None,
    }
}

/// `p + q sqrt(d)` as a printable structure (`2 * sqrt(13) - 5`).
fn surd_structure(p: &Number, q: &Number, d: &Number) -> Option<MathStructure> {
    if q.is_zero() || d.is_zero() {
        return Some(MathStructure::Number(p.clone()));
    }
    let (outside, radicand) = sqrt_parts(d)?;
    let mut coeff = q.clone();
    if !coeff.multiply(&outside) {
        return None;
    }
    let root = match radicand {
        None => {
            let mut v = coeff;
            if !v.add(p) {
                return None;
            }
            return Some(MathStructure::Number(v));
        }
        Some(r) => MathStructure::Power {
            base: Box::new(MathStructure::Number(Number::from_i64(r))),
            exponent: Box::new(MathStructure::Number(Number::from_ints(1, 2, 0))),
        },
    };
    let term = scaled(&coeff, root);
    if p.is_zero() {
        return Some(term);
    }
    Some(MathStructure::Addition(vec![
        term,
        MathStructure::Number(p.clone()),
    ]))
}

/// `sqrt(n)` split into the rational factor outside the radical and the
/// remaining radicand (`None` when `n` is a perfect square).
fn sqrt_parts(n: &Number) -> Option<(Number, Option<i64>)> {
    let num_i = n.numerator().to_i64()?;
    let den_i = n.denominator().to_i64()?;
    let mut radicand = num_i.checked_mul(den_i)?;
    if radicand < 0 {
        return None;
    }
    let mut outside: i64 = 1;
    let mut f: i64 = 2;
    // Bounded trial division; anything with a larger square factor keeps it.
    while f <= 46_340 && f.checked_mul(f).is_some_and(|sq| sq <= radicand) {
        let sq = f * f;
        let mut guard = 0;
        while radicand % sq == 0 && guard < 128 {
            radicand /= sq;
            outside = outside.checked_mul(f)?;
            guard += 1;
        }
        f += 1;
    }
    let mut coeff = Number::from_i64(outside);
    if !coeff.divide(&Number::from_i64(den_i)) {
        return None;
    }
    Some((coeff, (radicand != 1).then_some(radicand)))
}

// ----------------------------------------------------------------------
// Lambert-W equations
// ----------------------------------------------------------------------
//
// `isolate_x` reaches for `FUNCTION_ID_LAMBERT_W` whenever the unknown
// appears both inside and outside an exponential or logarithm
// (`MathStructure-isolatex.cc`). The three shapes the transcripts use are:
//
// ```text
//   a ln(x) + b x + c = 0   ->  x = (a/b) W((b/a) e^(-c/a))
//   k A^(m x) + b x + c = 0 ->  x = -c/b - W((m k/b) A^(-m c/b) ln A) / (m ln A)
//   x^(a x) + c = 0         ->  x = e^W(ln(-c) / a)
// ```
//
// The last one has a second real solution on the `W_-1` branch whenever the
// argument lies in `(-1/e, 0)`, which is what the reference prints as
// `lambertw(z, -1)`.

/// One term of a Lambert-W-shaped sum.
enum LTerm {
    Const(Number),
    /// `b x`
    Linear(Number),
    /// `a ln(x)`
    Log(Number),
    /// `k A^(m x)`
    Exp {
        k: Number,
        base: Number,
        m: Number,
    },
    /// `x^(a x)`
    PowX(Number),
}

fn classify_lterm(m: &MathStructure, xvar: &MathStructure) -> Option<LTerm> {
    let one = Number::from_i64(1);
    // Split an optional numeric coefficient off the front.
    let (coeff, body) = match m {
        MathStructure::Multiplication(v) if v.len() == 2 => match &v[0] {
            MathStructure::Number(n) => (n.clone(), &v[1]),
            _ => (one.clone(), m),
        },
        _ => (one.clone(), m),
    };
    if let MathStructure::Number(n) = body {
        let mut c = coeff;
        return c.multiply(n).then_some(LTerm::Const(c));
    }
    if body.equals(xvar) {
        return Some(LTerm::Linear(coeff));
    }
    if let MathStructure::Function { id, args } = body {
        if id.0 == bid::LN && args.len() == 1 && args[0].equals(xvar) {
            return Some(LTerm::Log(coeff));
        }
        return None;
    }
    if let MathStructure::Power { base, exponent } = body {
        // The exponent must be `m x` with no constant part.
        let (a, b) = linear_form(exponent, xvar)?;
        if !b.is_zero() || a.is_zero() {
            return None;
        }
        if base.equals(xvar) {
            return coeff.is_one().then_some(LTerm::PowX(a));
        }
        if let MathStructure::Number(n) = &**base {
            if n.is_positive() && !n.is_one() {
                return Some(LTerm::Exp {
                    k: coeff,
                    base: n.clone(),
                    m: a,
                });
            }
        }
    }
    None
}

/// `A^e` split into a rational factor and a residual root, the way the
/// reference prints it: `2^(15/4)` is `8 * 8^(1/4)`.
fn split_root_power(base: &Number, e: &Number) -> Option<(Number, Option<MathStructure>)> {
    let en = e.numerator().to_i64()?;
    let ed = e.denominator().to_i64().filter(|d| (1..=64).contains(d))?;
    let whole = en.div_euclid(ed);
    let rem = en.rem_euclid(ed);
    let mut coeff = base.clone();
    if !coeff.raise(&Number::from_i64(whole), true) || coeff.is_approximate() {
        return None;
    }
    if rem == 0 {
        return Some((coeff, None));
    }
    // `A^(r/d) = (A^r)^(1/d)`
    let mut inner = base.clone();
    if !inner.raise(&Number::from_i64(rem), true) || inner.is_approximate() {
        return None;
    }
    Some((
        coeff,
        Some(MathStructure::Power {
            base: Box::new(MathStructure::Number(inner)),
            exponent: Box::new(MathStructure::Number(Number::from_ints(1, ed, 0))),
        }),
    ))
}

fn lambertw(arg: MathStructure, branch: Option<i64>) -> MathStructure {
    let mut args = vec![arg];
    if let Some(k) = branch {
        args.push(MathStructure::Number(Number::from_i64(k)));
    }
    func(crate::explog::id::LAMBERT_W, args)
}

/// `n · body`, in the shape the reference prints: `ln(2) / 3`,
/// `-ln(2) / 3`, `6 * 8^(1/4) * ln(2)`.
fn scaled(coeff: &Number, body: MathStructure) -> MathStructure {
    if coeff.is_one() {
        return body;
    }
    let numer = coeff.numerator();
    let denom = coeff.denominator();
    let mut factors: Vec<MathStructure> = Vec::new();
    if numer.is_one() {
        // nothing
    } else if numer.equals(&Number::from_i64(-1), false, false) {
        factors.push(num(-1));
    } else {
        factors.push(MathStructure::Number(numer));
    }
    factors.push(body);
    if !denom.is_one() {
        factors.push(MathStructure::Power {
            base: Box::new(MathStructure::Number(denom)),
            exponent: Box::new(num(-1)),
        });
    }
    if factors.len() == 1 {
        factors.pop().expect("len 1")
    } else {
        MathStructure::Multiplication(factors)
    }
}

/// Solve the Lambert-W shapes of `expr = 0`.
pub fn solve_lambert(expr: &MathStructure, xvar: &MathStructure) -> Option<Vec<MathStructure>> {
    let terms: Vec<&MathStructure> = match expr {
        MathStructure::Addition(v) => v.iter().collect(),
        other => vec![other],
    };
    if terms.len() > 4 {
        return None;
    }
    let mut c = Number::new();
    let (mut b, mut a) = (Number::new(), Number::new());
    let mut expo: Option<(Number, Number, Number)> = None;
    let mut powx: Option<Number> = None;
    for t in &terms {
        match classify_lterm(t, xvar)? {
            LTerm::Const(n) => {
                if !c.add(&n) {
                    return None;
                }
            }
            LTerm::Linear(n) => {
                if !b.add(&n) {
                    return None;
                }
            }
            LTerm::Log(n) => {
                if !a.add(&n) {
                    return None;
                }
            }
            LTerm::Exp { k, base, m } => {
                if expo.is_some() {
                    return None;
                }
                expo = Some((k, base, m));
            }
            LTerm::PowX(n) => {
                if powx.is_some() {
                    return None;
                }
                powx = Some(n);
            }
        }
    }
    let ln = |n: Number| func(bid::LN, vec![MathStructure::Number(n)]);
    let e_sym = || MathStructure::symbolic("e");

    // x^(a x) + c = 0  ->  x = e^W(ln(-c) / a)
    if let Some(av) = powx {
        if !a.is_zero() || !b.is_zero() || expo.is_some() || c.is_zero() {
            return None;
        }
        let mut rhs = c.clone();
        if !rhs.negate() || !rhs.is_positive() {
            return None;
        }
        let mut inv = av.clone();
        if !inv.recip() {
            return None;
        }
        let arg = scaled(&inv, ln(rhs.clone()));
        // The second branch exists on `(-1/e, 0)`.
        let mut val = rhs.float_value().ln();
        val /= av.float_value();
        let mut out = vec![MathStructure::Power {
            base: Box::new(e_sym()),
            exponent: Box::new(lambertw(arg.clone(), None)),
        }];
        if val < 0.0 && val > -std::f64::consts::E.recip() {
            out.push(MathStructure::Power {
                base: Box::new(e_sym()),
                exponent: Box::new(lambertw(arg, Some(-1))),
            });
        }
        return Some(out);
    }

    // k A^(m x) + b x + c = 0
    if let Some((k, abase, m)) = expo {
        if !a.is_zero() || b.is_zero() {
            return None;
        }
        // arg = (m k / b) · A^(-m c / b) · ln(A)
        let mut coeff = m.clone();
        if !coeff.multiply(&k) || !coeff.divide(&b) {
            return None;
        }
        let mut e_exp = m.clone();
        if !e_exp.multiply(&c) || !e_exp.divide(&b) || !e_exp.negate() {
            return None;
        }
        let (pow_coeff, root) = split_root_power(&abase, &e_exp)?;
        if !coeff.multiply(&pow_coeff) {
            return None;
        }
        let mut factors: Vec<MathStructure> = Vec::new();
        if !coeff.is_one() {
            factors.push(MathStructure::Number(coeff));
        }
        if let Some(r) = root {
            factors.push(r);
        }
        factors.push(ln(abase.clone()));
        let arg = if factors.len() == 1 {
            factors.pop().expect("len 1")
        } else {
            MathStructure::Multiplication(factors)
        };
        // x = -c/b - W(arg) / (m ln A)
        let mut shift = c.clone();
        if !shift.divide(&b) || !shift.negate() {
            return None;
        }
        let denom = if m.is_one() {
            ln(abase)
        } else {
            MathStructure::Multiplication(vec![MathStructure::Number(m), ln(abase)])
        };
        let wpart = MathStructure::Multiplication(vec![
            num(-1),
            lambertw(arg, None),
            MathStructure::Power {
                base: Box::new(denom),
                exponent: Box::new(num(-1)),
            },
        ]);
        let sol = if shift.is_zero() {
            wpart
        } else {
            MathStructure::Addition(vec![MathStructure::Number(shift), wpart])
        };
        return Some(vec![sol]);
    }

    // a ln(x) + b x + c = 0  ->  x = (a/b) W((b/a) e^(-c/a))
    if a.is_zero() || b.is_zero() {
        return None;
    }
    let mut ba = b.clone();
    if !ba.divide(&a) {
        return None;
    }
    let mut ca = c.clone();
    if !ca.divide(&a) || !ca.negate() {
        return None;
    }
    let epart = if ca.is_zero() {
        MathStructure::Number(Number::from_i64(1))
    } else if ca.is_one() {
        e_sym()
    } else {
        MathStructure::Power {
            base: Box::new(e_sym()),
            exponent: Box::new(MathStructure::Number(ca)),
        }
    };
    let arg = scaled(&ba, epart);
    let mut ab = a;
    if !ab.divide(&b) {
        return None;
    }
    Some(vec![scaled(&ab, lambertw(arg, None))])
}

// ----------------------------------------------------------------------
// Trigonometric equations
// ----------------------------------------------------------------------
//
// `MathStructure::isolate_x_sub` inverts a single `sin`/`cos`/`tan` call by
// applying `asin`/`acos`/`atan` to the other side and adding the period
// (`MathStructure-isolatex.cc:5996`); `sync_trigonometric_functions`
// (`:6784`) first rewrites the equation so that every trigonometric call
// shares one angle, using the double-angle identity
// `sin(2u) = 2 sin(u) cos(u)` and `cos(u)^2 = 1 - sin(u)^2`.
//
// This port does the same in three steps:
//
//   1. every `sin`/`cos` argument is read as `m * u` for a common base angle
//      `u = α x + β π`, and expanded into a polynomial in `S = sin(u)` and
//      `C = cos(u)` with the Chebyshev recurrences;
//   2. `C^2` is replaced by `1 - S^2`, leaving `P(S) + C Q(S)`, which is
//      factored so each factor involves a single trigonometric function;
//   3. each factor is inverted with the C++ branch formulas, giving
//      `x = A π n + B π`.
//
// Solutions whose angle is not a rational multiple of π cannot be written in
// this form, and the whole equation is then left unsolved (the C++ would
// print `asin(...)` — TODO(port)).

/// A solution family `x = a·π·n + b·π`.
#[derive(Clone, Debug)]
struct Family {
    a: Number,
    b: Number,
}

/// The base angle `u = α x + β π` every trigonometric call is expressed in.
struct BaseAngle {
    alpha: Number,
    beta: Number,
}

/// A polynomial in `S = sin(u)` and `C = cos(u)`: `terms[(i, j)]` is the
/// coefficient of `S^i C^j`. The coefficients are floats — the exact values
/// are recovered afterwards by matching the resulting angle against the
/// rational multiples of π.
#[derive(Clone, Default, Debug)]
struct ScPoly {
    terms: Vec<((u8, u8), f64)>,
}

const SC_MAX_DEGREE: u8 = 16;

impl ScPoly {
    fn constant(v: f64) -> Self {
        ScPoly {
            terms: vec![((0, 0), v)],
        }
    }
    fn var(i: u8, j: u8) -> Self {
        ScPoly {
            terms: vec![((i, j), 1.0)],
        }
    }
    fn add_term(&mut self, key: (u8, u8), v: f64) {
        if let Some(e) = self.terms.iter_mut().find(|(k, _)| *k == key) {
            e.1 += v;
        } else {
            self.terms.push((key, v));
        }
    }
    fn add(&self, o: &ScPoly) -> ScPoly {
        let mut r = self.clone();
        for (k, v) in &o.terms {
            r.add_term(*k, *v);
        }
        r.prune();
        r
    }
    fn mul(&self, o: &ScPoly) -> Option<ScPoly> {
        let mut r = ScPoly::default();
        for (ka, va) in &self.terms {
            for (kb, vb) in &o.terms {
                let i = ka.0.checked_add(kb.0)?;
                let j = ka.1.checked_add(kb.1)?;
                if i > SC_MAX_DEGREE || j > SC_MAX_DEGREE {
                    return None;
                }
                r.add_term((i, j), va * vb);
            }
        }
        r.prune();
        Some(r)
    }
    fn scale(&self, k: f64) -> ScPoly {
        let mut r = self.clone();
        for t in r.terms.iter_mut() {
            t.1 *= k;
        }
        r.prune();
        r
    }
    fn prune(&mut self) {
        self.terms.retain(|(_, v)| v.abs() > 1e-12);
    }
    fn is_zero(&self) -> bool {
        self.terms.is_empty()
    }
    /// `cos(u)^2 = 1 - sin(u)^2`, applied until every `C` exponent is 0 or 1.
    fn reduce_cos(&self) -> Option<ScPoly> {
        let mut p = self.clone();
        for _ in 0..64 {
            let Some(idx) = p.terms.iter().position(|((_, j), _)| *j >= 2) else {
                return Some(p);
            };
            let ((i, j), v) = p.terms.remove(idx);
            p.add_term((i, j - 2), v);
            let i2 = i.checked_add(2)?;
            if i2 > SC_MAX_DEGREE {
                return None;
            }
            p.add_term((i2, j - 2), -v);
            p.prune();
        }
        None
    }
}

/// `T_m(C)` and `S · U_{m-1}(C)` — `cos(m u)` and `sin(m u)`.
fn chebyshev(m: u32, sine: bool) -> Option<ScPoly> {
    if m == 0 {
        return Some(if sine {
            ScPoly::default()
        } else {
            ScPoly::constant(1.0)
        });
    }
    if m > 12 {
        return None;
    }
    let c = ScPoly::var(0, 1);
    let two_c = c.scale(2.0);
    let (mut prev, mut cur) = if sine {
        // U_0 = 1, U_1 = 2C
        (ScPoly::constant(1.0), two_c.clone())
    } else {
        // T_0 = 1, T_1 = C
        (ScPoly::constant(1.0), c.clone())
    };
    let target = if sine { m - 1 } else { m };
    let mut k = 1u32;
    while k < target {
        let next = two_c.mul(&cur)?.add(&prev.scale(-1.0));
        prev = cur;
        cur = next;
        k += 1;
    }
    let poly = if target == 0 { prev } else { cur };
    if sine {
        ScPoly::var(1, 0).mul(&poly)
    } else {
        Some(poly)
    }
}

/// True when `xvar` occurs anywhere in `m`.
fn contains_var(m: &MathStructure, xvar: &MathStructure) -> bool {
    if m.equals(xvar) {
        return true;
    }
    (0..m.size()).any(|i| m.get(i).is_some_and(|c| contains_var(c, xvar)))
}

/// The numeric value of an `x`-free structure.
fn const_value(m: &MathStructure) -> Option<f64> {
    let mut c = m.clone();
    let mut eo2 = eo();
    eo2.approximation = crate::options::ApproximationMode::Approximate;
    for _ in 0..8 {
        let changed = crate::builtins::calculate_functions(&mut c);
        let merged = c.calculatesub(&eo2);
        if !changed && !merged {
            break;
        }
    }
    match c {
        MathStructure::Number(n) if n.is_real() && !n.is_infinite(false) => Some(n.float_value()),
        _ => None,
    }
}

/// Read a trigonometric argument as `α x + β π`.
fn linear_form(m: &MathStructure, xvar: &MathStructure) -> Option<(Number, Number)> {
    let zero = Number::new();
    if m.equals(xvar) {
        return Some((Number::from_i64(1), zero));
    }
    match m {
        MathStructure::Symbolic(s) if s == "pi" => Some((zero.clone(), Number::from_i64(1))),
        MathStructure::Number(n) if n.is_zero() => Some((zero.clone(), zero)),
        MathStructure::Addition(v) => {
            let mut a = zero.clone();
            let mut b = zero;
            for t in v {
                let (ta, tb) = linear_form(t, xvar)?;
                if !a.add(&ta) || !b.add(&tb) {
                    return None;
                }
            }
            Some((a, b))
        }
        MathStructure::Multiplication(v) => {
            let mut k = Number::from_i64(1);
            let mut sym: Option<bool> = None; // Some(true) = x, Some(false) = pi
            for f in v {
                if f.equals(xvar) {
                    if sym.is_some() {
                        return None;
                    }
                    sym = Some(true);
                    continue;
                }
                match f {
                    MathStructure::Symbolic(s) if s == "pi" => {
                        if sym.is_some() {
                            return None;
                        }
                        sym = Some(false);
                    }
                    other => {
                        let n = const_number(other)?;
                        if !k.multiply(&n) {
                            return None;
                        }
                    }
                }
            }
            match sym {
                Some(true) => Some((k, Number::new())),
                Some(false) => Some((Number::new(), k)),
                None => None,
            }
        }
        other => {
            let n = const_number(other)?;
            n.is_zero().then(|| (Number::new(), Number::new()))
        }
    }
}

/// The exact numeric value of an `x`-free structure (used for the rational
/// coefficients of a trigonometric argument).
fn const_number(m: &MathStructure) -> Option<Number> {
    let mut c = m.clone();
    ev(&mut c);
    match c {
        MathStructure::Number(n) if n.is_rational() => Some(n),
        _ => None,
    }
}

/// Expand `m` into a polynomial in `sin(u)` and `cos(u)`.
fn sc_poly(
    m: &MathStructure,
    xvar: &MathStructure,
    base: &BaseAngle,
    depth: usize,
) -> Option<ScPoly> {
    if depth > 24 {
        return None;
    }
    if !contains_var(m, xvar) {
        return Some(ScPoly::constant(const_value(m)?));
    }
    match m {
        MathStructure::Addition(v) => {
            let mut acc = ScPoly::default();
            for t in v {
                acc = acc.add(&sc_poly(t, xvar, base, depth + 1)?);
            }
            Some(acc)
        }
        MathStructure::Multiplication(v) => {
            let mut acc = ScPoly::constant(1.0);
            for f in v {
                acc = acc.mul(&sc_poly(f, xvar, base, depth + 1)?)?;
            }
            Some(acc)
        }
        MathStructure::Power { base: b, exponent } => {
            let MathStructure::Number(e) = &**exponent else {
                return None;
            };
            let k = e.to_i64().filter(|k| (0..=12).contains(k))?;
            let inner = sc_poly(b, xvar, base, depth + 1)?;
            let mut acc = ScPoly::constant(1.0);
            for _ in 0..k {
                acc = acc.mul(&inner)?;
            }
            Some(acc)
        }
        MathStructure::Function { id, args } if args.len() == 1 => {
            let sine = match id.0 {
                bid::SIN => true,
                bid::COS => false,
                _ => return None,
            };
            let (a, b) = linear_form(&args[0], xvar)?;
            // a = m·α and b = m·β for a positive integer m.
            let mut mult = a.clone();
            if !mult.divide(&base.alpha) {
                return None;
            }
            let m_i = mult.to_i64().filter(|m| (1..=12).contains(m))?;
            let mut expect = base.beta.clone();
            if !expect.multiply(&Number::from_i64(m_i)) || !expect.equals(&b, false, false) {
                return None;
            }
            chebyshev(m_i as u32, sine)
        }
        _ => None,
    }
}

/// θ as an exact rational multiple of π, or `None` when it is not one.
fn pi_fraction(theta: f64) -> Option<Number> {
    let v = theta / std::f64::consts::PI;
    for q in 1..=720i64 {
        let p = (v * q as f64).round();
        if p.abs() > 1.0e12 {
            return None;
        }
        if (v - p / q as f64).abs() < 1.0e-9 {
            return Some(Number::from_ints(p as i64, q, 0));
        }
    }
    None
}

fn frac(p: i64, q: i64) -> Number {
    Number::from_ints(p, q, 0)
}

/// `sin(u) = r` — `MathStructure-isolatex.cc:6052`: `r = 0` gives `π n`,
/// `r = 1` gives `2 π n + π/2`, and otherwise the two branches
/// `asin(r) + 2 π n` and `π - asin(r) + 2 π n`.
fn invert_sin(r: f64) -> Option<Vec<Family>> {
    let one = Number::from_i64(1);
    let two = Number::from_i64(2);
    if r.abs() < 1e-12 {
        return Some(vec![Family { a: one, b: Number::new() }]);
    }
    if (r - 1.0).abs() < 1e-12 {
        return Some(vec![Family { a: two, b: frac(1, 2) }]);
    }
    if (r + 1.0).abs() < 1e-12 {
        return Some(vec![Family { a: two, b: frac(-1, 2) }]);
    }
    if r.abs() > 1.0 {
        return Some(Vec::new());
    }
    let t = pi_fraction(r.asin())?;
    let mut alt = Number::from_i64(1);
    if !alt.subtract(&t) {
        return None;
    }
    Some(vec![
        Family { a: two.clone(), b: t },
        Family { a: two, b: alt },
    ])
}

/// `cos(u) = r`: `r = 0` gives `π n - π/2`, `r = 1` gives `2 π n`, and
/// otherwise `± acos(r) + 2 π n`.
fn invert_cos(r: f64) -> Option<Vec<Family>> {
    let one = Number::from_i64(1);
    let two = Number::from_i64(2);
    if r.abs() < 1e-12 {
        return Some(vec![Family { a: one, b: frac(-1, 2) }]);
    }
    if (r - 1.0).abs() < 1e-12 {
        return Some(vec![Family { a: two, b: Number::new() }]);
    }
    if (r + 1.0).abs() < 1e-12 {
        return Some(vec![Family { a: two, b: Number::from_i64(1) }]);
    }
    if r.abs() > 1.0 {
        return Some(Vec::new());
    }
    let t = pi_fraction(r.acos())?;
    let mut neg = t.clone();
    if !neg.negate() {
        return None;
    }
    Some(vec![
        Family { a: two.clone(), b: t },
        Family { a: two, b: neg },
    ])
}

/// The angle of the point `(cos, sin) = (c, s)` on the unit circle, with the
/// full `2 π` period.
fn invert_pair(s: f64, c: f64) -> Option<Vec<Family>> {
    if (s * s + c * c - 1.0).abs() > 1e-8 {
        return Some(Vec::new());
    }
    let t = pi_fraction(s.atan2(c))?;
    Some(vec![Family { a: Number::from_i64(2), b: t }])
}

/// Real roots of a float polynomial on `[-1, 1]` (the range of `sin`).
fn unit_roots(coef: &[f64]) -> Vec<f64> {
    const SAMPLES: usize = 4000;
    let eval = |x: f64| coef.iter().rev().fold(0.0, |acc, c| acc * x + c);
    let mut out: Vec<f64> = Vec::new();
    let push = |out: &mut Vec<f64>, r: f64| {
        if !out.iter().any(|o: &f64| (o - r).abs() < 1e-7) {
            out.push(r);
        }
    };
    let mut prev = (-1.0f64, eval(-1.0));
    if prev.1.abs() < 1e-11 {
        push(&mut out, -1.0);
    }
    for i in 1..=SAMPLES {
        let x = -1.0 + 2.0 * i as f64 / SAMPLES as f64;
        let y = eval(x);
        if y.abs() < 1e-11 {
            push(&mut out, x);
        } else if (prev.1 < 0.0) != (y < 0.0) && prev.1.abs() > 1e-11 {
            // Bisection, bounded.
            let (mut lo, mut hi) = (prev.0, x);
            let lo_neg = prev.1 < 0.0;
            for _ in 0..200 {
                let mid = (lo + hi) / 2.0;
                if mid == lo || mid == hi {
                    break;
                }
                if (eval(mid) < 0.0) == lo_neg {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            push(&mut out, (lo + hi) / 2.0);
        }
        prev = (x, y);
    }
    out
}

/// Solve a trigonometric equation `expr = 0`.
pub fn solve_trig(expr: &MathStructure, xvar: &MathStructure) -> Option<Vec<MathStructure>> {
    // The base angle: the trigonometric argument with the smallest |α|.
    let mut args: Vec<(Number, Number, bool)> = Vec::new();
    collect_trig_args(expr, xvar, &mut args, 0)?;
    if args.is_empty() {
        return None;
    }
    let mut base_i = 0usize;
    for (i, a) in args.iter().enumerate() {
        let mut cur = a.0.clone();
        let mut best = args[base_i].0.clone();
        cur.abs();
        best.abs();
        if cur.is_less_than(&best) {
            base_i = i;
        }
    }
    let base = BaseAngle {
        alpha: args[base_i].0.clone(),
        beta: args[base_i].1.clone(),
    };
    if base.alpha.is_zero() {
        return None;
    }

    // `tan` is only inverted when the equation is linear in it.
    if args.iter().any(|a| a.2) {
        return solve_tan(expr, xvar, &base, &args);
    }

    let poly = sc_poly(expr, xvar, &base, 0)?.reduce_cos()?;
    if poly.is_zero() {
        return None;
    }
    let deg = |sel: u8| -> Vec<f64> {
        let n = poly
            .terms
            .iter()
            .filter(|((_, j), _)| *j == sel)
            .map(|((i, _), _)| *i as usize)
            .max();
        let mut v = vec![0.0; n.map_or(0, |n| n + 1)];
        for ((i, j), c) in &poly.terms {
            if *j == sel {
                v[*i as usize] = *c;
            }
        }
        v
    };
    let mut p = deg(0);
    let mut q = deg(1);

    // Pull out the common `S^a` factor: `S = 0` is a solution of its own.
    let low = |v: &[f64]| v.iter().position(|c| c.abs() > 1e-12);
    let shift = match (low(&p), low(&q)) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    let mut fams: Vec<Family> = Vec::new();
    if shift > 0 {
        p.drain(..shift.min(p.len()));
        q.drain(..shift.min(q.len()));
        fams.extend(invert_sin(0.0)?);
    }
    let is_const = |v: &[f64]| v.iter().skip(1).all(|c| c.abs() <= 1e-12);

    if q.iter().all(|c| c.abs() <= 1e-12) {
        // P(S) = 0
        for r in unit_roots(&p) {
            fams.extend(invert_sin(r)?);
        }
    } else if p.iter().all(|c| c.abs() <= 1e-12) {
        // C · Q(S) = 0
        fams.extend(invert_cos(0.0)?);
        for r in unit_roots(&q) {
            fams.extend(invert_sin(r)?);
        }
    } else if is_const(&p) && is_const(&q) {
        // P + C Q = 0 with both constant: a plain `cos(u) = -P/Q`.
        fams.extend(invert_cos(-p[0] / q[0])?);
    } else {
        // Mixed: eliminate C with `C = -P/Q` and `C^2 = 1 - S^2`, i.e.
        // `P^2 - Q^2 (1 - S^2) = 0`, then keep the consistent roots.
        let p2 = poly_mul(&p, &p);
        let q2 = poly_mul(&q, &q);
        let mut rhs = poly_mul(&q2, &[1.0, 0.0, -1.0]);
        for (i, c) in rhs.iter_mut().enumerate() {
            *c = p2.get(i).copied().unwrap_or(0.0) - *c;
        }
        let evalp = |v: &[f64], x: f64| v.iter().rev().fold(0.0, |acc, c| acc * x + c);
        for s in unit_roots(&rhs) {
            let qv = evalp(&q, s);
            if qv.abs() < 1e-12 {
                continue;
            }
            fams.extend(invert_pair(s, -evalp(&p, s) / qv)?);
        }
    }

    families_to_solutions(fams, &base)
}

fn poly_mul(a: &[f64], b: &[f64]) -> Vec<f64> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut r = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            r[i + j] += x * y;
        }
    }
    r
}

/// `k · tan(u) + d = 0` → `u = atan(-d/k) + π n`.
fn solve_tan(
    expr: &MathStructure,
    xvar: &MathStructure,
    base: &BaseAngle,
    args: &[(Number, Number, bool)],
) -> Option<Vec<MathStructure>> {
    if args.len() != 1 || !args[0].2 {
        return None;
    }
    if !args[0].0.equals(&base.alpha, false, false) {
        return None;
    }
    // Two numeric probes recover `k` and `d` from `expr = k·tan(u) + d`
    // without expanding the structure: `tan(u)` is replaced by a symbol.
    let tanf = func(bid::TAN, vec![rebuild_angle(base, xvar)]);
    let probe = |t: f64| -> Option<f64> {
        let mut m = expr.clone();
        let mut n = Number::new();
        n.set_float(t);
        replace(&mut m, &tanf, &MathStructure::Number(n));
        const_value(&m)
    };
    let d = probe(0.0)?;
    let k = probe(1.0)? - d;
    if k.abs() < 1e-12 {
        return None;
    }
    let t = pi_fraction((-d / k).atan())?;
    families_to_solutions(
        vec![Family {
            a: Number::from_i64(1),
            b: t,
        }],
        base,
    )
}

fn rebuild_angle(base: &BaseAngle, xvar: &MathStructure) -> MathStructure {
    let mut terms = Vec::new();
    if !base.alpha.is_zero() {
        terms.push(MathStructure::Multiplication(vec![
            MathStructure::Number(base.alpha.clone()),
            xvar.clone(),
        ]));
    }
    if !base.beta.is_zero() {
        terms.push(MathStructure::Multiplication(vec![
            MathStructure::Number(base.beta.clone()),
            MathStructure::symbolic("pi"),
        ]));
    }
    let mut m = if terms.len() == 1 {
        terms.pop().expect("len 1")
    } else {
        MathStructure::Addition(terms)
    };
    ev(&mut m);
    m
}

/// Collect `(α, β, is_tan)` for every trigonometric call in `m`.
fn collect_trig_args(
    m: &MathStructure,
    xvar: &MathStructure,
    out: &mut Vec<(Number, Number, bool)>,
    depth: usize,
) -> Option<()> {
    if depth > 24 {
        return None;
    }
    if let MathStructure::Function { id, args } = m {
        let is_trig = matches!(id.0, bid::SIN | bid::COS | bid::TAN);
        if is_trig && args.len() == 1 && contains_var(&args[0], xvar) {
            let (a, b) = linear_form(&args[0], xvar)?;
            if a.is_zero() {
                return None;
            }
            out.push((a, b, id.0 == bid::TAN));
            return Some(());
        }
        // Any other function of `x` puts the equation out of reach.
        if args.iter().any(|a| contains_var(a, xvar)) {
            return None;
        }
        return Some(());
    }
    for i in 0..m.size() {
        if let Some(c) = m.get(i) {
            if contains_var(c, xvar) {
                collect_trig_args(c, xvar, out, depth + 1)?;
            }
        }
    }
    Some(())
}

/// `u = A π n + B π` → `x = (u - β π) / α`, deduplicated and ordered the way
/// the reference prints the `or` chain: the families with no constant term
/// first, then by decreasing constant.
fn families_to_solutions(fams: Vec<Family>, base: &BaseAngle) -> Option<Vec<MathStructure>> {
    let mut out: Vec<Family> = Vec::new();
    for f in fams {
        let mut a = f.a.clone();
        let mut b = f.b.clone();
        if !b.subtract(&base.beta) || !a.divide(&base.alpha) || !b.divide(&base.alpha) {
            return None;
        }
        if a.is_negative() && !a.negate() {
            return None;
        }
        // Fold the constant into `(-a/2, a/2]`-ish only far enough to make
        // duplicate families compare equal.
        if out.iter().any(|o| same_family(o, &a, &b)) {
            continue;
        }
        out.push(Family { a, b });
    }
    if out.is_empty() {
        return None;
    }
    out.sort_by(|x, y| {
        let xz = x.b.is_zero();
        let yz = y.b.is_zero();
        yz.cmp(&xz).then_with(|| {
            if x.b.is_greater_than(&y.b) {
                std::cmp::Ordering::Less
            } else if y.b.is_greater_than(&x.b) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });
    Some(out.iter().map(family_structure).collect())
}

/// Two families describe the same set when their periods agree and their
/// constants differ by a whole number of periods.
fn same_family(o: &Family, a: &Number, b: &Number) -> bool {
    if !o.a.equals(a, false, false) {
        return false;
    }
    let mut d = b.clone();
    if !d.subtract(&o.b) || !d.divide(a) {
        return false;
    }
    d.is_integer()
}

fn family_structure(f: &Family) -> MathStructure {
    let n_term = pi_term(&f.a, true);
    if f.b.is_zero() {
        return n_term;
    }
    MathStructure::Addition(vec![n_term, pi_term(&f.b, false)])
}

/// `c · π` (times `n` when `with_n`), in the shapes the reference prints:
/// `2 pi * n`, `(2/3) * pi * n`, `(pi * n) / 2`, `pi / 6`, `(5/18) * pi`.
fn pi_term(c: &Number, with_n: bool) -> MathStructure {
    let pi = MathStructure::symbolic("pi");
    let nn = MathStructure::symbolic("n");
    let numer = c.numerator();
    let denom = c.denominator();
    let unit_numer = numer.is_one() || numer.equals(&Number::from_i64(-1), false, false);
    let mut factors: Vec<MathStructure> = Vec::new();
    if denom.is_one() {
        if !numer.is_one() {
            factors.push(MathStructure::Number(numer));
        }
        factors.push(pi);
        if with_n {
            factors.push(nn);
        }
    } else if unit_numer {
        if numer.is_negative() {
            factors.push(num(-1));
        }
        factors.push(pi);
        if with_n {
            factors.push(nn);
        }
        factors.push(MathStructure::Power {
            base: Box::new(MathStructure::Number(denom)),
            exponent: Box::new(num(-1)),
        });
    } else {
        factors.push(MathStructure::Number(c.clone()));
        factors.push(pi);
        if with_n {
            factors.push(nn);
        }
    }
    if factors.len() == 1 {
        factors.pop().expect("len 1")
    } else {
        MathStructure::Multiplication(factors)
    }
}

/// Cardano's formula for `a x^3 + b x^2 + c x + d = 0`, kept in radicals.
///
/// The depressed cubic `t^3 + p t + q` (with `x = t - b/(3a)`) has
/// `Δ = q²/4 + p³/27`. Writing `E = 27q/2` and `F = 729 Δ = 729q²/4 + 27p³`
/// — both rational — the real root is
///
/// ```text
///     C = cbrt(sqrt(F) + E)
///     x = p / C - C / 3 - b / (3a)
/// ```
///
/// which is exactly the shape the reference prints:
/// `2 / (3 * cbrt(3 * sqrt(561) - 71)) - 1/3 - cbrt(3 * sqrt(561) - 71) / 3`.
///
/// `F < 0` is the casus irreducibilis (three real irrational roots, whose
/// radical form needs complex cube roots); the reference does not print
/// those either, so it is left unsolved. TODO(port).
fn cubic_roots(a: &Number, b: &Number, c: &Number, d: &Number) -> Option<Vec<MathStructure>> {
    let n = |i: i64| Number::from_i64(i);
    let chk = |ok: bool| if ok { Some(()) } else { None };

    // p = (3ac - b²) / (3a²), q = (2b³ - 9abc + 27a²d) / (27a³)
    let mut p = a.clone();
    chk(p.multiply(c) && p.multiply(&n(3)))?;
    let mut b2 = b.clone();
    chk(b2.square())?;
    chk(p.subtract(&b2))?;
    let mut a2 = a.clone();
    chk(a2.square())?;
    let mut den = a2.clone();
    chk(den.multiply(&n(3)) && p.divide(&den))?;

    let mut q = b2.clone();
    chk(q.multiply(b) && q.multiply(&n(2)))?; // 2b³
    let mut t = a.clone();
    chk(t.multiply(b) && t.multiply(c) && t.multiply(&n(9)))?;
    chk(q.subtract(&t))?; // -9abc
    let mut t = a2.clone();
    chk(t.multiply(d) && t.multiply(&n(27)))?;
    chk(q.add(&t))?; // +27a²d
    let mut den = a2.clone();
    chk(den.multiply(a) && den.multiply(&n(27)) && q.divide(&den))?;

    // A depressed cubic with no linear term is a plain cube root:
    // `x^3 = 2` is `cbrt(2)` and `x^3 = -2` is `-cbrt(2)`. Cardano's
    // formula cannot be used here — `27q/2 + sqrt((27q/2)^2)` cancels to
    // zero for `q < 0`. With a shift the reference leaves the equation
    // unsolved (`x^3 + 3x^2 + 3x = 5` prints back unchanged), so this
    // follows it.
    if p.is_zero() {
        if !b.is_zero() {
            return None;
        }
        let mut r = q.clone();
        chk(r.negate())?;
        let negative = r.is_negative();
        if negative {
            chk(r.negate())?;
        }
        let mut exact = r.clone();
        let body = if exact.raise(&Number::from_ints(1, 3, 0), true) && !exact.is_approximate() {
            MathStructure::Number(exact)
        } else {
            func(bid::CBRT, vec![MathStructure::Number(r)])
        };
        return Some(vec![if negative {
            MathStructure::Multiplication(vec![num(-1), body])
        } else {
            body
        }]);
    }

    // E = 27q/2, F = 729q²/4 + 27p³ = (27q/2)² + 27p³
    let mut e = q.clone();
    chk(e.multiply(&n(27)) && e.divide(&n(2)))?;
    let mut f = e.clone();
    chk(f.square())?;
    let mut p3 = p.clone();
    chk(p3.square() && p3.multiply(&p) && p3.multiply(&n(27)))?;
    chk(f.add(&p3))?;
    if f.is_negative() {
        return None;
    }

    // C = cbrt(sqrt(F) + E). A perfect square collapses to a plain number.
    let radicand = radical(&f)?;
    let inner = match radicand {
        MathStructure::Number(mut r) => {
            chk(r.add(&e))?;
            MathStructure::Number(r)
        }
        s if e.is_zero() => s,
        s => MathStructure::Addition(vec![s, MathStructure::Number(e.clone())]),
    };
    let cbrt_c = func(bid::CBRT, vec![inner]);

    let mut terms: Vec<MathStructure> = Vec::new();
    // p / C, printed as `2 / (3 * cbrt(D))` — the numerator and the
    // denominator of `p` stay split so the printer recovers that fraction.
    if !p.is_zero() {
        let pn = p.numerator();
        let pd = p.denominator();
        let denom = if pd.is_one() {
            cbrt_c.clone()
        } else {
            MathStructure::Multiplication(vec![MathStructure::Number(pd), cbrt_c.clone()])
        };
        terms.push(MathStructure::Multiplication(vec![
            MathStructure::Number(pn),
            MathStructure::Power {
                base: Box::new(denom),
                exponent: Box::new(num(-1)),
            },
        ]));
    }
    // -b / (3a)
    let mut shift = b.clone();
    let mut a3 = a.clone();
    chk(a3.multiply(&n(3)) && shift.divide(&a3) && shift.negate())?;
    if !shift.is_zero() {
        terms.push(MathStructure::Number(shift));
    }
    // -C / 3
    terms.push(MathStructure::Multiplication(vec![
        num(-1),
        cbrt_c,
        MathStructure::Power {
            base: Box::new(num(3)),
            exponent: Box::new(num(-1)),
        },
    ]));
    Some(vec![MathStructure::Addition(terms)])
}

/// `sqrt(n)` for a non-negative rational `n`, with the square factors pulled
/// out: `sqrt(5049)` becomes `3 * sqrt(561)`, which is what the reference
/// prints. A perfect square returns a plain `Number`.
fn radical(n: &Number) -> Option<MathStructure> {
    if n.is_zero() {
        return Some(MathStructure::Number(Number::new()));
    }
    // sqrt(a/b) = sqrt(a*b) / b
    let (coeff, radicand) = sqrt_parts(n)?;
    let Some(radicand) = radicand else {
        return Some(MathStructure::Number(coeff));
    };
    let root = MathStructure::Power {
        base: Box::new(MathStructure::Number(Number::from_i64(radicand))),
        exponent: Box::new(MathStructure::Number(Number::from_ints(1, 2, 0))),
    };
    if coeff.is_one() {
        Some(root)
    } else {
        Some(MathStructure::Multiplication(vec![
            MathStructure::Number(coeff),
            root,
        ]))
    }
}

// ----------------------------------------------------------------------
// Numeric solving (newtonsolve / secantsolve and the approximate mode)
// ----------------------------------------------------------------------

/// Evaluate `expr` with `xvar` replaced by the number `x`, returning the
/// numeric value when the substitution reduces to one.
pub fn eval_at(expr: &MathStructure, xvar: &MathStructure, x: &Number) -> Option<Number> {
    let mut m = expr.clone();
    replace(&mut m, xvar, &MathStructure::Number(x.clone()));
    let mut eo2 = eo();
    eo2.approximation = crate::options::ApproximationMode::Approximate;
    for _ in 0..8 {
        let changed = crate::builtins::calculate_functions(&mut m);
        let merged = m.calculatesub(&eo2);
        if !changed && !merged {
            break;
        }
    }
    match m {
        MathStructure::Number(n) => Some(n),
        _ => None,
    }
}

/// Replace every occurrence of `from` with `to` (C++ `MathStructure::replace`).
pub fn replace(m: &mut MathStructure, from: &MathStructure, to: &MathStructure) {
    if m.equals(from) {
        *m = to.clone();
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get_mut(i) {
            replace(c, from, to);
        }
    }
}

/// An iterate as a floating-point value, so its size stays bounded by the
/// working precision.
fn float_iterate(n: &Number) -> Number {
    let mut f = n.clone();
    if !f.set_to_floating_point() {
        return n.clone();
    }
    f
}

/// Secant iteration on `expr = 0` starting from `x0`, `x1`.
pub fn secant_solve(
    expr: &MathStructure,
    xvar: &MathStructure,
    x0: &Number,
    x1: &Number,
) -> Option<Number> {
    // The iterates are kept floating-point. An exact rational start makes
    // every secant step an exact rational too, and `f(x) = x^4 - 2` then
    // quadruples the digit count per iteration — 55000 digits by the eighth
    // step, long before the convergence test can stop the loop.
    let mut a = float_iterate(x0);
    let mut b = float_iterate(x1);
    let mut fa = eval_at(expr, xvar, &a)?;
    let mut fb = eval_at(expr, xvar, &b)?;
    // Every arithmetic failure ends the iteration rather than the whole
    // solve: once the secant has converged, `f(b) - f(a)` can underflow and
    // the step can no longer be formed, but `b` is already the root. The
    // iteration otherwise stops on the relative step size (`converged`).
    for _ in 0..200 {
        let mut denom = fb.clone();
        if !denom.subtract(&fa) || denom.is_zero() {
            break;
        }
        let mut step = b.clone();
        if !step.subtract(&a) {
            break;
        }
        if !step.multiply(&fb) || !step.divide(&denom) {
            break;
        }
        let mut c = b.clone();
        if !c.subtract(&step) {
            break;
        }
        c = float_iterate(&c);
        let Some(fc) = eval_at(expr, xvar, &c) else {
            break;
        };
        a = b;
        fa = fb;
        b = c;
        fb = fc;
        if fb.is_zero() {
            break;
        }
        if converged(&step, &b) {
            break;
        }
    }
    Some(b)
}

/// The convergence test of `NewtonRaphsonFunction::calculate`
/// (`BuiltinFunctions-algebra.cc:1544`): the step is divided by the iterate
/// and the *relative* size compared against
/// `nr_prec = 10^-(PRECISION - arg4)`, with the default fourth argument
/// `-10`. A zero iterate falls back to the absolute size, as it does there.
///
/// The test has to be relative. An absolute floor cannot be reached: once the
/// iteration has converged the step is pure rounding noise, whose size is set
/// by the working precision (about `1e-30` for a root of order one), and
/// every further step then *widens* the result — `newtonsolve(Ei(x) = 3i, 1)`
/// converges by the thirteenth iteration and drifts back out to `-1.7 + 0.5i`
/// by the two hundredth. The real cases hid this: there `f(b) - f(a)`
/// underflows to zero at convergence, which ends the loop by itself, but a
/// complex `f` keeps producing a nonzero difference indefinitely.
fn converged(step: &Number, x: &Number) -> bool {
    let mut mag = step.clone();
    if !mag.abs() {
        return false;
    }
    let mut tol = Number::from_ints(1, 1, -(qalc_num::context::precision() as i64 + 10));
    if !x.is_zero() {
        let mut scale = x.clone();
        if !scale.abs() || !tol.multiply(&scale) {
            return false;
        }
    }
    mag.is_less_than(&tol)
}

/// Newton iteration where the C++ uses the symbolic `diff`.
///
/// [`crate::differentiate::differentiate`] exists, but this seeds a secant
/// step at `x0 * (1 + 1e-8)` instead: the secant iteration is already here for
/// `secantsolve`, and a numeric derivative avoids the symbolic derivative
/// failing on the very expressions `newtonsolve` is reached for.
pub fn newton_solve(
    expr: &MathStructure,
    xvar: &MathStructure,
    x0: &Number,
) -> Option<Number> {
    let mut h = x0.clone();
    if !h.multiply(&Number::from_ints(1, 1, -8)) {
        return None;
    }
    let mut eps = Number::from_ints(1, 1, -8);
    if h.is_zero() {
        h = eps.clone();
    }
    let _ = &mut eps;
    let mut x1 = x0.clone();
    if !x1.add(&h) {
        return None;
    }
    secant_solve(expr, xvar, x0, &x1)
}

// ----------------------------------------------------------------------
// Comparison evaluation
// ----------------------------------------------------------------------

/// The `eo.isolate_x` hook: solve a top-level equation for its unknown.
///
/// Only an `=` comparison that actually contains an unknown is touched, so
/// numeric comparisons and everything else keep their existing behaviour.
/// Returns true when the comparison was replaced by a solution, so the caller
/// knows the structure needs another trip through the merge engine.
pub fn isolate_x_toplevel(m: &mut MathStructure, eo: &EvaluationOptions) -> bool {
    if !matches!(m, MathStructure::Comparison { op: ComparisonType::Equals, .. }) {
        return false;
    }
    // Solving is a top-level step. Several builtins re-enter the evaluator on
    // their own arguments, and the evaluator ends with this call, so without
    // a guard `x^3 = 5` recurses: solve -> evaluate -> solve -> ... The C++
    // isolates once, at the top, which is what this reproduces.
    if SOLVING.with(|s| s.get()) {
        return false;
    }
    SOLVING.with(|s| s.set(true));
    CURRENT_EO.with(|c| *c.borrow_mut() = eo.clone());
    let mut solved = false;
    if let Some(xvar) = poly::find_x_var(m) {
        solved = isolate_x(m, &xvar);
    }
    SOLVING.with(|s| s.set(false));
    solved
}

thread_local! {
    /// True while `isolate_x_toplevel` is on the stack.
    static SOLVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Try to isolate `xvar` in a comparison, rewriting it in place.
/// Returns true when the comparison was replaced by a solution.
pub fn isolate_x(m: &mut MathStructure, xvar: &MathStructure) -> bool {
    let MathStructure::Comparison { left, op, right } = m else {
        return false;
    };
    if *op != ComparisonType::Equals {
        return false;
    }
    let Some(sols) = solve_equation(left, right, xvar) else {
        return false;
    };
    *m = solutions_to_structure(xvar, sols);
    true
}

/// `x = a or x = b or ...` (a `LogicalOr` of comparisons), matching the
/// reference output shape.
pub fn solutions_to_structure(xvar: &MathStructure, sols: Vec<MathStructure>) -> MathStructure {
    let mut cmps: Vec<MathStructure> = Vec::new();
    for s in sols {
        cmps.push(MathStructure::Comparison {
            left: Box::new(xvar.clone()),
            op: ComparisonType::Equals,
            right: Box::new(s),
        });
    }
    if cmps.len() == 1 {
        cmps.into_iter().next().expect("len 1")
    } else {
        MathStructure::LogicalOr(cmps)
    }
}

// ----------------------------------------------------------------------
// Builtin dispatch
// ----------------------------------------------------------------------

pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    let args = args.clone();
    match fid {
        id::SOLVE => {
            let Some(MathStructure::Comparison { left, op, right }) = args.first() else {
                return false;
            };
            if *op != ComparisonType::Equals {
                return false;
            }
            let Some(xvar) = args
                .get(1)
                .filter(|a| a.is_symbolic())
                .cloned()
                .or_else(|| poly::find_x_var(&args[0]))
            else {
                return false;
            };
            let Some(sols) = solve_equation(left, right, &xvar) else {
                return false;
            };
            *m = solutions_to_structure(&xvar, sols);
            true
        }
        id::NEWTON_RAPHSON | id::SECANT_METHOD => {
            let Some(MathStructure::Comparison { left, op, right }) = args.first() else {
                return false;
            };
            if *op != ComparisonType::Equals {
                return false;
            }
            let Some(xvar) = poly::find_x_var(&args[0]) else {
                return false;
            };
            let mut expr = (**left).clone();
            expr.calculate_subtract((**right).clone(), &eo());
            let guess = |i: usize| -> Option<Number> {
                match args.get(i) {
                    Some(MathStructure::Number(n)) => Some(n.clone()),
                    _ => None,
                }
            };
            let Some(g1) = guess(1) else { return false };
            let root = if fid == id::NEWTON_RAPHSON {
                newton_solve(&expr, &xvar, &g1)
            } else {
                let Some(g2) = guess(2) else { return false };
                secant_solve(&expr, &xvar, &g1, &g2)
            };
            match root {
                Some(r) => {
                    *m = MathStructure::Number(r);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let id = match name {
        "solve" => id::SOLVE,
        "newtonsolve" => id::NEWTON_RAPHSON,
        "secantsolve" => id::SECANT_METHOD,
        _ => return None,
    };
    Some(FunctionId(id))
}

pub fn function_name(id: u32) -> Option<&'static str> {
    Some(match id {
        id::SOLVE => "solve",
        id::NEWTON_RAPHSON => "newtonsolve",
        id::SECANT_METHOD => "secantsolve",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::session::Session;

    /// Solve with the transcript's settings (`solver.batch` opens with
    /// `/set approximation exact` and `/set fr 2`). Every expected value in
    /// this module comes from the reference binary under the same settings.
    fn ex(expr: &str) -> String {
        let mut s = Session::new();
        s.evaluate_line("/set approximation exact").expect("set");
        s.evaluate_line("/set fr 2").expect("set");
        s.evaluate_line(expr).expect("evaluates")
    }

    /// Solve in the default (`try exact`) mode, with the Unicode signs the
    /// transcript turns on before the approximate cases.
    fn ap(expr: &str) -> String {
        let mut s = Session::new();
        s.evaluate_line("/set unicode 1").expect("set");
        s.evaluate_line(expr).expect("evaluates")
    }

    // ---- cubic radicals -------------------------------------------------

    #[test]
    fn cubic_without_rational_roots_uses_cardano() {
        assert_eq!(
            ex("x^3 + x^2 + x = 5"),
            "x = 2 / (3 * cbrt(3 * sqrt(561) - 71)) - 1/3 - cbrt(3 * sqrt(561) - 71) / 3"
        );
    }

    #[test]
    fn depressed_cubic_has_no_shift_term() {
        assert_eq!(
            ex("x^3 + 3x = 1"),
            "x = 3 / cbrt((27/2) * sqrt(5) - 27/2) - cbrt((27/2) * sqrt(5) - 27/2) / 3"
        );
    }

    #[test]
    fn cubic_without_a_linear_term_is_a_cube_root() {
        // Cardano would cancel to `cbrt(0)` here.
        assert_eq!(ex("x^3 = 2"), "x = cbrt(2)");
        assert_eq!(ex("x^3 - 6 = 0"), "x = cbrt(6)");
        assert_eq!(ex("x^3 = -2"), "x = -cbrt(2)");
        // With a shift the reference leaves it unsolved, and so does this.
        assert_eq!(ex("x^3 + 3x^2 + 3x = 5"), "x^3 + 3x^2 + 3x = 5");
    }

    #[test]
    fn secant_iterates_stay_bounded() {
        // Exact-rational iterates quadruple in length every step for
        // `x^4 - 2`; the float iterates keep this instant.
        assert_eq!(ap("secantsolve(x^5 = 2, 1, 2)"), "1.148698355");
        assert_eq!(ap("secantsolve(x^4 = 2, 1, 2)"), "1.189207115");
    }

    #[test]
    fn cubic_with_rational_roots_still_factors() {
        assert_eq!(ex("x^3 - 6x^2 + 11x - 6 = 0"), "x = 3 or x = 2 or x = 1");
        assert_eq!(ex("x^4 + 20x^3 + 150x^2 + 500x + 625 = 0"), "x = -5");
    }

    // ---- trigonometric general solutions --------------------------------

    #[test]
    fn sine_equal_to_one_has_a_single_family() {
        assert_eq!(ex("1/3 * sin(3x) - 1/3 = 0"), "x = (2/3) * pi * n + pi / 6");
    }

    #[test]
    fn sine_has_two_branches() {
        assert_eq!(
            ex("2/3 * sin(3x) - 1/3 = 0"),
            "x = (2/3) * pi * n + (5/18) * pi or x = (2/3) * pi * n + pi / 18"
        );
        assert_eq!(
            ex("sin(x) = -1/2"),
            "x = 2 pi * n + (7/6) * pi or x = 2 pi * n - pi / 6"
        );
    }

    #[test]
    fn cosine_branches_are_symmetric() {
        assert_eq!(
            ex("cos(x) = 1/2"),
            "x = 2 pi * n + pi / 3 or x = 2 pi * n - pi / 3"
        );
        assert_eq!(
            ex("sqrt(2) * cos(3x + pi/6) = 1"),
            "x = (2/3) * pi * n + pi / 36 or x = (2/3) * pi * n - (5/36) * pi"
        );
    }

    #[test]
    fn sine_plus_cosine_solves_as_a_point_on_the_circle() {
        assert_eq!(ex("sin(x) + cos(x) = 1"), "x = 2 pi * n or x = 2 pi * n + pi / 2");
        assert_eq!(
            ex("sin(x) = 1 + cos(x)"),
            "x = 2 pi * n + pi or x = 2 pi * n + pi / 2"
        );
        assert_eq!(
            ex("sqrt(3) * sin(x) + cos(x) = sqrt(3)"),
            "x = 2 pi * n + pi / 2 or x = 2 pi * n + pi / 6"
        );
    }

    #[test]
    fn tangent_has_the_half_period() {
        assert_eq!(ex("tan(x) = 1"), "x = pi * n + pi / 4");
        assert_eq!(ex("tan(x/4 + pi/3) = sqrt(3)"), "x = 4 pi * n");
    }

    #[test]
    fn zero_of_sine_is_pulled_out_as_a_factor() {
        assert_eq!(ex("sin(2x) = 0"), "x = (pi * n) / 2");
        assert_eq!(ex("sin(x)^2 = sin(x)^3"), "x = pi * n or x = 2 pi * n + pi / 2");
    }

    #[test]
    fn half_and_double_angles_share_one_base_angle() {
        // sin(x) = 2 sin(x/2) cos(x/2)
        assert_eq!(
            ex("sin(x) = sin(x/2)"),
            "x = 2 pi * n or x = 4 pi * n + (2/3) * pi or x = 4 pi * n - (2/3) * pi"
        );
        assert_eq!(
            ex("sin(4x) + cos(2x) = 0"),
            "x = pi * n + (7/12) * pi or x = pi * n - pi / 12 or x = (pi * n) / 2 - pi / 4"
        );
    }

    #[test]
    fn scaled_argument_scales_the_period() {
        assert_eq!(
            ex("2 * sin(3x/4) = 1"),
            "x = (8/3) * pi * n + (10/9) * pi or x = (8/3) * pi * n + (2/9) * pi"
        );
    }

    // ---- Lambert W -------------------------------------------------------

    #[test]
    fn logarithm_plus_linear_inverts_to_lambert_w() {
        assert_eq!(ex("ln(x) + x = 3"), "x = lambertw(e^3)");
        assert_eq!(ex("ln(x) + 2x = 4"), "x = lambertw(2e^4) / 2");
    }

    #[test]
    fn exponential_plus_linear_inverts_to_lambert_w() {
        assert_eq!(
            ex("2^(3x) + 4x = 5"),
            "x = 5/4 - lambertw(6 * 8^(1/4) * ln(2)) / (3 * ln(2))"
        );
        assert_eq!(
            ex("3^(2x) + x = 1"),
            "x = 1 - lambertw(18 * ln(3)) / (2 * ln(3))"
        );
    }

    #[test]
    fn x_to_the_x_uses_both_lambert_branches() {
        // The argument of W is in (-1/e, 0), so the k = -1 branch is real too.
        assert_eq!(
            ex("x^(-3x) = 2"),
            "x = e^lambertw(-ln(2) / 3) or x = e^lambertw(-ln(2) / 3, -1)"
        );
        // A positive argument has only the principal branch.
        assert_eq!(ex("x^(2x) = 3"), "x = e^lambertw(ln(3) / 2)");
    }

    // ---- substitution u = x^(1/k) ---------------------------------------

    #[test]
    fn fractional_powers_substitute_and_reject_the_complex_root() {
        assert_eq!(ex("x^(1/3) + x^(2/3) = 3"), "x = 2 * sqrt(13) - 5");
        assert_eq!(ex("x^(1/2) + x = 2"), "x = 1");
    }

    // ---- numeric fallback ------------------------------------------------

    #[test]
    fn approximate_solutions_print_with_the_almost_equal_sign() {
        assert_eq!(ap("x^7 - x^5 + 3x^2 + 5x = 3"), "x \u{2248} 0.4706753153");
        assert_eq!(ap("x^(5x) = 5"), "x \u{2248} 1.284730245");
    }

    #[test]
    fn secant_and_newton_converge_on_a_special_function() {
        assert_eq!(ap("newtonsolve(Ei(x) = 3, 1)"), "1.397510842");
        assert_eq!(ap("secantsolve(Ei(x) = 3, 1, 4)"), "1.397510842");
    }

    #[test]
    fn newton_stops_when_the_relative_step_is_negligible() {
        // A complex right-hand side takes the iterates off the real line, and
        // `f(b) - f(a)` then never underflows the way it does for a real root
        // — without the relative convergence test the iteration converges by
        // the thirteenth step and drifts back out over the next hundred.
        assert_eq!(
            ap("newtonsolve(Ei(x) = 3i, 1)"),
            "\u{2212}1.160849461 + 1.034283360i"
        );
        assert_eq!(
            ap("secantsolve(Ei(x) = 3i, 1, 2)"),
            "\u{2212}1.160849461 + 1.034283360i"
        );
    }
}
