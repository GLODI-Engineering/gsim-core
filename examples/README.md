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
