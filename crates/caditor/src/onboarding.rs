use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use caditor_document::FeatureKind;
use caditor_file::Settings;
use egui::{
    Align, Align2, Area, CornerRadius, Id, Label, Layout, Order, Rect, RichText, Sense, Sides,
    Stroke, StrokeKind, TextWrapMode, Ui, UiBuilder, vec2,
};

use crate::{
    appearance::{self, BORDER_WIDTH, CARD_RADIUS, SPACE_M, SPACE_S, WIDGET_RADIUS},
    commands::{self, Command, Keymap, Offer},
    dialog_parts::BodyRoom,
    editing::{SketchEditing, Tool},
    fonts, icons,
    model::{Model, display_name},
    preferences::InputMode,
    samples::Sample,
    sketch_status::{SketchStatus, SketchSummary},
    widgets::{self, DialogWidth},
};

const WELCOMED_KEY: &str = "onboarding.welcomed";
const HINTS_KEY: &str = "onboarding.hints";
const DISMISSED_KEY: &str = "onboarding.dismissed_hints";
const TIP_TITLE: &str = "Tip";
const DISMISS_TIP: &str = "Dismiss the tip";
const RECENT_TITLE: &str = "Recent files";
const RECENT_SHOWN: usize = 5;
const HINT_WIDTH: f32 = 360.0;
const HINT_BOTTOM_CLEARANCE: f32 = 16.0;
const SAMPLE_ICON_SIZE: f32 = 24.0;
const WELCOME_HEIGHT_SHARE: f32 = 0.9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hint {
    Start,
    Draw,
    Constrain,
    Sweep,
    Navigate,
    Palette,
}

impl Hint {
    const ALL: [Self; 6] = [
        Self::Start,
        Self::Draw,
        Self::Constrain,
        Self::Sweep,
        Self::Navigate,
        Self::Palette,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Draw => "draw",
            Self::Constrain => "constrain",
            Self::Sweep => "sweep",
            Self::Navigate => "navigate",
            Self::Palette => "palette",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|hint| hint.id() == id)
    }

    pub fn text(self, keymap: &Keymap, input_mode: InputMode) -> String {
        let keys = |command: Command, fallback: &str| {
            keymap.first(command).map_or_else(
                || fallback.to_owned(),
                |shortcut| commands::display(&shortcut),
            )
        };
        match self {
            Self::Start => "Start with New sketch: pick a plane, draw a closed outline and \
                            Extrude it into a body. Or open a sample from File › Open sample to \
                            see a finished model."
                .to_owned(),
            Self::Draw => format!(
                "Pick a drawing tool in the sketch ribbon ({} for lines, {} for rectangles, {} for circles) and \
                 click in the view. Typing x, y places a point exactly.",
                keys(Command::SketchTool(Tool::Line), "Line"),
                keys(Command::SketchTool(Tool::Rectangle), "Rectangle"),
                keys(Command::SketchTool(Tool::Circle), "Circle"),
            ),
            Self::Constrain => "Select geometry and add constraints and dimensions from the \
                                sketch ribbon. The counter says how much can still move, and dragging \
                                geometry shows what; fully constrained geometry turns green. \
                                Nothing is lost if you stop here."
                .to_owned(),
            Self::Sweep => "The sketch has a closed outline: Extrude or Revolve it to make a \
                            body. Its size stays editable in the feature's panel."
                .to_owned(),
            Self::Navigate => format!(
                "{} Double-click a face to open the feature that made it, and change any value \
                 in the feature tree or in Parameters.",
                input_mode.navigation_tip()
            ),
            Self::Palette => format!(
                "Press {} to find any command by name, including ones without a button.",
                keys(Command::Palette, "the Search commands button")
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    pub welcomed: bool,
    pub hints: bool,
    pub dismissed: BTreeSet<Hint>,
}

impl Default for Onboarding {
    fn default() -> Self {
        Self {
            welcomed: false,
            hints: true,
            dismissed: BTreeSet::new(),
        }
    }
}

impl Onboarding {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            welcomed: settings.flag(WELCOMED_KEY).unwrap_or(false),
            hints: settings.flag(HINTS_KEY).unwrap_or(true),
            dismissed: settings
                .texts(DISMISSED_KEY)
                .unwrap_or_default()
                .iter()
                .filter_map(|id| Hint::from_id(id))
                .collect(),
        }
    }

    pub fn write(&self, settings: &mut Settings) {
        settings.set_flag(WELCOMED_KEY, self.welcomed);
        settings.set_flag(HINTS_KEY, self.hints);
        let known: Vec<String> = self
            .dismissed
            .iter()
            .map(|hint| hint.id().to_owned())
            .collect();
        let unknown = settings
            .texts(DISMISSED_KEY)
            .unwrap_or_default()
            .into_iter()
            .filter(|id| Hint::from_id(id).is_none());
        let all: Vec<String> = known.into_iter().chain(unknown).collect();
        settings.set_texts(DISMISSED_KEY, &all);
    }

    #[cfg(test)]
    pub fn finished() -> Self {
        Self {
            welcomed: true,
            hints: false,
            dismissed: BTreeSet::new(),
        }
    }
}

