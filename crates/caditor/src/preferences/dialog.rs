use caditor_document::Rgb;
use caditor_render::ProjectionMode;
use egui::{Id, Label, Ui};

use super::{
    Accent, Appearance, GridColour, InputMode, MAX_SPEED, MIN_SPEED, PreferenceChange, Preferences,
    PreferencesCommand, PreferencesTab, Restored, Theme, TitleBar, ViewChoice,
    projection_description, projection_label,
};
use crate::{
    appearance::{self, MAX_SCALE, MIN_SCALE, SCALE_STEP, SPACE_M, SPACE_S},
    body_appearance::{self, color32},
    colour_selector,
    commands::Command,
    defender, dialog_parts,
    files::templates::{self, Templates},
    graphics::{self, Hardware},
    icons,
    scene_palette::{self, Canvas, Contrast},
    themes::{Look, UserThemes},
    units::{AngleUnit, LengthUnit},
    widgets::{self, DialogBody, DialogWidth, Tab, ThemeSample, Tone},
};

const EMPTY_MODEL: &str = "An empty model";
const HEIGHT_CHANGE: f32 = 0.5;
const CONFIRM_KEY: &str = "preferences-confirm-defaults";
const TALLEST_KEY: &str = "preferences-tallest-body";
const RESTORE_DEFAULTS: &str = "Restore defaults";

pub struct PreferencesView<'a> {
    pub tab: PreferencesTab,
    pub hardware: &'a Hardware,
    pub templates: &'a Templates,
    pub switch_keys: bool,
    pub restored: Option<&'a Restored>,
    pub themes: &'a UserThemes,
}

pub fn dialog(
    ctx: &egui::Context,
    preferences: &Preferences,
    view: &PreferencesView<'_>,
) -> Option<PreferencesCommand> {
    let confirm = Id::new(CONFIRM_KEY);
    let response = widgets::dialog(ctx, "preferences", "Preferences", DialogWidth::Wide, |ui| {
        let mut command = None;
        let tabs = PreferencesTab::ALL.map(|tab| Tab {
            glyph: tab.icon(),
            label: tab.label(),
        });
        let mut tab = view.tab;
        if let Some(chosen) = widgets::tabs(ui, "preferences", &tabs, tab.index(), view.switch_keys)
            .and_then(|index| PreferencesTab::ALL.get(index).copied())
        {
            tab = chosen;
            command = Some(PreferencesCommand::Tab(chosen));
            ui.data_mut(|data| data.remove::<PreferencesTab>(confirm));
        }
        body(ui, preferences, view, tab, &mut command);
        if view.restored.and_then(Restored::tab) == Some(tab)
            && dialog_parts::undo_note(
                ui,
                "This tab is back to its defaults.",
                "Put back the settings this tab had before",
            )
        {
            command = Some(PreferencesCommand::Undo);
        }
        let confirming = ui.data(|data| data.get_temp::<PreferencesTab>(confirm)) == Some(tab);
        if confirming {
            ui.add_space(SPACE_M);
            dialog_parts::confirmation(
                ui,
                &format!("Restore the defaults of the {} tab?", tab.label()),
                tab.defaults(),
            );
            if let Some(confirmed) = dialog_parts::confirm_footer(ui, RESTORE_DEFAULTS, "Cancel") {
                ui.data_mut(|data| data.remove::<PreferencesTab>(confirm));
                if confirmed {
                    command = Some(PreferencesCommand::Change(PreferenceChange::Defaults(tab)));
                }
            }
        } else {
            let chosen = widgets::footer_split(
                ui,
                |ui| {
                    ui.add(widgets::danger_button(RESTORE_DEFAULTS))
                        .on_hover_text(tab.defaults())
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
                Some(true) => command = Some(PreferencesCommand::Hide),
                Some(false) => ui.data_mut(|data| {
                    data.insert_temp(confirm, tab);
                }),
                None => {}
            }
        }
        command
    });
    let closed = response.should_close().then_some(PreferencesCommand::Hide);
    let command = response.inner.or(closed);
    if command == Some(PreferencesCommand::Hide) {
        ctx.data_mut(|data| data.remove::<PreferencesTab>(confirm));
    }
    command
}

fn body(
    ui: &mut Ui,
    preferences: &Preferences,
    view: &PreferencesView<'_>,
    tab: PreferencesTab,
    command: &mut Option<PreferencesCommand>,
) {
    let tallest_id = Id::new(TALLEST_KEY);
    let tallest = ui
        .data(|data| data.get_temp::<f32>(tallest_id))
        .unwrap_or_default();
    let height = DialogBody::default()
        .id_salt(("preferences", tab.label()))
        .min_height(tallest)
        .show(ui, |ui| {
            ui.scope(|ui| match tab {
                PreferencesTab::General => general(ui, preferences, view.templates, command),
                PreferencesTab::Appearance => appearance(ui, preferences, view.themes, command),
                PreferencesTab::Navigation => navigation(ui, preferences, command),
                PreferencesTab::Graphics => {
                    graphics::tab(ui, &preferences.graphics, view.hardware, command);
                }
            })
            .response
            .rect
            .height()
        });
    if height > tallest + HEIGHT_CHANGE {
        ui.data_mut(|data| data.insert_temp(tallest_id, height));
        ui.ctx().request_repaint();
    }
}

pub fn section(
    ui: &mut Ui,
    title: &str,
    id: &str,
    note: Option<String>,
    rows: impl FnOnce(&mut Ui),
) {
    ui.add_space(SPACE_M);
    widgets::section(ui, &format!("preferences-{id}"), title, None, None, |ui| {
        widgets::card(ui, |ui| {
            widgets::properties(ui, id, rows);
            if let Some(note) = note {
                ui.add_space(SPACE_S);
                ui.add(Label::new(widgets::muted(note, ui)).wrap());
            }
        });
    });
}

pub fn change(command: &mut Option<PreferencesCommand>, change: PreferenceChange) {
    *command = Some(PreferencesCommand::Change(change));
}

pub fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    options: &[(T, &str, &str)],
    current: T,
) -> Option<T> {
    let selected = options
        .iter()
        .position(|(value, _, _)| *value == current)
        .unwrap_or(usize::MAX);
    let labels: Vec<(&str, &str)> = options
        .iter()
        .map(|(_, label, hover)| (*label, *hover))
        .collect();
    widgets::segmented(ui, &labels, selected)
        .and_then(|index| options.get(index))
        .map(|(value, _, _)| *value)
}

