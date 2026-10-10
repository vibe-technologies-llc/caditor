use std::collections::BTreeSet;

use caditor_document::{Edit, FeatureId, Transaction, TransactionBuilder};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{
    Constraint, ConstraintId, Entity, EntityId, EntityState, FitSpacing, Reference, Relations,
    Sketch, SketchSolution, SplineKind,
};

use crate::{
    feature_tree::count,
    field::{self, DimensionTarget, sentence},
    model::Model,
    selection::{Pickable, Selection},
    units::Units,
    variants::all_variants,
};

const DISPLAY_DECIMALS: f64 = 3.0;
const SIGNIFICANT_DIGITS: f64 = 6.0;
const DEGENERATE_LENGTH: f64 = 1e-12;
const UNCHANGED_SCALE: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConstraintTool {
    Coincident,
    Midpoint,
    Concentric,
    Collinear,
    Fix,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Curvature,
    Equal,
    Symmetric,
    Distance,
    HorizontalDistance,
    VerticalDistance,
    Angle,
    Radius,
    Diameter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Point,
    Line,
    Circular,
    Spline,
    Elliptic,
}

type Item = (EntityId, Shape);

all_variants!(ConstraintTool: Coincident, Midpoint, Concentric, Collinear, Fix, Horizontal, Vertical, Parallel, Perpendicular, Tangent, Curvature, Equal, Symmetric, Distance, HorizontalDistance, VerticalDistance, Angle, Radius, Diameter);

impl ConstraintTool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Coincident => "Coincident",
            Self::Midpoint => "Midpoint",
            Self::Concentric => "Concentric",
            Self::Collinear => "Collinear",
            Self::Fix => "Fix",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
            Self::Curvature => "Curvature",
            Self::Equal => "Equal",
            Self::Symmetric => "Symmetric",
            Self::Distance => "Distance",
            Self::HorizontalDistance => "Horizontal distance",
            Self::VerticalDistance => "Vertical distance",
            Self::Angle => "Angle",
            Self::Radius => "Radius",
            Self::Diameter => "Diameter",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Coincident => "Join two points, or put a point on a curve",
            Self::Midpoint => "Put a point at the middle of a line, arc or elliptical arc",
            Self::Concentric => {
                "Give circles, arcs and ellipses one centre, or put a point at their centre"
            }
            Self::Collinear => "Put lines on one straight line",
            Self::Fix => "Lock points where they are; a curve is locked by its points",
            Self::Horizontal => {
                "Make lines or an ellipse's major axis horizontal, or line points up horizontally"
            }
            Self::Vertical => {
                "Make lines or an ellipse's major axis vertical, or line points up vertically"
            }
            Self::Parallel => "Make lines parallel",
            Self::Perpendicular => {
                "Make two lines meet at a right angle, or a line cross a circle or arc square, \
                 through its centre"
            }
            Self::Tangent => "Make a line and a curve, or two curves, touch smoothly",
            Self::Curvature => {
                "Make a spline run on from a line, arc or spline it shares an end with, tangent \
                 and bending alike, so the joint shows no kink in its curvature"
            }
            Self::Equal => {
                "Give lines and splines the same length, circles and arcs the same radius, or \
                 ellipses the same radii"
            }
            Self::Symmetric => "Mirror two points, lines, circles or arcs about a line or a point",
            Self::Distance => {
                "Fix the distance between two points, lines or circles, any two of them, a spline \
                 and a point, line or circle, the ends of a line, or the length of an arc"
            }
            Self::HorizontalDistance => {
                "Fix the horizontal distance between two points, the ends of a line, a point or \
                 circle and where a line crosses the level through it, or circles' centres"
            }
            Self::VerticalDistance => {
                "Fix the vertical distance between two points, the ends of a line, a point or \
                 circle and where a line crosses the upright through it, or circles' centres"
            }
            Self::Angle => {
                "Fix the angle between two lines or a line and an arc at their shared end, or \
                 how far an arc sweeps"
            }
            Self::Radius => "Fix the radius of circles and arcs, or both radii of an ellipse",
            Self::Diameter => {
                "Fix the diameter of circles and arcs, or of a point turned about a line, twice \
                 its distance from it, as a lathe drawing shows a revolved profile"
            }
        }
    }

    pub fn selection_hint(self) -> &'static str {
        match self {
            Self::Coincident => {
                "Select two or more points, or points and one line, circle, arc or spline"
            }
            Self::Midpoint => "Select a point and a line, arc or elliptical arc",
            Self::Concentric => {
                "Select two or more circles, arcs or ellipses, or a point and one of them"
            }
            Self::Collinear | Self::Parallel => "Select two or more lines",
            Self::Fix => "Select the points or curves to lock",
            Self::Horizontal | Self::Vertical => {
                "Select one or more lines or ellipses, or two or more points"
            }
            Self::Perpendicular => {
                "Select two or more lines, the others turning square to the first, or a line and \
                 a circle or arc"
            }
            Self::Angle => {
                "Select two lines, a line and an arc or elliptical arc sharing an end, or one arc"
            }
            Self::Tangent => {
                "Select a line, circle or arc, and one or more circles, arcs, splines or ellipses to \
                 touch it, or two curves"
            }
            Self::Curvature => "Select a spline and the line, arc or spline at one of its ends",
            Self::Equal => {
                "Select two or more lines or splines, two or more circles or arcs, or two or more \
                 ellipses"
            }
            Self::Symmetric => {
                "Select two points, lines, circles or arcs, and the line or point to mirror them \
                 about"
            }
            Self::Distance => {
                "Select one line or arc, two of points, lines and circles, or a spline or ellipse \
                 and a point, line, circle or ellipse"
            }
            Self::HorizontalDistance | Self::VerticalDistance => {
                "Select two points, one line, a point or circle and a line, or a point and a \
                 circle, or two circles or arcs"
            }
            Self::Radius => "Select one or more circles, arcs or ellipses",
            Self::Diameter => {
                "Select one or more circles or arcs, or a point and the line it turns about"
            }
        }
    }

    pub fn is_dimension(self) -> bool {
        matches!(
            self,
            Self::Distance
                | Self::HorizontalDistance
                | Self::VerticalDistance
                | Self::Angle
                | Self::Radius
                | Self::Diameter
        )
    }

    #[cfg(test)]
    pub fn candidates(
        self,
        definition: &Sketch,
        shown: &Sketch,
        selected: &[EntityId],
    ) -> Result<Vec<Constraint>, String> {
        self.candidates_among(definition, shown, selected, &definition.relations())
    }

    pub fn candidates_among(
        self,
        definition: &Sketch,
        shown: &Sketch,
        selected: &[EntityId],
        relations: &Relations,
    ) -> Result<Vec<Constraint>, String> {
        let items: Option<Vec<Item>> = selected
            .iter()
            .map(|id| shape_of(definition, *id).map(|shape| (*id, shape)))
            .collect();
        let constraints = items
            .and_then(|items| self.propose(definition, shown, &items))
            .ok_or_else(|| self.selection_hint().to_owned())?;
        for constraint in &constraints {
            definition
                .check_constraint(constraint)
                .map_err(|error| format!("{}.", sentence(&error.to_string())))?;
        }
        new_relations(definition, relations, constraints)
    }

    fn propose(
        self,
        definition: &Sketch,
        shown: &Sketch,
        items: &[Item],
    ) -> Option<Vec<Constraint>> {
        use Shape::{Circular, Elliptic, Line, Point};
        match (self, items) {
            (Self::Coincident, _) => chained(items, Point, Constraint::Coincident)
                .or_else(|| onto_one_curve(items, Constraint::Coincident)),
            (
                Self::Midpoint,
                &[(point, Point), (curve, Line | Circular | Elliptic)]
                | &[(curve, Line | Circular | Elliptic), (point, Point)],
            ) => Some(vec![Constraint::Midpoint { point, curve }]),
            (
                Self::Concentric,
                &[(point, Point), (curve, Circular | Elliptic)]
                | &[(curve, Circular | Elliptic), (point, Point)],
            ) => Some(vec![Constraint::Concentric(point, curve)]),
            (Self::Concentric, _) => centred(items),
            (Self::Collinear, _) => chained(items, Line, Constraint::Collinear),
            (Self::Fix, _) => fixed(definition, shown, items),
            (Self::Horizontal, &[(_, Point), ..]) => {
                chained(items, Point, Constraint::HorizontalPoints)
            }
            (Self::Vertical, &[(_, Point), ..]) => {
                chained(items, Point, Constraint::VerticalPoints)
            }
            (Self::Horizontal, _) => each(items, Constraint::Horizontal),
            (Self::Vertical, _) => each(items, Constraint::Vertical),
            (Self::Parallel, _) => chained(items, Line, Constraint::Parallel),
            (
                Self::Perpendicular,
                &[(line, Line), (curve, Circular)] | &[(curve, Circular), (line, Line)],
            ) => Some(vec![Constraint::Perpendicular(line, curve)]),
            (Self::Perpendicular, _) => chained(items, Line, Constraint::Perpendicular),
            (Self::Tangent, _) => touching(items),
            (Self::Curvature, &[(a, first), (b, second)])
                if (first == Shape::Spline || second == Shape::Spline)
                    && first != Shape::Point
                    && second != Shape::Point =>
            {
                Some(vec![Constraint::Tangent(a, b), Constraint::Curvature(a, b)])
            }
            (Self::Equal, _) => chained(items, Line, Constraint::Equal)
                .or_else(|| chained(items, Circular, Constraint::Equal))
                .or_else(|| chained(items, Elliptic, Constraint::Equal))
                .or_else(|| equal_lengths(items)),
            (Self::Symmetric, _) => symmetric(definition, shown, items),
            (Self::Distance, &[(line, Line)]) => {
                let Some(&Entity::Line { start, end }) = definition.entity(line) else {
                    return None;
                };
                let (from, to) = shown.line_endpoints(line)?;
                Some(vec![distance(start, end, from.distance(to))])
            }
            (Self::Distance, &[(point, Point), (spline, Shape::Spline)])
            | (Self::Distance, &[(spline, Shape::Spline), (point, Point)]) => {
                Some(vec![measured(shown, |value| Constraint::Distance {
                    from: point,
                    to: spline,
                    value,
                })?])
            }
            (Self::Distance, &[(a, Point), (b, Point)]) => {
                let length = shown.point(a)?.distance(shown.point(b)?);
                Some(vec![distance(a, b, length)])
            }
            (Self::Distance, &[(point, Point), (line, Line)])
            | (Self::Distance, &[(line, Line), (point, Point)]) => {
                let length = distance_to_line(shown.point(point)?, line_through(shown, line)?);
                Some(vec![distance(point, line, length)])
            }
            (
                Self::Distance,
                &[(from, Point), (to, Circular)]
                | &[(to, Circular), (from, Point)]
                | &[(from, Line | Circular), (to, Line | Circular)]
                | &[(from, Shape::Spline), (to, Line | Circular)]
                | &[(from, Line | Circular), (to, Shape::Spline)]
                | &[
                    (from, Point | Line | Circular | Shape::Spline | Elliptic),
                    (to, Elliptic),
                ]
                | &[
                    (from, Elliptic),
                    (to, Point | Line | Circular | Shape::Spline),
                ],
            ) => Some(vec![measured(shown, |value| Constraint::Distance {
                from,
                to,
                value,
            })?]),
            (Self::HorizontalDistance | Self::VerticalDistance, &[(line, Line)]) => {
                let Some(&Entity::Line { start, end }) = definition.entity(line) else {
                    return None;
                };
                Some(vec![self.offset(shown, start, end)?])
            }
            (Self::HorizontalDistance | Self::VerticalDistance, &[(a, Point), (b, Point)]) => {
                Some(vec![self.offset(shown, a, b)?])
            }
            (
                Self::HorizontalDistance | Self::VerticalDistance,
                &[(from, Point | Circular), (to, Line | Circular)]
                | &[(from, Line | Circular), (to, Point | Circular)],
            ) => {
                Some(vec![self.offset(shown, from, to).unwrap_or_else(|| {
                    self.offset_of(from, to, millimetres(0.0))
                })])
            }
            (Self::Angle, &[(a, Line), (b, Line)]) => Some(vec![angle(shown, a, b)?]),
            (Self::Angle, &[(line, Line), (arc, Circular)] | &[(arc, Circular), (line, Line)])
                if is_arc(definition, arc) =>
            {
                Some(vec![angle_to_arc(shown, line, arc)?])
            }
            (Self::Angle, &[(line, Line), (arc, Elliptic)] | &[(arc, Elliptic), (line, Line)]) => {
                Some(vec![angle_to_arc(shown, line, arc)?])
            }
            (Self::Distance, &[(arc, Circular)]) if is_arc(definition, arc) => {
                Some(vec![measured(shown, |value| Constraint::ArcLength {
                    arc,
                    value,
                })?])
            }
            (Self::Angle, &[(arc, Circular)]) if is_arc(definition, arc) => {
                let sweep = shown.measured(&Constraint::Sweep {
                    arc,
                    value: Expression::Number(0.0),
                })?;
                Some(vec![Constraint::Sweep {
                    arc,
                    value: Expression::Measure(rounded_for_display(sweep), Unit::Degree),
                }])
            }
            (Self::Radius, _) => radii(shown, items),
            (Self::Diameter, &[(point, Point), (axis, Line)] | &[(axis, Line), (point, Point)]) => {
                Some(vec![measured(shown, |value| Constraint::AxisDiameter {
                    point,
                    axis,
                    value,
                })?])
            }
            (Self::Diameter, _) => each_measured(shown, items, |entity, value| {
                Constraint::Diameter { entity, value }
            }),
            _ => None,
        }
    }

    fn offset(self, shown: &Sketch, from: EntityId, to: EntityId) -> Option<Constraint> {
        measured(shown, |value| self.offset_of(from, to, value))
    }

    fn offset_of(self, from: EntityId, to: EntityId, value: Expression) -> Constraint {
        match self {
            Self::VerticalDistance => Constraint::VerticalDistance { from, to, value },
            _ => Constraint::HorizontalDistance { from, to, value },
        }
    }
}

