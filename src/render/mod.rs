use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};

use image::{DynamicImage, RgbaImage};
use petgraph::graph::{Graph, NodeIndex};
use ratatui::crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::graph;
use crate::layout::{self, Position};
use crate::vault::Note;
use crate::view::{self, DEFAULT_HOPS, View};

/// Default camera orientation, matching the fixed values Phase 5 shipped
/// with before orbit/zoom/pan became live (Phase 7).
const DEFAULT_ROTATION_X: f64 = 0.6;
const DEFAULT_ROTATION_Z: f64 = 0.3;

/// Per-keypress increments for orbit/pan/zoom. Chosen empirically for
/// "one press = a small, visible nudge" — not derived from anything.
const ROTATE_STEP: f64 = 0.15;
const PAN_STEP: f64 = 0.12;
const ZOOM_STEP: f64 = 1.15;

/// Zoom is a divisor on the derived camera distance (see
/// `effective_camera_distance`): `ZOOM_MAX` must stay low enough that even
/// at max zoom-in, `base_distance / ZOOM_MAX` (after `MIN_CAMERA_DISTANCE`
/// re-flooring) stays safely above the data's own radius — the same
/// perspective-divide sign-flip risk documented on `camera_distance_for`
/// below. 2.0 keeps the effective distance at >= 1.25x radius whenever the
/// floor isn't binding, and the floor re-application protects the small-
/// graph case where it is. See `effective_camera_distance_after_max_zoom_
/// stays_safely_above_the_data_radius` for the regression test.
const ZOOM_MAX: f64 = 2.0;
const ZOOM_MIN: f64 = 0.3;

/// Floor for the derived camera distance (see `camera_distance_for`), so
/// a near-empty or single-node graph (radius ~0) still gets a sane camera
/// placement instead of one that's degenerately close.
const MIN_CAMERA_DISTANCE: f64 = 4.0;

/// Fallback raster oversampling per terminal cell, used only when the
/// terminal doesn't report real font-cell pixel dimensions (see
/// `cell_pixel_size`). Terminal fonts are roughly twice as tall as wide in
/// pixels, so rows get twice the per-cell pixel budget of columns — an
/// empirical guess, not derived from queryable metrics.
const PX_PER_COL: u32 = 10;
const PX_PER_ROW: u32 = 20;

/// How many live-filtered matches a search/tag/folder prompt (Phase 8)
/// shows at once. Small on purpose — this is a terminal overlay competing
/// with the image for vertical space, not a full-screen picker.
const MAX_PROMPT_MATCHES: usize = 6;

const HELP_TEXT: &str =
    "arrows/hjkl orbit  wasd pan  +/- zoom  r reset  q/esc/ctrl-c quit";

const BACKGROUND: (u8, u8, u8) = (8, 10, 18);
const NODE_FAR: (u8, u8, u8) = (30, 90, 110);
const NODE_NEAR: (u8, u8, u8) = (140, 230, 255);
const EDGE_COLOR: (u8, u8, u8) = (120, 150, 170);

/// Live camera state driven by keyboard input (Phase 7). `zoom` is a
/// divisor applied to the data-derived camera distance (see
/// `effective_camera_distance`) rather than a distance itself, so 1.0
/// always means "no zoom" regardless of graph scale. `pan_x`/`pan_y` are
/// fractions of the current viewport half-span (see `apply_pan`) rather
/// than absolute units, so a pan step feels the same size on screen at any
/// zoom level instead of shrinking as the view zooms in.
struct Camera {
    rotation_x: f64,
    rotation_z: f64,
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            rotation_x: DEFAULT_ROTATION_X,
            rotation_z: DEFAULT_ROTATION_Z,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

impl Camera {
    /// Applies one key press to the camera. Returns `true` if it's a quit
    /// key. Unrecognized keys are a no-op — most of the keyboard isn't
    /// bound to anything, and that's fine.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,

            KeyCode::Left | KeyCode::Char('h') => self.rotation_z -= ROTATE_STEP,
            KeyCode::Right | KeyCode::Char('l') => self.rotation_z += ROTATE_STEP,
            KeyCode::Up | KeyCode::Char('k') => self.rotation_x -= ROTATE_STEP,
            KeyCode::Down | KeyCode::Char('j') => self.rotation_x += ROTATE_STEP,

            KeyCode::Char('w') => self.pan_y += PAN_STEP,
            KeyCode::Char('s') => self.pan_y -= PAN_STEP,
            KeyCode::Char('a') => self.pan_x -= PAN_STEP,
            KeyCode::Char('d') => self.pan_x += PAN_STEP,

            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.zoom = (self.zoom * ZOOM_STEP).min(ZOOM_MAX);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.zoom = (self.zoom / ZOOM_STEP).max(ZOOM_MIN);
            }

            KeyCode::Char('r') => *self = Camera::default(),

            _ => {}
        }
        false
    }
}

/// Which of the three live filters (`TODO.md` Phase 8) a `Prompt` is
/// currently gathering text for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    /// Jump-to-note: picks a note to become the neighborhood center.
    Search,
    Tag,
    Folder,
}

impl PromptKind {
    fn label(self) -> &'static str {
        match self {
            PromptKind::Search => "search",
            PromptKind::Tag => "tag",
            PromptKind::Folder => "folder",
        }
    }
}

