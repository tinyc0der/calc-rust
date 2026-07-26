//! Self-consistency properties of the calculus engine, ported from
//! `src/test.cc`.
//!
//! | module | test.cc | claim |
//! |---|---|---|
//! | [`integrate_differentiate_roundtrip`] | `:576`-`:740` | `d/dx ∫f dx` agrees with `f`, pointwise |
//!
//! See `crates/qalc-core/tests/evaluation_invariants.rs` for the four
//! properties that came before this one, and for why "port libqalculate's C++
//! unit-test assertions" turned into "port the self-consistency properties
//! `test.cc` encodes": libqalculate has no `assert()` anywhere, `unittest.cc`
//! is a directory walker, and `test.cc` is a `main()` that prints counters and
//! is excluded from `make check`.
//!
//! This file's contribution is the only property in the set that needs no
//! oracle **and** checks a value rather than a spelling. `integrate.rs` is
//! 1 830 lines carrying 20 unit tests and 24 transcript lines; a rule with a
//! wrong constant, a missing chain-rule factor, or a sign error in a
//! substitution is invisible to all of them unless somebody happened to write
//! a transcript for that exact integrand. Differentiating the answer needs no
//! reference at all.
//!
//! # What it found
//!
//! Over the 2 280 integrands `test.cc` builds (20 bases × 19 wrappers × 6
//! shapes), evaluated at `x = 3` and `x = -5` — 4 560 integrand/point pairs:
//!
//! | | count |
//! |---|---|
//! | held | 599 |
//! | violated | 3 |
//! | out of scope: `integrate.rs` has no rule | 1 598 |
//! | out of scope: the answer contains `abs()`, which `differentiate.rs` cannot differentiate | 56 |
//! | out of scope: the answer is non-elementary (`Si`/`Ci`/…) | 24 |
//! | timed out | 0 |
//!
//! **The three violations are not wrong integrals.** All three are
//! `acos(u)` with `u` monic in `x`, and all three are the *evaluator* failing
//! to cancel `A - A` in a single pass; see
//! [`integrate_differentiate_roundtrip::KNOWN_VIOLATIONS`]. No integrand in
//! the corpus produced an antiderivative whose derivative had the wrong value.
//!
//! **The 1 598 skips are the real finding**, and the driver prints them broken
//! down by wrapper so the number is not opaque: `tan(@)` and `acosh(@)` are
//! 0/120, `ln(@)` / `asin(@)` / `atan(@)` / `atanh(@)` and the rest of the
//! inverse family are 9/120 (only the shape that applies no multiplier), and
//! the power wrappers run 51-79/120. Measured against the reference on the
//! same 2 280 lines —
//!
//! ```text
//! TMPHOME=$(mktemp -d)
//! printf 'integrate(EXPR, x)\n' | (cd /root/Project/libqalculate && \
//!     env HOME=$TMPHOME QALCULATE_DEFINITIONS_DIR=$PWD/data ./src/qalc -t +u8 --defaults)
//! ```
//!
//! — libqalculate declines 796 and answers 1 446, where this port answers 682.
//! Integration by parts against a polynomial factor (`ln(4x+5) · x`) is the
//! largest single gap. None of that is this file's to fix; recording it is the
//! point, because until now nothing measured it.
//!
//! # Runtime
//!
//! About nine seconds. Each case runs on a worker thread under a wall-clock
//! cap ([`timeout_ms`]), so an integrand that never returns is reported by name
//! instead of hanging the suite; none does today. `integrate.rs`'s documented
//! numeric refusals (`Shi`/`Chi` decline `|z| > 60`, `fresnel` declines
//! `|x| > 12`) never fire here, because the corpus is integrated symbolically
//! and only evaluated at `x = 3` and `x = -5`.

use std::collections::BTreeSet;

use qalc_core::options::{ApproximationMode, EvaluationOptions};
use qalc_core::structure::MathStructure;
use qalc_core::{parser, Session};
use qalc_num::Number;

// =====================================================================
// harness
// =====================================================================

/// Running one case without letting it take the suite down with it.
///
/// Transcribed from `evaluation_invariants.rs`, and load-bearing for the same
/// two reasons: a case can panic, and a case can fail to terminate. Both are
/// recorded as results rather than ending the run. `integrate.rs`'s partial
/// fractions and `by parts` recursion are the plausible non-terminating paths
/// here, and the corpus deliberately feeds them expressions nobody has run
/// before.
mod harness {
    use super::*;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

