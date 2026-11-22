use std::collections::BTreeMap;

use caditor_document::FeatureId;
use caditor_sketch::{EntityId, Heading, PatternCopy, PatternRow, Sides, Sketch};
use egui::{Id, Label, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    editing::{EditingCommand, Tool},
    feature_tree::count,
    field::{self, LiveValue},
    guide::Page,
    icons,
    layout::RightPanel,
    model::{Action, Model, Notice},
    patterning::{
        ADD_DIRECTION, ADD_DIRECTION_ABOUT, DISTRIBUTIONS, Direction, PatternEvaluation,
        PatternField, PatternKind, PatternSettings, Patterning, RUNS_ALONG, SIDES, SIDES_CAPTION,
        Spec, TurnKind, copy_about, copy_name,
    },
    trimming::capitalized,
    widgets::{self, PanelTitle, Tone},
};

pub const SHAPE: &str = "Shape";
pub const DISTRIBUTION: &str = "Distribution";
pub const SPREAD: &str = "Spread";
pub const BY_ANGLE: &str = "The angle below";
pub const CHOOSING: &str = "Choosing: click a line or axis in the view";
pub const STOP_CHOOSING: &str = "Stop choosing";
pub const CENTRE: &str = "Centre";
pub const REPEATS: &str = "Repeats";
pub const NO_CENTRE: &str = "None chosen";
pub const CENTRE_HINT: &str = "Click a point in the sketch to repeat about.";
pub const USE_ORIGIN: &str = "Use the origin";
pub const NOTHING_REPEATED: &str =
    "Nothing yet: click items in the sketch or drag a box around them.";
pub const MAKE: &str = "Make the copies";
pub const COPIES: &str = "Copies";
pub const BRING_BACK: &str = "Bring the copies back";
const BRING_BACK_ABOUT: &str = "Make every copy left out again";
pub const COPIES_HINT: &str =
    "Click a copy here or in the view to leave it out, and again to make it.";
pub const RECTANGULAR_ABOUT: &str = "Repeat the selection along one direction, or along several \
                                     for a grid, every combination of steps a copy. Every value \
                                     takes units and expressions using your parameters; the copies \
                                     keep following them.";
pub const CIRCULAR_ABOUT: &str = "Repeat the selection about a point. Every value takes units \
                                  and expressions using your parameters; the copies keep \
                                  following them.";
const PANEL: RightPanel = RightPanel {
    id: "sketch-pattern",
};

pub fn field_id(field: PatternField) -> Id {
    Id::unique((
        "sketch-pattern-field",
        field.key(),
        field.direction().map(Direction::index),
    ))
}

pub fn close_label(kind: PatternKind) -> String {
    format!("Close {}", kind.tool().label())
}

pub struct Context<'a> {
    pub model: &'a Model,
    pub feature: FeatureId,
    pub sketch: Option<&'a Sketch>,
}

