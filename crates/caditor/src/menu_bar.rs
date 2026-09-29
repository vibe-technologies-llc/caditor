use egui::{Align, Button, Id, Label, Layout, RichText, TextStyle, Ui, vec2};

use crate::{
    appearance,
    commands::{CameraMove, Command, CommandFrame, Offer, Scope, StandardView},
    editing::{SketchEditing, Tool},
    files::{self, Files},
    icons,
    model::{Action, Model},
    sketch_tools::ConstraintTool,
    widgets::{self, Tone},
};

pub const SEARCH_LABEL: &str = "Search commands";
const SEARCH_WIDTH: f32 = 240.0;
const NAME_ROOM: f32 = 160.0;
const SKETCH_ONLY: &str = "Only while a sketch is being edited";
const NOT_HERE: &str = "Not available right now";

pub struct MenuContext<'a> {
    pub files: &'a Files,
    pub editing: &'a SketchEditing,
    pub offers: &'a [Offer],
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    context: &MenuContext<'_>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let trailing_id = Id::new("menu-bar-trailing");
    egui::Panel::top("menu-bar")
        .show_separator_line(false)
        .show(ui, |ui| {
            let mut wrapped = false;
            egui::MenuBar::new().ui(ui, |ui| {
                files::menu(ui, model, context.files, context.editing, commands, actions);
                let mut menus = Menus {
                    offers: context.offers,
                    commands,
                    chosen: Vec::new(),
                };
                menus.edit(ui);
                menus.view(ui);
                menus.model(ui);
                menus.sketch(ui);
                menus.help(ui);
                let chosen = std::mem::take(&mut menus.chosen);
                for command in chosen {
                    commands.trigger(command);
                }
                if ui.available_width() >= widgets::remembered_width(ui, trailing_id) {
                    trailing(ui, model, commands, trailing_id);
                } else {
                    wrapped = true;
                }
            });
            if wrapped {
                trailing(ui, model, commands, trailing_id);
            }
        });
}

fn trailing(ui: &mut Ui, model: &Model, commands: &mut CommandFrame<'_>, id: Id) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        search(ui, commands);
        widgets::remember_width(ui, id, ui.min_rect().width() + NAME_ROOM);
        document_name(ui, model);
    });
}

struct Menus<'a, 'b> {
    offers: &'a [Offer],
    commands: &'a CommandFrame<'b>,
    chosen: Vec<Command>,
}

impl Menus<'_, '_> {
    fn availability(&self, command: Command) -> Result<(), String> {
        match self.offers.iter().find(|offer| offer.command == command) {
            Some(offer) => offer.availability.clone(),
            None if command.scope() == Scope::Sketch => Err(SKETCH_ONLY.to_owned()),
            None => Err(NOT_HERE.to_owned()),
        }
    }

    fn item(&mut self, ui: &mut Ui, command: Command) {
        let availability = self.availability(command);
        let response = ui.add_enabled_ui(availability.is_ok(), |ui| {
            widgets::menu_item(
                ui,
                icons::command(command),
                &command.title(),
                self.commands.keys(command),
            )
        });
        let response = match &availability {
            Ok(()) => response.inner,
            Err(reason) => response.inner.on_disabled_hover_text(reason),
        };
        if response.clicked() {
            self.chosen.push(command);
        }
    }

    fn items(&mut self, ui: &mut Ui, commands: impl IntoIterator<Item = Command>) {
        for command in commands {
            self.item(ui, command);
        }
    }

    fn edit(&mut self, ui: &mut Ui) {
        ui.menu_button("Edit", |ui| {
            self.items(ui, [Command::Undo, Command::Redo]);
            ui.separator();
            self.item(ui, Command::DeleteSelection);
            ui.separator();
            self.item(ui, Command::Palette);
        });
    }

