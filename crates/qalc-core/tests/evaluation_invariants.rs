//! Self-consistency invariants of the evaluator, ported from `src/test.cc`.
//!
//! The 17 `.batch` transcripts check one thing: that a given input prints a
//! given string. They cannot see anything a *pair* of evaluations would
//! disagree about, and they cannot see a wrong answer that the reference
//! binary also produces — which is most of what a solver gets wrong.
//!
//! libqalculate's own `src/test.cc` checks four such pairings. It has no
//! `assert()`s (it is a `main()` that prints counters), so what is ported here
//! is the *predicate*, turned into a test. None of the four needs an oracle:
//! each compares the evaluator against itself.
//!
//! | module | test.cc | claim |
//! |---|---|---|
//! | [`p1_exact_then_approximate`] | `:1645`, `:1940` | exact-then-approximate equals direct-approximate |
//! | [`p2_substitution_commutes`]  | `:1706`, `:2001` | `eval(e[x:=n])` equals `eval(eval(e))[x:=n]` |
//! | [`p3_roots_are_roots`]        | `:1794`-`:1899`  | every root the solver returns satisfies its equation |
//! | [`p8_interval_flag_survives`] | `:1670`, `:1713`, `:1900`, `:1964` | evaluation leaves the global numeric context as it found it |
//!
//! # What is deliberately *not* ported
//!
//! `test.cc`'s `rnd_expression` generator (`:1393`-`:1595`) builds ~200 lines
//! of random expression trees from `rand()`. A failure it finds cannot be
//! reproduced without the seed, and the seed is the wall clock. Every case
//! here comes instead from a fixed corpus: every expression appearing in
//! libqalculate's own `tests/*.batch`, plus the equations in
//! [`corpus::CONSTRUCTED_EQUATIONS`]. Same inputs on every run, on every
//! machine.
//!
//! # Recorded failures
//!
//! Each module carries a `KNOWN_VIOLATIONS` table with a diagnosis per entry,
//! in the same shape as `crates/qalc/tests/transcripts.rs`: a new violation
//! fails the test, and so does an entry that has started passing. The counts
//! can only go down. Nothing here is fixed by this file — the violations are
//! real defects in `qalc-core`/`qalc-num`, recorded so the next change to
//! those crates cannot add more without saying so.
//!
//! At the time of writing, over 693 corpus expressions (660 from the
//! transcripts, 33 constructed) and the 63 of them that are equations:
//!
//! | property | checked | violations |
//! |---|---|---|
//! | P1 | 674 | 7, in two classes — all of them spellings, none a wrong value |
//! | P2 | 44  | 1 |
//! | P3 | 60 of 63 equations | 0 |
//! | P8 | 693 | 0 |
//!
//! # Proving the checks can fail
//!
//! Three of the four found few or no violations, which is only informative if
//! they *could* have found one. `p3_residual_check_rejects_a_near_root`,
//! `p3_residual_check_rejects_a_near_root_of_a_trigonometric_equation` and
//! `p8_detects_a_dropped_restore` inject the exact defect each module hunts
//! and assert it is caught, and [`drive`] refuses to pass a property that
//! skipped every one of its cases. Without those, a broken predicate and a
//! correct evaluator look identical.
//!
//! # Runtime
//!
//! About three minutes wall clock on an idle machine, the four properties
//! running concurrently; measured at twelve with a dozen other `cargo test`
//! processes competing for the CPU. Each case runs on a worker thread under a
//! wall-clock cap ([`timeout_ms`]), so a non-terminating expression is
//! reported by name rather than hanging the suite.

use std::collections::BTreeSet;
use std::path::PathBuf;

use qalc_core::options::{ApproximationMode, EvaluationOptions};
use qalc_core::structure::{ComparisonType, MathStructure};
use qalc_core::{parser, Session};
use qalc_num::Number;

// =====================================================================
// corpus
// =====================================================================

/// The fixed input set: the transcripts, plus hand-written equations for the
/// solver.
mod corpus {
    use super::*;

