//! `.gsim_sch` §5: resolving a sheet's parameter table, and substituting the result at every
//! §5.3 use site.
//!
//! This crate is the loader §5.2 means by "the loader evaluates expression strings against the
//! resolved `params` table", and until `GLODI-Engineering/gsim-core#94` it did neither. What it
//! did instead was emit the text: `{Rload * 2}` reached the deck as `{Rload * 2}`, where the
//! solver refused it with a message in its own vocabulary — `unexpected extra parameter '*'`.
//! A user who typed that in the inspector, watched its resolved-value readout say `14`, and
//! pressed Run was told that `*` is an unexpected extra parameter.
//!
//! # Substitution happens on the compile path, never on load
//!
//! This is the load-bearing choice in this module and it is worth stating before the code,
//! because the obvious reading of #94 — "the loader resolves and substitutes" — describes a
//! change that would destroy a user's work.
//!
//! `save_document` is a parse followed by a serialize (#78/#79), so anything substituted at
//! parse time is substituted *in the saved file*. A sheet written with `"R": "{Rload * 2}"`
//! would come back with `"R": 14`, and the parameterisation the user built — the thing that
//! makes one sheet serve a sweep — would be gone, silently, because they opened and saved it.
//!
//! So the document keeps the expression and the **netlist** gets the number.
//! [`crate::compile::compile_with`] resolves the table once and compiles a substituted *clone*;
//! nothing on disk changes. `docs/decisions/0005` drew the same line for a file path ("the path
//! is emitted exactly as written") and `0009` for a build's defines: what the document carries
//! is the user's declaration, and what the compiler emits is what that declaration resolves to.
//!
//! # What a consumer gets
//!
//! The resolved table is returned in [`crate::compile::CompileOutput::params`], so a front end
//! reads numbers from the compiler rather than evaluating §5.2 itself. That removes the
//! asymmetry `omni-schematics#199` describes, where the inspector showed a confident `14`
//! beside a field that could not compile — two implementations of one grammar, one of them not
//! a grammar at all.

mod grammar;

use crate::document::{Model, ModelParam};
use grammar::Expr;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub use grammar::ExprError as ParamExprError;

/// Where a value that failed to resolve was written, in the words the document uses for it, so
/// a message can say `block 'R1' parameter 'R'` rather than a JSON pointer.
#[derive(Debug, Clone, PartialEq)]
pub struct Site(pub String);

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamError {
    /// §5.2: "a `params` table is well-formed iff its dependency graph is a DAG. ... cycles are
    /// rejected with a clear error." Carries the cycle, in order, so the message can print the
    /// loop rather than name one arbitrary member of it.
    Cycle { cycle: Vec<String> },
    /// A §5.2 expression or a §5.1 reference that does not resolve — a syntax error, an
    /// undeclared name, a division by zero, a non-finite result.
    Unresolvable {
        site: Site,
        text: String,
        reason: String,
    },
    /// A `model.params` entry whose value is neither a number, a boolean, a §5.1/§5.2 string nor
    /// a structured constant — so there is nothing to resolve it to.
    NotAValue { name: String, value: Value },
}

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamError::Cycle { cycle } => write!(
                f,
                "these sheet parameters are defined in terms of each other, so none of them has a value: {}",
                cycle.join(" -> ")
            ),
            ParamError::Unresolvable { site, text, reason } => {
                write!(f, "{site}: '{text}' cannot be resolved -- {reason}")
            }
            ParamError::NotAValue { name, value } => write!(
                f,
                "sheet parameter '{name}' has the value {value}, which is not a number, an expression or a reference"
            ),
        }
    }
}

impl std::error::Error for ParamError {}

/// A sheet's parameters, resolved. Numeric entries only: §5.2 evaluates to a number, and a
/// reference resolves to one.
///
/// A parameter whose value is a **structured constant** (an array — a lookup table, a matrix)
/// is kept separately rather than forced into this map,
/// because it is a legitimate value that no §5.2 expression can be written over. A reference to
/// one resolves; arithmetic on one is refused by the grammar, which has no array.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedParams {
    numbers: BTreeMap<String, f64>,
    structured: BTreeMap<String, Value>,
}

impl ResolvedParams {
    /// The resolved value of one parameter, as JSON, whichever kind it is.
    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(n) = self.numbers.get(name) {
            return Some(number(*n));
        }
        self.structured.get(name).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.numbers.contains_key(name) || self.structured.contains_key(name)
    }

    /// Every resolved parameter, for a consumer that wants the whole table — notably a front end
    /// showing a resolved-value readout, which is the thing that must not be a second
    /// implementation of §5.2.
    pub fn to_map(&self) -> Map<String, Value> {
        let mut out = Map::new();
        for (name, value) in &self.numbers {
            out.insert(name.clone(), number(*value));
        }
        for (name, value) in &self.structured {
            out.insert(name.clone(), value.clone());
        }
        out
    }

    pub fn names(&self) -> Vec<String> {
        self.to_map().keys().cloned().collect()
    }
}

