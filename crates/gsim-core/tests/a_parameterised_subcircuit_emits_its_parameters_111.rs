//! A definition's formal parameters and an instance's values reach the deck (#111).
//!
//! The document has carried both halves since the subcircuit machinery was written —
//! `SubcktDef::params` and `SubcktInstance::params`/the instance block's `params` — and the
//! emitter read neither. So a `.subckt` card named its ports and stopped, an `X` card named its
//! connections and the definition and stopped, and **two instances of one definition compiled
//! byte-identically**, with `notes: []`. Nothing said a word about the values that had been
//! dropped.
//!
//! This was not fixed earlier on purpose. `general-mna` parsed both halves and bound neither
//! (`general-mna#63`), so a deck carrying `Rv=100` into that solver would have parsed cleanly
//! and lost the value one repository further down — worse than refusing, because it looks
//! correct. Decision 0010's amendment made that ordering a rule: *this crate must not emit a
//! parameter it cannot show the solver binds.* The solver side shipped in
//! `general-simulator 0.11.0`, which is what unblocked this.
//!
//! `examples/parameterised_rpart.py` is the end-to-end half: it compiles this same circuit and,
//! with `GSIM_SIMULATOR` set, runs the deck and asserts 5/320 A and a mid-node at 5 × 220/320 —
//! and asserts that the all-defaults numbers are *not* what came out. What is here is the deck.

use gsim_core::compile::compile;
use gsim_core::document::{
    ConnectOptions, Endpoint, ModelBuilder, ModelParam, NewBlockSpec, NewSubcktInstanceSpec,
    Paradigm, SubcktDef, SubcktInstance, SubcktPort,
};
use serde_json::{json, Map, Value};

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
        Paradigm::SpiceSubckt => "mixed",
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

fn elec(builder: &mut ModelBuilder, a: (&str, &str), b: (&str, &str), net: Option<&str>) {
    let mut options = ConnectOptions::default();
    if let Some(net) = net {
        options.net = Some(net.to_string());
    }
    builder
        .connect(
            Endpoint::new(a.0, a.1),
            Endpoint::new(b.0, b.1),
            "physical-electrical",
            options,
        )
        .unwrap_or_else(|e| panic!("{}.{} -- {}.{}: {e}", a.0, a.1, b.0, b.1));
}

/// `RPART`: one resistor valued `value`, with `formals` declared on the definition.
fn rpart_valued(formals: Vec<ModelParam>, value: Value) -> SubcktDef {
    let mut def = SubcktDef::new("rpart", "RPART").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    def.params = formals;
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
        "R",
        "passive.resistor",
        Paradigm::PhysicalNetwork,
        json!({ "R": value }),
    );
    elec(&mut body, ("rpart", "p"), ("R", "p"), None);
    elec(&mut body, ("R", "n"), ("rpart", "n"), None);
    let m = body.into_model();
    def.blocks = m.blocks;
    def.wires = m.wires;
    def
}

/// `Vin -- X1 -- X2 -- gnd`, each instance given `params`.
fn two_instances(
    formals: Vec<ModelParam>,
    x1: Value,
    x2: Value,
) -> Result<String, gsim_core::compile::CompileError> {
    // The body names the definition's first formal when there is one, and is an ordinary
    // number when there is not -- a body naming a parameter nothing declares is refused by
    // §5.3 (#121), which is a different rule and has its own test below.
    let body = match formals.first() {
        Some(first) => json!(format!("{{{}}}", first.name)),
        None => json!(1000.0),
    };
    two_instances_valued(formals, body, x1, x2)
}

