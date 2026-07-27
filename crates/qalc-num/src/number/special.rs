//! Special functions — the `Number` methods libqalculate delegates to MPFR
//! (`mpfr_gamma`, `mpfr_digamma`, `mpfr_zeta`, `mpfr_erf`/`mpfr_erfc`,
//! `mpfr_eint`, `mpfr_li2`, `mpfr_ai`, `mpfr_jn`/`mpfr_yn`, `mpfr_gamma_inc`).
//!
//! astro-float ships no special functions, so these are hand-rolled series and
//! asymptotic expansions evaluated with guard bits on top of
//! [`context::bit_precision`].
//!
//! Conventions shared with [`super::transcendental`]:
//!   * mutate-and-return-`bool`; `false` means "not applicable / undefined".
//!   * exact shortcuts first (integer gamma → factorial, even zeta → π powers,
//!     Bernoulli numbers → exact rationals), float series only as a fallback.
//!
//! Interval handling: unlike the elementary functions there is no cheap
//! directed-rounding oracle, so the value is computed once at high internal
//! precision and then rounded outwards to the working precision (a one-ulp
//! interval).  Genuinely wide input intervals are mapped endpoint-wise and
//! hulled, which is exact for monotone stretches only — the same caveat
//! libqalculate reports as "lacks proper interval arithmetic support".

use super::{Number, RealValue};
use crate::context;
use crate::float::{bigfloat_from_bigint, bigfloat_from_ratio, bigfloat_is_integer};
use astro_float::{BigFloat, Consts, RoundingMode};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, Zero};
use std::cell::RefCell;

const RM: RoundingMode = RoundingMode::ToEven;

/// Guard bits added on top of the working precision for every series.
const GUARD: usize = 64;

/// Largest Bernoulli index computed exactly (the recurrence is O(n²) on
/// integers that grow like `n log n` digits).
const BERNOULLI_MAX: usize = 2048;

/// Largest |x| accepted by the (non-asymptotic) exponential/trigonometric
/// integral series.
const INTEGRAL_ARG_MAX: f64 = 1000.0;

// ----------------------------------------------------------------------
// Bernoulli numbers (exact rationals)
// ----------------------------------------------------------------------

thread_local! {
    static BERNOULLI_CACHE: RefCell<Vec<BigRational>> = const { RefCell::new(Vec::new()) };
}

/// `B_n` as an exact rational, in the `B_1 = −1/2` convention libqalculate
/// uses (`bernoulli_numbers.h`).  Memoized per thread; `None` once `n`
/// exceeds [`BERNOULLI_MAX`].
pub(crate) fn bernoulli_rational(n: usize) -> Option<BigRational> {
    if n > 1 && n % 2 == 1 {
        return Some(BigRational::zero());
    }
    if n > BERNOULLI_MAX {
        return None;
    }
    BERNOULLI_CACHE.with(|c| {
        let mut v = c.borrow_mut();
        if v.len() <= n {
            // Grow with slack so a sweep over consecutive indices rebuilds once.
            *v = build_bernoulli((n + n / 4 + 16).min(BERNOULLI_MAX + 1));
        }
        Some(v[n].clone())
    })
}

/// `B_0 … B_limit` from the Knuth–Buckholtz tangent-number recurrence —
/// integer-only (no gcds), unlike the binomial recurrence on rationals.
/// `B_{2k} = (−1)^{k−1}·2k·T_k / (4^k(4^k−1))`.
fn build_bernoulli(limit: usize) -> Vec<BigRational> {
    let m = limit / 2; // highest tangent number needed
    let mut t = vec![BigInt::zero(); m + 2];
    if m >= 1 {
        t[1] = BigInt::one();
    }
    for k in 2..=m {
        t[k] = BigInt::from(k - 1) * &t[k - 1];
    }
    for k in 2..=m {
        for j in k..=m {
            let a = BigInt::from(j - k) * &t[j - 1];
            t[j] = a + BigInt::from(j - k + 2) * &t[j];
        }
    }
    let mut out = Vec::with_capacity(limit + 1);
    out.push(BigRational::one()); // B_0
    if limit >= 1 {
        out.push(BigRational::new(BigInt::from(-1), BigInt::from(2))); // B_1
    }
    for i in 2..=limit {
        if i % 2 == 1 {
            out.push(BigRational::zero());
            continue;
        }
        let k = i / 2;
        let p4 = BigInt::one() << (2 * k); // 4^k
        let mut num = BigInt::from(2 * k) * &t[k];
        if k % 2 == 0 {
            num = -num; // (−1)^{k−1}
        }
        out.push(BigRational::new(num, &p4 * (&p4 - BigInt::one())));
    }
    out
}

// ----------------------------------------------------------------------
// BigFloat helpers
// ----------------------------------------------------------------------

fn f_i(i: i64, p: usize) -> BigFloat {
    BigFloat::from_i64(i, p)
}

/// The real part of `n` as a `[lower, upper]` pair of floats at precision `p`
/// — the same widening [`Number::apply_special`] applies to its argument, for
/// the two-argument functions that cannot go through it.
fn real_bounds(n: &Number, p: usize) -> (BigFloat, BigFloat) {
    match &n.value {
        RealValue::Rational(r) => {
            let x = bigfloat_from_ratio(r.numer(), r.denom(), p, RM);
            (x.clone(), x)
        }
        _ => (n.lower_bound_float(p), n.upper_bound_float(p)),
    }
}

/// `|term|` is below the working precision relative to `reference`.
fn negligible(term: &BigFloat, reference: &BigFloat, wp: usize) -> bool {
    if term.is_zero() {
        return true;
    }
    if term.is_nan() || term.is_inf() {
        return false;
    }
    let te = match term.exponent() {
        Some(e) => e as i64,
        None => return false,
    };
    let re = if reference.is_zero() {
        1
    } else {
        match reference.exponent() {
            Some(e) => e as i64,
            None => 1,
        }
    };
    te + (wp as i64) < re
}

/// Extra bits needed so that `exp(v)` keeps full relative precision when `v`
/// itself is large (absolute error in `v` becomes relative error in `exp v`).
fn exp_guard(v: &BigFloat) -> usize {
    match v.exponent() {
        Some(e) if e > 0 => e as usize,
        _ => 0,
    }
}

fn fact_bigint(n: u64) -> BigInt {
    let mut r = BigInt::one();
    for i in 2..=n {
        r *= BigInt::from(i);
    }
    r
}

// ----------------------------------------------------------------------
// log-gamma / gamma / digamma
// ----------------------------------------------------------------------

/// Argument the Stirling series is shifted up to.  Large enough that the
/// asymptotic series reaches `wp` bits well before its minimum term
/// (≈ e^{−2πz}), small enough that the shift product stays cheap.
fn stirling_target(wp: usize) -> i64 {
    ((wp as f64) * 0.25).ceil() as i64 + 8
}

fn stirling_terms(wp: usize) -> usize {
    ((wp as f64) * 0.2) as usize + 20
}

/// `ln Γ(z)` for `z` ≥ [`stirling_target`] via
/// `(z−½)ln z − z + ½ln 2π + Σ B_2k / (2k(2k−1) z^{2k−1})`.
fn stirling_ln_gamma(z: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let two = f_i(2, wp);
    let lnz = z.ln(wp, RM, cc);
    let half = f_i(1, wp).div(&two, wp, RM);
    let mut acc = z.sub(&half, wp, RM).mul(&lnz, wp, RM).sub(z, wp, RM);
    let ln2pi = cc.pi(wp, RM).mul(&two, wp, RM).ln(wp, RM, cc);
    acc = acc.add(&ln2pi.div(&two, wp, RM), wp, RM);

    let z2 = z.mul(z, wp, RM);
    let mut zpow = f_i(1, wp).div(z, wp, RM); // z^−(2k−1), k = 1
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut converged = false;
    for k in 1..=stirling_terms(wp) {
        let b = match bernoulli_rational(2 * k) {
            Some(b) => b,
            None => break, // ran out of Bernoulli numbers — not converged
        };
        let den = b.denom() * BigInt::from(2 * k) * BigInt::from(2 * k - 1);
        let coef = bigfloat_from_ratio(b.numer(), &den, wp, RM);
        let term = coef.mul(&zpow, wp, RM);
        let at = term.abs();
        if matches!(at.cmp(&prev), Some(c) if c > 0) {
            // Past the smallest term.  `z` is chosen so the minimum term is
            // far below 2^−wp, so this counts as converged.
            converged = true;
            break;
        }
        acc = acc.add(&term, wp, RM);
        if negligible(&at, &acc, wp) {
            converged = true;
            break;
        }
        prev = at;
        zpow = zpow.div(&z2, wp, RM);
    }
    if converged {
        acc
    } else {
        BigFloat::nan(None)
    }
}

/// `ln Γ(x)` for `x > 0`: shift the argument up into the asymptotic regime.
fn ln_gamma_pos(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let target = stirling_target(wp);
    let tf = f_i(target, wp);
    let one = f_i(1, wp);
    let mut z = x.clone();
    let mut prod = one.clone();
    let mut steps = 0i64;
    while matches!(z.cmp(&tf), Some(c) if c < 0) {
        prod = prod.mul(&z, wp, RM);
        z = z.add(&one, wp, RM);
        steps += 1;
        if steps > target + 4 {
            break;
        }
    }
    let lg = stirling_ln_gamma(&z, wp, cc);
    if steps == 0 {
        lg
    } else {
        lg.sub(&prod.ln(wp, RM, cc), wp, RM)
    }
}

/// `Γ(x)` for real `x`, `x` not a non-positive integer.
fn gamma_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if x.is_nan() || x.is_inf() {
        return BigFloat::nan(None);
    }
    if !x.is_negative() {
        // exp(ln Γ) loses one bit of relative precision per bit of integer
        // part in ln Γ — measure it and redo at the wider precision.
        let lg = ln_gamma_pos(x, wp, cc);
        let g = exp_guard(&lg);
        let lg = if g > 4 { ln_gamma_pos(x, wp + g, cc) } else { lg };
        return lg.exp(wp + g, RM, cc);
    }
    // Reflection: Γ(x) = π / (sin(πx)·Γ(1−x)).  sin has period 2π and tan
    // period π, so reduce x to its fractional part exactly first.
    let fl = x.floor();
    let frac = x.sub_full_prec(&fl);
    if frac.is_zero() {
        return BigFloat::nan(None); // pole
    }
    let pi = cc.pi(wp, RM);
    let mut s = pi.mul(&frac, wp, RM).sin(wp, RM, cc);
    if !bigfloat_is_even_integer(&fl) {
        s = s.neg();
    }
    let one = f_i(1, wp);
    let lg = ln_gamma_pos(&one.sub(x, wp, RM), wp, cc);
    let g = exp_guard(&lg);
    let lg = if g > 4 {
        ln_gamma_pos(&one.sub(x, wp + g, RM), wp + g, cc)
    } else {
        lg
    };
    let g1 = lg.exp(wp + g, RM, cc);
    pi.div(&s.mul(&g1, wp + g, RM), wp + g, RM)
}

fn bigfloat_is_even_integer(f: &BigFloat) -> bool {
    match crate::float::bigfloat_to_bigint_trunc(f) {
        Some(z) => (z % 2i32).is_zero(),
        None => false,
    }
}

