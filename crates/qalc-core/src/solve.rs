//! Equation solving — port of `MathStructure::isolate_x`
//! (`MathStructure-isolatex.cc`) and the `solve()` builtin
//! (`BuiltinFunctions-algebra.cc`).
//!
//! The C++ isolates `x` by repeatedly moving everything else to the other
//! side of a `STRUCT_COMPARISON`, falling back to polynomial factorization
//! for higher degrees. This port implements the cases the transcripts use:
//!
//! - linear and quadratic equations (the quadratic formula, kept exact),
//! - polynomials of any degree that factor into rational roots,
//! - `a^x = b` → `x = ln(b)/ln(a)`,
//! - numeric root finding for `newtonsolve`/`secantsolve` and for the
//!   approximate cases.
//!
//! TODO(port): the trigonometric general solutions (`sin(x) = a` →
//! `x = 2*pi*n + asin(a)`), Lambert-W inversion, `x^(a x) = b`, cubic/quartic
//! radicals for non-rational roots, and interval/assumption filtering of the
//! solution set.

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

/// The options in force for the current top-level evaluation. The C++ passes
/// `EvaluationOptions` explicitly through `isolate_x`; this port stashes them
/// so the (recursive, structure-shaped) helpers stay simple.
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
        _ => {
            // TODO(port): cubic/quartic radicals and numeric fallback.
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

/// Secant iteration on `expr = 0` starting from `x0`, `x1`.
pub fn secant_solve(
    expr: &MathStructure,
    xvar: &MathStructure,
    x0: &Number,
    x1: &Number,
) -> Option<Number> {
    let mut a = x0.clone();
    let mut b = x1.clone();
    let mut fa = eval_at(expr, xvar, &a)?;
    let mut fb = eval_at(expr, xvar, &b)?;
    for _ in 0..200 {
        let mut denom = fb.clone();
        if !denom.subtract(&fa) {
            return None;
        }
        if denom.is_zero() {
            break;
        }
        let mut step = b.clone();
        if !step.subtract(&a) {
            return None;
        }
        if !step.multiply(&fb) || !step.divide(&denom) {
            return None;
        }
        let mut c = b.clone();
        if !c.subtract(&step) {
            return None;
        }
        let fc = eval_at(expr, xvar, &c)?;
        a = b;
        fa = fb;
        b = c;
        fb = fc;
        if fb.is_zero() {
            break;
        }
        let mut mag = step.clone();
        mag.abs();
        if mag.is_less_than(&Number::from_ints(1, 1, -40)) {
            break;
        }
    }
    Some(b)
}

/// Newton iteration using a numeric derivative (the C++ uses the symbolic
/// `diff`, which is not ported yet — TODO(port)).
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
pub fn isolate_x_toplevel(m: &mut MathStructure, eo: &EvaluationOptions) {
    if !matches!(m, MathStructure::Comparison { op: ComparisonType::Equals, .. }) {
        return;
    }
    CURRENT_EO.with(|c| *c.borrow_mut() = eo.clone());
    let Some(xvar) = poly::find_x_var(m) else {
        return;
    };
    isolate_x(m, &xvar);
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
