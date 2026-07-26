//! Golden transcripts for functions the reference test suite never calls.
//!
//! libqalculate's `data/functions.xml.in` defines 420 functions; the 17
//! `.batch` transcripts shipped with the reference invoke 141 of them. The
//! remaining 279 — including every hand-rolled replacement for MPFR in
//! `qalc-num`'s `number/special.rs` — had no oracle backing at all.
//!
//! The files under `tests/golden/` close that hole. They are ordinary
//! `.batch` transcripts in the reference's own format (expression at column
//! 0, tab-indented expectation on the next line) and every expectation was
//! captured from the reference binary under exactly the options
//! `qalc --test-file=` applies:
//!
//! ```text
//! TMPHOME=$(mktemp -d)
//! printf 'EXPR\n' | (cd .../libqalculate \
//!     && env HOME=$TMPHOME QALCULATE_DEFINITIONS_DIR=$PWD/data \
//!        ./src/qalc -t +u8 --defaults)
//! ```
//!
//! The `HOME=` assignment must apply to `qalc`, not to `printf`: with the
//! subshell dropped, `qalc` reads the developer's own config, evaluates at
//! precision 25, and the captured values are silently 25 digits wide instead
//! of 10. `1/7` is the canary — it must read `0.1428571429`.
//!
//! They run through [`qalc::cli::run_transcript_file`], which is the same
//! entry point `--test-file` and the reference-parity test in
//! `crates/qalc/tests/transcripts.rs` use, so a golden case and a reference
//! case are evaluated by identical code.
//!
//! [`KNOWN_FAILURES`] pins what does not match yet, and is checked in both
//! directions: an unlisted failure fails the test, and a listed case that has
//! started passing fails it too. Fixing a divergence therefore requires
//! deleting its line, so the list cannot rot.
//!
//! The five files assert 674 results, of which 423 match and 251 do not:
//!
//! | file                 | assertions | matching |
//! |----------------------|-----------:|---------:|
//! | `special.batch`      |        234 |      155 |
//! | `rounding.batch`     |        127 |       92 |
//! | `numbertheory.batch` |        117 |      116 |
//! | `bases.batch`        |        105 |       31 |
//! | `bitops.batch`       |         91 |       29 |
//!
//! The shape of the 251 matters more than the number, and it is now uniform:
//! every one of them is a function the port does not implement *saying so*.
//! 194 are calls to a name that is not a registered function and are rejected
//! outright; the rest are registered functions declining a case they cannot
//! evaluate and echoing themselves back. Both are visible to a user.
//!
//! It did not use to be. An unregistered name fell through to the
//! unknown-symbol path and the call silently became a product — `isprime(7)`
//! answered `7 isprime`, `bitget(12, 3)` answered `[(12 bitget)  (3 bitget)]`
//! — and when an argument was zero the product collapsed and the nonsense
//! became indistinguishable from an answer: `airy(0)` gave `0`, whose true
//! value is 0.3550280539. `parse_primary` now refuses a parenthesised
//! argument list after a name nothing answers to. Implicit multiplication of
//! *identifiers* is untouched: `abc` is still `are*barn*c`.
//!
//! The reassuring half: every special function the port *does* implement
//! agrees with MPFR to all ten printed digits over the arguments exercised
//! here — `gamma`, `digamma`, `erf`, `erfc`, `erfi`, `zeta`, `bernoulli`,
//! `Si`, `Ci`, `Chi`, `Shi`, `li`, `fresnelc`, `fresnels`, `betainc`,
//! `igamma` and `atan2`, at poles, at branch points, at negative and
//! rational arguments. `number/special.rs`'s hand-rolled series are sound;
//! what is missing is missing, not wrong.

mod generated_coverage {
    use std::path::{Path, PathBuf};

    use qalc::batch::Outcome;

