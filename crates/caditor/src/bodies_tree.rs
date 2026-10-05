use caditor_document::{Feature, FeatureId};
use egui::{Id, Label, RichText, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance::{self, SPACE_S},
    body_appearance,
    commands::{Command, CommandFrame},
    feature_tree::{self, CommandContext},
    icons,
    model::{Action, Model},
    panels::{Painting, PanelState},
    selection::Pickable,
    tree_row::{self, CHILD_INDENT, Look},
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Bodies";
const NO_BODY_CHOSEN: &str = "Select a face, edge or vertex of a body, or a body in the tree";
const SEVERAL_BODIES: &str = "Select faces, edges or vertices of one body only";

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
    if state
        .painting
        .is_some_and(|painting| !bodies.contains(&painting.body))
    {
        state.painting = None;
    }
    if bodies.is_empty() {
        return;
    }
    let title = group_title(bodies.len());
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id("bodies-group"),
        false,
    );
    if state
        .painting
        .is_some_and(|painting| painting.focus_pending && bodies.contains(&painting.body))
    {
        collapsing.set_open(true);
    }
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
            item(ui, model, state, actions, body);
        }
    });
    collapsing.store(ui.ctx());
}

fn item(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    body: FeatureId,
) {
    let Some(feature) = model.document().feature(body) else {
        return;
    };
    let painting = state.painting.filter(|painting| painting.body == body);
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
            let paint =
                tree_row::slot(ui, |ui| paint_button(ui, &feature.name, painting.is_some()));
            let eye = tree_row::slot(ui, |ui| match &toggle {
                Ok(transaction) => eye(ui, shown, transaction.label()),
                Err(_) => false,
            });
            (paint, eye)
        },
    );
    let (paint, eye) = row.trailing;
    let name = row.leading;
    if name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    if name.clicked() || name.gained_focus() {
        state.choose_only(body);
    }
    if eye && let Ok(transaction) = toggle {
        actions.push(Action::Apply(transaction));
    }
    if paint {
        state.painting = match painting {
            Some(_) => None,
            None => Some(Painting {
                body,
                focus_pending: false,
            }),
        };
    }
    let Some(painting) = state.painting.filter(|painting| painting.body == body) else {
        return;
    };
    let focused = tree_row::indented(ui, |ui| {
        ui.add_space(SPACE_S);
        widgets::card(ui, |ui| {
            body_appearance::show(ui, model, actions, feature, painting.focus_pending)
        })
    });
    if painting.focus_pending && focused {
        state.painting = Some(Painting {
            focus_pending: false,
            ..painting
        });
    }
    ui.add_space(SPACE_S);
}

fn paint_button(ui: &mut Ui, name: &str, open: bool) -> bool {
    let hover = if open {
        format!("Close the colour and material of {name}")
    } else {
        format!("Colour and material of {name}")
    };
    widgets::icon_button(ui, icons::BODY_APPEARANCE, &hover).clicked()
}

fn chosen_body(
    context: &CommandContext<'_>,
    state: &PanelState,
) -> Result<FeatureId, &'static str> {
    let document = context.model.document();
    let mut selected: Vec<FeatureId> = context
        .selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, .. }
            | Pickable::Edge { body, .. }
            | Pickable::Vertex { body, .. } => Some(body),
            _ => None,
        })
        .collect();
    selected.dedup();
    match selected.as_slice() {
        [body] => return Ok(*body),
        [_, _, ..] => return Err(SEVERAL_BODIES),
        [] => {}
    }
    feature_tree::current_feature(document, context.editing, state)
        .and_then(|feature| feature.body())
        .filter(|body| document.feature(*body).is_some_and(Feature::makes_body))
        .ok_or(NO_BODY_CHOSEN)
}

pub fn commands(
    context: &CommandContext<'_>,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
) {
    let body = chosen_body(context, state);
    let detail = body
        .ok()
        .and_then(|body| context.model.document().feature(body))
        .map(|feature| feature.name.clone());
    if commands.invoke_detailed(Command::BodyAppearance, detail, &body)
        && let Ok(body) = body
    {
        state.painting = Some(Painting {
            body,
            focus_pending: true,
        });
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
