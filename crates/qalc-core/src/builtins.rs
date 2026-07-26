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

    pub const FACTORIAL: u32 = 1500;
    pub const DOUBLE_FACTORIAL: u32 = 1501;
    pub const BINOMIAL: u32 = 1502;

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
}

/// Does this builtin always return a scalar?
///
/// `MathFunction::representsNonMatrix()` — the C++ asks the function
/// definition; here the answer is "yes" for the plain numeric blocks and
/// "unknown" for everything else, which is what lets `0 * sin(x)` collapse to
/// zero while `0 * solve(...)` does not.
pub fn returns_scalar(id: u32) -> bool {
    (1000..=1014).contains(&id)      // trigonometric
        || (1201..=1208).contains(&id) // roots, exp, logarithms
        || (1400..=1412).contains(&id) // abs, sgn, gamma, erf, zeta, ...
        || (1500..=1502).contains(&id) // factorials and binomial
        || (1700..=1725).contains(&id) // integer and bitwise helpers
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
    // `Argument::handleVector` (Function.cc:1730): a function whose argument
    // is a scalar maps over a vector element by element — `abs([1 -2])` is
    // `[1 2]`, which `geomean(abs(v))` depends on.
    if handle_vector(m) {
        return true;
    }
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let id = id.0;
    // Every builtin here is numeric: bail out unless all arguments reduced
    // to numbers.
    let mut nums: Vec<Number> = Vec::with_capacity(args.len());
    for a in args.iter() {
        match a {
            MathStructure::Number(n) => nums.push(n.clone()),
            _ => return false,
        }
    }
    let Some(result) = apply(id, &nums) else {
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
/// argument receives a vector, `MathFunction::calculate` applies the
/// function to each element (Function.cc:1730).
fn handle_vector(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    let [MathStructure::Vector(items)] = args.as_slice() else {
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
    true
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
        (id::BITWISE_NOT, 1) => unary(args, |n| n.bit_not()),
        // Special functions (hand-rolled in qalc-num; MPFR has no pure-Rust
        // equivalent).
        (id::GAMMA, 1) => unary(args, |n| n.gamma()),
        (id::DIGAMMA, 1) => unary(args, |n| n.digamma()),
        (id::ERF, 1) => unary(args, |n| n.erf()),
        (id::ERFC, 1) => unary(args, |n| n.erfc()),
        (id::ERFI, 1) => unary(args, |n| n.erfi()),
        (id::ZETA, 1) => unary(args, |n| n.zeta()),
        (id::BERNOULLI, 1) => unary(args, |n| n.bernoulli()),
        (id::EXPINT, 1) => unary(args, |n| n.expint()),
        (id::LOGINT, 1) => unary(args, |n| n.logint()),
        (id::SININT, 1) => unary(args, |n| n.sinint()),
        (id::COSINT, 1) => unary(args, |n| n.cosint()),
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
        (id::GCD, 2) => binary(args, |n, o| n.gcd(o)),
        (id::LCM, 2) => binary(args, |n, o| n.lcm(o)),
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
    let exact = eo.approximation == crate::options::ApproximationMode::Exact;
    calculate_functions_inner(m, exact)
}

fn calculate_functions_inner(m: &mut MathStructure, exact: bool) -> bool {
    let mut changed = false;
    match m {
        MathStructure::Vector(v)
        | MathStructure::Addition(v)
        | MathStructure::Multiplication(v)
        | MathStructure::BitwiseAnd(v)
        | MathStructure::BitwiseOr(v)
        | MathStructure::BitwiseXor(v)
        | MathStructure::LogicalAnd(v)
        | MathStructure::LogicalOr(v)
        | MathStructure::LogicalXor(v) => {
            for child in v.iter_mut() {
                changed |= calculate_functions_inner(child, exact);
            }
        }
        MathStructure::Power { base, exponent } => {
            changed |= calculate_functions_inner(base, exact);
            changed |= calculate_functions_inner(exponent, exact);
        }
        MathStructure::Comparison { left, right, .. } => {
            changed |= calculate_functions_inner(left, exact);
            changed |= calculate_functions_inner(right, exact);
        }
        MathStructure::BitwiseNot(x) | MathStructure::LogicalNot(x) => {
            changed |= calculate_functions_inner(x, exact);
        }
        MathStructure::Conversion { value, .. } => {
            changed |= calculate_functions_inner(value, exact);
        }
        MathStructure::Function { args, .. } => {
            for a in args.iter_mut() {
                changed |= calculate_functions_inner(a, exact);
            }
        }
        _ => {}
    }
    if matches!(m, MathStructure::Function { .. }) {
        changed |= calculate_function_exact(m, exact);
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
        "atan2" | "arg" => id::ATAN2,
        "cot" => id::COT,
        "acot" | "arccot" => id::ACOT,
        "gamma" => id::GAMMA,
        "erf" => id::ERF,
        "erfc" => id::ERFC,
        "zeta" => id::ZETA,
        "digamma" => id::DIGAMMA,
        "erfi" => id::ERFI,
        "bernoulli" => id::BERNOULLI,
        "Ei" => id::EXPINT,
        "li" => id::LOGINT,
        "Si" => id::SININT,
        "Ci" => id::COSINT,
        "factorial" => id::FACTORIAL,
        "factorial2" => id::DOUBLE_FACTORIAL,
        "binomial" | "comb" => id::BINOMIAL,
        "mod" => id::MOD,
        "rem" => id::REM,
        "idiv" => id::IDIV,
        "gcd" => id::GCD,
        "lcm" => id::LCM,
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
}
