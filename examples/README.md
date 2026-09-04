# Examples

Real, runnable proof cases for `gsim-py`/`gsim-core`, not illustrative snippets.

## `h_bridge_3phase.py`

Proves whether `gsim-py` is mature enough to author a component-library **stamp** — a
fixed-position, pre-wired block of components meant to be copied into a parent
schematic (as opposed to a nested subsystem reference, which `gsim-core`'s document
model doesn't support yet — see `docs/decisions/0001-scope-and-naming.md`).

Builds a 3-phase (6-switch) ideal-switch bridge using the standard power-electronics
drawing convention — each leg a vertical stack (high-side over low-side, phase tap
between), three legs left-to-right, DC+/DC− rails shared across all three legs — and
exposes exactly 11 boundary attachment points (6 gate drives + 3 phase taps + DC+/DC−).
**Positions are hand-specified, not auto-arranged** — `gsim-core`'s `layered_layout` has
no concept of "this is a power-electronics leg," so a library stamp's whole job is to
carry the visually-correct fixed layout that generic autolayout can't produce.

### Run it

```bash
cd crates/gsim-py
python3 -m venv .venv && source .venv/bin/activate   # any venv works
maturin develop
cd ../..
python3 examples/h_bridge_3phase.py
```

Writes `h_bridge_3phase.gsim_sch` next to the script and prints the 11-port boundary
manifest. The script asserts the document round-trips through real schema validation
(`load_document`) and has the expected block/wire counts — it's a test, not just a demo.

## the acceptance-test script

The real acceptance test for `gsim-core`'s SPICE `.subckt` support — the **subschematic**
mechanism the `h_bridge_3phase.py` entry above named as "not buildable yet."

Reproduces the topology of an actual power-electronics circuit,
the hand-written reference netlist
(a real multi-leg power-electronics reference circuit), as a `.gsim_sch` document: `sw_leg_fixed`
defined **once** and instantiated **four** times (`X_DQ1..X_DQ4`), `bidir_sw_fixed` defined
once and instantiated twice (`X_S1`, `X_S2`), each instance with its own external connections
— crucially including a *different* PWM phase per instance, which is exactly what a
per-instance embedded copy (the `hierarchy.subsystem.*` mechanism) could not express without
four near-identical definitions.

Two conventions it demonstrates, both documented in
`crates/gsim-core/src/document/subckt.rs`:

- A def's own boundary ports are reached from inside its structured body through a boundary
  pseudo-block whose id is the def's id (`Wire`'s `Endpoint` is always `(block, port)` — there
  is no third "the enclosing def's own port" endpoint kind), keeping each def body a
  self-consistent mini-model with no dangling endpoints.
- `signal` is an allowed `SubcktPort` kind, unlike in the Python reference — the real
  `sw_leg_fixed`'s `ctrl_freq`/`ctrl_phase`/`ctrl_duty` boundary nets are signal-kind.

Positions are hand-specified in the standard leg-stack convention, matching
`h_bridge_3phase.py` — not autolayout. The script asserts its own correctness (defs, port
order, instance counts, one distinct phase source per instance, `save_document`/
`load_document` round-trip) and writes the acceptance-test document next to itself.

Not netlist-compiled or simulated — no compiler exists yet (deliberately out of Phase 1
scope). The proof is that the *document* is buildable, valid, and structurally faithful.

### Run it

```bash
cd crates/gsim-py
python3 -m venv .venv && source .venv/bin/activate
maturin develop
cd ../.. && python3 the acceptance-test script
```

## `h_bridge_3phase_driven.py`

The **flat (non-subckt) compiler path**, end to end. `h_bridge_3phase.gsim_sch` on its own is
deliberately not a runnable circuit — it is a library *stamp*, an incomplete fragment whose 11
boundary attachment points a caller is expected to wire; compiling it standalone correctly
fails with `unconnected-ref: port 'lega_high.g' must reference another block's output`.

This script completes that same stamp into a real 3-phase inverter (DC link, ground, one
phase-shift PWM modulator per leg, a gate-drive converter per switch, star-connected R-L
load), compiles it with `gsim_py.compile`, and runs the result through `general-simulator`.

Beyond the flat path it proves **per-output naming** for real: each leg's high-side switch is
gated by its modulator's *first* output (aliasing the modulator's own name) and its low-side
switch by the *second* (`MOD_a_comp`), so a wrong second-output name would cause a
shoot-through or a dead leg rather than a silent no-op.

```bash
GSIM_SIMULATOR=/path/to/general-simulator python3 examples/h_bridge_3phase_driven.py
```

Writes `h_bridge_3phase_driven.cir`. With no simulator on `PATH` (or `GSIM_SIMULATOR` unset)
it compiles and asserts, but does not run.

## the compiled netlist