/// `ψ(x)` for `x > 0`.
fn digamma_pos(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let target = stirling_target(wp);
    let tf = f_i(target, wp);
    let one = f_i(1, wp);
    let mut z = x.clone();
    let mut shift = f_i(0, wp);
    let mut steps = 0i64;
    while matches!(z.cmp(&tf), Some(c) if c < 0) {
        shift = shift.add(&one.div(&z, wp, RM), wp, RM);
        z = z.add(&one, wp, RM);
        steps += 1;
        if steps > target + 4 {
            break;
        }
    }
    // ψ(z) = ln z − 1/(2z) − Σ B_2k / (2k z^{2k})
    let mut acc = z.ln(wp, RM, cc);
    acc = acc.sub(&one.div(&z.mul(&f_i(2, wp), wp, RM), wp, RM), wp, RM);
    let z2 = z.mul(&z, wp, RM);
    let mut zpow = one.div(&z2, wp, RM);
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut converged = false;
    for k in 1..=stirling_terms(wp) {
        let b = match bernoulli_rational(2 * k) {
            Some(b) => b,
            None => break,
        };
        let den = b.denom() * BigInt::from(2 * k);
        let coef = bigfloat_from_ratio(b.numer(), &den, wp, RM);
        let term = coef.mul(&zpow, wp, RM);
        let at = term.abs();
        if matches!(at.cmp(&prev), Some(c) if c > 0) {
            converged = true;
            break;
        }
        acc = acc.sub(&term, wp, RM);
        if negligible(&at, &acc, wp) {
            converged = true;
            break;
        }
        prev = at;
        zpow = zpow.div(&z2, wp, RM);
    }
    if !converged {
        return BigFloat::nan(None);
    }
    acc.sub(&shift, wp, RM)
}

/// `ψ(x)` for real `x` that is not a non-positive integer.
fn digamma_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if x.is_nan() || x.is_inf() {
        return BigFloat::nan(None);
    }
    if !x.is_negative() && !x.is_zero() {
        return digamma_pos(x, wp, cc);
    }
    // Reflection: ψ(x) = ψ(1−x) − π·cot(πx); cot has period π.
    let fl = x.floor();
    let frac = x.sub_full_prec(&fl);
    if frac.is_zero() {
        return BigFloat::nan(None);
    }
    let pi = cc.pi(wp, RM);
    let t = pi.mul(&frac, wp, RM).tan(wp, RM, cc);
    let one = f_i(1, wp);
    let a = digamma_pos(&one.sub(x, wp, RM), wp, cc);
    a.sub(&pi.div(&t, wp, RM), wp, RM)
}

/// Euler–Mascheroni constant: γ = −ψ(1).
fn euler_gamma(wp: usize, cc: &mut Consts) -> BigFloat {
    digamma_pos(&f_i(1, wp), wp, cc).neg()
}

// ----------------------------------------------------------------------
// erf / erfc
// ----------------------------------------------------------------------

/// |x| beyond which the asymptotic expansion of erfc reaches `wp` bits.
fn erf_crossover(wp: usize) -> f64 {
    ((wp as f64) * std::f64::consts::LN_2).sqrt() + 1.0
}

/// `erf(x) = 2x·e^{−x²}/√π · Σ_k (2x²)^k / (2k+1)!!` — every term positive,
/// so there is no cancellation regardless of |x|.
fn erf_series(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let x2 = x.mul(x, wp, RM);
    let tx2 = x2.mul(&f_i(2, wp), wp, RM);
    let mut term = f_i(1, wp);
    let mut sum = term.clone();
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut k: u64 = 0;
    loop {
        k += 1;
        if k > 1_000_000 {
            return BigFloat::nan(None);
        }
        term = term.mul(&tx2, wp, RM).div(&f_i((2 * k + 1) as i64, wp), wp, RM);
        sum = sum.add(&term, wp, RM);
        let at = term.abs();
        // Require geometric decay ≤ ½ so the discarded tail is ≤ 2·term.
        if !matches!(at.mul(&f_i(2, wp), wp, RM).cmp(&prev), Some(c) if c > 0) && negligible(&at, &sum, wp) {
            break;
        }
        prev = at;
    }
    let sp = cc.pi(wp, RM).sqrt(wp, RM);
    let e = x2.neg().exp(wp, RM, cc);
    x.mul(&f_i(2, wp), wp, RM)
        .mul(&e, wp, RM)
        .mul(&sum, wp, RM)
        .div(&sp, wp, RM)
}

/// `erfc(x) = e^{−x²}/(x√π)·Σ_k (−1)^k (2k−1)!!/(2x²)^k` — valid for
/// x ≥ [`erf_crossover`], truncated at the smallest term.
fn erfc_asymptotic(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let x2 = x.mul(x, wp, RM);
    let two_x2 = x2.mul(&f_i(2, wp), wp, RM);
    let mut term = f_i(1, wp);
    let mut sum = term.clone();
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut k: u64 = 0;
    loop {
        k += 1;
        if k > 1_000_000 {
            break;
        }
        term = term
            .mul(&f_i((2 * k - 1) as i64, wp), wp, RM)
            .div(&two_x2, wp, RM)
            .neg();
        let at = term.abs();
        if matches!(at.cmp(&prev), Some(c) if c > 0) {
            break; // past the smallest term
        }
        sum = sum.add(&term, wp, RM);
        if negligible(&at, &sum, wp) {
            break;
        }
        prev = at;
    }
    let sp = cc.pi(wp, RM).sqrt(wp, RM);
    x2.neg()
        .exp(wp, RM, cc)
        .div(&x.mul(&sp, wp, RM), wp, RM)
        .mul(&sum, wp, RM)
}

/// Is `x` below the crossover where the asymptotic expansion takes over?
///
/// astro-float's `BigFloat::cmp` returns a *signed magnitude*, not −1/0/1:
/// when two values share an exponent it hands back the difference of their
/// mantissa words, so `8.cmp(14.53)` is a large negative number, not −1.
/// Testing `!= Some(-1)` therefore reads "not less than" as true for values
/// that genuinely are less, and erf/erfc take the asymptotic branch below the
/// crossover — which is where it does not converge. Only the *sign* of the
/// result is meaningful.
fn below_crossover(x: &BigFloat, wp: usize) -> bool {
    matches!(
        x.cmp(&BigFloat::from_f64(erf_crossover(wp), wp)),
        Some(c) if c < 0
    )
}

fn erf_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if x.is_nan() {
        return BigFloat::nan(None);
    }
    let ax = x.abs();
    if !below_crossover(&ax, wp) {
        // erfc is negligible next to 1 here — no cancellation.
        let r = f_i(1, wp).sub(&erfc_asymptotic(&ax, wp, cc), wp, RM);
        return if x.is_negative() { r.neg() } else { r };
    }
    erf_series(x, wp, cc)
}

fn erfc_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if x.is_nan() {
        return BigFloat::nan(None);
    }
    if !x.is_negative() && !below_crossover(x, wp) {
        return erfc_asymptotic(x, wp, cc);
    }
    // 1 − erf(x) cancels to ≈ e^{−x²} for positive x; x is below the
    // crossover, so x² < wp·ln2 and doubling the precision always suffices.
    let w2 = wp * 2 + 32;
    f_i(1, w2).sub(&erf_series(x, w2, cc), w2, RM)
}

/// `erfi(x) = 2x·e^{x²}/√π · Σ_k (2x²)^k / ((2k+1)·(2k)!!)`… implemented as
/// the imaginary-error-function series `2/√π · Σ x^{2k+1}/(k!(2k+1))`.
fn erfi_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let x2 = x.mul(x, wp, RM);
    let mut term = x.clone(); // x^{2k+1}/k!
    let mut sum = x.clone(); // divided by (2k+1) below
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut k: u64 = 0;
    loop {
        k += 1;
        if k > 1_000_000 {
            return BigFloat::nan(None);
        }
        term = term.mul(&x2, wp, RM).div(&f_i(k as i64, wp), wp, RM);
        let t = term.div(&f_i((2 * k + 1) as i64, wp), wp, RM);
        sum = sum.add(&t, wp, RM);
        let at = t.abs();
        if !matches!(at.mul(&f_i(2, wp), wp, RM).cmp(&prev), Some(c) if c > 0) && negligible(&at, &sum, wp) {
            break;
        }
        prev = at;
    }
    let sp = cc.pi(wp, RM).sqrt(wp, RM);
    sum.mul(&f_i(2, wp), wp, RM).div(&sp, wp, RM)
}

// ----------------------------------------------------------------------
// zeta
// ----------------------------------------------------------------------

/// Borwein's algorithm 2 for the Dirichlet eta function, then
/// `ζ(s) = η(s)/(1 − 2^{1−s})`.  Valid for real `s ≥ ½`.
fn zeta_borwein(s: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    // error ≈ (3+√8)^{−n} = 2^{−2.5431 n}
    let n = ((wp as f64) / 2.54).ceil() as usize + 5;
    let mut e = BigInt::one(); // e_0 = 1
    let mut acc = BigInt::one();
    let mut d: Vec<BigInt> = Vec::with_capacity(n + 1);
    d.push(acc.clone());
    for i in 1..=n {
        // e_i = e_{i−1}·4(n+i−1)(n−i+1) / ((2i)(2i−1))  — exact at every step
        let num = &e * BigInt::from(4u32) * BigInt::from(n + i - 1) * BigInt::from(n - i + 1);
        e = num / (BigInt::from(2 * i) * BigInt::from(2 * i - 1));
        acc += &e;
        d.push(acc.clone());
    }
    let dn = d[n].clone();
    let neg_s = s.neg();
    let mut sum = f_i(0, wp);
    for (k, dk) in d.iter().enumerate().take(n) {
        let w = &dn - dk;
        let wf = bigfloat_from_bigint(&w, wp, RM);
        let t = BigFloat::from_u64((k + 1) as u64, wp)
            .pow(&neg_s, wp, RM, cc)
            .mul(&wf, wp, RM);
        sum = if k % 2 == 0 {
            sum.add(&t, wp, RM)
        } else {
            sum.sub(&t, wp, RM)
        };
    }
    let eta = sum.div(&bigfloat_from_bigint(&dn, wp, RM), wp, RM);
    let one = f_i(1, wp);
    let denom = one.sub(
        &f_i(2, wp).pow(&one.sub(s, wp, RM), wp, RM, cc),
        wp,
        RM,
    );
    eta.div(&denom, wp, RM)
}

fn zeta_float(s: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if s.is_nan() || s.is_inf() {
        return BigFloat::nan(None);
    }
    // 1 − 2^{1−s} vanishes at s = 1: buy back the cancelled bits.
    let one = f_i(1, wp);
    let dist = s.sub(&one, wp, RM).abs();
    let extra = match dist.exponent() {
        Some(e) if e < 0 => ((-e) as usize).min(wp),
        _ => 0,
    };
    let wp = wp + extra;
    let half = BigFloat::from_f64(0.5, wp);
    if !matches!(s.cmp(&half), Some(c) if c < 0) {
        return zeta_borwein(s, wp, cc);
    }
    // Functional equation: ζ(s) = 2^s π^{s−1} sin(πs/2) Γ(1−s) ζ(1−s).
    let one = f_i(1, wp);
    let two = f_i(2, wp);
    let pi = cc.pi(wp, RM);
    let om = one.sub(s, wp, RM);
    let a = two.pow(s, wp, RM, cc);
    let b = pi.pow(&s.sub(&one, wp, RM), wp, RM, cc);
    let c = pi.mul(s, wp, RM).div(&two, wp, RM).sin(wp, RM, cc);
    let g = gamma_float(&om, wp, cc);
    let z = zeta_borwein(&om, wp, cc);
    a.mul(&b, wp, RM)
        .mul(&c, wp, RM)
        .mul(&g, wp, RM)
        .mul(&z, wp, RM)
}

