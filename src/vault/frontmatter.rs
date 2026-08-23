use gray_matter::Matter;
use gray_matter::engine::YAML;
use serde::Deserialize;

#[derive(Deserialize, Default, Debug, PartialEq)]
pub struct FrontMatter {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Splits a raw note into (frontmatter, body). A missing or malformed
/// frontmatter block is not fatal — it just yields an empty `FrontMatter`
/// and the raw text is still returned so links can be extracted from it.
pub fn parse(raw: &str) -> (FrontMatter, String) {
    let matter: Matter<YAML> = Matter::new();

    match matter.parse::<FrontMatter>(raw) {
        Ok(entity) => (entity.data.unwrap_or_default(), entity.content),
        Err(_) => (FrontMatter::default(), raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_and_aliases() {
        let raw = "---\ntags:\n  - a\n  - b\naliases:\n  - Alias\n---\nbody text";
        let (front, body) = parse(raw);
        assert_eq!(front.tags, vec!["a", "b"]);
        assert_eq!(front.aliases, vec!["Alias"]);
        assert_eq!(body, "body text");
    }

    #[test]
    fn missing_frontmatter_is_not_an_error() {
        let raw = "just plain content, no frontmatter";
        let (front, body) = parse(raw);
        assert_eq!(front, FrontMatter::default());
        assert_eq!(body, raw);
    }

    #[test]
    fn frontmatter_without_tags_or_aliases_defaults_to_empty() {
        let raw = "---\nid: some-note\n---\nbody";
        let (front, _) = parse(raw);
        assert!(front.tags.is_empty());
        assert!(front.aliases.is_empty());
    }

    #[test]
    fn malformed_yaml_falls_back_gracefully() {
        let raw = "---\nkey: [unclosed\n---\nbody text";
        let (front, body) = parse(raw);
        assert_eq!(front, FrontMatter::default());
        // We don't get gray_matter's stripped body back on a parse error,
        // so we fall back to the raw text — links inside it are still
        // extractable rather than losing the note entirely.
        assert_eq!(body, raw);
    }
}
