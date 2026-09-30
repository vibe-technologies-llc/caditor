use std::f64::consts::{PI, TAU};

use caditor_document::{FeatureId, Transaction, TransactionBuilder};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, BSpline, Constraint, Entity, EntityId, Sketch};

use crate::{
    editing::{self, ActiveSketch, Tool},
    model::Model,
    shapes::{self, Circular, DEGENERATE_LENGTH, MAX_SIDES, MIN_SIDES, Slot},
    sketch_tools,
    snap::{self, Accept, Pointer, Screen, Target},
};

const ALIGN_ANGLE_DEGREES: f64 = 3.0;
const ALIGN_TOLERANCE: f64 = 6.0;
const MIN_ALIGN_LENGTH: f64 = 12.0;
const TYPED_TOLERANCE: f64 = 1e-6;
const PREVIEW_SEGMENT_ANGLE: f64 = PI / 60.0;
const BACK_TO_SELECT: &str = "Esc: back to Select";

const TOO_FEW_SIDES: &str = "A polygon needs at least three sides";
const TOO_MANY_SIDES: &str = "A polygon has at most 64 sides";
const NOT_A_POLYGON: &str = "Choose the Polygon tool first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Line,
    Rectangle,
    Circle,
    ArcRadius,
    ArcSweep,
    ArcInLine,
    TangentStart,
    TangentStraight,
    SlotLength,
    SlotWidth,
    PolygonSize,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Line => "A line needs its end away from its start",
            Self::Rectangle => "A rectangle needs its corners apart in both directions",
            Self::Circle => "A circle needs its rim away from its centre",
            Self::ArcRadius => "An arc needs its start away from its centre",
            Self::ArcSweep => "An arc needs its end away from its start",
            Self::ArcInLine => {
                "An arc through three points needs the third point off the line through the \
                 other two"
            }
            Self::TangentStart => "A tangent arc starts at the end of a line, arc or spline",
            Self::TangentStraight => {
                "A tangent arc needs its end off the line it continues along; draw a line there \
                 instead"
            }
            Self::SlotLength => "A slot needs the centres of its two ends apart",
            Self::SlotWidth => {
                "A slot needs a width: click away from the line through the centres of its ends"
            }
            Self::PolygonSize => "A polygon needs its corner away from its centre",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tangent {
    curve: EntityId,
    direction: Vector2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides(usize);

impl Default for Sides {
    fn default() -> Self {
        Self(shapes::DEFAULT_SIDES)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Snap {
    Free,
    Target(Target),
    Aligned(Direction),
}

impl Snap {
    fn entity(self) -> Option<EntityId> {
        match self {
            Self::Target(target) => target.entity(),
            Self::Free | Self::Aligned(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: Point2,
    pub snap: Snap,
}

impl Placement {
    fn free(position: Point2) -> Self {
        Self {
            position,
            snap: Snap::Free,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sweep {
    center: Point2,
    start_angle: f64,
    last_angle: f64,
    turned: f64,
    reversed: bool,
}

impl Sweep {
    pub fn new(center: Point2, start: Point2) -> Self {
        let start_angle = angle_of(start - center);
        Self {
            center,
            start_angle,
            last_angle: start_angle,
            turned: 0.0,
            reversed: false,
        }
    }

    pub fn follow(&mut self, point: Point2) {
        let offset = point - self.center;
        if offset.length_squared() == 0.0 {
            return;
        }
        let angle = angle_of(offset);
        self.turned += shortest_turn(self.last_angle, angle);
        self.last_angle = angle;
    }

    pub fn aim(&mut self, point: Point2) {
        let offset = point - self.center;
        if offset.length_squared() == 0.0 {
            return;
        }
        let angle = angle_of(offset);
        self.turned = shortest_turn(self.start_angle, angle);
        self.last_angle = angle;
    }

    pub fn reverse(&mut self) {
        self.reversed = !self.reversed;
    }

    pub fn counter_clockwise(&self) -> bool {
        (self.turned >= 0.0) != self.reversed
    }
}

fn shortest_turn(from: f64, to: f64) -> f64 {
    let turn = (to - from + PI).rem_euclid(TAU) - PI;
    if turn <= -PI { PI } else { turn }
}

fn point_target(snap: Snap) -> Option<EntityId> {
    match snap {
        Snap::Target(Target::Point(point)) => Some(point),
        Snap::Free | Snap::Aligned(_) | Snap::Target(_) => None,
    }
}

fn angle_of(direction: Vector2) -> f64 {
    direction.y.atan2(direction.x)
}

pub fn arc_ends<T>(counter_clockwise: bool, start: T, end: T) -> (T, T) {
    if counter_clockwise {
        (start, end)
    } else {
        (end, start)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preview {
    pub curves: Vec<Vec<Point2>>,
    pub points: Vec<Point2>,
    pub snap: Option<Point2>,
    pub construction: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub keys: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Drawing {
    context: Option<(FeatureId, Tool)>,
    construction: bool,
    placed: Vec<Placement>,
    hover: Option<Placement>,
    sweep: Option<Sweep>,
    tangent: Option<Tangent>,
    chain_start: Vec<EntityId>,
    sides: Sides,
}

impl Drawing {
    pub fn is_active(&self) -> bool {
        self.context.is_some()
    }

    pub fn in_progress(&self) -> bool {
        !self.placed.is_empty()
    }

    pub fn snap_entity(&self) -> Option<EntityId> {
        self.hover?.snap.entity()
    }

    pub fn sync(&mut self, active: Option<ActiveSketch>, sketch: Option<&Sketch>) {
        let context = active
            .filter(|active| active.tool.draws())
            .map(|active| (active.feature, active.tool));
        if context != self.context {
            *self = Self {
                context,
                sides: self.sides,
                ..Self::default()
            };
        }
        self.construction = active.is_some_and(|active| active.construction);
        let lost_anchor = sketch.is_some_and(|sketch| {
            let lost_point = self.placed.iter().any(|placement| {
                placement
                    .snap
                    .entity()
                    .is_some_and(|entity| !sketch.contains(entity))
            });
            let lost_curve = !self.placed.is_empty()
                && self
                    .tangent
                    .is_some_and(|tangent| !sketch.contains(tangent.curve));
            lost_point || lost_curve
        });
        if lost_anchor {
            self.cancel();
        }
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        let Some((_, tool)) = self.context else {
            self.hover = None;
            return;
        };
        self.hover = pointer.map(|pointer| self.place(tool, sketch, screen, pointer));
        if let (Some(sweep), Some(hover)) = (&mut self.sweep, self.hover) {
            sweep.follow(hover.position);
        }
        self.find_tangent(tool, sketch);
    }

    fn find_tangent(&mut self, tool: Tool, sketch: &Sketch) {
        if tool == Tool::TangentArc && self.placed.is_empty() {
            self.tangent = point_target(self.hover.map_or(Snap::Free, |hover| hover.snap))
                .and_then(|point| continuing(sketch, point));
        }
    }

    pub fn last_placed(&self) -> Option<Point2> {
        self.placed.last().map(|placement| placement.position)
    }

    pub fn pointer_position(&self) -> Option<Point2> {
        self.hover.map(|hover| hover.position)
    }

    pub fn type_point(&mut self, sketch: &Sketch, position: Point2) {
        let Some((_, tool)) = self.context else {
            return;
        };
        let same = |candidate: Point2| candidate.distance(position) <= TYPED_TOLERANCE;
        let pending = self
            .pending(tool)
            .into_iter()
            .find(|(_, candidate)| same(*candidate))
            .map(|(index, candidate)| (candidate, Target::Pending(index)));
        let existing = snap::points(sketch)
            .into_iter()
            .find(|candidate| same(candidate.position))
            .map(|candidate| (candidate.position, candidate.target));
        let placement = match pending.or(existing) {
            Some((position, target)) => Placement {
                position,
                snap: Snap::Target(target),
            },
            None => Placement::free(position),
        };
        self.hover = Some(placement);
        if let Some(sweep) = &mut self.sweep {
            sweep.aim(placement.position);
        }
        self.find_tangent(tool, sketch);
    }

    pub fn reversible(&self) -> Result<(), &'static str> {
        self.sweep
            .map(|_| ())
            .ok_or("Place an arc's centre and start first; then its end can go either way round")
    }

    pub fn reverse_arc(&mut self) {
        if let Some(sweep) = &mut self.sweep {
            sweep.reverse();
        }
    }

    pub fn more_sides(&self) -> Result<(), &'static str> {
        self.polygon_sides()?;
        if self.sides.0 >= MAX_SIDES {
            return Err(TOO_MANY_SIDES);
        }
        Ok(())
    }

    pub fn fewer_sides(&self) -> Result<(), &'static str> {
        self.polygon_sides()?;
        if self.sides.0 <= MIN_SIDES {
            return Err(TOO_FEW_SIDES);
        }
        Ok(())
    }

    pub fn add_sides(&mut self, change: isize) {
        self.sides = Sides(
            self.sides
                .0
                .saturating_add_signed(change)
                .clamp(MIN_SIDES, MAX_SIDES),
        );
    }

    fn polygon_sides(&self) -> Result<(), &'static str> {
        match self.context {
            Some((_, Tool::Polygon)) => Ok(()),
            _ => Err(NOT_A_POLYGON),
        }
    }

    pub fn leave(&mut self) {
        self.hover = None;
    }

    pub fn cancel(&mut self) {
        self.placed.clear();
        self.sweep = None;
        self.tangent = None;
        self.chain_start.clear();
    }

    pub fn remove_last(&mut self) {
        self.placed.pop();
        if self.placed.len() < 2 {
            self.sweep = None;
        }
        if self.placed.is_empty() {
            self.tangent = None;
            self.chain_start.clear();
        }
    }

    pub fn click(&mut self, model: &Model) -> Result<Option<Transaction>, Refusal> {
        let (Some((feature, tool)), Some(placement)) = (self.context, self.hover) else {
            return Ok(None);
        };
        if let Snap::Target(Target::Pending(_)) = placement.snap {
            return Ok(match tool {
                Tool::Spline => self.finish(model),
                Tool::Select
                | Tool::Point
                | Tool::Line
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Arc
                | Tool::ThreePointArc
                | Tool::TangentArc
                | Tool::Slot
                | Tool::Polygon => {
                    self.cancel();
                    None
                }
            });
        }
        self.drawn(model, feature, tool, placement)
    }

    fn drawn(
        &mut self,
        model: &Model,
        feature: FeatureId,
        tool: Tool,
        placement: Placement,
    ) -> Result<Option<Transaction>, Refusal> {
        let shape = tool.label().to_lowercase();
        let draft = || Draft::new(model, feature, &shape, self.construction);
        Ok(match (tool, self.placed.as_slice()) {
            (Tool::Select, _) => None,
            (Tool::Point, _) => draft().map(|mut draft| {
                draft.point(placement);
                draft.finish()
            }),
            (Tool::Line, &[start]) => {
                if start.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::Line);
                }
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                let (first, end) = draft.line(start, placement);
                if self.chain_start.is_empty() {
                    self.chain_start.push(first);
                    self.chain_start.extend(point_target(start.snap));
                }
                let closed = point_target(placement.snap)
                    .is_some_and(|target| self.chain_start.contains(&target));
                if closed {
                    self.cancel();
                } else {
                    self.placed = vec![Placement {
                        position: placement.position,
                        snap: Snap::Target(Target::Point(end)),
                    }];
                }
                Some(draft.finish())
            }
            (Tool::Rectangle, &[corner]) => {
                let size = (placement.position - corner.position).abs();
                if size.min_element() < DEGENERATE_LENGTH {
                    return Err(Refusal::Rectangle);
                }
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                draft.rectangle(corner, placement);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::Circle, &[center]) => {
                if center.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::Circle);
                }
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                draft.circle(center, placement);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::Arc, &[center]) => {
                if center.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::ArcRadius);
                }
                self.sweep = Some(Sweep::new(center.position, placement.position));
                self.placed.push(placement);
                None
            }
            (Tool::Arc, &[center, start]) => {
                let radius = center.position.distance(start.position);
                let end = match placement.snap {
                    Snap::Target(_)
                        if snap::on_circle(center.position, radius, placement.position) =>
                    {
                        placement
                    }
                    Snap::Free | Snap::Target(_) | Snap::Aligned(_) => Placement::free(
                        arc_end(center.position, start.position, placement.position)
                            .ok_or(Refusal::ArcSweep)?,
                    ),
                };
                if end.position.distance(start.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::ArcSweep);
                }
                let counter_clockwise = self.sweep.is_none_or(|sweep| sweep.counter_clockwise());
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                draft.arc(center, start, end, counter_clockwise);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::ThreePointArc, &[start]) => {
                if start.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::ArcSweep);
                }
                self.placed.push(placement);
                None
            }
            (Tool::ThreePointArc, &[start, end]) => {
                let circular =
                    shapes::through_three(start.position, end.position, placement.position)
                        .ok_or(Refusal::ArcInLine)?;
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                draft.three_point_arc(start, end, placement, circular);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::TangentArc, &[]) => {
                if self.tangent.is_none() {
                    return Err(Refusal::TangentStart);
                }
                self.placed.push(placement);
                None
            }
            (Tool::TangentArc, &[start]) => {
                if start.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::ArcSweep);
                }
                let tangent = self.tangent.ok_or(Refusal::TangentStart)?;
                let circular =
                    shapes::tangent_from(start.position, tangent.direction, placement.position)
                        .ok_or(Refusal::TangentStraight)?;
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                let drawn = draft.tangent_arc(start, placement, tangent.curve, circular);
                if self.chain_start.is_empty() {
                    self.chain_start.push(drawn.start);
                    self.chain_start.extend(point_target(start.snap));
                }
                let closed = point_target(placement.snap)
                    .is_some_and(|target| self.chain_start.contains(&target));
                match shapes::leaving(circular, placement.position) {
                    Some(direction) if !closed => {
                        self.placed = vec![Placement {
                            position: placement.position,
                            snap: Snap::Target(Target::Point(drawn.end)),
                        }];
                        self.tangent = Some(Tangent {
                            curve: drawn.arc,
                            direction,
                        });
                    }
                    Some(_) | None => self.cancel(),
                }
                Some(draft.finish())
            }
            (Tool::Slot, &[first]) => {
                if first.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::SlotLength);
                }
                self.placed.push(placement);
                None
            }
            (Tool::Slot, &[first, second]) => {
                let slot = Slot::new(first.position, second.position, placement.position)
                    .ok_or(Refusal::SlotWidth)?;
                let Some(mut draft) = draft() else {
                    return Ok(None);
                };
                draft.slot(first, second, &slot);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::Polygon, &[center]) => {
                if center.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return Err(Refusal::PolygonSize);
                }
                let name = shapes::polygon_name(self.sides.0);
                let Some(mut draft) = Draft::new(model, feature, &name, self.construction) else {
                    return Ok(None);
                };
                draft.polygon(center, placement, self.sides.0);
                self.cancel();
                Some(draft.finish())
            }
            (
                Tool::Line
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Arc
                | Tool::ThreePointArc
                | Tool::Slot
                | Tool::Polygon
                | Tool::Spline,
                _,
            )
            | (Tool::TangentArc, _) => {
                self.placed.push(placement);
                None
            }
        })
    }

    pub fn finish(&mut self, model: &Model) -> Option<Transaction> {
        let (feature, tool) = self.context?;
        if tool != Tool::Spline {
            return None;
        }
        let placed = std::mem::take(&mut self.placed);
        if placed.len() < 2 {
            return None;
        }
        let mut draft = Draft::new(
            model,
            feature,
            &tool.label().to_lowercase(),
            self.construction,
        )?;
        draft.spline(&placed);
        Some(draft.finish())
    }

    pub fn preview(&self) -> Preview {
        let Some((_, tool)) = self.context else {
            return Preview::default();
        };
        let hover = self.hover.map(|hover| hover.position);
        let placed: Vec<Point2> = self
            .placed
            .iter()
            .map(|placement| placement.position)
            .collect();
        let mut preview = Preview {
            points: placed.iter().copied().chain(hover).collect(),
            snap: self
                .hover
                .filter(|hover| matches!(hover.snap, Snap::Target(_)))
                .map(|hover| hover.position),
            construction: self.construction,
            ..Preview::default()
        };
        let Some(cursor) = hover else {
            return preview;
        };
        match (tool, placed.as_slice()) {
            (Tool::Line, &[start]) => preview.curves.push(vec![start, cursor]),
            (Tool::Rectangle, &[corner]) => {
                let [a, b, c, d] = rectangle_corners(corner, cursor);
                preview.curves.push(vec![a, b, c, d, a]);
            }
            (Tool::Circle, &[center]) => preview.curves.push(
                ArcGeometry::full_circle(center, center.distance(cursor))
                    .polyline(PREVIEW_SEGMENT_ANGLE),
            ),
            (Tool::Arc, &[center]) => preview.curves.push(vec![center, cursor]),
            (Tool::Arc, &[center, start]) => {
                if let Some(end) = arc_end(center, start, cursor) {
                    let counter_clockwise =
                        self.sweep.is_none_or(|sweep| sweep.counter_clockwise());
                    let (from, to) = arc_ends(counter_clockwise, start, end);
                    preview.curves.push(
                        ArcGeometry::from_points(center, from, to).polyline(PREVIEW_SEGMENT_ANGLE),
                    );
                    preview.points = vec![center, start, end];
                }
            }
            (Tool::ThreePointArc | Tool::Slot, &[start]) => {
                preview.curves.push(vec![start, cursor]);
            }
            (Tool::ThreePointArc, &[start, end]) => {
                preview
                    .curves
                    .push(match shapes::through_three(start, end, cursor) {
                        Some(circular) => circular.arc(start, end).polyline(PREVIEW_SEGMENT_ANGLE),
                        None => vec![start, end],
                    });
            }
            (Tool::TangentArc, &[start]) => {
                let circular = self
                    .tangent
                    .and_then(|tangent| shapes::tangent_from(start, tangent.direction, cursor));
                preview.curves.push(match circular {
                    Some(circular) => circular.arc(start, cursor).polyline(PREVIEW_SEGMENT_ANGLE),
                    None => vec![start, cursor],
                });
            }
            (Tool::Slot, &[first, second]) => {
                preview.curves.push(match Slot::new(first, second, cursor) {
                    Some(slot) => slot.outline(PREVIEW_SEGMENT_ANGLE),
                    None => vec![first, second],
                });
                preview.points = vec![first, second];
            }
            (Tool::Polygon, &[center]) => {
                let mut corners = shapes::polygon_corners(center, cursor, self.sides.0);
                corners.extend(corners.first().copied());
                preview.curves.push(corners);
            }
            (Tool::Spline, _) if !placed.is_empty() => {
                if let Some(spline) = BSpline::clamped(preview.points.clone()) {
                    preview.curves.push(spline.polyline(PREVIEW_SEGMENT_ANGLE));
                }
            }
            _ => {}
        }
        preview
    }

    pub fn snap_label(&self, sketch: &Sketch) -> Option<String> {
        let (_, tool) = self.context?;
        Some(match self.hover?.snap {
            Snap::Free => return None,
            Snap::Aligned(Direction::Horizontal) => "Horizontal".to_owned(),
            Snap::Aligned(Direction::Vertical) => "Vertical".to_owned(),
            Snap::Target(Target::Pending(_)) if tool == Tool::Spline => {
                "Finish the spline".to_owned()
            }
            Snap::Target(Target::Pending(_)) => "Stop here".to_owned(),
            Snap::Target(Target::Point(_))
                if tool == Tool::TangentArc
                    && self.placed.is_empty()
                    && let Some(tangent) = self.tangent =>
            {
                format!("Continue {}", sketch.entity_label(tangent.curve))
            }
            Snap::Target(Target::Point(EntityId::ORIGIN)) => "Origin".to_owned(),
            Snap::Target(Target::Point(entity) | Target::Curve(entity)) => {
                format!("On {}", sketch.entity_label(entity))
            }
        })
    }

    pub fn prompt(&self) -> Option<Prompt> {
        let (_, tool) = self.context?;
        let prompt = |text: &str, keys| {
            Some(Prompt {
                text: text.to_owned(),
                keys,
            })
        };
        match (tool, self.placed.len()) {
            (Tool::Select, _) => None,
            (Tool::Point, _) => prompt("Click to place a point", BACK_TO_SELECT),
            (Tool::Line, 0) => prompt("Click the start of the line", BACK_TO_SELECT),
            (Tool::Line, _) => prompt(
                "Click to end the line, Escape to stop",
                "Click the start to close, or the last point again to stop",
            ),
            (Tool::Rectangle, 0) => prompt("Click the rectangle's first corner", BACK_TO_SELECT),
            (Tool::Rectangle, _) => {
                prompt("Click the opposite corner", "Esc: cancel the rectangle")
            }
            (Tool::Circle, 0) => prompt("Click the circle's centre", BACK_TO_SELECT),
            (Tool::Circle, _) => prompt("Click a point on the circle", "Esc: cancel the circle"),
            (Tool::Arc, 0) => prompt("Click the arc's centre", BACK_TO_SELECT),
            (Tool::Arc, 1) => prompt("Click where the arc starts", "Esc: cancel the arc"),
            (Tool::Arc, _) => prompt(
                "Click where the arc ends",
                "The arc follows your sweep around the centre, a typed end the shorter way   Esc: \
                 cancel the arc",
            ),
            (Tool::ThreePointArc, 0) => prompt("Click where the arc starts", BACK_TO_SELECT),
            (Tool::ThreePointArc, 1) => prompt("Click where the arc ends", "Esc: cancel the arc"),
            (Tool::ThreePointArc, _) => prompt(
                "Click a point the arc passes through",
                "Esc: cancel the arc",
            ),
            (Tool::TangentArc, 0) => prompt(
                "Click the end of a line, arc or spline to continue from",
                BACK_TO_SELECT,
            ),
            (Tool::TangentArc, _) => prompt(
                "Click where the arc ends, Escape to stop",
                "Click the start to close, or the last point again to stop",
            ),
            (Tool::Slot, 0) => prompt("Click the centre of the slot's first end", BACK_TO_SELECT),
            (Tool::Slot, 1) => prompt(
                "Click the centre of the slot's other end",
                "Esc: cancel the slot",
            ),
            (Tool::Slot, _) => prompt("Click to set the slot's width", "Esc: cancel the slot"),
            (Tool::Polygon, 0) => Some(Prompt {
                text: format!("Click the {}'s centre", shapes::polygon_name(self.sides.0)),
                keys: BACK_TO_SELECT,
            }),
            (Tool::Polygon, _) => Some(Prompt {
                text: format!(
                    "Click a corner of the {}",
                    shapes::polygon_name(self.sides.0)
                ),
                keys: "Esc: cancel the polygon",
            }),
            (Tool::Spline, 0) => prompt("Click the spline's first control point", BACK_TO_SELECT),
            (Tool::Spline, _) => prompt(
                "Click the next control point",
                "Enter or double-click: finish   Backspace: remove the last point   Esc: cancel",
            ),
        }
    }

    fn place(
        &self,
        tool: Tool,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Pointer,
    ) -> Placement {
        if let (Tool::Slot, &[_, _]) = (tool, self.placed.as_slice()) {
            return Placement::free(pointer.sketch);
        }
        let pending = self.pending(tool);
        let accept = self.accept(tool);
        if let Some(snapped) = snap::resolve(sketch, screen, pointer, &pending, accept) {
            return Placement {
                position: snapped.position,
                snap: Snap::Target(snapped.target),
            };
        }
        match (tool, self.placed.as_slice()) {
            (Tool::Line | Tool::Slot, &[start]) => align(start.position, screen, pointer),
            _ => None,
        }
        .unwrap_or(Placement::free(pointer.sketch))
    }
}

