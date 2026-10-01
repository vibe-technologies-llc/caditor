use egui::{Event, Grid, KeyboardShortcut, Label, ScrollArea, TextEdit, Ui};

use crate::{
    appearance::{self, SPACE_L, SPACE_M, SPACE_S, SPACE_XS},
    commands::{self, Category, Command, Keymap, Scope},
    dialog_parts, icons,
    preferences::{PreferenceChange, PreferencesCommand},
    widgets::{self, DialogWidth, Tone},
};

pub const TITLE: &str = "Keyboard shortcuts";
const RESET_ALL: &str = "Reset all shortcuts";
const FILTER_HINT: &str = "Filter by command or keys";
const BINDINGS_WIDTH: f32 = 170.0;
const ACTIONS_WIDTH: f32 = 180.0;
const SCROLL_BAR_ROOM: f32 = 12.0;
const LIST_HEIGHT: f32 = 420.0;
const CATEGORY_GAP: f32 = SPACE_M;
const ROW_SPACING: [f32; 2] = [SPACE_L, SPACE_S];
const RECORDING_TEXT: &str = "Press the keys… (Esc cancels)";
const RESERVED_TEXT: &str = "Esc, Enter and Tab keep their meaning everywhere (back out, confirm, move between fields), so \
     they cannot be shortcuts.";
const CATEGORIES: [Category; 7] = [
    Category::File,
    Category::Edit,
    Category::View,
    Category::Model,
    Category::Sketch,
    Category::Constraint,
    Category::Help,
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    command: Command,
    shortcut: KeyboardShortcut,
    holders: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEditor {
    query: String,
    recording: Option<Command>,
    pending: Option<Pending>,
    message: Option<String>,
    focus_filter: bool,
    confirm_reset: bool,
}

impl Default for ShortcutEditor {
    fn default() -> Self {
        Self {
            query: String::new(),
            recording: None,
            pending: None,
            message: None,
            focus_filter: true,
            confirm_reset: false,
        }
    }
}

impl ShortcutEditor {
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    fn capture(&mut self, ctx: &egui::Context, keymap: &Keymap) -> Option<PreferencesCommand> {
        let command = self.recording?;
        let pressed = ctx.input_mut(|input| {
            let pressed = input.events.iter().find_map(commands::pressed_shortcut);
            input
                .events
                .retain(|event| !matches!(event, Event::Key { .. } | Event::Text(_)));
            pressed
        })?;
        self.recording = None;
        if pressed.logical_key == egui::Key::Escape {
            return None;
        }
        if commands::is_reserved(pressed.logical_key) {
            self.message = Some(RESERVED_TEXT.to_owned());
            return None;
        }
        let holders = keymap.conflicts(command, &pressed);
        if holders.is_empty() {
            self.message = None;
            return Some(bind(command, pressed));
        }
        self.pending = Some(Pending {
            command,
            shortcut: pressed,
            holders,
        });
        None
    }
}

fn bind(command: Command, shortcut: KeyboardShortcut) -> PreferencesCommand {
    PreferencesCommand::Change(PreferenceChange::Bind(command, shortcut))
}

pub fn dialog(
    ctx: &egui::Context,
    editor: &mut ShortcutEditor,
    keymap: &Keymap,
    reset_undoable: bool,
) -> Option<PreferencesCommand> {
    let captured = editor.capture(ctx, keymap);
    let response = widgets::dialog(ctx, "keyboard-shortcuts", TITLE, DialogWidth::Wide, |ui| {
        ui.label(widgets::muted(
            "Add… records the next keys you press, and the cross beside a shortcut removes it. \
             Sketch and constraint shortcuts only act while a sketch is edited.",
            ui,
        ));
        let mut command = None;
        conflict(ui, editor, &mut command);
        if let Some(message) = &editor.message {
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.label(message);
            });
        }
        let filter = ui
            .horizontal(|ui| {
                let muted = appearance::tokens(ui).text_muted;
                widgets::icon_label(ui, icons::SEARCH, muted);
                widgets::text_field(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut editor.query)
                            .hint_text(FILTER_HINT)
                            .desired_width(f32::INFINITY),
                    )
                })
            })
            .inner;
        if std::mem::take(&mut editor.focus_filter) {
            filter.request_focus();
        }
        let columns = Columns::of(ui);
        header(ui, columns);
        let height = widgets::list_height(ui.ctx(), LIST_HEIGHT);
        ScrollArea::vertical()
            .max_height(height)
            .min_scrolled_height(height)
            .show(ui, |ui| {
                ui.set_min_height(height);
                list(ui, editor, keymap, columns, &mut command);
            });
        if reset_undoable
            && dialog_parts::undo_note(
                ui,
                "Every shortcut is back to the one caditor starts with.",
                "Put back the shortcuts as they were before",
            )
        {
            command = Some(PreferencesCommand::Undo);
        }
        if editor.confirm_reset {
            ui.add_space(SPACE_M);
            dialog_parts::confirmation(
                ui,
                "Reset all shortcuts?",
                "Every command goes back to the shortcuts caditor starts with; the ones you \
                 added, removed or moved are lost.",
            );
            if let Some(confirmed) = dialog_parts::confirm_footer(ui, RESET_ALL, "Cancel") {
                editor.confirm_reset = false;
                if confirmed {
                    command = Some(PreferencesCommand::Change(PreferenceChange::ResetShortcuts));
                }
            }
        } else {
            let all_default = keymap.is_all_default();
            let chosen = widgets::footer_split(
                ui,
                |ui| {
                    ui.add_enabled(!all_default, widgets::danger_button(RESET_ALL))
                        .on_hover_text("Go back to the shortcuts caditor starts with")
                        .on_disabled_hover_text(
                            "Every shortcut is already the one caditor starts with",
                        )
                        .clicked()
                        .then_some(false)
                },
                |ui| {
                    ui.add(widgets::primary_button(ui, "Close"))
                        .clicked()
                        .then_some(true)
                },
            );
            match chosen {
                Some(true) => command = Some(PreferencesCommand::HideShortcuts),
                Some(false) => editor.confirm_reset = true,
                None => {}
            }
        }
        command
    });
    let closed = (!editor.is_recording() && response.should_close())
        .then_some(PreferencesCommand::HideShortcuts);
    captured.or(response.inner).or(closed)
}