    /// What one case did.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Verdict {
        /// The property was checked and held.
        Held,
        /// The case is out of scope; the string says why.
        Skipped(String),
        /// The property was violated; the string says how.
        Violation(String),
    }

    impl Verdict {
        pub fn violated(msg: impl Into<String>) -> Verdict {
            Verdict::Violation(msg.into())
        }
        pub fn skipped(reason: impl Into<String>) -> Verdict {
            Verdict::Skipped(reason.into())
        }
    }

    type Job = Box<dyn FnOnce() -> Verdict + Send>;

    /// A single long-lived worker thread that cases are fed to one at a time,
    /// retired whenever one times out or panics — after either, the
    /// thread-locals (`qalc_num::context`, `solve::SOLVING`) are untrustworthy.
    pub struct Runner {
        tx: Option<Sender<Job>>,
        rx: Option<Receiver<Verdict>>,
        pub timeouts: Vec<String>,
    }

    impl Default for Runner {
        fn default() -> Self {
            Runner::new()
        }
    }

    impl Runner {
        pub fn new() -> Runner {
            let mut r = Runner {
                tx: None,
                rx: None,
                timeouts: Vec::new(),
            };
            r.spawn();
            r
        }

        fn spawn(&mut self) {
            let (job_tx, job_rx) = mpsc::channel::<Job>();
            let (out_tx, out_rx) = mpsc::channel::<Verdict>();
            std::thread::Builder::new()
                .name("calculus-worker".into())
                // The antiderivatives this corpus produces nest deeply; the
                // default 2 MiB overflows on the `w * w` shapes.
                .stack_size(64 * 1024 * 1024)
                .spawn(move || {
                    while let Ok(job) = job_rx.recv() {
                        reset_context();
                        let verdict =
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                                Ok(v) => v,
                                Err(payload) => {
                                    Verdict::violated(format!("PANIC: {}", panic_message(&payload)))
                                }
                            };
                        if out_tx.send(verdict).is_err() {
                            return;
                        }
                    }
                })
                .expect("worker thread spawns");
            self.tx = Some(job_tx);
            self.rx = Some(out_rx);
        }

        /// Run `job`, giving it at most `timeout_ms` of wall clock.
        pub fn run(
            &mut self,
            id: &str,
            timeout_ms: u64,
            job: impl FnOnce() -> Verdict + Send + 'static,
        ) -> Verdict {
            if self
                .tx
                .as_ref()
                .expect("worker alive")
                .send(Box::new(job))
                .is_err()
            {
                self.spawn();
                return Verdict::violated("worker thread died before the case ran");
            }
            let got = self
                .rx
                .as_ref()
                .expect("worker alive")
                .recv_timeout(std::time::Duration::from_millis(timeout_ms));
            match got {
                Ok(Verdict::Violation(m)) if m.starts_with("PANIC: ") => {
                    self.retire();
                    Verdict::Violation(m)
                }
                Ok(v) => v,
                Err(RecvTimeoutError::Timeout) => {
                    self.timeouts.push(id.to_string());
                    self.retire();
                    Verdict::violated(format!("TIMEOUT after {timeout_ms} ms"))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.spawn();
                    Verdict::violated("worker thread aborted the process-wide unwind")
                }
            }
        }

        /// Abandon the current worker and start a clean one. A hung worker is
        /// leaked deliberately: there is no way to kill a thread in safe Rust,
        /// and joining it would *be* the hang.
        fn retire(&mut self) {
            self.tx = None;
            self.rx = None;
            self.spawn();
        }
    }

    fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        }
    }

    pub fn reset_context() {
        qalc_num::context::set_precision(qalc_num::context::DEFAULT_PRECISION);
        qalc_num::context::set_create_interval(true);
        qalc_num::context::set_interval_calculation(
            qalc_num::context::IntervalCalculation::VarianceFormula,
        );
    }

    thread_local! {
        /// One session per worker thread — `Session::new` is what makes `sin`,
        /// `sqrt` and `cbrt` resolve to functions rather than bare symbols.
        static SESSION: std::cell::RefCell<Option<Session>> =
            const { std::cell::RefCell::new(None) };
    }

    pub fn with_session<R>(f: impl FnOnce(&Session) -> R) -> R {
        SESSION.with(|cell| {
            let mut cell = cell.borrow_mut();
            let s = cell.get_or_insert_with(Session::new);
            f(s)
        })
    }

    pub fn parse(expr: &str) -> Result<MathStructure, String> {
        with_session(|s| parser::parse_with(expr, &s.parse_options, s).map_err(|e| e.to_string()))
    }

    /// `MathStructure::eval(eo)` with `test_integration6`'s options.
    ///
    /// `test.cc:580`-`:582` sets `angle_unit = ANGLE_UNIT_RADIANS` and
    /// `assume_denominators_nonzero = true` and leaves everything else at the
    /// `EvaluationOptions` default, which is `APPROXIMATION_TRY_EXACT`. This
    /// port has no angle-unit setting — `sin` is radians unconditionally — and
    /// `assume_denominators_nonzero` already defaults to `true`, so the C++'s
    /// `eo` *is* `EvaluationOptions::default()` here.
    pub fn evaluate_in_place(m: &mut MathStructure) {
        let eo = EvaluationOptions::default();
        qalc_core::eval::evaluate_calculated_with(m, &eo);
    }

    /// The same under `APPROXIMATION_APPROXIMATE`. Not used by the property
    /// itself — `test_integration6` evaluates everything with one `eo` — but
    /// needed by [`super::the_check_rejects_a_wrong_constant_factor`], which
    /// has to reduce both sides to numbers to compare them.
    pub fn evaluate_approx_in_place(m: &mut MathStructure) {
        let eo = EvaluationOptions {
            approximation: ApproximationMode::Approximate,
            ..EvaluationOptions::default()
        };
        qalc_core::eval::evaluate_calculated_with(m, &eo);
    }

    pub fn print(m: &MathStructure) -> String {
        with_session(|s| qalc_core::print::print(m, &s.print_options))
    }
}

