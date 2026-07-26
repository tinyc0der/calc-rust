//! Arithmetic simplification core — port of the merge engine in
//! `MathStructure-calculate.cc` (`merge_addition`, `merge_multiplication`,
//! `merge_power`, the `calculate*` wrappers and `calculatesub`).
//!
//! # The C++ merge protocol
//!
//! `merge_addition`/`merge_multiplication`/`merge_power` all have the shape
//!
//! ```cpp
//! int MathStructure::merge_x(MathStructure &mstruct, const EvaluationOptions &eo,
//!                            MathStructure *mparent, size_t index_this,
//!                            size_t index_mstruct, bool reversed);
//! ```
//!
//! and answer "can these two operands be combined into one?" with an `int`
//! whose meanings are fixed by the `MERGE_INDEX` / `MERGE_ALL` macros that
//! consume it (`MathStructure-calculate.cc:5253` and `:6641`):
//!
//! | C++ | [`MergeResult`] | meaning |
//! |-----|-----------------|---------|
//! | `-1` | [`MergeResult::Failed`] | not mergeable; both operands stay |
//! | `0` | [`MergeResult::TryReversed`] | swap the operands and try again (`reversed = true`) |
//! | `1` | [`MergeResult::Merged`] | merged; the result is in `*this`, `mstruct` is erased |
//! | `2` | [`MergeResult::MergedUnchanged`] | merged and `*this` did *not* change value (e.g. `a+0`); `mstruct` is erased |
//! | `3` | [`MergeResult::MergedIntoOther`] | the result is `mstruct`'s value (e.g. `0+a`) |
//!
//! `2` and `3` exist purely so the caller can skip re-scanning: after a `2`
//! the already-merged operand cannot merge with anything new, and after a
//! `3` the C++ swapped the two children through `mparent` instead of
//! copying. The caller flips `2 <-> 3` after a successful reversed retry.
//!
//! The Rust port has no `mparent`: `other` is always merged *into* `self`
//! (the `mparent == NULL` branch of the C++, which uses
//! `set_nocopy(mstruct)` for case `3`). Callers therefore treat all three
//! success codes as "result is in `self`, drop `other`", and only use the
//! distinction for the loop optimizations.
//!
//! # Deliberate omissions (TODO(port))
//!
//! - unit synchronization (`syncUnits`, `eo.sync_units`, temperature modes)
//! - function evaluation and every function-specific identity
//!   (`sin(x)^2+cos(x)^2`, `sgn`, `abs`, `ln`, ...) — needs the function
//!   registry
//! - variable substitution (`eo.calculate_variables`) — needs the variable
//!   registry
//! - `eval()` with structuring/factorization, `evalSort`, `format`
//! - comparison, bitwise and logical merging beyond recursing into children
//! - matrix/vector arithmetic and the non-reorderable matrix merge loop
//! - fraction reduction / polynomial division (`eo.reduce_divisions`)
//! - `split_squares` (needs the prime tables) and interval/precision
//!   bookkeeping (`MERGE_APPROX_AND_PREC`, `b_approx`, `i_precision`)

use crate::options::{ApproximationMode, EvaluationOptions};
use crate::structure::MathStructure;
use qalc_num::Number;

// ----------------------------------------------------------------------
// Merge result
// ----------------------------------------------------------------------

/// Outcome of a `merge_*` call — the Rust spelling of the C++ `int` return
/// documented in the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeResult {
    /// C++ `-1`: the two operands cannot be combined.
    Failed,
    /// C++ `0`: retry with the operands swapped (`reversed = true`).
    TryReversed,
    /// C++ `1`: merged, the result is in `self`.
    Merged,
    /// C++ `2`: merged, and `self` kept its previous value.
    MergedUnchanged,
    /// C++ `3`: merged, and the result is `other`'s value (moved into
    /// `self` by this port, swapped through the parent by the C++).
    MergedIntoOther,
}

impl MergeResult {
    /// C++ `r >= 1`: the two operands became one.
    pub fn is_merged(self) -> bool {
        matches!(
            self,
            MergeResult::Merged | MergeResult::MergedUnchanged | MergeResult::MergedIntoOther
        )
    }

    /// The `2 <-> 3` flip the C++ macros apply after a successful reversed
    /// retry (the roles of `this` and `mstruct` were swapped).
    fn flipped(self) -> MergeResult {
        match self {
            MergeResult::MergedUnchanged => MergeResult::MergedIntoOther,
            MergeResult::MergedIntoOther => MergeResult::MergedUnchanged,
            other => other,
        }
    }
}

/// Signature shared by the three merge operations, so the generic merge
/// loops (`MERGE_ALL` / `MERGE_INDEX`) can be written once.
type MergeFn = fn(&mut MathStructure, &mut MathStructure, &EvaluationOptions, bool) -> MergeResult;

// ----------------------------------------------------------------------
// represents_* predicates
// ----------------------------------------------------------------------

/// Minimal port of the `MathStructure::represents*` family (`MathStructure.cc`).
///
/// Symbolic values answer from `CALCULATOR->defaultAssumptions()`, which
/// defaults to `ASSUMPTION_TYPE_NUMBER` + `ASSUMPTION_SIGN_UNKNOWN`
/// (`Variable.cc:25`): a symbol is a (possibly complex) number of unknown
/// sign, so only `represents_number` and `represents_non_matrix` hold.
/// Variables, units, functions and date/times always answer conservatively
/// here because the registries are not ported yet.
pub(crate) mod represents {
    use crate::structure::MathStructure as M;

    /// `representsNumber(false)`
    pub fn number(m: &M) -> bool {
        match m {
            M::Number(n) => !n.includes_infinity(),
            // default assumptions: ASSUMPTION_TYPE_NUMBER
            M::Symbolic(_) => true,
            M::Power { base, exponent } => {
                (non_zero(base) || positive(exponent)) && number(base) && number(exponent)
            }
            M::Addition(v) | M::Multiplication(v) => v.iter().all(number),
            _ => false,
        }
    }

    /// `representsReal(false)` — false for symbols (complex allowed).
    pub fn real(m: &M) -> bool {
        match m {
            M::Number(n) => n.is_real(),
            M::Addition(v) | M::Multiplication(v) => v.iter().all(real),
            M::Power { base, exponent } => {
                (positive(base) && real(exponent))
                    || (real(base) && integer(exponent) && (positive(exponent) || non_zero(base)))
            }
            _ => false,
        }
    }

    /// `representsInteger(false)`
    pub fn integer(m: &M) -> bool {
        match m {
            M::Number(n) => n.is_integer(),
            M::Addition(v) | M::Multiplication(v) => v.iter().all(integer),
            M::Power { base, exponent } => {
                integer(base) && integer(exponent) && positive(exponent)
            }
            _ => false,
        }
    }

    /// `representsNonInteger(false)`
    pub fn non_integer(m: &M) -> bool {
        matches!(m, M::Number(n) if n.is_rational() && !n.is_integer())
    }

    /// `representsFraction(false)` — a rational strictly between -1 and 1.
    pub fn fraction(m: &M) -> bool {
        matches!(m, M::Number(n) if n.is_fraction())
    }

    /// `representsPositive(false)`
    pub fn positive(m: &M) -> bool {
        match m {
            M::Symbolic(_) => crate::assumptions::unknowns_are_positive(),
            M::Number(n) => n.is_positive(),
            M::Addition(v) => v.iter().all(positive),
            M::Multiplication(v) => {
                let mut b = true;
                for c in v {
                    if negative(c) {
                        b = !b;
                    } else if !positive(c) {
                        return false;
                    }
                }
                b
            }
            M::Power { base, exponent } => {
                (positive(base) && real(exponent))
                    || (non_zero(base) && real(base) && even(exponent) && integer(exponent))
            }
            _ => false,
        }
    }

    /// `representsNegative(false)`
    pub fn negative(m: &M) -> bool {
        match m {
            M::Number(n) => n.is_negative(),
            M::Addition(v) => v.iter().all(negative),
            M::Multiplication(v) => {
                let mut b = false;
                for c in v {
                    if negative(c) {
                        b = !b;
                    } else if !positive(c) {
                        return false;
                    }
                }
                b
            }
            M::Power { base, exponent } => {
                integer(exponent) && odd(exponent) && negative(base)
            }
            _ => false,
        }
    }

    /// `representsNonNegative(false)`
    pub fn non_negative(m: &M) -> bool {
        match m {
            M::Symbolic(_) => crate::assumptions::unknowns_are_positive(),
            M::Number(n) => n.is_non_negative(),
            M::Addition(v) => v.iter().all(non_negative),
            M::Multiplication(v) => {
                let mut b = true;
                for c in v {
                    if negative(c) {
                        b = !b;
                    } else if !non_negative(c) {
                        return false;
                    }
                }
                b
            }
            M::Power { base, exponent } => {
                (base.is_zero() && non_negative(exponent))
                    || (non_negative(base) && real(exponent))
                    || (real(base) && even(exponent) && integer(exponent))
            }
            _ => false,
        }
    }

