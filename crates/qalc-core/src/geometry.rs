//! Geometry builtins — area/volume/perimeter helpers.
//!
//! In libqalculate these are *user functions*: they have no C++ class, only
//! an `<expression>` in `data/functions.xml.in` (category "Geometry", plus
//! the Circle/Cylinder/Cone/Sphere/Square/Cube/Rectangle/Prism/Pyramid/
//! Parallelogram/Trapezoid subcategories). This module evaluates the same
//! formulas numerically; each entry below quotes the XML expression it
//! ports so the two stay comparable.
//!
//! Every function takes plain numbers, so the module is a small numeric
//! dispatcher hung off [`crate::builtins::calculate_function`] in the same
//! way [`crate::matrix`] hangs off it for vector arguments.

use crate::ids::FunctionId;
use crate::structure::MathStructure;
use qalc_num::Number;

/// Ids for the geometry user functions. These have no `FUNCTION_ID_*` in
/// `BuiltinFunctions.h` (they are XML definitions), so the port allocates a
/// private block that does not overlap the C++ ranges.
pub mod id {
    pub const CIRCLE: u32 = 2900;
    pub const CIRCUMFERENCE: u32 = 2901;
    pub const CONE: u32 = 2902;
    pub const CONE_SA: u32 = 2903;
    pub const CUBE: u32 = 2904;
    pub const CUBE_SA: u32 = 2905;
    pub const CYLINDER: u32 = 2906;
    pub const CYLINDER_SA: u32 = 2907;
    pub const PARALLELOGRAM: u32 = 2908;
    pub const PARALLELOGRAM_PERIMETER: u32 = 2909;
    pub const RECTPRISM: u32 = 2910;
    pub const RECTPRISM_SA: u32 = 2911;
    pub const TRIANGLEPRISM: u32 = 2912;
    pub const TETRAHEDRON: u32 = 2913;
    pub const TETRAHEDRON_HEIGHT: u32 = 2914;
    pub const TETRAHEDRON_SA: u32 = 2915;
    pub const SQPYRAMID: u32 = 2916;
    pub const SQPYRAMID_HEIGHT: u32 = 2917;
    pub const SQPYRAMID_SA: u32 = 2918;
    pub const PYRAMID: u32 = 2919;
    pub const RECT: u32 = 2920;
    pub const RECT_PERIMETER: u32 = 2921;
    pub const SPHERE: u32 = 2922;
    pub const SPHERE_SA: u32 = 2923;
    pub const SQUARE: u32 = 2924;
    pub const SQUARE_PERIMETER: u32 = 2925;
    pub const TRAPEZOID: u32 = 2926;
    pub const TRIANGLE: u32 = 2927;
    pub const TRIANGLE_PERIMETER: u32 = 2928;
    pub const HYPOT: u32 = 2929;
}

/// Resolve a geometry function name to its id.
pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let v = match name {
        "circle" => id::CIRCLE,
        "circumference" => id::CIRCUMFERENCE,
        "cone" => id::CONE,
        "cone_sa" => id::CONE_SA,
        "cube" => id::CUBE,
        "cube_sa" => id::CUBE_SA,
        "cylinder" => id::CYLINDER,
        "cylinder_sa" => id::CYLINDER_SA,
        "parallelogram" => id::PARALLELOGRAM,
        "parallelogram_perimeter" => id::PARALLELOGRAM_PERIMETER,
        "rectprism" => id::RECTPRISM,
        "rectprism_sa" => id::RECTPRISM_SA,
        "triangleprism" => id::TRIANGLEPRISM,
        "tetrahedron" => id::TETRAHEDRON,
        "tetrahedron_height" => id::TETRAHEDRON_HEIGHT,
        "tetrahedron_sa" => id::TETRAHEDRON_SA,
        "sqpyramid" => id::SQPYRAMID,
        "sqpyramid_height" => id::SQPYRAMID_HEIGHT,
        "sqpyramid_sa" => id::SQPYRAMID_SA,
        "pyramid" => id::PYRAMID,
        "rect" => id::RECT,
        "rect_perimeter" => id::RECT_PERIMETER,
        "sphere" => id::SPHERE,
        "sphere_sa" => id::SPHERE_SA,
        "square" => id::SQUARE,
        "square_perimeter" => id::SQUARE_PERIMETER,
        "trapezoid" => id::TRAPEZOID,
        "triangle" => id::TRIANGLE,
        "triangle_perimeter" => id::TRIANGLE_PERIMETER,
        "hypot" => id::HYPOT,
        _ => return None,
    };
    Some(FunctionId(v))
}

