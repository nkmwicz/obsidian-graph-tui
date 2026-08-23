use std::sync::LazyLock;

use regex::Regex;

static FENCED_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```.*?```").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!?)\[\[([^\]|#]+)(?:#([^\]|]*))?(?:\|([^\]]*))?\]\]").unwrap()
});
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]\(([^)]+)\)").unwrap());

#[derive(Debug, PartialEq, Eq)]
pub struct RawLink {
    pub target: String,
    pub is_embed: bool,
}

/// Removes fenced code blocks and inline code spans so `[[...]]`-looking
/// text used as documentation (e.g. in this vault's own fixture notes)
/// isn't mistaken for a real link. Approximates CommonMark's actual rule,
/// not a full parser.
fn strip_code_spans(body: &str) -> String {
    let no_fences = FENCED_CODE.replace_all(body, "");
    INLINE_CODE.replace_all(&no_fences, "").into_owned()
}

/// Extracts `[[wikilinks]]` (with optional `#heading`/`|alias`, and an
/// optional leading `!` for embeds) and plain markdown `[text](path)`
/// links from a note body. External URLs (anything containing `://`) are
/// dropped — they never become graph edges.
pub fn extract_links(body: &str) -> Vec<RawLink> {
    let cleaned = strip_code_spans(body);
    let mut links = Vec::new();

    for cap in WIKILINK.captures_iter(&cleaned) {
        let is_embed = &cap[1] == "!";
        let target = cap[2].trim().to_string();
        links.push(RawLink { target, is_embed });
    }

    for cap in MARKDOWN_LINK.captures_iter(&cleaned) {
        let target = cap[1].trim();
        if target.contains("://") {
            continue;
        }
        links.push(RawLink {
            target: target.to_string(),
            is_embed: false,
        });
    }

    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_wikilink() {
        let links = extract_links("see [[project-alpha]] for details");
        assert_eq!(
            links,
            vec![RawLink {
                target: "project-alpha".into(),
                is_embed: false
            }]
        );
    }

    #[test]
    fn wikilink_with_alias() {
        let links = extract_links("[[project-alpha|Project A]]");
        assert_eq!(links[0].target, "project-alpha");
    }

    #[test]
    fn wikilink_with_heading() {
        let links = extract_links("[[projects/project-beta#Status]]");
        assert_eq!(links[0].target, "projects/project-beta");
    }

    #[test]
    fn wikilink_with_heading_and_alias() {
        let links = extract_links("[[projects/project-beta#Status|Beta]]");
        assert_eq!(links[0].target, "projects/project-beta");
    }

    #[test]
    fn embed() {
        let links = extract_links("![[diagram.png]]");
        assert_eq!(
            links,
            vec![RawLink {
                target: "diagram.png".into(),
                is_embed: true
            }]
        );
    }

    #[test]
    fn backtick_wrapped_wikilink_is_not_extracted() {
        let links = extract_links("documented as `[[CaseTest]]` in the text");
        assert!(links.is_empty());
    }

    #[test]
    fn fenced_code_block_wikilink_is_not_extracted() {
        let body = "before\n```\n[[fake-link]]\n```\nafter [[real-link]]";
        let links = extract_links(body);
        assert_eq!(links, vec![RawLink {
            target: "real-link".into(),
            is_embed: false
        }]);
    }

    #[test]
    fn markdown_link_is_extracted() {
        let links = extract_links("[Project Alpha](projects/project-alpha.md)");
        assert_eq!(
            links,
            vec![RawLink {
                target: "projects/project-alpha.md".into(),
                is_embed: false
            }]
        );
    }

    #[test]
    fn external_url_is_not_extracted() {
        let links = extract_links("[Example](https://example.com)");
        assert!(links.is_empty());
    }
}
