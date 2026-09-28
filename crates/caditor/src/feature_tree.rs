use caditor_document::{
    Document, Edit, Feature, FeatureError, FeatureKind, FeatureState, FeatureStatus, FixTarget,
    SketchFeature, SolidResult, Transaction,
};
use caditor_sketch::{ConstraintId, Redundancy, Sketch};
use egui::{
    Align, Button, Color32, CornerRadius, Frame, Id, Label, Margin, Rect, Response, RichText,
    Sense, Sides, Ui, collapsing_header::CollapsingState, containers::menu::MenuButton, vec2,
};

use crate::{
    appearance::{self, WIDGET_RADIUS},
    blend_panel,
    commands::{Command, CommandFrame},
    datum_panel, datum_tools,
    editing::{EditingCommand, SketchEditing},
    field::{self, DimensionTarget},
    icons,
    model::{Action, Model},
    panels::{Focus, PanelState, Renaming},
    selection::Selection,
    shell_panel, sketch_placement,
    sketch_status::{self, SketchSummary},
    sketch_tools, solid_panel,
    widgets::{self, NAME_FIELD_WIDTH, Tone},
};

const NO_FEATURE_CHOSEN: &str = "Select a feature in the tree, or open one, first";
const NOTHING_SELECTED: &str =
    "Select geometry in an edited sketch, or a feature in the tree, to delete it";
const MORE_HINT: &str = "Rename, move or delete (also on right-click)";
const DIMENSION_FIELD_WIDTH: f32 = 150.0;
const EDIT_SKETCH_LABEL: &str = "Edit sketch";
const FINISH_SKETCH_LABEL: &str = "Finish sketch";
const OPEN_SOLID_LABEL: &str = "Edit feature and choose its regions in the view";
const CLOSE_SOLID_LABEL: &str = "Done editing this feature";
const OPEN_BLEND_LABEL: &str = "Edit feature and choose its edges in the view";
const OPEN_SHELL_LABEL: &str = "Edit feature and choose its open faces in the view";
const OPEN_DATUM_LABEL: &str = "Edit this plane or axis";
const PLACE_ON_PLANE_LABEL: &str = "Place on selected plane";
const PLACE_ON_FACE_LABEL: &str = "Place on selected face";
const DETACH_LABEL: &str = "Detach";
const EMPTY_TREE: &str = "The model has no features yet. Start with New sketch in the toolbar.";
const ROW_MARGIN: Margin = Margin::symmetric(4, 2);
const EDITED_BAR_WIDTH: f32 = 3.0;
const BODY_INDENT: f32 = 8.0;
const DIMENSION_INDENT: f32 = 16.0;
const ROW_GAP: f32 = 2.0;

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    if editing.feature().is_none() {
        state.opened_for_editing = None;
    }
    let document = model.document();
    if document.features().len() == 0 {
        ui.label(widgets::muted(EMPTY_TREE, ui));
    }
    ui.spacing_mut().item_spacing.y = ROW_GAP;
    let count = document.features().len();
    for (index, feature) in document.features().enumerate() {
        let row = Row {
            feature,
            selection,
            position: Position { index, count },
            edited: editing.feature() == Some(feature.id())
                || editing.solid() == Some(feature.id()),
            selected: state.selected == Some(feature.id()),
        };
        ui.push_id(("feature", feature.id()), |ui| {
            feature_row(ui, model, state, actions, &row);
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct Position {
    index: usize,
    count: usize,
}

struct Row<'a> {
    feature: &'a Feature,
    selection: &'a Selection,
    position: Position,
    edited: bool,
    selected: bool,
}

fn feature_row(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let document = model.document();
    let feature = row.feature;
    let id = feature.id();
    let status = model.evaluation().feature(id);

    if let Some(renaming) = state.renaming.filter(|renaming| renaming.feature == id) {
        rename_row(ui, document, state, actions, feature, renaming);
        return;
    }

    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("feature-header", id)),
        false,
    );
    let editing_started = row.edited && state.opened_for_editing != Some(id);
    if editing_started {
        state.opened_for_editing = Some(id);
    }
    if state.focus_inside(id) || editing_started {
        collapsing.set_open(true);
    }

    let tokens = appearance::tokens(ui);
    let mut prepared = Frame::new()
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .inner_margin(ROW_MARGIN)
        .begin(ui);
    let (toggled, name) = {
        let ui = &mut prepared.content_ui;
        let open = collapsing.is_open();
        Sides::new()
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    let chevron = if open {
                        icons::EXPANDED
                    } else {
                        icons::COLLAPSED
                    };
                    let hint = if open { "Hide details" } else { "Show details" };
                    let toggle = ui
                        .add(
                            Button::new(widgets::icon(chevron).color(tokens.text_muted))
                                .frame(false),
                        )
                        .on_hover_text(hint);
                    let kind_color = if row.edited {
                        tokens.accent_text
                    } else {
                        state_color(ui, status).unwrap_or(tokens.text_muted)
                    };
                    widgets::icon_label(ui, icons::feature(&feature.kind), kind_color);
                    let name = ui.add(
                        Label::new(name_text(ui, feature, status))
                            .selectable(false)
                            .sense(Sense::click())
                            .truncate(),
                    );
                    (toggle.clicked(), name)
                },
                |ui| {
                    more_menu(ui, document, state, actions, row);
                    edit_button(ui, row, actions);
                    status_icon(ui, status);
                },
            )
            .0
    };
    let rect = prepared.content_ui.min_rect() + ROW_MARGIN;
    prepared.frame.fill = if row.edited {
        tokens.accent_subtle
    } else if row.selected {
        tokens.hover
    } else if ui.rect_contains_pointer(rect) {
        tokens.stripe
    } else {
        Color32::TRANSPARENT
    };
    let row_rect = prepared.end(ui).rect;
    if row.edited {
        let bar = Rect::from_min_size(row_rect.min, vec2(EDITED_BAR_WIDTH, row_rect.height()));
        ui.painter()
            .rect_filled(bar, CornerRadius::same(WIDGET_RADIUS), tokens.accent);
    }
    if toggled || name.clicked() {
        collapsing.toggle(ui);
    }
    if name.clicked() || name.gained_focus() {
        state.selected = Some(id);
    }
    let mut name = name;
    if state.take_focus(Focus::Feature(id)) {
        name.scroll_to_me(Some(Align::Center));
        name = name.highlight();
    }
    if name.double_clicked() {
        start_renaming(state, feature);
    }
    name.context_menu(|ui| context_menu(ui, document, state, actions, row));

    match status.map(|status| &status.state) {
        Some(FeatureState::Failed(error)) => failure(ui, document, state, error),
        Some(FeatureState::Outdated) => {
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.label("Not recomputed, because the recompute was cancelled.");
                if ui.button("Recompute").clicked() {
                    actions.push(Action::Recompute);
                }
            });
        }
        Some(FeatureState::UpToDate) | None => {}
    }

    collapsing.show_body_unindented(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(BODY_INDENT);
            ui.vertical(|ui| {
                widgets::card(ui, |ui| body(ui, model, state, actions, row));
            });
        });
    });
    collapsing.store(ui.ctx());
}