// =====================================================================
// shared predicates (same definitions as evaluation_invariants.rs)
// =====================================================================

/// `MathStructure::containsInfinity` — the first of `test.cc`'s two documented
/// exclusions.
fn contains_infinity(m: &MathStructure) -> bool {
    match m {
        MathStructure::Number(n) => n.includes_infinity(),
        MathStructure::Undefined => true,
        MathStructure::Power { base, exponent } => {
            contains_infinity(base) || contains_infinity(exponent)
        }
        MathStructure::Comparison { left, right, .. } => {
            contains_infinity(left) || contains_infinity(right)
        }
        _ => m.children().any(contains_infinity),
    }
}

/// `contains_division_by_zero` — the second exclusion.
fn contains_division_by_zero(m: &MathStructure) -> bool {
    if let MathStructure::Power { base, exponent } = m {
        if base.is_zero() {
            if let MathStructure::Number(e) = &**exponent {
                if e.is_negative() {
                    return true;
                }
            }
        }
    }
    match m {
        MathStructure::Power { base, exponent } => {
            contains_division_by_zero(base) || contains_division_by_zero(exponent)
        }
        _ => m.children().any(contains_division_by_zero),
    }
}

fn excluded(a: &MathStructure, b: &MathStructure) -> bool {
    contains_infinity(a)
        || contains_infinity(b)
        || contains_division_by_zero(a)
        || contains_division_by_zero(b)
        || a.is_aborted()
        || b.is_aborted()
}

/// `|z|` as an `f64`, for a possibly-complex `Number`.
fn magnitude(n: &Number) -> f64 {
    let re = n.real_part().float_value();
    let im = n.imaginary_part().float_value();
    (re * re + im * im).sqrt()
}

/// Two `Number`s agreeing to a relative `1e-9` — the same tolerance
/// `evaluation_invariants.rs` uses, standing in for the C++'s
/// interval-overlap `MathStructure::compare`, at the default 10 significant
/// digits.
fn numbers_agree(a: &MathStructure, b: &MathStructure) -> bool {
    let (MathStructure::Number(x), MathStructure::Number(y)) = (a, b) else {
        return false;
    };
    let (mx, my) = (magnitude(x), magnitude(y));
    if !mx.is_finite() || !my.is_finite() {
        return false;
    }
    let mut d = x.clone();
    if !d.subtract(y) {
        return false;
    }
    magnitude(&d) <= 1e-9 * mx.max(my).max(1.0)
}

/// The comparison `test_integration6` makes, with two deliberate relaxations.
///
/// The C++ compares `mstruct3.print(...)` strings outright, and that is the
/// first thing tried here. It is not sufficient on its own, because under
/// `APPROXIMATION_TRY_EXACT` the two sides routinely stop at different points
/// on the exact-to-approximate scale: `d/dx (2/3)x^(3/2)` at `x = 3` comes back
/// as `sqrt(3)` while `sqrt(x)` at `x = 3` comes back as `1.732050808`. Those
/// are one value in two spellings, and calling them a violated integral would
/// bury the real defects under thirty of them.
///
/// So the last resort is to push both sides through
/// `APPROXIMATION_APPROXIMATE` and compare the numbers. That is *weaker* than
/// the C++'s string equality in exactly one direction — it stops caring how
/// the answer is spelled — and unchanged in the direction that matters, which
/// is whether `d/dx ∫f dx` is the same *value* as `f`. It is also why
/// [`the_check_rejects_a_wrong_constant_factor`] and
/// [`the_check_rejects_a_missing_chain_rule_factor`] exist: a comparison that
/// has been relaxed twice has to be shown to still reject something.
fn results_agree(a: &MathStructure, b: &MathStructure) -> bool {
    if a.equals(b) || numbers_agree(a, b) {
        return true;
    }
    if harness::print(a) == harness::print(b) {
        return true;
    }
    let mut na = a.clone();
    let mut nb = b.clone();
    harness::evaluate_approx_in_place(&mut na);
    harness::evaluate_approx_in_place(&mut nb);
    numbers_agree(&na, &nb) || harness::print(&na) == harness::print(&nb)
}

