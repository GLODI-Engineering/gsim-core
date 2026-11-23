# 0001 — Scope and naming

**Status:** accepted (2026-08-29); the taxonomy decision amended 2026-11-21 (see the amendment at the end).

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
- **Removing a port with a live wire attached leaves that wire dangling with an error marker**
  in the consuming frontend (rather than auto-deleting the wire or blocking the parameter edit).
  The frontend's existing `apps/frontend/src/problems/problems-store.ts` already implements this
  exact pattern for a different case (`setCompatWarnings`); the same pattern extends with an
  optional `wireId`/`portId` on `ProblemItem` rather than needing a new subsystem.
- **Generated port ids must be stable — never reassigned to a different logical port.** The
  original phrasing here ("index-based and stable") overclaimed: implementing and testing it
  (`contract::tests::a_bare_string_param_only_gives_positional_not_true_identity_stability`,
  `gsim-core` commit `b9cd690`) showed a `for_each` over a *bare* array/string param (`signs:
  "++-"` or `signs: [1, 1, -1]`) only gives stability across **append/remove-at-the-end** —
  removing an element from the *middle* does shift every later element's derived id, since a
  bare array element has no identity beyond its position.
  **Fixed, not just documented as a limitation**: when true stability across interior
  insert/remove is needed, author the driving parameter as an **array of objects, each carrying
  its own persistent `id`** (e.g. `operands: [{"id": "s1", "sign": 1}, {"id": "s2", "sign": -1}]`)
  and have `for_each` bind directly to each object (`as: "operand"`), with `emit` referencing
  `${{ operand.id }}` instead of a loop index. This needed **no engine change** — dotted field
  access on a `for_each`-bound value was already general — and is proven end-to-end by the
  `control.sum.stable_ids` fixture and its
  `stable_id_pattern_survives_removing_an_interior_operand` test (`gsim-core` commit `f15982c`'s
  follow-up, 2026-09-04): removing the middle operand leaves every surviving operand's port id
  unchanged. **This is now the recommended pattern** for any component whose port-generating
  parameter needs to support interior edits (not just growing/shrinking at the end) — a bare
  array/string remains fine for append/trim-only editing (e.g. a UI that only offers "+"/"−"
  buttons at the end of a list). Assigning each new element a persistent id at creation time
  (e.g. a UUID or an incrementing counter) is the frontend/document-layer's responsibility, not
  this crate's — `gsim-core` only guarantees that *if* given a stable id, it preserves it.

## Decision: taxonomy — simulator target can mean a different physical model

`general-simulator` (the Rust LCP/PWL engine) models a diode and a MOSFET-like switch as
piecewise-linear, with parameters (`g_breakdown`, `v_breakdown`, `g_off`, `v_th`, `g_on`, `Ron` +
body diode) that are not the same as a real SPICE exponential diode or a BSIM MOSFET model
targeting ngspice/Xyce. Mid-conversation, these were renamed in `general-simulator` (and
cascaded into `general-mna`, `spice-lsp`'s grammar doc, and a sibling experiments repo)
from "diode"/"mosfet" to "ideal diode"/"ideal switch", specifically to stop
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

## Amendment — the distinct leaf is a distinct *group*, `ideal.*` (#60)

**2026-11-21.** The taxonomy decision above says a component targeting `general-simulator`'s
ideal diode/switch "must be a **distinct catalog leaf** from `semi.diode`/`semi.mosfet.n` […]
not a parameter variant". It did not say where the distinction lives, and the library put it in
the leaf word: `passive.diode.ideal` and `passive.switch.ideal`.

That turned out to be the wrong half of the name to carry it, for two separate reasons.

**The group was false.** `passive.switch.ideal`'s own `TypeSpec` declares `node_ports`
`["d", "s"]` — drain and source — a block-driven `ctrl` mapped to a third terminal `g`, a
`v_gs_th` gate threshold, and an intrinsic body diode with its own `body_v_f`,
`body_v_breakdown`, `body_g_on` and `body_g_off`. A three-terminal gate-driven active device
with a body diode was filed next to the resistor. `passive.diode.ideal` is passive in the
energy sense but is not a lumped R/L/C element, which is the only thing `passive.` was ever
describing.

**The obvious fix would have broken this very decision.** `semi.diode.ideal` reads as a
parameter variant of `semi.diode` — precisely the conflation the paragraph above exists to
prevent. Moving the qualifier to the leaf's end cannot express "different model", because leaf
qualifiers are how parameter variants are spelled everywhere else in the taxonomy.

So the distinction goes in the group:

| was | is |
|---|---|
| `passive.diode.ideal` | `ideal.diode` |
| `passive.switch.ideal` | `ideal.switch` |

`ideal.*` names general-simulator's piecewise-linear ideal-device family for what it is. It
cannot be read as a SPICE model's variant, it leaves `semi.diode` and `semi.mosfet.n` unclaimed
for a real SPICE target, and it leaves `passive.` holding exactly the six lumped elements it
always meant: resistor, capacitor, inductor, `inductor.coupled`, transformer,
`mutual_inductor`.

**The leaf words are unchanged on purpose.** The emitted netlist still carries
`kind=ideal_switch`, so nothing desynchronises from `general-simulator`, `general-mna` or
`spice-lsp`'s grammar. Verified rather than assumed: every example in `examples/` was run after
the rename and all four that name a renamed type emit a netlist byte-identical to its committed
`.cir`.

**The old ids still open a saved sheet.** The model behind each is unchanged, so
`passive.diode.ideal` and `passive.switch.ideal` are kept as aliases and resolve to the same
`TypeSpec`. What is *not* kept is `passive.switch.voltage`: a voltage-controlled switch is not
the ideal PWL switch, and an alias that answers for it is the conflation this decision forbids.

**What this cost.** The whole workspace suite stayed green through the rename — nothing pinned
either name, in a repo whose entire purpose is to be the shared source of those names. The
guards now exist in `tests/the_ideal_devices_are_not_passive.rs`, and each was watched fail
before it was trusted.

