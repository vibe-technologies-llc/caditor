use std::collections::{BTreeMap, BTreeSet};

use caditor_document::{
    Document, Edit, FeatureId, MAX_SET_NAME_CHARS, SelectionSet, SelectionSets, SetItem, SetLoss,
    SetMember, SolidResult, Transaction, set_name,
};
use caditor_kernel::{EdgeName, EdgeNaming, EdgeReference, FaceId, FaceReference};
use egui::{Id, Key, ScrollArea, Sides, TextEdit, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    bodies::{self, FaceKey},
    body_selection,
    commands::Command,
    dialog_parts::BodyRoom,
    icons,
    model::Model,
    selection::{Pickable, Selection},
    visibility,
    widgets::{self, DialogWidth, Tone},
};

pub const TITLE: &str = "Selection sets";
pub const CLOSE_LABEL: &str = "Close";
pub const SAVE_LABEL: &str = "Save selection";
pub const NAME_CAPTION: &str = "Name";
pub const NOTHING_TO_KEEP: &str =
    "Select faces, edges or whole bodies first; a selection set keeps those";
const EXPLANATION: &str = "Keep a group of faces, edges or bodies under a name and select it \
                           again later, for a fillet, a hide or a pattern. Sets belong to the \
                           model and follow its faces and edges as it changes.";
const EMPTY: &str = "No selection set is saved yet. Select faces, edges or bodies, then save them \
                     here or with the Save the selection as a set command.";
const DIALOG_ID: &str = "selection-sets";
const HEIGHT_SHARE: f32 = 0.8;
const LIST_HEIGHT: f32 = 240.0;
const SELECT_HOVER: &str = "Select what this set holds";
const UPDATE_HOVER: &str = "Replace it with the current selection";
const RENAME_HOVER: &str = "Rename it";
const DELETE_HOVER: &str = "Delete it";

#[derive(Debug, Clone, PartialEq)]
pub struct Captured {
    pub members: Vec<SetMember>,
    pub left_out: usize,
}

pub fn capture(model: &Model, selection: &Selection) -> Captured {
    let evaluation = model.evaluation();
    let mut faces: BTreeMap<FeatureId, BTreeSet<FaceKey>> = BTreeMap::new();
    let mut edges: BTreeMap<FeatureId, Vec<EdgeName>> = BTreeMap::new();
    let mut left_out = 0_usize;
    for pickable in selection.in_pick_order() {
        match pickable {
            Pickable::Face { body, face } => {
                faces.entry(body).or_default().insert(face);
            }
            Pickable::Edge { body, edge } => edges.entry(body).or_default().push(edge),
            _ => left_out += 1,
        }
    }
    let mut members = Vec::new();
    for (body, keys) in faces {
        let Some(result) = bodies::shown(evaluation, body) else {
            left_out += keys.len();
            continue;
        };
        let every = bodies::face_keys(&result.solid);
        if every.len() == keys.len() && every.iter().all(|(_, key)| keys.contains(key)) {
            members.push(SetMember::Body(body));
            continue;
        }
        for key in keys {
            match bodies::find_face(result, key)
                .and_then(|face| FaceReference::capture(&result.solid, face))
            {
                Some(face) => members.push(SetMember::Face { body, face }),
                None => left_out += 1,
            }
        }
    }
    for (body, names) in edges {
        let Some(result) = bodies::shown(evaluation, body) else {
            left_out += names.len();
            continue;
        };
        let naming = EdgeNaming::new(&result.solid);
        for name in names {
            match bodies::find_edge(result, name)
                .and_then(|edge| EdgeReference::capture_in(&naming, edge))
            {
                Some(edge) => members.push(SetMember::Edge { body, edge }),
                None => left_out += 1,
            }
        }
    }
    Captured { members, left_out }
}

fn captured_members(model: &Model, selection: &Selection) -> Result<Captured, String> {
    let captured = capture(model, selection);
    if captured.members.is_empty() {
        return Err(NOTHING_TO_KEEP.to_owned());
    }
    Ok(captured)
}

pub fn left_out_note(left_out: usize) -> Option<String> {
    match left_out {
        0 => None,
        1 => Some(
            "One selected item is not a face, edge or body, so the set leaves it out.".to_owned(),
        ),
        count => Some(format!(
            "{count} selected items are not faces, edges or bodies, so the set leaves them out."
        )),
    }
}

fn replacing(
    document: &Document,
    label: String,
    change: impl FnOnce(&mut SelectionSets) -> Result<(), String>,
) -> Result<Transaction, String> {
    let mut sets = document.selection_sets().clone();
    change(&mut sets)?;
    let transaction = Transaction::single(
        label,
        Edit::SetSelectionSets {
            sets: Box::new(sets),
        },
    );
    document
        .check(&transaction)
        .map_err(|error| error.to_string())?;
    Ok(transaction)
}