fn is_arc(sketch: &Sketch, entity: EntityId) -> bool {
    matches!(sketch.entity(entity), Some(Entity::Arc { .. }))
}

fn new_relations(
    definition: &Sketch,
    relations: &Relations,
    constraints: Vec<Constraint>,
) -> Result<Vec<Constraint>, String> {
    for constraint in &constraints {
        if let Some(existing) = relations.contradicting(constraint) {
            return Err(format!(
                "{} would contradict {}, which is already in the sketch. Delete that first.",
                definition.describe(constraint),
                definition.describe_constraint(existing)
            ));
        }
    }
    let (restated, fresh): (Vec<Constraint>, Vec<Constraint>) = constraints
        .into_iter()
        .partition(|constraint| relations.restating(definition, constraint).is_some());
    match (fresh.is_empty(), restated.first()) {
        (true, Some(first)) if restated.len() == 1 => Err(format!(
            "{} is already in the sketch.",
            definition.describe(first)
        )),
        (true, Some(_)) => Err("Every one of these is already in the sketch.".to_owned()),
        _ => Ok(fresh),
    }
}

fn shape_of(sketch: &Sketch, id: EntityId) -> Option<Shape> {
    match id.reference() {
        Some(Reference::Origin) => return Some(Shape::Point),
        Some(Reference::HorizontalAxis | Reference::VerticalAxis) => return Some(Shape::Line),
        None => {}
    }
    Some(match sketch.entity(id)? {
        Entity::Point(_) => Shape::Point,
        Entity::Line { .. } => Shape::Line,
        Entity::Circle { .. } | Entity::Arc { .. } => Shape::Circular,
        Entity::Spline { .. } => Shape::Spline,
        Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => Shape::Elliptic,
    })
}

