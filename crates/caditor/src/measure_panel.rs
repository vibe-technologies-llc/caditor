use std::collections::BTreeSet;

use caditor_document::FeatureId;
use caditor_kernel::Accuracy;
use egui::{Label, ScrollArea, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    bodies::{BodyMass, BodyMeshes, MassAccuracy},
    icons,
    measure::{Freshness, MeasureTool, MeasuredLine, Readout, Value},
    model::Model,
    selection::{Pickable, Selection},
    units::{LengthUnit, angle_text},
    visibility,
    widgets::{self, Tone},
};

pub const TITLE: &str = "Measure";
pub const CLOSE: &str = "Close the measure panel";
pub const COPY_ALL: &str = "Copy all";
pub const MEASURING: &str = "Measuring…";
pub const MASS_TITLE: &str = "Mass properties";
pub const EMPTY_HINT: &str = "Select a vertex, edge, face or sketch point to measure it, or two of \
                              them to measure between them. Shift or Ctrl adds to the selection.";
const PANEL_WIDTH: f32 = 300.0;
const MIN_PANEL_WIDTH: f32 = 220.0;
const MASS_SECTION: &str = "measure-mass";
const STALE_OPACITY: f32 = 0.5;
const APPROXIMATELY: &str = "≈ ";
const MESHING: &str = "Waiting for the body's mesh.";
const ALL_BODIES: &str = "Every body shown; select a face, edge or vertex for one body alone.";

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
    fn text(&self) -> String {
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
    pub tree_selected: Option<FeatureId>,
}

fn value_text(value: Value, accuracy: Accuracy, unit: LengthUnit) -> String {
    let text = match value {
        Value::Length(length) => unit.measured_length(length),
        Value::Area(area) => unit.measured_area(area),
        Value::Angle(radians) => angle_text(radians),
        Value::Position(point) => unit.measured_position([point.x, point.y, point.z]),
    };
    approximately(text, accuracy == Accuracy::Approximate)
}

fn approximately(text: String, approximate: bool) -> String {
    if approximate {
        format!("{APPROXIMATELY}{text}")
    } else {
        text
    }
}

pub fn line_label(line: &MeasuredLine, unit: LengthUnit) -> String {
    value_text(
        Value::Length(line.from.distance(line.to)),
        line.accuracy,
        unit,
    )
}

pub fn readout_cards(readout: &Readout, unit: LengthUnit) -> Vec<Card> {
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

fn mass_card(name: String, mass: Option<&BodyMass>, unit: LengthUnit) -> Card {
    let Some(mass) = mass else {
        return Card {
            title: name,
            rows: Vec::new(),
            notes: vec![(Tone::Neutral, MESHING.to_owned())],
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
    Card {
        title: name,
        rows: vec![
            Row {
                label: "Volume".to_owned(),
                text: approximately(unit.measured_volume(properties.volume), approximate),
            },
            Row {
                label: "Surface area".to_owned(),
                text: approximately(unit.measured_area(properties.area), approximate),
            },
            Row {
                label: "Centroid".to_owned(),
                text: approximately(
                    unit.measured_position([centroid.x, centroid.y, centroid.z]),
                    approximate,
                ),
            },
        ],
        notes,
    }
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
                .tree_selected
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

pub fn mass_cards(context: &MeasureContext<'_>) -> (Vec<Card>, bool) {
    let unit = context.model.length_unit();
    let (bodies, everything) = measured_bodies(context);
    let cards = bodies
        .into_iter()
        .map(|body| {
            let name = context.model.document().feature(body).map_or_else(
                || "A deleted body".to_owned(),
                |feature| feature.name.clone(),
            );
            mass_card(name, context.bodies.get(body).map(|mesh| &mesh.mass), unit)
        })
        .collect();
    (cards, everything)
}

pub fn show(ui: &mut Ui, context: &MeasureContext<'_>, tool: &mut MeasureTool) {
    tool.measurements.refresh(context.model, context.selection);
    let unit = context.model.length_unit();
    let shown = tool.measurements.shown();
    let measuring = tool.measurements.is_measuring();
    let cards =
        shown.map(|(readout, freshness)| (readout, readout_cards(readout, unit), freshness));
    let (masses, everything) = mass_cards(context);
    let mut close = false;
    egui::Panel::right("measure")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .min_size(MIN_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::MEASURE, TITLE, |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
                let everything_text = cards
                    .iter()
                    .flat_map(|(_, cards, _)| cards)
                    .chain(&masses)
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
            ScrollArea::vertical().show(ui, |ui| {
                if context.selection.is_empty() {
                    widgets::callout(ui, Tone::Info, |ui| ui.label(EMPTY_HINT));
                    ui.add_space(SPACE_M);
                } else if let Some((readout, cards, freshness)) = &cards {
                    ui.scope(|ui| {
                        if *freshness == Freshness::Stale {
                            ui.multiply_opacity(STALE_OPACITY);
                        }
                        readings(ui, readout, cards);
                    });
                }
                mass_section(ui, &masses, everything);
            });
        });
    if close {
        tool.toggle();
    }
}

fn readings(ui: &mut Ui, readout: &Readout, cards: &[Card]) {
    if let Some(problem) = readout.problem {
        widgets::callout(ui, Tone::Warning, |ui| ui.label(problem));
        ui.add_space(SPACE_M);
    }
    for (index, card) in cards.iter().enumerate() {
        show_card(ui, ("measured", index), card);
        ui.add_space(SPACE_M);
    }
}

fn mass_section(ui: &mut Ui, masses: &[Card], everything: bool) {
    widgets::section(
        ui,
        MASS_SECTION,
        MASS_TITLE,
        Some(masses.len()),
        None,
        |ui| {
            if everything && !masses.is_empty() {
                ui.label(widgets::muted(ALL_BODIES, ui));
            }
            if masses.is_empty() {
                ui.label(widgets::muted("There are no bodies yet.", ui));
            }
            for (index, card) in masses.iter().enumerate() {
                show_card(ui, ("mass", index), card);
                ui.add_space(SPACE_S);
            }
        },
    );
}

fn show_card(ui: &mut Ui, id: (&str, usize), card: &Card) {
    widgets::card(ui, |ui| {
        ui.add(
            Label::new(widgets::strong(&card.title))
                .wrap_mode(TextWrapMode::Wrap)
                .selectable(false),
        );
        if !card.rows.is_empty() {
            widgets::properties(ui, id, |ui| {
                for row in &card.rows {
                    widgets::property(ui, &row.label, |ui| {
                        ui.add(Label::new(&row.text).selectable(true));
                    });
                }
            });
        }
    });
    for (tone, note) in &card.notes {
        widgets::callout(ui, *tone, |ui| ui.label(note));
    }
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
            },
            accuracy: MassAccuracy::Mesh {
                chord: 0.01,
                volume_within: 2.5,
            },
        };

        let card = mass_card("Extrude 1".to_owned(), Some(&mass), LengthUnit::Millimetre);
        let waiting = mass_card("Extrude 2".to_owned(), None, LengthUnit::Millimetre);

        assert_eq!(card.rows[0].text, "≈ 1000.0 mm³");
        assert_eq!(card.rows[2].text, "≈ 5.000, 5.000, 5.000 mm");
        assert!(card.notes[0].1.contains("within 0.010 mm"));
        assert!(card.notes[0].1.contains("within 2.5 mm³"));
        assert!(waiting.rows.is_empty());
    }
}
