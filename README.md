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

1. **Transcript parity.** `scripts/parity.sh` runs all 17 transcripts from
   `libqalculate/tests/` through this CLI and reports per-file scores:

   ```
   ./scripts/parity.sh /path/to/libqalculate
   ```

2. **Unit tests.** `cargo test` — several hundred, most asserting
   oracle-verified output.

3. **Dependency audit.** `cargo tree` shows no `-sys` crate, no `libc`, and no
   `build.rs` linking a system library.

## Status

This is an in-progress port. Fully passing transcripts include the operator,
bitwise, matrix/vector, geometry and variable suites; symbolic algebra
(limits, polynomial factoring, equation solving) is still being built out.
Run `scripts/parity.sh` for the current numbers.
