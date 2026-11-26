//! What a sheet says about how its code blocks are built — the document half of
//! [`docs/decisions/0009`](../../../../docs/decisions/0009-a-document-carried-build-for-code-blocks.md).
//!
//! This module is shape only: it parses, round-trips and re-serializes a plan without judging
//! any of it. Every refusal the record lists lives in [`crate::compile::code_build`], because a
//! refusal needs `model.params` and a block's `lib` to say anything, and because a plan a
//! *reader* rejects is a plan a save would delete.
//!
//! That last point is why [`CodeBuildStep::kind`] is a `String` and not an enum. 0009 §10
//! requires two different things of a step kind this version does not implement: the document
//! must **keep** it verbatim — or opening a sheet in an older editor and saving it would throw
//! away the user's build — and the compiler must **refuse** it rather than skip it, because a
//! skipped step leaves an artifact nobody built, which runs, which is
//! `GLODI-Engineering/omni-schematics#235`. A serde enum would fail at the parse step and so
//! satisfy only the second. Round-tripping is a separate obligation from understanding
//! (`docs/decisions/0008`), and here the two are discharged in two different modules.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A `-D` on a compile line, and the one place the circuit's own numbers reach a code block.
///
/// Exactly one of [`from`](CodeDefine::from) and [`literal`](CodeDefine::literal) carries the
/// value. `from` names an entry of [`super::Model::params`] — the same table the netlist is
/// generated from, which is the entire point: the define and the element card are resolved by
/// one pass over one dictionary, so they cannot disagree. `literal` is the author stating that
/// this constant is *not* a circuit quantity, spelled differently so a reader scanning a plan
/// can see at a glance which constants the circuit owns.
///
/// There is deliberately no third form, and in particular no form in which a number lives both
/// here and on a drawn component. Two places holding one quantity, one of them edited, is the
/// failure 0009 exists to prevent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeDefine {
    /// The macro name. Must be an identifier; see
    /// [`crate::compile::code_build`]'s refusal for why a name that is not one is rejected
    /// rather than quoted.
    pub name: String,
    /// The `model.params` entry this define's value comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// A constant that is not a circuit quantity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub literal: Option<Value>,
    /// `true` when the author has decided this define may be absent — and the **only** way it
    /// may be. 0009 §8: absence on the author's side is a decision, absence on the document's
    /// side is a mistake, and nothing in the format lets the second look like the first. An
    /// omitted `optional` define's name is recorded in the resolved step's `omitted_defines`,
    /// so a build made without it never looks like a build that forgot it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    /// See [`super::Model::extra`].
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One product of a build. `shared-library` is the only [`kind`](CodeBuildStep::kind) this
/// version implements; the field is open so that 0009 §10's expected additions — a host program
/// compiled and then run to generate a header, and a generated constants file for a Python or
/// Octave block — arrive without a breaking change.
///
/// A `shared-library` step carries **no output path**. Its artifact is the `lib` parameter of
/// the block(s) that name it (0009 §4), so the artifact is named in exactly one place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeBuildStep {
    /// Unique within the plan. A block's `build` names this.
    pub id: String,
    /// `"shared-library"`. Kept as free text on purpose — see this module's doc.
    pub kind: String,
    /// `"c"` or `"c++"`. Required for a `shared-library` step: it decides which compiler driver
    /// the builder invokes, and inferring it from a source file's extension is a guess in a
    /// feature whose whole subject is not guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Every file that goes on the compile line — the thin `cscript_*` adapter *and* the shared
    /// sources it needs. Relative to the schematic's own directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// Appended to the plan's own `includes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Appended to the plan's own `defines`; an entry whose `name` repeats a plan-level one
    /// replaces it in place rather than adding a second `-D` (0009 §7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defines: Vec<CodeDefine>,
    /// Appended to the plan's own `flags`. Opaque compiler-specific text, never interpreted
    /// here — which is also what makes a sheet that uses it a sheet that builds only where
    /// those flags are understood.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    /// Appended to the plan's own `libs`. A bare library name (`"m"`), not a flag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libs: Vec<String>,
    /// Step ids that must have run first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// See [`super::Model::extra`].
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A sheet's build plan. Lives on the model rather than on a block because the case to serve is
/// five blocks sharing two sources: the include directories and defines belong to the sheet
/// outright, the shared sources are visible side by side in the step list, and a step whose
/// product no block claims — the host program that generates a lookup table — belongs to no
/// block at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeBuild {
    /// Include directories every step gets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Defines every step gets — the circuit's own numbers, written once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defines: Vec<CodeDefine>,
    /// Opaque compiler flags every step gets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    /// Libraries every step links.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libs: Vec<String>,
    #[serde(default)]
    pub steps: Vec<CodeBuildStep>,
    /// See [`super::Model::extra`].
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl CodeBuild {
    pub fn step(&self, id: &str) -> Option<&CodeBuildStep> {
        self.steps.iter().find(|s| s.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_round_trips_and_emits_nothing_it_was_not_given() {
        let text = r#"{
          "includes": ["code"],
          "defines": [{"name":"L_F","from":"L_f"},{"name":"N","literal":3}],
          "libs": ["m"],
          "steps": [{"id":"ctrl","kind":"shared-library","lang":"c",
                     "sources":["code/cscript_ctrl.c","code/law.c"]}]
        }"#;
        let plan: CodeBuild = serde_json::from_str(text).unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.defines[0].from.as_deref(), Some("L_f"));
        assert!(!plan.defines[0].optional);

        let json = serde_json::to_string(&plan).unwrap();
        // Empty slots stay out entirely, so a plan written by hand comes back as it went in.
        assert!(!json.contains("flags"), "{json}");
        assert!(!json.contains("optional"), "{json}");
        assert!(!json.contains("needs"), "{json}");
        assert_eq!(serde_json::from_str::<CodeBuild>(&json).unwrap(), plan);
    }

    /// 0009 §10, as code: a kind this version does not implement must survive a save. The
    /// compiler refuses it; the document keeps it.
    #[test]
    fn an_unknown_step_kind_survives_a_round_trip_rather_than_failing_to_parse() {
        let text = r#"{"steps":[{"id":"table","kind":"host-program","sources":["code/gen.c"],
                      "run":{"stdout":"code/generated/table.h"}}]}"#;
        let plan: CodeBuild = serde_json::from_str(text).unwrap();
        assert_eq!(plan.steps[0].kind, "host-program");
        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("host-program"), "{json}");
        // And the field no struct here models comes back too.
        assert!(json.contains("\"run\""), "{json}");
    }
}
