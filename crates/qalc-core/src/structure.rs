//! `MathStructure` — port of libqalculate's expression-tree node
//! (`MathStructure.h`/`MathStructure.cc`).
//!
//! The C++ class is a mutable refcounted node with a `StructureType` tag and
//! a `v_subs` child vector; the Rust port is a plain owned enum mutated
//! through `&mut`. Only representation, construction, traversal, structural
//! transformation and equality are ported here — evaluation
//! (`MathStructure-calculate.cc` etc.), sorting and real printing come with
//! later ports.
//!
//! C++ conventions mirrored here (verified in `MathStructure.cc`):
//! - Subtraction is an `Addition` with the subtrahend negated
//!   (`subtract()` = copy, `negate()`, `add_nocopy()`).
//! - Division is a `Multiplication` with the divisor raised to -1
//!   (`divide()` = copy, `inverse()`, `multiply_nocopy()`; `inverse()` is
//!   `raise(m_minus_one)`).
//! - The default value is the number zero (`init()` sets
//!   `m_type = STRUCT_NUMBER` with a default `Number`).

use std::fmt;

use crate::defs::PrefixId;
use crate::ids::{FunctionId, UnitId, VariableId};
use qalc_num::{Number, PrintOptions};

/// Comparison signs for comparison structures (`ComparisonType` in
/// `includes.h`; declaration order preserved).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComparisonType {
    Less,
    Greater,
    EqualsLess,
    EqualsGreater,
    Equals,
    NotEquals,
}

/// Placeholder for `QalculateDateTime` until the date/time module is ported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DateTime;

/// A date/time value carried in the expression tree.
pub type DateTimeValue = qalc_datetime::QalculateDateTime;

/// A structure representing a mathematical value/expression/result.
///
/// Variants mirror the C++ `StructureType` tags (`STRUCT_*`). The
/// formatting-only C++ tags `STRUCT_INVERSE`, `STRUCT_DIVISION` and
/// `STRUCT_NEGATE` are intentionally absent: they only exist in formatted
/// C++ structures and will be handled by the (later) format/print port.
#[derive(Debug, Clone)]
pub enum MathStructure {
    /// `STRUCT_NUMBER`
    Number(Number),
    /// `STRUCT_VECTOR` (a matrix is a vector of vectors)
    Vector(Vec<MathStructure>),
    /// `STRUCT_SYMBOLIC`
    Symbolic(String),
    /// A text value — also `STRUCT_SYMBOLIC` in C++, where the distinction
    /// is carried by the *printer*: an unknown name is a `STRUCT_VARIABLE`
    /// and prints bare, while a quoted literal is a `STRUCT_SYMBOLIC` and
    /// prints quoted (MathStructure-print.cc:4184). This port resolves
    /// unknown names to [`MathStructure::Symbolic`], so text needs its own
    /// tag to keep the two printing rules apart.
    Text(String),
    /// `STRUCT_ADDITION`: two or more terms
    Addition(Vec<MathStructure>),
    /// `STRUCT_MULTIPLICATION`: two or more factors
    Multiplication(Vec<MathStructure>),
    /// `STRUCT_POWER`: children 0 and 1 in C++
    Power {
        base: Box<MathStructure>,
        exponent: Box<MathStructure>,
    },
    /// `STRUCT_FUNCTION`: `o_function` is an ID until the registry lands
    Function {
        id: FunctionId,
        args: Vec<MathStructure>,
    },
    /// `STRUCT_VARIABLE`
    Variable(VariableId),
    /// `STRUCT_UNIT`. The optional prefix is the C++ `o_prefix` member,
    /// which only a unit structure may carry (`km` is the meter unit with the
    /// kilo prefix, not a multiplication by 1000).
    Unit {
        id: UnitId,
        prefix: Option<PrefixId>,
    },
    /// `STRUCT_COMPARISON`: children 0 and 1 in C++, plus `ct_comp`
    Comparison {
        left: Box<MathStructure>,
        op: ComparisonType,
        right: Box<MathStructure>,
    },
    /// `STRUCT_BITWISE_AND`
    BitwiseAnd(Vec<MathStructure>),
    /// `STRUCT_BITWISE_OR`
    BitwiseOr(Vec<MathStructure>),
    /// `STRUCT_BITWISE_XOR`
    BitwiseXor(Vec<MathStructure>),
    /// `STRUCT_BITWISE_NOT`: single child in C++
    BitwiseNot(Box<MathStructure>),
    /// `STRUCT_LOGICAL_AND`
    LogicalAnd(Vec<MathStructure>),
    /// `STRUCT_LOGICAL_OR`
    LogicalOr(Vec<MathStructure>),
    /// `STRUCT_LOGICAL_XOR`
    LogicalXor(Vec<MathStructure>),
    /// `STRUCT_LOGICAL_NOT`: single child in C++
    LogicalNot(Box<MathStructure>),
    /// `STRUCT_UNDEFINED`
    Undefined,
    /// `STRUCT_ABORTED`
    Aborted,
    /// `STRUCT_DATETIME`
    DateTime(Box<DateTimeValue>),
    /// An `expr to target` conversion.
    ///
    /// The C++ handles `to` outside the structure tree — the CLI splits it
    /// off with `Calculator::separateToExpression` and applies it to the
    /// print options or via `Calculator::convert`. Representing it as a node
    /// keeps the parser total; evaluation unwraps it.
    Conversion {
        value: Box<MathStructure>,
        target: ConversionTarget,
    },
}

