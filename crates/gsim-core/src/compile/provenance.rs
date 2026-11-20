//! The map between what the user drew and what the netlist calls it — and the check that the
//! map is a map at all.
//!
//! # Why this exists
//!
//! Everything downstream of a run identifies a device by its **netlist name**. An engine event
//! says `target: "DSW"`; a rule card's `target=` says `Rload`; a waveform is `V(n_Rload_n)`.
//! Everything *upstream* identifies it by the **block id** the editor assigned when the user
//! dropped the symbol. A front end showing a rule violation on the schematic has to cross that
//! gap, and it is the compiler that chose the names, so it is the compiler that has to say what
//! it chose. See `general-simulator`'s `book/dev-guide/src/output-contract.md`, of which this
//! map is the first layer.
//!
//! Deriving the map downstream — reimplementing [`spec::element_name`] in the front end — is
//! both the coupling a bridge exists to remove and, as the next section shows, impossible.
//!
//! # Why the check exists
//!
//! [`spec::element_name`] prefixes the device letter onto a block id that does not already
//! start with it. That map is **not injective**: blocks `load` and `Rload` both compile to the
//! element name `Rload`. Before this module, a sheet holding both produced
//!
//! ```text
//! Rload n_Rload_p n_Rload_n 10
//! Rload n_Rload_n 0 20
//! ```
//!
//! with no error and no note — and `general-mna` accepted it, because a duplicate name was
//! rejected there only for the device letters whose name becomes an unknown (fixed in
//! general-mna 0.5.1, `GOTCHA-001`). The *voltages* were right: stamping is positional and
//! never looks a name up. What is lost is every reference **by** name — a rating on the device
//! line, a `kind=rule` card, a `kind=measure`, a controlling branch, a GUI highlight. Each
//! resolves to one of the two devices and nobody can say which.
//!
//! So a name is claimed by exactly one block, and a collision is refused here, where both
//! blocks are visible, rather than left to be discovered by whoever reads a result.

use super::spec::{self};
use super::{general_simulator::x_card_name, CompileContext, CompileError, NameCollision, Scope};
use crate::document::{Endpoint, SubcktDef};
use serde::Serialize;
use std::collections::BTreeMap;

/// What a netlist name belongs to, and which document object produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum CardRole {
    /// One element card — `Rload n1 n2 10`.
    Element,
    /// One `kind=` block card — a signal-domain block.
    Block,
    /// Both cards, under one name. An ideal switch or diode emits an element card *and* a
    /// `kind=` card, and [`spec::card_name`] requires them to share a name: `general-mna` joins
    /// the two by name.
    ElementAndBlock,
    /// One card of a composite block's expansion — a transformer's `ET1_pri`/`FT1_sec`, a
    /// coupled-inductor pair's `LK1_a`. `suffix` is the sub-element's name within its parent.
    Part { suffix: String },
    /// An `X` card instantiating a `.SUBCKT`.
    Instance,
}

/// One netlist name, and the drawn object it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElementOrigin {
    /// The name as the **flattened** deck spells it, which is what a result or an event carries:
    /// `Rload` at the top level, `X1.Rload` for a block inside a subcircuit instance. Flattening
    /// and its dotted prefix are `general-mna`'s (`hierarchy::flatten`), not this compiler's.
    pub element: String,
    /// Block ids from the top-level sheet inward: `["Rload"]`, or `["buck1", "Rload"]` for a
    /// block inside the subcircuit instance `buck1`. The last entry is the block that produced
    /// the card; the ones before it are the instances it sits inside.
    pub blocks: Vec<String>,
    #[serde(flatten)]
    pub role: CardRole,
    /// The `.SUBCKT` card name whose body this came from, or `None` at the top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub def: Option<String>,
}

/// One port on one block, as a document address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortOrigin {
    /// Block ids from the top-level sheet inward, as in [`ElementOrigin::blocks`].
    pub blocks: Vec<String>,
    pub port: String,
}

