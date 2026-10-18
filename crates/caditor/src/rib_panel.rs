use caditor_document::{
    Feature, FeatureId, Rib, RibDirection, RibEnds, RibSide, Transaction, rib_direction_name,
    rib_ends_name, rib_side_name,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Choice, MISSING_BODY, MISSING_SKETCH, Quantity, Rule, Segment},
    model::{Action, Model},
    rib_tools::{self, RibError},
    widgets,
};

pub const SKETCH: &str = "Sketch";
pub const BODY: &str = "Body";
pub const DIRECTION: &str = "Direction";
pub const THICKNESS: &str = "Thickness";
pub const DRAFT: &str = "Draft";
pub const ENDS: &str = "Free ends";
pub const SIDE: &str = "Side";
pub const REVERSE_FILL: &str = "Fill the other side of the curves";
pub const REVERSE_DIRECTION: &str = "Run the other way from the sketch";
const PARALLEL_HOVER: &str = "A rib: the material lies in the sketch's plane, on one side of the \
                              curves, and runs on until it meets the body; the thickness is \
                              across the sketch";
const SQUARE_HOVER: &str = "A web: the curves are thickened in the sketch's plane and the wall \
                            runs square to the sketch until it meets the body";

fn direction_hover(direction: RibDirection) -> &'static str {
    match direction {
        RibDirection::Parallel => PARALLEL_HOVER,
        RibDirection::Square => SQUARE_HOVER,
    }
}

fn side_hover(direction: RibDirection, side: RibSide) -> &'static str {
    match (direction, side) {
        (RibDirection::Parallel, RibSide::OneSide) => {
            "All the thickness in front of the sketch, the way its normal points"
        }
        (RibDirection::Parallel, RibSide::OtherSide) => "All the thickness behind the sketch",
        (RibDirection::Square, RibSide::OneSide) => {
            "All the thickness to the left of the curves, walked from their first end"
        }
        (RibDirection::Square, RibSide::OtherSide) => {
            "All the thickness to the right of the curves, walked from their first end"
        }
        (_, RibSide::Symmetric) => "Half the thickness on each side of the curves",
    }
}

fn ends_hover(ends: RibEnds) -> &'static str {
    match ends {
        RibEnds::Straight => {
            "Free ends run on straight along their last direction until they meet the body"
        }
        RibEnds::RoundArcs => {
            "A free end of an arc runs on round the arc's circle until it meets the body; a line \
             still runs on straight"
        }
    }
}

const END_KINDS: [RibEnds; 2] = [RibEnds::Straight, RibEnds::RoundArcs];
const DIRECTIONS: [RibDirection; 2] = [RibDirection::Parallel, RibDirection::Square];
const SIDES: [RibSide; 3] = [RibSide::OneSide, RibSide::OtherSide, RibSide::Symmetric];

