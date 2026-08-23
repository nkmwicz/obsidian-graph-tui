use std::collections::HashMap;
use std::io;

use petgraph::graph::{Graph, NodeIndex};
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::Span;
use ratatui::widgets::canvas::{Canvas, Context, Line};

use crate::layout::Position;
use crate::vault::Note;

/// Fixed camera orientation/distance for this static frame — same
/// rotate-Z-then-rotate-X + perspective-divide scheme as ratatui's
/// `volatility-surface` example (`examples/apps/volatility-surface/src/
/// display/surface_3d.rs`), the project's designated reference for
/// Canvas+Braille 3D projection (see CLAUDE.md). Phase 6 makes these live
/// (orbit/zoom via keyboard) instead of fixed.
const ROTATION_X: f64 = 0.6;
const ROTATION_Z: f64 = 0.3;
const CAMERA_DISTANCE: f64 = 4.0;

/// Draws one static wireframe frame of `graph` laid out at `positions`,
/// then blocks until any key is pressed before restoring the terminal.
/// No camera interaction yet (Phase 6) — this only proves the pipeline
/// end to end: parse -> graph -> layout -> a visible 3D shape.
pub fn run(graph: &Graph<Note, ()>, positions: &HashMap<NodeIndex, Position>) -> io::Result<()> {
    let projected: HashMap<NodeIndex, (f64, f64)> = positions
        .iter()
        .map(|(&idx, pos)| {
            (
                idx,
                project(f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)),
            )
        })
        .collect();
    let (x_bounds, y_bounds) = bounds(projected.values().copied());

    ratatui::run(|terminal| {
        terminal.draw(|frame| draw(frame, graph, &projected, x_bounds, y_bounds))?;
        wait_for_keypress()
    })
}

/// Rotates `(x, y, z)` around the Z then X axes and applies a perspective
/// divide, matching `Surface3D::project` in the reference example.
fn project(x: f64, y: f64, z: f64) -> (f64, f64) {
    let (sin_x, cos_x) = ROTATION_X.sin_cos();
    let (sin_z, cos_z) = ROTATION_Z.sin_cos();

    let x1 = x * cos_z - y * sin_z;
    let y1 = x * sin_z + y * cos_z;

    let y2 = y1 * cos_x - z * sin_x;
    let z2 = y1 * sin_x + z * cos_x;

    let perspective = CAMERA_DISTANCE / (CAMERA_DISTANCE + z2);
    (x1 * perspective, y2 * perspective)
}

/// Computes Canvas `x_bounds`/`y_bounds` that fit every projected point
/// with a 10% margin. Unlike the reference example (whose input data is
/// pre-normalized to a known range), `fdg-sim` positions are in arbitrary
/// simulation units that scale with graph size, so the viewport has to be
/// derived from the actual projected points rather than fixed.
///
/// The span is the same on both axes (an equal-radius square around the
/// data's center), not each axis's own independent data range: Braille's
/// 2-wide×4-tall sub-cell grid already approximates a square dot for a
/// typical monospace terminal font, so an isotropic mapping here is what
/// keeps the projected shape from looking stretched — padding x and y
/// independently would distort it whenever the layout happens to spread
/// further along one axis than the other, which `fdg-sim` doesn't
/// guarantee against.
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

