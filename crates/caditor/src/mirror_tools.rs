use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Mirror, PlaneReference, PrincipalPlane, Transaction,
};

use crate::{
    datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    move_tools,
    selection::Selection,
};

pub const TITLE: &str = "Mirror body";
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
}

pub fn source(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
) -> Result<MirrorSource, &'static str> {
    let chosen = datum_tools::chosen_plane(model, selection, model.document().bar_index())?;
    let face = chosen.as_ref().and_then(|chosen| chosen.face);
    let body = move_tools::chosen_body_beside(model, selection, tree, face, NO_BODY)?;
    Ok(MirrorSource {
        body,
        plane: chosen.map(|chosen| chosen.plane),
    })
}

pub fn create(document: &Document, source: &MirrorSource) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Mirror(Mirror {
            body: source.body,
            plane: source.plane.clone().unwrap_or(DEFAULT_PLANE),
            keep_original: true,
        }),
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
