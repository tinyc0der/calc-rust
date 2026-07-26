//! Pure-Rust port of the libqalculate library.
//!
//! Modules mirror the C++ source layout: parser (`Calculator-parse.cc`),
//! `MathStructure`, `Calculator`, units, variables, builtin functions,
//! definitions loading, and output formatting.

pub mod ids;
pub mod lexer;
pub mod parser;
pub mod print;
pub mod structure;

pub use ids::{FunctionId, UnitId, VariableId};
pub use qalc_num::Number;
pub use structure::{ComparisonType, MathStructure};