    /// One input, tagged with where it came from so a violation entry is
    /// greppable.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Case {
        /// `"solver.batch:12"`, or `"constructed:x^2 = 4"`.
        pub id: String,
        pub expr: String,
    }

    /// Equations the transcripts do not contain, covering the solver paths
    /// `solve.rs` implements separately: the polynomial closed forms, the
    /// `a^x` / Lambert-W reductions, and the trigonometric families whose
    /// general solution carries a free integer.
    pub const CONSTRUCTED_EQUATIONS: &[&str] = &[
        // Degree 2: distinct real, repeated, complex-conjugate, non-monic.
        "x^2 - 5x + 6 = 0",
        "x^2 - 2x + 1 = 0",
        "x^2 + 2x + 5 = 0",
        "2x^2 - 3x - 7 = 0",
        "x^2 = 4",
        "x^2 + 1 = 0",
        // Degree 3: three rational roots, one real root, depressed, cubic
        // needing the trigonometric (casus irreducibilis) branch.
        "x^3 - 6x^2 + 11x - 6 = 0",
        "x^3 - 2 = 0",
        "x^3 + x^2 + x = 5",
        "x^3 - 3x + 1 = 0",
        "x^3 + 3x^2 + 3x + 1 = 0",
        // Degree 4: biquadratic, a perfect fourth power, and a general one.
        "x^4 - 5x^2 + 4 = 0",
        "x^4 - 2 = 0",
        "x^4 + 20x^3 + 150x^2 + 500x + 625 = 0",
        "x^4 - x^3 - 7x^2 + x + 6 = 0",
        // Exponential and Lambert-W reductions.
        "5^x = 3",
        "2^x = 10",
        "ln(x) + x = 3",
        "x^(-3x) = 2",
        "2^(3x) + 4x = 5",
        "x*e^x = 1",
        "x^(1/3) + x^(2/3) = 3",
        // Trigonometric families: the general solution contains a free `n`.
        "sin(3x) = 1/3",
        "sin(x) = 0",
        "cos(2x) = 1/2",
        "tan(x) = 1",
        "1/3 * sin(3x) - 1/3 = 0",
        "2/3 * sin(3x) - 1/3 = 0",
        "sin(x) + cos(x) = 1",
        // Radicals and rational equations.
        "sqrt(x) = 3",
        "sqrt(x + 1) = x - 1",
        "1/x + 1/(x+1) = 1",
        "(x^2 - 1) / (x - 1) = 4",
    ];

    /// libqalculate's `tests/` directory.
    ///
    /// Deliberately panics rather than skipping, for the reason
    /// `crates/qalc/tests/transcripts.rs` gives: a suite that silently tests
    /// nothing when the reference checkout is missing is worse than one that
    /// fails.
    pub fn transcripts_dir() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("QALCULATE_TESTS_DIR") {
            return Some(PathBuf::from(dir));
        }
        for candidate in [
            "/root/Project/libqalculate/tests",
            "../libqalculate/tests",
            "../../libqalculate/tests",
            "../../../libqalculate/tests",
        ] {
            let path = PathBuf::from(candidate);
            if path.join("operators.batch").is_file() {
                return Some(path);
            }
        }
        if std::env::var("QALC_ALLOW_MISSING_ORACLE").is_ok() {
            return None;
        }
        panic!(
            "reference transcripts not found. Set QALCULATE_TESTS_DIR to \
             libqalculate's tests/ directory, or QALC_ALLOW_MISSING_ORACLE=1 to skip."
        );
    }

    /// Turn one transcript line into the expression it denotes, or `None` when
    /// the line is a CLI command rather than an expression.
    ///
    /// The transcript format puts expressions at column 0 and expected results
    /// behind a TAB. Some column-0 lines are not expressions: `/set …` and
    /// `set …` are options, `delete v` removes a variable, and `alpha := 5`
    /// and `factor x^2-1` wrap an expression in a command word. The last two
    /// are unwrapped rather than dropped, because the expression inside them
    /// is a perfectly good corpus entry.
    pub fn normalize(line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            return None;
        }
        if line.starts_with('/') || line.starts_with("set ") || line.starts_with("delete ") {
            return None;
        }
        if let Some((_name, rhs)) = line.split_once(":=") {
            return Some(rhs.trim().to_string()).filter(|s| !s.is_empty());
        }
        for word in ["factorize ", "factor ", "expand ", "simplify "] {
            if let Some(rest) = line.strip_prefix(word) {
                return Some(rest.trim().to_string()).filter(|s| !s.is_empty());
            }
        }
        Some(line.to_string())
    }

    /// Every expression in every `.batch` file, in file-then-line order,
    /// followed by [`CONSTRUCTED_EQUATIONS`].
    pub fn all() -> Vec<Case> {
        let mut cases = Vec::new();
        if let Some(dir) = transcripts_dir() {
            let mut batches: Vec<PathBuf> = std::fs::read_dir(&dir)
                .expect("transcripts directory is readable")
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "batch"))
                .collect();
            batches.sort();
            assert!(!batches.is_empty(), "no .batch files in {}", dir.display());
            for path in batches {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let src = std::fs::read_to_string(&path).expect("transcript is readable");
                for (idx, raw) in src.lines().enumerate() {
                    // A TAB- or space-indented line is an expected result.
                    if raw.starts_with('\t') || raw.starts_with(' ') {
                        continue;
                    }
                    if let Some(expr) = normalize(raw) {
                        cases.push(Case {
                            id: format!("{name}:{}", idx + 1),
                            expr,
                        });
                    }
                }
            }
        }
        for eq in CONSTRUCTED_EQUATIONS {
            cases.push(Case {
                id: format!("constructed:{eq}"),
                expr: (*eq).to_string(),
            });
        }
        cases
    }

    /// The subset that parses to an equation in one unknown — the P3 corpus.
    pub fn equations() -> Vec<Case> {
        all()
            .into_iter()
            .filter(|c| {
                super::harness::with_session(|s| {
                    let Ok(m) = parser::parse_with(&c.expr, &s.parse_options, s) else {
                        return false;
                    };
                    matches!(
                        m,
                        MathStructure::Comparison {
                            op: ComparisonType::Equals,
                            ..
                        }
                    ) && qalc_core::polynomial::find_x_var(&m).is_some()
                })
            })
            .collect()
    }
}

// =====================================================================
// harness
// =====================================================================

/// Running one case without letting it take the suite down with it.
///
/// Two things a corpus case can do that a plain loop cannot survive: panic
/// (there are `expect`s on the evaluation path), and not terminate (the merge
/// engine has a pass cap, but the solver's iterations and the unit reducer do
/// not all have one). Both are treated as *results* here — a violation with a
/// distinct label — rather than as a reason for the run to stop.
mod harness {
    use super::*;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

    /// What one case did.
    ///
    /// "Held" and "not applicable" are kept apart deliberately. A property
    /// suite that reports 693 green cases when 600 of them were silently
    /// skipped is measuring its own corpus filter, not the evaluator, so every
    /// skip carries a reason and the driver prints the tally.
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

