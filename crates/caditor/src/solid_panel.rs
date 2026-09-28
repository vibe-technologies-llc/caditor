use caditor_document::{
    BodyOperation, Document, Extrude, ExtrudeExtent, Feature, FeatureId, RegionChoice, Revolve,
    RevolveExtent, SolidFeature, Transaction,
};
use caditor_expression::{Dimension, Expression};
use caditor_sketch::{Entity, EntityId, Reference};
use egui::{Button, ComboBox, Grid, Id, Ui};

use crate::{
    editing::EditingCommand,
    field::{self, Expected},
    model::{Action, Model},
    scene, selection,
    solid_tools::{self, DEFAULT_PARTIAL_ANGLE},
};

const FIELD_WIDTH: f32 = 110.0;
const FULL_TURN_DEGREES: f64 = 360.0;

struct Panel<'a> {
    model: &'a Model,
    feature: &'a Feature,
    solid: &'a SolidFeature,
    actions: &'a mut Vec<Action>,
}

struct Choice {
    label: String,
    selected: bool,
    change: Result<Transaction, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Positive,
    PositiveTurn,
    Any,
}

impl Panel<'_> {
    fn document(&self) -> &Document {
        self.model.document()
    }

    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, solid: SolidFeature) -> Result<Transaction, String> {
        change(self.model, self.id(), solid)
    }

    fn combo(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        selected: &str,
        options: impl FnOnce(&Self) -> Vec<Choice>,
    ) {
        let mut chosen = None;
        ComboBox::from_id_salt((salt, self.id()))
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for option in options(self) {
                    let enabled = option.change.is_ok() || option.selected;
                    let response =
                        ui.add_enabled(enabled, Button::selectable(option.selected, &option.label));
                    let response = match &option.change {
                        Err(reason) if !option.selected => response.on_disabled_hover_text(reason),
                        Err(_) | Ok(_) => response,
                    };
                    if response.clicked()
                        && !option.selected
                        && let Ok(transaction) = option.change
                    {
                        chosen = Some(transaction);
                    }
                }
            });
        if let Some(transaction) = chosen {
            self.actions.push(Action::Apply(transaction));
        }
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        expression: &Expression,
        dimension: Dimension,
        rule: Rule,
        rebuild: impl FnOnce(Expression) -> SolidFeature,
    ) {
        let model = self.model;
        let id = self.id();
        let document = model.document();
        let parameters = model.parameters();
        let mut error = None;
        ui.horizontal(|ui| {
            let field = field::commit_field(
                ui,
                Id::new(("solid-field", salt, id)),
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
                    )?;
                    let value = parameters
                        .evaluate_expression(&parsed)
                        .map_err(|error| field::sentence(&error.to_string()))?
                        .value;
                    check_rule(rule, value)?;
                    change(model, id, rebuild(parsed))
                },
            );
            if let Some(transaction) = field.committed {
                self.actions.push(Action::Apply(transaction));
            }
            if field.error.is_none()
                && let Some(preview) = field::value_preview(parameters, expression)
            {
                ui.weak(preview);
            }
            error = field.error;
        });
        ui.end_row();
        if let Some(error) = error {
            ui.label("");
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.end_row();
        }
    }

    fn sketch_row(&mut self, ui: &mut Ui) {
        ui.label("Sketch");
        let current = self.solid.sketch();
        let model = self.model;
        let name = model
            .document()
            .feature(current)
            .map_or("a missing sketch", |sketch| sketch.name.as_str());
        self.combo(ui, "sketch", name, |panel| {
            let end = panel.document().feature_index(panel.id()).unwrap_or(0);
            panel
                .document()
                .features()
                .take(end)
                .filter(|candidate| candidate.kind.sketch().is_some())
                .map(|candidate| Choice {
                    label: candidate.name.clone(),
                    selected: candidate.id() == current,
                    change: panel.change(with_sketch(panel.solid, candidate.id())),
                })
                .collect()
        });
        ui.end_row();
    }

    fn regions_row(&mut self, ui: &mut Ui, opened: bool) {
        ui.label("Regions");
        let model = self.model;
        let document = model.document();
        let evaluation = model.evaluation();
        let regions = selection::swept_regions(document, evaluation, self.id());
        let all = matches!(self.solid.regions(), RegionChoice::All);
        let chosen_count = regions.map_or(0, |(_, regions)| {
            scene::chosen_regions(self.solid.regions(), regions).len()
        });
        let mut change = None;
        ui.vertical(|ui| {
            if ui
                .radio(all, "All closed regions")
                .on_hover_text("Sweep every region that is not a hole of another")
                .clicked()
                && !all
            {
                change =
                    Some(self.change(solid_tools::with_regions(self.solid, RegionChoice::All)));
            }
            let chosen_text = format!(
                "Chosen: {}",
                crate::feature_tree::count(chosen_count, "region", "regions")
            );
            let can_choose = regions.is_some();
            let response = ui.add_enabled(can_choose, egui::RadioButton::new(!all, chosen_text));
            if response.clicked()
                && all
                && let Some((_, regions)) = regions
            {
                let keys = scene::chosen_regions(&RegionChoice::All, regions)
                    .into_iter()
                    .collect();
                change = Some(self.change(solid_tools::with_regions(
                    self.solid,
                    RegionChoice::Chosen(keys),
                )));
            }
            if opened {
                ui.weak("Click regions in the view to include or leave them out.");
            } else if ui
                .small_button("Choose in the view")
                .on_hover_text("Show the regions of the sketch so you can click them")
                .clicked()
            {
                self.actions
                    .push(Action::Editing(EditingCommand::OpenSolid(self.id())));
            }
        });
        ui.end_row();
        match change {
            Some(Ok(transaction)) => self.actions.push(Action::Apply(transaction)),
            Some(Err(reason)) => log::warn!("could not change the regions: {reason}"),
            None => {}
        }
    }

    fn extrude_rows(&mut self, ui: &mut Ui, extrude: &Extrude) {
        ui.label("Extent");
        let current = extent_name(&extrude.extent);
        self.combo(ui, "extrude-extent", current, |panel| {
            let distance = match &extrude.extent {
                ExtrudeExtent::OneSide { distance, .. }
                | ExtrudeExtent::Symmetric { distance }
                | ExtrudeExtent::TwoSides {
                    forward: distance, ..
                } => distance.clone(),
            };
            [
                ExtrudeExtent::OneSide {
                    distance: distance.clone(),
                    reversed: false,
                },
                ExtrudeExtent::Symmetric {
                    distance: distance.clone(),
                },
                ExtrudeExtent::TwoSides {
                    forward: distance.clone(),
                    backward: distance,
                },
            ]
            .into_iter()
            .map(|extent| Choice {
                label: extent_name(&extent).to_owned(),
                selected: extent_name(&extent) == current,
                change: panel.change(SolidFeature::Extrude(Extrude {
                    extent,
                    ..extrude.clone()
                })),
            })
            .collect()
        });
        ui.end_row();
        match &extrude.extent {
            ExtrudeExtent::OneSide { distance, reversed } => {
                ui.label("Distance");
                let reversed = *reversed;
                self.expression(
                    ui,
                    "distance",
                    distance,
                    Dimension::LENGTH,
                    Rule::Positive,
                    |distance| {
                        SolidFeature::Extrude(Extrude {
                            extent: ExtrudeExtent::OneSide { distance, reversed },
                            ..extrude.clone()
                        })
                    },
                );
                ui.label("Direction");
                let mut flipped = reversed;
                if ui.checkbox(&mut flipped, "Reversed").changed() {
                    let flipped = SolidFeature::Extrude(Extrude {
                        extent: ExtrudeExtent::OneSide {
                            distance: distance.clone(),
                            reversed: flipped,
                        },
                        ..extrude.clone()
                    });
                    self.apply(flipped);
                }
                ui.end_row();
            }
            ExtrudeExtent::Symmetric { distance } => {
                ui.label("Total distance");
                self.expression(
                    ui,
                    "distance",
                    distance,
                    Dimension::LENGTH,
                    Rule::Positive,
                    |distance| {
                        SolidFeature::Extrude(Extrude {
                            extent: ExtrudeExtent::Symmetric { distance },
                            ..extrude.clone()
                        })
                    },
                );
            }
            ExtrudeExtent::TwoSides { forward, backward } => {
                ui.label("Forward");
                self.expression(
                    ui,
                    "forward",
                    forward,
                    Dimension::LENGTH,
                    Rule::Any,
                    |forward| {
                        SolidFeature::Extrude(Extrude {
                            extent: ExtrudeExtent::TwoSides {
                                forward,
                                backward: backward.clone(),
                            },
                            ..extrude.clone()
                        })
                    },
                );
                ui.label("Backward");
                self.expression(
                    ui,
                    "backward",
                    backward,
                    Dimension::LENGTH,
                    Rule::Any,
                    |backward| {
                        SolidFeature::Extrude(Extrude {
                            extent: ExtrudeExtent::TwoSides {
                                forward: forward.clone(),
                                backward,
                            },
                            ..extrude.clone()
                        })
                    },
                );
            }
        }
    }

    fn revolve_rows(&mut self, ui: &mut Ui, revolve: &Revolve) {
        ui.label("Axis");
        let model = self.model;
        let sketch = model
            .document()
            .feature(revolve.sketch)
            .and_then(|feature| feature.kind.sketch());
        let axis_name = sketch.map_or_else(
            || "a missing line".to_owned(),
            |sketch| sketch.entity_label(revolve.axis),
        );
        self.combo(ui, "axis", &axis_name, |panel| {
            let Some(sketch) = sketch else {
                return Vec::new();
            };
            let lines = sketch
                .entities()
                .filter_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id));
            [Reference::HorizontalAxis.id(), Reference::VerticalAxis.id()]
                .into_iter()
                .chain(lines)
                .map(|axis| Choice {
                    label: sketch.entity_label(axis),
                    selected: axis == revolve.axis,
                    change: panel.change(SolidFeature::Revolve(Revolve {
                        axis,
                        ..revolve.clone()
                    })),
                })
                .collect()
        });
        ui.end_row();

        ui.label("Extent");
        let current = turn_name(&revolve.extent);
        self.combo(ui, "revolve-extent", current, |panel| {
            let angle = match &revolve.extent {
                RevolveExtent::Full => solid_tools::degrees(DEFAULT_PARTIAL_ANGLE),
                RevolveExtent::OneSide { angle, .. } | RevolveExtent::Symmetric { angle } => {
                    angle.clone()
                }
            };
            [
                RevolveExtent::Full,
                RevolveExtent::OneSide {
                    angle: angle.clone(),
                    reversed: false,
                },
                RevolveExtent::Symmetric { angle },
            ]
            .into_iter()
            .map(|extent| Choice {
                label: turn_name(&extent).to_owned(),
                selected: turn_name(&extent) == current,
                change: panel.change(SolidFeature::Revolve(Revolve {
                    extent,
                    ..revolve.clone()
                })),
            })
            .collect()
        });
        ui.end_row();

        match &revolve.extent {
            RevolveExtent::Full => {}
            RevolveExtent::OneSide { angle, reversed } => {
                ui.label("Angle");
                let reversed = *reversed;
                self.expression(
                    ui,
                    "angle",
                    angle,
                    Dimension::ANGLE,
                    Rule::PositiveTurn,
                    |angle| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::OneSide { angle, reversed },
                            ..revolve.clone()
                        })
                    },
                );
                ui.label("Direction");
                let mut flipped = reversed;
                if ui.checkbox(&mut flipped, "Reversed").changed() {
                    let flipped = SolidFeature::Revolve(Revolve {
                        extent: RevolveExtent::OneSide {
                            angle: angle.clone(),
                            reversed: flipped,
                        },
                        ..revolve.clone()
                    });
                    self.apply(flipped);
                }
                ui.end_row();
            }
            RevolveExtent::Symmetric { angle } => {
                ui.label("Total angle");
                self.expression(
                    ui,
                    "angle",
                    angle,
                    Dimension::ANGLE,
                    Rule::PositiveTurn,
                    |angle| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::Symmetric { angle },
                            ..revolve.clone()
                        })
                    },
                );
            }
        }
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let operation = self.solid.operation();
        let bodies = solid_tools::bodies_before(self.document(), self.id());
        let fallback = bodies.last().copied();
        ui.label("Result");
        self.combo(ui, "operation", operation_name(operation), |panel| {
            let target = operation.target().or(fallback);
            let candidates = [
                Some(BodyOperation::NewBody),
                target.map(BodyOperation::Add),
                target.map(BodyOperation::Remove),
                target.map(BodyOperation::Intersect),
            ];
            candidates
                .into_iter()
                .flatten()
                .map(|candidate| Choice {
                    label: operation_name(candidate).to_owned(),
                    selected: operation_name(candidate) == operation_name(operation),
                    change: panel.change(solid_tools::with_operation(panel.solid, candidate)),
                })
                .collect()
        });
        ui.end_row();
        let Some(target) = operation.target() else {
            return;
        };
        ui.label("Body");
        let model = self.model;
        let name = model
            .document()
            .feature(target)
            .map_or("a missing body", |body| body.name.as_str());
        self.combo(ui, "body", name, |panel| {
            bodies
                .iter()
                .filter_map(|body| {
                    let label = panel.document().feature(*body)?.name.clone();
                    Some(Choice {
                        label,
                        selected: *body == target,
                        change: panel.change(solid_tools::with_operation(
                            panel.solid,
                            operation.with_target(*body),
                        )),
                    })
                })
                .collect()
        });
        ui.end_row();
    }

    fn apply(&mut self, solid: SolidFeature) {
        match self.change(solid) {
            Ok(transaction) => self.actions.push(Action::Apply(transaction)),
            Err(reason) => log::warn!("could not change {}: {reason}", self.feature.name),
        }
    }
}

