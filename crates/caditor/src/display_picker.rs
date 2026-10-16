use egui::{Id, Rect, Response, Ui, pos2, vec2};

use crate::{
    canvas, commands::Command, display_style::DisplayStyle, icons, menu_bar::MenuEntries, ribbon,
    view_cube::Corner,
};

pub const NAME: &str = "Display style";
const HOVER: &str = "Choose how bodies are drawn, from shaded with edges to the rendered view lit \
                     by a sun";
const SECTIONS: [&[Command]; 2] = [
    &[
        Command::Style(DisplayStyle::ShadedWithEdges),
        Command::Style(DisplayStyle::ShadedWithHiddenEdges),
        Command::Style(DisplayStyle::Shaded),
        Command::Style(DisplayStyle::Wireframe),
        Command::Style(DisplayStyle::HiddenLine),
        Command::Style(DisplayStyle::XRay),
        Command::Style(DisplayStyle::Rendered),
    ],
    &[Command::Lighting],
];

pub fn name_of(style: DisplayStyle) -> String {
    format!("{NAME}: {}", style.title())
}

pub fn area(ui: &Ui, viewport: Rect, cube: Option<Corner>, style: DisplayStyle) -> Rect {
    let width = canvas::menu_button_width(ui, glyph(style), style.short_title());
    let left = viewport.left() + canvas::MARGIN;
    let right = viewport.right() - canvas::MARGIN - width;
    let x = match cube {
        Some(Corner::TopLeft) => right,
        Some(Corner::TopRight | Corner::BottomLeft | Corner::BottomRight) | None => left,
    };
    Rect::from_min_size(
        pos2(x.max(left), viewport.top() + canvas::MARGIN),
        vec2(width, canvas::BUTTON_HEIGHT),
    )
}

pub fn show(ui: &Ui, rect: Rect, style: DisplayStyle) -> Response {
    canvas::menu_button(
        ui,
        rect,
        Id::unique("display-style-picker"),
        (glyph(style), style.short_title()),
        &name_of(style),
    )
    .on_hover_text(format!("{}. {HOVER}.", name_of(style)))
}

pub fn menu(button: &Response, entries: MenuEntries<'_>) -> Vec<Command> {
    ribbon::group_menu(button, entries, |ui, entries| {
        ribbon::menu_sections(
            ui,
            entries,
            &SECTIONS,
            |ui, entries, command| match command {
                Command::Style(_) => entries.choice(ui, command),
                _ => entries.item(ui, command),
            },
        );
    })
}

fn glyph(style: DisplayStyle) -> &'static str {
    icons::command(Command::Style(style))
}
