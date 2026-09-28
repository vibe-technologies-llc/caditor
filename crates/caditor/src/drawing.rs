use std::f64::consts::{PI, TAU};

use caditor_document::{FeatureId, Transaction, TransactionBuilder};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, BSpline, Constraint, Entity, EntityId, Sketch};

use crate::{
    editing::{self, ActiveSketch, Tool},
    model::Model,
    sketch_tools,
    snap::{self, Pointer, Screen, Target},
};

const ALIGN_ANGLE_DEGREES: f64 = 3.0;
const ALIGN_TOLERANCE: f64 = 6.0;
const MIN_ALIGN_LENGTH: f64 = 12.0;
const DEGENERATE_LENGTH: f64 = 1e-9;
const TYPED_TOLERANCE: f64 = 1e-6;
const PREVIEW_SEGMENT_ANGLE: f64 = PI / 60.0;
const BACK_TO_SELECT: &str = "Esc: back to Select";

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
    last_angle: f64,
    turned: f64,
}

impl Sweep {
    pub fn new(center: Point2, start: Point2) -> Self {
        Self {
            center,
            last_angle: angle_of(start - center),
            turned: 0.0,
        }
    }

    pub fn follow(&mut self, point: Point2) {
        let offset = point - self.center;
        if offset.length_squared() == 0.0 {
            return;
        }
        let angle = angle_of(offset);
        self.turned += (angle - self.last_angle + PI).rem_euclid(TAU) - PI;
        self.last_angle = angle;
    }

    pub fn counter_clockwise(&self) -> bool {
        self.turned >= 0.0
    }
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    pub text: &'static str,
    pub keys: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Drawing {
    context: Option<(FeatureId, Tool)>,
    placed: Vec<Placement>,
    hover: Option<Placement>,
    sweep: Option<Sweep>,
    chain_start: Vec<EntityId>,
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
                ..Self::default()
            };
        }
        let lost_anchor = sketch.is_some_and(|sketch| {
            self.placed.iter().any(|placement| {
                placement
                    .snap
                    .entity()
                    .is_some_and(|entity| !sketch.contains(entity))
            })
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
    }

    pub fn last_placed(&self) -> Option<Point2> {
        self.placed.last().map(|placement| placement.position)
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
            sweep.follow(position);
        }
    }

    pub fn leave(&mut self) {
        self.hover = None;
    }

    pub fn cancel(&mut self) {
        self.placed.clear();
        self.sweep = None;
        self.chain_start.clear();
    }

    pub fn remove_last(&mut self) {
        self.placed.pop();
        if self.placed.len() < 2 {
            self.sweep = None;
        }
        if self.placed.is_empty() {
            self.chain_start.clear();
        }
    }

    pub fn click(&mut self, model: &Model) -> Option<Transaction> {
        let (feature, tool) = self.context?;
        let placement = self.hover?;
        if let Snap::Target(Target::Pending(_)) = placement.snap {
            return match tool {
                Tool::Spline => self.finish(model),
                Tool::Select
                | Tool::Point
                | Tool::Line
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Arc => {
                    self.cancel();
                    None
                }
            };
        }
        let draft = || Draft::new(model, feature, tool);
        match (tool, self.placed.as_slice()) {
            (Tool::Select, _) => None,
            (Tool::Point, _) => {
                let mut draft = draft()?;
                draft.point(placement);
                Some(draft.finish())
            }
            (Tool::Line, &[start]) => {
                if start.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return None;
                }
                let mut draft = draft()?;
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
                    return None;
                }
                let mut draft = draft()?;
                draft.rectangle(corner, placement);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::Circle, &[center]) => {
                if center.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return None;
                }
                let mut draft = draft()?;
                draft.circle(center, placement);
                self.cancel();
                Some(draft.finish())
            }
            (Tool::Arc, &[center]) => {
                if center.position.distance(placement.position) < DEGENERATE_LENGTH {
                    return None;
                }
                self.sweep = Some(Sweep::new(center.position, placement.position));
                self.placed.push(placement);
                None
            }
            (Tool::Arc, &[center, start]) => {
                let end = arc_end(center.position, start.position, placement.position)?;
                if end.distance(start.position) < DEGENERATE_LENGTH {
                    return None;
                }
                let counter_clockwise = self.sweep.is_none_or(|sweep| sweep.counter_clockwise());
                let mut draft = draft()?;
                draft.arc(
                    center,
                    start,
                    Placement {
                        position: end,
                        snap: placement.snap,
                    },
                    counter_clockwise,
                );
                self.cancel();
                Some(draft.finish())
            }
            (Tool::Line | Tool::Rectangle | Tool::Circle | Tool::Arc | Tool::Spline, _) => {
                self.placed.push(placement);
                None
            }
        }
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
        let mut draft = Draft::new(model, feature, tool)?;
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
            Snap::Target(Target::Point(EntityId::ORIGIN)) => "Origin".to_owned(),
            Snap::Target(Target::Point(entity) | Target::Curve(entity)) => {
                format!("On {}", sketch.entity_label(entity))
            }
        })
    }

    pub fn prompt(&self) -> Option<Prompt> {
        let (_, tool) = self.context?;
        let prompt = |text, keys| Some(Prompt { text, keys });
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
                "The arc follows the way you sweep around the centre   Esc: cancel the arc",
            ),
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
        let pending = self.pending(tool);
        if let Some(snapped) = snap::resolve(sketch, screen, pointer, &pending) {
            return Placement {
                position: snapped.position,
                snap: Snap::Target(snapped.target),
            };
        }
        match (tool, self.placed.as_slice()) {
            (Tool::Line, &[start]) => align(start.position, screen, pointer),
            _ => None,
        }
        .unwrap_or(Placement::free(pointer.sketch))
    }
}

impl Drawing {
    fn pending(&self, tool: Tool) -> Vec<(usize, Point2)> {
        match tool {
            Tool::Line => self.placed.first().map(|start| (0, start.position)),
            Tool::Spline => self
                .placed
                .last()
                .map(|last| (self.placed.len() - 1, last.position)),
            Tool::Select | Tool::Point | Tool::Rectangle | Tool::Circle | Tool::Arc => None,
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
}

impl<'a> Draft<'a> {
    fn new(model: &'a Model, feature: FeatureId, tool: Tool) -> Option<Self> {
        let shadow = editing::edited_sketch(model.document(), feature)?.clone();
        let label = format!("Draw {}", tool.label().to_lowercase());
        Some(Self {
            feature,
            transaction: sketch_tools::settled_transaction(model, feature, label),
            shadow,
        })
    }

    fn finish(self) -> Transaction {
        self.transaction.finish()
    }

    fn entity(&mut self, entity: Entity) -> EntityId {
        let id = self
            .transaction
            .add_sketch_entity(self.feature, entity.clone());
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