/// Live text-entry state for the search/tag/folder overlay: a query
/// string edited character-by-character, and which of the live-filtered
/// matches (see `prompt_matches`) is currently selected for `Enter` to
/// apply. Only one can be open at a time — opening a new one (there's no
/// key bound to that while a prompt is already active; see
/// `interactive_loop`) isn't a case that needs handling.
struct Prompt {
    kind: PromptKind,
    query: String,
    selected: usize,
}

impl Prompt {
    fn new(kind: PromptKind) -> Self {
        Prompt {
            kind,
            query: String::new(),
            selected: 0,
        }
    }
}

/// Notes/tags/folders in `graph` whose text contains `prompt`'s query
/// (case-insensitive substring — a plain, cheap stand-in for the fuzzy
/// search `TODO.md` Phase 8 asks for; "contains" already narrows a
/// real vault's worth of notes down to a handful of keystrokes, and
/// pulling in a fuzzy-matching crate for more than that felt like more
/// than this needed). Sorted for a stable on-screen order and truncated
/// to `MAX_PROMPT_MATCHES` so the overlay stays small regardless of vault
/// size.
fn prompt_matches(graph: &Graph<Note, ()>, prompt: &Prompt) -> Vec<String> {
    let mut candidates: Vec<String> = match prompt.kind {
        PromptKind::Search => graph
            .node_weights()
            .map(|n| n.path.to_string_lossy().into_owned())
            .collect(),
        PromptKind::Tag => graph
            .node_weights()
            .flat_map(|n| n.tags.iter().cloned())
            .collect(),
        PromptKind::Folder => graph
            .node_weights()
            .filter_map(|n| n.path.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    };
    candidates.sort();
    candidates.dedup();

    let query = prompt.query.to_lowercase();
    candidates.retain(|c| c.to_lowercase().contains(&query));
    candidates.truncate(MAX_PROMPT_MATCHES);
    candidates
}

/// Applies the chosen match to `view`, according to what kind of prompt
/// produced it. A search match that isn't found in `graph` (shouldn't
/// happen — `chosen` always comes from `prompt_matches` run against this
/// same `graph`) is silently ignored rather than panicking.
///
/// Re-centering on a *different* note via search keeps whatever hop count
/// was last set rather than resetting to `DEFAULT_HOPS` — only the first
/// centering (no previous center at all) gets the default, so narrowing
/// the hop count and then jumping to a nearby note doesn't undo that
/// narrowing.
fn apply_prompt_selection(graph: &Graph<Note, ()>, prompt: &Prompt, chosen: &str, view: &mut View) {
    match prompt.kind {
        PromptKind::Search => {
            if let Some(idx) = graph
                .node_indices()
                .find(|&i| graph[i].path.to_string_lossy() == chosen)
            {
                if view.center.is_none() {
                    view.hops = DEFAULT_HOPS;
                }
                view.center = Some(idx);
            }
        }
        PromptKind::Tag => view.tag = Some(chosen.to_string()),
        PromptKind::Folder => view.folder = Some(chosen.to_string()),
    }
}

/// One line of camera controls, one line of the current view (whole vault,
/// or the active neighborhood/tag/folder filter) plus how to change it,
/// and — while a prompt is open — the query line and its live matches.
fn header_lines(graph: &Graph<Note, ()>, view: &View, prompt: &Option<Prompt>) -> Vec<String> {
    let mut lines = vec![
        HELP_TEXT.to_string(),
        format!(
            "{}  |  / search  t tag  f folder  0 clear  [ ] hops",
            view_status(graph, view)
        ),
    ];

    if let Some(prompt) = prompt {
        lines.push(format!("{}> {}", prompt.kind.label(), prompt.query));

        let matches = prompt_matches(graph, prompt);
        if matches.is_empty() {
            lines.push("  (no matches)".to_string());
        } else {
            let selected = prompt.selected.min(matches.len() - 1);
            for (i, m) in matches.iter().enumerate() {
                let marker = if i == selected { '>' } else { ' ' };
                lines.push(format!("{marker} {m}"));
            }
        }
    }

    lines
}

/// Describes the active `View` in one line — "whole vault" when
/// unfiltered, otherwise the neighborhood center/hop-count and any tag/
/// folder filter, comma-separated.
fn view_status(graph: &Graph<Note, ()>, view: &View) -> String {
    if view.is_unfiltered() {
        return format!("view: whole vault ({} notes)", graph.node_count());
    }

    let mut parts = Vec::new();
    if let Some(center) = view.center
        && let Some(note) = graph.node_weight(center)
    {
        let hop_word = if view.hops == 1 { "hop" } else { "hops" };
        parts.push(format!("{} ({} {hop_word})", note.path.display(), view.hops));
    }
    if let Some(tag) = &view.tag {
        parts.push(format!("tag={tag}"));
    }
    if let Some(folder) = &view.folder {
        parts.push(format!("folder={folder}"));
    }

    format!("view: {}", parts.join(", "))
}

/// Cached induced subgraph + its own re-layout for the currently active
/// `View`, keyed by that `View` so `interactive_loop` only recomputes it
/// when the view actually changed since the last frame.
struct ViewCache(View, Graph<Note, ()>, HashMap<NodeIndex, Position>);

struct Projected {
    x: f64,
    y: f64,
    /// `camera_distance / (camera_distance + view_space_z)` from
    /// `project()` — larger means closer to the camera. Used as a depth
    /// proxy for size/color shading, the same role depth plays in a point
    /// cloud renderer (e.g. deck.gl's `PointCloudLayer`).
    depth: f64,
}

/// RAII terminal-state guard: enters raw mode + the alternate screen on
/// construction, and — critically — always restores both on `Drop`, which
/// runs on a normal return *and* on an unwinding panic. Without this, a
/// panic mid-orbit would leave the user's real terminal stuck in raw mode
/// (no echo, no line buffering, no Ctrl-C) after the process exits.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
        terminal::enable_raw_mode()?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), cursor::Show, LeaveAlternateScreen);
    }
}

