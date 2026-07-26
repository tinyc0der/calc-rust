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
use crate::structure::{ConversionTarget, MathStructure};
use qalc_num::{ParseOptions, PrintOptions};

/// A calculator session.
pub struct Session {
    /// User-assigned variables, by name.
    variables: HashMap<String, MathStructure>,
    pub parse_options: ParseOptions,
    pub print_options: PrintOptions,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Session {
            variables: HashMap::new(),
            parse_options: ParseOptions::default(),
            print_options: crate::eval::batch_print_options(),
        }
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
        if let Some((name, expr)) = split_assignment(line) {
            let value = self.evaluate_expression(expr)?;
            let printed = print::print(&value, &self.print_options);
            self.set_variable(name, value);
            return Ok(printed);
        }
        let value = self.evaluate_expression(line)?;
        let mut value = value;
        let mut po = self.print_options.clone();
        apply_conversion(&mut value, &mut po)?;
        Ok(print::print(&value, &po))
    }

    /// Parse and evaluate an expression against this session's variables.
    fn evaluate_expression(&self, expr: &str) -> Result<MathStructure, String> {
        let mut m = self.parse(expr).map_err(|e| e.to_string())?;
        crate::percent::apply(&mut m);
        crate::eval::evaluate_calculated(&mut m);
        Ok(m)
    }

    fn parse(&self, expr: &str) -> Result<MathStructure, ParseError> {
        parser::parse_with(expr, &self.parse_options, self)
    }
}

impl NameResolver for Session {
    fn resolve(&self, name: &str) -> Option<MathStructure> {
        // A user assignment shadows everything else, as in the reference.
        if let Some(v) = self.variables.get(name) {
            return Some(v.clone());
        }
        Some(MathStructure::symbolic(name))
    }

    fn resolve_function(&self, name: &str) -> Option<FunctionId> {
        builtins::function_id_for_name(name)
    }
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

/// Fold an outer `to <base>` conversion into the print options.
fn apply_conversion(m: &mut MathStructure, po: &mut PrintOptions) -> Result<(), String> {
    let MathStructure::Conversion { value, target } = m else {
        return Ok(());
    };
    match target {
        ConversionTarget::NumberBase { base, bits } => {
            po.base = *base;
            po.binary_bits = *bits;
        }
        ConversionTarget::Base(expr) => {
            let mut b = (**expr).clone();
            crate::eval::evaluate(&mut b);
            match &b {
                MathStructure::Number(n) => match n.to_i64() {
                    Some(v) if (2..=36).contains(&v) => po.base = v as i32,
                    _ => return Err("unsupported number base".to_string()),
                },
                _ => return Err("number base must evaluate to a number".to_string()),
            }
        }
        ConversionTarget::Unit(_) => {
            // TODO(port): unit conversion needs the unit registry.
            return Err("unit conversion is not implemented yet".to_string());
        }
    }
    *m = (**value).clone();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
