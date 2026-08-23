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

## Phase 4 — 3D force-directed layout ✅

`fdg-sim` pins `petgraph = "0.6"`, a different major version than this
project's `petgraph` (0.8.3) — confirmed by building with both in the
dependency tree (`cargo build` succeeds, two separate `petgraph` crates
compiled side by side). Its `ForceGraph` (a `StableGraph` from that inner
petgraph) is therefore a distinct type from `graph::Graph` and can't be
constructed directly from it; `layout::layout()` copies nodes/edges across
by iteration instead, tracking the correspondence in a `HashMap`.

- [x] Convert the `petgraph` graph into an `fdg-sim` simulation
      (`Dimensions::Three`) — `src/layout/mod.rs`
- [x] Run the simulation to convergence, extract 3D coordinates per node —
      fixed budget of 1000 steps at `dt = 0.035` (Fruchterman-Reingold,
      `scale = 45.0`, `cooloff_factor = 0.975`); `fdg-sim` has no built-in
      stability check, and the cooloff factor decays forces toward zero
      each step, so a generous fixed step count stands in for convergence
      detection without adding one

**Done when:** every node has a stable (x, y, z) position. ✅ Verified:
unit tests in `src/layout/mod.rs` (every node gets a position, connected
nodes land at distinct positions, a self-loop doesn't panic) plus
`cargo run --release -- ~/vaults/obg-test` producing 14 positions for 14
notes in ~20ms.

**Correction, found in Phase 5:** those unit tests only checked position
*count*, not that the values were usable — every position for the full
fixture vault was actually `NaN`. A self-loop (`self-loop.md`) makes a
node its own neighbor in the physics graph, and `fdg-sim`'s attraction
force divides by node-to-neighbor distance, which is zero for a node and
itself; the resulting `NaN` spreads to every other node within a handful
of steps, since each step's repulsion/attraction reads every other
node's position. Only surfaced by actually trying to render something
(Phase 5) — no unit test had asserted finiteness. Fixed by excluding
self-loops from the physics graph entirely in
`undirected_edge_pairs()` (`src/layout/mod.rs`) — they have no visible
geometry anyway (see Phase 5's note below). New regression test
(`every_position_is_finite`) covers it going forward.

## Phase 5 — Static render ✅

First real visual milestone — prove the whole pipeline end-to-end.

- [x] Study `ratatui`'s `volatility-surface` and `canvas` examples
      (`examples/apps/` in the ratatui repo) — used `Surface3D::project`'s
      rotate-Z-then-rotate-X + perspective-divide scheme directly as the
      reference for `render::project()`
- [x] ~~`ratatui` `Canvas` widget + `Marker::Braille`~~ — built first,
      then superseded (see "Rendering pivot" below); `render::project()`'s
      math is the one part that survived unchanged
- [x] Draw the laid-out graph as a single static frame (nodes + edges)

**Done when:** `obg /path/to/vault` renders a recognizable 3D graph shape
in the terminal, once, no interaction yet. ✅ Verified (Braille version):
unit tests for the pure projection/bounds math in `src/render/mod.rs`,
plus an actual run against `~/vaults/obg-test` in a real pty (captured
and rendered through a minimal ANSI/VT emulator to inspect the output
directly) — produced a recognizable connected wireframe shape, not just
"didn't crash." That real-render check is what caught the Phase 4 `NaN`
bug above; the pipeline wouldn't have proven itself end-to-end without
it. ⚠️ **Not yet re-verified for the raster/Kitty version** — see the
pivot note below; that needs the user's own Kitty terminal, which this
environment doesn't have.

**Rendering pivot, still within Phase 5:** after seeing the actual
Braille output, direct feedback was that thin, jagged dot-matrix strokes
were a hard ceiling, not a tuning gap — the actual bar was something like
deck.gl's `PointCloudLayer` (smooth anti-aliased circles, real depth
shading). Confirmed the user's daily terminal is Kitty (the Kitty
graphics protocol's native target) before committing to the swap. Now:
`tiny-skia` rasterizes an anti-aliased scene (nodes as depth-shaded
circles, edges as thin depth-shaded strokes) and `viuer` displays it
inline, auto-detecting Kitty/iTerm/Sixel and falling back to half-block
characters otherwise. Full reasoning, the two Braille-era bugs whose
fixes carried forward (node/edge indistinguishability, isotropic
bounds), and the portability tradeoff this accepts are in `CLAUDE.md`'s
"Rendering" section — don't duplicate it here.

A third, independent bug was caught mid-pivot: the reference example's
`CAMERA_DISTANCE = 4.0` is only safe for its own pre-normalized data;
`fdg-sim` positions are large enough that the perspective divide could
go negative/blow up for realistic graphs, once depth became
load-bearing for shading rather than cosmetic. Fixed in
`camera_distance_for()` — camera distance now scales with the data's own
radius. See `CLAUDE.md` for detail; this was likely silently affecting
the Braille version too.

Note: `~/vaults/obg-test/self-loop.md`'s self-referencing edge (verified
in Phase 3) has no visible geometry in a wireframe — a node linking to
itself draws nothing. That's expected, not a rendering bug. It's also
excluded from the physics graph entirely (see the Phase 4 correction
above), so there's nothing left to "fix" here.