// =====================================================================
// P5
// =====================================================================

/// **Differentiating an integral returns the integrand.**
/// (`test.cc:576`-`:740`, `test_integration` through `test_integration6`.)
///
/// `∫f dx` then `d/dx`, evaluated at `x = 3` and `x = -5`, must print what `f`
/// prints at those points. The reference builds its integrands by crossing 20
/// polynomial/radical/exponential bases with 19 wrappers and 6 shapes; this
/// module transcribes all three lists.
///
/// `test_integration6` skips a case whose integral came back still containing
/// `integrate(` — the reference's way of saying "no rule applied". In this port
/// the equivalent is `integrate::integrate` returning `None`, since the
/// unevaluated `integrate(…)` node is only reconstructed by the *function*
/// wrapper (`calculate_integrate`); both are treated as skips, and both are
/// counted, because "no rule applied" 2 000 times would make a green run
/// meaningless.
mod integrate_differentiate_roundtrip {
    use super::*;

    /// `test.cc:696`-`:739`, in order. The reference parses each of these and
    /// hands it to `test_integration2`.
    pub const BASES: &[&str] = &[
        "4x+5",
        "-2x+7",
        "4.7x-5.2",
        "-4.3x-5",
        "4x",
        "-2.3x",
        "x+6",
        "x-7",
        "x",
        "x^2",
        "2x^2+5",
        "-2x^2-5",
        "sqrt(x)",
        "sqrt(3x+3)",
        "5*sqrt(3x)-2",
        "cbrt(3x+3)",
        "(3x+3)^(1/3)",
        "cbrt(x)",
        "x^(1/3)",
        "5^x",
    ];

    /// `test_integration2` (`test.cc:633`-`:695`). `@` is the base.
    ///
    /// The trigonometric four multiply by `CALCULATOR->getRadUnit()` first;
    /// with `angle_unit = ANGLE_UNIT_RADIANS` that is the identity, and this
    /// port has no angle unit at all, so `sin(@)` is the faithful reading.
    pub const WRAPPERS: &[&str] = &[
        "ln(@)",
        "sin(@)",
        "cos(@)",
        "tan(@)",
        "asin(@)",
        "acos(@)",
        "atan(@)",
        "sinh(@)",
        "cosh(@)",
        "tanh(@)",
        "asinh(@)",
        "acosh(@)",
        "atanh(@)",
        "(@)^2",
        "(@)^(-1)",
        "(@)^(-2)",
        "(@)^3",
        "(@)^(-3)",
        "(@)^(1/3)",
    ];

    /// `test_integration3` (`test.cc:617`-`:632`). `@` is the wrapped
    /// expression, `$` the bare base — `mstruct_arg`, the second parameter.
    ///
    /// The fourth entry is `mstruct2.last()[1] = nr_minus_one`, which edits the
    /// exponent of the `x^2` the previous line appended: `w·x²` becomes `w/x`.
    pub const SHAPES: &[&str] = &[
        "@",
        "(@)*x",
        "(@)*x^2",
        "(@)*x^(-1)",
        "(@)*($)",
        "(@)*(@)",
    ];

    /// `test_integration4` (`test.cc:613`): the two evaluation points.
    ///
    /// The commented-out `test_integration5(mstruct, Number(2, 1), Number(7, 3))`
    /// above it is the *definite*-integral property, which needs `romberg` and
    /// is a different claim; `test_integration6` is the one that is live.
    pub const POINTS: &[i64] = &[3, -5];

