use std::collections::HashMap;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use petgraph::graph::{Graph, NodeIndex};
use serde::{Deserialize, Serialize};

use super::{Position, layout};
use crate::vault::Note;

#[derive(Serialize, Deserialize)]
struct CachedLayout {
    structure_hash: u64,
    positions: Vec<CachedPosition>,
}

#[derive(Serialize, Deserialize)]
struct CachedPosition {
    path: String,
    x: f32,
    y: f32,
    z: f32,
}

/// FNV-1a, hand-rolled rather than pulling in a hashing crate: this is
/// purely a cache-invalidation key, not anything security-sensitive, and
/// the cost of a hash collision or of the algorithm changing across a
/// future toolchain upgrade is just a spurious cache miss (recompute),
/// never a correctness problem.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    bytes
        .iter()
        .fold(OFFSET, |hash, &b| (hash ^ b as u64).wrapping_mul(PRIME))
}

/// Hashes the graph's resolved node/edge structure — note paths and edge
/// pairs, sorted so the hash doesn't depend on incidental filesystem walk
/// order — so an unchanged vault produces the same key on every run and
/// any real change (a note added, removed, or relinked) invalidates the
/// cache. Deliberately not based on file mtimes (see TODO.md Phase 9):
/// those are fragile against touches/moves that don't change content.
fn structure_hash(graph: &Graph<Note, ()>) -> u64 {
    let mut paths: Vec<String> = graph
        .node_weights()
        .map(|n| n.path.to_string_lossy().into_owned())
        .collect();
    paths.sort_unstable();

    let mut edges: Vec<(String, String)> = graph
        .edge_indices()
        .map(|e| {
            let (from, to) = graph.edge_endpoints(e).unwrap();
            (
                graph[from].path.to_string_lossy().into_owned(),
                graph[to].path.to_string_lossy().into_owned(),
            )
        })
        .collect();
    edges.sort_unstable();

    let mut buf = String::new();
    for p in &paths {
        buf.push_str(p);
        buf.push('\n');
    }
    buf.push_str("--edges--\n");
    for (a, b) in &edges {
        buf.push_str(a);
        buf.push('\t');
        buf.push_str(b);
        buf.push('\n');
    }

    fnv1a(buf.as_bytes())
}

/// Where a given vault's cached layout lives: `~/.cache/obg/layout-
/// <hash-of-vault-path>.toml` on Linux (via `directories`' XDG
/// conventions, same crate `config::config_path` already uses), the
/// platform equivalent elsewhere. Keying the filename off the vault's own
/// path (rather than one shared file) lets multiple vaults cache
/// independently.
fn cache_path(vault_path: &Path) -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "obg")?;
    let vault_key = fnv1a(vault_path.to_string_lossy().as_bytes());
    Some(dirs.cache_dir().join(format!("layout-{vault_key:016x}.toml")))
}

/// Returns cached positions for `graph` if a cache exists for `vault_path`
/// and its structure hash matches the current graph exactly; otherwise
/// runs the simulation fresh (`layout::layout`) and writes a new cache for
/// next time. A cache read/write failure (missing file, unwritable cache
/// dir, corrupt contents) is never treated as an error — caching is a
/// pure performance optimization on top of `layout`, not a new
/// correctness requirement, so any failure just falls back to recomputing.
pub fn load_or_compute(graph: &Graph<Note, ()>, vault_path: &Path) -> HashMap<NodeIndex, Position> {
    let hash = structure_hash(graph);
    let path = cache_path(vault_path);

    if let Some(path) = &path
        && let Some(positions) = try_load(path, hash, graph)
    {
        return positions;
    }

    let positions = layout(graph);

    if let Some(path) = &path {
        save(path, hash, graph, &positions);
    }

    positions
}

