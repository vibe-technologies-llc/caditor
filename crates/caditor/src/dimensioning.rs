use std::f64::consts::TAU;

use caditor_document::FeatureId;
use caditor_expression::Expression;
use caditor_geometry::Point2;
use caditor_sketch::{Constraint, Entity, EntityId, Reference, Sketch};

use crate::{
    model::Model,
    sketch_tools::{self, Added, ConstraintTool},
};

pub const PICK_FIRST: &str = "Click a line, circle, arc or point to dimension it";
pub const FIRST_KEYS: &str = "Esc: back to Select";
pub const PICKED_KEYS: &str =
    "Enter: dimension it   Click empty space: dimension it as placed   Esc: start again";
pub const PLACING_KEYS: &str = "Enter: the aligned distance   Esc: start again";
pub const POINT_KEYS: &str = "Esc: start again";
pub const AXIS_KEYS: &str = "Enter: the distance   Esc: start again";
const SPLINE_REFUSED: &str = "A spline takes only a distance from a point, line, circle, arc or \
                              ellipse; dimension the points or lines that shape it otherwise";
const NOT_IN_SKETCH: &str = "That is not part of the sketch being edited";
const ELLIPSE_REFUSED: &str = "An ellipse takes its major and minor radii alone, or a distance from \
                               one point or curve; dimension its centre and axis points otherwise";
const PARALLEL_TOLERANCE: f64 = 1e-9;
const LEVEL_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Point,
    Line,
    Circle,
    Arc,
    Spline,
    Ellipse,
}

