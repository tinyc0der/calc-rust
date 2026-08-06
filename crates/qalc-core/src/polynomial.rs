//! Polynomial algebra — port of `MathStructure-polynomial.cc` and the parts
//! of `MathStructure-gcd.cc` / `MathStructure-factor.cc` the transcripts
//! exercise.
//!
//! A "polynomial" here is an ordinary [`MathStructure`] whose shape happens
//! to be a sum of terms in some symbol `xvar` — exactly the C++ convention.
//! Nothing is converted into a dense coefficient vector at the API level;
//! [`degree`], [`ldegree`] and [`coefficient`] read the tree directly, which
//! is what makes `coeff(3x + 2y + 4, 1, y)` work without choosing a main
//! variable up front. The dense form only appears inside [`factor`] and
//! [`roots`], where it is genuinely needed.
//!
//! Implemented (and verified against the reference binary):
//! `degree`, `ldegree`, `coeff`, `lcoeff`, `tcoeff`, `punit`, `pcontent`,
//! `primpart`, polynomial quotient/division, a reduced `gcd`, square-free
//! decomposition and rational-root factorization.
//!
//! TODO(port): `sr_gcd`/`heur_gcd` (subresultant and heuristic GCD) are
//! replaced by a univariate subresultant PRS plus the multivariate special
//! cases the transcripts need; Zassenhaus/Hensel factorization over Z is not
//! ported (only rational roots and quadratic factors).

use crate::options::EvaluationOptions;
use crate::structure::MathStructure;
use qalc_num::Number;

// ----------------------------------------------------------------------
// Function ids (values from `BuiltinFunctions.h`)
// ----------------------------------------------------------------------

pub mod id {
    pub const POLYNOMIAL_UNIT: u32 = 2000;
    pub const POLYNOMIAL_PRIMPART: u32 = 2001;
    pub const POLYNOMIAL_CONTENT: u32 = 2002;
    pub const COEFF: u32 = 2003;
    pub const L_COEFF: u32 = 2004;
    pub const T_COEFF: u32 = 2005;
    pub const DEGREE: u32 = 2006;
    pub const L_DEGREE: u32 = 2007;
    /// `factorize()`/`factor` — `FUNCTION_ID_FACTORIZE` has no stable value
    /// in the C++ header (it is a plain `MathFunction`), so the port picks a
    /// free slot in the polynomial range.
    pub const FACTORIZE: u32 = 2010;
    pub const EXPAND: u32 = 2011;
}

// ----------------------------------------------------------------------
// Small structural helpers
// ----------------------------------------------------------------------

/// `IS_A_SYMBOL(x)` — a symbolic leaf (the C++ also accepts an unknown
/// variable; this port represents unknowns as `Symbolic`).
pub fn is_symbol(m: &MathStructure) -> bool {
    m.is_symbolic()
}

/// `MathStructure::hasNegativeSign` (`MathStructure.cc`).
pub fn has_negative_sign(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => n.is_negative(),
        MathStructure::Multiplication(v) => v.first().is_some_and(has_negative_sign),
        _ => false,
    }
}

/// `MathStructure::overallCoefficient` (`MathStructure-polynomial.cc:1061`).
pub fn overall_coefficient(m: &MathStructure) -> Number {
    match m {
        MathStructure::Number(n) => n.clone(),
        MathStructure::Multiplication(v) => v
            .iter()
            .find_map(MathStructure::number)
            .cloned()
            .unwrap_or_else(|| Number::from_i64(1)),
        MathStructure::Addition(v) => v
            .iter()
            .find_map(MathStructure::number)
            .cloned()
            .unwrap_or_else(Number::new),
        _ => Number::new(),
    }
}

/// The terms of `m` viewed as an addition (a non-addition is a single term).
fn terms(m: &MathStructure) -> Vec<&MathStructure> {
    match m {
        MathStructure::Addition(v) => v.iter().collect(),
        other => vec![other],
    }
}

/// `x`, `x^n` → `Some(n)` when the base equals `xvar`.
fn power_of(m: &MathStructure, xvar: &MathStructure) -> Option<Number> {
    if m.equals(xvar) {
        return Some(Number::from_i64(1));
    }
    if let MathStructure::Power { base, exponent } = m {
        if base.equals(xvar) {
            if let MathStructure::Number(n) = exponent.as_ref() {
                return Some(n.clone());
            }
        }
    }
    None
}

// ----------------------------------------------------------------------
// degree / ldegree / coefficient
// ----------------------------------------------------------------------

/// `MathStructure::degree` (`MathStructure-polynomial.cc:227`) — the highest
/// power of `xvar` occurring in `m`, or zero when it does not occur.
pub fn degree(m: &MathStructure, xvar: &MathStructure) -> Number {
    let mut best: Option<Number> = None;
    for term in terms(m) {
        if let Some(p) = power_of(term, xvar) {
            if best.as_ref().is_none_or(|c| c.is_less_than(&p)) {
                best = Some(p);
            }
        } else if let MathStructure::Multiplication(fs) = term {
            for f in fs {
                if let Some(p) = power_of(f, xvar) {
                    if best.as_ref().is_none_or(|c| c.is_less_than(&p)) {
                        best = Some(p);
                    }
                }
            }
        }
    }
    best.unwrap_or_else(Number::new)
}

/// `MathStructure::ldegree` (`MathStructure-polynomial.cc:263`) — the lowest
/// power of `xvar`. A term free of `xvar` short-circuits the answer to zero,
/// exactly as the C++ `return nr_zero` does.
pub fn ldegree(m: &MathStructure, xvar: &MathStructure) -> Number {
    let mut best: Option<Number> = None;
    for term in terms(m) {
        if term.equals(xvar) {
            best = Some(Number::from_i64(1));
            continue;
        }
        if let Some(p) = power_of(term, xvar) {
            if best.as_ref().is_none_or(|c| c.is_greater_than(&p)) {
                best = Some(p);
            }
            continue;
        }
        if let MathStructure::Multiplication(fs) = term {
            let mut found = false;
            for f in fs {
                if f.equals(xvar) {
                    best = Some(Number::from_i64(1));
                    found = true;
                } else if let Some(p) = power_of(f, xvar) {
                    if best.as_ref().is_none_or(|c| c.is_greater_than(&p)) {
                        best = Some(p);
                    }
                    found = true;
                }
            }
            if !found {
                return Number::new();
            }
            continue;
        }
        return Number::new();
    }
    best.unwrap_or_else(Number::new)
}

/// `MathStructure::coefficient` (`MathStructure-polynomial.cc:307`) — the
/// coefficient of `xvar^pow`.
pub fn coefficient(m: &MathStructure, xvar: &MathStructure, pow: &Number) -> MathStructure {
    let mut acc: Option<MathStructure> = None;
    let add = |acc: &mut Option<MathStructure>, piece: MathStructure| match acc {
        Some(a) => a.add(piece, true),
        None => *acc = Some(piece),
    };
    for term in terms(m) {
        if let Some(p) = power_of(term, xvar) {
            if p.equals(pow, false, false) {
                add(&mut acc, MathStructure::from(1));
            }
            continue;
        }
        if let MathStructure::Multiplication(fs) = term {
            let mut has_var = false;
            let mut hit: Option<usize> = None;
            for (i, f) in fs.iter().enumerate() {
                if let Some(p) = power_of(f, xvar) {
                    has_var = true;
                    if hit.is_none() && p.equals(pow, false, false) {
                        hit = Some(i);
                    }
                }
            }
            if let Some(i) = hit {
                if fs.len() == 1 {
                    add(&mut acc, MathStructure::from(1));
                } else {
                    let rest: Vec<MathStructure> = fs
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .map(|(_, f)| f.clone())
                        .collect();
                    let piece = if rest.len() == 1 {
                        rest.into_iter().next().expect("len 1")
                    } else {
                        MathStructure::Multiplication(rest)
                    };
                    add(&mut acc, piece);
                }
            } else if !has_var && pow.is_zero() {
                add(&mut acc, term.clone());
            }
            continue;
        }
        if pow.is_zero() {
            add(&mut acc, term.clone());
        }
    }
    let mut out = acc.unwrap_or_else(MathStructure::new);
    // C++ finishes with `mcoeff.evalSort()`; the port normalizes through the
    // ordinary merge engine so `x + x` style duplicates collapse.
    let eo = EvaluationOptions::default();
    out.calculatesub(&eo);
    crate::sort::sort(&mut out);
    out
}

