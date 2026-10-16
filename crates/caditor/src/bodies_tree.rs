use std::borrow::Cow;

use caditor_document::{Feature, FeatureId};
use egui::{Id, Label, RichText, Ui, collapsing_header::CollapsingState, vec2};

use crate::{
    appearance::{self, SPACE_S},
    body_appearance,
    body_groups::{self, Entry, Listing},
    body_selection,
    commands::{Command, CommandFrame, Reason},
    feature_tree::{self, CommandContext},
    icons,
    model::{Action, Model, Notice},
    move_tools,
    panels::{Painting, PanelState},
    removal,
    selection::Selection,
    split_tools,
    tree_row::{self, CHILD_INDENT, GROUP_INDENT, Look},
    visibility, widgets,
};

pub const GROUP_TITLE: &str = "Bodies";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
enum PaintedBodyError {
    #[error("Select a face, edge or vertex of a body, or a body in the tree")]
    NoBody,
    #[error("Select faces, edges or vertices of one body only, or one body in the tree")]
    SeveralBodies,
}

impl Reason for PaintedBodyError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

fn group_title(count: usize) -> String {
    format!("{GROUP_TITLE} ({count})")
}

pub fn listed(model: &Model) -> Vec<FeatureId> {
    let document = model.document();
    model
        .evaluation()
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| document.body_maker(*body).is_some())
        .collect()
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let bodies = listed(model);
    if state
        .painting
        .is_some_and(|painting| !bodies.contains(&painting.body))
    {
        state.painting = None;
    }
    let entries = body_groups::arrange(document, &bodies);
    let order = body_groups::in_order(&entries);
    let folders = body_groups::folders(&entries);
    if state
        .renaming_body_group
        .as_ref()
        .is_some_and(|renaming| !renaming.focus_pending && !folders.contains(&renaming.folder))
    {
        state.renaming_body_group = None;
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
    let renaming_group = state
        .renaming_body_group
        .as_ref()
        .is_some_and(|renaming| renaming.focus_pending);
    if renaming_group
        || state
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
                Id::unique("bodies-group-row"),
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
    let listing = Listing {
        order: &order,
        folders: &folders,
    };
    collapsing.show_body_unindented(ui, |ui| {
        rows(
            ui,
            model,
            selection,
            state,
            actions,
            (&entries, 0, &listing),
        );
    });
    collapsing.store(ui.ctx());
}

fn rows(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (entries, depth, listing): (&[Entry], usize, &Listing<'_>),
) {
    for entry in entries {
        match entry {
            Entry::Body(body) => {
                if let Some(height) = off_screen_height(ui, state, *body) {
                    ui.allocate_space(vec2(ui.available_width(), height));
                    continue;
                }
                let shown = item(
                    ui,
                    model,
                    selection,
                    state,
                    actions,
                    (*body, depth, listing),
                );
                if let Some(height) = shown.filter(|_| is_plain(ui, state, *body)) {
                    state.plain_body_row_height = Some(height);
                }
            }
            Entry::Group(group) => {
                if body_groups::header(ui, model, state, actions, (group, depth, listing)) {
                    rows(
                        ui,
                        model,
                        selection,
                        state,
                        actions,
                        (&group.entries, depth + 1, listing),
                    );
                }
            }
        }
    }
}

fn off_screen_height(ui: &Ui, state: &PanelState, body: FeatureId) -> Option<f32> {
    let height = state.plain_body_row_height?;
    (!feature_tree::near_view(ui, height) && is_plain(ui, state, body)).then_some(height)
}

fn is_plain(ui: &Ui, state: &PanelState, body: FeatureId) -> bool {
    state.painting.is_none_or(|painting| painting.body != body)
        && !ui.memory(|memory| memory.has_focus(row_id(body)))
}

fn row_id(body: FeatureId) -> Id {
    Id::unique(("body-row", body))
}

fn item(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (body, depth, listing): (FeatureId, usize, &Listing<'_>),
) -> Option<f32> {
    let document = model.document();
    let feature = document.body_maker(body)?;
    let name = document.body_name(body)?;
    let name = name.as_ref();
    let painting = state.painting.filter(|painting| painting.body == body);
    let shown = !document.body_hidden(body);
    let muted = appearance::tokens(ui).text_muted;
    let toggle = visibility::toggle_body(document, body);
    let look = Look {
        selected: state.reads_body_selected(body),
        open: false,
    };
    let row = tree_row::show(
        ui,
        look,
        |ui| {
            ui.add_space(CHILD_INDENT + depth as f32 * GROUP_INDENT);
            widgets::icon_label(ui, icons::feature(&feature.kind), muted);
            ui.add(
                Label::new(label(ui, name, shown))
                    .selectable(false)
                    .truncate(),
            );
            tree_row::sense(ui, row_id(body), ui.min_rect().left(), name, look.selected)
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
    if ui.rect_contains_pointer(row.rect) {
        state.hovered_row = Some(body);
    }
    if row_name.clicked() {
        let modifiers = ui.input(|input| input.modifiers);
        feature_tree::choose_among(state, listing.order, body, modifiers);
        state.body_group = None;
    } else if row_name.gained_focus() {
        state.choose_only(body);
        state.body_group = None;
    }
    row_name.context_menu(|ui| {
        widgets::fitted_menu(ui, |ui| {
            row_menu(ui, model, selection, state, actions, (body, listing));
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
        return Some(row.rect.height());
    };
    let focused = tree_row::indented(ui, |ui| {
        ui.add_space(SPACE_S);
        widgets::card(ui, |ui| {
            body_appearance::show(
                ui,
                model,
                selection,
                actions,
                body,
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
    None
}

fn row_menu(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (body, listing): (FeatureId, &Listing<'_>),
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
    body_groups::body_menu(ui, model.document(), state, actions, (body, listing));
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
) -> Result<FeatureId, PaintedBodyError> {
    let document = context.model.document();
    let tree = body_selection::tree_bodies(document, &state.chosen());
    let chosen = if tree.is_empty() {
        body_selection::bodies_in(context.selection)
    } else {
        tree
    };
    match chosen.as_slice() {
        [body] => return Ok(*body),
        [_, _, ..] => return Err(PaintedBodyError::SeveralBodies),
        [] => {}
    }
    context
        .editing
        .feature()
        .or(context.editing.solid())
        .and_then(|id| document.feature(id))
        .filter(|feature| !feature.separates_pieces())
        .and_then(Feature::body)
        .filter(|body| document.feature(*body).is_some_and(Feature::makes_body))
        .ok_or(PaintedBodyError::NoBody)
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
        .map(Cow::into_owned);
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
    body_groups::commands(context, state, commands, actions);
}

fn label(ui: &Ui, text: &str, shown: bool) -> RichText {
    let text = RichText::new(text);
    if shown {
        text
    } else {
        text.color(appearance::tokens(ui).text_muted).italics()
    }
}

pub fn eye(ui: &mut Ui, shown: bool, name: &str) -> bool {
    let (glyph, hover) = if shown {
        (icons::SHOW, "Hide")
    } else {
        (icons::HIDE, "Show")
    };
    widgets::named(widgets::icon_button(ui, glyph, hover), name).clicked()
}
