//! Exponential/logarithm builtins that need more than a plain `Number`
//! call — `lambertw`, `powertower` and `allroots`.
//!
//! Ports the corresponding entries of `BuiltinFunctions-explog.cc`
//! (`LambertWFunction`, `PowerTowerFunction`, `AllRootsFunction`).

use crate::ids::FunctionId;
use crate::structure::MathStructure;
use qalc_num::Number;

/// Ids for this module. `lambertw` has `FUNCTION_ID_LAMBERT_W` in the C++,
/// but the port allocates its own private block (2700) that does not
/// overlap the ranges already used by the other modules.
pub mod id {
    pub const LAMBERT_W: u32 = 2700;
    pub const POWER_TOWER: u32 = 2701;
    pub const ALL_ROOTS: u32 = 2702;
}

pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let id = match name {
        "lambertw" => id::LAMBERT_W,
        "powertower" => id::POWER_TOWER,
        "allroots" => id::ALL_ROOTS,
        _ => return None,
    };
    Some(FunctionId(id))
}

pub fn function_name(fid: u32) -> Option<&'static str> {
    Some(match fid {
        id::LAMBERT_W => "lambertw",
        id::POWER_TOWER => "powertower",
        id::ALL_ROOTS => "allroots",
        _ => return None,
    })
}

/// True if `fid` belongs to this module.
///
/// `uncertainty` is deliberately absent from [`function_name`]: the parser
/// already turns `a+/-b` into it and `print.rs` owns its spelling, but the
/// generic numeric dispatcher never evaluates it, so the implementation
/// lives here.
pub fn owns(fid: u32) -> bool {
    fid == crate::builtins::id::UNCERTAINTY || function_name(fid).is_some()
}

/// Evaluate one of this module's builtins in place.
pub fn calculate_function(m: &mut MathStructure, exact: bool) -> bool {
    if calculate_logarithm(m) {
        return true;
    }
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    if !owns(fid) {
        return false;
    }
    // Every function here is numeric.
    let mut nums: Vec<Number> = Vec::with_capacity(args.len());
    for a in args.iter() {
        match a {
            MathStructure::Number(n) => nums.push(n.clone()),
            _ => return false,
        }
    }
    // `numeric_result_ok` in the merge engine: an exact-mode calculation
    // must not silently become approximate, which is what keeps
    // `x = lambertw(e^3)` symbolic under `/set approximation exact`.
    if exact
        && fid != crate::builtins::id::UNCERTAINTY
        && !nums.iter().any(Number::is_approximate)
        && !exactly_representable(fid, &nums)
    {
        return false;
    }
    match apply(fid, &nums) {
        Some(r) => {
            *m = r;
            true
        }
        None => false,
    }
}

/// True when this call has an exact (non-approximate) value, so it may run
/// under `/set approximation exact`.
fn exactly_representable(fid: u32, nums: &[Number]) -> bool {
    match fid {
        // `lambertw(0) = 0`; every other value is transcendental.
        id::LAMBERT_W => nums.first().is_some_and(Number::is_zero),
        // A power tower of rationals stays rational.
        id::POWER_TOWER => nums.iter().all(|n| n.is_rational()),
        _ => false,
    }
}

fn apply(fid: u32, nums: &[Number]) -> Option<MathStructure> {
    if fid == crate::builtins::id::UNCERTAINTY {
        // `uncertainty(value, unc, relative = 0)` (`BuiltinFunctions-util.cc`
        // and the `FUNCTION_ID_UNCERTAINTY` branch of `Function.cc:891`).
        // `a+/-b` always parses with `relative = 0`.
        let mut value = nums.first()?.clone();
        let unc = nums.get(1)?;
        let relative = nums.get(2).is_some_and(|n| !n.is_zero());
        if qalc_num::context::interval_calculation() == qalc_num::context::IntervalCalculation::None
        {
            return Some(MathStructure::Number(value));
        }
        let mut u = unc.clone();
        if relative && !u.multiply(&value.clone()) {
            return None;
        }
        // The variance formula keeps the uncertainty beside the value so the
        // operations that follow can scale it by their own derivative;
        // interval arithmetic widens the value itself instead.
        if qalc_num::context::interval_calculation()
            == qalc_num::context::IntervalCalculation::VarianceFormula
        {
            value.add_variance_uncertainty(&u);
        } else {
            value.set_uncertainty(&u);
        }
        return Some(MathStructure::Number(value));
    }
    match fid {
        id::LAMBERT_W => {
            let mut z = nums.first()?.clone();
            let k = match nums.get(1) {
                Some(n) => n.to_i64()?,
                None => 0,
            };
            if !z.lambert_w(k) {
                return None;
            }
            Some(MathStructure::Number(z))
        }
        id::POWER_TOWER => {
            let r = power_tower(nums.first()?, nums.get(1)?.to_i64()?)?;
            Some(MathStructure::Number(r))
        }
        id::ALL_ROOTS => {
            let roots = all_roots(nums.first()?, nums.get(1)?.to_i64()?)?;
            Some(MathStructure::Vector(
                roots.into_iter().map(MathStructure::Number).collect(),
            ))
        }
        _ => None,
    }
}

