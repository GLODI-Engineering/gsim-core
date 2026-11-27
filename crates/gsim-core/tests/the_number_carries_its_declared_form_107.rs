//! A number carries the form the library declares for it, from the moment it enters (#107).
//!
//! The defect, in the owner's own reproduction: `gsim-tools` writes a Python `int` for a
//! parameter this crate declares real, `save_document` is a parse followed by a serialize, and
//! so the document says integer. A Python reader of it is told the author meant an integer;
//! Rust and JavaScript widen both to `f64` and cannot tell, which is why two of the three
//! writers never noticed.
//!
//! The fix is a lookup at the boundary rather than a policy at write time, which is the owner's
//! decision: *"enforce from the beginning the type of the number so that the type may be
//! attached to it."* Every writer is then correct by construction, because the number already
//! knows what it is.
//!
//! These are the end-to-end assertions — through `from_json`, `to_json_pretty`, and the builder
//! — over the real library rather than a fixture catalogue. The unit rules live in
//! `document::number_form`.

use gsim_core::document::{
    Block, DocumentLoadError, GsimSchDocument, Mirror, ModelBuilder, NewBlockSpec, Paradigm,
};
use serde_json::{json, Value};

/// A one-resistor sheet whose `R` is written however `params` says.
fn sheet_with(params: &str) -> String {
    format!(
        r#"{{
  "schema_version": 2,
  "model": {{
    "id": "m",
    "name": "M",
    "grid": 10,
    "params": [],
    "blocks": [
      {{
        "id": "R1",
        "type": "passive.resistor",
        "paradigm": "physical-network",
        "domain": "physical-electrical",
        "fidelity": "ideal",
        "position": [0, 0],
        "params": {params}
      }}
    ],
    "wires": []
  }}
}}"#
    )
}

/// `R1`'s params, as the document comes back out.
fn saved_params(text: &str) -> Value {
    let doc = GsimSchDocument::from_json(text).expect("a valid v2 document");
    let json: Value = serde_json::from_str(&doc.to_json_pretty().unwrap()).unwrap();
    json["model"]["blocks"][0]["params"].clone()
}

