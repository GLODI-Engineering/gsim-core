//! §5.3 reaches inside a `.subckt` definition, and stops at the definition's own parameters
//! (#121).
//!
//! `params::substitute` states an invariant in its own doc comment: *"after `substitute`, no
//! §5.1 reference and no §5.2 expression remains anywhere in the model. Everything downstream
//! -- element cards, `ic`, a build's `-D` bindings -- can then read a value without knowing
//! §5.2 exists, which is the whole point of resolving in one place."*
//!
//! It walked `params`, `blocks` and `simulation`, and never `subckt_defs`. So a resistor inside
//! a definition valued `"{Rload}"` compiled, without a note, to `Rin p n {Rload}` — a deck
//! naming a symbol nothing declares, because this crate resolves §5 at compile time and emits
//! numbers rather than `.param` cards. The same reference one line outside the definition
//! resolved to `330`.
//!
//! The reason this is more than the same loop repeated is that a definition brings a **second
//! scope**. A body's reference to one of the definition's own declared parameters is not this
//! crate's to resolve: the instance supplies it and the solver binds it where the subcircuit is
//! expanded (`general-mna#63`, shipped in `general-simulator 0.11.0`). Those names are left
//! exactly as written. Everything else resolves against the sheet's table.
//!
//! Which makes the interesting case the one that names both, and it is refused rather than
//! guessed: there is no single scope to read it in.

use gsim_core::compile::compile;
use gsim_core::document::{
    ConnectOptions, Endpoint, ModelBuilder, ModelParam, NewBlockSpec, NewSubcktInstanceSpec,
    Paradigm, SubcktDef, SubcktPort,
};
use serde_json::{json, Map, Value};

/// One declared parameter of a definition.
fn formal(name: &str, value: Value) -> ModelParam {
    ModelParam {
        name: name.to_string(),
        value,
        unit: None,
        doc: None,
        extra: Map::new(),
    }
}

