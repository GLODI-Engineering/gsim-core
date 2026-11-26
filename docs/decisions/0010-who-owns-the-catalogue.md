# 0010 — Who owns the catalogue, and what a composite component is

**Status:** accepted 2026-11-26. Refines [0007](0007-the-library-is-what-the-engine-compiles.md)
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
