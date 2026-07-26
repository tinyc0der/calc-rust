//! Vector and matrix support — port of `MathStructure-matrixvector.cc` and
//! `BuiltinFunctions-matrixvector.cc`.
//!
//! A matrix is a non-empty [`MathStructure::Vector`] whose children are all
//! vectors of equal length (`MathStructure::isMatrix`, MathStructure.cc:714);
//! any other vector is a flat vector. That length may be zero — `[[],[]]` is
//! a 2×0 matrix — so a column count is never safe to divide or chunk by. The
//! two are not separate types, so nearly every operation here starts by
//! classifying its operand.
//!
//! The module owns three things:
//!
//! * the builtin functions (`det`, `transpose`, `rref`, …), dispatched from
//!   [`crate::builtins::calculate_functions`];
//! * the vector cases of the arithmetic merge engine, called from the three
//!   hooks in [`crate::calculate`] so that `calculate.rs` stays scalar-only;
//! * the print-time vector formatting (`MathStructure::formatsub`'s
//!   `STRUCT_VECTOR` case), which collapses one-element vectors and
//!   single-row matrices.

use crate::ids::FunctionId;
use crate::options::EvaluationOptions;
use crate::structure::MathStructure;
use crate::MergeResult;
use qalc_num::Number;

use MathStructure as M;

/// `FUNCTION_ID_*` values from BuiltinFunctions.h:213. `CROSS` has no
/// builtin id in the C++ (it is an XML-defined function); 1148 is free in
/// the matrix/vector block.
pub mod id {
    pub const VECTOR: u32 = 1100;
    pub const LIMITS: u32 = 1101;
    pub const RANK: u32 = 1102;
    pub const SORT: u32 = 1103;
    pub const DIMENSION: u32 = 1105;
    pub const MATRIX: u32 = 1106;
    pub const MERGE_VECTORS: u32 = 1107;
    pub const MATRIX_TO_VECTOR: u32 = 1108;
    pub const AREA: u32 = 1109;
    pub const ROWS: u32 = 1110;
    pub const COLUMNS: u32 = 1111;
    pub const ROW: u32 = 1112;
    pub const COLUMN: u32 = 1113;
    pub const ELEMENTS: u32 = 1114;
    pub const ELEMENT: u32 = 1115;
    pub const TRANSPOSE: u32 = 1116;
    pub const IDENTITY: u32 = 1117;
    pub const DETERMINANT: u32 = 1118;
    pub const PERMANENT: u32 = 1119;
    pub const ADJOINT: u32 = 1120;
    pub const COFACTOR: u32 = 1121;
    pub const INVERSE: u32 = 1122;
    pub const MAGNITUDE: u32 = 1123;
    pub const ENTRYWISE: u32 = 1125;
    pub const GENERATE_VECTOR: u32 = 1128;
    pub const RREF: u32 = 1132;
    pub const MATRIX_RANK: u32 = 1133;
    pub const DOT_PRODUCT: u32 = 1134;
    pub const ENTRYWISE_MULTIPLICATION: u32 = 1135;
    pub const ENTRYWISE_DIVISION: u32 = 1136;
    pub const ENTRYWISE_POWER: u32 = 1137;
    pub const NORM: u32 = 1138;
    pub const VERTCAT: u32 = 1139;
    pub const HORZCAT: u32 = 1140;
    pub const CROSS: u32 = 1148;
}

/// Resolve a matrix/vector builtin name to its id.
pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let v = match name {
        "vector" => id::VECTOR,
        "slice" | "limits" => id::LIMITS,
        "rank" => id::RANK,
        "sort" => id::SORT,
        "dimension" => id::DIMENSION,
        "matrix" => id::MATRIX,
        "combine" | "mergevectors" => id::MERGE_VECTORS,
        "matrix2vector" => id::MATRIX_TO_VECTOR,
        "part" | "area" => id::AREA,
        "rows" => id::ROWS,
        "columns" => id::COLUMNS,
        "row" => id::ROW,
        "column" => id::COLUMN,
        "elements" => id::ELEMENTS,
        "element" => id::ELEMENT,
        "transpose" => id::TRANSPOSE,
        "identity" => id::IDENTITY,
        "det" | "determinant" => id::DETERMINANT,
        "permanent" => id::PERMANENT,
        "adj" | "adjoint" => id::ADJOINT,
        "cofactor" => id::COFACTOR,
        "inverse" | "inv" => id::INVERSE,
        "magnitude" => id::MAGNITUDE,
        "entrywise" => id::ENTRYWISE,
        "genvector" => id::GENERATE_VECTOR,
        "rref" => id::RREF,
        "rk" => id::MATRIX_RANK,
        "dot" => id::DOT_PRODUCT,
        "multiply" | "times" | "hadamard" => id::ENTRYWISE_MULTIPLICATION,
        "divide" | "rdivide" => id::ENTRYWISE_DIVISION,
        "pow" | "raise" | "power" => id::ENTRYWISE_POWER,
        "norm" => id::NORM,
        "vertcat" => id::VERTCAT,
        "horzcat" => id::HORZCAT,
        "cross" => id::CROSS,
        _ => return None,
    };
    Some(FunctionId(v))
}

/// Display names for the ids above, used when a call cannot be evaluated.
pub fn function_name(fid: u32) -> Option<&'static str> {
    Some(match fid {
        id::VECTOR => "vector",
        id::LIMITS => "slice",
        id::RANK => "rank",
        id::SORT => "sort",
        id::DIMENSION => "dimension",
        id::MATRIX => "matrix",
        id::MERGE_VECTORS => "combine",
        id::MATRIX_TO_VECTOR => "matrix2vector",
        id::AREA => "part",
        id::ROWS => "rows",
        id::COLUMNS => "columns",
        id::ROW => "row",
        id::COLUMN => "column",
        id::ELEMENTS => "elements",
        id::ELEMENT => "element",
        id::TRANSPOSE => "transpose",
        id::IDENTITY => "identity",
        id::DETERMINANT => "det",
        id::PERMANENT => "permanent",
        id::ADJOINT => "adj",
        id::COFACTOR => "cofactor",
        id::INVERSE => "inverse",
        id::MAGNITUDE => "magnitude",
        id::ENTRYWISE => "entrywise",
        id::GENERATE_VECTOR => "genvector",
        id::RREF => "rref",
        id::MATRIX_RANK => "rk",
        id::DOT_PRODUCT => "dot",
        id::ENTRYWISE_MULTIPLICATION => "multiply",
        id::ENTRYWISE_DIVISION => "divide",
        id::ENTRYWISE_POWER => "pow",
        id::NORM => "norm",
        id::VERTCAT => "vertcat",
        id::HORZCAT => "horzcat",
        id::CROSS => "cross",
        _ => return None,
    })
}

// ----------------------------------------------------------------------
// Classification
// ----------------------------------------------------------------------

/// `MathStructure::isMatrix` (MathStructure.cc:714).
pub fn is_matrix(m: &M) -> bool {
    let M::Vector(v) = m else { return false };
    if v.is_empty() {
        return false;
    }
    let mut cols = None;
    for c in v {
        let M::Vector(row) = c else { return false };
        match cols {
            None => cols = Some(row.len()),
            Some(n) if n != row.len() => return false,
            _ => {}
        }
    }
    true
}

