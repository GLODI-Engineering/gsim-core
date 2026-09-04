//! PyO3 bindings for `gsim-core`.
//!
//! # Interface design
//!
//! Every function here is **stateless and JSON-in/JSON-out**: it takes Python `str` (JSON text),
//! parses it with `serde_json`, calls straight into `gsim-core`, and returns Python `str` (JSON
//! text) or raises a `ValueError` carrying the real Rust error's `Display` message. There is no
//! stateful Rust object (e.g. `ModelBuilder`) exposed across the FFI boundary — a caller builds a
//! `Model` up incrementally by round-tripping its JSON through successive calls
//! (`new_model` -> `add_block` -> `connect` -> ... -> `save_document`). This matches `gsim-core`'s
//! own internal philosophy of JSON as the interchange format everywhere, avoids needing GC/
//! lifetime integration for a wrapped Rust object living inside Python's object model, and keeps
//! this crate a thin wrapper rather than a second place schema/validation/layout logic could
//! drift. `crates/gsim-wasm` exposes the same JSON contracts to JavaScript.
//!
//! No function here reimplements schema, validation, or layout logic — see `AGENTS.md`'s "one
//! implementation, thin bindings" rule. A couple of small additive helpers were added to
//! `gsim-core` itself to support this thin-wrapper shape without duplicating shape definitions in
//! this crate: `Deserialize` on `document::NewBlockSpec`/`document::ConnectOptions`, `Serialize`
//! on `contract::ValidationFailure`, and `Serialize`/`Deserialize` on `layout::Rect` — see each
//! item's doc comment in `gsim-core` for why.

use gsim_core::compile::{compile as compile_model, CompileError};
use gsim_core::contract::{self, ComponentContract};
use gsim_core::document::{
    BuilderError, CompatFailure, ConnectOptions, Endpoint, GsimSchDocument, Model, ModelBuilder,
    NewBlockSpec, NewSubcktInstanceSpec, SubcktDef, SubcktError,
};
use gsim_core::layout::{self, Axis, LayoutInput, Rect};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Converts any `Display`-able error into a Python `ValueError` carrying its real message —
/// never an opaque panic, and never a generic message that swallows which specific error variant
/// occurred.
fn to_py_err<E: std::fmt::Display>(e: E) -> PyErr {
    PyErr::new::<PyValueError, _>(e.to_string())
}

/// Like [`to_py_err`], but for `BuilderError` specifically: prefixes the real `Display` message
/// with the variant's canonical keyword ("duplicate", "dangling", "cross-paradigm", ...) so a
/// caller can reliably match on *which* case occurred from the message text alone, even though
/// `BuilderError`'s own `Display` impl phrases some variants (`CrossParadigm`, `DanglingWire`)
/// descriptively rather than using that exact word. The full underlying message always follows,
/// unmodified — this never swallows detail, only adds a stable keyword in front of it.
fn builder_err_to_py(e: BuilderError) -> PyErr {
    let keyword = match &e {
        BuilderError::DuplicateId(_) => "duplicate",
        BuilderError::DanglingWire { .. } => "dangling",
        BuilderError::CrossParadigm { .. } => "cross-paradigm",
        // One keyword per `port-compat.ts` reason rather than a single "port-incompatible":
        // the three cases need different fixes (drop a v3-reserved kind / correct the wire kind
        // / insert a routing.merge), so a caller branching on them is the expected use. Same
        // sub-matching precedent as `Subckt(CyclicDefReference)` below.
        BuilderError::PortIncompatible(inc) => match &inc.failure {
            CompatFailure::ReservedKind { .. } => "port-reserved-kind",
            CompatFailure::KindMismatch { .. } => "port-kind-mismatch",
            CompatFailure::FanInOnSignal { .. } => "port-fan-in-on-signal",
        },
        BuilderError::UnknownBlock(_) => "unknown-block",
        BuilderError::Block(_) => "invalid-block",
        BuilderError::UnknownSubcktDef(_) => "unknown-subckt-def",
        BuilderError::Subckt(SubcktError::CyclicDefReference { .. }) => "cyclic-subckt-def",
        BuilderError::Subckt(_) => "invalid-subckt",
    };
    PyErr::new::<PyValueError, _>(format!("{keyword}: {e}"))
}

/// Same stable-keyword-prefix idea as [`builder_err_to_py`], for the compiler's errors.
fn compile_err_to_py(e: CompileError) -> PyErr {
    let keyword = match &e {
        CompileError::UnknownType { .. } => "unknown-type",
        CompileError::UnknownTarget(_) => "unknown-target",
        CompileError::FloatingPort { .. } => "floating-port",
        CompileError::UnconnectedRef { .. } => "unconnected-ref",
        CompileError::MissingParam { .. } => "missing-param",
        CompileError::UnknownSubcktDef { .. } => "unknown-subckt-def",
        CompileError::UnconnectedSubcktPort { .. } => "unconnected-subckt-port",
        CompileError::UnknownVariant { .. } => "unknown-variant",
        CompileError::VectorOutputPort { .. } => "vector-output-port",
        CompileError::IndexedRefArity { .. } => "indexed-ref-arity",
        CompileError::UnresolvedFromTag { .. } => "unresolved-from-tag",
        CompileError::AmbiguousFromTag { .. } => "ambiguous-from-tag",
        CompileError::CyclicFromTag { .. } => "cyclic-from-tag",
        CompileError::RoutingKindUnsupported { .. } => "routing-kind-unsupported",
    };
    PyErr::new::<PyValueError, _>(format!("{keyword}: {e}"))
}

