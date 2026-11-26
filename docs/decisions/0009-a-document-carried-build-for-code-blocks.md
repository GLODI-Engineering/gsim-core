# 0009 — A document-carried build for code blocks, and the define that has to be derivable

**Status:** accepted 2026-11-26. `GLODI-Engineering/gsim-core#98`, raised by #97. Builds on
[0005](0005-a-document-that-names-a-file.md) (a document that references a file outside itself)
and [0008](0008-subsystems-and-buses.md) (what "preserve, do not interpret" settles). Reverses
the first of the three decisions recorded on `GLODI-Engineering/omni-schematics#253`.

## Context

A `cscript` block's parameter is **`lib`** — the compiled artifact. Nothing in the document says
what it was compiled from, with which include directories, or with which `-D` defines. The other
four extensibility types name a source with `path=`, but nothing about how it is prepared.

`GLODI-Engineering/omni-schematics#235` measured what that costs. A generic one-input C block,
`out[0] = 2.0 * in[0]`, compiled; the source then edited to `10.0 * in[0]` and not rebuilt. The
run exited 0, wrote a complete samples container and a complete manifest, and the waveform was
`2 x` the input. There is no warning anywhere in the output. Editing a block's code in the tab
the editor itself opened, pressing Run, and being shown the previous code's numbers is the whole
sequence, and nothing in it is loud.

Three facts about real code blocks decide what a fix has to be able to say. They come from
reading how these blocks are actually built, and each kills a smaller design:

1. **A C block is never one file.** The pattern is a thin `cscript_*` adapter plus one or two
   *shared* sources — a control law, a modulator, a transform — reused by several blocks on the
   same sheet, built with `-I` for the shared headers and `-lm`. A scheme that compiles "the
   `.c` next to the `.so`" links none of them.
2. **The build flags come from the circuit.** Blocks are rebuilt per operating point with `-D`
   defines derived from *the same parameter dictionary that generates the netlist* — the
   inductance, the capacitance, the reference, the load appear on both sides. A parameter sweep
   is N builds of one source with N define sets.
3. **"Build this block" is a graph, not a command.** One project compiles a host program, runs
   it to emit a generated lookup-table header, and only then compiles the block.

And the layout a convention cannot reach: sources live in a `code/` folder, and the artifact is
written into a per-case run directory beside the netlist.

`omni-schematics#253` weighed two answers — the extension running a build command the user
configures, or the document carrying the description — and recommended the command first with
the document "as the long-term shape". The owner chose the document, immediately, and said why:
consistency. This record is that choice, taken properly rather than taken as written.

### What makes this worth a record rather than a schema patch

Fact 2 is not a convenience. A build that omits the defines produces a block whose constants
disagree with the circuit it is wired into, and the run then reports a **confident wrong
answer** — #235 again, caused by the feature meant to end it. The asymmetry is the whole
argument: a failed build is obvious and costs a minute, while a wrong constant is a plausible
waveform that can survive a whole study. This repository family spent 2026-11-26 removing three
bugs of exactly that shape (`omni-schematics#235`, `#236`, `#244`). The feature must not add a
fourth.

So the question this record exists to answer is not "where do we put the flags". It is: **what
makes "these constants came from these parameters" checkable rather than asserted?**

## Decision

### 1. One compile call produces the netlist and the build, from one read of the parameters

This is the central decision and everything else follows from it.

The resolved build is an output of `compile()`, beside the netlist and the provenance map:

```rust
pub struct CompileOutput {
    pub netlist: String,
    pub notes: Vec<String>,
    pub provenance: Provenance,
    pub code_build: Option<ResolvedCodeBuild>,   // new
}
```

A caller cannot obtain a deck without obtaining the build description that belongs to it,
because they are the same return value from the same call, resolved against the same
`model.params` in the same pass. There is no sequencing in which one is stale with respect to
the other, and no second parameter lookup to get wrong.

The converse is what gives the refusal rule its teeth: when a define cannot be derived,
`compile()` returns `Err`, so **there is no netlist either**. The refusal is not a build step
declining to run — that could be skipped, worked around, or made a warning by a hurried reader.
It is the deck failing to exist. A caller that wants to run the circuit anyway has nothing to
run.

### 2. The plan is on the model; the block names its step

`model.code_build` is one object per sheet. It holds a list of **steps**, plus the include
directories, defines, flags and libraries shared by all of them. A block names the step that
produces its artifact.