/// `MathStructure::rows` (MathStructure-matrixvector.cc:221).
pub fn rows(m: &M) -> usize {
    match m {
        M::Vector(v) if v.is_empty() => 0,
        M::Vector(v) if is_matrix(m) => v.len(),
        _ => 1,
    }
}

/// `MathStructure::columns` (MathStructure-matrixvector.cc:226).
pub fn columns(m: &M) -> usize {
    match m {
        M::Vector(v) if v.is_empty() => 0,
        M::Vector(v) => {
            if is_matrix(m) {
                v[0].size()
            } else {
                v.len()
            }
        }
        _ => 1,
    }
}

/// `representsScalar()`, reduced to the node types this port has.
pub fn represents_scalar(m: &M) -> bool {
    match m {
        M::Vector(_) | M::Function { .. } | M::Variable(_) => false,
        M::Addition(v) | M::Multiplication(v) => v.iter().all(represents_scalar),
        M::Power { base, exponent } => represents_scalar(base) && represents_scalar(exponent),
        _ => true,
    }
}

/// `representsNonMatrix()` — true when `m` can never be a matrix.
fn non_matrix(m: &M) -> bool {
    match m {
        M::Vector(v) => v.iter().all(|c| !c.is_vector()),
        M::Addition(v) | M::Multiplication(v) => v.iter().all(non_matrix),
        M::Power { base, .. } => non_matrix(base),
        M::Function { .. } | M::Variable(_) => false,
        _ => true,
    }
}

/// The rows of `m` as a rectangular grid, applying `MatrixArgument`'s
/// coercions (Function.cc:2482): a scalar becomes 1×1, a flat vector one row.
fn as_matrix(m: &M) -> Option<Vec<Vec<M>>> {
    match m {
        M::Vector(v) if is_matrix(m) => Some(
            v.iter()
                .map(|r| r.children().cloned().collect())
                .collect(),
        ),
        M::Vector(v) => {
            if v.iter().any(|c| c.is_vector()) {
                None
            } else {
                Some(vec![v.clone()])
            }
        }
        _ if represents_scalar(m) => Some(vec![vec![m.clone()]]),
        _ => None,
    }
}

/// `VectorArgument`'s coercion (Function.cc:2396): a scalar becomes a
/// one-element vector; a single-column matrix is transposed to a row.
fn as_vector(m: &M) -> Vec<M> {
    match m {
        M::Vector(v) => {
            if is_matrix(m) && columns(m) == 1 && rows(m) > 1 {
                v.iter().map(|r| r.get(0).cloned().unwrap_or_default()).collect()
            } else {
                v.clone()
            }
        }
        _ => vec![m.clone()],
    }
}

fn matrix_struct(grid: Vec<Vec<M>>) -> M {
    M::Vector(grid.into_iter().map(M::Vector).collect())
}

fn num(i: i64) -> M {
    M::Number(Number::from_i64(i))
}

fn usize_num(i: usize) -> M {
    M::Number(Number::from_i64(i as i64))
}

/// An integer argument, or `None` if it is not an integer number.
fn int_arg(m: &M) -> Option<i64> {
    m.number().filter(|n| n.is_integer()).and_then(|n| n.to_i64())
}

// ----------------------------------------------------------------------
// Small evaluated-arithmetic helpers
// ----------------------------------------------------------------------

fn eo() -> EvaluationOptions {
    EvaluationOptions::default()
}

/// `MathStructure::calculateAdd` — build the sum and merge it.
fn calc_add(a: &mut M, b: M) {
    a.add(b, true);
    a.calculatesub(&eo());
}

fn calc_sub(a: &mut M, b: M) {
    a.subtract(b, true);
    a.calculatesub(&eo());
}

fn calc_mul(a: &mut M, b: M) {
    a.multiply(b, true);
    a.calculatesub(&eo());
}

fn calc_div(a: &mut M, b: M) {
    a.divide(b, true);
    a.calculatesub(&eo());
}

fn calc_pow(a: &mut M, b: M) {
    a.raise(b);
    a.calculatesub(&eo());
}

fn calc_neg(a: &mut M) {
    a.negate();
    a.calculatesub(&eo());
}

fn added(a: &M, b: &M) -> M {
    let mut r = a.clone();
    calc_add(&mut r, b.clone());
    r
}

fn multiplied(a: &M, b: &M) -> M {
    let mut r = a.clone();
    calc_mul(&mut r, b.clone());
    r
}

/// Fully evaluate a subexpression (functions plus the merge engine).
fn eval_full(m: &mut M) {
    let opts = eo();
    for _ in 0..8 {
        let f = crate::builtins::calculate_functions(m);
        let g = m.calculatesub(&opts);
        if !f && !g {
            break;
        }
    }
}

/// `MathStructure::replace` — substitute every occurrence of `from`.
pub fn replace(m: &mut M, from: &M, to: &M) {
    if m.equals(from) {
        *m = to.clone();
        return;
    }
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            replace(child, from, to);
        }
    }
}

// ----------------------------------------------------------------------
// Merge hooks (called from calculate.rs)
// ----------------------------------------------------------------------

/// The `STRUCT_VECTOR` case of `merge_addition`
/// (MathStructure-calculate.cc:208).
pub fn merge_addition_vector(a: &mut M, b: &mut M, _eo: &EvaluationOptions) -> MergeResult {
    use MergeResult::*;
    if !a.is_vector() {
        return if b.is_vector() { TryReversed } else { Failed };
    }
    if b.is_addition() {
        return TryReversed;
    }
    if b.is_vector() {
        let (b1, b2) = (is_matrix(a), is_matrix(b));
        if (!b1 && !non_matrix(a)) || (!b2 && !non_matrix(b)) {
            return Failed;
        }
        return match broadcast(a, b, b1, b2) {
            Some((x, y)) => {
                *a = zip_with(&x, &y, added);
                Merged
            }
            None => Failed,
        };
    }
    if represents_scalar(b) {
        // [a1,a2,...]+b=[a1+b,a2+b,...]
        map_elements(a, &|e| calc_add(e, b.clone()));
        return Merged;
    }
    Failed
}

/// The `STRUCT_VECTOR` case of `merge_multiplication`
/// (MathStructure-calculate.cc:1734).
pub fn merge_multiplication_vector(
    a: &mut M,
    b: &mut M,
    _eo: &EvaluationOptions,
) -> MergeResult {
    use MergeResult::*;
    if !a.is_vector() {
        return if b.is_vector() { TryReversed } else { Failed };
    }
    if b.is_addition() {
        return TryReversed;
    }
    if b.is_vector() {
        // A row vector times a matrix is treated as a 1×n matrix.
        let mut lhs = a.clone();
        if !is_matrix(&lhs) && is_matrix(b) {
            if !non_matrix(&lhs) || lhs.size() != b.size() {
                return Failed;
            }
            lhs = M::Vector(vec![lhs]);
        }
        if !is_matrix(&lhs) {
            // Two plain vectors: the C++ refuses and points at
            // cross()/dot()/hadamard().
            return Failed;
        }
        let mut rhs = b.clone();
        if !is_matrix(&rhs) {
            // Matlab style: the second operand becomes a 1×n matrix, so the
            // first must have exactly one column.
            if columns(&lhs) != 1 {
                return Failed;
            }
            rhs = M::Vector(vec![rhs]);
        }
        let (Some(x), Some(y)) = (as_matrix(&lhs), as_matrix(&rhs)) else {
            return Failed;
        };
        let Some(p) = matrix_product(&x, &y) else {
            return Failed;
        };
        *a = collapse_1x1(p);
        return Merged;
    }
    if represents_scalar(b) {
        map_elements(a, &|e| calc_mul(e, b.clone()));
        return Merged;
    }
    Failed
}

