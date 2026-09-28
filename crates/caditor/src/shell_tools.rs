use std::collections::BTreeSet;

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, Shell, Transaction, face_plane,
};
use caditor_kernel::{FaceId, FaceReference, ReferenceError, Solid};

use crate::{
    bodies::{self, FaceKey},
    editing::{self, EditingCommand},
    model::{Action, Model},
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
}

pub fn selected_faces(model: &Model, selection: &Selection) -> Result<FaceSource, &'static str> {
    let mut body = None;
    let mut faces = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err("Select faces of one body only"),
                _ => body = Some(owner),
            }
            faces.push(face);
        }
    }
    let body = body.ok_or("Select the faces of a body to leave open")?;
    let solid = model
        .evaluation()
        .body(body)
        .ok_or("Select the faces of a body to leave open")?;
    let flat = faces.iter().all(|face| {
        bodies::find_face(solid, *face).is_some_and(|face| face_plane(solid, face).is_some())
    });
    if !flat {
        return Err("Only flat faces can be left open, so select flat faces only");
    }
    Ok(FaceSource { body, faces })
}

pub fn create(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    unit: LengthUnit,
) -> Option<(Transaction, FeatureId)> {
    let solid = evaluation.body(source.body)?;
    let open: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(solid, bodies::find_face(solid, *key)?))
        .collect();
    if open.len() < source.faces.len() {
        return None;
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
    Some((transaction.finish(), feature))
}

pub fn create_actions(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceSource,
    unit: LengthUnit,
) -> Vec<Action> {
    let Some((transaction, feature)) = create(document, evaluation, source, unit) else {
        return Vec::new();
    };
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
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

pub fn resolved(solid: &Solid, reference: &FaceReference) -> Vec<FaceId> {
    match reference.resolve(solid) {
        Ok(face) => vec![face],
        Err(ReferenceError::Ambiguous(pieces)) => pieces,
        Err(ReferenceError::Missing) => Vec::new(),
    }
}

pub fn opened_faces(solid: &Solid, shell: &Shell) -> BTreeSet<FaceKey> {
    let opened: BTreeSet<FaceId> = shell
        .open
        .iter()
        .flat_map(|reference| resolved(solid, reference))
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
    let solid = bodies::input_solid(model.evaluation(), feature)?;
    let clicked = bodies::find_face(solid, face)?;
    face_plane(solid, clicked)?;
    let mut changed = shell.clone();
    changed
        .open
        .retain(|reference| !resolved(solid, reference).contains(&clicked));
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
