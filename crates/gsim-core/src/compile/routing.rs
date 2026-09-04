//! Goto/From lowering — `docs/specs/gsim-sch-format.md` §10's implicit-connection mechanism,
//! resolved at compile time so the emitted netlist contains neither block.
//!
//! A `routing.goto` is a **signal sink**: it broadcasts whatever drives its `in` port under a
//! string `tag`. A `routing.from` is a **signal source** with no wire input at all: it resolves
//! to the `routing.goto` sharing its tag and then behaves exactly as if it had been wired
//! straight to *that* Goto's own driver. Neither block is a device, so both are registered
//! `structural` in [`super::spec`] and skipped by the emitter; the only thing lowering has to
//! produce is a redirect table that [`super::Scope::ref_name`] consults when a `field=`
//! reference lands on a `From`.
//!
//! # Visibility: a deliberate, documented scope limitation
//!
//! §10.1 gives a Goto three visibilities — `local` (same subsystem only), `scoped` (that
//! subsystem and its descendants) and `global` (anywhere in the model). All three are
//! *accepted* here, but `gsim-core`'s document model has **no subsystem hierarchy**: a
//! [`crate::document::Block`] has no parent, and [`crate::document::ModelBuilder`] has no way to
//! author one (`hierarchy.subsystem.*` exists in the spec's catalogue and nowhere in this
//! crate). Every block in a scope therefore has the same — absent — parent, which makes all
//! three visibilities resolve identically: `local`'s "same subsystem" test
//! (`gotoParent === fromParent` in the frontend's own `goto-from-resolver.ts`) is trivially
//! true for every pair, and so is `scoped`'s ancestor test. Implementing the distinction now
//! would mean building subsystem nesting as a side effect of this pass, which is explicitly out
//! of scope.
//!
//! So: one flat tag namespace per compile scope (the top-level model, or one `SubcktDef` body —
//! a def's body is a real independent namespace, which is the *one* scoping distinction this
//! crate can already make honestly). Revisit this the day `Block` gains a parent; the
//! resolution shape here is deliberately the same as the frontend resolver's so that adding a
//! `visible_to` predicate is the only change needed.
//!
//! # Tag namespace per port-kind
//!
//! §10.2: tags are partitioned by port-kind, so a `signal` tag `"CLK"` and a `physical-signal`
//! tag `"CLK"` do not collide — and **acausal kinds cannot use Goto/From at all**, they must go
//! through named nets (§9). Both rules are enforced by [`port_kind_of`] +
//! [`is_routable_kind`].

use super::CompileError;
use crate::document::{Block, Wire};
use std::collections::{BTreeMap, BTreeSet};

pub const GOTO_TYPE: &str = "routing.goto";
pub const FROM_TYPE: &str = "routing.from";

/// The Goto's single input port id, per §10's own example blocks.
pub const GOTO_IN_PORT: &str = "in";
/// The From's single output port id.
pub const FROM_OUT_PORT: &str = "out";

/// Port kinds a Goto/From pair may carry: the causal ones. Everything else — the acausal
/// `physical-*` conserving kinds, `mixed`, and the `reserved.*` placeholders — is rejected,
/// per §10.2's "acausal kinds cannot be routed via Goto/From".
///
/// Note `physical-signal` *is* here: it is causal ("Sensor output / actuator input" in §8's
/// catalogue), and §10.2 names it explicitly as a second, independent tag namespace.
const ROUTABLE_KINDS: &[&str] = &["signal", "signal-bus", "function-call", "physical-signal"];

pub fn is_routable_kind(kind: &str) -> bool {
    ROUTABLE_KINDS.contains(&kind)
}

pub fn is_goto(block: &Block) -> bool {
    block.r#type == GOTO_TYPE
}

pub fn is_from(block: &Block) -> bool {
    block.r#type == FROM_TYPE
}

