use caditor_document::{
    Datum, Document, Edit, Feature, FeatureError, FeatureId, FeatureKind, FeatureState,
    FeatureStatus, FixTarget, Healing, RollbackBar, SketchFeature, SolidFeature, SolidResult,
    Transaction, TreeRow,
};
use caditor_expression::Expression;
use caditor_sketch::{Constraint, ConstraintId, Redundancy, Sketch};
use egui::{
    Align, Align2, Area, Color32, CursorIcon, FontId, Frame, Id, Key, Label, Modifiers, Order,
    Popup, Pos2, Rect, Response, RichText, Sense, Sides, Stroke, TextStyle, Ui, WidgetInfo,
    WidgetType, collapsing_header::CollapsingState, pos2, vec2,
};

use crate::{
    appearance::{self, ICON_SIZE, SPACE_L, SPACE_M, SPACE_S},
    blend_panel, combine_panel,
    commands::{Command, CommandFrame},
    datum_panel,
    editing::{EditingCommand, SketchEditing},
    field::{self, DimensionTarget},
    files::FileCommand,
    fonts, icons,
    model::{Action, Model, Notice},
    move_panel,
    panels::{Focus, PanelState, Renaming},
    pattern_panel,
    pattern_tools::{self, Reference},
    preferences::PreferencesCommand,
    principal_tree,
    selection::{Pickable, Selection},
    shell_panel, sketch_export,
    sketch_placement::{self, PlacementTarget},
    sketch_status::{self, SketchSummary},
    sketch_tools, solid_panel,
    tree_row::{self, Look},
    visibility,
    widgets::{self, DialogWidth, Tone},
};

const NO_FEATURE_CHOSEN: &str = "Select a feature in the tree, or open one, first";
const NOTHING_SELECTED: &str =
    "Select geometry in an edited sketch, or a feature in the tree, to delete it";
const NOTHING_OPEN: &str = "No feature is open; open one with Edit feature first";
const MORE_HINT: &str = "Rename, move, suppress, roll back or delete (also on right-click)";
const DIMENSION_FIELD_WIDTH: f32 = 150.0;
const FINISH_SKETCH_LABEL: &str = "Finish sketch";
const PLACE_ON_PLANE_LABEL: &str = "Place on selected plane";
const PLACE_ON_FACE_LABEL: &str = "Place on selected face";
const DETACH_LABEL: &str = "Detach";
const RECOMPUTE_LABEL: &str = "Recompute";
const UPDATE_REFERENCES_LABEL: &str = "Update references";
const HEALED_HINT: &str =
    "Some of what it uses changed and was matched to the most similar geometry";
const NOTHING_HEALED: &str = "Every reference it holds is found as it was chosen";
pub const OPEN_SAMPLE_LABEL: &str = "Open a sample";
pub const EMPTY_TREE: &str = "The model has no features yet. Start with New sketch, here or in the ribbon, or open a \
     sample to see how one is built.";
pub const DIMENSIONS_TITLE: &str = "Dimensions";
pub const CONSTRAINTS_TITLE: &str = "Constraints";
pub const ROLLBACK_BAR_NAME: &str = "Rollback bar";
const ROLLBACK_BAR_HINT: &str = "Drag to roll the model back to any point: features below the \
                                 bar are not computed, and new features go in above it";
const SUPPRESSED_HINT: &str = "Suppressed: left out when the model is computed";
const ROLLED_BACK_HINT: &str = "Below the rollback bar: not computed";
const BAR_AT_END: &str = "The rollback bar is already at the end";
const BAR_AT_TOP: &str = "The rollback bar is already at the top";
pub const DELETE_WITH_DEPENDENTS: &str = "Delete with dependents";
pub const KEEP_DEPENDENTS: &str = "Keep dependents";
pub const CANCEL_DELETE: &str = "Cancel";
const BAR_ROW_HEIGHT: f32 = 16.0;
const BAR_THICKNESS: f32 = 3.0;
const DROP_LINE_WIDTH: f32 = 2.0;
const DROP_REASON_OFFSET: egui::Vec2 = vec2(18.0, 12.0);
const DROP_REASON_WIDTH: f32 = 280.0;
const DEPENDENTS_HEIGHT: f32 = 220.0;
const AUTOSCROLL_EDGE: f32 = 24.0;
const AUTOSCROLL_RATE: f32 = 0.5;

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
    tree_row::rows(ui, |ui| {
        rows(ui, model, selection, editing, state, actions);
    });
}

fn rows(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let document = model.document();
    principal_tree::show(ui, model, selection, state, actions);
    let count = document.features().len();
    if count == 0 {
        tree_row::content(ui, |ui| empty_tree(ui, actions));
        state.dragging = None;
        return;
    }
    let bar = document.bar_index();
    let chosen = state.chosen();
    let mut placed = Vec::with_capacity(count + 1);
    for (index, feature) in document.features().enumerate() {
        if index == bar {
            placed.push((TreeRow::Bar, rollback_bar(ui, document, state)));
        }
        let id = feature.id();
        let row = Row {
            feature,
            selection,
            edited: editing.feature() == Some(id) || editing.solid() == Some(id),
            selected: chosen.contains(&id),
            rolled_back: index >= bar,
        };
        let rect = ui
            .push_id(("feature", id), |ui| {
                feature_row(ui, model, state, actions, &row)
            })
            .inner;
        placed.push((TreeRow::Feature(id), rect));
    }
    if bar >= count {
        placed.push((TreeRow::Bar, rollback_bar(ui, document, state)));
    }
    drag_and_drop(ui, document, state, actions, &placed);
}

fn empty_tree(ui: &mut Ui, actions: &mut Vec<Action>) {
    widgets::empty_state(ui, icons::FEATURES, EMPTY_TREE, |ui| {
        let title = Command::NewSketch.title();
        let new_sketch = widgets::small_button(ui, icons::command(Command::NewSketch), &title);
        if ui
            .add(new_sketch)
            .on_hover_text("Start a sketch on the plane or flat face you click next")
            .clicked()
        {
            actions.push(Action::Editing(EditingCommand::NewSketch(None)));
        }
        let sample = widgets::small_button(ui, icons::SAMPLE, OPEN_SAMPLE_LABEL);
        if ui
            .add(sample)
            .on_hover_text("Choose one of the sample models to open")
            .clicked()
        {
            actions.push(Action::Preferences(PreferencesCommand::ShowWelcome));
        }
    });
}

struct Row<'a> {
    feature: &'a Feature,
    selection: &'a Selection,
    edited: bool,
    selected: bool,
    rolled_back: bool,
}

impl Row<'_> {
    fn active(&self) -> bool {
        !self.rolled_back && !self.feature.suppressed
    }
}

