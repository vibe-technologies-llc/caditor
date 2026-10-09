use caditor_document::{
    BodyOperation, Feature, FeatureId, Primitive, PrimitiveAnchor, PrimitiveKind, SizeRule,
    capitalized, describe_plane,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Label, Ui};

use crate::{
    feature_fields::{self, Choice, MISSING_BODY, Picker, Quantity, Rule, Segment},
    model::{Action, Model},
    primitive_tools,
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

const PICK_HOVER: &str = "Place it on the selected plane or flat face instead";
const POSITION_CAPTIONS: [&str; 2] = ["Position X", "Position Y"];

fn shape_hover(kind: PrimitiveKind) -> &'static str {
    match kind {
        PrimitiveKind::Box => "A block of a length, width and height",
        PrimitiveKind::Cylinder => "A round post of a diameter and height",
        PrimitiveKind::Sphere => "A ball of a diameter",
        PrimitiveKind::Torus => "A ring of a diameter and a tube diameter",
        PrimitiveKind::Cone => {
            "A cone or frustum of two diameters, either of them zero, and a height"
        }
        PrimitiveKind::Wedge => "A block whose top is shorter, sloping down to the right",
        PrimitiveKind::Prism => {
            "A straight prism of a number of sides, a diameter across its corners and a height"
        }
    }
}

