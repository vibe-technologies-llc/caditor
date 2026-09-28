use std::collections::BTreeSet;

use caditor_document::FeatureKind;
use caditor_file::Settings;
use egui::{
    Align2, Area, CornerRadius, Frame, Id, Margin, Order, Rect, RichText, Sense, Stroke, Ui, vec2,
};

use crate::{
    appearance::{self, CARD_RADIUS},
    commands::{self, Command, Keymap, Offer},
    editing::{SketchEditing, Tool},
    fonts, icons,
    model::Model,
    samples::Sample,
    sketch_status::{SketchStatus, SketchSummary},
    widgets::{self, DialogWidth},
};

const WELCOMED_KEY: &str = "onboarding.welcomed";
const HINTS_KEY: &str = "onboarding.hints";
const DISMISSED_KEY: &str = "onboarding.dismissed_hints";
const TIP_TITLE: &str = "Tip";
const HINT_WIDTH: f32 = 360.0;
const HINT_MARGIN: i8 = 14;
const HINT_BOTTOM_CLEARANCE: f32 = 16.0;
const HINT_GAP: f32 = 4.0;
const SAMPLE_GAP: f32 = 8.0;
const SAMPLE_MARGIN: i8 = 10;
const SAMPLE_ICON_SIZE: f32 = 24.0;
const FOCUS_WIDTH: f32 = 2.0;

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

    pub fn text(self, keymap: &Keymap) -> String {
        let keys = |command: Command, fallback: &str| {
            keymap.first(command).map_or_else(
                || fallback.to_owned(),
                |shortcut| commands::display(&shortcut),
            )
        };
        match self {
            Self::Start => "Start with New sketch: pick a plane, draw a closed outline and \
                            Extrude it into a body. Or open a sample from File › Open Sample to \
                            see a finished model."
                .to_owned(),
            Self::Draw => format!(
                "Pick a drawing tool above ({} for lines, {} for rectangles, {} for circles) and \
                 click in the view. Typing x, y places a point exactly.",
                keys(Command::SketchTool(Tool::Line), "Line"),
                keys(Command::SketchTool(Tool::Rectangle), "Rectangle"),
                keys(Command::SketchTool(Tool::Circle), "Circle"),
            ),
            Self::Constrain => "Select geometry and add constraints and dimensions from the \
                                toolbar. The counter says how much can still move; fully \
                                constrained geometry turns green. Nothing is lost if you stop \
                                here."
                .to_owned(),
            Self::Sweep => "The sketch has a closed outline: Extrude or Revolve it to make a \
                            body. Its size stays editable in the feature's panel."
                .to_owned(),
            Self::Navigate => "Right-drag to orbit, middle-drag to pan and scroll to zoom. \
                               Double-click a face to open the feature that made it, and change \
                               any value in the tree or the parameter table."
                .to_owned(),
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
) -> Option<HintChoice> {
    let anchor = viewport.center_bottom() - vec2(0.0, HINT_BOTTOM_CLEARANCE);
    Area::new(Id::new("onboarding-hint"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            widgets::dialog_frame(ctx)
                .inner_margin(Margin::same(HINT_MARGIN))
                .show(ui, |ui| {
                    ui.set_max_width(HINT_WIDTH);
                    ui.horizontal_top(|ui| {
                        let accent = appearance::tokens(ui).accent_text;
                        widgets::icon_label(ui, icons::TIP, accent);
                        ui.vertical(|ui| {
                            ui.label(widgets::section_title(TIP_TITLE));
                            ui.label(hint.text(keymap));
                            ui.add_space(HINT_GAP);
                            ui.horizontal(|ui| {
                                if ui.add(widgets::primary_button(ui, "Got it")).clicked() {
                                    return Some(HintChoice::Dismiss(hint));
                                }
                                if ui
                                    .button("Hide tips")
                                    .on_hover_text("Turn tips back on in Preferences")
                                    .clicked()
                                {
                                    return Some(HintChoice::HideAll);
                                }
                                None
                            })
                            .inner
                        })
                        .inner
                    })
                    .inner
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
}

pub fn welcome(ctx: &egui::Context, keymap: &Keymap) -> Option<WelcomeChoice> {
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
            ui.add_space(SAMPLE_GAP);
            ui.label(widgets::section_title(
                "Open a sample to see how a model is built",
            ));
            let mut choice = None;
            for sample in Sample::ALL {
                if sample_card(ui, sample) {
                    choice = Some(WelcomeChoice::Sample(sample));
                }
            }
            let palette = keymap.first(Command::Palette).map_or_else(
                || "Search commands".to_owned(),
                |shortcut| commands::display(&shortcut),
            );
            ui.add_space(SAMPLE_GAP);
            ui.label(widgets::muted(
                format!(
                    "{palette} finds any command by name. Tips in the view suggest the next step; \
                     Help › Welcome brings this back."
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
                let open =
                    widgets::small_button(ui, icons::command(Command::Open), "Open a model…");
                if ui.add(open).clicked() {
                    choice = Some(WelcomeChoice::Open);
                }
            });
            choice
        },
    );
    let closed = response.should_close().then_some(WelcomeChoice::Close);
    response.inner.or(closed)
}

fn sample_card(ui: &mut Ui, sample: Sample) -> bool {
    let tokens = appearance::tokens(ui);
    let mut prepared = Frame::new()
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(SAMPLE_MARGIN))
        .begin(ui);
    {
        let ui = &mut prepared.content_ui;
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icons::SAMPLE)
                    .font(egui::FontId::new(SAMPLE_ICON_SIZE, fonts::icons()))
                    .color(tokens.accent_text),
            );
            ui.vertical(|ui| {
                ui.label(RichText::new(sample.title()).strong());
                ui.label(widgets::muted(sample.description(), ui));
            });
        });
    }
    let hovered = ui.rect_contains_pointer(prepared.content_ui.min_rect());
    prepared.frame.fill = if hovered { tokens.hover } else { tokens.raised };
    let response = prepared
        .end(ui)
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(CARD_RADIUS),
            Stroke::new(FOCUS_WIDTH, tokens.focus),
            egui::StrokeKind::Inside,
        );
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
