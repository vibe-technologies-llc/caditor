use std::collections::BTreeMap;

use caditor_document::{Document, Edit, FeatureId, Hole, Transaction};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_render::{Batch, View};
use caditor_sketch::{ConstraintId, Entity, EntityId, Sketch};

use crate::{
    feature_fields::{POSITION_CAPTIONS, Rule},
    feature_tree, field,
    handle_snap::Snap,
    hole_on_curve,
    hole_panel::EDGE_DISTANCE,
    hole_placement::{self, Anchor, Held as HeldEdge, Placement},
    hole_tools,
    manipulator::Held,
    model::Model,
    move_manipulator::{
        ARROW_POINTS, Arrow, END_ON, GAP_POINTS, HIT_POINTS, Handle, HandleSegment,
        segment_distance, step_for,
    },
    place_handles::{self, Line, Slide},
    scene_palette::ScenePalette,
    units::Units,
};

const ARROW_SHARE: f64 = 0.7;
const ALONG_EDGE: &str = "Along the edge";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoleGrip {
    Square(EntityId),
    Distance(ConstraintId),
    Along(EntityId),
}

#[derive(Debug, Clone, PartialEq)]
struct Stand {
    grip: HoleGrip,
    number: usize,
    at: Point2,
    per_point: f64,
    away: Option<Away>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Away {
    direction: Vector2,
    distance: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HoleHandles {
    pub feature: FeatureId,
    plane: Plane,
    forward: Vector3,
    several: bool,
    stands: Vec<Stand>,
}

fn committed_hole(document: &Document, feature: FeatureId) -> Option<&Hole> {
    document.feature(feature)?.kind.hole()
}

fn previewed_points(
    model: &Model,
    feature: FeatureId,
    sketch: FeatureId,
) -> BTreeMap<EntityId, Point2> {
    model
        .draft_transaction(feature)
        .into_iter()
        .flat_map(Transaction::edits)
        .filter_map(|edit| match edit {
            Edit::SetSketchEntity {
                feature: owner,
                id,
                entity: Entity::Point(at),
            } if *owner == sketch => Some((*id, *at)),
            _ => None,
        })
        .collect()
}

fn leaving(solved: &Sketch, held: &HeldEdge, at: Point2) -> Option<Away> {
    let Some(Entity::Line { start, end }) = solved.entity(held.curve) else {
        return None;
    };
    let from = solved.point(*start)?;
    let along = (solved.point(*end)? - from).try_normalize()?;
    let normal = along.perp();
    let offset = (at - from).dot(normal);
    (offset.abs() > f64::EPSILON).then(|| Away {
        direction: normal * offset.signum(),
        distance: offset.abs(),
    })
}

fn running(solved: &Sketch, held: &HeldEdge) -> Option<Vector2> {
    let Some(Entity::Line { start, end }) = solved.entity(held.curve) else {
        return None;
    };
    (solved.point(*end)? - solved.point(*start)?).try_normalize()
}

fn along_words(
    document: &Document,
    number: usize,
    placement: &Placement,
    point: EntityId,
) -> Option<String> {
    let placed = placement.hole(point)?;
    if placed.along.iter().all(Option::is_none) {
        return None;
    }
    let names: Vec<String> = placed
        .along
        .iter()
        .flatten()
        .flat_map(|along| along.value.parameters())
        .map(|id| {
            document.parameter(id).map_or_else(
                || "a deleted parameter".to_owned(),
                |used| used.name.clone(),
            )
        })
        .collect();
    let names = feature_tree::in_words(&names);
    Some(if names.is_empty() {
        format!("Hole {number} is held by a dimension from the origin; change it in the sketch")
    } else {
        format!("Hole {number}'s position follows {names}; change {names} in Parameters")
    })
}

impl Stand {
    fn centre(&self, plane: &Plane) -> Point3 {
        plane.to_world(self.at)
    }

    fn step(&self) -> f64 {
        step_for(self.per_point)
    }

    fn direction(&self, plane: &Plane) -> Option<Vector3> {
        let away = self.away?;
        Some(plane.x_axis() * away.direction.x + plane.y_axis() * away.direction.y)
    }
}

impl HoleHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let document = model.document();
        let hole = committed_hole(document, feature)?;
        if hole_on_curve::mount(document, hole).is_some()
            || hole_tools::lone_point(document, hole).is_some()
        {
            return None;
        }
        let placement = hole_placement::read(document, hole)?;
        let evaluation = model
            .draft_evaluation_of(feature)
            .unwrap_or_else(|| model.evaluation());
        let solved = hole_placement::solved_in(evaluation, hole.sketch)?;
        let plane = solved.plane();
        let previewed = previewed_points(model, feature, hole.sketch);
        let per_point_at = |at: Point3| {
            let depth = view.view_depth(at);
            let per_point = pixels_per_point * view.units_per_pixel_at(depth);
            (depth.is_finite() && depth > 0.0 && per_point.is_finite() && per_point > 0.0)
                .then_some(per_point)
        };
        let mut stands = Vec::new();
        for (index, placed) in placement.holes.iter().enumerate() {
            let number = index + 1;
            let at = previewed
                .get(&placed.point)
                .copied()
                .or_else(|| solved.point(placed.point))
                .unwrap_or(placed.at);
            let Some(per_point) = per_point_at(plane.to_world(at)) else {
                continue;
            };
            match &placed.anchor {
                Anchor::Free => stands.push(Stand {
                    grip: HoleGrip::Square(placed.point),
                    number,
                    at,
                    per_point,
                    away: None,
                }),
                Anchor::Edges(edges) => {
                    if let [only] = edges.as_slice()
                        && let Some(direction) = running(solved, &only.held)
                    {
                        stands.push(Stand {
                            grip: HoleGrip::Along(placed.point),
                            number,
                            at,
                            per_point,
                            away: Some(Away {
                                direction,
                                distance: 0.0,
                            }),
                        });
                    }
                    for distance in edges {
                        if let Some(away) = leaving(solved, &distance.held, at) {
                            stands.push(Stand {
                                grip: HoleGrip::Distance(distance.held.constraint),
                                number,
                                at,
                                per_point,
                                away: Some(away),
                            });
                        }
                    }
                }
                Anchor::Concentric(_) => {}
            }
        }
        (!stands.is_empty()).then(|| Self {
            feature,
            plane,
            forward: view.forward(),
            several: placement.holes.len() > 1,
            stands,
        })
    }