/// What an `expr to target` conversion asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversionTarget {
    /// A number base, with an optional fixed bit width (`bin16`).
    NumberBase { base: i32, bits: u32 },
    /// `to base N`, where N is an expression.
    Base(Box<MathStructure>),
    /// A unit expression.
    Unit {
        expr: Box<MathStructure>,
        /// `to -unit` suppresses mixed-unit output
        /// (`Calculator-convert.cc:2294`).
        mix: bool,
        /// `to ?unit` / `to b?unit` asks for an automatic decimal/binary
        /// output prefix.
        prefix: crate::units::PrefixMode,
    },
    /// `to base` with no argument: expand to base units.
    BaseUnits,
}

/// `MathStructure()` initializes to the number zero (`init()` sets
/// `m_type = STRUCT_NUMBER`, `o_number` default-constructed to 0).
impl Default for MathStructure {
    fn default() -> Self {
        MathStructure::Number(Number::new())
    }
}

impl From<i64> for MathStructure {
    fn from(i: i64) -> Self {
        MathStructure::Number(Number::from_i64(i))
    }
}

impl From<Number> for MathStructure {
    fn from(n: Number) -> Self {
        MathStructure::Number(n)
    }
}

impl From<&str> for MathStructure {
    fn from(sym: &str) -> Self {
        MathStructure::symbolic(sym)
    }
}

/// Structural equality, like C++ `operator ==` (which calls `equals`).
impl PartialEq for MathStructure {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl MathStructure {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// `MathStructure()` — the number zero.
    pub fn new() -> Self {
        MathStructure::default()
    }

    /// `MathStructure(int num, int den, int exp10)`.
    pub fn from_ints(num: i64, den: i64, exp10: i64) -> Self {
        MathStructure::Number(Number::from_ints(num, den, exp10))
    }

    pub fn from_i64(i: i64) -> Self {
        MathStructure::from(i)
    }

    /// `MathStructure(string sym, force_symbol = true)`. The non-forced C++
    /// path (detecting "undefined" and date strings) belongs to parsing and
    /// is not ported here.
    pub fn symbolic(sym: impl Into<String>) -> Self {
        MathStructure::Symbolic(sym.into())
    }

    /// A bare (unprefixed) unit structure.
    pub fn unit(id: UnitId) -> Self {
        MathStructure::Unit { id, prefix: None }
    }

    /// `clear()` — reset the value to zero.
    pub fn clear(&mut self) {
        *self = MathStructure::default();
    }

    /// `clearVector()` — set the structure to an empty vector.
    pub fn clear_vector(&mut self) {
        *self = MathStructure::Vector(Vec::new());
    }

    // ------------------------------------------------------------------
    // Type predicates
    // ------------------------------------------------------------------

