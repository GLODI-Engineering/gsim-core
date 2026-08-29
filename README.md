# gsim-core

`gsim-core` is the shared Rust implementation behind `general-simulator-gui`'s schematic
tooling — one implementation instead of the separate TypeScript (frontend) and Python
(`packages/gsim-model`) ones that exist today.

## Scope (Phase 1)

- **GSF v2** — a parametric component-contract schema: ports, geometry, and parameter
  validation that can depend on parameter values and are evaluated *live* while a user edits
  (e.g. a Sum block gaining a port when its `signs` parameter grows). This supersedes today's
  `.gsim_sym` / GSF v1 (`docs/specs/gsim-symbol-format-spec.md` in `general-simulator-gui`),
  which is fully implemented but bakes primitives to fixed coordinates once, at authoring time,
  by explicit design — confirmed by reading its implementation
  (`packages/gsim-model/src/gsim_model/symbol.py`) before starting this repo.
- **`.gsim_sch` document model** — block/wire/hierarchy construction and validation,
  superseding `packages/gsim-model`'s Python-only `ModelBuilder`/`document.py`/`wire.py`.
- **Autolayout** — real auto-spacing/collision-avoidance placement helpers. These do not exist
  anywhere in the codebase today: `packages/gsim-model/src/gsim_model/layout.py` is pure
  port-geometry math (port centers, a single-bend wire router), not autolayout. This matters
  because a programmatic caller (an agent building a schematic entirely in code) needs to
  produce a visually clean, non-overlapping schematic without manually reasoning about every
  coordinate — today that's impossible without hand-placing everything.

**Out of scope for Phase 1**: compiling a `.gsim_sch` document to an ngspice/Xyce/
general-simulator netlist. That logic currently lives in the much larger
`packages/simulator/src/simulator/compiler/` pipeline (`ir1_builder`, `ir2_lowering`,
`spice_netlist_writer`, `dae_system`, `scipy_compiler`, and more) and stays there until a later
phase, once Phase 1 has landed and proven itself.

Full rationale and everything settled before writing code: [`docs/decisions/`](docs/decisions/).

## Why one implementation

Hand-keeping the same contract-evaluation and document-validation logic in TypeScript and
Python separately is exactly the kind of two-implementation drift `general-simulator-gui`'s own
`docs/simulator-component-integration.md` already documents as a real, recurring risk (the
`BLOCK_TO_SPICE` mapping precedent). `gsim-core` exists once; two thin binding crates expose it:

- `crates/gsim-py` — PyO3 bindings. Intended to become the implementation behind (and a
  drop-in successor for) `general-simulator-gui`'s existing `packages/gsim-model`, which today
  is a real, tested, pydantic-based library (`ModelBuilder`, `SymbolBuilder`,
  `ComponentDefinition`, `.gsim_sym` I/O) — not a stub. `gsim-py` is the automation library a
  user or agent uses to build a schematic entirely from code, with no matplotlib/pandas/etc:
  its only job is schematic construction and (in a later phase) netlist compilation.
- `crates/gsim-wasm` — wasm-bindgen bindings, consumed by `general-simulator-gui`'s frontend
  for its own `.gsim_sch` serialization and for evaluating the GSF v2 contract to paint a
  component's ports/geometry — the frontend still owns all actual SVG/DOM painting.

The same `gsim-wasm` binding that the frontend needs for its own serialization also makes a
Node.js/JS automation library (a JS equivalent of `gsim-py`) essentially free — worth building
once there's real demand for it, since it would need no new Rust-side work.

Design precedent for this Rust-core / multi-language-binding shape already exists in this
project family: `general-mna`'s README documents the same pattern, and `general-simulator`
already ships `pyblock-ffi` and `cscript-ffi` around its own Rust core.

## Status

Design in progress; no schema/interpreter/document code has landed yet. The crates currently
expose only a `version()` smoke-test function so the PyO3/wasm-bindgen build pipelines exist
before there is real surface to bind against.

## Relationship to the sibling repos

```
Électronique/
├── gsim-core/                (this repo)
├── general-simulator/        Rust LCP/PWL circuit+block simulator, a future compile target
├── general-mna/              symbolic MNA + converter averaging (general-simulator's dependency)
├── spice-lsp/                general-spice-core: the ngspice/Xyce netlist parser
└── general-simulator-gui/    frontend + Python backend; will consume gsim-wasm/gsim-py,
                               superseding packages/gsim-model once Phase 1 lands
```

`gsim-core` does not depend on any of these; it is a leaf dependency they will consume.

## Development

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Continuity log: [`docs/journal/`](docs/journal/). Read the newest entry before continuing prior
work. Lasting design rationale: [`docs/decisions/`](docs/decisions/).

## No `eval`

The expression DSL (`${{ }}`, used for parametric ports/geometry/validation) is a
closed-grammar, pure-data interpreter by design — no dynamic code execution of any kind, in any
language binding. This is a hard constraint, not a starting point to relax later: component
contracts are expected to be loaded from third-party/community libraries, and arbitrary code
execution in a loaded schematic-component definition is not an acceptable risk.
