use std::collections::{HashMap, HashSet};

use fdg_sim::{Dimensions, ForceGraph, ForceGraphHelper, Simulation, SimulationParameters, force};
use petgraph::graph::{Graph, NodeIndex};

use crate::vault::Note;

mod cache;
pub use cache::load_or_compute;

/// A node's stable position after the simulation settles, in the
/// simulation's arbitrary units (not terminal cells — Phase 5 maps these
/// to screen space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Safety ceiling on simulation steps — same value the old fixed budget
/// used. Fruchterman-Reingold's cooloff factor decays forces geometrically
/// each step, so in practice `run_to_convergence` below stops well before
/// this (see the Phase 9 benchmark for real numbers); the cap just bounds
/// worst-case runtime for a pathological graph rather than driving normal
/// behavior.
const MAX_STEPS: usize = 1000;
const STEP_DT: f32 = 0.035;

/// Stop once no node moves more than this far (in simulation units, which
/// `force::fruchterman_reingold`'s `scale = 45.0` puts on the order of
/// tens of units across a real graph) in a single step — small enough
/// relative to that scale to be visually settled, without waiting for
/// forces to hit exactly zero.
const CONVERGENCE_THRESHOLD: f32 = 3.0;

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
    let (mut simulation, mapping) = build_simulation(graph);
    run_to_convergence(&mut simulation);
    extract_positions(&simulation, mapping)
}

/// Same as `layout`, but with an exact step count instead of running to
/// convergence — split out so the Phase 9 benchmark (see the
/// `layout_benchmark` test below) can time a fixed, comparable amount of
/// work at each graph size. Not used outside tests, hence `#[cfg(test)]`.
#[cfg(test)]
fn layout_with_steps(graph: &Graph<Note, ()>, steps: usize) -> HashMap<NodeIndex, Position> {
    let (mut simulation, mapping) = build_simulation(graph);
    for _ in 0..steps {
        simulation.update(STEP_DT);
    }
    extract_positions(&simulation, mapping)
}

type Sim = Simulation<(), ()>;

fn build_simulation(graph: &Graph<Note, ()>) -> (Sim, HashMap<NodeIndex, fdg_sim::petgraph::graph::NodeIndex>) {
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
    (Simulation::from_graph(force_graph, parameters), mapping)
}

