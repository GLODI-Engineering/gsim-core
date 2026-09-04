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