pub struct Situation<'a> {
    pub model: &'a Model,
    pub editing: &'a SketchEditing,
    pub offers: &'a [Offer],
}

pub fn applicable(situation: &Situation<'_>) -> Vec<Hint> {
    let document = situation.model.document();
    let has_body = document
        .features()
        .any(|feature| matches!(feature.kind, FeatureKind::Solid(_) | FeatureKind::Import(_)));
    let mut hints = Vec::new();
    if let Some(feature) = situation.editing.feature() {
        let drawn = document
            .feature(feature)
            .and_then(|feature| feature.kind.sketch())
            .is_some_and(|sketch| sketch.entities().len() > 0);
        if !drawn {
            hints.push(Hint::Draw);
        } else if matches!(
            SketchSummary::of(situation.model.evaluation(), feature).status,
            SketchStatus::Free(_)
        ) {
            hints.push(Hint::Constrain);
        }
    } else if !situation.editing.is_choosing_plane() && situation.editing.solid().is_none() {
        if document.features().len() == 0 {
            hints.push(Hint::Start);
        }
        let can_sweep = situation
            .offers
            .iter()
            .any(|offer| offer.command == Command::Extrude && offer.availability.is_ok());
        if can_sweep && !has_body {
            hints.push(Hint::Sweep);
        }
        if has_body {
            hints.push(Hint::Navigate);
        }
        if document.features().len() > 0 {
            hints.push(Hint::Palette);
        }
    }
    hints
}

pub fn current(onboarding: &Onboarding, situation: &Situation<'_>) -> Option<Hint> {
    if !onboarding.hints {
        return None;
    }
    applicable(situation)
        .into_iter()
        .find(|hint| !onboarding.dismissed.contains(hint))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintChoice {
    Dismiss(Hint),
    HideAll,
}

pub fn show_hint(
    ctx: &egui::Context,
    viewport: Rect,
    hint: Hint,
    keymap: &Keymap,
    input_mode: InputMode,
) -> Option<HintChoice> {
    let anchor = viewport.center_bottom() - vec2(0.0, HINT_BOTTOM_CLEARANCE);
    Area::new(Id::new("onboarding-hint"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            widgets::dialog_frame(ctx)
                .show(ui, |ui| {
                    ui.set_width(widgets::fitting_width(ctx, HINT_WIDTH));
                    let closed = Sides::new()
                        .show(
                            ui,
                            |ui| {
                                let accent = appearance::tokens(ui).accent_text;
                                widgets::icon_label(ui, icons::TIP, accent);
                                ui.label(widgets::section_title(TIP_TITLE));
                            },
                            |ui| widgets::icon_button(ui, icons::CLOSE, DISMISS_TIP).clicked(),
                        )
                        .1;
                    ui.add(Label::new(hint.text(keymap, input_mode)).wrap());
                    let chosen = widgets::footer_split(
                        ui,
                        |ui| {
                            ui.add(widgets::button("Hide tips"))
                                .on_hover_text("Turn tips back on in Preferences › General")
                                .clicked()
                                .then_some(HintChoice::HideAll)
                        },
                        |ui| {
                            ui.add(widgets::primary_button(ui, "Got it"))
                                .clicked()
                                .then_some(HintChoice::Dismiss(hint))
                        },
                    );
                    chosen.or(closed.then_some(HintChoice::Dismiss(hint)))
                })
                .inner
        })
        .inner
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WelcomeChoice {
    Close,
    Empty,
    Sample(Sample),
    Open,
    OpenRecent(PathBuf),
    ClearRecent,
}

pub fn welcome(ctx: &egui::Context, keymap: &Keymap, recent: &[PathBuf]) -> Option<WelcomeChoice> {
    let response = widgets::dialog(
        ctx,
        "welcome",
        "Welcome to caditor",
        DialogWidth::Medium,
        |ui| {
            ui.label(widgets::muted(
                "Models are built from sketches and features that stay editable: change a value \
                 or an early sketch and everything after it follows. Every change can be undone, \
                 and unsaved work survives a crash.",
                ui,
            ));
            let mut room = BodyRoom::measure(ui, "welcome", WELCOME_HEIGHT_SHARE);
            let mut choice = egui::ScrollArea::vertical()
                .max_height(room.height)
                .show(ui, |ui| {
                    let mut choice = None;
                    if !recent.is_empty() {
                        ui.add_space(SPACE_M);
                        choice = recent_files(ui, recent);
                    }
                    ui.add_space(SPACE_M);
                    ui.label(widgets::section_title(
                        "Open a sample to see how a model is built",
                    ));
                    for sample in Sample::ALL {
                        if sample_card(ui, sample) {
                            choice = Some(WelcomeChoice::Sample(sample));
                        }
                    }
                    choice
                })
                .inner;
            room.body_ended(ui);
            let palette = keymap.first(Command::Palette).map_or_else(
                || Command::Palette.title(),
                |shortcut| commands::display(&shortcut),
            );
            ui.add_space(SPACE_M);
            ui.label(widgets::muted(
                format!(
                    "{palette} finds any command by name. Tips in the view suggest the next step; \
                     Help › Welcome and samples brings this back."
                ),
                ui,
            ));
            widgets::footer(ui, |ui| {
                if ui
                    .add(widgets::primary_button(ui, "Start with an empty model"))
                    .clicked()
                {
                    choice = Some(WelcomeChoice::Empty);
                }
                let open = widgets::small_button(
                    ui,
                    icons::command(Command::Open),
                    &Command::Open.title(),
                );
                if ui
                    .add(open)
                    .on_hover_text("Choose a model file to open")
                    .clicked()
                {
                    choice = Some(WelcomeChoice::Open);
                }
            });
            room.dialog_ended(ui);
            choice
        },
    );
    let closed = response.should_close().then_some(WelcomeChoice::Close);
    response.inner.or(closed)
}

fn recent_files(ui: &mut Ui, recent: &[PathBuf]) -> Option<WelcomeChoice> {
    let mut choice = None;
    Sides::new().show(
        ui,
        |ui| ui.label(widgets::section_title(RECENT_TITLE)),
        |ui| {
            let clear = widgets::small_button(
                ui,
                icons::command(Command::ClearRecent),
                &Command::ClearRecent.title(),
            );
            if ui
                .add(clear)
                .on_hover_text("Forget these files; the files themselves stay where they are")
                .clicked()
            {
                choice = Some(WelcomeChoice::ClearRecent);
            }
        },
    );
    widgets::card(ui, |ui| {
        for path in recent.iter().take(RECENT_SHOWN) {
            if recent_row(ui, path) {
                choice = Some(WelcomeChoice::OpenRecent(path.clone()));
            }
        }
    });
    choice
}

fn recent_row(ui: &mut Ui, path: &Path) -> bool {
    let tokens = appearance::tokens(ui);
    let name = display_name(Some(path));
    let folder = folder_name(path);
    let width = ui.available_width();
    let height = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(WIDGET_RADIUS), tokens.hover);
    }
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(SPACE_S, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
    widgets::icon_label(&mut child, icons::FILE, tokens.text_muted);
    child.add(Label::new(name.as_str()).selectable(false));
    child.add(Label::new(widgets::muted(folder, ui)).selectable(false));
    let response = widgets::named(response, &format!("Open {name}"))
        .on_hover_text(path.display().to_string())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.has_focus() {
        widgets::paint_focus_ring(ui, rect);
    }
    response.clicked()
}

pub fn folder_name(path: &Path) -> String {
    path.parent()
        .map(|folder| match std::env::home_dir() {
            Some(home) if folder == home => "~".to_owned(),
            Some(home) => folder.strip_prefix(&home).map_or_else(
                |_| folder.display().to_string(),
                |inside| format!("~/{}", inside.display()),
            ),
            None => folder.display().to_string(),
        })
        .unwrap_or_default()
}

fn sample_icon(sample: Sample) -> &'static str {
    match sample {
        Sample::Plate => icons::command(Command::DatumPlane),
        Sample::Spool => icons::command(Command::Revolve),
        Sample::Bracket => icons::command(Command::Extrude),
    }
}

