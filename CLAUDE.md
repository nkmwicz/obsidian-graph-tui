# obsidian-graph-tui — Project Brief

## What this is

A terminal-native, genuinely 3D, interactively navigable graph visualization
of an Obsidian vault's note links — with a real graph model behind it
(traversal/filtering, not just a static picture). Written in Rust.

## Why (context for judgment calls)

- The user is terminal/nvim-first and actively avoids context-switching to
  GUI apps. Notes are edited directly in nvim (via `obsidian.nvim`, which
  provides LSP-backed wiki-link completion, backlinks via `grr`, rename via
  `grn`, daily notes, etc.) against a vault that's just a folder of plain
  markdown files.
- The vault syncs via `obsidian-headless` (a CLI-only Obsidian Sync client:
  `ob login` / `ob sync-create-remote` / `ob sync-setup` / `ob sync`), not
  the desktop app's built-in sync. The desktop app, when opened at all, has
  Sync toggled off for this vault and is used only as an occasional local
  GUI viewer (graph view, Dataview) — nothing in this project should assume
  Obsidian.app is running.
- Obsidian's own graph view is 2D. A "3D Graph" community plugin exists but
  is Electron/WebGL-based — no terminal equivalent. This project's whole
  point is closing that gap natively in a terminal.
- "True graph query" matters here specifically: the user wants actual
  traversal (N-hop neighborhoods, filters) backing the visualization, not
  just a static rendered export.

## Architecture (decided, in order of layers)

