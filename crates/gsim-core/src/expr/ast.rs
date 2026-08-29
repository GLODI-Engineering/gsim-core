//! The `${{ ... }}` expression AST. Closed grammar, pure data, no side effects — see
//! `docs/decisions/0001-scope-and-naming.md` for why this exists instead of embedding a real
//! scripting language.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

/// A parsed expression. `Var` holds a dotted path (`params.signs` -> `["params", "signs"]`);
/// `Call` is restricted to the fixed builtin whitelist resolved at eval time in `expr::eval`,
/// never a user-defined or dynamically-dispatched function.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    Var(Vec<String>),
    Call(String, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
}
