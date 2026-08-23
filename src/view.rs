use std::collections::{HashSet, VecDeque};

use petgraph::graph::{Graph, NodeIndex};

use crate::vault::Note;

/// Default hop count applied the first time a note is jumped to via
/// search — chosen the same way `ROTATE_STEP`/`PAN_STEP`/`ZOOM_STEP` were
/// in `render`: a small, immediately-legible value, not derived from
/// anything. Re-centering on a different note later (Phase 8) keeps
/// whatever hop count was last set rather than resetting to this.
pub const DEFAULT_HOPS: usize = 2;

/// Which subset of the full graph is currently visible in the renderer
/// (`TODO.md` Phase 8) — narrowed live from the interactive session, not
/// by restarting the tool with different CLI flags. The default (every
/// field unset) shows the whole vault; `is_unfiltered()` checks exactly
/// that.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct View {
    /// Neighborhood center, set via jump-to-note search. `hops` is only
    /// meaningful when this is `Some`.
    pub center: Option<NodeIndex>,
    pub hops: usize,
    pub tag: Option<String>,
    pub folder: Option<String>,
}

impl View {
    pub fn is_unfiltered(&self) -> bool {
        self.center.is_none() && self.tag.is_none() && self.folder.is_none()
    }
}

/// Resolves `view` against `graph` into the concrete set of nodes that
/// should be visible. The neighborhood center (if any) narrows first, to
/// nodes within `hops` of it; tag/folder then narrow further — a plain
/// intersection, so the order between tag and folder doesn't matter. A
/// center that no longer exists in `graph` (shouldn't happen in practice,
/// since `View` is only ever built from indices into the same graph it's
/// resolved against, but not worth a panic if it did) is treated as no
/// center at all.
pub fn visible_nodes(graph: &Graph<Note, ()>, view: &View) -> HashSet<NodeIndex> {
    let mut nodes: HashSet<NodeIndex> = match view.center {
        Some(center) if graph.node_weight(center).is_some() => {
            neighborhood(graph, center, view.hops)
        }
        _ => graph.node_indices().collect(),
    };

    if let Some(tag) = &view.tag {
        nodes.retain(|&i| graph[i].tags.iter().any(|t| t == tag));
    }
    if let Some(folder) = &view.folder {
        nodes.retain(|&i| graph[i].path.starts_with(folder));
    }

    nodes
}

/// Undirected BFS out to `hops` steps from `center`, inclusive of `center`
/// itself — the same traversal `algo::shortest_path` already does
/// (undirected: see CLAUDE.md on why links get that treatment absent
/// typed-link data), with a hop bound instead of a target note. `hops =
/// 0` returns just `center`.
fn neighborhood(graph: &Graph<Note, ()>, center: NodeIndex, hops: usize) -> HashSet<NodeIndex> {
    let mut visited = HashSet::new();
    visited.insert(center);
    let mut frontier = VecDeque::new();
    frontier.push_back((center, 0usize));

    while let Some((node, depth)) = frontier.pop_front() {
        if depth == hops {
            continue;
        }
        for neighbor in graph.neighbors_undirected(node) {
            if neighbor != node && visited.insert(neighbor) {
                frontier.push_back((neighbor, depth + 1));
            }
        }
    }

    visited
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{Edge, ParsedVault};

    fn note(path: &str, tags: &[&str]) -> Note {
        Note {
            path: path.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            aliases: Vec::new(),
        }
    }

    /// a - b - c - d (a chain), plus an unconnected orphan `e`.
    fn chain_graph() -> Graph<Note, ()> {
        let vault = ParsedVault {
            notes: vec![
                note("a.md", &[]),
                note("b.md", &[]),
                note("c.md", &[]),
                note("d.md", &[]),
                note("e.md", &[]),
            ],
            edges: vec![
                Edge { from: 0, to: 1 },
                Edge { from: 1, to: 2 },
                Edge { from: 2, to: 3 },
            ],
            unresolved_links: 0,
        };
        crate::graph::build(vault)
    }

    #[test]
    fn default_view_is_unfiltered() {
        assert!(View::default().is_unfiltered());
    }

    #[test]
    fn a_view_with_a_center_is_not_unfiltered() {
        let view = View {
            center: Some(NodeIndex::new(0)),
            ..View::default()
        };
        assert!(!view.is_unfiltered());
    }

    #[test]
    fn zero_hops_returns_only_the_center() {
        let graph = chain_graph();
        let a = NodeIndex::new(0);
        let view = View {
            center: Some(a),
            hops: 0,
            ..View::default()
        };
        assert_eq!(visible_nodes(&graph, &view), HashSet::from([a]));
    }

    #[test]
    fn one_hop_returns_direct_neighbors_only() {
        let graph = chain_graph();
        let (a, b) = (NodeIndex::new(0), NodeIndex::new(1));
        let view = View {
            center: Some(a),
            hops: 1,
            ..View::default()
        };
        assert_eq!(visible_nodes(&graph, &view), HashSet::from([a, b]));
    }

    #[test]
    fn two_hops_reaches_further_but_not_the_whole_chain() {
        let graph = chain_graph();
        let (a, b, c) = (NodeIndex::new(0), NodeIndex::new(1), NodeIndex::new(2));
        let d = NodeIndex::new(3);
        let view = View {
            center: Some(a),
            hops: 2,
            ..View::default()
        };
        let visible = visible_nodes(&graph, &view);
        assert_eq!(visible, HashSet::from([a, b, c]));
        assert!(!visible.contains(&d));
    }

    #[test]
    fn no_center_shows_every_node() {
        let graph = chain_graph();
        let view = View::default();
        assert_eq!(visible_nodes(&graph, &view).len(), graph.node_count());
    }

    #[test]
    fn tag_filter_narrows_to_matching_notes() {
        let vault = ParsedVault {
            notes: vec![note("a.md", &["claim"]), note("b.md", &["source"])],
            edges: vec![Edge { from: 0, to: 1 }],
            unresolved_links: 0,
        };
        let graph = crate::graph::build(vault);
        let view = View {
            tag: Some("source".to_string()),
            ..View::default()
        };
        assert_eq!(visible_nodes(&graph, &view), HashSet::from([NodeIndex::new(1)]));
    }

    #[test]
    fn folder_filter_narrows_to_matching_paths() {
        let vault = ParsedVault {
            notes: vec![
                note("projects/alpha.md", &[]),
                note("journal/today.md", &[]),
            ],
            edges: Vec::new(),
            unresolved_links: 0,
        };
        let graph = crate::graph::build(vault);
        let view = View {
            folder: Some("projects".to_string()),
            ..View::default()
        };
        assert_eq!(visible_nodes(&graph, &view), HashSet::from([NodeIndex::new(0)]));
    }

    #[test]
    fn center_and_tag_filters_combine_as_an_intersection() {
        let graph = chain_graph();
        let a = NodeIndex::new(0);
        // No node in this fixture has a "source" tag, so combining a
        // 1-hop neighborhood with a tag filter that matches nothing
        // should leave nothing visible — proves the two layers actually
        // intersect rather than one silently winning.
        let view = View {
            center: Some(a),
            hops: 1,
            tag: Some("source".to_string()),
            ..View::default()
        };
        assert!(visible_nodes(&graph, &view).is_empty());
    }

    #[test]
    fn a_center_index_no_longer_in_the_graph_is_treated_as_no_center() {
        let graph = chain_graph();
        let bogus = NodeIndex::new(999);
        let view = View {
            center: Some(bogus),
            hops: 1,
            ..View::default()
        };
        assert_eq!(visible_nodes(&graph, &view).len(), graph.node_count());
    }
}