fn two_instances_valued(
    formals: Vec<ModelParam>,
    body: Value,
    x1: Value,
    x2: Value,
) -> Result<String, gsim_core::compile::CompileError> {
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_subckt_def(rpart_valued(formals, body)).unwrap();
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        json!({}),
    );
    add(
        &mut b,
        "Vin",
        "source.voltage.dc",
        Paradigm::PhysicalNetwork,
        json!({ "V": 5.0 }),
    );
    for (id, params) in [("X1", x1), ("X2", x2)] {
        b.instantiate_subckt(NewSubcktInstanceSpec {
            params: params.as_object().cloned().unwrap_or_default(),
            ..NewSubcktInstanceSpec::new("rpart", (0, 0)).with_id(id)
        })
        .unwrap();
    }
    elec(&mut b, ("Vin", "p"), ("X1", "p"), Some("VIN"));
    elec(&mut b, ("X1", "n"), ("X2", "p"), Some("MID"));
    elec(&mut b, ("X2", "n"), ("gnd", "p"), None);
    elec(&mut b, ("Vin", "n"), ("gnd", "p"), None);
    compile(&b.into_model(), "general-simulator").map(|out| out.netlist)
}

/// Every `X` card in a deck, in order.
fn x_cards(netlist: &str) -> Vec<&str> {
    netlist
        .lines()
        .filter(|line| line.starts_with('X'))
        .collect()
}

#[test]
fn the_definitions_formal_list_follows_its_ports_on_the_subckt_card() {
    let netlist = two_instances(
        vec![formal("Rv", json!(1000.0))],
        json!({ "Rv": 100.0 }),
        json!({ "Rv": 220.0 }),
    )
    .expect("the sheet compiles");
    assert!(
        netlist.contains(".subckt RPART p n Rv=1000"),
        "the ports come first, then the formals: {netlist}"
    );
    assert!(netlist.contains(".ends RPART"), "{netlist}");
}

#[test]
fn each_instance_carries_its_own_value_so_two_are_no_longer_identical() {
    // The headline, and the shape of the defect: before this, these two cards differed only
    // by their name and their nets, and the values the document held were emitted nowhere.
    let netlist = two_instances(
        vec![formal("Rv", json!(1000.0))],
        json!({ "Rv": 100.0 }),
        json!({ "Rv": 220.0 }),
    )
    .expect("the sheet compiles");
    assert_eq!(
        x_cards(&netlist),
        vec!["X1 VIN MID RPART Rv=100", "X2 MID 0 RPART Rv=220"]
    );
}

#[test]
fn the_body_keeps_its_reference_for_the_solver_to_bind() {
    // `{Rv}` is the definition's own parameter, so this crate leaves it alone (#121) and the
    // solver binds it per instance. A deck where the body had been resolved to a number would
    // mean one resistance for both instances, which is the thing the formals exist to avoid.
    let netlist = two_instances(
        vec![formal("Rv", json!(1000.0))],
        json!({ "Rv": 100.0 }),
        json!({ "Rv": 220.0 }),
    )
    .expect("the sheet compiles");
    let body = netlist
        .split(".subckt RPART p n Rv=1000")
        .nth(1)
        .unwrap()
        .split(".ends RPART")
        .next()
        .unwrap();
    assert!(body.contains("R p n {Rv}"), "{netlist}");
}

#[test]
fn an_instance_that_overrides_nothing_leaves_its_card_bare() {
    // The default is on the `.subckt` card already. Repeating it on every call site would
    // make a deck where the declared default and the emitted one can silently disagree.
    let netlist = two_instances(
        vec![formal("Rv", json!(1000.0))],
        json!({}),
        json!({ "Rv": 220.0 }),
    )
    .expect("the sheet compiles");
    assert_eq!(
        x_cards(&netlist),
        vec!["X1 VIN MID RPART", "X2 MID 0 RPART Rv=220"]
    );
}

#[test]
fn a_definition_with_no_parameters_emits_exactly_what_it_always_did() {
    // The regression guard for every sheet that existed before this: no formals declared, no
    // tail emitted, the cards byte-identical to the ones already committed under `examples/`.
    let netlist = two_instances(vec![], json!({}), json!({})).expect("the sheet compiles");
    assert!(netlist.contains(".subckt RPART p n\n"), "{netlist}");
    assert!(netlist.contains("R p n 1000"), "{netlist}");
    assert_eq!(
        x_cards(&netlist),
        vec!["X1 VIN MID RPART", "X2 MID 0 RPART"]
    );
}

