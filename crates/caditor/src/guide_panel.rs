use egui::{
    Align, Id, Label, Layout, Response, RichText, ScrollArea, Sense, TextEdit, TextStyle, Ui,
    WidgetInfo, WidgetType,
};

use crate::{
    appearance::{self, SPACE_L, SPACE_M, SPACE_S},
    commands::{self, Keymap},
    fonts,
    guide::{self, Block, Chapter, Hit, Named, Page, Span, Target},
    icons, layout,
    widgets::{self, Tone},
};

pub const TITLE: &str = "User guide";
pub const CLOSE: &str = "Close the guide";
pub const BACK: &str = "Back";
pub const CONTENTS: &str = "Contents";
pub const SEARCH_HINT: &str = "Search the guide";
const PANEL_WIDTH: f32 = 340.0;
const MIN_PANEL_WIDTH: f32 = 240.0;
const NEXT: &str = "Next:";
const UNDERLINE_WIDTH: f32 = 1.0;
const PAGE_TITLE_SIZE: f32 = 20.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Shown {
    #[default]
    Contents,
    Page(Page),
}

#[derive(Debug, Clone, Default)]
pub struct Guide {
    pub open: bool,
    shown: Shown,
    back: Vec<Shown>,
    query: String,
    focus_search: bool,
}

impl Guide {
    #[cfg(test)]
    pub fn page(&self) -> Option<Page> {
        match self.shown {
            Shown::Contents => None,
            Shown::Page(page) => Some(page),
        }
    }

    pub fn open_at(&mut self, page: Page) {
        self.go(Shown::Page(page));
        self.open = true;
    }

    pub fn toggle_at(&mut self, page: Option<Page>) {
        let showing = self.open && self.query.is_empty();
        match page {
            Some(page) if !(showing && self.shown == Shown::Page(page)) => self.open_at(page),
            None if !self.open => {
                self.open = true;
                self.focus_search = true;
            }
            Some(_) | None => self.open = false,
        }
    }

    fn go(&mut self, shown: Shown) {
        self.query.clear();
        if self.open && self.shown != shown {
            self.back.push(self.shown);
        }
        self.shown = shown;
    }

    fn go_back(&mut self) {
        self.query.clear();
        if let Some(previous) = self.back.pop() {
            self.shown = previous;
        }
    }
}

enum Choice {
    Close,
    Back,
    Show(Shown),
}

pub fn show(ui: &mut Ui, state: &mut Guide, keymap: &Keymap, room: f32) {
    let mut choice = None;
    egui::Panel::right("guide")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .size_range(layout::panel_widths(room, MIN_PANEL_WIDTH))
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::GUIDE, TITLE, |ui| {
                if widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked() {
                    choice = Some(Choice::Close);
                }
                let contents = ui
                    .add_enabled_ui(state.shown != Shown::Contents, |ui| {
                        widgets::icon_button(ui, icons::CONTENTS, CONTENTS)
                    })
                    .inner;
                if contents.clicked() {
                    choice = Some(Choice::Show(Shown::Contents));
                }
                let back = ui
                    .add_enabled_ui(!state.back.is_empty(), |ui| {
                        widgets::icon_button(ui, icons::BACK, BACK)
                    })
                    .inner;
                if back.clicked() {
                    choice = Some(Choice::Back);
                }
            });
            ui.add_space(SPACE_S);
            search_field(ui, state);
            ui.add_space(SPACE_M);
            let salt = match state.shown {
                Shown::Contents => "contents",
                Shown::Page(page) => page.id(),
            };
            let query = state.query.trim().to_owned();
            ScrollArea::vertical()
                .id_salt(("guide", salt, query.is_empty()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let chosen = if !query.is_empty() {
                        hits(ui, &query, &guide::search(&query))
                    } else {
                        match state.shown {
                            Shown::Contents => contents(ui),
                            Shown::Page(page) => page_body(ui, page, keymap),
                        }
                    };
                    if let Some(page) = chosen {
                        choice = Some(Choice::Show(Shown::Page(page)));
                    }
                });
        });
    match choice {
        Some(Choice::Close) => state.open = false,
        Some(Choice::Back) => state.go_back(),
        Some(Choice::Show(shown)) => state.go(shown),
        None => {}
    }
}

fn search_id() -> Id {
    Id::new("guide-search")
}

fn search_field(ui: &mut Ui, state: &mut Guide) {
    ui.horizontal(|ui| {
        let muted = appearance::tokens(ui).text_muted;
        widgets::icon_label(ui, icons::SEARCH, muted);
        let field = widgets::text_field(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut state.query)
                    .id(search_id())
                    .hint_text(SEARCH_HINT)
                    .desired_width(f32::INFINITY),
            )
        });
        if std::mem::take(&mut state.focus_search) {
            field.request_focus();
        }
    });
}

fn link(ui: &mut Ui, text: &str, style: Option<TextStyle>) -> Response {
    let tokens = appearance::tokens(ui);
    let mut rich = RichText::new(text).color(tokens.accent_text).underline();
    if let Some(style) = style {
        rich = rich.text_style(style);
    }
    let response = ui.add(Label::new(rich).selectable(false).sense(Sense::click()));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, true, text));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let stroke = egui::Stroke::new(UNDERLINE_WIDTH * 2.0, tokens.accent_text);
        ui.painter()
            .hline(response.rect.x_range(), response.rect.bottom(), stroke);
    }
    if response.has_focus() {
        widgets::paint_focus_ring(ui, response.rect);
    }
    response
}

fn page_link(ui: &mut Ui, page: Page) -> bool {
    link(ui, page.title(), None).clicked()
}

