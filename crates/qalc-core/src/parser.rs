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
use crate::lexer::{Tok, Token};
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

/// How deeply the recursive-descent ladder may nest before the parser gives
/// up with a [`ParseError`] instead of running the stack out.
///
/// The C++ parser rewrites strings and keeps its pending groups on the heap,
/// so it happily swallows tens of thousands of nested parentheses. This port
/// descends the precedence ladder recursively, which trades that headroom for
/// speed: one `(…)` level costs about eighteen frames (~9 KB in release,
/// ~50 KB in a debug build), so a few thousand parentheses overflow an 8 MiB
/// stack and abort the process — a crash no `Result` can catch.
///
/// The counter is bumped at the three points where the ladder can re-enter
/// itself — [`Parser::parse_expression`] (grouping: `(`, `[`, `|…|`, call
/// arguments), [`Parser::parse_power`] (right-associative `^`) and
/// [`Parser::parse_unary`] (leading `-`/`+`/`~`/`!`) — so a parenthesis level
/// counts three, a `^` level two and a sign one.
///
/// 300 therefore allows ~100 nested parentheses, ~150 nested exponents and
/// 300 leading signs. Reaching the limit costs ~0.9 MB of stack in release
/// and ~5 MB in a debug build, both of which fit the 8 MiB main thread with
/// room left for the evaluation pass over the tree that was just built.
pub const MAX_PARSE_DEPTH: u32 = 300;

