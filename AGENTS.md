# Agent Guidelines

`gsim-core` is the shared Rust implementation behind `general-simulator-gui`'s schematic
tooling. See `README.md` for scope and `docs/decisions/` for why it exists and what was ruled
out before writing code. This file lists only what mechanical checks cannot enforce.

## Authority order

1. **Mechanical gates** — `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`. If a
   gate fails, fix the cause; never bypass.
2. **This file** — invariants that cannot be mechanized.
3. **`docs/decisions/`** — lasting design rationale; read before revisiting a settled question.

## Project layout

```
crates/gsim-core/   schema types, expression-DSL interpreter, .gsim_sch document model, autolayout
crates/gsim-py/      PyO3 bindings (Python automation library)
crates/gsim-wasm/    wasm-bindgen bindings (general-simulator-gui frontend)
docs/decisions/      why, not what -- read before changing settled scope
docs/journal/        chronological continuity log, one file per month
```

## Architectural invariants

1. **No `eval`, ever, in any binding.** The `${{ }}` expression DSL is a closed-grammar,
   pure-data interpreter. Component contracts are expected to be loaded from third-party
   libraries; arbitrary code execution in a loaded definition is not an acceptable risk. See
   `docs/decisions/0001-scope-and-naming.md`.
2. **Phase 1 excludes netlist compilation.** Do not add ngspice/Xyce/general-simulator netlist
   emission here — that logic lives in `general-simulator-gui`'s
   `packages/simulator/src/simulator/compiler/` until a later, explicitly-scoped phase.
3. **Stable port ids.** A generated port's id is index-based and never reassigned when other
   ports are added/removed. Removing a connected port leaves its wire dangling with an error
   marker in the consuming frontend — it must never silently reattach to a different port.
4. **One implementation, thin bindings.** `gsim-py`/`gsim-wasm` must not reimplement schema,
   validation, or layout logic. If a binding needs to duplicate logic to be ergonomic, that
   logic belongs in `gsim-core`, exposed through a cleaner API — not copied.
5. **No half-finished implementations.** This repo starts from an explicit audit of what
   already exists (`packages/gsim-model` in `general-simulator-gui`) — do not re-guess scope
   that audit already settled; read `docs/decisions/` first.

## Relationship to sibling repos

Read-only path deps, same convention `general-simulator`/`general-mna` already use — see
`README.md`'s diagram. `gsim-core` has no dependencies on any sibling repo; it is a leaf.

## What NOT to do

- Don't bypass gates with `--no-verify`/`SKIP=`.
- Don't add abstractions without an existing call site.
- Don't duplicate content across `AGENTS.md`, `CLAUDE.md`, and `docs/decisions/`. One source of
  truth per rule.
- Don't claim a task complete if CI/gates are red.

## Tool-specific notes

- **Claude Code** reads this file plus `CLAUDE.md` (a thin pointer).
- **Other tools**: read this file directly.