impl Drawing {
    fn accept(&self, tool: Tool) -> Accept {
        match (tool, self.placed.as_slice()) {
            (Tool::Circle, &[_]) | (Tool::ThreePointArc, &[_, _]) | (Tool::TangentArc, &[]) => {
                Accept::Points
            }
            (Tool::Arc, &[center, start]) => Accept::OnCircle {
                center: center.position,
                radius: center.position.distance(start.position),
            },
            _ => Accept::Anything,
        }
    }

    fn pending(&self, tool: Tool) -> Vec<(usize, Point2)> {
        match tool {
            Tool::Line | Tool::TangentArc => self.placed.first().map(|start| (0, start.position)),
            Tool::Spline => self
                .placed
                .last()
                .map(|last| (self.placed.len() - 1, last.position)),
            Tool::Select
            | Tool::Point
            | Tool::Rectangle
            | Tool::Circle
            | Tool::Arc
            | Tool::ThreePointArc
            | Tool::Slot
            | Tool::Polygon => None,
        }
        .into_iter()
        .collect()
    }
}

pub fn align(start: Point2, screen: &impl Screen, pointer: Pointer) -> Option<Placement> {
    let from = screen.to_screen(start)?;
    let drawn = pointer.screen - from;
    if drawn.length() < MIN_ALIGN_LENGTH {
        return None;
    }
    let max_angle = ALIGN_ANGLE_DEGREES.to_radians();
    let end = pointer.sketch;
    [
        (Direction::Horizontal, Point2::new(end.x, start.y)),
        (Direction::Vertical, Point2::new(start.x, end.y)),
    ]
    .into_iter()
    .filter_map(|(direction, position)| {
        let offset = screen.to_screen(position)?.distance(pointer.screen);
        let along = screen.to_screen(position)? - from;
        let angle = drawn.angle_to(along).abs();
        (offset <= ALIGN_TOLERANCE || angle <= max_angle).then_some((offset, direction, position))
    })
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, direction, position)| Placement {
        position,
        snap: Snap::Aligned(direction),
    })
}

