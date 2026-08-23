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

## Phase 1 — CLI args & config surface ✅

Vault path must be configurable, not hardcoded (CLAUDE.md, "Vault access").

- [x] Add `clap`, wire a vault-path positional/flag argument
- [x] Add `serde` + `toml` + `directories` for a config file (vault path,
      later: default filters, camera defaults)
- [x] CLI arg takes precedence over config file when both are present
- [x] Clear, non-panicking error message on a missing/invalid vault path

**Done when:** `obg /path/to/vault` and a config-file-supplied path both
resolve to the same vault path, with a clean error on a bad path.

## Phase 2 — Vault parser ✅

First-party — nothing off the shelf handles Obsidian wikilinks. Test
against `~/vaults/obg-test/`, a purpose-built fixture vault (legend at
`~/vaults/obg-test/_fixture-notes.md`) covering: orphan node, broken link,
self-loop, missing frontmatter, case-mismatched links, alias/heading/embed
link forms, a non-edge external URL, a duplicate basename in two folders,
and an empty `.obsidian/` that must be skipped.

- [x] Walk the vault's `.md` files (`walkdir` or `ignore`; skip `.obsidian/`,
      `.git/`)
- [x] Extract `[[wikilink]]`, `[[wikilink|alias]]`, `[[note#heading]]`,
      `![[embed]]`
- [x] Extract plain markdown `[text](file.md)` links
- [x] Parse YAML frontmatter (`gray_matter`) for `tags:` / `aliases:`
- [x] Produce a node/edge list (in-memory structs, pre-`petgraph`)
- [x] Error out (clear message, exit 1) if the resolved vault path
      contains zero `.md` files — Phase 1 only validates that the path is
      a directory (e.g. `obg .` from this repo "succeeds" and resolves to
      the repo root), so the parser is where a not-actually-a-vault path
      needs to get caught, before it silently produces an empty graph

Also required, discovered during implementation (see `CLAUDE.md`): a
naive regex scan can't tell a real `[[wikilink]]` from one written as
`` `[[documentation]]` `` inside backticks — code-span/fence stripping is
now a required pre-pass, not optional polish. Two fixture-vault authoring
bugs surfaced by this while verifying hand-computed counts (`index.md`
accidentally linking `[[orphan]]` for real instead of describing it in
backticks; `aliases-and-headings.md` testing `[[CaseTest]]` against actual
file `case-test.md` — a missing hyphen, not a casing difference) — both
fixed in the vault itself.

**Done when:** running against `~/vaults/obg-test/` produces the correct
node/edge count and correctly handles every edge case listed in
`_fixture-notes.md` (orphan stays isolated, broken link doesn't crash,
self-loop doesn't crash, `.obsidian/` and the external URL are excluded,
etc.) — then spot-check against a real vault too.

## Phase 3 — Graph model ✅

Design question resolved (see `CLAUDE.md` architecture section): `Note`
becomes the node weight — `Graph<Note, ()>` — so path/tags/aliases travel
with the graph instead of living in a second lookup structure. `ParsedVault`
stays exactly what it is today, the parser's output type; a new
`graph::build(ParsedVault) -> Graph<Note, ()>` is the one-time conversion
step, owning the `Vec<Note>` index → `NodeIndex` mapping. Nothing later
(layout, rendering, query) should need to reach back into `ParsedVault`.

- [x] Add `petgraph` to `Cargo.toml` (0.8.3, confirmed)
- [x] `graph::build()`: convert `ParsedVault` into `Graph<Note, ()>`,
      inserting nodes in `notes` order and edges from `edges` — dedupes
      parallel edges between the same pair (a note linking to the same
      target twice) since `petgraph::Graph` is a multigraph by default
- [x] Basic traversal sanity check (e.g. neighbor count for a given note)
      — unit tests in `src/graph/mod.rs`
- [x] Verify self-loops convert cleanly — `~/vaults/obg-test/self-loop.md`
      already produces a real `Edge { from: i, to: i }` in `ParsedVault`
      (not hypothetical, it's in the current fixture output); covered by
      a unit test and confirmed against the real fixture vault (`cargo
      run -- ~/vaults/obg-test` doesn't panic, node/edge counts match
      `ParsedVault`'s exactly — no duplicate-edge pairs exist in this
      fixture, so the dedup path isn't exercised there, only by the
      hand-built unit test)

**Done when:** traversal calls return correct results for a known note,
including the self-loop case.

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

Note: `~/vaults/obg-test/self-loop.md`'s self-referencing edge (verified
in Phase 3) has no visible geometry in a wireframe — a node linking to
itself draws nothing. That's expected, not a rendering bug; don't spend
time "fixing" it here.

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
