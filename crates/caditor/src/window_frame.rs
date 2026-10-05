use egui::{
    Align, Area, Context, CornerRadius, CursorIcon, Id, LayerId, Layout, Order, PointerButton,
    Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, ViewportCommand, WidgetInfo,
    WidgetType, pos2, vec2, viewport::ResizeDirection,
};

use crate::{
    appearance::{self, BORDER_WIDTH, CONTROL_HEIGHT, FOCUS_WIDTH, SPACE_XS, WIDGET_RADIUS},
    commands::{Command, CommandFrame},
    files::FileCommand,
    icons,
    model::Action,
    preferences::{PreferenceChange, PreferencesCommand, TitleBar},
    widgets,
};

pub const MINIMIZE: &str = "Minimize";
pub const MAXIMIZE: &str = "Maximize";
pub const RESTORE: &str = "Restore";
pub const CLOSE: &str = "Close";
pub const LEAVE_FULL_SCREEN: &str = "Leave full screen";
pub const FULL_SCREEN: &str = "Full screen";
pub const SYSTEM_TITLE_BAR: &str = "Use the system title bar";
const IN_FULL_SCREEN: &str = "Leave full screen first";
const EDGE: f32 = 5.0;
const CORNER: f32 = 16.0;
const CONTROL_SIZE: Vec2 = vec2(40.0, CONTROL_HEIGHT);
const CONTROL_GAP: f32 = SPACE_XS;
const BAR_RECT_KEY: &str = "title-bar-rect";
const CONTROLS_ROW_KEY: &str = "title-bar-controls-row";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub focused: bool,
}

impl WindowState {
    pub fn of(ctx: &Context) -> Self {
        ctx.input(|input| {
            let viewport = input.viewport();
            Self {
                maximized: viewport.maximized.unwrap_or(false),
                fullscreen: viewport.fullscreen.unwrap_or(false),
                focused: viewport.focused.unwrap_or(true),
            }
        })
    }

    fn framed(self) -> bool {
        !self.maximized && !self.fullscreen
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chrome {
    pub state: WindowState,
    pub title_bar: TitleBar,
}

impl Chrome {
    pub fn of(ctx: &Context, title_bar: TitleBar) -> Self {
        Self {
            state: WindowState::of(ctx),
            title_bar,
        }
    }

    pub fn built_in(self) -> bool {
        !self.title_bar.decorated()
    }
}

pub fn commands(ctx: &Context, state: WindowState, commands: &mut CommandFrame<'_>) {
    if commands.available(Command::MinimizeWindow) {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    let maximizable = if state.fullscreen {
        Err(IN_FULL_SCREEN)
    } else {
        Ok(())
    };
    if commands.invoke(Command::MaximizeWindow, &maximizable) {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!state.maximized));
    }
    if commands.available(Command::FullScreen) {
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!state.fullscreen));
    }
}

pub fn apply_title_bar(ctx: &Context, applied: &mut Option<TitleBar>, wanted: TitleBar) {
    if *applied != Some(wanted) {
        if applied.is_some() {
            ctx.send_viewport_cmd(ViewportCommand::Decorations(wanted.decorated()));
        }
        *applied = Some(wanted);
    }
}

