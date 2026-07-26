//! Solar and lunar astronomy — the port of the *Calendrical Calculations*
//! routines at the bottom of `QalculateDateTime.cc` (lines 1395-2790).
//!
//! Only what `lunarphase` and `nextlunarphase` need is ported: the Gregorian
//! fixed-day conversions, `ephemeris_correction`, `solar_longitude`,
//! `lunar_longitude`, `nth_new_moon`, and the two phase entry points.
//! `solarLongitude` / `findNextSolarLongitude` and the non-Gregorian calendars
//! are still absent (see the crate-level TODO).
//!
//! The C++ wraps every one of these in
//! `beginTemporaryStopIntervalArithmetic`, so [`lunar_phase_of`] and
//! [`next_lunar_phase`] do the same: an interval would widen without bound
//! through the bisection and the several hundred sines each evaluation costs.
//!
//! Where the C++ writes `setFloat(0.9287892L)` this parses the same decimal
//! digits into an exact rational. That is *more* precise than the original
//! long double, not less, and the values stay small because every series term
//! ends in a `sin` that returns a float.

use crate::{days_per_month, is_leap_year, QalculateDateTime};
use qalc_num::Number;

/// `MEAN_SYNODIC_MONTH`
fn mean_synodic_month() -> Number {
    dec("29.530588861")
}

/// `J2000` — the fixed-day number of noon, 2000-01-01 TT.
fn j2000() -> Number {
    dec("730120.5")
}

/// A literal, exactly. Stands in for the C++ `setFloat(...L)`.
///
/// `a/b` and `a*b` are accepted so a coefficient the C++ writes as
/// `1.0L/538841.0L` or `29.530588861L * 1236.85L` can be transcribed verbatim
/// rather than pre-multiplied by hand.
fn dec(s: &str) -> Number {
    let po = qalc_num::ParseOptions::default();
    if let Some((a, b)) = s.split_once('/') {
        let mut n = Number::parse(a.trim(), &po);
        n.divide(&Number::parse(b.trim(), &po));
        return n;
    }
    if let Some((a, b)) = s.split_once('*') {
        let mut n = Number::parse(a.trim(), &po);
        n.multiply(&Number::parse(b.trim(), &po));
        return n;
    }
    Number::parse(s, &po)
}

fn int(i: i64) -> Number {
    Number::from_i64(i)
}

// ----------------------------------------------------------------------
// Small arithmetic helpers, spelled as the C++ does
// ----------------------------------------------------------------------

fn add(a: &Number, b: &Number) -> Number {
    let mut r = a.clone();
    r.add(b);
    r
}

fn sub(a: &Number, b: &Number) -> Number {
    let mut r = a.clone();
    r.subtract(b);
    r
}

fn mul(a: &Number, b: &Number) -> Number {
    let mut r = a.clone();
    r.multiply(b);
    r
}

fn div_i64(a: &Number, d: i64) -> Number {
    let mut r = a.clone();
    r.divide_i64(d);
    r
}

/// `quotient(nr, d)` — floored division.
fn quotient(a: &Number, d: i64) -> Number {
    let mut r = div_i64(a, d);
    r.floor();
    r
}

/// `nr.mod(d)` — the C++ `Number::mod` is a *floored* remainder, so the result
/// takes the sign of the divisor.
fn modulo(a: &Number, d: &Number) -> Number {
    let mut r = a.clone();
    r.mod_floor(d);
    r
}

fn modulo_i64(a: &Number, d: i64) -> Number {
    modulo(a, &int(d))
}

/// `sin` of an argument given in degrees.
fn sin_degrees(deg: &Number) -> Number {
    let mut r = mul(deg, &pi());
    r.divide_i64(180);
    r.sin();
    r
}

fn pi() -> Number {
    let mut p = Number::new();
    p.pi();
    p
}

/// `cal_poly(c, n, a0, a1, ...)` — Horner in the other direction, term by
/// term, exactly as the variadic C++ does it.
fn cal_poly(c: &Number, coefficients: &[&str]) -> Number {
    let mut x = int(1);
    let mut poly = Number::new();
    for a in coefficients {
        poly.add(&mul(&dec(a), &x));
        x = mul(&x, c);
    }
    poly
}