    fn view(&mut self, ui: &mut Ui) {
        ui.menu_button("View", |ui| {
            self.item(ui, Command::FitView);
            self.items(
                ui,
                [
                    Command::HideSelection,
                    Command::ToggleVisibility,
                    Command::ShowAll,
                ],
            );
            submenu(
                ui,
                icons::command(Command::View(StandardView::Front)),
                "Standard views",
                |ui| {
                    self.items(ui, StandardView::ALL.map(Command::View));
                },
            );
            submenu(
                ui,
                icons::command(Command::Camera(CameraMove::OrbitLeft)),
                "Camera",
                |ui| {
                    self.items(ui, CameraMove::ALL.map(Command::Camera));
                },
            );
            ui.separator();
            self.items(
                ui,
                [
                    Command::HighlightNext,
                    Command::HighlightPrevious,
                    Command::ActivateHighlighted,
                ],
            );
            ui.separator();
            self.items(
                ui,
                [
                    Command::LargerInterface,
                    Command::SmallerInterface,
                    Command::NormalInterface,
                ],
            );
        });
    }

    fn model(&mut self, ui: &mut Ui) {
        ui.menu_button("Model", |ui| {
            self.item(ui, Command::NewSketch);
            ui.separator();
            self.items(ui, [Command::Extrude, Command::Revolve]);
            ui.separator();
            self.items(ui, [Command::Fillet, Command::Chamfer, Command::Shell]);
            ui.separator();
            self.items(ui, [Command::DatumPlane, Command::DatumAxis]);
            ui.separator();
            self.items(
                ui,
                [
                    Command::EditFeature,
                    Command::CloseFeature,
                    Command::RenameFeature,
                    Command::MoveFeatureUp,
                    Command::MoveFeatureDown,
                    Command::DeleteFeature,
                ],
            );
            ui.separator();
            self.items(ui, [Command::AddParameter, Command::DeleteParameter]);
            ui.separator();
            self.items(
                ui,
                [
                    Command::Recompute,
                    Command::CancelRecompute,
                    Command::ShowFirstFailed,
                ],
            );
        });
    }

    fn sketch(&mut self, ui: &mut Ui) {
        ui.menu_button("Sketch", |ui| {
            self.item(ui, Command::FinishSketch);
            ui.separator();
            self.items(ui, Tool::ALL.map(Command::SketchTool));
            self.item(ui, Command::ReverseArc);
            ui.separator();
            submenu(
                ui,
                icons::constraint(ConstraintTool::Coincident),
                "Constraints",
                |ui| self.items(ui, ConstraintTool::ALL.map(Command::Constraint)),
            );
        });
    }

    fn help(&mut self, ui: &mut Ui) {
        ui.menu_button("Help", |ui| {
            self.items(
                ui,
                [
                    Command::Welcome,
                    Command::Palette,
                    Command::KeyboardShortcuts,
                    Command::About,
                ],
            );
        });
    }
}

fn submenu(ui: &mut Ui, glyph: &str, title: &str, add: impl FnOnce(&mut Ui)) {
    let muted = appearance::tokens(ui).text_muted;
    let submenu = ui.menu_button((widgets::icon(glyph).color(muted), title.to_owned()), add);
    widgets::named(submenu.response, title);
}

fn search(ui: &mut Ui, commands: &mut CommandFrame<'_>) {
    let tokens = appearance::tokens(ui);
    let mut button = Button::new((
        widgets::icon(icons::SEARCH).color(tokens.text_muted),
        RichText::new(SEARCH_LABEL).color(tokens.text_muted),
    ))
    .fill(tokens.sunken)
    .stroke(egui::Stroke::new(1.0, tokens.border))
    .min_size(vec2(SEARCH_WIDTH, 0.0));
    if let Some(keys) = commands.keys(Command::Palette) {
        button = button.shortcut_text(RichText::new(keys).text_style(TextStyle::Small));
    }
    let hover = commands.with_keys(Command::Palette, "Search every command by name");
    if ui
        .add(widgets::Named::new(button, SEARCH_LABEL))
        .on_hover_text(hover)
        .clicked()
    {
        commands.trigger(Command::Palette);
    }
}

fn document_name(ui: &mut Ui, model: &Model) {
    if model.is_dirty() {
        widgets::pill(ui, Tone::Neutral, "Unsaved");
    }
    let name = model.display_name();
    let shown = ui.add(Label::new(&name).truncate());
    if let Some(path) = model.path() {
        shown.on_hover_text(path.display().to_string());
    }
}
