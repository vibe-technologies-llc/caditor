use caditor_expression::{Dimension, Expression};
use caditor_geometry::Point3;
use caditor_render::{CutFace, MAX_SECTION_PLANES};
use egui::{Id, Label, ScrollArea, Sides, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    feature_fields, field,
    guide::Page,
    guide_panel, icons,
    layout::RightPanel,
    model::Model,
    section::{self, Cut, SectionTool},
    selection::{Pickable, PrincipalPlane, Selection},
    widgets::{self, FIELD_WIDTH, Tone},
};

pub const TITLE: &str = "Section view";
pub const CLOSE: &str = "Close the section view";
pub const ABOUT: &str = "Each plane cuts away the bodies on the side it faces, so you see inside \
                         them, with the cut faces hatched or filled. Several planes together \
                         keep only what every plane keeps. Picking and Measure reach only what \
                         is shown. It never changes the model.";
pub const NO_PLANES: &str = "There is no section plane, so nothing is cut away. Add a plane to \
                             look inside the bodies.";
pub const ADD: &str = "Add a plane";
pub const CURRENT: &str = "Current";
pub const THROUGH: &str = "Through";
pub const PLANE: &str = "Plane";
pub const USE_SELECTED_HOVER: &str = "Put the plane on the selected plane, flat face or sketch";
pub const OFFSET: &str = "Offset";
pub const OFFSET_HOVER: &str = "How far the plane moves into the side it keeps";
pub const TILT: &str = "Tilt";
pub const TILT_HOVER: &str = "Turns the plane about its own x axis";
pub const TURN: &str = "Turn";
pub const TURN_HOVER: &str = "Turns the plane about its own y axis";
pub const FLIP: &str = "Cut away the other side";
pub const CUT_FACES: &str = "Cut faces";
const PANEL: RightPanel = RightPanel {
    id: "section",
    width: 300.0,
    least: 220.0,
};
const PRINCIPAL: [(PrincipalPlane, &str); 3] = [
    (PrincipalPlane::Xy, "XY"),
    (PrincipalPlane::Xz, "XZ"),
    (PrincipalPlane::Yz, "YZ"),
];
const CUT_FACE_STYLES: [(CutFace, &str, &str); 2] = [
    (CutFace::Hatched, "Hatched", "Draw the cut faces hatched"),
    (CutFace::Filled, "Filled", "Draw the cut faces filled"),
];

pub struct SectionContext<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub centre: Option<Point3>,
}

struct Value<'a> {
    caption: &'a str,
    hover: &'a str,
    id: Id,
    dimension: Dimension,
}

fn value_row(ui: &mut Ui, model: &Model, value: &Value<'_>, expression: &mut Expression) -> bool {
    widgets::caption(ui, value.caption);
    let document = model.document();
    let parameters = model.parameters();
    let units = model.units();
    let field = field::commit_field(
        ui,
        value.id,
        &document.expression_text(expression),
        FIELD_WIDTH,
        false,
        |text| {
            let parsed = field::parse_expression(
                document,
                parameters,
                text,
                field::Expected {
                    dimension: Some(value.dimension),
                    non_negative: false,
                },
                units,
            )?;
            parameters
                .evaluate_expression(&parsed)
                .map_err(|error| field::sentence(&error.to_string()))?;
            Ok(parsed)
        },
    );
    field.response.on_hover_text(value.hover);
    let committed = field.committed.map(|parsed| *expression = parsed);
    ui.end_row();
    if let Some(error) = field.error {
        widgets::error_row(ui, &error);
    }
    committed.is_some()
}

fn base_rows(ui: &mut Ui, context: &SectionContext<'_>, cut: &mut Cut) -> bool {
    let mut changed = false;
    widgets::caption(ui, THROUGH);
    let choices: Vec<(&str, &str)> = PRINCIPAL
        .iter()
        .map(|(plane, letters)| (*letters, plane.name()))
        .collect();
    let chosen = PRINCIPAL
        .iter()
        .position(|(plane, _)| cut.base == Pickable::Plane(*plane));
    if let Some(index) = widgets::segmented(ui, &choices, chosen.unwrap_or(usize::MAX))
        && let Some((plane, _)) = PRINCIPAL.get(index)
    {
        cut.base = Pickable::Plane(*plane);
        changed = true;
    }
    ui.end_row();

    widgets::caption(ui, PLANE);
    ui.add(Label::new(cut.base_name(context.model)).wrap_mode(TextWrapMode::Wrap));
    ui.end_row();

    ui.label("");
    let selected = section::base_from(context.model, context.selection);
    let button = widgets::small_button(ui, icons::USE_SELECTED, feature_fields::USE_SELECTED);
    let hover = match &selected {
        Ok(_) => USE_SELECTED_HOVER.to_owned(),
        Err(refusal) => refusal.to_string(),
    };
    if ui
        .add_enabled(selected.is_ok(), button)
        .on_hover_text(hover)
        .clicked()
        && let Ok(base) = selected
    {
        cut.base = base;
        changed = true;
    }
    ui.end_row();
    changed
}