    pub fn is_number(&self) -> bool {
        matches!(self, MathStructure::Number(_))
    }
    /// `isZero()`: a number structure holding zero.
    pub fn is_zero(&self) -> bool {
        matches!(self, MathStructure::Number(n) if n.is_zero())
    }
    /// `isOne()`: a number structure holding one.
    pub fn is_one(&self) -> bool {
        matches!(self, MathStructure::Number(n) if n.is_one())
    }
    /// `isMinusOne()`: a number structure holding minus one.
    pub fn is_minus_one(&self) -> bool {
        matches!(self, MathStructure::Number(n) if n.is_minus_one())
    }
    /// `isInteger()`: a number structure holding an integer.
    pub fn is_integer(&self) -> bool {
        matches!(self, MathStructure::Number(n) if n.is_integer())
    }
    pub fn is_vector(&self) -> bool {
        matches!(self, MathStructure::Vector(_))
    }
    pub fn is_symbolic(&self) -> bool {
        matches!(self, MathStructure::Symbolic(_))
    }
    /// `isSymbolic()` for a *text* value (see [`MathStructure::Text`]).
    pub fn is_text(&self) -> bool {
        matches!(self, MathStructure::Text(_))
    }
    pub fn is_addition(&self) -> bool {
        matches!(self, MathStructure::Addition(_))
    }
    pub fn is_multiplication(&self) -> bool {
        matches!(self, MathStructure::Multiplication(_))
    }
    pub fn is_power(&self) -> bool {
        matches!(self, MathStructure::Power { .. })
    }
    pub fn is_function(&self) -> bool {
        matches!(self, MathStructure::Function { .. })
    }
    pub fn is_variable(&self) -> bool {
        matches!(self, MathStructure::Variable(_))
    }
    pub fn is_unit(&self) -> bool {
        matches!(self, MathStructure::Unit { .. })
    }
    pub fn is_comparison(&self) -> bool {
        matches!(self, MathStructure::Comparison { .. })
    }
    pub fn is_bitwise_and(&self) -> bool {
        matches!(self, MathStructure::BitwiseAnd(_))
    }
    pub fn is_bitwise_or(&self) -> bool {
        matches!(self, MathStructure::BitwiseOr(_))
    }
    pub fn is_bitwise_xor(&self) -> bool {
        matches!(self, MathStructure::BitwiseXor(_))
    }
    pub fn is_bitwise_not(&self) -> bool {
        matches!(self, MathStructure::BitwiseNot(_))
    }
    pub fn is_logical_and(&self) -> bool {
        matches!(self, MathStructure::LogicalAnd(_))
    }
    pub fn is_logical_or(&self) -> bool {
        matches!(self, MathStructure::LogicalOr(_))
    }
    pub fn is_logical_xor(&self) -> bool {
        matches!(self, MathStructure::LogicalXor(_))
    }
    pub fn is_logical_not(&self) -> bool {
        matches!(self, MathStructure::LogicalNot(_))
    }
    pub fn is_undefined(&self) -> bool {
        matches!(self, MathStructure::Undefined)
    }
    pub fn is_aborted(&self) -> bool {
        matches!(self, MathStructure::Aborted)
    }
    pub fn is_datetime(&self) -> bool {
        matches!(self, MathStructure::DateTime(_))
    }

    // TODO: port the represents_* family (representsPositive,
    // representsInteger, representsNumber, representsNonMatrix, ...) from
    // MathStructure.cc once variables/functions/units exist to answer the
    // assumption queries they depend on.

    // ------------------------------------------------------------------
    // Value access
    // ------------------------------------------------------------------

    /// `number()` — the numeric value, if this is a number structure.
    pub fn number(&self) -> Option<&Number> {
        match self {
            MathStructure::Number(n) => Some(n),
            _ => None,
        }
    }

    pub fn number_mut(&mut self) -> Option<&mut Number> {
        match self {
            MathStructure::Number(n) => Some(n),
            _ => None,
        }
    }

    /// `symbol()` — the text of a symbolic structure.
    pub fn symbol(&self) -> Option<&str> {
        match self {
            MathStructure::Symbolic(s) => Some(s),
            _ => None,
        }
    }

    /// `symbol()` restricted to text values.
    pub fn text(&self) -> Option<&str> {
        match self {
            MathStructure::Text(s) => Some(s),
            _ => None,
        }
    }

    /// A text value (`MathStructure(string, force_symbol = false)`).
    pub fn text_value(s: impl Into<String>) -> Self {
        MathStructure::Text(s.into())
    }

    /// `base()` — the base of a power structure (child 0 in C++).
    pub fn base(&self) -> Option<&MathStructure> {
        match self {
            MathStructure::Power { base, .. } => Some(base),
            _ => None,
        }
    }

    /// `exponent()` — the exponent of a power structure (child 1 in C++).
    pub fn exponent(&self) -> Option<&MathStructure> {
        match self {
            MathStructure::Power { exponent, .. } => Some(exponent),
            _ => None,
        }
    }

    /// `comparisonType()`.
    pub fn comparison_type(&self) -> Option<ComparisonType> {
        match self {
            MathStructure::Comparison { op, .. } => Some(*op),
            _ => None,
        }
    }

    // ------------------------------------------------------------------
    // Children (`v_subs` in C++)
    // ------------------------------------------------------------------

    /// `size()` / `countChildren()` — number of children. Power and
    /// comparison structures have exactly two children (base/exponent,
    /// left/right); the *Not structures exactly one, matching `v_subs`.
    pub fn size(&self) -> usize {
        use MathStructure::*;
        match self {
            Number(_) | Symbolic(_) | Text(_) | Variable(_) | Unit { .. } | Undefined | Aborted
            | DateTime(_) => 0,
            // A conversion wraps exactly one value.
            Conversion { .. } => 1,
            Vector(v) | Addition(v) | Multiplication(v) | BitwiseAnd(v) | BitwiseOr(v)
            | BitwiseXor(v) | LogicalAnd(v) | LogicalOr(v) | LogicalXor(v) => v.len(),
            Function { args, .. } => args.len(),
            Power { .. } | Comparison { .. } => 2,
            BitwiseNot(_) | LogicalNot(_) => 1,
        }
    }

