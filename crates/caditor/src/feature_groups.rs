use caditor_document::{Document, FeatureId, Transaction, group_name};
use egui::{Id, Key, Label, Popup, Rect, Response, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance, field, icons,
    model::Action,
    panels::{Focus, PanelState, Renaming},
    tree_row::{self, Look},
    widgets,
};

pub const NOTHING_TO_GROUP: &str = "Select features in the tree to group them";
pub const NOT_IN_A_GROUP: &str = "The feature chosen in the tree is in no group";
pub const UNGROUP_LABEL: &str = "Ungroup";
pub const RENAME_GROUP_LABEL: &str = "Rename group";
const GROUP_MORE_HINT: &str = "Rename or ungroup";
const UNNAMED_GROUP: &str = "A group needs a name; Ungroup takes its features out of it";

pub struct Header {
    pub rect: Rect,
    pub open: bool,
}

enum Name {
    Shown(Response),
    Renaming(field::FieldResponse<Transaction>),
}

pub fn grouping(document: &Document, features: &[FeatureId]) -> Result<Transaction, String> {
    if features.is_empty() {
        return Err(NOTHING_TO_GROUP.to_owned());
    }
    let name = document.unused_group_name();
    let label = format!(
        "Group {}",
        crate::feature_tree::count(features.len(), "feature", "features")
    );
    document
        .grouping(features, &name, label)
        .map_err(|error| error.to_string())
}

pub fn ungrouping(document: &Document, member: FeatureId) -> Result<Transaction, String> {
    let name = document.group_of(member).ok_or(NOT_IN_A_GROUP)?;
    document
        .regrouping(&document.group_run(member), None, format!("Ungroup {name}"))
        .map_err(|error| error.to_string())
}

pub fn start_renaming(state: &mut PanelState, member: FeatureId) {
    state.renaming_group = Some(Renaming {
        feature: member,
        focus_pending: true,
    });
}

pub fn header(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    run: &[FeatureId],
    name: &str,
) -> Header {
    let Some(&first) = run.first() else {
        return Header {
            rect: Rect::NOTHING,
            open: true,
        };
    };
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("feature-group", first)),
        true,
    );
    let renaming = state
        .renaming_group
        .filter(|renaming| run.contains(&renaming.feature));
    let wanted = run
        .iter()
        .any(|id| state.revealing(*id) || state.wants_focus(Focus::Feature(*id)));
    if renaming.is_some() || wanted {
        collapsing.set_open(true);
    }
    let open = collapsing.is_open();
    let chosen = state.chosen();
    let selected = run.iter().all(|id| chosen.contains(id));
    let muted = appearance::tokens(ui).text_muted;
    let shown = tree_row::show(
        ui,
        Look {
            selected,
            open: false,
        },
        |ui| {
            let toggle = tree_row::chevron(ui, open, name);
            widgets::icon_label(ui, icons::FEATURE_GROUP, muted);
            let named = match renaming {
                Some(renaming) => Name::Renaming(rename_field(ui, document, run, name, renaming)),
                None => {
                    ui.add(Label::new(name).selectable(false).truncate());
                    ui.label(widgets::muted(run.len().to_string(), ui));
                    Name::Shown(tree_row::sense(
                        ui,
                        Id::new(("feature-group-row", first)),
                        toggle.rect.right(),
                        name,
                        selected,
                    ))
                }
            };
            (toggle.clicked(), named)
        },
        |ui| {
            tree_row::slot(ui, |ui| {
                let response = widgets::named(
                    widgets::icon_button(ui, icons::MORE, GROUP_MORE_HINT),
                    &format!("More actions for {name}"),
                );
                Popup::menu(&response).show(|ui| {
                    widgets::fitted_menu(ui, |ui| menu(ui, document, state, actions, first));
                });
            });
        },
    );
    let (toggled, named) = shown.leading;
    if toggled {
        collapsing.toggle(ui);
    }
    match named {
        Name::Shown(response) => {
            if response.has_focus() {
                tree_row::focus_outline(ui, shown.rect);
            }
            let enter = ui.input(|input| input.key_pressed(Key::Enter));
            if response.clicked() || response.gained_focus() {
                state.choose_all(run);
            }
            if response.double_clicked() {
                start_renaming(state, first);
            } else if response.has_focus() && enter {
                collapsing.toggle(ui);
            }
            response.context_menu(|ui| {
                widgets::fitted_menu(ui, |ui| menu(ui, document, state, actions, first));
            });
        }
        Name::Renaming(field) => {
            if let Some(transaction) = field.committed {
                actions.push(Action::Apply(transaction));
            }
            state.renaming_group = if field.response.lost_focus() && field.error.is_none() {
                None
            } else {
                Some(Renaming {
                    feature: first,
                    focus_pending: false,
                })
            };
        }
    }
    collapsing.store(ui.ctx());
    Header {
        rect: shown.rect,
        open,
    }
}

fn rename_field(
    ui: &mut Ui,
    document: &Document,
    run: &[FeatureId],
    name: &str,
    renaming: Renaming,
) -> field::FieldResponse<Transaction> {
    let width = ui.available_width();
    field::commit_field(
        ui,
        Id::new(("rename-group", renaming.feature)),
        name,
        width,
        renaming.focus_pending,
        |text| {
            let renamed = group_name(text).ok_or(UNNAMED_GROUP)?;
            document
                .regrouping(run, Some(renamed), format!("Rename {name}"))
                .map_err(|error| error.to_string())
        },
    )
}

fn menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    member: FeatureId,
) {
    if widgets::menu_item(ui, icons::RENAME, RENAME_GROUP_LABEL, None).clicked() {
        start_renaming(state, member);
        ui.close();
    }
    if widgets::menu_item(
        ui,
        icons::command(crate::commands::Command::Ungroup),
        UNGROUP_LABEL,
        None,
    )
    .clicked()
    {
        if let Ok(transaction) = ungrouping(document, member) {
            actions.push(Action::Apply(transaction));
        }
        ui.close();
    }
}
