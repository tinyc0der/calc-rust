//! The `qalc` CLI's reusable parts.
//!
//! The transcript harness and the CLI's evaluation path live here rather than
//! in the binary, so the parity test in `tests/transcripts.rs` drives exactly
//! what `--test-file` drives instead of an approximation of it.

pub mod batch;
pub mod cli;
