use std::collections::{BTreeMap, BTreeSet};

use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, FeatureResult, Hole, Outline, ProjectionSource,
    SketchFeature, Transaction, capitalized, describe_edge, face_plane,
};
use caditor_expression::Expression;
use caditor_geometry::{Point2, Ray};
use caditor_kernel::{ANGULAR_RESOLUTION, EdgeName, EdgeReference, LINEAR_RESOLUTION};
use caditor_sketch::{Constraint, ConstraintId, Entity, EntityId, Sketch};

use crate::{
    bodies, field, hole_tools,
    model::{Action, Model, Notice},
    projecting, scene,
    selection::{Pickable, Selection},
    sketch_placement::FaceChoice,
    solid_tools,
};

pub const MAX_EDGES: usize = 2;
const GONE: &str = "The hole no longer exists";
const NOT_PLACED: &str = "The hole's sketch holds more than its points and the edges placing them; \
                          edit the sketch to place its points";
const NO_POINT: &str = "That hole is no longer in the sketch";
const NO_EDGE: &str = "Select one edge of a body";
const SEVERAL_EDGES: &str = "Select only one edge";
const MADE_AFTER: &str = "That edge is made after the hole's sketch in the tree, so the hole \
                          cannot be placed by it";
const NOT_PROJECTED: &str = "That edge cannot be drawn into the hole's sketch";
const NOT_STRAIGHT: &str = "Choose a straight edge to measure the hole from";
const END_ON: &str = "That edge runs square to the face, so it gives no distance; choose an edge \
                      running along the face";
const CONCENTRIC: &str = "The hole is concentric with a round edge; stop that before measuring it \
                          from edges";
const TWO_EDGES: &str = "The hole is already measured from two edges; remove one first";
const SAME_EDGE: &str = "The hole is already measured from that edge";
const PARALLEL: &str = "That edge runs parallel to the edge the hole is already measured from; \
                        choose an edge running across it";
const ON_EDGE: &str = "The hole's centre lies on that edge; move it off the edge first";
const NOT_ROUND: &str = "Choose a round edge, a circle or an arc, to centre the hole on";
const ALREADY_CONCENTRIC: &str = "The hole is already concentric with that edge";
const NOT_FREE: &str = "That hole is placed by edges; change its distances or remove them to \
                        move it";
