use caditor_document::{
    AxisReference, BodyOperation, Document, Extrude, ExtrudeEnd, ExtrudeExtent, Feature, FeatureId,
    PlaneReference, RegionChoice, Revolve, RevolveAxis, RevolveExtent, SolidFeature, SolidStart,
    Transaction, capitalized, describe_plane,
};
use caditor_expression::{Dimension, Expression};
use caditor_sketch::{Entity, EntityId, Reference};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    editing::EditingCommand,
    feature_fields::{
        self, Choice, MISSING_BODY, MISSING_SKETCH, Picker, Quantity, REVERSE_DIRECTION, Rule,
        Segment, Shown,
    },
    feature_tree::count,
    field, icons,
    model::{Action, Model},
    reference_picking::{self, Picking, Side, Slot},
    scene,
    selection::{self, Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    solid_tools::{self, DEFAULT_BACKWARD_ANGLE, DEFAULT_PARTIAL_ANGLE},
    widgets,
};

const FULL_TURN_DEGREES: f64 = 360.0;
pub const ALSO_CUTS: &str = "Also cuts";
pub const ADD_CUT_BODY: &str = "Add another body";
const START_OFFSET: &str = "Start offset";
const START_BY_OFFSET: &str = "Sketch plane, offset";
const START_ON_FACE: &str = "Face or plane";
const THROUGH_ALL_NEEDS_A_CUT: &str = "Through all cuts into a body or intersects with it; choose Remove from body or Intersect \
     with body first";
const UP_TO_NEXT_NEEDS_A_BODY: &str = "Up to next stops at the body this feature changes; choose Add, Remove or Intersect with a \
     body first";
const NO_TARGET_SELECTED: &str =
    "Select a flat face or a plane made before this feature, then use it";
const CURVED_TARGET: &str =
    "The selected face is curved; an extrusion can only end on a flat face or plane";
const SIDES_CHANGED: &str = "The extrusion no longer has that end; choose the face again";

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    solid: &'a SolidFeature,
    actions: &'a mut Vec<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndKind {
    Distance,
    ThroughAll,
    UpToNext,
    UpToFace,
}

impl EndKind {
    const ALL: [Self; 4] = [
        Self::Distance,
        Self::ThroughAll,
        Self::UpToNext,
        Self::UpToFace,
    ];

    fn of(end: &ExtrudeEnd) -> Self {
        match end {
            ExtrudeEnd::Distance(_) => Self::Distance,
            ExtrudeEnd::ThroughAll => Self::ThroughAll,
            ExtrudeEnd::UpToNext => Self::UpToNext,
            ExtrudeEnd::UpToFace(_) => Self::UpToFace,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Distance => "Distance",
            Self::ThroughAll => "Through all",
            Self::UpToNext => "Up to next",
            Self::UpToFace => "Up to face",
        }
    }
}

