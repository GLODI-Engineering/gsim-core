//! Real autolayout — auto-spacing and collision avoidance, which does not exist anywhere in the
//! codebase today. `packages/gsim-model`'s own `layout.py` was confirmed by audit to be pure
//! port-geometry math (port centers, a single-bend wire router), not autolayout — this is new
//! work, not a port. Exists so a programmatic caller (an agent building a schematic entirely in
//! code) can produce a visually clean, non-overlapping schematic without manually reasoning
//! about every coordinate — see `docs/decisions/0001-scope-and-naming.md`.
//!
//! Two capabilities, for two different callers:
//! - [`place_without_overlap`] — "place this one new block somewhere sensible near a point,"
//!   for incremental construction.
//! - [`layered_layout`] — "arrange this whole schematic," a simple longest-path layered
//!   (Sugiyama-style) layout driven by wire connectivity: blocks flow left-to-right by
//!   signal-flow depth, stacked vertically within a layer. It does not attempt to minimize wire
//!   crossings — only to guarantee no overlap and a left-to-right reading order, which is the
//!   actual gap (today: nothing, or every block stacked at the origin).

use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

impl Rect {
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

fn snap(v: i64, grid: i64) -> i64 {
    if grid <= 0 {
        return v;
    }
    (v as f64 / grid as f64).round() as i64 * grid
}

fn overlaps_with_margin(candidate: &Rect, existing: &Rect, margin: i64) -> bool {
    let inflated = Rect {
        x: existing.x - margin,
        y: existing.y - margin,
        w: existing.w + 2 * margin,
        h: existing.h + 2 * margin,
    };
    candidate.overlaps(&inflated)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Finds the nearest non-overlapping, grid-snapped position for a `size`-shaped block, starting
/// at `desired` and searching outward in both directions along `axis` in grid-sized steps.
/// Terminates (returning `desired`, snapped) after a generous search bound rather than looping
/// forever on pathological input (e.g. `existing` covering an unbounded region).
pub fn place_without_overlap(
    existing: &[Rect],
    size: (i64, i64),
    desired: (i64, i64),
    axis: Axis,
    margin: i64,
    grid: i64,
) -> (i64, i64) {
    let (w, h) = size;
    let step = grid.max(1);
    let mut offset = 0i64;
    loop {
        let deltas: &[i64] = if offset == 0 {
            &[0]
        } else {
            &[offset, -offset]
        };
        for &d in deltas {
            let (x, y) = match axis {
                Axis::Horizontal => (desired.0 + d, desired.1),
                Axis::Vertical => (desired.0, desired.1 + d),
            };
            let (x, y) = (snap(x, grid), snap(y, grid));
            let candidate = Rect { x, y, w, h };
            if !existing
                .iter()
                .any(|r| overlaps_with_margin(&candidate, r, margin))
            {
                return (x, y);
            }
        }
        offset += step;
        if offset > 1_000_000 {
            return (snap(desired.0, grid), snap(desired.1, grid));
        }
    }
}

pub struct LayoutInput<'a> {
    pub id: &'a str,
    pub size: (i64, i64),
}

/// Arranges a whole schematic left-to-right by signal-flow depth. `edges` is `(from_block,
/// to_block)`; an edge touching an id not present in `blocks` is ignored (e.g. a wire to a
/// subsystem boundary port this layout pass wasn't given). A cycle (a feedback loop — common,
/// e.g. a PID wired around a plant) does not hang the algorithm: Kahn's algorithm simply leaves
/// cycle members at whatever layer they'd reached before the cycle blocked further progress,
/// which is an acceptable cosmetic compromise since this is a layout aid, not a correctness
/// requirement.
pub fn layered_layout(
    blocks: &[LayoutInput<'_>],
    edges: &[(&str, &str)],
    column_gap: i64,
    row_gap: i64,
    grid: i64,
) -> HashMap<String, (i64, i64)> {
    let ids: HashSet<&str> = blocks.iter().map(|b| b.id).collect();
    let mut out_edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = blocks.iter().map(|b| (b.id, 0)).collect();
    for &(from, to) in edges {
        if ids.contains(from) && ids.contains(to) {
            out_edges.entry(from).or_default().push(to);
            *in_degree.entry(to).or_insert(0) += 1;
        }
    }

    let mut layer: HashMap<&str, i64> = blocks.iter().map(|b| (b.id, 0)).collect();
    let mut remaining_in_degree = in_degree.clone();
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();
    while let Some(node) = queue.pop_front() {
        let this_layer = layer[node];
        if let Some(children) = out_edges.get(node) {
            for &child in children {
                if layer[child] < this_layer + 1 {
                    layer.insert(child, this_layer + 1);
                }
                let d = remaining_in_degree
                    .get_mut(child)
                    .expect("child was seeded above");
                *d -= 1;
                if *d == 0 {
                    queue.push_back(child);
                }
            }
        }
    }

    let mut by_layer: HashMap<i64, Vec<&LayoutInput<'_>>> = HashMap::new();
    for b in blocks {
        by_layer.entry(layer[b.id]).or_default().push(b);
    }
    let mut layer_numbers: Vec<i64> = by_layer.keys().copied().collect();
    layer_numbers.sort_unstable();

    let mut positions = HashMap::new();
    let mut x = 0i64;
    for l in layer_numbers {
        let members = &by_layer[&l];
        let column_width = members.iter().map(|b| b.size.0).max().unwrap_or(0);
        let mut y = 0i64;
        for b in members {
            positions.insert(b.id.to_string(), (snap(x, grid), snap(y, grid)));
            y += b.size.1 + row_gap;
        }
        x += column_width + column_gap;
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_without_overlap_returns_desired_when_clear() {
        let pos = place_without_overlap(&[], (20, 20), (100, 100), Axis::Horizontal, 5, 10);
        assert_eq!(pos, (100, 100));
    }

    #[test]
    fn place_without_overlap_steps_around_a_blocker() {
        let existing = [Rect {
            x: 100,
            y: 100,
            w: 20,
            h: 20,
        }];
        let (x, y) =
            place_without_overlap(&existing, (20, 20), (100, 100), Axis::Horizontal, 5, 10);
        let candidate = Rect { x, y, w: 20, h: 20 };
        assert!(!overlaps_with_margin(&candidate, &existing[0], 5));
        assert_eq!(y, 100); // never moved off the desired row -- pure horizontal search
    }

    #[test]
    fn layered_layout_places_a_chain_in_increasing_columns() {
        let blocks = vec![
            LayoutInput {
                id: "a",
                size: (40, 20),
            },
            LayoutInput {
                id: "b",
                size: (40, 20),
            },
            LayoutInput {
                id: "c",
                size: (40, 20),
            },
        ];
        let edges = [("a", "b"), ("b", "c")];
        let pos = layered_layout(&blocks, &edges, 20, 10, 10);
        assert!(pos["a"].0 < pos["b"].0);
        assert!(pos["b"].0 < pos["c"].0);
        assert_eq!(pos["a"].1, 0);
        assert_eq!(pos["b"].1, 0);
        assert_eq!(pos["c"].1, 0);
    }

    #[test]
    fn layered_layout_places_a_diamond_join_after_both_branches() {
        // a -> b -> d
        // a -> c -> d
        let blocks = vec![
            LayoutInput {
                id: "a",
                size: (40, 20),
            },
            LayoutInput {
                id: "b",
                size: (40, 20),
            },
            LayoutInput {
                id: "c",
                size: (40, 20),
            },
            LayoutInput {
                id: "d",
                size: (40, 20),
            },
        ];
        let edges = [("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")];
        let pos = layered_layout(&blocks, &edges, 20, 10, 10);
        // b and c share a-'s next column and stack vertically without overlapping.
        assert_eq!(pos["b"].0, pos["c"].0);
        assert_ne!(pos["b"].1, pos["c"].1);
        // d comes after both branches.
        assert!(pos["d"].0 > pos["b"].0);
        assert!(pos["d"].0 > pos["a"].0);
    }

    #[test]
    fn layered_layout_terminates_and_covers_every_block_even_with_a_cycle() {
        let blocks = vec![
            LayoutInput {
                id: "a",
                size: (40, 20),
            },
            LayoutInput {
                id: "b",
                size: (40, 20),
            },
        ];
        let edges = [("a", "b"), ("b", "a")]; // feedback loop
        let pos = layered_layout(&blocks, &edges, 20, 10, 10);
        assert_eq!(pos.len(), 2);
        assert!(pos.contains_key("a"));
        assert!(pos.contains_key("b"));
    }

    #[test]
    fn layered_layout_ignores_edges_to_unknown_blocks() {
        let blocks = vec![LayoutInput {
            id: "a",
            size: (40, 20),
        }];
        let edges = [("a", "ghost"), ("ghost", "a")];
        let pos = layered_layout(&blocks, &edges, 20, 10, 10);
        assert_eq!(pos.len(), 1);
        assert_eq!(pos["a"], (0, 0));
    }
}
