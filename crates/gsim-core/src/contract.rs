//! GSF v2: the component-contract schema and its top-level evaluation entry point,
//! `evaluate_component`. A contract with no `${{ }}` expressions anywhere behaves exactly like a
//! static GSF v1 component — evaluation is a no-op pass-through in that case.

use crate::expr::Context;
use crate::port::Port;
use crate::primitive::Primitive;
use crate::resolve::{self, ResolveError};
use crate::template::Template;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// A `${{ ... }}` boolean expression evaluated with this parameter's current value bound to
    /// `value`, in addition to the usual `params.*` scope. Absent means no validation beyond
    /// whatever type-level checking the schema itself does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Geometry {
    /// `[width, height]`, matching the existing GSF v1 `metadata.symbol.bbox` convention
    /// (`apps/frontend/src/canvas/nodes/primitive-renderer.ts`). Either element may be a plain
    /// number or a `"${{ ... }}"` string.
    pub bbox: [Value; 2],
    /// Each element is either a plain (already-`Primitive`-shaped) object, or a `for_each`/
    /// `when` control node — see `crate::resolve`.
    #[serde(default)]
    pub primitives: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentContract {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Each element is either a plain `Port`-shaped object, or a `for_each`/`when` control node.
    #[serde(default)]
    pub ports: Vec<Value>,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvaluatedSymbol {
    pub bbox: (f64, f64),
    pub ports: Vec<Port>,
    pub primitives: Vec<Primitive>,
}

/// Serializable so bindings (`gsim-py`, `gsim-wasm`) can return `validate_parameters`'s result
/// straight as JSON (`{"parameter": ..., "message": ...}` per failure) instead of hand-rolling an
/// equivalent shadow struct per binding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationFailure {
    pub parameter: String,
    pub message: String,
}

#[derive(Debug)]
pub enum ContractError {
    Resolve(ResolveError),
    Shape(serde_json::Error),
    Eval(String),
}

impl From<ResolveError> for ContractError {
    fn from(e: ResolveError) -> Self {
        ContractError::Resolve(e)
    }
}

