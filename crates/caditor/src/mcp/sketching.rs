use std::collections::BTreeSet;

use caditor_document::{Datum, FeatureId, FeatureKind, PrincipalPlane, SketchFeature};
use caditor_expression::Dimension;
use caditor_geometry::Point2;
use caditor_kernel::MAX_SIZE;
use caditor_sketch::{
    Constraint, ConstraintId, Entity, EntityId, MAX_RHO, MIN_RHO, Sketch, SplineKind,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    datum_tools,
    drawing::{arc_ends, continuing},
    editing,
    field::{self, DimensionTarget},
    mcp::{
        references::{self, EntityName, Named},
        session::{Session, Work},
        sketch_edits::amount,
        tools::{Answer, ToolError},
        writing::{self, MOST_ITEMS, NewSketch, Pen, PlaneName, SketchingError, point},
    },
    shapes::{
        ArcSlot, DEFAULT_SIDES, DEGENERATE_LENGTH, MAX_SIDES, MIN_SIDES, Slot, ellipse_through,
        polygon_around_side_middle, polygon_corners, polygon_on_side, tangent_from, through_three,
        toward_on_ellipse,
    },
    sketch_placement::DatumTarget,
    sketch_status::{SketchSummary, up_to_date_result},
    units::Units,
};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SketchToolError {
    #[error(
        "an arc is drawn from a centre (with start and end) or through three points (start, end and through), not both and not neither"
    )]
    ArcMode,
    #[error(
        "the arc has no size: its start must be away from its centre and its end must not lie in the direction of its start"
    )]
    DegenerateArc,
    #[error(
        "the arc's start, end and through points lie in a line, or the circle through them is too large; move one of them"
    )]
    ArcInLine,
    #[error(
        "a slot's ends must be apart and its width a finite number above zero within a kilometre, not {width}"
    )]
    DegenerateSlot { width: String },
    #[error(
        "an arc slot needs its start and end away from the centre, a width narrower than twice the arc's radius, and ends that do not meet"
    )]
    DegenerateArcSlot,
    #[error("a polygon has {MIN_SIDES} to {MAX_SIDES} sides, not {0}")]
    Sides(usize),
    #[error(
        "give a polygon one of corner (a vertex) or side_middle (the middle of its first side), each with center, or from and to (the ends of one side) without center"
    )]
    PolygonMode,
    #[error(
        "a polygon needs its corner or side middle away from its centre, and a side's ends apart"
    )]
    DegeneratePolygon,
    #[error(
        "a {form} spline needs at least {needed} points, and a call holds at most {MOST_ITEMS}"
    )]
    SplinePoints { form: &'static str, needed: usize },
    #[error(
        "an ellipse needs a major end away from its centre and a minor radius that is a finite number above zero within a kilometre, not {0}"
    )]
    DegenerateEllipse(f64),
    #[error(
        "an elliptical arc's start and end must point away from its centre in different directions"
    )]
    EllipticalArcEnds,
    #[error("a conic needs its end away from its start")]
    ConicEnds,
    #[error("a conic's apex must lie off the line through its start and end")]
    ConicApex,
    #[error("a conic's rho is a number from {MIN_RHO} to {MAX_RHO}, not {0}")]
    Rho(f64),
    #[error(
        "{point} in {sketch} does not end a line, arc or spline, so a tangent arc has no direction to leave it in"
    )]
    NoTangentStart { sketch: String, point: EntityName },
    #[error(
        "a tangent arc needs its end away from its start and off the line it continues; draw a line there instead"
    )]
    TangentArcEnd,
    #[error(
        "give a tangent arc either from_point (a point ending a curve) or after_item (the index of an earlier item of this call), not both and not neither"
    )]
    TangentArcStart,
    #[error("after_item {index} is not an earlier item of this call")]
    NoEarlierItem { index: usize },
    #[error("item {index} of this call has no end for a tangent arc to continue from")]
    NoEarlierEnd { index: usize },
    #[error(
        "a frame is a coordinate system; give plane (XY, XZ or YZ) to choose which of its planes the sketch lies on"
    )]
    FrameNeedsPlane,
    #[error("give a datum plane or a coordinate system, not both")]
    DatumAndFrame,
    #[error("{name} is not a datum plane; datum planes are listed by list_features")]
    NotADatumPlane { name: String },
    #[error("{name} is not a coordinate system; coordinate systems are listed by list_features")]
    NotAFrame { name: String },
    #[error("{name} has no position yet: it failed or is waiting; list_features says why")]
    NotPlaced { name: String },
    #[error("{sketch} has no constraint {constraint}; get_sketch lists its constraints")]
    NoConstraint { sketch: String, constraint: u64 },
    #[error("the sketch's own origin and axes cannot be removed")]
    ReferenceGeometry,
    #[error("{description} in {sketch} is not a dimension, so it has no value; give active alone")]
    NotADimension { sketch: String, description: String },
    #[error(
        "say what to change: a value for a dimension, or active to turn the constraint on or off"
    )]
    NothingToEdit,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcShape {
    pub center: Option<[f64; 2]>,
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub through: Option<[f64; 2]>,
    #[serde(default)]
    pub clockwise: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotShape {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcSlotShape {
    pub center: [f64; 2],
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolygonShape {
    pub center: Option<[f64; 2]>,
    pub corner: Option<[f64; 2]>,
    pub side_middle: Option<[f64; 2]>,
    pub from: Option<[f64; 2]>,
    pub to: Option<[f64; 2]>,
    pub sides: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplineForm {
    Fit,
    Control,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplineShape {
    pub points: Vec<[f64; 2]>,
    pub form: Option<SplineForm>,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EllipseShape {
    pub center: [f64; 2],
    pub major_end: [f64; 2],
    pub minor_radius: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EllipticalArcShape {
    pub center: [f64; 2],
    pub major_end: [f64; 2],
    pub minor_radius: f64,
    pub start: [f64; 2],
    pub end: [f64; 2],
    #[serde(default)]
    pub clockwise: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConicShape {
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub apex: [f64; 2],
    pub rho: Option<RhoValue>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TangentArcShape {
    pub from_point: Option<EntityName>,
    pub after_item: Option<usize>,
    pub to: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RhoValue {
    Number(f64),
    Expression(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Removing {
    pub sketch: Named,
    #[serde(default)]
    pub constraints: Vec<u64>,
    #[serde(default)]
    pub entities: Vec<EntityName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintEdit {
    pub sketch: Named,
    pub constraint: u64,
    pub value: Option<String>,
    pub active: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
struct Drawn {
    curve: EntityId,
    start: EntityId,
    end: EntityId,
}

const DEFAULT_RHO: f64 = 0.5;

fn sized(value: f64) -> bool {
    value.is_finite() && value > DEGENERATE_LENGTH && value <= MAX_SIZE
}

impl Pen<'_, '_> {
    fn curve(&mut self, entity: Entity) -> EntityId {
        self.place(entity, self.construction)
    }

    fn guide(&mut self, entity: Entity) -> EntityId {
        self.place(entity, true)
    }

    fn arc_through(&mut self, center: EntityId, start: Point2, end: Point2) -> Drawn {
        let start = self.point(start);
        let end = self.point(end);
        let curve = self.curve(Entity::Arc { center, start, end });
        Drawn { curve, start, end }
    }

    fn free_line(&mut self, start: Point2, end: Point2) -> Drawn {
        let start = self.point(start);
        let end = self.point(end);
        let curve = self.line(start, end);
        Drawn { curve, start, end }
    }

    fn join(&mut self, joints: [(EntityId, EntityId); 4], tangents: [(EntityId, EntityId); 4]) {
        for (point, other) in joints {
            self.constrain(Constraint::Coincident(point, other));
        }
        for (first, second) in tangents {
            self.constrain(Constraint::Tangent(first, second));
        }
    }
}

impl ArcShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let start = point(self.start)?;
        let end = point(self.end)?;
        let counter_clockwise = !self.clockwise;
        match (self.center, self.through) {
            (Some(center), None) => {
                let center = point(center)?;
                let radius = center.distance(start);
                let on_circle = (end - center)
                    .try_normalize()
                    .filter(|_| sized(radius))
                    .map(|direction| center + direction * radius)
                    .filter(|on_circle| on_circle.distance(start) > DEGENERATE_LENGTH)
                    .ok_or(SketchToolError::DegenerateArc)?;
                let center = pen.point(center);
                let start = pen.point(start);
                let end = pen.point(on_circle);
                Ok(arc_entry(
                    pen,
                    center,
                    arc_ends(counter_clockwise, start, end),
                ))
            }
            (None, Some(through)) => {
                let through = point(through)?;
                let circular =
                    through_three(start, end, through).ok_or(SketchToolError::ArcInLine)?;
                let center = pen.point(circular.center);
                let start = pen.point(start);
                let end = pen.point(end);
                Ok(arc_entry(
                    pen,
                    center,
                    arc_ends(circular.counter_clockwise, start, end),
                ))
            }
            _ => Err(SketchToolError::ArcMode.into()),
        }
    }
}

fn arc_entry(pen: &mut Pen<'_, '_>, center: EntityId, (start, end): (EntityId, EntityId)) -> Value {
    let arc = pen.curve(Entity::Arc { center, start, end });
    json!({
        "kind": "arc",
        "id": arc.raw(),
        "center": center.raw(),
        "start": start.raw(),
        "end": end.raw()
    })
}

impl SlotShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let first = point(self.from)?;
        let second = point(self.to)?;
        let degenerate = || SketchToolError::DegenerateSlot {
            width: self.width.to_string(),
        };
        if !sized(self.width) {
            return Err(degenerate().into());
        }
        let along = (second - first).try_normalize().ok_or_else(degenerate)?;
        let width_point = first + along.perp() * (self.width / 2.0);
        let slot = Slot::new(first, second, width_point).ok_or_else(degenerate)?;

        let first_center = pen.point(first);
        let second_center = pen.point(second);
        let [a, b, c, d] = slot.corners();
        let second_end = pen.arc_through(second_center, a, b);
        let first_end = pen.arc_through(first_center, c, d);
        let top = pen.free_line(b, c);
        let bottom = pen.free_line(d, a);
        pen.join(
            [
                (top.start, second_end.end),
                (top.end, first_end.start),
                (bottom.start, first_end.end),
                (bottom.end, second_end.start),
            ],
            [
                (top.curve, second_end.curve),
                (top.curve, first_end.curve),
                (bottom.curve, first_end.curve),
                (bottom.curve, second_end.curve),
            ],
        );
        pen.constrain(Constraint::Equal(first_end.curve, second_end.curve));
        if first.y == second.y {
            pen.constrain(Constraint::Horizontal(top.curve));
        } else if first.x == second.x {
            pen.constrain(Constraint::Vertical(top.curve));
        }
        Ok(json!({
            "kind": "slot",
            "centers": writing::ids(&[first_center, second_center]),
            "arcs": writing::ids(&[first_end.curve, second_end.curve]),
            "lines": writing::ids(&[top.curve, bottom.curve])
        }))
    }
}

impl ArcSlotShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let center = point(self.center)?;
        let first = point(self.from)?;
        let last = point(self.to)?;
        let radius = center.distance(first);
        if !sized(self.width) || !sized(radius) {
            return Err(SketchToolError::DegenerateArcSlot.into());
        }
        let outward = (first - center)
            .try_normalize()
            .ok_or(SketchToolError::DegenerateArcSlot)?;
        let width_point = center + outward * (radius + self.width / 2.0);
        let slot = ArcSlot::new(center, [first, last], width_point)
            .ok_or(SketchToolError::DegenerateArcSlot)?;

        let centre = pen.point(center);
        let [first_position, last_position] = slot.ends;
        let first_center = pen.point(first_position);
        let last_center = pen.point(last_position);
        let [outer_first, outer_last, inner_first, inner_last] = slot.corners();
        let outer = pen.arc_through(centre, outer_first, outer_last);
        let inner = pen.arc_through(centre, inner_first, inner_last);
        let first_end = pen.arc_through(first_center, inner_first, outer_first);
        let last_end = pen.arc_through(last_center, outer_last, inner_last);
        pen.join(
            [
                (first_end.start, inner.start),
                (first_end.end, outer.start),
                (last_end.start, outer.end),
                (last_end.end, inner.end),
            ],
            [
                (first_end.curve, outer.curve),
                (first_end.curve, inner.curve),
                (last_end.curve, outer.curve),
                (last_end.curve, inner.curve),
            ],
        );
        Ok(json!({
            "kind": "arc_slot",
            "center": centre.raw(),
            "end_centers": writing::ids(&[first_center, last_center]),
            "arcs": writing::ids(&[outer.curve, inner.curve]),
            "end_arcs": writing::ids(&[first_end.curve, last_end.curve])
        }))
    }
}

impl PolygonShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let sides = self.sides.unwrap_or(DEFAULT_SIDES);
        if !(MIN_SIDES..=MAX_SIDES).contains(&sides) {
            return Err(SketchToolError::Sides(sides).into());
        }
        if self.from.is_some() != self.to.is_some() {
            return Err(SketchToolError::PolygonMode.into());
        }
        let (center, corners, middle) = match (
            self.center,
            self.corner,
            self.side_middle,
            self.from.zip(self.to),
        ) {
            (Some(center), Some(corner), None, None) => {
                let center = point(center)?;
                let corner = point(corner)?;
                (center, polygon_corners(center, corner, sides), None)
            }
            (Some(center), None, Some(middle), None) => {
                let center = point(center)?;
                let middle = point(middle)?;
                (
                    center,
                    polygon_around_side_middle(center, middle, sides),
                    Some(middle),
                )
            }
            (None, None, None, Some((from, to))) => {
                let (center, corners) = polygon_on_side(point(from)?, point(to)?, sides)
                    .ok_or(SketchToolError::DegeneratePolygon)?;
                (center, corners, None)
            }
            _ => return Err(SketchToolError::PolygonMode.into()),
        };
        let radius = corners
            .first()
            .map_or(0.0, |corner| center.distance(*corner));
        if !sized(radius) {
            return Err(SketchToolError::DegeneratePolygon.into());
        }
        for corner in &corners {
            writing::coordinate(corner.x)?;
            writing::coordinate(corner.y)?;
        }

        let centre = pen.point(center);
        let circle = pen.guide(Entity::Circle {
            center: centre,
            radius,
        });
        let mut drawn = Vec::with_capacity(corners.len());
        for (index, corner) in corners.iter().enumerate() {
            let next = corners
                .get((index + 1) % corners.len())
                .copied()
                .unwrap_or(*corner);
            drawn.push(pen.free_line(*corner, next));
        }
        for (index, side) in drawn.iter().enumerate() {
            if let Some(next) = drawn.get((index + 1) % drawn.len()) {
                pen.constrain(Constraint::Coincident(side.end, next.start));
            }
        }
        let first = drawn.first().map(|side| side.curve);
        for side in &drawn {
            pen.constrain(Constraint::Coincident(side.start, circle));
            if let Some(first) = first
                && side.curve != first
            {
                pen.constrain(Constraint::Equal(first, side.curve));
            }
        }
        let inscribed = match (first, middle) {
            (Some(first), Some(middle)) => {
                let inscribed = pen.guide(Entity::Circle {
                    center: centre,
                    radius: center.distance(middle),
                });
                pen.constrain(Constraint::Tangent(first, inscribed));
                Some(inscribed)
            }
            _ => None,
        };
        let starts: Vec<EntityId> = drawn.iter().map(|side| side.start).collect();
        let lines: Vec<EntityId> = drawn.iter().map(|side| side.curve).collect();
        let mut entry = json!({
            "kind": "polygon",
            "sides": sides,
            "center": centre.raw(),
            "circle": circle.raw(),
            "corners": writing::ids(&starts),
            "lines": writing::ids(&lines)
        });
        if let (Some(inscribed), Some(entry)) = (inscribed, entry.as_object_mut()) {
            entry.insert("inscribed_circle".to_owned(), json!(inscribed.raw()));
        }
        Ok(entry)
    }
}

impl SplineShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let (form, kind) = match self.form.unwrap_or(SplineForm::Fit) {
            SplineForm::Fit => ("fit", SplineKind::fit(self.closed)),
            SplineForm::Control => (
                "control",
                SplineKind::Control {
                    closed: self.closed,
                },
            ),
        };
        if self.points.len() < kind.fewest_points() || self.points.len() > MOST_ITEMS {
            return Err(SketchToolError::SplinePoints {
                form,
                needed: kind.fewest_points(),
            }
            .into());
        }
        let placed: Vec<Point2> = self
            .points
            .iter()
            .map(|at| point(*at))
            .collect::<Result<_, _>>()?;
        let points: Vec<EntityId> = placed.iter().map(|at| pen.point(*at)).collect();
        let spline = pen.curve(Entity::Spline {
            points: points.clone(),
            kind,
        });
        Ok(json!({
            "kind": "spline",
            "form": form,
            "id": spline.raw(),
            "points": writing::ids(&points)
        }))
    }
}

impl EllipseShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let center = point(self.center)?;
        let major_end = point(self.major_end)?;
        if !sized(self.minor_radius) || !sized(center.distance(major_end)) {
            return Err(SketchToolError::DegenerateEllipse(self.minor_radius).into());
        }
        let centre = pen.point(center);
        let major = pen.point(major_end);
        let ellipse = pen.curve(Entity::Ellipse {
            center: centre,
            major,
            minor_radius: self.minor_radius,
        });
        Ok(json!({
            "kind": "ellipse",
            "id": ellipse.raw(),
            "center": centre.raw(),
            "major": major.raw()
        }))
    }
}

impl EllipticalArcShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let center = point(self.center)?;
        let major_end = point(self.major_end)?;
        let degenerate = SketchToolError::DegenerateEllipse(self.minor_radius);
        if !sized(self.minor_radius) || !sized(center.distance(major_end)) {
            return Err(degenerate.into());
        }
        let across = (major_end - center)
            .try_normalize()
            .map(|along| center + along.perp() * self.minor_radius)
            .ok_or_else(|| degenerate.clone())?;
        let ellipse = ellipse_through(center, major_end, across).ok_or(degenerate)?;
        let start = toward_on_ellipse(&ellipse, point(self.start)?)
            .ok_or(SketchToolError::EllipticalArcEnds)?;
        let end = toward_on_ellipse(&ellipse, point(self.end)?)
            .filter(|end| end.distance(start) >= DEGENERATE_LENGTH)
            .ok_or(SketchToolError::EllipticalArcEnds)?;

        let (first, last) = arc_ends(!self.clockwise, start, end);
        let centre = pen.point(center);
        let major = pen.point(major_end);
        let start = pen.point(first);
        let end = pen.point(last);
        let arc = pen.curve(Entity::EllipticalArc {
            center: centre,
            major,
            minor_radius: self.minor_radius,
            start,
            end,
        });
        Ok(json!({
            "kind": "elliptical_arc",
            "id": arc.raw(),
            "center": centre.raw(),
            "major": major.raw(),
            "start": start.raw(),
            "end": end.raw()
        }))
    }
}