/// `MathStructure::lcoefficient` — the coefficient of the highest power.
pub fn lcoefficient(m: &MathStructure, xvar: &MathStructure) -> MathStructure {
    coefficient(m, xvar, &degree(m, xvar))
}

/// `MathStructure::tcoefficient` — the coefficient of the lowest power.
pub fn tcoefficient(m: &MathStructure, xvar: &MathStructure) -> MathStructure {
    coefficient(m, xvar, &ldegree(m, xvar))
}

/// `MathStructure::polynomialUnit` (`MathStructure-polynomial.cc:35`).
pub fn polynomial_unit(m: &MathStructure, xvar: &MathStructure) -> i32 {
    if has_negative_sign(&lcoefficient(m, xvar)) {
        -1
    } else {
        1
    }
}

// ----------------------------------------------------------------------
// content / primitive part
// ----------------------------------------------------------------------

/// `integer_content` (`MathStructure-polynomial.cc:42`): the gcd of the
/// numerators of the term coefficients over the lcm of their denominators.
pub fn integer_content(m: &MathStructure) -> Number {
    match m {
        MathStructure::Number(n) => {
            let mut c = n.clone();
            c.abs();
            c
        }
        MathStructure::Addition(v) => {
            let mut icontent = Number::new();
            let mut l = Number::from_i64(1);
            for term in v {
                match term {
                    MathStructure::Number(n) => {
                        if !icontent.is_one() {
                            let c = icontent.clone();
                            icontent = n.numerator();
                            icontent.gcd(&c);
                        }
                        let l2 = l.clone();
                        l = n.denominator();
                        l.lcm(&l2);
                    }
                    MathStructure::Multiplication(_) => {
                        let oc = overall_coefficient(term);
                        if !icontent.is_one() {
                            let c = icontent.clone();
                            icontent = oc.numerator();
                            icontent.gcd(&c);
                        }
                        let l2 = l.clone();
                        l = oc.denominator();
                        l.lcm(&l2);
                    }
                    _ => icontent = Number::from_i64(1),
                }
            }
            icontent.divide(&l);
            icontent
        }
        MathStructure::Multiplication(_) => {
            let mut c = overall_coefficient(m);
            c.abs();
            c
        }
        _ => Number::from_i64(1),
    }
}

/// `MathStructure::polynomialContent` (`MathStructure-polynomial.cc:80`).
pub fn polynomial_content(
    m: &MathStructure,
    xvar: &MathStructure,
    eo: &EvaluationOptions,
) -> MathStructure {
    if m.is_zero() {
        return MathStructure::new();
    }
    if let MathStructure::Number(n) = m {
        let mut c = n.clone();
        c.abs();
        return MathStructure::Number(c);
    }
    let c = integer_content(m);
    let mut r = m.clone();
    if !c.is_one() {
        r.calculate_divide(MathStructure::Number(c.clone()), eo);
    }
    let lcoeff = lcoefficient(&r, xvar);
    if lcoeff.is_integer() {
        return MathStructure::Number(c);
    }
    let deg = degree(&r, xvar);
    let ldeg = ldegree(&r, xvar);
    if deg.equals(&ldeg, false, false) {
        let mut content = lcoeff.clone();
        let mut c = c;
        if polynomial_unit(&lcoeff, xvar) == -1 {
            c.negate();
        }
        content.calculate_multiply(MathStructure::Number(c), eo);
        return content;
    }
    let mut content = MathStructure::new();
    let mut i = ldeg.clone();
    while i.is_less_than_or_equal_to(&deg) {
        // Note: the C++ takes the coefficients of `*this`, not of `r`.
        let coeff = coefficient(m, xvar, &i);
        let tmp = content.clone();
        content = gcd(&coeff, &tmp, eo).unwrap_or_else(|| MathStructure::from(1));
        if content.is_one() {
            break;
        }
        if !i.add_i64(1) {
            break;
        }
    }
    if !c.is_one() {
        content.calculate_multiply(MathStructure::Number(c), eo);
    }
    content
}

/// `MathStructure::polynomialPrimpart` (`MathStructure-polynomial.cc:124`).
pub fn polynomial_primpart(
    m: &MathStructure,
    xvar: &MathStructure,
    eo: &EvaluationOptions,
) -> MathStructure {
    if m.is_zero() {
        return MathStructure::new();
    }
    if m.is_number() {
        return MathStructure::from(1);
    }
    let c = polynomial_content(m, xvar, eo);
    if c.is_zero() {
        return MathStructure::new();
    }
    let negative = polynomial_unit(m, xvar) == -1;
    if let MathStructure::Number(n) = &c {
        let mut d = n.clone();
        if negative {
            d.negate();
        }
        let mut prim = m.clone();
        prim.calculate_divide(MathStructure::Number(d), eo);
        return prim;
    }
    let mut d = c;
    if negative {
        d.calculate_negate_eo(eo);
    }
    polynomial_quotient(m, &d, xvar, eo).unwrap_or_else(|| m.clone())
}

// ----------------------------------------------------------------------
// division
// ----------------------------------------------------------------------

/// `MathStructure::polynomialQuotient` (`MathStructure-polynomial.cc:377`).
pub fn polynomial_quotient(
    num: &MathStructure,
    den: &MathStructure,
    xvar: &MathStructure,
    eo: &EvaluationOptions,
) -> Option<MathStructure> {
    polynomial_division_remainder(num, den, xvar, eo).map(|(q, _)| q)
}

