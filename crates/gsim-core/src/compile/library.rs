//! The component library: what a front end shows, declared here.
//!
//! The front end used to keep its own component catalogue — labels, categories, port
//! geometry, parameter names and defaults — beside this crate's compile registry. Two
//! descriptions of the same components, maintained by hand, and they drifted: the catalogue
//! said a switch takes `r_off` while the registry's switch takes `g_off`, and a bridge
//! between them would have had to carry a reciprocal nobody would remember.
//!
//! So the engine owns both halves. [`catalog`] is the one description: each component's
//! presentation *and* whether this engine can compile it, which it answers by looking the
//! component's `compiles_to` up in [`REGISTRY`](super::spec::REGISTRY) rather than by
//! storing a second opinion about it.
//!
//! The data lives in `data/component-library.json`, embedded at build time. It is generated
//! from the front end's metadata as a one-time migration and edited here afterwards: this is
//! now the place a component's label or default is changed.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::spec::{TypeSpec, REGISTRY};

/// Bumped when the shape of [`Catalog`] changes in a way a consumer must notice.
pub const CATALOG_SCHEMA_VERSION: u32 = 1;

const LIBRARY_JSON: &str = include_str!("../../data/component-library.json");

/// One component, exactly as the library file describes it.
///
/// The presentation fields are deliberately loose [`serde_json::Value`]s: a port's offset or a
/// symbol's bounding box means nothing to this crate, and parsing it into a type here would
/// only create something else to keep in step with whatever draws it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    /// Stable identity — the component's type when it has one, else its name.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paradigm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// The registry type this component compiles to, when one exists.
    ///
    /// This is the spelling bridge, and it lives on the component so that nothing outside
    /// this crate has to keep a parallel map. A sheet carries
    /// `physical-electrical.passive.resistor`; the registry calls it `passive.resistor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiles_to: Option<String>,
    #[serde(flatten)]
    pub presentation: serde_json::Map<String, serde_json::Value>,
}

/// A component as a consumer sees it: the library's description plus this engine's verdict.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    #[serde(flatten)]
    pub component: Component,
    /// Whether this engine can compile it — `compiles_to` naming a live registry type.
    ///
    /// Not stored in the library file. Derived, so it cannot disagree with the registry.
    pub compilable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub generator: String,
    pub components: Vec<CatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct LibraryFile {
    #[allow(dead_code)]
    schema_version: u32,
    components: Vec<Component>,
}

fn library() -> &'static Vec<Component> {
    static LIBRARY: OnceLock<Vec<Component>> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        // A parse failure here is a broken build artefact, not a runtime condition: the file
        // is embedded from this repository and a test parses it.
        let parsed: LibraryFile = serde_json::from_str(LIBRARY_JSON)
            .expect("embedded component-library.json is not valid for LibraryFile");
        parsed.components
    })
}

fn spec_for(type_name: &str) -> Option<&'static TypeSpec> {
    REGISTRY.iter().find(|s| s.type_name == type_name)
}

/// Every component this engine knows about, each saying whether it can be compiled.
///
/// Components it cannot compile are included rather than hidden. A palette that silently
/// dropped them would be telling the user those components do not exist, when what is true is
/// that this engine has no model for them yet — a different statement, and the one worth
/// showing.
pub fn catalog() -> Catalog {
    let components = library()
        .iter()
        .map(|component| CatalogEntry {
            compilable: component
                .compiles_to
                .as_deref()
                .is_some_and(|t| spec_for(t).is_some()),
            component: component.clone(),
        })
        .collect();
    Catalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        generator: format!("gsim-core {}", env!("CARGO_PKG_VERSION")),
        components,
    }
}

/// The component a name refers to: its id, its `type`, or any alias it answers to.
///
/// Documents already on disk name components the way the front end's catalogue did
/// (`physical-electrical.passive.resistor`), while the library is now keyed the way the engine
/// names them (`passive.resistor`). Those older spellings are kept as aliases, and this is
/// what makes a saved sheet still resolve.
pub fn find_component(name: &str) -> Option<&'static Component> {
    library().iter().find(|c| {
        c.id == name || c.r#type.as_deref() == Some(name) || c.aliases.iter().any(|a| a == name)
    })
}

