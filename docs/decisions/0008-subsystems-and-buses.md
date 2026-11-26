# 0008 — Subsystems and buses: what each resolves into, and what preserving without interpreting does and does not settle

**Status:** accepted 2026-11-26. `GLODI-Engineering/gsim-core#61`, raised by #59. Builds on
[0006](0006-the-components-the-editor-owns.md) (what a universal component is) and
[0007](0007-the-library-is-what-the-engine-compiles.md) (and its #63 amendment, which already
settled two of the six candidates).

## Context

[#59](https://github.com/GLODI-Engineering/gsim-core/issues/59) deleted 164 components. Six of
them were neither junk nor engine features, but candidates for the **universal** set 0006
established — the schematic's own vocabulary, resolved by the pipeline before any target sees
one:

| candidate | why it might be universal |
| --- | --- |
| `hierarchy.subsystem.virtual` | pure drawing hierarchy; flattened, it disappears |
| `hierarchy.inport` / `hierarchy.outport` | the subsystem boundary's interface |
| `routing.bus_creator` / `bus_selector` / `bus_assignment` | bundling signals onto one wire is a *wire* concept |

0006 fixed the test each has to pass: a universal component must be **resolvable into something
target-independent**, and the question is therefore *what does it become?* A Goto became a
redirect table, a ground became `nets::REFERENCE_NET`, a junction became merged conductors.

Two things have happened since #61 was filed, and together they narrow the question rather than
answer it.

**#78/#79 made the document carry what this crate does not model.** Every struct of the document
tree gained a flattened catch-all, so a block's `subsystem` — an entire nested model — a
`.SUBCKT`'s SPICE `body`, `model.ports` and `parent_param_aliases` now survive a round trip
verbatim, as opaque values. `omni-schematics#200` was closed on that basis. The crate therefore
**preserves** subsystems without **interpreting** any of them.

**`omni-schematics` deleted its Python packages** in its 0.9.0 release, and `omni-schematics#112`
— "a second `.gsim_sch` implementation lives in the view repo, in Python" — was closed
`not_planned`. `gsim_sch_flatten.py`, the one flattening pass that existed anywhere, went with
them.

So #61's question becomes a sharper one: **is "preserve, do not interpret" a settled division of
labour, or a position nobody is holding the other end of?**

## Decision

It is both, and which one it is differs per candidate. The division is permanent where something
else does the interpreting or where no interpretation is owed; it is not a resting place where
the result is that nobody interprets.

### 1. `hierarchy.inport` / `hierarchy.outport` — universal, and already are

Settled, by work merged before this record. 0007's #63 amendment put both back in the library,
each with a `structural` `TypeSpec`; `spec::universal_types()` lists them, the capability
manifest reports them, and `group-blocks.ts` builds a boundary port from the catalogue entry —
which is why Ctrl+G was a silent no-op without them.

They resolve into nothing, which is the point: a `structural` type emits no card, and a boundary
port's *name* is what an `X` card substitutes into positionally
(`nets::ScopeOptions::boundary_block`). #61's first bullet is closed by code, not by this
record.

### 2. `hierarchy.subsystem.virtual` — universal, and the flattening pass belongs here

A virtual subsystem means "these blocks are drawn together". It carries no execution semantics,
it means the same thing on every sheet for every engine, and it resolves into exactly one thing:
its contents, inlined, with its inport/outport wires spliced through. That is Goto's shape, and
it passes 0006's test.

**Preserving it is not a division of labour, because there is no longer another side.** Until
0.9.0 the editor flattened in Python; that code is deleted and `omni-schematics#112` is closed
`not_planned`. Today nothing flattens a subsystem anywhere, so a grouped sheet reaches
`compile()` with a `hierarchy.subsystem.virtual` block this registry has no `TypeSpec` for and
fails with `UnknownType` — which 0007's amendment recorded as the honest outcome of fixing
Ctrl+G, and which was honest precisely because the resolution was expected to land somewhere.
It has not.

The argument for putting it here rather than back in a front end is 0006's own, one level up.
0006 says no *target* should have to handle a universal component, because the one that forgets
produces a malformed card and nothing tells its author. The same holds for front ends: a second
editor, or `gsim-tools`, or a script driving `gsim_py`, would each have to reimplement
flattening, and the one that forgets produces a document that fails at Run naming a type the
user never typed. A pass every consumer needs and no consumer can skip belongs in the one
implementation.

What this costs, stated plainly so it is not discovered later: the pass needs this crate to
**read** a block's `subsystem` body, which #79 deliberately left opaque. Typing that field is
the first step, and it is a different undertaking from round-tripping it. `compile/routing.rs`
records the related limitation — this crate's document model has no hierarchy, so §10.1's
`local`/`scoped`/`global` Goto visibilities all resolve identically against one flat tag
namespace per scope — and flattening is what makes those three distinguishable. Both wait on the
same change.

**`hierarchy.subsystem.atomic` / `.enabled` / `.triggered` are not universal**, and #61's own
reasoning for that is ratified here rather than revisited: they carry execution semantics —
scheduled as a unit, conditional execution — which are simulator features. An engine that does
not implement them must be allowed to refuse them, and `UnknownType` is the right answer for a
document a target genuinely cannot compile (0006).

### 3. `routing.bus_*` — neither universal nor an engine feature, and deliberately undecided

A bus resolves into N separate signals, which sounds like the junction's shape and is not. The
resolution needs a **bus type**: what is in the bundle, in what order, whether a bundle may
contain a bundle, and what a partially-assigned bus means. Goto needed no new type because "the
signal continues over there" adds nothing to the format; a bus does.

Nor can the engine take them instead. `signal-bus` is in the port-kind vocabulary and in
`routing::ROUTABLE_KINDS`, but no `TypeSpec` emits one and `general-simulator`'s netlist grammar
has no bundle — a `kind=` field's value is one signal reference. So a bus is a *format* feature
with no implementation on either side.

Declaring them universal now would promise a resolution nobody has designed, which is the exact
thing #59's prune was taken to stop: the library holding components that might one day be
drawable. They stay out of the library until the type exists, and because they are out of the
library the editor cannot place one — so there is no half-working path for a user to find. The
design is worth its own issue and its own record; it is not blocked on anything here.

### 4. What "preserve, do not interpret" settles permanently

For three of the four things #78/#79 rescued, the division is final and needs no further work:

- **A `.SUBCKT`'s `body`.** This crate has no SPICE text reader and wants none
  (`document/mod.rs`'s scope note). A netlist-only def is text destined for a solver that
  already parses SPICE; interpreting it here would be a second parser to keep in step.
- **`parent_param_aliases`** and **`model.ports`.** Document-level naming, resolved by whatever
  authored them.
- **Any unrecognised key.** #79's deliberate bias: for a format whose writer may be newer than
  the engine reading it, preserving the unknown beats deleting a user's work.

Round-tripping is a separate obligation from understanding, and #79 said so. This record adds
the converse: round-tripping is not a *substitute* for understanding where something downstream
needs the understanding and no longer has it.

## Why not flatten in the editor after all

It is where the code used to live, it is where the canvas already walks a hierarchy for
navigation and Ctrl+G, and it would need no change to the document model here.

Rejected because of what the editor would be flattening *for*. Navigation and grouping are view
concerns and stay there; flattening exists only to produce something a compiler can read, and
the compiler is here. An editor that flattens is an editor that has to agree with this crate
about net naming inside a body, about which port ids the splice keeps (AGENTS.md invariant 3:
stable, index-based, never reassigned), and about Goto visibility across a boundary — three
agreements maintained by hand, which is the arrangement 0007 was written to end for the
component catalogue and 0006 for the universal set.

## Consequences

- **#61's first bullet is already satisfied**; this record says by what, so the issue closes
  against a decision rather than against a shrug.
- **The virtual subsystem needs its own issue**, and has one: `#88` — type `Block::subsystem`,
  add the flattening pass, give the type a `structural` `TypeSpec` and a library entry, and make
  the three Goto visibilities mean three different things. That is a milestone, not a checkbox.
- **Until it lands, Ctrl+G still produces a document that fails at Run.** It fails honestly,
  which is better than the silent no-op it replaced, but the editor can say so at group time
  instead of at Run — `capabilities`' `universal_types` does not list
  `hierarchy.subsystem.virtual`, so the editor already has the fact it would need.
- **Buses need a design record before a library entry**, filed as `#89` with the four questions
  the bus type has to answer. `omni-schematics`' `BusInspector` and its bus artwork are drawings
  of a format feature with no compile path; nothing here promises one, and no catalogue entry
  invites a user to try.
- **No code changes with this record.** It is the answer #61 asked for, and the work it implies
  is filed separately rather than smuggled in beside it.
