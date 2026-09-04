//! Port-kind compatibility for [`super::ModelBuilder::connect`] — the finer-grained sibling of
//! the block-level cross-paradigm check (`docs/specs/gsim-sch-format.md` §8.1/§8.2, where §8.3's
//! paradigm boundary is the coarse rule [`super::Paradigm::bridges_paradigms`] already covers).
//!
//! # What this is ported from
//!
//! The reference implementation is `general-simulator-gui`'s
//! `apps/frontend/src/canvas/port-compat.ts` (`canConnect(src, dst)`), read in full before
//! writing this — not the spec prose, which has diverged from real code in this project before.
//! Its four rules, in its own order, are `reserved-kind`, `kind-mismatch`, `fan-in-on-signal`
//! and `dim-mismatch`. (It has **no** `direction-mismatch` rule; §8.2's direction wording is
//! realised there purely as the fan-in check.)
//!
//! `canConnect` takes two `PortDesc`s the canvas has already looked up from each block's
//! evaluated GSF v2 symbol, so it always knows both endpoints' declared `kind` and their
//! resolved signal dimensions. This module reconstructs those two `PortDesc`s from the
//! *document* — see [`PortFacts`] — and there are now three sources for them, tried in that
//! order of authority:
//!
//! 1. **A [`super::SubcktDef`]'s `ports[].kind`**, for a `spice-subckt` instance. A genuine
//!    per-port catalogue that is already inside the document; exact, and needs nothing external.
//! 2. **A [`crate::contract::ContractRegistry`] entry for the block's `type`**, evaluated
//!    against that block's own `params` via [`crate::contract::evaluate_component`]. This is the
//!    same data `canConnect`'s caller uses, so an endpoint resolved this way is checked exactly
//!    as the canvas checks it — kind *and* `dim`.
//! 3. **The block's `domain`**, the pre-registry fallback, deliberately kept.
//!
//! | `port-compat.ts` rule | here | source of truth |
//! |---|---|---|
//! | `reserved-kind` | **full parity** | the wire's own `kind` (§8.4 makes it the endpoints' kind), or either endpoint's declared kind |
//! | `kind-mismatch` | **full parity when a contract or def is available**, else the `domain` fallback below | (1), (2) or (3) above |
//! | `fan-in-on-signal` | **full parity, computed** | the model's own wire list — better than the GUI's caller-supplied `driven` flag |
//! | `dim-mismatch` | **full parity when both endpoints' dims resolve** | (2) above — `Port::dim`, resolved per block's `params` |
//!
//! ## Why the `domain` fallback stays, and what it can and cannot say
//!
//! A block with no registered contract and no `SubcktDef` offers only `domain`, which §6 defines
//! as the block's *primary* port-kind (the reading `crate::compile::routing::port_kind_of`
//! already takes). It is a per-block value, not a per-port one, and real documents populate it
//! loosely — a `signal-flow` block carrying `domain: "electrical"` is common in this repo's own
//! examples. Enforcing "wire kind must equal domain" would reject valid models. So only the
//! universal incompatibility no catalogue could ever override is rejected from `domain` alone: a
//! conserving wire crossing between two different physical domains, e.g. a `physical-thermal`
//! wire landing on an `electrical`-domain block — see [`physical_domain_of`].
//!
//! Registering a contract *upgrades* an endpoint from that coarse rule to the exact one; it
//! never downgrades one. A model built with an empty registry behaves exactly as it did before
//! the registry existed, which is the property [`check`]'s tests pin down.
//!
//! ## What is still not checked, on purpose
//!
//! - **Port existence.** An endpoint naming a port that neither the def nor the contract
//!   declares resolves to "nothing known" and is skipped, never turned into an error —
//!   preserving the property [`super::ModelBuilder::instantiate_subckt`] documents.
//! - **`direction`.** `canConnect` does not check it either (only `Canvas.tsx` uses `direction`,
//!   and only to decide which end of a drag is the source); §8.2's direction wording is realised
//!   as the fan-in rule, which is implemented.
//! - **Dimension *propagation*.** `canConnect` compares two already-resolved dims and nothing
//!   more; inferring an unannotated port's dim from what drives it is a separate subsystem in
//!   the reference (`apps/frontend/src/canvas/dim-propagate.ts`, with its own `inherit`/`from`/
//!   `sum`/`partition` signature language and loop detection), not part of the rule ported here.
//!   A port whose contract declares no `dim` is therefore "unknown", not "scalar".

use super::block::Block;
use super::model::Model;
use super::wire::Endpoint;
use crate::contract::ContractRegistry;

/// §8's `reserved.*` kinds: catalogued for v3, but "block instances using them are rejected by
/// the loader in v2" (§8.1). Same constant `port-compat.ts` uses.
pub const RESERVED_KIND_PREFIX: &str = "reserved.";

