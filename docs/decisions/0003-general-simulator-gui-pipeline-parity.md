# 0003 — What parity with `general-simulator-gui`'s live pipeline actually requires

**Status:** accepted (2026-09-04). Findings, not a plan — this records what was measured, so the
roadmap is argued from numbers rather than from an assumption about how close the two sides are.

## Context

`gsim-core` was built to become the one implementation behind `general-simulator-gui`'s schematic
tooling, with Phase 2 (`compile/`) targeting `general-simulator`'s netlist grammar. A request came
in to *consolidate*: route `general-simulator-gui`'s live "Run" path through `gsim_py.compile()`
and stop maintaining the parallel Python compiler in
`general-simulator-gui/packages/simulator/src/simulator/compiler/`.

Before doing that, the Python pipeline was read in full and the gap was **measured with a built
`gsim-py` wheel against that repo's own real fixtures**, not estimated. The result is that the
swap is not currently possible at all, for a reason more structural than "some block types are
missing." This document records why, and what the ordered prerequisites are.

## Finding 1 — the two sides do not share a simulator

This is the blocker that subsumes every other one.

`gsim-core` has exactly one `NetlistTarget`. `compile::compile()` matches `"general-simulator"` and
returns `CompileError::UnknownTarget` for anything else. `AGENTS.md` invariant 2 already says
ngspice/Xyce targets are unstarted.

`general-simulator-gui`'s live simulate path — `apps/kernel-sidecar/src/kernel_sidecar/ipc/handlers/simulate.py`
— dispatches to exactly three backends, and **none of them is `general-simulator`**:

| Model class | Compiler | Solver |
|---|---|---|
| any block with a circuit prefix | `spice_netlist_writer.write_netlist` | `ngspice` or `Xyce` subprocess |
| pure signal-flow | `ir2_lowering` + `scipy_compiler` | `scipy.integrate.solve_ivp` |
| user-pasted netlist text | — | `ngspice` or `Xyce` subprocess |

`packages/simulator/src/simulator/solvers/` contains `ngspice_solver.py`, `xyce_solver.py`,
`scipy_solver.py` and nothing else. The string "general-simulator" appears in that repo only as the
*project's own name* (netlist header comments, XDG directory paths) — never as a solver.

So `spice_netlist_writer.py` and `signal_flow_spice_lower.py` do not emit `general-simulator`'s
`kind=` grammar. They emit an **ngspice/XSPICE deck**: `R`/`C`/`L`/`V`/`D`/`Q`/`M`/`S`/`W`/`E`/`B`
element cards, `.MODEL`, `.PARAM`, `.IC`, `.SUBCKT`, `.TRAN`/`.AC`/`.DC`/`.OP`, `.END`, plus XSPICE
`A`-device instances (`int`, `d_dt`, `s_xfer`, `limit`, `slew`, `hyst`, `pwl`, `astate`,
`adc_bridge`, `dac_bridge`, `d_dff`, `d_srff`, `d_pulldown`). Substituting `gsim_py.compile()` at
those call sites would hand ngspice a netlist in a grammar it cannot parse.

**Prerequisite: an `NgspiceTarget` (and a `XyceTarget`) implementing `NetlistTarget`.** Everything
below is downstream of that.

## Finding 2 — the type vocabularies are disjoint, measured

A `gsim-py` wheel was built (`maturin build --release`) and run against all three `.gsim_sch`
documents that exist in `general-simulator-gui`. Every one **loads** and **none compiles**:

| Fixture | `load_document` | `compile(model, "general-simulator")` |
|---|---|---|
| `docs/circuit_test/buck_converter/buck_converter.gsim_sch` | OK | `unknown-type: block 'vin' has type 'physical_electrical.source.voltage.dc'` |
| `docs/circuit_test/buck_converter/buck_converter_synchronous.gsim_sch` | OK | `unknown-type: block 'm1' has type 'physical-electrical.semiconductor.mosfet_n'` |
| `tests/fixtures/gsim_sch/comprehensive.gsim_sch` | OK | `unknown-type: block 'const1' has type 'signal.constant'` |

Each fails on its *first* block. `REGISTRY` currently holds 55 type names; the GUI's live pipeline
recognises 25 distinct `physical-electrical.*` types across 39 `(type, fidelity)` pairs
(`spice_card_emitters._V2_REGISTRY`), plus 38 `signal-flow.*` types
(`signal_flow_spice_lower.SIGNAL_FLOW_TYPES`), plus `spice-subckt.instance`, plus a legacy v1
fallback set. The names overlap on four strings (`passive.resistor`, `passive.capacitor`,
`passive.inductor`, `source.voltage.dc`) and are otherwise disjoint:

