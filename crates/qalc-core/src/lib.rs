//! Pure-Rust port of the libqalculate library.
//!
//! Modules mirror the C++ source layout: parser (`Calculator-parse.cc`),
//! `MathStructure`, `Calculator`, units, variables, builtin functions,
//! definitions loading, and output formatting.
//!
//! # The evaluation pipeline
//!
//! One expression goes through, in [`eval::evaluate`] and
//! [`eval::evaluate_calculated_with`]:
//!
//! ```text
//! parser::parse
//!   -> percent::apply
//!   -> loop { builtins::calculate_functions_eo ; MathStructure::calculatesub }
//!   -> datetime::apply
//!   -> solve::isolate_x_toplevel
//!   -> sort::sort
//!   -> print::print
//! ```
//!
//! The order is not arbitrary, and three of the steps are load-bearing where
//! they sit:
//!
//! - **`percent::apply` runs before any merging.** A percent means different
//!   things depending on what it sits next to — `100 + 10%` is `110`, not
//!   `100.1` — so it has to be rewritten into concrete arithmetic while the
//!   sum's term order is still the order the user wrote. Once the merge engine
//!   has reassociated the addition, the information is gone.
//!
//! - **`datetime::apply` runs after the merge loop, and exactly once.** After,
//!   because a duration like `523d` is only a plain count of seconds once the
//!   unit reduction inside the loop has run. Exactly once, because the day
//!   count it produces for a date difference must not be fed back through the
//!   loop, which would reduce it straight back to seconds.
//!
//! - **`sort::sort` runs last**, mirroring the C++ `evalSort` before printing.
//!   The merge engine appends rather than inserts (`x+x` leaves
//!   `Multiplication[x, 2]`), so canonical ordering is a separate final pass
//!   rather than an invariant the engine maintains.
//!
//! The merge loop itself alternates function evaluation and arithmetic merging
//! until neither makes progress, because each can unblock the other: a
//! resolved function call exposes a new merge, and a completed merge finishes a
//! function's arguments. `eval::MAX_EVAL_PASSES` caps it against a rewrite
//! cycle.
//!
//! [`solve::isolate_x_toplevel`] sits between the loop and the sort: it is the
//! C++ `eo.isolate_x`, and it turns an equation in one unknown into a solution
//! rather than a simplified equation.
//!
//! [`eval::evaluate_to_string`] adds the two ends — parsing, and
//! [`eval::apply_conversion`] (an explicit `to <target>`, or the automatic
//! optimal-SI post-conversion when there is none) before printing.

pub mod absolute;
pub mod assumptions;
pub mod builtins;
pub mod calculate;
pub mod datetime;
pub mod defs;
pub mod differentiate;
pub mod eval;
pub mod explog;
pub mod geometry;
pub mod ids;
pub mod integrate;
pub mod lexer;
pub mod limit;
pub mod matrix;
pub mod names;
pub mod options;
pub mod parser;
pub mod percent;
pub mod polynomial;
pub mod print;
pub mod session;
pub mod solve;
pub mod sort;
pub mod stats;
pub mod strings;
pub mod structure;
pub mod units;

pub use calculate::MergeResult;
pub use ids::{FunctionId, UnitId, VariableId};
pub use options::{ApproximationMode, EvaluationOptions, StructuringMode};
pub use qalc_num::Number;
pub use eval::{evaluate, evaluate_to_string, parse_expression};
pub use session::Session;
pub use structure::{ComparisonType, MathStructure};
