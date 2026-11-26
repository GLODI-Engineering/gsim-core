//! A wire into a port the library calls causal is a block reference, whatever the wire says.
//!
//! # The number this protects
//!
//! #80 fixed the half where the *wire's own* `kind` said `physical-signal` and the predicate
//! misread it. This is the other half: `docs/specs/gsim-sch-format.md` §8.4 makes `wire.kind` a
//! **duplicate** of the endpoints' port kind, so that schema validation never needs the
//! component catalogue — and a duplicate can disagree with the thing it duplicates.
//!
//! It does, in this repository's own `examples/buck_state_space_pi.py`: the gate-drive wire from
//! the `converter.sig2phys` output to the switch's `g` pin is spelled `physical-electrical`,
//! while the library declares that pin `signal`. So a net was invented for a causal reference,
//! named `n_DSW_g`, and handed on as a circuit node.
//!
//! In that example the invented node reaches no card, because an ideal switch's node ports are
//! `d` and `s` and its gate travels as `ctrl=GATE`. Draw the ordinary next thing — a gate
//! resistor — and it does reach one. `general-mna`'s own `BlockKind::Sig2Phys` documentation
//! says what happens then:
//!
//! > a converter must always be *referenced by name* (a source's value field, or a switch's
//! > `gate=`/`ctrl=`), never wired. […] this block has no terminals and stamps nothing into the
//! > MNA system, so a net sharing its name is merely undriven, and KCL solves it to a silently
//! > wrong `0` rather than failing.
//!
//! A silently wrong `0`, from a sheet that drew a gate drive. Same failure class as #80, reached
//! from the other side, which is why #77 asks for the nature of a signal connection to be a fact
//! of the netlist rather than something inferred downstream.
//!
//! # What decides it now
//!
//! `compile::library::port_domain` — the one description that cannot drift, because it is the
//! description. `compile::build_scope` collects the endpoints it calls causal and
//! `nets::resolve_scope` makes no net for a wire touching one. An endpoint the library knows
//! nothing about still falls back to the wire's own `kind`, exactly as before.
//!
//! # What this does not fix, deliberately
//!
//! A `converter.sig2phys` output wired to an *acausal* port — a gate resistor's pin rather than
//! the switch's gate — is still emitted as a node, now named after the converter's own port:
//!
//! ```text
//! Rg n_GATE_out 0 10
//! GATE kind=sig2phys domain=voltage in=MOD
//! ```
//!
//! Nothing drives `n_GATE_out`, because a `kind=sig2phys` card declares no terminals. Both of
//! that wire's ends are declared `physical-electrical` by the library, so the rule here cannot
//! see it: what is wrong is the port's own declaration, and changing it decides how a converter
//! is *drawn*. That is a construct the netlist grammar cannot express, which is what #77's third
//! ask collects rather than works around; it is reported there and tracked separately.

use gsim_core::compile::{compile, CompileOutput};
use gsim_core::document::{ConnectOptions, Endpoint, Model, ModelBuilder, NewBlockSpec, Paradigm};
use serde_json::{json, Map, Value};

