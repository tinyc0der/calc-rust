//! Text values and string builtins — port of the `char`/`code`/`len`/
//! `concatenate`/`characters` family in `BuiltinFunctions-util.cc`, plus the
//! text-argument parsing rule from `Argument::parse` (Function.cc:1614) and
//! the quoting rule from `MathStructure::print` (MathStructure-print.cc:4184).
//!
//! # Text arguments are *not* ordinary expressions
//!
//! libqalculate's `TextArgument` does not parse its source text in the usual
//! way. `Argument::parse` (the `b_text` branch) says, in effect:
//!
//! * a source fragment containing no parenthesis and no quoted literal is
//!   taken **verbatim** — which is why `concatenate(1*2, 5)` is `"1*25"` and
//!   `len(5/6)` is `3`;
//! * except that a bare name bound to a *text* variable resolves to that
//!   variable — `alpha:="c"` makes `concatenate(alpha)` yield `"c"`, while
//!   `beta:=2` leaves `concatenate(beta)` as the literal `"beta"`;
//! * anything else (a call, a quoted literal) is parsed and evaluated
//!   normally.
//!
//! [`text_arg_indices`] tells the parser which argument positions follow
//! that rule; the parser implements it in `parse_text_call`.
//!
//! # Quoting
//!
//! A text value prints quoted (`allow_non_usable = false`, the default):
//! single quotes when it is exactly one character and contains no `'`, or
//! when it contains a `"`; double quotes otherwise. Hence `"x"` prints as
//! `'x'` but `"xx"` prints as `"xx"` and `'12'` prints as `"12"`.

use crate::ids::FunctionId;
use crate::structure::MathStructure;
use qalc_num::{Number, ParseOptions, PrintOptions};

/// `FUNCTION_ID_*` values from BuiltinFunctions.h:440.
pub mod id {
    pub const ASCII: u32 = 2500;
    pub const CHAR: u32 = 2501;
    pub const LENGTH: u32 = 2502;
    pub const CONCATENATE: u32 = 2503;
    pub const CHARACTERS: u32 = 2505;
}

/// Resolve a string builtin name to its id.
pub fn function_id_for_name(name: &str) -> Option<FunctionId> {
    let v = match name {
        "code" | "ascii" => id::ASCII,
        "char" => id::CHAR,
        "len" | "length" => id::LENGTH,
        "concatenate" => id::CONCATENATE,
        "characters" => id::CHARACTERS,
        _ => return None,
    };
    Some(FunctionId(v))
}

/// Display names for the ids above.
pub fn function_name(fid: u32) -> Option<&'static str> {
    Some(match fid {
        id::ASCII => "code",
        id::CHAR => "char",
        id::LENGTH => "len",
        id::CONCATENATE => "concatenate",
        id::CHARACTERS => "characters",
        _ => return None,
    })
}

/// Which argument positions of `fid` are `TextArgument`s.
///
/// `None` means the function has none and is parsed normally. A position of
/// [`usize::MAX`] in the returned slice means "and every later position too"
/// — `concatenate` is variadic and treats all of its arguments as text.
pub fn text_arg_indices(fid: u32) -> Option<&'static [usize]> {
    use crate::builtins::id as b;
    Some(match fid {
        // ConcatenateFunction: TextArgument on 1 and 2, and the reference
        // keeps later arguments verbatim as well.
        id::CONCATENATE => &[usize::MAX],
        // LengthFunction / CharactersFunction: one TextArgument.
        id::LENGTH | id::CHARACTERS => &[0],
        // AsciiFunction("code", 1, 3): text, text encoding, boolean.
        id::ASCII => &[0, 1],
        // DecFunction/HexFunction/... all take the digits as text, which is
        // how `hex(34)` reads "34" in base 16.
        b::BASE_HEX | b::BASE_BIN | b::BASE_OCT | b::BASE_DEC | b::BASE_N => &[0],
        // LoadFunction: `FileArgument` on 1 (text) and the delimiter on 3.
        crate::stats::id::LOAD => &[0, 2],
        _ => return None,
    })
}

/// Does argument `index` of `fid` take raw text?
pub fn is_text_arg(fid: u32, index: usize) -> bool {
    match text_arg_indices(fid) {
        Some(idx) => idx.contains(&usize::MAX) || idx.contains(&index),
        None => false,
    }
}

/// True when this function takes at least one text argument.
pub fn has_text_args(fid: u32) -> bool {
    text_arg_indices(fid).is_some()
}

