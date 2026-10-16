use std::{collections::BTreeSet, fmt::Display};

use caditor_expression::Dimension;
use caditor_geometry::Vector2;
use caditor_sketch::{Drag, Entity, EntityId, MAX_LENGTH, Sketch};
use serde::Deserialize;
use serde_json::json;

use crate::{
    drag_solver::DragCommand,
    mcp::{
        references::{EntityName, Named},
        session::{Cancelled, Session, Work},
        sketch_edits::{Planned, SketchEditError, Target, amount},
        tools::ToolError,
        writing::{coordinate, point},
    },
    sketch_drag::{self, Transform, Transforming},
    sketch_status::up_to_date_result,
};

const REACHED_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Moving {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub by: Option<[f64; 2]>,
    pub to: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turning {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub angle: String,
    pub about: Option<EntityName>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scaling {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub factor: f64,
    pub about: Option<EntityName>,
}

fn solved_shape<'s>(target: &Target<'s>) -> Result<&'s Sketch, ToolError> {
    up_to_date_result(target.evaluation, target.feature)
        .map(|result| &result.geometry)
        .ok_or_else(|| {
            SketchEditError::NotSolved {
                sketch: target.name.to_owned(),
            }
            .into()
        })
}

fn unmovable(reason: impl Display) -> ToolError {
    SketchEditError::Unmovable {
        reason: reason.to_string(),
    }
    .into()
}

pub(super) fn move_geometry(
    session: &Session,
    work: &Work<'_>,
    target: &Target<'_>,
    asked: &Moving,
) -> Result<Planned, ToolError> {
    let items = target.entities(&asked.items)?;
    let shown = solved_shape(target)?;
    let moving = sketch_drag::Moving::of(shown, target.feature, &items).map_err(unmovable)?;
    let destination = match (asked.by, asked.to) {
        (Some([x, y]), None) => moving.anchor + Vector2::new(coordinate(x)?, coordinate(y)?),
        (None, Some(to)) => point(to)?,
        _ => return Err(SketchEditError::MoveTarget.into()),
    };
    dragged(session, work, target, shown, moving.to(destination))
}

fn transformed(
    target: &Target<'_>,
    shown: &Sketch,
    asked_items: &[EntityName],
    about: Option<&EntityName>,
    transform: Transform,
) -> Result<Transforming, ToolError> {
    let items = target.entities(asked_items)?;
    let Some(about) = about else {
        return Transforming::of(shown, target.feature, &items, transform).map_err(unmovable);
    };
    let pivot = target.point_of(about)?;
    let moved_points: BTreeSet<EntityId> = items
        .iter()
        .flat_map(|item| match shown.entity(*item) {
            Some(Entity::Point(_)) => vec![*item],
            Some(other) => other.points(),
            None => Vec::new(),
        })
        .collect();
    if moved_points.contains(&pivot) {
        return Err(SketchEditError::AboutMoves {
            sketch: target.name.to_owned(),
            entity: about.clone(),
        }
        .into());
    }
    let pivot_at = shown
        .point(pivot)
        .ok_or_else(|| SketchEditError::NotAPoint {
            sketch: target.name.to_owned(),
            entity: about.clone(),
        })?;
    let selected: Vec<EntityId> = items.into_iter().chain([pivot]).collect();
    let mut transforming =
        Transforming::of(shown, target.feature, &selected, transform).map_err(unmovable)?;
    transforming.pivot = pivot_at;
    Ok(transforming)
}

pub(super) fn rotate(
    session: &Session,
    work: &Work<'_>,
    target: &Target<'_>,
    asked: &Turning,
) -> Result<Planned, ToolError> {
    let shown = solved_shape(target)?;
    let angle = amount(session, "angle", &asked.angle, Dimension::ANGLE)?;
    let transforming = transformed(
        target,
        shown,
        &asked.items,
        asked.about.as_ref(),
        Transform::Rotate,
    )?;
    let commands = transforming.rotated(angle.value).map_err(unmovable)?;
    dragged(session, work, target, shown, commands)
}

pub(super) fn scale(
    session: &Session,
    work: &Work<'_>,
    target: &Target<'_>,
    asked: &Scaling,
) -> Result<Planned, ToolError> {
    if !(asked.factor.is_finite() && asked.factor > 0.0) {
        return Err(SketchEditError::Factor(asked.factor).into());
    }
    let shown = solved_shape(target)?;
    let transforming = transformed(
        target,
        shown,
        &asked.items,
        asked.about.as_ref(),
        Transform::Scale,
    )?;
    let commands = transforming.scaled(asked.factor).map_err(unmovable)?;
    dragged(session, work, target, shown, commands)
}

fn dragged(
    session: &Session,
    work: &Work<'_>,
    target: &Target<'_>,
    shown: &Sketch,
    commands: Vec<DragCommand>,
) -> Result<Planned, ToolError> {
    let mut label = String::new();
    let mut drags = Vec::new();
    for command in commands {
        if let DragCommand::Move {
            label: named,
            drags: moves,
            ..
        } = command
        {
            label = named;
            drags = moves;
        }
    }
    let beyond = drags.iter().any(|drag| match drag {
        Drag::Point { to, .. } => !to.is_finite() || to.abs().max_element() > MAX_LENGTH,
        Drag::Radius { to, .. } => !to.is_finite() || *to > MAX_LENGTH,
    });
    if beyond {
        return Err(SketchEditError::OutOfReach.into());
    }
    let parameters = session.parameters();
    let solved = shown.solve_geometry_from(
        &|id| parameters.value(id),
        &|| work.is_cancelled(),
        &drags,
        None,
    );
    let geometry = match solved {
        Ok((geometry, _)) => geometry,
        Err(_) if work.is_cancelled() => return Err(Cancelled.into()),
        Err(error) => return Err(SketchEditError::Unsolved(error).into()),
    };
    let mut transaction = target.document.transaction(label);
    transaction.settle_sketch(target.feature, &geometry);
    let transaction = transaction.finish();
    if transaction.is_empty() {
        return Err(SketchEditError::Held {
            sketch: target.name.to_owned(),
        }
        .into());
    }
    let reached = drags.iter().all(|drag| match drag {
        Drag::Point { point, to } => geometry.point(*point).is_some_and(|at| {
            at.distance(*to) <= REACHED_TOLERANCE * to.abs().max_element().max(1.0)
        }),
        Drag::Radius { circle, to } => match geometry.entity(*circle) {
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
            ) => (radius - to).abs() <= REACHED_TOLERANCE * to.max(1.0),
            _ => false,
        },
    });
    let mut planned = Planned::of(transaction);
    planned.details.insert("reached".to_owned(), json!(reached));
    if !reached {
        planned.details.insert(
            "note".to_owned(),
            json!(
                "constraints kept part of it from going all the way; get_sketch shows where it is"
            ),
        );
    }
    Ok(planned)
}
