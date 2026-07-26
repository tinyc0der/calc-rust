//! Evaluation options — port of `EvaluationOptions` and the enums it uses
//! from `includes.h` (`ApproximationMode`, `StructuringMode`).
//!
//! Only the fields the calculation core ([`crate::calculate`]) actually
//! consults are ported so far; the unit-conversion, isolation and printing
//! fields arrive with their respective ports. Defaults are taken verbatim
//! from `EvaluationOptions::EvaluationOptions()` in `Calculator.cc:72`.

/// `ApproximationMode` (`includes.h:607`). Declaration order is preserved
/// because the C++ code compares modes with `<` and `>=`
/// (e.g. `eo.approximation >= APPROXIMATION_APPROXIMATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApproximationMode {
    /// `APPROXIMATION_EXACT`: the result must be exact.
    Exact,
    /// `APPROXIMATION_TRY_EXACT`: exact if possible (default).
    TryExact,
    /// `APPROXIMATION_APPROXIMATE`: approximation is allowed everywhere.
    Approximate,
    /// `APPROXIMATION_EXACT_VARIABLES`
    ExactVariables,
}

/// `StructuringMode` (`includes.h:620`). `STRUCTURING_SIMPLIFY` is a
/// `#define` alias for `STRUCTURING_EXPAND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructuringMode {
    /// `STRUCTURING_NONE`
    None,
    /// `STRUCTURING_EXPAND` (= `STRUCTURING_SIMPLIFY`)
    Expand,
    /// `STRUCTURING_FACTORIZE`
    Factorize,
}

/// `EvaluationOptions` (`includes.h:748`).
///
/// TODO(port): the omitted fields are `keep_prefixes`,
/// `sync_nonlinear_unit_relations`, `combine_divisions`, `isolate_x`,
/// `isolate_var`, `auto_post_conversion`, `mixed_units_conversion`,
/// `parse_options`, `do_polynomial_division`, `protected_function`,
/// `complex_number_form`, `local_currency_conversion`,
/// `transform_trigonometric_functions` and `interval_calculation`. They need
/// the unit/function/variable registries or the printing layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationOptions {
    /// How exact the result must be. Default: `TryExact`.
    pub approximation: ApproximationMode,
    /// Whether units are synced/converted to allow evaluation.
    /// Default: true. TODO(port): not consulted yet — no unit registry.
    pub sync_units: bool,
    /// Whether known variables are replaced by their value. Default: true.
    /// TODO(port): not consulted yet — no variable registry.
    pub calculate_variables: bool,
    /// Whether functions are calculated. Default: true.
    /// TODO(port): not consulted yet — no function registry.
    pub calculate_functions: bool,
    /// Whether comparisons are evaluated (`5>2` => `1`). Default: true
    /// (C++ uses an `int`, where 2 means "only if the result is definite").
    /// TODO(port): not consulted yet.
    pub test_comparisons: i32,
    /// Whether factors/bases containing an addition are expanded
    /// (`z(x+y)=zx+zy`). Default: 1 (`true`). Negative values are the C++
    /// "limited expansion" modes.
    pub expand: i32,
    /// Whether non-numerical parts of a fraction are reduced. Default: true.
    pub reduce_divisions: bool,
    /// Whether complex numbers may be produced. Default: true.
    pub allow_complex: bool,
    /// Whether infinities may be produced. Default: true.
    pub allow_infinite: bool,
    /// Whether denominators of unknown value may be assumed non-zero.
    /// Default: true (C++ uses an `int`).
    pub assume_denominators_nonzero: bool,
    /// Whether to warn when a denominator was assumed non-zero.
    /// Default: false. There is no message system yet, so this only
    /// suppresses the simplifications the C++ gates behind the warning.
    pub warn_about_denominators_assumed_nonzero: bool,
    /// Whether `sqrt(8)` is split into `2 sqrt(2)`. Default: true.
    /// TODO(port): the prime tables are not ported yet.
    pub split_squares: bool,
    /// Whether units with zero quantity are preserved. Default: true.
    pub keep_zero_units: bool,
    /// Whether the result is expanded or factorized. Default: `Expand`
    /// (`STRUCTURING_SIMPLIFY`). TODO(port): only stored, `eval()` is not
    /// ported yet.
    pub structuring: StructuringMode,
}

impl Default for EvaluationOptions {
    /// `EvaluationOptions::EvaluationOptions()` (`Calculator.cc:72`).
    fn default() -> Self {
        EvaluationOptions {
            approximation: ApproximationMode::TryExact,
            sync_units: true,
            calculate_variables: true,
            calculate_functions: true,
            test_comparisons: 1,
            expand: 1,
            reduce_divisions: true,
            allow_complex: true,
            allow_infinite: true,
            assume_denominators_nonzero: true,
            warn_about_denominators_assumed_nonzero: false,
            split_squares: true,
            keep_zero_units: true,
            structuring: StructuringMode::Expand,
        }
    }
}

impl EvaluationOptions {
    /// Convenience: the default options with `approximation` set to
    /// `APPROXIMATION_EXACT`.
    pub fn exact() -> Self {
        EvaluationOptions {
            approximation: ApproximationMode::Exact,
            ..Default::default()
        }
    }

    /// Convenience: the default options with `approximation` set to
    /// `APPROXIMATION_APPROXIMATE`.
    pub fn approximate() -> Self {
        EvaluationOptions {
            approximation: ApproximationMode::Approximate,
            ..Default::default()
        }
    }
}