/// Display names, used when a call cannot be evaluated.
pub fn function_name(fid: u32) -> Option<&'static str> {
    Some(match fid {
        id::CIRCLE => "circle",
        id::CIRCUMFERENCE => "circumference",
        id::CONE => "cone",
        id::CONE_SA => "cone_sa",
        id::CUBE => "cube",
        id::CUBE_SA => "cube_sa",
        id::CYLINDER => "cylinder",
        id::CYLINDER_SA => "cylinder_sa",
        id::PARALLELOGRAM => "parallelogram",
        id::PARALLELOGRAM_PERIMETER => "parallelogram_perimeter",
        id::RECTPRISM => "rectprism",
        id::RECTPRISM_SA => "rectprism_sa",
        id::TRIANGLEPRISM => "triangleprism",
        id::TETRAHEDRON => "tetrahedron",
        id::TETRAHEDRON_HEIGHT => "tetrahedron_height",
        id::TETRAHEDRON_SA => "tetrahedron_sa",
        id::SQPYRAMID => "sqpyramid",
        id::SQPYRAMID_HEIGHT => "sqpyramid_height",
        id::SQPYRAMID_SA => "sqpyramid_sa",
        id::PYRAMID => "pyramid",
        id::RECT => "rect",
        id::RECT_PERIMETER => "rect_perimeter",
        id::SPHERE => "sphere",
        id::SPHERE_SA => "sphere_sa",
        id::SQUARE => "square",
        id::SQUARE_PERIMETER => "square_perimeter",
        id::TRAPEZOID => "trapezoid",
        id::TRIANGLE => "triangle",
        id::TRIANGLE_PERIMETER => "triangle_perimeter",
        id::HYPOT => "hypot",
        _ => return None,
    })
}

/// True if `fid` belongs to this module.
pub fn owns(fid: u32) -> bool {
    function_name(fid).is_some()
}

// ----------------------------------------------------------------------
// Small numeric helpers (the `Number` API mutates and reports success)
// ----------------------------------------------------------------------

fn n(i: i64) -> Number {
    Number::from_i64(i)
}

fn pi() -> Number {
    let mut p = Number::new();
    p.pi();
    p
}

/// `a^e` for a small integer exponent.
fn powi(a: &Number, e: i64) -> Option<Number> {
    let mut r = a.clone();
    r.raise(&n(e), true).then_some(r)
}

fn mul(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.multiply(b).then_some(r)
}

fn add(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.add(b).then_some(r)
}

fn div(a: &Number, b: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.divide(b).then_some(r)
}

fn sqrt_of(a: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.raise(&Number::from_ints(1, 2, 0), true).then_some(r)
}

/// `sqrt(k)` for a small integer `k`.
fn sqrt_i(k: i64) -> Option<Number> {
    sqrt_of(&n(k))
}

fn abs_of(a: &Number) -> Option<Number> {
    let mut r = a.clone();
    r.abs().then_some(r)
}

