use std::sync::Arc;

use egui::{
    Align, Button, Color32, CornerRadius, CursorIcon, FocusDirection, Frame, Galley, Grid, Id, Key,
    Label, Layout, Margin, Modal, Modifiers, Popup, Rect, Response, RichText, Sense, Sides, Stroke,
    StrokeKind, TextStyle, TextWrapMode, Ui, Vec2, Widget, WidgetInfo, WidgetText, WidgetType,
    accesskit::Role, collapsing_header::CollapsingState, pos2, vec2,
};

use crate::{
    appearance::{
        self, CARD_RADIUS, ICON_SIZE, SECTION, SMALL_SIZE, TOOL_ICON_SIZE, Tokens, WIDGET_RADIUS,
    },
    fonts, icons,
};

const DIALOG_EDGE: f32 = 8.0;
const MIN_DIALOG_WIDTH: f32 = 240.0;
const LIST_SCREEN_SHARE: f32 = 0.4;
const MIN_LIST_HEIGHT: f32 = 96.0;
pub const FIELD_WIDTH: f32 = 120.0;
pub const NAME_FIELD_WIDTH: f32 = 180.0;
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
const FOCUS_WIDTH: f32 = 2.0;
const SELECTED_WIDTH: f32 = 1.0;
pub const COMPACT_TOOL_GAP: f32 = 2.0;
const DIALOG_MARGIN: i8 = 20;
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
    let button = Button::new(icon(glyph).color(muted))
        .frame_when_inactive(false)
        .min_size(vec2(
            ui.spacing().interact_size.y,
            ui.spacing().interact_size.y,
        ));
    ui.add(Named::new(button, hover)).on_hover_text(hover)
}

pub fn removable_row(ui: &mut Ui, text: RichText, hover: &str) -> bool {
    Sides::new()
        .shrink_left()
        .wrap()
        .show(
            ui,
            |ui| ui.label(text),
            |ui| icon_button(ui, icons::REMOVE, hover).clicked(),
        )
        .1
}

pub fn small_button(ui: &mut Ui, glyph: &str, text: &str) -> Named<Button<'static>> {
    let muted = appearance::tokens(ui).text_muted;
    let button = Button::new((
        icon(glyph).color(muted),
        RichText::new(text.to_owned()).text_style(TextStyle::Body),
    ));
    Named::new(button, text)
}

pub fn primary_button(ui: &Ui, text: impl Into<String>) -> Button<'static> {
    let tokens = appearance::tokens(ui);
    Button::new(RichText::new(text).color(tokens.text_on_accent))
        .fill(tokens.accent)
        .stroke(Stroke::NONE)
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
    let tokens = appearance::tokens(ui);
    let color = tone.color(tokens);
    Frame::new()
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
                );
            });
        })
        .response
}

pub fn primary_icon_button(ui: &Ui, glyph: &str, text: &str) -> Button<'static> {
    let tokens = appearance::tokens(ui);
    Button::new((
        icon(glyph).color(tokens.text_on_accent),
        RichText::new(text.to_owned()).color(tokens.text_on_accent),
    ))
    .fill(tokens.accent)
    .stroke(Stroke::NONE)
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
        .stroke(Stroke::new(1.0, tokens.border))
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
    let tokens = appearance::tokens(ui);
    let mut state =
        CollapsingState::load_with_default_open(ui.ctx(), Id::new(("section", id)), true);
    let (toggled, acted) = ui
        .horizontal(|ui| {
            let chevron = if state.is_open() {
                icons::EXPANDED
            } else {
                icons::COLLAPSED
            };
            let header = ui.add(Named::new(
                Button::new((icon(chevron).color(tokens.text_muted), section_title(title)))
                    .frame_when_inactive(false),
                title,
            ));
            if let Some(count) = count {
                ui.add(
                    Label::new(
                        RichText::new(count.to_string())
                            .text_style(TextStyle::Small)
                            .color(tokens.text_muted),
                    )
                    .selectable(false),
                );
            }
            let acted = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    action
                        .is_some_and(|action| icon_button(ui, action.glyph, action.hover).clicked())
                })
                .inner;
            (header.clicked(), acted)
        })
        .inner;
    if toggled {
        state.toggle(ui);
    }
    if acted {
        state.set_open(true);
    }
    state.show_body_unindented(ui, body);
    state.store(ui.ctx());
    acted
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
    let side = glyph.size().max_elem();
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
    glyph + TOOL_LABEL_GAP + ui.text_style_height(&TextStyle::Small) + 2.0 * TOOL_PADDING.y
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
            } else if self.selected {
                Stroke::new(SELECTED_WIDTH, tokens.accent_text)
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
    fn points(self) -> f32 {
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
            add(ui)
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
