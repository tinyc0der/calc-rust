//! Arbitrary-precision numeric core for rust-calc.
//!
//! This crate replaces libqalculate's use of GMP (integers/rationals) and
//! MPFR (floats), plus the `Number` class itself (`Number.cc`), with pure
//! Rust. No C FFI anywhere in the dependency tree.

pub mod context;
pub mod float;
pub mod number;
pub mod options;

pub use number::{ieee, Number, RealValue};
pub use options::{ComparisonResult, ParseOptions, PrintOptions};

