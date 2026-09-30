use caditor_geometry::{Aabb, Point2, Point3};

use super::{Accuracy, EdgeShape, Element, FaceShape, MeasureError, Separation, Shape, box_gap};
use crate::{surface::Surface, tolerance::LINEAR_RESOLUTION};

const CURVE_SEEDS: usize = 48;
const SURFACE_SEEDS_PER_SIDE: usize = 16;
const REFINED_SEEDS: usize = 4;
const MAX_REFINEMENT_STEPS: usize = 64;
const SETTLED: f64 = 1e-3 * LINEAR_RESOLUTION;
const DEGENERATE_SQUARED_LENGTH: f64 = 1e-24;

pub fn distance(first: Element<'_>, second: Element<'_>) -> Result<Separation, MeasureError> {
    let first = Shape::of(first)?;
    let second = Shape::of(second)?;
    let closest = match (&first, &second) {
        (Shape::Point(from), Shape::Point(to)) => {
            let mut closest = Closest::default();
            closest.offer(*from, *to);
            closest
        }
        (Shape::Point(point), Shape::Edge(edge)) => point_edge(*point, edge),
        (Shape::Edge(edge), Shape::Point(point)) => point_edge(*point, edge).swapped(),
        (Shape::Point(point), Shape::Face(face)) => point_face(*point, face),
        (Shape::Face(face), Shape::Point(point)) => point_face(*point, face).swapped(),
        (Shape::Edge(first), Shape::Edge(second)) => edge_edge(first, second),
        (Shape::Edge(edge), Shape::Face(face)) => edge_face(edge, face),
        (Shape::Face(face), Shape::Edge(edge)) => edge_face(edge, face).swapped(),
        (Shape::Face(first), Shape::Face(second)) => face_face(first, second),
    };
    closest.finish()
}

#[derive(Debug, Clone, Copy, Default)]
struct Closest {
    best: Option<Separation>,
    accuracy: Accuracy,
}

impl Closest {
    fn with_accuracy(accuracy: Accuracy) -> Self {
        Self {
            best: None,
            accuracy,
        }
    }

    fn offer(&mut self, from: Point3, to: Point3) {
        let candidate = Separation::between(from, to, Accuracy::Exact);
        if !candidate.distance.is_finite() {
            return;
        }
        if self
            .best
            .is_none_or(|best| candidate.distance < best.distance)
        {
            self.best = Some(candidate);
        }
    }

    fn approximate(&mut self) {
        self.accuracy = Accuracy::Approximate;
    }

    fn merge(&mut self, other: Self) {
        self.accuracy = self.accuracy.and(other.accuracy);
        if let Some(candidate) = other.best {
            self.offer(candidate.from, candidate.to);
        }
    }

    fn bound(&self) -> f64 {
        self.best.map_or(f64::INFINITY, |best| best.distance)
    }

    fn reaches(&self, gap: f64) -> bool {
        gap <= self.bound() + LINEAR_RESOLUTION
    }

    fn swapped(self) -> Self {
        Self {
            best: self.best.map(Separation::swapped),
            ..self
        }
    }

    fn finish(self) -> Result<Separation, MeasureError> {
        self.best
            .map(|best| Separation {
                accuracy: self.accuracy,
                ..best
            })
            .ok_or(MeasureError::NoClosestPoints)
    }
}

fn point_edge(point: Point3, edge: &EdgeShape<'_>) -> Closest {
    let mut closest = Closest::with_accuracy(Accuracy::of(edge.has_exact_closest_points()));
    closest.offer(point, edge.point(edge.closest(point)));
    closest.offer(point, edge.start);
    closest.offer(point, edge.end);
    closest
}

fn point_face(point: Point3, face: &FaceShape<'_>) -> Closest {
    let mut closest = Closest::with_accuracy(Accuracy::of(face.projects_exactly()));
    let uv = face.surface.project(point, None);
    if face.contains(uv) {
        closest.offer(point, face.surface.point_at(uv));
        if face.projects_exactly() {
            return closest;
        }
    }
    let here = Aabb::from_point(point);
    for edge in &face.edges {
        if closest.reaches(box_gap(&here, &edge.bounds)) {
            closest.merge(point_edge(point, edge));
        }
    }
    closest
}

