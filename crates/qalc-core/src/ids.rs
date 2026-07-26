//! Registry identifiers.
//!
//! The C++ `MathStructure` stores raw `MathFunction*` / `Variable*` /
//! `Unit*` pointers into the global `CALCULATOR` registries. To break that
//! pointer cycle, the Rust port stores plain integer IDs; the registries
//! (ported later with `Calculator`) will map IDs to definitions.

/// Identifies a `MathFunction` in the (future) function registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(pub u32);

/// Identifies a `Variable` in the (future) variable registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariableId(pub u32);

/// Identifies a `Unit` in the (future) unit registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(pub u32);
