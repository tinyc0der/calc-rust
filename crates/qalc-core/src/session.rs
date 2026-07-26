//! Evaluation session — the mutable state a `qalc` run carries between
//! expressions: user-defined variables, print options and parse options.
//!
//! This is the slice of the C++ `Calculator` singleton that the CLI needs
//! per line. Registry-backed definitions (units, builtin variables) live in
//! [`crate::defs::Registry`]; user assignments live here.

use std::collections::HashMap;

use crate::builtins;
use crate::ids::FunctionId;
use crate::parser::{self, NameResolver, ParseError};
use crate::print;
use crate::options::{ApproximationMode, EvaluationOptions};
use crate::structure::MathStructure;
use qalc_num::options::NumberFractionFormat;
use qalc_num::{ParseOptions, PrintOptions};

/// A calculator session.
pub struct Session {
    /// User-assigned variables, by name.
    variables: HashMap<String, MathStructure>,
    pub parse_options: ParseOptions,
    pub print_options: PrintOptions,
    /// `/set approximation ...` and friends.
    pub eval_options: EvaluationOptions,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let mut s = Session {
            variables: HashMap::new(),
            parse_options: ParseOptions::default(),
            print_options: crate::eval::batch_print_options(),
            eval_options: EvaluationOptions::default(),
        };
        s.install_builtin_constants();
        // Build the unit store up front. The parser consults it to decide
        // whether a name is already taken (which is what keeps `2m` metres
        // rather than 2000000), and it can only do so through the
        // non-blocking accessor — so the store has to already exist by the
        // time any user expression is parsed.
        let _ = crate::units::store();
        s
    }

    /// Constants that `Calculator::addBuiltinVariables` defines.
    ///
    /// Only the imaginary unit for now. `pi`, `e` and the rest are
    /// `KnownVariable`s that stay symbolic under exact evaluation and only
    /// collapse to a value under approximation; that needs the two-phase
    /// exact-then-approximate pass `eval` does not implement yet, and
    /// defining them here would break the symbolic output the limit
    /// transcript depends on.
    fn install_builtin_constants(&mut self) {
        let mut i = qalc_num::Number::new();
        i.set_imaginary_part(&qalc_num::Number::from_i64(1));
        self.set_variable("i", MathStructure::Number(i));
    }

    /// Define or replace a variable.
    pub fn set_variable(&mut self, name: impl Into<String>, value: MathStructure) {
        self.variables.insert(name.into(), value);
    }

    pub fn variable(&self, name: &str) -> Option<&MathStructure> {
        self.variables.get(name)
    }

    /// Evaluate one input line, returning its printed result.
    ///
    /// Handles `name := expr` assignment, which stores the evaluated value
    /// and echoes it (matching the reference CLI).
    pub fn evaluate_line(&mut self, line: &str) -> Result<String, String> {
        let line = line.trim();
        // `/set <option> <value>` — the CLI command set (src/qalc.cc). Only
        // the options the transcripts depend on are honoured; the rest are
        // accepted and ignored so a transcript keeps running.
        if let Some(rest) = line.strip_prefix('/') {
            // `/assume <sign>` is its own command, not a `set` option.
            if let Some(word) = rest.strip_prefix("assume ") {
                if let Some(sign) = crate::assumptions::parse_sign(word.trim()) {
                    crate::assumptions::set_sign(sign);
                }
                return Ok(String::new());
            }
            return Ok(self.set_option(rest));
        }
        // The CLI accepts the same commands without the slash.
        if line.starts_with("set ") {
            return Ok(self.set_option(line));
        }
        // The CLI command words that apply an operation to the rest of the
        // line (`factor x^2-1`, `expand (x+1)^2`) — src/qalc.cc.
        for (word, fid) in [
            ("factor", crate::polynomial::id::FACTORIZE),
            ("factorize", crate::polynomial::id::FACTORIZE),
            ("expand", crate::polynomial::id::EXPAND),
            ("simplify", crate::polynomial::id::EXPAND),
        ] {
            if let Some(rest) = line.strip_prefix(word) {
                let rest = rest.trim_start();
                if rest.len() + word.len() < line.len() && !rest.is_empty() {
                    let value = self.evaluate_expression(rest)?;
                    let mut out = if fid == crate::polynomial::id::FACTORIZE {
                        // The result must not go back through the merge
                        // engine, which would expand the product again.
                        crate::polynomial::factor(&value, &self.eval_options)
                    } else {
                        let mut eo = self.eval_options.clone();
                        eo.expand = 1;
                        let mut v = value;
                        crate::eval::evaluate_calculated_with(&mut v, &eo);
                        v
                    };
                    crate::sort::sort(&mut out);
                    return Ok(print::print(&out, &self.print_options));
                }
            }
        }
        // `delete <name>` removes a user variable.
        if let Some(name) = line.strip_prefix("delete ") {
            self.variables.remove(name.trim());
            return Ok(String::new());
        }
        if let Some((name, expr)) = split_assignment(line) {
            let value = self.evaluate_expression(expr)?;
            let printed = print::print(&value, &self.print_options);
            self.set_variable(name, value);
            return Ok(printed);
        }
        // `name = expr` is an assignment too when the left side is a plain
        // name: the reference CLI stores the value and echoes it (in
        // interactive mode it first asks). Anything else with `=` stays a
        // comparison for the solver.
        if let Some((name, expr)) = self.split_plain_assignment(line) {
            let value = self.evaluate_expression(expr)?;
            let printed = print::print(&value, &self.print_options);
            self.set_variable(name, value);
            return Ok(printed);
        }
        let value = self.evaluate_expression(line)?;
        let mut value = value;
        let mut po = self.print_options.clone();
        crate::eval::apply_conversion(&mut value, &mut po)?;
        Ok(print::print(&value, &po))
    }

    /// Split `name = expr` when `name` is a free identifier — not a unit and
    /// not a function. Comparison forms (`x^2 = 4`, `a == b`, `x <= 1`) are
    /// rejected so the solver still sees them.
    fn split_plain_assignment<'l>(&self, line: &'l str) -> Option<(&'l str, &'l str)> {
        let idx = line.find('=')?;
        if line[idx + 1..].starts_with('=') {
            return None;
        }
        let name = line[..idx].trim();
        let expr = line[idx + 1..].trim();
        if name.is_empty() || expr.is_empty() {
            return None;
        }
        // `<=`, `>=`, `!=` end in `=` but are not assignments.
        if name.ends_with(['<', '>', '!', ':']) {
            return None;
        }
        if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        if builtins::function_id_for_name(name).is_some() {
            return None;
        }
        // A name the unit registry already owns stays a comparison.
        if !self.variables.contains_key(name) {
            if let Some(store) = crate::units::store() {
                if store.resolve_name(name).is_some() {
                    return None;
                }
            }
        }
        Some((name, expr))
    }

    /// Apply a `/set <option> <value>` command. Unknown options are ignored,
    /// like the reference CLI's `set_option` when it cannot match a name.
    fn set_option(&mut self, cmd: &str) -> String {
        let mut words = cmd.split_whitespace();
        if words.next() != Some("set") {
            return String::new();
        }
        let Some(option) = words.next() else {
            return String::new();
        };
        // `set input base 16` and `set base 16` — a two-word option name, so
        // the value is one word further along (`set_option`, src/qalc.cc:1366;
        // the two-word split is at :1377-1443).
        let (option, value) = match option {
            "input" | "output" => (
                match (option, words.next()) {
                    ("input", Some("base")) => "inbase",
                    ("output", Some("base")) => "outbase",
                    _ => return String::new(),
                },
                words.next().unwrap_or("1"),
            ),
            _ => (option, words.next().unwrap_or("1")),
        };
        match option {
            // The input base decides whether A-F are digits and whether `p`
            // is a binary exponent, so it belongs to the parse options.
            "inbase" | "in" => {
                if let Some(b) = parse_base(value) {
                    self.parse_options.base = b;
                }
            }
            "outbase" | "out" => {
                if let Some(b) = parse_base(value) {
                    self.print_options.base = b;
                }
            }
            // Plain `base` sets both, as the reference CLI does.
            "base" => {
                if let Some(b) = parse_base(value) {
                    self.parse_options.base = b;
                    self.print_options.base = b;
                }
            }
            // `set precision N` (src/qalc.cc) — the working decimal precision
            // every numeric result is rounded to.
            "precision" | "prec" => {
                if let Ok(digits) = value.parse::<i32>() {
                    qalc_num::context::set_precision(digits);
                }
            }
            "unicode" => {
                self.print_options.use_unicode_signs = value != "0" && value != "off";
            }
            // `/set approximation exact | try exact | approximate`
            "approximation" | "appr" | "approx" => {
                self.eval_options.approximation = match value {
                    "exact" | "0" => ApproximationMode::Exact,
                    "approximate" | "2" => ApproximationMode::Approximate,
                    _ => ApproximationMode::TryExact,
                };
            }
            // `/set fractions 2` (FRACTION_FRACTIONAL) prints rationals as
            // `1/2` and pulls a fractional coefficient into a division.
            "fractions" | "fr" => {
                self.print_options.number_fraction_format = match value {
                    "2" | "fraction" | "fractional" => NumberFractionFormat::Fractional,
                    "3" | "combined" | "mixed" => NumberFractionFormat::Combined,
                    "1" | "exact" => NumberFractionFormat::DecimalExact,
                    _ => NumberFractionFormat::Decimal,
                };
            }
            _ => {}
        }
        String::new()
    }

    /// Parse and evaluate an expression against this session's variables.
    fn evaluate_expression(&self, expr: &str) -> Result<MathStructure, String> {
        let mut m = self.parse(expr).map_err(|e| e.to_string())?;
        crate::percent::apply(&mut m);
        crate::eval::evaluate_calculated_with(&mut m, &self.eval_options);
        Ok(m)
    }

    fn parse(&self, expr: &str) -> Result<MathStructure, ParseError> {
        parser::parse_with(expr, &self.parse_options, self)
    }
}