    /// Cases that do not match the reference yet, as `(file, line, why)`.
    ///
    /// The `why` column is not read by the test; it is there so a reader can
    /// tell a missing feature from a wrong answer without re-running
    /// anything. Wrong *values* are the ones that matter — an unimplemented
    /// function echoing itself back is visible to a user, a special function
    /// silently off in the 4th digit is not.
    ///
    /// Entries are grouped by file and then by function, under a comment
    /// carrying the diagnosis for the whole group; the per-entry string is the
    /// short form of the same thing. Nothing here was fixed while it was
    /// written down — the point of the list is to say what is broken, not to
    /// hide it.
    const KNOWN_FAILURES: &[(&str, usize, &str)] = &[
        // ---------------- bases.batch ----------------
        // base: lowercase digits above 9 are not accepted (base("z", 36)); the number
        //   parser only recognises the uppercase forms.
        ("bases.batch", 52, "lowercase digits above 9 rejected by the base-n reader"),
        ("bases.batch", 70, "lowercase digits above 9 rejected by the base-n reader"),
        // roman: Roman numerals are absent, in both directions
        ("bases.batch", 74, "no roman: rejected as an unknown function"),
        ("bases.batch", 76, "no roman: rejected as an unknown function"),
        ("bases.batch", 78, "no roman: rejected as an unknown function"),
        ("bases.batch", 80, "no roman: rejected as an unknown function"),
        ("bases.batch", 82, "no roman: rejected as an unknown function"),
        ("bases.batch", 84, "no roman: rejected as an unknown function"),
        ("bases.batch", 86, "no roman: rejected as an unknown function"),
        ("bases.batch", 88, "no roman: rejected as an unknown function"),
        ("bases.batch", 90, "no roman: rejected as an unknown function"),
        ("bases.batch", 92, "no roman: rejected as an unknown function"),
        ("bases.batch", 94, "no roman: rejected as an unknown function"),
        // bijective: bijective base-26 is absent, in both directions
        ("bases.batch", 98, "no bijective: rejected as an unknown function"),
        ("bases.batch", 100, "no bijective: rejected as an unknown function"),
        ("bases.batch", 102, "no bijective: rejected as an unknown function"),
        ("bases.batch", 104, "no bijective: rejected as an unknown function"),
        ("bases.batch", 106, "no bijective: rejected as an unknown function"),
        ("bases.batch", 108, "no bijective: rejected as an unknown function"),
        ("bases.batch", 110, "no bijective: rejected as an unknown function"),
        ("bases.batch", 112, "no bijective: rejected as an unknown function"),
        ("bases.batch", 114, "no bijective: rejected as an unknown function"),
        ("bases.batch", 116, "no bijective: rejected as an unknown function"),
        // bcd: binary-coded decimal is absent
        ("bases.batch", 120, "no bcd: rejected as an unknown function"),
        ("bases.batch", 122, "no bcd: rejected as an unknown function"),
        ("bases.batch", 124, "no bcd: rejected as an unknown function"),
        ("bases.batch", 126, "no bcd: rejected as an unknown function"),
        ("bases.batch", 128, "no bcd: rejected as an unknown function"),
        ("bases.batch", 130, "no bcd: rejected as an unknown function"),
        ("bases.batch", 132, "no bcd: rejected as an unknown function"),
        ("bases.batch", 134, "no bcd: rejected as an unknown function"),
        // digitGet: positional digit read is absent
        ("bases.batch", 138, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 140, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 142, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 144, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 146, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 148, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 150, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 152, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 154, "no digitGet: rejected as an unknown function"),
        ("bases.batch", 156, "no digitGet: rejected as an unknown function"),
        // digitSet: positional digit write is absent
        ("bases.batch", 158, "no digitSet: rejected as an unknown function"),
        ("bases.batch", 160, "no digitSet: rejected as an unknown function"),
        ("bases.batch", 162, "no digitSet: rejected as an unknown function"),
        ("bases.batch", 164, "no digitSet: rejected as an unknown function"),
        ("bases.batch", 166, "no digitSet: rejected as an unknown function"),
        ("bases.batch", 168, "no digitSet: rejected as an unknown function"),
        // integerDigits: digit-vector expansion is absent
        ("bases.batch", 172, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 174, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 176, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 178, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 180, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 182, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 184, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 186, "no integerDigits: rejected as an unknown function"),
        ("bases.batch", 188, "no integerDigits: rejected as an unknown function"),
        // floatBits: IEEE 754 bit-pattern encode is absent
        ("bases.batch", 192, "no floatBits: rejected as an unknown function"),
        ("bases.batch", 194, "no floatBits: rejected as an unknown function"),
        ("bases.batch", 196, "no floatBits: rejected as an unknown function"),
        ("bases.batch", 198, "no floatBits: rejected as an unknown function"),
        ("bases.batch", 200, "no floatBits: rejected as an unknown function"),
        ("bases.batch", 202, "no floatBits: rejected as an unknown function"),
        ("bases.batch", 204, "no floatBits: rejected as an unknown function"),
        // floatParts: IEEE 754 sign/exponent/mantissa split is absent
        ("bases.batch", 206, "no floatParts: rejected as an unknown function"),
        ("bases.batch", 208, "no floatParts: rejected as an unknown function"),
        ("bases.batch", 210, "no floatParts: rejected as an unknown function"),
        ("bases.batch", 212, "no floatParts: rejected as an unknown function"),
        ("bases.batch", 214, "no floatParts: rejected as an unknown function"),
        ("bases.batch", 216, "no floatParts: rejected as an unknown function"),
        // floatValue: IEEE 754 bit-pattern decode is absent
        ("bases.batch", 218, "no floatValue: rejected as an unknown function"),
        ("bases.batch", 220, "no floatValue: rejected as an unknown function"),
        ("bases.batch", 222, "no floatValue: rejected as an unknown function"),
        ("bases.batch", 224, "no floatValue: rejected as an unknown function"),
        ("bases.batch", 226, "no floatValue: rejected as an unknown function"),
        // ---------------- bitops.batch ----------------
        // bitget: single-bit and bit-range read are absent
        ("bitops.batch", 2, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 4, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 6, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 8, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 10, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 12, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 14, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 16, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 18, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 20, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 22, "no bitget: rejected as an unknown function"),
        ("bitops.batch", 24, "no bitget: rejected as an unknown function"),
        // bitset: single-bit write is absent
        ("bitops.batch", 28, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 30, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 32, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 34, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 36, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 38, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 40, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 42, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 44, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 46, "no bitset: rejected as an unknown function"),
        ("bitops.batch", 48, "no bitset: rejected as an unknown function"),
        // bitcmp: width-limited one's complement is absent
        ("bitops.batch", 52, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 54, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 56, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 58, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 60, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 62, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 64, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 66, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 68, "no bitcmp: rejected as an unknown function"),
        ("bitops.batch", 70, "no bitcmp: rejected as an unknown function"),
        // bitrot: circular bit rotation is absent
        ("bitops.batch", 74, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 76, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 78, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 80, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 82, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 84, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 86, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 88, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 90, "no bitrot: rejected as an unknown function"),
        ("bitops.batch", 92, "no bitrot: rejected as an unknown function"),
        // setbits: bit-range write is absent
        ("bitops.batch", 118, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 120, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 122, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 124, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 126, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 128, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 130, "no setbits: rejected as an unknown function"),
        ("bitops.batch", 132, "no setbits: rejected as an unknown function"),
        // shift: the shift() function form is absent (the << and >> operators are not)
        ("bitops.batch", 136, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 138, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 140, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 142, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 144, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 146, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 148, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 150, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 152, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 154, "no shift(): rejected as an unknown function"),
        ("bitops.batch", 156, "no shift(): rejected as an unknown function"),
        // ---------------- numbertheory.batch ----------------
        // binomial: binomial with a non-integer first argument (the generalized binomial) is
        //   unimplemented.
        ("numbertheory.batch", 252, "generalized (non-integer n) binomial unimplemented"),
        // ---------------- rounding.batch ----------------
        // floor: a symbolic constant argument is not approximated first: floor(pi) is
        //   returned unevaluated.
        ("rounding.batch", 22, "symbolic constant argument not approximated; returned unevaluated"),
        ("rounding.batch", 24, "symbolic constant argument not approximated; returned unevaluated"),
        // ceil: a symbolic constant argument is not approximated first: ceil(pi) is
        //   returned unevaluated.
        ("rounding.batch", 46, "symbolic constant argument not approximated; returned unevaluated"),
        ("rounding.batch", 48, "symbolic constant argument not approximated; returned unevaluated"),
        // trunc: a symbolic constant argument is not approximated first: trunc(pi) is
        //   returned unevaluated.
        ("rounding.batch", 68, "symbolic constant argument not approximated; returned unevaluated"),
        // round: round(x, decimals) and round(x, decimals, method) are unimplemented - only
        //   the 1-argument default-mode form evaluates. The 11 IEEE-style rounding
        //   modes (half-to-even .. down) are all unreachable.
        ("rounding.batch", 120, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 124, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 126, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 128, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 130, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 132, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 134, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 136, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 138, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 140, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 144, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 146, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 148, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 150, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 152, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 154, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 156, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 158, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 160, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 162, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 164, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 166, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 168, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 170, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 172, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 174, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 176, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 178, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        ("rounding.batch", 180, "round(x, n) / round(x, n, mode) unimplemented; returned unevaluated"),
        // exp: exp does not recognise Euler's identity: exp(i*pi) is returned
        //   unevaluated.
        ("rounding.batch", 268, "exp(i*pi) not recognised; returned unevaluated"),
        // ---------------- special.batch ----------------
        // beta: Euler beta is absent entirely - no Number method and no function id
        ("special.batch", 70, "no beta: rejected as an unknown function"),
        ("special.batch", 72, "no beta: rejected as an unknown function"),
        ("special.batch", 74, "no beta: rejected as an unknown function"),
        ("special.batch", 76, "no beta: rejected as an unknown function"),
        ("special.batch", 78, "no beta: rejected as an unknown function"),
        ("special.batch", 80, "no beta: rejected as an unknown function"),
        ("special.batch", 82, "no beta: rejected as an unknown function"),
        ("special.batch", 84, "no beta: rejected as an unknown function"),
        ("special.batch", 86, "no beta: rejected as an unknown function"),
        ("special.batch", 88, "no beta: rejected as an unknown function"),
        ("special.batch", 498, "no beta: rejected as an unknown function"),
        // igamma: igamma is undefined for a negative first argument, and igamma(5, 0) loses
        //   exactness (24.00000000 rather than the exact 24 the reference keeps).
        ("special.batch", 104, "negative order unported; igamma(5, 0) also loses exactness"),
        ("special.batch", 106, "negative order unported; igamma(5, 0) also loses exactness"),
        // betaincinv: inverse regularized incomplete beta is absent (betainc itself is present)
        ("special.batch", 122, "no betaincinv: rejected as an unknown function"),
        ("special.batch", 124, "no betaincinv: rejected as an unknown function"),
        ("special.batch", 126, "no betaincinv: rejected as an unknown function"),
        ("special.batch", 128, "no betaincinv: rejected as an unknown function"),
        ("special.batch", 130, "no betaincinv: rejected as an unknown function"),
        ("special.batch", 132, "no betaincinv: rejected as an unknown function"),
        // erfinv: erfinv_f64 exists in qalc-core/src/stats.rs but no function id maps to it.
        //   `erfinv(0)` used to "pass" only because the unknown name collapsed the
        //   product to zero, which happened to be the right answer; the call is now
        //   rejected outright, so it is a visible failure like the rest of the group.
        ("special.batch", 180, "no erfinv: rejected as an unknown function"),
        ("special.batch", 182, "no erfinv: rejected as an unknown function"),
        ("special.batch", 184, "no erfinv: rejected as an unknown function"),
        ("special.batch", 186, "no erfinv: rejected as an unknown function"),
        ("special.batch", 188, "no erfinv: rejected as an unknown function"),
        ("special.batch", 190, "no erfinv: rejected as an unknown function"),
        ("special.batch", 192, "no erfinv: rejected as an unknown function"),
        ("special.batch", 194, "no erfinv: rejected as an unknown function"),
        // zeta: only the 1-argument Riemann zeta evaluates; the 2-argument Hurwitz form is
        //   unimplemented.
        ("special.batch", 226, "2-argument Hurwitz form unimplemented; returned unevaluated"),
        ("special.batch", 228, "2-argument Hurwitz form unimplemented; returned unevaluated"),
        ("special.batch", 230, "2-argument Hurwitz form unimplemented; returned unevaluated"),
        // besselj: unregistered; qalc-num Number::besselj is a `false` stub (special.rs:1240)
        ("special.batch", 260, "no besselj: rejected as an unknown function"),
        ("special.batch", 262, "no besselj: rejected as an unknown function"),
        ("special.batch", 264, "no besselj: rejected as an unknown function"),
        ("special.batch", 266, "no besselj: rejected as an unknown function"),
        ("special.batch", 268, "no besselj: rejected as an unknown function"),
        ("special.batch", 270, "no besselj: rejected as an unknown function"),
        ("special.batch", 272, "no besselj: rejected as an unknown function"),
        ("special.batch", 274, "no besselj: rejected as an unknown function"),
        ("special.batch", 276, "no besselj: rejected as an unknown function"),
        ("special.batch", 278, "no besselj: rejected as an unknown function"),
        ("special.batch", 490, "no besselj: rejected as an unknown function"),
        // bessely: unregistered; qalc-num Number::bessely is a `false` stub (special.rs:1245)
        ("special.batch", 280, "no bessely: rejected as an unknown function"),
        ("special.batch", 282, "no bessely: rejected as an unknown function"),
        ("special.batch", 284, "no bessely: rejected as an unknown function"),
        ("special.batch", 286, "no bessely: rejected as an unknown function"),
        ("special.batch", 288, "no bessely: rejected as an unknown function"),
        ("special.batch", 290, "no bessely: rejected as an unknown function"),
        ("special.batch", 292, "no bessely: rejected as an unknown function"),
        // airy: unregistered; qalc-num Number::airy is a `false` stub (special.rs:1250)
        ("special.batch", 296, "no airy: rejected as an unknown function"),
        ("special.batch", 298, "no airy: rejected as an unknown function"),
        ("special.batch", 300, "no airy: rejected as an unknown function"),
        ("special.batch", 302, "no airy: rejected as an unknown function"),
        ("special.batch", 304, "no airy: rejected as an unknown function"),
        ("special.batch", 306, "no airy: rejected as an unknown function"),
        ("special.batch", 308, "no airy: rejected as an unknown function"),
        ("special.batch", 310, "no airy: rejected as an unknown function"),
        ("special.batch", 312, "no airy: rejected as an unknown function"),
        // Si: a symbolic constant argument is not approximated first, and complex
        //   arguments are unported.
        ("special.batch", 324, "symbolic-constant or complex argument; returned unevaluated"),
        ("special.batch", 494, "symbolic-constant or complex argument; returned unevaluated"),
        // Ci: a symbolic constant argument is not approximated first, and complex
        //   arguments are unported.
        ("special.batch", 338, "symbolic-constant or complex argument; returned unevaluated"),
        ("special.batch", 496, "symbolic-constant or complex argument; returned unevaluated"),
        // Chi: Chi is real-only: x < 0 (which is complex) and x = 0 (which is -infinity)
        //   both come back unevaluated.
        ("special.batch", 348, "complex / -infinity branch unported; returned unevaluated"),
        ("special.batch", 350, "complex / -infinity branch unported; returned unevaluated"),
        // Li: unregistered; qalc-num Number::polylog is a `false` stub (special.rs:1255).
        //   `Li` is not a known name, so the call is rejected rather than read as L*i.
        ("special.batch", 370, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 372, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 374, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 376, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 378, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 380, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 382, "no Li/polylog: rejected as an unknown function"),
        ("special.batch", 492, "no Li/polylog: rejected as an unknown function"),
        // sinc / cis: both evaluate now, but only for an argument that reduces to a
        //   number. `pi` stays symbolic in this port, so every case whose argument is
        //   written in terms of pi is still returned unevaluated.
        ("special.batch", 412, "sinc(pi): pi is not approximated; returned unevaluated"),
        ("special.batch", 444, "cis(pi): pi is not approximated; returned unevaluated"),
        ("special.batch", 446, "cis(pi/2): pi is not approximated; returned unevaluated"),
        ("special.batch", 448, "cis(pi/4): pi is not approximated; returned unevaluated"),
        ("special.batch", 452, "cis(-pi/2): pi is not approximated; returned unevaluated"),
        // erf: erf/erfc/erfi are real-only; the reference evaluates them over the whole
        //   complex plane.
        ("special.batch", 482, "complex argument unported; returned unevaluated"),
        // erfc: erf/erfc/erfi are real-only; the reference evaluates them over the whole
        //   complex plane.
        ("special.batch", 484, "complex argument unported; returned unevaluated"),
        // erfi: erf/erfc/erfi are real-only; the reference evaluates them over the whole
        //   complex plane.
        ("special.batch", 486, "complex argument unported; returned unevaluated"),
    ];

    /// Every `.batch` file in `tests/golden/`.
    fn golden_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("golden")
    }