fn draw(
    frame: &mut Frame,
    graph: &Graph<Note, ()>,
    projected: &HashMap<NodeIndex, (f64, f64)>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
) {
    let area = frame.area();
    let dot = dot_size(area, x_bounds, y_bounds);
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .paint(|ctx| {
            for edge in graph.edge_indices() {
                let (from, to) = graph.edge_endpoints(edge).unwrap();
                // A self-loop has no visible geometry in a wireframe —
                // expected (see TODO.md Phase 5), not skipped as a bug fix.
                if from == to {
                    continue;
                }
                let (Some(&(x1, y1)), Some(&(x2, y2))) = (projected.get(&from), projected.get(&to))
                else {
                    continue;
                };
                draw_edge(ctx, x1, y1, x2, y2, dot, Color::Gray);
            }

            // Nodes are printed as a full-cell glyph via `ctx.print()`
            // rather than drawn with the canvas's Braille marker: a
            // Braille-marker point is a single sub-cell dot, visually
            // identical to the dots making up an edge's line, so nodes
            // and edges were indistinguishable (see CLAUDE.md's Phase 5
            // note on this). `print()` always renders on top of the
            // marker layer regardless of the canvas's configured marker,
            // giving nodes a distinctly bigger, brighter mark.
            for &(x, y) in projected.values() {
                ctx.print(x, y, Span::styled("●", Style::new().fg(Color::Cyan)));
            }
        });

    frame.render_widget(canvas, area);
}

/// The size, in canvas data units, of one Braille sub-cell dot along each
/// axis (the canvas maps `x_bounds`/`y_bounds` onto a `width*2` ×
/// `height*4` dot grid).
fn dot_size(area: Rect, x_bounds: [f64; 2], y_bounds: [f64; 2]) -> (f64, f64) {
    let dots_wide = f64::from(area.width) * 2.0;
    let dots_tall = f64::from(area.height) * 4.0;
    (
        (x_bounds[1] - x_bounds[0]) / dots_wide.max(1.0),
        (y_bounds[1] - y_bounds[0]) / dots_tall.max(1.0),
    )
}

/// Draws an edge as two parallel Bresenham lines, offset from each other
/// by roughly one Braille dot perpendicular to the edge. A single-dot-wide
/// diagonal line renders as a thread of visually disconnected Braille
/// glyphs (each terminal cell shows only 1-2 of its 8 dots, so adjacent
/// cells along the line don't read as continuous); two adjacent dot-wide
/// lines fill enough of each cell to read as one continuous stroke.
fn draw_edge(ctx: &mut Context, x1: f64, y1: f64, x2: f64, y2: f64, dot: (f64, f64), color: Color) {
    ctx.draw(&Line {
        x1,
        y1,
        x2,
        y2,
        color,
    });

    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = dx.hypot(dy);
    if len < f64::EPSILON {
        return;
    }
    let (ox, oy) = (-dy / len * dot.0, dx / len * dot.1);
    ctx.draw(&Line {
        x1: x1 + ox,
        y1: y1 + oy,
        x2: x2 + ox,
        y2: y2 + oy,
        color,
    });
}

fn wait_for_keypress() -> io::Result<()> {
    loop {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_projects_to_origin_regardless_of_camera_angle() {
        assert_eq!(project(0.0, 0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn a_point_on_the_z_axis_projects_with_zero_screen_x() {
        // x1 = x*cos_z - y*sin_z depends only on x and y; with x = y = 0
        // the screen x-coordinate is always exactly 0, whatever z (depth)
        // is — only screen y shifts as the fixed camera tilt (ROTATION_X)
        // mixes z into it.
        let (x, _y) = project(0.0, 0.0, 5.0);
        assert_eq!(x, 0.0);
    }

    #[test]
    fn bounds_of_no_points_falls_back_to_a_default_viewport() {
        assert_eq!(bounds(std::iter::empty()), ([-1.0, 1.0], [-1.0, 1.0]));
    }

    #[test]
    fn bounds_pad_around_the_center_of_the_points() {
        let points = [(0.0, 0.0), (10.0, 20.0)];
        let (x_bounds, y_bounds) = bounds(points.into_iter());

        // Center (5, 10); half-span is the larger axis's span (20) / 2,
        // padded 10% => 11.
        assert_eq!(x_bounds, [-6.0, 16.0]);
        assert_eq!(y_bounds, [-1.0, 21.0]);
    }

    #[test]
    fn bounds_are_isotropic_even_when_data_spreads_unevenly() {
        // Wide in x, narrow in y — the projected shape shouldn't stretch
        // just because the layout happened to spread further on one axis.
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
}