fn speed_slider(
    ui: &mut Ui,
    current: f64,
    make: fn(f64) -> PreferenceChange,
    command: &mut Option<PreferencesCommand>,
) {
    let mut speed = current;
    let response = widgets::slider(
        ui,
        egui::Slider::new(&mut speed, MIN_SPEED..=MAX_SPEED).logarithmic(true),
    );
    widgets::tie_to_caption(ui, &response);
    let settled = response.drag_stopped() || !response.dragged();
    if settled && (response.changed() || response.drag_stopped()) {
        change(command, make(speed));
    } else if response.changed() {
        *command = Some(PreferencesCommand::Preview(make(speed)));
    }
}

fn general(
    ui: &mut Ui,
    preferences: &Preferences,
    templates: &Templates,
    command: &mut Option<PreferencesCommand>,
) {
    units(ui, preferences, command);
    new_models(ui, preferences, templates, command);
    keyboard(ui, command);
    tips(ui, preferences, command);
    if defender::ON_WINDOWS {
        saving_on_windows(ui, command);
    }
}

fn saving_on_windows(ui: &mut Ui, command: &mut Option<PreferencesCommand>) {
    let note = "Excluding your models' folder from Microsoft Defender's scanning makes saving \
                faster; the reminder explains what that trades away.";
    section(
        ui,
        "Saving on Windows",
        "saving-on-windows",
        Some(note.to_owned()),
        |ui| {
            widgets::property(ui, "Microsoft Defender", |ui| {
                let button = widgets::small_button(ui, icons::DEFENDER, defender::SHOW_AGAIN);
                if ui
                    .add(button)
                    .on_hover_text("Close Preferences and show the reminder over the view")
                    .clicked()
                {
                    *command = Some(PreferencesCommand::ShowDefenderReminder);
                }
            });
        },
    );
}

