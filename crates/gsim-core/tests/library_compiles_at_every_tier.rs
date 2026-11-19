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
//! port, and asserts it compiles. It asserts nothing about the emitted text -- the field
//! spellings are pinned by `compile::general_simulator`'s unit tests.

use gsim_core::compile::library::catalog;
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
    // GLODI-Engineering/gsim-core#15 item 2 -- `signs`/`inputs` arity. These two carry variadic
    // inputs whose count comes from a parameter, and the tier lists only `out`, so the sweep
    // has nothing to drive and the emitter reports a hole in a positional list. The fix is for
    // the tier to carry the expanded pins (as `control.sum`'s own `n_a` tier used to, with
    // `in0`/`in1`), which is the same `for_each` expansion the contract already does.
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
            match result {
                Ok(netlist) if netlist.is_empty() => failures.push(format!(
                    "{} @ {name}: compiled to an empty netlist",
                    entry.component.id
                )),
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
