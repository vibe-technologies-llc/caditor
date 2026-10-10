use std::collections::BTreeMap;

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Id, Margin, Shadow, Stroke, Style, TextStyle, Ui,
    Visuals, style::ScrollStyle, vec2,
};

use crate::{fonts, scene_palette::Canvas};

pub const MIN_SCALE: f32 = 0.75;
pub const MAX_SCALE: f32 = 2.0;
pub const SCALE_STEP: f32 = 0.125;

pub const WIDGET_RADIUS: u8 = 6;
pub const CARD_RADIUS: u8 = 8;
pub const WINDOW_RADIUS: u8 = 10;
pub const BODY_SIZE: f32 = 13.5;
pub const SMALL_SIZE: f32 = 11.5;
pub const HEADING_SIZE: f32 = 17.0;
const SECTION_SIZE: f32 = 12.5;
pub const ICON_SIZE: f32 = 16.0;
pub const TOOL_ICON_SIZE: f32 = 20.0;
pub const SECTION: &str = "section";

pub const BORDER_WIDTH: f32 = 1.0;
pub const FOCUS_WIDTH: f32 = 2.0;
pub const CONTROL_HEIGHT: f32 = 24.0;
pub const SPACE_XS: f32 = 2.0;
pub const SPACE_S: f32 = 4.0;
pub const SPACE_M: f32 = 8.0;
pub const SPACE_L: f32 = 12.0;
pub const DIALOG_MARGIN: i8 = 20;
const HIGH_CONTRAST_DISABLED_ALPHA: f32 = 0.7;
const DISABLED_ALPHA: f32 = 0.5;
const MONOSPACE_SIZE: f32 = 12.5;
const SKINS_KEY: &str = "appearance-skins";

pub const READABLE: f32 = 4.5;
pub const HIGHLY_READABLE: f32 = 7.0;
pub const VISIBLE_OUTLINE: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub panel: Color32,
    pub raised: Color32,
    pub sunken: Color32,
    pub stripe: Color32,
    pub button: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub field_border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_on_accent: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    pub accent_text: Color32,
    pub accent_subtle: Color32,
    pub accent_surface: Color32,
    pub focus: Color32,
    pub error: Color32,
    pub error_subtle: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    pub danger_pressed: Color32,
    pub warn: Color32,
    pub warn_subtle: Color32,
    pub success: Color32,
    pub success_subtle: Color32,
    pub shadow: Color32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Panel,
    Raised,
    Sunken,
    Stripe,
    Button,
    Hover,
    Pressed,
    Border,
    BorderStrong,
    FieldBorder,
    Text,
    TextMuted,
    TextOnAccent,
    Accent,
    AccentHover,
    AccentPressed,
    AccentText,
    AccentSubtle,
    AccentSurface,
    Focus,
    Error,
    ErrorSubtle,
    Danger,
    DangerHover,
    DangerPressed,
    Warn,
    WarnSubtle,
    Success,
    SuccessSubtle,
    Shadow,
}