const LAST_HOLE: &str = "A hole keeps at least one point; delete the hole to remove it";
const NOT_ON_FACE: &str = "Click the face the holes are drilled on";
const MISSED: &str = "The click missed the face's plane";
const NO_ROOM: &str = "There is no room left on that face for another hole";
const LOST_EDGE: &str = "An edge no longer found";

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub holes: Vec<PlacedHole>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedHole {
    pub point: EntityId,
    pub at: Point2,
    pub anchor: Anchor,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    Free,
    Edges(Vec<EdgeDistance>),
    Concentric(Held),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub constraint: ConstraintId,
    pub curve: EntityId,
    pub body: FeatureId,
    pub edge: EdgeReference,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeDistance {
    pub held: Held,
    pub value: Expression,
}

impl Anchor {
    pub fn held(&self) -> Vec<&Held> {
        match self {
            Self::Free => Vec::new(),
            Self::Edges(edges) => edges.iter().map(|distance| &distance.held).collect(),
            Self::Concentric(held) => vec![held],
        }
    }
}

impl Placement {
    pub fn hole(&self, point: EntityId) -> Option<&PlacedHole> {
        self.holes.iter().find(|hole| hole.point == point)
    }

    fn held(&self, constraint: ConstraintId) -> Option<&Held> {
        self.holes
            .iter()
            .flat_map(|hole| hole.anchor.held())
            .find(|held| held.constraint == constraint)
    }

    pub fn taken(&self) -> Vec<Point2> {
        self.holes.iter().map(|hole| hole.at).collect()
    }
}

pub fn read(document: &Document, hole: &Hole) -> Option<Placement> {
    match &document.feature(hole.sketch)?.kind {
        FeatureKind::Sketch(definition) => of_sketch(definition),
        _ => None,
    }
}

pub fn of_sketch(definition: &SketchFeature) -> Option<Placement> {
    let sketch = &definition.sketch;
    let mut curves: BTreeMap<EntityId, (FeatureId, EdgeReference)> = BTreeMap::new();
    let mut curve_points: BTreeSet<EntityId> = BTreeSet::new();
    for (id, source) in &definition.projections {
        let ProjectionSource::Edge { body, edge } = source else {
            return None;
        };
        let entity = sketch.entity(*id)?;
        if !matches!(
            entity,
            Entity::Line { .. } | Entity::Circle { .. } | Entity::Arc { .. }
        ) {
            return None;
        }
        curve_points.extend(entity.points());
        curves.insert(*id, (*body, *edge));
    }
    let mut points: BTreeMap<EntityId, Point2> = BTreeMap::new();
    for (id, entity) in sketch.entities() {
        match entity {
            Entity::Point(_) if curve_points.contains(&id) => {}
            Entity::Point(at) => {
                if sketch.is_construction(id) || sketch.is_projected(id) {
                    return None;
                }
                points.insert(id, *at);
            }
            _ if curves.contains_key(&id) => {}
            _ => return None,
        }
    }
    if points.is_empty() {
        return None;
    }
    let held = |constraint: ConstraintId, curve: EntityId| {
        curves.get(&curve).map(|(body, edge)| Held {
            constraint,
            curve,
            body: *body,
            edge: *edge,
        })
    };
    let mut edges: BTreeMap<EntityId, Vec<EdgeDistance>> = BTreeMap::new();
    let mut concentric: BTreeMap<EntityId, Held> = BTreeMap::new();
    let mut used: BTreeSet<EntityId> = BTreeSet::new();
    for (constraint, kind) in sketch.constraints() {
        if !sketch.is_active(constraint) {
            return None;
        }
        match kind {
            Constraint::Distance { from, to, value } => {
                let (point, line) = if points.contains_key(from) {
                    (*from, *to)
                } else {
                    (*to, *from)
                };
                let straight = matches!(sketch.entity(line), Some(Entity::Line { .. }));
                if !points.contains_key(&point) || !straight || !used.insert(line) {
                    return None;
                }
                edges.entry(point).or_default().push(EdgeDistance {
                    held: held(constraint, line)?,
                    value: value.clone(),
                });
            }
            Constraint::Coincident(first, second) => {
                let (point, centre) = if points.contains_key(first) {
                    (*first, *second)
                } else {
                    (*second, *first)
                };
                let curve = curves
                    .keys()
                    .copied()
                    .find(|curve| sketch.center_of(*curve) == Some(centre))?;
                if !points.contains_key(&point)
                    || !used.insert(curve)
                    || concentric.contains_key(&point)
                {
                    return None;
                }
                concentric.insert(point, held(constraint, curve)?);
            }
            _ => return None,
        }
    }
    if used.len() != curves.len() {
        return None;
    }
    let holes = points
        .into_iter()
        .map(|(point, at)| {
            let anchor = match (edges.remove(&point), concentric.remove(&point)) {
                (None, None) => Anchor::Free,
                (Some(edges), None) if edges.len() <= MAX_EDGES => Anchor::Edges(edges),
                (None, Some(held)) => Anchor::Concentric(held),
                _ => return None,
            };
            Some(PlacedHole { point, at, anchor })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Placement { holes })
}

fn solved(model: &Model, sketch: FeatureId) -> Option<&Sketch> {
    model
        .evaluation()
        .feature(sketch)?
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
}

pub fn solved_point(model: &Model, sketch: FeatureId, hole: &PlacedHole) -> Point2 {
    solved(model, sketch)
        .and_then(|solved| solved.point(hole.point))
        .unwrap_or(hole.at)
}

pub fn edge_words(model: &Model, sketch: FeatureId, held: &Held) -> String {
    let document = model.document();
    model
        .evaluation()
        .body_result_seen_by(sketch, held.body)
        .and_then(|result| {
            let edge = held.edge.resolve(&result.solid).ok()?;
            Some(capitalized(&describe_edge(document, &result.solid, edge)))
        })
        .unwrap_or_else(|| LOST_EDGE.to_owned())
}

fn picked_edge(selection: &Selection) -> Result<(FeatureId, EdgeName), &'static str> {
    let mut edges = selection.iter().filter_map(|pickable| match pickable {
        Pickable::Edge { body, edge } => Some((body, edge)),
        _ => None,
    });
    match (edges.next(), edges.next()) {
        (Some(edge), None) => Ok(edge),
        (None, _) => Err(NO_EDGE),
        (Some(_), Some(_)) => Err(SEVERAL_EDGES),
    }
}

fn projected_edge(
    model: &Model,
    hole: &Hole,
    (body, name): (FeatureId, EdgeName),
) -> Result<(ProjectionSource, Outline), &'static str> {
    let evaluation = model.evaluation();
    let result = evaluation
        .body_result_seen_by(hole.sketch, body)
        .ok_or(MADE_AFTER)?;
    let edge = bodies::find_edge(result, name).ok_or(MADE_AFTER)?;
    let plane = scene::sketch_plane(model.document(), evaluation, hole.sketch).ok_or(GONE)?;
    projecting::edge_projection(result, body, edge, &plane).ok_or(NOT_PROJECTED)
}

