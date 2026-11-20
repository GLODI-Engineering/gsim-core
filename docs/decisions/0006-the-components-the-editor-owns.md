# 0006 — The components the editor owns, and why every target must support them

**Status:** accepted 2026-11-20. `GLODI-Engineering/gsim-core#54`; the editor side is
`omni-schematics#112`.

## Context

A resistor belongs to a device library. It models a thing, a target decides how to spell it, and
an engine that cannot model one may legitimately refuse it — `CompileError::UnknownType` is the
right answer for a document a target genuinely cannot compile.

A Goto is not like that. It is not a part anyone could buy. It is a way of saying *this signal
continues over there*, and it means exactly the same thing on every sheet, whatever engine the
sheet is bound for. Four more are in the same class:

| Type | What it is |
| --- | --- |
| `routing.goto`, `routing.from` | §10's implicit connection: a tagged broadcast and its receiver |
| `routing.ground` | Only *names* the reference net |
| `routing.junction` | A wire tap the front end splices in when a branch lands on a conductor rather than a port |
| `sinks.scope`, `signal-flow.sink.scope` | A display concern; the trace already carries every signal |
| `spice-subckt.boundary` | The pseudo-block a `SubcktDef` body's ports hang off |

These are the schematic's **own vocabulary**, not any library's device set. The editor owns them,
and that ownership has a consequence this crate is responsible for.

## Decision

**A universal component is one the editor owns, and every target supports them all.** Not by
each target handling them, but by no target ever being handed one.

The pipeline resolves each into something target-independent before
`NetlistTarget::emit` is called:

- `compile::routing::resolve_scope` turns a matched Goto/From pair into the redirect table on
  `Scope::from_source`, so a `field=` reference landing on a `From` is rewritten to the real
  driver behind its Goto. Called from `compile::build_scope`, once per scope, for every target.
- The net walk merges the conductors meeting at a junction's `node` port, so the tap has no
  separate identity by the time anything emits.
- A ground's ports become `nets::REFERENCE_NET`.

Then `Scope::emittable()` leaves them out of what a target iterates. `spec::is_universal` is the
predicate, `spec::universal_types()` the list — derived from `TypeSpec::structural` in the
registry rather than written beside it, so marking one new spec `structural` is the only edit
needed to add a universal component.

An **unknown** type is deliberately *not* filtered: it is not universal, so it still reaches the
target and still raises `CompileError::UnknownType`. A type this crate has never heard of must
fail loudly, which is the one thing worse than quietly emitting a Goto.

## Why not leave it to each target

That is what this crate did until #54, and it worked — for as long as there was one target.
`general_simulator.rs` checked `spec.structural` in two places, the emission path and the
node-collection path. A second target would have had to remember in its own two places, and
nothing would have told its author: a `routing.goto`'s spec declares no device letter and no
`kind=` name, so emitting one writes a malformed card rather than raising an error. The failure
would have surfaced as a netlist the engine rejects for reasons naming neither the Goto nor the
target.

The rule also cannot live in the target because it is not the target's rule. A target gets to
decide how it spells a resistor and whether it supports one. It does not get a say in whether a
sheet may contain a Goto.

## How the guarantee is tested, and one thing that did not work

`tests/universal_components.rs` compiles a sheet carrying every placeable universal component —
a matched Goto/From pair with a real driver, a ground, a junction, a scope — plus a DC source
across a resistor so the netlist is not trivially empty.

**Asserting against `general-simulator`'s output does not test anything.** That emitter is
tolerant: a `routing.ground` has no device letter and no `kind=` name, so handing it one produces
no card rather than a bad one. Verified by reverting the filter and watching the output
assertions stay green.

So the test defines its own `NaiveTarget`, which emits one line per block it is handed and
filters nothing. It is the stand-in for the second target this crate does not have yet, and it
is how the guarantee gets checked by code that does not already know the rule. `compile_with`
exists so that a target can be written and tested outside `target_for`'s match. Reverting
`emittable()` to leak everything fails that test by name.

`compile::TARGETS` is one list of every target, so the sweep picks up a new one without its
author being told to add it.

## Consequence for the editor

`capabilities()` carries `universal_types`, separately from `types`. The question a front end
needs answered about these is not "how does this engine spell it" — no engine spells them — but
"may I always offer this component?". The answer is yes, whatever engine a sheet is bound for.
**A palette that filters on engine support must not filter these out.** The manifest's
`schema_version` goes to 3 for the new field.

## What this does not settle

`gsim_sch_flatten` — flattening a subsystem body — is the one operation `omni-schematics` still
does for itself that may belong here. `instantiate_subckt` is not obviously the same thing, and
`compile/routing.rs`'s own doc records the related limitation: this crate's document model has no
subsystem hierarchy, so §10.1's `local`/`scoped`/`global` Goto visibilities all resolve
identically against one flat tag namespace per scope. Both wait on `Block` gaining a parent.