| The GUI's documents say | `REGISTRY` says |
|---|---|
| `signal.constant` | `source.const` |
| `control.integrator` | (none — `control.transfer_function` is the nearest) |
| `converter.signal_to_physical_signal` | `converter.sig2phys` |
| `converter.physical_signal_to_signal` | `converter.phys2sig` |
| `source.voltage.pulse` | `source.pulsewave` |
| `passive.switch.voltage` | `passive.switch.ideal` |
| `physical_electrical.semiconductor.diode` | `passive.diode.ideal` |
| `physical-electrical.semiconductor.mosfet_n` / `_p` | (none) |
| `physical-electrical.semiconductor.bjt_npn` / `_pnp` | (none) |
| `physical-electrical.opamp.*` (4 types × 2 tiers) | (none) |
| `physical-electrical.probe.voltmeter` / `.ammeter` | (none) |
| `physical-electrical.passive.transformer` / `.mutual_inductor` | (none) |
| `sinks.scope` | (none) |
| `hierarchy.subsystem.virtual` | (none — see Finding 4) |

`REGISTRY`'s names came from `general-mna`/`general-simulator`'s own engine vocabulary; the GUI's
came from its block catalogue under `packages/blocks/src/blocks/`. Neither is wrong, but there is
no mapping layer, and adding one is a real design decision (translate in `gsim-core`? alias entries
in `REGISTRY`? change the catalogue?) rather than a mechanical fill-in.

Note also that the GUI's documents are internally inconsistent about separators —
`physical-electrical.*` (hyphen) and `physical_electrical.*` (underscore) both appear, in the two
buck fixtures respectively. Any mapping layer has to absorb that.

**This inconsistency is not only a `gsim-core` problem — it is already broken *inside* the Python
pipeline.** Running that repo's full gate with real solver paths surfaced
`test_buck_simulation.py::test_buck_gsim_model_xyce` failing with `UnsupportedBlockError: No SPICE
writer for ('physical-electrical.source.voltage.dc', 'ideal')`: its block catalogue carries *two*
directories for one component (`source/voltage_dc/` and `source/voltage/dc/`) and only the first
has an emitter. Filed there as `BG-12`. Worth knowing before designing a mapping layer against
that catalogue — it is not yet a single consistent source of truth.

## Finding 3 — `fidelity` has no counterpart

`spice_card_emitters` keys its registry on `(block_type, fidelity)`, with `"ideal"`,
`"behavioral"` and `"spice"` tiers selecting genuinely different cards for the same block type.
`gsim-core`'s `Block` carries a `fidelity` field but `compile::spec::lookup()` keys on `type` alone
and never reads it. Any ngspice target has to decide whether tiers are separate `TypeSpec`s or a
new dimension on the lookup.

## Finding 4 — the Python pipeline has six load-time passes with no `gsim-core` equivalent

These run *before* netlist writing in `general-simulator-gui`, and are the substantive part of what
"the logic" means in "I want all the logic in one place". None exists in `gsim-core` today:

1. **`gsim_sch_flatten.py`** (401 lines) — inlines `hierarchy.subsystem.*` blocks into one flat
   block/wire list, with duplicate-id detection (§17.1.3) and explicit rejection of the iterator
   and function-call subsystem variants (§11.2). `gsim-core` knows `Paradigm::Hierarchy` exists but
   has no flattening pass, and `0001-scope-and-naming.md` recorded subsystem authoring as out of
   scope.
2. **`dim_propagate.py`** (571 lines) — fixed-point signal-dimension inference across signal-flow
   wires from each block's catalogue `port_sigs`, raising distinct `DimensionMismatchError` /
   `DimensionUnresolvedError` / `DimensionLoopError`. No `gsim-core` analogue; scalar is assumed
   throughout.
3. **`param_expression.py` + `param_substitution.py`** (536 lines) — a closed-grammar evaluator for
   §5.2 param expressions (`+ - * / **`, `sin cos tan exp log log10 sqrt abs min max pow`, `pi`,
   `e`, cross-references to other params) with cycle detection, plus cross-scope `.PARAM`
   substitution. `gsim-core`'s `document/model.rs` explicitly states `.gsim_sch`'s `{...}`
   param-reference syntax "is not evaluated by this crate in Phase 1" — it is a *different*
   mechanism from this crate's `${{ }}` GSF v2 grammar and is currently unimplemented.
