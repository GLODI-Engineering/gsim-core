# 0002 — No private/proprietary-derived content in this repo

**Status:** accepted (2026-09-04).

## Context

`gsim-core` is intended for eventual public release. Separately, real-world validation work for
this project family sometimes involves reproducing real, occasionally vendor-proprietary
reference designs in a private, non-public project.

While proving the Phase 2 netlist compiler, one such real design was reproduced via `gsim-py`
and committed into `gsim-core/examples/` as the compiler's acceptance test — genuinely useful
as a real-world proof (it matched a hand-written reference netlist to ~1e-11 absolute
precision), but wrong to have placed in a repo meant to go public, and wrong to have left in
git history even once removed.

## Decision

Nothing derived from a private or non-public source — a full reproduction, an adaptation, or a
result validated against a design that lives there — may exist in `gsim-core`, in any commit,
past or present. This is a hard rule, not a judgment call per case:

- Such reproductions belong in whichever private project they came from, using
  `gsim-core`/`gsim-py`/`gsim-wasm` as an external tool dependency, the same way that project
  already depends on other external simulators/tools.
- `gsim-core`'s own examples/fixtures/acceptance tests must be fully original or
  textbook-generic (a standard H-bridge, a synthetic RC/RLC network, a hand-constructed circuit
  built specifically to exercise one engine behavior) — never modeled on, adapted from, or
  numerically validated against a specific real-world or proprietary design.
- If a violation is ever found already committed, it must be purged from git history
  (`git filter-repo --path <path> --invert-paths`, or equivalent), not merely deleted in a
  follow-up commit — a public repo's history is exactly as visible as its current tree.

## What was done for this specific violation

A reproduction of a real reference design was removed from `gsim-core` via `git filter-repo`
(purged from every commit that touched it, not just deleted going forward — see
`docs/journal/2026-09.md` for the exact commits affected and the resulting hash changes) and
relocated to the private project it came from. The compiler's own correctness proof (~1e-11
agreement with a hand-written reference netlist) is recorded in `gsim-core`'s journal as a fact
established externally, without the reproduction's content living in this repo.
