use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::Direction;
use petgraph::graph::{Graph, NodeIndex};

use crate::vault::Note;

/// Tag marking a source-note hub (see CLAUDE.md's note-taking
/// conventions): every claim from a book links back to its source note,
/// making it trivially high-degree by construction, not because the
/// *book* is structurally load-bearing. Excluded from centrality/
/// community queries below so those rankings reflect argument structure
/// rather than "which book has the most notes."
const SOURCE_TAG: &str = "source";

const PAGERANK_DAMPING: f64 = 0.85;
const PAGERANK_MAX_ITERATIONS: usize = 100;
const PAGERANK_TOLERANCE: f64 = 1e-8;

/// Notes not tagged `source`.
fn claim_notes(graph: &Graph<Note, ()>) -> Vec<NodeIndex> {
    graph
        .node_indices()
        .filter(|&i| !graph[i].tags.iter().any(|t| t == SOURCE_TAG))
        .collect()
}

/// Ranks notes by PageRank (structural centrality) — a hand-rolled power
/// iteration over `graph` directly (see CLAUDE.md: an embedded Cypher
/// graph DB was tried for this phase and dropped over its C++ build
/// weight — no crate or external process here, just petgraph + std).
/// Restricted to non-`source`-tagged notes (see `SOURCE_TAG`) and
/// directed exactly as links are authored — unlike `shortest_path`/
/// `communities`, PageRank's meaning comes from link direction (A links
/// to B is A endorsing B), so it isn't given the undirected treatment
/// `layout`'s physics graph and the other two queries below use. Highest
/// rank first, truncated to `top`.
pub fn pagerank(graph: &Graph<Note, ()>, top: usize) -> Vec<(String, f64)> {
    let nodes = claim_notes(graph);
    let included: HashSet<NodeIndex> = nodes.iter().copied().collect();
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let index_of: HashMap<NodeIndex, usize> =
        nodes.iter().enumerate().map(|(i, &idx)| (idx, i)).collect();

    // out_targets[i] = local indices of i's outgoing neighbors, restricted
    // to included nodes, self-loops excluded — a note linking to itself
    // has no meaningful PageRank contribution, the same call
    // `layout::undirected_edge_pairs()` already makes for the physics
    // graph.
    let out_targets: Vec<Vec<usize>> = nodes
        .iter()
        .map(|&idx| {
            graph
                .neighbors_directed(idx, Direction::Outgoing)
                .filter(|&t| t != idx && included.contains(&t))
                .filter_map(|t| index_of.get(&t).copied())
                .collect()
        })
        .collect();
    let out_degree: Vec<usize> = out_targets.iter().map(Vec::len).collect();

    let mut rank = vec![1.0 / n as f64; n];
    for _ in 0..PAGERANK_MAX_ITERATIONS {
        // Dangling nodes (no outgoing edges) would otherwise leak rank
        // mass out of the system entirely; standard fix is to
        // redistribute their mass uniformly across every node.
        let dangling_mass: f64 = (0..n).filter(|&i| out_degree[i] == 0).map(|i| rank[i]).sum();
        let base =
            (1.0 - PAGERANK_DAMPING) / n as f64 + PAGERANK_DAMPING * dangling_mass / n as f64;

        let mut new_rank = vec![base; n];
        for i in 0..n {
            if out_degree[i] == 0 {
                continue;
            }
            let share = PAGERANK_DAMPING * rank[i] / out_degree[i] as f64;
            for &j in &out_targets[i] {
                new_rank[j] += share;
            }
        }

        let delta: f64 = rank.iter().zip(&new_rank).map(|(a, b)| (a - b).abs()).sum();
        rank = new_rank;
        if delta < PAGERANK_TOLERANCE {
            break;
        }
    }

    let mut ranked: Vec<(String, f64)> = nodes
        .iter()
        .enumerate()
        .map(|(i, &idx)| (graph[idx].path.to_string_lossy().into_owned(), rank[i]))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    ranked.truncate(top);
    ranked
}