// ----------------------------------------------------------------------
// Gregorian fixed-day numbers
// ----------------------------------------------------------------------

/// `date_to_fixed(y, m, d, CALENDAR_GREGORIAN)`.
pub fn date_to_fixed(y: i64, m: i64, d: i64) -> Number {
    let year = int(y - 1);
    let mut fixed = mul(&year, &int(365));
    fixed.add(&quotient(&year, 4));
    fixed.subtract(&quotient(&year, 100));
    fixed.add(&quotient(&year, 400));
    fixed.add(&quotient(&int(367 * m - 362), 12));
    if m > 2 {
        fixed.subtract(&int(if is_leap_year(y) { 1 } else { 2 }));
    }
    fixed.add(&int(d));
    fixed
}

/// `gregorian_year_from_fixed(date)`.
pub fn gregorian_year_from_fixed(date: &Number) -> i64 {
    let d0 = sub(date, &int(1));
    let n400 = quotient(&d0, 146097);
    let d1 = modulo_i64(&d0, 146097);
    let n100 = quotient(&d1, 36524);
    let d2 = modulo_i64(&d1, 36524);
    let n4 = quotient(&d2, 1461);
    let d3 = modulo_i64(&d2, 1461);
    let n1 = quotient(&d3, 365);
    let mut year = if !n100.equals_i64(4) && !n1.equals_i64(4) {
        int(1)
    } else {
        Number::new()
    };
    year.add(&mul(&n400, &int(400)));
    year.add(&mul(&n100, &int(100)));
    year.add(&mul(&n4, &int(4)));
    year.add(&n1);
    year.to_i64().unwrap_or(0)
}

/// `fixed_to_date(date, y, m, d, CALENDAR_GREGORIAN)`.
pub fn fixed_to_date(date: &Number) -> Option<(i64, i64, i64)> {
    let y = gregorian_year_from_fixed(date);
    let mut prior_days = sub(date, &date_to_fixed(y, 1, 1));
    if !date.is_less_than(&date_to_fixed(y, 3, 1)) {
        prior_days.add(&int(if is_leap_year(y) { 1 } else { 2 }));
    }
    prior_days.multiply(&int(12));
    prior_days.add(&int(373));
    let m = quotient(&prior_days, 367).to_i64()?;
    let mut day = sub(date, &date_to_fixed(y, m, 1));
    day.add(&int(1));
    // `date` carries the fraction of a day when it comes from the phase
    // solver, so the day number is the floor; the caller adds the remaining
    // fraction back with `add_days`.
    day.floor();
    let d = day.to_i64()?;
    if !(1..=12).contains(&m) || d < 1 || d > days_per_month(m, y) {
        return None;
    }
    Some((y, m, d))
}

/// `gregorian_date_difference`.
fn gregorian_date_difference(
    y1: i64,
    m1: i64,
    d1: i64,
    y2: i64,
    m2: i64,
    d2: i64,
) -> Number {
    sub(&date_to_fixed(y2, m2, d2), &date_to_fixed(y1, m1, d1))
}

// ----------------------------------------------------------------------
// Time scales
// ----------------------------------------------------------------------