    /// A single long-lived worker thread that cases are fed to one at a time.
    ///
    /// One thread rather than one per case, because the numeric context is
    /// thread-local: `qalc_num::context::CONSTS` holds an astro-float
    /// constants cache that costs more to build than most cases cost to run.
    /// The thread is retired — leaked, deliberately, since a hung or panicked
    /// thread cannot be joined — whenever a case times out or panics, because
    /// after either the thread-locals are untrustworthy (`solve::SOLVING` is a
    /// plain `Cell<bool>` set around `isolate_x`, so a panic inside the solver
    /// leaves it `true` and silently disables solving for everything after).
    pub struct Runner {
        tx: Option<Sender<Job>>,
        rx: Option<Receiver<Verdict>>,
        /// Cases that hit the wall-clock cap, for the run report.
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
                .name("invariant-worker".into())
                // The evaluator recurses over the structure tree; the default
                // 2 MiB is not enough for the deeper transcript expressions.
                .stack_size(32 * 1024 * 1024)
                .spawn(move || {
                    while let Ok(job) = job_rx.recv() {
                        reset_context();
                        let verdict =
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                                Ok(v) => v,
                                Err(payload) => Verdict::violated(format!(
                                    "PANIC: {}",
                                    panic_message(&payload)
                                )),
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
                    // The thread survived, but its thread-locals may not have.
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

        /// Abandon the current worker and start a clean one.
        fn retire(&mut self) {
            // Dropping the sender lets a *live* worker exit its loop; a hung
            // one is left running until the process does. There is no way to
            // kill a thread in safe Rust, and joining it would be the hang.
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

    /// Put the thread-local numeric context back to its defaults before each
    /// case, so that a case which leaks state cannot be blamed on its
    /// predecessor — and so that [`super::p8_interval_flag_survives`] measures
    /// one evaluation rather than an accumulation.
    pub fn reset_context() {
        qalc_num::context::set_precision(qalc_num::context::DEFAULT_PRECISION);
        qalc_num::context::set_create_interval(true);
        qalc_num::context::set_interval_calculation(
            qalc_num::context::IntervalCalculation::VarianceFormula,
        );
    }

    thread_local! {
        /// One session per worker thread. `Session::new` installs the
        /// imaginary unit and forces the unit store (a process-wide
        /// `OnceLock`) to build, which is what makes `2 m`, `sin`, and `pi`
        /// resolve at all — `eval::parse_expression` uses `SymbolicResolver`
        /// and would turn most of the corpus into bare symbols.
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

    /// Parse with the session's resolver, without evaluating.
    pub fn parse(expr: &str) -> Result<MathStructure, String> {
        with_session(|s| parser::parse_with(expr, &s.parse_options, s).map_err(|e| e.to_string()))
    }

    /// Evaluate an already-parsed structure in place under `mode`.
    ///
    /// This is `Session::evaluate_expression` with the approximation mode
    /// swapped: percent rewriting, then the merge loop, dates, `isolate_x`
    /// and the canonical sort.
    pub fn evaluate_in_place(m: &mut MathStructure, mode: ApproximationMode) {
        let mut eo = EvaluationOptions::default();
        eo.approximation = mode;
        qalc_core::eval::evaluate_calculated_with(m, &eo);
    }

    /// Parse, apply percents, and evaluate under `mode`.
    pub fn evaluate(expr: &str, mode: ApproximationMode) -> Result<MathStructure, String> {
        let mut m = parse(expr)?;
        qalc_core::percent::apply(&mut m);
        evaluate_in_place(&mut m, mode);
        Ok(m)
    }

    pub fn print(m: &MathStructure) -> String {
        with_session(|s| qalc_core::print::print(m, &s.print_options))
    }
}

// =====================================================================
// shared predicates
// =====================================================================

/// `contains_infinity` (`MathStructure::containsInfinity`) — the first of
/// `test.cc`'s two documented exclusions.
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

/// `contains_division_by_zero` (test.cc's local helper) — the second
/// exclusion. A `0^-n` that survived evaluation is a division by zero the
/// engine declined to fold, and every downstream comparison on it is
/// meaningless.
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
        MathStructure::Comparison { left, right, .. } => {
            contains_division_by_zero(left) || contains_division_by_zero(right)
        }
        _ => m.children().any(contains_division_by_zero),
    }
}

/// Functions that take `x` as a *name* rather than as a value.
///
/// `primpart(-12x^3 + 30x - 20)` is a statement about a polynomial in `x`, and
/// `newtonsolve(x^3 + x = 5, 1)` is a statement about the unknown `x`.
/// Substituting a number for `x` before calling them does not produce "the
/// same thing evaluated at 3" — it produces a different question
/// (`primpart(-254)`). They are excluded from [`p2_substitution_commutes`] for
/// the same reason `test.cc` never feeds them to it: its `mp` comes from
/// `rnd_expression`, which builds arithmetic, not binders.
/// Every name here is a real entry in one of the port's
/// `function_id_for_name` tables; a name that resolved to nothing would
/// silently exclude nothing.
const X_BINDING_FUNCTIONS: &[&str] = &[
    // Calculus: the variable of differentiation/integration/limit.
    "diff",
    "derivative",
    "integrate",
    "integral",
    "romberg",
    "limit",
    "lim",
    // Solvers: the unknown being solved for.
    "solve",
    "newtonsolve",
    "secantsolve",
    // Polynomial structure: the name of the indeterminate.
    "coeff",
    "lcoeff",
    "tcoeff",
    "degree",
    "ldegree",
    "pcontent",
    "primpart",
    "punit",
    "factorize",
    "factor",
    "expand",
    // Vector generation: the variable the elements are generated over.
    "genvector",
];

/// Whether `m` passes its unknown to one of [`X_BINDING_FUNCTIONS`].
fn binds_x(m: &MathStructure) -> bool {
    if let MathStructure::Function { id, .. } = m {
        if X_BINDING_FUNCTIONS
            .iter()
            .map(|n| {
                qalc_core::builtins::function_id_for_name(n)
                    .unwrap_or_else(|| panic!("`{n}` is not a function name this port knows"))
            })
            .any(|want| want == *id)
        {
            return true;
        }
    }
    match m {
        MathStructure::Power { base, exponent } => binds_x(base) || binds_x(exponent),
        MathStructure::Comparison { left, right, .. } => binds_x(left) || binds_x(right),
        MathStructure::Conversion { value, .. } => binds_x(value),
        _ => m.children().any(binds_x),
    }
}

/// Whether either side is excluded from comparison, per `test.cc:1704`.
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

/// The C++ compares two results with `MathStructure::compare`, which for
/// interval-valued numbers answers `UNKNOWN` (and so *not* "unequal") when the
/// intervals overlap. Structural equality would flag every last-digit rounding
/// difference, so the numeric case is compared with a relative tolerance that
/// stands in for that overlap test, at the default 10-digit precision.
fn results_agree(a: &MathStructure, b: &MathStructure) -> bool {
    if a.equals(b) {
        return true;
    }
    if let (MathStructure::Number(x), MathStructure::Number(y)) = (a, b) {
        let (mx, my) = (magnitude(x), magnitude(y));
        if !mx.is_finite() || !my.is_finite() {
            return false;
        }
        let mut d = x.clone();
        if !d.subtract(y) {
            return false;
        }
        let diff = magnitude(&d);
        return diff <= 1e-9 * mx.max(my).max(1.0);
    }
    // Non-numeric results: the printed form is the contract the transcripts
    // hold the port to, so it is the right granularity here too.
    harness::print(a) == harness::print(b)
}

/// Compare a produced violation list against a recorded one, in the shape
/// `crates/qalc/tests/transcripts.rs` uses: neither list may gain an entry,
/// and neither may keep a stale one.
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
        "{property}: {} case(s) newly violate this invariant. Either the \
         change that caused it is a regression, or add them to \
         KNOWN_VIOLATIONS with a diagnosis:\n{}",
        new.len(),
        new.join("\n")
    );
    assert!(
        stale.is_empty(),
        "{property}: {} KNOWN_VIOLATIONS entr(y/ies) now pass. Remove them, \
         so the recorded count cannot creep back up:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
}

/// Wall clock allowed per case, before it is declared hung.
///
/// Generous on purpose, and it has to be.
///
/// The four tests run concurrently under `cargo test`, each with its own
/// worker thread, and the slowest legitimate cases in the corpus are the
/// Lambert-W solves (`solver.batch:19`, `:22`, `:74`, `:77`, `:80`) and
/// `betainc(5i - 2, 32, 3.2)` (`calculus.batch:24`) — seconds each on an idle
/// machine, tens of seconds under contention. At 20 s those six timed out on a
/// loaded machine and passed on a quiet one, which is a flaky test dressed up
/// as a hang detector. This cap exists only to stop a *non-terminating* case
/// from hanging CI for good; anything that finishes at all finishes far
/// inside it, so making it large costs nothing except on a case that was
/// never going to return. `QALC_INVARIANT_TIMEOUT_MS` overrides it.
fn timeout_ms() -> u64 {
    std::env::var("QALC_INVARIANT_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300_000)
}

/// Run one property over a corpus, report its coverage, and hold it to its
/// recorded violation list.
///
/// The four tests differ only in corpus, predicate, timeout and table, so they
/// share this. `check` is a plain `fn` rather than a closure so it can cross
/// into the worker thread without capturing anything.
fn drive(
    property: &str,
    cases: &[corpus::Case],
    timeout_ms: u64,
    check: fn(String) -> harness::Verdict,
    known: &[(&str, &str)],
) {
    let mut runner = harness::Runner::new();
    let mut violations: Vec<(String, String)> = Vec::new();
    let mut held = 0usize;
    let mut skips: std::collections::BTreeMap<String, Vec<String>> = Default::default();

    for case in cases {
        let expr = case.expr.clone();
        match runner.run(&case.id, timeout_ms, move || check(expr)) {
            harness::Verdict::Held => held += 1,
            harness::Verdict::Skipped(reason) => {
                skips.entry(reason).or_default().push(case.id.clone())
            }
            harness::Verdict::Violation(why) => violations.push((case.id.clone(), why)),
        }
    }

    let skipped: usize = skips.values().map(|v| v.len()).sum();
    println!(
        "\n{property}\n  {} corpus case(s): {held} held, {} violated, {skipped} out of scope, \
         {} timed out",
        cases.len(),
        violations.len(),
        runner.timeouts.len()
    );
    for (reason, ids) in &skips {
        println!("    out of scope ({}): {reason}", ids.len());
        // Small groups are named, because "14 cases were skipped" is exactly
        // the kind of number that hides a broken predicate.
        if ids.len() <= 20 {
            for id in ids {
                println!("        {id}");
            }
        }
    }
    for t in &runner.timeouts {
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

// =====================================================================
// P1
// =====================================================================

/// **Evaluating exactly and then approximating gives what approximating
/// directly gives.** (`test.cc:1636`-`:1666`, counters `rt1`/`rt2`.)
///
/// `APPROXIMATION_EXACT` is not a display setting: it changes which
/// simplifications the merge engine is allowed to make, and an exact result is
/// supposed to be a *different spelling of the same value*, not a different
/// value. So re-running an exact result under `APPROXIMATION_APPROXIMATE` must
/// land where going straight to approximate lands. When it does not, one of
/// the two paths has dropped or invented information — and the transcripts
/// cannot see it, because each transcript runs exactly one of the two.
mod p1_exact_then_approximate {
    use super::*;

    /// `(case id, diagnosis)`. See the module docs on `KNOWN_VIOLATIONS`.
    ///
    /// Two defects remain, in descending order of how wrong they are. Three
    /// classes have been retired:
    ///
    /// Class **A** — `Approximate` mode returning a wrong limit *value* for
    /// `limits.batch:325` and `:358` — is fixed. Its cause was the generic
    /// bottom-up function pass numerifying a `limit()` call's argument before
    /// dispatching the call, which destroyed the structural `0/0` the limit
    /// machinery looks for: `sqrt(x+3) - sqrt(3)` reached it as
    /// `sqrt(x+3) - 1.732050808`, whose value at `x = 0` exact evaluation
    /// cannot collapse (one term symbolic, one a float), so the denominator
    /// looked non-zero and the answer came back `0`.
    /// `eval::evaluate_calculated_with` now resolves limit subtrees exactly
    /// first, as `LimitFunction::calculate` does.
    ///
    /// Classes **C** (solver output left symbolic by `Approximate`, 8 cases)
    /// and **E** (a rational multiple of `pi` folded on one path only, 13
    /// cases) are fixed, and were one defect: `solve::isolate_x_toplevel` runs
    /// *after* `evaluate_calculated_with`'s merge loop, so nothing merged its
    /// output. The C++ solves inside `calculatesub`, where the loop that
    /// produced a solution also merges it. `evaluate_calculated_with` now runs
    /// the merge loop again when the solver fired — under `Approximate` only,
    /// because under `Exact` the extra pass re-associates the surds the
    /// reference's closed forms are spelled with (`(sqrt(5) + 3) / 2` becomes
    /// `sqrt(5) / 2 + 3/2`), which `polynomial.batch:6`, `solver.batch:7` and
    /// `solver.batch:19` pin.
    ///
    /// **B. Evaluation is not idempotent for `abs(a) - abs(-a)`** (1 case).
    /// Neither mode cancels `abs(x - y) - abs(y - x)` in one pass — both leave
    /// `|x - y| - |x - y|`, two structurally distinct terms the printer
    /// renders identically. A *second* pass folds them to `0`, which is why
    /// this shows up as an exact-versus-approximate difference here and why
    /// the transcript still passes: `eval::apply_conversion` runs the
    /// optimal-SI post-conversion afterwards, and that gives the CLI its
    /// second pass.
    ///
    /// **D. `Approximate` numerifies what `Exact` leaves symbolic** (6 cases).
    /// `ln(x) + x = 3` gives `x = 2.207940032` directly but `x = lambertw(e^3)`
    /// via exact — here the *exact* result is the one the second pass cannot
    /// finish. Not the same defect as C/E, and not fixed with them: this port
    /// parses `e` to `Symbolic("e")` and never gives it a value in *any* mode
    /// (`Session::install_builtin_constants` says so, and `qalc -t e` prints
    /// `e`), so `lambertw(e^3)` has no numeric argument to work from, while
    /// `solve.rs` reaches the number by its own numeric route. Retiring this
    /// class means numerifying `e` (and `pi`) under `Approximate`, which is
    /// part of the two-phase TRY_EXACT question the CLI's
    /// `approximation = Approximate` stand-in defers, not a solver bug.
    pub const KNOWN_VIOLATIONS: &[(&str, &str)] = &[
        // --- B: missing cancellation under Approximate -------------------
        (
            "polynomial.batch:23",
            "B: abs(x - y) - abs(y - x). One evaluation pass leaves \
             `|x - y| - |x - y|` in *either* mode; the second pass that \
             exact-then-approximate performs folds it to 0. The `abs` \
             argument-negation normalisation that makes the two terms equal \
             runs after the addition has already been merged, so within a \
             single pass they are never re-offered to the merge engine. \
             Verified directly: evaluating the expression once under \
             Approximate, once under Exact, and once more under Approximate \
             all print `|x - y| - |x - y|`.",
        ),
        // --- D: exact output left symbolic by the second pass ------------
        (
            "solver.batch:16",
            "D: ln(x) + x = 3. Direct Approximate gives x = 2.207940032; \
             exact-then-approximate is stuck at `x = lambertw(e^3)`, because `e` \
             is a bare symbol in this port in every mode, so `lambertw` has no \
             numeric argument to evaluate.",
        ),
        (
            "constructed:ln(x) + x = 3",
            "D: same as solver.batch:16.",
        ),
        (
            "solver.batch:74",
            "D: x^(5x) = 5. Direct Approximate gives 1.284730245; exact-then-approximate \
             stops at `e^0.2505487699`.",
        ),
        (
            "solver.batch:22",
            "D: x^(-3x) = 2. Direct Approximate gives 0.7280844118/0.1006083268; \
             exact-then-approximate stops at `1 / e^0.3173382872`.",
        ),
        (
            "solver.batch:80",
            "D: same as solver.batch:22.",
        ),
        (
            "constructed:x^(-3x) = 2",
            "D: same as solver.batch:22.",
        ),
    ];

    pub fn check(expr: String) -> harness::Verdict {
        let Ok(mut exact) = harness::evaluate(&expr, ApproximationMode::Exact) else {
            // A parse error is not this property's business; P1 is about the
            // two evaluation paths agreeing on inputs both can parse.
            return harness::Verdict::skipped("does not parse");
        };
        let Ok(direct) = harness::evaluate(&expr, ApproximationMode::Approximate) else {
            return harness::Verdict::skipped("does not parse");
        };
        // The C++ re-runs `calculate` on the already-exact structure rather
        // than re-parsing, which is the whole point: the second pass sees the
        // exact result, not the source text.
        harness::evaluate_in_place(&mut exact, ApproximationMode::Approximate);
        if excluded(&exact, &direct) {
            return harness::Verdict::skipped(
                "test.cc exclusion: result contains an infinity or a division by zero",
            );
        }
        if results_agree(&exact, &direct) {
            return harness::Verdict::Held;
        }
        harness::Verdict::violated(format!(
            "exact-then-approximate = {}   but direct-approximate = {}",
            harness::print(&exact),
            harness::print(&direct)
        ))
    }
}

#[test]
fn exact_then_approximate_equals_direct_approximate() {
    drive(
        "P1 exact-then-approximate",
        &corpus::all(),
        timeout_ms(),
        p1_exact_then_approximate::check,
        p1_exact_then_approximate::KNOWN_VIOLATIONS,
    );
}

// =====================================================================
// P2
// =====================================================================

/// **Substituting for `x` and evaluating commute.** (`test.cc:1672`-`:1712`,
/// counters `rt3`/`rt4`/`rt5`.)
///
/// `eval(e[x := n])` must equal `eval(eval(e)[x := n])`. Simplification is
/// only sound if it holds for *every* value of the unknown; a rewrite that is
/// valid for the generic case but wrong at a particular `n` — cancelling a
/// factor that vanishes, choosing a branch of a root by the sign of a symbol —
/// shows up here and nowhere in the transcripts, which never substitute.
mod p2_substitution_commutes {
    use super::*;

    /// One entry, and it is the same defect P1 records as class B.
    pub const KNOWN_VIOLATIONS: &[(&str, &str)] = &[(
        "polynomial.batch:23",
        "abs(x - y) - abs(y - x). The same non-idempotence P1 records as class \
         B, seen from the other side: substituting x := 3 first gives the \
         expression one evaluation pass, which leaves `|y - 3| - |y - 3|`, \
         while evaluating first and substituting after gives it two, and the \
         second pass cancels to 0. Two independent predicates landing on one \
         bug is the useful thing about having both.",
    )];

    /// The values substituted for the unknown.
    ///
    /// `test.cc` draws a random `rnd_number` here. Two fixed values instead:
    /// one integer and one non-dyadic rational, so that a rewrite which is
    /// only wrong on non-integers is still caught, and both far enough from
    /// the small poles (`0`, `±1`) that most of the corpus stays in scope.
    pub const SUBSTITUTIONS: &[(i64, i64)] = &[(3, 1), (7, 5)];

    pub fn check(expr: String) -> harness::Verdict {
        let Ok(parsed) = harness::parse(&expr) else {
            return harness::Verdict::skipped("does not parse");
        };
        // `test.cc` always substitutes for `CALCULATOR->v_x`, because its
        // `rnd_expression` only ever generates that one unknown. The corpus
        // here writes `y`, `z` and `a` as freely as `x`, so the unknown is
        // whichever one the port's own `find_x_var` would pick — the same
        // choice the solver makes.
        let Some(x) = qalc_core::polynomial::find_x_var(&parsed) else {
            return harness::Verdict::skipped("no free symbol to substitute for");
        };
        // An equation is not an expression with a free `x`: `eval` runs
        // `isolate_x` on it, so `eval(x^2 = 4)` is `x = 2 or x = -2`, and
        // substituting 3 into *that* asks "is 3 one of the roots" while
        // substituting into the equation first asks "is 9 equal to 4". Two
        // different questions, so no commuting law between them. `test.cc`
        // keeps its equations in the separate `test_equation` block for
        // exactly this reason — which is what P3 ports.
        if parsed.is_comparison() {
            return harness::Verdict::skipped(
                "an equation: `eval` solves it, so substitution cannot commute (see P3)",
            );
        }
        if binds_x(&parsed) {
            return harness::Verdict::skipped(
                "the unknown is bound by a calculus/polynomial/vector function, \
                 not a free value",
            );
        }
        // `eval(eval(e))` — the exact evaluation, substituted into afterwards.
        let mut simplified = parsed.clone();
        qalc_core::percent::apply(&mut simplified);
        harness::evaluate_in_place(&mut simplified, ApproximationMode::Exact);

        let mut held_for_some_value = false;
        for (num, den) in SUBSTITUTIONS {
            let n = MathStructure::Number(Number::from_ints(*num, *den, 0));

            let mut substituted_first = parsed.clone();
            qalc_core::solve::replace(&mut substituted_first, &x, &n);
            qalc_core::percent::apply(&mut substituted_first);
            harness::evaluate_in_place(&mut substituted_first, ApproximationMode::Approximate);

            let mut evaluated_first = simplified.clone();
            qalc_core::solve::replace(&mut evaluated_first, &x, &n);
            harness::evaluate_in_place(&mut evaluated_first, ApproximationMode::Approximate);

            if excluded(&substituted_first, &evaluated_first) {
                continue;
            }
            if results_agree(&substituted_first, &evaluated_first) {
                held_for_some_value = true;
                continue;
            }
            return harness::Verdict::violated(format!(
                "at {} = {num}/{den}: eval(e[{0}:=n]) = {}   but eval(e)[{0}:=n] = {}   \
                 (eval(e) = {})",
                harness::print(&x),
                harness::print(&substituted_first),
                harness::print(&evaluated_first),
                harness::print(&simplified),
            ));
        }
        if held_for_some_value {
            harness::Verdict::Held
        } else {
            harness::Verdict::skipped(
                "test.cc exclusion: every substitution gave an infinity or a division by zero",
            )
        }
    }
}

#[test]
fn substitution_and_evaluation_commute() {
    drive(
        "P2 substitution commutes",
        &corpus::all(),
        timeout_ms(),
        p2_substitution_commutes::check,
        p2_substitution_commutes::KNOWN_VIOLATIONS,
    );
}

// =====================================================================
// P3
// =====================================================================

/// **Every root the solver returns satisfies the equation it was given.**
/// (`test.cc:1794`-`:1899`, counters `rt6`/`rt7`.)
///
/// This is the one invariant here that catches a *wrong answer* rather than an
/// inconsistency. `solve.rs` is 2248 lines of closed forms, substitutions and
/// numeric iteration; the transcripts pin 76 of its outputs against the
/// reference, which means a root that is wrong in both implementations, or
/// wrong on an equation nobody wrote a transcript for, is invisible. Putting
/// the root back into the equation needs no reference at all.
///
/// The recursion through `or`/`and` to three levels is the C++'s, and is
/// needed because `solve` answers `x = 2 or x = -2`, and conditional solutions
/// come back as `x = a and x != 0`.
mod p3_roots_are_roots {
    use super::*;

    pub const KNOWN_VIOLATIONS: &[(&str, &str)] = &[];

    /// `log10(|f(r)|) < -10`, verbatim from `test.cc:1815`.
    pub const RESIDUAL_LOG10_MAX: f64 = -10.0;

    /// Working precision for this module.
    ///
    /// Raised from the default 10 because the criterion is absolute: a root
    /// carrying 10 correct significant digits leaves a residual of about
    /// `1e-10 * |r * f'(r)|`, which is over the threshold for any equation
    /// whose derivative is larger than 1 — the test would then be measuring
    /// the display precision rather than the solver. At 30 digits a correct
    /// root clears the bar by 19 orders of magnitude, so anything that fails
    /// is wrong rather than merely rounded.
    pub const PRECISION: i32 = 30;

    /// Values substituted for the free integer in a general trigonometric
    /// solution (`x = (2/3) * pi * n + pi/6`). `test.cc:1801` uses the single
    /// value 3; a general solution has to hold for all of these.
    pub const N_VALUES: &[i64] = &[0, 1, -1, 2];

    /// Collect the right-hand sides of every `x = …` in a solved structure,
    /// descending through `or`/`and` to three levels as the C++ does.
    ///
    /// An `and` branch is dropped when one of its non-equality conditions
    /// evaluates to false, mirroring `test.cc:1824`: `x = 1 and 1 = 0` asserts
    /// nothing about `x`.
    pub fn collect_roots(m: &MathStructure, x: &MathStructure, depth: u32, out: &mut Vec<MathStructure>) {
        if depth > 3 {
            return;
        }
        if let MathStructure::Comparison { left, op, right } = m {
            if *op == ComparisonType::Equals && left.equals(x) {
                out.push((**right).clone());
            }
            return;
        }
        if let MathStructure::LogicalAnd(parts) = m {
            for p in parts {
                if let MathStructure::Comparison { op, .. } = p {
                    if *op != ComparisonType::Equals {
                        let mut cond = p.clone();
                        harness::evaluate_in_place(&mut cond, ApproximationMode::Approximate);
                        if cond.is_zero() {
                            return;
                        }
                    }
                }
            }
        }
        for child in m.children() {
            collect_roots(child, x, depth + 1, out);
        }
    }

    /// Every concrete root to test: each collected right-hand side, with the
    /// free integer `n` instantiated when it carries one.
    fn instantiate(root: &MathStructure) -> Vec<(String, MathStructure)> {
        let n = MathStructure::symbolic("n");
        if !qalc_core::differentiate::contains(root, &n) {
            return vec![(String::new(), root.clone())];
        }
        N_VALUES
            .iter()
            .map(|k| {
                let mut r = root.clone();
                qalc_core::solve::replace(&mut r, &n, &MathStructure::from_i64(*k));
                (format!(" (n = {k})"), r)
            })
            .collect()
    }

    /// `lhs - rhs`, unevaluated. Evaluating it here would let `isolate_x` run
    /// on it and turn the residual back into a solution.
    pub fn residual_template(equation: &MathStructure) -> Option<MathStructure> {
        let MathStructure::Comparison { left, op, right } = equation else {
            return None;
        };
        if *op != ComparisonType::Equals {
            return None;
        }
        let mut f = MathStructure::Addition(vec![
            (**left).clone(),
            MathStructure::Multiplication(vec![
                MathStructure::from_i64(-1),
                (**right).clone(),
            ]),
        ]);
        qalc_core::percent::apply(&mut f);
        Some(f)
    }

    /// `log10(|f(candidate)|)`, or `None` when the substitution does not
    /// reduce to a number. An exact zero is reported as `f64::NEG_INFINITY`,
    /// which passes every threshold.
    pub fn residual_log10(
        template: &MathStructure,
        x: &MathStructure,
        candidate: &MathStructure,
    ) -> Option<(f64, MathStructure)> {
        let mut f = template.clone();
        qalc_core::solve::replace(&mut f, x, candidate);
        numerify_constants(&mut f);
        harness::evaluate_in_place(&mut f, ApproximationMode::Approximate);
        let MathStructure::Number(v) = &f else {
            return None;
        };
        if v.is_zero() {
            return Some((f64::NEG_INFINITY, f.clone()));
        }
        Some((magnitude(v).log10(), f.clone()))
    }

    /// Give `pi` (and `e`) their numeric values before the residual is
    /// measured.
    ///
    /// This port parses `pi` to `Symbolic("pi")` and never collapses it —
    /// `Session::install_builtin_constants` says so explicitly, because the
    /// limit transcripts depend on the symbol surviving. Every trigonometric
    /// general solution therefore comes back as `(2/3) * pi * n + pi / 6`, and
    /// substituting it into `sin(3x) - 1/3` leaves a symbolic expression whose
    /// magnitude cannot be measured. Without this step the 14 cases that make
    /// the trigonometric solver worth testing are all silently "inconclusive".
    fn numerify_constants(m: &mut MathStructure) {
        let mut pi = Number::new();
        pi.pi();
        qalc_core::solve::replace(
            m,
            &MathStructure::symbolic("pi"),
            &MathStructure::Number(pi),
        );
        let mut e = Number::new();
        e.e();
        qalc_core::solve::replace(m, &MathStructure::symbolic("e"), &MathStructure::Number(e));
    }

    pub fn check(expr: String) -> harness::Verdict {
        qalc_num::context::set_precision(PRECISION);

        let Ok(equation) = harness::parse(&expr) else {
            return harness::Verdict::skipped("does not parse");
        };
        let Some(residual_template) = residual_template(&equation) else {
            return harness::Verdict::skipped("not an equality");
        };
        let Some(x) = qalc_core::polynomial::find_x_var(&equation) else {
            return harness::Verdict::skipped("no unknown to solve for");
        };

        let mut solved = equation.clone();
        qalc_core::percent::apply(&mut solved);
        harness::evaluate_in_place(&mut solved, ApproximationMode::Approximate);

        let mut roots = Vec::new();
        collect_roots(&solved, &x, 0, &mut roots);
        // A "root" still mentioning the unknown is not a solution; the C++
        // requires `m1[0] == v_x` and drops the rest.
        roots.retain(|r| !qalc_core::differentiate::contains(r, &x));
        if roots.is_empty() {
            // The solver declined to isolate. That is a coverage gap in
            // `solve.rs`, not a violation of this property — a solver that
            // says nothing has not said anything false.
            return harness::Verdict::skipped(
                "solver returned no isolated root (equation left unsolved)",
            );
        }

        let mut checked = 0usize;
        let mut worst: Option<(String, String, f64)> = None;
        for root in &roots {
            for (label, concrete) in instantiate(root) {
                let Some((log10, f)) = residual_log10(&residual_template, &x, &concrete) else {
                    // The substitution did not reduce to a number, so the
                    // check is inconclusive rather than failed.
                    continue;
                };
                checked += 1;
                if log10 < RESIDUAL_LOG10_MAX {
                    continue;
                }
                if worst.as_ref().is_none_or(|(_, _, w)| log10 > *w || w.is_nan()) {
                    worst = Some((
                        format!("{}{label}", harness::print(root)),
                        harness::print(&f),
                        log10,
                    ));
                }
            }
        }
        match worst {
            Some((root, residual, log10)) => harness::Verdict::violated(format!(
                "x = {root} is not a root: f(x) = {residual} (log10|f| = {log10:.2}, \
                 must be < {RESIDUAL_LOG10_MAX}); solver returned `{}`",
                harness::print(&solved)
            )),
            None if checked == 0 => harness::Verdict::skipped(
                "roots substitute back to a non-numeric expression",
            ),
            None => harness::Verdict::Held,
        }
    }
}

#[test]
fn solver_roots_satisfy_their_equations() {
    drive(
        "P3 roots are roots",
        &corpus::equations(),
        timeout_ms(),
        p3_roots_are_roots::check,
        p3_roots_are_roots::KNOWN_VIOLATIONS,
    );
}

/// The residual check must reject a value that is *nearly* a root, or P3 is
/// green for free.
///
/// `2.0000001` sits 1e-7 from a root of `x^2 = 4`, which the transcripts'
/// 10-digit output would happily print as `2`. The threshold has to catch it.
#[test]
fn p3_residual_check_rejects_a_near_root() {
    qalc_num::context::set_precision(p3_roots_are_roots::PRECISION);
    let equation = harness::parse("x^2 = 4").expect("parses");
    let template = p3_roots_are_roots::residual_template(&equation).expect("is an equality");
    let x = MathStructure::symbolic("x");

    let (exact, _) = p3_roots_are_roots::residual_log10(&template, &x, &MathStructure::from_i64(2))
        .expect("reduces to a number");
    assert!(
        exact < p3_roots_are_roots::RESIDUAL_LOG10_MAX,
        "the true root x = 2 was rejected (log10|f| = {exact})"
    );

    let near = MathStructure::Number(Number::from_ints(20000001, 10000000, 0));
    let (off, _) = p3_roots_are_roots::residual_log10(&template, &x, &near)
        .expect("reduces to a number");
    assert!(
        off >= p3_roots_are_roots::RESIDUAL_LOG10_MAX,
        "x = 2.0000001 was accepted as a root of x^2 = 4 (log10|f| = {off}) — \
         the P3 threshold has no teeth"
    );
}

/// The same, on the path that needed `numerify_constants` — and the one place
/// the criterion is genuinely blunt.
///
/// Every trigonometric case in the P3 corpus reaches the threshold through a
/// root of the form `(2/3) * pi * n + pi / 18`, which only becomes a number
/// because `pi` is substituted. If that substitution silently stopped working
/// the residual would go non-numeric and 14 cases would report "inconclusive"
/// instead of failing, so both halves are pinned here.
///
/// The second assertion is on `2/3 * sin(3x) = 1/3` rather than
/// `1/3 * sin(3x) = 1/3` deliberately. The latter asks for `sin(3x) = 1`,
/// which is the *maximum* of the sine: `f'` vanishes at the root, so a value
/// 1e-7 away still has a residual of 1.5e-14 and passes. That is a real
/// weakness of `test.cc`'s absolute `log10|f(r)| < -10`, inherited here — at a
/// repeated root the test only bounds the error by its square root. It is
/// recorded rather than fixed, because changing the criterion would stop this
/// from being a port of `test.cc:1815`.
#[test]
fn p3_residual_check_rejects_a_near_root_of_a_trigonometric_equation() {
    qalc_num::context::set_precision(p3_roots_are_roots::PRECISION);
    let equation = harness::parse("2/3 * sin(3x) - 1/3 = 0").expect("parses");
    let template = p3_roots_are_roots::residual_template(&equation).expect("is an equality");
    let x = MathStructure::symbolic("x");

    // pi/18 is the n = 0 member of the general solution.
    let root = harness::parse("pi / 18").expect("parses");
    let (exact, _) = p3_roots_are_roots::residual_log10(&template, &x, &root)
        .expect("reduces to a number — `pi` must have been substituted");
    assert!(
        exact < p3_roots_are_roots::RESIDUAL_LOG10_MAX,
        "the true root x = pi/18 was rejected (log10|f| = {exact})"
    );

    let near = harness::parse("pi / 18 + 1/10000000").expect("parses");
    let (off, _) = p3_roots_are_roots::residual_log10(&template, &x, &near)
        .expect("reduces to a number");
    assert!(
        off >= p3_roots_are_roots::RESIDUAL_LOG10_MAX,
        "x = pi/18 + 1e-7 was accepted as a root (log10|f| = {off})"
    );
}

// =====================================================================
// P8
// =====================================================================

/// **Evaluation leaves the global numeric context as it found it.**
/// (`test.cc:1670`, `:1713`, `:1900`, `:1964` — `INTERVAL ARITHMETIC
/// CHANGED1`/`3`/`5`.)
///
/// `qalc_num::context` is thread-local global state: `CREATE_INTERVAL` decides
/// whether every arithmetic result carries an uncertainty, and `PRECISION`
/// decides how many bits it carries. Several routines turn them off and up for
/// an inner iteration and restore them by hand afterwards, with no guard:
/// `Number::lambert_w` (`qalc-num/src/number/lambertw.rs:83`-`:87`) and
/// `Number::expint`'s complex branch
/// (`qalc-num/src/number/special.rs:1025`-`:1029`) both do
///
/// ```text
/// let bak = context::precision();
/// context::set_precision(bak * 2 + 20);
/// context::set_create_interval(false);
/// let outcome = …;                       // <- any early return or panic here
/// context::set_precision(bak);
/// context::set_create_interval(bak_iv);
/// ```
///
/// so a panic, or an early return added to the middle later, leaves the thread
/// at double precision with interval arithmetic off — for every expression
/// after it, in that process. Nothing in the transcripts would show it as
/// anything but unrelated wrong digits somewhere else.
///
/// The check is one comparison per case, so it runs over the whole corpus.
mod p8_interval_flag_survives {
    use super::*;

    pub const KNOWN_VIOLATIONS: &[(&str, &str)] = &[];

    /// The three thread-local settings an evaluation must give back.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Snapshot {
        interval: bool,
        precision: i32,
        calculation: qalc_num::context::IntervalCalculation,
    }

    pub fn snapshot() -> Snapshot {
        Snapshot {
            interval: qalc_num::context::create_interval(),
            precision: qalc_num::context::precision(),
            calculation: qalc_num::context::interval_calculation(),
        }
    }

    /// Run `f` and report every context setting it failed to restore.
    pub fn context_delta(f: impl FnOnce()) -> Vec<String> {
        let before = snapshot();
        f();
        let after = snapshot();
        let mut changed = Vec::new();
        if after.interval != before.interval {
            changed.push(format!(
                "create_interval: {} -> {}",
                before.interval, after.interval
            ));
        }
        if after.precision != before.precision {
            changed.push(format!(
                "precision: {} -> {}",
                before.precision, after.precision
            ));
        }
        if after.calculation != before.calculation {
            changed.push(format!(
                "interval_calculation: {:?} -> {:?}",
                before.calculation, after.calculation
            ));
        }
        changed
    }

    pub fn check(expr: String) -> harness::Verdict {
        if harness::parse(&expr).is_err() {
            return harness::Verdict::skipped("does not parse");
        }
        let changed = context_delta(|| {
            // Both modes, since the two hand-restored routines are reached
            // from different branches.
            let _ = harness::evaluate(&expr, ApproximationMode::Exact);
            let _ = harness::evaluate(&expr, ApproximationMode::Approximate);
        });
        if changed.is_empty() {
            harness::Verdict::Held
        } else {
            harness::Verdict::violated(format!(
                "evaluation left the numeric context modified: {}",
                changed.join(", ")
            ))
        }
    }
}

#[test]
fn evaluation_restores_the_numeric_context() {
    drive(
        "P8 numeric context survives",
        &corpus::all(),
        timeout_ms(),
        p8_interval_flag_survives::check,
        p8_interval_flag_survives::KNOWN_VIOLATIONS,
    );
}

/// P8 passes over the whole corpus today, which is only meaningful if the
/// check would fail were the flag actually dropped. This is the leak the
/// module documents — `set_create_interval(false)` with the restore skipped —
/// injected directly.
#[test]
fn p8_detects_a_dropped_restore() {
    harness::reset_context();
    let changed = p8_interval_flag_survives::context_delta(|| {
        qalc_num::context::set_create_interval(false);
        qalc_num::context::set_precision(qalc_num::context::precision() * 2 + 20);
        // The `set_precision(bak)` / `set_create_interval(bak_iv)` pair that
        // `lambertw.rs:86` runs is deliberately missing here.
    });
    harness::reset_context();
    assert_eq!(
        changed,
        vec![
            "create_interval: true -> false".to_string(),
            "precision: 10 -> 40".to_string(),
        ],
        "the P8 check did not notice a dropped restore"
    );
}

// =====================================================================
// the corpus itself
// =====================================================================

/// The corpus is an input to four tests, so its size is worth pinning: a
/// change to [`corpus::normalize`] that quietly drops half the transcripts
/// would turn all four green for the wrong reason.
#[test]
fn corpus_is_the_size_it_should_be() {
    let all = corpus::all();
    let ids: BTreeSet<&str> = all.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), all.len(), "case ids are not unique");
    assert!(
        all.len() >= 600,
        "corpus shrank to {} cases; the transcripts alone hold ~660 expressions",
        all.len()
    );
    let equations = corpus::equations();
    assert!(
        equations.len() >= corpus::CONSTRUCTED_EQUATIONS.len(),
        "only {} equations found, fewer than the {} constructed ones",
        equations.len(),
        corpus::CONSTRUCTED_EQUATIONS.len()
    );
    println!(
        "corpus: {} expressions ({} from transcripts, {} constructed), \
         {} of them equations in one unknown",
        all.len(),
        all.len() - corpus::CONSTRUCTED_EQUATIONS.len(),
        corpus::CONSTRUCTED_EQUATIONS.len(),
        equations.len()
    );
}