// ----------------------------------------------------------------------
// Printing
// ----------------------------------------------------------------------

/// `unicode_length` — the number of code points, as libqalculate counts a
/// text's length.
pub fn unicode_length(s: &str) -> usize {
    s.chars().count()
}

/// Quote a text value for output, per MathStructure-print.cc:4184.
pub fn quote_text(s: &str) -> String {
    if (unicode_length(s) == 1 && !s.contains('\'')) || s.contains('"') {
        format!("'{s}'")
    } else {
        format!("\"{s}\"")
    }
}

/// `Number::print` for `BASE_UNICODE` (Number.cc:11185), restricted to the
/// single-digit case: an integer below 2^32 is one Unicode "digit".
///
/// With `use_unicode_signs` off — the mode `--test-file` runs in — every
/// code point above 0x7f is escaped as `\<decimal>`, which is why
/// `0xD8 to unicode` prints `\216` unless `/set unicode 1` ran first.
pub fn unicode_digits(n: &Number, po: &PrintOptions) -> Option<String> {
    if !n.is_integer() || n.is_negative() {
        return None;
    }
    let mut v = n.to_i64()?;
    // Base 2^32: split into digits, most significant first.
    let base = 1i64 << 32;
    let mut digits = Vec::new();
    if v == 0 {
        digits.push(0);
    }
    while v > 0 {
        digits.push(v % base);
        v /= base;
    }
    digits.reverse();
    let mut out = String::new();
    let mut prev_esc = false;
    for &c in &digits {
        let ch = u32::try_from(c).ok().and_then(char::from_u32);
        if c <= 32 || (!po.use_unicode_signs && c > 0x7f) || c >= 1_114_112 || ch.is_none() {
            out.push('\\');
            out.push_str(&c.to_string());
            prev_esc = true;
        } else if prev_esc && (b'0'..=b'9').contains(&(c as u8)) && c <= 0x7f {
            out.push('\\');
            out.push_str(&c.to_string());
            prev_esc = true;
        } else {
            out.push(ch.expect("checked above"));
            prev_esc = false;
        }
    }
    Some(out)
}

// ----------------------------------------------------------------------
// Evaluation
// ----------------------------------------------------------------------

/// Render an evaluated argument the way `format_and_print` does when
/// `concatenate` folds a non-text value into its result.
fn print_value(m: &MathStructure) -> String {
    crate::print::print(m, &crate::eval::batch_print_options())
}

/// The string an argument contributes to a concatenation.
fn as_concat_part(m: &MathStructure) -> String {
    match m {
        MathStructure::Text(s) => s.clone(),
        other => print_value(other),
    }
}

/// `CharFunction::calculate` — a code point as a one-character text.
fn char_of(n: &Number) -> Option<MathStructure> {
    let v = n.to_i64()?;
    let c = u32::try_from(v).ok().and_then(char::from_u32)?;
    Some(MathStructure::Text(c.to_string()))
}

/// `AsciiFunction::calculate` — the code points (or bytes) of a text.
fn code_of(text: &str, encoding: i32, as_vector: bool) -> Option<MathStructure> {
    if text.is_empty() {
        return None;
    }
    let mut values: Vec<i64> = Vec::new();
    if encoding == 0 {
        // UTF-8 / ASCII: one value per byte.
        values.extend(text.bytes().map(i64::from));
        if as_vector && text.len() > 1 {
            return Some(MathStructure::Vector(
                values.into_iter().map(int_struct).collect(),
            ));
        }
        return Some(int_struct(fold_digits(&values, 0x100)));
    }
    for ch in text.chars() {
        let c = ch as i64;
        if encoding == 1 && c >= 0x10000 {
            // UTF-16: a surrogate pair.
            let x = c - 0x10000;
            values.push(0xD800 + x / 0x400);
            values.push(0xDC00 + x % 0x400);
        } else {
            values.push(c);
        }
    }
    if as_vector {
        if values.len() == 1 {
            return Some(int_struct(values[0]));
        }
        return Some(MathStructure::Vector(
            values.into_iter().map(int_struct).collect(),
        ));
    }
    let radix = if encoding == 1 { 0x10000 } else { 0x1_0000_0000 };
    Some(int_struct(fold_digits(&values, radix)))
}

/// Combine digit values into one number in the given radix.
fn fold_digits(values: &[i64], radix: i64) -> i64 {
    let mut acc = 0i64;
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            acc = acc.saturating_mul(radix);
        }
        acc = acc.saturating_add(*v);
    }
    acc
}