fn parse_json<T: for<'de> Deserialize<'de>>(text: &str, what: &str) -> PyResult<T> {
    serde_json::from_str(text).map_err(|e| to_py_err(format!("invalid {what} JSON: {e}")))
}

fn to_json<T: Serialize>(value: &T) -> PyResult<String> {
    serde_json::to_string(value).map_err(to_py_err)
}

#[pyfunction]
fn version() -> &'static str {
    gsim_core::version()
}

/// Parses `contract_json` as a `ComponentContract` and `params_json` as a JSON object of
/// parameter values, evaluates it (GSF v2), and returns the resulting `EvaluatedSymbol`
/// (`{"bbox": [w, h], "ports": [...], "primitives": [...]}`) as JSON.
#[pyfunction]
fn evaluate_component(contract_json: &str, params_json: &str) -> PyResult<String> {
    let contract: ComponentContract = parse_json(contract_json, "contract")?;
    let params: Map<String, Value> = parse_json(params_json, "params")?;
    let evaluated = contract::evaluate_component(&contract, &params).map_err(to_py_err)?;
    to_json(&evaluated)
}

/// Returns a JSON array of `{"parameter": ..., "message": ...}` failures (empty array `"[]"`
/// means every validated parameter passed). Never raises for validation failures themselves —
/// only for malformed input JSON.
#[pyfunction]
fn validate_parameters(contract_json: &str, params_json: &str) -> PyResult<String> {
    let contract: ComponentContract = parse_json(contract_json, "contract")?;
    let params: Map<String, Value> = parse_json(params_json, "params")?;
    let failures = contract::validate_parameters(&contract, &params);
    to_json(&failures)
}

/// Returns a fresh, empty `Model` as JSON.
#[pyfunction]
fn new_model(id: &str, name: &str, grid: i64) -> PyResult<String> {
    to_json(&Model::new(id, name, grid))
}

#[derive(Serialize)]
struct AddBlockResult {
    model: Model,
    block_id: String,
}

/// `spec_json` matches `NewBlockSpec`'s JSON shape (`type`, `paradigm`, `domain`, `fidelity?`,
/// `position`, `rotation`, `mirror?`, `params?`, `ic?`, `label?`, `comment_before?`,
/// `comment_after?`, `id?`). `comment_before`/`comment_after` are lists of comment lines (a
/// single string containing newlines is split too) emitted as `*`-prefixed SPICE comment lines
/// immediately above/below this block's card(s) by `compile`. Returns
/// `{"model": <updated Model JSON>, "block_id": "<assigned id>"}`. Raises with the real
/// `BuilderError`/`BlockError` message (duplicate id, invalid id/type pattern, invalid rotation)
/// on failure.
#[pyfunction]
fn add_block(model_json: &str, spec_json: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let spec: NewBlockSpec = parse_json(spec_json, "block spec")?;
    let mut builder = ModelBuilder::from_model(model);
    let block_id = builder.add_block(spec).map_err(builder_err_to_py)?;
    to_json(&AddBlockResult {
        model: builder.into_model(),
        block_id,
    })
}

#[derive(Serialize)]
struct AddSubcktDefResult {
    model: Model,
    def_id: String,
}

/// Adds a reusable SPICE `.SUBCKT` definition to the model.
///
/// **Shared JSON contract** (identical in `gsim-wasm`'s `addSubcktDef`):
/// `def_json` is a `SubcktDef` — `{"id", "name", "ports": [{"id", "kind"}], "params"?,
/// "blocks"?, "wires"?, "instances"? : [{"id", "def_ref", "connections"?, "params"?}]}`.
/// Returns `{"model": <updated Model JSON>, "def_id": "<id>"}`, mirroring `add_block`'s
/// `{"model", "block_id"}` shape. Raises on an invalid id pattern (`invalid-subckt: ...`), an
/// invalid port kind (`invalid-subckt: ...`), a duplicate def id (`duplicate: ...`), or a
/// cyclic `def_ref` graph (`cyclic-subckt-def: Cyclic SubcktDef reference: a -> b -> a`).
#[pyfunction]
fn add_subckt_def(model_json: &str, def_json: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let def: SubcktDef = parse_json(def_json, "subckt def")?;
    let mut builder = ModelBuilder::from_model(model);
    let def_id = builder.add_subckt_def(def).map_err(builder_err_to_py)?;
    to_json(&AddSubcktDefResult {
        model: builder.into_model(),
        def_id,
    })
}