fn units(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let unit = preferences.unit.label().to_lowercase();
    let angle = preferences.angle.label().to_lowercase();
    let note = format!(
        "Lengths are shown in {unit} and plain numbers typed for a length mean {unit}; angles are \
         shown in {angle} and plain numbers typed for an angle mean {angle}. Values already in the \
         model keep the units they were entered in."
    );
    let hovers = LengthUnit::ALL.map(|unit| {
        format!(
            "Show lengths in {} ({})",
            unit.label().to_lowercase(),
            unit.symbol()
        )
    });
    let angle_hovers = AngleUnit::ALL.map(|unit| {
        format!(
            "Show angles in {} ({})",
            unit.label().to_lowercase(),
            unit.symbol()
        )
    });
    section(ui, "Units", "units", Some(note), |ui| {
        widgets::property(ui, "Length", |ui| {
            let options: Vec<(LengthUnit, &str, &str)> = LengthUnit::ALL
                .iter()
                .zip(&hovers)
                .map(|(unit, hover)| (*unit, unit.label(), hover.as_str()))
                .collect();
            if let Some(unit) = choice(ui, &options, preferences.unit) {
                change(command, PreferenceChange::Unit(unit));
            }
        });
        widgets::property(ui, "Angle", |ui| {
            let options: Vec<(AngleUnit, &str, &str)> = AngleUnit::ALL
                .iter()
                .zip(&angle_hovers)
                .map(|(unit, hover)| (*unit, unit.label(), hover.as_str()))
                .collect();
            if let Some(unit) = choice(ui, &options, preferences.angle) {
                change(command, PreferenceChange::Angle(unit));
            }
        });
    });
}

fn new_models(
    ui: &mut Ui,
    preferences: &Preferences,
    templates: &Templates,
    command: &mut Option<PreferencesCommand>,
) {
    let current = preferences.default_template.as_deref();
    let missing = current.is_some_and(|name| !templates.lists(name));
    let note = match templates.folder() {
        Some(folder) => format!(
            "New model starts from the chosen template's parameters, model properties, saved \
             views, sketches and features, as an untitled copy. Templates are caditor models \
             kept in {}; File › Save as template… adds one.",
            folder.display()
        ),
        None => "Templates need a configuration folder, and caditor could not find one, so new \
                 models start empty."
            .to_owned(),
    };
    section(ui, "New models", "new-models", Some(note), |ui| {
        widgets::property(ui, "Start from", |ui| {
            let selected: egui::WidgetText = match current {
                None => EMPTY_MODEL.into(),
                Some(name) if missing => {
                    let warning = appearance::tokens(ui).warn;
                    egui::RichText::new(format!("{} (not found)", templates::title(name)))
                        .color(warning)
                        .into()
                }
                Some(name) => templates::title(name).into(),
            };
            let combo = widgets::combo("default-template", selected).show(ui, |ui| {
                let empty = widgets::menu_option(ui, current.is_none(), EMPTY_MODEL)
                    .on_hover_text("New model starts with nothing in it");
                if empty.clicked() && current.is_some() {
                    change(command, PreferenceChange::DefaultTemplate(None));
                }
                for name in templates.names() {
                    let chosen = current == Some(name);
                    let response = widgets::menu_option(ui, chosen, &templates::title(name))
                        .on_hover_text(format!(
                            "New model starts as a copy of the template {name}"
                        ));
                    if response.clicked() && !chosen {
                        change(
                            command,
                            PreferenceChange::DefaultTemplate(Some(name.to_owned())),
                        );
                    }
                }
            });
            if missing {
                combo.response.on_hover_text(
                    "This template is no longer in the templates folder, so New model starts \
                     empty and says so. Choose another, or An empty model.",
                );
            }
        });
    });
}

fn keyboard(ui: &mut Ui, command: &mut Option<PreferencesCommand>) {
    let note = "Search commands finds every command by name, whether or not it has a shortcut.";
    section(ui, "Keyboard", "keyboard", Some(note.to_owned()), |ui| {
        widgets::property(ui, "Shortcuts", |ui| {
            let button = widgets::small_button(
                ui,
                icons::command(Command::KeyboardShortcuts),
                &Command::KeyboardShortcuts.title(),
            );
            if ui
                .add(button)
                .on_hover_text("See every shortcut and change any of them")
                .clicked()
            {
                *command = Some(PreferencesCommand::ShowShortcuts);
            }
        });
    });
}

