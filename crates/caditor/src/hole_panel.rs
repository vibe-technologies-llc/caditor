use caditor_document::{Feature, FeatureId, Hole, HoleDepth, HoleStyle, Transaction};
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

    fn style_row(&mut self, ui: &mut Ui) {
        let current = Kind::of(&self.hole.style);
        let unit = self.model.length_unit();
        let segments = Kind::ALL
            .into_iter()
            .map(|kind| Segment {
                label: kind.label(),
                hover: kind.description(),
                change: (kind != current).then(|| {
                    self.change(Hole {
                        style: kind.default_style(unit),
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
                        rule: Rule::Any,
                    },
                    &angle.clone(),
                    |hole, value| Hole {
                        style: HoleStyle::Countersink {
                            diameter: wide.clone(),
                            angle: value,
                        },
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
                ..hole.clone()
            },
        );
        panel.style_rows(ui);
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