struct Panel<'a> {
    model: &'a Model,
    feature: &'a Feature,
    rib: &'a Rib,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, rib: Rib) -> Result<Transaction, RibError> {
        rib_tools::change(self.model, self.id(), rib)
    }

    fn sketch_row(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let before = document.feature_index(self.id()).unwrap_or(0);
        let current = self.rib.sketch;
        widgets::caption(ui, SKETCH);
        let name = document.feature(current).map(|sketch| sketch.name.as_str());
        let chosen = ui
            .horizontal(|ui| {
                let text = feature_fields::combo_text(ui, name, MISSING_SKETCH);
                feature_fields::combo(ui, Id::unique(("rib-sketch", self.id())), text, || {
                    document
                        .features()
                        .take(before)
                        .filter(|feature| feature.kind.sketch().is_some())
                        .map(|sketch| {
                            let change =
                                if rib_tools::qualifies(self.model.evaluation(), sketch.id()) {
                                    self.change(Rib {
                                        sketch: sketch.id(),
                                        ..self.rib.clone()
                                    })
                                } else {
                                    Err(RibError::PathClosed)
                                };
                            Choice {
                                label: sketch.name.clone(),
                                selected: sketch.id() == current,
                                change: change.map(Action::Apply),
                            }
                        })
                        .collect()
                })
            })
            .inner;
        ui.end_row();
        self.actions.extend(chosen);
    }

    fn body_row(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let bodies = document.bodies_before_in(self.id(), self.model.evaluation());
        let current = self.rib.body;
        widgets::caption(ui, BODY);
        let name = document.body_label(current);
        let chosen = ui
            .horizontal(|ui| {
                let text = feature_fields::combo_text(ui, name.as_deref(), MISSING_BODY);
                feature_fields::combo(ui, Id::unique(("rib-body", self.id())), text, || {
                    bodies
                        .iter()
                        .filter_map(|body| {
                            Some(Choice {
                                label: document.body_label(*body)?,
                                selected: *body == current,
                                change: self
                                    .change(Rib {
                                        body: *body,
                                        ..self.rib.clone()
                                    })
                                    .map(Action::Apply),
                            })
                        })
                        .collect()
                })
            })
            .inner;
        ui.end_row();
        self.actions.extend(chosen);
    }

    fn direction_row(&mut self, ui: &mut Ui) {
        let current = self.rib.direction;
        let segments = DIRECTIONS
            .into_iter()
            .map(|direction| Segment {
                label: rib_direction_name(direction),
                hover: direction_hover(direction),
                change: (direction != current).then(|| {
                    self.change(rib_tools::with_direction(
                        self.model,
                        self.id(),
                        self.rib,
                        direction,
                    ))
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, DIRECTION, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn thickness_row(&mut self, ui: &mut Ui) {
        let rib = self.rib.clone();
        let quantity = Quantity {
            feature: self.id(),
            id: Id::unique(("rib-field", self.id(), "thickness")),
            expression: &self.rib.thickness,
            dimension: Dimension::LENGTH,
            rule: Rule::AboveZero,
        };
        let changed = move |thickness: Expression| Rib {
            thickness,
            ..rib.clone()
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, THICKNESS, quantity, |value| {
                rib_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn draft_row(&mut self, ui: &mut Ui) {
        let rib = self.rib.clone();
        let quantity = Quantity {
            feature: self.id(),
            id: Id::unique(("rib-field", self.id(), "draft")),
            expression: &self.rib.draft,
            dimension: Dimension::ANGLE,
            rule: Rule::RibDraft,
        };
        let changed = move |draft: Expression| Rib {
            draft,
            ..rib.clone()
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, DRAFT, quantity, |value| {
                rib_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn ends_row(&mut self, ui: &mut Ui) {
        let current = self.rib.ends;
        let segments = END_KINDS
            .into_iter()
            .map(|ends| Segment {
                label: rib_ends_name(ends),
                hover: ends_hover(ends),
                change: (ends != current)
                    .then(|| self.change(rib_tools::with_ends(self.rib, ends))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, ENDS, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn side_row(&mut self, ui: &mut Ui) {
        let current = self.rib.side;
        let direction = self.rib.direction;
        let segments = SIDES
            .into_iter()
            .map(|side| Segment {
                label: rib_side_name(side),
                hover: side_hover(direction, side),
                change: (side != current)
                    .then(|| self.change(rib_tools::with_side(self.rib, side))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SIDE, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn reverse_row(&mut self, ui: &mut Ui) {
        let label = match self.rib.direction {
            RibDirection::Parallel => REVERSE_FILL,
            RibDirection::Square => REVERSE_DIRECTION,
        };
        if feature_fields::reverse_row(ui, label, self.rib.reversed).is_some() {
            let change = self.change(rib_tools::reversed(self.rib));
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }
}

pub fn show(ui: &mut Ui, model: &Model, actions: &mut Vec<Action>, feature: &Feature, rib: &Rib) {
    let mut panel = Panel {
        model,
        feature,
        rib,
        actions,
    };
    widgets::properties(ui, ("rib-properties", feature.id()), |ui| {
        panel.sketch_row(ui);
        panel.body_row(ui);
        panel.direction_row(ui);
        panel.thickness_row(ui);
        panel.side_row(ui);
        panel.draft_row(ui);
        if rib.direction == RibDirection::Square {
            panel.ends_row(ui);
        }
        panel.reverse_row(ui);
    });
}