fn contents(ui: &mut Ui) -> Option<Page> {
    let mut chosen = None;
    for chapter in Chapter::ALL {
        ui.add(Label::new(widgets::section_title(chapter.title())).selectable(false));
        ui.add_space(SPACE_S);
        for page in chapter.pages() {
            ui.horizontal(|ui| {
                ui.add_space(SPACE_M);
                if page_link(ui, page) {
                    chosen = Some(page);
                }
            });
        }
        ui.add_space(SPACE_L);
    }
    chosen
}

fn hits(ui: &mut Ui, query: &str, hits: &[Hit]) -> Option<Page> {
    if hits.is_empty() {
        widgets::callout(ui, Tone::Info, |ui| {
            ui.add(
                Label::new(format!(
                    "Nothing in the guide mentions “{query}”. Try fewer or other words, or browse \
                     the contents."
                ))
                .wrap(),
            );
        });
        return None;
    }
    let mut chosen = None;
    for hit in hits {
        if page_link(ui, hit.page) {
            chosen = Some(hit.page);
        }
        ui.add(Label::new(widgets::muted(&hit.snippet, ui)).wrap());
        ui.add_space(SPACE_M);
    }
    chosen
}

fn page_body(ui: &mut Ui, page: Page, keymap: &Keymap) -> Option<Page> {
    let mut chosen = None;
    let bullet = appearance::tokens(ui).text;
    ui.add(Label::new(widgets::muted(page.chapter().title(), ui)).selectable(false));
    ui.add(
        Label::new(
            RichText::new(page.title())
                .family(fonts::semibold())
                .size(PAGE_TITLE_SIZE),
        )
        .wrap(),
    );
    ui.add_space(SPACE_M);
    for block in page.blocks() {
        let clicked = match block {
            Block::Heading(text) => {
                ui.add_space(SPACE_S);
                ui.add(Label::new(widgets::section_title(text)).wrap());
                None
            }
            Block::Paragraph(spans) => paragraph(ui, spans, keymap),
            Block::Item(spans) => {
                ui.horizontal_top(|ui| {
                    widgets::icon_label(ui, icons::BULLET, bullet);
                    ui.vertical(|ui| paragraph(ui, spans, keymap)).inner
                })
                .inner
            }
        };
        chosen = chosen.or(clicked);
        ui.add_space(SPACE_S);
    }
    if let Some(next) = next_page(page) {
        ui.add_space(SPACE_M);
        ui.separator();
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(Label::new(widgets::muted(NEXT, ui)).selectable(false));
            ui.add_space(SPACE_S);
            if page_link(ui, next) {
                chosen = Some(next);
            }
        });
    }
    chosen
}

fn next_page(page: Page) -> Option<Page> {
    Page::ALL
        .into_iter()
        .skip_while(|listed| *listed != page)
        .nth(1)
}

fn paragraph(ui: &mut Ui, spans: &[Span], keymap: &Keymap) -> Option<Page> {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut chosen = None;
        for span in spans {
            match span {
                Span::Text(text) => {
                    ui.add(Label::new(text.as_str()));
                }
                Span::Strong(text) => {
                    ui.add(Label::new(widgets::strong(text.as_str())));
                }
                Span::Code(text) => {
                    ui.add(Label::new(RichText::new(text.as_str()).monospace()));
                }
                Span::Link { text, target } => {
                    let clicked = link(ui, text, None).clicked();
                    if clicked && let Target::Page(page) = target {
                        chosen = Some(*page);
                    }
                }
                Span::Command(named) => {
                    ui.add(Label::new(widgets::strong(command_text(named, keymap))));
                }
            }
        }
        chosen
    })
    .inner
}

pub fn command_text(named: &Named, keymap: &Keymap) -> String {
    match named {
        Named::Command(command) => match keymap.first(*command) {
            Some(shortcut) => format!("{} ({})", command.title(), commands::display(&shortcut)),
            None => command.title(),
        },
        Named::Unknown(id) => id.clone(),
    }
}

pub fn help_button(ui: &mut Ui, page: Page) {
    let hover = format!("Open “{}” in the user guide", page.title());
    if widgets::icon_button(ui, icons::HELP, &hover).clicked() {
        guide::ask(ui.ctx(), page);
    }
}

pub fn help_link(ui: &mut Ui, page: Page) {
    let text = format!("Help on {}", decapitalized(page.title()));
    let button = widgets::small_button(ui, icons::HELP, &text);
    if ui
        .add(button)
        .on_hover_text("Open its page in the user guide")
        .clicked()
    {
        guide::ask(ui.ctx(), page);
    }
}

fn decapitalized(title: &str) -> String {
    let mut characters = title.chars();
    match characters.next() {
        Some(first) if !title.starts_with("STEP") => {
            first.to_lowercase().chain(characters).collect()
        }
        Some(_) | None => title.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_opens_the_context_page_and_closes_it_when_shown() {
        let mut guide = Guide::default();
        guide.toggle_at(Some(Page::Extrude));
        assert!(guide.open);
        assert_eq!(guide.page(), Some(Page::Extrude));
        guide.toggle_at(Some(Page::Line));
        assert_eq!(guide.page(), Some(Page::Line));
        guide.go_back();
        assert_eq!(guide.page(), Some(Page::Extrude));
        guide.toggle_at(Some(Page::Extrude));
        assert!(!guide.open);
        guide.toggle_at(None);
        assert!(guide.open);
        assert_eq!(guide.page(), Some(Page::Extrude));
        guide.toggle_at(None);
        assert!(!guide.open);
    }

    #[test]
    fn every_page_but_the_last_leads_to_the_next() {
        assert_eq!(next_page(Page::Start), Page::ALL.get(1).copied());
        assert_eq!(next_page(Page::Accessibility), None);
    }
}