fn rollback_bar(ui: &mut Ui, document: &Document, state: &mut PanelState) -> Rect {
    let tokens = appearance::tokens(ui);
    let below = document
        .features()
        .len()
        .saturating_sub(document.bar_index());
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), BAR_ROW_HEIGHT),
        Sense::click_and_drag(),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, ROLLBACK_BAR_NAME));
    let response = response
        .on_hover_text(ROLLBACK_BAR_HINT)
        .on_hover_cursor(CursorIcon::Grab);
    if response.drag_started() {
        state.dragging = Some(TreeRow::Bar);
    }
    let emphasised =
        response.hovered() || response.has_focus() || state.dragging == Some(TreeRow::Bar);
    let color = if emphasised {
        tokens.accent
    } else {
        tokens.accent_text
    };
    let painter = ui.painter();
    let middle = rect.center().y;
    let grip = painter.text(
        pos2(rect.left() + SPACE_S, middle),
        Align2::LEFT_CENTER,
        icons::ROLLBACK_BAR,
        FontId::new(ICON_SIZE, fonts::icons()),
        color,
    );
    let mut start = grip.right() + SPACE_M;
    if below > 0 {
        let caption = painter.text(
            pos2(start, middle),
            Align2::LEFT_CENTER,
            format!("{} rolled back", count(below, "feature", "features")),
            TextStyle::Small.resolve(ui.style()),
            tokens.text_muted,
        );
        start = caption.right() + SPACE_M;
    }
    if start < rect.right() {
        painter.hline(
            start..=rect.right(),
            middle,
            Stroke::new(BAR_THICKNESS, color),
        );
    }
    if response.has_focus() {
        tree_row::focus_outline(ui, rect);
    }
    rect
}

fn drag_and_drop(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    placed: &[(TreeRow, Rect)],
) {
    let Some(dragged) = state.dragging else {
        return;
    };
    let (pointer, held, cancelled) = ui.input(|input| {
        (
            input.pointer.latest_pos(),
            input.pointer.primary_down(),
            input.key_pressed(Key::Escape),
        )
    });
    let Some(pointer) = pointer.filter(|_| !cancelled) else {
        state.dragging = None;
        return;
    };
    let visible = ui.clip_rect();
    if !visible.contains(pointer) {
        if !held {
            state.dragging = None;
        }
        return;
    }
    let beyond_top = visible.top() + AUTOSCROLL_EDGE - pointer.y;
    let beyond_bottom = pointer.y - (visible.bottom() - AUTOSCROLL_EDGE);
    if held && beyond_top > 0.0 {
        ui.scroll_with_delta(vec2(0.0, beyond_top * AUTOSCROLL_RATE));
    } else if held && beyond_bottom > 0.0 {
        ui.scroll_with_delta(vec2(0.0, -beyond_bottom * AUTOSCROLL_RATE));
    }
    let gap = placed
        .iter()
        .position(|(_, rect)| pointer.y < rect.center().y)
        .unwrap_or(placed.len());
    let outcome = document.move_row(dragged, gap, drag_label(document, dragged));
    if !held {
        state.dragging = None;
        if let Ok(transaction) = outcome
            && !transaction.is_empty()
        {
            actions.push(Action::Apply(transaction));
        }
        return;
    }
    let tokens = appearance::tokens(ui);
    let line = match placed.get(gap) {
        Some((_, rect)) => rect.top() - tree_row::ROW_GAP / 2.0,
        None => placed.last().map_or(pointer.y, |(_, rect)| {
            rect.bottom() + tree_row::ROW_GAP / 2.0
        }),
    };
    let span = placed
        .first()
        .map_or_else(|| ui.max_rect().x_range(), |(_, rect)| rect.x_range());
    match &outcome {
        Ok(transaction) if transaction.is_empty() => {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        Ok(_) => {
            ui.painter()
                .hline(span, line, Stroke::new(DROP_LINE_WIDTH, tokens.accent));
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        Err(error) => {
            ui.painter()
                .hline(span, line, Stroke::new(DROP_LINE_WIDTH, tokens.error));
            ui.ctx().set_cursor_icon(CursorIcon::NotAllowed);
            drop_refusal(ui.ctx(), pointer, &error.to_string());
        }
    }
    ui.ctx().request_repaint();
}

fn drag_label(document: &Document, row: TreeRow) -> String {
    match row {
        TreeRow::Feature(id) => format!("Move {}", feature_name(document, id)),
        TreeRow::Bar => "Move the rollback bar".to_owned(),
    }
}

fn drop_refusal(ctx: &egui::Context, pointer: Pos2, reason: &str) {
    Area::new(Id::new("feature-tree-drop-refusal"))
        .order(Order::Tooltip)
        .fixed_pos(pointer + DROP_REASON_OFFSET)
        .interactable(false)
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(DROP_REASON_WIDTH);
                ui.horizontal_wrapped(|ui| {
                    let color = appearance::tokens(ui).error;
                    widgets::icon_label(ui, icons::FAILED, color);
                    ui.label(reason);
                });
            });
        });
}

fn feature_name(document: &Document, id: FeatureId) -> String {
    document
        .feature(id)
        .map_or_else(|| "the feature".to_owned(), |feature| feature.name.clone())
}

enum Name {
    Shown(Response),
    Renaming(field::FieldResponse<Transaction>),
}

fn feature_row(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) -> Rect {
    let document = model.document();
    let feature = row.feature;
    let id = feature.id();
    let status = model.evaluation().feature(id);
    let renaming = state.renaming.filter(|renaming| renaming.feature == id);

    let mut collapsing = CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("feature-header", id)),
        false,
    );
    let editing_started = row.edited && state.opened_for_editing != Some(id);
    if editing_started {
        state.opened_for_editing = Some(id);
        state.reveal(id);
    }
    if state.focus_inside(id) || editing_started {
        collapsing.set_open(true);
    }

    let tokens = appearance::tokens(ui);
    let open = collapsing.is_open();
    let look = Look {
        selected: row.selected,
        open: row.edited,
    };
    let shown = tree_row::show(
        ui,
        look,
        |ui| {
            let toggle = tree_row::chevron(ui, open, &feature.name);
            widgets::icon_label(ui, icons::feature(&feature.kind), kind_color(tokens, row));
            let name = match renaming {
                Some(renaming) => Name::Renaming(rename_field(ui, document, feature, renaming)),
                None => {
                    ui.add(
                        Label::new(name_text(ui, row, status))
                            .selectable(false)
                            .truncate(),
                    );
                    Name::Shown(tree_row::sense(
                        ui,
                        Id::new(("feature-row", id)),
                        toggle.rect.right(),
                        &feature.name,
                        row.selected,
                    ))
                }
            };
            (toggle.clicked(), name)
        },
        |ui| {
            tree_row::slot(ui, |ui| more_menu(ui, document, state, actions, row));
            tree_row::slot(ui, |ui| visibility_button(ui, row, actions));
            tree_row::slot(ui, |ui| {
                if row.active() || row.edited {
                    edit_button(ui, row, actions);
                }
            });
            tree_row::slot(ui, |ui| status_icon(ui, row, status));
        },
    );
    let row_rect = shown.rect;
    let (toggled, name) = shown.leading;
    if toggled {
        collapsing.toggle(ui);
    }
    match name {
        Name::Shown(name) => {
            if name.has_focus() {
                tree_row::focus_outline(ui, row_rect);
            }
            respond(ui, document, state, actions, row, &mut collapsing, name);
        }
        Name::Renaming(field) => finish_renaming(ui, state, actions, id, field),
    }

    let state_shown = status.map(|status| &status.state);
    let healing = status
        .filter(|status| status.state == FeatureState::UpToDate)
        .and_then(|status| status.healing.as_deref());
    let has_callout = healing.is_some()
        || matches!(
            state_shown,
            Some(FeatureState::Failed(_) | FeatureState::Outdated)
        );
    if has_callout || collapsing.openness(ui.ctx()) > 0.0 {
        let below = tree_row::indented(ui, |ui| {
            match state_shown {
                Some(FeatureState::Failed(error)) => {
                    failure(ui, document, state, actions, error);
                }
                Some(FeatureState::Outdated) => outdated(ui, actions),
                Some(
                    FeatureState::UpToDate | FeatureState::Suppressed | FeatureState::RolledBack,
                )
                | None => {}
            }
            if let Some(healing) = healing {
                healed(ui, document, actions, healing);
            }
            collapsing.show_body_unindented(ui, |ui| {
                widgets::card(ui, |ui| body(ui, model, state, actions, row));
            })
        });
        ui.add_space(SPACE_S);
        if state.revealing(id) {
            let card = below.map_or(row_rect, |below| row_rect.union(below.response.rect));
            ui.scroll_to_rect(card, None);
        }
    } else if state.revealing(id) {
        ui.scroll_to_rect(row_rect, None);
    }
    collapsing.store(ui.ctx());
    row_rect
}

