use caditor_document::{
    AxisReference, Between, DatumResult, Document, Edit, FeatureId, FeatureKind, FeatureState,
    MASS, MeasuredItem, Measurement, MeasurementResult, Of, ParameterOwner, PlaneReference,
    PointReference, PrincipalAxis, Reading, Transaction,
};
use caditor_expression::{Dimension, Expression, ParameterId};
use caditor_geometry::Point3;
use caditor_kernel::{
    Accuracy, EdgeForm, EdgeReference, FaceForm, FaceReference, LINEAR_RESOLUTION, edge_measure,
    face_form, vertex_names,
};
use caditor_sketch::Entity;

use crate::{
    bodies, datum_tools, editing, field,
    measure::APPROXIMATELY,
    measure_panel::mass_text,
    model::Model,
    parameter_table,
    selection::{Pickable, Selection},
    sketch_placement,
    units::Units,
};

pub const TITLE: &str = "Measurement";
pub const KEEP: &str = "Keep this measurement";
pub const READING: &str = "Reading";
pub const ALONG_AN_AXIS: &str = "Offset along an axis";
pub const POSITION_ALONG_AN_AXIS: &str = "Position along an axis";
const AREA: Dimension = Dimension::new(2, 0);
const GONE: &str = "The measurement no longer exists";
const CHOOSE_ONE: &str = "Select the one item it should measure first";
const ONE_ITEM: &str = "Select only the one item it should measure";
const NOT_MEASURABLE: &str = "A kept measurement cannot hold that; choose a point, an edge, a \
                              face, an axis, a plane, sketch geometry or a centre of mass";
const NOT_OF_A_BODY: &str = "Choose a face, edge or corner of the body it should measure";
const MADE_LATER: &str = "That is made further down the tree than the measurement, or is not \
                          worked out yet; choose something made before it";
const NOT_AN_AXIS: &str = "Choose an axis, a straight edge, a round face or a sketch line to \
                           measure along";
const SAME_ITEM: &str = "It already measures that";
const NO_ANGLE: &str = "A point has no direction, so there is no angle to measure";
const ONE_ITEM_ONLY: &str = "This reading takes one item; switch to Distance, Angle or an offset \
                             along an axis to measure between two";
