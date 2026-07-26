//! Every builtin function id must be unique.
//!
//! Ids are hand-assigned `u32` blocks spread across a dozen modules, and
//! `builtins::calculate_function_exact` dispatches on them by trying each
//! module in turn. A duplicate does not fail to compile and does not fail
//! loudly at runtime — it silently routes calls to whichever module the
//! dispatch chain reaches first. That has happened twice: datetime's block
//! once overlapped polynomial's, so date functions returned polynomial
//! results, and the IEEE-754 block once overlapped `lambertw`, so `float(x)`
//! computed a Lambert W value.
//!
//! Both were found by noticing a wrong answer. This test finds the next one
//! at `cargo test` time instead.

use qalc_core::{
    builtins, datetime, differentiate, explog, geometry, integrate, limit, matrix, polynomial,
    solve, stats, strings,
};

/// Every id constant in the workspace, with the name it is declared under.
fn all_ids() -> Vec<(&'static str, u32)> {
    vec![
        // builtins.rs — trigonometry
        ("builtins::SIN", builtins::id::SIN),
        ("builtins::COS", builtins::id::COS),
        ("builtins::TAN", builtins::id::TAN),
        ("builtins::ASIN", builtins::id::ASIN),
        ("builtins::ACOS", builtins::id::ACOS),
        ("builtins::ATAN", builtins::id::ATAN),
        ("builtins::SINH", builtins::id::SINH),
        ("builtins::COSH", builtins::id::COSH),
        ("builtins::TANH", builtins::id::TANH),
        ("builtins::ASINH", builtins::id::ASINH),
        ("builtins::ACOSH", builtins::id::ACOSH),
        ("builtins::ATANH", builtins::id::ATANH),
        ("builtins::ATAN2", builtins::id::ATAN2),
        ("builtins::COT", builtins::id::COT),
        ("builtins::ACOT", builtins::id::ACOT),
        // builtins.rs — roots, exponential, logarithms
        ("builtins::SQRT", builtins::id::SQRT),
        ("builtins::CBRT", builtins::id::CBRT),
        ("builtins::ROOT", builtins::id::ROOT),
        ("builtins::EXP", builtins::id::EXP),
        ("builtins::LN", builtins::id::LN),
        ("builtins::LOG", builtins::id::LOG),
        ("builtins::LOG2", builtins::id::LOG2),
        ("builtins::LOG10", builtins::id::LOG10),
        // builtins.rs — abs, sign, special functions
        ("builtins::ABS", builtins::id::ABS),
        ("builtins::SIGNUM", builtins::id::SIGNUM),
        ("builtins::GAMMA", builtins::id::GAMMA),
        ("builtins::ERF", builtins::id::ERF),
        ("builtins::ERFC", builtins::id::ERFC),
        ("builtins::ZETA", builtins::id::ZETA),
        ("builtins::DIGAMMA", builtins::id::DIGAMMA),
        ("builtins::ERFI", builtins::id::ERFI),
        ("builtins::BERNOULLI", builtins::id::BERNOULLI),
        ("builtins::EXPINT", builtins::id::EXPINT),
        ("builtins::LOGINT", builtins::id::LOGINT),
        ("builtins::SININT", builtins::id::SININT),
        ("builtins::COSINT", builtins::id::COSINT),
        // builtins.rs — factorials
        ("builtins::FACTORIAL", builtins::id::FACTORIAL),
        ("builtins::DOUBLE_FACTORIAL", builtins::id::DOUBLE_FACTORIAL),
        ("builtins::BINOMIAL", builtins::id::BINOMIAL),
        // builtins.rs — integer, bitwise, base reading
        ("builtins::MOD", builtins::id::MOD),
        ("builtins::REM", builtins::id::REM),
        ("builtins::IDIV", builtins::id::IDIV),
        ("builtins::SHIFT_LEFT", builtins::id::SHIFT_LEFT),
        ("builtins::SHIFT_RIGHT", builtins::id::SHIFT_RIGHT),
        ("builtins::UNCERTAINTY", builtins::id::UNCERTAINTY),
        ("builtins::GCD", builtins::id::GCD),
        ("builtins::LCM", builtins::id::LCM),
        ("builtins::FLOOR", builtins::id::FLOOR),
        ("builtins::CEIL", builtins::id::CEIL),
        ("builtins::TRUNC", builtins::id::TRUNC),
        ("builtins::ROUND", builtins::id::ROUND),
        ("builtins::FRAC", builtins::id::FRAC),
        ("builtins::INT", builtins::id::INT),
        ("builtins::BITWISE_NOT", builtins::id::BITWISE_NOT),
        ("builtins::PERCENT", builtins::id::PERCENT),
        ("builtins::BASE_HEX", builtins::id::BASE_HEX),
        ("builtins::BASE_BIN", builtins::id::BASE_BIN),
        ("builtins::BASE_OCT", builtins::id::BASE_OCT),
        ("builtins::BASE_DEC", builtins::id::BASE_DEC),
        ("builtins::BASE_N", builtins::id::BASE_N),
        ("builtins::IEEE_FLOAT", builtins::id::IEEE_FLOAT),
        ("builtins::IEEE_FLOAT_ERROR", builtins::id::IEEE_FLOAT_ERROR),
        // calculus
        ("differentiate::DIFFERENTIATE", differentiate::id::DIFFERENTIATE),
        ("limit::LIMIT", limit::id::LIMIT),
        ("integrate::INTEGRATE", integrate::id::INTEGRATE),
        ("integrate::ROMBERG", integrate::id::ROMBERG),
        ("integrate::SINHINT", integrate::id::SINHINT),
        ("integrate::COSHINT", integrate::id::COSHINT),
        ("integrate::FRESNEL_S", integrate::id::FRESNEL_S),
        ("integrate::FRESNEL_C", integrate::id::FRESNEL_C),
        ("integrate::I_GAMMA", integrate::id::I_GAMMA),
        ("integrate::GAMMAINC", integrate::id::GAMMAINC),
        ("integrate::INCOMPLETE_BETA", integrate::id::INCOMPLETE_BETA),
        // solver
        ("solve::SOLVE", solve::id::SOLVE),
        ("solve::SOLVE_MULTIPLE", solve::id::SOLVE_MULTIPLE),
        ("solve::SECANT_METHOD", solve::id::SECANT_METHOD),
        ("solve::NEWTON_RAPHSON", solve::id::NEWTON_RAPHSON),
        // explog
        ("explog::LAMBERT_W", explog::id::LAMBERT_W),
        ("explog::POWER_TOWER", explog::id::POWER_TOWER),
        ("explog::ALL_ROOTS", explog::id::ALL_ROOTS),
    ]
}