fn continuing(sketch: &Sketch, point: EntityId) -> Option<Tangent> {
    let at = |id: EntityId| sketch.point(id);
    sketch
        .entities()
        .filter_map(|(curve, entity)| {
            let direction = match entity {
                Entity::Line { start, end } if *end == point => at(*end)? - at(*start)?,
                Entity::Line { start, end } if *start == point => at(*start)? - at(*end)?,
                Entity::Arc { center, end, .. } if *end == point => {
                    (at(*end)? - at(*center)?).perp()
                }
                Entity::Arc { center, start, .. } if *start == point => {
                    -(at(*start)? - at(*center)?).perp()
                }
                Entity::Spline { control_points } => match control_points.as_slice() {
                    [first, second, ..] if *first == point => at(*first)? - at(*second)?,
                    [.., before, last] if *last == point => at(*last)? - at(*before)?,
                    _ => return None,
                },
                Entity::Point(_)
                | Entity::Line { .. }
                | Entity::Circle { .. }
                | Entity::Arc { .. } => return None,
            };
            Some(Tangent {
                curve,
                direction: direction.try_normalize()?,
            })
        })
        .last()
}

fn arc_end(center: Point2, start: Point2, toward: Point2) -> Option<Point2> {
    snap::closest_on_circle(center, center.distance(start), toward)
}