/// One net, and every drawn port sitting on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetOrigin {
    /// The node name as the flattened deck spells it. `"0"` is the reference node. A net inside
    /// a subcircuit instance is `X1.<name>`, *except* where it reaches a `.SUBCKT` boundary
    /// port, in which case it is the parent's own net at that connection — the substitution the
    /// `X` card performs.
    pub net: String,
    pub ports: Vec<PortOrigin>,
}

/// The whole document-to-netlist map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Provenance {
    /// Every emitted card name, sorted by name.
    pub elements: Vec<ElementOrigin>,
    /// Every net, sorted by name.
    pub nets: Vec<NetOrigin>,
}

impl Provenance {
    /// The block path that produced `element`, if any — the lookup an event-to-schematic bridge
    /// performs.
    pub fn blocks_of(&self, element: &str) -> Option<&[String]> {
        self.elements
            .iter()
            .find(|e| e.element.eq_ignore_ascii_case(element))
            .map(|e| e.blocks.as_slice())
    }
}

/// A name already claimed in this scope, and by what.
struct Claim {
    blocks: Vec<String>,
    role: &'static str,
}

/// Collects the map for a whole model, refusing any name claimed twice.
pub fn build(ctx: &CompileContext<'_>) -> Result<Provenance, CompileError> {
    let mut walker = Walker {
        ctx,
        elements: Vec::new(),
        nets: BTreeMap::new(),
    };
    walker.scope(&ctx.top, None, "", &[], &BTreeMap::new(), &mut Vec::new())?;
    let Walker { elements, nets, .. } = walker;
    let mut elements = elements;
    elements.sort_by(|a, b| a.element.cmp(&b.element));
    Ok(Provenance {
        elements,
        nets: nets
            .into_iter()
            .map(|(net, ports)| NetOrigin { net, ports })
            .collect(),
    })
}

struct Walker<'a, 'm> {
    ctx: &'a CompileContext<'m>,
    elements: Vec<ElementOrigin>,
    nets: BTreeMap<String, Vec<PortOrigin>>,
}