fn missing(name: &str) -> String {
    format!("The selection set '{name}' no longer exists")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub transaction: Transaction,
    pub left_out: usize,
}

pub fn save(model: &Model, selection: &Selection, name: &str) -> Result<Saved, String> {
    let captured = captured_members(model, selection)?;
    let name = set_name(name);
    let transaction = replacing(
        model.document(),
        format!("Save selection set “{name}”"),
        |sets| {
            sets.sets.push(SelectionSet {
                name: name.clone(),
                members: captured.members,
            });
            Ok(())
        },
    )?;
    Ok(Saved {
        transaction,
        left_out: captured.left_out,
    })
}

pub fn update(model: &Model, selection: &Selection, name: &str) -> Result<Saved, String> {
    let captured = captured_members(model, selection)?;
    let transaction = replacing(
        model.document(),
        format!("Update selection set “{name}”"),
        |sets| {
            let index = sets.position(name).ok_or_else(|| missing(name))?;
            if let Some(set) = sets.sets.get_mut(index) {
                set.members = captured.members;
            }
            Ok(())
        },
    )?;
    Ok(Saved {
        transaction,
        left_out: captured.left_out,
    })
}

pub fn rename(document: &Document, name: &str, to: &str) -> Result<Transaction, String> {
    let to = set_name(to);
    replacing(
        document,
        format!("Rename selection set “{name}” to “{to}”"),
        |sets| {
            let index = sets.position(name).ok_or_else(|| missing(name))?;
            if let Some(set) = sets.sets.get_mut(index) {
                set.name = to.clone();
            }
            Ok(())
        },
    )
}

pub fn delete(document: &Document, name: &str) -> Result<Transaction, String> {
    replacing(
        document,
        format!("Delete selection set “{name}”"),
        |sets| {
            let index = sets.position(name).ok_or_else(|| missing(name))?;
            sets.sets.remove(index);
            Ok(())
        },
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    pub pickables: Vec<Pickable>,
    pub report: String,
}

#[derive(Debug, Clone, Default)]
struct Missed {
    bodies: Vec<FeatureId>,
    hidden: Vec<FeatureId>,
    faces: BTreeMap<FeatureId, usize>,
    edges: BTreeMap<FeatureId, usize>,
}

fn counted(count: usize, one: &str, several: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {several}"),
    }
}

