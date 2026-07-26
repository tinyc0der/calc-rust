//! Closure of every `Number` operation over infinities, intervals, and
//! complex values with an infinite imaginary part.
//!
//! A port of `test_intervals` (libqalculate `src/test.cc:742-1392`) — the one
//! place in the reference's own testing where degenerate values are pushed
//! through the whole arithmetic and transcendental surface. None of it is
//! reachable from the 17 `.batch` transcripts: those go through the parser, and
//! `setInterval(+infinity, -1)`, or "a real interval whose imaginary part is
//! infinite", have no expression syntax.
//!
//! The C++ builds a vector of 44 edge values — several by *mutating* the
//! previous entry, so construction order is load-bearing — and crosses it with
//! itself through `* + - / ^ log atan2`, with `root(x, 2..9)`, with the
//! integers −3..3 through `+ - * /`, and with 26 unary operations: 16 280
//! evaluations.
//!
//! # The golden file
//!
//! `tests/golden/interval_closure.txt` records the *reference's* answer for
//! every one of them, produced by a throwaway C++ shim linked against the
//! libqalculate build in `/root/Project/libqalculate` — `test.cc`'s vector
//! transcribed statement for statement, `PRECISION` 10, interval arithmetic on,
//! and `PrintOptions` set field by field to match this crate's
//! `PrintOptions::default()` so a difference is never the renderer's. One line
//! per `(value, op, value)` triple, so a diff names the exact failing
//! combination instead of just failing.
//!
//! The shim forks per evaluation, because the reference does not survive its
//! own table: `root(-infinity, 3)` segfaults and five `besselj`/`bessely` rows
//! never return. Those outcomes are recorded as `CRASH` and `HANG`. To rebuild
//! it, walk `test.cc:748-793` into a `vector<Number>`, print each row as
//! `key = canon(result)` with the same `canon` as [`closure_table::canon`], and
//! link with
//! `g++ -I/root/Project/libqalculate -I/usr/include/libxml2 shim.cc
//! libqalculate/.libs/libqalculate.so -lmpfr -lgmp -lpthread`.
//!
//! Results are rendered by [`closure_table::canon`], not by `Number::print` of
//! the whole value: neither of the reference's whole-value interval renderings
//! is usable here. `INTERVAL_DISPLAY_INTERVAL` is not ported, and
//! `INTERVAL_DISPLAY_SIGNIFICANT_DIGITS` prints both `[-1:1]` and `[-2:2]` as
//! `0`, which would hide exactly the widening bugs this suite exists to catch.
//! `canon` prints each endpoint separately, needing only the non-interval print
//! path — the one the 656 transcript assertions already pin.
//!
//! # KNOWN_DIVERGENCES
//!
//! Same contract as `crates/qalc/tests/transcripts.rs`, one size up: 5 178 of
//! the 16 324 rows do not match yet, so the ledger lives in a second golden
//! file (`interval_closure_divergences.txt`, `key<TAB>diagnosis`) rather than
//! in a 5 000-element array literal. A new divergence fails the test; so does a
//! listed one that has started matching. The set can only shrink.
//!
//! Fixing the arithmetic is not this suite's job — keeping the set from growing
//! is. What the ledger says, in rough order of blast radius:
//!
//! * **3 753 rows: `v31`–`v38` were never built.** `Number::set_interval`
//!   returns false for an infinite endpoint, because `is_real()` excludes
//!   ±infinity — so `setInterval(-infinity, -1)` and the seven values after it
//!   silently leave `v30`'s `[-3:-2]` in place, and every row mentioning one of
//!   them is comparing the wrong thing. One bug, a quarter of the table.
//! * **~600 rows: an operand includes an infinity the port refuses.** `*`, `/`,
//!   `log`, `sq` and the integer forms return false against a value whose
//!   imaginary part is infinite (`v11`, `v20`, `v38`); the reference carries the
//!   infinity through componentwise.
//! * **~400 rows: complex interval arithmetic is composed, not computed.**
//!   `z/w` goes through `z·(1/w)` with `1/w = conj(w)/|w|²`, which mentions
//!   `w`'s parts more than once, so the enclosure is wider than the
//!   reference's; `recip`, `tan`, `tanh`, `asin`, `acos`, `acosh`, `atanh` and
//!   `^` on complex intervals lose the same way.
//! * **~350 rows: the port refuses where the reference leaves the real line.**
//!   `sqrt`, `cbrt`, `root`, `ln`, `log`, `asin`, `acos`, `acosh`, `atanh`,
//!   `erf`, `erfc` return false on an interval whose image is complex or
//!   unbounded, instead of continuing into the complex plane.
//! * **125 rows: `^` never returns** on a dyadic base with an integral interval
//!   exponent — see [`closure_table::NON_TERMINATING`].
//! * **63 rows: `atan2` runs past +π** when `y` straddles zero and `x` may be
//!   negative; it stays on one branch instead of widening to `[-π, π]`.
//! * A long tail of individually diagnosed rows: `gamma` evaluated only at the
//!   endpoints (so it misses the minimum near 1.4616), an interval straddling
//!   zero squared to a range that dips below zero, `besselj`/`bessely` stubs,
//!   and the four rows where the *reference* segfaults.
//!
//! # Diagnosing one row
//!
//! ```text
//! QALC_CLOSURE_TRACE=1 QALC_CLOSURE_RANGE=7880,7900 cargo test --release \
//!     -p qalc-num --test interval_closure -- --ignored --nocapture worker
//! ```
//!
//! prints `key<TAB>result` for that slice of the plan and nothing else, naming
//! each row on stderr *before* evaluating it — the only way to identify a row
//! that never returns.

