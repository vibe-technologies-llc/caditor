use caditor_document::{
    Feature, FeatureId, Hole, HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard, HoleStyle,
    MetricSize, Transaction, circle_sizes,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Choice, Quantity, REVERSE_DIRECTION, Rule, Segment},
    field,
    hole_tools::{self, Kind},
    model::{Action, Model},
    widgets,
};

pub const DESCRIPTION: &str = "Drills a hole at every point of the sketch";
const NOT_A_BODY: &str = "A body that is no longer there";
pub const CUSTOM_SIZE: &str = "Custom";
pub const SIZED_BY_CIRCLES: &str = "Circles";
const SIZED_BY_CIRCLES_NOTE: &str = "Holes at circles take each circle's diameter; holes at \
                                     points take the diameter below";

fn sizing_label(sizing: HoleSizing) -> &'static str {
    match sizing {
        HoleSizing::Typed => "Diameter",
        HoleSizing::Circles => SIZED_BY_CIRCLES,
    }
}

fn sizing_hover(sizing: HoleSizing) -> &'static str {
    match sizing {
        HoleSizing::Typed => "Every hole takes the typed diameter, whatever the circle drawn",
        HoleSizing::Circles => "Each hole at a circle takes that circle's diameter",
    }
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
                let standard = HoleStandard { size, fit };
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
        let segments = HoleFit::ALL
            .into_iter()
            .map(|fit| Segment {
                label: fit.label(),
                hover: fit.description(),
                change: (fit != standard.fit).then(|| {
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
        let segments = [HoleSizing::Typed, HoleSizing::Circles]
            .into_iter()
            .map(|sizing| Segment {
                label: sizing_label(sizing),
                hover: sizing_hover(sizing),
                change: (sizing != self.hole.sizing).then(|| {
                    self.change(Hole {
                        sizing,
                        ..self.hole.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Sized by", &self.feature.name, segments);
        self.actions.extend(chosen);
        if self.hole.sizing == HoleSizing::Circles {
            feature_fields::description_row(ui, SIZED_BY_CIRCLES_NOTE);
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

    fn length_row(
        &mut self,
        ui: &mut Ui,
        field: Field,
        shown: &Expression,
        rebuild: impl Fn(&Hole, Expression) -> Hole,
    ) {
        let quantity = Quantity {
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

    fn style_rows(&mut self, ui: &mut Ui) {
        match self.hole.style.clone() {
            HoleStyle::Plain => {}
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
                        rule: Rule::Countersink,
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