fn tips(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let onboarding = &preferences.onboarding;
    section(ui, "Tips", "tips", None, |ui| {
        widgets::property(ui, "Getting started", |ui| {
            let mut shown = onboarding.hints;
            if widgets::checkbox(ui, &mut shown, "Show tips in the view").changed() {
                change(command, PreferenceChange::ShowHints(shown));
            }
        });
        ui.label("");
        let restore = ui
            .add_enabled(
                !onboarding.dismissed.is_empty(),
                widgets::button("Show dismissed tips again"),
            )
            .on_hover_text("Bring back the tips dismissed with Got it")
            .on_disabled_hover_text("No tip has been dismissed")
            .clicked();
        if restore {
            change(command, PreferenceChange::RestoreHints);
        }
        ui.end_row();
    });
}

pub const RELOAD_THEMES: &str = "Reload themes";
pub const THEMES_SECTION: &str = "Theme";
pub const AUTOMATIC_GRID: &str = "Grid: automatic";
pub const CUSTOM_GRID: &str = "Custom grid colour";
pub const GRID_SELECTOR: &str = "preferences-grid-colour";

pub fn theme_card_id(theme: &Theme) -> Id {
    Id::new(("theme-card", theme.key()))
}

pub fn accent_name(accent: Accent) -> String {
    format!("Accent: {}", accent.label())
}

fn sample(look: &Look, appearance: &Appearance) -> ThemeSample {
    let skin = appearance.skin_of(look);
    let tokens = skin.tokens;
    let contrast = Contrast::of(skin.high_contrast);
    let palette = contrast.palette(skin.canvas);
    ThemeSample {
        panel: tokens.panel,
        raised: tokens.raised,
        border: tokens.border,
        text: tokens.text,
        muted: tokens.text_muted,
        accent: tokens.accent,
        canvas: skin.canvas.colour(),
        grid: scene_palette::shown_grid(skin.grid, contrast, skin.canvas),
        body: color32(body_appearance::DEFAULT_COLOUR),
        edge: scene_palette::on_canvas(palette.body_edge, skin.canvas),
    }
}

fn theme_samples(theme: &Theme, themes: &UserThemes, appearance: &Appearance) -> Vec<ThemeSample> {
    match theme {
        Theme::System => [Theme::Dark, Theme::Light]
            .iter()
            .filter_map(|half| half.preview(themes))
            .map(|look| sample(&look, appearance))
            .collect(),
        other => other
            .preview(themes)
            .map(|look| vec![sample(&look, appearance)])
            .unwrap_or_default(),
    }
}

fn theme_cards(
    ui: &mut Ui,
    current: &Appearance,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    let mut offered: Vec<Theme> = Theme::SHIPPED.to_vec();
    offered.extend(
        themes
            .themes()
            .iter()
            .map(|theme| Theme::User(theme.key.clone())),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_M, SPACE_M);
        for theme in offered {
            let samples = theme_samples(&theme, themes, current);
            let name = themes.name_of(&theme);
            let chosen = current.theme == theme;
            let response = widgets::theme_card(
                ui,
                theme_card_id(&theme),
                &samples,
                &name,
                theme.description(),
                chosen,
            );
            if response.clicked() && !chosen {
                change(command, PreferenceChange::Theme(theme));
            }
        }
    });
}

fn themes_folder(ui: &mut Ui, themes: &UserThemes, command: &mut Option<PreferencesCommand>) {
    let note = match themes.folder() {
        Some(folder) => format!(
            "Choosing a theme changes the whole window at once. Your own themes are JSON files \
             in {}, each a base (dark or light), a 3D view, a grid colour and colours by name, \
             checked for readable contrast before they are offered.",
            folder.display()
        ),
        None => "Choosing a theme changes the whole window at once. Your own themes need a \
                 configuration folder, and caditor could not find one."
            .to_owned(),
    };
    ui.add_space(SPACE_S);
    ui.add(Label::new(widgets::muted(note, ui)).wrap());
    for refusal in themes.refused() {
        ui.add_space(SPACE_S);
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.add(Label::new(refusal.words()).wrap());
        });
    }
    if themes.folder().is_some() {
        ui.add_space(SPACE_S);
        let button = widgets::small_button(ui, icons::RELOAD, RELOAD_THEMES);
        if ui
            .add(button)
            .on_hover_text("Read the themes folder again, after adding or changing a theme")
            .clicked()
        {
            *command = Some(PreferencesCommand::ReloadThemes);
        }
    }
}

