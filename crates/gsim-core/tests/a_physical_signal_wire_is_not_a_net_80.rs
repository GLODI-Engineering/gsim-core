//! A `physical-signal` wire is a causal block reference. It must never become a circuit node.
//!
//! # The number this protects
//!
//! `compile::nets` decided what a net was with `kind.starts_with("physical-")`, and
//! `"physical-signal"` starts with `"physical-"`. So a sensor-output to actuator-input
//! connection was union-found into a net, named `n_<block>_<port>`, and emitted as a node. The
//! solver then did exactly the right thing with the node it was given: it reported that node's
//! voltage, with `quantity=voltage` and unit `V`. A sensor reading of `0.42` arrived on a plot
//! as `0.42` volts.
//!
//! Nothing errored. The value was plausibly scaled, correctly labelled, and entirely fictional
//! — which is the hardest kind of wrong to notice, and the kind this repository's guards exist
//! for. See #80, and #77 for the failure class.
//!
//! # Why the existing guard did not see it
//!
//! `compile::nets`'s own `signal_wires_never_become_nets` passed the kind `"signal"` and
//! nothing else, so the one causal kind the predicate misclassified was the one kind the test
//! never tried. That test now sweeps every causal kind the format has; this file is the
//! end-to-end half — a real model, through `compile`, with the netlist and the provenance map
//! asserted rather than the predicate.
//!
//! # It was reachable, and reached
//!
//! `ModelBuilder::connect(..., "physical-signal")` accepts it (and `port_compat`'s own tests
//! do exactly that), and omni-schematics' `tests/fixtures/gsim_sch/comprehensive.gsim_sch`
//! carries such a wire between a `sig2phys` and a `phys2sig` converter. This is not a
//! hypothetical spelling.

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