fn extract_positions(
    simulation: &Sim,
    mapping: HashMap<NodeIndex, fdg_sim::petgraph::graph::NodeIndex>,
) -> HashMap<NodeIndex, Position> {
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

/// Steps the simulation until the largest single-step node displacement
/// drops below `CONVERGENCE_THRESHOLD`, or `MAX_STEPS` is hit. Returns the
/// number of steps actually run (used by tests/benchmarking to see how
/// much this saves over the old fixed budget).
fn run_to_convergence(simulation: &mut Sim) -> usize {
    let mut previous: Vec<(f32, f32, f32)> = simulation
        .get_graph()
        .node_weights()
        .map(|n| (n.location.x, n.location.y, n.location.z))
        .collect();

    for step in 1..=MAX_STEPS {
        simulation.update(STEP_DT);

        let mut max_displacement_sq = 0.0_f32;
        for (node, prev) in simulation.get_graph().node_weights().zip(previous.iter_mut()) {
            let dx = node.location.x - prev.0;
            let dy = node.location.y - prev.1;
            let dz = node.location.z - prev.2;
            max_displacement_sq = max_displacement_sq.max(dx * dx + dy * dy + dz * dz);
            *prev = (node.location.x, node.location.y, node.location.z);
        }

        if max_displacement_sq < CONVERGENCE_THRESHOLD * CONVERGENCE_THRESHOLD {
            return step;
        }
    }

    MAX_STEPS
}

/// Collapses edges between the same pair of nodes down to one unordered
/// pair each, and drops self-loops entirely. Obsidian links are
/// directional, but a note reciprocally linking back (`A -> B` and
/// `B -> A`) is common and shouldn't pull the two notes together twice as
/// hard as a one-way link would: `fdg-sim`'s `ForceGraph` is an undirected
/// multigraph, so feeding it both directed edges would add two parallel
/// edges, and its Fruchterman-Reingold attraction force sums once per
/// parallel edge between neighbors.
///
/// Self-loops are dropped, not just deduped, because they are actively
/// destructive to the simulation, not merely redundant: a self-loop makes
/// a node its own neighbor, and `fdg-sim`'s attraction force divides by
/// the distance between a node and its neighbor — zero, for a node and
/// itself — producing `NaN` that spreads to every other node's position
/// within a handful of simulation steps (every node's repulsion/attraction
/// each step reads every other node's position). Confirmed directly: the
/// full `~/vaults/obg-test` fixture (which has a real self-loop,
/// `self-loop.md`) rendered all 14 positions as `NaN` before this fix.
/// A self-loop has no visible geometry in a wireframe regardless (see
/// TODO.md Phase 5), so there's nothing lost by excluding it here.
fn undirected_edge_pairs(graph: &Graph<Note, ()>) -> HashSet<(NodeIndex, NodeIndex)> {
    let mut pairs = HashSet::with_capacity(graph.edge_count());
    for edge in graph.edge_indices() {
        let (from, to) = graph.edge_endpoints(edge).unwrap();
        if from == to {
            continue;
        }
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
    fn every_position_is_finite() {
        // Regression test: a self-loop used to poison every node's
        // position with NaN (see `undirected_edge_pairs`'s doc comment) —
        // a bug the other tests here didn't catch because they only
        // checked position *count*, not that the values were usable.
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md"));
        let b = graph.add_node(note("b.md"));
        let c = graph.add_node(note("self-loop.md"));
        graph.add_edge(a, b, ());
        graph.add_edge(c, c, ());

        let positions = layout(&graph);

        for pos in positions.values() {
            assert!(pos.x.is_finite() && pos.y.is_finite() && pos.z.is_finite());
        }
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
    fn self_loops_are_excluded_from_the_physics_graph() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md"));
        let b = graph.add_node(note("b.md"));
        graph.add_edge(a, a, ());
        graph.add_edge(a, b, ());

        assert_eq!(undirected_edge_pairs(&graph), HashSet::from([(a, b)]));
    }

    #[test]
    fn self_loop_does_not_panic() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("self-loop.md"));
        graph.add_edge(a, a, ());

        let positions = layout(&graph);

        assert_eq!(positions.len(), 1);
    }

    /// A simple deterministic linear congruential generator, so the
    /// synthetic benchmark graphs below are reproducible without pulling in
    /// a `rand` dependency for a one-off perf measurement.
    struct Lcg(u64);

    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }

        fn next_usize(&mut self, bound: usize) -> usize {
            (self.next_u64() % bound as u64) as usize
        }
    }

    fn synthetic_graph(n: usize, avg_degree: usize, rng: &mut Lcg) -> Graph<Note, ()> {
        let mut graph = Graph::new();
        let nodes: Vec<NodeIndex> = (0..n)
            .map(|i| graph.add_node(note(&format!("note-{i}.md"))))
            .collect();
        for &from in &nodes {
            for _ in 0..avg_degree {
                let to = nodes[rng.next_usize(n)];
                if to != from {
                    graph.add_edge(from, to, ());
                }
            }
        }
        graph
    }

    /// TODO.md Phase 9's first task: real numbers on how the layout scales,
    /// before committing to a caching/threshold design. Not run as part of
    /// the normal suite (`#[ignore]`) — invoke explicitly with `cargo test
    /// --release -- --ignored --nocapture layout_benchmark`.
    ///
    /// Two measurements: (1) fixed-step cost, timed at a reduced step count
    /// and extrapolated to `MAX_STEPS` — running the full budget at every
    /// size would take far too long at the larger end (repulsion is a
    /// plain O(n^2) nested loop, no spatial partitioning — see CLAUDE.md),
    /// and this is valid because each step's cost is driven by node/edge
    /// count alone, not by how "settled" the layout already is; (2) actual
    /// `run_to_convergence` behavior — how many steps real graphs need
    /// before `CONVERGENCE_THRESHOLD` kicks in, and what that costs
    /// end-to-end, which is what a user actually experiences on a cache
    /// miss.
    #[test]
    #[ignore = "manual perf benchmark — cargo test --release -- --ignored --nocapture layout_benchmark"]
    fn layout_benchmark() {
        use std::time::Instant;

        const BENCH_STEPS: usize = 20;
        let mut rng = Lcg(0xC0FFEE);

        println!(
            "\nfixed-step timing (extrapolated to MAX_STEPS={MAX_STEPS} from {BENCH_STEPS}-step timings)"
        );
        println!("{:>8}  {:>10}  {:>16}", "nodes", "ms/step", "extrapolated ms");
        for &n in &[100usize, 500, 1000, 2000, 5000, 10000] {
            let graph = synthetic_graph(n, 3, &mut rng);
            let start = Instant::now();
            layout_with_steps(&graph, BENCH_STEPS);
            let elapsed = start.elapsed();
            let ms_per_step = elapsed.as_secs_f64() * 1000.0 / BENCH_STEPS as f64;
            println!(
                "{n:>8}  {:>10.3}  {:>16.0}",
                ms_per_step,
                ms_per_step * MAX_STEPS as f64
            );
        }

        println!("\nactual convergence-based layout() timing:");
        println!("{:>8}  {:>8}  {:>10}", "nodes", "steps", "ms");
        for &n in &[100usize, 500, 1000, 2000, 5000] {
            let graph = synthetic_graph(n, 3, &mut rng);
            let (mut simulation, _mapping) = build_simulation(&graph);
            let start = Instant::now();
            let steps = run_to_convergence(&mut simulation);
            let elapsed = start.elapsed();
            println!(
                "{n:>8}  {steps:>8}  {:>10.0}",
                elapsed.as_secs_f64() * 1000.0
            );
        }
    }
}
