//! `SubcktDef`/`SubcktPort`/`SubcktInstance` — true SPICE `.SUBCKT`/`X`-card reuse: define a
//! subcircuit **once**, instantiate it **many** times, each instance with its own distinct
//! external connections.
//!
//! Ported from `general-simulator-gui`'s `packages/gsim-model/src/gsim_model/subckt.py`
//! (read in full before writing this), with two deliberate, documented divergences.
//!
//! # Divergence 1 — structured body, not opaque text
//!
//! `gsim_model.SubcktDef.body` is raw SPICE element-card text: "readable verbatim (e.g. by an
//! AI) but not decomposed into blocks/wires". That is a reasonable choice for *its* purpose
//! (round-tripping hand-authored SPICE that might not be block/wire-representable), but it is
//! the wrong one here: the whole point of this crate's subckt support is **programmatic
//! construction** (`gsim-py`'s `add_block`/`connect`-style API, which cannot emit opaque text)
//! and eventual **visual rendering / GSF v2 validation** (which cannot inspect opaque text).
//! So [`SubcktDef`] carries `blocks: Vec<Block>` + `wires: Vec<Wire>` + `instances:
//! Vec<SubcktInstance>` instead of a `body: String`.
//!
//! This was confirmed viable against a real circuit, not assumed: every element card in
//! the hand-written reference netlist's
//! `sw_leg_fixed`/`bidir_sw_fixed` bodies (`ideal_switch`, `pspwm`, `sig2phys`, plain `R`/`C`)
//! already maps onto an existing GSF v2 fixture or an existing document-model block type.
//! Nothing in a real subckt body actually needs to be opaque.
//!
//! # Divergence 2 — `signal` is an allowed port kind
//!
//! `gsim_model` restricts `SubcktPort.kind` to the four acausal physical-network kinds
//! (`physical-electrical`, `physical-translational`, `physical-rotational`,
//! `physical-thermal`), excluding `signal` and `physical-signal`. That restriction cannot
//! survive contact with the real circuit above: `sw_leg_fixed`'s port list is
//! `d s ctrl_freq ctrl_phase ctrl_duty`, and the three `ctrl_*` nets carry **signal**-kind
//! values (`FSW_CONST`/`PH_DQ1`/`DUTY_ALL`, all `kind=const` signal sources at the top level).
//! Crucially they must stay *external* ports rather than being folded into the def body as
//! internal constant blocks: the four `X_DQ1..4` instances share one def but each is handed a
//! **different** phase (`PH_DQ1`..`PH_DQ4`). Internalising them would mean four near-identical
//! defs — destroying the define-once/instantiate-many property that is this module's entire
//! reason to exist.
//!
//! So `signal` is allowed here. `physical-signal` remains rejected, for the same reason
//! `gsim_model` gives: it is a *converter bus* type (`docs/specs/gsim-sch-format.md` §8), not a
//! real SPICE net kind — a subckt boundary is never a converter.
//!
//! The reference implementation never had to answer this question, because its bodies were
//! opaque text with no structural port-kind checking of any kind.

use super::block::Block;
use super::model::ModelParam;
use super::wire::Wire;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Same pattern `gsim_model.SubcktDef` enforces, and the same one [`super::Block`] uses.
fn id_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_-]*$").unwrap())
}

/// Acausal physical-network kinds, plus `signal` — see this module's "Divergence 2" note for
/// why `signal` is here and `physical-signal` deliberately is not.
pub const SUBCKT_PORT_KINDS: [&str; 5] = [
    "physical-electrical",
    "physical-translational",
    "physical-rotational",
    "physical-thermal",
    "signal",
];

/// The `type` of the pseudo-block standing for a def's own boundary inside its structured body
/// — see [`SubcktDef::boundary_endpoint`].
pub const SUBCKT_BOUNDARY_TYPE: &str = "spice-subckt.boundary";

#[derive(Debug, Clone, PartialEq)]
pub enum SubcktError {
    InvalidDefId(String),
    InvalidPortKind { port: String, kind: String },
    CyclicDefReference { cycle: Vec<String> },
}

