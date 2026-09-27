use caditor_document::{FeatureId, Transaction, TransactionBuilder};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Reference, Sketch};
use egui::Key;

use crate::{
    field::sentence,
    model::Model,
    selection::{Pickable, Selection},
};

const DISPLAY_DECIMALS: f64 = 3.0;
const SIGNIFICANT_DIGITS: f64 = 6.0;
const DEGENERATE_LENGTH: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintTool {
    Coincident,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Tangent,
    Equal,
    Distance,
    Angle,
    Radius,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Point,
    Line,
    Circular,
    Spline,
}

type Item = (EntityId, Shape);

impl ConstraintTool {
    pub const ALL: [Self; 10] = [
        Self::Coincident,
        Self::Horizontal,
        Self::Vertical,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
        Self::Equal,
        Self::Distance,
        Self::Angle,
        Self::Radius,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Coincident => "Coincident",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
            Self::Equal => "Equal",
            Self::Distance => "Distance",
            Self::Angle => "Angle",
            Self::Radius => "Radius",
        }
    }

    pub fn key(self) -> Key {
        match self {
            Self::Coincident => Key::C,
            Self::Horizontal => Key::H,
            Self::Vertical => Key::V,
            Self::Parallel => Key::P,
            Self::Perpendicular => Key::L,
            Self::Tangent => Key::T,
            Self::Equal => Key::E,
            Self::Distance => Key::D,
            Self::Angle => Key::A,
            Self::Radius => Key::R,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Coincident => "Join two points, or put a point on a curve",
            Self::Horizontal => "Make lines horizontal",
            Self::Vertical => "Make lines vertical",
            Self::Parallel => "Make two lines parallel",
            Self::Perpendicular => "Make two lines meet at a right angle",
            Self::Tangent => "Make a line and a curve, or two curves, touch smoothly",
            Self::Equal => "Give two lines the same length, or two circles or arcs the same radius",
            Self::Distance => {
                "Fix the distance between two points, a point and a line, or the ends of a line"
            }
            Self::Angle => "Fix the angle between two lines",
            Self::Radius => "Fix the radius of circles and arcs",
        }
    }

    pub fn selection_hint(self) -> &'static str {
        match self {
            Self::Coincident => "Select two points, or a point and a line, circle or arc",
            Self::Horizontal | Self::Vertical => "Select one or more lines",
            Self::Parallel | Self::Perpendicular | Self::Angle => "Select two lines",
            Self::Tangent => "Select a line and a circle or arc, or two circles or arcs",
            Self::Equal => "Select two lines, or two circles or arcs",
            Self::Distance => "Select two points, a point and a line, or one line",
            Self::Radius => "Select one or more circles or arcs",
        }
    }

    pub fn is_dimension(self) -> bool {
        matches!(self, Self::Distance | Self::Angle | Self::Radius)
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
        Ok(constraints)
    }

    fn propose(
        self,
        definition: &Sketch,
        shown: &Sketch,
        items: &[Item],
    ) -> Option<Vec<Constraint>> {
        use Shape::{Circular, Line, Point};
        match (self, items) {
            (Self::Coincident, &[(a, Point), (b, Point | Line | Circular)])
            | (Self::Coincident, &[(a, Line | Circular), (b, Point)]) => {
                Some(vec![Constraint::Coincident(a, b)])
            }
            (Self::Horizontal, _) => each(items, Line, Constraint::Horizontal),
            (Self::Vertical, _) => each(items, Line, Constraint::Vertical),
            (Self::Parallel, &[(a, Line), (b, Line)]) => Some(vec![Constraint::Parallel(a, b)]),
            (Self::Perpendicular, &[(a, Line), (b, Line)]) => {
                Some(vec![Constraint::Perpendicular(a, b)])
            }
            (Self::Tangent, &[(a, Line | Circular), (b, Circular)])
            | (Self::Tangent, &[(a, Circular), (b, Line)]) => Some(vec![Constraint::Tangent(a, b)]),
            (Self::Equal, &[(a, Line), (b, Line)] | &[(a, Circular), (b, Circular)]) => {
                Some(vec![Constraint::Equal(a, b)])
            }
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
            (Self::Angle, &[(a, Line), (b, Line)]) => Some(vec![angle(shown, a, b)?]),
            (Self::Radius, _) => items
                .iter()
                .map(|(entity, shape)| {
                    (*shape == Circular).then_some(())?;
                    let (_, radius) = shown.circle(*entity)?;
                    Some(Constraint::Radius {
                        entity: *entity,
                        value: millimetres(radius),
                    })
                })
                .collect::<Option<Vec<_>>>()
                .filter(|constraints| !constraints.is_empty()),
            _ => None,
        }
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
    let signed = first.perp_dot(second).atan2(first.dot(second)).to_degrees();
    let (from, to) = if signed < 0.0 { (b, a) } else { (a, b) };
    Some(Constraint::Angle {
        from,
        to,
        value: Expression::Measure(rounded_for_display(signed.abs()), Unit::Degree),
    })
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
            | Pickable::Plane(_) => None,
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
            | Pickable::Plane(_) => None,
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
            Err("Select one or more lines".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[]),
            Err("Select one or more lines".to_owned())
        );
        assert_eq!(
            candidates(&f, ConstraintTool::Horizontal, &[EntityId::HORIZONTAL_AXIS]),
            Err("It only uses reference geometry, which never moves.".to_owned())
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
            Err("Select two lines".to_owned())
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
            Err("Select two points, or a point and a line, circle or arc".to_owned())
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
        assert!(candidates(&f, ConstraintTool::Coincident, &[f.lone, f.spline]).is_err());
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
                value: measure(45.0, Unit::Degree),
            }])
        );
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
