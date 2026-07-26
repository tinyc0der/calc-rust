//! Expression parser — port of `Calculator::parseOperators`
//! (Calculator-parse.cc:4290).
//!
//! The C++ implementation is a string-rewriting parser: it repeatedly finds
//! the innermost parentheses, parses the contents, stores the result in a
//! side table and substitutes an internal id marker, then splits the
//! remaining string at each operator class from lowest to highest
//! precedence. This port keeps the *same precedence order and associativity*
//! but implements it as recursive descent over a token stream, which is
//! equivalent for well-formed input and far cheaper.
//!
//! Precedence, loosest to tightest (the order `parseOperators` splits in):
//!   1. `&&`  logical and
//!   2. `||`  logical or
//!   3. `|`   bitwise or
//!   4. `xor` bitwise xor
//!   5. `&`   bitwise and
//!   6. comparisons `= != < > <= >=`
//!   7. `<<` `>>` shifts
//!   8. `+` `-`
//!   9. `*` `/` `mod` `%%` `\`
//!  10. implicit multiplication (`2x`, `5 km`)
//!  11. `^` (right-associative)
//!  12. unary `-` `+` `~` `!`, postfix `!`, primaries
//!
//! Adaptive-mode subtlety verified against the reference binary: division
//! followed by an unspaced implicit product binds the whole product into the
//! denominator (`1/2x` → `1/(2x)`), but a space breaks it (`1/2 x` → `0.5x`).

use crate::ids::FunctionId;
use crate::lexer::{tokenize, Tok, Token};
use crate::structure::{ComparisonType, ConversionTarget, MathStructure};
use qalc_num::{Number, ParseOptions};

/// A parse failure with the byte offset where it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub pos: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.pos)
    }
}

impl std::error::Error for ParseError {}

/// Resolves identifiers to variables, units and functions.
///
/// The C++ parser consults the `CALCULATOR` singleton's `ufv` name buckets;
/// threading a resolver keeps `qalc-core` free of global state. Until the
/// registries are ported, [`SymbolicResolver`] turns every name into a
/// symbol.
pub trait NameResolver {
    /// Resolve `name` to a structure (variable, unit, or constant).
    fn resolve(&self, name: &str) -> Option<MathStructure>;
    /// Resolve `name` to a function id, if it names a function.
    fn resolve_function(&self, name: &str) -> Option<FunctionId>;
}

/// Fallback resolver: every identifier becomes a symbol, every call becomes
/// an unresolved function reference.
pub struct SymbolicResolver;

impl NameResolver for SymbolicResolver {
    fn resolve(&self, name: &str) -> Option<MathStructure> {
        Some(MathStructure::symbolic(name))
    }
    fn resolve_function(&self, name: &str) -> Option<FunctionId> {
        crate::builtins::function_id_for_name(name)
    }
}

/// Parse `expr` into a `MathStructure` using `resolver` for names.
pub fn parse_with(
    expr: &str,
    po: &ParseOptions,
    resolver: &dyn NameResolver,
) -> Result<MathStructure, ParseError> {
    let tokens = tokenize(expr);
    let mut p = Parser {
        toks: tokens,
        i: 0,
        po: po.clone(),
        resolver,
        abs_depth: 0,
    };
    let m = p.parse_expression()?;
    p.expect_eof()?;
    Ok(m)
}

/// Parse `expr` with every name left symbolic.
pub fn parse(expr: &str, po: &ParseOptions) -> Result<MathStructure, ParseError> {
    parse_with(expr, po, &SymbolicResolver)
}

