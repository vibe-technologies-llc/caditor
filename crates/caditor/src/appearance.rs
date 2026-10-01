use std::collections::BTreeMap;

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, TextStyle, Ui,
    Visuals, style::ScrollStyle, vec2,
};

use crate::fonts;

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
pub const SPACE_XL: f32 = 20.0;
pub const DIALOG_MARGIN: i8 = 20;
const HIGH_CONTRAST_DISABLED_ALPHA: f32 = 0.7;
const DISABLED_ALPHA: f32 = 0.5;
const MONOSPACE_SIZE: f32 = 12.5;

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

const DARK: Tokens = Tokens {
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

const LIGHT: Tokens = Tokens {
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

const DARK_HIGH_CONTRAST: Tokens = Tokens {
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

const LIGHT_HIGH_CONTRAST: Tokens = Tokens {
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

pub fn tokens(ui: &Ui) -> &'static Tokens {
    tokens_for(ui.visuals())
}

pub fn tokens_for(visuals: &Visuals) -> &'static Tokens {
    let high_contrast = Tokens::of(visuals.dark_mode, true).panel == visuals.panel_fill;
    Tokens::of(visuals.dark_mode, high_contrast)
}

pub fn visuals(dark: bool, high_contrast: bool) -> Visuals {
    let tokens = Tokens::of(dark, high_contrast);
    let mut visuals = if dark {
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

pub fn style(dark: bool, high_contrast: bool) -> Style {
    let mut style = Style {
        visuals: visuals(dark, high_contrast),
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

#[cfg(test)]
pub mod tests {
    use super::*;

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

    pub const READABLE: f32 = 4.5;
    pub const HIGHLY_READABLE: f32 = 7.0;
    const VISIBLE_OUTLINE: f32 = 3.0;

    fn assert_readable(what: &str, foreground: Color32, background: Color32, minimum: f32) {
        let ratio = contrast_ratio(foreground, background);
        assert!(
            ratio >= minimum,
            "{what}: {foreground:?} on {background:?} is {ratio:.2}:1, below {minimum}:1"
        );
    }

    const CASES: [(bool, bool); 4] = [(true, false), (false, false), (true, true), (false, true)];

    #[test]
    fn every_text_colour_meets_the_contrast_it_promises() {
        for (dark, high_contrast) in CASES {
            let visuals = visuals(dark, high_contrast);
            let body = if high_contrast {
                HIGHLY_READABLE
            } else {
                READABLE
            };
            let panel = visuals.panel_fill;
            let widgets = &visuals.widgets;
            let case = |what: &str| format!("{what} (dark {dark}, high contrast {high_contrast})");
            assert_readable(&case("text"), visuals.text_color(), panel, body);
            assert_readable(
                &case("text in dialogs"),
                visuals.text_color(),
                visuals.window_fill,
                body,
            );
            for background in [panel, visuals.window_fill] {
                assert_readable(
                    &case("weak text"),
                    visuals.weak_text_color(),
                    background,
                    READABLE,
                );
                assert_readable(
                    &case("errors"),
                    visuals.error_fg_color,
                    background,
                    READABLE,
                );
                assert_readable(
                    &case("warnings"),
                    visuals.warn_fg_color,
                    background,
                    READABLE,
                );
                assert_readable(
                    &case("links"),
                    visuals.hyperlink_color,
                    background,
                    READABLE,
                );
                assert_readable(
                    &case("success"),
                    tokens_for(&visuals).success,
                    background,
                    READABLE,
                );
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
            if high_contrast {
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
    fn every_token_pairing_is_readable() {
        for (dark, high_contrast) in CASES {
            let tokens = Tokens::of(dark, high_contrast);
            let body = if high_contrast {
                HIGHLY_READABLE
            } else {
                READABLE
            };
            let case = |what: &str| format!("{what} (dark {dark}, high contrast {high_contrast})");
            assert_readable(
                &case("text on accent"),
                tokens.text_on_accent,
                tokens.accent,
                READABLE,
            );
            for (what, background) in [
                ("accent row", tokens.accent_subtle),
                ("accent surface", tokens.accent_surface),
                ("error callout", tokens.error_subtle),
                ("warning callout", tokens.warn_subtle),
                ("success callout", tokens.success_subtle),
                ("stripe", tokens.stripe),
                ("button", tokens.button),
                ("hover", tokens.hover),
            ] {
                assert_readable(&case(what), tokens.text, background, body);
                assert_readable(&case(what), tokens.text_muted, background, READABLE);
            }
            for (what, background) in [
                ("accent text on accent row", tokens.accent_subtle),
                ("accent text on accent surface", tokens.accent_surface),
            ] {
                assert_readable(&case(what), tokens.accent_text, background, READABLE);
            }
            for (what, foreground, background) in [
                ("neutral pill", tokens.text_muted, tokens.button),
                ("info pill", tokens.accent_text, tokens.accent_subtle),
                ("error on its callout", tokens.error, tokens.error_subtle),
                ("warning on its callout", tokens.warn, tokens.warn_subtle),
                (
                    "success on its callout",
                    tokens.success,
                    tokens.success_subtle,
                ),
            ] {
                assert_readable(&case(what), foreground, background, body);
            }
            assert_readable(
                &case("focus outline"),
                tokens.focus,
                tokens.panel,
                VISIBLE_OUTLINE,
            );
        }
    }

    #[test]
    fn every_control_boundary_and_focus_ring_stands_out() {
        for (dark, high_contrast) in CASES {
            let tokens = Tokens::of(dark, high_contrast);
            let case = |what: &str| format!("{what} (dark {dark}, high contrast {high_contrast})");
            for (what, background) in [
                ("panel", tokens.panel),
                ("card", tokens.raised),
                ("field", tokens.sunken),
                ("accent surface", tokens.accent_surface),
            ] {
                assert_readable(
                    &case(&format!("field outline on {what}")),
                    tokens.field_border,
                    background,
                    VISIBLE_OUTLINE,
                );
                assert_readable(
                    &case(&format!("focus ring on {what}")),
                    tokens.focus,
                    background,
                    VISIBLE_OUTLINE,
                );
            }
            for (what, background) in [("panel", tokens.panel), ("dialog", tokens.raised)] {
                assert_readable(
                    &case(&format!("destructive button on {what}")),
                    tokens.danger,
                    background,
                    VISIBLE_OUTLINE,
                );
            }
            for (what, fill) in [
                ("primary button", tokens.accent),
                ("hovered primary button", tokens.accent_hover),
                ("pressed primary button", tokens.accent_pressed),
                ("destructive button", tokens.danger),
                ("hovered destructive button", tokens.danger_hover),
                ("pressed destructive button", tokens.danger_pressed),
            ] {
                assert_readable(&case(what), tokens.text_on_accent, fill, READABLE);
            }
            assert_readable(
                &case("selected row"),
                tokens.text,
                tokens.accent_subtle,
                if high_contrast {
                    HIGHLY_READABLE
                } else {
                    READABLE
                },
            );
        }
    }

    #[test]
    fn the_tokens_are_found_again_from_the_visuals_they_built() {
        for (dark, high_contrast) in CASES {
            let built = visuals(dark, high_contrast);
            assert!(std::ptr::eq(
                tokens_for(&built),
                Tokens::of(dark, high_contrast)
            ));
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
