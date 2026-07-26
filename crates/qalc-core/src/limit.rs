//! Limits — port of `MathStructure::calculateLimit`
//! (`MathStructure-limit.cc`) and the `limit()` builtin
//! (`BuiltinFunctions-calculus.cc:744`).
//!
//! The C++ drives everything from one recursive `calculateLimit` plus the
//! asymptotic comparator `limit_inf_cmp`. This port keeps the same two
//! ingredients but arranges them around a single normalization: every limit
//! is first rewritten so the variable tends to **0** or to **+infinity**.
//!
//! * `x -> -infinity` becomes `x -> +infinity` after `x := -x`,
//! * `x -> a` (finite) becomes `x -> 0` after `x := x + a`.
//!
//! On the normalized problem three tools are applied in order:
//!
//! 1. [`lead`] — the asymptotic leading term `c * x^d` (the analogue of
//!    `limit_inf_cmp`). This settles every rational and radical case,
//!    including the ones a repeated L'Hopital would never finish
//!    (`((x-1)^100 (6x+1)^200) / (3x+5)^300`).
//! 2. structural recursion over sums, products, powers and function calls,
//!    with the standard rules for the determinate forms.
//! 3. L'Hopital's rule for `0/0` and `inf/inf`, using
//!    [`crate::differentiate`], after putting the expression over a common
//!    denominator ([`together`]); plus conjugate rationalization for the
//!    `inf - inf` and `0 - 0` forms that hide a radical.
//!
//! Everything is depth- and size-bounded: no unbounded loop exists in this
//! module, because a runaway limit would hang the whole batch runner.

use crate::builtins::id as bid;
use crate::differentiate::{contains, differentiate};
use crate::ids::FunctionId;
use crate::options::EvaluationOptions;
use crate::structure::MathStructure;
use qalc_num::Number;

pub mod id {
    /// `FUNCTION_ID_LIMIT` (BuiltinFunctions.h:353).
    pub const LIMIT: u32 = 1810;
}

// ----------------------------------------------------------------------
// Small builders
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

fn inv(m: MathStructure) -> MathStructure {
    pow(m, num(-1))
}

fn e_sym() -> MathStructure {
    MathStructure::symbolic("e")
}

fn pi_sym() -> MathStructure {
    MathStructure::symbolic("pi")
}

fn plus_inf() -> MathStructure {
    let mut n = Number::new();
    n.set_plus_infinity(false, false);
    nr(n)
}

fn minus_inf() -> MathStructure {
    let mut n = Number::new();
    n.set_minus_infinity(false, false);
    nr(n)
}

/// Evaluate with `APPROXIMATION_EXACT`, like `LimitFunction::calculate`
/// (which forces `eo2.approximation = APPROXIMATION_EXACT`).
fn eo() -> EvaluationOptions {
    EvaluationOptions::exact()
}

fn ev(m: &mut MathStructure) {
    let o = eo();
    for _ in 0..8 {
        let changed = crate::builtins::calculate_functions_eo(m, &o);
        let merged = m.calculatesub(&o);
        simplify_local(m, 0);
        if !changed && !merged {
            break;
        }
    }
    crate::sort::sort(m);
}