1. **Parser** (first-party, nothing exists off the shelf for this; done —
   `src/vault/`): walk the vault's `.md` files (`walkdir`, skipping
   `.obsidian/`/`.git/`), extract `[[wikilink]]` targets (with optional
   `#heading`/`|alias`, and a leading `!` for embeds) plus plain markdown
   `[text](file.md)` links, build a node/edge list. Not a CommonMark
   construct, so a CommonMark parser (`pulldown-cmark`) wouldn't catch
   wikilinks anyway — hand-rolled `regex` over the raw file text.
   `gray_matter` for YAML frontmatter (`tags:`, `aliases:`) — captured now,
   consumed starting Phase 7 (filter by tag/folder).
   - **Code-span stripping is required, not optional**: a naive regex scan
     can't tell a real `[[wikilink]]` from one written as
     `` `[[documentation]]` `` inside backticks. Fenced blocks and inline
     spans are stripped before the link regexes run — discovered because
     the fixture vault's own documentation notes (`case-test.md`,
     `_fixture-notes.md`) do exactly that, and would otherwise be
     misparsed.
   - **Link resolution is case-insensitive.** Path-qualified targets
     (contain `/`) match exactly; bare basenames match by filename alone.
     An ambiguous bare basename (two notes share it, e.g. the fixture
     vault's two `project-alpha.md` files) resolves to whichever
     candidate's full relative path sorts first alphabetically —
     deterministic, simple, not fully Obsidian-faithful, stated once.
   - **Target classification is by extension**: no extension or `.md` →
     note reference (resolved/unresolved); any other extension (`.png`,
     etc.) → attachment, never an edge, never counted as a broken link.
2. **Graph model / query**: [`petgraph`](https://github.com/petgraph/petgraph)
   (confirmed on crates.io: 0.8.3, actively maintained) — in-memory graph
   with real traversal (BFS/DFS, shortest path, connected components, N-hop
   neighborhoods). **Not Kùzu for v1** — deliberately deferred, see
   "Explicitly out of scope" below. Swappable later without touching the
   layout or rendering layers.
   - **`ParsedVault` vs `petgraph::Graph` (resolved in Phase 3):** `Note`
     (path/tags/aliases) is the node weight — `Graph<Note, ()>` — rather
     than keeping `ParsedVault` alive as a second, parallel lookup
     structure. `ParsedVault` (`src/vault/mod.rs`) stays exactly what it
     already is: the parser's output type, nothing more. A one-time
     `graph::build(ParsedVault) -> Graph<Note, ()>` owns the
     `Vec<Note>` index → `NodeIndex` mapping. Everything from Phase 4
     onward (layout, rendering, query) operates on the `petgraph::Graph`
     and never reaches back into `ParsedVault` — metadata needed for
     filtering (Phase 7) reads off the node weight, not a side table.
3. **3D layout**: [`fdg-sim`](https://crates.io/crates/fdg-sim) — **not
   `fdg`**, that crate name doesn't exist on crates.io. `fdg-sim` 0.9.1 is
   the actual published crate (from the `grantshandy/fdg` GitHub repo's
   `old` branch); last published Dec 2022 but verified working: it has
   `Dimensions::Two`/`Dimensions::Three` and converts directly from a
   `petgraph::Graph`. The `grantshandy/fdg` repo has since been rewritten
   into a new unified `fdg` crate (nalgebra-based, const-generic over N
   dimensions, still actively developed as of March 2025) — but that
   rewrite has **never been published to crates.io**, only usable via a
   `git = "..."` Cargo dependency. Use published `fdg-sim = "0.9"` for v1 to
   avoid a git dependency; revisit the rewrite only if `fdg-sim` proves
   insufficient.
4. **Rendering**: [`ratatui`](https://ratatui.rs/) (confirmed: 0.30.2,
   actively maintained), using the `Canvas` widget's `Marker::Braille` mode
   (2×4 sub-cell resolution — no GPU, no terminal graphics protocol
   dependency, works in any terminal, not just Kitty). Reference
   implementation to study first: ratatui's own `volatility-surface`
   example (`examples/apps/volatility-surface` in the ratatui repo, still
   present in the current tree) — it already does perspective projection +
   interactive rotate/zoom via Canvas+Braille. **Decision: hand-roll
   projection math following that example rather than depending on
   `ratatui-3d` or `ratatui-wireframe`.** Both were considered and rejected:
   both are tiny/unproven (525 and ~400 downloads, `ratatui-3d` first
   published ~March 2026, `ratatui-wireframe` ~June 2026), and
   `ratatui-wireframe` specifically has a red flag — its crates.io
   `repository` field points to an unrelated GitHub repo
   (`Vaishnav-Sabari-Girish/ComChan`, an unrelated serial-comm tool), which
   means either bad metadata or a low-quality/squatted crate. Don't add
   either without re-vetting.
5. **Input**: `crossterm` (confirmed: 0.29.0, actively maintained; also a
   transitive dep of `ratatui`) for live keyboard-driven camera
   orbit/zoom/pan.

> Crate names/versions above were verified against crates.io + GitHub on
> 2026-08-22 (not just identified via web research) — see corrections to
> `fdg`→`fdg-sim` and the rejection of `ratatui-3d`/`ratatui-wireframe`
> above. Versions will drift over time; re-check before pinning in
> `Cargo.toml` if this file is stale relative to the date above.

## MVP scope, in order

1. Parse a given vault path into a `petgraph` graph.
2. Force-directed 3D layout via `fdg-sim`.
3. Static rendered view in `ratatui` (Braille wireframe, no interaction yet)
   — prove the rendering pipeline end to end first.
4. Add camera controls (orbit/zoom via keyboard).
5. Add basic query/traversal: local graph around one note (N-hop
   neighborhood), filter by tag/folder.
6. *(Later, not v1 — but a confirmed direction, not just a maybe)*
   Introduce an embedded graph DB ([Kùzu](https://kuzudb.github.io/docs/) —
   Cypher, single-file, no server, native Rust bindings) alongside or in
   place of `petgraph`, to expose ad-hoc Cypher querying from within the
   TUI. See `TODO.md` Phase 8 — scoped further once Phases 0–7 there surface
   concrete query needs `petgraph` traversal can't cover.

## Explicitly out of scope for v1

- Kùzu/Cypher querying for v1 specifically — see above, it's a confirmed
  future phase (`TODO.md` Phase 8), just not part of the initial build.
- Dataview-equivalent dynamic queries, or Obsidian plugin-ecosystem parity.
- Pre-rendered rotating-frame animation tricks for terminals without Braille
  support — not worth the complexity for a personal tool.
- Kitty-graphics-protocol static 2D rendering (`neato -Tkitty` piped from
  Graphviz) — an earlier, simpler idea explored before "true 3D + true
  query" became the actual requirement. Worth knowing it exists as a
  fallback if the ratatui/Braille approach hits a wall, but not the plan.

## Environment already provisioned (via `~/dotfiles/install.sh`)

- `rustup` (default profile: `rustc`/`cargo`/`clippy`/`rustfmt`) plus the
  `rust-analyzer` component for nvim's LSP.
- `bacon` (background check/test runner, rerun on save) and `cargo-nextest`
  for the day-to-day dev loop.
- `gcc`/`make` already present as the linker — no networking crates are
  expected in this stack, so no OpenSSL/`pkg-config` dependency anticipated.

## Repo / naming

- Repo name: **`obsidian-graph-tui`** — chosen over shorter options (`obg`,
  `orbit`, `constellation`) for discoverability if this ever gets shared
  publicly.
- Consider giving the built binary a short name for daily typing (e.g.
  `obg`, via a `[[bin]] name = "obg"` in `Cargo.toml`, or just a shell
  alias) — doesn't need to match the repo name.
- Deliberately a **separate repo** from `~/dotfiles`, not a subfolder of it.
  Reasoning from the planning conversation: dotfiles is explicitly
  non-build/config-only (symlink-and-done, no CI, no compiled artifacts),
  and a real Cargo project brings its own build/test/lint/dependency-churn
  lifecycle that doesn't fit there. This decision is reversible later via
  `git subtree`/`filter-repo` if the boundary ever needs to move, but start
  separate.
- Once there's a working `cargo build --release` binary, `~/dotfiles`'s
  `install.sh` should get a step to clone + build + install it — mirroring
  the existing `fancy-cat` pattern there (git clone --depth 1 into a tmp
  dir, build, move the binary to `~/.local/bin`, clean up). That's
  dotfiles' job to add later, not something this repo needs to do itself.

## Vault access

- Treat the vault purely as a folder of markdown files on disk — no
  assumption that Obsidian.app (desktop or headless) is running while this
  tool runs.
- Vault path must be configurable (CLI arg and/or config file), not
  hardcoded to any one location.
- **Rejected: `obsidian-cli`.** Originally considered as the vault-access
  layer, but it requires an actual Obsidian.app install to function against
  — a hard dependency this project explicitly avoids (see above: nothing
  here should assume Obsidian.app is running). Reading the vault directly
  off disk with first-party Rust parsing keeps the tool fast and removes
  that dependency entirely; this is why the parser is first-party rather
  than a wrapper around existing Obsidian tooling.
- **Test vault**: `~/vaults/obg-test/` — a purpose-built fixture vault
  (separate from the user's real vaults also under `~/vaults/`), matching
  real vault frontmatter conventions (`id`/`aliases`/`tags`). Covers the
  parser edge cases deliberately: orphan node, broken/unresolved link,
  self-loop, missing frontmatter, case-mismatched links, alias/heading/embed
  link forms, plain markdown links (incl. one external URL that must NOT
  become a graph edge), a duplicate basename in two folders (ambiguous
  resolution), and an empty `.obsidian/` dir that must be skipped during
  traversal. Full legend of what each file tests is in the vault itself at
  `~/vaults/obg-test/_fixture-notes.md`. Use this vault for Phase 2+
  parser development and testing (see `TODO.md`).

## TODO.md is the source of direction — keep it current

`TODO.md` (repo root) is the authoritative, living roadmap: a phased,
checkable to-do list from the hello-world CLI through Cypher querying. This
file (`CLAUDE.md`) holds the *why* and the architectural decisions; `TODO.md`
holds *what's next and what's done*. When the two would otherwise disagree
on sequencing, `TODO.md` wins — update this file's "MVP scope" section to
match rather than letting them drift apart.

- At the start of any work session, check `TODO.md` to see which phase is
  in progress or next — don't re-derive the plan from conversation memory.
- Update `TODO.md` in the same session as the work it describes: check off
  completed items, add sub-tasks discovered mid-work, adjust phase
  boundaries if reality diverges from the original plan. A stale TODO.md is
  worse than none — don't let completed work go unchecked or new decisions
  go unrecorded.
- If a decision changes direction (a crate swap, a rejected approach, a
  scope change), record it in both places: the reasoning in `CLAUDE.md`
  (as this file already does for e.g. the `fdg`→`fdg-sim` correction and
  the `obsidian-cli` rejection), the resulting task-list change in
  `TODO.md`.

## Starting point for a fresh session

Check `TODO.md` for the current phase. As of this writing Phases 0–3
(hello world CLI, CLI args & config, vault parser, `petgraph` graph model)
are done — next up is Phase 4 (`fdg-sim` 3D layout), then one static
`ratatui` Braille frame (Phase 5) before adding physics tuning, camera
interaction, or query features. That thin vertical slice is the thing to
prove first.