fn accent_swatches(
    ui: &mut Ui,
    current: &Appearance,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    let look = chosen_look(current, themes);
    ui.horizontal_wrapped(|ui| {
        for accent in Accent::ALL {
            let offered = accent.applied(&look);
            let fill = offered.map_or(look.tokens.accent, |tokens| tokens.accent);
            let chosen = current.accent == accent;
            let name = accent_name(accent);
            let response = ui
                .add_enabled_ui(offered.is_some() && !current.high_contrast, |ui| {
                    widgets::swatch(ui, fill, &name, chosen)
                })
                .inner
                .on_disabled_hover_text(if current.high_contrast {
                    "High contrast keeps its own accent"
                } else {
                    "This accent would not keep its text readable in this theme"
                });
            if response.clicked() && !chosen {
                change(command, PreferenceChange::Accent(accent));
            }
        }
    });
}

fn chosen_look(current: &Appearance, themes: &UserThemes) -> Look {
    current.theme.preview(themes).unwrap_or(Look {
        tokens: appearance::DARK,
        dark: true,
        canvas: Canvas::Dark,
        grid: None,
    })
}

pub fn view_name(view: ViewChoice) -> String {
    format!("3D view: {}", view.label())
}

pub fn custom_grid_name(colour: Rgb) -> String {
    format!("Grid: {}", colour_selector::describe(colour))
}

fn view_swatches(
    ui: &mut Ui,
    current: &Appearance,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    ui.horizontal_wrapped(|ui| {
        for view in ViewChoice::ALL {
            let chosen = current.view == view;
            let name = view_name(view);
            let response = match view {
                ViewChoice::Theme => widgets::icon_swatch(ui, icons::APPEARANCE, &name, chosen),
                ViewChoice::Canvas(canvas) => widgets::swatch(ui, canvas.colour(), &name, chosen),
            };
            if response.clicked() && !chosen {
                change(command, PreferenceChange::View(view));
            }
        }
    });
    let canvas = current.view.canvas(chosen_look(current, themes).canvas);
    let words = match current.view {
        ViewChoice::Theme => format!(
            "With the theme, now {}: {}.",
            canvas.label(),
            canvas.description()
        ),
        ViewChoice::Canvas(_) => format!("{}: {}.", canvas.label(), canvas.description()),
    };
    ui.add(Label::new(widgets::muted(words, ui)).wrap());
}

fn grid_swatches(
    ui: &mut Ui,
    current: &Appearance,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    let look = chosen_look(current, themes);
    let canvas = current.view.canvas(look.canvas);
    let contrast = Contrast::of(current.high_contrast);
    let automatic = Appearance {
        grid: GridColour::Automatic,
        ..current.clone()
    }
    .skin_of(&look)
    .grid;
    let automatic = scene_palette::shown_grid(automatic, contrast, canvas);
    let custom = current.grid.custom();
    let selector = Id::new(GRID_SELECTOR);
    let refusal = selector.with("refused");
    let selecting = colour_selector::is_open(ui, selector);
    let toggled = ui
        .horizontal_wrapped(|ui| {
            let automatic = widgets::swatch(ui, automatic, AUTOMATIC_GRID, custom.is_none());
            if automatic.clicked() && custom.is_some() {
                change(command, PreferenceChange::Grid(GridColour::Automatic));
            }
            if let Some(colour) = custom {
                widgets::swatch(ui, color32(colour), &custom_grid_name(colour), true);
            }
            widgets::icon_swatch(ui, icons::ADD, CUSTOM_GRID, selecting).clicked()
        })
        .inner;
    let shown = custom.unwrap_or_else(|| Rgb::new(automatic.r(), automatic.g(), automatic.b()));
    if toggled {
        colour_selector::toggle(ui, selector, shown);
    }
    if let Some(colour) = colour_selector::show(ui, selector, shown) {
        match scene_palette::seen_grid(color32(colour), canvas, contrast) {
            Ok(_) => {
                ui.data_mut(|data| data.remove::<String>(refusal));
                change(command, PreferenceChange::Grid(GridColour::Custom(colour)));
            }
            Err(unseen) => {
                ui.data_mut(|data| data.insert_temp(refusal, unseen.sentence()));
            }
        }
    }
    if !colour_selector::is_open(ui, selector) {
        ui.data_mut(|data| data.remove::<String>(refusal));
    }
    if let Some(words) = ui.data(|data| data.get_temp::<String>(refusal)) {
        widgets::callout(ui, Tone::Error, |ui| {
            ui.add(Label::new(words).wrap());
        });
    } else if let Some(unseen) = current.unseen_grid(canvas) {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.add(Label::new(unseen.drawn_instead()).wrap());
        });
    }
}

