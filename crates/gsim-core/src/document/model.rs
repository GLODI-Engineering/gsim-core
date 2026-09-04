//! The `.gsim_sch` model object and its top-level document wrapper. Field shape matches
//! `docs/specs/gsim-sch-format.md` §3-4. `lines` (annotations), `nets` (named nets), and
//! `simulation` (analysis directives) are round-tripped as opaque JSON in this phase — the same
//! conscious scope boundary `packages/gsim-model` drew (its own `Model.simulation` docstring
//! calls this "out of scope for the connectivity/builder work"), not a gap unique to this crate.

use super::block::Block;
use super::subckt::SubcktDef;
use super::wire::Wire;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

fn default_grid() -> i64 {
    10
}

/// `.gsim_sch` §4.1: a hint, not a constraint. `"library"` means the document carries reusable
/// content (a stamp, `subckt_defs`, or both) rather than being a directly-simulatable top-level
/// schematic — nothing here validates that a stamp actually has dangling ports, nor lets a
/// schematic omit them. Absent on `Model::role` means `Schematic`, matching every other
/// forward-compatible field in this struct (`subckt_defs` included) — an invalid string value is
/// rejected by `serde`'s own enum deserialization, the same practical effect as
/// `general-simulator-gui`'s `gsim_sch_loader.py::_check_role`, achieved by typing rather than a
/// hand-written check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Schematic,
    Library,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelParam {
    pub name: String,
    /// A number, or a `"{expression}"` string per §5.1 — not this crate's `${{ }}` GSF v2
    /// grammar; `.gsim_sch`'s own `{...}` param-reference syntax is a separate, narrower
    /// mechanism (a bare cross-reference to another named param, not a general expression
    /// language) and is not evaluated by this crate in Phase 1.
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    #[serde(default = "default_grid")]
    pub grid: i64,
    #[serde(default)]
    pub params: Vec<ModelParam>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub wires: Vec<Wire>,
    /// Reusable SPICE `.SUBCKT` definitions — see [`super::subckt`]. Defaults to empty, matching
    /// the forward-compatible field style every other optional field here already uses, so a
    /// document written before this field existed still loads unchanged.
    #[serde(default)]
    pub subckt_defs: Vec<SubcktDef>,
    /// `None` means `Schematic` (§4.1) — and, on write, `None` stays absent from the emitted
    /// JSON (`skip_serializing_if`), not materialized to `"schematic"`, so the byte-exact
    /// roundtrip guarantee holds for every document written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ModelRole>,
    #[serde(default)]
    pub lines: Vec<Value>,
    #[serde(default)]
    pub nets: Vec<Value>,
    #[serde(default)]
    pub simulation: Map<String, Value>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl Model {
    pub fn new(id: impl Into<String>, name: impl Into<String>, grid: i64) -> Model {
        Model {
            id: id.into(),
            name: name.into(),
            grid,
            params: Vec::new(),
            blocks: Vec::new(),
            wires: Vec::new(),
            subckt_defs: Vec::new(),
            role: None,
            lines: Vec::new(),
            nets: Vec::new(),
            simulation: Map::new(),
            metadata: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GsimSchDocument {
    pub schema_version: u32,
    pub model: Model,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WrongSchemaVersion {
    pub found: u32,
}

impl std::fmt::Display for WrongSchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported schema_version {} (only 2 is supported -- there is no v1 reader, per docs/specs/gsim-sch-format.md)",
            self.found
        )
    }
}

impl std::error::Error for WrongSchemaVersion {}

impl GsimSchDocument {
    pub fn new(model: Model) -> GsimSchDocument {
        GsimSchDocument {
            schema_version: 2,
            model,
        }
    }

    /// Parses and checks `schema_version == 2` up front — matching
    /// `docs/specs/gsim-sch-format.md` §3's "the v2 loader rejects any other value. There is no
    /// v1 reader," rather than silently accepting an unrelated document shape.
    pub fn from_json(text: &str) -> Result<GsimSchDocument, DocumentLoadError> {
        let doc: GsimSchDocument = serde_json::from_str(text)?;
        if doc.schema_version != 2 {
            return Err(DocumentLoadError::WrongVersion(WrongSchemaVersion {
                found: doc.schema_version,
            }));
        }
        Ok(doc)
    }

    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Debug)]
pub enum DocumentLoadError {
    Json(serde_json::Error),
    WrongVersion(WrongSchemaVersion),
}

impl From<serde_json::Error> for DocumentLoadError {
    fn from(e: serde_json::Error) -> Self {
        DocumentLoadError::Json(e)
    }
}

impl std::fmt::Display for DocumentLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentLoadError::Json(e) => write!(f, "{e}"),
            DocumentLoadError::WrongVersion(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DocumentLoadError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_non_v2_schema_version() {
        let text = r#"{"schema_version":1,"model":{"id":"m","name":"M","blocks":[],"wires":[]}}"#;
        assert!(matches!(
            GsimSchDocument::from_json(text),
            Err(DocumentLoadError::WrongVersion(WrongSchemaVersion {
                found: 1
            }))
        ));
    }

    #[test]
    fn round_trips_an_empty_model() {
        let doc = GsimSchDocument::new(Model::new("m", "My Model", 10));
        let text = doc.to_json_pretty().unwrap();
        let reloaded = GsimSchDocument::from_json(&text).unwrap();
        assert_eq!(reloaded.model.id, "m");
        assert_eq!(reloaded.schema_version, 2);
    }

    #[test]
    fn role_defaults_to_absent_and_is_never_materialized_on_write() {
        let doc = GsimSchDocument::new(Model::new("m", "My Model", 10));
        assert_eq!(doc.model.role, None);
        let text = doc.to_json_pretty().unwrap();
        assert!(!text.contains("\"role\""));
        let reloaded = GsimSchDocument::from_json(&text).unwrap();
        assert_eq!(reloaded.model.role, None);
    }

    #[test]
    fn role_round_trips_when_set_and_rejects_an_unknown_value() {
        let mut model = Model::new("m", "My Model", 10);
        model.role = Some(ModelRole::Library);
        let doc = GsimSchDocument::new(model);
        let text = doc.to_json_pretty().unwrap();
        assert!(text.contains("\"role\": \"library\""));
        let reloaded = GsimSchDocument::from_json(&text).unwrap();
        assert_eq!(reloaded.model.role, Some(ModelRole::Library));

        let bad = r#"{"schema_version":2,"model":{"id":"m","name":"M","blocks":[],"wires":[],"role":"gallery"}}"#;
        assert!(GsimSchDocument::from_json(bad).is_err());
    }
}
