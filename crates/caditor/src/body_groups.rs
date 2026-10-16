use std::{borrow::Cow, collections::BTreeMap};

use caditor_document::{
    BodyGrouping, Document, EditError, FeatureId, Transaction, folder_label, group_name, is_within,
};
use egui::{Id, Key, Label, Popup, Response, Ui, collapsing_header::CollapsingState};

use crate::{
    appearance, bodies_tree, body_selection,
    commands::{Command, CommandFrame, Reason},
    feature_tree::{self, CommandContext},
    field, icons,
    model::{Action, Model, Notice},
    panels::{GroupRenaming, PanelState},
    selection::Selection,
    tree_row::{self, CHILD_CHEVRON_INDENT, GROUP_INDENT, Look},
    visibility, widgets,
};

pub const MOVE_TO_GROUP_LABEL: &str = "Move to group";
pub const TOP_LEVEL_LABEL: &str = "Out of every group";
const GROUP_MORE_HINT: &str = "Rename, ungroup or move";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BodyGroupError {
    #[error("Select bodies in the Bodies list or in the view to group them")]
    NothingToGroup,
    #[error("Choose a group in the Bodies list, or a body in one")]
    NoGroupChosen,
    #[error("The bodies chosen are in no group")]
    NotInAGroup,
    #[error("A group needs a name; Ungroup takes its bodies out of it")]
    Unnamed,
    #[error("{group} is in no other group")]
    InNoOtherGroup { group: String },
    #[error(transparent)]
    Rejected(#[from] EditError),
}

impl Reason for BodyGroupError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

pub enum Entry {
    Body(FeatureId),
    Group(Group),
}

pub struct Group {
    pub folder: Vec<String>,
    pub entries: Vec<Entry>,
    pub members: Vec<FeatureId>,
}

impl Group {
    fn name(&self) -> &str {
        self.folder.last().map_or("", String::as_str)
    }
}

pub fn arrange(document: &Document, bodies: &[FeatureId]) -> Vec<Entry> {
    let placed = bodies
        .iter()
        .map(|body| (*body, document.body_folder(*body)))
        .collect();
    arranged(&[], placed)
}

fn arranged(parent: &[String], bodies: Vec<(FeatureId, Vec<String>)>) -> Vec<Entry> {
    enum Slot {
        Body(FeatureId),
        Group(String),
    }
    let mut slots = Vec::new();
    let mut inside: BTreeMap<String, Vec<(FeatureId, Vec<String>)>> = BTreeMap::new();
    for (body, folder) in bodies {
        let Some(name) = folder.get(parent.len()).cloned() else {
            slots.push(Slot::Body(body));
            continue;
        };
        let members = inside.entry(name.clone()).or_default();
        if members.is_empty() {
            slots.push(Slot::Group(name));
        }
        members.push((body, folder));
    }
    slots
        .into_iter()
        .filter_map(|slot| match slot {
            Slot::Body(body) => Some(Entry::Body(body)),
            Slot::Group(name) => {
                let members = inside.remove(&name)?;
                let folder: Vec<String> = parent.iter().cloned().chain([name]).collect();
                Some(Entry::Group(Group {
                    members: members.iter().map(|(body, _)| *body).collect(),
                    entries: arranged(&folder, members),
                    folder,
                }))
            }
        })
        .collect()
}

pub fn in_order(entries: &[Entry]) -> Vec<FeatureId> {
    entries
        .iter()
        .flat_map(|entry| match entry {
            Entry::Body(body) => vec![*body],
            Entry::Group(group) => in_order(&group.entries),
        })
        .collect()
}

pub fn folders(entries: &[Entry]) -> Vec<Vec<String>> {
    entries
        .iter()
        .flat_map(|entry| match entry {
            Entry::Body(_) => Vec::new(),
            Entry::Group(group) => std::iter::once(group.folder.clone())
                .chain(folders(&group.entries))
                .collect(),
        })
        .collect()
}

fn body_count(amount: usize) -> String {
    feature_tree::count(amount, "body", "bodies")
}

fn bodies_named(document: &Document, bodies: &[FeatureId]) -> String {
    match bodies {
        [body] => document
            .body_name(*body)
            .map_or_else(|| body_count(1), |name| name.into_owned()),
        _ => body_count(bodies.len()),
    }
}

fn place_words(folder: &[String]) -> String {
    if folder.is_empty() {
        "out of every group".to_owned()
    } else {
        format!("into {}", folder_label(folder))
    }
}

pub fn grouping(
    document: &Document,
    bodies: &[FeatureId],
    listed: &[FeatureId],
) -> Result<BodyGrouping, BodyGroupError> {
    if bodies.is_empty() {
        return Err(BodyGroupError::NothingToGroup);
    }
    Ok(document.body_grouping(
        bodies,
        listed,
        format!("Group {}", bodies_named(document, bodies)),
    )?)
}

pub fn ungrouping(document: &Document, folder: &[String]) -> Result<Transaction, BodyGroupError> {
    Ok(document.body_ungrouping(folder, format!("Ungroup {}", folder_label(folder)))?)
}

pub fn renaming(
    document: &Document,
    folder: &[String],
    text: &str,
) -> Result<Transaction, BodyGroupError> {
    let name = group_name(text).ok_or(BodyGroupError::Unnamed)?;
    Ok(document.body_group_renaming(folder, &name, format!("Rename {}", folder_label(folder)))?)
}

pub fn moving_group(
    document: &Document,
    folder: &[String],
    into: &[String],
) -> Result<Transaction, BodyGroupError> {
    let label = format!("Move {} {}", folder_label(folder), place_words(into));
    Ok(document.body_group_moving(folder, into, label)?)
}

pub fn moving_bodies(
    document: &Document,
    bodies: &[FeatureId],
    into: &[String],
) -> Result<Transaction, BodyGroupError> {
    let label = format!(
        "Move {} {}",
        bodies_named(document, bodies),
        place_words(into)
    );
    Ok(document.bodies_moving(bodies, into, label)?)
}

pub fn chosen_bodies(
    document: &Document,
    state: &PanelState,
    selection: &Selection,
) -> Vec<FeatureId> {
    let tree = body_selection::tree_bodies(document, &state.chosen());
    if tree.is_empty() {
        body_selection::bodies_in(selection)
    } else {
        tree
    }
}

fn chosen_row<'a>(
    document: &Document,
    state: &'a PanelState,
    chosen: &[FeatureId],
) -> Option<&'a Vec<String>> {
    state.body_group.as_ref().filter(|folder| {
        !chosen.is_empty()
            && chosen
                .iter()
                .all(|body| is_within(&document.body_folder(*body), folder))
    })
}

