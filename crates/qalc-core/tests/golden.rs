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
//! As written, the five files assert 674 results, of which 280 match and 394
//! do not:
//!
//! | file                 | assertions | matching |
//! |----------------------|-----------:|---------:|
//! | `special.batch`      |        234 |      138 |
//! | `rounding.batch`     |        127 |       84 |
//! | `numbertheory.batch` |        117 |       32 |
//! | `bases.batch`        |        105 |       20 |
//! | `bitops.batch`       |         91 |        6 |
//!
//! The shape of the 394 matters more than the number. Only ~50 are a function
//! declining to evaluate and echoing itself back — the visible, harmless kind.
//! The large majority is one failure mode repeated: a function the port never
//! registers, whose name therefore falls through to the unknown-symbol path
//! and turns the call into implicit multiplication. `isprime(7)` does not
//! raise; it answers `7 isprime`. `bitget(12, 3)` answers
//! `[(12 bitget)  (3 bitget)]`. When a zero is involved the product collapses
//! and the nonsense becomes indistinguishable from an answer: `airy(0)` gives
//! `0` (the true value is 0.3550280539) and `nextprime(0)` gives `0`.
//!
//! The single worst case is `psi(4)`: `psi` is the reference's alias for
//! digamma, but the port resolves it to the pressure unit, so a valid call
//! returns `27579.02917 Pa` instead of `1.256117668`.
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
        // bin: bin(x, 1) has the WRONG SEMANTICS: the reference reads x as a
        //   two's-complement binary literal and returns a number, the port formats x
        //   as a binary string. bin(x, 0, 1) (reverse conversion) returns 0.
        ("bases.batch", 12, "WRONG SEMANTICS: bin(x, 1) formats a string instead of reading two's complement"),
        ("bases.batch", 14, "WRONG SEMANTICS: bin(x, 1) formats a string instead of reading two's complement"),
        ("bases.batch", 16, "WRONG SEMANTICS: bin(x, 1) formats a string instead of reading two's complement"),
        ("bases.batch", 18, "WRONG SEMANTICS: bin(x, 1) formats a string instead of reading two's complement"),
        ("bases.batch", 22, "WRONG SEMANTICS: bin(x, 1) formats a string instead of reading two's complement"),
        // oct: oct(x, 1) (reverse conversion) returns a quoted string where the reference
        //   returns the octal value as a number.
        ("bases.batch", 34, "reverse conversion returns a quoted string, not a number"),
        ("bases.batch", 36, "reverse conversion returns a quoted string, not a number"),
        // base: the reverse conversion base(n, b, digits, 1) is not implemented and,
        //   worse, prints under the name `f`; and lowercase digits above 9 are not
        //   accepted (base("z", 36)).
        ("bases.batch", 52, "reverse conversion unimplemented and misprinted as `f`; lowercase digits rejected"),
        ("bases.batch", 56, "reverse conversion unimplemented and misprinted as `f`; lowercase digits rejected"),
        ("bases.batch", 58, "reverse conversion unimplemented and misprinted as `f`; lowercase digits rejected"),
        ("bases.batch", 60, "reverse conversion unimplemented and misprinted as `f`; lowercase digits rejected"),
        ("bases.batch", 68, "reverse conversion unimplemented and misprinted as `f`; lowercase digits rejected"),
        ("bases.batch", 70, "reverse conversion unimplemented and misprinted as `f`; lowercase digits rejected"),
        // roman: Roman numerals are absent, in both directions
        ("bases.batch", 74, "no roman: parses as a free symbol"),
        ("bases.batch", 76, "no roman: parses as a free symbol"),
        ("bases.batch", 78, "no roman: parses as a free symbol"),
        ("bases.batch", 80, "no roman: parses as a free symbol"),
        ("bases.batch", 82, "no roman: parses as a free symbol"),
        ("bases.batch", 84, "no roman: parses as a free symbol"),
        ("bases.batch", 86, "no roman: parses as a free symbol"),
        ("bases.batch", 88, "no roman: parses as a free symbol"),
        ("bases.batch", 90, "no roman: parses as a free symbol"),
        ("bases.batch", 92, "no roman: parses as a free symbol"),
        ("bases.batch", 94, "no roman: parses as a free symbol"),
        // bijective: bijective base-26 is absent, in both directions
        ("bases.batch", 98, "no bijective: parses as a free symbol"),
        ("bases.batch", 100, "no bijective: parses as a free symbol"),
        ("bases.batch", 102, "no bijective: parses as a free symbol"),
        ("bases.batch", 104, "no bijective: parses as a free symbol"),
        ("bases.batch", 106, "no bijective: parses as a free symbol"),
        ("bases.batch", 108, "no bijective: parses as a free symbol"),
        ("bases.batch", 110, "no bijective: parses as a free symbol"),
        ("bases.batch", 112, "no bijective: parses as a free symbol"),
        ("bases.batch", 114, "no bijective: parses as a free symbol"),
        ("bases.batch", 116, "no bijective: parses as a free symbol"),
        // bcd: binary-coded decimal is absent; parses as b*c*d
        ("bases.batch", 120, "no bcd: parses as b*c*d"),
        ("bases.batch", 122, "no bcd: parses as b*c*d"),
        ("bases.batch", 124, "no bcd: parses as b*c*d"),
        ("bases.batch", 126, "no bcd: parses as b*c*d"),
        ("bases.batch", 128, "no bcd: parses as b*c*d"),
        ("bases.batch", 130, "no bcd: parses as b*c*d"),
        ("bases.batch", 132, "no bcd: parses as b*c*d"),
        ("bases.batch", 134, "no bcd: parses as b*c*d"),
        // digitGet: positional digit read is absent
        ("bases.batch", 138, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 140, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 142, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 144, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 146, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 148, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 150, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 152, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 154, "no digitGet: args become a vector of free symbols"),
        ("bases.batch", 156, "no digitGet: args become a vector of free symbols"),
        // digitSet: positional digit write is absent
        ("bases.batch", 158, "no digitSet: args become a vector of free symbols"),
        ("bases.batch", 160, "no digitSet: args become a vector of free symbols"),
        ("bases.batch", 162, "no digitSet: args become a vector of free symbols"),
        ("bases.batch", 164, "no digitSet: args become a vector of free symbols"),
        ("bases.batch", 166, "no digitSet: args become a vector of free symbols"),
        ("bases.batch", 168, "no digitSet: args become a vector of free symbols"),
        // integerDigits: digit-vector expansion is absent
        ("bases.batch", 172, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 174, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 176, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 178, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 180, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 182, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 184, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 186, "no integerDigits: parses as a free symbol"),
        ("bases.batch", 188, "no integerDigits: parses as a free symbol"),
        // floatBits: IEEE 754 bit-pattern encode is absent
        ("bases.batch", 192, "no floatBits: args become a vector of free symbols"),
        ("bases.batch", 194, "no floatBits: args become a vector of free symbols"),
        ("bases.batch", 196, "no floatBits: args become a vector of free symbols"),
        ("bases.batch", 198, "no floatBits: args become a vector of free symbols"),
        ("bases.batch", 200, "no floatBits: args become a vector of free symbols"),
        ("bases.batch", 202, "no floatBits: args become a vector of free symbols"),
        ("bases.batch", 204, "no floatBits: args become a vector of free symbols"),
        // floatParts: IEEE 754 sign/exponent/mantissa split is absent
        ("bases.batch", 206, "no floatParts: parses as a free symbol"),
        ("bases.batch", 208, "no floatParts: parses as a free symbol"),
        ("bases.batch", 210, "no floatParts: parses as a free symbol"),
        ("bases.batch", 212, "no floatParts: parses as a free symbol, and the product collapses to a plausible-looking number"),
        ("bases.batch", 214, "no floatParts: parses as a free symbol"),
        ("bases.batch", 216, "no floatParts: parses as a free symbol"),
        // floatValue: IEEE 754 bit-pattern decode is absent
        ("bases.batch", 218, "no floatValue: args become a vector of free symbols"),
        ("bases.batch", 220, "no floatValue: args become a vector of free symbols"),
        ("bases.batch", 222, "no floatValue: args become a vector of free symbols"),
        ("bases.batch", 224, "no floatValue: args become a vector of free symbols"),
        ("bases.batch", 226, "no floatValue: args become a vector of free symbols"),
        // ---------------- bitops.batch ----------------
        // bitget: single-bit and bit-range read are absent
        ("bitops.batch", 2, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 4, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 6, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 8, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 10, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 12, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 14, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 16, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 18, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 20, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 22, "no bitget: args become a vector of free symbols"),
        ("bitops.batch", 24, "no bitget: args become a vector of free symbols"),
        // bitset: single-bit write is absent
        ("bitops.batch", 28, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 30, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 32, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 34, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 36, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 38, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 40, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 42, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 44, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 46, "no bitset: args become a vector of free symbols"),
        ("bitops.batch", 48, "no bitset: args become a vector of free symbols"),
        // bitcmp: width-limited one's complement is absent
        ("bitops.batch", 52, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 54, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 56, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 58, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 60, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 62, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 64, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 66, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 68, "no bitcmp: args become a vector of free symbols"),
        ("bitops.batch", 70, "no bitcmp: args become a vector of free symbols"),
        // bitrot: circular bit rotation is absent
        ("bitops.batch", 74, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 76, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 78, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 80, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 82, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 84, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 86, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 88, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 90, "no bitrot: args become a vector of free symbols"),
        ("bitops.batch", 92, "no bitrot: args become a vector of free symbols"),
        // popCount: population count is absent
        ("bitops.batch", 96, "no popCount: parses as a free symbol"),
        ("bitops.batch", 100, "no popCount: parses as a free symbol"),
        ("bitops.batch", 102, "no popCount: parses as a free symbol"),
        ("bitops.batch", 104, "no popCount: parses as a free symbol"),
        ("bitops.batch", 106, "no popCount: parses as a free symbol"),
        ("bitops.batch", 108, "no popCount: parses as a free symbol"),
        ("bitops.batch", 110, "no popCount: parses as a free symbol"),
        ("bitops.batch", 112, "no popCount: parses as a free symbol"),
        ("bitops.batch", 114, "no popCount: parses as a free symbol"),
        // setbits: bit-range write is absent
        ("bitops.batch", 118, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 120, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 122, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 124, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 126, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 128, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 130, "no setbits: args become a vector of free symbols"),
        ("bitops.batch", 132, "no setbits: args become a vector of free symbols"),
        // shift: the shift() function form is absent (the << and >> operators are not)
        ("bitops.batch", 136, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 138, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 140, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 142, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 144, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 146, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 148, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 150, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 152, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 154, "no shift(): args become a vector of free symbols"),
        ("bitops.batch", 156, "no shift(): args become a vector of free symbols"),
        // xor: `xor` lexes only as an infix operator, so the function-call form xor(a, b)
        //   is a parse error - the reference accepts both
        ("bitops.batch", 160, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 162, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 164, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 166, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 168, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 170, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 172, "xor(a, b) is a parse error; only infix xor lexes"),
        ("bitops.batch", 174, "xor(a, b) is a parse error; only infix xor lexes"),
        // lxor: logical xor is absent as a function
        ("bitops.batch", 176, "no lxor: args become a vector of free symbols"),
        ("bitops.batch", 178, "no lxor: args become a vector of free symbols"),
        ("bitops.batch", 180, "no lxor: args become a vector of free symbols"),
        ("bitops.batch", 182, "no lxor: args become a vector of free symbols"),
        ("bitops.batch", 184, "no lxor: args become a vector of free symbols"),
        ("bitops.batch", 186, "no lxor: args become a vector of free symbols"),
        // ---------------- numbertheory.batch ----------------
        // isprime: no primality test is registered
        ("numbertheory.batch", 2, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 4, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 6, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 8, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 12, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 14, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 16, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 18, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 20, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 22, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 24, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 26, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 28, "no isprime: parses as a free symbol"),
        ("numbertheory.batch", 30, "no isprime: parses as a free symbol"),
        // nextprime: no prime successor is registered
        ("numbertheory.batch", 34, "no nextprime: parses as a free symbol"),
        ("numbertheory.batch", 36, "no nextprime: parses as a free symbol"),
        ("numbertheory.batch", 38, "no nextprime: parses as a free symbol"),
        ("numbertheory.batch", 40, "no nextprime: parses as a free symbol"),
        ("numbertheory.batch", 42, "no nextprime: parses as a free symbol, and the product collapses to a plausible-looking number"),
        ("numbertheory.batch", 44, "no nextprime: parses as a free symbol"),
        ("numbertheory.batch", 46, "no nextprime: parses as a free symbol"),
        ("numbertheory.batch", 48, "no nextprime: parses as a free symbol"),
        // prevprime: no prime predecessor is registered
        ("numbertheory.batch", 50, "no prevprime: parses as a free symbol"),
        ("numbertheory.batch", 52, "no prevprime: parses as a free symbol"),
        ("numbertheory.batch", 54, "no prevprime: parses as a free symbol"),
        ("numbertheory.batch", 56, "no prevprime: parses as a free symbol"),
        ("numbertheory.batch", 58, "no prevprime: parses as a free symbol"),
        ("numbertheory.batch", 60, "no prevprime: parses as a free symbol"),
        // nthprime: no nth-prime is registered
        ("numbertheory.batch", 64, "no nthprime: parses as a free symbol"),
        ("numbertheory.batch", 66, "no nthprime: parses as a free symbol"),
        ("numbertheory.batch", 68, "no nthprime: parses as a free symbol"),
        ("numbertheory.batch", 70, "no nthprime: parses as a free symbol"),
        ("numbertheory.batch", 72, "no nthprime: parses as a free symbol"),
        ("numbertheory.batch", 74, "no nthprime: parses as a free symbol, and the product collapses to a plausible-looking number"),
        ("numbertheory.batch", 76, "no nthprime: parses as a free symbol"),
        ("numbertheory.batch", 78, "no nthprime: parses as a free symbol"),
        // prime_pi: no prime-counting function is registered
        ("numbertheory.batch", 82, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 84, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 86, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 88, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 92, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 94, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 96, "no prime_pi: parses as a free symbol"),
        ("numbertheory.batch", 98, "no prime_pi: parses as a free symbol"),
        // primes: no prime-sieve function is registered
        ("numbertheory.batch", 102, "no primes: parses as a free symbol"),
        ("numbertheory.batch", 104, "no primes: parses as a free symbol"),
        ("numbertheory.batch", 106, "no primes: parses as a free symbol"),
        ("numbertheory.batch", 108, "no primes: parses as a free symbol"),
        ("numbertheory.batch", 110, "no primes: parses as a free symbol, and the product collapses to a plausible-looking number"),
        ("numbertheory.batch", 112, "no primes: parses as a free symbol"),
        ("numbertheory.batch", 114, "no primes: parses as a free symbol"),
        // divisors: no divisor list is registered; Number::factorize (integer.rs:484) is
        //   reachable from nothing
        ("numbertheory.batch", 118, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 120, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 122, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 124, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 126, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 128, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 130, "no divisors: parses as a free symbol, and the product collapses to a plausible-looking number"),
        ("numbertheory.batch", 132, "no divisors: parses as a free symbol"),
        ("numbertheory.batch", 134, "no divisors: parses as a free symbol"),
        // lcm: lcm takes exactly 2 integer arguments: 3 arguments and rational arguments
        //   are unimplemented, and lcm(0, 5) WRONGLY returns 0 where the reference
        //   declines to evaluate.
        ("numbertheory.batch", 142, "3-argument / rational lcm unimplemented; lcm(0, 5) returns a WRONG 0"),
        ("numbertheory.batch", 146, "3-argument / rational lcm unimplemented; lcm(0, 5) returns a WRONG 0"),
        ("numbertheory.batch", 148, "3-argument / rational lcm unimplemented; lcm(0, 5) returns a WRONG 0"),
        // HCF: gcd exists under `gcd`, but the `HCF` alias is missing, so it parses as
        //   H*C*F
        ("numbertheory.batch", 152, "HCF alias missing: parses as H*C*F"),
        ("numbertheory.batch", 154, "HCF alias missing: parses as H*C*F"),
        ("numbertheory.batch", 156, "HCF alias missing: parses as H*C*F"),
        ("numbertheory.batch", 158, "HCF alias missing: parses as H*C*F"),
        ("numbertheory.batch", 160, "HCF alias missing: parses as H*C*F"),
        ("numbertheory.batch", 162, "HCF alias missing: parses as H*C*F"),
        ("numbertheory.batch", 164, "HCF alias missing: parses as H*C*F"),
        // powmod: modular exponentiation is absent
        ("numbertheory.batch", 168, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 170, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 172, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 174, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 176, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 178, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 180, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 182, "no powmod: args become a vector of free symbols"),
        ("numbertheory.batch", 184, "no powmod: args become a vector of free symbols"),
        // multifactorial: n-th order factorial is absent (factorial and factorial2 are present)
        ("numbertheory.batch", 220, "no multifactorial: args become a vector of free symbols"),
        ("numbertheory.batch", 222, "no multifactorial: args become a vector of free symbols"),
        ("numbertheory.batch", 224, "no multifactorial: args become a vector of free symbols"),
        ("numbertheory.batch", 226, "no multifactorial: args become a vector of free symbols"),
        ("numbertheory.batch", 228, "no multifactorial: args become a vector of free symbols"),
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
        // sq: sq(x) = x^2 is absent; parses as s*q
        ("rounding.batch", 230, "no sq: parses as s*q"),
        ("rounding.batch", 232, "no sq: parses as s*q"),
        ("rounding.batch", 236, "no sq: parses as s*q"),
        ("rounding.batch", 238, "no sq: parses as s*q"),
        ("rounding.batch", 240, "no sq: parses as s*q"),
        ("rounding.batch", 242, "no sq: parses as s*q"),
        ("rounding.batch", 244, "no sq: parses as s*q"),
        ("rounding.batch", 246, "no sq: parses as s*q"),
        // exp: exp does not recognise Euler's identity: exp(i*pi) is returned
        //   unevaluated.
        ("rounding.batch", 268, "exp(i*pi) not recognised; returned unevaluated"),
        // ---------------- special.batch ----------------
        // psi: `psi` is the digamma alias in the reference; the port resolves it to the
        //   pressure unit instead
        ("special.batch", 66, "name collision: psi resolves to the pressure unit"),
        // beta: Euler beta is absent entirely - no Number method and no function id
        ("special.batch", 70, "no beta: args become a vector of free symbols"),
        ("special.batch", 72, "no beta: args become a vector of free symbols"),
        ("special.batch", 74, "no beta: args become a vector of free symbols"),
        ("special.batch", 76, "no beta: args become a vector of free symbols"),
        ("special.batch", 78, "no beta: args become a vector of free symbols"),
        ("special.batch", 80, "no beta: args become a vector of free symbols"),
        ("special.batch", 82, "no beta: args become a vector of free symbols"),
        ("special.batch", 84, "no beta: args become a vector of free symbols"),
        ("special.batch", 86, "no beta: args become a vector of free symbols"),
        ("special.batch", 88, "no beta: args become a vector of free symbols"),
        ("special.batch", 498, "no beta: args become a vector of free symbols"),
        // igamma: igamma is undefined for a negative first argument, and igamma(5, 0) loses
        //   exactness (24.00000000 rather than the exact 24 the reference keeps).
        ("special.batch", 104, "negative order unported; igamma(5, 0) also loses exactness"),
        ("special.batch", 106, "negative order unported; igamma(5, 0) also loses exactness"),
        // betaincinv: inverse regularized incomplete beta is absent (betainc itself is present)
        ("special.batch", 122, "no betaincinv: args become a vector of free symbols"),
        ("special.batch", 124, "no betaincinv: args become a vector of free symbols"),
        ("special.batch", 126, "no betaincinv: args become a vector of free symbols"),
        ("special.batch", 128, "no betaincinv: args become a vector of free symbols"),
        ("special.batch", 130, "no betaincinv: args become a vector of free symbols"),
        ("special.batch", 132, "no betaincinv: args become a vector of free symbols"),
        // erfinv: erfinv_f64 exists in qalc-core/src/stats.rs but no function id maps to it
        ("special.batch", 182, "no erfinv: parses as a free symbol"),
        ("special.batch", 184, "no erfinv: parses as a free symbol"),
        ("special.batch", 186, "no erfinv: parses as a free symbol"),
        ("special.batch", 188, "no erfinv: parses as a free symbol"),
        ("special.batch", 190, "no erfinv: parses as a free symbol"),
        ("special.batch", 192, "no erfinv: parses as a free symbol"),
        ("special.batch", 194, "no erfinv: parses as a free symbol"),
        // zeta: only the 1-argument Riemann zeta evaluates; the 2-argument Hurwitz form is
        //   unimplemented.
        ("special.batch", 226, "2-argument Hurwitz form unimplemented; returned unevaluated"),
        ("special.batch", 228, "2-argument Hurwitz form unimplemented; returned unevaluated"),
        ("special.batch", 230, "2-argument Hurwitz form unimplemented; returned unevaluated"),
        // besselj: unregistered; qalc-num Number::besselj is a `false` stub (special.rs:1240)
        ("special.batch", 260, "no besselj: args become a vector of free symbols"),
        ("special.batch", 262, "no besselj: args become a vector of free symbols"),
        ("special.batch", 264, "no besselj: args become a vector of free symbols"),
        ("special.batch", 266, "no besselj: args become a vector of free symbols"),
        ("special.batch", 268, "no besselj: args become a vector of free symbols"),
        ("special.batch", 270, "no besselj: args become a vector of free symbols"),
        ("special.batch", 272, "no besselj: args become a vector of free symbols"),
        ("special.batch", 274, "no besselj: args become a vector of free symbols"),
        ("special.batch", 276, "no besselj: args become a vector of free symbols"),
        ("special.batch", 278, "no besselj: args become a vector of free symbols"),
        ("special.batch", 490, "no besselj: args become a vector of free symbols"),
        // bessely: unregistered; qalc-num Number::bessely is a `false` stub (special.rs:1245)
        ("special.batch", 280, "no bessely: args become a vector of free symbols"),
        ("special.batch", 282, "no bessely: args become a vector of free symbols"),
        ("special.batch", 284, "no bessely: args become a vector of free symbols"),
        ("special.batch", 286, "no bessely: args become a vector of free symbols"),
        ("special.batch", 288, "no bessely: args become a vector of free symbols"),
        ("special.batch", 290, "no bessely: args become a vector of free symbols"),
        ("special.batch", 292, "no bessely: args become a vector of free symbols"),
        // airy: unregistered; qalc-num Number::airy is a `false` stub (special.rs:1250)
        ("special.batch", 296, "no airy: name parses as a free symbol, and the product collapses to a plausible-looking number"),
        ("special.batch", 298, "no airy: name parses as a free symbol"),
        ("special.batch", 300, "no airy: name parses as a free symbol"),
        ("special.batch", 302, "no airy: name parses as a free symbol"),
        ("special.batch", 304, "no airy: name parses as a free symbol"),
        ("special.batch", 306, "no airy: name parses as a free symbol"),
        ("special.batch", 308, "no airy: name parses as a free symbol"),
        ("special.batch", 310, "no airy: name parses as a free symbol"),
        ("special.batch", 312, "no airy: name parses as a free symbol"),
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
        // Li: unregistered; qalc-num Number::polylog is a `false` stub (special.rs:1255)
        ("special.batch", 370, "no Li/polylog: parses as L*i"),
        ("special.batch", 372, "no Li/polylog: parses as L*i"),
        ("special.batch", 374, "no Li/polylog: parses as L*i"),
        ("special.batch", 376, "no Li/polylog: parses as L*i"),
        ("special.batch", 378, "no Li/polylog: parses as L*i"),
        ("special.batch", 380, "no Li/polylog: parses as L*i"),
        ("special.batch", 382, "no Li/polylog: parses as L*i"),
        ("special.batch", 492, "no Li/polylog: parses as L*i"),
        // sinc: cardinal sine is absent; parses as s*i*n*c
        ("special.batch", 408, "no sinc: parses as s*i*n*c"),
        ("special.batch", 410, "no sinc: parses as s*i*n*c"),
        ("special.batch", 412, "no sinc: parses as s*i*n*c"),
        ("special.batch", 414, "no sinc: parses as s*i*n*c"),
        ("special.batch", 416, "no sinc: parses as s*i*n*c"),
        ("special.batch", 418, "no sinc: parses as s*i*n*c"),
        // arg: `arg` is aliased onto ATAN2 (builtins.rs:554) but ATAN2 has no 1-argument
        //   form, so the complex argument is never taken and the result even prints
        //   under the wrong name
        ("special.batch", 422, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 424, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 426, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 428, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 430, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 432, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 434, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 436, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 438, "arg aliased to atan2, which has no unary form"),
        ("special.batch", 440, "arg aliased to atan2, which has no unary form"),
        // cis: cis(x) = cos x + i sin x is absent; parses as c*i*s
        ("special.batch", 442, "no cis: parses as c*i*s"),
        ("special.batch", 444, "no cis: parses as c*i*s"),
        ("special.batch", 446, "no cis: parses as c*i*s"),
        ("special.batch", 448, "no cis: parses as c*i*s"),
        ("special.batch", 450, "no cis: parses as c*i*s"),
        ("special.batch", 452, "no cis: parses as c*i*s"),
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
