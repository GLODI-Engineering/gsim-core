//! `gsim-core` — Phase 1 of the shared Rust implementation behind `general-simulator-gui`'s
//! schematic tooling. See `README.md` and `docs/decisions/0001-scope-and-naming.md` for full
//! scope and rationale. Two of Phase 1's three deliverables are implemented:
//!
//! - [`expr`] — the closed-grammar `${{ ... }}` expression language (lexer/parser/AST/eval).
//! - [`template`] — `${{ }}` string interpolation on top of `expr`.
//! - [`resolve`] — the `for_each`/`when`/`emit` mechanism that expands a dynamic `ports:`/
//!   `primitives:` list into its concrete, static form for a given set of parameter values.
//! - [`port`] / [`primitive`] — the resolved, static shapes, matching the frontend's existing
//!   port-geometry convention and its `Primitive` union exactly.
//! - [`contract`] — `ComponentContract`, `Parameter` validation, and the top-level
//!   [`contract::evaluate_component`] pipeline tying all of the above together (GSF v2, done).
//! - [`document`] — the `.gsim_sch` document/builder model, superseding
//!   `packages/gsim-model`'s Python-only equivalent (done).
//! - [`layout`] — real autolayout (auto-spacing/collision avoidance), which existed nowhere in
//!   the codebase before this crate (done — all three Phase 1 deliverables now complete).
//!
//! No `eval`/dynamic code execution of any kind belongs in this crate — see the module doc on
//! [`expr`].

pub mod compile;
pub mod contract;
pub mod document;
pub mod expr;
mod fmt;
pub mod layout;
pub mod port;
pub mod primitive;
pub mod resolve;
pub mod template;

/// Crate version, exposed for FFI smoke tests in `gsim-py`/`gsim-wasm`.
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
