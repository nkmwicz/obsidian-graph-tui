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
   `Dimensions::Two`/`Dimensions::Three`. **Correction (Phase 4):** it does
   *not* convert directly from a `petgraph::Graph` — `fdg-sim` pins
   `petgraph = "0.6"`, a different major version than this project's
   `petgraph` (0.8.3), so its `ForceGraph` (a `StableGraph` from that inner
   petgraph) is a distinct type. Both versions coexist fine in the
   dependency tree (`cargo build` compiles both side by side without
   conflict), but the conversion has to happen by hand: `layout::layout()`
   (`src/layout/mod.rs`) iterates the project's `Graph<Note, ()>` and
   copies nodes/edges into a fresh `fdg_sim::ForceGraph`, tracking the
   correspondence in a `HashMap`. **Also discovered in Phase 4:** because
   Obsidian links are directional but `ForceGraph` is an undirected
   multigraph, a mutual link (`A -> B` and `B -> A`, common in practice)
   would otherwise become two parallel edges and get double the
   Fruchterman-Reingold attraction of a one-way link — `layout::layout()`
   collapses edges to unordered node pairs (`undirected_edge_pairs()`)
   before feeding them in, so reciprocal links pull no harder than one-way
   links, matching Obsidian's own graph view (a single undirected line
   either way). Direction itself is untouched in `graph::Graph` — this
   dedup only affects what feeds the physics, not later traversal/backlink
   queries (Phase 7). **Critical correction, found in Phase 5:** a
   self-loop makes a node its own neighbor in the physics graph, and
   `fdg-sim`'s Fruchterman-Reingold attraction divides by node-to-neighbor
   distance — zero, for a node and itself — producing `NaN` that spreads
   to every other node's position within a handful of steps (each step's
   repulsion/attraction reads every other node's position). This silently
   NaN'd all 14 positions for the full `~/vaults/obg-test` fixture (which
   has a real self-loop) — the Phase 4 unit tests didn't catch it because
   they only asserted position *count*, never that the values were
   finite; it only surfaced once Phase 5 actually tried to render
   something. Fixed by dropping self-loops in `undirected_edge_pairs()`
   entirely, not just deduping them — they have no visible geometry in a
   wireframe anyway (see the rendering section below), so nothing is lost
   by excluding them from the physics. This is the concrete case for why
   CLAUDE.md's "prove the pipeline end-to-end" ordering matters: a passing
   unit test suite had already called Phase 4 done. The `grantshandy/fdg` repo has since been rewritten
   into a new unified `fdg` crate (nalgebra-based, const-generic over N
   dimensions, still actively developed as of March 2025) — but that
   rewrite has **never been published to crates.io**, only usable via a
   `git = "..."` Cargo dependency. Use published `fdg-sim = "0.9"` for v1 to
   avoid a git dependency; revisit the rewrite only if `fdg-sim` proves
   insufficient.
   - **Known scaling limit, deliberately not addressed yet:** confirmed by
     reading `fdg-sim`'s source directly, its Fruchterman-Reingold
     repulsion is a plain O(n²) nested loop — no Barnes-Hut/quadtree
     spatial partitioning — run for a fixed 1000 steps regardless of graph
     size. Fine at fixture-vault scale; the real cost on a large vault.
     Traversal (`petgraph` BFS/DFS) and parsing stay fast at any realistic
     vault size — this is specifically a layout-algorithm limitation, not
     a whole-pipeline one. See `TODO.md` Phase 8 (layout caching &
     performance, deliberately scoped for after the interactive pipeline
     is proven, not now) — caching only helps the *repeat-launch,
     unchanged-vault* case; it doesn't remove the O(n²) ceiling itself,
     which would need an algorithmic fix (Barnes-Hut, likely a different
     crate) if a real vault ever proves it necessary.
