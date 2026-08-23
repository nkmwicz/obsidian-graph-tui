use std::collections::HashSet;

use petgraph::graph::Graph;

use crate::vault::{Note, ParsedVault};

/// Builds a directed graph from a parsed vault: `Note`s become node
/// weights, `ParsedVault::edges` become directed graph edges. Consumes
/// `vault` — everything from Phase 4 onward operates on the returned
/// graph and shouldn't need `ParsedVault` again (see CLAUDE.md's "Graph
/// model / query").
///
/// `petgraph::Graph` is a multigraph by default, so a note linking to the
/// same target twice would otherwise produce two parallel edges; those
/// collapse into one here rather than inflating neighbor/degree counts
/// for later phases (layout, query) with no benefit.
pub fn build(vault: ParsedVault) -> Graph<Note, ()> {
    let mut graph = Graph::new();
    let indices: Vec<_> = vault
        .notes
        .into_iter()
        .map(|note| graph.add_node(note))
        .collect();

    let mut seen = HashSet::with_capacity(vault.edges.len());
    for edge in vault.edges {
        if seen.insert((edge.from, edge.to)) {
            graph.add_edge(indices[edge.from], indices[edge.to], ());
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Edge;

    fn note(path: &str) -> Note {
        Note {
            path: path.into(),
            tags: Vec::new(),
            aliases: Vec::new(),
        }
    }

    #[test]
    fn node_count_matches_note_count() {
        let vault = ParsedVault {
            notes: vec![note("a.md"), note("b.md"), note("c.md")],
            edges: vec![Edge { from: 0, to: 1 }],
            unresolved_links: 0,
        };
        let graph = build(vault);
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn neighbor_count_for_a_known_note() {
        // a -> b, a -> c, b -> c
        let vault = ParsedVault {
            notes: vec![note("a.md"), note("b.md"), note("c.md")],
            edges: vec![
                Edge { from: 0, to: 1 },
                Edge { from: 0, to: 2 },
                Edge { from: 1, to: 2 },
            ],
            unresolved_links: 0,
        };
        let graph = build(vault);
        let a = petgraph::graph::NodeIndex::new(0);
        assert_eq!(graph.neighbors(a).count(), 2);
    }

    #[test]
    fn self_loop_converts_without_panicking() {
        let vault = ParsedVault {
            notes: vec![note("self-loop.md")],
            edges: vec![Edge { from: 0, to: 0 }],
            unresolved_links: 0,
        };
        let graph = build(vault);
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 1);
        let idx = petgraph::graph::NodeIndex::new(0);
        assert_eq!(graph.neighbors(idx).count(), 1);
        assert_eq!(graph.neighbors(idx).next(), Some(idx));
    }

    #[test]
    fn duplicate_links_between_the_same_pair_collapse_to_one_edge() {
        let vault = ParsedVault {
            notes: vec![note("a.md"), note("b.md")],
            edges: vec![Edge { from: 0, to: 1 }, Edge { from: 0, to: 1 }],
            unresolved_links: 0,
        };
        let graph = build(vault);
        assert_eq!(graph.edge_count(), 1);
    }
}