pub fn current_group(
    document: &Document,
    state: &PanelState,
    chosen: &[FeatureId],
) -> Result<Vec<String>, BodyGroupError> {
    if let Some(folder) = chosen_row(document, state, chosen) {
        return Ok(folder.clone());
    }
    chosen
        .first()
        .map(|body| document.body_folder(*body))
        .filter(|folder| !folder.is_empty())
        .ok_or(BodyGroupError::NoGroupChosen)
}

fn moving_out(
    document: &Document,
    state: &PanelState,
    chosen: &[FeatureId],
) -> Result<Transaction, BodyGroupError> {
    if let Some(folder) = chosen_row(document, state, chosen) {
        let Some(parent) = folder
            .len()
            .checked_sub(2)
            .and_then(|end| folder.get(..end))
        else {
            return Err(BodyGroupError::InNoOtherGroup {
                group: folder_label(folder),
            });
        };
        return moving_group(document, folder, parent);
    }
    let grouped: Vec<FeatureId> = chosen
        .iter()
        .copied()
        .filter(|body| !document.body_folder(*body).is_empty())
        .collect();
    if grouped.is_empty() {
        return Err(BodyGroupError::NotInAGroup);
    }
    let label = format!(
        "Move {} out of {}",
        bodies_named(document, &grouped),
        if grouped.len() == 1 {
            "its group"
        } else {
            "their groups"
        }
    );
    Ok(document.bodies_moving_out(&grouped, label)?)
}

pub fn start_renaming(state: &mut PanelState, folder: Vec<String>) {
    state.renaming_body_group = Some(GroupRenaming {
        folder,
        focus_pending: true,
    });
}

fn apply_or_tell(
    actions: &mut Vec<Action>,
    title: &str,
    made: Result<Transaction, BodyGroupError>,
) {
    match made {
        Ok(transaction) => actions.push(Action::Apply(transaction)),
        Err(reason) => actions.push(Action::Inform(Notice::warning(format!(
            "{title}: {reason}."
        )))),
    }
}

pub fn group_chosen(
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (chosen, listed): (&[FeatureId], &[FeatureId]),
) {
    match grouping(document, chosen, listed) {
        Ok(grouped) => {
            actions.push(Action::Apply(grouped.transaction));
            start_renaming(state, grouped.folder);
        }
        Err(reason) => actions.push(Action::Inform(Notice::warning(format!(
            "{}: {reason}.",
            Command::GroupBodies.title()
        )))),
    }
}

