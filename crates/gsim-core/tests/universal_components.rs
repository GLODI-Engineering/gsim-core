//! No target emits a component the editor owns.
//!
//! # What this guards
//!
//! A resistor belongs to a device library: a target decides how to spell it, and an engine that
//! cannot model one may legitimately refuse it. A Goto does not. It is not a part anyone could
//! buy — it is a way of saying *this signal continues over there*, and it means the same thing
//! on every sheet whatever engine the sheet is bound for. So are a ground symbol, a wire
//! junction, a scope, a `.SUBCKT` boundary and the two ports that mark a subsystem's boundary
//! from the inside.
//!
//! Those are the schematic's own vocabulary, the editor owns them, and the consequence this
//! crate is responsible for is that **every target must support them** — they are not the
//! target's components to support or decline.
//!
//! # Why it is a test and not a convention
//!
//! Until gsim-core#54 the guarantee was that each target remembered to skip them.
//! `general_simulator.rs` did remember, in two places. A second target would have had to
//! remember too, and nothing would have told its author: a `routing.goto`'s spec declares no
//! device letter and no `kind=` name, so emitting one writes a malformed card rather than
//! raising a compile error. The failure would have surfaced as a netlist the engine rejects for
//! reasons that name neither the Goto nor the target.
//!
//! Now the shared pipeline resolves each one and `Scope::emittable` leaves them out of what a
//! target iterates, so a target cannot emit one by accident. This test is what says so, and it
//! sweeps `compile::TARGETS` rather than naming a target, so a new one is covered the day it is
//! added to that list.

use gsim_core::compile::spec::{self, universal_types};
use gsim_core::compile::{
    compile, compile_with, CompileContext, CompileError, CompileOutput, NetlistTarget, TARGETS,
};
use gsim_core::document::{ConnectOptions, Endpoint, ModelBuilder, NewBlockSpec, Paradigm};
use serde_json::{json, Map, Value};

/// `spice-subckt.boundary` is a pseudo-block the pipeline creates for a `SubcktDef` body, not
/// something a user places on a sheet, so a top-level document cannot legally contain one. It
/// is universal and it is covered — by every `.SUBCKT` fixture in
/// `tests/fixtures/general_simulator/`, none of whose emitted netlists carry a boundary card.
const NOT_PLACEABLE_ON_A_SHEET: &[&str] = &["spice-subckt.boundary"];

