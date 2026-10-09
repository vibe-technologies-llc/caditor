use std::collections::BTreeSet;

use caditor_document::{DensityError, FeatureId, displayed_frame};
use caditor_expression::format_number;
use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{Accuracy, MassProperties, SecondMoment};
use egui::{ComboBox, Label, Popup, ScrollArea, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    bodies::{BodyMass, BodyMesh, BodyMeshes, MassAccuracy},
    datum_tools, field,
    guide::Page,
    guide_panel, icons,
    layout::RightPanel,
    measure::{APPROXIMATELY, Freshness, MeasureTool, MeasuredLine, Readout, Relative, Value},
    model::{Action, Model, Notice},
    panels::PanelState,
    parameter_table,
    selection::{Pickable, Selection},
    units::{LengthUnit, Units},
    visibility,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Measure";
pub const CLOSE: &str = "Close the measure panel";
pub const COPY_ALL: &str = "Copy all";
pub const MEASURING: &str = "Measuring…";
pub const MASS_TITLE: &str = "Mass properties";
pub const EMPTY_HINT: &str = "Select a vertex, edge, face or sketch point to measure it, or two of \
                              them to measure between them, or sketch regions for their area and \
                              section properties. Shift or Ctrl adds to the selection.";
const PANEL: RightPanel = RightPanel {
    id: "measure",
    width: 300.0,
    least: 220.0,
};
const MAX_MASS_CARDS: usize = 50;
const MASS_SECTION: &str = "measure-mass";
const STALE_OPACITY: f32 = 0.5;
const DIRECTION_STEP: f64 = 1e-4;
const WORKING_OUT_MASS: &str = "Working out this body's mass properties.";
const NO_DENSITY: &str = "No density set";
const GRAMS_PER_KILOGRAM: f64 = 1000.0;
const GRAMS_PER_CUBIC_MILLIMETRE: f64 = 1e-3;
const ALL_BODIES: &str = "Every body shown; select a face, edge or vertex for one body alone.";
const INERTIA_DIGITS: i32 = 4;
const NOT_EVERY_DENSITY: &str = "Not every body has a density";
pub const COPY_VALUE: &str = "Copy value";
pub const NEW_PARAMETER: &str = "New parameter from this value";
const MORE_FOR_VALUE: &str = "Copy this value or make it a parameter";
pub const RELATIVE_TO: &str = "Relative to";
pub const WORLD: &str = "World";
const RELATIVE_HOVER: &str = "Positions, directions and the distances along X, Y and Z are read \
                              in this coordinate system";
const WORLD_HOVER: &str = "Read from the origin along the principal axes";

pub fn relative(model: &Model, tool: &MeasureTool) -> Result<Relative, String> {
    let Some(frame) = tool.relative_to else {
        return Ok(Relative::WORLD);
    };
    let name = model
        .document()
        .feature(frame)
        .map(|feature| feature.name.clone());
    match name {
        None => Err(
            "The chosen coordinate system no longer exists, so the readings are relative to the \
             world"
                .to_owned(),
        ),
        Some(name) => displayed_frame(model.evaluation(), frame)
            .and_then(|placed| Relative::to(&placed))
            .ok_or_else(|| {
                format!("{name} has no result yet, so the readings are relative to the world")
            }),
    }
}

fn relative_row(ui: &mut Ui, model: &Model, relative_to: &mut Option<FeatureId>) {
    let document = model.document();
    let frames = datum_tools::frames_before(document, usize::MAX);
    if frames.is_empty() && relative_to.is_none() {
        return;
    }
    let name = |frame: FeatureId| {
        document.feature(frame).map_or_else(
            || "A deleted coordinate system".to_owned(),
            |feature| feature.name.clone(),
        )
    };
    let shown = relative_to.map_or_else(|| WORLD.to_owned(), name);
    widgets::properties(ui, "measure-relative", |ui| {
        widgets::property(ui, RELATIVE_TO, |ui| {
            let combo = ComboBox::from_id_salt("measure-relative-to")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    let world = widgets::menu_option(ui, relative_to.is_none(), WORLD)
                        .on_hover_text(WORLD_HOVER);
                    if world.clicked() {
                        *relative_to = None;
                    }
                    for frame in frames {
                        let chosen = *relative_to == Some(frame);
                        if widgets::menu_option(ui, chosen, &name(frame)).clicked() {
                            *relative_to = Some(frame);
                        }
                    }
                });
            widgets::tie_to_caption(ui, &combo.response);
            combo.response.on_hover_text(RELATIVE_HOVER);
        });
    });
    ui.add_space(SPACE_S);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub title: String,
    pub rows: Vec<Row>,
    pub notes: Vec<(Tone, String)>,
}