/// A complete, compilable sheet whose one `physical-signal` wire joins the two converters —
/// the same shape omni-schematics' `comprehensive` fixture carries.
///
/// A real circuit hangs off it (a DC source and a resistor with a reference) so that the
/// netlist is non-trivial and the test cannot pass by the document failing to compile.
fn sheet_with_a_physical_signal_wire() -> Model {
    let mut b = ModelBuilder::new("m", "A causal wire is not a node", 10);

    add(
        &mut b,
        "vin",
        "source.voltage.dc",
        Paradigm::PhysicalNetwork,
        "electrical",
        params(&[("V", json!(5))]),
    );
    add(
        &mut b,
        "rload",
        "passive.resistor",
        Paradigm::PhysicalNetwork,
        "electrical",
        params(&[("R", json!(1000))]),
    );
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        "electrical",
        Map::new(),
    );
    b.connect(
        Endpoint::new("vin", "p"),
        Endpoint::new("rload", "p"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .unwrap();
    b.connect(
        Endpoint::new("vin", "n"),
        Endpoint::new("gnd", "gnd"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .unwrap();
    b.connect(
        Endpoint::new("rload", "n"),
        Endpoint::new("gnd", "gnd"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .unwrap();

    // The pair under test. `sig2phys` drives a node from a signal, `phys2sig` reads one back
    // out; the wire between them is causal, and what travels along it is a block *name*, not a
    // node.
    add(
        &mut b,
        "s2p",
        "converter.sig2phys",
        Paradigm::Converter,
        "electrical",
        params(&[("domain", json!("voltage"))]),
    );
    add(
        &mut b,
        "p2s",
        "converter.phys2sig",
        Paradigm::Converter,
        "signal",
        params(&[("node", json!("n_rload_p"))]),
    );
    b.connect(
        Endpoint::new("p2s", "out"),
        Endpoint::new("s2p", "in"),
        "physical-signal",
        ConnectOptions::default(),
    )
    .expect("`physical-signal` is a kind `connect` accepts, which is why #80 was reachable");

    b.into_model()
}

fn compiled() -> CompileOutput {
    let model = sheet_with_a_physical_signal_wire();
    compile(&model, "general-simulator").expect("the sheet compiles")
}

/// The endpoints of the causal wire, which must appear in no net whatsoever.
const CAUSAL_ENDPOINTS: [(&str, &str); 2] = [("p2s", "out"), ("s2p", "in")];

#[test]
fn the_causal_wire_puts_its_endpoints_on_no_net_at_all() {
    let out = compiled();
    for (block, port) in CAUSAL_ENDPOINTS {
        let net = out
            .provenance
            .nets
            .iter()
            .find(|entry| {
                entry
                    .ports
                    .iter()
                    .any(|origin| origin.blocks == [block] && origin.port == port)
            })
            .map(|entry| entry.net.as_str());
        assert_eq!(
            net, None,
            "{block}.{port} sits on net {net:?}. It is one end of a `physical-signal` wire -- a \
             causal block reference -- so a node invented for it is reported by the solver as a \
             voltage, and a sensor reading of 0.42 reaches a plot as 0.42 V (#80)."
        );
    }
}

#[test]
fn the_causal_wire_travels_as_a_block_name_instead() {
    // The positive half of the same statement: `sig2phys` names the block it reads from in a
    // `field=value` token, which is what the target grammar has for a causal connection. This
    // held before #80 was fixed as well -- the emitter never used the invented net for this
    // token -- so it is a control on the shape, not a guard on the defect.
    let out = compiled();
    assert!(
        out.netlist.contains("in=p2s"),
        "the causal wire did not become a block reference:\n{}",
        out.netlist
    );
}

/// The same predicate decides a `.SUBCKT` boundary port's token, and there the invented node
/// reaches the emitted netlist.
///
/// `SubcktPort::new` rejects `physical-signal`, so this is only reachable by hand-written JSON
/// -- `SubcktPort` derives `Deserialize` with no equivalent validator. That is enough: a
/// document is a file, and a file does not go through a Rust constructor.
///
/// Measured on 0.6.1, before the fix, the `X` card read:
///
/// ```text
/// X1 n_X1_a n_X1_ctl LEAF
/// ```
///
/// `n_X1_ctl` is a node invented for a causal port, declared nowhere, handed to the solver.
/// With one predicate it reads `X1 n_X1_a step LEAF` -- the name of the block driving it.
#[test]
fn a_subckt_boundary_port_of_a_causal_kind_carries_a_block_name_not_a_node() {
    use gsim_core::document::GsimSchDocument;

    let doc = GsimSchDocument::from_json(DOC_WITH_A_CAUSAL_SUBCKT_PORT).unwrap();
    let out = compile(&doc.model, "general-simulator").expect("the document compiles");
    let x_card = out
        .netlist
        .lines()
        .find(|line| line.starts_with("X1 "))
        .unwrap_or_else(|| panic!("no X card in:\n{}", out.netlist));
    assert_eq!(
        x_card, "X1 n_X1_a step LEAF",
        "the causal boundary port was emitted as a circuit node rather than as the name of the \
         block driving it (#80). Whole netlist:\n{}",
        out.netlist
    );
    assert!(
        !out.netlist.contains("n_X1_ctl"),
        "the invented node n_X1_ctl reached the netlist:\n{}",
        out.netlist
    );
}

/// A `.SUBCKT` whose boundary declares one acausal port and one causal one, instantiated once.
///
/// Written as JSON rather than built with `ModelBuilder` on purpose: the constructor refuses
/// this port kind, and the point is that the loader does not.
const DOC_WITH_A_CAUSAL_SUBCKT_PORT: &str = r#"{
 "schema_version": 2,
 "model": {
  "id": "m", "name": "M", "grid": 10,
  "subckt_defs": [{
    "id": "leaf", "name": "LEAF",
    "ports": [
      { "id": "a", "kind": "physical-electrical" },
      { "id": "ctl", "kind": "physical-signal" }
    ],
    "blocks": [
      { "id": "leaf", "type": "spice-subckt.boundary", "paradigm": "spice-subckt",
        "domain": "electrical", "position": [0, 0] },
      { "id": "Rin", "type": "passive.resistor", "paradigm": "physical-network",
        "domain": "electrical", "position": [0, 0], "params": { "R": 100 } }
    ],
    "wires": [
      { "id": "wi", "from": { "block": "leaf", "port": "a" }, "to": { "block": "Rin", "port": "p" },
        "kind": "physical-electrical", "net": null },
      { "id": "wj", "from": { "block": "Rin", "port": "n" }, "to": { "block": "leaf", "port": "a" },
        "kind": "physical-electrical", "net": null }
    ]
  }],
  "blocks": [
    { "id": "vin", "type": "source.voltage.dc", "paradigm": "physical-network",
      "domain": "electrical", "position": [0, 0], "params": { "V": 5 } },
    { "id": "gnd", "type": "routing.ground", "paradigm": "physical-network",
      "domain": "electrical", "position": [0, 0] },
    { "id": "step", "type": "signal.step", "paradigm": "signal-flow", "domain": "signal",
      "position": [0, 0], "params": { "before": 0, "after": 1, "t_step": 0.001 } },
    { "id": "X1", "type": "spice-subckt.instance", "paradigm": "spice-subckt",
      "domain": "electrical", "position": [0, 0], "subckt_ref": "leaf" }
  ],
  "wires": [
    { "id": "w1", "from": { "block": "vin", "port": "p" }, "to": { "block": "X1", "port": "a" },
      "kind": "physical-electrical", "net": null },
    { "id": "w2", "from": { "block": "vin", "port": "n" }, "to": { "block": "gnd", "port": "gnd" },
      "kind": "physical-electrical", "net": null },
    { "id": "w3", "from": { "block": "step", "port": "out" }, "to": { "block": "X1", "port": "ctl" },
      "kind": "physical-signal", "net": null }
  ]
 }
}"#;

#[test]
fn the_real_circuit_still_compiles_to_real_nodes() {
    // The control. Without it this file would pass just as happily against a compiler that
    // made no nets at all, which is the other way to get these assertions green.
    let out = compiled();
    assert!(
        out.provenance.nets.iter().any(|n| n.net == "n_rload_p"),
        "the acausal half of the sheet lost its node too:\n{}",
        out.netlist
    );
    assert!(out.netlist.contains("rload"), "{}", out.netlist);
}