fn rectangle_corners(corner: Point2, opposite: Point2) -> [Point2; 4] {
    [
        corner,
        Point2::new(opposite.x, corner.y),
        opposite,
        Point2::new(corner.x, opposite.y),
    ]
}

struct Draft<'a> {
    feature: FeatureId,
    transaction: TransactionBuilder<'a>,
    shadow: Sketch,
    construction: bool,
}

struct TangentArc {
    arc: EntityId,
    start: EntityId,
    end: EntityId,
}

impl<'a> Draft<'a> {
    fn new(model: &'a Model, feature: FeatureId, shape: &str, construction: bool) -> Option<Self> {
        let shadow = editing::edited_sketch(model.document(), feature)?.clone();
        let kind = if construction {
            format!("construction {shape}")
        } else {
            shape.to_owned()
        };
        Some(Self {
            feature,
            transaction: sketch_tools::settled_transaction(model, feature, format!("Draw {kind}")),
            shadow,
            construction,
        })
    }

    fn finish(self) -> Transaction {
        self.transaction.finish()
    }

    fn entity(&mut self, entity: Entity) -> EntityId {
        let construction = self.construction && !matches!(entity, Entity::Point(_));
        self.entity_as(entity, construction)
    }

    fn entity_as(&mut self, entity: Entity, construction: bool) -> EntityId {
        let id = self
            .transaction
            .add_sketch_entity_as(self.feature, entity.clone(), construction);
        if let Err(error) = self.shadow.insert_entity(id, entity) {
            log::debug!("the drawing check does not see entity {id}: {error}");
        }
        id
    }

