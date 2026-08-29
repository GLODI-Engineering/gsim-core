//! `${{ ... }}` string interpolation. A YAML/JSON string field is either plain literal text (no
//! change), a single whole-string expression (`"${{ max(40, x) }}"`, which evaluates to its
//! *typed* result — a number stays a number), or literal text with one or more embedded
//! expressions (`"in${{ i }}"`, which evaluates to a string by concatenation).

use crate::expr::{self, Context, ExprError};
use crate::fmt::stringify;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Literal(String),
    Expr(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    /// Never fails: an unmatched `${{` with no closing `}}` is treated as literal text, since a
    /// component definition author's stray text should not become an eval-time error avenue —
    /// only actual `${{ ... }}` expression *bodies* go through the parser/evaluator.
    pub fn parse(src: &str) -> Template {
        let mut parts = Vec::new();
        let mut rest = src;
        while let Some(start) = rest.find("${{") {
            if start > 0 {
                parts.push(Part::Literal(rest[..start].to_string()));
            }
            let after_open = &rest[start + 3..];
            match after_open.find("}}") {
                Some(end) => {
                    parts.push(Part::Expr(after_open[..end].trim().to_string()));
                    rest = &after_open[end + 2..];
                }
                None => {
                    // No closing delimiter -- treat the rest, including the stray "${{", as
                    // literal text.
                    parts.push(Part::Literal(rest[start..].to_string()));
                    rest = "";
                    break;
                }
            }
        }
        if !rest.is_empty() {
            parts.push(Part::Literal(rest.to_string()));
        }
        Template { parts }
    }

    /// Whether this template contains at least one `${{ }}` expression (used to skip evaluation
    /// entirely for plain literal fields, which is the common case for a static component).
    pub fn is_dynamic(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Expr(_)))
    }

    pub fn eval(&self, ctx: &Context) -> Result<Value, ExprError> {
        if let [Part::Expr(src)] = self.parts.as_slice() {
            return expr::eval_source(src, ctx);
        }
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Literal(s) => out.push_str(s),
                Part::Expr(src) => {
                    let v = expr::eval_source(src, ctx)?;
                    out.push_str(&stringify(&v));
                }
            }
        }
        Ok(Value::String(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> Context {
        Context::new(json!({"signs": "++-"}).as_object().unwrap().clone())
    }

    #[test]
    fn plain_literal_passes_through() {
        let t = Template::parse("left");
        assert!(!t.is_dynamic());
        assert_eq!(t.eval(&ctx()).unwrap(), json!("left"));
    }

    #[test]
    fn whole_string_expression_keeps_its_type() {
        let t = Template::parse("${{ count(params.signs) }}");
        assert_eq!(t.eval(&ctx()).unwrap(), json!(3.0));
    }

    #[test]
    fn interpolated_expression_becomes_a_string() {
        let ctx = Context::new(serde_json::Map::new())
            .child(json!({"i": 2}).as_object().unwrap().clone());
        let t = Template::parse("in${{ i }}");
        assert_eq!(t.eval(&ctx).unwrap(), json!("in2"));
    }

    #[test]
    fn unclosed_delimiter_is_literal_not_an_error() {
        let t = Template::parse("oops ${{ not closed");
        assert!(!t.is_dynamic());
        assert_eq!(t.eval(&ctx()).unwrap(), json!("oops ${{ not closed"));
    }
}
