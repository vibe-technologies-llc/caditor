use caditor_document::{
    Feature, FeatureId, Primitive, PrimitiveAnchor, PrimitiveKind, SizeRule, capitalized,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    feature_fields::{self, OperationRows, Picker, Quantity, Rule, Segment},
    model::{Action, Model},
    plane_selector, primitive_tools,
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

const PICK_HOVER: &str = "Place it on the selected plane or flat face instead";

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
        let row = plane_selector::Row {
            caption: "Placed on",
            id: Id::new(("primitive-plane", self.id())),
            planes: datum_tools::listed_planes(self.model.document(), self.id()),
            current: Some(&self.primitive.plane),
            picker,
            remove: None,
        };
        let mut actions = Vec::new();
        plane_selector::row(
            ui,
            self.model,
            row,
            |plane| primitive_tools::placed_on(self.model, self.id(), self.primitive, plane),
            &mut actions,
        );
        self.actions.extend(actions);
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
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |value| {
                primitive_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn position_rows(&mut self, ui: &mut Ui) {
        for (index, caption) in feature_fields::POSITION_CAPTIONS.into_iter().enumerate() {
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
        let rows = OperationRows {
            feature: self.id(),
            operation: self.primitive.operation,
            ids: ("primitive-operation", "primitive-body"),
        };
        let chosen = feature_fields::body_operation_rows(ui, self.model, &rows, |operation| {
            primitive_tools::change(
                self.model,
                self.id(),
                primitive_tools::with_operation(self.primitive, operation),
            )
        });
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
