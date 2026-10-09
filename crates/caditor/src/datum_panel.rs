use caditor_document::{
    AxisReference, CurveStation, Datum, DatumAxis, DatumFrame, DatumPlane, DatumPoint, Document,
    FaceTangent, Feature, FeatureId, PlaneReference, PlaneRotation, PlaneThrough, PointBy,
    PointReference, Transaction, capitalized, describe_axis, describe_curve, describe_origin,
    describe_plane, describe_point, describe_points,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::{EdgeReference, FaceReference, Surface};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    feature_fields::{self, Picker, Quantity, Rule, Shown},
    field,
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    solid_tools, widgets,
};

pub const PLANE_DESCRIPTION: &str = "A reference plane to sketch on or extrude up to";
pub const AXIS_DESCRIPTION: &str = "A reference axis to revolve, pattern or turn planes about";
pub const POINT_DESCRIPTION: &str = "A reference point to place planes and axes through";
pub const FRAME_DESCRIPTION: &str = "A second origin to measure and move from, and whose axes and \
                                     planes place patterns, mirrors and sketches: X runs along its \
                                     axis laid into its plane, Z stands square to the plane";
pub const REVERSE_X: &str = "Reverse X";
pub const REVERSE_Z: &str = "Reverse Z";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePart {
    Origin,
    XAxis,
    Plane,
}

impl FramePart {
    fn slot(self) -> Slot {
        match self {
            Self::Origin => Slot::FrameOrigin,
            Self::XAxis => Slot::FrameAxis,
            Self::Plane => Slot::FramePlane,
        }
    }

    fn caption(self) -> &'static str {
        match self {
            Self::Origin => "Origin",
            Self::XAxis => "X axis",
            Self::Plane => "XY plane",
        }
    }

    fn hover(self) -> &'static str {
        match self {
            Self::Origin => {
                "Stand its origin at the selected corner, centre, sketch point or datum point"
            }
            Self::XAxis => {
                "Run its X axis along the selected axis, straight edge, round face or sketch line"
            }
            Self::Plane => "Lay its XY plane along the selected plane or flat face",
        }
    }

    fn missing(self) -> &'static str {
        match self {
            Self::Origin => {
                "Select a corner, round edge, sketch point or datum point made before it"
            }
            Self::XAxis => {
                "Select an axis, straight edge, round face or sketch line made before it"
            }
            Self::Plane => "Select a plane or flat face made before it",
        }
    }
}
const CONTAINS_AXIS: &str = "Contains it";
const SQUARE_TO_AXIS: &str = "Square to it";
const TANGENT_TO_FACE: &str = "Tangent to it";
const CENTRE_OF_EDGE: &str = "Its centre";
const ALONG_THE_EDGE: &str = "Along it";
pub const MIDDLE_OF_EDGE: &str = "Its middle";
const OFFSET_AXES: [(&str, &str); 3] = [
    ("Offset X", "offset-x"),
    ("Offset Y", "offset-y"),
    ("Offset Z", "offset-z"),
];

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    index: usize,
    actions: &'a mut Vec<Action>,
}

struct Chooser<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: FeatureId,
    index: usize,
}

