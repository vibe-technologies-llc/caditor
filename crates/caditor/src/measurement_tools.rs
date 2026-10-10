use caditor_document::{
    AxisReference, Between, DatumResult, Document, Edit, Feature, FeatureKind, FeatureState,
    MeasuredItem, Measurement, MeasurementResult, Of, ParameterOwner, PlaneReference,
    PointReference, Reading, Transaction,
};
use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::Point3;
use caditor_kernel::{Accuracy, EdgeReference, FaceReference, LINEAR_RESOLUTION};
use egui::Ui;

use crate::{
    bodies, datum_tools, editing, feature_fields, measure::APPROXIMATELY, model::Model,
    parameter_table, selection::Pickable, units::Units, widgets,
};

pub const TITLE: &str = "Measurement";
pub const KEEP: &str = "Keep this measurement";
pub const READING: &str = "Reading";
pub const DESCRIPTION: &str = "A reading of the model taken where it stands in the tree and kept \
                               up to date on every recompute; features below it can use its value \
                               by name";
const AREA: Dimension = Dimension::new(2, 0);
const NO_READING: &str = "No reading now";
const NOT_NAMED: &str = "Not named";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keepable {
    Distance,
    Angle,
    Length,
    Radius,
    Area,
}

impl Keepable {
    pub fn of_row(label: &str, between: bool) -> Option<Self> {
        match (between, label) {
            (true, "Distance") => Some(Self::Distance),
            (
                true,
                "Angle at the corner"
                | "Angle between the lines"
                | "Angle between the planes"
                | "Angle to the plane",
            ) => Some(Self::Angle),
            (false, "Length") => Some(Self::Length),
            (false, "Radius") => Some(Self::Radius),
            (false, "Area") => Some(Self::Area),
            _ => None,
        }
    }

    pub fn reading(self, items: &[MeasuredItem]) -> Option<Reading> {
        let between = |quantity| match items {
            [first, second] => Some(Reading::Between {
                quantity,
                first: first.clone(),
                second: second.clone(),
            }),
            _ => None,
        };
        let of = |quantity| match items {
            [item] => Some(Reading::Of {
                quantity,
                item: item.clone(),
            }),
            _ => None,
        };
        match self {
            Self::Distance => between(Between::Distance),
            Self::Angle => between(Between::Angle),
            Self::Length => of(Of::Length),
            Self::Radius => of(Of::Radius),
            Self::Area => of(Of::Area),
        }
    }
}

