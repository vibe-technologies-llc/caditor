use caditor_document::Rgb;
use caditor_file::Retention;
use egui::{Id, Label, TextEdit, TextStyle, TextWrapMode, Ui, WidgetText};

use super::{
    Accent, Appearance, BodyColour, Choice, DoubleClick, ExportMemory, GridColour, NewBodies,
    PreferenceChange, Preferences, PreferencesCommand, PreferencesTab, RecentListed, Restored,
    Restoring, Section, Startup, Theme, TitleBar, TooltipDelay, ViewChoice, motion,
};
use crate::{
    appearance::{self, MAX_SCALE, MIN_SCALE, SCALE_STEP, SPACE_M, SPACE_S},
    body_appearance::{self, SWATCHES, color32},
    colour_selector,
    commands::Command,
    defender, dialog_parts,
    files::templates::{self, Templates},
    graphics::{self, Hardware},
    icons, lighting_panel,
    scene_palette::{self, Canvas, Contrast},
    themes::{Look, UserThemes},
    units::{AngleUnit, Decimals, LengthUnit, TrailingZeros, Units},
    view_aids::EdgeWeight,
    view_cube::{Corner, CubeSize},
    widgets::{self, DialogBody, DialogWidth, SectionAction, Tab, ThemeSample, Tone},
};

pub const NEW_BODIES_CAPTION: &str = "New bodies from separate pieces";
const EMPTY_MODEL: &str = "An empty model";
const SAMPLE_LENGTH: f64 = 12.5;
const SAMPLE_ANGLE: f64 = 30.0;
const HEIGHT_CHANGE: f32 = 0.5;
const CONFIRM_KEY: &str = "preferences-confirm-defaults";
const TALLEST_KEY: &str = "preferences-tallest-body";
const RESTORE_DEFAULTS: &str = "Restore defaults";
const RESET_KEY: &str = "preferences-reset-section";
pub const SEARCH_FIELD: &str = "preferences-search";
pub const SEARCH_HINT: &str = "Search settings";
pub const CLEAR_SEARCH: &str = "Clear the search";

pub struct PreferencesView<'a> {
    pub tab: PreferencesTab,
    pub hardware: &'a Hardware,
    pub templates: &'a Templates,
    pub switch_keys: bool,
    pub restored: Option<&'a Restored>,
    pub themes: &'a UserThemes,
    pub search: &'a str,
}

pub struct Shown<'a> {
    pub preferences: &'a Preferences,
    pub query: &'a str,
}