    /// Child by 0-based index, like the C++ `operator []`. (The C++
    /// `getChild()` is 1-based; the Rust port is uniformly 0-based.)
    pub fn get(&self, index: usize) -> Option<&MathStructure> {
        use MathStructure::*;
        match self {
            Vector(v) | Addition(v) | Multiplication(v) | BitwiseAnd(v) | BitwiseOr(v)
            | BitwiseXor(v) | LogicalAnd(v) | LogicalOr(v) | LogicalXor(v) => v.get(index),
            Function { args, .. } => args.get(index),
            Power { base, exponent } => match index {
                0 => Some(base),
                1 => Some(exponent),
                _ => None,
            },
            Comparison { left, right, .. } => match index {
                0 => Some(left),
                1 => Some(right),
                _ => None,
            },
            BitwiseNot(c) | LogicalNot(c) => (index == 0).then_some(&**c),
            Conversion { value, .. } => (index == 0).then_some(&**value),
            _ => None,
        }
    }

    /// Mutable child by 0-based index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut MathStructure> {
        use MathStructure::*;
        match self {
            Vector(v) | Addition(v) | Multiplication(v) | BitwiseAnd(v) | BitwiseOr(v)
            | BitwiseXor(v) | LogicalAnd(v) | LogicalOr(v) | LogicalXor(v) => v.get_mut(index),
            Function { args, .. } => args.get_mut(index),
            Power { base, exponent } => match index {
                0 => Some(base),
                1 => Some(exponent),
                _ => None,
            },
            Comparison { left, right, .. } => match index {
                0 => Some(left),
                1 => Some(right),
                _ => None,
            },
            BitwiseNot(c) | LogicalNot(c) => (index == 0).then_some(&mut **c),
            Conversion { value, .. } => (index == 0).then_some(&mut **value),
            _ => None,
        }
    }

    /// `last()` — the last child.
    pub fn last(&self) -> Option<&MathStructure> {
        let n = self.size();
        if n == 0 {
            None
        } else {
            self.get(n - 1)
        }
    }

    pub fn last_mut(&mut self) -> Option<&mut MathStructure> {
        let n = self.size();
        if n == 0 {
            None
        } else {
            self.get_mut(n - 1)
        }
    }

    /// Iterate over the children.
    pub fn children(&self) -> impl Iterator<Item = &MathStructure> {
        (0..self.size()).map(move |i| self.get(i).expect("index < size"))
    }

    fn children_vec_mut(&mut self) -> Option<&mut Vec<MathStructure>> {
        use MathStructure::*;
        match self {
            Vector(v) | Addition(v) | Multiplication(v) | BitwiseAnd(v) | BitwiseOr(v)
            | BitwiseXor(v) | LogicalAnd(v) | LogicalOr(v) | LogicalXor(v) => Some(v),
            Function { args, .. } => Some(args),
            _ => None,
        }
    }

    /// `addChild()` — append a child to an n-ary structure (vector,
    /// addition, multiplication, function, bitwise/logical n-ary).
    ///
    /// The C++ version appends to `v_subs` of any type; the Rust enum has
    /// no free-form child vector for fixed-arity or leaf variants, so this
    /// panics for those (a programming error, like the C++ setType being
    /// "dangerous").
    pub fn add_child(&mut self, o: MathStructure) {
        self.children_vec_mut()
            .expect("add_child on a structure without an n-ary child list")
            .push(o);
    }

    /// Insert a child at the front (C++ `insertChild(o, 1)`).
    pub fn prepend_child(&mut self, o: MathStructure) {
        self.children_vec_mut()
            .expect("prepend_child on a structure without an n-ary child list")
            .insert(0, o);
    }

    /// `delChild()` (0-based) — remove and return a child of an n-ary
    /// structure.
    pub fn del_child(&mut self, index: usize) -> Option<MathStructure> {
        let v = self.children_vec_mut()?;
        if index < v.len() {
            Some(v.remove(index))
        } else {
            None
        }
    }

    /// `countTotalChildren(count_function_as_one = false)` — recursive
    /// child count.
    pub fn count_total_children(&self) -> usize {
        let mut count = 0;
        for i in 0..self.size() {
            count += 1 + self.get(i).expect("index < size").count_total_children();
        }
        count
    }

    // ------------------------------------------------------------------
    // Structural transformations (no calculation), from MathStructure.cc
    // ------------------------------------------------------------------

    /// `add(o, append)`: if this is already an addition and `append` is
    /// true, push `o` as another term; otherwise become `Addition[this, o]`.
    pub fn add(&mut self, o: MathStructure, append: bool) {
        match self {
            MathStructure::Addition(v) if append => v.push(o),
            _ => {
                let this = std::mem::take(self);
                *self = MathStructure::Addition(vec![this, o]);
            }
        }
    }

    /// `subtract(o, append)`: negate `o` and add it — subtraction is
    /// represented as an addition with a negated term.
    pub fn subtract(&mut self, mut o: MathStructure, append: bool) {
        o.negate();
        self.add(o, append);
    }

