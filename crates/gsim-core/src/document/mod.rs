//! The `.gsim_sch` document/builder model — Phase 1's second deliverable (alongside GSF v2 in
//! [`crate::contract`]). Supersedes `general-simulator-gui`'s Python-only
//! `packages/gsim-model`'s `document.py`/`block.py`/`wire.py`/`builder.py`.
//!
//! **Deliberately out of scope this phase**, matching the same boundary `packages/gsim-model`
//! already drew (not a new gap introduced here):
//! - Subsystem/hierarchy block authoring (`SubsystemModel` exists in the schema; no builder
//!   method creates one yet, same as `gsim_model`).
//! - `spice-subckt` body parsing (kept as opaque text, same as `gsim_model`'s `SubcktDef.body`).
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
mod wire;

pub use block::{Block, BlockError, Mirror, NewBlock};
pub use builder::{BuilderError, ConnectOptions, ModelBuilder, NewBlockSpec};
pub use model::{DocumentLoadError, GsimSchDocument, Model, ModelParam, WrongSchemaVersion};
pub use paradigm::Paradigm;
pub use wire::{Endpoint, Wire};
