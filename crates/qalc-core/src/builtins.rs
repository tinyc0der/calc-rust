//! Builtin function evaluation — port of the `calculate` bodies in
//! `BuiltinFunctions-*.cc`.
//!
//! Functions are dispatched on the stable `FUNCTION_ID_*` values from
//! `BuiltinFunctions.h`, so the mapping survives the registry port. A
//! function that cannot be evaluated (wrong arity, non-numeric argument,
//! domain error) leaves the call unevaluated, matching the C++ convention
//! of returning 0 from `calculate`.

use crate::ids::FunctionId;
use crate::structure::MathStructure;
use qalc_num::options::RoundingMode;
use qalc_num::Number;

/// Stable function ids. Values match `FUNCTION_ID_*` in BuiltinFunctions.h
/// where known; the arithmetic-operator helpers the parser emits reuse the
/// number-category range.
pub mod id {
    pub const SQRT: u32 = 1201;
    pub const CBRT: u32 = 1202;
    pub const ROOT: u32 = 1203;
    pub const EXP: u32 = 1204;
    pub const LN: u32 = 1205;
    pub const LOG: u32 = 1206;
    pub const LOG2: u32 = 1207;
    pub const LOG10: u32 = 1208;
    /// `sq(x) = x²`.
    pub const SQ: u32 = 1209;
    /// `cis(x) = e^(ix) = cos x + i·sin x`.
    pub const CIS: u32 = 1210;

    pub const SIN: u32 = 1000;
    pub const COS: u32 = 1001;
    pub const TAN: u32 = 1002;
    pub const ASIN: u32 = 1003;
    pub const ACOS: u32 = 1004;
    pub const ATAN: u32 = 1005;
    pub const SINH: u32 = 1006;
    pub const COSH: u32 = 1007;
    pub const TANH: u32 = 1008;
    pub const ASINH: u32 = 1009;
    pub const ACOSH: u32 = 1010;
    pub const ATANH: u32 = 1011;
    pub const ATAN2: u32 = 1012;
    /// `cot`/`acot` are XML-defined in the reference
    /// (`cos(x)/sin(x)` and `atan(1/x)`); the port gives them ids in the
    /// trigonometric block so the parser can resolve the names.
    pub const COT: u32 = 1013;
    pub const ACOT: u32 = 1014;
    /// `sinc(x) = sin(x)/x`, with `sinc(0) = 1`
    /// (BuiltinFunctions-trigonometry.cc:1655).
    pub const SINC: u32 = 1015;

    pub const ABS: u32 = 1400;
    pub const SIGNUM: u32 = 1401;
    pub const GAMMA: u32 = 1402;
    pub const ERF: u32 = 1403;
    pub const ERFC: u32 = 1404;
    pub const ZETA: u32 = 1405;
    pub const DIGAMMA: u32 = 1406;
    pub const ERFI: u32 = 1407;
    pub const BERNOULLI: u32 = 1408;
    pub const EXPINT: u32 = 1409;
    pub const LOGINT: u32 = 1410;
    pub const SININT: u32 = 1411;
    pub const COSINT: u32 = 1412;
    /// `arg(z)` — the complex argument. Distinct from [`ATAN2`]: the C++ has
    /// a separate `ArgFunction` (FUNCTION_ID_ARG) with one argument, and
    /// aliasing the two makes `arg(1+i)` print as the unparseable
    /// `atan2(1 + i)`.
    pub const ARG: u32 = 1413;

    pub const FACTORIAL: u32 = 1500;
    pub const DOUBLE_FACTORIAL: u32 = 1501;
    pub const BINOMIAL: u32 = 1502;
    pub const MULTI_FACTORIAL: u32 = 1503;

    pub const MOD: u32 = 1700;
    pub const REM: u32 = 1701;
    pub const IDIV: u32 = 1702;
    pub const SHIFT_LEFT: u32 = 1703;
    pub const SHIFT_RIGHT: u32 = 1704;
    pub const UNCERTAINTY: u32 = 1705;
    pub const GCD: u32 = 1706;
    pub const LCM: u32 = 1707;
    pub const FLOOR: u32 = 1708;
    pub const CEIL: u32 = 1709;
    pub const TRUNC: u32 = 1710;
    pub const ROUND: u32 = 1711;
    pub const FRAC: u32 = 1712;
    pub const INT: u32 = 1713;
    pub const BITWISE_NOT: u32 = 1714;
    pub const PERCENT: u32 = 1720;
    /// IEEE-754 helpers. The 3100 block: every block below it is taken
    /// (1000-2900), and an overlap silently dispatches to another module —
    /// these first sat at 2700, where lambertW already lived, so `float(x)`
    /// computed a Lambert W value instead.
    pub const IEEE_FLOAT: u32 = 3100;
    pub const IEEE_FLOAT_ERROR: u32 = 3101;
    /// Read a literal written in a given base: `hex(34)` is 52.
    pub const BASE_HEX: u32 = 1721;
    pub const BASE_BIN: u32 = 1722;
    pub const BASE_OCT: u32 = 1723;
    pub const BASE_DEC: u32 = 1724;
    pub const BASE_N: u32 = 1725;
    /// Number theory (BuiltinFunctions-number.cc). All scalar-valued.
    pub const ISPRIME: u32 = 1726;
    pub const NEXTPRIME: u32 = 1727;
    pub const PREVPRIME: u32 = 1728;
    pub const NTHPRIME: u32 = 1729;
    pub const PRIME_PI: u32 = 1730;
    pub const POWMOD: u32 = 1731;
    pub const POPCOUNT: u32 = 1732;
    /// Vector-valued number theory, deliberately outside the scalar block
    /// above so `returns_scalar` keeps saying "unknown" for them — otherwise
    /// `0 * divisors(12)` would collapse to zero.
    pub const DIVISORS: u32 = 1740;
    pub const PRIMES: u32 = 1741;
}

/// Does this builtin always return a scalar?
///
/// `MathFunction::representsNonMatrix()` — the C++ asks the function
/// definition; here the answer is "yes" for the plain numeric blocks and
/// "unknown" for everything else, which is what lets `0 * sin(x)` collapse to
/// zero while `0 * solve(...)` does not.
pub fn returns_scalar(id: u32) -> bool {
    (1000..=1015).contains(&id)      // trigonometric
        || (1201..=1210).contains(&id) // roots, exp, logarithms, sq, cis
        || (1400..=1413).contains(&id) // abs, sgn, gamma, erf, zeta, arg, ...
        || (1500..=1503).contains(&id) // factorials and binomial
        || (1700..=1732).contains(&id) // integer, bitwise and prime helpers
        || (3100..=3101).contains(&id) // IEEE-754 helpers
}