fn params(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

fn add(
    b: &mut ModelBuilder,
    id: &str,
    r#type: &str,
    paradigm: Paradigm,
    domain: &str,
    p: Map<String, Value>,
) {
    b.add_block(NewBlockSpec {
        r#type: r#type.to_string(),
        paradigm,
        domain: domain.to_string(),
        id: Some(id.to_string()),
        params: p,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("adding {id}: {e}"));
}

fn switch_params() -> Map<String, Value> {
    params(&[
        ("r_on", json!(0.01)),
        ("body_g_breakdown", json!(0.0)),
        ("body_v_breakdown", json!(-500.0)),
        ("body_g_off", json!(1e-9)),
        ("body_v_f", json!(0.5)),
        ("body_g_on", json!(200.0)),
        ("v_gs_th", json!(0.5)),
    ])
}

/// The gate-drive crossing exactly as `examples/buck_state_space_pi.py` draws it: a constant
/// through a `sig2phys`, then a wire spelled `physical-electrical` onto the switch's `g` pin,
/// which the library declares `signal`.
///
/// A real circuit hangs off it so the netlist is non-trivial and no assertion below can be met
/// by the document simply failing to compile.
fn sheet_whose_gate_wire_is_spelled_physical() -> Model {
    let mut b = ModelBuilder::new("m", "A gate pin is not a node", 10);

    add(
        &mut b,
        "Vdc",
        "source.voltage.dc",
        Paradigm::PhysicalNetwork,
        "electrical",
        params(&[("V", json!(24))]),
    );
    add(
        &mut b,
        "DSW",
        "ideal.switch",
        Paradigm::PhysicalNetwork,
        "electrical",
        switch_params(),
    );
    add(
        &mut b,
        "Rload",
        "passive.resistor",
        Paradigm::PhysicalNetwork,
        "electrical",
        params(&[("R", json!(5))]),
    );
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        "electrical",
        Map::new(),
    );
    for (from, to) in [
        (("Vdc", "p"), ("DSW", "d")),
        (("DSW", "s"), ("Rload", "p")),
        (("Rload", "n"), ("gnd", "gnd")),
        (("Vdc", "n"), ("gnd", "gnd")),
    ] {
        b.connect(
            Endpoint::new(from.0, from.1),
            Endpoint::new(to.0, to.1),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{from:?} -> {to:?}: {e}"));
    }

    add(
        &mut b,
        "MOD",
        "source.const",
        Paradigm::SignalFlow,
        "signal",
        params(&[("value", json!(1.0))]),
    );
    add(
        &mut b,
        "GATE",
        "converter.sig2phys",
        Paradigm::Converter,
        "electrical",
        params(&[("domain", json!("voltage"))]),
    );
    b.connect(
        Endpoint::new("MOD", "out"),
        Endpoint::new("GATE", "in"),
        "signal",
        ConnectOptions::default(),
    )
    .unwrap();
    // The wire under test, spelled as the committed example spells it.
    b.connect(
        Endpoint::new("GATE", "out"),
        Endpoint::new("DSW", "g"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .expect("the committed buck example draws exactly this, so it has to stay drawable");

    b.into_model()
}

fn compiled(model: &Model) -> CompileOutput {
    compile(model, "general-simulator").unwrap_or_else(|e| panic!("compile: {e}"))
}

/// Every net the provenance map puts `(block, port)` on.
fn nets_of(out: &CompileOutput, block: &str, port: &str) -> Vec<String> {
    out.provenance
        .nets
        .iter()
        .filter(|net| {
            net.ports
                .iter()
                .any(|p| p.blocks.last().map(String::as_str) == Some(block) && p.port == port)
        })
        .map(|net| net.net.clone())
        .collect()
}

#[test]
fn a_gate_pin_the_library_calls_signal_sits_on_no_net() {
    let out = compiled(&sheet_whose_gate_wire_is_spelled_physical());
    let on = nets_of(&out, "DSW", "g");
    assert!(
        on.is_empty(),
        "DSW.g is declared `signal` by the library, and the compiler put it on {on:?} because \
         the wire drawn into it says `physical-electrical`. A node invented for a causal \
         reference is reported by the solver as a voltage (#80, #77)"
    );
    // The other end of that same wire. This holds *because* the wire is filtered whole, not
    // because the converter's output port is understood -- see the module doc's "what this does
    // not fix": wire it to an acausal port instead and a node is still invented.
    let converter = nets_of(&out, "GATE", "out");
    assert!(
        converter.is_empty(),
        "GATE.out sits on {converter:?}; a `kind=sig2phys` card declares no node, so a net \
         there is undriven and KCL solves it to a silently wrong 0"
    );
    assert!(
        !out.netlist.contains("n_DSW_g") && !out.netlist.contains("n_GATE_out"),
        "the invented node reached the deck:\n{}",
        out.netlist
    );
}

#[test]
fn the_gate_still_travels_as_the_converter_block_name() {
    // The control, so the assertions above cannot be satisfied by a compiler that has stopped
    // connecting the gate at all. What crosses the boundary is a name, which is the whole of
    // #77's first ask.
    let out = compiled(&sheet_whose_gate_wire_is_spelled_physical());
    assert!(
        out.netlist.contains("gate=block ctrl=GATE"),
        "the switch no longer reads its gate from the converter:\n{}",
        out.netlist
    );
    assert!(
        out.netlist
            .contains("GATE kind=sig2phys domain=voltage in=MOD"),
        "the crossing is not emitted as a crossing:\n{}",
        out.netlist
    );
}

#[test]
fn the_real_circuit_still_compiles_to_real_nodes() {
    // The second control. Three physical nets and the reference, all still there.
    let out = compiled(&sheet_whose_gate_wire_is_spelled_physical());
    assert_eq!(nets_of(&out, "Vdc", "p"), nets_of(&out, "DSW", "d"));
    assert_eq!(nets_of(&out, "DSW", "s"), nets_of(&out, "Rload", "p"));
    assert_eq!(nets_of(&out, "Rload", "n"), vec!["0".to_string()]);
    assert!(
        out.netlist.lines().any(|l| l.starts_with("Rload ")),
        "{}",
        out.netlist
    );
}

#[test]
fn no_element_card_carries_a_node_that_came_from_a_signal_port() {
    // #77's "done when", asked of the netlist rather than of the predicate: for a sheet holding
    // both kinds of wire, every node name on an element card belongs to a net made only of
    // acausal ports. A name that reached a card from a signal port is the thing the solver then
    // labels `quantity=voltage`.
    let model = sheet_whose_gate_wire_is_spelled_physical();
    let out = compiled(&model);

    let signal_ports: Vec<(&str, &str)> = vec![("DSW", "g"), ("GATE", "out"), ("MOD", "out")];
    for net in &out.provenance.nets {
        for port in &net.ports {
            let here = (
                port.blocks.last().map(String::as_str).unwrap_or_default(),
                port.port.as_str(),
            );
            assert!(
                !signal_ports.contains(&here),
                "net {:?} contains the causal port {}.{}",
                net.net,
                here.0,
                here.1
            );
        }
    }

    // And the signal chain is there, as `field=` references to block names.
    for reference in ["in=MOD", "ctrl=GATE"] {
        assert!(
            out.netlist.contains(reference),
            "{reference} missing from:\n{}",
            out.netlist
        );
    }
}

#[test]
fn a_wire_whose_endpoints_the_library_does_not_know_still_goes_by_its_own_kind() {
    // The fallback, held explicitly so the new rule cannot quietly become the only rule. A
    // `spice-subckt` instance's ports are declared by the document, not by the library, so
    // `port_domain` answers `None` for them and `wire.kind` decides as it always did.
    let mut b = ModelBuilder::new("m", "An unknown port keeps the wire's word", 10);
    add(
        &mut b,
        "R1",
        "passive.resistor",
        Paradigm::PhysicalNetwork,
        "electrical",
        params(&[("R", json!(100))]),
    );
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        "electrical",
        Map::new(),
    );
    add(
        &mut b,
        "Vdc",
        "source.voltage.dc",
        Paradigm::PhysicalNetwork,
        "electrical",
        params(&[("V", json!(1))]),
    );
    for (from, to) in [
        (("Vdc", "p"), ("R1", "p")),
        (("R1", "n"), ("gnd", "gnd")),
        (("Vdc", "n"), ("gnd", "gnd")),
    ] {
        b.connect(
            Endpoint::new(from.0, from.1),
            Endpoint::new(to.0, to.1),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap();
    }
    let out = compiled(&b.into_model());
    assert_eq!(nets_of(&out, "Vdc", "p"), nets_of(&out, "R1", "p"));
}
