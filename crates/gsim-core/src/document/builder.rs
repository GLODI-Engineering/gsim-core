//! `ModelBuilder` — the "code-equivalent-of-the-GUI" construction API, matching
//! `packages/gsim-model`'s real `ModelBuilder` surface (audited before writing this: explicit
//! position/rotation/mirror on `add_block`, `connect()` validating both endpoints exist,
//! cascading wire cleanup on `remove_block`) plus one deliberate improvement: `connect()` now
//! actually enforces the cross-paradigm boundary (`docs/specs/gsim-sch-format.md` §8.3) via
//! [`BuilderError::CrossParadigm`] — `gsim_model` only ever *referenced* a
//! `CrossParadigmWireError` in comments; it was never implemented there.

use super::block::{Block, BlockError, Mirror, NewBlock};
use super::model::Model;
use super::paradigm::Paradigm;
use super::port_compat::{self, PortIncompat};
use super::subckt::{check_no_cyclic_defs, SubcktDef, SubcktError};
use super::wire::{Endpoint, Wire};
use serde::Deserialize;
use serde_json::{Map, Value};

/// Every `spice-subckt` instance block gets this `type`, matching the reference
/// implementation's own `gsim_model.builder.instantiate_subckt`.
pub const SUBCKT_INSTANCE_TYPE: &str = "spice-subckt.instance";

#[derive(Debug, Clone, PartialEq)]
pub enum BuilderError {
    DuplicateId(String),
    DanglingWire {
        block: String,
    },
    CrossParadigm {
        from: Paradigm,
        to: Paradigm,
    },
    /// The finer-grained §8.1/§8.2 port-kind rule, below [`BuilderError::CrossParadigm`]'s
    /// block-level paradigm boundary — see [`super::port_compat`], which also documents exactly
    /// which of `port-compat.ts`'s rules this covers and which still need a contract registry.
    /// Boxed to keep `BuilderError` small — see [`PortIncompat`].
    PortIncompatible(Box<PortIncompat>),
    UnknownBlock(String),
    Block(BlockError),
    /// `instantiate_subckt` (or a `SubcktInstance`) named a `def_ref` with no matching
    /// [`SubcktDef`] in the model — the reference implementation's `UnknownSubcktDefError`.
    UnknownSubcktDef(String),
    /// An invalid def id / port kind, or a cyclic `def_ref` graph — see [`SubcktError`], whose
    /// `Display` names the actual cycle (`"a -> b -> a"`).
    Subckt(SubcktError),
}

impl From<BlockError> for BuilderError {
    fn from(e: BlockError) -> Self {
        BuilderError::Block(e)
    }
}

impl From<SubcktError> for BuilderError {
    fn from(e: SubcktError) -> Self {
        BuilderError::Subckt(e)
    }
}

