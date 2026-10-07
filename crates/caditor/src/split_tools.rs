use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, PlaneReference, PrincipalPlane, Split, Transaction,
};

use crate::{
    datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model},
    move_tools,
    selection::{Pickable, Selection},
};

const NO_BODY: &str = "Select a face or edge of the body to split";
const NO_PLANE: &str = "Select a plane or flat face made before this feature";
const ALREADY: &str = "The body is already split along the selected plane or face";
const GONE: &str = "The feature no longer exists";
const DEFAULT_PLANE: PlaneReference = PlaneReference::Principal(PrincipalPlane::Yz);

#[derive(Debug, Clone, PartialEq)]
pub struct SplitSource {
    pub body: FeatureId,
    pub plane: Option<PlaneReference>,
}

pub fn source(model: &Model, selection: &Selection) -> Result<SplitSource, &'static str> {
    let body = move_tools::chosen_body(model, selection, NO_BODY)?;
    let end = model.document().bar_index();
    let plane = selection.iter().find_map(|pickable| match pickable {
        Pickable::Plane(_) | Pickable::Datum(_) => {
            datum_tools::plane_reference(model, pickable, end)
        }
        _ => None,
    });
    Ok(SplitSource { body, plane })
}

pub fn create(document: &Document, source: &SplitSource) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, "Split");
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Split(Split {
            body: source.body,
            plane: source.plane.clone().unwrap_or(DEFAULT_PLANE),
            flipped: false,
        }),
    );
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: &SplitSource) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source);
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, split: Split) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Split(split),
        },
    ))
}

pub fn change(model: &Model, feature: FeatureId, split: Split) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, split).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn plane_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    split: &Split,
) -> Result<Transaction, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| GONE.to_owned())?;
    let plane = selection
        .iter()
        .find_map(|pickable| datum_tools::plane_reference(model, pickable, index))
        .ok_or_else(|| NO_PLANE.to_owned())?;
    if plane == split.plane {
        return Err(ALREADY.to_owned());
    }
    change(
        model,
        feature,
        Split {
            plane,
            ..split.clone()
        },
    )
}