impl Chooser<'_> {
    fn of<'a>(model: &'a Model, selection: &'a Selection, feature: FeatureId) -> Chooser<'a> {
        Chooser {
            model,
            selection,
            feature,
            index: model.document().feature_index(feature).unwrap_or(0),
        }
    }

    fn change(&self, datum: Datum) -> Result<Transaction, String> {
        let document = self.model.document();
        let transaction = datum_tools::edit(document, self.feature, datum)
            .ok_or_else(|| "The feature no longer exists".to_owned())?;
        field::checked(document, transaction)
    }

    fn only_plane(&self) -> Result<Option<PlaneReference>, &'static str> {
        datum_tools::only_plane(self.model, self.selection, self.index)
    }

    fn only_axis(&self) -> Result<Option<AxisReference>, &'static str> {
        datum_tools::only_axis(self.model, self.selection, self.index)
    }

    fn base(&self, datum: &Datum) -> Result<Datum, &'static str> {
        match datum {
            Datum::Plane(plane) => match self.only_plane()? {
                Some(base) if base == plane.base => Err("It already starts from the selection"),
                Some(base) => Ok(Datum::Plane(DatumPlane {
                    base,
                    ..plane.clone()
                })),
                None => Err("Select a plane or flat face made before this plane"),
            },
            Datum::PlaneThrough(_) if self.selection.is_empty() => Err(
                "Select three points, two planes, an axis and a point, two lines or a curved edge",
            ),
            Datum::PlaneThrough(held) => {
                let chosen =
                    datum_tools::plane_from_selection(self.model, self.selection, self.index)?;
                match keeping_plane_mode(held, chosen) {
                    chosen if &chosen == datum => Err("It already follows the selection"),
                    chosen => Ok(chosen),
                }
            }
            Datum::Axis(axis) => {
                match datum_tools::axis_from_selection(self.model, self.selection, self.index) {
                    Ok(chosen) if &chosen == axis => Err("It already follows the selection"),
                    Ok(chosen) => Ok(Datum::Axis(chosen)),
                    Err(reason) => Err(reason),
                }
            }
            Datum::Point(_) | Datum::PointBy(_) if self.selection.is_empty() => {
                Err("Select a corner, round edge, sketch point or datum point")
            }
            Datum::Point(point) => {
                match datum_tools::point_from_selection(self.model, self.selection, self.index)? {
                    Datum::Point(chosen) if chosen.base == point.base => {
                        Err("It already sits at the selection")
                    }
                    Datum::Point(chosen) => Ok(Datum::Point(DatumPoint {
                        base: chosen.base,
                        offset: point.offset.clone(),
                    })),
                    chosen => Ok(chosen),
                }
            }
            Datum::PointBy(held) => {
                let chosen =
                    datum_tools::point_from_selection(self.model, self.selection, self.index)?;
                match keeping_point_mode(held, chosen) {
                    chosen if &chosen == datum => Err("It already follows the selection"),
                    chosen => Ok(chosen),
                }
            }
            Datum::Frame(held) => {
                match datum_tools::frame_from_selection(self.model, self.selection, self.index)? {
                    Datum::Frame(chosen) => Ok(Datum::Frame(Box::new(DatumFrame {
                        reverse_x: held.reverse_x,
                        reverse_z: held.reverse_z,
                        ..*chosen
                    }))),
                    chosen => Ok(chosen),
                }
            }
        }
    }

    fn frame_part(&self, datum: &Datum, part: FramePart) -> Result<Datum, &'static str> {
        let Datum::Frame(frame) = datum else {
            return Err("Only a coordinate system has an origin, an X axis and an XY plane");
        };
        let mut changed = frame.as_ref().clone();
        let unchanged = match part {
            FramePart::Origin => {
                let origin = datum_tools::only_point(self.model, self.selection, self.index)?
                    .ok_or(part.missing())?;
                std::mem::replace(&mut changed.origin, origin) == changed.origin
            }
            FramePart::XAxis => {
                let axis = self.only_axis()?.ok_or(part.missing())?;
                std::mem::replace(&mut changed.x_axis, axis) == changed.x_axis
            }
            FramePart::Plane => {
                let plane = self.only_plane()?.ok_or(part.missing())?;
                std::mem::replace(&mut changed.plane, plane) == changed.plane
            }
        };
        if unchanged {
            return Err("It already follows the selection");
        }
        Ok(Datum::Frame(Box::new(changed)))
    }

    fn rotation(&self, datum: &Datum) -> Result<Datum, &'static str> {
        let Datum::Plane(plane) = datum else {
            return Err("Only a datum plane turns about an axis");
        };
        match self.only_axis()? {
            Some(axis) if plane.rotation.as_ref().map(|rotation| &rotation.axis) == Some(&axis) => {
                Err("It already turns about the selection")
            }
            Some(axis) => Ok(Datum::Plane(DatumPlane {
                rotation: Some(PlaneRotation {
                    axis,
                    angle: plane.rotation.as_ref().map_or_else(
                        || solid_tools::degrees(datum_tools::DEFAULT_ANGLE),
                        |rotation| rotation.angle.clone(),
                    ),
                }),
                ..plane.clone()
            })),
            None => Err("Select an axis, straight edge or round face made before this plane"),
        }
    }

    fn checked(&self, datum: Result<Datum, &str>) -> Result<Transaction, String> {
        datum
            .map_err(str::to_owned)
            .and_then(|datum| self.change(datum))
    }
}