/// Calculates quotient and remainder of polynomial long division.
pub fn polynomial_division_remainder(
    num: &MathStructure,
    den: &MathStructure,
    xvar: &MathStructure,
    eo: &EvaluationOptions,
) -> Option<(MathStructure, MathStructure)> {
    let mut inner_eo = eo.clone();
    inner_eo.reduce_divisions = false;
    let eo = &inner_eo;
    if den.is_zero() {
        return None;
    }
    if num.is_zero() {
        return Some((MathStructure::new(), MathStructure::new()));
    }
    if let (MathStructure::Number(a), MathStructure::Number(b)) = (num, den) {
        let mut q = a.clone();
        if !q.divide(b) {
            return None;
        }
        return Some((MathStructure::Number(q), MathStructure::new()));
    }
    if num.equals(den) {
        return Some((MathStructure::from(1), MathStructure::new()));
    }

    let mut numdeg = degree(num, xvar);
    let dendeg = degree(den, xvar);
    let dencoeff = coefficient(den, xvar, &dendeg);
    let mut rem = num.clone();
    let mut quotient = MathStructure::new();
    let mut guard = 0;
    while numdeg.is_greater_than_or_equal_to(&dendeg) {
        guard += 1;
        if guard > 1000 {
            return None;
        }
        let mut numcoeff = coefficient(&rem, xvar, &numdeg);
        if !numdeg.subtract(&dendeg) {
            return None;
        }
        if numcoeff.equals(&dencoeff) {
            numcoeff = if numdeg.is_zero() {
                MathStructure::from(1)
            } else {
                var_power(xvar, &numdeg)
            };
        } else {
            if let MathStructure::Number(d) = &dencoeff {
                if let MathStructure::Number(n) = &numcoeff {
                    let mut q = n.clone();
                    if !q.divide(d) {
                        return None;
                    }
                    numcoeff = MathStructure::Number(q);
                } else {
                    numcoeff.calculate_divide(dencoeff.clone(), eo);
                }
            } else {
                numcoeff = polynomial_divide(&numcoeff, &dencoeff, eo)?;
            }
            if !numdeg.is_zero() && !numcoeff.is_zero() {
                if numcoeff.is_one() {
                    numcoeff = var_power(xvar, &numdeg);
                } else {
                    numcoeff.calculate_multiply(var_power(xvar, &numdeg), eo);
                }
            }
        }
        if quotient.is_zero() {
            quotient = numcoeff.clone();
        } else {
            quotient.calculate_add(numcoeff.clone(), eo);
        }
        numcoeff.calculate_multiply(den.clone(), eo);
        rem.calculate_subtract(numcoeff, eo);
        if rem.is_zero() {
            break;
        }
        numdeg = degree(&rem, xvar);
    }
    Some((quotient, rem))
}

/// `xvar^n`, collapsing `n == 1` to the bare symbol.
fn var_power(xvar: &MathStructure, n: &Number) -> MathStructure {
    if n.is_one() {
        xvar.clone()
    } else {
        MathStructure::Power {
            base: Box::new(xvar.clone()),
            exponent: Box::new(MathStructure::Number(n.clone())),
        }
    }
}

/// `MathStructure::polynomialDivide` (`MathStructure-polynomial.cc:547`) —
/// exact division, returning `None` when the division leaves a remainder.
pub fn polynomial_divide(
    num: &MathStructure,
    den: &MathStructure,
    eo: &EvaluationOptions,
) -> Option<MathStructure> {
    if den.is_zero() {
        return None;
    }
    if num.is_zero() {
        return Some(MathStructure::new());
    }
    if let (MathStructure::Number(a), MathStructure::Number(b)) = (num, den) {
        let mut q = a.clone();
        if !q.divide(b) {
            return None;
        }
        return Some(MathStructure::Number(q));
    }
    if num.equals(den) {
        return Some(MathStructure::from(1));
    }
    if let MathStructure::Number(b) = den {
        let mut q = num.clone();
        let mut inv = b.clone();
        if !inv.recip() {
            return None;
        }
        q.calculate_multiply(MathStructure::Number(inv), eo);
        return Some(q);
    }
    let xvar = first_symbol(den).or_else(|| first_symbol(num))?;
    let q = polynomial_quotient(num, den, &xvar, eo)?;
    // Verify exactness: `q * den` must reproduce `num`.
    let mut check = q.clone();
    check.calculate_multiply(den.clone(), eo);
    check.calculate_subtract(num.clone(), eo);
    check.calculatesub(eo);
    if check.is_zero() {
        Some(q)
    } else {
        None
    }
}

/// `get_first_symbol` (`MathStructure-polynomial.cc:534`).
pub fn first_symbol(m: &MathStructure) -> Option<MathStructure> {
    if is_symbol(m) {
        return Some(m.clone());
    }
    match m {
        MathStructure::Addition(v) | MathStructure::Multiplication(v) => {
            v.iter().find_map(first_symbol)
        }
        MathStructure::Power { base, .. } => first_symbol(base),
        _ => None,
    }
}

/// `collect_symbols` (`MathStructure-polynomial.cc:480`).
pub fn collect_symbols(m: &MathStructure, out: &mut Vec<MathStructure>) {
    if is_symbol(m) {
        if !out.iter().any(|s| s.equals(m)) {
            out.push(m.clone());
        }
        return;
    }
    match m {
        MathStructure::Addition(v) | MathStructure::Multiplication(v) => {
            for c in v {
                collect_symbols(c, out);
            }
        }
        MathStructure::Power { base, .. } => collect_symbols(base, out),
        _ => {}
    }
}

// ----------------------------------------------------------------------
// gcd
// ----------------------------------------------------------------------

/// `MathStructure::gcd` (`MathStructure-gcd.cc:323`), reduced to the cases
/// the transcripts reach: numbers, equal operands, zero, a factored
/// multiplication, and the "one operand has degree 0 in the main variable"
/// branch. Univariate integer polynomials go through a subresultant PRS.
///
/// TODO(port): `heur_gcd`, the full `sr_gcd` with symbol statistics, and the
/// `ca`/`cb` cofactor outputs.
pub fn gcd(
    m1: &MathStructure,
    m2: &MathStructure,
    eo: &EvaluationOptions,
) -> Option<MathStructure> {
    if m1.is_one() || m2.is_one() {
        return Some(MathStructure::from(1));
    }
    if let (MathStructure::Number(a), MathStructure::Number(b)) = (m1, m2) {
        let mut g = a.clone();
        if !a.is_integer() || !b.is_integer() || !g.gcd(b) {
            return Some(MathStructure::from(1));
        }
        return Some(MathStructure::Number(g));
    }
    if m1.equals(m2) {
        return Some(m1.clone());
    }
    if m1.is_zero() {
        return Some(m2.clone());
    }
    if m2.is_zero() {
        return Some(m1.clone());
    }

    let mut syms = Vec::new();
    collect_symbols(m1, &mut syms);
    collect_symbols(m2, &mut syms);
    if syms.is_empty() {
        return Some(MathStructure::from(1));
    }
    // Pick the variable of maximum degree, like `get_symbol_stats` ordering.
    let xvar = syms
        .iter()
        .max_by(|a, b| {
            let da = degree(m1, a).max_of(&degree(m2, a));
            let db = degree(m1, b).max_of(&degree(m2, b));
            if da.is_less_than(&db) {
                std::cmp::Ordering::Less
            } else if db.is_less_than(&da) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })?
        .clone();

    let deg_a = degree(m1, &xvar);
    let deg_b = degree(m2, &xvar);
    // The `deg == 0` branches: reduce to the content of the other operand.
    if deg_a.is_zero() {
        let c = polynomial_content(m2, &xvar, eo);
        if c.equals(m2) {
            return Some(MathStructure::from(1));
        }
        return gcd(m1, &c, eo);
    }
    if deg_b.is_zero() {
        let c = polynomial_content(m1, &xvar, eo);
        if c.equals(m1) {
            return Some(MathStructure::from(1));
        }
        return gcd(&c, m2, eo);
    }

    // Univariate integer case: content * primitive subresultant gcd.
    let (a, b) = (
        to_dense(m1, &xvar)?,
        to_dense(m2, &xvar)?,
    );
    let g = dense_gcd(&a, &b)?;
    let cont_a = dense_content(&a);
    let cont_b = dense_content(&b);
    let mut cont_gcd = cont_a;
    cont_gcd.gcd(&cont_b);
    let scaled_g = dense_scale(&g, &cont_gcd);
    Some(from_dense(&scaled_g, &xvar))
}

trait MaxOf {
    fn max_of(&self, other: &Self) -> Self;
}
impl MaxOf for Number {
    fn max_of(&self, other: &Self) -> Self {
        if self.is_less_than(other) {
            other.clone()
        } else {
            self.clone()
        }
    }
}

