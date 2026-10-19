use caditor_cloud::{Service, UploadFormat};
use egui::{
    Align, CornerRadius, FontId, Frame, Image, Label, Layout, RichText, Sense, Stroke, StrokeKind,
    TextStyle, TextWrapMode, TextureHandle, Ui, UiBuilder, load::SizedTexture, vec2,
};
use jiff::Timestamp;

use crate::{
    appearance::{self, BORDER_WIDTH, CARD_RADIUS, SPACE_L, SPACE_S},
    cloud::{CloudCommand, Destination, Failure, Link, Linked, SendStage, Stage},
    fonts, icons,
    widgets::{self, Tone},
};

pub const TILE_HEIGHT: f32 = 60.0;
pub const TILE_GAP: f32 = 8.0;
const TILE_ICON_SIZE: f32 = 22.0;
const TILE_PICTURE_SIDE: f32 = TILE_HEIGHT - 2.0 * SPACE_S;
const TILE_PICTURE_WIDTH: f32 = TILE_PICTURE_SIDE * 4.0 / 3.0;
const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const MONTH: i64 = 30 * DAY;
const YEAR: i64 = 365 * DAY;
const YESTERDAY_ENDS: i64 = 2 * DAY - 1;
pub const TRY_AGAIN: &str = "Try again";
pub const DISMISS: &str = "Dismiss";
pub const CANCEL_IMPORT_HOVER: &str = "Stop this import; nothing is added to the model";
pub const CANCEL_SEND_HOVER: &str = "Stop sending; nothing is added to the folder";
pub const CANCEL_TRANSLATING_HOVER: &str =
    "Stop waiting; the service may still finish adding the Part Studio to the document";
pub const CANCEL_PREPARING_HOVER: &str = "Stop preparing the file; nothing is sent";

#[derive(Debug, Clone, PartialEq)]
pub enum SendChoice {
    Cloud(CloudCommand),
    Again(Destination),
}

pub fn upload_icon(format: UploadFormat) -> &'static str {
    if format.is_drawing() {
        icons::UPLOADED_DRAWING
    } else if format.is_mesh() {
        icons::UPLOADED_MESH
    } else {
        icons::BODIES
    }
}

pub fn page_card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let tokens = appearance::tokens(ui);
    let edge = if appearance::is_high_contrast(ui) {
        Stroke::new(BORDER_WIDTH, tokens.border)
    } else {
        Stroke::NONE
    };
    Frame::new()
        .fill(tokens.raised)
        .stroke(edge)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(appearance::margin(SPACE_L, SPACE_L))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub struct Tile<'a> {
    pub glyph: &'a str,
    pub title: &'a str,
    pub detail: &'a str,
    pub name: &'a str,
    pub hover: &'a str,
    pub enabled: bool,
    pub picture: Option<&'a TextureHandle>,
}

pub fn tile(ui: &mut Ui, width: f32, tile: &Tile<'_>) -> bool {
    let tokens = appearance::tokens(ui);
    let sense = if tile.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, TILE_HEIGHT), sense);
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(CARD_RADIUS);
        ui.painter().rect_filled(rect, radius, tokens.raised);
        if appearance::is_high_contrast(ui) {
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(BORDER_WIDTH, tokens.border),
                StrokeKind::Inside,
            );
        }
        if tile.enabled && response.hovered() {
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(BORDER_WIDTH, tokens.border_strong),
                StrokeKind::Inside,
            );
        }
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(SPACE_L, SPACE_S)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
        match tile.picture {
            Some(texture) => {
                let natural = texture.size_vec2();
                let fitted = (TILE_PICTURE_SIDE / natural.y.max(1.0))
                    .min(TILE_PICTURE_WIDTH / natural.x.max(1.0));
                let side = natural * fitted;
                let picture = child.add(
                    Image::new(SizedTexture::new(texture.id(), side))
                        .fit_to_exact_size(side)
                        .corner_radius(CornerRadius::same(CARD_RADIUS / 2)),
                );
                widgets::decorative(&child, &picture);
            }
            None => {
                let glyph_colour = if tile.enabled {
                    tokens.accent_text
                } else {
                    tokens.text_muted
                };
                let glyph = child.add(
                    Label::new(
                        RichText::new(tile.glyph)
                            .font(FontId::new(TILE_ICON_SIZE, fonts::icons()))
                            .color(glyph_colour),
                    )
                    .selectable(false),
                );
                widgets::decorative(&child, &glyph);
            }
        }
        let text_height =
            2.0 * child.text_style_height(&TextStyle::Body) + child.spacing().item_spacing.y;
        let text_size = vec2(child.available_width(), text_height);
        child.allocate_ui_with_layout(text_size, Layout::top_down(Align::Min), |ui| {
            ui.add(Label::new(widgets::strong(tile.title)).selectable(false));
            ui.add(Label::new(widgets::muted(tile.detail, ui)).selectable(false));
        });
    }
    let response = widgets::named(response, tile.name).on_hover_text(tile.hover);
    let response = if tile.enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    if response.has_focus() {
        widgets::paint_focus_ring(ui, rect);
    }
    tile.enabled && response.clicked()
}

