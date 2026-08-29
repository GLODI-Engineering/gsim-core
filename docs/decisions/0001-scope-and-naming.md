# 0001 — Scope and naming

**Status:** accepted (2026-08-29).

## Context

`general-simulator-gui`'s frontend draws schematic components through three parallel
mechanisms (a hand-authored JSX registry, a generic metadata-driven primitive renderer, and a
handful of raw imported SVGs), and separately maintains a Python library
(`packages/gsim-model`) that builds the same `.gsim_sch` schematic files programmatically. Both
need the same underlying logic — what a component's ports/geometry/parameters are, and what a
valid schematic looks like — but today that logic exists only per-language, per-mechanism, with
no shared implementation. This repo is the shared implementation.

## Decision: naming

"GSF" already names something in `general-simulator-gui`:
`docs/specs/gsim-symbol-format-spec.md` defines "GSF — Gsim Symbol Format" as a single-*component*
authoring format, fully implemented in `packages/gsim-model/src/gsim_model/symbol.py` (primitive
builders, transform chaining, `SymbolBuilder`, `ComponentDefinition`, `.gsim_sym` file I/O,
`merge_into_catalog()`). It is real, tested, in production use — and out of scope to rename or
break.

This repo is therefore named `gsim-core`, not `gsf-core`, to match the ecosystem's actual
prevailing name (`.gsim`, `.gsim_sch`, `gsim_model`, `gsim_loader.py`) rather than introduce a
second acronym that collides with an existing, narrower meaning. Its crates are `gsim-core`
(the shared logic), `gsim-py` (PyO3 bindings), `gsim-wasm` (wasm-bindgen bindings).

File extensions are **not** renamed by this decision: `.gsim_sym` stays the name for one
component's definition, `.gsim_sch` stays the name for a full schematic. The bare `.gsim`
extension is retired (it was the v1 flat-schematic format `.gsim_sch` v2 explicitly superseded,
"hard cutover, no migration shim") and is not reclaimed for a third, unrelated meaning. "GSF"
and "gsim" are used in this repo's own docs only as conceptual/project-level names ("GSF v2" =
the new parametric component-contract design), not as new file extensions.

## Decision: what GSF v2 changes versus GSF v1

GSF v1 (`gsim_model/symbol.py`) resolves every transform (`translate`/`rotate`/`scale`/`mirror`)
to absolute, fixed coordinates once, at authoring time — confirmed by reading the implementation
directly, not inferred from the spec. Its own docstring states this is deliberate: "no runtime
transform evaluation is needed." There is no mechanism anywhere in the codebase for a
component's port count or geometry to depend on a parameter value.

GSF v2 (designed here, not yet implemented) adds exactly that: ports and geometry can be
functions of parameter values, evaluated **live** while a user edits a component's parameters in
the running frontend (e.g. a Sum block's `signs` parameter growing from `"++"` to `"++-"` adds a
third input port immediately, without recreating the block). This is additive to GSF v1's
existing static case, not a replacement of it — a component with no expressions in its
definition is just an ordinary static component, unchanged.

### Expression DSL shape

Chosen: YAML-native control blocks with inline `${{ ... }}` expressions (GitHub-Actions-style),
not raw nested JSON-Logic-style operator objects (`{"multiply": [...]}`) for every arithmetic
operation. Example (a variadic Sum block):

```yaml
ports:
  for_each: "${{ split(params.signs, '') }}"
  as: sign
  index: i
  emit:
    id: "in${{ i }}"
    label: "${{ sign }}"
    side: left
    kind: signal
```

Rules settled:
- `${{ }}` is the only place expressions appear; everything else is plain YAML data.
- `for_each` / `as` / `index` / `emit` is the one repetition construct, usable under `ports:` or
  nested inside `primitives:` — no separate mechanism for "repeat a port" vs. "repeat a drawing
  element."
- `when: "${{ ... }}"` is the equivalent conditional guard for optional primitives/ports.
- The expression grammar is small and closed: dotted `var` lookups, arithmetic/comparison/
  boolean operators, string interpolation, and a fixed whitelist of pure functions (`split`,
  `count`, `concat`, `max`, `min`, `matches`, extended only as real needs appear). No loops
  beyond `for_each`, no user-defined functions, no I/O.
