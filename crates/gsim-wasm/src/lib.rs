//! wasm-bindgen bindings for `gsim-core`, exposed to `general-simulator-gui`'s frontend.
//!
//! **Interface design**: stateless, JSON-in/JSON-out free functions -- deliberately not a
//! stateful `#[wasm_bindgen]` struct wrapping a `ModelBuilder` across the FFI boundary. A
//! long-lived wasm object needs JS-side lifecycle management (explicit `.free()` calls, easy to
//! leak or double-free) that isn't worth the complexity for a first pass; a JSON round-trip is
//! simpler, more robust, and matches this crate's own internal philosophy of JSON as the
//! interchange format everywhere. Every function here takes/returns a JS `string` (JSON text)
//! and throws a real JS `Error` (never an opaque panic/trap) on failure. `crates/gsim-py` builds
//! the same JSON contracts as a PyO3 equivalent, so a frontend and a Python caller can perform
//! the same operations against the same shapes.
//!
//! This crate is a thin wrapper only: it does not reimplement any schema/validation/layout
//! logic. See `../gsim-core/src/lib.rs` for the actual implementation.
//!
//! **Why every `#[wasm_bindgen]`-exported function's body is one line**: `wasm-bindgen`'s
//! `JsValue`/`js_sys` bindings are backed by real JS imports that exist only on the `wasm32`
//! target -- constructing *any* `JsValue` (even `JsValue::from_str`) on the host test target
//! panics ("function not implemented on non-wasm32 targets"). Since `cargo test` on the host
//! target is this crate's primary verification path (per the crate's process notes), all real
//! logic lives in a private `*_impl` function returning `Result<String, String>` (or `String`
//! for the one infallible case), which touches no wasm-bindgen type at all and is fully testable
//! on the host; each public `#[wasm_bindgen]` function is a one-line wrapper converting that
//! `Result`'s `Err` into a thrown `JsValue` via [`js_err`] -- the only place in this file that
//! constructs a `JsValue`, and therefore the only place host tests must avoid calling directly.

use gsim_core::compile;
use gsim_core::contract::{self, ComponentContract, ContractRegistry};
use gsim_core::document::{
    deserialize_comment, ConnectOptions, Endpoint, GsimSchDocument, Mirror, Model, ModelBuilder,
    NewBlockSpec, NewSubcktInstanceSpec, Paradigm, SubcktDef,
};
use gsim_core::layout::{self, Axis, LayoutInput, Rect};
use serde::Deserialize;
use serde_json::{Map, Value};
use wasm_bindgen::prelude::*;

/// Converts an error message into a thrown JS value carrying that message, so a caller never
/// sees an opaque panic/trap. On the real `wasm32` target this constructs a genuine JS `Error`
/// object (idiomatic: gives callers `.message`/stack, not just a bare string). This function
/// itself is never exercised by the host test suite (see the module doc) -- only the `*_impl`
/// functions it wraps are.
fn js_err(msg: impl std::fmt::Display) -> JsValue {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Error::new(&msg.to_string()).into()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Host target: js_sys::Error's imports don't exist here. Unreachable in practice since
        // nothing calls js_err from #[cfg(test)] code, but kept total rather than panicking.
        let _ = msg;
        JsValue::UNDEFINED
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(json: &str, what: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| format!("invalid {what} JSON: {e}"))
}

fn respond_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("failed to serialize result: {e}"))
}

#[wasm_bindgen]
pub fn version() -> String {
    gsim_core::version().to_string()
}

fn evaluate_component_impl(contract_json: &str, params_json: &str) -> Result<String, String> {
    let contract: ComponentContract = parse_json(contract_json, "contract")?;
    let params: Map<String, Value> = parse_json(params_json, "params")?;
    let evaluated = contract::evaluate_component(&contract, &params).map_err(|e| e.to_string())?;
    respond_json(&evaluated)
}

/// Parses `contract_json` as a `ComponentContract` and `params_json` as a JSON object of
/// parameter values, evaluates it, and returns the resulting `EvaluatedSymbol` as JSON.
#[wasm_bindgen(js_name = evaluateComponent)]
pub fn evaluate_component(contract_json: &str, params_json: &str) -> Result<String, JsValue> {
    evaluate_component_impl(contract_json, params_json).map_err(js_err)
}

fn validate_parameters_impl(contract_json: &str, params_json: &str) -> Result<String, String> {
    let contract: ComponentContract = parse_json(contract_json, "contract")?;
    let params: Map<String, Value> = parse_json(params_json, "params")?;
    let failures = contract::validate_parameters(&contract, &params);
    let json = failures
        .into_iter()
        .map(|f| serde_json::json!({"parameter": f.parameter, "message": f.message}))
        .collect::<Vec<_>>();
    respond_json(&json)
}

/// Returns a JSON array of `ValidationFailure` (`{"parameter":..., "message":...}`); an empty
/// array means every parameter with a `validate` rule passed. Only errors on malformed input
/// JSON -- validation failures themselves are reported in the returned array, never as a thrown
/// error.
#[wasm_bindgen(js_name = validateParameters)]
pub fn validate_parameters(contract_json: &str, params_json: &str) -> Result<String, JsValue> {
    validate_parameters_impl(contract_json, params_json).map_err(js_err)
}

