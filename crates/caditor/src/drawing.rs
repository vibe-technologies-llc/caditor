use std::f64::consts::{PI, TAU};

use caditor_document::{FeatureId, Transaction, TransactionBuilder};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{ArcGeometry, BSpline, Constraint, Entity, EntityId, Faceting, Sketch};

use crate::{
    editing::{self, ActiveSketch, Tool},
    model::Model,
    shape_modes::{CircleMode, PolygonMode, RectangleMode, ShapeMode, ShapeModes, SlotMode},
    shapes::{self, ArcSlot, Circular, DEGENERATE_LENGTH, MAX_SIDES, MIN_SIDES, Slot},
    sketch_tools,
    snap::{self, Accept, Pointer, Screen, Snapped, Target},
    tracking::{self, Acquired, Tracked, Tracks},
    units::Units,
};

const ALIGN_ANGLE_DEGREES: f64 = 3.0;
const ALIGN_TOLERANCE: f64 = 6.0;
const MIN_ALIGN_LENGTH: f64 = 12.0;
const NEARBY_LINES: usize = 6;
const HELD_TOLERANCE: f64 = 1e-9;
const ALIGNED_CROSSING_TOLERANCE: f64 = 12.0;
const TYPED_TOLERANCE: f64 = 1e-6;
const BACK_TO_SELECT: &str = "Esc: back to Select";
const CANCEL_RECTANGLE: &str = "Esc: cancel the rectangle";
const CANCEL_CIRCLE: &str = "Esc: cancel the circle";
const CANCEL_SLOT: &str = "Esc: cancel the slot";
const CANCEL_POLYGON: &str = "Esc: cancel the polygon";

const TOO_FEW_SIDES: &str = "A polygon needs at least three sides";
const TOO_MANY_SIDES: &str = "A polygon has at most 64 sides";
const NOT_A_POLYGON: &str = "Choose the Polygon tool first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Line,
    Rectangle,
    RectangleSide,
    RectangleWidth,
    Circle,
    CircleDiameter,
    CircleInLine,
    ArcRadius,
    ArcSweep,
    ArcInLine,
    TangentStart,
    TangentStraight,
    SlotLength,
    SlotWidth,
    ArcSlotRadius,
    ArcSlotSweep,
    ArcSlotWidth,
    PolygonSize,
    PolygonSideMiddle,
    PolygonSide,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Line => "A line needs its end away from its start",
            Self::Rectangle => "A rectangle needs its corners apart in both directions",
            Self::RectangleSide => "A rectangle needs the two ends of its first side apart",
            Self::RectangleWidth => "A rectangle needs a width: click away from its first side",
            Self::Circle => "A circle needs its rim away from its centre",
            Self::CircleDiameter => "A circle needs the two ends of its diameter apart",
            Self::CircleInLine => {
                "A circle through three points needs them apart and not all on one line"
            }
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
            Self::ArcSlotRadius => {
                "An arc slot needs the centre of its first end away from the centre of its arc"
            }
            Self::ArcSlotSweep => "An arc slot needs the centres of its two ends apart",
            Self::ArcSlotWidth => {
                "An arc slot needs a width: click off its arc, nearer to it than the arc's \
                 centre, and keep its round ends from meeting"
            }
            Self::PolygonSize => "A polygon needs its corner away from its centre",
            Self::PolygonSideMiddle => {
                "A polygon needs the middle of its side away from its centre"
            }
            Self::PolygonSide => "A polygon needs the two ends of its side apart",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Point,
    Line,
    Rectangle(RectangleMode),
    Circle(CircleMode),
    Arc,
    ThreePointArc,
    TangentArc,
    Slot(SlotMode),
    Polygon(PolygonMode),
    Spline,
}

impl From<ShapeMode> for Shape {
    fn from(mode: ShapeMode) -> Self {
        match mode {
            ShapeMode::Rectangle(mode) => Self::Rectangle(mode),
            ShapeMode::Circle(mode) => Self::Circle(mode),
            ShapeMode::Polygon(mode) => Self::Polygon(mode),
            ShapeMode::Slot(mode) => Self::Slot(mode),
        }
    }
}

impl Shape {
    fn of(tool: Tool, modes: ShapeModes) -> Option<Self> {
        match tool {
            Tool::Rectangle | Tool::Circle | Tool::Polygon | Tool::Slot => {
                modes.of(tool).map(Self::from)
            }
            Tool::Point => Some(Self::Point),
            Tool::Line => Some(Self::Line),
            Tool::Arc => Some(Self::Arc),
            Tool::ThreePointArc => Some(Self::ThreePointArc),
            Tool::TangentArc => Some(Self::TangentArc),
            Tool::Spline => Some(Self::Spline),
            Tool::Select
            | Tool::Trim
            | Tool::Extend
            | Tool::Offset
            | Tool::Mirror
            | Tool::Fillet
            | Tool::Project => None,
        }
    }

    fn mode(self) -> Option<ShapeMode> {
        match self {
            Self::Rectangle(mode) => Some(ShapeMode::Rectangle(mode)),
            Self::Circle(mode) => Some(ShapeMode::Circle(mode)),
            Self::Polygon(mode) => Some(ShapeMode::Polygon(mode)),
            Self::Slot(mode) => Some(ShapeMode::Slot(mode)),
            Self::Point
            | Self::Line
            | Self::Arc
            | Self::ThreePointArc
            | Self::TangentArc
            | Self::Spline => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Line => "line",
            Self::Rectangle(_) => "rectangle",
            Self::Circle(_) => "circle",
            Self::Arc => "arc",
            Self::ThreePointArc => "3-point arc",
            Self::TangentArc => "tangent arc",
            Self::Slot(SlotMode::Arc) => "arc slot",
            Self::Slot(SlotMode::Ends | SlotMode::Center) => "slot",
            Self::Polygon(_) => "polygon",
            Self::Spline => "spline",
        }
    }

    fn sizes_by_width(self, placed: usize) -> bool {
        match self {
            Self::Rectangle(RectangleMode::ThreePoints)
            | Self::Slot(SlotMode::Ends | SlotMode::Center) => placed == 2,
            Self::Slot(SlotMode::Arc) => placed == 3,
            Self::Point
            | Self::Line
            | Self::Rectangle(RectangleMode::Corners | RectangleMode::Center)
            | Self::Circle(_)
            | Self::Arc
            | Self::ThreePointArc
            | Self::TangentArc
            | Self::Polygon(_)
            | Self::Spline => false,
        }
    }