    /// `representsNonZero(false)`
    pub fn non_zero(m: &M) -> bool {
        match m {
            M::Number(n) => n.is_nonzero(),
            M::Addition(v) => {
                // all terms positive, or all terms negative
                v.iter().all(positive) || v.iter().all(negative)
            }
            M::Multiplication(v) => v.iter().all(non_zero),
            M::Power { base, exponent } => non_zero(base) && number(exponent),
            _ => false,
        }
    }

    /// `representsZero(false)`
    pub fn zero(m: &M) -> bool {
        match m {
            M::Number(n) => n.is_zero(),
            M::Multiplication(v) => v.iter().any(zero),
            _ => false,
        }
    }

    /// `representsEven(false)`
    pub fn even(m: &M) -> bool {
        matches!(m, M::Number(n) if n.is_even())
    }

    /// `representsOdd(false)`
    pub fn odd(m: &M) -> bool {
        matches!(m, M::Number(n) if n.is_odd())
    }

    /// `representsUndefined(include_childs = false, include_infinite = false,
    /// be_strict = false)`.
    pub fn undefined(m: &M) -> bool {
        match m {
            M::Undefined => true,
            M::Power { base, exponent } => {
                (zero(base) && negative(exponent)) || (infinite(base) && zero(exponent))
            }
            M::Multiplication(v) => v.len() > 1 && v[0].is_zero() && infinite(&v[1]),
            _ => false,
        }
    }

    /// `isInfinity()` — an infinite *number* leaf.
    pub fn infinite(m: &M) -> bool {
        matches!(m, M::Number(n) if n.is_infinite(false))
    }

    /// `representsNonMatrix()` — only vectors can be matrices here; the
    /// variable/function cases need the registries.
    pub fn non_matrix(m: &M) -> bool {
        match m {
            M::Vector(v) => v.is_empty() || v.iter().all(|c| !c.is_vector()),
            M::Addition(v) | M::Multiplication(v) => v.iter().all(non_matrix),
            M::Power { base, .. } => non_matrix(base),
            M::Function { id, .. } => crate::builtins::returns_scalar(id.0),
            M::Variable(_) => false,
            _ => true,
        }
    }
}

// ----------------------------------------------------------------------
// Small structural helpers
// ----------------------------------------------------------------------

/// The child vector of an n-ary addition/multiplication.
fn nary_mut(m: &mut MathStructure) -> Option<&mut Vec<MathStructure>> {
    match m {
        MathStructure::Addition(v) | MathStructure::Multiplication(v) => Some(v),
        _ => None,
    }
}

/// `MERGE_ALL2` / the tail of `MERGE_INDEX2`: an n-ary structure that
/// shrank to one child becomes that child (`setToChild(1)`), and an empty
/// one becomes zero (`clear()`).
fn collapse_nary(m: &mut MathStructure) {
    let size = match nary_mut(m) {
        Some(v) => v.len(),
        None => return,
    };
    if size == 1 {
        let child = nary_mut(m).expect("n-ary").remove(0);
        *m = child;
    } else if size == 0 {
        m.clear();
    }
}

/// Two distinct elements of a slice, mutably (`i != j`), returned in
/// `(v[i], v[j])` order.
fn pair_mut<T>(v: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    assert_ne!(i, j, "pair_mut requires distinct indices");
    if i < j {
        let (head, tail) = v.split_at_mut(j);
        (&mut head[i], &mut tail[0])
    } else {
        let (head, tail) = v.split_at_mut(i);
        (&mut tail[0], &mut head[j])
    }
}

/// One `CHILD(i).FUNC(CHILD(j), ...)` attempt including the C++ reversed
/// retry: on `TryReversed` the children are swapped, the merge is retried
/// with `reversed = true`, and the `2 <-> 3` codes are flipped. On failure
/// the swap is undone. The result always ends up in `v[i]`; the caller
/// removes `v[j]` when the result is merged.
fn try_merge_pair(
    v: &mut [MathStructure],
    i: usize,
    j: usize,
    eo: &EvaluationOptions,
    f: MergeFn,
) -> MergeResult {
    let r = {
        let (a, b) = pair_mut(v, i, j);
        f(a, b, eo, false)
    };
    if r != MergeResult::TryReversed {
        return r;
    }
    v.swap(i, j);
    let r2 = {
        let (a, b) = pair_mut(v, i, j);
        f(a, b, eo, true)
    };
    if !r2.is_merged() {
        v.swap(i, j);
        return MergeResult::Failed;
    }
    r2.flipped()
}

/// `MERGE_INDEX(FUNC, TRY_LABEL)` (`MathStructure-calculate.cc:6641`):
/// merge the child at `index` with every other child, restarting whenever
/// something changed.
fn merge_index(
    v: &mut Vec<MathStructure>,
    mut index: usize,
    eo: &EvaluationOptions,
    f: MergeFn,
) -> bool {
    let mut b = false;
    'restart: loop {
        let mut i = 0;
        while i < index {
            let r = try_merge_pair(v, i, index, eo, f);
            if r.is_merged() {
                v.remove(index);
                if !b && r == MergeResult::MergedUnchanged {
                    // this child absorbed the other without changing:
                    // nothing new can merge, stop scanning
                    return true;
                }
                b = true;
                index = i;
                continue 'restart;
            }
            i += 1;
        }
        let mut i = index + 1;
        while i < v.len() {
            let r = try_merge_pair(v, index, i, eo, f);
            if r.is_merged() {
                v.remove(i);
                if !b && r == MergeResult::MergedIntoOther {
                    return true;
                }
                b = true;
                if r != MergeResult::MergedUnchanged {
                    continue 'restart;
                }
                // value unchanged: keep scanning from the same position
                continue;
            }
            i += 1;
        }
        break;
    }
    b
}

/// `MERGE_ALL(FUNC, TRY_LABEL)` (`MathStructure-calculate.cc:5253`): try to
/// merge every child with every other child.
///
/// The C++ macro threads two indices through a `goto` loop to avoid
/// rescanning; this port simply restarts the double loop after every
/// successful merge. Same fixpoint, `O(n^3)` instead of `O(n^2)` in the
/// worst case — acceptable for the sizes the port handles today.
fn merge_all(v: &mut Vec<MathStructure>, eo: &EvaluationOptions, f: MergeFn) -> bool {
    let mut changed = false;
    'restart: loop {
        for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                let r = try_merge_pair(v, i, j, eo, f);
                if r.is_merged() {
                    v.remove(j);
                    changed = true;
                    continue 'restart;
                }
            }
        }
        break;
    }
    changed
}

/// Shared numeric-result acceptance test used by every numeric fast path:
/// the C++ rejects a numeric operation whose result became approximate
/// while neither operand was (unless approximation is allowed anyway).
fn numeric_result_ok(
    result: &Number,
    a: &Number,
    b: &Number,
    eo: &EvaluationOptions,
    check_complex_and_infinite: bool,
) -> bool {
    if eo.approximation < ApproximationMode::Approximate
        && result.is_approximate()
        && !a.is_approximate()
        && !b.is_approximate()
    {
        return false;
    }
    if check_complex_and_infinite {
        if !eo.allow_complex && result.is_complex() && !a.is_complex() && !b.is_complex() {
            return false;
        }
        if !eo.allow_infinite
            && result.includes_infinity()
            && !a.includes_infinity()
            && !b.includes_infinity()
        {
            return false;
        }
    }
    true
}

/// The `(Number, Number)` pair of two number structures, cloned so the
/// borrow of `self`/`other` ends before they are mutated.
fn number_pair(a: &MathStructure, b: &MathStructure) -> Option<(Number, Number)> {
    match (a.number(), b.number()) {
        (Some(x), Some(y)) => Some((x.clone(), y.clone())),
        _ => None,
    }
}

impl MathStructure {
    // ------------------------------------------------------------------
    // merge_addition
    // ------------------------------------------------------------------

    /// `MathStructure::merge_addition` (`MathStructure-calculate.cc:156`).
    pub fn merge_addition(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
    ) -> MergeResult {
        self.merge_addition_ordered(other, eo, false)
    }