impl Session {
    /// The predefined one-letter unknowns of the C++ (`VARIABLE_ID_X`,
    /// `_Y`, `_Z`, `_N`, `_C`).
    const UNKNOWNS: [char; 5] = ['x', 'y', 'z', 'n', 'c'];

    /// Whether `c` is a name the calculator knows on its own.
    fn resolves_alone(&self, c: char) -> bool {
        if Session::UNKNOWNS.contains(&c) {
            return true;
        }
        let s = c.to_string();
        if self.variables.contains_key(&s) {
            return true;
        }
        crate::units::store().is_some_and(|store| store.resolve_name(&s).is_some())
    }

    fn resolve_char(&self, c: char) -> MathStructure {
        let s = c.to_string();
        if let Some(v) = self.variables.get(&s) {
            return v.clone();
        }
        if !Session::UNKNOWNS.contains(&c) {
            if let Some(m) = crate::units::store().and_then(|store| store.resolve_name(&s)) {
                return m;
            }
        }
        MathStructure::symbolic(s)
    }
}

impl NameResolver for Session {
    fn resolve(&self, name: &str) -> Option<MathStructure> {
        // A user assignment shadows everything else, as in the reference.
        if let Some(v) = self.variables.get(name) {
            return Some(v.clone());
        }
        // Then the unit registry, including prefixed forms (`km`, `dm3`).
        if let Some(store) = crate::units::store() {
            if let Some(m) = store.resolve_name(name) {
                return Some(m);
            }
        }
        // The "Special Numbers" builtin variables (data/variables.xml).
        // These have to be values, not symbols: the whole indeterminate-form
        // machinery in `calculate` keys off an infinite `Number` and off
        // `MathStructure::Undefined`, so a symbolic `infinity` would let
        // `0*infinity` collapse to `0`.
        if let Some(m) = special_number(name) {
            return Some(m);
        }
        // `Calculator::parse` matches the longest *known* name at each
        // position, so a name built entirely out of known one-character names
        // splits into a product: `3yx^2` parses as `3*y*x^2` and `abc` as
        // `are*barn*c`. A name with an unknown character (`pi`, `foo`) stays
        // one symbol, which is exactly what keeps `pi` intact.
        if name.len() > 1 && name.chars().all(|c| self.resolves_alone(c)) {
            let parts: Vec<MathStructure> = name
                .chars()
                .map(|c| self.resolve_char(c))
                .collect();
            return Some(MathStructure::Multiplication(parts));
        }
        Some(MathStructure::symbolic(name))
    }

