use caditor_geometry::{Aabb, Aabb2, Point2, Point3, Vector3};

use super::{
    MeasureError,
    mass::{curve_breaks, surface_u_breaks, surface_v_breaks},
    quadrature,
};
use crate::{
    curve::Curve,
    interrupt,
    intersect::patch_bounds,
    interval::Interval,
    surface::Surface,
    topology::{FaceContainment, FaceId, Solid, SolidClassifier, axis_extremes},
};

const AXES: [Vector3; 3] = [Vector3::X, Vector3::Y, Vector3::Z];
const PIECE_SAMPLES: usize = 4;
const MIN_SEEDS: usize = 4;
const MAX_SEEDS: usize = 24;
const MAX_CANDIDATES: usize = 4;
const MAX_ITERATIONS: usize = 40;
const MAX_HALVINGS: usize = 30;
const STEP_EPSILON: f64 = 1e-14;

pub fn extent(solid: &Solid) -> Result<Option<Aabb>, MeasureError> {
    let mut points: Vec<Point3> = solid.vertices().map(|(_, vertex)| vertex.point()).collect();
    for (_, edge) in solid.edges() {
        interrupt::check()?;
        edge_extremes(edge.curve(), edge.interval(), &mut points);
    }
    let Some(outline) = Aabb::from_points(points.iter().copied()) else {
        return Ok(None);
    };
    let classifier = solid.classifier();
    for (id, face) in solid.faces() {
        interrupt::check()?;
        let surface = face.surface();
        for point in axis_extremes(surface) {
            let uv = surface.project(point, None);
            if inside(&classifier, id, uv) {
                points.push(surface.point_at(uv));
            }
        }
        if matches!(surface, Surface::Revolution(_) | Surface::BSpline(_)) {
            let Some(uv_box) = classifier.face_uv_box(id) else {
                continue;
            };
            if !within(&patch_bounds(surface, uv_box), &outline) {
                points.extend(interior_extremes(&classifier, id, surface, uv_box));
            }
        }
    }
    Ok(Aabb::from_points(points))
}

fn within(inner: &Aabb, outer: &Aabb) -> bool {
    inner.min().cmpge(outer.min()).all() && inner.max().cmple(outer.max()).all()
}

fn inside(classifier: &SolidClassifier<'_>, face: FaceId, uv: Point2) -> bool {
    matches!(
        classifier.point_in_face(face, uv),
        Some(FaceContainment::Inside | FaceContainment::OnBoundary)
    )
}

fn edge_extremes(curve: &Curve, range: Interval, points: &mut Vec<Point3>) {
    match curve {
        Curve::Line(_) => {}
        Curve::Circle(_) | Curve::Ellipse(_) => {
            let bounds = curve.bounding_box(range);
            points.extend([bounds.min(), bounds.max()]);
        }
        Curve::BSpline(_) | Curve::Intersection(_) => {
            let samples: Vec<f64> = quadrature::pieces(range, curve_breaks(curve, range))
                .into_iter()
                .flat_map(|piece| piece.split(PIECE_SAMPLES).collect::<Vec<_>>())
                .collect();
            for axis in AXES {
                let slope = |parameter: f64| curve.evaluate(parameter).first.dot(axis);
                for pair in samples.windows(2) {
                    let [start, end] = pair else {
                        continue;
                    };
                    let (low, high) = (slope(*start), slope(*end));
                    if low == 0.0 {
                        points.push(curve.point(*start));
                    } else if low * high < 0.0 {
                        let root = slope_root(curve, axis, *start, *end, low);
                        points.push(curve.point(root));
                    }
                }
            }
        }
    }
}

fn slope_root(curve: &Curve, axis: Vector3, start: f64, end: f64, start_slope: f64) -> f64 {
    let (mut low, mut high) = (start, end);
    let mut parameter = 0.5 * (low + high);
    for _ in 0..MAX_ITERATIONS {
        let derivatives = curve.evaluate(parameter);
        let slope = derivatives.first.dot(axis);
        if slope == 0.0 {
            return parameter;
        }
        if (slope > 0.0) == (start_slope > 0.0) {
            low = parameter;
        } else {
            high = parameter;
        }
        let bend = derivatives.second.dot(axis);
        let newton = parameter - slope / bend;
        let next = if bend != 0.0 && newton > low && newton < high {
            newton
        } else {
            0.5 * (low + high)
        };
        if (next - parameter).abs() <= STEP_EPSILON * (1.0 + parameter.abs()) {
            return next;
        }
        parameter = next;
    }
    parameter
}