/// Evaluate a function call in place. Returns true if it was replaced by a
/// value.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    calculate_function_exact(m, false)
}

/// [`calculate_function`], refusing an approximate numeric result when
/// `exact` is set (`/set approximation exact`).
pub fn calculate_function_exact(m: &mut MathStructure, exact: bool) -> bool {
    let mut eo = crate::options::EvaluationOptions::default();
    if exact {
        eo.approximation = crate::options::ApproximationMode::Exact;
    }
    calculate_function_eo(m, &eo)
}

pub fn calculate_function_eo(m: &mut MathStructure, eo: &crate::options::EvaluationOptions) -> bool {
    let exact = eo.approximation == crate::options::ApproximationMode::Exact;
    // Matrix/vector builtins take structured (non-numeric) arguments, so
    // they are dispatched before the numeric fast path below.
    if crate::matrix::calculate_function(m) {
        return true;
    }
    // Polynomial and solver builtins take structured (non-numeric)
    // arguments too, so they are dispatched before the numeric fast path.
    if crate::polynomial::calculate_function(m) {
        return true;
    }
    // `abs` with a symbolic argument has rewriting rules of its own; the
    // numeric case still falls through to `apply` below.
    if crate::absolute::calculate_function(m) {
        return true;
    }
    // Calculus builtins (`diff`, `limit`) take structured arguments and
    // re-enter the evaluator on their own, so they go before the numeric
    // fast path too.
    if crate::differentiate::calculate_function(m) {
        return true;
    }
    if crate::limit::calculate_function(m) {
        return true;
    }
    if crate::integrate::calculate_function(m) {
        return true;
    }
    if crate::solve::calculate_function(m) {
        return true;
    }
    // `lambertw`/`powertower`/`allroots` have their own id block; `allroots`
    // returns a vector, so it must run before `handle_vector`.
    if crate::explog::calculate_function(m, exact) {
        return true;
    }
    // Geometry is numeric-only but has its own id block and formulas.
    if crate::geometry::calculate_function(m) {
        return true;
    }
    // Text builtins take `MathStructure::Text` arguments (and give the
    // base-reading functions their text form), so they also go first.
    if crate::datetime::calculate_function(m) {
        return true;
    }
    if crate::strings::calculate_function(m) {
        return true;
    }
    if crate::stats::calculate_function(m) {
        return true;
    }
    // `Argument::handlesVector()` (MathStructure-calculate.cc:7178-7188, in
    // `calculateFunctions`): a function whose argument is a scalar maps over a
    // vector element by element — `abs([1 -2])` is `[1 2]`, which
    // `geomean(abs(v))` depends on. (There is no `Argument::handleVector`
    // symbol in the C++; the flag is read, never dispatched through.)
    // Vector-valued number theory (`divisors`, `primes`) must precede
    // `handle_vector`, which would otherwise read their scalar argument as
    // something to map over.
    if calculate_structured(m) {
        return true;
    }
    if handle_vector(m) {
        return true;
    }
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    if args.len() == 1 {
        if let Some(r) = crate::limit::eval_trig_exact(fid, &args[0]) {
            *m = r;
            return true;
        }
    }

    if fid == crate::builtins::id::SQRT && args.len() == 1 {
        if let MathStructure::Number(ref n) = args[0] {
            if eo.split_squares || eo.approximation < crate::options::ApproximationMode::Approximate {
                if let Some((k, rem)) = n.extract_square_factor() {
                    let is_neg = rem.is_negative();
                    let mut rem_abs = rem.clone();
                    if is_neg {
                        rem_abs.negate();
                    }
                    let coeff = if is_neg {
                        let mut c = Number::from_i64(0);
                        c.set_imaginary_part(&k);
                        c
                    } else {
                        k.clone()
                    };

                    if !k.is_one() || is_neg {
                        if rem_abs.is_one() {
                            *m = MathStructure::Number(coeff);
                            return true;
                        } else {
                            *m = MathStructure::Multiplication(vec![
                                MathStructure::Number(coeff),
                                MathStructure::Function {
                                    id: crate::ids::FunctionId(crate::builtins::id::SQRT),
                                    args: vec![MathStructure::Number(rem_abs)],
                                },
                            ]);
                            return true;
                        }
                    }
                }
            }
        }
    }



    // Every builtin here is numeric: bail out unless all arguments reduced
    // to numbers.
    let mut nums: Vec<Number> = Vec::with_capacity(args.len());
    for a in args.iter() {
        match a {
            MathStructure::Number(n) => nums.push(n.clone()),
            _ => return false,
        }
    }
    let Some(result) = apply(fid, &nums) else {
        return false;
    };
    // `numeric_result_ok` in the merge engine: an exact-mode calculation
    // must not silently become approximate.
    if exact && result.is_approximate() && !nums.iter().any(Number::is_approximate) {
        return false;
    }
    *m = MathStructure::Number(result);
    true
}

/// Map a one-argument numeric builtin over a vector argument.
///
/// The C++ `Argument` carries a `b_handle_vector` flag; when a scalar
/// argument receives a vector, `MathStructure::calculateFunctions` applies the
/// function to each element (MathStructure-calculate.cc:7178-7188).
fn handle_vector(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    if args.len() == 1 {
        let MathStructure::Vector(items) = &args[0] else {
            return false;
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let MathStructure::Number(n) = item else {
                return false;
            };
            match apply(fid, std::slice::from_ref(n)) {
                Some(r) => out.push(MathStructure::Number(r)),
                None => return false,
            }
        }
        *m = MathStructure::Vector(out);
        return true;
    } else if args.len() > 1 {
        let mut nums = Vec::with_capacity(args.len());
        for a in args.iter() {
            match a {
                MathStructure::Number(n) => nums.push(n.clone()),
                _ => return false,
            }
        }
        // If applying fid to all arguments together succeeds (e.g. log(100, 10), atan2(y, x), gcd(a, b)),
        // then this is a valid multi-argument call, not vector distribution.
        if apply(fid, &nums).is_some() {
            return false;
        }
        let mut out = Vec::with_capacity(nums.len());
        for n in &nums {
            match apply(fid, std::slice::from_ref(n)) {
                Some(r) => out.push(MathStructure::Number(r)),
                None => return false,
            }
        }
        *m = MathStructure::Vector(out);
        return true;
    }
    false
}