fn outdated(ui: &mut Ui, actions: &mut Vec<Action>) {
    widgets::callout(ui, Tone::Warning, |ui| {
        ui.label("Not recomputed, because the recompute was cancelled.");
        let recompute =
            widgets::small_button(ui, icons::command(Command::Recompute), RECOMPUTE_LABEL);
        if ui.add(recompute).clicked() {
            actions.push(Action::Recompute);
        }
    });
}

fn healed(ui: &mut Ui, document: &Document, actions: &mut Vec<Action>, healing: &Healing) {
    widgets::callout(ui, Tone::Warning, |ui| {
        ui.label(healing.reason());
        ui.label(widgets::muted(healing.remedy(), ui));
        let Some(update) = healing.update(document) else {
            return;
        };
        let button = widgets::small_button(
            ui,
            icons::command(Command::UpdateReferences),
            UPDATE_REFERENCES_LABEL,
        );
        if ui.add(button).clicked() {
            actions.push(Action::Apply(update));
        }
    });
}

fn respond(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
    collapsing: &mut CollapsingState,
    name: Response,
) {
    let id = row.feature.id();
    let (modifiers, enter) = ui.input(|input| (input.modifiers, input.key_pressed(Key::Enter)));
    if name.clicked() {
        choose(state, document, id, modifiers);
    } else if name.gained_focus() {
        state.choose_only(id);
    }
    if name.drag_started() {
        state.dragging = Some(TreeRow::Feature(id));
    }
    if state.take_focus(Focus::Feature(id)) {
        state.choose_only(id);
        name.scroll_to_me(Some(Align::Center));
    }
    if name.double_clicked() || (name.has_focus() && enter) {
        open_feature(ui, document, actions, row, collapsing);
    }
    name.context_menu(|ui| context_menu(ui, document, state, actions, row));
}

fn open_feature(
    ui: &Ui,
    document: &Document,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
    collapsing: &mut CollapsingState,
) {
    if row.edited {
        collapsing.set_open(true);
        return;
    }
    let feature = row.feature;
    match edit_command(feature, false) {
        None => collapsing.toggle(ui),
        Some(command) => match editable(document, feature) {
            Ok(()) => actions.push(Action::Editing(command)),
            Err(reason) => actions.push(Action::Inform(Notice::info(reason))),
        },
    }
}

