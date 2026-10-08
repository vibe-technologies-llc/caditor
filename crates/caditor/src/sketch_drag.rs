use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Drag, Entity, EntityId, Faceting, MAX_LENGTH, Sketch};

use crate::{
    drag_solver::{DragCommand, Join},
    feature_tree::count,
    snap::{Pointer, Screen},
    tracking::{self, Acquired, Landing},
};

const SMALLEST_DRAGGED_RADIUS: f64 = 1e-3;
const MOVE_WHILE_DRAWING: &str = "Switch to the Select tool to move geometry";
const NOTHING_TO_SELECT: &str = "The sketch has no geometry to select";

#[derive(Debug, Clone, PartialEq)]
enum Handles {
    Points(Vec<(EntityId, Point2)>),
    Radius {
        circle: EntityId,
        centre: Point2,
        radius: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grab {
    feature: FeatureId,
    label: String,
    from: Point2,
    handles: Handles,
    handle: Option<(EntityId, Point2)>,
    sent: Option<Vec<Drag>>,
    landing: Option<Landing>,
    guides: Vec<[Point2; 2]>,
    acquired: Acquired,
}

impl Grab {
    pub fn of(
        sketch: &Sketch,
        feature: FeatureId,
        grabbed: EntityId,
        selected: &[EntityId],
        from: Point2,
    ) -> Option<Self> {
        if grabbed.is_reference() || sketch.entity(grabbed).is_none() {
            return None;
        }
        let together: Vec<EntityId> = if selected.contains(&grabbed) {
            selected
                .iter()
                .copied()
                .filter(|entity| !entity.is_reference())
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
            _ => Handles::Points(points_of(sketch, &together)),
        };
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
            sent: None,
            landing: None,
            guides: Vec::new(),
            acquired: Acquired::default(),
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

    pub fn guides(&self) -> &[[Point2; 2]] {
        &self.guides
    }

    pub fn follow(
        &mut self,
        cursor: Point2,
        snapping: Option<(&Sketch, &impl Screen)>,
    ) -> Option<DragCommand> {
        let offset = cursor - self.from;
        let landing = snapping
            .zip(self.handle)
            .and_then(|((sketch, screen), (_, original))| {
                let moved = original + offset;
                let pointer = Pointer {
                    screen: screen.to_screen(moved)?,
                    sketch: moved,
                };
                let ignored = self.moving_with(sketch);
                let landing = tracking::land(sketch, screen, pointer, &self.acquired, &ignored)?;
                Some((sketch, landing))
            });
        if let Some((sketch, landing)) = landing
            && let Some(target) = landing.target
        {
            self.acquired.note(sketch, target);
        }
        self.guides = landing
            .map(|(sketch, landing)| landing.guides(sketch))
            .unwrap_or_default();
        self.landing = landing.map(|(_, landing)| landing);
        let target = match (self.landing, self.handle) {
            (Some(landing), Some((_, original))) => self.from + (landing.position - original),
            _ => cursor,
        };
        self.to(target)
    }

    pub fn finish(&self) -> DragCommand {
        let join = self
            .handle
            .zip(self.landing)
            .map(|((point, _), landing)| Join {
                point,
                at: landing.position,
                constraints: landing.joins(point),
            });
        DragCommand::Finish { join }
    }

    pub fn feature(&self) -> FeatureId {
        self.feature
    }

    pub fn to(&mut self, cursor: Point2) -> Option<DragCommand> {
        let drags = match &self.handles {
            Handles::Points(points) => translated(points, cursor - self.from),
            Handles::Radius {
                circle,
                centre,
                radius,
            } => {
                let grown = radius + centre.distance(cursor) - centre.distance(self.from);
                vec![Drag::Radius {
                    circle: *circle,
                    to: grown.max(radius * SMALLEST_DRAGGED_RADIUS),
                }]
            }
        };
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

#[derive(Debug, Clone, PartialEq)]
pub struct Moving {
    pub feature: FeatureId,
    pub label: String,
    pub anchor: Point2,
    points: Vec<(EntityId, Point2)>,
}

impl Moving {
    pub fn offered(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
        drawing: bool,
    ) -> Result<Self, String> {
        if drawing {
            Err(MOVE_WHILE_DRAWING.to_owned())
        } else {
            Self::of(sketch, feature, selected)
        }
    }

    pub fn of(sketch: &Sketch, feature: FeatureId, selected: &[EntityId]) -> Result<Self, String> {
        let entities: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|entity| !entity.is_reference() && sketch.entity(*entity).is_some())
            .collect();
        let points = points_of(sketch, &entities);
        let Some(&(_, anchor)) = points.first() else {
            return Err("Select sketch geometry to move it".to_owned());
        };
        Ok(Self {
            feature,
            label: format!("Move {}", subject(sketch, &entities)),
            anchor,
            points,
        })
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn nothing_selected(self) -> String {
        format!(
            "Select sketch geometry to {} it",
            self.verb().to_lowercase()
        )
    }

    fn needs_pivot(self) -> String {
        format!(
            "{} a lone point by selecting the point to {} it about with it",
            self.verb(),
            match self {
                Self::Rotate => "turn",
                Self::Scale => "scale",
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transforming {
    pub feature: FeatureId,
    pub transform: Transform,
    pub label: String,
    pub pivot: Point2,
    points: Vec<(EntityId, Point2)>,
    circles: Vec<(EntityId, f64)>,
}

impl Transforming {
    pub fn offered(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
        drawing: bool,
        transform: Transform,
    ) -> Result<Self, String> {
        if drawing {
            Err(format!(
                "Switch to the Select tool to {} geometry",
                transform.verb().to_lowercase()
            ))
        } else {
            Self::of(sketch, feature, selected, transform)
        }
    }

    pub fn of(
        sketch: &Sketch,
        feature: FeatureId,
        selected: &[EntityId],
        transform: Transform,
    ) -> Result<Self, String> {
        let present: Vec<EntityId> = selected
            .iter()
            .copied()
            .filter(|entity| sketch.entity(*entity).is_some() || *entity == EntityId::ORIGIN)
            .collect();
        let curve_points: BTreeSet<EntityId> = present
            .iter()
            .filter_map(|entity| sketch.entity(*entity))
            .flat_map(Entity::points)
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
            .filter(|entity| !entity.is_reference() && Some(*entity) != pivot_point)
            .collect();
        let points = points_of(sketch, &moved);
        if points.is_empty() {
            return Err(transform.nothing_selected());
        }
        let circles: Vec<(EntityId, f64)> = moved
            .iter()
            .filter_map(|entity| match sketch.entity(*entity) {
                Some(Entity::Circle { radius, .. }) => Some((*entity, *radius)),
                _ => None,
            })
            .collect();
        let pivot = match pivot_point.and_then(|point| sketch.point(point)) {
            Some(pivot) => pivot,
            None if points.len() == 1 && circles.is_empty() => {
                return Err(transform.needs_pivot());
            }
            None => extent_centre(sketch, &points, &circles),
        };
        Ok(Self {
            feature,
            transform,
            label: format!("{} {}", transform.verb(), subject(sketch, &moved)),
            pivot,
            points,
            circles,
        })
    }

    pub fn rotated(&self, degrees: f64) -> Result<Vec<DragCommand>, String> {
        let turn = Vector2::from_angle(degrees.to_radians());
        let drags = self
            .points
            .iter()
            .map(|(point, at)| Drag::Point {
                point: *point,
                to: self.pivot + turn.rotate(*at - self.pivot),
            })
            .collect();
        self.commands(drags)
    }

    pub fn scaled(&self, factor: f64) -> Result<Vec<DragCommand>, String> {
        if !(factor.is_finite() && factor > 0.0) {
            return Err("The factor must be a number above zero".to_owned());
        }
        let points = self.points.iter().map(|(point, at)| Drag::Point {
            point: *point,
            to: self.pivot + (*at - self.pivot) * factor,
        });
        let radii = self.circles.iter().map(|(circle, radius)| Drag::Radius {
            circle: *circle,
            to: radius * factor,
        });
        self.commands(points.chain(radii).collect())
    }

    fn commands(&self, drags: Vec<Drag>) -> Result<Vec<DragCommand>, String> {
        let beyond = drags.iter().any(|drag| match drag {
            Drag::Point { to, .. } => to.abs().max_element() > MAX_LENGTH,
            Drag::Radius { to, .. } => *to > MAX_LENGTH,
        });
        if beyond {
            return Err(format!(
                "Keep the geometry within {} m of the sketch's origin",
                MAX_LENGTH / 1_000.0
            ));
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

fn extent_centre(
    sketch: &Sketch,
    points: &[(EntityId, Point2)],
    circles: &[(EntityId, f64)],
) -> Point2 {
    let rims = circles.iter().filter_map(|(circle, radius)| {
        let centre = match sketch.entity(*circle) {
            Some(Entity::Circle { center, .. }) => sketch.point(*center)?,
            _ => return None,
        };
        Some([
            centre - Vector2::splat(*radius),
            centre + Vector2::splat(*radius),
        ])
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

fn subject(sketch: &Sketch, entities: &[EntityId]) -> String {
    match entities {
        [only] => sketch.entity_label(*only),
        entities => count(entities.len(), "item", "items"),
    }
}

fn points_of(sketch: &Sketch, entities: &[EntityId]) -> Vec<(EntityId, Point2)> {
    let mut seen = BTreeSet::new();
    entities
        .iter()
        .flat_map(|entity| match sketch.entity(*entity) {
            Some(Entity::Point(_)) => vec![*entity],
            Some(other) => other.points(),
            None => Vec::new(),
        })
        .filter(|point| seen.insert(*point))
        .filter_map(|point| Some((point, sketch.point(point)?)))
        .collect()
}

fn translated(points: &[(EntityId, Point2)], offset: Vector2) -> Vec<Drag> {
    points
        .iter()
        .map(|(point, at)| Drag::Point {
            point: *point,
            to: *at + offset,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxMode {
    Window,
    Crossing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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
    first * second <= 0.0 && third * fourth <= 0.0
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
            Entity::Point(position) => screen
                .to_screen(*position)
                .is_some_and(|point| area.contains(point)),
            _ => {
                let outline: Option<Vec<Vector2>> = sketch
                    .faceted(*id, faceting)
                    .map(|points| {
                        points
                            .into_iter()
                            .map(|point| screen.to_screen(point))
                            .collect()
                    })
                    .unwrap_or_default();
                outline.is_some_and(|outline| match mode {
                    BoxMode::Window => outline.iter().all(|point| area.contains(*point)),
                    BoxMode::Crossing => outline
                        .windows(2)
                        .any(|pair| matches!(pair, [from, to] if area.crosses(*from, *to))),
                })
            }
        })
        .map(|(id, _)| id);
    selectable(sketch, caught)
}

pub fn select_all(sketch: &Sketch) -> Result<Vec<EntityId>, &'static str> {
    let everything = everything(sketch);
    if everything.is_empty() {
        Err(NOTHING_TO_SELECT)
    } else {
        Ok(everything)
    }
}

pub fn can_select_all(sketch: &Sketch) -> Result<(), &'static str> {
    if sketch.entities().any(|(id, _)| !id.is_reference()) {
        Ok(())
    } else {
        Err(NOTHING_TO_SELECT)
    }
}

pub fn everything(sketch: &Sketch) -> Vec<EntityId> {
    selectable(
        sketch,
        sketch
            .entities()
            .map(|(id, _)| id)
            .filter(|id| !id.is_reference()),
    )
}

fn selectable(sketch: &Sketch, caught: impl Iterator<Item = EntityId>) -> Vec<EntityId> {
    let caught: Vec<EntityId> = caught.collect();
    let owned: BTreeSet<EntityId> = caught
        .iter()
        .filter_map(|id| sketch.entity(*id))
        .flat_map(Entity::points)
        .collect();
    caught
        .into_iter()
        .filter(|id| !owned.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use caditor_document::{Document, FeatureKind};
    use caditor_geometry::Plane;
    use caditor_sketch::Constraint;

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
    fn a_lone_grabbed_point_lands_on_its_snap_and_joins_it_when_released() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::ZERO, Point2::new(4.0, 0.0));
        let (_, end) = ends(&sketch, line);
        let lone = sketch.add_point(Point2::new(10.0, 10.0));
        let mut grab = Grab::of(&sketch, feature(), end, &[], Point2::new(4.1, 0.1)).unwrap();

        let command = grab.follow(Point2::new(10.3, 9.8), Some((&sketch, &Scaled(10.0))));

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

        let Some(DragCommand::Move { drags, .. }) =
            grab.follow(Point2::new(28.05, 20.02), Some((&sketch, &Scaled(10.0))))
        else {
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
                }),
            }
        );
    }

    #[test]
    fn a_grabbed_point_tracks_the_other_end_of_its_line_and_keeps_it_level() {
        let mut sketch = Sketch::new(Plane::XY);
        let line = sketch.add_line(Point2::new(0.0, 10.0), Point2::new(10.0, 15.0));
        let (start, end) = ends(&sketch, line);
        let mut grab = Grab::of(&sketch, feature(), end, &[], Point2::new(10.0, 15.0)).unwrap();

        grab.follow(Point2::new(20.0, 10.2), Some((&sketch, &Scaled(10.0))));
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
                    constraints: vec![Constraint::HorizontalPoints(end, start)],
                }),
            }
        );

        let mut free = Grab::of(&sketch, feature(), end, &[], Point2::new(10.0, 15.0)).unwrap();
        free.follow(Point2::new(20.0, 10.2), None::<(&Sketch, &Scaled)>);
        assert_eq!(free.landing(), None);
        assert_eq!(free.finish(), DragCommand::Finish { join: None });
    }
}