use std::collections::{HashMap, HashSet};

/// The reference's answers, keyed by row.
const GOLDEN: &str = include_str!("golden/interval_closure.txt");

/// Rows where this port disagrees with the reference, and why: one
/// `key<TAB>diagnosis` line each.
///
/// The diagnosis describes the *port's* behaviour. A few are places where the
/// reference is the one misbehaving — it segfaults on `root(-infinity, odd)`
/// and hangs on `besselj` of a wide interval — and the port's answer is the
/// better one. They are still divergences and still listed: the golden file is
/// a record of what the reference does, not a claim about what is correct.
const KNOWN_DIVERGENCES: &str = include_str!("golden/interval_closure_divergences.txt");

mod closure_table {
    use qalc_num::{Number, PrintOptions};

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    /// One real (imaginary-part-free) component: an interval as its two
    /// endpoints, anything else as itself.
    fn part(n: &Number, po: &PrintOptions) -> String {
        if n.is_interval(true) {
            format!(
                "[{}:{}]",
                n.lower_end_point().print(po),
                n.upper_end_point().print(po)
            )
        } else {
            n.print(po)
        }
    }

    /// `real`, or `real+(imag)i` when there is an imaginary part, each
    /// component rendered by [`part`]. Mirrors `canon()` in the C++ shim.
    pub fn canon(n: &Number) -> String {
        let po = PrintOptions::default();
        let mut s = part(&n.real_part(), &po);
        if n.has_imaginary_part() {
            s.push_str("+(");
            s.push_str(&part(&n.imaginary_part(), &po));
            s.push_str(")i");
        }
        s
    }

    pub fn vname(i: usize) -> String {
        format!("v{i:02}")
    }

    // ------------------------------------------------------------------
    // The vector
    // ------------------------------------------------------------------

    fn plus_inf() -> Number {
        let mut n = Number::new();
        n.set_plus_infinity(false, false);
        n
    }

    fn minus_inf() -> Number {
        let mut n = Number::new();
        n.set_minus_infinity(false, false);
        n
    }

    fn q(num: i64, den: i64) -> Number {
        Number::from_ints(num, den, 0)
    }

