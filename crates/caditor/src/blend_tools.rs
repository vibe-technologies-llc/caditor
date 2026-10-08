use std::collections::BTreeSet;

use caditor_document::{
    Blend, BlendKind, Document, Edit, Evaluation, FeatureId, FeatureKind, Transaction,
};
use caditor_kernel::{EdgeId, EdgeName, EdgeNaming, EdgeReference, Solid, blend_chain};

use crate::{
    bodies, body_selection,
    editing::{self, EditingCommand},
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    units::LengthUnit,
};

pub const DEFAULT_SIZE: f64 = 1.0;
const NO_SHAPE: &str = "The body has no shape yet; recompute the model, then try again";

pub const KINDS: [BlendKind; 2] = [BlendKind::Fillet, BlendKind::Chamfer];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeSource {
    pub body: FeatureId,
    pub edges: Vec<EdgeName>,
    pub left_out: Vec<Pickable>,
}

pub fn selected_edges(selection: &Selection) -> Result<EdgeSource, &'static str> {
    let mut body = None;
    let mut edges = Vec::new();
    let mut left_out = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Edge { body: owner, edge } = pickable {
            match body {
                Some(known) if known != owner => return Err("Select edges of one body only"),
                _ => body = Some(owner),
            }
            edges.push(edge);
        } else {
            left_out.push(pickable);
        }
    }
    match body {
        Some(body) => Ok(EdgeSource {
            body,
            edges,
            left_out,
        }),
        None => Err("Select the edges of a body first"),
    }
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    kind: BlendKind,
    source: &EdgeSource,
    unit: LengthUnit,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body).ok_or(NO_SHAPE)?;
    let naming = EdgeNaming::new(&shown.solid);
    let edges: Vec<EdgeReference> = source
        .edges
        .iter()
        .filter_map(|name| {
            let edge = bodies::find_edge(shown, *name)?;
            EdgeReference::capture_in(&naming, edge)
        })
        .collect();
    if edges.is_empty() {
        return Err("The selected edges are no longer part of the model");
    }
    if edges.len() < source.edges.len() {
        return Err("Some of the selected edges are no longer part of the model");
    }
    let name = editing::next_feature_name(document, kind.title());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Blend(Blend {
            kind,
            body: source.body,
            edges,
            size: unit.default_length(DEFAULT_SIZE),
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    kind: BlendKind,
    source: &EdgeSource,
    unit: LengthUnit,
) -> Vec<Action> {
    match create(document, evaluation, kind, source, unit) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::info(format!(
                    "{} takes edges only, so {words}.",
                    kind.title()
                )))
            });
            [
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ]
            .into_iter()
            .chain(told)
            .collect()
        }
        Err(reason) => vec![Action::Inform(Notice::info(format!(
            "{}: {reason}.",
            kind.title()
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, blend: Blend) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Blend(blend),
        },
    ))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChosenEdges {
    pub explicit: BTreeSet<EdgeName>,
    pub followed: BTreeSet<EdgeName>,
}

pub fn chosen_edges(solid: &Solid, blend: &Blend) -> ChosenEdges {
    let name = |edge: &EdgeId| solid.edge(*edge).map(|edge| edge.name());
    let explicit: Vec<EdgeId> = blend
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let followed = blend_chain(solid, &explicit)
        .iter()
        .filter_map(name)
        .collect();
    ChosenEdges {
        explicit: explicit.iter().filter_map(name).collect(),
        followed,
    }
}

pub fn toggle_edge(model: &Model, feature: FeatureId, edge: EdgeName) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let blend = owner.kind.blend()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_edge(input, edge)?;
    let solid = &input.solid;
    let mut changed = blend.clone();
    changed.edges = blend
        .edges
        .iter()
        .zip(blend.resolutions(solid))
        .filter(|(_, resolution)| !blend_chain(solid, resolution.found()).contains(&clicked))
        .map(|(reference, _)| *reference)
        .collect();
    let label = if changed.edges.len() == blend.edges.len() {
        changed
            .edges
            .push(EdgeReference::capture_in(&EdgeNaming::new(solid), clicked)?);
        format!("Add an edge to {}", owner.name)
    } else {
        format!("Leave an edge out of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Blend(changed),
        },
    ))
}

pub fn with_selected_edges(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let blend = owner.kind.blend()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let naming = EdgeNaming::new(solid);
    let mut taken: BTreeSet<EdgeId> = blend
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| blend_chain(solid, resolution.found()))
        .collect();
    let mut changed = blend.clone();
    for pickable in selection.iter() {
        let Pickable::Edge { body, edge } = pickable else {
            continue;
        };
        let Some(found) = (body == blend.body)
            .then(|| bodies::find_edge(input, edge))
            .flatten()
            .filter(|found| !taken.contains(found))
        else {
            continue;
        };
        changed
            .edges
            .push(EdgeReference::capture_in(&naming, found)?);
        taken.extend(blend_chain(solid, &[found]));
    }
    (changed.edges.len() > blend.edges.len()).then(|| {
        Transaction::single(
            format!("Add the selected edges to {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::Blend(changed),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use caditor_document::{CancelToken, ModelEvaluator, Recompute};

    use super::*;
    use crate::samples::Sample;

    #[test]
    fn a_fillet_is_refused_when_any_selected_edge_is_gone() {
        let document = Sample::Plate.document().unwrap();

        let evaluation = Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        let (body, _) = evaluation.bodies().next().unwrap();
        let solid = evaluation.body(body).unwrap();
        let present = solid.edges().next().map(|(_, edge)| edge.name()).unwrap();
        let gone = EdgeName::from_digest(7);
        let create = |edges: Vec<EdgeName>| {
            create(
                &document,
                &evaluation,
                BlendKind::Fillet,
                &EdgeSource {
                    body,
                    edges,
                    left_out: Vec::new(),
                },
                LengthUnit::Millimetre,
            )
            .map(|_| ())
        };

        assert_eq!(create(vec![present]), Ok(()));
        assert_eq!(
            create(vec![present, gone]),
            Err("Some of the selected edges are no longer part of the model")
        );
        assert_eq!(
            create(vec![gone]),
            Err("The selected edges are no longer part of the model")
        );
    }
}
