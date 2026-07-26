//! Date and time values in expressions — the evaluation side of
//! `QalculateDateTime`.
//!
//! Behaviour confirmed against the reference binary:
//!
//! | expression | result |
//! |---|---|
//! | `"2020-05-20" + 523d` | `"2021-10-25"` |
//! | `addDays(2020-05-20; 523)` | `"2021-10-25"` |
//! | `"2020-11-05" - "2020-10-05"` | `31 d` |
//! | `"2020-10-05" - "2020-10-15"` | `-10 d` |
//! | `timestamp(2020-05-20T00:00:00Z)` | `1589932800` |
//!
//! A date prints as a quoted ISO string. Subtracting two dates gives a
//! duration in days; adding a duration to a date gives a date.

use crate::ids::FunctionId;
use crate::structure::{DateTimeValue, MathStructure};
use qalc_num::Number;

/// Function ids for the date builtins.
///
/// The 2600 block: 2000 is polynomial's, and an overlap silently routes
/// dispatch to the wrong module.
pub mod id {
    pub const TIMESTAMP: u32 = 2600;
    pub const STAMP_TO_DATE: u32 = 2601;
    pub const ADD_DAYS: u32 = 2602;
    pub const ADD_MONTHS: u32 = 2603;
    pub const ADD_YEARS: u32 = 2604;
    pub const DAYS_BETWEEN: u32 = 2605;
    pub const YEAR: u32 = 2606;
    pub const MONTH: u32 = 2607;
    pub const DAY: u32 = 2608;
    pub const WEEKDAY: u32 = 2609;
    pub const WEEK: u32 = 2610;
    pub const YEARDAY: u32 = 2611;
}

/// Resolve a builtin date function name to its id.
pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let id = match name {
        "timestamp" => id::TIMESTAMP,
        "stamptodate" => id::STAMP_TO_DATE,
        "addDays" | "adddays" => id::ADD_DAYS,
        "addMonths" | "addmonths" => id::ADD_MONTHS,
        "addYears" | "addyears" => id::ADD_YEARS,
        "days" => id::DAYS_BETWEEN,
        "year" => id::YEAR,
        "month" => id::MONTH,
        "day" => id::DAY,
        "weekday" => id::WEEKDAY,
        "week" => id::WEEK,
        "yearday" => id::YEARDAY,
        _ => return None,
    };
    Some(FunctionId(id))
}

/// Display name for a date function id.
pub fn function_name(id: u32) -> Option<&'static str> {
    Some(match id {
        id::TIMESTAMP => "timestamp",
        id::STAMP_TO_DATE => "stamptodate",
        id::ADD_DAYS => "addDays",
        id::ADD_MONTHS => "addMonths",
        id::ADD_YEARS => "addYears",
        id::DAYS_BETWEEN => "days",
        id::YEAR => "year",
        id::MONTH => "month",
        id::DAY => "day",
        id::WEEKDAY => "weekday",
        id::WEEK => "week",
        id::YEARDAY => "yearday",
        _ => return None,
    })
}

/// Argument positions that are `DateArgument`s, which `Argument::parse`
/// takes from the *source text* rather than from a parsed expression — the
/// same rule as `TextArgument` (see [`crate::strings`]).
///
/// This is why `addDays(2020-05-20; 523)` gives a date while a bare
/// `2020-05-20` on its own is arithmetic and gives 1995: only inside a date
/// argument is the fragment re-read as a date.
pub fn date_arg_indices(fid: u32) -> Option<&'static [usize]> {
    Some(match fid {
        id::TIMESTAMP => &[0],
        id::ADD_DAYS | id::ADD_MONTHS | id::ADD_YEARS => &[0],
        id::DAYS_BETWEEN => &[0, 1],
        id::YEAR | id::MONTH | id::DAY | id::WEEKDAY | id::WEEK | id::YEARDAY => &[0],
        _ => return None,
    })
}

/// Is this argument position a date argument?
pub fn is_date_arg(fid: u32, index: usize) -> bool {
    match date_arg_indices(fid) {
        Some(idx) => idx.contains(&index),
        None => false,
    }
}

/// True when this function takes at least one date argument.
pub fn has_date_args(fid: u32) -> bool {
    date_arg_indices(fid).is_some()
}

/// Parse `text` as a date, if it looks like one.
///
/// Used for both quoted strings (`"2020-05-20"`) and the bare form the
/// reference also accepts (`addDays(2020-05-20; 523)`).
pub fn parse_date(text: &str) -> Option<DateTimeValue> {
    let t = text.trim();
    // Require a digit-led, dash- or colon-bearing string so ordinary words
    // and plain numbers are not swallowed.
    if !t.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return None;
    }
    if !t.contains('-') && !t.contains(':') {
        return None;
    }
    DateTimeValue::from_str(t)
}

