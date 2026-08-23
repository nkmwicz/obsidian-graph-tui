# obsidian-graph-tui — TODO

Phased roadmap. Each phase is a thin vertical slice — runnable/verifiable on
its own, not a pile of half-finished work (see CLAUDE.md's "Starting point
for a fresh session"). Work top to bottom; don't start a phase until the one
above it is done and verified.

## Phase 0 — Hello World CLI ✅

Prove the binary exists and builds before any real logic.

- [x] `cargo init` a binary crate at the repo root
- [x] Name the binary `obg` (package name is `obg`; binary defaults to the
      package name, no `[[bin]]` override needed)
- [x] `main.rs` prints `Hello, world!`
- [x] Verify `cargo run` prints it
- [x] Verify `cargo build --release` produces a working `obg` binary

**Done when:** running `obg` prints `hello world`.

## Phase 1 — CLI args & config surface

Vault path must be configurable, not hardcoded (CLAUDE.md, "Vault access").

- [ ] Add `clap`, wire a vault-path positional/flag argument
- [ ] Add `serde` + `toml` + `directories` for a config file (vault path,
      later: default filters, camera defaults)
- [ ] CLI arg takes precedence over config file when both are present
- [ ] Clear, non-panicking error message on a missing/invalid vault path

**Done when:** `obg /path/to/vault` and a config-file-supplied path both
resolve to the same vault path, with a clean error on a bad path.

## Phase 2 — Vault parser

First-party — nothing off the shelf handles Obsidian wikilinks. Test
against `~/vaults/obg-test/`, a purpose-built fixture vault (legend at
`~/vaults/obg-test/_fixture-notes.md`) covering: orphan node, broken link,
self-loop, missing frontmatter, case-mismatched links, alias/heading/embed
link forms, a non-edge external URL, a duplicate basename in two folders,
and an empty `.obsidian/` that must be skipped.

- [ ] Walk the vault's `.md` files (`walkdir` or `ignore`; skip `.obsidian/`,
      `.git/`)
- [ ] Extract `[[wikilink]]`, `[[wikilink|alias]]`, `[[note#heading]]`,
      `![[embed]]`
- [ ] Extract plain markdown `[text](file.md)` links
- [ ] Parse YAML frontmatter (`gray_matter`) for `tags:` / `aliases:`
- [ ] Produce a node/edge list (in-memory structs, pre-`petgraph`)

**Done when:** running against `~/vaults/obg-test/` produces the correct
node/edge count and correctly handles every edge case listed in
`_fixture-notes.md` (orphan stays isolated, broken link doesn't crash,
self-loop doesn't crash, `.obsidian/` and the external URL are excluded,
etc.) — then spot-check against a real vault too.

## Phase 3 — Graph model

- [ ] Build a `petgraph::Graph` from the parsed notes/links
- [ ] Basic traversal sanity check (e.g. neighbor count for a given note)

**Done when:** traversal calls return correct results for a known note.

## Phase 4 — 3D force-directed layout

- [ ] Convert the `petgraph` graph into an `fdg-sim` simulation
      (`Dimensions::Three`)
- [ ] Run the simulation to convergence, extract 3D coordinates per node

**Done when:** every node has a stable (x, y, z) position.

## Phase 5 — Static render

First real visual milestone — prove the whole pipeline end-to-end.

- [ ] Study `ratatui`'s `volatility-surface` and `canvas` examples
      (`examples/apps/` in the ratatui repo)
- [ ] `ratatui` `Canvas` widget + `Marker::Braille`, hand-rolled projection
- [ ] Draw the laid-out graph as a single static wireframe frame (nodes +
      edges)

**Done when:** `obg /path/to/vault` renders a recognizable 3D graph shape
in the terminal, once, no interaction yet.

## Phase 6 — Camera interaction

- [ ] `crossterm` keyboard input loop
- [ ] Orbit, zoom, pan; live re-render on input
- [ ] Clean quit (`q` / `Ctrl-C`), terminal state restored on exit/panic

**Done when:** you can freely orbit around the graph and it stays legible.

## Phase 7 — Query & traversal

- [ ] N-hop neighborhood view centered on a given note
- [ ] Filter by tag / folder
- [ ] Jump-to-note (fuzzy search) that recenters the view

**Done when:** you can narrow from "whole vault" to "this note's local
neighborhood" without restarting the tool.

## Phase 8 — Cypher querying (future, confirmed direction, not v1)

Promoted from "maybe revisit" to a real planned phase — this project is
expected to grow into this, not just consider it.

- [ ] Introduce Kùzu as an embedded graph DB alongside or in place of
      `petgraph`
- [ ] Expose ad-hoc Cypher queries from within the TUI
- [ ] Scope further once Phases 0–7 are done and the query needs that
      `petgraph` traversal can't cover are concrete

**Done when:** not yet scoped — revisit once earlier phases surface real
query needs.
