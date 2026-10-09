use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Mirror, PlaneReference, PrincipalPlane, Transaction,
    describe_plane,
};

use crate::{
    datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    move_tools, pattern_tools,
    selection::Selection,
};

pub const TITLE: &str = "Mirror body";
pub const FEATURES_TITLE: &str = "Mirror features";
pub const DESCRIPTION: &str =
    "Mirror a body across a plane or flat face, joined to the original or in its place";
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
            ..mirror.clone()
        },
    )
}