pub fn tiles(
    ui: &mut Ui,
    count: usize,
    least_width: f32,
    mut draw: impl FnMut(&mut Ui, usize, f32) -> bool,
) -> Option<usize> {
    let available = ui.available_width();
    let columns = (((available + TILE_GAP) / (least_width + TILE_GAP)).floor() as usize).max(1);
    let gaps = (columns - 1) as f32 * TILE_GAP;
    let width = ((available - gaps) / columns as f32).max(0.0);
    let mut chosen = None;
    let mut first = 0;
    while first < count {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = TILE_GAP;
            for at in first..(first + columns).min(count) {
                if draw(ui, at, width) {
                    chosen = Some(at);
                }
            }
        });
        first += columns;
    }
    chosen
}

pub fn modified_text(modified: Option<Timestamp>, now: Timestamp) -> String {
    let Some(modified) = modified else {
        return String::new();
    };
    let seconds = now.as_second().saturating_sub(modified.as_second());
    let plural = |count: i64, unit: &str| {
        let s = if count == 1 { "" } else { "s" };
        format!("Edited {count} {unit}{s} ago")
    };
    if seconds < MINUTE {
        "Edited just now".to_owned()
    } else if seconds < HOUR {
        plural(seconds / MINUTE, "minute")
    } else if seconds < DAY {
        plural(seconds / HOUR, "hour")
    } else if seconds <= YESTERDAY_ENDS {
        "Edited yesterday".to_owned()
    } else if seconds < MONTH {
        plural(seconds / DAY, "day")
    } else if seconds < YEAR {
        plural(seconds / MONTH, "month")
    } else {
        plural(seconds / YEAR, "year")
    }
}

pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{bytes} {name}")
    } else {
        format!("{value:.1} {name}")
    }
}

pub fn link_pill(ui: &mut Ui, link: &Link) {
    let (tone, text) = match link {
        Link::Off => (Tone::Neutral, "Off"),
        Link::Starting => (Tone::Info, "Checking the sign-in"),
        Link::SignedOut => (Tone::Neutral, "Signed out"),
        Link::SigningIn(_) => (Tone::Info, "Signing in"),
        Link::Linked(_) => (Tone::Success, "Signed in"),
    };
    widgets::status_pill(ui, tone, text);
}

pub fn failure_callout(ui: &mut Ui, tone: Tone, summary: &str, failure: &Failure) -> bool {
    widgets::callout(ui, tone, |ui| {
        ui.add(Label::new(format!("{summary} {failure}")).wrap());
        ui.add(widgets::button(TRY_AGAIN)).clicked()
    })
}