    fn constrain(&mut self, constraint: Constraint) {
        if let Err(error) = self.shadow.check_constraint(&constraint) {
            log::debug!("not inferring {constraint:?}: {error}");
            return;
        }
        let id = self
            .transaction
            .add_sketch_constraint(self.feature, constraint.clone());
        if let Err(error) = self.shadow.insert_constraint(id, constraint) {
            log::debug!("the drawing check does not see constraint {id}: {error}");
        }
    }

    fn point(&mut self, placement: Placement) -> EntityId {
        let point = self.entity(Entity::Point(placement.position));
        if let Some(target) = placement.snap.entity() {
            self.constrain(Constraint::Coincident(point, target));
        }
        point
    }

    fn line(&mut self, start: Placement, end: Placement) -> (EntityId, EntityId) {
        let start = self.point(start);
        let end_point = self.point(end);
        let line = self.entity(Entity::Line {
            start,
            end: end_point,
        });
        match end.snap {
            Snap::Aligned(Direction::Horizontal) => self.constrain(Constraint::Horizontal(line)),
            Snap::Aligned(Direction::Vertical) => self.constrain(Constraint::Vertical(line)),
            Snap::Free | Snap::Target(_) => {}
        }
        (start, end_point)
    }

    fn rectangle(&mut self, corner: Placement, opposite: Placement) {
        let corners = rectangle_corners(corner.position, opposite.position);
        let snaps = [corner.snap, Snap::Free, opposite.snap, Snap::Free];
        let mut sides = Vec::with_capacity(corners.len());
        for (index, (position, snap)) in corners.into_iter().zip(snaps).enumerate() {
            let next = corners
                .get((index + 1) % corners.len())
                .copied()
                .unwrap_or(position);
            let start = self.point(Placement { position, snap });
            let end = self.point(Placement::free(next));
            let line = self.entity(Entity::Line { start, end });
            sides.push((line, start, end));
        }
        for (index, (line, _, end)) in sides.iter().enumerate() {
            if let Some((_, next_start, _)) = sides.get((index + 1) % sides.len()) {
                self.constrain(Constraint::Coincident(*end, *next_start));
            }
            self.constrain(if index % 2 == 0 {
                Constraint::Horizontal(*line)
            } else {
                Constraint::Vertical(*line)
            });
        }
    }

