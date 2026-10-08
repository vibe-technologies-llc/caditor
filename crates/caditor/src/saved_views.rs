use caditor_document::{
    Document, Edit, HOME_VIEW_NAME, MAX_VIEW_NAME_CHARS, NamedView, SavedView, SavedViews,
    Transaction, view_name,
};
use caditor_render::Viewpoint;
use egui::{Id, Key, ScrollArea, Sides, TextEdit, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    commands::{Command, StandardView},
    dialog_parts::BodyRoom,
    icons,
    widgets::{self, DialogWidth, Tone},
};

pub const TITLE: &str = "Saved views";
pub const CLOSE_LABEL: &str = "Close";
pub const SAVE_LABEL: &str = "Save view";
pub const NAME_CAPTION: &str = "Name";
const EXPLANATION: &str = "Come back to an angle and zoom worth keeping. Saved views belong to \
                           the model, so they travel with the file.";
const EMPTY: &str = "No view is saved yet. Turn the model to an angle worth keeping, then save it \
                     here or with the Save the current view command.";
const ISOMETRIC_EXPLANATION: &str = "The Isometric view (0) shows the model from the standard \
                                     corner. Make it the current view to change where it looks \
                                     from, how near and at what angle; models open from it.";
const DIALOG_ID: &str = "saved-views";
const HEIGHT_SHARE: f32 = 0.8;
const LIST_HEIGHT: f32 = 240.0;
const SHOW_HOVER: &str = "Show this view";
const UPDATE_HOVER: &str = "Replace it with the current view";
const RENAME_HOVER: &str = "Rename it";
const DELETE_HOVER: &str = "Delete it";

pub fn saved_view(viewpoint: Viewpoint) -> SavedView {
    SavedView {
        target: viewpoint.target,
        orientation: viewpoint.orientation,
        distance: viewpoint.distance,
    }
}

pub fn viewpoint(view: SavedView) -> Viewpoint {
    Viewpoint {
        target: view.target,
        orientation: view.orientation,
        distance: view.distance,
    }
}

fn replacing(
    document: &Document,
    label: String,
    change: impl FnOnce(&mut SavedViews) -> Result<(), String>,
) -> Result<Transaction, String> {
    let mut views = document.saved_views().clone();
    change(&mut views)?;
    let transaction = Transaction::single(
        label,
        Edit::SetSavedViews {
            views: Box::new(views),
        },
    );
    document
        .check(&transaction)
        .map_err(|error| error.to_string())?;
    Ok(transaction)
}

fn missing(name: &str) -> String {
    format!("The view '{name}' no longer exists")
}

pub fn save(document: &Document, name: &str, view: SavedView) -> Result<Transaction, String> {
    let name = view_name(name);
    replacing(document, format!("Save view “{name}”"), |views| {
        views.named.push(NamedView {
            name: name.clone(),
            view,
        });
        Ok(())
    })
}

pub fn update(document: &Document, name: &str, view: SavedView) -> Result<Transaction, String> {
    replacing(document, format!("Update view “{name}”"), |views| {
        let index = views.position(name).ok_or_else(|| missing(name))?;
        if let Some(named) = views.named.get_mut(index) {
            named.view = view;
        }
        Ok(())
    })
}

pub fn rename(document: &Document, name: &str, to: &str) -> Result<Transaction, String> {
    let to = view_name(to);
    replacing(
        document,
        format!("Rename view “{name}” to “{to}”"),
        |views| {
            let index = views.position(name).ok_or_else(|| missing(name))?;
            if let Some(named) = views.named.get_mut(index) {
                named.name = to.clone();
            }
            Ok(())
        },
    )
}

pub fn delete(document: &Document, name: &str) -> Result<Transaction, String> {
    replacing(document, format!("Delete view “{name}”"), |views| {
        let index = views.position(name).ok_or_else(|| missing(name))?;
        views.named.remove(index);
        Ok(())
    })
}

pub fn set_home(document: &Document, view: SavedView) -> Result<Transaction, String> {
    replacing(
        document,
        format!("Change the {HOME_VIEW_NAME} view"),
        |views| {
            views.home = Some(view);
            Ok(())
        },
    )
}

pub fn reset_home(document: &Document) -> Result<Transaction, String> {
    replacing(
        document,
        format!("Reset the {HOME_VIEW_NAME} view"),
        |views| {
            views.home = None;
            Ok(())
        },
    )
}

