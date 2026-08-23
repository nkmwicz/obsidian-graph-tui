use std::collections::{HashMap, HashSet};

use fdg_sim::{force, Dimensions, ForceGraph, ForceGraphHelper, Simulation, SimulationParameters};
use petgraph::graph::{Graph, NodeIndex};

use crate::vault::Note;

/// A node's stable position after the simulation settles, in the
/// simulation's arbitrary units (not terminal cells — Phase 5 maps these
/// to screen space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Fixed step count standing in for "run to convergence": `fdg-sim` has no
/// built-in stability check, and Fruchterman-Reingold's cooloff factor
/// decays forces toward zero each step, so a generous fixed budget settles
/// the layout without needing to detect convergence explicitly.
const STEPS: usize = 1000;
const STEP_DT: f32 = 0.035;

/// Runs a Fruchterman-Reingold force-directed simulation over `graph` and
/// returns a stable 3D position per node, keyed by the same `NodeIndex`
/// values `graph` uses.
///
/// `fdg-sim` pins `petgraph = "0.6"`, a different major version than this
/// project's `petgraph` (0.8) — its `ForceGraph` is a distinct type from
/// `graph::Graph` and can't be built directly from it. Nodes and edges are
/// copied across by iteration instead, with `mapping` tracking which
/// `fdg-sim` node corresponds to which of ours.
pub fn layout(graph: &Graph<Note, ()>) -> HashMap<NodeIndex, Position> {
    let mut force_graph: ForceGraph<(), ()> = ForceGraph::default();
    let mut mapping = HashMap::with_capacity(graph.node_count());

    for idx in graph.node_indices() {
        let note = &graph[idx];
        let fdg_idx = force_graph.add_force_node(note.path.to_string_lossy(), ());
        mapping.insert(idx, fdg_idx);
    }

    for (from, to) in undirected_edge_pairs(graph) {
        force_graph.add_edge(mapping[&from], mapping[&to], ());
    }

    let parameters = SimulationParameters::new(
        200.0,
        Dimensions::Three,
        force::fruchterman_reingold(45.0, 0.975),
    );
    let mut simulation = Simulation::from_graph(force_graph, parameters);

    for _ in 0..STEPS {
        simulation.update(STEP_DT);
    }

    mapping
        .into_iter()
        .map(|(idx, fdg_idx)| {
            let location = simulation.get_graph()[fdg_idx].location;
            (
                idx,
                Position {
                    x: location.x,
                    y: location.y,
                    z: location.z,
                },
            )
        })
        .collect()
}

/// Collapses edges between the same pair of nodes down to one unordered
/// pair each. Obsidian links are directional, but a note reciprocally
/// linking back (`A -> B` and `B -> A`) is common and shouldn't pull the
/// two notes together twice as hard as a one-way link would: `fdg-sim`'s
/// `ForceGraph` is an undirected multigraph, so feeding it both directed
/// edges would add two parallel edges, and its Fruchterman-Reingold
/// attraction force sums once per parallel edge between neighbors.
fn undirected_edge_pairs(graph: &Graph<Note, ()>) -> HashSet<(NodeIndex, NodeIndex)> {
    let mut pairs = HashSet::with_capacity(graph.edge_count());
    for edge in graph.edge_indices() {
        let (from, to) = graph.edge_endpoints(edge).unwrap();
        pairs.insert(if from <= to { (from, to) } else { (to, from) });
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str) -> Note {
        Note {
            path: path.into(),
            tags: Vec::new(),
            aliases: Vec::new(),
        }
    }

    #[test]
    fn every_node_gets_a_position() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md"));
        let b = graph.add_node(note("b.md"));
        let c = graph.add_node(note("c.md"));
        graph.add_edge(a, b, ());

        let positions = layout(&graph);

        assert_eq!(positions.len(), 3);
        assert!(positions.contains_key(&a));
        assert!(positions.contains_key(&b));
        assert!(positions.contains_key(&c));
    }

    #[test]
    fn connected_nodes_end_up_at_distinct_positions() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md"));
        let b = graph.add_node(note("b.md"));
        graph.add_edge(a, b, ());

        let positions = layout(&graph);

        assert_ne!(positions[&a], positions[&b]);
    }

    #[test]
    fn mutual_link_collapses_to_a_single_undirected_edge() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md"));
        let b = graph.add_node(note("b.md"));
        graph.add_edge(a, b, ());
        graph.add_edge(b, a, ());

        assert_eq!(undirected_edge_pairs(&graph).len(), 1);
    }

    #[test]
    fn self_loop_does_not_panic() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("self-loop.md"));
        graph.add_edge(a, a, ());

        let positions = layout(&graph);

        assert_eq!(positions.len(), 1);
    }
}