impl Token {
    pub const ALL: [Self; 30] = [
        Self::Panel,
        Self::Raised,
        Self::Sunken,
        Self::Stripe,
        Self::Button,
        Self::Hover,
        Self::Pressed,
        Self::Border,
        Self::BorderStrong,
        Self::FieldBorder,
        Self::Text,
        Self::TextMuted,
        Self::TextOnAccent,
        Self::Accent,
        Self::AccentHover,
        Self::AccentPressed,
        Self::AccentText,
        Self::AccentSubtle,
        Self::AccentSurface,
        Self::Focus,
        Self::Error,
        Self::ErrorSubtle,
        Self::Danger,
        Self::DangerHover,
        Self::DangerPressed,
        Self::Warn,
        Self::WarnSubtle,
        Self::Success,
        Self::SuccessSubtle,
        Self::Shadow,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Raised => "raised",
            Self::Sunken => "sunken",
            Self::Stripe => "stripe",
            Self::Button => "button",
            Self::Hover => "hover",
            Self::Pressed => "pressed",
            Self::Border => "border",
            Self::BorderStrong => "border_strong",
            Self::FieldBorder => "field_border",
            Self::Text => "text",
            Self::TextMuted => "text_muted",
            Self::TextOnAccent => "text_on_accent",
            Self::Accent => "accent",
            Self::AccentHover => "accent_hover",
            Self::AccentPressed => "accent_pressed",
            Self::AccentText => "accent_text",
            Self::AccentSubtle => "accent_subtle",
            Self::AccentSurface => "accent_surface",
            Self::Focus => "focus",
            Self::Error => "error",
            Self::ErrorSubtle => "error_subtle",
            Self::Danger => "danger",
            Self::DangerHover => "danger_hover",
            Self::DangerPressed => "danger_pressed",
            Self::Warn => "warn",
            Self::WarnSubtle => "warn_subtle",
            Self::Success => "success",
            Self::SuccessSubtle => "success_subtle",
            Self::Shadow => "shadow",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|token| token.key() == key)
    }

    pub fn may_be_translucent(self) -> bool {
        self == Self::Shadow
    }

    pub fn of(self, tokens: &Tokens) -> Color32 {
        let mut copy = *tokens;
        *self.slot(&mut copy)
    }

    pub fn set(self, tokens: &mut Tokens, colour: Color32) {
        *self.slot(tokens) = colour;
    }

    fn slot(self, tokens: &mut Tokens) -> &mut Color32 {
        match self {
            Self::Panel => &mut tokens.panel,
            Self::Raised => &mut tokens.raised,
            Self::Sunken => &mut tokens.sunken,
            Self::Stripe => &mut tokens.stripe,
            Self::Button => &mut tokens.button,
            Self::Hover => &mut tokens.hover,
            Self::Pressed => &mut tokens.pressed,
            Self::Border => &mut tokens.border,
            Self::BorderStrong => &mut tokens.border_strong,
            Self::FieldBorder => &mut tokens.field_border,
            Self::Text => &mut tokens.text,
            Self::TextMuted => &mut tokens.text_muted,
            Self::TextOnAccent => &mut tokens.text_on_accent,
            Self::Accent => &mut tokens.accent,
            Self::AccentHover => &mut tokens.accent_hover,
            Self::AccentPressed => &mut tokens.accent_pressed,
            Self::AccentText => &mut tokens.accent_text,
            Self::AccentSubtle => &mut tokens.accent_subtle,
            Self::AccentSurface => &mut tokens.accent_surface,
            Self::Focus => &mut tokens.focus,
            Self::Error => &mut tokens.error,
            Self::ErrorSubtle => &mut tokens.error_subtle,
            Self::Danger => &mut tokens.danger,
            Self::DangerHover => &mut tokens.danger_hover,
            Self::DangerPressed => &mut tokens.danger_pressed,
            Self::Warn => &mut tokens.warn,
            Self::WarnSubtle => &mut tokens.warn_subtle,
            Self::Success => &mut tokens.success,
            Self::SuccessSubtle => &mut tokens.success_subtle,
            Self::Shadow => &mut tokens.shadow,
        }
    }

    fn words(self) -> &'static str {
        match self {
            Self::Panel => "the panel",
            Self::Raised => "a card or dialog",
            Self::Sunken => "a field",
            Self::Stripe => "a striped row",
            Self::Button => "a button",
            Self::Hover => "a hovered button",
            Self::Pressed => "a pressed button",
            Self::Border => "the border",
            Self::BorderStrong => "the strong border",
            Self::FieldBorder => "the field outline",
            Self::Text => "the text",
            Self::TextMuted => "the muted text",
            Self::TextOnAccent => "the text on accent buttons",
            Self::Accent => "the accent",
            Self::AccentHover => "the hovered accent",
            Self::AccentPressed => "the pressed accent",
            Self::AccentText => "the accent text",
            Self::AccentSubtle => "a selected row",
            Self::AccentSurface => "the accent surface",
            Self::Focus => "the focus outline",
            Self::Error => "the error text",
            Self::ErrorSubtle => "an error callout",
            Self::Danger => "a destructive button",
            Self::DangerHover => "a hovered destructive button",
            Self::DangerPressed => "a pressed destructive button",
            Self::Warn => "the warning text",
            Self::WarnSubtle => "a warning callout",
            Self::Success => "the success text",
            Self::SuccessSubtle => "a success callout",
            Self::Shadow => "the shadow",
        }
    }
}

