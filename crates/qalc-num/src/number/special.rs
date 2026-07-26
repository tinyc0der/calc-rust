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

fn erf_float(x: &BigFloat, wp: usize, cc: &mut Consts) -> BigFloat {
    if x.is_nan() {
        return BigFloat::nan(None);
    }
    let ax = x.abs();
    if ax.cmp(&BigFloat::from_f64(erf_crossover(wp), wp)) != Some(-1) {
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
    if !x.is_negative() && x.cmp(&BigFloat::from_f64(erf_crossover(wp), wp)) != Some(-1) {
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
        if self.has_imaginary_part() {
            return false; // TODO(port): complex gamma (Lanczos on the complex plane)
        }
        if self.is_plus_infinity() {
            return true;
        }
        if self.is_minus_infinity() {
            return false;
        }
        if let RealValue::Rational(r) = &self.value {
            if r.denom().is_one() {
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
            if *r.denom() == BigInt::from(2) {
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
        self.apply_special(wp, gamma_float)
    }

    /// `digamma()` — ψ(x) = Γ′(x)/Γ(x).
    pub fn digamma(&mut self) -> bool {
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
        if self.has_imaginary_part() {
            return false; // TODO(port): complex erf
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
        if self.has_imaginary_part() {
            return false; // TODO(port): complex erfc
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
        if self.has_imaginary_part() {
            return false; // TODO(port): complex erfi
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

    // ------------------------------------------------------------------
    // zeta
    // ------------------------------------------------------------------

    /// `zeta()` — the Riemann zeta function for real arguments.
    ///
    /// Exact for `s = 0`, non-positive integers (`−B_{n+1}/(n+1)`) and even
    /// positive integers (rational multiple of `π^n`); Borwein's alternating
    /// series for `s ≥ ½` and the functional equation below that.
    pub fn zeta(&mut self) -> bool {
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
            return false; // TODO(port): complex Ei
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
        if self.has_imaginary_part() {
            return false; // TODO(port): complex Si
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
        if self.has_imaginary_part() {
            return false; // TODO(port): complex Ci
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
            return false; // TODO(port): Ci(x) is complex for x < 0
        }
        let g = match self.integral_guard() {
            Some(g) => g,
            None => return false, // TODO(port): asymptotic Ci for large |x|
        };
        let wp = context::bit_precision() + GUARD + g;
        self.apply_special(wp, ci_float)
    }

    // ------------------------------------------------------------------
    // Not yet ported
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

    /// TODO(port): upper incomplete gamma, `mpfr_gamma_inc`.
    pub fn igamma(&mut self, _o: &Number) -> bool {
        false
    }

    /// TODO(port): Fresnel sine integral S(x).
    pub fn fresnels(&mut self) -> bool {
        false
    }

    /// TODO(port): Fresnel cosine integral C(x).
    pub fn fresnelc(&mut self) -> bool {
        false
    }

    /// TODO(port): inverse error function.
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

    #[test]
    fn cosint_domain() {
        let mut n = Number::from_i64(-1);
        assert!(!n.cosint(), "Ci(x) is complex for x < 0");
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