pub fn import_status(ui: &mut Ui, service: Service, linked: &Linked) -> Option<CloudCommand> {
    let import = linked.import.as_ref()?;
    let name = &import.item.name();
    let words = match &import.stage {
        Stage::Failed(failure) => {
            let summary = format!("Could not import “{name}” from {service}.");
            let mut command = None;
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.add(Label::new(format!("{summary} {failure}")).wrap());
                ui.horizontal(|ui| {
                    if ui.add(widgets::button(TRY_AGAIN)).clicked() {
                        command = Some(import.item.command(service));
                    }
                    if ui.add(widgets::button(DISMISS)).clicked() {
                        command = Some(CloudCommand::DismissImport(service));
                    }
                });
            });
            return command;
        }
        Stage::Starting if import.item.is_converted() => {
            format!("Asking {service} to convert “{name}”…")
        }
        Stage::Starting => format!("Asking {service} for “{name}”…"),
        Stage::Converting(Some(percent)) => {
            format!("{service} is converting “{name}” to STEP… {percent}%")
        }
        Stage::Converting(None) => format!("{service} is converting “{name}” to STEP…"),
        Stage::Downloading {
            received,
            total: Some(total),
        } => format!(
            "Downloading “{name}”… {} of {}",
            size_text(*received),
            size_text(*total)
        ),
        Stage::Downloading {
            received,
            total: None,
        } => format!("Downloading “{name}”… {}", size_text(*received)),
    };
    widgets::progress(words)
        .cancel(CANCEL_IMPORT_HOVER)
        .wrapping()
        .show(ui)
        .then_some(CloudCommand::CancelImport(service))
}

pub fn send_status(ui: &mut Ui, service: Service, linked: &Linked) -> Option<SendChoice> {
    let send = linked.sending.as_ref()?;
    let name = &send.shipment.name;
    let words = match &send.stage {
        SendStage::Failed(failure) => {
            let summary = format!("Could not send “{name}” to {service}.");
            let mut choice = None;
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.add(Label::new(format!("{summary} {failure}")).wrap());
                ui.horizontal(|ui| {
                    if ui.add(widgets::button(TRY_AGAIN)).clicked() {
                        choice = Some(SendChoice::Again(send.shipment.destination.clone()));
                    }
                    if ui.add(widgets::button(DISMISS)).clicked() {
                        choice = Some(SendChoice::Cloud(CloudCommand::DismissSend(service)));
                    }
                });
            });
            return choice;
        }
        SendStage::Starting => format!("Asking {service} for a place to put “{name}”…"),
        SendStage::Uploading {
            sent,
            total: Some(total),
        } if sent >= total => format!("Finishing “{name}” on {service}…"),
        SendStage::Uploading {
            sent,
            total: Some(total),
        } => format!(
            "Sending “{name}” to {service}… {} of {}",
            size_text(*sent),
            size_text(*total)
        ),
        SendStage::Translating => {
            format!("{service} is turning “{name}” into a Part Studio…")
        }
        SendStage::Uploading { sent, total: None } => {
            format!("Sending “{name}” to {service}… {}", size_text(*sent))
        }
    };
    let hover = match send.stage {
        SendStage::Translating => CANCEL_TRANSLATING_HOVER,
        _ => CANCEL_SEND_HOVER,
    };
    widgets::progress(words)
        .cancel(hover)
        .wrapping()
        .show(ui)
        .then_some(SendChoice::Cloud(CloudCommand::CancelSend(service)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds_ago: i64) -> (Option<Timestamp>, Timestamp) {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let modified = Timestamp::from_second(1_800_000_000 - seconds_ago).unwrap();
        (Some(modified), now)
    }

    fn text(seconds_ago: i64) -> String {
        let (modified, now) = at(seconds_ago);
        modified_text(modified, now)
    }

    #[test]
    fn times_read_as_the_largest_unit_that_fits() {
        assert_eq!(text(5), "Edited just now");
        assert_eq!(text(60), "Edited 1 minute ago");
        assert_eq!(text(5 * 60), "Edited 5 minutes ago");
        assert_eq!(text(3 * HOUR), "Edited 3 hours ago");
        assert_eq!(text(DAY + HOUR), "Edited yesterday");
        assert_eq!(text(4 * DAY), "Edited 4 days ago");
        assert_eq!(text(65 * DAY), "Edited 2 months ago");
        assert_eq!(text(800 * DAY), "Edited 2 years ago");
    }

    #[test]
    fn an_unknown_time_says_nothing_and_a_future_one_reads_as_just_now() {
        assert_eq!(modified_text(None, Timestamp::UNIX_EPOCH), "");
        assert_eq!(text(-500), "Edited just now");
    }

    #[test]
    fn sizes_use_decimal_si_units() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(999), "999 B");
        assert_eq!(size_text(1_500), "1.5 kB");
        assert_eq!(size_text(12_340_000), "12.3 MB");
        assert_eq!(size_text(7_000_000_000), "7.0 GB");
        assert_eq!(size_text(7_000_000_000_000), "7000.0 GB");
    }
}
