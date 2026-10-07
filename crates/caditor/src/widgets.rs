use std::sync::Arc;

use egui::{
    Align, Button, Color32, CornerRadius, CursorIcon, FocusDirection, Frame, Galley, Grid, Id, Key,
    Label, Layout, Margin, Modal, Modifiers, Popup, Rect, Response, RichText, Sense, Sides, Stroke,
    StrokeKind, TextStyle, TextWrapMode, Ui, Vec2, Widget, WidgetInfo, WidgetText, WidgetType,
    accesskit::{Live, Role},
    collapsing_header::CollapsingState,
    pos2, vec2,
};

use crate::{
    appearance::{
        self, BORDER_WIDTH, CARD_RADIUS, CONTROL_HEIGHT, DIALOG_MARGIN, FOCUS_WIDTH, ICON_SIZE,
        SECTION, SMALL_SIZE, SPACE_S, SPACE_XS, TOOL_ICON_SIZE, Tokens, WIDGET_RADIUS,
    },
    fonts, icons,
};

const DIALOG_EDGE: f32 = 8.0;
const MIN_DIALOG_WIDTH: f32 = 240.0;
const LIST_SCREEN_SHARE: f32 = 0.4;
const MIN_LIST_HEIGHT: f32 = 96.0;
const MIN_MENU_SHARE: f32 = 0.5;
pub const FIELD_WIDTH: f32 = 120.0;
const PROPERTY_SPACING: [f32; 2] = [12.0, 8.0];
const CAPTION_WIDTH: f32 = 84.0;
const CARD_MARGIN: i8 = 10;
const CALLOUT_MARGIN: Margin = Margin::symmetric(10, 8);
const PILL_MARGIN: Margin = Margin::symmetric(8, 2);
const PILL_RADIUS: u8 = 10;
const PILL_ICON_GAP: f32 = 4.0;
const TOOL_PADDING: Vec2 = vec2(6.0, 5.0);
const TOOL_MIN_WIDTH: f32 = 46.0;
const TOOL_LABEL_GAP: f32 = 2.0;
const SELECTED_WIDTH: f32 = 1.0;
pub const COMPACT_TOOL_GAP: f32 = 2.0;
const FOCUS_GAP: f32 = 2.0;
const SWATCH_INSET: f32 = 3.0;
const SECTION_BAND_MARGIN: Margin = Margin::symmetric(2, 2);
const SWATCH_CHOSEN_WIDTH: f32 = 2.0;
const SEGMENT_INSET: i8 = 2;
const KEY_CAP_MARGIN: Margin = Margin::symmetric(5, 1);
const KEY_CAP_RADIUS: u8 = 4;
const EMPTY_STATE_MARGIN: Margin = Margin::symmetric(4, 6);
const MIN_CORNER_BUTTON_SIDE: f32 = 14.0;
const DIALOG_FOOTER_GAP: f32 = 14.0;
const UNDERLINE_WIDTH: f32 = 1.0;
const TAB_PADDING: egui::Vec2 = vec2(10.0, 6.0);
const TAB_ICON_GAP: f32 = 6.0;
const TAB_GAP: f32 = 2.0;
const TAB_UNDERLINE: f32 = 2.0;
const CAPTION_KEY: &str = "property-caption";
const WIDTH_CHANGE: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Error,
}

impl Tone {
    pub fn color(self, tokens: &Tokens) -> Color32 {
        match self {
            Self::Neutral => tokens.text_muted,
            Self::Info => tokens.accent_text,
            Self::Success => tokens.success,
            Self::Warning => tokens.warn,
            Self::Error => tokens.error,
        }
    }

    pub fn fill(self, tokens: &Tokens) -> Color32 {
        match self {
            Self::Neutral => tokens.button,
            Self::Info => tokens.accent_subtle,
            Self::Success => tokens.success_subtle,
            Self::Warning => tokens.warn_subtle,
            Self::Error => tokens.error_subtle,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Neutral | Self::Info => icons::INFO,
            Self::Success => icons::UP_TO_DATE,
            Self::Warning => icons::WARNING,
            Self::Error => icons::FAILED,
        }
    }
}

pub fn icon(glyph: &str) -> RichText {
    RichText::new(glyph).font(egui::FontId::new(ICON_SIZE, fonts::icons()))
}

pub fn muted(text: impl Into<String>, ui: &Ui) -> RichText {
    RichText::new(text).color(appearance::tokens(ui).text_muted)
}

pub fn icon_label(ui: &mut Ui, glyph: &str, color: Color32) -> Response {
    let response = ui.add(Label::new(icon(glyph).color(color)).selectable(false));
    decorative(ui, &response);
    response
}

pub fn described_icon(ui: &mut Ui, glyph: &str, color: Color32, description: &str) -> Response {
    let response = ui.add(Label::new(icon(glyph).color(color)).selectable(false));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, description));
    response.on_hover_text(description)
}

pub fn named(response: Response, name: &str) -> Response {
    name_button(response, name, None)
}

pub fn announced(ui: &Ui, response: &Response, urgent: bool) {
    let live = if urgent {
        Live::Assertive
    } else {
        Live::Polite
    };
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_live(live));
}

pub fn decorative(ui: &Ui, response: &Response) {
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_hidden());
}

fn name_button(response: Response, name: &str, selected: Option<bool>) -> Response {
    let enabled = response.enabled();
    response.widget_info(|| match selected {
        Some(selected) => WidgetInfo::selected(WidgetType::Button, enabled, selected, name),
        None => WidgetInfo::labeled(WidgetType::Button, enabled, name),
    });
    response
}

pub struct Named<W> {
    widget: W,
    name: String,
    selected: Option<bool>,
}