/// The `STRUCT_VECTOR` case of `merge_power`
/// (MathStructure-calculate.cc:3422): integer matrix powers, with a negative
/// exponent inverting the result.
pub fn merge_power_vector(a: &mut M, b: &mut M, _eo: &EvaluationOptions) -> MergeResult {
    use MergeResult::*;
    if !a.is_vector() {
        return Failed;
    }
    let Some(exp) = int_arg(b) else { return Failed };
    let Some(grid) = as_matrix(a) else { return Failed };
    if grid.len() != grid[0].len() {
        return Failed;
    }
    let n = exp.unsigned_abs();
    let mut acc = if n == 0 {
        identity_grid(grid.len())
    } else {
        let mut acc = grid.clone();
        for _ in 1..n {
            acc = match matrix_product(&acc, &grid) {
                Some(p) => p,
                None => return Failed,
            };
        }
        acc
    };
    if exp < 0 {
        acc = match invert(&acc) {
            Some(inv) => inv,
            None => return Failed,
        };
    }
    *a = collapse_1x1(acc);
    Merged
}

/// Apply `f` to every element of a vector or matrix.
fn map_elements(m: &mut M, f: &dyn Fn(&mut M)) {
    let M::Vector(v) = m else { return };
    for child in v.iter_mut() {
        if child.is_vector() {
            map_elements(child, f);
        } else {
            f(child);
        }
    }
}

/// Broadcast two vector operands to a common shape, following the row/column
/// expansion rules of the vector cases in `merge_addition`. Returns the two
/// operands as equally-shaped grids of rows.
fn broadcast(a: &M, b: &M, a_matrix: bool, b_matrix: bool) -> Option<(Vec<Vec<M>>, Vec<Vec<M>>)> {
    let mut x: Vec<Vec<M>> = if a_matrix {
        as_matrix(a)?
    } else {
        vec![a.children().cloned().collect()]
    };
    let mut y: Vec<Vec<M>> = if b_matrix {
        as_matrix(b)?
    } else {
        vec![b.children().cloned().collect()]
    };
    // A column vector broadcasts across the other operand's columns.
    if b_matrix && y[0].len() == 1 && ((!a_matrix && !x[0].is_empty()) || x.len() == y.len()) {
        if !a_matrix {
            x = vec![x[0].clone(); y.len()];
        }
        let cols = x[0].len();
        y = y.iter().map(|r| vec![r[0].clone(); cols]).collect();
        return Some((x, y));
    }
    if a_matrix && x[0].len() == 1 && ((!b_matrix && !y[0].is_empty()) || x.len() == y.len()) {
        let cols = y[0].len();
        x = x.iter().map(|r| vec![r[0].clone(); cols]).collect();
        if !b_matrix {
            y = vec![y[0].clone(); x.len()];
        }
        return Some((x, y));
    }
    if x.len() == y.len() && x[0].len() == y[0].len() {
        return Some((x, y));
    }
    // A matrix and a row vector of matching width: apply row-wise.
    if a_matrix && !b_matrix && x[0].len() == y[0].len() {
        y = vec![y[0].clone(); x.len()];
        return Some((x, y));
    }
    if b_matrix && !a_matrix && y[0].len() == x[0].len() {
        x = vec![x[0].clone(); y.len()];
        return Some((x, y));
    }
    None
}

/// Combine two equally-shaped grids and rebuild the original shape: a
/// one-row result of a non-matrix operation stays flat.
fn zip_with(x: &[Vec<M>], y: &[Vec<M>], f: impl Fn(&M, &M) -> M) -> M {
    let grid: Vec<Vec<M>> = x
        .iter()
        .zip(y)
        .map(|(rx, ry)| rx.iter().zip(ry).map(|(a, b)| f(a, b)).collect())
        .collect();
    if grid.len() == 1 {
        M::Vector(grid.into_iter().next().expect("one row"))
    } else {
        matrix_struct(grid)
    }
}

fn collapse_1x1(grid: Vec<Vec<M>>) -> M {
    if grid.len() == 1 && grid[0].len() == 1 {
        grid.into_iter().next().expect("row").into_iter().next().expect("cell")
    } else {
        matrix_struct(grid)
    }
}

// ----------------------------------------------------------------------
// Linear algebra
// ----------------------------------------------------------------------

fn matrix_product(a: &[Vec<M>], b: &[Vec<M>]) -> Option<Vec<Vec<M>>> {
    let inner = a[0].len();
    if inner != b.len() {
        return None;
    }
    let cols = b[0].len();
    let mut out = Vec::with_capacity(a.len());
    for row in a {
        let mut r = Vec::with_capacity(cols);
        for c in 0..cols {
            let mut acc = num(0);
            for k in 0..inner {
                calc_add(&mut acc, multiplied(&row[k], &b[k][c]));
            }
            r.push(acc);
        }
        out.push(r);
    }
    Some(out)
}

fn identity_grid(n: usize) -> Vec<Vec<M>> {
    (0..n)
        .map(|r| (0..n).map(|c| num(if r == c { 1 } else { 0 })).collect())
        .collect()
}

/// The minor obtained by deleting row `r` and column `c` (0-based).
fn minor(g: &[Vec<M>], r: usize, c: usize) -> Vec<Vec<M>> {
    g.iter()
        .enumerate()
        .filter(|(i, _)| *i != r)
        .map(|(_, row)| {
            row.iter()
                .enumerate()
                .filter(|(j, _)| *j != c)
                .map(|(_, v)| v.clone())
                .collect()
        })
        .collect()
}

/// Laplace expansion. Exact for the symbolic entries this port supports,
/// where the C++ switches to Gaussian elimination for numeric matrices —
/// both give the same value.
fn determinant(g: &[Vec<M>]) -> Option<M> {
    let n = g.len();
    if n == 0 || g.iter().any(|r| r.len() != n) {
        return None;
    }
    if n == 1 {
        return Some(g[0][0].clone());
    }
    if n == 2 {
        let mut acc = multiplied(&g[0][0], &g[1][1]);
        calc_sub(&mut acc, multiplied(&g[0][1], &g[1][0]));
        return Some(acc);
    }
    // Expand along the row with the most zeroes, which keeps the recursion
    // cheap on sparse matrices.
    let best = (0..n)
        .max_by_key(|&r| g[r].iter().filter(|e| e.is_zero()).count())
        .unwrap_or(0);
    let mut acc = num(0);
    for c in 0..n {
        if g[best][c].is_zero() {
            continue;
        }
        let sub = determinant(&minor(g, best, c))?;
        let mut term = multiplied(&g[best][c], &sub);
        if (best + c) % 2 == 1 {
            calc_neg(&mut term);
        }
        calc_add(&mut acc, term);
    }
    Some(acc)
}

fn permanent(g: &[Vec<M>]) -> Option<M> {
    let n = g.len();
    if n == 0 || g.iter().any(|r| r.len() != n) {
        return None;
    }
    if n == 1 {
        return Some(g[0][0].clone());
    }
    let mut acc = num(0);
    for c in 0..n {
        let sub = permanent(&minor(g, 0, c))?;
        calc_add(&mut acc, multiplied(&g[0][c], &sub));
    }
    Some(acc)
}