struct EndRows<'a> {
    end: &'a str,
    distance: &'a str,
    face: &'a str,
    salt: &'a str,
    side: Side,
    rule: Rule,
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

    fn default_distance(&self) -> Expression {
        self.model
            .length_unit()
            .default_length(solid_tools::DEFAULT_DISTANCE)
    }

    fn picking(&self, ui: &Ui, slot: Slot) -> bool {
        reference_picking::current(ui.ctx()).is_some_and(|picking| picking.is_for(self.id(), slot))
    }

    fn combo(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        selected: impl Into<egui::WidgetText>,
        options: impl FnOnce(&Self) -> Vec<Choice>,
    ) {
        let chosen =
            feature_fields::combo(ui, Id::new((salt, self.id())), selected, || options(self));
        self.actions.extend(chosen);
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        salt: &str,
        expression: &Expression,
        (dimension, rule): (Dimension, Rule),
        rebuild: impl FnOnce(Expression) -> SolidFeature,
    ) {
        let model = self.model;
        let id = self.id();
        let quantity = Quantity {
            id: Id::new(("solid-field", salt, id)),
            expression,
            dimension,
            rule,
        };
        let committed = feature_fields::expression_row(ui, model, caption, quantity, |parsed| {
            change(model, id, rebuild(parsed))
        });
        self.actions.extend(committed.map(Action::Apply));
    }

    fn sketch_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Sketch");
        let current = self.solid.sketch();
        let model = self.model;
        ui.horizontal(|ui| {
            let name = feature_fields::feature_name(model.document(), current);
            let text = feature_fields::combo_text(ui, name, MISSING_SKETCH);
            self.combo(ui, "sketch", text, |panel| {
                let end = panel.document().feature_index(panel.id()).unwrap_or(0);
                panel
                    .document()
                    .features()
                    .take(end)
                    .filter(|candidate| candidate.kind.sketch().is_some())
                    .map(|candidate| Choice {
                        label: candidate.name.clone(),
                        selected: candidate.id() == current,
                        change: panel
                            .change(with_sketch(panel.solid, candidate.id()))
                            .map(Action::Apply),
                    })
                    .collect()
            });
        });
        ui.end_row();
    }

    fn regions_row(&mut self, ui: &mut Ui, opened: bool) {
        widgets::caption(ui, "Regions");
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
            let choices = [
                (
                    "All closed",
                    "Sweep every region that is not a hole of another",
                ),
                ("Chosen", "Sweep only the regions you choose in the view"),
            ];
            match widgets::segmented(ui, &choices, usize::from(!all)) {
                Some(0) => {
                    change =
                        Some(self.change(solid_tools::with_regions(self.solid, RegionChoice::All)));
                }
                Some(_) => {
                    if let Some((_, regions)) = regions {
                        let keys = scene::chosen_regions(&RegionChoice::All, regions);
                        change = Some(self.change(solid_tools::with_regions(
                            self.solid,
                            RegionChoice::Chosen(scene::region_references(&keys, regions)),
                        )));
                    }
                }
                None => {}
            }
            if !all {
                let chosen = format!("{} chosen", count(chosen_count, "region", "regions"));
                ui.label(widgets::muted(chosen, ui));
            }
            if widgets::choose_in_view(
                ui,
                feature_fields::choosing_list(ui, opened),
                "Click regions in the view to include or leave them out.",
                "Show the regions of the sketch so you can click them",
            ) {
                self.actions
                    .push(Action::Editing(EditingCommand::OpenSolid(self.id())));
            }
        });
        ui.end_row();
        if let Some(change) = change {
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }

    fn end_choices(
        &self,
        extrude: &Extrude,
        end: &ExtrudeEnd,
        side: Side,
        rebuild: &dyn Fn(ExtrudeEnd) -> ExtrudeExtent,
    ) -> Vec<Choice> {
        let current = EndKind::of(end);
        EndKind::ALL
            .into_iter()
            .map(|kind| {
                let candidate = match kind {
                    EndKind::Distance => Ok(ExtrudeEnd::Distance(
                        end.distance()
                            .cloned()
                            .unwrap_or_else(|| self.default_distance()),
                    )),
                    EndKind::ThroughAll => match extrude.operation {
                        BodyOperation::Remove(_) | BodyOperation::Intersect(_) => {
                            Ok(ExtrudeEnd::ThroughAll)
                        }
                        BodyOperation::NewBody | BodyOperation::Add(_) => {
                            Err(THROUGH_ALL_NEEDS_A_CUT.to_owned())
                        }
                    },
                    EndKind::UpToNext => extrude
                        .operation
                        .target()
                        .map(|_| ExtrudeEnd::UpToNext)
                        .ok_or_else(|| UP_TO_NEXT_NEEDS_A_BODY.to_owned()),
                    EndKind::UpToFace => selected_target(self.model, self.selection, self.id())
                        .map(ExtrudeEnd::UpToFace),
                };
                let change = match candidate {
                    Ok(candidate) => self
                        .change(with_extent_of(extrude, rebuild(candidate)))
                        .map(Action::Apply),
                    Err(_) if kind == EndKind::UpToFace => Ok(Action::Editing(
                        EditingCommand::Pick(Picking::new(self.id(), Slot::ExtrudeTarget(side))),
                    )),
                    Err(reason) => Err(reason),
                };
                Choice {
                    label: kind.label().to_owned(),
                    selected: kind == current,
                    change,
                }
            })
            .collect()
    }

    fn end_rows(
        &mut self,
        ui: &mut Ui,
        rows: &EndRows<'_>,
        extrude: &Extrude,
        end: &ExtrudeEnd,
        rebuild: &dyn Fn(ExtrudeEnd) -> ExtrudeExtent,
    ) {
        widgets::caption(ui, rows.end);
        let salt = format!("{}-end", rows.salt);
        self.combo(ui, &salt, EndKind::of(end).label(), |panel| {
            panel.end_choices(extrude, end, rows.side, rebuild)
        });
        ui.end_row();
        let slot = Slot::ExtrudeTarget(rows.side);
        let picker = Picker {
            feature: self.id(),
            slot,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), slot),
                || target_change(self.model, self.selection, self.id(), extrude, rows.side),
            ),
            hover: "Run up to the selected flat face or plane instead",
        };
        match end {
            ExtrudeEnd::Distance(distance) => {
                self.expression(
                    ui,
                    rows.distance,
                    rows.salt,
                    distance,
                    (Dimension::LENGTH, rows.rule),
                    |distance| with_extent_of(extrude, rebuild(ExtrudeEnd::Distance(distance))),
                );
            }
            ExtrudeEnd::UpToFace(target) => {
                let shown = Shown::Named(capitalized(&describe_plane(self.document(), target)));
                feature_fields::reference_row(
                    ui,
                    self.model,
                    rows.face,
                    shown,
                    picker,
                    None,
                    self.actions,
                );
                return;
            }
            ExtrudeEnd::ThroughAll | ExtrudeEnd::UpToNext => {}
        }
        if self.picking(ui, slot) {
            feature_fields::reference_row(
                ui,
                self.model,
                rows.face,
                Shown::NoneChosen,
                picker,
                None,
                self.actions,
            );
        }
    }

    fn extent_row(&mut self, ui: &mut Ui, extrude: &Extrude) {
        let fallback = self.default_distance();
        let current = extent_name(&extrude.extent);
        let segments = [Shape::OneSide, Shape::Symmetric, Shape::TwoSides]
            .into_iter()
            .map(|shape| {
                let extent = reshaped(&extrude.extent, shape, &fallback);
                let label = extent_name(&extent);
                Segment {
                    label,
                    hover: shape.description(),
                    change: (label != current)
                        .then(|| self.change(with_extent_of(extrude, extent))),
                }
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Extent", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn extrude_rows(&mut self, ui: &mut Ui, extrude: &Extrude) {
        match &extrude.extent {
            ExtrudeExtent::OneSide { end, reversed } => {
                let reversed = *reversed;
                let rows = EndRows {
                    end: "End",
                    distance: "Distance",
                    face: "Up to",
                    salt: "distance",
                    side: Side::One,
                    rule: Rule::AboveZeroOrReverse,
                };
                self.end_rows(ui, &rows, extrude, end, &|end| ExtrudeExtent::OneSide {
                    end,
                    reversed,
                });
                if let Some(flipped) = feature_fields::reverse_row(ui, REVERSE_DIRECTION, reversed)
                {
                    let flipped = with_extent_of(
                        extrude,
                        ExtrudeExtent::OneSide {
                            end: end.clone(),
                            reversed: flipped,
                        },
                    );
                    self.apply(flipped);
                }
            }
            ExtrudeExtent::Symmetric { distance } => {
                self.expression(
                    ui,
                    "Total distance",
                    "distance",
                    distance,
                    (Dimension::LENGTH, Rule::AboveZero),
                    |distance| {
                        SolidFeature::Extrude(Extrude {
                            extent: ExtrudeExtent::Symmetric { distance },
                            ..extrude.clone()
                        })
                    },
                );
            }
            ExtrudeExtent::TwoSides { forward, backward } => {
                let forward_rows = EndRows {
                    end: "Forward end",
                    distance: "Forward distance",
                    face: "Forward up to",
                    salt: "forward",
                    side: Side::Forward,
                    rule: Rule::AboveZero,
                };
                self.end_rows(ui, &forward_rows, extrude, forward, &|end| {
                    ExtrudeExtent::TwoSides {
                        forward: end,
                        backward: backward.clone(),
                    }
                });
                let backward_rows = EndRows {
                    end: "Backward end",
                    distance: "Backward distance",
                    face: "Backward up to",
                    salt: "backward",
                    side: Side::Backward,
                    rule: Rule::AboveZero,
                };
                self.end_rows(ui, &backward_rows, extrude, backward, &|end| {
                    ExtrudeExtent::TwoSides {
                        forward: forward.clone(),
                        backward: end,
                    }
                });
            }
        }
        self.start_rows(ui, extrude.start.as_ref());
    }

    fn degrees_of(&self, angle: &Expression) -> Option<f64> {
        self.model
            .parameters()
            .evaluate_expression(angle)
            .ok()
            .map(|value| value.value)
    }

    fn two_angles(&self, angle: &Expression) -> RevolveExtent {
        let fits = self
            .degrees_of(angle)
            .is_some_and(|value| value + DEFAULT_BACKWARD_ANGLE <= FULL_TURN_DEGREES);
        RevolveExtent::TwoSides {
            forward: if fits {
                angle.clone()
            } else {
                solid_tools::degrees(DEFAULT_PARTIAL_ANGLE)
            },
            backward: solid_tools::degrees(DEFAULT_BACKWARD_ANGLE),
        }
    }

    fn turn_row(&mut self, ui: &mut Ui, revolve: &Revolve) {
        widgets::caption(ui, "Extent");
        let current = turn_name(&revolve.extent);
        self.combo(ui, "revolve-extent", current, |panel| {
            let angle = match &revolve.extent {
                RevolveExtent::Full => solid_tools::degrees(DEFAULT_PARTIAL_ANGLE),
                RevolveExtent::OneSide { angle, .. }
                | RevolveExtent::Symmetric { angle }
                | RevolveExtent::TwoSides { forward: angle, .. } => angle.clone(),
            };
            let two_angles = match &revolve.extent {
                RevolveExtent::TwoSides { .. } => revolve.extent.clone(),
                RevolveExtent::Full
                | RevolveExtent::OneSide { .. }
                | RevolveExtent::Symmetric { .. } => panel.two_angles(&angle),
            };
            [
                RevolveExtent::Full,
                RevolveExtent::OneSide {
                    angle: angle.clone(),
                    reversed: false,
                },
                RevolveExtent::Symmetric { angle },
                two_angles,
            ]
            .into_iter()
            .map(|extent| Choice {
                label: turn_name(&extent).to_owned(),
                selected: turn_name(&extent) == current,
                change: panel
                    .change(SolidFeature::Revolve(Revolve {
                        extent,
                        ..revolve.clone()
                    }))
                    .map(Action::Apply),
            })
            .collect()
        });
        ui.end_row();
    }

    fn start_rows(&mut self, ui: &mut Ui, start: Option<&SolidStart>) {
        let solid = self.solid;
        let on_face = start.and_then(SolidStart::target);
        widgets::caption(ui, "Start");
        self.combo(
            ui,
            "start-kind",
            if on_face.is_some() {
                START_ON_FACE
            } else {
                START_BY_OFFSET
            },
            |panel| {
                let kept = start.filter(|start| start.distance().is_some()).cloned();
                let offset = Choice {
                    label: START_BY_OFFSET.to_owned(),
                    selected: on_face.is_none(),
                    change: panel
                        .change(solid_tools::with_start(solid, kept))
                        .map(Action::Apply),
                };
                let face = Choice {
                    label: START_ON_FACE.to_owned(),
                    selected: on_face.is_some(),
                    change: match start_change(panel.model, panel.selection, panel.id(), solid) {
                        Ok(transaction) => Ok(Action::Apply(transaction)),
                        Err(_) => Ok(Action::Editing(EditingCommand::Pick(Picking::new(
                            panel.id(),
                            Slot::StartPlane,
                        )))),
                    },
                };
                vec![offset, face]
            },
        );
        ui.end_row();
        let picker = Picker {
            feature: self.id(),
            slot: Slot::StartPlane,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::StartPlane),
                || start_change(self.model, self.selection, self.id(), solid),
            ),
            hover: "Start from the selected flat face or plane instead",
        };
        if let Some(target) = on_face {
            let shown = Shown::Named(capitalized(&describe_plane(self.document(), target)));
            let removed = feature_fields::reference_row(
                ui,
                self.model,
                "Starts at",
                shown,
                picker,
                Some("Start at the sketch plane again"),
                self.actions,
            );
            if removed {
                self.apply(solid_tools::with_start(solid, None));
            }
            return;
        }
        if self.picking(ui, Slot::StartPlane) {
            feature_fields::reference_row(
                ui,
                self.model,
                "Starts at",
                Shown::NoneChosen,
                picker,
                None,
                self.actions,
            );
        }
        let distance = start
            .and_then(SolidStart::distance)
            .cloned()
            .unwrap_or_else(|| self.model.length_unit().default_length(0.0));
        self.expression(
            ui,
            START_OFFSET,
            "start",
            &distance,
            (Dimension::LENGTH, Rule::Any),
            |value| {
                solid_tools::with_start(
                    solid,
                    (!is_zero(&value)).then_some(SolidStart::Distance(value)),
                )
            },
        );
    }

    fn axis_row(&mut self, ui: &mut Ui, revolve: &Revolve) {
        widgets::caption(ui, "Axis");
        let model = self.model;
        let sketch = model
            .document()
            .feature(revolve.sketch)
            .and_then(|feature| feature.kind.sketch());
        let axis_name = solid_tools::axis_name(model.document(), revolve.sketch, &revolve.axis);
        ui.vertical(|ui| {
            self.combo(ui, "axis", axis_name.clone(), |panel| {
                let Some(sketch) = sketch else {
                    return Vec::new();
                };
                let lines = sketch
                    .entities()
                    .filter_map(|(id, entity)| matches!(entity, Entity::Line { .. }).then_some(id));
                let model_axis = revolve.axis.model().map(|_| Choice {
                    label: axis_name.clone(),
                    selected: true,
                    change: Err(String::new()),
                });
                [Reference::HorizontalAxis.id(), Reference::VerticalAxis.id()]
                    .into_iter()
                    .chain(lines)
                    .map(|axis| Choice {
                        label: sketch.entity_label(axis),
                        selected: revolve.axis == RevolveAxis::Sketch(axis),
                        change: panel
                            .change(SolidFeature::Revolve(Revolve {
                                axis: RevolveAxis::Sketch(axis),
                                ..revolve.clone()
                            }))
                            .map(Action::Apply),
                    })
                    .chain(model_axis)
                    .collect()
            });
            let picker = Picker {
                feature: self.id(),
                slot: Slot::RevolveAxis,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    self.selection,
                    (self.id(), Slot::RevolveAxis),
                    || selected_axis_change(model, self.selection, self.id(), revolve),
                ),
                hover: "Turn about the selected axis, edge or round face",
            };
            feature_fields::reference_picker(ui, model, picker, self.actions);
        });
        ui.end_row();
    }

    fn revolve_rows(&mut self, ui: &mut Ui, revolve: &Revolve) {
        self.axis_row(ui, revolve);
        match &revolve.extent {
            RevolveExtent::Full => {}
            RevolveExtent::OneSide { angle, reversed } => {
                let reversed = *reversed;
                self.expression(
                    ui,
                    "Angle",
                    "angle",
                    angle,
                    (Dimension::ANGLE, Rule::Turn),
                    |angle| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::OneSide { angle, reversed },
                            ..revolve.clone()
                        })
                    },
                );
                if let Some(flipped) = feature_fields::reverse_row(ui, REVERSE_DIRECTION, reversed)
                {
                    let flipped = SolidFeature::Revolve(Revolve {
                        extent: RevolveExtent::OneSide {
                            angle: angle.clone(),
                            reversed: flipped,
                        },
                        ..revolve.clone()
                    });
                    self.apply(flipped);
                }
            }
            RevolveExtent::Symmetric { angle } => {
                self.expression(
                    ui,
                    "Total angle",
                    "angle",
                    angle,
                    (Dimension::ANGLE, Rule::Turn),
                    |angle| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::Symmetric { angle },
                            ..revolve.clone()
                        })
                    },
                );
            }
            RevolveExtent::TwoSides { forward, backward } => {
                let beside =
                    |other: &Expression| Rule::TurnBeside(self.degrees_of(other).unwrap_or(0.0));
                let (forward_rule, backward_rule) = (beside(backward), beside(forward));
                self.expression(
                    ui,
                    "Forward",
                    "forward-angle",
                    forward,
                    (Dimension::ANGLE, forward_rule),
                    |forward| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::TwoSides {
                                forward,
                                backward: backward.clone(),
                            },
                            ..revolve.clone()
                        })
                    },
                );
                self.expression(
                    ui,
                    "Backward",
                    "backward-angle",
                    backward,
                    (Dimension::ANGLE, backward_rule),
                    |backward| {
                        SolidFeature::Revolve(Revolve {
                            extent: RevolveExtent::TwoSides {
                                forward: forward.clone(),
                                backward,
                            },
                            ..revolve.clone()
                        })
                    },
                );
            }
        }
        self.start_rows(ui, revolve.start.as_ref());
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let operation = self.solid.operation();
        let bodies = self.document().bodies_before(self.id());
        let fallback = bodies.last().copied();
        widgets::caption(ui, "Result");
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
                    change: panel
                        .change(solid_tools::with_operation(panel.solid, candidate))
                        .map(Action::Apply),
                })
                .collect()
        });
        ui.end_row();
        let Some(target) = operation.target() else {
            return;
        };
        widgets::caption(ui, "Body");
        let model = self.model;
        ui.horizontal(|ui| {
            let name = feature_fields::feature_name(model.document(), target);
            let text = feature_fields::combo_text(ui, name, MISSING_BODY);
            self.combo(ui, "body", text, |panel| {
                bodies
                    .iter()
                    .filter_map(|body| {
                        let label = panel.document().feature(*body)?.name.clone();
                        Some(Choice {
                            label,
                            selected: *body == target,
                            change: panel
                                .change(solid_tools::with_operation(
                                    panel.solid,
                                    operation.with_target(*body),
                                ))
                                .map(Action::Apply),
                        })
                    })
                    .collect()
            });
        });
        ui.end_row();
        if let BodyOperation::Remove(_) = operation {
            self.other_body_rows(ui, &bodies, target);
        }
    }

    fn other_body_rows(&mut self, ui: &mut Ui, bodies: &[FeatureId], target: FeatureId) {
        let others = self.solid.other_bodies().to_vec();
        for (index, other) in others.iter().enumerate() {
            if index == 0 {
                widgets::caption(ui, ALSO_CUTS);
            } else {
                ui.label("");
            }
            let name = feature_fields::feature_name(self.document(), *other);
            let hover = format!(
                "Stop cutting {}",
                name.unwrap_or("the missing body")
            );
            let mut dropped = false;
            ui.horizontal(|ui| {
                match &name {
                    Some(name) => {
                        ui.label(*name);
                    }
                    None => feature_fields::missing(ui, MISSING_BODY),
                }
                dropped = widgets::icon_button(ui, icons::REMOVE, &hover).clicked();
            });
            ui.end_row();
            if dropped {
                let kept = others
                    .iter()
                    .copied()
                    .filter(|kept| kept != other)
                    .collect();
                self.apply(solid_tools::with_other_bodies(self.solid, kept));
            }
        }
        let candidates: Vec<FeatureId> = bodies
            .iter()
            .copied()
            .filter(|body| *body != target && !others.contains(body))
            .collect();
        if candidates.is_empty() {
            return;
        }
        if others.is_empty() {
            widgets::caption(ui, ALSO_CUTS);
        } else {
            ui.label("");
        }
        self.combo(ui, "also-cut", ADD_CUT_BODY, |panel| {
            candidates
                .iter()
                .filter_map(|body| {
                    let label = panel.document().feature(*body)?.name.clone();
                    let mut chosen = others.clone();
                    chosen.push(*body);
                    Some(Choice {
                        label,
                        selected: false,
                        change: panel
                            .change(solid_tools::with_other_bodies(panel.solid, chosen))
                            .map(Action::Apply),
                    })
                })
                .collect()
        });
        ui.end_row();
    }

    fn apply(&mut self, solid: SolidFeature) {
        let change = self.change(solid);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }
}

