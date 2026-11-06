//! Netlist compilation — Phase 2 (`docs/decisions/0001-scope-and-naming.md` deferred this
//! explicitly; this module opens it).
//!
//! Turns a [`crate::document::Model`] into netlist text for a [`NetlistTarget`]. The first (and
//! so far only) real target is [`GeneralSimulatorTarget`], emitting `general-simulator`'s own
//! dual-grammar format — see [`spec`] for what those two grammars are and why one registry
//! covers both.
//!
//! # Pipeline
//!
//! 1. [`nets::resolve_scope`] union-finds physical-network wires into named nets, once per
//!    scope: the top-level model, plus one independent scope per [`crate::document::SubcktDef`]
//!    body (real `.SUBCKT` scoping — a def's internal nets are invisible to its parent).
//! 2. [`Outputs`] records what every block's output(s) are *called* in emitted text, since a
//!    `field=value` reference names a block output, not a net.
//! 3. The target renders `.SUBCKT`/`.ENDS` blocks, then top-level element and `kind=` cards.
//!
//! Netlist emission logic lives here, in `gsim-core`, not in a binding — `AGENTS.md`'s "one
//! implementation, thin bindings" rule.

pub mod nets;
pub mod routing;
pub mod spec;

mod general_simulator;

pub use general_simulator::GeneralSimulatorTarget;
pub use nets::{NetScope, ScopeOptions};
pub use spec::{lookup, TypeSpec};

