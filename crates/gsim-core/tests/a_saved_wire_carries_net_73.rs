//! A wire this crate saves carries `net`, even when nothing named the net.
//!
//! `Wire::net` was `skip_serializing_if = "Option::is_none"`, like every other optional field
//! on the document types. For `label`, `subckt_ref`, `ratings`, `role`, `unit` and `doc` that
//! is correct -- the document schema does not require any of them. For `net` it was not: the
//! schema lists it in `Wire.required` and types it `string | null`, so `null` is a legal value
//! and an absent key is a validation failure.
//!
//! Nothing here noticed, because this crate reads its own output back without complaint --
//! `#[serde(default)]` fills the `None` straight back in. The failure surfaced two repositories
//! away, as `/model/wires/0: must have required property 'net'` from the editor's validator,
//! after all five of omni-schematics' committed example schematics turned out to be unopenable.
//! Every one of them is generated through `save_document`.
//!
//! So the assertion worth having is not "it round-trips" -- it always did. It is that the
//! serialised text contains the key. Both tests below were watched fail against the previous
//! `skip_serializing_if` before the attribute was removed.

use gsim_core::document::{GsimSchDocument, Model};

/// The document schema's `Wire.required`, restated.
///
/// Restated rather than read, because the schema file lives in the omni-schematics repository
/// and this crate deliberately depends on nothing there. That makes this list a copy, and a
/// copy can drift -- but a drifting copy still fails loudly here the moment a required field
/// stops being emitted, which is strictly better than the nothing that was checked before.
const WIRE_REQUIRED: [&str; 7] = ["id", "from", "to", "waypoints", "pinned", "net", "kind"];

/// A two-block sheet with one wire between them, carrying `net` only when given.
///
/// Parameterised rather than built once and patched: `json!(..).to_string()` emits compact
/// JSON, so a `.replace(r#""pinned": false"#, ..)` silently matches nothing and the "named
/// net" case quietly becomes the unnamed one. That is how the first version of this file
/// managed to fail for the wrong reason.
fn wire_sheet(net: Option<&str>) -> String {
    let mut value = serde_json::json!({
        "id": "sheet",
        "name": "sheet",
        "grid": 10,
        "params": [],
        "blocks": [
            {
                "id": "r1",
                "type": "passive.resistor",
                "paradigm": "physical-network",
                "domain": "physical-electrical",
                "fidelity": "ideal",
                "position": [0, 0],
                "rotation": 0,
                "mirror": "none",
                "params": {"r": 1.0},
                "ic": {}
            },
            {
                "id": "gnd",
                "type": "routing.ground",
                "paradigm": "routing",
                "domain": "physical-electrical",
                "fidelity": "ideal",
                "position": [0, 100],
                "rotation": 0,
                "mirror": "none",
                "params": {},
                "ic": {}
            }
        ],
        "wires": [
            {
                "id": "w1",
                "from": {"block": "r1", "port": "p"},
                "to": {"block": "gnd", "port": "gnd"},
                "kind": "physical-electrical",
                "waypoints": [],
                "pinned": false
            }
        ],
        "lines": [],
        "nets": [],
        "simulation": {},
        "metadata": {}
    });
    if let Some(name) = net {
        value["wires"][0]["net"] = serde_json::Value::String(name.to_owned());
    }
    value.to_string()
}

fn save(model_json: &str) -> serde_json::Value {
    let model: Model = serde_json::from_str(model_json).expect("the fixture parses");
    let text = GsimSchDocument::new(model)
        .to_json_pretty()
        .expect("the document serialises");
    serde_json::from_str(&text).expect("the output is JSON")
}

#[test]
fn an_unnamed_net_is_emitted_as_null_not_omitted() {
    let doc = save(&wire_sheet(None));
    let wire = &doc["model"]["wires"][0];

    // `.get` rather than indexing: indexing a missing key yields `Value::Null`, which is
    // exactly the value a passing test expects, so it cannot tell the two apart. That
    // conflation is the whole reason this went unnoticed.
    let net = wire
        .as_object()
        .expect("a wire is an object")
        .get("net")
        .expect("the wire carries a 'net' key");
    assert!(
        net.is_null(),
        "an unnamed net should serialise as null, got {net}"
    );
}

#[test]
fn a_saved_wire_carries_every_field_the_schema_requires() {
    let doc = save(&wire_sheet(None));
    let wire = doc["model"]["wires"][0]
        .as_object()
        .expect("a wire is an object")
        .clone();

    let missing: Vec<&str> = WIRE_REQUIRED
        .iter()
        .copied()
        .filter(|key| !wire.contains_key(*key))
        .collect();
    assert!(
        missing.is_empty(),
        "a saved wire is missing schema-required field(s) {missing:?}; it carries {:?}",
        wire.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_named_net_still_round_trips_as_its_name() {
    // The behaviour that already worked, pinned so the fix above cannot regress it into
    // always-null.
    let doc = save(&wire_sheet(Some("vout")));
    assert_eq!(doc["model"]["wires"][0]["net"], "vout");
}
