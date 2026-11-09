//! What this engine supports, as data the front end can render.
//!
//! The compile registry in [`super::spec`] already knows, for every type it can emit, which
//! ports become nodes, which block parameters the card consumes, and which of those are
//! required. Until this module existed none of it left the binary, so a GUI had to keep its
//! own copy of the same list — and the copy drifted. It was measured drifting: of 143 types in
//! `general-simulator-gui`'s catalogue, **zero** matched its hand-written "supported" list,
//! because that list spelled types the way this registry does and a schematic spells them the
//! way the catalogue does. Every block on every sheet reported "not supported by the
//! simulator".
//!
//! So the direction is fixed here: the engine declares, the front end renders. A manifest is
//! generated from [`REGISTRY`](super::spec::REGISTRY) rather than written by hand, which is
//! what makes drift impossible rather than merely discouraged — there is no second list to
//! forget.
//!
//! What a consumer gets per type: the dotted type name, whether it emits a SPICE element card
//! (and under which device letter) or a `kind=` block card or nothing at all, the node ports in
//! positional order, the output ports, and every parameter the card reads with its netlist
//! field name and whether it is required.
//!
//! What it deliberately does not carry: labels, units, defaults and widget hints. Those are
//! presentation, they are already in the catalogue's own metadata, and putting them here would
//! make this file the second copy of *that*.

use serde::Serialize;

use super::spec::{ElementTail, FieldSpec, KindName, TypeSpec, REGISTRY};

