//! Net resolution — union-find over a `Model`'s physical-network wires, assigning each
//! connected group of ports one net name.
//!
//! Ported in spirit from the editor's Python model package (`gsim_model`), whose own
//! connectivity module had no Rust port before this one, including the **two-pass named-net fix
//! that module documents as a real bug it hit**: a name
//! recorded against `find(from)` mid-loop can go stale, because a later `union` may fold that
//! very root under a new parent. Names are therefore collected first and re-resolved through
//! the *final* structure once every wire has been processed.
//!
//! Two deliberate differences from that reference:
//!
//! - **Only physical-network wires make nets.** A `signal`-kind wire is not a net at all in
//!   `general-simulator`'s netlist grammar — it is a direct block-name reference in a
//!   `field=value` token (`spice-lsp/docs/GRAMMAR.md` §12). Unioning signal wires into nets
//!   would invent nodes the target grammar has no way to express.
//! - **Auto-generated names are derived from the lexicographically smallest endpoint in the
//!   net, not from the union-find root.** The root depends on wire *order*, so using it makes
//!   emitted netlists unstable across unrelated edits; the smallest endpoint is canonical.
//!   The generated form is `n_<block>_<port>`, which satisfies
//!   `docs/specs/gsim-sch-format.md` §9's `^[A-Za-z_][A-Za-z0-9_]*$` net-name rule (a `Block`
//!   id may contain `-`, which is not legal in a net name, so it is mapped to `_`).

use super::{CompileError, NetCollision};
use crate::document::{Endpoint, Wire};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A wire whose `kind` starts with this is an acausal physical-network connection, and is what
/// nets are made of. Everything else (`signal`, `physical-signal`) is a causal block reference.
pub const PHYSICAL_KIND_PREFIX: &str = "physical-";

/// The reference ("ground") net's name in emitted netlist text. Real SPICE, and
/// `general-simulator`'s dialect of it, both spell node zero exactly this way.
pub const REFERENCE_NET: &str = "0";

pub fn is_physical_kind(kind: &str) -> bool {
    kind.starts_with(PHYSICAL_KIND_PREFIX)
}

/// Maps a `Block` id (or port id) onto §9's net-name character set.
fn sanitize(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.insert(0, 'n');
    }
    out
}

/// One namespace of nets: the top-level model, or one `SubcktDef` body (real SPICE `.SUBCKT`
/// scoping — a def's internal nets are its own, invisible to the parent and to sibling defs).
#[derive(Debug, Default, Clone)]
pub struct NetScope {
    net_of: HashMap<Endpoint, String>,
    names: BTreeSet<String>,
}

