//! Port-kind compatibility for [`super::ModelBuilder::connect`] — the finer-grained sibling of
//! the block-level cross-paradigm check (`docs/specs/gsim-sch-format.md` §8.1/§8.2, where §8.3's
//! paradigm boundary is the coarse rule [`super::Paradigm::bridges_paradigms`] already covers).
//!
//! # What this is ported from, and why it is a *subset*
//!
//! The reference implementation is `general-simulator-gui`'s
//! `apps/frontend/src/canvas/port-compat.ts` (`canConnect(src, dst)`), read in full before
//! writing this — not the spec prose, which has diverged from real code in this project before.
//! Its four rules, in its own order, are `reserved-kind`, `kind-mismatch`, `fan-in-on-signal`
//! and `dim-mismatch`. (It has **no** `direction-mismatch` rule; §8.2's direction wording is
//! realised there purely as the fan-in check.)
//!
//! `canConnect` takes two `PortDesc`s the canvas has already looked up from each block's
//! evaluated GSF v2 symbol, so it always knows both endpoints' declared `kind` (and, for
//! `dim-mismatch`, their resolved signal dimensions). **`ModelBuilder` has no such catalogue.**
//! That was investigated rather than assumed:
//!
//! - [`super::Block`] stores `type`/`paradigm`/`domain`/`params` — never a per-instance port
//!   list, so an instance cannot answer "what kind is port `p`?" on its own.
//! - [`crate::contract::evaluate_component`] *does* produce `Vec<Port>` with `kind`/`direction`
//!   from a block's `params`, but it needs a [`crate::contract::ComponentContract`], and this
//!   crate has no contract *registry*: `gsim-py`/`gsim-wasm` hand contracts in one at a time as
//!   JSON. There is nothing for `ModelBuilder` to consult.
//! - Giving `ModelBuilder` an optional catalogue reference was considered and rejected: no
//!   caller in either binding has a catalogue to supply, so it would be exactly the
//!   "abstraction without an existing call site" `AGENTS.md` forbids. The precedent is
//!   [`super::ModelBuilder::auto_arrange`], which takes a `bbox_of` closure rather than learning
//!   about contracts, for the same decoupling reason.
//!
//! So this module checks what the document model genuinely knows, and nothing it would have to
//! guess at:
//!
//! | `port-compat.ts` rule | here | source of truth |
//! |---|---|---|
//! | `reserved-kind` | **full parity** | the wire's own `kind` (§8.4 makes it the endpoints' kind) |
//! | `kind-mismatch` | **partial** | a [`super::SubcktDef`]'s `ports[].kind` (exact), else the block's `domain` restricted to cross-*physical-domain* conflicts |
//! | `fan-in-on-signal` | **full parity, computed** | the model's own wire list — better than the GUI's caller-supplied `driven` flag |
//! | `dim-mismatch` | **not implemented** | needs resolved signal dimensions, which only contract evaluation produces |
//!
//! ## The `kind-mismatch` gap, precisely
//!
//! A `spice-subckt` instance block is the one case where a real per-port catalogue *is* already
//! in the document: `SubcktDef.ports[].kind`. Those endpoints get the same exact check
//! `canConnect` does.
//!
//! Every other block only offers `domain`, which §6 defines as the block's *primary* port-kind
//! and which `crate::compile::routing::port_kind_of` already treats that way. It is a per-block
//! value, not a per-port one, and real documents populate it loosely (a `signal-flow` block
//! carrying `domain: "electrical"` is common). Enforcing "wire kind must equal domain" would
//! therefore reject valid models. What *is* safe, and is the universal incompatibility no
//! catalogue could ever override, is a conserving wire crossing between two different physical
//! domains: a `physical-thermal` wire landing on an `electrical`-domain block. Only that is
//! rejected — see [`physical_domain_of`].
//!
//! Closing the remainder (a `signal` wire onto a component's `physical-electrical` port, a
//! `signal` onto a `signal-bus`, direction/`dim` checks) requires a real contract registry
//! reachable from `ModelBuilder`. That is a deliberate, documented gap, not an oversight.
//!
//! Port *existence* is still not validated here, preserving the property
//! [`super::ModelBuilder::instantiate_subckt`] documents: an endpoint naming a port the def does
//! not declare resolves to "unknown kind" and is skipped, never turned into an error.

use super::block::Block;
use super::model::Model;
use super::wire::Endpoint;

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

/// Why a connection was refused. One-to-one with the `reason` strings `port-compat.ts` returns,
/// minus `dim-mismatch` (see the module note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatFailure {
    /// A `reserved.*` kind — catalogued for a future schema version, not connectable in v2.
    ReservedKind { kind: String },
    /// The wire's declared kind contradicts the kind `at` actually declares.
    KindMismatch { at: Endpoint, declared: String },
    /// A causal port cannot be driven twice; `existing_wire` already drives it.
    FanInOnSignal { at: Endpoint, existing_wire: String },
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
        }
    }
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
        }
    }
}

/// The kind an endpoint genuinely declares, or `None` when the document cannot say.
///
/// `Some` in exactly two situations, both of which are real stored data rather than inference:
/// a `spice-subckt` instance whose referenced def declares that port, or a non-bridging block
/// whose `domain` names one of the four conserving physical domains.
fn declared_kind_of(model: &Model, block: &Block, endpoint: &Endpoint) -> Option<String> {
    if let Some(def_ref) = &block.subckt_ref {
        // The def's port list is a genuine per-port catalogue, already in this document.
        let def = model.subckt_defs.iter().find(|d| d.id == *def_ref)?;
        // An endpoint naming a port the def does not declare stays unchecked on purpose --
        // `connect()` has never validated port existence and this must not start doing so.
        return def
            .ports
            .iter()
            .find(|p| p.id == endpoint.port)
            .map(|p| p.kind.clone());
    }
    // A bridging block (converter/routing/hierarchy/annotation) legitimately has ports of
    // different kinds on either side, and `domain` names only one of them -- it cannot be used
    // to judge a particular port, so say nothing.
    if block.paradigm.bridges_paradigms() {
        return None;
    }
    physical_domain_of(&block.domain).map(|d| d.to_string())
}

/// Runs the checks in `port-compat.ts`'s own order: reserved kind, then kind mismatch, then
/// fan-in. `Ok(())` means the connection is compatible as far as the document can tell.
pub fn check(
    model: &Model,
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

    // 2. kind mismatch, against whatever each endpoint genuinely declares.
    for (block, endpoint) in [(from_block, from), (to_block, to)] {
        let Some(declared) = declared_kind_of(model, block, endpoint) else {
            continue;
        };
        if declared.starts_with(RESERVED_KIND_PREFIX) {
            return Err(CompatFailure::ReservedKind { kind: declared });
        }
        // A `domain`-derived declaration is a *domain*, not a full kind: it may only contradict
        // a wire that is itself conserving. `physical_domain_of` collapses both spellings, so
        // `physical-electrical` against domain `electrical` agrees, and against `thermal` does
        // not. A non-conserving wire kind (`signal`, `mixed`, ...) yields `None` and skips.
        let conflicting = match (physical_domain_of(kind), physical_domain_of(&declared)) {
            (Some(wire_domain), Some(port_domain)) => wire_domain != port_domain,
            // Neither side is a physical domain: only an exact kind declaration (a SubcktDef
            // port) can be judged, and then only literally.
            _ => block.subckt_ref.is_some() && declared != kind,
        };
        if conflicting {
            return Err(CompatFailure::KindMismatch {
                at: endpoint.clone(),
                declared,
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
