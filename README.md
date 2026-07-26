# rust-calc

A pure-Rust port of [libqalculate](https://github.com/Qalculate/libqalculate) — the library behind the `qalc` calculator.

## Goals

- Full feature parity with libqalculate: arbitrary-precision arithmetic, units,
  currency conversion, symbolic algebra (differentiation, integration, factoring),
  matrices/vectors, statistics, combinatorics, date/time, plotting hooks, and a
  `qalc`-equivalent CLI.
- **Zero C/C++ FFI.** No `-sys` crates, no linkage against GMP, MPFR, libxml2,
  libcurl, ICU, or readline. Pure-Rust dependencies only.

## Workspace layout

| Crate | Replaces | Contents |
|-------|----------|----------|
| `crates/qalc-num` | GMP + MPFR + `Number.cc` | Arbitrary-precision integers, rationals, binary floats with interval bounds, complex numbers; the `Number` type and its printing. |
| `crates/qalc-core` | `libqalculate` library | Parser, `MathStructure`, `Calculator`, units, variables, builtin functions, definitions loading, formatting. |
| `crates/qalc` | `src/qalc.cc` | Interactive CLI / batch runner. |

## Verification

1. Every transcript in `libqalculate/tests/*.batch` produces byte-identical
   output between the original `qalc` and this CLI.
2. libqalculate's C++ unit-test assertions ported to Rust tests.
3. `cargo tree` shows no `-sys` crate and no `build.rs` linking a C library.
