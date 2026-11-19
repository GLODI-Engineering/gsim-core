# Agent Guidelines

`gsim-core` is the one implementation of what a valid schematic is and what it compiles to,
shared by the schematic editor, the Python automation library and the engine's catalogue. See
`README.md` for scope and `docs/decisions/` for why it exists and what was ruled
out before writing code. This file lists only what mechanical checks cannot enforce.

## Authority order

1. **Mechanical gates** — `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`. If a
   gate fails, fix the cause; never bypass.
2. **This file** — invariants that cannot be mechanized.
3. **`docs/decisions/`** — lasting design rationale; read before revisiting a settled question.

## Project layout

```
crates/gsim-core/   schema types, expression-DSL interpreter, .gsim_sch document model, autolayout
crates/gsim-py/      PyO3 bindings (Python automation library)
crates/gsim-wasm/    wasm-bindgen bindings (the editor's front end)
docs/decisions/      why, not what -- read before changing settled scope
docs/journal/        chronological continuity log, one file per month
```

## Architectural invariants

1. **No `eval`, ever, in any binding.** The `${{ }}` expression DSL is a closed-grammar,
   pure-data interpreter. Component contracts are expected to be loaded from third-party
   libraries; arbitrary code execution in a loaded definition is not an acceptable risk. See
   `docs/decisions/0001-scope-and-naming.md`.
2. **Netlist compilation is now in scope (Phase 2, `crates/gsim-core/src/compile/`)** —
   `general-simulator` is the first real `NetlistTarget`. ngspice/Xyce targets are still
   unstarted; don't assume `general-simulator`'s dual grammar (SPICE-element syntax for
   `physical-network` blocks, `kind=` block-graph syntax for `signal-flow`/`converter` blocks)
   generalizes to them without checking.
3. **Stable port ids.** A generated port's id is index-based and never reassigned when other
   ports are added/removed. Removing a connected port leaves its wire dangling with an error
   marker in the consuming frontend — it must never silently reattach to a different port.
4. **One implementation, thin bindings.** `gsim-py`/`gsim-wasm` must not reimplement schema,
   validation, or layout logic. If a binding needs to duplicate logic to be ergonomic, that
   logic belongs in `gsim-core`, exposed through a cleaner API — not copied.
5. **No half-finished implementations.** This repo starts from an explicit audit of what
   already existed — the editor's own TypeScript and Python implementations — so do not
   re-guess scope that audit already settled; read `docs/decisions/` first.
6. **One netlist name per drawn object, and the map back is output, not a guess.** `element_name`
   prefixes the device letter onto a block id only when the id does not already start with it,
   so `load` and `Rload` both compile to `Rload`. Two objects claiming one name is refused
   (`CompileError::DuplicateName`) because a name is the only handle a rating, a `kind=rule`
   card, a `kind=measure`, a controlling branch and a front end's highlight have on a device.
   The reverse map — block id to netlist name, block port to net — is emitted in
   `CompileOutput.provenance` and crosses both bindings. **Do not re-derive it downstream**: it
   cannot be re-derived, which is the whole point. See `crates/gsim-core/src/compile/provenance.rs`.
7. **Nothing derived from private/internal sources may ever land in this repo — not in a
   commit, not transiently, not as an "example."** This repo is meant for eventual public
   release. Never reproduce, adapt, or validate against a real-world, vendor, or proprietary
   design, or anything sourced from a private, non-public project — even when the reproduction
   is meant as an acceptance test for something in this crate. Work like that belongs in
   whatever private project it came from, using this crate as an external dependency, not the
   other way around. Examples/tests/fixtures in this repo must be fully original or
   textbook-generic (e.g. a standard H-bridge, a synthetic RC/RLC circuit built to exercise one
   specific engine behavior) — never modeled on, adapted from, or validated against a specific
   real-world/proprietary design. If a commit violates this, it must be purged from git history
   (`git filter-repo`), not just deleted in a follow-up commit — see
   `docs/decisions/0002-no-private-content.md`.

## Development cycle

The mechanical gates say how code is written. This says how the work is recorded, which matters for
the same reason: someone reads it later, and what they can reconstruct is what was written down.

1. **A large instruction becomes a milestone**, named for the outcome rather than the task
   ("Rules and diagnostics", not "implement the rules spec").
2. **One issue per deliverable phase or decision**, each stating the problem rather than the
   patch. The instruction as a whole — its reasoning, what it leaves open — is an issue in that
   milestone too, and the rest reference it with `Part of #N`.
3. **A branch and a pull request per issue**, the description ending `Closes #N`, assigned to the
   milestone. Nothing lands on the default branch directly.
4. **The merge closes the issue.** Closing it by hand loses the link between the issue and the
   commit that answered it, which is the whole point of the exercise.
5. **Anything found but not done becomes its own issue**, not a sentence in a pull request body.
   A merged pull request's prose is invisible; an open issue is not. A deliberate omission with
   a reason is worth more than a half-finished corner of a change.
6. **Gates run before pushing**, never after. A red branch spends someone's CI and tells the
   reader the author did not look.

Finish a repository's open pull request before starting new work in it, so review has one thing
to hold at a time.

The mechanical gates in **Authority order** above run before pushing, never after.

## Relationship to sibling repos

Read-only path deps, same convention `general-simulator`/`general-mna` already use — see
`README.md`'s diagram. `gsim-core` has no dependencies on any sibling repo; it is a leaf.

## What NOT to do

- Don't bypass gates with `--no-verify`/`SKIP=`.
- Don't add abstractions without an existing call site.
- Don't duplicate content across `AGENTS.md`, `CLAUDE.md`, and `docs/decisions/`. One source of
  truth per rule.
- Don't claim a task complete if CI/gates are red.
- Don't reproduce, adapt, or validate against any private/real-world/proprietary design in
  this repo — see invariant 6 above.

## Tool-specific notes

- **Claude Code** reads this file plus `CLAUDE.md` (a thin pointer).
- **Other tools**: read this file directly.