    /// `merge_addition` with the C++ `reversed` flag, which only decides
    /// whether new terms are prepended or appended.
    pub fn merge_addition_ordered(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
        reversed: bool,
    ) -> MergeResult {
        use MergeResult::*;

        // number + number
        if let Some((a, b)) = number_pair(self, other) {
            let mut nr = a.clone();
            if nr.add(&b) && numeric_result_ok(&nr, &a, &b, eo, false) {
                let unchanged = a.equals(&nr, false, false);
                *self = MathStructure::Number(nr);
                return if unchanged { MergedUnchanged } else { Merged };
            }
            return Failed;
        }

        // 0+a=a
        if self.is_zero() {
            *self = std::mem::take(other);
            return MergedIntoOther;
        }
        // a+0=a
        if other.is_zero() {
            return MergedUnchanged;
        }

        // infinity+a=infinity (a must be real, otherwise inf + i*inf etc.)
        if let MathStructure::Number(n) = &*self {
            if n.is_infinite(false) && represents::real(other) {
                return MergedUnchanged;
            }
        } else if let MathStructure::Number(n) = &*other {
            if n.is_infinite(false) && represents::real(self) {
                *self = std::mem::take(other);
                return MergedIntoOther;
            }
        }

        if represents::undefined(self) || represents::undefined(other) {
            return Failed;
        }

        // STRUCT_VECTOR: element-wise addition and matrix broadcasting live
        // in `crate::matrix`. (TODO(port): STRUCT_DATETIME arithmetic.)
        if self.is_vector() || (other.is_vector() && !self.is_addition()) {
            return crate::matrix::merge_addition_vector(self, other, eo);
        }

        if self.is_addition() {
            if other.is_addition() {
                // (a1+a2+...)+(b1+b2+...)=a1+a2+...+b1+b2+...
                let terms = match other {
                    MathStructure::Addition(v) => std::mem::take(v),
                    _ => unreachable!(),
                };
                for (k, term) in terms.into_iter().enumerate() {
                    let pos = if reversed {
                        let v = nary_mut(self).expect("addition");
                        let pos = k.min(v.len());
                        v.insert(pos, term);
                        pos
                    } else {
                        let v = nary_mut(self).expect("addition");
                        v.push(term);
                        v.len() - 1
                    };
                    self.calculate_add_index(pos, eo, false);
                }
                collapse_nary(self);
                return Merged;
            }
            // (a1+a2+...)+b=a1+a2+...+b
            let term = std::mem::take(other);
            let pos = {
                let v = nary_mut(self).expect("addition");
                if reversed {
                    v.insert(0, term);
                    0
                } else {
                    v.push(term);
                    v.len() - 1
                }
            };
            self.calculate_add_index(pos, eo, true);
            return Merged;
        }

        if self.is_multiplication() {
            if other.is_addition() || other.is_vector() {
                return TryReversed;
            }
            if other.is_multiplication() {
                return self.merge_addition_of_multiplications(other, eo);
            }
            // ax+x=(a+1)x
            if self.size() == 2
                && self.get(0).expect("size 2").is_number()
                && self.get(1).expect("size 2").equals(other)
            {
                let mut coeff = self
                    .get(0)
                    .and_then(MathStructure::number)
                    .expect("number")
                    .clone();
                if !coeff.add_i64(1) {
                    return Failed;
                }
                *self.get_mut(0).expect("size 2") = MathStructure::Number(coeff);
                self.calculate_multiply_index(0, eo, true);
                return Merged;
            }
            return Failed;
        }

        // `default_addition_merge` (MathStructure-calculate.cc:1098) —
        // reached directly and by `goto` from the POWER/FUNCTION cases.
        if other.is_vector()
            || other.is_datetime()
            || other.is_addition()
            || other.is_multiplication()
        {
            return TryReversed;
        }
        if self.equals(other) {
            // x+x=2x
            self.multiply(MathStructure::from(2), true);
            let last = self.size() - 1;
            self.calculate_multiply_index(last, eo, true);
            return Merged;
        }
        Failed
    }

    /// `ax+bx=(a+b)x`, `axy+xy=(a+1)xy`, `xy+xy=2xy` — the
    /// `STRUCT_MULTIPLICATION` / `STRUCT_MULTIPLICATION` branch of
    /// `merge_addition` (`MathStructure-calculate.cc:1358`).
    fn merge_addition_of_multiplications(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
    ) -> MergeResult {
        let i1 = usize::from(self.get(0).is_some_and(MathStructure::is_number));
        let i2 = usize::from(other.get(0).is_some_and(MathStructure::is_number));
        if self.size() - i1 != other.size() - i2 {
            return MergeResult::Failed;
        }
        for i in i1..self.size() {
            let a = self.get(i).expect("i < size");
            let b = other.get(i + i2 - i1).expect("equal remaining sizes");
            if !a.equals(b) {
                return MergeResult::Failed;
            }
        }
        // Compute the new coefficient before mutating, so a failed
        // Number::add cannot leave a stray factor behind.
        let mut coeff = if i1 == 0 {
            Number::from_i64(1)
        } else {
            self.get(0)
                .and_then(MathStructure::number)
                .expect("number")
                .clone()
        };
        let ok = if i2 == 0 {
            coeff.add_i64(1)
        } else {
            let b = other
                .get(0)
                .and_then(MathStructure::number)
                .expect("number")
                .clone();
            coeff.add(&b)
        };
        if !ok {
            return MergeResult::Failed;
        }
        if i1 == 0 {
            self.prepend_child(MathStructure::Number(coeff));
        } else {
            *self.get_mut(0).expect("non-empty") = MathStructure::Number(coeff);
        }
        self.calculate_multiply_index(0, eo, true);
        MergeResult::Merged
    }

    // ------------------------------------------------------------------
    // merge_multiplication
    // ------------------------------------------------------------------

    /// `MathStructure::merge_multiplication`
    /// (`MathStructure-calculate.cc:1263`).
    pub fn merge_multiplication(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
    ) -> MergeResult {
        self.merge_multiplication_ordered(other, eo, false)
    }

