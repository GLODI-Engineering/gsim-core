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

`gsim-core`'s SPICE `.subckt` support (define a subcircuit once, instantiate it many times
with distinct external connections each time — the **subschematic** mechanism the
`h_bridge_3phase.py` entry above named as "not buildable yet") has its own real acceptance
test, exercised against a real multi-instance circuit, but that reproduction lives in the
private project it was drawn from rather than in this repo — see
`docs/decisions/0002-no-private-content.md`. Two conventions it proved out are documented in
`crates/gsim-core/src/document/subckt.rs`: a def's own boundary ports are reached from inside
its structured body through a boundary pseudo-block whose id is the def's id (`Wire`'s
`Endpoint` is always `(block, port)` — there is no third "the enclosing def's own port"
endpoint kind), and `signal` is an allowed `SubcktPort` kind (unlike in the Python reference
this was ported from), since a real subckt's control inputs are often signal-kind, not just
the four acausal physical-network kinds.

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

The compiler's output for the `.subckt`-based acceptance circuit mentioned above agreed with
the hand-written reference netlist it was compiled from to ~1e-11 absolute on ~30 A currents
when both were run through `general-simulator` — floating-point noise from a different MNA
node ordering, not a modelling difference. See `docs/journal/2026-09.md` for the fuller
account (kept generic there too, per `docs/decisions/0002-no-private-content.md`).

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

## `annotated_rc_comments.py`

**Per-block authoring comments**, from `gsim_py` through to the emitted `.cir` text. A block
carries two independent optional slots, `comment_before` and `comment_after` (a list of lines,
or a single string that may contain newlines), and the compiler renders each as full-line `*`
SPICE comments immediately above/below that block's own card(s). The script asserts placement
*positionally* — the line directly above and below the card — across both emitter sub-grammars,
since they are separate code paths: `R1`/`C1` are element cards, `K1` is a `kind=` block card.

```
* series element of the RC low-pass
* 1 kohm with C1 -> f_c = 159 Hz
R1 n_R1_p1 n_C1_p1 1000
* (tolerance is not modeled here)
C1 n_C1_p1 0 1e-6
* storage element -- ic defaults to 0 V
* loop gain
* re-tuned after the step test
K1 kind=gain k=2.5 in=U1
```

`K1`'s two comment lines were authored as one Python string containing `\n`: an embedded
newline must become a second `*` line, never raw text on a line the lexer would read as a card.
The script also checks the exact total `*` line count, so a comment can neither leak onto an
uncommented block nor be silently dropped. (The circuit is a trivial RC low-pass — the comments
are the subject, so `general-simulator` is only run, if present, to confirm the extra lines are
inert to the parser.)

## `tagged_gate_goto_from.py`

`docs/specs/gsim-sch-format.md` §10's **Goto/From implicit connections**, lowered away
completely by the compiler. A `routing.goto` broadcasts whatever drives its `in` port under a
string tag; a `routing.from` has no wire input at all and resolves to the Goto sharing that
tag, behaving exactly as if wired straight to that Goto's own driver. Neither is a device, so
neither may reach the netlist.

A single-switch 24 V chopper whose gate command is broadcast under the tag `GATE_CMD` and
picked up by **two** Froms — one feeding the `converter.sig2phys` gate driver, one feeding a
monitor gain — with no wire anywhere between `KGATE` and either consumer:

```
DSW kind=ideal_switch r_on=0.01 ... gate=block ctrl=GATE
CMD kind=const value=1
KGATE kind=gain k=1 in=CMD
GATE kind=sig2phys domain=voltage in=KGATE     <- reached only through the tag
MON kind=gain k=2 in=KGATE                     <- same tag, second consumer
```

The strongest assertion is the **twin**: the same circuit rebuilt with real wires and no
routing blocks compiles to a *byte-identical* netlist, so a Goto/From pair is provably nothing
but a drawing convenience. §10.3's two hard errors are asserted too — a From matching no Goto
(`unresolved-from-tag`) and a tag broadcast by two Gotos (`ambiguous-from-tag`).

```
general-simulator tagged_gate_goto_from.cir --mode transient --tfinal 1e-3 --dt 1e-6
exit 0, 1000 steps
V(SW) = 23.976024 V (the closed-switch 0.01 Ω / 10 Ω divider, exactly)
```

A gate reference that failed to reach the real driver would leave the switch open and V(SW)
pinned at 0, so the run is a real end-to-end proof that the tag carried a live signal.

## `port_kind_rejection.py`

Proves the **per-port** compatibility rules (`docs/specs/gsim-sch-format.md` §8.1/§8.2) that
`crates/gsim-core/src/document/port_compat.rs` adds below `connect()`'s older block-level
cross-paradigm check (§8.3), ported from `general-simulator-gui`'s
`apps/frontend/src/canvas/port-compat.ts`.

A textbook-generic RC low-pass with an independent thermal branch (a thermal resistance to a
thermal mass) in the same document, plus two signal sources and two gains. Three rejections
are raised for real through `gsim_py.connect`, each with its own stable keyword prefix:

```
port-kind-mismatch: cannot connect 'R1.p2' to 'Rth.a' as a 'physical-electrical' wire:
    kind-mismatch -- port 'Rth.a' is declared 'thermal'
port-fan-in-on-signal: cannot connect 'U2.out' to 'K1.in' as a 'signal' wire:
    fan-in-on-signal -- causal port 'K1.in' is already driven by wire 'wire1' ...
port-reserved-kind: cannot connect 'R1.p2' to 'C1.p1' as a 'reserved.physical-magnetic'
    wire: reserved-kind -- port kind 'reserved.physical-magnetic' is reserved ...
```

The electrical/thermal case is the one that matters most: **both blocks are
`physical-network`**, so the block-level paradigm check has nothing to say about it — that is
exactly the gap the per-port rule closes.

The second half is the other side of the proof, and is why a checker that simply rejected
everything would not pass this script: the *correct* wiring of those same blocks is accepted
(electrical loop, thermal branch, one driver per signal input — fan-**out** and multi-wire
acausal nodes both still legal), round-trips through `load_document`, and compiles to a real
netlist. Only the thermal blocks are dropped before compiling, because `compile::spec::REGISTRY`
has no `thermal.*` emitter yet — a netlist-target gap, not a connectivity one.
