use caditor_document::{
    Feature, FeatureId, Hole, HoleBottom, HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard,
    HoleStep, HoleStyle, MAX_HOLE_STEPS, MetricSize, TappedThread, ThreadHand, ThreadSide,
    Transaction, capitalized, circle_sizes, describe_plane, hole_thread, pitch_text,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Label, Ui};

use crate::{
    editing::EditingCommand,
    feature_fields::{self, Choice, Picker, Quantity, REVERSE_DIRECTION, Rule, Segment, Shown},
    field,
    hole_tools::{self, DEFAULT_STEP_DIAMETER, Kind},
    icons,
    model::{Action, Model},
    reference_picking::{self, Picking, Slot},
    selection::Selection,
    sketch_placement,
    solid_panel::{self, END_OFFSET},
    widgets,
};

pub const DESCRIPTION: &str = "Drills a hole at every point of the sketch";
const NOT_A_BODY: &str = "A body that is no longer there";
pub const CUSTOM_SIZE: &str = "Custom";
pub const SIZED_BY_CIRCLES: &str = "Circles";
const SIZED_BY_CIRCLES_NOTE: &str = "Holes at circles take each circle's diameter; holes at \
                                     points take the diameter below";
const SCALED_BY_CIRCLES_NOTE: &str = "Holes at circles take each circle's diameter, and their \
                                      counterbore or countersink grows with it from the sizes \
                                      below; holes at points take the sizes below";
pub const SCALE_HEADS: &str = "Scale the counterbore or countersink with each circle";
pub const PITCH: &str = "Pitch";
pub const DRILL_POINT: &str = "Drill point";
pub const DRILL_POINT_ANGLE: &str = "Drill point angle";
pub const ADD_STEP: &str = "Add a step";
pub const REMOVE_STEP: &str = "Remove the last step";
pub const THREAD_CLASS: &str = "Thread class";
pub const THREAD_HAND: &str = "Thread hand";
pub const THREAD_LENGTH: &str = "Thread length";
pub const THREAD_DEPTH: &str = "Thread depth";
pub const WHOLE_BORE: &str = "Whole bore";
pub const TO_A_DEPTH: &str = "To a depth";
pub const BLIND: &str = "Blind";
pub const THROUGH_ALL: &str = "Through all";
pub const UP_TO_NEXT: &str = "Up to next";
pub const UP_TO_FACE: &str = "Up to face";
const DEFAULT_THREAD_DEPTH: f64 = 10.0;
pub const PLACED_ON: &str = "Placed on";
pub const POSITION_CAPTIONS: [&str; 2] = ["Position X", "Position Y"];
pub const EDIT_SKETCH: &str = "Edit the sketch";
const EDIT_SKETCH_HOVER: &str = "Show and edit the sketch holding the hole's points, to \
                                 dimension them or add more holes";
const PLACE_HOVER: &str = "Drill the hole on the selected flat face instead, at its middle";
const NOT_ON_A_FACE: &str = "Not on a face";

fn depth_label(depth: &HoleDepth) -> &'static str {
    match depth {
        HoleDepth::Blind(_) => BLIND,
        HoleDepth::ThroughAll => THROUGH_ALL,
        HoleDepth::UpToNext { .. } => UP_TO_NEXT,
        HoleDepth::UpToFace { .. } => UP_TO_FACE,
    }
}

fn sizing_label(sizing: HoleSizing) -> &'static str {
    match sizing {
        HoleSizing::Typed => "Diameter",
        HoleSizing::Circles | HoleSizing::CirclesAndHeads => SIZED_BY_CIRCLES,
    }
}

fn sizing_hover(sizing: HoleSizing) -> &'static str {
    match sizing {
        HoleSizing::Typed => "Every hole takes the typed diameter, whatever the circle drawn",
        HoleSizing::Circles | HoleSizing::CirclesAndHeads => {
            "Each hole at a circle takes that circle's diameter"
        }
    }
}

fn insert_note(standard: HoleStandard) -> Option<String> {
    let insert = standard.insert()?;
    Some(format!(
        "For a standard {} heat-set insert {} mm long: make the hole at least that deep and \
         leave at least {} mm of material around it",
        standard.size.name(),
        pitch_text(insert.length),
        pitch_text(insert.wall)
    ))
}

