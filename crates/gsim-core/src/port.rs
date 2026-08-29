//! The resolved port shape, matching the `side`/`offset`/`kind`/`direction` convention already
//! used by `general-simulator-gui`'s GSF v1 (`gsim_model.symbol.SymbolBuilder.port()` /
//! `gsim_model.layout.PortGeometry`) — GSF v2 does not introduce a new port-positioning scheme,
//! only a way to generate a *list* of these dynamically (see `crate::resolve`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortSide {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortDirection {
    In,
    Out,
    Inout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Port {
    /// Stable, never reassigned to a different logical port even if sibling ports are added or
    /// removed by a `for_each` re-evaluation — see
    /// `docs/decisions/0001-scope-and-naming.md`'s "port identity" section.
    pub id: String,
    pub side: PortSide,
    #[serde(default)]
    pub offset: f64,
    /// Port-kind catalogue (`docs/specs/gsim-sch-format.md` §8) is open-ended; kept as a plain
    /// string rather than a closed Rust enum so a new kind never requires a schema-breaking
    /// release of this crate.
    pub kind: String,
    pub direction: PortDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