#[derive(Debug, Clone, Copy)]
struct Columns {
    command: f32,
    shortcuts: f32,
    actions: f32,
}

impl Columns {
    fn of(ui: &Ui) -> Self {
        let spacing = 2.0 * ROW_SPACING[0] + SCROLL_BAR_ROOM;
        let room = ui.available_width() - spacing;
        let actions = ACTIONS_WIDTH.min(room / 3.0);
        let shortcuts = BINDINGS_WIDTH.min(room / 3.0);
        Self {
            command: (room - actions - shortcuts).max(0.0),
            shortcuts,
            actions,
        }
    }
}

fn header(ui: &mut Ui, columns: Columns) {
    Grid::new("shortcut-columns")
        .num_columns(3)
        .spacing(ROW_SPACING)
        .show(ui, |ui| {
            for (text, width) in [
                ("Command", columns.command),
                ("Shortcuts", columns.shortcuts),
                ("", columns.actions),
            ] {
                ui.scope(|ui| {
                    ui.set_width(width);
                    widgets::column_caption(ui, text);
                });
            }
            ui.end_row();
        });
}

fn conflict(ui: &mut Ui, editor: &mut ShortcutEditor, command: &mut Option<PreferencesCommand>) {
    let Some(pending) = editor.pending.clone() else {
        return;
    };
    let keys = commands::display(&pending.shortcut);
    let holders: Vec<String> = pending
        .holders
        .iter()
        .map(|holder| holder.title())
        .collect();
    widgets::callout(ui, Tone::Warning, |ui| {
        ui.label(format!(
            "{keys} is already used by {}. Use it for {} instead?",
            holders.join(" and "),
            pending.command.title()
        ));
        ui.horizontal(|ui| {
            if ui
                .add(widgets::button(format!("Move {keys} here")))
                .clicked()
            {
                *command = Some(bind(pending.command, pending.shortcut));
                editor.pending = None;
            }
            if ui.add(widgets::button("Keep it where it is")).clicked() {
                editor.pending = None;
            }
        });
    });
}