fn joined(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl Missed {
    fn words(&self, document: &Document) -> Option<String> {
        let name = |body: FeatureId| {
            document
                .body_name(body)
                .map_or_else(|| "a deleted body".to_owned(), str::to_owned)
        };
        let mut parts: Vec<String> = Vec::new();
        for (body, count) in &self.faces {
            parts.push(format!(
                "{} of {} could not be found",
                counted(*count, "face", "faces"),
                name(*body)
            ));
        }
        for (body, count) in &self.edges {
            parts.push(format!(
                "{} of {} could not be found",
                counted(*count, "edge", "edges"),
                name(*body)
            ));
        }
        for body in &self.bodies {
            parts.push(match document.feature(*body) {
                Some(_) => format!("{} has no body now", name(*body)),
                None => "a body in it was deleted".to_owned(),
            });
        }
        for body in &self.hidden {
            parts.push(format!("{} is hidden", name(*body)));
        }
        (!parts.is_empty()).then(|| joined(&parts))
    }
}

fn pickable_of(
    item: SetItem,
    result: &SolidResult,
    keys: &BTreeMap<FaceId, FaceKey>,
) -> Option<Pickable> {
    match item {
        SetItem::Face { body, face } => Some(Pickable::Face {
            body,
            face: *keys.get(&face)?,
        }),
        SetItem::Edge { body, edge } => {
            if bodies::is_seam(&result.solid, edge) {
                return None;
            }
            Some(Pickable::Edge {
                body,
                edge: result.solid.edge(edge)?.name(),
            })
        }
    }
}

fn item_body(item: SetItem) -> FeatureId {
    match item {
        SetItem::Face { body, .. } | SetItem::Edge { body, .. } => body,
    }
}

pub fn choose(model: &Model, index: usize, in_sketch: bool) -> Result<Chosen, String> {
    if in_sketch {
        return Err(body_selection::IN_SKETCH.to_owned());
    }
    let document = model.document();
    let evaluation = model.evaluation();
    let set = document
        .selection_sets()
        .sets
        .get(index)
        .ok_or_else(|| "That selection set no longer exists".to_owned())?;
    let resolved = set.resolve(evaluation);
    let mut missed = Missed::default();
    for lost in resolved.lost {
        match lost {
            SetLoss::Body(body) => missed.bodies.push(body),
            SetLoss::Face(body) => *missed.faces.entry(body).or_default() += 1,
            SetLoss::Edge(body) => *missed.edges.entry(body).or_default() += 1,
        }
    }
    let mut keys: BTreeMap<FeatureId, BTreeMap<FaceId, FaceKey>> = BTreeMap::new();
    let mut pickables = Vec::new();
    for item in resolved.found {
        let body = item_body(item);
        if !visibility::is_shown(document, body) {
            if !missed.hidden.contains(&body) {
                missed.hidden.push(body);
            }
            continue;
        }
        let Some(result) = bodies::shown(evaluation, body) else {
            continue;
        };
        let keys = keys
            .entry(body)
            .or_insert_with(|| bodies::face_keys(&result.solid).into_iter().collect());
        pickables.extend(pickable_of(item, result, keys));
    }
    let name = &set.name;
    let missed = missed.words(document);
    if pickables.is_empty() {
        return Err(match missed {
            Some(missed) => format!(
                "Nothing in the selection set “{name}” can be selected now: {missed}. Show its \
                 bodies or replace the set with a new selection"
            ),
            None => format!("Nothing in the selection set “{name}” can be selected now"),
        });
    }
    let report = match missed {
        Some(missed) => format!(
            "Selected what is left of “{name}”, but {missed}. Replace the set with the current \
             selection to keep it as it is now."
        ),
        None => format!("Selected the set “{name}”."),
    };
    Ok(Chosen { pickables, report })
}

#[derive(Debug, Clone, PartialEq)]
struct Renaming {
    from: String,
    to: String,
    focus_pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetsDraft {
    name: String,
    renaming: Option<Renaming>,
    problem: Option<String>,
    note: Option<String>,
    focus_pending: bool,
}

impl SetsDraft {
    pub fn of(document: &Document) -> Self {
        Self {
            name: document.selection_sets().unused_name(),
            renaming: None,
            problem: None,
            note: None,
            focus_pending: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Apply(Transaction),
    Select(usize),
}

pub fn name_field_id() -> Id {
    Id::new((DIALOG_ID, "name"))
}

pub fn rename_field_id() -> Id {
    Id::new((DIALOG_ID, "rename"))
}

pub fn dialog(
    ctx: &egui::Context,
    model: &Model,
    selection: &Selection,
    draft: &mut SetsDraft,
) -> Option<Outcome> {
    let response = widgets::dialog(ctx, DIALOG_ID, TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(EXPLANATION, ui));
        ui.add_space(SPACE_S);
        let mut room = BodyRoom::measure(ui, DIALOG_ID, HEIGHT_SHARE);
        let outcome = ScrollArea::vertical()
            .max_height(room.height)
            .show(ui, |ui| body(ui, model, selection, draft))
            .inner;
        room.body_ended(ui);
        let closed = widgets::footer(ui, |ui| {
            ui.add(widgets::primary_button(ui, CLOSE_LABEL)).clicked()
        });
        room.dialog_ended(ui);
        match (outcome, closed) {
            (Some(outcome), _) => Some(outcome),
            (None, true) => Some(Outcome::Close),
            (None, false) => None,
        }
    });
    let closed = response.should_close().then_some(Outcome::Close);
    response.inner.or(closed)
}

fn body(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    draft: &mut SetsDraft,
) -> Option<Outcome> {
    let sets = model.document().selection_sets();
    let mut outcome = None;
    widgets::caption(ui, "Sets");
    if sets.sets.is_empty() {
        ui.label(widgets::muted(EMPTY, ui));
    } else {
        ScrollArea::vertical()
            .id_salt((DIALOG_ID, "list"))
            .max_height(widgets::list_height(ui.ctx(), LIST_HEIGHT))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, set) in sets.sets.iter().enumerate() {
                    if let Some(chosen) = row(ui, model, selection, draft, index, set) {
                        outcome = Some(chosen);
                    }
                }
            });
    }
    ui.add_space(SPACE_M);
    if let Some(saved) = saving(ui, model, selection, draft) {
        outcome = Some(saved);
    }
    if let Some(problem) = &draft.problem {
        ui.add_space(SPACE_S);
        widgets::callout(ui, Tone::Error, |ui| ui.label(problem));
    } else if let Some(note) = &draft.note {
        ui.add_space(SPACE_S);
        widgets::callout(ui, Tone::Info, |ui| ui.label(note));
    }
    outcome
}

fn applying(draft: &mut SetsDraft, built: Result<Transaction, String>) -> Option<Outcome> {
    draft.note = None;
    match built {
        Ok(transaction) => {
            draft.problem = None;
            Some(Outcome::Apply(transaction))
        }
        Err(problem) => {
            draft.problem = Some(problem);
            None
        }
    }
}

fn applying_saved(draft: &mut SetsDraft, built: Result<Saved, String>) -> Option<Outcome> {
    let left_out = built.as_ref().map_or(0, |saved| saved.left_out);
    let outcome = applying(draft, built.map(|saved| saved.transaction));
    draft.note = left_out_note(left_out);
    outcome
}