/// Rasterizes `graph` laid out at `positions` into an anti-aliased image
/// (nodes sized/colored by depth, edges as thin depth-shaded strokes) and
/// displays it inline via the terminal's image protocol (Kitty/iTerm/Sixel,
/// auto-detected; falls back to half-block characters if none is
/// available — see CLAUDE.md's rendering section for why this replaced
/// the earlier `ratatui` Canvas+Braille renderer).
///
/// When stdout is a real terminal, this drives a live keyboard-controlled
/// orbit/zoom/pan loop (Phase 7); when it isn't (piped/redirected output,
/// or this project's own test/CI environment, which has no real tty),
/// raw mode can't be entered at all, so it falls back to printing a single
/// static frame at the default camera angle — the same behavior this
/// function had before Phase 7.
pub fn run(graph: &Graph<Note, ()>, positions: &HashMap<NodeIndex, Position>) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return print_frame(graph, positions, &Camera::default(), &[]);
    }

    let _guard = TerminalGuard::enter()?;
    interactive_loop(graph, positions)
}

/// Redraws on every key press (and terminal resize, which also arrives as
/// an `Event`) and blocks on `event::read()` in between — there's nothing
/// to animate on its own, so a busy-poll loop would just burn CPU for no
/// benefit over waiting for the next input event.
///
/// Owns the live `View` (Phase 8's neighborhood/tag/folder narrowing) on
/// top of the `Camera` (Phase 7): `graph`/`positions` are always the
/// *whole* vault, laid out once by the caller; `cache` holds the induced
/// subgraph and its own re-layout for the current `View` when one is
/// active, recomputed only when `view` actually changes (not on every
/// camera-only redraw, which would re-run the layout simulation for
/// nothing). An unfiltered `View` deliberately keeps `cache` empty rather
/// than caching a no-op copy of the whole graph, so the common case (no
/// filter active) never pays a subgraph/layout cost at all.
fn interactive_loop(
    graph: &Graph<Note, ()>,
    positions: &HashMap<NodeIndex, Position>,
) -> io::Result<()> {
    let mut camera = Camera::default();
    let mut view = View::default();
    let mut prompt: Option<Prompt> = None;
    let mut cache: Option<ViewCache> = None;

    loop {
        let needs_recompute = match &cache {
            Some(ViewCache(cached_view, _, _)) => cached_view != &view,
            None => !view.is_unfiltered(),
        };
        if needs_recompute {
            cache = if view.is_unfiltered() {
                None
            } else {
                let keep = view::visible_nodes(graph, &view);
                let sub = graph::induced_subgraph(graph, &keep);
                let sub_positions = layout::layout(&sub);
                Some(ViewCache(view.clone(), sub, sub_positions))
            };
        }
        let (draw_graph, draw_positions) = match &cache {
            Some(ViewCache(_, g, p)) => (g, p),
            None => (graph, positions),
        };

        let header = header_lines(graph, &view, &prompt);
        print_frame(draw_graph, draw_positions, &camera, &header)?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        if let Some(active_prompt) = &mut prompt {
            match key.code {
                KeyCode::Esc => prompt = None,
                KeyCode::Enter => {
                    let matches = prompt_matches(graph, active_prompt);
                    if let Some(chosen) = matches.get(active_prompt.selected.min(
                        matches.len().saturating_sub(1),
                    )) {
                        apply_prompt_selection(graph, active_prompt, chosen, &mut view);
                    }
                    prompt = None;
                }
                KeyCode::Backspace => {
                    active_prompt.query.pop();
                    active_prompt.selected = 0;
                }
                KeyCode::Up => active_prompt.selected = active_prompt.selected.saturating_sub(1),
                KeyCode::Down => {
                    let len = prompt_matches(graph, active_prompt).len();
                    if len > 0 {
                        active_prompt.selected = (active_prompt.selected + 1).min(len - 1);
                    }
                }
                KeyCode::Char(c) => {
                    active_prompt.query.push(c);
                    active_prompt.selected = 0;
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Char('/') => prompt = Some(Prompt::new(PromptKind::Search)),
            KeyCode::Char('t') => prompt = Some(Prompt::new(PromptKind::Tag)),
            KeyCode::Char('f') => prompt = Some(Prompt::new(PromptKind::Folder)),
            KeyCode::Char('0') => view = View::default(),
            KeyCode::Char('[') if view.center.is_some() => {
                view.hops = view.hops.saturating_sub(1);
            }
            KeyCode::Char(']') if view.center.is_some() => {
                view.hops += 1;
            }
            _ => {
                if camera.handle_key(key) {
                    return Ok(());
                }
            }
        }
    }
}

/// Renders one frame at `camera`'s current state and prints it. `header`
/// (empty for the static, non-interactive fallback) is printed as plain
/// lines above the image, and its length determines how many terminal
/// rows are reserved for it; a non-empty header also clears the screen
/// first so each frame replaces the last rather than scrolling.
fn print_frame(
    graph: &Graph<Note, ()>,
    positions: &HashMap<NodeIndex, Position>,
    camera: &Camera,
    header: &[String],
) -> io::Result<()> {
    let mut stdout = io::stdout();
    if !header.is_empty() {
        execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        for line in header {
            write!(stdout, "{line}\r\n")?;
        }
    }

    let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));
    let (px_per_col, px_per_row) = cell_pixel_size();
    let cols = term_cols.saturating_sub(2).max(20);
    let header_rows = header.len() as u16;
    let rows = term_rows.saturating_sub(4 + header_rows).max(10);
    let width = (f64::from(cols) * px_per_col).round() as u32;
    let height = (f64::from(rows) * px_per_row).round() as u32;

    let pixmap = render_frame(graph, positions, camera, width, height);
    let image = pixmap_to_image(&pixmap);

    let config = viuer::Config {
        width: Some(u32::from(cols)),
        height: Some(u32::from(rows)),
        absolute_offset: false,
        ..Default::default()
    };
    viuer::print(&image, &config)
        .map(|_| ())
        .map_err(io::Error::other)
}