4. **`unit_table.py`** (180 lines) — a curated engineering unit table (V/kV/mV/µV, A/mA/µA/kA,
   Ω/kΩ/MΩ/mΩ, F/µF/nF/pF, H/mH/µH, W/kW/MW/mW, J/kJ/MJ, K/°C, rad/°) driving
   `unit_conversion` block lowering, with `UnknownUnitError`. No analogue.
5. **`routing_lower.py`** (496 lines) — `signal-flow.routing.{mux,demux,selector,bus_creator,
   bus_selector,bus_assignment,merge,manual_switch,multiport_switch,switch,data_store_memory,
   data_store_read,data_store_write}` with build-time `DimMismatchError`,
   `SelectorOutOfRangeError`, `UnresolvedDataStoreError`. `gsim-core`'s `compile/routing.rs` covers
   only Goto/From tag resolution (§10), which *is* at parity and is genuinely good — but it is one
   of thirteen routing concerns here.
6. **`simulation_extract.py`** (215 lines) — reads `model.simulation` into a typed `SimulationPlan`,
   expands parameter sweeps (linear and by-values, Cartesian product across multiple sweeps on one
   analysis), and rejects `NOISE`/`TF` with `UnsupportedAnalysisError`. `gsim-core`'s compiler
   never reads `model.simulation` or `model.params` at all — a `grep` over `compile/` for either
   returns nothing.

## Finding 5 — no analysis directives, `.MODEL`, `.PARAM` or `.IC` are emitted

`GeneralSimulatorTarget::emit` produces a header comment, `.subckt`/`.ends` bodies, element and
`kind=` cards, and `X` cards. It emits no analysis line, because `general-simulator` takes the
analysis as CLI flags (`--mode transient --tfinal --dt`). ngspice does not — it needs a
`.TRAN`/`.AC`/`.DC`/`.OP` card in the deck, which is what `spice_netlist_writer._analysis_line_v2`
builds from an `AnalysisRequest`.

`spice_netlist_writer._write_netlist_v2` additionally emits three card classes with no `gsim-core`
counterpart, each carrying a real decision worth porting rather than rediscovering:

- **`.PARAM`** — one per `ir1.params` entry, in declaration order.
- **`.MODEL`**, with a genuinely careful diode rule. If a diode block sets `Is`/`N`/`Rs`
  overrides, it gets its *own uniquely named* `.MODEL` so two differently-parameterised ideal
  diodes never collide under one name. If it names no model (or names the internal default `"D"`),
  one shared ideal `.MODEL D D(Is=1n N=1 Rs=0.01)` is generated. If it names anything else with no
  overrides, **no `.MODEL` is generated at all** — the name is assumed to reference a real
  manufacturer part supplied via an external `.lib`/`.include`, and auto-generating would silently
  shadow it with a fake ideal one. BJT/MOSFET models are deduplicated by name into
  `.MODEL <name> NPN|PNP|NMOS|PMOS`.
- **`.IC V(node)=value`** — from `ir1.simulation.initial_conditions` entries with `kind == "node"`.

## Finding 6 — `scipy_compiler.py` is redundant *in principle*, not *in practice*

This was the open architectural question, so it was tested directly rather than reasoned about.

**`general-simulator` can simulate a pure signal-flow model with zero circuit elements.** A
two-line netlist with no element cards at all —

```
REF kind=const value=1
LAG kind=tf num=[1] den=[0.001,1] in=REF
```

— run through the built `general-simulator` binary at `--dt 1e-4` returns `LAG = 0.6321202256` at
t = 1 ms against the analytic `1 − e⁻¹ = 0.6321205588`: correct to six digits. `dae-runtime` folds
the `continuous-blocks` descriptor-DAE fragments without needing a circuit to attach them to, so
the model class is genuinely supported. `scipy_compiler.py` is **not** covering for a capability
`general-simulator` lacks.

But it is not redundant today, for three separate reasons:

1. **It is not a netlist writer.** `scipy_compiler.compile()` (110 lines) emits a Python closure
   `f(t, x) -> dx/dt` that walks `ir2.eval_order`, calling each block's
   `equations.derivatives(t, inputs, state, params)` — methods on classes loaded from
   `packages/blocks/`'s per-block `equations.py` templates via
   `equation_templates.loader.get_registry()`. Its "block library" is Python code in the GUI repo,
   not a fixed set of engine primitives. Replacing it means every one of those templates must have
   an engine-side `kind=` equivalent; the sinks (`sinks.scope`, wired as `ObservationHook`s) have
   no netlist spelling at all.
