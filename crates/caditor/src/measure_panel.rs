use std::collections::BTreeSet;

use caditor_document::{DensityError, FeatureId};
use caditor_expression::format_number;
use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{Accuracy, MassProperties, SecondMoment};
use egui::{Label, ScrollArea, TextWrapMode, Ui};

use crate::{
    appearance::{SPACE_M, SPACE_S},
    bodies::{BodyMass, BodyMeshes, MassAccuracy},
    field, icons, layout,
    measure::{APPROXIMATELY, Freshness, MeasureTool, MeasuredLine, Readout, Value},
    model::Model,
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
const PANEL_WIDTH: f32 = 300.0;
const MIN_PANEL_WIDTH: f32 = 220.0;
const MAX_MASS_CARDS: usize = 50;
const MASS_SECTION: &str = "measure-mass";
const STALE_OPACITY: f32 = 0.5;
const DIRECTION_STEP: f64 = 1e-4;
const MESHING: &str = "Waiting for the body's mesh.";
const NO_DENSITY: &str = "No density set";
const GRAMS_PER_KILOGRAM: f64 = 1000.0;
const GRAMS_PER_CUBIC_MILLIMETRE: f64 = 1e-3;
const ALL_BODIES: &str = "Every body shown; select a face, edge or vertex for one body alone.";
const INERTIA_DIGITS: i32 = 4;
const NOT_EVERY_DENSITY: &str = "Not every body has a density";

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
                context.bodies.get(*body).map(|mesh| &mesh.mass),
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
                .map(|mesh| (&mesh.mass, substance_of(*body)))
        })
        .collect();
    Masses {
        cards,
        total: parts.and_then(|parts| total_card(&parts, unit)),
        everything,
        bodies: count,
    }
}

pub fn show(ui: &mut Ui, context: &MeasureContext<'_>, tool: &mut MeasureTool, room: f32) {
    tool.measurements.refresh(context.model, context.selection);
    let unit = context.model.units();
    let shown = tool.measurements.shown();
    let measuring = tool.measurements.is_measuring();
    let cards =
        shown.map(|(readout, freshness)| (readout, readout_cards(readout, unit), freshness));
    let masses = mass_cards(context);
    let mut close = false;
    egui::Panel::right("measure")
        .resizable(true)
        .default_size(PANEL_WIDTH)
        .size_range(layout::panel_widths(room, MIN_PANEL_WIDTH))
        .show(ui, |ui| {
            ui.add_space(SPACE_S);
            widgets::panel_header(ui, icons::MEASURE, TITLE, |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, CLOSE).clicked();
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
                mass_section(ui, &masses);
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

fn mass_section(ui: &mut Ui, masses: &Masses) {
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
                show_card(ui, ("mass-total", 0), total);
                ui.add_space(SPACE_S);
            }
            for (index, card) in masses.cards.iter().enumerate() {
                show_card(ui, ("mass", index), card);
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
}

pub fn show_card(ui: &mut Ui, id: (&str, usize), card: &Card) {
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
}
