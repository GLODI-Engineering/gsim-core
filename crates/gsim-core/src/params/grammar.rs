//! `.gsim_sch` §5.2's expression grammar: lexer, parser and evaluator, numeric only.
//!
//! # Why this is not [`crate::expr`]
//!
//! This crate already has an expression language — the `${{ ... }}` DSL a component contract
//! interpolates with. Reusing it here was considered and is wrong in both directions, which
//! `GLODI-Engineering/gsim-core#94` already recorded at the level of vocabulary: §5.2 has `**`
//! and `sin`/`cos`/`tan`/`exp`/`log`/`log10`/`sqrt`/`abs`/`pow`, which `expr` does not, while
//! `expr` has `%`, comparisons, `and`/`or`, `at()` and `matches()`, which §5.2 explicitly
//! forbids. So sharing it would accept what the spec forbids *and* reject what it requires.
//!
//! But the decisive reason is smaller and worse than a vocabulary mismatch: **the two languages
//! disagree about what a number is.** `expr`'s lexer takes digits and dots, so `1.5e-6` comes
//! out as four tokens — `1.5`, the identifier `e`, `-`, `6` — and `e` is a §5.2 *constant*. A
//! shared lexer would therefore read `{1.5e-6 * 2}` as something involving Euler's number and
//! either fail obscurely or, worse, not fail. Every value on a power-electronics sheet is
//! written that way, so this is not an edge case; it is the common case, and it would have been
//! silently wrong rather than loudly broken.
//!
//! Scientific notation is consequently the one thing this lexer is careful about, and
//! `a_value_in_scientific_notation_is_one_number_not_four` is the test that holds it.
//!
//! # What §5.2 deliberately cannot do
//!
//! No attribute access, no subscripts, no comparisons, no conditionals, no string operations,
//! no function definitions, no identifiers containing `__`. Each is refused with a message that
//! says §5.2 forbids it rather than "unexpected token", because a user who typed `a > b` in a
//! parameter field needs to be told it is not that kind of language.

use std::collections::BTreeMap;

/// A §5.2 token. Numeric only — there is no string, boolean or null token, because a §5.2
/// expression evaluates to a number and nothing else.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Comma,
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    Power,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprError {
    pub message: String,
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError {
        message: message.into(),
    })
}

/// What §5.2 forbids, with the message each gets. Spelled out rather than left to fall through
/// to "unexpected character", because the point of refusing is that the user learns what kind
/// of language this is.
fn forbidden(c: char) -> Option<&'static str> {
    match c {
        '.' => Some(
            "'.' is attribute access, which §5.2 does not have (a decimal point inside a number is fine)",
        ),
        '[' | ']' => Some("subscripts are not part of §5.2"),
        '<' | '>' | '=' | '!' => Some("comparisons are not part of §5.2"),
        '&' | '|' => Some("logical operators are not part of §5.2"),
        '?' | ':' => Some("conditionals are not part of §5.2"),
        '\'' | '"' => Some("string literals are not part of §5.2 — a parameter is a number"),
        '%' => Some("'%' is not part of §5.2"),
        _ => None,
    }
}

fn tokenize(src: &str) -> Result<Vec<Token>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut out = Vec::new();

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if let Some(why) = forbidden(c) {
            // One exception: a '.' that starts a number (".5") is a decimal point, not access.
            let decimal_point = c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if !decimal_point {
                return err(why);
            }
        }
        match c {
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' => {
                out.push(Token::Minus);
                i += 1;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                out.push(Token::Power);
                i += 2;
            }
            '*' => {
                out.push(Token::Star);
                i += 1;
            }
            '/' => {
                out.push(Token::Slash);
                i += 1;
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == '.')
                {
                    i += 1;
                }
                // The exponent, and the whole reason this lexer exists rather than `crate::expr`'s:
                // `e`/`E` is consumed into the number only when real digits follow it, with an
                // optional sign. So `1.5e-6` is one token while a bare `e` stays the constant and
                // `2e` is a number followed by an identifier, which the parser then refuses.
                if matches!(chars.get(i), Some('e' | 'E')) {
                    let sign = usize::from(matches!(chars.get(i + 1), Some('+' | '-')));
                    if chars.get(i + 1 + sign).is_some_and(char::is_ascii_digit) {
                        i += 1 + sign;
                        while chars.get(i).is_some_and(char::is_ascii_digit) {
                            i += 1;
                        }
                    }
                }
                let text: String = chars[start..i].iter().collect();
                match text.parse::<f64>() {
                    Ok(n) => out.push(Token::Number(n)),
                    Err(_) => {
                        return err(format!(
                            "'{text}' is not a number (§5.2 has no SI suffixes — write 2.8e-6, not 2.8u)"
                        ))
                    }
                }
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                if text.contains("__") {
                    return err(format!("'{text}' contains '__', which §5.2 forbids"));
                }
                out.push(Token::Ident(text));
            }
            other => return err(format!("unexpected character '{other}'")),
        }
    }
    out.push(Token::End);
    Ok(out)
}