/// `powertower(a, n)` = `a^(a^(a^...))` with `n` levels
/// (`PowerTowerFunction::calculate`).
fn power_tower(a: &Number, n: i64) -> Option<Number> {
    if !(1..=64).contains(&n) {
        return None;
    }
    let mut acc = a.clone();
    for _ in 1..n {
        let mut base = a.clone();
        if !base.raise(&acc, true) {
            return None;
        }
        acc = base;
    }
    Some(acc)
}

/// `allroots(x, n)` — every complex `n`-th root of `x`
/// (`Number::allroots`, `Number.cc:4583`).
fn all_roots(x: &Number, n: i64) -> Option<Vec<Number>> {
    // `IntegerArgument(ARGUMENT_MIN_MAX_POSITIVE, INTEGER_TYPE_SIZE)`.
    if n <= 0 || n > 1_000_000 {
        return None;
    }
    if x.is_one() || n == 1 || x.is_zero() {
        return Some(vec![x.clone()]);
    }
    if n == 2 {
        let mut nr = x.clone();
        if !nr.sqrt() {
            return None;
        }
        let mut neg = nr.clone();
        if !neg.negate() {
            return None;
        }
        return Some(vec![nr, neg]);
    }
    if x.is_infinite(false) {
        return None;
    }
    let order = Number::from_i64(n);
    let mut o_inv = order.clone();
    if !o_inv.recip() {
        return None;
    }
    // arg(x), via atan2(Im x, Re x) exactly as the C++ does.
    let mut nr_arg = x.imaginary_part();
    if !nr_arg.atan2(&x.real_part(), true) {
        return None;
    }
    // |x|^(1/n).
    let mut nr_re = x.real_part();
    let mut nr_im = x.imaginary_part();
    if !nr_re.square()
        || !nr_im.square()
        || !nr_re.add(&nr_im)
        || !nr_re.sqrt()
        || !nr_re.raise(&o_inv, true)
    {
        return None;
    }
    let mut nr_pi2 = Number::new();
    nr_pi2.pi();
    if !nr_pi2.multiply(&Number::from_i64(2)) {
        return None;
    }
    let one_i = imaginary_unit();
    let mut roots = Vec::with_capacity(n as usize);
    for k in 0..n {
        let mut nr = nr_pi2.clone();
        if !nr.multiply(&Number::from_i64(k))
            || !nr.add(&nr_arg)
            || !nr.multiply(&one_i)
            || !nr.multiply(&o_inv)
            || !nr.exp()
            || !nr.multiply(&nr_re)
        {
            return None;
        }
        roots.push(nr);
    }
    Some(roots)
}