/// `ephemeris_correction(tee)` — TT minus UT, in days.
fn ephemeris_correction(tee: &Number) -> Number {
    let mut floored = tee.clone();
    floored.floor();
    let year = gregorian_year_from_fixed(&floored);
    let seconds_per_day = 86400;

    if !(-500..=2150).contains(&year) {
        let y = div_i64(&int(year - 1820), 100);
        let mut a = int(-20);
        a.add(&mul(&int(32), &mul(&y, &y)));
        return div_i64(&a, seconds_per_day);
    }
    if year < 500 {
        let y = div_i64(&int(year), 100);
        return div_i64(
            &cal_poly(
                &y,
                &[
                    "10583.6",
                    "-1014.41",
                    "33.78311",
                    "-5.952053",
                    "-0.1798452",
                    "0.022174192",
                    "0.0090316521",
                ],
            ),
            seconds_per_day,
        );
    }
    if year < 1600 {
        let y = div_i64(&int(year - 1000), 100);
        return div_i64(
            &cal_poly(
                &y,
                &[
                    "1574.2",
                    "-556.01",
                    "71.23472",
                    "0.319781",
                    "-0.8503463",
                    "-0.005050998",
                    "0.0083572073",
                ],
            ),
            seconds_per_day,
        );
    }
    if year < 1700 {
        let y = int(year - 1600);
        return div_i64(
            &cal_poly(&y, &["120", "-0.9808", "-0.01532", "0.000140272128"]),
            seconds_per_day,
        );
    }
    if year < 1800 {
        let y = int(year - 1700);
        return div_i64(
            &cal_poly(
                &y,
                &["8.118780842", "-0.005092142", "0.003336121", "-0.0000266484"],
            ),
            seconds_per_day,
        );
    }
    if year < 1900 {
        // Already in days: this branch of the C++ does not divide by 86400.
        let c = div_i64(&gregorian_date_difference(1900, 1, 1, year, 7, 1), 36525);
        return cal_poly(
            &c,
            &[
                "-0.000009",
                "0.003844",
                "0.083563",
                "0.865736",
                "4.867575",
                "15.845535",
                "31.332267",
                "38.291999",
                "28.316289",
                "11.636204",
                "2.043794",
            ],
        );
    }
    if year < 1987 {
        let c = div_i64(&gregorian_date_difference(1900, 1, 1, year, 7, 1), 36525);
        return cal_poly(
            &c,
            &[
                "-0.00002",
                "0.000297",
                "0.025184",
                "-0.181133",
                "0.553040",
                "-0.861938",
                "0.677066",
                "-0.212591",
            ],
        );
    }
    if year < 2006 {
        let y = int(year - 2000);
        return div_i64(
            &cal_poly(
                &y,
                &[
                    "63.86",
                    "0.3345",
                    "-0.060374",
                    "0.0017275",
                    "0.000651814",
                    "0.00002373599",
                ],
            ),
            seconds_per_day,
        );
    }
    let y = int(year - 2000);
    div_i64(
        &cal_poly(&y, &["62.92", "0.32217", "0.005589"]),
        seconds_per_day,
    )
}

fn dynamical_from_universal(tee: &Number) -> Number {
    add(tee, &ephemeris_correction(tee))
}

fn universal_from_dynamical(tee: &Number) -> Number {
    sub(tee, &ephemeris_correction(tee))
}

/// `julian_centuries(tee)`.
fn julian_centuries(tee: &Number) -> Number {
    let mut c = dynamical_from_universal(tee);
    c.subtract(&j2000());
    c.divide_i64(36525);
    c
}

// ----------------------------------------------------------------------
// Solar position
// ----------------------------------------------------------------------

fn nutation(tee: &Number) -> Number {
    let c = julian_centuries(tee);
    let cap_a = cal_poly(&c, &["124.90", "-1934.134", "0.002063"]);
    let cap_b = cal_poly(&c, &["201.11", "72001.5377", "0.00057"]);
    let mut r = mul(&sin_degrees(&cap_a), &dec("-0.004778"));
    r.add(&mul(&sin_degrees(&cap_b), &dec("-0.0003667")));
    r
}

fn aberration(tee: &Number) -> Number {
    let mut c = mul(&julian_centuries(tee), &dec("35999.01848"));
    c.add(&dec("177.63"));
    c.multiply(&pi());
    c.divide_i64(180);
    c.cos();
    c.multiply(&dec("0.0000974"));
    c.subtract(&dec("0.005575"));
    c
}

const SOLAR_COEFFICIENTS: [i64; 49] = [
    403406, 195207, 119433, 112392, 3891, 2819, 1721, 660, 350, 334, 314, 268, 242, 234, 158,
    132, 129, 114, 99, 93, 86, 78, 72, 68, 64, 46, 38, 37, 32, 29, 28, 27, 27, 25, 24, 21, 21,
    20, 18, 17, 14, 13, 13, 13, 12, 10, 10, 10, 10,
];