fn body(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let feature = row.feature;
    match &feature.kind {
        FeatureKind::Sketch(sketch) => {
            placement(ui, model, row.selection, actions, feature, sketch);
            sketch_body(ui, model, state, actions, feature, &sketch.sketch);
        }
        FeatureKind::Solid(solid) => {
            solid_panel::show(
                ui,
                model,
                row.selection,
                actions,
                feature,
                solid,
                row.edited,
            );
            body_display(ui, model, feature);
        }
        FeatureKind::Blend(blend) => {
            blend_panel::show(ui, model, actions, feature, blend, row.edited);
            body_display(ui, model, feature);
        }
        FeatureKind::Shell(shell) => {
            shell_panel::show(ui, model, actions, feature, shell, row.edited);
            body_display(ui, model, feature);
        }
        FeatureKind::Datum(datum) => {
            datum_panel::show(ui, model, row.selection, actions, feature, datum);
        }
        FeatureKind::Import(import) => {
            ui.horizontal(|ui| {
                let muted = appearance::tokens(ui).text_muted;
                widgets::icon_label(ui, icons::FILE, muted);
                ui.label(format!("Imported from “{}”", import.source));
            });
            body_display(ui, model, feature);
        }
    }
}

fn state_color(ui: &Ui, status: Option<&FeatureStatus>) -> Option<Color32> {
    match status.map(|status| &status.state) {
        Some(FeatureState::Failed(_)) => Some(ui.visuals().error_fg_color),
        Some(FeatureState::Outdated) => Some(ui.visuals().warn_fg_color),
        Some(FeatureState::UpToDate) | None => None,
    }
}

