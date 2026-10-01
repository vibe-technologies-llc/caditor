use egui::{
    Color32, CornerRadius, Direction, Frame, Id, Layout, Margin, Rect, Response, Sense, Sides,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType, vec2,
};

use crate::{
    appearance::{
        self, CONTROL_HEIGHT, FOCUS_WIDTH, SPACE_L, SPACE_M, SPACE_S, SPACE_XS, WIDGET_RADIUS,
    },
    icons, widgets,
};

pub const ROW_GAP: f32 = SPACE_XS;
pub const CHILD_INDENT: f32 = CHEVRON_SIDE + SPACE_XS + SPACE_L;
pub const BODY_INDENT: f32 = SPACE_M;
const ROW_MARGIN: Margin = Margin::symmetric(SPACE_S as i8, SPACE_XS as i8);
const OPEN_BAR_WIDTH: f32 = 3.0;
const CHEVRON_SIDE: f32 = CONTROL_HEIGHT - SPACE_S;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Look {
    pub selected: bool,
    pub open: bool,
}

pub struct Shown<L, T> {
    pub rect: Rect,
    pub leading: L,
    pub trailing: T,
}

pub fn rows<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        add(ui)
    })
    .inner
}

pub fn content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let normal = ui.ctx().global_style().spacing.item_spacing.y;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = normal;
        add(ui)
    })
    .inner
}

pub fn indented<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    content(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(BODY_INDENT);
            ui.vertical(add).inner
        })
        .inner
    })
}

pub fn show<L, T>(
    ui: &mut Ui,
    look: Look,
    leading: impl FnOnce(&mut Ui) -> L,
    trailing: impl FnOnce(&mut Ui) -> T,
) -> Shown<L, T> {
    let tokens = appearance::tokens(ui);
    let mut prepared = Frame::new()
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(ROW_MARGIN)
        .begin(ui);
    let (leading, trailing) = {
        let ui = &mut prepared.content_ui;
        let spacing = ui.spacing_mut();
        spacing.item_spacing.x = SPACE_S;
        spacing.button_padding = Vec2::splat(SPACE_S);
        Sides::new()
            .shrink_left()
            .truncate()
            .height(CONTROL_HEIGHT)
            .spacing(SPACE_S)
            .show(ui, leading, |ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                trailing(ui)
            })
    };
    let rect = prepared.content_ui.min_rect() + ROW_MARGIN;
    prepared.frame.fill = if look.selected {
        tokens.accent_subtle
    } else if look.open {
        tokens.accent_surface
    } else if ui.rect_contains_pointer(rect) {
        tokens.hover
    } else {
        Color32::TRANSPARENT
    };
    let rect = prepared.end(ui).rect;
    if look.open {
        let bar = Rect::from_min_size(rect.min, vec2(OPEN_BAR_WIDTH, rect.height()));
        ui.painter()
            .rect_filled(bar, CornerRadius::same(WIDGET_RADIUS), tokens.accent);
    }
    Shown {
        rect,
        leading,
        trailing,
    }
}

pub fn slot<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (_, rect) = ui.allocate_space(Vec2::splat(CONTROL_HEIGHT));
    let mut slot = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::centered_and_justified(Direction::LeftToRight)),
    );
    slot.spacing_mut().button_padding = Vec2::splat(SPACE_S);
    add(&mut slot)
}

pub fn empty_slot(ui: &mut Ui) {
    slot(ui, |_| {});
}

pub fn chevron(ui: &mut Ui, open: bool, subject: &str) -> Response {
    let (glyph, hint) = if open {
        (icons::EXPANDED, "Hide details")
    } else {
        (icons::COLLAPSED, "Show details")
    };
    let gap = ui.spacing().item_spacing.x;
    let spacing = ui.spacing_mut();
    spacing.item_spacing.x = SPACE_XS;
    spacing.interact_size.y = CHEVRON_SIDE;
    spacing.button_padding = Vec2::splat(SPACE_XS);
    let response = widgets::named(
        widgets::icon_button(ui, glyph, hint),
        &format!("{hint} of {subject}"),
    );
    let spacing = ui.spacing_mut();
    spacing.item_spacing.x = gap;
    spacing.interact_size.y = CONTROL_HEIGHT;
    spacing.button_padding = Vec2::splat(SPACE_S);
    response
}

pub fn sense(ui: &mut Ui, id: Id, from: f32, name: &str, selected: bool) -> Response {
    let area = ui.max_rect().with_min_x(from);
    let response = ui.interact(area, id, Sense::click_and_drag());
    let enabled = ui.is_enabled();
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, name));
    response
}

pub fn focus_outline(ui: &Ui, rect: Rect) {
    let focus = appearance::tokens(ui).focus;
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(WIDGET_RADIUS),
        Stroke::new(FOCUS_WIDTH, focus),
        StrokeKind::Inside,
    );
}
