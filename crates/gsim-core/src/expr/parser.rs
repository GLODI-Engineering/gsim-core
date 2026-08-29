//! Recursive-descent parser for the `${{ ... }}` grammar. Precedence, low to high:
//! `||` `&&` equality comparison additive multiplicative unary postfix(call/dotted-var) primary.

use super::ast::{BinOp, Expr, UnOp};
use super::lexer::{tokenize, LexError, Token};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
}

impl From<LexError> for ParseError {
    fn from(e: LexError) -> Self {
        ParseError {
            message: format!("{} (at byte {})", e.message, e.at),
        }
    }
}

pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(src)?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_or()?;
    p.expect(&Token::End)?;
    Ok(expr)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
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

    fn expect(&mut self, want: &Token) -> Result<(), ParseError> {
        if self.peek() == want {
            self.advance();
            Ok(())
        } else {
            Err(ParseError {
                message: format!("expected {want:?}, found {:?}", self.peek()),
            })
        }
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while *self.peek() == Token::Or {
            self.advance();
            let rhs = self.parse_and()?;
            lhs = Expr::Binary(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_equality()?;
        while *self.peek() == Token::And {
            self.advance();
            let rhs = self.parse_equality()?;
            lhs = Expr::Binary(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Token::Eq => BinOp::Eq,
                Token::Ne => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_comparison()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Token::Lt => BinOp::Lt,
                Token::Le => BinOp::Le,
                Token::Gt => BinOp::Gt,
                Token::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_additive()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_multiplicative()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Rem,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Token::Minus => {
                self.advance();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?)))
            }
            Token::Not => {
                self.advance();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)))
            }
            _ => self.parse_postfix(),
        }
    }

    /// `IDENT` -> a dotted variable path (`a.b.c`) or, if immediately followed by `(`, a
    /// builtin-function call (`count(x)`). Calls do not chain further dots on their result —
    /// the grammar has no need for it yet.
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        if let Token::Ident(name) = self.peek().clone() {
            self.advance();
            if *self.peek() == Token::LParen {
                self.advance();
                let mut args = Vec::new();
                if *self.peek() != Token::RParen {
                    args.push(self.parse_or()?);
                    while *self.peek() == Token::Comma {
                        self.advance();
                        args.push(self.parse_or()?);
                    }
                }
                self.expect(&Token::RParen)?;
                return Ok(Expr::Call(name, args));
            }
            let mut path = vec![name];
            while *self.peek() == Token::Dot {
                self.advance();
                match self.advance() {
                    Token::Ident(seg) => path.push(seg),
                    other => {
                        return Err(ParseError {
                            message: format!("expected identifier after '.', found {other:?}"),
                        })
                    }
                }
            }
            return Ok(Expr::Var(path));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.advance() {
            Token::Number(n) => Ok(Expr::Literal(Value::from(n))),
            Token::String(s) => Ok(Expr::Literal(Value::from(s))),
            Token::True => Ok(Expr::Literal(Value::from(true))),
            Token::False => Ok(Expr::Literal(Value::from(false))),
            Token::Null => Ok(Expr::Literal(Value::Null)),
            Token::LParen => {
                let inner = self.parse_or()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            other => Err(ParseError {
                message: format!("unexpected token {other:?}"),
            }),
        }
    }
}