pub fn show(
    ui: &mut Ui,
    context: &Context<'_>,
    (settings, patterning): (&mut PatternSettings, &mut Patterning),
    room: f32,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let kind = patterning.kind();
    let lines = patterning.directions().to_vec();

    let evaluation = settings.evaluate(context.model, kind, &lines);
    let fields = settings.shown_fields(kind, &lines);
    let centre = patterning.chosen_centre();

    let ready = evaluation.spec.is_ok()
        && patterning.repeated() > 0
        && (kind == PatternKind::Rectangular || centre.is_some())
        && patterning.problem().is_none();
    let close = close_label(kind);
    let header = PanelTitle {
        glyph: icons::tool(kind.tool()),
        title: kind.tool().label(),
        close: &close,
        help: Some(Page::SketchPatterns),
    };
    let mut chosen = Chosen::default();

    let panel = widgets::side_panel_parts(
        ui,
        room,
        PANEL,
        &header,
        |_| {},
        |ui| {
            let pressed = widgets::panel_footer(ui, PANEL.id, |ui| {
                widgets::footer(ui, |ui| {
                    ui.add_enabled(ready, widgets::primary_button(ui, MAKE))
                        .clicked()
                })
            });

            widgets::panel_scroll(ui, PANEL.id, |ui| {
                let about = match kind {
                    PatternKind::Rectangular => RECTANGULAR_ABOUT,
                    PatternKind::Circular => CIRCULAR_ABOUT,
                };

                wrapped(ui, widgets::muted(about, ui));
                ui.add_space(SPACE_S);
                widgets::properties(ui, "sketch-pattern-values", |ui| {
                    chosen.kind = shapes(ui, kind);
                    match kind {
                        PatternKind::Rectangular => {
                            distribution_row(ui, settings);

                            let shown = Shown {
                                model: context.model,
                                sketch: context.sketch,
                                fields: &fields,
                                evaluation: &evaluation,
                            };
                            let mut removed = None;

                            for direction in settings.directions() {
                                if direction_rows(ui, &shown, (settings, patterning), direction) {
                                    removed = Some(direction);
                                }
                            }
                            if let Some(direction) = removed {
                                settings.remove_direction(direction);
                                patterning.remove_direction(direction);
                            }

                            let add = widgets::property(ui, "", |ui| {
                                ui.add(widgets::button(ADD_DIRECTION))
                                    .on_hover_text(ADD_DIRECTION_ABOUT)
                                    .clicked()
                            });
                            if add {
                                settings.add_direction();
                                patterning.direction_added();
                            }
                        }
                        PatternKind::Circular => {
                            chosen.origin = centre_row(ui, context.sketch, centre);
                            turn_row(ui, settings);

                            let shown = Shown {
                                model: context.model,
                                sketch: context.sketch,
                                fields: &fields,
                                evaluation: &evaluation,
                            };

                            field_rows(ui, &shown, settings, &PatternField::CIRCULAR);
                        }
                    }
                    if let Ok(spec) = &evaluation.spec {
                        copies_row(ui, spec, patterning);
                    }
                    widgets::property(ui, REPEATS, |ui| match patterning.repeated() {
                        0 => wrapped(ui, widgets::muted(NOTHING_REPEATED, ui)),
                        items => wrapped(ui, count(items, "item", "items")),
                    });
                });
                ui.add_space(SPACE_M);
                outcome(ui, (kind, settings.turn()), &evaluation, patterning);
            });
            pressed
        },
    );

    if panel.closed {
        actions.push(Action::Editing(EditingCommand::SetTool(Tool::Select)));
    }
    if let Some(other) = chosen.kind.filter(|other| *other != kind) {
        actions.push(Action::Editing(EditingCommand::SetTool(other.tool())));
    }
    if chosen.origin {
        patterning.choose_centre(EntityId::ORIGIN);
    }
    if panel.inner {
        actions.push(match patterning.make(context.model, context.feature) {
            Ok(transaction) => Action::Apply(transaction),
            Err(reason) => Action::Inform(Notice::warning(reason.to_string())),
        });
    }
    actions
}

#[derive(Default)]
struct Chosen {
    kind: Option<PatternKind>,
    origin: bool,
}

struct Shown<'a> {
    model: &'a Model,
    sketch: Option<&'a Sketch>,
    fields: &'a [PatternField],
    evaluation: &'a PatternEvaluation,
}

fn shapes(ui: &mut Ui, kind: PatternKind) -> Option<PatternKind> {
    widgets::property(ui, SHAPE, |ui| {
        let choices = PatternKind::ALL.map(|kind| (kind.label(), kind.about()));
        let current = PatternKind::ALL
            .iter()
            .position(|shown| *shown == kind)
            .unwrap_or(usize::MAX);
        widgets::segmented(ui, &choices, current)
            .and_then(|index| PatternKind::ALL.get(index).copied())
    })
}

fn distribution_row(ui: &mut Ui, settings: &mut PatternSettings) {
    let current = settings.distribution();
    let picked = widgets::property(ui, DISTRIBUTION, |ui| {
        let choices = DISTRIBUTIONS.map(|(_, label, hover)| (label, hover));
        let index = DISTRIBUTIONS
            .iter()
            .position(|(shown, ..)| *shown == current)
            .unwrap_or(usize::MAX);
        widgets::segmented(ui, &choices, index)
            .and_then(|index| DISTRIBUTIONS.get(index).map(|(choice, ..)| *choice))
    });
    if let Some(picked) = picked {
        settings.set_distribution(picked);
    }
}

fn turn_row(ui: &mut Ui, settings: &mut PatternSettings) {
    let current = settings.turn();
    let picked = widgets::property(ui, SPREAD, |ui| {
        let choices = TurnKind::ALL.map(|turn| (turn.label(), turn.about()));
        let index = TurnKind::ALL
            .iter()
            .position(|shown| *shown == current)
            .unwrap_or(usize::MAX);
        widgets::segmented(ui, &choices, index).and_then(|index| TurnKind::ALL.get(index).copied())
    });
    if let Some(picked) = picked {
        settings.set_turn(picked);
    }
}

