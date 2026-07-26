//! Registry identifiers.
//!
//! The C++ `MathStructure` stores raw `MathFunction*` / `Variable*` /
//! `Unit*` pointers into the global `CALCULATOR` registries. To break that
//! pointer cycle, the Rust port stores plain integer IDs; the registries
//! (ported later with `Calculator`) will map IDs to definitions.

/// The bit that separates registry-allocated function ids from the builtin
/// ids hand-assigned in [`crate::builtins::id`] and its sibling modules.
///
/// The registry numbers its functions densely from zero, and the builtin
/// blocks start at 1000 — so the 173rd XML function would collide with `sin`
/// the moment name resolution starts returning registry ids. Setting this bit
/// on every registry id makes the two spaces provably disjoint instead of
/// merely far apart. Function ids never leave the process, so the encoding
/// costs nothing.
pub const REGISTRY_ID_BIT: u32 = 0x8000_0000;

/// Identifies a `MathFunction` in the (future) function registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(pub u32);

impl FunctionId {
    /// The id of the `index`-th function in the definition registry.
    pub fn from_registry_index(index: usize) -> FunctionId {
        FunctionId(REGISTRY_ID_BIT | index as u32)
    }

    /// Was this id allocated by the registry rather than hand-assigned?
    pub fn is_registry(self) -> bool {
        self.0 & REGISTRY_ID_BIT != 0
    }

    /// The registry index this id refers to, if it is a registry id.
    pub fn registry_index(self) -> Option<usize> {
        self.is_registry().then(|| (self.0 & !REGISTRY_ID_BIT) as usize)
    }
}

/// Identifies a `Variable` in the (future) variable registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariableId(pub u32);

/// Identifies a `Unit` in the (future) unit registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(pub u32);
