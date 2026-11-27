# 0010 — Who owns the catalogue, and what a composite component is

**Status:** accepted 2026-11-26, **amended 2026-11-27** (see
[Amendment](#amendment--a-parameterised-subcircuit-is-bound-by-the-solver-not-substituted-here-2026-11-27)).
Refines [0007](0007-the-library-is-what-the-engine-compiles.md)
(the library is exactly what this engine compiles) and
[0006](0006-the-components-the-editor-owns.md) (the components the editor owns). Extends
[0008](0008-subsystems-and-buses.md) (subsystems and buses) with what a composite component is.
Corrects the framing recorded in `GLODI-Engineering/omni-schematics#253` and in this
repository's own documentation.

## Context

The documentation has said, in several places, that **the simulator decides which components
exist and a front end obeys**. The owner raised that this is wrong, and not as a matter of
wording:

> *"The front end does not see the simulator directly. It's the compiler it sees. So I think it
> is the compiler who has the responsibility to present the list of components needed and their
> specification to the front end. And it's the front end's responsibility to display them. So
> the front end will have a catalogue of things it can display and the properties, because
> ultimately it's the front end who shows and gets the value."*

He is describing a direction of declaration and an owner of display, and the current code has
neither clearly.

### What the code actually does today

One file, `crates/gsim-core/data/component-library.json`, holds 78 components. It is generated
into the editor as `apps/frontend/src/library/engine-catalog.json`, where every entry cites its
origin — `component-library.json#control.counter` — so the editor is a pure consumer with no
catalogue of its own.

Each entry declares eighteen fields, and they fall cleanly into two groups:

| What a front end draws and collects | What this engine compiles |
| --- | --- |
| `label`, `symbol`, `category` | `compiles_to`, `ports`, `port_sigs` |
| `description`, `tags`, `main_param` | `params`, `paradigm`, `fidelities` |
| | `behavior`, `domain` |

A `symbol` is a drawing. A `category` is how a palette groups a list. A `description` is help
text. **This engine has no opinion worth having about any of them**, and it carries them because
something had to.

### 0007 already found this seam and widened a definition to cover it

0007 decided that "the library holds exactly what this engine compiles" — and was **amended the
same day** to "…plus the universal components a front end has to be told how to draw", moving
the count from 76 to 78. The two components added are ones this engine never emits: 0006 resolves
them away before any target is called.

So a decision whose one-line summary is *the library is what the engine compiles* contains two
components that the engine does not compile, because a front end has to draw them. That is not a
flaw in 0007. It is the seam announcing itself, and the amendment is the evidence that one file
was already being asked two questions.

## Decision

**Three statements, and a gate that makes them safe.**

### 1. The compiler declares; the front end displays

The direction is from compiler to front end, because the front end never speaks to a simulator.
`gsim-core` declares, for each component it supports: the kind, its ports and their signal
natures, its parameters and their natures, what it compiles to, and which paradigm and fidelities
apply. That is the **specification**, and it is the compiler's alone — it is the only thing that
knows what it can emit.

### 2. The front end owns the catalogue of what it can display

A symbol, a label, a category, help text, which parameter is the prominent one: these belong to
the front end, because the front end is what draws a symbol and collects a value. The compiler
names a display kind — *this is a resistor*, *this is a MOSFET* — and the front end decides what
a resistor looks like.

This also settles what 0007's amendment had to work around: a component a front end must draw
but no engine compiles is **a front-end catalogue entry with no compiler declaration**, which is
a sentence the split can say and the single file could not.

### 3. The front end keeps capabilities no compiler grants

Placing, wiring, selecting, undo, layout, and the presentation of hierarchy are the front end's
regardless of which compiler is attached. A front end with no engine loaded is still an editor.

### And the gate, which is the condition of the split

**Every kind the compiler declares must exist in the front end's catalogue, with matching port
count, port order and parameter names, or CI fails.**

This is not optional bookkeeping; it is the reason two owners are safer than one rather than more
dangerous. Drift between a symbol and a specification is this project's recurring defect — the
signal-domain port mismatches the owner reported came from exactly that shape, with one file
supposedly preventing it. Two files and an assertion beat one file and a hope, but only with the
assertion.

## Sub-schematics belong to the compiler

The owner considered both and landed on the compiler. That is right, for a reason worth recording
because it generalises: **a sub-schematic has to flatten into a netlist, and flattening is
compilation.** Node renaming, parameter scoping, collision handling and the rules for what a
port means are all decisions the compiler already makes for a flat sheet.

A front end that flattened would be a second implementation of that, and the two would disagree
— the same failure as two expression evaluators, which `#94` was opened to end. The front end
owns the *presentation* of hierarchy: opening a block, drawing its interior, editing inside it.
The semantics are not presentation.

Current state, stated so no one assumes otherwise: subcircuit machinery exists in this crate's
`compile/` — `library.rs`, `routing.rs`, `spec.rs`. The editor has **no** sub-schematic support
at all. This is a front end to be built against a back end that partly exists, not a design to
be invented on both sides.

## A composite component is a sub-schematic with a symbol and promoted parameters

The owner wants a resistor that is not an ideal resistor:

> *"I want the possibility to have a resistor which is not only a simple resistor, but a resistor
> with for example a series capacitance, a parallel capacitance, a series inductance, to model a
> real one. When you double click on them, they are not one component, but there are many in
> them, and you can choose the value of any one of them independently."*

**This is not a second mechanism.** A real resistor is two terminals outside, three components
inside, and three independently editable values surfaced on the outer block. That is a
sub-schematic, plus:

1. **a symbol** — it is drawn as a resistor, not as a box with two pins;
2. **parameter promotion** — named inner values appear as parameters of the outer component.

One mechanism, two presentations. The consequence is that composite components are not scheduled
after sub-schematics as a separate feature; they are what sub-schematics are *for*, in the case
that matters most to this project's actual work. And both emit the same thing: a parameterised
subcircuit definition instantiated per use, which is a form this engine must be able to emit
anyway.

## What this costs

**Two files to keep in step instead of one.** Paid for by the gate, and the gate is cheaper than
the defect class it replaces.

**A migration with no user-visible result.** Splitting 78 entries across two owners changes
nothing anybody can see, which makes it the kind of work that gets left half-done. It should be
one change that moves every component, with the gate landing in the same commit as the split, or
it will be a third state that is neither.

**A display kind is a new vocabulary**, and vocabularies drift. The gate covers ports and
parameters; it cannot check that a thing declared a MOSFET is drawn as one. That is a review
question and this record does not pretend otherwise.

## What this does not decide

**Libraries of components** — how a user ships, versions and shares a set of composites. The
owner has deferred it explicitly, twice, and it should not be smuggled in through the
sub-schematic work. A composite that cannot yet be shared is still worth having.

**Whether a second target ever exists.** 0007 and `AGENTS.md` invariant 2 say `general-simulator`
is the one `NetlistTarget`. Nothing here changes that; the split makes a second target a smaller
change if it ever happens, which is a consequence and not a reason.

## Amendment — a parameterised subcircuit is bound by the solver, not substituted here (2026-11-27)

The section above calls per-instance parameters *"the one that matters most"*, and it is the one
thing this record left unplaced: it says a sub-schematic must take parameters per instance, and
never says which layer turns `Rv=100` on an `X` card into a resistance. Measured on 2026-11-27,
against `general-simulator 0.10.0`, the answer was **no layer at all**.

Two decks. The first solves:

```spice
.param Rv=330
V1 A 0 5
R1 A 0 {Rv}
```
→ `t,V(A),I(V1)` / `0,5,-0.015151515151515152` — 5 V over 330 Ω, so the §5.2 evaluator and its
symbol table work.

The second does not:

```spice
.subckt RPART p n Rv=1000
R p n {Rv}
.ends RPART
V1 A 0 5
X1 A MID RPART Rv=100
X2 MID 0 RPART Rv=220
```
→ `error: Evaluate(MissingSymbol): evaluating an expression failed: missing value for symbol
'Rv'`.

The grammar is not the problem — `spice-lsp/docs/GRAMMAR.md` already specifies both the formal
list and the instance tail, and both parse. What is missing is a **scope**: nothing binds a
definition's formals inside the body it expands.

The capability stops in three places, and each stores the value it then ignores:

| layer | what it holds | what it does with it |
| --- | --- | --- |
| `gsim-core` | `SubcktDef::params`, `SubcktInstance::params` (`document/subckt.rs`) | nothing — `.subckt NAME ports` carries no formals and an `X` card is `X<id> <connections> <def>`; no code under `compile/` reads either field |
| `spice-lsp` | `Subckt::params` | parsed, and no consumer anywhere |
| `general-mna` | the `X` card's tail | `hierarchy::expand_body` threads a `port_map` for nodes and nothing for parameters |

So two instances with different parameters compile byte-identically, with no diagnostic. That is
the silent-wrongness class this project treats as its worst defect, and it is why the repair is
not simply "emit the cards".

**The owner's decision: the solver substitutes.** `general-mna`'s flatten binds the definition's
declared defaults, overrides them from the instance's `ident=value` tail, and resolves brace
expressions as it expands — `GLODI-Engineering/general-mna#63`.

**Why not here.** This crate could substitute at compile time, by emitting one `.subckt` per
distinct parameter set or by inlining the body per instance. Both make the compiler a second
implementation of flattening, which is the failure this record opens by naming, and both destroy
what the netlist is for: the owner reads these decks, and a deck in which `X1` and `X2` differ by
a resolved number no longer shows that one definition is instantiated twice.

**The ordering rule this makes normative.** *This crate must not emit a parameter it cannot show
the solver binds.* Emitting `Rv=100` before a `general-simulator` release carries general-mna#63
is worse than today's state, not better: the deck parses cleanly and loses the value one
repository further down, where nothing is looking for it. Until then, a per-instance parameter
on a subcircuit instance is **refused with the instance and parameter named** — never dropped.
Refusal is correct under either answer, so it is not waiting on anything.

The sequence is therefore: general-mna#63 → a `general-simulator` release carrying it → `#111`
emits formals and the instance tail → `#112`'s composite component, which is the first feature
that needs any of it.

### Closed, 2026-11-27

That sequence ran in a day, so the refusal clause was never needed: `general-mna#63` landed,
`general-simulator 0.11.0` shipped carrying it, and `#111` emits the formal list on the
`.subckt` card and the `name=value` tail on each `X` card. The ordering rule is kept in place
rather than struck out, because it is the general statement and not a note about one feature —
**this crate does not emit what it cannot show the solver binds**, and the next capability that
crosses the same seam is held to it the same way.

Two things were found on the way and are worth keeping beside the decision:

**A reference inside a definition was emitted verbatim** (`#121`). §5.3 substitution walked the
sheet's blocks and never a definition's, so a resistor inside a `.subckt` valued `"{Rload}"`
reached the deck as `{Rload}` and failed in the solver as a symbol nothing declares — while the
same reference one line outside the definition resolved. That had to be repaired before this one
could be, and it is what settled where the boundary between the two scopes lies: a body's
reference to one of the definition's **own** parameters is left for the solver, anything else
resolves here, and a value naming both is refused because there is no single scope to read it in.

**An instance parameter the definition does not declare is refused on both sides.** The solver
refuses it when it binds (`general-mna#63`) and this crate refuses it when it emits, which is
not duplication: the compiler can name the block as the *document* names it, before any `X` card
exists to point at, and a front end gets the refusal without a solver in the loop.