pub fn commands(
    context: &CommandContext<'_>,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let document = context.model.document();
    let chosen = chosen_bodies(document, state, context.selection);
    let groupable: Result<(), BodyGroupError> = if chosen.is_empty() {
        Err(BodyGroupError::NothingToGroup)
    } else {
        Ok(())
    };
    if commands.invoke(Command::GroupBodies, &groupable) {
        let listed = bodies_tree::listed(context.model);
        group_chosen(document, state, actions, (&chosen, &listed));
    }
    let group = current_group(document, state, &chosen);
    let detail = group.as_ref().ok().map(|folder| folder_label(folder));
    if commands.invoke_detailed(Command::UngroupBodies, detail.clone(), &group)
        && let Ok(folder) = &group
    {
        apply_or_tell(
            actions,
            &Command::UngroupBodies.title(),
            ungrouping(document, folder),
        );
    }
    if commands.invoke_detailed(Command::RenameBodyGroup, detail, &group)
        && let Ok(folder) = group
    {
        start_renaming(state, folder);
    }
    let in_a_group = chosen_row(document, state, &chosen).is_some()
        || chosen
            .iter()
            .any(|body| !document.body_folder(*body).is_empty());
    let out: Result<(), BodyGroupError> = if in_a_group {
        Ok(())
    } else {
        Err(BodyGroupError::NotInAGroup)
    };
    if commands.invoke(Command::MoveOutOfBodyGroup, &out) {
        apply_or_tell(
            actions,
            &Command::MoveOutOfBodyGroup.title(),
            moving_out(document, state, &chosen),
        );
    }
}

pub struct Listing<'a> {
    pub order: &'a [FeatureId],
    pub folders: &'a [Vec<String>],
}

enum Name {
    Shown(Response),
    Renaming(field::FieldResponse<Transaction>),
}

fn wants_open(state: &PanelState, group: &Group) -> bool {
    let renaming_inside = state.renaming_body_group.as_ref().is_some_and(|renaming| {
        renaming.folder.len() > group.folder.len() && is_within(&renaming.folder, &group.folder)
    });
    let painting_inside = state
        .painting
        .is_some_and(|painting| painting.focus_pending && group.members.contains(&painting.body));
    renaming_inside || painting_inside
}

pub fn header(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (group, depth, listing): (&Group, usize, &Listing<'_>),
) -> bool {
    let document = model.document();
    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("body-group", &group.folder)),
        true,
    );
    if wants_open(state, group) {
        collapsing.set_open(true);
    }
    let renaming = state
        .renaming_body_group
        .as_ref()
        .filter(|renaming| renaming.folder == group.folder)
        .map(|renaming| renaming.focus_pending);
    let open = collapsing.is_open();
    let chosen = state.chosen();
    let selected = state.body_group.as_ref() == Some(&group.folder)
        && group.members.iter().all(|body| chosen.contains(body));
    let name = group.name();
    let shown = group
        .members
        .iter()
        .any(|body| !document.body_hidden(*body));
    let toggle = visibility::toggle_bodies(document, &group.members, name);
    let muted = appearance::tokens(ui).text_muted;
    let row = tree_row::show(
        ui,
        Look {
            selected,
            open: false,
        },
        |ui| {
            ui.add_space(CHILD_CHEVRON_INDENT + depth as f32 * GROUP_INDENT);
            let chevron = tree_row::chevron(ui, open, name);
            widgets::icon_label(ui, icons::FEATURE_GROUP, muted);
            let named = match renaming {
                Some(focus_pending) => {
                    Name::Renaming(rename_field(ui, document, &group.folder, focus_pending))
                }
                None => {
                    ui.add(Label::new(name).selectable(false).truncate());
                    ui.label(widgets::muted(group.members.len().to_string(), ui));
                    Name::Shown(tree_row::sense(
                        ui,
                        Id::unique(("body-group-row", &group.folder)),
                        chevron.rect.right(),
                        name,
                        selected,
                    ))
                }
            };
            (chevron.clicked(), named)
        },
        |ui| {
            tree_row::slot(ui, |ui| {
                let response = widgets::named(
                    widgets::icon_button(ui, icons::MORE, GROUP_MORE_HINT),
                    &format!("More actions for {name}"),
                );
                Popup::menu(&response).show(|ui| {
                    widgets::fitted_menu(ui, |ui| {
                        group_menu(ui, document, state, actions, (group, listing));
                    });
                });
            });
            tree_row::slot(ui, |ui| match &toggle {
                Ok(transaction) => bodies_tree::eye(ui, shown, transaction.label()),
                Err(_) => false,
            })
        },
    );
    let (toggled, named) = row.leading;
    if toggled {
        collapsing.toggle(ui);
    }
    if row.trailing
        && let Ok(transaction) = toggle
    {
        actions.push(Action::Apply(transaction));
    }
    match named {
        Name::Shown(response) => {
            if response.has_focus() {
                tree_row::focus_outline(ui, row.rect);
            }
            if response.clicked() || response.gained_focus() {
                state.choose_all(&group.members);
                state.body_group = Some(group.folder.clone());
            }
            if response.double_clicked() {
                start_renaming(state, group.folder.clone());
            } else if response.has_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                collapsing.toggle(ui);
            }
            response.context_menu(|ui| {
                widgets::fitted_menu(ui, |ui| {
                    group_menu(ui, document, state, actions, (group, listing));
                });
            });
        }
        Name::Renaming(field) => {
            if let Some(transaction) = field.committed {
                actions.push(Action::Apply(transaction));
            }
            state.renaming_body_group = if field.response.lost_focus() && field.error.is_none() {
                None
            } else {
                Some(GroupRenaming {
                    folder: group.folder.clone(),
                    focus_pending: false,
                })
            };
        }
    }
    let open = collapsing.is_open();
    collapsing.store(ui.ctx());
    open
}