/// A §5.2 expression. Kept as data rather than evaluated during parsing, so that collecting a
/// value's *dependencies* — which is what the topological resolve in [`super`] needs — does not
/// require a second parser.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Var(String),
    Call(String, Vec<Expr>),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Pow(Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Every identifier this expression reads that is not a §5.2 constant, in first-seen order.
    pub fn dependencies(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_deps(&mut out);
        out
    }

    fn collect_deps(&self, out: &mut Vec<String>) {
        match self {
            Expr::Number(_) => {}
            Expr::Var(name) => {
                if constant(name).is_none() && !out.iter().any(|n| n == name) {
                    out.push(name.clone());
                }
            }
            Expr::Call(_, args) => {
                for a in args {
                    a.collect_deps(out);
                }
            }
            Expr::Neg(inner) => inner.collect_deps(out),
            Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::Pow(a, b) => {
                a.collect_deps(out);
                b.collect_deps(out);
            }
        }
    }
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        _ => None,
    }
}

/// §5.2's function list, and its arity. Nothing else is callable: a name that is not here is
/// refused as a function rather than looked up anywhere.
fn arity(name: &str) -> Option<usize> {
    match name {
        "sin" | "cos" | "tan" | "exp" | "log" | "log10" | "sqrt" | "abs" => Some(1),
        "min" | "max" | "pow" => Some(2),
        _ => None,
    }
}

const FUNCTIONS: &str = "sin, cos, tan, exp, log, log10, sqrt, abs, min, max, pow";

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

/// Parses a §5.2 expression — the text *inside* the braces, which [`super::expression_body`]
/// strips.
pub fn parse(src: &str) -> Result<Expr, ExprError> {
    let tokens = tokenize(src)?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.additive()?;
    if p.peek() != &Token::End {
        return err(format!(
            "unexpected {} after a complete expression",
            describe(p.peek())
        ));
    }
    Ok(expr)
}

