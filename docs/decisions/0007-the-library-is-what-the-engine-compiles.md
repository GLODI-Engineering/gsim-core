# 0007 — The library is exactly what this engine compiles

**Status:** accepted 2026-11-20. `GLODI-Engineering/gsim-core#59`; the taxonomy follow-up is
`#60`, the universal-component question `#61`, and the editor side `omni-schematics#114`.

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

**The library holds exactly what this engine compiles, plus the universal components that are
placed from a palette.** 76 components. Everything else is removed; git history keeps it.

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

Of the seven `structural` types — the universal components of
[0006](0006-the-components-the-editor-owns.md) — five are palette entries and two are not:

| exempt type | why it has no library entry |
| --- | --- |
| `routing.junction` | The front end splices one in when a branch is landed on an existing conductor instead of on a port. It is created by *drawing*, never picked up. |
| `spice-subckt.boundary` | Marks a `SubcktDef` body's interface. Compiler-internal; nothing places one. |

So the library's size is a consequence of the registry: every emittable type, plus the five
universals a user can place. `the_embedded_library_parses` asserts exactly that sum rather than
a magic number, so adding a component with its `TypeSpec` moves the count with it, while a
truncated read or a one-sided addition fails.

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
satisfied the second guard the entire time. Two of the 76 "compilable" components were not
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
