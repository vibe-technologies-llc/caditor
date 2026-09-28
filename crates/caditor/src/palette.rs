use egui::{Align2, Button, Id, Key, Modal, Modifiers, RichText, ScrollArea, TextEdit, vec2};

use crate::commands::{self, Command, Keymap, Offer};

const WIDTH: f32 = 520.0;
const TOP_MARGIN: f32 = 72.0;
const LIST_HEIGHT: f32 = 360.0;
const RECENT_LIMIT: usize = 6;
const FIELD_HINT: &str = "Type to find a command";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    Start,
    WordStart,
    Inside,
    Scattered,
}

#[derive(Debug, Clone, Default)]
pub struct Palette {
    open: bool,
    query: String,
    highlighted: usize,
    chosen: Option<Command>,
    recent: Vec<Command>,
}

pub struct Entry<'a> {
    pub offer: &'a Offer,
    pub text: String,
}

impl Palette {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.highlighted = 0;
    }

    pub fn take_chosen(&mut self) -> Option<Command> {
        self.chosen.take()
    }

    fn choose(&mut self, command: Command) {
        self.chosen = Some(command);
        self.open = false;
        self.recent.retain(|recent| *recent != command);
        self.recent.insert(0, command);
        self.recent.truncate(RECENT_LIMIT);
    }

    pub fn entries<'a>(&self, offers: &'a [Offer]) -> Vec<Entry<'a>> {
        let query = self.query.trim().to_lowercase();
        let mut ranked: Vec<(_, Entry<'a>)> = offers
            .iter()
            .enumerate()
            .filter(|(_, offer)| offer.command != Command::Palette)
            .filter_map(|(order, offer)| {
                let text = entry_text(offer.command);
                let (fit, length) = if query.is_empty() {
                    (Fit::Start, 0)
                } else {
                    let title = offer.command.title().to_lowercase();
                    (fit(&query, &title, &text.to_lowercase())?, title.len())
                };
                let recent = self
                    .recent
                    .iter()
                    .position(|recent| *recent == offer.command)
                    .unwrap_or(RECENT_LIMIT);
                let key = (fit, offer.availability.is_err(), recent, length, order);
                Some((key, Entry { offer, text }))
            })
            .collect();
        ranked.sort_by_key(|(key, _)| *key);
        ranked.into_iter().map(|(_, entry)| entry).collect()
    }

    pub fn show(&mut self, ctx: &egui::Context, offers: &[Offer], keymap: &Keymap) {
        if !self.open {
            return;
        }
        let (up, down, enter) = ctx.input_mut(|input| {
            (
                input.consume_key(Modifiers::NONE, Key::ArrowUp),
                input.consume_key(Modifiers::NONE, Key::ArrowDown),
                input.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        let id = Id::new("command-palette");
        let area = Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, TOP_MARGIN));
        let response = Modal::new(id).area(area).show(ctx, |ui| {
            ui.set_width(WIDTH);
            let field = ui.add(
                TextEdit::singleline(&mut self.query)
                    .hint_text(FIELD_HINT)
                    .desired_width(f32::INFINITY),
            );
            if field.changed() {
                self.highlighted = 0;
            }
            field.request_focus();
            let entries = self.entries(offers);
            let last = entries.len().saturating_sub(1);
            if down {
                self.highlighted = (self.highlighted + 1).min(last);
            }
            if up {
                self.highlighted = self.highlighted.saturating_sub(1);
            }
            self.highlighted = self.highlighted.min(last);
            let mut clicked = None;
            ui.add_space(4.0);
            if entries.is_empty() {
                ui.weak(
                    "No command matches. Commands that do not fit what you are doing are left out.",
                );
            }
            ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .show(ui, |ui| {
                    for (index, entry) in entries.iter().enumerate() {
                        let highlighted = index == self.highlighted;
                        let mut button = Button::selectable(highlighted, entry.text.as_str())
                            .min_size(vec2(ui.available_width(), 0.0));
                        if let Some(shortcut) = keymap.first(entry.offer.command) {
                            button = button.shortcut_text(commands::display(&shortcut));
                        }
                        let ready = entry.offer.availability.is_ok();
                        let row = ui.add_enabled(ready, button);
                        if highlighted && (up || down) {
                            row.scroll_to_me(None);
                        }
                        let row = match &entry.offer.availability {
                            Ok(()) => row,
                            Err(reason) => row.on_disabled_hover_text(reason),
                        };
                        if row.clicked() {
                            clicked = Some(entry.offer.command);
                        }
                    }
                });
            let highlighted = entries.get(self.highlighted);
            if let Some(Entry {
                offer:
                    Offer {
                        availability: Err(reason),
                        ..
                    },
                text,
            }) = highlighted
            {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{text} is not available: {reason}.")).weak());
            }
            let entered = highlighted
                .filter(|entry| enter && entry.offer.availability.is_ok())
                .map(|entry| entry.offer.command);
            clicked.or(entered)
        });
        if let Some(command) = response.inner {
            self.choose(command);
            ctx.request_repaint();
        } else if response.should_close() {
            self.open = false;
        }
    }
}

pub fn entry_text(command: Command) -> String {
    format!("{}: {}", command.category().label(), command.title())
}

fn fit(query: &str, title: &str, text: &str) -> Option<Fit> {
    if title.starts_with(query) || text.starts_with(query) {
        return Some(Fit::Start);
    }
    let words_fit = query.split_whitespace().all(|term| {
        text.split(|character: char| !character.is_alphanumeric())
            .any(|word| word.starts_with(term))
    });
    if words_fit {
        return Some(Fit::WordStart);
    }
    if text.contains(query) {
        return Some(Fit::Inside);
    }
    let mut remaining = text.chars();
    query
        .chars()
        .filter(|character| !character.is_whitespace())
        .all(|wanted| remaining.any(|character| character == wanted))
        .then_some(Fit::Scattered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::Tool;

    fn offer(command: Command, ready: bool) -> Offer {
        Offer {
            command,
            availability: if ready {
                Ok(())
            } else {
                Err("Select edges".to_owned())
            },
        }
    }

    fn found(palette: &Palette, offers: &[Offer]) -> Vec<Command> {
        palette
            .entries(offers)
            .iter()
            .map(|entry| entry.offer.command)
            .collect()
    }

    #[test]
    fn entries_rank_word_starts_first_and_unavailable_commands_last() {
        let offers = [
            offer(Command::Fillet, false),
            offer(Command::SaveAs, true),
            offer(Command::SketchTool(Tool::Line), true),
            offer(Command::Save, true),
            offer(Command::Palette, true),
        ];
        let mut palette = Palette::default();
        assert_eq!(
            found(&palette, &offers),
            vec![
                Command::SaveAs,
                Command::SketchTool(Tool::Line),
                Command::Save,
                Command::Fillet
            ]
        );
        palette.query = "save".to_owned();
        assert_eq!(
            found(&palette, &offers),
            vec![Command::Save, Command::SaveAs]
        );
        palette.query = "draw li".to_owned();
        assert_eq!(
            found(&palette, &offers),
            vec![Command::SketchTool(Tool::Line)]
        );
        palette.query = "flt".to_owned();
        assert_eq!(found(&palette, &offers), vec![Command::Fillet]);
        palette.query = "zzz".to_owned();
        assert!(found(&palette, &offers).is_empty());

        palette.query.clear();
        palette.choose(Command::Save);
        assert_eq!(found(&palette, &offers).first(), Some(&Command::Save));
        assert_eq!(palette.take_chosen(), Some(Command::Save));
        assert_eq!(palette.take_chosen(), None);
    }
}