use crate::document::{Block, Endpoint, Model, SubcktDef, Wire};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum CompileError {
    /// No [`spec::TypeSpec`] for a block's `type`. Deliberately fatal rather than skipped: a
    /// silently-dropped component is a wrong circuit that still simulates.
    UnknownType {
        block: String,
        type_name: String,
    },
    UnknownTarget(String),
    /// A port the type's spec needs a node for is not on any physical wire.
    FloatingPort {
        block: String,
        port: String,
    },
    /// A `field=` needing a block reference has nothing wired to that port.
    UnconnectedRef {
        block: String,
        port: String,
    },
    MissingParam {
        block: String,
        param: String,
    },
    /// A block instantiates a `SubcktDef` that isn't in the model.
    UnknownSubcktDef {
        block: String,
        def_ref: String,
    },
    /// An `X` card needs one connection per def port; a port had no wire.
    UnconnectedSubcktPort {
        block: String,
        port: String,
    },
    /// A `kind=` variant parameter whose value has no mapped `kind=` string.
    UnknownVariant {
        block: String,
        param: String,
        value: String,
    },
    /// A wire reads an output past the first on a family that emits **one vector-valued
    /// signal** under the block's own name (`kind=statespace`/`discretestatespace`). There is
    /// no netlist spelling for "element 2 of that vector", so emitting the block name would
    /// silently hand the consumer the whole vector instead.
    VectorOutputPort {
        block: String,
        port: String,
        type_name: String,
    },
    /// An `inputs=` list whose length comes from a parameter had a hole, or that parameter is
    /// missing/not an array so the length is unknowable.
    IndexedRefArity {
        block: String,
        field: String,
        expected: usize,
        connected: usize,
    },
    /// A `routing.from` whose tag matches no `routing.goto` in its scope
    /// (`docs/specs/gsim-sch-format.md` §10.3 — a hard error, never a silently dangling input).
    UnresolvedFromTag {
        block: String,
        tag: String,
    },
    /// A `routing.from` whose tag matches more than one `routing.goto` — §10.3's second hard
    /// error. There is no "nearest wins" tiebreak to fall back on without a subsystem hierarchy;
    /// see [`routing`]'s module doc.
    AmbiguousFromTag {
        block: String,
        tag: String,
        candidates: Vec<String>,
    },
    /// A `routing.from` whose matched `routing.goto` is itself (transitively) driven by that
    /// same `From` — an implicit connection that closes a loop with no block in it.
    CyclicFromTag {
        block: String,
        tag: String,
    },
    /// A `routing.goto`/`routing.from` on a port-kind §10.2 forbids — the acausal conserving
    /// kinds, which must use named nets (§9) instead.
    RoutingKindUnsupported {
        block: String,
        type_name: String,
        kind: String,
    },
    /// A `field=` reference landed on an endpoint that drives nothing — typically a peer
    /// block's *input* port, reached by chaining one consumer to another instead of fanning
    /// out from the driver's output.
    ///
    /// Fatal on purpose. This used to fall back to the peer block's own name, which almost
    /// always names a block that really exists, so the netlist loaded and the engine
    /// simulated a different circuit than the one drawn, with no diagnostic anywhere.
    RefNotAnOutput {
        block: String,
        port: String,
        peer_block: String,
        peer_port: String,
    },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::UnknownType { block, type_name } => write!(
                f,
                "block '{block}' has type '{type_name}', which no netlist emitter knows -- add a TypeSpec for it in compile::spec::REGISTRY"
            ),
            CompileError::UnknownTarget(t) => {
                write!(f, "unknown netlist target '{t}' (known: general-simulator)")
            }
            CompileError::FloatingPort { block, port } => write!(
                f,
                "port '{block}.{port}' needs a node but no physical-network wire reaches it"
            ),
            CompileError::UnconnectedRef { block, port } => write!(
                f,
                "port '{block}.{port}' must reference another block's output, but nothing is wired to it"
            ),
            CompileError::MissingParam { block, param } => {
                write!(f, "block '{block}' is missing required parameter '{param}'")
            }
            CompileError::UnknownSubcktDef { block, def_ref } => write!(
                f,
                "block '{block}' instantiates SubcktDef '{def_ref}', which is not in this model"
            ),
            CompileError::UnconnectedSubcktPort { block, port } => write!(
                f,
                "subckt instance '{block}' has nothing wired to its port '{port}' -- an X card's connections are positional and cannot have a hole"
            ),
            CompileError::UnknownVariant {
                block,
                param,
                value,
            } => write!(
                f,
                "block '{block}' parameter '{param}' value '{value}' maps to no netlist kind= string"
            ),
            CompileError::VectorOutputPort {
                block,
                port,
                type_name,
            } => write!(
                f,
                "port '{block}.{port}' cannot be referenced: '{type_name}' emits one vector-valued signal under the block's own name, so only its first output port is addressable in netlist text"
            ),
            CompileError::IndexedRefArity {
                block,
                field,
                expected,
                connected,
            } => write!(
                f,
                "block '{block}' needs {expected} driver(s) for its '{field}=' list but only {connected} of its indexed ports are wired -- a positional list cannot have a hole"
            ),
            CompileError::UnresolvedFromTag { block, tag } => write!(
                f,
                "Unresolved From tag \"{tag}\" on block '{block}': no routing.goto in this scope broadcasts it on the same port-kind"
            ),
            CompileError::AmbiguousFromTag {
                block,
                tag,
                candidates,
            } => write!(
                f,
                "Ambiguous From tag \"{tag}\" on block '{block}': {} routing.goto blocks broadcast it ({})",
                candidates.len(),
                candidates.join(", ")
            ),
            CompileError::CyclicFromTag { block, tag } => write!(
                f,
                "From tag \"{tag}\" on block '{block}' resolves to a routing.goto that this same From drives -- an implicit connection cannot close a loop with no block in it"
            ),
            CompileError::RoutingKindUnsupported {
                block,
                type_name,
                kind,
            } => write!(
                f,
                "block '{block}' is a '{type_name}' on port-kind '{kind}', which cannot be routed by tag -- acausal kinds must be connected through a named net instead (docs/specs/gsim-sch-format.md §10.2)"
            ),
            CompileError::RefNotAnOutput {
                block,
                port,
                peer_block,
                peer_port,
            } => write!(
                f,
                "port '{block}.{port}' must reference a block output, but the wire on it lands on '{peer_block}.{peer_port}', which drives nothing -- fan out from the driving block's own output port instead of chaining from another consumer's input"
            ),
        }
    }
}

impl std::error::Error for CompileError {}

/// A compiled netlist plus anything the compiler decided heuristically.
#[derive(Debug, Clone, Default)]
pub struct CompileOutput {
    pub netlist: String,
    /// Info-level decisions worth surfacing — notably `docs/specs/gsim-sch-format.md` §9's
    /// "if no `is_reference` net is declared ... the compiler selects one heuristically and
    /// emits an info-level note".
    pub notes: Vec<String>,
}

