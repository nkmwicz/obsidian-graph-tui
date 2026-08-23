use std::collections::{HashMap, HashSet};

use petgraph::graph::{Graph, NodeIndex};

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

/// Builds a new graph containing only `keep`'s nodes, plus the edges of
/// `graph` whose *both* endpoints survive — used by the interactive
/// neighborhood/tag/folder view (`TODO.md` Phase 8) to hand `layout::
/// layout` a genuinely smaller graph to re-lay-out, rather than filtering
/// a whole-vault layout's positions after the fact. The latter would keep
/// showing the whole-vault shape (nodes positioned relative to neighbors
/// that are no longer drawn) instead of a layout that reflects just the
/// narrowed subgraph — defeating the point of narrowing the view.
pub fn induced_subgraph(graph: &Graph<Note, ()>, keep: &HashSet<NodeIndex>) -> Graph<Note, ()> {
    let mut sub = Graph::new();
    let mut mapping: HashMap<NodeIndex, NodeIndex> = HashMap::with_capacity(keep.len());

    // Iterate in original node-index order (not `keep`'s arbitrary hash
    // order) so the subgraph's own node order is deterministic run to run.
    for old in graph.node_indices() {
        if keep.contains(&old) {
            let new = sub.add_node(graph[old].clone());
            mapping.insert(old, new);
        }
    }

    for edge in graph.edge_indices() {
        let (from, to) = graph.edge_endpoints(edge).unwrap();
        if let (Some(&nf), Some(&nt)) = (mapping.get(&from), mapping.get(&to)) {
            sub.add_edge(nf, nt, ());
        }
    }

    sub
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

    #[test]
    fn induced_subgraph_keeps_only_edges_with_both_endpoints_kept() {
        // a -> b -> c; keeping {a, b} should drop c and the b->c edge, but
        // keep a->b since both of its endpoints survive.
        let vault = ParsedVault {
            notes: vec![note("a.md"), note("b.md"), note("c.md")],
            edges: vec![Edge { from: 0, to: 1 }, Edge { from: 1, to: 2 }],
            unresolved_links: 0,
        };
        let graph = build(vault);
        let a = petgraph::graph::NodeIndex::new(0);
        let b = petgraph::graph::NodeIndex::new(1);

        let sub = induced_subgraph(&graph, &HashSet::from([a, b]));

        assert_eq!(sub.node_count(), 2);
        assert_eq!(sub.edge_count(), 1);
    }

    #[test]
    fn induced_subgraph_preserves_note_metadata() {
        let vault = ParsedVault {
            notes: vec![Note {
                path: "a.md".into(),
                tags: vec!["source".to_string()],
                aliases: Vec::new(),
            }],
            edges: Vec::new(),
            unresolved_links: 0,
        };
        let graph = build(vault);
        let a = petgraph::graph::NodeIndex::new(0);

        let sub = induced_subgraph(&graph, &HashSet::from([a]));

        assert_eq!(sub.node_weights().next().unwrap().tags, vec!["source"]);
    }
}