fn choose(state: &mut PanelState, document: &Document, id: FeatureId, modifiers: Modifiers) {
    if modifiers.command {
        state.toggle_chosen(id);
        return;
    }
    let anchor = state
        .selected
        .and_then(|anchor| document.feature_index(anchor));
    match (modifiers.shift, anchor, document.feature_index(id)) {
        (true, Some(anchor), Some(end)) => {
            let (first, last) = (anchor.min(end), anchor.max(end));
            let range = document
                .features()
                .skip(first)
                .take(last - first + 1)
                .map(Feature::id)
                .collect();
            state.choose_range(range);
        }
        _ => state.choose_only(id),
    }
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
            blend_panel::show(
                ui,
                model,
                &mut state.reference_rows,
                actions,
                feature,
                blend,
                row.edited,
            );
            body_display(ui, model, feature);
        }
        FeatureKind::Shell(shell) => {
            shell_panel::show(
                ui,
                model,
                &mut state.reference_rows,
                actions,
                feature,
                shell,
                row.edited,
            );
            body_display(ui, model, feature);
        }
        FeatureKind::Combine(combine) => {
            combine_panel::show(ui, model, actions, feature, combine);
            body_display(ui, model, feature);
        }
        FeatureKind::Move(movement) => {
            move_panel::show(ui, model, actions, feature, movement);
            body_display(ui, model, feature);
        }
        FeatureKind::Pattern(pattern) => {
            pattern_panel::show(ui, model, row.selection, actions, feature, pattern);
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

fn kind_color(tokens: &appearance::Tokens, row: &Row<'_>) -> Color32 {
    if row.edited {
        return tokens.accent_text;
    }
    if !row.active() || row.feature.hidden {
        return tokens.text_muted;
    }
    match row.feature.kind {
        FeatureKind::Sketch(_) => tokens.accent_text,
        FeatureKind::Datum(_) => tokens.text_muted,
        FeatureKind::Solid(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Move(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Import(_) => tokens.text,
    }
}

fn name_text(ui: &Ui, row: &Row<'_>, status: Option<&FeatureStatus>) -> RichText {
    let feature = row.feature;
    let muted = appearance::tokens(ui).text_muted;
    let text = RichText::new(&feature.name);
    if feature.suppressed {
        return text.color(muted).strikethrough();
    }
    if row.rolled_back || status.is_none() {
        return text.color(muted);
    }
    if feature.hidden {
        return text.color(muted).italics();
    }
    text
}

fn visibility_button(ui: &mut Ui, row: &Row<'_>, actions: &mut Vec<Action>) {
    let feature = row.feature;
    let Ok(transaction) = visibility::toggle(feature) else {
        return;
    };
    let (glyph, hover) = if feature.hidden {
        (icons::HIDE, "Show")
    } else {
        (icons::SHOW, "Hide")
    };
    let response = widgets::named(
        widgets::icon_button(ui, glyph, hover),
        &format!("{hover} {}", feature.name),
    );
    if response.clicked() {
        actions.push(Action::Apply(transaction));
    }
}

fn status_icon(ui: &mut Ui, row: &Row<'_>, status: Option<&FeatureStatus>) {
    let tokens = appearance::tokens(ui);
    let waiting = (icons::PENDING, tokens.text_muted, "Waiting to be computed");
    let (glyph, color, hint) = if row.feature.suppressed {
        (icons::SUPPRESS, tokens.text_muted, SUPPRESSED_HINT)
    } else if row.rolled_back {
        (icons::ROLLED_BACK, tokens.text_muted, ROLLED_BACK_HINT)
    } else {
        match status.map(|status| &status.state) {
            Some(FeatureState::Failed(_)) => (icons::FAILED, tokens.error, "This feature failed"),
            Some(FeatureState::Outdated) => (
                icons::OUTDATED,
                tokens.warn,
                "Not recomputed, because the recompute was cancelled",
            ),
            Some(FeatureState::Suppressed | FeatureState::RolledBack) | None => waiting,
            Some(FeatureState::UpToDate)
                if status.is_some_and(|status| status.healing.is_some()) =>
            {
                (icons::WARNING, tokens.warn, HEALED_HINT)
            }
            Some(FeatureState::UpToDate) => return,
        }
    };
    widgets::described_icon(ui, glyph, color, hint);
}

fn more_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let response = widgets::named(
        widgets::icon_button(ui, icons::MORE, MORE_HINT),
        &format!("More actions for {}", row.feature.name),
    );
    Popup::menu(&response).show(|ui| {
        context_menu(ui, document, state, actions, row);
    });
}

fn edit_button(ui: &mut Ui, row: &Row<'_>, actions: &mut Vec<Action>) {
    let Some(command) = edit_command(row.feature, row.edited) else {
        return;
    };
    let glyph = if row.edited { icons::DONE } else { icons::EDIT };
    let title = edit_title(row.feature, row.edited);
    let response = widgets::icon_button(ui, glyph, &title);
    if response.clicked() {
        actions.push(Action::Editing(command));
    }
}

fn edit_command(feature: &Feature, edited: bool) -> Option<EditingCommand> {
    let id = feature.id();
    Some(match (&feature.kind, edited) {
        (FeatureKind::Sketch(_), true) => EditingCommand::Finish,
        (FeatureKind::Sketch(_), false) => EditingCommand::Enter(id),
        (
            FeatureKind::Solid(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Combine(_)
            | FeatureKind::Move(_)
            | FeatureKind::Pattern(_)
            | FeatureKind::Datum(_),
            true,
        ) => EditingCommand::CloseSolid,
        (
            FeatureKind::Solid(_)
            | FeatureKind::Blend(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Combine(_)
            | FeatureKind::Move(_)
            | FeatureKind::Pattern(_)
            | FeatureKind::Datum(_),
            false,
        ) => EditingCommand::OpenSolid(id),
        (FeatureKind::Import(_), _) => return None,
    })
}

pub fn edit_title(feature: &Feature, edited: bool) -> String {
    match (&feature.kind, edited) {
        (FeatureKind::Sketch(_), true) => FINISH_SKETCH_LABEL.to_owned(),
        (_, true) => Command::CloseFeature.title(),
        (_, false) => format!("Edit {}", feature.name),
    }
}

fn start_renaming(state: &mut PanelState, feature: &Feature) {
    state.renaming = Some(Renaming {
        feature: feature.id(),
        focus_pending: true,
    });
}

fn rename_field(
    ui: &mut Ui,
    document: &Document,
    feature: &Feature,
    renaming: Renaming,
) -> field::FieldResponse<Transaction> {
    let id = feature.id();
    let width = ui.available_width();
    field::commit_field(
        ui,
        Id::new(("rename-feature", id)),
        &feature.name,
        width,
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
    )
}

fn finish_renaming(
    ui: &mut Ui,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    id: FeatureId,
    field: field::FieldResponse<Transaction>,
) {
    if let Some(error) = &field.error {
        tree_row::indented(ui, |ui| field_error(ui, error));
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

fn menu_entry<T>(ui: &mut Ui, glyph: &str, label: &str, outcome: &Result<T, String>) -> bool {
    let response = ui
        .add_enabled_ui(outcome.is_ok(), |ui| {
            widgets::menu_item(ui, glyph, label, None)
        })
        .inner;
    let response = match outcome {
        Err(reason) => response.on_disabled_hover_text(reason),
        Ok(_) => response,
    };
    let clicked = response.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn context_menu(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    row: &Row<'_>,
) {
    let feature = row.feature;
    if let Some(command) = edit_command(feature, row.edited) {
        let editable = if row.edited {
            Ok(command)
        } else {
            editable(document, feature).map(|()| command)
        };
        let glyph = if row.edited { icons::DONE } else { icons::EDIT };
        if menu_entry(ui, glyph, &edit_title(feature, row.edited), &editable)
            && let Ok(command) = editable
        {
            actions.push(Action::Editing(command));
        }
        ui.separator();
    }
    if widgets::menu_item(ui, icons::RENAME, "Rename", None).clicked() {
        start_renaming(state, feature);
        ui.close();
    }
    if let Ok(transaction) = visibility::toggle(feature) {
        let (glyph, label) = if feature.hidden {
            (icons::SHOW, "Show")
        } else {
            (icons::HIDE, "Hide")
        };
        if widgets::menu_item(ui, glyph, label, None).clicked() {
            actions.push(Action::Apply(transaction));
            ui.close();
        }
    }
    let chosen = state.chosen();
    let targets = targets(document, &chosen, Some(feature));
    let (glyph, label) = suppress_title(&targets);
    let suppression = suppress_change(document, &targets);
    if menu_entry(ui, glyph, label, &suppression)
        && let Ok(transaction) = suppression
    {
        actions.push(Action::Apply(transaction));
    }
    for direction in Direction::BOTH {
        let transaction = direction.transaction(document, feature);
        if menu_entry(ui, direction.glyph(), direction.label(), &transaction)
            && let Ok(transaction) = transaction
        {
            actions.push(Action::Apply(transaction));
        }
    }
    ui.separator();
    let here = roll_to_here(document, feature);
    let here_label = format!("{} here", roll_verb(document, feature));
    if menu_entry(ui, icons::ROLL_TO_HERE, &here_label, &here)
        && let Ok(transaction) = here
    {
        actions.push(Action::Apply(transaction));
    }
    if document.rollback_bar() != RollbackBar::AtEnd {
        let end = roll_to_end(document);
        if menu_entry(ui, icons::ROLL_TO_END, "Roll to end", &end)
            && let Ok(transaction) = end
        {
            actions.push(Action::Apply(transaction));
        }
    }
    ui.separator();
    let delete = delete_request(document, &targets);
    let label = match delete {
        Ok(Deletion::Ask(_)) => "Delete…",
        Ok(Deletion::Now(_)) | Err(_) => "Delete",
    };
    if menu_entry(ui, icons::DELETE, label, &delete)
        && let Ok(deletion) = delete
    {
        deletion.perform(state, actions);
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

    fn transaction(self, document: &Document, feature: &Feature) -> Result<Transaction, String> {
        let name = &feature.name;
        let row = TreeRow::Feature(feature.id());
        let mut rows = document.tree_rows();
        if document.rollback_bar() == RollbackBar::AtEnd {
            rows.pop();
        }
        let at = rows
            .iter()
            .position(|candidate| *candidate == row)
            .ok_or_else(|| format!("{name} no longer exists"))?;
        let gap = match self {
            Self::Up => at
                .checked_sub(1)
                .ok_or_else(|| format!("{name} is already the first feature"))?,
            Self::Down => Some(at + 2)
                .filter(|gap| *gap <= rows.len())
                .ok_or_else(|| format!("{name} is already the last feature"))?,
        };
        document
            .move_row(row, gap, format!("{} {name}", self.label()))
            .map_err(|error| error.to_string())
    }
}

fn editable(document: &Document, feature: &Feature) -> Result<(), String> {
    let name = &feature.name;
    if feature.suppressed {
        return Err(format!("{name} is suppressed; unsuppress it to edit it"));
    }
    if document.is_rolled_back(feature.id()) {
        return Err(format!(
            "{name} is below the rollback bar; roll forward past it to edit it"
        ));
    }
    Ok(())
}

fn targets<'a>(
    document: &'a Document,
    chosen: &[FeatureId],
    current: Option<&'a Feature>,
) -> Vec<&'a Feature> {
    match current {
        Some(feature) if !chosen.contains(&feature.id()) => vec![feature],
        _ if !chosen.is_empty() => document
            .features()
            .filter(|feature| chosen.contains(&feature.id()))
            .collect(),
        _ => current.into_iter().collect(),
    }
}

fn described(targets: &[&Feature]) -> String {
    match targets {
        [only] => only.name.clone(),
        _ => count(targets.len(), "feature", "features"),
    }
}

fn suppress_title(targets: &[&Feature]) -> (&'static str, &'static str) {
    if targets.iter().all(|feature| feature.suppressed) && !targets.is_empty() {
        (icons::UNSUPPRESS, "Unsuppress")
    } else {
        (icons::SUPPRESS, "Suppress")
    }
}

fn suppress_change(document: &Document, targets: &[&Feature]) -> Result<Transaction, String> {
    if targets.is_empty() {
        return Err(NO_FEATURE_CHOSEN.to_owned());
    }
    let suppress = targets.iter().any(|feature| !feature.suppressed);
    let verb = if suppress { "Suppress" } else { "Unsuppress" };
    let ids: Vec<FeatureId> = targets.iter().map(|feature| feature.id()).collect();
    Ok(document.suppression(&ids, suppress, format!("{verb} {}", described(targets))))
}

fn reference_update(model: &Model, targets: &[&Feature]) -> Result<Transaction, String> {
    if targets.is_empty() {
        return Err(NO_FEATURE_CHOSEN.to_owned());
    }
    let document = model.document();
    let updates: Vec<Transaction> = targets
        .iter()
        .filter_map(|feature| {
            model
                .evaluation()
                .feature(feature.id())?
                .healing
                .as_ref()?
                .update(document)
        })
        .collect();
    match updates.as_slice() {
        [] => Err(NOTHING_HEALED.to_owned()),
        [only] => Ok(only.clone()),
        _ => Ok(Transaction::new(
            format!(
                "Update references of {}",
                count(updates.len(), "feature", "features")
            ),
            updates
                .iter()
                .flat_map(|update| update.edits().iter().cloned())
                .collect(),
        )),
    }
}

fn unsuppress(document: &Document, id: FeatureId) -> Transaction {
    document.suppression(
        &[id],
        false,
        format!("Unsuppress {}", feature_name(document, id)),
    )
}

fn roll_to_here(document: &Document, feature: &Feature) -> Result<Transaction, String> {
    let name = &feature.name;
    let index = document
        .feature_index(feature.id())
        .ok_or_else(|| format!("{name} no longer exists"))?;
    let bar = document.bar_before_index(index + 1);
    if bar == document.rollback_bar() {
        return Err(format!("The rollback bar is already right below {name}"));
    }
    Ok(document.roll_to(bar, format!("{} {name}", roll_verb(document, feature))))
}

fn roll_verb(document: &Document, feature: &Feature) -> &'static str {
    if document.is_rolled_back(feature.id()) {
        "Roll forward to"
    } else {
        "Roll back to"
    }
}

fn roll_to_end(document: &Document) -> Result<Transaction, String> {
    if document.rollback_bar() == RollbackBar::AtEnd {
        return Err(BAR_AT_END.to_owned());
    }
    Ok(document.roll_to(RollbackBar::AtEnd, "Roll to end"))
}

fn step_bar(document: &Document, up: bool) -> Result<Transaction, String> {
    let rows = document.tree_rows();
    let at = rows
        .iter()
        .position(|row| *row == TreeRow::Bar)
        .unwrap_or(rows.len());
    let gap = if up {
        at.checked_sub(1).ok_or(BAR_AT_TOP)?
    } else {
        Some(at + 2)
            .filter(|gap| *gap <= rows.len())
            .ok_or(BAR_AT_END)?
    };
    let label = if up {
        Command::RollbackUp.title()
    } else {
        Command::RollbackDown.title()
    };
    document
        .move_row(TreeRow::Bar, gap, label)
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone)]
enum Deletion {
    Now(Transaction),
    Ask(Vec<FeatureId>),
}

impl Deletion {
    fn perform(self, state: &mut PanelState, actions: &mut Vec<Action>) {
        match self {
            Self::Now(transaction) => {
                state.selected = None;
                actions.push(Action::Apply(transaction));
            }
            Self::Ask(features) => state.deleting = Some(features),
        }
    }
}

fn delete_request(document: &Document, targets: &[&Feature]) -> Result<Deletion, String> {
    if targets.is_empty() {
        return Err(NO_FEATURE_CHOSEN.to_owned());
    }
    let ids: Vec<FeatureId> = targets.iter().map(|feature| feature.id()).collect();
    if document.dependents_of(&ids).is_empty() {
        let label = format!("Delete {}", described(targets));
        return Ok(Deletion::Now(document.deletion(&ids, label)));
    }
    Ok(Deletion::Ask(ids))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteChoice {
    WithDependents,
    KeepDependents,
    Cancel,
}

pub fn delete_dialog(
    ctx: &egui::Context,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let Some(doomed) = state.deleting.clone() else {
        return;
    };
    let targets: Vec<&Feature> = document
        .features()
        .filter(|feature| doomed.contains(&feature.id()))
        .collect();
    let ids: Vec<FeatureId> = targets.iter().map(|feature| feature.id()).collect();
    let dependents = document.dependents_of(&ids);
    if targets.is_empty() {
        state.deleting = None;
        return;
    }
    let subject = match targets.as_slice() {
        [only] => format!("“{}”", only.name),
        _ => count(targets.len(), "feature", "features"),
    };
    let title = format!("Delete {subject}?");
    let response = widgets::dialog(ctx, "delete-features", &title, DialogWidth::Medium, |ui| {
        let verb = if dependents.len() == 1 {
            "depends"
        } else {
            "depend"
        };
        ui.label(format!(
            "{} {verb} on {subject}:",
            count(dependents.len(), "feature", "features")
        ));
        egui::ScrollArea::vertical()
            .max_height(widgets::list_height(ui.ctx(), DEPENDENTS_HEIGHT))
            .show(ui, |ui| {
                dependent_rows(ui, document, &ids, &dependents);
            });
        ui.label(widgets::muted(
            format!(
                "{DELETE_WITH_DEPENDENTS} removes them as well. {KEEP_DEPENDENTS} leaves them in \
                 the tree, where they fail until you undo the deletion or delete them too."
            ),
            ui,
        ));
        delete_footer(ui)
    });
    let closed = response.should_close().then_some(DeleteChoice::Cancel);
    let Some(choice) = response.inner.or(closed) else {
        return;
    };
    state.deleting = None;
    let everything: Vec<FeatureId> = ids.iter().chain(&dependents).copied().collect();
    let transaction = match choice {
        DeleteChoice::WithDependents => {
            let whose = if targets.len() == 1 { "its" } else { "their" };
            document.deletion(
                &everything,
                format!("Delete {} and {whose} dependents", described(&targets)),
            )
        }
        DeleteChoice::KeepDependents => {
            document.deletion(&ids, format!("Delete {}", described(&targets)))
        }
        DeleteChoice::Cancel => return,
    };
    state.selected = None;
    actions.push(Action::Apply(transaction));
}

fn delete_footer(ui: &mut Ui) -> Option<DeleteChoice> {
    widgets::footer(ui, |ui| {
        let cancel = ui.add(widgets::button(CANCEL_DELETE));
        let focused_here = ui
            .memory(|memory| memory.focused())
            .and_then(|focused| ui.ctx().read_response(focused))
            .is_some_and(|focused| focused.layer_id == ui.layer_id());
        if !focused_here {
            cancel.request_focus();
        }
        if cancel.has_focus() {
            widgets::paint_focus_ring(ui, cancel.rect);
        }
        if cancel.clicked() {
            return Some(DeleteChoice::Cancel);
        }
        if ui.add(widgets::button(KEEP_DEPENDENTS)).clicked() {
            return Some(DeleteChoice::KeepDependents);
        }
        ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
            ui.add(widgets::danger_button(DELETE_WITH_DEPENDENTS))
                .clicked()
                .then_some(DeleteChoice::WithDependents)
        })
        .inner
    })
}

fn dependent_rows(ui: &mut Ui, document: &Document, ids: &[FeatureId], dependents: &[FeatureId]) {
    let muted = appearance::tokens(ui).text_muted;
    for (index, dependent) in dependents.iter().enumerate() {
        let Some(feature) = document.feature(*dependent) else {
            continue;
        };
        let upstream: Vec<&FeatureId> = ids.iter().chain(dependents.iter().take(index)).collect();
        let uses: Vec<String> = feature
            .kind
            .dependencies()
            .into_iter()
            .filter(|used| upstream.contains(&used))
            .map(|used| feature_name(document, used))
            .collect();
        ui.horizontal_wrapped(|ui| {
            widgets::icon_label(ui, icons::feature(&feature.kind), muted);
            ui.label(&feature.name);
            ui.label(widgets::muted(format!("uses {}", in_words(&uses)), ui));
        });
    }
}

fn in_words(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
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

type FeatureChange<'a> = &'a dyn Fn(&Feature) -> Result<Transaction, String>;

pub struct CommandContext<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub editing: &'a SketchEditing,
}

fn is_edited(editing: &SketchEditing, feature: &Feature) -> bool {
    let id = Some(feature.id());
    editing.feature() == id || editing.solid() == id
}

fn edit_change(
    document: &Document,
    editing: &SketchEditing,
    feature: &Feature,
) -> Result<EditingCommand, String> {
    let name = &feature.name;
    if is_edited(editing, feature) {
        return Err(format!("{name} is already being edited"));
    }
    editable(document, feature)?;
    edit_command(feature, false)
        .ok_or_else(|| format!("{name} is an imported body and has no settings to edit"))
}

fn detach_change(model: &Model, feature: &Feature) -> Result<Transaction, String> {
    let name = &feature.name;
    if feature.kind.sketch().is_none() {
        return Err(format!("{name} is not a sketch"));
    }
    if feature.kind.attachment().is_none() {
        return Err(format!("{name} does not lie on a face or datum plane"));
    }
    sketch_placement::detach(model, feature.id())
        .ok_or_else(|| format!("{name} has no position yet; recompute the model first"))
}

fn place_change(
    model: &Model,
    selection: &Selection,
    feature: &Feature,
) -> Result<Transaction, String> {
    if feature.kind.sketch().is_none() {
        return Err(format!("{} is not a sketch", feature.name));
    }
    sketch_placement::place_on_selection(model, selection, feature.id()).map_err(str::to_owned)
}

fn axis_change(
    model: &Model,
    selection: &Selection,
    feature: &Feature,
) -> Result<Transaction, String> {
    match feature.kind.solid() {
        Some(SolidFeature::Revolve(revolve)) => {
            solid_panel::selected_axis_change(model, selection, feature.id(), revolve)
        }
        Some(SolidFeature::Extrude(_)) | None => Err(format!("{} is not a revolve", feature.name)),
    }
}

fn up_to_change(
    model: &Model,
    selection: &Selection,
    feature: &Feature,
) -> Result<Transaction, String> {
    match feature.kind.solid() {
        Some(SolidFeature::Extrude(extrude)) => {
            solid_panel::up_to_selected_change(model, selection, feature.id(), extrude)
        }
        Some(SolidFeature::Revolve(_)) | None => {
            Err(format!("{} is not an extrusion", feature.name))
        }
    }
}

fn datum_change(
    feature: &Feature,
    change: impl FnOnce(&Datum) -> Result<Transaction, String>,
) -> Result<Transaction, String> {
    feature
        .kind
        .datum()
        .ok_or_else(|| format!("{} is not a datum plane or axis", feature.name))
        .and_then(change)
}

fn pattern_change(
    model: &Model,
    selection: &Selection,
    feature: &Feature,
    reference: Reference,
) -> Result<Transaction, String> {
    let pattern = feature
        .kind
        .pattern()
        .ok_or_else(|| format!("{} is not a pattern", feature.name))?;
    pattern_tools::selected_change(model, selection, feature.id(), pattern, reference)
}

fn invoke_on<T>(
    commands: &mut CommandFrame<'_>,
    command: Command,
    current: Option<&Feature>,
    change: impl FnOnce(&Feature) -> Result<T, String>,
) -> Option<T> {
    let result = current.map_or_else(|| Err(NO_FEATURE_CHOSEN.to_owned()), change);
    let detail = current.map(|feature| feature.name.clone());
    if commands.invoke_detailed(command, detail, &result) {
        result.ok()
    } else {
        None
    }
}

fn feature_commands(
    context: &CommandContext<'_>,
    current: Option<&Feature>,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let CommandContext {
        model,
        selection,
        editing,
    } = *context;
    if let Some(command) = invoke_on(commands, Command::EditFeature, current, |feature| {
        edit_change(model.document(), editing, feature)
    }) {
        actions.push(Action::Editing(command));
    }
    let open = editing.solid().ok_or(NOTHING_OPEN);
    if commands.invoke(Command::CloseFeature, &open) {
        actions.push(Action::Editing(EditingCommand::CloseSolid));
    }
    if let Some(transaction) = invoke_on(
        commands,
        Command::ToggleVisibility,
        current,
        visibility::toggle,
    ) {
        actions.push(Action::Apply(transaction));
    }
    let document = model.document();
    let hide = visibility::hide_selection(document, selection, editing.feature());
    if commands.invoke(Command::HideSelection, &hide)
        && let Ok(transaction) = hide
    {
        actions.push(Action::Apply(transaction));
    }
    let show = visibility::show_all(document);
    if commands.invoke(Command::ShowAll, &show)
        && let Ok(transaction) = show
    {
        actions.push(Action::Apply(transaction));
    }
    if commands.invoke(Command::TogglePrincipal, &Ok::<_, String>(())) {
        actions.push(Action::Apply(visibility::toggle_principal_group(document)));
    }
    let changes: [(Command, FeatureChange<'_>); 8] = [
        (Command::DetachSketch, &|feature| {
            detach_change(model, feature)
        }),
        (Command::PlaceSketch, &|feature| {
            place_change(model, selection, feature)
        }),
        (Command::UseSelectedAxis, &|feature| {
            axis_change(model, selection, feature)
        }),
        (Command::ExtrudeUpToSelected, &|feature| {
            up_to_change(model, selection, feature)
        }),
        (Command::DatumUseSelected, &|feature| {
            datum_change(feature, |datum| {
                datum_panel::base_change(model, selection, feature.id(), datum)
            })
        }),
        (Command::DatumTurnAboutSelected, &|feature| {
            datum_change(feature, |datum| {
                datum_panel::rotation_change(model, selection, feature.id(), datum)
            })
        }),
        (Command::PatternUseSelected, &|feature| {
            pattern_change(model, selection, feature, Reference::First)
        }),
        (Command::PatternSecondUseSelected, &|feature| {
            pattern_change(model, selection, feature, Reference::Second)
        }),
    ];
    for (command, change) in changes {
        if let Some(transaction) = invoke_on(commands, command, current, change) {
            actions.push(Action::Apply(transaction));
        }
    }
}

pub fn commands(
    context: &CommandContext<'_>,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    let CommandContext { model, editing, .. } = *context;
    let document = model.document();
    let current = current_feature(document, editing, state);
    feature_commands(context, current, commands, actions);
    let chosen = current.ok_or(NO_FEATURE_CHOSEN);
    if commands.invoke(Command::RenameFeature, &chosen)
        && let Some(feature) = current
    {
        state.choose_only(feature.id());
        start_renaming(state, feature);
    }
    for direction in Direction::BOTH {
        let transaction = current.map_or_else(
            || Err(NO_FEATURE_CHOSEN.to_owned()),
            |feature| direction.transaction(document, feature),
        );
        if commands.invoke(direction.command(), &transaction)
            && let Ok(transaction) = transaction
        {
            actions.push(Action::Apply(transaction));
        }
    }
    let targets = targets(document, &state.chosen(), current);
    let detail = (!targets.is_empty()).then(|| described(&targets));
    let suppression = suppress_change(document, &targets);
    if commands.invoke_detailed(Command::SuppressFeature, detail.clone(), &suppression)
        && let Ok(transaction) = suppression
    {
        actions.push(Action::Apply(transaction));
    }
    let exportable = current
        .filter(|feature| feature.kind.sketch().is_some())
        .ok_or(sketch_export::NOT_A_SKETCH)
        .and_then(|feature| match model.displayed_sketch(feature) {
            Some(_) => Ok(feature),
            None => Err(sketch_export::NOT_SOLVED),
        });
    if commands.invoke_detailed(
        Command::ExportSketch,
        exportable.ok().map(|feature| feature.name.clone()),
        &exportable,
    ) && let Ok(feature) = exportable
    {
        actions.push(Action::File(FileCommand::ExportSketch(feature.id())));
    }
    if let Some(transaction) = invoke_on(commands, Command::RollToHere, current, |feature| {
        roll_to_here(document, feature)
    }) {
        actions.push(Action::Apply(transaction));
    }
    let rolls = [
        (Command::RollToEnd, roll_to_end(document)),
        (Command::RollbackUp, step_bar(document, true)),
        (Command::RollbackDown, step_bar(document, false)),
    ];
    for (command, roll) in rolls {
        if commands.invoke(command, &roll)
            && let Ok(transaction) = roll
        {
            actions.push(Action::Apply(transaction));
        }
    }
    let update = reference_update(model, &targets);
    if commands.invoke_detailed(Command::UpdateReferences, detail.clone(), &update)
        && let Ok(transaction) = update
    {
        actions.push(Action::Apply(transaction));
    }
    let delete = delete_request(document, &targets);
    if commands.invoke_detailed(Command::DeleteFeature, detail, &delete)
        && let Ok(deletion) = delete
    {
        deletion.perform(state, actions);
    }
    if editing.active().is_none() {
        let chosen = state.chosen();
        let selected = if chosen.is_empty() {
            Err(NOTHING_SELECTED.to_owned())
        } else {
            let features: Vec<&Feature> = document
                .features()
                .filter(|feature| chosen.contains(&feature.id()))
                .collect();
            delete_request(document, &features)
        };
        if commands.invoke(Command::DeleteSelection, &selected)
            && let Ok(deletion) = selected
        {
            deletion.perform(state, actions);
        }
    }
}

fn failure(
    ui: &mut Ui,
    document: &Document,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    error: &FeatureError,
) {
    widgets::callout(ui, Tone::Error, |ui| {
        ui.label(&error.reason);
        ui.label(widgets::muted(&error.remedy, ui));
        let Some(target) = error.fix else {
            return;
        };
        if let FixTarget::Unsuppress(id) = target {
            let label = format!("Unsuppress {}", feature_name(document, id));
            let button = widgets::small_button(ui, icons::UNSUPPRESS, &label);
            if ui.add(button).clicked() {
                actions.push(Action::Apply(unsuppress(document, id)));
            }
            return;
        }
        let (glyph, label) = match target {
            FixTarget::Dimension { .. } => (icons::EDIT, "Edit the dimension".to_owned()),
            _ => (icons::GO_TO, fix_label(document, target)),
        };
        let button = widgets::small_button(ui, glyph, &label);
        if ui.add(button).clicked() {
            state.request_focus(target.into());
        }
    });
}

fn fix_label(document: &Document, target: FixTarget) -> String {
    match target {
        FixTarget::Parameter(id) => {
            format!(
                "Go to {}",
                document.parameter_name(id).unwrap_or("the parameter")
            )
        }
        FixTarget::Dimension { .. } => "Edit the dimension".to_owned(),
        FixTarget::Feature(id) | FixTarget::Unsuppress(id) => {
            format!("Go to {}", feature_name(document, id))
        }
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
    }
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
                let detach = widgets::small_button(ui, icons::DETACH, DETACH_LABEL);
                let detach = ui.add(detach).on_hover_text(
                    "Keep the sketch where it is and stop following what it lies on",
                );
                if detach.clicked() {
                    actions.push(Action::Apply(transaction));
                }
            }
        });
    }
    let (label, hover) = match sketch_placement::placement_target(model.document(), selection) {
        Some(PlacementTarget::Plane(_)) => (
            PLACE_ON_PLANE_LABEL,
            "Move this sketch onto the selected plane; it follows the plane when the model changes",
        ),
        Some(PlacementTarget::Face(_)) => (
            PLACE_ON_FACE_LABEL,
            "Move this sketch onto the selected face; it follows the face when the model changes",
        ),
        None => return,
    };
    let button = widgets::small_button(ui, icons::USE_SELECTED, label);
    match sketch_placement::place_on_selection(model, selection, feature.id()) {
        Ok(transaction) => {
            if ui.add(button).on_hover_text(hover).clicked() {
                actions.push(Action::Apply(transaction));
            }
        }
        Err(reason) => {
            ui.add_enabled(false, button).on_disabled_hover_text(reason);
        }
    }
}