fn add(builder: &mut ModelBuilder, id: &str, r#type: &str, paradigm: Paradigm, params: Value) {
    let domain = match paradigm {
        Paradigm::SignalFlow => "signal",
        _ => "physical-electrical",
    };
    builder
        .add_block(NewBlockSpec {
            id: Some(id.to_string()),
            r#type: r#type.to_string(),
            paradigm,
            domain: domain.to_string(),
            position: (0, 0),
            params: params.as_object().cloned().unwrap_or_default(),
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("{id} is placeable: {e}"));
}

fn elec(builder: &mut ModelBuilder, a: (&str, &str), b: (&str, &str)) {
    builder
        .connect(
            Endpoint::new(a.0, a.1),
            Endpoint::new(b.0, b.1),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{}.{} -- {}.{}: {e}", a.0, a.1, b.0, b.1));
}

/// A sheet with `Rload = 330`, one definition whose single resistor is valued `written`, and one
/// instance of it. Returns the compiled netlist, or the refusal.
fn netlist_with_body_resistor(written: Value, formals: Vec<ModelParam>) -> Result<String, String> {
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_model_param("Rload", json!(330.0)).unwrap();

    let mut def = SubcktDef::new("rpart", "RPART").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    def.params = formals;
    {
        let mut body = ModelBuilder::new("__rpart", "rpart", 10);
        add(
            &mut body,
            "rpart",
            "spice-subckt.boundary",
            Paradigm::SpiceSubckt,
            json!({}),
        );
        add(
            &mut body,
            "Rin",
            "passive.resistor",
            Paradigm::PhysicalNetwork,
            json!({ "R": written }),
        );
        elec(&mut body, ("rpart", "p"), ("Rin", "p"));
        elec(&mut body, ("Rin", "n"), ("rpart", "n"));
        let m = body.into_model();
        def.blocks = m.blocks;
        def.wires = m.wires;
    }
    b.add_subckt_def(def).unwrap();
    b.instantiate_subckt(NewSubcktInstanceSpec::new("rpart", (0, 0)).with_id("X1"))
        .unwrap();
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        json!({}),
    );
    elec(&mut b, ("X1", "p"), ("gnd", "p"));
    elec(&mut b, ("X1", "n"), ("gnd", "p"));

    compile(&b.into_model(), "general-simulator")
        .map(|out| out.netlist)
        .map_err(|e| e.to_string())
}

/// The `Rin` card out of a compiled deck.
fn resistor_card(netlist: &str) -> String {
    netlist
        .lines()
        .find(|line| line.starts_with("Rin "))
        .unwrap_or_else(|| panic!("no Rin card in\n{netlist}"))
        .to_string()
}

#[test]
fn a_reference_in_a_body_resolves_to_the_sheets_number() {
    let netlist = netlist_with_body_resistor(json!("Rload"), vec![]).expect("the sheet compiles");
    assert_eq!(resistor_card(&netlist), "Rin p n 330");
}

#[test]
fn an_expression_in_a_body_resolves_in_the_body_too() {
    let netlist =
        netlist_with_body_resistor(json!("{Rload * 2}"), vec![]).expect("the sheet compiles");
    assert_eq!(resistor_card(&netlist), "Rin p n 660");
}

#[test]
fn and_the_same_value_one_line_outside_a_definition_always_resolved() {
    // The asymmetry that made this a defect rather than a missing feature: a top-level block
    // has always had its references resolved, so a user moving a resistor into a subcircuit
    // watched a working value stop working.
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_model_param("Rload", json!(330.0)).unwrap();
    add(
        &mut b,
        "Rin",
        "passive.resistor",
        Paradigm::PhysicalNetwork,
        json!({ "R": "{Rload * 2}" }),
    );
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        json!({}),
    );
    elec(&mut b, ("Rin", "p"), ("gnd", "p"));
    elec(&mut b, ("Rin", "n"), ("gnd", "p"));
    let netlist = compile(&b.into_model(), "general-simulator")
        .expect("the sheet compiles")
        .netlist;
    // Both terminals are the reference node here, so only the value is the subject: `660`,
    // resolved, the way it has always been outside a definition.
    assert_eq!(resistor_card(&netlist), "Rin 0 0 660");
}

#[test]
fn a_reference_to_the_definitions_own_parameter_is_left_for_the_solver() {
    // Not resolved, and not refused either. `Rv` is the definition's own declared parameter:
    // the instance supplies it and the solver binds it when the subcircuit is expanded, so the
    // only correct thing this crate can do with `{Rv}` is leave it exactly as written.
    //
    // Before the shadowing rule this value took the other path and the compile *failed* --
    // `grammar::eval` has no `Rv`, so making substitution reach into bodies would have turned
    // every parameterised subcircuit into an unresolvable-reference error.
    let netlist = netlist_with_body_resistor(json!("{Rv}"), vec![formal("Rv", json!(1000.0))])
        .expect("a reference to the definition's own parameter is not an error");
    assert_eq!(resistor_card(&netlist), "Rin p n {Rv}");
}

#[test]
fn the_bare_identifier_form_of_the_same_reference_is_left_alone_as_well() {
    let netlist = netlist_with_body_resistor(json!("Rv"), vec![formal("Rv", json!(1000.0))])
        .expect("the sheet compiles");
    assert_eq!(resistor_card(&netlist), "Rin p n Rv");
}

#[test]
fn a_value_naming_both_scopes_is_refused_and_names_both() {
    // There is no one scope to read `{Rv * Rload}` in: `Rv` arrives from the instance at
    // expansion time and `Rload` is resolved here, before the deck exists. Resolving the half
    // this crate can would emit `{Rv * 330}` and leave the other half to a solver that would
    // bind `Rv` into it -- which works, by accident, and reads as if one mechanism handled
    // both. Refusing says which two scopes were named.
    let said = netlist_with_body_resistor(json!("{Rv * Rload}"), vec![formal("Rv", json!(1000.0))])
        .expect_err("a value naming both scopes is refused");
    assert!(said.contains("Rv"), "{said}");
    assert!(said.contains("Rload"), "{said}");
    assert!(said.contains("one scope"), "{said}");
    assert!(
        said.contains("subcircuit 'rpart' block 'Rin' parameter 'R'"),
        "the refusal must say where it was written: {said}"
    );
}