fn keeping_plane_mode(held: &PlaneThrough, chosen: Datum) -> Datum {
    match (held, chosen) {
        (
            PlaneThrough::Tangent(_),
            Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
                AxisReference::Face { body, face },
                toward,
            )),
        ) => Datum::PlaneThrough(PlaneThrough::Tangent(Box::new(FaceTangent {
            body,
            face,
            toward,
        }))),
        (
            PlaneThrough::TangentAt(_),
            Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
                AxisReference::Face { body, face },
                toward,
            )),
        ) => Datum::PlaneThrough(PlaneThrough::TangentAt(Box::new(FaceTangent {
            body,
            face,
            toward,
        }))),
        (
            PlaneThrough::SquareToCurve(held),
            Datum::PlaneThrough(PlaneThrough::SquareToCurve(mut chosen)),
        ) => {
            chosen.distance = held.distance.clone();
            Datum::PlaneThrough(PlaneThrough::SquareToCurve(chosen))
        }
        (_, chosen) => chosen,
    }
}

fn keeping_point_mode(held: &PointBy, chosen: Datum) -> Datum {
    match (held, chosen) {
        (PointBy::Along(held), Datum::PointBy(PointBy::Along(mut chosen))) => {
            chosen.distance = held.distance.clone();
            Datum::PointBy(PointBy::Along(chosen))
        }
        (_, chosen) => chosen,
    }
}

pub fn base_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    datum: &Datum,
) -> Result<Transaction, String> {
    let chooser = Chooser::of(model, selection, feature);
    chooser.checked(chooser.base(datum))
}

pub fn frame_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    datum: &Datum,
    part: FramePart,
) -> Result<Transaction, String> {
    let chooser = Chooser::of(model, selection, feature);
    chooser.checked(chooser.frame_part(datum, part))
}

