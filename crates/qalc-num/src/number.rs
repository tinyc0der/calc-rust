//! The `Number` type, mirroring libqalculate's `Number` class.
//!
//! Representation (to be filled in as the port proceeds):
//! - exact integers and rationals (bignum),
//! - arbitrary-precision binary floats with interval bounds,
//! - complex numbers via a boxed imaginary part,
//! - signed infinities,
//! - approximation flag and precision tracking.

/// Placeholder skeleton; the full port of `Number.h`/`Number.cc` lands here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Number;