fn appearance(
    ui: &mut Ui,
    preferences: &Preferences,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    let current = &preferences.appearance;
    ui.add_space(SPACE_M);
    widgets::section(ui, "preferences-theme", THEMES_SECTION, None, None, |ui| {
        widgets::card(ui, |ui| {
            theme_cards(ui, current, themes, command);
            themes_folder(ui, themes, command);
        });
    });
    let note = "The accent colours buttons, selections and links. High contrast replaces the \
                theme's colours with stronger ones of the same lightness.";
    section(ui, "Colours", "colours", Some(note.to_owned()), |ui| {
        widgets::property(ui, "Accent", |ui| {
            accent_swatches(ui, current, themes, command);
        });
        widgets::property(ui, "3D view", |ui| {
            ui.vertical(|ui| view_swatches(ui, current, themes, command));
        });
        widgets::property(ui, "Grid", |ui| {
            ui.vertical(|ui| grid_swatches(ui, current, themes, command));
        });
        widgets::property(ui, "Contrast", |ui| {
            let mut high_contrast = current.high_contrast;
            if widgets::checkbox(ui, &mut high_contrast, "High contrast")
                .on_hover_text("Stronger text, outlined buttons and a bright focus outline")
                .changed()
            {
                change(command, PreferenceChange::HighContrast(high_contrast));
            }
        });
    });
    section(ui, "Interface", "interface", None, |ui| {
        widgets::property(ui, "Size", |ui| {
            let smaller = Command::SmallerInterface.title();
            let larger = Command::LargerInterface.title();
            let step = widgets::stepper(
                ui,
                &format!("{:.0}%", current.scale * 100.0),
                (&smaller, current.scale > MIN_SCALE),
                (&larger, current.scale < MAX_SCALE),
            );
            if let Some(step) = step {
                let scale = current.scale + f32::from(step) * SCALE_STEP;
                change(command, PreferenceChange::Scale(scale));
            }
        });
        widgets::property(ui, "Title bar", |ui| {
            let options = TitleBar::ALL.map(|bar| (bar, bar.label(), bar.description()));
            if let Some(bar) = choice(ui, &options, preferences.title_bar) {
                change(command, PreferenceChange::TitleBar(bar));
            }
        });
    });
}

fn navigation(ui: &mut Ui, preferences: &Preferences, command: &mut Option<PreferencesCommand>) {
    let navigation = preferences.navigation;
    let toggle = preferences
        .keymap
        .hint(Command::ToggleProjection)
        .map_or_else(|| Command::ToggleProjection.title(), str::to_owned);
    let note = format!("{toggle} switches between perspective and orthographic in the view.");
    section(ui, "View", "navigation-view", Some(note), |ui| {
        widgets::property(ui, "Projection", |ui| {
            let options = ProjectionMode::ALL.map(|projection| {
                (
                    projection,
                    projection_label(projection),
                    projection_description(projection),
                )
            });
            if let Some(projection) = choice(ui, &options, navigation.projection) {
                change(command, PreferenceChange::Projection(projection));
            }
        });
    });
    section(ui, "Input", "navigation-input", None, |ui| {
        widgets::property(ui, "Input mode", |ui| {
            let options = InputMode::ALL.map(|mode| (mode, mode.label(), mode.description()));
            if let Some(mode) = choice(ui, &options, navigation.input_mode) {
                change(command, PreferenceChange::InputMode(mode));
            }
        });
        ui.label("");
        ui.add(Label::new(widgets::muted(navigation.input_mode.description(), ui)).wrap());
        ui.end_row();
    });
    section(ui, "Movement", "navigation", None, |ui| {
        widgets::property(ui, "Orbit speed", |ui| {
            speed_slider(
                ui,
                navigation.orbit_speed,
                PreferenceChange::OrbitSpeed,
                command,
            );
        });
        widgets::property(ui, "Zoom speed", |ui| {
            speed_slider(
                ui,
                navigation.zoom_speed,
                PreferenceChange::ZoomSpeed,
                command,
            );
        });
        widgets::property(ui, "Scrolling", |ui| {
            let mut invert = navigation.invert_zoom;
            if widgets::checkbox(ui, &mut invert, "Scroll up to zoom out").changed() {
                change(command, PreferenceChange::InvertZoom(invert));
            }
        });
    });
}