pub fn rotation_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    datum: &Datum,
) -> Result<Transaction, String> {
    let chooser = Chooser::of(model, selection, feature);
    chooser.checked(chooser.rotation(datum))
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn chooser(&self) -> Chooser<'_> {
        Chooser {
            model: self.model,
            selection: self.selection,
            feature: self.id(),
            index: self.index,
        }
    }

    fn change(&self, datum: Datum) -> Result<Transaction, String> {
        self.chooser().change(datum)
    }

    fn apply(&mut self, change: Result<Transaction, String>) {
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn picker(
        &self,
        slot: Slot,
        selected: Result<Transaction, String>,
        hover: &'static str,
    ) -> Picker<'static> {
        Picker {
            feature: self.id(),
            slot,
            selected,
            hover,
        }
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        salt: &str,
        expression: &Expression,
        dimension: Dimension,
        rebuild: impl Fn(Expression) -> Datum,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("datum-field", salt, self.id())),
            expression,
            dimension,
            rule: Rule::Any,
        };
        let committed =
            feature_fields::expression_row(ui, self.model, caption, quantity, |parsed| {
                self.change(rebuild(parsed))
            });
        self.actions.extend(committed.map(Action::Apply));
    }

    fn base_row(&mut self, ui: &mut Ui, plane: &DatumPlane) {
        let document = self.model.document();
        let chooser = self.chooser();
        let based = feature_fields::offered_change(
            ui.ctx(),
            self.model,
            self.selection,
            (self.id(), Slot::DatumBase),
            || chooser.checked(chooser.base(&Datum::Plane(plane.clone()))),
        );
        let shown = Shown::Named(capitalized(&describe_plane(document, &plane.base)));
        let picker = self.picker(
            Slot::DatumBase,
            based,
            "Start from the selected plane or flat face",
        );
        feature_fields::reference_row(
            ui,
            self.model,
            "Starts from",
            shown,
            picker,
            None,
            self.actions,
        );
    }

    fn rotation_row(&mut self, ui: &mut Ui, plane: &DatumPlane) {
        let document = self.model.document();
        let shown = match &plane.rotation {
            Some(rotation) => Shown::Named(capitalized(&describe_axis(document, &rotation.axis))),
            None => Shown::NoneChosen,
        };
        let chooser = self.chooser();
        let turned = feature_fields::offered_change(
            ui.ctx(),
            self.model,
            self.selection,
            (self.id(), Slot::DatumRotation),
            || chooser.checked(chooser.rotation(&Datum::Plane(plane.clone()))),
        );
        let picker = self.picker(
            Slot::DatumRotation,
            turned,
            "Pass through the selected axis and turn about it by the angle",
        );
        let remove = plane.rotation.is_some().then_some("Stop turning the plane");
        let removed = feature_fields::reference_row(
            ui,
            self.model,
            "Turned about",
            shown,
            picker,
            remove,
            self.actions,
        );
        if removed {
            let change = self.change(Datum::Plane(DatumPlane {
                rotation: None,
                ..plane.clone()
            }));
            self.apply(change);
        }
    }

    fn plane_rows(&mut self, ui: &mut Ui, plane: &DatumPlane) {
        feature_fields::description_row(ui, PLANE_DESCRIPTION);
        self.base_row(ui, plane);
        self.rotation_row(ui, plane);
        if let Some(rotation) = &plane.rotation {
            self.expression(
                ui,
                "Angle",
                "angle",
                &rotation.angle,
                Dimension::ANGLE,
                |angle| {
                    Datum::Plane(DatumPlane {
                        rotation: Some(PlaneRotation {
                            angle,
                            ..rotation.clone()
                        }),
                        ..plane.clone()
                    })
                },
            );
        }
        self.expression(
            ui,
            "Offset",
            "offset",
            &plane.offset,
            Dimension::LENGTH,
            |offset| {
                Datum::Plane(DatumPlane {
                    offset,
                    ..plane.clone()
                })
            },
        );
    }

    fn axis_rows(&mut self, ui: &mut Ui, axis: &DatumAxis) {
        let document = self.model.document();
        feature_fields::description_row(ui, AXIS_DESCRIPTION);
        let (caption, text) = match axis {
            DatumAxis::Along(reference) => (
                "Runs along",
                capitalized(&describe_axis(document, reference)),
            ),
            DatumAxis::Intersection(first, second) => (
                "Defined by",
                format!(
                    "{} meets {}",
                    capitalized(&describe_plane(document, first)),
                    describe_plane(document, second)
                ),
            ),
            DatumAxis::Points(first, second) => (
                "Defined by",
                format!("Through {}", describe_points(document, &[first, second])),
            ),
            DatumAxis::NormalTo(plane, point) => (
                "Defined by",
                format!(
                    "Square to {} through {}",
                    describe_plane(document, plane),
                    describe_point(document, point)
                ),
            ),
            DatumAxis::SquareToFace(tangent) => (
                "Defined by",
                format!(
                    "Square to {} nearest {}",
                    describe_origin(document, tangent.face.origin()),
                    describe_point(document, &tangent.toward)
                ),
            ),
        };
        self.defined_by_row(
            ui,
            caption,
            text,
            &Datum::Axis(axis.clone()),
            "Run along the selected edge, round face or axis, where the two selected planes \
             meet, through two selected points, square to a selected plane through a point, or \
             square to a selected curved face nearest a point",
        );
    }

    fn defined_by_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        text: String,
        datum: &Datum,
        hover: &'static str,
    ) {
        let chooser = self.chooser();
        let chosen = feature_fields::offered_change(
            ui.ctx(),
            self.model,
            self.selection,
            (self.id(), Slot::DatumBase),
            || chooser.checked(chooser.base(datum)),
        );
        let picker = self.picker(Slot::DatumBase, chosen, hover);
        feature_fields::reference_row(
            ui,
            self.model,
            caption,
            Shown::Named(text),
            picker,
            None,
            self.actions,
        );
    }

    fn plane_through_rows(&mut self, ui: &mut Ui, through: &PlaneThrough) {
        let document = self.model.document();
        feature_fields::description_row(ui, PLANE_DESCRIPTION);
        self.defined_by_row(
            ui,
            "Defined by",
            describe_through(document, through),
            &Datum::PlaneThrough(through.clone()),
            "Pass through the three selected points, lie midway between the two selected \
             planes, or pass through the selected axis and point",
        );
        let (axis, point, mode) = match through {
            PlaneThrough::AxisAndPoint(axis, point) => (axis.clone(), point.clone(), 0),
            PlaneThrough::NormalTo(axis, point) => (axis.clone(), point.clone(), 1),
            PlaneThrough::Tangent(tangent) => (
                AxisReference::Face {
                    body: tangent.body,
                    face: tangent.face.clone(),
                },
                tangent.toward.clone(),
                2,
            ),
            PlaneThrough::TangentAt(tangent) if self.has_axis(tangent.body, &tangent.face) => (
                AxisReference::Face {
                    body: tangent.body,
                    face: tangent.face.clone(),
                },
                tangent.toward.clone(),
                2,
            ),
            PlaneThrough::TangentAt(_) => return,
            PlaneThrough::SquareToCurve(station) => {
                self.station_row(ui, station, |distance| {
                    Datum::PlaneThrough(PlaneThrough::SquareToCurve(Box::new(CurveStation {
                        distance,
                        ..station.as_ref().clone()
                    })))
                });
                return;
            }
            PlaneThrough::Points(_) | PlaneThrough::Midway(..) | PlaneThrough::Lines(..) => {
                return;
            }
        };
        let choices = [
            (
                CONTAINS_AXIS,
                "The plane holds the axis and passes through the point",
            ),
            (
                SQUARE_TO_AXIS,
                "The plane stands square to the axis at the point",
            ),
            (
                TANGENT_TO_FACE,
                "The plane touches the round face on the side of the point",
            ),
        ];
        let tangent_offered = matches!(axis, AxisReference::Face { .. });
        let offered: Vec<(&str, &str)> = choices
            .into_iter()
            .enumerate()
            .filter(|(index, _)| *index < 2 || tangent_offered)
            .map(|(_, choice)| choice)
            .collect();
        let chosen = widgets::property(ui, "Axis", |ui| widgets::segmented(ui, &offered, mode));
        let switched = match (chosen, &axis) {
            (Some(0), _) => PlaneThrough::AxisAndPoint(axis, point),
            (Some(1), _) => PlaneThrough::NormalTo(axis, point),
            (Some(_), AxisReference::Face { body, face }) => {
                let tangent = Box::new(FaceTangent {
                    body: *body,
                    face: face.clone(),
                    toward: point,
                });
                match self.is_cylinder_or_cone(*body, face) {
                    true => PlaneThrough::Tangent(tangent),
                    false => PlaneThrough::TangentAt(tangent),
                }
            }
            _ => return,
        };
        let change = self.change(Datum::PlaneThrough(switched));
        self.apply(change);
    }

    fn surface(&self, body: FeatureId, face: &FaceReference) -> Option<Surface> {
        datum_tools::face_surface(self.model, body, face, self.index)
    }

    fn has_axis(&self, body: FeatureId, face: &FaceReference) -> bool {
        matches!(
            self.surface(body, face),
            Some(
                Surface::Cylinder(_)
                    | Surface::Cone(_)
                    | Surface::Torus(_)
                    | Surface::Revolution(_)
            )
        )
    }

    fn is_cylinder_or_cone(&self, body: FeatureId, face: &FaceReference) -> bool {
        matches!(
            self.surface(body, face),
            Some(Surface::Cylinder(_) | Surface::Cone(_))
        )
    }

    fn edge_mode_row(&mut self, ui: &mut Ui, body: FeatureId, edge: &EdgeReference, mode: usize) {
        let round = mode == 0;
        let choices = [
            (CENTRE_OF_EDGE, "Sit at the centre of the round edge"),
            (
                ALONG_THE_EDGE,
                "Sit a distance along the edge from where it starts",
            ),
            (MIDDLE_OF_EDGE, "Sit halfway along the edge"),
        ];
        let offered: Vec<(&str, &str)> = choices
            .into_iter()
            .enumerate()
            .filter(|(index, _)| *index > 0 || round)
            .map(|(_, choice)| choice)
            .collect();
        let shown = if round { mode } else { mode - 1 };
        let chosen = widgets::property(ui, "Edge", |ui| widgets::segmented(ui, &offered, shown))
            .map(|chosen| if round { chosen } else { chosen + 1 });
        let switched = match chosen {
            Some(1) if mode != 1 => PointBy::Along(Box::new(CurveStation {
                body,
                edge: Box::new(*edge),
                distance: self.model.length_unit().default_length(0.0),
            })),
            Some(2) if mode != 2 => PointBy::EdgeMiddle {
                body,
                edge: Box::new(*edge),
            },
            _ => return,
        };
        let change = self.change(Datum::PointBy(switched));
        self.apply(change);
    }

    fn station_row(
        &mut self,
        ui: &mut Ui,
        station: &CurveStation,
        rebuild: impl Fn(Expression) -> Datum,
    ) {
        self.expression(
            ui,
            "Distance along",
            "distance",
            &station.distance,
            Dimension::LENGTH,
            rebuild,
        );
    }

    fn point_by_rows(&mut self, ui: &mut Ui, by: &PointBy) {
        let document = self.model.document();
        feature_fields::description_row(ui, POINT_DESCRIPTION);
        self.defined_by_row(
            ui,
            "At",
            describe_point_by(document, by),
            &Datum::PointBy(by.clone()),
            "Sit where the two selected lines cross, the selected line meets the selected plane, \
             the three selected planes meet, a distance along the selected edge, or at the centre \
             of the selected face",
        );
        match by {
            PointBy::Along(station) => {
                self.edge_mode_row(ui, station.body, &station.edge, 1);
                self.station_row(ui, station, |distance| {
                    Datum::PointBy(PointBy::Along(Box::new(CurveStation {
                        distance,
                        ..station.as_ref().clone()
                    })))
                });
            }
            PointBy::EdgeMiddle { body, edge } => self.edge_mode_row(ui, *body, edge, 2),
            PointBy::LinesCross(..)
            | PointBy::AxisAndPlane(..)
            | PointBy::ThreePlanes(_)
            | PointBy::FaceCentre { .. } => {}
        }
    }

    fn frame_rows(&mut self, ui: &mut Ui, frame: &DatumFrame) {
        let document = self.model.document();
        feature_fields::description_row(ui, FRAME_DESCRIPTION);
        let datum = Datum::Frame(Box::new(frame.clone()));
        for (part, shown) in [
            (FramePart::Origin, describe_point(document, &frame.origin)),
            (FramePart::XAxis, describe_axis(document, &frame.x_axis)),
            (FramePart::Plane, describe_plane(document, &frame.plane)),
        ] {
            let chooser = self.chooser();
            let chosen = feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), part.slot()),
                || chooser.checked(chooser.frame_part(&datum, part)),
            );
            let picker = self.picker(part.slot(), chosen, part.hover());
            feature_fields::reference_row(
                ui,
                self.model,
                part.caption(),
                Shown::Named(capitalized(&shown)),
                picker,
                None,
                self.actions,
            );
        }
        if let Some(reverse_x) = feature_fields::reverse_row(ui, REVERSE_X, frame.reverse_x) {
            let change = self.change(Datum::Frame(Box::new(DatumFrame {
                reverse_x,
                ..frame.clone()
            })));
            self.apply(change);
        }
        if let Some(reverse_z) = feature_fields::reverse_row(ui, REVERSE_Z, frame.reverse_z) {
            let change = self.change(Datum::Frame(Box::new(DatumFrame {
                reverse_z,
                ..frame.clone()
            })));
            self.apply(change);
        }
    }

    fn point_rows(&mut self, ui: &mut Ui, point: &DatumPoint) {
        let document = self.model.document();
        feature_fields::description_row(ui, POINT_DESCRIPTION);
        self.defined_by_row(
            ui,
            "At",
            capitalized(&describe_point(document, &point.base)),
            &Datum::Point(point.clone()),
            "Sit at the selected corner, round edge's centre, sketch point or datum point",
        );
        if let PointReference::Centre { body, edge } = &point.base {
            self.edge_mode_row(ui, *body, edge, 0);
        }
        for (index, (caption, salt)) in OFFSET_AXES.into_iter().enumerate() {
            let Some(offset) = point.offset.get(index) else {
                continue;
            };
            self.expression(ui, caption, salt, offset, Dimension::LENGTH, |value| {
                let mut offset = point.offset.clone();
                if let Some(slot) = offset.get_mut(index) {
                    *slot = value;
                }
                Datum::Point(DatumPoint {
                    base: point.base.clone(),
                    offset,
                })
            });
        }
    }
}

