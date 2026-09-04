//! Net resolution — union-find over a `Model`'s physical-network wires, assigning each
//! connected group of ports one net name.
//!
//! Ported in spirit from `general-simulator-gui`'s
//! `packages/gsim-model/src/gsim_model/connectivity.py` (there was no Rust port before this),
//! including the **two-pass named-net fix that file documents as a real bug it hit**: a name
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
pub fn resolve_scope(wires: &[Wire], opts: &ScopeOptions<'_>) -> NetScope {
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

    let mut scope = NetScope::default();
    for (i, ep) in endpoints.iter().enumerate() {
        let name = name_of_root[&uf.find(i)].clone();
        scope.names.insert(name.clone());
        scope.net_of.insert(ep.clone(), name);
    }
    scope
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
        let scope = resolve_scope(&wires, &ScopeOptions::default());
        let a = scope.net_of(&Endpoint::new("r1", "p2")).unwrap();
        let b = scope.net_of(&Endpoint::new("r3", "p1")).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "n_r1_p2");
    }

    #[test]
    fn signal_wires_never_become_nets() {
        let wires = vec![wire("w1", "c1", "out", "g1", "in", "signal")];
        let scope = resolve_scope(&wires, &ScopeOptions::default());
        assert!(scope.net_of(&Endpoint::new("c1", "out")).is_none());
    }

    #[test]
    fn an_explicit_wire_net_name_wins_over_the_generated_one() {
        let mut wires = vec![wire("w1", "r1", "p2", "r2", "p1", "physical-electrical")];
        wires[0].net = Some("VOUT".to_string());
        let scope = resolve_scope(&wires, &ScopeOptions::default());
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
        let scope = resolve_scope(&wires, &ScopeOptions::default());
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
        );
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
        );
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
        let a = resolve_scope(&forward, &ScopeOptions::default());
        let b = resolve_scope(&reversed, &ScopeOptions::default());
        let ep = Endpoint::new("r2", "p1");
        assert_eq!(a.net_of(&ep).unwrap(), b.net_of(&ep).unwrap());
    }

    #[test]
    fn a_block_id_with_a_dash_is_sanitized_into_a_legal_net_name() {
        let wires = vec![wire("w1", "r-1", "p2", "r2", "p1", "physical-electrical")];
        let scope = resolve_scope(&wires, &ScopeOptions::default());
        assert_eq!(
            scope.net_of(&Endpoint::new("r2", "p1")).unwrap(),
            "n_r_1_p2"
        );
    }
}