fn params(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

fn add(b: &mut ModelBuilder, id: &str, r#type: &str, paradigm: Paradigm, p: Map<String, Value>) {
    b.add_block(NewBlockSpec {
        r#type: r#type.to_string(),
        paradigm,
        domain: "electrical".to_string(),
        id: Some(id.to_string()),
        params: p,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("adding {type}: {e}", type = r#type));
}

/// A sheet carrying every universal component a user can place, plus one real device so the
/// netlist is not trivially empty.
///
/// The Goto/From pair is matched and driven: an unmatched `From` is `UnresolvedFromTag`, which
/// would fail this test for the wrong reason.
fn sheet_with_every_universal_component() -> gsim_core::document::Model {
    let mut b = ModelBuilder::new("m", "Universal components", 10);

    // A complete circuit, so the sheet compiles for reasons unrelated to this test: a DC
    // source across a resistor, referenced to ground.
    add(
        &mut b,
        "v1",
        "source.voltage.dc",
        Paradigm::PhysicalNetwork,
        params(&[("V", json!(5.0))]),
    );
    add(
        &mut b,
        "r1",
        "passive.resistor",
        Paradigm::PhysicalNetwork,
        params(&[("R", json!(1000.0))]),
    );
    // A wire tap: the front end splices one in when a branch lands on a conductor rather than
    // on a port, so a real sheet carries these without anyone placing one.
    add(
        &mut b,
        "j1",
        "routing.junction",
        Paradigm::PhysicalNetwork,
        Map::new(),
    );
    add(
        &mut b,
        "gnd",
        "routing.ground",
        Paradigm::PhysicalNetwork,
        Map::new(),
    );
    add(
        &mut b,
        "scope1",
        "sinks.scope",
        Paradigm::SignalFlow,
        Map::new(),
    );
    // A signal source to drive the Goto, since a Goto broadcasts whatever drives its `in`.
    add(
        &mut b,
        "src",
        "source.sinwave",
        Paradigm::SignalFlow,
        params(&[("va", json!(1.0)), ("freq", json!(50.0))]),
    );
    add(
        &mut b,
        "g1",
        "routing.goto",
        Paradigm::SignalFlow,
        params(&[("tag", json!("CLK"))]),
    );
    add(
        &mut b,
        "f1",
        "routing.from",
        Paradigm::SignalFlow,
        params(&[("tag", json!("CLK"))]),
    );
    // A subsystem's boundary, from the inside. Grouping blocks synthesizes one of each per
    // wire that crossed the selection, so a real sheet carries these without anyone placing
    // one -- and an inport is a driver, so this pair is wired as the pass-through it is.
    add(
        &mut b,
        "in1",
        "hierarchy.inport",
        Paradigm::SignalFlow,
        params(&[("id", json!("in1")), ("kind", json!("signal"))]),
    );
    add(
        &mut b,
        "out1",
        "hierarchy.outport",
        Paradigm::SignalFlow,
        params(&[("id", json!("out1")), ("kind", json!("signal"))]),
    );

    b.connect(
        Endpoint::new("src", "out"),
        Endpoint::new("g1", "in"),
        "signal",
        ConnectOptions::default(),
    )
    .expect("wiring the source into the Goto");
    b.connect(
        Endpoint::new("f1", "out"),
        Endpoint::new("scope1", "in"),
        "signal",
        ConnectOptions::default(),
    )
    .expect("wiring the From into the scope");
    b.connect(
        Endpoint::new("in1", "out"),
        Endpoint::new("out1", "in"),
        "signal",
        ConnectOptions::default(),
    )
    .expect("the boundary inport carries straight through to the outport");
    b.connect(
        Endpoint::new("v1", "p"),
        Endpoint::new("j1", "node"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .expect("tapping the source's positive terminal");
    b.connect(
        Endpoint::new("j1", "node"),
        Endpoint::new("r1", "p"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .expect("the tap carries on to the resistor");
    b.connect(
        Endpoint::new("r1", "n"),
        Endpoint::new("gnd", "gnd"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .expect("grounding the resistor");
    b.connect(
        Endpoint::new("v1", "n"),
        Endpoint::new("gnd", "gnd"),
        "physical-electrical",
        ConnectOptions::default(),
    )
    .expect("grounding the source");

    b.into_model()
}

#[test]
fn the_registry_names_every_universal_component_once() {
    let types: Vec<&str> = universal_types().collect();

    assert!(
        types.contains(&"routing.goto") && types.contains(&"routing.from"),
        "Goto/From are the case the contract exists for: {types:?}"
    );
    assert!(
        types.contains(&"routing.ground") && types.contains(&"routing.junction"),
        "a ground only names the reference net and a junction only merges conductors: {types:?}"
    );
    assert!(
        types.contains(&"hierarchy.inport") && types.contains(&"hierarchy.outport"),
        "the subsystem boundary ports only mark an interface, like a .SUBCKT boundary: {types:?}"
    );

    let mut sorted = types.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        types.len(),
        "one registry entry per universal type, not several: {types:?}"
    );

    for t in &types {
        assert!(
            spec::is_universal(t),
            "{t} is in the list but is_universal says otherwise"
        );
    }
    assert!(
        !spec::is_universal("passive.resistor"),
        "a resistor belongs to a library, not to the editor"
    );
    assert!(
        !spec::is_universal("no.such.type"),
        "an unknown type is not universal -- it must still raise UnknownType"
    );
}

#[test]
fn no_target_emits_a_component_the_editor_owns() {
    let model = sheet_with_every_universal_component();
    let placeable: Vec<&str> = universal_types()
        .filter(|t| !NOT_PLACEABLE_ON_A_SHEET.contains(t))
        .collect();
    assert!(!placeable.is_empty(), "nothing to check");

    for target in TARGETS {
        let out = compile(&model, target)
            .unwrap_or_else(|e| panic!("{target}: the sheet should compile: {e:?}"));

        // The device survived, so the netlist is not empty for an unrelated reason.
        assert!(
            out.netlist.contains("R1") || out.netlist.to_lowercase().contains("r1"),
            "{target}: the resistor should still be emitted:\n{}",
            out.netlist
        );

        for block_id in ["gnd", "scope1", "g1", "f1", "j1", "in1", "out1"] {
            for line in out.netlist.lines() {
                let card = line.trim();
                assert!(
                    !card.split_whitespace().next().is_some_and(|name| {
                        name.eq_ignore_ascii_case(block_id)
                            || name.trim_start_matches(|c: char| c.is_ascii_alphabetic())
                                == block_id
                    }),
                    "{target}: a universal component was emitted as a card:\n  {card}\nin:\n{}",
                    out.netlist
                );
            }
        }

        for t in &placeable {
            assert!(
                !out.netlist.contains(t),
                "{target}: the netlist names the universal type {t}:\n{}",
                out.netlist
            );
        }
    }
}

/// A target written by somebody who has never read the universal-component rule.
///
/// It emits one line per block it is handed, with no filtering of any kind. That is the whole
/// point: it is the stand-in for the second target this crate does not have yet, and it is how
/// the guarantee gets tested by code that does not already know the answer.
///
/// `general_simulator.rs` cannot play this role. Its emitter is *tolerant*: a `routing.ground`
/// has no device letter and no `kind=` name, so handing it one produces no card rather than a
/// bad one, and an output assertion against that target passes whether or not the filtering
/// works. Verified by reverting the filter and watching the output assertions stay green --
/// which is why this target exists.
struct NaiveTarget;

impl NetlistTarget for NaiveTarget {
    fn name(&self) -> &'static str {
        "naive-test-target"
    }

    fn emit(&self, ctx: &CompileContext<'_>) -> Result<CompileOutput, CompileError> {
        let mut netlist = String::new();
        for block in ctx.top.emittable() {
            netlist.push_str(&format!("{} {}\n", block.id, block.r#type));
        }
        Ok(CompileOutput {
            netlist,
            notes: Vec::new(),
            provenance: Default::default(),
            code_build: None,
            params: Default::default(),
        })
    }
}

/// The guarantee, tested by a target that does no filtering of its own.
///
/// If `Scope::emittable` ever leaked a universal component, this target would write it into its
/// netlist and this test would name it. That is the regression the output assertions against
/// `general-simulator` could not catch.
#[test]
fn a_target_that_filters_nothing_still_emits_no_universal_component() {
    let model = sheet_with_every_universal_component();
    let out = compile_with(&model, &NaiveTarget).expect("the sheet compiles");

    assert!(
        out.netlist.contains("passive.resistor") && out.netlist.contains("source.voltage.dc"),
        "the naive target really does emit what it is handed:\n{}",
        out.netlist
    );

    for t in universal_types() {
        assert!(
            !out.netlist.contains(t),
            "a target that filters nothing was handed the universal component {t}:\n{}",
            out.netlist
        );
    }
    for block_id in ["gnd", "scope1", "g1", "f1", "j1", "in1", "out1"] {
        assert!(
            !out.netlist
                .lines()
                .any(|l| l.split_whitespace().next() == Some(block_id)),
            "block {block_id} reached a target that filters nothing:\n{}",
            out.netlist
        );
    }
}

/// The From's reference was rewritten to the real driver, which is the *point* of resolving
/// Goto/From in the shared pipeline: the scope reads the source, not a block that no longer
/// exists.
#[test]
fn resolution_happens_before_any_target_sees_the_document() {
    let model = sheet_with_every_universal_component();

    for target in TARGETS {
        let out = compile(&model, target).expect("compiles");
        assert!(
            !out.netlist.contains("f1"),
            "{target}: the From should have been resolved away:\n{}",
            out.netlist
        );
    }
}

/// A saved sheet that names a component the way the editor's old catalogue did still compiles.
///
/// `library::find_component` resolves a name through the library's aliases and its doc comment
/// says why it exists: *"Documents already on disk name components the way the front end's
/// catalogue did (`physical-electrical.passive.resistor`), while the library is now keyed the way
/// the engine names them (`passive.resistor`). Those older spellings are kept as aliases, and
/// this is what makes a saved sheet still resolve."*
///
/// It had **no callers**, so it made nothing resolve. `spec::lookup` was exact-match, and one
/// crate gave two answers about one name: the library saying
/// `physical-electrical.source.voltage_dc` is a compilable DC source, the compiler saying
/// `unknown-type: ... add a TypeSpec for it`.
///
/// Found by compiling this project's own `.gsim_sch` fixtures through the WASM binding — every
/// one failed on an alias spelling. The editor only gets away with it because its loader
/// canonicalises types on deserialize; a document written by hand, or saved before that
/// canonicalisation existed, did not compile at all.
#[test]
fn an_alias_spelling_resolves_to_its_registry_entry() {
    use gsim_core::compile::library;

    // Every spelling the library declares for one component must reach one registry entry.
    for alias in [
        "source.voltage.dc",
        "physical-electrical.source.voltage_dc",
        "physical_electrical.source.voltage_dc",
        "voltage_dc",
    ] {
        let found = spec::lookup(alias)
            .unwrap_or_else(|| panic!("{alias} should resolve through the library's aliases"));
        assert_eq!(
            found.type_name, "source.voltage.dc",
            "{alias} resolved to the wrong entry"
        );
    }

    for alias in [
        "passive.resistor",
        "physical-electrical.passive.resistor",
        "physical_electrical.passive.resistor",
    ] {
        assert_eq!(
            spec::lookup(alias).map(|s| s.type_name),
            Some("passive.resistor"),
            "{alias}"
        );
    }

    // `id` is the engine's name and `type` is the legacy spelling -- the opposite of what the
    // field names suggest. Getting it the wrong way round resolves every alias to another name
    // the registry does not have, which is exactly what the first version of this fix did.
    let component = library::find_component("physical-electrical.source.voltage_dc")
        .expect("the library knows the alias");
    assert_eq!(component.id, "source.voltage.dc", "id is the engine's name");
    assert_eq!(
        component.r#type.as_deref(),
        Some("physical-electrical.source.voltage_dc"),
        "type is the legacy catalogue spelling"
    );

    // A name nothing knows is still unknown -- the fallback must not invent a resolution.
    assert!(spec::lookup("no.such.component").is_none());
}