struct Parser<'a> {
    toks: Vec<Token>,
    i: usize,
    po: ParseOptions,
    resolver: &'a dyn NameResolver,
    /// Nesting depth inside `|…|` absolute-value bars. While non-zero, `|`
    /// terminates the operand instead of acting as bitwise or — the C++
    /// decides this up front by scanning for the `b_abs_or` pattern
    /// (Calculator-parse.cc:4440).
    abs_depth: u32,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn peek_token(&self) -> &Token {
        &self.toks[self.i]
    }

    fn pos(&self) -> usize {
        self.toks[self.i].pos
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i].tok.clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            message: msg.into(),
            pos: self.pos(),
        })
    }

    fn expect_eof(&self) -> Result<(), ParseError> {
        if *self.peek() == Tok::Eof {
            Ok(())
        } else {
            Err(ParseError {
                message: format!("unexpected token {:?}", self.peek()),
                pos: self.pos(),
            })
        }
    }

    // ---------------------------------------------------------------
    // Precedence ladder
    // ---------------------------------------------------------------

    fn parse_expression(&mut self) -> Result<MathStructure, ParseError> {
        let left = self.parse_logical_and()?;
        // `expr to <target>` — the conversion operator binds loosest of all
        // (`Calculator::separateToExpression` splits it off before parsing).
        if *self.peek() == Tok::To {
            self.bump();
            let target = self.parse_conversion_target()?;
            return Ok(MathStructure::Conversion {
                value: Box::new(left),
                target,
            });
        }
        Ok(left)
    }

    /// Parse what follows `to`: a base name, `base N`, or a unit expression.
    fn parse_conversion_target(&mut self) -> Result<ConversionTarget, ParseError> {
        if let Tok::Ident(name) = self.peek().clone() {
            let lower = name.to_ascii_lowercase();
            // `to base N`
            if lower == "base" {
                self.bump();
                let m = self.parse_logical_and()?;
                return Ok(ConversionTarget::Base(Box::new(m)));
            }
            if let Some(t) = base_target_from_name(&lower) {
                self.bump();
                return Ok(t);
            }
        }
        // Anything else is a unit expression.
        let m = self.parse_logical_and()?;
        Ok(ConversionTarget::Unit(Box::new(m)))
    }

    fn parse_logical_and(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_logical_or()?;
        while *self.peek() == Tok::LogicalAnd {
            self.bump();
            let right = self.parse_logical_or()?;
            left = MathStructure::LogicalAnd(vec![left, right]);
        }
        Ok(left)
    }

    fn parse_logical_or(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_bit_or()?;
        while *self.peek() == Tok::LogicalOr {
            self.bump();
            let right = self.parse_bit_or()?;
            left = MathStructure::LogicalOr(vec![left, right]);
        }
        Ok(left)
    }

    fn parse_bit_or(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_bit_xor()?;
        while *self.peek() == Tok::BitOr && self.abs_depth == 0 {
            self.bump();
            let right = self.parse_bit_xor()?;
            left = MathStructure::BitwiseOr(vec![left, right]);
        }
        Ok(left)
    }

    fn parse_bit_xor(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_bit_and()?;
        while *self.peek() == Tok::BitXor {
            self.bump();
            let right = self.parse_bit_and()?;
            left = MathStructure::BitwiseXor(vec![left, right]);
        }
        Ok(left)
    }

    fn parse_bit_and(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_comparison()?;
        while *self.peek() == Tok::BitAnd {
            self.bump();
            let right = self.parse_comparison()?;
            left = MathStructure::BitwiseAnd(vec![left, right]);
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_shift()?;
        loop {
            let op = match self.peek() {
                Tok::Equals => ComparisonType::Equals,
                Tok::NotEquals => ComparisonType::NotEquals,
                Tok::Less => ComparisonType::Less,
                Tok::Greater => ComparisonType::Greater,
                Tok::LessEquals => ComparisonType::EqualsLess,
                Tok::GreaterEquals => ComparisonType::EqualsGreater,
                _ => break,
            };
            self.bump();
            let right = self.parse_shift()?;
            left = MathStructure::Comparison {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let f = match self.peek() {
                Tok::ShiftLeft => BuiltinOp::ShiftLeft,
                Tok::ShiftRight => BuiltinOp::ShiftRight,
                _ => break,
            };
            self.bump();
            let right = self.parse_additive()?;
            left = builtin_call(f, vec![left, right]);
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<MathStructure, ParseError> {
        let mut terms = vec![self.parse_multiplicative()?];
        loop {
            match self.peek() {
                Tok::Plus => {
                    self.bump();
                    terms.push(self.parse_multiplicative()?);
                }
                Tok::Minus => {
                    self.bump();
                    let mut t = self.parse_multiplicative()?;
                    t.negate();
                    terms.push(t);
                }
                Tok::PlusMinus => {
                    self.bump();
                    let right = self.parse_multiplicative()?;
                    let left = if terms.len() == 1 {
                        terms.pop().unwrap()
                    } else {
                        MathStructure::Addition(std::mem::take(&mut terms))
                    };
                    terms.push(builtin_call(BuiltinOp::Uncertainty, vec![left, right]));
                }
                _ => break,
            }
        }
        Ok(if terms.len() == 1 {
            terms.pop().unwrap()
        } else {
            MathStructure::Addition(terms)
        })
    }

    /// Explicit `*`, `/`, `mod`, `%%`, `\` — left-associative.
    fn parse_multiplicative(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_implicit_product()?;
        loop {
            match self.peek() {
                Tok::Times | Tok::ElementTimes => {
                    self.bump();
                    let right = self.parse_implicit_product()?;
                    left = match left {
                        MathStructure::Multiplication(mut v) => {
                            v.push(right);
                            MathStructure::Multiplication(v)
                        }
                        other => MathStructure::Multiplication(vec![other, right]),
                    };
                }
                Tok::Divide => {
                    self.bump();
                    // Adaptive mode: an unspaced implicit product after `/`
                    // goes wholly into the denominator (`1/2x` = `1/(2x)`),
                    // but `1/2 x` divides first.
                    let mut denom = self.parse_implicit_product()?;
                    denom.inverse();
                    // C++ `divide(o, append)` appends into an existing
                    // multiplication (MathStructure.cc:1620), so `6/3/2`
                    // yields a flat Multiplication[6, 3^-1, 2^-1].
                    left = match left {
                        MathStructure::Multiplication(mut v) => {
                            v.push(denom);
                            MathStructure::Multiplication(v)
                        }
                        other => MathStructure::Multiplication(vec![other, denom]),
                    };
                }
                Tok::IntDivide => {
                    self.bump();
                    let right = self.parse_implicit_product()?;
                    left = builtin_call(BuiltinOp::IntDivide, vec![left, right]);
                }
                Tok::Mod => {
                    self.bump();
                    let right = self.parse_implicit_product()?;
                    left = builtin_call(BuiltinOp::Mod, vec![left, right]);
                }
                Tok::Rem => {
                    self.bump();
                    let right = self.parse_implicit_product()?;
                    left = builtin_call(BuiltinOp::Rem, vec![left, right]);
                }
                // `%` is binary mod when an operand follows (`6%2` = 0) and
                // postfix percent otherwise (`50%` = 0.5) — both verified
                // against the reference binary.
                Tok::Percent if self.percent_is_binary() => {
                    self.bump();
                    let right = self.parse_implicit_product()?;
                    left = builtin_call(BuiltinOp::Rem, vec![left, right]);
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// Implicit multiplication: adjacent primaries with no operator between
    /// them (`2x`, `5 km`, `2(3+4)`). Binds tighter than explicit `*` and `/`.
    fn parse_implicit_product(&mut self) -> Result<MathStructure, ParseError> {
        let first = self.parse_power()?;
        let mut factors = vec![first];
        while self.starts_implicit_factor() {
            factors.push(self.parse_power()?);
        }
        Ok(if factors.len() == 1 {
            factors.pop().unwrap()
        } else {
            MathStructure::Multiplication(factors)
        })
    }

    /// Is the `%` at the cursor a binary modulo rather than postfix percent?
    /// True when an operand follows it.
    fn percent_is_binary(&self) -> bool {
        matches!(
            self.toks.get(self.i + 1).map(|t| &t.tok),
            Some(Tok::Number(_) | Tok::Ident(_) | Tok::LParen)
        )
    }

    /// Can the current token begin an implicitly-multiplied factor?
    fn starts_implicit_factor(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Number(_) | Tok::Ident(_) | Tok::LParen | Tok::LBracket | Tok::LBrace
        )
    }

    /// `^` — right-associative, binds tighter than implicit multiplication.
    fn parse_power(&mut self) -> Result<MathStructure, ParseError> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Tok::Power | Tok::ElementPower) {
            self.bump();
            // Right-associative: 2^3^2 = 2^(3^2). The exponent is parsed at
            // unary level so `2^-1` works and `2^3x` is `(2^3)x`.
            let exp = self.parse_power()?;
            let mut m = base;
            m.raise(exp);
            return Ok(m);
        }
        Ok(base)
    }

    fn parse_unary(&mut self) -> Result<MathStructure, ParseError> {
        match self.peek().clone() {
            Tok::Minus => {
                self.bump();
                let mut m = self.parse_unary()?;
                // `-2^2` is `-(2^2)`: bind the power before negating.
                if matches!(self.peek(), Tok::Power | Tok::ElementPower) {
                    self.bump();
                    let exp = self.parse_power()?;
                    m.raise(exp);
                }
                m.negate();
                Ok(m)
            }
            Tok::Plus => {
                self.bump();
                self.parse_unary()
            }
            Tok::BitNot => {
                self.bump();
                let m = self.parse_unary()?;
                Ok(MathStructure::BitwiseNot(Box::new(m)))
            }
            Tok::LogicalNot => {
                self.bump();
                let m = self.parse_unary()?;
                Ok(MathStructure::LogicalNot(Box::new(m)))
            }
            _ => self.parse_postfix(),
        }
    }

    /// Postfix `!` (factorial), `!!` (double factorial), `%` (percent).
    fn parse_postfix(&mut self) -> Result<MathStructure, ParseError> {
        let mut m = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::LogicalNot => {
                    // Postfix position: factorial. `n!!` is double factorial.
                    self.bump();
                    if *self.peek() == Tok::LogicalNot {
                        self.bump();
                        m = builtin_call(BuiltinOp::DoubleFactorial, vec![m]);
                    } else {
                        m = builtin_call(BuiltinOp::Factorial, vec![m]);
                    }
                }
                // Postfix percent only when no operand follows; otherwise
                // this `%` is binary modulo and belongs to the caller.
                Tok::Percent if !self.percent_is_binary() => {
                    self.bump();
                    // Kept as a marker call rather than an immediate
                    // multiplication by 1/100, because a percent term in a
                    // sum means "of the running total" (`100 + 10%` = 110).
                    m = builtin_call(BuiltinOp::Percent, vec![m]);
                }
                _ => break,
            }
        }
        Ok(m)
    }

    fn parse_primary(&mut self) -> Result<MathStructure, ParseError> {
        let tok = self.peek_token().clone();
        match tok.tok {
            Tok::Number(ref s) => {
                self.bump();
                // Digit grouping: `1 000 000` and `2 3` join into one number
                // when spaced groups of digits follow (libqalculate treats a
                // space as a thousands separator).
                let mut text = s.clone();
                while let Tok::Number(next) = self.peek().clone() {
                    if self.peek_token().space_before && next.chars().all(|c| c.is_ascii_digit()) {
                        self.bump();
                        text.push_str(&next);
                    } else {
                        break;
                    }
                }
                Ok(MathStructure::Number(Number::parse(&text, &self.po)))
            }
            Tok::Ident(ref name) => {
                self.bump();
                // Function call: identifier immediately followed by `(`.
                if *self.peek() == Tok::LParen && !self.peek_token().space_before {
                    if let Some(fid) = self.resolver.resolve_function(name) {
                        let args = self.parse_call_args()?;
                        return Ok(MathStructure::Function { id: fid, args });
                    }
                }
                match self.resolver.resolve(name) {
                    Some(m) => Ok(m),
                    None => self.err(format!("unknown name `{name}`")),
                }
            }
            Tok::Str(ref s) => {
                self.bump();
                Ok(MathStructure::symbolic(s.clone()))
            }
            Tok::LParen => {
                self.bump();
                if self.eat(&Tok::RParen) {
                    // "Empty expression in parentheses interpreted as zero."
                    return Ok(MathStructure::Number(Number::new()));
                }
                let m = self.parse_expression()?;
                if !self.eat(&Tok::RParen) {
                    // The C++ appends a missing right parenthesis rather than
                    // failing; mirror that leniency.
                    return Ok(m);
                }
                Ok(m)
            }
            Tok::LBracket | Tok::LBrace => {
                let close = if tok.tok == Tok::LBracket {
                    Tok::RBracket
                } else {
                    Tok::RBrace
                };
                self.bump();
                let mut items = Vec::new();
                if self.peek() != &close {
                    loop {
                        items.push(self.parse_expression()?);
                        if !self.eat(&Tok::Comma) && !self.eat(&Tok::Semicolon) {
                            break;
                        }
                    }
                }
                self.eat(&close);
                Ok(MathStructure::Vector(items))
            }
            Tok::BitOr => {
                // |x| absolute value.
                self.bump();
                self.abs_depth += 1;
                let m = self.parse_expression();
                self.abs_depth -= 1;
                let m = m?;
                self.eat(&Tok::BitOr);
                Ok(builtin_call(BuiltinOp::Abs, vec![m]))
            }
            // Word operators double as function names when called:
            // `mod(x, 2)` alongside `x mod 2`.
            Tok::Mod | Tok::Rem if self.toks.get(self.i + 1).map(|t| &t.tok) == Some(&Tok::LParen) => {
                let name = if tok.tok == Tok::Mod { "mod" } else { "rem" };
                self.bump();
                let fid = self
                    .resolver
                    .resolve_function(name)
                    .unwrap_or_else(|| BuiltinOp::Mod.function_id());
                let args = self.parse_call_args()?;
                Ok(MathStructure::Function { id: fid, args })
            }
            Tok::Eof => self.err("unexpected end of expression"),
            ref t => self.err(format!("unexpected token {t:?}")),
        }
    }

    fn parse_call_args(&mut self) -> Result<Vec<MathStructure>, ParseError> {
        self.bump(); // consume '('
        let mut args = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                args.push(self.parse_expression()?);
                // libqalculate accepts both `,` and `;` as argument separators.
                if !self.eat(&Tok::Comma) && !self.eat(&Tok::Semicolon) {
                    break;
                }
            }
        }
        self.eat(&Tok::RParen);
        Ok(args)
    }
}