fn is_zero(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Number(value) | Expression::Measure(value, _) if *value == 0.0
    )
}

fn with_extent_of(extrude: &Extrude, extent: ExtrudeExtent) -> SolidFeature {
    SolidFeature::Extrude(Extrude {
        extent,
        ..extrude.clone()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    OneSide,
    Symmetric,
    TwoSides,
}

impl Shape {
    fn description(self) -> &'static str {
        match self {
            Self::OneSide => "Extrude from the sketch to one side",
            Self::Symmetric => "Extrude the same distance to both sides of the sketch",
            Self::TwoSides => "Extrude to each side by its own end",
        }
    }
}

fn reshaped(extent: &ExtrudeExtent, shape: Shape, fallback: &Expression) -> ExtrudeExtent {
    let distance = extent
        .ends()
        .into_iter()
        .find_map(ExtrudeEnd::distance)
        .or(match extent {
            ExtrudeExtent::Symmetric { distance } => Some(distance),
            ExtrudeExtent::OneSide { .. } | ExtrudeExtent::TwoSides { .. } => None,
        })
        .unwrap_or(fallback)
        .clone();
    match (extent, shape) {
        (ExtrudeExtent::OneSide { .. }, Shape::OneSide)
        | (ExtrudeExtent::Symmetric { .. }, Shape::Symmetric)
        | (ExtrudeExtent::TwoSides { .. }, Shape::TwoSides) => extent.clone(),
        (ExtrudeExtent::TwoSides { forward: end, .. }, Shape::OneSide) => ExtrudeExtent::OneSide {
            end: end.clone(),
            reversed: false,
        },
        (ExtrudeExtent::Symmetric { .. }, Shape::OneSide) => {
            ExtrudeExtent::one_side(distance, false)
        }
        (ExtrudeExtent::OneSide { .. } | ExtrudeExtent::TwoSides { .. }, Shape::Symmetric) => {
            ExtrudeExtent::Symmetric { distance }
        }
        (ExtrudeExtent::OneSide { end, reversed }, Shape::TwoSides) => {
            let other = ExtrudeEnd::Distance(distance);
            if *reversed {
                ExtrudeExtent::TwoSides {
                    forward: other,
                    backward: end.clone(),
                }
            } else {
                ExtrudeExtent::TwoSides {
                    forward: end.clone(),
                    backward: other,
                }
            }
        }
        (ExtrudeExtent::Symmetric { .. }, Shape::TwoSides) => {
            ExtrudeExtent::two_sides(distance.clone(), distance)
        }
    }
}

fn not_a_target(model: &Model, pickable: Pickable, index: usize) -> Option<&'static str> {
    match pickable {
        Pickable::Face { body, face } => {
            match sketch_placement::attachment_at(model, FaceChoice { body, face }, index) {
                Err(sketch_placement::NOT_FLAT) => Some(CURVED_TARGET),
                Err(reason) => Some(reason),
                Ok(_) => None,
            }
        }
        Pickable::Datum(datum) if datum_tools::is_plane(model.document(), datum) => {
            Some("The selected plane comes after this feature in the tree")
        }
        Pickable::Datum(_) => Some("The selected datum is not a plane"),
        _ => None,
    }
}