/// The pure compute step of a frame: project every node under `camera`,
/// derive the viewport, and rasterize. Split out from `print_frame` so the
/// projection/pan math can be unit tested without a real terminal or an
/// actual image print.
fn render_frame(
    graph: &Graph<Note, ()>,
    positions: &HashMap<NodeIndex, Position>,
    camera: &Camera,
    width: u32,
    height: u32,
) -> Pixmap {
    let camera_distance = effective_camera_distance(positions.values(), camera);
    let projected: HashMap<NodeIndex, Projected> = positions
        .iter()
        .map(|(&idx, pos)| {
            let (x, y, depth) = project(
                f64::from(pos.x),
                f64::from(pos.y),
                f64::from(pos.z),
                camera_distance,
                camera.rotation_x,
                camera.rotation_z,
            );
            (idx, Projected { x, y, depth })
        })
        .collect();

    let (mut x_bounds, mut y_bounds) = bounds(projected.values().map(|p| (p.x, p.y)));
    apply_pan(&mut x_bounds, &mut y_bounds, camera);

    rasterize(graph, &projected, x_bounds, y_bounds, width, height)
}

/// Shifts the viewport by the camera's pan, in units of the (isotropic)
/// viewport half-span — see `Camera`'s doc comment for why pan is stored
/// as a fraction rather than an absolute offset. This is a genuine camera
/// pan (the viewport moves), not a drag-to-scroll: panning "right" (`d`)
/// looks further right, so on-screen content drifts left, matching how
/// panning a camera works.
fn apply_pan(x_bounds: &mut [f64; 2], y_bounds: &mut [f64; 2], camera: &Camera) {
    let half_span = (x_bounds[1] - x_bounds[0]) / 2.0;
    let dx = camera.pan_x * half_span;
    let dy = camera.pan_y * half_span;
    x_bounds[0] += dx;
    x_bounds[1] += dx;
    y_bounds[0] += dy;
    y_bounds[1] += dy;
}

/// Real per-cell pixel dimensions (width, height), read from the
/// terminal's own report (`TIOCGWINSZ`'s `ws_xpixel`/`ws_ypixel`, exposed
/// as `crossterm::terminal::window_size()`) when available — replacing
/// the `PX_PER_COL`/`PX_PER_ROW` guess with the terminal's actual
/// font-cell aspect ratio, so an isotropic viewport (see `bounds()`)
/// actually renders isotropic instead of stretched by a mismatched guess.
/// Kitty fills these in correctly; crossterm's own docs note some
/// terminals report zero here ("unused" per the tty_ioctl man page), so
/// fall back to the empirical constants when that happens.
fn cell_pixel_size() -> (f64, f64) {
    match terminal::window_size() {
        Ok(ws) if ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0 => (
            f64::from(ws.width) / f64::from(ws.columns),
            f64::from(ws.height) / f64::from(ws.rows),
        ),
        _ => (f64::from(PX_PER_COL), f64::from(PX_PER_ROW)),
    }
}

/// Picks a camera distance proportional to the data's own scale: 2.5x the
/// farthest node from the origin, floored at `MIN_CAMERA_DISTANCE`.
///
/// **Bug this fixes:** the reference example hardcodes `CAMERA_DISTANCE =
/// 4.0`, which is fine for its data (pre-normalized to ~1.5-unit half
/// width), but `fdg-sim` positions routinely span tens of units (its
/// Fruchterman-Reingold `scale` parameter is 45.0). With a fixed small
/// camera distance, `camera_distance + z2` in `project()` can go negative
/// or near-zero for realistic graphs, making the perspective divide blow
/// up or flip sign — turning one node into a wild outlier that dominates
/// the viewport, for reasons unrelated to the actual layout. Caught by a
/// unit test asserting a closer point gets a larger depth value, which
/// failed with the old fixed constant.
fn camera_distance_for<'a>(positions: impl Iterator<Item = &'a Position>) -> f64 {
    let max_radius = positions
        .map(|p| (f64::from(p.x).powi(2) + f64::from(p.y).powi(2) + f64::from(p.z).powi(2)).sqrt())
        .fold(0.0_f64, f64::max);
    (max_radius * 2.5).max(MIN_CAMERA_DISTANCE)
}

