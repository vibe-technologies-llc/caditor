use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_geometry::{Point2, Vector2};
use caditor_sketch::{Drag, Entity, EntityId, Faceting, Sketch};

use crate::{
    drag_solver::{DragCommand, Join},
    feature_tree::count,
    snap::{Screen, Snapped},
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
    sent: Option<Vec<Drag>>,
    snapped: Option<Snapped>,
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
        Some(Self {
            feature,
            label: format!("Drag {}", subject(sketch, &together)),
            from,
            handles,
            sent: None,
            snapped: None,
        })
    }

    pub fn lone_point(&self) -> Option<EntityId> {
        match &self.handles {
            Handles::Points(points) => match points.as_slice() {
                [(point, _)] => Some(*point),
                _ => None,
            },
            Handles::Radius { .. } => None,
        }
    }

    pub fn moving_with(&self, sketch: &Sketch) -> Vec<EntityId> {
        let Some(point) = self.lone_point() else {
            return Vec::new();
        };
        std::iter::once(point)
            .chain(
                sketch
                    .entities()
                    .filter(|(_, entity)| entity.points().contains(&point))
                    .map(|(id, _)| id),
            )
            .collect()
    }

    pub fn snapped(&self) -> Option<Snapped> {
        self.snapped
    }

    pub fn snap_to(&mut self, cursor: Point2, snapped: Option<Snapped>) -> Option<DragCommand> {
        let snapped = snapped.filter(|_| self.lone_point().is_some());
        self.snapped = snapped;
        let target = match (snapped, &self.handles) {
            (Some(snapped), Handles::Points(points)) => match points.as_slice() {
                [(_, original)] => self.from + (snapped.position - *original),
                _ => cursor,
            },
            _ => cursor,
        };
        self.to(target)
    }

    pub fn finish(&self) -> DragCommand {
        let join = self
            .lone_point()
            .zip(self.snapped)
            .map(|(point, snapped)| Join {
                point,
                at: snapped.position,
                constraints: snapped.target.joins(point),
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

fn segments_cross(a: Vector2, b: Vector2, c: Vector2, d: Vector2) -> bool {
    let side = |p: Vector2, q: Vector2, r: Vector2| (q - p).perp_dot(r - p);
    let (first, second) = (side(a, b, c), side(a, b, d));
    let (third, fourth) = (side(c, d, a), side(c, d, b));
    first * second <= 0.0 && third * fourth <= 0.0
}

pub fn within(
    sketch: &Sketch,
    screen: &impl Screen,
    area: ScreenBox,
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
    use crate::snap::Target;

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
            within(&sketch, &Flat, window, Faceting::within(0.01)),
            vec![inside, across_start, lone]
        );
        assert_eq!(
            within(&sketch, &Flat, crossing, Faceting::within(0.01)),
            vec![inside, across, lone]
        );
        let all = everything(&sketch);
        assert_eq!(all, vec![inside, across, lone, circle]);
        assert!(!all.contains(&across_end));
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

        assert_eq!(grab.moving_with(&sketch), vec![end, line]);
        let snapped = Snapped {
            position: Point2::new(10.0, 10.0),
            target: Target::Point(lone),
        };
        let command = grab.snap_to(Point2::new(10.3, 9.8), Some(snapped));

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
            grab.finish(),
            DragCommand::Finish {
                join: Some(Join {
                    point: end,
                    at: Point2::new(10.0, 10.0),
                    constraints: vec![Constraint::Coincident(end, lone)],
                }),
            }
        );

        let whole = Grab::of(&sketch, feature(), line, &[], Point2::new(2.0, 0.0)).unwrap();
        assert_eq!(whole.lone_point(), None);
    }
}