impl ConicShape {
    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let start = point(self.start)?;
        let end = point(self.end)?;
        let apex = point(self.apex)?;
        let (rho, held_by) = match &self.rho {
            None => (DEFAULT_RHO, None),
            Some(RhoValue::Number(rho)) => (*rho, None),
            Some(RhoValue::Expression(text)) => {
                let parsed = amount(pen.session, "rho", text, Dimension::NONE)?;
                let held =
                    (!parsed.expression.parameters().is_empty()).then_some(parsed.expression);
                (parsed.value, held)
            }
        };
        let chord = end - start;
        if !(MIN_RHO..=MAX_RHO).contains(&rho) {
            return Err(SketchToolError::Rho(rho).into());
        }
        if chord.length() < DEGENERATE_LENGTH {
            return Err(SketchToolError::ConicEnds.into());
        }
        if chord.perp_dot(apex - start).abs() <= DEGENERATE_LENGTH * chord.length().max(1.0) {
            return Err(SketchToolError::ConicApex.into());
        }

        let points = vec![pen.point(start), pen.point(apex), pen.point(end)];
        let conic = pen.curve(Entity::Spline {
            points: points.clone(),
            kind: SplineKind::Conic { rho },
        });
        if let Some(value) = held_by {
            pen.constrain(Constraint::Rho { conic, value });
        }
        Ok(json!({
            "kind": "conic",
            "id": conic.raw(),
            "points": writing::ids(&points),
            "rho": rho
        }))
    }
}