impl<W> Named<W> {
    pub fn new(widget: W, name: impl Into<String>) -> Self {
        Self {
            widget,
            name: name.into(),
            selected: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }
}

impl<W: Widget> Widget for Named<W> {
    fn ui(self, ui: &mut Ui) -> Response {
        name_button(self.widget.ui(ui), &self.name, self.selected)
    }
}

pub fn icon_button(ui: &mut Ui, glyph: &str, hover: &str) -> Response {
    let muted = appearance::tokens(ui).text_muted;
    let side = ui.spacing().interact_size.y;
    let button = Button::new(icon(glyph).color(muted))
        .frame_when_inactive(false)
        .min_size(Vec2::splat(side));
    ui.scope(|ui| {
        let padding = ((side - ICON_SIZE) / 2.0).max(0.0);
        ui.spacing_mut().button_padding = vec2(padding, padding.min(SPACE_XS));
        ui.add(Named::new(button, hover))
    })
    .inner
    .on_hover_text(hover)
}

pub fn swatch(ui: &mut Ui, fill: Color32, name: &str, chosen: bool) -> Response {
    let tokens = appearance::tokens(ui);
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    if ui.is_rect_visible(rect) {
        let colour = rect.shrink(SWATCH_INSET);
        let radius = CornerRadius::same(WIDGET_RADIUS);
        let outline = if response.hovered() {
            tokens.text_muted
        } else {
            tokens.field_border
        };
        let painter = ui.painter();
        painter.rect_filled(colour, radius, fill);
        painter.rect_stroke(
            colour,
            radius,
            Stroke::new(BORDER_WIDTH, outline),
            StrokeKind::Inside,
        );
        if chosen {
            painter.rect_stroke(
                colour,
                radius,
                Stroke::new(SWATCH_CHOSEN_WIDTH, tokens.text),
                StrokeKind::Outside,
            );
        }
        if response.has_focus() {
            paint_focus_ring(ui, rect);
        }
    }
    name_button(response, name, Some(chosen)).on_hover_text(name)
}

pub fn instance_toggle(ui: &mut Ui, kept: bool, name: &str, hover: &str) -> Response {
    let side = ui.spacing().interact_size.y;
    let glyph = if kept { icons::DONE } else { "" };
    let button = Button::new(icon(glyph))
        .selected(kept)
        .min_size(Vec2::splat(side));
    ui.add(Named::new(button, name).selected(kept))
        .on_hover_text(hover)
        .on_disabled_hover_text(hover)
}

pub fn removable_row(ui: &mut Ui, text: RichText, hover: &str) -> bool {
    let button_side = ui.spacing().interact_size.y;
    let gap = ui.spacing().item_spacing.x;
    ui.horizontal_top(|ui| {
        let room = (ui.available_width() - button_side - gap).max(button_side);
        ui.allocate_ui_with_layout(vec2(room, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_min_width(room);
            ui.set_max_width(room);
            ui.add(Label::new(text).wrap());
        });
        icon_button(ui, icons::REMOVE, hover).clicked()
    })
    .inner
}

pub fn small_button(ui: &mut Ui, glyph: &str, text: &str) -> Named<Button<'static>> {
    let muted = appearance::tokens(ui).text_muted;
    let button = Button::new((
        icon(glyph).color(muted),
        RichText::new(text.to_owned()).text_style(TextStyle::Body),
    ));
    Named::new(button, text)
}

pub fn button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text.into()).text_style(TextStyle::Body))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Emphasis {
    Primary,
    Danger,
}

impl Emphasis {
    fn fills(self, tokens: &Tokens) -> [Color32; 3] {
        match self {
            Self::Primary => [tokens.accent, tokens.accent_hover, tokens.accent_pressed],
            Self::Danger => [tokens.danger, tokens.danger_hover, tokens.danger_pressed],
        }
    }
}

pub struct EmphasizedButton {
    glyph: Option<String>,
    text: String,
    emphasis: Emphasis,
    min_size: Vec2,
}

pub type PrimaryButton = EmphasizedButton;

impl EmphasizedButton {
    fn new(glyph: Option<&str>, text: impl Into<String>, emphasis: Emphasis) -> Self {
        Self {
            glyph: glyph.map(str::to_owned),
            text: text.into(),
            emphasis,
            min_size: Vec2::ZERO,
        }
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }
}

impl Widget for EmphasizedButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let tokens = appearance::tokens(ui);
        let [rest, hover, pressed] = self.emphasis.fills(tokens);
        let response = ui
            .scope(|ui| {
                let widgets = &mut ui.visuals_mut().widgets;
                for (state, fill) in [
                    (&mut widgets.inactive, rest),
                    (&mut widgets.hovered, hover),
                    (&mut widgets.active, pressed),
                    (&mut widgets.open, pressed),
                ] {
                    state.bg_fill = fill;
                    state.weak_bg_fill = fill;
                    state.bg_stroke = Stroke::NONE;
                    state.fg_stroke = Stroke::new(BORDER_WIDTH, tokens.text_on_accent);
                }
                let text = RichText::new(self.text.clone()).color(tokens.text_on_accent);
                let button = match &self.glyph {
                    Some(glyph) => Button::new((icon(glyph).color(tokens.text_on_accent), text)),
                    None => Button::new(text),
                };
                ui.add(Named::new(
                    button.min_size(self.min_size),
                    self.text.as_str(),
                ))
            })
            .inner;
        if response.has_focus() {
            paint_focus_ring(ui, response.rect.expand(FOCUS_GAP));
        }
        if self.emphasis == Emphasis::Primary && ui.is_enabled() {
            ui.data_mut(|data| data.insert_temp(primary_action_key(), response.id));
        }
        response
    }
}

