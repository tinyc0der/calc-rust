//! Pure-Rust port of the libqalculate library.
//!
//! Modules mirror the C++ source layout: parser (`Calculator-parse.cc`),
//! `MathStructure`, `Calculator`, units, variables, builtin functions,
//! definitions loading, and output formatting.

pub mod builtins;
pub mod calculate;
pub mod defs;
pub mod eval;
pub mod ids;
pub mod lexer;
pub mod names;
pub mod options;
pub mod parser;
pub mod percent;
pub mod print;
pub mod session;
pub mod sort;
pub mod structure;

pub use calculate::MergeResult;
pub use ids::{FunctionId, UnitId, VariableId};
pub use options::{ApproximationMode, EvaluationOptions, StructuringMode};
pub use qalc_num::Number;
pub use eval::{evaluate, evaluate_to_string, parse_expression};
pub use session::Session;
pub use structure::{ComparisonType, MathStructure};