/// A date structure from a value.
pub fn date_structure(d: DateTimeValue) -> MathStructure {
    MathStructure::DateTime(Box::new(d))
}

/// If `m` is a date, borrow it.
pub fn as_date(m: &MathStructure) -> Option<&DateTimeValue> {
    match m {
        MathStructure::DateTime(d) => Some(d),
        _ => None,
    }
}

/// Evaluate a date builtin in place. Returns true when it was replaced.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id, args } = m else {
        return false;
    };
    let fid = id.0;
    let args = args.clone();
    match apply_builtin(fid, &args) {
        Some(v) => {
            *m = v;
            true
        }
        None => false,
    }
}

/// Apply a date builtin, or `None` when the arguments do not fit.
fn apply_builtin(fid: u32, args: &[MathStructure]) -> Option<MathStructure> {
    match fid {
        id::TIMESTAMP => {
            let d = arg_date(args, 0)?;
            Some(MathStructure::Number(d.timestamp()))
        }
        id::STAMP_TO_DATE => {
            let n = arg_number(args, 0)?;
            Some(date_structure(DateTimeValue::from_timestamp(&n)))
        }
        id::ADD_DAYS | id::ADD_MONTHS | id::ADD_YEARS => {
            let mut d = arg_date(args, 0)?;
            let n = arg_number(args, 1)?;
            let ok = match fid {
                id::ADD_DAYS => d.add_days(&n),
                id::ADD_MONTHS => d.add_months(&n),
                _ => d.add_years(&n),
            };
            ok.then(|| date_structure(d))
        }
        id::DAYS_BETWEEN => {
            let a = arg_date(args, 0)?;
            let b = arg_date(args, 1)?;
            let basis = args
                .get(2)
                .and_then(number_of)
                .and_then(|n| n.to_i64())
                .unwrap_or(1) as i32;
            Some(MathStructure::Number(a.days_to(&b, basis, true, true)))
        }
        id::YEAR => Some(MathStructure::Number(Number::from_i64(arg_date(args, 0)?.year()))),
        id::MONTH => Some(MathStructure::Number(Number::from_i64(arg_date(args, 0)?.month()))),
        id::DAY => Some(MathStructure::Number(Number::from_i64(arg_date(args, 0)?.day()))),
        id::WEEKDAY => Some(MathStructure::Number(Number::from_i64(
            arg_date(args, 0)?.weekday() as i64,
        ))),
        id::YEARDAY => Some(MathStructure::Number(Number::from_i64(
            arg_date(args, 0)?.yearday() as i64,
        ))),
        _ => None,
    }
}

fn number_of(m: &MathStructure) -> Option<Number> {
    match m {
        MathStructure::Number(n) => Some(n.clone()),
        _ => None,
    }
}

fn arg_number(args: &[MathStructure], i: usize) -> Option<Number> {
    number_of(args.get(i)?)
}

/// A date argument, accepting a date value or anything that parses as one
/// (a text value, or a bare `2020-05-20` that survived as a symbol).
fn arg_date(args: &[MathStructure], i: usize) -> Option<DateTimeValue> {
    let a = args.get(i)?;
    if let Some(d) = as_date(a) {
        return Some(d.clone());
    }
    match a {
        MathStructure::Text(t) => parse_date(t),
        MathStructure::Symbolic(s) => parse_date(s),
        _ => None,
    }
}

/// Fold date arithmetic in a sum: `date + duration` and `date - date`.
///
/// Durations arrive as a quantity in seconds (`523d` is `45187200 s`), which
/// is why this runs after unit reduction.
pub fn merge_addition(terms: &mut Vec<MathStructure>) -> bool {
    let date_positions: Vec<usize> = terms
        .iter()
        .enumerate()
        .filter(|(_, t)| as_date(t).is_some() || negated_date(t).is_some())
        .map(|(i, _)| i)
        .collect();
    if date_positions.is_empty() {
        return false;
    }
    // date - date: both a plain and a negated date are present.
    if date_positions.len() == 2 {
        let (a, b) = (date_positions[0], date_positions[1]);
        let plain = as_date(&terms[a]).cloned();
        let neg = negated_date(&terms[b]);
        if let (Some(later), Some(earlier)) = (plain, neg) {
            {
                let days = earlier.days_to(&later, 1, true, true);
                let mut rest: Vec<MathStructure> = terms
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != a && *i != b)
                    .map(|(_, t)| t.clone())
                    .collect();
                DATE_DURATION.with(|c| c.set(true));
                rest.push(days_quantity(days));
                *terms = rest;
                return true;
            }
        }
        return false;
    }
    if date_positions.len() != 1 {
        return false;
    }
    let di = date_positions[0];
    let Some(mut date) = as_date(&terms[di]).cloned() else {
        return false;
    };
    // Every other term must be a duration in seconds.
    let mut seconds = Number::new();
    for (i, t) in terms.iter().enumerate() {
        if i == di {
            continue;
        }
        let Some(s) = seconds_of(t) else {
            return false;
        };
        if !seconds.add(&s) {
            return false;
        }
    }
    if seconds.is_zero() {
        return false;
    }
    // A whole number of days is added as days, not seconds, so a date-only
    // value stays date-only: the reference prints `"2021-10-25"`, not
    // `"2021-10-25T00:00:00"`.
    let mut whole_days = seconds.clone();
    let day_secs = Number::from_i64(86_400);
    let is_whole_days = whole_days.divide(&day_secs) && whole_days.is_integer();
    let ok = if is_whole_days && !date.time_is_set() {
        date.add_days(&whole_days)
    } else {
        date.add_seconds(&seconds, true, true)
    };
    if !ok {
        return false;
    }
    *terms = vec![date_structure(date)];
    true
}