2. **Different numerics.** `scipy_solver.py` calls `solve_ivp` with a caller-selectable `method`
   (default `Radau`), `rtol`, `atol`, `dense_output=True` and adaptive stepping.
   `general-simulator` runs trapezoidal with backward Euler on the first step and on any
   LCP-resolved mode change, at a **fixed** `--dt`. A model tuned for adaptive stiff integration is
   not guaranteed to reproduce under fixed-step trapezoidal, and the GUI exposes `method`/`rtol`/
   `atol` in its request payload today.
3. **`ir2_lowering.py` carries real semantics the netlist path does not.** Its topological sort
   deliberately treats a stateful block's output as state-derived, so it does *not* create a
   current-step dependency — which is what lets feedback through an integrator pass while a true
   algebraic loop (a feedback path with no stateful block) is rejected with a named error listing
   the offending block ids.

**Conclusion:** consolidating the pure-signal-flow path onto `gsim-core` → `general-simulator` is
architecturally sound and worth doing *eventually*, but it is a block-library port plus a numerics
change, not a call-site swap. It must not be bundled into the netlist-target work.

## Finding 7 — validation behaviour worth preserving verbatim

Measured with the built wheel, `gsim_py.load_document` vs `gsim_sch_loader.load`:

| Check | `gsim_sch_loader` | `gsim_py.load_document` |
|---|---|---|
| float coordinate | rejected, message names `model.blocks[N].position[I]` | rejected, message names `line 1 column 151` |
| invalid `model.role` | rejected, lists allowed values | rejected, lists allowed values |
| `schema_version != 2` | rejected | rejected |
| param expression cycle `a→b→a` | rejected (`ParamCycleError`, prints the chain) | **accepted silently** |
| param expression needing evaluation | resolved to a float | **accepted, left as the string `"a*3"`** |
| JSON Schema validation | `jsonschema.validate` against `packages/schemas/definitions/model/gsim_sch.schema.json` | none (serde struct shape only) |

So `gsim-core` is close on structural validation and **not a substitute** on §5.2 params: it neither
evaluates nor cycle-checks them, and `resolved_params` — which every downstream Python stage
consumes — has no counterpart. Two things to carry across when this is closed:

- **Error messages must name the JSON path**, not the byte offset. `model.blocks[3].position[1]` is
  actionable in a GUI; `line 1 column 151` is not, and the GUI has no original-text offsets to map
  it back with after a canvas edit.
- The GUI's other named error types are worth mirroring rather than collapsing into one:
  `CrossParadigmWireError` (a signal wire touching a physical block with no `converter.*` between —
  `converter_lower.py`), `UnsupportedSubsystemError`, `UnsupportedAnalysisError`, and
  `UnsupportedSignalFlowParamError`. That last one is a good pattern specifically: it is raised for
  `signal_flow.lookup.2d`, `signal_flow.discrete.zero_order_hold`, and PID with `kd != 0`, each with
  a message explaining *why* the mapping does not exist rather than emitting something subtly wrong.

## Ordered prerequisites

1. `NgspiceTarget: NetlistTarget` — element cards, `.MODEL`/`.PARAM`/`.IC`, an analysis directive
   fed from `model.simulation`, and the XSPICE `A`-device vocabulary for signal-flow blocks.
2. A type-vocabulary decision: mapping layer, alias entries, or catalogue change (Finding 2),
   including the hyphen/underscore inconsistency.
3. `fidelity` in spec lookup (Finding 3).
4. §5.2 param evaluation + cycle detection, with JSON-path error messages (Findings 4.3, 7).
5. Subsystem flattening, dimension propagation, routing lowering, unit table (Findings 4.1, 4.2,
   4.4, 4.5).
6. Only then, separately, the `scipy_compiler` question (Finding 6) — which is a block-library port
   and a numerics decision, not part of this line of work.

Until (1) and (2) land, no call site in `general-simulator-gui`'s live pipeline can be routed
through `gsim_py.compile()`, and none was.

## Method

Reproducible from this repo plus a checkout of `general-simulator-gui`, and kept runnable there as
`scripts/gsim-py-parity-probe.py`:

```bash
python3 -m venv .venv && . .venv/bin/activate
maturin build --release -m crates/gsim-py/Cargo.toml --out wheels && pip install wheels/*.whl
python3 <general-simulator-gui>/scripts/gsim-py-parity-probe.py
```

Note `compile()` takes a **`Model`** JSON, not a whole `{"schema_version": 2, "model": ...}`
document — passing the document fails with a bare `missing field 'id'`, which cost time here and is
worth a clearer error.