fn seed_count(breaks: Vec<f64>, range: Interval) -> usize {
    (quadrature::pieces(range, breaks).len() * 3).clamp(MIN_SEEDS, MAX_SEEDS)
}

fn interior_extremes(
    classifier: &SolidClassifier<'_>,
    face: FaceId,
    surface: &Surface,
    uv_box: Aabb2,
) -> Vec<Point3> {
    let (min, max) = (uv_box.min(), uv_box.max());
    let (Some(u_range), Some(v_range)) = (Interval::new(min.x, max.x), Interval::new(min.y, max.y))
    else {
        return Vec::new();
    };
    let columns = seed_count(surface_u_breaks(surface, u_range), u_range);
    let rows = seed_count(surface_v_breaks(surface, v_range), v_range);
    let us: Vec<f64> = u_range.split(columns).collect();
    let vs: Vec<f64> = v_range.split(rows).collect();
    let grid: Vec<Vec<Point3>> = vs
        .iter()
        .map(|v| us.iter().map(|u| surface.point(*u, *v)).collect())
        .collect();
    let mut found = Vec::new();
    for axis in AXES {
        for direction in [axis, -axis] {
            let height = |row: usize, column: usize| {
                grid.get(row)
                    .and_then(|points| points.get(column))
                    .map(|point| point.dot(direction))
            };
            let mut peaks: Vec<(f64, usize, usize)> = Vec::new();
            for (row, points) in grid.iter().enumerate() {
                for (column, point) in points.iter().enumerate() {
                    let value = point.dot(direction);
                    let neighbours = [
                        (row.wrapping_sub(1), column),
                        (row + 1, column),
                        (row, column.wrapping_sub(1)),
                        (row, column + 1),
                    ];
                    let peak = neighbours.iter().all(|(other_row, other_column)| {
                        height(*other_row, *other_column).is_none_or(|other| other <= value)
                    });
                    if peak {
                        peaks.push((value, row, column));
                    }
                }
            }
            peaks.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, row, column) in peaks.into_iter().take(MAX_CANDIDATES) {
                let (Some(u), Some(v)) = (us.get(column), vs.get(row)) else {
                    continue;
                };
                let seed = Point2::new(*u, *v);
                for uv in [seed, highest_near(surface, direction, seed, uv_box)] {
                    if inside(classifier, face, uv) {
                        found.push(surface.point_at(uv));
                    }
                }
            }
        }
    }
    found
}

fn highest_near(surface: &Surface, direction: Vector3, seed: Point2, uv_box: Aabb2) -> Point2 {
    let height = |uv: Point2| surface.point_at(uv).dot(direction);
    let within_box = |uv: Point2| {
        uv.is_finite()
            && uv.x >= uv_box.min().x
            && uv.y >= uv_box.min().y
            && uv.x <= uv_box.max().x
            && uv.y <= uv_box.max().y
    };
    let mut uv = seed;
    let mut best = height(uv);
    for _ in 0..MAX_ITERATIONS {
        let at = surface.evaluate(uv.x, uv.y);
        let gradient = Point2::new(at.du.dot(direction), at.dv.dot(direction));
        let (a, b, c) = (
            at.duu.dot(direction),
            at.duv.dot(direction),
            at.dvv.dot(direction),
        );
        let determinant = a * c - b * b;
        if determinant == 0.0 || !determinant.is_finite() {
            break;
        }
        let step = Point2::new(
            (c * gradient.x - b * gradient.y) / determinant,
            (a * gradient.y - b * gradient.x) / determinant,
        );
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..MAX_HALVINGS {
            let candidate = uv - step * scale;
            if within_box(candidate) {
                let value = height(candidate);
                if value >= best {
                    accepted = Some((candidate, value));
                    break;
                }
            }
            scale *= 0.5;
        }
        let Some((next, value)) = accepted else {
            break;
        };
        let moved = (next - uv).length();
        uv = next;
        best = value;
        if moved <= STEP_EPSILON * (1.0 + uv.length()) {
            break;
        }
    }
    uv
}