/// Apply the builtin with id `id` to numeric arguments.
fn apply(id: u32, args: &[Number]) -> Option<Number> {
    match (id, args.len()) {
        // --- unary numeric ---
        (id::ABS, 1) => unary(args, |n| n.abs()),
        (id::SIGNUM, 1) => unary(args, |n| n.signum()),
        (id::SQRT, 1) => unary(args, |n| n.sqrt()),
        (id::CBRT, 1) => unary(args, |n| n.cbrt()),
        (id::EXP, 1) => unary(args, |n| n.exp()),
        (id::LN, 1) => unary(args, |n| n.ln()),
        (id::SIN, 1) => unary(args, |n| n.sin()),
        (id::COS, 1) => unary(args, |n| n.cos()),
        (id::TAN, 1) => unary(args, |n| n.tan()),
        (id::ASIN, 1) => unary(args, |n| n.asin()),
        (id::ACOS, 1) => unary(args, |n| n.acos()),
        (id::ATAN, 1) => unary(args, |n| n.atan()),
        // cot(x) = cos(x)/sin(x), acot(x) = atan(1/x) (data/functions.xml.in).
        (id::COT, 1) => {
            let mut c = args[0].clone();
            let mut s = args[0].clone();
            if !c.cos() || !s.sin() || s.is_zero() || !c.divide(&s) {
                return None;
            }
            Some(c)
        }
        (id::ACOT, 1) => {
            if args[0].is_zero() {
                return None;
            }
            let mut v = Number::from_i64(1);
            if !v.divide(&args[0]) || !v.atan() {
                return None;
            }
            Some(v)
        }
        // `sinc(x) = sin(x)/x`, continuous at 0 (the C++ special-cases the
        // zero before building the quotient).
        (id::SINC, 1) => {
            if args[0].is_zero() {
                return Some(Number::from_i64(1));
            }
            let mut s = args[0].clone();
            (s.sin() && s.divide(&args[0])).then_some(s)
        }
        (id::SQ, 1) => unary(args, |n| n.square()),
        // `cis(x) = e^(ix)`. The C++ builds the power and lets the evaluator
        // reduce it; here the numeric identity is direct.
        (id::CIS, 1) => {
            if args[0].has_imaginary_part() {
                return None;
            }
            let mut re = args[0].clone();
            let mut im = args[0].clone();
            if !re.cos() || !im.sin() {
                return None;
            }
            if !im.is_zero() {
                re.set_imaginary_part(&im);
            }
            Some(re)
        }
        (id::SINH, 1) => unary(args, |n| n.sinh()),
        (id::COSH, 1) => unary(args, |n| n.cosh()),
        (id::TANH, 1) => unary(args, |n| n.tanh()),
        (id::ASINH, 1) => unary(args, |n| n.asinh()),
        (id::ACOSH, 1) => unary(args, |n| n.acosh()),
        (id::ATANH, 1) => unary(args, |n| n.atanh()),
        (id::FACTORIAL, 1) => unary(args, |n| n.factorial()),
        (id::DOUBLE_FACTORIAL, 1) => unary(args, |n| n.double_factorial()),
        (id::FLOOR, 1) => unary(args, |n| n.floor()),
        (id::CEIL, 1) => unary(args, |n| n.ceil()),
        (id::TRUNC, 1) | (id::INT, 1) => unary(args, |n| n.trunc()),
        (id::FRAC, 1) => unary(args, |n| n.frac()),
        (id::ROUND, 1) => unary(args, |n| n.round(RoundingMode::HalfAwayFromZero)),
        // `round(x, decimals)` / `round(x, decimals, mode)`
        // (RoundFunction::calculate, BuiltinFunctions-number.cc:1026): scale by
        // 10^decimals, round, scale back. Both extra arguments are
        // `IntegerArgument`s and the third is clamped to the enum's range, so a
        // fractional or out-of-range one leaves the call unevaluated.
        (id::ROUND, 2) | (id::ROUND, 3) => {
            let mode = if args.len() == 3 {
                RoundingMode::from_index(args[2].to_i64()?)?
            } else {
                RoundingMode::HalfAwayFromZero
            };
            let decimals = &args[1];
            if !decimals.is_integer() || args[0].has_imaginary_part() {
                return None;
            }
            let mut n = args[0].clone();
            if !decimals.is_zero() && !n.exp10_mul(decimals) {
                return None;
            }
            if !n.round(mode) {
                return None;
            }
            if !decimals.is_zero() {
                let mut back = decimals.clone();
                if !back.negate() || !n.exp10_mul(&back) {
                    return None;
                }
            }
            Some(n)
        }
        (id::BITWISE_NOT, 1) => unary(args, |n| n.bit_not()),
        // Special functions (hand-rolled in qalc-num; MPFR has no pure-Rust
        // equivalent).
        (id::GAMMA, 1) => unary(args, |n| n.gamma()),
        (id::DIGAMMA, 1) => unary(args, |n| n.digamma()),
        (id::ERF, 1) => unary(args, |n| n.erf()),
        (id::ERFC, 1) => unary(args, |n| n.erfc()),
        (id::ERFI, 1) => unary(args, |n| n.erfi()),
        (id::ZETA, 1) => unary(args, |n| n.zeta()),
        // `zeta(s, a)` — the Hurwitz zeta (ZetaFunction takes 1-2 arguments,
        // BuiltinFunctions-special.cc).
        (id::ZETA, 2) => binary(args, |n, o| n.hurwitz_zeta(o)),
        (id::BERNOULLI, 1) => unary(args, |n| n.bernoulli()),
        (id::EXPINT, 1) => unary(args, |n| n.expint()),
        (id::LOGINT, 1) => unary(args, |n| n.logint()),
        (id::SININT, 1) => unary(args, |n| n.sinint()),
        (id::COSINT, 1) => unary(args, |n| n.cosint()),
        // `arg(0)` is undefined and `Number::arg` says so by returning false,
        // which leaves the call unevaluated — as the reference does.
        (id::ARG, 1) => unary(args, |n| n.arg()),
        (id::PERCENT, 1) => binary_with(args, &Number::from_i64(100), |n, d| n.divide(d)),

        // --- logarithms ---
        (id::LOG2, 1) => binary_with(args, &Number::from_i64(2), |n, b| n.log(b)),
        (id::LOG10, 1) => binary_with(args, &Number::from_i64(10), |n, b| n.log(b)),
        (id::LOG, 1) => unary(args, |n| n.ln()),
        (id::LOG, 2) => binary(args, |n, b| n.log(b)),

        // --- binary numeric ---
        (id::ROOT, 2) => binary(args, |n, o| n.root(o)),
        (id::ATAN2, 2) => binary(args, |n, o| n.atan2(o, false)),
        (id::MOD, 2) => binary(args, |n, o| n.mod_floor(o)),
        (id::REM, 2) => binary(args, |n, o| n.rem(o)),
        (id::IDIV, 2) => binary(args, |n, o| n.iquo(o)),
        (id::SHIFT_LEFT, 2) => binary(args, |n, o| n.shift_left(o)),
        (id::SHIFT_RIGHT, 2) => binary(args, |n, o| n.shift_right(o)),
        // `gcd`/`lcm` take `RATIONAL_POLYNOMIAL_ARGUMENT`s, and
        // `MathStructure::isRationalPolynomial` (MathStructure-polynomial.cc:
        // 1026) rejects a zero number — so the reference *declines* on any
        // zero operand and echoes `lcm(0, 5)` back rather than answering 0.
        // Both are variadic (`gcd(a, b, c)`).
        (id::GCD, _) | (id::LCM, _) if args.len() >= 2 => {
            if args.iter().any(Number::is_zero) {
                return None;
            }
            let mut acc = args[0].clone();
            for o in &args[1..] {
                acc = rational_gcd_lcm(&acc, o, id == id::GCD)?;
            }
            Some(acc)
        }
        // --- number theory ---
        (id::ISPRIME, 1) => {
            let n = nonnegative_integer(&args[0])?;
            Some(Number::from_i64(i64::from(is_prime(n))))
        }
        (id::NEXTPRIME, 1) => {
            // `NumberArgument(ARGUMENT_MIN_MAX_NONNEGATIVE)`, then `ceil`.
            if args[0].is_negative() {
                return None;
            }
            let mut n = args[0].clone();
            if !n.ceil() {
                return None;
            }
            let mut v = n.to_i64()?;
            if v <= 2 {
                return Some(Number::from_i64(2));
            }
            while !is_prime(v) {
                v += 1;
            }
            Some(Number::from_i64(v))
        }
        (id::PREVPRIME, 1) => {
            // `NumberArgument` with min 2, then `floor`.
            let mut n = args[0].clone();
            if !n.floor() {
                return None;
            }
            let mut v = n.to_i64()?;
            if v < 2 {
                return None;
            }
            while !is_prime(v) {
                v -= 1;
            }
            Some(Number::from_i64(v))
        }
        (id::NTHPRIME, 1) => {
            // `IntegerArgument` with min 1. The reference reads a compiled-in
            // table; this port sieves, so it needs a bound — `n(ln n + ln ln
            // n)` is an upper bound on the n-th prime for n >= 6.
            let n = nonnegative_integer(&args[0])?;
            if n < 1 || n > NTH_PRIME_LIMIT {
                return None;
            }
            let f = n as f64;
            let bound = if n < 6 {
                15
            } else {
                (f * (f.ln() + f.ln().ln())).ceil() as i64 + 1
            };
            primes_up_to(bound)?
                .get(n as usize - 1)
                .copied()
                .map(Number::from_i64)
        }
        (id::PRIME_PI, 1) => {
            // `NumberArgument(ARGUMENT_MIN_MAX_NONNEGATIVE)`, then `floor`.
            if args[0].is_negative() {
                return None;
            }
            let mut n = args[0].clone();
            if !n.floor() {
                return None;
            }
            let v = n.to_i64()?;
            Some(Number::from_i64(primes_up_to(v)?.len() as i64))
        }
        (id::POWMOD, 3) => {
            let m = integer(&args[2])?;
            if m == 0 {
                return None;
            }
            let (a, e) = (integer(&args[0])?, integer(&args[1])?);
            pow_mod(a, e, m).map(Number::from_i64)
        }
        // `n(n-x)(n-2x)…`, down to the last positive factor.
        (id::MULTI_FACTORIAL, 2) => {
            let n = nonnegative_integer(&args[0])?;
            let step = nonnegative_integer(&args[1])?;
            if step < 1 || n / step > 100_000 {
                return None;
            }
            let mut acc = Number::from_i64(1);
            let mut k = n;
            while k > 0 {
                if !acc.multiply(&Number::from_i64(k)) {
                    return None;
                }
                k -= step;
            }
            Some(acc)
        }
        (id::POPCOUNT, 1) => {
            // `IntegerArgument(ARGUMENT_MIN_MAX_NONNEGATIVE)`; the count is
            // over the magnitude's limbs, so it must not go through i64 —
            // `popCount(2^64 - 1)` is 64 and does not fit.
            if !args[0].is_integer() || args[0].is_negative() {
                return None;
            }
            let (_, limbs) = args[0].to_bigint()?.to_u32_digits();
            let bits: u32 = limbs.iter().map(|d| d.count_ones()).sum();
            Some(Number::from_i64(i64::from(bits)))
        }
        // `float(0100...)` decodes an IEEE-754 bit string; `floatError(x)`
        // is how far the single-precision encoding of x misses it.
        (id::IEEE_FLOAT, 1) => {
            // The bit string arrives as an integer, so take its exact
            // digits — printing it would give scientific notation and lose
            // them. `from_float` left-pads, restoring the leading zero that
            // the integer conversion dropped.
            let digits = args[0].to_bigint()?.magnitude().to_string();
            qalc_num::ieee::from_float(&digits, 32, 0)
        }
        (id::IEEE_FLOAT_ERROR, 1) => qalc_num::ieee::float_error(&args[0], 32, 0),
        // `hex(34)` re-reads the digits of its argument in another base.
        (id::BASE_HEX, 1) => reinterpret_in_base(&args[0], 16),
        (id::BASE_BIN, 1) => reinterpret_in_base(&args[0], 2),
        (id::BASE_OCT, 1) => reinterpret_in_base(&args[0], 8),
        (id::BASE_DEC, 1) => reinterpret_in_base(&args[0], 10),
        (id::BASE_N, 2) => {
            let b = args[1].to_i64()?;
            if !(2..=36).contains(&b) {
                return None;
            }
            reinterpret_in_base(&args[0], b as u32)
        }
        (id::BINOMIAL, 2) => {
            let mut r = Number::new();
            r.binomial(&args[0], &args[1]).then_some(r)
        }
        _ => None,
    }
}