impl TangentArcShape {
    fn start(&self, pen: &Pen<'_, '_>) -> Result<(EntityId, Sketch), ToolError> {
        match (&self.from_point, self.after_item) {
            (Some(named), None) => Ok((
                references::entity(pen.sketch_name, pen.shown, named)?,
                pen.shown.clone(),
            )),
            (None, Some(index)) => {
                let earlier = pen
                    .results
                    .get(index)
                    .ok_or(SketchToolError::NoEarlierItem { index })?;
                let end = earlier
                    .get("end")
                    .or_else(|| {
                        earlier
                            .get("points")
                            .and_then(|points| points.as_array()?.last())
                    })
                    .and_then(Value::as_u64)
                    .ok_or(SketchToolError::NoEarlierEnd { index })?;
                Ok((EntityId::from_raw(end), pen.drawn_so_far()))
            }
            _ => Err(SketchToolError::TangentArcStart.into()),
        }
    }

    pub(super) fn draw(&self, pen: &mut Pen<'_, '_>) -> Result<Value, ToolError> {
        let (from, known) = self.start(pen)?;
        let no_start = || SketchToolError::NoTangentStart {
            sketch: pen.sketch_name.to_owned(),
            point: EntityName::Id(from.raw()),
        };
        let tangent = continuing(&known, from).ok_or_else(no_start)?;
        let start = known.point(from).ok_or_else(no_start)?;
        let end = point(self.to)?;
        if start.distance(end) < DEGENERATE_LENGTH {
            return Err(SketchToolError::TangentArcEnd.into());
        }
        let circular =
            tangent_from(start, tangent.direction, end).ok_or(SketchToolError::TangentArcEnd)?;

        let center = pen.point(circular.center);
        let first = pen.point(start);
        let last = pen.point(end);
        let (arc_start, arc_end) = arc_ends(circular.counter_clockwise, first, last);
        let arc = pen.curve(Entity::Arc {
            center,
            start: arc_start,
            end: arc_end,
        });
        pen.constrain(Constraint::Coincident(first, from));
        pen.constrain(Constraint::Tangent(tangent.curve, arc));
        Ok(json!({
            "kind": "tangent_arc",
            "id": arc.raw(),
            "center": center.raw(),
            "start": first.raw(),
            "end": last.raw(),
            "tangent_to": tangent.curve.raw()
        }))
    }
}