/// Returns a fresh, empty `Model` as JSON. Cannot fail.
#[wasm_bindgen(js_name = newModel)]
pub fn new_model(id: &str, name: &str, grid: i64) -> String {
    let model = Model::new(id, name, grid);
    // A freshly constructed Model always serializes cleanly -- no user input to fail on.
    serde_json::to_string(&model).expect("Model always serializes")
}

#[derive(Deserialize)]
struct NewBlockSpecJson {
    r#type: String,
    paradigm: Paradigm,
    domain: String,
    #[serde(default)]
    fidelity: Option<String>,
    position: (i64, i64),
    #[serde(default)]
    rotation: i64,
    #[serde(default)]
    mirror: Mirror,
    #[serde(default)]
    params: Map<String, Value>,
    #[serde(default)]
    ic: Map<String, Value>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default, deserialize_with = "deserialize_comment")]
    comment_before: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_comment")]
    comment_after: Vec<String>,
    #[serde(default)]
    id: Option<String>,
}

impl From<NewBlockSpecJson> for NewBlockSpec {
    fn from(spec: NewBlockSpecJson) -> Self {
        NewBlockSpec {
            r#type: spec.r#type,
            paradigm: spec.paradigm,
            domain: spec.domain,
            fidelity: spec.fidelity,
            position: spec.position,
            rotation: spec.rotation,
            mirror: spec.mirror,
            params: spec.params,
            ic: spec.ic,
            label: spec.label,
            comment_before: spec.comment_before,
            comment_after: spec.comment_after,
            id: spec.id,
        }
    }
}

fn add_block_impl(model_json: &str, spec_json: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let spec: NewBlockSpecJson = parse_json(spec_json, "block spec")?;
    let mut builder = ModelBuilder::from_model(model);
    let block_id = builder.add_block(spec.into()).map_err(|e| e.to_string())?;
    let result = serde_json::json!({"model": builder.into_model(), "block_id": block_id});
    respond_json(&result)
}

/// Adds a block to `model_json` from `spec_json` (matching `NewBlockSpec`'s fields). Returns
/// `{"model": <updated Model JSON>, "block_id": "<assigned id>"}`.
#[wasm_bindgen(js_name = addBlock)]
pub fn add_block(model_json: &str, spec_json: &str) -> Result<String, JsValue> {
    add_block_impl(model_json, spec_json).map_err(js_err)
}

fn compile_impl(model_json: &str, target: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let out = compile::compile(&model, target).map_err(|e| e.to_string())?;
    respond_json(&serde_json::json!({"netlist": out.netlist, "notes": out.notes}))
}

/// Compiles a `Model` into netlist text for `target` (currently only `"general-simulator"`).
///
/// **Shared JSON contract** (identical to `gsim-py`'s `compile`): returns
/// `{"netlist": "<text>", "notes": [...]}`; throws a real JS `Error` carrying the compiler's
/// own message on failure.
#[wasm_bindgen(js_name = compile)]
pub fn compile(model_json: &str, target: &str) -> Result<String, JsValue> {
    compile_impl(model_json, target).map_err(js_err)
}

fn add_subckt_def_impl(model_json: &str, def_json: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let def: SubcktDef = parse_json(def_json, "subckt def")?;
    let mut builder = ModelBuilder::from_model(model);
    let def_id = builder.add_subckt_def(def).map_err(|e| e.to_string())?;
    respond_json(&serde_json::json!({"model": builder.into_model(), "def_id": def_id}))
}

/// Adds a reusable SPICE `.SUBCKT` definition to `model_json`.
///
/// **Shared JSON contract** (identical to `gsim-py`'s `add_subckt_def`): `def_json` is a
/// `SubcktDef` — `{"id", "name", "ports": [{"id", "kind"}], "params"?, "blocks"?, "wires"?,
/// "instances"?: [{"id", "def_ref", "connections"?, "params"?}]}`. Returns
/// `{"model": <updated Model JSON>, "def_id": "<id>"}`, mirroring `addBlock`'s
/// `{"model", "block_id"}` shape. Throws with the real error message on an invalid id/port
/// kind, a duplicate def id, or a cyclic `def_ref` graph (the thrown message names the actual
/// cycle, e.g. `"Cyclic SubcktDef reference: a -> b -> a"`).
#[wasm_bindgen(js_name = addSubcktDef)]
pub fn add_subckt_def(model_json: &str, def_json: &str) -> Result<String, JsValue> {
    add_subckt_def_impl(model_json, def_json).map_err(js_err)
}

fn instantiate_subckt_impl(model_json: &str, spec_json: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let spec: NewSubcktInstanceSpec = parse_json(spec_json, "subckt instance spec")?;
    let mut builder = ModelBuilder::from_model(model);
    let block_id = builder
        .instantiate_subckt(spec)
        .map_err(|e| e.to_string())?;
    respond_json(&serde_json::json!({"model": builder.into_model(), "block_id": block_id}))
}

/// Instantiates an existing `SubcktDef` as a top-level `spice-subckt` block.
///
/// **Shared JSON contract** (identical to `gsim-py`'s `instantiate_subckt`): `spec_json` is
/// `{"def_ref", "position": [x, y], "rotation"?, "domain"?, "fidelity"?, "mirror"?, "params"?,
/// "label"?, "comment_before"?, "comment_after"?, "id"?}`. Returns `{"model": <updated Model JSON>, "block_id": "<assigned id>"}`,
/// the same shape `addBlock` returns. The created block's wireable ports are exactly the
/// referenced def's `ports[].id` list — wire them with ordinary `connect` calls.
#[wasm_bindgen(js_name = instantiateSubckt)]
pub fn instantiate_subckt(model_json: &str, spec_json: &str) -> Result<String, JsValue> {
    instantiate_subckt_impl(model_json, spec_json).map_err(js_err)
}

