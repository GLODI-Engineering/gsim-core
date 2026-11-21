//! Every component the library advertises as compilable must still compile when a user has it
//! at any of the fidelity tiers the library offers for it.
//!
//! `general_simulator_registry_coverage` already sweeps the *fixture* set this way. The gap it
//! leaves is the one that reaches a user: a fixture is a hand-written contract in this
//! repository, while `data/component-library.json` is what a front end paints its palette and
//! its inspector from. A component can be perfectly described by a fixture and still be
//! undrawable-then-uncompilable as the library describes it, because the library declares its
//! own port ids, its own parameter names and one list of parameters *per tier* -- and the
//! registry declares those three things again, separately.
//!
//! That is not a hypothetical split. Two guards in `compile::library` already check the *type*
//! lists in both directions, and the missing guard below them is this one: for every library
//! component that claims to compile, every port the registry turns into a node and every
//! parameter it requires has to be there, in the form it reads, in the tier the user is
//! actually holding.
//!
//! Why per tier rather than once per component: a tier is not a view of one parameter list. The
//! front end's `fidelity-swap` computes a delta and **removes** every parameter the target tier
//! does not list, so picking a tier is what decides which parameters a block carries. A
//! component whose `ideal` tier omits a parameter the registry requires is a component that
//! compiles until the user touches the fidelity dropdown.
//!
//! The test is structural, like the fixture sweep: it builds a one-block document per
//! (component, tier) from that tier's own declared ports and defaults, drives or grounds every
//! port, and asserts it compiles *and emits a card of its own*. It asserts nothing about what
//! the card says -- the field spellings are pinned by `compile::general_simulator`'s unit
//! tests.

use gsim_core::compile::library::catalog;
use gsim_core::compile::spec::is_universal;
use gsim_core::compile::{compile, CompileError};
use gsim_core::document::{ConnectOptions, Endpoint, ModelBuilder, NewBlockSpec, Paradigm};
use serde_json::{Map, Value};

/// One port as a fidelity tier declares it: a flat list entry with an explicit direction, and
/// `kind` as a bare string -- unlike the component-level `ports`, which is grouped by direction
/// and carries `kind` as an object. The tier's spelling is the one used here, because the tier
/// is what a placed block is an instance of.
struct TierPort {
    id: String,
    is_input: bool,
    kind: String,
}