/// `ζ(s, a) = Σ_{n≥0} (a+n)^{−s}` by Euler–Maclaurin, for real `s > 1`,
/// `a > 0`.
///
/// The C++ (`Number::zeta(const Number&)`, Number.cc:5634) sums the defining
/// series term by term and stops when the relative change falls under its
/// tolerance. That series converges like `N^{1−s}`: at `s` just above 1 it
/// needs more terms than there are atoms, and even at `s = 2` ten digits cost
/// 10¹⁰ of them. Euler–Maclaurin sums the first `N` terms directly and
/// replaces the tail with
/// `(a+N)^{1−s}/(s−1) + ½(a+N)^{−s} + Σ_k B_{2k}/(2k)! · (s)_{2k−1} ·
/// (a+N)^{−s−2k+1}`,
/// which reaches the same answer in `N ≈ wp/2` terms and a handful of
/// Bernoulli corrections.
fn hurwitz_zeta_float(s: &BigFloat, a: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if s.is_nan() || a.is_nan() || s.is_inf() || a.is_inf() {
        return BigFloat::nan(None);
    }
    let one = f_i(1, wp);
    let two = f_i(2, wp);
    let neg_s = s.neg();
    // Enough head terms that the Bernoulli corrections fall off like
    // (2k)!/(2πN)^{2k}; N also has to clear |s| for the rising factorial not
    // to outrun the powers of (a+N).
    let n_terms: usize = (wp / 2 + 24).max(32);
    let mut sum = f_i(0, wp);
    for j in 0..n_terms {
        let x = a.add(&f_i(j as i64, wp), wp, RM);
        if x.is_zero() || x.is_negative() {
            return BigFloat::nan(None); // a pole of the summand
        }
        sum = sum.add(&x.pow(&neg_s, wp, RM, cc), wp, RM);
    }
    let b = a.add(&f_i(n_terms as i64, wp), wp, RM); // a + N
    let bpow = b.pow(&neg_s, wp, RM, cc); // (a+N)^{−s}
    // (a+N)^{1−s}/(s−1), written as (a+N)·(a+N)^{−s}/(s−1).
    sum = sum.add(
        &b.mul(&bpow, wp, RM).div(&s.sub(&one, wp, RM), wp, RM),
        wp,
        RM,
    );
    sum = sum.add(&bpow.div(&two, wp, RM), wp, RM);

    let b2 = b.mul(&b, wp, RM);
    let mut rising = s.clone(); // (s)_{2k−1}, k = 1
    let mut bp = bpow.div(&b, wp, RM); // (a+N)^{−s−2k+1}, k = 1
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    for k in 1..=(wp / 4 + 8) {
        let bern = match bernoulli_rational(2 * k) {
            Some(v) => v,
            None => break,
        };
        let den = bern.denom() * fact_bigint(2 * k as u64);
        let coef = bigfloat_from_ratio(bern.numer(), &den, wp, RM);
        let term = coef.mul(&rising, wp, RM).mul(&bp, wp, RM);
        let at = term.abs();
        // The expansion is asymptotic: past its smallest term it diverges.
        if matches!(at.cmp(&prev), Some(c) if c > 0) {
            break;
        }
        sum = sum.add(&term, wp, RM);
        if negligible(&at, &sum, wp) {
            break;
        }
        prev = at;
        let k2 = f_i(2 * k as i64, wp);
        rising = rising
            .mul(&s.add(&k2.sub(&one, wp, RM), wp, RM), wp, RM)
            .mul(&s.add(&k2, wp, RM), wp, RM);
        bp = bp.div(&b2, wp, RM);
    }
    sum
}

// ----------------------------------------------------------------------
// exponential / trigonometric integrals
// ----------------------------------------------------------------------

/// `Ei(x) = γ + ln|x| + Σ_{k≥1} x^k/(k·k!)`.
fn ei_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let mut tk = f_i(1, wp); // x^k/k!
    let mut sum = f_i(0, wp);
    let mut maxterm = f_i(0, wp);
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut k: u64 = 0;
    loop {
        k += 1;
        if k > 1_000_000 {
            return BigFloat::nan(None);
        }
        tk = tk.mul(x, wp, RM).div(&f_i(k as i64, wp), wp, RM);
        let t = tk.div(&f_i(k as i64, wp), wp, RM);
        sum = sum.add(&t, wp, RM);
        let at = t.abs();
        if matches!(at.cmp(&maxterm), Some(c) if c > 0) {
            maxterm = at.clone();
        }
        if !matches!(at.mul(&f_i(2, wp), wp, RM).cmp(&prev), Some(c) if c > 0) && negligible(&at, &maxterm, wp) {
            break;
        }
        prev = at;
    }
    let g = euler_gamma(wp, cc);
    let l = x.abs().ln(wp, RM, cc);
    g.add(&l, wp, RM).add(&sum, wp, RM)
}

/// `Si(x) = Σ_{k≥0} (−1)^k x^{2k+1}/((2k+1)(2k+1)!)`.
fn si_float(x: &BigFloat, wp: usize, _cc: &mut Consts) -> BigFloat {
    let mx2 = x.mul(x, wp, RM).neg();
    let mut term = x.clone();
    let mut sum = x.clone();
    let mut maxterm = term.abs();
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut k: u64 = 0;
    loop {
        k += 1;
        if k > 1_000_000 {
            return BigFloat::nan(None);
        }
        // a_k/a_{k−1} = −x²(2k−1) / ((2k+1)²·2k)
        let num = f_i((2 * k - 1) as i64, wp);
        let den = f_i(((2 * k + 1) * (2 * k + 1) * 2 * k) as i64, wp);
        term = term.mul(&mx2, wp, RM).mul(&num, wp, RM).div(&den, wp, RM);
        sum = sum.add(&term, wp, RM);
        let at = term.abs();
        if matches!(at.cmp(&maxterm), Some(c) if c > 0) {
            maxterm = at.clone();
        }
        if !matches!(at.mul(&f_i(2, wp), wp, RM).cmp(&prev), Some(c) if c > 0) && negligible(&at, &maxterm, wp) {
            break;
        }
        prev = at;
    }
    sum
}

/// `Ci(x) = γ + ln x + Σ_{k≥1} (−1)^k x^{2k}/((2k)(2k)!)` for x > 0.
fn ci_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    let mx2 = x.mul(x, wp, RM).neg();
    let mut term = mx2.div(&f_i(4, wp), wp, RM); // k = 1
    let mut sum = term.clone();
    let mut maxterm = term.abs();
    let mut prev = BigFloat::from_f64(f64::INFINITY, wp);
    let mut k: u64 = 1;
    loop {
        k += 1;
        if k > 1_000_000 {
            return BigFloat::nan(None);
        }
        // b_k/b_{k−1} = −x²(2k−2) / (2k·2k·(2k−1))
        let num = f_i((2 * k - 2) as i64, wp);
        let den = f_i((2 * k * 2 * k * (2 * k - 1)) as i64, wp);
        term = term.mul(&mx2, wp, RM).mul(&num, wp, RM).div(&den, wp, RM);
        sum = sum.add(&term, wp, RM);
        let at = term.abs();
        if matches!(at.cmp(&maxterm), Some(c) if c > 0) {
            maxterm = at.clone();
        }
        if !matches!(at.mul(&f_i(2, wp), wp, RM).cmp(&prev), Some(c) if c > 0) && negligible(&at, &maxterm, wp) {
            break;
        }
        prev = at;
    }
    let g = euler_gamma(wp, cc);
    let l = x.abs().ln(wp, RM, cc);
    g.add(&l, wp, RM).add(&sum, wp, RM)
}

// ----------------------------------------------------------------------
// Number-level plumbing
// ----------------------------------------------------------------------

/// `c·e^(∓f²)/sqrt(pi)` — the derivative shared by the erf family, with
/// `c` the leading coefficient and `negate_exponent` selecting `erf`/`erfc`'s
/// `e^(−f²)` over `erfi`'s `e^(f²)`.
fn gaussian_derivative(x: &Number, c: i64, negate_exponent: bool) -> Option<Number> {
    let mut d = x.clone();
    if !d.square() || (negate_exponent && !d.negate()) || !d.exp() {
        return None;
    }
    let mut root_pi = Number::new();
    root_pi.pi();
    (root_pi.sqrt() && d.multiply(&Number::from_i64(c)) && d.divide(&root_pi)).then_some(d)
}

/// The abscissa of Γ's minimum on the positive reals, 1.46163214496836234…,
/// and the value there, 0.88560319441088870… Both are the reference's own
/// literals (Number.cc:5755, :5765) — Γ turns around here, so an interval
/// bracketing this point has its lower bound *at* it rather than at either
/// end.
const GAMMA_MIN_X: &str = "1.46163214496836234126265954232572132846819620400644635129598840859878644035380181024307499273372559";
const GAMMA_MIN_Y: &str = "0.88560319441088870027881590058258873320795153366990344887120016587513622741739634666479828021420359";

fn gamma_min_x(p: usize) -> BigFloat {
    context::with_consts(|cc| {
        BigFloat::parse(GAMMA_MIN_X, astro_float::Radix::Dec, p, RoundingMode::None, cc)
    })
}

fn gamma_min_y(p: usize) -> BigFloat {
    context::with_consts(|cc| {
        BigFloat::parse(GAMMA_MIN_Y, astro_float::Radix::Dec, p, RoundingMode::Down, cc)
    })
}

impl Number {
    /// Evaluate a real special function.  `wp` is the internal working
    /// precision; the value is rounded outwards to `bit_precision()` at the
    /// end (see the module docs on interval fidelity).
    fn apply_special<F>(&mut self, wp: usize, f: F) -> bool
    where
        F: Fn(&BigFloat, usize, &mut Consts) -> BigFloat,
    {
        let p = context::bit_precision();
        let (al, au) = match &self.value {
            RealValue::Rational(r) => {
                let x = bigfloat_from_ratio(r.numer(), r.denom(), wp, RM);
                (x.clone(), x)
            }
            _ => (self.lower_bound_float(wp), self.upper_bound_float(wp)),
        };
        let same = al == au;
        let (mut lo, mut hi) = context::with_consts(|cc| {
            let a = f(&al, wp, cc);
            let b = if same { a.clone() } else { f(&au, wp, cc) };
            (a, b)
        });
        if lo.is_nan() || hi.is_nan() {
            return false;
        }
        if matches!(lo.cmp(&hi), Some(c) if c > 0) {
            std::mem::swap(&mut lo, &mut hi);
        }
        let (lower, upper) = if context::create_interval() {
            if lo.set_precision(p, RoundingMode::Down).is_err()
                || hi.set_precision(p, RoundingMode::Up).is_err()
            {
                return false;
            }
            (lo, hi)
        } else {
            if lo.set_precision(p, RM).is_err() {
                return false;
            }
            (lo.clone(), lo)
        };
        self.value = RealValue::Float { lower, upper };
        self.imag = None;
        self.approx = true;
        self.test_float_result(true)
    }

    /// Replace with `n`, keeping our approximation flags.
    fn take_keeping_flags(&mut self, n: Number) {
        let keep = (self.approx, self.precision);
        *self = n;
        self.approx |= keep.0;
        if keep.1 >= 0 && (self.precision < 0 || keep.1 < self.precision) {
            self.precision = keep.1;
        }
    }

