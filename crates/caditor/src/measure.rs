use std::{
    collections::BTreeSet,
    f64::consts::TAU,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{
    DatumResult, FeatureId, FeatureResult, MeasuredItem, displayed_frame, face_perimeter,
    profile_curve,
};
use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};
use caditor_kernel::{
    Accuracy, AngleKind, Axis, Curve, Curve2, EdgeForm, EdgeId, EdgeMeasure, Element, FaceForm,
    FaceId, Interval, LINEAR_RESOLUTION, MeasureError, Region, Separation, Solid, angle, axis_of,
    axis_separation, curve_measure, distance, edge_measure, face_area, face_form, section_of,
};

use crate::{
    bodies::{self, BodyMass, Converted, MassAccuracy},
    datum_tools, measurement_tools,
    model::{Model, Waker},
    scene,
    selection::{self, Pickable, Selection},
};

pub const TOO_MANY: &str = "Select one or two items to measure between them.";
pub const FAILED: &str = "The measurement could not be worked out for this selection.";
pub const UNMEASURABLE: &str =
    "Only points, edges, faces, sketch curves, sketch regions, planes and axes can be measured.";
pub const APPROXIMATELY: &str = "≈ ";
const FULL_TURN_SLACK: f64 = 1e-9;
const PARALLEL_SINE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Length(f64),
    Area(f64),
    Angle(f64),
    Position(Point3),
    Direction(Vector3),
    SecondMoment(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub label: &'static str,
    pub value: Value,
    pub accuracy: Accuracy,
}

impl Reading {
    fn exact(label: &'static str, value: Value) -> Self {
        Self {
            label,
            value,
            accuracy: Accuracy::Exact,
        }
    }

