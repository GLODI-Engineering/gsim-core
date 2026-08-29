//! A wire connecting two block ports. `kind` duplicates the endpoints' port-kind per
//! `docs/specs/gsim-sch-format.md` §8.4, so schema validation never needs the block catalogue.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
