use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_sketch::{Constraint, Entity, EntityId, Fix, Flaw, Kept, RelationKind, Sketch};
use egui::{Label, ScrollArea, Sense, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    commands::{Command, CommandFrame},
    editing::SketchEditing,
    guide::Page,
    guide_panel, icons,
    layout::RightPanel,
    model::{Action, Model},
    selection::{Pickable, Selection},
    sketch_tools,
    tidying::{self, Found, Looseness, Progress, Proposal, Task, Tidying},
    units::Units,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Constrain automatically";
pub const CLOSE: &str = "Close Constrain automatically";
pub const WORKING: &str = "Looking through the sketch…";
pub const NO_RELATIONS: &str = "The drawing shows no relation the sketch does not already hold.";
pub const NO_DIMENSIONS: &str = "Nothing is left free for a dimension from the datum to hold.";
pub const NO_FLAWS: &str = "No flaws found: every end is joined or clearly apart, no curve lies \
                            on another and every curve has a length.";
pub const TOLERANCE: &str = "Tolerance";
pub const DATUM: &str = "Datum";
pub const USE_SELECTED_POINT: &str = "Use the selected point";
pub const USE_ORIGIN: &str = "Use the origin";
pub const NOT_EDITING: &str = "Edit a sketch first";
const SELECT_ONE_POINT: &str = "Select one point of the sketch to measure from it";
const PANEL: RightPanel = RightPanel {
    id: "constrain-automatically",
    width: 320.0,
    least: 240.0,
};
const MAX_LISTED: usize = 40;

#[derive(Default)]
pub struct Shown {
    pub actions: Vec<Action>,
    pub previewed: Vec<Pickable>,
}

pub fn commands(
    editing: &SketchEditing,
    selection: &Selection,
    tidying: &mut Tidying,
    commands: &mut CommandFrame<'_>,
) {
    let edited = editing.active().map(|active| active.feature);
    let availability = edited.map(|_| ()).ok_or_else(|| NOT_EDITING.to_owned());
    for (command, task) in [
        (Command::FindRelations, Task::Relations),
        (Command::DimensionFromDatum, Task::Dimensions),
        (Command::CheckSketch, Task::Check),
    ] {
        if commands.invoke(command, &availability)
            && let Some(feature) = edited
        {
            tidying.open_at(task, feature);
            if task == Task::Dimensions
                && let Some(point) = selected_point(selection, feature)
            {
                tidying.datum = point;
            }
        }
    }
}

fn selected_point(selection: &Selection, feature: FeatureId) -> Option<EntityId> {
    match sketch_tools::selected_entities(selection, feature).as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    tidying: &mut Tidying,
    selection: &Selection,
    room: f32,
) -> Shown {
    let mut shown = Shown::default();
    let Some(feature) = tidying.feature() else {
        return shown;
    };
    let mut close = false;
    let mut accepted = false;
    PANEL.show(ui, room, |ui| {
        ui.add_space(SPACE_S);
        widgets::panel_header(ui, icons::AUTOMATIC_CONSTRAINTS, TITLE, |ui| {
            close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
            guide_panel::help_button(ui, Page::AutomaticConstraints);
        });
        ui.add_space(SPACE_S);
        let choices: Vec<(&str, &str)> = Task::ALL
            .iter()
            .map(|task| (task.label(), task.about()))
            .collect();
        let chosen = Task::ALL
            .iter()
            .position(|task| *task == tidying.task)
            .unwrap_or_default();
        if let Some(index) = widgets::segmented(ui, &choices, chosen)
            && let Some(task) = Task::ALL.get(index)
        {
            tidying.task = *task;
        }
        ui.add_space(SPACE_S);
        let primary = match tidying.progress() {
            Progress::Found(found) => primary_label(found, tidying.task),
            _ => None,
        };
        if let Some(label) = primary {
            egui::Panel::bottom("constrain-automatically-footer")
                .show_separator_line(false)
                .show(ui, |ui| {
                    accepted = footer_button(ui, &label);
                });
        }
        ScrollArea::vertical().show(ui, |ui| {
            wrapped(ui, widgets::muted(tidying.task.about(), ui));
            ui.add_space(SPACE_S);
            settings(ui, model, tidying, selection, feature);
            ui.add_space(SPACE_M);
            let task = tidying.task;
            match tidying.progress() {
                Progress::Closed => {}
                Progress::NotSettled => {
                    wrapped(ui, widgets::muted(tidying::NOT_SETTLED, ui));
                }
                Progress::Working => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(WORKING);
                    });
                }
                Progress::Failed(reason) => {
                    widgets::callout(ui, Tone::Error, |ui| wrapped(ui, reason));
                }
                Progress::Found(found) => {
                    found_proposal(ui, model, feature, task, found, &mut shown);
                }
            }
        });
    });
    if accepted && let Progress::Found(found) = tidying.progress() {
        shown
            .actions
            .extend(accept(model, feature, tidying.task, tidying.datum, found));
    }
    if close {
        tidying.close();
    }
    shown
}