    fn resolve_function(&self, name: &str) -> Option<FunctionId> {
        builtins::function_id_for_name(name)
    }

    /// A name is "known" when something in the registries answers to it, as
    /// opposed to becoming a symbol only because nothing did. The predefined
    /// unknowns count: the C++ holds `x`, `y`, `z`, `n` and `c` as real
    /// variable objects, so its name loop matches them like any other.
    fn is_known_name(&self, name: &str) -> bool {
        if self.variables.contains_key(name) || builtins::function_id_for_name(name).is_some() {
            return true;
        }
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if Session::UNKNOWNS.contains(&c) {
                return true;
            }
        }
        crate::units::store().is_some_and(|store| store.resolve_name(name).is_some())
    }
}

/// The `<builtin_variable>` entries of the "Special Numbers" category that
/// carry a fixed value (`data/variables.xml`). `i` and the rest still need the
/// `DynamicVariable` port, so only the three that are pure `MathStructure`
/// constants are answered here.
fn special_number(name: &str) -> Option<MathStructure> {
    let mut n = qalc_num::Number::new();
    match name {
        "infinity" | "plus_infinity" | "∞" => n.set_plus_infinity(false, false),
        "minus_infinity" => n.set_minus_infinity(false, false),
        "undefined" => return Some(MathStructure::Undefined),
        _ => return None,
    };
    Some(MathStructure::Number(n))
}

