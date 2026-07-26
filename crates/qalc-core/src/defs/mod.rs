//! Definition items (prefixes, units, variables, functions) and the registry
//! that owns them.
//!
//! This is the Rust port of `Prefix.h`, `Unit.h`, `Variable.h` and the
//! metadata half of `Function.h`, together with the loader for the shipped
//! Qalculate XML definition files (`Calculator-definitions.cc`,
//! `loadGlobalDefinitions` / `loadDefinitions`).
//!
//! Two deliberate departures from the C++ original:
//!
//! * Items reference each other by integer id ([`UnitId`], [`PrefixId`], …)
//!   rather than by raw pointer, matching the rest of this port.
//! * Nothing is evaluated at load time. Alias-unit relations, variable values
//!   and user-function formulas are kept as expression *strings* exactly as
//!   the C++ does (`KnownVariable::b_expression`, `AliasUnit::svalue`), so a
//!   registry can be built without a `Calculator`.

pub mod xml;

use std::collections::HashMap;

use crate::ids::{FunctionId, UnitId, VariableId};
use crate::names::NameSet;
use qalc_num::Number;

pub use xml::{
    load_definitions_file, load_definitions_str, load_global_definitions, definitions_dir,
    LoadError, DEFINITIONS_DIR_ENV,
};

/// Identifies a [`Prefix`] in a [`Registry`].
///
/// The other id types live in [`crate::ids`]; prefixes have no presence in
/// `MathStructure` yet, so their id is defined here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PrefixId(pub u32);

// ---------------------------------------------------------------------------
// Prefixes — Prefix.h
// ---------------------------------------------------------------------------

/// `PrefixType` from `Prefix.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixKind {
    /// `DecimalPrefix`: value is `10^exponent`.
    Decimal,
    /// `BinaryPrefix`: value is `2^exponent`.
    Binary,
    /// `NumberPrefix`: a free numeric value with no fixed base.
    Number,
}

/// A quantity multiplier prepended to a unit (`kilo`, `kibi`, …).
#[derive(Debug, Clone)]
pub struct Prefix {
    pub id: PrefixId,
    pub names: NameSet,
    pub kind: PrefixKind,
    /// Exponent over the kind's base. Zero and meaningless for
    /// [`PrefixKind::Number`].
    pub exponent: i64,
    /// The prefix value for a unit exponent of one.
    pub value: Number,
}

impl Prefix {
    pub fn decimal(id: PrefixId, names: NameSet, exponent: i64) -> Prefix {
        Prefix {
            id,
            names,
            kind: PrefixKind::Decimal,
            exponent,
            value: pow_base(10, exponent),
        }
    }

    pub fn binary(id: PrefixId, names: NameSet, exponent: i64) -> Prefix {
        Prefix {
            id,
            names,
            kind: PrefixKind::Binary,
            exponent,
            value: pow_base(2, exponent),
        }
    }

    pub fn number(id: PrefixId, names: NameSet, value: Number) -> Prefix {
        Prefix {
            id,
            names,
            kind: PrefixKind::Number,
            exponent: 0,
            value,
        }
    }

    /// `DecimalPrefix::exponent(int iexp)` / `BinaryPrefix::exponent`: the
    /// prefix exponent scaled by the power the prefixed unit is raised to.
    ///
    /// Returns `None` for a [`PrefixKind::Number`] prefix, which has no
    /// exponent over a fixed base.
    pub fn exponent_for(&self, unit_exponent: i64) -> Option<i64> {
        match self.kind {
            PrefixKind::Decimal | PrefixKind::Binary => Some(self.exponent * unit_exponent),
            PrefixKind::Number => None,
        }
    }

    /// `Prefix::value(int iexp)` — the multiplier contributed by this prefix
    /// when the prefixed unit is raised to `unit_exponent`.
    ///
    /// `base_exponent` is the exponent of the prefix itself over its base;
    /// for decimal/binary prefixes the value is `base^(base_exponent *
    /// unit_exponent)`, for number prefixes it is `value^unit_exponent`.
    pub fn value_for(&self, unit_exponent: i64) -> Number {
        match self.kind {
            PrefixKind::Decimal => pow_base(10, self.exponent * unit_exponent),
            PrefixKind::Binary => pow_base(2, self.exponent * unit_exponent),
            PrefixKind::Number => {
                let mut n = self.value.clone();
                n.raise(&Number::from_i64(unit_exponent), true);
                n
            }
        }
    }