fn anchor_hover(anchor: PrimitiveAnchor) -> &'static str {
    match anchor {
        PrimitiveAnchor::Corner => "The position is a corner of the shape's footprint",
        PrimitiveAnchor::BaseCentre => "The position is the middle of the shape's base",
        PrimitiveAnchor::Centre => "The position is the middle of the shape, half above the plane",
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

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    primitive: &'a Primitive,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, primitive: Primitive) -> Result<Action, String> {
        primitive_tools::change(self.model, self.id(), primitive).map(Action::Apply)
    }

    fn apply(&mut self, primitive: Primitive) {
        let change = primitive_tools::change(self.model, self.id(), primitive);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn shape_row(&mut self, ui: &mut Ui) {
        let current = self.primitive.shape.kind();
        let unit = self.model.length_unit();
        let segments = PrimitiveKind::ALL
            .into_iter()
            .map(|kind| Segment {
                label: kind.name(),
                hover: shape_hover(kind),
                change: (kind != current).then(|| {
                    primitive_tools::change(
                        self.model,
                        self.id(),
                        primitive_tools::with_shape(self.primitive, kind, unit),
                    )
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Shape", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn plane_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        widgets::caption(ui, "Placed on");
        let picker = Picker {
            feature: self.id(),
            slot: Slot::PrimitivePlace,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::PrimitivePlace),
                || {
                    primitive_tools::place_change(
                        self.model,
                        self.selection,
                        self.id(),
                        self.primitive,
                    )
                },
            ),
            hover: PICK_HOVER,
        };
        let text = capitalized(&describe_plane(document, &self.primitive.plane));
        ui.vertical(|ui| {
            ui.add(Label::new(text).wrap());
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
    }

    fn value_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        key: (&'static str, usize),
        (dimension, rule): (Dimension, Rule),
        changed: impl Fn(Expression) -> Primitive,
    ) {
        let expression = match key {
            ("at", index) => self.primitive.at.get(index),
            (_, index) => self
                .primitive
                .shape
                .sizes()
                .into_iter()
                .nth(index)
                .map(|(_, size)| size),
        };
        let Some(expression) = expression else {
            return;
        };
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("primitive-field", self.id(), key)),
            expression,
            dimension,
            rule,
        };
        let committed =
            feature_fields::expression_row(ui, self.model, caption, quantity, |value| {
                primitive_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(committed.map(Action::Apply));
    }

    fn position_rows(&mut self, ui: &mut Ui) {
        for (index, caption) in POSITION_CAPTIONS.into_iter().enumerate() {
            let primitive = self.primitive.clone();
            let any = (Dimension::LENGTH, Rule::Any);
            self.value_row(ui, caption, ("at", index), any, move |value| {
                let mut changed = primitive.clone();
                if let Some(slot) = changed.at.get_mut(index) {
                    *slot = value;
                }
                changed
            });
        }
    }

    fn anchor_row(&mut self, ui: &mut Ui) {
        let current = self.primitive.anchor;
        let segments = PrimitiveAnchor::ALL
            .into_iter()
            .map(|anchor| Segment {
                label: anchor.name(),
                hover: anchor_hover(anchor),
                change: (anchor != current).then(|| {
                    primitive_tools::change(
                        self.model,
                        self.id(),
                        Primitive {
                            anchor,
                            ..self.primitive.clone()
                        },
                    )
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Starts at", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn size_rows(&mut self, ui: &mut Ui) {
        let captions: Vec<String> = self
            .primitive
            .shape
            .sizes()
            .into_iter()
            .map(|(what, _)| capitalized(what))
            .collect();
        let rules = self.primitive.shape.rules();
        for (index, (caption, rule)) in captions.iter().zip(rules).enumerate() {
            let primitive = self.primitive.clone();
            let checked = match rule {
                SizeRule::AboveZero => (Dimension::LENGTH, Rule::AboveZero),
                SizeRule::ZeroOrMore => (Dimension::LENGTH, Rule::ZeroOrMore),
                SizeRule::Sides => (Dimension::NONE, Rule::Sides),
            };
            self.value_row(ui, caption, ("size", index), checked, move |value| {
                let mut changed = primitive.clone();
                if let Some(slot) = changed.shape.sizes_mut().into_iter().nth(index) {
                    *slot = value;
                }
                changed
            });
        }
    }

    fn reverse_row(&mut self, ui: &mut Ui) {
        if self.primitive.anchor == PrimitiveAnchor::Centre {
            return;
        }
        if let Some(reversed) = feature_fields::reverse_row(
            ui,
            feature_fields::REVERSE_DIRECTION,
            self.primitive.reversed,
        ) {
            self.apply(Primitive {
                reversed,
                ..self.primitive.clone()
            });
        }
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let operation = self.primitive.operation;
        let bodies = self.model.document().bodies_before(self.id());
        let target = operation.target().or(bodies.last().copied());
        widgets::caption(ui, "Result");
        let candidates = [
            Some(BodyOperation::NewBody),
            target.map(BodyOperation::Add),
            target.map(BodyOperation::Remove),
            target.map(BodyOperation::Intersect),
        ];
        let chosen = feature_fields::combo(
            ui,
            Id::new(("primitive-operation", self.id())),
            operation_name(operation),
            || {
                candidates
                    .into_iter()
                    .flatten()
                    .map(|candidate| Choice {
                        label: operation_name(candidate).to_owned(),
                        selected: operation_name(candidate) == operation_name(operation),
                        change: self
                            .change(primitive_tools::with_operation(self.primitive, candidate)),
                    })
                    .collect()
            },
        );
        ui.end_row();
        self.actions.extend(chosen);
        let Some(target) = operation.target() else {
            return;
        };
        widgets::caption(ui, "Body");
        let document = self.model.document();
        let name = feature_fields::feature_name(document, target);
        let chosen = ui
            .horizontal(|ui| {
                let text = feature_fields::combo_text(ui, name, MISSING_BODY);
                feature_fields::combo(ui, Id::new(("primitive-body", self.id())), text, || {
                    bodies
                        .iter()
                        .filter_map(|body| {
                            Some(Choice {
                                label: document.feature(*body)?.name.clone(),
                                selected: *body == target,
                                change: self.change(primitive_tools::with_operation(
                                    self.primitive,
                                    operation.with_target(*body),
                                )),
                            })
                        })
                        .collect()
                })
            })
            .inner;
        ui.end_row();
        self.actions.extend(chosen);
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    primitive: &Primitive,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        primitive,
        actions,
    };
    widgets::properties(ui, ("primitive-properties", feature.id()), |ui| {
        panel.shape_row(ui);
        panel.plane_rows(ui);
        panel.position_rows(ui);
        panel.anchor_row(ui);
        panel.size_rows(ui);
        panel.reverse_row(ui);
        panel.operation_rows(ui);
    });
}
