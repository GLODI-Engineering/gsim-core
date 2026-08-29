//! The `${{ ... }}` expression language: lexer, parser, AST, and evaluator. See
//! `docs/decisions/0001-scope-and-naming.md` for why this exists and what it deliberately
//! cannot do (no loops beyond the `for_each` construct in `crate::resolve`, no user-defined
//! functions, no I/O — a closed grammar over a fixed builtin whitelist).

mod ast;
mod eval;
mod lexer;
mod parser;

pub use ast::{BinOp, Expr, UnOp};
pub use eval::{eval, Context, EvalError};
pub use parser::{parse, ParseError};

#[derive(Debug, Clone, PartialEq)]
pub enum ExprError {
    Parse(ParseError),
    Eval(EvalError),
}

impl From<ParseError> for ExprError {
    fn from(e: ParseError) -> Self {
        ExprError::Parse(e)
    }
}

impl From<EvalError> for ExprError {
    fn from(e: EvalError) -> Self {
        ExprError::Eval(e)
    }
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExprError::Parse(e) => write!(f, "parse error: {}", e.message),
            ExprError::Eval(e) => write!(f, "eval error: {}", e.message),
        }
    }
}

impl std::error::Error for ExprError {}

/// Parse and evaluate a `${{ ... }}` body (without the delimiters) in one call.
pub fn eval_source(src: &str, ctx: &Context) -> Result<serde_json::Value, ExprError> {
    let expr = parse(src)?;
    Ok(eval(&expr, ctx)?)
}