/// `Number::gcd`/`Number::lcm` over the rationals (Number.cc:`bool
/// Number::gcd`): for non-integers the C++ takes the gcd of the numerators
/// over the lcm of the denominators (and the mirror image for lcm), so
/// `gcd(1/2, 1/3)` is `1/6` rather than a refusal.
fn rational_gcd_lcm(a: &Number, b: &Number, is_gcd: bool) -> Option<Number> {
    if !a.is_rational() || !b.is_rational() {
        return None;
    }
    if a.is_integer() && b.is_integer() {
        let mut r = a.clone();
        let ok = if is_gcd { r.gcd(b) } else { r.lcm(b) };
        return ok.then_some(r);
    }
    let mut num = a.numerator();
    let mut den = a.denominator();
    let (other_num, other_den) = (b.numerator(), b.denominator());
    let ok = if is_gcd {
        num.gcd(&other_num) && den.lcm(&other_den)
    } else {
        num.lcm(&other_num) && den.gcd(&other_den)
    };
    (ok && num.divide(&den)).then_some(num)
}

/// The reference answers `primePi`, `primes` and `nthprime` from tables
/// compiled into the binary; this port sieves, so it needs a ceiling. Past it
/// the call is declined and echoed back — visibly unanswered, rather than
/// hanging.
const PRIME_SIEVE_LIMIT: i64 = 2_000_000;
/// `nthprime(n)` sieves to roughly `n·ln n`, so its own limit is lower.
const NTH_PRIME_LIMIT: i64 = 100_000;
/// `divisors` factorises by trial division up to `sqrt(n)`; this caps that at
/// ten million steps.
const DIVISORS_LIMIT: i64 = 100_000_000_000_000;