```json
"code_build": {
  "includes": ["code", "code/generated"],
  "defines": [
    { "name": "L_F",   "from": "L_f" },
    { "name": "C_F",   "from": "C_f" },
    { "name": "FSW",   "from": "fsw" },
    { "name": "N_PHASES", "literal": 3 }
  ],
  "libs": ["m"],
  "steps": [
    {
      "id": "ctrl",
      "kind": "shared-library",
      "lang": "c",
      "sources": ["code/cscript_ctrl.c", "code/control_law.c", "code/clarke.c"]
    },
    {
      "id": "modulator",
      "kind": "shared-library",
      "lang": "c",
      "sources": ["code/cscript_mod.c", "code/control_law.c", "code/clarke.c"],
      "defines": [{ "name": "MOD_DEADTIME_NS", "from": "t_dead_ns" }]
    }
  ]
}
```

```json
{ "id": "CTRL", "type": "extensibility.cscript",
  "params": { "lib": "runs/case3/ctrl", "input_names": ["e"], "output_names": ["d"] },
  "source": "code/cscript_ctrl.c",
  "build": "ctrl" }
```

**Why the model and not the block.** Five blocks sharing two sources is the case to serve, and
it has three parts that each belong somewhere different. The *shared sources* belong to the
sheet, so they are listed once per step and the steps sit together where a reader can see that
two of them name `control_law.c`. The *include directories and defines* belong to the sheet
outright — they are the circuit's own numbers, and repeating them per block is the drift this
record exists to prevent. The *artifact* belongs to the block, and stays there (§4). A
multi-step graph belongs to no block at all: the host program that generates a lookup table is
not any block's property.

Step-level `includes`, `defines`, `flags` and `libs` are **appended** to the plan-level lists,
in that order. One level of composition, no recursion, no templating, and a step can add but
never subtract — stated here because it is a real limitation and someone will meet it. A step
that must not see a plan-level define does not exist today; if one does, it needs its own
record, not a `"shared": false` flag added quietly.

**`source` and `build` are siblings of `params`, not entries in it** — the same placement, and
for the same reason, as [`Block::ratings`]. Neither is a model parameter: no `TypeSpec` field
emits them onto a card, and neither changes when the block's fidelity tier changes, so a tier
that did not list them would delete them (invariant 6). As typed fields on `Block` they are
validated by this crate rather than hoped for.

`source` is the file the user edits, and it settles this crate's half of
`GLODI-Engineering/omni-schematics#222`: a `cscript` block's source becomes a fact the document
states rather than four candidate paths the editor tries. It is not emitted into the netlist.
It stands on its own — a block with `source` and no `build` is a well-formed block, which is
what makes #222 fixable before anything in this milestone ships, and what keeps #235's staleness
check honest on a sheet with no plan.

### 3. A define is a binding, and the binding is the only spelling that reaches a parameter

A define is one of exactly two shapes, and they do not look alike:

| shape | meaning |
| --- | --- |
| `{ "name": "L_F", "from": "L_f" }` | the value of `model.params["L_f"]`, the dictionary the netlist is generated from |
| `{ "name": "N_PHASES", "literal": 3 }` | a constant that is not a circuit quantity, written out by the author |

Exactly one of `from` and `literal` is present. Neither, or both, is refused.

There is no third shape, and in particular **no form in which an author writes a number that
is also on a component**. That is the drift this record is about: two places holding one
quantity, one of them edited. `literal` exists because `-DN_PHASES=3` is real and is not a
circuit quantity, and it is spelled differently from `from` so that a reader scanning the list
can see which constants the circuit owns and which the author does.

`from` names a **model parameter**, by its bare name. A dotted `"<block>.<param>"` is refused
with a message saying so, rather than silently treated as a name containing a dot: binding a
define to a component's own parameter is a reasonable thing to want, and reserving the spelling
keeps it available for a record that designs it. It is not needed today, because the pattern
this serves already puts the quantity in `model.params` and references it from the component
(§5.1), which is the same dictionary from the other side.

**The value is formatted by the same code that formats it into the netlist** (`crate::fmt`).
This is not an implementation note; it is the point. A separate formatter is a second
implementation of "what number is this", and the two agree until an exponent or a trailing zero
makes them not.

#### The whole loop, in one example

The owner's description of what he wants is the test this format has to pass, so it is written
out end to end rather than described. A constant is declared **once**, in the sheet; the
schematic uses it for a device value; the C source names it as a macro; the same number arrives
on both sides.

**Declared once** — in `model.params`, which is where `.gsim_sch` §5 already puts a sheet's
constants. A parallel table was considered and rejected in one line: the whole claim is that the
build and the netlist read *the same* dictionary, and a second table is two dictionaries however
carefully they are kept in step. `params` is also already editable (the "Model Parameters"
drawer), already referenced from a device value, already sweepable (§13.2) and already the thing
`.PARAM` means.

```json
"params": [
  { "name": "L_f",      "value": 2.8e-6, "unit": "H"  },
  { "name": "fsw",      "value": 250000, "unit": "Hz" },
  { "name": "vq_table", "value": [0.0, 0.25, 0.5, 0.75, 1.0] }
]
```

**Used in the schematic.** The inductor's value is the parameter, by §5.1 reference:

