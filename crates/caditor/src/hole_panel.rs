use caditor_document::{
    Feature, FeatureId, Hole, HoleBottom, HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard,
    HoleStep, HoleStyle, MAX_HOLE_STEPS, MetricSize, Transaction, circle_sizes, pitch_text,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Choice, Quantity, REVERSE_DIRECTION, Rule, Segment},
    field,
    hole_tools::{self, DEFAULT_STEP_DIAMETER, Kind},
    icons,
    model::{Action, Model},
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
                    "Thread {thread}: printed at the tap drill size, then cut with a tap or by a \
                     self-tapping screw"
                ),
            );
        }
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
        let blind = matches!(self.hole.depth, HoleDepth::Blind(_));
        let options = [
            (
                "Blind",
                "Stop at a depth",
                HoleDepth::Blind(hole_tools::default_depth(unit)),
                blind,
            ),
            (
                "Through all",
                "Drill all the way through the body",
                HoleDepth::ThroughAll,
                !blind,
            ),
        ];
        let segments = options
            .into_iter()
            .map(|(label, hover, depth, current)| Segment {
                label,
                hover,
                change: (!current).then(|| {
                    self.change(Hole {
                        depth,
                        ..self.hole.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Depth", &self.feature.name, segments);
        self.actions.extend(chosen);
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
        let committed =
            feature_fields::expression_row(ui, self.model, field.caption, quantity, |value| {
                self.change(rebuild(hole, value))
            });
        self.actions.extend(committed.map(Action::Apply));
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
                let committed =
                    feature_fields::expression_row(ui, self.model, &caption, quantity, |value| {
                        let mut changed = steps.to_vec();
                        if let Some(step) = changed.get_mut(index) {
                            if diameter {
                                step.diameter = value;
                            } else {
                                step.depth = value;
                            }
                        }
                        self.with_steps(changed)
                    });
                self.actions.extend(committed.map(Action::Apply));
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

pub fn show(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>, feature: &Feature, hole: &Hole) {
    let mut panel = Panel {
        model,
        feature,
        hole,
        actions,
    };
    widgets::properties(ui, ("hole-properties", feature.id()), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
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
        feature_fields::feature_row(ui, model.document(), "Sketch", hole.sketch);
        panel.body_row(ui);
    });
}