pub fn primary_button(_ui: &Ui, text: impl Into<String>) -> PrimaryButton {
    EmphasizedButton::new(None, text, Emphasis::Primary)
}

pub fn primary_icon_button(_ui: &Ui, glyph: &str, text: &str) -> PrimaryButton {
    EmphasizedButton::new(Some(glyph), text, Emphasis::Primary)
}

pub fn danger_button(text: impl Into<String>) -> EmphasizedButton {
    EmphasizedButton::new(None, text, Emphasis::Danger)
}

pub fn paint_focus_ring(ui: &Ui, rect: Rect) {
    let focus = appearance::tokens(ui).focus;
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(WIDGET_RADIUS + FOCUS_GAP as u8),
        Stroke::new(FOCUS_WIDTH, focus),
        StrokeKind::Outside,
    );
}

pub fn text_field<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let tokens = appearance::tokens(ui);
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.bg_stroke = Stroke::new(BORDER_WIDTH, tokens.field_border);
        visuals.widgets.hovered.bg_stroke = Stroke::new(BORDER_WIDTH, tokens.text_muted);
        visuals.selection.stroke = Stroke::new(FOCUS_WIDTH, tokens.focus);
        add(ui)
    })
    .inner
}

pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(fonts::semibold())
}

pub struct Segment<'a> {
    pub label: &'a str,
    pub hover: &'a str,
    pub refusal: Option<&'a str>,
}

pub fn segmented(ui: &mut Ui, choices: &[(&str, &str)], selected: usize) -> Option<usize> {
    let segments: Vec<Segment<'_>> = choices
        .iter()
        .map(|(label, hover)| Segment {
            label,
            hover,
            refusal: None,
        })
        .collect();
    segmented_with(ui, &segments, selected)
}

pub fn segmented_with(ui: &mut Ui, segments: &[Segment<'_>], selected: usize) -> Option<usize> {
    if segmented_width(ui, segments) > ui.available_width() + WIDTH_CHANGE {
        segmented_menu(ui, segments, selected)
    } else {
        segmented_row(ui, segments, selected)
    }
}

fn segmented_width(ui: &Ui, segments: &[Segment<'_>]) -> f32 {
    let padding = ui.spacing().button_padding.x;
    let labels: f32 = segments
        .iter()
        .map(|segment| {
            unwrapped(ui, RichText::new(segment.label), TextStyle::Button)
                .size()
                .x
                + 2.0 * padding
        })
        .sum();
    let gaps = SPACE_XS * segments.len().saturating_sub(1) as f32;
    labels + gaps + 2.0 * f32::from(SEGMENT_INSET)
}

fn offered(response: Response, segment: &Segment<'_>) -> Response {
    let response = if segment.hover.is_empty() {
        response
    } else {
        response.on_hover_text(segment.hover)
    };
    match segment.refusal {
        Some(refusal) => response.on_disabled_hover_text(refusal),
        None => response,
    }
}

fn segmented_menu(ui: &mut Ui, segments: &[Segment<'_>], selected: usize) -> Option<usize> {
    let mut chosen = None;
    let current = segments.get(selected).map_or("", |segment| segment.label);
    let salt: Vec<&str> = segments.iter().map(|segment| segment.label).collect();
    let combo = egui::ComboBox::from_id_salt(("segmented", salt))
        .selected_text(current)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (index, segment) in segments.iter().enumerate() {
                let response = ui
                    .add_enabled_ui(segment.refusal.is_none(), |ui| {
                        menu_option(ui, index == selected, segment.label)
                    })
                    .inner;
                let response = offered(response, segment);
                if response.clicked() && index != selected {
                    chosen = Some(index);
                }
            }
        });
    tie_to_caption(ui, &combo.response);
    chosen
}

fn segmented_row(ui: &mut Ui, segments: &[Segment<'_>], selected: usize) -> Option<usize> {
    let tokens = appearance::tokens(ui);
    let mut chosen = None;
    Frame::new()
        .fill(tokens.button)
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(Margin::same(SEGMENT_INSET))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                let segment_height = CONTROL_HEIGHT - 2.0 * f32::from(SEGMENT_INSET);
                for (index, segment) in segments.iter().enumerate() {
                    let current = index == selected;
                    let color = if current {
                        tokens.accent_text
                    } else {
                        tokens.text
                    };
                    let mut button = Button::new(RichText::new(segment.label).color(color))
                        .min_size(vec2(0.0, segment_height))
                        .corner_radius(CornerRadius::same(WIDGET_RADIUS - SEGMENT_INSET as u8));
                    button = if current {
                        button
                            .fill(tokens.raised)
                            .stroke(Stroke::new(BORDER_WIDTH, tokens.accent_text))
                    } else {
                        button.frame_when_inactive(false)
                    };
                    let named = Named::new(button, segment.label).selected(current);
                    let response =
                        offered(ui.add_enabled(segment.refusal.is_none(), named), segment);
                    if response.clicked() && !current {
                        chosen = Some(index);
                    }
                }
            });
        });
    chosen
}

pub fn key_cap(ui: &mut Ui, keys: &str) -> Response {
    let tokens = appearance::tokens(ui);
    let backwards = ui.layout().prefer_right_to_left();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        ui.spacing_mut().interact_size.y = 0.0;
        let mut keys: Vec<&str> = keys.split('+').filter(|key| !key.is_empty()).collect();
        if backwards {
            keys.reverse();
        }
        for key in keys {
            Frame::new()
                .fill(tokens.button)
                .stroke(Stroke::new(BORDER_WIDTH, tokens.border))
                .corner_radius(CornerRadius::same(KEY_CAP_RADIUS))
                .inner_margin(KEY_CAP_MARGIN)
                .show(ui, |ui| {
                    ui.add(
                        Label::new(
                            RichText::new(key)
                                .text_style(TextStyle::Small)
                                .color(tokens.text_muted),
                        )
                        .selectable(false),
                    );
                });
        }
    })
    .response
    .on_hover_text(keys)
}