const SOLAR_MULTIPLIERS: [&str; 49] = [
    "0.9287892", "35999.1376958", "35999.4089666", "35998.7287385", "71998.20261",
    "71998.4403", "36000.35726", "71997.4812", "32964.4678", "-19.4410", "445267.1117",
    "45036.8840", "3.1008", "22518.4434", "-19.9739", "65928.9345", "9038.0293", "3034.7684",
    "33718.148", "3034.448", "-2280.773", "29929.992", "31556.493", "149.588", "9037.750",
    "107997.405", "-4444.176", "151.771", "67555.316", "31556.080", "-4561.540", "107996.706",
    "1221.655", "62894.167", "31437.369", "14578.298", "-31931.757", "34777.243", "1221.999",
    "62894.511", "-4442.039", "107997.909", "119.066", "16859.071", "-4.578", "26895.292",
    "-39.127", "12297.536", "90073.778",
];

const SOLAR_ADDENDS: [&str; 49] = [
    "270.54861", "340.19128", "63.91854", "331.26220", "317.843", "86.631", "240.052", "310.26",
    "247.23", "260.87", "297.82", "343.14", "166.79", "81.53", "3.50", "132.75", "182.95",
    "162.03", "29.8", "266.4", "249.2", "157.6", "257.8", "185.1", "69.9", "8.0", "197.1",
    "250.4", "65.3", "162.7", "341.5", "291.6", "98.5", "146.7", "110.0", "5.2", "342.6",
    "230.9", "256.1", "45.3", "242.9", "115.2", "151.8", "285.3", "53.3", "126.6", "205.7",
    "85.9", "146.1",
];

/// `solar_longitude(tee)` — the sun's apparent longitude in degrees.
fn solar_longitude(tee: &Number) -> Number {
    let c = julian_centuries(tee);
    let mut lam = dec("282.7771834");
    lam.add(&mul(&dec("36000.76953744"), &c));

    let mut series = Number::new();
    for i in 0..SOLAR_COEFFICIENTS.len() {
        let mut z = mul(&dec(SOLAR_MULTIPLIERS[i]), &c);
        z.add(&dec(SOLAR_ADDENDS[i]));
        let mut term = sin_degrees(&z);
        term.multiply(&int(SOLAR_COEFFICIENTS[i]));
        series.add(&term);
    }
    series.multiply(&dec("0.000005729577951308232"));

    lam.add(&series);
    lam.add(&aberration(tee));
    lam.add(&nutation(tee));
    modulo_i64(&lam, 360)
}

// ----------------------------------------------------------------------
// Lunar position
// ----------------------------------------------------------------------

fn mean_lunar_longitude(c: &Number) -> Number {
    modulo_i64(
        &cal_poly(
            c,
            &[
                "218.3164477",
                "481267.88123421",
                "-0.0015786",
                "1/538841",
                "-1/65194000",
            ],
        ),
        360,
    )
}

fn lunar_elongation(c: &Number) -> Number {
    modulo_i64(
        &cal_poly(
            c,
            &[
                "297.8501921",
                "445267.1114034",
                "-0.0018819",
                "1/545868",
                "-1/113065000",
            ],
        ),
        360,
    )
}

fn solar_anomaly(c: &Number) -> Number {
    modulo_i64(
        &cal_poly(
            c,
            &["357.5291092", "35999.0502909", "-0.0001536", "1/24490000"],
        ),
        360,
    )
}

fn lunar_anomaly(c: &Number) -> Number {
    modulo_i64(
        &cal_poly(
            c,
            &[
                "134.9633964",
                "477198.8675055",
                "0.0087414",
                "1/69699",
                "-1/14712000",
            ],
        ),
        360,
    )
}

fn moon_node(c: &Number) -> Number {
    modulo_i64(
        &cal_poly(
            c,
            &[
                "93.2720950",
                "483202.0175233",
                "-0.0036539",
                "-1/3526000",
                "1/863310000",
            ],
        ),
        360,
    )
}

#[rustfmt::skip]
const LUNAR_ARGS_ELONGATION: [i64; 59] = [
    0, 2, 2, 0, 0, 0, 2, 2, 2, 2, 0, 1, 0, 2, 0, 0, 4, 0, 4, 2, 2, 1,
    1, 2, 2, 4, 2, 0, 2, 2, 1, 2, 0, 0, 2, 2, 2, 4, 0, 3, 2, 4, 0, 2,
    2, 2, 4, 0, 4, 1, 2, 0, 1, 3, 4, 2, 0, 1, 2,
];

