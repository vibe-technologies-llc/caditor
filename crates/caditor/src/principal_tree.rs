use caditor_document::{Document, PrincipalGeometry};
use egui::{Id, Label, RichText, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance, icons,
    model::{Action, Model},
    panels::PanelState,
    selection::Selection,
    tree_row::{self, CHILD_INDENT, Look},
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Principal planes, axes and origin";

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id("principal-geometry"),
        false,
    );
    let shown = visibility::any_principal_shown(document);
    let muted = appearance::tokens(ui).text_muted;
    let open = collapsing.is_open();
    let row = tree_row::show(
        ui,
        Look::default(),
        |ui| {
            let toggle = tree_row::chevron(ui, open, GROUP_TITLE);
            widgets::icon_label(ui, icons::PRINCIPAL_GROUP, muted);
            ui.add(
                Label::new(title(ui, GROUP_TITLE, shown))
                    .selectable(false)
                    .truncate(),
            );
            let name = tree_row::sense(
                ui,
                Id::new("principal-group-row"),
                toggle.rect.right(),
                GROUP_TITLE,
                false,
            );
            (toggle.clicked(), name)
        },
        |ui| {
            tree_row::empty_slot(ui);
            tree_row::slot(ui, |ui| {
                let transaction = visibility::toggle_principal_group(document);
                if eye(ui, shown, transaction.label()) {
                    actions.push(Action::Apply(transaction));
                }
            });
        },
    );
    let (toggled, name) = row.leading;
    if name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    if toggled || name.clicked() {
        collapsing.toggle(ui);
    }
    collapsing.show_body_unindented(ui, |ui| {
        for geometry in PrincipalGeometry::ALL {
            item(ui, document, selection, state, actions, geometry);
        }
    });
    collapsing.store(ui.ctx());
}

fn item(
    ui: &mut Ui,
    document: &Document,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    geometry: PrincipalGeometry,
) {
    let shown = visibility::is_principal_shown(document, geometry);
    let muted = appearance::tokens(ui).text_muted;
    let pickable = visibility::pickable(geometry);
    let transaction = visibility::toggle_principal(document, geometry);
    let look = Look {
        selected: selection.contains(pickable),
        open: false,
    };
    let row = tree_row::show(
        ui,
        look,
        |ui| {
            ui.add_space(CHILD_INDENT);
            widgets::icon_label(ui, icons::principal(geometry), muted);
            ui.add(
                Label::new(title(ui, geometry.name(), shown))
                    .selectable(false)
                    .truncate(),
            );
            tree_row::sense(
                ui,
                Id::new(("principal-row", geometry)),
                ui.min_rect().left(),
                geometry.name(),
                look.selected,
            )
        },
        |ui| {
            tree_row::empty_slot(ui);
            tree_row::slot(ui, |ui| eye(ui, shown, transaction.label()))
        },
    );
    let name = row.leading;
    if name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    if ui.rect_contains_pointer(row.rect) && shown {
        state.hovered_in_tree = Some(pickable);
    }
    if (name.clicked() || name.gained_focus()) && shown {
        state.selected = None;
        state.chosen_in_tree = Some(pickable);
    }
    if row.trailing {
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
    let (glyph, hover) = if shown {
        (icons::SHOW, "Hide")
    } else {
        (icons::HIDE, "Show")
    };
    widgets::named(widgets::icon_button(ui, glyph, hover), name).clicked()
}