#[test]
fn a_definitions_declared_default_resolves_in_the_sheets_scope() {
    // A default is written in the document beside everything else, so a reference in one is a
    // sheet reference. Asserted on the substituted model rather than the deck, because #111 is
    // what puts a default on the `.subckt` card.
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_model_param("Rload", json!(330.0)).unwrap();
    let mut def = SubcktDef::new("rpart", "RPART").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    def.params = vec![formal("Rv", json!("{Rload}"))];
    b.add_subckt_def(def).unwrap();
    let model = b.into_model();

    let table = gsim_core::params::resolve_table(&model).expect("the table resolves");
    let substituted = gsim_core::params::substitute(&model, &table).expect("substitution");
    assert_eq!(substituted.subckt_defs[0].params[0].value, json!(330));
    // And the document itself still carries the expression, which is the whole point of
    // substituting into a clone.
    assert_eq!(model.subckt_defs[0].params[0].value, json!("{Rload}"));
}

#[test]
fn an_instance_parameter_resolves_in_the_scope_it_is_written_in() {
    // Two instances, at two depths. `X1`'s value is written on the sheet, so `{Rload}` is a
    // sheet reference and resolves. The nested instance's value is written inside `outer`'s
    // body, so a reference to `outer`'s own `Ro` is left for the solver -- the same rule as a
    // body block's, because a call site *is* part of the body it is written in.
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_model_param("Rload", json!(330.0)).unwrap();

    let inner = SubcktDef::new("inner", "INNER").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    let mut inner = inner;
    inner.params = vec![formal("Ri", json!(1.0))];
    b.add_subckt_def(inner).unwrap();

    let mut outer = SubcktDef::new("outer", "OUTER").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    outer.params = vec![formal("Ro", json!(2.0))];
    let mut nested =
        gsim_core::document::SubcktInstance::new("Xi", "inner").with_connections(["p", "n"]);
    nested.params.insert("Ri".to_string(), json!("{Ro}"));
    outer.instances = vec![nested];
    b.add_subckt_def(outer).unwrap();

    b.instantiate_subckt(NewSubcktInstanceSpec::new("outer", (0, 0)).with_id("X1"))
        .unwrap();
    let model = {
        let mut model = b.into_model();
        model.blocks[0]
            .params
            .insert("Ro".to_string(), json!("{Rload}"));
        model
    };

    let table = gsim_core::params::resolve_table(&model).expect("the table resolves");
    let substituted = gsim_core::params::substitute(&model, &table).expect("substitution");
    assert_eq!(substituted.blocks[0].params["Ro"], json!(330));
    let outer_def = substituted
        .subckt_defs
        .iter()
        .find(|d| d.id == "outer")
        .expect("outer survived");
    assert_eq!(outer_def.instances[0].params["Ri"], json!("{Ro}"));
}

#[test]
fn nothing_here_passed_because_the_body_was_empty() {
    // The guard this repository now carries on every rule: each assertion above reads one card
    // out of a compiled deck, so a definition that lost its body would make `resistor_card`
    // panic rather than pass. This states the other half -- that the deck really does contain
    // a `.subckt` with that resistor inside it, instantiated once.
    let netlist = netlist_with_body_resistor(json!("Rload"), vec![]).expect("the sheet compiles");
    assert!(netlist.contains(".subckt RPART p n"), "{netlist}");
    assert!(netlist.contains(".ends RPART"), "{netlist}");
    let body: Vec<&str> = netlist
        .split(".subckt RPART p n")
        .nth(1)
        .unwrap()
        .split(".ends RPART")
        .next()
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(body, vec!["Rin p n 330"], "{netlist}");
    assert!(netlist.contains("X1 "), "{netlist}");
}