/// The `tag` parameter, trimmed. An absent or blank tag resolves to nothing, exactly as the
/// frontend resolver treats it (`tagOf` there returns `""`, which matches no Goto).
fn tag_of(block: &Block) -> String {
    block
        .params
        .get("tag")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// The port-kind a routing block carries.
///
/// Preferred source is the `kind` of a wire actually touching the block, which §8.4 defines as
/// duplicating both endpoints' port-kind — that is the authoritative value and the one the
/// loader validates. A block with no wire at all (an unused `From`, a stub `Goto`) falls back to
/// its own `domain` field, which §6 defines as the block's primary port-kind and which §10's
/// example blocks set to `"signal"`.
pub fn port_kind_of(block: &Block, wires: &[Wire]) -> String {
    wires
        .iter()
        .find(|w| w.from.block == block.id || w.to.block == block.id)
        .map(|w| w.kind.clone())
        .unwrap_or_else(|| block.domain.clone())
}

/// Resolves every `routing.from` in one scope to the single `routing.goto` it binds to.
///
/// Returns a `from block id -> goto block id` map. Errors are hard, per §10.3 ("Every `From`
/// must resolve to exactly one `Goto`. Unresolved or ambiguous `From`s are hard errors"); a
/// `Goto` nobody reads is fine and produces no entry at all.
pub fn resolve_scope(
    blocks: &[Block],
    wires: &[Wire],
) -> Result<BTreeMap<String, String>, CompileError> {
    // Kind check first, over *every* routing block — a Goto wired into an acausal network is
    // wrong whether or not any From reads it, and saying so here gives the clearer error.
    for block in blocks.iter().filter(|b| is_goto(b) || is_from(b)) {
        let kind = port_kind_of(block, wires);
        if !is_routable_kind(&kind) {
            return Err(CompileError::RoutingKindUnsupported {
                block: block.id.clone(),
                type_name: block.r#type.clone(),
                kind,
            });
        }
    }

    let gotos: Vec<(&Block, String, String)> = blocks
        .iter()
        .filter(|b| is_goto(b))
        .map(|b| (b, tag_of(b), port_kind_of(b, wires)))
        .collect();

    let mut resolved = BTreeMap::new();
    for from in blocks.iter().filter(|b| is_from(b)) {
        let tag = tag_of(from);
        let kind = port_kind_of(from, wires);
        // Tags are partitioned by port-kind (§10.2) — same string, different kind, no collision.
        let candidates: Vec<&Block> = gotos
            .iter()
            .filter(|(_, g_tag, g_kind)| !g_tag.is_empty() && *g_tag == tag && *g_kind == kind)
            .map(|(b, _, _)| *b)
            .collect();
        match candidates.len() {
            0 => {
                return Err(CompileError::UnresolvedFromTag {
                    block: from.id.clone(),
                    tag,
                })
            }
            1 => {
                resolved.insert(from.id.clone(), candidates[0].id.clone());
            }
            _ => {
                return Err(CompileError::AmbiguousFromTag {
                    block: from.id.clone(),
                    tag,
                    candidates: candidates.iter().map(|b| b.id.clone()).collect(),
                })
            }
        }
    }
    Ok(resolved)
}

/// Guards the chase in [`super::build_scope`] against a `Goto` whose own input is (transitively)
/// driven by a `From` bound back to it.
pub struct Chase {
    seen: BTreeSet<String>,
}

impl Chase {
    pub fn new() -> Chase {
        Chase {
            seen: BTreeSet::new(),
        }
    }

    /// Records one hop, erroring if this `From` has already been visited on this chase.
    pub fn visit(&mut self, from: &Block) -> Result<(), CompileError> {
        if !self.seen.insert(from.id.clone()) {
            return Err(CompileError::CyclicFromTag {
                block: from.id.clone(),
                tag: tag_of(from),
            });
        }
        Ok(())
    }
}

impl Default for Chase {
    fn default() -> Chase {
        Chase::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Endpoint, Mirror, NewBlock, Paradigm};
    use serde_json::{Map, Value};

    fn routing_block(id: &str, type_name: &str, tag: &str, domain: &str) -> Block {
        let mut params = Map::new();
        params.insert("tag".to_string(), Value::String(tag.to_string()));
        Block::new(NewBlock {
            id: id.to_string(),
            r#type: type_name.to_string(),
            paradigm: Paradigm::Routing,
            domain: domain.to_string(),
            fidelity: Some("n_a".to_string()),
            position: (0, 0),
            rotation: 0,
            mirror: Mirror::None,
            params,
            ic: Map::new(),
            label: None,
            comment_before: Vec::new(),
            comment_after: Vec::new(),
            subckt_ref: None,
        })
        .unwrap()
    }

    fn wire(id: &str, from: (&str, &str), to: (&str, &str), kind: &str) -> Wire {
        Wire {
            id: id.to_string(),
            from: Endpoint::new(from.0, from.1),
            to: Endpoint::new(to.0, to.1),
            kind: kind.to_string(),
            waypoints: Vec::new(),
            pinned: false,
            net: None,
            label: None,
        }
    }

    #[test]
    fn resolves_a_single_matching_pair() {
        let blocks = vec![
            routing_block("G1", GOTO_TYPE, "CLK", "signal"),
            routing_block("F1", FROM_TYPE, "CLK", "signal"),
        ];
        let resolved = resolve_scope(&blocks, &[]).unwrap();
        assert_eq!(resolved.get("F1").map(String::as_str), Some("G1"));
        // A Goto nobody reads produces no entry -- and is not an error (§10.3).
        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn an_unread_goto_is_not_an_error() {
        let blocks = vec![routing_block("G1", GOTO_TYPE, "CLK", "signal")];
        assert!(resolve_scope(&blocks, &[]).unwrap().is_empty());
    }

    #[test]
    fn an_unmatched_tag_is_a_hard_error() {
        let blocks = vec![
            routing_block("G1", GOTO_TYPE, "CLK", "signal"),
            routing_block("F1", FROM_TYPE, "ENABLE", "signal"),
        ];
        assert_eq!(
            resolve_scope(&blocks, &[]),
            Err(CompileError::UnresolvedFromTag {
                block: "F1".to_string(),
                tag: "ENABLE".to_string(),
            })
        );
    }

    #[test]
    fn a_blank_tag_never_matches_even_another_blank_one() {
        let blocks = vec![
            routing_block("G1", GOTO_TYPE, "  ", "signal"),
            routing_block("F1", FROM_TYPE, "", "signal"),
        ];
        assert!(matches!(
            resolve_scope(&blocks, &[]),
            Err(CompileError::UnresolvedFromTag { .. })
        ));
    }

    #[test]
    fn two_gotos_on_one_tag_are_ambiguous() {
        let blocks = vec![
            routing_block("G1", GOTO_TYPE, "CLK", "signal"),
            routing_block("G2", GOTO_TYPE, "CLK", "signal"),
            routing_block("F1", FROM_TYPE, "CLK", "signal"),
        ];
        assert_eq!(
            resolve_scope(&blocks, &[]),
            Err(CompileError::AmbiguousFromTag {
                block: "F1".to_string(),
                tag: "CLK".to_string(),
                candidates: vec!["G1".to_string(), "G2".to_string()],
            })
        );
    }

    #[test]
    fn the_same_tag_string_in_two_port_kinds_does_not_collide() {
        // §10.2: `signal` "CLK" and `physical-signal` "CLK" are independent namespaces, so this
        // is two clean pairs, not one ambiguity.
        let blocks = vec![
            routing_block("Gs", GOTO_TYPE, "CLK", "signal"),
            routing_block("Fs", FROM_TYPE, "CLK", "signal"),
            routing_block("Gp", GOTO_TYPE, "CLK", "physical-signal"),
            routing_block("Fp", FROM_TYPE, "CLK", "physical-signal"),
        ];
        let resolved = resolve_scope(&blocks, &[]).unwrap();
        assert_eq!(resolved.get("Fs").map(String::as_str), Some("Gs"));
        assert_eq!(resolved.get("Fp").map(String::as_str), Some("Gp"));
    }

    #[test]
    fn a_wire_kind_outranks_the_blocks_own_domain_when_partitioning_tags() {
        // The block's `domain` says `signal`, but §8.4 makes the wire's own `kind` the
        // authoritative port-kind -- so this pair is a `physical-signal` pair and must not bind
        // to the `signal` Goto carrying the same tag.
        let blocks = vec![
            routing_block("Gs", GOTO_TYPE, "CLK", "signal"),
            routing_block("Gp", GOTO_TYPE, "CLK", "signal"),
            routing_block("F1", FROM_TYPE, "CLK", "signal"),
        ];
        let wires = vec![
            wire(
                "w1",
                ("SRC", "out"),
                ("Gp", GOTO_IN_PORT),
                "physical-signal",
            ),
            wire(
                "w2",
                ("F1", FROM_OUT_PORT),
                ("SINK", "in"),
                "physical-signal",
            ),
        ];
        let resolved = resolve_scope(&blocks, &wires).unwrap();
        assert_eq!(resolved.get("F1").map(String::as_str), Some("Gp"));
    }

    #[test]
    fn an_acausal_kind_is_rejected_outright() {
        let blocks = vec![routing_block(
            "G1",
            GOTO_TYPE,
            "RAIL",
            "physical-electrical",
        )];
        assert_eq!(
            resolve_scope(&blocks, &[]),
            Err(CompileError::RoutingKindUnsupported {
                block: "G1".to_string(),
                type_name: GOTO_TYPE.to_string(),
                kind: "physical-electrical".to_string(),
            })
        );
    }

    #[test]
    fn an_acausal_wire_kind_is_rejected_even_when_the_domain_looks_causal() {
        let blocks = vec![routing_block("G1", GOTO_TYPE, "RAIL", "signal")];
        let wires = vec![wire(
            "w1",
            ("N1", "p1"),
            ("G1", GOTO_IN_PORT),
            "physical-electrical",
        )];
        assert!(matches!(
            resolve_scope(&blocks, &wires),
            Err(CompileError::RoutingKindUnsupported { .. })
        ));
    }
}
