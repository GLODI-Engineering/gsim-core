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

use gsim_core::contract::{self, ComponentContract};
use gsim_core::document::{
    ConnectOptions, Endpoint, GsimSchDocument, Mirror, Model, ModelBuilder, NewBlockSpec, Paradigm,
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
) -> Result<String, String> {
    let model: Model = parse_json(model_json, "model")?;
    let from: EndpointJson = parse_json(from_json, "from endpoint")?;
    let to: EndpointJson = parse_json(to_json, "to endpoint")?;
    let opts: ConnectOptionsJson = parse_json(opts_json, "connect options")?;
    let mut builder = ModelBuilder::from_model(model);
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
/// dangling endpoint / duplicate id) -- see `gsim_core::document::BuilderError`'s `Display` impl
/// -- so the frontend can distinguish a cross-paradigm attempt (which needs immediate red-X
/// feedback per `docs/specs/gsim-sch-format.md` §8.3) from every other failure.
#[wasm_bindgen]
pub fn connect(
    model_json: &str,
    from_json: &str,
    to_json: &str,
    kind: &str,
    opts_json: &str,
) -> Result<String, JsValue> {
    connect_impl(model_json, from_json, to_json, kind, opts_json).map_err(js_err)
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
        let r3 = connect_impl(&model_json, &from, &to, "signal", "{}").unwrap();
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
            &connect_impl(&model_json, &from, &to, "physical-electrical", "{}").unwrap(),
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
        let err = connect_impl(&model_json, &from, &to, "physical-electrical", "{}").unwrap_err();
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
        let err = connect_impl(&model_json, &from, &to, "signal", "{}").unwrap_err();
        assert!(
            err.to_lowercase().contains("cannot connect")
                && err.contains("PhysicalNetwork")
                && err.contains("SignalFlow"),
            "expected a clear cross-paradigm message, got: {err}"
        );
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
            &connect_impl(&model_json, &from, &to, "physical-electrical", &opts).unwrap(),
        )
        .unwrap()["model"]
            .to_string();
        let err = connect_impl(&model_json, &from, &to, "physical-electrical", &opts).unwrap_err();
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
}