/// Detects communities via Louvain modularity optimization (Blondel et
/// al. 2008) over the undirected, unweighted claim-note subgraph — same
/// `SOURCE_TAG` exclusion as `pagerank`, and the same undirected/
/// self-loop-free treatment `layout::undirected_edge_pairs()` gives the
/// physics graph, for the same reason given there: there's no typed-link
/// distinction yet (`TODO.md` Phase 10) between an argument's lineage and
/// its rebuttal chain, so direction isn't required for "which notes
/// cluster together." Returns `(community_id, member note paths)` pairs,
/// ordered by community id.
pub fn communities(graph: &Graph<Note, ()>) -> Vec<(usize, Vec<String>)> {
    let nodes = claim_notes(graph);
    let included: HashSet<NodeIndex> = nodes.iter().copied().collect();
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let index_of: HashMap<NodeIndex, usize> =
        nodes.iter().enumerate().map(|(i, &idx)| (idx, i)).collect();

    let mut seen_pairs: HashSet<(usize, usize)> = HashSet::new();
    let mut edges: Vec<(usize, usize, f64)> = Vec::new();
    for &idx in &nodes {
        let i = index_of[&idx];
        for neighbor in graph.neighbors_undirected(idx) {
            if neighbor == idx || !included.contains(&neighbor) {
                continue;
            }
            let j = index_of[&neighbor];
            let pair = (i.min(j), i.max(j));
            if seen_pairs.insert(pair) {
                edges.push((pair.0, pair.1, 1.0));
            }
        }
    }

    let assignment = louvain(n, &edges);

    // Compact community ids to a dense 0.. range in first-seen order.
    let mut id_map: HashMap<usize, usize> = HashMap::new();
    let mut groups: Vec<Vec<String>> = Vec::new();
    for (i, &idx) in nodes.iter().enumerate() {
        let raw_id = assignment[i];
        let id = *id_map.entry(raw_id).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[id].push(graph[idx].path.to_string_lossy().into_owned());
    }

    groups.into_iter().enumerate().collect()
}

/// One full run of the Louvain algorithm: repeated rounds of local-moving
/// (greedy modularity-gain node reassignment) followed by aggregating
/// each round's communities into super-nodes for the next round, until a
/// round produces no further improvement. Returns each original node's
/// final community id (not yet compacted/ordered — `communities()` does
/// that).
fn louvain(n: usize, edges: &[(usize, usize, f64)]) -> Vec<usize> {
    let mut node_to_final: Vec<usize> = (0..n).collect();
    let mut level_n = n;
    let mut level_edges = edges.to_vec();

    loop {
        let (community, improved) = local_moving(level_n, &level_edges);
        if !improved {
            break;
        }

        let mut id_map: HashMap<usize, usize> = HashMap::new();
        let mut next_id = 0;
        let compact: Vec<usize> = community
            .iter()
            .map(|&c| {
                *id_map.entry(c).or_insert_with(|| {
                    let id = next_id;
                    next_id += 1;
                    id
                })
            })
            .collect();

        for c in node_to_final.iter_mut() {
            *c = compact[*c];
        }

        if next_id == level_n {
            // No two nodes merged this round — further aggregation can't
            // help; stop rather than loop forever. (Shouldn't actually
            // trigger given `improved` was true, but cheap insurance.)
            break;
        }

        // Aggregate: one super-node per compacted community, edge
        // weights summed per community pair — including a self-loop per
        // community summing its own internal edges. Real Louvain keeps
        // these; they contribute to a super-node's own degree/modularity
        // term at the next level.
        let mut aggregated: HashMap<(usize, usize), f64> = HashMap::new();
        for &(a, b, w) in &level_edges {
            let (ca, cb) = (compact[a], compact[b]);
            let key = (ca.min(cb), ca.max(cb));
            *aggregated.entry(key).or_insert(0.0) += w;
        }

        level_edges = aggregated.into_iter().map(|((a, b), w)| (a, b, w)).collect();
        level_n = next_id;
    }

    node_to_final
}