    /// `merge_multiplication` with the C++ `reversed` flag. The C++
    /// `do_append` parameter is always `true` here (the `false` path is only
    /// used by the not-yet-ported nested merge helpers).
    pub fn merge_multiplication_ordered(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
        reversed: bool,
    ) -> MergeResult {
        use MergeResult::*;

        // number * number
        if let Some((a, b)) = number_pair(self, other) {
            let mut nr = a.clone();
            if nr.multiply(&b) && numeric_result_ok(&nr, &a, &b, eo, true) {
                let unchanged = a.equals(&nr, false, false);
                *self = MathStructure::Number(nr);
                return if unchanged { MergedUnchanged } else { Merged };
            }
            return Failed;
        }

        // x*1=x
        if other.is_one() {
            return MergedUnchanged;
        }
        // 1*x=x
        if self.is_one() {
            *self = std::mem::take(other);
            return MergedIntoOther;
        }

        // (+-infinity)*x, x*(+-infinity)
        if let Some(r) = self.merge_multiplication_infinite(other) {
            return r;
        }

        if represents::undefined(self) || represents::undefined(other) {
            return Failed;
        }

        // TODO(port): eo.reduce_divisions — cancelling common factors of a
        // numerator against a polynomial denominator.
        // STRUCT_VECTOR: scalar broadcasting and matrix products.
        if self.is_vector() || (other.is_vector() && !self.is_addition()) {
            return crate::matrix::merge_multiplication_vector(self, other, eo);
        }

        if self.is_addition() {
            if other.is_addition() {
                // multiplication of polynomials
                if eo.expand != 0 && self.size() * other.size() < 500 {
                    self.expand_over(other, eo, reversed);
                    return Merged;
                }
                if self.equals(other) {
                    // (x+y)*(x+y)=(x+y)^2
                    self.raise(MathStructure::from(2));
                    return Merged;
                }
                return Failed;
            }
            if eo.expand == 0 {
                return Failed;
            }
            // (a1+a2+...)*b=(ba1+ba2+...)
            let factor = std::mem::take(other);
            let terms = nary_mut(self).expect("addition").len();
            for i in 0..terms {
                let term = nary_mut(self).expect("addition").get_mut(i).expect("i < size");
                if reversed {
                    let old = std::mem::replace(term, factor.clone());
                    term.multiply(old, true);
                    term.calculate_multiply_index(term.size() - 1, eo, true);
                } else {
                    term.multiply(factor.clone(), true);
                    term.calculate_multiply_index(term.size() - 1, eo, true);
                }
            }
            self.calculatesub_opt(eo, false);
            return Merged;
        }

        if self.is_multiplication() {
            if other.is_addition() || other.is_vector() {
                if eo.expand == 0 {
                    let f = std::mem::take(other);
                    nary_mut(self).expect("multiplication").push(f);
                    return Merged;
                }
                return TryReversed;
            }
            if other.is_multiplication() {
                // (a1*a2*...)(b1*b2*...)=a1*a2*...*b1*b2*...
                let factors = match other {
                    MathStructure::Multiplication(v) => std::mem::take(v),
                    _ => unreachable!(),
                };
                for (k, f) in factors.into_iter().enumerate() {
                    let pos = if reversed {
                        let v = nary_mut(self).expect("multiplication");
                        let pos = k.min(v.len());
                        v.insert(pos, f);
                        pos
                    } else {
                        let v = nary_mut(self).expect("multiplication");
                        v.push(f);
                        v.len() - 1
                    };
                    self.calculate_multiply_index(pos, eo, false);
                }
                collapse_nary(self);
                return Merged;
            }
            // xy*(xy)^a=(xy)^(a+1)
            if let MathStructure::Power { base, exponent } = &*other {
                if exponent.is_number() && self.equals(base) && self.power_combine_allowed(exponent, eo)
                {
                    let mut e = exponent.number().expect("number").clone();
                    if e.add_i64(1) {
                        let b = (**base).clone();
                        *self = MathStructure::Power {
                            base: Box::new(b),
                            exponent: Box::new(MathStructure::Number(e)),
                        };
                        self.calculate_raise_exponent(eo);
                        return Merged;
                    }
                }
            }
            // (a1*a2*...)*b=a1*a2*...*b
            let f = std::mem::take(other);
            let pos = {
                let v = nary_mut(self).expect("multiplication");
                if reversed {
                    v.insert(0, f);
                    0
                } else {
                    v.push(f);
                    v.len() - 1
                }
            };
            self.calculate_multiply_index(pos, eo, true);
            return Merged;
        }

        if self.is_power() {
            if other.is_vector() || other.is_addition() || other.is_multiplication() {
                return TryReversed;
            }
            if other.is_power() {
                if let Some(r) = self.merge_multiplication_powers(other, eo) {
                    return r;
                }
            } else {
                // x*x^a=x^(a+1)
                let matches_base = !other.is_number()
                    && self.exponent().is_some_and(MathStructure::is_number)
                    && self.base().is_some_and(|b| b.equals(other));
                if matches_base {
                    let exp_is_ok = {
                        let e = self.exponent().expect("power");
                        self.base()
                            .is_some_and(|b| base_nonzero_ok(b, e, eo))
                    };
                    if exp_is_ok {
                        let mut e = self
                            .exponent()
                            .and_then(MathStructure::number)
                            .expect("number")
                            .clone();
                        if e.add_i64(1) {
                            *self.get_mut(1).expect("power") = MathStructure::Number(e);
                            self.calculate_raise_exponent(eo);
                            return Merged;
                        }
                    }
                }
            }
            // x^a*0=0
            if other.is_zero()
                && zero_may_absorb(self, eo)
                && !represents::undefined(self)
                && represents::non_matrix(self)
            {
                self.clear();
                return Merged;
            }
            return Failed;
        }

        // default multiplication merge
        if other.is_vector() || other.is_addition() || other.is_multiplication() || other.is_power()
        {
            return TryReversed;
        }
        // x*0=0
        if other.is_zero()
            && zero_may_absorb(self, eo)
            && !represents::undefined(self)
            && represents::non_matrix(self)
        {
            self.clear();
            return MergedIntoOther;
        }
        // 0*x=0
        if self.is_zero()
            && zero_may_absorb(other, eo)
            && !represents::undefined(other)
            && represents::non_matrix(other)
        {
            return MergedUnchanged;
        }
        if self.equals(other) {
            // x*x=x^2
            self.raise(MathStructure::from(2));
            self.calculate_raise_exponent(eo);
            return Merged;
        }
        Failed
    }

    /// The `+-infinity` factor rules at the head of `merge_multiplication`
    /// (`MathStructure-calculate.cc:1292`). Returns `None` when neither
    /// operand is an infinite number (or the sign is unknown), so the caller
    /// continues with the normal cases.
    ///
    /// TODO(port): the `APPROXIMATION_EXACT` fallback that re-evaluates the
    /// other factor with interval arithmetic to decide its sign.
    fn merge_multiplication_infinite(&mut self, other: &MathStructure) -> Option<MergeResult> {
        let self_inf = match self.number() {
            Some(n) if n.is_infinite(false) => Some(n.is_plus_infinity()),
            _ => None,
        };
        if let Some(plus) = self_inf {
            if represents::positive(other) {
                return Some(MergeResult::MergedUnchanged);
            }
            if represents::negative(other) {
                let mut n = Number::new();
                if plus {
                    n.set_minus_infinity(false, false);
                } else {
                    n.set_plus_infinity(false, false);
                }
                *self = MathStructure::Number(n);
                return Some(MergeResult::Merged);
            }
            return None;
        }
        let other_inf = match other.number() {
            Some(n) if n.is_infinite(false) => Some(n.is_plus_infinity()),
            _ => None,
        };
        if let Some(plus) = other_inf {
            let sign_flip = if represents::positive(self) {
                Some(false)
            } else if represents::negative(self) {
                Some(true)
            } else {
                None
            };
            if let Some(flip) = sign_flip {
                let mut n = Number::new();
                if plus != flip {
                    n.set_plus_infinity(false, false);
                } else {
                    n.set_minus_infinity(false, false);
                }
                *self = MathStructure::Number(n);
                return Some(MergeResult::Merged);
            }
        }
        None
    }

    /// `x^a*x^b=x^(a+b)` — the `STRUCT_POWER`/`STRUCT_POWER` branch of
    /// `merge_multiplication` (`MathStructure-calculate.cc:2128`).
    ///
    /// Only the real-numeric-exponent case is ported. The C++ additionally
    /// handles `(-x)^a*x^b`, symbolic exponents and the
    /// `x^a/x^b = x^(a-b+1)/x` split used when the exponents have different
    /// signs and the base might be zero — TODO(port).
    fn merge_multiplication_powers(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
    ) -> Option<MergeResult> {
        let same_base = match (self.base(), other.base()) {
            (Some(a), Some(b)) => a.equals(b),
            _ => false,
        };
        if !same_base {
            return None;
        }
        let (a, b) = match (
            self.exponent().and_then(MathStructure::number),
            other.exponent().and_then(MathStructure::number),
        ) {
            (Some(a), Some(b)) if a.is_real() && b.is_real() => (a.clone(), b.clone()),
            // TODO(port): non-numeric or complex exponents.
            _ => return None,
        };
        let mut sum = a.clone();
        if !sum.add(&b) {
            return None;
        }
        // The exponents combine safely when they have the same sign, or when
        // their sum is negative, or when the base is known (or assumed)
        // non-zero.
        let safe = a.is_positive() == b.is_positive()
            || sum.is_negative()
            || self
                .base()
                .is_some_and(|base| base_nonzero_ok(base, &MathStructure::Number(sum.clone()), eo));
        if !safe {
            return None;
        }
        *self.get_mut(1).expect("power") = MathStructure::Number(sum);
        self.calculate_raise_exponent(eo);
        Some(MergeResult::Merged)
    }

    /// The "is it safe to combine these exponents" guard shared by the
    /// `x*x^a` / `xy*(xy)^a` branches: a negative resulting exponent must
    /// not introduce a division by zero.
    fn power_combine_allowed(&self, exponent: &MathStructure, eo: &EvaluationOptions) -> bool {
        base_nonzero_ok(self, exponent, eo)
    }

    // ------------------------------------------------------------------
    // merge_power
    // ------------------------------------------------------------------

