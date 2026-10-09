use caditor_document::{Feature, FeatureId};
use egui::{Id, Label, RichText, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance::{self, SPACE_S},
    body_appearance, body_selection,
    commands::{Command, CommandFrame},
    feature_tree::CommandContext,
    icons,
    model::{Action, Model, Notice},
    move_tools,
    panels::{Painting, PanelState},
    removal,
    selection::Selection,
    split_tools,
    tree_row::{self, CHILD_INDENT, Look},
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Bodies";
const NO_BODY_CHOSEN: &str = "Select a face, edge or vertex of a body, or a body in the tree";
const SEVERAL_BODIES: &str =
    "Select faces, edges or vertices of one body only, or one body in the tree";

fn group_title(count: usize) -> String {
    format!("{GROUP_TITLE} ({count})")
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
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
            item(ui, model, selection, state, actions, body);
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
    body: FeatureId,
) {
    let Some(feature) = model.document().feature(body) else {
        return;
    };
    let name = model.document().body_name(body).unwrap_or(&feature.name);
    let painting = state.painting.filter(|painting| painting.body == body);
    let shown = !feature.hidden;
    let muted = appearance::tokens(ui).text_muted;
    let toggle = visibility::toggle(feature);
    let look = Look {
        selected: state.reads_selected(body),
        open: false,
    };
    let row = tree_row::show(
        ui,
        look,
        |ui| {
            ui.add_space(CHILD_INDENT);
            widgets::icon_label(ui, icons::feature(&feature.kind), muted);
            ui.add(
                Label::new(label(ui, name, shown))
                    .selectable(false)
                    .truncate(),
            );
            tree_row::sense(
                ui,
                Id::new(("body-row", body)),
                ui.min_rect().left(),
                name,
                look.selected,
            )
        },
        |ui| {
            let paint = tree_row::slot(ui, |ui| paint_button(ui, name, painting.is_some()));
            let eye = tree_row::slot(ui, |ui| match &toggle {
                Ok(transaction) => eye(ui, shown, transaction.label()),
                Err(_) => false,
            });
            (paint, eye)
        },
    );
    let (paint, eye) = row.trailing;
    let row_name = row.leading;
    if row_name.has_focus() {
        tree_row::focus_outline(ui, row.rect);
    }
    if row_name.clicked() || row_name.gained_focus() {
        state.choose_only(body);
    }
    row_name.context_menu(|ui| {
        widgets::fitted_menu(ui, |ui| {
            row_menu(ui, model, selection, state, actions, body)
        });
    });
    if eye && let Ok(transaction) = toggle {
        actions.push(Action::Apply(transaction));
    }
    if paint {
        state.painting = match painting {
            Some(_) => None,
            None => Some(Painting {
                body,
                focus_pending: false,
                naming: false,
            }),
        };
    }
    let Some(painting) = state.painting.filter(|painting| painting.body == body) else {
        return;
    };
    let focused = tree_row::indented(ui, |ui| {
        ui.add_space(SPACE_S);
        widgets::card(ui, |ui| {
            body_appearance::show(
                ui,
                model,
                selection,
                actions,
                feature,
                (painting.focus_pending, painting.naming),
            )
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

fn row_menu(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    body: FeatureId,
) {
    if widgets::menu_item(
        ui,
        icons::command(Command::RenameBody),
        "Rename body…",
        None,
    )
    .clicked()
    {
        state.painting = Some(Painting {
            body,
            focus_pending: true,
            naming: true,
        });
        ui.close();
    }
    if widgets::menu_item(
        ui,
        icons::command(Command::SelectBody),
        "Select the whole body",
        None,
    )
    .clicked()
    {
        state.selected_in_tree = Some(body_selection::whole_bodies(
            model,
            &[body],
            body_selection::Kind::Faces,
        ));
        ui.close();
    }
    if widgets::menu_item(ui, icons::command(Command::CopyBody), "Copy body", None).clicked() {
        actions.extend(move_tools::create_actions(model, body, true));
        ui.close();
    }
    if widgets::menu_item(ui, icons::command(Command::Split), "Split body", None).clicked() {
        match split_tools::source_for(model, selection, body) {
            Ok(source) => actions.extend(split_tools::create_actions(model, &source)),
            Err(reason) => actions.push(Action::Inform(Notice::warning(format!(
                "{}: {reason}.",
                split_tools::TITLE
            )))),
        }
        ui.close();
    }
    ui.separator();
    if widgets::menu_item(ui, icons::command(Command::RemoveBody), "Remove body", None).clicked() {
        actions.extend(removal::create_actions(model.document(), body));
        ui.close();
    }
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
    let tree = body_selection::tree_bodies(document, &state.chosen());
    let chosen = if tree.is_empty() {
        body_selection::bodies_in(context.selection)
    } else {
        tree
    };
    match chosen.as_slice() {
        [body] => return Ok(*body),
        [_, _, ..] => return Err(SEVERAL_BODIES),
        [] => {}
    }
    context
        .editing
        .feature()
        .or(context.editing.solid())
        .and_then(|id| document.feature(id))
        .and_then(Feature::body)
        .filter(|body| document.feature(*body).is_some_and(Feature::makes_body))
        .ok_or(NO_BODY_CHOSEN)
}

pub fn commands(
    context: &CommandContext<'_>,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let body = chosen_body(context, state);
    let detail = body
        .ok()
        .and_then(|body| context.model.document().body_name(body))
        .map(str::to_owned);
    if commands.invoke_detailed(Command::BodyAppearance, detail.clone(), &body)
        && let Ok(body) = body
    {
        state.painting = Some(Painting {
            body,
            focus_pending: true,
            naming: false,
        });
    }
    if commands.invoke_detailed(Command::RenameBody, detail.clone(), &body)
        && let Ok(body) = body
    {
        state.painting = Some(Painting {
            body,
            focus_pending: true,
            naming: true,
        });
    }
    if commands.invoke_detailed(Command::RemoveBody, detail, &body)
        && let Ok(body) = body
    {
        actions.extend(removal::create_actions(context.model.document(), body));
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