struct SketchCard<'a> {
    model: &'a Model,
    feature: &'a Feature,
    sketch: &'a Sketch,
    involved: Vec<ConstraintId>,
}

fn sketch_body(
    ui: &mut Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    feature: &Feature,
    sketch: &Sketch,
) {
    let summary = SketchSummary::of(model.evaluation(), feature.id());
    ui.horizontal_wrapped(|ui| {
        if let Some(focus) = sketch_status::show(ui, &summary) {
            state.request_focus(focus);
        }
    });
    ui.label(widgets::muted(
        count(sketch.entities().len(), "entity", "entities"),
        ui,
    ));
    let card = SketchCard {
        model,
        feature,
        sketch,
        involved: involved_constraints(model, feature),
    };
    let (dimensions, constraints): (Vec<_>, Vec<_>) = sketch
        .constraints()
        .partition(|(_, definition)| definition.dimension().is_some());
    if dimensions.is_empty() && constraints.is_empty() {
        ui.label(widgets::muted("No constraints yet.", ui));
        return;
    }
    let sections = [
        (DIMENSIONS_TITLE, dimensions),
        (CONSTRAINTS_TITLE, constraints),
    ];
    for (title, listed) in sections {
        if listed.is_empty() {
            continue;
        }
        let id = format!("{title}-{:?}", feature.id());
        if state.focus_inside(feature.id()) {
            widgets::reveal_section(ui.ctx(), &id);
        }
        widgets::section(ui, &id, title, Some(listed.len()), None, |ui| {
            for (constraint, definition) in listed {
                constraint_row(ui, &card, state, actions, constraint, definition);
            }
        });
    }
}