fn remove_block_impl(model_json: &str, block_id: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let mut builder = ModelBuilder::from_model(model);
    builder.remove_block(block_id);
    respond_json(&builder.into_model())
}

/// Removes a block (and cascades wire deletion) from `model_json`. Returns updated `Model` JSON.
#[wasm_bindgen(js_name = removeBlock)]
pub fn remove_block(model_json: &str, block_id: &str) -> Result<String, JsValue> {
    remove_block_impl(model_json, block_id).map_err(js_err)
}

fn set_param_impl(
    model_json: &str,
    block_id: &str,
    name: &str,
    value_json: &str,
) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let value: Value = parse_json(value_json, "value")?;
    let mut builder = ModelBuilder::from_model(model);
    builder
        .set_param(block_id, name, value)
        .map_err(|e| e.to_string())?;
    respond_json(&builder.into_model())
}

/// Sets a block parameter. Errors if `block_id` doesn't exist. Returns updated `Model` JSON.
#[wasm_bindgen(js_name = setParam)]
pub fn set_param(
    model_json: &str,
    block_id: &str,
    name: &str,
    value_json: &str,
) -> Result<String, JsValue> {
    set_param_impl(model_json, block_id, name, value_json).map_err(js_err)
}

fn add_model_param_impl(model_json: &str, name: &str, value_json: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let value: Value = parse_json(value_json, "value")?;
    let mut builder = ModelBuilder::from_model(model);
    builder
        .add_model_param(name, value)
        .map_err(|e| e.to_string())?;
    respond_json(&builder.into_model())
}

/// Adds a model-level parameter. Returns updated `Model` JSON.
#[wasm_bindgen(js_name = addModelParam)]
pub fn add_model_param(model_json: &str, name: &str, value_json: &str) -> Result<String, JsValue> {
    add_model_param_impl(model_json, name, value_json).map_err(js_err)
}

#[derive(Deserialize)]
struct EndpointJson {
    block: String,
    port: String,
}

impl From<EndpointJson> for Endpoint {
    fn from(e: EndpointJson) -> Self {
        Endpoint::new(e.block, e.port)
    }
}

#[derive(Deserialize, Default)]
struct ConnectOptionsJson {
    #[serde(default)]
    waypoints: Vec<(i64, i64)>,
    #[serde(default)]
    net: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    label: Option<String>,
}

impl From<ConnectOptionsJson> for ConnectOptions {
    fn from(o: ConnectOptionsJson) -> Self {
        ConnectOptions {
            waypoints: o.waypoints,
            net: o.net,
            id: o.id,
            pinned: o.pinned,
            label: o.label,
        }
    }
}

fn connect_impl(
    model_json: &str,
    from_json: &str,
    to_json: &str,
    kind: &str,
    opts_json: &str,
    contracts_json: Option<String>,
) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let from: EndpointJson = parse_json(from_json, "from endpoint")?;
    let to: EndpointJson = parse_json(to_json, "to endpoint")?;
    let opts: ConnectOptionsJson = parse_json(opts_json, "connect options")?;
    let contracts: ContractRegistry = match &contracts_json {
        Some(text) => parse_json(text, "contract registry")?,
        None => ContractRegistry::new(),
    };
    let mut builder = ModelBuilder::from_model(model).with_contracts(contracts);
    let wire_id = builder
        .connect(from.into(), to.into(), kind, opts.into())
        .map_err(|e| e.to_string())?
        .to_string();
    let result = serde_json::json!({"model": builder.into_model(), "wire_id": wire_id});
    respond_json(&result)
}

/// Connects two block ports. `from_json`/`to_json` are `{"block":"...","port":"..."}`,
/// `opts_json` matches `ConnectOptions` (all fields optional -- `"{}"` is a valid default).
/// Returns `{"model": <updated Model JSON>, "wire_id": "<assigned id>"}`.
///
/// The thrown `Error`'s message clearly states which failure case occurred (cross-paradigm /
/// dangling endpoint / duplicate id / port incompatibility) -- see
/// `gsim_core::document::BuilderError`'s `Display` impl -- so the frontend can distinguish a
/// cross-paradigm attempt (which needs immediate red-X feedback per
/// `docs/specs/gsim-sch-format.md` §8.3) from every other failure.
///
/// A §8.1/§8.2 port-kind rejection additionally embeds the **same reason token
/// `apps/frontend/src/canvas/port-compat.ts`'s own `canConnect` returns** -- `reserved-kind`,
/// `kind-mismatch`, `fan-in-on-signal` or `dim-mismatch` -- so the canvas's existing
/// reject-flash branching needs no new vocabulary to handle a rejection that came from this
/// crate instead.
///
/// `contractsJson` is optional (pass `undefined`/`null` to omit) and is a
/// `{"<block type>": <ComponentContract>}` object -- the same contract shape
/// `evaluateComponent` takes. The frontend already holds exactly this per block type in its own
/// catalogue, and passing it lets `connect` resolve each endpoint's real `kind` and `dim` from
/// the block's own `params`, which is what makes `kind-mismatch` exact for ordinary components
/// and `dim-mismatch` possible at all -- i.e. what makes this the same check `canConnect`
/// performs, rather than a coarser one. Omitting it changes nothing that used to work: an
/// unregistered type keeps the block-`domain`-based approximation. See
/// `gsim_core::document::port_compat` for exactly what each source of truth can decide.
#[wasm_bindgen]
pub fn connect(
    model_json: &str,
    from_json: &str,
    to_json: &str,
    kind: &str,
    opts_json: &str,
    contracts_json: Option<String>,
) -> Result<String, JsValue> {
    connect_impl(
        model_json,
        from_json,
        to_json,
        kind,
        opts_json,
        contracts_json,
    )
    .map_err(js_err)
}