    /// `multiply(o, append)`: if this is already a multiplication and
    /// `append` is true, push `o` as another factor; otherwise become
    /// `Multiplication[this, o]`.
    pub fn multiply(&mut self, o: MathStructure, append: bool) {
        match self {
            MathStructure::Multiplication(v) if append => v.push(o),
            _ => {
                let this = std::mem::take(self);
                *self = MathStructure::Multiplication(vec![this, o]);
            }
        }
    }

    /// `divide(o, append)`: invert `o` and multiply — division is
    /// represented as a multiplication with an inverted factor.
    pub fn divide(&mut self, mut o: MathStructure, append: bool) {
        o.inverse();
        self.multiply(o, append);
    }

    /// `raise(o)`: become `Power { base: this, exponent: o }`.
    pub fn raise(&mut self, o: MathStructure) {
        let this = std::mem::take(self);
        *self = MathStructure::Power {
            base: Box::new(this),
            exponent: Box::new(o),
        };
    }

    /// `inverse()`: `raise(-1)` (C++ raises to `m_minus_one`).
    pub fn inverse(&mut self) {
        self.raise(MathStructure::from(-1));
    }

    /// `negate()` (MathStructure.cc:1896): unconditionally become
    /// `Multiplication[-1, this]`, numbers included. Numeric leaves are only
    /// negated in place by [`calculate_negate`](Self::calculate_negate).
    pub fn negate(&mut self) {
        let this = std::mem::take(self);
        *self = MathStructure::Multiplication(vec![MathStructure::from(-1), this]);
    }

    /// `calculateNegate()` (MathStructure-calculate.cc:6862): negate a
    /// numeric leaf in place; otherwise fall back to structural `negate()`.
    /// Returns true if the value was negated numerically.
    pub fn calculate_negate(&mut self) -> bool {
        if let MathStructure::Number(n) = self {
            let mut nr = n.clone();
            if nr.negate() {
                *n = nr;
                return true;
            }
        }
        self.negate();
        false
    }

    /// `setLogicalNot()`: become `LogicalNot(this)`.
    pub fn set_logical_not(&mut self) {
        let this = std::mem::take(self);
        *self = MathStructure::LogicalNot(Box::new(this));
    }

    /// `setBitwiseNot()`: become `BitwiseNot(this)`.
    pub fn set_bitwise_not(&mut self) {
        let this = std::mem::take(self);
        *self = MathStructure::BitwiseNot(Box::new(this));
    }

    /// `transform(ctype, o)`: become `Comparison { this, ctype, o }`.
    pub fn transform_comparison(&mut self, op: ComparisonType, o: MathStructure) {
        let this = std::mem::take(self);
        *self = MathStructure::Comparison {
            left: Box::new(this),
            op,
            right: Box::new(o),
        };
    }

    // ------------------------------------------------------------------
    // Equality
    // ------------------------------------------------------------------

    /// Structural equality (`equals` in C++). Numbers compare with
    /// `Number::equals` allowing intervals (`allow_interval = true`), per
    /// the port design; use [`MathStructure::equals_ext`] for the C++
    /// default flags.
    pub fn equals(&self, o: &MathStructure) -> bool {
        self.equals_ext(o, true, false)
    }

    /// `equals(o, allow_interval, allow_infinite)`.
    ///
    /// Structural, not mathematical: `1 + 2` equals `1 + 2` but not `2 + 1`
    /// (addition/multiplication children compare in order). Only the
    /// logical and/or/xor containers compare children order-insensitively,
    /// exactly as in the C++ implementation.
    pub fn equals_ext(&self, o: &MathStructure, allow_interval: bool, allow_infinite: bool) -> bool {
        use MathStructure::*;
        let ordered = |a: &Vec<MathStructure>, b: &Vec<MathStructure>| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(x, y)| x.equals_ext(y, allow_interval, allow_infinite))
        };
        match (self, o) {
            (Number(a), Number(b)) => a.equals(b, allow_interval, allow_infinite),
            (Symbolic(a), Symbolic(b)) | (Text(a), Text(b)) => a == b,
            (Variable(a), Variable(b)) => a == b,
            // C++ `equals` compares the unit *and* its prefix.
            (
                Unit { id: a, prefix: pa },
                Unit { id: b, prefix: pb },
            ) => a == b && pa == pb,
            (Undefined, Undefined) | (Aborted, Aborted) => true,
            (DateTime(a), DateTime(b)) => a == b,
            (Vector(a), Vector(b))
            | (Addition(a), Addition(b))
            | (Multiplication(a), Multiplication(b))
            | (BitwiseAnd(a), BitwiseAnd(b))
            | (BitwiseOr(a), BitwiseOr(b))
            | (BitwiseXor(a), BitwiseXor(b)) => ordered(a, b),
            (LogicalAnd(a), LogicalAnd(b))
            | (LogicalOr(a), LogicalOr(b))
            | (LogicalXor(a), LogicalXor(b)) => {
                unordered_children_equal(a, b, allow_interval, allow_infinite)
            }
            (BitwiseNot(a), BitwiseNot(b)) | (LogicalNot(a), LogicalNot(b)) => {
                a.equals_ext(b, allow_interval, allow_infinite)
            }
            (
                Power {
                    base: b1,
                    exponent: e1,
                },
                Power {
                    base: b2,
                    exponent: e2,
                },
            ) => {
                b1.equals_ext(b2, allow_interval, allow_infinite)
                    && e1.equals_ext(e2, allow_interval, allow_infinite)
            }
            // C++ additionally rejects functions whose definition takes no
            // arguments (`o_function->args() == 0`); that needs the
            // function registry and is deferred.
            (Function { id: i1, args: a1 }, Function { id: i2, args: a2 }) => {
                i1 == i2 && ordered(a1, a2)
            }
            (
                Comparison {
                    left: l1,
                    op: c1,
                    right: r1,
                },
                Comparison {
                    left: l2,
                    op: c2,
                    right: r2,
                },
            ) => {
                c1 == c2
                    && l1.equals_ext(l2, allow_interval, allow_infinite)
                    && r1.equals_ext(r2, allow_interval, allow_infinite)
            }
            _ => false,
        }
    }
}

