use std::{fs, path::Path};

use caditor_document::Rgb;
use egui::{
    Key, Modifiers,
    accesskit::{Role, Toggled},
};
use tempfile::TempDir;

use super::Harness;
use crate::{
    appearance::{self, PAPER},
    canvas, colour_selector,
    model::Action,
    preferences::{
        self, Accent, GridColour, PreferenceChange, PreferencesCommand, PreferencesTab, Theme,
        ViewChoice,
    },
    scene_palette::{Canvas, Contrast},
};

const OCEAN: &str = r##"{"name": "Ocean", "base": "dark", "view": "light",
    "colours": {"panel": "#0b2230", "raised": "#10293a", "accent": "#1f6fb2",
    "field_border": "#8090a8"}}"##;
const MURKY: &str = r##"{"name": "Murky", "colours": {"text_muted": "#3c3e46"}}"##;

fn themes_folder(dir: &Path) -> std::path::PathBuf {
    dir.join("config").join("themes")
}

fn with_themes(dir: &Path, themes: &[(&str, &str)]) -> Harness {
    let folder = themes_folder(dir);
    fs::create_dir_all(&folder).unwrap();
    for (name, text) in themes {
        fs::write(folder.join(name), text).unwrap();
    }
    let mut harness = Harness::with_directories(Some(dir));
    harness.wait_until("the themes are read", |harness| {
        harness.files.themes().generation() > 0
    });
    harness.frame();
    harness
}

fn open_appearance(harness: &mut Harness) {
    harness.perform(Action::Preferences(PreferencesCommand::Tab(
        PreferencesTab::Appearance,
    )));
    harness.perform(Action::Preferences(PreferencesCommand::Show));
    harness.context.enable_accesskit();
    harness.frame();
    harness.frame();
}

fn card_selected(harness: &Harness, name: &str) -> Option<bool> {
    harness
        .accessible
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some(name))
        .map(|(_, node)| node.toggled() == Some(Toggled::True))
}

#[test]
fn the_theme_picker_offers_a_card_per_theme_and_chooses_by_click_and_keyboard() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_themes(dir.path(), &[("ocean.json", OCEAN), ("murky.json", MURKY)]);
    open_appearance(&mut harness);

    for name in [
        "Follow the system",
        "Dark",
        "Light",
        "Midnight",
        "Graphite",
        "Paper",
        "Ocean",
    ] {
        assert!(card_selected(&harness, name).is_some(), "{name}");
    }
    assert_eq!(card_selected(&harness, "Follow the system"), Some(true));
    assert_eq!(card_selected(&harness, "Murky"), None);
    assert!(harness.shows_containing("murky.json was not loaded: the muted text (text_muted"));
    assert!(harness.unreadable_nodes().is_empty());

    harness.click_button("Paper");
    harness.frame();
    harness.frame();

    assert_eq!(harness.workspace.preferences.appearance.theme, Theme::Paper);
    assert_eq!(appearance::tokens_of(&harness.context), PAPER);
    assert_eq!(canvas::canvas(&harness.context), Canvas::Light);
    assert_eq!(harness.workspace.viewport.canvas(), Canvas::Light);
    assert_eq!(card_selected(&harness, "Paper"), Some(true));

    harness.context.memory_mut(|memory| {
        memory.request_focus(preferences::theme_card_id(&Theme::Paper));
    });
    harness.frame();
    harness.key(Key::ArrowLeft, Modifiers::NONE);
    harness.frame();
    assert_eq!(
        harness.focused(),
        Some(preferences::theme_card_id(&Theme::Graphite))
    );
    harness.key(Key::Enter, Modifiers::NONE);
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.workspace.preferences.appearance.theme,
        Theme::Graphite
    );
    assert_eq!(canvas::canvas(&harness.context), Canvas::Dark);

    harness.click_button("Ocean");
    harness.frame();
    harness.frame();

    assert_eq!(
        appearance::tokens_of(&harness.context).panel,
        egui::Color32::from_rgb(0x0b, 0x22, 0x30)
    );
    assert_eq!(canvas::canvas(&harness.context), Canvas::Light);
}

#[test]
fn an_accent_and_the_3d_view_change_any_theme_and_high_contrast_keeps_its_own() {
    let mut harness = Harness::new();
    open_appearance(&mut harness);
    let own = appearance::tokens_of(&harness.context).accent;

    harness.click_button(&preferences::accent_name(Accent::Teal));
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.workspace.preferences.appearance.accent,
        Accent::Teal
    );
    assert_ne!(appearance::tokens_of(&harness.context).accent, own);

    harness.click_button(&preferences::view_name(ViewChoice::Canvas(Canvas::Light)));
    harness.frame();
    harness.frame();
    assert_eq!(canvas::canvas(&harness.context), Canvas::Light);

    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::HighContrast(true),
    )));
    harness.frame();
    harness.frame();

    let skin = appearance::skin(&harness.context);
    assert!(skin.high_contrast);
    assert_eq!(skin.canvas, Canvas::Light);
    assert!(appearance::check(&skin.tokens, true).is_ok());
}