pub fn empty_state<R>(
    ui: &mut Ui,
    glyph: &str,
    text: &str,
    actions: impl FnOnce(&mut Ui) -> R,
) -> R {
    let tokens = appearance::tokens(ui);
    Frame::new()
        .inner_margin(EMPTY_STATE_MARGIN)
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                icon_label(ui, glyph, tokens.text_muted);
                ui.add(Label::new(RichText::new(text).color(tokens.text_muted)).wrap());
            });
            ui.add_space(SPACE_S);
            ui.horizontal_wrapped(actions).inner
        })
        .inner
}

pub fn stepper(
    ui: &mut Ui,
    value: &str,
    smaller: (&str, bool),
    larger: (&str, bool),
) -> Option<i8> {
    let mut step = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        let (smaller_name, can_shrink) = smaller;
        if ui
            .add_enabled_ui(can_shrink, |ui| {
                icon_button(ui, icons::SUBTRACT, smaller_name)
            })
            .inner
            .clicked()
        {
            step = Some(-1);
        }
        ui.add(
            Label::new(RichText::new(value).family(fonts::medium()))
                .selectable(false)
                .wrap_mode(TextWrapMode::Extend),
        );
        let (larger_name, can_grow) = larger;
        if ui
            .add_enabled_ui(can_grow, |ui| icon_button(ui, icons::ADD, larger_name))
            .inner
            .clicked()
        {
            step = Some(1);
        }
    });
    step
}

pub fn panel_header<R>(
    ui: &mut Ui,
    glyph: &str,
    title: &str,
    actions: impl FnOnce(&mut Ui) -> R,
) -> R {
    let tokens = appearance::tokens(ui);
    Sides::new()
        .show(
            ui,
            |ui| {
                icon_label(ui, glyph, tokens.text_muted);
                ui.add(Label::new(section_title(title)).selectable(false));
            },
            actions,
        )
        .1
}

fn primary_action_key() -> Id {
    Id::new("dialog-primary-action")
}

fn focus_primary_action(ctx: &egui::Context) {
    let primary = ctx.data(|data| data.get_temp::<Id>(primary_action_key()));
    let nothing_focused = ctx.memory(|memory| memory.focused().is_none());
    if let Some(primary) = primary
        && nothing_focused
    {
        ctx.memory_mut(|memory| memory.request_focus(primary));
        ctx.request_repaint();
    }
}

pub fn link_label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let response = ui.add(Label::new(text).selectable(false).sense(Sense::click()));
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        let stroke = Stroke::new(UNDERLINE_WIDTH, ui.visuals().text_color());
        ui.painter()
            .hline(response.rect.x_range(), response.rect.bottom(), stroke);
    }
    response
}

pub fn pill(ui: &mut Ui, tone: Tone, text: impl Into<String>) -> Response {
    let tokens = appearance::tokens(ui);
    Frame::new()
        .fill(tone.fill(tokens))
        .corner_radius(CornerRadius::same(PILL_RADIUS))
        .inner_margin(PILL_MARGIN)
        .show(ui, |ui| {
            ui.add(
                Label::new(
                    RichText::new(text)
                        .text_style(TextStyle::Small)
                        .color(tone.color(tokens)),
                )
                .selectable(false),
            )
        })
        .inner
}

pub fn status_pill(ui: &mut Ui, tone: Tone, text: impl Into<String>) -> Response {
    status_pill_parts(ui, tone, text).0
}

pub fn announced_status_pill(ui: &mut Ui, tone: Tone, text: impl Into<String>) -> Response {
    let (pill, label) = status_pill_parts(ui, tone, text);
    announced(ui, &label, tone == Tone::Error);
    pill
}

fn status_pill_parts(ui: &mut Ui, tone: Tone, text: impl Into<String>) -> (Response, Response) {
    let tokens = appearance::tokens(ui);
    let color = tone.color(tokens);
    let shown = Frame::new()
        .fill(tone.fill(tokens))
        .corner_radius(CornerRadius::same(PILL_RADIUS))
        .inner_margin(PILL_MARGIN)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = PILL_ICON_GAP;
                let glyph = ui.add(
                    Label::new(
                        RichText::new(tone.icon())
                            .font(icon_font(SMALL_SIZE))
                            .color(color),
                    )
                    .selectable(false),
                );
                decorative(ui, &glyph);
                ui.add(
                    Label::new(
                        RichText::new(text)
                            .text_style(TextStyle::Small)
                            .color(color),
                    )
                    .selectable(false),
                )
            })
            .inner
        });
    (shown.response, shown.inner)
}