fn kind(sketch: &Sketch, id: EntityId) -> Option<Kind> {
    match id.reference() {
        Some(Reference::Origin) => return Some(Kind::Point),
        Some(Reference::HorizontalAxis | Reference::VerticalAxis) => return Some(Kind::Line),
        None => {}
    }
    Some(match sketch.entity(id)? {
        Entity::Point(_) => Kind::Point,
        Entity::Line { .. } => Kind::Line,
        Entity::Circle { .. } => Kind::Circle,
        Entity::Arc { .. } => Kind::Arc,
        Entity::Spline { .. } => Kind::Spline,
        Entity::Ellipse { .. } | Entity::EllipticalArc { .. } => Kind::Ellipse,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Ready(ConstraintTool),
    Waiting,
    Refused(&'static str),
}

pub fn fitting(sketch: &Sketch, picks: &[EntityId]) -> Fit {
    let kinds: Option<Vec<Kind>> = picks.iter().map(|id| kind(sketch, *id)).collect();
    let Some(kinds) = kinds else {
        return Fit::Refused(NOT_IN_SKETCH);
    };
    match kinds.as_slice() {
        [Kind::Ellipse] => return Fit::Ready(ConstraintTool::Radius),
        [Kind::Line, Kind::Ellipse] | [Kind::Ellipse, Kind::Line]
            if picks
                .first()
                .zip(picks.get(1))
                .is_some_and(|(first, second)| sketch.angle_vertex(*first, *second).is_some()) =>
        {
            return Fit::Ready(ConstraintTool::Angle);
        }
        [_, Kind::Ellipse] | [Kind::Ellipse, _] => return Fit::Ready(ConstraintTool::Distance),
        _ if kinds.contains(&Kind::Ellipse) => return Fit::Refused(ELLIPSE_REFUSED),
        [Kind::Spline]
            if picks
                .iter()
                .all(|pick| sketch_tools::is_conic(sketch, *pick)) =>
        {
            return Fit::Ready(ConstraintTool::Radius);
        }
        [Kind::Spline] => return Fit::Waiting,
        [
            Kind::Point | Kind::Line | Kind::Circle | Kind::Arc,
            Kind::Spline,
        ]
        | [
            Kind::Spline,
            Kind::Point | Kind::Line | Kind::Circle | Kind::Arc,
        ] => {
            return Fit::Ready(ConstraintTool::Distance);
        }
        _ if kinds.contains(&Kind::Spline) => return Fit::Refused(SPLINE_REFUSED),
        _ => {}
    }
    match (kinds.as_slice(), picks) {
        ([] | [Kind::Point], _) => Fit::Waiting,
        ([Kind::Line], _) => Fit::Ready(ConstraintTool::Distance),
        ([Kind::Circle], _) => Fit::Ready(ConstraintTool::Diameter),
        ([Kind::Arc], _) => Fit::Ready(ConstraintTool::Radius),
        ([Kind::Line, Kind::Line], &[first, second]) if !parallel(sketch, first, second) => {
            Fit::Ready(ConstraintTool::Angle)
        }
        ([Kind::Line, Kind::Arc] | [Kind::Arc, Kind::Line], &[first, second])
            if sketch.angle_vertex(first, second).is_some() =>
        {
            Fit::Ready(ConstraintTool::Angle)
        }
        ([_, _], _) => Fit::Ready(ConstraintTool::Distance),
        _ => Fit::Refused(PICK_FIRST),
    }
}

pub fn awaits_placement(sketch: &Sketch, picks: &[EntityId]) -> bool {
    let both_points = |first: &EntityId, second: &EntityId| {
        kind(sketch, *first) == Some(Kind::Point) && kind(sketch, *second) == Some(Kind::Point)
    };
    match picks {
        [first, second] if both_points(first, second) => true,
        _ => about_axis(sketch, picks).is_some() || offset_ends(sketch, picks).is_some(),
    }
}

fn offset_ends(sketch: &Sketch, picks: &[EntityId]) -> Option<[(Point2, Point2); 2]> {
    let &[from, to] = picks else {
        return None;
    };
    let measurable = matches!(
        (kind(sketch, from)?, kind(sketch, to)?),
        (
            Kind::Point | Kind::Circle | Kind::Arc,
            Kind::Circle | Kind::Arc | Kind::Line
        ) | (Kind::Circle | Kind::Arc | Kind::Line, Kind::Point)
            | (Kind::Line, Kind::Circle | Kind::Arc)
    );
    if !measurable {
        return None;
    }
    let value = Expression::Number(0.0);
    let ends = |constraint: Constraint| {
        sketch
            .axis_offset_ends(&constraint)
            .map(|(start, end, _)| (start, end))
    };
    let across = ends(Constraint::HorizontalDistance {
        from,
        to,
        value: value.clone(),
    })?;
    let upright = ends(Constraint::VerticalDistance { from, to, value })?;
    let (wide, tall) = (
        (across.1 - across.0).x.abs(),
        (upright.1 - upright.0).y.abs(),
    );
    let tolerance = LEVEL_TOLERANCE * wide.max(tall);
    (wide > tolerance && tall > tolerance).then_some([across, upright])
}

fn oriented_between([across, upright]: [(Point2, Point2); 2], pointer: Point2) -> ConstraintTool {
    let within = |low: f64, high: f64, at: f64| (low.min(high)..=low.max(high)).contains(&at);
    match (
        within(across.0.x, across.1.x, pointer.x),
        within(upright.0.y, upright.1.y, pointer.y),
    ) {
        (true, false) => ConstraintTool::HorizontalDistance,
        (false, true) => ConstraintTool::VerticalDistance,
        _ => ConstraintTool::Distance,
    }
}

fn about_axis(sketch: &Sketch, picks: &[EntityId]) -> Option<(EntityId, EntityId)> {
    let (point, axis) = match *picks {
        [point, axis] if kind(sketch, point) == Some(Kind::Point) => (point, axis),
        [axis, point] if kind(sketch, point) == Some(Kind::Point) => (point, axis),
        _ => return None,
    };
    let is_axis = kind(sketch, axis) == Some(Kind::Line) && sketch.is_construction(axis);
    is_axis.then_some((point, axis))
}

fn across_axis(sketch: &Sketch, point: EntityId, axis: EntityId, pointer: Point2) -> Option<bool> {
    let point = sketch.point(point)?;
    let (start, end) = sketch.line_endpoints(axis)?;
    let along = end - start;
    Some(along.perp_dot(point - start) * along.perp_dot(pointer - start) < 0.0)
}

pub fn placed(
    sketch: &Sketch,
    picks: &[EntityId],
    pointer: Option<Point2>,
) -> Option<ConstraintTool> {
    let Fit::Ready(tool) = fitting(sketch, picks) else {
        return None;
    };
    let Some(pointer) = pointer else {
        return Some(tool);
    };
    let kinds: Vec<Option<Kind>> = picks.iter().map(|id| kind(sketch, *id)).collect();
    let placed = match (kinds.as_slice(), picks) {
        ([Some(Kind::Line)], [line]) => sketch
            .line_endpoints(*line)
            .map(|(start, end)| oriented(start, end, pointer)),
        ([Some(Kind::Point), Some(Kind::Point)], [first, second]) => sketch
            .point(*first)
            .zip(sketch.point(*second))
            .map(|(start, end)| oriented(start, end, pointer)),
        ([Some(Kind::Point), Some(Kind::Line)] | [Some(Kind::Line), Some(Kind::Point)], _) => {
            about_axis(sketch, picks)
                .and_then(|(point, axis)| across_axis(sketch, point, axis, pointer))
                .map(|across| {
                    if across {
                        ConstraintTool::Diameter
                    } else {
                        ConstraintTool::Distance
                    }
                })
        }
        ([Some(Kind::Arc)], [arc]) => sketch.arc(*arc).map(|arc| {
            let reach = pointer - arc.center;
            let turned = (reach.y.atan2(reach.x) - arc.start_angle).rem_euclid(TAU);
            if turned > arc.sweep {
                ConstraintTool::Radius
            } else if reach.length() > arc.radius {
                ConstraintTool::Distance
            } else {
                ConstraintTool::Angle
            }
        }),
        _ => None,
    }
    .or_else(|| offset_ends(sketch, picks).map(|ends| oriented_between(ends, pointer)));
    Some(placed.unwrap_or(tool))
}

fn oriented(start: Point2, end: Point2, pointer: Point2) -> ConstraintTool {
    let (low, high) = (start.min(end), start.max(end));
    let span = high - low;
    let tolerance = LEVEL_TOLERANCE * span.length();
    if span.x <= tolerance || span.y <= tolerance {
        return ConstraintTool::Distance;
    }
    let within_x = (low.x..=high.x).contains(&pointer.x);
    let within_y = (low.y..=high.y).contains(&pointer.y);
    match (within_x, within_y) {
        (true, false) => ConstraintTool::HorizontalDistance,
        (false, true) => ConstraintTool::VerticalDistance,
        _ => ConstraintTool::Distance,
    }
}

fn parallel(sketch: &Sketch, first: EntityId, second: EntityId) -> bool {
    let (Some(a), Some(b)) = (sketch.line_direction(first), sketch.line_direction(second)) else {
        return true;
    };
    a.perp_dot(b).abs() <= PARALLEL_TOLERANCE * a.length() * b.length()
}

pub fn words(sketch: &Sketch, tool: ConstraintTool, picks: &[EntityId]) -> String {
    let label = |id: &EntityId| sketch.entity_label(*id);
    let arc = |id: &EntityId| kind(sketch, *id) == Some(Kind::Arc);
    match (tool, picks) {
        (ConstraintTool::Distance, [arc_picked]) if arc(arc_picked) => {
            format!("the length of {}", label(arc_picked))
        }
        (ConstraintTool::Angle, [arc_picked]) if arc(arc_picked) => {
            format!("the sweep of {}", label(arc_picked))
        }
        (ConstraintTool::HorizontalDistance, [line]) => {
            format!("the horizontal length of {}", label(line))
        }
        (ConstraintTool::VerticalDistance, [line]) => {
            format!("the vertical length of {}", label(line))
        }
        (ConstraintTool::HorizontalDistance, [first, second]) => format!(
            "the horizontal distance between {} and {}",
            label(first),
            label(second)
        ),
        (ConstraintTool::VerticalDistance, [first, second]) => format!(
            "the vertical distance between {} and {}",
            label(first),
            label(second)
        ),
        (ConstraintTool::Distance, [line]) => format!("the length of {}", label(line)),
        (ConstraintTool::Diameter, [circle]) => format!("the diameter of {}", label(circle)),
        (ConstraintTool::Diameter, [_, _]) => match about_axis(sketch, picks) {
            Some((point, axis)) => {
                format!("the diameter of {} across {}", label(&point), label(&axis))
            }
            None => format!("a {}", ConstraintTool::Diameter.label().to_lowercase()),
        },
        (ConstraintTool::Radius, [ellipse]) if kind(sketch, *ellipse) == Some(Kind::Ellipse) => {
            format!("the major and minor radii of {}", label(ellipse))
        }
        (ConstraintTool::Radius, [conic]) if sketch_tools::is_conic(sketch, *conic) => {
            format!("the rho of {}", label(conic))
        }
        (ConstraintTool::Radius, [arc]) => format!("the radius of {}", label(arc)),
        (ConstraintTool::Angle, [first, second]) => {
            format!("the angle between {} and {}", label(first), label(second))
        }
        (_, [first, second]) => {
            format!(
                "the distance between {} and {}",
                label(first),
                label(second)
            )
        }
        (tool, _) => format!("a {}", tool.label().to_lowercase()),
    }
}

pub fn prompt(
    sketch: &Sketch,
    picks: &[EntityId],
    pointer: Option<Point2>,
) -> (String, &'static str) {
    let as_placed = placed(sketch, picks, pointer);
    if let Some((_, axis)) = about_axis(sketch, picks)
        && let Some(tool) = as_placed
    {
        return (
            format!(
                "Click across {} for the diameter, on the point's side for the distance: here {}",
                sketch.entity_label(axis),
                words(sketch, tool, picks)
            ),
            AXIS_KEYS,
        );
    }
    if awaits_placement(sketch, picks)
        && let Some(tool) = as_placed
    {
        return (
            format!(
                "Click above or below for the horizontal distance, beside for the vertical one, \
                 elsewhere for the aligned one: here {}",
                words(sketch, tool, picks)
            ),
            PLACING_KEYS,
        );
    }
    match (picks, fitting(sketch, picks)) {
        ([], _) => (PICK_FIRST.to_owned(), FIRST_KEYS),
        ([point], Fit::Waiting) => (
            format!(
                "Click a second point, a line or a circle for its distance from {}",
                sketch.entity_label(*point)
            ),
            POINT_KEYS,
        ),
        (_, Fit::Ready(tool)) => {
            let here = as_placed
                .filter(|placed| *placed != tool)
                .map_or_else(String::new, |placed| {
                    format!(" (empty space here: {})", words(sketch, placed, picks))
                });
            (
                format!(
                    "Click a second item to dimension against it, or press Enter for {}{here}",
                    words(sketch, tool, picks)
                ),
                PICKED_KEYS,
            )
        }
        (_, Fit::Waiting | Fit::Refused(_)) => (PICK_FIRST.to_owned(), FIRST_KEYS),
    }
}

pub fn hover_words(sketch: &Sketch, picks: &[EntityId], hovered: EntityId) -> String {
    let label = sketch.entity_label(hovered);
    if picks.contains(&hovered) {
        return format!("Click to let go of {label}");
    }
    let picked: Vec<EntityId> = picks.iter().copied().chain([hovered]).collect();
    if let Some((point, axis)) = about_axis(sketch, &picked) {
        return format!(
            "Click to pick {label}, then click across {} for the diameter of {}, or on its side \
             for the distance",
            sketch.entity_label(axis),
            sketch.entity_label(point)
        );
    }
    if awaits_placement(sketch, &picked) {
        return format!(
            "Click to pick {label}, then click where {} goes",
            words(sketch, ConstraintTool::Distance, &picked)
        );
    }
    match (picks, fitting(sketch, &picked)) {
        ([], Fit::Ready(tool)) => format!(
            "Click to pick {label}, then press Enter for {} or click a second item",
            words(sketch, tool, &picked)
        ),
        (_, Fit::Ready(tool)) => format!("Click to dimension {}", words(sketch, tool, &picked)),
        (_, Fit::Waiting) => {
            format!("Click to measure from {label}, then click what to measure to")
        }
        (_, Fit::Refused(reason)) => reason.to_owned(),
    }
}

pub fn dimension(
    model: &Model,
    feature: FeatureId,
    picks: &[EntityId],
    pointer: Option<Point2>,
) -> Result<Added, String> {
    let owner = model
        .document()
        .feature(feature)
        .ok_or_else(|| NOT_IN_SKETCH.to_owned())?;
    let definition = owner
        .kind
        .sketch()
        .ok_or_else(|| NOT_IN_SKETCH.to_owned())?;
    let shown = model
        .displayed_sketch(owner)
        .ok_or_else(|| NOT_IN_SKETCH.to_owned())?;
    let tool = match fitting(&shown, picks) {
        Fit::Ready(tool) => placed(&shown, picks, pointer).unwrap_or(tool),
        Fit::Waiting => return Err(prompt(&shown, picks, pointer).0),
        Fit::Refused(reason) => return Err(reason.to_owned()),
    };
    let constraints = tool.candidates_among(definition, &shown, picks, &definition.relations())?;
    let constraints = sketch_tools::in_unit(constraints, model.units());
    Ok(sketch_tools::add_constraints(
        model,
        feature,
        tool,
        constraints,
    ))
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Plane, Point2};

    use super::*;

    #[test]
    fn the_picks_choose_the_dimension_that_fits_them() {
        let mut sketch = Sketch::new(Plane::XY);
        let level = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
        let above = sketch.add_line(Point2::new(0.0, 5.0), Point2::new(20.0, 5.0));
        let slanted = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
        let circle = sketch.add_circle(Point2::new(40.0, 0.0), 5.0);
        let arc = sketch.add_arc(Point2::ZERO, Point2::new(3.0, 0.0), Point2::new(0.0, 3.0));
        let spline = sketch.add_spline(&[Point2::ZERO, Point2::new(1.0, 2.0)]);
        let point = sketch.add_point(Point2::new(3.0, 4.0));

        assert_eq!(fitting(&sketch, &[]), Fit::Waiting);
        assert_eq!(fitting(&sketch, &[point]), Fit::Waiting);
        assert_eq!(
            fitting(&sketch, &[level]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[circle]),
            Fit::Ready(ConstraintTool::Diameter)
        );
        assert_eq!(fitting(&sketch, &[arc]), Fit::Ready(ConstraintTool::Radius));
        assert_eq!(
            fitting(&sketch, &[level, slanted]),
            Fit::Ready(ConstraintTool::Angle)
        );
        assert_eq!(
            fitting(&sketch, &[level, above]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[point, EntityId::HORIZONTAL_AXIS]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[EntityId::ORIGIN, circle]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(fitting(&sketch, &[spline]), Fit::Waiting);
        assert_eq!(
            fitting(&sketch, &[spline, point]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[level, spline]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[spline, circle]),
            Fit::Ready(ConstraintTool::Distance)
        );
        let other_spline = sketch.add_spline(&[Point2::new(5.0, 0.0), Point2::new(6.0, 2.0)]);
        assert_eq!(
            fitting(&sketch, &[spline, other_spline]),
            Fit::Refused(SPLINE_REFUSED)
        );
        let ellipse = sketch.add_ellipse(Point2::new(0.0, 9.0), Point2::new(4.0, 9.0), 2.0);
        let other_ellipse = sketch.add_ellipse(Point2::new(9.0, 9.0), Point2::new(9.0, 12.0), 1.0);
        assert_eq!(
            fitting(&sketch, &[spline, ellipse]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[ellipse, other_ellipse]),
            Fit::Ready(ConstraintTool::Distance)
        );
        assert_eq!(
            fitting(&sketch, &[ellipse, other_ellipse, spline]),
            Fit::Refused(ELLIPSE_REFUSED)
        );
        assert_eq!(
            fitting(&sketch, &[level, arc]),
            Fit::Ready(ConstraintTool::Distance)
        );
        let Some(&Entity::Arc { start, .. }) = sketch.entity(arc) else {
            panic!("expected an arc");
        };
        let leaving = sketch.add_line(Point2::new(3.0, 0.0), Point2::new(3.0, -6.0));
        let Some(&Entity::Line { start: joint, .. }) = sketch.entity(leaving) else {
            panic!("expected a line");
        };
        sketch
            .add_constraint(caditor_sketch::Constraint::Coincident(joint, start))
            .unwrap();
        assert_eq!(
            fitting(&sketch, &[leaving, arc]),
            Fit::Ready(ConstraintTool::Angle)
        );
        assert_eq!(
            fitting(&sketch, &[level, above, slanted]),
            Fit::Refused(PICK_FIRST)
        );
    }

    #[test]
    fn the_words_say_what_a_click_or_enter_would_dimension() {
        let mut sketch = Sketch::new(Plane::XY);
        let level = sketch.add_line(Point2::ZERO, Point2::new(20.0, 0.0));
        let slanted = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
        let point = sketch.add_point(Point2::new(3.0, 4.0));
        let level_label = sketch.entity_label(level);
        let slanted_label = sketch.entity_label(slanted);
        let point_label = sketch.entity_label(point);

        assert_eq!(
            hover_words(&sketch, &[level], slanted),
            format!("Click to dimension the angle between {level_label} and {slanted_label}")
        );
        assert_eq!(
            hover_words(&sketch, &[], level),
            format!(
                "Click to pick {level_label}, then press Enter for the length of {level_label} or \
                 click a second item"
            )
        );
        assert_eq!(
            hover_words(&sketch, &[level], level),
            format!("Click to let go of {level_label}")
        );
        assert_eq!(
            prompt(&sketch, &[point], None).0,
            format!("Click a second point, a line or a circle for its distance from {point_label}")
        );
        assert_eq!(
            prompt(&sketch, &[level], None),
            (
                format!(
                    "Click a second item to dimension against it, or press Enter for the length \
                     of {level_label}"
                ),
                PICKED_KEYS
            )
        );
    }

    #[test]
    fn a_point_and_a_construction_line_wait_for_a_click_across_it_for_the_diameter() {
        let mut sketch = Sketch::new(Plane::XY);
        let axis = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(0.0, 10.0));
        sketch.set_construction(axis, true).unwrap();
        let edge = sketch.add_line(Point2::new(10.0, -10.0), Point2::new(10.0, 10.0));
        let point = sketch.add_point(Point2::new(6.0, 2.0));
        let at = |x: f64, y: f64| Some(Point2::new(x, y));
        let (point_label, axis_label) = (sketch.entity_label(point), sketch.entity_label(axis));

        assert!(awaits_placement(&sketch, &[point, axis]));
        assert!(awaits_placement(&sketch, &[axis, point]));
        assert!(!awaits_placement(&sketch, &[point, edge]));
        assert_eq!(
            placed(&sketch, &[point, axis], at(-4.0, 3.0)),
            Some(ConstraintTool::Diameter)
        );
        assert_eq!(
            placed(&sketch, &[axis, point], at(2.0, -6.0)),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &[point, axis], None),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &[point, edge], at(20.0, 0.0)),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            prompt(&sketch, &[point, axis], at(-4.0, 3.0)),
            (
                format!(
                    "Click across {axis_label} for the diameter, on the point's side for the \
                     distance: here the diameter of {point_label} across {axis_label}"
                ),
                AXIS_KEYS
            )
        );
        assert_eq!(
            hover_words(&sketch, &[point], axis),
            format!(
                "Click to pick {axis_label}, then click across {axis_label} for the diameter of \
                 {point_label}, or on its side for the distance"
            )
        );
    }

    #[test]
    fn a_point_or_circle_off_a_slanted_line_and_two_circles_wait_to_be_placed() {
        let mut sketch = Sketch::new(Plane::XY);
        let point = sketch.add_point(Point2::new(8.0, 3.0));
        let slanted = sketch.add_line(Point2::ZERO, Point2::new(10.0, 10.0));
        let level = sketch.add_line(Point2::new(0.0, -5.0), Point2::new(10.0, -5.0));
        let first = sketch.add_circle(Point2::ZERO, 2.0);
        let second = sketch.add_circle(Point2::new(10.0, 6.0), 1.0);
        let at = |x: f64, y: f64| Some(Point2::new(x, y));

        assert!(awaits_placement(&sketch, &[point, slanted]));
        assert!(awaits_placement(&sketch, &[first, second]));
        assert!(awaits_placement(&sketch, &[second, slanted]));
        assert!(!awaits_placement(&sketch, &[point, level]));
        assert_eq!(
            placed(&sketch, &[point, slanted], at(5.5, 0.0)),
            Some(ConstraintTool::HorizontalDistance)
        );
        assert_eq!(
            placed(&sketch, &[point, slanted], at(12.0, 5.0)),
            Some(ConstraintTool::VerticalDistance)
        );
        assert_eq!(
            placed(&sketch, &[point, slanted], at(20.0, 20.0)),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &[first, second], at(5.0, 10.0)),
            Some(ConstraintTool::HorizontalDistance)
        );
        assert_eq!(
            placed(&sketch, &[first, second], at(15.0, 3.0)),
            Some(ConstraintTool::VerticalDistance)
        );
        assert_eq!(
            placed(&sketch, &[point, level], at(8.0, 0.0)),
            Some(ConstraintTool::Distance)
        );
    }

    #[test]
    fn where_the_pointer_is_chooses_horizontal_vertical_or_aligned_and_an_arc_s_part() {
        let mut sketch = Sketch::new(Plane::XY);
        let first = sketch.add_point(Point2::ZERO);
        let second = sketch.add_point(Point2::new(10.0, 4.0));
        let level = sketch.add_point(Point2::new(10.0, 0.0));
        let slanted = sketch.add_line(Point2::ZERO, Point2::new(6.0, 8.0));
        let arc = sketch.add_arc(Point2::ZERO, Point2::new(5.0, 0.0), Point2::new(0.0, 5.0));
        let points = [first, second];
        let at = |x: f64, y: f64| Some(Point2::new(x, y));

        assert!(awaits_placement(&sketch, &points));
        assert!(!awaits_placement(&sketch, &[first]));
        assert!(!awaits_placement(&sketch, &[first, slanted]));
        assert_eq!(
            placed(&sketch, &points, at(5.0, 9.0)),
            Some(ConstraintTool::HorizontalDistance)
        );
        assert_eq!(
            placed(&sketch, &points, at(-3.0, 2.0)),
            Some(ConstraintTool::VerticalDistance)
        );
        assert_eq!(
            placed(&sketch, &points, at(15.0, 9.0)),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &points, None),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &[first, level], at(5.0, 9.0)),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &[slanted], at(3.0, 20.0)),
            Some(ConstraintTool::HorizontalDistance)
        );
        assert_eq!(
            placed(&sketch, &[arc], at(5.0, 5.0)),
            Some(ConstraintTool::Distance)
        );
        assert_eq!(
            placed(&sketch, &[arc], at(1.0, 1.0)),
            Some(ConstraintTool::Angle)
        );
        assert_eq!(
            placed(&sketch, &[arc], at(-4.0, -4.0)),
            Some(ConstraintTool::Radius)
        );
        assert_eq!(
            words(&sketch, ConstraintTool::Angle, &[arc]),
            format!("the sweep of {}", sketch.entity_label(arc))
        );
        assert_eq!(
            words(&sketch, ConstraintTool::VerticalDistance, &points),
            format!(
                "the vertical distance between {} and {}",
                sketch.entity_label(first),
                sketch.entity_label(second)
            )
        );
    }
}
