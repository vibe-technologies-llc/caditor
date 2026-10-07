use caditor_document::{Document, Edit, ModelProperties, ModelProperty, Transaction};
use egui::{Id, ScrollArea, TextEdit, Ui};

use crate::{
    appearance::SPACE_S,
    dialog_parts::BodyRoom,
    widgets::{self, DialogWidth},
};

pub const TITLE: &str = "Model properties";
pub const SAVE_LABEL: &str = "Save properties";
pub const CANCEL_LABEL: &str = "Cancel";
pub const CHANGE_LABEL: &str = "Change the model properties";
const EXPLANATION: &str = "Describe the model for the people and programs it goes to. Exports \
                           carry each field their format has room for; the notes stay in the \
                           model. Every field may stay empty.";
const DIALOG_ID: &str = "model-properties";
const NOTES_ROWS: usize = 5;
const HEIGHT_SHARE: f32 = 0.8;

const SINGLE_LINE: [ModelProperty; 6] = [
    ModelProperty::Title,
    ModelProperty::PartNumber,
    ModelProperty::Revision,
    ModelProperty::Author,
    ModelProperty::Organisation,
    ModelProperty::Description,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertiesDraft {
    properties: ModelProperties,
    focus_pending: bool,
}

impl PropertiesDraft {
    pub fn of(document: &Document) -> Self {
        Self {
            properties: document.properties().clone(),
            focus_pending: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Save(Transaction),
}

pub fn field_id(property: ModelProperty) -> Id {
    Id::new((DIALOG_ID, property))
}

pub fn transaction(document: &Document, properties: ModelProperties) -> Option<Transaction> {
    let properties = properties.normalized();
    (&properties != document.properties()).then(|| {
        Transaction::single(
            CHANGE_LABEL,
            Edit::SetModelProperties {
                properties: Box::new(properties),
            },
        )
    })
}

pub fn dialog(
    ctx: &egui::Context,
    document: &Document,
    draft: &mut PropertiesDraft,
) -> Option<Outcome> {
    let response = widgets::dialog(ctx, DIALOG_ID, TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(EXPLANATION, ui));
        ui.add_space(SPACE_S);
        let mut room = BodyRoom::measure(ui, DIALOG_ID, HEIGHT_SHARE);
        ScrollArea::vertical()
            .max_height(room.height)
            .show(ui, |ui| fields(ui, draft));
        room.body_ended(ui);
        let save = widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, SAVE_LABEL)).clicked() {
                return Some(true);
            }
            ui.add(widgets::button(CANCEL_LABEL))
                .clicked()
                .then_some(false)
        });
        room.dialog_ended(ui);
        save
    });
    let closed = response.should_close().then_some(false);
    match response.inner.or(closed)? {
        true => Some(
            transaction(document, draft.properties.clone()).map_or(Outcome::Close, Outcome::Save),
        ),
        false => Some(Outcome::Close),
    }
}

fn fields(ui: &mut Ui, draft: &mut PropertiesDraft) {
    let focus = std::mem::take(&mut draft.focus_pending);
    widgets::properties(ui, DIALOG_ID, |ui| {
        for property in SINGLE_LINE {
            widgets::property(ui, property.label(), |ui| {
                let field = widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(draft.properties.get_mut(property))
                            .id(field_id(property))
                            .char_limit(property.max_chars())
                            .desired_width(f32::INFINITY),
                    )
                });
                widgets::tie_to_caption(ui, &field);
                if focus && property == ModelProperty::Title {
                    field.request_focus();
                }
            });
        }
    });
    ui.add_space(SPACE_S);
    let notes = ModelProperty::Notes;
    widgets::caption(ui, notes.label());
    let field = widgets::text_field(ui, |ui| {
        ui.add(
            TextEdit::multiline(draft.properties.get_mut(notes))
                .id(field_id(notes))
                .char_limit(notes.max_chars())
                .desired_rows(NOTES_ROWS)
                .desired_width(f32::INFINITY),
        )
    });
    widgets::tie_to_caption(ui, &field);
}