fn direction_rows(
    ui: &mut Ui,
    shown: &Shown<'_>,
    (settings, patterning): (&mut PatternSettings, &mut Patterning),
    direction: Direction,
) -> bool {
    let removed = widgets::property(ui, &direction.title(), |ui| {
        direction != Direction::FIRST && ui.add(widgets::button(direction.remove_label())).clicked()
    });

    field_rows(
        ui,
        shown,
        settings,
        &[
            PatternField::Count(direction),
            PatternField::Spacing(direction),
        ],
    );

    let line = patterning.line_of(direction);
    let picking = patterning.picking() == Some(direction);
    let asked = widgets::property(ui, RUNS_ALONG, |ui| {
        ui.vertical(|ui| {
            let named = line
                .zip(shown.sketch)
                .map(|(line, sketch)| sketch.entity_label(line));

            match (picking, named) {
                (true, _) => wrapped(ui, widgets::muted(CHOOSING, ui)),
                (false, Some(name)) => wrapped(ui, name),
                (false, None) => wrapped(ui, widgets::muted(BY_ANGLE, ui)),
            }
            if picking {
                return ui
                    .add(widgets::button(STOP_CHOOSING))
                    .clicked()
                    .then_some(DirectionAsk::Stop);
            }

            let choose = ui.add(widgets::button(direction.choose_label()));

            if choose.clicked() {
                choose.surrender_focus();
            }

            let choose = choose.clicked().then_some(DirectionAsk::Choose);
            let angle =
                line.is_some() && ui.add(widgets::button(direction.angle_label())).clicked();
            choose.or(angle.then_some(DirectionAsk::Angle))
        })
        .inner
    });

    match asked {
        Some(DirectionAsk::Choose) => patterning.pick_direction(direction),
        Some(DirectionAsk::Stop) => patterning.stop_picking(),
        Some(DirectionAsk::Angle) => patterning.use_angle(direction),
        None => {}
    }
    field_rows(ui, shown, settings, &[PatternField::Angle(direction)]);

    let current = settings.sides(direction);
    let sides = widgets::property(ui, SIDES_CAPTION, |ui| {
        ui.push_id(("pattern-sides", direction), |ui| {
            let choices = SIDES.map(|(_, label, hover)| (label, hover));
            let index = SIDES
                .iter()
                .position(|(sides, ..)| *sides == current)
                .unwrap_or(usize::MAX);
            widgets::segmented(ui, &choices, index)
                .and_then(|index| SIDES.get(index).map(|(sides, ..)| *sides))
        })
        .inner
    });

    if let Some(sides) = sides {
        settings.set_sides(direction, sides);
    }

    let reversed = settings.reversed(direction);

    if current == Sides::One
        && let Some(flipped) =
            crate::feature_fields::reverse_row(ui, &direction.flip_label(), reversed)
    {
        settings.set_reversed(direction, flipped);
    }
    removed
}

enum DirectionAsk {
    Choose,
    Stop,
    Angle,
}

fn copies_row(ui: &mut Ui, spec: &Spec, patterning: &mut Patterning) {
    let rows = copy_rows(spec);

    if rows.is_empty() {
        return;
    }

    let left_out = patterning.left_out_of(spec);
    let mut toggled = None;
    let mut brought_back = false;

    widgets::property(ui, COPIES, |ui| {
        ui.vertical(|ui| {
            for row in &rows {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = ui.spacing().item_spacing.y;
                    for copy in row {
                        let kept = !patterning.is_left_out(copy);
                        let response = ui
                            .add_enabled_ui(!copy.is_original(), |ui| {
                                widgets::instance_toggle(
                                    ui,
                                    kept,
                                    &copy_name(copy),
                                    &copy_about(copy, !kept),
                                )
                            })
                            .inner;

                        if response.clicked() {
                            toggled = Some(copy.clone());
                        }
                    }
                });
            }

            let summary = match left_out {
                0 => COPIES_HINT.to_owned(),
                1 => "1 copy is left out.".to_owned(),
                count => format!("{count} copies are left out."),
            };

            wrapped(ui, widgets::muted(summary, ui));
            if left_out > 0 {
                let button = widgets::small_button(ui, icons::ADD, BRING_BACK);

                brought_back = ui.add(button).on_hover_text(BRING_BACK_ABOUT).clicked();
            }
        });
    });
    if let Some(copy) = toggled {
        patterning.toggle(&copy);
    }
    if brought_back {
        patterning.bring_back();
    }
}