    /// True if any part of the (real) value sits on a non-positive integer —
    /// a pole of gamma/digamma.
    fn touches_nonpositive_integer(&self) -> bool {
        match &self.value {
            RealValue::Rational(r) => r.denom().is_one() && !r.is_positive(),
            RealValue::Float { lower, upper } => {
                for f in [lower, upper] {
                    if bigfloat_is_integer(f) && (f.is_zero() || f.is_negative()) {
                        return true;
                    }
                }
                // An interval straddling a negative integer.
                if lower.is_negative() && lower != upper {
                    let (fl, fu) = (lower.floor(), upper.floor());
                    if fl != fu {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------
    // gamma
    // ------------------------------------------------------------------

    /// `gamma()` — Γ(x).  Exact for positive integers (factorial) and
    /// half-integers (rational multiple of √π); Stirling's asymptotic series
    /// with a shifted argument otherwise, plus the reflection formula for
    /// negative arguments.  Poles (0, −1, −2, …) return false.
    pub fn gamma(&mut self) -> bool {
        // `gamma(f)' = f'·gamma(f)·digamma(f)`
        // (MathStructure-differentiate.cc:286).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    let mut psi = x.clone();
                    (d.gamma() && psi.digamma() && d.multiply(&psi)).then_some(d)
                },
                Number::gamma_impl,
            );
        }
        self.gamma_impl()
    }

    fn gamma_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return false; // TODO(port): complex gamma (Lanczos on the complex plane)
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            return false;
        }
        // The exact closed forms below are the reference's, but the reference
        // only *reaches* them for an argument below 1000
        // (`GammaFunction::calculate`, BuiltinFunctions-special.cc:155:
        // `isRational() && (approximation == EXACT || (TRY_EXACT &&
        // isLessThan(1000)))`, and TRY_EXACT is the default). Past that it
        // hands the whole thing to `Number::gamma`, which is MPFR throughout.
        //
        // Without the bound `gamma(1000000)` builds 999999! exactly — an
        // 18-million-bit integer — and takes over 150s where the reference
        // takes 0.35s. The printed answer is identical either way, because a
        // 5.5-million-digit integer is still rounded to ten significant digits
        // on the way out; all the exactness buys is the arithmetic nobody
        // asked for.
        //
        // This lives here rather than in the function wrapper because the port
        // routes `gamma` straight to `Number::gamma` (builtins.rs:338) and
        // never gives `Number` the approximation mode. The bound is therefore
        // applied unconditionally, which matches the reference in its default
        // mode and is stricter than the reference under `approximation=EXACT`.
        let exact_form_in_range =
            matches!(&self.value, RealValue::Rational(r) if *r < BigRational::from_integer(BigInt::from(1000)));
        if let RealValue::Rational(r) = &self.value {
            if r.denom().is_one() && exact_form_in_range {
                if !r.is_positive() {
                    return false; // pole
                }
                let mut n = Number::from_bigint(r.numer() - 1);
                if !n.factorial() {
                    return false;
                }
                self.take_keeping_flags(n);
                return true;
            }
            if *r.denom() == BigInt::from(2) && exact_form_in_range {
                if let Some(c) = half_integer_gamma_coeff(r) {
                    let mut sp = Number::new();
                    sp.pi();
                    if !sp.sqrt() || !sp.multiply(&Number::from_rational(c)) {
                        return false;
                    }
                    let keep = (self.approx, self.precision);
                    *self = sp;
                    if keep.1 >= 0 && (self.precision < 0 || keep.1 < self.precision) {
                        self.precision = keep.1;
                    }
                    return true;
                }
            }
        }
        if self.touches_nonpositive_integer() {
            return false;
        }
        let wp = context::bit_precision() + GUARD;
        // Γ is *not* monotone on the positive reals: it falls to a minimum of
        // 0.8856031944… at x = 1.46163214… and climbs again. Evaluating the
        // two endpoints alone therefore misses the minimum whenever the
        // interval brackets it, and returns an enclosure that does not contain
        // values the function actually takes — `gamma([1:2])` came out `[1:1]`
        // where the true range is [0.8856031944:1]. The reference pins the
        // turning point as a literal and does the same (Number.cc:5750).
        let straddles_min = context::create_interval()
            && self.is_interval(true)
            && matches!(&self.value, RealValue::Float { lower, upper }
                if lower.is_positive()
                    && matches!(lower.cmp(&gamma_min_x(wp)), Some(c) if c < 0)
                    && matches!(upper.cmp(&gamma_min_x(wp)), Some(c) if c >= 0));
        if !self.apply_special(wp, gamma_float) {
            return false;
        }
        if straddles_min {
            if let RealValue::Float { lower, upper } = &mut self.value {
                let p = context::bit_precision();
                // `apply_special` ordered the two endpoint values; the larger
                // is the true upper bound, and the minimum replaces the lower.
                if matches!(lower.cmp(upper), Some(c) if c > 0) {
                    std::mem::swap(lower, upper);
                }
                *lower = gamma_min_y(p);
            }
        }
        true
    }

    /// `digamma()` — ψ(x) = Γ′(x)/Γ(x).
    ///
    /// Not in `function_differentiable`'s list
    /// (MathStructure-differentiate.cc:30), so the reference propagates an
    /// uncertainty through it with plain interval arithmetic rather than the
    /// variance formula: `digamma(3+/-0.1)` is `0.922±0.040`, the enclosure of
    /// ψ over [2.9, 3.1], not `|ψ'(3)|·0.1 = 0.0395`.
    pub fn digamma(&mut self) -> bool {
        self.resolve_variance_uncertainty();
        if !self.is_real() {
            return false;
        }
        if self.touches_nonpositive_integer() {
            return false;
        }
        let wp = context::bit_precision() + GUARD;
        self.apply_special(wp, digamma_float)
    }

    // ------------------------------------------------------------------
    // erf family
    // ------------------------------------------------------------------

    /// `erf()` — the error function.
    pub fn erf(&mut self) -> bool {
        // `erf(f)' = f'·2/(e^(f²)·sqrt(pi))`
        // (MathStructure-differentiate.cc:336).
        if self.unc.is_some() {
            return self.uncertain_unary(|x| gaussian_derivative(x, 2, true), Number::erf_impl);
        }
        self.erf_impl()
    }