fn each(items: &[Item], make: fn(EntityId) -> Constraint) -> Option<Vec<Constraint>> {
    items
        .iter()
        .map(|(entity, shape)| {
            matches!(shape, Shape::Line | Shape::Elliptic).then(|| make(*entity))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|constraints| !constraints.is_empty())
}

fn centred(items: &[Item]) -> Option<Vec<Constraint>> {
    let ((first, _), rest) = items.split_first()?;
    let round = items
        .iter()
        .all(|(_, shape)| matches!(shape, Shape::Circular | Shape::Elliptic));
    (round && !rest.is_empty()).then(|| {
        rest.iter()
            .map(|(other, _)| Constraint::Concentric(*first, *other))
            .collect()
    })
}

fn radii(shown: &Sketch, items: &[Item]) -> Option<Vec<Constraint>> {
    let mut constraints = Vec::new();
    for (entity, shape) in items {
        let entity = *entity;
        match shape {
            Shape::Circular => {
                constraints.push(measured(shown, |value| Constraint::Radius {
                    entity,
                    value,
                })?);
            }
            Shape::Elliptic => {
                constraints.push(measured(shown, |value| Constraint::MajorRadius {
                    ellipse: entity,
                    value,
                })?);
                constraints.push(measured(shown, |value| Constraint::MinorRadius {
                    ellipse: entity,
                    value,
                })?);
            }
            Shape::Spline if is_conic(shown, entity) => {
                let rho = shown.measured(&Constraint::Rho {
                    conic: entity,
                    value: Expression::Number(0.0),
                })?;
                constraints.push(Constraint::Rho {
                    conic: entity,
                    value: Expression::Number(rounded_for_display(rho)),
                });
            }
            Shape::Point | Shape::Line | Shape::Spline => return None,
        }
    }
    (!constraints.is_empty()).then_some(constraints)
}

pub fn is_conic(sketch: &Sketch, entity: EntityId) -> bool {
    matches!(
        sketch.entity(entity).and_then(Entity::spline_kind),
        Some(SplineKind::Conic { .. })
    )
}

fn onto_one_curve(
    items: &[Item],
    make: fn(EntityId, EntityId) -> Constraint,
) -> Option<Vec<Constraint>> {
    let (curves, points): (Vec<&Item>, Vec<&Item>) =
        items.iter().partition(|(_, shape)| *shape != Shape::Point);
    let [(curve, _)] = curves.as_slice() else {
        return None;
    };
    if let [(first, _), (second, _)] = items {
        return Some(vec![make(*first, *second)]);
    }
    (!points.is_empty()).then(|| {
        points
            .iter()
            .map(|(point, _)| make(*point, *curve))
            .collect()
    })
}

fn touching(items: &[Item]) -> Option<Vec<Constraint>> {
    if items.len() < 2 || items.iter().any(|(_, shape)| *shape == Shape::Point) {
        return None;
    }
    if let [(first, _), (second, _)] = items {
        return Some(vec![Constraint::Tangent(*first, *second)]);
    }
    let lines: Vec<EntityId> = items
        .iter()
        .filter(|(_, shape)| *shape == Shape::Line)
        .map(|(entity, _)| *entity)
        .collect();
    let base = match lines.as_slice() {
        [only] => *only,
        _ => items.first()?.0,
    };
    Some(
        items
            .iter()
            .filter(|(entity, _)| *entity != base)
            .map(|(other, _)| Constraint::Tangent(base, *other))
            .collect(),
    )
}

fn chained(
    items: &[Item],
    needed: Shape,
    make: fn(EntityId, EntityId) -> Constraint,
) -> Option<Vec<Constraint>> {
    let ((first, _), rest) = items.split_first()?;
    let all_needed = items.iter().all(|(_, shape)| *shape == needed);
    (all_needed && !rest.is_empty())
        .then(|| rest.iter().map(|(other, _)| make(*first, *other)).collect())
}

fn equal_lengths(items: &[Item]) -> Option<Vec<Constraint>> {
    let lengths = items
        .iter()
        .all(|(_, shape)| matches!(shape, Shape::Line | Shape::Spline));
    let splines = items.iter().any(|(_, shape)| *shape == Shape::Spline);
    let ((first, _), rest) = items.split_first()?;
    (lengths && splines && !rest.is_empty()).then(|| {
        rest.iter()
            .map(|(other, _)| Constraint::Equal(*first, *other))
            .collect()
    })
}

fn measured(shown: &Sketch, make: impl Fn(Expression) -> Constraint) -> Option<Constraint> {
    let value = shown.measured(&make(Expression::Number(0.0)))?;
    Some(make(millimetres(value)))
}

fn each_measured(
    shown: &Sketch,
    items: &[Item],
    make: fn(EntityId, Expression) -> Constraint,
) -> Option<Vec<Constraint>> {
    items
        .iter()
        .map(|(entity, shape)| {
            (*shape == Shape::Circular).then_some(())?;
            measured(shown, |value| make(*entity, value))
        })
        .collect::<Option<Vec<_>>>()
        .filter(|constraints| !constraints.is_empty())
}

fn fixed(definition: &Sketch, shown: &Sketch, items: &[Item]) -> Option<Vec<Constraint>> {
    let mut points: Vec<EntityId> = Vec::new();
    let mut seen: BTreeSet<EntityId> = BTreeSet::new();
    for (entity, shape) in items {
        let owned = match shape {
            Shape::Point => vec![*entity],
            Shape::Line | Shape::Circular | Shape::Spline | Shape::Elliptic => {
                definition.entity(*entity)?.points()
            }
        };
        for point in owned {
            if seen.insert(point) {
                points.push(point);
            }
        }
    }
    let constraints: Vec<Constraint> = points
        .into_iter()
        .map(|point| {
            Some(Constraint::Fix {
                point,
                at: shown.point(point)?,
            })
        })
        .collect::<Option<_>>()?;
    (!constraints.is_empty()).then_some(constraints)
}

#[derive(Debug, Clone, Copy)]
enum Mirror {
    Line(Point2, Vector2),
    Point(Point2),
}

impl Mirror {
    fn of(shown: &Sketch, (entity, shape): Item) -> Option<Self> {
        match shape {
            Shape::Point => Some(Self::Point(shown.point(entity)?)),
            Shape::Line => {
                let (origin, direction) = line_through(shown, entity)?;
                Some(Self::Line(origin, direction.try_normalize()?))
            }
            Shape::Circular | Shape::Spline | Shape::Elliptic => None,
        }
    }

    fn reflect(self, point: Point2) -> Point2 {
        match self {
            Self::Point(centre) => centre * 2.0 - point,
            Self::Line(origin, direction) => {
                let offset = point - origin;
                origin + direction * (2.0 * offset.dot(direction)) - offset
            }
        }
    }
}

fn symmetric(definition: &Sketch, shown: &Sketch, items: &[Item]) -> Option<Vec<Constraint>> {
    let [first, second, third] = *items else {
        return None;
    };
    let axes: Vec<Item> = items
        .iter()
        .copied()
        .filter(|(entity, _)| entity.is_reference())
        .collect();
    if let [axis] = axes.as_slice()
        && axis.1 == Shape::Line
    {
        let others: Vec<Item> = items.iter().copied().filter(|item| item != axis).collect();
        if let [a, b] = others.as_slice() {
            return mirrored(definition, shown, *a, *b, *axis).map(|(_, constraints)| constraints);
        }
    }
    [
        (first, second, third),
        (first, third, second),
        (second, third, first),
    ]
    .into_iter()
    .filter_map(|(a, b, about)| mirrored(definition, shown, a, b, about))
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, constraints)| constraints)
}

fn mirrored(
    definition: &Sketch,
    shown: &Sketch,
    a: Item,
    b: Item,
    about: Item,
) -> Option<(f64, Vec<Constraint>)> {
    let mirror = Mirror::of(shown, about)?;
    let error = |pairs: &[(EntityId, EntityId)]| -> Option<f64> {
        pairs.iter().try_fold(0.0, |sum, (from, to)| {
            Some(
                sum + mirror
                    .reflect(shown.point(*from)?)
                    .distance(shown.point(*to)?),
            )
        })
    };
    let pairs = match (a.1, b.1) {
        (Shape::Point, Shape::Point) => vec![(a.0, b.0)],
        (Shape::Line, Shape::Line) => {
            let ends = |line: EntityId| match definition.entity(line) {
                Some(&Entity::Line { start, end }) => Some((start, end)),
                _ => None,
            };
            let ((a_start, a_end), (b_start, b_end)) = (ends(a.0)?, ends(b.0)?);
            let straight = vec![(a_start, b_start), (a_end, b_end)];
            let crossed = vec![(a_start, b_end), (a_end, b_start)];
            if error(&straight)? <= error(&crossed)? {
                straight
            } else {
                crossed
            }
        }
        (Shape::Circular, Shape::Circular) => {
            return mirrored_circular(definition, shown, a.0, b.0, about.0, &mirror);
        }
        _ => return None,
    };
    Some((error(&pairs)?, symmetric_pairs(&pairs, about.0)))
}

fn symmetric_pairs(pairs: &[(EntityId, EntityId)], about: EntityId) -> Vec<Constraint> {
    pairs
        .iter()
        .map(|(first, second)| {
            if first == second {
                Constraint::Coincident(*first, about)
            } else {
                Constraint::Symmetric {
                    first: *first,
                    second: *second,
                    about,
                }
            }
        })
        .collect()
}

fn mirrored_circular(
    definition: &Sketch,
    shown: &Sketch,
    a: EntityId,
    b: EntityId,
    about: EntityId,
    mirror: &Mirror,
) -> Option<(f64, Vec<Constraint>)> {
    let off = |from: EntityId, to: EntityId| -> Option<f64> {
        Some(
            mirror
                .reflect(shown.point(from)?)
                .distance(shown.point(to)?),
        )
    };
    match (definition.entity(a)?, definition.entity(b)?) {
        (
            &Entity::Circle {
                center: a_center,
                radius: a_radius,
            },
            &Entity::Circle {
                center: b_center,
                radius: b_radius,
            },
        ) if a_center != b_center => {
            let error = off(a_center, b_center)? + (a_radius - b_radius).abs();
            let mut constraints = symmetric_pairs(&[(a_center, b_center)], about);
            constraints.push(Constraint::Equal(a, b));
            Some((error, constraints))
        }
        (
            &Entity::Arc {
                start: a_start,
                end: a_end,
                ..
            },
            &Entity::Arc {
                start: b_start,
                end: b_end,
                ..
            },
        ) if a != b => {
            let crossed = [(a_start, b_end), (a_end, b_start)];
            let straight = [(a_start, b_start), (a_end, b_end)];
            let error = |pairs: &[(EntityId, EntityId)]| {
                pairs
                    .iter()
                    .try_fold(0.0, |sum, (from, to)| Some(sum + off(*from, *to)?))
            };
            let (crossed_error, straight_error) = (error(&crossed)?, error(&straight)?);
            let (pairs, paired_error) = if crossed_error <= straight_error {
                (crossed, crossed_error)
            } else {
                (straight, straight_error)
            };
            let (a_arc, b_arc) = (shown.arc(a)?, shown.arc(b)?);
            let error = paired_error
                + mirror.reflect(a_arc.center).distance(b_arc.center)
                + (a_arc.radius - b_arc.radius).abs();
            let mut constraints = symmetric_pairs(&pairs, about);
            constraints.push(Constraint::Equal(a, b));
            Some((error, constraints))
        }
        _ => None,
    }
}

