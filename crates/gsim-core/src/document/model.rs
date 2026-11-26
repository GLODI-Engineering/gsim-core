//! The `.gsim_sch` model object and its top-level document wrapper. Field shape matches
//! `docs/specs/gsim-sch-format.md` §3-4. `lines` (annotations), `nets` (named nets), and
//! `simulation` (analysis directives) are round-tripped as opaque JSON in this phase — the same
//! conscious scope boundary `packages/gsim-model` drew (its own `Model.simulation` docstring
//! calls this "out of scope for the connectivity/builder work"), not a gap unique to this crate.

use super::block::Block;
use super::code_build::CodeBuild;
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
/// the editor's own `_check_role` in its `.gsim_sch` loader, achieved by typing rather than a
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
    /// See [`Model::extra`].
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
    /// How this sheet's code blocks are built — see [`super::CodeBuild`] and
    /// `docs/decisions/0009`. `None`, and absent from the emitted JSON, for every document
    /// written before that record, which is why a sheet with no plan compiles byte for byte as
    /// it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_build: Option<CodeBuild>,
    #[serde(default)]
    pub lines: Vec<Value>,
    #[serde(default)]
    pub nets: Vec<Value>,
    #[serde(default)]
    pub simulation: Map<String, Value>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    /// Every key of this object that the fields above do not model, kept verbatim.
    ///
    /// `save_document` is a parse followed by a serialize, and serde drops an unrecognised key
    /// at the *parse* step -- so before this field existed, loading and saving a document
    /// deleted `model.ports`, `model.parent_param_aliases`, `blocks[].subsystem` (an entire
    /// nested model) and `subckt_defs[].body` without a word, and without an error (#78). The
    /// editor writes all four. This crate models none of them, and
    /// `docs/decisions/0002-no-private-content.md`'s scope note plus this module's own
    /// "deliberately out of scope" list say it should not have to: **round-tripping is a
    /// separate obligation from understanding.** What the compiler does not interpret, the
    /// document still has to hand back.
    ///
    /// A catch-all rather than four typed fields, so the next field the format gains costs
    /// nothing here. The accepted cost is that a *misspelled* key now round-trips instead of
    /// being silently dropped -- for a format whose writer (an editor that may be newer than
    /// the engine reading it) that is the correct bias, because the alternative is deleting a
    /// user's work because we did not recognise it.
    ///
    /// Empty is the common case and emits nothing, so a document that uses none of this
    /// serializes exactly as it did before.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
            code_build: None,
            lines: Vec::new(),
            nets: Vec::new(),
            simulation: Map::new(),
            metadata: Map::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GsimSchDocument {
    pub schema_version: u32,
    pub model: Model,
    /// See [`Model::extra`] -- the same obligation, at the document's own top level.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
            extra: Map::new(),
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
