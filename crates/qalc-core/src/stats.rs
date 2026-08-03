//! Statistics builtins — the C++ `BuiltinFunctions-statistics.cc` classes
//! (`total`, `percentile`, `min`, `max`, `mode`) plus the large family of
//! *user functions* defined by `<expression>` in `data/functions.xml.in`
//! (`mean`, `stdev`, `covar`, `ttest`, `normdist`, `quadraticfit`, …).
//!
//! Each entry below quotes the XML expression (or the C++ method) it ports,
//! because the exact formula matters: libqalculate's `stdev` is the *sample*
//! standard deviation (`var` divides by `n-1`) while `pearson` uses the
//! *population* one (`varp`, dividing by `n`), and `percentile` implements
//! the nine R quantile methods with 7 as the default.
//!
//! Sums, means and variances are computed with `Number`, so a vector of
//! decimal literals is summed as an exact rational and only rounded at
//! print time. Only the handful of special functions with no closed form —
//! the regularized incomplete beta and gamma integrals behind `fdist` and
//! `chisqdistinv` — drop to `f64`.

use std::path::PathBuf;
use std::sync::RwLock;

use crate::ids::FunctionId;
use crate::structure::MathStructure;
use qalc_num::{Number, ParseOptions};

use MathStructure as M;

/// Function ids. `TOTAL`..`MODE` are the real `FUNCTION_ID_*` values from
/// BuiltinFunctions.h:406-410; `LOAD` is FUNCTION_ID_LOAD (1126, :239). The rest are
/// XML user functions with no C++ id, so the port allocates a private block.
pub mod id {
    pub const LOAD: u32 = 1126;
    pub const TOTAL: u32 = 2200;
    pub const PERCENTILE: u32 = 2201;
    pub const MIN: u32 = 2202;
    pub const MAX: u32 = 2203;
    pub const MODE: u32 = 2204;

    pub const MEAN: u32 = 2950;
    pub const MEDIAN: u32 = 2951;
    pub const QUARTILE: u32 = 2952;
    pub const DECILE: u32 = 2953;
    pub const IQR: u32 = 2954;
    pub const RANGE: u32 = 2955;
    pub const NUMBER: u32 = 2956;
    pub const HARMMEAN: u32 = 2957;
    pub const GEOMEAN: u32 = 2958;
    pub const TRIMMEAN: u32 = 2959;
    pub const WINSORMEAN: u32 = 2960;
    pub const WEIGHMEAN: u32 = 2961;
    pub const RMS: u32 = 2962;
    pub const VAR: u32 = 2963;
    pub const VARP: u32 = 2964;
    pub const STDEV: u32 = 2965;
    pub const STDEVP: u32 = 2966;
    pub const STDERR: u32 = 2967;
    pub const MEANDEV: u32 = 2968;
    pub const COVAR: u32 = 2969;
    pub const POOLVAR: u32 = 2970;
    pub const PEARSON: u32 = 2971;
    pub const SPEARMAN: u32 = 2972;
    pub const TTEST: u32 = 2973;
    pub const PTTEST: u32 = 2974;
    pub const NORMDIST: u32 = 2975;
    pub const NORMDISTINV: u32 = 2976;
    pub const CHISQDISTINV: u32 = 2977;
    pub const FDIST: u32 = 2978;
    pub const PROBIT: u32 = 2979;
    pub const QUADRATICFIT: u32 = 2980;
    pub const CUBICFIT: u32 = 2981;
}

/// Resolve a statistics builtin name to its id.
pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let v = match name {
        "load" => id::LOAD,
        "total" | "add" => id::TOTAL,
        "percentile" => id::PERCENTILE,
        "min" => id::MIN,
        "max" => id::MAX,
        "mode" => id::MODE,
        "mean" | "average" => id::MEAN,
        "median" => id::MEDIAN,
        "quartile" => id::QUARTILE,
        "decile" => id::DECILE,
        "iqr" => id::IQR,
        "range" => id::RANGE,
        "number" => id::NUMBER,
        "harmmean" => id::HARMMEAN,
        "geomean" => id::GEOMEAN,
        "trimmean" => id::TRIMMEAN,
        "winsormean" => id::WINSORMEAN,
        "weighmean" => id::WEIGHMEAN,
        "rms" => id::RMS,
        "var" => id::VAR,
        "varp" => id::VARP,
        "stdev" => id::STDEV,
        "stdevp" => id::STDEVP,
        "stderr" => id::STDERR,
        "meandev" => id::MEANDEV,
        "cov" | "covar" => id::COVAR,
        "poolvar" => id::POOLVAR,
        "pearson" | "correl" | "cor" => id::PEARSON,
        "spearman" => id::SPEARMAN,
        "ttest" => id::TTEST,
        "pttest" => id::PTTEST,
        "normdist" => id::NORMDIST,
        "normdistinv" => id::NORMDISTINV,
        "chisqdistinv" => id::CHISQDISTINV,
        "fdist" => id::FDIST,
        "probit" => id::PROBIT,
        "quadraticfit" => id::QUADRATICFIT,
        "cubicfit" => id::CUBICFIT,
        _ => return None,
    };
    Some(FunctionId(v))
}

/// Display names for the ids above.
pub fn function_name(fid: u32) -> Option<&'static str> {
    Some(match fid {
        id::LOAD => "load",
        id::TOTAL => "total",
        id::PERCENTILE => "percentile",
        id::MIN => "min",
        id::MAX => "max",
        id::MODE => "mode",
        id::MEAN => "mean",
        id::MEDIAN => "median",
        id::QUARTILE => "quartile",
        id::DECILE => "decile",
        id::IQR => "iqr",
        id::RANGE => "range",
        id::NUMBER => "number",
        id::HARMMEAN => "harmmean",
        id::GEOMEAN => "geomean",
        id::TRIMMEAN => "trimmean",
        id::WINSORMEAN => "winsormean",
        id::WEIGHMEAN => "weighmean",
        id::RMS => "rms",
        id::VAR => "var",
        id::VARP => "varp",
        id::STDEV => "stdev",
        id::STDEVP => "stdevp",
        id::STDERR => "stderr",
        id::MEANDEV => "meandev",
        id::COVAR => "covar",
        id::POOLVAR => "poolvar",
        id::PEARSON => "pearson",
        id::SPEARMAN => "spearman",
        id::TTEST => "ttest",
        id::PTTEST => "pttest",
        id::NORMDIST => "normdist",
        id::NORMDISTINV => "normdistinv",
        id::CHISQDISTINV => "chisqdistinv",
        id::FDIST => "fdist",
        id::PROBIT => "probit",
        id::QUADRATICFIT => "quadraticfit",
        id::CUBICFIT => "cubicfit",
        _ => return None,
    })
}