    fn noun(&self, number: usize) -> String {
        if self.several {
            format!("hole {number}")
        } else {
            "the hole".to_owned()
        }
    }

    fn stand(&self, grip: HoleGrip) -> Option<&Stand> {
        self.stands.iter().find(|stand| stand.grip == grip)
    }

    fn arrow(&self, stand: &Stand) -> Option<(Point3, Point3, Vector3)> {
        let direction = stand.direction(&self.plane)?;
        (direction.dot(self.forward).abs() < END_ON).then(|| {
            let centre = stand.centre(&self.plane);
            (
                centre + direction * GAP_POINTS * stand.per_point,
                centre + direction * ARROW_POINTS * ARROW_SHARE * stand.per_point,
                direction,
            )
        })
    }

    fn square(&self, stand: &Stand) -> Option<[Point3; 4]> {
        match stand.away {
            Some(_) => None,
            None => place_handles::square_around(
                &self.plane,
                stand.centre(&self.plane),
                stand.per_point,
                self.forward,
            ),
        }
    }

    pub fn segments(&self) -> Vec<HandleSegment> {
        self.stands
            .iter()
            .flat_map(|stand| {
                let arrow = self.arrow(stand).map(|(from, tip, _)| (from, tip));
                let edges = self
                    .square(stand)
                    .into_iter()
                    .flat_map(place_handles::square_edges);
                arrow.into_iter().chain(edges)
            })
            .collect()
    }

    #[cfg(test)]
    pub fn step(&self) -> f64 {
        self.stands.first().map_or(1.0, Stand::step)
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let Handle::Hole(grip) = handle else {
            return None;
        };
        let document = model.document();
        let hole = committed_hole(document, self.feature)?;
        let placement = hole_placement::read(document, hole)?;
        let stand = self.stand(grip)?;
        match grip {
            HoleGrip::Square(point) => along_words(document, stand.number, &placement, point),
            HoleGrip::Distance(constraint) => {
                let distance = placement.edge_distance(constraint)?;
                Held::of(document, self.feature, EDGE_DISTANCE, &distance.value).driven()
            }
            HoleGrip::Along(_) => None,
        }
    }

    pub fn words(&self, handle: Handle) -> String {
        let Handle::Hole(grip) = handle else {
            return handle.words();
        };
        let Some(stand) = self.stand(grip) else {
            return handle.words();
        };
        let noun = self.noun(stand.number);
        match grip {
            HoleGrip::Square(_) => format!("Drag to move {noun} on its face"),
            HoleGrip::Distance(_) => format!("Drag to change {noun}'s distance from its edge"),
            HoleGrip::Along(_) => format!("Drag to slide {noun} along its edge"),
        }
    }