pub fn callout<R>(ui: &mut Ui, tone: Tone, add: impl FnOnce(&mut Ui) -> R) -> R {
    let tokens = appearance::tokens(ui);
    Frame::new()
        .fill(tone.fill(tokens))
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(CALLOUT_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                icon_label(ui, tone.icon(), tone.color(tokens));
                ui.vertical(add).inner
            })
            .inner
        })
        .inner
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let tokens = appearance::tokens(ui);
    Frame::new()
        .fill(tokens.raised)
        .stroke(Stroke::new(BORDER_WIDTH, tokens.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(CARD_MARGIN))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn column_caption(ui: &mut Ui, text: &str) {
    let tokens = appearance::tokens(ui);
    ui.add(
        Label::new(
            RichText::new(text)
                .text_style(TextStyle::Small)
                .color(tokens.text_muted),
        )
        .selectable(false),
    );
}

pub fn caption(ui: &mut Ui, text: &str) {
    let tokens = appearance::tokens(ui);
    let label = ui.add(
        Label::new(RichText::new(text).color(tokens.text_muted))
            .selectable(false)
            .wrap_mode(TextWrapMode::Extend),
    );
    let key = ui.unique_id().with(CAPTION_KEY);
    ui.data_mut(|data| data.insert_temp(key, label.id));
}

pub fn tie_to_caption(ui: &Ui, field: &Response) {
    let caption = ui
        .stack()
        .iter()
        .find_map(|level| ui.data(|data| data.get_temp::<Id>(level.id.with(CAPTION_KEY))));
    if let Some(caption) = caption {
        field.clone().labelled_by(caption);
    }
}

pub fn section_title(text: &str) -> RichText {
    RichText::new(text).text_style(TextStyle::Name(SECTION.into()))
}

pub fn properties<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    Grid::new(Id::new(("properties", id)))
        .num_columns(2)
        .spacing(PROPERTY_SPACING)
        .min_col_width(CAPTION_WIDTH)
        .show(ui, add)
        .inner
}

pub fn property<R>(ui: &mut Ui, text: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    caption(ui, text);
    let inner = add(ui);
    ui.end_row();
    inner
}

pub fn error_row(ui: &mut Ui, text: &str) {
    ui.label("");
    let error = ui.visuals().error_fg_color;
    ui.horizontal_wrapped(|ui| {
        icon_label(ui, icons::FAILED, error);
        ui.colored_label(error, text);
    });
    ui.end_row();
}

pub fn choose_in_view(ui: &mut Ui, choosing: bool, waiting: &str, hover: &str) -> bool {
    if choosing {
        ui.label(muted(waiting, ui));
        false
    } else {
        let button = small_button(ui, icons::CHOOSE_IN_VIEW, "Choose in the view");
        ui.add(button).on_hover_text(hover).clicked()
    }
}

pub struct SectionAction<'a> {
    pub glyph: &'a str,
    pub hover: &'a str,
}

pub fn reveal_section(ctx: &egui::Context, id: &str) {
    set_section_open(ctx, id, true);
}

pub fn set_section_open(ctx: &egui::Context, id: &str, open: bool) {
    let mut state = CollapsingState::load_with_default_open(ctx, Id::new(("section", id)), true);
    if state.is_open() != open {
        state.set_open(open);
        state.store(ctx);
    }
}

pub fn is_section_open(ctx: &egui::Context, id: &str) -> bool {
    CollapsingState::load_with_default_open(ctx, Id::new(("section", id)), true).is_open()
}

pub fn section(
    ui: &mut Ui,
    id: &str,
    title: &str,
    count: Option<usize>,
    action: Option<SectionAction<'_>>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let header = SectionHeader {
        title,
        count,
        action,
        explanation: None,
    };
    section_with(ui, id, header, body)
}

pub fn panel_section(
    ui: &mut Ui,
    id: &str,
    header: SectionHeader<'_>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    section_with(ui, id, header, body)
}

pub struct SectionHeader<'a> {
    pub title: &'a str,
    pub count: Option<usize>,
    pub action: Option<SectionAction<'a>>,
    pub explanation: Option<&'a str>,
}

fn section_with(
    ui: &mut Ui,
    id: &str,
    header: SectionHeader<'_>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    let tokens = appearance::tokens(ui);
    let mut state =
        CollapsingState::load_with_default_open(ui.ctx(), Id::new(("section", id)), true);
    let row = |ui: &mut Ui| {
        ui.horizontal(|ui| {
            let chevron = if state.is_open() {
                icons::EXPANDED
            } else {
                icons::COLLAPSED
            };
            let button = ui.add(Named::new(
                Button::new((
                    icon(chevron).color(tokens.text_muted),
                    section_title(header.title),
                ))
                .frame_when_inactive(false),
                header.title,
            ));
            let button = match header.explanation {
                Some(explanation) => button.on_hover_text(explanation),
                None => button,
            };
            if let Some(count) = header.count {
                match header.explanation {
                    Some(_) => {
                        pill(ui, Tone::Neutral, count.to_string());
                    }
                    None => {
                        ui.add(
                            Label::new(
                                RichText::new(count.to_string())
                                    .text_style(TextStyle::Small)
                                    .color(tokens.text_muted),
                            )
                            .selectable(false),
                        );
                    }
                }
            }
            let acted = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    header
                        .action
                        .is_some_and(|action| icon_button(ui, action.glyph, action.hover).clicked())
                })
                .inner;
            (button.clicked(), acted)
        })
        .inner
    };
    let (toggled, acted) = match header.explanation {
        Some(_) => {
            Frame::new()
                .fill(tokens.raised)
                .stroke(Stroke::new(BORDER_WIDTH, tokens.border))
                .corner_radius(CornerRadius::same(WIDGET_RADIUS))
                .inner_margin(SECTION_BAND_MARGIN)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    row(ui)
                })
                .inner
        }
        None => row(ui),
    };
    if toggled {
        state.toggle(ui);
    }
    if acted {
        state.set_open(true);
    }
    state.show_body_unindented(ui, |ui| {
        if let Some(explanation) = header.explanation {
            ui.add_space(SPACE_XS);
            ui.add(
                Label::new(
                    RichText::new(explanation)
                        .text_style(TextStyle::Small)
                        .color(tokens.text_muted),
                )
                .wrap(),
            );
            ui.add_space(SPACE_XS);
        }
        body(ui);
    });
    state.store(ui.ctx());
    acted
}