/// The port-kind domain the library declares for one port of one component.
///
/// `None` when nothing is knowable: an unrecognised component or port, or a port carrying
/// `domain_from` — a subsystem boundary port, whose domain is a parameter's *value*, so the
/// library genuinely does not know it.
///
/// Why this exists, given that a [`crate::document::Wire`] already carries a `kind`: §8.4 makes
/// that field a *duplicate* of the endpoints' port kind, so schema validation never needs the
/// catalogue — and a duplicate can be wrong. Real documents spell a wire into a signal port
/// `physical-electrical`, this repository's own `buck_state_space_pi` example among them, and a
/// net invented for a causal connection is a node `general-simulator`'s grammar cannot express
/// (#80, #77). This is how [`super::build_scope`] asks the one description that cannot be wrong.
///
/// Variadic ports resolve through their group's template: `resolve-ports.ts` names an expanded
/// port `<template.id><index>`, so `in3` on a `control.sum` answers with the template's domain.
pub fn port_domain(type_name: &str, port: &str) -> Option<&'static str> {
    let component = find_component(type_name)?;
    let groups = component.presentation.get("ports")?;

    let domain_of = |declared: &'static serde_json::Value| -> Option<&'static str> {
        if declared.get("domain_from").is_some() {
            return None;
        }
        declared.get("kind")?.get("domain")?.as_str()
    };

    for group in ["inputs", "outputs"] {
        if let Some(list) = groups.get(group).and_then(|g| g.as_array()) {
            if let Some(declared) = list
                .iter()
                .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(port))
            {
                return domain_of(declared);
            }
        }
    }
    for group in ["variadic_inputs", "variadic_outputs"] {
        let Some(template) = groups.get(group).and_then(|g| g.get("template")) else {
            continue;
        };
        let Some(prefix) = template.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(index) = port.strip_prefix(prefix) else {
            continue;
        };
        if !index.is_empty() && index.chars().all(|c| c.is_ascii_digit()) {
            return domain_of(template);
        }
    }
    None
}

