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

Phase 1 deferred netlist compilation deliberately. It has since landed, in
[`compile`](crates/gsim-core/src/compile/): a `.gsim_sch` document compiles to
`general-simulator`'s own dual-grammar netlist (SPICE element cards and `kind=` block cards from
one registry), with real `.SUBCKT` scoping, so a def's internal nets stay invisible to its
parent. An unknown block type is a fatal error rather than a skipped line — a silently dropped
component is a wrong circuit that still simulates.

[`compile::capabilities`](crates/gsim-core/src/compile/capabilities.rs) exports that registry as
data, so a front end renders what the engine declares instead of keeping its own list. That list
had drifted all the way: of 143 types in `general-simulator-gui`'s catalogue, **zero** matched
its hand-written "supported" list, because the list spelled types the way the registry does and
schematics spell them the way the catalogue does. Generating the manifest makes the drift
impossible rather than merely discouraged.

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
  its only job is schematic construction and netlist compilation.
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

All three Phase 1 deliverables are implemented — the GSF v2 contract schema with its
`${{ }}` expression language, the `.gsim_sch` document model, and autolayout — and netlist
compilation, which Phase 1 had deferred, has landed on top of them.

```toml
[dependencies]
gsim-core = "0.3"
```

Module map: `expr` and `template` (the closed-grammar expression language), `resolve`
(`for_each`/`when`/`emit` expansion of dynamic port and primitive lists), `port` and `primitive`
(the resolved static shapes), `contract` (GSF v2 and `evaluate_component`), `document`
(`.gsim_sch` build and validation), `layout` (autolayout), `compile` (netlist emission).

Worked schematics, each with the netlist it compiles to, are in
[`examples/`](examples/README.md); they drive the crate through its `gsim-py` binding.

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
