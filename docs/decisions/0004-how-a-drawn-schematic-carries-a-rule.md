# 0004 — How a drawn schematic carries a rule or a measurement

**Status:** accepted 2026-11-19. Decided by the owner from the three shapes below;
`GLODI-Engineering/gsim-core#37` carries the original framing and the measurements.

## Context

`general-simulator` 0.6.0 is built around two netlist statement kinds that are not components:

```text
R_ileg  kind=rule    type=above  target=I(L1) max=60 unit=A severity=error
M_rms   kind=measure type=rms    target=I(L1) from=1m to=2m
```

Nine rule types and sixteen measure types, the event stream, `--fail-on`, the ZVS check, "a
deck is its own regression test" — all of it rests on those two cards. This compiler emitted
**neither**:

```console
$ grep -c "kind=rule\|kind=measure" crates/gsim-core/src/compile/spec.rs
0
$ grep -c "kind=rule\|kind=measure" crates/gsim-core/data/component-library.json
0
```

Nor could a limit reach the netlist by the shorter route the engine itself prefers — a rating
written on the device line. The emitter writes only what a `TypeSpec` declares, and `i_max`,
`i_sat`, `v_max` and `on_violation` appeared nowhere in this crate: a rating typed onto a drawn
component was dropped before the netlist existed.

So every rule and every measurement was reachable **only by hand-writing a netlist**. Someone
who draws a converter in the editor could not ask *"tell me if this switch exceeds its
rating"*, which is the feature the engine release is named for. Nothing in either repository
documented that limit, so the gap read as an oversight rather than a decision.

## The shapes considered

A rule is not a circuit element, so where it lives in a drawn document is a real choice and the
three candidates are not equally good.

1. **A placeable block** — `rule.above`, `rule.switch_timing`, one registry type per rule kind,
   dragged from the palette like a component. Fits the existing machinery exactly and needs no
   new editor UI at all. Costs roughly twenty-five types, each with its own parameter set, each
   a card to design and a drift guard to keep. And the thing placed has no position in the
   circuit: no pins, no wires, a box floating on the drawing beside the components it watches.
2. **A property on the component that owns the quantity** — `i_max=60` written on the inductor
   that was drawn. Four limit fields and three modifiers, no new types, no new UI beyond an
   inspector section, and it is the form the engine's own user guide recommends first
   ("Ratings on the device line"). Cannot express anything spanning two devices: no dead-time
   rule, no commutation order, no per-period measurement.
3. **A document-level list**, edited in a panel beside the schematic rather than drawn. Matches
   what a rule actually is, and scales to all twenty-five kinds with no palette growth. Needs
   editor UI that does not exist — the only one of the three that does.

## Decision

**(2) and (3), in that order. Not (1).**

(2) and (3) are not exclusive and together they cover the whole feature set: a datasheet limit
belongs on the part it describes, and everything that spans two devices belongs in a list that
is not pinned to either of them. (1) is declined — not because a palette block cannot work, but
because it would pay twenty-five types for the same expressive power (3) buys with one panel,
and because a rule block has nothing to connect, so the one advantage of being on the canvas is
unavailable to it.

This record covers **(2), which is implemented here**. (3) is an editor change, tracked
separately in `omni-schematics`, and this crate's part of it will be the `kind=rule` and
`kind=measure` card emitters — which is where the twenty-five kinds finally arrive, as data in
a panel rather than types in a palette.

## What (2) means in this crate

- **`Block::ratings`, a sibling of `params`, not an entry in it.** A rating is not a model
  parameter: it is what the part is rated for, and it stays true when the model under it
  changes. Switching a component's fidelity tier *removes* the parameters the target tier does
  not declare, so a rating stored in `params` would silently vanish the first time anyone moved
  an inductor from `ideal` to `spice` — losing a limit without saying so, which is worse than
  never having offered one. A test asserts the round trip.
- **The vocabulary is the engine's, held once.** `compile::spec::RATING_FIELDS` is the engine's
  `RATING_FIELDS`, and a CI job diffs the two lists against the engine's default branch. Two
  copies are unavoidable — neither repository depends on the other, and neither should — so the
  agreement is checked rather than assumed.
- **Which ratings a type can carry is derived, never stored.** A current rating needs a branch
  current the solver actually solves for (`V`, `L`, `E`, `H` — a resistor's current is computed
  *from* the solution, not solved for), and a voltage rating needs exactly two terminals to be
  across. Both facts are already in the `TypeSpec`, so `TypeSpec::ratings()` reads them off it
  and there is no second list to drift.
- **The checks are duplicated from the engine on purpose, and may never be laxer than it.** A
  rating is typed into an inspector, and the moment to say *"a resistor has no branch current
  to limit"* is while that inspector is open — not after a run the user started and waited for.
  A document this crate accepts and the engine then rejects is drift the user pays for, so the
  reasons are the engine's own words, down to what to do instead.
- **One thing deliberately not duplicated: the number grammar.** A rating accepts the netlist's
  own spellings, so `60m` is valid and arrives as a string. `general-mna` owns that grammar and
  the editor already mirrors it under a parity test; a third implementation here would be a
  third thing to keep agreeing about `M` meaning milli. This crate checks only the shape no
  number can have — it must start like one, and a magnitude limit is never signed — and passes
  everything else through untouched.
- **The manifest carries the vocabulary.** `compile::capabilities` exports the fields, which of
  them are limits, and the closed sets `on_violation` and `severity` may take, plus the
  per-type subset. The front end renders a *Ratings* section from the manifest and holds no
  list of its own, for the same reason the rest of that module exists.

## What a drawn schematic can and cannot carry, as of this record

| | from a drawing | by hand-writing a netlist |
|---|---|---|
| `i_max`, `i_sat` on `V`, `L`, `E`, `H` | **yes** | yes |
| `v_max` on any two-terminal device | **yes** | yes |
| `hold`, `on_violation`, `severity` beside a limit | **yes** | yes |
| `above`, `outside` on an arbitrary expression | no | yes |
| `switch_timing`, `forbidden_state`, `required_path` | no | yes |
| `body_diode`, `commutation_order`, `rate_of_change` | no | yes |
| `per_period` (`rms`, `integral`, …) | no | yes |
| every `kind=measure` type | no | yes |

The "no" column is (3)'s scope, and both repositories' user-facing documentation states this
table rather than leaving a reader to discover it.