impl std::fmt::Display for SubcktError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubcktError::InvalidDefId(id) => {
                write!(
                    f,
                    "SubcktDef id '{id}' does not match ^[A-Za-z][A-Za-z0-9_-]*$"
                )
            }
            SubcktError::InvalidPortKind { port, kind } => write!(
                f,
                "SubcktPort '{port}' kind '{kind}' must be one of {}",
                SUBCKT_PORT_KINDS.join(", ")
            ),
            // Message format matches the Python reference's own
            // (`f"Cyclic SubcktDef reference: {cycle}"`, cycle joined by " -> ").
            SubcktError::CyclicDefReference { cycle } => {
                write!(f, "Cyclic SubcktDef reference: {}", cycle.join(" -> "))
            }
        }
    }
}

impl std::error::Error for SubcktError {}

/// One boundary net of a `.SUBCKT` card, in declaration order (an `X` card's connections are
/// positional, matching this order).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubcktPort {
    pub id: String,
    /// One of [`SUBCKT_PORT_KINDS`]. Validated by [`SubcktPort::new`] — constructing the struct
    /// literally (or deserializing it) does not validate, matching how [`super::Block`]'s own
    /// pattern checks live in `Block::new` rather than in `Deserialize`.
    pub kind: String,
}

impl SubcktPort {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Result<SubcktPort, SubcktError> {
        let id = id.into();
        let kind = kind.into();
        if !SUBCKT_PORT_KINDS.contains(&kind.as_str()) {
            return Err(SubcktError::InvalidPortKind { port: id, kind });
        }
        Ok(SubcktPort { id, kind })
    }
}

/// A structured `X` card *inside* a [`SubcktDef`]'s body: lets one def nest another the way
/// real SPICE subckts nest other subckts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubcktInstance {
    pub id: String,
    pub def_ref: String,
    /// Ordered, positionally matching the referenced def's `ports`.
    #[serde(default)]
    pub connections: Vec<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
}

impl SubcktInstance {
    pub fn new(id: impl Into<String>, def_ref: impl Into<String>) -> SubcktInstance {
        SubcktInstance {
            id: id.into(),
            def_ref: def_ref.into(),
            connections: Vec::new(),
            params: Map::new(),
        }
    }

    pub fn with_connections(
        mut self,
        connections: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.connections = connections.into_iter().map(Into::into).collect();
        self
    }
}

/// Emitted once as `.SUBCKT <name> <ports...> ... .ENDS`; referenced by many `spice-subckt.*`
/// [`super::Block`] instances via their `subckt_ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubcktDef {
    /// Document-internal identity, `^[A-Za-z][A-Za-z0-9_-]*$`.
    pub id: String,
    /// The `.SUBCKT` card's own name. May differ from `id` (e.g. a netlist-facing
    /// `sw_leg_fixed` versus an internal document id).
    pub name: String,
    #[serde(default)]
    pub ports: Vec<SubcktPort>,
    #[serde(default)]
    pub params: Vec<ModelParam>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    #[serde(default)]
    pub instances: Vec<SubcktInstance>,
}

