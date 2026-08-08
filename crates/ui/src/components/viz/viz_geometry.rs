//! Pure geometry for the market viz primitives.
//!
//! Ports `components/viz/core/geometry.ts` from the Bazaar web client so the
//! two implementations stay numerically equivalent. Coordinates are logical
//! units (SVG-style, y grows downward); `polar` uses 0° = east, 90° = south.

/// The drawable footprint of a node an edge can anchor against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VizShape {
    Circle { radius: f32 },
    Rect { width: f32, height: f32 },
}

/// A positioned shape that edges, ports, and rings anchor to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VizAnchor {
    pub x: f32,
    pub y: f32,
    pub shape: VizShape,
}

impl VizAnchor {
    pub fn circle(x: f32, y: f32, radius: f32) -> Self {
        Self {
            x,
            y,
            shape: VizShape::Circle { radius },
        }
    }

    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            shape: VizShape::Rect { width, height },
        }
    }
}

/// Distance from the anchor center to its surface along the unit
/// direction `(ux, uy)`. Rects use slab intersection on the half extents.
pub fn center_to_surface_distance(anchor: &VizAnchor, ux: f32, uy: f32) -> f32 {
    match anchor.shape {
        VizShape::Circle { radius } => radius,
        VizShape::Rect { width, height } => {
            let half_width = width / 2.0;
            let half_height = height / 2.0;
            if half_width <= 0.0 || half_height <= 0.0 {
                return 0.0;
            }
            let horizontal = if ux.abs() < 1e-6 {
                f32::INFINITY
            } else {
                half_width / ux.abs()
            };
            let vertical = if uy.abs() < 1e-6 {
                f32::INFINITY
            } else {
                half_height / uy.abs()
            };
            horizontal.min(vertical)
        }
    }
}

/// The point on the anchor's surface along `(ux, uy)`, pushed out by `padding`.
pub fn surface_point(anchor: &VizAnchor, ux: f32, uy: f32, padding: f32) -> (f32, f32) {
    let distance = center_to_surface_distance(anchor, ux, uy) + padding;
    (anchor.x + ux * distance, anchor.y + uy * distance)
}

/// A straight edge between two anchors, surface-to-surface — never
/// center-to-center, so strokes meet node rims cleanly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VizEdgeGeometry {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub ux: f32,
    pub uy: f32,
    pub length: f32,
    pub approach_deg: f32,
}

pub fn edge_geometry(
    from: &VizAnchor,
    to: &VizAnchor,
    padding_from: f32,
    padding_to: f32,
) -> VizEdgeGeometry {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let center_distance = (dx * dx + dy * dy).sqrt().max(1e-6);
    let ux = dx / center_distance;
    let uy = dy / center_distance;
    let (x0, y0) = surface_point(from, ux, uy, padding_from);
    let (x1, y1) = surface_point(to, -ux, -uy, padding_to);
    let length = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    VizEdgeGeometry {
        x0,
        y0,
        x1,
        y1,
        ux,
        uy,
        length,
        approach_deg: uy.atan2(ux).to_degrees(),
    }
}

/// Polar coordinates with 0° = east and 90° = south (y grows downward).
pub fn polar(center_x: f32, center_y: f32, radius: f32, angle_deg: f32) -> (f32, f32) {
    let radians = angle_deg.to_radians();
    (
        center_x + radius * radians.cos(),
        center_y + radius * radians.sin(),
    )
}

pub fn perimeter(anchor: &VizAnchor) -> f32 {
    match anchor.shape {
        VizShape::Circle { radius } => 2.0 * std::f32::consts::PI * radius,
        VizShape::Rect { width, height } => 2.0 * (width + height),
    }
}

/// Scales a dash cycle so a whole number of dashes fits the perimeter —
/// the ring never ends mid-dash.
pub fn even_dash(perimeter_length: f32, dash: f32, gap: f32) -> (f32, f32) {
    let cycle = dash + gap;
    if perimeter_length <= 0.0 || cycle <= 0.0 {
        return (dash, gap);
    }
    let count = (perimeter_length / cycle).round().max(1.0);
    let unit = perimeter_length / count;
    let dash_length = unit * (dash / cycle);
    (dash_length, unit - dash_length)
}

