//! `.gsim_sch` §5.2 is evaluated, and the deck carries numbers — #94.
//!
//! What this replaces, measured on the bundle built from `5d4a83b` and recorded on the issue:
//!
//! | `params.R` | netlist line | notes | `.PARAM` cards |
//! |---|---|---|---|
//! | `"{3 + 4}"` | `rload n_cout_p 0 {3 + 4}` | `[]` | none |
//! | `"Rload"` (with `Rload = 7`) | `rload n_cout_p 0 Rload` | `[]` | none |
//!
//! No diagnostic, no note, and nothing that would let the solver resolve either — so the user
//! got `unexpected extra parameter '*'` from the solver about an expression the inspector had
//! just shown them the value of.
//!
//! The two properties worth testing separately, because they can each be true without the
//! other: the deck carries the **number**, and the document on disk still carries the
//! **expression**.

use gsim_core::compile::{compile, CompileError};
use gsim_core::document::GsimSchDocument;
use serde_json::json;

/// A textbook RC divider whose resistor's value comes from the sheet. Generic by construction
/// (`AGENTS.md` invariant 8).
fn sheet(params: serde_json::Value, r_value: serde_json::Value) -> String {
    json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": params,
        "blocks": [
          { "id": "V1", "type": "source.voltage.dc", "paradigm": "physical-network",
            "domain": "electrical", "position": [0, 0], "params": { "V": 5 } },
          { "id": "Rload", "type": "passive.resistor", "paradigm": "physical-network",
            "domain": "electrical", "position": [100, 0], "params": { "R": r_value } },
          { "id": "GND", "type": "routing.ground", "paradigm": "physical-network",
            "domain": "electrical", "position": [50, 100], "params": {} }
        ],
        "wires": [
          { "id": "w1", "kind": "physical-electrical", "net": "n1",
            "from": { "block": "V1", "port": "p" }, "to": { "block": "Rload", "port": "p" } },
          { "id": "w2", "kind": "physical-electrical", "net": "n0",
            "from": { "block": "Rload", "port": "n" }, "to": { "block": "GND", "port": "gnd" } },
          { "id": "w3", "kind": "physical-electrical", "net": "n0",
            "from": { "block": "V1", "port": "n" }, "to": { "block": "GND", "port": "gnd" } }
        ]
      }
    })
    .to_string()
}

fn card(netlist: &str, name: &str) -> String {
    netlist
        .lines()
        .find(|l| l.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("no card named {name} in:\n{netlist}"))
        .to_string()
}

// -- the deck carries numbers ---------------------------------------------------------------

#[test]
fn an_expression_in_a_block_param_reaches_the_deck_as_a_number() {
    let model = GsimSchDocument::from_json(&sheet(json!([]), json!("{3 + 4}")))
        .unwrap()
        .model;
    let out = compile(&model, "general-simulator").unwrap();
    assert!(
        card(&out.netlist, "Rload").ends_with(" 7"),
        "{}",
        out.netlist
    );
}

#[test]
fn a_reference_to_a_sheet_parameter_reaches_the_deck_as_its_value() {
    let model = GsimSchDocument::from_json(&sheet(
        json!([{ "name": "Rload_value", "value": 7 }]),
        json!("Rload_value"),
    ))
    .unwrap()
    .model;
    let out = compile(&model, "general-simulator").unwrap();
    assert!(
        card(&out.netlist, "Rload").ends_with(" 7"),
        "{}",
        out.netlist
    );
}

#[test]
fn an_expression_over_a_sheet_parameter_reaches_the_deck_evaluated() {
    let model = GsimSchDocument::from_json(&sheet(
        json!([{ "name": "Rload_value", "value": 7 }]),
        json!("{Rload_value * 2}"),
    ))
    .unwrap()
    .model;
    let out = compile(&model, "general-simulator").unwrap();
    assert!(
        card(&out.netlist, "Rload").ends_with(" 14"),
        "{}",
        out.netlist
    );
}