fn primary_label(found: &Found, task: Task) -> Option<String> {
    match (&found.proposal, task) {
        (Proposal::Relations(kept), Task::Relations) if !kept.constraints.is_empty() => {
            Some(format!(
                "Add {}",
                tidying::count(kept.constraints.len(), "relation", "relations")
            ))
        }
        (Proposal::Dimensions(kept), Task::Dimensions) if !kept.constraints.is_empty() => {
            Some(format!(
                "Add {}",
                tidying::count(kept.constraints.len(), "dimension", "dimensions")
            ))
        }
        (Proposal::Flaws(flaws), Task::Check) => {
            let fixable = flaws.iter().filter(|(_, fix)| !fix.is_empty()).count();
            (fixable > 0).then(|| format!("Fix {}", tidying::count(fixable, "flaw", "flaws")))
        }
        _ => None,
    }
}

fn accept(
    model: &Model,
    feature: FeatureId,
    task: Task,
    datum: EntityId,
    found: &Found,
) -> Option<Action> {
    let transaction = match (&found.proposal, task) {
        (Proposal::Relations(kept), Task::Relations) => {
            tidying::relations_transaction(model, feature, &kept.constraints)
        }
        (Proposal::Dimensions(kept), Task::Dimensions) => tidying::dimensions_transaction(
            model,
            feature,
            &datum_name(&found.sketch, datum),
            &kept.constraints,
            model.units(),
        ),
        (Proposal::Flaws(flaws), Task::Check) => {
            let fixes: Vec<&Fix> = flaws
                .iter()
                .map(|(_, fix)| fix)
                .filter(|fix| !fix.is_empty())
                .collect();
            let label = primary_label(found, task)?;
            tidying::fixes_transaction(model, feature, label, &fixes)
        }
        _ => return None,
    };
    Some(Action::Apply(transaction))
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(Label::new(text).wrap_mode(TextWrapMode::Wrap));
}

fn settings(
    ui: &mut Ui,
    model: &Model,
    tidying: &mut Tidying,
    selection: &Selection,
    feature: FeatureId,
) {
    widgets::properties(ui, "constrain-automatically-settings", |ui| {
        if tidying.task == Task::Dimensions {
            widgets::caption(ui, DATUM);
            ui.horizontal_wrapped(|ui| {
                let name = model.settled_sketch(feature).map_or_else(
                    || "Origin".to_owned(),
                    |sketch| sketch.entity_label(tidying.datum),
                );
                ui.add(Label::new(name).wrap_mode(TextWrapMode::Wrap));
                let point = selected_point(selection, feature).filter(|point| {
                    model
                        .settled_sketch(feature)
                        .is_some_and(|sketch| sketch.point(*point).is_some())
                });
                let button = widgets::small_button(ui, icons::USE_SELECTED, USE_SELECTED_POINT);
                let response = ui.add_enabled(point.is_some(), button);
                let response = if point.is_some() {
                    response
                } else {
                    response.on_disabled_hover_text(SELECT_ONE_POINT)
                };
                if response.clicked()
                    && let Some(point) = point
                {
                    tidying.datum = point;
                }
                if tidying.datum != EntityId::ORIGIN {
                    let origin = widgets::small_button(
                        ui,
                        icons::command(Command::DimensionFromDatum),
                        USE_ORIGIN,
                    );
                    if ui.add(origin).clicked() {
                        tidying.datum = EntityId::ORIGIN;
                    }
                }
            });
            ui.end_row();
            return;
        }
        widgets::caption(ui, TOLERANCE);
        let choices: Vec<(&str, &str)> = Looseness::ALL
            .iter()
            .map(|looseness| (looseness.label(), looseness_help(*looseness)))
            .collect();
        let chosen = Looseness::ALL
            .iter()
            .position(|looseness| *looseness == tidying.looseness)
            .unwrap_or_default();
        if let Some(index) = widgets::segmented(ui, &choices, chosen)
            && let Some(looseness) = Looseness::ALL.get(index)
        {
            tidying.looseness = *looseness;
        }
        ui.end_row();
    });
    if let Some(sketch) = model.settled_sketch(feature)
        && tidying.task != Task::Dimensions
    {
        let tolerance = tidying.looseness.tolerance(sketch);
        wrapped(
            ui,
            widgets::muted(
                format!(
                    "Within {} and {:.2}°",
                    model.units().length.measured_length(tolerance.distance),
                    tolerance.angle.to_degrees()
                ),
                ui,
            ),
        );
    }
    if tidying.task == Task::Relations {
        ui.add_space(SPACE_S);
        for kind in RelationKind::ALL {
            let mut chosen = tidying.kinds.contains(&kind);
            if ui.checkbox(&mut chosen, kind.label()).changed() {
                if chosen {
                    tidying.kinds.insert(kind);
                } else {
                    tidying.kinds.remove(&kind);
                }
            }
        }
    }
}

