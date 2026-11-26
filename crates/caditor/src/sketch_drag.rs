use std::{borrow::Cow, collections::BTreeSet, sync::Arc};

use caditor_document::{FeatureId, lettered_at};
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{
    Constraint, Drag, Entity, EntityId, EntityState, Faceting, GearPose, MAX_LENGTH, Sketch,
    SketchSolution, TextSize,
};
use thiserror::Error;

use crate::{
    body_snap::BodySnaps,
    commands::Reason,
    configuration_export::listed,
    drag_solver::{BodyJoin, DragCommand, Join},
    drawing::Preview,
    feature_tree::count,
    inference::Inferences,
    sketch_handles::wrapped,
    snap::{self, HELD_POINT_PULL, Hold, POINT_TOLERANCE, Pointer, Screen, Target},
    tracking::{self, Acquired, Landing, ShownOutlines, Tracks},
};

const SMALLEST_DRAGGED_RADIUS: f64 = 1e-3;
const SMALLEST_SNAPPED_RADIUS: f64 = 1e-3;
const KEPT_TOLERANCE: f64 = 1e-6;
pub const NOTHING_FREE: &str = "Everything in the sketch is fully constrained";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum SelectionError {
    #[error("The sketch has no geometry to select")]
    NothingToSelect,
    #[error(
        "What is still free is known once the sketch has solved; fix its problems or wait for it"
    )]
    NotSolved,
    #[error("{}", NOTHING_FREE)]
    NothingFree,
}

impl Reason for SelectionError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum MoveError {
    #[error("Switch to the Select tool to move geometry")]
    WhileDrawing,
    #[error("Select sketch geometry to move it")]
    NothingSelected,
    #[error(
        "Projected geometry follows the model geometry it comes from; keep the selected \
         projections as ordinary curves to move them"
    )]
    Projected,
}

impl Reason for MoveError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TransformError {
    #[error("Switch to the Select tool to {} geometry", .transform.verb().to_lowercase())]
    WhileDrawing { transform: Transform },
    #[error("Select sketch geometry to {} it", .transform.verb().to_lowercase())]
    NothingSelected { transform: Transform },
    #[error(
        "{} a lone point by selecting the point to {} it about with it",
        .transform.verb(),
        .transform.pivot_verb()
    )]
    NeedsPivot { transform: Transform },
    #[error(
        "Projected geometry follows the model geometry it comes from; keep the selected \
         projections as ordinary curves to {} them",
        .transform.verb().to_lowercase()
    )]
    Projected { transform: Transform },
    #[error("The factor must be a number above zero")]
    FactorNotPositive,
    #[error("Keep the geometry within {} m of the sketch's origin", MAX_LENGTH / 1_000.0)]
    TooFar,
}

