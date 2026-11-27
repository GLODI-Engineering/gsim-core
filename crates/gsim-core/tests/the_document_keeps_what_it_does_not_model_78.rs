//! A document round trip loses no key, including the keys this crate does not model.
//!
//! `save_document` — the `gsim_py` wheel's, the WebAssembly bundle's `saveDocument`, and the
//! Rust crate's — is a parse followed by a serialize. Serde drops an unrecognised key at the
//! *parse* step, so until #78 the round trip deleted four features of the `.gsim_sch` format
//! outright, with no error and no warning:
//!
//! | what went in | what came back |
//! |---|---|
//! | `model.blocks[].subsystem` — an entire nested model | the key was absent, inner blocks and wires gone |
//! | `model.subckt_defs[].body` — a netlist-only `.SUBCKT`'s SPICE text | absent |
//! | `model.ports` — a subsystem's boundary ports | absent |
//! | `model.parent_param_aliases` | absent |
//!
//! Scripting through `gsim_py` is the recommended way to build a document programmatically, so
//! the audience this API exists for was the audience losing data.
//!
//! The fix is a flattened catch-all on each struct of the document tree rather than four typed
//! fields, which is why the last case here matters most: a field *no* struct models survives
//! too, so the next addition to the format does not need #78 reopened.
//!
//! These assertions compare parsed JSON structurally. A string comparison would pass or fail
//! for the wrong reason — the catch-all's keys are emitted after the modelled ones, so the key
//! *order* legitimately changes while no key is lost.

use gsim_core::document::GsimSchDocument;
use serde_json::Value;

/// A document carrying all four features, plus a field invented for this test that no struct
/// anywhere models.
///
/// The resistor's `R` is spelled `1000.0` rather than `1000`, and the distinction is #107's:
/// the library declares `R` real, so the loader attaches that form at the boundary and a
/// fixture written `1000` comes back `1000.0`. That is a form being *declared*, not a key being
/// lost, so it is asserted in `the_number_carries_its_declared_form_107.rs` and this file's
/// value check stays exact.
fn document_with_everything() -> &'static str {
    r#"{
  "schema_version": 2,
  "model": {
    "id": "top",
    "name": "Top",
    "grid": 10,
    "params": [{ "name": "fsw", "value": 100000, "unit": "Hz", "provenance": "datasheet" }],
    "blocks": [
      {
        "id": "sub1",
        "type": "hierarchy.subsystem",
        "paradigm": "hierarchy",
        "domain": "mixed",
        "fidelity": "n_a",
        "position": [100, 200],
        "rotation": 0,
        "mirror": "none",
        "params": {},
        "ic": {},
        "z_order": 7,
        "subsystem": {
          "id": "inner",
          "name": "Inner",
          "grid": 10,
          "params": [],
          "blocks": [
            {
              "id": "gi",
              "type": "control.gain",
              "paradigm": "signal-flow",
              "domain": "signal",
              "fidelity": "n_a",
              "position": [0, 0],
              "rotation": 0,
              "mirror": "none",
              "params": { "k": 2.5 },
              "ic": {}
            }
          ],
          "wires": [],
          "lines": [],
          "nets": [],
          "ports": [{ "id": "p1", "direction": "in", "kind": "signal" }],
          "parent_param_aliases": [{ "inner": "x", "outer": "fsw" }],
          "simulation": {},
          "metadata": {}
        }
      },
      {
        "id": "R1",
        "type": "passive.resistor",
        "paradigm": "physical-network",
        "domain": "electrical",
        "fidelity": "ideal",
        "position": [0, 0],
        "rotation": 90,
        "mirror": "none",
        "params": { "R": 1000.0 },
        "ic": {},
        "subsystem": null
      }
    ],
    "wires": [
      {
        "id": "w1",
        "from": { "block": "R1", "port": "p" },
        "to": { "block": "R1", "port": "n" },
        "kind": "physical-electrical",
        "waypoints": [[10, 10]],
        "pinned": false,
        "net": "vout",
        "label": "out",
        "stroke_hint": "dashed"
      }
    ],
    "subckt_defs": [
      {
        "id": "od",
        "name": "OPAMP",
        "body": "E1 out 0 inp inn 1e5\nRout out 0 1e6",
        "ports": [
          { "id": "inp", "kind": "physical-electrical", "tooltip": "non-inverting" },
          { "id": "out", "kind": "physical-electrical" }
        ],
        "params": [{ "name": "gain", "value": 100000.0 }],
        "instances": [{ "id": "X1", "def_ref": "od", "connections": ["inp", "out"], "note": "self" }]
      }
    ],
    "role": "library",
    "lines": [{ "id": "a1", "kind": "text", "position": [0, 0], "text": "hello" }],
    "nets": [{ "name": "vout", "kind": "physical-electrical", "scope": "local" }],
    "ports": [{ "id": "p1", "direction": "in", "kind": "signal" }],
    "parent_param_aliases": [{ "inner": "x", "outer": "fsw" }],
    "simulation": { "analyses": [] },
    "metadata": { "author": "nobody" },
    "a_field_from_a_later_format_version": { "deep": [1, 2.5, null, "text"] }
  },
  "an_unmodelled_document_level_key": true
}"#
}