impl Walker<'_, '_> {
    /// Walks one netlist namespace: the top-level sheet, or one `.SUBCKT` body as instantiated
    /// by one `X` card.
    ///
    /// `prefix` is the dotted flattening prefix this namespace's names acquire (`""` at the top,
    /// `"X1."` inside instance `X1`, `"X1.X2."` nested). `path` is the block-id path to the
    /// instance that opened it. `boundary` maps this body's boundary-port net names onto the
    /// parent's own net names — the substitution the `X` card performs. `def_stack` guards
    /// against a def that instantiates itself: the recursion stops there and leaves the real
    /// diagnostic to `general-mna`'s own recursion check, rather than looping forever here.
    fn scope(
        &mut self,
        scope: &Scope<'_>,
        def: Option<&SubcktDef>,
        prefix: &str,
        path: &[String],
        boundary: &BTreeMap<String, String>,
        def_stack: &mut Vec<String>,
    ) -> Result<(), CompileError> {
        let def_name = def.map(|d| d.name.clone());
        // One namespace per scope, element and `kind=` cards together: `general-mna` resolves a
        // `field=` reference and a device name in ways that meet (an ideal switch's two cards
        // share a name by design), so a name shared by two *different* blocks is ambiguous
        // whichever kind of card each one is.
        let mut claimed: BTreeMap<String, Claim> = BTreeMap::new();

        for block in scope.blocks {
            let here: Vec<String> = path.iter().cloned().chain([block.id.clone()]).collect();

            if block.subckt_ref.is_some() {
                let name = x_card_name(&block.id);
                self.claim(
                    &mut claimed,
                    &name,
                    &here,
                    "an X card",
                    def.map(|d| &d.name),
                )?;
                self.elements.push(ElementOrigin {
                    element: format!("{prefix}{name}"),
                    blocks: here.clone(),
                    role: CardRole::Instance,
                    def: def_name.clone(),
                });
                self.record_ports(scope, &block.id, prefix, path, boundary);
                self.instance(
                    block.subckt_ref.as_deref().unwrap_or_default(),
                    &format!("{prefix}{name}."),
                    &here,
                    self.x_card_boundary(
                        scope,
                        block.subckt_ref.as_deref().unwrap_or_default(),
                        &block.id,
                        prefix,
                        boundary,
                    )?,
                    def_stack,
                )?;
                continue;
            }

            let Some(spec) = spec::lookup(&block.r#type) else {
                return Err(CompileError::UnknownType {
                    block: block.id.clone(),
                    type_name: block.r#type.clone(),
                });
            };
            if spec.structural {
                // A ground symbol names a net and emits no card, but its port is still on a net
                // and the front end still wants to find it.
                self.record_ports(scope, &block.id, prefix, path, boundary);
                continue;
            }

            if let Some(parts) = spec.expansion {
                for part in parts {
                    let name = format!("{}{}_{}", part.device_letter, block.id, part.suffix);
                    self.claim(
                        &mut claimed,
                        &name,
                        &here,
                        "part of a composite",
                        def.map(|d| &d.name),
                    )?;
                    self.elements.push(ElementOrigin {
                        element: format!("{prefix}{name}"),
                        blocks: here.clone(),
                        role: CardRole::Part {
                            suffix: part.suffix.to_string(),
                        },
                        def: def_name.clone(),
                    });
                }
            }

            let role = match (spec.device_letter.is_some(), spec.kind.is_some()) {
                (true, true) => Some(CardRole::ElementAndBlock),
                (true, false) => Some(CardRole::Element),
                (false, true) => Some(CardRole::Block),
                (false, false) => None,
            };
            if let Some(role) = role {
                let name = spec::card_name(&block.id, spec);
                self.claim(
                    &mut claimed,
                    &name,
                    &here,
                    role_word(&role),
                    def.map(|d| &d.name),
                )?;
                self.elements.push(ElementOrigin {
                    element: format!("{prefix}{name}"),
                    blocks: here.clone(),
                    role,
                    def: def_name.clone(),
                });
            }

            self.record_ports(scope, &block.id, prefix, path, boundary);
        }

        // `SubcktInstance`s declared directly in a def body: an `X` card whose connections are
        // already names in this body's namespace rather than endpoints to resolve.
        if let Some(def) = def {
            for instance in &def.instances {
                let name = x_card_name(&instance.id);
                let here: Vec<String> = path.iter().cloned().chain([instance.id.clone()]).collect();
                self.claim(&mut claimed, &name, &here, "an X card", Some(&def.name))?;
                self.elements.push(ElementOrigin {
                    element: format!("{prefix}{name}"),
                    blocks: here.clone(),
                    role: CardRole::Instance,
                    def: def_name.clone(),
                });
                let nested = self.nested_boundary(
                    &instance.def_ref,
                    &instance.connections,
                    prefix,
                    boundary,
                )?;
                self.instance(
                    &instance.def_ref,
                    &format!("{prefix}{name}."),
                    &here,
                    nested,
                    def_stack,
                )?;
            }
        }

        Ok(())
    }

    /// Recurses into one `.SUBCKT` body.
    fn instance(
        &mut self,
        def_ref: &str,
        prefix: &str,
        path: &[String],
        boundary: BTreeMap<String, String>,
        def_stack: &mut Vec<String>,
    ) -> Result<(), CompileError> {
        if def_stack.iter().any(|d| d == def_ref) {
            return Ok(()); // recursive def — see `scope`'s doc comment
        }
        let Some(body) = self.ctx.defs.get(def_ref) else {
            return Ok(()); // unknown def: the emitter reports it, with the block's own id
        };
        let def = self
            .ctx
            .model
            .subckt_defs
            .iter()
            .find(|d| d.id == def_ref)
            .cloned();
        def_stack.push(def_ref.to_string());
        let result = self.scope(body, def.as_ref(), prefix, path, &boundary, def_stack);
        def_stack.pop();
        result
    }