fn name_text(ui: &Ui, feature: &Feature, status: Option<&FeatureStatus>) -> RichText {
    let text = RichText::new(&feature.name);
    match status {
        None => text.color(appearance::tokens(ui).text_muted),
        Some(_) => match state_color(ui, status) {
            Some(color) => text.color(color),
            None => text,
        },
    }
}

fn status_icon(ui: &mut Ui, status: Option<&FeatureStatus>) {
    let tokens = appearance::tokens(ui);
    let (glyph, color, hint) = match status.map(|status| &status.state) {
        Some(FeatureState::Failed(_)) => (icons::FAILED, tokens.error, "This feature failed"),
        Some(FeatureState::Outdated) => (
            icons::OUTDATED,
            tokens.warn,
            "Not recomputed, because the recompute was cancelled",
        ),
        None => (icons::PENDING, tokens.text_muted, "Waiting to be computed"),
        Some(FeatureState::UpToDate) => return,
    };
    widgets::icon_label(ui, glyph, color).on_hover_text(hint);
}

fn more_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let muted = appearance::tokens(ui).text_muted;
    let button = Button::new(widgets::icon(icons::MORE).color(muted)).frame_when_inactive(false);
    let (response, _) = MenuButton::from_button(button).ui(ui, |ui| {
        context_menu(ui, document, state, actions, row);
    });
    response.on_hover_text(MORE_HINT);
}

fn edit_button(ui: &mut Ui, row: &Row<'_>, actions: &mut Vec<Action>) {
    let Some((hover, command)) = edit_command(row) else {
        return;
    };
    let tokens = appearance::tokens(ui);
    let (glyph, color) = if row.edited {
        (icons::DONE, tokens.accent_text)
    } else {
        (icons::EDIT, tokens.text_muted)
    };
    let response = ui
        .add(Button::new(widgets::icon(glyph).color(color)).frame_when_inactive(false))
        .on_hover_text(hover);
    if response.clicked() {
        actions.push(Action::Editing(command));
    }
}

fn edit_command(row: &Row<'_>) -> Option<(&'static str, EditingCommand)> {
    let id = row.feature.id();
    Some(match (&row.feature.kind, row.edited) {
        (FeatureKind::Sketch(_), true) => (FINISH_SKETCH_LABEL, EditingCommand::Finish),
        (FeatureKind::Sketch(_), false) => (EDIT_SKETCH_LABEL, EditingCommand::Enter(id)),
        (
            FeatureKind::Solid(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Datum(_),
            true,
        ) => (CLOSE_SOLID_LABEL, EditingCommand::CloseSolid),
        (FeatureKind::Solid(_), false) => (OPEN_SOLID_LABEL, EditingCommand::OpenSolid(id)),
        (FeatureKind::Blend(_), false) => (OPEN_BLEND_LABEL, EditingCommand::OpenSolid(id)),
        (FeatureKind::Shell(_), false) => (OPEN_SHELL_LABEL, EditingCommand::OpenSolid(id)),
        (FeatureKind::Datum(_), false) => (OPEN_DATUM_LABEL, EditingCommand::OpenSolid(id)),
        (FeatureKind::Import(_), _) => return None,
    })
}

fn start_renaming(state: &mut PanelState, feature: &Feature) {
    state.renaming = Some(Renaming {
        feature: feature.id(),
        focus_pending: true,
    });
}

fn rename_row(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    renaming: Renaming,
) {
    let id = feature.id();
    let field = field::commit_field(
        ui,
        Id::new(("rename-feature", id)),
        &feature.name,
        NAME_FIELD_WIDTH,
        renaming.focus_pending,
        |text| {
            field::checked(
                document,
                Transaction::single(
                    format!("Rename {}", feature.name),
                    Edit::RenameFeature {
                        id,
                        name: text.to_owned(),
                    },
                ),
            )
        },
    );
    if let Some(error) = &field.error {
        field_error(ui, error);
    }
    if let Some(transaction) = field.committed {
        actions.push(Action::Apply(transaction));
    }
    state.renaming = if field.response.lost_focus() && field.error.is_none() {
        None
    } else {
        Some(Renaming {
            feature: id,
            focus_pending: false,
        })
    };
}

fn context_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let feature = row.feature;
    let position = row.position;
    if let Some((label, command)) = edit_command(row) {
        if widgets::menu_item(ui, icons::EDIT, label, None).clicked() {
            actions.push(Action::Editing(command));
            ui.close();
        }
        ui.separator();
    }
    if widgets::menu_item(ui, icons::RENAME, "Rename", None).clicked() {
        start_renaming(state, feature);
        ui.close();
    }
    for direction in Direction::BOTH {
        let transaction = direction.transaction(document, feature, position);
        let response = ui
            .add_enabled_ui(transaction.is_ok(), |ui| {
                widgets::menu_item(ui, direction.glyph(), direction.label(), None)
            })
            .inner;
        let response = match &transaction {
            Err(reason) => response.on_disabled_hover_text(reason),
            Ok(_) => response,
        };
        if response.clicked()
            && let Ok(transaction) = transaction
        {
            actions.push(Action::Apply(transaction));
            ui.close();
        }
    }
    ui.separator();
    let delete = delete_transaction(document, feature);
    let response = ui
        .add_enabled_ui(delete.is_ok(), |ui| {
            widgets::menu_item(ui, icons::DELETE, "Delete", None)
        })
        .inner;
    let response = match &delete {
        Err(reason) => response.on_disabled_hover_text(reason),
        Ok(_) => response,
    };
    if response.clicked()
        && let Ok(delete) = delete
    {
        actions.push(Action::Apply(delete));
        ui.close();
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Up,
    Down,
}

impl Direction {
    const BOTH: [Self; 2] = [Self::Up, Self::Down];

    fn label(self) -> &'static str {
        match self {
            Self::Up => "Move up",
            Self::Down => "Move down",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Up => icons::MOVE_UP,
            Self::Down => icons::MOVE_DOWN,
        }
    }

    fn command(self) -> Command {
        match self {
            Self::Up => Command::MoveFeatureUp,
            Self::Down => Command::MoveFeatureDown,
        }
    }

    fn transaction(
        self,
        document: &Document,
        feature: &Feature,
        position: Position,
    ) -> Result<Transaction, String> {
        let name = &feature.name;
        let index = match self {
            Self::Up => position
                .index
                .checked_sub(1)
                .ok_or_else(|| format!("{name} is already the first feature"))?,
            Self::Down => Some(position.index + 1)
                .filter(|below| *below < position.count)
                .ok_or_else(|| format!("{name} is already the last feature"))?,
        };
        let transaction = Transaction::single(
            format!("{} {name}", self.label()),
            Edit::MoveFeature {
                id: feature.id(),
                index,
            },
        );
        document
            .check(&transaction)
            .map_err(|error| error.to_string())?;
        Ok(transaction)
    }
}