// ----------------------------------------------------------------------
// Dense univariate representation (rational coefficients, index = degree)
// ----------------------------------------------------------------------

/// Convert `m` to a dense coefficient vector in `xvar`, lowest degree first.
/// Returns `None` when a coefficient is not a plain rational number or an
/// exponent is not a non-negative integer.
pub fn to_dense(m: &MathStructure, xvar: &MathStructure) -> Option<Vec<Number>> {
    let mut out: Vec<Number> = Vec::new();
    for term in terms(m) {
        let (coeff, pow) = split_term(term, xvar)?;
        let p = pow.to_i64()?;
        if p < 0 || p > 10_000 {
            return None;
        }
        let p = p as usize;
        while out.len() <= p {
            out.push(Number::new());
        }
        if !out[p].add(&coeff) {
            return None;
        }
    }
    while out.len() > 1 && out.last().is_some_and(Number::is_zero) {
        out.pop();
    }
    if out.is_empty() {
        out.push(Number::new());
    }
    Some(out)
}

/// [`to_dense`] allowing fractional exponents: returns the coefficients in
/// `u = xvar^(1/root)` together with `root`.
pub fn to_dense_rational(m: &MathStructure, xvar: &MathStructure) -> Option<(Vec<Number>, i64)> {
    // First pass: the lcm of the exponent denominators.
    let mut root = Number::from_i64(1);
    for term in terms(m) {
        let (_, pow) = split_term(term, xvar)?;
        if !pow.is_rational() {
            return None;
        }
        let d = pow.denominator();
        if !root.lcm(&d) {
            return None;
        }
    }
    let root_i = root.to_i64()?;
    if root_i < 1 || root_i > 64 {
        return None;
    }
    let mut out: Vec<Number> = Vec::new();
    for term in terms(m) {
        let (coeff, mut pow) = split_term(term, xvar)?;
        if !pow.multiply(&root) {
            return None;
        }
        let p = pow.to_i64()?;
        if p < 0 || p > 10_000 {
            return None;
        }
        let p = p as usize;
        while out.len() <= p {
            out.push(Number::new());
        }
        if !out[p].add(&coeff) {
            return None;
        }
    }
    while out.len() > 1 && out.last().is_some_and(Number::is_zero) {
        out.pop();
    }
    if out.is_empty() {
        out.push(Number::new());
    }
    Some((out, root_i))
}

/// Split a term into `(numeric coefficient, power of xvar)`.
fn split_term(term: &MathStructure, xvar: &MathStructure) -> Option<(Number, Number)> {
    if let MathStructure::Number(n) = term {
        return Some((n.clone(), Number::new()));
    }
    if let Some(p) = power_of(term, xvar) {
        return Some((Number::from_i64(1), p));
    }
    if let MathStructure::Multiplication(fs) = term {
        let mut coeff = Number::from_i64(1);
        let mut pow = Number::new();
        for f in fs {
            if let Some(p) = power_of(f, xvar) {
                if !pow.add(&p) {
                    return None;
                }
            } else if let MathStructure::Number(n) = f {
                if !coeff.multiply(n) {
                    return None;
                }
            } else {
                return None;
            }
        }
        return Some((coeff, pow));
    }
    None
}

/// Rebuild a structure from a dense coefficient vector.
pub fn from_dense(c: &[Number], xvar: &MathStructure) -> MathStructure {
    let mut ts: Vec<MathStructure> = Vec::new();
    for (i, coeff) in c.iter().enumerate().rev() {
        if coeff.is_zero() {
            continue;
        }
        if i == 0 {
            ts.push(MathStructure::Number(coeff.clone()));
        } else if coeff.is_one() {
            ts.push(var_power(xvar, &Number::from_i64(i as i64)));
        } else {
            ts.push(MathStructure::Multiplication(vec![
                MathStructure::Number(coeff.clone()),
                var_power(xvar, &Number::from_i64(i as i64)),
            ]));
        }
    }
    match ts.len() {
        0 => MathStructure::new(),
        1 => ts.into_iter().next().expect("len 1"),
        _ => MathStructure::Addition(ts),
    }
}

fn dense_degree(c: &[Number]) -> usize {
    let mut d = c.len();
    while d > 0 && c[d - 1].is_zero() {
        d -= 1;
    }
    d.saturating_sub(1)
}

fn dense_is_zero(c: &[Number]) -> bool {
    c.iter().all(Number::is_zero)
}

/// The rational content (gcd of numerators / lcm of denominators).
fn dense_content(c: &[Number]) -> Number {
    let mut num = Number::new();
    let mut den = Number::from_i64(1);
    for x in c {
        if x.is_zero() {
            continue;
        }
        let mut n = x.numerator();
        n.abs();
        num.gcd(&n);
        let d = x.denominator();
        den.lcm(&d);
    }
    if num.is_zero() {
        return Number::from_i64(1);
    }
    num.divide(&den);
    num
}

fn dense_scale(c: &[Number], k: &Number) -> Vec<Number> {
    c.iter()
        .map(|x| {
            let mut y = x.clone();
            y.multiply(k);
            y
        })
        .collect()
}

fn dense_primitive(c: &[Number]) -> Vec<Number> {
    let cont = dense_content(c);
    if cont.is_zero() || cont.is_one() {
        return c.to_vec();
    }
    let mut inv = cont;
    if !inv.recip() {
        return c.to_vec();
    }
    dense_scale(c, &inv)
}

/// Pseudo-remainder of `a` by `b` (both dense, rational coefficients).
fn dense_prem(a: &[Number], b: &[Number]) -> Option<Vec<Number>> {
    let db = dense_degree(b);
    let mut r = a.to_vec();
    let lb = b[db].clone();
    loop {
        let dr = dense_degree(&r);
        if dense_is_zero(&r) || dr < db {
            break;
        }
        let mut factor = r[dr].clone();
        if !factor.divide(&lb) {
            return None;
        }
        let shift = dr - db;
        for i in 0..=db {
            let mut t = b[i].clone();
            t.multiply(&factor);
            if !r[i + shift].subtract(&t) {
                return None;
            }
        }
        r[dr] = Number::new();
    }
    while r.len() > 1 && r.last().is_some_and(Number::is_zero) {
        r.pop();
    }
    Some(r)
}

/// Univariate polynomial gcd over Q, normalized to a primitive integer
/// polynomial with positive leading coefficient (C++ `sr_gcd` result shape).
pub fn dense_gcd(a: &[Number], b: &[Number]) -> Option<Vec<Number>> {
    if dense_is_zero(a) {
        return Some(dense_primitive(b));
    }
    if dense_is_zero(b) {
        return Some(dense_primitive(a));
    }
    let mut x = dense_primitive(a);
    let mut y = dense_primitive(b);
    let mut guard = 0;
    while !dense_is_zero(&y) {
        guard += 1;
        if guard > 500 {
            return None;
        }
        if dense_degree(&x) < dense_degree(&y) {
            std::mem::swap(&mut x, &mut y);
            continue;
        }
        let r = dense_prem(&x, &y)?;
        x = y;
        y = dense_primitive(&r);
    }
    let mut g = dense_primitive(&x);
    let d = dense_degree(&g);
    if g[d].is_negative() {
        for c in g.iter_mut() {
            c.negate();
        }
    }
    Some(g)
}

