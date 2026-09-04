//! Every GSF v2 contract fixture under `tests/fixtures/general_simulator/` must have a real
//! Phase 2 emitter entry in `compile::spec::REGISTRY`.
//!
//! The fixture directory is the closest thing this repo has to an authoritative "what does
//! `general-simulator` actually support" list, so a fixture without a registry entry is a
//! component a document can legally contain and the compiler then refuses with
//! [`CompileError::UnknownType`]. That gap was found the hard way (a real circuit needed
//! `control.sum`/`math.fn1`/`source.sinwave` and hit `UnknownType`); this test is the
//! regression that keeps the two lists in sync as fixtures are added.
//!
//! It is deliberately *structural*, not a golden-netlist test: it builds a one-block document
//! per fixture from that fixture's own declared ports and parameter defaults, drives every
//! input, and asserts the whole thing compiles. Exact field spellings are asserted in
//! `compile::general_simulator`'s own unit tests and proved end to end by running the emitted
//! netlist through the real simulator (see `docs/journal/2026-09.md`).

use gsim_core::compile::{compile, CompileError};
use gsim_core::contract::{evaluate_component, ComponentContract};
use gsim_core::document::{ConnectOptions, Endpoint, ModelBuilder, NewBlockSpec, Paradigm};
use gsim_core::port::PortDirection;
use serde_json::{Map, Value};

/// Fixtures deliberately excluded from the "must compile" sweep, with the reason.
///
/// `control.sum.stable_ids` is not a `general-simulator` component type at all — it is a second,
/// *illustrative* fixture for `control.sum` that demonstrates the identity-based `for_each`
/// authoring pattern (`in_${{ operand.id }}` over an array of objects) rather than the
/// position-indexed one the real `control.sum` fixture uses. See
/// `general_simulator_fixtures::stable_id_pattern_survives_removing_an_interior_operand`. The
/// engine has one `kind=sum`; giving this a registry entry would register a type
/// `general-simulator` does not have.
const NOT_A_DISTINCT_TYPE: &[&str] = &["control.sum.stable_ids"];

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/general_simulator")
}

fn fixture_stems() -> Vec<String> {
    let mut stems: Vec<String> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .filter_map(|e| {
            let path = e.ok()?.path();
            if path.extension()? != "json" {
                return None;
            }
            Some(path.file_stem()?.to_string_lossy().into_owned())
        })
        .collect();
    stems.sort();
    stems
}

fn load(stem: &str) -> ComponentContract {
    let path = fixture_dir().join(format!("{stem}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {path:?}: {e}"))
}

/// The fixture's own declared defaults, as the `params` map a document block would carry. The
/// compiler has no contract knowledge — it reads `Block.params` directly — so a realistic
/// instance is one whose params were defaulted at insert time, which is what this builds.
fn defaults(contract: &ComponentContract) -> Map<String, Value> {
    contract
        .parameters
        .iter()
        .map(|p| (p.name.clone(), p.default.clone()))
        .collect()
}

