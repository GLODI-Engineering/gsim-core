//! A `-D` a code block is compiled with comes from the same parameter table the netlist is
//! generated from, or there is no netlist — `docs/decisions/0009`, #100.
//!
//! The refusal is the subject here, not the happy path, because the failure it prevents is the
//! one this repository family keeps finding: a block whose constants disagree with the circuit
//! it is wired into runs, exits 0, writes a complete result and reports a confident wrong
//! answer (`GLODI-Engineering/omni-schematics#235`). A failed compile costs a minute. A wrong
//! constant survives a study.
//!
//! **Every refusal test here has a control** — the same document with the one thing fixed,
//! compiling. Without it a refusal test passes against an implementation that refuses
//! everything, which is the standard way this kind of test is worthless.

use gsim_core::compile::{compile, CompileError};
use gsim_core::document::{GsimSchDocument, Model};
use serde_json::json;

/// A generic one-block sheet: a C block fed by a constant, with a plan that compiles a thin
/// adapter plus a shared source. Textbook-generic by construction — nothing here is modelled on
/// any real design (`AGENTS.md` invariant 8).
fn sheet(params: serde_json::Value, defines: serde_json::Value) -> Model {
    let doc = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": params,
        "code_build": {
          "includes": ["code"],
          "defines": defines,
          "libs": ["m"],
          "steps": [{
            "id": "ctrl", "kind": "shared-library", "lang": "c",
            "sources": ["code/cscript_ctrl.c", "code/control_law.c"]
          }]
        },
        "blocks": [
          { "id": "SRC", "type": "source.const", "paradigm": "signal-flow",
            "domain": "signal", "position": [0, 0], "params": { "value": 1.0 } },
          { "id": "CTRL", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 0],
            "params": { "lib": "runs/case3/ctrl", "input_names": ["e"], "output_names": ["d"] },
            "source": "code/cscript_ctrl.c",
            "build": "ctrl" }
        ],
        "wires": [
          { "id": "w1", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" },
            "to": { "block": "CTRL", "port": "in0" } }
        ]
      }
    });
    GsimSchDocument::from_json(&doc.to_string())
        .expect("fixture parses")
        .model
}

fn one_define() -> serde_json::Value {
    json!([{ "name": "L_F", "from": "L_f" }])
}

// -- the refusal, and its control ------------------------------------------------------------

#[test]
fn a_define_whose_parameter_is_not_in_the_sheet_refuses_and_names_both() {
    let model = sheet(
        json!([{ "name": "something_else", "value": 1.0 }]),
        one_define(),
    );
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    match &err {
        CompileError::UnderivableDefine { define, param, .. } => {
            assert_eq!(define, "L_F");
            assert_eq!(param, "L_f");
        }
        other => panic!("wrong refusal: {other:?}"),
    }
    // The message, not only the variant: a user reads the sentence, and it has to carry both
    // names and say why it did not guess.
    let message = err.to_string();
    assert!(message.contains("L_F"), "{message}");
    assert!(message.contains("L_f"), "{message}");
    assert!(
        message.contains("rather than guess"),
        "the message must say it refused on purpose: {message}"
    );
    // And what the sheet does declare, so a misspelt `from` is a one-line fix rather than a
    // dead end -- the courtesy 0005 settled for a capture whose column is not there.
    assert!(message.contains("something_else"), "{message}");
}

/// The control. The same sheet with the parameter present compiles, and the resolved define
/// carries the parameter's value — so the test above is about the missing parameter and not
/// about an implementation that refuses every sheet with a plan.
#[test]
fn the_same_sheet_with_the_parameter_present_builds() {
    let model = sheet(json!([{ "name": "L_f", "value": 2.8e-6 }]), one_define());
    let out = compile(&model, "general-simulator").expect("must compile");
    let build = out.code_build.expect("a sheet with a plan resolves one");
    assert_eq!(build.steps.len(), 1);
    let step = &build.steps[0];
    assert_eq!(step.defines, vec!["L_F=2.8e-6".to_string()]);
    assert!(step.omitted_defines.is_empty());
    assert_eq!(step.artifact, "runs/case3/ctrl");
    assert_eq!(step.blocks, vec!["CTRL".to_string()]);
    assert_eq!(step.includes, vec!["code".to_string()]);
    assert_eq!(step.libs, vec!["m".to_string()]);
    assert_eq!(
        step.sources,
        vec![
            "code/cscript_ctrl.c".to_string(),
            "code/control_law.c".to_string()
        ]
    );
}

