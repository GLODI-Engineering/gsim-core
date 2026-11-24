//! A wire connecting two block ports. `kind` duplicates the endpoints' port-kind per
//! `docs/specs/gsim-sch-format.md` §8.4, so schema validation never needs the block catalogue.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Endpoint {
    pub block: String,
    pub port: String,
}

impl Endpoint {
    pub fn new(block: impl Into<String>, port: impl Into<String>) -> Endpoint {
        Endpoint {
            block: block.into(),
            port: port.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wire {
    pub id: String,
    #[serde(rename = "from")]
    pub from: Endpoint,
    pub to: Endpoint,
    /// Port-kind catalogue string (`docs/specs/gsim-sch-format.md` §8) — kept open (not a closed
    /// Rust enum) for the same forward-compatibility reason as `Port::kind`.
    pub kind: String,
    #[serde(default)]
    pub waypoints: Vec<(i64, i64)>,
    #[serde(default)]
    pub pinned: bool,
    /// Emitted even when absent, as `"net": null`.
    ///
    /// Not `skip_serializing_if`, unlike every other optional field on this struct and on
    /// [`super::block::Block`]. The document schema that governs `.gsim_sch` lists `net` in
    /// `Wire.required` and types it `string | null`, so `null` is a legal value and a missing
    /// key is not. Skipping it produced documents this crate re-read quite happily and the
    /// editor's validator rejected outright with `/model/wires/0: must have required property
    /// 'net'` -- which is how all five of omni-schematics' committed examples became
    /// unopenable while every test here passed (#73).
    ///
    /// `label` below keeps its `skip_serializing_if` because the schema genuinely does not
    /// require it. An audit at the time of this change found `net` to be the only
    /// schema-required field in this module that was being skipped.
    #[serde(default)]
    pub net: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