fn try_load(path: &Path, hash: u64, graph: &Graph<Note, ()>) -> Option<HashMap<NodeIndex, Position>> {
    let contents = std::fs::read_to_string(path).ok()?;
    let cached: CachedLayout = toml::from_str(&contents).ok()?;
    if cached.structure_hash != hash {
        return None;
    }

    let by_path: HashMap<&str, &CachedPosition> = cached
        .positions
        .iter()
        .map(|p| (p.path.as_str(), p))
        .collect();

    let mut result = HashMap::with_capacity(graph.node_count());
    for idx in graph.node_indices() {
        let note_path = graph[idx].path.to_str()?;
        let cached_pos = by_path.get(note_path)?;
        result.insert(
            idx,
            Position {
                x: cached_pos.x,
                y: cached_pos.y,
                z: cached_pos.z,
            },
        );
    }

    Some(result)
}

fn save(path: &Path, hash: u64, graph: &Graph<Note, ()>, positions: &HashMap<NodeIndex, Position>) {
    let cached = CachedLayout {
        structure_hash: hash,
        positions: graph
            .node_indices()
            .filter_map(|idx| {
                let pos = positions.get(&idx)?;
                Some(CachedPosition {
                    path: graph[idx].path.to_string_lossy().into_owned(),
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                })
            })
            .collect(),
    };

    let Ok(serialized) = toml::to_string(&cached) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serialized);
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

    fn sample_graph() -> Graph<Note, ()> {
        let mut graph = Graph::new();
        let a = graph.add_node(note("a.md"));
        let b = graph.add_node(note("b.md"));
        let c = graph.add_node(note("c.md"));
        graph.add_edge(a, b, ());
        graph.add_edge(b, c, ());
        graph
    }

    #[test]
    fn structure_hash_is_stable_across_calls() {
        let graph = sample_graph();
        assert_eq!(structure_hash(&graph), structure_hash(&graph));
    }

    #[test]
    fn structure_hash_is_independent_of_node_insertion_order() {
        let mut a = Graph::new();
        let a1 = a.add_node(note("a.md"));
        let a2 = a.add_node(note("b.md"));
        a.add_edge(a1, a2, ());

        let mut b = Graph::new();
        let b2 = b.add_node(note("b.md"));
        let b1 = b.add_node(note("a.md"));
        b.add_edge(b1, b2, ());

        assert_eq!(structure_hash(&a), structure_hash(&b));
    }

    #[test]
    fn structure_hash_changes_when_a_note_is_added() {
        let graph = sample_graph();
        let mut changed = sample_graph();
        changed.add_node(note("d.md"));

        assert_ne!(structure_hash(&graph), structure_hash(&changed));
    }

    #[test]
    fn structure_hash_changes_when_an_edge_changes() {
        let graph = sample_graph();
        let mut changed = sample_graph();
        let a = changed.node_indices().next().unwrap();
        let c = changed.node_indices().next_back().unwrap();
        changed.add_edge(a, c, ());

        assert_ne!(structure_hash(&graph), structure_hash(&changed));
    }

    #[test]
    fn round_trips_positions_through_a_temp_cache_file() {
        let graph = sample_graph();
        let hash = structure_hash(&graph);
        let mut positions = HashMap::new();
        for (i, idx) in graph.node_indices().enumerate() {
            positions.insert(
                idx,
                Position {
                    x: i as f32,
                    y: i as f32 * 2.0,
                    z: i as f32 * 3.0,
                },
            );
        }

        let dir = std::env::temp_dir().join(format!(
            "obg-cache-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layout-test.toml");

        save(&path, hash, &graph, &positions);
        let loaded = try_load(&path, hash, &graph).expect("cache should load back");

        for idx in graph.node_indices() {
            assert_eq!(loaded[&idx], positions[&idx]);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hash_mismatch_is_treated_as_a_cache_miss() {
        let graph = sample_graph();
        let hash = structure_hash(&graph);
        let positions: HashMap<NodeIndex, Position> = graph
            .node_indices()
            .map(|idx| (idx, Position { x: 0.0, y: 0.0, z: 0.0 }))
            .collect();

        let dir = std::env::temp_dir().join(format!(
            "obg-cache-test-mismatch-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("layout-test.toml");

        save(&path, hash, &graph, &positions);
        assert!(try_load(&path, hash.wrapping_add(1), &graph).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