fn rename_field(
    ui: &mut Ui,
    document: &Document,
    folder: &[String],
    focus_pending: bool,
) -> field::FieldResponse<Transaction> {
    let width = ui.available_width();
    field::commit_field(
        ui,
        Id::unique(("rename-body-group", folder)),
        folder.last().map_or("", String::as_str),
        width,
        focus_pending,
        |text| renaming(document, folder, text),
    )
}

fn group_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (group, listing): (&Group, &Listing<'_>),
) {
    if widgets::menu_item(ui, icons::RENAME, "Rename group", None).clicked() {
        start_renaming(state, group.folder.clone());
        ui.close();
    }
    if widgets::menu_item(ui, icons::command(Command::UngroupBodies), "Ungroup", None).clicked() {
        apply_or_tell(actions, "Ungroup", ungrouping(document, &group.folder));
        ui.close();
    }
    let parent = group
        .folder
        .split_last()
        .map_or(&[][..], |(_, parent)| parent);
    let targets: Vec<&Vec<String>> = listing
        .folders
        .iter()
        .filter(|folder| !is_within(folder, &group.folder) && folder.as_slice() != parent)
        .collect();
    if let Some(into) = move_menu(ui, &targets, !parent.is_empty()) {
        apply_or_tell(
            actions,
            MOVE_TO_GROUP_LABEL,
            moving_group(document, &group.folder, &into),
        );
    }
}

pub fn body_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    (body, listing): (FeatureId, &Listing<'_>),
) {
    let chosen = body_selection::tree_bodies(document, &state.chosen());
    let bodies = if chosen.contains(&body) {
        chosen
    } else {
        vec![body]
    };
    ui.separator();
    if widgets::menu_item(
        ui,
        icons::command(Command::GroupBodies),
        &Command::GroupBodies.title(),
        None,
    )
    .clicked()
    {
        group_chosen(document, state, actions, (&bodies, listing.order));
        ui.close();
    }
    let own = document.body_folder(body);
    let targets: Vec<&Vec<String>> = listing
        .folders
        .iter()
        .filter(|folder| **folder != own)
        .collect();
    if let Some(into) = move_menu(ui, &targets, !own.is_empty()) {
        apply_or_tell(
            actions,
            MOVE_TO_GROUP_LABEL,
            moving_bodies(document, &bodies, &into),
        );
    }
    if !own.is_empty()
        && widgets::menu_item(
            ui,
            icons::command(Command::MoveOutOfBodyGroup),
            &Command::MoveOutOfBodyGroup.title(),
            None,
        )
        .clicked()
    {
        state.body_group = None;
        apply_or_tell(
            actions,
            &Command::MoveOutOfBodyGroup.title(),
            moving_out(document, state, &bodies),
        );
        ui.close();
    }
}

fn move_menu(ui: &mut Ui, targets: &[&Vec<String>], top_level: bool) -> Option<Vec<String>> {
    if targets.is_empty() && !top_level {
        return None;
    }
    let muted = appearance::tokens(ui).text_muted;
    let mut chosen = None;
    let submenu = ui.menu_button(
        (
            widgets::icon(icons::MOVE_TO_GROUP).color(muted),
            MOVE_TO_GROUP_LABEL.to_owned(),
        ),
        |ui| {
            widgets::fitted_menu(ui, |ui| {
                for folder in targets {
                    let label = folder_label(folder);
                    if widgets::menu_item(ui, icons::FEATURE_GROUP, &label, None).clicked() {
                        chosen = Some((*folder).clone());
                        ui.close();
                    }
                }
                if top_level
                    && widgets::menu_item(ui, icons::TOP_LEVEL, TOP_LEVEL_LABEL, None).clicked()
                {
                    chosen = Some(Vec::new());
                    ui.close();
                }
            });
        },
    );
    widgets::named(submenu.response, MOVE_TO_GROUP_LABEL);
    chosen
}