#[test]
fn the_declaration_sets_the_order_rather_than_the_instance() {
    // Three formals, and the instance's map is sorted by key, so "the order they were written"
    // is not available to preserve. Walking the declaration gives a deck that does not
    // reshuffle itself when a parameter is renamed -- and it is what makes the refusal below
    // free, since anything left over is a name the definition does not have.
    let netlist = two_instances(
        vec![
            formal("Rv", json!(1.0)),
            formal("alpha", json!(2.0)),
            formal("Beta", json!(3.0)),
        ],
        json!({ "Beta": 30.0, "Rv": 10.0, "alpha": 20.0 }),
        json!({}),
    )
    .expect("the sheet compiles");
    assert!(
        netlist.contains(".subckt RPART p n Rv=1 alpha=2 Beta=3"),
        "{netlist}"
    );
    assert_eq!(
        x_cards(&netlist)[0],
        "X1 VIN MID RPART Rv=10 alpha=20 Beta=30"
    );
}

#[test]
fn a_parameter_the_definition_does_not_declare_is_refused_and_names_both() {
    // Dropping it would solve 1000 Ω while the document says 100, and a mistyped parameter
    // name is exactly how that happens. The same refusal exists on the solver side; having it
    // here means the message can name the block as the document names it, before any `X` card
    // exists to point at.
    let said = two_instances(
        vec![formal("Rv", json!(1000.0))],
        json!({ "Rw": 100.0 }),
        json!({}),
    )
    .expect_err("an undeclared parameter is refused")
    .to_string();
    assert!(said.contains("X1"), "{said}");
    assert!(said.contains("Rw"), "{said}");
    assert!(said.contains("RPART"), "{said}");
    assert!(
        said.contains("Rv"),
        "it must say what the instance could have meant: {said}"
    );
}

#[test]
fn and_says_so_plainly_when_the_definition_declares_none() {
    let said = two_instances(vec![], json!({ "Rv": 100.0 }), json!({}))
        .expect_err("an undeclared parameter is refused")
        .to_string();
    assert!(said.contains("no parameters"), "{said}");
}

#[test]
fn a_nested_instance_carries_its_tail_too() {
    // The second spelling of an `X` card: a `SubcktInstance` inside a definition's body, whose
    // connections are already names in that body's namespace. It goes through the same
    // emission, so a nested instance is parameterised exactly as a top-level one is -- and
    // this is the shape the composite component needs, since a composite's body holds the
    // instances whose values it promotes.
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_subckt_def(rpart_valued(
        vec![formal("Rv", json!(1000.0))],
        json!("{Rv}"),
    ))
    .unwrap();

    let mut outer = SubcktDef::new("outer", "OUTER").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    let mut nested = SubcktInstance::new("Xi", "rpart").with_connections(["p", "n"]);
    nested.params.insert("Rv".to_string(), json!(47.0));
    outer.instances = vec![nested];
    b.add_subckt_def(outer).unwrap();

    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        json!({}),
    );
    b.instantiate_subckt(NewSubcktInstanceSpec::new("outer", (0, 0)).with_id("X1"))
        .unwrap();
    elec(&mut b, ("X1", "p"), ("gnd", "p"), None);
    elec(&mut b, ("X1", "n"), ("gnd", "p"), None);

    let netlist = compile(&b.into_model(), "general-simulator")
        .expect("the sheet compiles")
        .netlist;
    let outer_body = netlist
        .split(".subckt OUTER p n")
        .nth(1)
        .unwrap()
        .split(".ends OUTER")
        .next()
        .unwrap();
    assert!(
        outer_body.contains("Xi p n RPART Rv=47"),
        "the nested card must carry its own value: {netlist}"
    );
}