fn edge_edge(first: &EdgeShape<'_>, second: &EdgeShape<'_>) -> Closest {
    if let (Some(first), Some(second)) = (first.segment(), second.segment()) {
        let mut closest = Closest::default();
        let [from, to] = segment_segment(first, second);
        closest.offer(from, to);
        return closest;
    }
    let mut closest = Closest::default();
    for end in [first.start, first.end] {
        closest.merge(point_edge(end, second));
    }
    for end in [second.start, second.end] {
        closest.merge(point_edge(end, first).swapped());
    }
    closest.approximate();
    let first_seeds: Vec<(f64, Point3)> = curve_seeds(first);
    let second_seeds: Vec<(f64, Point3)> = curve_seeds(second);
    let mut pairs: Vec<(f64, f64, f64)> = first_seeds
        .iter()
        .flat_map(|(s, p)| {
            second_seeds
                .iter()
                .map(move |(t, q)| (p.distance_squared(*q), *s, *t))
        })
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, s, t) in pairs.into_iter().take(REFINED_SEEDS) {
        let [from, to] = refine_curves(first, second, s, t);
        closest.offer(from, to);
    }
    closest
}

fn curve_seeds(edge: &EdgeShape<'_>) -> Vec<(f64, Point3)> {
    edge.interval
        .split(CURVE_SEEDS)
        .map(|parameter| (parameter, edge.point(parameter)))
        .collect()
}

fn refine_curves(
    first: &EdgeShape<'_>,
    second: &EdgeShape<'_>,
    mut s: f64,
    mut t: f64,
) -> [Point3; 2] {
    let mut from = first.point(s);
    let mut to = second.point(t);
    for _ in 0..MAX_REFINEMENT_STEPS {
        t = second.closest(from);
        let next_to = second.point(t);
        s = first.closest(next_to);
        let next_from = first.point(s);
        let settled = next_from.distance(from) <= SETTLED && next_to.distance(to) <= SETTLED;
        from = next_from;
        to = next_to;
        if settled {
            break;
        }
    }
    [from, to]
}

fn segment_segment([p1, q1]: [Point3; 2], [p2, q2]: [Point3; 2]) -> [Point3; 2] {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);
    if a <= DEGENERATE_SQUARED_LENGTH && e <= DEGENERATE_SQUARED_LENGTH {
        return [p1, p2];
    }
    let (s, t) = if a <= DEGENERATE_SQUARED_LENGTH {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= DEGENERATE_SQUARED_LENGTH {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denominator = a * e - b * b;
            let s = if denominator > DEGENERATE_SQUARED_LENGTH * a * e {
                ((b * f - c * e) / denominator).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t = (b * s + f) / e;
            if t < 0.0 {
                ((-c / a).clamp(0.0, 1.0), 0.0)
            } else if t > 1.0 {
                (((b - c) / a).clamp(0.0, 1.0), 1.0)
            } else {
                (s, t)
            }
        }
    };
    [p1 + d1 * s, p2 + d2 * t]
}

fn edge_face(edge: &EdgeShape<'_>, face: &FaceShape<'_>) -> Closest {
    let mut closest = edge_across_face(edge, face);
    for boundary in &face.edges {
        if closest.reaches(box_gap(&edge.bounds, &boundary.bounds)) {
            closest.merge(edge_edge(edge, boundary));
        }
    }
    closest
}

fn edge_across_face(edge: &EdgeShape<'_>, face: &FaceShape<'_>) -> Closest {
    let mut closest = Closest::default();
    for end in [edge.start, edge.end] {
        closest.merge(point_face(end, face));
    }
    match (edge.segment(), face.surface) {
        (Some([start, end]), Surface::Plane(plane)) => {
            let frame = plane.frame();
            let above_start = (start - frame.origin()).dot(frame.normal());
            let above_end = (end - frame.origin()).dot(frame.normal());
            if above_start * above_end < 0.0 {
                let crossing = start + (end - start) * (above_start / (above_start - above_end));
                if face.contains(face.surface.project(crossing, None)) {
                    closest.offer(crossing, crossing);
                }
            }
        }
        _ => {
            closest.approximate();
            let mut seeds: Vec<(f64, f64, Point2)> = curve_seeds(edge)
                .into_iter()
                .map(|(parameter, point)| {
                    let uv = face.surface.project(point, None);
                    (
                        point.distance_squared(face.surface.point_at(uv)),
                        parameter,
                        uv,
                    )
                })
                .collect();
            seeds.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, parameter, uv) in seeds.into_iter().take(REFINED_SEEDS) {
                let (parameter, uv) = refine_curve_surface(edge, face, parameter, uv);
                let interior = parameter > edge.interval.start() && parameter < edge.interval.end();
                if interior && face.contains(uv) {
                    closest.offer(edge.point(parameter), face.surface.point_at(uv));
                }
            }
        }
    }
    closest
}