impl Reason for TransformError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Handles {
    Points(Vec<(EntityId, Point2)>),
    Radius {
        circle: EntityId,
        centre: Point2,
        radius: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Grab {
    feature: FeatureId,
    label: String,
    from: Point2,
    handles: Handles,
    handle: Option<(EntityId, Point2)>,
    landed: Option<(EntityId, Point2)>,
    sent: Option<Vec<Drag>>,
    landing: Option<Landing>,
    joins: Vec<Constraint>,
    guides: Vec<[Point2; 2]>,
    acquired: Acquired,
    outlines: ShownOutlines,
    bodies: SharedSnaps,
    inferred: Inferences,
}

#[derive(Clone, Debug, Default)]
struct SharedSnaps(Arc<BodySnaps>);

impl PartialEq for SharedSnaps {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Grab {
    pub fn of(
        sketch: &Sketch,
        feature: FeatureId,
        grabbed: EntityId,
        selected: &[EntityId],
        from: Point2,
    ) -> Option<Self> {
        if !movable(sketch, grabbed) {
            return None;
        }

        let together: Vec<EntityId> = if selected.contains(&grabbed) {
            selected
                .iter()
                .copied()
                .filter(|entity| movable(sketch, *entity))
                .collect()
        } else {
            vec![grabbed]
        };
        let handles = match (together.as_slice(), sketch.entity(grabbed)) {
            ([_], Some(Entity::Circle { center, radius })) => Handles::Radius {
                circle: grabbed,
                centre: sketch.point(*center)?,
                radius: *radius,
            },
            _ => Handles::Points(
                points_of(sketch, &together)
                    .into_iter()
                    .filter(|(point, _)| !sketch.is_fixed(*point))
                    .collect(),
            ),
        };

        if matches!(&handles, Handles::Points(points) if points.is_empty()) {
            return None;
        }

        let handle = match &handles {
            Handles::Points(points) => points.iter().copied().min_by(|a, b| {
                a.1.distance_squared(from)
                    .total_cmp(&b.1.distance_squared(from))
            }),
            Handles::Radius { .. } => None,
        };

        let mut grab = Self {
            feature,
            label: format!("Drag {}", subject(sketch, &together)),
            from,
            handles,
            handle,
            landed: None,
            sent: None,
            landing: None,
            joins: Vec::new(),
            guides: Vec::new(),
            acquired: Acquired::default(),
            outlines: ShownOutlines::default(),
            bodies: SharedSnaps::default(),
            inferred: Inferences::default(),
        };

        grab.acquire_neighbours(sketch);
        Some(grab)
    }

    fn acquire_neighbours(&mut self, sketch: &Sketch) {
        let Some((handle, _)) = self.handle else {
            return;
        };
        let moving = self.moving_points();

        let neighbours: Vec<EntityId> = sketch
            .entities()
            .map(|(_, entity)| entity.points())
            .filter(|points| points.contains(&handle))
            .flatten()
            .filter(|point| !moving.contains(point))
            .collect();
        for point in neighbours.into_iter().rev() {
            self.acquired.point(point);
        }
    }

    fn moving_points(&self) -> Vec<EntityId> {
        match &self.handles {
            Handles::Points(points) => points.iter().map(|(point, _)| *point).collect(),
            Handles::Radius { .. } => Vec::new(),
        }
    }

    pub fn moving_with(&self, sketch: &Sketch) -> Vec<EntityId> {
        let points = self.moving_points();
        let curves: Vec<EntityId> = sketch
            .entities()
            .filter(|(_, entity)| entity.points().iter().any(|point| points.contains(point)))
            .map(|(id, _)| id)
            .collect();
        points.into_iter().chain(curves).collect()
    }

    pub fn landing(&self) -> Option<Landing> {
        self.landing
    }

    pub fn snapping_to_bodies(mut self, bodies: &Arc<BodySnaps>) -> Self {
        self.bodies = SharedSnaps(Arc::clone(bodies));
        self
    }

    pub fn inferring(mut self, inferred: Inferences) -> Self {
        self.inferred = inferred;
        self
    }

    pub fn snap_entities(&self, sketch: &Sketch) -> Vec<EntityId> {
        let Some(landing) = self.landing else {
            return Vec::new();
        };
        let moving = self.moving_with(sketch);
        let mut shown = Vec::new();

        for target in landing.target.into_iter().chain(landing.tracks.anchors()) {
            match target {
                Target::Centre { .. } | Target::Centroid(_) => {
                    self.outlines.show(target, &mut shown);
                }
                _ => shown.extend(target.involved()),
            }
        }
        shown.retain(|entity| !moving.contains(entity));
        shown.sort_unstable();
        shown.dedup();
        shown
    }

    pub fn landing_label(&self, sketch: &Sketch) -> Option<String> {
        let landing = self.landing?;
        match landing.target {
            Some(Target::Body(target)) => self.bodies.0.label(target),
            _ => landing.label(sketch),
        }
    }

    pub fn guides(&self) -> &[[Point2; 2]] {
        &self.guides
    }

    pub fn follow(
        &mut self,
        cursor: Point2,
        snapping: Option<(&Sketch, &impl Screen)>,
        hold: Option<Hold>,
        grid: Option<f64>,
    ) -> Option<DragCommand> {
        if let Handles::Radius {
            circle,
            centre,
            radius,
        } = self.handles
        {
            return self.follow_rim((circle, centre, radius), cursor, snapping, hold);
        }

        let offset = cursor - self.from;
        let landed = snapping.and_then(|(sketch, screen)| {
            Some((
                sketch,
                self.snapper_landing(sketch, screen, offset, hold, grid)?,
            ))
        });

        if let Some((sketch, (_, _, landing))) = landed
            && let Some(target) = landing.target
        {
            self.acquired.note(sketch, target, landing.position);
        }
        self.guides = landed
            .map(|(sketch, (_, _, landing))| landing.guides(sketch))
            .unwrap_or_default();
        self.joins = landed
            .map(|(sketch, (point, _, landing))| landing.joins(sketch, point))
            .unwrap_or_default();
        match landed {
            Some((sketch, (_, _, landing))) => self.outlines.keep(
                sketch,
                landing.target.into_iter().chain(landing.tracks.anchors()),
            ),
            None => self.outlines.keep_none(),
        }
        self.landing = landed.map(|(_, (_, _, landing))| landing);
        self.landed = landed.map(|(_, (point, original, _))| (point, original));

        let target = match (self.landing, self.landed) {
            (Some(landing), Some((_, original))) => self.from + (landing.position - original),
            _ => cursor,
        };
        self.to(target)
    }

    fn snapper_landing(
        &self,
        sketch: &Sketch,
        screen: &impl Screen,
        offset: Vector2,
        hold: Option<Hold>,
        grid: Option<f64>,
    ) -> Option<(EntityId, Point2, Landing)> {
        let (handle, from) = self.handle?;
        let ignored = self.moving_with(sketch);
        let pointer_at = |original: Point2| {
            let moved = original + offset;
            Some(Pointer {
                screen: screen.to_screen(moved)?,
                sketch: moved,
            })
        };

        let by_handle = pointer_at(from).and_then(|pointer| {
            let landing = tracking::land(
                sketch,
                screen,
                pointer,
                (&self.acquired, &ignored),
                hold,
                Some(&self.bodies.0),
                self.inferred,
            )?;
            Some((pointer, landing))
        });
        let handle_reach = by_handle
            .filter(|(_, landing)| {
                landing.on_grid || landing.target.is_some_and(Target::is_point_like)
            })
            .and_then(|(pointer, landing)| {
                Some(screen.to_screen(landing.position)?.distance(pointer.screen))
            });
        let reach = handle_reach.unwrap_or(if hold.is_some() {
            HELD_POINT_PULL
        } else {
            POINT_TOLERANCE
        });
        let Handles::Points(points) = &self.handles else {
            return None;
        };
        let moved: Vec<(EntityId, Pointer)> = points
            .iter()
            .filter(|(point, _)| *point != handle)
            .filter_map(|(point, original)| Some((*point, pointer_at(*original)?)))
            .collect();
        let moving: BTreeSet<EntityId> = ignored.iter().copied().collect();

        let by_other =
            snap::nearest_moved(sketch, screen, &moved, &moving, reach, hold, self.inferred)
                .and_then(|(point, snapped)| {
                    let (_, original) = points.iter().find(|(each, _)| *each == point)?;
                    let landing = Landing {
                        position: snapped.position,
                        target: Some(snapped.target),
                        tracks: Tracks::default(),
                        on_grid: false,
                    };
                    Some((point, *original, landing))
                });
        by_other
            .or_else(|| by_handle.map(|(_, landing)| (handle, from, landing)))
            .or_else(|| {
                let position = snap::nearby_grid_crossing(grid?, screen, pointer_at(from)?)?;
                let landing = Landing {
                    position,
                    target: None,
                    tracks: Tracks::default(),
                    on_grid: true,
                };
                Some((handle, from, landing))
            })
    }

    fn follow_rim(
        &mut self,
        (circle, centre, radius): (EntityId, Point2, f64),
        cursor: Point2,
        snapping: Option<(&Sketch, &impl Screen)>,
        hold: Option<Hold>,
    ) -> Option<DragCommand> {
        let wanted = self.cursor_radius(centre, radius, cursor);

        self.guides = Vec::new();
        self.outlines.keep_none();
        self.landing = snapping.and_then(|(sketch, screen)| {
            let Some(Entity::Circle { center, .. }) = sketch.entity(circle) else {
                return None;
            };
            let minimum = radius * SMALLEST_DRAGGED_RADIUS;
            let at = snap::closest_on_circle(centre, wanted.max(minimum), cursor)?;
            let pointer = Pointer {
                screen: screen.to_screen(at)?,
                sketch: at,
            };
            let reach = match hold {
                Some(hold) => hold
                    .grid
                    .and_then(|spacing| snap::grid_crossing(spacing, screen, pointer))
                    .map_or(HELD_POINT_PULL, |(offset, _)| HELD_POINT_PULL.min(offset)),
                None => POINT_TOLERANCE,
            };

            let snapped = snap::point_within(
                sketch,
                screen,
                pointer,
                &[*center],
                self.inferred,
                reach,
                &|point| {
                    let distance = centre.distance(point);
                    distance >= minimum && distance > SMALLEST_SNAPPED_RADIUS
                },
            )?;
            Some(Landing {
                position: snapped.position,
                target: Some(snapped.target),
                tracks: Tracks::default(),
                on_grid: false,
            })
        });
        match self.landing {
            Some(landing) => self.radius_to(centre.distance(landing.position)),
            None => self.radius_to(wanted),
        }
    }

    pub fn finish(&self) -> DragCommand {
        let join = match &self.handles {
            Handles::Radius { circle, .. } => {
                self.landing.and_then(|landing| match landing.target {
                    Some(Target::Point(point)) => Some(Join {
                        point,
                        at: landing.position,
                        constraints: vec![Constraint::Coincident(point, *circle)],
                        body: None,
                        rim_of: Some(*circle),
                    }),
                    _ => None,
                })
            }
            Handles::Points(_) => self
                .landed
                .zip(self.landing)
                .map(|((point, _), landing)| Join {
                    point,
                    at: landing.position,
                    constraints: self.joins.clone(),
                    body: match landing.target {
                        Some(Target::Body(target)) => self.bodies.0.source(target).map(|source| {
                            Box::new(BodyJoin {
                                projection: source.projection.clone(),
                                outline: source.outline.clone(),
                                part: target.part,
                            })
                        }),
                        _ => None,
                    },
                    rim_of: None,
                }),
        };
        DragCommand::Finish { join }
    }

    pub fn feature(&self) -> FeatureId {
        self.feature
    }

    pub fn to(&mut self, cursor: Point2) -> Option<DragCommand> {
        match &self.handles {
            Handles::Points(points) => {
                let drags = translated(points, cursor - self.from);
                self.send(drags)
            }
            Handles::Radius { centre, radius, .. } => {
                self.radius_to(self.cursor_radius(*centre, *radius, cursor))
            }
        }
    }

    fn cursor_radius(&self, centre: Point2, radius: f64, cursor: Point2) -> f64 {
        radius + (centre.distance(cursor) - centre.distance(self.from))
    }

    fn radius_to(&mut self, wanted: f64) -> Option<DragCommand> {
        let Handles::Radius { circle, radius, .. } = &self.handles else {
            return None;
        };

        let drags = vec![Drag::Radius {
            circle: *circle,
            to: wanted.max(radius * SMALLEST_DRAGGED_RADIUS),
        }];
        self.send(drags)
    }

    fn send(&mut self, drags: Vec<Drag>) -> Option<DragCommand> {
        if self.sent.as_ref() == Some(&drags) {
            return None;
        }
        self.sent = Some(drags.clone());
        Some(DragCommand::Move {
            feature: self.feature,
            label: self.label.clone(),
            drags,
        })
    }

    pub fn has_moved(&self) -> bool {
        self.sent.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Moving {
    pub feature: FeatureId,
    pub label: String,
    pub anchor: Point2,
    selected: Vec<EntityId>,
    moved: Vec<EntityId>,
    points: Vec<(EntityId, Point2)>,
}

impl Moving {
    pub fn offered(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
        drawing: bool,
    ) -> Result<Self, MoveError> {
        if drawing {
            Err(MoveError::WhileDrawing)
        } else {
            Self::of(sketch, feature, selected)
        }
    }

    pub fn of(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
    ) -> Result<Self, MoveError> {
        let entities: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|entity| movable(sketch, *entity))
            .collect();
        let points = points_of(sketch, &entities);
        let Some(&(_, anchor)) = points.first() else {
            return Err(if selects_projected(sketch, selected) {
                MoveError::Projected
            } else {
                MoveError::NothingSelected
            });
        };
        Ok(Self {
            feature,
            label: format!("Move {}", subject(sketch, &entities)),
            anchor,
            selected: selected.to_vec(),
            moved: entities,
            points,
        })
    }

    pub fn refreshed(&self, sketch: &Sketch) -> Result<Self, MoveError> {
        Self::of(sketch, self.feature, &self.selected)
    }

    pub fn selected(&self) -> &[EntityId] {
        &self.selected
    }

    pub fn to(&self, target: Point2) -> Vec<DragCommand> {
        vec![
            DragCommand::Move {
                feature: self.feature,
                label: self.label.clone(),
                drags: translated(&self.points, target - self.anchor),
            },
            DragCommand::Finish { join: None },
        ]
    }

    pub fn preview(&self, sketch: &Sketch, target: Option<Point2>, faceting: Faceting) -> Preview {
        let mut preview = Preview {
            snap: Some(self.anchor),
            ..Preview::default()
        };
        let Some(target) = target else {
            return preview;
        };

        preview.guides.push([self.anchor, target]);
        ghost(
            sketch,
            &self.moved,
            Placement::Shift(target - self.anchor),
            faceting,
            &mut preview,
        );
        preview
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Rotate,
    Scale,
}

impl Transform {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
        }
    }

    fn pivot_verb(self) -> &'static str {
        match self {
            Self::Rotate => "turn",
            Self::Scale => "scale",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transforming {
    pub feature: FeatureId,
    pub transform: Transform,
    pub label: String,
    pub pivot: Point2,
    selected: Vec<EntityId>,
    moved: Vec<EntityId>,
    points: Vec<(EntityId, Point2)>,
    circles: Vec<(EntityId, f64)>,
    poses: Vec<Drag>,
}

impl Transforming {
    pub fn offered(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
        drawing: bool,
        transform: Transform,
    ) -> Result<Self, TransformError> {
        if drawing {
            Err(TransformError::WhileDrawing { transform })
        } else {
            Self::of(sketch, feature, selected, transform)
        }
    }

    pub fn of(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
        transform: Transform,
    ) -> Result<Self, TransformError> {
        let present: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|entity| {
                sketch.entity(*entity).is_some()
                    || sketch.text(*entity).is_some()
                    || sketch.gear(*entity).is_some()
                    || *entity == EntityId::ORIGIN
            })
            .collect();
        let curve_points: BTreeSet<EntityId> = present
            .iter()
            .flat_map(|entity| match sketch.entity(*entity) {
                Some(entity) => entity.points(),
                None => sketch
                    .text(*entity)
                    .map(|text| text.anchor)
                    .or_else(|| sketch.gear(*entity).map(|gear| gear.centre))
                    .into_iter()
                    .collect(),
            })
            .collect();

        let lone_points: Vec<EntityId> = present
            .iter()
            .copied()
            .filter(|entity| {
                (*entity == EntityId::ORIGIN
                    || matches!(sketch.entity(*entity), Some(Entity::Point(_))))
                    && !curve_points.contains(entity)
            })
            .collect();
        let pivot_point = match lone_points.as_slice() {
            [only] if present.len() > 1 => Some(*only),
            _ => None,
        };
        let moved: Vec<EntityId> = present
            .iter()
            .copied()
            .filter(|entity| movable(sketch, *entity) && Some(*entity) != pivot_point)
            .collect();
        let points = points_of(sketch, &moved);

        if points.is_empty() {
            return Err(if selects_projected(sketch, selected) {
                TransformError::Projected { transform }
            } else {
                TransformError::NothingSelected { transform }
            });
        }

        let circles = circles_of(sketch, &moved);
        let poses = poses_of(sketch, &moved);

        let pivot = match pivot_point.and_then(|point| sketch.point(point)) {
            Some(pivot) => pivot,
            None if points.len() == 1 && circles.is_empty() && poses.is_empty() => {
                return Err(TransformError::NeedsPivot { transform });
            }
            None => extent_centre(sketch, &points, &circles),
        };
        Ok(Self {
            feature,
            transform,
            label: format!("{} {}", transform.verb(), subject(sketch, &moved)),
            pivot,
            selected: selected.to_vec(),
            moved,
            points,
            circles,
            poses,
        })
    }

    pub fn selected(&self) -> &[EntityId] {
        &self.selected
    }

    pub fn refreshed(&self, sketch: &Sketch) -> Result<Self, TransformError> {
        Self::of(sketch, self.feature, &self.selected, self.transform)
    }

    pub fn rotated(&self, degrees: f64) -> Result<Vec<DragCommand>, TransformError> {
        self.commands(Placement::Turn {
            pivot: self.pivot,
            degrees,
        })
    }

    pub fn scaled(&self, factor: f64) -> Result<Vec<DragCommand>, TransformError> {
        if !(factor.is_finite() && factor > 0.0) {
            return Err(TransformError::FactorNotPositive);
        }
        self.commands(Placement::Scale {
            pivot: self.pivot,
            factor,
        })
    }

    pub fn preview(&self, sketch: &Sketch, amount: Option<f64>, faceting: Faceting) -> Preview {
        let mut preview = Preview {
            snap: Some(self.pivot),
            ..Preview::default()
        };
        let Some(placement) = amount.and_then(|amount| self.placement(amount)) else {
            return preview;
        };

        ghost(sketch, &self.moved, placement, faceting, &mut preview);
        preview
    }

    fn placement(&self, amount: f64) -> Option<Placement> {
        let pivot = self.pivot;
        match self.transform {
            Transform::Rotate if amount.is_finite() => Some(Placement::Turn {
                pivot,
                degrees: amount,
            }),
            Transform::Scale if amount.is_finite() && amount > 0.0 => Some(Placement::Scale {
                pivot,
                factor: amount,
            }),
            Transform::Rotate | Transform::Scale => None,
        }
    }

    fn commands(&self, placement: Placement) -> Result<Vec<DragCommand>, TransformError> {
        let drags = placement.drags(&self.points, &self.circles, &self.poses);

        if !within_limits(&drags) {
            return Err(TransformError::TooFar);
        }
        Ok(vec![
            DragCommand::Move {
                feature: self.feature,
                label: self.label.clone(),
                drags,
            },
            DragCommand::Finish { join: None },
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Placement {
    Shift(Vector2),
    Turn { pivot: Point2, degrees: f64 },
    Scale { pivot: Point2, factor: f64 },
}

impl Placement {
    pub(crate) fn point(self, at: Point2) -> Point2 {
        match self {
            Self::Shift(offset) => at + offset,
            Self::Turn { pivot, degrees } => {
                pivot + Vector2::from_angle(degrees.to_radians()).rotate(at - pivot)
            }
            Self::Scale { pivot, factor } => pivot + (at - pivot) * factor,
        }
    }

    fn turn_and_factor(self) -> Option<(f64, f64)> {
        match self {
            Self::Shift(_) => None,
            Self::Turn { degrees, .. } => Some((degrees, 1.0)),
            Self::Scale { factor, .. } => Some((0.0, factor)),
        }
    }

    pub(crate) fn drags(
        self,
        points: &[(EntityId, Point2)],
        circles: &[(EntityId, f64)],
        poses: &[Drag],
    ) -> Vec<Drag> {
        let moved = points.iter().map(|(point, at)| Drag::Point {
            point: *point,
            to: self.point(*at),
        });
        let radii = match self {
            Self::Scale { factor, .. } => circles
                .iter()
                .map(|(circle, radius)| Drag::Radius {
                    circle: *circle,
                    to: radius * factor,
                })
                .collect(),
            Self::Shift(_) | Self::Turn { .. } => Vec::new(),
        };
        moved.chain(radii).chain(self.posed(poses)).collect()
    }

    fn posed(self, poses: &[Drag]) -> Vec<Drag> {
        let Some((degrees, factor)) = self.turn_and_factor() else {
            return Vec::new();
        };
        poses
            .iter()
            .filter_map(|pose| match *pose {
                Drag::Text { text, to } => Some(Drag::Text {
                    text,
                    to: TextSize {
                        height: to.height * factor,
                        angle: wrapped(to.angle + degrees),
                    },
                }),
                Drag::Gear { gear, to } => Some(Drag::Gear {
                    gear,
                    to: GearPose {
                        pitch_radius: to.pitch_radius * factor,
                        turn: wrapped(to.turn + degrees),
                    },
                }),
                Drag::Point { .. } | Drag::Radius { .. } => None,
            })
            .collect()
    }
}

pub(crate) fn poses_of(sketch: &Sketch, entities: &[EntityId]) -> Vec<Drag> {
    entities
        .iter()
        .filter_map(|entity| {
            if let Some(size) = sketch.text(*entity).and(sketch.text_size(*entity)) {
                return Some(Drag::Text {
                    text: *entity,
                    to: size,
                });
            }

            let values = sketch.gear(*entity).and(sketch.gear_values(*entity))?;
            Some(Drag::Gear {
                gear: *entity,
                to: GearPose {
                    pitch_radius: values.shape.pitch_radius(),
                    turn: values.turn,
                },
            })
        })
        .collect()
}

pub(crate) fn within_limits(drags: &[Drag]) -> bool {
    drags.iter().all(|drag| match drag {
        Drag::Point { to, .. } => to.is_finite() && to.abs().max_element() <= MAX_LENGTH,
        Drag::Radius { to, .. } => to.is_finite() && *to > 0.0 && *to <= MAX_LENGTH,
        Drag::Text { to, .. } => {
            to.height.is_finite() && to.height > 0.0 && to.height <= MAX_LENGTH
        }
        Drag::Gear { to, .. } => {
            to.pitch_radius.is_finite() && to.pitch_radius > 0.0 && to.pitch_radius <= MAX_LENGTH
        }
    })
}

pub(crate) fn ghost(
    sketch: &Sketch,
    moved: &[EntityId],
    placement: Placement,
    faceting: Faceting,
    preview: &mut Preview,
) {
    let poses = sketch
        .poses(&placement.posed(&poses_of(sketch, moved)))
        .unwrap_or_default();

    for entity in moved {
        if let Some(Entity::Point(at)) = sketch.entity(*entity) {
            preview.points.push(placement.point(*at));
        } else if let Some(points) = sketch.faceted(*entity, faceting) {
            preview.curves.push(
                points
                    .into_iter()
                    .map(|point| placement.point(point))
                    .collect(),
            );
        } else if let Some((text, size)) = poses
            .text(*entity)
            .or_else(|| sketch.text(*entity).zip(sketch.text_size(*entity)))
        {
            let placed = sketch
                .point(text.anchor)
                .and_then(|anchor| lettered_at(text, placement.point(anchor), size));

            preview.curves.extend(
                placed
                    .map(|placed| placed.polylines(faceting))
                    .unwrap_or_default(),
            );
        } else if let Some(gear) = sketch.gear(*entity) {
            let values = poses
                .gear(*entity)
                .map(|(_, values)| values)
                .or_else(|| sketch.gear_values(*entity));
            let outline = values
                .zip(sketch.point(gear.centre))
                .and_then(|(values, centre)| {
                    values
                        .shape
                        .outline_turned(placement.point(centre), values.turn.to_radians())
                        .ok()
                });

            preview.curves.extend(
                outline
                    .map(|outline| outline.faceted(faceting))
                    .unwrap_or_default(),
            );
        }
    }
}

pub(crate) fn kept(solved: &Sketch, drags: &[Drag]) -> Vec<String> {
    drags
        .iter()
        .flat_map(|drag| match *drag {
            Drag::Text { text, to } => kept_of_text(solved, text, to),
            Drag::Gear { gear, to } => kept_of_gear(solved, gear, to),
            Drag::Point { .. } | Drag::Radius { .. } => Vec::new(),
        })
        .collect()
}

pub(crate) fn kept_notice(label: &str, kept: &[String]) -> Option<String> {
    (!kept.is_empty()).then(|| format!("{label} kept {}.", listed(kept)))
}

fn kept_of_text(solved: &Sketch, text: EntityId, to: TextSize) -> Vec<String> {
    let Some(size) = solved.text_size(text) else {
        return Vec::new();
    };
    let label = solved.entity_label(text);
    let mut kept = Vec::new();

    if differs(size.height, to.height) {
        kept.push(format!("the height of {label}, which a parameter sets"));
    }
    if wrapped(to.angle - size.angle).abs() > KEPT_TOLERANCE {
        kept.push(format!("the angle of {label}, which a parameter sets"));
    }
    kept
}

fn kept_of_gear(solved: &Sketch, gear: EntityId, to: GearPose) -> Vec<String> {
    let (Some(held), Some(values)) = (solved.gear(gear), solved.gear_values(gear)) else {
        return Vec::new();
    };
    let label = solved.entity_label(gear);
    let size_reason = if held.is_sprocket() {
        "its chain sets"
    } else if held.meshing.is_some() {
        "the gear it meshes with sets"
    } else {
        "a parameter sets"
    };
    let turn_reason = match &held.meshing {
        Some(meshing) if meshing.direction.parameters().is_empty() => {
            "the gear it meshes with sets"
        }
        Some(_) | None => "a parameter sets",
    };
    let mut kept = Vec::new();

    if differs(values.shape.pitch_radius(), to.pitch_radius) {
        kept.push(format!("the size of {label}, which {size_reason}"));
    }
    if wrapped(to.turn - values.turn).abs() > KEPT_TOLERANCE {
        kept.push(format!("the turn of {label}, which {turn_reason}"));
    }
    kept
}

fn differs(held: f64, wanted: f64) -> bool {
    (wanted - held).abs() > KEPT_TOLERANCE * held.abs().max(1.0)
}

pub(crate) fn circles_of(sketch: &Sketch, entities: &[EntityId]) -> Vec<(EntityId, f64)> {
    entities
        .iter()
        .filter_map(|entity| match sketch.entity(*entity) {
            Some(
                Entity::Circle { radius, .. }
                | Entity::Ellipse {
                    minor_radius: radius,
                    ..
                }
                | Entity::EllipticalArc {
                    minor_radius: radius,
                    ..
                },
            ) => Some((*entity, *radius)),
            _ => None,
        })
        .collect()
}

pub(crate) fn extent_centre(
    sketch: &Sketch,
    points: &[(EntityId, Point2)],
    circles: &[(EntityId, f64)],
) -> Point2 {
    let rims = circles.iter().filter_map(|(circle, radius)| {
        let (centre, half) = match sketch.entity(*circle) {
            Some(Entity::Circle { center, .. }) => {
                (sketch.point(*center)?, Vector2::splat(*radius))
            }
            Some(
                Entity::Ellipse { center, major, .. } | Entity::EllipticalArc { center, major, .. },
            ) => {
                let centre = sketch.point(*center)?;
                let reach = sketch.point(*major)? - centre;
                (centre, ellipse_half_extent(reach, *radius))
            }
            _ => return None,
        };
        Some([centre - half, centre + half])
    });
    let (low, high) = points.iter().map(|(_, at)| [*at, *at]).chain(rims).fold(
        (
            Point2::splat(f64::INFINITY),
            Point2::splat(f64::NEG_INFINITY),
        ),
        |(low, high), [from, to]| (low.min(from), high.max(to)),
    );
    (low + high) * 0.5
}

fn ellipse_half_extent(major: Vector2, minor_radius: f64) -> Vector2 {
    let Some(direction) = major.try_normalize() else {
        return Vector2::splat(minor_radius);
    };
    let along = major.length();
    Vector2::new(
        (along * direction.x).hypot(minor_radius * direction.y),
        (along * direction.y).hypot(minor_radius * direction.x),
    )
}

pub(crate) fn movable(sketch: &Sketch, entity: EntityId) -> bool {
    !sketch.is_fixed(entity)
        && (sketch.entity(entity).is_some()
            || sketch.text(entity).is_some()
            || sketch.gear(entity).is_some())
}

pub(crate) fn selects_projected(sketch: &Sketch, selected: &[EntityId]) -> bool {
    selected.iter().any(|entity| sketch.is_projected(*entity))
}

pub(crate) fn subject(sketch: &Sketch, entities: &[EntityId]) -> String {
    match entities {
        [only] => sketch.entity_label(*only),
        entities => count(entities.len(), "item", "items"),
    }
}

pub(crate) fn points_of(sketch: &Sketch, entities: &[EntityId]) -> Vec<(EntityId, Point2)> {
    let mut seen = BTreeSet::new();
    entities
        .iter()
        .flat_map(|entity| match sketch.entity(*entity) {
            Some(Entity::Point(_)) => vec![*entity],
            Some(other) => other.points(),
            None => sketch
                .text(*entity)
                .map(|text| text.anchor)
                .or_else(|| sketch.gear(*entity).map(|gear| gear.centre))
                .into_iter()
                .collect(),
        })
        .filter(|point| seen.insert(*point))
        .filter_map(|point| Some((point, sketch.point(point)?)))
        .collect()
}

fn translated(points: &[(EntityId, Point2)], offset: Vector2) -> Vec<Drag> {
    Placement::Shift(offset).drags(points, &[], &[])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxMode {
    Window,
    Crossing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenBox {
    pub from: Vector2,
    pub to: Vector2,
}

impl ScreenBox {
    pub fn mode(self) -> BoxMode {
        if self.to.x >= self.from.x {
            BoxMode::Window
        } else {
            BoxMode::Crossing
        }
    }

    fn min(self) -> Vector2 {
        self.from.min(self.to)
    }

    fn max(self) -> Vector2 {
        self.from.max(self.to)
    }

    pub fn contains(self, point: Vector2) -> bool {
        let (min, max) = (self.min(), self.max());
        point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y
    }

    pub fn crosses(self, from: Vector2, to: Vector2) -> bool {
        if self.contains(from) || self.contains(to) {
            return true;
        }

        let (min, max) = (self.min(), self.max());

        let corners = [
            min,
            Vector2::new(max.x, min.y),
            max,
            Vector2::new(min.x, max.y),
        ];
        (0..4).any(|index| {
            let start = corners.get(index).copied().unwrap_or(min);
            let end = corners.get((index + 1) % 4).copied().unwrap_or(min);
            segments_cross(from, to, start, end)
        })
    }
}

const LASSO_STEP: f64 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub enum ScreenArea {
    Box(ScreenBox),
    Lasso(Vec<Vector2>),
}

impl ScreenArea {
    pub fn starting_at(point: Vector2, lasso: bool) -> Self {
        if lasso {
            Self::Lasso(vec![point])
        } else {
            Self::Box(ScreenBox {
                from: point,
                to: point,
            })
        }
    }

    pub fn reach(&mut self, point: Vector2) {
        match self {
            Self::Box(area) => area.to = point,
            Self::Lasso(points) => {
                if points
                    .last()
                    .is_none_or(|last| last.distance(point) >= LASSO_STEP)
                {
                    points.push(point);
                }
            }
        }
    }

    pub fn mode(&self) -> BoxMode {
        match self {
            Self::Box(area) => area.mode(),
            Self::Lasso(_) => BoxMode::Window,
        }
    }

    pub fn bounds(&self) -> (Vector2, Vector2) {
        match self {
            Self::Box(area) => (area.min(), area.max()),
            Self::Lasso(points) => points.iter().fold(
                (
                    Vector2::splat(f64::INFINITY),
                    Vector2::splat(f64::NEG_INFINITY),
                ),
                |(low, high), point| (low.min(*point), high.max(*point)),
            ),
        }
    }

    pub fn centre(&self) -> Vector2 {
        let (low, high) = self.bounds();
        (low + high) * 0.5
    }

    pub fn contains(&self, point: Vector2) -> bool {
        match self {
            Self::Box(area) => area.contains(point),
            Self::Lasso(points) => {
                let mut inside = false;

                for (index, current) in points.iter().enumerate() {
                    let previous = points
                        .get(index.checked_sub(1).unwrap_or(points.len() - 1))
                        .copied()
                        .unwrap_or(*current);
                    if (current.y > point.y) != (previous.y > point.y) {
                        let across = previous.x
                            + (point.y - previous.y) / (current.y - previous.y)
                                * (current.x - previous.x);
                        if point.x < across {
                            inside = !inside;
                        }
                    }
                }
                inside
            }
        }
    }

    pub fn crosses(&self, from: Vector2, to: Vector2) -> bool {
        match self {
            Self::Box(area) => area.crosses(from, to),
            Self::Lasso(points) => {
                self.contains(from)
                    || self.contains(to)
                    || points
                        .iter()
                        .zip(points.iter().cycle().skip(1))
                        .any(|(start, end)| segments_cross(from, to, *start, *end))
            }
        }
    }
}

fn segments_cross(a: Vector2, b: Vector2, c: Vector2, d: Vector2) -> bool {
    let side = |p: Vector2, q: Vector2, r: Vector2| (q - p).perp_dot(r - p);
    let (first, second) = (side(a, b, c), side(a, b, d));
    let (third, fourth) = (side(c, d, a), side(c, d, b));

    if first == 0.0 && second == 0.0 && third == 0.0 && fourth == 0.0 {
        return collinear_segments_overlap(a, b, c, d);
    }
    first * second <= 0.0 && third * fourth <= 0.0
}

fn collinear_segments_overlap(a: Vector2, b: Vector2, c: Vector2, d: Vector2) -> bool {
    let along_x = (a - b).x.abs().max((c - d).x.abs()) >= (a - b).y.abs().max((c - d).y.abs());
    let coordinate = |p: Vector2| if along_x { p.x } else { p.y };
    let span = |p: Vector2, q: Vector2| {
        (
            coordinate(p).min(coordinate(q)),
            coordinate(p).max(coordinate(q)),
        )
    };
    let ((low, high), (other_low, other_high)) = (span(a, b), span(c, d));
    low <= other_high && other_low <= high
}

fn kept_pieces(outline: &[Point2], screen: &impl Screen) -> Option<Vec<(Vector2, Vector2)>> {
    let segments: Vec<(Point2, Point2)> = match outline {
        [only] => vec![(*only, *only)],
        _ => outline
            .windows(2)
            .filter_map(|pair| match pair {
                [from, to] => Some((*from, *to)),
                _ => None,
            })
            .collect(),
    };
    segments
        .into_iter()
        .filter_map(|(from, to)| screen.kept(from, to))
        .map(|(from, to)| Some((screen.to_screen(from)?, screen.to_screen(to)?)))
        .collect()
}

pub fn within(
    sketch: &Sketch,
    screen: &impl Screen,
    area: &ScreenArea,
    faceting: Faceting,
) -> Vec<EntityId> {
    let mode = area.mode();
    let caught = sketch
        .entities()
        .filter(|(id, _)| !id.is_reference())
        .filter(|(id, entity)| match entity {
            Entity::Point(position) => {
                screen.is_shown(*position)
                    && screen
                        .to_screen(*position)
                        .is_some_and(|point| area.contains(point))
            }
            _ => {
                let outline = sketch.faceted(*id, faceting).unwrap_or_default();
                let pieces = kept_pieces(&outline, screen);
                pieces.is_some_and(|pieces| match mode {
                    BoxMode::Window => {
                        !pieces.is_empty()
                            && pieces
                                .iter()
                                .all(|(from, to)| area.contains(*from) && area.contains(*to))
                    }
                    BoxMode::Crossing => pieces.iter().any(|(from, to)| area.crosses(*from, *to)),
                })
            }
        })
        .map(|(id, _)| id);
    let point_in = |point: EntityId| {
        sketch.point(point).is_some_and(|position| {
            screen.is_shown(position)
                && screen
                    .to_screen(position)
                    .is_some_and(|at| area.contains(at))
        })
    };
    let texts = sketch
        .texts()
        .filter(|(_, text)| point_in(text.anchor))
        .map(|(id, _)| id);
    let gears = sketch
        .gears()
        .filter(|(_, gear)| point_in(gear.centre))
        .map(|(id, _)| id);
    selectable(sketch, caught.chain(texts).chain(gears))
}

pub fn select_all(sketch: &Sketch) -> Result<Vec<EntityId>, SelectionError> {
    let everything = everything(sketch);
    if everything.is_empty() {
        Err(SelectionError::NothingToSelect)
    } else {
        Ok(everything)
    }
}

pub fn select_free(
    sketch: &Sketch,
    projected: &BTreeSet<EntityId>,
    solution: Option<&SketchSolution>,
) -> Result<Vec<EntityId>, SelectionError> {
    let solution = solution.ok_or(SelectionError::NotSolved)?;
    let free = selectable(
        sketch,
        sketch
            .entities()
            .map(|(id, _)| id)
            .filter(|id| is_free(*id, projected, solution)),
    );
    if free.is_empty() {
        Err(SelectionError::NothingFree)
    } else {
        Ok(free)
    }
}

pub fn can_select_free(
    sketch: &Sketch,
    projected: &BTreeSet<EntityId>,
    solution: Option<&SketchSolution>,
) -> Result<(), SelectionError> {
    let solution = solution.ok_or(SelectionError::NotSolved)?;
    if sketch
        .entities()
        .any(|(id, _)| is_free(id, projected, solution))
    {
        Ok(())
    } else {
        Err(SelectionError::NothingFree)
    }
}

fn is_free(id: EntityId, projected: &BTreeSet<EntityId>, solution: &SketchSolution) -> bool {
    !id.is_reference()
        && !projected.contains(&id)
        && solution.entity_state(id) != Some(EntityState::FullyConstrained)
}

pub fn can_select_all(sketch: &Sketch) -> Result<(), SelectionError> {
    if sketch.entities().any(|(id, _)| !id.is_reference())
        || sketch.texts().next().is_some()
        || sketch.gears().next().is_some()
    {
        Ok(())
    } else {
        Err(SelectionError::NothingToSelect)
    }
}

pub fn everything(sketch: &Sketch) -> Vec<EntityId> {
    selectable(
        sketch,
        sketch
            .entities()
            .map(|(id, _)| id)
            .filter(|id| !id.is_reference())
            .chain(sketch.texts().map(|(id, _)| id))
            .chain(sketch.gears().map(|(id, _)| id)),
    )
}

fn selectable(sketch: &Sketch, caught: impl Iterator<Item = EntityId>) -> Vec<EntityId> {
    let caught: Vec<EntityId> = caught.collect();
    let owned: BTreeSet<EntityId> = caught
        .iter()
        .flat_map(|id| match sketch.entity(*id) {
            Some(entity) => entity.points(),
            None => sketch
                .text(*id)
                .map(|text| text.anchor)
                .or_else(|| sketch.gear(*id).map(|gear| gear.centre))
                .into_iter()
                .collect(),
        })
        .collect();
    caught
        .into_iter()
        .filter(|id| !owned.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use caditor_document::{Document, FeatureKind};
    use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
    use caditor_geometry::Plane;
    use caditor_sketch::{Gear, Text, TextAlign};

    use super::*;
    use crate::snap::{Target, tests::Scaled};

    struct Flat;

    impl Screen for Flat {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            Some(Vector2::new(point.x, -point.y))
        }
    }

    fn feature() -> FeatureId {
        let mut document = Document::default();

        let mut transaction = document.transaction("Add sketch");

        let feature = transaction.add_feature("Sketch", FeatureKind::from(Sketch::new(Plane::XY)));

        document.apply(transaction.finish()).unwrap();
        feature
    }

    fn ends(sketch: &Sketch, line: EntityId) -> (EntityId, EntityId) {
        let points = sketch.entity(line).unwrap().points();
        (points[0], points[1])
    }

    #[test]
    fn collinear_segments_cross_only_where_they_overlap() {
        let at = |x: f64, y: f64| Vector2::new(x, y);

        assert!(!segments_cross(
            at(0.0, 0.0),
            at(1.0, 1.0),
            at(2.0, 2.0),
            at(3.0, 3.0)
        ));
        assert!(!segments_cross(
            at(0.0, 0.0),
            at(0.0, 1.0),
            at(0.0, 2.0),
            at(0.0, 3.0)
        ));
        assert!(segments_cross(
            at(0.0, 0.0),
            at(2.0, 2.0),
            at(1.0, 1.0),
            at(3.0, 3.0)
        ));
        assert!(segments_cross(
            at(0.0, 0.0),
            at(1.0, 1.0),
            at(1.0, 1.0),
            at(2.0, 2.0)
        ));
        assert!(segments_cross(
            at(3.0, 3.0),
            at(0.0, 0.0),
            at(1.0, 1.0),
            at(2.0, 2.0)
        ));
        assert!(segments_cross(
            at(0.0, 0.0),
            at(2.0, 0.0),
            at(1.0, -1.0),
            at(1.0, 1.0)
        ));
        assert!(!segments_cross(
            at(0.0, 0.0),
            at(1.0, 0.0),
            at(2.0, -1.0),
            at(2.0, 1.0)
        ));
        assert!(!segments_cross(
            at(1.0, 1.0),
            at(1.0, 1.0),
            at(2.0, 2.0),
            at(3.0, 3.0)
        ));
        assert!(segments_cross(
            at(2.0, 2.0),
            at(2.0, 2.0),
            at(1.0, 1.0),
            at(3.0, 3.0)
        ));
    }

    #[test]
    fn the_cheap_availability_checks_agree_with_the_selections_they_stand_for() {
        let mut sketch = Sketch::new(Plane::XY);
        let nothing = BTreeSet::new();

        assert_eq!(can_select_all(&sketch), select_all(&sketch).map(|_| ()));
        assert_eq!(
            can_select_free(&sketch, &nothing, None),
            select_free(&sketch, &nothing, None).map(|_| ())
        );
        assert_eq!(
            can_select_free(&sketch, &nothing, None),
            Err(SelectionError::NotSolved)
        );

        sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));

        assert_eq!(can_select_all(&sketch), select_all(&sketch).map(|_| ()));
        assert_eq!(can_select_all(&sketch), Ok(()));
    }

    #[test]
    fn a_grabbed_curve_moves_all_its_points_by_the_pointer_offset() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let (start, end) = ends(&sketch, line);
        let mut grab = Grab::of(&sketch, feature(), line, &[], Point2::new(5.0, 0.0)).unwrap();

        let Some(DragCommand::Move { drags, label, .. }) = grab.to(Point2::new(7.0, 3.0)) else {
            panic!("the first move is sent");
        };

        assert_eq!(label, format!("Drag {}", sketch.entity_label(line)));
        assert_eq!(
            drags,
            vec![
                Drag::Point {
                    point: start,
                    to: Point2::new(2.0, 3.0)
                },
                Drag::Point {
                    point: end,
                    to: Point2::new(12.0, 3.0)
                },
            ]
        );
        assert!(grab.to(Point2::new(7.0, 3.0)).is_none());
        assert!(grab.has_moved());
    }

    #[test]
    fn a_grabbed_circle_lands_its_rim_on_a_point_and_is_held_there() {
        let mut sketch = Sketch::new(Plane::XY);

        let circle = sketch.add_circle(Point2::ZERO, 5.0);
        let corner = sketch.add_point(Point2::new(30.0, 40.0));
        let mut grab = Grab::of(&sketch, feature(), circle, &[], Point2::new(5.0, 0.0)).unwrap();

        let Some(DragCommand::Move { drags, .. }) = grab.follow(
            Point2::new(30.05, 40.02),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        ) else {
            panic!("the radius is dragged");
        };

        assert_eq!(drags, vec![Drag::Radius { circle, to: 50.0 }]);
        assert_eq!(
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: corner,
                    at: Point2::new(30.0, 40.0),
                    constraints: vec![Constraint::Coincident(corner, circle)],
                    body: None,
                    rim_of: Some(circle),
                }),
            }
        );

        let Some(DragCommand::Move { drags, .. }) = grab.follow(
            Point2::new(60.0, 0.0),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        ) else {
            panic!("the radius is dragged");
        };

        assert_eq!(drags, vec![Drag::Radius { circle, to: 60.0 }]);
        assert_eq!(grab.finish(), DragCommand::Finish { join: None });
    }

    #[test]
    fn a_rim_drag_snaps_from_the_grab_offset_and_skips_points_inside_its_radius_floor() {
        let mut sketch = Sketch::new(Plane::XY);

        let circle = sketch.add_circle(Point2::ZERO, 1000.0);

        sketch.add_point(Point2::new(0.95, 0.0));

        let target = sketch.add_point(Point2::new(1.4, 0.0));
        let mut grab = Grab::of(&sketch, feature(), circle, &[], Point2::new(1001.0, 0.0)).unwrap();
        let screen = Scaled(10.0);

        let Some(DragCommand::Move { drags, .. }) =
            grab.follow(Point2::new(2.05, 0.0), Some((&sketch, &screen)), None, None)
        else {
            panic!("the radius is dragged");
        };

        assert_eq!(drags, vec![Drag::Radius { circle, to: 1.4 }]);
        assert_eq!(grab.landing().unwrap().target, Some(Target::Point(target)));
    }

    #[test]
    fn a_grabbed_circle_changes_its_radius_and_a_selected_one_moves_with_the_rest() {
        let mut sketch = Sketch::new(Plane::XY);

        let circle = sketch.add_circle(Point2::ZERO, 5.0);
        let point = sketch.add_point(Point2::new(20.0, 0.0));
        let centre = sketch.center_of(circle).unwrap();
        let feature = feature();

        let mut alone =
            Grab::of(&sketch, feature, circle, &[point], Point2::new(5.0, 0.0)).unwrap();

        let Some(DragCommand::Move { drags, .. }) = alone.to(Point2::new(0.0, 8.0)) else {
            panic!("the radius is dragged");
        };

        assert_eq!(drags, vec![Drag::Radius { circle, to: 8.0 }]);

        let mut together = Grab::of(
            &sketch,
            feature,
            circle,
            &[circle, point],
            Point2::new(5.0, 0.0),
        )
        .unwrap();

        let Some(DragCommand::Move { drags, label, .. }) = together.to(Point2::new(6.0, 1.0))
        else {
            panic!("the selection is dragged");
        };

        assert_eq!(label, "Drag 2 items");
        assert_eq!(
            drags,
            vec![
                Drag::Point {
                    point: centre,
                    to: Point2::new(1.0, 1.0)
                },
                Drag::Point {
                    point,
                    to: Point2::new(21.0, 1.0)
                },
            ]
        );
        assert!(Grab::of(&sketch, feature, EntityId::ORIGIN, &[], Point2::ZERO).is_none());
    }

    #[test]
    fn a_lasso_takes_what_lies_inside_its_outline_even_where_it_bends_in() {
        let mut sketch = Sketch::new(Plane::XY);

        let inside = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(2.0, 1.0));
        let in_the_notch = sketch.add_line(Point2::new(5.0, 6.0), Point2::new(6.0, 6.0));
        let mut lasso = ScreenArea::starting_at(Vector2::new(0.0, 0.0), true);

        for (x, y) in [(10.0, 0.0), (10.0, -10.0), (5.0, -3.0), (0.0, -10.0)] {
            lasso.reach(Vector2::new(x, y));
        }

        let caught = within(&sketch, &Flat, &lasso, Faceting::within(0.01));

        assert_eq!(lasso.mode(), BoxMode::Window);
        assert!(lasso.contains(Vector2::new(1.5, -1.0)));
        assert!(!lasso.contains(Vector2::new(5.0, -6.0)));
        assert!(lasso.crosses(Vector2::new(5.0, -6.0), Vector2::new(1.5, -1.0)));
        assert!(!lasso.crosses(Vector2::new(20.0, 0.0), Vector2::new(20.0, -5.0)));
        assert!(caught.contains(&inside));
        assert!(!caught.contains(&in_the_notch));
    }

    #[test]
    fn a_window_takes_what_lies_inside_and_a_crossing_box_what_it_touches() {
        let mut sketch = Sketch::new(Plane::XY);

        let inside = sketch.add_line(Point2::new(1.0, 1.0), Point2::new(4.0, 1.0));
        let across = sketch.add_line(Point2::new(3.0, 3.0), Point2::new(30.0, 3.0));
        let lone = sketch.add_point(Point2::new(2.0, 4.0));
        let circle = sketch.add_circle(Point2::new(50.0, 0.0), 2.0);
        let (across_start, across_end) = ends(&sketch, across);
        let window = ScreenBox {
            from: Vector2::new(0.0, -5.0),
            to: Vector2::new(6.0, 0.0),
        };
        let crossing = ScreenBox {
            from: window.to,
            to: window.from,
        };

        assert_eq!(window.mode(), BoxMode::Window);
        assert_eq!(
            within(
                &sketch,
                &Flat,
                &ScreenArea::Box(window),
                Faceting::within(0.01)
            ),
            vec![inside, across_start, lone]
        );
        assert_eq!(
            within(
                &sketch,
                &Flat,
                &ScreenArea::Box(crossing),
                Faceting::within(0.01)
            ),
            vec![inside, across, lone]
        );

        let all = everything(&sketch);

        assert_eq!(all, vec![inside, across, lone, circle]);
        assert!(!all.contains(&across_end));
    }

    #[test]
    fn a_selection_turns_about_a_lone_selected_point_or_else_about_its_centre() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::new(2.0, 0.0), Point2::new(6.0, 0.0));
        let (start, end) = ends(&sketch, line);
        let pivot = sketch.add_point(Point2::new(0.0, 0.0));
        let circle = sketch.add_circle(Point2::new(10.0, 0.0), 2.0);
        let feature = feature();

        let about_point =
            Transforming::of(&sketch, feature, &[line, pivot], Transform::Rotate).unwrap();
        let about_centre =
            Transforming::of(&sketch, feature, &[line, circle], Transform::Scale).unwrap();
        let commands = about_point.rotated(90.0).unwrap();
        let scaled = about_centre.scaled(0.5).unwrap();

        assert_eq!(about_point.pivot, Point2::ZERO);
        assert_eq!(about_centre.pivot, Point2::new(7.0, 0.0));

        let Some(DragCommand::Move { drags, .. }) = commands.first() else {
            panic!("a rotation moves the points");
        };
        let at = |point: EntityId| {
            drags.iter().find_map(|drag| match drag {
                Drag::Point { point: dragged, to } if *dragged == point => Some(*to),
                _ => None,
            })
        };

        assert!(at(start).unwrap().distance(Point2::new(0.0, 2.0)) < 1e-12);
        assert!(at(end).unwrap().distance(Point2::new(0.0, 6.0)) < 1e-12);
        assert!(at(pivot).is_none());

        let Some(DragCommand::Move { drags, .. }) = scaled.first() else {
            panic!("a scale moves the points");
        };

        assert!(drags.contains(&Drag::Radius { circle, to: 1.0 }));
        assert!(drags.contains(&Drag::Point {
            point: start,
            to: Point2::new(4.5, 0.0)
        }));
        assert!(about_centre.scaled(0.0).is_err());
        assert!(about_centre.scaled(f64::NAN).is_err());
        assert!(Transforming::of(&sketch, feature, &[pivot], Transform::Rotate).is_err());
        assert!(Transforming::of(&sketch, feature, &[EntityId::ORIGIN], Transform::Scale).is_err());
    }

    #[test]
    fn a_lone_ellipse_turns_and_scales_about_its_centre_not_its_major_end() {
        let mut sketch = Sketch::new(Plane::XY);

        let level = sketch.add_ellipse(Point2::ZERO, Point2::new(10.0, 0.0), 4.0);
        let slanted = sketch.add_ellipse(Point2::new(20.0, 5.0), Point2::new(23.0, 9.0), 2.0);
        let feature = feature();

        for (ellipse, centre) in [(level, Point2::ZERO), (slanted, Point2::new(20.0, 5.0))] {
            for transform in [Transform::Rotate, Transform::Scale] {
                let transforming =
                    Transforming::of(&sketch, feature, &[ellipse], transform).unwrap();

                assert!(transforming.pivot.distance(centre) < 1e-12);
            }
        }
    }

    #[test]
    fn moving_the_selection_puts_its_first_point_on_the_target() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::new(1.0, 2.0), Point2::new(5.0, 2.0));
        let (start, end) = ends(&sketch, line);
        let feature = feature();

