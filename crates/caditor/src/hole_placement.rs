use std::collections::{BTreeMap, BTreeSet};

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, FeatureResult, Hole, Outline,
    ProjectionSource, SketchFeature, Transaction, capitalized, describe_edge, face_plane,
};
use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Point2, Ray};
use caditor_kernel::{ANGULAR_RESOLUTION, EdgeName, LINEAR_RESOLUTION};
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
const NO_LINE: &str = "Select a straight edge, a line of another sketch or an axis";
const NO_CENTRE: &str =
    "Select a round edge, a circle or arc of another sketch, a corner, a datum point or the origin";
const SEVERAL_PICKED: &str = "Select only one edge, curve, axis or point";
const MADE_AFTER: &str = "That edge is made after the hole's sketch in the tree, so the hole \
                          cannot be placed by it";
const NOT_PROJECTED: &str = "That edge cannot be drawn into the hole's sketch";
const NOT_STRAIGHT: &str =
    "Choose a straight edge, a line of another sketch or an axis to measure the hole from";
const END_ON: &str = "That runs square to the face, so it gives no distance; choose an edge or \
                      axis running along the face";
const CONCENTRIC: &str = "The hole is centred on an edge or point; stop that before measuring it \
                          from edges";
const TWO_EDGES: &str = "The hole is already measured from two edges; remove one first";
const SAME_EDGE: &str = "The hole is already measured from that";
const PARALLEL: &str = "That runs parallel to what the hole is already measured from; choose an \
                        edge or axis running across it";
const ON_EDGE: &str = "The hole's centre lies on that line; move it off the line first";
const NOT_ROUND: &str =
    "Choose a round edge, a circle or an arc, or a point, to centre the hole on";
const ALREADY_CONCENTRIC: &str = "The hole is already centred on that";
const NOT_FREE: &str = "That hole is placed by edges; change its distances or remove them to \
                        move it";
const DRIVEN: &str = "The hole's position follows parameters through more than one expression \
                       or one parameter twice; change them in Parameters";
const NOT_SLIDING: &str = "That hole slides along an edge only while it is measured from exactly \
                           one edge";