    fn circle(&mut self, center: Placement, rim: Placement) {
        let radius = center.position.distance(rim.position);
        let center = self.point(center);
        let circle = self.entity(Entity::Circle { center, radius });
        if let Snap::Target(Target::Point(point)) = rim.snap {
            self.constrain(Constraint::Coincident(point, circle));
        }
    }

    fn arc(
        &mut self,
        center: Placement,
        start: Placement,
        end: Placement,
        counter_clockwise: bool,
    ) {
        let center = self.point(center);
        let start = self.point(start);
        let end = self.point(end);
        let (start, end) = arc_ends(counter_clockwise, start, end);
        self.entity(Entity::Arc { center, start, end });
    }

    fn arc_through(
        &mut self,
        center: EntityId,
        start: Point2,
        end: Point2,
    ) -> (EntityId, EntityId, EntityId) {
        let start = self.entity(Entity::Point(start));
        let end = self.entity(Entity::Point(end));
        let arc = self.entity(Entity::Arc { center, start, end });
        (arc, start, end)
    }

    fn free_line(&mut self, start: Point2, end: Point2) -> (EntityId, EntityId, EntityId) {
        let start = self.entity(Entity::Point(start));
        let end = self.entity(Entity::Point(end));
        let line = self.entity(Entity::Line { start, end });
        (line, start, end)
    }

