use std::borrow::Cow;

use caditor_document::{
    Document, Edit, Evaluation, FeatureId, FeatureKind, Mirror, PlaneReference, PrincipalPlane,
    Transaction, describe_plane,
};
use caditor_kernel::FaceReference;

use crate::{
    bodies::{self, FaceKey},
    commands::{Command, Reason},
    datum_tools,
    editing::{self, EditingCommand},
    field::{self, CheckError},
    model::{Action, Model},
    move_tools, pattern_tools, queued,
    selection::{Pickable, Selection},
};

pub const TITLE: &str = "Mirror body";
pub const FEATURES_TITLE: &str = "Mirror features";
pub const FACES_TITLE: &str = "Mirror faces";
pub const DESCRIPTION: &str =
    "Mirror a body across a plane or flat face, joined to the original or in its place";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MirrorToolError {
    #[error("Select the faces of a body to mirror, such as a pocket's walls and floor")]
    NoFaces,
    #[error("Select faces of one body only")]
    OneBodyOnly,
    #[error("{}", queued::BODY_SHAPE)]
    NoShape,
    #[error("Some of the selected faces are no longer part of the model")]
    FacesGone,
    #[error("Select faces of the mirrored body in the view first")]
    NoSelectedFaces,
    #[error(
        "Some selected faces are not part of the body as the mirror finds it; select faces made before the mirror"
    )]
    LaterFaces,
    #[error("The mirror already mirrors the selected faces")]
    AlreadyFaces,
    #[error("Select a plane or flat face made before this feature")]
    NoPlane,
    #[error("The body is already mirrored across the selected plane or face")]
    Already,
    #[error("The feature no longer exists")]
    FeatureGone,
    #[error("{words}")]
    ChosenPlane { words: &'static str },
    #[error(transparent)]
    ChosenBody(#[from] move_tools::BodyChoiceError),
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for MirrorToolError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

impl From<MirrorToolError> for String {
    fn from(error: MirrorToolError) -> Self {
        error.to_string()
    }
}

const NO_BODY: &str = "Select a face or edge of the body to mirror";
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
) -> Result<FaceMirrorSource, MirrorToolError> {
    let mut body = None;
    let mut faces = Vec::new();
    let mut others = Vec::new();
    for pickable in selection.iter() {
        if let Pickable::Face { body: owner, face } = pickable {
            match body {
                Some(known) if known != owner => return Err(MirrorToolError::OneBodyOnly),
                _ => body = Some(owner),
            }
            faces.push(face);
        } else {
            others.push(pickable);
        }
    }
    let body = body.ok_or(MirrorToolError::NoFaces)?;
    let mut rest = Selection::default();
    rest.extend(others);
    bodies::shown(model.evaluation(), body).ok_or(MirrorToolError::NoShape)?;
    let plane = datum_tools::chosen_plane(model, &rest, model.document().bar_index())
        .map_err(|words| MirrorToolError::ChosenPlane { words })?
        .map(|chosen| chosen.plane);
    Ok(FaceMirrorSource { body, faces, plane })
}

pub fn create_faces(
    document: &Document,
    evaluation: &Evaluation,
    source: &FaceMirrorSource,
) -> Result<(Transaction, FeatureId), MirrorToolError> {
    let shown = bodies::shown(evaluation, source.body).ok_or(MirrorToolError::NoShape)?;
    let faces: Vec<FaceReference> = source
        .faces
        .iter()
        .filter_map(|key| FaceReference::capture(&shown.solid, bodies::find_face(shown, *key)?))
        .collect();
    if faces.len() < source.faces.len() {
        return Err(MirrorToolError::FacesGone);
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
        Err(error) => vec![queued::refused(
            Command::MirrorFaces,
            FACES_TITLE,
            &error.to_string(),
        )],
    }
}

pub fn selected_faces(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mirror: &Mirror,
) -> Result<Vec<FaceReference>, MirrorToolError> {
    let keys: Vec<FaceKey> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, face } if body == mirror.body => Some(face),
            _ => None,
        })
        .collect();
    if keys.is_empty() {
        return Err(MirrorToolError::NoSelectedFaces);
    }
    let seen = model
        .evaluation()
        .body_result_seen_by(feature, mirror.body)
        .ok_or(MirrorToolError::NoShape)?;
    let faces: Vec<FaceReference> = keys
        .iter()
        .filter_map(|key| FaceReference::capture(&seen.solid, bodies::find_face(seen, *key)?))
        .collect();
    if faces.len() < keys.len() {
        return Err(MirrorToolError::LaterFaces);
    }
    if faces == mirror.faces {
        return Err(MirrorToolError::AlreadyFaces);
    }
    Ok(faces)
}

pub fn faces_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mirror: &Mirror,
) -> Result<Transaction, MirrorToolError> {
    let faces = selected_faces(model, selection, feature, mirror)?;
    mirroring_faces(model, feature, mirror, faces)
}

pub fn mirroring_faces(
    model: &Model,
    feature: FeatureId,
    mirror: &Mirror,
    faces: Vec<FaceReference>,
) -> Result<Transaction, MirrorToolError> {
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
) -> Result<MirrorSource, MirrorToolError> {
    let document = model.document();
    let chosen = datum_tools::chosen_plane(model, selection, document.bar_index())
        .map_err(|words| MirrorToolError::ChosenPlane { words })?;
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

pub fn change(
    model: &Model,
    feature: FeatureId,
    mirror: Mirror,
) -> Result<Transaction, MirrorToolError> {
    let document = model.document();
    let transaction = edit(document, feature, mirror).ok_or(MirrorToolError::FeatureGone)?;
    field::checked(document, transaction)
}

pub fn plane_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    mirror: &Mirror,
) -> Result<Transaction, MirrorToolError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(MirrorToolError::FeatureGone)?;
    let plane = datum_tools::chosen_plane(model, selection, index)
        .map_err(|words| MirrorToolError::ChosenPlane { words })?
        .ok_or(MirrorToolError::NoPlane)?
        .plane;
    if plane == mirror.plane {
        return Err(MirrorToolError::Already);
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
) -> Result<Transaction, MirrorToolError> {
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
