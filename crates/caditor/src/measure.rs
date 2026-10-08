use std::{
    f64::consts::TAU,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_document::{DatumResult, FeatureId, FeatureResult, profile_curve};
use caditor_geometry::{Point3, Vector3};
use caditor_kernel::{
    Accuracy, AngleKind, Axis, Curve, EdgeForm, EdgeId, EdgeMeasure, Element, FaceForm, FaceId,
    Interval, LINEAR_RESOLUTION, MeasureError, Separation, angle, axis_of, axis_separation,
    curve_measure, distance, edge_measure, face_form, planar_area,
};

use crate::{
    bodies::{self, BodyMass, MassAccuracy},
    datum_tools,
    model::{Model, Waker},
    scene,
    selection::{Pickable, Selection},
};

pub const TOO_MANY: &str = "Select one or two items to measure between them.";
pub const FAILED: &str = "The measurement could not be worked out for this selection.";
pub const UNMEASURABLE: &str =
    "Only points, edges, faces, sketch curves, planes and axes can be measured.";
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
    CentreOfMass(Arc<FeatureResult>),
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
            Subject::CentreOfMass(result) => Some(Element::Point(
                result.solid()?.mesh()?.mass_properties().centroid,
            )),
            Subject::Unmeasurable => None,
        }
    }
}

fn body_result(model: &Model, body: FeatureId) -> Option<Arc<FeatureResult>> {
    model.evaluation().body_result(body).cloned()
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
        },
        Pickable::CentreOfMass(body) => Subject::CentreOfMass(body_result(model, body)?),
        Pickable::SketchConstraint { .. }
        | Pickable::SketchRegion { .. }
        | Pickable::Region { .. }
        | Pickable::BlendEdge { .. }
        | Pickable::ShellFace { .. } => Subject::Unmeasurable,
    })
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
    let item = Item {
        name: String::new(),
        subject: subject_of(model, pickable)?,
    };
    let (label, readings) = match &item.subject {
        Subject::Face { .. } => ("Area", readings_of(&item, item.element()?).ok()?),
        Subject::Edge { .. } => ("Length", readings_of(&item, item.element()?).ok()?),
        _ => return None,
    };
    let reading = readings
        .into_iter()
        .find(|reading| reading.label == label)?;
    let units = model.units();
    let text = match reading.value {
        Value::Area(area) => units.measured_area(area),
        Value::Length(length) => units.measured_length(length),
        _ => return None,
    };
    let approximately = if reading.accuracy == Accuracy::Approximate {
        APPROXIMATELY
    } else {
        ""
    };
    Some(format!("{label} {approximately}{text}"))
}

pub fn body_size_text(model: &Model, body: FeatureId) -> Option<String> {
    let result = body_result(model, body)?;
    let solid = result.solid()?;
    let mass = BodyMass::of(&solid.solid, solid.mesh()?);
    let text = model.units().measured_size(mass.size?);
    let approximately = if mass.accuracy == MassAccuracy::Exact {
        ""
    } else {
        APPROXIMATELY
    };
    Some(format!("Size {approximately}{text}"))
}

fn items_of(model: &Model, selection: &Selection) -> Vec<Item> {
    selection
        .iter()
        .map(|pickable| Item {
            name: pickable.describe(model.document(), model.evaluation()),
            subject: subject_of(model, pickable).unwrap_or(Subject::Unmeasurable),
        })
        .collect()
}

fn read(items: &[Item]) -> Readout {
    if items.len() > 2 {
        return too_many();
    }
    let mut readout = Readout {
        groups: items.iter().map(describe).collect(),
        ..Readout::default()
    };
    if let [first, second] = items
        && let (Some(first), Some(second)) = (first.element(), second.element())
    {
        match between(first, second) {
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
            match planar_area(solid, face)? {
                Some((area, accuracy)) => {
                    readings.push(Reading::with("Area", Value::Area(area), accuracy));
                }
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
    let mesh = result.solid()?.mesh()?;
    Some(
        mesh.mass_properties_where(|candidate| candidate == face)
            .area,
    )
}

fn between(
    first: Element<'_>,
    second: Element<'_>,
) -> Result<(Group, Option<MeasuredLine>), MeasureError> {
    let closest = distance(first, second)?;
    let mut readings = vec![Reading::with(
        "Distance",
        Value::Length(closest.distance),
        closest.accuracy,
    )];
    let offset = closest.offset();
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

fn measure(items: &[Item]) -> Readout {
    panic::catch_unwind(AssertUnwindSafe(|| read(items))).unwrap_or_else(|_| {
        log::error!("measuring the selection panicked");
        Readout {
            groups: Vec::new(),
            line: None,
            problem: Some(FAILED),
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    selection: u64,
    revision: u64,
    evaluation: u64,
}

struct Job {
    ticket: u64,
    items: Vec<Item>,
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
                    if sender.send((job.ticket, measure(&job.items))).is_err() {
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
    pub fn refresh(&mut self, model: &Model, selection: &Selection) {
        let basis = Basis {
            selection: selection.generation(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
        };
        if self.basis.as_ref() != Some(&basis) {
            self.basis = Some(basis);
            self.restart();
            match selection.len() {
                0 => self.arrive(self.ticket, Readout::default()),
                1 | 2 => self.submit(model, items_of(model, selection)),
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

    fn submit(&mut self, model: &Model, items: Vec<Item>) {
        if self.worker.is_none() {
            self.worker = Worker::spawn(model.waker());
        }
        let job = Job {
            ticket: self.ticket,
            items,
        };
        let refused = match &self.worker {
            Some(worker) => worker.jobs.send(job).err().map(|refused| refused.0),
            None => Some(job),
        };
        if let Some(job) = refused {
            log::error!("no measure worker, so the selection is measured on the UI thread");
            self.worker = None;
            self.arrive(job.ticket, measure(&job.items));
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
        let readout = measure(&[point(1.0, 2.0, 3.0), point(4.0, 6.0, 3.0)]);

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
        let readout = measure(&[
            point(0.0, 0.0, 0.0),
            point(1.0, 0.0, 0.0),
            point(2.0, 0.0, 0.0),
        ]);

        assert!(readout.groups.is_empty());
        assert_eq!(readout.problem, Some(TOO_MANY));
        assert_eq!(readout.line, None);
    }

    #[test]
    fn the_last_readout_stays_shown_as_stale_until_the_new_one_arrives() {
        let first = measure(&[point(0.0, 0.0, 0.0)]);
        let second = measure(&[point(1.0, 0.0, 0.0), point(2.0, 0.0, 0.0)]);
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
        let readout = measure(&[Item {
            name: "Region".to_owned(),
            subject: Subject::Unmeasurable,
        }]);

        assert!(readout.groups[0].problem.is_some());
        assert!(readout.groups[0].readings.is_empty());
    }
}