/// A single Louvain "local moving" phase: repeatedly considers moving
/// each node into whichever neighboring community maximizes modularity
/// gain, until a full pass makes no moves. Standard Blondel et al. gain
/// formula, with `Q`'s common `1/(2m)` factor dropped since it doesn't
/// affect which candidate community scores highest for a given node.
fn local_moving(n: usize, edges: &[(usize, usize, f64)]) -> (Vec<usize>, bool) {
    let mut adjacency: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut self_loop_weight = vec![0.0; n];
    let mut total_weight = 0.0;
    for &(a, b, w) in edges {
        total_weight += w;
        if a == b {
            self_loop_weight[a] += w;
            continue;
        }
        adjacency[a].push((b, w));
        adjacency[b].push((a, w));
    }
    let m2 = 2.0 * total_weight;
    if m2 <= 0.0 {
        return ((0..n).collect(), false);
    }

    let degree: Vec<f64> = (0..n)
        .map(|i| adjacency[i].iter().map(|&(_, w)| w).sum::<f64>() + 2.0 * self_loop_weight[i])
        .collect();

    let mut community: Vec<usize> = (0..n).collect();
    let mut community_tot = degree.clone();
    let mut any_move = false;

    loop {
        let mut moved_this_pass = false;
        for i in 0..n {
            let current = community[i];

            let mut neighbor_weight: HashMap<usize, f64> = HashMap::new();
            for &(j, w) in &adjacency[i] {
                if j == i {
                    continue;
                }
                *neighbor_weight.entry(community[j]).or_insert(0.0) += w;
            }

            // Remove i from its current community before scoring
            // candidates, so "stay" is judged on equal footing with
            // every "move" candidate.
            community_tot[current] -= degree[i];

            let score = |comm: usize, k_in: f64| k_in - community_tot[comm] * degree[i] / m2;

            let mut best_comm = current;
            let mut best_score =
                score(current, neighbor_weight.get(&current).copied().unwrap_or(0.0));
            for (&comm, &k_in) in &neighbor_weight {
                let s = score(comm, k_in);
                if s > best_score {
                    best_score = s;
                    best_comm = comm;
                }
            }

            community_tot[best_comm] += degree[i];
            if best_comm != current {
                community[i] = best_comm;
                moved_this_pass = true;
                any_move = true;
            }
        }
        if !moved_this_pass {
            break;
        }
    }

    (community, any_move)
}