/// A chain, resolved in dependency order rather than document order — and in a unit the sheet
/// would really use, because scientific notation is where a shared lexer would have broken.
#[test]
fn a_chain_of_parameters_resolves_whatever_order_it_is_written_in() {
    let model = GsimSchDocument::from_json(&sheet(
        json!([
            { "name": "Rseries", "value": "{Rbase * 0.1}" },
            { "name": "Rbase", "value": 1.5e3 },
            { "name": "Rtotal", "value": "{Rbase + Rseries}" }
        ]),
        json!("Rtotal"),
    ))
    .unwrap()
    .model;
    let out = compile(&model, "general-simulator").unwrap();
    assert!(
        card(&out.netlist, "Rload").ends_with(" 1650"),
        "{}",
        out.netlist
    );
}

#[test]
fn the_resolved_table_comes_back_so_a_front_end_never_evaluates_5_2_itself() {
    let model = GsimSchDocument::from_json(&sheet(
        json!([
            { "name": "Rbase", "value": 1000 },
            { "name": "Rseries", "value": "{Rbase * 0.1}" },
            { "name": "Tend", "value": "{5 / 1000}" }
        ]),
        json!("Rbase"),
    ))
    .unwrap()
    .model;
    let out = compile(&model, "general-simulator").unwrap();
    assert_eq!(out.params["Rbase"], json!(1000));
    assert_eq!(out.params["Rseries"], json!(100));
    assert_eq!(out.params["Tend"], json!(0.005));
}

// -- and the document keeps the expression --------------------------------------------------

/// The property that makes resolving safe at all. Substituting on load would mean a sheet
/// written with `"R": "{Rload_value * 2}"` came back from a save with `"R": 14` — the user's
/// parameterisation deleted because they opened and saved their own file. So substitution lives
/// on the compile path, and this test is what stops a later reader "simplifying" it onto the
/// loader.
#[test]
fn a_round_trip_still_carries_the_expression_the_user_wrote() {
    let text = sheet(
        json!([{ "name": "Rload_value", "value": 7 }]),
        json!("{Rload_value * 2}"),
    );
    let doc = GsimSchDocument::from_json(&text).unwrap();
    let saved = doc.to_json_pretty().unwrap();
    assert!(saved.contains("{Rload_value * 2}"), "{saved}");
    assert!(!saved.contains("\"R\": 14"), "{saved}");

    // Compiling it does not change it either: `compile` takes `&Model` and substitutes a clone.
    let out = compile(&doc.model, "general-simulator").unwrap();
    assert!(card(&out.netlist, "Rload").ends_with(" 14"));
    let saved_again = doc.to_json_pretty().unwrap();
    assert_eq!(saved, saved_again);
}

// -- cycles, and the message -----------------------------------------------------------------

#[test]
fn a_cycle_is_rejected_with_the_loop_in_the_message() {
    let model = GsimSchDocument::from_json(&sheet(
        json!([
            { "name": "A", "value": "{B + 1}" },
            { "name": "B", "value": "{A + 1}" }
        ]),
        json!(10),
    ))
    .unwrap()
    .model;
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    assert!(matches!(err, CompileError::BadParams(_)));
    let message = err.to_string();
    assert!(
        message.contains("A -> B -> A") || message.contains("B -> A -> B"),
        "{message}"
    );
    assert!(message.contains("in terms of each other"), "{message}");
}

// -- every use site, not only a block param --------------------------------------------------

#[test]
fn an_initial_condition_resolves_like_a_parameter() {
    let text = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": [{ "name": "v0", "value": 2.5 }],
        "blocks": [
          { "id": "C1", "type": "passive.capacitor", "paradigm": "physical-network",
            "domain": "electrical", "position": [0, 0],
            "params": { "C": 1e-6 }, "ic": { "V": "{v0 * 2}" } },
          { "id": "R1", "type": "passive.resistor", "paradigm": "physical-network",
            "domain": "electrical", "position": [100, 0], "params": { "R": 1000 } },
          { "id": "GND", "type": "routing.ground", "paradigm": "physical-network",
            "domain": "electrical", "position": [50, 100], "params": {} }
        ],
        "wires": [
          { "id": "w1", "kind": "physical-electrical", "net": "n1",
            "from": { "block": "C1", "port": "p" }, "to": { "block": "R1", "port": "p" } },
          { "id": "w2", "kind": "physical-electrical", "net": "n0",
            "from": { "block": "C1", "port": "n" }, "to": { "block": "GND", "port": "gnd" } },
          { "id": "w3", "kind": "physical-electrical", "net": "n0",
            "from": { "block": "R1", "port": "n" }, "to": { "block": "GND", "port": "gnd" } }
        ]
      }
    })
    .to_string();
    let model = GsimSchDocument::from_json(&text).unwrap().model;
    // Nothing in this crate emits an `ic` yet, so the observable effect is that it is *resolved*
    // rather than accepted as text — which is what #94's "every use site" asks for. The
    // document is unchanged; the compile's view of it is not.
    let out = compile(&model, "general-simulator");
    assert!(out.is_ok(), "{:?}", out.err());
    let resolved =
        gsim_core::params::substitute(&model, &gsim_core::params::resolve_table(&model).unwrap())
            .unwrap();
    assert_eq!(resolved.blocks[0].ic["V"], json!(5));
    // And the document still says what the user wrote.
    assert_eq!(model.blocks[0].ic["V"], json!("{v0 * 2}"));
}