fn distance(from: EntityId, to: EntityId, length: f64) -> Constraint {
    Constraint::Distance {
        from,
        to,
        value: millimetres(length),
    }
}

fn line_through(sketch: &Sketch, line: EntityId) -> Option<(Point2, Vector2)> {
    let direction = sketch.line_direction(line)?;
    let origin = match line.reference() {
        Some(_) => Point2::ZERO,
        None => sketch.line_endpoints(line)?.0,
    };
    Some((origin, direction))
}

fn distance_to_line(point: Point2, (origin, direction): (Point2, Vector2)) -> f64 {
    let length = direction.length();
    if length < DEGENERATE_LENGTH {
        return point.distance(origin);
    }
    direction.perp_dot(point - origin).abs() / length
}

fn angle(shown: &Sketch, a: EntityId, b: EntityId) -> Option<Constraint> {
    let (first, second) = (shown.line_direction(a)?, shown.line_direction(b)?);
    let reversed = opens_backwards(shown, (a, first), (b, second));
    let from_ray = if reversed { -first } else { first };
    let signed = from_ray
        .perp_dot(second)
        .atan2(from_ray.dot(second))
        .to_degrees();
    let (from, to) = if signed < 0.0 { (b, a) } else { (a, b) };
    Some(Constraint::Angle {
        from,
        to,
        reversed,
        value: Expression::Measure(rounded_for_display(signed.abs()), Unit::Degree),
    })
}

fn angle_to_arc(shown: &Sketch, line: EntityId, arc: EntityId) -> Option<Constraint> {
    let vertex = shown.angle_vertex(arc, line)?;
    let direction = shown.line_direction(line)?;
    let (start, end) = shown.line_endpoints(line)?;
    let reversed = ((start + end) / 2.0 - vertex).dot(direction) < 0.0;
    let line_ray = if reversed { -direction } else { direction };
    let arc_ray = shown.angle_direction(arc, line)?;
    let signed = line_ray
        .perp_dot(arc_ray)
        .atan2(line_ray.dot(arc_ray))
        .to_degrees();
    let (from, to) = if signed < 0.0 {
        (arc, line)
    } else {
        (line, arc)
    };
    Some(Constraint::Angle {
        from,
        to,
        reversed,
        value: Expression::Measure(rounded_for_display(signed.abs()), Unit::Degree),
    })
}

fn opens_backwards(
    shown: &Sketch,
    (a, first): (EntityId, Vector2),
    (b, second): (EntityId, Vector2),
) -> bool {
    let (Some((a_origin, _)), Some((b_origin, _))) =
        (line_through(shown, a), line_through(shown, b))
    else {
        return false;
    };
    let sine = first.perp_dot(second);
    if sine.abs() <= DEGENERATE_LENGTH * first.length() * second.length() {
        return false;
    }
    let vertex = a_origin + first * ((b_origin - a_origin).perp_dot(second) / sine);
    let side = |line: EntityId, direction: Vector2| {
        let middle = shown
            .line_endpoints(line)
            .map(|(start, end)| (start + end) / 2.0);
        match middle {
            Some(middle) if (middle - vertex).dot(direction) < 0.0 => -1.0,
            Some(_) | None => 1.0,
        }
    };
    side(a, first) * side(b, second) < 0.0
}

pub fn in_unit(constraints: Vec<Constraint>, unit: impl Into<Units>) -> Vec<Constraint> {
    let unit = unit.into();
    let converted = |value: Expression| match value {
        Expression::Measure(length, Unit::Millimetre) => unit.measured(length),
        other => other,
    };
    let turned = |value: Expression| match value {
        Expression::Measure(degrees, Unit::Degree) => unit.angle.measured(degrees),
        other => other,
    };
    constraints
        .into_iter()
        .map(|constraint| match constraint {
            Constraint::Distance { from, to, value } => Constraint::Distance {
                from,
                to,
                value: converted(value),
            },
            Constraint::HorizontalDistance { from, to, value } => Constraint::HorizontalDistance {
                from,
                to,
                value: converted(value),
            },
            Constraint::VerticalDistance { from, to, value } => Constraint::VerticalDistance {
                from,
                to,
                value: converted(value),
            },
            Constraint::Radius { entity, value } => Constraint::Radius {
                entity,
                value: converted(value),
            },
            Constraint::Diameter { entity, value } => Constraint::Diameter {
                entity,
                value: converted(value),
            },
            Constraint::AxisDiameter { point, axis, value } => Constraint::AxisDiameter {
                point,
                axis,
                value: converted(value),
            },
            Constraint::ArcLength { arc, value } => Constraint::ArcLength {
                arc,
                value: converted(value),
            },
            Constraint::MajorRadius { ellipse, value } => Constraint::MajorRadius {
                ellipse,
                value: converted(value),
            },
            Constraint::MinorRadius { ellipse, value } => Constraint::MinorRadius {
                ellipse,
                value: converted(value),
            },
            Constraint::Sweep { arc, value } => Constraint::Sweep {
                arc,
                value: turned(value),
            },
            Constraint::Angle {
                from,
                to,
                reversed,
                value,
            } => Constraint::Angle {
                from,
                to,
                reversed,
                value: turned(value),
            },
            other => other,
        })
        .collect()
}

fn millimetres(length: f64) -> Expression {
    Expression::Measure(rounded_for_display(length), Unit::Millimetre)
}

pub fn rounded_for_display(value: f64) -> f64 {
    rounded_to_decimals(value, DISPLAY_DECIMALS)
}

pub fn rounded_to_decimals(value: f64, least_decimals: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let magnitude = value.abs().log10().floor();
    let decimals = if magnitude >= 0.0 {
        least_decimals
    } else {
        least_decimals.max(SIGNIFICANT_DIGITS - 1.0 - magnitude)
    };
    let scale = 10f64.powf(decimals);
    let rounded = (value * scale).round() / scale;
    if rounded.is_finite() { rounded } else { value }
}

pub fn selected_entities(selection: &Selection, feature: FeatureId) -> Vec<EntityId> {
    selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchEntity {
                feature: owner,
                entity,
            } if owner == feature => Some(entity),
            Pickable::SketchEntity { .. }
            | Pickable::SketchConstraint { .. }
            | Pickable::Origin
            | Pickable::Axis(_)
            | Pickable::Plane(_)
            | Pickable::Face { .. }
            | Pickable::Edge { .. }
            | Pickable::Vertex { .. }
            | Pickable::SketchRegion { .. }
            | Pickable::Region { .. }
            | Pickable::BlendEdge { .. }
            | Pickable::ShellFace { .. }
            | Pickable::CentreOfMass(_)
            | Pickable::FrameAxis { .. }
            | Pickable::FramePlane { .. }
            | Pickable::FeatureValue { .. }
            | Pickable::Datum(_) => None,
        })
        .collect()
}

pub fn selected_constraints(selection: &Selection, feature: FeatureId) -> Vec<ConstraintId> {
    selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchConstraint {
                feature: owner,
                constraint,
            } if owner == feature => Some(constraint),
            Pickable::SketchConstraint { .. }
            | Pickable::SketchEntity { .. }
            | Pickable::Origin
            | Pickable::Axis(_)
            | Pickable::Plane(_)
            | Pickable::Face { .. }
            | Pickable::Edge { .. }
            | Pickable::Vertex { .. }
            | Pickable::SketchRegion { .. }
            | Pickable::Region { .. }
            | Pickable::BlendEdge { .. }
            | Pickable::ShellFace { .. }
            | Pickable::CentreOfMass(_)
            | Pickable::FrameAxis { .. }
            | Pickable::FramePlane { .. }
            | Pickable::FeatureValue { .. }
            | Pickable::Datum(_) => None,
        })
        .collect()
}

pub struct Added {
    pub transaction: Transaction,
    pub constraints: Vec<ConstraintId>,
    pub references: Vec<ConstraintId>,
}

pub fn add_constraints(
    model: &Model,
    feature: FeatureId,
    tool: ConstraintTool,
    constraints: Vec<Constraint>,
) -> Added {
    let solution = model
        .settled_solution(feature)
        .filter(|_| tool.is_dimension());
    let determined: Vec<bool> = constraints
        .iter()
        .map(|constraint| solution.is_some_and(|solution| determines(solution, constraint)))
        .collect();
    let label = if determined.iter().all(|determined| *determined) && !determined.is_empty() {
        format!("Add reference {}", tool.label())
    } else {
        format!("Add {}", tool.label())
    };
    let mut transaction = settled_transaction(model, feature, label);
    let constraints: Vec<ConstraintId> = constraints
        .into_iter()
        .map(|constraint| transaction.add_sketch_constraint(feature, constraint))
        .collect();
    let references: Vec<ConstraintId> = constraints
        .iter()
        .zip(&determined)
        .filter(|(_, determined)| **determined)
        .map(|(id, _)| *id)
        .collect();
    for id in &references {
        transaction.set_sketch_constraint_active(feature, *id, false);
    }
    Added {
        transaction: transaction.finish(),
        constraints,
        references,
    }
}