pub fn menu_room(ui: &Ui) -> f32 {
    let screen = ui.ctx().content_rect();
    let margin = ui.spacing().menu_margin;
    let below = screen.bottom() - ui.max_rect().top() - f32::from(margin.bottom) - DIALOG_EDGE;
    let least = (screen.height() - margin.sum().y - 2.0 * DIALOG_EDGE) * MIN_MENU_SHARE;
    below.max(least).max(MIN_LIST_HEIGHT)
}

pub fn fitted_menu<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let room = menu_room(ui);
    ui.set_max_height(room);
    egui::ScrollArea::vertical()
        .max_height(room)
        .show(ui, add)
        .inner
}

pub fn menu_item(ui: &mut Ui, glyph: &str, title: &str, keys: Option<String>) -> Response {
    let muted = appearance::tokens(ui).text_muted;
    let mut button = Button::new((icon(glyph).color(muted), title.to_owned()));
    if let Some(keys) = keys {
        button = button.shortcut_text(RichText::new(keys).text_style(TextStyle::Small));
    }
    ui.add(Named::new(button, title))
}

pub fn menu_choice(
    ui: &mut Ui,
    glyph: &str,
    title: &str,
    keys: Option<String>,
    chosen: bool,
) -> Response {
    let muted = appearance::tokens(ui).text_muted;
    let mut button = Button::new((icon(glyph).color(muted), title.to_owned())).selected(chosen);
    if let Some(keys) = keys {
        button = button.shortcut_text(RichText::new(keys).text_style(TextStyle::Small));
    }
    ui.add(Named::new(button, title).selected(chosen))
}

pub fn corner_menu_button(
    ui: &mut Ui,
    id: Id,
    host: Rect,
    name: &str,
    on_selected: bool,
) -> Response {
    let tokens = appearance::tokens(ui);
    let glyph = unwrapped(
        ui,
        RichText::new(icons::EXPANDED).font(icon_font(SMALL_SIZE)),
        TextStyle::Small,
    );
    let side = glyph.size().max_elem().max(MIN_CORNER_BUTTON_SIDE);
    let rect = Rect::from_min_size(pos2(host.right() - side, host.top()), Vec2::splat(side));
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), name));
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            tokens.pressed
        } else if response.hovered() || open {
            tokens.hover
        } else {
            Color32::TRANSPARENT
        };
        let stroke = if response.has_focus() {
            Stroke::new(FOCUS_WIDTH, tokens.focus)
        } else {
            Stroke::NONE
        };
        let color = if on_selected {
            tokens.accent_text
        } else {
            tokens.text_muted
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(WIDGET_RADIUS),
            fill,
            stroke,
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - glyph.size() / 2.0, glyph, color);
    }
    response.on_hover_text(name)
}

pub struct ToolButton<'a> {
    glyph: &'a str,
    label: &'a str,
    selected: bool,
    compact: bool,
}

impl<'a> ToolButton<'a> {
    pub fn new(glyph: &'a str, label: &'a str) -> Self {
        Self {
            glyph,
            label,
            selected: false,
            compact: false,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

pub fn tool_height(ui: &Ui) -> f32 {
    let glyph = ui.fonts_mut(|fonts| fonts.row_height(&icon_font(TOOL_ICON_SIZE)));
    let natural =
        glyph + TOOL_LABEL_GAP + ui.text_style_height(&TextStyle::Small) + 2.0 * TOOL_PADDING.y;
    natural.max(2.0 * CONTROL_HEIGHT + COMPACT_TOOL_GAP)
}

pub fn compact_tool_side(ui: &Ui) -> f32 {
    (tool_height(ui) - COMPACT_TOOL_GAP) / 2.0
}

fn icon_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, fonts::icons())
}

fn unwrapped(ui: &Ui, text: RichText, style: TextStyle) -> Arc<Galley> {
    WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, style)
}

impl Widget for ToolButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let tokens = appearance::tokens(ui);
        let glyph_size = if self.compact {
            ICON_SIZE
        } else {
            TOOL_ICON_SIZE
        };
        let glyph = unwrapped(
            ui,
            RichText::new(self.glyph).font(icon_font(glyph_size)),
            TextStyle::Body,
        );
        let label = (!self.compact).then(|| {
            unwrapped(
                ui,
                RichText::new(self.label).text_style(TextStyle::Small),
                TextStyle::Small,
            )
        });
        let size = match &label {
            Some(label) => vec2(
                (glyph.size().x.max(label.size().x) + 2.0 * TOOL_PADDING.x).max(TOOL_MIN_WIDTH),
                tool_height(ui),
            ),
            None => Vec2::splat(compact_tool_side(ui)),
        };
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::Button,
                ui.is_enabled(),
                self.selected,
                self.label,
            )
        });
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact_selectable(&response, self.selected);
            let pressed = response.is_pointer_button_down_on();
            let hovered = response.hovered() && ui.is_enabled();
            let fill = match (self.selected, pressed, hovered) {
                (true, true, _) => tokens.pressed,
                (true, false, _) => tokens.accent_subtle,
                (false, true, _) => tokens.pressed,
                (false, false, true) => tokens.hover,
                (false, false, false) => Color32::TRANSPARENT,
            };
            let stroke = if response.has_focus() {
                Stroke::new(FOCUS_WIDTH, tokens.focus)
            } else if self.selected {
                Stroke::new(SELECTED_WIDTH, tokens.accent_text)
            } else if hovered {
                Stroke::new(SELECTED_WIDTH, tokens.border_strong)
            } else {
                Stroke::NONE
            };
            let radius = CornerRadius::same(WIDGET_RADIUS);
            ui.painter()
                .rect(rect, radius, fill, stroke, StrokeKind::Inside);
            let color = if self.selected {
                tokens.accent_text
            } else {
                visuals.text_color()
            };
            match label {
                Some(label) => {
                    let top = rect.top() + TOOL_PADDING.y;
                    let glyph_pos = egui::pos2(rect.center().x - glyph.size().x / 2.0, top);
                    let label_pos = egui::pos2(
                        rect.center().x - label.size().x / 2.0,
                        top + glyph.size().y + TOOL_LABEL_GAP,
                    );
                    ui.painter().galley(glyph_pos, glyph, color);
                    ui.painter().galley(label_pos, label, color);
                }
                None => {
                    let glyph_pos = rect.center() - glyph.size() / 2.0;
                    ui.painter().galley(glyph_pos, glyph, color);
                }
            }
        }
        response
    }
}