/// `camera_distance_for` scaled by the live zoom (Phase 7): `zoom` is a
/// divisor, so zooming in (higher `zoom`) shrinks the effective distance.
/// The floor is re-applied after dividing, not just inherited from
/// `camera_distance_for`, because dividing can push a value that was
/// already above the floor back below it — see `ZOOM_MAX`'s doc comment
/// for why that re-flooring is what keeps this safe at max zoom.
fn effective_camera_distance<'a>(
    positions: impl Iterator<Item = &'a Position>,
    camera: &Camera,
) -> f64 {
    (camera_distance_for(positions) / camera.zoom).max(MIN_CAMERA_DISTANCE)
}

/// Rotates `(x, y, z)` around the Z then X axes and applies a perspective
/// divide, matching `Surface3D::project` in the reference example. Also
/// returns the perspective factor itself as a depth proxy for shading.
fn project(
    x: f64,
    y: f64,
    z: f64,
    camera_distance: f64,
    rotation_x: f64,
    rotation_z: f64,
) -> (f64, f64, f64) {
    let (sin_x, cos_x) = rotation_x.sin_cos();
    let (sin_z, cos_z) = rotation_z.sin_cos();

    let x1 = x * cos_z - y * sin_z;
    let y1 = x * sin_z + y * cos_z;

    let y2 = y1 * cos_x - z * sin_x;
    let z2 = y1 * sin_x + z * cos_x;

    let perspective = camera_distance / (camera_distance + z2);
    (x1 * perspective, y2 * perspective, perspective)
}

/// Computes an isotropic square viewport (equal span on both axes,
/// centered on the data, 10% margin) that fits every projected point.
/// Isotropic rather than padding x/y independently so the shape isn't
/// stretched just because the layout spread further on one axis than the
/// other, which `fdg-sim` doesn't guard against.
fn bounds(points: impl Iterator<Item = (f64, f64)>) -> ([f64; 2], [f64; 2]) {
    let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
    let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);

    for (x, y) in points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }

    if min_x > max_x {
        return ([-1.0, 1.0], [-1.0, 1.0]);
    }

    let center_x = (min_x + max_x) / 2.0;
    let center_y = (min_y + max_y) / 2.0;
    let half_span = ((max_x - min_x).max(max_y - min_y) / 2.0).max(0.5) * 1.1;

    (
        [center_x - half_span, center_x + half_span],
        [center_y - half_span, center_y + half_span],
    )
}

fn rasterize(
    graph: &Graph<Note, ()>,
    projected: &HashMap<NodeIndex, Projected>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    width: u32,
    height: u32,
) -> Pixmap {
    let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("nonzero dimensions");
    let (r, g, b) = BACKGROUND;
    pixmap.fill(Color::from_rgba8(r, g, b, 255));

    let (min_depth, max_depth) = depth_range(projected.values());

    for edge in graph.edge_indices() {
        let (from, to) = graph.edge_endpoints(edge).unwrap();
        // A self-loop has no visible geometry in a wireframe — expected
        // (see TODO.md Phase 5), not skipped as a bug fix.
        if from == to {
            continue;
        }
        let (Some(a), Some(b)) = (projected.get(&from), projected.get(&to)) else {
            continue;
        };
        draw_edge(
            &mut pixmap,
            a,
            b,
            x_bounds,
            y_bounds,
            width,
            height,
            min_depth,
            max_depth,
        );
    }

    for p in projected.values() {
        draw_node(
            &mut pixmap,
            p,
            x_bounds,
            y_bounds,
            width,
            height,
            min_depth,
            max_depth,
        );
    }

    pixmap
}

fn depth_range<'a>(points: impl Iterator<Item = &'a Projected>) -> (f64, f64) {
    let mut min_d = f64::MAX;
    let mut max_d = f64::MIN;
    for p in points {
        min_d = min_d.min(p.depth);
        max_d = max_d.max(p.depth);
    }
    if min_d > max_d {
        (0.0, 1.0)
    } else {
        (min_d, max_d)
    }
}

fn normalize_depth(depth: f64, min_depth: f64, max_depth: f64) -> f64 {
    if (max_depth - min_depth).abs() < f64::EPSILON {
        0.5
    } else {
        (depth - min_depth) / (max_depth - min_depth)
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn lerp_color(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    (
        lerp(f64::from(a.0), f64::from(b.0), t) as u8,
        lerp(f64::from(a.1), f64::from(b.1), t) as u8,
        lerp(f64::from(a.2), f64::from(b.2), t) as u8,
    )
}

fn to_pixel(
    x: f64,
    y: f64,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    width: u32,
    height: u32,
) -> (f32, f32) {
    let px = (x - x_bounds[0]) / (x_bounds[1] - x_bounds[0]) * f64::from(width);
    // Image rows increase downward; data y increases upward, so flip.
    let py = (1.0 - (y - y_bounds[0]) / (y_bounds[1] - y_bounds[0])) * f64::from(height);
    (px as f32, py as f32)
}

#[expect(clippy::too_many_arguments)]
fn draw_node(
    pixmap: &mut Pixmap,
    p: &Projected,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    width: u32,
    height: u32,
    min_depth: f64,
    max_depth: f64,
) {
    let (px, py) = to_pixel(p.x, p.y, x_bounds, y_bounds, width, height);
    let t = normalize_depth(p.depth, min_depth, max_depth);
    let radius = lerp(3.0, 8.0, t) as f32;
    let (r, g, b) = lerp_color(NODE_FAR, NODE_NEAR, t);

    let mut pb = PathBuilder::new();
    pb.push_circle(px, py, radius);
    let Some(path) = pb.finish() else { return };

    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, 255);
    paint.anti_alias = true;

    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::default(), None);
}