fn list(
    ui: &mut Ui,
    editor: &mut ShortcutEditor,
    keymap: &Keymap,
    columns: Columns,
    command: &mut Option<PreferencesCommand>,
) {
    let query = editor.query.trim().to_lowercase();
    let mut shown_any = false;
    for category in CATEGORIES {
        let shown: Vec<Command> = Command::all()
            .filter(|listed| listed.category() == category)
            .filter(|listed| {
                query.is_empty()
                    || listed.title().to_lowercase().contains(&query)
                    || category.label().to_lowercase().contains(&query)
                    || keymap
                        .shortcuts(*listed)
                        .iter()
                        .any(|shortcut| commands::is_named_by(shortcut, &query))
            })
            .collect();
        let Some(first) = shown.first() else {
            continue;
        };
        let heading = match first.scope() {
            Scope::Anywhere => category.label().to_owned(),
            Scope::Sketch => format!("{} ({})", category.label(), Scope::Sketch.describe()),
        };
        if shown_any {
            ui.add_space(CATEGORY_GAP);
        }
        shown_any = true;
        ui.label(widgets::section_title(&heading));
        Grid::new(("shortcuts", category.label()))
            .num_columns(3)
            .spacing(ROW_SPACING)
            .striped(true)
            .show(ui, |ui| {
                for listed in shown {
                    row(ui, editor, keymap, listed, columns, command);
                    ui.end_row();
                }
            });
    }
    if !shown_any {
        let text = format!("No command or shortcut matches “{}”.", editor.query.trim());
        let clear = widgets::empty_state(ui, icons::SEARCH, &text, |ui| {
            ui.add(widgets::button("Clear the filter")).clicked()
        });
        if clear {
            editor.query.clear();
            editor.focus_filter = true;
        }
    }
}

fn row(
    ui: &mut Ui,
    editor: &mut ShortcutEditor,
    keymap: &Keymap,
    listed: Command,
    columns: Columns,
    command: &mut Option<PreferencesCommand>,
) {
    ui.horizontal(|ui| {
        ui.set_width(columns.command);
        let muted = appearance::tokens(ui).text_muted;
        widgets::icon_label(ui, icons::command(listed), muted);
        ui.add(Label::new(listed.title()).truncate());
    });
    ui.horizontal_wrapped(|ui| {
        ui.set_width(columns.shortcuts);
        let shortcuts = keymap.shortcuts(listed);
        if shortcuts.is_empty() {
            ui.label(widgets::muted("None", ui));
        }
        for shortcut in shortcuts {
            let keys = commands::display(&shortcut);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                widgets::key_cap(ui, &keys);
                let name = format!("Remove {keys} from {}", listed.title());
                if widgets::icon_button(ui, icons::REMOVE, &name).clicked() {
                    *command = Some(PreferencesCommand::Change(PreferenceChange::Unbind(
                        listed, shortcut,
                    )));
                }
            });
        }
    });
    ui.horizontal(|ui| {
        ui.set_width(columns.actions);
        if editor.recording == Some(listed) {
            widgets::pill(ui, Tone::Info, RECORDING_TEXT);
            return;
        }
        let add_name = format!("Record a new shortcut for {}", listed.title());
        let add = ui
            .add(widgets::Named::new(
                widgets::button("Add…"),
                add_name.clone(),
            ))
            .on_hover_text(add_name);
        if add.clicked() {
            editor.recording = Some(listed);
            editor.pending = None;
            editor.message = None;
            editor.confirm_reset = false;
        }
        let reset_name = format!(
            "Reset {} to the shortcut caditor starts with",
            listed.title()
        );
        let reset = ui
            .add_enabled(
                !keymap.is_default(listed),
                widgets::Named::new(widgets::button("Reset"), reset_name.clone()),
            )
            .on_hover_text(reset_name);
        if reset.clicked() {
            *command = Some(PreferencesCommand::Change(PreferenceChange::ResetShortcut(
                listed,
            )));
        }
    });
}