fn refine_curve_surface(
    edge: &EdgeShape<'_>,
    face: &FaceShape<'_>,
    mut parameter: f64,
    mut uv: Point2,
) -> (f64, Point2) {
    let mut from = edge.point(parameter);
    let mut to = face.surface.point_at(uv);
    for _ in 0..MAX_REFINEMENT_STEPS {
        uv = face.surface.project(from, Some(uv));
        let next_to = face.surface.point_at(uv);
        parameter = edge.closest(next_to);
        let next_from = edge.point(parameter);
        let settled = next_from.distance(from) <= SETTLED && next_to.distance(to) <= SETTLED;
        from = next_from;
        to = next_to;
        if settled {
            break;
        }
    }
    (parameter, uv)
}

fn face_face(first: &FaceShape<'_>, second: &FaceShape<'_>) -> Closest {
    let mut closest = Closest::default();
    for edge in &first.edges {
        closest.merge(edge_across_face(edge, second));
    }
    for edge in &second.edges {
        closest.merge(edge_across_face(edge, first).swapped());
    }
    for edge in &first.edges {
        for other in &second.edges {
            if closest.reaches(box_gap(&edge.bounds, &other.bounds)) {
                closest.merge(edge_edge(edge, other));
            }
        }
    }
    if first.is_flat() && second.is_flat() {
        return closest;
    }
    closest.approximate();
    let mut seeds: Vec<(f64, Point2, Point2)> = surface_seeds(first)
        .into_iter()
        .map(|uv| {
            let point = first.surface.point_at(uv);
            let other = second.surface.project(point, None);
            (
                point.distance_squared(second.surface.point_at(other)),
                uv,
                other,
            )
        })
        .collect();
    seeds.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, uv, other) in seeds.into_iter().take(REFINED_SEEDS) {
        let [uv, other] = refine_surfaces(first, second, uv, other);
        if first.contains(uv) && second.contains(other) {
            closest.offer(first.surface.point_at(uv), second.surface.point_at(other));
        }
    }
    closest
}

fn surface_seeds(face: &FaceShape<'_>) -> Vec<Point2> {
    let Some(uv_box) = face.uv_box() else {
        return Vec::new();
    };
    let (low, high) = (uv_box.min(), uv_box.max());
    let steps = SURFACE_SEEDS_PER_SIDE as f64;
    (0..SURFACE_SEEDS_PER_SIDE)
        .flat_map(|row| (0..SURFACE_SEEDS_PER_SIDE).map(move |column| (row, column)))
        .map(|(row, column)| {
            Point2::new(
                low.x + (high.x - low.x) * (column as f64 + 0.5) / steps,
                low.y + (high.y - low.y) * (row as f64 + 0.5) / steps,
            )
        })
        .filter(|uv| face.contains(*uv))
        .collect()
}

fn refine_surfaces(
    first: &FaceShape<'_>,
    second: &FaceShape<'_>,
    mut uv: Point2,
    mut other: Point2,
) -> [Point2; 2] {
    let mut from = first.surface.point_at(uv);
    let mut to = second.surface.point_at(other);
    for _ in 0..MAX_REFINEMENT_STEPS {
        uv = first.surface.project(to, Some(uv));
        let next_from = first.surface.point_at(uv);
        other = second.surface.project(next_from, Some(other));
        let next_to = second.surface.point_at(other);
        let settled = next_from.distance(from) <= SETTLED && next_to.distance(to) <= SETTLED;
        from = next_from;
        to = next_to;
        if settled {
            break;
        }
    }
    [uv, other]
}