/// The value a define carries is the value the netlist carries, formatted by one function. This
/// is the claim the whole record rests on, so it is asserted against the deck itself rather
/// than against another copy of the formatter.
#[test]
fn the_define_and_the_netlist_agree_on_how_the_number_prints() {
    let mut model = sheet(json!([{ "name": "g", "value": 3.0 }]), json!([]));
    model.blocks[0].params.insert("value".into(), json!(3.0));
    model.code_build.as_mut().unwrap().defines = serde_json::from_value(json!([
        { "name": "GAIN", "from": "g" }
    ]))
    .unwrap();
    let out = compile(&model, "general-simulator").expect("must compile");
    let step = &out.code_build.as_ref().unwrap().steps[0];
    assert_eq!(step.defines, vec!["GAIN=3".to_string()]);
    // `source.const value=3.0` prints the same `3`, because both go through `crate::fmt`.
    assert!(
        out.netlist.contains("value=3\n") || out.netlist.contains("value=3 "),
        "{}",
        out.netlist
    );
}

// -- the expression case, which is what #94 makes real --------------------------------------

#[test]
fn a_parameter_holding_an_expression_is_underivable_and_says_so() {
    let model = sheet(
        json!([{ "name": "L_f", "value": "{L_base * 2}" }]),
        one_define(),
    );
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    let message = err.to_string();
    assert!(
        matches!(err, CompileError::UnderivableDefine { .. }),
        "{message}"
    );
    assert!(message.contains("{L_base * 2}"), "{message}");
    assert!(message.contains("#94"), "{message}");
}

#[test]
fn a_parameter_holding_a_bare_reference_is_underivable_too() {
    let model = sheet(json!([{ "name": "L_f", "value": "L_base" }]), one_define());
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::UnderivableDefine { .. })
    ));
}

// -- optional: the author's declared absence, and nothing else -------------------------------

#[test]
fn an_optional_define_whose_parameter_is_absent_builds_and_records_the_omission() {
    let model = sheet(
        json!([]),
        json!([{ "name": "DEBUG_LOG", "from": "debug_log", "optional": true }]),
    );
    let out = compile(&model, "general-simulator").expect("must compile");
    let step = &out.code_build.as_ref().unwrap().steps[0];
    assert!(step.defines.is_empty());
    assert_eq!(step.omitted_defines, vec!["DEBUG_LOG".to_string()]);
}

/// `optional` forgives the parameter not being there. It does not forgive the parameter being
/// there and unusable: the author asked for that parameter's value, and the value is not
/// available, which is a different sentence.
#[test]
fn an_optional_define_whose_parameter_holds_an_expression_is_still_refused() {
    let model = sheet(
        json!([{ "name": "debug_log", "value": "{n + 1}" }]),
        json!([{ "name": "DEBUG_LOG", "from": "debug_log", "optional": true }]),
    );
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::UnderivableDefine { .. })
    ));
}

#[test]
fn a_define_with_no_source_of_truth_and_one_with_two_are_both_refused() {
    for defines in [
        json!([{ "name": "X" }]),
        json!([{ "name": "X", "from": "a", "literal": 1 }]),
    ] {
        let model = sheet(json!([{ "name": "a", "value": 1 }]), defines.clone());
        assert!(
            matches!(
                compile(&model, "general-simulator"),
                Err(CompileError::BadCodeBuild { .. })
            ),
            "{defines} should be refused"
        );
    }
}

#[test]
fn a_literal_is_the_one_way_to_write_a_constant_out() {
    let model = sheet(json!([]), json!([{ "name": "N_PHASES", "literal": 3 }]));
    let out = compile(&model, "general-simulator").expect("must compile");
    assert_eq!(
        out.code_build.unwrap().steps[0].defines,
        vec!["N_PHASES=3".to_string()]
    );
}

#[test]
fn a_dotted_from_is_refused_rather_than_read_as_a_name_with_a_dot_in_it() {
    let model = sheet(json!([]), json!([{ "name": "L_F", "from": "L1.L" }]));
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    assert!(err.to_string().contains("reserved"), "{err}");
}

