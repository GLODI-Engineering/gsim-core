//! A block's paradigm — `docs/specs/gsim-sch-format.md` §4 design goal D2: two component
//! paradigms coexist (signal-flow, physical-network), plus the structural/bridging kinds that
//! connect them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Paradigm {
    PhysicalNetwork,
    SignalFlow,
    Converter,
    Hierarchy,
    Routing,
    Annotation,
    SpiceSubckt,
}

impl Paradigm {
    /// A bridging paradigm connects two differently-paradigmed blocks without itself needing to
    /// match either side — see `docs/specs/gsim-sch-format.md` §8.3 ("cross-paradigm boundary"):
    /// a `signal` wire and a `physical-electrical` wire cannot connect directly, only through a
    /// `converter.*` block; `routing`/`hierarchy`/`annotation` blocks are paradigm-agnostic
    /// passthroughs by construction (a junction, a subsystem boundary, a text label).
    pub fn bridges_paradigms(self) -> bool {
        matches!(
            self,
            Paradigm::Converter | Paradigm::Routing | Paradigm::Hierarchy | Paradigm::Annotation
        )
    }
}