fn determines(solution: &SketchSolution, constraint: &Constraint) -> bool {
    let shapes_without_freedom = matches!(constraint, Constraint::Rho { .. });
    constraint.dimension().is_some()
        && !shapes_without_freedom
        && constraint
            .entities()
            .into_iter()
            .all(|entity| solution.entity_state(entity) == Some(EntityState::FullyConstrained))
}

pub fn remove_items(
    model: &Model,
    feature: FeatureId,
    label: String,
    entities: Vec<EntityId>,
    constraints: Vec<ConstraintId>,
) -> Transaction {
    let mut transaction = settled_transaction(model, feature, label);
    transaction.remove_sketch_items(feature, entities, constraints);
    transaction.finish()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityChange {
    pub constraints: Vec<ConstraintId>,
    pub active: bool,
}

impl ActivityChange {
    pub fn of(sketch: &Sketch, selected: &[ConstraintId]) -> Option<Self> {
        let existing: Vec<ConstraintId> = selected
            .iter()
            .copied()
            .filter(|id| sketch.constraint(*id).is_some())
            .collect();
        let active = existing.iter().all(|id| !sketch.is_active(*id));
        let changed: Vec<ConstraintId> = existing
            .into_iter()
            .filter(|id| sketch.is_active(*id) != active)
            .collect();
        (!changed.is_empty()).then_some(Self {
            constraints: changed,
            active,
        })
    }

    pub fn verb(&self) -> &'static str {
        if self.active { "Enable" } else { "Disable" }
    }

    pub fn label(&self, sketch: &Sketch) -> String {
        let subject = match self.constraints.as_slice() {
            [only] => sketch.describe_constraint(*only),
            constraints => count(constraints.len(), "constraint", "constraints"),
        };
        format!("{} {subject}", self.verb())
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId, sketch: &Sketch) -> Transaction {
        let mut transaction = settled_transaction(model, feature, self.label(sketch));
        for id in &self.constraints {
            transaction.set_sketch_constraint_active(feature, *id, self.active);
        }
        transaction.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructionChange {
    pub curves: Vec<EntityId>,
    pub construction: bool,
}

impl ConstructionChange {
    pub fn of(sketch: &Sketch, selected: &[EntityId]) -> Option<Self> {
        let curves: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|id| {
                sketch
                    .entity(*id)
                    .is_some_and(|entity| !matches!(entity, Entity::Point(_)))
            })
            .collect();
        let construction = curves.iter().any(|curve| !sketch.is_construction(*curve));
        let changed: Vec<EntityId> = curves
            .into_iter()
            .filter(|curve| sketch.is_construction(*curve) != construction)
            .collect();
        (!changed.is_empty()).then_some(Self {
            curves: changed,
            construction,
        })
    }

    pub fn label(&self, sketch: &Sketch) -> String {
        let subject = match self.curves.as_slice() {
            [only] => sketch.entity_label(*only),
            curves => count(curves.len(), "curve", "curves"),
        };
        let kind = if self.construction {
            "construction"
        } else {
            "ordinary"
        };
        format!("Make {subject} {kind} geometry")
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId, sketch: &Sketch) -> Transaction {
        let mut transaction = settled_transaction(model, feature, self.label(sketch));
        for curve in &self.curves {
            transaction.edit(Edit::SetSketchConstruction {
                feature,
                id: *curve,
                construction: self.construction,
            });
        }
        transaction.finish()
    }
}

pub fn dimension_change(
    model: &Model,
    target: DimensionTarget,
    text: &str,
    first_dimension_scales: bool,
) -> Result<Transaction, String> {
    let plain = field::dimension_transaction(
        model.document(),
        model.parameters(),
        target,
        text,
        model.units(),
    )?;
    if !first_dimension_scales {
        return Ok(plain);
    }
    Ok(scaled_to_first_dimension(model, target, &plain).unwrap_or(plain))
}

fn scaled_to_first_dimension(
    model: &Model,
    target: DimensionTarget,
    plain: &Transaction,
) -> Option<Transaction> {
    let document = model.document();
    let wanted = plain.edits().iter().find_map(|edit| match edit {
        Edit::SetDimension { value, .. } => model.parameters().evaluate_expression(value).ok(),
        _ => None,
    })?;
    let owner = document.feature(target.feature)?;
    let sketch = owner.kind.sketch()?;
    let constraint = sketch.constraint(target.constraint)?;
    let only_dimension = sketch.constraints().all(|(id, other)| {
        id == target.constraint
            || !sketch.is_active(id)
            || (other.dimension().is_none() && !matches!(other, Constraint::Fix { .. }))
    });
    if constraint.dimension_kind() != Some(Dimension::LENGTH)
        || !sketch.is_active(target.constraint)
        || !only_dimension
        || sketch.projected().next().is_some()
    {
        return None;
    }
    let settled = model.settled_sketch(target.feature)?;
    let factor = wanted.value / settled.measured(constraint)?;
    if !factor.is_finite() || factor <= 0.0 || (factor - 1.0).abs() < UNCHANGED_SCALE {
        return None;
    }
    let mut scaled = settled.clone();
    for (id, entity) in settled.entities() {
        let resized = match entity {
            _ if id.is_reference() => continue,
            Entity::Point(position) => Entity::Point(*position * factor),
            Entity::Circle { center, radius } => Entity::Circle {
                center: *center,
                radius: radius * factor,
            },
            Entity::Ellipse {
                center,
                major,
                minor_radius,
            } => Entity::Ellipse {
                center: *center,
                major: *major,
                minor_radius: minor_radius * factor,
            },
            Entity::EllipticalArc {
                center,
                major,
                minor_radius,
                start,
                end,
            } => Entity::EllipticalArc {
                center: *center,
                major: *major,
                minor_radius: minor_radius * factor,
                start: *start,
                end: *end,
            },
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Spline { .. } => continue,
        };
        scaled.replace_entity(id, resized).ok()?;
    }
    let mut transaction =
        document.transaction(format!("Scale {} to its first dimension", owner.name));
    transaction.settle_sketch(target.feature, &scaled);
    for edit in plain.edits() {
        transaction.edit(edit.clone());
    }
    field::checked(document, transaction.finish()).ok()
}

pub fn settled_transaction(
    model: &Model,
    feature: FeatureId,
    label: String,
) -> TransactionBuilder<'_> {
    let mut transaction = model.document().transaction(label);
    if let Some(settled) = model.settled_sketch(feature) {
        transaction.settle_sketch(feature, settled);
    }
    transaction
}

#[cfg(test)]
mod tests {
    use caditor_expression::Quantity;
    use caditor_geometry::Plane;

    use super::*;
    use crate::units::LengthUnit;

    struct Fixture {
        sketch: Sketch,
        horizontal: EntityId,
        slanted: EntityId,
        slanted_start: EntityId,
        slanted_end: EntityId,
        circle: EntityId,
        arc: EntityId,
        spline: EntityId,
        lone: EntityId,
    }