- **No `eval`, in any language binding, ever.** Component contracts are expected to be loaded
  from third-party/community libraries; arbitrary code execution in a loaded definition is not
  an acceptable risk. Parameter validation uses the same closed expression grammar (e.g.
  `validate: "${{ matches(value, '^[+-]{1,8}$') }}"`) rather than a separate "JS validation
  script" escape hatch.
- Underneath, an expression compiles to a pure, portable JSON-serializable AST — the same tree
  evaluates identically in the Rust core now, and would evaluate identically in a from-scratch
  reimplementation in another language later, without carrying an embedded scripting runtime
  (QuickJS or similar) into three languages.

### Port identity across live geometry changes

Because port count can change while a user is actively editing (not only at component-creation
time), two rules were settled to keep wires from silently misbehaving:
- **Generated port ids are index-based and stable** — removing input 2 never renumbers input 3
  to become input 2; ids are never reassigned to a different logical port.
- **Removing a port with a live wire attached leaves that wire dangling with an error marker**
  in the consuming frontend (rather than auto-deleting the wire or blocking the parameter edit).
  The frontend's existing `apps/frontend/src/problems/problems-store.ts` already implements this
  exact pattern for a different case (`setCompatWarnings`); the same pattern extends with an
  optional `wireId`/`portId` on `ProblemItem` rather than needing a new subsystem.

## Decision: taxonomy — simulator target can mean a different physical model

`general-simulator` (the Rust LCP/PWL engine) models a diode and a MOSFET-like switch as
piecewise-linear, with parameters (`g_breakdown`, `v_breakdown`, `g_off`, `v_th`, `g_on`, `Ron` +
body diode) that are not the same as a real SPICE exponential diode or a BSIM MOSFET model
targeting ngspice/Xyce. Mid-conversation, these were renamed in `general-simulator` (and
cascaded into `general-mna`, `spice-lsp`'s grammar doc, and a private, non-public project
experiments) from "diode"/"mosfet" to "ideal diode"/"ideal switch", specifically to stop
implying they are the same model as a real diode/MOSFET.

Consequence for this repo's taxonomy design: a component targeting `general-simulator`'s ideal
diode/switch must be a **distinct catalog leaf** from `semi.diode`/`semi.mosfet.n` (which target
ngspice/Xyce), not a parameter variant or a `simulators:` mapping entry on the same leaf. Folding
both into one taxonomy entry would silently conflate two different physical models — exactly the
kind of contract bug `general-simulator-gui`'s own
`docs/simulator-component-integration.md` already warns about for `BLOCK_TO_SPICE`.

## Decision: staged scope, not one-shot

An audit of `packages/gsim-model` (full source read, not just filenames) found netlist
compilation is not part of that package at all — it lives in the substantially larger
`packages/simulator/src/simulator/compiler/` pipeline (`ir1_builder`, `ir2_lowering`,
`spice_netlist_writer`, `dae_system`, `scipy_compiler`, `physical_network_lower`,
`signal_flow_lower`, `subckt_lower`, `converter_lower`). Absorbing that into `gsim-core` in the
same pass as the component-contract and document-model work was judged too large a single step.

**Phase 1** (this repo, starting now): GSF v2 component-contract schema/interpreter, the
`.gsim_sch` document/builder model (superseding `packages/gsim-model`), and real autolayout
(auto-spacing/collision-avoidance — confirmed to not exist anywhere today; `gsim_model/layout.py`
is pure port-geometry math despite its name, not autolayout).

**Deferred to a later, separately-scoped phase**: absorbing the netlist-compiler pipeline.

## Gaps noted for Phase 1 to resolve deliberately, not inherit silently

- `packages/gsim-model`'s `ModelBuilder` has zero support for authoring subsystem/hierarchy
  blocks (`SubsystemModel` exists in the schema; no `add_subsystem_block()` exists in the
  builder) despite hierarchy being a first-class v2 schema feature
  (`docs/specs/gsim-sch-format.md` §11).
- `CrossParadigmWireError` is referenced in `gsim_model` comments/docstrings as if it is an
  existing, enforced error, but is not defined anywhere in that codebase. Phase 1 must either
  implement this validation for real or explicitly decide it is not needed — not carry forward
  a reference to something that doesn't exist.
- `packages/gsim-model` does not validate a document against the full `.gsim_sch` JSON Schema at
  build time — only in its own test suite, against the real schema file, as a drift check.
  Phase 1's document model should decide explicitly whether to enforce full schema conformance
  at construction time (stricter) or keep the "fast-fail subset + external full check" split
  `gsim_model` uses today.
