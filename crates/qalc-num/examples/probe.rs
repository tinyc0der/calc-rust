//! Scratch probe: confirm astro-float covers the MPFR surface Number.cc needs.

use astro_float::{BigFloat, Consts, RoundingMode, Sign};

fn main() {
    let p = 128; // bits, like mpfr precision
    let mut cc = Consts::new().unwrap();

    // Directed rounding for interval arithmetic (mpfr RNDD/RNDU equivalents)
    let a = BigFloat::from_f64(1.0, p);
    let three = BigFloat::from_f64(3.0, p);
    let down = a.div(&three, p, RoundingMode::Down);
    let up = a.div(&three, p, RoundingMode::Up);
    println!("1/3 down = {:?}", down.format(astro_float::Radix::Dec, RoundingMode::None, &mut cc));
    println!("1/3 up   = {:?}", up.format(astro_float::Radix::Dec, RoundingMode::None, &mut cc));
    assert!(down.cmp(&up) == Some(-1));

    // Transcendentals + constants
    let pi = cc.pi(p, RoundingMode::ToEven);
    let s = pi.sin(p, RoundingMode::ToEven, &mut cc);
    println!("sin(pi) = {:?}", s.format(astro_float::Radix::Dec, RoundingMode::None, &mut cc));
    let e = cc.e(p, RoundingMode::ToEven);
    println!("e = {:?}", e.format(astro_float::Radix::Dec, RoundingMode::None, &mut cc));
    let l = BigFloat::from_f64(10.0, p).ln(p, RoundingMode::ToEven, &mut cc);
    println!("ln(10) = {:?}", l.format(astro_float::Radix::Dec, RoundingMode::None, &mut cc));

    // Mantissa/exponent access for conversion to/from bigint rationals
    let x = BigFloat::from_f64(6.25, p);
    println!("mantissa digits = {:?}, exp = {:?}, sign = {:?}",
        x.mantissa_digits(), x.exponent(), x.sign());

    // Special values
    let inf = BigFloat::from_f64(f64::INFINITY, p);
    println!("inf: is_inf={} sign={:?}", inf.is_inf(), inf.sign());
    let nan = BigFloat::nan(None);
    println!("nan: is_nan={}", nan.is_nan());
    let _ = Sign::Pos;
}