// ----------------------------------------------------------------------
// Data directory for `load`
// ----------------------------------------------------------------------

static DATA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Directory that `load("relative/path")` falls back to when the path does
/// not resolve against the working directory. The CLI points this at the
/// transcript's project root so `load(tests/data.csv)` works from anywhere.
pub fn set_data_dir(dir: PathBuf) {
    if let Ok(mut d) = DATA_DIR.write() {
        *d = Some(dir);
    }
}

fn read_data_file(path: &str) -> Option<String> {
    if let Ok(s) = std::fs::read_to_string(path) {
        return Some(s);
    }
    let base = DATA_DIR.read().ok()?.clone()?;
    std::fs::read_to_string(base.join(path)).ok()
}

/// `Calculator::importCSV` — the body of `LoadFunction`
/// (BuiltinFunctions-matrixvector.cc:1716).
fn import_csv(text: &str, first_row: i64, delim: &str) -> Option<M> {
    let delim = if delim == "tab" { "\t" } else { delim };
    let sep = delim.chars().next().unwrap_or(',');
    let mut rows: Vec<M> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if (i as i64) < first_row - 1 {
            continue;
        }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let cells: Vec<M> = line
            .split(sep)
            .map(|c| {
                let c = c.trim().trim_matches('"');
                M::Number(Number::parse(c, &ParseOptions::default()))
            })
            .collect();
        rows.push(M::Vector(cells));
    }
    match rows.len() {
        0 => None,
        // A single-row file is a plain vector, not a 1×n matrix.
        1 => Some(rows.into_iter().next().expect("one row")),
        _ => Some(M::Vector(rows)),
    }
}

// ----------------------------------------------------------------------
// Numeric helpers (`Number` mutates and reports success)
// ----------------------------------------------------------------------

fn n(i: i64) -> Number {
    Number::from_i64(i)
}

fn add(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.add(b).then_some(r)
}
fn sub(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.subtract(b).then_some(r)
}
fn mul(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.multiply(b).then_some(r)
}
fn div(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.divide(b).then_some(r)
}
fn powi(a: &Number, e: i64) -> Option<Number> {
    let mut r = a.clone();
    r.raise(&n(e), true).then_some(r)
}
fn abs_of(a: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.abs().then_some(r)
}
/// `x^(1/2)` — the form the XML formulas use, so the rounding matches.
fn sqrt_of(a: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.raise(&Number::from_ints(1, 2, 0), false).then_some(r)
}
fn from_f64(v: f64) -> Option<Number> {
    if !v.is_finite() {
        return None;
    }
    let mut r = Number::new();
    r.set_float(v);
    Some(r)
}

/// The numeric elements of a `VectorArgument`.
fn number_vector(m: &M) -> Option<Vec<Number>> {
    match m {
        M::Vector(v) => v.iter().map(|e| e.number().cloned()).collect(),
        M::Number(x) => Some(vec![x.clone()]),
        _ => None,
    }
}

/// `VectorArgument`'s reoccurring-argument mode (Function.cc:2363): a
/// one-vector function called as `mean(5; 6; 4)` collects every argument
/// into the vector.
fn vector_arg_all(args: &[M]) -> Option<Vec<Number>> {
    if args.len() == 1 {
        return number_vector(&args[0]);
    }
    let mut out = Vec::new();
    for a in args {
        out.extend(number_vector(a)?);
    }
    Some(out)
}