/// Recognize a base name after `to`: `bin`, `bin16`, `oct`, `hex`, `dec`,
/// `roman`, `sexa`, `float`, and the `binN`/`hexN` bit-width forms.
fn base_target_from_name(lower: &str) -> Option<ConversionTarget> {
    use qalc_num::options::base;
    // `binN` / `hexN` carry an explicit bit width.
    for (prefix, b) in [("bin", 2i32), ("hex", 16i32), ("oct", 8i32)] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if rest.is_empty() {
                return Some(ConversionTarget::NumberBase { base: b, bits: 0 });
            }
            if let Ok(bits) = rest.parse::<u32>() {
                return Some(ConversionTarget::NumberBase { base: b, bits });
            }
            return None;
        }
    }
    let b = match lower {
        "binary" => 2,
        "octal" => 8,
        "dec" | "decimal" => 10,
        "duo" | "duodecimal" => 12,
        "hexadecimal" => 16,
        "roman" => base::ROMAN_NUMERALS,
        "sexa" | "sexagesimal" => base::SEXAGESIMAL,
        "time" => base::TIME,
        "float" => base::FP32,
        "double" => base::FP64,
        _ => return None,
    };
    Some(ConversionTarget::NumberBase { base: b, bits: 0 })
}

/// Builtin operations the parser desugars into function calls. Real
/// `FunctionId`s arrive with the registry port; these placeholder ids match
/// the C++ `FUNCTION_ID_*` values so the mapping stays stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinOp {
    Abs,
    Factorial,
    DoubleFactorial,
    Mod,
    Rem,
    IntDivide,
    ShiftLeft,
    ShiftRight,
    Uncertainty,
    Percent,
}

