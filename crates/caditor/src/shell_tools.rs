use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Evaluation, FaceThickness, FeatureId, FeatureKind, Shell, ShellSide,
    Transaction, openable_face,
};
use caditor_kernel::{FaceId, FaceReference, Solid};

use crate::{
    bodies::{self, FaceKey},
    body_selection,
    commands::Command,
    editing::{self, EditingCommand},
    field,
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    queued::{self, Rerun},
    selection::{Pickable, Selection},
};

pub const DEFAULT_THICKNESS: f64 = 1.0;
pub const TITLE: &str = "Shell";
const OWN_THICKNESS_TITLE: &str = "Own thickness";
pub const DESCRIPTION: &str = "Hollow the body out, leaving the selected faces open";
const NOT_OPENABLE: &str = "Only flat faces and cylinder, cone, sphere or torus faces can be left open, so select those \
     only";
const GONE: &str = "The feature no longer exists";
const NO_SHAPE: &str = queued::BODY_SHAPE;
const NOT_A_FACE: &str = "Click a face of the body being shelled";
const OPENED: &str = "That face is open, so it has no wall; close it first to give it its own \
                      thickness";
const NOT_OFFSET: &str = "Only flat faces and cylinder, cone, sphere or torus faces can take \
                          their own thickness";

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
    let openable = faces.iter().all(|face| {
        bodies::find_face(shown, *face).is_some_and(|face| openable_face(&shown.solid, face))
    });
    if !openable {
        return Err(NOT_OPENABLE);
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
    starts: &Starts,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body).ok_or(queued::BODY_SHAPE)?;
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
            thickness: starts.length(Remembered::ShellThickness, DEFAULT_THICKNESS),
            side: ShellSide::Inside,
            faces: Vec::new(),
        }),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    starts: &Starts,
) -> Vec<Action> {
    match create(document, evaluation, source, starts) {
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
        Err(reason) => vec![queued::refused(Command::Shell, TITLE, reason)],
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

fn without_own_thickness(shell: &mut Shell, solid: &Solid, opened: &BTreeSet<FaceId>) {
    let kept: Vec<FaceThickness> = shell
        .faces
        .iter()
        .zip(shell.wall_resolutions(solid))
        .filter(|(_, resolution)| !resolution.found().iter().any(|face| opened.contains(face)))
        .map(|(entry, _)| entry.clone())
        .collect();
    shell.faces = kept;
}

pub fn toggle_face(model: &Model, feature: FeatureId, face: FaceKey) -> Option<Transaction> {
    let document = model.document();
    let owner = document.feature(feature)?;
    let shell = owner.kind.shell()?;
    let input = bodies::input(model.evaluation(), feature)?;
    let clicked = bodies::find_face(input, face)?;
    let solid = &input.solid;
    if !openable_face(solid, clicked) {
        return None;
    }
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
        without_own_thickness(&mut changed, solid, &BTreeSet::from([clicked]));
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
            .filter(|found| !taken.contains(found) && openable_face(solid, *found))
        else {
            continue;
        };
        changed.open.push(FaceReference::capture(solid, found)?);
        taken.insert(found);
    }
    without_own_thickness(&mut changed, solid, &taken);
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

pub fn side_change(
    document: &Document,
    feature: FeatureId,
    side: ShellSide,
) -> Option<Transaction> {
    let owner = document.feature(feature)?;
    let shell = owner.kind.shell()?;
    let changed = Shell {
        side,
        ..shell.clone()
    };
    Some(Transaction::single(
        format!(
            "Set the walls of {} to {}",
            owner.name,
            side.label().to_lowercase()
        ),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Shell(changed),
        },
    ))
}

fn own_thickness_toggled(
    model: &Model,
    feature: FeatureId,
    clicked: FaceId,
) -> Result<Transaction, String> {
    let document = model.document();
    let owner = document.feature(feature).ok_or(GONE)?;
    let shell = owner.kind.shell().ok_or(GONE)?;
    let input = bodies::input(model.evaluation(), feature).ok_or(NO_SHAPE)?;
    let solid = &input.solid;
    let opened = shell
        .resolutions(solid)
        .iter()
        .any(|resolution| resolution.found().contains(&clicked));
    if opened {
        return Err(OPENED.to_owned());
    }
    if !openable_face(solid, clicked) {
        return Err(NOT_OFFSET.to_owned());
    }
    let mut changed = shell.clone();
    without_own_thickness(&mut changed, solid, &BTreeSet::from([clicked]));
    let label = if changed.faces.len() == shell.faces.len() {
        changed.faces.push(FaceThickness {
            face: FaceReference::capture(solid, clicked).ok_or(NO_SHAPE)?,
            thickness: shell.thickness.clone(),
        });
        format!("Give a face of {} its own thickness", owner.name)
    } else {
        format!("Give a face of {} the shell's thickness", owner.name)
    };
    field::checked(
        document,
        Transaction::single(
            label,
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::Shell(changed),
            },
        ),
    )
}

fn clicked_face(model: &Model, feature: FeatureId, pickable: Pickable) -> Option<FaceId> {
    let shell = model.document().feature(feature)?.kind.shell()?;
    let input = bodies::input(model.evaluation(), feature)?;
    match pickable {
        Pickable::ShellFace {
            feature: owner,
            face,
        } if owner == feature => bodies::find_face(input, face),
        Pickable::Face { body, face } if body == shell.body => bodies::find_face(input, face),
        _ => None,
    }
}

pub fn own_thickness_click(model: &Model, feature: FeatureId, pickable: Pickable) -> Vec<Action> {
    if bodies::input(model.evaluation(), feature).is_none() {
        let rerun = Rerun::OwnThickness { feature, pickable };
        return vec![queued::refused_rerun(rerun, OWN_THICKNESS_TITLE, NO_SHAPE)];
    }
    let toggled = clicked_face(model, feature, pickable)
        .ok_or_else(|| NOT_A_FACE.to_owned())
        .and_then(|face| own_thickness_toggled(model, feature, face));
    match toggled {
        Ok(transaction) => vec![Action::Apply(transaction)],
        Err(reason) => vec![Action::Inform(Notice::warning(format!("{reason}.")))],
    }
}

pub fn own_thickness_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
) -> Result<Transaction, String> {
    let mut picked = selection
        .iter()
        .filter_map(|pickable| clicked_face(model, feature, pickable));
    match (picked.next(), picked.next()) {
        (Some(face), None) => own_thickness_toggled(model, feature, face),
        _ => Err(NOT_A_FACE.to_owned()),
    }
}