/// [`catalog`] as JSON.
pub fn catalog_json() -> String {
    serde_json::to_string_pretty(&catalog()).expect("Catalog serialises")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn the_embedded_library_parses() {
        // Derived rather than a magic number. The bound used to be `> 200`, which was a
        // truncated-read check while the library held 240 components of which 164 compiled
        // nowhere. Since #59 it holds exactly what this engine compiles, so the size is a
        // consequence of the registry: every type a target emits, plus the universal
        // components a front end has to be *told how to draw* -- a Goto, a From, a ground,
        // both catalogue spellings of a scope, and the two subsystem boundary ports.
        //
        // Two universals are deliberately absent, which is why this is not simply
        // `REGISTRY.len()`, and the line between them and the seven is what a front end needs
        // rather than who performs the gesture. A `routing.junction` is spliced in by drawing
        // onto a conductor and is drawn as a dot by the editor's own wire renderer; a
        // `spice-subckt.boundary` is a pseudo-block this crate invents for a def's interface
        // and no front end ever sees one. Neither has anything to read out of a catalogue.
        //
        // The boundary ports are the case that drew the line (#63). They are not placed by a
        // user any more than a junction is -- grouping blocks into a subsystem synthesizes one
        // per crossing wire -- but `group-blocks.ts` builds them with
        // `createBlockFromMetadata(catalog["hierarchy.inport"])`, so it needs their ports,
        // their symbol and their `id`/`kind` params from here. Leaving them out made Ctrl+G
        // return `null` and do nothing at all. See `docs/decisions/0007`.
        //
        // `the_registry_is_not_drifting_further_from_the_library` exempts every structural
        // type for that reason; this counts the seven that a catalogue has to describe.
        const DESCRIBED_UNIVERSALS: usize = 7;
        let emittable = REGISTRY.iter().filter(|s| !s.structural).count();
        assert_eq!(
            library().len(),
            emittable + DESCRIBED_UNIVERSALS,
            "the embedded library did not parse whole, or a type was added on one side only"
        );
    }

    #[test]
    fn every_compiles_to_names_a_real_registry_type() {
        // The drift guard. A component pointing at a type the registry does not have would
        // report itself compilable-looking in the file and fail at compile time instead, which
        // is the exact failure mode this module exists to remove.
        let dangling: Vec<_> = library()
            .iter()
            .filter_map(|c| c.compiles_to.as_deref().map(|t| (c.id.as_str(), t)))
            .filter(|(_, t)| spec_for(t).is_none())
            .collect();
        assert!(dangling.is_empty(), "no such registry type: {dangling:?}");
    }

    #[test]
    fn no_port_sits_on_a_face_corner() {
        // A symbol is drawn inside its bbox, so a port declared on a face *corner* can only be
        // met by artwork running into that corner -- which no symbol does. The port marker then
        // draws as a loose circle beside the component with nothing joining the two, and a
        // reader cannot tell which terminal belongs to what. Four components shipped that way
        // (both dependent current-controlled sources with p and n on the same face's two
        // corners, the comparator carrying the ideal opamp's offsets in a shorter box, and the
        // n-MESFET putting all three terminals at the far corner of their faces).
        //
        // The geometry means nothing to this crate, which is exactly why the rule has to be
        // asserted here rather than left to whichever generator wrote the file: the data is
        // embedded from this repository, so this is the only place a fifth cannot slip past.
        let mut offenders = Vec::new();
        for c in library() {
            let Some(bbox) = c
                .presentation
                .get("symbol")
                .and_then(|s| s.get("bbox"))
                .and_then(|b| b.as_array())
            else {
                continue;
            };
            let (Some(w), Some(h)) = (
                bbox.first().and_then(|v| v.as_f64()),
                bbox.get(1).and_then(|v| v.as_f64()),
            ) else {
                continue;
            };
            let Some(ports) = c.presentation.get("ports") else {
                continue;
            };
            for group in ["inputs", "outputs"] {
                let Some(list) = ports.get(group).and_then(|g| g.as_array()) else {
                    continue;
                };
                for port in list {
                    let (Some(side), Some(offset)) = (
                        port.get("side").and_then(|v| v.as_str()),
                        port.get("offset").and_then(|v| v.as_f64()),
                    ) else {
                        continue;
                    };
                    // A left/right port runs along the box's height; top/bottom along its width.
                    let extent = match side {
                        "left" | "right" => h,
                        "top" | "bottom" => w,
                        _ => continue,
                    };
                    if offset <= 2.0 || offset >= extent - 2.0 {
                        let id = port
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("<unnamed>");
                        offenders.push(format!(
                            "{} {group}:{id} side={side} offset={offset} extent={extent}",
                            c.id
                        ));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "{} port(s) sit within 2 units of a face corner, where no artwork reaches them:\n  {}",
            offenders.len(),
            offenders.join("\n  ")
        );
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = library().iter().map(|c| c.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate component ids");
    }

    #[test]
    fn a_component_carries_what_a_front_end_needs_to_draw_it() {
        let r = library()
            .iter()
            .find(|c| c.id == "passive.resistor")
            .expect("the resistor is in the library");
        assert_eq!(r.compiles_to.as_deref(), Some("passive.resistor"));
        assert!(r.label.is_some(), "no label to show");
        for key in ["ports", "params", "symbol"] {
            assert!(
                r.presentation.contains_key(key),
                "the resistor has no {key} to draw from"
            );
        }
    }

    /// Every node port a `TypeSpec` asks for is a port the component actually declares.
    ///
    /// This is the guard the `p1`/`p2` versus `p`/`n` split got through (#27, #4): the library
    /// said a resistor's pins are `p` and `n`, `TWO_PIN` in `compile::spec` said `p1` and `p2`,
    /// and nothing compared the two — so a sheet wired as the palette invited failed with
    /// `floating-port: port 'R1.p1' needs a node`, naming a port the sheet never used. The type
    /// lists are guarded in both directions above; this is the level below them, the port names
    /// inside one component.
    ///
    /// Checked against the component's own `ports` and against **every fidelity tier's** ports,
    /// because a tier carries its own port list and is what a placed block is an instance of.
    #[test]
    fn every_node_port_a_spec_requires_is_declared_by_its_component() {
        fn port_ids(value: Option<&serde_json::Value>) -> Vec<String> {
            let mut ids = Vec::new();
            match value {
                // Component level: grouped by direction, `{"inputs": [...], "outputs": [...]}`.
                Some(serde_json::Value::Object(groups)) => {
                    for group in groups.values() {
                        if let Some(list) = group.as_array() {
                            ids.extend(
                                list.iter()
                                    .filter_map(|p| p.get("id")?.as_str().map(str::to_string)),
                            );
                        }
                    }
                }
                // Tier level: one flat list.
                Some(serde_json::Value::Array(list)) => ids.extend(
                    list.iter()
                        .filter_map(|p| p.get("id")?.as_str().map(str::to_string)),
                ),
                _ => {}
            }
            ids
        }

        let mut missing: Vec<String> = Vec::new();
        for component in library() {
            let Some(type_name) = component.compiles_to.as_deref() else {
                continue;
            };
            let Some(spec) = crate::compile::lookup(type_name) else {
                continue; // guarded by `every_compiles_to_names_a_real_registry_type`
            };
            if spec.node_ports.is_empty() {
                continue;
            }

            let declared = port_ids(component.presentation.get("ports"));
            for wanted in spec.node_ports {
                if !declared.iter().any(|d| d == wanted) {
                    missing.push(format!(
                        "{}: spec wants node port '{}', component declares [{}]",
                        component.id,
                        wanted,
                        declared.join(", ")
                    ));
                }
            }

            if let Some(tiers) = component
                .presentation
                .get("fidelities")
                .and_then(|f| f.as_array())
            {
                for tier in tiers {
                    let name = tier
                        .get("tier")
                        .and_then(|t| t.as_str())
                        .unwrap_or("<unnamed>");
                    let at_tier = port_ids(tier.get("ports"));
                    if at_tier.is_empty() {
                        continue;
                    }
                    for wanted in spec.node_ports {
                        if !at_tier.iter().any(|d| d == wanted) {
                            missing.push(format!(
                                "{} @ {}: spec wants node port '{}', tier declares [{}]",
                                component.id,
                                name,
                                wanted,
                                at_tier.join(", ")
                            ));
                        }
                    }
                }
            }
        }

        assert!(
            missing.is_empty(),
            "{} component/port disagreements:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }

    /// Every parameter the registry **requires** is offered by the tier a user actually gets.
    ///
    /// The level below the port guard above, and the other half of #15. Two of its four items
    /// were this: `control.pid` required `n`, `clamp_lo` and `clamp_hi` while the `ideal` tier
    /// offered `anti_windup`, and `control.sum`'s `signs` was a string where the emitter read
    /// an array. A block dropped from the palette with its own defaults untouched then failed
    /// at Run with `missing-param` or `indexed-ref-arity`, naming something the user never
    /// chose.
    ///
    /// Only `kind=` card fields are checked, because that is where a required parameter is
    /// declared per type; an element card's value comes from [`ElementTail`], whose own
    /// parameter is checked by the compile sweep in
    /// `tests/library_compiles_at_every_tier.rs`.
    #[test]
    fn every_required_parameter_is_offered_by_every_tier() {
        fn required_params(spec: &crate::compile::TypeSpec) -> Vec<&'static str> {
            let Some(kind) = spec.kind.as_ref() else {
                return Vec::new();
            };
            kind.fields
                .iter()
                .filter_map(|f| match f {
                    crate::compile::spec::FieldSpec::Param {
                        param,
                        required: true,
                        ..
                    }
                    | crate::compile::spec::FieldSpec::ParamList {
                        param,
                        required: true,
                        ..
                    } => Some(*param),
                    _ => None,
                })
                .collect()
        }

        let mut missing: Vec<String> = Vec::new();
        for component in library() {
            let Some(type_name) = component.compiles_to.as_deref() else {
                continue;
            };
            let Some(spec) = crate::compile::lookup(type_name) else {
                continue;
            };
            let wanted = required_params(spec);
            if wanted.is_empty() {
                continue;
            }

            let Some(tiers) = component
                .presentation
                .get("fidelities")
                .and_then(|f| f.as_array())
            else {
                continue;
            };
            for tier in tiers {
                let name = tier
                    .get("tier")
                    .and_then(|t| t.as_str())
                    .unwrap_or("<unnamed>");
                let offered: Vec<String> = tier
                    .get("params")
                    .and_then(|p| p.as_array())
                    .map(|list| {
                        list.iter()
                            .filter_map(|p| p.get("id")?.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                for want in &wanted {
                    if !offered.iter().any(|o| o == want) {
                        missing.push(format!(
                            "{} @ {}: emitter requires '{}', tier offers [{}]",
                            component.id,
                            name,
                            want,
                            offered.join(", ")
                        ));
                    }
                }
            }
        }

        assert!(
            missing.is_empty(),
            "{} required parameters a tier does not offer:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }

    #[test]
    fn compilable_is_derived_from_the_registry_not_stored() {
        // Asserted as the derivation itself rather than by example: for every component,
        // `compilable` is exactly whether `spec::lookup` finds a `TypeSpec`. Nothing is read
        // from the data file, so the flag cannot disagree with what the compiler will do.
        //
        // This used to be pinned with a resistor (compilable) and a MOSFET (not), the MOSFET
        // standing for "in the library, honestly marked unsupported". Since #59 the library
        // holds only what this engine compiles, so no such component exists to point at --
        // `every_component_resolves_under_its_own_id` is now what catches an entry the
        // registry does not back.
        for entry in &catalog().components {
            let id = &entry.component.id;
            assert_eq!(
                entry.compilable,
                crate::compile::spec::lookup(id).is_some(),
                "{id}: the catalogue's `compilable` disagrees with `spec::lookup`"
            );
        }
    }

    /// Registry types no component points at — compilable by the engine, unreachable from a
    /// palette. Printed by the test below so the list is visible rather than implied.
    fn registry_types_without_a_component() -> Vec<&'static str> {
        let reachable: Vec<&str> = library()
            .iter()
            .filter_map(|c| c.compiles_to.as_deref())
            .collect();
        let mut orphans: Vec<&str> = REGISTRY
            .iter()
            .filter(|s| !s.structural)
            .map(|s| s.type_name)
            .filter(|t| !reachable.contains(t))
            .collect();
        orphans.sort_unstable();
        orphans
    }

    #[test]
    fn the_registry_is_not_drifting_further_from_the_library() {
        // Both directions matter. The library naming a type the registry lacks is caught
        // above; this is the other way round — a type this engine can compile that no
        // component can place, so a user cannot reach it from a schematic at all.
        //
        // Zero, and it stays zero: adding a registry type without a component to place it
        // puts the two descriptions back out of step, which is what the single library was
        // for. A type the engine can compile and no palette can reach is not reachable by a
        // user at all.
        let orphans = registry_types_without_a_component();
        assert!(
            orphans.is_empty(),
            "{} registry types have no component to place them:\n  {}",
            orphans.len(),
            orphans.join("\n  ")
        );
    }

    /// Every component resolves under **its own id** -- the name a saved document carries.
    ///
    /// Not the same question as the guard above, which walks the other direction through
    /// `compiles_to`. `compiles_to` is a *claim* in the data file; `spec::lookup` is what the
    /// compiler runs, and it resolves an unknown spelling by finding the component and then
    /// matching `component.id` against the registry. The two disagreed, and nothing said so:
    ///
    /// ```text
    /// lookup=false  find_component.id=Some("physical-electrical.passive.transformer")
    /// lookup=true   find_component.id=None   <- physical_electrical.passive.transformer
    /// ```
    ///
    /// The library spelled the transformer and the mutual inductor with a **hyphen** and the
    /// registry with an **underscore**, so each was split into two unreachable halves: the
    /// drawable name failed `UnknownType`, and the compilable name had no component to place
    /// it. Both carried `compiles_to` pointing at the registry's spelling, so the guard above
    /// was satisfied the entire time and two components were unusable in either direction.
    ///
    /// This is the guard that makes "the library is exactly what this engine compiles" true
    /// under the resolver a document actually goes through, rather than under a field that
    /// only promises it.
    #[test]
    fn every_component_resolves_under_its_own_id() {
        let mut unresolvable: Vec<&str> = library()
            .iter()
            .filter(|c| crate::compile::spec::lookup(&c.id).is_none())
            .map(|c| c.id.as_str())
            .collect();
        unresolvable.sort_unstable();
        assert!(
            unresolvable.is_empty(),
            "{} components cannot be compiled under the name a document saves:\n  {}",
            unresolvable.len(),
            unresolvable.join("\n  ")
        );
    }

    /// Every component resolves under its **`name`** -- the type string a front end places.
    ///
    /// The sibling of `every_component_resolves_under_its_own_id`, one field across. That guard
    /// was written for the transformer, whose library spelling and registry spelling disagreed
    /// by one character, and it asks about `id`. A front end does not place by `id`:
    /// `omni-schematics`' `library/insert.ts` builds a block with `type: metadata.name`, and
    /// `loadCatalog` keys the palette by `name`, `type` and every alias -- so a component whose
    /// `name` resolves nowhere inserts happily and leaves a block nothing downstream can
    /// compile.
    ///
    /// Two did. `signal-flow.sink.scope` and `signal-flow.source.step` are the only entries
    /// whose `name` differs from their `id` (underscored where the id is hyphenated), and the
    /// underscored spelling was in none of the three fields `find_component` matches. Measured
    /// on a 0.6.2 wheel rather than reasoned about:
    ///
    /// ```text
    /// signal-flow.sink.scope     OK
    /// signal_flow.sink.scope     FAIL unknown-type: block 'x1' has type
    ///                                 'signal_flow.sink.scope', which no netlist emitter knows
    /// ```
    ///
    /// Fixed by making each `name` an alias of its own component, which is what an alias is for:
    /// a spelling a saved sheet carries. The alternative -- renaming `name` to match `id` --
    /// would orphan every sheet already saved with the underscored type.
    #[test]
    fn every_component_resolves_under_the_name_a_front_end_places_it_by() {
        let mut unresolvable: Vec<&str> = library()
            .iter()
            .filter(|c| crate::compile::spec::lookup(&c.name).is_none())
            .map(|c| c.name.as_str())
            .collect();
        unresolvable.sort_unstable();
        assert!(
            unresolvable.is_empty(),
            "{} component name(s) cannot be compiled, so placing one from a palette leaves a \
             block that fails at Run naming a type the user never typed:\n  {}",
            unresolvable.len(),
            unresolvable.join("\n  ")
        );
    }

    #[test]
    fn a_sheet_saved_under_the_old_spelling_still_finds_its_component() {
        // The library used to describe many components twice -- once under the engine's name
        // and once under the front end's -- with the same ports, symbol and parameters. The
        // duplicates were merged under the engine's name, which is only safe because every
        // spelling the other entry answered to became an alias. Documents on disk carry those
        // spellings, so this is the test that stops the merge from orphaning saved work.
        for old_name in [
            "physical-electrical.passive.resistor",
            "physical_electrical.passive.resistor",
            "resistor",
            "R",
        ] {
            let found =
                find_component(old_name).unwrap_or_else(|| panic!("nothing answers to {old_name}"));
            assert_eq!(
                found.id, "passive.resistor",
                "{old_name} resolved elsewhere"
            );
        }
        assert_eq!(
            find_component("passive.resistor").map(|c| c.id.as_str()),
            Some("passive.resistor")
        );
        assert!(find_component("no.such.component").is_none());
    }

    #[test]
    fn a_main_parameter_names_one_of_the_components_own_parameters() {
        // Merging the duplicate spellings took one entry's parameters and left the other's
        // `main_param` beside them, so two components pointed at a parameter that was no
        // longer there and the front end refused to load the whole catalogue. A main parameter
        // that names nothing is worse than none: an inspector would highlight a field that
        // does not exist.
        let broken: Vec<_> = library()
            .iter()
            .filter_map(|c| {
                let main = c.presentation.get("main_param")?.as_str()?;
                let params = c.presentation.get("params")?.as_array()?;
                let named = params
                    .iter()
                    .filter_map(|p| p.get("name")?.as_str())
                    .any(|n| n == main);
                (!named).then_some((c.id.as_str(), main))
            })
            .collect();
        assert!(
            broken.is_empty(),
            "main_param names no parameter: {broken:?}"
        );
    }

    #[test]
    fn the_catalog_covers_the_components_a_schematic_actually_uses() {
        let catalog = catalog();
        let compilable = catalog.components.iter().filter(|c| c.compilable).count();
        assert!(
            compilable >= 26,
            "only {compilable} components can be compiled"
        );
    }

    #[test]
    fn no_two_components_share_a_label() {
        // The palette shows a component's label and nothing else, so two components wearing one
        // label are indistinguishable at the moment of choosing between them -- found by
        // `omni-schematics`' real-VS-Code sweep clicking all 68 entries, not by reading the
        // data (#83, `omni-schematics#216`).
        //
        // Two pairs shared one: `signal.step` / `signal-flow.source.step` and `sinks.scope` /
        // `signal-flow.sink.scope`. Each pair is one component twice -- the engine's name and
        // the GSF v2 spelling a saved sheet may carry -- and `compile::spec` says so beside
        // both ("Both catalogue spellings are here"). So the label says that, which is the only
        // difference there is to state.
        let mut by_label: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for component in library() {
            let Some(label) = component.label.as_deref() else {
                continue;
            };
            by_label.entry(label).or_default().push(&component.id);
        }
        let shared: Vec<String> = by_label
            .iter()
            .filter(|(_, ids)| ids.len() > 1)
            .map(|(label, ids)| format!("{label:?}: {}", ids.join(", ")))
            .collect();
        assert!(
            shared.is_empty(),
            "{} label(s) are worn by more than one component, so the palette cannot tell them \
             apart:\n  {}",
            shared.len(),
            shared.join("\n  ")
        );
    }

    // --- `port_sigs`: a signal port says what it carries, or nothing says it ----------------
    //
    // `port_sigs` is a per-port *dimension* signature that nothing in this crate reads: it is
    // data this library carries on a front end's behalf, like a symbol or a label, and the
    // editor's signal-dimension overlay (FE-65) derives a wire's width from it. The overlay
    // needs a signature on **every** block along a path, so one missing entry silences the
    // whole sheet -- which is what 45 missing entries did (#64).
    //
    // The closed vocabulary, from the front end's own `block-metadata.schema.json`:
    // `[N]` / `[N,M]` (a literal width), `inherit`, `from(<port>)`, `sum(<port>*)`,
    // `partition(<port>, <param>)`, `shape(<param>)` and `bus`.
    //
    // What makes this worth a guard rather than a bulk fill: a *wrong* signature is worse than
    // a missing one. A missing one draws nothing; a wrong one draws a width the engine never
    // produces, confidently, which is this project's recurring failure mode. So the guard asks
    // two questions -- is a signature present, and is the one present well-formed against the
    // ports and parameters the component actually declares.

    /// Signal ports that deliberately carry **no** signature, with the reason.
    ///
    /// Not a to-do list. Each of these is a width this engine really has and the signature
    /// language really cannot name, so the honest entry is the absent one: the overlay then
    /// draws nothing for that wire instead of drawing a number that is wrong.
    ///
    /// * `control.state_space`, `control.discrete_state_space` -- `TypeSpec::vector_output`:
    ///   these two emit **one vector-valued signal** under the block's own name, and its width
    ///   is the row count of the `c` matrix parameter. The language has `shape(<param>)`, which
    ///   is a matrix's `[rows, cols]`, and no way to say "that matrix's rows". `[1]` would be a
    ///   lie for every state-space block with more than one output.
    /// * `hierarchy.inport`, `hierarchy.outport` -- their single port declares
    ///   `domain_from: "kind"`, so the author picks the domain from five choices and the port
    ///   may not be a signal port at all. A dimension is a signal-domain concept; declaring one
    ///   for a port that may be `physical-electrical` would be declaring it for a port that has
    ///   none.
    /// * `routing.from` -- a From's width is whatever its matched `routing.goto` was handed, and
    ///   the pairing is by the `tag` parameter's *value*, across blocks. No signature can name
    ///   another block's port.
    ///
    /// Not in this list, and deliberately out of the guard's scope: `ideal.switch`'s `g` gate
    /// port, the one signal port on a `physical-electrical` component. The overlay skips a
    /// block whose paradigm is not `signal-flow` before it ever looks at a port, so a signature
    /// there could not be read by the one consumer this field exists for.
    const NO_SIGNATURE_IS_THE_HONEST_ANSWER: &[&str] = &[
        "control.discrete_state_space",
        "control.state_space",
        "hierarchy.inport",
        "hierarchy.outport",
        "routing.from",
    ];

    /// One port, as this guard needs to see it: its id (or `id*` for a variadic group's
    /// template), and whether it is an input.
    struct SignalPort {
        id: String,
        input: bool,
    }

    /// Every signal port `component` declares, variadic groups included as `<template>*`.
    ///
    /// A port carrying `domain_from` is skipped: its domain is a parameter's value, so it is
    /// not knowably a signal port. A port whose `kind.domain` is not `signal` is skipped too --
    /// `converter.sig2phys`'s `out` is `physical-electrical`, and an acausal connection has no
    /// width.
    fn signal_ports(component: &Component) -> Vec<SignalPort> {
        fn is_signal(port: &serde_json::Value) -> bool {
            port.get("domain_from").is_none()
                && port
                    .get("kind")
                    .and_then(|k| k.get("domain"))
                    .and_then(|d| d.as_str())
                    == Some("signal")
        }

        let Some(groups) = component.presentation.get("ports") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (group, input) in [("inputs", true), ("outputs", false)] {
            let Some(list) = groups.get(group).and_then(|g| g.as_array()) else {
                continue;
            };
            for port in list.iter().filter(|p| is_signal(p)) {
                if let Some(id) = port.get("id").and_then(|v| v.as_str()) {
                    out.push(SignalPort {
                        id: id.to_string(),
                        input,
                    });
                }
            }
        }
        for (group, input) in [("variadic_inputs", true), ("variadic_outputs", false)] {
            let Some(template) = groups.get(group).and_then(|g| g.get("template")) else {
                continue;
            };
            if !is_signal(template) {
                continue;
            }
            if let Some(id) = template.get("id").and_then(|v| v.as_str()) {
                // `resolve-ports.ts` names an expanded port `<template.id><index>`, so the only
                // signature that can cover the group is the wildcard.
                out.push(SignalPort {
                    id: format!("{id}*"),
                    input,
                });
            }
        }
        out
    }

    /// Does `key` (a `port_sigs` key, possibly `in*`) cover the port `id` (possibly `in*`)?
    fn covers(key: &str, id: &str) -> bool {
        if key == id {
            return true;
        }
        match key.strip_suffix('*') {
            // `in*` covers `in0`, and covers the variadic group `in*` itself.
            Some(prefix) => id.strip_suffix('*').unwrap_or(id).starts_with(prefix),
            None => false,
        }
    }

    fn port_sigs_of(component: &Component) -> BTreeMap<String, String> {
        component
            .presentation
            .get("port_sigs")
            .and_then(|v| v.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Components whose ports a dimension signature is meaningful for: the signal-domain ones,
    /// and the two `mixed` converters, which have a signal port on one side.
    fn signal_domain_components() -> Vec<&'static Component> {
        library()
            .iter()
            .filter(|c| matches!(c.domain.as_deref(), Some("signal") | Some("mixed")))
            .collect()
    }

    #[test]
    fn every_signal_port_declares_a_dimension_signature() {
        let mut missing: Vec<String> = Vec::new();
        for component in signal_domain_components() {
            if NO_SIGNATURE_IS_THE_HONEST_ANSWER.contains(&component.id.as_str()) {
                continue;
            }
            let sigs = port_sigs_of(component);
            for port in signal_ports(component) {
                if !sigs.keys().any(|key| covers(key, &port.id)) {
                    missing.push(format!("{}:{}", component.id, port.id));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{} signal port(s) carry no dimension signature, so the editor's FE-65 overlay \
             resolves nothing for any sheet they appear on (#64). Give each one a signature, or \
             add the component to `NO_SIGNATURE_IS_THE_HONEST_ANSWER` with the reason:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }

    #[test]
    fn a_declared_signature_is_well_formed_against_the_ports_it_names() {
        // The other half, and the one a bulk fill would have failed: a signature naming a port
        // or a parameter that is not there resolves to nothing at best, and to a width the
        // engine never produces at worst.
        let mut wrong: Vec<String> = Vec::new();
        for component in signal_domain_components() {
            let ports = signal_ports(component);
            let params: Vec<&str> = component
                .presentation
                .get("params")
                .and_then(|p| p.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|p| p.get("name")?.as_str())
                        .collect()
                })
                .unwrap_or_default();
            let names_an_input = |named: &str| {
                ports
                    .iter()
                    .any(|p| p.input && (covers(named, &p.id) || covers(&p.id, named)))
            };

            for (key, sig) in port_sigs_of(component) {
                let (key, sig) = (key.as_str(), sig.as_str());
                let Some(port) = ports.iter().find(|p| covers(key, &p.id)) else {
                    wrong.push(format!(
                        "{}: `port_sigs` key '{key}' matches no signal port",
                        component.id
                    ));
                    continue;
                };
                let complain = |what: &str| format!("{}:{key} = '{sig}' -- {what}", component.id);

                // A literal width: `[3]`, or `[2,3]` for a matrix.
                if let Some(dims) = sig.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    let legal = !dims.is_empty()
                        && dims
                            .split(',')
                            .all(|d| d.trim().parse::<usize>().is_ok_and(|n| n >= 1));
                    if !legal {
                        wrong.push(complain("not a literal width like `[1]` or `[2,3]`"));
                    }
                    continue;
                }

                if sig == "inherit" {
                    // An output declared `inherit` is unresolvable by construction:
                    // `dim-propagate.ts` resolves an output from `from`/`sum`/`partition`/a
                    // literal only, reports the port as unresolved, and returns
                    // `dim-unresolved` for the **whole sheet** -- so one of these silences the
                    // overlay everywhere, not just on its own wire.
                    if !port.input {
                        wrong.push(complain("`inherit` on an output port resolves to nothing"));
                    }
                    continue;
                }

                if port.input {
                    wrong.push(complain(
                        "an input port takes a literal width or `inherit`; the deriving forms \
                         describe an output",
                    ));
                    continue;
                }

                let derived = sig
                    .strip_prefix("from(")
                    .and_then(|s| s.strip_suffix(')'))
                    .map(|p| (p.trim().to_string(), "from"))
                    .or_else(|| {
                        sig.strip_prefix("sum(")
                            .and_then(|s| s.strip_suffix(')'))
                            .map(|p| (p.trim().to_string(), "sum"))
                    })
                    .or_else(|| {
                        sig.strip_prefix("partition(")
                            .and_then(|s| s.strip_suffix(')'))
                            .and_then(|s| s.split_once(','))
                            .map(|(p, _)| (p.trim().to_string(), "partition"))
                    });
                if let Some((named, form)) = derived {
                    if !names_an_input(&named) {
                        wrong.push(complain(&format!(
                            "`{form}` names '{named}', which is not an input port of this \
                             component"
                        )));
                    }
                    continue;
                }

                if let Some(param) = sig.strip_prefix("shape(").and_then(|s| s.strip_suffix(')')) {
                    if !params.contains(&param.trim()) {
                        wrong.push(complain(&format!(
                            "`shape` names the parameter '{}', which this component does not \
                             declare",
                            param.trim()
                        )));
                    }
                    continue;
                }

                if sig == "bus" {
                    continue;
                }
                wrong.push(complain("not a form the signature language has"));
            }
        }
        assert!(
            wrong.is_empty(),
            "{} malformed port signature(s):\n  {}",
            wrong.len(),
            wrong.join("\n  ")
        );
    }
}
