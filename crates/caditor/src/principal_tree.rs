use caditor_document::{PrincipalGeometry, PrincipalShowing};
use egui::{Id, Label, Response, RichText, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance, feature_tree, icons,
    model::{Action, Model},
    panels::PanelState,
    selection::Selection,
    tree_row::{self, CHILD_INDENT, Look},
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Principal planes, axes and origin";
pub const ONLY_WHEN_NEEDED: &str = "Show only when needed";
const HIDE: &str = "Hide";
const SHOW: &str = "Show";
const KEEP_SHOWN: &str = "Show, and keep shown";

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let view = model.principal_view();
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id("principal-geometry"),
        false,
    );
    let shown = visibility::any_principal_shown(document, view);
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
                Id::unique("principal-group-row"),
                toggle.rect.right(),
                GROUP_TITLE,
                false,
            );
            (toggle.clicked(), name)
        },
        |ui| {
            tree_row::empty_slot(ui);
            tree_row::slot(ui, |ui| {
                let toggle = visibility::toggle_principal_group(document, view);
                let hover = if shown { HIDE } else { SHOW };
                if eye(ui, shown, hover, toggle.label()) {
                    actions.push(Action::Apply(toggle));
                }
            });
        },
    );
    let (toggled, name) = row.leading;
    if name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    row_menu(&name, model, actions);
    if toggled || name.clicked() {
        collapsing.toggle(ui);
    }
    collapsing.show_body_unindented(ui, |ui| {
        for geometry in PrincipalGeometry::ALL {
            item(ui, model, selection, state, actions, geometry);
        }
    });
    collapsing.store(ui.ctx());
}

fn item(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    geometry: PrincipalGeometry,
) {
    let document = model.document();
    let view = model.principal_view();
    let shown = view.reads_shown(document, geometry);
    let choosable = !document.is_principal_hidden(geometry);
    let muted = appearance::tokens(ui).text_muted;
    let pickable = visibility::pickable(geometry);
    let toggle = visibility::toggle_principal(document, view, geometry);
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
                Id::unique(("principal-row", geometry)),
                ui.min_rect().left(),
                geometry.name(),
                look.selected,
            )
        },
        |ui| {
            tree_row::empty_slot(ui);
            tree_row::slot(ui, |ui| {
                eye(ui, shown, eye_hover(model, geometry), toggle.label())
            })
        },
    );
    let name = row.leading;
    if name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    row_menu(&name, model, actions);
    if ui.rect_contains_pointer(row.rect) && choosable {
        state.hovered_in_tree = Some(pickable);
    }
    if (name.clicked() || name.gained_focus()) && choosable {
        state.selected = None;
        state.chosen_in_tree = Some(pickable);
    }
    if row.trailing {
        actions.push(Action::Apply(toggle));
    }
}

fn eye_hover(model: &Model, geometry: PrincipalGeometry) -> &'static str {
    let document = model.document();
    let view = model.principal_view();
    if view.reads_shown(document, geometry) {
        HIDE
    } else if document.principal_showing(geometry) == PrincipalShowing::WhenNeeded {
        KEEP_SHOWN
    } else {
        SHOW
    }
}

fn row_menu(name: &Response, model: &Model, actions: &mut Vec<Action>) {
    name.context_menu(|ui| {
        widgets::fitted_menu(ui, |ui| {
            let restore = visibility::principal_only_when_needed(model.document());
            if feature_tree::menu_entry(ui, icons::PRINCIPAL_GROUP, ONLY_WHEN_NEEDED, &restore)
                && let Ok(transaction) = restore
            {
                actions.push(Action::Apply(transaction));
            }
        });
    });
}

fn title(ui: &Ui, text: &str, shown: bool) -> RichText {
    let text = RichText::new(text);
    if shown {
        text
    } else {
        text.color(appearance::tokens(ui).text_muted).italics()
    }
}

fn eye(ui: &mut Ui, shown: bool, hover: &str, name: &str) -> bool {
    let glyph = if shown { icons::SHOW } else { icons::HIDE };
    widgets::named(widgets::icon_button(ui, glyph, hover), name).clicked()
}