fn same_edge(source: &ProjectionSource, held: &Held) -> bool {
    match source {
        ProjectionSource::Edge { body, edge } => {
            *body == held.body && edge.name() == held.edge.name()
        }
        _ => false,
    }
}

struct Target<'a> {
    name: &'a str,
    placement: Placement,
    placed: PlacedHole,
}

fn target<'a>(
    document: &'a Document,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
) -> Result<Target<'a>, &'static str> {
    let name = document.feature(feature).ok_or(GONE)?.name.as_str();
    let placement = read(document, hole).ok_or(NOT_PLACED)?;
    let placed = placement.hole(point).ok_or(NO_POINT)?.clone();
    Ok(Target {
        name,
        placement,
        placed,
    })
}

pub fn edge_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
) -> Result<Transaction, String> {
    let document = model.document();
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    let measured = match &placed.anchor {
        Anchor::Free => &[][..],
        Anchor::Edges(edges) => edges.as_slice(),
        Anchor::Concentric(_) => return Err(CONCENTRIC.to_owned()),
    };
    if measured.len() >= MAX_EDGES {
        return Err(TWO_EDGES.to_owned());
    }
    let (source, outline) = projected_edge(model, hole, picked_edge(selection)?)?;
    let (start, end) = match outline {
        Outline::Line { start, end } => (start, end),
        Outline::Point(_) => return Err(END_ON.to_owned()),
        _ => return Err(NOT_STRAIGHT.to_owned()),
    };
    if measured
        .iter()
        .any(|distance| same_edge(&source, &distance.held))
    {
        return Err(SAME_EDGE.to_owned());
    }
    let along = end - start;
    let length = along.length();
    if length <= LINEAR_RESOLUTION {
        return Err(END_ON.to_owned());
    }
    let solved_sketch = solved(model, hole.sketch);
    let crossed = measured.iter().all(|distance| {
        solved_sketch
            .and_then(|solved| solved.line_endpoints(distance.held.curve))
            .is_none_or(|(first, second)| {
                (second - first)
                    .normalize_or_zero()
                    .perp_dot(along / length)
                    .abs()
                    > ANGULAR_RESOLUTION
            })
    });
    if !crossed {
        return Err(PARALLEL.to_owned());
    }
    let at = solved_point(model, hole.sketch, &placed);
    let distance = along.perp_dot(at - start).abs() / length;
    if distance <= LINEAR_RESOLUTION {
        return Err(ON_EDGE.to_owned());
    }
    let mut transaction = document.transaction(format!("Place {name} from an edge"));
    let projected = transaction.add_projected(hole.sketch, source, &outline);
    transaction.edit(Edit::SetSketchConstruction {
        feature: hole.sketch,
        id: projected.id,
        construction: true,
    });
    transaction.add_sketch_constraint(
        hole.sketch,
        Constraint::Distance {
            from: point,
            to: projected.id,
            value: model.length_unit().measured(distance),
        },
    );
    field::checked(document, transaction.finish())
}

pub fn concentric_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
) -> Result<Transaction, String> {
    let document = model.document();
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    let (source, outline) = projected_edge(model, hole, picked_edge(selection)?)?;
    let centre = match outline {
        Outline::Circle { center, .. } | Outline::Arc { center, .. } => center,
        _ => return Err(NOT_ROUND.to_owned()),
    };
    if let Anchor::Concentric(held) = &placed.anchor
        && same_edge(&source, held)
    {
        return Err(ALREADY_CONCENTRIC.to_owned());
    }
    let mut transaction = document.transaction(format!("Make {name} concentric with an edge"));
    let held = placed.anchor.held();
    transaction.remove_sketch_items(
        hole.sketch,
        held.iter().map(|held| held.curve),
        held.iter().map(|held| held.constraint),
    );
    let projected = transaction.add_projected(hole.sketch, source, &outline);
    let centre_point = projected
        .entities
        .iter()
        .find_map(|(_, entity)| match entity {
            Entity::Circle { center, .. } | Entity::Arc { center, .. } => Some(*center),
            _ => None,
        })
        .ok_or(NOT_ROUND)?;
    transaction.edit(Edit::SetSketchConstruction {
        feature: hole.sketch,
        id: projected.id,
        construction: true,
    });
    transaction.edit(Edit::SetSketchEntity {
        feature: hole.sketch,
        id: point,
        entity: Entity::Point(centre),
    });
    transaction.add_sketch_constraint(hole.sketch, Constraint::Coincident(point, centre_point));
    field::checked(document, transaction.finish())
}

