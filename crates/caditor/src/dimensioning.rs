use caditor_document::FeatureId;
use caditor_sketch::{Entity, EntityId, Reference, Sketch};

use crate::{
    model::Model,
    sketch_tools::{self, Added, ConstraintTool},
};

pub const PICK_FIRST: &str = "Click a line, circle, arc or point to dimension it";
pub const FIRST_KEYS: &str = "Esc: back to Select";
pub const PICKED_KEYS: &str = "Enter or click empty space: dimension it   Esc: start again";
pub const POINT_KEYS: &str = "Esc: start again";
const SPLINE_REFUSED: &str =
    "A spline takes no dimension; dimension the points or lines that shape it instead";
const NOT_IN_SKETCH: &str = "That is not part of the sketch being edited";
const PARALLEL_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Point,
    Line,
    Circle,
    Arc,
    Spline,
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
    if kinds.contains(&Kind::Spline) {
        return Fit::Refused(SPLINE_REFUSED);
    }
    match (kinds.as_slice(), picks) {
        ([] | [Kind::Point], _) => Fit::Waiting,
        ([Kind::Line], _) => Fit::Ready(ConstraintTool::Distance),
        ([Kind::Circle], _) => Fit::Ready(ConstraintTool::Diameter),
        ([Kind::Arc], _) => Fit::Ready(ConstraintTool::Radius),
        ([Kind::Line, Kind::Line], &[first, second]) if !parallel(sketch, first, second) => {
            Fit::Ready(ConstraintTool::Angle)
        }
        ([_, _], _) => Fit::Ready(ConstraintTool::Distance),
        _ => Fit::Refused(PICK_FIRST),
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
    match (tool, picks) {
        (ConstraintTool::Distance, [line]) => format!("the length of {}", label(line)),
        (ConstraintTool::Diameter, [circle]) => format!("the diameter of {}", label(circle)),
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

pub fn prompt(sketch: &Sketch, picks: &[EntityId]) -> (String, &'static str) {
    match (picks, fitting(sketch, picks)) {
        ([], _) => (PICK_FIRST.to_owned(), FIRST_KEYS),
        ([point], Fit::Waiting) => (
            format!(
                "Click a second point, a line or a circle for its distance from {}",
                sketch.entity_label(*point)
            ),
            POINT_KEYS,
        ),
        (_, Fit::Ready(tool)) => (
            format!(
                "Click a second item to dimension against it, or press Enter for {}",
                words(sketch, tool, picks)
            ),
            PICKED_KEYS,
        ),
        (_, Fit::Waiting | Fit::Refused(_)) => (PICK_FIRST.to_owned(), FIRST_KEYS),
    }
}

pub fn hover_words(sketch: &Sketch, picks: &[EntityId], hovered: EntityId) -> String {
    let label = sketch.entity_label(hovered);
    if picks.contains(&hovered) {
        return format!("Click to let go of {label}");
    }
    let picked: Vec<EntityId> = picks.iter().copied().chain([hovered]).collect();
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

pub fn dimension(model: &Model, feature: FeatureId, picks: &[EntityId]) -> Result<Added, String> {
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
        Fit::Ready(tool) => tool,
        Fit::Waiting => return Err(prompt(&shown, picks).0),
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
        assert_eq!(fitting(&sketch, &[spline]), Fit::Refused(SPLINE_REFUSED));
        assert_eq!(
            fitting(&sketch, &[level, spline]),
            Fit::Refused(SPLINE_REFUSED)
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
            prompt(&sketch, &[point]).0,
            format!("Click a second point, a line or a circle for its distance from {point_label}")
        );
        assert_eq!(
            prompt(&sketch, &[level]),
            (
                format!(
                    "Click a second item to dimension against it, or press Enter for the length \
                     of {level_label}"
                ),
                PICKED_KEYS
            )
        );
    }
}
