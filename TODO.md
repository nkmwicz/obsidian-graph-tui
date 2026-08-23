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
it. ✅ **Raster/Kitty version confirmed working in the user's real
terminal** (this environment has no real Kitty terminal, so this
couldn't be verified directly — see the pivot note below): user reports
it "looks much better" / "produces a pretty graph" on `cargo run --
~/vaults/obg-test`. `--release` behaves identically (expected — release
vs. debug only affects compile optimization, not output). Aspect-ratio
correctness (`PX_PER_COL`/`PX_PER_ROW`) not explicitly confirmed either
way — asked, no answer yet; treat as still open until confirmed.

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

- [x] Get the user's own confirmation of what this actually looks like
      in their real Kitty terminal — done, see above, positive.
- [x] `PX_PER_COL`/`PX_PER_ROW` aspect ratio — resolved without needing an
      eyeball check: `render::cell_pixel_size()` now reads the terminal's
      actual font-cell pixel dimensions via `crossterm::terminal::
      window_size()` (`TIOCGWINSZ`'s `ws_xpixel`/`ws_ypixel`, which Kitty
      fills in correctly) and uses those directly, falling back to the
      old empirical 10×20 guess only when the terminal reports zero
      (crossterm's own docs note this is unreliable on some platforms).
      Since the isotropic-bounds fix (Phase 5, above) already guarantees
      equal data-space span on both axes, using the terminal's real
      per-cell aspect ratio for the pixel buffer removes the only
      remaining source of stretch — no more guessing needed.

## Phase 6 — Graph algorithms: PageRank, community detection, path tracing

**Reprioritized ahead of camera interaction and petgraph-based
query/traversal** (2026-08-22) — moved from last place to next, on
request, after establishing (see `CLAUDE.md`'s "Why" and "Note-taking
conventions" sections) that this project's actual research value for a
historian's monograph notes is multi-hop path tracing, centrality, and
community detection — not the rendering/camera work, which is polish on
top of a static picture, not a different kind of capability.

**This phase was originally scoped as "Cypher querying & graph
algorithms via Kùzu" and pivoted away from Kùzu entirely while actually
implementing it (2026-08-23).** Kept here as the record of why, since it
was a real, hard-won decision, not a footnote:

- `petgraph` 0.8.3's `algo` module (checked via docs.rs) has `page_rank`,
  `all_simple_paths`/`all_simple_paths_multi`, and shortest-path
  (`dijkstra`/`astar`/`bidirectional_dijkstra`) — but no betweenness
  centrality and no community/modularity detection built in.
  **`communities()`/`pagerank()` in `src/algo/mod.rs` don't use
  `petgraph::algo::page_rank` either, in the end** — see below.
- Kùzu's `algo` extension looked like a direct match (Louvain, PageRank,
  connected components) and was added as a dependency, with a working
  `src/kuzu/mod.rs` written and unit-tested against it. Abandoned after
  directly hitting, in order: (1) betweenness centrality wasn't actually
  in the extension despite this file's original claim (Kùzu's own docs
  only ever promised it via a Python/NetworkX round-trip, not applicable
  to a Rust binary — decided to use PageRank-only for centrality, which
  survived the later pivot); (2) the entire `kuzudb` GitHub org turned
  out to be archived (Oct 2025 — the company is "working on something
  new" per their own README) — `kuzu` 0.11.3 works today but is very
  likely the last release ever; (3) the actively-maintained community
  fork `ryugraph` was considered and rejected — same Rust API, but its
  published crate's build script doesn't actually wire up the `algo`
  extension despite its docs claiming otherwise (verified directly:
  `kuzu`'s `build.rs` passes `-DSTATICALLY_LINKED_EXTENSIONS=algo` to
  CMake, `ryugraph`'s doesn't, and there's no Cargo feature or env var to
  add it); (4) several pure-Rust alternatives (CozoDB/Datalog, `grafeo`,
  `sparrowdb`, `indradb`, "kyugraph") were evaluated and each rejected on
  a concrete, verified fact — wrong query paradigm, a live-but-stalled
  project with open data-integrity bugs, missing algorithms entirely,
  missing Cypher entirely, or too immature to evaluate (see `CLAUDE.md`
  for the specifics on each); (5) even sticking with `kuzu` 0.11.3 hit a
  real, reproducible build failure in this environment — a `cxx`/
  `cxx-build` version-skew undefined-symbol link error, fixable with an
  exact-version pin, but which surfaced while directly experiencing what
  a from-scratch Kùzu C++ rebuild actually costs: 15+ minutes, several GB
  of build artifacts, and an actual OOM kill at full parallelism on a
  normal 15GB desktop. That cost isn't a one-time tax — `CLAUDE.md`'s own
  planned `~/dotfiles/install.sh` integration does a fresh clone + build
  on every (re)install, so every future install/update would pay it
  again. For a personal CLI tool meant to be as easy to install as
  `fancy-cat`, that's a bad trade for centrality/community detection
  specifically — decided with the user to drop the embedded-graph-DB
  approach entirely rather than accept it.
- **Final decision: hand-roll PageRank and Louvain community detection
  directly over `petgraph::Graph<Note, ()>`** — pure Rust, no C++
  toolchain, builds in seconds like the rest of this stack. Both are
  well-specified, standard algorithms (power-iteration PageRank; Blondel
  et al. 2008 Louvain modularity optimization) — "more Rust code to
  write and verify ourselves," accepted deliberately over any of the
  above, not a fallback taken reluctantly.

- [x] ~~Introduce Kùzu as an embedded graph DB~~ — reverted; see above.
      No external graph DB or query engine. `src/algo/mod.rs` takes
      `&Graph<Note, ()>` directly (same shape as `layout::layout`/
      `render::run`) and computes everything in-process.
- [x] Multi-hop path query between two notes (argument lineage tracing)
      — `algo::shortest_path()`, a plain BFS over `Graph::
      neighbors_undirected` (direction-blind — see CLAUDE.md), exposed as
      `obg <vault> path <from> <to>`.
- [x] Centrality ranking (PageRank) to surface structurally load-bearing
      notes — `algo::pagerank()`, a hand-rolled damped power iteration,
      exposed as `obg <vault> pagerank`. Restricted to non-`source`-tagged
      notes (see CLAUDE.md's note-taking conventions) so a book's source
      note doesn't dominate every ranking just by being linked from every
      claim about it. Betweenness centrality stays explicitly deferred —
      `petgraph` doesn't have it either and hand-rolling Brandes' wasn't
      done this phase.
- [x] Community detection (Louvain) to surface historiographical
      clusters/camps from link structure — `algo::communities()`, a
      hand-rolled multi-level Louvain (local-moving + aggregation,
      repeated to convergence) over the undirected claim-note subgraph;
      same source-note exclusion as PageRank.
- [x] Some way to see query results from the TUI — a plain text
      list/table is a fine v1, doesn't need graph-view integration yet
- [ ] ~~Expose ad-hoc Cypher queries directly~~ — no longer applicable;
      there's no query engine to expose. If ad-hoc querying is wanted
      later, it'd mean something else entirely (e.g. a small expression
      language over `petgraph`), not scoped now.

**Done when:** you can run at least one query from each category above
(path, centrality, community) against a real vault and get a result
that's plausibly useful, not just "doesn't crash." ✅ Verified against
`~/vaults/obg-test`: `pagerank`/`communities`/`path` all produce
plausible, non-degenerate output; the no-subcommand render path is
unchanged; full test suite (53 tests) and `cargo clippy` both clean.

## Phase 7 — Camera interaction

Written before the Phase 5 rendering pivot (`CLAUDE.md`'s "Rendering"
section) — re-read that before starting. "Live re-render" no longer
means a `ratatui` `Frame`/`Buffer` diff redraw; it means recompute
rotation → re-rasterize with `tiny-skia` → re-print via `viuer` each
frame, which has different performance and terminal-state implications
(raw mode/alternate screen needs deciding fresh — the static-print flow
today doesn't use either) worth thinking through at the start of this
phase, not assumed from the checklist below.

- [ ] `crossterm` keyboard input loop
- [ ] Orbit, zoom, pan; live re-render on input
- [ ] Clean quit (`q` / `Ctrl-C`), terminal state restored on exit/panic

**Done when:** you can freely orbit around the graph and it stays legible.

## Phase 8 — Query & traversal

Overlaps in spirit with Phase 6 (both are "query" features, and both are
now `petgraph`-native — Phase 6 no longer pulls in a separate graph DB,
see its own section above) but stays distinct in *purpose*: Phase 6 is
global structural analysis (rank/cluster/connect the whole graph), this
phase is local view/filtering (narrow what's rendered around one note).
Worth reusing what already exists rather than re-deriving it: `algo::
shortest_path()` (`src/algo/mod.rs`) already does undirected BFS from a
start note — an N-hop neighborhood view is the same traversal with a hop
count bound instead of a target note, not a new algorithm. `algo::
claim_notes()`'s tag-based filter is a narrow, hardcoded precedent
(exclude `source`-tagged notes from two specific algorithms) — this
phase's "filter by tag/folder" is a genuinely more general, user-facing
feature, not something to just extend from that function, but the same
`Note::tags`/path-prefix data it'd read is already there on the node
weight.

- [ ] N-hop neighborhood view centered on a given note
- [ ] Filter by tag / folder
- [ ] Jump-to-note (fuzzy search) that recenters the view

**Done when:** you can narrow from "whole vault" to "this note's local
neighborhood" without restarting the tool.

## Phase 9 — Layout caching & performance

Placed here, not right after Phase 4, on purpose: the project's own
priority is proving the interactive pipeline end-to-end first (nothing's
rendered yet as of Phase 4), and Phase 8's filtering/N-hop views affect
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

## Phase 10 — Typed links

Not yet scoped in depth — deliberately deferred until Phase 6's
untyped path-tracing has actually been used for a while and the
limitation is felt firsthand, not built speculatively now. Recorded
here (2026-08-23) so the idea isn't lost, not because it's next.

Right now every edge means "these are connected," full stop. The real
gap this closes: distinguishing an argument's *lineage* ("A led to B
led to C") from its *rebuttal chain* ("A refutes B") when tracing paths
— categorically more useful for historiography than undifferentiated
connectivity once Phase 6's path queries are in daily use.

- [ ] Adopt (and document in `CLAUDE.md`'s note-taking conventions) a
      body-text convention for typing a link — `Refutes:: [[Note]]` on
      its own line, Dataview's real inline-field syntax (`Key:: Value`),
      verified against Dataview's own docs. Deliberately not a
      frontmatter field: the parser only reads the body for links (see
      CLAUDE.md), and this convention costs nothing to start using today
      even before the parser understands it — plain `[[wikilinks]]`
      still extract normally either way.
- [ ] Parser: recognize an optional `Word::` prefix immediately before a
      wikilink on the same line, capture it as the edge's relation
- [ ] Graph model: edge weight changes from `Graph<Note, ()>` to
      something like `Graph<Note, Option<String>>` (or a small closed
      enum of known relation types) — **real ripple effect, not a small
      change**: `graph::build()`, `layout::layout()`'s edge iteration,
      and `render`'s edge-drawing all currently assume `()` edges and
      would need updating
- [ ] Decide how (or whether) relation type affects layout physics —
      probably not at first; type should inform *query* results
      (Phase 6 path/traversal output), not necessarily the force
      simulation
- [ ] Untyped links (no `Word::` prefix) stay valid and untyped —
      backward compatible with every note already written

**Done when:** not yet scoped — revisit once Phase 6 is in real use and
this gap is actually felt, not before.

## Phase 11 — `research.lua` bridge

Also deferred, also recorded now so it isn't lost (2026-08-23). The
most concretely motivated of the "what's after Phase 9" ideas discussed
— not speculative feature-brainstorming, a real extension of a parser
that already exists.

The user's `research.lua` (`~/.config/nvim/lua/config/research.lua`) is
a separate system, one stage downstream of this vault: it manages
per-manuscript-project `research/*.md` snippet files (curated,
rewritten-for-the-argument, atomic at the `##`-heading level) and
tracks traceability into the actual Quarto manuscript prose via
`<!-- research: <file> » <heading> -->` HTML-comment markers inserted
at the point of use (`<leader>ri`) and queried in reverse (`<leader>ru`
— "which manuscript sections cite this note"). That marker is plain,
greppable text, structurally not that different from a wikilink.

If the parser also recognized that marker convention, the graph could
span three stages at once: raw Obsidian claim-notes → curated
`research.lua` snippets → the actual manuscript paragraphs that cite
them. Concretely new, not available in either system alone today:
seeing a claim's whole lineage from first reading to published
sentence, or querying "which of my original reading notes actually
made it into print."

- [ ] Decide vault scope: does this mean parsing a *second* root
      directory (the manuscript project's `research/` + `sections/`)
      alongside the Obsidian vault, or treating them as separate graphs
      queried together? Not obvious which is right — think it through
      at the start of this phase, don't assume
- [ ] Parser: recognize `<!-- research: <file> » <heading> -->` as an
      edge (from the citing manuscript section to the cited research
      snippet's heading-anchor)
- [ ] Decide how `research.lua`'s heading-level atomicity (multiple
      snippets per file) maps onto this project's file-level node
      identity (`graph::build()` currently treats one note = one graph
      node) — these are different atomicity strategies for a similar
      need, reconciling them is real design work, not a given
- [ ] Don't automate the *curation* step (deciding what's worth pulling
      from a reading note into a manuscript snippet) — that's real
      editorial/scholarly judgment. Only automate the mechanical parts:
      the marker as a graph edge, and (if genuinely useful) a query that
      surfaces which claims are already curated vs. which have never
      been used in any manuscript.

**Done when:** not yet scoped — revisit once Phase 6 is working and it's
clear this connection is something to reach for, not just something
that sounds good in the abstract.
