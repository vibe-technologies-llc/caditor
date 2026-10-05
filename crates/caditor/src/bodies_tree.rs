use caditor_document::{Document, FeatureId};
use egui::{Id, Label, RichText, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance, icons,
    model::{Action, Model},
    panels::PanelState,
    tree_row::{self, CHILD_INDENT, Look},
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Bodies";

fn group_title(count: usize) -> String {
    format!("{GROUP_TITLE} ({count})")
}

pub fn show(ui: &mut Ui, model: &Model, state: &mut PanelState, actions: &mut Vec<Action>) {
    let document = model.document();
    let bodies: Vec<FeatureId> = model
        .evaluation()
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| document.feature(*body).is_some())
        .collect();
    if bodies.is_empty() {
        return;
    }
    let title = group_title(bodies.len());
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id("bodies-group"),
        false,
    );
    let muted = appearance::tokens(ui).text_muted;
    let open = collapsing.is_open();
    let row = tree_row::show(
        ui,
        Look::default(),
        |ui| {
            let toggle = tree_row::chevron(ui, open, &title);
            widgets::icon_label(ui, icons::BODIES, muted);
            ui.add(
                Label::new(RichText::new(&title))
                    .selectable(false)
                    .truncate(),
            );
            let name = tree_row::sense(
                ui,
                Id::new("bodies-group-row"),
                toggle.rect.right(),
                &title,
                false,
            );
            (toggle.clicked(), name)
        },
        |ui| {
            tree_row::empty_slot(ui);
            tree_row::empty_slot(ui);
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
        for body in bodies {
            item(ui, document, state, actions, body);
        }
    });
    collapsing.store(ui.ctx());
}

fn item(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    body: FeatureId,
) {
    let Some(feature) = document.feature(body) else {
        return;
    };
    let shown = !feature.hidden;
    let muted = appearance::tokens(ui).text_muted;
    let toggle = visibility::toggle(feature);
    let look = Look {
        selected: state.chosen().contains(&body),
        open: false,
    };
    let row = tree_row::show(
        ui,
        look,
        |ui| {
            ui.add_space(CHILD_INDENT);
            widgets::icon_label(ui, icons::feature(&feature.kind), muted);
            ui.add(
                Label::new(label(ui, &feature.name, shown))
                    .selectable(false)
                    .truncate(),
            );
            tree_row::sense(
                ui,
                Id::new(("body-row", body)),
                ui.min_rect().left(),
                &feature.name,
                look.selected,
            )
        },
        |ui| {
            tree_row::empty_slot(ui);
            tree_row::slot(ui, |ui| match &toggle {
                Ok(transaction) => eye(ui, shown, transaction.label()),
                Err(_) => false,
            })
        },
    );
    let name = row.leading;
    if name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    if name.clicked() || name.gained_focus() {
        state.choose_only(body);
    }
    if row.trailing
        && let Ok(transaction) = toggle
    {
        actions.push(Action::Apply(transaction));
    }
}

fn label(ui: &Ui, text: &str, shown: bool) -> RichText {
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