impl Card {
    pub fn text(&self) -> String {
        std::iter::once(self.title.clone())
            .chain(
                self.rows
                    .iter()
                    .map(|row| format!("  {}: {}", row.label, row.text)),
            )
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct MeasureContext<'a> {
    pub model: &'a Model,
    pub selection: &'a Selection,
    pub bodies: &'a BodyMeshes,
    pub tree_bodies: &'a [FeatureId],
}

fn value_text(value: Value, accuracy: Accuracy, unit: Units) -> String {
    let text = match value {
        Value::Length(length) => unit.measured_length(length),
        Value::Area(area) => unit.measured_area(area),
        Value::Angle(radians) => unit.angle.text_of_radians(radians),
        Value::Position(point) => unit.measured_position([point.x, point.y, point.z]),
        Value::Direction(direction) => direction_text(direction),
        Value::SecondMoment(moment) => unit.measured_second_moment(moment),
    };
    approximately(text, accuracy == Accuracy::Approximate)
}

fn direction_text(direction: Vector3) -> String {
    let unit = direction.normalize_or_zero();
    let component = |value: f64| {
        let rounded = (value / DIRECTION_STEP).round() * DIRECTION_STEP + 0.0;
        format_number(rounded)
    };
    format!(
        "({}, {}, {})",
        component(unit.x),
        component(unit.y),
        component(unit.z)
    )
}

fn approximately(text: String, approximate: bool) -> String {
    if approximate {
        format!("{APPROXIMATELY}{text}")
    } else {
        text
    }
}

pub fn line_label(line: &MeasuredLine, unit: impl Into<Units>) -> String {
    value_text(
        Value::Length(line.from.distance(line.to)),
        line.accuracy,
        unit.into(),
    )
}

pub fn readout_cards(readout: &Readout, unit: impl Into<Units>) -> Vec<Card> {
    let unit = unit.into();
    readout
        .groups
        .iter()
        .map(|group| {
            let approximate = group
                .readings
                .iter()
                .any(|reading| reading.accuracy == Accuracy::Approximate);
            let notes = group
                .problem
                .iter()
                .map(|problem| (Tone::Warning, problem.clone()))
                .chain(approximate.then(|| {
                    (
                        Tone::Info,
                        "Values marked ≈ were found numerically or from the display mesh."
                            .to_owned(),
                    )
                }))
                .collect();
            Card {
                title: group.title.clone(),
                rows: group
                    .readings
                    .iter()
                    .map(|reading| Row {
                        label: reading.label.to_owned(),
                        text: value_text(reading.value, reading.accuracy, unit),
                    })
                    .collect(),
                notes,
            }
        })
        .collect()
}

pub struct Substance<'a> {
    pub material: Option<&'a str>,
    pub density: Option<Result<f64, DensityError>>,
}

impl Substance<'_> {
    const UNKNOWN: Self = Self {
        material: None,
        density: None,
    };
}

pub fn mass_text(grams: f64) -> String {
    if grams.abs() < GRAMS_PER_KILOGRAM {
        format!("{grams:.2} g")
    } else {
        format!("{:.3} kg", grams / GRAMS_PER_KILOGRAM)
    }
}

fn mass_rows(
    substance: &Substance<'_>,
    volume: f64,
    approximate: bool,
) -> (Vec<Row>, Option<String>) {
    let material = substance.material.map(|material| Row {
        label: "Material".to_owned(),
        text: material.to_owned(),
    });
    let (text, problem) = match &substance.density {
        None => (NO_DENSITY.to_owned(), None),
        Some(Ok(density)) => (
            approximately(
                mass_text(volume * density * GRAMS_PER_CUBIC_MILLIMETRE),
                approximate,
            ),
            None,
        ),
        Some(Err(error)) => (
            "Not available".to_owned(),
            Some(format!(
                "{} Correct it in the body's colour and material.",
                field::sentence(&format!("{error}."))
            )),
        ),
    };
    let mass = Row {
        label: "Mass".to_owned(),
        text,
    };
    (material.into_iter().chain([mass]).collect(), problem)
}