4. **Rendering**: raster image, composited with [`tiny-skia`](https://github.com/linebender/tiny-skia)
   (confirmed: 0.12.0, 43M downloads, now under the Linebender org, updated
   Feb 2026) and displayed inline via [`viuer`](https://github.com/atanunq/viuer)
   (confirmed: 0.11.0, 1.2M downloads, updated Dec 2025), which auto-detects
   the terminal's image protocol (Kitty graphics protocol, iTerm2, Sixel)
   and falls back to half-block Unicode characters if none is available.
   **This replaced an initial `ratatui` `Canvas`+`Marker::Braille` renderer
   (built and shipped first in Phase 5) after direct user feedback that its
   output — thin, jagged Braille dot-matrix strokes — was a hard ceiling,
   not a tuning problem, and that genuinely smooth anti-aliased lines/depth
   shading (comparable to e.g. deck.gl's `PointCloudLayer`,
   <https://deck.gl/examples/point-cloud-layer>) were the actual goal.**
   Confirmed the user's daily terminal is Kitty itself before committing to
   this — the Kitty graphics protocol's native target — so the tradeoff
   below is made with that specific fact in hand, not assumed.
   - **The Braille attempt and what it taught (kept for the record, not
     because it's still used):** the reference implementation studied
     first was ratatui's own `volatility-surface` example
     (`examples/apps/volatility-surface`) — its `Surface3D::project`
     rotate-Z-then-rotate-X + perspective-divide scheme is still exactly
     what `render::project()` uses today, unchanged by the rendering
     pivot. Two real bugs were found and fixed while tuning the Braille
     version, and both lessons carried forward: (1) nodes drawn with the
     same `Marker::Braille` dot as edges are visually indistinguishable —
     the raster renderer draws nodes as distinctly larger, depth-shaded
     circles for exactly this reason. (2) `bounds()`/viewport framing must
     be *isotropic* (equal span on both axes) rather than padding x/y
     independently, because `fdg-sim` positions don't spread evenly across
     axes and independent padding stretches the shape — the raster
     renderer's `bounds()` keeps this fix verbatim. Verified empirically
     (see below) that even with both fixes and doubled-up Bresenham
     strokes, Braille's dot-matrix text ceiling was real, not a tuning
     gap — confirming the pivot was necessary, not premature.
   - **A real, independently-worthwhile bug caught during the pivot:**
     the reference example's `CAMERA_DISTANCE = 4.0` constant is safe only
     because its data is pre-normalized to a ~1.5-unit half-width.
     `fdg-sim` positions routinely span tens of units (Fruchterman-
     Reingold `scale = 45.0`), so with a fixed small camera distance,
     `camera_distance + z2` in `project()` can go negative or near-zero
     for realistic graphs — the perspective divide blows up or flips
     sign, turning one node into a wild outlier that dominates the
     viewport for reasons unrelated to the actual layout. This was
     silently present in the Braille renderer too (the isotropic-bounds
     fix partly masked it by absorbing the outlier into a bigger, blander
     viewport rather than surfacing it). Caught by a unit test
     (`camera_distance_scales_with_the_data_and_keeps_the_denominator_
     positive`) once depth-based shading made the sign of the depth value
     load-bearing rather than cosmetic. Fixed in `camera_distance_for()`
     (`src/render/mod.rs`): camera distance now scales with the data's
     own radius instead of a borrowed constant.
   - **Portability, stated plainly:** this narrows "looks good" to
     terminals implementing an image protocol (Kitty, WezTerm, Ghostty,
     Konsole partial, iTerm2, Sixel-capable terminals) — reversing the
     earlier Braille decision's explicit "works in any terminal, no
     graphics-protocol dependency" requirement. `viuer`'s half-block
     fallback means the tool doesn't hard-fail elsewhere, but that
     fallback is coarser than the tuned Braille renderer was (1×2
     sub-cell resolution vs. Braille's 2×4). This was a deliberate,
     eyes-open tradeoff for this specific user's daily-driver terminal
     (Kitty), not a default recommendation — see "Explicitly out of
     scope" below, which used to list this as a mere fallback.
   - **`image` crate: pin `default-features = false`.** `viuer`'s own
     `Cargo.toml` only requests `image` with the `png` feature — enabling
     default features on our own dependency line pulled in every codec
     (`avif` via `rav1e`, `tiff`, `webp`, `exr`, `gif`, ...) neither we nor
     `viuer`'s Kitty path need (confirmed from the wire format: the actual
     Kitty escape sequence transmits raw pixels, `f=24`, not PNG), roughly
     6x more crates to compile for zero benefit. Trimmed once noticed.
   - **How this was actually verified, and the real limit of that
     verification:** no screenshot tool or real Kitty terminal is
     available in this environment. What *was* verified: the release
     binary runs to completion (exit 0) against the fixture vault, the
     Kitty-protocol detection handshake is genuinely attempted
     (`_Gi=...a=q...` observed in the raw output), and — since this
     sandbox isn't a real Kitty terminal — it correctly falls through to
     `viuer`'s half-block fallback, which was confirmed to carry real,
     non-constant color data (not a blank/degenerate image). What was
     **not** verified from this environment: what the image actually
     looks like through the real Kitty graphics protocol. That requires
     the user to run it in their own terminal and report back — treat
     this rendering pipeline as "believed correct, not yet eyes-verified"
     until that happens.
   - Earlier Braille-specific corrections (node/edge indistinguishability,
     line thickness, isotropic bounds) are superseded by the above but
     intentionally left in git history rather than scrubbed — they're
     what established both bugs the raster renderer inherited fixes for.
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
3. Static rendered view — a `tiny-skia`-rasterized, `viuer`-displayed
   image (Kitty graphics protocol, auto-detected; see "Rendering" above
   for why this replaced an initial `ratatui` Canvas+Braille attempt), no
   interaction yet — prove the rendering pipeline end to end first.
4. Add camera controls (orbit/zoom via keyboard).
5. Add basic query/traversal: local graph around one note (N-hop
   neighborhood), filter by tag/folder.
6. Layout caching & performance: persist computed positions so an
   unchanged vault reloads instead of rerunning the simulation; replace
   the fixed 1000-step budget with a convergence check. See `TODO.md`
   Phase 8 — placed after query/traversal deliberately, not right after
   the layout work itself (Phase 4), per the "prove the pipeline first"
   philosophy below.
7. *(Later, not v1 — but a confirmed direction, not just a maybe)*
   Introduce an embedded graph DB ([Kùzu](https://kuzudb.github.io/docs/) —
   Cypher, single-file, no server, native Rust bindings) alongside or in
   place of `petgraph`, to expose ad-hoc Cypher querying from within the
   TUI. See `TODO.md` Phase 9 — scoped further once Phases 0–8 there surface
   concrete query needs `petgraph` traversal can't cover.

## Explicitly out of scope for v1

- Kùzu/Cypher querying for v1 specifically — see above, it's a confirmed
  future phase (`TODO.md` Phase 9), just not part of the initial build.
- Dataview-equivalent dynamic queries, or Obsidian plugin-ecosystem parity.
- `neato -Tkitty` piped from Graphviz (external-tool static 2D rendering)
  — an earlier, simpler idea explored before "true 3D + true query"
  became the actual requirement. Superseded, not merely deprioritized:
  the project does now render via the Kitty graphics protocol (see
  "Rendering" above), but as our own `tiny-skia`-rasterized 3D scene, not
  a piped-through external tool's static export.
- A non-graphics-protocol fallback renderer for terminals without image
  protocol support beyond `viuer`'s built-in half-block fallback — see
  "Rendering" above for the portability tradeoff this accepts. Revisit
  only if that fallback's quality becomes a real problem, not
  speculatively.

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

Check `TODO.md` for the current phase. As of this writing Phases 0–5
(hello world CLI, CLI args & config, vault parser, `petgraph` graph model,
`fdg-sim` 3D layout, static `ratatui` Braille render) are done — next up
is Phase 6: live camera controls (orbit/zoom/pan via `crossterm`) over
the same static frame, before query/traversal or layout-caching features.