/// Kinds with no causality: both endpoints participate in one conservation law (§8.2), so any
/// number of wires may meet at one port and the fan-in rule does not apply.
///
/// Mirrors `port-compat.ts`'s `ACAUSAL_KINDS` verbatim, bare short names included — a document
/// may spell an electrical port either way, and `default_subckt_domain` in the builder already
/// produces the short form. Deliberately *not* `compile::nets::is_physical_kind`, whose
/// `physical-` prefix test would wrongly class `physical-signal` (causal per §8) as acausal.
const ACAUSAL_KINDS: &[&str] = &[
    "physical-electrical",
    "physical-translational",
    "physical-rotational",
    "physical-thermal",
    "electrical",
    "translational",
    "rotational",
    "thermal",
];

pub fn is_acausal(kind: &str) -> bool {
    ACAUSAL_KINDS.contains(&kind)
}

/// The physical domain a kind *or* a block `domain` names, in short form, or `None` when it is
/// not one of §8's four conserving domains.
///
/// Both spellings collapse here: the wire kind `physical-thermal` and the block domain
/// `"thermal"` both yield `Some("thermal")`. Anything outside the closed set — `signal`,
/// `mixed`, `physical-signal`, a future kind, or a domain a caller uses loosely — yields `None`,
/// which makes the cross-domain rule skip rather than guess.
pub fn physical_domain_of(value: &str) -> Option<&'static str> {
    match value.strip_prefix("physical-").unwrap_or(value) {
        "electrical" => Some("electrical"),
        "translational" => Some("translational"),
        "rotational" => Some("rotational"),
        "thermal" => Some("thermal"),
        _ => None,
    }
}

/// Why a connection was refused. One-to-one with the `reason` strings `port-compat.ts` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatFailure {
    /// A `reserved.*` kind — catalogued for a future schema version, not connectable in v2.
    ReservedKind { kind: String },
    /// The wire's declared kind contradicts the kind `at` actually declares.
    KindMismatch { at: Endpoint, declared: String },
    /// A causal port cannot be driven twice; `existing_wire` already drives it.
    FanInOnSignal { at: Endpoint, existing_wire: String },
    /// Both endpoints resolved a signal dimension and they are not the same shape.
    ///
    /// `expected`/`actual` keep `canConnect`'s own field naming: `expected` is the *source*
    /// endpoint's dim (what the wire will carry), `actual` the *sink*'s. Neither endpoint is
    /// repeated here -- [`PortIncompat`] already carries both, and duplicating them would make
    /// this the largest variant in the enum for no added information.
    DimMismatch {
        expected: Vec<i64>,
        actual: Vec<i64>,
    },
}

impl CompatFailure {
    /// The stable discriminator for this failure — **byte-identical to the `reason` string
    /// `port-compat.ts`'s `canConnect` returns for the same case**, so a frontend already
    /// branching on those strings needs no new vocabulary. It appears in
    /// [`super::BuilderError`]'s `Display` output (hence in `gsim-wasm`'s thrown `Error`
    /// message) and is what `gsim-py`'s `builder_err_to_py` builds its keyword from.
    pub fn reason(&self) -> &'static str {
        match self {
            CompatFailure::ReservedKind { .. } => "reserved-kind",
            CompatFailure::KindMismatch { .. } => "kind-mismatch",
            CompatFailure::FanInOnSignal { .. } => "fan-in-on-signal",
            CompatFailure::DimMismatch { .. } => "dim-mismatch",
        }
    }
}

/// `[3]` / `[2, 3]`, matching how `port-compat.ts`'s `dimMismatchTooltip` prints a dim.
fn fmt_dim(dim: &[i64]) -> String {
    let inner: Vec<String> = dim.iter().map(i64::to_string).collect();
    format!("[{}]", inner.join(", "))
}

/// The full context of a refused connection, as carried by
/// [`super::BuilderError::PortIncompatible`].
///
/// A separate boxed struct rather than inline enum fields purely for size: four owned `String`s
/// plus two [`Endpoint`]s would make `BuilderError` ~200 bytes, so every `Result<_, BuilderError>`
/// in the builder would pay for the rarest variant (`clippy::result_large_err`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortIncompat {
    pub from: Endpoint,
    pub to: Endpoint,
    /// The kind the caller declared for the wire (§8.4).
    pub kind: String,
    pub failure: CompatFailure,
}

impl std::fmt::Display for PortIncompat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot connect '{}.{}' to '{}.{}' as a '{}' wire: {} -- {}",
            self.from.block,
            self.from.port,
            self.to.block,
            self.to.port,
            self.kind,
            self.failure.reason(),
            self.failure
        )
    }
}

impl std::fmt::Display for CompatFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompatFailure::ReservedKind { kind } => write!(
                f,
                "port kind '{kind}' is reserved for a future schema version and cannot be wired in v2"
            ),
            CompatFailure::KindMismatch { at, declared } => write!(
                f,
                "port '{}.{}' is declared '{declared}'",
                at.block, at.port
            ),
            CompatFailure::FanInOnSignal { at, existing_wire } => write!(
                f,
                "causal port '{}.{}' is already driven by wire '{existing_wire}' (fan-in is forbidden on a causal kind; converge with a routing.merge block instead)",
                at.block, at.port
            ),
            CompatFailure::DimMismatch { expected, actual } => write!(
                f,
                "the source produces dim {}, but the destination requires dim {}",
                fmt_dim(expected),
                fmt_dim(actual)
            ),
        }
    }
}