/// Two reductions the general merge engine deliberately leaves alone but a
/// limit calculation depends on:
///
/// * `0 * f(x)` is zero — the engine keeps the factor because
///   `representsNumber` answers conservatively for function calls, so
///   `sqrt(3) - sqrt(3)` collapses only to `0 sqrt(3)`.
/// * `ln(e) = 1` and `ln(e^k) = k` — `e` is a plain symbol in this port, so
///   no numeric rule fires.
fn simplify_local(m: &mut MathStructure, depth: usize) {
    if depth > 24 {
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get_mut(i) {
            simplify_local(c, depth + 1);
        }
    }
    match m {
        MathStructure::Multiplication(v) => {
            if v.iter().any(|f| matches!(f, MathStructure::Number(n) if n.is_zero())) {
                *m = num(0);
                return;
            }
            // `e^x * e^-x` is 1, but the merge engine only combines powers
            // with *numeric* exponents, so a symbolic exponent survives.
            if let Some(c) = combine_same_base_powers(v) {
                *m = c;
            }
        }
        MathStructure::Function { id, args } if id.0 == bid::LN && args.len() == 1 => {
            match &args[0] {
                MathStructure::Symbolic(s) if s == "e" => *m = num(1),
                MathStructure::Power { base, exponent }
                    if matches!(base.as_ref(), MathStructure::Symbolic(s) if s == "e") =>
                {
                    *m = (**exponent).clone();
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Fold `b^p * b^q` into `b^(p+q)` inside a product.
fn combine_same_base_powers(v: &[MathStructure]) -> Option<MathStructure> {
    let split = |f: &MathStructure| -> (MathStructure, MathStructure) {
        match f {
            MathStructure::Power { base, exponent } => ((**base).clone(), (**exponent).clone()),
            other => (other.clone(), num(1)),
        }
    };
    let mut bases: Vec<MathStructure> = Vec::new();
    let mut exps: Vec<Vec<MathStructure>> = Vec::new();
    let mut merged = false;
    for f in v {
        let (b, e) = split(f);
        // Numbers already merge in the engine; leaving them alone keeps the
        // printed shape of `2 * sqrt(2)` intact.
        let numeric = matches!(b, MathStructure::Number(_));
        match bases.iter().position(|c| !numeric && c.equals(&b)) {
            Some(i) => {
                exps[i].push(e);
                merged = true;
            }
            None => {
                bases.push(b);
                exps.push(vec![e]);
            }
        }
    }
    if !merged {
        return None;
    }
    let mut out = Vec::with_capacity(bases.len());
    for (b, es) in bases.into_iter().zip(exps) {
        if es.len() == 1 {
            let e = es.into_iter().next().expect("len 1");
            out.push(if e.is_one() { b } else { pow(b, e) });
            continue;
        }
        let mut e = add(es);
        ev(&mut e);
        if e.is_zero() {
            out.push(num(1));
        } else if e.is_one() {
            out.push(b);
        } else {
            out.push(pow(b, e));
        }
    }
    Some(mul(out))
}

/// `m` is zero, looking through the products the merge engine leaves alone.
fn is_zero_expr(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => n.is_zero(),
        MathStructure::Multiplication(v) => v.iter().any(is_zero_expr),
        MathStructure::Addition(v) => v.iter().all(is_zero_expr),
        _ => false,
    }
}

fn evd(mut m: MathStructure) -> MathStructure {
    ev(&mut m);
    m
}

fn is_one(m: &MathStructure) -> bool {
    m.is_one()
}

// ----------------------------------------------------------------------
// Limit values
// ----------------------------------------------------------------------

/// The value of a limit: a finite expression, or a signed infinity.
#[derive(Clone, Debug)]
pub enum Lim {
    Val(MathStructure),
    Pos,
    Neg,
}

impl Lim {
    fn zero() -> Lim {
        Lim::Val(num(0))
    }
    fn is_zero(&self) -> bool {
        matches!(self, Lim::Val(v) if is_zero_expr(v))
    }
    fn is_inf(&self) -> bool {
        matches!(self, Lim::Pos | Lim::Neg)
    }
    fn inf_with(sign: i32) -> Lim {
        if sign >= 0 {
            Lim::Pos
        } else {
            Lim::Neg
        }
    }
    fn into_structure(self) -> MathStructure {
        match self {
            Lim::Val(v) => v,
            Lim::Pos => plus_inf(),
            Lim::Neg => minus_inf(),
        }
    }
}

/// Where the variable tends after normalization.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum At {
    /// `x -> 0`; the payload is the C++ approach direction
    /// (`0` both sides, `1` from above, `-1` from below).
    Zero(i32),
    /// `x -> +infinity`.
    Inf,
}

const MAX_DEPTH: usize = 20;
const MAX_SIZE: usize = 600;

// ----------------------------------------------------------------------
// Sign of a constant expression
// ----------------------------------------------------------------------

/// `+1`, `-1` or `None` when the sign of the (variable-free) `m` is unknown.
fn sign_of(m: &MathStructure) -> Option<i32> {
    match m {
        MathStructure::Number(n) => {
            if n.is_zero() {
                Some(0)
            } else if n.is_negative() {
                Some(-1)
            } else if n.is_positive() {
                Some(1)
            } else {
                None
            }
        }
        // `e` and `pi` are positive constants.
        MathStructure::Symbolic(s) if s == "e" || s == "pi" => Some(1),
        MathStructure::Multiplication(v) => {
            let mut s = 1;
            for f in v {
                s *= sign_of(f)?;
            }
            Some(s)
        }
        MathStructure::Addition(v) => {
            let mut s = 0;
            for t in v {
                let ts = sign_of(t)?;
                if ts == 0 {
                    continue;
                }
                if s == 0 {
                    s = ts;
                } else if s != ts {
                    return None;
                }
            }
            Some(s)
        }
        MathStructure::Power { base, exponent } => {
            let bs = sign_of(base)?;
            if bs >= 0 {
                return Some(if bs == 0 { 0 } else { 1 });
            }
            // A negative base needs an integer exponent to stay real.
            let MathStructure::Number(e) = exponent.as_ref() else {
                return None;
            };
            if !e.is_integer() {
                return None;
            }
            Some(if e.is_even() { 1 } else { -1 })
        }
        MathStructure::Function { id, args } => match (id.0, args.len()) {
            (bid::SQRT, 1) | (bid::EXP, 1) => Some(1),
            (bid::LN, 1) => {
                let a = sign_of_ln_argument(&args[0])?;
                Some(a)
            }
            (bid::CBRT, 1) => sign_of(&args[0]),
            _ => None,
        },
    _ => None,
    }
}

/// `ln(a)` is positive for `a > 1`, negative for `0 < a < 1`.
fn sign_of_ln_argument(a: &MathStructure) -> Option<i32> {
    let MathStructure::Number(n) = a else {
        return None;
    };
    if n.is_one() {
        Some(0)
    } else if n.is_greater_than(&Number::from_i64(1)) {
        Some(1)
    } else if n.is_positive() {
        Some(-1)
    } else {
        None
    }
}

// ----------------------------------------------------------------------
// together(): put an expression over a common denominator
// ----------------------------------------------------------------------

/// Split `m` into `(numerator, denominator)` structurally, without
/// evaluating. This is the port's stand-in for `limit_combine_divisions`
/// (MathStructure-limit.cc:139), which does the same job in place.
fn together(m: &MathStructure, depth: usize) -> (MathStructure, MathStructure) {
    if depth > 8 {
        return (m.clone(), num(1));
    }
    match m {
        MathStructure::Addition(terms) => {
            let parts: Vec<(MathStructure, MathStructure)> =
                terms.iter().map(|t| together(t, depth + 1)).collect();
            if parts.iter().all(|(_, d)| is_one(d)) {
                return (m.clone(), num(1));
            }
            // The common denominator is the *least* one: the multiset union
            // of each term's denominator factors. Multiplying every
            // denominator together would leave a spurious common factor that
            // L'Hopital would then have to differentiate away
            // (`tan(x)/x^3 - sin(x)/x^3` must not become `.../x^6`).
            let per_term: Vec<Vec<MathStructure>> =
                parts.iter().map(|(_, d)| denominator_factors(d)).collect();
            let mut lcm: Vec<MathStructure> = Vec::new();
            for factors in &per_term {
                let mut pool = lcm.clone();
                let mut extra: Vec<MathStructure> = Vec::new();
                for f in factors {
                    match pool.iter().position(|g| g.equals(f)) {
                        Some(i) => {
                            pool.remove(i);
                        }
                        None => extra.push(f.clone()),
                    }
                }
                lcm.extend(extra);
            }
            let den = mul(lcm.clone());
            let mut sum = Vec::with_capacity(parts.len());
            for ((n, _), factors) in parts.iter().zip(&per_term) {
                let mut pool = factors.clone();
                let mut mult = vec![n.clone()];
                for f in &lcm {
                    match pool.iter().position(|g| g.equals(f)) {
                        Some(i) => {
                            pool.remove(i);
                        }
                        None => mult.push(f.clone()),
                    }
                }
                sum.push(mul(mult));
            }
            (add(sum), den)
        }
        MathStructure::Multiplication(v) => {
            let parts: Vec<(MathStructure, MathStructure)> =
                v.iter().map(|t| together(t, depth + 1)).collect();
            if parts.iter().all(|(_, d)| is_one(d)) {
                return (m.clone(), num(1));
            }
            let n = mul(parts.iter().map(|(n, _)| n.clone()).collect());
            let d = mul(
                parts
                    .iter()
                    .filter(|(_, d)| !is_one(d))
                    .map(|(_, d)| d.clone())
                    .collect(),
            );
            (n, d)
        }
        MathStructure::Power { base, exponent } => {
            let MathStructure::Number(e) = exponent.as_ref() else {
                return (m.clone(), num(1));
            };
            if !e.is_integer() {
                return (m.clone(), num(1));
            }
            let (nb, db) = together(base, depth + 1);
            let neg = e.is_negative();
            let mut a = e.clone();
            if neg {
                a.negate();
            }
            let ae = nr(a);
            let (up, down) = if neg { (db, nb) } else { (nb, db) };
            let up = if is_one(&up) {
                num(1)
            } else {
                pow(up, ae.clone())
            };
            let down = if is_one(&down) { num(1) } else { pow(down, ae) };
            (up, down)
        }
        _ => (m.clone(), num(1)),
    }
}

/// Multiply out sums and non-negative integer powers of sums.
///
/// The merge engine keeps `(x + 2)^2` folded, but [`lead`] must refuse a sum
/// whose leading coefficients cancel (that cancellation is real for
/// `sqrt(x^2+x) - x`). Expanding first turns the polynomial cases into
/// genuine monomials, where the cancellation is exact and the next term
/// takes over.
fn expand_sum(m: &MathStructure) -> MathStructure {
    match expand_terms(m, 0) {
        Some(terms) => add(terms),
        None => m.clone(),
    }
}

const MAX_TERMS: usize = 400;

fn expand_terms(m: &MathStructure, depth: usize) -> Option<Vec<MathStructure>> {
    if depth > 8 {
        return None;
    }
    match m {
        MathStructure::Addition(v) => {
            let mut out = Vec::new();
            for t in v {
                out.extend(expand_terms(t, depth + 1)?);
                if out.len() > MAX_TERMS {
                    return None;
                }
            }
            Some(out)
        }
        MathStructure::Multiplication(v) => {
            let mut out = vec![num(1)];
            for f in v {
                let fs = expand_terms(f, depth + 1)?;
                if out.len() * fs.len() > MAX_TERMS {
                    return None;
                }
                let mut next = Vec::with_capacity(out.len() * fs.len());
                for a in &out {
                    for b in &fs {
                        next.push(mul(vec![a.clone(), b.clone()]));
                    }
                }
                out = next;
            }
            Some(out)
        }
        MathStructure::Power { base, exponent } => {
            let MathStructure::Number(k) = exponent.as_ref() else {
                return Some(vec![m.clone()]);
            };
            let Some(n) = k.to_i64() else {
                return Some(vec![m.clone()]);
            };
            if !k.is_integer() || !(0..=60).contains(&n) {
                return Some(vec![m.clone()]);
            }
            let base_terms = expand_terms(base, depth + 1)?;
            if base_terms.len() < 2 {
                return Some(vec![m.clone()]);
            }
            let mut out = vec![num(1)];
            for _ in 0..n {
                if out.len() * base_terms.len() > MAX_TERMS {
                    return None;
                }
                let mut next = Vec::with_capacity(out.len() * base_terms.len());
                for a in &out {
                    for b in &base_terms {
                        next.push(mul(vec![a.clone(), b.clone()]));
                    }
                }
                out = next;
            }
            Some(out)
        }
        _ => Some(vec![m.clone()]),
    }
}

/// A denominator broken into its multiset of factors, so a least common
/// denominator can be formed: `(x+2)^2` is `[x+2, x+2]`.
fn denominator_factors(d: &MathStructure) -> Vec<MathStructure> {
    if is_one(d) {
        return Vec::new();
    }
    match d {
        MathStructure::Multiplication(v) => v.iter().flat_map(denominator_factors).collect(),
        MathStructure::Power { base, exponent } => {
            let MathStructure::Number(k) = exponent.as_ref() else {
                return vec![d.clone()];
            };
            match k.to_i64() {
                Some(n) if k.is_integer() && (1..=20).contains(&n) => {
                    vec![(**base).clone(); n as usize]
                }
                _ => vec![d.clone()],
            }
        }
        _ => vec![d.clone()],
    }
}

// ----------------------------------------------------------------------
// lead(): the asymptotic leading term
// ----------------------------------------------------------------------

/// `m ~ coeff * t^exp`, where `t -> 0+` for [`At::Zero`] and `t -> +inf` for
/// [`At::Inf`]. `coeff` is variable-free and non-zero.
///
/// This is the port's equivalent of `limit_inf_cmp`: it compares growth by
/// an explicit rational exponent instead of the C++ ordinal ranking, which
/// is what lets radicals (`root(x^5, 4)`) participate. Any exact
/// cancellation of the leading coefficients makes it give up (`None`) rather
/// than guess a lower-order term.
fn lead(
    m: &MathStructure,
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<(MathStructure, Number)> {
    if depth > 12 {
        return None;
    }
    if !contains(m, x) {
        let v = evd(m.clone());
        if is_zero_expr(&v) {
            return None;
        }
        return Some((v, Number::new()));
    }
    if m.equals(x) {
        return Some((num(1), Number::from_i64(1)));
    }
    match m {
        MathStructure::Multiplication(v) => {
            let mut coeff: Vec<MathStructure> = Vec::new();
            let mut e = Number::new();
            for f in v {
                let (c, d) = lead(f, x, at, depth + 1)?;
                coeff.push(c);
                if !e.add(&d) {
                    return None;
                }
            }
            Some((evd(mul(coeff)), e))
        }
        MathStructure::Power { base, exponent } => {
            if contains(exponent, x) {
                return None;
            }
            let ex = evd((**exponent).clone());
            let MathStructure::Number(k) = &ex else {
                return None;
            };
            let (c, d) = lead(base, x, at, depth + 1)?;
            lead_pow(c, d, k)
        }
        MathStructure::Addition(terms) => {
            let mut parts: Vec<(MathStructure, Number)> = Vec::new();
            for t in terms {
                if !contains(t, x) && is_zero_expr(&evd(t.clone())) {
                    continue;
                }
                parts.push(lead(t, x, at, depth + 1)?);
            }
            if parts.is_empty() {
                return None;
            }
            let mut best = parts[0].1.clone();
            for (_, e) in &parts[1..] {
                let better = match at {
                    At::Inf => e.is_greater_than(&best),
                    At::Zero(_) => e.is_less_than(&best),
                };
                if better {
                    best = e.clone();
                }
            }
            let sum: Vec<MathStructure> = parts
                .iter()
                .filter(|(_, e)| e.equals(&best, false, false))
                .map(|(c, _)| c.clone())
                .collect();
            let c = evd(add(sum));
            if is_zero_expr(&c) {
                return None;
            }
            Some((c, best))
        }
        MathStructure::Function { id, args } => match (id.0, args.len()) {
            (bid::SQRT, 1) => {
                let (c, d) = lead(&args[0], x, at, depth + 1)?;
                lead_pow(c, d, &Number::from_ints(1, 2, 0))
            }
            (bid::CBRT, 1) => {
                let (c, d) = lead(&args[0], x, at, depth + 1)?;
                lead_pow(c, d, &Number::from_ints(1, 3, 0))
            }
            (bid::ROOT, 2) => {
                let MathStructure::Number(n) = &args[1] else {
                    return None;
                };
                let mut k = Number::from_i64(1);
                if !k.divide(n) {
                    return None;
                }
                let (c, d) = lead(&args[0], x, at, depth + 1)?;
                lead_pow(c, d, &k)
            }
            (bid::ABS, 1) => {
                let (c, d) = lead(&args[0], x, at, depth + 1)?;
                let s = sign_of(&c)?;
                let c = if s < 0 { evd(mul(vec![num(-1), c])) } else { c };
                // |x^d| only keeps the exponent for a one-sided approach.
                match at {
                    At::Inf | At::Zero(1) => Some((c, d)),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// `(c * t^d)^k = c^k * t^(d k)`, refusing a negative base under a
/// non-integer power (which would leave the reals).
fn lead_pow(c: MathStructure, d: Number, k: &Number) -> Option<(MathStructure, Number)> {
    if !k.is_integer() && sign_of(&c).is_some_and(|s| s < 0) {
        return None;
    }
    let mut e = d;
    if !e.multiply(k) {
        return None;
    }
    let coeff = evd(pow(c, nr(k.clone())));
    if is_zero_expr(&coeff) {
        return None;
    }
    Some((coeff, e))
}

/// Turn a leading term `c * t^e` into a limit value.
fn from_lead(c: MathStructure, e: Number, at: At) -> Option<Lim> {
    if e.is_zero() {
        return Some(Lim::Val(c));
    }
    let positive_exponent = e.is_positive();
    let vanishes = match at {
        At::Inf => !positive_exponent,
        At::Zero(_) => positive_exponent,
    };
    if vanishes {
        return Some(Lim::zero());
    }
    let s = sign_of(&c)?;
    if s == 0 {
        return Some(Lim::zero());
    }
    // Blowing up: at 0 the sign of `t^e` depends on the approach direction.
    let side = match at {
        At::Inf | At::Zero(1) => 1,
        At::Zero(-1) => {
            if !e.is_integer() {
                return None;
            }
            if e.is_even() {
                1
            } else {
                -1
            }
        }
        At::Zero(_) => {
            if !e.is_integer() || !e.is_even() {
                return None;
            }
            1
        }
    };
    Some(Lim::inf_with(s * side))
}

/// The sign `m` takes just next to the approach point.
fn sign_near(m: &MathStructure, x: &MathStructure, at: At) -> Option<i32> {
    let (c, e) = lead(m, x, at, 0)?;
    let s = sign_of(&c)?;
    if s == 0 {
        return None;
    }
    match at {
        At::Inf | At::Zero(1) => Some(s),
        At::Zero(-1) => {
            if !e.is_integer() {
                return None;
            }
            Some(if e.is_even() { s } else { -s })
        }
        At::Zero(_) => {
            if !e.is_integer() || !e.is_even() {
                return None;
            }
            Some(s)
        }
    }
}

// ----------------------------------------------------------------------
// The limit engine
// ----------------------------------------------------------------------

fn lim(m: &MathStructure, x: &MathStructure, at: At, depth: usize) -> Option<Lim> {
    if depth > MAX_DEPTH || m.count_total_children() > MAX_SIZE {
        return None;
    }
    if !contains(m, x) {
        return Some(Lim::Val(evd(m.clone())));
    }
    // 1. leading-term analysis on the expression as written.
    if let Some((c, e)) = lead(m, x, at, 0) {
        if let Some(r) = from_lead(c, e, at) {
            return Some(r);
        }
    }
    // 2. over a common denominator and multiplied out, then leading terms
    //    again (this resolves the cancellations that defeat step 1) and
    //    finally L'Hopital.
    let (n, d) = together(m, 0);
    if !is_one(&d) {
        let ne = evd(expand_sum(&n));
        let de = evd(expand_sum(&d));
        if let Some(r) = lead_quotient(&ne, &de, x, at) {
            return Some(r);
        }
        if let Some(r) = lim_quotient(&ne, &de, x, at, depth) {
            return Some(r);
        }
    } else {
        let ex = evd(expand_sum(m));
        if !ex.equals(m) {
            if let Some((c, e)) = lead(&ex, x, at, 0) {
                if let Some(r) = from_lead(c, e, at) {
                    return Some(r);
                }
            }
        }
    }
    // 3. structural rules.
    if let Some(r) = lim_struct(m, x, at, depth) {
        return Some(r);
    }
    // 4. conjugate rationalization of radical differences.
    let r = rationalize(m, x, 0);
    if !r.equals(m) {
        return lim(&evd(r), x, at, depth + 1);
    }
    None
}

fn lead_quotient(n: &MathStructure, d: &MathStructure, x: &MathStructure, at: At) -> Option<Lim> {
    let (cn, en) = lead(n, x, at, 0)?;
    let (cd, ed) = lead(d, x, at, 0)?;
    let mut e = en;
    let mut neg = ed;
    if !neg.negate() || !e.add(&neg) {
        return None;
    }
    let c = evd(mul(vec![cn, inv(cd)]));
    from_lead(c, e, at)
}

fn lim_quotient(
    n: &MathStructure,
    d: &MathStructure,
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    let ln = lim(n, x, at, depth + 1);
    let ld = lim(d, x, at, depth + 1);
    match (&ln, &ld) {
        (Some(Lim::Val(a)), Some(Lim::Val(b))) => {
            if !is_zero_expr(b) {
                return Some(Lim::Val(evd(mul(vec![a.clone(), inv(b.clone())]))));
            }
            if !is_zero_expr(a) {
                let sd = sign_near(d, x, at)?;
                let sa = sign_of(a)?;
                return Some(Lim::inf_with(sa * sd));
            }
            // 0/0 falls through to L'Hopital.
        }
        (Some(Lim::Val(_)), Some(inf)) if inf.is_inf() => return Some(Lim::zero()),
        (Some(a), Some(Lim::Val(b))) if a.is_inf() => {
            let sa = if matches!(a, Lim::Pos) { 1 } else { -1 };
            let sb = if is_zero_expr(b) {
                sign_near(d, x, at)?
            } else {
                sign_of(b)?
            };
            if sb == 0 {
                return None;
            }
            return Some(Lim::inf_with(sa * sb));
        }
        (Some(a), Some(b)) if a.is_inf() && b.is_inf() => {
            // inf/inf: try dividing through by the dominant exponential
            // before differentiating (L'Hopital never terminates on
            // `(e^x + e^-x) / (e^x - e^-x)`).
            if at == At::Inf {
                if let Some(r) = divide_by_dominant(n, d, x, at, depth) {
                    return Some(r);
                }
            }
        }
        _ => return None,
    }
    lhopital(n, d, x, at, depth)
}

/// `inf/inf` with exponentials: divide numerator and denominator by the
/// fastest-growing `b^f(x)` factor and retry.
fn divide_by_dominant(
    n: &MathStructure,
    d: &MathStructure,
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    let mut best: Option<MathStructure> = None;
    for side in [n, d] {
        collect_growing_exponentials(side, x, at, depth, &mut best);
    }
    let t = best?;
    let n2 = evd(mul(vec![n.clone(), inv(t.clone())]));
    let d2 = evd(mul(vec![d.clone(), inv(t)]));
    if n2.equals(n) || d2.equals(d) {
        return None;
    }
    let ln = lim(&n2, x, at, depth + 1)?;
    let ld = lim(&d2, x, at, depth + 1)?;
    match (ln, ld) {
        (Lim::Val(a), Lim::Val(b)) if !is_zero_expr(&b) => {
            Some(Lim::Val(evd(mul(vec![a, inv(b)]))))
        }
        _ => None,
    }
}

fn collect_growing_exponentials(
    m: &MathStructure,
    x: &MathStructure,
    at: At,
    depth: usize,
    best: &mut Option<MathStructure>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if let MathStructure::Power { base, exponent } = m {
        if !contains(base, x) && contains(exponent, x) {
            if matches!(lim(m, x, at, depth + 1), Some(Lim::Pos)) && best.is_none() {
                *best = Some(m.clone());
            }
            return;
        }
    }
    for i in 0..m.size() {
        if let Some(c) = m.get(i) {
            collect_growing_exponentials(c, x, at, depth, best);
        }
    }
}

fn lhopital(
    n: &MathStructure,
    d: &MathStructure,
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    if depth + 2 > MAX_DEPTH {
        return None;
    }
    let dn = evd(differentiate(n, x)?);
    let dd = evd(differentiate(d, x)?);
    if dd.is_zero() {
        return None;
    }
    if dn.count_total_children() + dd.count_total_children() > MAX_SIZE {
        return None;
    }
    let q = evd(mul(vec![dn, inv(dd)]));
    if q.count_total_children() > MAX_SIZE {
        return None;
    }
    lim(&q, x, at, depth + 1)
}

fn lim_struct(m: &MathStructure, x: &MathStructure, at: At, depth: usize) -> Option<Lim> {
    match m {
        _ if m.equals(x) => Some(match at {
            At::Inf => Lim::Pos,
            At::Zero(_) => Lim::zero(),
        }),
        MathStructure::Addition(terms) => lim_addition(terms, x, at, depth),
        MathStructure::Multiplication(v) => lim_multiplication(v, x, at, depth),
        MathStructure::Power { base, exponent } => lim_power(base, exponent, x, at, depth),
        MathStructure::Function { id, args } => lim_function(id.0, args, x, at, depth),
        _ => None,
    }
}

fn lim_addition(
    terms: &[MathStructure],
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    let mut vals: Vec<MathStructure> = Vec::new();
    let mut pos = 0;
    let mut neg = 0;
    for t in terms {
        match lim(t, x, at, depth + 1)? {
            Lim::Val(v) => vals.push(v),
            Lim::Pos => pos += 1,
            Lim::Neg => neg += 1,
        }
    }
    if pos > 0 && neg > 0 {
        let whole = MathStructure::Addition(terms.to_vec());
        // `inf - inf`: fold a logarithm difference into one log,
        // or pull out a factor shared by every term.
        let c = combine_logs(&whole);
        if !c.equals(&whole) {
            return lim(&evd(c), x, at, depth + 1);
        }
        if let Some(f) = factor_common(terms, x) {
            return lim(&f, x, at, depth + 1);
        }
        return None;
    }
    if pos > 0 {
        return Some(Lim::Pos);
    }
    if neg > 0 {
        return Some(Lim::Neg);
    }
    Some(Lim::Val(evd(add(vals))))
}

/// `2^(1/x) x - x` becomes `x (2^(1/x) - 1)`: pulling a factor shared by
/// every term out of an `inf - inf` sum turns it into a `0 * inf` product,
/// which the quotient machinery can finish.
fn factor_common(terms: &[MathStructure], x: &MathStructure) -> Option<MathStructure> {
    if terms.len() < 2 {
        return None;
    }
    let lists: Vec<Vec<MathStructure>> = terms
        .iter()
        .map(|t| match t {
            MathStructure::Multiplication(v) => v.clone(),
            other => vec![other.clone()],
        })
        .collect();
    let mut common: Vec<MathStructure> = Vec::new();
    let mut pool = lists[0].clone();
    while let Some(pos) = pool.iter().position(|f| {
        contains(f, x)
            && lists[1..]
                .iter()
                .all(|l| l.iter().any(|g| g.equals(f)))
    }) {
        common.push(pool.remove(pos));
        if common.len() > 8 {
            break;
        }
    }
    if common.is_empty() {
        return None;
    }
    let mut rest = Vec::with_capacity(lists.len());
    for l in &lists {
        let mut remaining = l.clone();
        for c in &common {
            if let Some(i) = remaining.iter().position(|g| g.equals(c)) {
                remaining.remove(i);
            }
        }
        rest.push(mul(remaining));
    }
    Some(mul(vec![mul(common), add(rest)]))
}

fn lim_multiplication(
    factors: &[MathStructure],
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    let mut lims: Vec<Option<Lim>> = Vec::with_capacity(factors.len());
    for f in factors {
        lims.push(lim(f, x, at, depth + 1));
    }
    // `bounded * 0 = 0`: sin/cos have no limit at infinity but are bounded.
    if lims.iter().any(Option::is_none) {
        let all_bounded_or_known = factors
            .iter()
            .zip(&lims)
            .all(|(f, l)| l.is_some() || is_bounded(f));
        let has_zero = lims
            .iter()
            .any(|l| matches!(l, Some(v) if v.is_zero()));
        let has_inf = lims.iter().any(|l| matches!(l, Some(v) if v.is_inf()));
        if all_bounded_or_known && has_zero && !has_inf {
            return Some(Lim::zero());
        }
        return None;
    }
    let lims: Vec<Lim> = lims.into_iter().map(|l| l.expect("checked")).collect();
    let n_inf = lims.iter().filter(|l| l.is_inf()).count();
    let n_zero = lims.iter().filter(|l| l.is_zero()).count();
    if n_inf == 0 {
        let vals: Vec<MathStructure> = lims
            .into_iter()
            .map(|l| match l {
                Lim::Val(v) => v,
                _ => unreachable!(),
            })
            .collect();
        return Some(Lim::Val(evd(mul(vals))));
    }
    if n_zero == 0 {
        let mut sign = 1;
        for l in &lims {
            match l {
                Lim::Pos => {}
                Lim::Neg => sign = -sign,
                Lim::Val(v) => sign *= sign_of(v)?,
            }
        }
        if sign == 0 {
            return None;
        }
        return Some(Lim::inf_with(sign));
    }
    // `0 * inf`: rewrite as a quotient and differentiate.
    let mut numer: Vec<MathStructure> = Vec::new();
    let mut denom: Vec<MathStructure> = Vec::new();
    for (f, l) in factors.iter().zip(&lims) {
        if l.is_inf() {
            denom.push(inv(f.clone()));
        } else {
            numer.push(f.clone());
        }
    }
    let n = evd(mul(numer));
    let d = evd(mul(denom));
    lim_quotient(&n, &d, x, at, depth)
}

/// Functions with a bounded range, which annihilate a factor going to zero.
fn is_bounded(m: &MathStructure) -> bool {
    match m {
        MathStructure::Function { id, args } => matches!(
            (id.0, args.len()),
            (bid::SIN, 1) | (bid::COS, 1) | (bid::ATAN, 1) | (bid::ACOT, 1) | (bid::SIGNUM, 1)
        ),
        MathStructure::Multiplication(v) => v.iter().all(|f| is_bounded(f) || !f.is_function()),
        _ => false,
    }
}

fn lim_power(
    base: &MathStructure,
    exponent: &MathStructure,
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    let b_has = contains(base, x);
    let e_has = contains(exponent, x);
    if b_has && !e_has {
        let lb = lim(base, x, at, depth + 1)?;
        let k = evd(exponent.clone());
        // A non-zero finite base needs no sign information about the
        // exponent: `(x+1)^(1/z)` at 0 is simply `1`.
        if let Lim::Val(v) = &lb {
            if v.is_one() {
                // `1^k = 1`; the merge engine leaves a symbolic exponent
                // (`1^(1/y)`) alone.
                return Some(Lim::Val(num(1)));
            }
            if !is_zero_expr(v) {
                return Some(Lim::Val(evd(pow(v.clone(), k))));
            }
        }
        let ks = sign_of(&k)?;
        return match lb {
            Lim::Val(v) if is_zero_expr(&v) => {
                if ks > 0 {
                    Some(Lim::zero())
                } else if ks == 0 {
                    Some(Lim::Val(num(1)))
                } else {
                    let s = sign_near(base, x, at)?;
                    let MathStructure::Number(kn) = &k else {
                        return None;
                    };
                    if s < 0 && !kn.is_integer() {
                        return None;
                    }
                    let flip = s < 0 && kn.is_integer() && !kn.is_even();
                    Some(Lim::inf_with(if flip { -1 } else { 1 }))
                }
            }
            // A non-zero finite base was already returned above.
            Lim::Val(v) => Some(Lim::Val(evd(pow(v, k)))),
            Lim::Pos => Some(if ks > 0 {
                Lim::Pos
            } else if ks == 0 {
                Lim::Val(num(1))
            } else {
                Lim::zero()
            }),
            Lim::Neg => {
                let MathStructure::Number(kn) = &k else {
                    return None;
                };
                if !kn.is_integer() {
                    return None;
                }
                if ks == 0 {
                    return Some(Lim::Val(num(1)));
                }
                let s = if kn.is_even() { 1 } else { -1 };
                Some(if ks > 0 {
                    Lim::inf_with(s)
                } else {
                    Lim::zero()
                })
            }
        };
    }
    if !b_has && e_has {
        let le = lim(exponent, x, at, depth + 1)?;
        let b = evd(base.clone());
        return match le {
            Lim::Val(v) => Some(Lim::Val(evd(pow(b, v)))),
            Lim::Pos | Lim::Neg => {
                let one = Number::from_i64(1);
                let MathStructure::Number(bn) = &b else {
                    // `e^x` and friends: the base is a positive constant > 1.
                    let big = matches!(&b, MathStructure::Symbolic(s) if s == "e");
                    if !big {
                        return None;
                    }
                    return Some(match le {
                        Lim::Pos => Lim::Pos,
                        _ => Lim::zero(),
                    });
                };
                if !bn.is_positive() {
                    return None;
                }
                let grows = bn.is_greater_than(&one);
                if bn.equals(&one, false, false) {
                    return Some(Lim::Val(num(1)));
                }
                Some(match (le, grows) {
                    (Lim::Pos, true) | (Lim::Neg, false) => Lim::Pos,
                    _ => Lim::zero(),
                })
            }
        };
    }
    // Both sides depend on x: determinate cases first, then `exp(g ln f)`.
    // A missing sub-limit is not fatal here — `(1 + 2x)^(1/x)` has no
    // two-sided limit in the exponent yet a perfectly good limit overall.
    let lb = lim(base, x, at, depth + 1);
    let le = lim(exponent, x, at, depth + 1);
    if let (Some(lb), Some(le)) = (&lb, &le) {
    match (lb, le) {
        (Lim::Val(v), Lim::Val(w)) => {
            if !is_zero_expr(v) && !v.is_one() && sign_of(v).is_some_and(|s| s > 0) {
                return Some(Lim::Val(evd(pow(v.clone(), w.clone()))));
            }
            if is_zero_expr(v) && sign_of(w).is_some_and(|s| s > 0) {
                return Some(Lim::zero());
            }
        }
        (Lim::Val(v), inf) if inf.is_inf() => {
            if let Some(s) = sign_of(v) {
                if s > 0 && !v.is_one() {
                    let one = Number::from_i64(1);
                    if let MathStructure::Number(vn) = v {
                        let grows = vn.is_greater_than(&one);
                        return Some(match (inf, grows) {
                            (Lim::Pos, true) | (Lim::Neg, false) => Lim::Pos,
                            _ => Lim::zero(),
                        });
                    }
                }
            }
        }
        (Lim::Pos, Lim::Val(w)) => {
            let s = sign_of(w)?;
            return Some(if s > 0 {
                Lim::Pos
            } else if s == 0 {
                Lim::Val(num(1))
            } else {
                Lim::zero()
            });
        }
        (Lim::Pos, Lim::Pos) => return Some(Lim::Pos),
        (Lim::Pos, Lim::Neg) => return Some(Lim::zero()),
        _ => {}
    }
    }
    // Indeterminate `1^inf`, `0^0`, `inf^0`: exp(exponent * ln(base)).
    let inner = mul(vec![
        exponent.clone(),
        func(bid::LN, vec![base.clone()]),
    ]);
    let l = lim(&inner, x, at, depth + 1)?;
    Some(exp_lim(l))
}

fn exp_lim(l: Lim) -> Lim {
    match l {
        Lim::Pos => Lim::Pos,
        Lim::Neg => Lim::zero(),
        Lim::Val(v) => Lim::Val(e_pow(v)),
    }
}

/// `e^v`, in the shape the reference prints: `e^3`, `1 / e^5`,
/// `e^2 * cbrt(e)`, `e^7 * sqrt(e)`, `1 / (e * sqrt(e))`.
fn e_pow(v: MathStructure) -> MathStructure {
    let MathStructure::Number(n) = &v else {
        return evd(pow(e_sym(), v));
    };
    if n.is_zero() {
        return num(1);
    }
    if !n.is_rational() || n.is_approximate() {
        return evd(pow(e_sym(), v));
    }
    let neg = n.is_negative();
    let mut a = n.clone();
    if neg && !a.negate() {
        return evd(pow(e_sym(), v));
    }
    let den = a.denominator();
    let body = if a.is_integer() {
        e_power_int(&a)
    } else if den.equals(&Number::from_i64(2), false, false)
        || den.equals(&Number::from_i64(3), false, false)
    {
        // Split into the integer part and a square/cube root, the way the
        // reference renders `e^(7/3)` as `e^2 * cbrt(e)`.
        let mut whole = a.clone();
        whole.floor();
        let mut frac = a.clone();
        if !frac.subtract(&whole) {
            return evd(pow(e_sym(), v));
        }
        let root = pow(e_sym(), nr(frac.clone()));
        if whole.is_zero() {
            root
        } else {
            mul(vec![e_power_int(&whole), root])
        }
    } else {
        pow(e_sym(), nr(a.clone()))
    };
    if neg {
        inv(body)
    } else {
        body
    }
}

fn e_power_int(k: &Number) -> MathStructure {
    if k.is_one() {
        e_sym()
    } else {
        pow(e_sym(), nr(k.clone()))
    }
}

fn lim_function(
    id: u32,
    args: &[MathStructure],
    x: &MathStructure,
    at: At,
    depth: usize,
) -> Option<Lim> {
    if args.is_empty() {
        return None;
    }
    let la = lim(&args[0], x, at, depth + 1)?;
    match id {
        bid::LN | bid::LOG if args.len() == 1 => match la {
            Lim::Pos => Some(Lim::Pos),
            Lim::Val(v) if v.is_zero() => Some(Lim::Neg),
            Lim::Val(v) => {
                let s = sign_of(&v)?;
                if s <= 0 {
                    return None;
                }
                Some(Lim::Val(evd(func(bid::LN, vec![v]))))
            }
            Lim::Neg => None,
        },
        bid::EXP => Some(exp_lim(la)),
        bid::SQRT => match la {
            Lim::Pos => Some(Lim::Pos),
            Lim::Val(v) => {
                if sign_of(&v).is_some_and(|s| s < 0) {
                    return None;
                }
                Some(Lim::Val(evd(func(bid::SQRT, vec![v]))))
            }
            Lim::Neg => None,
        },
        bid::CBRT => match la {
            Lim::Pos => Some(Lim::Pos),
            Lim::Neg => Some(Lim::Neg),
            Lim::Val(v) => Some(Lim::Val(evd(func(bid::CBRT, vec![v])))),
        },
        bid::ROOT if args.len() == 2 => match la {
            Lim::Pos => Some(Lim::Pos),
            Lim::Val(v) => Some(Lim::Val(evd(func(bid::ROOT, vec![v, args[1].clone()])))),
            Lim::Neg => None,
        },
        bid::ABS => match la {
            Lim::Pos | Lim::Neg => Some(Lim::Pos),
            Lim::Val(v) => Some(Lim::Val(evd(func(bid::ABS, vec![v])))),
        },
        bid::SIN | bid::COS | bid::TAN | bid::COT => match la {
            Lim::Val(v) => trig_value(id, &v),
            _ => None,
        },
        bid::ATAN => match la {
            Lim::Pos => Some(Lim::Val(mul(vec![ratio(1, 2), pi_sym()]))),
            Lim::Neg => Some(Lim::Val(mul(vec![ratio(-1, 2), pi_sym()]))),
            Lim::Val(v) => inverse_trig_value(bid::ATAN, &v),
        },
        bid::ACOT => match la {
            Lim::Pos | Lim::Neg => Some(Lim::zero()),
            Lim::Val(v) => {
                if v.is_zero() {
                    Some(Lim::Val(mul(vec![ratio(1, 2), pi_sym()])))
                } else {
                    inverse_trig_value(bid::ACOT, &v)
                }
            }
        },
        bid::ASIN | bid::ACOS => match la {
            Lim::Val(v) => inverse_trig_value(id, &v),
            _ => None,
        },
        bid::SINH => match la {
            Lim::Pos => Some(Lim::Pos),
            Lim::Neg => Some(Lim::Neg),
            Lim::Val(v) => Some(Lim::Val(evd(func(bid::SINH, vec![v])))),
        },
        bid::COSH => match la {
            Lim::Pos | Lim::Neg => Some(Lim::Pos),
            Lim::Val(v) => Some(Lim::Val(evd(func(bid::COSH, vec![v])))),
        },
        bid::TANH => match la {
            Lim::Pos => Some(Lim::Val(num(1))),
            Lim::Neg => Some(Lim::Val(num(-1))),
            Lim::Val(v) => Some(Lim::Val(evd(func(bid::TANH, vec![v])))),
        },
        _ => match la {
            Lim::Val(v) => {
                let mut a = args.to_vec();
                a[0] = v;
                let r = evd(func(id, a));
                if contains(&r, x) {
                    None
                } else {
                    Some(Lim::Val(r))
                }
            }
            _ => None,
        },
    }
}

// ----------------------------------------------------------------------
// Exact trigonometric values at rational multiples of pi
// ----------------------------------------------------------------------

/// `v = k * pi` → `Some(k)`.
fn pi_multiple(v: &MathStructure) -> Option<Number> {
    match v {
        MathStructure::Number(n) if n.is_zero() => Some(Number::new()),
        MathStructure::Symbolic(s) if s == "pi" => Some(Number::from_i64(1)),
        MathStructure::Multiplication(f) if f.len() == 2 => {
            let (MathStructure::Number(k), MathStructure::Symbolic(s)) = (&f[0], &f[1]) else {
                return None;
            };
            (s == "pi").then(|| k.clone())
        }
        _ => None,
    }
}

/// `sin(k * pi)` for `k` with denominator 1, 2, 3, 4 or 6.
fn sin_pi(k: &Number) -> Option<MathStructure> {
    let two = Number::from_i64(2);
    let mut r = k.clone();
    if !r.mod_floor(&two) {
        return None;
    }
    let half = Number::from_ints(1, 2, 0);
    let one = Number::from_i64(1);
    if !r.is_less_than(&one) {
        // sin((r) pi) = -sin((r-1) pi)
        let mut s = r.clone();
        if !s.subtract(&one) {
            return None;
        }
        let inner = sin_pi(&s)?;
        return Some(evd(mul(vec![num(-1), inner])));
    }
    if r.is_greater_than(&half) {
        let mut s = one.clone();
        if !s.subtract(&r) {
            return None;
        }
        return sin_pi(&s);
    }
    let table: [(i64, i64, MathStructure); 5] = [
        (0, 1, num(0)),
        (1, 6, ratio(1, 2)),
        // 1/sqrt(2), the form the reference keeps (`-1 / sqrt(2)`).
        (1, 4, pow(num(2), ratio(-1, 2))),
        (1, 3, mul(vec![ratio(1, 2), pow(num(3), ratio(1, 2))])),
        (1, 2, num(1)),
    ];
    for (a, b, v) in table {
        if r.equals(&Number::from_ints(a, b, 0), false, false) {
            return Some(evd(v));
        }
    }
    None
}

fn cos_pi(k: &Number) -> Option<MathStructure> {
    let mut s = k.clone();
    if !s.add(&Number::from_ints(1, 2, 0)) {
        return None;
    }
    sin_pi(&s)
}

/// The exact value of `sin`/`cos`/`tan`/`cot` at a limit point, falling back
/// to the ordinary (exact-mode) evaluation of the call.
fn trig_value(id: u32, v: &MathStructure) -> Option<Lim> {
    if let Some(k) = pi_multiple(v) {
        let value = match id {
            bid::SIN => sin_pi(&k),
            bid::COS => cos_pi(&k),
            bid::TAN => {
                let s = sin_pi(&k)?;
                let c = cos_pi(&k)?;
                if c.is_zero() {
                    return None;
                }
                Some(evd(mul(vec![s, inv(c)])))
            }
            bid::COT => {
                let s = sin_pi(&k)?;
                let c = cos_pi(&k)?;
                if s.is_zero() {
                    return None;
                }
                Some(evd(mul(vec![c, inv(s)])))
            }
            _ => None,
        };
        if let Some(value) = value {
            return Some(Lim::Val(value));
        }
    }
    let r = evd(func(id, vec![v.clone()]));
    Some(Lim::Val(r))
}

/// Exact inverse-trigonometric values at the rational arguments the
/// transcripts need (`acos(1/2) = pi/3`).
fn inverse_trig_value(id: u32, v: &MathStructure) -> Option<Lim> {
    let MathStructure::Number(n) = v else {
        return Some(Lim::Val(evd(func(id, vec![v.clone()]))));
    };
    if !n.is_rational() || n.is_approximate() {
        return Some(Lim::Val(evd(func(id, vec![v.clone()]))));
    }
    let table: [(i64, i64, i64, i64, i64, i64); 5] = [
        // (value num, value den, asin*pi num, den, acos*pi num, den)
        (-1, 1, -1, 2, 1, 1),
        (-1, 2, -1, 6, 2, 3),
        (0, 1, 0, 1, 1, 2),
        (1, 2, 1, 6, 1, 3),
        (1, 1, 1, 2, 0, 1),
    ];
    for (a, b, sn, sd, cn, cd) in table {
        if !n.equals(&Number::from_ints(a, b, 0), false, false) {
            continue;
        }
        let (p, q) = match id {
            bid::ASIN => (sn, sd),
            bid::ACOS => (cn, cd),
            _ => break,
        };
        if p == 0 {
            return Some(Lim::zero());
        }
        return Some(Lim::Val(evd(mul(vec![ratio(p, q), pi_sym()]))));
    }
    if id == bid::ATAN {
        if n.is_zero() {
            return Some(Lim::zero());
        }
        if n.is_one() {
            return Some(Lim::Val(evd(mul(vec![ratio(1, 4), pi_sym()]))));
        }
        if n.is_minus_one() {
            return Some(Lim::Val(evd(mul(vec![ratio(-1, 4), pi_sym()]))));
        }
    }
    Some(Lim::Val(evd(func(id, vec![v.clone()]))))
}

// ----------------------------------------------------------------------
// Conjugate rationalization
// ----------------------------------------------------------------------

/// True when `m` contains a square/cube root or a fractional power.
fn has_radical(m: &MathStructure) -> bool {
    match m {
        MathStructure::Function { id, .. } => {
            matches!(id.0, bid::SQRT | bid::CBRT | bid::ROOT)
        }
        MathStructure::Power { exponent, .. } => {
            matches!(exponent.as_ref(), MathStructure::Number(n) if !n.is_integer())
        }
        _ => (0..m.size()).any(|i| m.get(i).is_some_and(has_radical)),
    }
}

/// The square of `m`, resolving `sqrt(u)^2` to `u`.
fn square(m: &MathStructure) -> MathStructure {
    match m {
        MathStructure::Function { id, args } if id.0 == bid::SQRT && args.len() == 1 => {
            args[0].clone()
        }
        MathStructure::Power { base, exponent } => {
            let mut e = (**exponent).clone();
            if let MathStructure::Number(n) = &mut e {
                let mut d = n.clone();
                if d.multiply(&Number::from_i64(2)) {
                    return pow((**base).clone(), nr(d));
                }
            }
            pow(m.clone(), num(2))
        }
        MathStructure::Multiplication(v) => {
            MathStructure::Multiplication(v.iter().map(square).collect())
        }
        _ => pow(m.clone(), num(2)),
    }
}

/// Replace every two-term sum containing a radical by `(a^2 - b^2)/(a - b)`.
///
/// This is what turns `sqrt(x^2 + x) - x` (an `inf - inf` form) into
/// `x / (sqrt(x^2 + x) + x)`, which the leading-term analysis then settles.
fn rationalize(m: &MathStructure, x: &MathStructure, depth: usize) -> MathStructure {
    if depth > 6 {
        return m.clone();
    }
    let mut out = m.clone();
    for i in 0..out.size() {
        if let Some(c) = out.get_mut(i) {
            let r = rationalize(&c.clone(), x, depth + 1);
            *c = r;
        }
    }
    let MathStructure::Addition(terms) = &out else {
        return out;
    };
    if terms.len() != 2 || !has_radical(&out) || !contains(&out, x) {
        return out;
    }
    let a = terms[0].clone();
    let b = terms[1].clone();
    let numer = evd(add(vec![square(&a), mul(vec![num(-1), square(&b)])]));
    let denom = evd(add(vec![a, mul(vec![num(-1), b])]));
    if denom.is_zero() || numer.equals(&out) {
        return out;
    }
    mul(vec![numer, inv(denom)])
}

// ----------------------------------------------------------------------
// Preprocessing: cot -> cos/sin
// ----------------------------------------------------------------------

/// Two rewrites applied before the limit is taken:
///
/// * `cot(u)` is `cos(u)/sin(u)` in `data/functions.xml.in`; expanding it
///   lets the quotient machinery handle `x cot(2x)` instead of an opaque
///   call.
/// * `ln(a^b)` becomes `b ln(a)`, which turns `ln(((3x+1)/(3x-5))^-x)` into
///   a `0 * inf` product L'Hopital can finish.
fn preprocess(m: &mut MathStructure, depth: usize) {
    if depth > 12 {
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get_mut(i) {
            preprocess(c, depth + 1);
        }
    }
    let MathStructure::Function { id, args } = m else {
        return;
    };
    // Radicals become powers so that `sqrt(3)` built by the engine and
    // `3^(1/2)` built by the leading-term analysis are the *same* structure
    // and cancel against each other (`sqrt(x+3) - sqrt(3)` at 0 must be 0).
    match (id.0, args.len()) {
        (bid::SQRT, 1) => {
            *m = pow(args[0].clone(), ratio(1, 2));
            return;
        }
        (bid::CBRT, 1) => {
            *m = pow(args[0].clone(), ratio(1, 3));
            return;
        }
        (bid::ROOT, 2) => {
            if let MathStructure::Number(n) = &args[1] {
                let mut k = Number::from_i64(1);
                if k.divide(n) {
                    *m = pow(args[0].clone(), nr(k));
                    return;
                }
            }
        }
        _ => {}
    }
    if id.0 == bid::COT && args.len() == 1 {
        let u = args[0].clone();
        *m = mul(vec![
            func(bid::COS, vec![u.clone()]),
            inv(func(bid::SIN, vec![u])),
        ]);
        return;
    }
    if (id.0 == bid::LN || id.0 == bid::EXP) && args.len() == 1 {
        if id.0 != bid::LN {
            return;
        }
        if let MathStructure::Power { base, exponent } = &args[0] {
            let (b, e) = ((**base).clone(), (**exponent).clone());
            *m = mul(vec![e, func(bid::LN, vec![b])]);
        }
    }
}

/// `ln(a) - ln(b)` (with a common cofactor) becomes `ln(a/b)`, the standard
/// way out of the `inf - inf` form in `x (ln(x+3) - ln(x))`.
fn combine_logs(m: &MathStructure) -> MathStructure {
    let MathStructure::Addition(terms) = m else {
        return m.clone();
    };
    // Decompose each term into (cofactor, ln argument).
    let split = |t: &MathStructure| -> Option<(MathStructure, MathStructure)> {
        match t {
            MathStructure::Function { id, args } if id.0 == bid::LN && args.len() == 1 => {
                Some((num(1), args[0].clone()))
            }
            MathStructure::Multiplication(v) => {
                let mut arg = None;
                let mut rest = Vec::new();
                for f in v {
                    match f {
                        MathStructure::Function { id, args }
                            if id.0 == bid::LN && args.len() == 1 && arg.is_none() =>
                        {
                            arg = Some(args[0].clone());
                        }
                        other => rest.push(other.clone()),
                    }
                }
                arg.map(|a| (mul(rest), a))
            }
            _ => None,
        }
    };
    let parts: Vec<Option<(MathStructure, MathStructure)>> = terms.iter().map(split).collect();
    let mut used = vec![false; terms.len()];
    let mut out: Vec<MathStructure> = Vec::new();
    let mut changed = false;
    for i in 0..terms.len() {
        if used[i] {
            continue;
        }
        let Some((ci, ai)) = &parts[i] else {
            used[i] = true;
            out.push(terms[i].clone());
            continue;
        };
        let mut merged = false;
        for j in (i + 1)..terms.len() {
            if used[j] {
                continue;
            }
            let Some((cj, aj)) = &parts[j] else { continue };
            if !is_zero_expr(&evd(add(vec![ci.clone(), cj.clone()]))) {
                continue;
            }
            used[i] = true;
            used[j] = true;
            out.push(mul(vec![
                ci.clone(),
                func(bid::LN, vec![mul(vec![ai.clone(), inv(aj.clone())])]),
            ]));
            merged = true;
            changed = true;
            break;
        }
        if !merged {
            used[i] = true;
            out.push(terms[i].clone());
        }
    }
    if changed {
        add(out)
    } else {
        m.clone()
    }
}

// ----------------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------------

/// Is `m` the (unresolved) `infinity` constant, optionally negated?
fn infinity_sign(m: &MathStructure) -> Option<i32> {
    match m {
        MathStructure::Number(n) if n.is_plus_infinity() => Some(1),
        MathStructure::Number(n) if n.is_minus_infinity() => Some(-1),
        MathStructure::Symbolic(s) if s == "infinity" || s == "inf" => Some(1),
        MathStructure::Multiplication(v) if v.len() == 2 => {
            let MathStructure::Number(k) = &v[0] else {
                return None;
            };
            let s = infinity_sign(&v[1])?;
            if k.is_negative() {
                Some(-s)
            } else if k.is_positive() {
                Some(s)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// `limit(expr, value, x, approach)` — the public entry point.
pub fn calculate_limit(
    expr: &MathStructure,
    xvar: &MathStructure,
    value: &MathStructure,
    approach: i32,
) -> Option<MathStructure> {
    let mut e = expr.clone();
    preprocess(&mut e, 0);
    let (at, e) = match infinity_sign(value) {
        Some(1) => (At::Inf, e),
        Some(-1) => {
            let mut m = e;
            crate::solve::replace(&mut m, xvar, &mul(vec![num(-1), xvar.clone()]));
            (At::Inf, m)
        }
        Some(_) => return None,
        None => {
            let v = evd(value.clone());
            if !v.is_zero() {
                let mut m = e;
                crate::solve::replace(&mut m, xvar, &add(vec![xvar.clone(), v]));
                (At::Zero(approach), m)
            } else {
                (At::Zero(approach), e)
            }
        }
    };
    let e = evd(e);
    lim(&e, xvar, at, 0)
        .map(Lim::into_structure)
        .map(|mut r| {
            normalize_radical(&mut r, 0);
            r
        })
}

/// `sqrt(3) / 3` is `3^(-1/2)` — the shape the reference prints as
/// `1 / sqrt(3)`. Only the exact `1/p` coefficient folds in; `4 sqrt(3)` and
/// `sqrt(2) / 4` stay as products, matching the reference.
fn normalize_radical(m: &mut MathStructure, depth: usize) {
    if depth > 12 {
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get_mut(i) {
            normalize_radical(c, depth + 1);
        }
    }
    let MathStructure::Multiplication(v) = m else {
        return;
    };
    if v.len() != 2 {
        return;
    }
    let (MathStructure::Number(c), MathStructure::Power { base, exponent }) = (&v[0], &v[1]) else {
        return;
    };
    let (MathStructure::Number(p), MathStructure::Number(e)) = (base.as_ref(), exponent.as_ref())
    else {
        return;
    };
    if !c.is_rational() || c.is_approximate() || e.is_integer() || !p.is_positive() {
        return;
    }
    let mut inv_p = Number::from_i64(1);
    if !inv_p.divide(p) {
        return;
    }
    let mut abs_c = c.clone();
    let negative = abs_c.is_negative();
    if negative && !abs_c.negate() {
        return;
    }
    if !abs_c.equals(&inv_p, false, false) {
        return;
    }
    let mut new_e = e.clone();
    if !new_e.subtract(&Number::from_i64(1)) {
        return;
    }
    let folded = pow(nr(p.clone()), nr(new_e));
    *m = if negative {
        mul(vec![num(-1), folded])
    } else {
        folded
    };
}

// ----------------------------------------------------------------------
// Builtin dispatch
// ----------------------------------------------------------------------

pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    if id.0 != id::LIMIT || args.len() < 2 || args.len() > 4 {
        return false;
    }
    // A nested limit() would re-enter through the evaluation helper.
    if BUSY.with(|b| b.get()) {
        return false;
    }
    let args = args.clone();
    // `LimitFunction` gives argument 3 the default value "x"
    // (BuiltinFunctions-calculus.cc:750), so a plain `x` wins over any other
    // symbol in the expression.
    let xvar = match args.get(2) {
        Some(v) if v.is_symbolic() => v.clone(),
        _ => {
            let plain_x = MathStructure::symbolic("x");
            if contains(&args[0], &plain_x) {
                plain_x
            } else {
                crate::polynomial::find_x_var(&args[0]).unwrap_or(plain_x)
            }
        }
    };
    let approach = match args.get(3) {
        Some(MathStructure::Number(n)) => n.to_i64().unwrap_or(0).clamp(-1, 1) as i32,
        _ => 0,
    };
    BUSY.with(|b| b.set(true));
    let r = calculate_limit(&args[0], &xvar, &args[1], approach);
    BUSY.with(|b| b.set(false));
    match r {
        Some(v) => {
            *m = v;
            true
        }
        None => false,
    }
}

thread_local! {
    /// True while a limit is being computed, so the evaluation helper does
    /// not recurse into the same call.
    static BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    match name {
        "limit" | "lim" => Some(FunctionId(id::LIMIT)),
        _ => None,
    }
}

pub fn function_name(id: u32) -> Option<&'static str> {
    match id {
        self::id::LIMIT => Some("limit"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    /// Evaluate through a session configured like `limits.batch`
    /// (`/set approximation exact`, `/set fr 2`).
    fn lm(expr: &str) -> String {
        let mut s = Session::new();
        s.evaluate_line("/set approximation exact").ok();
        s.evaluate_line("/set fr 2").ok();
        s.evaluate_line(expr).expect("evaluates")
    }

    #[test]
    fn removable_singularity() {
        assert_eq!(lm("limit((x^2-4)/(x-2),2)"), "4");
        assert_eq!(lm("limit((x-2)/(x^2-3x+2),2)"), "1");
    }

    #[test]
    fn direct_substitution() {
        assert_eq!(lm("limit(x^2-4,2)"), "0");
        assert_eq!(lm("limit((x+1)/(x-1),2)"), "3");
    }

    #[test]
    fn poles_are_signed_infinities() {
        assert_eq!(lm("limit(x^3/(x+1)^2,-1)"), "-infinity");
        assert_eq!(lm("limit((x^2+2x+3)/(x-1)^2,1)"), "+infinity");
    }

    #[test]
    fn rational_limits_at_infinity() {
        assert_eq!(lm("limit((x^2-1)/(2x^2+1),infinity)"), "1/2");
        assert_eq!(lm("limit((3x^2+2x-1)/(x^3-x+2),infinity)"), "0");
        assert_eq!(lm("limit((x^3+x^2-4)/(2x^3+x+11),-infinity)"), "1/2");
    }

    #[test]
    fn degree_comparison_beats_lhopital() {
        // 300 differentiations would never finish; the leading term does.
        assert_eq!(
            lm("limit(((x-1)^100*(6x+1)^200)/(3x+5)^300,infinity)"),
            "3117982410208"
        );
    }

    #[test]
    fn radical_conjugates() {
        assert_eq!(lm("limit(sqrt(x^2+x)-x,infinity)"), "1/2");
        assert_eq!(lm("limit(x(sqrt(x^2+1)-x),infinity)"), "1/2");
        assert_eq!(lm("limit(sqrt(x^2+1)-x,-infinity)"), "+infinity");
    }

    #[test]
    fn radicals_at_infinity_by_degree() {
        assert_eq!(lm("limit((sqrt(x^2+9))/(x+3),infinity)"), "1");
        assert_eq!(
            lm("limit((root(x^5,4)+root(x^3,5)+root(x^8,6))/(cbrt(x^4+2)),infinity)"),
            "1"
        );
    }

    #[test]
    fn exponential_indeterminate_forms() {
        assert_eq!(lm("limit((1+1/x)^(3x),infinity)"), "e^3");
        assert_eq!(lm("limit((1-5/x)^x,infinity)"), "1 / e^5");
        assert_eq!(lm("limit((1+7/(3x))^(x),infinity)"), "e^2 * cbrt(e)");
        assert_eq!(lm("limit(((2x+5)/(2x))^(3x),infinity)"), "e^7 * sqrt(e)");
    }

    #[test]
    fn trigonometric_limits_at_zero() {
        assert_eq!(lm("limit(sin(10x)/(10x),0)"), "1");
        assert_eq!(lm("limit(tan(8x)/(x),0)"), "8");
        assert_eq!(lm("limit((tan(x)-sin(x))/(x^3),0)"), "1/2");
        assert_eq!(lm("limit(x*cot(2x),0)"), "1/2");
    }

    #[test]
    fn logarithmic_and_exponential_quotients() {
        assert_eq!(lm("limit((3^x-1)/(6^x-1),0)"), "ln(3) / ln(6)");
        assert_eq!(lm("limit(x(2^(1/x)-1),infinity)"), "ln(2)");
        assert_eq!(lm("limit((5^x-1)/(x),0)"), "ln(5)");
    }

    #[test]
    fn exact_values_of_pi() {
        assert_eq!(lm("limit(x*sin(pi/x),infinity)"), "pi");
        assert_eq!(lm("limit(acos(sqrt(x^2+x)-x),infinity)"), "pi / 3");
    }

    #[test]
    fn dominant_exponential_division() {
        assert_eq!(lm("limit((e^x+e^(-x))/(e^x-e^(-x)),-infinity)"), "-1");
    }

    #[test]
    fn together_finds_the_common_denominator() {
        assert_eq!(lm("limit(1/(1-x)-3/(1-x^3),1)"), "-1");
        assert_eq!(lm("limit(x^2-(x^4-1)/(x^2-2),infinity)"), "-2");
    }

    #[test]
    fn e_pow_renders_like_the_reference() {
        assert_eq!(super::e_pow(num(0)), num(1));
        // e^(7/3) -> e^2 * cbrt(e)
        let m = super::e_pow(ratio(7, 3));
        assert!(m.is_multiplication());
        assert_eq!(m.size(), 2);
    }

    #[test]
    fn sin_pi_table() {
        assert!(sin_pi(&Number::new()).expect("sin(0)").is_zero());
        assert!(sin_pi(&Number::from_ints(1, 2, 0)).expect("sin(pi/2)").is_one());
        assert!(cos_pi(&Number::from_ints(1, 2, 0)).expect("cos(pi/2)").is_zero());
        assert!(cos_pi(&Number::new()).expect("cos(0)").is_one());
    }

    #[test]
    fn bounded_times_zero() {
        assert_eq!(lm("limit(2^x*sin(2pi*x),-infinity)"), "0");
    }

    #[test]
    fn one_sided_limits_follow_the_approach_argument() {
        // `limit(expr, value, x, approach)` — argument 4 is -1/0/1.
        assert_eq!(lm("limit(1/x,0,x,1)"), "+infinity");
        assert_eq!(lm("limit(1/x,0,x,-1)"), "-infinity");
        assert_eq!(lm("limit(ln(x),0,x,1)"), "-infinity");
        // A one-sided argument does not disturb an ordinary limit.
        assert_eq!(lm("limit((x^2-4)/(x-2),2,x,1)"), "4");
    }

    #[test]
    fn a_sign_flipping_pole_has_no_two_sided_limit() {
        // The reference leaves the call unevaluated rather than guessing.
        assert_eq!(lm("limit(1/x,0)"), "limit(1 / x, 0)");
    }

    #[test]
    fn nested_indeterminate_powers() {
        assert_eq!(lm("limit((1-sin(x)/x)^(1/ln(x)),0)"), "e^2");
        assert_eq!(lm("limit((e^x+x)^(1/x),0)"), "e^2");
    }

    #[test]
    fn logarithm_differences_combine() {
        assert_eq!(lm("limit(x*(ln(x+3)-ln(x)),infinity)"), "3");
        assert_eq!(lm("limit((x+1)*(ln(x+1)-ln(x)),infinity)"), "1");
    }

    #[test]
    fn a_long_exact_fraction_prints_as_a_decimal() {
        // 3^30/2^30 has 15 digits over 10; the reference switches to the
        // decimal form at precision + 3 digits.
        assert_eq!(
            lm("limit(((2x-3)^20*(3x+2)^30)/(2x+1)^50,-infinity)"),
            "191751.0592"
        );
    }

    #[test]
    fn symbolic_exponents_survive() {
        assert_eq!(lm("limit((1-x^(1/z))/(1-x^(1/y)),1)"), "y / z");
    }

    #[test]
    fn inverse_trigonometric_limits() {
        assert_eq!(lm("limit(asin(5x)/(3x),0)"), "5/3");
        assert_eq!(lm("limit(acot(x)/(x^2-x),infinity)"), "0");
        assert_eq!(lm("limit(sqrt(pi/2-atan(1/(x-1)^2)),1)"), "0");
    }
}
