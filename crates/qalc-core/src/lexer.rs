//! Tokenizer for qalc expressions.
//!
//! Mirrors the character classes and Unicode operator aliases handled by
//! `Calculator::parseSigns` (Calculator-parse.cc:458) before the C++ parser
//! rewrites the string. Whitespace is recorded per-token because
//! libqalculate's adaptive parsing mode is whitespace-sensitive: `1/2x`
//! parses as `1/(2x)` while `1/2 x` parses as `(1/2)x`.

/// A lexical token.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A numeric literal, in its original text form (parsed by `Number::set`).
    Number(String),
    /// An identifier: variable, unit or function name.
    Ident(String),
    /// A quoted string literal.
    Str(String),
    Plus,
    Minus,
    Times,
    Divide,
    /// `\` integer division (`//` also maps here).
    IntDivide,
    Mod,
    /// `%%` remainder.
    Rem,
    Power,
    /// `.^` element-wise power.
    ElementPower,
    /// `.*` element-wise multiply / dot product.
    ElementTimes,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Colon,
    Equals,
    NotEquals,
    Less,
    Greater,
    LessEquals,
    GreaterEquals,
    /// `:=` assignment.
    Assign,
    LogicalAnd,
    LogicalOr,
    LogicalNot,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    ShiftLeft,
    ShiftRight,
    /// `|` when used as absolute-value delimiter is resolved by the parser.
    Percent,
    /// `+/-` or `±` uncertainty operator.
    PlusMinus,
    /// `->` or `to` conversion operator.
    To,
    /// `where` clause keyword.
    Where,
    /// End of input.
    Eof,
}

/// A token plus its source position and whether whitespace preceded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    /// Byte offset of the token start in the source string.
    pub pos: usize,
    /// True if one or more whitespace characters immediately precede it.
    pub space_before: bool,
}

/// Tokenize `src`. Unknown characters become part of identifiers so that
/// unit/variable names with unusual characters survive to name resolution.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    // Byte offsets for each char index, so `pos` refers to the source string.
    let mut offsets = Vec::with_capacity(chars.len() + 1);
    {
        let mut b = 0usize;
        for c in &chars {
            offsets.push(b);
            b += c.len_utf8();
        }
        offsets.push(b);
    }
    let mut i = 0usize;
    let mut space_before = false;
    while i < chars.len() {
        let c = chars[i];
        if is_space(c) {
            space_before = true;
            i += 1;
            continue;
        }
        let pos = offsets[i];
        let start = i;
        let tok = if c.is_ascii_digit() || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
            let s = lex_number(&chars, &mut i);
            Tok::Number(s)
        } else if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != quote {
                s.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            Tok::Str(s)
        } else if is_ident_start(c) {
            let mut s = String::new();
            while i < chars.len() && is_ident_char(chars[i]) {
                s.push(chars[i]);
                i += 1;
            }
            match s.as_str() {
                "to" => Tok::To,
                "where" => Tok::Where,
                "mod" => Tok::Mod,
                "rem" => Tok::Rem,
                "xor" => Tok::BitXor,
                "and" => Tok::LogicalAnd,
                "or" => Tok::LogicalOr,
                "not" => Tok::LogicalNot,
                "plus" => Tok::Plus,
                "minus" => Tok::Minus,
                "times" => Tok::Times,
                "per" => Tok::Divide,
                _ => Tok::Ident(s),
            }
        } else {
            match lex_operator(&chars, &mut i) {
                Some(t) => t,
                None => {
                    // Unrecognized character: treat as a one-char identifier
                    // so name resolution can complain meaningfully.
                    i += 1;
                    Tok::Ident(chars[start].to_string())
                }
            }
        };
        debug_assert!(i > start, "lexer made no progress at {start}");
        out.push(Token { tok, pos, space_before });
        space_before = false;
    }
    out.push(Token {
        tok: Tok::Eof,
        pos: src.len(),
        space_before,
    });
    out
}