    /// `MathStructure::merge_power` (`MathStructure-calculate.cc:3075`).
    /// `self` is the base, `other` the exponent.
    pub fn merge_power(
        &mut self,
        other: &mut MathStructure,
        eo: &EvaluationOptions,
    ) -> MergeResult {
        use MergeResult::*;

        // number ^ number
        if let Some((a, b)) = number_pair(self, other) {
            let mut nr = a.clone();
            let try_exact = eo.approximation < ApproximationMode::Approximate;
            if nr.raise(&b, try_exact) && numeric_result_ok(&nr, &a, &b, eo, true) {
                let unchanged = a.equals(&nr, false, false);
                *self = MathStructure::Number(nr);
                return if unchanged { MergedUnchanged } else { Merged };
            }
            // TODO(port): the exact-arithmetic fallbacks
            // (`a^(-b)=a^(-b+1)/a`, `(-a)^b=(-1)^b*a^b`, `a^(n/d)=(a^n)^(1/d)`,
            // `eo.split_squares`) that keep roots exact.
            return Failed;
        }

        // x^1=x
        if other.is_one() {
            return MergedUnchanged;
        }

        // x^log(y, x)=y (MathStructure-calculate.cc, the LOG branch of
        // default_power_merge).
        if let MathStructure::Function { id, args } = &*other {
            if id.0 == crate::builtins::id::LOG && args.len() == 2 && args[1].equals(self) {
                *self = args[0].clone();
                return Merged;
            }
        }
        // 1^x=1
        if self.is_one() && represents::number(other) {
            return Merged;
        }

        // infinity^a
        if let Some(n) = self.number() {
            if n.is_infinite(false) {
                let plus = n.is_plus_infinity();
                if represents::negative(other) {
                    // infinity^(-a)=0
                    self.clear();
                    return Merged;
                }
                if represents::positive(other) {
                    if plus || represents::even(other) {
                        let mut nn = Number::new();
                        nn.set_plus_infinity(false, false);
                        *self = MathStructure::Number(nn);
                        return Merged;
                    }
                    if represents::odd(other) {
                        return MergedUnchanged;
                    }
                }
            }
        }

        if represents::undefined(self) || represents::undefined(other) {
            return Failed;
        }

        // STRUCT_VECTOR: integer matrix powers (a negative exponent inverts).
        if self.is_vector() {
            return crate::matrix::merge_power_vector(self, other, eo);
        }

        // 0^a=0 if a is positive
        if self.is_zero() {
            if represents::positive(other) {
                return Merged;
            }
            // 0^negative is a division by zero; the C++ only emits a message.
        }

        // x^0=1
        if other.is_zero()
            && !represents::undefined(self)
            && (eo.assume_denominators_nonzero || represents::non_zero(self))
        {
            *self = MathStructure::from(1);
            return Merged;
        }

        // (xy)^a=x^a*y^a — always for an integer exponent, and for any real
        // exponent when every factor is non-negative, which is what splits
        // `sqrt(xy)` under `/assume positive`.
        if self.is_multiplication()
            && (represents::integer(other)
                || (represents::real(other)
                    && self.children().all(represents::non_negative)))
        {
            let exp = std::mem::take(other);
            let n = nary_mut(self).expect("multiplication").len();
            for i in 0..n {
                let factor = nary_mut(self).expect("multiplication").get_mut(i).expect("i < n");
                factor.calculate_raise(exp.clone(), eo);
            }
            self.calculatesub_opt(eo, false);
            return Merged;
        }

        if self.is_power() {
            // (x^a)^b=x^(a*b) if x>=0 or -1<a<1 or b is integer
            let inner_exp = self.exponent().expect("power").clone();
            if represents::fraction(&inner_exp)
                || represents::integer(other)
                || represents::non_negative(self.base().expect("power"))
            {
                // The C++ rewrites the base to abs(x) when the inner
                // exponent is even; without the function registry we can
                // only accept the cases that need no rewriting.
                if !represents::non_integer(&inner_exp) && !represents::integer(other) {
                    let base_ok = represents::odd(&inner_exp)
                        || represents::non_negative(self.base().expect("power"));
                    if !base_ok {
                        return Failed;
                    }
                }
                let b = std::mem::take(other);
                let exp = self.get_mut(1).expect("power");
                exp.calculate_multiply(b, eo);
                self.calculate_raise_exponent(eo);
                return Merged;
            }
        }

        // A nested square root can sometimes be flattened:
        // `sqrt(8 + 2sqrt(15))` is `sqrt(5) + sqrt(3)`.
        if self.is_addition() && other.number().is_some_and(|n| n.equals(&half(), false, false)) {
            if let Some(denested) = denest_square_root(self) {
                *self = denested;
                self.calculatesub_opt(eo, false);
                return Merged;
            }
        }

        // A root of a sum is worth trying to factor: `sqrt(x + 2sqrt(x) + 1)`
        // is `sqrt((sqrt(x) + 1)^2)`, which the rule above then reduces.
        // Only an exact `1/n` is worth the attempt, and only when factoring
        // actually produces an `n`-th power, so this cannot loop.
        if self.is_addition() {
            if let Some(root) = reciprocal_integer(other) {
                if let Some(factored) = factor_to_power(self, root, eo) {
                    *self = factored;
                    return self.merge_power(other, eo);
                }
            }
        }

        // `default_power_merge` — everything else stays a power.
        Failed
    }

    // ------------------------------------------------------------------
    // calculate* wrappers
    // ------------------------------------------------------------------

    /// `calculateAddIndex` (`MathStructure-calculate.cc:7079`). `self` must
    /// be an addition.
    pub fn calculate_add_index(
        &mut self,
        index: usize,
        eo: &EvaluationOptions,
        check_size: bool,
    ) -> bool {
        self.calculate_merge_index(index, eo, MathStructure::merge_addition_ordered, check_size)
    }

    /// `calculateMultiplyIndex` (`MathStructure-calculate.cc:6971`). `self`
    /// must be a multiplication.
    pub fn calculate_multiply_index(
        &mut self,
        index: usize,
        eo: &EvaluationOptions,
        check_size: bool,
    ) -> bool {
        self.calculate_merge_index(
            index,
            eo,
            MathStructure::merge_multiplication_ordered,
            check_size,
        )
    }

    fn calculate_merge_index(
        &mut self,
        index: usize,
        eo: &EvaluationOptions,
        f: MergeFn,
        check_size: bool,
    ) -> bool {
        let b = match nary_mut(self) {
            Some(v) if index < v.len() => merge_index(v, index, eo, f),
            // C++ logs "This is a bug. Please report it." and returns false.
            _ => return false,
        };
        // MERGE_INDEX2
        if b && check_size {
            collapse_nary(self);
        }
        b
    }

    /// `calculateAdd` (`MathStructure-calculate.cc:7090`).
    pub fn calculate_add(&mut self, madd: MathStructure, eo: &EvaluationOptions) -> bool {
        if let Some((a, b)) = number_pair(self, &madd) {
            let mut nr = a.clone();
            if nr.add(&b) && numeric_result_ok(&nr, &a, &b, eo, false) {
                *self = MathStructure::Number(nr);
                return true;
            }
        }
        self.add(madd, true);
        let last = self.size() - 1;
        self.calculate_add_index(last, eo, true)
    }

    /// `calculateSubtract` (`MathStructure-calculate.cc:7103`).
    pub fn calculate_subtract(&mut self, msub: MathStructure, eo: &EvaluationOptions) -> bool {
        if let Some((a, b)) = number_pair(self, &msub) {
            let mut nr = a.clone();
            if nr.subtract(&b) && numeric_result_ok(&nr, &a, &b, eo, false) {
                *self = MathStructure::Number(nr);
                return true;
            }
        }
        self.add(msub, true);
        let last = self.size() - 1;
        self.get_mut(last).expect("just appended").calculate_negate_eo(eo);
        self.calculate_add_index(last, eo, true)
    }

    /// `calculateMultiply` (`MathStructure-calculate.cc:7048`).
    pub fn calculate_multiply(&mut self, mmul: MathStructure, eo: &EvaluationOptions) -> bool {
        if let Some((a, b)) = number_pair(self, &mmul) {
            let mut nr = a.clone();
            if nr.multiply(&b) && numeric_result_ok(&nr, &a, &b, eo, true) {
                *self = MathStructure::Number(nr);
                return true;
            }
        }
        self.multiply(mmul, true);
        let last = self.size() - 1;
        self.calculate_multiply_index(last, eo, true)
    }

    /// `calculateDivide` (`MathStructure-calculate.cc:7061`).
    pub fn calculate_divide(&mut self, mdiv: MathStructure, eo: &EvaluationOptions) -> bool {
        if let Some((a, b)) = number_pair(self, &mdiv) {
            let mut nr = a.clone();
            if nr.divide(&b) && numeric_result_ok(&nr, &a, &b, eo, true) {
                *self = MathStructure::Number(nr);
                return true;
            }
        }
        self.multiply(mdiv, true);
        let last = self.size() - 1;
        self.get_mut(last).expect("just appended").calculate_inverse(eo);
        self.calculate_multiply_index(last, eo, true)
    }

    /// `calculateRaise` (`MathStructure-calculate.cc:6898`).
    pub fn calculate_raise(&mut self, mexp: MathStructure, eo: &EvaluationOptions) -> bool {
        if let Some((a, b)) = number_pair(self, &mexp) {
            let mut nr = a.clone();
            let try_exact = eo.approximation < ApproximationMode::Approximate;
            if nr.raise(&b, try_exact) && numeric_result_ok(&nr, &a, &b, eo, true) {
                *self = MathStructure::Number(nr);
                return true;
            }
        }
        self.raise(mexp);
        self.calculate_raise_exponent(eo)
    }

    /// `calculateRaiseExponent` (`MathStructure-calculate.cc:6886`): merge
    /// the base and exponent of an existing power, replacing the power with
    /// the merged base on success.
    pub fn calculate_raise_exponent(&mut self, eo: &EvaluationOptions) -> bool {
        let merged = match self {
            MathStructure::Power { base, exponent } => base.merge_power(exponent, eo).is_merged(),
            // C++ logs "This is a bug. Please report it." and returns false.
            _ => return false,
        };
        if merged {
            if let MathStructure::Power { base, .. } = self {
                let b = std::mem::take(&mut **base);
                *self = b;
            }
            return true;
        }
        false
    }