/// Instantiates an existing `SubcktDef` as a top-level `spice-subckt` block.
///
/// **Shared JSON contract** (identical in `gsim-wasm`'s `instantiateSubckt`): `spec_json`
/// matches `NewSubcktInstanceSpec` — `{"def_ref", "position": [x, y], "rotation"?, "domain"?,
/// "fidelity"?, "mirror"?, "params"?, "label"?, "comment_before"?, "comment_after"?, "id"?}`.
/// Returns
/// `{"model": <updated Model JSON>, "block_id": "<assigned id>"}`, the same shape `add_block`
/// returns. The created block's wireable ports are exactly the referenced def's `ports[].id`
/// list — wire them with ordinary `connect` calls. Raises `unknown-subckt-def: ...` if
/// `def_ref` names no def, `duplicate: ...` on a colliding block id.
#[pyfunction]
fn instantiate_subckt(model_json: &str, spec_json: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let spec: NewSubcktInstanceSpec = parse_json(spec_json, "subckt instance spec")?;
    let mut builder = ModelBuilder::from_model(model);
    let block_id = builder
        .instantiate_subckt(spec)
        .map_err(builder_err_to_py)?;
    to_json(&AddBlockResult {
        model: builder.into_model(),
        block_id,
    })
}

#[derive(Serialize)]
struct CompileResult {
    netlist: String,
    notes: Vec<String>,
}

/// Compiles a `Model` into netlist text for `target` (currently only `"general-simulator"`).
///
/// **Shared JSON contract** (identical in `gsim-wasm`'s `compile`): returns
/// `{"netlist": "<text>", "notes": ["<info-level compiler decision>", ...]}`. Raises a
/// `ValueError` whose message is keyword-prefixed the same way the builder errors are
/// (`unknown-type:`, `unknown-target:`, `floating-port:`, `unconnected-ref:`,
/// `missing-param:`, `unknown-subckt-def:`, `unconnected-subckt-port:`, `unknown-variant:`, `vector-output-port:`,
/// `indexed-ref-arity:`, `unresolved-from-tag:`, `ambiguous-from-tag:`, `cyclic-from-tag:`,
/// `routing-kind-unsupported:`),
/// with the real underlying message following unmodified.
#[pyfunction]
fn compile(model_json: &str, target: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let out = compile_model(&model, target).map_err(compile_err_to_py)?;
    to_json(&CompileResult {
        netlist: out.netlist,
        notes: out.notes,
    })
}

/// Removes a block and cascades to every wire touching it. Returns the updated `Model` JSON.
#[pyfunction]
fn remove_block(model_json: &str, block_id: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let mut builder = ModelBuilder::from_model(model);
    builder.remove_block(block_id);
    to_json(&builder.into_model())
}

/// Sets one parameter on one block. Returns the updated `Model` JSON. Raises if `block_id`
/// doesn't exist.
#[pyfunction]
fn set_param(model_json: &str, block_id: &str, name: &str, value_json: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let value: Value = parse_json(value_json, "value")?;
    let mut builder = ModelBuilder::from_model(model);
    builder
        .set_param(block_id, name, value)
        .map_err(builder_err_to_py)?;
    to_json(&builder.into_model())
}

/// Adds a model-level (schematic-wide) parameter. Returns the updated `Model` JSON. Raises on a
/// duplicate parameter name.
#[pyfunction]
fn add_model_param(model_json: &str, name: &str, value_json: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let value: Value = parse_json(value_json, "value")?;
    let mut builder = ModelBuilder::from_model(model);
    builder
        .add_model_param(name, value)
        .map_err(builder_err_to_py)?;
    to_json(&builder.into_model())
}

#[derive(Serialize)]
struct ConnectResult {
    model: Model,
    wire_id: String,
}

/// `from_json`/`to_json` are `{"block": "...", "port": "..."}` endpoints; `opts_json` matches
/// `ConnectOptions` (`waypoints?`, `net?`, `id?`, `pinned?`, `label?`, all optional -- `"{}"` is a
/// valid payload). Returns `{"model": <updated Model JSON>, "wire_id": "<assigned id>"}`. Raises
/// with the real `BuilderError` message on failure -- the message text always names the specific
/// case ("dangling", "cross-paradigm", "duplicate") rather than a generic "connect failed", since
/// `BuilderError`'s `Display` impl already does this and this wrapper does not paper over it.
///
/// Beyond the block-level cross-paradigm boundary, the per-port §8.1/§8.2 rules in
/// `gsim_core::document::port_compat` also apply, raising with the keyword
/// `port-reserved-kind`, `port-kind-mismatch` or `port-fan-in-on-signal`. That module documents
/// exactly which of the frontend's `port-compat.ts` rules this crate can enforce without a
/// component-contract registry, and which remain open.
#[pyfunction]
fn connect(
    model_json: &str,
    from_json: &str,
    to_json_: &str,
    kind: &str,
    opts_json: &str,
) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let from: Endpoint = parse_json(from_json, "from endpoint")?;
    let to: Endpoint = parse_json(to_json_, "to endpoint")?;
    let opts: ConnectOptions = parse_json(opts_json, "connect options")?;
    let mut builder = ModelBuilder::from_model(model);
    let wire_id = builder
        .connect(from, to, kind, opts)
        .map_err(builder_err_to_py)?
        .to_string();
    to_json(&ConnectResult {
        model: builder.into_model(),
        wire_id,
    })
}