    /// `(case id, diagnosis)`, in the shape `evaluation_invariants.rs` uses:
    /// a new violation fails the test, and so does an entry that has started
    /// passing. See the module docs on the ledger.
    /// One defect, in three cases, and it is not in `integrate.rs`.
    ///
    /// All three are `acos(u)` with `u` monic in `x`. `∫acos(u) dx` comes back
    /// as `x·acos(u) - sqrt(1 - u²)/u'` — correct — and differentiating it
    /// gives `x/sqrt(1-u²) - x/sqrt(1-u²) + acos(u)`, whose first two terms are
    /// structurally identical and must cancel. One `evaluate_calculated_with`
    /// pass does not cancel them; a second one does. Verified directly on
    /// `acos(x)`: pass 1 prints `x / sqrt(1 - x^2) - x / sqrt(1 - x^2) +
    /// acos(x)`, pass 2 prints `acos(x)`, pass 3 is stable. `test_integration6`
    /// evaluates once (`test.cc:586`), so the uncancelled pair survives to the
    /// substitution — and at `x = 3` the two terms become `3 / sqrt(-8)`, a
    /// complex float whose interval carries a rounding residue, so they no
    /// longer cancel *at all* and the result prints as `… + acos(3)` with a
    /// leading `0.000000000i`.
    ///
    /// This is the same non-idempotence `evaluation_invariants.rs` records as
    /// P1 class B for `abs(x - y) - abs(y - x)`, reached from a third
    /// direction. It is invisible through the CLI, because
    /// `differentiate::calculate_function` hands its result back to the outer
    /// merge loop, which supplies the second pass — `diff(acos(x) * x -
    /// sqrt(1 - x^2), x)` prints `acos(x)` there.
    ///
    /// The wrapper's four other bases (`4x+5`, `-2x+7`, `4.7x-5.2`, …) hold,
    /// because a non-unit `u'` leaves the two terms with different constant
    /// factorisations that the *first* pass does merge.
    pub const KNOWN_VIOLATIONS: &[(&str, &str)] = &[
        (
            "acos(x)",
            "evaluation is not idempotent: d/dx of the (correct) antiderivative \
             leaves `x / sqrt(1 - x^2) - x / sqrt(1 - x^2) + acos(x)`, which a \
             second pass folds to `acos(x)`. Substituting x = 3 into the \
             unfolded form turns the pair into `3 / sqrt(-8) - 3 / sqrt(-8)`, a \
             complex float difference that evaluates to `0.000000000i` rather \
             than 0. Not a wrong integral — `∫acos(x) dx = x acos(x) - \
             sqrt(1 - x^2)` is right.",
        ),
        (
            "acos(x+6)",
            "same as `acos(x)`: the uncancelled `x / sqrt(1 - (x+6)^2)` pair \
             survives one evaluation pass and stops cancelling once x = 3 makes \
             it `3 / sqrt(-80)`.",
        ),
        (
            "acos(x-7)",
            "same as `acos(x)`, with `3 / sqrt(-15)`.",
        ),
    ];

    /// Whether `m` mentions any of `names`, resolved through the port's own
    /// function tables so a typo excludes nothing silently.
    fn mentions(m: &MathStructure, names: &[&str]) -> bool {
        if let MathStructure::Function { id, .. } = m {
            if names.iter().any(|n| {
                qalc_core::builtins::function_id_for_name(n)
                    .unwrap_or_else(|| panic!("`{n}` is not a function name this port knows"))
                    == *id
            }) {
                return true;
            }
        }
        match m {
            MathStructure::Power { base, exponent } => {
                mentions(base, names) || mentions(exponent, names)
            }
            _ => m.children().any(|c| mentions(c, names)),
        }
    }

    /// The non-elementary functions `integrate.rs` answers with. An
    /// antiderivative built out of these is a correct answer that
    /// `differentiate.rs` has no rule to undo.
    const SPECIAL_FUNCTIONS: &[&str] = &[
        "Si", "Ci", "Shi", "Chi", "Ei", "li", "erf", "erfi", "fresnels", "fresnelc",
    ];

