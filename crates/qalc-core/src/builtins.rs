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

    pub const ABS: u32 = 1400;
    pub const SIGNUM: u32 = 1401;
    pub const GAMMA: u32 = 1402;
    pub const ERF: u32 = 1403;
    pub const ERFC: u32 = 1404;
    pub const ZETA: u32 = 1405;

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
}

/// Evaluate a function call in place. Returns true if it was replaced by a
/// value.
pub fn calculate_function(m: &mut MathStructure) -> bool {
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
    *m = MathStructure::Number(result);
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
        (id::BINOMIAL, 2) => {
            let mut r = Number::new();
            r.binomial(&args[0], &args[1]).then_some(r)
        }
        _ => None,
    }
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
                changed |= calculate_functions(child);
            }
        }
        MathStructure::Power { base, exponent } => {
            changed |= calculate_functions(base);
            changed |= calculate_functions(exponent);
        }
        MathStructure::Comparison { left, right, .. } => {
            changed |= calculate_functions(left);
            changed |= calculate_functions(right);
        }
        MathStructure::BitwiseNot(x) | MathStructure::LogicalNot(x) => {
            changed |= calculate_functions(x);
        }
        MathStructure::Conversion { value, .. } => {
            changed |= calculate_functions(value);
        }
        MathStructure::Function { args, .. } => {
            for a in args.iter_mut() {
                changed |= calculate_functions(a);
            }
        }
        _ => {}
    }
    if matches!(m, MathStructure::Function { .. }) {
        changed |= calculate_function(m);
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
        "gamma" => id::GAMMA,
        "erf" => id::ERF,
        "erfc" => id::ERFC,
        "zeta" => id::ZETA,
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
        _ => return None,
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