fn sample_card(ui: &mut Ui, sample: Sample) -> bool {
    let tokens = appearance::tokens(ui);
    let card = ui.scope(|ui| {
        widgets::card(ui, |ui| {
            ui.horizontal_top(|ui| {
                let glyph = ui.add(
                    Label::new(
                        RichText::new(sample_icon(sample))
                            .font(egui::FontId::new(SAMPLE_ICON_SIZE, fonts::icons()))
                            .color(tokens.accent_text),
                    )
                    .selectable(false),
                );
                widgets::decorative(ui, &glyph);
                ui.vertical(|ui| {
                    ui.label(widgets::strong(sample.title()));
                    ui.label(widgets::muted(sample.description(), ui));
                });
            });
        });
    });
    let response = ui
        .interact(
            card.response.rect,
            Id::new(("sample-card", sample)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = widgets::named(response, &Command::OpenSample(sample).title());
    if response.hovered() {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(CARD_RADIUS),
            Stroke::new(BORDER_WIDTH, tokens.border_strong),
            StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        widgets::paint_focus_ring(ui, response.rect);
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_settings_round_trip_and_keep_hints_a_newer_version_added() {
        let mut settings = Settings::default();
        settings.set_texts(DISMISSED_KEY, &["start".to_owned(), "future".to_owned()]);
        let mut onboarding = Onboarding::from_settings(&settings);
        assert!(!onboarding.welcomed);
        assert!(onboarding.hints);
        assert_eq!(onboarding.dismissed, BTreeSet::from([Hint::Start]));
        onboarding.welcomed = true;
        onboarding.dismissed.insert(Hint::Palette);
        onboarding.write(&mut settings);
        assert_eq!(
            settings.texts(DISMISSED_KEY),
            Some(vec![
                "start".to_owned(),
                "palette".to_owned(),
                "future".to_owned()
            ])
        );
        assert_eq!(Onboarding::from_settings(&settings), onboarding);
    }
}