#[rustfmt::skip]
const LUNAR_ARGS_SOLAR_ANOMALY: [i64; 59] = [
    0, 0, 0, 0, 1, 0, 0, -1, 0, -1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1, 1,
    0, 1, -1, 0, 0, 0, 1, 0, -1, 0, -2, 1, 2, -2, 0, 0, -1, 0, 0, 1,
    -1, 2, 2, 1, -1, 0, 0, -1, 0, 1, 0, 1, 0, 0, -1, 2, 1, 0,
];

#[rustfmt::skip]
const LUNAR_ARGS_LUNAR_ANOMALY: [i64; 59] = [
    1, -1, 0, 2, 0, 0, -2, -1, 1, 0, -1, 0, 1, 0, 1, 1, -1, 3, -2,
    -1, 0, -1, 0, 1, 2, 0, -3, -2, -1, -2, 1, 0, 2, 0, -1, 1, 0,
    -1, 2, -1, 1, -2, -1, -1, -2, 0, 1, 4, 0, -2, 0, 2, 1, -2, -3,
    2, 1, -1, 3,
];

#[rustfmt::skip]
const LUNAR_ARGS_MOON_NODE: [i64; 59] = [
    0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, -2, 2, -2, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, -2, 2, 0, 2, 0, 0, 0, 0,
    0, 0, -2, 0, 0, 0, 0, -2, -2, 0, 0, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
const LUNAR_SINE_COEFF: [i64; 59] = [
    6288774, 1274027, 658314, 213618, -185116, -114332,
    58793, 57066, 53322, 45758, -40923, -34720, -30383,
    15327, -12528, 10980, 10675, 10034, 8548, -7888,
    -6766, -5163, 4987, 4036, 3994, 3861, 3665, -2689,
    -2602, 2390, -2348, 2236, -2120, -2069, 2048, -1773,
    -1595, 1215, -1110, -892, -810, 759, -713, -700, 691,
    596, 549, 537, 520, -487, -399, -381, 351, -340, 330,
    327, -323, 299, 294,
];

/// `lunar_longitude(tee)` — the moon's apparent longitude in degrees.
fn lunar_longitude(tee: &Number) -> Number {
    let c = julian_centuries(tee);
    let cap_l_prime = mean_lunar_longitude(&c);
    let cap_d = lunar_elongation(&c);
    let cap_m = solar_anomaly(&c);
    let cap_m_prime = lunar_anomaly(&c);
    let cap_f = moon_node(&c);
    let cap_e = cal_poly(&c, &["1", "-0.002516", "-0.0000074"]);

    let mut correction = Number::new();
    for i in 0..LUNAR_SINE_COEFF.len() {
        let mut angle = mul(&int(LUNAR_ARGS_ELONGATION[i]), &cap_d);
        angle.add(&mul(&int(LUNAR_ARGS_SOLAR_ANOMALY[i]), &cap_m));
        angle.add(&mul(&int(LUNAR_ARGS_LUNAR_ANOMALY[i]), &cap_m_prime));
        angle.add(&mul(&int(LUNAR_ARGS_MOON_NODE[i]), &cap_f));

        // The eccentricity factor is raised to |solar anomaly argument|,
        // which is the reason the solar terms fade with time.
        let mut e_power = cap_e.clone();
        e_power.raise(&int(LUNAR_ARGS_SOLAR_ANOMALY[i].abs()), true);

        let mut term = mul(&int(LUNAR_SINE_COEFF[i]), &e_power);
        term.multiply(&sin_degrees(&angle));
        correction.add(&term);
    }
    correction.multiply(&dec("0.000001"));

    let mut venus = mul(&dec("131.849"), &c);
    venus.add(&dec("119.75"));
    let venus = mul(&sin_degrees(&venus), &dec("0.003958"));

    let mut jupiter = mul(&dec("479264.29"), &c);
    jupiter.add(&dec("53.09"));
    let jupiter = mul(&sin_degrees(&jupiter), &dec("0.000318"));

    let flat_earth = mul(
        &sin_degrees(&sub(&cap_l_prime, &cap_f)),
        &dec("0.001962"),
    );

    let mut ret = cap_l_prime;
    ret.add(&correction);
    ret.add(&venus);
    ret.add(&jupiter);
    ret.add(&flat_earth);
    ret.add(&nutation(tee));
    modulo_i64(&ret, 360)
}

#[rustfmt::skip]
const NEW_MOON_E_FACTOR: [i64; 24] = [
    0, 1, 0, 0, 1, 1, 2, 0, 0, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
#[rustfmt::skip]
const NEW_MOON_SOLAR_COEFF: [i64; 24] = [
    0, 1, 0, 0, -1, 1, 2, 0, 0, 1, 0, 1, 1, -1, 2, 0, 3, 1, 0, 1, -1, -1, 1, 0,
];
#[rustfmt::skip]
const NEW_MOON_LUNAR_COEFF: [i64; 24] = [
    1, 0, 2, 0, 1, 1, 0, 1, 1, 2, 3, 0, 0, 2, 1, 2, 0, 1, 2, 1, 1, 1, 3, 4,
];
#[rustfmt::skip]
const NEW_MOON_MOON_COEFF: [i64; 24] = [
    0, 0, 0, 2, 0, 0, 0, -2, 2, 0, 0, 2, -2, 0, 0, -2, 0, -2, 2, 2, 2, -2, 0, 0,
];
#[rustfmt::skip]
const NEW_MOON_SINE_COEFF: [&str; 24] = [
    "-0.40720", "0.17241", "0.01608", "0.01039", "0.00739", "-0.00514", "0.00208",
    "-0.00111", "-0.00057", "0.00056", "-0.00042", "0.00042", "0.00038", "-0.00024",
    "-0.00007", "0.00004", "0.00004", "0.00003", "0.00003", "-0.00003", "0.00003",
    "-0.00002", "-0.00002", "0.00002",
];
#[rustfmt::skip]
const NEW_MOON_ADD_CONST: [&str; 13] = [
    "251.88", "251.83", "349.42", "84.66", "141.74", "207.14", "154.84", "34.52",
    "207.19", "291.34", "161.72", "239.56", "331.55",
];
#[rustfmt::skip]
const NEW_MOON_ADD_COEFF: [&str; 13] = [
    "0.016321", "26.651886", "36.412478", "18.206239", "53.303771", "2.453732",
    "7.306860", "27.261239", "0.121824", "1.844379", "24.198154", "25.513099", "3.592518",
];
#[rustfmt::skip]
const NEW_MOON_ADD_FACTOR: [&str; 13] = [
    "0.000165", "0.000164", "0.000126", "0.000110", "0.000062", "0.000060", "0.000056",
    "0.000047", "0.000042", "0.000040", "0.000037", "0.000035", "0.000023",
];

/// `nth_new_moon(n)` — the moment of the n-th new moon since 2000-01-06.
fn nth_new_moon(n: &Number) -> Number {
    let k = sub(n, &int(24724));
    let mut c = dec("1236.85");
    c.recip();
    let c = mul(&c, &k);

    let mut approx = j2000();
    approx.add(&cal_poly(
        &c,
        &[
            "5.09766",
            "29.530588861 * 1236.85",
            "0.00015437",
            "-0.000000150",
            "0.00000000073",
        ],
    ));

    let cap_e = cal_poly(&c, &["1", "-0.002516", "-0.0000074"]);
    let solar_anomaly_n = cal_poly(
        &c,
        &["2.5534", "1236.85 * 29.10535670", "-0.0000014", "-0.00000011"],
    );
    let lunar_anomaly_n = cal_poly(
        &c,
        &[
            "201.5643",
            "385.81693528 * 1236.85",
            "0.0107582",
            "0.00001238",
            "-0.000000058",
        ],
    );
    let moon_argument = cal_poly(
        &c,
        &[
            "160.7108",
            "390.67050284 * 1236.85",
            "-0.0016118",
            "-0.00000227",
            "0.000000011",
        ],
    );
    let cap_omega = cal_poly(&c, &["124.7746", "-1.56375588 * 1236.85", "0.0020672", "0.00000215"]);

    let mut correction = mul(&sin_degrees(&cap_omega), &dec("-0.00017"));
    for i in 0..NEW_MOON_SINE_COEFF.len() {
        let mut angle = mul(&int(NEW_MOON_SOLAR_COEFF[i]), &solar_anomaly_n);
        angle.add(&mul(&int(NEW_MOON_LUNAR_COEFF[i]), &lunar_anomaly_n));
        angle.add(&mul(&int(NEW_MOON_MOON_COEFF[i]), &moon_argument));
        let mut e_power = cap_e.clone();
        e_power.raise(&int(NEW_MOON_E_FACTOR[i]), true);
        let mut term = mul(&dec(NEW_MOON_SINE_COEFF[i]), &e_power);
        term.multiply(&sin_degrees(&angle));
        correction.add(&term);
    }

    let extra = mul(
        &sin_degrees(&cal_poly(&c, &["299.77", "132.8475848", "-0.009173"])),
        &dec("0.000325"),
    );

    let mut additional = Number::new();
    for i in 0..NEW_MOON_ADD_CONST.len() {
        let mut angle = mul(&dec(NEW_MOON_ADD_COEFF[i]), &k);
        angle.add(&dec(NEW_MOON_ADD_CONST[i]));
        additional.add(&mul(&sin_degrees(&angle), &dec(NEW_MOON_ADD_FACTOR[i])));
    }

    approx.add(&correction);
    approx.add(&extra);
    approx.add(&additional);
    universal_from_dynamical(&approx)
}

/// `lunar_phase(tee)` — the moon's phase angle in degrees.
///
/// The direct difference of the two longitudes loses accuracy near the
/// quarters, so the C++ cross-checks it against the elapsed fraction of the
/// synodic month and takes the latter when the two disagree by more than a
/// half turn.
fn lunar_phase(tee: &Number) -> Number {
    let phi = modulo_i64(&sub(&lunar_longitude(tee), &solar_longitude(tee)), 360);
    let t0 = nth_new_moon(&Number::new());
    let mut n = sub(tee, &t0);
    n.divide(&mean_synodic_month());
    n.round_default(false);
    let mut phi_prime = sub(tee, &nth_new_moon(&n));
    phi_prime.divide(&mean_synodic_month());
    phi_prime = modulo_i64(&phi_prime, 1);
    phi_prime.multiply(&int(360));
    let mut test = sub(&phi, &phi_prime);
    test.abs();
    if test.is_greater_than(&int(180)) {
        phi_prime
    } else {
        phi
    }
}

/// `lunar_phase_at_or_after(phase, tee)` — bisection for the next moment the
/// moon reaches `phase` degrees.
fn lunar_phase_at_or_after(phase: &Number, tee: &Number) -> Number {
    let rate = div_i64(&mean_synodic_month(), 360);
    let mut tau = modulo_i64(&sub(phase, &lunar_phase(tee)), 360);
    tau.multiply(&rate);
    tau.add(tee);

    let mut a = sub(&tau, &int(5));
    if tee.is_greater_than(&a) {
        a = tee.clone();
    }
    let mut b = add(&tau, &int(5));

    let prec = dec("0.00001");
    let mut phase_low = sub(phase, &prec);
    let mut phase_high = add(phase, &prec);
    if phase_low.is_negative() {
        phase_low.add(&int(360));
    }
    if phase_high.is_greater_than(&int(360)) {
        phase_high.subtract(&int(360));
    }

    // The interval halves every pass, so this cannot run away; the bound is
    // only a guard against a pathological non-convergence.
    for _ in 0..200 {
        let mut test = sub(&b, &a);
        test.divide_i64(2);
        test.add(&a);
        let newphase = lunar_phase(&test);
        if phase_high.is_less_than(&phase_low) {
            if !newphase.is_less_than(&phase_low) || !newphase.is_greater_than(&phase_high) {
                return test;
            }
        } else if !newphase.is_less_than(&phase_low) && !newphase.is_greater_than(&phase_high) {
            return test;
        }
        // Which half to keep is decided by the *signed* distance around the
        // circle, not a plain comparison: the target can sit either side of
        // the 0/360 wrap.
        let delta = modulo_i64(&sub(&newphase, phase), 360);
        if delta.is_less_than(&int(180)) {
            b = test;
        } else {
            a = test;
        }
    }
    a
}

// ----------------------------------------------------------------------
// Entry points
// ----------------------------------------------------------------------

/// The fixed-day number, including the fraction of a day, of a date.
///
/// The C++ subtracts `dateTimeZone(date, false)` here; values are stored in
/// UTC in this port, so there is nothing to subtract.
fn fixed_moment(date: &QalculateDateTime) -> Number {
    let mut time = date.second().clone();
    time.divide_i64(60);
    time.add(&int(date.minute()));
    time.divide_i64(60);
    time.add(&int(date.hour()));
    time.divide_i64(24);
    add(&date_to_fixed(date.year(), date.month(), date.day()), &time)
}

/// Run `f` with interval arithmetic off, as
/// `beginTemporaryStopIntervalArithmetic` does around every one of these.
fn without_intervals<R>(f: impl FnOnce() -> R) -> R {
    let saved = qalc_num::context::create_interval();
    qalc_num::context::set_create_interval(false);
    let r = f();
    qalc_num::context::set_create_interval(saved);
    r
}

/// `lunarPhase(date)` — the phase as a fraction of a full cycle, 0 at new
/// moon and 0.5 at full moon.
pub fn lunar_phase_of(date: &QalculateDateTime) -> Number {
    without_intervals(|| {
        let mut phase = lunar_phase(&fixed_moment(date));
        phase.divide_i64(360);
        phase.set_precision(8);
        phase
    })
}

/// `findNextLunarPhase(date, phase)` — the first moment at or after `date`
/// when the moon reaches `phase` (a fraction of a full cycle).
pub fn next_lunar_phase(date: &QalculateDateTime, phase: &Number) -> Option<QalculateDateTime> {
    without_intervals(|| {
        let mut degrees = phase.clone();
        degrees.multiply(&int(360));
        let fixed = lunar_phase_at_or_after(&degrees, &fixed_moment(date));
        let (y, m, d) = fixed_to_date(&fixed)?;
        let mut dt = QalculateDateTime::from_date(y, m, d);
        dt.add_days(&sub(&fixed, &date_to_fixed(y, m, d)));
        Some(dt)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gregorian_fixed_day_round_trip() {
        // R.D. 1 is 0001-01-01 by construction.
        assert_eq!(date_to_fixed(1, 1, 1).to_i64(), Some(1));
        assert_eq!(fixed_to_date(&int(1)), Some((1, 1, 1)));
        // 2000-01-01 is R.D. 730120, the integer part of J2000.
        assert_eq!(date_to_fixed(2000, 1, 1).to_i64(), Some(730120));
        assert_eq!(fixed_to_date(&int(730120)), Some((2000, 1, 1)));
        assert_eq!(fixed_to_date(&date_to_fixed(2022, 2, 11)), Some((2022, 2, 11)));
    }

    #[test]
    fn lunar_phase_matches_the_reference() {
        // Oracle: `lunarphase(2022-02-11T00:00Z)` is 0.32288434.
        let date = QalculateDateTime::from_str("2022-02-11T00:00Z").expect("parses");
        let phase = lunar_phase_of(&date);
        let s = phase.print(&qalc_num::PrintOptions::default());
        assert_eq!(s, "0.32288434", "lunar phase, got {s}");
    }

    #[test]
    fn next_lunar_phase_matches_the_reference() {
        // Oracle: `nextlunarphase(0.5, 2022-02-11T00:00Z)` is
        // "2022-02-16T16:56:27Z".
        let date = QalculateDateTime::from_str("2022-02-11T00:00Z").expect("parses");
        let half = Number::parse("0.5", &qalc_num::ParseOptions::default());
        let next = next_lunar_phase(&date, &half).expect("converges");
        assert_eq!(next.to_iso_string(), "2022-02-16T16:56:27");
    }
}