fn delete_transaction(document: &Document, feature: &Feature) -> Result<Transaction, String> {
    let id = feature.id();
    document
        .can_remove_feature(id)
        .map_err(|error| error.to_string())?;
    Ok(Transaction::single(
        format!("Delete {}", feature.name),
        Edit::RemoveFeature { id },
    ))
}

pub fn current_feature<'a>(
    document: &'a Document,
    editing: &SketchEditing,
    state: &PanelState,
) -> Option<&'a Feature> {
    state
        .selected
        .and_then(|id| document.feature(id))
        .or_else(|| {
            editing
                .feature()
                .or(editing.solid())
                .and_then(|id| document.feature(id))
        })
}

pub fn commands(
    model: &Model,
    editing: &SketchEditing,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    let current = current_feature(document, editing, state);
    let chosen = current.ok_or(NO_FEATURE_CHOSEN);
    if commands.invoke(Command::RenameFeature, &chosen)
        && let Some(feature) = current
    {
        state.selected = Some(feature.id());
        start_renaming(state, feature);
    }
    let position = current.and_then(|feature| {
        Some(Position {
            index: document.feature_index(feature.id())?,
            count: document.features().len(),
        })
    });
    for direction in Direction::BOTH {
        let transaction = match current.zip(position) {
            Some((feature, position)) => direction.transaction(document, feature, position),
            None => Err(NO_FEATURE_CHOSEN.to_owned()),
        };
        if commands.invoke(direction.command(), &transaction)
            && let Ok(transaction) = transaction
        {
            actions.push(Action::Apply(transaction));
        }
    }
    let delete = current.map_or_else(
        || Err(NO_FEATURE_CHOSEN.to_owned()),
        |feature| delete_transaction(document, feature),
    );
    if commands.invoke(Command::DeleteFeature, &delete)
        && let Ok(delete) = delete
    {
        actions.push(Action::Apply(delete));
    }
    if editing.active().is_none() {
        let selected = state
            .selected
            .and_then(|id| document.feature(id))
            .ok_or(NOTHING_SELECTED.to_owned())
            .and_then(|feature| delete_transaction(document, feature));
        if commands.invoke(Command::DeleteSelection, &selected)
            && let Ok(delete) = selected
        {
            state.selected = None;
            actions.push(Action::Apply(delete));
        }
    }
}