/// What this document can say about one endpoint — the `PortDesc` `canConnect` is handed,
/// reconstructed from whatever source of truth is actually available.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PortFacts {
    /// The kind this port declares, or `None` when nothing knows.
    pub kind: Option<String>,
    /// Whether [`PortFacts::kind`] is a genuine *per-port* declaration (a `SubcktDef` port or an
    /// evaluated contract port) rather than the block-wide `domain` approximation.
    ///
    /// This is the flag that keeps the fallback honest: an exact kind may be compared literally,
    /// a `domain`-derived one only across physical domains.
    pub exact: bool,
    /// The resolved signal dimension, from a contract's `Port::dim`. `None` means *unknown*, and
    /// makes the dim rule skip this endpoint — never "assume scalar".
    pub dim: Option<Vec<i64>>,
}

/// Resolves one endpoint against, in order: its `SubcktDef`'s port list, a registered contract
/// for its block `type`, then its block `domain`. Every source may decline, and declining is
/// always "nothing known" rather than an error — see the module doc.
pub fn facts_of(
    model: &Model,
    registry: &ContractRegistry,
    block: &Block,
    endpoint: &Endpoint,
) -> PortFacts {
    if let Some(def_ref) = &block.subckt_ref {
        // The def's port list is a genuine per-port catalogue, already in this document. It wins
        // over a contract because a subckt instance's `type` is the generic
        // `SUBCKT_INSTANCE_TYPE` -- any contract registered under that name would describe some
        // other def entirely.
        //
        // An endpoint naming a port the def does not declare stays unchecked on purpose --
        // `connect()` has never validated port existence and this must not start doing so.
        let kind = model
            .subckt_defs
            .iter()
            .find(|d| d.id == *def_ref)
            .and_then(|def| def.ports.iter().find(|p| p.id == endpoint.port))
            .map(|p| p.kind.clone());
        return PortFacts {
            exact: kind.is_some(),
            kind,
            dim: None,
        };
    }

    // A registered contract is the same data `canConnect`'s own caller looks up, evaluated
    // against this block's own params -- so an endpoint resolved here is checked exactly as the
    // canvas checks it, `dim` included. This is what closes `signal` vs `physical-electrical`,
    // `signal` vs `signal-bus`, and `dim-mismatch` for ordinary components.
    if let Some(port) = registry.port_of(&block.r#type, &block.params, &endpoint.port) {
        return PortFacts {
            kind: Some(port.kind),
            exact: true,
            dim: port.dim,
        };
    }

    // Fallback, unchanged from before the registry existed. A bridging block
    // (converter/routing/hierarchy/annotation) legitimately has ports of different kinds on
    // either side, and `domain` names only one of them -- it cannot be used to judge a
    // particular port, so say nothing.
    if block.paradigm.bridges_paradigms() {
        return PortFacts::default();
    }
    PortFacts {
        kind: physical_domain_of(&block.domain).map(str::to_string),
        exact: false,
        dim: None,
    }
}