/// `f64` to JSON, preferring an integer when the value is exactly one so that `1000` does not
/// become `1000.0` on an element card. `crate::fmt::stringify` makes the same choice for the
/// same reason; this keeps the two agreeing at the `Value` level rather than only at the string
/// level.
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.007_199_254_740_992e15 {
        return Value::from(v as i64);
    }
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// §5.1's three string forms. A value that is not a string is already resolved.
enum Form {
    /// `"{Rload * 2}"`
    Expression(String),
    /// `"Rload"`
    Reference(String),
    /// Anything else — a string that is a value in its own right, like a `cscript` block's
    /// `lib`. Deliberately *not* treated as a reference: §5.1's reference form matches an
    /// identifier, and `./ctrl.so` is not one.
    Literal,
}

fn classify(text: &str) -> Form {
    let trimmed = text.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Form::Expression(trimmed[1..trimmed.len() - 1].to_string());
    }
    let mut chars = trimmed.chars();
    let identifier = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if identifier {
        return Form::Reference(trimmed.to_string());
    }
    Form::Literal
}

/// Resolves `model.params` topologically.
///
/// Note what is *not* here: no fixed-point iteration and no "resolve until nothing changes".
/// A cycle has to be a named error rather than a loop that stops making progress, because the
/// message is the deliverable — a user with `A = {B + 1}`, `B = {A + 1}` needs to be shown the
/// loop, not told that resolution gave up.
pub fn resolve_table(model: &Model) -> Result<ResolvedParams, ParamError> {
    let mut out = ResolvedParams::default();
    let mut pending: BTreeMap<&str, Option<Expr>> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();

    for ModelParam { name, value, .. } in &model.params {
        match value {
            Value::Number(n) => {
                out.numbers
                    .insert(name.clone(), n.as_f64().unwrap_or_default());
            }
            Value::Bool(b) => {
                out.numbers.insert(name.clone(), f64::from(*b));
            }
            // A structured constant (`docs/decisions/0009` §3) is a value, not an expression.
            Value::Array(_) => {
                out.structured.insert(name.clone(), value.clone());
            }
            Value::String(text) => match classify(text) {
                Form::Expression(body) => {
                    let expr = grammar::parse(&body).map_err(|e| ParamError::Unresolvable {
                        site: Site(format!("sheet parameter '{name}'")),
                        text: text.clone(),
                        reason: e.message,
                    })?;
                    pending.insert(name.as_str(), Some(expr));
                    order.push(name.as_str());
                }
                Form::Reference(target) => {
                    pending.insert(name.as_str(), Some(Expr::Var(target)));
                    order.push(name.as_str());
                }
                Form::Literal => {
                    return Err(ParamError::NotAValue {
                        name: name.clone(),
                        value: value.clone(),
                    })
                }
            },
            _ => {
                return Err(ParamError::NotAValue {
                    name: name.clone(),
                    value: value.clone(),
                })
            }
        }
    }

    // Kahn over the dependency graph, one pass per round; a round that settles nothing means
    // everything left is on, or behind, a cycle.
    let mut remaining: Vec<&str> = order;
    while !remaining.is_empty() {
        let mut settled_any = false;
        let mut still: Vec<&str> = Vec::new();
        for name in remaining {
            let expr = pending[name].as_ref().expect("pending holds every name");
            let ready = expr.dependencies().iter().all(|dep| {
                out.numbers.contains_key(dep.as_str()) || !pending.contains_key(dep.as_str())
            });
            if !ready {
                still.push(name);
                continue;
            }
            // A reference to a structured constant resolves to the constant itself: there is no
            // f64 for it, and §5.2 cannot do arithmetic on one anyway.
            if let Expr::Var(target) = expr {
                if let Some(value) = out.structured.get(target.as_str()) {
                    let value = value.clone();
                    out.structured.insert(name.to_string(), value);
                    settled_any = true;
                    continue;
                }
            }
            let value =
                grammar::eval(expr, &out.numbers).map_err(|e| ParamError::Unresolvable {
                    site: Site(format!("sheet parameter '{name}'")),
                    text: original(model, name),
                    reason: e.message,
                })?;
            out.numbers.insert(name.to_string(), value);
            settled_any = true;
        }
        if !settled_any {
            return Err(ParamError::Cycle {
                cycle: shortest_cycle(&pending, &still),
            });
        }
        remaining = still;
    }

    Ok(out)
}