```json
{ "id": "L1", "type": "passive.inductor", "params": { "L": "L_f" } }
```

**Named by a macro in the C source**, which carries no number at all:

```c
#include "gsim_cscript.h"

/* L_F and FSW arrive as -D on the compile line. LUT_VQ arrives in a generated header
   that the compile line force-includes, so there is no #include to write here either.
   Nothing in this file is a literal. */
static const double vq[] = LUT_VQ;

void cscript_output(void *state, const double *in, int in_len, double dt,
                     double *out, int out_len) {
    out[0] = vq[(int)(in[0] * 4.0)] * dt / (L_F * FSW);
}
```

**Bound in the plan**, which is the one place that says which sheet constant answers to which
macro:

```json
"defines": [
  { "name": "L_F",    "from": "L_f" },
  { "name": "FSW",    "from": "fsw" },
  { "name": "LUT_VQ", "from": "vq_table" }
]
```

Nothing in that list says which constant travels how. The shape of the resolved value decides.

**And the sides agree, by construction.** One `compile()` produces all of it:

```text
L1 n1 n2 L_f                                     <- the netlist card
-DL_F=2.8e-6 -DFSW=250000                        <- the compile line, scalars
-include runs/case3/ctrl_gsim_constants.h        <- forced, so the .c writes no #include
```

```c
/* runs/case3/ctrl_gsim_constants.h -- generated every run, beside the artifact, gitignored.
   Never written into the user's code/ folder: it is a build output, not a source. */
#define LUT_VQ { 0, 0.25, 0.5, 0.75, 1 }
```