/// Exact dense division; `None` if there is a remainder.
pub fn dense_divide(a: &[Number], b: &[Number]) -> Option<Vec<Number>> {
    let db = dense_degree(b);
    let da = dense_degree(a);
    if dense_is_zero(b) || da < db {
        return if dense_is_zero(a) {
            Some(vec![Number::new()])
        } else {
            None
        };
    }
    let mut r = a.to_vec();
    let mut q = vec![Number::new(); da - db + 1];
    for k in (0..=(da - db)).rev() {
        let mut f = r[k + db].clone();
        if !f.divide(&b[db]) {
            return None;
        }
        if f.is_zero() {
            continue;
        }
        q[k] = f.clone();
        for i in 0..=db {
            let mut t = b[i].clone();
            t.multiply(&f);
            r[i + k].subtract(&t);
        }
    }
    if !dense_is_zero(&r) {
        return None;
    }
    Some(q)
}

fn dense_derivative(c: &[Number]) -> Vec<Number> {
    if c.len() <= 1 {
        return vec![Number::new()];
    }
    let mut out = Vec::with_capacity(c.len() - 1);
    for (i, x) in c.iter().enumerate().skip(1) {
        let mut y = x.clone();
        y.multiply(&Number::from_i64(i as i64));
        out.push(y);
    }
    out
}

/// Square-free decomposition (Yun's algorithm), returning `(factor,
/// multiplicity)` pairs — the port of `sqrfree` in `MathStructure-factor.cc`.
pub fn dense_sqrfree(c: &[Number]) -> Vec<(Vec<Number>, usize)> {
    let mut out = Vec::new();
    let p = dense_primitive(c);
    if dense_degree(&p) == 0 {
        return out;
    }
    let d = dense_derivative(&p);
    let Some(g) = dense_gcd(&p, &d) else {
        out.push((p, 1));
        return out;
    };
    if dense_degree(&g) == 0 {
        out.push((p, 1));
        return out;
    }
    let Some(mut w) = dense_divide(&p, &g) else {
        out.push((p, 1));
        return out;
    };
    let Some(mut y) = dense_divide(&d, &g) else {
        out.push((p, 1));
        return out;
    };
    let mut i = 1usize;
    loop {
        let dw = dense_derivative(&w);
        let mut z = y.clone();
        // z = y - w'
        while z.len() < dw.len() {
            z.push(Number::new());
        }
        for (k, t) in dw.iter().enumerate() {
            z[k].subtract(t);
        }
        if dense_is_zero(&z) {
            if dense_degree(&w) > 0 {
                out.push((dense_primitive(&w), i));
            }
            break;
        }
        let Some(g2) = dense_gcd(&w, &z) else { break };
        if dense_degree(&g2) > 0 {
            out.push((dense_primitive(&g2), i));
        }
        let Some(nw) = dense_divide(&w, &g2) else { break };
        let Some(ny) = dense_divide(&z, &g2) else { break };
        w = nw;
        y = ny;
        i += 1;
        if i > 64 {
            break;
        }
    }
    out
}

/// Rational roots of a dense integer/rational polynomial, via the rational
/// root theorem. Used by [`factor`] and by the equation solver.
pub fn rational_roots(c: &[Number]) -> Vec<Number> {
    let mut roots = Vec::new();
    let prim = dense_primitive(c);
    let deg = dense_degree(&prim);
    if deg == 0 {
        return roots;
    }
    // Strip the x^k factor first: x = 0 is a root of multiplicity k.
    let mut work = prim;
    let mut shift = 0usize;
    while work.len() > 1 && work[0].is_zero() {
        work.remove(0);
        shift += 1;
    }
    if shift > 0 {
        roots.push(Number::new());
    }
    let d = dense_degree(&work);
    if d == 0 {
        return roots;
    }
    let (Some(a0), Some(an)) = (work[0].to_i64(), work[d].to_i64()) else {
        return roots;
    };
    let ps = divisors(a0);
    let qs = divisors(an);
    let mut seen: Vec<Number> = Vec::new();
    for p in &ps {
        for q in &qs {
            for sign in [1i64, -1] {
                let r = Number::from_ints(sign * *p, *q, 0);
                if seen.iter().any(|s| s.equals(&r, false, false)) {
                    continue;
                }
                seen.push(r.clone());
                if dense_eval(&work, &r).is_zero() {
                    roots.push(r);
                }
            }
        }
    }
    roots
}

/// Evaluate a dense polynomial at `x` (Horner).
pub fn dense_eval(c: &[Number], x: &Number) -> Number {
    let mut acc = Number::new();
    for coeff in c.iter().rev() {
        acc.multiply(x);
        acc.add(coeff);
    }
    acc
}

/// Positive divisors of a non-zero integer (bounded, so a huge coefficient
/// simply yields fewer candidate roots instead of hanging).
fn divisors(z: i64) -> Vec<i64> {
    let n = z.unsigned_abs();
    if n == 0 {
        return vec![1];
    }
    let mut out = Vec::new();
    let mut d = 1u64;
    while d.saturating_mul(d) <= n && d < 1_000_000 {
        if n % d == 0 {
            if d <= i64::MAX as u64 {
                out.push(d as i64);
            }
            let div = n / d;
            if div != d && div <= i64::MAX as u64 {
                out.push(div as i64);
            }
        }
        d += 1;
    }
    if out.is_empty() {
        out.push(1);
    }
    out.sort_unstable();
    out.dedup();
    out
}

// ----------------------------------------------------------------------
// factorization
// ----------------------------------------------------------------------

/// Factor `m` into a product of a numeric content and irreducible-ish
/// factors. Only univariate rational-root and square-free factorization is
/// ported; anything else is returned unchanged.
pub fn factor(m: &MathStructure, eo: &EvaluationOptions) -> MathStructure {
    // Prime factorization for plain integers: `factor(60)` -> `[2 2 3 5]`.
    // The polynomial factorizer below bails on numbers (`find_x_var` is
    // `None`), so handle them here before that. This mirrors the reference
    // where `factor(60)` is a number-theory builtin, not a polynomial one.
    if let MathStructure::Number(n) = m {
        let mut facs = Vec::new();
        if n.factorize(&mut facs) {
            return MathStructure::Vector(
                facs.into_iter()
                    .map(MathStructure::Number)
                    .collect(),
            );
        }
        return m.clone();
    }
    // A perfect-square trinomial needs no variable of its own and may have
    // two, so it is tried before the univariate machinery:
    // `x + 2sqrt(x)sqrt(y) + y` is `(sqrt(x) + sqrt(y))^2`.
    if let Some(square) = factor_perfect_square(m, eo) {
        return square;
    }
    let Some(xvar) = crate::polynomial::find_x_var(m) else {
        return m.clone();
    };
    // Radicals are handled by substituting `u = x^(1/d)`, where `d` is the
    // lcm of the fractional exponent denominators: `x + 2sqrt(x) + 1` becomes
    // `u^2 + 2u + 1` and factors as `(sqrt(x) + 1)^2`.
    let Some((dense, root)) = to_dense_rational(m, &xvar) else {
        return m.clone();
    };
    let uvar = if root == 1 {
        xvar.clone()
    } else {
        MathStructure::Power {
            base: Box::new(xvar.clone()),
            exponent: Box::new(MathStructure::Number(Number::from_ints(1, root, 0))),
        }
    };
    if dense_degree(&dense) < 2 {
        return m.clone();
    }
    let factors = factor_dense(&dense);
    // A single factor of multiplicity one is the polynomial itself.
    if factors.is_empty() || (factors.len() == 1 && factors[0].1 == 1) {
        return m.clone();
    }
    let xvar = uvar;
    let mut out: Vec<MathStructure> = Vec::new();
    for (f, mult) in factors {
        let mut s = from_dense(&f, &xvar);
        if mult > 1 {
            s = MathStructure::Power {
                base: Box::new(s),
                exponent: Box::new(MathStructure::from(mult as i64)),
            };
        }
        out.push(s);
    }
    let mut r = if out.len() == 1 {
        out.into_iter().next().expect("len 1")
    } else {
        MathStructure::Multiplication(out)
    };
    let mut eo2 = eo.clone();
    eo2.expand = 0;
    r.calculatesub(&eo2);
    crate::sort::sort(&mut r);
    r
}