/// Order-insensitive child comparison used for the logical and/or/xor
/// containers, ported from `MathStructure::equals` (each child of `a` must
/// match a distinct, not-yet-taken child of `b`). Empty logical containers
/// compare unequal in C++ (`if(SIZE < 1) return false`).
fn unordered_children_equal(
    a: &[MathStructure],
    b: &[MathStructure],
    allow_interval: bool,
    allow_infinite: bool,
) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let mut taken = vec![false; b.len()];
    for x in a {
        let mut found = false;
        for (i, y) in b.iter().enumerate() {
            if !taken[i] && x.equals_ext(y, allow_interval, allow_infinite) {
                taken[i] = true;
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

// ----------------------------------------------------------------------
// Display placeholder — a debug rendering; the real formatter is ported
// later with MathStructure-print.cc.
// ----------------------------------------------------------------------

impl fmt::Display for MathStructure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use MathStructure::*;
        let joined = |f: &mut fmt::Formatter<'_>, v: &[MathStructure], sep: &str| {
            write!(f, "(")?;
            for (i, c) in v.iter().enumerate() {
                if i > 0 {
                    write!(f, "{sep}")?;
                }
                write!(f, "{c}")?;
            }
            write!(f, ")")
        };
        match self {
            Number(n) => write!(f, "{}", n.print(&PrintOptions::default())),
            Symbolic(s) => write!(f, "{s}"),
            Text(s) => write!(f, "{}", crate::strings::quote_text(s)),
            Conversion { value, .. } => write!(f, "{value}"),
            Vector(v) => {
                write!(f, "[")?;
                for (i, c) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{c}")?;
                }
                write!(f, "]")
            }
            Addition(v) => joined(f, v, " + "),
            Multiplication(v) => joined(f, v, " * "),
            Power { base, exponent } => write!(f, "({base} ^ {exponent})"),
            Function { id, args } => {
                write!(f, "function#{}(", id.0)?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
            Variable(v) => write!(f, "variable#{}", v.0),
            Unit { id, prefix } => match prefix {
                Some(p) => write!(f, "unit#{}[prefix#{}]", id.0, p.0),
                None => write!(f, "unit#{}", id.0),
            },
            Comparison { left, op, right } => {
                let s = match op {
                    ComparisonType::Less => "<",
                    ComparisonType::Greater => ">",
                    ComparisonType::EqualsLess => "<=",
                    ComparisonType::EqualsGreater => ">=",
                    ComparisonType::Equals => "=",
                    ComparisonType::NotEquals => "!=",
                };
                write!(f, "({left} {s} {right})")
            }
            BitwiseAnd(v) => joined(f, v, " & "),
            BitwiseOr(v) => joined(f, v, " | "),
            BitwiseXor(v) => joined(f, v, " xor "),
            BitwiseNot(c) => write!(f, "~{c}"),
            LogicalAnd(v) => joined(f, v, " && "),
            LogicalOr(v) => joined(f, v, " || "),
            LogicalXor(v) => joined(f, v, " xor "),
            LogicalNot(c) => write!(f, "!{c}"),
            Undefined => write!(f, "undefined"),
            Aborted => write!(f, "aborted"),
            DateTime(_) => write!(f, "datetime"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(i: i64) -> MathStructure {
        MathStructure::from(i)
    }

    fn sym(s: &str) -> MathStructure {
        MathStructure::symbolic(s)
    }

    #[test]
    fn default_is_zero_number() {
        // C++ MathStructure() -> init() -> STRUCT_NUMBER with Number zero.
        let m = MathStructure::default();
        assert!(m.is_number());
        assert!(m.is_zero());
        assert_eq!(m.size(), 0);
        assert!(m.equals(&MathStructure::new()));
    }

    #[test]
    fn construction_and_conversions() {
        let a = MathStructure::from_i64(5);
        assert!(a.is_number() && a.is_integer());
        assert!(a.number().unwrap().equals_i64(5));

        let b = MathStructure::from(Number::from_ints(1, 2, 0));
        assert!(b.is_number() && !b.is_integer());

        let h = MathStructure::from_ints(25, 1, -1); // 2.5
        assert!(h.is_number() && !h.is_integer());

        let c = MathStructure::from("x");
        assert!(c.is_symbolic());
        assert_eq!(c.symbol(), Some("x"));

        assert!(num(1).is_one());
        assert!(num(-1).is_minus_one());
    }

    #[test]
    fn equality_same_tree() {
        // 1 + 2 equals 1 + 2 (structurally identical trees).
        let mut a = num(1);
        a.add(num(2), false);
        let mut b = num(1);
        b.add(num(2), false);
        assert!(a.is_addition());
        assert!(a.equals(&b));
        assert_eq!(a, b);
    }

    #[test]
    fn equality_is_structural_not_mathematical() {
        // 1 + 2 does NOT equal 2 + 1: addition children compare in order.
        let mut a = num(1);
        a.add(num(2), false);
        let mut b = num(2);
        b.add(num(1), false);
        assert!(!a.equals(&b));
        assert_ne!(a, b);
        // ... and an addition never equals its numeric value.
        assert!(!a.equals(&num(3)));
    }

    #[test]
    fn equality_across_types_and_ids() {
        assert!(!num(0).equals(&MathStructure::Undefined));
        assert!(MathStructure::Undefined.equals(&MathStructure::Undefined));
        assert!(MathStructure::Aborted.equals(&MathStructure::Aborted));
        assert!(!sym("x").equals(&sym("y")));
        assert!(MathStructure::Variable(VariableId(3)).equals(&MathStructure::Variable(VariableId(3))));
        assert!(!MathStructure::Variable(VariableId(3)).equals(&MathStructure::Variable(VariableId(4))));
        assert!(!MathStructure::unit(UnitId(1)).equals(&MathStructure::Variable(VariableId(1))));
    }

    #[test]
    fn logical_and_children_compare_unordered() {
        // C++ equals() matches LOGICAL_AND/OR/XOR children in any order.
        let a = MathStructure::LogicalAnd(vec![sym("a"), sym("b")]);
        let b = MathStructure::LogicalAnd(vec![sym("b"), sym("a")]);
        assert!(a.equals(&b));
        // ...but bitwise (like addition) stays ordered.
        let c = MathStructure::BitwiseAnd(vec![sym("a"), sym("b")]);
        let d = MathStructure::BitwiseAnd(vec![sym("b"), sym("a")]);
        assert!(!c.equals(&d));
    }

    #[test]
    fn negate_number_in_place() {
        // C++ negate() wraps even a number: 5 -> Multiplication[-1, 5].
        let mut m = num(5);
        m.negate();
        assert!(m.is_multiplication());
        assert!(m.get(0).unwrap().is_minus_one());
        assert!(m.get(1).unwrap().equals(&num(5)));

        // calculateNegate() is the in-place numeric form.
        let mut n = num(5);
        assert!(n.calculate_negate());
        assert!(n.is_number());
        assert!(n.number().unwrap().equals_i64(-5));
        assert!(n.calculate_negate());
        assert!(n.equals(&num(5)));

        // On a non-numeric leaf it falls back to structural negation.
        let mut s = sym("x");
        assert!(!s.calculate_negate());
        assert!(s.is_multiplication());
    }

    #[test]
    fn negate_symbolic_wraps_in_multiplication() {
        // C++ negate(): x -> Multiplication[-1, x].
        let mut m = sym("x");
        m.negate();
        assert!(m.is_multiplication());
        assert_eq!(m.size(), 2);
        assert!(m.get(0).unwrap().is_minus_one());
        assert!(m.get(1).unwrap().equals(&sym("x")));
    }

    #[test]
    fn subtraction_is_addition_with_negated_term() {
        // C++ subtract(): x - y == Addition[x, Multiplication[-1, y]].
        let mut m = sym("x");
        m.subtract(sym("y"), false);
        assert!(m.is_addition());
        assert_eq!(m.size(), 2);
        assert!(m.get(0).unwrap().equals(&sym("x")));
        let t = m.get(1).unwrap();
        assert!(t.is_multiplication());
        assert_eq!(t.size(), 2);
        assert!(t.get(0).unwrap().is_minus_one());
        assert!(t.get(1).unwrap().equals(&sym("y")));
    }

    #[test]
    fn division_is_multiplication_with_inverse_power() {
        // C++ divide(): x / y == Multiplication[x, Power{y, -1}].
        let mut m = sym("x");
        m.divide(sym("y"), false);
        assert!(m.is_multiplication());
        assert_eq!(m.size(), 2);
        assert!(m.get(0).unwrap().equals(&sym("x")));
        let inv = m.get(1).unwrap();
        assert!(inv.is_power());
        assert!(inv.base().unwrap().equals(&sym("y")));
        assert!(inv.exponent().unwrap().is_minus_one());
    }

    #[test]
    fn inverse_raises_to_minus_one() {
        let mut m = sym("y");
        m.inverse();
        assert!(m.is_power());
        assert!(m.base().unwrap().is_symbolic());
        assert!(m.exponent().unwrap().is_minus_one());
    }

    #[test]
    fn add_append_semantics() {
        // append=true on an existing addition pushes a term; append=false
        // always nests (transform(STRUCT_ADDITION, o)).
        let mut m = num(1);
        m.add(num(2), false);
        m.add(num(3), true);
        assert!(m.is_addition());
        assert_eq!(m.size(), 3);

        let mut n = num(1);
        n.add(num(2), false);
        n.add(num(3), false); // nests: (1+2)+3
        assert!(n.is_addition());
        assert_eq!(n.size(), 2);
        assert!(n.get(0).unwrap().is_addition());
        assert!(!m.equals(&n));
    }

    #[test]
    fn raise_builds_power() {
        let mut m = sym("x");
        m.raise(num(2));
        assert!(m.is_power());
        assert!(m.base().unwrap().equals(&sym("x")));
        assert!(m.exponent().unwrap().equals(&num(2)));
        assert_eq!(m.size(), 2);
        assert!(m.get(0).unwrap().is_symbolic());
        assert!(m.get(1).unwrap().is_number());
        assert!(m.get(2).is_none());
    }

    #[test]
    fn child_access_and_mutation() {
        let mut m = MathStructure::Vector(vec![num(1), num(2), num(3)]);
        assert_eq!(m.size(), 3);
        assert!(m.get(1).unwrap().equals(&num(2)));
        assert!(m.last().unwrap().equals(&num(3)));
        // In-place &mut mutation replaces the C++ set* API.
        *m.get_mut(1).unwrap() = sym("x");
        assert!(m.get(1).unwrap().is_symbolic());
        assert_eq!(m.count_total_children(), 3);

        let mut c = sym("a");
        c.transform_comparison(ComparisonType::Less, sym("b"));
        assert!(c.is_comparison());
        assert_eq!(c.comparison_type(), Some(ComparisonType::Less));
        assert!(c.get(0).unwrap().equals(&sym("a")));
        assert!(c.get(1).unwrap().equals(&sym("b")));
    }

    #[test]
    fn add_prepend_del_child() {
        let mut m = MathStructure::Vector(vec![]);
        m.add_child(num(2));
        m.add_child(num(3));
        m.prepend_child(num(1));
        assert_eq!(m.size(), 3);
        assert!(m.get(0).unwrap().equals(&num(1)));
        assert!(m.get(2).unwrap().equals(&num(3)));
        let removed = m.del_child(1).unwrap();
        assert!(removed.equals(&num(2)));
        assert_eq!(m.size(), 2);
        assert!(m.del_child(5).is_none());
    }

    #[test]
    fn nested_tree_equality() {
        // (x + 2) * 3 built twice compares equal; small change breaks it.
        let build = |c: i64| {
            let mut m = sym("x");
            m.add(num(2), false);
            m.multiply(num(c), false);
            m
        };
        assert!(build(3).equals(&build(3)));
        assert!(!build(3).equals(&build(4)));
    }

    #[test]
    fn clear_resets_to_zero() {
        let mut m = sym("x");
        m.raise(num(2));
        m.clear();
        assert!(m.is_zero());
        m.clear_vector();
        assert!(m.is_vector());
        assert_eq!(m.size(), 0);
    }

    #[test]
    fn display_placeholder_smoke() {
        let mut m = sym("x");
        m.add(num(2), false);
        m.multiply(num(3), false);
        assert_eq!(format!("{m}"), "((x + 2) * 3)");
        let mut d = sym("x");
        d.divide(sym("y"), false);
        assert_eq!(format!("{d}"), "(x * (y ^ -1))");
        assert_eq!(format!("{}", MathStructure::Undefined), "undefined");
    }

    #[test]
    fn function_and_not_variants() {
        let f1 = MathStructure::Function {
            id: FunctionId(7),
            args: vec![sym("x"), num(2)],
        };
        let f2 = MathStructure::Function {
            id: FunctionId(7),
            args: vec![sym("x"), num(2)],
        };
        let f3 = MathStructure::Function {
            id: FunctionId(8),
            args: vec![sym("x"), num(2)],
        };
        assert!(f1.equals(&f2));
        assert!(!f1.equals(&f3));
        assert_eq!(f1.size(), 2);

        let mut l = num(1);
        l.set_logical_not();
        assert!(l.is_logical_not());
        assert_eq!(l.size(), 1);
        assert!(l.get(0).unwrap().is_one());

        let mut b = num(1);
        b.set_bitwise_not();
        assert!(b.is_bitwise_not());
        assert!(!l.equals(&b));
    }
}