fn principal(name: PlaneName) -> PrincipalPlane {
    match name {
        PlaneName::Xy => PrincipalPlane::Xy,
        PlaneName::Xz => PrincipalPlane::Xz,
        PlaneName::Yz => PrincipalPlane::Yz,
    }
}

pub fn add_sketch_on_datum(
    session: &mut Session,
    work: &Work<'_>,
    asked: NewSketch,
) -> Result<Answer, ToolError> {
    if asked.body.is_some() || asked.face.is_some() {
        return Err(SketchingError::BothPlaces.into());
    }
    session.evaluate(work)?;
    let document = session.document();
    let target = match (&asked.datum, &asked.frame, asked.plane) {
        (Some(_), Some(_), _) => return Err(SketchToolError::DatumAndFrame.into()),
        (Some(_), None, Some(_)) => return Err(SketchingError::BothPlaces.into()),
        (Some(named), None, None) => {
            let datum = references::feature(document, named)?;
            if !datum_tools::is_plane(document, datum.id()) {
                return Err(SketchToolError::NotADatumPlane {
                    name: datum.name.clone(),
                }
                .into());
            }
            DatumTarget::Plane(datum.id())
        }
        (None, Some(named), plane) => {
            let frame = references::feature(document, named)?;
            if !frame.kind.datum().is_some_and(Datum::is_frame) {
                return Err(SketchToolError::NotAFrame {
                    name: frame.name.clone(),
                }
                .into());
            }
            let plane = plane.ok_or(SketchToolError::FrameNeedsPlane)?;
            DatumTarget::Frame {
                frame: frame.id(),
                plane: principal(plane),
            }
        }
        (None, None, _) => return Err(SketchingError::NoPlace.into()),
    };
    let plane = target
        .plane(session.evaluation())
        .ok_or_else(|| SketchToolError::NotPlaced {
            name: target.name(document),
        })?;
    let name = asked
        .name
        .unwrap_or_else(|| editing::next_sketch_name(document));
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Sketch(SketchFeature {
            attachment: Some(target.attachment()),
            ..SketchFeature::from(Sketch::new(plane))
        }),
    );
    writing::sketch_added(session, work, transaction.finish(), feature)
}