fn significant(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return format_number(value);
    }
    let decimals = INERTIA_DIGITS - 1 - value.abs().log10().floor() as i32;
    let step = 10f64.powi(decimals);
    format_number((value * step).round() / step)
}

fn inertia_text(grams_square_millimetres: [f64; 3], unit: LengthUnit) -> String {
    let scaled = grams_square_millimetres.map(|value| value / unit.millimetres().powi(2));
    let largest = scaled
        .iter()
        .fold(0.0, |most: f64, value| most.max(value.abs()));
    let (factor, mass) = if largest >= GRAMS_PER_KILOGRAM {
        (GRAMS_PER_KILOGRAM.recip(), "kg")
    } else {
        (1.0, "g")
    };
    let [x, y, z] = scaled.map(|value| significant(value * factor));
    format!("{x}, {y}, {z} {mass}·{}²", unit.symbol())
}

fn inertia_rows(
    second_moment: &SecondMoment,
    density: f64,
    unit: LengthUnit,
    approximate: bool,
) -> [Row; 2] {
    let grams_per_volume = density * GRAMS_PER_CUBIC_MILLIMETRE;
    let inertia =
        MassProperties::inertia(second_moment).map(|row| row.map(|entry| entry * grams_per_volume));
    let [[xx, ..], [_, yy, _], [.., zz]] = inertia;
    [
        Row {
            label: "Inertia about the centroid (x, y, z)".to_owned(),
            text: approximately(inertia_text([xx, yy, zz], unit), approximate),
        },
        Row {
            label: "Principal moments".to_owned(),
            text: approximately(
                inertia_text(MassProperties::principal_moments(&inertia), unit),
                approximate,
            ),
        },
    ]
}

fn mass_card(
    name: String,
    mass: Option<&BodyMass>,
    unit: LengthUnit,
    substance: &Substance<'_>,
) -> Card {
    let Some(mass) = mass else {
        return Card {
            title: name,
            rows: Vec::new(),
            notes: vec![(Tone::Neutral, WORKING_OUT_MASS.to_owned())],
        };
    };
    let (approximate, notes) = match mass.accuracy {
        MassAccuracy::Exact => (false, Vec::new()),
        MassAccuracy::Mesh {
            chord,
            volume_within,
        } => (
            true,
            vec![(
                Tone::Info,
                format!(
                    "Found from the display mesh, which follows curved faces within {}; the \
                     volume is within {}.",
                    unit.measured_length(chord),
                    unit.measured_volume(volume_within)
                ),
            )],
        ),
    };
    let properties = mass.properties;
    let centroid = properties.centroid;
    let (substance_rows, problem) = mass_rows(substance, properties.volume, approximate);
    let inertia = match substance.density {
        Some(Ok(density)) => {
            inertia_rows(&properties.second_moment, density, unit, approximate).to_vec()
        }
        None | Some(Err(_)) => Vec::new(),
    };
    let notes = problem
        .map(|problem| (Tone::Warning, problem))
        .into_iter()
        .chain(notes)
        .collect();
    let volume = Row {
        label: "Volume".to_owned(),
        text: approximately(unit.measured_volume(properties.volume), approximate),
    };
    Card {
        title: name,
        rows: std::iter::once(volume)
            .chain(substance_rows)
            .chain([
                Row {
                    label: "Surface area".to_owned(),
                    text: approximately(unit.measured_area(properties.area), approximate),
                },
                Row {
                    label: "Size".to_owned(),
                    text: mass.size.map_or_else(
                        || "Not available".to_owned(),
                        |size| approximately(unit.measured_size(size), approximate),
                    ),
                },
                Row {
                    label: "Centroid".to_owned(),
                    text: approximately(
                        unit.measured_position([centroid.x, centroid.y, centroid.z]),
                        approximate,
                    ),
                },
            ])
            .chain(inertia)
            .collect(),
        notes,
    }
}

