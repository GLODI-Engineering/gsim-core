# gsim-core

[![crates.io](https://img.shields.io/crates/v/gsim-core.svg)](https://crates.io/crates/gsim-core)
[![docs.rs](https://img.shields.io/docsrs/gsim-core)](https://docs.rs/gsim-core)
[![license: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

`gsim-core` is one implementation of what a valid schematic is and what it compiles to: the
component contract, the `.gsim_sch` document model, autolayout, and netlist emission. The
schematic editor, the Python automation library and anything else that has to agree about a
schematic use this crate rather than keeping a copy of the rules — which is what the editor's
TypeScript front end and its Python model package used to do, in parallel.

## Scope (Phase 1)

- **GSF v2** — a parametric component-contract schema: ports, geometry, and parameter
  validation that can depend on parameter values and are evaluated *live* while a user edits
  (e.g. a Sum block gaining a port when its `signs` parameter grows). This supersedes the
  editor's `.gsim_sym` / GSF v1 format, which is fully implemented but bakes primitives to
  fixed coordinates once, at authoring time, by explicit design — confirmed by reading that
  implementation before starting this repo.
- **`.gsim_sch` document model** — block/wire/hierarchy construction and validation,
  superseding the editor's Python-only document builder.
- **Autolayout** — real auto-spacing/collision-avoidance placement helpers. These existed
  nowhere beforehand: the editor's layout module is pure port-geometry math (port centres, a
  single-bend wire router), not autolayout. This matters
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
had drifted all the way: of 143 types in the editor's catalogue, **zero** matched its
hand-written "supported" list, because the list spelled types the way the registry does and
schematics spell them the way the catalogue does. Generating the manifest makes the drift
impossible rather than merely discouraged.

Full rationale and everything settled before writing code: [`docs/decisions/`](docs/decisions/).

## Why one implementation

Hand-keeping the same contract-evaluation and document-validation logic in TypeScript and
Python separately is exactly the kind of two-implementation drift the editor's own integration
document records as a real, recurring risk (the `BLOCK_TO_SPICE` mapping precedent).
`gsim-core` exists once; two thin binding crates expose it:

- `crates/gsim-py` — PyO3 bindings, and the successor to the editor's pydantic-based Python
  model package, which was a real, tested library rather than a stub. `gsim-py` is the
  automation library a user or agent uses to build a schematic entirely from code, with no
  matplotlib/pandas/etc: its only job is schematic construction and netlist compilation. It is
  what [`gsim-tools`](https://github.com/GLODI-Engineering/gsim-tools) is built on.
- `crates/gsim-wasm` — wasm-bindgen bindings, for the editor's front end: its own `.gsim_sch`
  serialization, and evaluating the GSF v2 contract to paint a component's ports and geometry.
  The front end still owns all actual SVG/DOM painting.

The same `gsim-wasm` binding also makes a Node.js/JS automation library (a JS equivalent of
`gsim-py`) essentially free — worth building once there is real demand, since it needs no new
Rust-side work.

Design precedent for this Rust-core / multi-language-binding shape already exists in this
project family: `general-mna`'s README documents the same pattern, and `general-simulator`
already ships `pyblock-ffi` and `cscript-ffi` around its own Rust core.

## Status

All three Phase 1 deliverables are implemented — the GSF v2 contract schema with its
`${{ }}` expression language, the `.gsim_sch` document model, and autolayout — and netlist
compilation, which Phase 1 had deferred, has landed on top of them. Published at **0.5.0**,
together with both binding crates.

```toml
[dependencies]
gsim-core = "0.5"
```

### The bindings

A crates.io release of a binding crate is source, not an installable artifact: `gsim-py` has to
be built into a wheel and `gsim-wasm` into a JS package. Both are one command from a checkout.

```bash
git clone https://github.com/GLODI-Engineering/gsim-core
cd gsim-core

# Python: a wheel for the interpreter you intend to use it from.
pip install maturin
maturin build --release -m crates/gsim-py/Cargo.toml -i python3
pip install "$(ls -t target/wheels/gsim_py-*.whl | head -1)"

# JavaScript: an ES module package in crates/gsim-wasm/pkg/.
cargo install wasm-pack          # once
wasm-pack build crates/gsim-wasm --target web
```

The crate is not abi3, so a wheel is built for one Python version at a time — pass the
interpreter you want with `-i`.

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
├── general-simulator/        Rust LCP/PWL circuit+block simulator — the one implemented
│                               compile target
├── general-mna/              symbolic MNA + converter averaging (general-simulator's dependency)
├── spice-lsp/                general-spice-core: the ngspice/Xyce netlist parser
└── general-simulator-gui-for-vscode/
                              the schematic editor, a VS Code extension; consumes gsim-wasm
```

`gsim-core` does not depend on any of these; it is a leaf dependency they consume.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude gsim-py --all-targets -- -D warnings
cargo test --workspace --exclude gsim-py

# gsim-py links libpython, so its test binary needs the extension-module
# feature off -- a plain `cargo test --workspace` fails to link it.
cargo clippy -p gsim-py --all-targets --no-default-features -- -D warnings
cargo test -p gsim-py --no-default-features
```

Continuity log: [`docs/journal/`](docs/journal/). Read the newest entry before continuing prior
work. Lasting design rationale: [`docs/decisions/`](docs/decisions/).

## No `eval`

The expression DSL (`${{ }}`, used for parametric ports/geometry/validation) is a
closed-grammar, pure-data interpreter by design — no dynamic code execution of any kind, in any
language binding. This is a hard constraint, not a starting point to relax later: component
contracts are expected to be loaded from third-party/community libraries, and arbitrary code
execution in a loaded schematic-component definition is not an acceptable risk.