    /// Why an antiderivative could not be differentiated back.
    ///
    /// Split out because "80 cases could not be differentiated" is exactly the
    /// kind of number that hides two unrelated gaps in one bucket. It is two:
    /// the `∫ dx/(ax+b) = ln|ax+b| / a` rule puts an `abs()` in the answer and
    /// `differentiate.rs` has no `abs` rule, and the sine/cosine-integral
    /// answers are non-elementary by construction.
    fn undifferentiable_reason(anti: &MathStructure) -> &'static str {
        if mentions(anti, &["abs"]) {
            "the antiderivative contains abs() — the `int dx/(ax+b) = ln|ax+b|/a` \
             rule's output, which differentiate.rs has no rule for"
        } else if mentions(anti, SPECIAL_FUNCTIONS) {
            "the antiderivative is non-elementary (Si/Ci/Ei/erf/fresnel), which \
             differentiate.rs has no rule for"
        } else {
            "the antiderivative cannot be differentiated"
        }
    }

    fn contains_integrate(m: &MathStructure) -> bool {
        let want = qalc_core::integrate::function_id_for_name("integrate")
            .expect("`integrate` is a function name this port knows");
        if let MathStructure::Function { id, .. } = m {
            if *id == want {
                return true;
            }
        }
        match m {
            MathStructure::Power { base, exponent } => {
                contains_integrate(base) || contains_integrate(exponent)
            }
            _ => m.children().any(contains_integrate),
        }
    }

    /// One integrand, tagged with the three loops that built it.
    #[derive(Debug, Clone)]
    pub struct Case {
        /// The integrand, which is also its id: unique, greppable, and worth
        /// more in a ledger entry than an index would be.
        pub expr: String,
        /// The `test_integration2` template it came from.
        pub wrapper: &'static str,
        /// The `test_integration3` template it came from.
        pub shape: &'static str,
    }

    /// Every integrand the reference builds.
    pub fn corpus() -> Vec<Case> {
        let mut out = Vec::with_capacity(BASES.len() * WRAPPERS.len() * SHAPES.len());
        for base in BASES {
            for wrapper in WRAPPERS {
                let wrapped = wrapper.replace('@', base);
                for shape in SHAPES {
                    out.push(Case {
                        expr: shape.replace('@', &wrapped).replace('$', base),
                        wrapper,
                        shape,
                    });
                }
            }
        }
        out
    }

    pub fn check(expr: String) -> harness::Verdict {
        let x = MathStructure::symbolic("x");

        let Ok(mut f) = harness::parse(&expr) else {
            return harness::Verdict::skipped("does not parse");
        };
        qalc_core::percent::apply(&mut f);

        // `simplify_first` (MathStructure-integrate.cc:7407, mirrored by
        // `calculate_integrate`): the integrand is evaluated before the rule
        // table sees it, so `x^-1` and `1/x` reach the same shape.
        let mut integrand = f.clone();
        harness::evaluate_in_place(&mut integrand);

        let Some(mut anti) = qalc_core::integrate::integrate(&integrand, &x) else {
            return harness::Verdict::skipped("no integration rule applies");
        };
        harness::evaluate_in_place(&mut anti);
        if contains_integrate(&anti) {
            // `test.cc:585`: `if(mstruct2.containsFunction(f_integrate)) return;`
            return harness::Verdict::skipped("the integral still contains integrate()");
        }

        let Some(mut back) = qalc_core::differentiate::differentiate(&anti, &x) else {
            return harness::Verdict::skipped(undifferentiable_reason(&anti));
        };
        harness::evaluate_in_place(&mut back);

        compare_at_points(&f, &anti, &back)
    }

    /// `test_integration6`'s comparison loop (`test.cc:587`-`:611`), split out
    /// so the injected-defect tests can drive it with a *wrong* `back` and show
    /// that it says so. `anti` is carried only to make the failure message
    /// name the antiderivative that produced the disagreement.
    pub fn compare_at_points(
        f: &MathStructure,
        anti: &MathStructure,
        back: &MathStructure,
    ) -> harness::Verdict {
        let x = MathStructure::symbolic("x");
        let mut checked = 0usize;
        for p in POINTS {
            let n = MathStructure::from_i64(*p);

            let mut got = back.clone();
            qalc_core::solve::replace(&mut got, &x, &n);
            harness::evaluate_in_place(&mut got);

            let mut want = f.clone();
            qalc_core::solve::replace(&mut want, &x, &n);
            harness::evaluate_in_place(&mut want);

            if excluded(&got, &want) {
                continue;
            }
            checked += 1;
            if results_agree(&got, &want) {
                continue;
            }
            return harness::Verdict::violated(format!(
                "at x = {p}: d/dx ∫f dx = {}   but f = {}   \
                 (∫f dx = {}, d/dx of it = {})",
                harness::print(&got),
                harness::print(&want),
                harness::print(anti),
                harness::print(back),
            ));
        }
        if checked == 0 {
            return harness::Verdict::skipped(
                "test.cc exclusion: both points gave an infinity or a division by zero",
            );
        }
        harness::Verdict::Held
    }
}

