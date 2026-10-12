use caditor_document::{Document, PlaneReference, Transaction, capitalized, describe_plane};
use egui::{Id, Ui, WidgetText};

use crate::{
    editing::EditingCommand,
    feature_fields::{self, Choice, Picker},
    icons,
    model::{Action, Model},
    reference_picking, widgets,
};

pub const UNCHOSEN: &str = "Choose a plane";

pub fn label(document: &Document, plane: &PlaneReference) -> String {
    capitalized(&describe_plane(document, plane))
}

pub fn combo<T>(
    ui: &mut Ui,
    id: Id,
    document: &Document,
    planes: Vec<PlaneReference>,
    current: Option<&PlaneReference>,
    change: impl Fn(&PlaneReference) -> Result<T, String>,
) -> Option<T> {
    let shown: WidgetText = match current {
        Some(current) => label(document, current).into(),
        None => UNCHOSEN.into(),
    };
    combo_showing(
        ui,
        id,
        document,
        (shown, |plane| current == Some(plane)),
        planes,
        change,
    )
}

pub fn combo_showing<T>(
    ui: &mut Ui,
    id: Id,
    document: &Document,
    (shown, is_current): (WidgetText, impl Fn(&PlaneReference) -> bool),
    planes: Vec<PlaneReference>,
    change: impl Fn(&PlaneReference) -> Result<T, String>,
) -> Option<T> {
    feature_fields::combo(ui, id, shown, || {
        planes
            .into_iter()
            .map(|plane| Choice {
                label: label(document, &plane),
                selected: is_current(&plane),
                change: change(&plane),
            })
            .collect()
    })
}

pub struct Row<'a> {
    pub caption: &'a str,
    pub id: Id,
    pub planes: Vec<PlaneReference>,
    pub current: Option<&'a PlaneReference>,
    pub picker: Picker<'a>,
    pub remove: Option<&'a str>,
}

pub fn row(
    ui: &mut Ui,
    model: &Model,
    row: Row<'_>,
    change: impl Fn(&PlaneReference) -> Result<Transaction, String>,
    actions: &mut Vec<Action>,
) -> bool {
    widgets::caption(ui, row.caption);
    let document = model.document();
    let (feature, slot) = (row.picker.feature, row.picker.slot);
    let removed = ui
        .vertical(|ui| {
            let mut removed = false;
            ui.horizontal_wrapped(|ui| {
                let chosen = combo(ui, row.id, document, row.planes, row.current, |plane| {
                    change(plane).map(Action::Apply)
                });
                if let Some(action) = chosen {
                    actions.push(action);
                    let choosing = reference_picking::current(ui.ctx())
                        .is_some_and(|picking| picking.is_for(feature, slot));
                    if choosing {
                        actions.push(Action::Editing(EditingCommand::StopPicking));
                    }
                }
                if let Some(hover) = row.remove {
                    removed = widgets::icon_button(ui, icons::REMOVE, hover).clicked();
                }
            });
            feature_fields::reference_picker(ui, model, row.picker, actions);
            removed
        })
        .inner;
    ui.end_row();
    removed
}