/// An arrowhead drawn as a concentric arc hugging a circular target's rim
/// rather than a triangle floating in space. Returns the arc's start point,
/// end point, and radius; `approach_deg` is the edge's direction of travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VizArcHead {
    pub start: (f32, f32),
    pub end: (f32, f32),
    pub radius: f32,
}

pub fn arc_head(
    target: &VizAnchor,
    approach_deg: f32,
    gap: f32,
    spread_deg: f32,
) -> Option<VizArcHead> {
    let VizShape::Circle { radius } = target.shape else {
        return None;
    };
    let arc_radius = radius + gap;
    let facing = approach_deg + 180.0;
    Some(VizArcHead {
        start: polar(target.x, target.y, arc_radius, facing - spread_deg),
        end: polar(target.x, target.y, arc_radius, facing + spread_deg),
        radius: arc_radius,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn circle_surface_distance_is_the_radius() {
        let anchor = VizAnchor::circle(0.0, 0.0, 22.0);
        assert!(close(center_to_surface_distance(&anchor, 1.0, 0.0), 22.0));
        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(
            center_to_surface_distance(&anchor, diagonal, diagonal),
            22.0
        ));
    }

    #[test]
    fn rect_surface_distance_uses_slab_intersection() {
        let anchor = VizAnchor::rect(0.0, 0.0, 80.0, 24.0);
        assert!(close(center_to_surface_distance(&anchor, 1.0, 0.0), 40.0));
        assert!(close(center_to_surface_distance(&anchor, 0.0, 1.0), 12.0));
        // 45°: the shorter half extent wins — 12 / (√2/2) ≈ 16.97.
        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(
            center_to_surface_distance(&anchor, diagonal, diagonal),
            12.0 / diagonal
        ));
    }

    #[test]
    fn edges_anchor_surface_to_surface() {
        let from = VizAnchor::circle(0.0, 0.0, 10.0);
        let to = VizAnchor::circle(100.0, 0.0, 20.0);
        let geometry = edge_geometry(&from, &to, 2.0, 2.0);
        assert!(close(geometry.x0, 12.0));
        assert!(close(geometry.x1, 78.0));
        assert!(close(geometry.length, 66.0));
        assert!(close(geometry.approach_deg, 0.0));
    }

    #[test]
    fn polar_zero_is_east_and_ninety_is_south() {
        let (x, y) = polar(0.0, 0.0, 10.0, 0.0);
        assert!(close(x, 10.0) && close(y, 0.0));
        let (x, y) = polar(0.0, 0.0, 10.0, 90.0);
        assert!(close(x, 0.0) && close(y, 10.0));
    }

    #[test]
    fn even_dash_fits_a_whole_number_of_dashes() {
        let circumference = perimeter(&VizAnchor::circle(0.0, 0.0, 27.0));
        let (dash, gap) = even_dash(circumference, 3.0, 3.0);
        let cycle = dash + gap;
        let count = circumference / cycle;
        assert!(close(count, count.round()));
        assert!(close(dash, gap));
    }

    #[test]
    fn arc_head_hugs_the_target_rim() {
        let target = VizAnchor::circle(100.0, 0.0, 20.0);
        let head = arc_head(&target, 0.0, 2.5, 42.0).expect("circle target has an arc head");
        assert!(close(head.radius, 22.5));
        let start_distance = ((head.start.0 - 100.0).powi(2) + (head.start.1 - 0.0).powi(2)).sqrt();
        assert!(close(start_distance, 22.5));
        // The arc faces back along the approach: centered on 180°.
        assert!(head.start.0 < 100.0 && head.end.0 < 100.0);
        assert!(arc_head(&VizAnchor::rect(0.0, 0.0, 10.0, 10.0), 0.0, 2.5, 42.0).is_none());
    }
}

/// Flattens a circle into a closed polyline (the last point repeats the
/// first), for code paths that must avoid arc geometry.
pub fn flatten_circle(
    center_x: f32,
    center_y: f32,
    radius: f32,
    segments: usize,
) -> Vec<(f32, f32)> {
    let segments = segments.max(8);
    (0..=segments)
        .map(|index| {
            let angle = index as f32 / segments as f32 * 360.0;
            polar(center_x, center_y, radius, angle)
        })
        .collect()
}

