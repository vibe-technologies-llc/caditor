use caditor_document::{
    AxisReference, Datum, DatumAxis, DatumPlane, Feature, FeatureId, PlaneReference, PlaneRotation,
    Transaction, capitalized, describe_axis, describe_plane,
};
use caditor_expression::{Dimension, Expression};
use egui::{Button, Grid, Id, Ui};

use crate::{
    datum_tools,
    field::{self, Expected},
    model::{Action, Model},
    selection::Selection,
    solid_tools,
};

const FIELD_WIDTH: f32 = 110.0;
const USE_SELECTED: &str = "Use selected";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: FeatureId,
    index: usize,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn change(&self, datum: Datum) -> Result<Transaction, String> {
        let document = self.model.document();
        let transaction = datum_tools::edit(document, self.feature, datum)
            .ok_or_else(|| "The feature no longer exists".to_owned())?;
        field::checked(document, transaction)
    }

    fn apply(&mut self, change: Result<Transaction, String>) {
        match change {
            Ok(transaction) => self.actions.push(Action::Apply(transaction)),
            Err(reason) => log::warn!("could not change the datum: {reason}"),
        }
    }

    fn use_button(&mut self, ui: &mut Ui, hover: &str, change: Result<Datum, &str>) {
        let change = change
            .map_err(str::to_owned)
            .and_then(|datum| self.change(datum));
        let response = ui.add_enabled(change.is_ok(), Button::new(USE_SELECTED).small());
        match change {
            Ok(transaction) => {
                if response.on_hover_text(hover).clicked() {
                    self.actions.push(Action::Apply(transaction));
                }
            }
            Err(reason) => {
                response.on_disabled_hover_text(reason);
            }
        }
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        expression: &Expression,
        dimension: Dimension,
        rebuild: impl Fn(Expression) -> Datum,
    ) {
        let model = self.model;
        let document = model.document();
        let parameters = model.parameters();
        let mut error = None;
        let mut committed = None;
        ui.horizontal(|ui| {
            let field = field::commit_field(
                ui,
                Id::new(("datum-field", salt, self.feature)),
                &document.expression_text(expression),
                FIELD_WIDTH,
                false,
                |text| {
                    let parsed = field::parse_expression(
                        document,
                        parameters,
                        text,
                        Expected {
                            dimension: Some(dimension),
                            non_negative: false,
                        },
                        model.length_unit(),
                    )?;
                    self.change(rebuild(parsed))
                },
            );
            committed = field.committed;
            if field.error.is_none()
                && let Some(preview) =
                    field::value_preview(parameters, expression, model.length_unit())
            {
                ui.weak(preview);
            }
            error = field.error;
        });
        if let Some(transaction) = committed {
            self.actions.push(Action::Apply(transaction));
        }
        ui.end_row();
        if let Some(error) = error {
            ui.label("");
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.end_row();
        }
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

    fn plane_rows(&mut self, ui: &mut Ui, plane: &DatumPlane) {
        let document = self.model.document();
        ui.label("Starts from");
        ui.horizontal(|ui| {
            ui.label(capitalized(&describe_plane(document, &plane.base)));
            let change = match self.first_plane() {
                Some(base) if base == plane.base => Err("It already starts from the selection"),
                Some(base) => Ok(Datum::Plane(DatumPlane {
                    base,
                    ..plane.clone()
                })),
                None => Err("Select a plane or flat face made before this plane"),
            };
            self.use_button(ui, "Start from the selected plane or flat face", change);
        });
        ui.end_row();

        ui.label("Turned about");
        ui.horizontal(|ui| {
            match &plane.rotation {
                Some(rotation) => {
                    ui.label(capitalized(&describe_axis(document, &rotation.axis)));
                }
                None => {
                    ui.weak("Nothing");
                }
            }
            let change = match self.first_axis() {
                Some(axis)
                    if plane.rotation.as_ref().map(|rotation| &rotation.axis) == Some(&axis) =>
                {
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
            };
            self.use_button(
                ui,
                "Pass through the selected axis and turn about it by the angle",
                change,
            );
            if plane.rotation.is_some()
                && ui
                    .small_button("🗙")
                    .on_hover_text("Stop turning the plane")
                    .clicked()
            {
                let change = self.change(Datum::Plane(DatumPlane {
                    rotation: None,
                    ..plane.clone()
                }));
                self.apply(change);
            }
        });
        ui.end_row();

        if let Some(rotation) = &plane.rotation {
            ui.label("Angle");
            self.expression(ui, "angle", &rotation.angle, Dimension::ANGLE, |angle| {
                Datum::Plane(DatumPlane {
                    rotation: Some(PlaneRotation {
                        angle,
                        ..rotation.clone()
                    }),
                    ..plane.clone()
                })
            });
        }

        ui.label("Offset");
        self.expression(ui, "offset", &plane.offset, Dimension::LENGTH, |offset| {
            Datum::Plane(DatumPlane {
                offset,
                ..plane.clone()
            })
        });
    }

    fn axis_rows(&mut self, ui: &mut Ui, axis: &DatumAxis) {
        let document = self.model.document();
        let (title, text) = match axis {
            DatumAxis::Along(reference) => (
                "Runs along",
                capitalized(&describe_axis(document, reference)),
            ),
            DatumAxis::Intersection(first, second) => (
                "Where",
                format!(
                    "{} meets {}",
                    capitalized(&describe_plane(document, first)),
                    describe_plane(document, second)
                ),
            ),
        };
        ui.label(title);
        ui.horizontal(|ui| {
            ui.label(text);
            let change =
                match datum_tools::axis_from_selection(self.model, self.selection, self.index) {
                    Ok(chosen) if &chosen == axis => Err("It already follows the selection"),
                    Ok(chosen) => Ok(Datum::Axis(chosen)),
                    Err(reason) => Err(reason),
                };
            self.use_button(
                ui,
                "Run along the selected edge, round face or axis, or where the two selected \
                 planes meet",
                change,
            );
        });
        ui.end_row();
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
        feature: id,
        index: model.document().feature_index(id).unwrap_or(0),
        actions,
    };
    Grid::new(("datum-properties", id))
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| match datum {
            Datum::Plane(plane) => panel.plane_rows(ui, plane),
            Datum::Axis(axis) => panel.axis_rows(ui, axis),
        });
}