fn int_struct(v: i64) -> MathStructure {
    MathStructure::Number(Number::from_i64(v))
}

/// `AsciiFunction`'s encoding argument (`"UTF-32"` by default).
fn encoding_of(m: Option<&MathStructure>) -> i32 {
    let Some(m) = m else { return 2 };
    let s = match m {
        MathStructure::Text(s) | MathStructure::Symbolic(s) => s.clone(),
        MathStructure::Number(n) => n.print(&PrintOptions::default()),
        _ => return 2,
    };
    let s = s.trim().to_ascii_lowercase().replace(['-', '\u{2212}'], "");
    match s.as_str() {
        "utf16" | "1" => 1,
        "utf8" | "ascii" | "0" => 0,
        _ => 2,
    }
}

fn boolean_of(m: Option<&MathStructure>, default: bool) -> bool {
    match m {
        Some(MathStructure::Number(n)) => !n.is_zero(),
        Some(MathStructure::Text(s)) => s != "0" && !s.is_empty(),
        _ => default,
    }
}

/// Parse `text` in `base` and evaluate it — the body shared by `dec`, `hex`,
/// `bin`, `oct` and `base` (BuiltinFunctions-number.cc:1540).
fn read_in_base(text: &str, base: i32, twos_complement: bool) -> Option<MathStructure> {
    let mut po = ParseOptions::default();
    po.base = base;
    po.twos_complement = twos_complement && base == 2;
    po.hexadecimal_twos_complement = twos_complement && base == 16;
    let mut m = crate::parser::parse(text, &po).ok()?;
    crate::eval::evaluate(&mut m);
    Some(m)
}

/// `dec(x, 1)` and friends: evaluate normally, then print in `base`.
///
/// The C++ finishes with `mstruct.set(mstruct.print(po), true, true)`, which
/// is the **symbolic** setter — so `bin(255, 0, 1)` prints the bare digits
/// `11111111`, not the quoted text `"11111111"`.
///
/// It also prints under a *fresh* `PrintOptions`, not the CLI's. The only
/// field that matters is `base_display`, whose library default is
/// `BASE_DISPLAY_NONE` while qalc sets `BASE_DISPLAY_NORMAL` — and
/// `format_number_string` (Number.cc:242) hangs the zero-padding, the
/// four-digit grouping *and* the `0x`/`0` prefixes off exactly that test. So
/// `255 to bin` is `0000 0000 1111 1111` while `bin(255, 0, 1)` is
/// `11111111`, and `8 to oct` is `010` while `oct(8, 1)` is `10`.
fn print_in_base(text: &str, base: i32, twos_complement: bool) -> Option<MathStructure> {
    let mut m = crate::parser::parse(text, &ParseOptions::default()).ok()?;
    crate::eval::evaluate(&mut m);
    let mut po = PrintOptions::default();
    po.base = base;
    po.base_display = qalc_num::options::BaseDisplay::None;
    po.twos_complement = twos_complement;
    po.hexadecimal_twos_complement = twos_complement && base == 16;
    Some(MathStructure::Symbolic(crate::print::print(&m, &po)))
}

/// The text of an argument that the parser turned into a text value.
fn arg_text(m: &MathStructure) -> Option<String> {
    match m {
        MathStructure::Text(s) => Some(s.clone()),
        MathStructure::Symbolic(s) => Some(s.clone()),
        _ => None,
    }
}

/// Evaluate a string builtin in place. Returns true when it was replaced.
pub fn calculate_function(m: &mut MathStructure) -> bool {
    let MathStructure::Function { id: fid, args } = m else {
        return false;
    };
    let fid = fid.0;
    let args = args.clone();
    match apply(fid, &args) {
        Some(r) => {
            *m = r;
            true
        }
        None => false,
    }
}