    /// `Prefix::value()` — the value for a unit exponent of one.
    pub fn value(&self) -> &Number {
        &self.value
    }

    /// The prefix's untranslated identity, e.g. `kilo`.
    pub fn reference_name(&self) -> &str {
        self.names.reference_name().unwrap_or("")
    }
}

fn pow_base(base: i64, exponent: i64) -> Number {
    let mut n = Number::from_i64(base);
    n.raise(&Number::from_i64(exponent), true);
    n
}

// ---------------------------------------------------------------------------
// Units — Unit.h
// ---------------------------------------------------------------------------

/// One factor of a `CompositeUnit`: `unit^exponent` optionally prefixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositePart {
    pub unit: UnitId,
    pub prefix: Option<PrefixId>,
    pub exponent: i64,
}

/// `UnitSubtype` from `Unit.h`.
#[derive(Debug, Clone)]
pub enum UnitKind {
    /// `Unit` — a base unit, the reference other units are defined against.
    Base,
    /// `AliasUnit` — defined by an expression relating it to another unit.
    Alias {
        base: UnitId,
        /// The relation **expression string**, never evaluated at load time.
        /// `\x` stands for the value in the alias unit (e.g. `\x + 273.15`
        /// for degrees Celsius); a plain number means a scale factor.
        relation: String,
        /// `<inverse_relation>` (older files: `<reverse_relation>`), used when
        /// the relation is not invertible by the calculator.
        inverse_relation: Option<String>,
        /// The power of the base unit this alias corresponds to.
        exponent: i64,
        /// `AliasUnit::setMixWithBase` — non-zero enables mixed-unit output
        /// (`5 ft 3 in`); higher priority wins.
        mix_priority: i32,
        /// `AliasUnit::setMixWithBaseMinimum`.
        mix_min: i32,
        /// `<relation uncertainty=...>` / `relative_uncertainty=...`.
        uncertainty: Option<String>,
        relative_uncertainty: bool,
    },
    /// `CompositeUnit` — a product of powers of other units, e.g. `km_c`.
    Composite { parts: Vec<CompositePart> },
}

/// `Unit::setUseWithPrefixesByDefault` and the preferred-prefix window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixPreference {
    pub use_by_default: bool,
    pub max: i32,
    pub min: i32,
    pub default: i32,
}

impl Default for PrefixPreference {
    fn default() -> Self {
        PrefixPreference {
            use_by_default: false,
            max: i32::MAX,
            min: i32::MIN,
            default: 0,
        }
    }
}

/// A unit of measurement.
#[derive(Debug, Clone)]
pub struct Unit {
    pub id: UnitId,
    pub names: NameSet,
    pub kind: UnitKind,
    pub category: String,
    pub title: String,
    pub description: String,
    /// `<system>` — "SI", "Imperial/US", "CGS", …
    pub system: String,
    /// `<countries>` — comma separated, used by the currency list.
    pub countries: String,
    pub hidden: bool,
    pub approximate: bool,
    /// `-1` when unset, as in `ExpressionItem::setPrecision`.
    pub precision: i32,
    pub use_with_prefixes: Option<PrefixPreference>,
    /// Set when the definition came from `<builtin_unit name="...">`, naming
    /// the C++ unit object the entry decorates.
    pub builtin: Option<String>,
    /// TODO(port): true for a currency whose rate would come from
    /// `Calculator::loadExchangeRates`, which is not ported. The `relation`
    /// on such a unit is a placeholder, not a real exchange rate.
    pub pending_exchange_rate: bool,
    pub active: bool,
}