#[derive(Clone, Copy)]
struct Field {
    caption: &'static str,
    key: &'static str,
    dimension: Dimension,
    rule: Rule,
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    hole: &'a Hole,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, hole: Hole) -> Result<Transaction, String> {
        let document = self.model.document();
        let transaction = hole_tools::edit(document, self.id(), hole)
            .ok_or_else(|| "The feature no longer exists".to_owned())?;
        field::checked(document, transaction)
    }

    fn size_row(&mut self, ui: &mut Ui) {
        let unit = self.model.length_unit();
        let current = self.hole.standard;
        widgets::caption(ui, "Size");
        let selected = current.map_or(CUSTOM_SIZE, |standard| standard.size.name());
        let name = self.feature.name.clone();
        let chosen = feature_fields::combo(ui, Id::new(("hole-size", self.id())), selected, || {
            let custom = Choice {
                label: CUSTOM_SIZE.to_owned(),
                selected: current.is_none(),
                change: self
                    .change(Hole {
                        standard: None,
                        ..self.hole.clone()
                    })
                    .map(|transaction| feature_fields::applied(&name, Ok(transaction))),
            };
            let sizes = MetricSize::ALL.into_iter().map(|size| {
                let fit = current.map_or(HoleFit::Normal, |standard| standard.fit);
                let standard = HoleStandard::offered(size, fit);
                Choice {
                    label: size.name().to_owned(),
                    selected: current.is_some_and(|current| current.size == size),
                    change: self
                        .change(hole_tools::with_standard(self.hole, standard, unit))
                        .map(|transaction| feature_fields::applied(&name, Ok(transaction))),
                }
            });
            std::iter::once(custom).chain(sizes).collect()
        });
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn fit_row(&mut self, ui: &mut Ui, standard: HoleStandard) {
        let unit = self.model.length_unit();
        let segments = standard
            .size
            .fits()
            .into_iter()
            .map(|fit| Segment {
                label: fit.label(),
                hover: fit.description(),
                change: (!fit.same_kind(standard.fit)).then(|| {
                    self.change(hole_tools::with_standard(
                        self.hole,
                        HoleStandard { fit, ..standard },
                        unit,
                    ))
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Fit", &self.feature.name, segments);
        self.actions.extend(chosen);
        self.pitch_row(ui, standard);
        if let Some(note) = insert_note(standard) {
            feature_fields::description_row(ui, &note);
        }
        if let Some(thread) = standard.thread() {
            feature_fields::description_row(
                ui,
                &format!(
                    "Thread {thread}: drawn on each bore and named in the exports, printed at the \
                     tap drill size, then cut with a tap or by a self-tapping screw"
                ),
            );
        }
        self.thread_rows(ui);
    }

    fn thread_rows(&mut self, ui: &mut Ui) {
        let Some(designation) = hole_thread(self.hole) else {
            return;
        };
        let family = designation.size.family();
        let current = designation.class;
        let default = family.default_class(ThreadSide::Internal);
        let name = self.feature.name.clone();
        widgets::caption(ui, THREAD_CLASS);
        let chosen = feature_fields::combo(
            ui,
            Id::new(("hole-thread-class", self.id())),
            current.label(),
            || {
                family
                    .classes(ThreadSide::Internal)
                    .iter()
                    .map(|class| Choice {
                        label: class.label().to_owned(),
                        selected: *class == current,
                        change: self
                            .with_thread(TappedThread {
                                class: (*class != default).then_some(*class),
                                ..self.hole.thread.clone()
                            })
                            .map(|transaction| feature_fields::applied(&name, Ok(transaction))),
                    })
                    .collect()
            },
        );
        self.actions.extend(chosen);
        ui.end_row();
        let hands = ThreadHand::ALL
            .into_iter()
            .map(|hand| Segment {
                label: hand.label(),
                hover: match hand {
                    ThreadHand::Right => "Tightens turning clockwise, as most threads do",
                    ThreadHand::Left => "Tightens turning anticlockwise",
                },
                change: (hand != self.hole.thread.hand).then(|| {
                    self.with_thread(TappedThread {
                        hand,
                        ..self.hole.thread.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, THREAD_HAND, &self.feature.name, hands);
        self.actions.extend(chosen);
        self.thread_length_rows(ui);
    }

    fn thread_length_rows(&mut self, ui: &mut Ui) {
        let unit = self.model.length_unit();
        let depth = self.hole.thread.depth.clone();
        let options = [
            (
                WHOLE_BORE,
                "Thread the whole wall of each bore",
                None,
                depth.is_none(),
            ),
            (
                TO_A_DEPTH,
                "Thread a length down from the mouth of each bore",
                Some(unit.default_length(DEFAULT_THREAD_DEPTH)),
                depth.is_some(),
            ),
        ];
        let segments = options
            .into_iter()
            .map(|(label, hover, depth, current)| Segment {
                label,
                hover,
                change: (!current).then(|| {
                    self.with_thread(TappedThread {
                        depth,
                        ..self.hole.thread.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, THREAD_LENGTH, &self.feature.name, segments);
        self.actions.extend(chosen);
        if let Some(depth) = &depth {
            self.length_row(
                ui,
                Field {
                    caption: THREAD_DEPTH,
                    key: "thread-depth",
                    dimension: Dimension::LENGTH,
                    rule: Rule::AboveZero,
                },
                depth,
                |hole, value| Hole {
                    thread: TappedThread {
                        depth: Some(value),
                        ..hole.thread.clone()
                    },
                    ..hole.clone()
                },
            );
        }
    }

    fn with_thread(&self, thread: TappedThread) -> Result<Transaction, String> {
        self.change(Hole {
            thread,
            ..self.hole.clone()
        })
    }

    fn pitch_row(&mut self, ui: &mut Ui, standard: HoleStandard) {
        let fine = standard.size.fine_fits();
        if !standard.fit.is_fine() || fine.len() < 2 {
            return;
        }
        let unit = self.model.length_unit();
        let labels: Vec<String> = fine
            .iter()
            .map(|fit| {
                HoleStandard {
                    fit: *fit,
                    ..standard
                }
                .thread_pitch()
                .map_or_else(String::new, pitch_text)
            })
            .collect();
        let segments = fine
            .iter()
            .zip(&labels)
            .map(|(fit, label)| Segment {
                label,
                hover: "A fine pitch of ISO 261 for this size, in millimetres",
                change: (*fit != standard.fit).then(|| {
                    self.change(hole_tools::with_standard(
                        self.hole,
                        HoleStandard {
                            fit: *fit,
                            ..standard
                        },
                        unit,
                    ))
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, PITCH, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn shape_row(&mut self, ui: &mut Ui) {
        let unit = self.model.length_unit();
        let slot = matches!(self.hole.shape, HoleShape::Slot { .. });
        let options = [
            (
                "Round",
                "A round hole at each point",
                HoleShape::Round,
                !slot,
            ),
            (
                "Slot",
                "A slot centred on each point, for a part that must slide into place",
                hole_tools::default_slot(unit),
                slot,
            ),
        ];
        let segments = options
            .into_iter()
            .map(|(label, hover, shape, current)| Segment {
                label,
                hover,
                change: (!current).then(|| {
                    self.change(Hole {
                        shape,
                        ..self.hole.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Shape", &self.feature.name, segments);
        self.actions.extend(chosen);
        if let HoleShape::Slot { length, angle } = self.hole.shape.clone() {
            let turned = angle.clone();
            self.length_row(
                ui,
                Field {
                    caption: "Slot length",
                    key: "slot-length",
                    dimension: Dimension::LENGTH,
                    rule: Rule::AboveZero,
                },
                &length,
                |hole, value| Hole {
                    shape: HoleShape::Slot {
                        length: value,
                        angle: turned.clone(),
                    },
                    ..hole.clone()
                },
            );
            self.length_row(
                ui,
                Field {
                    caption: "Slot angle",
                    key: "slot-angle",
                    dimension: Dimension::ANGLE,
                    rule: Rule::Any,
                },
                &angle,
                |hole, value| Hole {
                    shape: HoleShape::Slot {
                        length: length.clone(),
                        angle: value,
                    },
                    ..hole.clone()
                },
            );
        }
    }

    fn has_circles(&self) -> bool {
        self.model
            .document()
            .feature(self.hole.sketch)
            .and_then(|feature| feature.kind.sketch())
            .is_some_and(|sketch| !circle_sizes(sketch).is_empty())
    }

    fn sizing_row(&mut self, ui: &mut Ui) {
        if self.hole.sizing == HoleSizing::Typed && !self.has_circles() {
            return;
        }
        let segments = [HoleSizing::Typed, HoleSizing::CirclesAndHeads]
            .into_iter()
            .map(|sizing| Segment {
                label: sizing_label(sizing),
                hover: sizing_hover(sizing),
                change: (sizing.by_circles() != self.hole.sizing.by_circles()).then(|| {
                    self.change(Hole {
                        sizing,
                        ..self.hole.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Sized by", &self.feature.name, segments);
        self.actions.extend(chosen);
        match self.hole.sizing {
            HoleSizing::Typed => return,
            HoleSizing::Circles => feature_fields::description_row(ui, SIZED_BY_CIRCLES_NOTE),
            HoleSizing::CirclesAndHeads => {
                feature_fields::description_row(ui, SCALED_BY_CIRCLES_NOTE);
            }
        }
        if self.hole.style == HoleStyle::Plain {
            return;
        }
        let scaled = self.hole.sizing == HoleSizing::CirclesAndHeads;
        if let Some(scale) = feature_fields::reverse_row(ui, SCALE_HEADS, scaled) {
            let sizing = if scale {
                HoleSizing::CirclesAndHeads
            } else {
                HoleSizing::Circles
            };
            let change = self.change(Hole {
                sizing,
                ..self.hole.clone()
            });
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }

    fn style_row(&mut self, ui: &mut Ui) {
        let current = Kind::of(&self.hole.style);
        let unit = self.model.length_unit();
        let standard = self.hole.standard;
        let segments = Kind::ALL
            .into_iter()
            .map(|kind| Segment {
                label: kind.label(),
                hover: kind.description(),
                change: (kind != current).then(|| {
                    self.change(Hole {
                        style: kind.style_for(standard, unit),
                        ..self.hole.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Style", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn depth_row(&mut self, ui: &mut Ui) {
        let unit = self.model.length_unit();
        let depth = &self.hole.depth;
        let kept = depth.offset().cloned();
        let current = depth_label(depth);
        widgets::caption(ui, "Depth");
        let chosen = feature_fields::combo(ui, Id::new(("hole-depth", self.id())), current, || {
            let options = [
                (BLIND, Ok(HoleDepth::Blind(hole_tools::default_depth(unit)))),
                (THROUGH_ALL, Ok(HoleDepth::ThroughAll)),
                (
                    UP_TO_NEXT,
                    Ok(HoleDepth::up_to_next().with_offset(kept.clone())),
                ),
                (
                    UP_TO_FACE,
                    solid_panel::selected_target(self.model, self.selection, self.id())
                        .map(|target| HoleDepth::up_to_face(target).with_offset(kept.clone())),
                ),
            ];
            options
                .into_iter()
                .map(|(label, depth)| Choice {
                    label: label.to_owned(),
                    selected: label == current,
                    change: match depth {
                        Ok(depth) => self
                            .change(Hole {
                                depth,
                                ..self.hole.clone()
                            })
                            .map(Action::Apply),
                        Err(_) => Ok(Action::Editing(EditingCommand::Pick(Picking::new(
                            self.id(),
                            Slot::HoleTarget,
                        )))),
                    },
                })
                .collect()
        });
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn end_rows(&mut self, ui: &mut Ui) {
        let id = self.id();
        let picker = Picker {
            feature: id,
            slot: Slot::HoleTarget,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::HoleTarget),
                || target_change(self.model, self.selection, id, self.hole),
            ),
            hover: "Drill up to the selected flat face or plane instead",
        };
        let picking = reference_picking::current(ui.ctx())
            .is_some_and(|picking| picking.is_for(id, Slot::HoleTarget));
        let shown = match self.hole.depth.target() {
            Some(target) => Some(Shown::Named(capitalized(&describe_plane(
                self.model.document(),
                target,
            )))),
            None if picking => Some(Shown::NoneChosen),
            None => None,
        };
        if let Some(shown) = shown {
            feature_fields::reference_row(
                ui,
                self.model,
                "Up to",
                shown,
                picker,
                None,
                self.actions,
            );
        }
        if !self.hole.depth.takes_offset() {
            return;
        }
        let offset = self
            .hole
            .depth
            .offset()
            .cloned()
            .unwrap_or_else(|| self.model.length_unit().default_length(0.0));
        self.length_row(
            ui,
            Field {
                caption: END_OFFSET,
                key: "end-offset",
                dimension: Dimension::LENGTH,
                rule: Rule::Any,
            },
            &offset,
            |hole, value| {
                let kept = (!solid_panel::is_zero(&value)).then_some(value);
                Hole {
                    depth: hole.depth.clone().with_offset(kept),
                    ..hole.clone()
                }
            },
        );
    }

    fn bottom_rows(&mut self, ui: &mut Ui) {
        let blind = matches!(self.hole.depth, HoleDepth::Blind(_));
        let round = matches!(self.hole.shape, HoleShape::Round);
        if !blind || !round {
            return;
        }
        let pointed = matches!(self.hole.bottom, HoleBottom::DrillPoint(_));
        if let Some(pointed) = feature_fields::reverse_row(ui, DRILL_POINT, pointed) {
            let bottom = if pointed {
                hole_tools::default_drill_point()
            } else {
                HoleBottom::Flat
            };
            let change = self.change(Hole {
                bottom,
                ..self.hole.clone()
            });
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
        if let HoleBottom::DrillPoint(angle) = &self.hole.bottom {
            self.length_row(
                ui,
                Field {
                    caption: DRILL_POINT_ANGLE,
                    key: "drill-point-angle",
                    dimension: Dimension::ANGLE,
                    rule: Rule::ConeAngle,
                },
                angle,
                |hole, value| Hole {
                    bottom: HoleBottom::DrillPoint(value),
                    ..hole.clone()
                },
            );
        }
    }

    fn length_row(
        &mut self,
        ui: &mut Ui,
        field: Field,
        shown: &Expression,
        rebuild: impl Fn(&Hole, Expression) -> Hole,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("hole-field", field.key, self.id())),
            expression: shown,
            dimension: field.dimension,
            rule: field.rule,
        };
        let hole = self.hole;
        let drafting = feature_fields::expression_row_drafting(
            ui,
            self.model,
            field.caption,
            quantity,
            |value| self.change(rebuild(hole, value)),
        );
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn body_row(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let id = self.id();
        widgets::caption(ui, "Body");
        let current = self.hole.body;
        let selected = feature_fields::combo_text(
            ui,
            feature_fields::feature_name(document, current),
            NOT_A_BODY,
        );
        let chosen = feature_fields::combo(ui, Id::new(("hole-body", id)), selected, || {
            document
                .bodies_before(id)
                .into_iter()
                .filter_map(|body| {
                    Some(Choice {
                        label: feature_fields::feature_name(document, body)?.to_owned(),
                        selected: body == current,
                        change: self
                            .change(Hole {
                                body,
                                ..self.hole.clone()
                            })
                            .map(|transaction| {
                                feature_fields::applied(&self.feature.name, Ok(transaction))
                            }),
                    })
                })
                .collect()
        });
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn placement_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let Some(point) = hole_tools::lone_point(document, self.hole) else {
            return;
        };
        let id = self.id();
        widgets::caption(ui, PLACED_ON);
        let picker = Picker {
            feature: id,
            slot: Slot::HolePlace,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::HolePlace),
                || hole_tools::place_change(self.model, self.selection, id, self.hole),
            ),
            hover: PLACE_HOVER,
        };
        let text = document
            .feature(self.hole.sketch)
            .and_then(|sketch| sketch.kind.attachment())
            .map_or_else(
                || NOT_ON_A_FACE.to_owned(),
                |attachment| capitalized(&sketch_placement::describe(document, attachment)),
            );
        ui.vertical(|ui| {
            ui.add(Label::new(text).wrap());
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
        let unit = self.model.length_unit();
        for (index, caption) in POSITION_CAPTIONS.into_iter().enumerate() {
            let along = if index == 0 { point.at.x } else { point.at.y };
            let expression = unit.measured(along);
            let quantity = Quantity {
                feature: id,
                id: Id::new(("hole-position", id, index)),
                expression: &expression,
                dimension: Dimension::LENGTH,
                rule: Rule::Any,
            };
            let parameters = self.model.parameters();
            let drafting = feature_fields::expression_row_drafting(
                ui,
                self.model,
                caption,
                quantity,
                |value| {
                    let millimetres = parameters
                        .evaluate_expression(&value)
                        .map_err(|error| field::sentence(&error.to_string()))?
                        .value;
                    let mut at = point.at;
                    if index == 0 {
                        at.x = millimetres;
                    } else {
                        at.y = millimetres;
                    }
                    hole_tools::moved(document, id, self.hole, at)
                        .and_then(|transaction| field::checked(document, transaction))
                },
            );
            self.actions.extend(drafting.into_actions(id));
        }
    }

    fn sketch_row(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let sketch = self.hole.sketch;
        widgets::caption(ui, "Sketch");
        ui.vertical(|ui| match feature_fields::feature_name(document, sketch) {
            Some(name) => {
                ui.label(name);
                let button = widgets::small_button(ui, icons::EDIT, EDIT_SKETCH);
                if ui.add(button).on_hover_text(EDIT_SKETCH_HOVER).clicked() {
                    self.actions
                        .push(Action::Editing(EditingCommand::Enter(sketch)));
                }
            }
            None => feature_fields::missing(ui, feature_fields::MISSING_BODY),
        });
        ui.end_row();
    }

    fn millimetres_of(&self, length: &Expression) -> Option<f64> {
        self.model
            .parameters()
            .evaluate_expression(length)
            .ok()
            .map(|value| value.value)
    }

    fn with_steps(&self, steps: Vec<HoleStep>) -> Result<Transaction, String> {
        self.change(Hole {
            style: HoleStyle::Stepped(steps),
            standard: None,
            ..self.hole.clone()
        })
    }

    fn next_step(&self, steps: &[HoleStep]) -> HoleStep {
        let unit = self.model.length_unit();
        let last = steps.last();
        let diameter = last
            .and_then(|last| self.millimetres_of(&last.diameter))
            .zip(self.millimetres_of(&self.hole.diameter))
            .map_or_else(
                || unit.default_length(DEFAULT_STEP_DIAMETER),
                |(wide, narrow)| hole_tools::millimetres(((wide + narrow) * 5.0).round() / 10.0),
            );
        let depth = last.map_or_else(
            || unit.default_length(hole_tools::DEFAULT_STEP_DEPTH),
            |last| last.depth.clone(),
        );
        HoleStep { diameter, depth }
    }

    fn step_rows(&mut self, ui: &mut Ui, steps: &[HoleStep]) {
        for (index, step) in steps.iter().enumerate() {
            let number = index + 1;
            let rows = [
                (true, format!("Step {number} diameter"), &step.diameter),
                (false, format!("Step {number} depth"), &step.depth),
            ];
            for (diameter, caption, shown) in rows {
                let quantity = Quantity {
                    feature: self.id(),
                    id: Id::new(("hole-step", index, diameter, self.id())),
                    expression: shown,
                    dimension: Dimension::LENGTH,
                    rule: Rule::AboveZero,
                };
                let drafting = feature_fields::expression_row_drafting(
                    ui,
                    self.model,
                    &caption,
                    quantity,
                    |value| {
                        let mut changed = steps.to_vec();
                        if let Some(step) = changed.get_mut(index) {
                            if diameter {
                                step.diameter = value;
                            } else {
                                step.depth = value;
                            }
                        }
                        self.with_steps(changed)
                    },
                );
                self.actions.extend(drafting.into_actions(self.id()));
            }
        }
        ui.label("");
        let mut change = None;
        ui.horizontal_wrapped(|ui| {
            let add = widgets::small_button(ui, icons::ADD, ADD_STEP);
            if ui
                .add_enabled(steps.len() < MAX_HOLE_STEPS, add)
                .on_disabled_hover_text(format!("A hole has at most {MAX_HOLE_STEPS} steps"))
                .clicked()
            {
                let mut more = steps.to_vec();
                more.push(self.next_step(steps));
                change = Some(self.with_steps(more));
            }
            let remove = widgets::small_button(ui, icons::SUBTRACT, REMOVE_STEP);
            let fewer = steps
                .split_last()
                .map(|(_, above)| above.to_vec())
                .filter(|above| !above.is_empty());
            if ui
                .add_enabled(fewer.is_some(), remove)
                .on_disabled_hover_text("A stepped hole keeps at least one step")
                .clicked()
                && let Some(fewer) = fewer
            {
                change = Some(self.with_steps(fewer));
            }
        });
        ui.end_row();
        if let Some(change) = change {
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }

    fn style_rows(&mut self, ui: &mut Ui) {
        match self.hole.style.clone() {
            HoleStyle::Plain => {}
            HoleStyle::Stepped(steps) => self.step_rows(ui, &steps),
            HoleStyle::Counterbore { diameter, depth } => {
                self.length_row(
                    ui,
                    Field {
                        caption: "Counterbore diameter",
                        key: "counterbore-diameter",
                        dimension: Dimension::LENGTH,
                        rule: Rule::AboveZero,
                    },
                    &diameter,
                    |hole, value| Hole {
                        style: HoleStyle::Counterbore {
                            diameter: value,
                            depth: depth.clone(),
                        },
                        standard: None,
                        ..hole.clone()
                    },
                );
                let wide = diameter.clone();
                self.length_row(
                    ui,
                    Field {
                        caption: "Counterbore depth",
                        key: "counterbore-depth",
                        dimension: Dimension::LENGTH,
                        rule: Rule::AboveZero,
                    },
                    &depth.clone(),
                    |hole, value| Hole {
                        style: HoleStyle::Counterbore {
                            diameter: wide.clone(),
                            depth: value,
                        },
                        standard: None,
                        ..hole.clone()
                    },
                );
            }
            HoleStyle::Countersink { diameter, angle } => {
                self.length_row(
                    ui,
                    Field {
                        caption: "Countersink diameter",
                        key: "countersink-diameter",
                        dimension: Dimension::LENGTH,
                        rule: Rule::AboveZero,
                    },
                    &diameter,
                    |hole, value| Hole {
                        style: HoleStyle::Countersink {
                            diameter: value,
                            angle: angle.clone(),
                        },
                        standard: None,
                        ..hole.clone()
                    },
                );
                let wide = diameter.clone();
                self.length_row(
                    ui,
                    Field {
                        caption: "Countersink angle",
                        key: "countersink-angle",
                        dimension: Dimension::ANGLE,
                        rule: Rule::ConeAngle,
                    },
                    &angle.clone(),
                    |hole, value| Hole {
                        style: HoleStyle::Countersink {
                            diameter: wide.clone(),
                            angle: value,
                        },
                        standard: None,
                        ..hole.clone()
                    },
                );
            }
        }
    }
}

pub fn target_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    hole: &Hole,
) -> Result<Transaction, String> {
    let target = solid_panel::selected_target(model, selection, feature)?;
    let depth = HoleDepth::up_to_face(target).with_offset(hole.depth.offset().cloned());
    if depth == hole.depth {
        return Err("The hole already ends at the selected face or plane".to_owned());
    }
    let transaction = hole_tools::edit(
        model.document(),
        feature,
        Hole {
            depth,
            ..hole.clone()
        },
    )
    .ok_or_else(|| "The feature no longer exists".to_owned())?;
    field::checked(model.document(), transaction)
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    hole: &Hole,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        hole,
        actions,
    };
    widgets::properties(ui, ("hole-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.placement_rows(ui);
        panel.size_row(ui);
        if let Some(standard) = hole.standard {
            panel.fit_row(ui, standard);
        }
        panel.sizing_row(ui);
        panel.style_row(ui);
        panel.length_row(
            ui,
            Field {
                caption: "Diameter",
                key: "diameter",
                dimension: Dimension::LENGTH,
                rule: Rule::AboveZero,
            },
            &hole.diameter,
            |hole, value| Hole {
                diameter: value,
                standard: None,
                ..hole.clone()
            },
        );
        panel.style_rows(ui);
        panel.shape_row(ui);
        panel.depth_row(ui);
        if let HoleDepth::Blind(depth) = &hole.depth {
            panel.length_row(
                ui,
                Field {
                    caption: "Depth",
                    key: "depth",
                    dimension: Dimension::LENGTH,
                    rule: Rule::AboveZero,
                },
                depth,
                |hole, value| Hole {
                    depth: HoleDepth::Blind(value),
                    ..hole.clone()
                },
            );
        }
        panel.end_rows(ui);
        panel.bottom_rows(ui);
        if let Some(reversed) = feature_fields::reverse_row(ui, REVERSE_DIRECTION, hole.reversed) {
            let flipped = Hole {
                reversed,
                ..hole.clone()
            };
            panel.actions.push(feature_fields::applied(
                &feature.name,
                panel.change(flipped),
            ));
        }
        panel.sketch_row(ui);
        panel.body_row(ui);
    });
}