fn looseness_help(looseness: Looseness) -> &'static str {
    match looseness {
        Looseness::Tight => "Only what is drawn almost exactly, as in an imported drawing",
        Looseness::Normal => "What is drawn with care",
        Looseness::Loose => "What is sketched roughly by hand",
    }
}

fn found_proposal(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    task: Task,
    found: &Found,
    shown: &mut Shown,
) {
    let units = model.units();
    match (&found.proposal, task) {
        (Proposal::Relations(kept), Task::Relations) => {
            if kept.constraints.is_empty() {
                widgets::callout(ui, Tone::Success, |ui| wrapped(ui, NO_RELATIONS));
                return;
            }
            constraints_list(ui, feature, &found.sketch, kept, units, true, shown);
        }
        (Proposal::Dimensions(kept), Task::Dimensions) => {
            if kept.constraints.is_empty() {
                widgets::callout(ui, Tone::Success, |ui| wrapped(ui, NO_DIMENSIONS));
                return;
            }
            constraints_list(ui, feature, &found.sketch, kept, units, false, shown);
        }
        (Proposal::Flaws(flaws), Task::Check) => {
            if flaws.is_empty() {
                widgets::callout(ui, Tone::Success, |ui| wrapped(ui, NO_FLAWS));
                return;
            }
            flaws_list(ui, model, feature, &found.sketch, flaws, shown);
        }
        _ => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(WORKING);
            });
        }
    }
}

fn datum_name(sketch: &Sketch, datum: EntityId) -> String {
    if datum == EntityId::ORIGIN {
        "the origin".to_owned()
    } else {
        sketch.entity_label(datum)
    }
}

fn footer_button(ui: &mut Ui, label: &str) -> bool {
    widgets::footer(ui, |ui| {
        ui.add(widgets::primary_button(ui, label)).clicked()
    })
}

fn constraints_list(
    ui: &mut Ui,
    feature: FeatureId,
    sketch: &Sketch,
    kept: &Kept,
    units: Units,
    relations: bool,
    shown: &mut Shown,
) {
    let freedom = match kept.degrees_of_freedom {
        0 => "Adding them leaves the sketch fully constrained.".to_owned(),
        left => format!(
            "Adding them leaves {}.",
            tidying::count(left, "degree of freedom", "degrees of freedom")
        ),
    };
    wrapped(ui, freedom);
    ui.add_space(SPACE_S);
    shown.previewed = pickables(
        feature,
        kept.constraints.iter().flat_map(Constraint::entities),
    );
    let mut groups: Vec<(&str, Vec<&Constraint>)> = Vec::new();
    for constraint in &kept.constraints {
        let title = match RelationKind::of(constraint) {
            Some(kind) if relations => kind.label(),
            Some(_) | None => "Dimensions",
        };
        match groups.iter_mut().find(|(known, _)| *known == title) {
            Some((_, members)) => members.push(constraint),
            None => groups.push((title, vec![constraint])),
        }
    }
    for (title, members) in groups {
        ui.label(widgets::section_title(&format!(
            "{title} ({})",
            members.len()
        )));
        ui.add_space(SPACE_S);
        widgets::card(ui, |ui| {
            for constraint in members.iter().take(MAX_LISTED) {
                let mut text = sketch.describe(constraint);
                if constraint.dimension().is_some()
                    && let Some(value) = sketch.measured(constraint)
                {
                    text = format!("{text}: {}", units.length.measured_length(value));
                }
                let row = ui.add(
                    Label::new(text)
                        .wrap_mode(TextWrapMode::Wrap)
                        .sense(Sense::hover()),
                );
                if row.hovered() {
                    shown.previewed = pickables(feature, constraint.entities());
                }
            }
            if members.len() > MAX_LISTED {
                ui.label(widgets::muted(
                    format!("and {} more", members.len() - MAX_LISTED),
                    ui,
                ));
            }
        });
        ui.add_space(SPACE_S);
    }
}