pub fn selected_target(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
) -> Result<PlaneReference, String> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    let mut targets = Vec::new();
    let mut refused = None;
    for pickable in selection.iter() {
        match datum_tools::plane_reference(model, pickable, index) {
            Some(target) => targets.push(target),
            None => {
                refused = refused.or_else(|| not_a_target(model, pickable, index));
            }
        }
    }
    match targets.as_slice() {
        [target] => Ok(target.clone()),
        [] => Err(refused.unwrap_or(NO_TARGET_SELECTED).to_owned()),
        _ => Err("Select only one face or plane".to_owned()),
    }
}

fn with_end_on(extent: &ExtrudeExtent, side: Side, end: ExtrudeEnd) -> Option<ExtrudeExtent> {
    match (extent, side) {
        (ExtrudeExtent::OneSide { reversed, .. }, Side::One) => Some(ExtrudeExtent::OneSide {
            end,
            reversed: *reversed,
        }),
        (ExtrudeExtent::TwoSides { backward, .. }, Side::Forward) => {
            Some(ExtrudeExtent::TwoSides {
                forward: end,
                backward: backward.clone(),
            })
        }
        (ExtrudeExtent::TwoSides { forward, .. }, Side::Backward) => {
            Some(ExtrudeExtent::TwoSides {
                forward: forward.clone(),
                backward: end,
            })
        }
        _ => None,
    }
}