/// `bbox_map_json` is `{"<block_id>": [w, h], ...}` for each block in the model -- the caller is
/// expected to have already evaluated each block's GSF v2 contract to get its bbox (this
/// function does not itself know about contracts, matching `ModelBuilder::auto_arrange`'s
/// decoupled `bbox_of` design). If a block id is missing from the map, `(0, 0)` is used as its
/// size rather than raising. Returns the updated `Model` JSON with positions rewritten.
#[pyfunction]
fn auto_arrange(
    model_json: &str,
    bbox_map_json: &str,
    column_gap: i64,
    row_gap: i64,
) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    let bbox_map: HashMap<String, (i64, i64)> = parse_json(bbox_map_json, "bbox map")?;
    let mut builder = ModelBuilder::from_model(model);
    builder.auto_arrange(
        |b| bbox_map.get(&b.id).copied().unwrap_or((0, 0)),
        column_gap,
        row_gap,
    );
    to_json(&builder.into_model())
}

/// Parses `json` as a full `.gsim_sch` document (rejects a non-v2 `schema_version` with a clear
/// message), and returns the canonical re-serialized JSON -- proving round-trip validity.
#[pyfunction]
fn load_document(json: &str) -> PyResult<String> {
    let doc = GsimSchDocument::from_json(json).map_err(to_py_err)?;
    to_json(&doc)
}

/// Wraps a `Model` JSON into a full `{"schema_version": 2, "model": ...}` document, pretty-
/// printed.
#[pyfunction]
fn save_document(model_json: &str) -> PyResult<String> {
    let model: Model = parse_json(model_json, "model")?;
    GsimSchDocument::new(model)
        .to_json_pretty()
        .map_err(to_py_err)
}

/// `existing_json` is a JSON array of `{"x":.., "y":.., "w":.., "h":..}` rectangles; `axis` is
/// `"horizontal"` or `"vertical"`. Returns `[x, y]` as a JSON array.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn place_without_overlap(
    existing_json: &str,
    w: i64,
    h: i64,
    desired_x: i64,
    desired_y: i64,
    axis: &str,
    margin: i64,
    grid: i64,
) -> PyResult<String> {
    let existing: Vec<Rect> = parse_json(existing_json, "existing rects")?;
    let axis = match axis {
        "horizontal" => Axis::Horizontal,
        "vertical" => Axis::Vertical,
        other => {
            return Err(to_py_err(format!(
                "invalid axis '{other}' (must be \"horizontal\" or \"vertical\")"
            )))
        }
    };
    let (x, y) = layout::place_without_overlap(
        &existing,
        (w, h),
        (desired_x, desired_y),
        axis,
        margin,
        grid,
    );
    to_json(&(x, y))
}

#[derive(Deserialize)]
struct LayoutBlockSpec {
    id: String,
    w: i64,
    h: i64,
}

/// `blocks_json` is `[{"id": "...", "w": .., "h": ..}, ...]`, `edges_json` is
/// `[["from_id","to_id"], ...]`. Returns `{"<block_id>": [x, y], ...}`.
#[pyfunction]
fn layered_layout(
    blocks_json: &str,
    edges_json: &str,
    column_gap: i64,
    row_gap: i64,
    grid: i64,
) -> PyResult<String> {
    let block_specs: Vec<LayoutBlockSpec> = parse_json(blocks_json, "blocks")?;
    let edge_specs: Vec<(String, String)> = parse_json(edges_json, "edges")?;

    let inputs: Vec<LayoutInput> = block_specs
        .iter()
        .map(|b| LayoutInput {
            id: &b.id,
            size: (b.w, b.h),
        })
        .collect();
    let edges: Vec<(&str, &str)> = edge_specs
        .iter()
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect();

    let positions = layout::layered_layout(&inputs, &edges, column_gap, row_gap, grid);
    to_json(&positions)
}