fn entity_ids(sketch: &Sketch) -> BTreeSet<u64> {
    sketch.entities().map(|(id, _)| id.raw()).collect()
}

fn constraint_ids(sketch: &Sketch) -> BTreeSet<u64> {
    sketch.constraints().map(|(id, _)| id.raw()).collect()
}

fn sketch_now(session: &Session, sketch: FeatureId) -> Option<&Sketch> {
    session
        .document()
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
}

fn gone(before: &BTreeSet<u64>, after: &BTreeSet<u64>) -> Value {
    Value::Array(before.difference(after).map(|id| json!(id)).collect())
}

pub fn remove(
    session: &mut Session,
    work: &Work<'_>,
    asked: Removing,
) -> Result<Answer, ToolError> {
    if asked.constraints.is_empty() && asked.entities.is_empty() {
        return Err(SketchingError::Nothing.into());
    }
    if asked.constraints.len() + asked.entities.len() > MOST_ITEMS {
        return Err(SketchingError::TooMany.into());
    }
    session.evaluate(work)?;
    let document = session.document();
    let (feature, definition) = references::sketch(document, &asked.sketch)?;
    let sketch = feature.id();
    let constraints: Vec<ConstraintId> = asked
        .constraints
        .iter()
        .map(|raw| existing_constraint(&feature.name, definition, *raw))
        .collect::<Result<_, _>>()?;
    let entities: Vec<EntityId> = asked
        .entities
        .iter()
        .map(|named| {
            let id = references::entity(&feature.name, definition, named)?;
            if id.is_reference() {
                Err(ToolError::from(SketchToolError::ReferenceGeometry))
            } else {
                Ok(id)
            }
        })
        .collect::<Result<_, _>>()?;
    let entities_before = entity_ids(definition);
    let constraints_before = constraint_ids(definition);
    let label = if entities.is_empty() {
        format!("Remove constraints from {}", feature.name)
    } else {
        format!("Remove from {}", feature.name)
    };
    let mut transaction = document.transaction(label);
    writing::settled(&mut transaction, session.evaluation(), sketch);
    transaction.remove_sketch_items(sketch, entities, constraints);
    let transaction = transaction.finish();
    let answer = writing::apply(session, work, transaction, Map::new())?;
    let mut data = answer.data;
    if let Some(remaining) = sketch_now(session, sketch) {
        data.insert(
            "removed_constraints".to_owned(),
            gone(&constraints_before, &constraint_ids(remaining)),
        );
        data.insert(
            "removed_entities".to_owned(),
            gone(&entities_before, &entity_ids(remaining)),
        );
    }
    data.insert(
        "sketch_status".to_owned(),
        json!(SketchSummary::of(session.evaluation(), sketch).status_text()),
    );
    Ok(Answer::of(data))
}