/// Evaluate a geometry call over numeric arguments.
pub fn apply(fid: u32, a: &[Number]) -> Option<Number> {
    match (fid, a.len()) {
        // <expression>\x^2*pi</expression        // <expression>\x^2*pi</expression>
        (id::CIRCLE, 1) => {
            let r = abs_of(&a[0])?;
            mul(&powi(&r, 2)?, &pi())
        }
        // <expression>\x*2*pi</expression>
        (id::CIRCUMFERENCE, 1) => {
            let r = abs_of(&a[0])?;
            mul(&mul(&r, &n(2))?, &pi())
        }
        // <expression>\x^2*pi*\y/3</expression>
        (id::CONE, 2) => {
            let r = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            let t = mul(&mul(&powi(&r, 2)?, &pi())?, &h)?;
            div(&t, &n(3))
        }
        // <expression>\x^2*pi+pi*\x*abs((\y^2+\x^2)^(1/2))</expression>
        (id::CONE_SA, 2) => {
            let r = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            let base = mul(&powi(&r, 2)?, &pi())?;
            let slant = abs_of(&sqrt_of(&add(&powi(&h, 2)?, &powi(&r, 2)?)?)?)?;
            add(&base, &mul(&mul(&pi(), &r)?, &slant)?)
        }
        // <expression>\x^3</expression>
        (id::CUBE, 1) => {
            let s = abs_of(&a[0])?;
            powi(&s, 3)
        }
        // <expression>(\x^2)*6</expression>
        (id::CUBE_SA, 1) => {
            let s = abs_of(&a[0])?;
            mul(&powi(&s, 2)?, &n(6))
        }
        // <expression>\x^2*pi*\y</expression>
        (id::CYLINDER, 2) => {
            let r = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            mul(&mul(&powi(&r, 2)?, &pi())?, &h)
        }
        // <expression>2*\x^2*pi+2*pi*\x*\y</expression>
        (id::CYLINDER_SA, 2) => {
            let r = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            let ends = mul(&mul(&n(2), &powi(&r, 2)?)?, &pi())?;
            let side = mul(&mul(&mul(&n(2), &pi())?, &r)?, &h)?;
            add(&ends, &side)
        }
        // <expression>\x*\y</expression>
        (id::PARALLELOGRAM, 2) | (id::RECT, 2) => {
            let w = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            mul(&w, &h)
        }
        // <expression>(\x+\y)*2</expression>
        (id::PARALLELOGRAM_PERIMETER, 2) | (id::RECT_PERIMETER, 2) => {
            let w = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            mul(&add(&w, &h)?, &n(2))
        }
        // <expression>\x*\y*\z</expression>
        (id::RECTPRISM, 3) => {
            let l = abs_of(&a[0])?;
            let w = abs_of(&a[1])?;
            let h = abs_of(&a[2])?;
            mul(&mul(&l, &w)?, &h)
        }
        // <expression>(\x*\y)*2+(\x*\z)*2+(\y*\z)*2</expression>
        (id::RECTPRISM_SA, 3) => {
            let l = abs_of(&a[0])?;
            let w = abs_of(&a[1])?;
            let h = abs_of(&a[2])?;
            let s = add(
                &add(&mul(&l, &w)?, &mul(&l, &h)?)?,
                &mul(&w, &h)?,
            )?;
            mul(&s, &n(2))
        }
        // <expression>\x*\y*\z/2</expression>
        (id::TRIANGLEPRISM, 3) => {
            let b = abs_of(&a[0])?;
            let h = abs_of(&a[1])?;
            let l = abs_of(&a[2])?;
            div(&mul(&mul(&b, &h)?, &l)?, &n(2))
        }
        // <expression>sqrt(2)/12*\x^3</expression>
        (id::TETRAHEDRON, 1) => {
            let a_s = abs_of(&a[0])?;
            mul(&div(&sqrt_i(2)?, &n(12))?, &powi(&a_s, 3)?)
        }
        // <expression>sqrt(6)/3*\x</expression>
        (id::TETRAHEDRON_HEIGHT, 1) => {
            let a_s = abs_of(&a[0])?;
            mul(&div(&sqrt_i(6)?, &n(3))?, &a_s)
        }
        // <expression>sqrt(3)*\x^2</expression>
        (id::TETRAHEDRON_SA, 1) => {
            let a_s = abs_of(&a[0])?;
            mul(&sqrt_i(3)?, &powi(&a_s, 2)?)
        }
        // <expression>sqrt(2)/6*\x^3</expression>
        (id::SQPYRAMID, 1) => {
            let a_s = abs_of(&a[0])?;
            mul(&div(&sqrt_i(2)?, &n(6))?, &powi(&a_s, 3)?)
        }
        // <expression>sqrt(2)/2*\x</expression>
        (id::SQPYRAMID_HEIGHT, 1) => {
            let a_s = abs_of(&a[0])?;
            mul(&div(&sqrt_i(2)?, &n(2))?, &a_s)
        }
        // <expression>(1+sqrt(3))*\x^2</expression>
        (id::SQPYRAMID_SA, 1) => {
            let a_s = abs_of(&a[0])?;
            mul(&add(&n(1), &sqrt_i(3)?)?, &powi(&a_s, 2)?)
        }
        // <expression>\x*\y*\z/3</expression>
        (id::PYRAMID, 3) => {
            let l = abs_of(&a[0])?;
            let w = abs_of(&a[1])?;
            let h = abs_of(&a[2])?;
            div(&mul(&mul(&l, &w)?, &h)?, &n(3))
        }
        // <expression>\x^3*pi*4/3</expression>
        (id::SPHERE, 1) => {
            let r = abs_of(&a[0])?;
            let t = mul(&mul(&powi(&r, 3)?, &pi())?, &n(4))?;
            div(&t, &n(3))
        }
        // <expression>\x^2*pi*4</expression>
        (id::SPHERE_SA, 1) => {
            let r = abs_of(&a[0])?;
            mul(&mul(&powi(&r, 2)?, &pi())?, &n(4))
        }
        // <expression>\x^2</expression>
        (id::SQUARE, 1) => powi(&a[0], 2),
        // <expression>\x*4</expression>
        (id::SQUARE_PERIMETER, 1) => mul(&a[0], &n(4)),
        // <expression>(\x+\y)/2*\z</expression>
        (id::TRAPEZOID, 3) => mul(&div(&add(&a[0], &a[1])?, &n(2))?, &a[2]),
        // <expression>(\x*\y)/2</expression>
        (id::TRIANGLE, 2) => div(&mul(&a[0], &a[1])?, &n(2)),
        // <expression>\x+\y+\z</expression>
        (id::TRIANGLE_PERIMETER, 3) => add(&add(&a[0], &a[1])?, &a[2]),
        // <expression>sqrt(\x^2+\y^2)</expression>
        (id::HYPOT, 2) => sqrt_of(&add(&powi(&a[0], 2)?, &powi(&a[1], 2)?)?),
        _ => None,
    }
}

