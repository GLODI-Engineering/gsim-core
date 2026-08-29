//! `gsim-core` — Phase 1 of the shared Rust implementation behind `general-simulator-gui`'s
//! schematic tooling:
//!
//! - **GSF v2**: a parametric component-contract schema (ports/geometry/parameter validation
//!   that can depend on parameter values, evaluated live while a user edits — unlike today's
//!   `.gsim_sym`/GSF v1, which bakes primitives to fixed coordinates at authoring time only;
//!   see `docs/decisions/0001-scope-and-naming.md`).
//! - **`.gsim_sch` document model**: block/wire/hierarchy construction and validation,
//!   superseding `general-simulator-gui`'s existing Python-only `packages/gsim-model`.
//! - **Autolayout**: real auto-spacing/collision-avoidance placement helpers, which do not
//!   exist anywhere in the codebase today (confirmed by audit — `gsim_model/layout.py` is pure
//!   port-geometry math, not autolayout) — needed so a programmatic (agent/automation) caller
//!   can produce a visually clean schematic without manually reasoning about every coordinate.
//!
//! **Out of scope for Phase 1**: compiling a `.gsim_sch` document to an ngspice/Xyce/
//! general-simulator netlist. That logic currently lives in the much larger
//! `packages/simulator/src/simulator/compiler/` pipeline and stays there until a later phase.
//!
//! Two thin binding crates expose this crate: `gsim-py` (PyO3, for the Python automation
//! library) and `gsim-wasm` (wasm-bindgen, for the frontend). Neither should reimplement any
//! schema, validation, or layout logic of their own — see the repo README.
//!
//! No `eval`/dynamic code execution of any kind belongs here — the expression DSL (`${{ }}`,
//! used for parametric ports/geometry/validation) is a closed-grammar, pure-data interpreter by
//! design, safe to load from third-party component definitions.

/// Crate version, exposed for FFI smoke tests in `gsim-py`/`gsim-wasm` before there is any real
/// surface to bind against.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }
}