fn failure(ui: &mut Ui, document: &Document, state: &mut PanelState, error: &FeatureError) {
    widgets::callout(ui, Tone::Error, |ui| {
        ui.label(&error.reason);
        ui.label(widgets::muted(&error.remedy, ui));
        let Some(target) = error.fix else {
            return;
        };
        let label = match target {
            FixTarget::Parameter(id) => {
                format!(
                    "Go to {}",
                    document.parameter_name(id).unwrap_or("the parameter")
                )
            }
            FixTarget::Dimension { .. } => "Edit the dimension".to_owned(),
            FixTarget::Feature(id) => format!(
                "Go to {}",
                document
                    .feature(id)
                    .map_or("the feature", |feature| feature.name.as_str())
            ),
            FixTarget::Constraint {
                feature,
                constraint,
            } => match document
                .feature(feature)
                .and_then(|owner| owner.kind.sketch())
            {
                Some(sketch) => format!("Go to {}", sketch.describe_constraint(constraint)),
                None => "Go to the constraint".to_owned(),
            },
        };
        if ui.button(label).clicked() {
            state.request_focus(target.into());
        }
    });
}

fn body_display(ui: &mut Ui, model: &Model, feature: &Feature) {
    let Some(body) = feature.body() else {
        return;
    };
    let failed = model
        .evaluation()
        .body_result(body)
        .and_then(|result| result.solid())
        .is_some_and(SolidResult::mesh_failed);
    if failed {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(
                "The body could not be drawn. Its shape is kept and later features still use it.",
            );
        });
    }
}

fn placement(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    sketch: &SketchFeature,
) {
    if let Some(attachment) = &sketch.attachment {
        ui.horizontal_wrapped(|ui| {
            let muted = appearance::tokens(ui).text_muted;
            widgets::icon_label(ui, icons::ATTACHED, muted);
            ui.label(format!(
                "Lies on {}",
                sketch_placement::describe(model.document(), attachment)
            ));
            if let Some(transaction) = sketch_placement::detach(model, feature.id()) {
                let detach = ui.small_button(DETACH_LABEL).on_hover_text(
                    "Keep the sketch where it is and stop following what it lies on",
                );
                if detach.clicked() {
                    actions.push(Action::Apply(transaction));
                }
            }
        });
    }
    if let Some(datum) = datum_tools::selected_datum_plane(model.document(), selection) {
        match sketch_placement::place_on_datum(model, feature.id(), datum) {
            Ok(transaction) => {
                let button = widgets::small_button(ui, icons::USE_SELECTED, PLACE_ON_PLANE_LABEL);
                let place = ui.add(button).on_hover_text(
                    "Move this sketch onto the selected plane; it follows the plane when the \
                     model changes",
                );
                if place.clicked() {
                    actions.push(Action::Apply(transaction));
                }
            }
            Err(reason) => {
                let button = widgets::small_button(ui, icons::USE_SELECTED, PLACE_ON_PLANE_LABEL);
                ui.add_enabled(false, button).on_disabled_hover_text(reason);
            }
        }
        return;
    }
    let Some(face) = sketch_placement::selected_face(selection) else {
        return;
    };
    match sketch_placement::place(model, feature.id(), face) {
        Ok(transaction) => {
            let button = widgets::small_button(ui, icons::USE_SELECTED, PLACE_ON_FACE_LABEL);
            let place = ui.add(button).on_hover_text(
                "Move this sketch onto the selected face; it follows the face when the model \
                 changes",
            );
            if place.clicked() {
                actions.push(Action::Apply(transaction));
            }
        }
        Err(reason) => {
            let button = widgets::small_button(ui, icons::USE_SELECTED, PLACE_ON_FACE_LABEL);
            ui.add_enabled(false, button).on_disabled_hover_text(reason);
        }
    }
}