impl Unit {
    pub fn reference_name(&self) -> &str {
        self.names.reference_name().unwrap_or("")
    }
    pub fn is_base(&self) -> bool {
        matches!(self.kind, UnitKind::Base)
    }
    pub fn is_alias(&self) -> bool {
        matches!(self.kind, UnitKind::Alias { .. })
    }
    pub fn is_composite(&self) -> bool {
        matches!(self.kind, UnitKind::Composite { .. })
    }
    /// The unit this one is defined against, if any.
    pub fn base_unit(&self) -> Option<UnitId> {
        match &self.kind {
            UnitKind::Alias { base, .. } => Some(*base),
            _ => None,
        }
    }
    /// The relation expression string of an alias unit.
    pub fn relation(&self) -> Option<&str> {
        match &self.kind {
            UnitKind::Alias { relation, .. } => Some(relation.as_str()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Variables — Variable.h
// ---------------------------------------------------------------------------

/// `AssumptionSign` from `Variable.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssumptionSign {
    #[default]
    Unknown,
    Positive,
    NonNegative,
    Negative,
    NonPositive,
    NonZero,
}

/// `AssumptionType` from `Variable.h`. Each variant is a subset of the one
/// above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssumptionType {
    /// Multiplication is not commutative.
    None = 0,
    NonMatrix = 1,
    Number = 2,
    Complex = 3,
    Real = 4,
    Rational = 5,
    Integer = 6,
    Boolean = 7,
}

impl Default for AssumptionType {
    /// `Assumptions::Assumptions()` defaults to `ASSUMPTION_TYPE_NUMBER`.
    fn default() -> Self {
        AssumptionType::Number
    }
}

/// What is assumed about an unknown value.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Assumptions {
    pub sign: AssumptionSign,
    pub atype: AssumptionType,
    /// `Assumptions::setMin` — kept as the raw expression string; unused by
    /// the shipped definitions.
    pub min: Option<String>,
    pub max: Option<String>,
    pub include_equals_min: bool,
    pub include_equals_max: bool,
}

impl Assumptions {
    pub fn new() -> Self {
        Assumptions {
            include_equals_min: true,
            include_equals_max: true,
            ..Default::default()
        }
    }
}

/// The value side of a variable definition.
#[derive(Debug, Clone)]
pub enum VariableValue {
    /// `KnownVariable` holding an unparsed expression (`b_expression` true).
    /// The C++ defers parsing until the variable is first used; so do we.
    Expression {
        expression: String,
        /// `<value unit="...">` — a unit expression applied to the value.
        unit: Option<String>,
        uncertainty: Option<String>,
        relative_uncertainty: bool,
    },
    /// `KnownVariable` holding an already-built `MathStructure`.
    Structure(Box<crate::structure::MathStructure>),
    /// TODO(port): `DynamicVariable` — a variable whose value is computed by
    /// C++ code (`pi`, `today`, `uptime`, …). The XML only decorates it with
    /// names and a title; the value implementation is not ported yet.
    Builtin { builtin_name: String },
    /// `UnknownVariable`.
    Unknown(Assumptions),
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub id: VariableId,
    pub names: NameSet,
    pub value: VariableValue,
    pub category: String,
    pub title: String,
    pub description: String,
    pub hidden: bool,
    pub approximate: bool,
    pub precision: i32,
    pub active: bool,
}

impl Variable {
    pub fn reference_name(&self) -> &str {
        self.names.reference_name().unwrap_or("")
    }
    /// `Variable::isKnown` — false only for `UnknownVariable`.
    pub fn is_known(&self) -> bool {
        !matches!(self.value, VariableValue::Unknown(_))
    }
}

// ---------------------------------------------------------------------------
// Functions — Function.h (metadata only)
// ---------------------------------------------------------------------------

/// `ArgumentType` from `Function.h`, as spelled in the `type` attribute of
/// `<argument>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArgumentType {
    /// Plain `Argument`: anything goes. This is what an unknown or missing
    /// `type` attribute maps to (the data files spell it `type="free"`).
    #[default]
    Free,
    Text,
    Symbol,
    Date,
    Integer,
    Number,
    Vector,
    Matrix,
    Boolean,
    Function,
    Unit,
    Variable,
    Object,
    Angle,
    DataObject,
    DataProperty,
}

impl ArgumentType {
    pub fn from_xml(s: &str) -> ArgumentType {
        match s {
            "text" => ArgumentType::Text,
            "symbol" => ArgumentType::Symbol,
            "date" => ArgumentType::Date,
            "integer" => ArgumentType::Integer,
            "number" => ArgumentType::Number,
            "vector" => ArgumentType::Vector,
            "matrix" => ArgumentType::Matrix,
            "boolean" => ArgumentType::Boolean,
            "function" => ArgumentType::Function,
            "unit" => ArgumentType::Unit,
            "variable" => ArgumentType::Variable,
            "object" => ArgumentType::Object,
            "angle" => ArgumentType::Angle,
            "data-object" => ArgumentType::DataObject,
            "data-property" => ArgumentType::DataProperty,
            // "free" and anything unrecognised.
            _ => ArgumentType::Free,
        }
    }
}