The compiler's output for the acceptance-test document, checked in as the reference artifact
for the acceptance test. Structurally line-for-line equivalent to the hand-written
the hand-written reference netlist (only net *names* differ — `n_Vdc_p` where
the hand-written file says `VDCP`), and running both through `general-simulator`
(`--mode transient --tfinal 3e-4 --dt 2e-8`) agrees to ~1e-11 absolute on ~30 A currents:
floating-point noise from a different MNA node ordering, not a modelling difference.

## `buck_state_space_pi.py`

The acceptance case for the **linear-dynamics** registry families
(`control.state_space`, `control.transfer_function`), plus three already-registered
signal-flow families whose netlist spelling had never actually been *run*
(`control.saturation`, `power.pwm1`, `converter.phys2sig`).

A 24 V / 12 V / 5 Ω buck converter at 100 kHz, regulated by a **MIMO state-space PI
compensator**: rather than needing a summing block the compiler has no entry for, the
compensator takes `[ref, measurement]` as two inputs and forms the error itself —
`a=[[0]] b=[[ki,-ki]] c=[[1]] d=[[kp,-kp]]` *is* a PI controller with the integrator as
its one state. That also makes it a real test of the `inputs=` list whose length comes
from `b`'s own column count. The measurement path runs through a `kind=tf` single-pole
filter, so both linear families are in one loop.

```
general-simulator buck_state_space_pi.cir --mode transient --tfinal 2e-2 --dt 5e-8
exit 0, 400000 steps, 14 columns
V(VOUT) settled 12.1516 V against a 12 V reference; duty 0.4979 (open-loop ideal 0.5)
```

The script asserts the settled output, so a netlist that parses but does not close the
loop fails rather than passing quietly.

```bash
GSIM_SIMULATOR=/path/to/general-simulator python3 examples/buck_state_space_pi.py
```

## `pyblock_buck_control.py`

The same power stage with its controller written in **Python** — the acceptance case for
the script-hosted families (`extensibility.pyfunc`, `extensibility.pyblock`). A
`kind=pyfunc` feedforward computes `d = Vref/Vin` and a headroom output; a stateful
`kind=pyblock` slew-rate limits it into a soft start; a `control.gain` consumes the
pyfunc's **second** output by its generated name, so a wrong second-output name is a hard
`MissingSymbol` rather than a silent no-op. `inputs=REF,VINM` order is load-bearing:
reversed, the duty would be `Vin/Vref = 2`, clamped to 0.95.

Needs a simulator built with the optional Python support
(`cargo build --release -p general-simulator-cli --features python`); the plain binary
answers `PythonSupportNotCompiledIn`.

```
exit 0, 200000 steps, 15 columns
V(VOUT) 11.9820 V; SLEW (slew-limited duty) 0.5000; HEAD (FF_headroom) 12.0000 V
```

`cscript`/`octfunc`/`octblock` share this field shape and are registered, but are covered
by unit tests only — `cscript` needs a compiled `.so` and the Octave pair needs a linked
Octave, neither of which this example can produce portably.

```bash
GSIM_SIMULATOR=/path/to/general-simulator python3 examples/pyblock_buck_control.py
```

## `nested_subckt_ladder.py`

A `.subckt` that instantiates another `.subckt` — the nesting case the Phase 2 emitter
left unimplemented. A four-section RC delay line built as two levels of reuse: `SECTION`
(R + C), `STAGE` (two `SECTION` instances and **nothing else** — a body made purely of
nested `X` cards), and two `STAGE` instances at the top level. That empty-of-ordinary-
blocks body is exactly the shape the old emitter dropped silently, writing `.SUBCKT` and
`.ENDS` with nothing between them.

```
general-simulator nested_subckt_ladder.cir --mode transient --tfinal 0.2 --dt 1e-5
exit 0, 20000 steps
V(VOUT) settled 3.57143 V == 5 * 10k/14k, the exact divider through all four sections
```

An open `STAGE` would leave `V(VOUT)` at 0, so the assertion is real.

## `floating_reference_bridge.py`

`docs/specs/gsim-sch-format.md` §9's **reference-net fallback**, run for real rather than
unit-tested. This circuit declares no `routing.ground` and no `is_reference` net at all;
its return rail is the net with the most endpoints (6, against 5 / 3 / 2), so the
heuristic must pick it — and it must, because an MNA system with no reference row is
singular and cannot be solved at all.

```
compile note: "<top level>: no reference net declared ... the largest net was selected as
node 0, per docs/specs/gsim-sch-format.md §9"
general-simulator floating_reference_bridge.cir --mode transient --tfinal 1e-3 --dt 1e-6
exit 0, 1000 steps
V(TOP) = 12.000000 V, V(MID) = 6.000000 V (exactly the 100 Ω / 4x400 Ω divider)
```
