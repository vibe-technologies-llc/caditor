use caditor_geometry::Vector2;
use egui::{
    Align, Context, Id, LayerId, Layout, Order, Popup, PopupAnchor, PopupCloseBehavior, PopupKind,
    Pos2, Ui, containers::menu::menu_style,
};

use crate::{
    commands::{Command, StandardView},
    editing::Tool,
    icons,
    menu_bar::{self, MenuEntries},
    selection::SelectionFilter,
    sketch_tools::ConstraintTool,
    widgets,
};

const MENU_ID: &str = "view-menu";
const MOST_INLINE_CONSTRAINTS: usize = 3;
pub const CONSTRAIN: &str = "Constrain";
pub const SELECT: &str = "Select";
pub const TRANSFORM: &str = "Move, rotate or scale";
pub const STANDARD_VIEWS: &str = "Standard views";
pub const SELECTION_FILTER: &str = "Selection filter";
pub const HIDE: &str = "Hide";
pub const HIDE_OTHERS: &str = "Hide everything else";
pub const SHOW_ALL: &str = "Show everything";
pub const LOOK_AT_FACE: &str = "Look straight at the face";
pub const FIT_SELECTION: &str = "Fit selection";
pub const FIT_ALL: &str = "Fit all";
pub const DELETE: &str = "Delete";
pub const CONSTRUCTION: &str = "Construction geometry";
pub const CANCEL_FEATURE: &str = "Cancel the changes";
pub const CLOSE_FEATURE: &str = "Finish editing";
const GROWING: [(Command, &str); 4] = [
    (Command::SelectBody, "The whole body"),
    (Command::SelectTangentEdges, "Tangent edges"),
    (Command::SelectTangentFaces, "Tangent faces"),
    (Command::SelectHole, "The whole hole"),
];
const SKETCH_SELECTING: [(Command, &str); 3] = [
    (Command::SelectAll, "All sketch geometry"),
    (Command::SelectFree, "What is still free"),
    (Command::SelectOpenEnds, "The open ends"),
];
const TRANSFORMING: [(Command, &str); 3] = [
    (Command::MoveGeometry, "Move"),
    (Command::RotateGeometry, "Rotate"),
    (Command::ScaleGeometry, "Scale"),
];
const CLIPBOARD: [(Command, &str); 3] = [
    (Command::CutGeometry, "Cut"),
    (Command::CopyGeometry, "Copy"),
    (Command::PasteGeometry, "Paste"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Model {
        on_item: bool,
        feature_open: bool,
        selected: bool,
    },
    Sketch {
        on_item: bool,
    },
    Shape,
}

impl Place {
    pub fn is_like(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Open,
    Closed,
    Chosen(Vec<Command>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Showing {
    Waiting,
    First,
    Shown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewMenu {
    anchor: Pos2,
    cursor: Vector2,
    place: Place,
    revision: u64,
    serial: u64,
    showing: Showing,
}

impl ViewMenu {
    pub fn new(anchor: Pos2, cursor: Vector2, place: Place, revision: u64, serial: u64) -> Self {
        Self {
            anchor,
            cursor,
            place,
            revision,
            serial,
            showing: Showing::Waiting,
        }
    }

    #[cfg(test)]
    pub fn anchor(&self) -> Pos2 {
        self.anchor
    }

    pub fn place(&self) -> Place {
        self.place
    }

    pub fn cursor(&self) -> Vector2 {
        self.cursor
    }

    pub fn opened(&self) -> u64 {
        self.revision
    }

    pub fn show(&mut self, ctx: &Context, entries: MenuEntries<'_>) -> Outcome {
        let mut entries = match self.showing {
            Showing::Waiting => {
                self.showing = Showing::First;
                ctx.request_repaint();
                return Outcome::Open;
            }
            Showing::First => entries.focusing_first(),
            Showing::Shown => entries,
        };
        self.showing = Showing::Shown;
        let id = Id::new(MENU_ID).with(self.serial);
        let place = self.place;
        let shown = Popup::new(
            id,
            ctx.clone(),
            PopupAnchor::Position(self.anchor),
            LayerId::new(Order::Foreground, id),
        )
        .kind(PopupKind::Menu)
        .layout(Layout::top_down_justified(Align::Min))
        .style(menu_style)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| widgets::fitted_menu(ui, |ui| contents(ui, place, &mut entries)));
        let chosen = entries.take_chosen();
        if !chosen.is_empty() {
            return Outcome::Chosen(chosen);
        }
        match shown {
            Some(shown) if !shown.response.should_close() => Outcome::Open,
            Some(_) | None => Outcome::Closed,
        }
    }
}

fn contents(ui: &mut Ui, place: Place, entries: &mut MenuEntries<'_>) {
    match place {
        Place::Model {
            on_item,
            feature_open,
            selected,
        } => {
            if feature_open {
                open_feature(ui, entries);
            }
            if on_item {
                model_item(ui, entries);
            } else {
                model_space(ui, entries, selected);
            }
        }
        Place::Sketch { on_item } => sketch(ui, entries, on_item),
        Place::Shape => shape(ui, entries),
    }
}

fn open_feature(ui: &mut Ui, entries: &mut MenuEntries<'_>) {
    entries.offered(
        ui,
        Command::ReverseDirection,
        &Command::ReverseDirection.title(),
    );
    entries.titled_item(ui, Command::CancelFeature, CANCEL_FEATURE);
    entries.titled_item(ui, Command::CloseFeature, CLOSE_FEATURE);
    ui.separator();
}

fn model_item(ui: &mut Ui, entries: &mut MenuEntries<'_>) {
    let edit = match entries.detail(Command::EditFeature) {
        Some(name) => format!("Edit {name}"),
        None => Command::EditFeature.title(),
    };
    entries.titled_item(ui, Command::EditFeature, &edit);
    ui.separator();
    entries.titled_item(ui, Command::HideSelection, HIDE);
    entries.titled_item(ui, Command::HideOthers, HIDE_OTHERS);
    entries.titled_item(ui, Command::ShowAll, SHOW_ALL);
    ui.separator();
    entries.offered(ui, Command::LookAtFace, LOOK_AT_FACE);
    entries.titled_item(ui, Command::FitView, FIT_SELECTION);
    entries.item(ui, Command::Measure);
    ui.separator();
    let growing: Vec<(Command, &str)> = GROWING
        .into_iter()
        .filter(|(command, _)| entries.is_available(*command))
        .collect();
    if !growing.is_empty() {
        titled_submenu(ui, entries, Command::SelectBody, SELECT, &growing);
    }
    entries.item(ui, Command::ListUnderPointer);
    let more = [Command::BodyAppearance, Command::CopyFeatures];
    if more.iter().any(|command| entries.is_available(*command)) {
        ui.separator();
        for command in more {
            entries.offered(ui, command, &command.title());
        }
    }
}

fn model_space(ui: &mut Ui, entries: &mut MenuEntries<'_>, selected: bool) {
    let fit = if selected { FIT_SELECTION } else { FIT_ALL };
    entries.titled_item(ui, Command::FitView, fit);
    entries.item(ui, Command::PreviousView);
    menu_bar::submenu(
        ui,
        icons::command(Command::View(StandardView::Front)),
        STANDARD_VIEWS,
        |ui| entries.items(ui, StandardView::ALL.map(Command::View)),
    );
    entries.titled_item(ui, Command::ShowAll, SHOW_ALL);
    ui.separator();
    entries.item(ui, Command::PasteFeatures);
    menu_bar::submenu(
        ui,
        icons::command(Command::Filter(SelectionFilter::Everything)),
        SELECTION_FILTER,
        |ui| {
            for filter in SelectionFilter::ALL {
                entries.choice(ui, Command::Filter(filter));
            }
            ui.separator();
            entries.item(ui, Command::CycleSelectionPriority);
        },
    );
}

fn sketch(ui: &mut Ui, entries: &mut MenuEntries<'_>, on_item: bool) {
    if on_item {
        constraints(ui, entries);
        entries.titled_item(ui, Command::Construction, CONSTRUCTION);
        entries.offered(ui, Command::SplitCurve, "Split the curve here");
        entries.offered(
            ui,
            Command::BreakCurves,
            "Break the curves where they cross",
        );
        entries.titled_item(ui, Command::DeleteSelection, DELETE);
        ui.separator();
        titled_submenu(ui, entries, Command::MoveGeometry, TRANSFORM, &TRANSFORMING);
        for (command, title) in CLIPBOARD {
            entries.titled_item(ui, command, title);
        }
    } else {
        entries.titled_item(ui, Command::PasteGeometry, "Paste");
        entries.item(ui, Command::FitView);
        entries.item(ui, Command::LookAtSketch);
    }
    ui.separator();
    titled_submenu(ui, entries, Command::SelectAll, SELECT, &SKETCH_SELECTING);
    entries.item(ui, Command::SketchTool(Tool::Dimension));
    ui.separator();
    entries.item(ui, Command::FinishSketch);
}

fn constraints(ui: &mut Ui, entries: &mut MenuEntries<'_>) {
    let offered: Vec<Command> = ConstraintTool::ALL
        .into_iter()
        .map(Command::Constraint)
        .filter(|command| entries.is_available(*command))
        .collect();
    if offered.is_empty() {
        return;
    }
    if offered.len() <= MOST_INLINE_CONSTRAINTS {
        entries.items(ui, offered);
    } else {
        menu_bar::submenu(
            ui,
            icons::constraint(ConstraintTool::Coincident),
            CONSTRAIN,
            |ui| entries.items(ui, offered),
        );
    }
    ui.separator();
}

fn shape(ui: &mut Ui, entries: &mut MenuEntries<'_>) {
    entries.item(ui, Command::TakeBackPoint);
    entries.offered(ui, Command::ReverseArc, &Command::ReverseArc.title());
    entries.offered(ui, Command::FinishShape, &Command::FinishShape.title());
    entries.item(ui, Command::CancelShape);
    ui.separator();
    entries.item(ui, Command::TypeValue);
}

fn titled_submenu(
    ui: &mut Ui,
    entries: &mut MenuEntries<'_>,
    shown: Command,
    title: &str,
    items: &[(Command, &str)],
) {
    menu_bar::submenu(ui, icons::command(shown), title, |ui| {
        for (command, item) in items {
            entries.titled_item(ui, *command, item);
        }
    });
}