/// Factor a dense polynomial into `(factor, multiplicity)` pairs, leading
/// numeric content first.
pub fn factor_dense(c: &[Number]) -> Vec<(Vec<Number>, usize)> {
    let mut out: Vec<(Vec<Number>, usize)> = Vec::new();
    let mut lead = Number::from_i64(1);
    let cont = dense_content(c);
    let mut work = dense_primitive(c);
    let d = dense_degree(&work);
    if work[d].is_negative() {
        for x in work.iter_mut() {
            x.negate();
        }
        lead.negate();
    }
    lead.multiply(&cont);
    for (sf, mult) in dense_sqrfree(&work) {
        let mut rest = sf;
        loop {
            let roots = rational_roots(&rest);
            let Some(r) = roots.into_iter().next() else {
                break;
            };
            // divide by (x - r)
            let mut neg = r.clone();
            neg.negate();
            let lin = vec![neg, Number::from_i64(1)];
            let Some(q) = dense_divide(&rest, &lin) else {
                break;
            };
            out.push((lin, mult));
            rest = q;
            if dense_degree(&rest) == 0 {
                break;
            }
        }
        if dense_degree(&rest) > 0 {
            let k = dense_content(&rest);
            lead.multiply(&k);
            out.push((dense_primitive(&rest), mult));
        } else if !rest[0].is_one() {
            lead.multiply(&rest[0]);
        }
    }
    if !lead.is_one() {
        out.insert(0, (vec![lead], 1));
    }
    out
}

// ----------------------------------------------------------------------
// find_x_var
// ----------------------------------------------------------------------

/// `MathStructure::find_x_var` (`MathStructure.cc:3140`).
///
/// The C++ prefers the predefined unknown variables `x`, then `y`, then `z`,
/// and pushes the integration constants `n`/`C` to the back; everything else
/// is compared by name. This port represents all of them as symbols, so the
/// preference order is reproduced by an explicit rank.
pub fn find_x_var(m: &MathStructure) -> Option<MathStructure> {
    let mut syms = Vec::new();
    collect_symbols(m, &mut syms);
    // Also look inside function arguments and comparisons, which
    // `collect_symbols` (a polynomial helper) deliberately does not descend.
    collect_all_symbols(m, &mut syms);
    syms.into_iter().min_by_key(|s| {
        let name = s.symbol().unwrap_or("").to_string();
        let rank = match name.as_str() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            "n" | "C" => 4,
            _ => 3,
        };
        (rank, name)
    })
}

fn collect_all_symbols(m: &MathStructure, out: &mut Vec<MathStructure>) {
    if is_symbol(m) {
        if !out.iter().any(|s| s.equals(m)) {
            out.push(m.clone());
        }
        return;
    }
    for i in 0..m.size() {
        if let Some(c) = m.get(i) {
            collect_all_symbols(c, out);
        }
    }
}

// ----------------------------------------------------------------------
// Builtin dispatch
// ----------------------------------------------------------------------

/// Evaluate a polynomial builtin in place. Returns true when handled.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    let eo = EvaluationOptions::default();
    // `SqrtFunction::calculate` returns `x^(1/2)` rather than a function call
    // (BuiltinFunctions-exponents.cc), which is what makes a radical sort
    // before the constant term of a sum and lets `merge_power` combine it.
    if fid == crate::builtins::id::SQRT && args.len() == 1 && !args[0].is_number() {
        let arg = args[0].clone();
        *m = MathStructure::Power {
            base: Box::new(arg),
            exponent: Box::new(MathStructure::Number(Number::from_ints(1, 2, 0))),
        };
        return true;
    }
    // Everything below belongs to a polynomial builtin. Bail out first for
    // anything else: this dispatcher runs for *every* `Function` node, and
    // the argument normalisation just below deep-clones the argument and runs
    // the whole evaluator over it. Without this gate that work happened once
    // per enclosing call as well as once for the call itself, so a nested
    // `sin(sin(…))` cost 2^depth evaluations of the innermost argument.
    if function_name(fid).is_none() {
        return false;
    }
    if fid == crate::builtins::id::GCD {
        if args.len() < 2
            || args.iter().all(|a| matches!(a, MathStructure::Number(_)))
            || args.iter().any(|a| a.is_zero())
        {
            return false;
        }
        let mut eval_args = Vec::with_capacity(args.len());
        for a in args {
            let mut p = a.clone();
            crate::eval::evaluate_calculated(&mut p);
            if p.is_zero() {
                return false;
            }
            eval_args.push(p);
        }
        if eval_args.iter().all(|a| matches!(a, MathStructure::Number(_))) {
            return false;
        }
        let mut acc = eval_args[0].clone();
        for next in &eval_args[1..] {
            let Some(g) = gcd(&acc, next, &eo) else {
                return false;
            };
            acc = g;
        }
        *m = acc;
        return true;
    }
    // The C++ `MathFunction::calculate` receives fully evaluated arguments
    // (`vargs`); this port's function dispatch runs before the merge engine,
    // so the polynomial argument is normalized here.
    let poly = match args.first() {
        Some(a) => {
            let mut p = a.clone();
            crate::eval::evaluate_calculated(&mut p);
            p
        }
        None => return false,
    };
    // The C++ `SymbolicArgument` with default "undefined" resolves to
    // `find_x_var(arg1)` (Function.cc:582).
    // When no unknown is found the C++ falls back to the predefined `x`
    // variable (`Function.cc:592`, "No unknown variable/symbol was found").
    let xvar_arg = |idx: usize| -> Option<MathStructure> {
        match args.get(idx) {
            Some(s @ MathStructure::Symbolic(_)) => Some(s.clone()),
            _ => Some(find_x_var(&poly).unwrap_or_else(|| MathStructure::symbolic("x"))),
        }
    };
    // A polynomial builtin whose main variable cannot be determined is left
    // unevaluated, like the C++ argument test failing.
    let pow_arg = match fid {
        id::COEFF => match args.get(1) {
            Some(MathStructure::Number(p)) => Some(p.clone()),
            _ => return false,
        },
        _ => None,
    };
    let xvar = match fid {
        id::FACTORIZE | id::EXPAND => None,
        id::COEFF => match xvar_arg(2) {
            Some(x) => Some(x),
            None => return false,
        },
        _ => match xvar_arg(1) {
            Some(x) => Some(x),
            None => return false,
        },
    };
    let result = match fid {
        id::DEGREE => MathStructure::Number(degree(&poly, xvar.as_ref().expect("xvar"))),
        id::L_DEGREE => MathStructure::Number(ldegree(&poly, xvar.as_ref().expect("xvar"))),
        id::L_COEFF => lcoefficient(&poly, xvar.as_ref().expect("xvar")),
        id::T_COEFF => tcoefficient(&poly, xvar.as_ref().expect("xvar")),
        id::POLYNOMIAL_UNIT => {
            MathStructure::from(polynomial_unit(&poly, xvar.as_ref().expect("xvar")) as i64)
        }
        id::POLYNOMIAL_CONTENT => {
            polynomial_content(&poly, xvar.as_ref().expect("xvar"), &eo)
        }
        id::POLYNOMIAL_PRIMPART => {
            polynomial_primpart(&poly, xvar.as_ref().expect("xvar"), &eo)
        }
        id::COEFF => coefficient(
            &poly,
            xvar.as_ref().expect("xvar"),
            &pow_arg.expect("power argument"),
        ),
        id::FACTORIZE => {
            let f = factor(&poly, &eo);
            // Function `factor` returns a vector of factors, matching
            // `qalc -t "factor(x^2-1)" → [(x + 1)  (x - 1)]`, while
            // `x^2-1 to factors` via the CLI keeps the product
            // `(x - 1)(x + 1)`. If `factor` did nothing (unfactored),
            // return the original to avoid wrapping `x^2+1` as `[x^2+1]`.
            if f.equals(&poly) {
                f
            } else {
                match f {
                    MathStructure::Multiplication(factors) => MathStructure::Vector(factors),
                    MathStructure::Vector(v) => MathStructure::Vector(v),
                    other => MathStructure::Vector(vec![other]),
                }
            }
        }
        id::EXPAND => {
            let mut e = poly.clone();
            let mut eo2 = eo.clone();
            eo2.expand = 1;
            e.calculatesub(&eo2);
            crate::sort::sort(&mut e);
            e
        }
        _ => return false,
    };
    *m = result;
    true
}

