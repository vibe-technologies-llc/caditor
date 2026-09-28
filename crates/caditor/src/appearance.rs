use egui::{Color32, Stroke, Visuals};

pub const MIN_SCALE: f32 = 0.75;
pub const MAX_SCALE: f32 = 2.0;
pub const SCALE_STEP: f32 = 0.125;
const FOCUS_STROKE_WIDTH: f32 = 2.0;
const HIGH_CONTRAST_BORDER_WIDTH: f32 = 1.0;
const HIGH_CONTRAST_DISABLED_ALPHA: f32 = 0.7;

struct Palette {
    text: Color32,
    weak: Color32,
    error: Color32,
    warn: Color32,
    hyperlink: Color32,
}

const DARK: Palette = Palette {
    text: Color32::from_gray(190),
    weak: Color32::from_gray(150),
    error: Color32::from_rgb(255, 120, 110),
    warn: Color32::from_rgb(255, 176, 70),
    hyperlink: Color32::from_rgb(120, 185, 255),
};

const LIGHT: Palette = Palette {
    text: Color32::from_gray(35),
    weak: Color32::from_gray(90),
    error: Color32::from_rgb(180, 20, 20),
    warn: Color32::from_rgb(145, 72, 0),
    hyperlink: Color32::from_rgb(0, 88, 176),
};

const DARK_HIGH_CONTRAST: Palette = Palette {
    text: Color32::WHITE,
    weak: Color32::from_gray(205),
    error: Color32::from_rgb(255, 150, 140),
    warn: Color32::from_rgb(255, 205, 80),
    hyperlink: Color32::from_rgb(150, 205, 255),
};

const LIGHT_HIGH_CONTRAST: Palette = Palette {
    text: Color32::BLACK,
    weak: Color32::from_gray(50),
    error: Color32::from_rgb(150, 0, 0),
    warn: Color32::from_rgb(115, 55, 0),
    hyperlink: Color32::from_rgb(0, 60, 150),
};

fn palette(dark: bool, high_contrast: bool) -> &'static Palette {
    match (dark, high_contrast) {
        (true, false) => &DARK,
        (false, false) => &LIGHT,
        (true, true) => &DARK_HIGH_CONTRAST,
        (false, true) => &LIGHT_HIGH_CONTRAST,
    }
}

pub fn success_color(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(120, 220, 140)
    } else {
        Color32::from_rgb(0, 100, 30)
    }
}

pub fn visuals(dark: bool, high_contrast: bool) -> Visuals {
    let colors = palette(dark, high_contrast);
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.widgets.noninteractive.fg_stroke.color = colors.text;
    visuals.weak_text_color = Some(colors.weak);
    visuals.error_fg_color = colors.error;
    visuals.warn_fg_color = colors.warn;
    visuals.hyperlink_color = colors.hyperlink;
    if high_contrast {
        raise_contrast(&mut visuals, colors);
    }
    visuals
}

fn raise_contrast(visuals: &mut Visuals, colors: &Palette) {
    let dark = visuals.dark_mode;
    let (background, field, button, hovered, border, focus, selection) = if dark {
        (
            Color32::BLACK,
            Color32::BLACK,
            Color32::from_gray(38),
            Color32::from_gray(60),
            Color32::from_gray(170),
            Color32::from_rgb(255, 214, 0),
            Color32::from_rgb(0, 80, 170),
        )
    } else {
        (
            Color32::WHITE,
            Color32::WHITE,
            Color32::from_gray(232),
            Color32::from_gray(212),
            Color32::from_gray(70),
            Color32::from_rgb(0, 70, 200),
            Color32::from_rgb(0, 70, 170),
        )
    };
    visuals.panel_fill = background;
    visuals.window_fill = background;
    visuals.extreme_bg_color = field;
    visuals.window_stroke = Stroke::new(HIGH_CONTRAST_BORDER_WIDTH, border);
    visuals.disabled_alpha = HIGH_CONTRAST_DISABLED_ALPHA;
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = background;
    widgets.noninteractive.weak_bg_fill = background;
    widgets.noninteractive.bg_stroke = Stroke::new(HIGH_CONTRAST_BORDER_WIDTH, border);
    for (state, fill) in [
        (&mut widgets.inactive, button),
        (&mut widgets.hovered, hovered),
        (&mut widgets.open, hovered),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.fg_stroke.color = colors.text;
        state.bg_stroke = Stroke::new(HIGH_CONTRAST_BORDER_WIDTH, border);
    }
    widgets.hovered.bg_stroke = Stroke::new(FOCUS_STROKE_WIDTH, colors.text);
    widgets.active.bg_fill = hovered;
    widgets.active.weak_bg_fill = hovered;
    widgets.active.fg_stroke.color = colors.text;
    widgets.active.bg_stroke = Stroke::new(FOCUS_STROKE_WIDTH, focus);
    visuals.selection.bg_fill = selection;
    visuals.selection.stroke = Stroke::new(FOCUS_STROKE_WIDTH, Color32::WHITE);
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
mod tests {
    use super::*;

    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
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

    const READABLE: f32 = 4.5;
    const HIGHLY_READABLE: f32 = 7.0;
    const VISIBLE_OUTLINE: f32 = 3.0;

    fn assert_readable(what: &str, foreground: Color32, background: Color32, minimum: f32) {
        let ratio = contrast_ratio(foreground, background);
        assert!(
            ratio >= minimum,
            "{what}: {foreground:?} on {background:?} is {ratio:.2}:1, below {minimum}:1"
        );
    }

    #[test]
    fn every_text_colour_meets_the_contrast_it_promises() {
        for (dark, high_contrast) in [(true, false), (false, false), (true, true), (false, true)] {
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
            assert_readable(
                &case("weak text"),
                visuals.weak_text_color(),
                panel,
                READABLE,
            );
            assert_readable(&case("errors"), visuals.error_fg_color, panel, READABLE);
            assert_readable(&case("warnings"), visuals.warn_fg_color, panel, READABLE);
            assert_readable(&case("links"), visuals.hyperlink_color, panel, READABLE);
            assert_readable(&case("success"), success_color(&visuals), panel, READABLE);
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
    fn contrast_follows_the_wcag_formula_and_scales_snap_to_steps() {
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-3);
        assert!((contrast_ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-6);
        assert_eq!(clamp_scale(1.3), 1.25);
        assert_eq!(clamp_scale(9.0), MAX_SCALE);
        assert_eq!(clamp_scale(0.1), MIN_SCALE);
        assert_eq!(clamp_scale(f32::NAN), 1.0);
    }
}