fn original(model: &Model, name: &str) -> String {
    model
        .params
        .iter()
        .find(|p| p.name == name)
        .map(|p| match &p.value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| name.to_string())
}

/// Walks the stuck set to find one actual loop, so the message prints `A -> B -> A` rather than
/// every parameter that happens to be downstream of a cycle. A user shown the whole stuck set
/// has to work out which edge to cut; a user shown the loop does not.
fn shortest_cycle(pending: &BTreeMap<&str, Option<Expr>>, stuck: &[&str]) -> Vec<String> {
    for start in stuck {
        let mut path: Vec<String> = vec![(*start).to_string()];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert((*start).to_string());
        let mut current = *start;
        while let Some(Some(expr)) = pending.get(current) {
            let next = expr
                .dependencies()
                .into_iter()
                .find(|d| pending.contains_key(d.as_str()));
            let Some(next) = next else { break };
            if next == *start {
                path.push(next);
                return path;
            }
            if !seen.insert(next.clone()) {
                break;
            }
            path.push(next.clone());
            // `pending`'s keys borrow from the model, so step to the stored key rather than the
            // owned copy.
            match pending.keys().find(|k| **k == next.as_str()) {
                Some(k) => current = k,
                None => break,
            }
        }
    }
    // No two-way loop found (a self-reference, `A = {A + 1}`, lands here when it is the only
    // entry): report the stuck set, which is still the honest answer.
    stuck.iter().map(|s| (*s).to_string()).collect()
}

/// Returns a copy of `model` with every §5.3 use site substituted for its resolved value.
///
/// A clone rather than an in-place edit, and the module doc says why: the document on disk
/// keeps the user's expression.
pub fn substitute(model: &Model, table: &ResolvedParams) -> Result<Model, ParamError> {
    let mut out = model.clone();

    // The table itself first, so the invariant this function establishes is the simple one:
    // **after `substitute`, no §5.1 reference and no §5.2 expression remains anywhere in the
    // model.** Everything downstream -- element cards, `ic`, a build's `-D` bindings -- can then
    // read a value without knowing §5.2 exists, which is the whole point of resolving in one
    // place.
    for param in &mut out.params {
        if let Some(resolved) = table.get(&param.name) {
            param.value = resolved;
        }
    }

    for block in &mut out.blocks {
        let at = |field: &str, key: &str| Site(format!("block '{}' {field} '{key}'", block.id));
        substitute_map(&mut block.params, table, &|k| at("parameter", k))?;
        substitute_map(&mut block.ic, table, &|k| at("initial condition", k))?;
    }

    // §5.3's third use site: "any numeric field" of an analysis directive. This crate does not
    // emit analysis directives, so substituting here changes no netlist today -- what it does
    // is stop `"t_end": "{T_sim}"` being *accepted in silence*, which is the half of #94 that
    // is not about the deck. A front end reads these from the substituted model and gets a
    // number; `omni-schematics#244` is the editor-side half of the same bug.
    let mut simulation = Value::Object(std::mem::take(&mut out.simulation));
    substitute_json(
        &mut simulation,
        table,
        &Site("a simulation directive".to_string()),
    )?;
    if let Value::Object(map) = simulation {
        out.simulation = map;
    }

    Ok(out)
}

fn substitute_map(
    map: &mut Map<String, Value>,
    table: &ResolvedParams,
    site: &dyn Fn(&str) -> Site,
) -> Result<(), ParamError> {
    for (key, value) in map.iter_mut() {
        substitute_json(value, table, &site(key))?;
    }
    Ok(())
}