/// The cofactor `C(r, c)` (1-based), i.e. `(-1)^(r+c)` times the minor's
/// determinant (MathStructure-matrixvector.cc:878).
fn cofactor(g: &[Vec<M>], r: usize, c: usize) -> Option<M> {
    if g.len() < 2 || r == 0 || c == 0 || r > g.len() || c > g[0].len() {
        return None;
    }
    let mut d = determinant(&minor(g, r - 1, c - 1))?;
    if (r + c) % 2 == 1 {
        calc_neg(&mut d);
    }
    Some(d)
}

/// The adjugate: the transposed cofactor matrix
/// (MathStructure-matrixvector.cc:847).
fn adjoint(g: &[Vec<M>]) -> Option<Vec<Vec<M>>> {
    let n = g.len();
    if n == 0 || g.iter().any(|r| r.len() != n) {
        return None;
    }
    if n == 1 {
        return Some(vec![vec![num(1)]]);
    }
    let mut out = vec![vec![num(0); n]; n];
    for r in 0..n {
        for c in 0..n {
            out[r][c] = cofactor(g, r + 1, c + 1)?;
        }
    }
    Some(transpose_grid(out))
}

fn transpose_grid(g: Vec<Vec<M>>) -> Vec<Vec<M>> {
    if g.is_empty() {
        return g;
    }
    let cols = g[0].len();
    (0..cols)
        .map(|c| g.iter().map(|row| row[c].clone()).collect())
        .collect()
}

fn invert(g: &[Vec<M>]) -> Option<Vec<Vec<M>>> {
    let det = determinant(g)?;
    if det.is_zero() {
        return None;
    }
    let adj = adjoint(g)?;
    Some(
        adj.into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|mut e| {
                        calc_div(&mut e, det.clone());
                        e
                    })
                    .collect()
            })
            .collect(),
    )
}

/// `matrix_to_rref` (BuiltinFunctions-matrixvector.cc:900) — Gauss-Jordan
/// reduction to reduced row echelon form.
fn rref(g: &mut Vec<Vec<M>>) -> bool {
    let rows_n = g.len();
    if rows_n == 0 {
        return true;
    }
    let cols_n = g[0].len();
    let mut cur = 0usize;
    let mut c = 0usize;
    while c < cols_n {
        let mut found = false;
        for r in cur..rows_n {
            if g[r][c].is_zero() {
                if !g[r][c].is_number() {
                    return false;
                }
                continue;
            }
            if !g[r][c].is_number() && !represents_scalar(&g[r][c]) {
                return false;
            }
            if r != cur {
                g.swap(r, cur);
            }
            let pivot = g[cur][c].clone();
            for r2 in 0..rows_n {
                if r2 == cur || g[r2][c].is_zero() {
                    continue;
                }
                let mut factor = g[r2][c].clone();
                calc_div(&mut factor, pivot.clone());
                calc_neg(&mut factor);
                for c2 in 0..cols_n {
                    if c2 == c {
                        g[r2][c2] = num(0);
                    } else {
                        let add = multiplied(&g[cur][c2], &factor);
                        calc_add(&mut g[r2][c2], add);
                    }
                }
            }
            for c2 in 0..cols_n {
                if c2 != c {
                    calc_div(&mut g[cur][c2], pivot.clone());
                }
            }
            g[cur][c] = num(1);
            cur += 1;
            found = true;
            break;
        }
        if cur == rows_n {
            break;
        }
        if !found {
            c += 1;
        }
    }
    true
}

// ----------------------------------------------------------------------
// Builtin dispatch
// ----------------------------------------------------------------------

/// Evaluate a matrix/vector builtin in place. Returns true when the call was
/// replaced by a value.
pub fn calculate_function(m: &mut M) -> bool {
    let M::Function { id: fid, args } = m else {
        return false;
    };
    let fid = fid.0;
    if function_name(fid).is_none() {
        return false;
    }
    let args = args.clone();
    match apply(fid, &args) {
        Some(r) => {
            *m = r;
            true
        }
        None => false,
    }
}

