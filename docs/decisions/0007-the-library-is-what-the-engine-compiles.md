# 0007 — The library is exactly what this engine compiles

**Status:** accepted 2026-11-20, **amended the same day** (see
[Amendment](#amendment-the-line-is-what-a-front-end-has-to-draw-63)).
`GLODI-Engineering/gsim-core#59`; the taxonomy follow-up is `#60`, the universal-component
question `#61`, and the editor side `omni-schematics#114`.

## Context

`data/component-library.json` held **240 components**. **164 of them (68%) could not be compiled
by anything**: no `TypeSpec` backed them, so placing one produced `CompileError::UnknownType`.
The palette offered them, the inspector described their parameters, and the fidelity dropdown
offered them tiers.

They were not one kind of thing:

| what it was | count |
| --- | --- |
| legacy duplicate spellings — the `physical-electrical.*` and `signal-flow.*` groups, superseded by the short names | 69 |
| whole unimplemented domains — `physical-thermal` 13, `physical-rotational` 11, `physical-translational` 11, `tline` 5 | 40 |
| Simulink's Signal Routing and Ports & Subsystems palettes, transcribed name for name | ~25 |
| the rest — `semi.*`, the domain `converter.*` pairs, `passive.memristor`, three more switches, `source.behavioral` | ~30 |

The flag that described all 164 was the absence of `compiles_to`, which conflated three
different answers: *the editor owns this and no engine should emit it*, *this is declared future
scope*, and *this was transcribed from another tool and nobody ever implemented it*.

## Decision

**The library holds exactly what this engine compiles, plus the universal components a front
end has to be told how to draw.** 78 components. Everything else is removed; git history keeps
it.

> The second clause originally read *"plus the universal components that are placed from a
> palette"*, and the count was 76. The amendment below says what moved it.

`gsim-core` compiles for one target, `general-simulator`, and
[#36](https://github.com/GLODI-Engineering/gsim-core/issues/36) already settled that this is a
decision rather than a gap: `AGENTS.md` invariant 2 states it is the one `NetlistTarget` and the
only one planned, and a second backend is a new decision with its own record. So "what components
exist" and "what can this engine emit" are the same question, and the library answers it by its
contents rather than by a field. Being in the library *is* the claim that the component compiles, and `REGISTRY`
is the proof.

### Why not a `support` or `planned` field

A field was the obvious alternative: mark each of the 164 `universal`, `planned` or `junk` and
keep them. It was rejected because

- a palette full of components that cannot run is the problem, not the record of it. Greying out
  40 physical-domain blocks still shows a user 40 things that will not work;
- the question "is this roadmap or junk?" would have to be re-answered per component, forever,
  and nothing would stop the next transcription being added under `planned`;
- a stored flag can only drift from the registry. The 164 were *already* marked, by the absence
  of `compiles_to`, and that mark had silently come to mean three different things.

Roadmap belongs in an issue, where it can carry a reason and be closed. It does not belong in the
vocabulary a user draws from.

### The two exemptions

Of the nine `structural` types — the universal components of
[0006](0006-the-components-the-editor-owns.md) — seven have a library entry and two do not:

| exempt type | why it has no library entry |
| --- | --- |
| `routing.junction` | The front end splices one in when a branch is landed on an existing conductor instead of on a port. It is created by *drawing*, never picked up. |
| `spice-subckt.boundary` | Marks a `SubcktDef` body's interface. Compiler-internal; nothing places one. |

So the library's size is a consequence of the registry: every emittable type, plus the seven
universals a catalogue has to describe. `the_embedded_library_parses` asserts exactly that sum
rather than a magic number, so adding a component with its `TypeSpec` moves the count with it,
while a truncated read or a one-sided addition fails.

## How the contract is enforced

Three guards, in `compile::library`:

| guard | direction |
| --- | --- |
| `every_compiles_to_names_a_real_registry_type` | a component's declared target exists |
| `the_registry_is_not_drifting_further_from_the_library` | every emittable `TypeSpec` has a component to place it |
| `every_component_resolves_under_its_own_id` | **new** — every component resolves through `spec::lookup`, the resolver a document actually goes through |

The third is not redundant, and the bug it caught on the day it was written says why.
`compiles_to` is a *claim in the data file*; `spec::lookup` resolves a name by finding the
component and then matching `component.id` against the registry. The transformer and the mutual
inductor were spelled with a **hyphen** in the library and an **underscore** in the registry:

```text
lookup=false  find_component.id=Some("physical-electrical.passive.transformer")
lookup=true   find_component.id=None   <- physical_electrical.passive.transformer
```

Each was therefore split into two unreachable halves — the drawable name failed `UnknownType`,
the compilable name had no component to place it — while both carried `compiles_to` and
satisfied the second guard the entire time. Two of the then 76 "compilable" components were not
compilable. They are now `passive.transformer` and `passive.mutual_inductor`, one spelling each,
with the old spellings kept as aliases.

Both new guards were verified to fail before being trusted: reinstating the hyphen spelling, and
re-adding `semi.mosfet.n`, each produce a failure naming the offender.

## Consequences

- **A palette entry is a promise.** Anything a user can place, this engine can compile. There is
  no "shown but unsupported" state left to explain, and `capabilities`' `compilable` flag is now
  true for every component — derived from `spec::lookup`, never stored.
- **Two deleted names no longer answer.** `semi.diode` and `passive.switch.voltage` survived as
  *aliases* of `passive.diode.ideal` and `passive.switch.ideal`. A deleted component that still
  answers to its name is not deleted, and resolving a real SPICE diode or a voltage-controlled
  switch to general-simulator's piecewise-linear ideal device is the model conflation
  [0001](0001-scope-and-naming.md) exists to forbid. Both aliases are gone; the names are now
  honestly unknown.
- **`CompileError` needs no "known but not compilable" variant.** With one target, in-library
  means compilable and out-of-library means `UnknownType`. The variant becomes real at the
  *second* target, when the library becomes the union of what the targets emit and a component
  can be known here and unemittable there. It is not real today.
- **The editor's catalogue must be regenerated behind this.** `omni-schematics` generates
  `engine-catalog.json` from this repository's master and gates on it, so its
  `engine-catalogue-drift` job fails until it does — `omni-schematics#114`.
- **Six removals are not settled.** `hierarchy.subsystem.virtual`, `hierarchy.inport`/`outport`
  and the three `routing.bus_*` components are candidates for the universal set rather than
  junk; `#61` decides what each would resolve into. They are removed here because a library
  entry alone would not have made them work — a universal component is a `structural` `TypeSpec`
  plus a resolution step in the pipeline, which is a different kind of entry.

## Amendment — the line is what a front end has to draw (#63)

Accepted 2026-11-20, hours after the decision above, because running `omni-schematics`' e2e
suite against the pruned library found a shipped regression this record's own reasoning had
invited.

### What broke

`omni-schematics`' `canvas/group-blocks.ts` synthesizes one boundary port per wire that crosses
a selection when a user groups blocks into a subsystem, and it builds each one from catalogue
metadata:

```ts
const inportMeta = catalog[INPORT_TYPE];     // "hierarchy.inport"
const outportMeta = catalog[OUTPORT_TYPE];   // "hierarchy.outport"
if (crossingWires.length > 0 && (!inportMeta || !outportMeta)) return null;
```

Both entries had just been removed, so **Ctrl+G silently did nothing whenever a wire crossed the
selection** — the ordinary case. Grouping a selection with no crossing wire still worked, which
is why only one of the two subsystem specs failed and why 1407 unit tests said nothing.

### Why the original line was wrong

The decision above drew the line at *who performs the gesture*: five universals "a user places"
get a library entry, two that the software creates do not. That is the wrong question, and the
boundary ports are the counter-example that shows it. Nobody places one — grouping synthesizes
them, exactly as drawing splices in a `routing.junction` — and yet the editor cannot create one
without reading its ports, its symbol and its `id`/`kind` parameters out of the catalogue. By
the old rule they were correctly excluded, and the feature broke.

The line that holds is **what a front end has to be told how to draw**:

| exempt type | why a catalogue entry would have nothing to say |
| --- | --- |
| `routing.junction` | Drawn as a dot by the editor's own wire renderer. It has no symbol, no parameters and no ports a palette or inspector would show. |
| `spice-subckt.boundary` | A pseudo-block this crate invents for a `SubcktDef` body. No front end ever receives one. |

Both exemptions survive the restatement unchanged — which is the point. The two types that
genuinely need no entry need none for a reason about the *data*, not about the user, and that
reason is checkable: ask whether a front end would read anything out of the entry.

### What this does not fix

The pair now compiles, by emitting nothing, like every other `structural` type — and that is
only exercised at the top level of a sheet. A *grouped* document puts them inside a
`hierarchy.subsystem.virtual`, which this engine has no `TypeSpec` for and cannot compile at
all. So Ctrl+G stops being a silent no-op and starts producing a document that fails honestly
with `UnknownType` on the subsystem block. The rest is `#61`.

The last consequence below therefore stands for four of its six names, not six: the three
`routing.bus_*` components and `hierarchy.subsystem.virtual` still need a resolution step in the
pipeline before a library entry would mean anything. The claim that *"a library entry alone would
not have made them work"* was simply false for the two boundary ports, where a library entry plus
a `structural` `TypeSpec` is the whole fix.

### Cost of the original decision, recorded honestly

Three consequences of the prune were not priced when it was taken, and the e2e suite found all
three:

1. This regression.
2. Two toolbar buttons in `omni-schematics` that had inserted a BJT and a MOSFET became dead
   buttons — found by rendering the palette and looking at it, not by a test.
3. **`port_sigs` fell from 55 components to 4.** That field drives the editor's signal-dimension
   overlay and only ever existed on signal-domain components; of the 51 entries that went, 42
   belonged to genuinely removed components, but **9 are aliases of components that survived**
   (`signal-flow.logic.and` → `logic.gate`, `signal-flow.discrete.delay` → `control.delay`, and
   seven more). So the 69 "legacy duplicate spellings" the table above counts as superseded were
   not purely redundant: some carried data their canonical entry lacks. 45 of the 49 surviving
   signal-domain components have no `port_sigs` at all, and the editor has been coasting on a
   builtin table keyed by the legacy names. Tracked separately.
