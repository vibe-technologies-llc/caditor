use caditor_document::{Document, PrincipalGeometry};
use egui::{
    Button, Color32, CornerRadius, Frame, Label, Margin, RichText, Sense, Sides, Ui,
    collapsing_header::CollapsingState,
};

use crate::{
    appearance::{self, WIDGET_RADIUS},
    icons,
    model::{Action, Model},
    panels::PanelState,
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Principal planes and axes";
const ROW_MARGIN: Margin = Margin::symmetric(4, 2);
const BODY_INDENT: f32 = 8.0;

pub fn show(ui: &mut Ui, model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    let document = model.document();
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id("principal-geometry"),
        false,
    );
    let shown = visibility::any_principal_shown(document);
    let tokens = appearance::tokens(ui);
    let mut prepared = Frame::new()
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(ROW_MARGIN)
        .begin(ui);
    let (toggled, name) = {
        let ui = &mut prepared.content_ui;
        let open = collapsing.is_open();
        Sides::new()
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    let chevron = if open {
                        icons::EXPANDED
                    } else {
                        icons::COLLAPSED
                    };
                    let hint = if open { "Hide details" } else { "Show details" };
                    let toggle = ui
                        .add(widgets::Named::new(
                            Button::new(widgets::icon(chevron).color(tokens.text_muted))
                                .frame(false),
                            format!("{hint} of {GROUP_TITLE}"),
                        ))
                        .on_hover_text(hint);
                    widgets::icon_label(ui, icons::PRINCIPAL_GROUP, tokens.text_muted);
                    let name = ui.add(
                        Label::new(title(ui, GROUP_TITLE, shown))
                            .selectable(false)
                            .sense(Sense::click())
                            .truncate(),
                    );
                    (toggle.clicked(), name)
                },
                |ui| {
                    let transaction = visibility::toggle_principal_group(document);
                    if eye(ui, shown, transaction.label()) {
                        actions.push(Action::Apply(transaction));
                    }
                },
            )
            .0
    };
    let rect = prepared.content_ui.min_rect() + ROW_MARGIN;
    prepared.frame.fill = if ui.rect_contains_pointer(rect) {
        tokens.stripe
    } else {
        Color32::TRANSPARENT
    };
    prepared.end(ui);
    if toggled || name.clicked() {
        collapsing.toggle(ui);
    }
    collapsing.show_body_unindented(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(BODY_INDENT);
            ui.vertical(|ui| {
                widgets::card(ui, |ui| {
                    for geometry in PrincipalGeometry::ALL {
                        item(ui, document, state, actions, geometry);
                    }
                });
            });
        });
    });
    collapsing.store(ui.ctx());
}

fn item(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    geometry: PrincipalGeometry,
) {
    let shown = visibility::is_principal_shown(document, geometry);
    let muted = appearance::tokens(ui).text_muted;
    let transaction = visibility::toggle_principal(document, geometry);
    let (row, clicked) = Sides::new().shrink_left().truncate().show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                widgets::icon_label(ui, icons::principal(geometry), muted);
                ui.add(
                    Label::new(title(ui, geometry.name(), shown))
                        .selectable(false)
                        .sense(Sense::hover())
                        .truncate(),
                )
            })
            .inner
        },
        |ui| eye(ui, shown, transaction.label()),
    );
    if row.hovered() && shown {
        state.hovered_in_tree = Some(visibility::pickable(geometry));
    }
    if clicked {
        actions.push(Action::Apply(transaction));
    }
}

fn title(ui: &Ui, text: &str, shown: bool) -> RichText {
    let text = RichText::new(text);
    if shown {
        text
    } else {
        text.color(appearance::tokens(ui).text_muted).italics()
    }
}

fn eye(ui: &mut Ui, shown: bool, name: &str) -> bool {
    let muted = appearance::tokens(ui).text_muted;
    let (glyph, hover) = if shown {
        (icons::SHOW, "Hide")
    } else {
        (icons::HIDE, "Show")
    };
    ui.add(widgets::Named::new(
        Button::new(widgets::icon(glyph).color(muted)).frame_when_inactive(false),
        name,
    ))
    .on_hover_text(hover)
    .clicked()
}