/// Lex a numeric literal: digits, a decimal point, base prefixes, and
/// exponent suffixes. Digit-group spaces are *not* consumed here — the
/// parser joins adjacent number tokens per libqalculate's separator rules.
fn lex_number(chars: &[char], i: &mut usize) -> String {
    let mut s = String::new();
    // Base prefix (0x, 0b, 0o, 0d) — pass through to Number::set.
    if chars[*i] == '0' && *i + 1 < chars.len() && matches!(chars[*i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O' | 'd' | 'D') {
        let is_hex = matches!(chars[*i + 1], 'x' | 'X');
        s.push(chars[*i]);
        s.push(chars[*i + 1]);
        *i += 2;
        while *i < chars.len() && (chars[*i].is_ascii_alphanumeric() || chars[*i] == '.') {
            // Stop at a hex `p` exponent's sign handling below.
            s.push(chars[*i]);
            *i += 1;
        }
        if is_hex && *i < chars.len() && (chars[*i] == '+' || chars[*i] == '-') && s.ends_with(['p', 'P']) {
            s.push(chars[*i]);
            *i += 1;
            while *i < chars.len() && chars[*i].is_ascii_digit() {
                s.push(chars[*i]);
                *i += 1;
            }
        }
        return s;
    }
    while *i < chars.len() && (chars[*i].is_ascii_digit() || chars[*i] == '.') {
        // A '.' only belongs to the number if followed by a digit, otherwise
        // it is the element-wise operator prefix or a trailing separator.
        if chars[*i] == '.' && !(*i + 1 < chars.len() && chars[*i + 1].is_ascii_digit()) {
            break;
        }
        s.push(chars[*i]);
        *i += 1;
    }
    // Scientific exponent: E/e followed by optional sign and digits.
    if *i < chars.len() && matches!(chars[*i], 'e' | 'E') {
        let mut j = *i + 1;
        if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
            j += 1;
        }
        if j < chars.len() && chars[j].is_ascii_digit() {
            while *i < j {
                s.push(chars[*i]);
                *i += 1;
            }
            while *i < chars.len() && chars[*i].is_ascii_digit() {
                s.push(chars[*i]);
                *i += 1;
            }
        }
    }
    s
}