pub fn target_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    extrude: &Extrude,
    side: Side,
) -> Result<Transaction, String> {
    let end = ExtrudeEnd::UpToFace(selected_target(model, selection, feature)?);
    let extent = with_end_on(&extrude.extent, side, end).ok_or_else(|| SIDES_CHANGED.to_owned())?;
    if extent == extrude.extent {
        return Err("This end already runs up to the selected face or plane".to_owned());
    }
    change(model, feature, with_extent_of(extrude, extent))
}

pub fn up_to_selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    extrude: &Extrude,
) -> Result<Transaction, String> {
    let end = ExtrudeEnd::UpToFace(selected_target(model, selection, feature)?);
    let extent = match &extrude.extent {
        ExtrudeExtent::OneSide { reversed, .. } => ExtrudeExtent::OneSide {
            end,
            reversed: *reversed,
        },
        ExtrudeExtent::Symmetric { .. } => ExtrudeExtent::OneSide {
            end,
            reversed: false,
        },
        ExtrudeExtent::TwoSides { backward, .. } => ExtrudeExtent::TwoSides {
            forward: end,
            backward: backward.clone(),
        },
    };
    if extent == extrude.extent {
        return Err("The extrusion already runs up to the selected face or plane".to_owned());
    }
    change(model, feature, with_extent_of(extrude, extent))
}