fn describe(t: &Token) -> String {
    match t {
        Token::Number(n) => format!("number {n}"),
        Token::Ident(name) => format!("'{name}'"),
        Token::Comma => "','".into(),
        Token::LParen => "'('".into(),
        Token::RParen => "')'".into(),
        Token::Plus => "'+'".into(),
        Token::Minus => "'-'".into(),
        Token::Star => "'*'".into(),
        Token::Slash => "'/'".into(),
        Token::Power => "'**'".into(),
        Token::End => "end of expression".into(),
    }
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn additive(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.multiplicative()?;
        loop {
            match self.peek() {
                Token::Plus => {
                    self.advance();
                    lhs = Expr::Add(Box::new(lhs), Box::new(self.multiplicative()?));
                }
                Token::Minus => {
                    self.advance();
                    lhs = Expr::Sub(Box::new(lhs), Box::new(self.multiplicative()?));
                }
                _ => return Ok(lhs),
            }
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.unary()?;
        loop {
            match self.peek() {
                Token::Star => {
                    self.advance();
                    lhs = Expr::Mul(Box::new(lhs), Box::new(self.unary()?));
                }
                Token::Slash => {
                    self.advance();
                    lhs = Expr::Div(Box::new(lhs), Box::new(self.unary()?));
                }
                _ => return Ok(lhs),
            }
        }
    }

    /// Unary binds *looser* than `**` on its left, which is Python's rule and the one §5.2's
    /// "Python-like" means: `-2**2` is `-(2**2)` = -4, not `(-2)**2` = 4. Getting this backwards
    /// would be a silently different number, so `minus_binds_looser_than_power` pins it.
    fn unary(&mut self) -> Result<Expr, ExprError> {
        match self.peek() {
            Token::Minus => {
                self.advance();
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Token::Plus => {
                self.advance();
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// `**` is right-associative — `2**3**2` is `2**(3**2)` — and its right operand goes through
    /// `unary`, so `2**-1` parses.
    fn power(&mut self) -> Result<Expr, ExprError> {
        let base = self.primary()?;
        if self.peek() == &Token::Power {
            self.advance();
            return Ok(Expr::Pow(Box::new(base), Box::new(self.unary()?)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, ExprError> {
        match self.advance() {
            Token::Number(n) => Ok(Expr::Number(n)),
            Token::LParen => {
                let inner = self.additive()?;
                match self.advance() {
                    Token::RParen => Ok(inner),
                    other => err(format!("expected ')', found {}", describe(&other))),
                }
            }
            Token::Ident(name) => {
                if self.peek() != &Token::LParen {
                    return Ok(Expr::Var(name));
                }
                let Some(want) = arity(&name) else {
                    return err(format!(
                        "'{name}' is not a §5.2 function (§5.2 has {FUNCTIONS})"
                    ));
                };
                self.advance();
                let mut args = Vec::new();
                if self.peek() != &Token::RParen {
                    loop {
                        args.push(self.additive()?);
                        if self.peek() == &Token::Comma {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                }
                match self.advance() {
                    Token::RParen => {}
                    other => {
                        return err(format!(
                            "expected ')' to close '{name}(', found {}",
                            describe(&other)
                        ))
                    }
                }
                if args.len() != want {
                    return err(format!(
                        "'{name}' takes {want} argument{}, not {}",
                        if want == 1 { "" } else { "s" },
                        args.len()
                    ));
                }
                Ok(Expr::Call(name, args))
            }
            other => err(format!(
                "expected a number, a name or '(', found {}",
                describe(&other)
            )),
        }
    }
}

/// Evaluates against an already-resolved table. Every identifier must be a §5.2 constant or a
/// key of `known`; the caller ([`super`]) guarantees dependency order, so an unknown name here
/// is a genuine undeclared reference rather than a resolution-order accident.
pub fn eval(expr: &Expr, known: &BTreeMap<String, f64>) -> Result<f64, ExprError> {
    let value = match expr {
        Expr::Number(n) => *n,
        Expr::Var(name) => match constant(name) {
            Some(c) => c,
            None => match known.get(name) {
                Some(v) => *v,
                None => {
                    return err(format!(
                        "'{name}' is not a parameter of this sheet{}",
                        if known.is_empty() {
                            ", which declares none".to_string()
                        } else {
                            format!(
                                " (it declares {})",
                                known.keys().cloned().collect::<Vec<_>>().join(", ")
                            )
                        }
                    ))
                }
            },
        },
        Expr::Call(name, args) => {
            let a = eval(&args[0], known)?;
            match name.as_str() {
                "sin" => a.sin(),
                "cos" => a.cos(),
                "tan" => a.tan(),
                "exp" => a.exp(),
                "log" => a.ln(),
                "log10" => a.log10(),
                "sqrt" => a.sqrt(),
                "abs" => a.abs(),
                _ => {
                    let b = eval(&args[1], known)?;
                    match name.as_str() {
                        "min" => a.min(b),
                        "max" => a.max(b),
                        "pow" => a.powf(b),
                        // `arity` is the only gate on which names reach here, and it and this
                        // match are the same list.
                        other => return err(format!("'{other}' is not a §5.2 function")),
                    }
                }
            }
        }
        Expr::Neg(inner) => -eval(inner, known)?,
        Expr::Add(a, b) => eval(a, known)? + eval(b, known)?,
        Expr::Sub(a, b) => eval(a, known)? - eval(b, known)?,
        Expr::Mul(a, b) => eval(a, known)? * eval(b, known)?,
        Expr::Div(a, b) => {
            let divisor = eval(b, known)?;
            // Refused rather than returned as an infinity. A netlist carrying `inf` is a deck
            // that fails somewhere else with a message about something unrelated, and
            // `general-mna` already hard-errors on a non-finite `.param` for the same reason.
            if divisor == 0.0 {
                return err("division by zero");
            }
            eval(a, known)? / divisor
        }
        Expr::Pow(a, b) => eval(a, known)?.powf(eval(b, known)?),
    };
    if !value.is_finite() {
        return err(format!(
            "evaluates to {value}, which is not a finite number"
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(src: &str) -> Result<f64, ExprError> {
        eval(&parse(src)?, &BTreeMap::new())
    }

    /// The reason this module exists instead of `crate::expr`. Under that lexer `1.5e-6` is
    /// four tokens and `e` is a constant, so every SI-scale value on a sheet would have been
    /// read as arithmetic on Euler's number.
    #[test]
    fn a_value_in_scientific_notation_is_one_number_not_four() {
        assert_eq!(parse("1.5e-6").unwrap(), Expr::Number(1.5e-6));
        assert_eq!(parse("2E3").unwrap(), Expr::Number(2000.0));
        assert_eq!(parse("2.8e+6").unwrap(), Expr::Number(2.8e6));
        assert_eq!(ev("1.5e-6 * 2").unwrap(), 3e-6);
        // And a bare `e` is still the constant.
        assert!((ev("e").unwrap() - std::f64::consts::E).abs() < 1e-12);
        // `2e` is a number followed by a name, which is not an expression.
        assert!(parse("2e").is_err());
    }

    #[test]
    fn minus_binds_looser_than_power() {
        assert_eq!(ev("-2**2").unwrap(), -4.0);
        assert_eq!(ev("(-2)**2").unwrap(), 4.0);
        assert_eq!(ev("2**-1").unwrap(), 0.5);
        // Right-associative, like Python.
        assert_eq!(ev("2**3**2").unwrap(), 512.0);
    }

    #[test]
    fn the_function_list_is_exactly_5_2s() {
        assert!((ev("sqrt(4)").unwrap() - 2.0).abs() < 1e-12);
        assert_eq!(ev("max(3, 7)").unwrap(), 7.0);
        assert_eq!(ev("pow(2, 10)").unwrap(), 1024.0);
        assert!((ev("log10(1000)").unwrap() - 3.0).abs() < 1e-12);
        // Not in §5.2, and refused as a function rather than looked up as a parameter.
        let e = parse("at(1)").unwrap_err();
        assert!(e.message.contains("not a §5.2 function"), "{e}");
        assert!(parse("tanh(1)").is_err());
        // Wrong arity is its own message.
        assert!(parse("sqrt(1, 2)")
            .unwrap_err()
            .message
            .contains("1 argument"));
    }

    #[test]
    fn what_5_2_forbids_says_so_rather_than_unexpected_token() {
        for (src, needle) in [
            ("a > b", "comparisons"),
            ("a == b", "comparisons"),
            ("a and b", "after a complete expression"),
            ("a && b", "logical operators"),
            ("a.b", "attribute access"),
            ("a[0]", "subscripts"),
            ("'x'", "string literals"),
            ("5 % 2", "'%' is not part of §5.2"),
            ("__dunder", "forbids"),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.message.contains(needle), "{src} -> {e}");
        }
        // A leading decimal point is a number, not attribute access.
        assert_eq!(ev(".5").unwrap(), 0.5);
    }

    #[test]
    fn an_si_suffix_is_refused_with_a_message_that_says_what_to_write() {
        let e = parse("2.8u").unwrap_err();
        assert!(e.message.contains("after a complete expression"), "{e}");
        // The number lexer's own message covers the malformed-literal case.
        assert!(parse("1.2.3").unwrap_err().message.contains("SI suffixes"));
    }

    #[test]
    fn a_division_by_zero_and_a_non_finite_result_are_refused_not_returned() {
        assert_eq!(ev("1 / 0").unwrap_err().message, "division by zero");
        assert!(ev("exp(100000)")
            .unwrap_err()
            .message
            .contains("not a finite number"));
    }

    #[test]
    fn dependencies_are_the_names_that_are_not_constants() {
        let e = parse("sin(pi * x) + max(y, e) - x").unwrap();
        assert_eq!(e.dependencies(), vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn an_unknown_name_names_the_parameters_the_sheet_does_declare() {
        let mut known = BTreeMap::new();
        known.insert("Rload".to_string(), 1000.0);
        let e = eval(&parse("Rsource * 2").unwrap(), &known).unwrap_err();
        assert!(e.message.contains("Rsource"), "{e}");
        assert!(e.message.contains("Rload"), "{e}");
    }
}