    /// The 44 values of `test.cc:748-793`, in construction order.
    ///
    /// The order is not just labelling. Entries 9-11, 17-20, 24, 28, 36-38 and
    /// 41 are built by calling `setImaginaryPart` on whatever the previous
    /// statement left in `nr`, and five of those take the imaginary part from
    /// `nrs[nrs.size() - 2]` — an entry pushed two statements earlier.
    /// `setInterval` and `setFloat` clear the imaginary part; `setImaginaryPart`
    /// leaves the real part alone. Transcribed statement for statement.
    pub fn values() -> Vec<Number> {
        let mut nrs: Vec<Number> = Vec::new();
        nrs.push(plus_inf());
        nrs.push(minus_inf());
        nrs.push(q(0, 1));
        nrs.push(q(1, 2));
        nrs.push(q(-1, 2));
        nrs.push(q(1, 1));
        nrs.push(q(-1, 1));
        nrs.push(q(2, 1));

        let mut nr = Number::new();
        // `nrs[nrs.size() - 2]`, cloned out so `nr` can still be mutated.
        macro_rules! back2 {
            () => {
                nrs[nrs.len() - 2].clone()
            };
        }

        nr.set_interval(&q(-1, 2), &q(1, 2), false);
        nrs.push(nr.clone());
        nr.set_imaginary_part(&q(1, 1));
        nrs.push(nr.clone());
        nr.set_imaginary_part(&q(-1, 2));
        nrs.push(nr.clone());
        nr.set_imaginary_part(&plus_inf());
        nrs.push(nr.clone());
        nr.set_interval(&q(-1, 1), &q(1, 2), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(-1, 2), &q(1, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(-1, 1), &q(1, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(-2, 1), &q(2, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(0, 1), &q(1, 2), false);
        nrs.push(nr.clone());
        let im = back2!();
        nr.set_imaginary_part(&im);
        nrs.push(nr.clone());
        nr.set_imaginary_part(&q(1, 1));
        nrs.push(nr.clone());
        nr.set_imaginary_part(&q(-1, 2));
        nrs.push(nr.clone());
        nr.set_imaginary_part(&plus_inf());
        nrs.push(nr.clone());
        nr.set_interval(&q(-1, 2), &q(0, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(0, 1), &q(2, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(1, 2), &q(1, 1), false);
        nrs.push(nr.clone());
        let im = back2!();
        nr.set_imaginary_part(&im);
        nrs.push(nr.clone());
        nr.set_interval(&q(1, 1), &q(2, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(2, 1), &q(3, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(-1, 1), &q(-1, 2), false);
        nrs.push(nr.clone());
        let im = back2!();
        nr.set_imaginary_part(&im);
        nrs.push(nr.clone());
        nr.set_interval(&q(-1, 1), &q(-2, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&q(-2, 1), &q(-3, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&minus_inf(), &q(-1, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&minus_inf(), &q(1, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&minus_inf(), &plus_inf(), false);
        nrs.push(nr.clone());
        nr.set_interval(&plus_inf(), &q(-1, 1), false);
        nrs.push(nr.clone());
        nr.set_interval(&plus_inf(), &q(1, 1), false);
        nrs.push(nr.clone());
        let im = back2!();
        nr.set_imaginary_part(&im);
        nrs.push(nr.clone());
        nr.set_imaginary_part(&q(1, 1));
        nrs.push(nr.clone());
        nr.set_imaginary_part(&minus_inf());
        nrs.push(nr.clone());
        nr.set_float(0.5);
        nrs.push(nr.clone());
        nr.set_float(-0.5);
        nrs.push(nr.clone());
        // test.cc assigns the imaginary part twice with no push between, so
        // the first assignment is dead. Kept, so the transcription stays
        // literal and the next reader does not "fix" it back.
        let im = back2!();
        nr.set_imaginary_part(&im);
        nr.set_imaginary_part(&q(1, 1));
        nrs.push(nr.clone());
        nr.set_float(1.5);
        nrs.push(nr.clone());
        nr.set_float(-1.5);
        nrs.push(nr.clone());

        nrs
    }

    // ------------------------------------------------------------------
    // The cross product
    // ------------------------------------------------------------------

    /// The binary operations, in `test.cc` order.
    const BINARY: [&str; 7] = ["*", "+", "-", "/", "^", "log", "atan2"];
    /// The integer-operand operations. The `i` suffix keeps their rows
    /// distinct from the `Number`-operand ones: the reference implements
    /// `add(long int)` separately from `add(const Number &)`, and they do not
    /// always agree.
    const INTEGER: [&str; 4] = ["+i", "-i", "*i", "/i"];
    const UNARY: [&str; 26] = [
        "recip", "neg", "abs", "sq", "sqrt", "cbrt", "sin", "asin", "sinh", "asinh", "cos",
        "acos", "cosh", "acosh", "tan", "atan", "tanh", "atanh", "ln", "gamma", "digamma",
        "besselj", "bessely", "erf", "erfc", "arg",
    ];

    /// One row of the table, as indices rather than values, so the plan can be
    /// built and sliced without evaluating anything.
    #[derive(Clone, Copy)]
    pub enum Job {
        /// The rendering of `values()[i]` itself.
        Value(usize),
        /// `BINARY[op]` applied to `(values()[i], values()[j])`.
        Bin { op: usize, i: usize, j: usize },
        /// `root(values()[i], k)`.
        Root { i: usize, k: i64 },
        /// `INTEGER[op]` applied to `(values()[i], k)`.
        Int { op: usize, i: usize, k: i64 },
        /// `UNARY[op]` applied to `values()[i]`.
        Un { op: usize, i: usize },
    }

    /// Rows this port does not *finish*.
    ///
    /// A row listed here is never evaluated. Unlike a panic, a computation that
    /// never returns cannot be caught, and Rust cannot cancel a running thread,
    /// so one of these would take the whole suite with it — each was confirmed
    /// still running after two minutes in a release build. They are reported as
    /// `NONTERMINATING`, and since the reference answers every one of them,
    /// they are all divergences and all carry a diagnosis in the ledger.
    ///
    /// Every one is `^` with a base whose endpoints are dyadic against an
    /// exponent interval with an integral endpoint (`[-2:2]`, `[0:2]`,
    /// `[1:2]`, `[2:3]`, `[-2:-1]`, `[-3:-2]`, and the four stale values that
    /// also hold `[-3:-2]`). `raise_impl`'s interval branch hands those to
    /// astro-float's `pow` with directed rounding; its Ziv refinement never
    /// settles when the true result is exactly representable. `pow.rs` already
    /// knows about this for the *non*-interval branch and works around it
    /// there — the interval branch has the same problem and no workaround.
    ///
    /// The C++ shim meets the mirror image of this and forks per evaluation, so
    /// the reference's own five `besselj`/`bessely` hangs are recorded as
    /// `HANG` in the golden file instead of ending the run.
    #[rustfmt::skip]
    pub const NON_TERMINATING: &[&str] = &[
        "v03 ^ v15", "v03 ^ v22", "v03 ^ v25", "v03 ^ v26", "v03 ^ v29", "v03 ^ v30",
        "v03 ^ v31", "v03 ^ v32", "v03 ^ v33", "v03 ^ v34", "v03 ^ v35",
        "v07 ^ v15", "v07 ^ v22", "v07 ^ v25", "v07 ^ v26", "v07 ^ v29", "v07 ^ v30",
        "v07 ^ v31", "v07 ^ v32", "v07 ^ v33", "v07 ^ v34", "v07 ^ v35",
        "v08 ^ v15", "v08 ^ v22", "v08 ^ v25", "v08 ^ v26", "v08 ^ v29", "v08 ^ v30",
        "v08 ^ v31", "v08 ^ v32", "v08 ^ v33", "v08 ^ v34", "v08 ^ v35",
        "v12 ^ v15", "v12 ^ v22", "v12 ^ v25", "v12 ^ v26", "v12 ^ v29", "v12 ^ v30",
        "v12 ^ v31", "v12 ^ v32", "v12 ^ v33", "v12 ^ v34", "v12 ^ v35",
        "v15 ^ v15", "v15 ^ v22", "v15 ^ v25", "v15 ^ v26", "v15 ^ v29", "v15 ^ v30",
        "v15 ^ v31", "v15 ^ v32", "v15 ^ v33", "v15 ^ v34", "v15 ^ v35",
        "v16 ^ v15", "v16 ^ v22", "v16 ^ v25", "v16 ^ v26", "v16 ^ v29", "v16 ^ v30",
        "v16 ^ v31", "v16 ^ v32", "v16 ^ v33", "v16 ^ v34", "v16 ^ v35",
        "v22 ^ v15", "v22 ^ v22", "v22 ^ v25", "v22 ^ v26", "v22 ^ v29", "v22 ^ v30",
        "v22 ^ v31", "v22 ^ v32", "v22 ^ v33", "v22 ^ v34", "v22 ^ v35",
        "v23 ^ v15", "v23 ^ v22", "v23 ^ v25", "v23 ^ v26", "v23 ^ v29", "v23 ^ v30",
        "v23 ^ v31", "v23 ^ v32", "v23 ^ v33", "v23 ^ v34", "v23 ^ v35",
        "v25 ^ v15", "v25 ^ v22", "v25 ^ v25", "v25 ^ v26", "v25 ^ v29", "v25 ^ v30",
        "v25 ^ v31", "v25 ^ v32", "v25 ^ v33", "v25 ^ v34", "v25 ^ v35",
        "v26 ^ v15", "v26 ^ v22", "v26 ^ v25", "v26 ^ v26", "v26 ^ v29", "v26 ^ v30",
        "v26 ^ v31", "v26 ^ v32", "v26 ^ v33", "v26 ^ v34", "v26 ^ v35",
        "v39 ^ v15", "v39 ^ v22", "v39 ^ v25", "v39 ^ v26", "v39 ^ v29", "v39 ^ v30",
        "v39 ^ v31", "v39 ^ v32", "v39 ^ v33", "v39 ^ v34", "v39 ^ v35",
        "v42 ^ v15", "v42 ^ v22", "v42 ^ v25", "v42 ^ v26",
    ];

    /// Every row, as `(key, job)`, in the golden file's order: the 44 value
    /// renderings first, then the 16 280 operation rows.
    pub fn plan() -> Vec<(String, Job)> {
        let n = 44;
        let mut out: Vec<(String, Job)> = Vec::with_capacity(16_324);

        for i in 0..n {
            out.push((vname(i), Job::Value(i)));
        }
        for (op, name) in BINARY.iter().enumerate() {
            for i in 0..n {
                for j in 0..n {
                    out.push((
                        format!("{} {} {}", vname(i), name, vname(j)),
                        Job::Bin { op, i, j },
                    ));
                }
            }
        }
        for i in 0..n {
            for k in 2..10i64 {
                out.push((format!("{} root {}", vname(i), k), Job::Root { i, k }));
            }
        }
        // test.cc runs the `+` loop over the integers twice, byte-identically.
        // The duplicate is dropped: a duplicated golden row localizes nothing.
        for (op, name) in INTEGER.iter().enumerate() {
            for i in 0..n {
                for k in -3..=3i64 {
                    out.push((
                        format!("{} {} {}", vname(i), name, k),
                        Job::Int { op, i, k },
                    ));
                }
            }
        }
        for (op, name) in UNARY.iter().enumerate() {
            for i in 0..n {
                out.push((format!("{} {}", name, vname(i)), Job::Un { op, i }));
            }
        }
        out
    }

    /// Evaluate one job. A `false` return or a panic becomes a recorded
    /// outcome rather than an abort — the C++ shim forks per row for the same
    /// reason, since one bad combination must not cost the other 16 323.
    pub fn run(nrs: &[Number], job: Job) -> String {
        let one = Number::from_i64(1);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut n;
            let ok = match job {
                Job::Value(i) => return canon(&nrs[i]),
                Job::Bin { op, i, j } => {
                    n = nrs[i].clone();
                    let b = &nrs[j];
                    match op {
                        0 => n.multiply(b),
                        1 => n.add(b),
                        2 => n.subtract(b),
                        3 => n.divide(b),
                        4 => n.raise(b, true),
                        5 => n.log(b),
                        _ => n.atan2(b, false),
                    }
                }
                Job::Root { i, k } => {
                    n = nrs[i].clone();
                    n.root(&Number::from_i64(k))
                }
                Job::Int { op, i, k } => {
                    n = nrs[i].clone();
                    match op {
                        0 => n.add_i64(k),
                        // There is no `subtract_i64`; the reference's
                        // `subtract(long int)` is a distinct implementation
                        // from `subtract(const Number &)`, so any divergence
                        // on these rows is a real one.
                        1 => n.subtract(&Number::from_i64(k)),
                        2 => n.multiply_i64(k),
                        _ => n.divide_i64(k),
                    }
                }
                Job::Un { op, i } => {
                    n = nrs[i].clone();
                    match op {
                        0 => n.recip(),
                        1 => n.negate(),
                        2 => n.abs(),
                        3 => n.square(),
                        4 => n.sqrt(),
                        5 => n.cbrt(),
                        6 => n.sin(),
                        7 => n.asin(),
                        8 => n.sinh(),
                        9 => n.asinh(),
                        10 => n.cos(),
                        11 => n.acos(),
                        12 => n.cosh(),
                        13 => n.acosh(),
                        14 => n.tan(),
                        15 => n.atan(),
                        16 => n.tanh(),
                        17 => n.atanh(),
                        18 => n.ln(),
                        19 => n.gamma(),
                        20 => n.digamma(),
                        21 => n.besselj(&one),
                        22 => n.bessely(&one),
                        23 => n.erf(),
                        24 => n.erfc(),
                        _ => n.arg(),
                    }
                }
            };
            if ok {
                canon(&n)
            } else {
                "FAILED".to_string()
            }
        }));
        r.unwrap_or_else(|_| "PANIC".to_string())
    }

    /// How long a single row may take before it is abandoned. Generous: it is
    /// a backstop for a *new* non-terminating row, not a performance budget —
    /// the known ones are skipped outright, and the slowest row that does
    /// finish is three orders of magnitude under this even in a debug build.
    const ROW_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

    /// Run one job with a deadline, so a row that never returns is reported
    /// instead of wedging the suite. There is no way to cancel a running
    /// thread in Rust, so a timed-out row leaves its worker spinning — which
    /// is exactly why [`NON_TERMINATING`] exists: the known offenders are
    /// never started, and this stays a backstop that fires zero times.
    fn run_guarded(nrs: &std::sync::Arc<Vec<Number>>, job: Job) -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        let nrs = std::sync::Arc::clone(nrs);
        std::thread::spawn(move || {
            let _ = tx.send(run(&nrs, job));
        });
        rx.recv_timeout(ROW_DEADLINE)
            .unwrap_or_else(|_| "TIMEOUT".to_string())
    }

    /// Run a slice of the plan, in order, tracing each key first when
    /// `QALC_CLOSURE_TRACE` is set — a row that never returns is then named
    /// before it hangs, which is the only way to find one in the first place.
    pub fn run_range(plan: &[(String, Job)], trace: bool) -> Vec<String> {
        let nrs = std::sync::Arc::new(values());
        plan.iter()
            .map(|(key, job)| {
                if trace {
                    eprintln!("{key}");
                }
                if NON_TERMINATING.contains(&key.as_str()) {
                    return "NONTERMINATING".to_string();
                }
                run_guarded(&nrs, *job)
            })
            .collect()
    }

    /// The whole table. Split across the available cores: 16 324 evaluations
    /// at 133 bits of working precision are not cheap, and the rows are
    /// independent (`qalc_num::context` state is thread-local, and a fresh
    /// thread starts at the same defaults this table assumes: precision 10,
    /// interval arithmetic on).
    pub fn rows() -> Vec<(String, String)> {
        let plan = plan();
        let trace = std::env::var_os("QALC_CLOSURE_TRACE").is_some();
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
        let chunk = plan.len().div_ceil(workers);

        let results: Vec<Vec<String>> = std::thread::scope(|s| {
            let handles: Vec<_> = plan
                .chunks(chunk)
                .map(|c| s.spawn(move || run_range(c, trace)))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        plan.into_iter()
            .map(|(k, _)| k)
            .zip(results.into_iter().flatten())
            .collect()
    }
}

/// Parse the golden file into `(key, value)` pairs, in file order.
fn golden_rows() -> Vec<(&'static str, &'static str)> {
    GOLDEN
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.split_once(" = ")
                .unwrap_or_else(|| panic!("malformed golden line: {l}"))
        })
        .collect()
}

/// Parse the divergence ledger into `key -> diagnosis`.
fn known_divergences() -> HashMap<&'static str, &'static str> {
    KNOWN_DIVERGENCES
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.split_once('\t')
                .unwrap_or_else(|| panic!("malformed divergence line: {l}"))
        })
        .collect()
}

/// Generated rows, computed once and shared by every test in this file.
fn ours() -> &'static [(String, String)] {
    static ROWS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        // Panics are an outcome here, not a bug report: `run` catches them so
        // one bad combination cannot take down 16 324 rows. Silence the hook
        // so a panicking row does not bury the diff under backtraces.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let rows = closure_table::rows();
        std::panic::set_hook(hook);
        rows
    })
}

#[test]
fn every_value_and_operation_matches_the_reference() {
    let ours = ours();
    let theirs = golden_rows();

    assert_eq!(
        ours.len(),
        theirs.len(),
        "the table changed shape: {} rows generated, {} in the golden file. \
         Regenerate the golden file from the C++ shim rather than editing it.",
        ours.len(),
        theirs.len()
    );

    let known = known_divergences();
    let mut unexpected: Vec<String> = Vec::new();
    let mut matching: HashSet<&str> = HashSet::new();

    for ((key, got), (gkey, want)) in ours.iter().zip(theirs.iter()) {
        assert_eq!(
            key, gkey,
            "row order diverged from the golden file at {key} (golden has {gkey})"
        );
        if got == want {
            matching.insert(key.as_str());
        } else if !known.contains_key(key.as_str()) {
            unexpected.push(format!(
                "{key}\n    reference: {want}\n    ours:      {got}"
            ));
        }
    }

    let mut fixed: Vec<String> = known
        .iter()
        .filter(|(k, _)| matching.contains(*k))
        .map(|(k, why)| format!("{k}  ({why})"))
        .collect();
    fixed.sort();

    assert!(
        unexpected.is_empty(),
        "{} new divergence(s) from the reference:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
    assert!(
        fixed.is_empty(),
        "{} row(s) now match the reference — remove them from \
         KNOWN_DIVERGENCES so the set cannot creep back up:\n{}",
        fixed.len(),
        fixed.join("\n")
    );
}

/// The vector itself, before any operation touches it.
///
/// Split out and named loudly because a divergence here mistrains everything
/// downstream: `v34` is not `[-1:+infinity]` in this port, so none of the ~670
/// rows that mention it is comparing what it claims to compare. Exactly eight
/// values are wrong — the eight `set_interval` refuses to build — and this
/// pins that number, so the contamination cannot spread quietly.
#[test]
fn only_the_eight_half_infinite_values_are_built_differently() {
    let values = closure_table::values();
    assert_eq!(values.len(), 44, "test.cc builds 44 values");

    let theirs = golden_rows();
    let known = known_divergences();
    let mut wrong: Vec<String> = Vec::new();
    let mut unlisted: Vec<String> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let (gkey, want) = theirs[i];
        assert_eq!(closure_table::vname(i), gkey);
        let got = closure_table::canon(v);
        if got != want {
            wrong.push(gkey.to_string());
            if !known.contains_key(gkey) {
                unlisted.push(format!("{gkey}: reference {want}, ours {got}"));
            }
        }
    }
    assert!(
        unlisted.is_empty(),
        "an edge value diverges without a diagnosis:\n{}",
        unlisted.join("\n")
    );
    assert_eq!(
        wrong,
        ["v31", "v32", "v33", "v34", "v35", "v36", "v37", "v38"],
        "the set of mis-built edge values changed"
    );
}

/// Guard on the size of the cross product, so a refactor that quietly drops a
/// loop cannot leave the suite green while testing a fraction of the surface.
#[test]
fn the_table_covers_the_whole_cross_product() {
    let n = 44;
    let expected = n                    // the value renderings
        + 7 * n * n                     // * + - / ^ log atan2
        + n * 8                         // root(x, 2..9)
        + 4 * n * 7                     // + - * / against -3..3
        + 26 * n; // the unary set
    assert_eq!(expected, 16_324);
    assert_eq!(closure_table::plan().len(), expected);
}

/// The ledger has to stay honest on its own terms, independently of whether the
/// arithmetic agrees: every entry names a row that exists, carries a real
/// diagnosis rather than a placeholder, and appears once.
#[test]
fn the_divergence_ledger_is_well_formed() {
    let mut seen: HashSet<&str> = HashSet::new();
    let plan_keys: HashSet<String> =
        closure_table::plan().into_iter().map(|(k, _)| k).collect();
    for line in KNOWN_DIVERGENCES
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let (key, why) = line.split_once('\t').expect("`key<TAB>diagnosis`");
        assert!(
            plan_keys.contains(key),
            "ledger names a row that is not in the table: {key}"
        );
        assert!(seen.insert(key), "ledger lists {key} twice");
        assert!(
            why.len() > 20 && !why.contains("TODO") && !why.contains("???"),
            "ledger entry for {key} is not a diagnosis: {why:?}"
        );
    }
    assert_eq!(seen.len(), 5178, "the recorded divergence count changed");

    for key in closure_table::NON_TERMINATING {
        assert!(
            plan_keys.contains(*key),
            "NON_TERMINATING names a row that is not in the table: {key}"
        );
        assert!(
            seen.contains(key),
            "{key} is skipped as non-terminating but has no ledger entry — a \
             skipped row can never match the reference, so it is a divergence"
        );
    }
}

/// Prints `key<TAB>result` for `QALC_CLOSURE_RANGE=start,end` (default: the
/// whole plan) and nothing else. Not part of the suite — it exists to
/// regenerate this file's expectations and to find rows that never return:
///
/// ```text
/// QALC_CLOSURE_TRACE=1 QALC_CLOSURE_RANGE=7880,7900 cargo test --release \
///     -p qalc-num --test interval_closure -- --ignored --nocapture worker
/// ```
#[test]
#[ignore = "diagnostic tool, not an assertion"]
fn worker() {
    let plan = closure_table::plan();
    let (start, end) = match std::env::var("QALC_CLOSURE_RANGE") {
        Ok(v) => {
            let (a, b) = v.split_once(',').expect("QALC_CLOSURE_RANGE=start,end");
            (a.parse().unwrap(), b.parse::<usize>().unwrap().min(plan.len()))
        }
        Err(_) => (0, plan.len()),
    };
    let slice = &plan[start..end];
    let trace = std::env::var_os("QALC_CLOSURE_TRACE").is_some();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let results = closure_table::run_range(slice, trace);
    std::panic::set_hook(hook);
    for ((key, _), value) in slice.iter().zip(results) {
        println!("{key}\t{value}");
    }
}