/// A netlist backend. One implementation per simulator dialect; the pipeline above
/// (net resolution, output naming) is shared by all of them.
pub trait NetlistTarget {
    fn name(&self) -> &'static str;
    fn emit(&self, ctx: &CompileContext<'_>) -> Result<CompileOutput, CompileError>;
}

/// What every block's output(s) are called in emitted netlist text.
///
/// Rule (a), confirmed against `general-mna/src/block_graph.rs`'s own `output_names`
/// documentation: **a bare block-name reference means that block's *first* declared output**,
/// so `output_names[0]` aliases the block's own name. Outputs beyond the first need their own
/// name, taken from the block's own `outputs=` parameter where its type declares one
/// (`cscript`/`octfunc`/`pyfunc`/`pspwm`/...), else from that family's documented default —
/// for `pspwm` that default is `[name, "{name}_comp"]`, read straight out of `system_builder.rs`.
///
/// **Index 0 is not nameable.** The rule above is not a convention this compiler could choose
/// to break: the engine registers a multi-output block's first output under the block's own
/// card name and reads only `outputs[1..]` from the emitted list. Honouring a user-declared
/// `outputs[0]` put a name in the netlist that the engine never registers, so the run died
/// with `UnknownBlockInput(<that name>)` — verified by running one. [`Outputs::record`]
/// therefore pins index 0 to the card name and records a note saying so.
#[derive(Debug, Default, Clone)]
pub struct Outputs {
    by_port: BTreeMap<(String, String), String>,
}

impl Outputs {
    fn record(&mut self, block: &Block, spec: &TypeSpec, notes: &mut Vec<String>) {
        let declared: Option<Vec<String>> = block.params.get("outputs").and_then(|v| match v {
            serde_json::Value::Array(items) => {
                Some(items.iter().map(spec::render_value).collect::<Vec<_>>())
            }
            serde_json::Value::String(s) => {
                Some(s.split(',').map(|p| p.trim().to_string()).collect())
            }
            _ => None,
        });
        let card = spec::card_name(&block.id, spec);
        // Index 0 is not the user's to name. The engine registers a multi-output block's
        // first output under the block's own card name and only reads `outputs[1..]` from the
        // list, so honouring a declared `outputs[0]` emitted a name nothing ever registers and
        // every consumer of it died with `UnknownBlockInput`. Say so rather than dropping it
        // in silence -- the user wrote that name expecting to use it.
        if let Some(names) = &declared {
            if let Some(first) = names.first() {
                if !first.is_empty() && *first != card && spec.output_port_ids(block).len() > 1 {
                    notes.push(format!(
                        "block '{}': output 0 is always named after the block itself ('{card}'), \
                         so the declared outputs[0] = '{first}' is not emitted -- refer to that \
                         output as '{card}'",
                        block.id
                    ));
                }
            }
        }
        for (i, port) in spec.output_port_ids(block).into_iter().enumerate() {
            let name = match &declared {
                // The first output always aliases the block's own (card) name, whatever the
                // block declared.
                _ if i == 0 => card.clone(),
                Some(names) if i < names.len() => names[i].clone(),
                // Family default for a second output, matching general-mna's own. The suffix
                // is the port id, or -- for a family carrying its outputs' declared names in a
                // parameter (`cscript`'s `output_names`) -- that name, so a generated
                // `outputs=` list stays readable. Prefixing the card name keeps every emitted
                // symbol unique even when two blocks declare the same output name.
                _ => format!("{card}_{}", spec.output_suffix(block, i, &port)),
            };
            self.by_port.insert((block.id.clone(), port), name);
        }
    }

    pub fn name_of(&self, block: &str, port: &str) -> Option<&str> {
        self.by_port
            .get(&(block.to_string(), port.to_string()))
            .map(String::as_str)
    }
}

/// One namespace being compiled: the top-level model, or one `SubcktDef` body.
pub struct Scope<'a> {
    pub blocks: &'a [Block],
    pub wires: &'a [Wire],
    pub nets: NetScope,
    pub outputs: Outputs,
    /// The def whose body this is, if any.
    pub def: Option<&'a SubcktDef>,
    /// Goto/From lowering, already resolved: `routing.from` block id -> the endpoint that
    /// *really* drives it (the far end of its matched `routing.goto`'s own `in` wire, chased
    /// through any further `From` hops). Consulted by [`Scope::ref_name`] so a reference landing
    /// on a `From` is rewritten to the real upstream source, leaving neither routing block with
    /// any netlist footprint. See [`routing`].
    pub from_source: BTreeMap<String, Endpoint>,
}