    fn erf_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.erf_complex();
        }
        if self.is_plus_infinity() {
            self.take_keeping_flags(Number::from_i64(1));
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            self.take_keeping_flags(Number::from_i64(-1));
            self.approx = true;
            return true;
        }
        if self.is_zero() {
            return true;
        }
        let wp = context::bit_precision() + GUARD;
        self.apply_special(wp, erf_float)
    }

    /// `erfc()` — the complementary error function, computed directly (not as
    /// `1 − erf`) once cancellation would bite.
    pub fn erfc(&mut self) -> bool {
        // `erfc(f)' = f'·−2/(e^(f²)·sqrt(pi))`
        // (MathStructure-differentiate.cc:361).
        if self.unc.is_some() {
            return self.uncertain_unary(|x| gaussian_derivative(x, -2, true), Number::erfc_impl);
        }
        self.erfc_impl()
    }

    fn erfc_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            // `erfc(z) = 1 − erf(z)` (Number.cc:6123). Unlike the real case
            // there is no cancellation to dodge: the complex `erf` is a Taylor
            // series that already loses its digits to the alternating signs and
            // buys them back with doubled precision.
            return self.erf_complex() && self.negate() && self.add(&Number::from_i64(1));
        }
        if self.is_plus_infinity() {
            self.take_keeping_flags(Number::new());
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            self.take_keeping_flags(Number::from_i64(2));
            self.approx = true;
            return true;
        }
        if self.is_zero() {
            self.take_keeping_flags(Number::from_i64(1));
            return true;
        }
        let wp = context::bit_precision() + GUARD;
        self.apply_special(wp, erfc_float)
    }

    /// `erfi()` — the imaginary error function, `erfi(x) = −i·erf(ix)`.
    pub fn erfi(&mut self) -> bool {
        // `erfi(f)' = f'·2·e^(f²)/sqrt(pi)`
        // (MathStructure-differentiate.cc:349).
        if self.unc.is_some() {
            return self.uncertain_unary(|x| gaussian_derivative(x, 2, false), Number::erfi_impl);
        }
        self.erfi_impl()
    }

    fn erfi_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            if !self.has_real_part() {
                // `erfi(x·i) = erf(x)·i` (Number.cc:6043) — worth taking
                // separately from the general identity below, which would
                // leave a rounding-noise real part on a value that has none.
                let mut re = self.imaginary_part();
                if !re.erf() {
                    return false;
                }
                let mut out = Number::new();
                out.set_imaginary_part(&re);
                self.take_keeping_flags(out);
                return true;
            }
            // `erfi(z) = −i·erf(i·z)` (Number.cc:6039).
            let i = imaginary_unit();
            let mut minus_i = i.clone();
            if !minus_i.negate() {
                return false;
            }
            return self.multiply(&i) && self.erf() && self.multiply(&minus_i);
        }
        if self.is_plus_infinity() || self.is_minus_infinity() {
            return true;
        }
        if self.is_zero() {
            return true;
        }
        let m = self.float_value().abs();
        if !m.is_finite() || m > INTEGRAL_ARG_MAX {
            return false; // TODO(port): asymptotic expansion for large |x|
        }
        let wp = context::bit_precision() + GUARD + ((m * m) * 1.4427).ceil() as usize;
        self.apply_special(wp, erfi_float)
    }

    /// `erf(z)` for complex `z` (Number.cc:5893).
    ///
    /// `erf(z) = 2/√π · Σ_{k≥0} (−1)^k z^{2k+1}/((2k+1)·k!)`, the same Taylor
    /// series the C++ falls back to because MPFR's `mpfr_erf` is real-only.
    /// The terms alternate and peak near `e^{|z|²}`, so the sum is taken at
    /// double the working precision — again as the C++ does
    /// (`setPrecision(PRECISION * 2 + 20)`).
    fn erf_complex(&mut self) -> bool {
        if self.includes_infinity() {
            return false;
        }
        // The series below runs in *point* mode (`with_series_precision`
        // turns interval arithmetic off so the alternating cancellation does
        // not blow the enclosure up), which means an interval argument would
        // be silently evaluated at one corner and returned as if it were the
        // answer for the whole box. That is an unsound enclosure, not a loose
        // one, so refuse instead.
        if self.is_interval(true)
            || self.imag.as_ref().is_some_and(|i| i.is_interval(true))
        {
            return false;
        }
        if !self.has_real_part() {
            // `erf(x·i) = erfi(x)·i` (Number.cc:6014).
            let mut im = self.imaginary_part();
            if !im.erfi() {
                return false;
            }
            let mut out = Number::new();
            out.set_imaginary_part(&im);
            self.take_keeping_flags(out);
            return true;
        }
        // The largest term is ≈ e^{|z|²}; everything above the answer's own
        // size is cancellation, and has to be bought back up front.
        let m = complex_magnitude(self);
        if !m.is_finite() || m * m > INTEGRAL_ARG_MAX {
            return false; // TODO(port): asymptotic expansion for large |z|
        }
        let result = with_series_precision(m * m, || self.erf_complex_series());
        match result {
            Some(v) => {
                *self = v;
                self.approx = true;
                self.test_float_result(true);
                true
            }
            None => false,
        }
    }

    fn erf_complex_series(&self) -> Option<Number> {
        let z = self.clone();
        let mut z2 = z.clone();
        if !z2.square() || !z2.negate() {
            return None;
        }
        let mut term = z.clone(); // (−1)^k z^{2k+1}/k!
        let mut sum = z.clone(); // …divided by (2k+1) as it is added
        let tolerance = series_tolerance()?;
        for k in 1..MAX_EXPINT_TERMS {
            if !term.multiply(&z2) || !term.divide_i64(k) {
                return None;
            }
            let mut t = term.clone();
            if !t.divide_i64(2 * k + 1) || !sum.add(&t) {
                return None;
            }
            if sum.is_infinite(false) {
                return None;
            }
            if term_is_negligible(&t, &sum, &tolerance)? {
                let mut root_pi = Number::new();
                root_pi.pi();
                if !root_pi.sqrt() || !sum.multiply_i64(2) || !sum.divide(&root_pi) {
                    return None;
                }
                return Some(sum);
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // zeta
    // ------------------------------------------------------------------

    /// `zeta()` — the Riemann zeta function for real arguments.
    ///
    /// Exact for `s = 0`, non-positive integers (`−B_{n+1}/(n+1)`) and even
    /// positive integers (rational multiple of `π^n`); Borwein's alternating
    /// series for `s ≥ ½` and the functional equation below that.
    ///
    /// Like [`Number::digamma`], zeta is missing from
    /// `function_differentiable` (MathStructure-differentiate.cc:30), so an
    /// uncertain argument is widened to an interval and enclosed rather than
    /// pushed through ζ′. That is why `zeta(2+/-0.1)` is `1.655±0.095` — the
    /// midpoint of `[ζ(2.1), ζ(1.9)]`, which is *not* ζ(2) — and not
    /// `1.645±0.094`.
    pub fn zeta(&mut self) -> bool {
        self.resolve_variance_uncertainty();
        if self.has_imaginary_part() {
            return false; // TODO(port): complex zeta
        }
        if self.is_plus_infinity() {
            self.take_keeping_flags(Number::from_i64(1));
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            return false;
        }
        let one = Number::from_i64(1);
        if !self.is_greater_than(&one) && !self.is_less_than(&one) {
            return false; // ζ(1) is undefined (and intervals straddling it)
        }
        if let RealValue::Rational(r) = &self.value {
            if r.denom().is_one() {
                let n = r.numer().clone();
                if n.is_zero() {
                    self.take_keeping_flags(Number::from_ints(-1, 2, 0));
                    return true;
                }
                if n.is_negative() {
                    // ζ(−m) = −B_{m+1}/(m+1)
                    let m = (-&n).to_string().parse::<usize>().unwrap_or(usize::MAX);
                    let b = match bernoulli_rational(m + 1) {
                        Some(b) => b,
                        None => return false,
                    };
                    let v = -b / BigRational::from_integer(BigInt::from(m + 1));
                    self.take_keeping_flags(Number::from_rational(v));
                    return true;
                }
                if (&n % 2i32).is_zero() {
                    // ζ(2k) = |B_2k|·(2π)^{2k} / (2·(2k)!)
                    let m = n.to_string().parse::<usize>().unwrap_or(usize::MAX);
                    if let Some(b) = bernoulli_rational(m) {
                        let num = b.numer().magnitude().clone();
                        let den = b.denom() * fact_bigint(m as u64) * BigInt::from(2);
                        let coeff = BigRational::new(
                            BigInt::from(num) * (BigInt::one() << m),
                            den,
                        );
                        let mut pi = Number::new();
                        pi.pi();
                        if !pi.raise(&Number::from_i64(m as i64), false)
                            || !pi.multiply(&Number::from_rational(coeff))
                        {
                            return false;
                        }
                        let keep = (self.approx, self.precision);
                        *self = pi;
                        if keep.1 >= 0 && (self.precision < 0 || keep.1 < self.precision) {
                            self.precision = keep.1;
                        }
                        return true;
                    }
                }
            }
        }
        let wp = context::bit_precision() + GUARD;
        self.apply_special(wp, zeta_float)
    }

    /// `zeta(o)` — the Hurwitz zeta `ζ(s, a)`, with `s = self` and `a = o`
    /// (`Number::zeta(const Number&)`, Number.cc:5634).
    ///
    /// The reference's domain, and the reason it is this narrow: the series
    /// `Σ (a+n)^{−s}` only converges for `s > 1`, and only has real terms for
    /// `a > 0`. `a = 1` is the Riemann zeta and is handed straight to it.
    pub fn hurwitz_zeta(&mut self, o: &Number) -> bool {
        self.resolve_variance_uncertainty();
        if o.is_one() {
            return self.zeta();
        }
        if o.includes_infinity() || !o.is_positive() {
            return false;
        }
        if !self.is_greater_than(&Number::from_i64(1)) {
            // The defining series diverges here, but the reference still
            // answers when `a` is a small integer: `ZetaFunction::calculate`
            // (BuiltinFunctions-special.cc:133) rewrites `ζ(s, a)` as the
            // Riemann zeta less its first `a−1` terms, which is defined
            // wherever `ζ(s)` is. That is how `zeta(0.5, 2)` reaches
            // −2.460354509 rather than declining. The bounds are the C++'s.
            return self.zeta_by_shifting_riemann(o);
        }
        if self.is_plus_infinity() {
            self.take_keeping_flags(Number::from_i64(1));
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            return false;
        }
        let wp = context::bit_precision() + GUARD;
        let (s_lo, s_hi) = real_bounds(self, wp);
        let (a_lo, a_hi) = real_bounds(o, wp);
        let same = s_lo == s_hi && a_lo == a_hi;
        let (mut lo, mut hi) = context::with_consts(|cc| {
            let x = hurwitz_zeta_float(&s_lo, &a_lo, wp, cc);
            let y = if same {
                x.clone()
            } else {
                hurwitz_zeta_float(&s_hi, &a_hi, wp, cc)
            };
            (x, y)
        });
        if lo.is_nan() || hi.is_nan() {
            return false;
        }
        if matches!(lo.cmp(&hi), Some(c) if c > 0) {
            std::mem::swap(&mut lo, &mut hi);
        }
        let keep = (self.approx, self.precision);
        if !self.set_from_float_bounds(lo, hi) {
            return false;
        }
        self.approx = true;
        if keep.1 >= 0 && (self.precision < 0 || keep.1 < self.precision) {
            self.precision = keep.1;
        }
        true
    }

    /// `ζ(s, a) = ζ(s) − Σ_{n=1}^{a−1} n^{−s}` for an integer `a ≥ 2`.
    ///
    /// Only reached below the defining series' domain; `false` outside the
    /// window the reference allows itself (`a ≤ 50`, `|s| ≤ 10`), where
    /// subtracting `a` large terms from `ζ(s)` would cost more digits than it
    /// is worth.
    fn zeta_by_shifting_riemann(&mut self, o: &Number) -> bool {
        if !o.is_integer() {
            return false;
        }
        let Some(a) = o.to_i64() else {
            return false;
        };
        if !(2..=50).contains(&a) {
            return false;
        }
        // The ±10 window is the one the C++ puts on its general rewrite. A
        // negative integer `s` is exempt because the reference reaches it by a
        // different, unbounded route (Bernoulli polynomials,
        // BuiltinFunctions-special.cc:121) that agrees with this one:
        // `zeta(-11, 2)` is −0.9789072039 either way.
        let negative_integer_order = self.is_integer() && self.is_negative();
        if !negative_integer_order
            && (self.is_greater_than(&Number::from_i64(10))
                || self.is_less_than(&Number::from_i64(-10)))
        {
            return false;
        }
        let mut neg_s = self.clone();
        if !neg_s.negate() {
            return false;
        }
        let mut acc = self.clone();
        if !acc.zeta() {
            return false;
        }
        for n in 1..a {
            let mut term = Number::from_i64(n);
            if !term.raise(&neg_s, false) || !acc.subtract(&term) {
                return false;
            }
        }
        *self = acc;
        true
    }

    /// Store `[lo, hi]` as this number's value, narrowed to the working
    /// precision the way [`Number::apply_special`] does (outward in interval
    /// mode, to-nearest otherwise).
    fn set_from_float_bounds(&mut self, mut lo: BigFloat, mut hi: BigFloat) -> bool {
        let p = context::bit_precision();
        let (lower, upper) = if context::create_interval() {
            if lo.set_precision(p, RoundingMode::Down).is_err()
                || hi.set_precision(p, RoundingMode::Up).is_err()
            {
                return false;
            }
            (lo, hi)
        } else {
            if lo.set_precision(p, RM).is_err() {
                return false;
            }
            (lo.clone(), lo)
        };
        self.value = RealValue::Float { lower, upper };
        self.imag = None;
        self.approx = true;
        self.test_float_result(true)
    }

    /// `bernoulli()` — replace an integer `n` with the exact rational `B_n`.
    pub fn bernoulli(&mut self) -> bool {
        if !self.is_integer() || self.is_negative() {
            return false;
        }
        let n = match self.to_bigint().and_then(|z| z.to_string().parse::<usize>().ok()) {
            Some(n) => n,
            None => return false,
        };
        match bernoulli_rational(n) {
            Some(b) => {
                self.take_keeping_flags(Number::from_rational(b));
                true
            }
            // TODO(port): B_n for n > BERNOULLI_MAX (libqalculate falls back
            // to −n·ζ(1−n) there).
            None => false,
        }
    }

    // ------------------------------------------------------------------
    // exponential / trigonometric integrals
    // ------------------------------------------------------------------

    /// Guard bits needed by the exponential/trigonometric integral series,
    /// whose largest term is ≈ e^{|x|}.  `None` when |x| is out of range.
    fn integral_guard(&self) -> Option<usize> {
        let m = self.float_value().abs();
        if !m.is_finite() || m > INTEGRAL_ARG_MAX {
            return None;
        }
        Some((m * 1.4427).ceil() as usize + 16)
    }

    /// `Ei(z)` for complex `z` (Number.cc:8868).
    ///
    /// `Ei(z) = γ + (ln(z) − ln(1/z))/2 + Σ_{k≥1} z^k / (k·k!)`. MPFR's
    /// `mpfr_eint` is real-only, so the C++ falls back to this series too.
    ///
    /// The halved difference of the two logarithms is not a roundabout way of
    /// writing `ln(z)`: it is what puts the branch cut where `Ei` needs it, on
    /// the negative real axis approached from *both* sides, so that
    /// `Ei(x + 0i)` and `Ei(x − 0i)` differ by the expected `2πi`.
    ///
    /// The series alternates in sign for a negative real part and loses
    /// digits to cancellation, so it is summed at double the working
    /// precision, exactly as the C++ does.
    fn expint_complex(&mut self) -> bool {
        if self.is_infinite(false) || self.imaginary_part().is_infinite(false) {
            return false;
        }
        if !self.is_nonzero() {
            return false;
        }
        let saved_precision = context::precision();
        let saved_interval = context::create_interval();
        context::set_precision(saved_precision * 2 + 20);
        context::set_create_interval(false);
        let result = self.expint_complex_series();
        context::set_precision(saved_precision);
        context::set_create_interval(saved_interval);
        match result {
            Some(v) => {
                *self = v;
                self.approx = true;
                self.test_float_result(true);
                true
            }
            None => false,
        }
    }

    fn expint_complex_series(&self) -> Option<Number> {
        let z = self.clone();

        // (ln(z) − ln(1/z))/2
        let mut log_term = z.clone();
        if !log_term.ln() {
            return None;
        }
        let mut reciprocal_log = z.clone();
        if !reciprocal_log.recip() || !reciprocal_log.ln() {
            return None;
        }
        if !log_term.subtract(&reciprocal_log) || !log_term.divide(&Number::from_i64(2)) {
            return None;
        }

        // Σ_{k≥1} z^k/(k·k!), starting from the k=1 term, which is z itself.
        let mut sum = z.clone();
        let mut power = z.clone();
        let mut factorial = Number::from_i64(1);
        // A relative change below this ends the sum; the bound is in terms of
        // the *caller's* precision, not the doubled working precision.
        let mut tolerance = Number::from_i64(10);
        if !tolerance.raise(
            &Number::from_i64(-(context::precision() as i64 / 2 + 10)),
            false,
        ) {
            return None;
        }
        for k in 2..MAX_EXPINT_TERMS {
            if !power.multiply(&z) || !factorial.multiply_i64(k) {
                return None;
            }
            let mut term = power.clone();
            if !term.divide(&factorial) || !term.divide_i64(k) {
                return None;
            }
            if !sum.add(&term) {
                return None;
            }
            if sum.is_infinite(false) {
                return None;
            }
            // Converged once the term is negligible against the running sum
            // in both components.
            let mut relative = term.clone();
            if sum.is_nonzero() && !relative.divide(&sum) {
                return None;
            }
            let real_small = magnitude_below(&relative.real_part(), &tolerance);
            let imaginary_small = magnitude_below(&relative.imaginary_part(), &tolerance);
            if real_small && imaginary_small {
                let gamma = euler_gamma_number();
                if !sum.add(&gamma) || !sum.add(&log_term) {
                    return None;
                }
                return Some(sum);
            }
        }
        None
    }

    /// `expint()` — the exponential integral `Ei(x)` (MPFR's `mpfr_eint`).
    pub fn expint(&mut self) -> bool {
        // d(Ei x)/dx = e^x / x.
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.exp() && d.divide(x)).then_some(d)
                },
                Number::expint_impl,
            );
        }
        self.expint_impl()
    }

    fn expint_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.expint_complex();
        }
        if self.is_zero() {
            self.set_minus_infinity(true, false);
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            self.clear(true);
            self.approx = true;
            return true;
        }
        if self.is_plus_infinity() {
            return true;
        }
        if !self.is_nonzero() {
            return false;
        }
        let g = match self.integral_guard() {
            Some(g) => g,
            None => return false, // TODO(port): asymptotic Ei for large |x|
        };
        let wp = context::bit_precision() + GUARD + g;
        self.apply_special(wp, ei_float)
    }

    /// `logint()` — the logarithmic integral `li(x) = Ei(ln x)`.
    pub fn logint(&mut self) -> bool {
        if self.is_zero() {
            return true;
        }
        let bak = self.clone();
        if !self.ln() || !self.expint() {
            *self = bak;
            return false;
        }
        true
    }

    /// `sinint()` — the sine integral `Si(x)`.
    pub fn sinint(&mut self) -> bool {
        // `Si(f)' = f'·sinc(f) = f'·sin(f)/f`
        // (MathStructure-differentiate.cc:417).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.sin() && d.divide(x)).then_some(d)
                },
                Number::sinint_impl,
            );
        }
        self.sinint_impl()
    }

    fn sinint_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.sinint_complex();
        }
        if self.is_plus_infinity() || self.is_minus_infinity() {
            let neg = self.is_minus_infinity();
            let mut pi = Number::new();
            pi.pi();
            if !pi.divide(&Number::from_i64(2)) {
                return false;
            }
            if neg && !pi.negate() {
                return false;
            }
            *self = pi;
            return true;
        }
        if self.is_zero() {
            return true;
        }
        let g = match self.integral_guard() {
            Some(g) => g,
            None => return false, // TODO(port): asymptotic Si for large |x|
        };
        let wp = context::bit_precision() + GUARD + g;
        self.apply_special(wp, si_float)
    }

    /// `cosint()` — the cosine integral `Ci(x)`, real for x > 0.
    pub fn cosint(&mut self) -> bool {
        // `Ci(f)' = f'·cos(f)/f` (MathStructure-differentiate.cc:425).
        if self.unc.is_some() {
            return self.uncertain_unary(
                |x| {
                    let mut d = x.clone();
                    (d.cos() && d.divide(x)).then_some(d)
                },
                Number::cosint_impl,
            );
        }
        self.cosint_impl()
    }

    fn cosint_impl(&mut self) -> bool {
        if self.has_imaginary_part() {
            return self.cosint_complex();
        }
        if self.is_plus_infinity() {
            self.clear(true);
            self.approx = true;
            return true;
        }
        if self.is_minus_infinity() {
            return false;
        }
        if self.is_zero() {
            self.set_minus_infinity(true, false);
            self.approx = true;
            return true;
        }
        if !self.real_part_is_positive() {
            // `Ci(−x) = Ci(x) + πi` for x > 0. The reference applies it in
            // `CosIntFunction::calculate` (BuiltinFunctions-calculus.cc:218)
            // rather than in `Number::cosint`, but it is the same branch of the
            // same logarithm, and the port has only the one entry point.
            if !self.real_part_is_negative() {
                return false; // an interval straddling zero
            }
            if !self.negate() || !self.cosint_impl() {
                return false;
            }
            let mut pi = Number::new();
            pi.pi();
            self.set_imaginary_part(&pi);
            return true;
        }
        let g = match self.integral_guard() {
            Some(g) => g,
            None => return false, // TODO(port): asymptotic Ci for large |x|
        };
        let wp = context::bit_precision() + GUARD + g;
        self.apply_special(wp, ci_float)
    }

    /// `Si(z)` for complex `z` (Number.cc:8976).
    ///
    /// `Si(z) = z·Σ_{k≥0} (−1)^k z^{2k}/((2k+1)(2k+1)!)`, summed at doubled
    /// precision like the C++ — the terms alternate and peak near `e^{|z|}`.
    fn sinint_complex(&mut self) -> bool {
        if self.includes_infinity() {
            return false;
        }
        let m = complex_magnitude(self);
        if !m.is_finite() || m > INTEGRAL_ARG_MAX {
            return false; // TODO(port): asymptotic Si for large |z|
        }
        let result = with_series_precision(m, || self.sinint_complex_series());
        match result {
            Some(v) => {
                *self = v;
                self.approx = true;
                self.test_float_result(true);
                true
            }
            None => false,
        }
    }

    fn sinint_complex_series(&self) -> Option<Number> {
        let z = self.clone();
        let mut mz2 = z.clone();
        if !mz2.square() || !mz2.negate() {
            return None;
        }
        // t_k/t_{k−1} = −z²(2k−1)/((2k+1)²·2k).
        let mut term = Number::from_i64(1);
        let mut sum = Number::from_i64(1);
        let tolerance = series_tolerance()?;
        for k in 1..MAX_EXPINT_TERMS {
            if !term.multiply(&mz2)
                || !term.multiply_i64(2 * k - 1)
                || !term.divide_i64((2 * k + 1) * (2 * k + 1))
                || !term.divide_i64(2 * k)
                || !sum.add(&term)
            {
                return None;
            }
            if sum.is_infinite(false) {
                return None;
            }
            if term_is_negligible(&term, &sum, &tolerance)? {
                return sum.multiply(&z).then_some(sum);
            }
        }
        None
    }

    /// `Ci(z)` for complex `z` (Number.cc:9406).
    ///
    /// `Ci(z) = γ + ln z + Σ_{k≥1} (−1)^k z^{2k}/((2k)(2k)!)`, with `ln` on its
    /// principal branch — which is what carries the `+πi` onto a negative real
    /// argument and the `+πi/2` onto a positive imaginary one.
    fn cosint_complex(&mut self) -> bool {
        if self.includes_infinity() {
            return false;
        }
        let m = complex_magnitude(self);
        if !m.is_finite() || m > INTEGRAL_ARG_MAX {
            return false; // TODO(port): asymptotic Ci for large |z|
        }
        let result = with_series_precision(m, || self.cosint_complex_series());
        match result {
            Some(v) => {
                *self = v;
                self.approx = true;
                self.test_float_result(true);
                true
            }
            None => false,
        }
    }

    fn cosint_complex_series(&self) -> Option<Number> {
        let z = self.clone();
        let mut mz2 = z.clone();
        if !mz2.square() || !mz2.negate() {
            return None;
        }
        // b_1 = −z²/4; b_k/b_{k−1} = −z²(2k−2)/((2k)²(2k−1)).
        let mut term = mz2.clone();
        if !term.divide_i64(4) {
            return None;
        }
        let mut sum = term.clone();
        let tolerance = series_tolerance()?;
        for k in 2..MAX_EXPINT_TERMS {
            if !term.multiply(&mz2)
                || !term.multiply_i64(2 * k - 2)
                || !term.divide_i64((2 * k) * (2 * k))
                || !term.divide_i64(2 * k - 1)
                || !sum.add(&term)
            {
                return None;
            }
            if sum.is_infinite(false) {
                return None;
            }
            if term_is_negligible(&term, &sum, &tolerance)? {
                let mut log = z;
                if !log.ln() || !sum.add(&log) || !sum.add(&euler_gamma_number()) {
                    return None;
                }
                return Some(sum);
            }
        }
        None
    }

    // ------------------------------------------------------------------
    // Stubs
    //
    // Two different things live here, and the difference matters:
    //
    // * `besselj`, `bessely`, `airy` and `polylog` are genuinely unported —
    //   the calculator cannot evaluate them at all (`besselj(0,1)` comes back
    //   unevaluated from the CLI).
    // * `igamma`, `fresnels`, `fresnelc` and `erfinv` are dead code. The
    //   calculator *does* have all four; they are just implemented a layer
    //   up, on `MathStructure` rather than on `Number`, because the reference
    //   defines them over structures too. `igamma`/`fresnels`/`fresnelc` are
    //   in `qalc-core/src/integrate.rs` (`igamma(3, 1)` = `1.839397206`,
    //   `fresnels(1)` = `0.4382591474`, `fresnelc(1)` = `0.7798934004`) and
    //   `erfinv` in `qalc-core/src/stats.rs` (`erfinv_f64`, which `probit`
    //   and `normdistinv` are built on).
    //
    // Nothing calls the second group; they are kept only so the `Number`
    // surface still names every `Number.cc` special function, and the test
    // below pins them at `false` so a caller cannot appear by accident.
    // ------------------------------------------------------------------

    /// TODO(port): Bessel function of the first kind, `mpfr_jn`.
    pub fn besselj(&mut self, _o: &Number) -> bool {
        false
    }

    /// TODO(port): Bessel function of the second kind, `mpfr_yn`.
    pub fn bessely(&mut self, _o: &Number) -> bool {
        false
    }

    /// TODO(port): Airy function Ai, `mpfr_ai`.
    pub fn airy(&mut self) -> bool {
        false
    }

    /// TODO(port): polylogarithm Li_o(x) (`mpfr_li2` for o = 2).
    pub fn polylog(&mut self, _o: &Number) -> bool {
        false
    }

    /// Dead: the upper incomplete gamma is implemented in
    /// `qalc-core/src/integrate.rs` (`FUNCTION_ID_I_GAMMA`), not here.
    pub fn igamma(&mut self, _o: &Number) -> bool {
        false
    }

    /// Dead: the Fresnel sine integral S(x) is implemented in
    /// `qalc-core/src/integrate.rs` (`FUNCTION_ID_FRESNEL_S`), not here.
    pub fn fresnels(&mut self) -> bool {
        false
    }

    /// Dead: the Fresnel cosine integral C(x) is implemented in
    /// `qalc-core/src/integrate.rs` (`FUNCTION_ID_FRESNEL_C`), not here.
    pub fn fresnelc(&mut self) -> bool {
        false
    }

    /// Dead: the inverse error function is implemented as `erfinv_f64` in
    /// `qalc-core/src/stats.rs`, not here.
    pub fn erfinv(&mut self) -> bool {
        false
    }
}