    /// The boundary substitution for a top-level-style instance block: for each def port, the
    /// parent-scope net the `X` card connects it to, already flattened.
    fn x_card_boundary(
        &self,
        scope: &Scope<'_>,
        def_ref: &str,
        block_id: &str,
        prefix: &str,
        boundary: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, CompileError> {
        let mut out = BTreeMap::new();
        let Some(def) = self.ctx.model.subckt_defs.iter().find(|d| d.id == def_ref) else {
            return Ok(out);
        };
        for port in &def.ports {
            if !super::nets::is_physical_kind(&port.kind) {
                continue; // a signal port substitutes a block-output name, not a net
            }
            if let Some(net) = scope.nets.net_of(&Endpoint::new(block_id, &port.id)) {
                out.insert(sanitized_port(&port.id), flatten_net(net, prefix, boundary));
            }
        }
        Ok(out)
    }

    /// The boundary substitution for a `SubcktInstance`, whose connections are net *names* in
    /// the enclosing body's namespace, positional against the def's ports.
    fn nested_boundary(
        &self,
        def_ref: &str,
        connections: &[String],
        prefix: &str,
        boundary: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, CompileError> {
        let mut out = BTreeMap::new();
        let Some(def) = self.ctx.model.subckt_defs.iter().find(|d| d.id == def_ref) else {
            return Ok(out);
        };
        for (port, token) in def.ports.iter().zip(connections) {
            if !super::nets::is_physical_kind(&port.kind) {
                continue;
            }
            out.insert(
                sanitized_port(&port.id),
                flatten_net(token, prefix, boundary),
            );
        }
        Ok(out)
    }

    /// Records every net this block's ports sit on.
    fn record_ports(
        &mut self,
        scope: &Scope<'_>,
        block_id: &str,
        prefix: &str,
        path: &[String],
        boundary: &BTreeMap<String, String>,
    ) {
        for (endpoint, net) in scope.nets.entries() {
            if endpoint.block != block_id {
                continue;
            }
            let flattened = flatten_net(net, prefix, boundary);
            self.nets.entry(flattened).or_default().push(PortOrigin {
                blocks: path.iter().cloned().chain([block_id.to_string()]).collect(),
                port: endpoint.port.clone(),
            });
        }
    }

    /// Claims one name in one namespace, or refuses it.
    fn claim(
        &self,
        claimed: &mut BTreeMap<String, Claim>,
        name: &str,
        blocks: &[String],
        role: &'static str,
        def: Option<&String>,
    ) -> Result<(), CompileError> {
        let key = name.to_ascii_uppercase();
        if let Some(previous) = claimed.get(&key) {
            // One block legitimately claims one name twice only through its own element and
            // `kind=` card pair, which `card_name` deliberately gives the same name -- and that
            // pair is recorded as a single `ElementAndBlock` claim, so reaching here means two
            // different document objects.
            return Err(CompileError::DuplicateName(Box::new(NameCollision {
                name: name.to_string(),
                first: previous.blocks.join("."),
                first_role: previous.role.to_string(),
                second: blocks.join("."),
                second_role: role.to_string(),
                def: def.cloned(),
            })));
        }
        claimed.insert(
            key,
            Claim {
                blocks: blocks.to_vec(),
                role,
            },
        );
        Ok(())
    }
}

fn role_word(role: &CardRole) -> &'static str {
    match role {
        CardRole::Element => "an element card",
        CardRole::Block => "a kind= card",
        CardRole::ElementAndBlock => "an element card and its kind= card",
        CardRole::Part { .. } => "part of a composite",
        CardRole::Instance => "an X card",
    }
}

/// A def port's name as the body's own net naming spells it — [`super::nets`] sanitizes a
/// boundary port name into the net-name character set before using it.
fn sanitized_port(port: &str) -> String {
    let mut out: String = port
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.insert(0, 'n');
    }
    out
}