pub const DARK: Tokens = Tokens {
    panel: Color32::from_rgb(22, 24, 29),
    raised: Color32::from_rgb(30, 33, 40),
    sunken: Color32::from_rgb(15, 17, 21),
    stripe: Color32::from_rgb(27, 30, 36),
    button: Color32::from_rgb(38, 42, 51),
    hover: Color32::from_rgb(47, 52, 64),
    pressed: Color32::from_rgb(55, 61, 74),
    border: Color32::from_rgb(42, 46, 55),
    border_strong: Color32::from_rgb(70, 77, 91),
    field_border: Color32::from_rgb(104, 112, 128),
    text: Color32::from_rgb(230, 232, 236),
    text_muted: Color32::from_rgb(155, 162, 174),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(47, 111, 224),
    accent_hover: Color32::from_rgb(40, 97, 204),
    accent_pressed: Color32::from_rgb(33, 83, 178),
    accent_text: Color32::from_rgb(122, 167, 255),
    accent_subtle: Color32::from_rgb(31, 49, 80),
    accent_surface: Color32::from_rgb(26, 36, 54),
    focus: Color32::from_rgb(122, 167, 255),
    error: Color32::from_rgb(255, 123, 114),
    error_subtle: Color32::from_rgb(58, 31, 34),
    danger: Color32::from_rgb(210, 48, 52),
    danger_hover: Color32::from_rgb(184, 38, 43),
    danger_pressed: Color32::from_rgb(150, 26, 32),
    warn: Color32::from_rgb(240, 181, 74),
    warn_subtle: Color32::from_rgb(56, 45, 22),
    success: Color32::from_rgb(95, 208, 138),
    success_subtle: Color32::from_rgb(23, 50, 38),
    shadow: Color32::from_black_alpha(110),
};

pub const LIGHT: Tokens = Tokens {
    panel: Color32::from_rgb(244, 245, 247),
    raised: Color32::WHITE,
    sunken: Color32::from_rgb(252, 252, 253),
    stripe: Color32::from_rgb(237, 239, 242),
    button: Color32::from_rgb(232, 234, 238),
    hover: Color32::from_rgb(222, 225, 231),
    pressed: Color32::from_rgb(210, 214, 222),
    border: Color32::from_rgb(221, 224, 230),
    border_strong: Color32::from_rgb(170, 176, 188),
    field_border: Color32::from_rgb(128, 135, 148),
    text: Color32::from_rgb(26, 29, 35),
    text_muted: Color32::from_rgb(90, 97, 112),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(37, 99, 235),
    accent_hover: Color32::from_rgb(29, 84, 212),
    accent_pressed: Color32::from_rgb(24, 70, 182),
    accent_text: Color32::from_rgb(29, 78, 216),
    accent_subtle: Color32::from_rgb(221, 232, 253),
    accent_surface: Color32::from_rgb(228, 236, 251),
    focus: Color32::from_rgb(37, 99, 235),
    error: Color32::from_rgb(192, 38, 45),
    error_subtle: Color32::from_rgb(253, 232, 232),
    danger: Color32::from_rgb(196, 36, 44),
    danger_hover: Color32::from_rgb(170, 28, 36),
    danger_pressed: Color32::from_rgb(146, 22, 30),
    warn: Color32::from_rgb(154, 91, 0),
    warn_subtle: Color32::from_rgb(253, 242, 220),
    success: Color32::from_rgb(19, 119, 61),
    success_subtle: Color32::from_rgb(225, 244, 232),
    shadow: Color32::from_black_alpha(40),
};

pub const MIDNIGHT: Tokens = Tokens {
    panel: Color32::from_rgb(14, 20, 36),
    raised: Color32::from_rgb(20, 28, 48),
    sunken: Color32::from_rgb(9, 13, 25),
    stripe: Color32::from_rgb(18, 25, 43),
    button: Color32::from_rgb(28, 38, 62),
    hover: Color32::from_rgb(36, 48, 78),
    pressed: Color32::from_rgb(44, 58, 92),
    border: Color32::from_rgb(32, 42, 66),
    border_strong: Color32::from_rgb(60, 74, 108),
    field_border: Color32::from_rgb(98, 114, 150),
    text: Color32::from_rgb(226, 232, 244),
    text_muted: Color32::from_rgb(156, 168, 192),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(54, 92, 214),
    accent_hover: Color32::from_rgb(46, 80, 192),
    accent_pressed: Color32::from_rgb(38, 68, 168),
    accent_text: Color32::from_rgb(138, 170, 255),
    accent_subtle: Color32::from_rgb(30, 44, 86),
    accent_surface: Color32::from_rgb(22, 32, 62),
    focus: Color32::from_rgb(138, 170, 255),
    error: Color32::from_rgb(255, 128, 120),
    error_subtle: Color32::from_rgb(52, 24, 34),
    danger: Color32::from_rgb(210, 48, 52),
    danger_hover: Color32::from_rgb(184, 38, 43),
    danger_pressed: Color32::from_rgb(150, 26, 32),
    warn: Color32::from_rgb(240, 186, 80),
    warn_subtle: Color32::from_rgb(50, 40, 24),
    success: Color32::from_rgb(100, 212, 144),
    success_subtle: Color32::from_rgb(20, 44, 40),
    shadow: Color32::from_black_alpha(120),
};