impl From<serde_json::Error> for ContractError {
    fn from(e: serde_json::Error) -> Self {
        ContractError::Shape(e)
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractError::Resolve(e) => write!(f, "{e}"),
            ContractError::Shape(e) => write!(f, "resolved value has the wrong shape: {e}"),
            ContractError::Eval(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ContractError {}

/// Every declared parameter's default, overridden by whatever the caller actually supplied.
/// Both [`validate_parameters`] and [`evaluate_component`] evaluate against this, not against
/// the caller's map directly — an expression that cross-references another parameter
/// (`${{ value < params.v_th }}` inside `v_breakdown`'s own `validate`) must see `v_th`'s
/// default even when the caller only supplied `v_breakdown`, or every such cross-reference would
/// spuriously fail as "unknown field" whenever a caller omits an unrelated, defaulted parameter.
fn resolved_params(
    contract: &ComponentContract,
    supplied: &Map<String, Value>,
) -> Map<String, Value> {
    let mut resolved: Map<String, Value> = contract
        .parameters
        .iter()
        .map(|p| (p.name.clone(), p.default.clone()))
        .collect();
    for (k, v) in supplied {
        resolved.insert(k.clone(), v.clone());
    }
    resolved
}

/// Validates every parameter with a `validate` expression against the supplied (or defaulted)
/// values. Returns one failure per parameter that fails — an empty vec means everything passed.
/// A parameter with no `validate` expression is never checked here (schema/type-level checks,
/// if any, are a separate concern).
pub fn validate_parameters(
    contract: &ComponentContract,
    params: &Map<String, Value>,
) -> Vec<ValidationFailure> {
    let resolved = resolved_params(contract, params);
    let mut failures = Vec::new();
    for p in &contract.parameters {
        let Some(validate_src) = &p.validate else {
            continue;
        };
        let value = resolved.get(&p.name).cloned().unwrap_or(Value::Null);
        let mut bindings = Map::new();
        bindings.insert("value".to_string(), value);
        let ctx = Context::new(resolved.clone()).child(bindings);
        match Template::parse(validate_src).eval(&ctx) {
            Ok(Value::Bool(true)) => {}
            Ok(Value::Bool(false)) => failures.push(ValidationFailure {
                parameter: p.name.clone(),
                message: p
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("{} is invalid", p.name)),
            }),
            Ok(other) => failures.push(ValidationFailure {
                parameter: p.name.clone(),
                message: format!(
                    "'validate' for {} must evaluate to a boolean, got {other}",
                    p.name
                ),
            }),
            Err(e) => failures.push(ValidationFailure {
                parameter: p.name.clone(),
                message: format!("'validate' for {} failed: {e}", p.name),
            }),
        }
    }
    failures
}

/// The whole GSF v2 pipeline: resolve `bbox`, `ports`, and `primitives` against `params`,
/// producing the exact static shapes a renderer (or a document builder) consumes. Does not run
/// parameter validation itself — call [`validate_parameters`] separately, since a caller may
/// want to evaluate geometry for a preview even while a parameter is currently invalid.
pub fn evaluate_component(
    contract: &ComponentContract,
    params: &Map<String, Value>,
) -> Result<EvaluatedSymbol, ContractError> {
    let ctx = Context::new(resolved_params(contract, params));

    let width = resolve::resolve_scalar(&contract.geometry.bbox[0], &ctx)?
        .as_f64()
        .ok_or_else(|| ContractError::Eval("bbox width must evaluate to a number".into()))?;
    let height = resolve::resolve_scalar(&contract.geometry.bbox[1], &ctx)?
        .as_f64()
        .ok_or_else(|| ContractError::Eval("bbox height must evaluate to a number".into()))?;

    let ports = resolve::resolve_list(&contract.ports, &ctx)?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Port>, _>>()?;

    let primitives = resolve::resolve_list(&contract.geometry.primitives, &ctx)?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<Primitive>, _>>()?;

    Ok(EvaluatedSymbol {
        bbox: (width, height),
        ports,
        primitives,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sum_contract() -> ComponentContract {
        serde_json::from_value(json!({
            "name": "control.sum",
            "label": "Sum",
            "parameters": [
                {
                    "name": "signs",
                    "type": "string",
                    "default": "++",
                    "validate": "${{ matches(value, '^[+-]{1,8}$') }}",
                    "error": "Signs must be 1-8 characters of + or -"
                }
            ],
            "ports": [{
                "for_each": "${{ split(params.signs, '') }}",
                "as": "sign",
                "index": "i",
                "emit": {
                    "id": "in${{ i }}",
                    "label": "${{ sign }}",
                    "side": "left",
                    "kind": "signal",
                    "direction": "in"
                }
            }],
            "geometry": {
                "bbox": [60, "${{ max(40, count(params.signs) * 16) }}"],
                "primitives": [
                    {"type": "circle", "cx": 30, "cy": 20, "r": 20}
                ]
            }
        }))
        .unwrap()
    }

    #[test]
    fn static_component_round_trips_with_no_params() {
        let contract: ComponentContract = serde_json::from_value(json!({
            "name": "passive.resistor",
            "label": "Resistor",
            "ports": [
                {"id": "p", "side": "left", "kind": "electrical", "direction": "inout"},
                {"id": "n", "side": "right", "kind": "electrical", "direction": "inout"}
            ],
            "geometry": {"bbox": [60, 20], "primitives": [
                {"type": "rect", "x": 0, "y": 0, "w": 60, "h": 20}
            ]}
        }))
        .unwrap();
        let evaluated = evaluate_component(&contract, &Map::new()).unwrap();
        assert_eq!(evaluated.bbox, (60.0, 20.0));
        assert_eq!(evaluated.ports.len(), 2);
        assert_eq!(evaluated.primitives.len(), 1);
    }

    #[test]
    fn sum_block_grows_a_port_and_the_bbox_when_signs_grows() {
        let contract = sum_contract();

        let two =
            evaluate_component(&contract, json!({"signs": "++"}).as_object().unwrap()).unwrap();
        assert_eq!(two.ports.len(), 2);
        assert_eq!(two.ports[0].id, "in0");
        assert_eq!(two.ports[1].id, "in1");
        assert_eq!(two.bbox, (60.0, 40.0)); // max(40, 2*16=32) = 40

        let three =
            evaluate_component(&contract, json!({"signs": "++-"}).as_object().unwrap()).unwrap();
        assert_eq!(three.ports.len(), 3);
        assert_eq!(three.ports[2].id, "in2");
        assert_eq!(three.ports[2].label.as_deref(), Some("-"));
        assert_eq!(three.bbox, (60.0, 48.0)); // max(40, 3*16=48) = 48
    }

    #[test]
    fn a_bare_string_param_only_gives_positional_not_true_identity_stability() {
        // `in${{ i }}` derives its id from *current position* in `split(signs)`, not from a
        // persisted per-element id. Appending/removing at the tail is therefore stable (this
        // case), but removing/inserting in the *middle* of `signs` reuses "in1" for a different
        // logical sign than before -- there is no way to do better with a bare string parameter,
        // since a character has no identity beyond its position. A component that needs genuine
        // stability across interior edits (not just append/trim-at-end) must use a structured
        // array parameter whose elements each carry their own `id`, and emit
        // `"${{ item.id }}"` instead of `"in${{ i }}"`. This test documents the limitation
        // rather than asserting a stability guarantee this contract shape cannot actually give.
        let contract = sum_contract();

        let before =
            evaluate_component(&contract, json!({"signs": "++-"}).as_object().unwrap()).unwrap();
        assert_eq!(
            before
                .ports
                .iter()
                .map(|p| (p.id.as_str(), p.label.as_deref()))
                .collect::<Vec<_>>(),
            vec![("in0", Some("+")), ("in1", Some("+")), ("in2", Some("-"))]
        );

        // Removing the *first* '+' shifts every remaining sign's position by one: "in1" used to
        // mean the second '+', now means '-'. Same id, different logical port -- exactly the
        // renumbering the port-identity design decision says must not happen for a wire's sake,
        // which is why the frontend layer generating `signs` edits should only append/remove at
        // the end for this contract shape, not splice the middle.
        let after =
            evaluate_component(&contract, json!({"signs": "+-"}).as_object().unwrap()).unwrap();
        assert_eq!(
            after
                .ports
                .iter()
                .map(|p| (p.id.as_str(), p.label.as_deref()))
                .collect::<Vec<_>>(),
            vec![("in0", Some("+")), ("in1", Some("-"))]
        );
    }

    #[test]
    fn validate_parameters_reports_the_authored_error_message() {
        let contract = sum_contract();
        let failures = validate_parameters(&contract, json!({"signs": "++*"}).as_object().unwrap());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].parameter, "signs");
        assert_eq!(
            failures[0].message,
            "Signs must be 1-8 characters of + or -"
        );

        let ok = validate_parameters(&contract, json!({"signs": "++-"}).as_object().unwrap());
        assert!(ok.is_empty());
    }
}