fn auto_arrange_impl(
    model_json: &str,
    bbox_map_json: &str,
    column_gap: i64,
    row_gap: i64,
) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let bbox_map: Map<String, Value> = parse_json(bbox_map_json, "bbox map")?;
    let mut builder = ModelBuilder::from_model(model);
    builder.auto_arrange(
        |block| {
            bbox_map
                .get(&block.id)
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    let w = arr.first()?.as_i64()?;
                    let h = arr.get(1)?.as_i64()?;
                    Some((w, h))
                })
                .unwrap_or((0, 0))
        },
        column_gap,
        row_gap,
    );
    respond_json(&builder.into_model())
}

/// Arranges every block via `gsim_core::layout::layered_layout`. `bbox_map_json` is
/// `{"<block_id>": [w, h], ...}` -- the caller has already evaluated each block's GSF v2
/// contract to get its bbox; this function doesn't know about contracts. Missing block ids
/// default to `(0, 0)`, never error. Returns updated `Model` JSON.
#[wasm_bindgen(js_name = autoArrange)]
pub fn auto_arrange(
    model_json: &str,
    bbox_map_json: &str,
    column_gap: i64,
    row_gap: i64,
) -> Result<String, JsValue> {
    auto_arrange_impl(model_json, bbox_map_json, column_gap, row_gap).map_err(js_err)
}

fn load_document_impl(json: &str) -> Result<String, String> {
    let doc = GsimSchDocument::from_json(json).map_err(|e| e.to_string())?;
    respond_json(&doc)
}

/// Parses `json` via `GsimSchDocument::from_json` (rejects a non-v2 `schema_version` with a
/// clear message) and returns canonical re-serialized JSON.
#[wasm_bindgen(js_name = loadDocument)]
pub fn load_document(json: &str) -> Result<String, JsValue> {
    load_document_impl(json).map_err(js_err)
}

fn save_document_impl(model_json: &str) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let doc = GsimSchDocument::new(model);
    doc.to_json_pretty().map_err(|e| e.to_string())
}

/// Wraps a `Model` JSON into `{"schema_version": 2, "model": ...}`, pretty-printed.
#[wasm_bindgen(js_name = saveDocument)]
pub fn save_document(model_json: &str) -> Result<String, JsValue> {
    save_document_impl(model_json).map_err(js_err)
}