    fn fixture() -> Fixture {
        let mut sketch = Sketch::new(Plane::XY);
        let horizontal = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(20.0, 5.0));
        let slanted = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
        let Some(&Entity::Line { start, end }) = sketch.entity(slanted) else {
            panic!("expected a line");
        };
        let circle = sketch.add_circle(Point2::new(30.0, 0.0), 4.25);
        let arc = sketch.add_arc(Point2::ZERO, Point2::new(3.0, 0.0), Point2::new(0.0, 3.0));
        let spline = sketch.add_spline(&[Point2::ZERO, Point2::new(1.0, 2.0)]);
        let lone = sketch.add_point(Point2::new(3.0, 4.0));
        Fixture {
            sketch,
            horizontal,
            slanted,
            slanted_start: start,
            slanted_end: end,
            circle,
            arc,
            spline,
            lone,
        }
    }

    fn candidates(
        fixture: &Fixture,
        tool: ConstraintTool,
        selected: &[EntityId],
    ) -> Result<Vec<Constraint>, String> {
        tool.candidates(&fixture.sketch, &fixture.sketch, selected)
    }

    fn measure(value: f64, unit: Unit) -> Expression {
        Expression::Measure(value, unit)
    }

    #[test]
    fn several_lines_each_get_the_constraint() {
        let f = fixture();
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[f.horizontal, f.slanted]),
            Ok(vec![
                Constraint::Horizontal(f.horizontal),
                Constraint::Horizontal(f.slanted)
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Vertical, &[f.slanted, f.lone]),
            Err("Select one or more lines or ellipses, or two or more points".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[]),
            Err("Select one or more lines or ellipses, or two or more points".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[EntityId::HORIZONTAL_AXIS]),
            Err("It only uses reference or projected geometry, which never moves.".to_owned())
        );
    }

    #[test]
    fn a_constraint_the_sketch_already_has_is_refused_and_a_batch_skips_it() {
        let mut f = fixture();
        f.sketch
            .add_constraint(Constraint::Horizontal(f.horizontal))
            .unwrap();
        f.sketch
            .add_constraint(Constraint::Parallel(f.horizontal, f.slanted))
            .unwrap();

        let level = f.sketch.entity_label(f.horizontal);
        let slanted = f.sketch.entity_label(f.slanted);

        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[f.horizontal]),
            Err(format!("Horizontal {level} is already in the sketch."))
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[f.horizontal, f.slanted]),
            Ok(vec![Constraint::Horizontal(f.slanted)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Parallel, &[f.slanted, f.horizontal]),
            Err(format!(
                "Parallel {slanted} and {level} is already in the sketch."
            ))
        );
        assert!(candidates(&f, ConstraintTool::Equal, &[f.slanted, f.horizontal]).is_ok());
    }

    #[test]
    fn a_constraint_that_contradicts_one_in_the_sketch_is_refused() {
        let mut f = fixture();
        f.sketch
            .add_constraint(Constraint::Horizontal(f.horizontal))
            .unwrap();

        let level = f.sketch.entity_label(f.horizontal);

        let refused = candidates(&f, ConstraintTool::Vertical, &[f.horizontal]);

        assert_eq!(
            refused,
            Err(format!(
                "Vertical {level} would contradict Horizontal {level}, which is already in the \
                 sketch. Delete that first."
            ))
        );
    }

    #[test]
    fn pairs_need_the_right_kinds_of_entity() {
        let f = fixture();
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Parallel,
                &[f.slanted, EntityId::VERTICAL_AXIS]
            ),
            Ok(vec![Constraint::Parallel(
                f.slanted,
                EntityId::VERTICAL_AXIS
            )])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Parallel, &[f.slanted_start]),
            Err("Select two or more lines".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Coincident, &[f.lone, f.circle]),
            Ok(vec![Constraint::Coincident(f.lone, f.circle)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Coincident, &[f.slanted, f.slanted_end]),
            Err("Point 4 is part of Line 5.".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Coincident, &[f.horizontal, f.slanted]),
            Err(
                "Select two or more points, or points and one line, circle, arc or spline"
                    .to_owned()
            )
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Tangent, &[f.slanted, f.circle]),
            Ok(vec![Constraint::Tangent(f.slanted, f.circle)])
        );
        assert!(candidates(&f, ConstraintTool::Tangent, &[f.slanted, f.horizontal]).is_err());
        assert_eq!(
            candidates(&f, ConstraintTool::Equal, &[f.arc, f.circle]),
            Ok(vec![Constraint::Equal(f.arc, f.circle)])
        );
        assert!(candidates(&f, ConstraintTool::Equal, &[f.slanted, f.circle]).is_err());
        assert_eq!(
            candidates(&f, ConstraintTool::Coincident, &[f.lone, f.spline]),
            Ok(vec![Constraint::Coincident(f.lone, f.spline)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Tangent, &[f.spline, f.slanted]),
            Ok(vec![Constraint::Tangent(f.spline, f.slanted)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Tangent, &[f.slanted, f.horizontal]),
            Err("Tangent does not apply to Line 5 and Line 2.".to_owned())
        );
    }

    #[test]
    fn ellipses_take_tangents_and_distances_with_splines_and_other_ellipses() {
        let mut f = fixture();
        let ellipse = f
            .sketch
            .add_ellipse(Point2::new(0.0, 20.0), Point2::new(6.0, 20.0), 2.0);
        let other = f
            .sketch
            .add_ellipse(Point2::new(20.0, 20.0), Point2::new(20.0, 24.0), 1.0);

        for pair in [[ellipse, other], [f.spline, ellipse], [ellipse, f.spline]] {
            assert_eq!(
                candidates(&f, ConstraintTool::Tangent, &pair),
                Ok(vec![Constraint::Tangent(pair[0], pair[1])])
            );
            let Ok(found) = candidates(&f, ConstraintTool::Distance, &pair) else {
                panic!("{pair:?} take a distance");
            };
            assert!(
                matches!(
                    found.as_slice(),
                    [Constraint::Distance { from, to, .. }] if [*from, *to] == pair
                ),
                "{found:?}"
            );
        }

        let arc = f.sketch.add_elliptical_arc(
            Point2::new(40.0, 0.0),
            Point2::new(50.0, 0.0),
            4.0,
            Point2::new(50.0, 0.0),
            Point2::new(40.0, 4.0),
        );
        let leaving = f
            .sketch
            .add_line(Point2::new(40.0, 4.0), Point2::new(40.0, 10.0));
        let (Some(&Entity::EllipticalArc { end, .. }), Some(&Entity::Line { start, .. })) =
            (f.sketch.entity(arc), f.sketch.entity(leaving))
        else {
            panic!("expected an elliptical arc and a line");
        };
        f.sketch
            .add_constraint(Constraint::Coincident(start, end))
            .unwrap();
        let Ok(found) = candidates(&f, ConstraintTool::Angle, &[leaving, arc]) else {
            panic!("a line leaving an elliptical arc takes an angle");
        };
        assert!(
            matches!(
                found.as_slice(),
                [Constraint::Angle { value, .. }] if *value == measure(90.0, Unit::Degree)
            ),
            "{found:?}"
        );
    }

    #[test]
    fn splines_take_equal_lengths_and_distances_from_lines_and_circles() {
        let mut f = fixture();
        let other = f.sketch.add_spline(&[
            Point2::new(5.0, 0.0),
            Point2::new(6.0, 3.0),
            Point2::new(8.0, 0.0),
        ]);

        assert_eq!(
            candidates(&f, ConstraintTool::Equal, &[f.spline, other, f.slanted]),
            Ok(vec![
                Constraint::Equal(f.spline, other),
                Constraint::Equal(f.spline, f.slanted),
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Tangent, &[f.spline, other]),
            Ok(vec![Constraint::Tangent(f.spline, other)])
        );
        let Ok(found) = candidates(&f, ConstraintTool::Distance, &[other, f.horizontal]) else {
            panic!("a spline takes a distance from a line");
        };
        let [Constraint::Distance { from, to, value }] = found.as_slice() else {
            panic!("expected one distance, found {found:?}");
        };
        assert_eq!((*from, *to), (other, f.horizontal));
        assert_eq!(*value, measure(3.5, Unit::Millimetre));
        assert!(candidates(&f, ConstraintTool::Distance, &[f.circle, other]).is_ok());
    }

    #[test]
    fn an_angle_runs_from_a_line_to_the_arc_it_shares_an_end_with() {
        let mut f = fixture();
        let Some(&Entity::Arc { start, .. }) = f.sketch.entity(f.arc) else {
            panic!("expected an arc");
        };
        let leaving = f
            .sketch
            .add_line(Point2::new(9.0, 0.0), Point2::new(3.0, 0.0));
        let Some(&Entity::Line { end: joint, .. }) = f.sketch.entity(leaving) else {
            panic!("expected a line");
        };
        f.sketch
            .add_constraint(Constraint::Coincident(joint, start))
            .unwrap();

        assert_eq!(
            candidates(&f, ConstraintTool::Angle, &[leaving, f.arc]),
            Ok(vec![Constraint::Angle {
                from: leaving,
                to: f.arc,
                reversed: true,
                value: measure(90.0, Unit::Degree),
            }])
        );
        assert!(candidates(&f, ConstraintTool::Angle, &[f.horizontal, f.arc]).is_err());
    }

    #[test]
    fn coincident_perpendicular_and_tangent_take_several_items() {
        let f = fixture();
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Coincident,
                &[f.lone, f.slanted_end, EntityId::ORIGIN]
            ),
            Ok(vec![
                Constraint::Coincident(f.lone, f.slanted_end),
                Constraint::Coincident(f.lone, EntityId::ORIGIN),
            ])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Coincident,
                &[f.lone, EntityId::ORIGIN, f.circle]
            ),
            Ok(vec![
                Constraint::Coincident(f.lone, f.circle),
                Constraint::Coincident(EntityId::ORIGIN, f.circle),
            ])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Perpendicular,
                &[f.horizontal, f.slanted, EntityId::HORIZONTAL_AXIS]
            ),
            Ok(vec![
                Constraint::Perpendicular(f.horizontal, f.slanted),
                Constraint::Perpendicular(f.horizontal, EntityId::HORIZONTAL_AXIS),
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Perpendicular, &[f.arc, f.slanted]),
            Ok(vec![Constraint::Perpendicular(f.slanted, f.arc)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Tangent, &[f.circle, f.slanted, f.arc]),
            Ok(vec![
                Constraint::Tangent(f.slanted, f.circle),
                Constraint::Tangent(f.slanted, f.arc),
            ])
        );
    }

    #[test]
    fn points_line_up_and_several_items_are_chained_to_the_first() {
        let f = fixture();
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Horizontal,
                &[f.slanted_start, f.lone, EntityId::ORIGIN]
            ),
            Ok(vec![
                Constraint::HorizontalPoints(f.slanted_start, f.lone),
                Constraint::HorizontalPoints(f.slanted_start, EntityId::ORIGIN),
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Vertical, &[f.lone, f.slanted_end]),
            Ok(vec![Constraint::VerticalPoints(f.lone, f.slanted_end)])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Parallel,
                &[f.horizontal, f.slanted, EntityId::VERTICAL_AXIS]
            ),
            Ok(vec![
                Constraint::Parallel(f.horizontal, f.slanted),
                Constraint::Parallel(f.horizontal, EntityId::VERTICAL_AXIS),
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Collinear, &[f.horizontal, f.slanted]),
            Ok(vec![Constraint::Collinear(f.horizontal, f.slanted)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Concentric, &[f.circle, f.arc]),
            Ok(vec![Constraint::Concentric(f.circle, f.arc)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Concentric, &[f.circle, f.lone]),
            Ok(vec![Constraint::Concentric(f.lone, f.circle)])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Midpoint, &[f.horizontal, f.lone]),
            Ok(vec![Constraint::Midpoint {
                point: f.lone,
                curve: f.horizontal
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Midpoint, &[f.slanted, f.slanted_end]),
            Err("Point 4 is part of Line 5.".to_owned())
        );
    }

    #[test]
    fn fixing_locks_every_point_of_the_selection_where_it_is_shown() {
        let f = fixture();
        assert_eq!(
            candidates(&f, ConstraintTool::Fix, &[f.slanted, f.slanted_end, f.lone]),
            Ok(vec![
                Constraint::Fix {
                    point: f.slanted_start,
                    at: Point2::ZERO,
                },
                Constraint::Fix {
                    point: f.slanted_end,
                    at: Point2::new(10.0, 10.0),
                },
                Constraint::Fix {
                    point: f.lone,
                    at: Point2::new(3.0, 4.0),
                },
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Fix, &[EntityId::ORIGIN]),
            Err("It only uses reference or projected geometry, which never moves.".to_owned())
        );
    }

    #[test]
    fn symmetry_finds_what_to_mirror_about() {
        let mut sketch = Sketch::new(Plane::XY);
        let left = sketch.add_line(Point2::new(-5.0, 0.0), Point2::new(-2.0, 8.0));
        let right = sketch.add_line(Point2::new(2.1, 8.0), Point2::new(5.0, 0.2));
        let mirror = sketch.add_line(Point2::new(0.0, -1.0), Point2::new(0.0, 9.0));
        let (a, b, c) = (
            sketch.add_point(Point2::new(10.0, 0.0)),
            sketch.add_point(Point2::new(14.0, 0.0)),
            sketch.add_point(Point2::new(12.1, 0.1)),
        );
        let ends = |line| match sketch.entity(line) {
            Some(&Entity::Line { start, end }) => (start, end),
            _ => panic!("expected a line"),
        };
        let ((left_start, left_end), (right_start, right_end)) = (ends(left), ends(right));
        let symmetric = |first, second, about| Constraint::Symmetric {
            first,
            second,
            about,
        };

        assert_eq!(
            ConstraintTool::Symmetric.candidates(&sketch, &sketch, &[mirror, left, right]),
            Ok(vec![
                symmetric(left_start, right_end, mirror),
                symmetric(left_end, right_start, mirror),
            ])
        );
        assert_eq!(
            ConstraintTool::Symmetric.candidates(&sketch, &sketch, &[a, b, c]),
            Ok(vec![symmetric(a, b, c)])
        );
        assert_eq!(
            ConstraintTool::Symmetric.candidates(
                &sketch,
                &sketch,
                &[a, b, EntityId::VERTICAL_AXIS]
            ),
            Ok(vec![symmetric(a, b, EntityId::VERTICAL_AXIS)])
        );
        assert_eq!(
            ConstraintTool::Symmetric.candidates(&sketch, &sketch, &[a, b]),
            Err(
                "Select two points, lines, circles or arcs, and the line or point to mirror them \
                 about"
                    .to_owned()
            )
        );
    }

    #[test]
    fn circles_and_arcs_mirror_with_their_sizes_equal_and_no_constraint_redundant() {
        let mut sketch = Sketch::new(Plane::XY);
        let left = sketch.add_circle(Point2::new(-10.0, 3.0), 2.0);
        let right = sketch.add_circle(Point2::new(11.0, 4.0), 3.0);
        let left_arc = sketch.add_arc(
            Point2::new(-20.0, 0.0),
            Point2::new(-15.0, 0.0),
            Point2::new(-20.0, 5.0),
        );
        let right_arc = sketch.add_arc(
            Point2::new(21.0, 1.0),
            Point2::new(21.0, 6.0),
            Point2::new(15.5, 1.0),
        );
        let axis = EntityId::VERTICAL_AXIS;

        let circles = ConstraintTool::Symmetric.candidates(&sketch, &sketch, &[left, right, axis]);
        let arcs =
            ConstraintTool::Symmetric.candidates(&sketch, &sketch, &[left_arc, right_arc, axis]);
        for constraint in circles
            .clone()
            .unwrap()
            .into_iter()
            .chain(arcs.clone().unwrap())
        {
            sketch.add_constraint(constraint).unwrap();
        }
        let solved = sketch
            .solve(&|_| Ok(Quantity::plain(0.0)), &|| false)
            .unwrap();

        assert_eq!(circles.unwrap().len(), 2);
        assert_eq!(arcs.unwrap().len(), 3);
        assert!(solved.solution.redundancies().is_empty());
        let geometry = solved.geometry;
        let (left_centre, left_radius) = geometry.circle(left).unwrap();
        let (right_centre, right_radius) = geometry.circle(right).unwrap();
        assert!((left_centre.x + right_centre.x).abs() < 1e-7);
        assert!((left_centre.y - right_centre.y).abs() < 1e-7);
        assert!((left_radius - right_radius).abs() < 1e-7);
        let (left_arc, right_arc) = (
            geometry.arc(left_arc).unwrap(),
            geometry.arc(right_arc).unwrap(),
        );
        assert!((left_arc.center.x + right_arc.center.x).abs() < 1e-7);
        assert!((left_arc.center.y - right_arc.center.y).abs() < 1e-7);
        assert!((left_arc.radius - right_arc.radius).abs() < 1e-7);
        assert!((left_arc.sweep - right_arc.sweep).abs() < 1e-7);
    }

    #[test]
    fn horizontal_and_vertical_distances_take_a_line_or_circles_and_refuse_a_level_line() {
        let f = fixture();
        let mm = |value| measure(value, Unit::Millimetre);

        assert_eq!(
            candidates(&f, ConstraintTool::HorizontalDistance, &[f.lone, f.slanted]),
            Ok(vec![Constraint::HorizontalDistance {
                from: f.lone,
                to: f.slanted,
                value: mm(1.0),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::VerticalDistance, &[f.circle, f.lone]),
            Ok(vec![Constraint::VerticalDistance {
                from: f.circle,
                to: f.lone,
                value: mm(4.0),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::HorizontalDistance, &[f.circle, f.arc]),
            Ok(vec![Constraint::HorizontalDistance {
                from: f.circle,
                to: f.arc,
                value: mm(30.0),
            }])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::HorizontalDistance,
                &[f.lone, f.horizontal]
            ),
            Err(format!(
                "{} is horizontal, so a horizontal distance cannot be measured from it.",
                f.sketch.entity_label(f.horizontal)
            ))
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::VerticalDistance,
                &[f.horizontal, f.slanted]
            ),
            Err(ConstraintTool::VerticalDistance.selection_hint().to_owned())
        );
    }

    #[test]
    fn added_dimensions_start_at_the_measured_value() {
        let f = fixture();
        let mm = |value| measure(value, Unit::Millimetre);
        assert_eq!(
            candidates(&f, ConstraintTool::HorizontalDistance, &[f.slanted]),
            Ok(vec![Constraint::HorizontalDistance {
                from: f.slanted_start,
                to: f.slanted_end,
                value: mm(10.0),
            }])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::VerticalDistance,
                &[f.lone, EntityId::ORIGIN]
            ),
            Ok(vec![Constraint::VerticalDistance {
                from: f.lone,
                to: EntityId::ORIGIN,
                value: mm(4.0),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Diameter, &[f.circle, f.arc]),
            Ok(vec![
                Constraint::Diameter {
                    entity: f.circle,
                    value: mm(8.5),
                },
                Constraint::Diameter {
                    entity: f.arc,
                    value: mm(6.0),
                },
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Diameter, &[f.horizontal, f.lone]),
            Ok(vec![Constraint::AxisDiameter {
                point: f.lone,
                axis: f.horizontal,
                value: mm(2.0),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.circle, f.lone]),
            Ok(vec![Constraint::Distance {
                from: f.lone,
                to: f.circle,
                value: mm(23.045),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.arc]),
            Ok(vec![Constraint::ArcLength {
                arc: f.arc,
                value: measure(4.712, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Angle, &[f.arc]),
            Ok(vec![Constraint::Sweep {
                arc: f.arc,
                value: measure(90.0, Unit::Degree),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Angle, &[f.circle]),
            Err(
                "Select two lines, a line and an arc or elliptical arc sharing an end, or one arc"
                    .to_owned()
            )
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.circle, f.horizontal]),
            Ok(vec![Constraint::Distance {
                from: f.circle,
                to: f.horizontal,
                value: measure(0.75, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.arc, f.circle]),
            Ok(vec![Constraint::Distance {
                from: f.arc,
                to: f.circle,
                value: measure(22.75, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Distance,
                &[f.horizontal, EntityId::HORIZONTAL_AXIS]
            ),
            Ok(vec![Constraint::Distance {
                from: f.horizontal,
                to: EntityId::HORIZONTAL_AXIS,
                value: mm(5.0),
            }])
        );
        assert_eq!(
            in_unit(
                vec![Constraint::Diameter {
                    entity: f.circle,
                    value: mm(8.5),
                }],
                LengthUnit::Centimetre
            ),
            vec![Constraint::Diameter {
                entity: f.circle,
                value: measure(0.85, Unit::Centimetre),
            }]
        );
    }

    #[test]
    fn an_angle_dimension_starts_in_the_chosen_angle_unit() {
        let f = fixture();
        let found = candidates(&f, ConstraintTool::Angle, &[f.horizontal, f.slanted]).unwrap();
        let radians = Units {
            length: LengthUnit::Millimetre,
            angle: crate::units::AngleUnit::Radian,
        };

        let turned = in_unit(found.clone(), radians);
        let [Constraint::Angle { value, .. }] = turned.as_slice() else {
            panic!("expected one angle");
        };
        let [
            Constraint::Angle {
                value: original, ..
            },
        ] = found.as_slice()
        else {
            panic!("expected one angle");
        };

        assert_eq!(*original, measure(45.0, Unit::Degree));
        let Expression::Measure(turned, Unit::Radian) = value else {
            panic!("expected radians");
        };
        assert!((turned - std::f64::consts::FRAC_PI_4).abs() < 1e-6);
        assert_eq!(in_unit(found.clone(), LengthUnit::Centimetre), found);
    }

    #[test]
    fn dimensions_start_at_the_measured_value() {
        let f = fixture();
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.slanted]),
            Ok(vec![Constraint::Distance {
                from: f.slanted_start,
                to: f.slanted_end,
                value: measure(14.142, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.lone, EntityId::ORIGIN]),
            Ok(vec![Constraint::Distance {
                from: f.lone,
                to: EntityId::ORIGIN,
                value: measure(5.0, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Distance, &[f.horizontal, f.lone]),
            Ok(vec![Constraint::Distance {
                from: f.lone,
                to: f.horizontal,
                value: measure(1.0, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Distance,
                &[f.lone, EntityId::VERTICAL_AXIS]
            ),
            Ok(vec![Constraint::Distance {
                from: f.lone,
                to: EntityId::VERTICAL_AXIS,
                value: measure(3.0, Unit::Millimetre),
            }])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Radius, &[f.circle, f.arc]),
            Ok(vec![
                Constraint::Radius {
                    entity: f.circle,
                    value: measure(4.25, Unit::Millimetre),
                },
                Constraint::Radius {
                    entity: f.arc,
                    value: measure(3.0, Unit::Millimetre),
                },
            ])
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Radius, &[f.slanted]),
            Err("Select one or more circles, arcs or ellipses".to_owned())
        );
    }

    #[test]
    fn angles_are_ordered_so_the_value_matches_the_drawing() {
        let f = fixture();
        let expected = Ok(vec![Constraint::Angle {
            from: f.horizontal,
            to: f.slanted,
            reversed: false,
            value: measure(45.0, Unit::Degree),
        }]);
        assert_eq!(
            candidates(&f, ConstraintTool::Angle, &[f.horizontal, f.slanted]),
            expected
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Angle, &[f.slanted, f.horizontal]),
            expected
        );
        assert_eq!(
            candidates(
                &f,
                ConstraintTool::Angle,
                &[f.slanted, EntityId::VERTICAL_AXIS]
            ),
            Ok(vec![Constraint::Angle {
                from: f.slanted,
                to: EntityId::VERTICAL_AXIS,
                reversed: false,
                value: measure(45.0, Unit::Degree),
            }])
        );
    }

    #[test]
    fn the_angle_of_a_corner_in_a_chain_is_measured_inside_it() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let second = sketch.add_line(
            Point2::new(10.0, 0.0),
            Point2::new(5.0, 5.0 * 3.0_f64.sqrt()),
        );
        let found = ConstraintTool::Angle.candidates(&sketch, &sketch, &[first, second]);
        let Ok(constraints) = found else {
            panic!("two lines can take an angle");
        };
        let [
            Constraint::Angle {
                reversed, value, ..
            },
        ] = constraints.as_slice()
        else {
            panic!("expected one angle");
        };
        assert!(*reversed);
        assert_eq!(*value, measure(60.0, Unit::Degree));
    }

    #[test]
    fn measured_values_are_rounded_only_slightly() {
        assert_eq!(rounded_for_display(37.253_123), 37.253);
        assert_eq!(rounded_for_display(-2.000_4), -2.0);
        assert_eq!(rounded_for_display(0.012_345_67), 0.012_345_7);
        assert_eq!(rounded_for_display(0.0), 0.0);
        assert_eq!(rounded_for_display(1e-320), 1e-320);
    }
}

pub const NOTHING_TO_SPLIT: &str = "Select a point lying on a line, arc or elliptical arc, with \
                                    that curve when the point lies on several";
pub const SPLIT_TITLE: &str = "Split curve";
pub const NOTHING_TO_BREAK: &str =
    "Select the lines, arcs and elliptical arcs to break at their crossings";
pub const BREAK_TITLE: &str = "Break curves";
pub const NOTHING_TO_RESPACE: &str = "Select fit-point splines drawn before their curves followed \
                                      the spacing of their points";
pub const RESPACE_TITLE: &str = "Respace fit-point splines";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RespaceChange {
    pub splines: Vec<EntityId>,
}

impl RespaceChange {
    pub fn of(sketch: &Sketch, selected: &[EntityId]) -> Result<Self, String> {
        let splines: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|id| {
                !id.is_reference()
                    && !sketch.is_projected(*id)
                    && matches!(
                        sketch.entity(*id).and_then(Entity::spline_kind),
                        Some(SplineKind::Fit {
                            spacing: FitSpacing::Even,
                            ..
                        })
                    )
            })
            .collect();
        if splines.is_empty() {
            Err(NOTHING_TO_RESPACE.to_owned())
        } else {
            Ok(Self { splines })
        }
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Result<Transaction, String> {
        crate::trimming::reshaped(model, feature, RESPACE_TITLE.to_owned(), |sketch| {
            for spline in &self.splines {
                sketch
                    .respace_fit_spline(*spline)
                    .map_err(|error| format!("{RESPACE_TITLE}: {error}."))?;
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitChange {
    pub curve: EntityId,
    pub point: EntityId,
}

impl SplitChange {
    pub fn of(sketch: &Sketch, selected: &[EntityId]) -> Result<Self, String> {
        let (points, curves): (Vec<EntityId>, Vec<EntityId>) = selected
            .iter()
            .copied()
            .filter(|id| !id.is_reference())
            .partition(|id| matches!(sketch.entity(*id), Some(Entity::Point(_))));
        let change = match (points.as_slice(), curves.as_slice()) {
            ([point], [curve]) => Self {
                curve: *curve,
                point: *point,
            },
            ([point], []) => {
                let carriers: BTreeSet<EntityId> = sketch
                    .constraints_using(*point)
                    .into_iter()
                    .filter_map(|id| match sketch.constraint(id)? {
                        Constraint::Coincident(a, b) if a == point => Some(*b),
                        Constraint::Coincident(a, b) if b == point => Some(*a),
                        _ => None,
                    })
                    .filter(|curve| {
                        matches!(
                            sketch.entity(*curve),
                            Some(
                                Entity::Line { .. }
                                    | Entity::Arc { .. }
                                    | Entity::EllipticalArc { .. }
                            )
                        )
                    })
                    .collect();
                let mut carriers = carriers.into_iter();
                match (carriers.next(), carriers.next()) {
                    (Some(curve), None) => Self {
                        curve,
                        point: *point,
                    },
                    _ => return Err(NOTHING_TO_SPLIT.to_owned()),
                }
            }
            _ => return Err(NOTHING_TO_SPLIT.to_owned()),
        };
        sketch
            .check_split(change.curve, change.point)
            .map_err(|error| crate::trimming::capitalized(&error.to_string()))?;
        Ok(change)
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Result<Transaction, String> {
        crate::trimming::reshaped(model, feature, SPLIT_TITLE.to_owned(), |sketch| {
            sketch
                .split_at(self.curve, self.point)
                .map(|_| ())
                .map_err(|error| format!("{SPLIT_TITLE}: {error}."))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakChange {
    pub curves: Vec<EntityId>,
}

impl BreakChange {
    pub fn of(sketch: &Sketch, selected: &[EntityId]) -> Result<Self, String> {
        let curves: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|id| {
                !id.is_reference()
                    && matches!(
                        sketch.entity(*id),
                        Some(
                            Entity::Line { .. } | Entity::Arc { .. } | Entity::EllipticalArc { .. }
                        )
                    )
            })
            .collect();
        if curves.is_empty() {
            Err(NOTHING_TO_BREAK.to_owned())
        } else {
            Ok(Self { curves })
        }
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Result<Transaction, String> {
        crate::trimming::reshaped(model, feature, BREAK_TITLE.to_owned(), |sketch| {
            sketch
                .break_curves(&self.curves)
                .map(|_| ())
                .map_err(|error| format!("{BREAK_TITLE}: {error}."))
        })
    }
}