impl Scope<'_> {
    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    /// The other end of whatever wire lands on `(block, port)`, preferring a wire whose far end
    /// is a *known output* — a `field=` reference names a driver, and a port can legally carry
    /// more than one wire (the chained fan-out convention `examples/h_bridge_3phase.py`
    /// established).
    pub fn peer_of(&self, block: &str, port: &str) -> Option<&Endpoint> {
        let me = Endpoint::new(block, port);
        let mut fallback = None;
        for w in self.wires {
            let peer = if w.from == me {
                &w.to
            } else if w.to == me {
                &w.from
            } else {
                continue;
            };
            if self.outputs.name_of(&peer.block, &peer.port).is_some() {
                return Some(peer);
            }
            fallback.get_or_insert(peer);
        }
        fallback
    }

    /// What a `field=` reference on `(block, port)` renders to: the driving block's output
    /// name, or — when the driver is this def's own boundary — the boundary *port* name, which
    /// is what an `X` card substitutes into positionally.
    pub fn ref_name(&self, block: &str, port: &str) -> Result<String, CompileError> {
        let peer = self
            .peer_of(block, port)
            .ok_or_else(|| CompileError::UnconnectedRef {
                block: block.to_string(),
                port: port.to_string(),
            })?;
        // Goto/From lowering (§10): a reference that lands on a `routing.from` means the real
        // driver behind that From's matched `routing.goto`. `from_source` is already fully
        // chased, so one substitution is enough and every case below -- def boundary,
        // vector-output rejection, output naming -- then sees the real source endpoint.
        let peer = self.from_source.get(&peer.block).unwrap_or(peer);
        if let Some(def) = self.def {
            if peer.block == def.id {
                return Ok(peer.port.clone());
            }
        }
        // A vector-output family has exactly one addressable name; reading any later port
        // would silently mean "the whole vector" instead of "that element".
        if let Some(peer_spec) = self
            .block(&peer.block)
            .and_then(|b| spec::lookup(&b.r#type))
        {
            if peer_spec.vector_output
                && peer_spec.output_ports.first() != Some(&peer.port.as_str())
            {
                return Err(CompileError::VectorOutputPort {
                    block: peer.block.clone(),
                    port: peer.port.clone(),
                    type_name: peer_spec.type_name.to_string(),
                });
            }
        }
        // No silent fallback to the peer block's own name here. That name nearly always
        // exists, so the netlist would load and the engine would simulate a circuit that was
        // never drawn -- see the `RefNotAnOutput` doc comment.
        self.outputs
            .name_of(&peer.block, &peer.port)
            .map(str::to_string)
            .ok_or_else(|| CompileError::RefNotAnOutput {
                block: block.to_string(),
                port: port.to_string(),
                peer_block: peer.block.clone(),
                peer_port: peer.port.clone(),
            })
    }
}

/// Everything a [`NetlistTarget`] needs: the model, and every scope already resolved.
pub struct CompileContext<'a> {
    pub model: &'a Model,
    pub top: Scope<'a>,
    /// Keyed by `SubcktDef::id`.
    pub defs: BTreeMap<String, Scope<'a>>,
    pub notes: Vec<String>,
}

/// Block types that only *name* the reference net rather than emitting anything.
const GROUND_TYPES: &[&str] = &["routing.ground", "physical_electrical.ground"];

fn reference_endpoints(blocks: &[Block], wires: &[Wire], model: &Model) -> Vec<Endpoint> {
    let mut eps: Vec<Endpoint> = Vec::new();
    for b in blocks {
        if GROUND_TYPES.contains(&b.r#type.as_str()) {
            for w in wires {
                for ep in [&w.from, &w.to] {
                    if ep.block == b.id {
                        eps.push(ep.clone());
                    }
                }
            }
        }
    }
    // A `nets[]` entry marked is_reference, reached through any wire assigned to that net.
    let reference_names: Vec<String> = model
        .nets
        .iter()
        .filter(|n| n.get("is_reference").and_then(|v| v.as_bool()) == Some(true))
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    for w in wires {
        if let Some(net) = &w.net {
            if reference_names.iter().any(|n| n == net) {
                eps.push(w.from.clone());
            }
        }
    }
    eps
}