/// Wall clock allowed per case.
///
/// A backstop for a non-terminating integrand, not a performance budget: the
/// slowest case that finishes at all finishes orders of magnitude inside this.
/// `QALC_CALCULUS_TIMEOUT_MS` overrides it.
fn timeout_ms() -> u64 {
    std::env::var("QALC_CALCULUS_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000)
}

/// Compare a produced violation list against a recorded one: neither list may
/// gain an entry, and neither may keep a stale one.
fn assert_matches_known(property: &str, found: &[(String, String)], known: &[(&str, &str)]) {
    let found_ids: BTreeSet<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
    let known_ids: BTreeSet<&str> = known.iter().map(|(id, _)| *id).collect();

    let new: Vec<String> = found
        .iter()
        .filter(|(id, _)| !known_ids.contains(id.as_str()))
        .map(|(id, why)| format!("  {id}\n      {why}"))
        .collect();
    let stale: Vec<&str> = known_ids
        .iter()
        .copied()
        .filter(|id| !found_ids.contains(id))
        .collect();

    assert!(
        new.is_empty(),
        "{property}: {} case(s) newly violate this property. Either the change \
         that caused it is a regression, or add them to KNOWN_VIOLATIONS with \
         a diagnosis:\n{}",
        new.len(),
        new.join("\n")
    );
    assert!(
        stale.is_empty(),
        "{property}: {} KNOWN_VIOLATIONS entr(y/ies) now pass. Remove them, so \
         the recorded count cannot creep back up:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
}

/// Run `check` over `cases` on a guarded worker, returning one verdict each.
fn run_all(
    cases: &[integrate_differentiate_roundtrip::Case],
    timeout_ms: u64,
    check: fn(String) -> harness::Verdict,
) -> (Vec<harness::Verdict>, Vec<String>) {
    let mut runner = harness::Runner::new();
    let verdicts = cases
        .iter()
        .map(|case| {
            let expr = case.expr.clone();
            runner.run(&case.expr, timeout_ms, move || check(expr))
        })
        .collect();
    (verdicts, runner.timeouts.clone())
}

/// Run the property over the corpus, report coverage, and hold it to its
/// recorded violation list.
fn drive(
    property: &str,
    cases: &[integrate_differentiate_roundtrip::Case],
    timeout_ms: u64,
    check: fn(String) -> harness::Verdict,
    known: &[(&str, &str)],
) {
    let (verdicts, timeouts) = run_all(cases, timeout_ms, check);

    let mut violations: Vec<(String, String)> = Vec::new();
    let mut held = 0usize;
    let mut skips: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    // Coverage per `test_integration2` wrapper: how many of its integrands the
    // port could integrate *and* differentiate back at all. Without this, the
    // out-of-scope total is a single opaque number, and a property that skips
    // most of its corpus is measuring its own filter.
    let mut by_wrapper: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();

    for (case, verdict) in cases.iter().zip(&verdicts) {
        let entry = by_wrapper.entry(case.wrapper).or_default();
        entry.1 += 1;
        match verdict {
            harness::Verdict::Held => {
                held += 1;
                entry.0 += 1;
            }
            harness::Verdict::Skipped(reason) => {
                skips.entry(reason.clone()).or_default().push(case.expr.clone())
            }
            harness::Verdict::Violation(why) => {
                entry.0 += 1;
                violations.push((case.expr.clone(), why.clone()))
            }
        }
    }

    let skipped: usize = skips.values().map(|v| v.len()).sum();
    println!(
        "\n{property}\n  {} integrand(s): {held} held, {} violated, {skipped} out of scope, \
         {} timed out",
        cases.len(),
        violations.len(),
        timeouts.len()
    );
    for (reason, ids) in &skips {
        println!("    out of scope ({}): {reason}", ids.len());
        if ids.len() <= 20 {
            for id in ids {
                println!("        {id}");
            }
        }
    }
    println!("    checked, by test_integration2 wrapper:");
    for (wrapper, (checked, total)) in &by_wrapper {
        println!("        {wrapper:>10}  {checked:>3}/{total}");
    }
    for t in &timeouts {
        println!("    TIMEOUT: {t}");
    }
    for (id, why) in &violations {
        println!("    VIOLATION {id}\n      {why}");
    }
    assert!(
        held > 0,
        "{property}: every case was skipped — the property tested nothing"
    );

    assert_matches_known(property, &violations, known);
}

#[test]
fn differentiating_an_integral_returns_the_integrand() {
    drive(
        "P5 integrate/differentiate round trip",
        &integrate_differentiate_roundtrip::corpus(),
        timeout_ms(),
        integrate_differentiate_roundtrip::check,
        integrate_differentiate_roundtrip::KNOWN_VIOLATIONS,
    );
}

// =====================================================================
// Proving the check can fail
// =====================================================================
//
// P5 found three violations over 599 checked integrands, which is only
// informative if it *could* have found more. The comparison it makes has been
// relaxed twice relative to `test_integration6`'s raw string equality (see
// [`results_agree`]), and each relaxation is a chance to have accidentally
// built a test that passes for free. The two tests below hand
// [`integrate_differentiate_roundtrip::compare_at_points`] a deliberately
// wrong antiderivative — the two ways an integration rule actually goes wrong
// — and assert it is rejected, next to the correct one being accepted.

/// Differentiate `expr` the way [`integrate_differentiate_roundtrip::check`]
/// does, and run the pointwise comparison against `integrand`.
fn roundtrip_verdict(integrand: &str, antiderivative: &str) -> harness::Verdict {
    let x = MathStructure::symbolic("x");
    let mut f = harness::parse(integrand).expect("the integrand parses");
    qalc_core::percent::apply(&mut f);
    let mut anti = harness::parse(antiderivative).expect("the antiderivative parses");
    harness::evaluate_in_place(&mut anti);
    let mut back = qalc_core::differentiate::differentiate(&anti, &x)
        .expect("the antiderivative differentiates");
    harness::evaluate_in_place(&mut back);
    integrate_differentiate_roundtrip::compare_at_points(&f, &anti, &back)
}

/// A wrong *constant* in an integration rule — `x^n/(n+1)` with the divisor
/// slightly off — must be caught.
///
/// This is the failure mode [`results_agree`]'s numeric fallback is closest to
/// swallowing: both sides reduce to numbers that agree to seven digits, and
/// the tolerance is `1e-9` relative. `1/2.9999999` instead of `1/3` puts the
/// derivative 3.3e-8 out, which the display's ten significant digits can
/// still see. Anything much smaller than that could not be distinguished from
/// rounding at the default precision, and the module docs say so; this test
/// pins where the line actually is.
#[test]
fn the_check_rejects_a_wrong_constant_factor() {
    assert_eq!(
        roundtrip_verdict("x^2", "x^3/3"),
        harness::Verdict::Held,
        "the correct antiderivative of x^2 was rejected — the comparison is \
         broken in the other direction"
    );
    let bad = roundtrip_verdict("x^2", "x^3/2.9999999");
    assert!(
        matches!(bad, harness::Verdict::Violation(_)),
        "an integration constant wrong by 3.3e-8 was accepted: {bad:?}"
    );
}

/// A missing chain-rule factor — the single most likely `integrate.rs` bug,
/// since almost every rule in it is `f(ax + b)` with a `1/a` out front — must
/// be caught.
#[test]
fn the_check_rejects_a_missing_chain_rule_factor() {
    assert_eq!(
        roundtrip_verdict("sin(4x+5)", "-cos(4x+5)/4"),
        harness::Verdict::Held,
        "the correct antiderivative of sin(4x + 5) was rejected"
    );
    let bad = roundtrip_verdict("sin(4x+5)", "-cos(4x+5)");
    assert!(
        matches!(bad, harness::Verdict::Violation(_)),
        "a missing 1/4 chain-rule factor was accepted: {bad:?}"
    );
    // And a sign error, which a magnitude-only comparison would miss.
    let flipped = roundtrip_verdict("sin(4x+5)", "cos(4x+5)/4");
    assert!(
        matches!(flipped, harness::Verdict::Violation(_)),
        "a sign error in the antiderivative was accepted: {flipped:?}"
    );
}

/// Prints `integrand<TAB>verdict` for every case and nothing else. Not part of
/// the suite — it exists to rebuild [`integrate_differentiate_roundtrip::KNOWN_VIOLATIONS`]
/// and to read a whole skip class at once, which the driver only summarises:
///
/// ```text
/// cargo test -q -p qalc-core --test calculus_properties -- --ignored --nocapture worker
/// ```
#[test]
#[ignore = "diagnostic tool, not an assertion"]
fn worker() {
    let cases = integrate_differentiate_roundtrip::corpus();
    let (verdicts, _) = run_all(
        &cases,
        timeout_ms(),
        integrate_differentiate_roundtrip::check,
    );
    for (case, verdict) in cases.iter().zip(verdicts) {
        let s = match verdict {
            harness::Verdict::Held => "HELD".to_string(),
            harness::Verdict::Skipped(r) => format!("SKIP\t{r}"),
            harness::Verdict::Violation(w) => format!("VIOLATION\t{w}"),
        };
        println!("{}\t{}\t{}\t{s}", case.expr, case.wrapper, case.shape);
    }
}

/// The corpus is the cross product `test.cc` builds; its size is worth pinning
/// so a template edit that quietly drops a wrapper cannot leave the suite green
/// while testing a fraction of `integrate.rs`.
#[test]
fn the_corpus_is_the_cross_product_test_cc_builds() {
    use integrate_differentiate_roundtrip as p5;
    let corpus = p5::corpus();
    assert_eq!(p5::BASES.len(), 20, "test.cc parses 20 base integrands");
    assert_eq!(p5::WRAPPERS.len(), 19, "test_integration2 applies 19 wrappers");
    assert_eq!(p5::SHAPES.len(), 6, "test_integration3 builds 6 shapes");
    assert_eq!(
        corpus.len(),
        p5::BASES.len() * p5::WRAPPERS.len() * p5::SHAPES.len()
    );
    let ids: BTreeSet<&str> = corpus.iter().map(|c| c.expr.as_str()).collect();
    assert_eq!(ids.len(), corpus.len(), "case ids are not unique");
    println!(
        "corpus: {} integrands ({} bases x {} wrappers x {} shapes), \
         {} integrand/point pairs",
        corpus.len(),
        p5::BASES.len(),
        p5::WRAPPERS.len(),
        p5::SHAPES.len(),
        corpus.len() * p5::POINTS.len()
    );
}