#[test]
fn a_sheet_that_arrives_with_an_integer_resistance_is_saved_declaring_a_real() {
    // The repair, end to end. 5 and 5.0 are the same double, so nothing about the simulation
    // changes; what changes is that the saved document no longer says the author meant an
    // integer for a parameter the library declares real.
    let saved = saved_params(&sheet_with(r#"{ "R": 5 }"#));
    assert_eq!(serde_json::to_string(&saved).unwrap(), r#"{"R":5.0}"#);
}

#[test]
fn a_sheet_that_already_declares_it_is_left_exactly_as_it_was() {
    for written in ["5.0", "1e-9", "0.1", "1000.0"] {
        let before: Value = serde_json::from_str(written).unwrap();
        let saved = saved_params(&sheet_with(&format!(r#"{{ "R": {written} }}"#)));
        assert_eq!(
            saved["R"].as_f64().unwrap().to_bits(),
            before.as_f64().unwrap().to_bits(),
            "{written} did not survive the round trip bit for bit"
        );
    }
}

#[test]
fn an_expression_survives_the_load_untouched() {
    // §5.3 substitution happens on the compile path, never on load: a loader that resolved
    // `{Rload * 2}` would have `save_document` write the resolved number into the user's file
    // and destroy the parameterisation that makes one sheet serve a sweep (#94's own reasoning).
    let saved = saved_params(&sheet_with(r#"{ "R": "{Rload * 2}" }"#));
    assert_eq!(saved["R"], json!("{Rload * 2}"));
}

#[test]
fn a_parameter_the_library_does_not_declare_is_carried_as_given() {
    // The third form, which is an answer rather than a fallback: nothing in the library says
    // what `tolerance` is, so inventing a form for it would be a guess written into a user's
    // file.
    let saved = saved_params(&sheet_with(r#"{ "R": 5, "tolerance": 1 }"#));
    assert_eq!(
        serde_json::to_string(&saved).unwrap(),
        r#"{"R":5.0,"tolerance":1}"#
    );
}

#[test]
fn a_matrix_keeps_its_integer_elements_because_the_declaration_names_the_container() {
    // One of the 13 `vector`/`matrix` parameters. A state matrix is real-valued, and the
    // library says `matrix` without saying what its elements are -- so this crate does not know,
    // and says so by changing nothing.
    let text = sheet_with(r#"{ "R": 5 }"#).replace(
        r#""type": "passive.resistor""#,
        r#""type": "control.state_space""#,
    );
    let text = text.replace(r#"{ "R": 5 }"#, r#"{ "a": [[0]], "b": [[1]] }"#);
    let saved = saved_params(&text);
    assert_eq!(serde_json::to_string(&saved["a"]).unwrap(), "[[0]]");
}

#[test]
fn a_fractional_value_for_an_integer_parameter_is_refused_on_load_and_names_it() {
    // `control.product` declares `n_inputs` an integer. Truncating 4.5 to 4 is the silent
    // wrongness this issue exists to end, so the load fails -- and the message says which block
    // and which parameter rather than "something, somewhere, wrote a number badly".
    let text = sheet_with(r#"{ "n_inputs": 4.5 }"#).replace(
        r#""type": "passive.resistor""#,
        r#""type": "control.product""#,
    );
    let err = GsimSchDocument::from_json(&text).expect_err("4.5 is not an integer");
    assert!(
        matches!(err, DocumentLoadError::WrongNumberForm(_)),
        "{err:?}"
    );
    let said = err.to_string();
    assert!(said.contains("block 'R1' parameter 'n_inputs'"), "{said}");
    assert!(said.contains("4.5"), "{said}");
}

#[test]
fn an_integral_real_narrows_for_an_integer_parameter() {
    let text = sheet_with(r#"{ "n_inputs": 4.0 }"#).replace(
        r#""type": "passive.resistor""#,
        r#""type": "control.product""#,
    );
    assert_eq!(
        serde_json::to_string(&saved_params(&text)).unwrap(),
        r#"{"n_inputs":4}"#
    );
}

#[test]
fn set_param_attaches_the_form_rather_than_storing_what_it_was_handed() {
    let mut builder = ModelBuilder::new("m", "M", 10);
    let id = builder
        .add_block(NewBlockSpec {
            id: Some("R1".to_string()),
            r#type: "passive.resistor".to_string(),
            paradigm: Paradigm::PhysicalNetwork,
            domain: "physical-electrical".to_string(),
            position: (0, 0),
            ..Default::default()
        })
        .expect("a resistor is placeable");
    builder.set_param(&id, "R", json!(47)).expect("47 ohms");
    let saved = serde_json::to_string(&builder.model().blocks[0].params).unwrap();
    assert_eq!(saved, r#"{"R":47.0}"#);
}

#[test]
fn add_block_attaches_the_form_of_every_parameter_it_is_given() {
    let mut builder = ModelBuilder::new("m", "M", 10);
    let mut params = serde_json::Map::new();
    params.insert("R".to_string(), json!(1000));
    builder
        .add_block(NewBlockSpec {
            id: Some("R1".to_string()),
            r#type: "passive.resistor".to_string(),
            paradigm: Paradigm::PhysicalNetwork,
            domain: "physical-electrical".to_string(),
            position: (0, 0),
            params,
            ..Default::default()
        })
        .expect("a resistor is placeable");
    assert_eq!(
        serde_json::to_string(&builder.model().blocks[0].params).unwrap(),
        r#"{"R":1000.0}"#
    );
}

#[test]
fn a_refused_value_leaves_no_half_placed_block_behind() {
    // The fail-before-mutating rule `add_subckt_def` states for itself, applied here: the form
    // is attached before the block exists, so a refusal cannot leave a model in a
    // construction-failed-but-mutated state.
    let mut builder = ModelBuilder::new("m", "M", 10);
    let mut params = serde_json::Map::new();
    params.insert("n_inputs".to_string(), json!(4.5));
    let err = builder
        .add_block(NewBlockSpec {
            id: Some("P1".to_string()),
            r#type: "control.product".to_string(),
            paradigm: Paradigm::SignalFlow,
            domain: "signal".to_string(),
            position: (0, 0),
            params,
            ..Default::default()
        })
        .expect_err("4.5 is not an integer");
    assert!(err.to_string().contains("parameter 'n_inputs'"), "{err}");
    assert!(
        builder.model().blocks.is_empty(),
        "the refused block was placed anyway"
    );
}

#[test]
fn a_model_level_parameter_is_carried_as_given_because_nothing_declares_it() {
    // One of the 11 document-level sites #268 declined deliberately: a model-level parameter is
    // whatever its author meant. `add_model_param` therefore has no lookup to do, and this
    // asserts the absence of one -- it would fail if someone gave model params a policy.
    let mut builder = ModelBuilder::new("m", "M", 10);
    builder.add_model_param("Rload", json!(7)).expect("a param");
    assert_eq!(
        serde_json::to_string(&builder.model().params[0].value).unwrap(),
        "7"
    );
}

#[test]
fn nothing_in_this_test_passed_by_looking_at_an_empty_library() {
    // The guard every rule in this repository now carries: a lookup that finds nothing answers
    // "as given" and changes nothing, so every assertion above would pass against an empty
    // catalogue. These two are the ones that cannot.
    use gsim_core::document::{declared_form, DeclaredForm};
    assert_eq!(
        declared_form("passive.resistor", "ideal", "R"),
        DeclaredForm::Real
    );
    assert_eq!(
        declared_form("control.product", "ideal", "n_inputs"),
        DeclaredForm::Integer
    );
    // And a block built by struct literal rather than through the builder still round-trips,
    // so the load arm is what repairs a document rather than the constructor.
    let block = Block {
        params: serde_json::Map::new(),
        ..serde_json::from_str::<Block>(
            r#"{"id":"R1","type":"passive.resistor","paradigm":"physical-network","domain":"physical-electrical","position":[0,0]}"#,
        )
        .unwrap()
    };
    assert_eq!(block.mirror, Mirror::None);
}