/// Substitutes in place, descending into arrays and objects so a matrix parameter whose entries
/// are references resolves element by element — `[[R, 0], [0, R]]` is a real way to write a
/// state-space matrix, and leaving the leaves alone would emit `R` into the deck.
fn substitute_json(
    value: &mut Value,
    table: &ResolvedParams,
    site: &Site,
) -> Result<(), ParamError> {
    match value {
        Value::String(text) => {
            let (body, expr) = match classify(text) {
                Form::Expression(body) => {
                    let parsed = grammar::parse(&body).map_err(|e| ParamError::Unresolvable {
                        site: site.clone(),
                        text: text.clone(),
                        reason: e.message,
                    })?;
                    (text.clone(), parsed)
                }
                Form::Reference(name) => {
                    // A §5.1 reference to a structured constant hands over the constant.
                    if let Some(resolved @ Value::Array(_)) = table.get(&name) {
                        *value = resolved;
                        return Ok(());
                    }
                    // A bare identifier that is not a declared parameter is left alone. It is
                    // how every netlist-level name is written -- `ts=variable`, a `kind=`
                    // variant, an output name -- and treating those as unresolved references
                    // would refuse almost every sheet.
                    if !table.contains(&name) {
                        return Ok(());
                    }
                    (text.clone(), Expr::Var(name))
                }
                Form::Literal => return Ok(()),
            };
            let resolved =
                grammar::eval(&expr, table.numbers()).map_err(|e| ParamError::Unresolvable {
                    site: site.clone(),
                    text: body,
                    reason: e.message,
                })?;
            *value = number(resolved);
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                substitute_json(item, table, site)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                substitute_json(item, table, site)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

impl ResolvedParams {
    fn numbers(&self) -> &BTreeMap<String, f64> {
        &self.numbers
    }
}

impl From<&str> for Site {
    fn from(s: &str) -> Self {
        Site(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::GsimSchDocument;
    use serde_json::json;

    fn model(params: Value) -> Model {
        let doc = json!({
            "schema_version": 2,
            "model": { "id": "m", "name": "M", "params": params, "blocks": [], "wires": [] }
        });
        GsimSchDocument::from_json(&doc.to_string())
            .expect("parses")
            .model
    }

    #[test]
    fn a_table_resolves_in_dependency_order_not_document_order() {
        let m = model(json!([
            { "name": "Rseries", "value": "{Rload * 0.1}" },
            { "name": "Rload", "value": 1000 },
            { "name": "Rtotal", "value": "{Rload + Rseries}" }
        ]));
        let t = resolve_table(&m).unwrap();
        assert_eq!(t.get("Rload"), Some(json!(1000)));
        assert_eq!(t.get("Rseries"), Some(json!(100)));
        assert_eq!(t.get("Rtotal"), Some(json!(1100)));
    }

    #[test]
    fn a_bare_reference_resolves_too() {
        let m = model(json!([
            { "name": "Rload", "value": 1000 },
            { "name": "Rsense", "value": "Rload" }
        ]));
        assert_eq!(resolve_table(&m).unwrap().get("Rsense"), Some(json!(1000)));
    }

    #[test]
    fn a_cycle_is_rejected_and_the_message_prints_the_loop() {
        let m = model(json!([
            { "name": "A", "value": "{B + 1}" },
            { "name": "B", "value": "{A + 1}" }
        ]));
        let e = resolve_table(&m).unwrap_err();
        match &e {
            ParamError::Cycle { cycle } => {
                assert!(cycle.contains(&"A".to_string()) && cycle.contains(&"B".to_string()));
            }
            other => panic!("expected a cycle: {other}"),
        }
        let message = e.to_string();
        assert!(
            message.contains("A -> B -> A") || message.contains("B -> A -> B"),
            "{message}"
        );
    }

    #[test]
    fn a_self_reference_is_a_cycle_as_well() {
        let m = model(json!([{ "name": "A", "value": "{A + 1}" }]));
        assert!(matches!(resolve_table(&m), Err(ParamError::Cycle { .. })));
    }

    #[test]
    fn a_structured_constant_is_a_value_and_a_reference_to_one_hands_it_over() {
        let m = model(json!([
            { "name": "lut", "value": [1, 2, 3] },
            { "name": "alias", "value": "lut" }
        ]));
        let t = resolve_table(&m).unwrap();
        assert_eq!(t.get("lut"), Some(json!([1, 2, 3])));
        assert_eq!(t.get("alias"), Some(json!([1, 2, 3])));
    }

    #[test]
    fn a_whole_number_resolves_to_an_integer_so_a_card_does_not_carry_a_trailing_zero() {
        let m = model(json!([{ "name": "n", "value": "{2 * 500}" }]));
        assert_eq!(resolve_table(&m).unwrap().get("n"), Some(json!(1000)));
    }

    #[test]
    fn substitution_leaves_a_string_that_is_not_a_reference_alone() {
        let m = model(json!([{ "name": "R", "value": 10 }]));
        let table = resolve_table(&m).unwrap();
        let mut value = json!({ "lib": "./ctrl.so", "ts": "variable", "R": "R" });
        substitute_json(&mut value, &table, &Site("test".into())).unwrap();
        assert_eq!(value["lib"], json!("./ctrl.so"));
        assert_eq!(value["ts"], json!("variable"));
        // And the one that *is* a declared parameter resolves.
        assert_eq!(value["R"], json!(10));
    }

    #[test]
    fn a_matrix_of_references_resolves_leaf_by_leaf() {
        let m = model(json!([{ "name": "k", "value": 4 }]));
        let table = resolve_table(&m).unwrap();
        let mut value = json!([["k", 0], [0, "{k / 2}"]]);
        substitute_json(&mut value, &table, &Site("test".into())).unwrap();
        assert_eq!(value, json!([[4, 0], [0, 2]]));
    }
}
