use std::{collections::BTreeMap, f64::consts::PI};

use caditor_document::{Document, FeatureId, FeatureKind, Transaction, TransactionBuilder};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{ArcGeometry, Constraint, Entity, EntityId, Sketch};

use crate::import::{Drawing, DrawingCurve, MAX_DRAWING_CURVES};

const RELATIVE_JOINT_TOLERANCE: f64 = 1e-6;
const MIN_JOINT_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub enum SketchTarget {
    Existing(FeatureId),
    New { name: String, plane: Plane },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingImport {
    pub transaction: Transaction,
    pub sketch: FeatureId,
    pub curves: usize,
    pub joints: usize,
}

pub fn drawing_transaction(
    document: &Document,
    drawing: &Drawing,
    target: SketchTarget,
    label: impl Into<String>,
) -> DrawingImport {
    let mut builder = document.transaction(label);
    let sketch = match target {
        SketchTarget::Existing(feature) => feature,
        SketchTarget::New { name, plane } => {
            builder.add_feature(name, FeatureKind::from(Sketch::new(plane)))
        }
    };
    let tolerance = (drawing.extent() * RELATIVE_JOINT_TOLERANCE).max(MIN_JOINT_TOLERANCE);
    let mut ends = Vec::new();
    let mut curves = 0;
    for (index, curve) in drawing.curves.iter().enumerate().take(MAX_DRAWING_CURVES) {
        let Some(curve) = usable(curve, tolerance) else {
            continue;
        };
        if !matches!(curve, DrawingCurve::Point(_)) {
            curves += 1;
        }
        let construction = drawing.construction.contains(&index);
        add_curve(&mut builder, sketch, &curve, construction, &mut ends);
    }
    let mut joints = 0;
    for cluster in clusters(&ends, tolerance) {
        let mut members = cluster.into_iter().filter_map(|index| ends.get(index));
        let Some((anchor, _)) = members.next() else {
            continue;
        };
        for (point, _) in members {
            builder.add_sketch_constraint(sketch, Constraint::Coincident(*anchor, *point));
            joints += 1;
        }
    }
    DrawingImport {
        transaction: builder.finish(),
        sketch,
        curves,
        joints,
    }
}

fn usable(curve: &DrawingCurve, tolerance: f64) -> Option<DrawingCurve> {
    match curve {
        DrawingCurve::Line { start, end } if start.distance(*end) <= tolerance => None,
        DrawingCurve::Circle { radius, .. } if *radius <= tolerance => None,
        DrawingCurve::Arc { center, start, end } if start.distance(*end) <= tolerance => {
            let arc = ArcGeometry::from_points(*center, *start, *end);
            (arc.radius > tolerance && arc.sweep > PI).then_some(DrawingCurve::Circle {
                center: *center,
                radius: arc.radius,
            })
        }
        _ => Some(curve.clone()),
    }
}

fn add_curve(
    builder: &mut TransactionBuilder<'_>,
    sketch: FeatureId,
    curve: &DrawingCurve,
    construction: bool,
    ends: &mut Vec<(EntityId, Point2)>,
) {
    let point = |builder: &mut TransactionBuilder<'_>, position: Point2| {
        builder.add_sketch_entity(sketch, Entity::Point(position))
    };
    match curve {
        DrawingCurve::Point(position) => {
            point(builder, *position);
        }
        DrawingCurve::Line { start, end } => {
            let (first, last) = (point(builder, *start), point(builder, *end));
            builder.add_sketch_entity_as(
                sketch,
                Entity::Line {
                    start: first,
                    end: last,
                },
                construction,
            );
            ends.extend([(first, *start), (last, *end)]);
        }
        DrawingCurve::Circle { center, radius } => {
            let center = point(builder, *center);
            builder.add_sketch_entity_as(
                sketch,
                Entity::Circle {
                    center,
                    radius: *radius,
                },
                construction,
            );
        }
        DrawingCurve::Arc { center, start, end } => {
            let center = point(builder, *center);
            let (first, last) = (point(builder, *start), point(builder, *end));
            builder.add_sketch_entity_as(
                sketch,
                Entity::Arc {
                    center,
                    start: first,
                    end: last,
                },
                construction,
            );
            ends.extend([(first, *start), (last, *end)]);
        }
        DrawingCurve::Spline { control_points } => {
            let ids: Vec<EntityId> = control_points
                .iter()
                .map(|position| point(builder, *position))
                .collect();
            if let (Some(first), Some(last), Some(start), Some(end)) = (
                ids.first(),
                ids.last(),
                control_points.first(),
                control_points.last(),
            ) {
                ends.extend([(*first, *start), (*last, *end)]);
            }
            builder.add_sketch_entity_as(
                sketch,
                Entity::Spline {
                    control_points: ids,
                },
                construction,
            );
        }
    }
}

fn clusters(ends: &[(EntityId, Point2)], tolerance: f64) -> Vec<Vec<usize>> {
    let cell = |position: Point2| {
        (
            (position.x / tolerance).floor() as i64,
            (position.y / tolerance).floor() as i64,
        )
    };
    let mut grid: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, (_, position)) in ends.iter().enumerate() {
        grid.entry(cell(*position)).or_default().push(index);
    }
    let mut parents: Vec<usize> = (0..ends.len()).collect();
    for (index, (_, position)) in ends.iter().enumerate() {
        let (x, y) = cell(*position);
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(neighbours) = grid.get(&(x + dx, y + dy)) else {
                    continue;
                };
                for other in neighbours {
                    let close = ends
                        .get(*other)
                        .is_some_and(|(_, near)| near.distance(*position) <= tolerance);
                    if *other > index && close {
                        union(&mut parents, index, *other);
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..ends.len() {
        let root = find(&mut parents, index);
        groups.entry(root).or_default().push(index);
    }
    groups
        .into_values()
        .filter(|members| members.len() > 1)
        .collect()
}

fn find(parents: &mut [usize], index: usize) -> usize {
    let mut root = index;
    while let Some(parent) = parents.get(root).copied().filter(|parent| *parent != root) {
        root = parent;
    }
    let mut current = index;
    while current != root {
        let Some(slot) = parents.get_mut(current) else {
            break;
        };
        current = std::mem::replace(slot, root);
    }
    root
}

fn union(parents: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parents, a), find(parents, b));
    if let Some(slot) = parents.get_mut(a.max(b)) {
        *slot = a.min(b);
    }
}