#[pymodule]
fn gsim_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(evaluate_component, m)?)?;
    m.add_function(wrap_pyfunction!(validate_parameters, m)?)?;
    m.add_function(wrap_pyfunction!(new_model, m)?)?;
    m.add_function(wrap_pyfunction!(add_block, m)?)?;
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    m.add_function(wrap_pyfunction!(add_subckt_def, m)?)?;
    m.add_function(wrap_pyfunction!(instantiate_subckt, m)?)?;
    m.add_function(wrap_pyfunction!(remove_block, m)?)?;
    m.add_function(wrap_pyfunction!(set_param, m)?)?;
    m.add_function(wrap_pyfunction!(add_model_param, m)?)?;
    m.add_function(wrap_pyfunction!(connect, m)?)?;
    m.add_function(wrap_pyfunction!(auto_arrange, m)?)?;
    m.add_function(wrap_pyfunction!(load_document, m)?)?;
    m.add_function(wrap_pyfunction!(save_document, m)?)?;
    m.add_function(wrap_pyfunction!(place_without_overlap, m)?)?;
    m.add_function(wrap_pyfunction!(layered_layout, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_flow_new_model_add_blocks_connect_auto_arrange_save_load_round_trips() {
        let model = new_model("m1", "My Model", 10).unwrap();

        let spec1 = r#"{"type":"signal.step","paradigm":"signal-flow","domain":"control","position":[0,0],"rotation":0,"id":"src1"}"#;
        let add1: Value = serde_json::from_str(&add_block(&model, spec1).unwrap()).unwrap();
        let model = add1["model"].to_string();
        assert_eq!(add1["block_id"], "src1");

        let spec2 = r#"{"type":"control.gain","paradigm":"signal-flow","domain":"control","position":[0,0],"rotation":0,"id":"gain1"}"#;
        let add2: Value = serde_json::from_str(&add_block(&model, spec2).unwrap()).unwrap();
        let model = add2["model"].to_string();
        assert_eq!(add2["block_id"], "gain1");

        let from = r#"{"block":"src1","port":"out"}"#;
        let to = r#"{"block":"gain1","port":"in"}"#;
        let conn: Value =
            serde_json::from_str(&connect(&model, from, to, "signal", "{}").unwrap()).unwrap();
        let model = conn["model"].to_string();
        assert_eq!(conn["wire_id"], "wire1");

        let bbox_map = r#"{"src1": [40, 20], "gain1": [40, 20]}"#;
        let model = auto_arrange(&model, bbox_map, 20, 10).unwrap();

        let saved = save_document(&model).unwrap();
        assert!(saved.contains("\"schema_version\": 2"));

        let reloaded = load_document(&saved).unwrap();
        let reloaded_val: Value = serde_json::from_str(&reloaded).unwrap();
        assert_eq!(reloaded_val["model"]["id"], "m1");
        assert_eq!(reloaded_val["model"]["blocks"].as_array().unwrap().len(), 2);
        assert_eq!(reloaded_val["model"]["wires"].as_array().unwrap().len(), 1);

        // auto_arrange actually moved src1 before gain1.
        let model_val: Value = serde_json::from_str(&model).unwrap();
        let blocks = model_val["blocks"].as_array().unwrap();
        let src_x = blocks.iter().find(|b| b["id"] == "src1").unwrap()["position"][0]
            .as_i64()
            .unwrap();
        let gain_x = blocks.iter().find(|b| b["id"] == "gain1").unwrap()["position"][0]
            .as_i64()
            .unwrap();
        assert!(src_x < gain_x);
    }

    #[test]
    fn remove_block_cascades_wire_removal() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec1 = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"r1"}"#;
        let add1: Value = serde_json::from_str(&add_block(&model, spec1).unwrap()).unwrap();
        let model = add1["model"].to_string();
        let spec2 = r#"{"type":"passive.capacitor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"c1"}"#;
        let add2: Value = serde_json::from_str(&add_block(&model, spec2).unwrap()).unwrap();
        let model = add2["model"].to_string();

        let from = r#"{"block":"r1","port":"n"}"#;
        let to = r#"{"block":"c1","port":"p"}"#;
        let conn: Value =
            serde_json::from_str(&connect(&model, from, to, "physical-electrical", "{}").unwrap())
                .unwrap();
        let model = conn["model"].to_string();

        let after_remove: Value =
            serde_json::from_str(&remove_block(&model, "r1").unwrap()).unwrap();
        assert_eq!(after_remove["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(after_remove["wires"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn set_param_and_add_model_param_round_trip() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"r1"}"#;
        let add: Value = serde_json::from_str(&add_block(&model, spec).unwrap()).unwrap();
        let model = add["model"].to_string();

        let model = set_param(&model, "r1", "R", "1000").unwrap();
        let model_val: Value = serde_json::from_str(&model).unwrap();
        assert_eq!(model_val["blocks"][0]["params"]["R"], 1000);

        let model = add_model_param(&model, "vcc", "5.0").unwrap();
        let model_val: Value = serde_json::from_str(&model).unwrap();
        assert_eq!(model_val["params"][0]["name"], "vcc");
        assert_eq!(model_val["params"][0]["value"], 5.0);
    }

    #[test]
    fn set_param_on_unknown_block_raises_with_real_message() {
        let model = new_model("m1", "M", 10).unwrap();
        let err = set_param(&model, "ghost", "R", "1000").unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(msg.contains("no block with id 'ghost'"), "got: {msg}");
        });
    }

    #[test]
    fn connect_dangling_endpoint_raises_with_dangling_in_message() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"r1"}"#;
        let add: Value = serde_json::from_str(&add_block(&model, spec).unwrap()).unwrap();
        let model = add["model"].to_string();

        let from = r#"{"block":"r1","port":"p"}"#;
        let to = r#"{"block":"ghost","port":"p"}"#;
        let err = connect(&model, from, to, "physical-electrical", "{}").unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(
                msg.to_lowercase().contains("unknown block")
                    || msg.to_lowercase().contains("dangling")
                    || msg.contains("ghost"),
                "got: {msg}"
            );
        });
    }

    #[test]
    fn connect_cross_paradigm_raises_with_cross_paradigm_in_message() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec1 = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"r1"}"#;
        let add1: Value = serde_json::from_str(&add_block(&model, spec1).unwrap()).unwrap();
        let model = add1["model"].to_string();
        let spec2 = r#"{"type":"control.gain","paradigm":"signal-flow","domain":"control","position":[0,0],"rotation":0,"id":"gain1"}"#;
        let add2: Value = serde_json::from_str(&add_block(&model, spec2).unwrap()).unwrap();
        let model = add2["model"].to_string();

        let from = r#"{"block":"r1","port":"p"}"#;
        let to = r#"{"block":"gain1","port":"in"}"#;
        let err = connect(&model, from, to, "signal", "{}").unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            // `BuilderError::CrossParadigm`'s `Display` names both paradigms and the reason
            // rather than using the literal word "cross-paradigm" -- this asserts the specific
            // variant's message (not a generic "connect failed"), matching what the task
            // description means by "include ... literally".
            assert!(
                msg.contains(
                    "cannot connect a PhysicalNetwork block directly to a SignalFlow block"
                ),
                "got: {msg}"
            );
        });
    }

    #[test]
    fn connect_two_different_physical_domains_raises_port_kind_mismatch() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec1 = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"r1"}"#;
        let add1: Value = serde_json::from_str(&add_block(&model, spec1).unwrap()).unwrap();
        let model = add1["model"].to_string();
        let spec2 = r#"{"type":"thermal.resistance","paradigm":"physical-network","domain":"thermal","position":[0,0],"rotation":0,"id":"rth"}"#;
        let add2: Value = serde_json::from_str(&add_block(&model, spec2).unwrap()).unwrap();
        let model = add2["model"].to_string();

        let from = r#"{"block":"r1","port":"p"}"#;
        let to = r#"{"block":"rth","port":"a"}"#;
        let err = connect(&model, from, to, "physical-electrical", "{}").unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            // The stable keyword prefix, then the full underlying message naming both the port
            // and the kind it actually declares -- the wrapper adds, never replaces.
            assert!(msg.starts_with("port-kind-mismatch: "), "got: {msg}");
            assert!(msg.contains("'rth.a' is declared 'thermal'"), "got: {msg}");
            assert!(
                msg.contains("as a 'physical-electrical' wire"),
                "got: {msg}"
            );
        });
    }

    #[test]
    fn connect_fanning_in_on_a_signal_port_raises_port_fan_in_on_signal() {
        let mut model = new_model("m1", "M", 10).unwrap();
        for id in ["s1", "s2", "g1"] {
            let spec = format!(
                r#"{{"type":"control.gain","paradigm":"signal-flow","domain":"control","position":[0,0],"rotation":0,"id":"{id}"}}"#
            );
            let add: Value = serde_json::from_str(&add_block(&model, &spec).unwrap()).unwrap();
            model = add["model"].to_string();
        }
        let to = r#"{"block":"g1","port":"in"}"#;
        let first: Value = serde_json::from_str(
            &connect(&model, r#"{"block":"s1","port":"out"}"#, to, "signal", "{}").unwrap(),
        )
        .unwrap();
        let model = first["model"].to_string();

        let err =
            connect(&model, r#"{"block":"s2","port":"out"}"#, to, "signal", "{}").unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(msg.starts_with("port-fan-in-on-signal: "), "got: {msg}");
            assert!(
                msg.contains("causal port 'g1.in' is already driven"),
                "got: {msg}"
            );
        });
    }

    #[test]
    fn connect_a_reserved_kind_raises_port_reserved_kind() {
        let mut model = new_model("m1", "M", 10).unwrap();
        for id in ["m1b", "m2b"] {
            let spec = format!(
                r#"{{"type":"magnetic.core","paradigm":"physical-network","domain":"mixed","position":[0,0],"rotation":0,"id":"{id}"}}"#
            );
            let add: Value = serde_json::from_str(&add_block(&model, &spec).unwrap()).unwrap();
            model = add["model"].to_string();
        }
        let err = connect(
            &model,
            r#"{"block":"m1b","port":"p"}"#,
            r#"{"block":"m2b","port":"p"}"#,
            "reserved.physical-magnetic",
            "{}",
        )
        .unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(msg.starts_with("port-reserved-kind: "), "got: {msg}");
            assert!(msg.contains("reserved.physical-magnetic"), "got: {msg}");
        });
    }

    #[test]
    fn add_block_duplicate_id_raises_with_duplicate_in_message() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"r1"}"#;
        let add: Value = serde_json::from_str(&add_block(&model, spec).unwrap()).unwrap();
        let model = add["model"].to_string();

        let err = add_block(&model, spec).unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(msg.to_lowercase().contains("duplicate"), "got: {msg}");
        });
    }

    #[test]
    fn add_block_invalid_rotation_raises() {
        let model = new_model("m1", "M", 10).unwrap();
        let spec = r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":45,"id":"r1"}"#;
        let err = add_block(&model, spec).unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(msg.to_lowercase().contains("rotation"), "got: {msg}");
        });
    }

    #[test]
    fn bad_json_raises_value_error_not_panic() {
        let result = add_block(r#"{not json"#, "{}");
        assert!(result.is_err());
    }

    #[test]
    fn evaluate_component_and_validate_parameters_on_a_real_contract() {
        let contract_json = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../gsim-core/tests/fixtures/general_simulator/control.sum.json"
        ))
        .unwrap();

        let evaluated = evaluate_component(&contract_json, r#"{"signs": [1, 1, -1]}"#).unwrap();
        let evaluated_val: Value = serde_json::from_str(&evaluated).unwrap();
        assert_eq!(evaluated_val["ports"].as_array().unwrap().len(), 4); // 3 inputs + out

        let failures = validate_parameters(&contract_json, r#"{"signs": []}"#).unwrap();
        let failures_val: Value = serde_json::from_str(&failures).unwrap();
        assert_eq!(failures_val.as_array().unwrap().len(), 1);
        assert_eq!(failures_val[0]["parameter"], "signs");

        let ok = validate_parameters(&contract_json, r#"{"signs": [1, -1]}"#).unwrap();
        assert_eq!(ok, "[]");
    }

    #[test]
    fn evaluate_component_bad_contract_json_raises() {
        let err = evaluate_component("{not json", "{}").unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(!msg.is_empty());
        });
    }

    #[test]
    fn place_without_overlap_steps_around_a_blocker() {
        let existing = r#"[{"x":100,"y":100,"w":20,"h":20}]"#;
        let result =
            place_without_overlap(existing, 20, 20, 100, 100, "horizontal", 5, 10).unwrap();
        let pos: (i64, i64) = serde_json::from_str(&result).unwrap();
        assert_ne!(pos, (100, 100));
    }

    #[test]
    fn place_without_overlap_invalid_axis_raises() {
        let err = place_without_overlap("[]", 20, 20, 0, 0, "diagonal", 5, 10).unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(msg.contains("axis"), "got: {msg}");
        });
    }

    #[test]
    fn layered_layout_orders_a_chain_left_to_right() {
        let blocks =
            r#"[{"id":"a","w":40,"h":20},{"id":"b","w":40,"h":20},{"id":"c","w":40,"h":20}]"#;
        let edges = r#"[["a","b"],["b","c"]]"#;
        let result = layered_layout(blocks, edges, 20, 10, 10).unwrap();
        let positions: HashMap<String, (i64, i64)> = serde_json::from_str(&result).unwrap();
        assert!(positions["a"].0 < positions["b"].0);
        assert!(positions["b"].0 < positions["c"].0);
    }

    #[test]
    fn load_document_rejects_non_v2_schema_version() {
        let text = r#"{"schema_version":1,"model":{"id":"m","name":"M","blocks":[],"wires":[]}}"#;
        let err = load_document(text).unwrap_err();
        Python::attach(|py| {
            let msg = err.value(py).to_string();
            assert!(
                msg.contains("schema_version") || msg.contains("2"),
                "got: {msg}"
            );
        });
    }

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }

    // -- subckt ------------------------------------------------------------

    const LEAF_DEF: &str = r#"{"id":"leaf","name":"LEAF","ports":[
        {"id":"a","kind":"physical-electrical"},
        {"id":"b","kind":"physical-electrical"}]}"#;

    fn err_msg(err: PyErr) -> String {
        Python::attach(|py| err.value(py).to_string())
    }

    #[test]
    fn add_subckt_def_then_instantiate_twice_and_wire_both() {
        let model = new_model("m1", "M", 10).unwrap();
        let added: Value =
            serde_json::from_str(&add_subckt_def(&model, LEAF_DEF).unwrap()).unwrap();
        assert_eq!(added["def_id"], "leaf");
        let model = serde_json::to_string(&added["model"]).unwrap();

        let mut model = model;
        for id in ["x1", "x2"] {
            let spec = format!(r#"{{"def_ref":"leaf","position":[0,0],"id":"{id}"}}"#);
            let r: Value =
                serde_json::from_str(&instantiate_subckt(&model, &spec).unwrap()).unwrap();
            assert_eq!(r["block_id"], id);
            model = serde_json::to_string(&r["model"]).unwrap();
        }

        // Ordinary connect() wires the def's own port ids -- no catalogue lookup involved.
        let r: Value = serde_json::from_str(
            &connect(
                &model,
                r#"{"block":"x1","port":"b"}"#,
                r#"{"block":"x2","port":"a"}"#,
                "physical-electrical",
                "{}",
            )
            .unwrap(),
        )
        .unwrap();
        let model = serde_json::to_string(&r["model"]).unwrap();

        let doc = save_document(&model).unwrap();
        let reloaded: Value = serde_json::from_str(&load_document(&doc).unwrap()).unwrap();
        assert_eq!(reloaded["schema_version"], 2);
        assert_eq!(
            reloaded["model"]["subckt_defs"].as_array().unwrap().len(),
            1
        );
        assert_eq!(reloaded["model"]["blocks"].as_array().unwrap().len(), 2);
        assert_eq!(reloaded["model"]["blocks"][0]["subckt_ref"], "leaf");
        assert_eq!(reloaded["model"]["blocks"][0]["paradigm"], "spice-subckt");
        assert_eq!(
            reloaded["model"]["blocks"][0]["type"],
            "spice-subckt.instance"
        );
    }

    #[test]
    fn add_subckt_def_duplicate_id_raises_with_duplicate_in_message() {
        let model = new_model("m1", "M", 10).unwrap();
        let added: Value =
            serde_json::from_str(&add_subckt_def(&model, LEAF_DEF).unwrap()).unwrap();
        let model = serde_json::to_string(&added["model"]).unwrap();
        let msg = err_msg(add_subckt_def(&model, LEAF_DEF).unwrap_err());
        assert!(msg.starts_with("duplicate:"), "got: {msg}");
    }

    #[test]
    fn add_subckt_def_cycle_raises_naming_the_actual_cycle() {
        let model = new_model("m1", "M", 10).unwrap();
        let a = r#"{"id":"a","name":"A","instances":[{"id":"X1","def_ref":"b"}]}"#;
        let added: Value = serde_json::from_str(&add_subckt_def(&model, a).unwrap()).unwrap();
        let model = serde_json::to_string(&added["model"]).unwrap();
        let b = r#"{"id":"b","name":"B","instances":[{"id":"X1","def_ref":"a"}]}"#;
        let msg = err_msg(add_subckt_def(&model, b).unwrap_err());
        assert!(msg.starts_with("cyclic-subckt-def:"), "got: {msg}");
        assert!(
            msg.contains("a -> b -> a") || msg.contains("b -> a -> b"),
            "the message must name the actual cycle, got: {msg}"
        );
    }

    #[test]
    fn add_subckt_def_self_reference_raises() {
        let model = new_model("m1", "M", 10).unwrap();
        let d = r#"{"id":"loopy","name":"L","instances":[{"id":"X1","def_ref":"loopy"}]}"#;
        let msg = err_msg(add_subckt_def(&model, d).unwrap_err());
        assert!(msg.contains("loopy -> loopy"), "got: {msg}");
    }

    #[test]
    fn add_subckt_def_bad_port_kind_raises() {
        let model = new_model("m1", "M", 10).unwrap();
        let d = r#"{"id":"d1","name":"D","ports":[{"id":"bus","kind":"physical-signal"}]}"#;
        let msg = err_msg(add_subckt_def(&model, d).unwrap_err());
        assert!(msg.starts_with("invalid-subckt:"), "got: {msg}");
        assert!(msg.contains("physical-signal"), "got: {msg}");
    }

    #[test]
    fn instantiate_unknown_def_ref_raises() {
        let model = new_model("m1", "M", 10).unwrap();
        let msg = err_msg(
            instantiate_subckt(&model, r#"{"def_ref":"ghost","position":[0,0]}"#).unwrap_err(),
        );
        assert!(msg.starts_with("unknown-subckt-def:"), "got: {msg}");
        assert!(msg.contains("ghost"), "got: {msg}");
    }

    #[test]
    fn instantiate_subckt_bad_json_raises_value_error_not_panic() {
        let model = new_model("m1", "M", 10).unwrap();
        assert!(instantiate_subckt(&model, "{not json").is_err());
        assert!(add_subckt_def(&model, "{not json").is_err());
    }

    #[test]
    fn compile_emits_a_netlist_and_reports_notes() {
        let model = new_model("m1", "M", 10).unwrap();
        let r = |m: &str, s: &str| -> String {
            let v: Value = serde_json::from_str(&add_block(m, s).unwrap()).unwrap();
            serde_json::to_string(&v["model"]).unwrap()
        };
        let model = r(
            &model,
            r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"R1","params":{"R":1000}}"#,
        );
        let model = r(
            &model,
            r#"{"type":"routing.ground","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"gnd"}"#,
        );
        let mut model = model;
        for port in ["p1", "p2"] {
            let v: Value = serde_json::from_str(
                &connect(
                    &model,
                    &format!(r#"{{"block":"R1","port":"{port}"}}"#),
                    r#"{"block":"gnd","port":"p"}"#,
                    "physical-electrical",
                    "{}",
                )
                .unwrap(),
            )
            .unwrap();
            model = serde_json::to_string(&v["model"]).unwrap();
        }
        let out: Value =
            serde_json::from_str(&compile(&model, "general-simulator").unwrap()).unwrap();
        assert!(
            out["netlist"].as_str().unwrap().contains("R1 0 0 1000"),
            "{out}"
        );
        assert!(out["notes"].is_array());
    }

    #[test]
    fn compile_unknown_target_raises_keyword_prefixed() {
        let model = new_model("m1", "M", 10).unwrap();
        let msg = err_msg(compile(&model, "ngspice").unwrap_err());
        assert!(msg.starts_with("unknown-target:"), "got: {msg}");
    }

    #[test]
    fn compile_unknown_type_raises_keyword_prefixed() {
        let model = new_model("m1", "M", 10).unwrap();
        let v: Value = serde_json::from_str(
            &add_block(
                &model,
                r#"{"type":"made.up.type","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"z1"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let model = serde_json::to_string(&v["model"]).unwrap();
        let msg = err_msg(compile(&model, "general-simulator").unwrap_err());
        assert!(msg.starts_with("unknown-type:"), "got: {msg}");
        assert!(msg.contains("made.up.type"), "got: {msg}");
    }
}
