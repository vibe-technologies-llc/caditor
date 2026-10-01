use caditor_document::{Edit, FeatureId, Transaction, TransactionBuilder};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Reference, Sketch};

use crate::{
    feature_tree::count,
    field::sentence,
    model::Model,
    selection::{Pickable, Selection},
    units::LengthUnit,
    variants::all_variants,
};

const DISPLAY_DECIMALS: f64 = 3.0;
const SIGNIFICANT_DIGITS: f64 = 6.0;
const DEGENERATE_LENGTH: f64 = 1e-12;

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
}

type Item = (EntityId, Shape);

all_variants!(ConstraintTool: Coincident, Midpoint, Concentric, Collinear, Fix, Horizontal, Vertical, Parallel, Perpendicular, Tangent, Equal, Symmetric, Distance, HorizontalDistance, VerticalDistance, Angle, Radius, Diameter);

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
            Self::Midpoint => "Put a point at the middle of a line",
            Self::Concentric => "Give circles and arcs one centre, or put a point at their centre",
            Self::Collinear => "Put lines on one straight line",
            Self::Fix => "Lock points where they are; a curve is locked by its points",
            Self::Horizontal => "Make lines horizontal, or line points up horizontally",
            Self::Vertical => "Make lines vertical, or line points up vertically",
            Self::Parallel => "Make lines parallel",
            Self::Perpendicular => "Make two lines meet at a right angle",
            Self::Tangent => "Make a line and a curve, or two curves, touch smoothly",
            Self::Equal => "Give lines the same length, or circles and arcs the same radius",
            Self::Symmetric => "Mirror two points, or two lines, about a line or a point",
            Self::Distance => {
                "Fix the distance between two points, a point and a line or circle, two lines, or \
                 the ends of a line"
            }
            Self::HorizontalDistance => {
                "Fix the horizontal distance between two points or the ends of a line"
            }
            Self::VerticalDistance => {
                "Fix the vertical distance between two points or the ends of a line"
            }
            Self::Angle => "Fix the angle between two lines",
            Self::Radius => "Fix the radius of circles and arcs",
            Self::Diameter => "Fix the diameter of circles and arcs",
        }
    }

    pub fn selection_hint(self) -> &'static str {
        match self {
            Self::Coincident => "Select two points, or a point and a line, circle, arc or spline",
            Self::Midpoint => "Select a point and a line",
            Self::Concentric => {
                "Select two or more circles or arcs, or a point and a circle or arc"
            }
            Self::Collinear | Self::Parallel => "Select two or more lines",
            Self::Fix => "Select the points or curves to lock",
            Self::Horizontal | Self::Vertical => "Select one or more lines, or two or more points",
            Self::Perpendicular | Self::Angle => "Select two lines",
            Self::Tangent => {
                "Select a line, circle or arc, and a circle, arc or spline to touch it"
            }
            Self::Equal => "Select two or more lines, or two or more circles or arcs",
            Self::Symmetric => {
                "Select two points or two lines, and the line or point to mirror them about"
            }
            Self::Distance => {
                "Select two points, a point and a line or circle, two lines, or one line"
            }
            Self::HorizontalDistance | Self::VerticalDistance => "Select two points or one line",
            Self::Radius | Self::Diameter => "Select one or more circles or arcs",
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

    pub fn candidates(
        self,
        definition: &Sketch,
        shown: &Sketch,
        selected: &[EntityId],
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
        new_relations(definition, constraints)
    }

    fn propose(
        self,
        definition: &Sketch,
        shown: &Sketch,
        items: &[Item],
    ) -> Option<Vec<Constraint>> {
        use Shape::{Circular, Line, Point, Spline};
        match (self, items) {
            (Self::Coincident, &[(a, Point), (b, Point | Line | Circular | Spline)])
            | (Self::Coincident, &[(a, Line | Circular | Spline), (b, Point)]) => {
                Some(vec![Constraint::Coincident(a, b)])
            }
            (Self::Midpoint, &[(point, Point), (line, Line)] | &[(line, Line), (point, Point)]) => {
                Some(vec![Constraint::Midpoint { point, line }])
            }
            (
                Self::Concentric,
                &[(point, Point), (curve, Circular)] | &[(curve, Circular), (point, Point)],
            ) => Some(vec![Constraint::Concentric(point, curve)]),
            (Self::Concentric, _) => chained(items, Circular, Constraint::Concentric),
            (Self::Collinear, _) => chained(items, Line, Constraint::Collinear),
            (Self::Fix, _) => fixed(definition, shown, items),
            (Self::Horizontal, &[(_, Point), ..]) => {
                chained(items, Point, Constraint::HorizontalPoints)
            }
            (Self::Vertical, &[(_, Point), ..]) => {
                chained(items, Point, Constraint::VerticalPoints)
            }
            (Self::Horizontal, _) => each(items, Line, Constraint::Horizontal),
            (Self::Vertical, _) => each(items, Line, Constraint::Vertical),
            (Self::Parallel, _) => chained(items, Line, Constraint::Parallel),
            (Self::Perpendicular, &[(a, Line), (b, Line)]) => {
                Some(vec![Constraint::Perpendicular(a, b)])
            }
            (Self::Tangent, &[(a, Line | Circular | Spline), (b, Line | Circular | Spline)]) => {
                Some(vec![Constraint::Tangent(a, b)])
            }
            (Self::Equal, _) => chained(items, Line, Constraint::Equal)
                .or_else(|| chained(items, Circular, Constraint::Equal)),
            (Self::Symmetric, _) => symmetric(definition, shown, items),
            (Self::Distance, &[(line, Line)]) => {
                let Some(&Entity::Line { start, end }) = definition.entity(line) else {
                    return None;
                };
                let (from, to) = shown.line_endpoints(line)?;
                Some(vec![distance(start, end, from.distance(to))])
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
                | &[(from, Line), (to, Line)],
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
            (Self::Angle, &[(a, Line), (b, Line)]) => Some(vec![angle(shown, a, b)?]),
            (Self::Radius, _) => each_measured(shown, items, |entity, value| Constraint::Radius {
                entity,
                value,
            }),
            (Self::Diameter, _) => each_measured(shown, items, |entity, value| {
                Constraint::Diameter { entity, value }
            }),
            _ => None,
        }
    }

    fn offset(self, shown: &Sketch, from: EntityId, to: EntityId) -> Option<Constraint> {
        measured(shown, |value| match self {
            Self::VerticalDistance => Constraint::VerticalDistance { from, to, value },
            _ => Constraint::HorizontalDistance { from, to, value },
        })
    }
}

fn new_relations(
    definition: &Sketch,
    constraints: Vec<Constraint>,
) -> Result<Vec<Constraint>, String> {
    for constraint in &constraints {
        if let Some(existing) = definition.contradicting(constraint) {
            return Err(format!(
                "{} would contradict {}, which is already in the sketch. Delete that first.",
                definition.describe(constraint),
                definition.describe_constraint(existing)
            ));
        }
    }
    let (restated, fresh): (Vec<Constraint>, Vec<Constraint>) = constraints
        .into_iter()
        .partition(|constraint| definition.restating(constraint).is_some());
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
    })
}