pub fn released(
    document: &Document,
    feature: FeatureId,
    hole: &Hole,
    constraint: ConstraintId,
) -> Result<Transaction, String> {
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let placement = read(document, hole).ok_or(NOT_PLACED)?;
    let held = placement.held(constraint).ok_or(NO_POINT)?;
    let mut transaction = document.transaction(format!("Free {name} from an edge"));
    transaction.remove_sketch_items(hole.sketch, [held.curve], [held.constraint]);
    field::checked(document, transaction.finish())
}

pub fn distance_change(
    document: &Document,
    feature: FeatureId,
    hole: &Hole,
    constraint: ConstraintId,
    value: Expression,
) -> Result<Transaction, String> {
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let placement = read(document, hole).ok_or(NOT_PLACED)?;
    placement.held(constraint).ok_or(NO_POINT)?;
    field::checked(
        document,
        Transaction::single(
            format!("Move {name}"),
            Edit::SetDimension {
                feature: hole.sketch,
                constraint,
                value,
            },
        ),
    )
}

pub fn moved(
    document: &Document,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
    at: Point2,
) -> Result<Transaction, String> {
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    if placed.anchor != Anchor::Free {
        return Err(NOT_FREE.to_owned());
    }
    field::checked(
        document,
        Transaction::single(
            format!("Move {name}"),
            Edit::SetSketchEntity {
                feature: hole.sketch,
                id: point,
                entity: Entity::Point(at),
            },
        ),
    )
}

pub fn removed(
    document: &Document,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
) -> Result<Transaction, String> {
    let Target {
        name,
        placement,
        placed,
    } = target(document, feature, hole, point)?;
    if placement.holes.len() <= 1 {
        return Err(LAST_HOLE.to_owned());
    }
    let held = placed.anchor.held();
    let mut transaction = document.transaction(format!("Remove a hole from {name}"));
    transaction.remove_sketch_items(
        hole.sketch,
        held.iter()
            .map(|held| held.curve)
            .chain(std::iter::once(point)),
        held.iter().map(|held| held.constraint),
    );
    field::checked(document, transaction.finish())
}

fn added_at(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Result<Transaction, String> {
    let document = model.document();
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let placement = read(document, hole).ok_or(NOT_PLACED)?;
    let face = FaceChoice::of(pickable).ok_or(NOT_ON_FACE)?;
    let plane = scene::sketch_plane(document, model.evaluation(), hole.sketch).ok_or(GONE)?;
    let shown = bodies::shown(model.evaluation(), face.body).ok_or(NOT_ON_FACE)?;
    let id = bodies::find_face(shown, face.face).ok_or(NOT_ON_FACE)?;
    let lies_on = face_plane(&shown.solid, id).ok_or(NOT_ON_FACE)?;
    if !solid_tools::same_plane(&lies_on, &plane) {
        return Err(NOT_ON_FACE.to_owned());
    }
    let at = match ray {
        Some(ray) => plane.to_local(ray.at(ray.intersect_plane(&plane).ok_or(MISSED)?)),
        None => {
            hole_tools::clear_spot(&shown.solid, id, &plane, &placement.taken()).ok_or(NO_ROOM)?
        }
    };
    let mut transaction = document.transaction(format!("Add a hole to {name}"));
    transaction.add_sketch_entity(hole.sketch, Entity::Point(at));
    field::checked(document, transaction.finish())
}

pub fn add_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    hole: &Hole,
) -> Result<Transaction, String> {
    let mut faces = selection
        .iter()
        .filter(|pickable| matches!(pickable, Pickable::Face { .. }));
    match (faces.next(), faces.next()) {
        (Some(face), None) => added_at(model, feature, hole, face, None),
        _ => Err(NOT_ON_FACE.to_owned()),
    }
}

pub fn add_click(
    model: &Model,
    feature: FeatureId,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Vec<Action> {
    let Some(hole) = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.hole())
    else {
        return Vec::new();
    };
    match added_at(model, feature, hole, pickable, ray) {
        Ok(transaction) => vec![Action::Apply(transaction)],
        Err(reason) => vec![Action::Inform(Notice::warning(reason))],
    }
}

#[cfg(test)]
mod tests;