const LAST_HOLE: &str = "A hole keeps at least one point; delete the hole to remove it";
const NOT_ON_FACE: &str = "Click the face the holes are drilled on";
const MISSED: &str = "The click missed the face's plane";
const NO_ROOM: &str = "There is no room left on that face for another hole";
const LOST_EDGE: &str = "An edge no longer found";
const LOST_FEATURE: &str = "A feature no longer in the model";

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub holes: Vec<PlacedHole>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedHole {
    pub point: EntityId,
    pub at: Point2,
    pub anchor: Anchor,
    pub along: [Option<Along>; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Along {
    pub constraint: ConstraintId,
    pub value: Expression,
}

impl PlacedHole {
    fn along_constraints(&self) -> impl Iterator<Item = ConstraintId> + '_ {
        self.along.iter().flatten().map(|along| along.constraint)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Driver {
    pub parameter: ParameterId,
    pub negated: bool,
}

pub fn driver(document: &Document, expression: &Expression) -> Option<Driver> {
    let mut negated = false;
    let mut current = expression;
    let mut seen = BTreeSet::new();
    loop {
        match current {
            Expression::Negate(inner) => {
                negated = !negated;
                current = inner;
            }
            Expression::Parameter(id) => {
                if !seen.insert(*id) {
                    return None;
                }
                let parameter = document.parameter(*id)?;
                if parameter.expression.parameters().is_empty() {
                    return Some(Driver {
                        parameter: *id,
                        negated,
                    });
                }
                current = &parameter.expression;
            }
            _ => return None,
        }
    }
}

pub fn drivers(document: &Document, placed: &PlacedHole) -> Option<Vec<ParameterId>> {
    let mut found = Vec::new();
    for along in placed.along.iter().flatten() {
        let driven = driver(document, &along.value)?.parameter;
        if found.contains(&driven) {
            return None;
        }
        found.push(driven);
    }
    Some(found)
}

pub fn signed(magnitude: &Expression, negative: bool) -> Expression {
    match (negative, magnitude) {
        (false, _) => magnitude.clone(),
        (true, Expression::Negate(inner)) => (**inner).clone(),
        (true, _) => Expression::Negate(Box::new(magnitude.clone())),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    Free,
    Edges(Vec<EdgeDistance>),
    Concentric(Held),
    At(ProjectionSource),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub constraint: ConstraintId,
    pub curve: EntityId,
    pub source: ProjectionSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeDistance {
    pub held: Held,
    pub value: Expression,
}

impl Held {
    pub fn noun(&self) -> &'static str {
        match self.source {
            ProjectionSource::Edge { .. } => "edge",
            ProjectionSource::DatumAxis { .. } | ProjectionSource::PrincipalAxis { .. } => "axis",
            ProjectionSource::Vertex { .. }
            | ProjectionSource::SketchEntity { .. }
            | ProjectionSource::DatumPoint { .. }
            | ProjectionSource::Origin
            | ProjectionSource::Section { .. }
            | ProjectionSource::DatumPlane { .. }
            | ProjectionSource::PrincipalPlane { .. } => "sketch curve",
        }
    }

    fn what(&self) -> String {
        let article = if matches!(self.noun(), "edge" | "axis") {
            "an"
        } else {
            "a"
        };
        format!("{article} {}", self.noun())
    }
}

impl Anchor {
    pub fn held(&self) -> Vec<&Held> {
        match self {
            Self::Free => Vec::new(),
            Self::Edges(edges) => edges.iter().map(|distance| &distance.held).collect(),
            Self::Concentric(held) => vec![held],
            Self::At(_) => Vec::new(),
        }
    }
}

impl Placement {
    pub fn hole(&self, point: EntityId) -> Option<&PlacedHole> {
        self.holes.iter().find(|hole| hole.point == point)
    }

    pub fn edge_distance(&self, constraint: ConstraintId) -> Option<&EdgeDistance> {
        self.holes.iter().find_map(|hole| match &hole.anchor {
            Anchor::Edges(edges) => edges
                .iter()
                .find(|distance| distance.held.constraint == constraint),
            Anchor::Free | Anchor::Concentric(_) | Anchor::At(_) => None,
        })
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
    let mut curves: BTreeMap<EntityId, &ProjectionSource> = BTreeMap::new();
    let mut pinned: BTreeMap<EntityId, &ProjectionSource> = BTreeMap::new();
    let mut curve_points: BTreeSet<EntityId> = BTreeSet::new();
    for (id, source) in &definition.projections {
        if !places_holes(source) {
            return None;
        }
        match sketch.entity(*id)? {
            Entity::Point(_) => {
                pinned.insert(*id, source);
            }
            entity @ (Entity::Line { .. } | Entity::Circle { .. } | Entity::Arc { .. }) => {
                curve_points.extend(entity.points());
                curves.insert(*id, source);
            }
            _ => return None,
        }
    }
    let mut points: BTreeMap<EntityId, Point2> = BTreeMap::new();
    for (id, entity) in sketch.entities() {
        match entity {
            Entity::Point(_) if curve_points.contains(&id) => {}
            Entity::Point(at) => {
                let projected = sketch.is_projected(id) && !pinned.contains_key(&id);
                if sketch.is_construction(id) || projected {
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
        curves.get(&curve).map(|source| Held {
            constraint,
            curve,
            source: (*source).clone(),
        })
    };
    let mut edges: BTreeMap<EntityId, Vec<EdgeDistance>> = BTreeMap::new();
    let mut concentric: BTreeMap<EntityId, Held> = BTreeMap::new();
    let mut along: BTreeMap<EntityId, [Option<Along>; 2]> = BTreeMap::new();
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
            Constraint::HorizontalDistance { from, to, value }
            | Constraint::VerticalDistance { from, to, value } => {
                let axis = usize::from(matches!(kind, Constraint::VerticalDistance { .. }));
                let point = match (*from, *to) {
                    (EntityId::ORIGIN, point) | (point, EntityId::ORIGIN) => point,
                    _ => return None,
                };
                if !points.contains_key(&point) {
                    return None;
                }
                let slot = along.entry(point).or_default().get_mut(axis)?;
                if slot.is_some() {
                    return None;
                }
                *slot = Some(Along {
                    constraint,
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
            let along = along.remove(&point);
            let held_by = (
                pinned.get(&point),
                edges.remove(&point),
                concentric.remove(&point),
                &along,
            );
            let anchor = match held_by {
                (Some(source), None, None, None) => Anchor::At((*source).clone()),
                (Some(_), ..) => return None,
                (None, None, None, _) => Anchor::Free,
                (None, Some(edges), None, None) if edges.len() <= MAX_EDGES => Anchor::Edges(edges),
                (None, None, Some(held), None) => Anchor::Concentric(held),
                _ => return None,
            };
            Some(PlacedHole {
                point,
                at,
                anchor,
                along: along.unwrap_or_default(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Placement { holes })
}

fn places_holes(source: &ProjectionSource) -> bool {
    match source {
        ProjectionSource::Edge { .. }
        | ProjectionSource::Vertex { .. }
        | ProjectionSource::SketchEntity { .. }
        | ProjectionSource::DatumAxis { .. }
        | ProjectionSource::PrincipalAxis { .. }
        | ProjectionSource::DatumPoint { .. }
        | ProjectionSource::Origin => true,
        ProjectionSource::Section { .. }
        | ProjectionSource::DatumPlane { .. }
        | ProjectionSource::PrincipalPlane { .. } => false,
    }
}

pub fn solved_in(evaluation: &Evaluation, sketch: FeatureId) -> Option<&Sketch> {
    evaluation
        .feature(sketch)?
        .result
        .as_deref()
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
}

fn solved(model: &Model, sketch: FeatureId) -> Option<&Sketch> {
    solved_in(model.evaluation(), sketch)
}

pub fn solved_point(model: &Model, sketch: FeatureId, hole: &PlacedHole) -> Point2 {
    solved(model, sketch)
        .and_then(|solved| solved.point(hole.point))
        .unwrap_or(hole.at)
}

pub fn source_words(model: &Model, sketch: FeatureId, source: &ProjectionSource) -> String {
    let document = model.document();
    let named = |feature: FeatureId| document.feature(feature).map(|found| found.name.clone());
    let words = match source {
        ProjectionSource::Edge { body, edge } => model
            .evaluation()
            .body_result_seen_by(sketch, *body)
            .and_then(|result| {
                let found = edge.resolve(&result.solid).ok()?;
                Some(capitalized(&describe_edge(document, &result.solid, found)))
            }),
        ProjectionSource::Vertex { body, .. } => {
            named(*body).map(|name| format!("A corner of {name}"))
        }
        ProjectionSource::SketchEntity {
            sketch: source,
            entity,
        } => document.feature(*source).and_then(|found| {
            let drawn = found.kind.sketch()?;
            drawn
                .entity(*entity)
                .map(|_| format!("{} of {}", drawn.entity_label(*entity), found.name))
        }),
        ProjectionSource::DatumAxis { datum, .. } | ProjectionSource::DatumPoint { datum } => {
            named(*datum)
        }
        ProjectionSource::PrincipalAxis { axis, .. } => Some(format!("The {}", axis.name())),
        ProjectionSource::Origin => Some("The origin".to_owned()),
        ProjectionSource::Section { .. }
        | ProjectionSource::DatumPlane { .. }
        | ProjectionSource::PrincipalPlane { .. } => None,
    };
    words.unwrap_or_else(|| match source {
        ProjectionSource::Edge { .. } => LOST_EDGE.to_owned(),
        _ => LOST_FEATURE.to_owned(),
    })
}

fn picked(selection: &Selection, nothing: &'static str) -> Result<Pickable, &'static str> {
    let mut usable = selection.iter().filter(|pickable| {
        matches!(
            pickable,
            Pickable::Edge { .. }
                | Pickable::Vertex { .. }
                | Pickable::SketchEntity { .. }
                | Pickable::Datum(_)
                | Pickable::Axis(_)
                | Pickable::Origin
        )
    });
    match (usable.next(), usable.next()) {
        (Some(one), None) => Ok(one),
        (None, _) => Err(nothing),
        (Some(_), Some(_)) => Err(SEVERAL_PICKED),
    }
}

fn projected(
    model: &Model,
    hole: &Hole,
    selection: &Selection,
    nothing: &'static str,
) -> Result<(ProjectionSource, Outline), String> {
    let pickable = picked(selection, nothing)?;
    if let Pickable::Edge { body, edge } = pickable {
        return Ok(projected_edge(model, hole, (body, edge))?);
    }
    let plane =
        scene::sketch_plane(model.document(), model.evaluation(), hole.sketch).ok_or(GONE)?;
    let mut found = projecting::projections_of(model, hole.sketch, &plane, pickable)?;
    match (found.pop(), found.is_empty()) {
        (Some(one), true) => Ok(one),
        _ => Err(NOT_PROJECTED.to_owned()),
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
        Anchor::Concentric(_) | Anchor::At(_) => return Err(CONCENTRIC.to_owned()),
    };
    if measured.len() >= MAX_EDGES {
        return Err(TWO_EDGES.to_owned());
    }
    let (source, outline) = projected(model, hole, selection, NO_LINE)?;
    let (start, end) = match outline {
        Outline::Line { start, end } => (start, end),
        Outline::Point(_) => return Err(END_ON.to_owned()),
        _ => return Err(NOT_STRAIGHT.to_owned()),
    };
    if measured
        .iter()
        .any(|distance| distance.held.source.is_same_source(&source))
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
    transaction.remove_sketch_items(hole.sketch, [], placed.along_constraints());
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
    let (source, outline) = projected(model, hole, selection, NO_CENTRE)?;
    let already = match &placed.anchor {
        Anchor::Concentric(held) => held.source.is_same_source(&source),
        Anchor::At(pinned) => pinned.is_same_source(&source),
        Anchor::Free | Anchor::Edges(_) => false,
    };
    if already {
        return Err(ALREADY_CONCENTRIC.to_owned());
    }
    let centre = match outline {
        Outline::Circle { center, .. } | Outline::Arc { center, .. } | Outline::Point(center) => {
            center
        }
        _ => return Err(NOT_ROUND.to_owned()),
    };
    let on_point = matches!(outline, Outline::Point(_));
    let words = if on_point {
        "centred on a point"
    } else {
        "concentric with an edge"
    };
    let mut transaction = document.transaction(format!("Make {name} {words}"));
    let held = placed.anchor.held();
    transaction.remove_sketch_items(
        hole.sketch,
        held.iter().map(|held| held.curve),
        held.iter()
            .map(|held| held.constraint)
            .chain(placed.along_constraints()),
    );
    if matches!(placed.anchor, Anchor::At(_)) {
        transaction.edit(Edit::SetSketchProjection {
            feature: hole.sketch,
            id: point,
            source: None,
        });
    }
    transaction.edit(Edit::SetSketchEntity {
        feature: hole.sketch,
        id: point,
        entity: Entity::Point(centre),
    });
    if on_point {
        transaction.edit(Edit::SetSketchProjection {
            feature: hole.sketch,
            id: point,
            source: Some(source),
        });
        return field::checked(document, transaction.finish());
    }
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
    transaction.add_sketch_constraint(hole.sketch, Constraint::Coincident(point, centre_point));
    field::checked(document, transaction.finish())
}

pub fn unpinned(
    document: &Document,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
) -> Result<Transaction, String> {
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    if !matches!(placed.anchor, Anchor::At(_)) {
        return Err(NO_POINT.to_owned());
    }
    field::checked(
        document,
        Transaction::single(
            format!("Free {name} from a point"),
            Edit::SetSketchProjection {
                feature: hole.sketch,
                id: point,
                source: None,
            },
        ),
    )
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
    let mut transaction = document.transaction(format!("Free {name} from {}", held.what()));
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

pub fn slid(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
    to: Point2,
) -> Result<Transaction, String> {
    let document = model.document();
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    match &placed.anchor {
        Anchor::Edges(edges) if edges.len() == 1 => {}
        _ => return Err(NOT_SLIDING.to_owned()),
    }
    field::checked(
        document,
        Transaction::single(
            format!("Move {name}"),
            Edit::SetSketchEntity {
                feature: hole.sketch,
                id: point,
                entity: Entity::Point(to),
            },
        ),
    )
}

pub fn dragged(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
    from: Point2,
    to: Point2,
) -> Result<Transaction, String> {
    let document = model.document();
    let unit = model.units().length;
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    if placed.anchor != Anchor::Free {
        return Err(NOT_FREE.to_owned());
    }
    drivers(document, &placed).ok_or(DRIVEN)?;
    let mut at = solved_point(model, hole.sketch, &placed);
    let mut transaction = document.transaction(format!("Move {name}"));
    for (axis, held) in placed.along.iter().enumerate() {
        let (was, now) = if axis == 0 {
            (from.x, to.x)
        } else {
            (from.y, to.y)
        };
        if was == now {
            continue;
        }
        if axis == 0 {
            at.x = now;
        } else {
            at.y = now;
        }
        if let Some(held) = held {
            let driver = driver(document, &held.value).ok_or(DRIVEN)?;
            let magnitude = now.abs();
            transaction.edit(Edit::SetParameterExpression {
                id: driver.parameter,
                expression: unit.measured(if driver.negated {
                    -magnitude
                } else {
                    magnitude
                }),
            });
        }
    }
    transaction.edit(Edit::SetSketchEntity {
        feature: hole.sketch,
        id: point,
        entity: Entity::Point(at),
    });
    field::checked(document, transaction.finish())
}

pub fn positioned(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    point: EntityId,
    typed: [Option<&Expression>; 2],
) -> Result<Transaction, String> {
    let document = model.document();
    let Target { name, placed, .. } = target(document, feature, hole, point)?;
    if placed.anchor != Anchor::Free {
        return Err(NOT_FREE.to_owned());
    }
    let parameters = model.parameters();
    let mut at = solved_point(model, hole.sketch, &placed);
    let mut dimensions = Vec::new();
    let mut freed = Vec::new();
    for (axis, (value, held)) in typed.into_iter().zip(&placed.along).enumerate() {
        let Some(value) = value else {
            continue;
        };
        let millimetres = parameters
            .evaluate_expression(value)
            .map_err(|error| field::sentence(&error.to_string()))?
            .value;
        if axis == 0 {
            at.x = millimetres;
        } else {
            at.y = millimetres;
        }
        let magnitude = signed(value, millimetres < 0.0);
        match (value.parameters().is_empty(), held) {
            (true, None) => {}
            (true, Some(held)) => freed.push(held.constraint),
            (false, Some(held)) => dimensions.push(Dimensioned::Kept(held.constraint, magnitude)),
            (false, None) => {
                dimensions.push(Dimensioned::Added(along_axis(axis, point, magnitude)));
            }
        }
    }
    let mut transaction = document.transaction(format!("Move {name}"));
    transaction.remove_sketch_items(hole.sketch, [], freed);
    transaction.edit(Edit::SetSketchEntity {
        feature: hole.sketch,
        id: point,
        entity: Entity::Point(at),
    });
    for dimension in dimensions {
        match dimension {
            Dimensioned::Kept(constraint, value) => {
                transaction.edit(Edit::SetDimension {
                    feature: hole.sketch,
                    constraint,
                    value,
                });
            }
            Dimensioned::Added(constraint) => {
                transaction.add_sketch_constraint(hole.sketch, constraint);
            }
        }
    }
    field::checked(document, transaction.finish())
}

enum Dimensioned {
    Kept(ConstraintId, Expression),
    Added(Constraint),
}

fn along_axis(axis: usize, point: EntityId, value: Expression) -> Constraint {
    if axis == 0 {
        Constraint::HorizontalDistance {
            from: EntityId::ORIGIN,
            to: point,
            value,
        }
    } else {
        Constraint::VerticalDistance {
            from: EntityId::ORIGIN,
            to: point,
            value,
        }
    }
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
        held.iter()
            .map(|held| held.constraint)
            .chain(placed.along_constraints()),
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