impl SubcktDef {
    /// Validates the id pattern up front, matching `gsim_model.SubcktDef`'s own
    /// `_check_id` model-validator (fail at construction, not deferred to a later pass) and
    /// this crate's own [`super::Block::new`] precedent.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Result<SubcktDef, SubcktError> {
        let id = id.into();
        if !id_pattern().is_match(&id) {
            return Err(SubcktError::InvalidDefId(id));
        }
        Ok(SubcktDef {
            id,
            name: name.into(),
            ports: Vec::new(),
            params: Vec::new(),
            blocks: Vec::new(),
            wires: Vec::new(),
            instances: Vec::new(),
        })
    }

    pub fn with_ports(mut self, ports: impl IntoIterator<Item = SubcktPort>) -> Self {
        self.ports = ports.into_iter().collect();
        self
    }

    pub fn with_instances(mut self, instances: impl IntoIterator<Item = SubcktInstance>) -> Self {
        self.instances = instances.into_iter().collect();
        self
    }

    /// How a wire *inside* this def's body reaches one of the def's own boundary ports.
    ///
    /// A structured body raised a question the opaque-text reference never had to answer: a
    /// `.SUBCKT`'s internal cards name its boundary nets directly (`DEV d s idealswitchmodel`),
    /// but a [`Wire`]'s [`super::Endpoint`] is always `(block, port)` — there is no third
    /// endpoint kind for "the enclosing def's own port". Rather than add one (which every
    /// consumer of `Wire` would then have to handle), the convention is:
    ///
    /// **a def's body contains one boundary block, whose `id` is the def's own `id` and whose
    /// `type` is [`SUBCKT_BOUNDARY_TYPE`], carrying exactly the def's `ports[].id` list.**
    ///
    /// This keeps the body a genuinely self-consistent mini-model — every wire endpoint
    /// resolves to a real block in `blocks`, so nothing in the body is dangling by the ordinary
    /// rules — and needs no new type. The boundary block's paradigm is
    /// [`super::Paradigm::SpiceSubckt`], which bridges paradigms, so it can legally carry both
    /// the `physical-electrical` and the `signal` boundary nets a real def mixes.
    pub fn boundary_endpoint(&self, port_id: impl Into<String>) -> super::Endpoint {
        super::Endpoint::new(self.id.clone(), port_id)
    }

    pub fn port_ids(&self) -> Vec<&str> {
        self.ports.iter().map(|p| p.id.as_str()).collect()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Color {
    White,
    Gray,
    Black,
}

/// Fail-fast check mirroring [`super::BuilderError::CrossParadigm`]'s build-time style (and
/// `gsim_model.check_no_cyclic_defs`, ported faithfully): the `def_ref` graph must be acyclic
/// before any netlist is ever written.
///
/// A **dangling** `def_ref` (naming a def not in `defs`) is deliberately *not* an error here —
/// it is a separate, schema-level concern, exactly as in the Python reference.
pub fn check_no_cyclic_defs(defs: &[SubcktDef]) -> Result<(), SubcktError> {
    let by_id: HashMap<&str, &SubcktDef> = defs.iter().map(|d| (d.id.as_str(), d)).collect();
    let mut color: HashMap<&str, Color> = by_id.keys().map(|k| (*k, Color::White)).collect();

    // Iterative DFS rather than the reference's recursion: a deeply-nested (or maliciously
    // deep) def chain must not blow the Rust stack. Same white/gray/black semantics.
    for &root in by_id.keys() {
        if color[root] != Color::White {
            continue;
        }
        // (def_id, index of the next instance to visit). The chain is the stack's def_ids.
        let mut stack: Vec<(&str, usize)> = vec![(root, 0)];
        color.insert(root, Color::Gray);
        while let Some(&(def_id, next)) = stack.last() {
            let def = by_id[def_id];
            if next >= def.instances.len() {
                color.insert(def_id, Color::Black);
                stack.pop();
                continue;
            }
            stack.last_mut().expect("stack is non-empty here").1 += 1;
            let child = def.instances[next].def_ref.as_str();
            let Some((&child_key, _)) = by_id.get_key_value(child) else {
                continue; // dangling def_ref: a schema-level concern, not a cycle
            };
            match color[child_key] {
                Color::Black => {}
                Color::Gray => {
                    let mut cycle: Vec<String> =
                        stack.iter().map(|(id, _)| (*id).to_string()).collect();
                    cycle.push(child_key.to_string());
                    return Err(SubcktError::CyclicDefReference { cycle });
                }
                Color::White => {
                    color.insert(child_key, Color::Gray);
                    stack.push((child_key, 0));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, refs: &[&str]) -> SubcktDef {
        SubcktDef::new(id, id.to_uppercase())
            .unwrap()
            .with_instances(
                refs.iter()
                    .enumerate()
                    .map(|(i, r)| SubcktInstance::new(format!("X{i}"), *r)),
            )
    }

    #[test]
    fn rejects_a_non_physical_non_signal_port_kind() {
        // Matches the Python reference's `test_non_physical_port_kind_rejected`, adjusted for
        // this crate's deliberate `signal` widening (see the module doc).
        assert!(matches!(
            SubcktPort::new("bus", "physical-signal"),
            Err(SubcktError::InvalidPortKind { .. })
        ));
        assert!(matches!(
            SubcktPort::new("x", "not-a-kind"),
            Err(SubcktError::InvalidPortKind { .. })
        ));
        assert!(SubcktPort::new("ctrl_freq", "signal").is_ok());
        assert!(SubcktPort::new("d", "physical-electrical").is_ok());
    }

    #[test]
    fn rejects_an_invalid_def_id() {
        assert!(matches!(
            SubcktDef::new("1bad", "BAD"),
            Err(SubcktError::InvalidDefId(_))
        ));
        assert!(SubcktDef::new("sw_leg_fixed", "sw_leg_fixed").is_ok());
    }

    #[test]
    fn accepts_an_acyclic_nesting_chain() {
        // Mirrors the reference's `test_nested_subckt_defs_compose`.
        let defs = vec![def("leaf", &[]), def("composite", &["leaf"])];
        assert!(check_no_cyclic_defs(&defs).is_ok());
    }

    #[test]
    fn rejects_a_self_reference() {
        // Mirrors `test_self_referencing_def_rejected`.
        let defs = vec![def("loopy", &["loopy"])];
        let err = check_no_cyclic_defs(&defs).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Cyclic SubcktDef reference: loopy -> loopy"
        );
    }

    #[test]
    fn rejects_a_two_cycle_and_names_it() {
        // Mirrors `test_mutual_cycle_rejected`, and additionally asserts the reference's exact
        // "a -> b -> a" message format is reproduced.
        let defs = vec![def("a", &["b"]), def("b", &["a"])];
        let err = check_no_cyclic_defs(&defs).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg == "Cyclic SubcktDef reference: a -> b -> a"
                || msg == "Cyclic SubcktDef reference: b -> a -> b",
            "unexpected cycle message: {msg}"
        );
    }

    #[test]
    fn rejects_a_three_cycle() {
        let defs = vec![def("a", &["b"]), def("b", &["c"]), def("c", &["a"])];
        assert!(matches!(
            check_no_cyclic_defs(&defs),
            Err(SubcktError::CyclicDefReference { .. })
        ));
    }

    #[test]
    fn a_dangling_def_ref_is_not_a_cycle() {
        // Faithful to the reference: "dangling def_ref is a separate (schema-level) concern".
        let defs = vec![def("a", &["ghost"])];
        assert!(check_no_cyclic_defs(&defs).is_ok());
    }

    #[test]
    fn a_diamond_is_not_a_cycle() {
        // A def reachable by two distinct paths must not be mistaken for a cycle -- this is
        // exactly what the black-colouring (not just gray) exists to prevent.
        let defs = vec![
            def("top", &["left", "right"]),
            def("left", &["leaf"]),
            def("right", &["leaf"]),
            def("leaf", &[]),
        ];
        assert!(check_no_cyclic_defs(&defs).is_ok());
    }

    #[test]
    fn boundary_endpoint_addresses_the_def_by_its_own_id() {
        let d = SubcktDef::new("sw_leg_fixed", "sw_leg_fixed")
            .unwrap()
            .with_ports([SubcktPort::new("d", "physical-electrical").unwrap()]);
        let ep = d.boundary_endpoint("d");
        assert_eq!(ep.block, "sw_leg_fixed");
        assert_eq!(ep.port, "d");
    }

    #[test]
    fn round_trips_a_def_through_json() {
        let d = SubcktDef::new("leaf", "LEAF").unwrap().with_ports([
            SubcktPort::new("a", "physical-electrical").unwrap(),
            SubcktPort::new("ctrl", "signal").unwrap(),
        ]);
        let text = serde_json::to_string(&d).unwrap();
        let back: SubcktDef = serde_json::from_str(&text).unwrap();
        assert_eq!(back.port_ids(), vec!["a", "ctrl"]);
        assert_eq!(back.name, "LEAF");
    }
}
