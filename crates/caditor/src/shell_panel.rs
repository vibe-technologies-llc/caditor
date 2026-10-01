use caditor_document::{Document, Feature, FeatureId, Resolution, Shell, SolidResult, Transaction};
use caditor_expression::Dimension;
use caditor_kernel::FaceId;
use egui::{Id, Ui};

use crate::{
    bodies,
    editing::EditingCommand,
    feature_fields::{self, Quantity, Rule},
    feature_tree::count,
    field,
    model::{Action, Model},
    reference_rows::{ReferenceRows, RowCache},
    shell_tools, widgets,
};

pub const DESCRIPTION: &str = "Hollows the body, opening the chosen faces";

fn change(model: &Model, feature: FeatureId, shell: Shell) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = shell_tools::edit(document, feature, shell)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn thickness_row(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    shell: &Shell,
    actions: &mut Vec<Action>,
) {
    let quantity = Quantity {
        id: Id::new(("shell-thickness", feature)),
        expression: &shell.thickness,
        dimension: Dimension::LENGTH,
        rule: Rule::AboveZero,
    };
    let committed = feature_fields::expression_row(ui, model, "Thickness", quantity, |thickness| {
        change(
            model,
            feature,
            Shell {
                thickness,
                ..shell.clone()
            },
        )
    });
    actions.extend(committed.map(Action::Apply));
}

const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";
const GONE: &str = "A face that is no longer there";

fn face_row(document: &Document, input: &SolidResult, resolution: &Resolution<FaceId>) -> String {
    match resolution {
        Resolution::One(face) => bodies::describe_face_id(document, input, *face),
        Resolution::Pieces(pieces) => match pieces.first().and_then(|face| input.solid.face(*face))
        {
            Some(first) => format!(
                "{}, split into {} pieces",
                bodies::describe_origin(document, first.origin()),
                pieces.len()
            ),
            None => GONE.to_owned(),
        },
        Resolution::Tied(candidates) => format!(
            "A face that now matches {} separate faces; close it and choose it again",
            candidates.len()
        ),
        Resolution::Missing => GONE.to_owned(),
    }
}

fn face_rows(document: &Document, input: Option<&SolidResult>, shell: &Shell) -> ReferenceRows {
    let summary = count(shell.open.len(), "face", "faces");
    let rows = match input {
        Some(input) => shell
            .resolutions(&input.solid)
            .iter()
            .map(|resolution| face_row(document, input, resolution))
            .collect(),
        None => vec![NO_SHAPE_YET.to_owned(); shell.open.len()],
    };
    ReferenceRows { summary, rows }
}

struct FacesRow<'a> {
    model: &'a Model,
    feature: &'a Feature,
    shell: &'a Shell,
    opened: bool,
}

fn faces_row(ui: &mut Ui, row: &FacesRow<'_>, cache: &mut RowCache, actions: &mut Vec<Action>) {
    let FacesRow {
        model,
        feature,
        shell,
        opened,
    } = *row;
    let id = feature.id();
    widgets::caption(ui, "Open faces");
    let evaluation = model.evaluation();
    let listed = cache.rows(id, evaluation.body_before(id), model.revision(), || {
        face_rows(model.document(), bodies::input(evaluation, id), shell)
    });
    ui.vertical(|ui| {
        if shell.open.is_empty() {
            ui.label(widgets::muted("None: the body is hollow and closed", ui));
        } else {
            ui.label(&listed.summary);
        }
        for (index, text) in listed.rows.iter().enumerate() {
            let text = widgets::muted(text, ui);
            if widgets::removable_row(ui, text, "Close this face") {
                let mut changed = shell.clone();
                changed.open.remove(index);
                actions.push(feature_fields::applied(
                    &feature.name,
                    change(model, id, changed),
                ));
            }
        }
        if widgets::choose_in_view(
            ui,
            feature_fields::choosing_list(ui, opened),
            "Click flat faces in the view to open them or close them again.",
            "Show the body as it was before this feature so you can click faces",
        ) {
            actions.push(Action::Editing(EditingCommand::OpenSolid(id)));
        }
    });
    ui.end_row();
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    cache: &mut RowCache,
    actions: &mut Vec<Action>,
    feature: &Feature,
    shell: &Shell,
    opened: bool,
) {
    let id = feature.id();
    let row = FacesRow {
        model,
        feature,
        shell,
        opened,
    };
    widgets::properties(ui, ("shell-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        faces_row(ui, &row, cache, actions);
        thickness_row(ui, model, id, shell, actions);
        feature_fields::feature_row(ui, model.document(), "Body", shell.body);
    });
}