fn build_scope<'a>(
    blocks: &'a [Block],
    wires: &'a [Wire],
    def: Option<&'a SubcktDef>,
    model: &Model,
    notes: &mut Vec<String>,
) -> Result<Scope<'a>, CompileError> {
    let refs = reference_endpoints(blocks, wires, model);
    let scope_label = def.map(|d| d.id.as_str()).unwrap_or("<top level>");
    if refs.is_empty() && def.is_none() && !wires.is_empty() {
        notes.push(format!(
            "{scope_label}: no reference net declared (no ground block, no nets[] entry with is_reference) -- \
             the largest net was selected as node 0, per docs/specs/gsim-sch-format.md §9"
        ));
    }
    let mut nets = nets::resolve_scope(
        wires,
        &ScopeOptions {
            reference_endpoints: &refs,
            boundary_block: def.map(|d| d.id.as_str()),
        },
    );
    if refs.is_empty() && def.is_none() {
        nets = heuristic_reference(wires, &nets);
    }

    let mut outputs = Outputs::default();
    for b in blocks {
        if let Some(spec) = spec::lookup(&b.r#type) {
            outputs.record(b, spec, notes);
        } else if b.subckt_ref.is_none() {
            return Err(CompileError::UnknownType {
                block: b.id.clone(),
                type_name: b.r#type.clone(),
            });
        }
    }
    let mut scope = Scope {
        blocks,
        wires,
        nets,
        outputs,
        def,
        from_source: BTreeMap::new(),
    };
    // Goto/From lowering runs here — after `outputs` (the chase uses `peer_of`, whose
    // driver-preference consults it) and before any target ever calls `ref_name`. Doing it as
    // part of scope construction, rather than inside the emitter, means every `field=` shape
    // (`in=`, `inputs=`, `ctrl=`, `gate=`, an `X` card's positional connections) picks up the
    // substitution for free, and an unresolved tag fails compilation even if nothing reads it.
    scope.from_source = resolve_from_sources(&scope)?;
    Ok(scope)
}

/// Resolves each `routing.from` in a scope to the endpoint that really drives it.
///
/// [`routing::resolve_scope`] does the tag/port-kind matching; this walks the one further step
/// the netlist needs — from the matched `Goto` to whatever is wired to its `in` port. That
/// driver may itself be another `From` (a Goto rebroadcasting a tag it received), so the walk
/// loops, guarded by [`routing::Chase`] against a tag that closes back on itself.
fn resolve_from_sources(scope: &Scope<'_>) -> Result<BTreeMap<String, Endpoint>, CompileError> {
    let matched = routing::resolve_scope(scope.blocks, scope.wires)?;
    let mut sources = BTreeMap::new();
    for from_id in matched.keys() {
        let mut chase = routing::Chase::new();
        let mut cursor = from_id.clone();
        let source = loop {
            let from_block = scope.block(&cursor).expect("id came from this scope");
            chase.visit(from_block)?;
            let goto_id = matched
                .get(&cursor)
                .expect("routing::resolve_scope covers every From in this scope");
            let peer = scope
                .peer_of(goto_id, routing::GOTO_IN_PORT)
                .ok_or_else(|| CompileError::UnconnectedRef {
                    block: goto_id.clone(),
                    port: routing::GOTO_IN_PORT.to_string(),
                })?;
            match scope.block(&peer.block) {
                Some(b) if routing::is_from(b) => cursor = peer.block.clone(),
                _ => break peer.clone(),
            }
        };
        sources.insert(from_id.clone(), source);
    }
    Ok(sources)
}

/// §9: "If no `is_reference` net is declared for an acausal sub-network, the compiler selects
/// one heuristically." The heuristic is the net with the most endpoints (ties broken by name,
/// so it is deterministic) — in practice the ground rail of any real circuit.
fn heuristic_reference(wires: &[Wire], scope: &NetScope) -> NetScope {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for w in wires {
        for ep in [&w.from, &w.to] {
            if let Some(name) = scope.net_of(ep) {
                *counts.entry(name).or_default() += 1;
            }
        }
    }
    let Some((&winner, _)) = counts.iter().max_by_key(|(name, count)| (**count, *name)) else {
        return scope.clone();
    };
    let refs: Vec<Endpoint> = wires
        .iter()
        .flat_map(|w| [w.from.clone(), w.to.clone()])
        .filter(|ep| scope.net_of(ep) == Some(winner))
        .collect();
    nets::resolve_scope(
        wires,
        &ScopeOptions {
            reference_endpoints: &refs,
            boundary_block: None,
        },
    )
}

/// Compiles `model` for the named target.
pub fn compile(model: &Model, target: &str) -> Result<CompileOutput, CompileError> {
    let target: Box<dyn NetlistTarget> = match target {
        "general-simulator" => Box::new(GeneralSimulatorTarget),
        other => return Err(CompileError::UnknownTarget(other.to_string())),
    };
    let mut notes = Vec::new();
    let top = build_scope(&model.blocks, &model.wires, None, model, &mut notes)?;
    let mut defs = BTreeMap::new();
    for def in &model.subckt_defs {
        let scope = build_scope(&def.blocks, &def.wires, Some(def), model, &mut notes)?;
        defs.insert(def.id.clone(), scope);
    }
    let ctx = CompileContext {
        model,
        top,
        defs,
        notes,
    };
    let mut out = target.emit(&ctx)?;
    out.notes.extend(ctx.notes.iter().cloned());
    Ok(out)
}