pub fn item_of(model: &Model, pickable: Pickable) -> Option<MeasuredItem> {
    let evaluation = model.evaluation();
    Some(match pickable {
        Pickable::Origin => MeasuredItem::Point(PointReference::Origin),
        Pickable::Vertex { body, vertex } => {
            let shown = bodies::shown(evaluation, body)?;
            let [_] = shown.names().vertices_named(vertex.name) else {
                return None;
            };
            MeasuredItem::Point(PointReference::Vertex {
                body,
                vertex: vertex.name,
            })
        }
        Pickable::Edge { body, edge } => {
            let shown = bodies::shown(evaluation, body)?;
            let found = bodies::find_edge(shown, edge)?;
            MeasuredItem::Edge {
                body,
                edge: Box::new(EdgeReference::capture(&shown.solid, found)?),
            }
        }
        Pickable::Face { body, face } => {
            let shown = bodies::shown(evaluation, body)?;
            let found = bodies::find_face(shown, face)?;
            MeasuredItem::Face {
                body,
                face: FaceReference::capture(&shown.solid, found)?,
            }
        }
        Pickable::SketchEntity { feature, entity } => MeasuredItem::Sketch {
            sketch: feature,
            entity,
        },
        Pickable::Axis(axis) => MeasuredItem::Axis(AxisReference::Principal(axis.principal())),
        Pickable::Plane(plane) => MeasuredItem::Plane(PlaneReference::Principal(plane)),
        Pickable::Datum(feature) => match datum_tools::result(evaluation, feature)? {
            DatumResult::Plane(_) => MeasuredItem::Plane(PlaneReference::Datum(feature)),
            DatumResult::Axis(_) => MeasuredItem::Axis(AxisReference::Datum(feature)),
            DatumResult::Point(_) => MeasuredItem::Point(PointReference::Datum(feature)),
            DatumResult::Frame(_) => MeasuredItem::Point(PointReference::Frame(feature)),
        },
        Pickable::FrameAxis { feature, axis } => MeasuredItem::Axis(AxisReference::Frame {
            frame: feature,
            axis,
        }),
        Pickable::FramePlane { feature, plane } => MeasuredItem::Plane(PlaneReference::Frame {
            frame: feature,
            plane,
        }),
        Pickable::SketchConstraint { .. }
        | Pickable::SketchRegion { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::CentreOfMass(_) => return None,
    })
}

pub struct Kept {
    pub transaction: Transaction,
    pub parameter: ParameterId,
    pub name: String,
    pub feature: String,
}

pub fn keep(
    document: &Document,
    reading: Reading,
    stem: &str,
    standing: Expression,
) -> Result<Kept, String> {
    let name = parameter_table::unused_name(document, stem);
    let feature = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Keep {}", reading.label().to_lowercase()));
    let parameter = transaction.add_parameter(name.clone(), standing);
    let measurement = transaction.add_feature(
        feature.clone(),
        FeatureKind::from(Measurement {
            reading,
            parameter: Some(parameter),
        }),
    );
    transaction.edit(Edit::SetParameterOwner {
        id: parameter,
        owner: Some(ParameterOwner::Feature {
            feature: measurement,
            value: READING.to_owned(),
        }),
    });
    let transaction = transaction.finish();
    document
        .check(&transaction)
        .map_err(|error| format!("The measurement could not be kept: {error}."))?;
    Ok(Kept {
        transaction,
        parameter,
        name,
        feature,
    })
}

pub fn reading_text(units: Units, result: &MeasurementResult) -> String {
    let value = result.value;
    let shown = match value.dimension {
        Dimension::LENGTH => units.measured_length(value.value),
        Dimension::ANGLE => units.angle.text(value.value),
        AREA => units.measured_area(value.value),
        _ => units.show(value),
    };
    if result.accuracy == Accuracy::Approximate {
        format!("{APPROXIMATELY}{shown}")
    } else {
        shown
    }
}

pub struct Shown {
    pub line: Option<[Point3; 2]>,
    pub anchor: Point3,
    pub label: String,
}

pub fn shown(model: &Model) -> Vec<Shown> {
    let document = model.document();
    let evaluation = model.evaluation();
    let units = model.units();
    document
        .active_features()
        .filter(|feature| !feature.hidden)
        .filter_map(|feature| {
            let measurement = feature.kind.measurement()?;
            let status = evaluation.feature(feature.id())?;
            if status.state != FeatureState::UpToDate {
                return None;
            }
            let result = status.result.as_deref()?.measurement()?;
            let line = result
                .line
                .filter(|(from, to)| from.distance(*to) > LINEAR_RESOLUTION)
                .map(|(from, to)| [from, to]);
            let named = measurement
                .parameter
                .and_then(|parameter| document.parameter_name(parameter))
                .unwrap_or(&feature.name);
            Some(Shown {
                line,
                anchor: result.anchor,
                label: format!("{named} = {}", reading_text(units, result)),
            })
        })
        .collect()
}

pub fn show(ui: &mut Ui, model: &Model, feature: &Feature, measurement: &Measurement) {
    let document = model.document();
    let id = feature.id();
    let status = model.evaluation().feature(id);
    let reading = status
        .filter(|status| status.state == FeatureState::UpToDate)
        .and_then(|status| status.result.as_deref())
        .and_then(|result| result.measurement())
        .map(|result| reading_text(model.units(), result));
    widgets::properties(ui, ("measurement-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        widgets::property(ui, "Measures", |ui| {
            ui.label(measurement.reading.describe(document));
        });
        widgets::property(ui, READING, |ui| match &reading {
            Some(text) => {
                ui.label(text);
            }
            None => {
                ui.label(widgets::muted(NO_READING, ui));
            }
        });
        widgets::property(ui, "Named", |ui| {
            match measurement
                .parameter
                .and_then(|parameter| document.parameter_name(parameter))
            {
                Some(name) => {
                    ui.label(name);
                }
                None => {
                    ui.label(widgets::muted(NOT_NAMED, ui));
                }
            }
        });
    });
}