Change `L_f` in the drawer and both change, because both came from the same read. Edit
`vq_table` and the header changes, the artifact is rebuilt (§6's record compares the content,
not the header's timestamp — it is rewritten every run, so a timestamp would say nothing), and
the run uses the new table. Misspell either `from` and **none** of it exists, because `compile()`
refused — the same refusal, with the same message, whichever shape the constant had.

#### How the value reaches the macro: three options, and the decision

The owner first described the mechanism as *"the macro will replace those by defines in the
file"*, and then said plainly that the description was not binding — *"If what I'm suggesting is the right
one, yes, we implement it. If yours is better, we take yours."* So the three candidates are
written out, because the argument is the deliverable here as much as the code is.

**Option A — `-D` on the compile line.** The user's `.c` is compiled untouched; the C
preprocessor does the substituting, with the value the command line gave it.

**Option B — substitute into the source.** Rewrite the `.c` (or a generated copy of it) with the
numbers pasted in, then compile that.

**Option C — a generated header.** Write the constants into a header in the build directory,
`#include`d by the block. The user's `.c` is untouched; the values arrive as `#define`s in a
file the build produced.

**Decided by the owner, after reading the argument: A for scalars, C for anything structured —
a lookup table, an array, a struct initialiser. B is rejected.** His words:

> I take your suggestion to use the `-D` to send the scalars, and for other data — lookup table,
> arrays, struct initializer — we create an include file alongside it, an `.h` file, and include
> it in our C file automatically and work based on it.

Against **B**, five things, and the first two are not preferences:

1. **It breaks the diagnostics the editor just built.** `omni-schematics#237` made a compiler
   error a clickable Problems entry on a file and a line. Compile a rewritten copy and every
   diagnostic points at a file the user never wrote, at a line that is not theirs. The standard
   answer is `#line` directives, which then have to be honoured by the Problems entry, by a
   debugger stepping into the `.so`, and by anything else reading the artifact's line table —
   three places to be right instead of zero.
2. **It breaks the editor the user is typing in.** `clangd` reads the file on disk. Rewriting
   into a copy leaves it looking at the original, where `L_F` is undefined, and underlining it.
   The fix is a `compile_commands.json` carrying the same `-D` flags — at which point the flags
   exist anyway and the rewrite has bought nothing. Under A, one flag list serves the build *and*
   the language server, and the file the user opens is the file the compiler read.
3. **It writes into the user's source tree**, or it makes a second copy — and a copy is a trap,
   because the next person edits the wrong one.
4. **It hides the constants.** The builder prints its command line verbatim before spawning
   (`omni-schematics#253`'s own requirement). Under A the values are *in that line*, so the
   Output panel says which constants the artifact was built with, and the line can be pasted
   into a terminal to reproduce the artifact by hand — worth a great deal when a result is
   disputed, and it is how the build scripts behind this feature already pass their defines.
   Under B you have to find and open a generated file.
5. **It needs a preprocessor nobody wrote.** Substituting a macro textually means getting
   strings, comments, partial identifiers and token pasting right — a second, hand-rolled C
   preprocessor inside a front end, in a feature whose entire subject is not being silently
   wrong.

B's one honest advantage is inspectability, and A has it without the file: §1's resolved step
carries the list, the command line prints it, and the build record (§6) keeps it beside the
artifact. Three places say `L_F=2.8e-6`.

**Where A stops, and what C does about it.** `-D` carries a scalar well and a structured
constant badly. C covers that with every property above intact: the `.c` is untouched, the
diagnostics are unmoved, `clangd` sees a real header, and the generated file is inspectable
because it *is* a file.

#### The generated header, and what "automatically" has to mean

The owner asked for the header to be included **automatically**. Taken literally — the editor
writing an `#include` line into the user's `.c` — that is option B again, with every objection
above restored. The mechanism that keeps the promise instead of breaking it is a **forced
include on the compile line**: `-include <header>` for clang and gcc, `/FI <header>` for MSVC.
The header is in scope without the user writing a line, and their source is never touched, which
is the whole reason A beat B.

Asking the user to write the `#include` themselves would also work, and is rejected as a default
for a specific reason: it is a line they must remember in *every* block, and forgetting it is a
compile error about an undefined identifier rather than a sentence about a missing include — a
late, confusing failure in the one feature whose subject is not confusing failures.

Three properties the header has to have, and each is a rule rather than a preference:

1. **It is a build output, not a source.** It is written beside the artifact — in the directory
   the block's `lib` names — regenerated every run, and gitignored. It must never land in the
   user's `code/` folder, where it would look like something they wrote and would be stale the
   moment a parameter changed. This crate emits its path *relative to the schematic*, derived
   from `lib`, so the document stays portable and no machine path is baked in.
2. **It is part of staleness.** `omni-schematics#235` refuses a library older than its source. A
   library is equally stale when the **constants** changed and it was not rebuilt, so the artifact
   has to be rebuilt when the header's content changes — and the check must compare *content*,
   never the header's timestamp. It is rewritten every run, so its mtime is always fresh and a
   timestamp check would be vacuous. §6's build record already carries the resolved list
   verbatim, which is exactly the content to compare; this is the same equality test, extended to
   cover the structured half.
3. **Both paths obey §9 identically.** A structured constant whose parameter the sheet does not
   declare refuses exactly as a scalar one does, with the same message naming the macro and the
   parameter. If the two differed in strictness, an author's choice of *shape* would quietly
   change how safe their build is.

Property 3 is not achieved by writing the same check twice. **It is achieved by there being one
list.** A `defines` entry says nothing about which mechanism carries it: the *shape of the
resolved value* decides. A scalar becomes `-DNAME=VALUE`; an array, or an array of arrays,
becomes a `#define NAME {...}` line in the header. One declaration site, one resolution pass,
one set of refusals, and the author never chooses a transport — so there is no second path that
could be laxer than the first.

```json
"defines": [
  { "name": "FSW",    "from": "fsw" },        /* a scalar  -> -DFSW=250000      */
  { "name": "LUT_VQ", "from": "vq_table" }    /* a table   -> a header #define  */
]
```

This is also what C costs, stated plainly: a structured constant has to be **declarable** in the
document, and `.gsim_sch` §5.1's `ParamValue` is a number, a boolean or a string. Extending it to
carry an array is part of this work rather than a later convenience, because without it the sheet
has nothing to bind and C is a transport with no cargo. The extension is additive and is already
what the engine accepts on the other side: `general-simulator`'s block grammar takes a
Python-style list literal (`a=[[1,2],[3,4]]`) as a field value, and this crate's registry already
has matrix-shaped parameters, so an array in `params` is a gap in the format's written form
rather than a new capability.

**One distinction to keep, because conflating these two would be a design error.** The real
project that generates a lookup-table header does not *declare* that table — it compiles a host
program and runs it to **compute** one. That is a build step whose product another step consumes
(§10's first expected kind), not a constant the sheet wrote down. A declared table needs a value
in the document; a computed table needs a step graph. The format has room for both, and should
not grow one mechanism that half-serves each.

#### Who writes the `defines` list

The document does, and that is what keeps the refusal mechanical: the author writes a name, the
sheet declares it or it does not, and nothing scans anything.

The alternative — take the define set from the macros the `.c` actually uses — is the one piece
of the owner's description this record implements differently, so the difference is stated
plainly. It would mean the build set depends on the *contents* of a file outside the document,
which 0005 settled against for `filewave` on reasoning that applies unchanged here: a document
whose build cannot be reviewed, diffed or compiled without its sources present is a document
nobody can review, and a source edited to drop a macro would silently change the build. It would
also mean this crate reading a `.c`, which it will not do.

What bridges the two is a front-end convenience with precedent in this very editor: the
Inspector already offers **"Read ports from source"**, which parses a block's source once and
fills `input_names`/`output_names` with what it found, for the user to accept. The same gesture
fills `defines` from the macros a `.c` names. The document still carries the declaration — so
the refusal still has something to refuse against — and the user still never types a name twice.
Filed on the editor as the companion to this record.

### 4. The artifact is the block's `lib`, and nowhere else

A `shared-library` step has no `output` field. Its artifact is the `lib` parameter of the
block(s) that name the step. If two blocks name one step, their `lib` must agree, which is
refused rather than resolved.

The alternative — a step declaring its own output, which the block's `lib` then has to match —
puts the same path in two places, and this record is about not doing that. Deriving `lib` from
the step instead was also rejected: `lib` is what the netlist card carries, it is what a
hand-written netlist and a pre-feature document already use, and it is what
`general-simulator#125` made platform-neutral by resolving a suffix-less `lib=` to the
platform's own library name. So `lib` is the one place the artifact is named, and a step is
told where to put its product by the block it serves.

This is also what makes the owner's layout expressible without a new concept: sources at
`code/...`, `lib` at `runs/case3/ctrl`, both relative to the schematic — a sheet in a `code/`
project writes down exactly the layout it already has.

### 5. None of this reaches the netlist, and that is forced rather than chosen

Worth stating before §6, because it bounds every other decision here.

`general-mna`'s `build_kind` ends with a check nobody can opt out of: a `Fields` wrapper records
every key any arm looked up, and `fields.unread()` then returns the first field on the line that
no parser asked for. An unknown `key=value` on a `kind=cscript` card is a **hard error** —
`line N: device 'S' unknown field 'gain' (kind=cscript accepts only: ...)` — and the comment
above it says the escape hatches are deliberately not exempt, because their configuration lives
in their library, not on the netlist line. A test is named for it.

So `build=`, `source=`, `defines=` or anything else on a code-block card would make every
existing engine refuse the whole deck, and the gate lives in a repository this crate treats as a
read-only path dependency. The description is therefore a **document and front-end contract
end to end**: it is resolved here, it is executed by whoever runs the build, and the netlist
never mentions it. `general-simulator`'s own `--capabilities` already says the same thing from
the other side — `cscript=load-only`, documented as "this binary loads a shared library and
never invokes a compiler. A caller that wants a `.so` built has to build it."

That is also the split the owner stated, and it is worth recording in his terms because it is
an architectural position and not only a consequence of a parser check:

> it is the responsibility of the compiler to take the information from the view and compile it
> accordingly — compile the schematic to the netlist, compile the C files — and send it to the
> simulator. The simulator, when it comes, it's just to take the schematic, everything set, and
> it does the simulation.

So the solver receives a netlist and artifacts that are **already consistent**, and builds
nothing. This is the existing division taken one step further rather than a new one: the webview
already compiles the netlist through `gsim-wasm`, the host already spawns the solver, and the
host now also produces the artifacts. `--capabilities`' `cscript=load-only` is the solver's own
statement of the same thing.

The corollary for the document is the owner's third point, and it closes
`omni-schematics#222` in the form he asked for: **the paths are set in the view.** Where the
`.c` is taken from is `Block::source`, where the `.so` is written is the block's `lib`, both
relative to the schematic and both editable beside the block's other properties. Neither is a
convention the editor guesses at — which is what `#222` was filed about, and what the four
candidate paths in `netlist/code-source.ts` were working around.

The three carriers that *would* smuggle it into a deck — a column-1 `*` comment, an unrecognised
dot command, a `kind=build` card stripped by the CLI before `general-mna` sees it, which is how
`kind=measure` and `kind=rule` already work — are all real and all rejected here. Two of them
disturb the checkpoint deck hash, and none of them is needed: nothing downstream of the netlist
has any use for the build. The one place a deck would genuinely want a say is checking the
receipt at load time (§6), and that is a field on nothing — it is a file beside the artifact.

### 6. The receipt: what makes it checkable *afterwards*, which is the case that matters

Refusing at build time guarantees the artifact matches the circuit **for the build that just
ran**. It cannot say anything about the artifact already on disk — built last week, on another
branch, at another sweep point — and that is precisely the artifact a run picks up. A
build-time-only check leaves the original failure intact, one step further back.

So a build writes a **build record** beside its artifact, and the thing that starts a run
compares it against what `compile()` resolved for the same step. A mismatch refuses the run.

```json
{
  "format": "gsim-build-record/1",
  "step": "ctrl",
  "artifact": "runs/case3/ctrl.so",
  "defines": ["L_F=2.8e-06", "C_F=5e-06", "FSW=250000", "N_PHASES=3"],
  "omitted_defines": ["DEBUG_LOG"],
  "header": {
    "path": "runs/case3/ctrl_gsim_constants.h",
    "defines": ["#define LUT_VQ { 0, 0.25, 0.5, 0.75, 1 }"]
  },
  "includes": ["code", "code/generated"],
  "sources": ["code/cscript_ctrl.c", "code/control_law.c", "code/clarke.c"],
  "libs": ["m"],
  "compiler": { "program": "clang", "source": "setting" },
  "built_at": "2026-11-26T11:04:08Z"
}
```

**The record carries the resolved defines themselves, not a digest of them.** A digest answers
"is something wrong"; the list answers "what is wrong" — `L_F was built as 2.8e-06, the circuit
now says 3.2e-06` is a sentence a user can act on, and `digest mismatch` is not. It also costs
this crate no new dependency, which a hash would. The comparison is therefore defined as
element-wise equality over all three lists, in order, and §7 fixes that order so two
implementations compare the same thing. The header's lines are compared the same way, which is
what makes a changed lookup table a stale artifact rather than an invisible one — the header is
rewritten every run, so its timestamp is always fresh and says nothing.

`compiler` is the one field of the record this crate has nothing to say about: it is a statement
about a machine (see "Does this crate interpret the description, or merely carry it?" below), so
the builder fills it in from its own choice and compares it on the way
back. Everything else is produced here, which is why the define lists can be compared at all.

The record is written by whoever runs the build and read by whoever starts the run. This crate
defines its content and produces the lists to compare against; it does not write files.

### 7. The canonical define list

Specified here because it is a wire format between two components and an equality test:

1. The plan's `defines`, in document order, then each step's own `defines`, in document order.
2. A later entry with the same `name` as an earlier one **replaces** it in place — it does not
   append a second `-D`. A duplicate `name` within one list (plan or step) is refused; a step
   overriding a plan-level define is the one legitimate repeat, and it is the reason the rule is
   "replace in place" rather than "refuse".
3. **The resolved value's shape decides which list the entry lands in**, and it is the only
   thing that does — see §3. A number, a boolean or a string renders as `NAME=VALUE` in the
   `-D` list, where `VALUE` is `crate::fmt::stringify` of it. An array, or an array of arrays,
   renders as `#define NAME { ... }` in the header list, with a nested array becoming a nested
   brace initialiser, which is what a C array or a struct initialiser wants.
4. A `NAME` with no `=` is not expressible. A define with no value is a different thing from a
   define with a value, and nothing in the cases this serves needs one.
5. An `optional` define whose parameter is absent produces **no** entry in that list. Its name
   goes instead into a second, parallel list — `omitted_defines`, in the same document order —
   so that "built without `DEBUG_LOG` on purpose" and "built without `DEBUG_LOG` because
   nobody noticed" are different values, visible in the receipt and compared by the same
   equality test. An artifact built when the parameter was absent does not match a circuit that
   now sets it, and the mismatch says which.

### 8. What is refused

Every one of these is a `CompileError`, so none of them produces a deck:

| refusal | message names |
| --- | --- |
| a define's `from` names no `model.params` entry | the define, the parameter, and the parameters that do exist |
| a define's `from` names a parameter whose value is a §5.2 expression or a §5.1 reference | the define, the parameter, and the unevaluated text |
| a define with both `from` and `literal`, or neither | the define |
| a define's `from` is dotted (`"L1.L"`) | the define and the reserved spelling |
| a duplicate define `name` within one list | the define and the list |
| a block's `build` names no step | the block and the step id |
| a step id used twice | the step id |
| a step with no `sources` | the step |
| a step kind this version does not implement | the step and the kind |
| a step whose `needs` names no step, or whose `needs` form a cycle | the steps on the cycle |
| a step no block names, and which nothing `needs` | the step |
| two blocks naming one step with different `lib` | both blocks and both paths |
| a block with `source` and `build` where `source` is not among the step's `sources` | the block, the source, and the step's sources |
| a `build` on a block whose type is not `extensibility.cscript` | the block and its type |
| an absolute path, or a `..` segment, in `sources`/`includes`/`lib` | the path and the field |

The second row is the one that will fire most often and is the most important to get right.
`model.params` is **not evaluated by this crate** — `#94` is open, and `ModelParam::value` says
so — so a parameter whose value is `"{Rload * 2}"` is, today, genuinely underivable. It is
refused, naming the expression, rather than emitting `-DR={Rload * 2}` for a compiler to choke
on or, worse, emitting nothing. When #94 lands, the same documents start building with no format
change, because the format never said anything about expressions: it said the value has to be
derivable.

**A define whose value is legitimately absent is declared on the define**, never inferred:

```json
{ "name": "DEBUG_LOG", "from": "debug_log", "optional": true }
```

`optional` is the author saying "build without this one if the sheet does not set it". The
define is then omitted, and its name is **recorded in `omitted_defines`** (§7.5) so the build
record shows the omission and the comparison in §6 still catches an artifact built on the other
side of it. Absence on the author's side is
a decision; absence on the document's side is a mistake, and nothing in the format lets the
second look like the first. `optional` has no effect on the other refusals: an `optional`
define whose parameter *is* present but carries an expression is still refused, because the
author asked for that parameter's value and the value is not available.

### 9. The refusal rule is a property of the format

Written normatively, because an implementation can be re-read and a convenience can be added by
someone being helpful:

> A reader of `.gsim_sch` that resolves a build description **must** refuse to produce a build,
> and must refuse to produce the netlist the build belongs to, when any non-`optional` define
> cannot be resolved to a value. It must not omit the define, substitute a default, substitute
> an empty value, or warn and continue. The refusal must name the define and the parameter it
> would have been derived from.

A second implementation that decided otherwise would produce a block whose constants disagree
with its circuit, which is the failure this entire record is built around. The rule is not this
crate's policy; it is what the format means.

### 10. One step kind now, and the extension point that keeps the rest open

`kind: "shared-library"` is the only kind this version implements. `steps` is a list, every step
has an `id` and an optional `needs`, and the steps are returned topologically ordered — so fact
3's build graph is expressible the day a second kind exists, with no change to anything written
before it.

Two kinds are expected and deliberately **not** specified here, so that they arrive with their
own reasoning rather than as a sketch someone implements:

- a host program compiled and then run to generate a header, which is fact 3's case;
- a generated constants file, which is how a **Python or Octave** block gets the circuit's
  numbers. `-D` is C's mechanism; the problem is not C's. This version builds C blocks only,
  and a Python block needs no compile step — but it has exactly the same constants problem, and
  the format's answer to it is a step that writes the bound defines to a file the script
  imports, not a second mechanism.

**An unrecognised `kind` is refused, not ignored.** This is the one place this record departs
from #79's "preserve the unknown" bias, and the departure is the point of 0008's closing
paragraph: round-tripping is a separate obligation from understanding. An unrecognised *drawing*
key is harmless to carry, because nothing downstream acts on it. An unrecognised *build step*
that is carried and skipped leaves an artifact nobody built, which runs, which is #235. So the
step round-trips verbatim (it must, or saving a sheet in an older editor would delete the user's
build) **and** refuses to compile. Both obligations, separately.

### 11. A document written before this feature

Compiles byte for byte as it does today, and that is testable rather than asserted. Every new
field is optional and `skip_serializing_if`, `code_build` absent means `code_build: None` on the
output, and a `cscript` block with `lib` and no `build` emits the same card it always did. The
existing 0.6.0 round-trip fixture is the control.

The converse — a document *with* a plan opened by a reader that predates this record — is worse
than it looks and is not solved here. An older reader carries `code_build` opaquely in
`Model::extra`, builds nothing, and runs whatever artifact is on disk. That is #235 across
versions. The receipt in §6 is what closes it, and it closes it properly only when the component
that **loads the library** checks it, because that is the only component no front end can
bypass. Filed on `general-simulator` rather than solved here; until then the editor's check is
the one that exists, and it is a front end's check.

## Does this crate interpret the description, or merely carry it?

0008 makes "preserve, do not interpret" the standing stance for anything another layer owns, and
a build is arguably the editor's business: the editor has the compiler, the process, the
diagnostics and the user. The case for carrying the description opaquely is real, and it is
cheap — a flattened catch-all already round-trips it, and nothing here would need to change.

It is rejected on the one thing that cannot be split: **the binding must be resolved by whatever
turns a parameter into the number on a netlist card.** The claim the whole feature makes is that
the `-D` and the card carry the same number. Resolved in the editor, that claim rests on a
second implementation of parameter lookup and of `fmt::stringify`, maintained by hand against
this one — and the two would agree on `1000` and disagree on `2.8e-06` or `1e3`, which is the
only kind of disagreement that matters. 0008's own test applies and gives the same answer it
gave for the flattening pass: a pass every consumer needs, that no consumer can skip, and that
every consumer would have to agree with this crate about, belongs in the one implementation.

So the division is drawn inside the description rather than around it:

- **Interpreted here:** the step graph, the binding of a define to a parameter, the resolved
  value and its formatting, the canonical define list, and every refusal in §8. These are
  statements about the circuit.
- **Not interpreted here, ever:** the compiler, the spelling of a flag — including which of
  `-include` and `/FI` forces the generated header in — the platform's library suffix, the
  process, the include directory `gsim_cscript.h` lives in, and the files the header and the
  build record are written to. This crate says *what* the header contains and *where* it goes
  relative to the schematic; it does not write it. These are statements about a machine. Nothing in this crate names a
  compiler, and `flags` is passed through as opaque strings precisely so that it cannot start
  to.

The machine half already exists, already has evidence behind it, and is already settled.
`omni-schematics`' `apps/vscode-extension/src/toolchain-probe.ts` resolves a compiler in three
steps — a block's own `compiler` parameter, then the `omniSchematics.compilerPath` setting, then
discovery — where discovery tries `clang`, `cc`, `gcc` (and `cl.exe` last, on Windows only) by
*actually building a two-line shared library* with each, rather than looking it up on `PATH`, and
reports the stderr of every failure; a candidate that exists but cannot link comes back
`"cannot-build"`, not `"works"`. **A named compiler is tried alone, with no fallback.** It is a
pure module with injected IO and it names no document concept. That is the right side of the
line, and nothing in this record moves it.

That last rule and §6's receipt are the same argument from two directions, which is worth saying
because the two have to agree. A library built by a compiler the user did not name, with flags
they did not choose, is a library whose provenance nothing records — so a build is refused rather
than silently redirected. The receipt exists for exactly that question one step later, which is
why it carries the compiler it was built with alongside the defines, and why a change there is
as much a mismatch as a changed constant. Neither mechanism is complete without the other: the
no-fallback rule protects the build, and the receipt protects the artifact the build left behind.

That line also says what `flags` costs. `sources`, `includes`, `defines`, `libs` and `lang` are
portable; `flags` is an escape hatch whose contents only some compilers accept, and a sheet that
uses it is a sheet that builds on some machines. Stated here so it is a known price rather than
a surprise.

## Consequences

- **`omni-schematics#253`'s first decision is reversed**, by the owner, on consistency. The
  issue's other two stand: build on Run and never on save, and detect a compiler rather than
  vendor one.
- **`omni-schematics#222` is settled rather than worked around.** `Block::source` is the fact;
  `netlist/code-source.ts`'s four candidate paths become a fallback for documents that do not
  carry one, and the Inspector stops telling the user it is guessing when the document says.
- **A `.gsim_sch` that carries a build is code-adjacent.** It names sources, include
  directories and opaque compiler flags, and something will compile them. That is not a new
  exposure for a workspace the user already trusts, and it is a real one for a sheet from
  elsewhere. Absolute paths and `..` escapes are refused (§8) so a sheet cannot reach outside
  its own tree, and no field anywhere in the description is a command or a shell string. The
  remaining exposure is `flags`, and the component that spawns a compiler is the one that has
  to gate it behind the same trust decision the rest of its tooling uses. Filed on the editor.
- **A sweep is a sequence, not a fan-out.** A swept `model.params` name bound to a define means
  the define's value changes per sweep point, and that falls out of §1 with no extra machinery:
  each point is one `compile()`, so each point has its own resolved define list. What the
  document cannot express is a *distinct artifact path per point* — `lib` is one string — so N
  points are N builds over one artifact, built and run in sequence. Parallel sweep points would
  need a per-point artifact, and that is a separate design.
- **A subsystem's build is out of reach.** `blocks[].subsystem` is opaque (0008 §2), so a
  `build` on a block inside one resolves against nothing. Flattening (#88) is where this
  becomes reachable, and the flattening pass has to decide whether an inner plan merges or
  shadows. Not decided here.
- **This crate gains no dependency**, which §6's choice of a list over a digest is what buys.
- **The include directory for `gsim_cscript.h` cannot be discovered.** `general-simulator`#131/
  PR #132 published the header, and it is the right thing for a generated build to include — but
  there is no `--print-include-dir`, no environment variable and no install step, and the
  published `cscript-ffi` crates on disk carry no `include/` directory at all; the header exists
  only in the unreleased 0.10.0 tree. A build that wants `-I` has to be told where, which is a
  setting, or the front end writes the header beside the source it generates, which is what
  `cscript_ffi::header()` exists for. Filed on `general-simulator`: the binary already answers
  `cscript=load-only` through `--capabilities`, and that is where a `cscript_include` line
  belongs.
- **The editor's own JSON Schema has to change too.** `packages/schemas/definitions/model/`'s
  `Model` is `additionalProperties: false`, and a block parameter's value is restricted to a
  number, a boolean or one of §5.1's two string forms — so neither `model.code_build` nor a
  structured per-block field validates until that schema is extended. That is one of the
  reasons `source` and `build` are typed fields rather than entries in `params`: a nested object
  could not have lived in `params` at all.
- **A compiler error is already clickable, and has no producer.** `code-block-failure.ts`'s
  `readCompilerDiagnostics` parses `$gcc` and MSVC diagnostics and `code-diagnostics.ts` places
  them, both tested, and nothing in the repository currently writes compiler output onto the
  transcript they read. A build step that feeds its stderr into `CodeProblems.report` inherits
  the Problems panel for free. A **refusal** does not: the preflight and staleness messages
  already go into that transcript and produce no diagnostic, because they do not match the
  engine-error grammar. So the refusal needs its own route to `publishFailures`, on the
  `.gsim_sch` with the block named — filed on the editor rather than assumed.
- **The code changes are filed separately** — `Block::source` and `Block::build`, the
  `code_build` schema and its round trip, and the resolution with its refusals — each with its
  own issue in this record's milestone, rather than smuggled in beside it.

[`Block::ratings`]: ../../crates/gsim-core/src/document/block.rs