/// Lex an operator, including multi-character and Unicode forms.
fn lex_operator(chars: &[char], i: &mut usize) -> Option<Tok> {
    let c = chars[*i];
    let next = chars.get(*i + 1).copied();
    let mut adv = 1usize;
    let tok = match c {
        '+' => {
            // `+/-` uncertainty
            if next == Some('/') && chars.get(*i + 2) == Some(&'-') {
                adv = 3;
                Tok::PlusMinus
            } else {
                Tok::Plus
            }
        }
        '-' | '−' | '–' => {
            if next == Some('>') {
                adv = 2;
                Tok::To
            } else {
                Tok::Minus
            }
        }
        '*' => {
            if next == Some('*') {
                adv = 2;
                Tok::Power
            } else {
                Tok::Times
            }
        }
        '×' | '·' | '⋅' | '∙' => Tok::Times,
        '/' | '÷' | '∕' => {
            if next == Some('/') {
                adv = 2;
                Tok::IntDivide
            } else {
                Tok::Divide
            }
        }
        '\\' => Tok::IntDivide,
        '^' => {
            Tok::Power
        }
        '%' => {
            if next == Some('%') {
                adv = 2;
                Tok::Rem
            } else {
                Tok::Percent
            }
        }
        '(' => Tok::LParen,
        ')' => Tok::RParen,
        '[' => Tok::LBracket,
        ']' => Tok::RBracket,
        '{' => Tok::LBrace,
        '}' => Tok::RBrace,
        ',' => Tok::Comma,
        ';' => Tok::Semicolon,
        ':' => {
            if next == Some('=') {
                adv = 2;
                Tok::Assign
            } else {
                Tok::Colon
            }
        }
        '=' => {
            if next == Some('=') {
                adv = 2;
                Tok::Equals
            } else if next == Some('>') {
                adv = 2;
                Tok::GreaterEquals
            } else if next == Some('<') {
                adv = 2;
                Tok::LessEquals
            } else {
                Tok::Equals
            }
        }
        '≠' => Tok::NotEquals,
        '≤' => Tok::LessEquals,
        '≥' => Tok::GreaterEquals,
        '±' => Tok::PlusMinus,
        '<' => {
            if next == Some('=') {
                adv = 2;
                Tok::LessEquals
            } else if next == Some('>') {
                adv = 2;
                Tok::NotEquals
            } else if next == Some('<') {
                adv = 2;
                Tok::ShiftLeft
            } else {
                Tok::Less
            }
        }
        '>' => {
            if next == Some('=') {
                adv = 2;
                Tok::GreaterEquals
            } else if next == Some('>') {
                adv = 2;
                Tok::ShiftRight
            } else {
                Tok::Greater
            }
        }
        '!' => {
            if next == Some('=') {
                adv = 2;
                Tok::NotEquals
            } else {
                Tok::LogicalNot
            }
        }
        '&' => {
            if next == Some('&') {
                adv = 2;
                Tok::LogicalAnd
            } else {
                Tok::BitAnd
            }
        }
        '∧' => Tok::LogicalAnd,
        '|' => {
            if next == Some('|') {
                adv = 2;
                Tok::LogicalOr
            } else {
                Tok::BitOr
            }
        }
        '∨' => Tok::BitOr,
        '~' | '¬' => Tok::BitNot,
        '⊻' => Tok::BitXor,
        '.' => {
            if next == Some('^') {
                adv = 2;
                Tok::ElementPower
            } else if next == Some('*') {
                adv = 2;
                Tok::ElementTimes
            } else {
                return None;
            }
        }
        _ => return None,
    };
    *i += adv;
    Some(tok)
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{202f}' || c == '\u{2009}' || c == '\u{a0}'
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '\u{b0}' /* ° */ || c == '$' || c == '€' || c == '£'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        tokenize(s).into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn numbers_and_operators() {
        assert_eq!(
            toks("1+2*3"),
            vec![
                Tok::Number("1".into()),
                Tok::Plus,
                Tok::Number("2".into()),
                Tok::Times,
                Tok::Number("3".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn decimals_and_exponents() {
        assert_eq!(toks("1.5e-2"), vec![Tok::Number("1.5e-2".into()), Tok::Eof]);
        assert_eq!(toks("2.5E2"), vec![Tok::Number("2.5E2".into()), Tok::Eof]);
        // `e` not followed by digits is an identifier (Euler's number).
        assert_eq!(
            toks("2e"),
            vec![Tok::Number("2".into()), Tok::Ident("e".into()), Tok::Eof]
        );
    }

    #[test]
    fn base_prefixes() {
        assert_eq!(toks("0xFF"), vec![Tok::Number("0xFF".into()), Tok::Eof]);
        assert_eq!(toks("0b1011"), vec![Tok::Number("0b1011".into()), Tok::Eof]);
    }

    #[test]
    fn multichar_operators() {
        assert_eq!(toks("a<=b"), vec![Tok::Ident("a".into()), Tok::LessEquals, Tok::Ident("b".into()), Tok::Eof]);
        assert_eq!(toks("1<<2"), vec![Tok::Number("1".into()), Tok::ShiftLeft, Tok::Number("2".into()), Tok::Eof]);
        assert_eq!(toks("2**3"), vec![Tok::Number("2".into()), Tok::Power, Tok::Number("3".into()), Tok::Eof]);
        assert_eq!(toks("5%%3"), vec![Tok::Number("5".into()), Tok::Rem, Tok::Number("3".into()), Tok::Eof]);
        assert_eq!(toks("x:=5"), vec![Tok::Ident("x".into()), Tok::Assign, Tok::Number("5".into()), Tok::Eof]);
    }

    #[test]
    fn unicode_operators() {
        // From tests/operators.batch: ×, ⋅, ∨, ¬, −
        assert_eq!(toks("2×3"), vec![Tok::Number("2".into()), Tok::Times, Tok::Number("3".into()), Tok::Eof]);
        assert_eq!(toks("2⋅3"), vec![Tok::Number("2".into()), Tok::Times, Tok::Number("3".into()), Tok::Eof]);
        assert_eq!(toks("1∨0"), vec![Tok::Number("1".into()), Tok::BitOr, Tok::Number("0".into()), Tok::Eof]);
        assert_eq!(toks("¬1"), vec![Tok::BitNot, Tok::Number("1".into()), Tok::Eof]);
        assert_eq!(toks("2−1"), vec![Tok::Number("2".into()), Tok::Minus, Tok::Number("1".into()), Tok::Eof]);
    }

    #[test]
    fn word_operators() {
        assert_eq!(toks("5 mod 3"), vec![Tok::Number("5".into()), Tok::Mod, Tok::Number("3".into()), Tok::Eof]);
        assert_eq!(toks("1 to 2"), vec![Tok::Number("1".into()), Tok::To, Tok::Number("2".into()), Tok::Eof]);
        assert_eq!(toks("2 plus 3"), vec![Tok::Number("2".into()), Tok::Plus, Tok::Number("3".into()), Tok::Eof]);
    }

    #[test]
    fn whitespace_is_recorded() {
        let t = tokenize("1/2 x");
        // tokens: 1 / 2 x EOF — the `x` has whitespace before it, `2` does not
        assert_eq!(t[2].tok, Tok::Number("2".into()));
        assert!(!t[2].space_before);
        assert_eq!(t[3].tok, Tok::Ident("x".into()));
        assert!(t[3].space_before, "adaptive mode needs this flag");

        let t2 = tokenize("1/2x");
        assert_eq!(t2[3].tok, Tok::Ident("x".into()));
        assert!(!t2[3].space_before);
    }

    #[test]
    fn identifiers_and_units() {
        assert_eq!(
            toks("5 km/h"),
            vec![
                Tok::Number("5".into()),
                Tok::Ident("km".into()),
                Tok::Divide,
                Tok::Ident("h".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn element_wise_ops() {
        assert_eq!(toks("a.^b"), vec![Tok::Ident("a".into()), Tok::ElementPower, Tok::Ident("b".into()), Tok::Eof]);
    }

    #[test]
    fn plus_minus_uncertainty() {
        assert_eq!(toks("5+/-1"), vec![Tok::Number("5".into()), Tok::PlusMinus, Tok::Number("1".into()), Tok::Eof]);
        assert_eq!(toks("5±1"), vec![Tok::Number("5".into()), Tok::PlusMinus, Tok::Number("1".into()), Tok::Eof]);
    }

    #[test]
    fn positions_are_byte_offsets() {
        let t = tokenize("π+1");
        assert_eq!(t[0].pos, 0);
        assert_eq!(t[1].tok, Tok::Plus);
        assert_eq!(t[1].pos, 'π'.len_utf8());
    }
}