#[derive(Debug, Clone, PartialEq)]
struct Renaming {
    from: String,
    to: String,
    focus_pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewsDraft {
    name: String,
    renaming: Option<Renaming>,
    problem: Option<String>,
    pub current: SavedView,
    focus_pending: bool,
}

impl ViewsDraft {
    pub fn of(document: &Document, current: SavedView) -> Self {
        Self {
            name: document.saved_views().unused_name(),
            renaming: None,
            problem: None,
            current,
            focus_pending: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Apply(Transaction),
    Show(usize),
}

pub fn name_field_id() -> Id {
    Id::new((DIALOG_ID, "name"))
}

pub fn rename_field_id() -> Id {
    Id::new((DIALOG_ID, "rename"))
}

pub fn dialog(ctx: &egui::Context, document: &Document, draft: &mut ViewsDraft) -> Option<Outcome> {
    let response = widgets::dialog(ctx, DIALOG_ID, TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(EXPLANATION, ui));
        ui.add_space(SPACE_S);
        let mut room = BodyRoom::measure(ui, DIALOG_ID, HEIGHT_SHARE);
        let outcome = ScrollArea::vertical()
            .max_height(room.height)
            .show(ui, |ui| body(ui, document, draft))
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

fn body(ui: &mut Ui, document: &Document, draft: &mut ViewsDraft) -> Option<Outcome> {
    let views = document.saved_views();
    let mut outcome = None;
    widgets::caption(ui, "Views");
    if views.named.is_empty() {
        ui.label(widgets::muted(EMPTY, ui));
    } else {
        ScrollArea::vertical()
            .id_salt((DIALOG_ID, "list"))
            .max_height(widgets::list_height(ui.ctx(), LIST_HEIGHT))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, named) in views.named.iter().enumerate() {
                    if let Some(chosen) = row(ui, document, draft, index, named) {
                        outcome = Some(chosen);
                    }
                }
            });
    }
    ui.add_space(SPACE_M);
    if let Some(saved) = saving(ui, document, draft) {
        outcome = Some(saved);
    }
    ui.add_space(SPACE_M);
    if let Some(changed) = isometric(ui, document, draft) {
        outcome = Some(changed);
    }
    if let Some(problem) = &draft.problem {
        ui.add_space(SPACE_S);
        widgets::callout(ui, Tone::Error, |ui| ui.label(problem));
    }
    outcome
}

fn applying(draft: &mut ViewsDraft, built: Result<Transaction, String>) -> Option<Outcome> {
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

fn row(
    ui: &mut Ui,
    document: &Document,
    draft: &mut ViewsDraft,
    index: usize,
    named: &NamedView,
) -> Option<Outcome> {
    let renaming = draft
        .renaming
        .as_ref()
        .is_some_and(|renaming| renaming.from == named.name);
    if renaming {
        return renaming_row(ui, document, draft);
    }
    let mut outcome = None;
    let mut rename = false;
    let mut delete = false;
    let mut update = false;
    let mut show = false;
    Sides::new().shrink_left().truncate().show(
        ui,
        |ui| {
            ui.label(&named.name);
        },
        |ui| {
            delete = widgets::icon_button(ui, icons::DELETE, DELETE_HOVER).clicked();
            rename = widgets::icon_button(ui, icons::RENAME, RENAME_HOVER).clicked();
            update =
                widgets::icon_button(ui, icons::command(Command::SaveView), UPDATE_HOVER).clicked();
            show = widgets::icon_button(
                ui,
                icons::command(Command::View(StandardView::Isometric)),
                SHOW_HOVER,
            )
            .clicked();
        },
    );
    if show {
        draft.current = named.view;
        outcome = Some(Outcome::Show(index));
    }
    if update {
        let built = self::update(document, &named.name, draft.current);
        outcome = applying(draft, built);
    }
    if delete {
        let built = self::delete(document, &named.name);
        outcome = applying(draft, built);
    }
    if rename {
        draft.renaming = Some(Renaming {
            from: named.name.clone(),
            to: named.name.clone(),
            focus_pending: true,
        });
    }
    outcome
}

fn renaming_row(ui: &mut Ui, document: &Document, draft: &mut ViewsDraft) -> Option<Outcome> {
    let renaming = draft.renaming.as_mut()?;
    let focus = std::mem::take(&mut renaming.focus_pending);
    let mut done = false;
    let mut cancelled = false;
    ui.horizontal(|ui| {
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut renaming.to)
                    .id(rename_field_id())
                    .char_limit(MAX_VIEW_NAME_CHARS)
                    .desired_width(ui.available_width() - 2.0 * ui.spacing().interact_size.y),
            )
        });
        if focus {
            field.request_focus();
        }
        let enter = field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let escape = field.has_focus() && ui.input(|input| input.key_pressed(Key::Escape));
        done = widgets::icon_button(ui, icons::DONE, "Rename the view").clicked() || enter;
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

fn saving(ui: &mut Ui, document: &Document, draft: &mut ViewsDraft) -> Option<Outcome> {
    let focus = std::mem::take(&mut draft.focus_pending);
    let mut save_now = false;
    widgets::properties(ui, (DIALOG_ID, "save"), |ui| {
        widgets::property(ui, NAME_CAPTION, |ui| {
            ui.horizontal(|ui| {
                let field = widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut draft.name)
                            .id(name_field_id())
                            .char_limit(MAX_VIEW_NAME_CHARS)
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
    let built = save(document, &name, draft.current);
    let outcome = applying(draft, built);
    if outcome.is_some() {
        let mut taken = document.saved_views().clone();
        taken.named.push(NamedView {
            name: view_name(&name),
            view: draft.current,
        });
        draft.name = taken.unused_name();
    }
    outcome
}

fn isometric(ui: &mut Ui, document: &Document, draft: &mut ViewsDraft) -> Option<Outcome> {
    let redefined = document.saved_views().home.is_some();
    let mut outcome = None;
    widgets::caption(ui, HOME_VIEW_NAME);
    ui.label(widgets::muted(ISOMETRIC_EXPLANATION, ui));
    ui.horizontal(|ui| {
        let set = ui.add(widgets::button(Command::SetHomeView.title()));
        if set.clicked() {
            let built = set_home(document, draft.current);
            outcome = applying(draft, built);
        }
        let reset = ui.add_enabled(redefined, widgets::button(Command::ResetHomeView.title()));
        if reset.clicked() {
            let built = reset_home(document);
            outcome = applying(draft, built);
        }
    });
    outcome
}