fn describe_through(document: &Document, through: &PlaneThrough) -> String {
    match through {
        PlaneThrough::Points([first, second, third]) => format!(
            "Through {}",
            describe_points(document, &[first, second, third])
        ),
        PlaneThrough::Midway(first, second) => format!(
            "Midway between {} and {}",
            describe_plane(document, first),
            describe_plane(document, second)
        ),
        PlaneThrough::AxisAndPoint(axis, point) => format!(
            "Through {} and {}",
            describe_axis(document, axis),
            describe_point(document, point)
        ),
        PlaneThrough::NormalTo(axis, point) => format!(
            "Square to {} at {}",
            describe_axis(document, axis),
            describe_point(document, point)
        ),
        PlaneThrough::Tangent(tangent) => format!(
            "Tangent to {} on the side of {}",
            describe_origin(document, tangent.face.origin()),
            describe_point(document, &tangent.toward)
        ),
        PlaneThrough::TangentAt(tangent) => format!(
            "Tangent to {} nearest {}",
            describe_origin(document, tangent.face.origin()),
            describe_point(document, &tangent.toward)
        ),
        PlaneThrough::SquareToCurve(station) => {
            format!("Square to {}", describe_curve(document, station))
        }
        PlaneThrough::Lines(first, second) => format!(
            "Through {} and {}",
            describe_axis(document, first),
            describe_axis(document, second)
        ),
    }
}

