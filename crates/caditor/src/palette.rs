use std::borrow::Cow;

use caditor_document::{ConfigurationId, Document};
use egui::{
    Align, Align2, CornerRadius, Id, Key, Label, Layout, Margin, Modal, Modifiers, Rect, Response,
    RichText, ScrollArea, Sense, TextEdit, TextStyle, TextWrapMode, Ui, UiBuilder, vec2,
};

use crate::{
    appearance::{self, CONTROL_HEIGHT, DIALOG_MARGIN, SPACE_M, SPACE_S, WIDGET_RADIUS},
    commands::{Command, Keymap, Offer, Scope},
    feature_tree, icons,
    panels::Focus,
    toggles::{Shown, ToggleStates},
    widgets::{self, Tone},
};

const TOP_MARGIN: f32 = 72.0;
const LIST_HEIGHT: f32 = 360.0;
pub const RECENT_LIMIT: usize = 6;
const ROW_HEIGHT: f32 = CONTROL_HEIGHT + SPACE_S;
const DETAIL_LINES: f32 = 2.0;
pub const FIELD_HINT: &str =
    "Search commands, features, parameters, views, selection sets and configurations";
const SKETCH_ONLY: &str = "works only while a sketch is edited";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    Start,
    WordStart,
    Inside,
    Keyword,
    Scattered,
}

type Rank = (Fit, u8, usize, usize, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Recent,
    Commands,
    Features,
    Parameters,
    Views,
    SelectionSets,
    Configurations,
}