#[test]
fn a_chosen_theme_failing_on_reload_is_refused_in_words_and_the_previous_theme_kept() {
    let dir = TempDir::new().unwrap();
    let mut harness = with_themes(dir.path(), &[("ocean.json", OCEAN)]);
    harness.perform(Action::Preferences(PreferencesCommand::Change(
        PreferenceChange::Theme(Theme::User("ocean".to_owned())),
    )));
    harness.frame();
    let ocean = appearance::tokens_of(&harness.context);
    assert_eq!(ocean.panel, egui::Color32::from_rgb(0x0b, 0x22, 0x30));

    fs::write(
        themes_folder(dir.path()).join("ocean.json"),
        r##"{"name": "Ocean", "colours": {"text": "#30343c"}}"##,
    )
    .unwrap();
    harness.perform(Action::Preferences(PreferencesCommand::ReloadThemes));
    harness.wait_until("the themes are read again", |harness| {
        harness.files.themes().generation() > 1
    });
    harness.frame();
    harness.frame();

    let notice = harness.model.notice().unwrap();
    assert!(
        notice
            .text
            .starts_with("The theme “ocean” could not be used: the text (text #30343c) on "),
        "{}",
        notice.text
    );
    assert!(notice.text.ends_with("The previous theme stays."));
    assert_eq!(appearance::tokens_of(&harness.context), ocean);
    assert_eq!(
        harness.workspace.preferences.appearance.theme,
        Theme::User("ocean".to_owned())
    );
}

#[test]
fn a_lighter_3d_view_and_a_grid_colour_are_chosen_in_preferences_and_an_unseen_grid_refused() {
    let mut harness = Harness::new();
    open_appearance(&mut harness);
    let studio = ViewChoice::Canvas(Canvas::Studio);
    let hex = colour_selector::hex_id(egui::Id::unique(preferences::GRID_SELECTOR));

    for view in ViewChoice::ALL {
        assert!(
            card_selected(&harness, &preferences::view_name(view)).is_some(),
            "{view:?}"
        );
    }
    harness.click_button(&preferences::view_name(studio));
    harness.frame();
    harness.frame();

    assert_eq!(harness.workspace.preferences.appearance.view, studio);
    assert_eq!(canvas::canvas(&harness.context), Canvas::Studio);
    assert_eq!(harness.workspace.viewport.canvas(), Canvas::Studio);
    assert_eq!(
        card_selected(&harness, &preferences::view_name(studio)),
        Some(true)
    );
    assert!(harness.shows_containing("Studio grey: A light grey 3D view"));
    assert_eq!(
        harness.built().scene.grid.unwrap().color,
        Contrast::Standard.palette(Canvas::Studio).grid
    );

    harness.click_button(preferences::CUSTOM_GRID);
    harness.frame();
    harness.type_into_field(hex, "#c4c7cc");
    harness.frame();

    assert_eq!(
        harness.workspace.preferences.appearance.grid,
        GridColour::Automatic
    );
    assert!(harness.shows_containing("The grid colour #c4c7cc shows at only 1.0"));
    assert!(harness.shows_containing("Choose a colour lighter or darker than the 3D view."));

    harness.type_into_field(hex, "#3c4250");
    harness.frame();
    harness.frame();

    let chosen = Rgb::new(0x3c, 0x42, 0x50);
    assert_eq!(
        harness.workspace.preferences.appearance.grid,
        GridColour::Custom(chosen)
    );
    assert!(!harness.shows_containing("The grid colour #c4c7cc"));
    assert_eq!(
        harness.built().scene.grid.unwrap().color,
        caditor_render::Color::from_rgb8(0x3c, 0x42, 0x50)
    );
    assert_eq!(
        card_selected(&harness, &preferences::custom_grid_name(chosen)),
        Some(true)
    );
    assert!(
        harness
            .workspace
            .preferences
            .settings()
            .text("appearance.grid")
            .is_some_and(|grid| grid == "#3c4250")
    );

    harness.click_button(&preferences::view_name(ViewChoice::Canvas(Canvas::Grey)));
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.built().scene.grid.unwrap().color,
        Contrast::Standard.palette(Canvas::Grey).grid
    );
    assert!(harness.shows_containing("so the 3D view's own grid colour is drawn"));

    harness.click_button(preferences::AUTOMATIC_GRID);
    harness.frame();
    harness.frame();

    assert_eq!(
        harness.workspace.preferences.appearance.grid,
        GridColour::Automatic
    );
    assert!(!harness.shows_containing("so the 3D view's own grid colour is drawn"));
    assert!(harness.unreadable_nodes().is_empty());
}
