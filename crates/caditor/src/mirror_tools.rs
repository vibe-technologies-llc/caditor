use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, Mirror, PlaneReference, PrincipalPlane,
    Transaction, describe_plane,
};
use caditor_kernel::FaceReference;

use crate::{
    bodies::{self, FaceKey},
    datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model, Notice},
    move_tools, pattern_tools,
    selection::{Pickable, Selection},
};

pub const TITLE: &str = "Mirror body";
pub const FEATURES_TITLE: &str = "Mirror features";
pub const FACES_TITLE: &str = "Mirror faces";
pub const DESCRIPTION: &str =
    "Mirror a body across a plane or flat face, joined to the original or in its place";
const NO_FACES: &str = "Select the faces of a body to mirror, such as a pocket's walls and floor";
const ONE_BODY: &str = "Select faces of one body only";
const NO_SHAPE: &str = "The body has no shape yet; recompute the model, then try again";
const FACES_GONE: &str = "Some of the selected faces are no longer part of the model";
const NO_SELECTED_FACES: &str = "Select faces of the mirrored body in the view first";
const LATER_FACES: &str = "Some selected faces are not part of the body as the mirror finds it; \
                           select faces made before the mirror";
const ALREADY_FACES: &str = "The mirror already mirrors the selected faces";
const NO_BODY: &str = "Select a face or edge of the body to mirror";
const NO_PLANE: &str = "Select a plane or flat face made before this feature";
const ALREADY: &str = "The body is already mirrored across the selected plane or face";
const GONE: &str = "The feature no longer exists";
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Yz);

#[derive(Debug, Clone, PartialEq)]
pub struct MirrorSource {
    pub body: FeatureId,
    pub plane: Option<PlaneReference>,
    pub mirrored: Vec<FeatureId>,
}

impl MirrorSource {
    pub fn subject(&self, document: &Document) -> String {
        pattern_tools::subject(document, self.body, &self.mirrored)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceMirrorSource {
    pub body: FeatureId,
    pub faces: Vec<FaceKey>,
    pub plane: Option<PlaneReference>,
}

pub fn faces_source(
    model: &Model,
    selection: &Selection,
) -> Result<FaceMirrorSource, &'static str> {
    let mut body = None;
    let mut faces = Vec::new();
    let mut others = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err(ONE_BODY),
                _ => body = Some(owner),
            }
            faces.push(face);
        } else {
            others.push(pickable);
        }
    }
    let body = body.ok_or(NO_FACES)?;
    let mut rest = Selection::default();
    rest.extend(others);
    bodies::shown(model.evaluation(), body).ok_or(NO_SHAPE)?;
    let plane = datum_tools::chosen_plane(model, &rest, model.document().bar_index())?
        .map(|chosen| chosen.plane);
    Ok(FaceMirrorSource { body, faces, plane })
}

pub fn create_faces(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceMirrorSource,
) -> Result<(Transaction, FeatureId), &'static str> {
    let shown = bodies::shown(evaluation, source.body).ok_or(NO_SHAPE)?;
    let faces: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < source.faces.len() {
        return Err(FACES_GONE);
    }
    let name = editing::next_feature_name(document, FACES_TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Mirror(
            Mirror::new(source.body, source.plane.clone().unwrap_or(DEFAULT_PLANE))
                .mirroring_faces(faces),
        ),
    );
    Ok((transaction.finish(), feature))
}

pub fn create_faces_actions(model: &Model, source: &FaceMirrorSource) -> Vec<Action> {
    match create_faces(model.document(), model.evaluation(), source) {
        Ok((transaction, feature)) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::OpenSolid(feature)),
        ],
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
            "{FACES_TITLE}: {reason}."
        )))],
    }
}

pub fn selected_faces(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mirror: &Mirror,
) -> Result<Vec<FaceReference>, &'static str> {
    let keys: Vec<FaceKey> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, face } if body == mirror.body => Some(face),
            _ => None,
        })
        .collect();
    if keys.is_empty() {
        return Err(NO_SELECTED_FACES);
    }
    let seen = model
        .evaluation()
        .body_result_seen_by(feature, mirror.body)
        .ok_or(NO_SHAPE)?;
    let faces: Vec<FaceReference> = keys
        .iter()
        .filter_map(|key| FaceReference::capture(&seen.solid, bodies::find_face(seen, *key)?))
        .collect();
    if faces.len() < keys.len() {
        return Err(LATER_FACES);
    }
    if faces == mirror.faces {
        return Err(ALREADY_FACES);
    }
    Ok(faces)
}

pub fn faces_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mirror: &Mirror,
) -> Result<Transaction, String> {
    let faces = selected_faces(model, selection, feature, mirror)?;
    mirroring_faces(model, feature, mirror, faces)
}

pub fn mirroring_faces(
    model: &Model,
    feature: FeatureId,
    mirror: &Mirror,
    faces: Vec<FaceReference>,
) -> Result<Transaction, String> {
    change(
        model,
        feature,
        Mirror {
            mirrored: Vec::new(),
            faces,
            ..mirror.clone()
        },
    )
}

pub fn features_hover(document: &Document, source: &MirrorSource) -> String {
    let subject = source.subject(document);
    match &source.plane {
        Some(plane) => format!(
            "Mirror {subject} across {}",
            describe_plane(document, plane)
        ),
        None => format!(
            "Mirror {subject} across {}, or across a plane or flat face you select first",
            describe_plane(document, &DEFAULT_PLANE)
        ),
    }
}

pub fn source(
    model: &Model,
    selection: &Selection,
    (tree, rows): (&[FeatureId], &[FeatureId]),
) -> Result<MirrorSource, &'static str> {
    let document = model.document();
    let chosen = datum_tools::chosen_plane(model, selection, document.bar_index())?;
    let face = chosen.as_ref().and_then(|chosen| chosen.face);
    let (body, mirrored) = match pattern_tools::repeatable(document, rows) {
        Some(features) => features,
        None => (
            move_tools::chosen_body_beside(model, selection, tree, face, NO_BODY)?,
            Vec::new(),
        ),
    };
    Ok(MirrorSource {
        body,
        plane: chosen.map(|chosen| chosen.plane),
        mirrored,
    })
}

pub fn create(document: &Document, source: &MirrorSource) -> (Transaction, FeatureId) {
    let title = if source.mirrored.is_empty() {
        TITLE
    } else {
        FEATURES_TITLE
    };
    let name = editing::next_feature_name(document, title);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Mirror(
            Mirror::new(source.body, source.plane.clone().unwrap_or(DEFAULT_PLANE))
                .mirroring(source.mirrored.clone()),
        ),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: &MirrorSource) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, mirror: Mirror) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Mirror(mirror),
        },
    ))
}

pub fn change(model: &Model, feature: FeatureId, mirror: Mirror) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, mirror).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn plane_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mirror: &Mirror,
) -> Result<Transaction, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let plane = datum_tools::chosen_plane(model, selection, index)?
        .ok_or(NO_PLANE)?
        .plane;
    if plane == mirror.plane {
        return Err(ALREADY.to_owned());
    }
    change(
        model,
        feature,
        Mirror {
            plane,
            ..mirror.clone()
        },
    )
}

pub fn mirroring(
    model: &Model,
    feature: FeatureId,
    mirror: &Mirror,
    mirrored: Vec<FeatureId>,
) -> Result<Transaction, String> {
    change(
        model,
        feature,
        Mirror {
            mirrored,
            faces: Vec::new(),
            ..mirror.clone()
        },
    )
}