pub fn drag_region(ui: &Ui, chrome: Chrome) -> Option<Response> {
    let rect = ui.data(|data| data.get_temp::<Rect>(Id::new(BAR_RECT_KEY)))?;
    chrome.built_in().then(|| {
        ui.interact(rect, Id::new("title-bar-drag"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::Default)
    })
}

pub fn remember_bar(ctx: &Context, rect: Rect) {
    ctx.data_mut(|data| data.insert_temp(Id::new(BAR_RECT_KEY), rect));
}

pub fn remember_controls_row(ctx: &Context, rect: Rect) {
    ctx.data_mut(|data| data.insert_temp(Id::new(CONTROLS_ROW_KEY), rect));
}

pub fn drags_window(
    response: &Response,
    chrome: Chrome,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let state = chrome.state;
    if response.double_clicked_by(PointerButton::Primary) && !state.fullscreen {
        response
            .ctx
            .send_viewport_cmd(ViewportCommand::Maximized(!state.maximized));
    } else if response.drag_started_by(PointerButton::Primary) {
        response.ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    response.context_menu(|ui| window_menu(ui, chrome, commands, actions));
}

fn window_menu(
    ui: &mut Ui,
    chrome: Chrome,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let state = chrome.state;
    let item = |ui: &mut Ui, glyph: &str, title: &str, command: Option<Command>| {
        widgets::menu_item(ui, glyph, title, command.and_then(|c| commands.keys(c)))
    };
    if item(ui, icons::MINIMIZE, MINIMIZE, None).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    let (glyph, title) = maximize_button(state);
    let maximize = ui
        .add_enabled_ui(!state.fullscreen, |ui| item(ui, glyph, title, None))
        .inner
        .on_disabled_hover_text(IN_FULL_SCREEN);
    if maximize.clicked() {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!state.maximized));
    }
    let (glyph, title) = if state.fullscreen {
        (icons::LEAVE_FULL_SCREEN, LEAVE_FULL_SCREEN)
    } else {
        (icons::FULL_SCREEN, FULL_SCREEN)
    };
    if item(ui, glyph, title, Some(Command::FullScreen)).clicked() {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Fullscreen(!state.fullscreen));
    }
    ui.separator();
    let mut system = chrome.title_bar.decorated();
    if ui
        .checkbox(&mut system, SYSTEM_TITLE_BAR)
        .on_hover_text("Let the window manager draw the title bar; also in Preferences")
        .changed()
    {
        let bar = if system {
            TitleBar::System
        } else {
            TitleBar::BuiltIn
        };
        actions.push(Action::Preferences(PreferencesCommand::Change(
            PreferenceChange::TitleBar(bar),
        )));
        ui.close();
    }
    ui.separator();
    if item(ui, icons::CLOSE, CLOSE, Some(Command::Quit)).clicked() {
        actions.push(Action::File(FileCommand::Quit));
    }
}

fn maximize_button(state: WindowState) -> (&'static str, &'static str) {
    if state.maximized {
        (icons::RESTORE, RESTORE)
    } else {
        (icons::MAXIMIZE, MAXIMIZE)
    }
}

pub fn controls(
    ui: &mut Ui,
    state: WindowState,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    ui.spacing_mut().item_spacing.x = CONTROL_GAP;
    let close = commands.with_keys(Command::Quit, CLOSE);
    if control(ui, icons::CLOSE, CLOSE, &close, true).clicked() {
        actions.push(Action::File(FileCommand::Quit));
    }
    if state.fullscreen {
        let hover = commands.with_keys(Command::FullScreen, LEAVE_FULL_SCREEN);
        if control(
            ui,
            icons::LEAVE_FULL_SCREEN,
            LEAVE_FULL_SCREEN,
            &hover,
            false,
        )
        .clicked()
        {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Fullscreen(false));
        }
        return;
    }
    let (glyph, name) = maximize_button(state);
    if control(ui, glyph, name, name, false).clicked() {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!state.maximized));
    }
    if control(ui, icons::MINIMIZE, MINIMIZE, MINIMIZE, false).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn reaching_the_corner(ui: &mut Ui, button: &Response) -> Response {
    let screen = ui.ctx().content_rect();
    let rect = button.rect;
    let reach = Rect::from_min_max(
        pos2(rect.min.x, screen.min.y),
        pos2(screen.max.x, rect.max.y),
    );
    let corner = ui
        .scope(|ui| {
            ui.set_clip_rect(reach);
            ui.interact(reach, button.id.with("corner"), Sense::click())
        })
        .inner;
    button.union(corner).with_new_rect(rect)
}

fn control(ui: &mut Ui, glyph: &str, name: &str, hover: &str, closes: bool) -> Response {
    let tokens = appearance::tokens(ui);
    let (rect, response) = ui.allocate_exact_size(CONTROL_SIZE, Sense::click());
    let response = if closes {
        reaching_the_corner(ui, &response)
    } else {
        response
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), name));
    if ui.is_rect_visible(rect) {
        let pressed = response.is_pointer_button_down_on();
        let hovered = response.hovered();
        let fill = match (closes, pressed, hovered) {
            (true, true, _) => tokens.danger_pressed,
            (true, false, true) => tokens.danger,
            (false, true, _) => tokens.pressed,
            (false, false, true) => tokens.hover,
            (_, false, false) => egui::Color32::TRANSPARENT,
        };
        let stroke = if response.has_focus() {
            Stroke::new(FOCUS_WIDTH, tokens.focus)
        } else {
            Stroke::NONE
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(WIDGET_RADIUS),
            fill,
            stroke,
            StrokeKind::Inside,
        );
        let color = match (closes, pressed || hovered) {
            (true, true) => tokens.text_on_accent,
            (false, true) => tokens.text,
            (_, false) => tokens.text_muted,
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::new(appearance::ICON_SIZE, crate::fonts::icons()),
            color,
        );
    }
    response.on_hover_text(hover)
}