fn sorted(v: &[Number]) -> Vec<Number> {
    let mut s = v.to_vec();
    s.sort_by(|a, b| {
        if a.is_less_than(b) {
            std::cmp::Ordering::Less
        } else if b.is_less_than(a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    s
}

// ----------------------------------------------------------------------
// The statistics proper
// ----------------------------------------------------------------------

/// `TotalFunction::calculate`.
fn total(v: &[Number]) -> Option<Number> {
    let mut acc = Number::new();
    for x in v {
        acc.add(x).then_some(())?;
    }
    Some(acc)
}

/// `<expression>total(\x)/dimension(\x)</expression>`
fn mean(v: &[Number]) -> Option<Number> {
    if v.is_empty() {
        return None;
    }
    div(&total(v)?, &n(v.len() as i64))
}

/// `<expression>total((\x-mean).^2)/(dimension(\x)-1)</expression>` (`var`)
/// and the `/dimension(\x)` population form (`varp`).
fn variance(v: &[Number], population: bool) -> Option<Number> {
    let m = mean(v)?;
    let mut acc = Number::new();
    for x in v {
        acc.add(&powi(&sub(x, &m)?, 2)?).then_some(())?;
    }
    let denom = if population {
        n(v.len() as i64)
    } else {
        n(v.len() as i64 - 1)
    };
    div(&acc, &denom)
}

/// `PercentileFunction::calculate` — the nine R quantile methods.
fn percentile(v: &[Number], p: &Number, variant: i64) -> Option<Number> {
    if v.is_empty() {
        return None;
    }
    let s = sorted(v);
    let len = s.len() as i64;
    let hundred = n(100);
    if p.equals(&hundred, true, false) {
        return Some(s[s.len() - 1].clone());
    }
    if p.is_zero() {
        return Some(s[0].clone());
    }
    let mut pfr = div(p, &hundred)?;
    // The median shortcut runs before the method table.
    let half = Number::from_ints(1, 2, 0);
    if pfr.equals(&half, true, false) {
        if s.len() % 2 == 1 {
            return Some(s[s.len() / 2].clone());
        }
        let a = &s[s.len() / 2 - 1];
        let b = &s[s.len() / 2];
        return mul(&add(a, b)?, &half);
    }
    match variant {
        2 => {
            let ufr = mul(&pfr, &n(len))?;
            if ufr.is_integer() {
                // The C++ indexes with `uintValue()` and no bounds test; a
                // percentage outside 0–100 is rejected by the argument
                // definition long before it gets here, so anything out of
                // range is left unevaluated rather than indexed.
                let lo = ufr.to_i64()?;
                if lo < 1 || lo > len {
                    return None;
                }
                let lo = lo as usize;
                let mut r = s[lo - 1].clone();
                if lo + 1 > s.len() {
                    return Some(r);
                }
                r.add(&s[lo]).then_some(())?;
                return mul(&r, &half);
            }
            // Falls through to method 1, as in the C++ switch.
            let mut q = mul(&pfr, &n(len))?;
            q.ceil().then_some(())?;
            return Some(s[index_clamped(&q, s.len())?].clone());
        }
        1 => {
            let mut q = mul(&pfr, &n(len))?;
            q.ceil().then_some(())?;
            return Some(s[index_clamped(&q, s.len())?].clone());
        }
        3 => {
            let mut q = mul(&pfr, &n(len))?;
            q.round(qalc_num::options::RoundingMode::HalfToEven)
                .then_some(())?;
            return Some(s[index_clamped(&q, s.len())?].clone());
        }
        4 => pfr = mul(&pfr, &n(len))?,
        5 => pfr = add(&mul(&pfr, &n(len))?, &half)?,
        6 => pfr = mul(&pfr, &n(len + 1))?,
        7 => pfr = add(&mul(&pfr, &n(len - 1))?, &n(1))?,
        9 => {
            pfr = mul(&pfr, &Number::from_ints(len * 4 + 1, 4, 0))?;
            pfr = add(&pfr, &Number::from_ints(3, 8, 0))?;
        }
        // 8 and anything else: the C++ default branch.
        _ => {
            pfr = mul(&pfr, &Number::from_ints(len * 3 + 1, 3, 0))?;
            pfr = add(&pfr, &Number::from_ints(1, 3, 0))?;
        }
    }
    let mut ufr = pfr.clone();
    ufr.ceil().then_some(())?;
    let mut lfr = pfr.clone();
    lfr.floor().then_some(())?;
    let frac = sub(&pfr, &lfr)?;
    let u_index = ufr.to_i64()?;
    let l_index = lfr.to_i64()?;
    if u_index > s.len() as i64 {
        return Some(s[s.len() - 1].clone());
    }
    if l_index <= 0 {
        return Some(s[0].clone());
    }
    let lo = &s[l_index as usize - 1];
    let hi = &s[u_index as usize - 1];
    add(lo, &mul(&sub(hi, lo)?, &frac)?)
}

/// An `IntegerArgument` with an inclusive range: a non-integer, a
/// non-number or an out-of-range value leaves the call unevaluated.
fn integer_in(m: Option<&M>, min: i64, max: i64) -> Option<i64> {
    let i = m?.number().filter(|x| x.is_integer())?.to_i64()?;
    (min..=max).contains(&i).then_some(i)
}

/// The quantile-method argument shared by `percentile`, `quartile`,
/// `decile` and `iqr`: an integer in 1–9, defaulting to 7.
fn quantile_variant(m: Option<&M>) -> Option<i64> {
    match m {
        None | Some(M::Undefined) => Some(7),
        Some(_) => integer_in(m, 1, 9),
    }
}

fn index_clamped(q: &Number, len: usize) -> Option<usize> {
    let mut i = q.to_i64()?;
    if i > len as i64 {
        i = len as i64;
    }
    if i < 1 {
        i = 1;
    }
    Some(i as usize - 1)
}

/// `ModeFunction::calculate` — the most frequent value, ties going to the
/// first run reached in sorted order.
fn mode(v: &[Number]) -> Option<Number> {
    if v.is_empty() {
        return None;
    }
    let s = sorted(v);
    let (mut run, mut best) = (1usize, 0usize);
    let mut value: Option<&Number> = None;
    for i in 1..s.len() {
        if s[i].equals(&s[i - 1], true, false) {
            run += 1;
        } else {
            if run > best {
                best = run;
                value = Some(&s[i - 1]);
            }
            run = 1;
        }
    }
    if run > best {
        value = s.last();
    }
    value.cloned().or_else(|| s.first().cloned())
}

fn min_of(v: &[Number]) -> Option<Number> {
    v.iter()
        .cloned()
        .reduce(|a, b| if b.is_less_than(&a) { b } else { a })
}

fn max_of(v: &[Number]) -> Option<Number> {
    v.iter()
        .cloned()
        .reduce(|a, b| if b.is_greater_than(&a) { b } else { a })
}

/// `element(v, i)` — the 1-based lookup the XML formulas use, `None` when
/// the index falls outside the vector.
fn nth(v: &[Number], i: i64) -> Option<&Number> {
    if i < 1 {
        return None;
    }
    v.get(i as usize - 1)
}

/// `limits(v, from, to)` — the 1-based inclusive slice the XML formulas use.
fn slice(v: &[Number], from: i64, to: i64) -> Vec<Number> {
    let lo = from.max(1) as usize;
    let hi = (to.max(0) as usize).min(v.len());
    if lo > hi {
        return Vec::new();
    }
    v[lo - 1..hi].to_vec()
}

/// `RankFunction` — the 1-based position of each element in sorted order.
fn ranks(v: &[Number]) -> Vec<Number> {
    let mut order: Vec<usize> = (0..v.len()).collect();
    order.sort_by(|&a, &b| {
        if v[a].is_less_than(&v[b]) {
            std::cmp::Ordering::Less
        } else if v[b].is_less_than(&v[a]) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    let mut out = vec![Number::new(); v.len()];
    for (rank, &i) in order.iter().enumerate() {
        out[i] = n(rank as i64 + 1);
    }
    out
}

/// `<expression>total((\x-mean(\x)).*(\y-mean(\y)))/dimension(\x)</expression>`
fn covar(x: &[Number], y: &[Number]) -> Option<Number> {
    if x.len() != y.len() || x.is_empty() {
        return None;
    }
    let mx = mean(x)?;
    let my = mean(y)?;
    let mut acc = Number::new();
    for (a, b) in x.iter().zip(y) {
        acc.add(&mul(&sub(a, &mx)?, &sub(b, &my)?)?).then_some(())?;
    }
    div(&acc, &n(x.len() as i64))
}

/// `<expression>(total((\x-mx).^2)+total((\y-my).^2))/(dim(\x)+dim(\y)-2)</expression>`
fn poolvar(x: &[Number], y: &[Number]) -> Option<Number> {
    let mx = mean(x)?;
    let my = mean(y)?;
    let mut acc = Number::new();
    for a in x {
        acc.add(&powi(&sub(a, &mx)?, 2)?).then_some(())?;
    }
    for b in y {
        acc.add(&powi(&sub(b, &my)?, 2)?).then_some(())?;
    }
    div(&acc, &n(x.len() as i64 + y.len() as i64 - 2))
}

/// `<expression>abs(varp(\x)^(1/2))</expression>`
fn stdevp(v: &[Number]) -> Option<Number> {
    abs_of(&sqrt_of(&variance(v, true)?)?)
}

/// `<expression>abs((var(\x)/dimension(\x))^(1/2))</expression>`
fn stderr_of(v: &[Number]) -> Option<Number> {
    abs_of(&sqrt_of(&div(&variance(v, false)?, &n(v.len() as i64))?)?)
}

/// Elementwise difference, for `pttest`.
fn elementwise_sub(x: &[Number], y: &[Number]) -> Option<Vec<Number>> {
    if x.len() != y.len() {
        return None;
    }
    x.iter().zip(y).map(|(a, b)| sub(a, b)).collect()
}

// ----------------------------------------------------------------------
// Distributions
// ----------------------------------------------------------------------

/// `log Γ(x)` — Lanczos approximation (g = 7, n = 9), good to ~1e-15
/// relative for the arguments the distribution functions use.
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection: Γ(x)Γ(1-x) = π / sin(πx)
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + 7.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularized lower incomplete gamma `P(a, x)`.
fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x < a + 1.0 {
        // Series expansion.
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..1000 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-17 {
                break;
            }
        }
        sum * (-x + a * x.ln() - ln_gamma(a)).exp()
    } else {
        // Continued fraction for Q(a, x) = 1 - P(a, x).
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..1000 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-17 {
                break;
            }
        }
        1.0 - (-x + a * x.ln() - ln_gamma(a)).exp() * h
    }
}

/// Continued fraction for the incomplete beta (Lentz's method).
fn beta_cf(z: f64, a: f64, b: f64) -> f64 {
    let tiny = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * z / qap;
    if d.abs() < tiny {
        d = tiny;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..500 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * z / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * z / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < tiny {
            d = tiny;
        }
        c = 1.0 + aa / c;
        if c.abs() < tiny {
            c = tiny;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-16 {
            break;
        }
    }
    h
}

/// Regularized incomplete beta `I_z(a, b)`.
fn beta_inc(z: f64, a: f64, b: f64) -> f64 {
    if z <= 0.0 {
        return 0.0;
    }
    if z >= 1.0 {
        return 1.0;
    }
    let front = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * z.ln() + b * (1.0 - z).ln()).exp();
    if z < (a + 1.0) / (a + b + 2.0) {
        front * beta_cf(z, a, b) / a
    } else {
        1.0 - front * beta_cf(1.0 - z, b, a) / b
    }
}

/// Invert `P(a, x) = p` in `x` — the Newton solve `chisqdistinv` performs on
/// `1 - igamma(k/2, x/2)/gamma(k/2) = p`.
fn gamma_p_inv(a: f64, p: f64) -> Option<f64> {
    if !(0.0..=1.0).contains(&p) {
        return None;
    }
    // Bracket, then bisect: robust and plenty accurate for f64.
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    while gamma_p(a, hi) < p && hi < 1e12 {
        hi *= 2.0;
    }
    for _ in 0..300 {
        let mid = 0.5 * (lo + hi);
        if gamma_p(a, mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// `erf(x)` from the regularized incomplete gamma: `erf(x) = sgn(x)·P(½, x²)`.
fn erf_f64(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let p = gamma_p(0.5, x * x);
    if x < 0.0 {
        -p
    } else {
        p
    }
}

/// `erfinv(y)` — Giles' rational initial guess refined by Newton steps on
/// [`erf_f64`]. (`Number::erfinv` is still a stub in qalc-num.)
fn erfinv_f64(y: f64) -> Option<f64> {
    if !(-1.0..=1.0).contains(&y) {
        return None;
    }
    if y == 0.0 {
        return Some(0.0);
    }
    if y.abs() == 1.0 {
        return None;
    }
    let mut w = -((1.0 - y) * (1.0 + y)).ln();
    let mut x;
    if w < 5.0 {
        w -= 2.5;
        let mut p = 2.810_226_36e-08;
        for c in [
            3.432_739_39e-07,
            -3.523_484_51e-06,
            -4.391_506_54e-06,
            0.000_218_580_87,
            -0.001_253_725_03,
            -0.004_177_681_36,
            0.246_640_727,
            1.501_409_41,
        ] {
            p = c + p * w;
        }
        x = p * y;
    } else {
        w = w.sqrt() - 3.0;
        let mut p = -0.000_200_214_257;
        for c in [
            0.000_100_950_558,
            0.001_349_343_22,
            -0.003_673_428_44,
            0.005_739_507_3,
            -0.007_622_461_3,
            0.009_438_870_47,
            1.001_674_06,
            2.832_976_82,
        ] {
            p = c + p * w;
        }
        x = p * y;
    }
    // Newton: f(x) = erf(x) - y, f'(x) = 2/sqrt(pi) e^(-x^2).
    let two_over_sqrt_pi = 2.0 / std::f64::consts::PI.sqrt();
    for _ in 0..4 {
        let err = erf_f64(x) - y;
        x -= err / (two_over_sqrt_pi * (-x * x).exp());
    }
    Some(x)
}

/// `<expression>sqrt(2)*erfinv(2\x-1)</expression>`
fn probit(p: &Number) -> Option<Number> {
    let t = sub(&mul(p, &n(2))?, &n(1))?;
    let inv = from_f64(erfinv_f64(t.float_value())?)?;
    mul(&sqrt_of(&n(2))?, &inv)
}

/// `normdist(x, mean{0}, stdev{1}, cumulative{0})`.
fn normdist(x: &Number, mu: &Number, sigma: &Number, cumulative: bool) -> Option<Number> {
    let z = div(&sub(x, mu)?, sigma)?;
    if cumulative {
        // (1 + erf((x - mean) / (stdev * sqrt(2)))) / 2
        let mut t = div(&z, &sqrt_of(&n(2))?)?;
        t.erf().then_some(())?;
        t.add(&n(1)).then_some(())?;
        return div(&t, &n(2));
    }
    // 1/stdev * 1/sqrt(2 pi) * e^(-1/2 * ((x - mean)/stdev)^2)
    let mut two_pi = Number::new();
    two_pi.pi();
    two_pi.multiply(&n(2)).then_some(())?;
    let mut e = mul(&powi(&z, 2)?, &Number::from_ints(-1, 2, 0))?;
    e.exp().then_some(())?;
    let d = mul(sigma, &sqrt_of(&two_pi)?)?;
    div(&e, &d)
}

/// `fdist(x, d1, d2, cumulative{0})`.
fn fdist(x: f64, d1: f64, d2: f64, cumulative: bool) -> Option<f64> {
    if x < 0.0 || d1 <= 0.0 || d2 <= 0.0 {
        return None;
    }
    if cumulative {
        // betainc(d1 x / (d1 x + d2), d1/2, d2/2)
        return Some(beta_inc(d1 * x / (d1 * x + d2), d1 / 2.0, d2 / 2.0));
    }
    // sqrt((d1 x)^d1 d2^d2 / (d1 x + d2)^(d1 + d2)) / (x B(d1/2, d2/2))
    let ln_num = d1 * (d1 * x).ln() + d2 * d2.ln() - (d1 + d2) * (d1 * x + d2).ln();
    let ln_beta = ln_gamma(d1 / 2.0) + ln_gamma(d2 / 2.0) - ln_gamma((d1 + d2) / 2.0);
    Some((0.5 * ln_num - ln_beta).exp() / x)
}

// ----------------------------------------------------------------------
// Polynomial regression (`quadraticfit` / `cubicfit`)
// ----------------------------------------------------------------------

/// Solve the normal equations for a degree-`deg` least-squares fit of
/// `ys` against `xs`, exactly, and return the coefficients highest power
/// first — the `multisolve(...)` subfunction of the XML definitions.
fn polyfit(xs: &[Number], ys: &[Number], deg: usize) -> Option<Vec<Number>> {
    let k = deg + 1;
    // Normal equations: sum(x^(i+j)) c_j = sum(x^i y).
    let mut a = vec![vec![Number::new(); k + 1]; k];
    for i in 0..k {
        for j in 0..k {
            let mut s = Number::new();
            for x in xs {
                s.add(&powi(x, (2 * deg - i - j) as i64)?).then_some(())?;
            }
            a[i][j] = s;
        }
        let mut s = Number::new();
        for (x, y) in xs.iter().zip(ys) {
            s.add(&mul(&powi(x, (deg - i) as i64)?, y)?).then_some(())?;
        }
        a[i][k] = s;
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..k {
        let pivot = (col..k).find(|&r| !a[r][col].is_zero())?;
        a.swap(col, pivot);
        for r in 0..k {
            if r == col {
                continue;
            }
            let factor = div(&a[r][col], &a[col][col])?;
            for c in col..=k {
                let t = mul(&factor, &a[col][c].clone())?;
                a[r][c] = sub(&a[r][c], &t)?;
            }
        }
    }
    (0..k).map(|i| div(&a[i][k], &a[i][i])).collect()
}

/// Build `c0 x^deg + c1 x^(deg-1) + … + cdeg`.
fn polynomial_structure(coeffs: &[Number]) -> M {
    let deg = coeffs.len() - 1;
    let x = M::symbolic("x");
    let mut terms = Vec::new();
    for (i, c) in coeffs.iter().enumerate() {
        let power = deg - i;
        if c.is_zero() {
            continue;
        }
        let term = match power {
            0 => M::Number(c.clone()),
            1 => M::Multiplication(vec![M::Number(c.clone()), x.clone()]),
            p => M::Multiplication(vec![
                M::Number(c.clone()),
                M::Power {
                    base: Box::new(x.clone()),
                    exponent: Box::new(M::from_i64(p as i64)),
                },
            ]),
        };
        terms.push(term);
    }
    match terms.len() {
        0 => M::Number(Number::new()),
        1 => terms.into_iter().next().expect("one term"),
        _ => M::Addition(terms),
    }
}

// ----------------------------------------------------------------------
// Dispatch
// ----------------------------------------------------------------------

/// Evaluate a statistics builtin in place.
pub fn calculate_function(m: &mut M) -> bool {
    let M::Function { id: fid, args } = m else {
        return false;
    };
    let fid = fid.0;
    if function_name(fid).is_none() && fid != crate::matrix::id::GENERATE_VECTOR {
        return false;
    }
    let args = args.clone();
    match apply(fid, &args) {
        Some(r) => {
            *m = r;
            true
        }
        None => false,
    }
}

fn apply(fid: u32, args: &[M]) -> Option<M> {
    // `genvector(expr, begin, end)` without an explicit step: the C++
    // default value for argument 4 is 1, which the matrix module's stricter
    // form does not supply.
    if fid == crate::matrix::id::GENERATE_VECTOR {
        if args.len() != 3 {
            return None;
        }
        let mut extended = args.to_vec();
        extended.push(M::from_i64(1));
        let mut call = M::Function {
            id: FunctionId(crate::matrix::id::GENERATE_VECTOR),
            args: extended,
        };
        return crate::matrix::calculate_function(&mut call).then_some(call);
    }
    if fid == id::LOAD {
        let path = args.first()?;
        let path = match path {
            M::Text(s) | M::Symbolic(s) => s.clone(),
            _ => return None,
        };
        let first_row = args.get(1).and_then(|m| m.number()?.to_i64()).unwrap_or(1);
        let delim = match args.get(2) {
            Some(M::Text(s)) | Some(M::Symbolic(s)) => s.clone(),
            _ => ",".to_string(),
        };
        return import_csv(&read_data_file(&path)?, first_row, &delim);
    }

    // Everything below is numeric.
    let num = |m: &M| m.number().cloned();
    match fid {
        id::TOTAL => total(&vector_arg_all(args)?).map(M::Number),
        id::MEAN => mean(&vector_arg_all(args)?).map(M::Number),
        id::MIN => min_of(&vector_arg_all(args)?).map(M::Number),
        id::MAX => max_of(&vector_arg_all(args)?).map(M::Number),
        id::MODE => mode(&vector_arg_all(args)?).map(M::Number),
        // <expression>dimension(\x)</expression>
        id::NUMBER => Some(M::from_i64(vector_arg_all(args)?.len() as i64)),
        // <expression>max(\x)-min(\x)</expression>
        id::RANGE => {
            let v = vector_arg_all(args)?;
            sub(&max_of(&v)?, &min_of(&v)?).map(M::Number)
        }
        id::PERCENTILE if args.len() >= 2 => {
            let v = number_vector(&args[0])?;
            let p = num(&args[1])?;
            // `NumberArgument` with min 0 and max 100, both inclusive
            // (BuiltinFunctions-statistics.cc:61); outside that the function
            // is left unevaluated.
            if p.is_less_than(&n(0)) || p.is_greater_than(&n(100)) {
                return None;
            }
            let variant = quantile_variant(args.get(2))?;
            percentile(&v, &p, variant).map(M::Number)
        }
        // <expression>percentile(\x,50)</expression>
        id::MEDIAN => {
            let v = vector_arg_all(args)?;
            percentile(&v, &n(50), 7).map(M::Number)
        }
        // <expression>percentile(\x,25*\y,\Z{7})</expression>
        id::QUARTILE | id::DECILE if args.len() >= 2 => {
            let v = number_vector(&args[0])?;
            // `<argument type="integer">` with min 0 and max 4 (quartile) or
            // 10 (decile), so the percentage handed on stays within 0–100.
            let (step, max_k) = if fid == id::QUARTILE { (25, 4) } else { (10, 10) };
            let k = integer_in(args.get(1), 0, max_k)?;
            let p = mul(&n(step), &n(k))?;
            let variant = quantile_variant(args.get(2))?;
            percentile(&v, &p, variant).map(M::Number)
        }
        // <expression>quartile(\x,3,\Y{7})-quartile(\x,1,\Y{7})</expression>
        id::IQR => {
            let v = number_vector(args.first()?)?;
            let variant = quantile_variant(args.get(1))?;
            let hi = percentile(&v, &n(75), variant)?;
            let lo = percentile(&v, &n(25), variant)?;
            sub(&hi, &lo).map(M::Number)
        }
        // <expression>dimension(\x)/total(\x.^-1)</expression>
        id::HARMMEAN => {
            let v = vector_arg_all(args)?;
            let mut acc = Number::new();
            for x in &v {
                acc.add(&powi(x, -1)?).then_some(())?;
            }
            div(&n(v.len() as i64), &acc).map(M::Number)
        }
        // <expression>exp(total(ln(\x))/dimension(\x))</expression>
        id::GEOMEAN => {
            let v = vector_arg_all(args)?;
            let mut acc = Number::new();
            for x in &v {
                let mut l = x.clone();
                l.ln().then_some(())?;
                acc.add(&l).then_some(())?;
            }
            let mut r = div(&acc, &n(v.len() as i64))?;
            r.exp().then_some(())?;
            Some(M::Number(r))
        }
        // <expression>abs((total(\x.^2)/dimension(\x))^(1/2))</expression>
        id::RMS => {
            let v = vector_arg_all(args)?;
            let mut acc = Number::new();
            for x in &v {
                acc.add(&powi(x, 2)?).then_some(())?;
            }
            abs_of(&sqrt_of(&div(&acc, &n(v.len() as i64))?)?).map(M::Number)
        }
        id::VAR => variance(&vector_arg_all(args)?, false).map(M::Number),
        id::VARP => variance(&vector_arg_all(args)?, true).map(M::Number),
        // <expression>abs(var(\x)^(1/2))</expression>
        id::STDEV => {
            let v = vector_arg_all(args)?;
            abs_of(&sqrt_of(&variance(&v, false)?)?).map(M::Number)
        }
        id::STDEVP => stdevp(&vector_arg_all(args)?).map(M::Number),
        id::STDERR => stderr_of(&vector_arg_all(args)?).map(M::Number),
        // <expression>total(abs(\x-mean(\x)))/dimension(\x)</expression>
        id::MEANDEV => {
            let v = vector_arg_all(args)?;
            let m = mean(&v)?;
            let mut acc = Number::new();
            for x in &v {
                acc.add(&abs_of(&sub(x, &m)?)?).then_some(())?;
            }
            div(&acc, &n(v.len() as i64)).map(M::Number)
        }
        // <expression>mean(limits(sort(\x),round(dim/100*\y)+1,round(dim/100*(100-\y))))</expression>
        id::TRIMMEAN if args.len() == 2 => {
            let v = number_vector(&args[0])?;
            let p = num(&args[1])?;
            let s = sorted(&v);
            let dim = n(v.len() as i64);
            let per = div(&dim, &n(100))?;
            let lo = rounded(&mul(&per, &p)?)?.checked_add(1)?;
            let hi = rounded(&mul(&per, &sub(&n(100), &p)?)?)?;
            mean(&slice(&s, lo, hi)).map(M::Number)
        }
        // <expression>(element(\1,\2-\3)*\3+element(\1,\3+1)*\3+total(limits(\1,\3+1,\2-\3)))/\2</expression>
        id::WINSORMEAN if args.len() == 2 => {
            let v = number_vector(&args[0])?;
            let p = num(&args[1])?;
            let s = sorted(&v);
            let dim = v.len() as i64;
            let k = rounded(&mul(&div(&n(dim), &n(100))?, &p)?)?;
            // `element(\1,\2-\3)` and `element(\1,\3+1)`: a winsorized share
            // that trims the whole vector puts these indices outside it, so
            // the call stays unevaluated instead of wrapping around.
            let hi_el = nth(&s, dim.checked_sub(k)?)?;
            let lo_el = nth(&s, k.checked_add(1)?)?;
            let kn = n(k);
            let inner = total(&slice(&s, k + 1, dim - k))?;
            let sum = add(&add(&mul(hi_el, &kn)?, &mul(lo_el, &kn)?)?, &inner)?;
            div(&sum, &n(dim)).map(M::Number)
        }
        // <expression>total(\x.*\y)/total(\y)</expression>
        id::WEIGHMEAN if args.len() == 2 => {
            let x = number_vector(&args[0])?;
            let y = number_vector(&args[1])?;
            if x.len() != y.len() {
                return None;
            }
            let mut acc = Number::new();
            for (a, b) in x.iter().zip(&y) {
                acc.add(&mul(a, b)?).then_some(())?;
            }
            div(&acc, &total(&y)?).map(M::Number)
        }
        id::COVAR if args.len() == 2 => {
            covar(&number_vector(&args[0])?, &number_vector(&args[1])?).map(M::Number)
        }
        id::POOLVAR if args.len() == 2 => {
            poolvar(&number_vector(&args[0])?, &number_vector(&args[1])?).map(M::Number)
        }
        // <expression>covar(\x,\y)/(stdevp(\x)*stdevp(\y))</expression>
        id::PEARSON if args.len() == 2 => {
            let x = number_vector(&args[0])?;
            let y = number_vector(&args[1])?;
            div(&covar(&x, &y)?, &mul(&stdevp(&x)?, &stdevp(&y)?)?).map(M::Number)
        }
        // <expression>pearson(rank(\x),rank(\y))</expression>
        id::SPEARMAN if args.len() == 2 => {
            let x = ranks(&number_vector(&args[0])?);
            let y = ranks(&number_vector(&args[1])?);
            div(&covar(&x, &y)?, &mul(&stdevp(&x)?, &stdevp(&y)?)?).map(M::Number)
        }
        // <expression>(mean(\x)-mean(\y))/abs((P/dim(\x)+P/dim(\y))^(1/2))</expression>
        id::TTEST if args.len() == 2 => {
            let x = number_vector(&args[0])?;
            let y = number_vector(&args[1])?;
            let p = poolvar(&x, &y)?;
            let denom = add(
                &div(&p, &n(x.len() as i64))?,
                &div(&p, &n(y.len() as i64))?,
            )?;
            div(&sub(&mean(&x)?, &mean(&y)?)?, &abs_of(&sqrt_of(&denom)?)?).map(M::Number)
        }
        // <expression>mean(\x-\y)/stderr(\x-\y)</expression>
        id::PTTEST if args.len() == 2 => {
            let d = elementwise_sub(&number_vector(&args[0])?, &number_vector(&args[1])?)?;
            div(&mean(&d)?, &stderr_of(&d)?).map(M::Number)
        }
        id::PROBIT if args.len() == 1 => probit(&num(&args[0])?).map(M::Number),
        id::NORMDIST if !args.is_empty() => {
            let x = num(&args[0])?;
            let mu = args.get(1).and_then(num).unwrap_or_else(Number::new);
            let sigma = args.get(2).and_then(num).unwrap_or_else(|| n(1));
            let cumulative = args.get(3).and_then(num).is_some_and(|c| !c.is_zero());
            normdist(&x, &mu, &sigma, cumulative).map(M::Number)
        }
        // <expression>probit(\x)*\Z{1}+\Y{0}</expression>
        id::NORMDISTINV if !args.is_empty() => {
            let p = num(&args[0])?;
            let mu = args.get(1).and_then(num).unwrap_or_else(Number::new);
            let sigma = args.get(2).and_then(num).unwrap_or_else(|| n(1));
            add(&mul(&probit(&p)?, &sigma)?, &mu).map(M::Number)
        }
        // <expression>newtonsolve(1-igamma(\y/2,"x"/2)/gamma(\y/2)=\x,…)</expression>
        id::CHISQDISTINV if args.len() == 2 => {
            let p = num(&args[0])?.float_value();
            let k = num(&args[1])?.float_value();
            let x = gamma_p_inv(k / 2.0, p)?;
            from_f64(x * 2.0).map(M::Number)
        }
        id::FDIST if args.len() >= 3 => {
            let x = num(&args[0])?.float_value();
            let d1 = num(&args[1])?.float_value();
            let d2 = num(&args[2])?.float_value();
            let cumulative = args.get(3).and_then(num).is_some_and(|c| !c.is_zero());
            from_f64(fdist(x, d1, d2, cumulative)?).map(M::Number)
        }
        id::QUADRATICFIT | id::CUBICFIT if !args.is_empty() => {
            let deg = if fid == id::QUADRATICFIT { 2 } else { 3 };
            let first = number_vector(&args[0])?;
            // With one argument the x values are 1..n
            // (`genvector("i",1,dimension(\x),1,"i",1)`).
            let (xs, ys) = match args.get(1).and_then(number_vector) {
                Some(y) => (first, y),
                None => (
                    (1..=first.len() as i64).map(n).collect::<Vec<_>>(),
                    first,
                ),
            };
            if xs.len() != ys.len() || xs.len() <= deg {
                return None;
            }
            Some(polynomial_structure(&polyfit(&xs, &ys, deg)?))
        }
        _ => None,
    }
}

/// `round(x)` as an integer, matching the XML formulas' use of `round`.
fn rounded(x: &Number) -> Option<i64> {
    let mut r = x.clone();
    r.round(qalc_num::options::RoundingMode::HalfAwayFromZero)
        .then_some(())?;
    r.to_i64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    /// Expectations come from `tests/stats.batch` and the reference binary.
    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    /// A session with the transcript's 100-value data vector loaded.
    fn data_session() -> Session {
        set_data_dir(PathBuf::from("/root/Project/libqalculate"));
        let mut s = Session::new();
        s.evaluate_line("v=load(tests/vectordata.csv)").unwrap();
        s.evaluate_line("w=load(tests/vectordata2.csv)").unwrap();
        s
    }

    fn have_data() -> bool {
        std::path::Path::new("/root/Project/libqalculate/tests/vectordata.csv").exists()
    }

    #[test]
    fn mean_and_stdev_of_an_argument_list() {
        // A one-vector function called with several arguments collects them.
        assert_eq!(ev("mean(5; 6; 4; 2; 3; 7)"), "4.5");
        assert_eq!(ev("stdev(5; 6; 4; 2; 3; 7)"), "1.870828693");
    }

    #[test]
    fn percentile_method_8_matches_quartile() {
        assert_eq!(ev("quartile((5; 6; 4; 2; 3; 7); 1; 8)"), "2.916666667");
        assert_eq!(ev("percentile([5 6 4 2 3 7]; 25; 8)"), "2.916666667");
    }

    #[test]
    fn median_and_mode() {
        assert_eq!(ev("mode([1 3 7 5 1 1 1 3])"), "1");
        assert_eq!(ev("median([1 3 7 5 1 1 1 3])"), "2");
    }

    #[test]
    fn normal_distribution_density_and_inverse() {
        assert_eq!(ev("normdist(7; 5)"), "0.05399096651");
        assert_eq!(ev("normdistinv(0.2, 5, 2)"), "3.316757533");
    }

    #[test]
    fn f_distribution_density_and_cumulative() {
        assert_eq!(ev("fdist(5, 2, 3, 0)"), "0.02558260445");
        assert_eq!(ev("fdist(5, 2, 3, 1)"), "0.8891420474");
    }

    #[test]
    fn chi_squared_inverse() {
        assert_eq!(ev("chisqdistinv(0.9, 3)"), "6.251388631");
    }

    #[test]
    fn polynomial_regression() {
        assert_eq!(
            ev("quadraticfit([5 3 4 5 6 7 13 24])"),
            "0.7797619048x^2 - 4.720238095x + 9.732142857"
        );
        assert_eq!(
            ev("cubicfit([5 3 4 5 6 7 13 24])"),
            "0.1489898990x^3 - 1.231601732x^2 + 2.952741703x + 2.357142857"
        );
    }

    #[test]
    fn csv_data_summary_statistics() {
        if !have_data() {
            return;
        }
        let mut s = data_session();
        assert_eq!(s.evaluate_line("mean(v)").unwrap(), "6.530919283");
        assert_eq!(s.evaluate_line("total(v)").unwrap(), "653.0919283");
        assert_eq!(s.evaluate_line("number(v)").unwrap(), "100");
        assert_eq!(s.evaluate_line("min(v)").unwrap(), "-43.38345286");
        assert_eq!(s.evaluate_line("max(v)").unwrap(), "54.40816396");
        assert_eq!(s.evaluate_line("range(v)").unwrap(), "97.79161682");
        assert_eq!(s.evaluate_line("median(v)").unwrap(), "8.084203925");
    }

    #[test]
    fn csv_data_spread_statistics() {
        if !have_data() {
            return;
        }
        let mut s = data_session();
        assert_eq!(s.evaluate_line("stdev(v)").unwrap(), "23.44646004");
        assert_eq!(s.evaluate_line("stderr(v)").unwrap(), "2.344646004");
        assert_eq!(s.evaluate_line("meandev(v)").unwrap(), "19.20169382");
        assert_eq!(s.evaluate_line("rms(v)").unwrap(), "24.22585458");
        assert_eq!(s.evaluate_line("iqr(v)").unwrap(), "33.42899060");
    }

    #[test]
    fn csv_data_quantiles_use_method_7_by_default() {
        if !have_data() {
            return;
        }
        let mut s = data_session();
        assert_eq!(s.evaluate_line("quartile(v, 1, 7)").unwrap(), "-10.48274166");
        assert_eq!(s.evaluate_line("percentile(v, 25, 7)").unwrap(), "-10.48274166");
        assert_eq!(s.evaluate_line("decile(v, 9, 7)").unwrap(), "38.27474287");
    }

    #[test]
    fn csv_data_alternative_means() {
        if !have_data() {
            return;
        }
        let mut s = data_session();
        assert_eq!(s.evaluate_line("geomean(abs(v))").unwrap(), "14.25624271");
        assert_eq!(s.evaluate_line("harmmean(abs(v))").unwrap(), "5.691924037");
        assert_eq!(s.evaluate_line("trimmean(v, 10)").unwrap(), "6.788959652");
        assert_eq!(s.evaluate_line("winsormean(v, 10)").unwrap(), "6.774860902");
        assert_eq!(
            s.evaluate_line("weighmean(v, genvector(2;1;100))").unwrap(),
            "6.530919283"
        );
    }

    #[test]
    fn two_sample_statistics() {
        if !have_data() {
            return;
        }
        let mut s = data_session();
        assert_eq!(s.evaluate_line("ttest(v, w)").unwrap(), "0.3493127334");
        assert_eq!(s.evaluate_line("pttest(v, w)").unwrap(), "1.583214005");
        assert_eq!(s.evaluate_line("pearson(v, w)").unwrap(), "0.9519790480");
        assert_eq!(s.evaluate_line("spearman(v, w)").unwrap(), "0.9742094209");
        assert_eq!(s.evaluate_line("covar(v, w)").unwrap(), "499.1760404");
        assert_eq!(s.evaluate_line("poolvar(v, w)").unwrap(), "530.0195143");
    }

    #[test]
    fn rank_is_the_position_in_sorted_order() {
        let v = vec![n(6), n(1), n(4)];
        let r: Vec<i64> = ranks(&v).iter().map(|x| x.to_i64().unwrap()).collect();
        assert_eq!(r, vec![3, 1, 2]);
    }

    #[test]
    fn variance_is_sample_by_default_and_population_for_varp() {
        // var divides by n-1, varp by n.
        assert_eq!(ev("var(5; 6; 4; 2; 3; 7)"), "3.5");
        assert_eq!(ev("varp(5; 6; 4; 2; 3; 7)"), "2.916666667");
    }

    /// A percentage outside 0–100 fails `percentile`'s `NumberArgument`
    /// range, so the reference leaves the call unevaluated instead of
    /// indexing past the end of the sorted vector.
    #[test]
    fn percentile_rejects_percentages_outside_zero_to_hundred() {
        assert_eq!(ev("percentile([1,2,3], 200, 2)"), "percentile([1  2  3], 200, 2)");
        assert_eq!(ev("percentile([1,2,3], -100, 2)"), "percentile([1  2  3], -100, 2)");
        assert_eq!(ev("percentile([1,2,3], 100.5, 2)"), "percentile([1  2  3], 100.5, 2)");
        assert_eq!(ev("percentile([1,2,3], 1e20, 2)"), "percentile([1  2  3], 1E20, 2)");
        // The endpoints themselves are inside the range.
        assert_eq!(ev("percentile([1,2,3], 0, 2)"), "1");
        assert_eq!(ev("percentile([1,2,3], 100, 2)"), "3");
    }

    /// `quartile`'s second argument is an integer 0–4 and `decile`'s an
    /// integer 0–10; anything else would scale to a percentage out of range.
    #[test]
    fn quartile_and_decile_reject_out_of_range_indices() {
        assert_eq!(ev("quartile([1,2,3], 8, 2)"), "quartile([1  2  3], 8, 2)");
        assert_eq!(ev("quartile([1,2,3], -1, 2)"), "quartile([1  2  3], -1, 2)");
        assert_eq!(ev("quartile([1,2,3], 2.5, 2)"), "quartile([1  2  3], 2.5, 2)");
        assert_eq!(ev("decile([1,2,3], 30, 2)"), "decile([1  2  3], 30, 2)");
        assert_eq!(ev("decile([1,2,3], 11)"), "decile([1  2  3], 11)");
        assert_eq!(ev("quartile([1,2,3], 4, 2)"), "3");
        assert_eq!(ev("decile([1,2,3], 10, 2)"), "3");
    }

    /// The quantile method is an integer 1–9 wherever it is accepted.
    #[test]
    fn quantile_method_must_be_an_integer_one_to_nine() {
        assert_eq!(ev("percentile([1,2,3], 50, 20)"), "percentile([1  2  3], 50, 20)");
        assert_eq!(ev("percentile([1,2,3], 50, 0)"), "percentile([1  2  3], 50, 0)");
        assert_eq!(ev("percentile([1,2,3], 50, 2.5)"), "percentile([1  2  3], 50, 2.5)");
        assert_eq!(ev("iqr([1,2,3], 0)"), "iqr([1  2  3], 0)");
        assert_eq!(ev("iqr([1,2,3], 2.5)"), "iqr([1  2  3], 2.5)");
        assert_eq!(ev("iqr([1,2,3,4,5], 1)"), "2");
    }

    /// A winsorized share that trims the whole vector puts
    /// `element(\1, \2-\3)` outside it; the index must not wrap around.
    #[test]
    fn winsormean_with_a_full_trim_stays_unevaluated() {
        assert_eq!(ev("winsormean([1,2], 100)"), "winsormean([1  2], 100)");
        assert_eq!(ev("winsormean([1], 100)"), "winsormean(1, 100)");
        assert_eq!(ev("winsormean([], 10)"), "winsormean([], 10)");
        assert_eq!(ev("winsormean([1,2,3,4,5,6,7,8,9,10], 20)"), "5.5");
    }

    #[test]
    fn percentile_endpoints_are_the_extremes() {
        assert_eq!(ev("percentile([5 6 4 2 3 7]; 0)"), "2");
        assert_eq!(ev("percentile([5 6 4 2 3 7]; 100)"), "7");
    }

    #[test]
    fn total_and_range_over_a_literal_vector() {
        assert_eq!(ev("total([1 2 3 4])"), "10");
        assert_eq!(ev("range([1 2 3 9])"), "8");
    }

    #[test]
    fn probit_inverts_the_normal_cumulative() {
        // probit(p) = sqrt(2) erfinv(2p - 1); normdistinv(p) with defaults.
        assert_eq!(ev("normdistinv(0.2)"), "-0.8416212336");
        assert_eq!(ev("probit(0.975)"), "1.959963985");
    }

    #[test]
    fn incomplete_functions_agree_with_closed_forms() {
        // I_z(1, b) = 1 - (1 - z)^b
        let z = 0.4;
        let b = 2.5;
        assert!((beta_inc(z, 1.0, b) - (1.0 - (1.0 - z).powf(b))).abs() < 1e-12);
        // P(1, x) = 1 - e^-x
        assert!((gamma_p(1.0, 2.0) - (1.0 - (-2.0f64).exp())).abs() < 1e-12);
    }
}
