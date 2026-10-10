use std::path::{Path, PathBuf};

use caditor_file::{ReadTheme, ThemeFile, ThemeFileError};
use egui::{Color32, ThemePreference};

use crate::{
    appearance::{
        self, ContrastFailure, DARK, GRAPHITE, LIGHT, MIDNIGHT, PAPER, READABLE, Skin, Skins,
        Token, Tokens, VISIBLE_OUTLINE,
    },
    scene_palette::Canvas,
};

const USER_PREFIX: &str = "user:";
const ACCENT_STEP: f32 = 0.04;
const ACCENT_STEPS: u8 = 25;
const HOVER_SHADE: f32 = 0.12;
const PRESSED_SHADE: f32 = 0.24;
const DARK_SUBTLE_SHARE: f32 = 0.26;
const LIGHT_SUBTLE_SHARE: f32 = 0.16;
const DARK_SURFACE_SHARE: f32 = 0.14;
const LIGHT_SURFACE_SHARE: f32 = 0.09;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
    Midnight,
    Graphite,
    Paper,
    User(String),
}

impl Theme {
    pub const SHIPPED: [Self; 6] = [
        Self::System,
        Self::Dark,
        Self::Light,
        Self::Midnight,
        Self::Graphite,
        Self::Paper,
    ];

    pub fn label(&self) -> String {
        match self {
            Self::System => "Follow the system".to_owned(),
            Self::Dark => "Dark".to_owned(),
            Self::Light => "Light".to_owned(),
            Self::Midnight => "Midnight".to_owned(),
            Self::Graphite => "Graphite".to_owned(),
            Self::Paper => "Paper".to_owned(),
            Self::User(key) => key.clone(),
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::System => "Dark or light as the desktop is, changing when it changes",
            Self::Dark => "Light text on dark panels",
            Self::Light => "Dark text on light panels",
            Self::Midnight => "Light text on deep blue panels",
            Self::Graphite => "Light text on neutral grey panels with a teal accent",
            Self::Paper => "Dark text on warm paper panels with a light 3D view",
            Self::User(_) => "A theme from your themes folder",
        }
    }