pub fn reset_name(section: Section) -> String {
    format!("Reset {} to its defaults", section.title())
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
        if let Some(search) = search_field(ui, view.search) {
            command = Some(search);
        }
        let searching = !view.search.trim().is_empty();
        body(ui, preferences, view, tab, &mut command);
        if let Some(section) = take_reset(ui) {
            command = Some(PreferencesCommand::Change(
                PreferenceChange::SectionDefaults(section),
            ));
        }
        if let Some(restoring) = view.restored.and_then(Restored::restoring)
            && (searching || restoring.tab() == tab)
            && undo_restoring(ui, restoring)
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

fn undo_restoring(ui: &mut Ui, restoring: Restoring) -> bool {
    match restoring {
        Restoring::Tab(_) => dialog_parts::undo_note(
            ui,
            "This tab is back to its defaults.",
            "Put back the settings this tab had before",
        ),
        Restoring::Section(section) => dialog_parts::undo_note(
            ui,
            &format!("{} is back to its defaults.", section.title()),
            "Put back the settings this section had before",
        ),
    }
}

fn search_field(ui: &mut Ui, search: &str) -> Option<PreferencesCommand> {
    let mut text = search.to_owned();
    let edited = ui
        .horizontal(|ui| {
            let muted = appearance::tokens(ui).text_muted;
            widgets::icon_label(ui, icons::SEARCH, muted);
            widgets::text_field(ui, |ui| {
                ui.add(
                    TextEdit::singleline(&mut text)
                        .id(Id::new(SEARCH_FIELD))
                        .hint_text(SEARCH_HINT)
                        .desired_width(f32::INFINITY),
                )
            })
            .changed()
        })
        .inner;
    ui.add_space(SPACE_S);
    edited.then_some(PreferencesCommand::Search(text))
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
    let query = view.search.trim();
    let height = DialogBody::default()
        .id_salt(("preferences", tab.label()))
        .min_height(tallest)
        .show(ui, |ui| {
            ui.scope(|ui| {
                let shown = Shown { preferences, query };
                if query.is_empty() {
                    tab_body(ui, &shown, view, tab, command);
                } else {
                    search_results(ui, &shown, view, command);
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

fn tab_body(
    ui: &mut Ui,
    shown: &Shown<'_>,
    view: &PreferencesView<'_>,
    tab: PreferencesTab,
    command: &mut Option<PreferencesCommand>,
) {
    match tab {
        PreferencesTab::General => general(ui, shown, view.templates, command),
        PreferencesTab::Appearance => appearance(ui, shown, view.themes, command),
        PreferencesTab::Navigation => motion::navigation(ui, shown, command),
        PreferencesTab::Sketching => motion::sketching(ui, shown, command),
        PreferencesTab::Graphics => graphics::tab(ui, shown, view.hardware, command),
    }
}

fn search_results(
    ui: &mut Ui,
    shown: &Shown<'_>,
    view: &PreferencesView<'_>,
    command: &mut Option<PreferencesCommand>,
) {
    let mut any = false;
    for tab in PreferencesTab::ALL {
        let matching = Section::of_tab(tab)
            .into_iter()
            .filter(|section| section.matches(shown.query))
            .any(|section| section != Section::SavingOnWindows || defender::ON_WINDOWS);
        if !matching {
            continue;
        }
        any = true;
        ui.add_space(SPACE_M);
        ui.label(widgets::strong(tab.label()));
        tab_body(ui, shown, view, tab, command);
    }
    if !any {
        ui.add_space(SPACE_M);
        let text = format!("No setting matches \u{201c}{}\u{201d}.", shown.query);
        let cleared = widgets::empty_state(ui, icons::SEARCH, &text, |ui| {
            ui.add(widgets::button(CLEAR_SEARCH)).clicked()
        });
        if cleared {
            *command = Some(PreferencesCommand::Search(String::new()));
        }
    }
}

pub fn section(
    ui: &mut Ui,
    shown: &Shown<'_>,
    section: Section,
    note: Option<String>,
    rows: impl FnOnce(&mut Ui),
) {
    section_card(ui, shown, section, |ui| {
        widgets::properties(ui, section.id(), rows);
        if let Some(note) = note {
            ui.add_space(SPACE_S);
            ui.add(Label::new(widgets::muted(note, ui)).wrap());
        }
    });
}

fn section_card(ui: &mut Ui, shown: &Shown<'_>, section: Section, content: impl FnOnce(&mut Ui)) {
    if !section.matches(shown.query) {
        return;
    }
    ui.add_space(SPACE_M);
    let reset = reset_name(section);
    let action = (section.is_resettable() && !section.is_default(shown.preferences)).then_some(
        SectionAction {
            glyph: icons::RESET_SECTION,
            hover: &reset,
        },
    );
    let id = format!("preferences-{}", section.id());
    let acted = widgets::section(ui, &id, section.title(), None, action, |ui| {
        widgets::card(ui, content);
    });
    if acted {
        ui.data_mut(|data| data.insert_temp(Id::new(RESET_KEY), section));
    }
}

fn take_reset(ui: &Ui) -> Option<Section> {
    let id = Id::new(RESET_KEY);
    let section = ui.data(|data| data.get_temp::<Section>(id));
    if section.is_some() {
        ui.data_mut(|data| data.remove::<Section>(id));
    }
    section
}

pub fn pick<T: Choice>(ui: &mut Ui, current: T) -> Option<T> {
    let options: Vec<(T, &str, &str)> = T::ALL
        .iter()
        .map(|option| (*option, option.label(), option.description()))
        .collect();
    choice(ui, &options, current)
}

pub fn pick_listed<T: Choice>(ui: &mut Ui, id: &str, current: T) -> Option<T> {
    let mut picked = None;
    widgets::combo(id, current.label())
        .hover(current.description())
        .show(ui, |ui| {
            for option in T::ALL {
                let chosen = *option == current;
                let response = widgets::menu_option(ui, chosen, option.label())
                    .on_hover_text(option.description());
                if response.clicked() && !chosen {
                    picked = Some(*option);
                }
            }
        });
    picked
}

pub fn toggle(
    ui: &mut Ui,
    on: bool,
    label: &str,
    hover: &str,
    make: impl FnOnce(bool) -> PreferenceChange,
    command: &mut Option<PreferencesCommand>,
) {
    let mut checked = on;
    let toggled = ui
        .scope(|ui| {
            if fits_on_one_line(ui, label) {
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
            }
            widgets::checkbox(ui, &mut checked, label)
                .on_hover_text(hover)
                .changed()
        })
        .inner;
    if toggled {
        change(command, make(checked));
    }
}

fn fits_on_one_line(ui: &Ui, label: &str) -> bool {
    let text = WidgetText::from(label).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let spacing = ui.spacing();
    let room = ui.clip_rect().right() - ui.cursor().left();
    text.size().x + spacing.icon_width + spacing.icon_spacing <= room
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

fn general(
    ui: &mut Ui,
    shown: &Shown<'_>,
    templates: &Templates,
    command: &mut Option<PreferencesCommand>,
) {
    units(ui, shown, command);
    numbers(ui, shown, command);
    starting(ui, shown, command);
    new_models(ui, shown, templates, command);
    feature_tree(ui, shown, command);
    version_history(ui, shown, command);
    exports(ui, shown, command);
    keyboard(ui, shown, command);
    tips(ui, shown, command);
    if defender::ON_WINDOWS {
        saving_on_windows(ui, shown, command);
    }
}

fn starting(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
    let note = "A model whose unsaved work caditor kept is offered for recovery first, and a file \
                named on the command line opens instead of the last model.";
    section(ui, shown, Section::Files, Some(note.to_owned()), |ui| {
        widgets::property(ui, "At startup, open", |ui| {
            if let Some(startup) = pick::<Startup>(ui, preferences.startup) {
                change(command, PreferenceChange::Startup(startup));
            }
        });
        widgets::property(ui, "Recent files listed", |ui| {
            if let Some(listed) = pick::<RecentListed>(ui, preferences.recent_listed) {
                change(command, PreferenceChange::RecentListed(listed));
            }
        });
        widgets::property(ui, "Empty models", |ui| {
            toggle(
                ui,
                preferences.onboarding.start_card,
                SHOW_START_CARD,
                "The card over the 3D view offering a first sketch, a box or an import",
                PreferenceChange::ShowStartCard,
                command,
            );
        });
    });
}

fn feature_tree(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
    section(ui, shown, Section::FeatureTree, None, |ui| {
        widgets::property(ui, "Double-clicking a feature", |ui| {
            if let Some(double_click) = pick::<DoubleClick>(ui, preferences.double_click) {
                change(command, PreferenceChange::DoubleClick(double_click));
            }
        });
        widgets::property(ui, NEW_BODIES_CAPTION, |ui| {
            if let Some(new_bodies) = pick::<NewBodies>(ui, preferences.new_bodies) {
                change(command, PreferenceChange::NewBodies(new_bodies));
            }
        });
    });
}

fn version_history(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
    let note = "Each save keeps the model it replaces as an earlier version inside the file, and \
                older versions thin out with age. Versions you keep in File › Version history \
                stay whatever you choose here. Unsaved work is always kept for recovery.";
    section(
        ui,
        shown,
        Section::VersionHistory,
        Some(note.to_owned()),
        |ui| {
            widgets::property(ui, "Old versions", |ui| {
                if let Some(retention) = pick::<Retention>(ui, preferences.retention) {
                    change(command, PreferenceChange::Retention(retention));
                }
            });
        },
    );
}

pub const FORGET_EXPORTS: &str = "Forget remembered choices";

fn exports(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
    let note = "Each export dialog keeps the format, options and folder you last exported with: \
                the model formats and their mesh and thread options, the drawing format, layout \
                and sheet, and the image size, scale and background.";
    section(ui, shown, Section::Exports, Some(note.to_owned()), |ui| {
        widgets::property(ui, "Export choices", |ui| {
            if let Some(memory) = pick::<ExportMemory>(ui, preferences.export_memory) {
                change(command, PreferenceChange::ExportMemory(memory));
            }
        });
        ui.label("");
        let forget = ui
            .add_enabled(
                !preferences.exports.is_empty(),
                widgets::button(FORGET_EXPORTS),
            )
            .on_hover_text(
                "Start the next session's export dialogs from caditor's defaults; this session \
                 keeps its choices",
            )
            .on_disabled_hover_text("No export choice is remembered")
            .clicked();
        if forget {
            change(command, PreferenceChange::ForgetExports);
        }
        ui.end_row();
    });
}

fn saving_on_windows(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let note = "Excluding your models' folder from Microsoft Defender's scanning makes saving \
                faster; the reminder explains what that trades away.";
    section(
        ui,
        shown,
        Section::SavingOnWindows,
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

fn units(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
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
    section(ui, shown, Section::Units, Some(note), |ui| {
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

const LENGTH_DECIMALS: &str = "preferences-length-decimals";
const ANGLE_DECIMALS: &str = "preferences-angle-decimals";

fn numbers(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let preferences = shown.preferences;
    let numbers = preferences.numbers;
    let units = Units {
        length: preferences.unit,
        angle: preferences.angle,
        numbers,
    };
    let note = format!(
        "A measured length reads {} and an angle {}. Rounding is only for reading: the model \
         keeps every value as entered, and fields show what was typed.",
        units.measured_length(SAMPLE_LENGTH),
        units.angle_text(SAMPLE_ANGLE)
    );
    section(ui, shown, Section::Numbers, Some(note), |ui| {
        widgets::property(ui, "Length places", |ui| {
            if let Some(decimals) = pick_listed::<Decimals>(ui, LENGTH_DECIMALS, numbers.lengths) {
                change(command, PreferenceChange::LengthDecimals(decimals));
            }
        });
        widgets::property(ui, "Angle places", |ui| {
            if let Some(decimals) = pick_listed::<Decimals>(ui, ANGLE_DECIMALS, numbers.angles) {
                change(command, PreferenceChange::AngleDecimals(decimals));
            }
        });
        widgets::property(ui, "Trailing zeros", |ui| {
            if let Some(zeros) = pick::<TrailingZeros>(ui, numbers.trailing_zeros) {
                change(command, PreferenceChange::TrailingZeros(zeros));
            }
        });
    });
}

fn new_models(
    ui: &mut Ui,
    shown: &Shown<'_>,
    templates: &Templates,
    command: &mut Option<PreferencesCommand>,
) {
    let preferences = shown.preferences;
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
    section(ui, shown, Section::NewModels, Some(note), |ui| {
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

fn keyboard(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let note = "Search commands finds every command by name, whether or not it has a shortcut.";
    section(ui, shown, Section::Keyboard, Some(note.to_owned()), |ui| {
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

fn tips(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let onboarding = &shown.preferences.onboarding;
    section(ui, shown, Section::Tips, None, |ui| {
        widgets::property(ui, "Getting started", |ui| {
            let mut on = onboarding.hints;
            if widgets::checkbox(ui, &mut on, "Show tips in the view").changed() {
                change(command, PreferenceChange::ShowHints(on));
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

pub const SHOW_START_CARD: &str = "Show the start card on empty models";
pub const RELOAD_THEMES: &str = "Reload themes";
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
        body: color32(appearance.body_on(skin.canvas)),
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

pub const DEFAULT_BODY: &str = "Body colour: default";
pub const CUSTOM_BODY: &str = "Custom body colour";
pub const BODY_SELECTOR: &str = "preferences-body-colour";

pub fn body_swatch_name(name: &str) -> String {
    format!("Body colour: {name}")
}

fn body_swatches(
    ui: &mut Ui,
    current: &Appearance,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    let canvas = current.view.canvas(chosen_look(current, themes).canvas);
    let contrast = Contrast::of(current.high_contrast);
    let custom = current.body.custom();
    let selector = Id::new(BODY_SELECTOR);
    let refusal = selector.with("refused");
    let selecting = colour_selector::is_open(ui, selector);
    let toggled = ui
        .horizontal_wrapped(|ui| {
            let default = widgets::swatch(
                ui,
                color32(body_appearance::DEFAULT_COLOUR),
                DEFAULT_BODY,
                custom.is_none(),
            );
            if default.clicked() && custom.is_some() {
                change(command, PreferenceChange::Body(BodyColour::Default));
            }
            for swatch in &SWATCHES {
                let fill = color32(swatch.colour);
                let seen = scene_palette::seen_body(fill, canvas, contrast);
                let chosen = custom == Some(swatch.colour);
                let name = body_swatch_name(swatch.name);
                let response = ui
                    .add_enabled_ui(seen.is_ok(), |ui| widgets::swatch(ui, fill, &name, chosen))
                    .inner;
                let response = match &seen {
                    Ok(_) => response,
                    Err(unseen) => response.on_disabled_hover_text(unseen.sentence()),
                };
                if response.clicked() && !chosen {
                    change(
                        command,
                        PreferenceChange::Body(BodyColour::Custom(swatch.colour)),
                    );
                }
            }
            if let Some(colour) =
                custom.filter(|colour| SWATCHES.iter().all(|swatch| swatch.colour != *colour))
            {
                let name = body_swatch_name(&colour_selector::describe(colour));
                widgets::swatch(ui, color32(colour), &name, true);
            }
            widgets::icon_swatch(ui, icons::ADD, CUSTOM_BODY, selecting).clicked()
        })
        .inner;
    let shown = custom.unwrap_or(body_appearance::DEFAULT_COLOUR);
    if toggled {
        colour_selector::toggle(ui, selector, shown);
    }
    if let Some(colour) = colour_selector::show(ui, selector, shown) {
        match scene_palette::seen_body(color32(colour), canvas, contrast) {
            Ok(_) => {
                ui.data_mut(|data| data.remove::<String>(refusal));
                change(command, PreferenceChange::Body(BodyColour::Custom(colour)));
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
    } else if let Some(unseen) = current.unseen_body(canvas) {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.add(Label::new(unseen.drawn_instead()).wrap());
        });
    }
}

fn appearance(
    ui: &mut Ui,
    shown: &Shown<'_>,
    themes: &UserThemes,
    command: &mut Option<PreferencesCommand>,
) {
    let preferences = shown.preferences;
    let current = &preferences.appearance;
    section_card(ui, shown, Section::Theme, |ui| {
        theme_cards(ui, current, themes, command);
        themes_folder(ui, themes, command);
    });
    let note = "The accent colours buttons, selections and links. Bodies without a colour of \
                their own are drawn in the body colour; a model's files and exports keep only \
                colours given to a body. High contrast replaces the theme's colours with stronger \
                ones of the same lightness.";
    section(ui, shown, Section::Colours, Some(note.to_owned()), |ui| {
        widgets::property(ui, "Accent", |ui| {
            accent_swatches(ui, current, themes, command);
        });
        widgets::property(ui, "3D view", |ui| {
            ui.vertical(|ui| view_swatches(ui, current, themes, command));
        });
        widgets::property(ui, "Grid", |ui| {
            ui.vertical(|ui| grid_swatches(ui, current, themes, command));
        });
        widgets::property(ui, "Bodies", |ui| {
            ui.vertical(|ui| body_swatches(ui, current, themes, command));
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
    section(ui, shown, Section::Interface, None, |ui| {
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
        widgets::property(ui, "Hover help", |ui| {
            if let Some(delay) = pick::<TooltipDelay>(ui, preferences.tooltip_delay) {
                change(command, PreferenceChange::TooltipDelay(delay));
            }
        });
    });
    in_the_view(ui, shown, command);
    rendered_view(ui, shown, command);
}

fn rendered_view(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let note = "These light the Rendered display style, which View › Display style chooses; \
                Light the rendered view opens them beside the 3D view.";
    section(
        ui,
        shown,
        Section::RenderedView,
        Some(note.to_owned()),
        |ui| {
            lighting_panel::rows(ui, shown.preferences.lighting, command);
        },
    );
}

fn in_the_view(ui: &mut Ui, shown: &Shown<'_>, command: &mut Option<PreferencesCommand>) {
    let details = shown.preferences.view_details;
    let note = "Hiding the view cube keeps its commands: the standard views, Fit view and the \
                Isometric view stay in the View menu, the palette and on their keys.";
    section(ui, shown, Section::InTheView, Some(note.to_owned()), |ui| {
        widgets::property(ui, "Edges", |ui| {
            if let Some(weight) = pick::<EdgeWeight>(ui, details.edges.weight) {
                change(command, PreferenceChange::EdgeWeight(weight));
            }
        });
        widgets::property(ui, "Outlines", |ui| {
            toggle(
                ui,
                !details.edges.outlines_hidden,
                "Outline curved faces",
                "Draw the edge of round faces as seen from the view, such as the sides of a \
                 cylinder, in the edge colour",
                PreferenceChange::Outlines,
                command,
            );
        });
        widgets::property(ui, "View cube", |ui| {
            toggle(
                ui,
                details.view_cube,
                "Show the view cube",
                "The cube in a corner of the 3D view with Fit all and the Isometric view button \
                 under it",
                PreferenceChange::ViewCube,
                command,
            );
        });
        widgets::property(ui, "View cube size", |ui| {
            if let Some(size) = pick::<CubeSize>(ui, details.cube.size) {
                change(command, PreferenceChange::ViewCubeSize(size));
            }
        });
        widgets::property(ui, "View cube corner", |ui| {
            if let Some(corner) = pick::<Corner>(ui, details.cube.corner) {
                change(command, PreferenceChange::ViewCubeCorner(corner));
            }
        });
        widgets::property(ui, "Principal planes", |ui| {
            toggle(
                ui,
                details.principal_when_needed,
                "Show principal planes only when needed",
                "Once the model has a body, the principal planes, axes and origin show only while \
                 you choose a plane, axis or point; showing them yourself keeps them shown",
                PreferenceChange::PrincipalWhenNeeded,
                command,
            );
        });
    });
}