fn existing_constraint(
    sketch_name: &str,
    sketch: &Sketch,
    raw: u64,
) -> Result<ConstraintId, ToolError> {
    let id = ConstraintId::from_raw(raw);
    if sketch.constraint(id).is_some() {
        Ok(id)
    } else {
        Err(SketchToolError::NoConstraint {
            sketch: sketch_name.to_owned(),
            constraint: raw,
        }
        .into())
    }
}

pub fn edit_constraint(
    session: &mut Session,
    work: &Work<'_>,
    asked: ConstraintEdit,
) -> Result<Answer, ToolError> {
    if asked.value.is_none() && asked.active.is_none() {
        return Err(SketchToolError::NothingToEdit.into());
    }
    session.evaluate(work)?;
    let document = session.document();
    let (feature, definition) = references::sketch(document, &asked.sketch)?;
    let sketch = feature.id();
    let id = existing_constraint(&feature.name, definition, asked.constraint)?;
    let description = definition.describe_constraint(id);
    let mut transaction = document.transaction(format!("Edit {description} in {}", feature.name));
    writing::settled(&mut transaction, session.evaluation(), sketch);
    if let Some(text) = &asked.value {
        let is_dimension = definition
            .constraint(id)
            .is_some_and(|constraint| constraint.dimension().is_some());
        if !is_dimension {
            return Err(SketchToolError::NotADimension {
                sketch: feature.name.clone(),
                description,
            }
            .into());
        }
        let target = DimensionTarget {
            feature: sketch,
            constraint: id,
        };
        let changed = field::dimension_transaction(
            document,
            &session.parameters(),
            target,
            text,
            Units::default(),
        )
        .map_err(|reason| ToolError::Value {
            field: "value",
            text: text.clone(),
            reason: reason.into(),
        })?;
        for edit in changed.edits() {
            transaction.edit(edit.clone());
        }
    }
    if let Some(active) = asked.active
        && definition.is_active(id) != active
    {
        transaction.set_sketch_constraint_active(sketch, id, active);
    }
    let transaction = transaction.finish();
    let mut details = Map::new();
    details.insert("constraint".to_owned(), json!(id.raw()));
    let answer = writing::apply(session, work, transaction, details)?;
    let mut data = answer.data;
    if let Some(edited) = sketch_now(session, sketch) {
        data.insert(
            "description".to_owned(),
            json!(edited.describe_constraint(id)),
        );
        data.insert("active".to_owned(), json!(edited.is_active(id)));
        if let Some(expression) = edited
            .constraint(id)
            .and_then(|constraint| constraint.dimension())
        {
            data.insert(
                "expression".to_owned(),
                json!(session.document().expression_text(expression)),
            );
        }
    }
    if let Some(value) = up_to_date_result(session.evaluation(), sketch)
        .and_then(|result| result.solution.dimension(id))
    {
        data.insert("value".to_owned(), json!(value));
    }
    data.insert(
        "sketch_status".to_owned(),
        json!(SketchSummary::of(session.evaluation(), sketch).status_text()),
    );
    Ok(Answer::of(data))
}