    pub fn key(&self) -> String {
        match self {
            Self::System => "system".to_owned(),
            Self::Dark => "dark".to_owned(),
            Self::Light => "light".to_owned(),
            Self::Midnight => "midnight".to_owned(),
            Self::Graphite => "graphite".to_owned(),
            Self::Paper => "paper".to_owned(),
            Self::User(key) => format!("{USER_PREFIX}{key}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        if let Some(user) = key.strip_prefix(USER_PREFIX) {
            return (!user.is_empty()).then(|| Self::User(user.to_owned()));
        }
        Self::SHIPPED.into_iter().find(|theme| theme.key() == key)
    }

    fn shipped(&self) -> Option<Look> {
        let look = |tokens, dark, canvas| Look {
            tokens,
            dark,
            canvas,
        };
        match self {
            Self::System | Self::Dark => Some(look(DARK, true, Canvas::Dark)),
            Self::Light => Some(look(LIGHT, false, Canvas::Dark)),
            Self::Midnight => Some(look(MIDNIGHT, true, Canvas::Dark)),
            Self::Graphite => Some(look(GRAPHITE, true, Canvas::Dark)),
            Self::Paper => Some(look(PAPER, false, Canvas::Light)),
            Self::User(_) => None,
        }
    }

    pub fn preview(&self, themes: &UserThemes) -> Option<Look> {
        match self {
            Self::User(key) => themes.find(key).map(UserTheme::look),
            shipped => shipped.shipped(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub tokens: Tokens,
    pub dark: bool,
    pub canvas: Canvas,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Accent {
    #[default]
    Theme,
    Blue,
    Teal,
    Green,
    Violet,
    Orange,
    Rose,
}

impl Accent {
    pub const ALL: [Self; 7] = [
        Self::Theme,
        Self::Blue,
        Self::Teal,
        Self::Green,
        Self::Violet,
        Self::Orange,
        Self::Rose,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "The theme's own accent",
            Self::Blue => "Blue",
            Self::Teal => "Teal",
            Self::Green => "Green",
            Self::Violet => "Violet",
            Self::Orange => "Orange",
            Self::Rose => "Rose",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Violet => "violet",
            Self::Orange => "orange",
            Self::Rose => "rose",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|accent| accent.key() == key)
    }

    fn hue(self) -> Option<Color32> {
        match self {
            Self::Theme => None,
            Self::Blue => Some(Color32::from_rgb(47, 111, 224)),
            Self::Teal => Some(Color32::from_rgb(13, 148, 136)),
            Self::Green => Some(Color32::from_rgb(34, 150, 80)),
            Self::Violet => Some(Color32::from_rgb(124, 77, 224)),
            Self::Orange => Some(Color32::from_rgb(224, 112, 24)),
            Self::Rose => Some(Color32::from_rgb(220, 60, 110)),
        }
    }

    pub fn applied(self, look: &Look) -> Option<Tokens> {
        let Some(hue) = self.hue() else {
            return Some(look.tokens);
        };
        let tokens = with_accent(&look.tokens, look.dark, hue);
        appearance::check(&tokens, false).ok().map(|()| tokens)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewChoice {
    #[default]
    Theme,
    Dark,
    Light,
}

impl ViewChoice {
    pub const ALL: [Self; 3] = [Self::Theme, Self::Dark, Self::Light];

    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "With the theme",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Theme => "The 3D view is dark or light as the chosen theme has it",
            Self::Dark => "A dark 3D view whatever the theme",
            Self::Light => "A light 3D view whatever the theme",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.key() == key)
    }

    fn canvas(self, theme: Canvas) -> Canvas {
        match self {
            Self::Theme => theme,
            Self::Dark => Canvas::Dark,
            Self::Light => Canvas::Light,
        }
    }
}

fn mix(from: Color32, to: Color32, share: f32) -> Color32 {
    let share = share.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * share)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}

fn nudged(start: Color32, toward: Color32, fits: impl Fn(Color32) -> bool) -> Color32 {
    (0..=ACCENT_STEPS)
        .map(|step| mix(start, toward, f32::from(step) * ACCENT_STEP))
        .find(|colour| fits(*colour))
        .unwrap_or(toward)
}

fn with_accent(base: &Tokens, dark: bool, hue: Color32) -> Tokens {
    let mut tokens = *base;
    let readable = |foreground: Color32, background: Color32| {
        appearance::contrast_ratio(foreground, background) >= READABLE
    };
    tokens.accent = nudged(hue, Color32::BLACK, |colour| {
        readable(tokens.text_on_accent, colour)
    });
    tokens.accent_hover = mix(tokens.accent, Color32::BLACK, HOVER_SHADE);
    tokens.accent_pressed = mix(tokens.accent, Color32::BLACK, PRESSED_SHADE);
    let (subtle, surface) = if dark {
        (DARK_SUBTLE_SHARE, DARK_SURFACE_SHARE)
    } else {
        (LIGHT_SUBTLE_SHARE, LIGHT_SURFACE_SHARE)
    };
    tokens.accent_subtle = nudged(mix(base.panel, hue, subtle), base.panel, |colour| {
        readable(base.text, colour) && readable(base.text_muted, colour)
    });
    tokens.accent_surface = nudged(mix(base.panel, hue, surface), base.panel, |colour| {
        readable(base.text, colour)
            && readable(base.text_muted, colour)
            && appearance::contrast_ratio(base.field_border, colour) >= VISIBLE_OUTLINE
    });
    let away = if dark { Color32::WHITE } else { Color32::BLACK };
    tokens.accent_text = nudged(hue, away, |colour| {
        [
            tokens.accent_subtle,
            tokens.accent_surface,
            base.panel,
            base.raised,
        ]
        .into_iter()
        .all(|background| readable(colour, background))
    });
    tokens.focus = tokens.accent_text;
    tokens
}

#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub theme: Theme,
    pub accent: Accent,
    pub view: ViewChoice,
    pub scale: f32,
    pub high_contrast: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            accent: Accent::default(),
            view: ViewChoice::default(),
            scale: 1.0,
            high_contrast: false,
        }
    }
}

