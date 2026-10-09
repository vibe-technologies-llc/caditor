use egui::{Align2, Rect, Ui, pos2, vec2};

use crate::{canvas, icons, view_cube};

pub const TEXT: &str = "Loading to the graphics card";
const NAME: &str = "uploading";
const ICON_GAP: f32 = 5.0;

pub fn show(ui: &Ui, view: Rect) {
    let painter = ui.painter();
    let icon = painter.layout_no_wrap(icons::UPLOADING.to_owned(), canvas::icon(), canvas::MUTED);
    let text = painter.layout_no_wrap(TEXT.to_owned(), canvas::small(), canvas::MUTED);

    let content = vec2(
        icon.size().x + ICON_GAP + text.size().x,
        icon.size().y.max(text.size().y),
    );
    let corner = pos2(
        view.right() - canvas::MARGIN,
        view_cube::area(view).bottom() + canvas::MARGIN / 2.0,
    );
    let badge = Align2::RIGHT_TOP.anchor_size(corner, canvas::chip_size(content));

    canvas::paint_backdrop(painter, badge);
    let inner = badge.shrink2(canvas::PADDING);
    let icon_at = Align2::LEFT_CENTER.anchor_size(inner.left_center(), icon.size());
    let text_at = Align2::RIGHT_CENTER.anchor_size(inner.right_center(), text.size());
    painter.galley(icon_at.min, icon, canvas::MUTED);
    painter.galley(text_at.min, text, canvas::MUTED);

    canvas::announce(ui, badge, NAME, TEXT, None);
}