pub const GRAPHITE: Tokens = Tokens {
    panel: Color32::from_rgb(32, 32, 34),
    raised: Color32::from_rgb(40, 40, 43),
    sunken: Color32::from_rgb(24, 24, 26),
    stripe: Color32::from_rgb(37, 37, 40),
    button: Color32::from_rgb(50, 50, 54),
    hover: Color32::from_rgb(60, 60, 65),
    pressed: Color32::from_rgb(70, 70, 76),
    border: Color32::from_rgb(54, 54, 58),
    border_strong: Color32::from_rgb(84, 84, 90),
    field_border: Color32::from_rgb(120, 120, 128),
    text: Color32::from_rgb(234, 234, 236),
    text_muted: Color32::from_rgb(174, 174, 180),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(14, 116, 104),
    accent_hover: Color32::from_rgb(10, 102, 92),
    accent_pressed: Color32::from_rgb(8, 88, 80),
    accent_text: Color32::from_rgb(96, 214, 196),
    accent_subtle: Color32::from_rgb(28, 62, 58),
    accent_surface: Color32::from_rgb(30, 46, 45),
    focus: Color32::from_rgb(96, 214, 196),
    error: Color32::from_rgb(255, 134, 124),
    error_subtle: Color32::from_rgb(62, 34, 34),
    danger: Color32::from_rgb(214, 50, 54),
    danger_hover: Color32::from_rgb(180, 38, 42),
    danger_pressed: Color32::from_rgb(146, 26, 30),
    warn: Color32::from_rgb(240, 190, 86),
    warn_subtle: Color32::from_rgb(60, 50, 28),
    success: Color32::from_rgb(110, 214, 142),
    success_subtle: Color32::from_rgb(30, 54, 40),
    shadow: Color32::from_black_alpha(110),
};

pub const PAPER: Tokens = Tokens {
    panel: Color32::from_rgb(246, 243, 236),
    raised: Color32::from_rgb(253, 251, 246),
    sunken: Color32::from_rgb(255, 254, 251),
    stripe: Color32::from_rgb(238, 234, 225),
    button: Color32::from_rgb(234, 229, 218),
    hover: Color32::from_rgb(224, 218, 205),
    pressed: Color32::from_rgb(212, 205, 190),
    border: Color32::from_rgb(224, 218, 206),
    border_strong: Color32::from_rgb(176, 168, 152),
    field_border: Color32::from_rgb(132, 124, 110),
    text: Color32::from_rgb(38, 32, 24),
    text_muted: Color32::from_rgb(96, 86, 72),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(176, 82, 24),
    accent_hover: Color32::from_rgb(156, 70, 18),
    accent_pressed: Color32::from_rgb(136, 60, 14),
    accent_text: Color32::from_rgb(150, 62, 10),
    accent_subtle: Color32::from_rgb(250, 226, 206),
    accent_surface: Color32::from_rgb(250, 236, 222),
    focus: Color32::from_rgb(176, 82, 24),
    error: Color32::from_rgb(176, 30, 30),
    error_subtle: Color32::from_rgb(252, 228, 224),
    danger: Color32::from_rgb(190, 34, 40),
    danger_hover: Color32::from_rgb(166, 26, 32),
    danger_pressed: Color32::from_rgb(142, 20, 26),
    warn: Color32::from_rgb(138, 82, 0),
    warn_subtle: Color32::from_rgb(250, 238, 212),
    success: Color32::from_rgb(20, 112, 56),
    success_subtle: Color32::from_rgb(226, 242, 226),
    shadow: Color32::from_black_alpha(40),
};

pub const DARK_HIGH_CONTRAST: Tokens = Tokens {
    panel: Color32::BLACK,
    raised: Color32::BLACK,
    sunken: Color32::BLACK,
    stripe: Color32::from_gray(20),
    button: Color32::from_gray(38),
    hover: Color32::from_gray(60),
    pressed: Color32::from_gray(60),
    border: Color32::from_gray(170),
    border_strong: Color32::from_gray(200),
    field_border: Color32::from_gray(200),
    text: Color32::WHITE,
    text_muted: Color32::from_gray(205),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(0, 80, 170),
    accent_hover: Color32::from_rgb(0, 68, 150),
    accent_pressed: Color32::from_rgb(0, 56, 128),
    accent_text: Color32::from_rgb(150, 205, 255),
    accent_subtle: Color32::from_rgb(0, 50, 110),
    accent_surface: Color32::from_rgb(0, 22, 50),
    focus: Color32::from_rgb(255, 214, 0),
    error: Color32::from_rgb(255, 150, 140),
    error_subtle: Color32::from_rgb(60, 0, 0),
    danger: Color32::from_rgb(190, 0, 0),
    danger_hover: Color32::from_rgb(160, 0, 0),
    danger_pressed: Color32::from_rgb(130, 0, 0),
    warn: Color32::from_rgb(255, 205, 80),
    warn_subtle: Color32::from_rgb(50, 36, 0),
    success: Color32::from_rgb(120, 230, 150),
    success_subtle: Color32::from_rgb(0, 45, 15),
    shadow: Color32::TRANSPARENT,
};

