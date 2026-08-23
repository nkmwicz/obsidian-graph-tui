mod frontmatter;
mod links;
pub(crate) mod resolve;

use std::path::{Path, PathBuf};

use resolve::{NoteIndex, Outcome};

pub struct Note {
    pub path: PathBuf,
    // Captured since Phase 2; consumed starting Phase 6 (excluding
    // `source`-tagged hub notes from centrality/community queries — see
    // CLAUDE.md's note-taking conventions). Phase 8's tag/folder view
    // filter is a separate, later consumer of the same field.
    pub tags: Vec<String>,
    #[allow(dead_code)]
    pub aliases: Vec<String>,
}

pub struct Edge {
    pub from: usize,
    pub to: usize,
}

pub struct ParsedVault {
    pub notes: Vec<Note>,
    pub edges: Vec<Edge>,
    pub unresolved_links: usize,
}

impl ParsedVault {
    /// Notes with no incoming or outgoing edges.
    pub fn orphan_count(&self) -> usize {
        let mut degree = vec![0usize; self.notes.len()];
        for edge in &self.edges {
            degree[edge.from] += 1;
            degree[edge.to] += 1;
        }
        degree.iter().filter(|&&d| d == 0).count()
    }
}

/// Walks `vault_root`, parsing every `.md` file into a `Note` and
/// resolving every link found into an `Edge`. Skips `.obsidian/` and
/// `.git/` entirely. Errors if the vault contains zero markdown notes —
/// path validation (Phase 1) only checks the path is a directory, so a
/// non-vault directory needs to be caught here instead of silently
/// producing an empty graph.
pub fn parse(vault_root: &Path) -> Result<ParsedVault, String> {
    let mut notes = Vec::new();
    let mut bodies = Vec::new();

    let walker = walkdir::WalkDir::new(vault_root)
        .into_iter()
        .filter_entry(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name != ".obsidian" && name != ".git")
                .unwrap_or(true)
        });

    for entry in walker {
        let entry = entry.map_err(|e| format!("error walking vault: {e}"))?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let is_markdown = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if !is_markdown {
            continue;
        }

        let relative = path.strip_prefix(vault_root).unwrap_or(path).to_path_buf();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let (front, body) = frontmatter::parse(&raw);

        notes.push(Note {
            path: relative,
            tags: front.tags,
            aliases: front.aliases,
        });
        bodies.push(body);
    }

    if notes.is_empty() {
        return Err(format!(
            "vault contains no markdown notes: {}",
            vault_root.display()
        ));
    }

    let note_paths: Vec<&Path> = notes.iter().map(|n| n.path.as_path()).collect();
    let index = NoteIndex::build(&note_paths);

    let mut edges = Vec::new();
    let mut unresolved_links = 0;

    for (from, body) in bodies.iter().enumerate() {
        for raw_link in links::extract_links(body) {
            match index.resolve(&raw_link.target) {
                Outcome::Resolved(to) => edges.push(Edge { from, to }),
                Outcome::Attachment => {}
                Outcome::Unresolved => unresolved_links += 1,
            }
        }
    }

    Ok(ParsedVault {
        notes,
        edges,
        unresolved_links,
    })
}