pub struct Tab<'a> {
    pub glyph: &'a str,
    pub label: &'a str,
}

struct TabButton<'a> {
    id: Id,
    tab: &'a Tab<'a>,
    selected: bool,
}

impl Widget for TabButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let tokens = appearance::tokens(ui);
        let glyph = WidgetText::from(icon(self.tab.glyph)).into_galley(
            ui,
            Some(TextWrapMode::Extend),
            f32::INFINITY,
            TextStyle::Body,
        );
        let label = WidgetText::from(RichText::new(self.tab.label).text_style(TextStyle::Body))
            .into_galley(
                ui,
                Some(TextWrapMode::Extend),
                f32::INFINITY,
                TextStyle::Body,
            );
        let content_height = glyph.size().y.max(label.size().y);
        let size = vec2(
            glyph.size().x + TAB_ICON_GAP + label.size().x + 2.0 * TAB_PADDING.x,
            content_height + 2.0 * TAB_PADDING.y,
        );
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let response = ui.interact(rect, self.id, Sense::click());
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::Button,
                ui.is_enabled(),
                self.selected,
                self.tab.label,
            )
        });
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_role(Role::Tab));
        if ui.is_rect_visible(rect) {
            let fill = if self.selected {
                tokens.accent_subtle
            } else if response.is_pointer_button_down_on() {
                tokens.pressed
            } else if response.hovered() {
                tokens.hover
            } else {
                Color32::TRANSPARENT
            };
            let stroke = if response.has_focus() {
                Stroke::new(FOCUS_WIDTH, tokens.focus)
            } else {
                Stroke::NONE
            };
            let radius = CornerRadius {
                nw: WIDGET_RADIUS,
                ne: WIDGET_RADIUS,
                sw: 0,
                se: 0,
            };
            ui.painter()
                .rect(rect, radius, fill, stroke, StrokeKind::Inside);
            let color = if self.selected {
                tokens.accent_text
            } else {
                tokens.text
            };
            let glyph_color = if self.selected {
                tokens.accent_text
            } else {
                tokens.text_muted
            };
            if self.selected {
                let underline = rect.bottom() - TAB_UNDERLINE / 2.0;
                ui.painter().hline(
                    rect.x_range(),
                    underline,
                    Stroke::new(TAB_UNDERLINE, tokens.accent),
                );
            }
            let left = rect.left() + TAB_PADDING.x;
            let glyph_pos = egui::pos2(left, rect.center().y - glyph.size().y / 2.0);
            let label_pos = egui::pos2(
                left + glyph.size().x + TAB_ICON_GAP,
                rect.center().y - label.size().y / 2.0,
            );
            ui.painter().galley(glyph_pos, glyph, glyph_color);
            ui.painter().galley(label_pos, label, color);
        }
        response
    }
}

pub fn tab_id(tabs: &str, index: usize) -> Id {
    Id::new(("tabs", tabs)).with(("tab", index))
}

fn switch_keys_step(ui: &mut Ui) -> Option<isize> {
    ui.input_mut(|input| {
        let backward = input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab)
            || input.consume_key(Modifiers::COMMAND, Key::PageUp);
        let forward = input.consume_key(Modifiers::COMMAND, Key::Tab)
            || input.consume_key(Modifiers::COMMAND, Key::PageDown);
        match (backward, forward) {
            (true, false) => Some(-1),
            (false, true) => Some(1),
            _ => None,
        }
    })
}

fn arrow_step(ui: &mut Ui, index: usize, count: usize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    ui.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
            stepped(index, 1, count)
        } else if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            stepped(index, -1, count)
        } else if input.consume_key(Modifiers::NONE, Key::Home) {
            Some(0)
        } else if input.consume_key(Modifiers::NONE, Key::End) {
            Some(last)
        } else {
            None
        }
    })
}

fn stepped(index: usize, step: isize, count: usize) -> Option<usize> {
    let count = isize::try_from(count).ok().filter(|count| *count > 0)?;
    let index = isize::try_from(index).ok()?;
    usize::try_from((index + step).rem_euclid(count)).ok()
}