fn apply(fid: u32, args: &[M]) -> Option<M> {
    match fid {
        id::VECTOR => Some(M::Vector(args.to_vec())),
        id::MATRIX => build_matrix(args),
        id::MERGE_VECTORS => {
            let mut out = Vec::new();
            for a in args {
                match a {
                    M::Vector(v) => out.extend(v.iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            Some(M::Vector(out))
        }
        id::MATRIX_TO_VECTOR => {
            let a = args.first()?;
            if !a.is_vector() {
                return Some(a.clone());
            }
            let mut out = Vec::new();
            for c in a.children() {
                match c {
                    M::Vector(v) => out.extend(v.iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            Some(M::Vector(out))
        }
        id::DIMENSION => Some(usize_num(as_vector(args.first()?).len())),
        id::ELEMENTS => {
            let a = args.first()?;
            if is_matrix(a) {
                Some(usize_num(rows(a) * columns(a)))
            } else if a.is_vector() {
                Some(usize_num(a.size()))
            } else {
                Some(num(1))
            }
        }
        id::ROWS => Some(usize_num(rows(args.first()?))),
        id::COLUMNS => Some(usize_num(columns(args.first()?))),
        id::ROW => {
            let g = as_matrix(args.first()?)?;
            let r = resolve_index(int_arg(args.get(1)?)?, g.len())?;
            Some(M::Vector(g[r - 1].clone()))
        }
        id::COLUMN => {
            let g = as_matrix(args.first()?)?;
            let c = resolve_index(int_arg(args.get(1)?)?, g[0].len())?;
            Some(M::Vector(g.iter().map(|r| r[c - 1].clone()).collect()))
        }
        id::ELEMENT => element(args),
        id::TRANSPOSE => {
            let a = args.first()?;
            let g = as_matrix(a)?;
            Some(matrix_struct(transpose_grid(g)))
        }
        id::IDENTITY => {
            let a = args.first()?;
            let n = if a.is_vector() {
                if rows(a) != columns(a) {
                    return None;
                }
                rows(a)
            } else {
                usize::try_from(int_arg(a)?).ok()?
            };
            if n == 0 || n > 1000 {
                return None;
            }
            Some(matrix_struct(identity_grid(n)))
        }
        id::DETERMINANT => determinant(&evaluated_matrix(args.first()?)?),
        id::PERMANENT => permanent(&evaluated_matrix(args.first()?)?),
        id::COFACTOR => {
            let g = evaluated_matrix(args.first()?)?;
            let r = usize::try_from(int_arg(args.get(1)?)?).ok()?;
            let c = usize::try_from(int_arg(args.get(2)?)?).ok()?;
            cofactor(&g, r, c)
        }
        id::ADJOINT => Some(matrix_struct(adjoint(&evaluated_matrix(args.first()?)?)?)),
        id::INVERSE => {
            let a = args.first()?;
            if represents_scalar(a) {
                let mut r = a.clone();
                r.inverse();
                r.calculatesub(&eo());
                return Some(r);
            }
            Some(matrix_struct(invert(&evaluated_matrix(a)?)?))
        }
        id::RREF => {
            let mut g = evaluated_matrix(args.first()?)?;
            rref(&mut g).then(|| matrix_struct(g))
        }
        id::MATRIX_RANK => {
            let mut g = evaluated_matrix(args.first()?)?;
            if !rref(&mut g) {
                return None;
            }
            let n = g.iter().take_while(|r| r.iter().any(|e| !e.is_zero())).count();
            Some(usize_num(n))
        }
        id::MAGNITUDE => norm(args.first()?, &num(2)),
        id::NORM => norm(args.first()?, args.get(1).unwrap_or(&M::Undefined)),
        id::DOT_PRODUCT => {
            let a = as_vector(args.first()?);
            let b = as_vector(args.get(1)?);
            if a.len() != b.len() || a.is_empty() {
                return None;
            }
            let mut acc = multiplied(&a[0], &b[0]);
            for i in 1..a.len() {
                calc_add(&mut acc, multiplied(&a[i], &b[i]));
            }
            Some(acc)
        }
        id::CROSS => {
            let a = as_vector(args.first()?);
            let b = as_vector(args.get(1)?);
            if a.len() != 3 || b.len() != 3 {
                return None;
            }
            let comp = |i: usize, j: usize| {
                let mut r = multiplied(&a[i], &b[j]);
                calc_sub(&mut r, multiplied(&a[j], &b[i]));
                r
            };
            Some(M::Vector(vec![comp(1, 2), comp(2, 0), comp(0, 1)]))
        }
        id::HORZCAT => concat(args, false),
        id::VERTCAT => concat(args, true),
        id::AREA => area(args),
        id::LIMITS => limits(args),
        id::SORT => {
            let mut v = as_vector(args.first()?);
            let asc = bool_arg(args.get(1), true)?;
            sort_elements(&mut v, asc)?;
            Some(M::Vector(v))
        }
        id::RANK => rank(args),
        id::ENTRYWISE => entrywise(args),
        id::GENERATE_VECTOR => generate_vector(args),
        id::ENTRYWISE_MULTIPLICATION => entrywise_op(args, EntryOp::Multiply),
        id::ENTRYWISE_DIVISION => entrywise_op(args, EntryOp::Divide),
        id::ENTRYWISE_POWER => entrywise_op(args, EntryOp::Power),
        _ => None,
    }
}

/// The matrix argument of a function that evaluates its elements first
/// (`EVAL_MATRIX` in BuiltinFunctions-matrixvector.cc:772).
fn evaluated_matrix(m: &M) -> Option<Vec<Vec<M>>> {
    let mut g = as_matrix(m)?;
    for row in g.iter_mut() {
        for e in row.iter_mut() {
            eval_full(e);
        }
    }
    Some(g)
}

/// A `BooleanArgument` with a default.
fn bool_arg(m: Option<&M>, default: bool) -> Option<bool> {
    match m {
        None | Some(M::Undefined) => Some(default),
        Some(v) => Some(!v.number()?.is_zero()),
    }
}

/// Resolve a 1-based index that may count from the back (`-1` is the last).
fn resolve_index(i: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let r = if i < 0 { len + 1 + i } else { i };
    if r < 1 || r > len {
        None
    } else {
        Some(r as usize)
    }
}

fn build_matrix(args: &[M]) -> Option<M> {
    let r = usize::try_from(int_arg(args.first()?)?).ok()?;
    let c = usize::try_from(int_arg(args.get(1)?)?).ok()?;
    if r == 0 || c == 0 || r > 1000 || c > 1000 {
        return None;
    }
    let rest = &args[2..];
    let elements: Vec<M> = match rest {
        [M::Vector(v)] => v.clone(),
        other => other.to_vec(),
    };
    let mut g = vec![vec![num(0); c]; r];
    for (i, e) in elements.iter().enumerate() {
        if i >= r * c {
            break;
        }
        g[i / c][i % c] = e.clone();
    }
    Some(matrix_struct(g))
}

fn element(args: &[M]) -> Option<M> {
    let a = args.first()?;
    let row = int_arg(args.get(1)?)?;
    let col = args.get(2).and_then(int_arg).unwrap_or(0);
    if col == 0 {
        // One index: pick a row of a matrix, or an element of a vector.
        let v = a.children().cloned().collect::<Vec<_>>();
        if v.len() == 1 && v[0].is_vector() {
            let inner = v[0].children().cloned().collect::<Vec<_>>();
            let r = resolve_index(row, inner.len())?;
            return Some(inner[r - 1].clone());
        }
        let r = resolve_index(row, v.len())?;
        let picked = &v[r - 1];
        if picked.is_vector() && picked.size() == 1 {
            return picked.get(0).cloned();
        }
        return Some(picked.clone());
    }
    let g = as_matrix(a)?;
    let r = resolve_index(row, g.len())?;
    let c = resolve_index(col, g[0].len())?;
    Some(g[r - 1][c - 1].clone())
}

fn concat(args: &[M], vertical: bool) -> Option<M> {
    let mut acc = as_matrix(args.first()?)?;
    for a in &args[1..] {
        let g = as_matrix(a)?;
        if vertical {
            if g[0].len() != acc[0].len() {
                return None;
            }
            acc.extend(g);
        } else {
            if g.len() != acc.len() {
                return None;
            }
            for (dst, src) in acc.iter_mut().zip(g) {
                dst.extend(src);
            }
        }
    }
    Some(matrix_struct(acc))
}

/// `part(matrix, row1, column1, row2, column2)` — `AreaFunction`
/// (BuiltinFunctions-matrixvector.cc:577).
fn area(args: &[M]) -> Option<M> {
    let a = args.first()?;
    let g = as_matrix(a)?;
    // `rows()`/`columns()` of the argument itself: an empty vector has no
    // rows and no columns, where `as_matrix` gives it one empty row.
    let (nr, nc) = (rows(a) as i64, columns(a) as i64);
    let idx = |v: Option<&M>, default: i64, len: i64| -> Option<i64> {
        let raw = match v {
            None | Some(M::Undefined) => default,
            Some(x) => int_arg(x)?,
        };
        Some(match raw {
            0 => default,
            n if n < 0 => len + 1 + n,
            n => n,
        })
    };
    let r1 = idx(args.get(1), 1, nr)?;
    let c1 = idx(args.get(2), 1, nc)?;
    let r2 = idx(args.get(3), nr, nr)?;
    let c2 = idx(args.get(4), nc, nc)?;
    if r1 <= 0 || r2 <= 0 || c1 <= 0 || c2 <= 0 {
        return None;
    }
    let (flip_r, flip_c) = (r2 < r1, c2 < c1);
    let (r1, r2) = (r1.min(r2), r1.max(r2));
    let (c1, c2) = (c1.min(c2), c1.max(c2));
    // The result always has the requested shape: `resizeMatrix` pads it with
    // zeroes where the request runs off the grid, and a top-left corner that
    // is outside it altogether gives an all-zero area.
    let (want_r, want_c) = ((r2 - r1 + 1) as usize, (c2 - c1 + 1) as usize);
    if want_r.saturating_mul(want_c) > 1_000_000 {
        return None;
    }
    let mut out = vec![vec![num(0); want_c]; want_r];
    if r1 <= nr && c1 <= nc {
        for r in r1..=r2.min(nr) {
            for c in c1..=c2.min(nc) {
                out[(r - r1) as usize][(c - c1) as usize] =
                    g[r as usize - 1][c as usize - 1].clone();
            }
        }
    }
    if flip_r {
        out.reverse();
    }
    if flip_c {
        for row in out.iter_mut() {
            row.reverse();
        }
    }
    Some(matrix_struct(out))
}

/// `slice(vector, index1, index2)` — `LimitsFunction`
/// (BuiltinFunctions-matrixvector.cc:537).
fn limits(args: &[M]) -> Option<M> {
    let v = as_vector(args.first()?);
    let n = v.len() as i64;
    let conv = |raw: i64, default: i64| match raw {
        0 => default,
        x if x < 0 => n + 1 + x,
        x => x,
    };
    let i1 = conv(int_arg(args.get(1)?)?, 1);
    let i2 = conv(args.get(2).and_then(int_arg).unwrap_or(0), n);
    if i1 <= 0 || i2 <= 0 {
        return None;
    }
    let (lo, hi) = (i1.min(i2), i1.max(i2));
    let mut out: Vec<M> = ((lo as usize)..=(hi.min(n) as usize))
        .map(|i| v[i - 1].clone())
        .collect();
    // Indices past the end pad the result with zeroes.
    while (out.len() as i64) < hi - lo + 1 {
        out.push(num(0));
    }
    if i2 < i1 {
        out.reverse();
    }
    Some(M::Vector(out))
}

/// Comparison used by `sort()`/`rank()`: numeric order, or `None` when the
/// elements are not comparable.
fn cmp_elements(a: &M, b: &M) -> Option<std::cmp::Ordering> {
    let (x, y) = (a.number()?, b.number()?);
    if x.equals(y, false, false) {
        Some(std::cmp::Ordering::Equal)
    } else if x.is_less_than(y) {
        Some(std::cmp::Ordering::Less)
    } else if x.is_greater_than(y) {
        Some(std::cmp::Ordering::Greater)
    } else {
        None
    }
}

fn sort_elements(v: &mut [M], ascending: bool) -> Option<()> {
    for e in v.iter_mut() {
        eval_full(e);
    }
    // Insertion sort: stable, and it lets an incomparable pair abort.
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 {
            let ord = cmp_elements(&v[j - 1], &v[j])?;
            let swap = if ascending {
                ord == std::cmp::Ordering::Greater
            } else {
                ord == std::cmp::Ordering::Less
            };
            if !swap {
                break;
            }
            v.swap(j - 1, j);
            j -= 1;
        }
    }
    Some(())
}

/// `rankVector` (MathStructure-matrixvector.cc:69): replace each element by
/// its position in sorted order, averaging the positions of equal elements.
fn rank(args: &[M]) -> Option<M> {
    let a = args.first()?;
    let ascending = bool_arg(args.get(1), true)?;
    let flat = is_matrix(a);
    let g = if flat { as_matrix(a)? } else { vec![as_vector(a)] };
    let mut values: Vec<M> = g.iter().flatten().cloned().collect();
    for e in values.iter_mut() {
        eval_full(e);
    }
    let n = values.len();
    let mut order: Vec<usize> = (0..n).collect();
    // Selection sort over indices, so ties keep their original order.
    for i in 0..n {
        for j in (i + 1)..n {
            let ord = cmp_elements(&values[order[j]], &values[order[i]])?;
            let swap = if ascending {
                ord == std::cmp::Ordering::Less
            } else {
                ord == std::cmp::Ordering::Greater
            };
            if swap {
                order.swap(i, j);
            }
        }
    }
    let mut ranks = vec![num(0); n];
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && cmp_elements(&values[order[j - 1]], &values[order[j]])? == std::cmp::Ordering::Equal
        {
            j += 1;
        }
        // Positions i+1 .. j share the average rank.
        let mut avg = Number::from_i64((i + 1 + j) as i64);
        avg.divide(&Number::from_i64(2));
        for k in i..j {
            ranks[order[k]] = M::Number(avg.clone());
        }
        i = j;
    }
    if flat {
        // `resizeMatrix(rows, cols)` (BuiltinFunctions-matrixvector.cc:96):
        // a matrix of empty rows keeps its shape, and `chunks` cannot take a
        // zero size.
        let cols = g[0].len();
        let grid: Vec<Vec<M>> = if cols == 0 {
            vec![Vec::new(); g.len()]
        } else {
            ranks.chunks(cols).map(|c| c.to_vec()).collect()
        };
        Some(matrix_struct(grid))
    } else {
        Some(M::Vector(ranks))
    }
}

/// `magnitude()`/`norm()` — `(sum |x_i|^p)^(1/p)`
/// (BuiltinFunctions-matrixvector.cc:855).
fn norm(a: &M, p: &M) -> Option<M> {
    if is_matrix(a) {
        return None;
    }
    let v = as_vector(a);
    let p = if matches!(p, M::Undefined) { num(2) } else { p.clone() };
    if v.is_empty() {
        return Some(num(0));
    }
    if v.len() == 1 {
        // A one-element vector's norm is just |x|.
        let mut f = M::Function {
            id: FunctionId(crate::builtins::id::ABS),
            args: vec![v[0].clone()],
        };
        eval_full(&mut f);
        return Some(f);
    }
    let mut acc = num(0);
    for e in &v {
        let mut t = e.clone();
        calc_pow(&mut t, p.clone());
        calc_add(&mut acc, t);
    }
    // The C++ writes `sum^(1/p)`; go through sqrt()/root() instead, which is
    // where this port's numeric root evaluation lives.
    let mut out = match int_arg(&p) {
        Some(2) => M::Function {
            id: FunctionId(crate::builtins::id::SQRT),
            args: vec![acc],
        },
        Some(n) if n > 0 => M::Function {
            id: FunctionId(crate::builtins::id::ROOT),
            args: vec![acc, num(n)],
        },
        _ => {
            let mut inv = p;
            inv.inverse();
            inv.calculatesub(&eo());
            calc_pow(&mut acc, inv);
            acc
        }
    };
    eval_full(&mut out);
    Some(out)
}

/// `entrywise(f, v1, x1, v2, x2, …)`
/// (BuiltinFunctions-matrixvector.cc:1017).
fn entrywise(args: &[M]) -> Option<M> {
    let expr = args.first()?;
    let pairs: Vec<(&M, &M)> = args[1..]
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| (&c[0], &c[1]))
        .collect();
    if pairs.is_empty() {
        return Some(expr.clone());
    }
    let matrix_mode = is_matrix(pairs[0].0);
    let grids: Vec<Vec<Vec<M>>> = pairs
        .iter()
        .map(|(v, _)| {
            if matrix_mode {
                as_matrix(v)
            } else {
                Some(vec![as_vector(v)])
            }
        })
        .collect::<Option<_>>()?;
    let (nr, nc) = (grids[0].len(), grids[0][0].len());
    if grids.iter().any(|g| g.len() != nr || g[0].len() != nc) {
        return None;
    }
    let mut out = vec![vec![num(0); nc]; nr];
    for r in 0..nr {
        for c in 0..nc {
            let mut e = expr.clone();
            for (i, (_, sym)) in pairs.iter().enumerate() {
                replace(&mut e, sym, &grids[i][r][c]);
            }
            eval_full(&mut e);
            out[r][c] = e;
        }
    }
    Some(if matrix_mode {
        matrix_struct(out)
    } else {
        M::Vector(out.into_iter().next().expect("one row"))
    })
}

/// `genvector(f, min, max, step-or-count, variable, use-step)`
/// (BuiltinFunctions-matrixvector.cc:1571).
fn generate_vector(args: &[M]) -> Option<M> {
    let expr = args.first()?;
    let min = args.get(1)?.number()?.clone();
    let max = args.get(2)?.number()?.clone();
    let fourth = args.get(3).and_then(|m| m.number()).cloned();
    let var = match args.get(4) {
        None | Some(M::Undefined) => M::symbolic("x"),
        Some(v) => v.clone(),
    };
    let use_step = args.get(5).and_then(int_arg).unwrap_or(-1);

    let mut b_step = use_step > 0;
    if !b_step && use_step < 0 {
        // The default mode treats a non-integer, negative or unit fourth
        // argument as a step size and anything else as an element count.
        b_step = match &fourth {
            Some(n) => !n.is_integer() || n.is_negative() || n.is_one(),
            None => false,
        };
    }

    let mut xs: Vec<Number> = Vec::new();
    if b_step {
        let mut step = fourth?;
        if step.is_zero() {
            return None;
        }
        if max.is_less_than(&min) != step.is_negative() {
            step.negate();
        }
        let mut x = min.clone();
        let mut guard = 0;
        loop {
            if step.is_negative() {
                if x.is_less_than(&max) {
                    break;
                }
            } else if x.is_greater_than(&max) {
                break;
            }
            xs.push(x.clone());
            x.add(&step);
            guard += 1;
            if guard > 1_000_000 {
                return None;
            }
        }
    } else {
        let steps = fourth?.to_i64()?;
        if steps < 1 || steps > 1_000_000 {
            return None;
        }
        let mut step = max.clone();
        step.subtract(&min);
        if steps != 1 {
            step.divide(&Number::from_i64(steps - 1));
        }
        let mut x = min.clone();
        for i in 0..steps {
            if i == steps - 1 {
                xs.push(max.clone());
            } else {
                xs.push(x.clone());
                x.add(&step);
            }
        }
    }
    if xs.is_empty() {
        return None;
    }
    let items = xs
        .into_iter()
        .map(|x| {
            let mut e = expr.clone();
            replace(&mut e, &var, &M::Number(x));
            eval_full(&mut e);
            e
        })
        .collect();
    Some(M::Vector(items))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryOp {
    Multiply,
    Divide,
    Power,
}

impl EntryOp {
    fn apply(self, a: &M, b: &M) -> M {
        let mut r = a.clone();
        match self {
            EntryOp::Multiply => calc_mul(&mut r, b.clone()),
            EntryOp::Divide => calc_div(&mut r, b.clone()),
            EntryOp::Power => calc_pow(&mut r, b.clone()),
        }
        r
    }
}

/// `multiply()`/`divide()`/`pow()` and the `.*`, `./`, `.^` operators
/// (BuiltinFunctions-matrixvector.cc:1076).
fn entrywise_op(args: &[M], op: EntryOp) -> Option<M> {
    if args.is_empty() {
        return Some(num(0));
    }
    if args.len() == 1 {
        return Some(args[0].clone());
    }
    if args.len() > 2 {
        // Fold left, so `multiply([1 2], 3, 4)` = `[12  24]`.
        let mut acc = entrywise_op(&args[..2], op)?;
        for a in &args[2..] {
            acc = entrywise_op(&[acc, a.clone()], op)?;
        }
        return Some(acc);
    }
    let (a, b) = (&args[0], &args[1]);
    if op != EntryOp::Power && represents_scalar(a) && represents_scalar(b) {
        return Some(op.apply(a, b));
    }
    if represents_scalar(b) {
        let mut r = a.clone();
        if !r.is_vector() {
            return Some(op.apply(a, b));
        }
        map_elements(&mut r, &|e| *e = op.apply(e, b));
        return Some(r);
    }
    if represents_scalar(a) && b.is_vector() && op != EntryOp::Power {
        let mut r = b.clone();
        map_elements(&mut r, &|e| *e = op.apply(a, e));
        return Some(r);
    }
    if !a.is_vector() || !b.is_vector() {
        return None;
    }
    let (x, y) = broadcast(a, b, is_matrix(a), is_matrix(b))?;
    Some(zip_with(&x, &y, |p, q| op.apply(p, q)))
}

// ----------------------------------------------------------------------
// Print-time formatting
// ----------------------------------------------------------------------

/// `MathStructure::formatsub`'s `STRUCT_VECTOR` case
/// (MathStructure-print.cc:2947): a single-row matrix and a one-element
/// vector both print as their content.
pub fn format_for_print(m: &mut M, parent_is_matrix: bool) {
    let self_is_matrix = is_matrix(m);
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            format_for_print(child, self_is_matrix);
        }
    }
    if !m.is_vector() {
        return;
    }
    let take_first = |m: &mut M| -> M {
        match m {
            M::Vector(v) => v.remove(0),
            _ => unreachable!("checked is_vector"),
        }
    };
    if is_matrix(m) {
        if rows(m) == 1 {
            *m = take_first(m);
            format_for_print(m, parent_is_matrix);
        }
    } else if m.size() == 1 && !parent_is_matrix {
        *m = take_first(m);
    }
}

#[cfg(test)]
mod tests {
    use crate::eval::evaluate_to_string;

    fn ev(s: &str) -> String {
        evaluate_to_string(s).expect("evaluates")
    }

    #[test]
    fn matlab_matrix_literals() {
        assert_eq!(ev("[1 2; 4 5]"), "[1  2; 4  5]");
        assert_eq!(ev("[1 2 3]"), "[1  2  3]");
        assert_eq!(ev("[1]"), "1");
        assert_eq!(ev("[]"), "[]");
    }

    #[test]
    fn old_style_matrix_literals() {
        assert_eq!(ev("[[1, 2], [4, 5]]"), "[1  2; 4  5]");
        assert_eq!(ev("[[1]]"), "1");
    }

    #[test]
    fn parenthesised_and_bare_comma_lists() {
        assert_eq!(ev("(1,)"), "[1  0]");
        assert_eq!(ev("1,"), "[1  0]");
        assert_eq!(ev("(1;;2)"), "[1  0  2]");
        assert_eq!(ev("((1, 2), (4, 5))"), "[1  2; 4  5]");
    }

    #[test]
    fn ragged_vector_of_vectors_prints_as_tuple() {
        assert_eq!(ev("( 1; 2; 3, 4, 5, 6 ); (4; 5)"), "([1  2  3  4  5  6], [4  5])");
    }

    #[test]
    fn scalar_arithmetic_broadcasts() {
        assert_eq!(ev("(1; 2; 3) * 2 - 2"), "[0  2  4]");
        assert_eq!(ev("[1 2; 4 5] * 2"), "[2  4; 8  10]");
        assert_eq!(ev("[2 4 12] / 2"), "[1  2  6]");
    }

    #[test]
    fn elementwise_addition() {
        assert_eq!(ev("[1,2,3]+[4,5,6]"), "[5  7  9]");
    }

    #[test]
    fn matrix_multiplication() {
        assert_eq!(
            ev("((1; 2; 3); (4; 5; 6)) * ((7; 8); (9; 10); (11; 12))"),
            "[58  64; 139  154]"
        );
        assert_eq!(ev("[1 2; 3 4]^2"), "[7  10; 15  22]");
    }

    #[test]
    fn matrix_inverse_via_power() {
        assert_eq!(ev("((1; 2); (3; 4))^-1"), "[-2  1; 1.5  -0.5]");
        assert_eq!(ev("inverse([1 2; 3 5])"), "[-5  2; 3  -1]");
    }

    #[test]
    fn determinant_and_permanent() {
        assert_eq!(ev("det([1 2; 4 5])"), "-3");
        assert_eq!(ev("det([1 2 3; 4 5 6; 1 0 9])"), "-30");
        assert_eq!(ev("det([3 4 7 9; 5 4 -1 4; 8 7 8 5; 4 3 0 9])"), "-412");
        assert_eq!(ev("permanent([1 2; 4 5])"), "13");
        assert_eq!(ev("permanent([1 2 3; 4 5 6; 1 0 9])"), "144");
    }

    #[test]
    fn adjoint_and_cofactor() {
        assert_eq!(ev("adj([1 2; 4 5])"), "[5  -2; -4  1]");
        assert_eq!(ev("cofactor([1 2 3; 4 5 6; 1 0 9], 1, 2)"), "-30");
    }

    #[test]
    fn transpose_and_identity() {
        assert_eq!(ev("transpose([1 2; 3 4])"), "[1  3; 2  4]");
        assert_eq!(ev("[1 2 3; 4 5 6].'"), "[1  4; 2  5; 3  6]");
        assert_eq!(ev("identity(3)"), "[1  0  0; 0  1  0; 0  0  1]");
        assert_eq!(ev("identity([1 2; 4 5])"), "[1  0; 0  1]");
    }

    #[test]
    fn dimensions_and_access() {
        assert_eq!(ev("rows([1 2; 3 4])"), "2");
        assert_eq!(ev("columns([1 2; 4 5])"), "2");
        assert_eq!(ev("columns([])"), "0");
        assert_eq!(ev("elements([1 2; 3 4])"), "4");
        assert_eq!(ev("dimension([1 2 3 4])"), "4");
        assert_eq!(ev("element([1 2 3; 4 5 6], 2, 1)"), "4");
        assert_eq!(ev("row([1 2; 3 4], 2)"), "[3  4]");
        assert_eq!(ev("column([1 2; 3 4], 2)"), "[2  4]");
    }

    #[test]
    fn constructors() {
        assert_eq!(ev("vector(1, 2, 3)"), "[1  2  3]");
        assert_eq!(ev("vector()"), "[]");
        assert_eq!(ev("matrix(3, 1, [1 2])"), "[1; 2; 0]");
        assert_eq!(ev("matrix(3, 3, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)"), "[1  2  3; 4  5  6; 7  8  9]");
    }

    #[test]
    fn concatenation() {
        assert_eq!(ev("horzcat([1], [2 3], [4 5 6 7])"), "[1  2  3  4  5  6  7]");
        assert_eq!(ev("vertcat([1 2], [3 4], [5 6])"), "[1  2; 3  4; 5  6]");
        assert_eq!(ev("combine([1, 2], [3], [4, 5, 6])"), "[1  2  3  4  5  6]");
        assert_eq!(ev("matrix2vector([1 2; 4 5])"), "[1  2  4  5]");
    }

    #[test]
    fn products() {
        assert_eq!(ev("dot((1; 2; 3); (4; 5; 6))"), "32");
        assert_eq!(ev("(1; 2; 3).(4; 5; 6)"), "32");
        assert_eq!(ev("cross((1; 2; 3); (4; 5; 6))"), "[-3  6  -3]");
        assert_eq!(ev("[1 2].*[3 4]"), "[3  8]");
        assert_eq!(ev("[1; 2].*[3 4]"), "[3  4; 6  8]");
    }

    #[test]
    fn entrywise_power_and_division() {
        assert_eq!(ev("[1 2; 3 4].^2"), "[1  4; 9  16]");
        assert_eq!(ev("[2; 3].^[3 4]"), "[8  16; 27  81]");
        assert_eq!(ev("[2 4; 6 12]./[1 2; 3 4]"), "[2  2; 2  3]");
    }

    #[test]
    fn rank_sort_and_norm() {
        assert_eq!(ev("sort([5, 2, 0, 1, 3, -4, 0])"), "[-4  0  0  1  2  3  5]");
        assert_eq!(ev("sort([5, 2, 0, 1, 3, -4, 0], 0)"), "[5  3  2  1  0  0  -4]");
        assert_eq!(ev("rank([6, 7, 1, 4])"), "[3  4  1  2]");
        assert_eq!(ev("norm([2, 3, 6])"), "7");
    }

    #[test]
    fn rref_and_matrix_rank() {
        assert_eq!(ev("rref([1 3 1 9; 1 1 -1 1; 3 11 5 35])"), "[1  0  -2  -3; 0  1  1  4; 0  0  0  0]");
        assert_eq!(ev("rk([1 2 3; 3 6 9])"), "1");
        assert_eq!(ev("rk(identity(3))"), "3");
    }

    #[test]
    fn parts_and_slices() {
        assert_eq!(ev("part([1 2 3; 4 5 6; 7 8 9; 10 11 12], 1, 3, 2, 3)"), "[3; 6]");
        assert_eq!(ev("slice([5, 6, 7, 8, 9], 2, 4)"), "[6  7  8]");
    }

    /// `AreaFunction` returns exactly the requested shape: whatever falls
    /// outside the grid is filled with zeroes (`resizeMatrix`, with the
    /// "expanded with zeroes" notice in the C++), rather than clamped.
    #[test]
    fn part_expands_out_of_range_areas_with_zeroes() {
        // An empty vector has no columns at all, so the whole area is filled.
        assert_eq!(ev("part([], 1, 1, 1, 1)"), "0");
        assert_eq!(ev("part([[],[]], 1, 1, 1, 1)"), "0");
        assert_eq!(ev("part([[1,2],[3,4]], 5, 5, 6, 6)"), "[0  0; 0  0]");
        assert_eq!(ev("part([[1,2],[3,4]], 1, 1, 3, 3)"), "[1  2  0; 3  4  0; 0  0  0]");
        assert_eq!(ev("part([1,2,3], 2, 2)"), "[0  0; 2  3]");
        assert_eq!(ev("part(5, 1, 1, 2, 2)"), "[5  0; 0  0]");
        // A reversed range still flips the result.
        assert_eq!(ev("part([[1,2],[3,4]], 1, 2, 2, 1)"), "[2  1; 4  3]");
        assert_eq!(ev("part([1,2,3], 1, 3, 1, 1)"), "[3  2  1]");
    }

    /// A matrix of empty rows keeps its shape through `rank()`, where
    /// splitting the (empty) rank vector into rows of 0 columns would ask
    /// `chunks` for a zero-sized chunk.
    #[test]
    fn rank_of_a_matrix_without_columns() {
        assert_eq!(ev("rank([[],[]])"), "([], [])");
        assert_eq!(ev("rank([[],[]], 0)"), "([], [])");
        assert_eq!(ev("rank([[1,2],[3,4]])"), "[1  2; 3  4]");
    }

    #[test]
    fn generated_and_entrywise_vectors() {
        assert_eq!(ev("genvector(x+10, 1, 2, 3)"), "[11  11.5  12]");
        assert_eq!(ev("genvector(x+100, -3, 5, 2, x, 1)"), "[97  99  101  103  105]");
        assert_eq!(ev("entrywise(x / y, [4 10 12], x, [2 2 4], y)"), "[2  5  3]");
    }
}
