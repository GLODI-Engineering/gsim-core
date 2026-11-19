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

/// [`catalog`] as JSON.
pub fn catalog_json() -> String {
    serde_json::to_string_pretty(&catalog()).expect("Catalog serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_library_parses() {
        assert!(
            library().len() > 200,
            "expected the whole library, got {}",
            library().len()
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
        let catalog = catalog();
        let resistor = catalog
            .components
            .iter()
            .find(|c| c.component.id == "passive.resistor")
            .expect("resistor");
        assert!(resistor.compilable);

        // A MOSFET is in the library and has no compile registry entry, so it is shown and
        // honestly marked rather than hidden.
        let mosfet = catalog
            .components
            .iter()
            .find(|c| c.component.id.contains("mosfet"))
            .expect("the library knows about MOSFETs");
        assert!(
            !mosfet.compilable,
            "{} claims to compile",
            mosfet.component.id
        );
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
}