impl BuiltinOp {
    /// The stable `FUNCTION_ID_*` value from BuiltinFunctions.h.
    pub fn function_id(self) -> FunctionId {
        FunctionId(match self {
            BuiltinOp::Abs => 1400,
            BuiltinOp::Factorial => 1500,
            BuiltinOp::DoubleFactorial => 1501,
            BuiltinOp::Mod => 1700,
            BuiltinOp::Rem => 1701,
            BuiltinOp::IntDivide => 1702,
            BuiltinOp::ShiftLeft => 1703,
            BuiltinOp::ShiftRight => 1704,
            BuiltinOp::Uncertainty => 1705,
            BuiltinOp::Percent => 1720,
        })
    }
}

fn builtin_call(op: BuiltinOp, args: Vec<MathStructure>) -> MathStructure {
    MathStructure::Function {
        id: op.function_id(),
        args,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> MathStructure {
        parse(s, &ParseOptions::default()).expect("parse")
    }

    fn render(m: &MathStructure) -> String {
        format!("{m}")
    }

    #[test]
    fn arithmetic_precedence() {
        // 1+2*3 → Addition[1, Multiplication[2,3]]
        let m = p("1+2*3");
        assert!(m.is_addition(), "got {}", render(&m));
        assert_eq!(m.size(), 2);
        assert!(m.get(1).unwrap().is_multiplication());
    }

    #[test]
    fn subtraction_negates() {
        // 5-3 → Addition[5, Multiplication[-1, 3]]
        let m = p("5-3");
        assert!(m.is_addition());
        assert!(m.get(1).unwrap().is_multiplication());
        assert!(m.get(1).unwrap().get(0).unwrap().is_minus_one());
    }

    #[test]
    fn power_is_right_associative() {
        // 2^3^2 = 2^(3^2), verified as 512 by the reference binary.
        let m = p("2^3^2");
        assert!(m.is_power());
        assert!(m.get(1).unwrap().is_power(), "exponent nests: {}", render(&m));
    }

    #[test]
    fn unary_minus_binds_looser_than_power() {
        // -2^2 = -(2^2) = -4 per the reference binary.
        let m = p("-2^2");
        assert!(m.is_multiplication(), "got {}", render(&m));
        assert!(m.get(0).unwrap().is_minus_one());
        assert!(m.get(1).unwrap().is_power());
    }

    #[test]
    fn implicit_multiplication_binds_tighter_than_division() {
        // 1/2x → 1/(2x): the multiplication ends up inside the inverse.
        let m = p("1/2x");
        assert!(m.is_multiplication(), "got {}", render(&m));
        // Second factor is (2x)^-1
        let second = m.get(1).unwrap();
        assert!(second.is_power(), "denominator is a power: {}", render(&m));
        assert!(
            second.get(0).unwrap().is_multiplication(),
            "denominator base is the implicit product 2x: {}",
            render(&m)
        );
    }

    #[test]
    fn implicit_grouping_of_parenthesized_factor() {
        // 8/2(2+2) = 1 per the reference binary — the (2+2) joins the
        // denominator.
        let m = p("8/2(2+2)");
        let second = m.get(1).unwrap();
        assert!(second.is_power());
        assert!(second.get(0).unwrap().is_multiplication());
        assert_eq!(second.get(0).unwrap().size(), 2);
    }

    #[test]
    fn division_is_left_associative() {
        // 6/3/2 = 1
        let m = p("6/3/2");
        assert!(m.is_multiplication());
        assert_eq!(m.size(), 3, "flat product with two inverses: {}", render(&m));
    }

    #[test]
    fn digit_grouping_joins_spaced_numbers() {
        // "2 3" is the number 23 per the reference binary.
        let m = p("2 3");
        assert!(m.is_number(), "got {}", render(&m));
        assert!(m.number().unwrap().equals_i64(23));
        let m2 = p("1 000 000");
        assert!(m2.number().unwrap().equals_i64(1_000_000));
    }

    #[test]
    fn vectors_and_matrices() {
        let m = p("[1, 2, 3]");
        assert!(m.is_vector());
        assert_eq!(m.size(), 3);
        let m2 = p("[[1,2],[3,4]]");
        assert_eq!(m2.size(), 2);
        assert!(m2.get(0).unwrap().is_vector());
    }

    #[test]
    fn comparisons() {
        let m = p("x = 5");
        assert!(matches!(m, MathStructure::Comparison { .. }));
        let m2 = p("1 < 2");
        assert!(matches!(
            m2,
            MathStructure::Comparison {
                op: ComparisonType::Less,
                ..
            }
        ));
    }

    #[test]
    fn logical_and_bitwise() {
        let m = p("1 && 0");
        assert!(matches!(m, MathStructure::LogicalAnd(_)));
        let m2 = p("0b1011 | 0b0101");
        assert!(matches!(m2, MathStructure::BitwiseOr(_)));
        let m3 = p("~1");
        assert!(matches!(m3, MathStructure::BitwiseNot(_)));
    }

    #[test]
    fn shifts_are_function_calls() {
        let m = p("18 >> 2");
        assert!(matches!(m, MathStructure::Function { .. }));
        if let MathStructure::Function { id, args } = &m {
            assert_eq!(*id, BuiltinOp::ShiftRight.function_id());
            assert_eq!(args.len(), 2);
        }
    }

    #[test]
    fn factorial_postfix() {
        let m = p("5!");
        assert!(matches!(m, MathStructure::Function { .. }));
        if let MathStructure::Function { id, .. } = &m {
            assert_eq!(*id, BuiltinOp::Factorial.function_id());
        }
        let m2 = p("5!!");
        if let MathStructure::Function { id, .. } = &m2 {
            assert_eq!(*id, BuiltinOp::DoubleFactorial.function_id());
        }
    }

    #[test]
    fn percent_parses_as_a_marker() {
        // The division by 100 is deferred: inside a sum a percent means
        // "of the running total" (see the `percent` module), so the parser
        // records the intent rather than the arithmetic.
        let m = p("50%");
        assert!(matches!(m, MathStructure::Function { .. }), "got {m}");
        if let MathStructure::Function { id, args } = &m {
            assert_eq!(*id, BuiltinOp::Percent.function_id());
            assert_eq!(args.len(), 1);
        }
    }

    #[test]
    fn absolute_value_bars() {
        let m = p("|-5|");
        assert!(matches!(m, MathStructure::Function { .. }));
        if let MathStructure::Function { id, .. } = &m {
            assert_eq!(*id, BuiltinOp::Abs.function_id());
        }
    }

    #[test]
    fn exact_number_literals() {
        let m = p("0.1");
        // Exact rational 1/10, not a binary float.
        assert!(m.number().unwrap().is_rational());
        let m2 = p("0xFF");
        assert!(m2.number().unwrap().equals_i64(255));
    }

    #[test]
    fn unicode_operator_forms() {
        let m = p("2×3");
        assert!(m.is_multiplication());
        let m2 = p("2−1");
        assert!(m2.is_addition());
    }

    #[test]
    fn empty_parens_is_zero() {
        let m = p("()");
        assert!(m.is_number() && m.number().unwrap().is_zero());
    }

    #[test]
    fn errors_have_positions() {
        let e = parse("1+", &ParseOptions::default()).unwrap_err();
        assert_eq!(e.pos, 2);
    }
}