impl NetScope {
    /// The net a port sits on, or `None` if no physical wire ever touched it (a genuinely
    /// floating port — the caller decides whether that is an error in its context).
    pub fn net_of(&self, endpoint: &Endpoint) -> Option<&str> {
        self.net_of.get(endpoint).map(String::as_str)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Every port this scope placed on a net, with the net's emitted name.
    ///
    /// Ordered by endpoint so a caller building a document-to-netlist map (see
    /// [`super::provenance`]) gets the same list for the same document every time;
    /// the underlying map is a `HashMap`, whose own order is not stable.
    pub fn entries(&self) -> Vec<(&Endpoint, &str)> {
        let mut out: Vec<(&Endpoint, &str)> = self
            .net_of
            .iter()
            .map(|(ep, name)| (ep, name.as_str()))
            .collect();
        out.sort_by(|a, b| (&a.0.block, &a.0.port).cmp(&(&b.0.block, &b.0.port)));
        out
    }
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> UnionFind {
        UnionFind {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    /// `find` without the path compression, for a caller that only has `&self` — the reporting
    /// path, which runs once and only when the compile is already failing.
    fn find_const(&self, mut x: usize) -> usize {
        while self.parent[x] != x {
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

/// Inputs that force or steer a scope's net naming.
#[derive(Default)]
pub struct ScopeOptions<'a> {
    /// Endpoints whose net must be named [`REFERENCE_NET`] — a ground block's port, or a port
    /// on a wire assigned to a `nets[]` entry marked `is_reference`.
    pub reference_endpoints: &'a [Endpoint],
    /// A `SubcktDef`'s boundary pseudo-block id, if this scope is a def body. A net touching
    /// `(boundary, p)` is named `p` — the `.SUBCKT` card's own port name, which is what an
    /// `X`-card's positional connections substitute into.
    pub boundary_block: Option<&'a str>,
}

/// Union-finds `wires` into nets and names each one.
///
/// Naming priority, highest first: the reference net; a `.SUBCKT` boundary port name; an
/// explicit `wire.net` (§9's user-named nets); else the deterministic `n_<block>_<port>` form.
pub fn resolve_scope(wires: &[Wire], opts: &ScopeOptions<'_>) -> Result<NetScope, CompileError> {
    let physical: Vec<&Wire> = wires.iter().filter(|w| is_physical_kind(&w.kind)).collect();

    let mut index: HashMap<Endpoint, usize> = HashMap::new();
    let mut endpoints: Vec<Endpoint> = Vec::new();
    for w in &physical {
        for ep in [&w.from, &w.to] {
            if !index.contains_key(ep) {
                index.insert(ep.clone(), endpoints.len());
                endpoints.push(ep.clone());
            }
        }
    }

    let mut uf = UnionFind::new(endpoints.len());
    let mut provisional: Vec<(usize, &str)> = Vec::new();
    for w in &physical {
        let (a, b) = (index[&w.from], index[&w.to]);
        uf.union(a, b);
        if let Some(name) = &w.net {
            provisional.push((a, name.as_str()));
        }
    }

    // Re-resolve every provisionally-recorded name through the *final* structure -- the exact
    // staleness bug connectivity.py documents.
    let mut explicit: BTreeMap<usize, &str> = BTreeMap::new();
    for (ep_index, name) in provisional {
        explicit.insert(uf.find(ep_index), name);
    }

    let mut reference_roots: BTreeSet<usize> = BTreeSet::new();
    for ep in opts.reference_endpoints {
        if let Some(&i) = index.get(ep) {
            reference_roots.insert(uf.find(i));
        }
    }

    // Boundary port names, and the canonical (smallest) endpoint, per root.
    let mut boundary_port: BTreeMap<usize, &str> = BTreeMap::new();
    let mut smallest: BTreeMap<usize, &Endpoint> = BTreeMap::new();
    for (i, ep) in endpoints.iter().enumerate() {
        let root = uf.find(i);
        if let Some(boundary) = opts.boundary_block {
            if ep.block == boundary {
                let entry = boundary_port.entry(root).or_insert(&ep.port);
                // Deterministic if a net somehow touches two boundary ports (a short): the
                // lexicographically smaller port name wins, rather than wire order.
                if ep.port.as_str() < *entry {
                    *entry = &ep.port;
                }
            }
        }
        smallest
            .entry(root)
            .and_modify(|best| {
                if (&ep.block, &ep.port) < (&best.block, &best.port) {
                    *best = ep;
                }
            })
            .or_insert(ep);
    }

    let mut name_of_root: BTreeMap<usize, String> = BTreeMap::new();
    for (&root, &ep) in &smallest {
        let name = if reference_roots.contains(&root) {
            REFERENCE_NET.to_string()
        } else if let Some(port) = boundary_port.get(&root) {
            sanitize(port)
        } else if let Some(explicit_name) = explicit.get(&root) {
            sanitize(explicit_name)
        } else {
            format!("n_{}_{}", sanitize(&ep.block), sanitize(&ep.port))
        };
        name_of_root.insert(root, name);
    }

    // Two *distinct* nets must not emit under one name. Nothing above guarantees it: the names
    // come from three sources sharing one namespace (a boundary port, a user's `wire.net`, the
    // generated `n_<block>_<port>`) and `sanitize` is lossy, mapping every non-alphanumeric
    // character to `_`, so `V-OUT` and `V_OUT` are the same emitted name. A collision here is
    // worse than the element-name one this file's sibling `provenance` module refuses: two
    // separate nodes become one node, which is a short circuit the author never drew, and the
    // numbers that come out are wrong rather than merely unattributable.
    //
    // The reference net is excluded, and only it: several independent grounds legitimately carry
    // node `0`, because being the reference *is* being the same node.
    let mut roots_by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (root, name) in &name_of_root {
        if name == REFERENCE_NET {
            continue;
        }
        roots_by_name.entry(name.as_str()).or_default().push(*root);
    }
    if let Some((name, roots)) = roots_by_name.iter().find(|(_, roots)| roots.len() > 1) {
        let ports_of = |root: usize| -> Vec<String> {
            let mut out: Vec<String> = endpoints
                .iter()
                .enumerate()
                .filter(|(i, _)| uf.find_const(*i) == root)
                .map(|(_, ep)| format!("{}.{}", ep.block, ep.port))
                .collect();
            out.sort();
            out
        };
        return Err(CompileError::DuplicateNet(Box::new(NetCollision {
            name: (*name).to_string(),
            first: ports_of(roots[0]),
            second: ports_of(roots[1]),
        })));
    }

    let mut scope = NetScope::default();
    for (i, ep) in endpoints.iter().enumerate() {
        let name = name_of_root[&uf.find(i)].clone();
        scope.names.insert(name.clone());
        scope.net_of.insert(ep.clone(), name);
    }
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(id: &str, fb: &str, fp: &str, tb: &str, tp: &str, kind: &str) -> Wire {
        Wire {
            id: id.to_string(),
            from: Endpoint::new(fb, fp),
            to: Endpoint::new(tb, tp),
            kind: kind.to_string(),
            waypoints: Vec::new(),
            pinned: false,
            net: None,
            label: None,
        }
    }

    #[test]
    fn chained_wires_collapse_into_one_net() {
        let wires = vec![
            wire("w1", "r1", "p2", "r2", "p1", "physical-electrical"),
            wire("w2", "r2", "p1", "r3", "p1", "physical-electrical"),
        ];
        let scope = resolve_scope(&wires, &ScopeOptions::default()).unwrap();
        let a = scope.net_of(&Endpoint::new("r1", "p2")).unwrap();
        let b = scope.net_of(&Endpoint::new("r3", "p1")).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "n_r1_p2");
    }

    #[test]
    fn signal_wires_never_become_nets() {
        let wires = vec![wire("w1", "c1", "out", "g1", "in", "signal")];
        let scope = resolve_scope(&wires, &ScopeOptions::default()).unwrap();
        assert!(scope.net_of(&Endpoint::new("c1", "out")).is_none());
    }

    #[test]
    fn an_explicit_wire_net_name_wins_over_the_generated_one() {
        let mut wires = vec![wire("w1", "r1", "p2", "r2", "p1", "physical-electrical")];
        wires[0].net = Some("VOUT".to_string());
        let scope = resolve_scope(&wires, &ScopeOptions::default()).unwrap();
        assert_eq!(scope.net_of(&Endpoint::new("r1", "p2")).unwrap(), "VOUT");
    }

    #[test]
    fn an_explicit_name_survives_a_later_union_folding_its_root() {
        // The exact staleness bug connectivity.py documents: w1 names the net, then w2/w3 merge
        // that group under a different root. A single-pass implementation loses "VOUT" here.
        let mut wires = vec![
            wire("w1", "r2", "p1", "r3", "p1", "physical-electrical"),
            wire("w2", "r1", "p1", "r2", "p1", "physical-electrical"),
            wire("w3", "r0", "p1", "r1", "p1", "physical-electrical"),
        ];
        wires[0].net = Some("VOUT".to_string());
        let scope = resolve_scope(&wires, &ScopeOptions::default()).unwrap();
        for ep in [
            Endpoint::new("r0", "p1"),
            Endpoint::new("r1", "p1"),
            Endpoint::new("r2", "p1"),
            Endpoint::new("r3", "p1"),
        ] {
            assert_eq!(scope.net_of(&ep).unwrap(), "VOUT");
        }
    }

    #[test]
    fn a_reference_endpoint_names_its_whole_net_zero() {
        let wires = vec![
            wire("w1", "gnd1", "p", "r1", "p2", "physical-electrical"),
            wire("w2", "r1", "p2", "r2", "p1", "physical-electrical"),
        ];
        let refs = [Endpoint::new("gnd1", "p")];
        let scope = resolve_scope(
            &wires,
            &ScopeOptions {
                reference_endpoints: &refs,
                boundary_block: None,
            },
        )
        .unwrap();
        assert_eq!(scope.net_of(&Endpoint::new("r2", "p1")).unwrap(), "0");
    }

    #[test]
    fn a_boundary_port_names_its_net_after_the_subckt_card_port() {
        let wires = vec![
            wire("w1", "leaf", "d", "DEV", "d", "physical-electrical"),
            wire("w2", "DEV", "d", "Rsnub", "p1", "physical-electrical"),
        ];
        let scope = resolve_scope(
            &wires,
            &ScopeOptions {
                reference_endpoints: &[],
                boundary_block: Some("leaf"),
            },
        )
        .unwrap();
        assert_eq!(scope.net_of(&Endpoint::new("Rsnub", "p1")).unwrap(), "d");
    }

    #[test]
    fn generated_names_are_stable_against_wire_order() {
        let forward = vec![
            wire("w1", "r1", "p2", "r2", "p1", "physical-electrical"),
            wire("w2", "r2", "p1", "r3", "p1", "physical-electrical"),
        ];
        let reversed = vec![
            wire("w2", "r3", "p1", "r2", "p1", "physical-electrical"),
            wire("w1", "r2", "p1", "r1", "p2", "physical-electrical"),
        ];
        let a = resolve_scope(&forward, &ScopeOptions::default()).unwrap();
        let b = resolve_scope(&reversed, &ScopeOptions::default()).unwrap();
        let ep = Endpoint::new("r2", "p1");
        assert_eq!(a.net_of(&ep).unwrap(), b.net_of(&ep).unwrap());
    }

    #[test]
    fn a_block_id_with_a_dash_is_sanitized_into_a_legal_net_name() {
        let wires = vec![wire("w1", "r-1", "p2", "r2", "p1", "physical-electrical")];
        let scope = resolve_scope(&wires, &ScopeOptions::default()).unwrap();
        assert_eq!(
            scope.net_of(&Endpoint::new("r2", "p1")).unwrap(),
            "n_r_1_p2"
        );
    }

    // --- Two nets must not share a node name ---------------------------------------------
    //
    // The bug these exist for: nothing compared the number of nets with the number of names.
    // Two roots resolving to one name simply wrote the same string into both maps, the set
    // absorbed the duplicate, and every endpoint of both nets reported the same node -- a short
    // circuit nobody drew, with wrong numbers rather than merely unattributable ones.

    fn named(id: &str, fb: &str, fp: &str, tb: &str, tp: &str, net: &str) -> Wire {
        let mut w = wire(id, fb, fp, tb, tp, "physical-electrical");
        w.net = Some(net.to_string());
        w
    }

    #[test]
    fn two_user_named_nets_that_sanitize_to_one_name_are_refused() {
        // `sanitize` maps every non-alphanumeric character to `_`, so these two names are one
        // emitted name -- and the two nets have no port in common.
        let wires = vec![
            named("w1", "R1", "p2", "C1", "p1", "V-OUT"),
            named("w2", "R2", "p2", "C2", "p1", "V_OUT"),
        ];
        let error = resolve_scope(&wires, &ScopeOptions::default()).unwrap_err();
        let message = error.to_string();
        assert!(
            message.starts_with("node name 'V_OUT' is claimed by two separate nets"),
            "{message}"
        );
        // Both nets are identified by their ports, because a net has no id of its own.
        assert!(message.contains("C1.p1, R1.p2"), "{message}");
        assert!(message.contains("C2.p1, R2.p2"), "{message}");
        // And the message says why two different spellings became one name.
        assert!(
            message.contains("'V-OUT' and 'V_OUT' are the same name"),
            "{message}"
        );
    }

    #[test]
    fn a_hand_written_net_name_cannot_take_the_generated_form_of_another_net() {
        // The generated name is `n_<block>_<port>` and shares the namespace with `wire.net`.
        let wires = vec![
            wire("w1", "R1", "p1", "R2", "p1", "physical-electrical"),
            named("w2", "R3", "p1", "R4", "p1", "n_R1_p1"),
        ];
        let error = resolve_scope(&wires, &ScopeOptions::default()).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("node name 'n_R1_p1' is claimed by two separate nets"),
            "{error}"
        );
    }

    #[test]
    fn several_independent_grounds_are_all_node_zero_and_that_is_not_a_collision() {
        // The one name many nets may share. Being the reference *is* being the same node, so
        // excluding it is not an exception to the rule -- it is the rule.
        let wires = vec![
            wire("w1", "R1", "p2", "gnd1", "p", "physical-electrical"),
            wire("w2", "R2", "p2", "gnd2", "p", "physical-electrical"),
        ];
        let refs = vec![Endpoint::new("gnd1", "p"), Endpoint::new("gnd2", "p")];
        let scope = resolve_scope(
            &wires,
            &ScopeOptions {
                reference_endpoints: &refs,
                boundary_block: None,
            },
        )
        .unwrap();
        assert_eq!(scope.net_of(&Endpoint::new("R1", "p2")), Some("0"));
        assert_eq!(scope.net_of(&Endpoint::new("R2", "p2")), Some("0"));
    }

    #[test]
    fn one_net_reaching_many_ports_is_still_one_net() {
        // The false positive to avoid: a star net has many endpoints and one root.
        let wires = vec![
            wire("w1", "R1", "p1", "R2", "p1", "physical-electrical"),
            wire("w2", "R2", "p1", "R3", "p1", "physical-electrical"),
            wire("w3", "R3", "p1", "R4", "p1", "physical-electrical"),
        ];
        let scope = resolve_scope(&wires, &ScopeOptions::default()).unwrap();
        let names: Vec<&str> = scope.names().collect();
        assert_eq!(names, vec!["n_R1_p1"]);
    }
}