fn constraint_row(
    ui: &mut Ui,
    card: &SketchCard<'_>,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    constraint: ConstraintId,
    definition: &Constraint,
) {
    let SketchCard {
        model,
        feature,
        sketch,
        ..
    } = *card;
    let description = sketch.describe_constraint(constraint);
    let solution = sketch_status::up_to_date_solution(model.evaluation(), feature.id());
    let redundancy = solution.and_then(|solution| solution.redundancy(constraint));
    let text = if card.involved.contains(&constraint) {
        RichText::new(&description).color(ui.visuals().error_fg_color)
    } else if redundancy.is_some() {
        RichText::new(&description).color(ui.visuals().warn_fg_color)
    } else {
        RichText::new(&description)
    };
    let (row, deleted) = Sides::new().shrink_left().truncate().show(
        ui,
        |ui| widgets::link_label(ui, text),
        |ui| widgets::icon_button(ui, icons::DELETE, "Delete this constraint").clicked(),
    );
    let pickable = Pickable::SketchConstraint {
        feature: feature.id(),
        constraint,
    };
    if row.hovered() {
        state.hovered_in_tree = Some(pickable);
    }
    if row.clicked() {
        actions.push(Action::Editing(EditingCommand::Enter(feature.id())));
        state.chosen_in_tree = Some(pickable);
    }
    let mut deleted = deleted;
    row.context_menu(|ui| {
        if widgets::menu_item(ui, icons::DELETE, "Delete", None).clicked() {
            deleted = true;
            ui.close();
        }
    });
    if deleted {
        actions.push(Action::Apply(sketch_tools::remove_items(
            model,
            feature.id(),
            format!("Delete {description}"),
            Vec::new(),
            vec![constraint],
        )));
    }
    if let Some(redundancy) = redundancy {
        widgets::callout(ui, Tone::Warning, |ui| {
            ui.label(redundancy_text(sketch, redundancy));
        });
    }
    let label = row.id;
    reveal_if_focused(
        state,
        row,
        Focus::Constraint {
            feature: feature.id(),
            constraint,
        },
    );
    if let Some(expression) = definition.dimension() {
        dimension_field(ui, card, state, actions, constraint, expression, label);
    }
}

fn dimension_field(
    ui: &mut Ui,
    card: &SketchCard<'_>,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    constraint: ConstraintId,
    expression: &Expression,
    label: Id,
) {
    let SketchCard { model, feature, .. } = *card;
    let document = model.document();
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
        ui.add_space(SPACE_L);
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
                    model.units(),
                )
            },
        );
        field.response.clone().labelled_by(label);
        state.focus_reached(focus, field.response.has_focus());
        if let Some(transaction) = field.committed {
            actions.push(Action::Apply(transaction));
        }
        if field.error.is_none()
            && let Some(preview) =
                field::value_preview(model.parameters(), expression, model.units())
        {
            ui.label(widgets::muted(preview, ui));
        }
        error = field.error;
    });
    if let Some(error) = error {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_L);
            field_error(ui, &error);
        });
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
        Some(
            FeatureState::UpToDate
            | FeatureState::Outdated
            | FeatureState::Suppressed
            | FeatureState::RolledBack,
        )
        | None => Vec::new(),
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