        assert!(Moving::of(&sketch, feature, &[EntityId::ORIGIN]).is_err());

        let moving = Moving::of(&sketch, feature, &[line]).unwrap();

        assert_eq!(moving.anchor, Point2::new(1.0, 2.0));

        let commands = moving.to(Point2::new(0.0, 0.0));

        assert_eq!(
            commands,
            vec![
                DragCommand::Move {
                    feature,
                    label: format!("Move {}", sketch.entity_label(line)),
                    drags: vec![
                        Drag::Point {
                            point: start,
                            to: Point2::ZERO
                        },
                        Drag::Point {
                            point: end,
                            to: Point2::new(4.0, 0.0)
                        },
                    ],
                },
                DragCommand::Finish { join: None },
            ]
        );
    }

    #[test]
    fn projected_geometry_is_left_out_of_moves_and_refused_alone() {
        let mut sketch = Sketch::new(Plane::XY);

        let projected = sketch.add_line(Point2::new(1.0, 2.0), Point2::new(5.0, 2.0));
        let (projected_start, projected_end) = ends(&sketch, projected);
        let free = sketch.add_line(Point2::new(0.0, 6.0), Point2::new(3.0, 6.0));
        let (free_start, free_end) = ends(&sketch, free);
        let feature = feature();

        for id in [projected, projected_start, projected_end] {
            sketch.set_projected(id, true).unwrap();
        }

        let both = [projected, free];
        let moving = Moving::of(&sketch, feature, &both).unwrap();
        let turned = Transforming::of(&sketch, feature, &both, Transform::Rotate).unwrap();

        assert_eq!(
            Moving::of(&sketch, feature, &[projected]),
            Err(MoveError::Projected)
        );

        for transform in [Transform::Rotate, Transform::Scale] {
            assert_eq!(
                Transforming::of(&sketch, feature, &[projected], transform),
                Err(TransformError::Projected { transform })
            );
        }

        assert_eq!(moving.anchor, Point2::new(0.0, 6.0));
        assert_eq!(
            moving.points,
            vec![
                (free_start, Point2::new(0.0, 6.0)),
                (free_end, Point2::new(3.0, 6.0))
            ]
        );
        assert_eq!(turned.pivot, Point2::new(1.5, 6.0));
    }

    struct BesideProjected {
        sketch: Sketch,
        free: EntityId,
        free_ends: (EntityId, EntityId),
        projected: EntityId,
        projected_ends: (EntityId, EntityId),
    }

    fn beside_projected() -> BesideProjected {
        let mut sketch = Sketch::new(Plane::XY);

        let free = sketch.add_line(Point2::new(0.0, 6.0), Point2::new(30.0, 6.0));
        let projected = sketch.add_line(Point2::new(14.0, 5.0), Point2::new(16.0, 5.0));
        let free_ends = ends(&sketch, free);
        let projected_ends = ends(&sketch, projected);

        for id in [projected, projected_ends.0, projected_ends.1] {
            sketch.set_projected(id, true).unwrap();
        }

        BesideProjected {
            sketch,
            free,
            free_ends,
            projected,
            projected_ends,
        }
    }

    #[test]
    fn a_group_grab_takes_a_moving_point_as_its_handle_and_sends_no_projected_point() {
        let BesideProjected {
            sketch,
            free,
            free_ends: (start, end),
            projected,
            projected_ends: (projected_start, projected_end),
        } = beside_projected();

        let mut grab = Grab::of(
            &sketch,
            feature(),
            free,
            &[free, projected],
            Point2::new(15.0, 6.0),
        )
        .unwrap();

        let handle = grab.handle.map(|(point, _)| point).unwrap();

        assert!(handle == start || handle == end);

        let Some(DragCommand::Move { drags, .. }) = grab.to(Point2::new(16.0, 7.0)) else {
            panic!("the selection is dragged");
        };

        assert_eq!(
            drags,
            vec![
                Drag::Point {
                    point: start,
                    to: Point2::new(1.0, 7.0)
                },
                Drag::Point {
                    point: end,
                    to: Point2::new(31.0, 7.0)
                },
            ]
        );

        let moving = grab.moving_with(&sketch);

        for stays in [projected, projected_start, projected_end] {
            assert!(!moving.contains(&stays));
        }
    }

    #[test]
    fn a_group_grab_lands_a_moving_point_on_what_is_projected() {
        let BesideProjected {
            sketch,
            free,
            free_ends: (_, end),
            projected,
            projected_ends: (_, projected_end),
        } = beside_projected();

        let mut grab = Grab::of(
            &sketch,
            feature(),
            free,
            &[free, projected],
            Point2::new(15.0, 6.0),
        )
        .unwrap();

        let command = grab.follow(
            Point2::new(1.0, 5.05),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        );

        assert!(command.is_some());
        assert_eq!(
            grab.landing().and_then(|landing| landing.target),
            Some(Target::Point(projected_end))
        );
        assert_eq!(grab.landed.map(|(point, _)| point), Some(end));
    }

    #[test]
    fn a_grabbed_projection_is_refused_and_a_selection_of_only_projections_has_nothing_to_grab() {
        let BesideProjected {
            sketch,
            free,
            projected,
            projected_ends: (projected_start, _),
            ..
        } = beside_projected();

        assert!(
            Grab::of(
                &sketch,
                feature(),
                projected,
                &[projected, free],
                Point2::new(15.0, 5.0)
            )
            .is_none()
        );
        assert!(
            Grab::of(
                &sketch,
                feature(),
                projected_start,
                &[],
                Point2::new(14.0, 5.0)
            )
            .is_none()
        );
    }

    fn text_at(sketch: &mut Sketch, at: Point2) -> (EntityId, EntityId) {
        let anchor = sketch.add_point(at);
        let text = sketch
            .add_text(Text {
                anchor,
                content: "Vent".to_owned(),
                height: Expression::measure(5.0, Unit::Millimetre),
                angle: Expression::measure(0.0, Unit::Degree),
                bold: false,
                italic: false,
                align: TextAlign::Left,
            })
            .unwrap();

        (text, anchor)
    }

    fn gear_at(sketch: &mut Sketch, at: Point2) -> (EntityId, EntityId) {
        let centre = sketch.add_point(at);
        let gear = sketch
            .add_parametric_gear(Gear {
                centre,
                module: Expression::measure(2.0, Unit::Millimetre),
                teeth: Expression::number(20.0),
                pressure_angle: Expression::measure(20.0, Unit::Degree),
                profile_shift: Expression::number(0.0),
                root_fillet: Expression::measure(0.5, Unit::Millimetre),
                bore: Expression::measure(6.0, Unit::Millimetre),
                turn: Expression::measure(0.0, Unit::Degree),
                meshing: None,
                chain: None,
            })
            .unwrap();

        (gear, centre)
    }

    #[test]
    fn a_selected_text_or_gear_grabbed_with_a_selected_line_moves_with_it() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
        let (start, end) = ends(&sketch, line);
        let (text, anchor) = text_at(&mut sketch, Point2::new(0.0, 20.0));
        let (gear, centre) = gear_at(&mut sketch, Point2::new(50.0, 50.0));

        let selected = [line, text, gear];

        for grabbed in [text, gear] {
            let mut grab = Grab::of(
                &sketch,
                feature(),
                grabbed,
                &selected,
                Point2::new(1.0, 1.0),
            )
            .unwrap();

            let Some(DragCommand::Move { drags, label, .. }) = grab.to(Point2::new(3.0, 2.0))
            else {
                panic!("the selection is dragged");
            };

            assert_eq!(label, "Drag 3 items");
            assert_eq!(
                drags,
                vec![
                    Drag::Point {
                        point: start,
                        to: Point2::new(2.0, 1.0)
                    },
                    Drag::Point {
                        point: end,
                        to: Point2::new(12.0, 1.0)
                    },
                    Drag::Point {
                        point: anchor,
                        to: Point2::new(2.0, 21.0)
                    },
                    Drag::Point {
                        point: centre,
                        to: Point2::new(52.0, 51.0)
                    },
                ]
            );
        }

        let mut alone = Grab::of(&sketch, feature(), text, &[line], Point2::new(1.0, 1.0)).unwrap();

        let Some(DragCommand::Move { drags, .. }) = alone.to(Point2::new(3.0, 2.0)) else {
            panic!("the text is dragged");
        };

        assert_eq!(
            drags,
            vec![Drag::Point {
                point: anchor,
                to: Point2::new(2.0, 21.0)
            }]
        );
    }

    struct Mixed {
        sketch: Sketch,
        line: EntityId,
        projected: EntityId,
        text: EntityId,
        gear: EntityId,
    }

    fn mixed() -> Mixed {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
        let projected = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(10.0, -10.0));
        let (projected_start, projected_end) = ends(&sketch, projected);
        let (text, _) = text_at(&mut sketch, Point2::new(0.0, 20.0));
        let (gear, _) = gear_at(&mut sketch, Point2::new(50.0, 50.0));

        for id in [projected, projected_start, projected_end] {
            sketch.set_projected(id, true).unwrap();
        }
        Mixed {
            sketch,
            line,
            projected,
            text,
            gear,
        }
    }

    fn near_segment(point: Point2, from: Point2, to: Point2) -> bool {
        let along = to - from;
        let share =
            ((point - from).dot(along) / along.length_squared().max(1e-300)).clamp(0.0, 1.0);
        point.distance(from + along * share) < 0.02
    }

    fn close_curves(drawn: &[Vec<Point2>], expected: &[Vec<Point2>]) -> bool {
        drawn.len() == expected.len()
            && drawn.iter().zip(expected).all(|(drawn, expected)| {
                drawn.iter().all(|drawn| {
                    expected.windows(2).any(|pair| match pair {
                        [from, to] => near_segment(*drawn, *from, *to),
                        _ => false,
                    })
                })
            })
    }

    #[test]
    fn the_typed_ghost_draws_the_line_text_and_gear_moved_and_leaves_the_projection_out() {
        let Mixed {
            sketch,
            line,
            projected,
            text,
            gear,
        } = mixed();
        let feature = feature();
        let faceting = Faceting::within(0.01);

        let selected = [line, projected, text, gear];
        let moving = Moving::of(&sketch, feature, &selected).unwrap();
        let turning = Transforming::of(&sketch, feature, &selected, Transform::Rotate).unwrap();
        let scaling = Transforming::of(&sketch, feature, &selected, Transform::Scale).unwrap();
        let letters = caditor_document::placed_text(&sketch, text)
            .unwrap()
            .polylines(faceting);
        let teeth = sketch.toothed_outline(gear).unwrap().faceted(faceting);

        let expected = |placement: Placement| -> Vec<Vec<Point2>> {
            [sketch.faceted(line, faceting).unwrap()]
                .into_iter()
                .chain(letters.iter().cloned())
                .chain(teeth.iter().cloned())
                .map(|curve| {
                    curve
                        .into_iter()
                        .map(|point| placement.point(point))
                        .collect()
                })
                .collect()
        };

        let moved = moving.preview(&sketch, Some(Point2::new(5.0, 5.0)), faceting);
        let turned = turning.preview(&sketch, Some(90.0), faceting);
        let scaled = scaling.preview(&sketch, Some(2.0), faceting);

        assert!(close_curves(
            &moved.curves,
            &expected(Placement::Shift(Vector2::new(5.0, 5.0)))
        ));
        assert!(close_curves(
            &turned.curves,
            &expected(Placement::Turn {
                pivot: turning.pivot,
                degrees: 90.0
            })
        ));
        assert!(moved.points.is_empty());
        assert_eq!(
            scaled.curves.len(),
            1 + letters.len()
                + sketch
                    .toothed_outline(gear)
                    .unwrap()
                    .faceted(faceting)
                    .len()
        );

        let bounds = |curves: &[Vec<Point2>]| {
            curves.iter().flatten().fold(
                (
                    Point2::splat(f64::INFINITY),
                    Point2::splat(f64::NEG_INFINITY),
                ),
                |(low, high), point| (low.min(*point), high.max(*point)),
            )
        };
        let scale = Placement::Scale {
            pivot: scaling.pivot,
            factor: 2.0,
        };
        let (low, high) = bounds(&teeth);
        let (scaled_low, scaled_high) = bounds(&scaled.curves[1 + letters.len()..]);
        let (letters_low, letters_high) = bounds(&letters);
        let (scaled_letters_low, scaled_letters_high) = bounds(&scaled.curves[1..=letters.len()]);

        assert!(scaled_low.distance(scale.point(low)) < 0.05);
        assert!(scaled_high.distance(scale.point(high)) < 0.05);
        assert!(scaled_letters_low.distance(scale.point(letters_low)) < 0.05);
        assert!(scaled_letters_high.distance(scale.point(letters_high)) < 0.05);
    }

    fn moved_by(commands: &[DragCommand]) -> Vec<Drag> {
        commands
            .iter()
            .find_map(|command| match command {
                DragCommand::Move { drags, .. } => Some(drags.clone()),
                DragCommand::Finish { .. } | DragCommand::Cancel => None,
            })
            .unwrap()
    }

    fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
        Err(EvalError::ParameterMissing)
    }

    #[test]
    fn rotating_turns_a_selected_text_and_gear_and_scaling_doubles_their_size() {
        let Mixed {
            sketch,
            line,
            projected,
            text,
            gear,
        } = mixed();
        let feature = feature();

        let selected = [line, projected, text, gear];
        let turning = Transforming::of(&sketch, feature, &selected, Transform::Rotate).unwrap();
        let scaling = Transforming::of(&sketch, feature, &selected, Transform::Scale).unwrap();
        let turned = moved_by(&turning.rotated(90.0).unwrap());
        let scaled = moved_by(&scaling.scaled(2.0).unwrap());

        assert!(turned.contains(&Drag::Text {
            text,
            to: TextSize {
                height: 5.0,
                angle: 90.0
            }
        }));
        assert!(turned.contains(&Drag::Gear {
            gear,
            to: GearPose {
                pitch_radius: 20.0,
                turn: 90.0
            }
        }));
        assert!(scaled.contains(&Drag::Text {
            text,
            to: TextSize {
                height: 10.0,
                angle: 0.0
            }
        }));
        assert!(scaled.contains(&Drag::Gear {
            gear,
            to: GearPose {
                pitch_radius: 40.0,
                turn: 0.0
            }
        }));

        let solve = |drags: &[Drag]| {
            sketch
                .solve_geometry_from(&no_parameters, &|| false, drags, None)
                .unwrap()
                .0
        };
        let turned_sketch = solve(&turned);
        let scaled_sketch = solve(&scaled);
        let measure = |expression: &Expression| expression.evaluate(&no_parameters).unwrap().value;

        assert!((measure(&turned_sketch.text(text).unwrap().angle) - 90.0).abs() < 1e-9);
        assert!((measure(&turned_sketch.gear(gear).unwrap().turn) - 90.0).abs() < 1e-9);
        assert!((measure(&scaled_sketch.text(text).unwrap().height) - 10.0).abs() < 1e-9);
        assert!((measure(&scaled_sketch.gear(gear).unwrap().module) - 4.0).abs() < 1e-9);
        assert!(kept(&turned_sketch, &turned).is_empty());
        assert!(kept(&scaled_sketch, &scaled).is_empty());

        let alone = Transforming::of(&sketch, feature, &[text], Transform::Rotate).unwrap();

        assert_eq!(
            Some(alone.pivot),
            sketch.point(sketch.text(text).unwrap().anchor)
        );
    }

    #[test]
    fn a_text_height_a_parameter_sets_is_kept_by_a_scale_and_said_so() {
        let mut sketch = Sketch::new(Plane::XY);

        let height = ParameterId::from_raw(1);
        let (text, _) = text_at(&mut sketch, Point2::new(0.0, 20.0));
        let line = sketch.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
        let values = |id| {
            if id == height {
                Ok(Quantity::length(5.0))
            } else {
                Err(EvalError::ParameterMissing)
            }
        };

        let driven = Text {
            height: Expression::Parameter(height),
            ..sketch.text(text).unwrap().clone()
        };

        sketch.replace_text(text, driven).unwrap();

        let sketch = sketch.solve(&values, &|| false).unwrap().geometry;
        let scaling =
            Transforming::of(&sketch, feature(), &[line, text], Transform::Scale).unwrap();
        let drags = moved_by(&scaling.scaled(2.0).unwrap());

        let scaled = sketch
            .solve_geometry_from(&values, &|| false, &drags, None)
            .unwrap()
            .0;
        let kept = kept(&scaled, &drags);

        assert_eq!(
            scaled.text(text).unwrap().height,
            Expression::Parameter(height)
        );
        assert_eq!(
            kept_notice("Scale 2 items", &kept).as_deref(),
            Some(
                format!(
                    "Scale 2 items kept the height of {}, which a parameter sets.",
                    scaled.entity_label(text)
                )
                .as_str()
            )
        );
    }

    #[test]
    fn a_lone_grabbed_point_lands_on_its_snap_and_joins_it_when_released() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
        let (_, end) = ends(&sketch, line);
        let lone = sketch.add_point(Point2::new(10.0, 10.0));
        let mut grab = Grab::of(&sketch, feature(), end, &[], Point2::new(4.1, 0.1)).unwrap();

        let command = grab.follow(
            Point2::new(10.3, 9.8),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        );

        assert_eq!(grab.moving_with(&sketch), vec![end, line]);
        assert_eq!(
            command,
            Some(DragCommand::Move {
                feature: feature(),
                label: format!("Drag {}", sketch.entity_label(end)),
                drags: vec![Drag::Point {
                    point: end,
                    to: Point2::new(10.0, 10.0),
                }],
            })
        );
        assert_eq!(
            grab.landing().and_then(|landing| landing.target),
            Some(Target::Point(lone))
        );
        assert_eq!(
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: end,
                    at: Point2::new(10.0, 10.0),
                    constraints: vec![Constraint::Coincident(end, lone)],
                    body: None,
                    rim_of: None,
                }),
            }
        );
    }

    #[test]
    fn a_grabbed_line_lands_its_nearer_end_on_a_snap_and_moves_the_rest_with_it() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let (start, end) = ends(&sketch, line);
        let lone = sketch.add_point(Point2::new(30.0, 20.0));
        let mut grab = Grab::of(&sketch, feature(), line, &[], Point2::new(8.0, 0.0)).unwrap();

        let Some(DragCommand::Move { drags, .. }) = grab.follow(
            Point2::new(28.05, 20.02),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        ) else {
            panic!("the line is dragged");
        };

        assert_eq!(
            drags,
            vec![
                Drag::Point {
                    point: start,
                    to: Point2::new(20.0, 20.0)
                },
                Drag::Point {
                    point: end,
                    to: Point2::new(30.0, 20.0)
                },
            ]
        );
        assert_eq!(
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: end,
                    at: Point2::new(30.0, 20.0),
                    constraints: vec![Constraint::Coincident(end, lone)],
                    body: None,
                    rim_of: None,
                }),
            }
        );
    }

    #[test]
    fn a_grabbed_line_lands_whichever_end_comes_near_a_point_not_only_its_handle() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let (start, end) = ends(&sketch, line);
        let lone = sketch.add_point(Point2::new(30.0, 20.0));
        let mut grab = Grab::of(&sketch, feature(), line, &[], Point2::new(1.0, 0.0)).unwrap();

        let Some(DragCommand::Move { drags, .. }) = grab.follow(
            Point2::new(21.05, 20.02),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        ) else {
            panic!("the line is dragged");
        };

        assert_eq!(
            drags,
            vec![
                Drag::Point {
                    point: start,
                    to: Point2::new(20.0, 20.0)
                },
                Drag::Point {
                    point: end,
                    to: Point2::new(30.0, 20.0)
                },
            ]
        );
        assert_eq!(
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: end,
                    at: Point2::new(30.0, 20.0),
                    constraints: vec![Constraint::Coincident(end, lone)],
                    body: None,
                    rim_of: None,
                }),
            }
        );
    }

    #[test]
    fn the_moving_point_nearest_its_snap_wins_over_a_handle_further_from_its_own() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let (_, end) = ends(&sketch, line);
        let near_handle = sketch.add_point(Point2::new(20.3, 20.0));
        let near_end = sketch.add_point(Point2::new(30.05, 20.0));
        let mut grab = Grab::of(&sketch, feature(), line, &[], Point2::new(1.0, 0.0)).unwrap();
        let screen = Scaled(10.0);

        grab.follow(
            Point2::new(21.0, 20.0),
            Some((&sketch, &screen)),
            None,
            None,
        );

        let DragCommand::Finish { join: Some(join) } = grab.finish() else {
            panic!("the drag lands on a point");
        };

        assert_eq!(join.point, end);
        assert_eq!(join.at, Point2::new(30.05, 20.0));
        assert_eq!(
            join.constraints,
            vec![Constraint::Coincident(end, near_end)]
        );
        assert!(grab.snap_entities(&sketch).contains(&near_end));
        assert!(!grab.snap_entities(&sketch).contains(&near_handle));
    }

    #[test]
    fn a_group_drag_keeps_the_midpoint_that_won_the_distance_comparison() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        let (_, end) = ends(&sketch, line);
        let upright = sketch.add_line(Point2::new(30.05, 10.0), Point2::new(30.05, 30.0));

        sketch.add_point(Point2::new(20.3, 20.0));
        sketch.add_point(Point2::new(30.7, 20.0));

        let mut grab = Grab::of(&sketch, feature(), line, &[], Point2::new(1.0, 0.0)).unwrap();
        let screen = Scaled(10.0);

        grab.follow(
            Point2::new(21.0, 20.0),
            Some((&sketch, &screen)),
            None,
            None,
        );

        let DragCommand::Finish { join: Some(join) } = grab.finish() else {
            panic!("the drag lands on the midpoint");
        };

        assert_eq!(join.point, end);
        assert_eq!(join.at, Point2::new(30.05, 20.0));
        assert_eq!(
            join.constraints,
            vec![Constraint::Midpoint {
                point: end,
                curve: upright
            }]
        );
    }

    #[test]
    fn a_grabbed_point_tracks_the_other_end_of_its_line_and_keeps_it_level() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(10.0, 15.0));
        let (start, end) = ends(&sketch, line);
        let mut grab = Grab::of(&sketch, feature(), end, &[], Point2::new(10.0, 15.0)).unwrap();

        grab.follow(
            Point2::new(20.0, 10.2),
            Some((&sketch, &Scaled(10.0))),
            None,
            None,
        );

        let landing = grab.landing().unwrap();

        assert_eq!(landing.position, Point2::new(20.0, 10.0));
        assert_eq!(landing.target, None);
        assert_eq!(
            landing.label(&sketch),
            Some(format!("Horizontal from {}", sketch.entity_label(start)))
        );
        assert_eq!(grab.guides(), &[[Point2::new(0.0, 10.0), landing.position]]);
        assert_eq!(
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: end,
                    at: Point2::new(20.0, 10.0),
                    constraints: vec![Constraint::HorizontalPoints(end, start.into())],
                    body: None,
                    rim_of: None,
                }),
            }
        );

        let mut free = Grab::of(&sketch, feature(), end, &[], Point2::new(10.0, 15.0)).unwrap();

        free.follow(
            Point2::new(20.0, 10.2),
            None::<(&Sketch, &Scaled)>,
            None,
            None,
        );

        assert_eq!(free.landing(), None);
        assert_eq!(free.finish(), DragCommand::Finish { join: None });
    }

    #[test]
    fn a_point_grabbed_while_the_snap_is_held_lands_on_the_nearest_grid_crossing() {
        let mut sketch = Sketch::new(Plane::XY);

        let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
        let (_, end) = ends(&sketch, line);
        let mut grab = Grab::of(&sketch, feature(), end, &[], Point2::new(4.0, 0.0)).unwrap();
        let hold = Hold { grid: Some(5.0) };

        grab.follow(
            Point2::new(11.2, 9.3),
            Some((&sketch, &Scaled(10.0))),
            Some(hold),
            None,
        );

        let landing = grab.landing().unwrap();

        assert_eq!(landing.position, Point2::new(10.0, 10.0));
        assert_eq!(landing.target, None);
        assert_eq!(landing.label(&sketch).as_deref(), Some("On the grid"));
        assert_eq!(
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: end,
                    at: Point2::new(10.0, 10.0),
                    constraints: Vec::new(),
                    body: None,
                    rim_of: None,
                }),
            }
        );
    }
}