    /// `calculateInverse` (`MathStructure-calculate.cc:6859`).
    pub fn calculate_inverse(&mut self, eo: &EvaluationOptions) -> bool {
        self.calculate_raise(MathStructure::from(-1), eo)
    }

    /// `calculateNegate` (`MathStructure-calculate.cc:6862`) — the full
    /// port, including the multiplication merge that
    /// [`MathStructure::calculate_negate`] (the options-free shortcut in
    /// `structure.rs`) leaves out.
    pub fn calculate_negate_eo(&mut self, eo: &EvaluationOptions) -> bool {
        if let MathStructure::Number(n) = self {
            let mut nr = n.clone();
            if nr.negate()
                && (eo.approximation >= ApproximationMode::Approximate
                    || !nr.is_approximate()
                    || n.is_approximate())
            {
                *self = MathStructure::Number(nr);
                return true;
            }
            self.negate();
            return false;
        }
        if !self.is_multiplication() {
            let this = std::mem::take(self);
            *self = MathStructure::Multiplication(vec![this]);
        }
        self.prepend_child(MathStructure::from(-1));
        self.calculate_multiply_index(0, eo, true)
    }

    /// `(a1+a2+...)*(b1+b2+...)` — the polynomial expansion at the head of
    /// the `STRUCT_ADDITION`/`STRUCT_ADDITION` branch of
    /// `merge_multiplication` (`MathStructure-calculate.cc:1857`).
    fn expand_over(&mut self, other: &mut MathStructure, eo: &EvaluationOptions, reversed: bool) {
        let msave = self.clone();
        let terms = match other {
            MathStructure::Addition(v) => std::mem::take(v),
            _ => unreachable!("expand_over expects an addition"),
        };
        *self = MathStructure::Addition(Vec::with_capacity(terms.len()));
        for term in terms {
            let mut product = msave.clone();
            if reversed {
                let old = std::mem::replace(&mut product, term);
                product.multiply(old, true);
            } else {
                product.multiply(term, true);
            }
            let last = product.size() - 1;
            product.calculate_multiply_index(last, eo, true);
            nary_mut(self).expect("addition").push(product);
        }
        self.calculatesub_opt(eo, false);
    }

    // ------------------------------------------------------------------
    // calculatesub
    // ------------------------------------------------------------------

    /// `MathStructure::calculatesub` (`MathStructure-calculate.cc:5583`)
    /// with `recursive = true`: evaluate the children bottom-up, then merge
    /// them pairwise.
    pub fn calculatesub(&mut self, eo: &EvaluationOptions) -> bool {
        self.calculatesub_opt(eo, true)
    }

    /// `calculatesub` with the C++ `recursive` flag.
    pub fn calculatesub_opt(&mut self, eo: &EvaluationOptions, recursive: bool) -> bool {
        let mut b = false;

        if self.is_power() {
            if recursive {
                if let MathStructure::Power { base, exponent } = self {
                    if base.calculatesub_opt(eo, true) {
                        b = true;
                    }
                    if exponent.calculatesub_opt(eo, true) {
                        b = true;
                    }
                }
            }
            // TODO(port): eo.sync_units relative-temperature handling.
            if self.calculate_raise_exponent(eo) {
                b = true;
            }
            return b;
        }

        if self.is_addition() || self.is_multiplication() {
            let f: MergeFn = if self.is_addition() {
                MathStructure::merge_addition_ordered
            } else {
                MathStructure::merge_multiplication_ordered
            };
            // MERGE_RECURSE (MathStructure-calculate.cc:5245)
            if recursive {
                let v = nary_mut(self).expect("n-ary");
                for child in v.iter_mut() {
                    if !child.is_number() && child.calculatesub_opt(eo, true) {
                        b = true;
                    }
                }
            }
            // TODO(port): eo.sync_units (syncUnits) before merging, and the
            // separate non-reorderable loop for matrix factors.
            if merge_all(nary_mut(self).expect("n-ary"), eo, f) {
                b = true;
            }
            // MERGE_ALL2
            collapse_nary(self);
            return b;
        }

        // TODO(port): comparison evaluation, bitwise/logical merging,
        // function calculation, variable substitution and vector operations.
        // Everything else only propagates the recursion into its children.
        if recursive {
            for i in 0..self.size() {
                if let Some(child) = self.get_mut(i) {
                    if child.calculatesub_opt(eo, true) {
                        b = true;
                    }
                }
            }
        }
        b
    }
}

/// `eo.keep_zero_units` (MathStructure-calculate.cc:2989): multiplying by zero
/// normally collapses the product, but a factor carrying a unit keeps it, so
/// `0 m` stays `0 m` and can still be converted.
/// One half, as a `Number`.
fn half() -> Number {
    Number::from_ints(1, 2, 0)
}

/// `sqrt(a + b*sqrt(c))` = `sqrt((a+d)/2) + sqrt((a-d)/2)`, where
/// `d = sqrt(a^2 - b^2*c)`, whenever that `d` is itself rational.
///
/// This is the classical denesting: squaring the right-hand side gives back
/// `a + b*sqrt(c)` exactly, so it is an identity rather than an
/// approximation — and it only fires when `d` comes out rational, which is
/// what keeps `sqrt(1 + sqrt(2))` alone.
fn denest_square_root(sum: &MathStructure) -> Option<MathStructure> {
    let MathStructure::Addition(terms) = sum else {
        return None;
    };
    if terms.len() != 2 {
        return None;
    }
    // One term is the rational part, the other a rational multiple of a
    // square root of a rational.
    let (a, radical) = match (terms[0].number(), terms[1].number()) {
        (Some(n), None) => (n.clone(), &terms[1]),
        (None, Some(n)) => (n.clone(), &terms[0]),
        _ => return None,
    };
    if !a.is_rational() || a.is_approximate() {
        return None;
    }
    let (b, c) = rational_multiple_of_square_root(radical)?;

    // d^2 = a^2 - b^2*c
    let mut d = a.clone();
    d.square();
    let mut subtrahend = b.clone();
    subtrahend.square();
    subtrahend.multiply(&c);
    if !d.subtract(&subtrahend) || d.is_negative() {
        return None;
    }
    if !d.sqrt() || !d.is_rational() || d.is_approximate() {
        return None;
    }

    let two = Number::from_i64(2);
    let mut p = a.clone();
    let mut q = a;
    if !p.add(&d) || !q.subtract(&d) || !p.divide(&two) || !q.divide(&two) {
        return None;
    }
    if p.is_negative() || q.is_negative() {
        return None;
    }
    // Emit `sqrt(n)` calls rather than `n^(1/2)` powers: that is the shape a
    // typed `sqrt(5)` keeps in exact mode, and the two forms do not compare
    // equal, so `sqrt(8 + 2sqrt(15)) - (sqrt(5) + sqrt(3))` would not cancel.
    let root = |n: Number| MathStructure::Function {
        id: crate::ids::FunctionId(crate::builtins::id::SQRT),
        args: vec![MathStructure::Number(n)],
    };
    let second = if b.is_negative() {
        MathStructure::Multiplication(vec![
            MathStructure::Number(Number::from_i64(-1)),
            root(q),
        ])
    } else {
        root(q)
    };
    Some(MathStructure::Addition(vec![root(p), second]))
}

/// `b` and `c` when `m` is `b*sqrt(c)` (or a bare `sqrt(c)`) with both
/// rational and `c` positive.
fn rational_multiple_of_square_root(m: &MathStructure) -> Option<(Number, Number)> {
    let (coefficient, power) = match m {
        MathStructure::Power { .. } => (Number::from_i64(1), m),
        MathStructure::Multiplication(v) if v.len() == 2 => {
            (v[0].number()?.clone(), &v[1])
        }
        _ => return None,
    };
    if !coefficient.is_rational() || coefficient.is_approximate() {
        return None;
    }
    // A square root reaches here in two shapes: as `n^(1/2)` when it was
    // written that way, and as an unevaluated `sqrt(n)` call when exact mode
    // refused to turn it into a float.
    let radicand = match power {
        MathStructure::Power { base, exponent }
            if exponent
                .number()
                .is_some_and(|e| e.equals(&half(), false, false)) =>
        {
            base.number()?.clone()
        }
        MathStructure::Function { id, args }
            if id.0 == crate::builtins::id::SQRT && args.len() == 1 =>
        {
            args[0].number()?.clone()
        }
        _ => return None,
    };
    if !radicand.is_rational() || radicand.is_approximate() || !radicand.is_positive() {
        return None;
    }
    Some((coefficient, radicand))
}