/// Split `name := expr`, rejecting `:=` that is not at the top level.
fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(":=")?;
    let (name, rest) = line.split_at(idx);
    let name = name.trim();
    let expr = rest[2..].trim();
    if name.is_empty() || expr.is_empty() {
        return None;
    }
    // The target must be a plain identifier.
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some((name, expr))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "Special Numbers" constants have to arrive as values: a symbolic
    /// `infinity` would be an ordinary unknown, and the merge engine would
    /// then happily reduce `0*infinity` to `0` and `infinity/infinity` to `1`.
    #[test]
    fn special_numbers_resolve_to_values() {
        let mut s = Session::new();
        assert_eq!(s.evaluate_line("infinity").unwrap(), "+infinity");
        assert_eq!(s.evaluate_line("plus_infinity").unwrap(), "+infinity");
        assert_eq!(s.evaluate_line("∞").unwrap(), "+infinity");
        assert_eq!(s.evaluate_line("minus_infinity").unwrap(), "-infinity");
        assert_eq!(s.evaluate_line("undefined").unwrap(), "undefined");
        // ...and behave like values from there on.
        assert_eq!(s.evaluate_line("infinity + 1").unwrap(), "+infinity");
        assert_eq!(s.evaluate_line("2 * infinity").unwrap(), "+infinity");
        assert_eq!(s.evaluate_line("1 / infinity").unwrap(), "0");
        // A user assignment still shadows them, as in the reference.
        assert_eq!(s.evaluate_line("infinity := 7").unwrap(), "7");
        assert_eq!(s.evaluate_line("infinity + 1").unwrap(), "8");
    }

    /// The variables.batch transcript, verified against the reference.
    #[test]
    fn assignment_transcript() {
        let mut s = Session::new();
        assert_eq!(s.evaluate_line("alpha := 5").unwrap(), "5");
        assert_eq!(s.evaluate_line("alpha").unwrap(), "5");
        assert_eq!(s.evaluate_line("beta := 2+1").unwrap(), "3");
        assert_eq!(s.evaluate_line("beta").unwrap(), "3");
        assert_eq!(s.evaluate_line("alpha + beta").unwrap(), "8");
        // Reassignment reads the previous values.
        assert_eq!(s.evaluate_line("alpha:= alpha + beta").unwrap(), "8");
        assert_eq!(s.evaluate_line("alpha:= alpha + beta").unwrap(), "11");
        assert_eq!(s.evaluate_line("alpha").unwrap(), "11");
        assert_eq!(s.evaluate_line("alpha^2 + 3beta").unwrap(), "130");
    }

    #[test]
    fn vector_valued_variable() {
        let mut s = Session::new();
        assert_eq!(s.evaluate_line("beta:=[1,2,3]").unwrap(), "[1  2  3]");
    }

    #[test]
    fn unassigned_names_stay_symbolic() {
        let mut s = Session::new();
        assert_eq!(s.evaluate_line("x + x").unwrap(), "2x");
    }

    #[test]
    fn assignment_target_must_be_an_identifier() {
        assert!(split_assignment("a := 1").is_some());
        assert!(split_assignment("1 := 2").is_none());
        assert!(split_assignment("a + b := 2").is_none());
        assert!(split_assignment(":= 2").is_none());
    }

    #[test]
    fn conversion_still_works_through_a_session() {
        let mut s = Session::new();
        assert_eq!(s.evaluate_line("52 to hex").unwrap(), "0x34");
    }
}

/// The value of a `set base` command: a plain number, or one of the named
/// bases the reference CLI accepts.
fn parse_base(value: &str) -> Option<i32> {
    use qalc_num::options::base;
    if let Ok(n) = value.parse::<i32>() {
        return Some(n);
    }
    Some(match value.to_ascii_lowercase().as_str() {
        "bin" | "binary" => 2,
        "oct" | "octal" => 8,
        "dec" | "decimal" => 10,
        "hex" | "hexadecimal" => 16,
        "duo" | "duodecimal" => 12,
        "roman" => base::ROMAN_NUMERALS,
        "sexa" | "sexagesimal" => base::SEXAGESIMAL,
        "time" => base::TIME,
        _ => return None,
    })
}
