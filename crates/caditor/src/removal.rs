use caditor_document::{Document, Edit, Feature, FeatureId, FeatureKind, Remove, Transaction};
use egui::{Id, Ui};

use crate::{
    editing,
    feature_fields::{self, Choice},
    field,
    model::{Action, Model, Notice},
    widgets,
};

pub const TITLE: &str = "Remove";
pub const DESCRIPTION: &str =
    "Takes a body out of the model from here on; suppress or delete this to bring it back";
const NOT_A_BODY: &str = "A body that is no longer there";

pub fn create(document: &Document, body: FeatureId) -> Option<(Transaction, String)> {
    let removed = document.body_name(body)?.to_owned();
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Remove {removed}"));
    transaction.add_feature(name.clone(), FeatureKind::Remove(Remove { body }));
    Some((transaction.finish(), name))
}

pub fn create_actions(document: &Document, body: FeatureId) -> Vec<Action> {
    let Some((transaction, name)) = create(document, body) else {
        return vec![Action::Inform(Notice::info("That body no longer exists."))];
    };
    if let Err(error) = document.check(&transaction) {
        return vec![Action::Inform(Notice::info(format!(
            "The body could not be removed: {error}."
        )))];
    }
    let removed = document.body_name(body).unwrap_or_default();
    vec![
        Action::Apply(transaction),
        Action::Inform(Notice::info(format!(
            "Removed {removed} with {name}. Undo, or suppress or delete {name}, to bring it back."
        ))),
    ]
}

fn change(model: &Model, feature: FeatureId, remove: Remove) -> Result<Transaction, String> {
    let document = model.document();
    let name = &document
        .feature(feature)
        .ok_or_else(|| "The feature no longer exists".to_owned())?
        .name;
    field::checked(
        document,
        Transaction::single(
            format!("Edit {name}"),
            Edit::SetFeatureKind {
                id: feature,
                kind: FeatureKind::Remove(remove),
            },
        ),
    )
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    remove: &Remove,
) {
    let document = model.document();
    let id = feature.id();
    widgets::properties(ui, ("remove-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        widgets::caption(ui, "Body");
        let selected = feature_fields::combo_text(
            ui,
            feature_fields::feature_name(document, remove.body),
            NOT_A_BODY,
        );
        let chosen = feature_fields::combo(ui, Id::new(("remove-body", id)), selected, || {
            document
                .bodies_before(id)
                .into_iter()
                .filter_map(|body| {
                    Some(Choice {
                        label: feature_fields::feature_name(document, body)?.to_owned(),
                        selected: body == remove.body,
                        change: change(model, id, Remove { body }).map(|transaction| {
                            feature_fields::applied(&feature.name, Ok(transaction))
                        }),
                    })
                })
                .collect()
        });
        actions.extend(chosen);
        ui.end_row();
    });
}