pub fn tabs(
    ui: &mut Ui,
    id: &str,
    tabs: &[Tab<'_>],
    selected: usize,
    switch_keys: bool,
) -> Option<usize> {
    let count = tabs.len();
    let mut chosen = switch_keys
        .then(|| switch_keys_step(ui))
        .flatten()
        .and_then(|step| stepped(selected, step, count));
    let mut focus = None;
    let row = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = TAB_GAP;
        for (index, tab) in tabs.iter().enumerate() {
            let response = ui.add(TabButton {
                id: tab_id(id, index),
                tab,
                selected: index == selected,
            });
            if response.clicked() {
                chosen = Some(index);
            }
            if response.has_focus()
                && let Some(next) = arrow_step(ui, index, count)
            {
                chosen = Some(next);
                focus = Some(next);
            }
        }
    });
    let border = appearance::tokens(ui).border;
    ui.painter().hline(
        row.response.rect.x_range(),
        row.response.rect.bottom(),
        Stroke::new(UNDERLINE_WIDTH, border),
    );
    if let Some(next) = focus {
        ui.memory_mut(|memory| {
            memory.move_focus(FocusDirection::None);
            memory.request_focus(tab_id(id, next));
        });
    }
    chosen.filter(|index| *index != selected && *index < count)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogWidth {
    Medium,
    Wide,
}

impl DialogWidth {
    pub fn points(self) -> f32 {
        match self {
            Self::Medium => 460.0,
            Self::Wide => 600.0,
        }
    }
}

pub struct DialogResponse<T> {
    pub inner: T,
    close: bool,
}

impl<T> DialogResponse<T> {
    pub fn should_close(&self) -> bool {
        self.close
    }
}

pub fn dialog<T>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: DialogWidth,
    add: impl FnOnce(&mut Ui) -> T,
) -> DialogResponse<T> {
    let mut closed = false;
    ctx.data_mut(|data| data.remove::<Id>(primary_action_key()));
    let response = Modal::new(Id::new(id))
        .frame(dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(fitting_width(ctx, width.points()));
            Sides::new().show(
                ui,
                |ui| ui.heading(title),
                |ui| closed = icon_button(ui, icons::CLOSE, "Close (Esc)").clicked(),
            );
            ui.add_space(ui.spacing().item_spacing.y);
            let inner = add(ui);
            focus_primary_action(ui.ctx());
            inner
        });
    let close = closed || response.should_close();
    DialogResponse {
        inner: response.inner,
        close,
    }
}

pub fn remembered_width(ui: &Ui, id: Id) -> f32 {
    ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0)
}

pub fn remember_width(ui: &Ui, id: Id, width: f32) {
    let known = ui.data(|data| data.get_temp::<f32>(id));
    if known.is_none_or(|known| (known - width).abs() > WIDTH_CHANGE) {
        ui.data_mut(|data| data.insert_temp(id, width));
        ui.ctx()
            .request_discard("a bar's trailing items changed width");
    }
}

pub fn fitting_width(ctx: &egui::Context, wanted: f32) -> f32 {
    let room = ctx.content_rect().width() - 2.0 * (DIALOG_EDGE + f32::from(DIALOG_MARGIN));
    wanted.min(room).max(MIN_DIALOG_WIDTH)
}

pub fn list_height(ctx: &egui::Context, wanted: f32) -> f32 {
    wanted
        .min(ctx.content_rect().height() * LIST_SCREEN_SHARE)
        .max(MIN_LIST_HEIGHT)
}

pub fn dialog_frame(ctx: &egui::Context) -> Frame {
    let visuals = ctx.global_style().visuals.clone();
    Frame::new()
        .fill(visuals.window_fill)
        .stroke(visuals.window_stroke)
        .corner_radius(visuals.window_corner_radius)
        .shadow(visuals.window_shadow)
        .inner_margin(Margin::same(DIALOG_MARGIN))
}

pub fn footer<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add_space(DIALOG_FOOTER_GAP);
    Sides::new().show(ui, |_| {}, add).1
}

pub fn footer_split<T>(
    ui: &mut Ui,
    start: impl FnOnce(&mut Ui) -> Option<T>,
    end: impl FnOnce(&mut Ui) -> Option<T>,
) -> Option<T> {
    footer(ui, |ui| {
        let ended = end(ui);
        let started = ui
            .with_layout(Layout::left_to_right(Align::Center), start)
            .inner;
        ended.or(started)
    })
}

pub fn menu_option(ui: &mut Ui, chosen: bool, title: &str) -> Response {
    ui.add(Named::new(Button::selectable(chosen, title), title).selected(chosen))
}

pub fn menu_item_with_detail(ui: &mut Ui, glyph: &str, title: &str, detail: &str) -> Response {
    let muted = appearance::tokens(ui).text_muted;
    let button = Button::new((
        icon(glyph).color(muted),
        title.to_owned(),
        egui::Atom::grow(),
        RichText::new(detail.to_owned())
            .text_style(TextStyle::Small)
            .color(muted),
    ));
    ui.add(Named::new(button, title))
}

#[cfg(test)]
mod tests {
    use egui::{Context, RawInput, Rect, pos2};

    use super::*;

    #[test]
    fn a_long_removable_row_wraps_to_keep_its_button_in_the_panel() {
        let context = Context::default();
        context.set_fonts(fonts::definitions());
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(220.0, 400.0));
        let description = "Edge between Extrude 1 side from Line 3 and Extrude 1 end cap".repeat(3);
        let mut measured = None;

        for _ in 0..2 {
            let input = RawInput {
                screen_rect: Some(screen),
                ..RawInput::default()
            };
            let mut output = context.run_ui(input, |ui| {
                let row = ui
                    .scope(|ui| removable_row(ui, RichText::new(description.as_str()), "Remove"))
                    .response
                    .rect;
                measured = Some((row, ui.max_rect(), ui.text_style_height(&TextStyle::Body)));
            });
            output.textures_delta.clear();
        }
        let (row, panel, line) = measured.unwrap();

        assert!(row.right() <= panel.right() + 0.5, "{row:?} in {panel:?}");
        assert!(row.height() > 2.0 * line, "{row:?}");
    }
}
