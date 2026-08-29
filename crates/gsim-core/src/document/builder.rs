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
use super::wire::{Endpoint, Wire};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum BuilderError {
    DuplicateId(String),
    DanglingWire { block: String },
    CrossParadigm { from: Paradigm, to: Paradigm },
    UnknownBlock(String),
    Block(BlockError),
}

impl From<BlockError> for BuilderError {
    fn from(e: BlockError) -> Self {
        BuilderError::Block(e)
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
            BuilderError::UnknownBlock(id) => write!(f, "no block with id '{id}'"),
            BuilderError::Block(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BuilderError {}

pub struct NewBlockSpec {
    pub r#type: String,
    pub paradigm: Paradigm,
    pub domain: String,
    pub fidelity: Option<String>,
    pub position: (i64, i64),
    pub rotation: i64,
    pub mirror: Mirror,
    pub params: Map<String, Value>,
    pub ic: Map<String, Value>,
    pub label: Option<String>,
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
            id: None,
        }
    }
}

#[derive(Default)]
pub struct ConnectOptions {
    pub waypoints: Vec<(i64, i64)>,
    pub net: Option<String>,
    pub id: Option<String>,
    pub pinned: bool,
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

    /// Validates both endpoints exist and that the two blocks' paradigms are connectable (same
    /// paradigm, or one side bridges — see [`Paradigm::bridges_paradigms`]), then appends a
    /// wire whose `kind` matches what the caller asserts the ports to be (there is no catalogue
    /// lookup in this phase to verify that assertion against the blocks' actual port
    /// definitions — see the crate-level Phase 1 scope note).
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
            kind: kind.into(),
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
}