#[derive(Deserialize)]
struct RectJson {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

impl From<RectJson> for Rect {
    fn from(r: RectJson) -> Self {
        Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn place_without_overlap_impl(
    existing_json: &str,
    w: i64,
    h: i64,
    desired_x: i64,
    desired_y: i64,
    axis: &str,
    margin: i64,
    grid: i64,
) -> Result<String, String> {
    let existing: Vec<RectJson> = parse_json(existing_json, "existing rects")?;
    let existing: Vec<Rect> = existing.into_iter().map(Into::into).collect();
    let axis = match axis {
        "horizontal" => Axis::Horizontal,
        "vertical" => Axis::Vertical,
        other => {
            return Err(format!(
                "invalid axis '{other}' (expected \"horizontal\" or \"vertical\")"
            ))
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
    respond_json(&[x, y])
}

/// `existing_json` is a JSON array of `{"x":..,"y":..,"w":..,"h":..}`; `axis` is `"horizontal"`
/// or `"vertical"`. Returns `[x, y]` as JSON.
#[wasm_bindgen(js_name = placeWithoutOverlap)]
#[allow(clippy::too_many_arguments)]
pub fn place_without_overlap(
    existing_json: &str,
    w: i64,
    h: i64,
    desired_x: i64,
    desired_y: i64,
    axis: &str,
    margin: i64,
    grid: i64,
) -> Result<String, JsValue> {
    place_without_overlap_impl(
        existing_json,
        w,
        h,
        desired_x,
        desired_y,
        axis,
        margin,
        grid,
    )
    .map_err(js_err)
}

#[derive(Deserialize)]
struct LayoutBlockJson {
    id: String,
    w: i64,
    h: i64,
}

fn layered_layout_impl(
    blocks_json: &str,
    edges_json: &str,
    column_gap: i64,
    row_gap: i64,
    grid: i64,
) -> Result<String, String> {
    let blocks: Vec<LayoutBlockJson> = parse_json(blocks_json, "blocks")?;
    let edges: Vec<(String, String)> = parse_json(edges_json, "edges")?;

    let inputs: Vec<LayoutInput> = blocks
        .iter()
        .map(|b| LayoutInput {
            id: &b.id,
            size: (b.w, b.h),
        })
        .collect();
    let edge_refs: Vec<(&str, &str)> = edges
        .iter()
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect();

    let positions = layout::layered_layout(&inputs, &edge_refs, column_gap, row_gap, grid);
    let map: std::collections::BTreeMap<String, (i64, i64)> = positions.into_iter().collect();
    respond_json(&map)
}

/// `blocks_json` is `[{"id":"...","w":..,"h":..}, ...]`, `edges_json` is
/// `[["from_id","to_id"], ...]`. Returns `{"<block_id>": [x, y], ...}`.
#[wasm_bindgen(js_name = layeredLayout)]
pub fn layered_layout(
    blocks_json: &str,
    edges_json: &str,
    column_gap: i64,
    row_gap: i64,
    grid: i64,
) -> Result<String, JsValue> {
    layered_layout_impl(blocks_json, edges_json, column_gap, row_gap, grid).map_err(js_err)
}

#[cfg(test)]
mod tests {
    //! All assertions call the `*_impl` functions directly (`Result<String, String>`), never the
    //! `#[wasm_bindgen]`-wrapped public functions' `Err` path -- constructing any `JsValue` on
    //! this host test target panics (see the module doc), so `js_err`/the public wrappers'
    //! error-conversion step is exercised only by the `wasm32` build, not `cargo test`. The
    //! `Ok` path of the public wrappers never touches `JsValue` at all, so those are exercised
    //! directly (`version`, `new_model`) where convenient.

    use super::*;
    use serde_json::json;

    fn pid_contract_json() -> String {
        json!({
            "name": "control.pid",
            "label": "PID",
            "parameters": [
                {"name": "kp", "type": "number", "default": 1.0}
            ],
            "ports": [
                {"id": "in", "side": "left", "kind": "signal", "direction": "in"},
                {"id": "out", "side": "right", "kind": "signal", "direction": "out"}
            ],
            "geometry": {
                "bbox": [60, 40],
                "primitives": [{"type": "rect", "x": 0, "y": 0, "w": 60, "h": 40}]
            }
        })
        .to_string()
    }

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }

    // --- evaluate_component ---

    #[test]
    fn evaluate_component_returns_evaluated_symbol_json() {
        let result = evaluate_component_impl(&pid_contract_json(), "{}").unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["bbox"], json!([60.0, 40.0]));
        assert_eq!(v["ports"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn evaluate_component_errors_on_bad_contract_json() {
        let err = evaluate_component_impl("not json", "{}").unwrap_err();
        assert!(err.contains("invalid contract JSON"));
    }

    // --- validate_parameters ---

    #[test]
    fn validate_parameters_empty_array_when_valid() {
        let contract = json!({
            "name": "x", "label": "X",
            "parameters": [{"name": "n", "type": "number", "default": 5, "validate": "${{ value > 0 }}"}],
            "ports": [], "geometry": {"bbox": [10, 10], "primitives": []}
        }).to_string();
        let result = validate_parameters_impl(&contract, "{}").unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn validate_parameters_reports_failures_not_errors() {
        let contract = json!({
            "name": "x", "label": "X",
            "parameters": [{"name": "n", "type": "number", "default": -1, "validate": "${{ value > 0 }}", "error": "must be positive"}],
            "ports": [], "geometry": {"bbox": [10, 10], "primitives": []}
        }).to_string();
        let result = validate_parameters_impl(&contract, "{}").unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["parameter"], "n");
        assert_eq!(v[0]["message"], "must be positive");
    }

    // --- new_model / add_block / connect / auto_arrange / save/load round trip ---

    #[test]
    fn full_flow_new_model_add_blocks_connect_auto_arrange_save_load() {
        let model_json = new_model("m1", "My Model", 10);

        let spec1 = json!({
            "type": "signal.step", "paradigm": "signal-flow", "domain": "control",
            "position": [0, 0], "rotation": 0, "id": "src1"
        })
        .to_string();
        let r1 = add_block_impl(&model_json, &spec1).unwrap();
        let r1v: Value = serde_json::from_str(&r1).unwrap();
        assert_eq!(r1v["block_id"], "src1");
        let model_json = r1v["model"].to_string();

        let spec2 = json!({
            "type": "control.gain", "paradigm": "signal-flow", "domain": "control",
            "position": [100, 0], "rotation": 0, "id": "gain1"
        })
        .to_string();
        let r2 = add_block_impl(&model_json, &spec2).unwrap();
        let r2v: Value = serde_json::from_str(&r2).unwrap();
        let model_json = r2v["model"].to_string();

        let from = json!({"block": "src1", "port": "out"}).to_string();
        let to = json!({"block": "gain1", "port": "in"}).to_string();
        let r3 = connect_impl(&model_json, &from, &to, "signal", "{}", None).unwrap();
        let r3v: Value = serde_json::from_str(&r3).unwrap();
        assert_eq!(r3v["wire_id"], "wire1");
        let model_json = r3v["model"].to_string();

        let bbox_map = json!({"src1": [40, 20], "gain1": [40, 20]}).to_string();
        let model_json = auto_arrange_impl(&model_json, &bbox_map, 20, 10).unwrap();

        let saved = save_document_impl(&model_json).unwrap();
        let loaded = load_document_impl(&saved).unwrap();
        let loaded_v: Value = serde_json::from_str(&loaded).unwrap();
        assert_eq!(loaded_v["schema_version"], 2);
        assert_eq!(loaded_v["model"]["id"], "m1");
        assert_eq!(loaded_v["model"]["blocks"].as_array().unwrap().len(), 2);
        assert_eq!(loaded_v["model"]["wires"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn add_block_rejects_duplicate_id() {
        let model_json = new_model("m1", "M", 10);
        let spec = json!({
            "type": "passive.resistor", "paradigm": "physical-network", "domain": "electrical",
            "position": [0, 0], "rotation": 0, "id": "r1"
        })
        .to_string();
        let r1 = add_block_impl(&model_json, &spec).unwrap();
        let model_json = serde_json::from_str::<Value>(&r1).unwrap()["model"].to_string();
        let err = add_block_impl(&model_json, &spec).unwrap_err();
        assert!(err.contains("duplicate id"));
    }

    #[test]
    fn add_block_rejects_invalid_rotation() {
        let model_json = new_model("m1", "M", 10);
        let spec = json!({
            "type": "passive.resistor", "paradigm": "physical-network", "domain": "electrical",
            "position": [0, 0], "rotation": 45
        })
        .to_string();
        let err = add_block_impl(&model_json, &spec).unwrap_err();
        assert!(err.contains("rotation"));
    }

    #[test]
    fn remove_block_cascades_wire_deletion() {
        let model_json = new_model("m1", "M", 10);
        let spec1 = json!({
            "type": "passive.resistor", "paradigm": "physical-network", "domain": "electrical",
            "position": [0, 0], "rotation": 0, "id": "r1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec1).unwrap()).unwrap()
                ["model"]
                .to_string();
        let spec2 = json!({
            "type": "passive.capacitor", "paradigm": "physical-network", "domain": "electrical",
            "position": [100, 0], "rotation": 0, "id": "c1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec2).unwrap()).unwrap()
                ["model"]
                .to_string();
        let from = json!({"block": "r1", "port": "p"}).to_string();
        let to = json!({"block": "c1", "port": "p"}).to_string();
        let model_json = serde_json::from_str::<Value>(
            &connect_impl(&model_json, &from, &to, "physical-electrical", "{}", None).unwrap(),
        )
        .unwrap()["model"]
            .to_string();

        let removed = remove_block_impl(&model_json, "r1").unwrap();
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert_eq!(v["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(v["wires"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn set_param_errors_on_unknown_block() {
        let model_json = new_model("m1", "M", 10);
        let err = set_param_impl(&model_json, "ghost", "R", "1000").unwrap_err();
        assert!(err.contains("no block with id"));
    }

    #[test]
    fn add_model_param_round_trips() {
        let model_json = new_model("m1", "M", 10);
        let updated = add_model_param_impl(&model_json, "vcc", "5.0").unwrap();
        let v: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(v["params"][0]["name"], "vcc");
        assert_eq!(v["params"][0]["value"], 5.0);
    }

    #[test]
    fn connect_rejects_dangling_endpoint() {
        let model_json = new_model("m1", "M", 10);
        let spec = json!({
            "type": "passive.resistor", "paradigm": "physical-network", "domain": "electrical",
            "position": [0, 0], "rotation": 0, "id": "r1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec).unwrap()).unwrap()
                ["model"]
                .to_string();
        let from = json!({"block": "r1", "port": "p"}).to_string();
        let to = json!({"block": "ghost", "port": "p"}).to_string();
        let err =
            connect_impl(&model_json, &from, &to, "physical-electrical", "{}", None).unwrap_err();
        assert!(err.contains("unknown block 'ghost'"));
    }

    #[test]
    fn connect_rejects_cross_paradigm_with_clear_message() {
        let model_json = new_model("m1", "M", 10);
        let spec1 = json!({
            "type": "passive.resistor", "paradigm": "physical-network", "domain": "electrical",
            "position": [0, 0], "rotation": 0, "id": "r1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec1).unwrap()).unwrap()
                ["model"]
                .to_string();
        let spec2 = json!({
            "type": "control.gain", "paradigm": "signal-flow", "domain": "control",
            "position": [100, 0], "rotation": 0, "id": "gain1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec2).unwrap()).unwrap()
                ["model"]
                .to_string();
        let from = json!({"block": "r1", "port": "p"}).to_string();
        let to = json!({"block": "gain1", "port": "in"}).to_string();
        let err = connect_impl(&model_json, &from, &to, "signal", "{}", None).unwrap_err();
        assert!(
            err.to_lowercase().contains("cannot connect")
                && err.contains("PhysicalNetwork")
                && err.contains("SignalFlow"),
            "expected a clear cross-paradigm message, got: {err}"
        );
    }

    #[test]
    fn connect_consults_a_supplied_contract_registry_for_kind_and_dim() {
        // The payload the frontend already holds per block type: a gain whose port `dim`
        // follows its own `width` parameter.
        let contracts = json!({
            "control.gain": {
                "name": "control.gain",
                "label": "Gain",
                "parameters": [{"name": "width", "type": "number", "default": 1}],
                "ports": [
                    {"id": "in", "side": "left", "kind": "signal", "direction": "in",
                     "dim": ["${{ params.width }}"]},
                    {"id": "out", "side": "right", "kind": "signal", "direction": "out",
                     "dim": ["${{ params.width }}"]}
                ],
                "geometry": {"bbox": [40, 30], "primitives": []}
            }
        })
        .to_string();

        let mut model_json = new_model("m1", "M", 10);
        for (id, width) in [("g1", 3), ("g2", 1)] {
            let spec = json!({
                "type": "control.gain", "paradigm": "signal-flow", "domain": "signal",
                "position": [0, 0], "rotation": 0, "id": id, "params": {"width": width}
            })
            .to_string();
            model_json =
                serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec).unwrap())
                    .unwrap()["model"]
                    .to_string();
        }
        let from = json!({"block": "g1", "port": "out"}).to_string();
        let to = json!({"block": "g2", "port": "in"}).to_string();

        let err = connect_impl(
            &model_json,
            &from,
            &to,
            "signal",
            "{}",
            Some(contracts.clone()),
        )
        .unwrap_err();
        // The message carries `canConnect`'s own reason token, so the canvas's existing
        // reject-flash branching needs no new vocabulary.
        assert!(err.contains("dim-mismatch"), "got: {err}");
        assert!(
            err.contains("the source produces dim [3], but the destination requires dim [1]"),
            "got: {err}"
        );

        // Omitting the registry leaves the old behaviour untouched -- the document alone cannot
        // see a dimension, so the same call succeeds.
        connect_impl(&model_json, &from, &to, "signal", "{}", None).unwrap();
    }

    #[test]
    fn connect_rejects_duplicate_wire_id() {
        let model_json = new_model("m1", "M", 10);
        let spec1 = json!({
            "type": "passive.resistor", "paradigm": "physical-network", "domain": "electrical",
            "position": [0, 0], "rotation": 0, "id": "r1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec1).unwrap()).unwrap()
                ["model"]
                .to_string();
        let spec2 = json!({
            "type": "passive.capacitor", "paradigm": "physical-network", "domain": "electrical",
            "position": [100, 0], "rotation": 0, "id": "c1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec2).unwrap()).unwrap()
                ["model"]
                .to_string();
        let from = json!({"block": "r1", "port": "p"}).to_string();
        let to = json!({"block": "c1", "port": "n"}).to_string();
        let opts = json!({"id": "w1"}).to_string();
        let model_json = serde_json::from_str::<Value>(
            &connect_impl(&model_json, &from, &to, "physical-electrical", &opts, None).unwrap(),
        )
        .unwrap()["model"]
            .to_string();
        let err =
            connect_impl(&model_json, &from, &to, "physical-electrical", &opts, None).unwrap_err();
        assert!(err.contains("duplicate id"));
    }

    // --- load_document / save_document ---

    #[test]
    fn load_document_rejects_non_v2_schema_version() {
        let json = r#"{"schema_version":1,"model":{"id":"m","name":"M","blocks":[],"wires":[]}}"#;
        let err = load_document_impl(json).unwrap_err();
        assert!(err.contains("schema_version"));
    }

    #[test]
    fn save_document_wraps_model_with_schema_version_2() {
        let model_json = new_model("m1", "M", 10);
        let saved = save_document_impl(&model_json).unwrap();
        let v: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(v["schema_version"], 2);
        assert_eq!(v["model"]["id"], "m1");
    }

    #[test]
    fn load_document_errors_on_malformed_json() {
        let err = load_document_impl("{not json").unwrap_err();
        assert!(!err.is_empty());
    }

    // --- place_without_overlap / layered_layout ---

    #[test]
    fn place_without_overlap_avoids_existing_rects() {
        let existing = json!([{"x": 0, "y": 0, "w": 60, "h": 40}]).to_string();
        let result =
            place_without_overlap_impl(&existing, 60, 40, 0, 0, "horizontal", 5, 10).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        let arr = v.as_array().unwrap();
        assert_ne!((arr[0].as_i64().unwrap(), arr[1].as_i64().unwrap()), (0, 0));
    }

    #[test]
    fn place_without_overlap_rejects_invalid_axis() {
        let err = place_without_overlap_impl("[]", 10, 10, 0, 0, "diagonal", 0, 10).unwrap_err();
        assert!(err.contains("invalid axis"));
    }

    #[test]
    fn layered_layout_places_chain_left_to_right() {
        let blocks = json!([
            {"id": "a", "w": 40, "h": 20},
            {"id": "b", "w": 40, "h": 20},
            {"id": "c", "w": 40, "h": 20}
        ])
        .to_string();
        let edges = json!([["a", "b"], ["b", "c"]]).to_string();
        let result = layered_layout_impl(&blocks, &edges, 20, 10, 10).unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        let ax = v["a"][0].as_i64().unwrap();
        let bx = v["b"][0].as_i64().unwrap();
        let cx = v["c"][0].as_i64().unwrap();
        assert!(ax < bx);
        assert!(bx < cx);
    }

    #[test]
    fn layered_layout_errors_on_bad_json() {
        let err = layered_layout_impl("not json", "[]", 20, 10, 10).unwrap_err();
        assert!(err.contains("invalid blocks JSON"));
    }

    #[test]
    fn auto_arrange_defaults_missing_bbox_to_zero() {
        let model_json = new_model("m1", "M", 10);
        let spec = json!({
            "type": "signal.step", "paradigm": "signal-flow", "domain": "control",
            "position": [5, 5], "rotation": 0, "id": "src1"
        })
        .to_string();
        let model_json =
            serde_json::from_str::<Value>(&add_block_impl(&model_json, &spec).unwrap()).unwrap()
                ["model"]
                .to_string();
        // Empty bbox map -- src1 is missing, must default to (0,0) rather than error.
        let result = auto_arrange_impl(&model_json, "{}", 20, 10);
        assert!(result.is_ok());
    }

    // -- subckt ------------------------------------------------------------

    const LEAF_DEF: &str = r#"{"id":"leaf","name":"LEAF","ports":[
        {"id":"a","kind":"physical-electrical"},
        {"id":"b","kind":"physical-electrical"}]}"#;

    #[test]
    fn add_subckt_def_then_instantiate_twice_and_wire_both() {
        let model = new_model("m1", "M", 10);
        let added: Value = serde_json::from_str(&add_subckt_def_impl(&model, LEAF_DEF).unwrap())
            .expect("valid JSON");
        assert_eq!(added["def_id"], "leaf");
        let mut model = added["model"].to_string();

        for id in ["x1", "x2"] {
            let spec = format!(r#"{{"def_ref":"leaf","position":[0,0],"id":"{id}"}}"#);
            let r: Value =
                serde_json::from_str(&instantiate_subckt_impl(&model, &spec).unwrap()).unwrap();
            assert_eq!(r["block_id"], id);
            model = r["model"].to_string();
        }

        let r: Value = serde_json::from_str(
            &connect_impl(
                &model,
                r#"{"block":"x1","port":"b"}"#,
                r#"{"block":"x2","port":"a"}"#,
                "physical-electrical",
                "{}",
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let model = r["model"].to_string();

        let doc = save_document_impl(&model).unwrap();
        let reloaded: Value = serde_json::from_str(&load_document_impl(&doc).unwrap()).unwrap();
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
    fn add_subckt_def_rejects_a_duplicate_id() {
        let model = new_model("m1", "M", 10);
        let added: Value =
            serde_json::from_str(&add_subckt_def_impl(&model, LEAF_DEF).unwrap()).unwrap();
        let model = added["model"].to_string();
        let err = add_subckt_def_impl(&model, LEAF_DEF).unwrap_err();
        assert!(err.contains("duplicate id 'leaf'"), "got: {err}");
    }

    #[test]
    fn add_subckt_def_cycle_error_names_the_actual_cycle() {
        let model = new_model("m1", "M", 10);
        let a = r#"{"id":"a","name":"A","instances":[{"id":"X1","def_ref":"b"}]}"#;
        let added: Value = serde_json::from_str(&add_subckt_def_impl(&model, a).unwrap()).unwrap();
        let model = added["model"].to_string();
        let b = r#"{"id":"b","name":"B","instances":[{"id":"X1","def_ref":"a"}]}"#;
        let err = add_subckt_def_impl(&model, b).unwrap_err();
        assert!(err.starts_with("Cyclic SubcktDef reference:"), "got: {err}");
        assert!(
            err.contains("a -> b -> a") || err.contains("b -> a -> b"),
            "the message must name the actual cycle, got: {err}"
        );
    }

    #[test]
    fn add_subckt_def_self_reference_rejected() {
        let model = new_model("m1", "M", 10);
        let d = r#"{"id":"loopy","name":"L","instances":[{"id":"X1","def_ref":"loopy"}]}"#;
        let err = add_subckt_def_impl(&model, d).unwrap_err();
        assert!(err.contains("loopy -> loopy"), "got: {err}");
    }

    #[test]
    fn add_subckt_def_rejects_a_bad_port_kind() {
        let model = new_model("m1", "M", 10);
        let d = r#"{"id":"d1","name":"D","ports":[{"id":"bus","kind":"physical-signal"}]}"#;
        let err = add_subckt_def_impl(&model, d).unwrap_err();
        assert!(err.contains("physical-signal"), "got: {err}");
    }

    #[test]
    fn instantiate_subckt_rejects_an_unknown_def_ref() {
        let model = new_model("m1", "M", 10);
        let err =
            instantiate_subckt_impl(&model, r#"{"def_ref":"ghost","position":[0,0]}"#).unwrap_err();
        assert!(err.contains("no SubcktDef with id 'ghost'"), "got: {err}");
    }

    #[test]
    fn subckt_functions_reject_bad_json_without_panicking() {
        let model = new_model("m1", "M", 10);
        assert!(add_subckt_def_impl(&model, "{not json").is_err());
        assert!(instantiate_subckt_impl(&model, "{not json").is_err());
    }

    #[test]
    fn compile_emits_a_netlist() {
        let model = new_model("m1", "M", 10);
        let step = |m: &str, s: &str| -> String {
            serde_json::from_str::<Value>(&add_block_impl(m, s).unwrap()).unwrap()["model"]
                .to_string()
        };
        let model = step(
            &model,
            r#"{"type":"passive.resistor","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"R1","params":{"R":1000}}"#,
        );
        let model = step(
            &model,
            r#"{"type":"routing.ground","paradigm":"physical-network","domain":"electrical","position":[0,0],"rotation":0,"id":"gnd"}"#,
        );
        let mut model = model;
        for port in ["p1", "p2"] {
            model = serde_json::from_str::<Value>(
                &connect_impl(
                    &model,
                    &format!(r#"{{"block":"R1","port":"{port}"}}"#),
                    r#"{"block":"gnd","port":"p"}"#,
                    "physical-electrical",
                    "{}",
                    None,
                )
                .unwrap(),
            )
            .unwrap()["model"]
                .to_string();
        }
        let out: Value =
            serde_json::from_str(&compile_impl(&model, "general-simulator").unwrap()).unwrap();
        assert!(
            out["netlist"].as_str().unwrap().contains("R1 0 0 1000"),
            "{out}"
        );
    }

    #[test]
    fn compile_rejects_an_unknown_target() {
        let model = new_model("m1", "M", 10);
        let err = compile_impl(&model, "ngspice").unwrap_err();
        assert!(
            err.contains("unknown netlist target 'ngspice'"),
            "got: {err}"
        );
    }
}
