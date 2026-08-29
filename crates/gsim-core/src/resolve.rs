//! Resolves a GSF v2 JSON tree (as parsed from a component's `ports:`/`primitives:` YAML,
//! converted to JSON) into a plain, static JSON tree with every `${{ }}` expression evaluated
//! and every `for_each`/`when` node expanded — after which it is deserializable into the same
//! static `Port`/`Primitive` shapes GSF v1 already used, unchanged.
//!
//! This is the one place `for_each`/`when`/`emit` are interpreted; nothing else in this crate
//! needs to know that syntax exists.

use crate::expr::{Context, ExprError};
use crate::template::Template;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct ResolveError {
    pub message: String,
}

impl From<ExprError> for ResolveError {
    fn from(e: ExprError) -> Self {
        ResolveError {
            message: e.to_string(),
        }
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ResolveError {}

fn err(msg: impl Into<String>) -> ResolveError {
    ResolveError {
        message: msg.into(),
    }
}

/// Recursively template-substitutes every string leaf in a JSON value. Does not interpret
/// `for_each`/`when`/`emit` — those are only meaningful as an *element* of a list, handled by
/// [`resolve_list`].
pub fn resolve_scalar(value: &Value, ctx: &Context) -> Result<Value, ResolveError> {
    match value {
        Value::String(s) => {
            let template = Template::parse(s);
            if template.is_dynamic() {
                Ok(template.eval(ctx)?)
            } else {
                Ok(value.clone())
            }
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_scalar(item, ctx)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), resolve_scalar(v, ctx)?);
            }
            Ok(Value::Object(out))
        }
        // Number, Bool, Null
        other => Ok(other.clone()),
    }
}

/// Resolves a list that may contain `for_each`/`when` control nodes alongside plain items —
/// used for both `ports:` and `primitives:`, which share this exact mechanism. Each input
/// element expands to zero (a `when: false` guard, or an empty `for_each` iterable), one (a
/// plain item, or a `when: true` guard), or many (a `for_each` loop) output elements; results
/// are flattened in order.
pub fn resolve_list(items: &[Value], ctx: &Context) -> Result<Vec<Value>, ResolveError> {
    let mut out = Vec::new();
    for item in items {
        out.extend(resolve_list_item(item, ctx)?);
    }
    Ok(out)
}

fn resolve_list_item(item: &Value, ctx: &Context) -> Result<Vec<Value>, ResolveError> {
    let Value::Object(obj) = item else {
        return Ok(vec![resolve_scalar(item, ctx)?]);
    };

    if let Some(for_each_src) = obj.get("for_each") {
        return resolve_for_each(obj, for_each_src, ctx);
    }

    if let Some(when_src) = obj.get("when") {
        let when_src = when_src
            .as_str()
            .ok_or_else(|| err("'when' must be a \"${{ ... }}\" string"))?;
        let cond = Template::parse(when_src).eval(ctx)?;
        let keep = cond
            .as_bool()
            .ok_or_else(|| err(format!("'when' must evaluate to a boolean, got {cond}")))?;
        if !keep {
            return Ok(vec![]);
        }
        let mut rest = obj.clone();
        rest.remove("when");
        return Ok(vec![resolve_scalar(&Value::Object(rest), ctx)?]);
    }

    Ok(vec![resolve_scalar(item, ctx)?])
}

fn resolve_for_each(
    obj: &Map<String, Value>,
    for_each_src: &Value,
    ctx: &Context,
) -> Result<Vec<Value>, ResolveError> {
    let for_each_src = for_each_src
        .as_str()
        .ok_or_else(|| err("'for_each' must be a \"${{ ... }}\" string"))?;
    let as_name = obj
        .get("as")
        .and_then(Value::as_str)
        .ok_or_else(|| err("'for_each' requires an 'as' binding name"))?;
    let index_name = obj.get("index").and_then(Value::as_str);
    let emit = obj
        .get("emit")
        .ok_or_else(|| err("'for_each' requires an 'emit' body"))?;

    let iterable = Template::parse(for_each_src).eval(ctx)?;
    let Value::Array(elements) = iterable else {
        return Err(err(format!(
            "'for_each' must evaluate to an array, got {iterable}"
        )));
    };

    let mut out = Vec::with_capacity(elements.len());
    for (index, element) in elements.into_iter().enumerate() {
        let mut bindings = Map::new();
        bindings.insert(as_name.to_string(), element);
        if let Some(index_name) = index_name {
            bindings.insert(index_name.to_string(), Value::from(index as f64));
        }
        let child_ctx = ctx.child(bindings);
        out.push(resolve_scalar(emit, &child_ctx)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx_with_signs(signs: &str) -> Context {
        Context::new(json!({"signs": signs}).as_object().unwrap().clone())
    }

    #[test]
    fn plain_items_pass_through_unresolved_when_static() {
        let items = json!([{"id": "p", "side": "left"}, {"id": "n", "side": "right"}]);
        let resolved = resolve_list(items.as_array().unwrap(), &ctx_with_signs("+")).unwrap();
        assert_eq!(resolved, items.as_array().unwrap().clone());
    }

    #[test]
    fn for_each_expands_one_port_per_sign_with_stable_index_ids() {
        let items = json!([{
            "for_each": "${{ split(params.signs, '') }}",
            "as": "sign",
            "index": "i",
            "emit": {
                "id": "in${{ i }}",
                "label": "${{ sign }}",
                "side": "left",
                "kind": "signal"
            }
        }]);
        let resolved = resolve_list(items.as_array().unwrap(), &ctx_with_signs("++-")).unwrap();
        assert_eq!(
            resolved,
            vec![
                json!({"id": "in0", "label": "+", "side": "left", "kind": "signal"}),
                json!({"id": "in1", "label": "+", "side": "left", "kind": "signal"}),
                json!({"id": "in2", "label": "-", "side": "left", "kind": "signal"}),
            ]
        );
    }

    #[test]
    fn for_each_over_empty_iterable_yields_no_items() {
        let items = json!([{
            "for_each": "${{ split(params.signs, '') }}",
            "as": "sign",
            "emit": {"id": "in", "label": "${{ sign }}"}
        }]);
        let resolved = resolve_list(items.as_array().unwrap(), &ctx_with_signs("")).unwrap();
        assert!(resolved.is_empty());
    }

    #[test]
    fn when_false_drops_the_item_when_true_keeps_it_without_the_when_key() {
        let items = json!([
            {"when": "${{ count(params.signs) > 2 }}", "id": "extra"},
            {"id": "always"},
        ]);
        let resolved = resolve_list(items.as_array().unwrap(), &ctx_with_signs("++-")).unwrap();
        assert_eq!(
            resolved,
            vec![json!({"id": "extra"}), json!({"id": "always"})]
        );

        let resolved_short = resolve_list(items.as_array().unwrap(), &ctx_with_signs("+")).unwrap();
        assert_eq!(resolved_short, vec![json!({"id": "always"})]);
    }
}
