use caditor_document::{
    FeatureId, FeatureKind, Outline, ProjectionSource, SketchFeature, TransactionBuilder,
};
use caditor_sketch::{Constraint, Entity, EntityId, PointTarget, Sketch};

use crate::{
    body_snap::{BodyItem, BodyPart, BodySnaps},
    drag_solver::BodyJoin,
    model::Model,
    projecting,
    selection::{self, Pickable, Selection},
    sketch_tools,
    tracking::Track,
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct BodyPick {
    pub body: FeatureId,
    pub item: BodyItem,
}

impl BodyPick {
    pub fn pickable(self, sketch: FeatureId) -> Pickable {
        Pickable::BodyItem {
            sketch,
            body: self.body,
            item: self.item,
        }
    }

    pub fn of(pickable: Pickable, sketch: FeatureId) -> Option<Self> {
        match pickable {
            Pickable::BodyItem {
                sketch: owner,
                body,
                item,
            } if owner == sketch => Some(Self { body, item }),
            _ => None,
        }
    }
}

pub fn selected(selection: &Selection, sketch: FeatureId) -> Vec<BodyPick> {
    selection
        .iter()
        .filter_map(|pickable| BodyPick::of(pickable, sketch))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unstaged {
    #[error("That is not part of the sketch being edited")]
    NotEdited,
    #[error("That corner or edge of {body} is no longer there; pick it again")]
    Gone { body: String },
    #[error("The centre of that round edge is not a point the sketch can use")]
    NoCentre,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projections {
    added: Vec<(ProjectionSource, Outline)>,
    fixed: Vec<EntityId>,
}

impl Projections {
    pub fn project(&self, transaction: &mut TransactionBuilder<'_>, feature: FeatureId) {
        for (source, outline) in &self.added {
            transaction.add_projected(feature, source.clone(), outline);
        }
    }

    pub fn is_fixed(&self, entity: EntityId) -> bool {
        self.fixed.contains(&entity)
    }
}

#[derive(Clone, Debug)]
pub struct Staged {
    pub definition: Sketch,
    pub shown: Sketch,
    pub picks: Vec<EntityId>,
    pub projections: Projections,
    names: Vec<(EntityId, String)>,
    bodies: Vec<(BodyPick, EntityId)>,
}

impl Staged {
    pub fn new(
        model: &Model,
        feature: FeatureId,
        snaps: &BodySnaps,
        entities: &[EntityId],
        bodies: &[BodyPick],
    ) -> Result<Self, Unstaged> {
        let owner = model
            .document()
            .feature(feature)
            .ok_or(Unstaged::NotEdited)?;
        let FeatureKind::Sketch(sketch) = &owner.kind else {
            return Err(Unstaged::NotEdited);
        };
        let shown = model.displayed_sketch(owner).ok_or(Unstaged::NotEdited)?;

        let mut staged = Self {
            definition: sketch.sketch.clone(),
            shown: Sketch::clone(&shown),
            picks: entities.to_vec(),
            projections: Projections::default(),
            names: Vec::new(),
            bodies: Vec::new(),
        };

        if bodies.is_empty() {
            return Ok(staged);
        }

        let mut transaction = sketch_tools::settled_transaction(model, feature, String::new());
        let mut projected: Vec<(usize, EntityId)> = Vec::new();

        for pick in bodies {
            let gone = || Unstaged::Gone {
                body: selection::body_name(model.document(), pick.body).to_owned(),
            };
            let target = snaps.find(pick.body, pick.item).ok_or_else(gone)?;

            let source = snaps.source(target).ok_or_else(gone)?;
            let known = projected
                .iter()
                .find(|(index, _)| *index == target.source)
                .map(|(_, id)| *id)
                .or_else(|| projecting::existing_projection(sketch, &source.projection));

            let curve = match known {
                Some(id) => id,
                None => {
                    let added = transaction.add_projected(
                        feature,
                        source.projection.clone(),
                        &source.outline,
                    );

                    for (id, entity) in added.entities {
                        staged.insert(id, entity);
                    }
                    staged
                        .projections
                        .added
                        .push((source.projection.clone(), source.outline.clone()));
                    added.id
                }
            };

            projected.push((target.source, curve));

            let id = match pick.item {
                BodyItem::Corner(_) | BodyItem::Edge(_) => curve,
                BodyItem::Centre(_) => centre_of(&staged.shown, curve).ok_or(Unstaged::NoCentre)?,
            };

            staged.names.push((
                id,
                pick.item
                    .words(&selection::body_name(model.document(), pick.body)),
            ));
            staged.picks.push(id);
            staged.bodies.push((*pick, id));
        }
        Ok(staged)
    }

    fn insert(&mut self, id: EntityId, entity: Entity) {
        for sketch in [&mut self.definition, &mut self.shown] {
            let inserted = sketch
                .insert_entity(id, entity.clone())
                .and_then(|()| sketch.set_projected(id, true));
            if let Err(error) = inserted {
                log::debug!("the staged sketch does not see projected entity {id}: {error}");
            }
        }
        self.projections.fixed.push(id);
    }

    pub fn names(&self) -> &[(EntityId, String)] {
        &self.names
    }

    pub fn body_entity(&self, pick: BodyPick) -> Option<EntityId> {
        self.bodies
            .iter()
            .find(|(staged, _)| *staged == pick)
            .map(|(_, id)| *id)
    }
}

pub struct Joined {
    pub constraint: Constraint,
    pub projects: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Holding {
    On,
    Along(Track),
}

pub fn body_joins(
    transaction: &mut TransactionBuilder<'_>,
    (definition, feature): (&SketchFeature, FeatureId),
    shadow: &mut Sketch,
    point: EntityId,
    holds: &[(Holding, &BodyJoin)],
) -> Vec<Option<Joined>> {
    let mut made: Vec<(&ProjectionSource, EntityId)> = Vec::new();

    holds
        .iter()
        .map(|(holding, body)| {
            if *holding != Holding::On && !body.part.is_point_like() {
                return None;
            }

            let (curve, projects) =
                projected(transaction, (definition, feature), shadow, &mut made, body);
            let constraint = match holding {
                Holding::On => match body.part {
                    BodyPart::Corner | BodyPart::Edge => Constraint::Coincident(point, curve),
                    BodyPart::Centre => Constraint::Coincident(point, centre_of(shadow, curve)?),
                    BodyPart::Middle => Constraint::Midpoint { point, curve },
                },
                Holding::Along(track) => track.holding(
                    point,
                    match body.part {
                        BodyPart::Corner => PointTarget::Point(curve),
                        BodyPart::Centre => PointTarget::Point(centre_of(shadow, curve)?),
                        BodyPart::Middle => PointTarget::Middle(curve),
                        BodyPart::Edge => return None,
                    },
                ),
            };
            Some(Joined {
                constraint,
                projects,
            })
        })
        .collect()
}

fn projected<'a>(
    transaction: &mut TransactionBuilder<'_>,
    (definition, feature): (&SketchFeature, FeatureId),
    shadow: &mut Sketch,
    made: &mut Vec<(&'a ProjectionSource, EntityId)>,
    body: &'a BodyJoin,
) -> (EntityId, bool) {
    if let Some((_, id)) = made
        .iter()
        .find(|(source, _)| source.is_same_source(&body.projection))
    {
        return (*id, false);
    }
    if let Some(id) = projecting::existing_projection(definition, &body.projection) {
        return (id, false);
    }

    let added = transaction.add_projected(feature, body.projection.clone(), &body.outline);

    for (id, entity) in added.entities {
        let inserted = shadow
            .insert_entity(id, entity)
            .and_then(|()| shadow.set_projected(id, true));
        if let Err(error) = inserted {
            log::debug!("the drag check does not see projected entity {id}: {error}");
        }
    }
    made.push((&body.projection, added.id));
    (added.id, true)
}

fn centre_of(sketch: &Sketch, curve: EntityId) -> Option<EntityId> {
    match sketch.entity(curve)? {
        Entity::Circle { center, .. }
        | Entity::Arc { center, .. }
        | Entity::Ellipse { center, .. }
        | Entity::EllipticalArc { center, .. } => Some(*center),
        Entity::Point(_) | Entity::Line { .. } | Entity::Spline { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use caditor_document::{
        Document, FeatureId, FeatureKind, Outline, ProjectionSource, SketchFeature,
    };
    use caditor_geometry::{Plane, Point2};
    use caditor_sketch::{Constraint, Entity, PointTarget, Sketch};

    use super::{Holding, body_joins};
    use crate::{
        body_snap::{BodyPart, BodyTarget},
        drag_solver::BodyJoin,
        snap::Target,
        tracking::{Axis, Track},
    };

    fn added(document: &mut Document, name: &str, sketch: Sketch) -> FeatureId {
        let mut adding = document.transaction("Add sketch");

        let feature = adding.add_feature(name, FeatureKind::from(sketch));

        document.apply(adding.finish()).unwrap();
        feature
    }

    fn definition(document: &Document, feature: FeatureId) -> &SketchFeature {
        match document.feature(feature).map(|owner| &owner.kind) {
            Some(FeatureKind::Sketch(definition)) => definition,
            _ => panic!("the sketch is a sketch feature"),
        }
    }

    fn track(from: Point2, axis: Axis) -> Track {
        Track {
            anchor: Target::Body(BodyTarget {
                source: 0,
                part: BodyPart::Centre,
                generation: 0,
            }),
            from,
            axis,
        }
    }

    #[test]
    fn a_point_on_a_body_edge_and_tracked_from_body_items_projects_each_item_once() {
        let mut document = Document::default();
        let mut other = Sketch::new(Plane::XY);
        let mut sketch = Sketch::new(Plane::XY);

        let side = other.add_line(Point2::new(40.0, 0.0), Point2::new(40.0, 40.0));
        let rim = other.add_circle(Point2::new(60.0, 20.0), 5.0);
        let point = sketch.add_point(Point2::new(40.0, 20.0));
        let source = added(&mut document, "Bodies", other);
        let feature = added(&mut document, "Dragged", sketch.clone());

        let edge = |part: BodyPart| BodyJoin {
            projection: ProjectionSource::SketchEntity {
                sketch: source,
                entity: side,
            },
            outline: Outline::Line {
                start: Point2::new(40.0, 0.0),
                end: Point2::new(40.0, 40.0),
            },
            part,
        };
        let on_edge = edge(BodyPart::Edge);
        let middle = edge(BodyPart::Middle);
        let round = BodyJoin {
            projection: ProjectionSource::SketchEntity {
                sketch: source,
                entity: rim,
            },
            outline: Outline::Circle {
                center: Point2::new(60.0, 20.0),
                radius: 5.0,
            },
            part: BodyPart::Centre,
        };
        let level = track(Point2::new(60.0, 20.0), Axis::Level);
        let upright = track(Point2::new(40.0, 20.0), Axis::Upright);
        let mut transaction = document.transaction("Drag");
        let mut shadow = sketch;

        let joined = body_joins(
            &mut transaction,
            (definition(&document, feature), feature),
            &mut shadow,
            point,
            &[
                (Holding::On, &on_edge),
                (Holding::Along(level), &round),
                (Holding::Along(upright), &middle),
                (Holding::Along(level), &on_edge),
            ],
        );
        let line = shadow
            .projected()
            .find(|id| matches!(shadow.entity(*id), Some(Entity::Line { .. })))
            .unwrap();
        let centre = shadow
            .projected()
            .find_map(|id| match shadow.entity(id) {
                Some(Entity::Circle { center, .. }) => Some(*center),
                _ => None,
            })
            .unwrap();
        let held: Vec<Option<(Constraint, bool)>> = joined
            .into_iter()
            .map(|joined| joined.map(|joined| (joined.constraint, joined.projects)))
            .collect();

        assert_eq!(
            held,
            vec![
                Some((Constraint::Coincident(point, line), true)),
                Some((
                    Constraint::HorizontalPoints(point, PointTarget::Point(centre)),
                    true
                )),
                Some((
                    Constraint::VerticalPoints(point, PointTarget::Middle(line)),
                    false
                )),
                None,
            ]
        );
        assert_eq!(shadow.projected().count(), 5);
    }
}