fn feature_name_of(document: &Document, feature: FeatureId) -> String {
    document.feature(feature).map_or_else(
        || "a deleted feature".to_owned(),
        |feature| feature.name.clone(),
    )
}

fn describe_point_by(document: &Document, by: &PointBy) -> String {
    match by {
        PointBy::LinesCross(first, second) => format!(
            "Where {} crosses {}",
            describe_axis(document, first),
            describe_axis(document, second)
        ),
        PointBy::AxisAndPlane(axis, plane) => format!(
            "Where {} meets {}",
            describe_axis(document, axis),
            describe_plane(document, plane)
        ),
        PointBy::ThreePlanes([first, second, third]) => format!(
            "Where {}, {} and {} meet",
            describe_plane(document, first),
            describe_plane(document, second),
            describe_plane(document, third)
        ),
        PointBy::Along(station) => format!("Along {}", describe_curve(document, station)),
        PointBy::EdgeMiddle { body, .. } => format!(
            "The middle of an edge of {}",
            feature_name_of(document, *body)
        ),
        PointBy::FaceCentre { face, .. } => {
            format!("The centre of {}", describe_origin(document, face.origin()))
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    datum: &Datum,
) {
    let id = feature.id();
    let mut panel = Panel {
        model,
        selection,
        feature,
        index: model.document().feature_index(id).unwrap_or(0),
        actions,
    };
    widgets::properties(ui, ("datum-properties", id), |ui| match datum {
        Datum::Plane(plane) => panel.plane_rows(ui, plane),
        Datum::PlaneThrough(through) => panel.plane_through_rows(ui, through),
        Datum::Axis(axis) => panel.axis_rows(ui, axis),
        Datum::Point(point) => panel.point_rows(ui, point),
        Datum::PointBy(by) => panel.point_by_rows(ui, by),
        Datum::Frame(frame) => panel.frame_rows(ui, frame),
    });
}