fn apply(fid: u32, args: &[MathStructure]) -> Option<MathStructure> {
    use crate::builtins::id as b;
    match fid {
        id::CONCATENATE if !args.is_empty() => {
            // A vector argument distributes: `concatenate(["a" "b"], "!")`
            // yields one result per element.
            if let Some(len) = args.iter().find_map(|a| match a {
                MathStructure::Vector(v) => Some(v.len()),
                _ => None,
            }) {
                let mut out = Vec::with_capacity(len);
                for i in 0..len {
                    let mut s = String::new();
                    for a in args {
                        match a {
                            MathStructure::Vector(v) => {
                                s.push_str(&as_concat_part(v.get(i)?));
                            }
                            other => s.push_str(&as_concat_part(other)),
                        }
                    }
                    out.push(MathStructure::Text(s));
                }
                return Some(MathStructure::Vector(out));
            }
            let mut s = String::new();
            for a in args {
                s.push_str(&as_concat_part(a));
            }
            Some(MathStructure::Text(s))
        }
        id::LENGTH if args.len() == 1 => {
            let t = arg_text(&args[0])?;
            Some(int_struct(unicode_length(&t) as i64))
        }
        id::CHARACTERS if args.len() == 1 => {
            let t = arg_text(&args[0])?;
            Some(MathStructure::Vector(
                t.chars().map(|c| MathStructure::Text(c.to_string())).collect(),
            ))
        }
        id::CHAR if args.len() == 1 => match &args[0] {
            MathStructure::Number(n) => char_of(n),
            // `b_handle_vector`: a vector argument maps elementwise.
            MathStructure::Vector(v) => {
                let mut out = Vec::with_capacity(v.len());
                for e in v {
                    out.push(char_of(e.number()?)?);
                }
                Some(MathStructure::Vector(out))
            }
            _ => None,
        },
        id::ASCII if (1..=3).contains(&args.len()) => {
            let t = arg_text(&args[0])?;
            let enc = encoding_of(args.get(1));
            let as_vector = boolean_of(args.get(2), true);
            code_of(&t, enc, as_vector)
        }
        // The base-reading functions take their digits as text.
        //
        // Argument layout, from the C++ constructors
        // (BuiltinFunctions-number.cc:1500-1590):
        //
        // | call                        | 2nd arg          | 3rd arg |
        // |-----------------------------|------------------|---------|
        // | `bin(x, twos, reverse)`     | two's complement | reverse |
        // | `hex(x, twos, reverse)`     | two's complement | reverse |
        // | `oct(x, reverse)`           | reverse          | —       |
        // | `dec(x, reverse)`           | reverse          | —       |
        //
        // Getting `bin`'s two flags the wrong way round is not a cosmetic
        // slip: `bin(11111111, 1)` is the two's-complement literal −1, and
        // the port used to answer with a formatted binary *string* instead.
        b::BASE_DEC | b::BASE_HEX | b::BASE_BIN | b::BASE_OCT => {
            let t = arg_text(args.first()?)?;
            let base = match fid {
                b::BASE_DEC => 10,
                b::BASE_HEX => 16,
                b::BASE_BIN => 2,
                _ => 8,
            };
            let has_twos_flag = fid == b::BASE_HEX || fid == b::BASE_BIN;
            let (twos, reverse) = if has_twos_flag {
                (boolean_of(args.get(1), false), boolean_of(args.get(2), false))
            } else {
                (false, boolean_of(args.get(1), false))
            };
            if reverse {
                print_in_base(&t, base, twos)
            } else {
                read_in_base(&t, base, twos)
            }
        }
        // `base(x, radix, digits, reverse)`.
        b::BASE_N if (2..=4).contains(&args.len()) => {
            let t = arg_text(&args[0])?;
            let base = args[1].number()?.to_i64()?;
            if !(2..=36).contains(&base) {
                return None;
            }
            if boolean_of(args.get(3), false) {
                print_in_base(&t, base as i32, false)
            } else {
                read_in_base(&t, base as i32, false)
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    /// Expectations come from `tests/strings.batch` and the reference
    /// binary (`qalc -t +u8`).
    fn ev(s: &str) -> String {
        Session::new().evaluate_line(s).expect("evaluates")
    }

    #[test]
    fn quoting_rule_matches_the_reference() {
        assert_eq!(quote_text(""), "\"\"");
        assert_eq!(quote_text("x"), "'x'");
        assert_eq!(quote_text("xx"), "\"xx\"");
        assert_eq!(quote_text("12"), "\"12\"");
        // A single character that *is* a quote falls to double quotes.
        assert_eq!(quote_text("'"), "\"'\"");
        // Anything containing a double quote uses single quotes.
        assert_eq!(quote_text("a\"b"), "'a\"b'");
    }

    #[test]
    fn literal_texts_print_quoted() {
        assert_eq!(ev("\"\""), "\"\"");
        assert_eq!(ev("\"x\""), "'x'");
        assert_eq!(ev("\"xx\""), "\"xx\"");
        assert_eq!(ev("\"meters\""), "\"meters\"");
        assert_eq!(ev("'12'"), "\"12\"");
        assert_eq!(ev("\"12\""), "\"12\"");
    }

    #[test]
    fn concatenate_joins_quoted_literals() {
        assert_eq!(ev("concatenate(\"a\", \"bc\", 'defg')"), "\"abcdefg\"");
        assert_eq!(ev("concatenate(\"\", \"c\", '', 'd')"), "\"cd\"");
    }

    #[test]
    fn concatenate_keeps_unquoted_arguments_verbatim() {
        // TextArgument does not parse: `1*2` contributes the source text.
        assert_eq!(ev("concatenate(1,2)"), "\"12\"");
        assert_eq!(ev("concatenate(1*2, 5)"), "\"1*25\"");
    }

    #[test]
    fn dec_reads_a_concatenated_expression() {
        assert_eq!(ev("dec(concatenate(4*2, 5))"), "100");
    }

    #[test]
    fn text_variables_resolve_but_numeric_ones_do_not() {
        let mut s = Session::new();
        s.evaluate_line("alpha:=\"c\"").unwrap();
        s.evaluate_line("beta:=2").unwrap();
        assert_eq!(
            s.evaluate_line("concatenate(concatenate(a, b), alpha, d, dec(123, 1), beta)")
                .unwrap(),
            "\"abcd123beta\""
        );
        // On its own `beta` is still the number.
        assert_eq!(s.evaluate_line("beta").unwrap(), "2");
    }

    #[test]
    fn length_counts_code_points_of_the_raw_text() {
        assert_eq!(ev("len(\"\")"), "0");
        assert_eq!(ev("len(\" \")"), "1");
        assert_eq!(ev("len(5)"), "1");
        // `5/6` is three characters of source, not the number 0.8333….
        assert_eq!(ev("len(5/6)"), "3");
        assert_eq!(ev("len(concatenate(\"a\", \"bc\"))"), "3");
    }

    #[test]
    fn char_builds_text_from_a_code_point() {
        assert_eq!(ev("char(0xD8)"), "'\u{d8}'");
        assert_eq!(ev("char([0xD8, 0x61])"), "['\u{d8}'  'a']");
    }

    #[test]
    fn code_returns_code_points() {
        assert_eq!(ev("code(abc)"), "[97  98  99]");
        assert_eq!(ev("code(\u{d8}) to hex"), "0xD8");
        assert_eq!(ev("code(\u{1f600}) to hex"), "0x1F600");
    }

    #[test]
    fn code_in_utf8_folds_the_bytes() {
        assert_eq!(ev("code(\u{1f349}, utf-8, 0) to hex"), "0xF09F8D89");
    }

    #[test]
    fn characters_splits_into_one_element_per_code_point() {
        assert_eq!(ev("characters(abc)"), "['a'  'b'  'c']");
    }

    #[test]
    fn hex_still_reads_digits_in_base_16() {
        // tests/numberbase.batch
        assert_eq!(ev("hex(34)"), "52");
    }

    #[test]
    fn unicode_base_escapes_without_unicode_signs() {
        let mut s = Session::new();
        // `--test-file` starts with unicode signs off.
        assert_eq!(s.evaluate_line("0xD8 to unicode").unwrap(), "\\216");
        s.evaluate_line("/set unicode 1").unwrap();
        assert_eq!(s.evaluate_line("0xD8 to unicode").unwrap(), "\u{d8}");
    }

    #[test]
    fn concatenate_distributes_over_a_vector_argument() {
        // ConcatenateFunction builds one result per element.
        assert_eq!(ev("concatenate([\"a\", \"b\"], \"!\")"), "[\"a!\"  \"b!\"]");
    }

    #[test]
    fn unicode_length_counts_code_points_not_bytes() {
        assert_eq!(unicode_length("\u{1f600}"), 1);
        assert_eq!(unicode_length("ab\u{d8}"), 3);
        assert_eq!(unicode_length(""), 0);
    }

    #[test]
    fn text_arg_positions() {
        assert!(is_text_arg(id::CONCATENATE, 0));
        assert!(is_text_arg(id::CONCATENATE, 7));
        assert!(is_text_arg(id::ASCII, 1));
        assert!(!is_text_arg(id::ASCII, 2));
        assert!(!is_text_arg(id::CHAR, 0));
        assert!(!has_text_args(crate::builtins::id::SQRT));
    }
}