#[expect(clippy::too_many_arguments)]
fn draw_edge(
    pixmap: &mut Pixmap,
    a: &Projected,
    b: &Projected,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
    width: u32,
    height: u32,
    min_depth: f64,
    max_depth: f64,
) {
    let (x1, y1) = to_pixel(a.x, a.y, x_bounds, y_bounds, width, height);
    let (x2, y2) = to_pixel(b.x, b.y, x_bounds, y_bounds, width, height);
    let t = normalize_depth((a.depth + b.depth) / 2.0, min_depth, max_depth);
    let alpha = lerp(70.0, 160.0, t) as u8;

    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    let Some(path) = pb.finish() else { return };

    let (r, g, b) = EDGE_COLOR;
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, alpha);
    paint.anti_alias = true;

    let stroke = Stroke {
        width: 1.4,
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::default(), None);
}

/// `Pixmap` is filled with an opaque background before anything is drawn
/// on top, so every pixel ends up fully opaque — premultiplied and
/// straight alpha coincide at alpha 255, so the raw bytes can be handed
/// to `image` directly without an unpremultiply pass.
fn pixmap_to_image(pixmap: &Pixmap) -> DynamicImage {
    let buf = RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.data().to_vec())
        .expect("pixmap dimensions match its own buffer length");
    DynamicImage::ImageRgba8(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, tags: &[&str]) -> Note {
        Note {
            path: path.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            aliases: Vec::new(),
        }
    }

    fn fixture_graph() -> Graph<Note, ()> {
        let mut graph = Graph::new();
        graph.add_node(note("projects/alpha.md", &["claim"]));
        graph.add_node(note("projects/beta.md", &["source"]));
        graph.add_node(note("journal/today.md", &[]));
        graph
    }

    #[test]
    fn prompt_matches_search_filters_case_insensitively_by_substring() {
        let graph = fixture_graph();
        let prompt = Prompt {
            kind: PromptKind::Search,
            query: "ALPHA".to_string(),
            selected: 0,
        };
        assert_eq!(prompt_matches(&graph, &prompt), vec!["projects/alpha.md"]);
    }

    #[test]
    fn prompt_matches_tag_lists_distinct_sorted_tags() {
        let graph = fixture_graph();
        let prompt = Prompt {
            kind: PromptKind::Tag,
            query: String::new(),
            selected: 0,
        };
        assert_eq!(prompt_matches(&graph, &prompt), vec!["claim", "source"]);
    }

    #[test]
    fn prompt_matches_folder_lists_distinct_folders() {
        let graph = fixture_graph();
        let prompt = Prompt {
            kind: PromptKind::Folder,
            query: String::new(),
            selected: 0,
        };
        assert_eq!(prompt_matches(&graph, &prompt), vec!["journal", "projects"]);
    }

    #[test]
    fn prompt_matches_truncates_to_the_max() {
        let mut graph = Graph::new();
        for i in 0..(MAX_PROMPT_MATCHES + 5) {
            graph.add_node(note(&format!("note-{i}.md"), &[]));
        }
        let prompt = Prompt {
            kind: PromptKind::Search,
            query: String::new(),
            selected: 0,
        };
        assert_eq!(prompt_matches(&graph, &prompt).len(), MAX_PROMPT_MATCHES);
    }

    #[test]
    fn search_selection_centers_the_view_and_applies_the_default_hop_count() {
        let graph = fixture_graph();
        let prompt = Prompt {
            kind: PromptKind::Search,
            query: String::new(),
            selected: 0,
        };
        let mut view = View::default();

        apply_prompt_selection(&graph, &prompt, "projects/alpha.md", &mut view);

        assert_eq!(view.center, Some(NodeIndex::new(0)));
        assert_eq!(view.hops, DEFAULT_HOPS);
    }

    #[test]
    fn re_centering_search_keeps_the_existing_hop_count() {
        let graph = fixture_graph();
        let prompt = Prompt {
            kind: PromptKind::Search,
            query: String::new(),
            selected: 0,
        };
        let mut view = View {
            center: Some(NodeIndex::new(0)),
            hops: 5,
            ..View::default()
        };

        apply_prompt_selection(&graph, &prompt, "journal/today.md", &mut view);

        assert_eq!(view.center, Some(NodeIndex::new(2)));
        assert_eq!(view.hops, 5);
    }

    #[test]
    fn tag_and_folder_selection_set_the_matching_filter() {
        let graph = fixture_graph();
        let mut view = View::default();

        apply_prompt_selection(
            &graph,
            &Prompt {
                kind: PromptKind::Tag,
                query: String::new(),
                selected: 0,
            },
            "claim",
            &mut view,
        );
        assert_eq!(view.tag.as_deref(), Some("claim"));

        apply_prompt_selection(
            &graph,
            &Prompt {
                kind: PromptKind::Folder,
                query: String::new(),
                selected: 0,
            },
            "journal",
            &mut view,
        );
        assert_eq!(view.folder.as_deref(), Some("journal"));
    }

    #[test]
    fn view_status_reports_whole_vault_when_unfiltered() {
        let graph = fixture_graph();
        assert_eq!(view_status(&graph, &View::default()), "view: whole vault (3 notes)");
    }

    #[test]
    fn view_status_reports_the_neighborhood_center_and_hop_count() {
        let graph = fixture_graph();
        let view = View {
            center: Some(NodeIndex::new(0)),
            hops: 1,
            ..View::default()
        };
        assert_eq!(
            view_status(&graph, &view),
            "view: projects/alpha.md (1 hop)"
        );
    }

    #[test]
    fn origin_projects_to_origin_regardless_of_camera_angle() {
        let (x, y, _depth) = project(
            0.0,
            0.0,
            0.0,
            MIN_CAMERA_DISTANCE,
            DEFAULT_ROTATION_X,
            DEFAULT_ROTATION_Z,
        );
        assert_eq!((x, y), (0.0, 0.0));
    }

    #[test]
    fn a_point_on_the_z_axis_projects_with_zero_screen_x() {
        // x1 = x*cos_z - y*sin_z depends only on x and y; with x = y = 0
        // the screen x-coordinate is always exactly 0, whatever z (depth)
        // is — only screen y shifts as the camera tilt mixes z into it.
        let (x, _y, _depth) = project(
            0.0,
            0.0,
            5.0,
            MIN_CAMERA_DISTANCE,
            DEFAULT_ROTATION_X,
            DEFAULT_ROTATION_Z,
        );
        assert_eq!(x, 0.0);
    }

    #[test]
    fn closer_points_have_a_larger_depth_value() {
        // Camera looks down +z (roughly); a point with a smaller z is
        // closer to the camera and should get a larger perspective factor.
        // Use a camera distance that comfortably exceeds these points'
        // scale, same as `camera_distance_for` would derive for real data
        // of this magnitude.
        let camera_distance = 20.0;
        let (_, _, near) = project(
            0.0,
            0.0,
            -5.0,
            camera_distance,
            DEFAULT_ROTATION_X,
            DEFAULT_ROTATION_Z,
        );
        let (_, _, far) = project(
            0.0,
            0.0,
            5.0,
            camera_distance,
            DEFAULT_ROTATION_X,
            DEFAULT_ROTATION_Z,
        );
        assert!(near > far);
    }

    #[test]
    fn camera_distance_scales_with_the_data_and_keeps_the_denominator_positive() {
        // Regression test: a fixed camera distance (the reference
        // example's constant, 4.0) is smaller than realistic fdg-sim
        // position magnitudes, so `camera_distance + z2` in `project()`
        // could go negative — the perspective divide would blow up or
        // flip sign, turning one node into a viewport-dominating outlier.
        let far_positions = [Position {
            x: 40.0,
            y: 0.0,
            z: 40.0,
        }];
        let camera_distance = camera_distance_for(far_positions.iter());

        for p in &far_positions {
            let (_, _, depth) = project(
                f64::from(p.x),
                f64::from(p.y),
                f64::from(p.z),
                camera_distance,
                DEFAULT_ROTATION_X,
                DEFAULT_ROTATION_Z,
            );
            assert!(depth.is_finite() && depth > 0.0);
        }
    }

    #[test]
    fn effective_camera_distance_after_max_zoom_stays_safely_above_the_data_radius() {
        // Regression test for the ZOOM_MAX safety margin documented on its
        // own const: zooming all the way in must never let camera_distance
        // fall to or below the data's own radius, or `project()` risks the
        // same sign-flip bug `camera_distance_for` was written to avoid.
        let positions = [
            Position { x: 2.0, y: 0.0, z: 0.0 },  // small graph, hits the floor
            Position { x: 20.0, y: 0.0, z: 0.0 }, // large graph, floor doesn't bind
        ];
        let mut camera = Camera::default();
        for _ in 0..50 {
            camera.zoom = (camera.zoom * ZOOM_STEP).min(ZOOM_MAX);
        }
        assert_eq!(camera.zoom, ZOOM_MAX);

        for p in &positions {
            let radius = f64::from(p.x);
            let distance = effective_camera_distance(std::iter::once(p), &camera);
            assert!(distance > radius);
        }
    }

    #[test]
    fn cell_pixel_size_is_always_positive() {
        // Can't force a real terminal's window_size() report in a test
        // environment (there may be no tty at all here), so this only
        // guards that whichever branch runs — real report or the
        // PX_PER_COL/PX_PER_ROW fallback — produces usable dimensions,
        // never zero or negative.
        let (w, h) = cell_pixel_size();
        assert!(w > 0.0 && h > 0.0);
    }

    #[test]
    fn camera_distance_has_a_floor_for_a_near_empty_graph() {
        assert_eq!(camera_distance_for(std::iter::empty()), MIN_CAMERA_DISTANCE);
    }

    #[test]
    fn bounds_of_no_points_falls_back_to_a_default_viewport() {
        assert_eq!(bounds(std::iter::empty()), ([-1.0, 1.0], [-1.0, 1.0]));
    }

    #[test]
    fn bounds_are_isotropic_even_when_data_spreads_unevenly() {
        let points = [(-50.0, -1.0), (50.0, 1.0)];
        let (x_bounds, y_bounds) = bounds(points.into_iter());

        let x_span = x_bounds[1] - x_bounds[0];
        let y_span = y_bounds[1] - y_bounds[0];
        assert!((x_span - y_span).abs() < f64::EPSILON);
    }

    #[test]
    fn bounds_of_a_single_point_still_produces_a_visible_viewport() {
        let (x_bounds, y_bounds) = bounds(std::iter::once((3.0, 3.0)));

        assert!(x_bounds[0] < 3.0 && x_bounds[1] > 3.0);
        assert!(y_bounds[0] < 3.0 && y_bounds[1] > 3.0);
    }

    #[test]
    fn panning_right_shifts_the_viewport_toward_positive_x() {
        let camera = Camera {
            pan_x: 1.0,
            ..Camera::default()
        };
        let mut x_bounds = [-1.0, 1.0];
        let mut y_bounds = [-1.0, 1.0];

        apply_pan(&mut x_bounds, &mut y_bounds, &camera);

        assert_eq!(x_bounds, [0.0, 2.0]);
        assert_eq!(y_bounds, [-1.0, 1.0]);
    }

    #[test]
    fn normalize_depth_of_a_flat_range_is_the_midpoint() {
        assert_eq!(normalize_depth(5.0, 5.0, 5.0), 0.5);
    }

    #[test]
    fn lerp_color_at_extremes_returns_the_endpoints() {
        assert_eq!(lerp_color(NODE_FAR, NODE_NEAR, 0.0), NODE_FAR);
        assert_eq!(lerp_color(NODE_FAR, NODE_NEAR, 1.0), NODE_NEAR);
    }

    #[test]
    fn q_and_ctrl_c_signal_quit_but_plain_c_does_not() {
        let mut camera = Camera::default();
        assert!(camera.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(camera.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(camera.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!camera.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)));
    }

    #[test]
    fn arrow_keys_and_hjkl_both_orbit() {
        let mut by_arrows = Camera::default();
        by_arrows.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        by_arrows.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

        let mut by_hjkl = Camera::default();
        by_hjkl.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
        by_hjkl.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));

        assert_eq!(by_arrows.rotation_z, by_hjkl.rotation_z);
        assert_eq!(by_arrows.rotation_x, by_hjkl.rotation_x);
    }

    #[test]
    fn r_resets_camera_to_defaults_after_it_was_moved() {
        let mut camera = Camera::default();
        camera.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        camera.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        camera.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));

        camera.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));

        assert_eq!(camera.rotation_z, DEFAULT_ROTATION_Z);
        assert_eq!(camera.rotation_x, DEFAULT_ROTATION_X);
        assert_eq!(camera.zoom, 1.0);
        assert_eq!(camera.pan_x, 0.0);
    }

    #[test]
    fn zoom_stays_within_bounds_after_many_presses() {
        let mut camera = Camera::default();
        for _ in 0..100 {
            camera.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        }
        assert_eq!(camera.zoom, ZOOM_MAX);

        for _ in 0..100 {
            camera.handle_key(KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE));
        }
        assert_eq!(camera.zoom, ZOOM_MIN);
    }

    #[test]
    fn rasterize_produces_a_pixmap_of_the_requested_size() {
        let mut graph = Graph::new();
        let a = graph.add_node(Note {
            path: "a.md".into(),
            tags: Vec::new(),
            aliases: Vec::new(),
        });
        let b = graph.add_node(Note {
            path: "b.md".into(),
            tags: Vec::new(),
            aliases: Vec::new(),
        });
        graph.add_edge(a, b, ());
        graph.add_edge(a, a, ()); // self-loop must not panic

        let mut projected = HashMap::new();
        projected.insert(
            a,
            Projected {
                x: -1.0,
                y: 0.0,
                depth: 1.0,
            },
        );
        projected.insert(
            b,
            Projected {
                x: 1.0,
                y: 0.0,
                depth: 0.8,
            },
        );

        let pixmap = rasterize(&graph, &projected, [-2.0, 2.0], [-2.0, 2.0], 40, 40);

        assert_eq!(pixmap.width(), 40);
        assert_eq!(pixmap.height(), 40);
    }

    #[test]
    fn render_frame_produces_a_pixmap_of_the_requested_size_end_to_end() {
        // Exercises the full per-frame pipeline (project -> bounds -> pan
        // -> rasterize) the interactive loop calls every keypress.
        let mut graph = Graph::new();
        let a = graph.add_node(Note {
            path: "a.md".into(),
            tags: Vec::new(),
            aliases: Vec::new(),
        });
        let b = graph.add_node(Note {
            path: "b.md".into(),
            tags: Vec::new(),
            aliases: Vec::new(),
        });
        graph.add_edge(a, b, ());

        let mut positions = HashMap::new();
        positions.insert(a, Position { x: -3.0, y: 1.0, z: 0.0 });
        positions.insert(b, Position { x: 3.0, y: -1.0, z: 2.0 });

        let mut camera = Camera::default();
        camera.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        camera.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));

        let pixmap = render_frame(&graph, &positions, &camera, 50, 50);

        assert_eq!(pixmap.width(), 50);
        assert_eq!(pixmap.height(), 50);
    }
}