    pub fn typing(&self, grip: HoleGrip) -> Option<(String, Vec<Rule>)> {
        self.stand(grip)?;
        let [x, y] = POSITION_CAPTIONS;
        match grip {
            HoleGrip::Square(_) => Some((format!("{x}, {y}"), vec![Rule::Any; 2])),
            HoleGrip::Distance(_) => Some((EDGE_DISTANCE.to_owned(), vec![Rule::ZeroOrMore])),
            HoleGrip::Along(_) => None,
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        let arrow = self
            .stands
            .iter()
            .filter_map(|stand| {
                let (from, to, _) = self.arrow(stand)?;
                let distance = segment_distance(view.project(from)?, view.project(to)?, cursor);
                Some((distance, Handle::Hole(stand.grip)))
            })
            .filter(|(distance, _)| *distance <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, handle)| handle);
        arrow.or_else(|| {
            self.stands.iter().find_map(|stand| {
                let corners = self.square(stand)?;
                place_handles::square_holds(&corners, view, cursor)
                    .then_some(Handle::Hole(stand.grip))
            })
        })
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, along: f64) -> Option<Point3> {
        let Handle::Hole(grip) = handle else {
            return None;
        };
        let stand = self.stand(grip)?;
        match self.arrow(stand) {
            Some((from, tip, direction)) => Some(from.lerp(tip, 0.6) + direction * along),
            None => Some(stand.centre(&self.plane) + self.plane.x_axis() * along),
        }
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        for stand in &self.stands {
            let colour = if highlighted == Some(Handle::Hole(stand.grip)) {
                palette.handle_highlighted
            } else {
                palette.handle
            };
            if let Some(corners) = self.square(stand) {
                place_handles::draw_square(batch, &corners, colour);
            }
            if let Some((from, tip, direction)) = self.arrow(stand) {
                Arrow {
                    from,
                    tip,
                    direction,
                    per_point: stand.per_point,
                    forward: self.forward,
                }
                .add_to(batch, colour);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HoleDrag {
    grip: HoleGrip,
    slide: Slide,
}

impl HoleDrag {
    pub fn begin(handles: &HoleHandles, grip: HoleGrip, ray: Ray) -> Option<Self> {
        let stand = handles.stand(grip)?;
        let step = stand.step();
        let kept_off_edge = matches!(grip, HoleGrip::Distance(_));
        let line = stand.away.map(|away| Line {
            direction: away.direction,
            value: away.distance,
            least: kept_off_edge.then_some(step),
        });
        let slide = Slide::begin(handles.plane, ray, step, stand.at, line)?;
        Some(Self { grip, slide })
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        self.slide.follow(ray, free, snap)
    }

    pub fn has_moved(&self) -> bool {
        self.slide.has_moved()
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let document = model.document();
        let hole = committed_hole(document, feature)?;
        let unit = model.units().length;
        match self.grip {
            HoleGrip::Square(point) => {
                let (from, at) = (self.slide.from(), self.slide.at());
                let (x, y) = (unit.measured(at.x), unit.measured(at.y));
                let typed = [
                    (from.x != at.x).then_some(&x),
                    (from.y != at.y).then_some(&y),
                ];
                hole_placement::positioned(model, feature, hole, point, typed).ok()
            }
            HoleGrip::Distance(constraint) => {
                let placement = hole_placement::read(document, hole)?;
                let distance = placement.edge_distance(constraint)?;
                let name = &document.feature(feature)?.name;
                let keeper = Held::of(document, feature, EDGE_DISTANCE, &distance.value);
                let mut value = distance.value.clone();
                let mut named = Vec::new();
                keeper.set(&mut value, unit.measured(self.slide.value()?), &mut named);
                let kept = (value != distance.value).then_some(Edit::SetDimension {
                    feature: hole.sketch,
                    constraint,
                    value,
                });
                let transaction = Transaction::new(
                    format!("Move {name}"),
                    kept.into_iter().chain(named).collect(),
                );
                field::checked(document, transaction).ok()
            }
            HoleGrip::Along(point) => {
                hole_placement::slid(model, feature, hole, point, self.slide.at()).ok()
            }
        }
    }

    pub fn readout(&self, units: Units) -> String {
        let shown = match (self.grip, self.slide.value()) {
            (HoleGrip::Distance(_), Some(value)) => {
                format!("{EDGE_DISTANCE} {}", units.readout_text(value))
            }
            (HoleGrip::Along(_), Some(value)) => {
                format!("{ALONG_EDGE} {}", units.readout_text(value))
            }
            _ => {
                let [x, y] = POSITION_CAPTIONS;
                let at = self.slide.at();
                format!(
                    "{x} {}, {y} {}",
                    units.readout_text(at.x),
                    units.readout_text(at.y)
                )
            }
        };
        match self.slide.snapped() {
            Some(snap) => format!("{shown} {}", snap.kind.words()),
            None => shown,
        }
    }
}

pub fn typed(
    model: &Model,
    feature: FeatureId,
    grip: HoleGrip,
    values: &[Expression],
) -> Option<Transaction> {
    let document = model.document();
    let hole = committed_hole(document, feature)?;
    match grip {
        HoleGrip::Square(point) => {
            hole_placement::positioned(model, feature, hole, point, [values.first(), values.get(1)])
                .ok()
        }
        HoleGrip::Distance(constraint) => {
            let value = values.first()?.clone();
            hole_placement::distance_change(document, feature, hole, constraint, value).ok()
        }
        HoleGrip::Along(_) => None,
    }
}
