use std::collections::HashMap;
use std::io;

use petgraph::graph::{Graph, NodeIndex};
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::style::Color;
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Line, Points};

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

    let pad = |min: f64, max: f64| {
        let span = (max - min).max(1.0);
        let margin = span * 0.1;
        [min - margin, max + margin]
    };

    (pad(min_x, max_x), pad(min_y, max_y))
}

fn draw(
    frame: &mut Frame,
    graph: &Graph<Note, ()>,
    projected: &HashMap<NodeIndex, (f64, f64)>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
) {
    let area = frame.area();
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
                ctx.draw(&Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    color: Color::DarkGray,
                });
            }

            let points: Vec<(f64, f64)> = projected.values().copied().collect();
            ctx.draw(&Points {
                coords: &points,
                color: Color::Cyan,
            });
        });

    frame.render_widget(canvas, area);
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
    fn bounds_pad_around_the_min_and_max_of_the_points() {
        let points = [(0.0, 0.0), (10.0, 20.0)];
        let (x_bounds, y_bounds) = bounds(points.into_iter());

        // 10% margin on a span of 10 (x) / 20 (y).
        assert_eq!(x_bounds, [-1.0, 11.0]);
        assert_eq!(y_bounds, [-2.0, 22.0]);
    }

    #[test]
    fn bounds_of_a_single_point_still_produces_a_visible_viewport() {
        let (x_bounds, y_bounds) = bounds(std::iter::once((3.0, 3.0)));

        assert!(x_bounds[0] < 3.0 && x_bounds[1] > 3.0);
        assert!(y_bounds[0] < 3.0 && y_bounds[1] > 3.0);
    }
}