/// Flattens a rounded rectangle outline into a closed polyline, with
/// `corner_segments` line segments per corner arc.
pub fn flatten_rounded_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    corner_radius: f32,
    corner_segments: usize,
) -> Vec<(f32, f32)> {
    let radius = corner_radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let corner_segments = corner_segments.max(1);
    let mut points = Vec::new();
    // Corner centers and the start angle of each 90° arc, walked clockwise
    // from the top edge (y grows downward, angles per `polar`).
    let corners = [
        (x + width - radius, y + radius, 270.0),
        (x + width - radius, y + height - radius, 0.0),
        (x + radius, y + height - radius, 90.0),
        (x + radius, y + radius, 180.0),
    ];
    for (corner_x, corner_y, start_angle) in corners {
        for index in 0..=corner_segments {
            let angle = start_angle + index as f32 / corner_segments as f32 * 90.0;
            points.push(polar(corner_x, corner_y, radius, angle));
        }
    }
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}

/// Splits a polyline into dash runs, each a polyline to stroke solid.
///
/// This exists because dashing via the tessellator's own dash support panics
/// on closed curved paths (lyon's normalized sampler emits NaN when a path's
/// end coincides with its start), so curved outlines are flattened and dashed
/// explicitly. Every emitted coordinate is a finite interpolation of inputs.
pub fn dash_polyline(points: &[(f32, f32)], dash: f32, gap: f32) -> Vec<Vec<(f32, f32)>> {
    if points.len() < 2 {
        return Vec::new();
    }
    if dash <= 0.0 || gap <= 0.0 {
        return vec![points.to_vec()];
    }
    let mut runs = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut drawing = true;
    let mut remaining = dash;
    for window in points.windows(2) {
        let (x0, y0) = window[0];
        let (x1, y1) = window[1];
        let segment_length = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        if segment_length <= f32::EPSILON {
            continue;
        }
        let mut travelled = 0.0;
        while travelled < segment_length {
            let step = remaining.min(segment_length - travelled);
            let start_fraction = travelled / segment_length;
            let end_fraction = (travelled + step) / segment_length;
            if drawing {
                let start = (
                    x0 + (x1 - x0) * start_fraction,
                    y0 + (y1 - y0) * start_fraction,
                );
                let end = (x0 + (x1 - x0) * end_fraction, y0 + (y1 - y0) * end_fraction);
                if current.is_empty() {
                    current.push(start);
                }
                current.push(end);
            }
            travelled += step;
            remaining -= step;
            if remaining <= f32::EPSILON {
                if drawing && !current.is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
                drawing = !drawing;
                remaining = if drawing { dash } else { gap };
            }
        }
    }
    if drawing && !current.is_empty() {
        runs.push(current);
    }
    runs
}

#[cfg(test)]
mod dash_tests {
    use super::*;

    #[test]
    fn dashed_circles_stay_finite_and_cover_the_circumference() {
        let circumference = 2.0 * std::f32::consts::PI * 24.0;
        let (dash, gap) = even_dash(circumference, 3.0, 3.0);
        let points = flatten_circle(100.0, 100.0, 24.0, 64);
        let runs = dash_polyline(&points, dash, gap);
        assert!(!runs.is_empty());
        let mut drawn = 0.0;
        for run in &runs {
            assert!(run.len() >= 2);
            for (x, y) in run {
                assert!(x.is_finite() && y.is_finite());
            }
            for window in run.windows(2) {
                let (x0, y0) = window[0];
                let (x1, y1) = window[1];
                drawn += ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
            }
        }
        // Half the perimeter is dashes (dash == gap), drawn as chords of the
        // flattened polygon, so allow a small flattening deficit.
        assert!((drawn - circumference / 2.0).abs() < circumference * 0.05);
    }

    #[test]
    fn dashed_rounded_rects_stay_finite() {
        let points = flatten_rounded_rect(10.0, 10.0, 190.0, 90.0, 10.0, 6);
        for run in dash_polyline(&points, 3.0, 3.0) {
            for (x, y) in run {
                assert!(x.is_finite() && y.is_finite());
            }
        }
    }

    #[test]
    fn non_positive_dash_falls_back_to_a_solid_run() {
        let points = vec![(0.0, 0.0), (10.0, 0.0)];
        let runs = dash_polyline(&points, 0.0, 3.0);
        assert_eq!(runs, vec![points]);
    }
}