pub const LIGHT_HIGH_CONTRAST: Tokens = Tokens {
    panel: Color32::WHITE,
    raised: Color32::WHITE,
    sunken: Color32::WHITE,
    stripe: Color32::from_gray(240),
    button: Color32::from_gray(232),
    hover: Color32::from_gray(212),
    pressed: Color32::from_gray(212),
    border: Color32::from_gray(70),
    border_strong: Color32::from_gray(40),
    field_border: Color32::from_gray(40),
    text: Color32::BLACK,
    text_muted: Color32::from_gray(50),
    text_on_accent: Color32::WHITE,
    accent: Color32::from_rgb(0, 70, 170),
    accent_hover: Color32::from_rgb(0, 58, 148),
    accent_pressed: Color32::from_rgb(0, 48, 126),
    accent_text: Color32::from_rgb(0, 60, 150),
    accent_subtle: Color32::from_rgb(214, 228, 255),
    accent_surface: Color32::from_rgb(238, 244, 255),
    focus: Color32::from_rgb(0, 70, 200),
    error: Color32::from_rgb(150, 0, 0),
    error_subtle: Color32::from_rgb(255, 228, 228),
    danger: Color32::from_rgb(160, 0, 0),
    danger_hover: Color32::from_rgb(135, 0, 0),
    danger_pressed: Color32::from_rgb(110, 0, 0),
    warn: Color32::from_rgb(115, 55, 0),
    warn_subtle: Color32::from_rgb(255, 240, 210),
    success: Color32::from_rgb(0, 90, 25),
    success_subtle: Color32::from_rgb(222, 245, 228),
    shadow: Color32::TRANSPARENT,
};