/// §5.3's third use site. `omni-schematics#244` fixed the editor half — a run whose `t_end` the
/// editor could not read no longer silently substitutes the settings panel's own — and this is
/// the engine half: an analysis directive carrying an expression is resolved, not accepted in
/// silence.
#[test]
fn an_analysis_directives_numeric_field_resolves() {
    let text = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": [{ "name": "T_sim", "value": "{5 / 1000}" }],
        "blocks": [], "wires": [],
        "simulation": {
          "default_analysis": "tran1",
          "analyses": [
            { "id": "tran1", "type": "TRAN", "t_end": "{T_sim}", "t_step": "{T_sim / 500}" }
          ]
        }
      }
    })
    .to_string();
    let model = GsimSchDocument::from_json(&text).unwrap().model;
    let table = gsim_core::params::resolve_table(&model).unwrap();
    let resolved = gsim_core::params::substitute(&model, &table).unwrap();
    let tran = &resolved.simulation["analyses"][0];
    assert_eq!(tran["t_end"], json!(0.005));
    assert_eq!(tran["t_step"], json!(1e-5));
    // Unchanged on disk.
    assert_eq!(model.simulation["analyses"][0]["t_end"], json!("{T_sim}"));
}

#[test]
fn an_unresolvable_analysis_field_refuses_rather_than_passing_through() {
    let text = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": [], "blocks": [], "wires": [],
        "simulation": {
          "analyses": [{ "id": "tran1", "type": "TRAN", "t_end": "{T_sim * 2}" }]
        }
      }
    })
    .to_string();
    let model = GsimSchDocument::from_json(&text).unwrap().model;
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    let message = err.to_string();
    assert!(matches!(err, CompileError::BadParams(_)), "{message}");
    assert!(message.contains("T_sim"), "{message}");
    assert!(message.contains("simulation directive"), "{message}");
}

// -- what stays a value rather than becoming a reference -------------------------------------

/// The refusal that would have made this feature unusable, so it has a test of its own: a bare
/// identifier that is not a declared parameter is left alone. `ts=variable`, a `kind=` variant,
/// an output name and a file path are all written that way, and treating them as unresolved
/// references would refuse almost every sheet on the way to fixing #94.
#[test]
fn a_bare_identifier_that_names_no_parameter_is_left_alone() {
    let text = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": [],
        "blocks": [
          { "id": "SRC", "type": "source.const", "paradigm": "signal-flow",
            "domain": "signal", "position": [0, 0], "params": { "value": 1 } },
          { "id": "CTRL", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 0],
            "params": { "lib": "./ctrl.so", "ts": "variable",
                        "input_names": ["e"], "output_names": ["d"] } }
        ],
        "wires": [
          { "id": "w1", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" },
            "to": { "block": "CTRL", "port": "in0" } }
        ]
      }
    })
    .to_string();
    let model = GsimSchDocument::from_json(&text).unwrap().model;
    let out = compile(&model, "general-simulator").unwrap();
    assert!(
        out.netlist.contains("lib=./ctrl.so") && out.netlist.contains("ts=variable"),
        "{}",
        out.netlist
    );
}

#[test]
fn a_sheet_with_no_params_compiles_exactly_as_it_did() {
    let model = GsimSchDocument::from_json(&sheet(json!([]), json!(1000)))
        .unwrap()
        .model;
    let out = compile(&model, "general-simulator").unwrap();
    assert!(card(&out.netlist, "Rload").ends_with(" 1000"));
    assert!(out.params.is_empty());
}