pub fn over_dialogs(
    ctx: &Context,
    chrome: Chrome,
    commands: &CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let Some(bar) = ctx.data(|data| data.get_temp::<Rect>(Id::new(BAR_RECT_KEY))) else {
        return;
    };
    if !chrome.built_in() {
        return;
    }
    let id = Id::new("title-bar-over-dialogs");
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(bar.min)
        .constrain(false)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_min_size(bar.size());
            let response = ui.interact(bar, id.with("drag"), Sense::click_and_drag());
            drags_window(&response, chrome, commands, actions);
            let row = ctx
                .data(|data| data.get_temp::<Rect>(Id::new(CONTROLS_ROW_KEY)))
                .unwrap_or_else(|| bar.shrink2(vec2(ui.spacing().item_spacing.x, 0.0)));
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(row)
                    .layout(Layout::right_to_left(Align::Center)),
                |ui| controls(ui, chrome.state, commands, actions),
            );
        });
    ctx.move_to_top(LayerId::new(Order::Foreground, id));
}

pub fn frame(ctx: &Context, chrome: Chrome) {
    if !chrome.built_in() || !chrome.state.framed() {
        return;
    }
    let screen = ctx.content_rect();
    let outline = LayerId::new(Order::Foreground, Id::new("window-outline"));
    let border = ctx
        .global_style()
        .visuals
        .widgets
        .noninteractive
        .bg_stroke
        .color;
    ctx.layer_painter(outline).rect_stroke(
        screen,
        CornerRadius::ZERO,
        Stroke::new(BORDER_WIDTH, border),
        StrokeKind::Inside,
    );
    ctx.move_to_top(outline);
    for edge in Edge::ALL {
        let rect = edge.rect(screen);
        let id = Id::new(("window-edge", edge));
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .interactable(true)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
                let pointer = ctx.input(|input| input.pointer.interact_pos());
                let Some(pointer) = pointer.filter(|_| response.contains_pointer()) else {
                    return;
                };
                let direction = direction_at(screen, pointer);
                ctx.set_cursor_icon(cursor(direction));
                if response.is_pointer_button_down_on()
                    && ctx.input(|input| input.pointer.primary_pressed())
                {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
                }
            });
        ctx.move_to_top(LayerId::new(Order::Foreground, id));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    const ALL: [Self; 4] = [Self::Top, Self::Bottom, Self::Left, Self::Right];

    fn rect(self, screen: Rect) -> Rect {
        match self {
            Self::Top => Rect::from_min_max(screen.min, pos2(screen.max.x, screen.min.y + EDGE)),
            Self::Bottom => Rect::from_min_max(pos2(screen.min.x, screen.max.y - EDGE), screen.max),
            Self::Left => Rect::from_min_max(screen.min, pos2(screen.min.x + EDGE, screen.max.y)),
            Self::Right => Rect::from_min_max(pos2(screen.max.x - EDGE, screen.min.y), screen.max),
        }
    }
}

fn direction_at(screen: Rect, pointer: egui::Pos2) -> ResizeDirection {
    let left = pointer.x < screen.min.x + CORNER;
    let right = pointer.x > screen.max.x - CORNER;
    let top = pointer.y < screen.min.y + CORNER;
    let bottom = pointer.y > screen.max.y - CORNER;
    let near_top = pointer.y < screen.min.y + EDGE;
    let near_bottom = pointer.y > screen.max.y - EDGE;
    let near_left = pointer.x < screen.min.x + EDGE;
    match (top, bottom, left, right) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (true, _, _, true) => ResizeDirection::NorthEast,
        (_, true, true, _) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        _ if near_top => ResizeDirection::North,
        _ if near_bottom => ResizeDirection::South,
        _ if near_left => ResizeDirection::West,
        _ => ResizeDirection::East,
    }
}

fn cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_border_resizes_towards_the_nearest_side_and_corners_diagonally() {
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));

        let cases = [
            (pos2(2.0, 2.0), ResizeDirection::NorthWest),
            (pos2(10.0, 2.0), ResizeDirection::NorthWest),
            (pos2(400.0, 2.0), ResizeDirection::North),
            (pos2(798.0, 10.0), ResizeDirection::NorthEast),
            (pos2(798.0, 300.0), ResizeDirection::East),
            (pos2(2.0, 300.0), ResizeDirection::West),
            (pos2(400.0, 598.0), ResizeDirection::South),
            (pos2(2.0, 598.0), ResizeDirection::SouthWest),
            (pos2(790.0, 598.0), ResizeDirection::SouthEast),
        ];

        for (pointer, expected) in cases {
            assert_eq!(direction_at(screen, pointer), expected, "{pointer:?}");
        }
    }
}