- [ ] **Follow-up, not yet done:** get the user's own confirmation of
      what this actually looks like in their real Kitty terminal, and
      adjust `PX_PER_COL`/`PX_PER_ROW` (the per-cell raster oversampling,
      currently an empirical 10×20 guess at typical terminal font pixel
      aspect ratio, unverified against a real terminal) if the shape
      looks stretched.

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

## Phase 8 — Layout caching & performance

Placed here, not right after Phase 4, on purpose: the project's own
priority is proving the interactive pipeline end-to-end first (nothing's
rendered yet as of Phase 4), and Phase 7's filtering/N-hop views affect
whether this phase should cache one full-vault layout or per-view
layouts — better scoped once that exists than guessed now.

Caching only fixes the *repeat-launch, nothing changed* case — it does
not fix the underlying scaling problem. `fdg-sim`'s Fruchterman-Reingold
repulsion is a plain O(n²) nested loop (confirmed directly in its source:
every node checks every other node, no spatial partitioning/Barnes-Hut),
run for a fixed 1000 steps regardless of graph size (Phase 4's `STEPS`/
`STEP_DT` in `src/layout/mod.rs`) — trivial at the 14-note fixture vault,
but this is the actual bottleneck on a large vault, not parsing (fast,
O(n)) or traversal (petgraph BFS/DFS, O(V+E), also fast at any realistic
size — see CLAUDE.md).

- [ ] Benchmark the current fixed-1000-step layout against a synthetic
      vault at a few sizes (e.g. 500 / 2,000 / 10,000 notes) to get real
      numbers instead of estimates, before committing to a specific
      caching/threshold design
- [ ] Persist computed positions to disk, keyed by a hash of the vault's
      resolved node/edge structure (not file mtimes — those are fragile
      against touches/moves that don't change content); reload instead
      of recomputing when the hash matches
- [ ] Replace the fixed `STEPS = 1000` budget with a convergence check
      (stop once max per-step node displacement drops below a
      threshold) — cuts wasted computation on cache misses and the
      first run alike, independent of caching
- [ ] Record the benchmark numbers and the resulting practical
      vault-size ceiling in `CLAUDE.md`
- [ ] *(Stretch, only if the benchmark shows it's needed)* incremental
      warm-start reflow: on a small vault edit, reuse cached positions as
      the simulation's starting point and only reflow the changed
      neighborhood, instead of invalidating the whole cache

**Done when:** relaunching against an unchanged vault reloads positions
without rerunning the simulation, and the benchmark numbers are recorded
so "large vault" isn't a guess anymore.

Note: if the benchmark shows a real vault the user actually has is slow
even on a cache hit's first computation, the fix is algorithmic —
Barnes-Hut/quadtree repulsion, which likely means a different crate (or
a hand-rolled force step; see the unpublished `fdg` rewrite noted under
Phase 4/CLAUDE.md's layout section) — not more caching. Revisit that
only if the benchmark proves it's needed, not speculatively here.

## Phase 9 — Cypher querying (future, confirmed direction, not v1)

Promoted from "maybe revisit" to a real planned phase — this project is
expected to grow into this, not just consider it.

- [ ] Introduce Kùzu as an embedded graph DB alongside or in place of
      `petgraph`
- [ ] Expose ad-hoc Cypher queries from within the TUI
- [ ] Scope further once Phases 0–7 are done and the query needs that
      `petgraph` traversal can't cover are concrete

**Done when:** not yet scoped — revisit once earlier phases surface real
query needs.
