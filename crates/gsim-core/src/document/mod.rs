//! The `.gsim_sch` document/builder model — Phase 1's second deliverable (alongside GSF v2 in
//! [`crate::contract`]). Supersedes `general-simulator-gui`'s Python-only
//! `packages/gsim-model`'s `document.py`/`block.py`/`wire.py`/`builder.py`.
//!
//! **Deliberately out of scope this phase**, matching the same boundary `packages/gsim-model`
//! already drew (not a new gap introduced here):
//! - Subsystem/hierarchy block authoring (`SubsystemModel` exists in the schema; no builder
//!   method creates one yet, same as `gsim_model`).
//! - Parsing raw `.SUBCKT` *text* into a [`SubcktDef`] (there is no SPICE text reader here).
//!   Note the def itself is **not** opaque any more — unlike `gsim_model`'s `SubcktDef.body`,
//!   [`subckt`]'s def has a structured `blocks`/`wires`/`instances` body; see that module's doc
//!   for why that divergence was necessary.
//! - `lines` (annotations), `nets` (named nets), `simulation` (analysis directives) — round-
//!   tripped as opaque JSON, not modeled in detail.
//! - Autolayout (a separate Phase 1 deliverable, not started).
//!
//! **Implemented beyond what `gsim_model` has today**: `connect()` actually enforces the
//! cross-paradigm boundary (`docs/specs/gsim-sch-format.md` §8.3) — see [`builder::BuilderError::CrossParadigm`]
//! — where `gsim_model` only ever referenced a `CrossParadigmWireError` in comments without
//! defining it (see `docs/decisions/0001-scope-and-naming.md`).

mod block;
mod builder;
mod model;
mod paradigm;
mod subckt;
mod wire;

pub use block::{deserialize_comment, Block, BlockError, Mirror, NewBlock};
pub use builder::{
    BuilderError, ConnectOptions, ModelBuilder, NewBlockSpec, NewSubcktInstanceSpec,
    SUBCKT_INSTANCE_TYPE,
};
pub use model::{DocumentLoadError, GsimSchDocument, Model, ModelParam, WrongSchemaVersion};
pub use paradigm::Paradigm;
pub use subckt::{
    check_no_cyclic_defs, SubcktDef, SubcktError, SubcktInstance, SubcktPort, SUBCKT_BOUNDARY_TYPE,
    SUBCKT_PORT_KINDS,
};
pub use wire::{Endpoint, Wire};