fn copy_rows(spec: &Spec) -> Vec<Vec<PatternCopy>> {
    let copies = spec.copies();

    if copies.is_empty() {
        return Vec::new();
    }

    let mut every = copies;

    every.push(spec.original());
    match spec {
        Spec::Circular(_) => {
            every.sort_by_key(|copy| match copy {
                PatternCopy::Turn(turn) => *turn,
                PatternCopy::Steps(_) => 0,
            });
            vec![every]
        }
        Spec::Rectangular(pattern) => {
            let rightward = drawn_sign(pattern.directions.first(), f64::cos);
            let upward = drawn_sign(pattern.directions.get(1), f64::sin);
            let mut rows: BTreeMap<(Vec<i64>, i64), Vec<PatternCopy>> = BTreeMap::new();

            for copy in every {
                let PatternCopy::Steps(steps) = &copy else {
                    continue;
                };
                let across = steps.get(1).copied().unwrap_or_default();
                let further = steps.get(2..).map(<[i64]>::to_vec).unwrap_or_default();

                rows.entry((further, -across * upward))
                    .or_default()
                    .push(copy);
            }
            rows.into_values()
                .map(|mut row| {
                    row.sort_by_key(|copy| match copy {
                        PatternCopy::Steps(steps) => {
                            steps.first().copied().unwrap_or_default() * rightward
                        }
                        PatternCopy::Turn(_) => 0,
                    });
                    row
                })
                .collect()
        }
    }
}

fn drawn_sign(row: Option<&PatternRow>, part: fn(f64) -> f64) -> i64 {
    let Some(row) = row else {
        return 1;
    };
    let along = match &row.heading {
        Heading::Angle(angle) => part(angle.value.to_radians()),
        Heading::Along { .. } => 1.0,
    };
    let flipped = if row.reversed { -1.0 } else { 1.0 };

    if along * row.distance.value * flipped < 0.0 {
        -1
    } else {
        1
    }
}

fn centre_row(ui: &mut Ui, sketch: Option<&Sketch>, centre: Option<EntityId>) -> bool {
    widgets::property(ui, CENTRE, |ui| {
        ui.vertical(|ui| {
            let named = centre
                .zip(sketch)
                .map(|(point, sketch)| sketch.entity_label(point));

            match named {
                Some(name) => wrapped(ui, name),
                None => wrapped(ui, widgets::muted(NO_CENTRE, ui)),
            }
            wrapped(ui, widgets::muted(CENTRE_HINT, ui));
            centre != Some(EntityId::ORIGIN) && ui.add(widgets::button(USE_ORIGIN)).clicked()
        })
        .inner
    })
}

fn field_rows(
    ui: &mut Ui,
    shown: &Shown<'_>,
    settings: &mut PatternSettings,
    fields: &[PatternField],
) {
    for field in fields {
        if shown.fields.contains(field) {
            field_row(ui, shown, settings, *field);
        }
    }
}

fn field_row(ui: &mut Ui, shown: &Shown<'_>, settings: &mut PatternSettings, field: PatternField) {
    let wording = settings.wording();
    let value = LiveValue {
        id: field_id(field),
        hint: field.placeholder(),
        about: wording.about(field),
        expected: field.expected(),
    };

    let refusal = widgets::property(ui, wording.caption(field), |ui| {
        let text = settings.text_mut(field)?;
        field::live_value(ui, shown.model, text, &value)
    });
    if let Some((_, problem)) = shown
        .evaluation
        .problems
        .iter()
        .find(|(wrong, _)| *wrong == field)
    {
        widgets::error_row(ui, &problem.to_string());
    } else if let Some(refusal) = refusal {
        widgets::error_row(ui, &refusal);
    }
}

fn outcome(
    ui: &mut Ui,
    (kind, turn): (PatternKind, TurnKind),
    evaluation: &PatternEvaluation,
    patterning: &Patterning,
) {
    if let Err(problem) = &evaluation.spec {
        widgets::callout(ui, Tone::Error, |ui| wrapped(ui, problem.to_string()));
        return;
    }
    if let Some(problem) = patterning.problem() {
        widgets::callout(ui, Tone::Error, |ui| {
            wrapped(ui, capitalized(&format!("{problem}.")))
        });
        return;
    }
    if kind == PatternKind::Circular {
        widgets::callout(ui, Tone::Info, |ui| {
            wrapped(ui, format!("{}.", turn.about()))
        });
    }
}

fn wrapped(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(Label::new(text).wrap_mode(TextWrapMode::Wrap));
}