/// Exact rational `c` with `Γ(numer/2) = c·√π`, for odd `numer`.
/// `Γ(m+½) = (2m)!/(4^m·m!)·√π`, `Γ(½−n) = (−4)^n·n!/(2n)!·√π`.
fn half_integer_gamma_coeff(r: &BigRational) -> Option<BigRational> {
    let numer = r.numer();
    let m = (numer - 1i32) / 2i32;
    let mi = m.to_string().parse::<i64>().ok()?;
    if mi.abs() > 4096 {
        return None; // fall back to the float path
    }
    if mi >= 0 {
        let m = mi as u64;
        let num = fact_bigint(2 * m);
        let den = fact_bigint(m) * (BigInt::one() << (2 * m as usize));
        Some(BigRational::new(num, den))
    } else {
        let n = (-mi) as u64;
        let mut num = fact_bigint(n) * (BigInt::one() << (2 * n as usize));
        if n % 2 == 1 {
            num = -num;
        }
        Some(BigRational::new(num, fact_bigint(2 * n)))
    }
}

#[cfg(test)]
mod uncertainty_tests {
    use crate::number::uncertainty_test_support::{plus_minus, uncertain};

    #[test]
    fn error_function_carries_the_gaussian() {
        // Reference: `erf(1+/-0.1)` = `0.843±0.042` — 2/(e·sqrt(pi))·0.1.
        let mut n = uncertain("1", "0.1");
        assert!(n.erf());
        assert_eq!(plus_minus(&n), "0.843±0.042");
    }

    #[test]
    fn gamma_carries_gamma_times_digamma() {
        // Reference: `gamma(3+/-0.1)` = `2.00±0.18` — Γ(3)·ψ(3)·0.1. The
        // uncertainty used to be dropped outright: the exact-integer branch
        // replaces the value through `take_keeping_flags`, which keeps the
        // approximation flags but not `unc`.
        let mut n = uncertain("3", "0.1");
        assert!(n.gamma());
        assert_eq!(plus_minus(&n), "2.00±0.18");
    }