fn total_card(parts: &[(&BodyMass, Substance<'_>)], unit: LengthUnit) -> Option<Card> {
    if parts.len() < 2 {
        return None;
    }
    let approximate = parts
        .iter()
        .any(|(mass, _)| mass.accuracy != MassAccuracy::Exact);
    let densities: Option<Vec<f64>> = parts
        .iter()
        .map(|(_, substance)| {
            substance
                .density
                .as_ref()
                .and_then(|density| density.as_ref().ok().copied())
        })
        .collect();
    let weights: Vec<f64> = match &densities {
        Some(densities) => densities.clone(),
        None => vec![1.0; parts.len()],
    };
    let volume: f64 = parts.iter().map(|(mass, _)| mass.properties.volume).sum();
    let area: f64 = parts.iter().map(|(mass, _)| mass.properties.area).sum();
    let weighted: f64 = parts
        .iter()
        .zip(&weights)
        .map(|((mass, _), weight)| mass.properties.volume * weight)
        .sum();
    let centroid = if weighted.abs() > f64::MIN_POSITIVE {
        parts
            .iter()
            .zip(&weights)
            .fold(Point3::ZERO, |sum, ((mass, _), weight)| {
                sum + mass.properties.centroid * (mass.properties.volume * weight / weighted)
            })
    } else {
        Point3::ZERO
    };
    let mass_row = Row {
        label: "Mass".to_owned(),
        text: match &densities {
            Some(_) => approximately(
                mass_text(weighted * GRAMS_PER_CUBIC_MILLIMETRE),
                approximate,
            ),
            None => NOT_EVERY_DENSITY.to_owned(),
        },
    };
    let inertia = densities.map(|densities| {
        let mut second: SecondMoment = [[0.0; 3]; 3];
        for ((mass, _), density) in parts.iter().zip(densities) {
            let about = mass.properties.second_moment_about(centroid);
            for (row, part_row) in second.iter_mut().zip(about) {
                for (entry, part) in row.iter_mut().zip(part_row) {
                    *entry += part * density;
                }
            }
        }
        inertia_rows(&second, 1.0, unit, approximate)
    });
    Some(Card {
        title: format!("All {} bodies", parts.len()),
        rows: [
            Row {
                label: "Volume".to_owned(),
                text: approximately(unit.measured_volume(volume), approximate),
            },
            mass_row,
            Row {
                label: "Surface area".to_owned(),
                text: approximately(unit.measured_area(area), approximate),
            },
            Row {
                label: "Centroid".to_owned(),
                text: approximately(
                    unit.measured_position([centroid.x, centroid.y, centroid.z]),
                    approximate,
                ),
            },
        ]
        .into_iter()
        .chain(inertia.into_iter().flatten())
        .collect(),
        notes: Vec::new(),
    })
}

fn measured_bodies(context: &MeasureContext<'_>) -> (Vec<FeatureId>, bool) {
    let evaluation = context.model.evaluation();
    let document = context.model.document();
    let chosen: BTreeSet<FeatureId> = context
        .selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, .. }
            | Pickable::Edge { body, .. }
            | Pickable::Vertex { body, .. } => Some(body),
            _ => None,
        })
        .chain(
            context
                .tree_bodies
                .iter()
                .copied()
                .filter(|feature| evaluation.body_result(*feature).is_some()),
        )
        .collect();
    if !chosen.is_empty() {
        return (chosen.into_iter().collect(), false);
    }
    let shown = evaluation
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| visibility::is_shown(document, *body))
        .collect();
    (shown, true)
}

pub struct Masses {
    pub cards: Vec<Card>,
    pub total: Option<Card>,
    pub everything: bool,
    pub bodies: usize,
}

