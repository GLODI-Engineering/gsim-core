# 0005 — A drawn document that names a file on disk

**Status:** accepted 2026-11-19. `GLODI-Engineering/gsim-core#48`; the engine side is
`general-mna#45` and `general-simulator#104`.

## Context

`kind=filewave` reads a measured waveform — an oscilloscope capture, or a table printed by the
microcontroller driving the board — and becomes one output per value column. A drawn schematic
has to be able to place one, which raises two questions no other type in this registry does.

**A document that references data outside itself** is new here. Three existing types name a file
(`cscript`'s `lib=`, `pyblock`/`pyfunc`/`octfunc`/`octblock`'s `path=`), but none of them has
ever been placed from a drawing with a path the editor manages, and none of their *shapes*
depend on the file's contents.

**An output count that comes from a file** is new too. `DynamicOutputs` already handles a count
that comes from a parameter; a count that comes from the number of columns in a file on disk is
a different thing, because the file is not part of the document and can change without it.

## Decision

**The document carries the column names. The compiler never reads the file.**

`output_names` is an ordinary list parameter, and the symbol's ports come from it. A sheet
therefore draws, reviews, diffs and compiles with no data file present at all — a test compiles
one without creating a file.

The alternative was to read the capture at compile time and take the count from it. It fails on
the cases that matter rather than on the happy one:

- A schematic whose pins appear only when its data file happens to be present is a schematic
  nobody can review. A reviewer opening a shared document would see a symbol with no outputs and
  a sheet with no connections, and nothing would say why.
- A file swapped for one with fewer columns would **silently change the drawing** — wires
  deleted because a column went away, which is a circuit edit made by a file copy.
- Compiling would need to read every referenced file, so a document could not be compiled
  anywhere its data is not, including in CI.

What the document keeps is the user's declaration of what the file holds. A file that stops
matching it is the engine's diagnostic — `'capture.csv' has no column 'CH3'; it has 2, named
CH1, CH2` — raised at parse time, before a step, naming the file and what it does hold. That is
a better failure than a redrawn symbol, because it says what changed.

**The path is emitted exactly as written.** Not resolved, not made absolute. The engine resolves
a relative path against the **process's working directory**, which `cscript.md` already
documents for `lib=`, so:

> a netlist compiled from a document is meant to be run from that document's own directory.

Rewriting the path here would be this crate guessing where the engine will be run, and baking an
absolute path into a document would break it for everyone the document is shared with. The
editor, which is the only component that knows where the document lives, is responsible for
running the engine from that directory — recorded as a requirement in `omni-schematics#98`
rather than left for someone to discover.

**Channel names are prefixed with the block.** The registry's existing `names_param` convention
emits `outputs=SCOPE,SCOPE_VOUT` for a capture named `SCOPE` whose columns are `VGS,VOUT`. This
was not chosen for this type; it is what every script block already does, and it turns out to
matter more here than there: a multi-output block's channels past the first are **top-level
signal names** in the engine, so a bare `VOUT` from a drawn capture would claim that name
globally and collide with anything else called `VOUT`. A drawn document gets collision-free
names for free, and a hand-written netlist opts into bare names by spelling `outputs=` itself.

## Consequences

- A document is portable. It references its data by a relative name and carries everything
  needed to draw and review it.
- Compiling never touches the filesystem, so it stays a pure function of the document.
- The editor owns path resolution and the working directory it runs the engine in. Until it
  does, a file source placed from the editor works when the engine happens to run in the right
  place, which is the one rough edge this decision accepts and `omni-schematics#98` closes.
