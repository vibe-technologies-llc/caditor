use egui::{Event, Grid, KeyboardShortcut, RichText, ScrollArea, TextEdit, Ui};

use crate::{
    appearance,
    commands::{self, Category, Command, Keymap, Scope},
    icons,
    preferences::{PreferenceChange, PreferencesCommand},
    widgets::{self, DialogWidth, Tone},
};

const BINDINGS_WIDTH: f32 = 180.0;
const LIST_HEIGHT: f32 = 420.0;
const CATEGORY_GAP: f32 = 10.0;
const ROW_SPACING: [f32; 2] = [12.0, 4.0];
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
}

impl Default for ShortcutEditor {
    fn default() -> Self {
        Self {
            query: String::new(),
            recording: None,
            pending: None,
            message: None,
            focus_filter: true,
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
) -> Option<PreferencesCommand> {
    let captured = editor.capture(ctx, keymap);
    let response = widgets::dialog(
        ctx,
        "keyboard-shortcuts",
        "Keyboard Shortcuts",
        DialogWidth::Wide,
        |ui| {
            ui.label(widgets::muted(
                "Add records the next keys you press, and clicking a shortcut removes it. Sketch \
                 and constraint shortcuts only act while a sketch is edited.",
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
                    ui.add(
                        TextEdit::singleline(&mut editor.query)
                            .hint_text("Filter commands")
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner;
            if std::mem::take(&mut editor.focus_filter) {
                filter.request_focus();
            }
            ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .min_scrolled_height(LIST_HEIGHT)
                .show(ui, |ui| list(ui, editor, keymap, &mut command));
            widgets::footer(ui, |ui| {
                if ui.add(widgets::primary_button(ui, "Close")).clicked() {
                    command = Some(PreferencesCommand::HideShortcuts);
                }
                let reset = ui
                    .add_enabled(
                        !keymap.is_all_default(),
                        egui::Button::new("Reset all shortcuts"),
                    )
                    .on_hover_text("Go back to the shortcuts caditor starts with");
                if reset.clicked() {
                    command = Some(PreferencesCommand::Change(PreferenceChange::ResetShortcuts));
                }
            });
            command
        },
    );
    let closed = (!editor.is_recording() && response.should_close())
        .then_some(PreferencesCommand::HideShortcuts);
    captured.or(response.inner).or(closed)
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
            if ui.button(format!("Move {keys} here")).clicked() {
                *command = Some(bind(pending.command, pending.shortcut));
                editor.pending = None;
            }
            if ui.button("Keep it where it is").clicked() {
                editor.pending = None;
            }
        });
    });
}

fn list(
    ui: &mut Ui,
    editor: &mut ShortcutEditor,
    keymap: &Keymap,
    command: &mut Option<PreferencesCommand>,
) {
    let query = editor.query.trim().to_lowercase();
    for category in CATEGORIES {
        let shown: Vec<Command> = Command::all()
            .filter(|listed| listed.category() == category)
            .filter(|listed| {
                query.is_empty()
                    || listed.title().to_lowercase().contains(&query)
                    || category.label().to_lowercase().contains(&query)
            })
            .collect();
        let Some(first) = shown.first() else {
            continue;
        };
        let heading = match first.scope() {
            Scope::Anywhere => category.label().to_owned(),
            Scope::Sketch => format!("{} ({})", category.label(), Scope::Sketch.describe()),
        };
        ui.add_space(CATEGORY_GAP);
        ui.label(widgets::section_title(&heading));
        Grid::new(("shortcuts", category.label()))
            .num_columns(3)
            .spacing(ROW_SPACING)
            .striped(true)
            .show(ui, |ui| {
                for listed in shown {
                    row(ui, editor, keymap, listed, command);
                    ui.end_row();
                }
            });
    }
}

fn row(
    ui: &mut Ui,
    editor: &mut ShortcutEditor,
    keymap: &Keymap,
    listed: Command,
    command: &mut Option<PreferencesCommand>,
) {
    ui.horizontal(|ui| {
        let muted = appearance::tokens(ui).text_muted;
        widgets::icon_label(ui, icons::command(listed), muted);
        ui.label(listed.title());
    });
    ui.horizontal_wrapped(|ui| {
        ui.set_min_width(BINDINGS_WIDTH);
        let shortcuts = keymap.shortcuts(listed);
        if shortcuts.is_empty() {
            ui.label(widgets::muted("None", ui));
        }
        for shortcut in shortcuts {
            let keys = commands::display(&shortcut);
            let muted = appearance::tokens(ui).text_muted;
            let chip = egui::Button::new((
                RichText::new(keys.clone()).text_style(egui::TextStyle::Small),
                widgets::icon(icons::REMOVE).color(muted),
            ));
            let removed = ui
                .add(chip)
                .on_hover_text(format!("Remove {keys} from {}", listed.title()))
                .clicked();
            if removed {
                *command = Some(PreferencesCommand::Change(PreferenceChange::Unbind(
                    listed, shortcut,
                )));
            }
        }
    });
    ui.horizontal(|ui| {
        if editor.recording == Some(listed) {
            widgets::pill(ui, Tone::Warning, RECORDING_TEXT);
            return;
        }
        let add = ui
            .button("Add…")
            .on_hover_text(format!("Record a new shortcut for {}", listed.title()));
        if add.clicked() {
            editor.recording = Some(listed);
            editor.pending = None;
            editor.message = None;
        }
        let reset = ui
            .add_enabled(!keymap.is_default(listed), egui::Button::new("Reset"))
            .on_hover_text("Go back to the shortcut caditor starts with");
        if reset.clicked() {
            *command = Some(PreferencesCommand::Change(PreferenceChange::ResetShortcut(
                listed,
            )));
        }
    });
}