impl std::fmt::Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuilderError::DuplicateId(id) => write!(f, "duplicate id '{id}'"),
            BuilderError::DanglingWire { block } => {
                write!(f, "wire references unknown block '{block}'")
            }
            BuilderError::CrossParadigm { from, to } => write!(
                f,
                "cannot connect a {from:?} block directly to a {to:?} block -- route through a converter/routing/hierarchy/annotation block"
            ),
            BuilderError::PortIncompatible(inc) => write!(f, "{inc}"),
            BuilderError::UnknownBlock(id) => write!(f, "no block with id '{id}'"),
            BuilderError::Block(e) => write!(f, "{e}"),
            BuilderError::UnknownSubcktDef(id) => write!(f, "no SubcktDef with id '{id}'"),
            BuilderError::Subckt(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BuilderError {}

/// Deserializable so `gsim-py`/`gsim-wasm` can parse a JSON spec straight into this type instead
/// of hand-rolling an equivalent shadow struct per binding (that would be exactly the
/// "reimplementing the shape" duplication `AGENTS.md`'s "one implementation, thin bindings" rule
/// warns about). `type`/`paradigm`/`domain`/`position`/`rotation` are required in the JSON; the
/// rest default the same way [`NewBlockSpec::default`] does.
#[derive(Deserialize)]
pub struct NewBlockSpec {
    #[serde(rename = "type")]
    pub r#type: String,
    pub paradigm: Paradigm,
    pub domain: String,
    #[serde(default)]
    pub fidelity: Option<String>,
    pub position: (i64, i64),
    pub rotation: i64,
    #[serde(default)]
    pub mirror: Mirror,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub ic: Map<String, Value>,
    #[serde(default)]
    pub label: Option<String>,
    /// Comment lines emitted above this block's card(s) by a [`crate::compile::NetlistTarget`] —
    /// see [`Block::comment_before`]. One entry per line; a single multi-line string works too.
    #[serde(default, deserialize_with = "super::block::deserialize_comment")]
    pub comment_before: Vec<String>,
    /// Comment lines emitted below this block's card(s) — see [`Block::comment_after`].
    #[serde(default, deserialize_with = "super::block::deserialize_comment")]
    pub comment_after: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
}

impl Default for NewBlockSpec {
    fn default() -> Self {
        NewBlockSpec {
            r#type: String::new(),
            paradigm: Paradigm::SignalFlow,
            domain: String::new(),
            fidelity: None,
            position: (0, 0),
            rotation: 0,
            mirror: Mirror::None,
            params: Map::new(),
            ic: Map::new(),
            label: None,
            comment_before: Vec::new(),
            comment_after: Vec::new(),
            id: None,
        }
    }
}

/// A subckt instance's `domain` when the caller doesn't name one: the def's own port kinds
/// decide it. A def whose ports are all one physical domain gets that domain's short name; a
/// def mixing domains (or mixing `signal` with a physical kind, as every real gate-driven
/// switch subckt does) gets `"mixed"`, since no single domain is truthful for it.
fn default_subckt_domain(def: &SubcktDef) -> String {
    let mut kinds: Vec<&str> = def.ports.iter().map(|p| p.kind.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    match kinds.as_slice() {
        [one] => one.strip_prefix("physical-").unwrap_or(one).to_string(),
        _ => "mixed".to_string(),
    }
}

/// The `instantiate_subckt` input, mirroring [`NewBlockSpec`]'s shape/ergonomics exactly, minus
/// the fields a subckt instance doesn't get to choose (`type` and `paradigm` are fixed to
/// [`SUBCKT_INSTANCE_TYPE`]/[`Paradigm::SpiceSubckt`]; `ic` is meaningless on a boundary block —
/// initial conditions belong to the real components inside the def's body).
#[derive(Deserialize)]
pub struct NewSubcktInstanceSpec {
    pub def_ref: String,
    pub position: (i64, i64),
    #[serde(default)]
    pub rotation: i64,
    /// Defaults from the referenced def's own port kinds — see [`default_subckt_domain`].
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub fidelity: Option<String>,
    #[serde(default)]
    pub mirror: Mirror,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub label: Option<String>,
    /// Comment lines emitted above this instance's `X` card — see [`Block::comment_before`].
    #[serde(default, deserialize_with = "super::block::deserialize_comment")]
    pub comment_before: Vec<String>,
    /// Comment lines emitted below this instance's `X` card — see [`Block::comment_after`].
    #[serde(default, deserialize_with = "super::block::deserialize_comment")]
    pub comment_after: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
}

impl NewSubcktInstanceSpec {
    pub fn new(def_ref: impl Into<String>, position: (i64, i64)) -> NewSubcktInstanceSpec {
        NewSubcktInstanceSpec {
            def_ref: def_ref.into(),
            position,
            rotation: 0,
            domain: None,
            fidelity: None,
            mirror: Mirror::None,
            params: Map::new(),
            label: None,
            comment_before: Vec::new(),
            comment_after: Vec::new(),
            id: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

/// Deserializable for the same reason [`NewBlockSpec`] is -- every field is optional, matching
/// `ConnectOptions::default()`, so `"{}"` is a valid JSON payload.
#[derive(Default, Deserialize)]
pub struct ConnectOptions {
    #[serde(default)]
    pub waypoints: Vec<(i64, i64)>,
    #[serde(default)]
    pub net: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub label: Option<String>,
}

pub struct ModelBuilder {
    model: Model,
    next_block_seq: u64,
    next_wire_seq: u64,
}

impl ModelBuilder {
    pub fn new(id: impl Into<String>, name: impl Into<String>, grid: i64) -> ModelBuilder {
        ModelBuilder {
            model: Model::new(id, name, grid),
            next_block_seq: 1,
            next_wire_seq: 1,
        }
    }

    pub fn from_model(model: Model) -> ModelBuilder {
        ModelBuilder {
            model,
            next_block_seq: 1,
            next_wire_seq: 1,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn into_model(self) -> Model {
        self.model
    }

    pub fn get_block(&self, id: &str) -> Option<&Block> {
        self.model.blocks.iter().find(|b| b.id == id)
    }

    fn has_block_id(&self, id: &str) -> bool {
        self.model.blocks.iter().any(|b| b.id == id)
    }

    fn has_wire_id(&self, id: &str) -> bool {
        self.model.wires.iter().any(|w| w.id == id)
    }

    fn auto_block_id(&mut self) -> String {
        loop {
            let candidate = format!("block{}", self.next_block_seq);
            self.next_block_seq += 1;
            if !self.has_block_id(&candidate) {
                return candidate;
            }
        }
    }

    fn auto_wire_id(&mut self) -> String {
        loop {
            let candidate = format!("wire{}", self.next_wire_seq);
            self.next_wire_seq += 1;
            if !self.has_wire_id(&candidate) {
                return candidate;
            }
        }
    }

    pub fn add_block(&mut self, spec: NewBlockSpec) -> Result<String, BuilderError> {
        let id = match spec.id {
            Some(id) => {
                if self.has_block_id(&id) {
                    return Err(BuilderError::DuplicateId(id));
                }
                id
            }
            None => self.auto_block_id(),
        };
        let block = Block::new(NewBlock {
            id: id.clone(),
            r#type: spec.r#type,
            paradigm: spec.paradigm,
            domain: spec.domain,
            fidelity: spec.fidelity,
            position: spec.position,
            rotation: spec.rotation,
            mirror: spec.mirror,
            params: spec.params,
            ic: spec.ic,
            label: spec.label,
            comment_before: spec.comment_before,
            comment_after: spec.comment_after,
            // `add_block` never creates a subckt instance -- `instantiate_subckt` is the only
            // path that sets this, so the "Some(def_id) exactly when paradigm == SpiceSubckt"
            // invariant has exactly one place to be established.
            subckt_ref: None,
        })?;
        self.model.blocks.push(block);
        Ok(id)
    }

    // -- subckt defs / instances --------------------------------------------

    pub fn get_subckt_def(&self, id: &str) -> Option<&SubcktDef> {
        self.model.subckt_defs.iter().find(|d| d.id == id)
    }

    /// Appends a reusable [`SubcktDef`], re-validating the *whole* def list's `def_ref` graph
    /// afterwards.
    ///
    /// **Fail before mutating state**, matching the Python reference's own stated philosophy and
    /// this crate's [`BuilderError::CrossParadigm`] precedent: the id pattern, port kinds and
    /// id-uniqueness are checked before the push, and if the cycle check (which can only run
    /// against the *complete* updated list) then fails, the push is rolled back — the model is
    /// never left in a construction-failed-but-mutated state.
    pub fn add_subckt_def(&mut self, def: SubcktDef) -> Result<String, BuilderError> {
        // Re-validate rather than trusting the caller: a `SubcktDef` can also be built by
        // struct literal or `serde` deserialization, both of which bypass `SubcktDef::new`.
        let checked = SubcktDef::new(&def.id, &def.name)?;
        for port in &def.ports {
            super::subckt::SubcktPort::new(&port.id, &port.kind)?;
        }
        let id = checked.id;
        if self.get_subckt_def(&id).is_some() {
            return Err(BuilderError::DuplicateId(id));
        }
        self.model.subckt_defs.push(def);
        if let Err(e) = check_no_cyclic_defs(&self.model.subckt_defs) {
            self.model.subckt_defs.pop();
            return Err(BuilderError::Subckt(e));
        }
        Ok(id)
    }

    /// Creates a top-level `spice-subckt` [`Block`] instantiating an existing [`SubcktDef`].
    ///
    /// The resulting block's wireable ports are exactly the referenced def's `ports[].id` list;
    /// ordinary [`ModelBuilder::connect`] calls wire them, unchanged. (`connect()` still does
    /// not look ports up against any catalogue — that property is deliberately preserved; the
    /// def-existence check lives here instead, where the def list is actually in hand.)
    pub fn instantiate_subckt(
        &mut self,
        spec: NewSubcktInstanceSpec,
    ) -> Result<String, BuilderError> {
        let def = self
            .get_subckt_def(&spec.def_ref)
            .ok_or_else(|| BuilderError::UnknownSubcktDef(spec.def_ref.clone()))?;
        let domain = spec
            .domain
            .clone()
            .unwrap_or_else(|| default_subckt_domain(def));

        let id = match spec.id {
            Some(id) => {
                if self.has_block_id(&id) {
                    return Err(BuilderError::DuplicateId(id));
                }
                id
            }
            None => self.auto_block_id(),
        };
        let block = Block::new(NewBlock {
            id: id.clone(),
            r#type: SUBCKT_INSTANCE_TYPE.to_string(),
            paradigm: Paradigm::SpiceSubckt,
            domain,
            fidelity: spec.fidelity,
            position: spec.position,
            rotation: spec.rotation,
            mirror: spec.mirror,
            params: spec.params,
            ic: Map::new(),
            label: spec.label,
            comment_before: spec.comment_before,
            comment_after: spec.comment_after,
            subckt_ref: Some(spec.def_ref),
        })?;
        self.model.blocks.push(block);
        Ok(id)
    }

    /// Removes a block and cascades to every wire touching it — matching
    /// `gsim_model.builder.ModelBuilder.remove_block`'s prevention-not-detection approach:
    /// there is never a state where a dangling wire from a removed block can exist.
    pub fn remove_block(&mut self, block_id: &str) {
        self.model.blocks.retain(|b| b.id != block_id);
        self.model
            .wires
            .retain(|w| w.from.block != block_id && w.to.block != block_id);
    }

    pub fn set_param(
        &mut self,
        block_id: &str,
        name: &str,
        value: Value,
    ) -> Result<(), BuilderError> {
        let block = self
            .model
            .blocks
            .iter_mut()
            .find(|b| b.id == block_id)
            .ok_or_else(|| BuilderError::UnknownBlock(block_id.to_string()))?;
        block.params.insert(name.to_string(), value);
        Ok(())
    }

    pub fn add_model_param(
        &mut self,
        name: impl Into<String>,
        value: Value,
    ) -> Result<(), BuilderError> {
        let name = name.into();
        if self.model.params.iter().any(|p| p.name == name) {
            return Err(BuilderError::DuplicateId(name));
        }
        self.model.params.push(super::model::ModelParam {
            name,
            value,
            unit: None,
            doc: None,
        });
        Ok(())
    }

    /// Arranges every block via [`crate::layout::layered_layout`] using wire connectivity, so a
    /// caller building a schematic entirely in code doesn't have to hand-pick every coordinate.
    /// `bbox_of` supplies each block's `(w, h)` — this module doesn't depend on `crate::contract`
    /// (GSF v2) to stay decoupled from rendering concerns, so the caller (which does have the
    /// component contracts) provides sizes rather than this method looking them up itself.
    pub fn auto_arrange(
        &mut self,
        bbox_of: impl Fn(&Block) -> (i64, i64),
        column_gap: i64,
        row_gap: i64,
    ) {
        let grid = self.model.grid;
        let positions = {
            let inputs: Vec<crate::layout::LayoutInput> = self
                .model
                .blocks
                .iter()
                .map(|b| crate::layout::LayoutInput {
                    id: &b.id,
                    size: bbox_of(b),
                })
                .collect();
            let edges: Vec<(&str, &str)> = self
                .model
                .wires
                .iter()
                .map(|w| (w.from.block.as_str(), w.to.block.as_str()))
                .collect();
            crate::layout::layered_layout(&inputs, &edges, column_gap, row_gap, grid)
        };
        for block in &mut self.model.blocks {
            if let Some(&pos) = positions.get(&block.id) {
                block.position = pos;
            }
        }
    }

    /// Validates both endpoints exist, that the two blocks' paradigms are connectable (same
    /// paradigm, or one side bridges — see [`Paradigm::bridges_paradigms`]), and then the finer
    /// per-port rules in [`super::port_compat`], before appending the wire.
    ///
    /// `kind` is still the caller's assertion of what the two ports are (§8.4). There is no
    /// component-contract registry in this crate for `connect()` to verify that assertion in
    /// full — [`super::port_compat`]'s module doc records exactly which of the frontend's
    /// `port-compat.ts` rules are therefore enforced here and which remain open, and why
    /// plumbing a catalogue into this builder was rejected rather than merely skipped.
    pub fn connect(
        &mut self,
        from: Endpoint,
        to: Endpoint,
        kind: impl Into<String>,
        opts: ConnectOptions,
    ) -> Result<&str, BuilderError> {
        let from_block = self
            .get_block(&from.block)
            .ok_or_else(|| BuilderError::DanglingWire {
                block: from.block.clone(),
            })?;
        let to_block = self
            .get_block(&to.block)
            .ok_or_else(|| BuilderError::DanglingWire {
                block: to.block.clone(),
            })?;

        let connectable = from_block.paradigm == to_block.paradigm
            || from_block.paradigm.bridges_paradigms()
            || to_block.paradigm.bridges_paradigms();
        if !connectable {
            return Err(BuilderError::CrossParadigm {
                from: from_block.paradigm,
                to: to_block.paradigm,
            });
        }

        let kind = kind.into();
        if let Err(failure) =
            port_compat::check(&self.model, from_block, to_block, &from, &to, &kind)
        {
            return Err(BuilderError::PortIncompatible(Box::new(PortIncompat {
                from,
                to,
                kind,
                failure,
            })));
        }

        let id = match opts.id {
            Some(id) => {
                if self.has_wire_id(&id) {
                    return Err(BuilderError::DuplicateId(id));
                }
                id
            }
            None => self.auto_wire_id(),
        };

        self.model.wires.push(Wire {
            id: id.clone(),
            from,
            to,
            kind,
            waypoints: opts.waypoints,
            pinned: opts.pinned,
            net: opts.net,
            label: opts.label,
        });
        // Borrow-check note: return the id, not a &Wire -- the caller already has the id and can
        // look the wire back up via `model().wires` if needed; avoids a self-referential borrow.
        Ok(self.model.wires.last().expect("just pushed").id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::super::subckt::{SubcktInstance, SubcktPort};
    use super::*;

    fn block(builder: &mut ModelBuilder, id: &str, paradigm: Paradigm, r#type: &str) {
        builder
            .add_block(NewBlockSpec {
                r#type: r#type.to_string(),
                paradigm,
                domain: "electrical".to_string(),
                id: Some(id.to_string()),
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn add_block_rejects_duplicate_explicit_ids() {
        let mut b = ModelBuilder::new("m", "M", 10);
        block(&mut b, "r1", Paradigm::PhysicalNetwork, "passive.resistor");
        let err = b.add_block(NewBlockSpec {
            r#type: "passive.resistor".to_string(),
            paradigm: Paradigm::PhysicalNetwork,
            domain: "electrical".to_string(),
            id: Some("r1".to_string()),
            ..Default::default()
        });
        assert_eq!(err, Err(BuilderError::DuplicateId("r1".to_string())));
    }

    #[test]
    fn auto_generated_ids_never_collide_with_explicit_ones() {
        let mut b = ModelBuilder::new("m", "M", 10);
        block(
            &mut b,
            "block1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
        );
        let auto_id = b
            .add_block(NewBlockSpec {
                r#type: "passive.capacitor".to_string(),
                paradigm: Paradigm::PhysicalNetwork,
                domain: "electrical".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_ne!(auto_id, "block1");
    }

    #[test]
    fn connect_rejects_a_dangling_endpoint() {
        let mut b = ModelBuilder::new("m", "M", 10);
        block(&mut b, "r1", Paradigm::PhysicalNetwork, "passive.resistor");
        let err = b.connect(
            Endpoint::new("r1", "p"),
            Endpoint::new("ghost", "p"),
            "physical-electrical",
            ConnectOptions::default(),
        );
        assert_eq!(
            err,
            Err(BuilderError::DanglingWire {
                block: "ghost".to_string()
            })
        );
    }

    #[test]
    fn connect_rejects_same_paradigm_wires_that_differ_without_a_bridge() {
        let mut b = ModelBuilder::new("m", "M", 10);
        block(&mut b, "r1", Paradigm::PhysicalNetwork, "passive.resistor");
        block(&mut b, "gain1", Paradigm::SignalFlow, "control.gain");
        let err = b.connect(
            Endpoint::new("r1", "p"),
            Endpoint::new("gain1", "in"),
            "signal",
            ConnectOptions::default(),
        );
        assert_eq!(
            err,
            Err(BuilderError::CrossParadigm {
                from: Paradigm::PhysicalNetwork,
                to: Paradigm::SignalFlow,
            })
        );
    }

    #[test]
    fn connect_allows_a_converter_block_to_bridge_paradigms() {
        let mut b = ModelBuilder::new("m", "M", 10);
        block(&mut b, "r1", Paradigm::PhysicalNetwork, "passive.resistor");
        block(
            &mut b,
            "conv1",
            Paradigm::Converter,
            "converter.signal-to-physical-electrical",
        );
        block(&mut b, "gain1", Paradigm::SignalFlow, "control.gain");
        b.connect(
            Endpoint::new("r1", "p"),
            Endpoint::new("conv1", "in"),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap();
        b.connect(
            Endpoint::new("conv1", "out"),
            Endpoint::new("gain1", "in"),
            "signal",
            ConnectOptions::default(),
        )
        .unwrap();
        assert_eq!(b.model().wires.len(), 2);
    }

    #[test]
    fn remove_block_cascades_wire_deletion() {
        let mut b = ModelBuilder::new("m", "M", 10);
        block(&mut b, "r1", Paradigm::PhysicalNetwork, "passive.resistor");
        block(&mut b, "c1", Paradigm::PhysicalNetwork, "passive.capacitor");
        b.connect(
            Endpoint::new("r1", "n"),
            Endpoint::new("c1", "p"),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap();
        assert_eq!(b.model().wires.len(), 1);
        b.remove_block("r1");
        assert!(b.get_block("r1").is_none());
        assert_eq!(b.model().wires.len(), 0);
    }

    #[test]
    fn set_param_on_unknown_block_errors() {
        let mut b = ModelBuilder::new("m", "M", 10);
        let err = b.set_param("ghost", "R", Value::from(1000));
        assert_eq!(err, Err(BuilderError::UnknownBlock("ghost".to_string())));
    }

    #[test]
    fn auto_arrange_places_a_wired_chain_left_to_right_without_hand_picked_coordinates() {
        let mut b = ModelBuilder::new("m", "M", 10);
        // Added out of visual order on purpose -- auto_arrange must not just trust insertion
        // order, only wire connectivity.
        block(&mut b, "gain1", Paradigm::SignalFlow, "control.gain");
        block(&mut b, "src1", Paradigm::SignalFlow, "signal.step");
        block(&mut b, "scope1", Paradigm::SignalFlow, "sinks.scope");
        b.connect(
            Endpoint::new("src1", "out"),
            Endpoint::new("gain1", "in"),
            "signal",
            ConnectOptions::default(),
        )
        .unwrap();
        b.connect(
            Endpoint::new("gain1", "out"),
            Endpoint::new("scope1", "in"),
            "signal",
            ConnectOptions::default(),
        )
        .unwrap();

        b.auto_arrange(|_block| (60, 40), 20, 10);

        let src_x = b.get_block("src1").unwrap().position.0;
        let gain_x = b.get_block("gain1").unwrap().position.0;
        let scope_x = b.get_block("scope1").unwrap().position.0;
        assert!(src_x < gain_x);
        assert!(gain_x < scope_x);
    }

    // -- subckt ------------------------------------------------------------

    fn two_pin_def(id: &str) -> SubcktDef {
        SubcktDef::new(id, id.to_uppercase()).unwrap().with_ports([
            SubcktPort::new("a", "physical-electrical").unwrap(),
            SubcktPort::new("b", "physical-electrical").unwrap(),
        ])
    }

    #[test]
    fn add_subckt_def_rejects_a_duplicate_id() {
        let mut b = ModelBuilder::new("m", "M", 10);
        assert_eq!(b.add_subckt_def(two_pin_def("leaf")).unwrap(), "leaf");
        let err = b.add_subckt_def(two_pin_def("leaf")).unwrap_err();
        assert_eq!(err, BuilderError::DuplicateId("leaf".to_string()));
        assert_eq!(b.model().subckt_defs.len(), 1);
    }

    #[test]
    fn add_subckt_def_rejects_an_invalid_id_or_port_kind() {
        let mut b = ModelBuilder::new("m", "M", 10);
        let mut bad_id = two_pin_def("ok");
        bad_id.id = "1bad".to_string();
        assert!(matches!(
            b.add_subckt_def(bad_id),
            Err(BuilderError::Subckt(SubcktError::InvalidDefId(_)))
        ));
        // A def built by struct literal / deserialization bypasses SubcktPort::new, so the
        // builder must re-check rather than trust it.
        let mut bad_kind = two_pin_def("ok");
        bad_kind.ports[0].kind = "physical-signal".to_string();
        assert!(matches!(
            b.add_subckt_def(bad_kind),
            Err(BuilderError::Subckt(SubcktError::InvalidPortKind { .. }))
        ));
        assert!(b.model().subckt_defs.is_empty());
    }

    #[test]
    fn add_subckt_def_rejects_a_cycle_and_rolls_back_the_push() {
        let mut b = ModelBuilder::new("m", "M", 10);
        b.add_subckt_def(two_pin_def("a").with_instances([SubcktInstance::new("X1", "b_def")]))
            .unwrap();
        let err = b
            .add_subckt_def(
                SubcktDef::new("b_def", "B")
                    .unwrap()
                    .with_instances([SubcktInstance::new("X1", "a")]),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("Cyclic SubcktDef reference:")
                && err.to_string().contains("->"),
            "error should name the cycle: {err}"
        );
        // Fail-before-mutating: the rejected def must not be left in the model.
        assert_eq!(b.model().subckt_defs.len(), 1);
        assert_eq!(b.model().subckt_defs[0].id, "a");
    }

    #[test]
    fn add_subckt_def_rejects_a_self_reference() {
        let mut b = ModelBuilder::new("m", "M", 10);
        let err = b
            .add_subckt_def(
                two_pin_def("loopy").with_instances([SubcktInstance::new("X1", "loopy")]),
            )
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Cyclic SubcktDef reference: loopy -> loopy"
        );
        assert!(b.model().subckt_defs.is_empty());
    }

    #[test]
    fn instantiate_subckt_rejects_an_unknown_def_ref() {
        let mut b = ModelBuilder::new("m", "M", 10);
        let err = b
            .instantiate_subckt(NewSubcktInstanceSpec::new("ghost", (0, 0)))
            .unwrap_err();
        assert_eq!(err, BuilderError::UnknownSubcktDef("ghost".to_string()));
        assert!(b.model().blocks.is_empty());
    }

    #[test]
    fn instantiate_subckt_sets_paradigm_type_and_ref() {
        let mut b = ModelBuilder::new("m", "M", 10);
        b.add_subckt_def(two_pin_def("leaf")).unwrap();
        let id = b
            .instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (10, 20)).with_id("x1"))
            .unwrap();
        let block = b.get_block(&id).unwrap();
        assert_eq!(block.r#type, SUBCKT_INSTANCE_TYPE);
        assert_eq!(block.paradigm, Paradigm::SpiceSubckt);
        assert_eq!(block.subckt_ref.as_deref(), Some("leaf"));
        // All ports physical-electrical -> domain collapses to "electrical".
        assert_eq!(block.domain, "electrical");
        assert_eq!(block.position, (10, 20));
    }

    #[test]
    fn a_mixed_port_kind_def_instantiates_with_domain_mixed() {
        let mut b = ModelBuilder::new("m", "M", 10);
        b.add_subckt_def(
            SubcktDef::new("sw_leg", "sw_leg_fixed")
                .unwrap()
                .with_ports([
                    SubcktPort::new("d", "physical-electrical").unwrap(),
                    SubcktPort::new("ctrl_duty", "signal").unwrap(),
                ]),
        )
        .unwrap();
        let id = b
            .instantiate_subckt(NewSubcktInstanceSpec::new("sw_leg", (0, 0)))
            .unwrap();
        assert_eq!(b.get_block(&id).unwrap().domain, "mixed");
    }

    #[test]
    fn instantiate_twice_wire_both_and_round_trip_through_json() {
        let mut b = ModelBuilder::new("m", "Two Instances", 10);
        b.add_subckt_def(two_pin_def("leaf")).unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("x1"))
            .unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (100, 0)).with_id("x2"))
            .unwrap();
        block(&mut b, "r1", Paradigm::PhysicalNetwork, "passive.resistor");

        // The instances' wireable ports are exactly the def's own port ids, wired by ordinary
        // connect() calls -- no catalogue lookup involved.
        b.connect(
            Endpoint::new("x1", "b"),
            Endpoint::new("x2", "a"),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap();
        b.connect(
            Endpoint::new("x2", "b"),
            Endpoint::new("r1", "p1"),
            "physical-electrical",
            ConnectOptions::default(),
        )
        .unwrap();

        let doc = super::super::GsimSchDocument::new(b.into_model());
        let text = doc.to_json_pretty().unwrap();
        let back = super::super::GsimSchDocument::from_json(&text).unwrap();
        assert_eq!(back.model.subckt_defs.len(), 1);
        assert_eq!(back.model.blocks.len(), 3);
        assert_eq!(back.model.wires.len(), 2);
        let x1 = back.model.blocks.iter().find(|b| b.id == "x1").unwrap();
        assert_eq!(x1.subckt_ref.as_deref(), Some("leaf"));
        assert_eq!(x1.paradigm, Paradigm::SpiceSubckt);
    }

    #[test]
    fn a_subckt_instance_connects_to_a_signal_flow_block_too() {
        // A subckt boundary bridges paradigms (see Paradigm::bridges_paradigms) -- a gate-driven
        // switch def takes signal-kind ctrl_* nets from top-level `kind=const` blocks while its
        // d/s ports are electrical, so this must be legal *on the signal-kind port*.
        //
        // The def is now mixed-kind rather than `two_pin_def`: this test used to wire a `signal`
        // wire onto a port the def declares `physical-electrical`, which contradicted its own
        // comment and is exactly what port_compat's SubcktDef-backed kind check now rejects.
        let mut b = ModelBuilder::new("m", "M", 10);
        b.add_subckt_def(SubcktDef::new("leaf", "LEAF").unwrap().with_ports([
            SubcktPort::new("d", "physical-electrical").unwrap(),
            SubcktPort::new("ctrl", "signal").unwrap(),
        ]))
        .unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("x1"))
            .unwrap();
        block(&mut b, "c1", Paradigm::SignalFlow, "source.const");
        b.connect(
            Endpoint::new("c1", "out"),
            Endpoint::new("x1", "ctrl"),
            "signal",
            ConnectOptions::default(),
        )
        .unwrap();

        // ...and the same wire onto the def's *electrical* port is now refused, naming the port
        // and the kind it actually declares.
        block(&mut b, "c2", Paradigm::SignalFlow, "source.const");
        let err = b
            .connect(
                Endpoint::new("c2", "out"),
                Endpoint::new("x1", "d"),
                "signal",
                ConnectOptions::default(),
            )
            .unwrap_err();
        assert!(matches!(
            &err,
            BuilderError::PortIncompatible(inc) if inc.failure.reason() == "kind-mismatch"
        ));
        assert!(
            err.to_string()
                .contains("'x1.d' is declared 'physical-electrical'")
                && err.to_string().contains("as a 'signal' wire"),
            "{err}"
        );
    }

    #[test]
    fn a_model_without_subckt_defs_still_loads() {
        // Forward-compatibility: documents written before this field existed must round-trip.
        let text = r#"{"schema_version":2,"model":{"id":"m","name":"M","blocks":[],"wires":[]}}"#;
        let doc = super::super::GsimSchDocument::from_json(text).unwrap();
        assert!(doc.model.subckt_defs.is_empty());
    }
}