impl Group {
    fn heading(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Commands => "Commands",
            Self::Features => "Features",
            Self::Parameters => "Parameters",
            Self::Views => "Views",
            Self::SelectionSets => "Selection sets",
            Self::Configurations => "Configurations",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Command(Command),
    Focus(Focus),
    View(usize),
    SelectionSet(usize),
    Configuration(ConfigurationId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Ready,
    Unavailable(Cow<'static, str>),
    OutOfContext(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    group: Group,
    pub choice: Choice,
    pub title: String,
    glyph: &'static str,
    note: Option<String>,
    keys: Option<String>,
    pub shown: Option<Shown>,
    pub state: State,
}

impl Entry {
    fn name(&self) -> String {
        match self.shown.and_then(Shown::pill) {
            Some(state) => format!("{}, {}", self.title, state.to_lowercase()),
            None => self.title.clone(),
        }
    }

    fn detail(&self) -> String {
        match (&self.state, self.choice) {
            (State::Unavailable(reason), _) => {
                format!("{} is not available: {reason}.", self.title)
            }
            (State::OutOfContext(reason), _) => {
                format!("{} is not available here: it {reason}.", self.title)
            }
            (State::Ready, Choice::Command(_)) => "Press Enter to run it.".to_owned(),
            (State::Ready, Choice::Focus(Focus::ParameterValue(_))) => {
                "Press Enter to edit its value in Parameters.".to_owned()
            }
            (State::Ready, Choice::Focus(_)) => {
                "Press Enter to select it in the feature tree.".to_owned()
            }
            (State::Ready, Choice::View(_)) => "Press Enter to go to this saved view.".to_owned(),
            (State::Ready, Choice::SelectionSet(_)) => {
                "Press Enter to select what this set holds.".to_owned()
            }
            (State::Ready, Choice::Configuration(_)) => {
                "Press Enter to switch the model to this configuration.".to_owned()
            }
        }
    }

    fn rank(&self) -> u8 {
        match self.state {
            State::Ready => 0,
            State::Unavailable(_) => 1,
            State::OutOfContext(_) => 2,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Palette {
    open: bool,
    query: String,
    highlighted: usize,
    chosen: Option<Command>,
    focus: Option<Focus>,
    view: Option<usize>,
    selection_set: Option<usize>,
    configuration: Option<ConfigurationId>,
    recent: Vec<Command>,
}

impl Palette {
    pub fn with_recent(recent: Vec<Command>) -> Self {
        Self {
            recent,
            ..Self::default()
        }
    }

    pub fn recent(&self) -> &[Command] {
        &self.recent
    }

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

    pub fn take_focus(&mut self) -> Option<Focus> {
        self.focus.take()
    }

    pub fn take_view(&mut self) -> Option<usize> {
        self.view.take()
    }

    pub fn take_selection_set(&mut self) -> Option<usize> {
        self.selection_set.take()
    }

    pub fn take_configuration(&mut self) -> Option<ConfigurationId> {
        self.configuration.take()
    }

    fn choose(&mut self, choice: Choice) {
        self.open = false;
        match choice {
            Choice::Command(command) => {
                self.chosen = Some(command);
                self.recent.retain(|recent| *recent != command);
                self.recent.insert(0, command);
                self.recent.truncate(RECENT_LIMIT);
            }
            Choice::Focus(focus) => self.focus = Some(focus),
            Choice::View(index) => self.view = Some(index),
            Choice::SelectionSet(index) => self.selection_set = Some(index),
            Choice::Configuration(id) => self.configuration = Some(id),
        }
    }

    fn recent_position(&self, command: Command) -> usize {
        self.recent
            .iter()
            .position(|recent| *recent == command)
            .unwrap_or(RECENT_LIMIT)
    }

    pub fn entries(
        &self,
        offers: &[Offer],
        keymap: &Keymap,
        document: &Document,
        toggles: &ToggleStates,
    ) -> Vec<Entry> {
        let query = self.query.trim().to_lowercase();
        let searching = !query.is_empty();
        let matches = |title: &str, text: &str, keywords: &[&str]| -> Option<(Fit, usize)> {
            if !searching {
                return Some((Fit::Start, 0));
            }
            let title = title.to_lowercase();
            fit_with_keywords(&query, &title, &text.to_lowercase(), keywords)
                .map(|fit| (fit, title.len()))
        };
        let mut ranked: Vec<(Rank, Entry)> = Vec::new();
        for (order, offer) in offers.iter().enumerate() {
            if offer.command == Command::Palette {
                continue;
            }
            let title = offer.title();
            let Some((fit, length)) = matches(&title, &entry_text(offer), offer.command.keywords())
            else {
                continue;
            };
            let state = match &offer.availability {
                Ok(()) => State::Ready,
                Err(reason) => State::Unavailable(reason.clone()),
            };
            let recent = self.recent_position(offer.command);
            let group = if !searching && recent < RECENT_LIMIT && state == State::Ready {
                Group::Recent
            } else {
                Group::Commands
            };
            let entry = command_entry(offer.command, title, state, group, keymap, toggles);
            ranked.push(((fit, entry.rank(), recent, length, order), entry));
        }
        let absent = Command::all().filter(|command| {
            *command != Command::Palette && offers.iter().all(|offer| offer.command != *command)
        });
        for (order, command) in absent.enumerate() {
            let Some(reason) = absence(command) else {
                continue;
            };
            let title = command.title();
            let text = format!("{}: {title}", command.category().label());
            let Some((fit, length)) = matches(&title, &text, command.keywords()) else {
                continue;
            };
            let state = State::OutOfContext(reason);
            let entry = command_entry(command, title, state, Group::Commands, keymap, toggles);
            ranked.push(((fit, entry.rank(), RECENT_LIMIT, length, order), entry));
        }
        if searching {
            for (order, feature) in document.features().enumerate() {
                let kind = feature_tree::kind_words(&feature.kind);
                let Some((fit, length)) = matches(&feature.name, &feature.name, kind) else {
                    continue;
                };
                let entry = Entry {
                    group: Group::Features,
                    choice: Choice::Focus(Focus::Feature(feature.id())),
                    title: feature.name.clone(),
                    glyph: icons::feature(&feature.kind),
                    note: None,
                    keys: None,
                    shown: None,
                    state: State::Ready,
                };
                ranked.push(((fit, 0, RECENT_LIMIT, length, order), entry));
            }
            for (order, parameter) in document.parameters().iter().enumerate() {
                let Some((fit, length)) = matches(&parameter.name, &parameter.name, &[]) else {
                    continue;
                };
                let entry = Entry {
                    group: Group::Parameters,
                    choice: Choice::Focus(Focus::ParameterValue(parameter.id())),
                    title: parameter.name.clone(),
                    glyph: icons::UNIT,
                    note: Some(document.expression_text(&parameter.expression)),
                    keys: None,
                    shown: None,
                    state: State::Ready,
                };
                ranked.push(((fit, 0, RECENT_LIMIT, length, order), entry));
            }
        }
        if searching {
            for (index, named) in document.saved_views().named.iter().enumerate() {
                let Some((fit, length)) = matches(&named.name, &named.name, &[]) else {
                    continue;
                };
                let entry = Entry {
                    group: Group::Views,
                    choice: Choice::View(index),
                    title: named.name.clone(),
                    glyph: icons::command(Command::SavedViews),
                    note: None,
                    keys: None,
                    shown: None,
                    state: State::Ready,
                };
                ranked.push(((fit, 0, RECENT_LIMIT, length, index), entry));
            }
            for (index, set) in document.selection_sets().sets.iter().enumerate() {
                let Some((fit, length)) = matches(&set.name, &set.name, &[]) else {
                    continue;
                };
                let entry = Entry {
                    group: Group::SelectionSets,
                    choice: Choice::SelectionSet(index),
                    title: set.name.clone(),
                    glyph: icons::command(Command::SelectionSets),
                    note: None,
                    keys: None,
                    shown: None,
                    state: State::Ready,
                };
                ranked.push(((fit, 0, RECENT_LIMIT, length, index), entry));
            }
            let configurations = document.configurations();
            for (index, row) in configurations.rows.iter().enumerate() {
                let title = format!("Switch to {}", row.name);
                let Some((fit, length)) = matches(&title, &row.name, &[]) else {
                    continue;
                };
                let active = configurations.is_active(row.id);
                let entry = Entry {
                    group: Group::Configurations,
                    choice: Choice::Configuration(row.id),
                    title,
                    glyph: icons::command(Command::Configurations),
                    note: active.then(|| "Active".to_owned()),
                    keys: None,
                    shown: None,
                    state: if active {
                        State::Unavailable(Cow::Borrowed("it is already the active configuration"))
                    } else {
                        State::Ready
                    },
                };
                ranked.push(((fit, 0, RECENT_LIMIT, length, index), entry));
            }
        }
        let best = |group: Group| {
            ranked
                .iter()
                .filter(|(_, entry)| entry.group == group)
                .map(|(key, _)| (key.0, key.1))
                .min()
        };
        let group_order = |group: Group| {
            if searching {
                (best(group), group)
            } else {
                (None, group)
            }
        };
        let mut groups: Vec<_> = [
            Group::Recent,
            Group::Commands,
            Group::Features,
            Group::Parameters,
            Group::Views,
            Group::SelectionSets,
            Group::Configurations,
        ]
        .into_iter()
        .map(|group| (group_order(group), group))
        .collect();
        groups.sort();
        ranked.sort_by(|(left, left_entry), (right, right_entry)| {
            let place = |group: Group| groups.iter().position(|(_, listed)| *listed == group);
            let recent_first = |key: &Rank, group: Group| {
                if group == Group::Recent {
                    (Fit::Start, 0, key.2, 0, 0)
                } else {
                    *key
                }
            };
            let elsewhere = |entry: &Entry| matches!(entry.state, State::OutOfContext(_));
            place(left_entry.group)
                .cmp(&place(right_entry.group))
                .then_with(|| elsewhere(left_entry).cmp(&elsewhere(right_entry)))
                .then_with(|| {
                    recent_first(left, left_entry.group)
                        .cmp(&recent_first(right, right_entry.group))
                })
        });
        ranked.into_iter().map(|(_, entry)| entry).collect()
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        offers: &[Offer],
        keymap: &Keymap,
        document: &Document,
        toggles: &ToggleStates,
    ) {
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
        let frame = widgets::dialog_frame(ctx).inner_margin(Margin::same(DIALOG_MARGIN));
        let response = Modal::new(id).area(area).frame(frame).show(ctx, |ui| {
            ui.set_width(widgets::fitting_width(
                ui.ctx(),
                widgets::DialogWidth::Wide.points(),
            ));
            let field = ui
                .horizontal(|ui| {
                    let muted = appearance::tokens(ui).text_muted;
                    widgets::icon_label(ui, icons::SEARCH, muted);
                    widgets::text_field(ui, |ui| {
                        ui.add(
                            TextEdit::singleline(&mut self.query)
                                .hint_text(FIELD_HINT)
                                .desired_width(f32::INFINITY),
                        )
                    })
                })
                .inner;
            if field.changed() {
                self.highlighted = 0;
            }
            field.request_focus();
            let entries = self.entries(offers, keymap, document, toggles);
            let last = entries.len().saturating_sub(1);
            if down {
                self.highlighted = (self.highlighted + 1).min(last);
            }
            if up {
                self.highlighted = self.highlighted.saturating_sub(1);
            }
            self.highlighted = self.highlighted.min(last);
            ui.add_space(SPACE_S);
            ui.separator();
            let mut clicked = None;
            if entries.is_empty() {
                let text = format!(
                    "Nothing is called “{}”. Try another word, or fewer letters.",
                    self.query.trim()
                );
                widgets::empty_state(ui, icons::SEARCH, &text, |_| {});
            } else {
                ScrollArea::vertical()
                    .max_height(widgets::list_height(ui.ctx(), LIST_HEIGHT))
                    .show(ui, |ui| {
                        let mut group = None;
                        for (index, entry) in entries.iter().enumerate() {
                            if group != Some(entry.group) {
                                if group.is_some() {
                                    ui.add_space(SPACE_S);
                                }
                                group = Some(entry.group);
                                widgets::column_caption(ui, entry.group.heading());
                            }
                            let highlighted = index == self.highlighted;
                            let row = row(ui, entry, highlighted);
                            if highlighted && (up || down) {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() {
                                clicked = Some(index);
                            }
                        }
                    });
            }
            ui.separator();
            detail(ui, entries.get(self.highlighted));
            if let Some(index) = clicked {
                self.highlighted = index;
            }
            clicked
                .or(enter.then_some(self.highlighted))
                .and_then(|index| entries.get(index))
                .filter(|entry| entry.state == State::Ready)
                .map(|entry| entry.choice)
        });
        if let Some(choice) = response.inner {
            self.choose(choice);
            ctx.request_repaint();
        } else if response.should_close() {
            self.open = false;
        }
    }
}

fn command_entry(
    command: Command,
    title: String,
    state: State,
    group: Group,
    keymap: &Keymap,
    toggles: &ToggleStates,
) -> Entry {
    Entry {
        group,
        choice: Choice::Command(command),
        title,
        glyph: icons::command(command),
        note: Some(command.category().label().to_owned()),
        keys: keymap.hint(command).map(str::to_owned),
        shown: toggles.shown(command),
        state,
    }
}

fn absence(command: Command) -> Option<&'static str> {
    (command.scope() == Scope::Sketch).then_some(SKETCH_ONLY)
}

fn row(ui: &mut Ui, entry: &Entry, highlighted: bool) -> Response {
    let tokens = appearance::tokens(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let fill = if highlighted {
        Some(tokens.accent_subtle)
    } else if response.hovered() {
        Some(tokens.hover)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(WIDGET_RADIUS), fill);
    }
    let ready = entry.state == State::Ready;
    let (text, glyph) = match (highlighted, ready) {
        (true, true) => (tokens.accent_text, tokens.accent_text),
        (false, true) => (tokens.text, tokens.text_muted),
        (_, false) => (tokens.text_muted, tokens.text_muted),
    };
    let inner = Rect::from_min_max(rect.min + vec2(SPACE_M, 0.0), rect.max - vec2(SPACE_M, 0.0));
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
    widgets::icon_label(&mut child, entry.glyph, glyph);
    child.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if let Some(keys) = &entry.keys {
            widgets::key_cap(ui, keys);
        }
        if let Some(pill) = entry.shown.and_then(Shown::pill) {
            let tone = if entry.shown.is_some_and(Shown::is_on) {
                Tone::Info
            } else {
                Tone::Neutral
            };
            widgets::pill(ui, tone, pill);
        }
        if let Some(note) = &entry.note {
            ui.add(
                Label::new(
                    RichText::new(note)
                        .text_style(TextStyle::Small)
                        .color(tokens.text_muted),
                )
                .selectable(false),
            );
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(
                Label::new(RichText::new(&entry.title).color(text))
                    .selectable(false)
                    .truncate(),
            );
        });
    });
    widgets::named(response, &entry.name()).on_hover_text(entry.detail())
}

fn detail(ui: &mut Ui, entry: Option<&Entry>) {
    let tokens = appearance::tokens(ui);
    let height = DETAIL_LINES * ui.text_style_height(&TextStyle::Body);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let Some(entry) = entry else {
        return;
    };
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let color = if entry.state == State::Ready {
        tokens.text_muted
    } else {
        widgets::icon_label(
            &mut child,
            Tone::Warning.icon(),
            Tone::Warning.color(&tokens),
        );
        tokens.text
    };
    child.add(
        Label::new(RichText::new(entry.detail()).color(color))
            .selectable(false)
            .wrap(),
    );
}

pub fn entry_text(offer: &Offer) -> String {
    format!("{}: {}", offer.command.category().label(), offer.title())
}

pub fn starts_words(query: &str, text: &str) -> bool {
    fit(query, text, text).is_some_and(|fitted| fitted <= Fit::WordStart)
}

fn fit_with_keywords(query: &str, title: &str, text: &str, keywords: &[&str]) -> Option<Fit> {
    let fitted = fit(query, title, text);
    if fitted.is_some_and(|fitted| fitted < Fit::Keyword) {
        return fitted;
    }
    let named = keywords.iter().any(|keyword| {
        keyword.starts_with(query)
            || query
                .split_whitespace()
                .all(|term| keyword.split(' ').any(|word| word.starts_with(term)))
    });
    if named { Some(Fit::Keyword) } else { fitted }
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
    use crate::{editing::Tool, viewport::ViewportState};

    fn toggles() -> ToggleStates {
        ToggleStates::of(&ViewportState::new(), false)
    }

    fn offer(command: Command, ready: bool) -> Offer {
        Offer {
            command,
            availability: if ready {
                Ok(())
            } else {
                Err(Cow::Borrowed("Select edges"))
            },
            detail: None,
        }
    }

    fn found(palette: &Palette, offers: &[Offer]) -> Vec<Choice> {
        palette
            .entries(offers, &Keymap::default(), &Document::default(), &toggles())
            .iter()
            .filter(|entry| !matches!(entry.state, State::OutOfContext(_)))
            .map(|entry| entry.choice)
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
            [
                Command::SaveAs,
                Command::SketchTool(Tool::Line),
                Command::Save,
                Command::Fillet
            ]
            .map(Choice::Command)
        );
        palette.query = "save".to_owned();
        assert_eq!(
            found(&palette, &offers),
            [Command::Save, Command::SaveAs].map(Choice::Command)
        );
        palette.query = "draw li".to_owned();
        assert_eq!(
            found(&palette, &offers),
            [Choice::Command(Command::SketchTool(Tool::Line))]
        );
        palette.query = "flt".to_owned();
        assert_eq!(found(&palette, &offers), [Choice::Command(Command::Fillet)]);
        palette.query = "zzz".to_owned();
        assert!(found(&palette, &offers).is_empty());

        palette.query.clear();
        palette.choose(Choice::Command(Command::Save));
        assert_eq!(
            found(&palette, &offers).first(),
            Some(&Choice::Command(Command::Save))
        );
        assert_eq!(palette.take_chosen(), Some(Command::Save));
        assert_eq!(palette.take_chosen(), None);
    }

    #[test]
    fn other_names_find_commands_after_title_matches_and_kind_words_find_features() {
        let offers = [
            offer(Command::Messages, true),
            offer(Command::UndoHistory, true),
            offer(Command::FitView, true),
            offer(Command::Measure, true),
        ];
        let mut document = Document::default();
        let mut transaction = document.transaction("Add a sketch");
        transaction.add_feature(
            "Plate outline",
            caditor_document::FeatureKind::from(caditor_sketch::Sketch::new(
                caditor_geometry::Plane::XY,
            )),
        );
        document.apply(transaction.finish()).unwrap();
        let mut palette = Palette::default();
        let mut found = |query: &str| {
            palette.query = query.to_owned();
            palette
                .entries(&offers, &Keymap::default(), &document, &toggles())
                .iter()
                .filter(|entry| !matches!(entry.state, State::OutOfContext(_)))
                .map(|entry| entry.choice)
                .collect::<Vec<_>>()
        };

        assert_eq!(found("zoom ext"), [Choice::Command(Command::FitView)]);
        assert_eq!(found("ruler"), [Choice::Command(Command::Measure)]);
        assert_eq!(
            found("history"),
            [Command::UndoHistory, Command::Messages].map(Choice::Command)
        );
        assert!(
            found("sketch")
                .iter()
                .any(|choice| matches!(choice, Choice::Focus(Focus::Feature(_))))
        );
    }

    #[test]
    fn toggle_rows_say_whether_they_are_on() {
        let offers = [
            offer(Command::ToggleSnapping, true),
            offer(Command::Save, true),
        ];
        let palette = Palette::default();

        let entries = palette.entries(
            &offers,
            &Keymap::default(),
            &Document::default(),
            &toggles(),
        );
        let shown: Vec<Option<Shown>> = entries
            .iter()
            .filter(|entry| entry.state == State::Ready)
            .map(|entry| entry.shown)
            .collect();

        assert_eq!(shown, [Some(Shown::On), None]);
        assert_eq!(
            entries.first().unwrap().name(),
            "Turn snapping on or off, on"
        );
    }

    #[test]
    fn commands_that_do_not_fit_the_context_follow_the_rest_with_their_reason() {
        let offers = [offer(Command::Save, true), offer(Command::Fillet, false)];
        let palette = Palette {
            query: "draw line".to_owned(),
            ..Palette::default()
        };

        let entries = palette.entries(
            &offers,
            &Keymap::default(),
            &Document::default(),
            &toggles(),
        );
        let line = entries
            .iter()
            .find(|entry| entry.choice == Choice::Command(Command::SketchTool(Tool::Line)))
            .unwrap();

        assert_eq!(line.state, State::OutOfContext(SKETCH_ONLY));
        assert_eq!(
            line.detail(),
            "Draw line is not available here: it works only while a sketch is edited."
        );
        assert!(entries.iter().all(|entry| entry.choice
            != Choice::Command(Command::OpenRecent(crate::commands::RecentSlot::ALL[0]))));
    }
}
