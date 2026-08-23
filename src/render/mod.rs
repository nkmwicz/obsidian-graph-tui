use std::collections::HashMap;
use std::io;

use image::{DynamicImage, RgbaImage};
use petgraph::graph::{Graph, NodeIndex};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::layout::Position;
use crate::vault::Note;

/// Fixed camera orientation for this static frame — same
/// rotate-Z-then-rotate-X + perspective-divide scheme as ratatui's
/// `volatility-surface` example (`examples/apps/volatility-surface/src/
/// display/surface_3d.rs`), the project's original reference for the
/// projection math (see CLAUDE.md). Phase 6 makes these live (orbit/zoom
/// via keyboard) instead of fixed.
const ROTATION_X: f64 = 0.6;
const ROTATION_Z: f64 = 0.3;

/// Floor for the derived camera distance (see `camera_distance_for`), so
/// a near-empty or single-node graph (radius ~0) still gets a sane camera
/// placement instead of one that's degenerately close.
const MIN_CAMERA_DISTANCE: f64 = 4.0;

/// Raster oversampling per terminal cell. Terminal fonts are roughly
/// twice as tall as wide in pixels, so rows get twice the per-cell pixel
/// budget of columns — matched empirically against a real Kitty terminal,
/// not derived from queryable font metrics (none are available to a
/// terminal application).
const PX_PER_COL: u32 = 10;
const PX_PER_ROW: u32 = 20;

const BACKGROUND: (u8, u8, u8) = (8, 10, 18);
const NODE_FAR: (u8, u8, u8) = (30, 90, 110);
const NODE_NEAR: (u8, u8, u8) = (140, 230, 255);
const EDGE_COLOR: (u8, u8, u8) = (120, 150, 170);

struct Projected {
    x: f64,
    y: f64,
    /// `camera_distance / (camera_distance + view_space_z)` from
    /// `project()` — larger means closer to the camera. Used as a depth
    /// proxy for size/color shading, the same role depth plays in a point
    /// cloud renderer (e.g. deck.gl's `PointCloudLayer`).
    depth: f64,
}

/// Rasterizes `graph` laid out at `positions` into an anti-aliased image
/// (nodes sized/colored by depth, edges as thin depth-shaded strokes) and
/// prints it inline via the terminal's image protocol (Kitty/iTerm/Sixel,
/// auto-detected; falls back to half-block characters if none is
/// available — see CLAUDE.md's rendering section for why this replaced
/// the earlier `ratatui` Canvas+Braille renderer).
pub fn run(graph: &Graph<Note, ()>, positions: &HashMap<NodeIndex, Position>) -> io::Result<()> {
    let camera_distance = camera_distance_for(positions.values());
    let projected: HashMap<NodeIndex, Projected> = positions
        .iter()
        .map(|(&idx, pos)| {
            let (x, y, depth) = project(
                f64::from(pos.x),
                f64::from(pos.y),
                f64::from(pos.z),
                camera_distance,
            );
            (idx, Projected { x, y, depth })
        })
        .collect();
    let (x_bounds, y_bounds) = bounds(projected.values().map(|p| (p.x, p.y)));

    let (term_cols, term_rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    let cols = term_cols.saturating_sub(2).max(20);
    let rows = term_rows.saturating_sub(4).max(10);
    let width = u32::from(cols) * PX_PER_COL;
    let height = u32::from(rows) * PX_PER_ROW;

    let pixmap = rasterize(graph, &projected, x_bounds, y_bounds, width, height);
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

/// Rotates `(x, y, z)` around the Z then X axes and applies a perspective
/// divide, matching `Surface3D::project` in the reference example. Also
/// returns the perspective factor itself as a depth proxy for shading.
fn project(x: f64, y: f64, z: f64, camera_distance: f64) -> (f64, f64, f64) {
    let (sin_x, cos_x) = ROTATION_X.sin_cos();
    let (sin_z, cos_z) = ROTATION_Z.sin_cos();

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

    #[test]
    fn origin_projects_to_origin_regardless_of_camera_angle() {
        let (x, y, _depth) = project(0.0, 0.0, 0.0, MIN_CAMERA_DISTANCE);
        assert_eq!((x, y), (0.0, 0.0));
    }

    #[test]
    fn a_point_on_the_z_axis_projects_with_zero_screen_x() {
        // x1 = x*cos_z - y*sin_z depends only on x and y; with x = y = 0
        // the screen x-coordinate is always exactly 0, whatever z (depth)
        // is — only screen y shifts as the fixed camera tilt (ROTATION_X)
        // mixes z into it.
        let (x, _y, _depth) = project(0.0, 0.0, 5.0, MIN_CAMERA_DISTANCE);
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
        let (_, _, near) = project(0.0, 0.0, -5.0, camera_distance);
        let (_, _, far) = project(0.0, 0.0, 5.0, camera_distance);
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
            );
            assert!(depth.is_finite() && depth > 0.0);
        }
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
    fn normalize_depth_of_a_flat_range_is_the_midpoint() {
        assert_eq!(normalize_depth(5.0, 5.0, 5.0), 0.5);
    }

    #[test]
    fn lerp_color_at_extremes_returns_the_endpoints() {
        assert_eq!(lerp_color(NODE_FAR, NODE_NEAR, 0.0), NODE_FAR);
        assert_eq!(lerp_color(NODE_FAR, NODE_NEAR, 1.0), NODE_NEAR);
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
}