    /// Run one transcript through the CLI's own evaluation path, returning the
    /// 1-based line of every case that differs.
    fn failing_lines(path: &Path) -> Vec<(usize, String)> {
        let report = qalc::cli::run_transcript_file(path).expect("transcript is readable");
        report
            .results
            .iter()
            .filter(|(_, outcome)| *outcome != Outcome::Pass)
            .map(|(case, outcome)| {
                let detail = match outcome {
                    Outcome::Mismatch { got } => format!(
                        "{}\n    expected: {}\n    got:      {}",
                        case.expression,
                        case.expected.as_deref().unwrap_or(""),
                        got
                    ),
                    Outcome::Error { message } => {
                        format!("{}\n    error: {message}", case.expression)
                    }
                    Outcome::Pass => unreachable!(),
                };
                (case.line, detail)
            })
            .collect()
    }

    #[test]
    fn every_golden_transcript_matches() {
        let dir = golden_dir();
        let mut batches: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("golden directory is readable")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "batch"))
            .collect();
        batches.sort();
        assert!(!batches.is_empty(), "no .batch files in {}", dir.display());

        let mut unexpected: Vec<String> = Vec::new();
        let mut fixed: Vec<String> = Vec::new();

        for path in &batches {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let failures = failing_lines(path);
            let failed_lines: Vec<usize> = failures.iter().map(|(line, _)| *line).collect();

            for (line, detail) in &failures {
                if !KNOWN_FAILURES
                    .iter()
                    .any(|(f, l, _)| *f == name && l == line)
                {
                    unexpected.push(format!("{name}:{line}  {detail}"));
                }
            }
            for (known_file, known_line, _) in KNOWN_FAILURES {
                if *known_file == name && !failed_lines.contains(known_line) {
                    fixed.push(format!("{known_file}:{known_line}"));
                }
            }
        }

        assert!(
            unexpected.is_empty(),
            "golden coverage regressed:\n{}",
            unexpected.join("\n")
        );
        assert!(
            fixed.is_empty(),
            "these cases now pass — remove them from KNOWN_FAILURES so the \
             count cannot creep back up:\n{}",
            fixed.join("\n")
        );
    }

    /// The transcripts must actually assert something.
    ///
    /// A `.batch` file whose expectations are space-indented parses as a list
    /// of setup lines with nothing to compare, and the suite passes green
    /// while testing nothing. The parser records those lines; this fails on
    /// them, and on a file that somehow carries no assertions at all.
    #[test]
    fn every_golden_transcript_asserts_something() {
        for entry in std::fs::read_dir(golden_dir()).expect("golden directory is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.extension().is_none_or(|e| e != "batch") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("transcript is readable");
            let transcript = qalc::batch::parse_transcript(&source);
            assert!(
                transcript.space_indented.is_empty(),
                "{}: expectations on lines {:?} are space-indented and assert \
                 nothing — they must start with a TAB",
                path.display(),
                transcript.space_indented
            );
            let asserted = transcript
                .cases
                .iter()
                .filter(|c| c.expected.is_some())
                .count();
            assert!(asserted > 0, "{}: no assertions", path.display());
        }
    }
}