fn each(
    items: &[Item],
    needed: Shape,
    make: fn(EntityId) -> Constraint,
) -> Option<Vec<Constraint>> {
    items
        .iter()
        .map(|(entity, shape)| (*shape == needed).then(|| make(*entity)))
        .collect::<Option<Vec<_>>>()
        .filter(|constraints| !constraints.is_empty())
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
    for (entity, shape) in items {
        let owned = match shape {
            Shape::Point => vec![*entity],
            Shape::Line | Shape::Circular | Shape::Spline => definition.entity(*entity)?.points(),
        };
        for point in owned {
            if !points.contains(&point) {
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
            Shape::Circular | Shape::Spline => None,
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
        _ => return None,
    };
    let constraints = pairs
        .iter()
        .map(|(first, second)| {
            if first == second {
                Constraint::Coincident(*first, about.0)
            } else {
                Constraint::Symmetric {
                    first: *first,
                    second: *second,
                    about: about.0,
                }
            }
        })
        .collect();
    Some((error(&pairs)?, constraints))
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

pub fn in_unit(constraints: Vec<Constraint>, unit: LengthUnit) -> Vec<Constraint> {
    let converted = |value: Expression| match value {
        Expression::Measure(length, Unit::Millimetre) => unit.measured(length),
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
            other => other,
        })
        .collect()
}

fn millimetres(length: f64) -> Expression {
    Expression::Measure(rounded_for_display(length), Unit::Millimetre)
}

pub fn rounded_for_display(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let magnitude = value.abs().log10().floor();
    let decimals = if magnitude >= 0.0 {
        DISPLAY_DECIMALS
    } else {
        SIGNIFICANT_DIGITS - 1.0 - magnitude
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
            | Pickable::Region { .. }
            | Pickable::BlendEdge { .. }
            | Pickable::ShellFace { .. }
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
            | Pickable::Region { .. }
            | Pickable::BlendEdge { .. }
            | Pickable::ShellFace { .. }
            | Pickable::Datum(_) => None,
        })
        .collect()
}

pub struct Added {
    pub transaction: Transaction,
    pub constraints: Vec<ConstraintId>,
}

pub fn add_constraints(
    model: &Model,
    feature: FeatureId,
    tool: ConstraintTool,
    constraints: Vec<Constraint>,
) -> Added {
    let mut transaction = settled_transaction(model, feature, format!("Add {}", tool.label()));
    let constraints = constraints
        .into_iter()
        .map(|constraint| transaction.add_sketch_constraint(feature, constraint))
        .collect();
    Added {
        transaction: transaction.finish(),
        constraints,
    }
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
    use caditor_geometry::Plane;

    use super::*;

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
            Err("Select one or more lines, or two or more points".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[]),
            Err("Select one or more lines, or two or more points".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[EntityId::HORIZONTAL_AXIS]),
            Err("It only uses reference geometry, which never moves.".to_owned())
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
            Err("Select two points, or a point and a line, circle, arc or spline".to_owned())
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
                line: f.horizontal
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
            Err("It only uses reference geometry, which never moves.".to_owned())
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
                "Select two points or two lines, and the line or point to mirror them about"
                    .to_owned()
            )
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
            candidates(&f, ConstraintTool::Distance, &[f.circle, f.lone]),
            Ok(vec![Constraint::Distance {
                from: f.lone,
                to: f.circle,
                value: mm(23.045),
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
            Err("Select one or more circles or arcs".to_owned())
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