/// Evaluate a geometry call in place. Returns true when it was replaced.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id: fid, args } = m else {
        return false;
    };
    let fid = fid.0;
    if !owns(fid) {
        return false;
    }
    let mut nums: Vec<Number> = Vec::with_capacity(args.len());
    for arg in args.iter() {
        match arg {
            MathStructure::Number(v) => nums.push(v.clone()),
            _ => return false,
        }
    }
    match apply(fid, &nums) {
        Some(r) => {
            *m = MathStructure::Number(r);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::evaluate_to_string;

    /// Every expectation here comes from
    /// `tests/geometry.batch` / the reference binary (`qalc -t +u8`).
    fn ev(s: &str) -> String {
        evaluate_to_string(s).expect("evaluates")
    }

    #[test]
    fn circle_and_circumference() {
        assert_eq!(ev("circle(3)"), "28.27433388");
        assert_eq!(ev("circumference(3)"), "18.84955592");
    }

    #[test]
    fn cone_volume_and_surface() {
        assert_eq!(ev("cone(3, 4)"), "37.69911184");
        assert_eq!(ev("cone_sa(3, 4)"), "75.39822369");
    }

    #[test]
    fn cube_volume_and_surface_are_exact() {
        assert_eq!(ev("cube(3)"), "27");
        assert_eq!(ev("cube_sa(3)"), "54");
    }

    #[test]
    fn cylinder_volume_and_surface() {
        assert_eq!(ev("cylinder(3, 4)"), "113.0973355");
        assert_eq!(ev("cylinder_sa(3, 4)"), "131.9468915");
    }

    #[test]
    fn parallelogram() {
        assert_eq!(ev("parallelogram(3, 4)"), "12");
        assert_eq!(ev("parallelogram_perimeter(3,4)"), "14");
    }

    #[test]
    fn rectangular_prism() {
        assert_eq!(ev("rectprism(3, 4, 5)"), "60");
        assert_eq!(ev("rectprism_sa(3, 4, 5)"), "94");
    }

    #[test]
    fn triangular_prism() {
        assert_eq!(ev("triangleprism(3, 4, 5)"), "30");
    }

    #[test]
    fn tetrahedron_family() {
        assert_eq!(ev("tetrahedron(3)"), "3.181980515");
        assert_eq!(ev("tetrahedron_height(3)"), "2.449489743");
        assert_eq!(ev("tetrahedron_sa(3)"), "15.58845727");
    }

    #[test]
    fn square_pyramid_family() {
        assert_eq!(ev("sqpyramid(3)"), "6.363961031");
        assert_eq!(ev("sqpyramid_height(3)"), "2.121320344");
        assert_eq!(ev("sqpyramid_sa(3)"), "24.58845727");
    }

    #[test]
    fn pyramid_volume() {
        assert_eq!(ev("pyramid(3, 4, 5)"), "20");
    }

    #[test]
    fn rectangle() {
        assert_eq!(ev("rect(3, 4)"), "12");
        assert_eq!(ev("rect_perimeter(3, 4)"), "14");
    }

    #[test]
    fn sphere_volume_and_surface() {
        assert_eq!(ev("sphere(4)"), "268.0825731");
        assert_eq!(ev("sphere_sa(4)"), "201.0619298");
    }

    #[test]
    fn square_area_and_perimeter() {
        assert_eq!(ev("square(3)"), "9");
        assert_eq!(ev("square_perimeter(3)"), "12");
    }

    #[test]
    fn trapezoid_area() {
        assert_eq!(ev("trapezoid(3, 4, 5)"), "17.5");
    }

    #[test]
    fn triangle_family_and_hypotenuse() {
        assert_eq!(ev("triangle(3, 4)"), "6");
        assert_eq!(ev("triangle_perimeter(3, 4, 5)"), "12");
        assert_eq!(ev("hypot(3, 4)"), "5");
    }

    #[test]
    fn wrong_arity_leaves_the_call_alone() {
        // The C++ convention: a call that cannot be evaluated stays put.
        let s = ev("circle(1, 2)");
        assert!(s.contains("circle"), "got {s}");
    }
}