/// `Multiplication[-1, DateTime]` — a date being subtracted.
fn negated_date(m: &MathStructure) -> Option<DateTimeValue> {
    let MathStructure::Multiplication(f) = m else {
        return None;
    };
    if f.len() != 2 {
        return None;
    }
    match (&f[0], &f[1]) {
        (MathStructure::Number(n), MathStructure::DateTime(d)) if n.is_minus_one() => {
            Some((**d).clone())
        }
        _ => None,
    }
}


/// Turn text values that look like dates into date values, and fold date
/// arithmetic in every sum. Runs as one pass over the tree.
pub fn apply(m: &mut MathStructure) -> bool {
    let mut changed = false;
    for i in 0..m.size() {
        if let Some(child) = m.get_mut(i) {
            changed |= apply(child);
        }
    }
    // A quoted string holding a date becomes a date value.
    if let MathStructure::Text(t) = m {
        if let Some(d) = parse_date(t) {
            *m = date_structure(d);
            return true;
        }
    }
    if let MathStructure::Addition(terms) = m {
        if merge_addition(terms) {
            changed = true;
            if terms.len() == 1 {
                *m = terms.remove(0);
            }
        }
    }
    changed
}

thread_local! {
    /// Set when a date difference produced a duration in days.
    ///
    /// The automatic optimal-SI post-conversion would otherwise rewrite that
    /// `31 d` as `2678400 s` — which is right for a user typing `31 d`, but
    /// not for a date difference, where the reference keeps days.
    static DATE_DURATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Did the last evaluation produce a date-derived duration? Clears the flag.
pub fn took_date_duration() -> bool {
    DATE_DURATION.with(|c| c.replace(false))
}

/// A duration in seconds, if `m` is a pure time quantity.
///
/// Durations reach this point already reduced to the base unit, so `523d`
/// arrives as `45187200 s`.
fn seconds_of(m: &MathStructure) -> Option<Number> {
    let store = crate::units::store()?;
    let q = crate::units::quantity_of(store, m)?;
    // Exactly one unit, to the first power, and it must be the second.
    if q.sig.len() != 1 {
        return None;
    }
    let (uid, exp) = q.sig.iter().next()?;
    if *exp != 1 {
        return None;
    }
    if store.reference_name(*uid) != "s" {
        return None;
    }
    Some(q.coeff)
}

/// `n d` — a count of days as a quantity, which is how the reference prints
/// the difference between two dates.
fn days_quantity(days: Number) -> MathStructure {
    match crate::units::store().and_then(|s| s.resolve_name("d")) {
        Some(unit) => MathStructure::Multiplication(vec![MathStructure::Number(days), unit]),
        None => MathStructure::Number(days),
    }
}

#[cfg(test)]
mod tests {
    use crate::session::Session;

    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn dates_parse_and_print_as_quoted_iso() {
        assert_eq!(ev("\"2020-05-20\""), "\"2020-05-20\"");
    }

    #[test]
    fn adding_a_duration_gives_a_date() {
        assert_eq!(ev("\"2020-05-20\" + 523d"), "\"2021-10-25\"");
    }

    #[test]
    fn add_days_builtin() {
        assert_eq!(ev("addDays(2020-05-20; 523)"), "\"2021-10-25\"");
    }

    #[test]
    fn subtracting_dates_gives_days() {
        assert_eq!(ev("\"2020-11-05\" - \"2020-10-05\""), "31 d");
        assert_eq!(ev("\"2020-10-05\" - \"2020-10-15\""), "-10 d");
    }

    #[test]
    fn timestamps_round_trip() {
        assert_eq!(ev("timestamp(2020-05-20T00:00:00Z)"), "1589932800");
    }
}