    fn three_point_arc(
        &mut self,
        start: Placement,
        end: Placement,
        through: Placement,
        circular: Circular,
    ) {
        let center = self.entity(Entity::Point(circular.center));
        let start = self.point(start);
        let end = self.point(end);
        let (start, end) = arc_ends(circular.counter_clockwise, start, end);
        let arc = self.entity(Entity::Arc { center, start, end });
        if let Some(point) = point_target(through.snap) {
            self.constrain(Constraint::Coincident(point, arc));
        }
    }

    fn tangent_arc(
        &mut self,
        start: Placement,
        end: Placement,
        from: EntityId,
        circular: Circular,
    ) -> TangentArc {
        let center = self.entity(Entity::Point(circular.center));
        let start = self.point(start);
        let end = self.point(end);
        let (first, last) = arc_ends(circular.counter_clockwise, start, end);
        let arc = self.entity(Entity::Arc {
            center,
            start: first,
            end: last,
        });
        self.constrain(Constraint::Tangent(from, arc));
        TangentArc { arc, start, end }
    }

    fn slot(&mut self, first: Placement, second: Placement, slot: &Slot) {
        let first_center = self.point(first);
        let second_center = self.point(second);
        let [a, b, c, d] = slot.corners();
        let (second_arc, second_start, second_end) = self.arc_through(second_center, a, b);
        let (first_arc, first_start, first_end) = self.arc_through(first_center, c, d);
        let (top, top_start, top_end) = self.free_line(b, c);
        let (bottom, bottom_start, bottom_end) = self.free_line(d, a);
        for (point, other) in [
            (top_start, second_end),
            (top_end, first_start),
            (bottom_start, first_end),
            (bottom_end, second_start),
        ] {
            self.constrain(Constraint::Coincident(point, other));
        }
        for (line, arc) in [
            (top, second_arc),
            (top, first_arc),
            (bottom, first_arc),
            (bottom, second_arc),
        ] {
            self.constrain(Constraint::Tangent(line, arc));
        }
        self.constrain(Constraint::Equal(first_arc, second_arc));
        match second.snap {
            Snap::Aligned(Direction::Horizontal) => self.constrain(Constraint::Horizontal(top)),
            Snap::Aligned(Direction::Vertical) => self.constrain(Constraint::Vertical(top)),
            Snap::Free | Snap::Target(_) => {}
        }
    }

    fn polygon(&mut self, center: Placement, corner: Placement, sides: usize) {
        let corners = shapes::polygon_corners(center.position, corner.position, sides);
        let radius = center.position.distance(corner.position);
        let center = self.point(center);
        let circle = self.entity_as(Entity::Circle { center, radius }, true);
        let mut edges = Vec::with_capacity(corners.len());
        for (index, position) in corners.iter().copied().enumerate() {
            let next = corners
                .get((index + 1) % corners.len())
                .copied()
                .unwrap_or(position);
            let snap = if index == 0 { corner.snap } else { Snap::Free };
            let start = self.point(Placement { position, snap });
            let end = self.point(Placement::free(next));
            let line = self.entity(Entity::Line { start, end });
            edges.push((line, start, end));
        }
        let Some(&(first, _, _)) = edges.first() else {
            return;
        };
        for (index, (line, start, end)) in edges.iter().copied().enumerate() {
            if let Some((_, next_start, _)) = edges.get((index + 1) % edges.len()) {
                self.constrain(Constraint::Coincident(end, *next_start));
            }
            self.constrain(Constraint::Coincident(start, circle));
            if line != first {
                self.constrain(Constraint::Equal(first, line));
            }
        }
    }