fn cut_rows(ui: &mut Ui, context: &SectionContext<'_>, index: usize, cut: &mut Cut) -> bool {
    let model = context.model;
    let mut changed = base_rows(ui, context, cut);
    for (caption, hover, slot, dimension, expression) in [
        (
            OFFSET,
            OFFSET_HOVER,
            "offset",
            Dimension::LENGTH,
            &mut cut.offset,
        ),
        (TILT, TILT_HOVER, "tilt", Dimension::ANGLE, &mut cut.tilt),
        (TURN, TURN_HOVER, "turn", Dimension::ANGLE, &mut cut.turn),
    ] {
        let value = Value {
            caption,
            hover,
            id: Id::new(("section", index, slot)),
            dimension,
        };
        changed |= value_row(ui, model, &value, expression);
    }
    if let Some(flipped) = feature_fields::reverse_row(ui, FLIP, cut.flipped) {
        cut.flipped = flipped;
        changed = true;
    }
    widgets::caption(ui, CUT_FACES);
    let choices: Vec<(&str, &str)> = CUT_FACE_STYLES
        .iter()
        .map(|(_, title, meaning)| (*title, *meaning))
        .collect();
    let chosen = CUT_FACE_STYLES
        .iter()
        .position(|(style, _, _)| *style == cut.cut_face)
        .unwrap_or(0);
    if let Some(chosen) = widgets::segmented(ui, &choices, chosen)
        && let Some((style, _, _)) = CUT_FACE_STYLES.get(chosen)
    {
        cut.cut_face = *style;
        changed = true;
    }
    ui.end_row();
    changed
}

enum CardAction {
    None,
    Changed,
    Remove,
}

fn cut_card(
    ui: &mut Ui,
    context: &SectionContext<'_>,
    (index, current): (usize, bool),
    cut: &mut Cut,
) -> CardAction {
    widgets::card(ui, |ui| {
        let remove = Sides::new()
            .show(
                ui,
                |ui| {
                    ui.add(
                        Label::new(widgets::section_title(&format!("Plane {}", index + 1)))
                            .selectable(false),
                    );
                    if current {
                        widgets::pill(ui, Tone::Info, CURRENT);
                    }
                },
                |ui| widgets::icon_button(ui, icons::DELETE, "Remove this section plane").clicked(),
            )
            .1;
        if remove {
            return CardAction::Remove;
        }
        let changed = widgets::properties(ui, ("section-plane", index), |ui| {
            cut_rows(ui, context, index, cut)
        });
        if let Err(problem) = cut.plane(context.model) {
            widgets::callout(ui, Tone::Warning, |ui| ui.label(problem.to_string()));
        }
        if changed {
            CardAction::Changed
        } else {
            CardAction::None
        }
    })
}

fn body(ui: &mut Ui, context: &SectionContext<'_>, tool: &mut SectionTool) {
    ui.label(widgets::muted(ABOUT, ui));
    ui.add_space(SPACE_S);
    if tool.cuts.is_empty() {
        widgets::callout(ui, Tone::Info, |ui| ui.label(NO_PLANES));
        ui.add_space(SPACE_S);
    }
    let mut removed = None;
    for (index, cut) in tool.cuts.iter_mut().enumerate() {
        match cut_card(ui, context, (index, index == tool.current), cut) {
            CardAction::None => {}
            CardAction::Changed => tool.current = index,
            CardAction::Remove => removed = Some(index),
        }
        ui.add_space(SPACE_S);
    }
    if let Some(index) = removed {
        tool.remove(index);
    }
    ui.add_space(SPACE_M);
    let addable = tool.can_add();
    let hover = match &addable {
        Ok(()) => format!("Add another plane, up to {MAX_SECTION_PLANES}"),
        Err(refusal) => refusal.to_string(),
    };
    let button = widgets::small_button(ui, icons::ADD_SECTION_PLANE, ADD);
    if ui
        .add_enabled(addable.is_ok(), button)
        .on_hover_text(hover)
        .clicked()
    {
        tool.add(context.model, context.centre);
    }
}

pub fn show(ui: &mut Ui, context: &SectionContext<'_>, tool: &mut SectionTool, room: f32) {
    let mut close = false;
    PANEL.panel(ui.ctx(), room).show(ui, |ui| {
        ui.add_space(SPACE_S);
        widgets::panel_header(ui, icons::SECTION, TITLE, |ui| {
            close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
            guide_panel::help_button(ui, Page::SectionView);
        });
        ui.add_space(SPACE_S);
        ScrollArea::vertical().show(ui, |ui| body(ui, context, tool));
    });
    if close {
        tool.open = false;
    }
}