/// One net name as the flattened deck spells it: the reference node everywhere, the parent's own
/// net where this one reaches a boundary port, else prefixed by the instance path.
fn flatten_net(local: &str, prefix: &str, boundary: &BTreeMap<String, String>) -> String {
    if local == super::nets::REFERENCE_NET {
        return local.to_string();
    }
    if let Some(parent) = boundary.get(local) {
        return parent.clone();
    }
    format!("{prefix}{local}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::document::{
        ConnectOptions, ModelBuilder, NewBlockSpec, NewSubcktInstanceSpec, Paradigm, SubcktDef,
        SubcktPort,
    };
    use serde_json::json;

    fn add(b: &mut ModelBuilder, id: &str, t: &str, params: serde_json::Value) {
        add_as(b, id, t, Paradigm::PhysicalNetwork, params)
    }

    fn add_as(
        b: &mut ModelBuilder,
        id: &str,
        t: &str,
        paradigm: Paradigm,
        params: serde_json::Value,
    ) {
        b.add_block(NewBlockSpec {
            r#type: t.to_string(),
            paradigm,
            domain: "electrical".to_string(),
            id: Some(id.to_string()),
            params: params.as_object().cloned().unwrap_or_default(),
            ..Default::default()
        })
        .unwrap();
    }

    fn elec(b: &mut ModelBuilder, fb: &str, fp: &str, tb: &str, tp: &str) {
        b.connect(
            Endpoint::new(fb, fp),
            Endpoint::new(tb, tp),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap();
    }

    fn resistor(b: &mut ModelBuilder, id: &str, r: f64) {
        add(b, id, "passive.resistor", json!({ "R": r }));
    }

    /// The divider that found this bug: `Rload` 10 Ohm in series with `load` 20 Ohm across 10 V.
    /// Both blocks compile to the element name `Rload`.
    fn colliding_divider() -> ModelBuilder {
        let mut b = ModelBuilder::new("m", "M", 10);
        add(&mut b, "Vdc", "source.voltage.dc", json!({"V": 10.0}));
        resistor(&mut b, "Rload", 10.0);
        resistor(&mut b, "load", 20.0);
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "Vdc", "p", "Rload", "p");
        elec(&mut b, "Rload", "n", "load", "p");
        elec(&mut b, "load", "n", "gnd", "p");
        elec(&mut b, "Vdc", "n", "gnd", "p");
        b
    }

    #[test]
    fn two_blocks_that_compile_to_one_element_name_are_refused() {
        // Before this check the sheet compiled to two cards both named `Rload`, with
        // `notes: []`, and solved to the right 6.667 V -- stamping is positional. Every
        // reference *by* name was ambiguous, including the one a front end needs.
        let error = compile(&colliding_divider().into_model(), "general-simulator").unwrap_err();
        let message = error.to_string();
        assert!(
            message.starts_with(
                "netlist name 'Rload' is claimed twice: by 'Rload' as an element card, and by \
                 'load' as an element card -- rename one block."
            ),
            "{message}"
        );
        // The message says *why* two different ids collided, which is the part nobody guesses.
        assert!(
            message.contains("'load' and 'Rload' both compile to 'Rload'"),
            "{message}"
        );
    }

    #[test]
    fn the_same_sheet_with_distinct_names_still_compiles() {
        let mut b = ModelBuilder::new("m", "M", 10);
        add(&mut b, "Vdc", "source.voltage.dc", json!({"V": 10.0}));
        resistor(&mut b, "Rtop", 10.0);
        resistor(&mut b, "Rbot", 20.0);
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "Vdc", "p", "Rtop", "p");
        elec(&mut b, "Rtop", "n", "Rbot", "p");
        elec(&mut b, "Rbot", "n", "gnd", "p");
        elec(&mut b, "Vdc", "n", "gnd", "p");
        let out = compile(&b.into_model(), "general-simulator").unwrap();
        assert!(out.netlist.contains("Rtop "), "{}", out.netlist);
        assert!(out.netlist.contains("Rbot "), "{}", out.netlist);
    }

    #[test]
    fn a_name_is_taken_whatever_its_case() {
        // `general-mna` normalises element names to upper case, so `rload` and `Rload` are one
        // device there. Refusing the pair here is what keeps this compiler's rule the same rule.
        let mut b = ModelBuilder::new("m", "M", 10);
        add(&mut b, "Vdc", "source.voltage.dc", json!({"V": 10.0}));
        resistor(&mut b, "Rload", 10.0);
        resistor(&mut b, "rload", 20.0);
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "Vdc", "p", "Rload", "p");
        elec(&mut b, "Rload", "n", "rload", "p");
        elec(&mut b, "rload", "n", "gnd", "p");
        elec(&mut b, "Vdc", "n", "gnd", "p");
        let message = compile(&b.into_model(), "general-simulator")
            .unwrap_err()
            .to_string();
        assert!(message.contains("claimed twice"), "{message}");
    }

    #[test]
    fn the_provenance_map_names_the_block_behind_every_card() {
        let mut b = ModelBuilder::new("m", "M", 10);
        add(&mut b, "Vdc", "source.voltage.dc", json!({"V": 10.0}));
        resistor(&mut b, "load", 20.0);
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "Vdc", "p", "load", "p");
        elec(&mut b, "load", "n", "gnd", "p");
        elec(&mut b, "Vdc", "n", "gnd", "p");
        let out = compile(&b.into_model(), "general-simulator").unwrap();

        // The prefixed name is the one the netlist, a result file and an event all carry -- and
        // the one a front end cannot derive back to `load` on its own.
        assert_eq!(
            out.provenance.blocks_of("Rload"),
            Some(&["load".to_string()][..])
        );
        assert_eq!(
            out.provenance.blocks_of("Vdc"),
            Some(&["Vdc".to_string()][..])
        );
        // A name nothing emitted is absent rather than guessed at.
        assert_eq!(out.provenance.blocks_of("Rnope"), None);

        let load = out
            .provenance
            .elements
            .iter()
            .find(|e| e.element == "Rload")
            .expect("the resistor is in the map");
        assert_eq!(load.role, CardRole::Element);
        assert_eq!(load.def, None);
    }

    #[test]
    fn a_ground_block_is_in_the_net_map_though_it_emits_no_card() {
        let mut b = ModelBuilder::new("m", "M", 10);
        add(&mut b, "Vdc", "source.voltage.dc", json!({"V": 10.0}));
        resistor(&mut b, "R1", 20.0);
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "Vdc", "p", "R1", "p");
        elec(&mut b, "R1", "n", "gnd", "p");
        elec(&mut b, "Vdc", "n", "gnd", "p");
        let out = compile(&b.into_model(), "general-simulator").unwrap();

        assert!(out.provenance.elements.iter().all(|e| e.element != "gnd"));
        let reference = out
            .provenance
            .nets
            .iter()
            .find(|n| n.net == "0")
            .expect("the reference net is in the map");
        assert!(
            reference
                .ports
                .iter()
                .any(|p| p.blocks == ["gnd"] && p.port == "p"),
            "{reference:?}"
        );
        // And the live node names the ports that sit on it, so a waveform `V(n_R1_p)` can be
        // pointed at the two terminals the user drew.
        let live = out
            .provenance
            .nets
            .iter()
            .find(|n| n.net == "n_R1_p")
            .expect("the live net is in the map");
        let mut ports: Vec<(&[String], &str)> = live
            .ports
            .iter()
            .map(|p| (p.blocks.as_slice(), p.port.as_str()))
            .collect();
        ports.sort();
        assert_eq!(
            ports,
            vec![
                (&["R1".to_string()][..], "p"),
                (&["Vdc".to_string()][..], "p")
            ]
        );
    }

    #[test]
    fn a_switch_claims_one_name_for_both_of_its_cards_and_that_is_not_a_collision() {
        // An ideal switch emits an element card *and* a `kind=` card, which `card_name`
        // deliberately gives one name: `general-mna` joins the two by name, and its diode
        // stamping introduces the symbol `<name>_G` that the `kind=` card has to resolve. A
        // naive "every emitted card claims a name" check would reject every switch ever drawn,
        // so the pair is recorded as the single claim it is.
        let mut b = ModelBuilder::new("m", "M", 10);
        add(
            &mut b,
            "DEV",
            "passive.switch.ideal",
            json!({"r_on": 0.04, "body_g_breakdown": 0.0, "body_v_breakdown": -1e6,
                   "body_g_off": 1e-6, "body_v_f": 0.6, "body_g_on": 100.0}),
        );
        add_as(
            &mut b,
            "GATE",
            "converter.sig2phys",
            Paradigm::Converter,
            json!({"domain": "voltage"}),
        );
        add_as(
            &mut b,
            "MOD",
            "source.const",
            Paradigm::SignalFlow,
            json!({"value": 1.0}),
        );
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "GATE", "out", "DEV", "g");
        elec(&mut b, "DEV", "d", "gnd", "p");
        elec(&mut b, "DEV", "s", "gnd", "p");
        b.connect(
            Endpoint::new("MOD", "out"),
            Endpoint::new("GATE", "in"),
            "signal",
            ConnectOptions::default(),
        )
        .unwrap();

        let out = compile(&b.into_model(), "general-simulator").unwrap();
        let switch: Vec<&ElementOrigin> = out
            .provenance
            .elements
            .iter()
            .filter(|e| e.blocks == ["DEV"])
            .collect();
        assert_eq!(switch.len(), 1, "{switch:?}");
        assert_eq!(switch[0].role, CardRole::ElementAndBlock);
        // A signal-only block is in the map too: a rule can name one as a `target=`, and the
        // front end has to find it on the sheet.
        assert_eq!(
            out.provenance.blocks_of("MOD"),
            Some(&["MOD".to_string()][..])
        );
        let modulator = out
            .provenance
            .elements
            .iter()
            .find(|e| e.element == "MOD")
            .unwrap();
        assert_eq!(modulator.role, CardRole::Block);
    }

    #[test]
    fn a_composites_parts_each_name_the_parent_block_that_produced_them() {
        // An ideal transformer is not one element: it emits an `E` and an `F`, named
        // `ET1_sec`/`FT1_pri`. Both are the block `T1` as far as the user is concerned -- an
        // event on `FT1_pri` has to highlight the transformer symbol, and the suffix is what
        // lets the front end say *which half* of it the engine is talking about.
        let mut b = ModelBuilder::new("m", "M", 10);
        add(&mut b, "T1", "passive.transformer", json!({"n": 3.0}));
        add(&mut b, "gnd", "routing.ground", json!({}));
        for (i, port) in ["p1", "n1", "p2", "n2"].iter().enumerate() {
            let r = format!("R{i}");
            resistor(&mut b, &r, 1.0);
            elec(&mut b, "T1", port, &r, "p");
            elec(&mut b, &r, "n", "gnd", "p");
        }

        let out = compile(&b.into_model(), "general-simulator").unwrap();
        let mut parts: Vec<(&str, &str)> = out
            .provenance
            .elements
            .iter()
            .filter_map(|e| match &e.role {
                CardRole::Part { suffix } => Some((e.element.as_str(), suffix.as_str())),
                _ => None,
            })
            .collect();
        parts.sort();
        assert_eq!(parts, vec![("ET1_sec", "sec"), ("FT1_pri", "pri")]);
        assert_eq!(
            out.provenance.blocks_of("FT1_pri"),
            Some(&["T1".to_string()][..])
        );
        assert_eq!(
            out.provenance.blocks_of("ET1_sec"),
            Some(&["T1".to_string()][..])
        );
    }

    #[test]
    fn a_block_inside_a_subckt_instance_carries_the_dotted_name_the_engine_will_report() {
        // `general-mna`'s `hierarchy::flatten` prefixes a def body's element names with the
        // instance's own card name and a dot, so `X1.Rin` -- not the body-local `Rin` -- is
        // what a result file, an event and a rule `target=` all carry. Two instances of one def
        // are the case that makes the distinction matter: there are two `Rin`s.
        let def = SubcktDef::new("leaf", "LEAF").unwrap().with_ports([
            SubcktPort::new("a", "physical-electrical").unwrap(),
            SubcktPort::new("b", "physical-electrical").unwrap(),
        ]);
        let mut def = def;
        {
            let mut body = ModelBuilder::new("__leaf", "leaf", 10);
            add_as(
                &mut body,
                "leaf",
                "spice-subckt.boundary",
                Paradigm::SpiceSubckt,
                json!({}),
            );
            resistor(&mut body, "in", 1000.0);
            elec(&mut body, "leaf", "a", "in", "p");
            elec(&mut body, "in", "n", "leaf", "b");
            let m = body.into_model();
            def.blocks = m.blocks;
            def.wires = m.wires;
        }
        let mut b = ModelBuilder::new("m", "M", 10);
        b.add_subckt_def(def).unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("X1"))
            .unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("X2"))
            .unwrap();
        add(&mut b, "gnd", "routing.ground", json!({}));
        elec(&mut b, "X1", "b", "X2", "a");
        elec(&mut b, "X1", "a", "gnd", "p");
        elec(&mut b, "X2", "b", "gnd", "p");

        let out = compile(&b.into_model(), "general-simulator").unwrap();
        let names: Vec<&str> = out
            .provenance
            .elements
            .iter()
            .map(|e| e.element.as_str())
            .collect();
        assert_eq!(names, vec!["X1", "X1.Rin", "X2", "X2.Rin"], "{names:?}");

        // Each dotted name resolves to the instance *and* the body block, in that order --
        // which is what a front end needs to open the right subsheet and highlight the right
        // symbol inside it.
        assert_eq!(
            out.provenance.blocks_of("X2.Rin"),
            Some(&["X2".to_string(), "in".to_string()][..])
        );
        let inner = out
            .provenance
            .elements
            .iter()
            .find(|e| e.element == "X2.Rin")
            .unwrap();
        assert_eq!(inner.def.as_deref(), Some("LEAF"));
        assert_eq!(inner.role, CardRole::Element);

        // Net flattening across the boundary: inside the body the resistor sits on the boundary
        // port nets `a` and `b`, and the `X` card substitutes the parent's own nets for them.
        // `X1.Rin`'s p1 is therefore node 0, and both instances' inner resistors meet on the
        // single middle net -- which is exactly what the emitted netlist says.
        assert!(out.netlist.contains("X1 0 n_X1_b LEAF"), "{}", out.netlist);
        let middle = out
            .provenance
            .nets
            .iter()
            .find(|n| n.net == "n_X1_b")
            .expect("the middle net is in the map");
        let mut on_it: Vec<String> = middle
            .ports
            .iter()
            .map(|p| format!("{}.{}", p.blocks.join("."), p.port))
            .collect();
        on_it.sort();
        // `X1.leaf.b` is the def's own boundary symbol, drawn in the subsheet: the net reaches
        // the subsheet connector as well as the resistor terminal, and a front end highlighting
        // this node should light both up.
        assert_eq!(
            on_it,
            vec![
                "X1.b",
                "X1.in.n",
                "X1.leaf.b",
                "X2.a",
                "X2.in.p",
                "X2.leaf.a"
            ],
            "{middle:?}"
        );
    }
}