fn members_text(set: &SelectionSet) -> String {
    let count =
        |wanted: fn(&SetMember) -> bool| set.members.iter().filter(|member| wanted(member)).count();
    let parts: Vec<String> = [
        (
            count(|member| matches!(member, SetMember::Body(_))),
            "body",
            "bodies",
        ),
        (
            count(|member| matches!(member, SetMember::Face { .. })),
            "face",
            "faces",
        ),
        (
            count(|member| matches!(member, SetMember::Edge { .. })),
            "edge",
            "edges",
        ),
    ]
    .into_iter()
    .filter(|(count, _, _)| *count > 0)
    .map(|(count, one, several)| counted(count, one, several))
    .collect();
    joined(&parts)
}

fn row(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    draft: &mut SetsDraft,
    index: usize,
    set: &SelectionSet,
) -> Option<Outcome> {
    let renaming = draft
        .renaming
        .as_ref()
        .is_some_and(|renaming| renaming.from == set.name);
    if renaming {
        return renaming_row(ui, model.document(), draft);
    }
    let mut outcome = None;
    let mut rename = false;
    let mut delete = false;
    let mut update = false;
    let mut select = false;
    Sides::new().shrink_left().truncate().show(
        ui,
        |ui| {
            ui.label(&set.name);
            ui.label(widgets::muted(members_text(set), ui));
        },
        |ui| {
            delete = widgets::icon_button(ui, icons::DELETE, DELETE_HOVER).clicked();
            rename = widgets::icon_button(ui, icons::RENAME, RENAME_HOVER).clicked();
            update =
                widgets::icon_button(ui, icons::command(Command::SaveSelectionSet), UPDATE_HOVER)
                    .clicked();
            select = widgets::icon_button(ui, icons::command(Command::SelectionSets), SELECT_HOVER)
                .clicked();
        },
    );
    if select {
        draft.problem = None;
        draft.note = None;
        outcome = Some(Outcome::Select(index));
    }
    if update {
        let built = self::update(model, selection, &set.name);
        outcome = applying_saved(draft, built);
    }
    if delete {
        let built = self::delete(model.document(), &set.name);
        outcome = applying(draft, built);
    }
    if rename {
        draft.renaming = Some(Renaming {
            from: set.name.clone(),
            to: set.name.clone(),
            focus_pending: true,
        });
    }
    outcome
}

fn renaming_row(ui: &mut Ui, document: &Document, draft: &mut SetsDraft) -> Option<Outcome> {
    let renaming = draft.renaming.as_mut()?;
    let focus = std::mem::take(&mut renaming.focus_pending);
    let mut done = false;
    let mut cancelled = false;
    ui.horizontal(|ui| {
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut renaming.to)
                    .id(rename_field_id())
                    .char_limit(MAX_SET_NAME_CHARS)
                    .desired_width(ui.available_width() - 2.0 * ui.spacing().interact_size.y),
            )
        });
        if focus {
            field.request_focus();
        }
        let enter = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let escape = field.has_focus() && ui.input(|input| input.key_pressed(Key::Escape));
        done = widgets::icon_button(ui, icons::DONE, "Rename the set").clicked() || enter;
        cancelled =
            widgets::icon_button(ui, icons::CLOSE, "Keep the name (Esc)").clicked() || escape;
    });
    if cancelled {
        draft.renaming = None;
        return None;
    }
    if !done {
        return None;
    }
    let renaming = draft.renaming.take()?;
    let built = rename(document, &renaming.from, &renaming.to);
    applying(draft, built)
}

fn saving(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    draft: &mut SetsDraft,
) -> Option<Outcome> {
    let focus = std::mem::take(&mut draft.focus_pending);
    let mut save_now = false;
    widgets::properties(ui, (DIALOG_ID, "save"), |ui| {
        widgets::property(ui, NAME_CAPTION, |ui| {
            ui.horizontal(|ui| {
                let field = widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut draft.name)
                            .id(name_field_id())
                            .char_limit(MAX_SET_NAME_CHARS)
                            .desired_width(
                                ui.available_width() - 2.0 * ui.spacing().interact_size.y,
                            ),
                    )
                });
                widgets::tie_to_caption(ui, &field);
                if focus {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
                save_now = ui.add(widgets::button(SAVE_LABEL)).clicked() || enter;
            });
        });
    });
    if !save_now {
        return None;
    }
    let name = draft.name.clone();
    let built = save(model, selection, &name);
    let outcome = applying_saved(draft, built);
    if outcome.is_some() {
        let mut taken = model.document().selection_sets().clone();
        taken.sets.push(SelectionSet {
            name: set_name(&name),
            members: Vec::new(),
        });
        draft.name = taken.unused_name();
    }
    outcome
}
