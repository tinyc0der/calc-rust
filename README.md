# rust-calc

A pure-Rust port of [libqalculate](https://github.com/Qalculate/libqalculate) — the
library behind the `qalc` calculator — with **no C or C++ dependencies at all**.

## Goals

- Feature parity with libqalculate: arbitrary-precision arithmetic, units,
  currency conversion, symbolic algebra, matrices and vectors, statistics,
  combinatorics, date/time, and a `qalc`-equivalent CLI.
- **Zero FFI.** No `-sys` crates, no linkage against GMP, MPFR, libxml2,
  libcurl, ICU or readline.

## Workspace layout

| Crate | Replaces | Contents |
|-------|----------|----------|
| `crates/qalc-num` | GMP + MPFR + `Number.cc` | Arbitrary-precision integers, rationals and binary floats with interval bounds; complex numbers; the `Number` type, its parsing, printing, transcendental and special functions. |
| `crates/qalc-datetime` | `QalculateDateTime.cc` | Proleptic Gregorian dates, date arithmetic, ISO parsing and printing. |
| `crates/qalc-core` | the `libqalculate` library | Lexer, parser, `MathStructure`, the arithmetic merge engine, units, definitions loading, builtin functions, formatting. |
| `crates/qalc` | `src/qalc.cc` | CLI and batch transcript runner. |

## How the pure-Rust numeric core works

libqalculate leans on GMP for exact integers and MPFR for arbitrary-precision
floats. Here:

- **Exact values** use `num-bigint` / `num-rational`, so `1/3 + 1/6` stays the
  exact rational `1/2`.
- **Approximate values** use `astro-float` and carry an *interval* — a lower
  and upper bound rounded outward — mirroring libqalculate's interval
  arithmetic. Directed rounding is what makes the interval sound.
- **Special functions** (gamma, digamma, erf, zeta, Bernoulli numbers, the
  exponential and trigonometric integrals) have no pure-Rust equivalent to
  MPFR, so they are implemented directly in `qalc-num/src/number/special.rs`
  with precision-scaled series and guard bits.
- A float result that lands on an exact integer is demoted back to an exact
  rational, which is load-bearing for keeping arithmetic exact.

## Verification

The reference binary is the oracle. Every behavioural decision in this port is
checked against it rather than assumed, and the interesting ones are recorded
as tests next to the code.

1. **Transcript parity, enforced by `cargo test`.** `crates/qalc/tests/transcripts.rs`
   runs all 17 transcripts from `libqalculate/tests/` and fails the build on
   any mismatch. Its `KNOWN_FAILURES` list is empty, and the test fails both
   when a new case breaks *and* when a listed one starts passing, so the count
   cannot drift in either direction.

   ```
   cargo test -p qalc --test transcripts
   ```

   `scripts/parity.sh` runs the same transcripts through the built CLI and
   prints per-file scores instead of a pass/fail. It has no per-file timeout —
   a transcript that hangs will hang the script rather than be scored zero.

   ```
   ./scripts/parity.sh /path/to/libqalculate
   ```

2. **Unit tests.** `cargo test` — several hundred, most asserting
   oracle-verified output.

3. **Dependency audit.** `./scripts/check-pure-rust.sh` is the proof of the
   zero-C-linkage constraint. It gates on resolved `cargo metadata` (no `-sys`
   crate, no `libc`, no `links` key), then scans the source of every crate in
   the graph for `extern "C"`, `#[link]` and `build.rs` native linkage, then
   scans the workspace itself. Both metadata gates use `--locked`, so an
   unreviewed lockfile change fails here rather than quietly re-resolving.

## Status

**All 17 transcripts pass: 656/656 assertions, byte-identical to the reference
binary.** That includes the symbolic-algebra suites — limits at 181/181,
polynomial at 49/49, solver at 25/25, calculus at 11/11 — alongside
matrix/vector (130/130), stats (39/39), operators, bitwise, geometry, units,
dates, strings, number bases, percentages, parser and variables.

Parity is a regression test, not a milestone: `cargo test` fails if it drops.

The port is not the whole of libqalculate, though. Known gaps, each recorded in
the module that would own it:

- unit synchronization in the merge engine — `1 m + 1 cm` does not combine
  (`qalc-core/src/calculate.rs`)
- the builtin constants `pi`, `e`, `c` are not substituted for their values
- function-specific simplification identities (`sin(x)^2+cos(x)^2`, `ln(e^2)`)
- comparison evaluation (`5>2` is not reduced to `true`) and fraction
  reduction (`(x^2-1)/(x-1)`)
- quartic radicals in the solver; Bessel, Airy and polylogarithm in `qalc-num`
- the interactive niceties of `qalc.cc`: line editing, `save`, RPN mode
