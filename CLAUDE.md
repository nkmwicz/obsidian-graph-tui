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

1. **Parser** (first-party, nothing exists off the shelf for this): walk the
   vault's `.md` files, extract `[[wikilink]]` targets (also consider plain
   markdown `[text](file.md)` links), build a node/edge list.
2. **Graph model / query**: [`petgraph`](https://github.com/petgraph/petgraph)
   — in-memory graph with real traversal (BFS/DFS, shortest path, connected
   components, N-hop neighborhoods). **Not Kùzu for v1** — deliberately
   deferred, see "Explicitly out of scope" below. Swappable later without
   touching the layout or rendering layers.
3. **3D layout**: [`fdg`](https://github.com/grantshandy/fdg) /
   [`fdg-sim`](https://crates.io/crates/fdg-sim) — force-directed graph
   layout, works in N dimensions (3D), converts directly from a
   `petgraph::Graph`.
4. **Rendering**: [`ratatui`](https://ratatui.rs/), using the `Canvas`
   widget's `Marker::Braille` mode (2×4 sub-cell resolution — no GPU, no
   terminal graphics protocol dependency, works in any terminal, not just
   Kitty). Reference implementation to study first: ratatui's own
   `volatility-surface` example (`cargo run -p volatility-surface` in the
   ratatui repo) — it already does perspective projection + interactive
   rotate/zoom via Canvas+Braille. Also evaluate
   [`ratatui-3d`](https://lib.rs/crates/ratatui-3d) (renders 3D scenes as a
   ratatui widget, has a Braille high-res render mode) and
   `ratatui-wireframe` (rotating 3D wireframe models specifically) as
   possible off-the-shelf building blocks before hand-rolling projection math.
5. **Input**: `crossterm` for live keyboard-driven camera orbit/zoom/pan.

> All of the above crates were identified via web research in the planning
> conversation, not verified hands-on — re-check current version, API shape,
> and maintenance status before committing to any of them.

## MVP scope, in order

1. Parse a given vault path into a `petgraph` graph.
2. Force-directed 3D layout via `fdg`.
3. Static rendered view in `ratatui` (Braille wireframe, no interaction yet)
   — prove the rendering pipeline end to end first.
4. Add camera controls (orbit/zoom via keyboard).
5. Add basic query/traversal: local graph around one note (N-hop
   neighborhood), filter by tag/folder.
6. *(Later, not v1)* Swap `petgraph` for an embedded graph DB
   ([Kùzu](https://kuzudb.github.io/docs/) — Cypher, single-file, no
   server, native Rust bindings) only if real ad-hoc query needs outgrow
   simple neighborhood/filter traversal.

## Explicitly out of scope for v1

- Kùzu/Cypher querying — see above, revisit only if needed.
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

## Starting point for a fresh session

Get the parser → `petgraph` → `fdg` layout → one static `ratatui` Braille
frame working end to end before adding physics tuning, camera interaction,
or query features. That thin vertical slice is the thing to prove first.