    #[test]
    fn zeta_falls_back_to_interval_arithmetic() {
        // Reference: `zeta(2+/-0.1)` = `1.655±0.095`. Not the variance
        // formula — zeta is not in `function_differentiable`'s list — so the
        // answer is the enclosure of ζ over [1.9, 2.1], whose midpoint is not
        // ζ(2) = 1.6449.
        let mut n = uncertain("2", "0.1");
        assert!(n.zeta());
        assert_eq!(plus_minus(&n), "1.655±0.095");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PrintOptions;

    fn po() -> PrintOptions {
        let mut po = PrintOptions::default();
        po.show_ending_zeroes = true;
        po
    }

    fn s(n: &Number) -> String {
        n.print(&po())
    }

    fn num(a: i64, b: i64) -> Number {
        Number::from_ints(a, b, 0)
    }

    // ---------------- gamma ----------------

    #[test]
    fn gamma_positive_integer_is_exact_factorial() {
        let mut n = Number::from_i64(5);
        assert!(n.gamma());
        assert!(n.is_integer() && !n.is_approximate(), "gamma(5) must stay exact");
        assert_eq!(s(&n), "24");
        let mut n = Number::from_i64(11);
        assert!(n.gamma());
        assert!(n.equals_i64(3628800));
    }

    #[test]
    fn gamma_half_integers() {
        // gamma(0.5) = sqrt(pi)
        let mut n = num(1, 2);
        assert!(n.gamma());
        assert_eq!(s(&n), "1.772453851");
        // gamma(1.5) = sqrt(pi)/2
        let mut n = num(3, 2);
        assert!(n.gamma());
        assert_eq!(s(&n), "0.8862269255");
        // gamma(-0.5) = -2 sqrt(pi)
        let mut n = num(-1, 2);
        assert!(n.gamma());
        assert_eq!(s(&n), "-3.544907702");
    }

    #[test]
    fn gamma_general_real() {
        let mut n = num(37, 10); // 3.7
        assert!(n.gamma());
        assert_eq!(s(&n), "4.170651784");
        let mut n = num(21, 2); // 10.5 — half-integer exact path
        assert!(n.gamma());
        assert_eq!(s(&n), "1133278.389");
        let mut n = num(-27, 10); // -2.7, reflection formula
        assert!(n.gamma());
        assert_eq!(s(&n), "-0.9310827848");
        let mut n = num(-3, 10); // -0.3
        assert!(n.gamma());
        assert_eq!(s(&n), "-4.326851109");
    }

    /// Either side of the threshold on the exact closed forms
    /// (`GammaFunction::calculate`, BuiltinFunctions-special.cc:155, which
    /// only reaches them for an argument `isLessThan(1000)`).
    ///
    /// Without it `gamma(1000000)` builds 999999! — an 18-million-bit integer
    /// — and took over 150s where the reference takes 0.35s. Every value here
    /// is the reference binary's, on both sides, and so is the split between
    /// an exact answer and an approximate one: 998! is exact, Γ(1000) is
    /// MPFR's.
    #[test]
    fn the_exact_gamma_forms_stop_at_a_thousand() {
        let g = |n: Number| {
            let mut n = n;
            assert!(n.gamma());
            n
        };
        assert_eq!(s(&g(Number::from_i64(999))), "4.027900501E2561");
        assert_eq!(s(&g(Number::from_i64(1000))), "4.023872601E2564");
        assert!(!g(Number::from_i64(999)).is_approximate());
        assert!(g(Number::from_i64(1000)).is_approximate());
        // The half-integer form is gated by the same test: 999.5 takes it,
        // 1000.5 does not.
        assert_eq!(s(&g(num(1999, 2))), "1.272937665E2563");
        assert_eq!(s(&g(num(2001, 2))), "1.272301196E2566");
        // (Both half-integer answers are approximate either way — they are a
        // rational times √π — so only the digits distinguish the two routes.)
        // An argument the bound exists for.
        assert_eq!(s(&g(Number::from_i64(10000))), "2.846259681E35655");
    }

    #[test]
    fn gamma_poles_fail() {
        for v in [0i64, -1, -2, -10] {
            let mut n = Number::from_i64(v);
            assert!(!n.gamma(), "gamma({v}) is a pole");
        }
    }

    // ---------------- erf ----------------

    #[test]
    fn erf_zero_is_exact() {
        let mut n = Number::new();
        assert!(n.erf());
        assert!(n.is_zero());
        assert_eq!(s(&n), "0");
    }

    #[test]
    fn erf_values() {
        let mut n = Number::from_i64(1);
        assert!(n.erf());
        assert_eq!(s(&n), "0.8427007929");
        let mut n = Number::from_i64(2);
        assert!(n.erf());
        assert_eq!(s(&n), "0.9953222650");
        let mut n = num(-1, 2);
        assert!(n.erf());
        assert_eq!(s(&n), "-0.5204998778");
    }

    #[test]
    fn erfc_values() {
        let mut n = Number::from_i64(1);
        assert!(n.erfc());
        assert_eq!(s(&n), "0.1572992071");
        // strong cancellation against 1 — the guard bits must cover it
        let mut n = Number::from_i64(3);
        assert!(n.erfc());
        assert_eq!(s(&n), "0.00002209049700");
    }

    #[test]
    fn erf_large_argument_uses_asymptotics() {
        // Beyond the crossover erfc switches to the asymptotic expansion.
        let mut n = Number::from_i64(20);
        assert!(n.erfc());
        assert!(n.is_positive());
        assert_eq!(s(&n), "5.395865612E-176");
    }

    #[test]
    fn erfi_value() {
        let mut n = Number::from_i64(1);
        assert!(n.erfi());
        assert_eq!(s(&n), "1.650425759");
    }

    // ---------------- digamma ----------------

    #[test]
    fn digamma_values() {
        let mut n = Number::from_i64(1);
        assert!(n.digamma());
        assert_eq!(s(&n), "-0.5772156649"); // −γ
        let mut n = num(1, 2);
        assert!(n.digamma());
        assert_eq!(s(&n), "-1.963510026");
        let mut n = Number::from_i64(3);
        assert!(n.digamma());
        assert_eq!(s(&n), "0.9227843351");
        let mut n = num(-3, 2); // reflection formula
        assert!(n.digamma());
        assert_eq!(s(&n), "0.7031566406");
    }

    #[test]
    fn digamma_poles_fail() {
        for v in [0i64, -1, -7] {
            let mut n = Number::from_i64(v);
            assert!(!n.digamma(), "digamma({v}) is a pole");
        }
    }

    // ---------------- zeta ----------------

    #[test]
    fn zeta_even_integers_exact_pi_powers() {
        let mut n = Number::from_i64(2);
        assert!(n.zeta());
        assert_eq!(s(&n), "1.644934067"); // π²/6
        let mut n = Number::from_i64(4);
        assert!(n.zeta());
        assert_eq!(s(&n), "1.082323234"); // π⁴/90
        let mut n = Number::from_i64(10);
        assert!(n.zeta());
        assert_eq!(s(&n), "1.000994575");
    }

    #[test]
    fn zeta_odd_and_fractional() {
        let mut n = Number::from_i64(3);
        assert!(n.zeta());
        assert_eq!(s(&n), "1.202056903"); // Apéry
        let mut n = num(3, 2);
        assert!(n.zeta());
        assert_eq!(s(&n), "2.612375349");
        let mut n = num(1, 2); // Borwein at the edge of its range
        assert!(n.zeta());
        assert_eq!(s(&n), "-1.460354509");
    }

    #[test]
    fn zeta_negative_integers_exact() {
        let mut n = Number::from_i64(-1);
        assert!(n.zeta());
        assert!(n.is_rational() && !n.is_approximate());
        assert_eq!(s(&n), "-0.08333333333"); // −1/12
        assert!(n.equals(&Number::from_ints(-1, 12, 0), false, false));
        let mut n = Number::from_i64(-3);
        assert!(n.zeta());
        assert_eq!(s(&n), "0.008333333333"); // 1/120
        let mut n = Number::from_i64(-2);
        assert!(n.zeta());
        assert!(n.is_zero());
        let mut n = Number::new();
        assert!(n.zeta());
        assert!(n.equals(&Number::from_ints(-1, 2, 0), false, false));
    }

    #[test]
    fn zeta_negative_fractional_via_functional_equation() {
        let mut n = num(-1, 2);
        assert!(n.zeta());
        assert_eq!(s(&n), "-0.2078862250");
    }

    #[test]
    fn zeta_one_fails() {
        let mut n = Number::from_i64(1);
        assert!(!n.zeta());
    }

    // ---------------- bernoulli ----------------

    #[test]
    fn bernoulli_exact_b0_b12() {
        let expect: [(i64, i64); 13] = [
            (1, 1),      // B0
            (-1, 2),     // B1
            (1, 6),      // B2
            (0, 1),      // B3
            (-1, 30),    // B4
            (0, 1),      // B5
            (1, 42),     // B6
            (0, 1),      // B7
            (-1, 30),    // B8
            (0, 1),      // B9
            (5, 66),     // B10
            (0, 1),      // B11
            (-691, 2730) // B12
        ];
        for (i, (a, b)) in expect.iter().enumerate() {
            let mut n = Number::from_i64(i as i64);
            assert!(n.bernoulli(), "bernoulli({i})");
            assert!(!n.is_approximate(), "B_{i} must be exact");
            assert!(
                n.equals(&Number::from_ints(*a, *b, 0), false, false),
                "B_{i} = {a}/{b}, got {}",
                s(&n)
            );
        }
    }

    #[test]
    fn bernoulli_matches_oracle_printing() {
        let mut n = Number::from_i64(20);
        assert!(n.bernoulli());
        assert_eq!(s(&n), "-529.1242424"); // −174611/330
        let mut n = Number::from_i64(2);
        assert!(n.bernoulli());
        assert_eq!(s(&n), "0.1666666667");
        let mut n = Number::from_i64(12);
        assert!(n.bernoulli());
        assert_eq!(s(&n), "-0.2531135531");
    }

    #[test]
    fn bernoulli_rejects_non_integers() {
        let mut n = num(1, 2);
        assert!(!n.bernoulli());
        let mut n = Number::from_i64(-2);
        assert!(!n.bernoulli());
    }

    // ---------------- integrals ----------------

    #[test]
    fn erf_picks_its_branch_by_the_sign_of_the_comparison() {
        // At precision 30 the crossover is ~12.77, and 8 shares a binade with
        // it — so astro-float's signed-magnitude `cmp` returns a large
        // negative mantissa difference rather than -1. Testing `!= Some(-1)`
        // sent erfc(8) down the asymptotic branch, below the crossover where
        // it does not converge. Values are the reference binary's at
        // `set precision 30`.
        let saved = crate::context::precision();
        crate::context::set_precision(30);
        let po = PrintOptions::default();
        let parse = crate::options::ParseOptions::default();

        let mut n = Number::from_i64(8);
        assert!(n.erfc());
        assert_eq!(
            n.print(&po),
            "0.0000000000000000000000000000112242971729829270799678884432"
        );

        let mut n = Number::from_i64(8);
        assert!(n.erf());
        assert_eq!(n.print(&po), "0.999999999999999999999999999989");

        // Just above the crossover, where the asymptotic branch is right.
        let mut n = Number::parse("14.5", &parse);
        assert!(n.erfc());
        assert_eq!(n.print(&po), "1.89939594197950304957420002393E-93");

        // Well below it, where the series is right.
        let mut n = Number::from_i64(3);
        assert!(n.erfc());
        assert_eq!(n.print(&po), "0.0000220904969985854413727761295823");

        crate::context::set_precision(saved);
    }

    #[test]
    fn expint_values() {
        let mut n = Number::from_i64(1);
        assert!(n.expint());
        assert_eq!(s(&n), "1.895117816");
        let mut n = Number::from_i64(2);
        assert!(n.expint());
        assert_eq!(s(&n), "4.954234356");
        let mut n = Number::from_i64(-1);
        assert!(n.expint());
        assert_eq!(s(&n), "-0.2193839344");
    }

    #[test]
    fn logint_values() {
        let mut n = Number::from_i64(2);
        assert!(n.logint());
        assert_eq!(s(&n), "1.045163780");
        let mut n = Number::from_i64(10);
        assert!(n.logint());
        assert_eq!(s(&n), "6.165599505");
    }

    #[test]
    fn sinint_cosint_values() {
        let mut n = Number::from_i64(1);
        assert!(n.sinint());
        assert_eq!(s(&n), "0.9460830704");
        let mut n = Number::from_i64(1);
        assert!(n.cosint());
        assert_eq!(s(&n), "0.3374039229");
        let mut n = Number::from_i64(2);
        assert!(n.sinint());
        assert_eq!(s(&n), "1.605412977");
        let mut n = Number::from_i64(2);
        assert!(n.cosint());
        assert_eq!(s(&n), "0.4229808288");
        let mut n = Number::from_i64(10);
        assert!(n.sinint());
        assert_eq!(s(&n), "1.658347594");
    }

    /// `Ci(x)` is complex for x < 0, and the port now says so with a value
    /// rather than by declining: `Ci(−x) = Ci(x) + πi`, which is what the
    /// reference prints for `Ci(-1)`.
    #[test]
    fn cosint_negative_is_complex() {
        let mut n = Number::from_i64(-1);
        assert!(n.cosint());
        assert_eq!(s(&n.real_part()), "0.3374039229");
        assert_eq!(s(&n.imaginary_part()), "3.141592654");
    }

    /// The complex branches of the erf family and of the trigonometric
    /// integrals, against the reference binary.
    #[test]
    fn complex_special_functions() {
        let z = |a: i64, b: i64| {
            let mut n = Number::from_i64(a);
            n.set_imaginary_part(&Number::from_i64(b));
            n
        };
        type F = fn(&mut Number) -> bool;
        let cases: &[(F, Number, &str, &str)] = &[
            (Number::erf, z(1, 1), "1.316151282", "0.1904534692"),
            (Number::erfc, z(1, 1), "-0.3161512817", "-0.1904534692"),
            (Number::erfi, z(1, 1), "0.1904534692", "1.316151282"),
            (Number::sinint, z(1, 1), "1.104222658", "0.8824538050"),
            (Number::cosint, z(1, 1), "0.8821721806", "0.2872491335"),
            (Number::erf, z(3, 4), "-120.1869914", "-27.75033729"),
            (Number::sinint, z(-2, 3), "-4.547513890", "1.399196581"),
            // Pure imaginary: `erf(x·i) = erfi(x)·i` leaves an exactly zero
            // real part rather than a rounding-noise one (it prints as
            // `0.000000000` only because the value carries the approximate
            // flag; the composite prints as plain `18.56480241i`).
            (Number::erf, z(0, 2), "0.000000000", "18.56480241"),
            (Number::cosint, z(0, 2), "2.452666923", "1.570796327"),
        ];
        for (f, arg, re, im) in cases {
            let mut n = arg.clone();
            assert!(f(&mut n), "declined {}", s(arg));
            assert_eq!(s(&n.real_part()), *re, "real part of f({})", s(arg));
            assert_eq!(s(&n.imaginary_part()), *im, "imaginary part of f({})", s(arg));
        }
    }

    /// Ten digits are not enough to catch a series that stops one term early:
    /// a convergence test stated relative to the *caller's* precision will
    /// agree with the reference at 10 digits and be wrong at 30. These are
    /// mpmath's values at 30 digits.
    #[test]
    fn the_new_series_hold_up_at_higher_precision() {
        let bak = context::precision();
        context::set_precision(30);
        let mut z = Number::from_i64(1);
        z.set_imaginary_part(&Number::from_i64(1));

        let mut n = z.clone();
        assert!(n.erf());
        let erf = (s(&n.real_part()), s(&n.imaginary_part()));

        let mut n = z.clone();
        assert!(n.sinint());
        let si = (s(&n.real_part()), s(&n.imaginary_part()));

        let mut n = z.clone();
        assert!(n.cosint());
        let ci = (s(&n.real_part()), s(&n.imaginary_part()));

        let mut n = Number::from_i64(3);
        assert!(n.hurwitz_zeta(&num(1, 2)));
        let hz = s(&n);

        context::set_precision(bak);
        assert_eq!(erf.0, "1.31615128169794764488027108024");
        assert_eq!(erf.1, "0.190453469237834686284108861969");
        assert_eq!(si.0, "1.10422265823558173955875396985");
        assert_eq!(si.1, "0.882453805007917743376124044695");
        assert_eq!(ci.0, "0.882172180555936325050614116656");
        assert_eq!(ci.1, "0.287249133519955939527283572386");
        assert_eq!(hz, "8.41439832211715999779816713058");
    }

    /// The Hurwitz zeta, on its reference domain (`s > 1`, `a > 0`).
    #[test]
    fn hurwitz_zeta_values() {
        let cases: &[(Number, Number, &str)] = &[
            (Number::from_i64(2), Number::from_i64(2), "0.6449340668"),
            (Number::from_i64(2), Number::from_i64(3), "0.3949340668"),
            (Number::from_i64(3), num(1, 2), "8.414398322"),
            (num(3, 2), num(5, 2), "1.403779769"),
            (Number::from_i64(2), num(1, 4), "17.19732915"),
            // a = 1 is the Riemann zeta, and is handed to it.
            (Number::from_i64(2), Number::from_i64(1), "1.644934067"),
            // Below s = 1 the defining series diverges and the value comes
            // from ζ(s) less its first a−1 terms instead.
            (num(1, 2), Number::from_i64(2), "-2.460354509"),
            (Number::new(), Number::from_i64(2), "-1.5"),
            (Number::from_i64(-1), Number::from_i64(3), "-3.083333333"),
            (Number::from_i64(-11), Number::from_i64(2), "-0.9789072039"),
            (Number::from_i64(-31), Number::from_i64(3), "-1675098781"),
        ];
        for (sv, a, want) in cases {
            let mut n = sv.clone();
            assert!(n.hurwitz_zeta(a), "declined zeta({}, {})", s(sv), s(a));
            assert_eq!(s(&n), *want, "zeta({}, {})", s(sv), s(a));
        }
        // Outside the domain the series does not converge, and the reference
        // declines rather than guess.
        let mut n = Number::from_i64(1);
        assert!(!n.hurwitz_zeta(&Number::from_i64(2)), "ζ(1) is a pole");
        // Below s = 1 only the integer-a shortcut is available, and only
        // inside the window the reference allows itself.
        let mut n = num(1, 2);
        assert!(!n.hurwitz_zeta(&num(5, 2)), "a must be an integer below s = 1");
        let mut n = num(1, 2);
        assert!(!n.hurwitz_zeta(&Number::from_i64(60)), "a is capped at 50");
        let mut n = Number::from_i64(2);
        assert!(!n.hurwitz_zeta(&Number::from_i64(0)), "a must be positive");
        let mut n = Number::from_i64(2);
        assert!(!n.hurwitz_zeta(&Number::from_i64(-1)), "a must be positive");
    }

    // ---------------- precision ----------------

    #[test]
    fn results_hold_up_at_higher_precision() {
        let bak = context::precision();
        context::set_precision(30);
        type F = fn(&mut Number) -> bool;
        let cases: &[(F, Number, &str)] = &[
            (Number::erf, Number::from_i64(1), "0.842700792949714869341220635083"),
            (Number::erfc, Number::from_i64(1), "0.157299207050285130658779364917"),
            (Number::digamma, Number::from_i64(1), "-0.577215664901532860606512090082"),
            (Number::zeta, Number::from_i64(3), "1.20205690315959428539973816151"),
            (Number::zeta, Number::from_ints(1, 2, 0), "-1.46035450880958681288949915252"),
            (Number::gamma, Number::from_ints(37, 10, 0), "4.17065178379660316539360299862"),
            (Number::gamma, Number::from_ints(-27, 10, 0), "-0.931082784838963780987400098321"),
            (Number::sinint, Number::from_i64(1), "0.946083070367183014941353313823"),
            (Number::expint, Number::from_i64(1), "1.89511781635593675546652093433"),
        ];
        let mut failures = Vec::new();
        for (f, arg, want) in cases {
            let mut n = arg.clone();
            if !f(&mut n) || s(&n) != *want {
                failures.push(format!("want {want}, got {}", s(&n)));
            }
        }
        context::set_precision(bak);
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn stubs_return_false() {
        let mut n = Number::from_i64(1);
        assert!(!n.airy());
        assert!(!n.fresnels());
        assert!(!n.fresnelc());
        assert!(!n.erfinv());
        assert!(!n.besselj(&Number::from_i64(0)));
        assert!(!n.bessely(&Number::from_i64(0)));
        assert!(!n.polylog(&Number::from_i64(2)));
        assert!(!n.igamma(&Number::from_i64(1)));
    }
}


/// Largest number of series terms before `Ei` gives up.
const MAX_EXPINT_TERMS: i64 = 100_000;

/// The Euler-Mascheroni constant as a `Number` at the working precision.
fn euler_gamma_number() -> Number {
    let wp = context::bit_precision();
    let g = context::with_consts(|cc| euler_gamma(wp, cc));
    Number::from_interval(g.clone(), g)
}

/// The imaginary unit, `i`.
fn imaginary_unit() -> Number {
    let mut n = Number::new();
    n.set_imaginary_part(&Number::from_i64(1));
    n
}

/// `|z|` as an `f64`, for sizing the guard digits a series needs.  Infinite
/// or NaN when either component is out of `f64` range, which the callers
/// treat as "out of the series' reach".
fn complex_magnitude(z: &Number) -> f64 {
    let re = z.real_part().float_value();
    let im = z.imaginary_part().float_value();
    (re * re + im * im).sqrt()
}

/// Run `f` with the working precision raised the way the C++ raises it around
/// its complex Taylor series — `CALCULATOR->setPrecision(PRECISION * 2 + 20)`
/// — plus `ln(peak_term)` digits on top.
///
/// The extra term is not in the C++ and is the difference between a wrong
/// answer and no answer: an alternating series whose largest term is `e^peak`
/// cancels away `peak·log₁₀e` digits before it reaches its sum, and at
/// doubled precision alone `erf(4+4i)` would print digits that are not there.
/// The context is restored whether or not `f` succeeds.
fn with_series_precision<T>(peak: f64, f: impl FnOnce() -> T) -> T {
    let saved_precision = context::precision();
    let saved_interval = context::create_interval();
    let extra = if peak.is_finite() && peak > 0.0 {
        (peak * std::f64::consts::LOG10_E).ceil() as i32
    } else {
        0
    };
    context::set_precision(saved_precision * 2 + 20 + extra);
    context::set_create_interval(false);
    let out = f();
    context::set_precision(saved_precision);
    context::set_create_interval(saved_interval);
    out
}

/// The relative size below which a series term no longer moves the sum.
/// Read at the *raised* precision the series runs at, so it is stated in terms
/// of the caller's precision, as the C++ `wprec` is.
fn series_tolerance() -> Option<Number> {
    let mut t = Number::from_i64(10);
    t.raise(
        &Number::from_i64(-(context::precision() as i64 / 2 + 10)),
        false,
    )
    .then_some(t)
}

/// Has `term` stopped moving `sum`, in both components?  `None` if the
/// comparison itself could not be formed.
fn term_is_negligible(term: &Number, sum: &Number, tolerance: &Number) -> Option<bool> {
    let mut relative = term.clone();
    if sum.is_nonzero() && !relative.divide(sum) {
        return None;
    }
    Some(
        magnitude_below(&relative.real_part(), tolerance)
            && magnitude_below(&relative.imaginary_part(), tolerance),
    )
}

/// Is `|value|` below `tolerance`? Used to decide that a series term no
/// longer moves the sum.
fn magnitude_below(value: &Number, tolerance: &Number) -> bool {
    let mut magnitude = value.clone();
    magnitude.abs();
    magnitude.is_less_than(tolerance)
}