/// `n` when `m` is exactly `1/n` for an integer `n >= 2`.
fn reciprocal_integer(m: &MathStructure) -> Option<i64> {
    let n = m.number()?;
    if !n.is_rational() || n.is_approximate() {
        return None;
    }
    let mut inverted = n.clone();
    if !inverted.recip() || !inverted.is_integer() {
        return None;
    }
    inverted.to_i64().filter(|v| *v >= 2)
}

/// Factor `m` and keep the result only when it came out as an exact `root`-th
/// power (possibly with a numeric coefficient in front).
fn factor_to_power(
    m: &MathStructure,
    root: i64,
    eo: &EvaluationOptions,
) -> Option<MathStructure> {
    let factored = crate::polynomial::factor(m, eo);
    let is_root_power = |x: &MathStructure| {
        matches!(x, MathStructure::Power { exponent, .. }
            if exponent.number().is_some_and(|e| e.equals_i64(root)))
    };
    match &factored {
        x if is_root_power(x) => Some(factored),
        MathStructure::Multiplication(v) if v.iter().any(is_root_power) => Some(factored),
        _ => None,
    }
}

fn zero_may_absorb(other: &MathStructure, eo: &EvaluationOptions) -> bool {
    !eo.keep_zero_units || !crate::units::contains_unit(other)
}

