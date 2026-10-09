use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, Shell, Transaction, face_plane,
};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    editing::{self, EditingCommand},
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    units::LengthUnit,
};

pub const DEFAULT_THICKNESS: f64 = 1.0;
pub const TITLE: &str = "Shell";
pub const DESCRIPTION: &str = "Hollow the body out, leaving the selected faces open";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceSource {
    pub body: FeatureId,
    pub faces: Vec<FaceKey>,
    pub left_out: Vec<Pickable>,
}

pub fn selected_faces(model: &Model, selection: &Selection) -> Result<FaceSource, &'static str> {
    let mut body = None;
    let mut faces = Vec::new();
    let mut left_out = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err("Select faces of one body only"),
                _ => body = Some(owner),
            }
            faces.push(face);
        } else {
            left_out.push(pickable);
        }
    }
    let body = body.ok_or("Select the faces of a body to leave open")?;
    let shown = bodies::shown(model.evaluation(), body)
        .ok_or("Select the faces of a body to leave open")?;
    let flat = faces.iter().all(|face| {
        bodies::find_face(shown, *face).is_some_and(|face| face_plane(&shown.solid, face).is_some())
    });
    if !flat {
        return Err("Only flat faces can be left open, so select flat faces only");
    }
    Ok(FaceSource {
        body,
        faces,
        left_out,
    })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    unit: LengthUnit,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body)
        .ok_or("The body has no shape yet; recompute the model, then try again")?;
    let open: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if open.len() < source.faces.len() {
        return Err("Some of the selected faces are no longer part of the model");
    }
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Shell(Shell {
            body: source.body,
            open,
            thickness: unit.default_length(DEFAULT_THICKNESS),
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    unit: LengthUnit,
) -> Vec<Action> {
    match create(document, evaluation, source, unit) {
        Ok((transaction, feature)) => {
            let told = body_selection::left_out_words(&source.left_out).map(|words| {
                Action::Inform(Notice::warning(format!(
                    "{TITLE} takes faces to leave open only, so {words}."
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
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
            "{TITLE}: {reason}."
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, shell: Shell) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Shell(shell),
        },
    ))
}

pub fn opened_faces(solid: &Solid, shell: &Shell) -> BTreeSet<FaceKey> {
    let opened: BTreeSet<FaceId> = shell
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    bodies::face_keys(solid)
        .into_iter()
        .filter_map(|(id, key)| opened.contains(&id).then_some(key))
        .collect()
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let shell = owner.kind.shell()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    face_plane(solid, clicked)?;
    let mut changed = shell.clone();
    changed.open = shell
        .open
        .iter()
        .zip(shell.resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().contains(&clicked))
        .map(|(reference, _)| reference.clone())
        .collect();
    let label = if changed.open.len() == shell.open.len() {
        changed.open.push(FaceReference::capture(solid, clicked)?);
        format!("Open a face of {}", owner.name)
    } else {
        format!("Close a face of {}", owner.name)
    };
    Some(Transaction::single(
        label,
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Shell(changed),
        },
    ))
}

pub fn with_selected_faces(
    model: &Model,
    feature: FeatureId,
    selection: &Selection,
) -> Option<Transaction> {
    let owner = model.document().feature(feature)?;
    let shell = owner.kind.shell()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let solid = &input.solid;
    let mut taken: BTreeSet<FaceId> = shell
        .resolutions(solid)
        .iter()
        .flat_map(|resolution| resolution.found().iter().copied())
        .collect();
    let mut changed = shell.clone();
    for pickable in selection.iter() {
        let Pickable::Face { body, face } = pickable else {
            continue;
        };
        let Some(found) = (body == shell.body)
            .then(|| bodies::find_face(input, face))
            .flatten()
            .filter(|found| !taken.contains(found) && face_plane(solid, *found).is_some())
        else {
            continue;
        };
        changed.open.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    (changed.open.len() > shell.open.len()).then(|| {
        Transaction::single(
            format!("Open the selected faces of {}", owner.name),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::Shell(changed),
            },
        )
    })
}