/// Sieve of Eratosthenes: every prime `<= n`, or `None` when `n` is out of
/// range for the sieve.
fn primes_up_to(n: i64) -> Option<Vec<i64>> {
    if n > PRIME_SIEVE_LIMIT {
        return None;
    }
    if n < 2 {
        return Some(Vec::new());
    }
    let n = n as usize;
    let mut sieve = vec![true; n + 1];
    sieve[0] = false;
    sieve[1] = false;
    let mut p = 2usize;
    while p * p <= n {
        if sieve[p] {
            let mut k = p * p;
            while k <= n {
                sieve[k] = false;
                k += p;
            }
        }
        p += 1;
    }
    Some(
        sieve
            .iter()
            .enumerate()
            .filter(|(_, &is)| is)
            .map(|(i, _)| i as i64)
            .collect(),
    )
}

/// An argument that must be a plain machine integer, as the C++
/// `IntegerArgument` demands.
fn integer(n: &Number) -> Option<i64> {
    n.is_integer().then(|| n.to_i64()).flatten()
}

/// [`integer`] with `ARGUMENT_MIN_MAX_NONNEGATIVE`.
fn nonnegative_integer(n: &Number) -> Option<i64> {
    let v = integer(n)?;
    (v >= 0).then_some(v)
}

/// `mpz_probab_prime_p` — deterministic Miller-Rabin over the first twelve
/// primes, which is exact for every `i64`.
fn is_prime(n: i64) -> bool {
    if n < 2 {
        return false;
    }
    const WITNESSES: [i64; 12] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37];
    for p in WITNESSES {
        if n % p == 0 {
            return n == p;
        }
    }
    let (mut d, mut r) = (n - 1, 0u32);
    while d % 2 == 0 {
        d /= 2;
        r += 1;
    }
    let m = n as i128;
    'witness: for a in WITNESSES {
        let mut x = mul_pow_mod(a as i128, d as i128, m);
        if x == 1 || x == m - 1 {
            continue;
        }
        for _ in 1..r {
            x = x * x % m;
            if x == m - 1 {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

fn mul_pow_mod(mut base: i128, mut exp: i128, m: i128) -> i128 {
    let mut acc = 1i128;
    base %= m;
    while exp > 0 {
        if exp & 1 == 1 {
            acc = acc * base % m;
        }
        base = base * base % m;
        exp >>= 1;
    }
    acc
}

/// `powmod(a, b, c)` — `mod(a^b, c)`, and the modular inverse of `a^|b|` for
/// a negative exponent (BuiltinFunctions-number.cc:1156).
fn pow_mod(a: i64, e: i64, m: i64) -> Option<i64> {
    let m128 = (m as i128).abs();
    if m128 == 1 {
        return Some(0);
    }
    let base = ((a as i128) % m128 + m128) % m128;
    if e >= 0 {
        return Some(mul_pow_mod(base, e as i128, m128) as i64);
    }
    // Negative exponent: invert first, which needs gcd(a, m) = 1.
    let inv = mod_inverse(base, m128)?;
    Some(mul_pow_mod(inv, (-(e as i128)) as i128, m128) as i64)
}

fn mod_inverse(a: i128, m: i128) -> Option<i128> {
    let (mut old_r, mut r) = (a, m);
    let (mut old_s, mut s) = (1i128, 0i128);
    while r != 0 {
        let q = old_r / r;
        (old_r, r) = (r, old_r - q * r);
        (old_s, s) = (s, old_s - q * s);
    }
    (old_r == 1).then(|| (old_s % m + m) % m)
}

/// Every positive divisor of `|n|`, ascending.
fn divisors_of(n: i64) -> Option<Vec<i64>> {
    let n = n.checked_abs()?;
    if n == 0 || n > DIVISORS_LIMIT {
        return None;
    }
    let mut small = Vec::new();
    let mut large = Vec::new();
    let mut d = 1i64;
    while d.checked_mul(d).is_some_and(|sq| sq <= n) {
        if n % d == 0 {
            small.push(d);
            if d != n / d {
                large.push(n / d);
            }
        }
        d += 1;
    }
    large.reverse();
    small.extend(large);
    Some(small)
}

/// The builtins whose result is a vector rather than a number.
///
/// They cannot go through [`apply`], which is typed `-> Option<Number>`, and
/// they must run before `handle_vector` so a vector-valued call is not
/// mistaken for an elementwise map.
fn calculate_structured(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let (id, [MathStructure::Number(n)]) = (id.0, args.as_slice()) else {
        return false;
    };
    let replacement = match id {
        id::DIVISORS => {
            let Some(v) = integer(n).and_then(divisors_of) else {
                return false;
            };
            v
        }
        id::PRIMES => {
            // `NumberArgument` with min 1; `floor` first.
            let mut f = n.clone();
            if !f.floor() {
                return false;
            }
            let Some(v) = f.to_i64().filter(|v| *v >= 1) else {
                return false;
            };
            let Some(v) = primes_up_to(v) else {
                return false;
            };
            v
        }
        _ => return false,
    };
    *m = MathStructure::Vector(
        replacement
            .into_iter()
            .map(|d| MathStructure::Number(Number::from_i64(d)))
            .collect(),
    );
    true
}

/// Re-read a number's decimal digits as if written in `base`.
///
/// `hex(34)` is 52: the argument is parsed normally (as decimal 34) and the
/// digit string "34" is then interpreted in base 16.
fn reinterpret_in_base(n: &Number, base: u32) -> Option<Number> {
    let mut po = qalc_num::PrintOptions::default();
    po.base = 10;
    let digits = n.print(&po);
    let mut parse = qalc_num::ParseOptions::default();
    parse.base = base as i32;
    let v = Number::parse(&digits, &parse);
    Some(v)
}

fn unary(args: &[Number], f: impl FnOnce(&mut Number) -> bool) -> Option<Number> {
    let mut n = args[0].clone();
    f(&mut n).then_some(n)
}

fn binary(args: &[Number], f: impl FnOnce(&mut Number, &Number) -> bool) -> Option<Number> {
    let mut n = args[0].clone();
    f(&mut n, &args[1]).then_some(n)
}

fn binary_with(
    args: &[Number],
    other: &Number,
    f: impl FnOnce(&mut Number, &Number) -> bool,
) -> Option<Number> {
    let mut n = args[0].clone();
    f(&mut n, other).then_some(n)
}

/// Recursively evaluate every function call in the tree, bottom-up.
pub fn calculate_functions(m: &mut MathStructure) -> bool {
    calculate_functions_eo(m, &crate::options::EvaluationOptions::default())
}

/// [`calculate_functions`] with evaluation options.
///
/// The only option consulted is `approximation`: under
/// `APPROXIMATION_EXACT` a call whose numeric result turned approximate
/// while its arguments were exact is left unevaluated, so `sqrt(5)` stays
/// symbolic instead of collapsing to 2.236… (the C++ gets the same effect
/// from `Number::raise(…, try_exact)` inside `merge_power`).
pub fn calculate_functions_eo(
    m: &mut MathStructure,
    eo: &crate::options::EvaluationOptions,
) -> bool {
    calculate_functions_inner(m, eo)
}

fn calculate_functions_inner(
    m: &mut MathStructure,
    eo: &crate::options::EvaluationOptions,
) -> bool {
    let mut changed = false;
    match m {
        MathStructure::Vector(v)
        | MathStructure::Addition(v)
        | MathStructure::BitwiseAnd(v)
        | MathStructure::BitwiseOr(v)
        | MathStructure::BitwiseXor(v)
        | MathStructure::LogicalAnd(v)
        | MathStructure::LogicalOr(v)
        | MathStructure::LogicalXor(v) => {
            for child in v.iter_mut() {
                changed |= calculate_functions_inner(child, eo);
            }
        }
        MathStructure::Multiplication(v) => {
            let is_factored_radical = eo.split_squares
                && v.len() == 2
                && v[0].is_number()
                && matches!(&v[1], MathStructure::Function { id, args } if id.0 == crate::builtins::id::SQRT && args.len() == 1 && args[0].number().is_some_and(|n| n.extract_square_factor().is_some_and(|(k, rem)| k.is_one() && !rem.is_one())));
            if !is_factored_radical {
                for child in v.iter_mut() {
                    changed |= calculate_functions_inner(child, eo);
                }
            }
        }
        MathStructure::Power { base, exponent } => {
            changed |= calculate_functions_inner(base, eo);
            changed |= calculate_functions_inner(exponent, eo);
        }
        MathStructure::Comparison { left, right, .. } => {
            changed |= calculate_functions_inner(left, eo);
            changed |= calculate_functions_inner(right, eo);
        }
        MathStructure::BitwiseNot(x) | MathStructure::LogicalNot(x) => {
            changed |= calculate_functions_inner(x, eo);
        }
        MathStructure::Conversion { value, .. } => {
            changed |= calculate_functions_inner(value, eo);
        }
        MathStructure::Function { args, .. } => {
            for a in args.iter_mut() {
                changed |= calculate_functions_inner(a, eo);
            }
        }
        _ => {}
    }
    if matches!(m, MathStructure::Function { .. }) {
        changed |= calculate_function_eo(m, eo);
    }
    // Bitwise/logical operators are dedicated node types rather than calls,
    // but their numeric evaluation belongs here.
    changed |= calculate_unary_node(m);
    changed |= calculate_nary_node(m);
    changed
}

/// Fold an n-ary bitwise/logical node over numeric operands.
fn calculate_nary_node(m: &mut MathStructure) -> bool {
    enum Op {
        And,
        Or,
        Xor,
        LAnd,
        LOr,
        LXor,
    }
    let (op, items) = match m {
        MathStructure::BitwiseAnd(v) => (Op::And, v),
        MathStructure::BitwiseOr(v) => (Op::Or, v),
        MathStructure::BitwiseXor(v) => (Op::Xor, v),
        MathStructure::LogicalAnd(v) => (Op::LAnd, v),
        MathStructure::LogicalOr(v) => (Op::LOr, v),
        MathStructure::LogicalXor(v) => (Op::LXor, v),
        _ => return false,
    };
    if items.len() < 2 {
        return false;
    }
    let mut nums: Vec<Number> = Vec::with_capacity(items.len());
    for i in items.iter() {
        match i {
            MathStructure::Number(n) => nums.push(n.clone()),
            _ => return false,
        }
    }
    let mut acc = nums[0].clone();
    for rhs in &nums[1..] {
        let ok = match op {
            Op::And => acc.bit_and(rhs),
            Op::Or => acc.bit_or(rhs),
            Op::Xor => acc.bit_xor(rhs),
            Op::LAnd => {
                let v = acc.get_boolean() == 1 && rhs.get_boolean() == 1;
                acc.set_true(v);
                true
            }
            Op::LOr => {
                let v = acc.get_boolean() == 1 || rhs.get_boolean() == 1;
                acc.set_true(v);
                true
            }
            Op::LXor => {
                let v = (acc.get_boolean() == 1) != (rhs.get_boolean() == 1);
                acc.set_true(v);
                true
            }
        };
        if !ok {
            return false;
        }
    }
    *m = MathStructure::Number(acc);
    true
}

/// Evaluate `BitwiseNot`/`LogicalNot` over a numeric operand.
fn calculate_unary_node(m: &mut MathStructure) -> bool {
    let replacement = match m {
        MathStructure::BitwiseNot(x) => match x.as_ref() {
            MathStructure::Number(n) => {
                let mut v = n.clone();
                v.bit_not().then_some(v)
            }
            _ => None,
        },
        MathStructure::LogicalNot(x) => match x.as_ref() {
            MathStructure::Number(n) => {
                let mut v = n.clone();
                v.set_logical_not();
                Some(v)
            }
            _ => None,
        },
        _ => None,
    };
    match replacement {
        Some(n) => {
            *m = MathStructure::Number(n);
            true
        }
        None => false,
    }
}

/// Resolve a builtin function name to its id, for the parser's
/// [`crate::parser::NameResolver`].
pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let id = match name {
        "abs" => id::ABS,
        "sgn" | "signum" => id::SIGNUM,
        "sqrt" => id::SQRT,
        "cbrt" => id::CBRT,
        "root" => id::ROOT,
        "exp" => id::EXP,
        "ln" => id::LN,
        "log" => id::LOG,
        "log2" | "lb" => id::LOG2,
        "log10" | "lg" => id::LOG10,
        "sin" => id::SIN,
        "cos" => id::COS,
        "tan" => id::TAN,
        "asin" | "arcsin" => id::ASIN,
        "acos" | "arccos" => id::ACOS,
        "atan" | "arctan" => id::ATAN,
        "sinh" => id::SINH,
        "cosh" => id::COSH,
        "tanh" => id::TANH,
        "asinh" | "arsinh" => id::ASINH,
        "acosh" | "arcosh" => id::ACOSH,
        "atanh" | "artanh" => id::ATANH,
        "atan2" => id::ATAN2,
        // `ArgFunction` is its own definition in the C++, not an alias for
        // atan2 — it takes one argument.
        "arg" => id::ARG,
        "cot" => id::COT,
        "acot" | "arccot" => id::ACOT,
        "sinc" => id::SINC,
        "sq" => id::SQ,
        "cis" => id::CIS,
        "gamma" => id::GAMMA,
        "erf" => id::ERF,
        "erfc" => id::ERFC,
        "zeta" => id::ZETA,
        // `psi` is the reference's second name for digamma
        // (data/functions.xml.in: `r:digamma,psi`). Without it the call form
        // fell through to the pressure unit and `psi(4)` answered
        // `27579.02917 Pa`. The bare name still resolves to the unit — the
        // function table is only consulted for `name(`.
        "digamma" | "psi" => id::DIGAMMA,
        "erfi" => id::ERFI,
        "bernoulli" => id::BERNOULLI,
        "Ei" => id::EXPINT,
        "li" => id::LOGINT,
        "Si" => id::SININT,
        "Ci" => id::COSINT,
        "factorial" => id::FACTORIAL,
        "factorial2" => id::DOUBLE_FACTORIAL,
        "binomial" | "comb" => id::BINOMIAL,
        "multifactorial" => id::MULTI_FACTORIAL,
        "mod" => id::MOD,
        "rem" => id::REM,
        "idiv" => id::IDIV,
        // `rc:gcd,c:GCD,c:gcf,c:GCF,c:hcf,c:HCF` — every spelling but the
        // reference name is case sensitive, so only these exact forms.
        "gcd" => id::GCD,
        "GCD" | "gcf" | "GCF" | "hcf" | "HCF" => id::GCD,
        "lcm" => id::LCM,
        "isprime" => id::ISPRIME,
        "nextprime" => id::NEXTPRIME,
        "prevprime" => id::PREVPRIME,
        "nthprime" => id::NTHPRIME,
        "primePi" | "prime_pi" => id::PRIME_PI,
        "primes" => id::PRIMES,
        "divisors" => id::DIVISORS,
        "powmod" | "power_mod" => id::POWMOD,
        "popCount" => id::POPCOUNT,
        "floor" => id::FLOOR,
        "ceil" | "ceiling" => id::CEIL,
        "trunc" => id::TRUNC,
        "round" => id::ROUND,
        "frac" => id::FRAC,
        "int" => id::INT,
        "float" => id::IEEE_FLOAT,
        "floatError" | "floaterror" => id::IEEE_FLOAT_ERROR,
        "hex" => id::BASE_HEX,
        "bin" => id::BASE_BIN,
        "oct" => id::BASE_OCT,
        "dec" => id::BASE_DEC,
        "base" => id::BASE_N,
        _ => {
            // Matrix names come first: `multiply` is the entrywise (Hadamard)
            // product in data/functions.xml.in, not an alias for `expand`.
            return crate::matrix::function_id_for_name(name)
                .or_else(|| crate::differentiate::function_id_for_name(name))
                .or_else(|| crate::limit::function_id_for_name(name))
                .or_else(|| crate::integrate::function_id_for_name(name))
                .or_else(|| crate::polynomial::function_id_for_name(name))
                .or_else(|| crate::solve::function_id_for_name(name))
                .or_else(|| crate::explog::function_id_for_name(name))
                .or_else(|| crate::geometry::function_id_for_name(name))
                .or_else(|| crate::strings::function_id_for_name(name))
                .or_else(|| crate::stats::function_id_for_name(name))
                .or_else(|| crate::datetime::function_id_for_name(name))
        }
    };
    Some(FunctionId(id))
}

/// Map a parser [`crate::parser::BuiltinOp`] to its function id.
pub fn op_id(op: crate::parser::BuiltinOp) -> FunctionId {
    op.function_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::evaluate_to_string;

    fn ev(s: &str) -> String {
        evaluate_to_string(s).expect("evaluates")
    }

    /// [`ev`] through a real [`crate::session::Session`], which is what binds
    /// `i` to the imaginary unit — the bare `SymbolicResolver` leaves it a
    /// free symbol, so `1+i` never becomes one complex `Number`.
    fn sev(s: &str) -> String {
        crate::session::Session::new()
            .evaluate_line(s)
            .expect("evaluates")
    }

    /// Nesting a symbolic call must cost linear time, not `2^depth`.
    ///
    /// [`calculate_function_exact`] dispatches every `Function` node through
    /// `polynomial::calculate_function`, which used to deep-clone its first
    /// argument and run the whole evaluator over it *before* checking whether
    /// the id was a polynomial builtin at all — so every nesting level paid
    /// for a second full evaluation of everything below it. `sin^22` took 10 s
    /// and `sin^24` 44 s in release, doubling per level; in this debug build
    /// depth 25 would have run for well over an hour. It is now milliseconds,
    /// so a budget this generous still fails loudly if the blowup returns.
    #[test]
    fn nested_calls_do_not_blow_up_exponentially() {
        let expr = "sin(".repeat(25) + "x" + &")".repeat(25);
        let start = std::time::Instant::now();
        let out = ev(&expr);
        let elapsed = start.elapsed();
        assert!(out.starts_with("sin(sin("), "got {out}");
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "25 nested calls took {elapsed:?}"
        );
    }

    #[test]
    fn modulo_and_remainder() {
        // Values from tests/operators.batch, verified against the reference.
        assert_eq!(ev("6%2"), "0");
        assert_eq!(ev("7 rem 2"), "1");
        assert_eq!(ev("-8%3"), "-2");
        assert_eq!(ev("3 %% 2"), "1");
        assert_eq!(ev("3 %% -2"), "-1");
        assert_eq!(ev("3 mod -2"), "-1");
    }

    #[test]
    fn integer_division() {
        assert_eq!(ev("5//2"), "2");
        assert_eq!(ev("5\\2"), "2");
        assert_eq!(ev("5 div 2"), "2");
    }

    #[test]
    fn factorials() {
        assert_eq!(ev("1!"), "1");
        assert_eq!(ev("5!"), "120");
        assert_eq!(ev("0!"), "1");
    }

    #[test]
    fn shifts_from_bitwise_batch() {
        assert_eq!(ev("18 >> 2"), "4");
        assert_eq!(ev("-18 >> 1"), "-9");
        assert_eq!(ev("18 << 1"), "36");
        assert_eq!(ev("-18 << 2"), "-72");
    }

    #[test]
    fn bitwise_not() {
        assert_eq!(ev("~0"), "-1");
        assert_eq!(ev("~-1"), "0");
        assert_eq!(ev("~ -812"), "811");
    }

    #[test]
    fn absolute_value() {
        assert_eq!(ev("|-5|"), "5");
    }

    #[test]
    fn unevaluable_calls_are_left_alone() {
        // A symbolic argument keeps the call intact rather than erroring.
        let s = ev("mod(x, 2)");
        assert!(s.contains("mod"), "got {s}");
    }

    /// `name(args)` has three outcomes, and the middle one is why this test
    /// exists (the rule itself is documented in [`crate::parser`]).
    ///
    /// This is the failure mode the golden suite was written to expose:
    /// `airy` is unimplemented, `airy * 0` is `0`, and `0` is
    /// indistinguishable from the 0.3550280539 the reference computes. But
    /// `airy` is not an *unknown* name — `data/functions.xml` declares it,
    /// this port has simply not implemented it — so the call is kept whole and
    /// unevaluated, and prints back as `airy(0)`. Wrong in a way anyone can
    /// see, rather than wrong in a way nobody can.
    ///
    /// Names nothing declares are left to decompose into products, which is
    /// what the reference does with them: it answers `2z^3` to `zzz(2)`.
    #[test]
    fn unknown_function_calls_are_echoed_not_multiplied() {
        let mut s = crate::session::Session::new();
        // 2. Declared, unimplemented: the call reaches the output intact —
        //    and, above all, is not the `0` the product used to give.
        for expr in ["airy(0)", "besselj(0, 0)", "floatParts(0)", "bitget(12, 3)"] {
            assert_eq!(s.evaluate_line(expr).expect(expr), expr);
        }
        // 1. Implemented: still evaluated.
        assert_eq!(s.evaluate_line("sin(0)").expect("sin(0)"), "0");
        // 3. Declared nowhere: a product, byte for byte what the reference
        //    prints for it.
        assert_eq!(s.evaluate_line("zzz(2)").expect("zzz(2)"), "2z^3");
        // Not a call: still a product of identifiers.
        assert!(s.evaluate_line("3yx^2").is_ok());
        // A single letter is a unit, a prefix or an unknown in the C++ name
        // table, never a function — `f(2)` is `2f`.
        assert_eq!(s.evaluate_line("f(2)").expect("f(2)"), "2f");
        // A name that *is* known keeps multiplying.
        assert_eq!(s.evaluate_line("x(2)").expect("x(2)"), "2x");
    }

    /// `psi` is digamma when called, the pressure unit otherwise.
    #[test]
    fn psi_is_the_digamma_alias() {
        assert_eq!(ev("psi(4)"), "1.256117668");
        let mut s = crate::session::Session::new();
        assert!(
            s.evaluate_line("psi").expect("psi").contains("Pa"),
            "the bare name is still the unit"
        );
    }

    /// `arg` has a one-argument form of its own; aliasing it onto `atan2`
    /// printed the unparseable `atan2(1 + i)`.
    #[test]
    fn complex_argument() {
        assert_eq!(sev("arg(1+i)"), "0.7853981634");
        assert_eq!(sev("arg(-1)"), "3.141592654");
        assert_eq!(sev("arg(i)"), "1.570796327");
        // arg(0) is undefined: the reference echoes the call back.
        assert_eq!(sev("arg(0)"), "arg(0)");
    }

    /// `xor` is both an infix operator and a function in the reference.
    #[test]
    fn xor_has_a_call_form() {
        assert_eq!(ev("xor(12, 10)"), "6");
        assert_eq!(ev("12 xor 10"), "6");
        assert_eq!(ev("lxor(5, 7)"), "0");
        assert_eq!(ev("lxor(1, 0)"), "1");
    }

    /// A zero operand fails `isRationalPolynomial`, so the reference declines
    /// rather than answering 0.
    #[test]
    fn gcd_and_lcm_decline_on_zero() {
        assert_eq!(ev("lcm(0, 5)"), "lcm(0, 5)");
        assert_eq!(ev("gcd(0, 0)"), "gcd(0, 0)");
        assert_eq!(ev("gcd(0, 5)"), "gcd(0, 5)");
        assert_eq!(ev("gcd(4, 6)"), "2");
        // Variadic, and defined over the rationals.
        assert_eq!(ev("lcm(4, 6, 8)"), "24");
        assert_eq!(ev("HCF(1/2, 1/3)"), "0.1666666667");
        assert_eq!(ev("lcm(1/2, 1/3)"), "1");
    }

    #[test]
    fn number_theory_builtins() {
        assert_eq!(ev("isprime(561)"), "0");
        assert_eq!(ev("isprime(2147483647)"), "1");
        assert_eq!(ev("isprime(4294967297)"), "0");
        // IntegerArgument(NONNEGATIVE): a negative or fractional argument
        // leaves the call unevaluated.
        assert_eq!(ev("isprime(-7)"), "isprime(-7)");
        assert_eq!(ev("nextprime(0)"), "2");
        assert_eq!(ev("nextprime(1000000)"), "1000003");
        assert_eq!(ev("prevprime(1000000)"), "999983");
        assert_eq!(ev("nthprime(1000)"), "7919");
        assert_eq!(ev("primePi(1000)"), "168");
        assert_eq!(ev("divisors(36)"), "[1  2  3  4  6  9  12  18  36]");
        assert_eq!(ev("divisors(0)"), "divisors(0)");
        assert_eq!(ev("primes(10)"), "[2  3  5  7]");
        assert_eq!(ev("powmod(1000003, 65537, 1000033)"), "898557");
        // A negative exponent is the modular inverse.
        assert_eq!(ev("powmod(2, -1, 7)"), "4");
        assert_eq!(ev("multifactorial(18, 4)"), "30240");
        // popCount counts limb bits, so it must survive values past i64.
        assert_eq!(ev("popCount(2^64 - 1)"), "64");
        assert_eq!(ev("popCount(-1)"), "popCount(-1)");
    }

    #[test]
    fn sq_sinc_and_cis() {
        assert_eq!(ev("sq(-3)"), "9");
        assert_eq!(sev("sq(1+i)"), "2i");
        assert_eq!(ev("sinc(0)"), "1");
        assert_eq!(ev("sinc(1)"), "0.8414709848");
        assert_eq!(ev("cis(0)"), "1");
        assert_eq!(sev("cis(1)"), "0.5403023059 + 0.8414709848i");
    }
}