impl Tokens {
    pub fn of(dark: bool, high_contrast: bool) -> &'static Self {
        match (dark, high_contrast) {
            (true, false) => &DARK,
            (false, false) => &LIGHT,
            (true, true) => &DARK_HIGH_CONTRAST,
            (false, true) => &LIGHT_HIGH_CONTRAST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Skin {
    pub tokens: Tokens,
    pub dark: bool,
    pub high_contrast: bool,
    pub canvas: Canvas,
}

impl Skin {
    pub fn built_in(dark: bool, high_contrast: bool) -> Self {
        Self {
            tokens: *Tokens::of(dark, high_contrast),
            dark,
            high_contrast,
            canvas: Canvas::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Skins {
    pub dark: Skin,
    pub light: Skin,
}

impl Skins {
    pub fn of(&self, dark_mode: bool) -> &Skin {
        if dark_mode { &self.dark } else { &self.light }
    }
}

pub fn publish(ctx: &egui::Context, skins: Skins) {
    ctx.data_mut(|data| data.insert_temp(Id::new(SKINS_KEY), skins));
}

pub fn skin_for(ctx: &egui::Context, visuals: &Visuals) -> Skin {
    ctx.data(|data| data.get_temp::<Skins>(Id::new(SKINS_KEY)))
        .map_or_else(
            || {
                let high_contrast = Tokens::of(visuals.dark_mode, true).panel == visuals.panel_fill;
                Skin::built_in(visuals.dark_mode, high_contrast)
            },
            |skins| *skins.of(visuals.dark_mode),
        )
}

pub fn skin(ctx: &egui::Context) -> Skin {
    skin_for(ctx, &ctx.global_style().visuals)
}

pub fn tokens(ui: &Ui) -> Tokens {
    skin_for(ui.ctx(), ui.visuals()).tokens
}

pub fn tokens_of(ctx: &egui::Context) -> Tokens {
    skin(ctx).tokens
}

pub fn is_high_contrast(ui: &Ui) -> bool {
    skin_for(ui.ctx(), ui.visuals()).high_contrast
}

pub fn visuals(skin: &Skin) -> Visuals {
    let tokens = &skin.tokens;
    let high_contrast = skin.high_contrast;
    let mut visuals = if skin.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let border = Stroke::new(BORDER_WIDTH, tokens.border);
    visuals.panel_fill = tokens.panel;
    visuals.window_fill = tokens.raised;
    visuals.extreme_bg_color = tokens.sunken;
    visuals.text_edit_bg_color = Some(tokens.sunken);
    visuals.code_bg_color = tokens.stripe;
    visuals.faint_bg_color = tokens.stripe;
    visuals.window_stroke = border;
    visuals.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: tokens.shadow,
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: tokens.shadow,
    };
    visuals.weak_text_color = Some(tokens.text_muted);
    visuals.error_fg_color = tokens.error;
    visuals.warn_fg_color = tokens.warn;
    visuals.hyperlink_color = tokens.accent_text;
    visuals.selection.bg_fill = tokens.accent_subtle;
    visuals.selection.stroke = Stroke::new(FOCUS_WIDTH, tokens.accent_text);
    visuals.slider_trailing_fill = true;
    visuals.text_cursor.blink = false;
    visuals.indent_has_left_vline = false;
    visuals.disabled_alpha = if high_contrast {
        HIGH_CONTRAST_DISABLED_ALPHA
    } else {
        DISABLED_ALPHA
    };

    let radius = CornerRadius::same(WIDGET_RADIUS);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = tokens.panel;
    widgets.noninteractive.weak_bg_fill = tokens.panel;
    widgets.noninteractive.bg_stroke = border;
    widgets.noninteractive.fg_stroke = Stroke::new(BORDER_WIDTH, tokens.text);
    widgets.noninteractive.corner_radius = radius;
    let control_stroke = if high_contrast { border } else { Stroke::NONE };
    for (state, fill, stroke) in [
        (&mut widgets.inactive, tokens.button, control_stroke),
        (
            &mut widgets.hovered,
            tokens.hover,
            Stroke::new(BORDER_WIDTH, tokens.border_strong),
        ),
        (
            &mut widgets.open,
            tokens.hover,
            Stroke::new(BORDER_WIDTH, tokens.border_strong),
        ),
        (
            &mut widgets.active,
            tokens.pressed,
            Stroke::new(FOCUS_WIDTH, tokens.focus),
        ),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke;
        state.fg_stroke = Stroke::new(BORDER_WIDTH, tokens.text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    if high_contrast {
        widgets.hovered.bg_stroke = Stroke::new(FOCUS_WIDTH, tokens.text);
        widgets.active.bg_stroke = Stroke::new(FOCUS_WIDTH, tokens.focus);
        visuals.selection.bg_fill = tokens.accent;
        visuals.selection.stroke = Stroke::new(FOCUS_WIDTH, tokens.text_on_accent);
    }
    visuals
}

pub fn style(skin: &Skin) -> Style {
    let mut style = Style {
        visuals: visuals(skin),
        ..Style::default()
    };
    style.text_styles = text_styles();
    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 6.0);
    spacing.button_padding = vec2(10.0, 4.0);
    spacing.interact_size = vec2(36.0, 24.0);
    spacing.menu_margin = Margin::same(6);
    spacing.window_margin = Margin::same(DIALOG_MARGIN);
    spacing.indent = 16.0;
    spacing.icon_width = 15.0;
    spacing.icon_width_inner = 9.0;
    spacing.icon_spacing = 6.0;
    spacing.tooltip_width = 340.0;
    spacing.menu_width = 260.0;
    spacing.combo_width = 140.0;
    spacing.scroll = ScrollStyle::thin();
    style.interaction.tooltip_delay = 0.35;
    style
}

fn text_styles() -> BTreeMap<TextStyle, FontId> {
    [
        (
            TextStyle::Small,
            FontId::new(SMALL_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (TextStyle::Button, FontId::new(BODY_SIZE, fonts::medium())),
        (
            TextStyle::Heading,
            FontId::new(HEADING_SIZE, fonts::semibold()),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONOSPACE_SIZE, FontFamily::Monospace),
        ),
        (
            TextStyle::Name(SECTION.into()),
            FontId::new(SECTION_SIZE, fonts::semibold()),
        ),
    ]
    .into()
}

pub fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        let steps = ((scale - 1.0) / SCALE_STEP).round();
        (1.0 + steps * SCALE_STEP).clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}

pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (lighter, darker) = {
        let (a, b) = (luminance(a), luminance(b));
        if a >= b { (a, b) } else { (b, a) }
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastFailure {
    pub foreground: Token,
    pub background: Token,
    pub foreground_colour: Color32,
    pub background_colour: Color32,
    pub ratio: f32,
    pub minimum: f32,
}

impl ContrastFailure {
    pub fn words(&self) -> String {
        let hex =
            |colour: Color32| format!("#{:02x}{:02x}{:02x}", colour.r(), colour.g(), colour.b());
        format!(
            "{} ({} {}) on {} ({} {}) is {:.1}:1, below the {}:1 it needs",
            self.foreground.words(),
            self.foreground.key(),
            hex(self.foreground_colour),
            self.background.words(),
            self.background.key(),
            hex(self.background_colour),
            self.ratio,
            self.minimum
        )
    }
}

struct Pair {
    foreground: Token,
    background: Token,
    minimum: f32,
}

const fn pair(foreground: Token, background: Token, minimum: f32) -> Pair {
    Pair {
        foreground,
        background,
        minimum,
    }
}

fn required_pairs(high_contrast: bool) -> Vec<Pair> {
    use Token::*;

    let body = if high_contrast {
        HIGHLY_READABLE
    } else {
        READABLE
    };
    let mut pairs = vec![
        pair(Text, Panel, body),
        pair(Text, Raised, body),
        pair(Text, Sunken, body),
        pair(Text, Button, READABLE),
        pair(Text, Hover, READABLE),
        pair(Text, Pressed, READABLE),
        pair(TextOnAccent, Accent, READABLE),
        pair(AccentText, AccentSubtle, READABLE),
        pair(AccentText, AccentSurface, READABLE),
        pair(TextMuted, Button, body),
        pair(AccentText, AccentSubtle, body),
        pair(Error, ErrorSubtle, body),
        pair(Warn, WarnSubtle, body),
        pair(Success, SuccessSubtle, body),
        pair(Danger, Panel, VISIBLE_OUTLINE),
        pair(Danger, Raised, VISIBLE_OUTLINE),
    ];
    for background in [Panel, Raised] {
        for foreground in [TextMuted, Error, Warn, AccentText, Success] {
            pairs.push(pair(foreground, background, READABLE));
        }
    }
    for background in [
        AccentSubtle,
        AccentSurface,
        ErrorSubtle,
        WarnSubtle,
        SuccessSubtle,
        Stripe,
        Button,
        Hover,
    ] {
        pairs.push(pair(Text, background, body));
        pairs.push(pair(TextMuted, background, READABLE));
    }
    for background in [Panel, Raised, Sunken, AccentSurface] {
        pairs.push(pair(FieldBorder, background, VISIBLE_OUTLINE));
        pairs.push(pair(Focus, background, VISIBLE_OUTLINE));
    }
    for fill in [
        Accent,
        AccentHover,
        AccentPressed,
        Danger,
        DangerHover,
        DangerPressed,
    ] {
        pairs.push(pair(TextOnAccent, fill, READABLE));
    }
    if high_contrast {
        pairs.push(pair(Border, Panel, VISIBLE_OUTLINE));
    }
    pairs
}

pub fn check(tokens: &Tokens, high_contrast: bool) -> Result<(), ContrastFailure> {
    for Pair {
        foreground,
        background,
        minimum,
    } in required_pairs(high_contrast)
    {
        let foreground_colour = foreground.of(tokens);
        let background_colour = background.of(tokens);
        let ratio = contrast_ratio(foreground_colour, background_colour);
        if ratio < minimum {
            return Err(ContrastFailure {
                foreground,
                background,
                foreground_colour,
                background_colour,
                ratio,
                minimum,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    pub use super::{HIGHLY_READABLE, READABLE, contrast_ratio};

    fn check_skin(skin: &Skin) -> Result<(), ContrastFailure> {
        check(&skin.tokens, skin.high_contrast)
    }

    fn every_skin() -> Vec<Skin> {
        let mut skins = Vec::new();
        for (dark, high_contrast) in [(true, false), (false, false), (true, true), (false, true)] {
            skins.push(Skin::built_in(dark, high_contrast));
        }
        for (tokens, dark) in [(MIDNIGHT, true), (GRAPHITE, true), (PAPER, false)] {
            skins.push(Skin {
                tokens,
                dark,
                high_contrast: false,
                canvas: Canvas::Dark,
            });
        }
        skins
    }

    fn assert_readable(what: &str, foreground: Color32, background: Color32, minimum: f32) {
        let ratio = contrast_ratio(foreground, background);
        assert!(
            ratio >= minimum,
            "{what}: {foreground:?} on {background:?} is {ratio:.2}:1, below {minimum}:1"
        );
    }

    #[test]
    fn every_shipped_token_set_passes_the_checks_a_loaded_theme_must_pass() {
        for skin in every_skin() {
            if let Err(failure) = check_skin(&skin) {
                panic!("{skin:?}: {}", failure.words());
            }
        }
    }

    #[test]
    fn every_text_colour_meets_the_contrast_it_promises_in_the_visuals_built() {
        for skin in every_skin() {
            let visuals = visuals(&skin);
            let body = if skin.high_contrast {
                HIGHLY_READABLE
            } else {
                READABLE
            };
            let panel = visuals.panel_fill;
            let widgets = &visuals.widgets;
            let case = |what: &str| format!("{what} ({skin:?})");
            assert_readable(&case("text"), visuals.text_color(), panel, body);
            assert_readable(
                &case("text in dialogs"),
                visuals.text_color(),
                visuals.window_fill,
                body,
            );
            for background in [panel, visuals.window_fill] {
                for (what, colour) in [
                    ("weak text", visuals.weak_text_color()),
                    ("errors", visuals.error_fg_color),
                    ("warnings", visuals.warn_fg_color),
                    ("links", visuals.hyperlink_color),
                ] {
                    assert_readable(&case(what), colour, background, READABLE);
                }
            }
            for (state, style) in [
                ("button", widgets.inactive),
                ("hovered button", widgets.hovered),
                ("pressed button", widgets.active),
            ] {
                assert_readable(
                    &case(state),
                    style.text_color(),
                    style.weak_bg_fill,
                    READABLE,
                );
            }
            assert_readable(
                &case("typed text"),
                widgets.inactive.text_color(),
                visuals.text_edit_bg_color(),
                body,
            );
            assert_readable(
                &case("selected text"),
                visuals.selection.stroke.color,
                visuals.selection.bg_fill,
                READABLE,
            );
            if skin.high_contrast {
                assert_readable(
                    &case("focus outline"),
                    widgets.active.bg_stroke.color,
                    panel,
                    VISIBLE_OUTLINE,
                );
                assert_readable(
                    &case("button outline"),
                    widgets.inactive.bg_stroke.color,
                    panel,
                    VISIBLE_OUTLINE,
                );
            }
        }
    }

    #[test]
    fn a_failing_pair_is_named_with_its_keys_colours_and_ratio() {
        let mut tokens = DARK;
        tokens.text_muted = Color32::from_rgb(60, 62, 70);

        let failure = check(&tokens, false).unwrap_err();

        assert_eq!(failure.foreground, Token::TextMuted);
        assert!(failure.ratio < READABLE);
        assert!(
            failure
                .words()
                .starts_with("the muted text (text_muted #3c3e46) on "),
            "{}",
            failure.words()
        );
        assert!(failure.words().ends_with("below the 4.5:1 it needs"));
    }

    #[test]
    fn high_contrast_holds_body_text_to_seven_to_one() {
        let mut tokens = DARK_HIGH_CONTRAST;
        tokens.text = Color32::from_gray(170);

        assert!(check(&tokens, false).is_ok());
        let failure = check(&tokens, true).unwrap_err();
        assert_eq!(failure.foreground, Token::Text);
        assert_eq!(failure.minimum, HIGHLY_READABLE);
    }

    #[test]
    fn every_token_has_a_key_read_back_and_a_slot_of_its_own() {
        let mut tokens = DARK;
        for (index, token) in Token::ALL.into_iter().enumerate() {
            assert_eq!(Token::from_key(token.key()), Some(token));
            token.set(&mut tokens, Color32::from_rgb(index as u8, 1, 2));
        }
        for (index, token) in Token::ALL.into_iter().enumerate() {
            assert_eq!(token.of(&tokens), Color32::from_rgb(index as u8, 1, 2));
        }
        assert_eq!(Token::from_key("background"), None);
    }

    #[test]
    fn the_skin_published_for_each_theme_is_what_tokens_reads_back() {
        let ctx = egui::Context::default();
        let mut paper = Skin::built_in(false, false);
        paper.tokens = PAPER;
        publish(
            &ctx,
            Skins {
                dark: Skin::built_in(true, false),
                light: paper,
            },
        );

        assert_eq!(skin_for(&ctx, &Visuals::light()).tokens, PAPER);
        assert_eq!(skin_for(&ctx, &Visuals::dark()).tokens, DARK);

        let unpublished = egui::Context::default();
        let high = visuals(&Skin::built_in(false, true));
        assert!(skin_for(&unpublished, &high).high_contrast);
        assert_eq!(skin_for(&unpublished, &high).tokens, LIGHT_HIGH_CONTRAST);
    }

    #[test]
    fn the_text_caret_is_solid_in_every_theme_so_an_idle_field_never_wakes_the_app() {
        for skin in every_skin() {
            assert!(!visuals(&skin).text_cursor.blink);
        }
    }

    #[test]
    fn contrast_follows_the_wcag_formula_and_scales_snap_to_steps() {
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-3);
        assert!((contrast_ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-6);
        assert_eq!(clamp_scale(1.3), 1.25);
        assert_eq!(clamp_scale(9.0), MAX_SCALE);
        assert_eq!(clamp_scale(0.1), MIN_SCALE);
        assert_eq!(clamp_scale(f32::NAN), 1.0);
    }
}
