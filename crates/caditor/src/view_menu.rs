use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_geometry::Vector2;
use egui::{
    Align, Context, Id, LayerId, Layout, Order, Popup, PopupAnchor, PopupCloseBehavior, PopupKind,
    Pos2, RectAlign, Ui, Vec2, containers::menu::menu_style, vec2,
};

use crate::{
    body_selection::{self, Kind},
    commands::{Command, StandardView},
    editing::Tool,
    icons,
    menu_bar::{self, MenuEntries},
    model::Model,
    selection::{Pickable, Selection, SelectionFilter},
    sketch_placement::{self, SketchTarget},
    sketch_tools::ConstraintTool,
    viewport, widgets,
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
pub const BODY_TOOLS: &str = "Body";
pub const MODIFY: &str = "Modify";
pub const FEATURE_COMMANDS: [Command; 3] = [
    Command::SuppressFeature,
    Command::RenameFeature,
    Command::DeleteFeature,
];
const EDGE_TOOLS: [Command; 2] = [Command::Fillet, Command::Chamfer];
const FACE_TOOLS: [Command; 7] = [
    Command::Extrude,
    Command::Hole,
    Command::OffsetFace,
    Command::Shell,
    Command::SplitFace,
    Command::Thread,
    Command::MirrorFaces,
];
const SKETCH_TOOLS: [Command; 2] = [Command::Extrude, Command::Revolve];
const BODY_COMMANDS: [Command; 8] = [
    Command::Move,
    Command::CopyBody,
    Command::Mirror,
    Command::LinearPattern,
    Command::CircularPattern,
    Command::Split,
    Command::Scale,
    Command::Combine,
];
const SKETCH_MODIFYING: [Tool; 7] = [
    Tool::Offset,
    Tool::Mirror,
    Tool::RectangularPattern,
    Tool::CircularPattern,
    Tool::Fillet,
    Tool::Chamfer,
    Tool::TangentCircle,
];
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MadeBy {
    feature: FeatureId,
    name: String,
    asks: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    faces: bool,
    edges: bool,
    bodies: bool,
    whole_bodies: bool,
    sketch: bool,
    plane: bool,
    made_by: Option<MadeBy>,
}

impl Held {
    pub fn of(model: &Model, selection: &Selection) -> Self {
        let items: Vec<Pickable> = selection.iter().collect();
        let bodies: BTreeSet<FeatureId> = items
            .iter()
            .filter_map(|pickable| match pickable {
                Pickable::Face { body, .. }
                | Pickable::Edge { body, .. }
                | Pickable::Vertex { body, .. } => Some(*body),
                _ => None,
            })
            .collect();
        let whole_bodies = !bodies.is_empty()
            && items
                .iter()
                .all(|pickable| matches!(pickable, Pickable::Face { .. }))
            && bodies.iter().all(|body| {
                body_selection::whole_bodies(model, &[*body], Kind::Faces)
                    .into_iter()
                    .all(|face| selection.contains(face))
            });
        let feature = match (whole_bodies, bodies.first()) {
            (true, Some(body)) if bodies.len() == 1 => Some(*body),
            (true, _) => None,
            (false, _) => sole_feature(model, &items),
        };
        let document = model.document();
        let made_by = feature.and_then(|id| {
            let feature = document.feature(id)?;
            Some(MadeBy {
                feature: id,
                name: feature.name.clone(),
                asks: !document.dependents_of(&[id]).is_empty(),
            })
        });
        let holds = |kind: fn(&Pickable) -> bool| items.iter().any(kind);
        Self {
            faces: !whole_bodies && holds(|pickable| matches!(pickable, Pickable::Face { .. })),
            edges: holds(|pickable| matches!(pickable, Pickable::Edge { .. })),
            bodies: !bodies.is_empty(),
            whole_bodies,
            sketch: holds(|pickable| {
                matches!(
                    pickable,
                    Pickable::SketchEntity { .. } | Pickable::SketchRegion { .. }
                )
            }),
            plane: matches!(
                sketch_placement::sketch_target(model, selection),
                Ok(SketchTarget::Face(_) | SketchTarget::Datum(_) | SketchTarget::Principal(_))
            ),
            made_by,
        }
    }

    pub fn feature(&self) -> Option<FeatureId> {
        self.made_by.as_ref().map(|made_by| made_by.feature)
    }

    fn tools(&self) -> Vec<Command> {
        let mut tools = Vec::new();
        if self.plane {
            tools.push(Command::NewSketch);
        }
        let kinds = [
            (self.edges, EDGE_TOOLS.as_slice()),
            (self.faces, FACE_TOOLS.as_slice()),
            (self.sketch, SKETCH_TOOLS.as_slice()),
        ];
        for command in kinds
            .into_iter()
            .filter(|(held, _)| *held)
            .flat_map(|(_, commands)| commands)
        {
            if !tools.contains(command) {
                tools.push(*command);
            }
        }
        tools
    }
}

fn sole_feature(model: &Model, items: &[Pickable]) -> Option<FeatureId> {
    let mut features = items
        .iter()
        .map(|pickable| viewport::feature_of(*pickable, model));
    let first = features.next()??;
    features
        .all(|feature| feature == Some(first))
        .then_some(first)
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
    held: Held,
    revision: u64,
    serial: u64,
    showing: Showing,
    size: Option<Vec2>,
}

impl ViewMenu {
    pub fn new(
        anchor: Pos2,
        cursor: Vector2,
        (place, held): (Place, Held),
        revision: u64,
        serial: u64,
    ) -> Self {
        Self {
            anchor,
            cursor,
            place,
            held,
            revision,
            serial,
            showing: Showing::Waiting,
            size: None,
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

    pub fn held(&self) -> &Held {
        &self.held
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
        let held = &self.held;
        let opening = widgets::menu_opening_down(ctx, self.anchor, self.size);
        let shown = Popup::new(
            id,
            ctx.clone(),
            PopupAnchor::Position(opening),
            LayerId::new(Order::Foreground, id),
        )
        .kind(PopupKind::Menu)
        .align(RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .layout(Layout::top_down_justified(Align::Min))
        .style(menu_style)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| widgets::measured_menu(ui, |ui| contents(ui, place, held, &mut entries)));
        if let Some(shown) = &shown {
            let (_, height) = shown.inner;
            self.size = Some(vec2(shown.response.rect.width(), height));
        }
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

fn contents(ui: &mut Ui, place: Place, held: &Held, entries: &mut MenuEntries<'_>) {
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
                model_item(ui, entries, held);
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

fn model_item(ui: &mut Ui, entries: &mut MenuEntries<'_>, held: &Held) {
    let edit = match entries.detail(Command::EditFeature) {
        Some(name) => format!("Edit {name}"),
        None => Command::EditFeature.title(),
    };
    entries.titled_item(ui, Command::EditFeature, &edit);
    ui.separator();
    if fitting_tools(ui, entries, held) {
        ui.separator();
    }
    if let Some(made_by) = &held.made_by {
        made_by_entries(ui, entries, made_by);
        ui.separator();
    }
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

fn fitting_tools(ui: &mut Ui, entries: &mut MenuEntries<'_>, held: &Held) -> bool {
    let tools: Vec<Command> = held
        .tools()
        .into_iter()
        .filter(|command| entries.is_available(*command))
        .collect();
    let body: Vec<Command> = BODY_COMMANDS
        .into_iter()
        .filter(|command| held.bodies && entries.is_available(*command))
        .collect();
    entries.items(ui, tools.iter().copied());
    match body.first() {
        Some(_) if tools.is_empty() || held.whole_bodies => {
            entries.items(ui, body.iter().copied());
        }
        Some(first) => menu_bar::submenu(ui, icons::command(*first), BODY_TOOLS, |ui| {
            entries.items(ui, body.iter().copied());
        }),
        None => {}
    }
    !tools.is_empty() || !body.is_empty()
}

fn made_by_entries(ui: &mut Ui, entries: &mut MenuEntries<'_>, made_by: &MadeBy) {
    let name = &made_by.name;
    let asking = if made_by.asks { "…" } else { "" };
    let titles = [
        format!("Suppress {name}"),
        format!("Rename {name}"),
        format!("Delete {name}{asking}"),
    ];
    for (command, title) in FEATURE_COMMANDS.into_iter().zip(titles) {
        entries.titled_item_with(ui, command, &title, Ok(()));
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
        sketch_modifying(ui, entries);
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

fn sketch_modifying(ui: &mut Ui, entries: &mut MenuEntries<'_>) {
    let offered: Vec<Command> = SKETCH_MODIFYING
        .into_iter()
        .map(Command::SketchTool)
        .filter(|command| entries.is_available(*command))
        .collect();
    if let Some(first) = offered.first() {
        menu_bar::submenu(ui, icons::command(*first), MODIFY, |ui| {
            entries.items(ui, offered.iter().copied());
        });
    }
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
