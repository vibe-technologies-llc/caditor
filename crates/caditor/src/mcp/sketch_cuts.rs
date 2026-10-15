use caditor_sketch::{Entity, EntityId, FitSpacing, SplineKind};
use serde::Deserialize;
use serde_json::json;

use crate::{
    mcp::{
        references::{EntityName, Named},
        sketch_edits::{Planned, SketchEditError, Target},
        tools::ToolError,
        writing,
    },
    sketch_tools::{BREAK_TITLE, ConstructionChange, RESPACE_TITLE, SPLIT_TITLE},
};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constructing {
    pub sketch: Named,
    pub items: Vec<EntityName>,
    pub construction: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Splitting {
    pub sketch: Named,
    pub curve: EntityName,
    pub point: EntityName,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Breaking {
    pub sketch: Named,
    pub curves: Vec<EntityName>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Respacing {
    pub sketch: Named,
    pub splines: Vec<EntityName>,
}

pub(super) fn construction(
    target: &Target<'_>,
    asked: &Constructing,
) -> Result<Planned, ToolError> {
    let items = target.entities(&asked.items)?;
    let shown = target.shown;
    let change = match asked.construction {
        None => ConstructionChange::of(shown, &items),
        Some(construction) => {
            let curves: Vec<EntityId> = items
                .iter()
                .copied()
                .filter(|id| {
                    shown
                        .entity(*id)
                        .is_some_and(|entity| !matches!(entity, Entity::Point(_)))
                        && shown.is_construction(*id) != construction
                })
                .collect();
            (!curves.is_empty()).then_some(ConstructionChange {
                curves,
                construction,
            })
        }
    }
    .ok_or_else(|| SketchEditError::NothingToToggle {
        sketch: target.name.to_owned(),
    })?;
    let mut transaction = target.document.transaction(change.label(shown));
    writing::settled(&mut transaction, target.evaluation, target.feature);
    change.apply(&mut transaction, target.feature);
    let mut planned = Planned::of(transaction.finish());
    planned
        .details
        .insert("construction".to_owned(), json!(change.construction));
    planned
        .details
        .insert("changed".to_owned(), writing::ids(&change.curves));
    Ok(planned)
}

pub(super) fn split(target: &Target<'_>, asked: &Splitting) -> Result<Planned, ToolError> {
    let curve = target.entity(&asked.curve)?;
    let point = target.point_of(&asked.point)?;
    let working = target.working();
    working
        .check_split(curve, point)
        .map_err(|error| ToolError::from(SketchEditError::Split(error)))?;
    let transaction = target.reshaped(SPLIT_TITLE.to_owned(), |sketch| {
        sketch
            .split_at(curve, point)
            .map(|_| ())
            .map_err(|error| SketchEditError::Split(error).into())
    })?;
    Ok(Planned::of(transaction))
}

pub(super) fn break_curves(target: &Target<'_>, asked: &Breaking) -> Result<Planned, ToolError> {
    let curves = target.entities(&asked.curves)?;
    let transaction = target.reshaped(BREAK_TITLE.to_owned(), |sketch| {
        sketch
            .break_curves(&curves)
            .map(|_| ())
            .map_err(|error| SketchEditError::Break(error).into())
    })?;
    Ok(Planned::of(transaction))
}

pub(super) fn respace(target: &Target<'_>, asked: &Respacing) -> Result<Planned, ToolError> {
    let splines = target.entities(&asked.splines)?;
    let uneven: Vec<EntityId> = splines
        .into_iter()
        .filter(|id| {
            !id.is_reference()
                && !target.shown.is_projected(*id)
                && matches!(
                    target.shown.entity(*id).and_then(Entity::spline_kind),
                    Some(SplineKind::Fit {
                        spacing: FitSpacing::Even,
                        ..
                    })
                )
        })
        .collect();
    if uneven.is_empty() {
        return Err(SketchEditError::NothingToRespace {
            sketch: target.name.to_owned(),
        }
        .into());
    }
    let transaction = target.reshaped(RESPACE_TITLE.to_owned(), |sketch| {
        for spline in &uneven {
            sketch
                .respace_fit_spline(*spline)
                .map_err(|error| ToolError::from(SketchEditError::Respace(error)))?;
        }
        Ok(())
    })?;
    let mut planned = Planned::of(transaction);
    planned
        .details
        .insert("respaced".to_owned(), writing::ids(&uneven));
    Ok(planned)
}