/// The `i` constant (`nr_one_i`).
fn imaginary_unit() -> Number {
    let mut n = Number::new();
    n.set_imaginary_part(&Number::from_i64(1));
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FunctionId;
    use qalc_num::PrintOptions;

    fn call(fid: u32, args: Vec<MathStructure>) -> Option<String> {
        let mut m = MathStructure::Function { id: FunctionId(fid), args };
        if !calculate_function(&mut m, false) {
            return None;
        }
        let mut po = crate::eval::batch_print_options();
        po.use_unicode_signs = false;
        Some(crate::print::print(&m, &po))
    }

    fn num(i: i64) -> MathStructure {
        MathStructure::Number(Number::from_i64(i))
    }

    #[test]
    fn all_roots_of_four_seventh() {
        // Oracle: `allroots(4, 7)`.
        let want = concat!(
            "[1.219013654  (0.7600425817 + 0.9530632524i)",
            "  (-0.2712560568 + 1.188450437i)",
            "  (-1.098293352 + 0.5289102023i)",
            "  (-1.098293352 - 0.5289102023i)",
            "  (-0.2712560568 - 1.188450437i)",
            "  (0.7600425817 - 0.9530632524i)]",
        );
        assert_eq!(call(id::ALL_ROOTS, vec![num(4), num(7)]).as_deref(), Some(want));
    }

    #[test]
    fn all_roots_square_root_pair() {
        // Oracle: `allroots(16, 2)` = `[4  -4]`.
        assert_eq!(call(id::ALL_ROOTS, vec![num(16), num(2)]).as_deref(), Some("[4  -4]"));
    }

    #[test]
    fn all_roots_of_one_is_the_value_itself() {
        // Oracle: `allroots(1, 5)` = 1 — a single root prints unwrapped.
        assert_eq!(call(id::ALL_ROOTS, vec![num(1), num(5)]).as_deref(), Some("1"));
    }

    #[test]
    fn all_roots_rejects_a_non_positive_order() {
        assert_eq!(call(id::ALL_ROOTS, vec![num(4), num(0)]), None);
        assert_eq!(call(id::ALL_ROOTS, vec![num(4), num(-3)]), None);
    }

    #[test]
    fn power_tower_levels() {
        // Oracle: `powertower(2, 4)` = 65536, `powertower(2, 3)` = 16.
        assert_eq!(call(id::POWER_TOWER, vec![num(2), num(4)]).as_deref(), Some("65536"));
        assert_eq!(call(id::POWER_TOWER, vec![num(2), num(3)]).as_deref(), Some("16"));
        assert_eq!(call(id::POWER_TOWER, vec![num(2), num(1)]).as_deref(), Some("2"));
    }

    #[test]
    fn power_tower_bounds_its_recursion() {
        assert_eq!(call(id::POWER_TOWER, vec![num(2), num(0)]), None);
        assert_eq!(call(id::POWER_TOWER, vec![num(2), num(1000)]), None);
    }

    #[test]
    fn lambert_w_branches() {
        // Oracle: `lambertw(1)` and `lambertw(-0.2, -1)`.
        assert_eq!(call(id::LAMBERT_W, vec![num(1)]).as_deref(), Some("0.5671432904"));
        let arg = MathStructure::Number(Number::parse("-0.2", &Default::default()));
        assert_eq!(
            call(id::LAMBERT_W, vec![arg, num(-1)]).as_deref(),
            Some("-2.542641358")
        );
    }

    #[test]
    fn uncertainty_attaches_the_uncertainty() {
        // `a+/-b` parses to `uncertainty(a, b)`; under the default variance
        // mode the uncertainty rides alongside the value.
        let mut m = MathStructure::Function {
            id: FunctionId(crate::builtins::id::UNCERTAINTY),
            args: vec![num(2), num(3)],
        };
        assert!(calculate_function(&mut m, false));
        let MathStructure::Number(n) = &m else {
            panic!("not a number: {m:?}");
        };
        let mut po = PrintOptions::default();
        po.interval_display = qalc_num::options::IntervalDisplay::PlusMinus;
        assert_eq!(n.print(&po), "2.0±3.0");
    }

    #[test]
    fn uncertainty_is_dropped_when_propagation_is_off() {
        // `/set ic 0` (INTERVAL_CALCULATION_NONE) ignores the uncertainty.
        qalc_num::context::set_interval_calculation(qalc_num::context::IntervalCalculation::None);
        let out = call(crate::builtins::id::UNCERTAINTY, vec![num(2), num(3)]);
        qalc_num::context::set_interval_calculation(
            qalc_num::context::IntervalCalculation::VarianceFormula,
        );
        assert_eq!(out.as_deref(), Some("2"));
    }

    #[test]
    fn module_claims_only_its_own_ids() {
        assert!(owns(id::LAMBERT_W) && owns(id::POWER_TOWER) && owns(id::ALL_ROOTS));
        assert!(owns(crate::builtins::id::UNCERTAINTY));
        // `print.rs` spells `uncertainty` itself; this module must not claim
        // the name, or the printed form would change.
        assert_eq!(function_name(crate::builtins::id::UNCERTAINTY), None);
        assert!(!owns(1));
    }
}

/// `ln(u^n)` is `n * ln(u)` when `u` is positive — the power rule of
/// `LogFunction::calculate`. A sum is factored first, so `ln(x^2 + 2x + 1)`
/// becomes `2 ln(x + 1)` once `/assume positive` makes `x + 1` positive.
fn calculate_logarithm(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    if id.0 != crate::builtins::id::LN || args.len() != 1 {
        return false;
    }
    let eo = crate::options::EvaluationOptions::default();
    let argument = match &args[0] {
        MathStructure::Addition(_) => crate::polynomial::factor(&args[0], &eo),
        other => other.clone(),
    };
    let MathStructure::Power { base, exponent } = &argument else {
        return false;
    };
    // Only an exact power of a positive base: `ln((-2)^2)` is not `2 ln(-2)`.
    if !crate::calculate::represents::positive(base) {
        return false;
    }
    let Some(n) = exponent.number() else {
        return false;
    };
    if !n.is_rational() || n.is_one() {
        return false;
    }
    *m = MathStructure::Multiplication(vec![
        MathStructure::Number(n.clone()),
        MathStructure::Function {
            id: crate::ids::FunctionId(crate::builtins::id::LN),
            args: vec![(**base).clone()],
        },
    ]);
    true
}