fn tier_ports(tier: &Value) -> Vec<TierPort> {
    tier.get("ports")
        .and_then(|p| p.as_array())
        .map(|ports| {
            ports
                .iter()
                .filter_map(|p| {
                    Some(TierPort {
                        id: p.get("id")?.as_str()?.to_string(),
                        is_input: p.get("direction").and_then(|d| d.as_str()) == Some("in"),
                        kind: p
                            .get("kind")
                            .and_then(|k| k.as_str())
                            .unwrap_or("signal")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The params a block carries at this tier: the tier's own list, keyed by `id`.
///
/// Deliberately *not* merged with the component-level `params` list (which is keyed `name` and
/// does list every parameter). Merging would test a block that the front end never builds:
/// `fidelity-swap` drops whatever the tier omits, so the tier's list alone is what survives
/// choosing it.
fn tier_params(tier: &Value) -> Map<String, Value> {
    tier.get("params")
        .and_then(|p| p.as_array())
        .map(|params| {
            params
                .iter()
                .filter_map(|p| {
                    let id = p.get("id")?.as_str()?.to_string();
                    Some((id, p.get("default").cloned().unwrap_or(Value::Null)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Builds a one-block document holding `registry_type` as the tier describes it, with every
/// input driven and every physical pin on a net, then compiles it.
fn compile_at_tier(
    registry_type: &str,
    ports: &[TierPort],
    params: Map<String, Value>,
) -> Result<String, CompileError> {
    let mut b = ModelBuilder::new("m", "Library tier sweep", 10);
    // The block's own declared domain, not a fixed "electrical": `routing.from`/`routing.to`
    // refuse to route an acausal kind by tag, so handing a signal-flow block an electrical
    // domain fails for a reason the library never asserted.
    let add = |b: &mut ModelBuilder,
               id: &str,
               t: &str,
               paradigm: Paradigm,
               params: Map<String, Value>| {
        let domain = if paradigm == Paradigm::SignalFlow {
            "signal"
        } else {
            "electrical"
        };
        b.add_block(NewBlockSpec {
            r#type: t.to_string(),
            paradigm,
            domain: domain.to_string(),
            id: Some(id.to_string()),
            params,
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("{t}: adding block: {e}"));
    };

    let converter = registry_type.starts_with("converter.");
    let physical = ports.iter().any(|p| p.kind != "signal");
    let paradigm = match (converter, physical) {
        (true, _) => Paradigm::Converter,
        (false, true) => Paradigm::PhysicalNetwork,
        (false, false) => Paradigm::SignalFlow,
    };

    add(&mut b, "dut", registry_type, paradigm, params);
    let needs_ground = physical && registry_type != "routing.ground";
    if needs_ground {
        add(
            &mut b,
            "gnd",
            "routing.ground",
            Paradigm::PhysicalNetwork,
            Map::new(),
        );
    }

    for (i, port) in ports.iter().enumerate() {
        if port.kind == "signal" {
            if !port.is_input {
                continue;
            }
            let drv = format!("drv{i}");
            let mut p = Map::new();
            p.insert("value".to_string(), Value::from(1.0));
            add(&mut b, &drv, "source.const", Paradigm::SignalFlow, p);
            if paradigm == Paradigm::PhysicalNetwork {
                // A physical-network block's control pin (an ideal switch's `g`) is declared
                // signal-kind but sits on the electrical side of the paradigm boundary, so the
                // document model requires a converter between the two -- exactly as a real
                // switching-converter sheet wires it.
                let conv = format!("conv{i}");
                let mut cp = Map::new();
                cp.insert("domain".to_string(), Value::from("voltage"));
                add(&mut b, &conv, "converter.sig2phys", Paradigm::Converter, cp);
                b.connect(
                    Endpoint::new(&drv, "out"),
                    Endpoint::new(&conv, "in"),
                    "signal",
                    ConnectOptions::default(),
                )
                .map_err(|e| CompileError::UnknownTarget(format!("wiring {conv}.in: {e}")))?;
                b.connect(
                    Endpoint::new(&conv, "out"),
                    Endpoint::new("dut", &port.id),
                    "physical-electrical",
                    ConnectOptions::default(),
                )
                .map_err(|e| CompileError::UnknownTarget(format!("wiring {}: {e}", port.id)))?;
            } else {
                b.connect(
                    Endpoint::new(&drv, "out"),
                    Endpoint::new("dut", &port.id),
                    "signal",
                    ConnectOptions::default(),
                )
                .map_err(|e| CompileError::UnknownTarget(format!("wiring {}: {e}", port.id)))?;
            }
        } else if needs_ground {
            b.connect(
                Endpoint::new("dut", &port.id),
                Endpoint::new("gnd", "p"),
                &port.kind,
                ConnectOptions::default(),
            )
            .map_err(|e| CompileError::UnknownTarget(format!("grounding {}: {e}", port.id)))?;
        }
    }

    compile(&b.into_model(), "general-simulator").map(|out| out.netlist)
}

/// (component id, tier) pairs that do not compile yet, each with why and where it is tracked.
///
/// This list is the point of the test as much as the sweep is. Every entry was found by running
/// it, and each is a sheet a user can draw and then fail to run; keeping them enumerated here
/// means the number can only go down by a fix, and a *new* disagreement fails the build on the
/// commit that introduces it rather than on the day someone draws it.
const KNOWN_BROKEN: &[(&str, &str, &str)] = &[
    // GLODI-Engineering/gsim-core#15 item 2, the half that is the sweep's own limitation
    // rather than a user-facing defect. Both of these carry variadic inputs, and a tier
    // declares only `out` -- the input pins come from the GSF v2 contract's `for_each`
    // expansion, which this structural sweep does not run -- so the sweep has nothing to
    // drive and the emitter correctly reports a hole in a positional list.
    //
    // The user-facing half *is* fixed: `signs` is declared `type: string` ("++"), the arity
    // could not be read from it at all, and a sum dropped from the palette reported "needs 0
    // driver(s)". `Arity::ParamLen` and the `ParamList` emitter accept that spelling now, so a
    // placed sum compiles. Closing the sweep's gap needs the contract expansion here, which is
    // a different piece of work.
    (
        "control.sum",
        "ideal",
        "#15.2: variadic inputs are not expanded into the tier",
    ),
    (
        "control.product",
        "ideal",
        "#15.2: variadic inputs are not expanded into the tier",
    ),
    // Not a library defect: a `routing.from` resolves a tag broadcast by a `routing.goto`
    // elsewhere in the same scope, so a one-block document can never satisfy it. Testing it
    // needs a two-block fixture, which is `general_simulator_fixtures`' job, not this sweep's.
    (
        "routing.from",
        "n_a",
        "structural: needs a partner routing.goto in scope",
    ),
];

fn known_broken(id: &str, tier: &str) -> bool {
    KNOWN_BROKEN.iter().any(|(c, t, _)| *c == id && *t == tier)
}

/// How many cards in `netlist` are the block under test's own.
///
/// The check below used to be `netlist.is_empty()`, which could not fail: every emitted netlist
/// opens with
///
/// ```text
/// * probe -- generated by gsim-core 0.5.0 from model 'm'
/// * Do not edit by hand: regenerate from the .gsim_sch document.
/// ```
///
/// so a one-block document that emitted nothing at all still came back 119 bytes long and
/// counted as compiled. Four components passed the sweep that way before #65, and
/// `hierarchy.inport`/`hierarchy.outport` joined them in #63 -- which is how it was found.
///
/// Counting non-comment lines is not enough either, and getting that wrong first is what
/// showed why: this harness *adds* blocks, one `source.const` per signal input and a ground
/// for a physical pin, and their cards are non-comment lines too. A structural block in a
/// document with one driver yields exactly one card, which looks like success from a distance
/// and is the driver's. So the question has to be asked about the block itself.
///
/// A card names the block when its first token is the block id, or the id behind the single
/// device letter `compile::spec::element_name` prefixes when the name lacks one
/// (`dut` -> `Rdut`).
///
/// `universal_components.rs` spells this as `trim_start_matches(char::is_ascii_alphabetic)`,
/// which works there only because every block id it checks carries a digit (`g1`, `scope1`).
/// `dut` is all letters, so trimming leaves the empty string and *no* card ever matches --
/// which reported 32 components as emitting nothing when they emit perfectly good cards. One
/// letter is what the emitter adds, so one letter is what this takes off.
///
/// A **composite** emits several cards instead of one, each named `<letter><id>_<part>` -- a
/// transformer is `Edut_sec` and `Fdut_pri`. Those are the block's cards too, and taking them
/// for somebody else's is what the `_` arm below is for.
fn cards_naming(netlist: &str, block_id: &str) -> usize {
    netlist
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| {
            if name.eq_ignore_ascii_case(block_id) {
                return true;
            }
            // Strip the one device letter the emitter prefixes, then accept the id exactly or
            // the id followed by a composite's `_part` suffix.
            let Some(rest) = name
                .strip_prefix(|c: char| c.is_ascii_alphabetic())
                .filter(|_| name.len() > block_id.len())
            else {
                return false;
            };
            rest.eq_ignore_ascii_case(block_id)
                || rest
                    .strip_prefix(block_id)
                    .is_some_and(|tail| tail.starts_with('_'))
        })
        .count()
}

#[test]
fn every_compilable_component_compiles_at_every_tier_it_offers() {
    let catalog = catalog();
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for entry in &catalog.components {
        if !entry.compilable {
            continue;
        }
        let Some(registry_type) = entry.component.compiles_to.as_deref() else {
            continue;
        };
        let tiers = entry
            .component
            .presentation
            .get("fidelities")
            .and_then(|f| f.as_array())
            .cloned()
            .unwrap_or_default();

        for tier in &tiers {
            let name = tier
                .get("tier")
                .and_then(|t| t.as_str())
                .unwrap_or("<unnamed>");
            let ports = tier_ports(tier);
            let params = tier_params(tier);
            checked += 1;
            let result = compile_at_tier(registry_type, &ports, params);
            if known_broken(&entry.component.id, name) {
                // A fixed entry must be taken off the list, or the list stops describing
                // reality and the next reader trusts it anyway.
                assert!(
                    result.is_err(),
                    "{} @ {name} is on KNOWN_BROKEN but compiles now -- remove the entry",
                    entry.component.id
                );
                continue;
            }
            // A `structural` type is *expected* to emit no card -- a ground only names the
            // reference net, a Goto only annotates connectivity, a boundary port only marks an
            // interface. Asserted rather than exempted: emitting one would mean
            // `Scope::emittable` had leaked, which `tests/universal_components.rs` guards from
            // the other side. The sweep still earns its keep for these, because it builds the
            // block from the tier's own ports and params, so a bad port id there still fails
            // at `connect`.
            //
            // Everything else has to put at least one device line in the netlist. The driver
            // blocks this harness adds are cards too, so a non-structural block that emitted
            // nothing can still clear a bare `> 0` -- hence the structural arm being the
            // *equality*, which is the half that pins something down.
            let structural = is_universal(registry_type);
            match result {
                Ok(netlist) if structural && cards_naming(&netlist, "dut") != 0 => {
                    failures.push(format!(
                        "{} @ {name}: structural, but emitted a card of its own:\n{netlist}",
                        entry.component.id
                    ))
                }
                Ok(netlist) if !structural && cards_naming(&netlist, "dut") == 0 => {
                    failures.push(format!(
                        "{} @ {name}: compiled, but emitted no card of its own:\n{netlist}",
                        entry.component.id
                    ))
                }
                Ok(_) => {}
                Err(e) => failures.push(format!("{} @ {name}: {e}", entry.component.id)),
            }
        }
    }

    assert!(checked > 60, "expected a real sweep, checked {checked}");
    assert!(
        failures.is_empty(),
        "{} of {checked} (component, tier) pairs the library advertises as compilable do not \
         compile as the library describes them:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