/// Builds a minimal document holding one instance of `type_name` with every input port driven,
/// then compiles it for the `general-simulator` target.
fn compile_one(type_name: &str, contract: &ComponentContract) -> Result<String, CompileError> {
    let params = defaults(contract);
    let evaluated = evaluate_component(contract, &params)
        .unwrap_or_else(|e| panic!("{type_name}: evaluating fixture with its own defaults: {e}"));

    let mut b = ModelBuilder::new("m", "Coverage", 10);
    let add = |b: &mut ModelBuilder, id: &str, t: &str, paradigm, params: Map<String, Value>| {
        b.add_block(NewBlockSpec {
            r#type: t.to_string(),
            paradigm,
            domain: "electrical".to_string(),
            id: Some(id.to_string()),
            params,
            ..Default::default()
        })
        .unwrap_or_else(|e| panic!("{t}: adding block: {e}"));
    };

    // A converter is the one family that legally carries pins of both kinds; everything else is
    // classified by whether it has any physical pin at all.
    let converter = type_name.starts_with("converter.");
    let physical = evaluated.ports.iter().any(|p| p.kind != "signal");
    let paradigm = match (converter, physical) {
        (true, _) => Paradigm::Converter,
        (false, true) => Paradigm::PhysicalNetwork,
        (false, false) => Paradigm::SignalFlow,
    };
    add(&mut b, "dut", type_name, paradigm, params);
    if physical {
        add(
            &mut b,
            "gnd",
            "routing.ground",
            Paradigm::PhysicalNetwork,
            Map::new(),
        );
    }

    for (i, port) in evaluated.ports.iter().enumerate() {
        if port.kind == "signal" {
            if port.direction != PortDirection::In {
                continue;
            }
            let drv = format!("drv{i}");
            let mut p = Map::new();
            p.insert("value".to_string(), Value::from(1.0));
            add(&mut b, &drv, "source.const", Paradigm::SignalFlow, p);
            if paradigm == Paradigm::PhysicalNetwork {
                // A `physical-network` block's control pin (an ideal switch's `g`) is declared
                // `kind: "signal"` but lives on the electrical side of the paradigm boundary --
                // the document model requires a converter in between, exactly as a real
                // switching-converter schematic wires it.
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
                .unwrap_or_else(|e| panic!("{type_name}: wiring {conv}.in: {e}"));
                b.connect(
                    Endpoint::new(&conv, "out"),
                    Endpoint::new("dut", &port.id),
                    "physical-electrical",
                    ConnectOptions::default(),
                )
                .unwrap_or_else(|e| panic!("{type_name}: wiring {}: {e}", port.id));
            } else {
                b.connect(
                    Endpoint::new(&drv, "out"),
                    Endpoint::new("dut", &port.id),
                    "signal",
                    ConnectOptions::default(),
                )
                .unwrap_or_else(|e| panic!("{type_name}: wiring {}: {e}", port.id));
            }
        } else {
            // Every physical pin gets a net so element cards have their positional nodes.
            b.connect(
                Endpoint::new("dut", &port.id),
                Endpoint::new("gnd", "p"),
                &port.kind,
                ConnectOptions::default(),
            )
            .unwrap_or_else(|e| panic!("{type_name}: grounding {}: {e}", port.id));
        }
    }

    compile(&b.into_model(), "general-simulator").map(|out| out.netlist)
}

#[test]
fn every_fixture_type_has_a_registry_entry_and_compiles() {
    let mut unknown = Vec::new();
    let mut failed = Vec::new();
    for stem in fixture_stems() {
        if NOT_A_DISTINCT_TYPE.contains(&stem.as_str()) {
            continue;
        }
        let contract = load(&stem);
        assert_eq!(
            contract.name, stem,
            "fixture file name must match the contract's own type name"
        );
        match compile_one(&stem, &contract) {
            Ok(netlist) => assert!(
                !netlist.is_empty(),
                "{stem}: compiled to an empty netlist -- a registered type must emit something"
            ),
            Err(CompileError::UnknownType { .. }) => unknown.push(stem),
            Err(other) => failed.push(format!("{stem}: {other}")),
        }
    }
    assert!(
        unknown.is_empty(),
        "fixture types with no compile::spec::REGISTRY entry: {unknown:#?}"
    );
    assert!(
        failed.is_empty(),
        "fixture types that failed to compile: {failed:#?}"
    );
}

#[test]
fn the_excluded_fixture_is_a_variant_of_an_already_registered_type() {
    for stem in NOT_A_DISTINCT_TYPE {
        // It must actually exist (a stale exclusion would silently shrink the sweep above)...
        let contract = load(stem);
        assert_eq!(&contract.name.as_str(), stem);
        // ...and the type it illustrates must itself be registered, so excluding it costs no
        // real coverage.
        let (base, _) = stem
            .rsplit_once('.')
            .expect("an illustrative variant is named <real type>.<variant>");
        assert!(
            gsim_core::compile::lookup(base).is_some(),
            "{stem} illustrates {base}, which must be registered"
        );
    }
}