const AXIS_NAMES: [(PrincipalAxis, &str); 3] = [
    (PrincipalAxis::X, "X"),
    (PrincipalAxis::Y, "Y"),
    (PrincipalAxis::Z, "Z"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keepable {
    Distance,
    Angle,
    Along {
        axis: PrincipalAxis,
        frame: Option<FeatureId>,
    },
    Length,
    Radius,
    Area,
    Sweep,
    Perimeter,
    Volume,
    Mass,
    Position {
        axis: PrincipalAxis,
        frame: Option<FeatureId>,
    },
}

impl Keepable {
    pub fn of_row(label: &str, between: bool, frame: Option<FeatureId>) -> Vec<Self> {
        match (between, label) {
            (false, "Position") => Self::positions(frame),
            _ => Self::of_reading_row(label, between, frame)
                .into_iter()
                .collect(),
        }
    }

    pub fn of_mass_row(label: &str) -> Vec<Self> {
        match label {
            "Volume" => vec![Self::Volume],
            "Mass" => vec![Self::Mass],
            "Surface area" => vec![Self::Area],
            "Centroid" => Self::positions(None),
            _ => Vec::new(),
        }
    }

    fn positions(frame: Option<FeatureId>) -> Vec<Self> {
        AXIS_NAMES
            .into_iter()
            .map(|(axis, _)| Self::Position { axis, frame })
            .collect()
    }

    pub fn axis_name(self) -> Option<&'static str> {
        let Self::Position { axis, .. } = self else {
            return None;
        };
        AXIS_NAMES
            .into_iter()
            .find(|(candidate, _)| *candidate == axis)
            .map(|(_, name)| name)
    }

    pub fn menu_label(self) -> String {
        match self.axis_name() {
            Some(name) => format!("{KEEP} along {name}"),
            None => KEEP.to_owned(),
        }
    }

    fn of_reading_row(label: &str, between: bool, frame: Option<FeatureId>) -> Option<Self> {
        let along = |axis| Some(Self::Along { axis, frame });
        match (between, label) {
            (true, "Distance") => Some(Self::Distance),
            (
                true,
                "Angle at the corner"
                | "Angle between the lines"
                | "Angle between the planes"
                | "Angle to the plane",
            ) => Some(Self::Angle),
            (true, "Along X") => along(PrincipalAxis::X),
            (true, "Along Y") => along(PrincipalAxis::Y),
            (true, "Along Z") => along(PrincipalAxis::Z),
            (false, "Length") => Some(Self::Length),
            (false, "Radius") => Some(Self::Radius),
            (false, "Area") => Some(Self::Area),
            (false, "Sweep") => Some(Self::Sweep),
            (false, "Perimeter") => Some(Self::Perimeter),
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
        let axis_item = |axis, frame: Option<FeatureId>| {
            MeasuredItem::Axis(frame.map_or(AxisReference::Principal(axis), |frame| {
                AxisReference::Frame { frame, axis }
            }))
        };
        match self {
            Self::Distance => between(Between::Distance),
            Self::Angle => between(Between::Angle),
            Self::Along { axis, frame } => match items {
                [first, second] => Some(Reading::Along {
                    first: first.clone(),
                    second: second.clone(),
                    axis: axis_item(axis, frame),
                }),
                _ => None,
            },
            Self::Position { axis, frame } => match items {
                [item] => Some(Reading::Position {
                    item: item.clone(),
                    axis: axis_item(axis, frame),
                }),
                _ => None,
            },
            Self::Volume => of(Of::Volume),
            Self::Mass => of(Of::Mass),
            Self::Length => of(Of::Length),
            Self::Radius => of(Of::Radius),
            Self::Area => of(Of::Area),
            Self::Sweep => of(Of::Sweep),
            Self::Perimeter => of(Of::Perimeter),
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
        | Pickable::FeatureValue { .. }
        | Pickable::SketchRegion { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. }
        | Pickable::BodyItem { .. } => return None,
        Pickable::CentreOfMass(body) => MeasuredItem::Body(body),
    })
}

fn body_at(model: &Model, pickable: Pickable, index: usize) -> Result<MeasuredItem, String> {
    let body = match pickable {
        Pickable::CentreOfMass(body) => Some(body),
        other => other.body(),
    }
    .ok_or(NOT_OF_A_BODY)?;
    let made_before = model
        .document()
        .feature_index(body)
        .is_some_and(|position| position < index);
    if made_before && sketch_placement::body_state_before(model, body, index).is_ok() {
        Ok(MeasuredItem::Body(body))
    } else {
        Err(MADE_LATER.to_owned())
    }
}

pub fn item_at(model: &Model, pickable: Pickable, index: usize) -> Result<MeasuredItem, String> {
    let item = item_of(model, pickable).ok_or(NOT_MEASURABLE)?;
    let document = model.document();
    let made_before = item.features().into_iter().all(|feature| {
        document
            .feature_index(feature)
            .is_some_and(|position| position < index)
    });
    let state = |body| sketch_placement::body_state_before(model, body, index).ok();
    let found_there = match &item {
        MeasuredItem::Edge { body, edge } => {
            state(*body).is_some_and(|solid| edge.resolve(solid).is_ok())
        }
        MeasuredItem::Face { body, face } => {
            state(*body).is_some_and(|solid| face.resolve(solid).is_ok())
        }
        MeasuredItem::Point(PointReference::Vertex { body, vertex }) => {
            state(*body).is_some_and(|solid| {
                vertex_names(solid)
                    .iter()
                    .filter(|(_, name)| *name == vertex)
                    .count()
                    == 1
            })
        }
        MeasuredItem::Body(body) => state(*body).is_some(),
        MeasuredItem::Point(_)
        | MeasuredItem::Axis(_)
        | MeasuredItem::Plane(_)
        | MeasuredItem::Sketch { .. } => true,
    };
    if made_before && found_there {
        Ok(item)
    } else {
        Err(MADE_LATER.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasuredPart {
    First,
    Second,
    Axis,
}

impl MeasuredPart {
    pub fn of(reading: &Reading) -> Vec<Self> {
        match reading {
            Reading::Between { .. } => vec![Self::First, Self::Second],
            Reading::Along { .. } => vec![Self::First, Self::Second, Self::Axis],
            Reading::Of { .. } => vec![Self::First],
            Reading::Position { .. } => vec![Self::First, Self::Axis],
        }
    }

    pub fn item(self, reading: &Reading) -> Option<&MeasuredItem> {
        match (self, reading) {
            (Self::First, Reading::Of { item, .. } | Reading::Position { item, .. }) => Some(item),
            (Self::First, Reading::Between { first, .. } | Reading::Along { first, .. }) => {
                Some(first)
            }
            (Self::Second, Reading::Between { second, .. } | Reading::Along { second, .. }) => {
                Some(second)
            }
            (Self::Axis, Reading::Along { axis, .. } | Reading::Position { axis, .. }) => {
                Some(axis)
            }
            (Self::Second | Self::Axis, _) => None,
        }
    }

    pub fn caption(self, reading: &Reading) -> &'static str {
        match (self, reading) {
            (Self::First, Reading::Of { .. } | Reading::Position { .. }) => "Of",
            (Self::First, _) => "From",
            (Self::Second, _) => "To",
            (Self::Axis, _) => "Along",
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::First | Self::Second => {
                "Click a point, edge, face, axis, plane, sketch geometry or centre of mass to \
                 measure"
            }
            Self::Axis => {
                "Click an axis, straight edge, round face or sketch line to measure along"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Point,
    Straight,
    Round,
    Curved,
    Flat,
    RoundFace,
    Face,
    Axis,
    Plane,
    Body,
    Unknown,
}

fn form_of(model: &Model, item: &MeasuredItem, index: usize) -> Form {
    let state = |body| sketch_placement::body_state_before(model, body, index).ok();
    match item {
        MeasuredItem::Point(_) => Form::Point,
        MeasuredItem::Axis(_) => Form::Axis,
        MeasuredItem::Plane(_) => Form::Plane,
        MeasuredItem::Body(_) => Form::Body,
        MeasuredItem::Edge { body, edge } => {
            let form = state(*body).and_then(|solid| {
                let found = edge.resolve(solid).ok()?;
                edge_measure(solid, found)
                    .ok()
                    .map(|measured| measured.form)
            });
            match form {
                Some(EdgeForm::Line { .. }) => Form::Straight,
                Some(EdgeForm::Circle { .. }) => Form::Round,
                Some(EdgeForm::Ellipse { .. } | EdgeForm::Curve) => Form::Curved,
                None => Form::Unknown,
            }
        }
        MeasuredItem::Face { body, face } => {
            let form = state(*body).and_then(|solid| {
                let found = face.resolve(solid).ok()?;
                face_form(solid, found).ok()
            });
            match form {
                Some(FaceForm::Plane { .. }) => Form::Flat,
                Some(FaceForm::Cylinder { .. } | FaceForm::Sphere { .. }) => Form::RoundFace,
                Some(_) => Form::Face,
                None => Form::Unknown,
            }
        }
        MeasuredItem::Sketch { sketch, entity } => {
            let entity = model
                .document()
                .feature(*sketch)
                .and_then(|feature| feature.kind.sketch())
                .and_then(|definition| definition.entity(*entity));
            match entity {
                Some(Entity::Point(_)) => Form::Point,
                Some(Entity::Line { .. }) => Form::Straight,
                Some(Entity::Arc { .. } | Entity::Circle { .. }) => Form::Round,
                Some(_) => Form::Curved,
                None => Form::Unknown,
            }
        }
    }
}

fn of_allows(quantity: Of, form: Form) -> bool {
    form == Form::Unknown
        || match quantity {
            Of::Length => matches!(form, Form::Straight | Form::Round | Form::Curved),
            Of::Radius => matches!(form, Form::Round | Form::RoundFace),
            Of::Sweep => form == Form::Round,
            Of::Area => matches!(form, Form::Flat | Form::RoundFace | Form::Face | Form::Body),
            Of::Perimeter => matches!(form, Form::Flat | Form::RoundFace | Form::Face),
            Of::Volume | Of::Mass => form == Form::Body,
        }
}

fn position_allows(form: Form) -> bool {
    matches!(form, Form::Point | Form::Body | Form::Unknown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    Between(Between),
    Along,
    Of(Of),
    Position,
}

impl Quantity {
    pub fn of(reading: &Reading) -> Self {
        match reading {
            Reading::Between { quantity, .. } => Self::Between(*quantity),
            Reading::Along { .. } => Self::Along,
            Reading::Of { quantity, .. } => Self::Of(*quantity),
            Reading::Position { .. } => Self::Position,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Between(quantity) => quantity.label(),
            Self::Along => ALONG_AN_AXIS,
            Self::Of(quantity) => quantity.label(),
            Self::Position => POSITION_ALONG_AN_AXIS,
        }
    }

    pub fn offered(reading: &Reading) -> Vec<Self> {
        match reading {
            Reading::Between { .. } | Reading::Along { .. } => Between::ALL
                .into_iter()
                .map(Self::Between)
                .chain([Self::Along])
                .collect(),
            Reading::Of { .. } | Reading::Position { .. } => Of::ALL
                .into_iter()
                .map(Self::Of)
                .chain([Self::Position])
                .collect(),
        }
    }
}

fn capitalised(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

fn has_no(document: &Document, item: &MeasuredItem, quantity: Of) -> String {
    format!(
        "{} has no {}",
        capitalised(&item.describe(document)),
        quantity.label().to_lowercase()
    )
}

fn has_no_position(document: &Document, item: &MeasuredItem) -> String {
    format!(
        "{} is not a point or a body, so it has no position",
        capitalised(&item.describe(document))
    )
}

fn principal_x() -> MeasuredItem {
    MeasuredItem::Axis(AxisReference::Principal(PrincipalAxis::X))
}

fn read_as(
    model: &Model,
    reading: &Reading,
    quantity: Quantity,
    index: usize,
) -> Result<Reading, String> {
    let document = model.document();
    let pair = match reading {
        Reading::Between { first, second, .. } | Reading::Along { first, second, .. } => {
            Some((first, second))
        }
        Reading::Of { .. } | Reading::Position { .. } => None,
    };
    match (quantity, pair, reading) {
        (Quantity::Between(Between::Angle), Some((first, second)), _)
            if [first, second]
                .into_iter()
                .any(|item| form_of(model, item, index) == Form::Point) =>
        {
            Err(NO_ANGLE.to_owned())
        }
        (Quantity::Between(quantity), Some((first, second)), _) => Ok(Reading::Between {
            quantity,
            first: first.clone(),
            second: second.clone(),
        }),
        (Quantity::Along, Some((first, second)), _) => {
            let axis = reading
                .measuring_axis()
                .cloned()
                .unwrap_or_else(principal_x);
            Ok(Reading::Along {
                first: first.clone(),
                second: second.clone(),
                axis,
            })
        }
        (
            Quantity::Of(quantity),
            None,
            Reading::Of { item, .. } | Reading::Position { item, .. },
        ) => {
            if of_allows(quantity, form_of(model, item, index)) {
                Ok(Reading::Of {
                    quantity,
                    item: item.clone(),
                })
            } else {
                Err(has_no(document, item, quantity))
            }
        }
        (Quantity::Position, None, Reading::Of { item, .. } | Reading::Position { item, .. }) => {
            if position_allows(form_of(model, item, index)) {
                Ok(Reading::Position {
                    item: item.clone(),
                    axis: reading
                        .measuring_axis()
                        .cloned()
                        .unwrap_or_else(principal_x),
                })
            } else {
                Err(has_no_position(document, item))
            }
        }
        (Quantity::Of(_) | Quantity::Position, _, _) => Err(ONE_ITEM_ONLY.to_owned()),
        (Quantity::Between(_) | Quantity::Along, _, _) => Err(ONE_ITEM_ONLY.to_owned()),
    }
}

pub fn edit(
    document: &Document,
    feature: FeatureId,
    measurement: Measurement,
) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::from(measurement),
        },
    ))
}

pub fn quantity_change(
    model: &Model,
    feature: FeatureId,
    measurement: &Measurement,
    quantity: Quantity,
) -> Result<Transaction, String> {
    let document = model.document();
    let index = document.feature_index(feature).ok_or(GONE)?;
    let reading = read_as(model, &measurement.reading, quantity, index)?;
    let changed = Measurement {
        reading,
        parameter: measurement.parameter,
    };
    let transaction = edit(document, feature, changed).ok_or(GONE)?;
    field::checked(document, transaction)
}

fn with_item(
    model: &Model,
    reading: &Reading,
    part: MeasuredPart,
    item: MeasuredItem,
    index: usize,
) -> Result<Reading, String> {
    let document = model.document();
    let placed = match (reading.clone(), part) {
        (Reading::Of { quantity, .. }, MeasuredPart::First) => {
            let form = form_of(model, &item, index);
            let allowed = [quantity]
                .into_iter()
                .chain(Of::ALL)
                .find(|quantity| of_allows(*quantity, form));
            match allowed {
                Some(quantity) => Reading::Of { quantity, item },
                None if position_allows(form) => Reading::Position {
                    item,
                    axis: principal_x(),
                },
                None => return Err(nothing_to_read(document, &item)),
            }
        }
        (Reading::Position { axis, .. }, MeasuredPart::First) => {
            let form = form_of(model, &item, index);
            if position_allows(form) {
                Reading::Position { item, axis }
            } else {
                let quantity = Of::ALL
                    .into_iter()
                    .find(|quantity| of_allows(*quantity, form))
                    .ok_or_else(|| nothing_to_read(document, &item))?;
                Reading::Of { quantity, item }
            }
        }
        (Reading::Position { item: kept, .. }, MeasuredPart::Axis) => Reading::Position {
            item: kept,
            axis: item,
        },
        (Reading::Of { .. }, MeasuredPart::Second | MeasuredPart::Axis)
        | (Reading::Position { .. }, MeasuredPart::Second) => {
            return Err(ONE_ITEM_ONLY.to_owned());
        }
        (
            Reading::Between {
                quantity, second, ..
            },
            MeasuredPart::First,
        ) => Reading::Between {
            quantity,
            first: item,
            second,
        },
        (
            Reading::Between {
                quantity, first, ..
            },
            MeasuredPart::Second,
        ) => Reading::Between {
            quantity,
            first,
            second: item,
        },
        (Reading::Along { second, axis, .. }, MeasuredPart::First) => Reading::Along {
            first: item,
            second,
            axis,
        },
        (Reading::Along { first, axis, .. }, MeasuredPart::Second) => Reading::Along {
            first,
            second: item,
            axis,
        },
        (Reading::Along { first, second, .. }, MeasuredPart::Axis) => Reading::Along {
            first,
            second,
            axis: item,
        },
        (Reading::Between { .. }, MeasuredPart::Axis) => return Err(NOT_AN_AXIS.to_owned()),
    };
    match read_as(model, &placed, Quantity::of(&placed), index) {
        Err(reason) if reason == NO_ANGLE => {
            read_as(model, &placed, Quantity::Between(Between::Distance), index)
        }
        other => other,
    }
}

fn nothing_to_read(document: &Document, item: &MeasuredItem) -> String {
    format!(
        "{} has no length, radius, sweep, area, perimeter, volume, mass or position; choose an \
         edge, a curve, a face, a point or a centre of mass",
        capitalised(&item.describe(document))
    )
}

pub fn reading_with(
    model: &Model,
    feature: FeatureId,
    measurement: &Measurement,
    part: MeasuredPart,
    item: MeasuredItem,
) -> Result<Transaction, String> {
    let document = model.document();
    let index = document.feature_index(feature).ok_or(GONE)?;
    let reading = with_item(model, &measurement.reading, part, item, index)?;
    if reading == measurement.reading {
        return Err(SAME_ITEM.to_owned());
    }
    let changed = Measurement {
        reading,
        parameter: measurement.parameter,
    };
    let transaction = edit(document, feature, changed).ok_or(GONE)?;
    field::checked(document, transaction)
}

pub fn item_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    measurement: &Measurement,
    part: MeasuredPart,
) -> Result<Transaction, String> {
    let index = model.document().feature_index(feature).ok_or(GONE)?;
    let mut picked = selection.iter();
    let pickable = picked.next().ok_or(CHOOSE_ONE)?;
    if picked.next().is_some() {
        return Err(ONE_ITEM.to_owned());
    }
    let reads_a_body = matches!(part.item(&measurement.reading), Some(MeasuredItem::Body(_)));
    let item = match part {
        MeasuredPart::Axis => MeasuredItem::Axis(
            datum_tools::axis_reference(model, pickable, index).ok_or(NOT_AN_AXIS)?,
        ),
        MeasuredPart::First | MeasuredPart::Second if reads_a_body => {
            body_at(model, pickable, index)?
        }
        MeasuredPart::First | MeasuredPart::Second => item_at(model, pickable, index)?,
    };
    reading_with(model, feature, measurement, part, item)
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
    let label = match reading {
        Reading::Along { .. } => "Keep offset".to_owned(),
        Reading::Between { .. } | Reading::Of { .. } | Reading::Position { .. } => {
            format!("Keep {}", reading.label().to_lowercase())
        }
    };
    let mut transaction = document.transaction(label);
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
        Dimension::VOLUME => units.measured_volume(value.value),
        MASS => mass_text(value.value),
        _ => units.show(value),
    };
    if result.accuracy == Accuracy::Approximate {
        format!("{APPROXIMATELY}{shown}")
    } else {
        shown
    }
}

pub fn current_reading(model: &Model, feature: FeatureId) -> Option<&MeasurementResult> {
    model
        .evaluation()
        .feature(feature)
        .filter(|status| status.state == FeatureState::UpToDate)
        .and_then(|status| status.result.as_deref())
        .and_then(|result| result.measurement())
}

pub struct Shown {
    pub line: Option<[Point3; 2]>,
    pub anchor: Point3,
    pub label: String,
}

pub fn shown(model: &Model) -> Vec<Shown> {
    let document = model.document();
    let units = model.units();
    document
        .active_features()
        .filter(|feature| !feature.hidden)
        .filter_map(|feature| {
            let measurement = feature.kind.measurement()?;
            let result = current_reading(model, feature.id())?;
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

#[cfg(test)]
mod tests {
    use caditor_document::{
        AxisReference, FeatureId, MeasuredItem, Of, PointReference, PrincipalAxis, Reading,
    };

    use super::Keepable;

    #[test]
    fn an_offset_row_keeps_an_offset_along_the_axis_it_was_read_in() {
        let origin = MeasuredItem::Point(PointReference::Origin);
        let items = [origin.clone(), origin.clone()];
        let frame = FeatureId::from_raw(3);
        let along = |axis| Reading::Along {
            first: origin.clone(),
            second: origin.clone(),
            axis: MeasuredItem::Axis(axis),
        };

        let keep = |label, between, frame, items: &[MeasuredItem]| {
            Keepable::of_row(label, between, frame)
                .first()
                .and_then(|kept| kept.reading(items))
        };
        let world = keep("Along Y", true, None, &items);
        let framed = keep("Along Y", true, Some(frame), &items);
        let perimeter = keep("Perimeter", false, None, &items[..1]);

        assert_eq!(
            world,
            Some(along(AxisReference::Principal(PrincipalAxis::Y)))
        );
        assert_eq!(
            framed,
            Some(along(AxisReference::Frame {
                frame,
                axis: PrincipalAxis::Y,
            }))
        );
        assert_eq!(
            perimeter,
            Some(Reading::Of {
                quantity: Of::Perimeter,
                item: origin.clone(),
            })
        );
        assert!(Keepable::of_row("Along Y", false, None).is_empty());
    }

    #[test]
    fn a_position_row_keeps_a_coordinate_along_each_axis_and_a_centroid_row_the_centre_of_mass() {
        let corner = MeasuredItem::Point(PointReference::Origin);
        let body = MeasuredItem::Body(FeatureId::from_raw(2));
        let frame = FeatureId::from_raw(3);

        let framed = Keepable::of_row("Position", false, Some(frame));
        let centroid = Keepable::of_mass_row("Centroid");
        let volume = Keepable::of_mass_row("Volume");

        assert_eq!(framed.len(), 3);
        assert_eq!(
            framed[1].menu_label(),
            "Keep this measurement along Y".to_owned()
        );
        assert_eq!(
            framed[1].reading(std::slice::from_ref(&corner)),
            Some(Reading::Position {
                item: corner.clone(),
                axis: MeasuredItem::Axis(AxisReference::Frame {
                    frame,
                    axis: PrincipalAxis::Y,
                }),
            })
        );
        assert_eq!(
            centroid[2].reading(std::slice::from_ref(&body)),
            Some(Reading::Position {
                item: body.clone(),
                axis: MeasuredItem::Axis(AxisReference::Principal(PrincipalAxis::Z)),
            })
        );
        assert_eq!(
            volume[0].reading(std::slice::from_ref(&body)),
            Some(Reading::Of {
                quantity: Of::Volume,
                item: body.clone(),
            })
        );
        assert!(Keepable::of_mass_row("Size").is_empty());
    }
}