pub fn mass_cards(context: &MeasureContext<'_>) -> Masses {
    let unit = context.model.length_unit();
    let (bodies, everything) = measured_bodies(context);
    let count = bodies.len();
    let parameters = context.model.parameters();
    let substance_of = |body: FeatureId| {
        context
            .model
            .document()
            .feature(body)
            .map_or(Substance::UNKNOWN, |feature| Substance {
                material: feature.appearance.material.as_deref(),
                density: feature.appearance.density_value(parameters),
            })
    };
    let cards = bodies
        .iter()
        .take(MAX_MASS_CARDS)
        .map(|body| {
            let name = context.model.document().feature(*body).map_or_else(
                || "A deleted body".to_owned(),
                |feature| feature.name.clone(),
            );
            mass_card(
                name,
                context.bodies.get(*body).and_then(BodyMesh::mass),
                unit,
                &substance_of(*body),
            )
        })
        .collect();
    let parts: Option<Vec<(&BodyMass, Substance<'_>)>> = bodies
        .iter()
        .map(|body| {
            context
                .bodies
                .get(*body)
                .and_then(BodyMesh::mass)
                .map(|mass| (mass, substance_of(*body)))
        })
        .collect();
    Masses {
        cards,
        total: parts.and_then(|parts| total_card(&parts, unit)),
        everything,
        bodies: count,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum RowChoice {
    Copy { label: String, text: String },
    Parameter { label: String, value: Value },
}

pub fn show(
    ui: &mut Ui,
    context: &MeasureContext<'_>,
    tool: &mut MeasureTool,
    room: f32,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    let relative = relative(context.model, tool);
    tool.measurements.refresh(
        context.model,
        context.selection,
        relative.clone().unwrap_or(Relative::WORLD),
    );
    let unit = context.model.units();
    let shown = tool.measurements.shown();
    let measuring = tool.measurements.is_measuring();
    let cards =
        shown.map(|(readout, freshness)| (readout, readout_cards(readout, unit), freshness));
    let masses = mass_cards(context);
    let mut close = false;
    let mut chosen = None;
    let mut relative_to = tool.relative_to;
    PANEL.show(ui, room, |ui| {
        ui.add_space(SPACE_S);
        widgets::panel_header(ui, icons::MEASURE, TITLE, |ui| {
            close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
            guide_panel::help_button(ui, Page::Measure);
            let everything_text = cards
                .iter()
                .flat_map(|(_, cards, _)| cards)
                .chain(&masses.cards)
                .map(Card::text)
                .collect::<Vec<_>>()
                .join("\n");
            let copy = widgets::small_button(ui, icons::COPY, COPY_ALL);
            if ui
                .add(copy)
                .on_hover_text("Copy every reading in the panel as text")
                .clicked()
            {
                ui.ctx().copy_text(everything_text);
            }
            if measuring && !context.selection.is_empty() {
                ui.label(widgets::muted(MEASURING, ui));
            }
        });
        ui.add_space(SPACE_S);
        relative_row(ui, context.model, &mut relative_to);
        if let Err(problem) = &relative {
            widgets::callout(ui, Tone::Warning, |ui| ui.label(problem));
            ui.add_space(SPACE_M);
        }
        ScrollArea::vertical().show(ui, |ui| {
            if context.selection.is_empty() {
                widgets::callout(ui, Tone::Info, |ui| ui.label(EMPTY_HINT));
                ui.add_space(SPACE_M);
            } else if let Some((readout, cards, freshness)) = &cards {
                ui.scope(|ui| {
                    if *freshness == Freshness::Stale {
                        ui.multiply_opacity(STALE_OPACITY);
                    }
                    chosen = readings(ui, readout, cards);
                });
            }
            chosen = mass_section(ui, &masses).or(chosen.take());
        });
    });
    tool.relative_to = relative_to;
    if close {
        tool.toggle();
    }
    if let Some(choice) = chosen {
        perform(ui, context.model, state, actions, choice);
    }
}

fn perform(
    ui: &Ui,
    model: &Model,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
    choice: RowChoice,
) {
    match choice {
        RowChoice::Copy { label, text } => {
            ui.ctx().copy_text(text.clone());
            actions.push(Action::Inform(Notice::success(format!(
                "Copied {}: {text}.",
                label.to_lowercase()
            ))));
        }
        RowChoice::Parameter { label, value } => {
            let Some(expression) = parameter_expression(value, model.units()) else {
                return;
            };
            let text = model.document().expression_text(&expression);
            let name = parameter_table::add_with(
                model,
                state,
                actions,
                &parameter_stem(&label),
                expression,
            );
            actions.push(Action::Inform(Notice::success(format!(
                "Added the parameter {name} = {text} from the measured {}. Type a new name in \
                 its field in Parameters to rename it.",
                label.to_lowercase()
            ))));
        }
    }
}

fn parameter_expression(value: Value, units: Units) -> Option<caditor_expression::Expression> {
    match value {
        Value::Length(millimetres) => Some(units.measured(millimetres)),
        Value::Angle(radians) => Some(units.angle.measured(radians.to_degrees())),
        Value::Area(square_millimetres) => Some(units.measured_area_expression(square_millimetres)),
        Value::Position(_) | Value::Direction(_) | Value::SecondMoment(_) => None,
    }
}

fn parameter_stem(label: &str) -> String {
    label
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn readings(ui: &mut Ui, readout: &Readout, cards: &[Card]) -> Option<RowChoice> {
    if let Some(problem) = readout.problem {
        widgets::callout(ui, Tone::Warning, |ui| ui.label(problem));
        ui.add_space(SPACE_M);
    }
    let mut chosen = None;
    for (index, (card, group)) in cards.iter().zip(&readout.groups).enumerate() {
        let values: Vec<Option<Value>> = group
            .readings
            .iter()
            .map(|reading| Some(reading.value))
            .collect();
        chosen = card_with_menus(ui, ("measured", index), card, &values).or(chosen.take());
        ui.add_space(SPACE_M);
    }
    chosen
}

fn mass_section(ui: &mut Ui, masses: &Masses) -> Option<RowChoice> {
    let mut chosen = None;
    widgets::section(
        ui,
        MASS_SECTION,
        MASS_TITLE,
        Some(masses.bodies),
        None,
        |ui| {
            if masses.everything && !masses.cards.is_empty() {
                ui.label(widgets::muted(ALL_BODIES, ui));
            }
            if masses.cards.is_empty() {
                ui.label(widgets::muted("There are no bodies yet.", ui));
            }
            if let Some(total) = &masses.total {
                chosen = card_with_menus(ui, ("mass-total", 0), total, &[]);
                ui.add_space(SPACE_S);
            }
            for (index, card) in masses.cards.iter().enumerate() {
                chosen = card_with_menus(ui, ("mass", index), card, &[]).or(chosen.take());
                ui.add_space(SPACE_S);
            }
            let unlisted = masses.bodies.saturating_sub(masses.cards.len());
            if unlisted > 0 {
                ui.label(widgets::muted(
                    format!("{unlisted} more not listed. Select bodies to see theirs."),
                    ui,
                ));
            }
        },
    );
    chosen
}

pub fn show_card(ui: &mut Ui, id: (&str, usize), card: &Card) {
    card_rows(ui, id, card, |ui, row, _| {
        ui.add(Label::new(&row.text).selectable(true));
        None
    });
}

fn card_with_menus(
    ui: &mut Ui,
    id: (&str, usize),
    card: &Card,
    values: &[Option<Value>],
) -> Option<RowChoice> {
    card_rows(ui, id, card, |ui, row, index| {
        let value = values.get(index).copied().flatten();
        value_with_menu(ui, row, value)
    })
}

fn card_rows(
    ui: &mut Ui,
    id: (&str, usize),
    card: &Card,
    mut value: impl FnMut(&mut Ui, &Row, usize) -> Option<RowChoice>,
) -> Option<RowChoice> {
    let mut chosen = None;
    widgets::card(ui, |ui| {
        ui.add(
            Label::new(widgets::strong(&card.title))
                .wrap_mode(TextWrapMode::Wrap)
                .selectable(false),
        );
        if !card.rows.is_empty() {
            widgets::properties(ui, id, |ui| {
                for (index, row) in card.rows.iter().enumerate() {
                    widgets::property(ui, &row.label, |ui| {
                        chosen = value(ui, row, index).or(chosen.take());
                    });
                }
            });
        }
    });
    for (tone, note) in &card.notes {
        widgets::callout(ui, *tone, |ui| ui.label(note));
    }
    chosen
}

fn value_with_menu(ui: &mut Ui, row: &Row, value: Option<Value>) -> Option<RowChoice> {
    ui.horizontal(|ui| {
        let shown = widgets::label_before_icon_buttons(ui, &row.text, 1);
        let more = widgets::named(
            widgets::icon_button(ui, icons::MORE, MORE_FOR_VALUE),
            &format!("More for {}", row.label),
        );
        let mut chosen = None;
        Popup::menu(&more).show(|ui| {
            widgets::fitted_menu(ui, |ui| chosen = row_menu(ui, row, value));
        });
        shown.context_menu(|ui| {
            widgets::fitted_menu(ui, |ui| chosen = row_menu(ui, row, value).or(chosen.take()));
        });
        chosen
    })
    .inner
}

fn row_menu(ui: &mut Ui, row: &Row, value: Option<Value>) -> Option<RowChoice> {
    let mut chosen = None;
    if widgets::menu_item(ui, icons::COPY, COPY_VALUE, None).clicked() {
        chosen = Some(RowChoice::Copy {
            label: row.label.clone(),
            text: row.text.clone(),
        });
        ui.close();
    }
    let parameterisable =
        value.filter(|value| matches!(value, Value::Length(_) | Value::Angle(_) | Value::Area(_)));
    if let Some(value) = parameterisable
        && widgets::menu_item(ui, icons::PARAMETERS, NEW_PARAMETER, None).clicked()
    {
        chosen = Some(RowChoice::Parameter {
            label: row.label.clone(),
            value,
        });
        ui.close();
    }
    chosen
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Point3;
    use caditor_kernel::MassProperties;

    use super::*;
    use crate::measure::{Group, Reading};

    #[test]
    fn readings_are_shown_in_the_chosen_unit_and_approximate_ones_are_marked() {
        let readout = Readout {
            groups: vec![Group {
                title: "Between them".to_owned(),
                readings: vec![
                    Reading {
                        label: "Distance",
                        value: Value::Length(25.4),
                        accuracy: Accuracy::Approximate,
                    },
                    Reading {
                        label: "Angle between the lines",
                        value: Value::Angle(std::f64::consts::FRAC_PI_2),
                        accuracy: Accuracy::Exact,
                    },
                ],
                problem: None,
            }],
            line: None,
            problem: None,
        };

        let cards = readout_cards(&readout, LengthUnit::Centimetre);

        assert_eq!(cards[0].rows[0].text, "≈ 2.5400 cm");
        assert_eq!(cards[0].rows[1].text, "90.00°");
        assert_eq!(cards[0].notes.len(), 1);
        assert_eq!(
            cards[0].text(),
            "Between them\n  Distance: ≈ 2.5400 cm\n  Angle between the lines: 90.00°"
        );
    }

    #[test]
    fn mass_properties_from_a_mesh_state_how_close_they_are() {
        let mass = BodyMass {
            properties: MassProperties {
                volume: 1000.0,
                area: 600.0,
                centroid: Point3::new(5.0, 5.0, 5.0),
                second_moment: [[0.0; 3]; 3],
            },
            accuracy: MassAccuracy::Mesh {
                chord: 0.01,
                volume_within: 2.5,
            },
            size: Some([10.0, 12.5, 8.0]),
        };

        let card = mass_card(
            "Extrude 1".to_owned(),
            Some(&mass),
            LengthUnit::Millimetre,
            &Substance::UNKNOWN,
        );
        let waiting = mass_card(
            "Extrude 2".to_owned(),
            None,
            LengthUnit::Millimetre,
            &Substance::UNKNOWN,
        );

        assert_eq!(card.rows[0].text, "≈ 1000.0 mm³");
        assert_eq!(card.rows[1].text, NO_DENSITY);
        assert_eq!(card.rows[3].text, "≈ 10.000 × 12.500 × 8.000 mm");
        assert_eq!(card.rows[4].text, "≈ 5.000, 5.000, 5.000 mm");
        assert!(card.notes[0].1.contains("within 0.010 mm"));
        assert!(card.notes[0].1.contains("within 2.5 mm³"));
        assert!(waiting.rows.is_empty());
    }

    fn cube_second_moment() -> SecondMoment {
        let along = 8000.0 * 400.0 / 12.0;
        [[along, 0.0, 0.0], [0.0, along, 0.0], [0.0, 0.0, along]]
    }

    #[test]
    fn several_bodies_add_up_about_their_common_centroid() {
        let cube = |x: f64| BodyMass {
            properties: MassProperties {
                volume: 8000.0,
                area: 2400.0,
                centroid: Point3::new(x, 0.0, 0.0),
                second_moment: cube_second_moment(),
            },
            accuracy: MassAccuracy::Exact,
            size: Some([20.0; 3]),
        };
        let (left, right) = (cube(-20.0), cube(20.0));
        let water = || Substance {
            material: None,
            density: Some(Ok(1.0)),
        };

        let total = total_card(
            &[(&left, water()), (&right, water())],
            LengthUnit::Millimetre,
        )
        .unwrap();
        let unknown = total_card(
            &[(&left, water()), (&right, Substance::UNKNOWN)],
            LengthUnit::Millimetre,
        )
        .unwrap();

        assert_eq!(total.title, "All 2 bodies");
        assert_eq!(total.rows[0].text, "16000.0 mm³");
        assert_eq!(total.rows[1].text, "16.00 g");
        assert_eq!(total.rows[3].text, "0.000, 0.000, 0.000 mm");
        assert_eq!(total.rows[4].text, "1.067, 7.467, 7.467 kg·mm²");
        assert_eq!(unknown.rows[1].text, NOT_EVERY_DENSITY);
        assert_eq!(unknown.rows.len(), 4);
        assert!(total_card(&[(&left, water())], LengthUnit::Millimetre).is_none());
    }

    #[test]
    fn a_body_with_a_density_shows_its_material_and_mass() {
        let mass = BodyMass {
            properties: MassProperties {
                volume: 8000.0,
                area: 2400.0,
                centroid: Point3::ZERO,
                second_moment: cube_second_moment(),
            },
            accuracy: MassAccuracy::Exact,
            size: Some([20.0; 3]),
        };
        let steel = Substance {
            material: Some("Steel"),
            density: Some(Ok(7.85)),
        };
        let wrong = Substance {
            material: None,
            density: Some(Err(DensityError::NotAboveZero(-1.0))),
        };

        let card = mass_card(
            "Block".to_owned(),
            Some(&mass),
            LengthUnit::Millimetre,
            &steel,
        );
        let refused = mass_card(
            "Block".to_owned(),
            Some(&mass),
            LengthUnit::Millimetre,
            &wrong,
        );

        assert_eq!(
            card.text(),
            "Block\n  Volume: 8000.0 mm³\n  Material: Steel\n  Mass: 62.80 g\n  Surface area: \
             2400.00 mm²\n  Size: 20.000 × 20.000 × 20.000 mm\n  Centroid: 0.000, 0.000, 0.000 \
             mm\n  Inertia about the centroid (x, y, z): 4.187, 4.187, 4.187 kg·mm²\n  Principal \
             moments: 4.187, 4.187, 4.187 kg·mm²"
        );
        assert!(card.notes.is_empty());
        assert_eq!(refused.rows[1].text, "Not available");
        assert_eq!(
            refused.notes,
            [(
                Tone::Warning,
                "The density must be above zero, and -1 is not. Correct it in the body's colour \
                 and material."
                    .to_owned()
            )]
        );
        assert_eq!(mass_text(1234.5), "1.234 kg");
        assert_eq!(mass_text(0.5), "0.50 g");
    }

    #[test]
    fn lengths_angles_and_areas_become_parameter_values_in_the_chosen_units() {
        let units = Units::from(LengthUnit::Centimetre);
        let text = |value| {
            parameter_expression(value, units)
                .map(|expression| expression.to_text(&|_| None::<&str>))
        };
        let area = parameter_expression(Value::Area(1234.5678), units).unwrap();
        let read_back =
            caditor_expression::Expression::parse_stored(&area.to_text(&|_| None::<&str>));

        assert_eq!(text(Value::Length(25.4)).as_deref(), Some("2.54 cm"));
        assert_eq!(
            text(Value::Angle(std::f64::consts::FRAC_PI_2)).as_deref(),
            Some("90 deg")
        );
        assert_eq!(
            text(Value::Area(1234.5678)).as_deref(),
            Some("12.345678 cm²")
        );
        assert_eq!(read_back.ok(), Some(area));
        assert_eq!(text(Value::Position(Point3::ZERO)), None);
        assert_eq!(
            parameter_stem("Angle between the lines"),
            "angle_between_the_lines"
        );
        assert_eq!(parameter_stem("Along X"), "along_x");
    }
}