/// "may the exponent of `base` become/stay negative?" — the recurring C++
/// guard
///
/// ```cpp
/// (!eo.warn_about_denominators_assumed_nonzero && eo.assume_denominators_nonzero && !base.representsZero(true))
///  || base.representsNonZero(true)
///  || exponent.representsPositive()
/// ```
fn base_nonzero_ok(base: &MathStructure, exponent: &MathStructure, eo: &EvaluationOptions) -> bool {
    (!eo.warn_about_denominators_assumed_nonzero
        && eo.assume_denominators_nonzero
        && !represents::zero(base))
        || represents::non_zero(base)
        || represents::positive(exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eo() -> EvaluationOptions {
        EvaluationOptions::default()
    }

    fn num(i: i64) -> MathStructure {
        MathStructure::from(i)
    }

    fn rat(n: i64, d: i64) -> MathStructure {
        MathStructure::from(Number::from_ints(n, d, 0))
    }

    fn sym(s: &str) -> MathStructure {
        MathStructure::symbolic(s)
    }

    /// Build `Addition[a, b]` and evaluate it, like the parser + eval would.
    fn eval_add(a: MathStructure, b: MathStructure) -> MathStructure {
        let mut m = MathStructure::Addition(vec![a, b]);
        m.calculatesub(&eo());
        m
    }

    fn eval_mul(a: MathStructure, b: MathStructure) -> MathStructure {
        let mut m = MathStructure::Multiplication(vec![a, b]);
        m.calculatesub(&eo());
        m
    }

    fn eval_pow(a: MathStructure, b: MathStructure) -> MathStructure {
        let mut m = MathStructure::Power {
            base: Box::new(a),
            exponent: Box::new(b),
        };
        m.calculatesub(&eo());
        m
    }

    // -- numeric arithmetic through the tree ---------------------------

    #[test]
    fn number_addition() {
        // operators.batch: "1 + 2" -> 3
        assert!(eval_add(num(1), num(2)).equals(&num(3)));
        // "5 - 2" is Addition[5, Multiplication[-1, 2]]
        let mut m = num(5);
        m.subtract(num(2), false);
        m.calculatesub(&eo());
        assert!(m.equals(&num(3)));
    }

    #[test]
    fn number_multiplication() {
        // operators.batch: "2*3" -> 6
        assert!(eval_mul(num(2), num(3)).equals(&num(6)));
        // "6/2" -> 3, built as Multiplication[6, Power{2,-1}]
        let mut m = num(6);
        m.divide(num(2), false);
        m.calculatesub(&eo());
        assert!(m.equals(&num(3)));
    }

    #[test]
    fn number_power() {
        // operators.batch: "5 ^ 2" -> 25
        assert!(eval_pow(num(5), num(2)).equals(&num(25)));
        assert!(eval_pow(num(2), num(10)).equals(&num(1024)));
        // negative exponent stays exact
        assert!(eval_pow(num(2), num(-2)).equals(&rat(1, 4)));
    }

    #[test]
    fn exact_rational_addition() {
        // 1/3 + 1/6 = 1/2, exactly (no float contamination).
        let r = eval_add(rat(1, 3), rat(1, 6));
        assert!(r.equals(&rat(1, 2)));
        let n = r.number().expect("number");
        assert!(!n.is_approximate());
        assert!(n.is_rational());
    }

    #[test]
    fn exact_rational_division() {
        // 1/2 as a division stays the exact rational 1/2.
        let mut m = num(1);
        m.divide(num(2), false);
        m.calculatesub(&eo());
        assert!(m.equals(&rat(1, 2)));
        assert!(!m.number().expect("number").is_approximate());
    }

    // -- identity elimination -------------------------------------------

    #[test]
    fn additive_identity() {
        // x+0=x and 0+x=x
        assert!(eval_add(sym("x"), num(0)).equals(&sym("x")));
        assert!(eval_add(num(0), sym("x")).equals(&sym("x")));
    }

    #[test]
    fn multiplicative_identity() {
        // x*1=x and 1*x=x
        assert!(eval_mul(sym("x"), num(1)).equals(&sym("x")));
        assert!(eval_mul(num(1), sym("x")).equals(&sym("x")));
    }

    #[test]
    fn multiplication_by_zero() {
        // x*0=0 and 0*x=0
        assert!(eval_mul(sym("x"), num(0)).is_zero());
        assert!(eval_mul(num(0), sym("x")).is_zero());
    }

    #[test]
    fn power_identities() {
        // x^1=x, x^0=1, 1^x=1, 0^2=0
        assert!(eval_pow(sym("x"), num(1)).equals(&sym("x")));
        assert!(eval_pow(sym("x"), num(0)).is_one());
        assert!(eval_pow(num(1), sym("x")).is_one());
        assert!(eval_pow(num(0), num(2)).is_zero());
    }

    // -- symbolic collection --------------------------------------------

    #[test]
    fn symbol_plus_symbol_collects_coefficient() {
        // x+x=2x. Without evalSort (not ported) the numeric factor is
        // appended, so the result is Multiplication[x, 2].
        let r = eval_add(sym("x"), sym("x"));
        assert!(r.is_multiplication());
        assert_eq!(r.size(), 2);
        assert!(r.get(0).expect("factor").equals(&sym("x")));
        assert!(r.get(1).expect("factor").equals(&num(2)));
    }

    #[test]
    fn coefficient_collection() {
        // 2x + 3x = 5x
        let two_x = MathStructure::Multiplication(vec![num(2), sym("x")]);
        let three_x = MathStructure::Multiplication(vec![num(3), sym("x")]);
        let r = eval_add(two_x, three_x);
        assert!(r.is_multiplication());
        assert!(r.get(0).expect("coefficient").equals(&num(5)));
        assert!(r.get(1).expect("symbol").equals(&sym("x")));
    }

    #[test]
    fn coefficient_collection_with_implicit_one() {
        // 3x + x = 4x (the C++ prepends m_one to the second factor)
        let three_x = MathStructure::Multiplication(vec![num(3), sym("x")]);
        let r = eval_add(three_x, sym("x"));
        assert!(r.is_multiplication());
        assert!(r.get(0).expect("coefficient").equals(&num(4)));
        assert!(r.get(1).expect("symbol").equals(&sym("x")));
    }

    #[test]
    fn coefficient_collection_cancels_to_zero() {
        // 2x - 2x = 0 (the coefficient becomes 0 and x*0=0 collapses it)
        let two_x = MathStructure::Multiplication(vec![num(2), sym("x")]);
        let minus_two_x = MathStructure::Multiplication(vec![num(-2), sym("x")]);
        assert!(eval_add(two_x, minus_two_x).is_zero());
    }

    #[test]
    fn symbol_times_symbol_becomes_square() {
        // x*x=x^2
        let r = eval_mul(sym("x"), sym("x"));
        assert!(r.is_power());
        assert!(r.base().expect("base").equals(&sym("x")));
        assert!(r.exponent().expect("exponent").equals(&num(2)));
    }

    #[test]
    fn powers_of_same_base_add_exponents() {
        // x^2 * x^3 = x^5
        let x2 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(2)),
        };
        let x3 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(3)),
        };
        let r = eval_mul(x2, x3);
        assert!(r.is_power());
        assert!(r.base().expect("base").equals(&sym("x")));
        assert!(r.exponent().expect("exponent").equals(&num(5)));
    }

    #[test]
    fn power_times_base_increments_exponent() {
        // x^2 * x = x^3 (and the reversed order goes through TryReversed)
        let x2 = || MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(2)),
        };
        for r in [eval_mul(x2(), sym("x")), eval_mul(sym("x"), x2())] {
            assert!(r.is_power(), "{r}");
            assert!(r.exponent().expect("exponent").equals(&num(3)), "{r}");
        }
    }

    #[test]
    fn dividing_powers_cancels() {
        // x^3 / x = x^2, i.e. x^3 * x^-1
        let x3 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(3)),
        };
        let xm1 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(-1)),
        };
        let r = eval_mul(x3, xm1);
        assert!(r.is_power());
        assert!(r.exponent().expect("exponent").equals(&num(2)));
        // x / x = 1
        let x = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(1)),
        };
        let xm1 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(-1)),
        };
        assert!(eval_mul(x, xm1).is_one());
    }

    #[test]
    fn power_of_power_multiplies_exponents() {
        // (x^2)^3 = x^6
        let x2 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(2)),
        };
        let r = eval_pow(x2, num(3));
        assert!(r.is_power());
        assert!(r.base().expect("base").equals(&sym("x")));
        assert!(r.exponent().expect("exponent").equals(&num(6)));
    }

    #[test]
    fn power_of_power_not_valid_for_even_inner_exponent() {
        // (x^2)^(1/2) must NOT become x — x could be negative.
        let x2 = MathStructure::Power {
            base: Box::new(sym("x")),
            exponent: Box::new(num(2)),
        };
        let r = eval_pow(x2, rat(1, 2));
        assert!(r.is_power());
        assert!(r.base().expect("base").is_power());
    }

    // -- flattening / association ---------------------------------------

    #[test]
    fn nested_addition_flattens() {
        // (x + y) + z = x + y + z (one flat addition)
        let inner = MathStructure::Addition(vec![sym("x"), sym("y")]);
        let mut m = MathStructure::Addition(vec![inner, sym("z")]);
        m.calculatesub(&eo());
        assert!(m.is_addition());
        assert_eq!(m.size(), 3);
        assert!(m.get(2).expect("term").equals(&sym("z")));
    }

    #[test]
    fn nested_multiplication_flattens_and_folds_numbers() {
        // 2 * (3 * x) = 6x
        let inner = MathStructure::Multiplication(vec![num(3), sym("x")]);
        let mut m = MathStructure::Multiplication(vec![num(2), inner]);
        m.calculatesub(&eo());
        assert!(m.is_multiplication());
        assert_eq!(m.size(), 2);
        assert!(m.get(0).expect("factor").equals(&num(6)));
        assert!(m.get(1).expect("factor").equals(&sym("x")));
    }

    #[test]
    fn deeply_nested_numeric_tree() {
        // ((1+2) + (3*4)) ^ 1 = 15
        let a = MathStructure::Addition(vec![num(1), num(2)]);
        let b = MathStructure::Multiplication(vec![num(3), num(4)]);
        let mut m = MathStructure::Power {
            base: Box::new(MathStructure::Addition(vec![a, b])),
            exponent: Box::new(num(1)),
        };
        m.calculatesub(&eo());
        assert!(m.equals(&num(15)));
    }

    // -- calculate* wrappers ---------------------------------------------

    #[test]
    fn calculate_wrappers_on_numbers() {
        let mut m = num(7);
        assert!(m.calculate_add(num(3), &eo()));
        assert!(m.equals(&num(10)));
        assert!(m.calculate_subtract(num(4), &eo()));
        assert!(m.equals(&num(6)));
        assert!(m.calculate_multiply(num(3), &eo()));
        assert!(m.equals(&num(18)));
        assert!(m.calculate_divide(num(9), &eo()));
        assert!(m.equals(&num(2)));
        assert!(m.calculate_raise(num(5), &eo()));
        assert!(m.equals(&num(32)));
        assert!(m.calculate_negate_eo(&eo()));
        assert!(m.equals(&num(-32)));
    }

    #[test]
    fn calculate_wrappers_on_symbols() {
        // x + x through calculate_add, x * x through calculate_multiply
        let mut m = sym("x");
        assert!(m.calculate_add(sym("x"), &eo()));
        assert!(m.is_multiplication());

        let mut m = sym("y");
        assert!(m.calculate_multiply(sym("y"), &eo()));
        assert!(m.is_power());
        assert!(m.exponent().expect("exponent").equals(&num(2)));

        // x - x = 0
        let mut m = sym("x");
        assert!(m.calculate_subtract(sym("x"), &eo()));
        assert!(m.is_zero());
    }

    #[test]
    fn expansion_of_products_of_sums() {
        // 2 * (x + 3) = 2x + 6
        let sum = MathStructure::Addition(vec![sym("x"), num(3)]);
        let r = eval_mul(num(2), sum);
        assert!(r.is_addition(), "{r}");
        assert_eq!(r.size(), 2);
        // one term is the constant 6, the other the 2x product
        let has_six = (0..r.size()).any(|i| r.get(i).expect("term").equals(&num(6)));
        assert!(has_six, "{r}");
    }

    #[test]
    fn merge_result_codes() {
        // a+0 leaves `this` untouched (C++ 2), 0+a moves the other operand
        // into `this` (C++ 3), and a reversed retry is requested for
        // symbol + multiplication (C++ 0).
        let mut a = sym("x");
        let mut b = num(0);
        assert_eq!(a.merge_addition(&mut b, &eo()), MergeResult::MergedUnchanged);

        let mut a = num(0);
        let mut b = sym("x");
        assert_eq!(a.merge_addition(&mut b, &eo()), MergeResult::MergedIntoOther);
        assert!(a.equals(&sym("x")));

        let mut a = sym("x");
        let mut b = MathStructure::Multiplication(vec![num(2), sym("x")]);
        assert_eq!(a.merge_addition(&mut b, &eo()), MergeResult::TryReversed);

        let mut a = sym("x");
        let mut b = sym("y");
        assert_eq!(a.merge_addition(&mut b, &eo()), MergeResult::Failed);
    }

    #[test]
    fn undefined_does_not_merge() {
        let mut a = MathStructure::Undefined;
        let mut b = sym("x");
        assert_eq!(a.merge_addition(&mut b, &eo()), MergeResult::Failed);
        assert_eq!(a.merge_multiplication(&mut b, &eo()), MergeResult::Failed);
    }

    #[test]
    fn infinity_absorbs_real_terms() {
        let mut inf = Number::new();
        inf.set_plus_infinity(false, false);
        let r = eval_add(MathStructure::Number(inf.clone()), num(5));
        assert!(r.number().expect("number").is_plus_infinity());

        // 5 + infinity too (the reversed direction)
        let r = eval_add(num(5), MathStructure::Number(inf.clone()));
        assert!(r.number().expect("number").is_plus_infinity());

        // (-2) * infinity = -infinity
        let r = eval_mul(num(-2), MathStructure::Number(inf));
        assert!(r.number().expect("number").is_minus_infinity());
    }

    #[test]
    fn approximation_mode_gates_inexact_results() {
        // 2^(1/2) has no exact rational value: in TRY_EXACT it stays a
        // power, in APPROXIMATE it becomes a number.
        let mut exact = MathStructure::Power {
            base: Box::new(num(2)),
            exponent: Box::new(rat(1, 2)),
        };
        exact.calculatesub(&EvaluationOptions::default());
        assert!(exact.is_power(), "{exact}");

        let mut approx = MathStructure::Power {
            base: Box::new(num(2)),
            exponent: Box::new(rat(1, 2)),
        };
        approx.calculatesub(&EvaluationOptions::approximate());
        assert!(approx.is_number(), "{approx}");
        assert!(approx.number().expect("number").is_approximate());
    }
}

#[cfg(test)]
mod denest_tests {
    use crate::session::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.evaluate_line("/set approximation exact").ok();
        s.evaluate_line("/set fr 2").ok();
        s
    }

    #[test]
    fn a_nested_square_root_flattens_when_it_can() {
        let mut s = session();
        assert_eq!(s.evaluate_line("sqrt(8 + 2*sqrt(15))").unwrap(), "sqrt(5) + sqrt(3)");
        // The denested form has to be the same shape a typed `sqrt(5)` keeps,
        // or the difference will not cancel.
        assert_eq!(
            s.evaluate_line("sqrt(8 + 2*sqrt(15)) - (sqrt(5) + sqrt(3))").unwrap(),
            "0"
        );
        assert_eq!(s.evaluate_line("sqrt(3 - 2*sqrt(2))").unwrap(), "sqrt(2) - 1");
    }

    #[test]
    fn a_root_that_does_not_denest_is_left_alone() {
        let mut s = session();
        assert_eq!(s.evaluate_line("sqrt(1 + sqrt(2))").unwrap(), "sqrt(1 + sqrt(2))");
    }
}