/// Traces the shortest connecting path between two notes, direction-blind
/// via `Graph::neighbors_undirected` (see CLAUDE.md: no typed-link
/// distinction yet between "lineage" and "rebuttal", and `layout` already
/// treats links this way via `undirected_edge_pairs()`). `from`/`to` are
/// exact note paths — the caller resolves user-typed names first (e.g.
/// via `vault::resolve::NoteIndex`). `None` means no path exists,
/// including either note not existing in the graph. Unlike
/// `pagerank`/`communities`, deliberately **not** restricted to
/// non-`source`-tagged notes — the user names both endpoints explicitly,
/// so excluding a source note here would just break legitimate queries
/// that happen to route through one.
pub fn shortest_path(graph: &Graph<Note, ()>, from: &str, to: &str) -> Option<Vec<String>> {
    let start = graph
        .node_indices()
        .find(|&i| graph[i].path.to_string_lossy() == from)?;
    let end = graph
        .node_indices()
        .find(|&i| graph[i].path.to_string_lossy() == to)?;

    if start == end {
        return Some(vec![graph[start].path.to_string_lossy().into_owned()]);
    }

    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut predecessor: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut queue: VecDeque<NodeIndex> = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);

    while let Some(current) = queue.pop_front() {
        if current == end {
            let mut path = vec![end];
            let mut node = end;
            while let Some(&prev) = predecessor.get(&node) {
                path.push(prev);
                node = prev;
            }
            path.reverse();
            return Some(
                path.into_iter()
                    .map(|idx| graph[idx].path.to_string_lossy().into_owned())
                    .collect(),
            );
        }
        for neighbor in graph.neighbors_undirected(current) {
            if neighbor == current || visited.contains(&neighbor) {
                continue;
            }
            visited.insert(neighbor);
            predecessor.insert(neighbor, current);
            queue.push_back(neighbor);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, tags: &[&str]) -> Note {
        Note {
            path: path.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            aliases: Vec::new(),
        }
    }

    #[test]
    fn pagerank_ranks_the_hub_of_a_star_graph_highest() {
        let mut graph = Graph::new();
        let hub = graph.add_node(note("hub.md", &[]));
        let leaves: Vec<_> = (0..4)
            .map(|i| graph.add_node(note(&format!("leaf-{i}.md"), &[])))
            .collect();
        for &leaf in &leaves {
            graph.add_edge(leaf, hub, ());
        }

        let ranked = pagerank(&graph, 10);
        assert_eq!(ranked.first().map(|(path, _)| path.as_str()), Some("hub.md"));
    }

    #[test]
    fn pagerank_excludes_source_tagged_notes() {
        let mut graph = Graph::new();
        let source = graph.add_node(note("book.md", &["source"]));
        let claim = graph.add_node(note("claim.md", &[]));
        graph.add_edge(claim, source, ());

        let ranked = pagerank(&graph, 10);
        assert!(ranked.iter().all(|(path, _)| path != "book.md"));
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn pagerank_self_loop_does_not_panic_or_produce_nan() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md", &[]));
        graph.add_edge(a, a, ());

        let ranked = pagerank(&graph, 10);
        assert_eq!(ranked.len(), 1);
        assert!(ranked[0].1.is_finite());
    }

    #[test]
    fn communities_separates_disconnected_clusters() {
        let mut graph = Graph::new();
        let a1 = graph.add_node(note("a1.md", &[]));
        let a2 = graph.add_node(note("a2.md", &[]));
        let b1 = graph.add_node(note("b1.md", &[]));
        let b2 = graph.add_node(note("b2.md", &[]));
        graph.add_edge(a1, a2, ());
        graph.add_edge(b1, b2, ());

        let groups = communities(&graph);
        let group_of = |path: &str| {
            groups
                .iter()
                .find(|(_, members)| members.iter().any(|m| m == path))
                .map(|(id, _)| *id)
        };
        assert_eq!(group_of("a1.md"), group_of("a2.md"));
        assert_eq!(group_of("b1.md"), group_of("b2.md"));
        assert_ne!(group_of("a1.md"), group_of("b1.md"));
    }

    #[test]
    fn communities_separates_two_triangles_joined_by_a_bridge() {
        let mut graph = Graph::new();
        let tri_a: Vec<_> = (0..3)
            .map(|i| graph.add_node(note(&format!("a{i}.md"), &[])))
            .collect();
        let tri_b: Vec<_> = (0..3)
            .map(|i| graph.add_node(note(&format!("b{i}.md"), &[])))
            .collect();
        for i in 0..3 {
            graph.add_edge(tri_a[i], tri_a[(i + 1) % 3], ());
            graph.add_edge(tri_b[i], tri_b[(i + 1) % 3], ());
        }
        graph.add_edge(tri_a[0], tri_b[0], ()); // single bridge edge

        let groups = communities(&graph);
        let group_of = |path: &str| {
            groups
                .iter()
                .find(|(_, members)| members.iter().any(|m| m == path))
                .map(|(id, _)| *id)
        };
        assert_eq!(group_of("a0.md"), group_of("a1.md"));
        assert_eq!(group_of("a1.md"), group_of("a2.md"));
        assert_eq!(group_of("b0.md"), group_of("b1.md"));
        assert_eq!(group_of("b1.md"), group_of("b2.md"));
        assert_ne!(group_of("a0.md"), group_of("b0.md"));
    }

    #[test]
    fn shortest_path_finds_a_chain_and_reports_none_when_disconnected() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md", &[]));
        let b = graph.add_node(note("b.md", &[]));
        let c = graph.add_node(note("c.md", &[]));
        let orphan = graph.add_node(note("orphan.md", &[]));
        graph.add_edge(a, b, ());
        graph.add_edge(b, c, ());
        let _ = orphan;

        let path = shortest_path(&graph, "a.md", "c.md");
        assert_eq!(
            path,
            Some(vec!["a.md".to_string(), "b.md".to_string(), "c.md".to_string()])
        );

        assert_eq!(shortest_path(&graph, "a.md", "orphan.md"), None);
        assert_eq!(shortest_path(&graph, "a.md", "nonexistent.md"), None);
    }

    #[test]
    fn shortest_path_is_direction_blind() {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md", &[]));
        let b = graph.add_node(note("b.md", &[]));
        graph.add_edge(b, a, ()); // b -> a, not a -> b

        assert_eq!(
            shortest_path(&graph, "a.md", "b.md"),
            Some(vec!["a.md".to_string(), "b.md".to_string()])
        );
    }
}