/// Resolve the name of a polynomial builtin.
pub fn function_id_for_name(name: &str) -> Option<crate::ids::FunctionId> {
    let id = match name {
        "coeff" => id::COEFF,
        "lcoeff" => id::L_COEFF,
        "tcoeff" => id::T_COEFF,
        "degree" => id::DEGREE,
        "ldegree" => id::L_DEGREE,
        "pcontent" => id::POLYNOMIAL_CONTENT,
        "primpart" => id::POLYNOMIAL_PRIMPART,
        "punit" => id::POLYNOMIAL_UNIT,
        "factorize" | "factor" => id::FACTORIZE,
        "expand" | "multiply" => id::EXPAND,
        "gcd" => crate::builtins::id::GCD,
        _ => return None,
    };
    Some(crate::ids::FunctionId(id))
}

/// Printable name for a polynomial builtin id.
pub fn function_name(id: u32) -> Option<&'static str> {
    Some(match id {
        id::COEFF => "coeff",
        id::L_COEFF => "lcoeff",
        id::T_COEFF => "tcoeff",
        id::DEGREE => "degree",
        id::L_DEGREE => "ldegree",
        id::POLYNOMIAL_CONTENT => "pcontent",
        id::POLYNOMIAL_PRIMPART => "primpart",
        id::POLYNOMIAL_UNIT => "punit",
        id::FACTORIZE => "factorize",
        id::EXPAND => "expand",
        crate::builtins::id::GCD => "gcd",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    /// Every expectation here was taken from the reference binary
    /// (`qalc -t +u8`), mostly from `tests/polynomial.batch`.
    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn coeff_transcript() {
        assert_eq!(ev("coeff(3x + 4, 0)"), "4");
        assert_eq!(ev("coeff(3y + 4, 1)"), "3");
        assert_eq!(ev("coeff(3a + 4, 2)"), "0");
        assert_eq!(ev("coeff(4x*(2x^2 + 5 -5x), 3)"), "8");
        assert_eq!(ev("coeff(x^3-7x^2-4x-5x^2, 2)"), "-12");
        assert_eq!(ev("coeff(1+x^3-4x-5x^2-1, 0)"), "0");
    }

    #[test]
    fn coeff_with_explicit_variable() {
        assert_eq!(ev("coeff(3x + 4, 1, x)"), "3");
        assert_eq!(ev("coeff(3x + 4, 1, y)"), "0");
        assert_eq!(ev("coeff(3x + 2y + 4, 1, y)"), "2");
    }

    #[test]
    fn degree_transcript() {
        assert_eq!(ev("degree(3x + 6)"), "1");
        assert_eq!(ev("degree(2x^3-4x^2-6x-8x^2)"), "3");
        assert_eq!(ev("degree(2x^3-3x^2-6x-2x^3)"), "2");
        assert_eq!(ev("degree(2x^3-3x^2-6x-2x^3, y)"), "0");
    }

    #[test]
    fn ldegree_transcript() {
        assert_eq!(ev("ldegree(3x)"), "1");
        assert_eq!(ev("ldegree(6 -5x^2 - 6)"), "2");
        assert_eq!(ev("ldegree(-5x^2, y)"), "0");
        assert_eq!(ev("ldegree(3yx^2 + 2y, y)"), "1");
    }

    #[test]
    fn lcoeff_transcript() {
        assert_eq!(ev("lcoeff(6+ 3x)"), "3");
        assert_eq!(ev("lcoeff(6 -5x^2 + 3x^2)"), "-2");
        assert_eq!(ev("lcoeff(6 -5x^2 + 3x^2, y)"), "6 - 2x^2");
        assert_eq!(ev("lcoeff(6 -5x^2 + 3x^2 + 2y, y)"), "2");
    }

    #[test]
    fn tcoeff_transcript() {
        assert_eq!(ev("tcoeff(6+ 3x)"), "6");
        assert_eq!(ev("tcoeff(-5x^2 + 3x - x)"), "2");
        assert_eq!(ev("tcoeff(6x -5x^2 + 3x^2, y)"), "6x - 2x^2");
        assert_eq!(ev("tcoeff(6 -5x^2 + 3x^2 + 2y, y)"), "6 - 2x^2");
    }

    #[test]
    fn pcontent_transcript() {
        assert_eq!(ev("pcontent(3x + 6)"), "3");
        assert_eq!(ev("pcontent(2x^3-4x^2-6x-8x^2)"), "2");
        assert_eq!(ev("pcontent(2y^3-3y^2-6y-8y^2)"), "1");
        assert_eq!(ev("pcontent(2x^3-3x^2-6x-8x^2, y)"), "2x^3 - 11x^2 - 6x");
        // Reference: pcontent(2xy + 8y + 16, y) == 4
        assert_eq!(ev("pcontent(2xy + 8y + 16, y)"), "4");
    }

    #[test]
    fn primpart_transcript() {
        assert_eq!(ev("primpart(3x + 6)"), "x + 2");
        assert_eq!(ev("primpart(-12x^3 + 30x - 20)"), "6x^3 - 15x + 10");
    }

    #[test]
    fn punit_transcript() {
        assert_eq!(ev("punit(-3x)"), "-1");
        assert_eq!(ev("punit(1-3x)"), "-1");
        assert_eq!(ev("punit(3x-1)"), "1");
    }

    #[test]
    fn degree_and_coefficient_primitives() {
        let x = MathStructure::symbolic("x");
        let m = crate::eval::parse_expression("x^3 - 5x^2 - 4x + 20").expect("parses");
        let mut m = m;
        crate::eval::evaluate(&mut m);
        assert!(degree(&m, &x).equals_i64(3));
        assert!(ldegree(&m, &x).equals_i64(0));
        assert!(lcoefficient(&m, &x).is_one());
        let c2 = coefficient(&m, &x, &Number::from_i64(2));
        assert!(c2.number().expect("number").equals_i64(-5));
    }

    #[test]
    fn dense_roundtrip() {
        let x = MathStructure::symbolic("x");
        let mut m = crate::eval::parse_expression("2x^3 - 6x + 4").expect("parses");
        crate::eval::evaluate(&mut m);
        let d = to_dense(&m, &x).expect("dense");
        assert_eq!(d.len(), 4);
        assert!(d[0].equals_i64(4));
        assert!(d[1].equals_i64(-6));
        assert!(d[2].is_zero());
        assert!(d[3].equals_i64(2));
        let back = from_dense(&d, &x);
        let mut back2 = back.clone();
        back2.calculate_subtract(m, &EvaluationOptions::default());
        back2.calculatesub(&EvaluationOptions::default());
        assert!(back2.is_zero(), "roundtrip lost information: {back}");
    }

    #[test]
    fn dense_gcd_and_division() {
        // gcd(x^2-1, x^2+2x+1) = x+1
        let a = vec![
            Number::from_i64(-1),
            Number::from_i64(0),
            Number::from_i64(1),
        ];
        let b = vec![
            Number::from_i64(1),
            Number::from_i64(2),
            Number::from_i64(1),
        ];
        let g = dense_gcd(&a, &b).expect("gcd");
        assert_eq!(g.len(), 2);
        assert!(g[0].equals_i64(1) && g[1].equals_i64(1));
        let q = dense_divide(&a, &g).expect("exact division");
        assert!(q[0].equals_i64(-1) && q[1].equals_i64(1));
        // Non-exact division is rejected.
        assert!(dense_divide(&b, &vec![Number::from_i64(1), Number::from_i64(3)]).is_none());
    }

    #[test]
    fn rational_roots_of_a_cubic() {
        // x^3 - 5x^2 - 4x + 20 = (x-5)(x-2)(x+2)
        let c = vec![
            Number::from_i64(20),
            Number::from_i64(-4),
            Number::from_i64(-5),
            Number::from_i64(1),
        ];
        let mut roots: Vec<i64> = rational_roots(&c)
            .into_iter()
            .filter_map(|n| n.to_i64())
            .collect();
        roots.sort_unstable();
        assert_eq!(roots, vec![-2, 2, 5]);
    }

    #[test]
    fn find_x_var_prefers_x_then_y() {
        let m = crate::eval::parse_expression("3a + 2y + x").expect("parses");
        assert_eq!(find_x_var(&m).and_then(|s| s.symbol().map(String::from)), Some("x".into()));
        let m = crate::eval::parse_expression("3a + 2y").expect("parses");
        assert_eq!(find_x_var(&m).and_then(|s| s.symbol().map(String::from)), Some("y".into()));
        let m = crate::eval::parse_expression("3a + 2b").expect("parses");
        assert_eq!(find_x_var(&m).and_then(|s| s.symbol().map(String::from)), Some("a".into()));
    }

    #[test]
    fn integer_content_matches_reference() {
        let mut m = crate::eval::parse_expression("2x^3-4x^2-6x-8x^2").expect("parses");
        crate::eval::evaluate(&mut m);
        assert!(integer_content(&m).equals_i64(2));
        let mut m = crate::eval::parse_expression("2y^3-3y^2-6y-8y^2").expect("parses");
        crate::eval::evaluate(&mut m);
        assert!(integer_content(&m).equals_i64(1));
    }

    #[test]
    fn polynomial_quotient_divides_exactly() {
        let eo = EvaluationOptions::default();
        let x = MathStructure::symbolic("x");
        let mut num = crate::eval::parse_expression("x^3 - 5x^2 - 4x + 20").expect("parses");
        crate::eval::evaluate(&mut num);
        let mut den = crate::eval::parse_expression("x - 5").expect("parses");
        crate::eval::evaluate(&mut den);
        let q = polynomial_quotient(&num, &den, &x, &eo).expect("quotient");

        // Compare against the expected quotient directly. Multiplying back
        // by the divisor would not verify anything: `calculate_multiply`
        // does not distribute, so `q * den` stays a product and the
        // subtraction has no like terms to cancel.
        let printed = crate::print::print(&q, &crate::eval::batch_print_options());
        assert_eq!(printed, "x^2 - 4", "reference gives x^2 - 4");
    }

    #[test]
    fn sqrfree_splits_a_square() {
        // (x+1)^2 (x-1) = x^3 + x^2 - x - 1
        let c = vec![
            Number::from_i64(-1),
            Number::from_i64(-1),
            Number::from_i64(1),
            Number::from_i64(1),
        ];
        let parts = dense_sqrfree(&c);
        let mults: Vec<usize> = parts.iter().map(|(_, m)| *m).collect();
        assert!(mults.contains(&1) && mults.contains(&2), "got {mults:?}");
    }
}

/// `u^2 + 2uv + v^2` -> `(u + v)^2`, and `u^2 - 2uv + v^2` -> `(u - v)^2`.
///
/// The univariate factorizer works on a dense coefficient vector over one
/// variable, so it cannot see this when `u` and `v` involve different
/// unknowns. Matching is by structure: the two outer terms give `u` and `v`
/// as their square roots, and the candidate cross term `2uv` has to come out
/// *equal* to the remaining term, which is a strong enough check that no
/// ordinary trinomial slips through.
fn factor_perfect_square(
    m: &MathStructure,
    eo: &EvaluationOptions,
) -> Option<MathStructure> {
    let MathStructure::Addition(terms) = m else {
        return None;
    };
    if terms.len() != 3 {
        return None;
    }
    let evaluated = |mut x: MathStructure| {
        crate::eval::evaluate_calculated_with(&mut x, eo);
        crate::sort::sort(&mut x);
        x
    };
    for cross in 0..3 {
        let square_a = &terms[(cross + 1) % 3];
        let square_b = &terms[(cross + 2) % 3];
        let middle = &terms[cross];
        let half = MathStructure::Number(Number::from_ints(1, 2, 0));
        let u = evaluated(MathStructure::Power {
            base: Box::new(square_a.clone()),
            exponent: Box::new(half.clone()),
        });
        let v = evaluated(MathStructure::Power {
            base: Box::new(square_b.clone()),
            exponent: Box::new(half),
        });
        for negated in [false, true] {
            let signed_v = if negated {
                crate::absolute::negate_struct(&v)
            } else {
                v.clone()
            };
            let candidate = evaluated(MathStructure::Multiplication(vec![
                MathStructure::from(2),
                u.clone(),
                signed_v.clone(),
            ]));
            if !candidate.equals(middle) {
                continue;
            }
            let sum = evaluated(MathStructure::Addition(vec![u.clone(), signed_v]));
            let mut result = MathStructure::Power {
                base: Box::new(sum),
                exponent: Box::new(MathStructure::from(2)),
            };
            crate::sort::sort(&mut result);
            return Some(result);
        }
    }
    None
}

#[cfg(test)]
mod perfect_square_tests {
    use crate::session::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.evaluate_line("/set approximation exact").ok();
        s.evaluate_line("/set fr 2").ok();
        s
    }

    #[test]
    fn a_trinomial_in_two_unknowns_is_still_a_square() {
        let mut s = session();
        s.evaluate_line("/assume positive").unwrap();
        assert_eq!(
            s.evaluate_line("factor x + 2 * sqrt(xy) + y").unwrap(),
            "(sqrt(x) + sqrt(y))^2"
        );
        s.evaluate_line("/assume unknown").unwrap();
    }

    #[test]
    fn ordinary_trinomials_still_factor_the_ordinary_way() {
        let mut s = session();
        crate::assumptions::set_sign(crate::assumptions::Sign::Unknown);
        assert_eq!(s.evaluate_line("factor x^2+2x+1").unwrap(), "(x + 1)^2");
        assert_eq!(s.evaluate_line("factor x^2-2x+1").unwrap(), "(x - 1)^2");
        // Not a square: the cross term does not match, so this falls through
        // to the univariate factorizer.
        assert_eq!(
            s.evaluate_line("factor x^2+3x+2").unwrap(),
            "(x + 1)(x + 2)"
        );
    }
}
