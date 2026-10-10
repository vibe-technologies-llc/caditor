use caditor_document::{
    FeatureId, FeatureKind, Outline, ProjectionSource, SketchFeature, TransactionBuilder,
};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};

use crate::{
    body_snap::{BodyItem, BodyPart, BodySnaps},
    drag_solver::BodyJoin,
    model::Model,
    projecting,
    selection::{self, Pickable, Selection},
    sketch_tools,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unstaged {
    #[error("That is not part of the sketch being edited")]
    NotEdited,
    #[error("That corner or edge of {body} is no longer there; pick it again")]
    Gone { body: String },
    #[error("The centre of that round edge is not a point the sketch can use")]
    NoCentre,
}

#[derive(Debug, Clone, Default, PartialEq)]
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

#[derive(Debug, Clone)]
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
                    .words(selection::body_name(model.document(), pick.body)),
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

pub fn body_join(
    transaction: &mut TransactionBuilder<'_>,
    (definition, feature): (&SketchFeature, FeatureId),
    shadow: &mut Sketch,
    point: EntityId,
    body: &BodyJoin,
) -> Option<Joined> {
    let existing = projecting::existing_projection(definition, &body.projection);
    let curve = match existing {
        Some(id) => id,
        None => {
            let added = transaction.add_projected(feature, body.projection.clone(), &body.outline);
            for (id, entity) in added.entities {
                let inserted = shadow
                    .insert_entity(id, entity)
                    .and_then(|()| shadow.set_projected(id, true));
                if let Err(error) = inserted {
                    log::debug!("the drag check does not see projected entity {id}: {error}");
                }
            }
            added.id
        }
    };
    let constraint = match body.part {
        BodyPart::Corner => Constraint::Coincident(point, curve),
        BodyPart::Centre => Constraint::Coincident(point, centre_of(shadow, curve)?),
        BodyPart::Middle => Constraint::Midpoint { point, curve },
        BodyPart::Edge => Constraint::Coincident(point, curve),
    };
    Some(Joined {
        constraint,
        projects: existing.is_none(),
    })
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