/// One `Argument` definition attached to a function.
#[derive(Debug, Clone)]
pub struct ArgumentDef {
    /// 1-based, from the `index` attribute.
    pub index: usize,
    pub name: String,
    pub atype: ArgumentType,
    /// `NumberArgument::setMin` / `IntegerArgument::setMin` — kept as the raw
    /// string, never evaluated at load time.
    pub min: Option<String>,
    pub max: Option<String>,
    pub include_equals_min: bool,
    pub include_equals_max: bool,
    pub complex_allowed: bool,
    pub matrix_allowed: bool,
    pub zero_forbidden: bool,
    pub tests: bool,
    pub handle_vector: bool,
    pub alerts: bool,
    /// `Argument::setCustomCondition` — an expression over `\x`.
    pub condition: Option<String>,
}

impl ArgumentDef {
    fn new(index: usize, atype: ArgumentType) -> ArgumentDef {
        ArgumentDef {
            index,
            name: String::new(),
            atype,
            min: None,
            max: None,
            include_equals_min: true,
            include_equals_max: true,
            // NumberArgument defaults to real-only.
            complex_allowed: false,
            matrix_allowed: false,
            zero_forbidden: false,
            tests: true,
            handle_vector: false,
            alerts: true,
            condition: None,
        }
    }
}

/// A `<subfunction>` of a `UserFunction`.
#[derive(Debug, Clone)]
pub struct Subfunction {
    pub expression: String,
    /// `precalculate="false"` keeps the subfunction unevaluated.
    pub precalculate: bool,
}

/// Whether the function body lives in C++ or in the XML.
#[derive(Debug, Clone)]
pub enum FunctionKind {
    /// `<builtin_function name="sin">` — bound to a C++ `MathFunction`
    /// subclass by name.
    ///
    /// TODO(port): the implementation, and with it the authoritative argument
    /// counts, comes from the builtin class. Until those are ported,
    /// `min_args`/`max_args` are inferred from the `<argument>` elements the
    /// XML documents, which can undercount optional arguments (`log` is
    /// documented with two arguments but accepts one).
    Builtin { builtin_name: String },
    /// `<function>` with an `<expression>` formula, i.e. `UserFunction`.
    User {
        /// The formula with `\X`-style optional placeholders rewritten to
        /// `\x`, as `UserFunction::setFormula` does.
        expression: String,
        /// The formula exactly as it appears in the file.
        raw_expression: String,
        subfunctions: Vec<Subfunction>,
        /// Defaults for the optional arguments, in order.
        default_values: Vec<String>,
    },
}

/// A function definition: everything except the maths.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub id: FunctionId,
    pub names: NameSet,
    pub kind: FunctionKind,
    /// Number of required arguments (`MathFunction::minargs`).
    pub min_args: i32,
    /// Maximum number of arguments; negative means unlimited
    /// (`MathFunction::maxargs`).
    pub max_args: i32,
    /// Argument definitions keyed by 1-based index. Sparse: the XML only
    /// describes the arguments it wants to name.
    pub arguments: Vec<ArgumentDef>,
    pub category: String,
    pub title: String,
    pub description: String,
    /// `MathFunction::setCondition` — an expression over `\x`, `\y`, … that
    /// all arguments must satisfy.
    pub condition: Option<String>,
    pub example: Option<String>,
    pub hidden: bool,
    pub approximate: bool,
    pub precision: i32,
    pub active: bool,
}

