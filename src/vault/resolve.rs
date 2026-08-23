use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Resolved(usize),
    /// A link target with a non-`.md` extension (e.g. `diagram.png`) —
    /// a real file may or may not exist, but it isn't a note and never
    /// becomes a graph edge or counts as a broken link.
    Attachment,
    Unresolved,
}

/// Maps link targets to note indices for resolution. Case-insensitive
/// throughout, matching Obsidian's own link resolution behavior.
pub struct NoteIndex {
    /// Lowercased relative path (no extension), indexed by note index —
    /// used to break ties on an ambiguous basename.
    paths: Vec<String>,
    by_path: HashMap<String, usize>,
    by_basename: HashMap<String, Vec<usize>>,
}

impl NoteIndex {
    pub fn build<P: AsRef<Path>>(note_paths: &[P]) -> Self {
        let mut paths = Vec::with_capacity(note_paths.len());
        let mut by_path = HashMap::new();
        let mut by_basename: HashMap<String, Vec<usize>> = HashMap::new();

        for (i, p) in note_paths.iter().enumerate() {
            let stem_path = p.as_ref().with_extension("");
            let key = stem_path.to_string_lossy().to_lowercase();

            let basename = stem_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();

            by_path.insert(key.clone(), i);
            by_basename.entry(basename).or_default().push(i);
            paths.push(key);
        }

        Self {
            paths,
            by_path,
            by_basename,
        }
    }

    /// Resolves a raw link target (already stripped of any `#heading` /
    /// `|alias` suffix) to a note index, an attachment, or unresolved.
    pub fn resolve(&self, target: &str) -> Outcome {
        let target = target.trim();
        let path = Path::new(target);

        let stripped = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("md") => &target[..target.len() - ext.len() - 1],
            Some(_) => return Outcome::Attachment,
            None => target,
        };

        let key = stripped.to_lowercase();

        if key.contains('/') {
            return self
                .by_path
                .get(&key)
                .map(|&i| Outcome::Resolved(i))
                .unwrap_or(Outcome::Unresolved);
        }

        match self.by_basename.get(&key) {
            None => Outcome::Unresolved,
            Some(indices) if indices.len() == 1 => Outcome::Resolved(indices[0]),
            // Ambiguous basename: resolve to whichever candidate's full
            // relative path sorts first alphabetically. Deterministic,
            // not fully Obsidian-faithful, documented in TODO.md/CLAUDE.md.
            Some(indices) => {
                let winner = indices.iter().min_by_key(|&&i| &self.paths[i]).copied().unwrap();
                Outcome::Resolved(winner)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> NoteIndex {
        NoteIndex::build(&[
            "index.md",
            "projects/project-alpha.md",
            "projects/project-beta.md",
            "duplicate-name/project-alpha.md",
        ])
    }

    #[test]
    fn exact_path_match() {
        assert_eq!(
            index().resolve("projects/project-beta"),
            Outcome::Resolved(2)
        );
    }

    #[test]
    fn unambiguous_basename() {
        assert_eq!(index().resolve("index"), Outcome::Resolved(0));
    }

    #[test]
    fn ambiguous_basename_resolves_alphabetically_first() {
        // "duplicate-name/project-alpha" < "projects/project-alpha"
        assert_eq!(
            index().resolve("project-alpha"),
            Outcome::Resolved(3)
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(index().resolve("INDEX"), Outcome::Resolved(0));
        assert_eq!(
            index().resolve("Projects/Project-Beta"),
            Outcome::Resolved(2)
        );
    }

    #[test]
    fn unresolved_target() {
        assert_eq!(index().resolve("nonexistent-note"), Outcome::Unresolved);
    }

    #[test]
    fn non_md_extension_is_an_attachment_not_unresolved() {
        assert_eq!(index().resolve("diagram.png"), Outcome::Attachment);
    }

    #[test]
    fn md_extension_is_stripped_and_resolved() {
        assert_eq!(
            index().resolve("projects/project-alpha.md"),
            Outcome::Resolved(1)
        );
    }
}
