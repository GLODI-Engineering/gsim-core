//! A block instance in a `.gsim_sch` model. Field shape matches
//! `docs/specs/gsim-sch-format.md` §6 and `packages/gsim-model`'s existing `Block` (audited
//! before writing this — same id/type patterns, same required fields).

use super::paradigm::Paradigm;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mirror {
    #[default]
    None,
    X,
    Y,
}

const VALID_ROTATIONS: [i64; 4] = [0, 90, 180, 270];

fn default_fidelity() -> String {
    "ideal".to_string()
}

fn id_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_-]*$").unwrap())
}

fn type_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)+$").unwrap())
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockError {
    InvalidId(String),
    InvalidType(String),
    InvalidRotation(i64),
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockError::InvalidId(id) => write!(f, "invalid block id '{id}'"),
            BlockError::InvalidType(t) => write!(f, "invalid block type '{t}'"),
            BlockError::InvalidRotation(r) => {
                write!(f, "invalid rotation {r} (must be one of 0, 90, 180, 270)")
            }
        }
    }
}

impl std::error::Error for BlockError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    pub r#type: String,
    pub paradigm: Paradigm,
    pub domain: String,
    #[serde(default = "default_fidelity")]
    pub fidelity: String,
    pub position: (i64, i64),
    #[serde(default)]
    pub rotation: i64,
    #[serde(default)]
    pub mirror: Mirror,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub ic: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Free-text authoring comment emitted *above* this block's card(s) in a compiled netlist.
    ///
    /// One entry per comment line (an entry containing `\n` is split further at emit time, so a
    /// single multi-line string is also accepted); empty vec — the default — means no comment.
    /// Purely an annotation: it never affects nets, params or validation, and it round-trips
    /// through `.gsim_sch` JSON like any other optional field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comment_before: Vec<String>,
    /// Free-text authoring comment emitted *below* this block's card(s) — see
    /// [`Block::comment_before`]. An independent slot: a block may carry either, both or neither.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comment_after: Vec<String>,
    /// `Some(def_id)` exactly when `paradigm == Paradigm::SpiceSubckt`: names which
    /// [`super::SubcktDef`] this block instantiates. A subckt-instance block's own wireable
    /// ports are exactly that def's `ports[].id` list.
    ///
    /// Populated by [`super::ModelBuilder::instantiate_subckt`], which is also where the
    /// `def_id`-actually-exists check lives — `Block::new` has no access to the document's def
    /// list, and `connect()` deliberately does not validate port existence against a catalogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subckt_ref: Option<String>,
}

pub struct NewBlock {
    pub id: String,
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
    pub comment_before: Vec<String>,
    pub comment_after: Vec<String>,
    pub subckt_ref: Option<String>,
}

impl Block {
    /// Validates id/type patterns and rotation before construction — matching
    /// `gsim_model.block.Block._validate_id_and_type`'s fail-fast-at-construction behavior,
    /// not deferred to a later schema pass.
    pub fn new(spec: NewBlock) -> Result<Block, BlockError> {
        if !id_pattern().is_match(&spec.id) {
            return Err(BlockError::InvalidId(spec.id));
        }
        if !type_pattern().is_match(&spec.r#type) {
            return Err(BlockError::InvalidType(spec.r#type));
        }
        if !VALID_ROTATIONS.contains(&spec.rotation) {
            return Err(BlockError::InvalidRotation(spec.rotation));
        }
        Ok(Block {
            id: spec.id,
            r#type: spec.r#type,
            paradigm: spec.paradigm,
            domain: spec.domain,
            fidelity: spec.fidelity.unwrap_or_else(default_fidelity),
            position: spec.position,
            rotation: spec.rotation,
            mirror: spec.mirror,
            params: spec.params,
            ic: spec.ic,
            label: spec.label,
            comment_before: spec.comment_before,
            comment_after: spec.comment_after,
            subckt_ref: spec.subckt_ref,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, r#type: &str) -> NewBlock {
        NewBlock {
            id: id.to_string(),
            r#type: r#type.to_string(),
            paradigm: Paradigm::PhysicalNetwork,
            domain: "electrical".to_string(),
            fidelity: None,
            position: (0, 0),
            rotation: 0,
            mirror: Mirror::None,
            params: Map::new(),
            ic: Map::new(),
            label: None,
            comment_before: Vec::new(),
            comment_after: Vec::new(),
            subckt_ref: None,
        }
    }

    #[test]
    fn rejects_invalid_id_and_type_patterns() {
        assert!(matches!(
            Block::new(spec("1bad", "passive.resistor")),
            Err(BlockError::InvalidId(_))
        ));
        assert!(matches!(
            Block::new(spec("r1", "NotDotted")),
            Err(BlockError::InvalidType(_))
        ));
    }

    #[test]
    fn rejects_off_grid_rotation() {
        let mut s = spec("r1", "passive.resistor");
        s.rotation = 45;
        assert!(matches!(
            Block::new(s),
            Err(BlockError::InvalidRotation(45))
        ));
    }

    #[test]
    fn defaults_fidelity_to_ideal() {
        let b = Block::new(spec("r1", "passive.resistor")).unwrap();
        assert_eq!(b.fidelity, "ideal");
    }
}