thread_local! {
    /// Current nesting depth. A thread-local rather than a `Parser` field
    /// because `[…]` elements and text arguments are re-parsed by a *fresh*
    /// `Parser` ([`Parser::parse_sub`]), which would otherwise restart the
    /// count at every bracket level.
    static PARSE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Holds one level of [`PARSE_DEPTH`], releasing it on every exit path
/// (including the `?` of a parse error further down).
struct DepthGuard;

impl DepthGuard {
    fn enter() -> Option<DepthGuard> {
        PARSE_DEPTH.with(|d| {
            let depth = d.get();
            if depth >= MAX_PARSE_DEPTH {
                return None;
            }
            d.set(depth + 1);
            Some(DepthGuard)
        })
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        PARSE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Resolves identifiers to variables, units and functions.
///
/// The C++ parser consults the `CALCULATOR` singleton's `ufv` name buckets;
/// threading a resolver keeps `qalc-core` free of global state.
/// [`crate::session::Session`] is the real implementation, backed by the unit
/// and definition registries. [`SymbolicResolver`] — every name becomes a
/// symbol — is what bare [`parse`] uses, for callers that only want a tree.
pub trait NameResolver {
    /// Resolve `name` to a structure (variable, unit, or constant).
    fn resolve(&self, name: &str) -> Option<MathStructure>;
    /// Resolve `name` to a function id, if it names a function.
    fn resolve_function(&self, name: &str) -> Option<FunctionId>;
    /// Whether the C++ `ufv` name-matching loop would *claim* `name` — i.e.
    /// it is a real unit, variable or function, rather than a symbol the
    /// fallback invents for text nobody recognised.
    ///
    /// Only [`Parser::magnitude_suffix`] needs to tell the two apart. The
    /// default is `true`, so a resolver that cannot make the distinction
    /// never lets a trailing letter be eaten as a multiplier.
    fn is_known_name(&self, _name: &str) -> bool {
        true
    }
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
    let tokens = crate::lexer::tokenize_with_base(expr, po.base);
    let mut p = Parser {
        src: expr,
        toks: tokens,
        i: 0,
        po: po.clone(),
        resolver,
        abs_depth: 0,
    };
    // A comma (or, outside brackets, a semicolon) at the top level builds a
    // vector: `Calculator::parse` wraps the enclosing parenthesis group in
    // `vector()` (Calculator-parse.cc:3930).
    let m = p.parse_vector_list(&[Tok::Eof])?;
    p.expect_eof()?;
    Ok(m)
}

/// Parse `expr` with every name left symbolic.
pub fn parse(expr: &str, po: &ParseOptions) -> Result<MathStructure, ParseError> {
    parse_with(expr, po, &SymbolicResolver)
}

struct Parser<'a> {
    /// The source text, needed by the `[…]` parser: libqalculate splits a
    /// bracket group into element *substrings* and re-parses each one, which
    /// is what makes `[1 2]` two elements while `1 2` is the number 12.
    src: &'a str,
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

    /// Take one level of recursion budget, or fail with a [`ParseError`].
    ///
    /// The returned guard must be kept alive for the whole of the recursive
    /// call it protects; dropping it gives the level back.
    fn enter(&self) -> Result<DepthGuard, ParseError> {
        match DepthGuard::enter() {
            Some(g) => Ok(g),
            None => Err(ParseError {
                message: format!("expression nested deeper than {MAX_PARSE_DEPTH} levels"),
                pos: self.pos(),
            }),
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

    /// A comma/semicolon separated list terminated by one of `enders`.
    ///
    /// Outside `[…]` libqalculate rewrites `;` to `,` (Calculator-parse.cc:1388)
    /// and then wraps every remaining comma group in `vector()`
    /// (Calculator-parse.cc:3930), so `1,2`, `(1;2)` and `(1,)` are all
    /// vectors. An empty element parses as zero.
    fn parse_vector_list(&mut self, enders: &[Tok]) -> Result<MathStructure, ParseError> {
        let ends = |p: &Self| enders.iter().any(|e| p.peek() == e);
        let element = |p: &mut Self| -> Result<MathStructure, ParseError> {
            if ends(p) || matches!(p.peek(), Tok::Comma | Tok::Semicolon) {
                return Ok(MathStructure::Number(Number::new()));
            }
            p.parse_expression()
        };
        let first = element(self)?;
        if !matches!(self.peek(), Tok::Comma | Tok::Semicolon) {
            return Ok(first);
        }
        let mut items = vec![first];
        while matches!(self.peek(), Tok::Comma | Tok::Semicolon) {
            self.bump();
            items.push(element(self)?);
        }
        Ok(MathStructure::Vector(items))
    }

    fn parse_expression(&mut self) -> Result<MathStructure, ParseError> {
        // Every grouping construct — `(…)`, `[…]`, `|…|`, a call argument —
        // re-enters the ladder here, so this is where nesting is counted.
        let _depth = self.enter()?;
        let left = self.parse_dot()?;
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
        // `to -unit` turns mixed-unit output off, `to +unit` forces it; the
        // C++ strips the sign from the "to" string before looking up the unit
        // (Calculator-convert.cc:2294).
        let mut mix = true;
        if *self.peek() == Tok::Minus && self.next_starts_unit() {
            self.bump();
            mix = false;
        }
        // `?unit` / `b?unit`: an automatic output prefix. The marker is part
        // of the identifier token, so rewrite the token and keep parsing.
        let mut prefix = crate::units::PrefixMode::None;
        if let Tok::Ident(name) = self.peek().clone() {
            if let Some(q) = name.find('?') {
                let head = &name[..q];
                let tail = &name[q + 1..];
                if !tail.is_empty() {
                    prefix = if head.eq_ignore_ascii_case("b") {
                        crate::units::PrefixMode::Binary
                    } else {
                        crate::units::PrefixMode::Decimal
                    };
                    self.toks[self.i].tok = Tok::Ident(tail.to_string());
                }
            }
        }
        if let Tok::Ident(name) = self.peek().clone() {
            let lower = name.to_ascii_lowercase();
            // `to base N`, and bare `to base` = expand to base units.
            if lower == "base" {
                self.bump();
                // Bare `to base` expands to base units; anything after it is
                // the number base (`to base 16`, `to base sqrt(2)`).
                if *self.peek() == Tok::Eof {
                    return Ok(ConversionTarget::BaseUnits);
                }
                let m = self.parse_logical_and()?;
                return Ok(ConversionTarget::Base(Box::new(m)));
            }
            if let Some(t) = self.time_zone_target(&lower) {
                return Ok(t);
            }
            if let Some(t) = base_target_from_name(&lower) {
                self.bump();
                return Ok(t);
            }
        }
        // Anything else is a unit expression.
        let m = self.parse_logical_and()?;
        Ok(ConversionTarget::Unit {
            expr: Box::new(m),
            mix,
            prefix,
        })
    }

    /// `to utc`, `to gmt`, `to utc+8`, `to utc-05:30`
    /// (Calculator-calculate.cc:2818).
    ///
    /// The C++ reads the offset with `sscanf("%2u:%2u")`, so `utc+05:30` is
    /// five and a half hours. Here the colon has already made `05:30` a single
    /// sexagesimal number worth 5.5, and multiplying by 60 lands on the same
    /// 330 minutes.
    fn time_zone_target(&mut self, lower: &str) -> Option<ConversionTarget> {
        if lower != "utc" && lower != "gmt" {
            return None;
        }
        self.bump();
        let sign = match self.peek() {
            Tok::Plus => 1,
            Tok::Minus => -1,
            _ => return Some(ConversionTarget::TimeZone { offset_minutes: None }),
        };
        let Some(Tok::Number(text)) = self.toks.get(self.i + 1).map(|t| t.tok.clone()) else {
            return Some(ConversionTarget::TimeZone { offset_minutes: None });
        };
        let mut hours = qalc_num::Number::parse(&text, &ParseOptions::default());
        hours.multiply_i64(60);
        hours.round(qalc_num::options::RoundingMode::HalfAwayFromZero);
        let Some(minutes) = hours.to_i64() else {
            return Some(ConversionTarget::TimeZone { offset_minutes: None });
        };
        self.bump();
        self.bump();
        Some(ConversionTarget::TimeZone {
            offset_minutes: Some((sign * minutes) as i32),
        })
    }

    /// Is the token after a `to`-leading `-` a plain unit name?
    fn next_starts_unit(&self) -> bool {
        matches!(
            self.toks.get(self.i + 1).map(|t| &t.tok),
            Some(Tok::Ident(_))
        )
    }

    /// The `.` dot-product operator (`Calculator::parseAdd`'s internal
    /// `\x16`, Calculator-parse.cc:5985). It binds looser than `+`/`-`.
    fn parse_dot(&mut self) -> Result<MathStructure, ParseError> {
        let mut left = self.parse_logical_and()?;
        while *self.peek() == Tok::Dot {
            self.bump();
            let right = self.parse_logical_and()?;
            left = MathStructure::Function {
                id: FunctionId(crate::matrix::id::DOT_PRODUCT),
                args: vec![left, right],
            };
        }
        Ok(left)
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
                Tok::Times => {
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
                // `.*` and `./` are the entrywise product/quotient functions,
                // not plain multiplication: they broadcast row against column
                // vectors (`[1; 2].*[3 4]` = `[3  4; 6  8]`).
                Tok::ElementTimes | Tok::ElementDivide => {
                    let div = *self.peek() == Tok::ElementDivide;
                    self.bump();
                    let right = self.parse_implicit_product()?;
                    let fid = if div {
                        crate::matrix::id::ENTRYWISE_DIVISION
                    } else {
                        crate::matrix::id::ENTRYWISE_MULTIPLICATION
                    };
                    left = MathStructure::Function {
                        id: FunctionId(fid),
                        args: vec![left, right],
                    };
                }
                Tok::Divide => {
                    // Adaptive mode is whitespace-sensitive on *both* sides of
                    // the operator. Verified against the reference binary:
                    //   `1/2x`    -> 1/(2x)      (nothing spaced)
                    //   `1/2 x`   -> 0.5x        (only the product is spaced)
                    //   `1 / 2 x` -> 1/(2x)      (the operator is spaced too)
                    // So the denominator only stops at a space when the
                    // division sign itself is unspaced.
                    let denom_stops_at_space = !self.peek_token().space_before;
                    self.bump();
                    // Adaptive mode: an unspaced implicit product after `/`
                    // goes wholly into the denominator (`1/2x` = `1/(2x)`),
                    // but `1/2 x` divides first.
                    let mut denom = self.parse_implicit_product_opt(denom_stops_at_space)?;
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
                    // The space ended the denominator, so whatever follows
                    // multiplies the quotient: `1/2 x` is `0.5x` and
                    // `10 N / 5 Pa` is `(10/5) N*Pa^-1`.
                    if self.starts_implicit_factor() {
                        let extra = self.parse_implicit_product()?;
                        left = match (left, extra) {
                            (MathStructure::Multiplication(mut v), MathStructure::Multiplication(w)) => {
                                v.extend(w);
                                MathStructure::Multiplication(v)
                            }
                            (MathStructure::Multiplication(mut v), other) => {
                                v.push(other);
                                MathStructure::Multiplication(v)
                            }
                            (l, r) => MathStructure::Multiplication(vec![l, r]),
                        };
                    }
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
        self.parse_implicit_product_opt(false)
    }

    /// Parse an implicit product. With `stop_at_space`, the product ends at
    /// the first factor preceded by whitespace — this is what makes the
    /// denominator of a division whitespace-sensitive: `1/2x` is `1/(2x)`
    /// but `1/2 x` is `(1/2)x`, and `8/2(2+2)` is 1 while `8/2 (2+2)` is 16.
    /// The flag never applies to the first factor, so `20 miles / 2h` still
    /// puts `2h` in the denominator.
    fn parse_implicit_product_opt(
        &mut self,
        stop_at_space: bool,
    ) -> Result<MathStructure, ParseError> {
        let first = self.parse_binary_exponent()?;
        let mut factors = vec![first];
        while self.starts_implicit_factor() {
            if stop_at_space && self.peek_token().space_before {
                break;
            }
            factors.push(self.parse_binary_exponent()?);
        }
        if factors.len() == 1 {
            return Ok(factors.pop().unwrap());
        }
        // A run of quantities in decreasing units is a sum: `10h 31min` is
        // 10 h + 31 min, not 310 h*min.
        if let Some(terms) = crate::units::mixed_unit_sum(&factors) {
            return Ok(MathStructure::Addition(terms));
        }
        Ok(MathStructure::Multiplication(factors))
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
            Tok::Number(_)
                | Tok::Ident(_)
                | Tok::LParen
                | Tok::LBracket
                | Tok::LBrace
                | Tok::BinExp
        )
    }

    /// `p` in base 16 — the binary exponent of `Calculator::parseOperators`
    /// (Calculator-parse.cc:6322). `AEp-2` is 174*2^-2; the exponent is always
    /// read in base 10, and a `p` with nothing before it takes a mantissa of
    /// one, so a bare `p23` is 2^23.
    fn parse_binary_exponent(&mut self) -> Result<MathStructure, ParseError> {
        let mut m = if *self.peek() == Tok::BinExp {
            MathStructure::from(1)
        } else {
            self.parse_power()?
        };
        while *self.peek() == Tok::BinExp {
            self.bump();
            let mut negative = false;
            loop {
                match self.peek() {
                    Tok::Minus => {
                        negative = !negative;
                        self.bump();
                    }
                    Tok::Plus => {
                        self.bump();
                    }
                    _ => break,
                }
            }
            let Tok::Number(text) = self.peek().clone() else {
                return self.err("a binary exponent needs a number");
            };
            self.bump();
            let mut exponent =
                qalc_num::Number::parse(&text, &ParseOptions::default());
            if negative {
                exponent.negate();
            }
            let mut power = MathStructure::from(2);
            power.raise(MathStructure::Number(exponent));
            m = MathStructure::Multiplication(vec![m, power]);
        }
        Ok(m)
    }

    /// `^` — right-associative, binds tighter than implicit multiplication.
    fn parse_power(&mut self) -> Result<MathStructure, ParseError> {
        // Right-associative, so `2^2^2^…` recurses once per operator.
        let _depth = self.enter()?;
        let base = self.parse_unary()?;
        if *self.peek() == Tok::ElementPower {
            self.bump();
            let exp = self.parse_power()?;
            return Ok(MathStructure::Function {
                id: FunctionId(crate::matrix::id::ENTRYWISE_POWER),
                args: vec![base, exp],
            });
        }
        if *self.peek() == Tok::Power {
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
        // A run of leading signs (`----…1`) recurses once per sign.
        let _depth = self.enter()?;
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
                // Postfix `.'` — matrix transpose (Calculator-parse.cc:6268).
                Tok::Transpose => {
                    self.bump();
                    m = MathStructure::Function {
                        id: FunctionId(crate::matrix::id::TRANSPOSE),
                        args: vec![m],
                    };
                }
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

    /// The `k`/`M` magnitude suffix of `Calculator::parseNumber`
    /// (Calculator-parse.cc:4120), returning the multiplier it stands for.
    ///
    /// This is *not* the SI prefix machinery — `M` here means "million", not
    /// "mega", and `m` never means "milli". It is the last-resort branch of
    /// the number parser: a token that is otherwise a decimal literal but
    /// carries one stray trailing character normally raises "trailing
    /// characters … were ignored" and drops it; when that single character is
    /// `k`/`K` it instead scales by 10^3, and `m`/`M` by 10^6.
    ///
    /// Being a last resort is the whole rule. The name-matching loop runs
    /// first and claims every known unit, variable and prefix+unit pair, so
    /// the suffix only ever sees letters nothing else wanted:
    ///
    /// * `11k` -> 11000, `2M` -> 2000000 — nothing is named `k` or `M`.
    /// * `2m` -> 2 m, `11K` -> 11 K — `m` is the metre and `K` the kelvin,
    ///   and matched names win outright.
    /// * `11km`, `2kg`, `11kB`, `11kK` — the prefix+unit name wins, which is
    ///   why a prefix letter is never a bare multiplier in front of a unit.
    /// * `k` alone is *not* 1000: with no digits before it the C++ reports
    ///   "not a valid variable/function/unit" instead (Calculator-parse.cc:4112).
    /// * `11kk`, `2f`, `2Z` — more than one leftover character, or a letter
    ///   that is not `k`/`m`, still takes the "ignored" branch.
    /// * Base 10 only, so `0x11k` keeps `k` as a separate (unknown) name.
    fn magnitude_suffix(&self, number_text: &str) -> Option<i64> {
        if self.po.base != 10 {
            return None;
        }
        // `0x…`/`0b…` literals and time forms like `10:31` are not the plain
        // decimal literals this branch of the C++ number parser handles.
        if number_text.contains(':')
            || (number_text.len() > 1
                && number_text.starts_with('0')
                && !number_text.as_bytes()[1].is_ascii_digit()
                && number_text.as_bytes()[1] != b'.')
        {
            return None;
        }
        let Tok::Ident(name) = self.peek() else {
            return None;
        };
        let factor = match name.as_str() {
            "k" | "K" => 1_000,
            "m" | "M" => 1_000_000,
            _ => return None,
        };
        // The letter has to be the last character of the numeric token, so a
        // digit glued straight onto it (`11k5`, `11k.5`) disqualifies it.
        if matches!(self.toks.get(self.i + 1), Some(t) if !t.space_before && matches!(t.tok, Tok::Number(_)))
        {
            return None;
        }
        if self.resolver.is_known_name(name) {
            return None;
        }
        Some(factor)
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
                let mut n = Number::parse(&text, &self.po);
                if let Some(factor) = self.magnitude_suffix(&text) {
                    self.bump();
                    n.multiply(&Number::from_i64(factor));
                }
                Ok(MathStructure::Number(n))
            }
            Tok::Ident(ref name) => {
                self.bump();
                // Function call: identifier immediately followed by `(`.
                if *self.peek() == Tok::LParen && !self.peek_token().space_before {
                    if let Some(fid) = self.resolver.resolve_function(name) {
                        // Functions with `TextArgument`s see their source
                        // text, not a parsed expression.
                        if crate::strings::has_text_args(fid.0)
                            || crate::datetime::has_date_args(fid.0)
                        {
                            return self.parse_text_call(fid);
                        }
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
                Ok(MathStructure::Text(s.clone()))
            }
            Tok::LParen | Tok::LBrace => {
                let close = if tok.tok == Tok::LParen {
                    Tok::RParen
                } else {
                    Tok::RBrace
                };
                self.bump();
                if self.eat(&close) {
                    // "Empty expression in parentheses interpreted as zero."
                    return Ok(MathStructure::Number(Number::new()));
                }
                let m = self.parse_vector_list(&[close.clone(), Tok::Eof])?;
                // The C++ appends a missing right parenthesis rather than
                // failing; mirror that leniency.
                self.eat(&close);
                Ok(m)
            }
            Tok::LBracket => self.parse_bracket_vector(),
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
            // A `.` with nothing to its left is a misplaced operator, which
            // the C++ drops with a warning — leaving the empty expression 0.
            Tok::Dot => {
                self.bump();
                Ok(MathStructure::Number(Number::new()))
            }
            Tok::Eof => self.err("unexpected end of expression"),
            ref t => self.err(format!("unexpected token {t:?}")),
        }
    }

    /// `[…]` — the matlab-style matrix / vector literal
    /// (Calculator-parse.cc:2056).
    ///
    /// libqalculate first decides whether the group is written in the *old*
    /// nested style (`[[1,2],[4,5]]`) or the matlab style (`[1 2; 4 5]`). In
    /// matlab style `;` separates rows and `,` — or, when the group contains
    /// no comma at all, a space — separates columns. Elements are then
    /// re-parsed as independent substrings, which is why a space inside
    /// brackets is a separator while `1 2` on its own is the number 12.
    fn parse_bracket_vector(&mut self) -> Result<MathStructure, ParseError> {
        let open = self.peek_token().pos;
        let close = matching_bracket(self.src, open);
        let content = &self.src[open + 1..close];
        // Skip the tokens the substring parse consumes.
        self.i = self
            .toks
            .iter()
            .position(|t| t.pos > close)
            .unwrap_or(self.toks.len() - 1);

        let (b_comma, old_style) = analyse_bracket(content);
        let base = open + 1;
        if old_style {
            let parts = split_top_level(content, |c| c == b',' || c == b';');
            let mut items = Vec::with_capacity(parts.len());
            for (s, e) in parts {
                items.push(self.parse_sub(&content[s..e], base + s)?);
            }
            return Ok(MathStructure::Vector(items));
        }
        self.parse_matlab_matrix(content, base, b_comma)
    }

    /// The matlab-style body of [`Self::parse_bracket_vector`].
    fn parse_matlab_matrix(
        &mut self,
        content: &str,
        base: usize,
        b_comma: bool,
    ) -> Result<MathStructure, ParseError> {
        let b: Vec<u8> = content.bytes().collect();
        let n = b.len();
        let mut result: Vec<MathStructure> = Vec::new();
        let mut row: Option<Vec<MathStructure>> = None;
        let mut col_index = 0usize;
        let mut brackets = 0i32;
        let mut pars = 0i32;
        let (mut cit1, mut cit2) = (false, false);

        let mut i = 0usize;
        while i <= n {
            let mut b_row = false;
            let mut b_col = false;
            if i == n {
                b_row = true;
                b_col = true;
            } else {
                match b[i] {
                    b'[' if !cit1 && !cit2 => brackets += 1,
                    b']' if !cit1 && !cit2 && brackets > 0 => brackets -= 1,
                    b'(' if !cit1 && !cit2 && brackets == 0 => pars += 1,
                    b')' if !cit1 && !cit2 && brackets == 0 && pars > 0 => pars -= 1,
                    b'"' if !cit2 => cit1 = !cit1,
                    b'\'' if !cit1 => cit2 = !cit2,
                    b';' if brackets == 0 && pars == 0 && !cit1 && !cit2 => {
                        b_row = true;
                        b_col = true;
                    }
                    b',' if brackets == 0 && pars == 0 && !cit1 && !cit2 => b_col = true,
                    b' ' if !b_comma && brackets == 0 && pars == 0 && !cit1 && !cit2 => {
                        b_col = space_is_separator(&b, i);
                    }
                    _ => {}
                }
            }
            if b_col {
                let text = content[col_index..i].trim();
                let offset = col_index + content[col_index..i].len()
                    - content[col_index..i].trim_start().len();
                let mcol = if b_comma || !text.is_empty() {
                    Some(self.parse_sub(text, base + offset)?)
                } else {
                    None
                };
                col_index = i + 1;
                let had_row = row.is_some();
                if let Some(r) = row.as_mut() {
                    if let Some(c) = mcol {
                        r.push(c);
                    }
                    if b_row {
                        result.push(MathStructure::Vector(row.take().expect("row")));
                    }
                } else if let Some(c) = mcol {
                    result.push(c);
                }
                if i < n && b_row {
                    if !had_row {
                        // The first `;` turns the columns collected so far
                        // into the matrix's first row.
                        result = vec![MathStructure::Vector(std::mem::take(&mut result))];
                    }
                    row = Some(Vec::new());
                }
            }
            i += 1;
        }
        // `while(mstruct2->isVector() && mstruct2->size() == 1) setToChild(1)`
        let mut m = MathStructure::Vector(result);
        while m.is_vector() && m.size() == 1 {
            m = match m {
                MathStructure::Vector(mut v) => v.remove(0),
                _ => unreachable!(),
            };
        }
        Ok(m)
    }

    /// Parse an element substring of a `[…]` group, reporting errors at the
    /// substring's offset in the original source.
    fn parse_sub(&self, text: &str, offset: usize) -> Result<MathStructure, ParseError> {
        if text.trim().is_empty() {
            return Ok(MathStructure::Number(Number::new()));
        }
        parse_with(text, &self.po, self.resolver).map_err(|e| ParseError {
            message: e.message,
            pos: offset + e.pos,
        })
    }

    /// A call whose arguments are (partly) `TextArgument`s, which
    /// `Argument::parse` (Function.cc:1614) takes from the *source text*
    /// rather than from a parsed expression. See [`crate::strings`] for the
    /// rule; the short version is that a fragment with no parenthesis and no
    /// quoted literal is kept verbatim, unless it names a text variable.
    fn parse_text_call(&mut self, fid: FunctionId) -> Result<MathStructure, ParseError> {
        let open = self.peek_token().pos;
        let close = matching_paren(self.src, open);
        let content = &self.src[open + 1..close.min(self.src.len())];
        // Skip past the tokens the raw slice covers.
        self.i = self
            .toks
            .iter()
            .position(|t| t.pos > close)
            .unwrap_or(self.toks.len() - 1);

        let mut args = Vec::new();
        if !content.trim().is_empty()
            || crate::strings::is_text_arg(fid.0, 0)
            || crate::datetime::is_date_arg(fid.0, 0)
        {
            for (index, (s, e)) in split_top_level(content, |c| c == b',' || c == b';')
                .into_iter()
                .enumerate()
            {
                let raw = &content[s..e];
                let text = raw.trim();
                if crate::datetime::is_date_arg(fid.0, index) {
                    // A date argument reads its source fragment as a date;
                    // anything that is not date-shaped falls through to the
                    // ordinary expression parse.
                    match crate::datetime::parse_date(text) {
                        Some(d) => args.push(crate::datetime::date_structure(d)),
                        None => {
                            let offset = s + raw.len() - raw.trim_start().len();
                            args.push(self.parse_sub(text, offset)?);
                        }
                    }
                } else if crate::strings::is_text_arg(fid.0, index) {
                    args.push(self.parse_text_arg(text, s)?);
                } else {
                    let offset = s + raw.len() - raw.trim_start().len();
                    args.push(self.parse_sub(text, offset)?);
                }
            }
        }
        Ok(MathStructure::Function { id: fid, args })
    }

    /// One `TextArgument`, from its source fragment.
    fn parse_text_arg(&self, text: &str, offset: usize) -> Result<MathStructure, ParseError> {
        if text.is_empty() {
            return Ok(MathStructure::Text(String::new()));
        }
        // A fragment holding a call or a quoted literal is parsed normally.
        if text.contains('(') || text.contains('"') || text.contains('\'') {
            return self.parse_sub(text, offset);
        }
        // A bare name bound to a text value resolves to that value; every
        // other fragment is kept verbatim. Only plain identifiers are looked
        // up — `getActiveVariable` in the C++ never sees anything else.
        let is_name = text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if is_name {
            if let Some(m @ MathStructure::Text(_)) = self.resolver.resolve(text) {
                return Ok(m);
            }
        }
        Ok(MathStructure::Text(text.to_string()))
    }

    fn parse_call_args(&mut self) -> Result<Vec<MathStructure>, ParseError> {
        self.bump(); // consume '('
        let mut args = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                // `MathFunction::args` splits the argument string on commas
                // and parses each part, so an omitted argument is zero
                // (`vector(,)` = `[0  0]`).
                if matches!(self.peek(), Tok::Comma | Tok::Semicolon | Tok::RParen | Tok::Eof) {
                    args.push(MathStructure::Number(Number::new()));
                } else {
                    args.push(self.parse_expression()?);
                }
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

/// The characters libqalculate treats as operators when deciding whether a
/// space inside `[…]` separates columns (`OPERATORS` in includes.h:921 plus
/// the internal `%`).
const BRACKET_OPERATORS: &[u8] = b"~+-*/^&|!<>=%";

/// Byte offset of the `]` matching the `[` at `open`, or `src.len()`.
fn matching_bracket(src: &str, open: usize) -> usize {
    let b = src.as_bytes();
    let mut depth = 0i32;
    let (mut cit1, mut cit2) = (false, false);
    for i in open..b.len() {
        match b[i] {
            b'"' if !cit2 => cit1 = !cit1,
            b'\'' if !cit1 => cit2 = !cit2,
            b'[' if !cit1 && !cit2 => depth += 1,
            b']' if !cit1 && !cit2 => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    src.len()
}

/// Byte index of the `)` matching the `(` at `open`, ignoring quoted text.
fn matching_paren(src: &str, open: usize) -> usize {
    let b = src.as_bytes();
    let mut depth = 0i32;
    let (mut cit1, mut cit2) = (false, false);
    for i in open..b.len() {
        match b[i] {
            b'"' if !cit2 => cit1 = !cit1,
            b'\'' if !cit1 => cit2 = !cit2,
            b'(' if !cit1 && !cit2 => depth += 1,
            b')' if !cit1 && !cit2 => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    src.len()
}

/// Is the space at `i` a column separator? Port of the guards in the matlab
/// branch (Calculator-parse.cc:2219): a space adjacent to an operator
/// belongs to that operator's expression rather than separating columns.
fn space_is_separator(b: &[u8], i: usize) -> bool {
    if i == 0 || i + 1 >= b.len() {
        return false;
    }
    let prev = b[i - 1];
    if prev == b';' || (BRACKET_OPERATORS.contains(&prev) && prev != b'!') {
        return false;
    }
    let next = b[i + 1];
    if BRACKET_OPERATORS.contains(&next) {
        if next == b'+' || next == b'-' {
            if i + 2 >= b.len() || b[i + 2] == b' ' {
                return false;
            }
        } else if next != b'~' && next != b'!' {
            return false;
        }
    }
    true
}

/// Split `content` at top-level (no brackets, parentheses or quotes)
/// separators, returning byte ranges.
fn split_top_level(content: &str, is_sep: impl Fn(u8) -> bool) -> Vec<(usize, usize)> {
    let b = content.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let (mut cit1, mut cit2) = (false, false);
    for i in 0..b.len() {
        match b[i] {
            b'"' if !cit2 => cit1 = !cit1,
            b'\'' if !cit1 => cit2 = !cit2,
            b'[' | b'(' if !cit1 && !cit2 => depth += 1,
            b']' | b')' if !cit1 && !cit2 && depth > 0 => depth -= 1,
            c if depth == 0 && !cit1 && !cit2 && is_sep(c) => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push((start, b.len()));
    out
}

/// Classify a `[…]` group: `(contains a top-level comma, is old nested
/// style)`. Port of the `matlab_matrices` pre-scan (Calculator-parse.cc:2062).
fn analyse_bracket(content: &str) -> (bool, bool) {
    let b: Vec<u8> = content.bytes().collect();
    let n = b.len();
    let mut b_old_matrix = -1i32;
    let mut b_comma = false;
    let mut brackets = 1i32;
    let mut pars = 0i32;
    let (mut cit1, mut cit2) = (false, false);
    let is_num = |c: u8| c.is_ascii_digit() || c == b'.';
    let mut i = 0usize;
    while i < n && brackets > 0 && (b_old_matrix != 0 || !b_comma) {
        match b[i] {
            b'"' if !cit2 => cit1 = !cit1,
            b'\'' if !cit1 => cit2 = !cit2,
            b'[' if !cit1 && !cit2 => brackets += 1,
            b']' if !cit1 && !cit2 && brackets > 0 => {
                if b_old_matrix != 0 && brackets == 2 {
                    // A closing inner bracket followed (possibly across a
                    // separator) by another `[` means old nested style.
                    let mut j = i + 1;
                    while j < n && b[j] == b' ' {
                        j += 1;
                    }
                    if j < n && (b[j] == b',' || b[j] == b';') {
                        j += 1;
                        while j < n && b[j] == b' ' {
                            j += 1;
                        }
                    }
                    let nc = if j < n { b[j] } else { b']' };
                    if nc == b'[' {
                        b_old_matrix = 1;
                    } else if nc != b']' {
                        b_old_matrix = 0;
                    }
                }
                brackets -= 1;
            }
            b'(' if !cit1 && !cit2 && brackets == 1 => pars += 1,
            b')' if !cit1 && !cit2 && brackets == 1 && pars > 0 => pars -= 1,
            b' ' if !cit1 && !cit2 && pars == 0 && b_old_matrix != 0 => {
                let prev = if i > 0 { b[i - 1] } else { b'[' };
                let next = if i + 1 < n { b[i + 1] } else { b']' };
                let positional = (brackets == 1 && i != 0)
                    || (brackets == 2 && is_num(prev) && is_num(next));
                if positional
                    && prev != b','
                    && prev != b';'
                    && next != b','
                    && next != b';'
                {
                    b_old_matrix = 0;
                }
            }
            c @ (b',' | b';') if !cit1 && !cit2 && brackets == 1 && pars == 0 => {
                if c == b',' {
                    b_comma = true;
                }
                let prev = b[..i].iter().rposition(|&x| x != b' ').map(|p| b[p]);
                if prev != Some(b']') {
                    b_old_matrix = 0;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (b_comma, b_old_matrix >= 1)
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
        "unicode" => base::UNICODE,
        "roman" => base::ROMAN_NUMERALS,
        "sexa" | "sexagesimal" => base::SEXAGESIMAL,
        "time" => base::TIME,
        "float" => base::FP32,
        "double" => base::FP64,
        _ => return None,
    };
    Some(ConversionTarget::NumberBase { base: b, bits: 0 })
}

/// Builtin operations the parser desugars into function calls. The ids are the
/// C++ `FUNCTION_ID_*` values, which is what lets [`crate::builtins`] dispatch
/// them directly.
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

    /// Run `f` on a thread with a stack big enough for the deepest nesting
    /// the parser accepts.
    ///
    /// Reaching [`MAX_PARSE_DEPTH`] costs about 0.9 MB of stack in release and
    /// about 5 MB in a debug build — more than the 2 MiB the test harness
    /// gives a test thread, and these tests run in debug.
    fn with_deep_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(f)
            .expect("spawn")
            .join()
            .expect("no stack overflow, and no panic");
    }

    /// Each of these aborted the process with a fatal stack overflow: the
    /// ladder is one mutually-recursive cycle that descends ~18 frames per
    /// `(`, and the abort happens at parse time, so no `Result` could catch
    /// it. They must now come back as ordinary parse errors.
    #[test]
    fn nesting_past_the_limit_is_an_error_not_a_stack_overflow() {
        with_deep_stack(|| {
            let cases = [
                "(".repeat(2000) + "1" + &")".repeat(2000),
                "[".repeat(500) + "1" + &"]".repeat(500),
                "-".repeat(100_000) + "1",
                "sin(".repeat(200) + "x" + &")".repeat(200),
                "2^".repeat(2000) + "2",
            ];
            for expr in cases {
                let err = parse(&expr, &ParseOptions::default())
                    .expect_err("nesting past the limit is refused");
                assert!(
                    err.message.contains("nested deeper"),
                    "expected a depth error, got {err}"
                );
            }
        });
    }

    /// The guard is released on the way back out, so nesting that fits keeps
    /// parsing — including the same expression repeatedly, which would fail if
    /// the depth counter leaked across a parse.
    #[test]
    fn nesting_within_the_limit_still_parses() {
        with_deep_stack(|| {
            let expr = "(".repeat(90) + "1+2" + &")".repeat(90);
            for _ in 0..3 {
                assert_eq!(render(&p(&expr)), "(1 + 2)");
            }
            // A parse that failed on depth must give its levels back too.
            assert!(parse(&"(".repeat(5000), &ParseOptions::default()).is_err());
            assert_eq!(render(&p(&expr)), "(1 + 2)");
        });
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

    /// The `k`/`M` magnitude suffix of `Calculator::parseNumber`. Every
    /// expectation below is the reference binary's own answer.
    mod magnitude_suffix {
        use crate::Session;

        fn ev(s: &str) -> String {
            Session::new().evaluate_line(s).expect("evaluates")
        }

        #[test]
        fn a_stray_k_or_m_after_a_number_scales_it() {
            assert_eq!(ev("11k"), "11000");
            assert_eq!(ev("1.5k"), "1500");
            assert_eq!(ev("2M"), "2000000");
            // Whitespace is removed before the number is parsed.
            assert_eq!(ev("11 k"), "11000");
            assert_eq!(ev("2 M"), "2000000");
            // `M` is "million" here, not the SI prefix "mega" — proof this
            // is the number parser and not the prefix machinery.
            assert_eq!(ev("11 M"), "11000000");
        }

        #[test]
        fn the_suffix_belongs_to_the_number_not_the_expression() {
            assert_eq!(ev("11k + 1"), "11001");
            assert_eq!(ev("x*11k"), "11000x");
            // (2*1000)^2, not 2*(1000^2).
            assert_eq!(ev("2k^2"), "4000000");
            assert_eq!(ev("11k%"), "110");
        }

        #[test]
        fn a_bare_letter_is_never_a_multiplier() {
            // With no digits in front the C++ reports "k is not a valid
            // variable/function/unit"; this port leaves it a symbol. What
            // matters is that it is not 1000.
            assert_eq!(ev("k"), "k");
            assert_eq!(ev("k + 1"), "k + 1");
            assert_eq!(ev("2*k"), "2k");
        }

        #[test]
        fn a_matched_name_beats_the_suffix() {
            // `m` is the metre and `K` the kelvin, so the name loop claims
            // them and the number parser never sees a stray character.
            assert_eq!(ev("2m"), "2 m");
            assert_eq!(ev("11 m"), "11 m");
            assert_eq!(ev("11K"), "11 K");
            assert_eq!(ev("2d"), "172800 s");
        }

        #[test]
        fn a_prefixed_unit_beats_the_suffix() {
            // The reason a bare prefix must not multiply: `k` in front of a
            // unit is part of that unit's name.
            assert_eq!(ev("11km"), "11 km");
            assert_eq!(ev("2kg"), "2 kg");
            assert_eq!(ev("11kB"), "11 kB");
            assert_eq!(ev("2kK"), "2 kK");
        }

        #[test]
        fn only_a_single_trailing_letter_in_base_ten_qualifies() {
            // More than one leftover character takes the C++'s "trailing
            // characters ... were ignored" branch instead.
            assert_eq!(ev("11kk"), "11 kk");
            assert_eq!(ev("11kj"), "11 kj");
            // Other prefix letters are not magnitude suffixes at all.
            assert_eq!(ev("2f"), "2f");
            assert_eq!(ev("2Z"), "2Z");
            // Base 10 only.
            assert_eq!(ev("0x11k"), "17");
        }

        #[test]
        fn a_user_variable_shadows_the_suffix() {
            let mut s = Session::new();
            s.evaluate_line("k := 7").expect("assigns");
            assert_eq!(s.evaluate_line("11k").expect("evaluates"), "77");
        }
    }

    #[test]
    fn errors_have_positions() {
        let e = parse("1+", &ParseOptions::default()).unwrap_err();
        assert_eq!(e.pos, 2);
    }
}