fn sketch_body(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    sketch: &Sketch,
) {
    let document = model.document();
    let summary = SketchSummary::of(model.evaluation(), feature.id());
    ui.horizontal_wrapped(|ui| {
        if let Some(focus) = sketch_status::show(ui, &summary) {
            state.request_focus(focus);
        }
    });
    ui.label(widgets::muted(
        format!(
            "{} · {}",
            count(sketch.entities().len(), "entity", "entities"),
            count(sketch.constraints().len(), "constraint", "constraints")
        ),
        ui,
    ));
    let involved = involved_constraints(model, feature);
    let solution = sketch_status::up_to_date_solution(model.evaluation(), feature.id());
    for (constraint, definition) in sketch.constraints() {
        let description = sketch.describe_constraint(constraint);
        let redundancy = solution.and_then(|solution| solution.redundancy(constraint));
        let text = if involved.contains(&constraint) {
            RichText::new(&description).color(ui.visuals().error_fg_color)
        } else if redundancy.is_some() {
            RichText::new(&description).color(ui.visuals().warn_fg_color)
        } else {
            RichText::new(&description)
        };
        let delete = || {
            Action::Apply(sketch_tools::remove_items(
                model,
                feature.id(),
                format!("Delete {description}"),
                Vec::new(),
                vec![constraint],
            ))
        };
        let (row, deleted) = Sides::new().shrink_left().truncate().show(
            ui,
            |ui| widgets::link_label(ui, text),
            |ui| widgets::icon_button(ui, icons::DELETE, "Delete this constraint").clicked(),
        );
        let mut deleted = deleted;
        row.context_menu(|ui| {
            if widgets::menu_item(ui, icons::DELETE, "Delete", None).clicked() {
                deleted = true;
                ui.close();
            }
        });
        if deleted {
            actions.push(delete());
        }
        if let Some(redundancy) = redundancy {
            widgets::callout(ui, Tone::Warning, |ui| {
                ui.label(redundancy_text(sketch, redundancy));
            });
        }
        reveal_if_focused(
            state,
            row,
            Focus::Constraint {
                feature: feature.id(),
                constraint,
            },
        );
        let Some(expression) = definition.dimension() else {
            continue;
        };
        let focus = Focus::Dimension {
            feature: feature.id(),
            constraint,
        };
        let target = DimensionTarget {
            feature: feature.id(),
            constraint,
        };
        let mut error = None;
        ui.horizontal(|ui| {
            ui.add_space(DIMENSION_INDENT);
            let field = field::commit_field(
                ui,
                focus.field_id(),
                &document.expression_text(expression),
                DIMENSION_FIELD_WIDTH,
                state.wants_focus(focus),
                |text| {
                    field::dimension_transaction(
                        document,
                        model.parameters(),
                        target,
                        text,
                        model.length_unit(),
                    )
                },
            );
            state.focus_reached(focus, field.response.has_focus());
            if let Some(transaction) = field.committed {
                actions.push(Action::Apply(transaction));
            }
            if field.error.is_none()
                && let Some(preview) =
                    field::value_preview(model.parameters(), expression, model.length_unit())
            {
                ui.label(widgets::muted(preview, ui));
            }
            error = field.error;
        });
        if let Some(error) = error {
            ui.horizontal(|ui| {
                ui.add_space(DIMENSION_INDENT);
                field_error(ui, &error);
            });
        }
    }
}

fn redundancy_text(sketch: &Sketch, redundancy: &Redundancy) -> String {
    let duplicates: Vec<String> = redundancy
        .duplicates
        .iter()
        .map(|duplicate| sketch.describe_constraint(*duplicate))
        .collect();
    match duplicates.as_slice() {
        [] => "Redundant: other constraints already do this. Delete it.".to_owned(),
        [only] => format!("Redundant: {only} already does this. Delete one of them."),
        [rest @ .., last] => format!(
            "Redundant: {} and {last} already do this. Delete one of them.",
            rest.join(", ")
        ),
    }
}

fn involved_constraints(model: &Model, feature: &Feature) -> Vec<ConstraintId> {
    match model
        .evaluation()
        .feature(feature.id())
        .map(|status| &status.state)
    {
        Some(FeatureState::Failed(error)) => error.constraints.clone(),
        Some(FeatureState::UpToDate | FeatureState::Outdated) | None => Vec::new(),
    }
}

fn reveal_if_focused(state: &mut PanelState, row: Response, focus: Focus) {
    if state.take_focus(focus) {
        row.scroll_to_me(Some(Align::Center));
        row.highlight();
    }
}

fn field_error(ui: &mut Ui, error: &str) {
    ui.horizontal_wrapped(|ui| {
        let color = ui.visuals().error_fg_color;
        widgets::icon_label(ui, icons::FAILED, color);
        ui.colored_label(color, error);
    });
}

pub fn count(amount: usize, singular: &str, plural: &str) -> String {
    let noun = if amount == 1 { singular } else { plural };
    format!("{amount} {noun}")
}