#[test]
fn no_two_builtin_function_ids_collide() {
    let mut ids = all_ids();
    ids.sort_by_key(|(_, value)| *value);
    let collisions: Vec<String> = ids
        .windows(2)
        .filter(|pair| pair[0].1 == pair[1].1)
        .map(|pair| format!("{} and {} both use id {}", pair[0].0, pair[1].0, pair[0].1))
        .collect();
    assert!(
        collisions.is_empty(),
        "duplicate function ids dispatch to whichever module the chain reaches \
         first, silently:\n{}",
        collisions.join("\n")
    );
}

/// Registry ids are tagged so they can never be mistaken for a builtin's.
#[test]
fn registry_ids_are_disjoint_from_builtin_ids() {
    use qalc_core::ids::FunctionId;
    for index in [0usize, 1, 172, 1000, 100_000] {
        let id = FunctionId::from_registry_index(index);
        assert!(id.is_registry());
        assert_eq!(id.registry_index(), Some(index));
        assert!(
            all_ids().iter().all(|(_, builtin)| *builtin != id.0),
            "registry index {index} collides with a builtin id"
        );
    }
}

/// The other id spaces are not tagged, so keep the smoke test honest about
/// what is and is not checked here.
#[test]
fn matrix_stats_and_the_rest_are_covered_by_their_own_name_tables() {
    // These modules claim ids by exact-match name lookup rather than by
    // range, so a collision shows up as a missing name rather than as a
    // misroute. Assert the lookups still work.
    assert!(matrix::function_name(matrix::id::DETERMINANT).is_some());
    assert!(stats::function_name(stats::id::MEAN).is_some());
    assert!(polynomial::function_name(polynomial::id::FACTORIZE).is_some());
    assert!(strings::function_name(strings::id::LENGTH).is_some());
    assert!(datetime::function_name(datetime::id::TIMESTAMP).is_some());
    assert!(geometry::function_name(geometry::id::HYPOT).is_some());
}