#[test]
fn a_nested_instances_undeclared_parameter_is_refused_as_well() {
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_subckt_def(rpart_valued(
        vec![formal("Rv", json!(1000.0))],
        json!("{Rv}"),
    ))
    .unwrap();
    let mut outer = SubcktDef::new("outer", "OUTER").unwrap().with_ports([
        SubcktPort::new("p", "physical-electrical").unwrap(),
        SubcktPort::new("n", "physical-electrical").unwrap(),
    ]);
    let mut nested = SubcktInstance::new("Xi", "rpart").with_connections(["p", "n"]);
    nested.params.insert("Rw".to_string(), json!(47.0));
    outer.instances = vec![nested];
    b.add_subckt_def(outer).unwrap();
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        json!({}),
    );
    b.instantiate_subckt(NewSubcktInstanceSpec::new("outer", (0, 0)).with_id("X1"))
        .unwrap();
    elec(&mut b, ("X1", "p"), ("gnd", "p"), None);
    elec(&mut b, ("X1", "n"), ("gnd", "p"), None);
    let said = compile(&b.into_model(), "general-simulator")
        .expect_err("an undeclared parameter is refused")
        .to_string();
    assert!(said.contains("Xi"), "{said}");
    assert!(said.contains("Rw"), "{said}");
}

#[test]
fn a_value_written_as_a_sheet_reference_reaches_the_card_as_a_number() {
    // The two mechanisms meeting: `Rv` is bound by the solver, and `{Rload}` is resolved here
    // (§5.3, #121). An instance's value is written on the sheet, so it is a sheet reference --
    // and what the deck carries is the number, because a solver given `{Rload}` on an `X` card
    // would have nothing to resolve it against.
    let mut b = ModelBuilder::new("m", "M", 10);
    b.add_model_param("Rload", json!(330.0)).unwrap();
    b.add_subckt_def(rpart_valued(
        vec![formal("Rv", json!(1000.0))],
        json!("{Rv}"),
    ))
    .unwrap();
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        json!({}),
    );
    b.instantiate_subckt(NewSubcktInstanceSpec {
        params: json!({ "Rv": "{Rload * 2}" }).as_object().cloned().unwrap(),
        ..NewSubcktInstanceSpec::new("rpart", (0, 0)).with_id("X1")
    })
    .unwrap();
    elec(&mut b, ("X1", "p"), ("gnd", "p"), None);
    elec(&mut b, ("X1", "n"), ("gnd", "p"), None);
    let netlist = compile(&b.into_model(), "general-simulator")
        .expect("the sheet compiles")
        .netlist;
    assert_eq!(x_cards(&netlist), vec!["X1 0 0 RPART Rv=660"]);
}

#[test]
fn a_body_naming_a_parameter_nothing_declares_is_still_refused() {
    // The neighbouring rule, so the two are not confused: `{Rv}` in a body is left for the
    // solver *because the definition declares `Rv`*. Declare nothing and the same text is an
    // ordinary §5.3 reference to a sheet parameter that does not exist, and #121 refuses it
    // rather than emitting a symbol into the deck.
    let said = two_instances_valued(vec![], json!("{Rv}"), json!({}), json!({}))
        .expect_err("a reference to nothing is refused")
        .to_string();
    assert!(said.contains("block 'R' parameter 'R'"), "{said}");
    assert!(said.contains("Rv"), "{said}");
}

#[test]
fn nothing_here_passed_against_a_deck_with_no_subcircuit_in_it() {
    // The guard: every assertion above reads a `.subckt` card or an `X` card out of a compiled
    // deck, so a definition that vanished would make them fail rather than pass. This states
    // the shape of the whole deck once, so a reader can see there is a definition, a body, two
    // instances and a source.
    let netlist = two_instances(
        vec![formal("Rv", json!(1000.0))],
        json!({ "Rv": 100.0 }),
        json!({ "Rv": 220.0 }),
    )
    .expect("the sheet compiles");
    let cards: Vec<&str> = netlist
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('*'))
        .collect();
    assert_eq!(
        cards,
        vec![
            ".subckt RPART p n Rv=1000",
            "R p n {Rv}",
            ".ends RPART",
            "Vin VIN 0 5",
            "X1 VIN MID RPART Rv=100",
            "X2 MID 0 RPART Rv=220",
        ],
        "{netlist}"
    );
}