    fn with(label: &'static str, value: Value, accuracy: Accuracy) -> Self {
        Self {
            label,
            value,
            accuracy,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub title: String,
    pub readings: Vec<Reading>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredLine {
    pub from: Point3,
    pub to: Point3,
    pub accuracy: Accuracy,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Readout {
    pub groups: Vec<Group>,
    pub line: Option<MeasuredLine>,
    pub problem: Option<&'static str>,
    pub kept: Vec<MeasuredItem>,
}

#[derive(Debug, Clone)]
enum Subject {
    Point(Point3),
    Edge {
        result: Arc<FeatureResult>,
        edge: EdgeId,
    },
    Face {
        result: Arc<FeatureResult>,
        face: FaceId,
    },
    Curve {
        curve: Box<Curve>,
        interval: Interval,
    },
    Axis(Axis),
    Plane {
        origin: Point3,
        normal: Vector3,
    },
    CentreOfMass(Point3),
    Regions {
        sketch: FeatureId,
        plane: Plane,
        regions: Vec<Region>,
    },
    Unmeasurable,
}

#[derive(Debug, Clone)]
struct Item {
    name: String,
    subject: Subject,
}

impl Item {
    fn element(&self) -> Option<Element<'_>> {
        match &self.subject {
            Subject::Point(point) => Some(Element::Point(*point)),
            Subject::Edge { result, edge } => Some(Element::Edge {
                solid: &result.solid()?.solid,
                edge: *edge,
            }),
            Subject::Face { result, face } => Some(Element::Face {
                solid: &result.solid()?.solid,
                face: *face,
            }),
            Subject::Curve { curve, interval } => Some(Element::Curve {
                curve,
                interval: *interval,
            }),
            Subject::Axis(axis) => Some(Element::Axis(*axis)),
            Subject::Plane { origin, normal } => Some(Element::Plane {
                origin: *origin,
                normal: *normal,
            }),
            Subject::CentreOfMass(centroid) => Some(Element::Point(*centroid)),
            Subject::Regions { .. } | Subject::Unmeasurable => None,
        }
    }
}

fn body_result(model: &Model, body: FeatureId) -> Option<Arc<FeatureResult>> {
    model.evaluation().body_result(body).cloned()
}

fn shown_face_area(model: &Model, result: &Arc<FeatureResult>, face: FaceId) -> Option<Reading> {
    let exact = match model.display().meshing.lookup(result) {
        Converted::Ready(mesh) => bodies::face_keys(&result.solid()?.solid)
            .into_iter()
            .find(|(id, _)| *id == face)
            .and_then(|(_, key)| mesh.face_area(key)),
        Converted::Pending | Converted::Missing => None,
    };
    Some(match exact {
        Some(area) => Reading::exact("Area", Value::Area(area)),
        None => Reading::with(
            "Area",
            Value::Area(result_mesh_area(result, face)?),
            Accuracy::Approximate,
        ),
    })
}

fn body_mass(model: &Model, body: FeatureId) -> Option<BodyMass> {
    let result = body_result(model, body)?;
    match model.display().meshing.lookup(&result) {
        Converted::Ready(mesh) => mesh.mass().copied(),
        Converted::Pending | Converted::Missing => None,
    }
}

fn subject_of(model: &Model, pickable: Pickable) -> Option<Subject> {
    let evaluation = model.evaluation();
    Some(match pickable {
        Pickable::Origin => Subject::Point(Point3::ZERO),
        Pickable::Vertex { body, vertex } => {
            let shown = bodies::shown(evaluation, body)?;
            let id = bodies::find_vertex(shown, vertex)?;
            Subject::Point(shown.solid.vertex(id)?.point())
        }
        Pickable::Edge { body, edge } => {
            let result = body_result(model, body)?;
            let edge = bodies::find_edge(result.solid()?, edge)?;
            Subject::Edge { result, edge }
        }
        Pickable::Face { body, face } => {
            let result = body_result(model, body)?;
            let face = bodies::find_face(result.solid()?, face)?;
            Subject::Face { result, face }
        }
        Pickable::SketchEntity { feature, entity } => {
            let owner = model.document().feature(feature)?;
            let sketch = model.displayed_sketch(owner)?;
            let plane = scene::sketch_plane(model.document(), evaluation, feature)?;
            match sketch.point(entity) {
                Some(point) => Subject::Point(plane.to_world(point)),
                None => {
                    let (curve, interval) = profile_curve(&sketch, entity)?.curve().ok()?;
                    Subject::Curve {
                        curve: Box::new(curve.on_plane(&plane).ok()?),
                        interval,
                    }
                }
            }
        }
        Pickable::Axis(axis) => {
            let ray = axis.principal().ray()?;
            Subject::Axis(Axis {
                origin: ray.origin(),
                direction: ray.direction(),
            })
        }
        Pickable::Plane(plane) => {
            let plane = plane.plane();
            Subject::Plane {
                origin: plane.origin(),
                normal: plane.normal(),
            }
        }
        Pickable::Datum(feature) => match datum_tools::result(evaluation, feature)? {
            DatumResult::Plane(plane) => Subject::Plane {
                origin: plane.origin(),
                normal: plane.normal(),
            },
            DatumResult::Axis(ray) => Subject::Axis(Axis {
                origin: ray.origin(),
                direction: ray.direction(),
            }),
            DatumResult::Point(point) => Subject::Point(point),
            DatumResult::Frame(frame) => Subject::Point(frame.origin()),
        },
        Pickable::FrameAxis { feature, axis } => {
            let ray = axis.in_frame(&displayed_frame(evaluation, feature)?)?;
            Subject::Axis(Axis {
                origin: ray.origin(),
                direction: ray.direction(),
            })
        }
        Pickable::FramePlane { feature, plane } => {
            let plane = plane.in_frame(&displayed_frame(evaluation, feature)?)?;
            Subject::Plane {
                origin: plane.origin(),
                normal: plane.normal(),
            }
        }
        Pickable::CentreOfMass(body) => {
            Subject::CentreOfMass(body_mass(model, body)?.properties.centroid)
        }
        Pickable::SketchRegion { feature, region } => {
            let found = selection::sketch_regions(evaluation, feature)?
                .iter()
                .find(|candidate| candidate.region.key() == region)?;
            Subject::Regions {
                sketch: feature,
                plane: scene::sketch_plane(model.document(), evaluation, feature)?,
                regions: vec![found.region.clone()],
            }
        }
        Pickable::SketchConstraint { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. } => Subject::Unmeasurable,
    })
}

pub fn point_of(model: &Model, pickable: Pickable) -> Option<Point3> {
    match subject_of(model, pickable)? {
        Subject::Point(point) | Subject::CentreOfMass(point) => Some(point),
        _ => None,
    }
}

pub fn direction_of(model: &Model, pickable: Pickable) -> Option<Vector3> {
    let item = Item {
        name: String::new(),
        subject: subject_of(model, pickable)?,
    };
    let element = item.element()?;
    let measured = match element {
        Element::Edge { solid, edge } => edge_measure(solid, edge).ok(),
        Element::Curve { curve, interval } => Some(curve_measure(curve, interval)),
        Element::Point(_) | Element::Face { .. } | Element::Axis(_) | Element::Plane { .. } => None,
    };
    if let Some(EdgeMeasure {
        form: EdgeForm::Line { start, end },
        ..
    }) = measured
    {
        return Some((end - start).normalize_or_zero());
    }
    if let Some(axis) = axis_of(element).ok().flatten() {
        return Some(axis.direction);
    }
    plane_of(element).ok().flatten().map(|(_, normal)| normal)
}

pub fn size_text(model: &Model, pickable: Pickable) -> Option<String> {
    let subject = subject_of(model, pickable)?;
    let unit = model.length_unit();
    let parts = match &subject {
        Subject::Face { result, face } => {
            let solid = &result.solid()?.solid;
            let area = shown_face_area(model, result, *face)?;
            face_parts(model, solid, *face, &area)
        }
        Subject::Edge { result, edge } => {
            curve_parts(model, edge_measure(&result.solid()?.solid, *edge).ok()?)
        }
        Subject::Curve { curve, interval } => curve_parts(model, curve_measure(curve, *interval)),
        Subject::Regions { regions, .. } => {
            let section = section_of(regions)?;
            let approximately = approximately(section.accuracy);
            let area = format!("Area {approximately}{}", unit.measured_area(section.area));
            match regions.as_slice() {
                [region] => outline_parts(model, &region_sides(region))
                    .map(|mut parts| {
                        parts.push(area.clone());
                        parts
                    })
                    .unwrap_or_else(|| {
                        vec![
                            area.clone(),
                            format!(
                                "Perimeter {approximately}{}",
                                unit.measured_length(section.perimeter)
                            ),
                        ]
                    }),
                _ => vec![
                    area,
                    format!(
                        "Perimeter {approximately}{}",
                        unit.measured_length(section.perimeter)
                    ),
                ],
            }
        }
        _ => return None,
    };
    (!parts.is_empty()).then(|| parts.join(SIZE_SEPARATOR))
}

const SIZE_SEPARATOR: &str = "  ·  ";
const RIGHT_ANGLE_SLACK: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
enum OutlineSide {
    Line { start: Point3, end: Point3 },
    Arc { radius: f64, sweep: f64 },
    Other,
}

fn approximately(accuracy: Accuracy) -> &'static str {
    if accuracy == Accuracy::Approximate {
        APPROXIMATELY
    } else {
        ""
    }
}

fn circle_parts(model: &Model, radius: f64) -> Vec<String> {
    let unit = model.length_unit();
    vec![
        format!("Ø {}", unit.measured_length(2.0 * radius)),
        format!("R {}", unit.measured_length(radius)),
        format!("Circumference {}", unit.measured_length(TAU * radius)),
    ]
}

fn curve_parts(model: &Model, measured: EdgeMeasure) -> Vec<String> {
    let unit = model.length_unit();
    let length = format!(
        "Length {}{}",
        approximately(measured.length_accuracy),
        unit.measured_length(measured.length)
    );
    match measured.form {
        EdgeForm::Circle { radius, sweep, .. } if sweep >= TAU - FULL_TURN_SLACK => {
            circle_parts(model, radius)
        }
        EdgeForm::Circle { radius, sweep, .. } => vec![
            format!("R {}", unit.measured_length(radius)),
            length,
            format!(
                "Sweep {}",
                model.units().angle.readout_text(sweep.to_degrees())
            ),
        ],
        EdgeForm::Ellipse {
            major_radius,
            minor_radius,
            ..
        } => vec![
            format!(
                "Radii {} × {}",
                unit.measured_length(major_radius),
                unit.measured_length(minor_radius)
            ),
            length,
        ],
        EdgeForm::Line { .. } | EdgeForm::Curve => vec![length],
    }
}

fn outline_parts(model: &Model, sides: &[OutlineSide]) -> Option<Vec<String>> {
    let unit = model.length_unit();
    match sides {
        [OutlineSide::Arc { radius, sweep }] if *sweep >= TAU - FULL_TURN_SLACK => {
            Some(circle_parts(model, *radius))
        }
        [first, second, third, fourth] => {
            let directions: Option<Vec<(Vector3, f64)>> = [first, second, third, fourth]
                .into_iter()
                .map(|side| match side {
                    OutlineSide::Line { start, end } => {
                        let along = *end - *start;
                        Some((along.try_normalize()?, along.length()))
                    }
                    OutlineSide::Arc { .. } | OutlineSide::Other => None,
                })
                .collect();
            let directions = directions?;
            let square = directions
                .iter()
                .zip(directions.iter().cycle().skip(1))
                .all(|((a, _), (b, _))| a.dot(*b).abs() <= RIGHT_ANGLE_SLACK);
            let [(_, width), (_, height), ..] = directions.as_slice() else {
                return None;
            };
            square.then(|| {
                vec![format!(
                    "{} × {}",
                    unit.measured_length(width.max(*height)),
                    unit.measured_length(width.min(*height))
                )]
            })
        }
        _ => None,
    }
}

fn region_sides(region: &Region) -> Vec<OutlineSide> {
    region
        .outer()
        .pieces()
        .iter()
        .map(|piece| {
            let range = piece.range();
            let point = |at: f64| {
                let point = piece.curve().point(at);
                Point3::new(point.x, point.y, 0.0)
            };
            match piece.curve() {
                Curve2::Line(_) => OutlineSide::Line {
                    start: point(range.start()),
                    end: point(range.end()),
                },
                Curve2::Circle(circle) => OutlineSide::Arc {
                    radius: circle.radius(),
                    sweep: piece.curve().length(range) / circle.radius(),
                },
                _ => OutlineSide::Other,
            }
        })
        .collect()
}

fn face_sides(solid: &Solid, face: FaceId) -> Vec<OutlineSide> {
    let Some(outer) = solid
        .face(face)
        .and_then(|face| face.loops().first().copied())
        .and_then(|id| solid.face_loop(id))
    else {
        return Vec::new();
    };
    outer
        .coedges()
        .iter()
        .filter_map(|coedge| solid.coedge(*coedge).map(|coedge| coedge.edge()))
        .filter(|edge| !bodies::is_seam(solid, *edge))
        .map(
            |edge| match edge_measure(solid, edge).map(|measured| measured.form) {
                Ok(EdgeForm::Line { start, end }) => OutlineSide::Line { start, end },
                Ok(EdgeForm::Circle { radius, sweep, .. }) => OutlineSide::Arc { radius, sweep },
                Ok(EdgeForm::Ellipse { .. } | EdgeForm::Curve) | Err(_) => OutlineSide::Other,
            },
        )
        .collect()
}

fn face_parts(model: &Model, solid: &Solid, face: FaceId, area: &Reading) -> Vec<String> {
    let unit = model.length_unit();
    let area_text = match area.value {
        Value::Area(value) => Some(format!(
            "Area {}{}",
            approximately(area.accuracy),
            unit.measured_area(value)
        )),
        _ => None,
    };
    let mut parts = match face_form(solid, face) {
        Ok(FaceForm::Cylinder { radius, .. }) => vec![
            format!("Ø {}", unit.measured_length(2.0 * radius)),
            format!("R {}", unit.measured_length(radius)),
        ],
        Ok(FaceForm::Sphere { radius, .. }) => vec![
            format!("Ø {}", unit.measured_length(2.0 * radius)),
            format!("R {}", unit.measured_length(radius)),
        ],
        Ok(FaceForm::Cone { half_angle, .. }) => vec![format!(
            "Half angle {}",
            model.units().angle.readout_text(half_angle.to_degrees())
        )],
        Ok(FaceForm::Torus {
            major_radius,
            minor_radius,
            ..
        }) => vec![
            format!("Ring R {}", unit.measured_length(major_radius)),
            format!("Tube R {}", unit.measured_length(minor_radius)),
        ],
        Ok(FaceForm::Plane { .. }) => {
            let sides = face_sides(solid, face);
            outline_parts(model, &sides).unwrap_or_else(|| {
                let perimeter: Option<f64> = face_boundary_length(solid, face);
                perimeter
                    .map(|length| vec![format!("Perimeter {}", unit.measured_length(length))])
                    .unwrap_or_default()
            })
        }
        Ok(FaceForm::Surface) | Err(_) => Vec::new(),
    };
    parts.extend(area_text);
    parts
}

fn face_boundary_length(solid: &Solid, face: FaceId) -> Option<f64> {
    let outer = solid
        .face(face)?
        .loops()
        .first()
        .and_then(|id| solid.face_loop(*id))?;
    outer
        .coedges()
        .iter()
        .filter_map(|coedge| solid.coedge(*coedge).map(|coedge| coedge.edge()))
        .filter(|edge| !bodies::is_seam(solid, *edge))
        .map(|edge| {
            edge_measure(solid, edge)
                .ok()
                .map(|measured| measured.length)
        })
        .sum()
}

pub fn body_size_text(model: &Model, body: FeatureId) -> Option<String> {
    let mass = body_mass(model, body)?;
    let text = model.units().measured_size(mass.size?);
    let approximately = if mass.accuracy == MassAccuracy::Exact {
        ""
    } else {
        APPROXIMATELY
    };
    Some(format!("Size {approximately}{text}"))
}

fn items_of(model: &Model, selection: &Selection) -> Vec<Item> {
    let document = model.document();
    let mut items: Vec<Item> = Vec::new();
    for pickable in selection.iter() {
        let name = || pickable.describe(document, model.evaluation());
        match subject_of(model, pickable).unwrap_or(Subject::Unmeasurable) {
            Subject::Regions {
                sketch,
                plane,
                regions,
            } => match regions_of_sketch(&mut items, sketch) {
                Some(gathered) => gathered.extend(regions),
                None => items.push(Item {
                    name: name(),
                    subject: Subject::Regions {
                        sketch,
                        plane,
                        regions,
                    },
                }),
            },
            subject => items.push(Item {
                name: name(),
                subject,
            }),
        }
    }
    for item in &mut items {
        if let Subject::Regions {
            sketch, regions, ..
        } = &item.subject
            && regions.len() > 1
        {
            let sketch = document
                .feature(*sketch)
                .map_or("the sketch", |feature| feature.name.as_str());
            item.name = format!("{sketch} › {} regions", regions.len());
        }
    }
    items
}

fn kept_items(model: &Model, selection: &Selection) -> Vec<MeasuredItem> {
    selection
        .iter()
        .map(|pickable| measurement_tools::item_of(model, pickable))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

fn regions_of_sketch(items: &mut [Item], sketch: FeatureId) -> Option<&mut Vec<Region>> {
    items.iter_mut().find_map(|item| match &mut item.subject {
        Subject::Regions {
            sketch: gathered,
            regions,
            ..
        } if *gathered == sketch => Some(regions),
        _ => None,
    })
}

fn item_count(selection: &Selection) -> usize {
    let mut sketches = BTreeSet::new();
    selection
        .iter()
        .filter(|pickable| match pickable {
            Pickable::SketchRegion { feature, .. } => sketches.insert(*feature),
            _ => true,
        })
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Relative {
    to_local: RigidTransform,
}

impl Relative {
    pub const WORLD: Self = Self {
        to_local: RigidTransform::IDENTITY,
    };

    pub fn to(frame: &Plane) -> Option<Self> {
        Some(Self {
            to_local: RigidTransform::from_frame(frame)?.inverse(),
        })
    }

    fn position(&self, point: Point3) -> Point3 {
        self.to_local.apply_point(point)
    }

    fn direction(&self, direction: Vector3) -> Vector3 {
        self.to_local.apply_vector(direction)
    }

    fn reading(&self, reading: Reading) -> Reading {
        let value = match reading.value {
            Value::Position(point) => Value::Position(self.position(point)),
            Value::Direction(direction) => Value::Direction(self.direction(direction)),
            value @ (Value::Length(_)
            | Value::Area(_)
            | Value::Angle(_)
            | Value::SecondMoment(_)) => value,
        };
        Reading { value, ..reading }
    }
}

fn read(items: &[Item], relative: Relative) -> Readout {
    if items.len() > 2 {
        return too_many();
    }
    let mut readout = Readout {
        groups: items
            .iter()
            .map(|item| {
                let mut group = describe(item);
                group.readings = group
                    .readings
                    .into_iter()
                    .map(|reading| relative.reading(reading))
                    .collect();
                group
            })
            .collect(),
        ..Readout::default()
    };
    if let [first, second] = items
        && let (Some(first), Some(second)) = (first.element(), second.element())
    {
        match between(first, second, relative) {
            Ok((group, line)) => {
                readout.groups.push(group);
                readout.line = line;
            }
            Err(error) => {
                log::warn!("measuring between two items failed: {error}");
                readout.problem = Some(FAILED);
            }
        }
    }
    readout
}

fn too_many() -> Readout {
    Readout {
        problem: Some(TOO_MANY),
        ..Readout::default()
    }
}

fn describe(item: &Item) -> Group {
    let (readings, problem) = match (&item.subject, item.element()) {
        (Subject::Regions { plane, regions, .. }, _) => match section_readings(plane, regions) {
            Some(readings) => (readings, None),
            None => {
                log::warn!("measuring {} found no area", item.name);
                (Vec::new(), Some(FAILED.to_owned()))
            }
        },
        (Subject::Unmeasurable, _) | (_, None) => (Vec::new(), Some(UNMEASURABLE.to_owned())),
        (_, Some(element)) => match readings_of(item, element) {
            Ok(readings) => (readings, None),
            Err(error) => {
                log::warn!("measuring {} failed: {error}", item.name);
                (Vec::new(), Some(FAILED.to_owned()))
            }
        },
    };
    Group {
        title: item.name.clone(),
        readings,
        problem,
    }
}

fn readings_of(item: &Item, element: Element<'_>) -> Result<Vec<Reading>, MeasureError> {
    Ok(match element {
        Element::Point(point) => vec![Reading::exact("Position", Value::Position(point))],
        Element::Edge { solid, edge } => curve_readings(edge_measure(solid, edge)?),
        Element::Curve { curve, interval } => curve_readings(curve_measure(curve, interval)),
        Element::Axis(axis) => vec![
            Reading::exact("Through", Value::Position(axis.origin)),
            Reading::exact("Direction", Value::Direction(axis.direction)),
        ],
        Element::Plane { origin, normal } => vec![
            Reading::exact("Through", Value::Position(origin)),
            Reading::exact("Normal", Value::Direction(normal)),
        ],
        Element::Face { solid, face } => {
            let mut readings = Vec::new();
            match face_area(solid, face)? {
                Some(area) => readings.push(Reading::exact("Area", Value::Area(area))),
                None => {
                    if let Some(area) = mesh_area(item, face) {
                        readings.push(Reading::with(
                            "Area",
                            Value::Area(area),
                            Accuracy::Approximate,
                        ));
                    }
                }
            }
            if let Some((length, accuracy)) = face_perimeter(solid, face) {
                readings.push(Reading::with("Perimeter", Value::Length(length), accuracy));
            }
            match face_form(solid, face)? {
                FaceForm::Cylinder { radius, .. } => readings.extend([
                    Reading::exact("Radius", Value::Length(radius)),
                    Reading::exact("Diameter", Value::Length(2.0 * radius)),
                ]),
                FaceForm::Sphere { center, radius } => readings.extend([
                    Reading::exact("Radius", Value::Length(radius)),
                    Reading::exact("Centre", Value::Position(center)),
                ]),
                FaceForm::Cone {
                    apex, half_angle, ..
                } => readings.extend([
                    Reading::exact("Half angle", Value::Angle(half_angle)),
                    Reading::exact("Apex", Value::Position(apex)),
                ]),
                FaceForm::Torus {
                    major_radius,
                    minor_radius,
                    ..
                } => readings.extend([
                    Reading::exact("Ring radius", Value::Length(major_radius)),
                    Reading::exact("Tube radius", Value::Length(minor_radius)),
                ]),
                FaceForm::Plane { .. } | FaceForm::Surface => {}
            }
            readings
        }
    })
}

fn section_readings(plane: &Plane, regions: &[Region]) -> Option<Vec<Reading>> {
    let section = section_of(regions)?;
    let accuracy = section.accuracy;
    let moments = section.moments;
    let principal = moments.principal();
    let reading = |label, value| Reading::with(label, value, accuracy);
    let mut readings = vec![
        reading("Area", Value::Area(section.area)),
        reading("Perimeter", Value::Length(section.perimeter)),
        reading(
            "Centroid",
            Value::Position(plane.to_world(section.centroid)),
        ),
        reading(
            "Ix about the centroid",
            Value::SecondMoment(moments.about_x),
        ),
        reading(
            "Iy about the centroid",
            Value::SecondMoment(moments.about_y),
        ),
        reading(
            "Ixy about the centroid",
            Value::SecondMoment(moments.product),
        ),
        reading("Polar moment J", Value::SecondMoment(moments.polar())),
        reading("Principal moment I1", Value::SecondMoment(principal.major)),
        reading("Principal moment I2", Value::SecondMoment(principal.minor)),
    ];
    if let Some(turn) = principal.angle {
        let (sin, cos) = turn.sin_cos();
        readings.extend([
            reading("Principal angle from x", Value::Angle(turn)),
            reading(
                "Principal axis of I1",
                Value::Direction(plane.x_axis() * cos + plane.y_axis() * sin),
            ),
        ]);
    }
    Some(readings)
}

fn curve_readings(measured: EdgeMeasure) -> Vec<Reading> {
    let mut readings = vec![Reading::with(
        "Length",
        Value::Length(measured.length),
        measured.length_accuracy,
    )];
    match measured.form {
        EdgeForm::Circle {
            center,
            radius,
            sweep,
            ..
        } => {
            readings.extend([
                Reading::exact("Radius", Value::Length(radius)),
                Reading::exact("Diameter", Value::Length(2.0 * radius)),
                Reading::exact("Centre", Value::Position(center)),
            ]);
            if sweep < TAU - FULL_TURN_SLACK {
                readings.push(Reading::exact("Sweep", Value::Angle(sweep)));
            }
        }
        EdgeForm::Ellipse { center, .. } => {
            readings.push(Reading::exact("Centre", Value::Position(center)));
        }
        EdgeForm::Line { .. } | EdgeForm::Curve => {}
    }
    readings
}

fn mesh_area(item: &Item, face: FaceId) -> Option<f64> {
    let Subject::Face { result, .. } = &item.subject else {
        return None;
    };
    result_mesh_area(result, face)
}

fn result_mesh_area(result: &FeatureResult, face: FaceId) -> Option<f64> {
    let mesh = result.solid()?.mesh()?;
    Some(
        mesh.mass_properties_where(|candidate| candidate == face)
            .area,
    )
}

fn between(
    first: Element<'_>,
    second: Element<'_>,
    relative: Relative,
) -> Result<(Group, Option<MeasuredLine>), MeasureError> {
    let closest = distance(first, second)?;
    let mut readings = vec![Reading::with(
        "Distance",
        Value::Length(closest.distance),
        closest.accuracy,
    )];
    let offset = relative.direction(closest.offset());
    readings.extend([
        Reading::with("Along X", Value::Length(offset.x.abs()), closest.accuracy),
        Reading::with("Along Y", Value::Length(offset.y.abs()), closest.accuracy),
        Reading::with("Along Z", Value::Length(offset.z.abs()), closest.accuracy),
    ]);
    readings.extend(centres_and_axes(first, second)?);
    readings.extend(plane_gap(first, second)?);
    if let Some(found) = angle(first, second)? {
        let label = match found.kind {
            AngleKind::Corner => "Angle at the corner",
            AngleKind::Lines => "Angle between the lines",
            AngleKind::Planes => "Angle between the planes",
            AngleKind::LineAndPlane => "Angle to the plane",
        };
        readings.push(Reading::exact(label, Value::Angle(found.radians)));
    }
    let line = (closest.distance > LINEAR_RESOLUTION).then_some(MeasuredLine {
        from: closest.from,
        to: closest.to,
        accuracy: closest.accuracy,
    });
    Ok((
        Group {
            title: "Between them".to_owned(),
            readings,
            problem: None,
        },
        line,
    ))
}

fn circle_center(element: Element<'_>) -> Result<Option<Point3>, MeasureError> {
    let measured = match element {
        Element::Edge { solid, edge } => edge_measure(solid, edge)?,
        Element::Curve { curve, interval } => curve_measure(curve, interval),
        Element::Point(_) | Element::Face { .. } | Element::Axis(_) | Element::Plane { .. } => {
            return Ok(None);
        }
    };
    Ok(match measured.form {
        EdgeForm::Circle { center, .. } => Some(center),
        _ => None,
    })
}

fn centres_and_axes(first: Element<'_>, second: Element<'_>) -> Result<Vec<Reading>, MeasureError> {
    if let (Some(from), Some(to)) = (circle_center(first)?, circle_center(second)?) {
        return Ok(vec![Reading::exact(
            "Centre to centre",
            Value::Length(from.distance(to)),
        )]);
    }
    let (Some(first), Some(second)) = (axis_of(first)?, axis_of(second)?) else {
        return Ok(Vec::new());
    };
    let apart: Separation = axis_separation(first, second);
    let mut readings = vec![Reading::exact(
        "Axis to axis",
        Value::Length(apart.distance),
    )];
    let turn = first
        .direction
        .cross(second.direction)
        .length()
        .atan2(first.direction.dot(second.direction).abs());
    if turn > PARALLEL_SINE {
        readings.push(Reading::exact("Angle between the axes", Value::Angle(turn)));
    }
    Ok(readings)
}

fn plane_of(element: Element<'_>) -> Result<Option<(Point3, Vector3)>, MeasureError> {
    Ok(match element {
        Element::Face { solid, face } => match face_form(solid, face)? {
            FaceForm::Plane { origin, normal } => Some((origin, normal)),
            _ => None,
        },
        Element::Plane { origin, normal } => Some((origin, normal)),
        Element::Point(_) | Element::Edge { .. } | Element::Curve { .. } | Element::Axis(_) => None,
    })
}

fn plane_gap(first: Element<'_>, second: Element<'_>) -> Result<Option<Reading>, MeasureError> {
    let (Some((origin, normal)), Some((other_origin, other_normal))) =
        (plane_of(first)?, plane_of(second)?)
    else {
        return Ok(None);
    };
    let parallel = normal.cross(other_normal).length() <= PARALLEL_SINE;
    Ok(parallel.then(|| {
        Reading::exact(
            "Gap between the planes",
            Value::Length((other_origin - origin).dot(normal).abs()),
        )
    }))
}

fn measure(items: &[Item], relative: Relative) -> Readout {
    panic::catch_unwind(AssertUnwindSafe(|| read(items, relative))).unwrap_or_else(|_| {
        log::error!("measuring the selection panicked");
        Readout {
            problem: Some(FAILED),
            ..Readout::default()
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    selection: u64,
    revision: u64,
    evaluation: u64,
    relative: Relative,
    masses: u64,
}

struct Job {
    ticket: u64,
    items: Vec<Item>,
    kept: Vec<MeasuredItem>,
    relative: Relative,
}

impl Job {
    fn run(self) -> (u64, Readout) {
        let readout = Readout {
            kept: self.kept,
            ..measure(&self.items, self.relative)
        };
        (self.ticket, readout)
    }
}

struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(u64, Readout)>,
}

impl Worker {
    fn spawn(wake: Waker) -> Option<Self> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let (sender, done) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("measure".to_owned())
            .spawn(move || {
                while let Ok(mut job) = queue.recv() {
                    while let Ok(newer) = queue.try_recv() {
                        job = newer;
                    }
                    if sender.send(job.run()).is_err() {
                        break;
                    }
                    wake();
                }
            });
        match spawned {
            Ok(_) => Some(Self { jobs, done }),
            Err(error) => {
                log::error!("could not start the measure worker: {error}");
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Current,
    Stale,
}

#[derive(Default)]
pub struct Measurements {
    worker: Option<Worker>,
    basis: Option<Basis>,
    ticket: u64,
    readout: Option<Readout>,
    previous: Option<Readout>,
}

impl Measurements {
    pub fn refresh(&mut self, model: &Model, selection: &Selection, relative: Relative) {
        let basis = Basis {
            selection: selection.generation(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            relative,
            masses: model.masses_measured(),
        };
        if self.basis.as_ref() != Some(&basis) {
            self.basis = Some(basis);
            self.restart();
            match item_count(selection) {
                0 => self.arrive(self.ticket, Readout::default()),
                1 | 2 => self.submit(
                    model,
                    (items_of(model, selection), kept_items(model, selection)),
                    relative,
                ),
                _ => self.arrive(self.ticket, too_many()),
            }
        }
        self.poll();
    }

    fn restart(&mut self) {
        self.ticket = self.ticket.wrapping_add(1);
        if let Some(current) = self.readout.take() {
            self.previous = Some(current);
        }
    }

    fn arrive(&mut self, ticket: u64, readout: Readout) {
        if ticket == self.ticket {
            self.readout = Some(readout);
            self.previous = None;
        }
    }

    fn submit(
        &mut self,
        model: &Model,
        (items, kept): (Vec<Item>, Vec<MeasuredItem>),
        relative: Relative,
    ) {
        if self.worker.is_none() {
            self.worker = Worker::spawn(model.waker());
        }
        let job = Job {
            ticket: self.ticket,
            items,
            kept,
            relative,
        };
        let refused = match &self.worker {
            Some(worker) => worker.jobs.send(job).err().map(|refused| refused.0),
            None => Some(job),
        };
        if let Some(job) = refused {
            log::error!("no measure worker, so the selection is measured on the UI thread");
            self.worker = None;
            let (ticket, readout) = job.run();
            self.arrive(ticket, readout);
        }
    }

    fn poll(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        let arrived: Vec<(u64, Readout)> = worker.done.try_iter().collect();
        for (ticket, readout) in arrived {
            self.arrive(ticket, readout);
        }
    }

    pub fn readout(&self) -> Option<&Readout> {
        self.readout.as_ref()
    }

    pub fn shown(&self) -> Option<(&Readout, Freshness)> {
        self.readout
            .as_ref()
            .map(|readout| (readout, Freshness::Current))
            .or_else(|| {
                self.previous
                    .as_ref()
                    .map(|readout| (readout, Freshness::Stale))
            })
    }

    pub fn is_measuring(&self) -> bool {
        self.basis.is_some() && self.readout.is_none()
    }

    pub fn forget(&mut self) {
        self.basis = None;
        self.readout = None;
        self.previous = None;
    }
}

#[derive(Default)]
pub struct MeasureTool {
    pub open: bool,
    pub measurements: Measurements,
    pub relative_to: Option<FeatureId>,
}

impl MeasureTool {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.measurements.forget();
        }
    }
}

#[cfg(test)]
mod tests {
    use caditor_kernel::Accuracy;

    use super::*;

    fn point(x: f64, y: f64, z: f64) -> Item {
        Item {
            name: format!("Point at {x}"),
            subject: Subject::Point(Point3::new(x, y, z)),
        }
    }

    #[test]
    fn two_points_read_their_distance_and_offsets_along_each_axis() {
        let readout = measure(
            &[point(1.0, 2.0, 3.0), point(4.0, 6.0, 3.0)],
            Relative::WORLD,
        );

        let between = readout.groups.last().unwrap();
        assert_eq!(between.title, "Between them");
        assert_eq!(
            between.readings.first(),
            Some(&Reading::exact("Distance", Value::Length(5.0)))
        );
        assert!(
            between
                .readings
                .contains(&Reading::exact("Along Y", Value::Length(4.0)))
        );
        assert!(
            between
                .readings
                .iter()
                .all(|reading| reading.accuracy == Accuracy::Exact)
        );
        assert_eq!(
            readout.line.map(|line| line.to),
            Some(Point3::new(4.0, 6.0, 3.0))
        );
    }

    #[test]
    fn three_items_are_not_read_but_asked_to_be_fewer() {
        let readout = measure(
            &[
                point(0.0, 0.0, 0.0),
                point(1.0, 0.0, 0.0),
                point(2.0, 0.0, 0.0),
            ],
            Relative::WORLD,
        );

        assert!(readout.groups.is_empty());
        assert_eq!(readout.problem, Some(TOO_MANY));
        assert_eq!(readout.line, None);
    }

    #[test]
    fn the_last_readout_stays_shown_as_stale_until_the_new_one_arrives() {
        let first = measure(&[point(0.0, 0.0, 0.0)], Relative::WORLD);
        let second = measure(
            &[point(1.0, 0.0, 0.0), point(2.0, 0.0, 0.0)],
            Relative::WORLD,
        );
        let mut measurements = Measurements::default();
        measurements.restart();
        let started = measurements.ticket;
        measurements.arrive(started, first.clone());

        measurements.restart();
        let stale = measurements
            .shown()
            .map(|(readout, freshness)| (readout.clone(), freshness));
        let line_while_stale = measurements.readout().cloned();
        measurements.arrive(started, Readout::default());
        let after_outdated = measurements.shown().map(|(_, freshness)| freshness);
        measurements.arrive(measurements.ticket, second.clone());

        assert_eq!(stale, Some((first, Freshness::Stale)));
        assert_eq!(line_while_stale, None);
        assert_eq!(after_outdated, Some(Freshness::Stale));
        assert_eq!(measurements.shown(), Some((&second, Freshness::Current)));
        assert_eq!(measurements.readout(), Some(&second));
    }

    #[test]
    fn an_item_that_cannot_be_measured_says_so() {
        let readout = measure(
            &[Item {
                name: "Region".to_owned(),
                subject: Subject::Unmeasurable,
            }],
            Relative::WORLD,
        );

        assert!(readout.groups[0].problem.is_some());
        assert!(readout.groups[0].readings.is_empty());
    }
}