    fn spline(&mut self, placed: &[Placement]) {
        let control_points = placed
            .iter()
            .map(|placement| self.point(*placement))
            .collect();
        self.entity(Entity::Spline { control_points });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snap::tests::Scaled;

    fn pointer(sketch: Point2) -> Pointer {
        Pointer {
            screen: Scaled(10.0).to_screen(sketch).unwrap(),
            sketch,
        }
    }

    #[test]
    fn the_arc_runs_the_way_the_pointer_swept() {
        let center = Point2::ZERO;
        let mut sweep = Sweep::new(center, Point2::X);
        for point in [Point2::new(1.0, 0.5), Point2::Y, Point2::new(-1.0, 0.1)] {
            sweep.follow(point);
        }
        assert!(sweep.counter_clockwise());
        assert_eq!(arc_ends(sweep.counter_clockwise(), 1, 2), (1, 2));

        let mut sweep = Sweep::new(center, Point2::X);
        for point in [Point2::new(1.0, -0.5), -Point2::Y, Point2::new(-1.0, -0.1)] {
            sweep.follow(point);
        }
        assert!(!sweep.counter_clockwise());
        assert_eq!(arc_ends(sweep.counter_clockwise(), 1, 2), (2, 1));
    }

    #[test]
    fn a_sweep_past_half_a_turn_keeps_its_direction() {
        let mut sweep = Sweep::new(Point2::ZERO, Point2::X);
        for point in [Point2::Y, -Point2::X, -Point2::Y, Point2::new(0.9, -0.4)] {
            sweep.follow(point);
        }
        assert!(sweep.counter_clockwise());
        let arc = ArcGeometry::from_points(Point2::ZERO, Point2::X, Point2::new(0.9, -0.4));
        assert!(arc.sweep > 1.5 * PI);
    }

    #[test]
    fn a_typed_end_goes_the_shorter_way_round_unless_the_arc_is_reversed() {
        let mut sweep = Sweep::new(Point2::ZERO, Point2::X);
        for point in [Point2::new(0.5, 1.0), Point2::new(-1.0, 0.2)] {
            sweep.follow(point);
        }
        sweep.aim(Point2::new(0.0, -1.0));
        assert!(!sweep.counter_clockwise());

        sweep.reverse();
        assert!(sweep.counter_clockwise());
        sweep.aim(Point2::Y);
        assert!(!sweep.counter_clockwise());
        sweep.follow(Point2::new(-1.0, 0.5));
        assert!(!sweep.counter_clockwise());

        let mut half_turn = Sweep::new(Point2::ZERO, Point2::X);
        half_turn.aim(Point2::new(-1.0, 0.0));
        assert!(half_turn.counter_clockwise());
        half_turn.aim(Point2::new(-1.0, -0.0));
        assert!(half_turn.counter_clockwise());
    }

    #[test]
    fn nearly_level_lines_snap_level_within_a_few_degrees_or_pixels() {
        let screen = Scaled(10.0);
        let start = Point2::new(10.0, 10.0);

        let level = align(start, &screen, pointer(Point2::new(40.0, 11.0))).unwrap();
        assert_eq!(level.snap, Snap::Aligned(Direction::Horizontal));
        assert_eq!(level.position, Point2::new(40.0, 10.0));

        let upright = align(start, &screen, pointer(Point2::new(10.4, -20.0))).unwrap();
        assert_eq!(upright.snap, Snap::Aligned(Direction::Vertical));
        assert_eq!(upright.position, Point2::new(10.0, -20.0));

        let short = align(start, &screen, pointer(Point2::new(12.0, 10.5))).unwrap();
        assert_eq!(short.position, Point2::new(12.0, 10.0));

        assert_eq!(
            align(start, &screen, pointer(Point2::new(40.0, 13.0))),
            None
        );
        assert_eq!(
            align(start, &screen, pointer(Point2::new(10.8, 10.3))),
            None
        );
    }

    #[test]
    fn placing_prefers_existing_geometry_to_alignment() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        let lone = sketch.add_point(Point2::new(40.0, 10.5));
        let document = caditor_document::Document::default();
        let feature = document.transaction("Sketch").add_feature(
            "Sketch",
            caditor_document::FeatureKind::from(sketch.clone()),
        );
        let mut drawing = Drawing {
            context: Some((feature, Tool::Line)),
            placed: vec![Placement::free(Point2::new(10.0, 10.0))],
            ..Drawing::default()
        };
        let screen = Scaled(10.0);

        drawing.hover(&sketch, &screen, Some(pointer(Point2::new(40.0, 10.8))));
        assert_eq!(drawing.snap_entity(), Some(lone));

        drawing.hover(&sketch, &screen, Some(pointer(Point2::new(30.0, 10.4))));
        assert_eq!(
            drawing.hover.map(|hover| hover.snap),
            Some(Snap::Aligned(Direction::Horizontal))
        );
        assert_eq!(drawing.snap_label(&sketch).as_deref(), Some("Horizontal"));

        drawing.hover(&sketch, &screen, Some(pointer(Point2::new(10.3, 10.2))));
        assert_eq!(drawing.snap_label(&sketch).as_deref(), Some("Stop here"));
        let preview = drawing.preview();
        assert_eq!(preview.curves.len(), 1);
        assert_eq!(preview.snap, Some(Point2::new(10.0, 10.0)));
    }
}