pub fn start_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    solid: &SolidFeature,
) -> Result<Transaction, String> {
    let target = selected_target(model, selection, feature)?;
    if solid.start().and_then(SolidStart::target) == Some(&target) {
        return Err("This feature already starts from the selected face or plane".to_owned());
    }
    change(
        model,
        feature,
        solid_tools::with_start(solid, Some(SolidStart::Plane(target))),
    )
}

pub fn selected_axis_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    revolve: &Revolve,
) -> Result<Transaction, String> {
    let index = model.document().feature_index(feature).unwrap_or(0);
    let chosen = selection
        .iter()
        .find_map(|pickable| datum_tools::axis_reference(model, pickable, index))
        .map(|axis| match axis {
            AxisReference::Sketch { sketch, entity } if sketch == revolve.sketch => {
                RevolveAxis::Sketch(entity)
            }
            axis => RevolveAxis::Model(axis),
        });
    match chosen {
        Some(axis) if revolve.axis == axis => {
            Err("The revolve already turns about the selected axis".to_owned())
        }
        Some(axis) => change(
            model,
            feature,
            SolidFeature::Revolve(Revolve {
                axis,
                ..revolve.clone()
            }),
        ),
        None => Err(
            "Select an axis, a straight edge or a round face made before this feature".to_owned(),
        ),
    }
}

fn change(model: &Model, feature: FeatureId, solid: SolidFeature) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = solid_tools::edit(document, feature, solid)
        .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(document, transaction)
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
            axis: match &revolve.axis {
                RevolveAxis::Sketch(line) if !line.is_reference() => {
                    RevolveAxis::Sketch(EntityId::VERTICAL_AXIS)
                }
                RevolveAxis::Sketch(_) | RevolveAxis::Model(_) => revolve.axis.clone(),
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
        RevolveExtent::TwoSides { .. } => "Two angles",
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
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    solid: &SolidFeature,
    opened: bool,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        solid,
        actions,
    };
    widgets::properties(ui, ("solid-properties", feature.id()), |ui| {
        match solid {
            SolidFeature::Extrude(extrude) => panel.extent_row(ui, extrude),
            SolidFeature::Revolve(revolve) => panel.turn_row(ui, revolve),
        }
        panel.sketch_row(ui);
        panel.regions_row(ui, opened);
        match solid {
            SolidFeature::Extrude(extrude) => panel.extrude_rows(ui, extrude),
            SolidFeature::Revolve(revolve) => panel.revolve_rows(ui, revolve),
        }
        panel.operation_rows(ui);
    });
}