fn flaws_list(
    ui: &mut Ui,
    model: &Model,
    feature: FeatureId,
    sketch: &Sketch,
    flaws: &[(Flaw, Fix)],
    shown: &mut Shown,
) {
    let units = model.units();
    shown.previewed = pickables(feature, flaws.iter().flat_map(|(flaw, _)| subjects(flaw)));
    let mut fixed: Option<(String, Vec<&Fix>)> = None;
    widgets::card(ui, |ui| {
        for (flaw, fix) in flaws.iter().take(MAX_LISTED) {
            let (problem, remedy) = words(sketch, flaw, units);
            let row = ui.add(
                Label::new(problem)
                    .wrap_mode(TextWrapMode::Wrap)
                    .sense(Sense::hover()),
            );
            if row.hovered() {
                shown.previewed = pickables(feature, subjects(flaw));
            }
            ui.horizontal_wrapped(|ui| {
                if fix.is_empty() {
                    ui.label(widgets::muted(remedy, ui));
                } else {
                    let button = widgets::small_button(ui, icons::FIX, &remedy);
                    if ui.add(button).clicked() {
                        fixed = Some((remedy.clone(), vec![fix]));
                    }
                }
            });
            ui.add_space(SPACE_S);
        }
        if flaws.len() > MAX_LISTED {
            ui.label(widgets::muted(
                format!("and {} more", flaws.len() - MAX_LISTED),
                ui,
            ));
        }
    });
    if let Some((label, fixes)) = fixed {
        shown.actions.push(Action::Apply(tidying::fixes_transaction(
            model, feature, label, &fixes,
        )));
    }
}

fn subjects(flaw: &Flaw) -> Vec<EntityId> {
    match *flaw {
        Flaw::NearlyJoined { first, second, .. } => vec![first, second],
        Flaw::LiesOn { curve, on } => vec![curve, on],
        Flaw::Overlaps { curve, other } => vec![curve, other],
        Flaw::NoLength { curve } => vec![curve],
    }
}

fn owner(sketch: &Sketch, point: EntityId) -> String {
    sketch
        .entities_using(point)
        .into_iter()
        .find(|user| !matches!(sketch.entity(*user), Some(Entity::Point(_)) | None))
        .map_or_else(
            || sketch.entity_label(point),
            |curve| sketch.entity_label(curve),
        )
}

pub fn words(sketch: &Sketch, flaw: &Flaw, units: Units) -> (String, String) {
    let label = |entity: EntityId| sketch.entity_label(entity);
    match *flaw {
        Flaw::NearlyJoined { first, second, gap } => (
            format!(
                "Ends of {} and {} are {} apart but not joined",
                owner(sketch, first),
                owner(sketch, second),
                units.length.measured_length(gap)
            ),
            "Join them".to_owned(),
        ),
        Flaw::LiesOn { curve, on } => (
            format!("{} lies on {}", label(curve), label(on)),
            format!("Delete {}", label(curve)),
        ),
        Flaw::Overlaps { curve, other } => (
            format!(
                "{} overlaps {} along part of it",
                label(curve),
                label(other)
            ),
            "Trim one of them where the other begins".to_owned(),
        ),
        Flaw::NoLength { curve } => (
            format!("{} has no length", label(curve)),
            format!("Delete {}", label(curve)),
        ),
    }
}

fn pickables(feature: FeatureId, entities: impl IntoIterator<Item = EntityId>) -> Vec<Pickable> {
    let unique: BTreeSet<EntityId> = entities
        .into_iter()
        .filter(|entity| !entity.is_reference())
        .collect();
    unique
        .into_iter()
        .map(|entity| Pickable::SketchEntity { feature, entity })
        .collect()
}