/// The round trip every binding performs: parse the whole document, then serialize it back.
fn round_trip(text: &str) -> Value {
    let doc = GsimSchDocument::from_json(text).expect("the fixture is a valid v2 document");
    serde_json::from_str(&doc.to_json_pretty().expect("serializing it back")).expect("valid JSON")
}

/// Every key reachable in `wanted` is reachable in `got`, with an equal value.
///
/// Deliberately a *superset* check rather than equality: the engine legitimately materialises
/// defaults the input omitted (`net: null`, `subckt_defs: []`, `fidelity: "ideal"`). What #78
/// is about is the other direction — a key going in and not coming out.
fn assert_nothing_lost(wanted: &Value, got: &Value, path: &str) {
    match (wanted, got) {
        (Value::Object(w), Value::Object(g)) => {
            for (key, value) in w {
                let here = format!("{path}/{key}");
                let Some(mine) = g.get(key) else {
                    panic!("{here} was lost: the round trip did not emit it at all");
                };
                assert_nothing_lost(value, mine, &here);
            }
        }
        (Value::Array(w), Value::Array(g)) => {
            assert_eq!(w.len(), g.len(), "{path}: array length changed");
            for (i, value) in w.iter().enumerate() {
                assert_nothing_lost(value, &g[i], &format!("{path}/{i}"));
            }
        }
        _ => assert_eq!(wanted, got, "{path}: the value changed"),
    }
}

#[test]
fn a_document_carrying_all_four_features_loses_no_key() {
    let text = document_with_everything();
    let before: Value = serde_json::from_str(text).unwrap();
    let after = round_trip(text);
    assert_nothing_lost(&before, &after, "");
}

#[test]
fn the_four_features_come_back_with_their_contents_intact() {
    // The superset check above would also pass on an empty-but-present key, so each of the four
    // is named here and asserted on its content -- which is what a user would lose.
    let after = round_trip(document_with_everything());
    let model = &after["model"];

    let subsystem = &model["blocks"][0]["subsystem"];
    assert_eq!(subsystem["blocks"][0]["id"], "gi");
    assert_eq!(subsystem["blocks"][0]["params"]["k"], 2.5);
    assert_eq!(subsystem["ports"][0]["id"], "p1");
    assert_eq!(subsystem["parent_param_aliases"][0]["outer"], "fsw");

    assert_eq!(
        model["subckt_defs"][0]["body"],
        "E1 out 0 inp inn 1e5\nRout out 0 1e6"
    );
    assert_eq!(model["ports"][0]["direction"], "in");
    assert_eq!(model["parent_param_aliases"][0]["inner"], "x");

    // `"subsystem": null` is a key with a value, not an absent key -- the editor writes it on
    // every non-subsystem block, so dropping it is still a difference between the two writers.
    assert_eq!(model["blocks"][1]["subsystem"], Value::Null);
}

#[test]
fn a_field_no_struct_models_survives_at_every_level_of_the_tree() {
    // The point of a catch-all rather than four typed fields: the fifth field costs nothing.
    let after = round_trip(document_with_everything());
    assert_eq!(after["an_unmodelled_document_level_key"], true);

    let model = &after["model"];
    assert_eq!(
        model["a_field_from_a_later_format_version"]["deep"],
        serde_json::json!([1, 2.5, null, "text"])
    );
    assert_eq!(model["params"][0]["provenance"], "datasheet");
    assert_eq!(model["blocks"][0]["z_order"], 7);
    assert_eq!(model["wires"][0]["stroke_hint"], "dashed");
    assert_eq!(
        model["subckt_defs"][0]["ports"][0]["tooltip"],
        "non-inverting"
    );
    assert_eq!(model["subckt_defs"][0]["instances"][0]["note"], "self");
}

/// A document that uses none of this must serialize exactly as it did before #78 -- not
/// "equivalently", byte for byte.
///
/// The golden string below was produced by the 0.6.0 build, before the catch-all existed, from
/// the input directly above it. It exercises every struct the catch-all was added to
/// (`GsimSchDocument`, `Model`, `ModelParam`, `Block`, `Wire`, `SubcktDef`, `SubcktPort`,
/// `SubcktInstance`), so a flattened struct that reordered or re-spelled its output would show
/// up here rather than in a user's diff.
#[test]
fn a_document_using_none_of_it_serializes_byte_for_byte_as_it_did_before() {
    const INPUT: &str = include_str!("fixtures/golden_input.json");

    let doc = GsimSchDocument::from_json(INPUT).unwrap();
    let got = doc.to_json_pretty().unwrap();
    assert_eq!(
        got,
        include_str!("fixtures/document_0_6_0_output.json").trim_end(),
        "a document using no unmodelled field must serialize exactly as 0.6.0 serialized it"
    );
}