/// The manifest's own version, so a consumer can refuse one it does not understand.
pub const CAPABILITIES_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub schema_version: u32,
    /// The crate version that produced this.
    pub generator: String,
    pub types: Vec<TypeCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeCapability {
    /// The dotted type a `.gsim_sch` block carries, e.g. `source.voltage.sin`.
    pub r#type: String,
    /// How this type reaches the netlist.
    pub emits: Emits,
    /// Ports that become an element card's positional nodes, in order.
    pub node_ports: Vec<String>,
    /// Output ports in `output_names` order; empty when the count is per-instance.
    pub output_ports: Vec<String>,
    /// True when the output count depends on a parameter rather than the type.
    pub dynamic_outputs: bool,
    /// True when this family emits one vector-valued signal rather than one per output.
    pub vector_output: bool,
    /// Parameters this type reads, in the order the card consumes them.
    pub params: Vec<ParamCapability>,
    /// For a family whose `kind=` is chosen by a parameter, the values that parameter may
    /// take. Empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "card", rename_all = "snake_case")]
pub enum Emits {
    /// A SPICE element card under this device letter, e.g. `V`.
    Element { letter: char },
    /// A `kind=` block card.
    Block { kind: String },
    /// Both — an element card whose real parameters live on an accompanying block card.
    ElementAndBlock { letter: char, kind: String },
    /// Nothing: a ground only names a net, a subckt boundary only marks an interface.
    Nothing,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParamCapability {
    /// The block parameter's own name, as a `.gsim_sch` document spells it.
    pub param: String,
    /// The netlist field or positional role it is rendered into.
    pub field: String,
    pub required: bool,
    /// How the value is rendered, which is what tells a front end what shape to collect.
    pub form: ParamForm,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamForm {
    /// A single scalar.
    Scalar,
    /// A comma-joined list, rendered without brackets.
    List,
    /// The name of another block, resolved through wire adjacency rather than typed.
    Reference,
    /// One of a fixed set of positional fields of a transient source call, e.g. `SIN(...)`.
    SourceFunctionField,
    /// One of a closed set of values, listed in the type's own `choices`.
    Choice,
}

fn param_from_field(field: &FieldSpec, out: &mut Vec<ParamCapability>) {
    match field {
        FieldSpec::Param {
            field,
            param,
            required,
        } => out.push(ParamCapability {
            param: (*param).to_string(),
            field: (*field).to_string(),
            required: *required,
            form: ParamForm::Scalar,
        }),
        FieldSpec::ParamList {
            field,
            param,
            required,
        } => out.push(ParamCapability {
            param: (*param).to_string(),
            field: (*field).to_string(),
            required: *required,
            form: ParamForm::List,
        }),
        // One field rendered from three parameters: a symmetric limit, or the explicit
        // hi/lo pair that overrides it. All three are things a user sets.
        FieldSpec::SymmetricLimit {
            field,
            limit_param,
            hi_param,
            lo_param,
        } => {
            for param in [limit_param, hi_param, lo_param] {
                out.push(ParamCapability {
                    param: (*param).to_string(),
                    field: (*field).to_string(),
                    required: false,
                    form: ParamForm::Scalar,
                });
            }
        }
        // A `Guarded` field is only emitted when its guard parameter says so, but the parameter
        // it wraps is still one a user can set — so the guard is unwrapped rather than skipped.
        FieldSpec::Guarded { inner, .. } => param_from_field(inner, out),
        // Everything else is resolved from the document's own wiring or is a fixed token: a
        // front end has nothing to ask for.
        FieldSpec::Literal { .. }
        | FieldSpec::Ref { .. }
        | FieldSpec::Refs { .. }
        | FieldSpec::RefsIndexed { .. }
        | FieldSpec::PrevRef { .. }
        | FieldSpec::OptionalRef { .. }
        | FieldSpec::Outputs { .. } => {}
    }
}

fn params_of(spec: &TypeSpec) -> Vec<ParamCapability> {
    let mut params = Vec::new();

    // The element card's tail comes first: it is what follows the positional nodes.
    match spec.tail {
        ElementTail::Value(param) => params.push(ParamCapability {
            param: param.to_string(),
            field: "value".to_string(),
            required: true,
            form: ParamForm::Scalar,
        }),
        ElementTail::ControlThenValue { control, gain } => {
            params.push(ParamCapability {
                param: control.to_string(),
                field: "control".to_string(),
                required: true,
                form: ParamForm::Reference,
            });
            params.push(ParamCapability {
                param: gain.to_string(),
                field: "gain".to_string(),
                required: true,
                form: ParamForm::Scalar,
            });
        }
        ElementTail::Sin {
            v0,
            va,
            freq,
            td,
            theta,
            phase,
        } => {
            for (param, field) in [
                (v0, "SIN.v0"),
                (va, "SIN.va"),
                (freq, "SIN.freq"),
                (td, "SIN.td"),
                (theta, "SIN.theta"),
                (phase, "SIN.phase"),
            ] {
                params.push(ParamCapability {
                    param: param.to_string(),
                    field: field.to_string(),
                    required: true,
                    form: ParamForm::SourceFunctionField,
                });
            }
        }
        ElementTail::Pulse {
            v1,
            v2,
            td,
            tr,
            tf,
            pw,
            per,
        } => {
            for (param, field) in [
                (v1, "PULSE.v1"),
                (v2, "PULSE.v2"),
                (td, "PULSE.td"),
                (tr, "PULSE.tr"),
                (tf, "PULSE.tf"),
                (pw, "PULSE.pw"),
                (per, "PULSE.per"),
            ] {
                params.push(ParamCapability {
                    param: param.to_string(),
                    field: field.to_string(),
                    required: true,
                    form: ParamForm::SourceFunctionField,
                });
            }
        }
        // A model token names a `.MODEL` card; the parameters live on the `kind=` card below.
        ElementTail::Model(_) | ElementTail::None => {}
    }

    if let Some(kind) = spec.kind {
        for field in kind.fields {
            param_from_field(field, &mut params);
        }
    }
    params
}

/// The `kind=` string this type emits, and — when a parameter chooses it — that parameter
/// and the values it may take.
///
/// The second case is worth surfacing rather than flattening: a family whose variant is a
/// parameter (`kind=clarke` / `kind=park` / ...) is exactly a closed set of choices, which is
/// what a front end wants to render as a list rather than a free-text field.
fn kind_of(spec: &TypeSpec) -> Option<(String, Option<ParamCapability>, Vec<String>)> {
    let kind = spec.kind?;
    match kind.kind {
        KindName::Fixed(name) => Some((name.to_string(), None, Vec::new())),
        KindName::PerVariant { param, map } => {
            let chooser = ParamCapability {
                param: param.to_string(),
                field: "kind".to_string(),
                required: true,
                form: ParamForm::Choice,
            };
            let first = map
                .first()
                .map(|(_, kind)| (*kind).to_string())
                .unwrap_or_default();
            let choices = map.iter().map(|(value, _)| (*value).to_string()).collect();
            Some((first, Some(chooser), choices))
        }
    }
}

fn emits_of(spec: &TypeSpec, kind: Option<&String>) -> Emits {
    match (spec.device_letter, kind) {
        (Some(letter), Some(kind)) => Emits::ElementAndBlock {
            letter,
            kind: kind.clone(),
        },
        (Some(letter), None) => Emits::Element { letter },
        (None, Some(kind)) => Emits::Block { kind: kind.clone() },
        (None, None) => Emits::Nothing,
    }
}

/// Everything this engine can compile, as data.
pub fn capabilities() -> Capabilities {
    Capabilities {
        schema_version: CAPABILITIES_SCHEMA_VERSION,
        generator: format!("gsim-core {}", env!("CARGO_PKG_VERSION")),
        types: REGISTRY
            .iter()
            .map(|spec| {
                let kind = kind_of(spec);
                let mut params = params_of(spec);
                if let Some((_, Some(chooser), _)) = &kind {
                    params.insert(0, chooser.clone());
                }
                TypeCapability {
                    r#type: spec.type_name.to_string(),
                    emits: emits_of(spec, kind.as_ref().map(|(name, _, _)| name)),
                    node_ports: spec.node_ports.iter().map(|s| (*s).to_string()).collect(),
                    output_ports: spec.output_ports.iter().map(|s| (*s).to_string()).collect(),
                    dynamic_outputs: spec.dynamic_outputs.is_some(),
                    vector_output: spec.vector_output,
                    params,
                    choices: kind.map(|(_, _, c)| c).unwrap_or_default(),
                }
            })
            .collect(),
    }
}

/// [`capabilities`] as JSON — the form every consumer actually wants.
pub fn capabilities_json() -> String {
    serde_json::to_string_pretty(&capabilities())
        .expect("Capabilities is plain data and cannot fail to serialise")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registry_type_is_described() {
        let caps = capabilities();
        assert_eq!(caps.types.len(), REGISTRY.len());
        assert_eq!(caps.schema_version, CAPABILITIES_SCHEMA_VERSION);
    }

    #[test]
    fn a_sine_source_declares_its_six_fields_in_spice_order() {
        // The point of the manifest in one case: a front end reading this knows to collect
        // six numbers, in this order, under these names — without a second copy of the list.
        let caps = capabilities();
        let sin = caps
            .types
            .iter()
            .find(|t| t.r#type == "source.voltage.sin")
            .expect("the registry has a sine source");

        let names: Vec<&str> = sin.params.iter().map(|p| p.param.as_str()).collect();
        assert_eq!(names, ["VO", "VA", "FREQ", "TD", "THETA", "PHASE"]);
        assert!(matches!(sin.emits, Emits::Element { letter: 'V' }));
        assert_eq!(sin.node_ports, ["p", "n"]);
    }

    #[test]
    fn a_structural_type_declares_that_it_emits_nothing() {
        let caps = capabilities();
        let ground = caps
            .types
            .iter()
            .find(|t| t.r#type == "routing.ground")
            .expect("the registry has a ground");
        assert!(matches!(ground.emits, Emits::Nothing));
        assert!(ground.params.is_empty());
    }

    #[test]
    fn a_resistor_declares_the_one_value_it_takes() {
        let caps = capabilities();
        let r = caps
            .types
            .iter()
            .find(|t| t.r#type == "passive.resistor")
            .expect("the registry has a resistor");
        assert_eq!(r.params.len(), 1);
        assert_eq!(r.params[0].param, "R");
        assert!(r.params[0].required);
    }

    #[test]
    fn the_manifest_is_json_a_consumer_can_read() {
        let json = capabilities_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed["schema_version"], CAPABILITIES_SCHEMA_VERSION);
        assert!(parsed["types"].as_array().expect("types is an array").len() > 10);
    }
}