impl Appearance {
    pub fn skin_of(&self, look: &Look) -> Skin {
        let canvas = self.view.canvas(look.canvas);
        if self.high_contrast {
            return Skin {
                canvas,
                ..Skin::built_in(look.dark, true)
            };
        }
        Skin {
            tokens: self.accent.applied(look).unwrap_or(look.tokens),
            dark: look.dark,
            high_contrast: false,
            canvas,
        }
    }

    pub fn look(&self, themes: &UserThemes) -> Result<Look, Unusable> {
        match &self.theme {
            Theme::User(key) => themes
                .find(key)
                .map(UserTheme::look)
                .ok_or_else(|| themes.why_not(key)),
            shipped => shipped.shipped().ok_or_else(|| themes.why_not("")),
        }
    }

    pub fn resolve(&self, themes: &UserThemes) -> Result<Resolved, Unusable> {
        let look = self.look(themes)?;
        Ok(self.resolve_look(&look, self.theme == Theme::System))
    }

    pub fn resolve_look(&self, look: &Look, system: bool) -> Resolved {
        let chosen = self.skin_of(look);
        let skins = if system {
            Skins {
                dark: chosen,
                light: self.skin_of(&Look {
                    tokens: LIGHT,
                    dark: false,
                    canvas: Canvas::Dark,
                }),
            }
        } else {
            Skins {
                dark: chosen,
                light: chosen,
            }
        };
        let preference = if system {
            ThemePreference::System
        } else if look.dark {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        };
        Resolved { skins, preference }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub skins: Skins,
    pub preference: ThemePreference,
}

#[derive(Debug)]
pub enum Unusable {
    NotListedYet,
    Missing {
        key: String,
        folder: Option<PathBuf>,
    },
    Refused(String),
}

impl Unusable {
    pub fn words(&self, name: &str) -> Option<String> {
        match self {
            Self::NotListedYet => None,
            Self::Missing { key, folder } => Some(format!(
                "The theme “{name}” could not be used: there is no {key}.json in {}. The \
                 previous theme stays.",
                folder.as_deref().map_or_else(
                    || "a themes folder".to_owned(),
                    |folder| folder.display().to_string()
                )
            )),
            Self::Refused(reason) => Some(format!(
                "The theme “{name}” could not be used: {reason}. The previous theme stays."
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserTheme {
    pub key: String,
    pub name: String,
    pub dark: bool,
    pub canvas: Canvas,
    pub tokens: Tokens,
}

impl UserTheme {
    pub fn look(&self) -> Look {
        Look {
            tokens: self.tokens,
            dark: self.dark,
            canvas: self.canvas,
        }
    }
}

#[derive(Debug)]
pub enum ThemeProblem {
    File(ThemeFileError),
    UnknownBase(String),
    UnknownView(String),
    UnknownColour(String),
    BadColour { key: String, value: String },
    Translucent { key: String, value: String },
    Repeated,
    Contrast(ContrastFailure),
}

impl ThemeProblem {
    pub fn words(&self) -> String {
        match self {
            Self::File(error) => error.to_string(),
            Self::UnknownBase(base) => {
                format!("its base “{base}” is neither “dark” nor “light”")
            }
            Self::UnknownView(view) => {
                format!("its view “{view}” is neither “dark” nor “light”")
            }
            Self::UnknownColour(key) => format!(
                "it names a colour “{key}” caditor does not have; the colours are {}",
                Token::ALL.map(Token::key).join(", ")
            ),
            Self::BadColour { key, value } => {
                format!("its {key} “{value}” is not a colour written as #rrggbb")
            }
            Self::Translucent { key, value } => {
                format!("its {key} “{value}” is see-through, and only the shadow may be")
            }
            Self::Repeated => "another theme file has the same name".to_owned(),
            Self::Contrast(failure) => failure.words(),
        }
    }
}

#[derive(Debug)]
pub struct Refusal {
    pub file: String,
    pub problem: ThemeProblem,
}

impl Refusal {
    pub fn words(&self) -> String {
        format!("{} was not loaded: {}.", self.file, self.problem.words())
    }
}

fn parse_colour(token: Token, value: &str) -> Result<Color32, ThemeProblem> {
    let bad = || ThemeProblem::BadColour {
        key: token.key().to_owned(),
        value: value.to_owned(),
    };
    let digits = value.strip_prefix('#').ok_or_else(bad)?;
    let byte = |index: usize| {
        digits
            .get(index..index + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
    };
    let colour = match digits.len() {
        6 => Color32::from_rgb(
            byte(0).ok_or_else(bad)?,
            byte(2).ok_or_else(bad)?,
            byte(4).ok_or_else(bad)?,
        ),
        8 => Color32::from_rgba_unmultiplied(
            byte(0).ok_or_else(bad)?,
            byte(2).ok_or_else(bad)?,
            byte(4).ok_or_else(bad)?,
            byte(6).ok_or_else(bad)?,
        ),
        _ => return Err(bad()),
    };
    if colour.a() < u8::MAX && !token.may_be_translucent() {
        return Err(ThemeProblem::Translucent {
            key: token.key().to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(colour)
}

fn lightness(
    value: Option<&str>,
    unknown: fn(String) -> ThemeProblem,
) -> Result<Option<bool>, ThemeProblem> {
    match value {
        None => Ok(None),
        Some("dark") => Ok(Some(true)),
        Some("light") => Ok(Some(false)),
        Some(other) => Err(unknown(other.to_owned())),
    }
}

pub fn build(key: &str, file: &ThemeFile) -> Result<UserTheme, ThemeProblem> {
    let dark = lightness(file.base.as_deref(), ThemeProblem::UnknownBase)?.unwrap_or(true);
    let canvas = match lightness(file.view.as_deref(), ThemeProblem::UnknownView)? {
        Some(false) => Canvas::Light,
        Some(true) | None => Canvas::Dark,
    };
    let mut tokens = if dark { DARK } else { LIGHT };
    for (name, value) in &file.colours {
        let token =
            Token::from_key(name).ok_or_else(|| ThemeProblem::UnknownColour(name.clone()))?;
        token.set(&mut tokens, parse_colour(token, value)?);
    }
    appearance::check(&tokens, false).map_err(ThemeProblem::Contrast)?;
    let name = file
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(key)
        .to_owned();
    Ok(UserTheme {
        key: key.to_owned(),
        name,
        dark,
        canvas,
        tokens,
    })
}

pub fn sort(read: Vec<ReadTheme>) -> (Vec<UserTheme>, Vec<Refusal>) {
    let mut themes: Vec<UserTheme> = Vec::new();
    let mut refused = Vec::new();
    for theme in read {
        let key = theme.key();
        let file = theme.file_name();
        let built = match theme.result {
            Err(error) => Err(ThemeProblem::File(error)),
            Ok(_) if themes.iter().any(|kept| kept.key == key) => Err(ThemeProblem::Repeated),
            Ok(read) => build(&key, &read),
        };
        match built {
            Ok(built) => themes.push(built),
            Err(problem) => refused.push(Refusal { file, problem }),
        }
    }
    (themes, refused)
}

#[derive(Debug, Default)]
pub struct UserThemes {
    folder: Option<PathBuf>,
    themes: Vec<UserTheme>,
    refused: Vec<Refusal>,
    listed: bool,
    generation: u64,
}

impl UserThemes {
    pub fn in_config(config_dir: Option<&Path>) -> Self {
        Self {
            folder: config_dir.map(caditor_file::themes_folder),
            ..Self::default()
        }
    }

    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    pub fn themes(&self) -> &[UserTheme] {
        &self.themes
    }

    pub fn refused(&self) -> &[Refusal] {
        &self.refused
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn find(&self, key: &str) -> Option<&UserTheme> {
        self.themes.iter().find(|theme| theme.key == key)
    }

    pub fn name_of(&self, theme: &Theme) -> String {
        match theme {
            Theme::User(key) => self
                .find(key)
                .map_or_else(|| key.clone(), |theme| theme.name.clone()),
            shipped => shipped.label(),
        }
    }

    pub fn listed(&mut self, themes: Vec<UserTheme>, refused: Vec<Refusal>) {
        self.themes = themes;
        self.refused = refused;
        self.listed = true;
        self.generation = self.generation.wrapping_add(1);
    }

    fn why_not(&self, key: &str) -> Unusable {
        if !self.listed {
            return Unusable::NotListedYet;
        }
        let file = format!("{key}.json");
        match self
            .refused
            .iter()
            .find(|refusal| refusal.file.eq_ignore_ascii_case(&file))
        {
            Some(refusal) => Unusable::Refused(refusal.problem.words()),
            None => Unusable::Missing {
                key: key.to_owned(),
                folder: self.folder.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use caditor_file::parse_theme;

    use super::*;

    fn file(base: &str, colours: &[(&str, &str)]) -> ThemeFile {
        ThemeFile {
            name: Some("Ocean".to_owned()),
            base: Some(base.to_owned()),
            view: None,
            colours: colours
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn shipped_themes_keep_their_keys_and_older_preference_values_read_back() {
        for theme in Theme::SHIPPED {
            assert_eq!(Theme::from_key(&theme.key()), Some(theme));
        }
        assert_eq!(Theme::from_key("light"), Some(Theme::Light));
        assert_eq!(
            Theme::from_key("user:ocean"),
            Some(Theme::User("ocean".to_owned()))
        );
        assert_eq!(Theme::from_key("user:"), None);
        assert_eq!(Theme::from_key("sepia"), None);
        for accent in Accent::ALL {
            assert_eq!(Accent::from_key(accent.key()), Some(accent));
        }
        for view in ViewChoice::ALL {
            assert_eq!(ViewChoice::from_key(view.key()), Some(view));
        }
    }

    #[test]
    fn every_accent_on_every_shipped_theme_passes_the_contrast_checks() {
        for theme in Theme::SHIPPED {
            let look = theme.shipped().unwrap();
            let mut fills = Vec::new();
            for accent in Accent::ALL {
                if let Some(hue) = accent.hue()
                    && let Err(failure) =
                        appearance::check(&with_accent(&look.tokens, look.dark, hue), false)
                {
                    panic!("{accent:?} on {theme:?}: {}", failure.words());
                }
                let tokens = accent.applied(&look).unwrap();
                assert!(appearance::check(&tokens, false).is_ok());
                if accent != Accent::Theme {
                    assert!(!fills.contains(&tokens.accent), "{accent:?} on {theme:?}");
                    fills.push(tokens.accent);
                }
            }
        }
    }

    #[test]
    fn a_theme_file_overrides_its_base_and_names_itself() {
        let built = build(
            "ocean",
            &file(
                "light",
                &[
                    ("panel", "#f0f6fa"),
                    ("accent", "#1f5fbf"),
                    ("shadow", "#00000030"),
                ],
            ),
        )
        .unwrap();

        assert_eq!(built.name, "Ocean");
        assert!(!built.dark);
        assert_eq!(built.canvas, Canvas::Dark);
        assert_eq!(built.tokens.panel, Color32::from_rgb(0xf0, 0xf6, 0xfa));
        assert_eq!(built.tokens.text, LIGHT.text);
        assert_eq!(built.tokens.shadow.a(), 0x30);
    }

    #[test]
    fn a_theme_failing_the_contrast_checks_is_refused_naming_the_colour_pair() {
        let problem = build("murky", &file("dark", &[("text_muted", "#3c3e46")])).unwrap_err();

        let words = problem.words();
        assert!(matches!(problem, ThemeProblem::Contrast(_)));
        assert!(words.contains("text_muted #3c3e46"), "{words}");
        assert!(words.contains("below the 4.5:1 it needs"), "{words}");
    }

    #[test]
    fn unknown_names_bad_colours_and_damaged_files_are_refused_in_words() {
        let unknown = build("a", &file("dark", &[("background", "#000000")])).unwrap_err();
        let bad = build("b", &file("dark", &[("panel", "black")])).unwrap_err();
        let translucent = build("c", &file("dark", &[("panel", "#00000080")])).unwrap_err();
        let base = build("d", &file("sepia", &[])).unwrap_err();

        assert!(unknown.words().contains("“background”"));
        assert!(unknown.words().contains("text_muted"));
        assert_eq!(
            bad.words(),
            "its panel “black” is not a colour written as #rrggbb"
        );
        assert!(translucent.words().contains("see-through"));
        assert!(base.words().contains("“sepia”"));

        let damaged = ReadTheme {
            path: PathBuf::from("themes/broken.json"),
            result: parse_theme(b"{ nope"),
        };
        let repeated = ReadTheme {
            path: PathBuf::from("themes/Ocean.JSON"),
            result: Ok(file("dark", &[])),
        };
        let ocean = ReadTheme {
            path: PathBuf::from("themes/Ocean.json"),
            result: Ok(file("dark", &[])),
        };
        let (themes, refused) = sort(vec![damaged, ocean, repeated]);

        assert_eq!(themes.len(), 1);
        assert_eq!(refused.len(), 2);
        assert!(
            refused[0]
                .words()
                .starts_with("broken.json was not loaded: it is not a theme caditor understands")
        );
        assert_eq!(
            refused[1].words(),
            "Ocean.JSON was not loaded: another theme file has the same name."
        );
    }

    #[test]
    fn a_chosen_theme_that_cannot_be_used_says_why_once_the_folder_was_read() {
        let mut themes = UserThemes::in_config(Some(Path::new("config")));
        let appearance = Appearance {
            theme: Theme::User("murky".to_owned()),
            ..Appearance::default()
        };

        assert!(matches!(
            appearance.resolve(&themes),
            Err(Unusable::NotListedYet)
        ));

        let (kept, refused) = sort(vec![ReadTheme {
            path: PathBuf::from("config/themes/murky.json"),
            result: Ok(file("dark", &[("text_muted", "#3c3e46")])),
        }]);
        themes.listed(kept, refused);
        let words = appearance
            .resolve(&themes)
            .unwrap_err()
            .words("murky")
            .unwrap();

        assert!(words.starts_with("The theme “murky” could not be used: the muted text"));
        assert!(words.ends_with("The previous theme stays."));

        themes.listed(Vec::new(), Vec::new());
        let missing = appearance
            .resolve(&themes)
            .unwrap_err()
            .words("murky")
            .unwrap();
        assert!(
            missing.contains("there is no murky.json in config"),
            "{missing}"
        );
    }

    #[test]
    fn high_contrast_and_the_view_choice_shape_the_skin_of_any_theme() {
        let paper = Theme::Paper.shipped().unwrap();
        let mut appearance = Appearance {
            theme: Theme::Paper,
            accent: Accent::Teal,
            ..Appearance::default()
        };

        let skin = appearance.skin_of(&paper);
        assert_eq!(skin.canvas, Canvas::Light);
        assert!(!skin.dark);
        assert_ne!(skin.tokens.accent, PAPER.accent);

        appearance.view = ViewChoice::Dark;
        appearance.high_contrast = true;
        let skin = appearance.skin_of(&paper);
        assert_eq!(skin.canvas, Canvas::Dark);
        assert!(skin.high_contrast);
        assert_eq!(skin.tokens, appearance::LIGHT_HIGH_CONTRAST);

        let resolved = Appearance::default()
            .resolve(&UserThemes::default())
            .unwrap();
        assert_eq!(resolved.preference, ThemePreference::System);
        assert_eq!(resolved.skins.dark.tokens, DARK);
        assert_eq!(resolved.skins.light.tokens, LIGHT);
    }
}