#[test]
fn a_define_name_that_is_not_an_identifier_is_refused() {
    let model = sheet(json!([]), json!([{ "name": "-o /tmp/x", "literal": 1 }]));
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn a_step_may_override_a_plan_level_define_but_a_list_may_not_repeat_itself() {
    // Override: one `-D`, the step's value, in the plan-level position.
    let mut model = sheet(
        json!([{ "name": "a", "value": 1 }, { "name": "b", "value": 2 }]),
        json!([{ "name": "K", "from": "a" }, { "name": "OTHER", "from": "b" }]),
    );
    model.code_build.as_mut().unwrap().steps[0].defines =
        serde_json::from_value(json!([{ "name": "K", "from": "b" }])).unwrap();
    let out = compile(&model, "general-simulator").expect("must compile");
    assert_eq!(
        out.code_build.unwrap().steps[0].defines,
        vec!["K=2".to_string(), "OTHER=2".to_string()]
    );

    // A list disagreeing with itself is refused.
    let model = sheet(
        json!([{ "name": "a", "value": 1 }, { "name": "b", "value": 2 }]),
        json!([{ "name": "K", "from": "a" }, { "name": "K", "from": "b" }]),
    );
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

// -- the structural refusals -----------------------------------------------------------------

#[test]
fn a_block_naming_a_step_that_does_not_exist_is_refused_and_lists_the_ones_that_do() {
    let mut model = sheet(json!([]), json!([]));
    model.blocks[1].build = Some("modulator".to_string());
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    let message = err.to_string();
    assert!(message.contains("modulator"), "{message}");
    assert!(message.contains("ctrl"), "{message}");
}

#[test]
fn a_block_naming_a_step_on_a_sheet_with_no_plan_is_refused() {
    let mut model = sheet(json!([]), json!([]));
    model.code_build = None;
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn a_step_kind_this_version_cannot_build_is_refused_rather_than_skipped() {
    let mut model = sheet(json!([]), json!([]));
    model.code_build.as_mut().unwrap().steps[0].kind = "host-program".to_string();
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    let message = err.to_string();
    assert!(message.contains("host-program"), "{message}");
    assert!(message.contains("skipping it"), "{message}");
}

#[test]
fn a_step_that_does_not_say_which_language_it_is_is_refused() {
    let mut model = sheet(json!([]), json!([]));
    model.code_build.as_mut().unwrap().steps[0].lang = None;
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn a_step_with_no_sources_is_refused() {
    let mut model = sheet(json!([]), json!([]));
    model.code_build.as_mut().unwrap().steps[0].sources.clear();
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn a_step_nothing_builds_for_is_refused_rather_than_ignored() {
    let mut model = sheet(json!([]), json!([]));
    model.blocks[1].build = None;
    model.blocks[1].source = None;
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn two_steps_claiming_one_id_are_refused() {
    let mut model = sheet(json!([]), json!([]));
    let plan = model.code_build.as_mut().unwrap();
    let twin = plan.steps[0].clone();
    plan.steps.push(twin);
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn steps_that_need_each_other_in_a_circle_are_refused_and_named() {
    let mut model = sheet(json!([]), json!([]));
    let plan = model.code_build.as_mut().unwrap();
    let mut other = plan.steps[0].clone();
    other.id = "other".to_string();
    other.needs = vec!["ctrl".to_string()];
    plan.steps[0].needs = vec!["other".to_string()];
    plan.steps.push(other);
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    let message = err.to_string();
    assert!(message.contains("circle"), "{message}");
    assert!(
        message.contains("ctrl") && message.contains("other"),
        "{message}"
    );
}

#[test]
fn a_need_naming_no_step_is_refused() {
    let mut model = sheet(json!([]), json!([]));
    model.code_build.as_mut().unwrap().steps[0].needs = vec!["nowhere".to_string()];
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

#[test]
fn a_block_whose_source_is_not_one_of_its_steps_sources_is_refused() {
    let mut model = sheet(json!([]), json!([]));
    model.blocks[1].source = Some("code/something_else.c".to_string());
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    assert!(err.to_string().contains("code/something_else.c"), "{err}");
}

#[test]
fn a_build_on_a_block_that_is_not_compiled_is_refused() {
    let mut model = sheet(json!([]), json!([]));
    model.blocks[0].build = Some("ctrl".to_string());
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    assert!(err.to_string().contains("source.const"), "{err}");
}

#[test]
fn an_absolute_path_and_one_that_climbs_out_of_the_sheet_are_both_refused() {
    for path in ["/opt/shared/law.c", "../shared/law.c", "C:\\shared\\law.c"] {
        let mut model = sheet(json!([]), json!([]));
        model.code_build.as_mut().unwrap().steps[0].sources =
            vec!["code/cscript_ctrl.c".to_string(), path.to_string()];
        assert!(
            matches!(
                compile(&model, "general-simulator"),
                Err(CompileError::BadCodeBuild { .. })
            ),
            "{path} should be refused"
        );
    }
}

#[test]
fn a_block_naming_a_step_with_no_lib_has_nowhere_to_put_the_artifact() {
    let mut model = sheet(json!([]), json!([]));
    model.blocks[1].params.remove("lib");
    assert!(matches!(
        compile(&model, "general-simulator"),
        Err(CompileError::BadCodeBuild { .. })
    ));
}

// -- the sharing case the record was written for ---------------------------------------------

/// Five blocks, two steps, two shared sources, one set of defines. The point is what is *not*
/// repeated: the include directories and the circuit's numbers appear once for the sheet, and
/// each step names the shared sources it links rather than every block naming them.
#[test]
fn five_blocks_sharing_two_sources_say_the_defines_once() {
    let doc = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "params": [
          { "name": "l_f", "value": 2.8e-6 },
          { "name": "fsw", "value": 250000.0 }
        ],
        "code_build": {
          "includes": ["code", "code/generated"],
          "defines": [
            { "name": "L_F", "from": "l_f" },
            { "name": "FSW", "from": "fsw" }
          ],
          "libs": ["m"],
          "steps": [
            { "id": "law", "kind": "shared-library", "lang": "c",
              "sources": ["code/cscript_law.c", "code/control_law.c", "code/clarke.c"] },
            { "id": "mod", "kind": "shared-library", "lang": "c",
              "sources": ["code/cscript_mod.c", "code/control_law.c", "code/clarke.c"],
              "flags": ["-ffast-math"] }
          ]
        },
        "blocks": [
          { "id": "SRC", "type": "source.const", "paradigm": "signal-flow",
            "domain": "signal", "position": [0, 0], "params": { "value": 1.0 } },
          { "id": "A", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 0],
            "params": { "lib": "build/law", "input_names": ["e"], "output_names": ["d"] },
            "build": "law" },
          { "id": "B", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 60],
            "params": { "lib": "build/law", "input_names": ["e"], "output_names": ["d"] },
            "build": "law" },
          { "id": "C", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 120],
            "params": { "lib": "build/mod", "input_names": ["e"], "output_names": ["d"] },
            "build": "mod" },
          { "id": "D", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 180],
            "params": { "lib": "build/mod", "input_names": ["e"], "output_names": ["d"] },
            "build": "mod" },
          { "id": "E", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 240],
            "params": { "lib": "build/mod", "input_names": ["e"], "output_names": ["d"] },
            "build": "mod" }
        ],
        "wires": [
          { "id": "w1", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" }, "to": { "block": "A", "port": "in0" } },
          { "id": "w2", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" }, "to": { "block": "B", "port": "in0" } },
          { "id": "w3", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" }, "to": { "block": "C", "port": "in0" } },
          { "id": "w4", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" }, "to": { "block": "D", "port": "in0" } },
          { "id": "w5", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" }, "to": { "block": "E", "port": "in0" } }
        ]
      }
    });
    let model = GsimSchDocument::from_json(&doc.to_string())
        .expect("fixture parses")
        .model;
    let out = compile(&model, "general-simulator").expect("must compile");
    let build = out.code_build.expect("resolves");
    assert_eq!(build.steps.len(), 2);

    let law = &build.steps[0];
    let modulator = &build.steps[1];
    assert_eq!(law.blocks, vec!["A".to_string(), "B".to_string()]);
    assert_eq!(
        modulator.blocks,
        vec!["C".to_string(), "D".to_string(), "E".to_string()]
    );
    // One set of defines, resolved once, identical on both steps -- which is the whole claim.
    let expected = vec!["L_F=2.8e-6".to_string(), "FSW=250000".to_string()];
    assert_eq!(law.defines, expected);
    assert_eq!(modulator.defines, expected);
    // A step adds to the plan's flags rather than replacing them.
    assert!(law.flags.is_empty());
    assert_eq!(modulator.flags, vec!["-ffast-math".to_string()]);
    assert_eq!(
        law.includes,
        vec!["code".to_string(), "code/generated".to_string()]
    );
}

#[test]
fn two_blocks_naming_one_step_must_agree_on_where_its_artifact_goes() {
    let mut model = sheet(json!([]), json!([]));
    let mut twin = model.blocks[1].clone();
    twin.id = "CTRL2".to_string();
    twin.params.insert("lib".into(), json!("runs/other/ctrl"));
    model.blocks.push(twin);
    let err = compile(&model, "general-simulator").expect_err("must refuse");
    let message = err.to_string();
    assert!(message.contains("runs/case3/ctrl"), "{message}");
    assert!(message.contains("runs/other/ctrl"), "{message}");
}

// -- what a document written before this feature does ---------------------------------------

#[test]
fn a_sheet_with_no_plan_compiles_exactly_as_it_did_and_resolves_no_build() {
    let doc = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10,
        "blocks": [
          { "id": "SRC", "type": "source.const", "paradigm": "signal-flow",
            "domain": "signal", "position": [0, 0], "params": { "value": 1.0 } },
          { "id": "CTRL", "type": "extensibility.cscript", "paradigm": "signal-flow",
            "domain": "signal", "position": [100, 0],
            "params": { "lib": "./ctrl.so", "input_names": ["e"], "output_names": ["d"] } }
        ],
        "wires": [
          { "id": "w1", "kind": "signal", "net": "",
            "from": { "block": "SRC", "port": "out" },
            "to": { "block": "CTRL", "port": "in0" } }
        ]
      }
    });
    let text = doc.to_string();
    let parsed = GsimSchDocument::from_json(&text).expect("parses");
    // Nothing new appears in a document that did not ask for it.
    let saved = parsed.to_json_pretty().unwrap();
    assert!(!saved.contains("code_build"), "{saved}");
    assert!(!saved.contains("\"source\""), "{saved}");
    assert!(!saved.contains("\"build\""), "{saved}");

    let out = compile(&parsed.model, "general-simulator").expect("must compile");
    assert!(out.code_build.is_none());
    assert!(
        out.netlist.contains("kind=cscript lib=./ctrl.so"),
        "{}",
        out.netlist
    );
}

/// Nothing from the description reaches the card. `general-mna`'s `build_kind` ends with
/// `fields.unread()`, so an unknown `key=value` on a `kind=cscript` line is a hard deck error
/// with a test of its own over there — which makes this assertion a contract between two
/// repositories rather than tidiness (`docs/decisions/0009` §5).
#[test]
fn a_cscript_card_carries_nothing_the_build_description_added() {
    let model = sheet(json!([{ "name": "L_f", "value": 2.8e-6 }]), one_define());
    let out = compile(&model, "general-simulator").expect("must compile");
    let card = out
        .netlist
        .lines()
        .find(|line| line.contains("kind=cscript"))
        .expect("a cscript card");
    for forbidden in [
        "build=",
        "source=",
        "defines=",
        "includes=",
        "sources=",
        "lang=",
        "L_F",
    ] {
        assert!(
            !card.contains(forbidden),
            "'{forbidden}' leaked onto: {card}"
        );
    }
}

/// A plan survives a save even when the compiler refuses it — 0009 §10's two obligations, which
/// are discharged in two different modules and must not be confused for one.
#[test]
fn a_plan_the_compiler_refuses_still_round_trips_without_losing_a_key() {
    let text = json!({
      "schema_version": 2,
      "model": {
        "id": "top", "name": "Top", "grid": 10, "blocks": [], "wires": [],
        "code_build": {
          "steps": [{ "id": "table", "kind": "host-program",
                      "sources": ["code/gen_table.c"],
                      "run": { "stdout": "code/generated/table.h" } }]
        }
      }
    })
    .to_string();
    let parsed = GsimSchDocument::from_json(&text).expect("parses");
    let saved: serde_json::Value = serde_json::from_str(&parsed.to_json_pretty().unwrap()).unwrap();
    let step = &saved["model"]["code_build"]["steps"][0];
    assert_eq!(step["kind"], "host-program");
    assert_eq!(step["run"]["stdout"], "code/generated/table.h");
}