/// Runs the checks in `port-compat.ts`'s own order: reserved kind, kind mismatch, fan-in, then
/// dim mismatch. `Ok(())` means the connection is compatible as far as the document plus
/// `registry` can tell; an empty registry reproduces the pre-registry behaviour exactly.
pub fn check(
    model: &Model,
    registry: &ContractRegistry,
    from_block: &Block,
    to_block: &Block,
    from: &Endpoint,
    to: &Endpoint,
    kind: &str,
) -> Result<(), CompatFailure> {
    // 1. reserved kind -- on the wire, or on either endpoint's own declaration.
    if kind.starts_with(RESERVED_KIND_PREFIX) {
        return Err(CompatFailure::ReservedKind {
            kind: kind.to_string(),
        });
    }

    let from_facts = facts_of(model, registry, from_block, from);
    let to_facts = facts_of(model, registry, to_block, to);

    // 2. kind mismatch, against whatever each endpoint genuinely declares.
    //
    // `canConnect` compares `src.kind` to `dst.kind` directly; here each endpoint is compared to
    // the wire's declared kind instead, which §8.4 makes the endpoints' kind. Two endpoints that
    // each agree with the wire agree with each other, so the two formulations coincide -- and
    // this one additionally catches a wire whose *own* declared kind is wrong, which a
    // caller-supplied `kind: impl Into<String>` makes possible and the canvas cannot express.
    for (facts, endpoint) in [(&from_facts, from), (&to_facts, to)] {
        let Some(declared) = facts.kind.as_deref() else {
            continue;
        };
        if declared.starts_with(RESERVED_KIND_PREFIX) {
            return Err(CompatFailure::ReservedKind {
                kind: declared.to_string(),
            });
        }
        // `physical_domain_of` collapses both spellings, so `physical-electrical` against a
        // declared `electrical` agrees, and against `thermal` does not -- this arm is the only
        // one an inexact, `domain`-derived declaration is ever allowed to reach.
        let conflicting = match (physical_domain_of(kind), physical_domain_of(declared)) {
            (Some(wire_domain), Some(port_domain)) => wire_domain != port_domain,
            // At least one side is not a conserving domain (`signal`, `signal-bus`, `mixed`,
            // ...). Only a genuine per-port declaration may be judged here, and then literally
            // -- which is exactly `canConnect`'s own `src.kind !== dst.kind`.
            _ => facts.exact && declared != kind,
        };
        if conflicting {
            return Err(CompatFailure::KindMismatch {
                at: endpoint.clone(),
                declared: declared.to_string(),
            });
        }
    }

    // 3. fan-in on a causal port. Computed from the model's real wire list rather than a
    // caller-supplied flag; `to` is the driven side, matching `connect()`'s from->to
    // source->sink convention (the same one `compile::routing` relies on for Goto/From).
    if !is_acausal(kind) {
        if let Some(existing) = model.wires.iter().find(|w| w.to == *to) {
            return Err(CompatFailure::FanInOnSignal {
                at: to.clone(),
                existing_wire: existing.id.clone(),
            });
        }
    }

    // 4. dim mismatch -- only when *both* sides resolved a dimension, exactly as `canConnect`
    // guards on `src.dim !== undefined && dst.dim !== undefined`. Comparison is elementwise
    // equality with no broadcasting and no scalar special case, which is all `dimsEqual` does;
    // inferring a missing dim is `dim-propagate.ts`'s job, not this rule's.
    if let (Some(expected), Some(actual)) = (&from_facts.dim, &to_facts.dim) {
        if expected != actual {
            return Err(CompatFailure::DimMismatch {
                expected: expected.clone(),
                actual: actual.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::builder::{
        ConnectOptions, ModelBuilder, NewBlockSpec, NewSubcktInstanceSpec,
    };
    use super::super::paradigm::Paradigm;
    use super::super::subckt::{SubcktDef, SubcktPort};
    use super::super::BuilderError;
    use super::*;
    use crate::contract::ComponentContract;
    use serde_json::json;

    /// Exercises the rules through the real `connect()` entry point rather than calling `check`
    /// directly -- the wiring into the builder is part of what is under test.
    fn builder() -> ModelBuilder {
        ModelBuilder::new("m", "M", 10)
    }

    fn add(b: &mut ModelBuilder, id: &str, paradigm: Paradigm, r#type: &str, domain: &str) {
        b.add_block(NewBlockSpec {
            r#type: r#type.to_string(),
            paradigm,
            domain: domain.to_string(),
            id: Some(id.to_string()),
            ..Default::default()
        })
        .unwrap();
    }

    fn connect(
        b: &mut ModelBuilder,
        from: (&str, &str),
        to: (&str, &str),
        kind: &str,
    ) -> Result<String, BuilderError> {
        b.connect(
            Endpoint::new(from.0, from.1),
            Endpoint::new(to.0, to.1),
            kind,
            ConnectOptions::default(),
        )
        .map(|id| id.to_string())
    }

    fn reason(err: &BuilderError) -> &'static str {
        match err {
            BuilderError::PortIncompatible(inc) => inc.failure.reason(),
            other => panic!("expected PortIncompatible, got {other:?}"),
        }
    }

    // -- reserved-kind -------------------------------------------------------

    #[test]
    fn a_non_reserved_kind_connects() {
        let mut b = builder();
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        add(
            &mut b,
            "r2",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        connect(&mut b, ("r1", "p"), ("r2", "p"), "physical-electrical").unwrap();
        assert_eq!(b.model().wires.len(), 1);
    }

    #[test]
    fn a_reserved_kind_is_refused_and_names_the_kind() {
        let mut b = builder();
        add(
            &mut b,
            "m1",
            Paradigm::PhysicalNetwork,
            "magnetic.core",
            "mixed",
        );
        add(
            &mut b,
            "m2",
            Paradigm::PhysicalNetwork,
            "magnetic.core",
            "mixed",
        );
        let err = connect(
            &mut b,
            ("m1", "p"),
            ("m2", "p"),
            "reserved.physical-magnetic",
        )
        .unwrap_err();
        assert_eq!(reason(&err), "reserved-kind");
        assert!(
            err.to_string().contains("reserved.physical-magnetic")
                && err
                    .to_string()
                    .contains("reserved for a future schema version"),
            "{err}"
        );
        assert!(b.model().wires.is_empty(), "the wire must not be appended");
    }

    // -- kind-mismatch, SubcktDef-backed (exact, per-port) --------------------

    fn mixed_def() -> SubcktDef {
        SubcktDef::new("leaf", "LEAF").unwrap().with_ports([
            SubcktPort::new("d", "physical-electrical").unwrap(),
            SubcktPort::new("ctrl", "signal").unwrap(),
        ])
    }

    #[test]
    fn a_subckt_port_accepts_a_wire_of_its_own_declared_kind() {
        let mut b = builder();
        b.add_subckt_def(mixed_def()).unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("x1"))
            .unwrap();
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        add(&mut b, "c1", Paradigm::SignalFlow, "source.const", "signal");
        connect(&mut b, ("r1", "p"), ("x1", "d"), "physical-electrical").unwrap();
        connect(&mut b, ("c1", "out"), ("x1", "ctrl"), "signal").unwrap();
        assert_eq!(b.model().wires.len(), 2);
    }

    #[test]
    fn a_subckt_port_refuses_a_wire_of_a_different_declared_kind() {
        let mut b = builder();
        b.add_subckt_def(mixed_def()).unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("x1"))
            .unwrap();
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        let err = connect(&mut b, ("r1", "p"), ("x1", "ctrl"), "physical-electrical").unwrap_err();
        assert_eq!(reason(&err), "kind-mismatch");
        // The message must name the offending port *and* both kinds involved.
        let msg = err.to_string();
        assert!(msg.contains("'x1.ctrl' is declared 'signal'"), "{msg}");
        assert!(msg.contains("as a 'physical-electrical' wire"), "{msg}");
    }

    #[test]
    fn an_endpoint_naming_a_port_the_def_does_not_declare_stays_unchecked() {
        // connect() has never validated port existence (see instantiate_subckt's doc); an
        // unknown port must resolve to "no declared kind" and be skipped, not become an error.
        let mut b = builder();
        b.add_subckt_def(mixed_def()).unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("x1"))
            .unwrap();
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        connect(
            &mut b,
            ("r1", "p"),
            ("x1", "nosuchport"),
            "physical-electrical",
        )
        .unwrap();
    }

    // -- kind-mismatch, domain-backed (cross-physical-domain only) -----------

    #[test]
    fn a_conserving_wire_matching_the_blocks_domain_connects_in_either_spelling() {
        let mut b = builder();
        // `domain` uses the short spelling, the wire the `physical-` one -- these agree.
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        add(
            &mut b,
            "r2",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "physical-electrical",
        );
        connect(&mut b, ("r1", "p"), ("r2", "p"), "physical-electrical").unwrap();
    }

    #[test]
    fn two_different_physical_domains_cannot_be_wired_directly() {
        let mut b = builder();
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        add(
            &mut b,
            "rth",
            Paradigm::PhysicalNetwork,
            "thermal.resistance",
            "thermal",
        );
        let err = connect(&mut b, ("r1", "p"), ("rth", "a"), "physical-electrical").unwrap_err();
        assert_eq!(reason(&err), "kind-mismatch");
        assert!(
            err.to_string().contains("'rth.a' is declared 'thermal'"),
            "{err}"
        );
    }

    #[test]
    fn a_loosely_populated_domain_never_causes_a_false_rejection() {
        // Real documents give signal-flow blocks a `domain` of "electrical"; a `signal` wire is
        // not a conserving kind, so the domain rule must not fire. This is the deliberate
        // limitation the module doc records -- a genuine signal/physical port confusion on a
        // non-subckt block still needs a contract registry to catch.
        let mut b = builder();
        add(
            &mut b,
            "g1",
            Paradigm::SignalFlow,
            "control.gain",
            "electrical",
        );
        add(
            &mut b,
            "s1",
            Paradigm::SignalFlow,
            "sinks.scope",
            "electrical",
        );
        connect(&mut b, ("g1", "out"), ("s1", "in"), "signal").unwrap();
    }

    #[test]
    fn a_bridging_blocks_domain_never_judges_a_port() {
        // A converter genuinely has different kinds on its two sides; `domain` names only one,
        // so it must not be used to reject either.
        let mut b = builder();
        add(
            &mut b,
            "conv1",
            Paradigm::Converter,
            "converter.sig2phys",
            "electrical",
        );
        add(
            &mut b,
            "th1",
            Paradigm::PhysicalNetwork,
            "thermal.mass",
            "thermal",
        );
        connect(&mut b, ("conv1", "out"), ("th1", "a"), "physical-thermal").unwrap();
    }

    // -- fan-in-on-signal ----------------------------------------------------

    #[test]
    fn fan_out_from_one_causal_port_to_many_is_allowed() {
        let mut b = builder();
        add(
            &mut b,
            "src",
            Paradigm::SignalFlow,
            "source.const",
            "signal",
        );
        add(&mut b, "a", Paradigm::SignalFlow, "control.gain", "signal");
        add(&mut b, "c", Paradigm::SignalFlow, "sinks.scope", "signal");
        connect(&mut b, ("src", "out"), ("a", "in"), "signal").unwrap();
        connect(&mut b, ("src", "out"), ("c", "in"), "signal").unwrap();
        assert_eq!(b.model().wires.len(), 2);
    }

    #[test]
    fn fan_in_onto_one_causal_port_is_refused_and_names_the_existing_wire() {
        let mut b = builder();
        add(&mut b, "s1", Paradigm::SignalFlow, "source.const", "signal");
        add(&mut b, "s2", Paradigm::SignalFlow, "source.const", "signal");
        add(&mut b, "g1", Paradigm::SignalFlow, "control.gain", "signal");
        let first = connect(&mut b, ("s1", "out"), ("g1", "in"), "signal").unwrap();
        let err = connect(&mut b, ("s2", "out"), ("g1", "in"), "signal").unwrap_err();
        assert_eq!(reason(&err), "fan-in-on-signal");
        let msg = err.to_string();
        assert!(
            msg.contains("causal port 'g1.in' is already driven"),
            "{msg}"
        );
        assert!(msg.contains(&format!("wire '{first}'")), "{msg}");
        assert_eq!(b.model().wires.len(), 1);
    }

    #[test]
    fn many_acausal_wires_may_meet_at_one_port() {
        // §8.2: conserving ports have no direction, so the fan-in rule must not apply to them.
        let mut b = builder();
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        add(
            &mut b,
            "r2",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        add(
            &mut b,
            "r3",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        connect(&mut b, ("r1", "p"), ("r3", "p"), "physical-electrical").unwrap();
        connect(&mut b, ("r2", "p"), ("r3", "p"), "physical-electrical").unwrap();
        assert_eq!(b.model().wires.len(), 2);
    }

    #[test]
    fn physical_signal_is_causal_and_still_gets_the_fan_in_rule() {
        // The one case a `physical-` prefix test (compile::nets::is_physical_kind) would get
        // wrong: physical-signal is causal per §8, so it must not be treated as acausal here.
        assert!(!is_acausal("physical-signal"));
        let mut b = builder();
        add(
            &mut b,
            "sen1",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        add(
            &mut b,
            "sen2",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        add(
            &mut b,
            "act",
            Paradigm::PhysicalNetwork,
            "actuator.force",
            "mixed",
        );
        connect(&mut b, ("sen1", "out"), ("act", "in"), "physical-signal").unwrap();
        let err = connect(&mut b, ("sen2", "out"), ("act", "in"), "physical-signal").unwrap_err();
        assert_eq!(reason(&err), "fan-in-on-signal");
    }

    // -- kind-mismatch and dim-mismatch, contract-registry-backed (exact, per-port) ----------

    fn contract(value: serde_json::Value) -> ComponentContract {
        serde_json::from_value(value).unwrap()
    }

    /// An ordinary (non-subckt) component with one `signal` input and one conserving output --
    /// the shape the `domain`-only fallback provably cannot judge.
    fn sensor_contract() -> ComponentContract {
        contract(json!({
            "name": "sensor.voltage",
            "label": "Voltage sensor",
            "ports": [
                {"id": "p", "side": "left", "kind": "physical-electrical", "direction": "inout"},
                {"id": "out", "side": "right", "kind": "signal", "direction": "out"}
            ],
            "geometry": {"bbox": [40, 30], "primitives": []}
        }))
    }

    /// A gain whose port width follows its `width` param -- makes `dim` genuinely
    /// parameter-dependent rather than a constant baked into the contract.
    fn vector_gain_contract() -> ComponentContract {
        contract(json!({
            "name": "control.gain",
            "label": "Gain",
            "parameters": [{"name": "width", "type": "number", "default": 1}],
            "ports": [
                {"id": "in", "side": "left", "kind": "signal", "direction": "in",
                 "dim": ["${{ params.width }}"]},
                {"id": "out", "side": "right", "kind": "signal", "direction": "out",
                 "dim": ["${{ params.width }}"]}
            ],
            "geometry": {"bbox": [40, 30], "primitives": []}
        }))
    }

    fn add_with_params(
        b: &mut ModelBuilder,
        id: &str,
        paradigm: Paradigm,
        r#type: &str,
        domain: &str,
        params: serde_json::Value,
    ) {
        b.add_block(NewBlockSpec {
            r#type: r#type.to_string(),
            paradigm,
            domain: domain.to_string(),
            id: Some(id.to_string()),
            params: params.as_object().unwrap().clone(),
            ..Default::default()
        })
        .unwrap();
    }

    /// Two gains of the given widths, wired out -> in, with the gain contract registered.
    fn gain_chain(from_width: i64, to_width: i64) -> Result<String, BuilderError> {
        let mut b = builder();
        b.register_contract("control.gain", vector_gain_contract());
        add_with_params(
            &mut b,
            "g1",
            Paradigm::SignalFlow,
            "control.gain",
            "signal",
            json!({"width": from_width}),
        );
        add_with_params(
            &mut b,
            "g2",
            Paradigm::SignalFlow,
            "control.gain",
            "signal",
            json!({"width": to_width}),
        );
        connect(&mut b, ("g1", "out"), ("g2", "in"), "signal")
    }

    #[test]
    fn a_signal_wire_onto_an_ordinary_components_physical_electrical_port_is_refused() {
        // The headline case the `domain` fallback could never reach: both blocks are
        // `physical-network`, both carry `domain: "mixed"`, so nothing block-level objects --
        // only the registered contract knows `sensor.voltage.p` is a conserving port.
        let mut b = builder();
        b.register_contract("sensor.voltage", sensor_contract());
        add(
            &mut b,
            "sen",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        add(
            &mut b,
            "sen2",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );

        let err = connect(&mut b, ("sen", "out"), ("sen2", "p"), "signal").unwrap_err();
        assert_eq!(reason(&err), "kind-mismatch");
        let msg = err.to_string();
        assert!(
            msg.contains("'sen2.p' is declared 'physical-electrical'"),
            "{msg}"
        );
        assert!(msg.contains("as a 'signal' wire"), "{msg}");
        assert!(b.model().wires.is_empty());
    }

    #[test]
    fn the_same_two_blocks_connect_when_each_port_is_used_for_what_it_declares() {
        // The other half of the proof -- a checker that rejected everything would pass the test
        // above and fail this one.
        let mut b = builder();
        b.register_contract("sensor.voltage", sensor_contract());
        add(
            &mut b,
            "sen",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        add(
            &mut b,
            "sen2",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        connect(&mut b, ("sen", "p"), ("sen2", "p"), "physical-electrical").unwrap();
        connect(&mut b, ("sen", "out"), ("sen2", "out"), "signal").unwrap();
        assert_eq!(b.model().wires.len(), 2);
    }

    #[test]
    fn without_a_registered_contract_that_same_connection_is_still_accepted() {
        // The fallback is preserved, not silently tightened: an unregistered type keeps exactly
        // the coarse `domain` check it had before the registry existed. This test is the pin
        // that stops a future change turning "unknown" into "reject".
        let mut b = builder();
        add(
            &mut b,
            "sen",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        add(
            &mut b,
            "sen2",
            Paradigm::PhysicalNetwork,
            "sensor.voltage",
            "mixed",
        );
        connect(&mut b, ("sen", "out"), ("sen2", "p"), "signal").unwrap();
        assert_eq!(b.model().wires.len(), 1);
    }

    #[test]
    fn signal_and_signal_bus_are_different_kinds() {
        // Neither is a conserving domain, so `physical_domain_of` says nothing about either --
        // this case is reachable only through an exact per-port declaration.
        let bus = contract(json!({
            "name": "routing.bus_creator",
            "label": "Bus creator",
            "ports": [{"id": "bus", "side": "right", "kind": "signal-bus", "direction": "out"}],
            "geometry": {"bbox": [20, 20], "primitives": []}
        }));
        let mut b = builder();
        b.register_contract("routing.bus_creator", bus);
        b.register_contract("control.gain", vector_gain_contract());
        add(
            &mut b,
            "bus1",
            Paradigm::SignalFlow,
            "routing.bus_creator",
            "signal",
        );
        add(&mut b, "g1", Paradigm::SignalFlow, "control.gain", "signal");

        let err = connect(&mut b, ("bus1", "bus"), ("g1", "in"), "signal").unwrap_err();
        assert_eq!(reason(&err), "kind-mismatch");
        assert!(
            err.to_string()
                .contains("'bus1.bus' is declared 'signal-bus'"),
            "{err}"
        );

        // ...and declaring the wire a `signal-bus` instead now fails on the *other* endpoint,
        // which proves both sides are genuinely being consulted rather than one.
        let err = connect(&mut b, ("bus1", "bus"), ("g1", "in"), "signal-bus").unwrap_err();
        assert_eq!(reason(&err), "kind-mismatch");
        assert!(
            err.to_string().contains("'g1.in' is declared 'signal'"),
            "{err}"
        );
    }

    #[test]
    fn a_contract_declaring_a_reserved_kind_is_refused_even_on_a_plain_wire() {
        let magnetic = contract(json!({
            "name": "magnetic.core",
            "label": "Core",
            "ports": [{"id": "m", "side": "left", "kind": "reserved.physical-magnetic",
                       "direction": "inout"}],
            "geometry": {"bbox": [20, 20], "primitives": []}
        }));
        let mut b = builder();
        b.register_contract("magnetic.core", magnetic);
        add(
            &mut b,
            "m1",
            Paradigm::PhysicalNetwork,
            "magnetic.core",
            "mixed",
        );
        add(
            &mut b,
            "m2",
            Paradigm::PhysicalNetwork,
            "magnetic.core",
            "mixed",
        );
        let err = connect(&mut b, ("m1", "m"), ("m2", "m"), "physical-electrical").unwrap_err();
        assert_eq!(reason(&err), "reserved-kind");
        assert!(
            err.to_string().contains("reserved.physical-magnetic"),
            "{err}"
        );
    }

    #[test]
    fn a_subckt_instances_own_def_still_wins_over_any_registered_contract() {
        // A subckt instance's `type` is the generic SUBCKT_INSTANCE_TYPE, so a contract
        // registered under that name would describe some other def entirely. The def must win.
        let bogus = contract(json!({
            "name": super::super::builder::SUBCKT_INSTANCE_TYPE,
            "label": "Bogus",
            "ports": [{"id": "ctrl", "side": "left", "kind": "physical-thermal",
                       "direction": "inout"}],
            "geometry": {"bbox": [20, 20], "primitives": []}
        }));
        let mut b = builder();
        b.register_contract(super::super::builder::SUBCKT_INSTANCE_TYPE, bogus);
        b.add_subckt_def(mixed_def()).unwrap();
        b.instantiate_subckt(NewSubcktInstanceSpec::new("leaf", (0, 0)).with_id("x1"))
            .unwrap();
        add(&mut b, "c1", Paradigm::SignalFlow, "source.const", "signal");
        // The def says `ctrl` is `signal`; the bogus contract says `physical-thermal`.
        connect(&mut b, ("c1", "out"), ("x1", "ctrl"), "signal").unwrap();
    }

    #[test]
    fn a_dim_mismatch_between_two_resolved_ports_is_refused_and_names_both_dims() {
        let err = gain_chain(3, 1).unwrap_err();
        assert_eq!(reason(&err), "dim-mismatch");
        let msg = err.to_string();
        // Both endpoints come from `PortIncompat`'s own prefix; the failure adds both dims.
        assert!(msg.contains("cannot connect 'g1.out' to 'g2.in'"), "{msg}");
        assert!(
            msg.contains("the source produces dim [3], but the destination requires dim [1]"),
            "{msg}"
        );
    }

    #[test]
    fn equal_dims_connect_including_the_widths_the_contracts_defaults_produce() {
        gain_chain(3, 3).unwrap();
        gain_chain(1, 1).unwrap();
    }

    #[test]
    fn a_dim_is_resolved_per_block_not_per_type() {
        // Both blocks share one contract; only their own `params` differ. If the dim were read
        // from the contract's defaults rather than each block's params, 3 -> 1 would pass.
        assert!(gain_chain(3, 1).is_err());
        assert!(gain_chain(1, 3).is_err());
        assert!(gain_chain(4, 4).is_ok());
    }

    #[test]
    fn an_unknown_dim_on_either_side_skips_the_rule_rather_than_assuming_scalar() {
        // `canConnect` guards on `src.dim !== undefined && dst.dim !== undefined`; a port whose
        // contract declares no `dim` must not be treated as `[1]`, which would reject this.
        let dimless = contract(json!({
            "name": "sinks.scope",
            "label": "Scope",
            "ports": [{"id": "in", "side": "left", "kind": "signal", "direction": "in"}],
            "geometry": {"bbox": [40, 30], "primitives": []}
        }));
        let mut b = builder();
        b.register_contract("control.gain", vector_gain_contract());
        b.register_contract("sinks.scope", dimless);
        add_with_params(
            &mut b,
            "g1",
            Paradigm::SignalFlow,
            "control.gain",
            "signal",
            json!({"width": 3}),
        );
        add(&mut b, "s1", Paradigm::SignalFlow, "sinks.scope", "signal");
        connect(&mut b, ("g1", "out"), ("s1", "in"), "signal").unwrap();
        assert_eq!(b.model().wires.len(), 1);
    }

    #[test]
    fn an_endpoint_naming_a_port_the_contract_does_not_declare_stays_unchecked() {
        // Same property the SubcktDef path has: port existence is not validated, and consulting
        // a contract must not start validating it.
        let mut b = builder();
        b.register_contract("control.gain", vector_gain_contract());
        add(&mut b, "g1", Paradigm::SignalFlow, "control.gain", "signal");
        add(&mut b, "g2", Paradigm::SignalFlow, "control.gain", "signal");
        connect(&mut b, ("g1", "nosuchport"), ("g2", "in"), "signal").unwrap();
    }

    #[test]
    fn facts_from_a_contract_are_exact_while_facts_from_a_domain_are_not() {
        // The distinction the whole fallback rests on, asserted directly rather than only
        // through its consequences.
        let mut b = builder();
        b.register_contract("control.gain", vector_gain_contract());
        add(&mut b, "g1", Paradigm::SignalFlow, "control.gain", "signal");
        add(
            &mut b,
            "r1",
            Paradigm::PhysicalNetwork,
            "passive.resistor",
            "electrical",
        );
        let model = b.model();

        let from_contract = facts_of(
            model,
            b.contracts(),
            b.get_block("g1").unwrap(),
            &Endpoint::new("g1", "in"),
        );
        assert_eq!(from_contract.kind.as_deref(), Some("signal"));
        assert!(from_contract.exact);
        assert_eq!(from_contract.dim, Some(vec![1]));

        let from_domain = facts_of(
            model,
            b.contracts(),
            b.get_block("r1").unwrap(),
            &Endpoint::new("r1", "p"),
        );
        assert_eq!(from_domain.kind.as_deref(), Some("electrical"));
        assert!(!from_domain.exact);
        assert_eq!(from_domain.dim, None);
    }

    // -- helpers -------------------------------------------------------------

    #[test]
    fn physical_domain_of_collapses_both_spellings_and_rejects_everything_else() {
        assert_eq!(
            physical_domain_of("physical-electrical"),
            Some("electrical")
        );
        assert_eq!(physical_domain_of("electrical"), Some("electrical"));
        assert_eq!(physical_domain_of("physical-thermal"), Some("thermal"));
        assert_eq!(physical_domain_of("signal"), None);
        assert_eq!(physical_domain_of("physical-signal"), None);
        assert_eq!(physical_domain_of("mixed"), None);
        assert_eq!(physical_domain_of("reserved.physical-magnetic"), None);
    }
}