fn change(model: &Model, feature: FeatureId, solid: SolidFeature) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = solid_tools::edit(document, feature, solid)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
}

fn check_rule(rule: Rule, value: f64) -> Result<(), String> {
    match rule {
        Rule::Any => Ok(()),
        Rule::Positive if value > 0.0 => Ok(()),
        Rule::PositiveTurn if value > 0.0 && value <= FULL_TURN_DEGREES => Ok(()),
        Rule::Positive => {
            Err("Enter a value above zero. Use Reversed to go the other way".to_owned())
        }
        Rule::PositiveTurn => Err("Enter an angle above zero and at most 360°".to_owned()),
    }
}

fn with_sketch(solid: &SolidFeature, sketch: FeatureId) -> SolidFeature {
    match solid {
        SolidFeature::Extrude(extrude) => SolidFeature::Extrude(Extrude {
            sketch,
            regions: RegionChoice::All,
            ..extrude.clone()
        }),
        SolidFeature::Revolve(revolve) => SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: if revolve.axis.is_reference() {
                revolve.axis
            } else {
                EntityId::VERTICAL_AXIS
            },
            ..revolve.clone()
        }),
    }
}

fn extent_name(extent: &ExtrudeExtent) -> &'static str {
    match extent {
        ExtrudeExtent::OneSide { .. } => "One side",
        ExtrudeExtent::Symmetric { .. } => "Symmetric",
        ExtrudeExtent::TwoSides { .. } => "Two sides",
    }
}

fn turn_name(extent: &RevolveExtent) -> &'static str {
    match extent {
        RevolveExtent::Full => "Full turn",
        RevolveExtent::OneSide { .. } => "One side",
        RevolveExtent::Symmetric { .. } => "Symmetric",
    }
}

fn operation_name(operation: BodyOperation) -> &'static str {
    match operation {
        BodyOperation::NewBody => "New body",
        BodyOperation::Add(_) => "Add to body",
        BodyOperation::Remove(_) => "Remove from body",
        BodyOperation::Intersect(_) => "Intersect with body",
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    actions: &mut Vec<Action>,
    feature: &Feature,
    solid: &SolidFeature,
    opened: bool,
) {
    let mut panel = Panel {
        model,
        feature,
        solid,
        actions,
    };
    Grid::new(("solid-properties", feature.id()))
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            panel.sketch_row(ui);
            panel.regions_row(ui, opened);
            match solid {
                SolidFeature::Extrude(extrude) => panel.extrude_rows(ui, extrude),
                SolidFeature::Revolve(revolve) => panel.revolve_rows(ui, revolve),
            }
            panel.operation_rows(ui);
        });
}