    fn aligns_second_point(self) -> bool {
        match self {
            Self::Line
            | Self::Rectangle(RectangleMode::ThreePoints)
            | Self::Slot(SlotMode::Ends | SlotMode::Center)
            | Self::Polygon(PolygonMode::Side) => true,
            Self::Point
            | Self::Rectangle(RectangleMode::Corners | RectangleMode::Center)
            | Self::Circle(_)
            | Self::Arc
            | Self::ThreePointArc
            | Self::TangentArc
            | Self::Slot(SlotMode::Arc)
            | Self::Polygon(PolygonMode::Corner | PolygonMode::SideMiddle)
            | Self::Spline => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tangent {
    curve: EntityId,
    direction: Vector2,
}

const SCRUB_POINTS_PER_SIDE: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scrub {
    from: f64,
    sides: usize,
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
    Parallel(EntityId),
    Perpendicular(EntityId),
}

impl Direction {
    fn words(self) -> (&'static str, Option<EntityId>) {
        match self {
            Self::Horizontal => ("Horizontal", None),
            Self::Vertical => ("Vertical", None),
            Self::Parallel(line) => ("Parallel to", Some(line)),
            Self::Perpendicular(line) => ("Perpendicular to", Some(line)),
        }
    }

    fn reference(self) -> Option<EntityId> {
        self.words().1
    }

    fn label(self, sketch: &Sketch) -> String {
        let (words, reference) = self.words();
        with_reference(words.to_owned(), reference, sketch)
    }

    fn joined_label(self, sketch: &Sketch) -> String {
        let (words, reference) = self.words();
        with_reference(words.to_lowercase(), reference, sketch)
    }

    fn constraint(self, line: EntityId) -> Constraint {
        match self {
            Self::Horizontal => Constraint::Horizontal(line),
            Self::Vertical => Constraint::Vertical(line),
            Self::Parallel(reference) => Constraint::Parallel(line, reference),
            Self::Perpendicular(reference) => Constraint::Perpendicular(line, reference),
        }
    }
}

fn capitalized(words: &str) -> String {
    let mut letters = words.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

fn with_reference(words: String, reference: Option<EntityId>, sketch: &Sketch) -> String {
    match reference {
        Some(line) => format!("{words} {}", sketch.entity_label(line)),
        None => words,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Guide {
    direction: Direction,
    along: Vector2,
}

const LEVEL_AND_UPRIGHT: [Guide; 2] = [
    Guide {
        direction: Direction::Horizontal,
        along: Vector2::X,
    },
    Guide {
        direction: Direction::Vertical,
        along: Vector2::Y,
    },
];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Aligned {
    guide: Guide,
    position: Point2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct NearbyLine {
    distance: f64,
    line: EntityId,
    along: Vector2,
    continued: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Snap {
    Free,
    Target(Target),
    Aligned(Direction),
    AlignedOn(Target, Direction),
}

impl Snap {
    fn target(self) -> Option<Target> {
        match self {
            Self::Target(target) | Self::AlignedOn(target, _) => Some(target),
            Self::Free | Self::Aligned(_) => None,
        }
    }

    fn direction(self) -> Option<Direction> {
        match self {
            Self::Aligned(direction) | Self::AlignedOn(_, direction) => Some(direction),
            Self::Free | Self::Target(_) => None,
        }
    }

    fn entity(self) -> Option<EntityId> {
        self.target()?.entity()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: Point2,
    pub snap: Snap,
    pub tracks: Tracks,
}

impl Placement {
    fn at(position: Point2, snap: Snap) -> Self {
        Self {
            position,
            snap,
            tracks: Tracks::default(),
        }
    }

    fn free(position: Point2) -> Self {
        Self::at(position, Snap::Free)
    }

    fn snapped(snapped: Snapped) -> Self {
        Self::at(snapped.position, Snap::Target(snapped.target))
    }

    fn tracked(tracked: Tracked, snap: Snap) -> Self {
        Self {
            position: tracked.position,
            snap,
            tracks: tracked.tracks,
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

    pub fn degrees(&self) -> f64 {
        let turned = self.turned.abs();
        (if self.reversed { TAU - turned } else { turned }).to_degrees()
    }
}

fn shortest_turn(from: f64, to: f64) -> f64 {
    let turn = (to - from + PI).rem_euclid(TAU) - PI;
    if turn <= -PI { PI } else { turn }
}

fn point_target(snap: Snap) -> Option<EntityId> {
    match snap.target()? {
        Target::Point(point) => Some(point),
        Target::Pending(_)
        | Target::Curve(_)
        | Target::Extension(_)
        | Target::Midpoint(_)
        | Target::Intersection(..)
        | Target::Centre { .. } => None,
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
    pub removed: Vec<Vec<Point2>>,
    pub points: Vec<Point2>,
    pub snap: Option<Point2>,
    pub guides: Vec<[Point2; 2]>,
    pub construction: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub keys: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Drawing {
    context: Option<(FeatureId, Shape)>,
    construction: bool,
    placed: Vec<Placement>,
    hover: Option<Placement>,
    sweep: Option<Sweep>,
    tangent: Option<Tangent>,
    chain: Vec<ChainStep>,
    chain_start: Vec<EntityId>,
    sides: Sides,
    scrub: Option<Scrub>,
    free: bool,
    acquired: Acquired,
    extension_guide: Option<[Point2; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
struct Carried {
    placed: Vec<Placement>,
    tangent: Option<Tangent>,
    chain: Vec<ChainStep>,
    chain_start: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq)]
struct ChainStep {
    start: Placement,
    tangent: Option<Tangent>,
    label: String,
}

impl Drawing {
    pub fn is_active(&self) -> bool {
        self.context.is_some()
    }

    pub fn in_progress(&self) -> bool {
        !self.placed.is_empty()
    }

    pub fn mode(&self) -> Option<ShapeMode> {
        self.context.and_then(|(_, shape)| shape.mode())
    }

    pub fn snap_entities(&self) -> Vec<EntityId> {
        let Some(hover) = self.hover else {
            return Vec::new();
        };
        hover
            .snap
            .entity()
            .into_iter()
            .chain(hover.snap.target().and_then(Target::second_entity))
            .chain(hover.snap.direction().and_then(Direction::reference))
            .chain(hover.tracks.points())
            .collect()
    }

    pub fn sync(
        &mut self,
        active: Option<ActiveSketch>,
        modes: ShapeModes,
        sketch: Option<&Sketch>,
    ) {
        let context =
            active.and_then(|active| Some((active.feature, Shape::of(active.tool, modes)?)));
        if context != self.context {
            let carried = self.continued_in(context, sketch);
            let same_sketch =
                self.context.map(|(feature, _)| feature) == context.map(|(feature, _)| feature);
            let acquired = if same_sketch {
                std::mem::take(&mut self.acquired)
            } else {
                Acquired::default()
            };
            *self = Self {
                context,
                sides: self.sides,
                acquired,
                ..Self::default()
            };
            if let Some(carried) = carried {
                self.placed = carried.placed;
                self.tangent = carried.tangent;
                self.chain = carried.chain;
                self.chain_start = carried.chain_start;
            }
        }
        self.construction = active.is_some_and(|active| active.construction);
        if let Some(sketch) = sketch {
            self.acquired.retain(sketch);
        }
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
            self.step_back_or_cancel(sketch);
        }
    }

    fn continued_in(
        &self,
        context: Option<(FeatureId, Shape)>,
        sketch: Option<&Sketch>,
    ) -> Option<Carried> {
        let (feature, from) = self.context?;
        let (next_feature, to) = context?;
        let [start] = self.placed.as_slice() else {
            return None;
        };
        let tangent = match (from, to) {
            (Shape::Line, Shape::TangentArc) => {
                Some(continuing(sketch?, point_target(start.snap)?)?)
            }
            (Shape::TangentArc, Shape::Line) => None,
            _ => return None,
        };
        (feature == next_feature).then(|| Carried {
            placed: vec![*start],
            tangent,
            chain: self.chain.clone(),
            chain_start: self.chain_start.clone(),
        })
    }

    fn step_back_or_cancel(&mut self, sketch: Option<&Sketch>) {
        let present = |entity: EntityId| sketch.is_some_and(|sketch| sketch.contains(entity));
        let arcing = self
            .context
            .is_some_and(|(_, shape)| shape == Shape::TangentArc);
        while let Some(step) = self.chain.pop() {
            let tangent = match step.tangent {
                None if arcing => sketch
                    .zip(point_target(step.start.snap))
                    .and_then(|(sketch, point)| continuing(sketch, point)),
                tangent => tangent,
            };
            let alive = step.start.snap.entity().is_none_or(present)
                && tangent.is_none_or(|tangent| present(tangent.curve))
                && (tangent.is_some() || !arcing);
            if alive {
                self.placed = vec![step.start];
                self.tangent = tangent;
                self.sweep = None;
                if self.chain.is_empty() {
                    self.chain_start.clear();
                }
                return;
            }
        }
        self.cancel();
    }

    fn segment_label(&self, shape: Shape) -> String {
        if self.construction {
            format!("Draw construction {}", shape.name())
        } else {
            format!("Draw {}", shape.name())
        }
    }

    pub fn place_freely(&mut self, free: bool) {
        self.free = free;
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        let Some((_, shape)) = self.context else {
            self.hover = None;
            return;
        };
        if self.scrub.is_some() {
            return;
        }
        self.hover = pointer.map(|pointer| self.place(shape, sketch, screen, pointer));
        self.extension_guide = self.hover.and_then(|hover| match hover.snap.target()? {
            Target::Extension(line) => snap::extension_guide(sketch, line, hover.position),
            _ => None,
        });
        if let Some(target) = self.hover.and_then(|hover| hover.snap.target()) {
            self.acquired.note(sketch, target);
        }
        if self.choosing_arc_end()
            && let (Some(sweep), Some(hover)) = (&mut self.sweep, self.hover)
        {
            sweep.follow(hover.position);
        }
        self.find_tangent(shape, sketch);
    }

    fn choosing_arc_end(&self) -> bool {
        self.sweep.is_some() && self.placed.len() == 2
    }

    fn find_tangent(&mut self, shape: Shape, sketch: &Sketch) {
        if shape == Shape::TangentArc && self.placed.is_empty() {
            self.tangent = point_target(self.hover.map_or(Snap::Free, |hover| hover.snap))
                .and_then(|point| continuing(sketch, point));
        }
    }

    pub fn readout(&self, unit: Units) -> Option<String> {
        let (_, shape) = self.context?;
        let hover = self.hover?.position;
        let length = |millimetres: f64| unit.readout_text(millimetres);
        let sides = self.sides.0;
        let [first] = self.placed.as_slice() else {
            return match (shape, self.placed.as_slice()) {
                (Shape::Arc, [center, start]) => {
                    let sweep = self.sweep?;
                    Some(format!(
                        "R {}   {}",
                        length(center.position.distance(start.position)),
                        unit.angle.readout_text(sweep.degrees())
                    ))
                }
                (Shape::ThreePointArc | Shape::Circle(CircleMode::ThreePoints), [a, b]) => {
                    let circle = shapes::circle_through_three(a.position, b.position, hover)?;
                    Some(format!("R {}", length(circle.radius)))
                }
                (Shape::Rectangle(RectangleMode::ThreePoints), [a, b]) => {
                    let side = b.position - a.position;
                    let across = side.try_normalize()?.perp();
                    Some(format!(
                        "{} × {}",
                        length(side.length()),
                        length(across.dot(hover - b.position).abs())
                    ))
                }
                (Shape::Slot(SlotMode::Ends), [a, b]) => {
                    let along = (b.position - a.position).try_normalize()?;
                    Some(format!(
                        "{} × Ø {}",
                        length(a.position.distance(b.position)),
                        length(2.0 * along.perp_dot(hover - a.position).abs())
                    ))
                }
                (Shape::Slot(SlotMode::Center), [center, end]) => {
                    let along = (end.position - center.position).try_normalize()?;
                    Some(format!(
                        "{} × Ø {}",
                        length(2.0 * center.position.distance(end.position)),
                        length(2.0 * along.perp_dot(hover - center.position).abs())
                    ))
                }
                _ => None,
            };
        };
        let delta = hover - first.position;
        match shape {
            Shape::Arc => Some(format!("R {}", length(delta.length()))),
            Shape::Circle(CircleMode::ThreePoints) | Shape::ThreePointArc => {
                Some(length(delta.length()))
            }
            Shape::Slot(SlotMode::Ends) => Some(length(delta.length())),
            Shape::Slot(SlotMode::Center) => Some(length(2.0 * delta.length())),
            Shape::Polygon(PolygonMode::Corner) => {
                Some(format!("R {}   {sides} sides", length(delta.length())))
            }
            Shape::Polygon(PolygonMode::SideMiddle) => {
                Some(format!("r {}   {sides} sides", length(delta.length())))
            }
            Shape::Polygon(PolygonMode::Side) => {
                Some(format!("{}   {sides} sides", length(delta.length())))
            }
            Shape::Rectangle(RectangleMode::ThreePoints) => Some(length(delta.length())),
            Shape::Line => Some(format!(
                "{}   {}",
                length(delta.length()),
                unit.angle.readout_text(delta.y.atan2(delta.x).to_degrees())
            )),
            Shape::Rectangle(RectangleMode::Corners) => Some(format!(
                "{} × {}",
                length(delta.x.abs()),
                length(delta.y.abs())
            )),
            Shape::Rectangle(RectangleMode::Center) => Some(format!(
                "{} × {}",
                length(2.0 * delta.x.abs()),
                length(2.0 * delta.y.abs())
            )),
            Shape::Circle(CircleMode::Center) => Some(format!("R {}", length(delta.length()))),
            Shape::Circle(CircleMode::TwoPoints) => Some(format!("Ø {}", length(delta.length()))),
            _ => None,
        }
    }

    pub fn last_placed(&self) -> Option<Point2> {
        self.placed.last().map(|placement| placement.position)
    }

    pub fn pointer_position(&self) -> Option<Point2> {
        self.hover.map(|hover| hover.position)
    }

    pub fn type_point(&mut self, sketch: &Sketch, position: Point2) {
        let Some((_, shape)) = self.context else {
            return;
        };
        let same = |candidate: Point2| candidate.distance(position) <= TYPED_TOLERANCE;
        let pending = self
            .pending(shape)
            .into_iter()
            .find(|(_, candidate)| same(*candidate))
            .map(|(index, candidate)| (candidate, Target::Pending(index)));
        let existing = snap::points(sketch)
            .into_iter()
            .find(|candidate| same(candidate.position))
            .map(|candidate| (candidate.position, candidate.target));
        let placement = match pending.or(existing) {
            Some((position, target)) => Placement::at(position, Snap::Target(target)),
            None => Placement::free(position),
        };
        self.hover = Some(placement);
        self.extension_guide = None;
        if self.choosing_arc_end()
            && let Some(sweep) = &mut self.sweep
        {
            sweep.aim(placement.position);
        }
        self.find_tangent(shape, sketch);
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

    pub fn set_sides(&mut self, count: usize) -> Result<(), &'static str> {
        self.polygon_sides()?;
        if count < MIN_SIDES {
            return Err(TOO_FEW_SIDES);
        }
        if count > MAX_SIDES {
            return Err(TOO_MANY_SIDES);
        }
        self.sides = Sides(count);
        Ok(())
    }

    pub fn can_type_sides(&self) -> bool {
        self.polygon_sides().is_ok()
    }

    pub fn scrub_sides(&mut self, pointer_x: Option<f64>) {
        let scrubbing = self.in_progress() && self.polygon_sides().is_ok();
        let Some(x) = pointer_x.filter(|_| scrubbing) else {
            self.scrub = None;
            return;
        };
        let scrub = *self.scrub.get_or_insert(Scrub {
            from: x,
            sides: self.sides.0,
        });
        let reach = MAX_SIDES as f64;
        let steps = ((x - scrub.from) / SCRUB_POINTS_PER_SIDE)
            .round()
            .clamp(-reach, reach);
        self.sides = Sides(
            scrub
                .sides
                .saturating_add_signed(steps as isize)
                .clamp(MIN_SIDES, MAX_SIDES),
        );
    }

    pub fn is_scrubbing(&self) -> bool {
        self.scrub.is_some()
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
            Some((_, Shape::Polygon(_))) => Ok(()),
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
        self.scrub = None;
        self.chain.clear();
        self.chain_start.clear();
    }

    pub fn remove_last(&mut self, undo_label: Option<&str>) -> bool {
        let undoes_segment = self.placed.len() == 1
            && self
                .chain
                .last()
                .is_some_and(|step| undo_label == Some(step.label.as_str()));
        if undoes_segment {
            return true;
        }
        self.placed.pop();
        if self.placed.len() < 2 {
            self.sweep = None;
        }
        if self.placed.is_empty() {
            self.tangent = None;
            self.chain.clear();
            self.chain_start.clear();
        }
        false
    }

    pub fn click(&mut self, model: &Model) -> Result<Option<Transaction>, Refusal> {
        let (Some((feature, shape)), Some(placement)) = (self.context, self.hover) else {
            return Ok(None);
        };
        if let Snap::Target(Target::Pending(_)) = placement.snap {
            return Ok(match shape {
                Shape::Spline => self.finish(model),
                Shape::Point
                | Shape::Line
                | Shape::Rectangle(_)
                | Shape::Circle(_)
                | Shape::Arc
                | Shape::ThreePointArc
                | Shape::TangentArc
                | Shape::Slot(_)
                | Shape::Polygon(_) => {
                    self.cancel();
                    None
                }
            });
        }
        self.drawn(model, feature, shape, placement)
    }

    fn drawn(
        &mut self,
        model: &Model,
        feature: FeatureId,
        shape: Shape,
        placement: Placement,
    ) -> Result<Option<Transaction>, Refusal> {
        let draft = |name: &str| Draft::new(model, feature, name, self.construction);
        let apart = |from: Placement, refusal: Refusal| {
            if from.position.distance(placement.position) < DEGENERATE_LENGTH {
                Err(refusal)
            } else {
                Ok(())
            }
        };
        let finished = match (shape, self.placed.as_slice()) {
            (Shape::Point, _) => draft(shape.name()).map(|mut draft| {
                draft.point(placement);
                draft
            }),
            (Shape::Line, &[start]) => {
                apart(start, Refusal::Line)?;
                let Some(mut draft) = draft(shape.name()) else {
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
                    self.chain.push(ChainStep {
                        start,
                        tangent: self.tangent,
                        label: self.segment_label(shape),
                    });
                    self.placed = vec![Placement::at(
                        placement.position,
                        Snap::Target(Target::Point(end)),
                    )];
                }
                return Ok(Some(draft.finish()));
            }
            (Shape::Rectangle(RectangleMode::Corners), &[corner]) => {
                if (placement.position - corner.position).abs().min_element() < DEGENERATE_LENGTH {
                    return Err(Refusal::Rectangle);
                }
                draft(shape.name()).map(|mut draft| {
                    draft.rectangle(corner, placement);
                    draft
                })
            }
            (Shape::Rectangle(RectangleMode::Center), &[center]) => {
                if (placement.position - center.position).abs().min_element() < DEGENERATE_LENGTH {
                    return Err(Refusal::Rectangle);
                }
                draft(shape.name()).map(|mut draft| {
                    draft.centered_rectangle(center, placement);
                    draft
                })
            }
            (Shape::Rectangle(RectangleMode::ThreePoints), &[first]) => {
                apart(first, Refusal::RectangleSide)?;
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::Rectangle(RectangleMode::ThreePoints), &[first, second]) => {
                let corners =
                    shapes::rectangle_on_side(first.position, second.position, placement.position)
                        .ok_or(Refusal::RectangleWidth)?;
                draft(shape.name()).map(|mut draft| {
                    draft.rectangle_on_side(first, second, corners);
                    draft
                })
            }
            (Shape::Circle(CircleMode::Center), &[center]) => {
                apart(center, Refusal::Circle)?;
                draft(shape.name()).map(|mut draft| {
                    draft.circle(center, placement);
                    draft
                })
            }
            (Shape::Circle(CircleMode::TwoPoints), &[first]) => {
                let circle = shapes::circle_on_diameter(first.position, placement.position)
                    .ok_or(Refusal::CircleDiameter)?;
                draft(shape.name()).map(|mut draft| {
                    draft.circle_on_diameter(first, placement, circle);
                    draft
                })
            }
            (Shape::Circle(CircleMode::ThreePoints), &[first]) => {
                apart(first, Refusal::CircleInLine)?;
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::Circle(CircleMode::ThreePoints), &[first, second]) => {
                let circle = shapes::circle_through_three(
                    first.position,
                    second.position,
                    placement.position,
                )
                .ok_or(Refusal::CircleInLine)?;
                draft(shape.name()).map(|mut draft| {
                    draft.circle_through(circle, &[first, second, placement]);
                    draft
                })
            }
            (Shape::Arc, &[center]) => {
                apart(center, Refusal::ArcRadius)?;
                self.sweep = Some(Sweep::new(center.position, placement.position));
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::Arc, &[center, start]) => {
                let end = landing_on_arc(center, start, placement).ok_or(Refusal::ArcSweep)?;
                let counter_clockwise = self.counter_clockwise();
                draft(shape.name()).map(|mut draft| {
                    draft.arc(center, start, end, counter_clockwise);
                    draft
                })
            }
            (Shape::ThreePointArc, &[start]) => {
                apart(start, Refusal::ArcSweep)?;
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::ThreePointArc, &[start, end]) => {
                let circular =
                    shapes::through_three(start.position, end.position, placement.position)
                        .ok_or(Refusal::ArcInLine)?;
                draft(shape.name()).map(|mut draft| {
                    draft.three_point_arc(start, end, placement, circular);
                    draft
                })
            }
            (Shape::TangentArc, &[]) => {
                if self.tangent.is_none() {
                    return Err(Refusal::TangentStart);
                }
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::TangentArc, &[start]) => {
                apart(start, Refusal::ArcSweep)?;
                let tangent = self.tangent.ok_or(Refusal::TangentStart)?;
                let circular =
                    shapes::tangent_from(start.position, tangent.direction, placement.position)
                        .ok_or(Refusal::TangentStraight)?;
                let Some(mut draft) = draft(shape.name()) else {
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
                        self.chain.push(ChainStep {
                            start,
                            tangent: Some(tangent),
                            label: self.segment_label(shape),
                        });
                        self.placed = vec![Placement::at(
                            placement.position,
                            Snap::Target(Target::Point(drawn.end)),
                        )];
                        self.tangent = Some(Tangent {
                            curve: drawn.curve,
                            direction,
                        });
                    }
                    Some(_) | None => self.cancel(),
                }
                return Ok(Some(draft.finish()));
            }
            (Shape::Slot(SlotMode::Ends), &[first]) => {
                apart(first, Refusal::SlotLength)?;
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::Slot(SlotMode::Ends), &[first, second]) => {
                let slot = Slot::new(first.position, second.position, placement.position)
                    .ok_or(Refusal::SlotWidth)?;
                draft(shape.name()).map(|mut draft| {
                    draft.slot(first, second, &slot);
                    draft
                })
            }
            (Shape::Slot(SlotMode::Center), &[center]) => {
                apart(center, Refusal::SlotLength)?;
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::Slot(SlotMode::Center), &[center, end]) => {
                let mirrored = shapes::mirrored(end.position, center.position);
                let slot = Slot::new(mirrored, end.position, placement.position)
                    .ok_or(Refusal::SlotWidth)?;
                draft(shape.name()).map(|mut draft| {
                    draft.centered_slot(center, end, &slot);
                    draft
                })
            }
            (Shape::Slot(SlotMode::Arc), &[center]) => {
                apart(center, Refusal::ArcSlotRadius)?;
                self.sweep = Some(Sweep::new(center.position, placement.position));
                self.placed.push(placement);
                return Ok(None);
            }
            (Shape::Slot(SlotMode::Arc), &[center, start]) => {
                let end = landing_on_arc(center, start, placement).ok_or(Refusal::ArcSlotSweep)?;
                self.placed.push(end);
                return Ok(None);
            }
            (Shape::Slot(SlotMode::Arc), &[center, start, end]) => {
                let (first, last) = arc_ends(self.counter_clockwise(), start, end);
                let slot = ArcSlot::new(
                    center.position,
                    [first.position, last.position],
                    placement.position,
                )
                .ok_or(Refusal::ArcSlotWidth)?;
                draft(shape.name()).map(|mut draft| {
                    draft.arc_slot(center, [first, last], &slot);
                    draft
                })
            }
            (Shape::Polygon(PolygonMode::Corner), &[center]) => {
                apart(center, Refusal::PolygonSize)?;
                let corners =
                    shapes::polygon_corners(center.position, placement.position, self.sides.0);
                self.polygon_draft(model, feature).map(|mut draft| {
                    draft.polygon(center, &placed_first(&corners, &[placement]));
                    draft
                })
            }
            (Shape::Polygon(PolygonMode::SideMiddle), &[center]) => {
                apart(center, Refusal::PolygonSideMiddle)?;
                let corners = shapes::polygon_around_side_middle(
                    center.position,
                    placement.position,
                    self.sides.0,
                );
                self.polygon_draft(model, feature).map(|mut draft| {
                    draft.polygon_around_side_middle(center, placement, &corners);
                    draft
                })
            }
            (Shape::Polygon(PolygonMode::Side), &[first]) => {
                let (center, corners) =
                    shapes::polygon_on_side(first.position, placement.position, self.sides.0)
                        .ok_or(Refusal::PolygonSide)?;
                self.polygon_draft(model, feature).map(|mut draft| {
                    draft.polygon_on_side(first, placement, center, &corners);
                    draft
                })
            }
            (
                Shape::Line
                | Shape::Rectangle(_)
                | Shape::Circle(_)
                | Shape::Arc
                | Shape::ThreePointArc
                | Shape::TangentArc
                | Shape::Slot(_)
                | Shape::Polygon(_)
                | Shape::Spline,
                _,
            ) => {
                self.placed.push(placement);
                return Ok(None);
            }
        };
        if shape != Shape::Point {
            self.cancel();
        }
        Ok(finished.map(Draft::finish))
    }

    fn counter_clockwise(&self) -> bool {
        self.sweep.is_none_or(|sweep| sweep.counter_clockwise())
    }

    fn polygon_draft<'a>(&self, model: &'a Model, feature: FeatureId) -> Option<Draft<'a>> {
        Draft::new(
            model,
            feature,
            &shapes::polygon_name(self.sides.0),
            self.construction,
        )
    }

    pub fn finish(&mut self, model: &Model) -> Option<Transaction> {
        let (feature, shape) = self.context?;
        if shape != Shape::Spline {
            return None;
        }
        let placed = std::mem::take(&mut self.placed);
        if placed.len() < 2 {
            return None;
        }
        let mut draft = Draft::new(model, feature, shape.name(), self.construction)?;
        draft.spline(&placed);
        Some(draft.finish())
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let Some((_, shape)) = self.context else {
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
                .filter(|hover| hover.snap.target().is_some())
                .map(|hover| hover.position),
            guides: self
                .hover
                .into_iter()
                .flat_map(|hover| hover.tracks.guides(hover.position))
                .chain(self.extension_guide)
                .collect(),
            construction: self.construction,
            ..Preview::default()
        };
        let Some(cursor) = hover else {
            return preview;
        };
        let closed = |mut corners: Vec<Point2>| {
            corners.extend(corners.first().copied());
            corners
        };
        match (shape, placed.as_slice()) {
            (Shape::Line, &[start]) => preview.curves.push(vec![start, cursor]),
            (Shape::Rectangle(RectangleMode::Corners), &[corner]) => {
                preview
                    .curves
                    .push(closed(rectangle_corners(corner, cursor).to_vec()));
            }
            (Shape::Rectangle(RectangleMode::Center), &[center]) => {
                let opposite = shapes::mirrored(cursor, center);
                preview
                    .curves
                    .push(closed(rectangle_corners(cursor, opposite).to_vec()));
            }
            (Shape::Rectangle(RectangleMode::ThreePoints), &[first, second]) => {
                preview
                    .curves
                    .push(match shapes::rectangle_on_side(first, second, cursor) {
                        Some(corners) => closed(corners.to_vec()),
                        None => vec![first, second],
                    });
            }
            (Shape::Circle(CircleMode::Center), &[center]) => preview
                .curves
                .push(ArcGeometry::full_circle(center, center.distance(cursor)).faceted(faceting)),
            (Shape::Circle(CircleMode::TwoPoints), &[first]) => {
                preview
                    .curves
                    .push(match shapes::circle_on_diameter(first, cursor) {
                        Some(circle) => circle.faceted(faceting),
                        None => vec![first, cursor],
                    });
            }
            (Shape::Circle(CircleMode::ThreePoints), &[first, second]) => {
                preview
                    .curves
                    .push(match shapes::circle_through_three(first, second, cursor) {
                        Some(circle) => circle.faceted(faceting),
                        None => vec![first, second],
                    });
            }
            (Shape::Arc | Shape::Slot(SlotMode::Arc), &[center]) => {
                preview.curves.push(vec![center, cursor]);
            }
            (Shape::Arc | Shape::Slot(SlotMode::Arc), &[center, start]) => {
                if let Some(end) = arc_end(center, start, cursor) {
                    preview
                        .curves
                        .push(self.arc_polyline(center, start, end, faceting));
                    preview.points = vec![center, start, end];
                }
            }
            (Shape::Slot(SlotMode::Arc), &[center, start, end]) => {
                let (first, last) = arc_ends(self.counter_clockwise(), start, end);
                preview
                    .curves
                    .push(match ArcSlot::new(center, [first, last], cursor) {
                        Some(slot) => slot.outline(faceting),
                        None => self.arc_polyline(center, start, end, faceting),
                    });
                preview.points = vec![center, start, end];
            }
            (
                Shape::ThreePointArc
                | Shape::Slot(SlotMode::Ends)
                | Shape::Rectangle(RectangleMode::ThreePoints)
                | Shape::Circle(CircleMode::ThreePoints),
                &[start],
            ) => {
                preview.curves.push(vec![start, cursor]);
            }
            (Shape::Slot(SlotMode::Center), &[center]) => {
                preview
                    .curves
                    .push(vec![shapes::mirrored(cursor, center), cursor]);
            }
            (Shape::ThreePointArc, &[start, end]) => {
                preview
                    .curves
                    .push(match shapes::through_three(start, end, cursor) {
                        Some(circular) => circular.arc(start, end).faceted(faceting),
                        None => vec![start, end],
                    });
            }
            (Shape::TangentArc, &[start]) => {
                let circular = self
                    .tangent
                    .and_then(|tangent| shapes::tangent_from(start, tangent.direction, cursor));
                preview.curves.push(match circular {
                    Some(circular) => circular.arc(start, cursor).faceted(faceting),
                    None => vec![start, cursor],
                });
            }
            (Shape::Slot(SlotMode::Ends), &[first, second]) => {
                preview.curves.push(match Slot::new(first, second, cursor) {
                    Some(slot) => slot.outline(faceting),
                    None => vec![first, second],
                });
                preview.points = vec![first, second];
            }
            (Shape::Slot(SlotMode::Center), &[center, end]) => {
                let mirrored = shapes::mirrored(end, center);
                preview.curves.push(match Slot::new(mirrored, end, cursor) {
                    Some(slot) => slot.outline(faceting),
                    None => vec![mirrored, end],
                });
                preview.points = vec![mirrored, center, end];
            }
            (Shape::Polygon(PolygonMode::Corner), &[center]) => {
                preview.curves.push(closed(shapes::polygon_corners(
                    center,
                    cursor,
                    self.sides.0,
                )));
            }
            (Shape::Polygon(PolygonMode::SideMiddle), &[center]) => {
                preview
                    .curves
                    .push(closed(shapes::polygon_around_side_middle(
                        center,
                        cursor,
                        self.sides.0,
                    )));
            }
            (Shape::Polygon(PolygonMode::Side), &[first]) => {
                preview
                    .curves
                    .push(match shapes::polygon_on_side(first, cursor, self.sides.0) {
                        Some((_, corners)) => closed(corners),
                        None => vec![first, cursor],
                    });
            }
            (Shape::Spline, _) if !placed.is_empty() => {
                if let Some(spline) = BSpline::clamped(preview.points.clone()) {
                    preview.curves.push(spline.faceted(faceting));
                }
            }
            _ => {}
        }
        preview
    }

    fn arc_polyline(
        &self,
        center: Point2,
        start: Point2,
        end: Point2,
        faceting: Faceting,
    ) -> Vec<Point2> {
        let (from, to) = arc_ends(self.counter_clockwise(), start, end);
        ArcGeometry::from_points(center, from, to).faceted(faceting)
    }

    pub fn snap_label(&self, sketch: &Sketch) -> Option<String> {
        let (_, shape) = self.context?;
        let hover = self.hover?;
        let snap = hover.snap;
        let label = match (snap.target(), snap.direction()) {
            (None, None) => None,
            (None, Some(direction)) => Some(direction.label(sketch)),
            (Some(target), None) => Some(self.target_label(shape, sketch, target)),
            (Some(target), Some(direction)) => Some(format!(
                "{}, {}",
                self.target_label(shape, sketch, target),
                direction.joined_label(sketch)
            )),
        };
        match (label, hover.tracks.label(sketch)) {
            (label, None) => label,
            (Some(label), Some(tracked)) => Some(format!("{label}, {tracked}")),
            (None, Some(tracked)) => Some(capitalized(&tracked)),
        }
    }

    fn target_label(&self, shape: Shape, sketch: &Sketch, target: Target) -> String {
        match target {
            Target::Pending(_) if shape == Shape::Spline => "Finish the spline".to_owned(),
            Target::Pending(_) => "Stop here".to_owned(),
            Target::Point(_)
                if shape == Shape::TangentArc
                    && self.placed.is_empty()
                    && let Some(tangent) = self.tangent =>
            {
                format!("Continue {}", sketch.entity_label(tangent.curve))
            }
            Target::Point(EntityId::ORIGIN) => "Origin".to_owned(),
            Target::Midpoint(line) => format!("Midpoint of {}", sketch.entity_label(line)),
            Target::Centre { outline, .. } => {
                format!("Centre of the outline of {}", sketch.entity_label(outline))
            }
            Target::Intersection(first, second) => format!(
                "Crossing of {} and {}",
                sketch.entity_label(first),
                sketch.entity_label(second)
            ),
            Target::Point(entity) | Target::Curve(entity) => {
                format!("On {}", sketch.entity_label(entity))
            }
            Target::Extension(line) => {
                format!("On the extension of {}", sketch.entity_label(line))
            }
        }
    }

    pub fn prompt(&self) -> Option<Prompt> {
        let (_, shape) = self.context?;
        let prompt = |text: &str, keys| {
            Some(Prompt {
                text: text.to_owned(),
                keys,
            })
        };
        let polygon = shapes::polygon_name(self.sides.0);
        let polygon_prompt = |text: String, keys| Some(Prompt { text, keys });
        match (shape, self.placed.len()) {
            (Shape::Point, _) => prompt("Click to place a point", BACK_TO_SELECT),
            (Shape::Line, 0) => prompt("Click the start of the line", BACK_TO_SELECT),
            (Shape::Line, _) => prompt(
                "Click to end the line, Escape to stop",
                "Click the start to close, or the last point again to stop   Backspace: step back",
            ),
            (Shape::Rectangle(RectangleMode::Corners), 0) => {
                prompt("Click the rectangle's first corner", BACK_TO_SELECT)
            }
            (Shape::Rectangle(RectangleMode::Corners), _) => {
                prompt("Click the opposite corner", CANCEL_RECTANGLE)
            }
            (Shape::Rectangle(RectangleMode::Center), 0) => {
                prompt("Click the rectangle's centre", BACK_TO_SELECT)
            }
            (Shape::Rectangle(RectangleMode::Center), _) => {
                prompt("Click a corner of the rectangle", CANCEL_RECTANGLE)
            }
            (Shape::Rectangle(RectangleMode::ThreePoints), 0) => prompt(
                "Click where the rectangle's first side starts",
                BACK_TO_SELECT,
            ),
            (Shape::Rectangle(RectangleMode::ThreePoints), 1) => {
                prompt("Click where its first side ends", CANCEL_RECTANGLE)
            }
            (Shape::Rectangle(RectangleMode::ThreePoints), _) => {
                prompt("Click to set the rectangle's width", CANCEL_RECTANGLE)
            }
            (Shape::Circle(CircleMode::Center), 0) => {
                prompt("Click the circle's centre", BACK_TO_SELECT)
            }
            (Shape::Circle(CircleMode::Center), _) => {
                prompt("Click a point on the circle", CANCEL_CIRCLE)
            }
            (Shape::Circle(CircleMode::TwoPoints), 0) => {
                prompt("Click one end of the circle's diameter", BACK_TO_SELECT)
            }
            (Shape::Circle(CircleMode::TwoPoints), _) => {
                prompt("Click the other end of the diameter", CANCEL_CIRCLE)
            }
            (Shape::Circle(CircleMode::ThreePoints), 0) => {
                prompt("Click a first point on the circle", BACK_TO_SELECT)
            }
            (Shape::Circle(CircleMode::ThreePoints), 1) => {
                prompt("Click a second point on the circle", CANCEL_CIRCLE)
            }
            (Shape::Circle(CircleMode::ThreePoints), _) => {
                prompt("Click a third point on the circle", CANCEL_CIRCLE)
            }
            (Shape::Arc, 0) => prompt("Click the arc's centre", BACK_TO_SELECT),
            (Shape::Arc, 1) => prompt("Click where the arc starts", "Esc: cancel the arc"),
            (Shape::Arc, _) => prompt(
                "Click where the arc ends",
                "The arc follows your sweep around the centre, a typed end the shorter way   Esc: \
                 cancel the arc",
            ),
            (Shape::ThreePointArc, 0) => prompt("Click where the arc starts", BACK_TO_SELECT),
            (Shape::ThreePointArc, 1) => prompt("Click where the arc ends", "Esc: cancel the arc"),
            (Shape::ThreePointArc, _) => prompt(
                "Click a point the arc passes through",
                "Esc: cancel the arc",
            ),
            (Shape::TangentArc, 0) => prompt(
                "Click the end of a line, arc or spline to continue from",
                BACK_TO_SELECT,
            ),
            (Shape::TangentArc, _) => prompt(
                "Click where the arc ends, Escape to stop",
                "Click the start to close, or the last point again to stop",
            ),
            (Shape::Slot(SlotMode::Ends), 0) => {
                prompt("Click the centre of the slot's first end", BACK_TO_SELECT)
            }
            (Shape::Slot(SlotMode::Ends), 1) => {
                prompt("Click the centre of the slot's other end", CANCEL_SLOT)
            }
            (Shape::Slot(SlotMode::Center), 0) => prompt("Click the slot's centre", BACK_TO_SELECT),
            (Shape::Slot(SlotMode::Center), 1) => {
                prompt("Click the centre of one of the slot's ends", CANCEL_SLOT)
            }
            (Shape::Slot(SlotMode::Arc), 0) => prompt(
                "Click the centre of the arc the slot follows",
                BACK_TO_SELECT,
            ),
            (Shape::Slot(SlotMode::Arc), 1) => {
                prompt("Click the centre of the slot's first end", CANCEL_SLOT)
            }
            (Shape::Slot(SlotMode::Arc), 2) => prompt(
                "Click the centre of the slot's other end",
                "The slot follows your sweep around the centre, a typed end the shorter way   \
                 Esc: cancel the slot",
            ),
            (Shape::Slot(_), _) => prompt("Click to set the slot's width", CANCEL_SLOT),
            (Shape::Polygon(PolygonMode::Corner | PolygonMode::SideMiddle), 0) => {
                polygon_prompt(format!("Click the {polygon}'s centre"), BACK_TO_SELECT)
            }
            (Shape::Polygon(PolygonMode::Corner), _) => {
                polygon_prompt(format!("Click a corner of the {polygon}"), CANCEL_POLYGON)
            }
            (Shape::Polygon(PolygonMode::SideMiddle), _) => polygon_prompt(
                format!("Click the middle of a side of the {polygon}"),
                CANCEL_POLYGON,
            ),
            (Shape::Polygon(PolygonMode::Side), 0) => polygon_prompt(
                format!("Click where a side of the {polygon} starts"),
                BACK_TO_SELECT,
            ),
            (Shape::Polygon(PolygonMode::Side), _) => polygon_prompt(
                format!("Click where that side of the {polygon} ends"),
                CANCEL_POLYGON,
            ),
            (Shape::Spline, 0) => prompt("Click the spline's first control point", BACK_TO_SELECT),
            (Shape::Spline, _) => prompt(
                "Click the next control point",
                "Enter or double-click: finish   Backspace: remove the last point   Esc: cancel",
            ),
        }
    }

    fn place(
        &self,
        shape: Shape,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Pointer,
    ) -> Placement {
        if self.free || shape.sizes_by_width(self.placed.len()) {
            return Placement::free(pointer.sketch);
        }
        let pending = self.pending(shape);
        let accept = self.accept(shape);
        let snapped = snap::resolve(
            sketch,
            screen,
            pointer,
            &pending,
            accept,
            self.acquired.lines(),
        );
        let tracks = match accept {
            Accept::Anything => {
                let placed: Vec<EntityId> = self
                    .placed
                    .iter()
                    .filter_map(|placement| point_target(placement.snap))
                    .collect();
                tracking::nearby(sketch, screen, pointer, &self.acquired, &placed)
            }
            Accept::Points | Accept::OnCircle { .. } => Tracks::default(),
        };
        let tracked_on = |snapped: Snapped| {
            tracking::on_curve(
                sketch,
                snapped.target,
                tracks,
                screen,
                pointer,
                ALIGNED_CROSSING_TOLERANCE,
            )
            .map(|tracked| Placement::tracked(tracked, Snap::Target(snapped.target)))
        };
        let tracked_alone = || {
            tracking::alone(tracks, screen, pointer)
                .map(|tracked| Placement::tracked(tracked, Snap::Free))
                .unwrap_or(Placement::free(pointer.sketch))
        };
        let Some(start) = self.aligned_from(shape) else {
            return match snapped {
                Some(snapped) => tracked_on(snapped).unwrap_or(Placement::snapped(snapped)),
                None => tracked_alone(),
            };
        };
        let continued = match shape {
            Shape::Line => point_target(start.snap),
            _ => None,
        };
        let guides = guides(sketch, screen, pointer, start.position, continued);
        match snapped {
            Some(snapped) => aligned_on(sketch, screen, pointer, start.position, snapped, &guides)
                .or_else(|| tracked_on(snapped))
                .unwrap_or(Placement::snapped(snapped)),
            None => match align(start.position, screen, pointer, &guides) {
                Some(aligned) => aligned
                    .snap
                    .direction()
                    .and_then(|direction| {
                        let along = aligned.position - start.position;
                        tracking::on_ray(
                            tracks,
                            start.position,
                            along,
                            screen,
                            pointer,
                            ALIGNED_CROSSING_TOLERANCE,
                        )
                        .filter(|tracked| {
                            tracked.position.distance(start.position) >= DEGENERATE_LENGTH
                        })
                        .map(|tracked| Placement::tracked(tracked, Snap::Aligned(direction)))
                    })
                    .unwrap_or(aligned),
                None => tracked_alone(),
            },
        }
    }

    fn aligned_from(&self, shape: Shape) -> Option<Placement> {
        match self.placed.as_slice() {
            &[start] if shape.aligns_second_point() => Some(start),
            _ => None,
        }
    }
}

impl Drawing {
    fn accept(&self, shape: Shape) -> Accept {
        match (shape, self.placed.as_slice()) {
            (Shape::Circle(CircleMode::Center), &[_])
            | (Shape::Circle(CircleMode::TwoPoints | CircleMode::ThreePoints), _)
            | (Shape::Polygon(PolygonMode::SideMiddle), &[_])
            | (Shape::ThreePointArc, &[_, _])
            | (Shape::TangentArc, &[]) => Accept::Points,
            (Shape::Arc | Shape::Slot(SlotMode::Arc), &[center, start]) => Accept::OnCircle {
                center: center.position,
                radius: center.position.distance(start.position),
            },
            _ => Accept::Anything,
        }
    }

    fn pending(&self, shape: Shape) -> Vec<(usize, Point2)> {
        match shape {
            Shape::Line | Shape::TangentArc => self.placed.first().map(|start| (0, start.position)),
            Shape::Spline => self
                .placed
                .last()
                .map(|last| (self.placed.len() - 1, last.position)),
            Shape::Point
            | Shape::Rectangle(_)
            | Shape::Circle(_)
            | Shape::Arc
            | Shape::ThreePointArc
            | Shape::Slot(_)
            | Shape::Polygon(_) => None,
        }
        .into_iter()
        .collect()
    }
}
fn guides(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    start: Point2,
    continued: Option<EntityId>,
) -> Vec<Guide> {
    let start_on_screen = screen.to_screen(start);
    let mut nearby: Vec<NearbyLine> = sketch
        .entities()
        .filter_map(|(line, entity)| {
            let Entity::Line {
                start: first,
                end: last,
            } = entity
            else {
                return None;
            };
            let (from, to) = sketch.line_endpoints(line)?;
            let along = (to - from).try_normalize()?;
            let (from, to) = (screen.to_screen(from)?, screen.to_screen(to)?);
            let distance = std::iter::once(pointer.screen)
                .chain(start_on_screen)
                .map(|at| at.distance(snap::closest_on_segment(from, to, at)))
                .fold(f64::INFINITY, f64::min);
            Some(NearbyLine {
                distance,
                line,
                along,
                continued: continued.is_some_and(|point| point == *first || point == *last),
            })
        })
        .collect();
    let by_distance = |a: &NearbyLine, b: &NearbyLine| a.distance.total_cmp(&b.distance);
    if nearby.len() > NEARBY_LINES {
        nearby.select_nth_unstable_by(NEARBY_LINES, by_distance);
        nearby.truncate(NEARBY_LINES);
    }
    nearby.sort_by(by_distance);
    let referenced = nearby.into_iter().flat_map(|nearby| {
        let parallel = (!nearby.continued).then_some(Guide {
            direction: Direction::Parallel(nearby.line),
            along: nearby.along,
        });
        let perpendicular = Guide {
            direction: Direction::Perpendicular(nearby.line),
            along: nearby.along.perp(),
        };
        parallel.into_iter().chain(std::iter::once(perpendicular))
    });
    LEVEL_AND_UPRIGHT.into_iter().chain(referenced).collect()
}

fn alignments(
    start: Point2,
    screen: &impl Screen,
    pointer: Pointer,
    guides: &[Guide],
) -> Vec<Aligned> {
    let Some(from) = screen.to_screen(start) else {
        return Vec::new();
    };
    let drawn = pointer.screen - from;
    if drawn.length() < MIN_ALIGN_LENGTH {
        return Vec::new();
    }
    let max_angle = ALIGN_ANGLE_DEGREES.to_radians();
    let mut found: Vec<(bool, f64, Aligned)> = guides
        .iter()
        .filter_map(|guide| {
            let position = start + guide.along * (pointer.sketch - start).dot(guide.along);
            let on_screen = screen.to_screen(position)?;
            let offset = on_screen.distance(pointer.screen);
            let angle = drawn.angle_to(on_screen - from).abs();
            let referenced = guide.direction.reference().is_some();
            let aligned = Aligned {
                guide: *guide,
                position,
            };
            (offset <= ALIGN_TOLERANCE || angle <= max_angle)
                .then_some((referenced, offset, aligned))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    found.into_iter().map(|(_, _, aligned)| aligned).collect()
}

fn align(
    start: Point2,
    screen: &impl Screen,
    pointer: Pointer,
    guides: &[Guide],
) -> Option<Placement> {
    alignments(start, screen, pointer, guides)
        .first()
        .map(|aligned| Placement::at(aligned.position, Snap::Aligned(aligned.guide.direction)))
}

fn aligned_on(
    sketch: &Sketch,
    screen: &impl Screen,
    pointer: Pointer,
    start: Point2,
    snapped: Snapped,
    guides: &[Guide],
) -> Option<Placement> {
    let on = |position: Point2, direction: Direction| {
        Placement::at(position, Snap::AlignedOn(snapped.target, direction))
    };
    match snapped.target {
        Target::Pending(_) => None,
        Target::Point(_)
        | Target::Midpoint(_)
        | Target::Intersection(..)
        | Target::Centre { .. } => {
            held(start, snapped.position, guides).map(|direction| on(snapped.position, direction))
        }
        Target::Curve(_) | Target::Extension(_) => alignments(start, screen, pointer, guides)
            .into_iter()
            .find_map(|aligned| {
                let crossing = snap::crossing_along(
                    sketch,
                    snapped.target,
                    start,
                    aligned.guide.along,
                    pointer.sketch,
                )?;
                let offset = screen.to_screen(crossing)?.distance(pointer.screen);
                let away = crossing.distance(start) >= DEGENERATE_LENGTH;
                (away && offset <= ALIGNED_CROSSING_TOLERANCE)
                    .then(|| on(crossing, aligned.guide.direction))
            }),
    }
}

fn held(start: Point2, end: Point2, guides: &[Guide]) -> Option<Direction> {
    let drawn = end - start;
    let length = drawn.length();
    if length < DEGENERATE_LENGTH {
        return None;
    }
    guides
        .iter()
        .find(|guide| guide.along.perp_dot(drawn).abs() <= HELD_TOLERANCE * length)
        .map(|guide| guide.direction)
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

fn landing_on_arc(center: Placement, start: Placement, placement: Placement) -> Option<Placement> {
    let radius = center.position.distance(start.position);
    let kept = placement.snap.target().is_some()
        && snap::on_circle(center.position, radius, placement.position);
    let end = if kept {
        placement
    } else {
        Placement::free(arc_end(
            center.position,
            start.position,
            placement.position,
        )?)
    };
    (end.position.distance(start.position) >= DEGENERATE_LENGTH).then_some(end)
}

fn placed_first(corners: &[Point2], placed: &[Placement]) -> Vec<Placement> {
    corners
        .iter()
        .enumerate()
        .map(|(index, corner)| {
            placed
                .get(index)
                .copied()
                .unwrap_or(Placement::free(*corner))
        })
        .collect()
}

struct Draft<'a> {
    feature: FeatureId,
    transaction: TransactionBuilder<'a>,
    shadow: Sketch,
    construction: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DrawnCurve {
    curve: EntityId,
    start: EntityId,
    end: EntityId,
}

struct Polygon {
    center: EntityId,
    first: DrawnCurve,
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
        match placement.snap.target() {
            Some(Target::Midpoint(curve)) => self.constrain(Constraint::Midpoint { point, curve }),
            Some(Target::Centre {
                corners: (first, second),
                ..
            }) => self.constrain(Constraint::Symmetric {
                first,
                second,
                about: point,
            }),
            Some(Target::Intersection(first, second)) => {
                self.constrain(Constraint::Coincident(point, first));
                self.constrain(Constraint::Coincident(point, second));
            }
            _ => {
                if let Some(target) = placement.snap.entity() {
                    self.constrain(Constraint::Coincident(point, target));
                }
            }
        }
        for constraint in placement.tracks.constraints(point) {
            self.constrain(constraint);
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
        if let Some(direction) = end.snap.direction() {
            self.constrain(direction.constraint(line));
        }
        (start, end_point)
    }

    fn joined_outline(&mut self, corners: &[Placement]) -> Vec<DrawnCurve> {
        let mut sides = Vec::with_capacity(corners.len());
        for (index, corner) in corners.iter().enumerate() {
            let next = corners
                .get((index + 1) % corners.len())
                .map_or(corner.position, |next| next.position);
            let start = self.point(*corner);
            let end = self.point(Placement::free(next));
            let curve = self.entity(Entity::Line { start, end });
            sides.push(DrawnCurve { curve, start, end });
        }
        for (index, side) in sides.iter().enumerate() {
            if let Some(next) = sides.get((index + 1) % sides.len()) {
                self.constrain(Constraint::Coincident(side.end, next.start));
            }
        }
        sides
    }

    fn level_rectangle(&mut self, corners: [Placement; 4]) -> Vec<DrawnCurve> {
        let sides = self.joined_outline(&corners);
        for (index, side) in sides.iter().enumerate() {
            self.constrain(if index % 2 == 0 {
                Constraint::Horizontal(side.curve)
            } else {
                Constraint::Vertical(side.curve)
            });
        }
        sides
    }

    fn rectangle(&mut self, corner: Placement, opposite: Placement) {
        let [_, second, _, fourth] = rectangle_corners(corner.position, opposite.position);
        self.level_rectangle([
            corner,
            Placement::free(second),
            opposite,
            Placement::free(fourth),
        ]);
    }

    fn centered_rectangle(&mut self, center: Placement, corner: Placement) {
        let opposite = shapes::mirrored(corner.position, center.position);
        let [_, second, third, fourth] = rectangle_corners(corner.position, opposite);
        let sides = self.level_rectangle([
            corner,
            Placement::free(second),
            Placement::free(third),
            Placement::free(fourth),
        ]);
        let center = self.point(center);
        if let (Some(first), Some(opposite)) = (sides.first(), sides.get(2)) {
            self.constrain(Constraint::Symmetric {
                first: first.start,
                second: opposite.start,
                about: center,
            });
        }
    }

    fn rectangle_on_side(&mut self, first: Placement, second: Placement, corners: [Point2; 4]) {
        let [_, _, third, fourth] = corners;
        let sides = self.joined_outline(&[
            first,
            second,
            Placement::free(third),
            Placement::free(fourth),
        ]);
        let [base, next, opposite, last] = sides.as_slice() else {
            return;
        };
        let (base, next, opposite, last) = (*base, *next, *opposite, *last);
        self.constrain(Constraint::Perpendicular(base.curve, next.curve));
        self.constrain(Constraint::Parallel(opposite.curve, base.curve));
        self.constrain(Constraint::Parallel(last.curve, next.curve));
        if let Some(direction) = second.snap.direction() {
            self.constrain(direction.constraint(base.curve));
        }
    }

    fn circle(&mut self, center: Placement, rim: Placement) {
        let radius = center.position.distance(rim.position);
        let center = self.point(center);
        let circle = self.entity(Entity::Circle { center, radius });
        if let Some(point) = point_target(rim.snap) {
            self.constrain(Constraint::Coincident(point, circle));
        }
    }

    fn circle_through(&mut self, circle: ArcGeometry, on: &[Placement]) -> EntityId {
        let center = self.entity(Entity::Point(circle.center));
        let id = self.entity(Entity::Circle {
            center,
            radius: circle.radius,
        });
        for point in on
            .iter()
            .filter_map(|placement| point_target(placement.snap))
        {
            self.constrain(Constraint::Coincident(point, id));
        }
        center
    }

    fn circle_on_diameter(&mut self, first: Placement, second: Placement, circle: ArcGeometry) {
        match (point_target(first.snap), point_target(second.snap)) {
            (Some(one_end), Some(other_end)) => {
                let center = self.circle_through(circle, &[first]);
                self.constrain(Constraint::Symmetric {
                    first: one_end,
                    second: other_end,
                    about: center,
                });
            }
            _ => {
                self.circle_through(circle, &[first, second]);
            }
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

    fn arc_through(&mut self, center: EntityId, start: Point2, end: Point2) -> DrawnCurve {
        let start = self.entity(Entity::Point(start));
        let end = self.entity(Entity::Point(end));
        let curve = self.entity(Entity::Arc { center, start, end });
        DrawnCurve { curve, start, end }
    }

    fn free_line(&mut self, start: Point2, end: Point2) -> DrawnCurve {
        let start = self.entity(Entity::Point(start));
        let end = self.entity(Entity::Point(end));
        let curve = self.entity(Entity::Line { start, end });
        DrawnCurve { curve, start, end }
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
    ) -> DrawnCurve {
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
        DrawnCurve {
            curve: arc,
            start,
            end,
        }
    }

    fn join(&mut self, joints: [(EntityId, EntityId); 4], tangents: [(EntityId, EntityId); 4]) {
        for (point, other) in joints {
            self.constrain(Constraint::Coincident(point, other));
        }
        for (first, second) in tangents {
            self.constrain(Constraint::Tangent(first, second));
        }
    }

    fn slot(&mut self, first: Placement, second: Placement, slot: &Slot) -> [EntityId; 2] {
        let first_center = self.point(first);
        let second_center = self.point(second);
        let [a, b, c, d] = slot.corners();
        let second_end = self.arc_through(second_center, a, b);
        let first_end = self.arc_through(first_center, c, d);
        let top = self.free_line(b, c);
        let bottom = self.free_line(d, a);
        self.join(
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
        self.constrain(Constraint::Equal(first_end.curve, second_end.curve));
        if let Some(direction) = second.snap.direction() {
            self.constrain(direction.constraint(top.curve));
        }
        [first_center, second_center]
    }

    fn centered_slot(&mut self, center: Placement, end: Placement, slot: &Slot) {
        let [mirrored, _] = slot.centers;
        let [first, second] = self.slot(Placement::free(mirrored), end, slot);
        let about = self.point(center);
        self.constrain(Constraint::Symmetric {
            first,
            second,
            about,
        });
    }

    fn arc_slot(&mut self, center: Placement, ends: [Placement; 2], slot: &ArcSlot) {
        let center = self.point(center);
        let [first, last] = ends;
        let [first_position, last_position] = slot.ends;
        let first_center = self.point(Placement {
            position: first_position,
            ..first
        });
        let last_center = self.point(Placement {
            position: last_position,
            ..last
        });
        let [outer_first, outer_last, inner_first, inner_last] = slot.corners();
        let outer = self.arc_through(center, outer_first, outer_last);
        let inner = self.arc_through(center, inner_first, inner_last);
        let first_end = self.arc_through(first_center, inner_first, outer_first);
        let last_end = self.arc_through(last_center, outer_last, inner_last);
        self.join(
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
    }

    fn regular_polygon(&mut self, center: Placement, corners: &[Placement]) -> Option<Polygon> {
        let radius = center.position.distance(corners.first()?.position);
        let center = self.point(center);
        let circle = self.entity_as(Entity::Circle { center, radius }, true);
        let sides = self.joined_outline(corners);
        let first = *sides.first()?;
        for side in &sides {
            self.constrain(Constraint::Coincident(side.start, circle));
            if side.curve != first.curve {
                self.constrain(Constraint::Equal(first.curve, side.curve));
            }
        }
        Some(Polygon { center, first })
    }

    fn polygon(&mut self, center: Placement, corners: &[Placement]) {
        self.regular_polygon(center, corners);
    }

    fn polygon_around_side_middle(
        &mut self,
        center: Placement,
        middle: Placement,
        corners: &[Point2],
    ) {
        let Some(polygon) = self.regular_polygon(center, &placed_first(corners, &[])) else {
            return;
        };
        let inscribed = self.entity_as(
            Entity::Circle {
                center: polygon.center,
                radius: center.position.distance(middle.position),
            },
            true,
        );
        self.constrain(Constraint::Tangent(polygon.first.curve, inscribed));
        if let Some(point) = point_target(middle.snap) {
            self.constrain(Constraint::Midpoint {
                point,
                curve: polygon.first.curve,
            });
        }
    }

    fn polygon_on_side(
        &mut self,
        first: Placement,
        second: Placement,
        center: Point2,
        corners: &[Point2],
    ) {
        let corners = placed_first(corners, &[first, second]);
        let Some(polygon) = self.regular_polygon(Placement::free(center), &corners) else {
            return;
        };
        if let Some(direction) = second.snap.direction() {
            self.constrain(direction.constraint(polygon.first.curve));
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

        let level = align(
            start,
            &screen,
            pointer(Point2::new(40.0, 11.0)),
            &LEVEL_AND_UPRIGHT,
        )
        .unwrap();
        assert_eq!(level.snap, Snap::Aligned(Direction::Horizontal));
        assert_eq!(level.position, Point2::new(40.0, 10.0));

        let upright = align(
            start,
            &screen,
            pointer(Point2::new(10.4, -20.0)),
            &LEVEL_AND_UPRIGHT,
        )
        .unwrap();
        assert_eq!(upright.snap, Snap::Aligned(Direction::Vertical));
        assert_eq!(upright.position, Point2::new(10.0, -20.0));

        let short = align(
            start,
            &screen,
            pointer(Point2::new(12.0, 10.5)),
            &LEVEL_AND_UPRIGHT,
        )
        .unwrap();
        assert_eq!(short.position, Point2::new(12.0, 10.0));

        assert_eq!(
            align(
                start,
                &screen,
                pointer(Point2::new(40.0, 13.0)),
                &LEVEL_AND_UPRIGHT
            ),
            None
        );
        assert_eq!(
            align(
                start,
                &screen,
                pointer(Point2::new(10.8, 10.3)),
                &LEVEL_AND_UPRIGHT
            ),
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
            context: Some((feature, Shape::Line)),
            placed: vec![Placement::free(Point2::new(10.0, 10.0))],
            ..Drawing::default()
        };
        let screen = Scaled(10.0);

        drawing.hover(&sketch, &screen, Some(pointer(Point2::new(40.0, 10.8))));
        assert_eq!(drawing.snap_entities(), vec![lone]);

        drawing.hover(&sketch, &screen, Some(pointer(Point2::new(30.0, 10.4))));
        assert_eq!(
            drawing.hover.map(|hover| hover.snap),
            Some(Snap::Aligned(Direction::Horizontal))
        );
        assert_eq!(drawing.snap_label(&sketch).as_deref(), Some("Horizontal"));

        drawing.hover(&sketch, &screen, Some(pointer(Point2::new(10.3, 10.2))));
        assert_eq!(drawing.snap_label(&sketch).as_deref(), Some("Stop here"));
        let preview = drawing.preview(Faceting::within(0.01));
        assert_eq!(preview.curves.len(), 1);
        assert_eq!(preview.snap, Some(Point2::new(10.0, 10.0)));
    }

    fn drawing_a_line(sketch: &Sketch, start: Placement) -> Drawing {
        let document = caditor_document::Document::default();
        let feature = document.transaction("Sketch").add_feature(
            "Sketch",
            caditor_document::FeatureKind::from(sketch.clone()),
        );
        Drawing {
            context: Some((feature, Shape::Line)),
            placed: vec![start],
            ..Drawing::default()
        }
    }

    fn hovered_at(drawing: &mut Drawing, sketch: &Sketch, at: Point2) -> Option<Placement> {
        drawing.hover(sketch, &Scaled(10.0), Some(pointer(at)));
        drawing.hover
    }

    fn crosses(a: Vector2, b: Vector2) -> f64 {
        a.normalize().perp_dot(b.normalize())
    }

    #[test]
    fn a_line_end_turns_parallel_or_perpendicular_to_a_nearby_line() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        let slanted = sketch.add_line(Point2::new(20.0, 20.0), Point2::new(50.0, 40.0));
        let start = Point2::new(60.0, 10.0);
        let mut drawing = drawing_a_line(&sketch, Placement::free(start));

        let parallel = hovered_at(&mut drawing, &sketch, Point2::new(90.0, 30.3)).unwrap();
        assert_eq!(parallel.snap, Snap::Aligned(Direction::Parallel(slanted)));
        assert!(crosses(parallel.position - start, Vector2::new(30.0, 20.0)).abs() < 1e-12);
        assert_eq!(
            drawing.snap_label(&sketch),
            Some(format!("Parallel to Line {slanted}"))
        );
        assert_eq!(drawing.snap_entities(), vec![slanted]);

        let square = hovered_at(&mut drawing, &sketch, Point2::new(40.0, 40.4)).unwrap();
        assert_eq!(
            square.snap,
            Snap::Aligned(Direction::Perpendicular(slanted))
        );
        assert!(
            (square.position - start)
                .dot(Vector2::new(30.0, 20.0))
                .abs()
                < 1e-9
        );
        assert_eq!(
            drawing.snap_label(&sketch),
            Some(format!("Perpendicular to Line {slanted}"))
        );

        let free = hovered_at(&mut drawing, &sketch, Point2::new(90.0, 33.0)).unwrap();
        assert_eq!(free.snap, Snap::Free);
        assert_eq!(drawing.snap_entities(), Vec::new());
    }

    #[test]
    fn level_and_upright_win_over_a_nearly_level_line() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        sketch.add_line(Point2::new(10.0, 30.0), Point2::new(40.0, 31.0));
        let mut drawing = drawing_a_line(&sketch, Placement::free(Point2::new(10.0, 10.0)));

        let level = hovered_at(&mut drawing, &sketch, Point2::new(40.0, 11.0)).unwrap();
        assert_eq!(level.snap, Snap::Aligned(Direction::Horizontal));
        assert_eq!(level.position, Point2::new(40.0, 10.0));
    }

    #[test]
    fn a_chained_line_turns_square_to_the_last_but_never_continues_it() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        let last = sketch.add_line(Point2::new(10.0, 10.0), Point2::new(40.0, 30.0));
        let end = sketch.entity(last).unwrap().points()[1];
        let joined = Placement::at(Point2::new(40.0, 30.0), Snap::Target(Target::Point(end)));
        let mut drawing = drawing_a_line(&sketch, joined);

        let straight_on = hovered_at(&mut drawing, &sketch, Point2::new(70.0, 50.3)).unwrap();
        assert_eq!(straight_on.snap, Snap::Free);

        let square = hovered_at(&mut drawing, &sketch, Point2::new(20.0, 60.4)).unwrap();
        assert_eq!(square.snap, Snap::Aligned(Direction::Perpendicular(last)));
    }

    #[test]
    fn a_line_ending_on_a_curve_keeps_its_direction_where_it_crosses() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        let slanted = sketch.add_line(Point2::new(0.0, 40.0), Point2::new(70.0, 70.0));
        let mut drawing = drawing_a_line(&sketch, Placement::free(Point2::new(30.0, 10.0)));
        let crossing = 40.0 + 30.0 * 30.0 / 70.0;

        let upright = hovered_at(&mut drawing, &sketch, Point2::new(30.3, crossing + 0.2)).unwrap();
        assert_eq!(
            upright.snap,
            Snap::AlignedOn(Target::Curve(slanted), Direction::Vertical)
        );
        assert!(upright.position.distance(Point2::new(30.0, crossing)) < 1e-12);
        assert_eq!(
            drawing.snap_label(&sketch),
            Some(format!("On Line {slanted}, vertical"))
        );
        assert_eq!(drawing.snap_entities(), vec![slanted]);
        assert_eq!(
            drawing.preview(Faceting::within(0.01)).snap,
            Some(upright.position)
        );
    }

    #[test]
    fn a_point_snap_keeps_a_direction_only_where_it_already_holds() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        let level = sketch.add_point(Point2::new(40.0, 10.0));
        let off = sketch.add_point(Point2::new(10.5, 40.0));
        let mut drawing = drawing_a_line(&sketch, Placement::free(Point2::new(10.0, 10.0)));

        let held = hovered_at(&mut drawing, &sketch, Point2::new(40.3, 10.2)).unwrap();
        assert_eq!(
            held.snap,
            Snap::AlignedOn(Target::Point(level), Direction::Horizontal)
        );
        assert_eq!(
            drawing.snap_label(&sketch),
            Some(format!("On Point {level}, horizontal"))
        );

        let not_held = hovered_at(&mut drawing, &sketch, Point2::new(10.4, 40.2)).unwrap();
        assert_eq!(not_held.snap, Snap::Target(Target::Point(off)));
        assert_eq!(not_held.position, Point2::new(10.5, 40.0));
    }

    #[test]
    fn a_rectangle_corner_tracks_a_point_hovered_before() {
        let mut sketch = Sketch::new(caditor_geometry::Plane::XY);
        let guide = sketch.add_point(Point2::new(40.0, 30.0));
        let mut drawing = drawing_a_line(&sketch, Placement::free(Point2::new(10.0, 5.0)));
        drawing.context = drawing
            .context
            .map(|(feature, _)| (feature, Shape::Rectangle(RectangleMode::Corners)));
        drawing.placed.clear();

        hovered_at(&mut drawing, &sketch, Point2::new(40.0, 30.0));
        drawing.placed = vec![Placement::free(Point2::new(10.0, 5.0))];
        let corner = hovered_at(&mut drawing, &sketch, Point2::new(40.2, 60.0)).unwrap();

        assert_eq!(corner.position, Point2::new(40.0, 60.0));
        assert_eq!(corner.tracks.points().collect::<Vec<_>>(), vec![guide]);
        assert_eq!(
            drawing.snap_label(&sketch),
            Some(format!("Vertical from Point {guide}"))
        );
        assert_eq!(
            drawing.preview(Faceting::within(0.01)).guides,
            vec![[Point2::new(40.0, 30.0), Point2::new(40.0, 60.0)]]
        );
    }
}
