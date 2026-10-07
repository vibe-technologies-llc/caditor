use caditor_document::{
    AxisReference, Datum, DatumAxis, DatumPlane, DatumPoint, Document, Feature, FeatureId,
    PlaneReference, PlaneRotation, PlaneThrough, Transaction, capitalized, describe_axis,
    describe_plane, describe_point, describe_points,
};
use caditor_expression::{Dimension, Expression};
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
const CONTAINS_AXIS: &str = "Contains it";
const SQUARE_TO_AXIS: &str = "Square to it";
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

    fn first_plane(&self) -> Option<PlaneReference> {
        self.selection
            .iter()
            .find_map(|pickable| datum_tools::plane_reference(self.model, pickable, self.index))
    }

    fn first_axis(&self) -> Option<AxisReference> {
        self.selection
            .iter()
            .find_map(|pickable| datum_tools::axis_reference(self.model, pickable, self.index))
    }

    fn base(&self, datum: &Datum) -> Result<Datum, &'static str> {
        match datum {
            Datum::Plane(plane) => match self.first_plane() {
                Some(base) if base == plane.base => Err("It already starts from the selection"),
                Some(base) => Ok(Datum::Plane(DatumPlane {
                    base,
                    ..plane.clone()
                })),
                None => Err("Select a plane or flat face made before this plane"),
            },
            Datum::PlaneThrough(_) if self.selection.is_empty() => {
                Err("Select three points, two planes, or an axis and a point")
            }
            Datum::PlaneThrough(_) => {
                match datum_tools::plane_from_selection(self.model, self.selection, self.index) {
                    Ok(chosen) if &chosen == datum => Err("It already follows the selection"),
                    Ok(chosen) => Ok(chosen),
                    Err(reason) => Err(reason),
                }
            }
            Datum::Axis(axis) => {
                match datum_tools::axis_from_selection(self.model, self.selection, self.index) {
                    Ok(chosen) if &chosen == axis => Err("It already follows the selection"),
                    Ok(chosen) => Ok(Datum::Axis(chosen)),
                    Err(reason) => Err(reason),
                }
            }
            Datum::Point(_) if self.selection.is_empty() => {
                Err("Select a corner, round edge, sketch point or datum point")
            }
            Datum::Point(point) => {
                match datum_tools::point_from_selection(self.model, self.selection, self.index) {
                    Ok(chosen) if chosen.base == point.base => {
                        Err("It already sits at the selection")
                    }
                    Ok(chosen) => Ok(Datum::Point(DatumPoint {
                        base: chosen.base,
                        offset: point.offset.clone(),
                    })),
                    Err(reason) => Err(reason),
                }
            }
        }
    }

    fn rotation(&self, datum: &Datum) -> Result<Datum, &'static str> {
        let Datum::Plane(plane) = datum else {
            return Err("Only a datum plane turns about an axis");
        };
        match self.first_axis() {
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

pub fn base_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    datum: &Datum,
) -> Result<Transaction, String> {
    let chooser = Chooser::of(model, selection, feature);
    chooser.checked(chooser.base(datum))
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
        };
        self.defined_by_row(
            ui,
            caption,
            text,
            &Datum::Axis(axis.clone()),
            "Run along the selected edge, round face or axis, where the two selected planes \
             meet, through two selected points, or square to a selected plane through a point",
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
        let (axis, point, square) = match through {
            PlaneThrough::AxisAndPoint(axis, point) => (axis, point, false),
            PlaneThrough::NormalTo(axis, point) => (axis, point, true),
            PlaneThrough::Points(_) | PlaneThrough::Midway(..) => return,
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
        ];
        let chosen = widgets::property(ui, "Axis", |ui| {
            widgets::segmented(ui, &choices, usize::from(square))
        });
        let flipped = match chosen {
            Some(0) => PlaneThrough::AxisAndPoint(axis.clone(), point.clone()),
            Some(_) => PlaneThrough::NormalTo(axis.clone(), point.clone()),
            None => return,
        };
        let change = self.change(Datum::PlaneThrough(flipped));
        self.apply(change);
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
    });
}