impl FunctionDef {
    pub fn reference_name(&self) -> &str {
        self.names.reference_name().unwrap_or("")
    }
    pub fn is_builtin(&self) -> bool {
        matches!(self.kind, FunctionKind::Builtin { .. })
    }
    pub fn argument(&self, index: usize) -> Option<&ArgumentDef> {
        self.arguments.iter().find(|a| a.index == index)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Name → id lookup honouring the per-name case-sensitivity flag.
///
/// `ExpressionName::case_sensitive` is set per name (single-character names
/// default to sensitive, and the `c` flag forces it either way). A
/// case-sensitive name is only found by an exact match; every other name is
/// additionally reachable through its lowercased form, which is how
/// `Calculator::getActiveUnit` and friends behave.
#[derive(Debug, Clone, Default)]
struct NameIndex<T> {
    exact: HashMap<String, T>,
    lowercase: HashMap<String, T>,
}

impl<T: Copy> NameIndex<T> {
    fn insert(&mut self, names: &NameSet, id: T) {
        for n in names.all() {
            self.exact.entry(n.name.clone()).or_insert(id);
            if !n.case_sensitive {
                self.lowercase
                    .entry(n.name.to_lowercase())
                    .or_insert(id);
            }
        }
    }

    fn get(&self, name: &str) -> Option<T> {
        if let Some(id) = self.exact.get(name) {
            return Some(*id);
        }
        self.lowercase.get(&name.to_lowercase()).copied()
    }

    fn get_case_sensitive(&self, name: &str) -> Option<T> {
        self.exact.get(name).copied()
    }
}

/// Everything the calculator knows: prefixes, units, variables and functions,
/// plus the name lookup tables. The Rust stand-in for the definition-owning
/// half of `Calculator`.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    prefixes: Vec<Prefix>,
    units: Vec<Unit>,
    variables: Vec<Variable>,
    functions: Vec<FunctionDef>,

    prefix_names: NameIndex<PrefixId>,
    unit_names: NameIndex<UnitId>,
    variable_names: NameIndex<VariableId>,
    function_names: NameIndex<FunctionId>,

    decimal_prefixes: HashMap<i64, PrefixId>,
    binary_prefixes: HashMap<i64, PrefixId>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    // -- prefixes ---------------------------------------------------------

    pub fn add_prefix(&mut self, mut p: Prefix) -> PrefixId {
        let id = PrefixId(self.prefixes.len() as u32);
        p.id = id;
        match p.kind {
            PrefixKind::Decimal => {
                self.decimal_prefixes.entry(p.exponent).or_insert(id);
            }
            PrefixKind::Binary => {
                self.binary_prefixes.entry(p.exponent).or_insert(id);
            }
            PrefixKind::Number => {}
        }
        self.prefix_names.insert(&p.names, id);
        self.prefixes.push(p);
        id
    }

    pub fn prefixes(&self) -> &[Prefix] {
        &self.prefixes
    }
    pub fn prefix(&self, id: PrefixId) -> &Prefix {
        &self.prefixes[id.0 as usize]
    }
    pub fn find_prefix(&self, name: &str) -> Option<&Prefix> {
        self.prefix_names.get(name).map(|id| self.prefix(id))
    }
    /// `Calculator::getExactDecimalPrefix`.
    pub fn exact_decimal_prefix(&self, exponent: i64) -> Option<PrefixId> {
        self.decimal_prefixes.get(&exponent).copied()
    }
    /// `Calculator::getExactBinaryPrefix`.
    pub fn exact_binary_prefix(&self, exponent: i64) -> Option<PrefixId> {
        self.binary_prefixes.get(&exponent).copied()
    }

    // -- units ------------------------------------------------------------

    pub fn add_unit(&mut self, mut u: Unit) -> UnitId {
        let id = UnitId(self.units.len() as u32);
        u.id = id;
        self.unit_names.insert(&u.names, id);
        self.units.push(u);
        id
    }

    pub fn units(&self) -> &[Unit] {
        &self.units
    }
    pub fn unit(&self, id: UnitId) -> &Unit {
        &self.units[id.0 as usize]
    }
    pub fn unit_mut(&mut self, id: UnitId) -> &mut Unit {
        &mut self.units[id.0 as usize]
    }
    /// `Calculator::getUnit` — resolves any of a unit's names.
    pub fn find_unit(&self, name: &str) -> Option<&Unit> {
        self.unit_names.get(name).map(|id| self.unit(id))
    }
    pub fn find_unit_id(&self, name: &str) -> Option<UnitId> {
        self.unit_names.get(name)
    }
    pub fn find_unit_id_case_sensitive(&self, name: &str) -> Option<UnitId> {
        self.unit_names.get_case_sensitive(name)
    }

    /// Walk an alias chain down to the base (or composite) unit it bottoms out
    /// in. Returns the input for a unit that is already a base or composite.
    pub fn resolve_base_unit(&self, mut id: UnitId) -> UnitId {
        let mut guard = 0;
        while let Some(next) = self.unit(id).base_unit() {
            id = next;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        id
    }

    // -- variables --------------------------------------------------------

    pub fn add_variable(&mut self, mut v: Variable) -> VariableId {
        let id = VariableId(self.variables.len() as u32);
        v.id = id;
        self.variable_names.insert(&v.names, id);
        self.variables.push(v);
        id
    }

    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }
    pub fn variable(&self, id: VariableId) -> &Variable {
        &self.variables[id.0 as usize]
    }
    pub fn find_variable(&self, name: &str) -> Option<&Variable> {
        self.variable_names.get(name).map(|id| self.variable(id))
    }
    pub fn find_variable_id(&self, name: &str) -> Option<VariableId> {
        self.variable_names.get(name)
    }

    // -- functions --------------------------------------------------------

    pub fn add_function(&mut self, mut f: FunctionDef) -> FunctionId {
        let id = FunctionId(self.functions.len() as u32);
        f.id = id;
        self.function_names.insert(&f.names, id);
        self.functions.push(f);
        id
    }

    pub fn functions(&self) -> &[FunctionDef] {
        &self.functions
    }
    pub fn function(&self, id: FunctionId) -> &FunctionDef {
        &self.functions[id.0 as usize]
    }
    pub fn find_function(&self, name: &str) -> Option<&FunctionDef> {
        self.function_names.get(name).map(|id| self.function(id))
    }
    pub fn find_function_id(&self, name: &str) -> Option<FunctionId> {
        self.function_names.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg_with_prefixes() -> Registry {
        let mut r = Registry::new();
        r.add_prefix(Prefix::decimal(
            PrefixId(0),
            NameSet::from_spec("ar:k,r:kilo"),
            3,
        ));
        r.add_prefix(Prefix::binary(
            PrefixId(0),
            NameSet::from_spec("ar:Ki,r:kibi"),
            10,
        ));
        r
    }

    #[test]
    fn decimal_prefix_value() {
        let r = reg_with_prefixes();
        let kilo = r.find_prefix("kilo").unwrap();
        assert_eq!(kilo.kind, PrefixKind::Decimal);
        assert_eq!(kilo.exponent, 3);
        assert!(kilo.value().equals_i64(1000));
        // A squared kilometre carries 10^6.
        assert_eq!(kilo.exponent_for(2), Some(6));
        assert!(kilo.value_for(2).equals_i64(1_000_000));
        // A reciprocal carries 10^-3.
        assert_eq!(kilo.exponent_for(-1), Some(-3));
    }

    #[test]
    fn binary_prefix_value() {
        let r = reg_with_prefixes();
        let kibi = r.find_prefix("kibi").unwrap();
        assert_eq!(kibi.kind, PrefixKind::Binary);
        assert_eq!(kibi.exponent, 10);
        assert!(kibi.value().equals_i64(1024));
        assert!(kibi.value_for(2).equals_i64(1024 * 1024));
    }

    #[test]
    fn number_prefix_raises_its_value() {
        let mut r = Registry::new();
        r.add_prefix(Prefix::number(
            PrefixId(0),
            NameSet::from_spec("r:dozen"),
            Number::from_i64(12),
        ));
        let p = r.find_prefix("dozen").unwrap();
        assert_eq!(p.exponent_for(3), None);
        assert!(p.value_for(2).equals_i64(144));
    }

    #[test]
    fn exact_prefix_lookup_by_exponent() {
        let r = reg_with_prefixes();
        let id = r.exact_decimal_prefix(3).unwrap();
        assert_eq!(r.prefix(id).reference_name(), "kilo");
        assert!(r.exact_decimal_prefix(10).is_none());
        let id = r.exact_binary_prefix(10).unwrap();
        assert_eq!(r.prefix(id).reference_name(), "kibi");
    }

    #[test]
    fn name_lookup_respects_case_sensitivity() {
        let mut r = Registry::new();
        r.add_unit(Unit {
            id: UnitId(0),
            names: NameSet::from_spec("ar:m,meter,p:meters"),
            kind: UnitKind::Base,
            category: String::new(),
            title: String::new(),
            description: String::new(),
            system: String::new(),
            countries: String::new(),
            hidden: false,
            approximate: false,
            precision: -1,
            use_with_prefixes: None,
            builtin: None,
            pending_exchange_rate: false,
            active: true,
        });
        assert!(r.find_unit("m").is_some());
        // Single-character names are case sensitive.
        assert!(r.find_unit("M").is_none());
        assert!(r.find_unit("Meter").is_some());
        assert!(r.find_unit("meters").is_some());
    }
}
