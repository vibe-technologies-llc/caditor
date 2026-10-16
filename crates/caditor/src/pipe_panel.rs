use caditor_document::{
    Feature, FeatureId, Pipe, Transaction, capitalized, pipe_shape_name, pipe_size_name,
};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::PipeShape;
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, OperationRows, Picker, Quantity, Rule, Segment, Shown},
    model::{Action, Model},
    pipe_tools::{self, PipeError},
    reference_picking::Slot,
    selection::Selection,
    solid_panel::PATH,
    sweep_tools, widgets,
};

pub const SECTION: &str = "Section";
pub const WALL: &str = "Wall";
pub const SOLID: &str = "Solid";
pub const HOLLOW: &str = "Hollow";
pub const WALL_THICKNESS: &str = "Wall thickness";
pub const BEND_RADIUS: &str = "Bend radius";
const FOLLOW_SELECTED: &str = "Follow the selected curves or edges instead";
const CLEAR_PATH: &str = "Clear the path";
const SOLID_HOVER: &str = "Fill the whole section";
const HOLLOW_HOVER: &str = "Leave a wall of a thickness round an empty middle, as a tube";

fn shape_hover(shape: PipeShape) -> &'static str {
    match shape {
        PipeShape::Round => "A round section of a diameter",
        PipeShape::Square => "A square section of a width, its sides along the path's plane",
        PipeShape::Triangle => {
            "An equilateral triangle of a side length, one side along the path's plane"
        }
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    pipe: &'a Pipe,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, pipe: Pipe) -> Result<Transaction, PipeError> {
        pipe_tools::change(self.model, self.id(), pipe)
    }

    fn apply(&mut self, pipe: Pipe) {
        let change = self.change(pipe);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn path_row(&mut self, ui: &mut Ui) {
        let id = self.id();
        let path = &self.pipe.path;
        let shown = if path.is_empty() {
            Shown::NoneChosen
        } else {
            Shown::Named(sweep_tools::path_words(self.model.document(), path))
        };
        let picker = Picker {
            feature: id,
            slot: Slot::SweepPath,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::SweepPath),
                || pipe_tools::selected_change(self.model, self.selection, id, self.pipe),
            ),
            hover: FOLLOW_SELECTED,
        };
        let removed = feature_fields::reference_row(
            ui,
            self.model,
            PATH,
            shown,
            picker,
            (!path.is_empty()).then_some(CLEAR_PATH),
            self.actions,
        );
        if removed {
            self.apply(pipe_tools::cleared(self.pipe));
        }
    }

    fn shape_row(&mut self, ui: &mut Ui) {
        let current = self.pipe.shape;
        let segments = PipeShape::ALL
            .into_iter()
            .map(|shape| Segment {
                label: pipe_shape_name(shape),
                hover: shape_hover(shape),
                change: (shape != current)
                    .then(|| self.change(pipe_tools::with_shape(self.pipe, shape))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SECTION, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn value_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        (key, expression, rule): (&'static str, &Expression, Rule),
        changed: impl Fn(Expression) -> Pipe,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::unique(("pipe-field", self.id(), key)),
            expression,
            dimension: Dimension::LENGTH,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |value| {
                pipe_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn size_row(&mut self, ui: &mut Ui) {
        let caption = capitalized(pipe_size_name(self.pipe.shape));
        let pipe = self.pipe.clone();
        let field = ("size", &self.pipe.size, Rule::AboveZero);
        self.value_row(ui, &caption, field, move |size| Pipe {
            size,
            ..pipe.clone()
        });
    }

    fn wall_rows(&mut self, ui: &mut Ui) {
        let hollow = self.pipe.wall.is_some();
        let unit = self.model.length_unit();
        let with = |wanted: bool| {
            (wanted != hollow).then(|| self.change(pipe_tools::hollowed(self.pipe, wanted, unit)))
        };
        let segments = vec![
            Segment {
                label: SOLID,
                hover: SOLID_HOVER,
                change: with(false),
            },
            Segment {
                label: HOLLOW,
                hover: HOLLOW_HOVER,
                change: with(true),
            },
        ];
        let chosen = feature_fields::segmented_row(ui, WALL, &self.feature.name, segments);
        self.actions.extend(chosen);
        let Some(wall) = &self.pipe.wall else {
            return;
        };
        let pipe = self.pipe.clone();
        let field = ("wall", wall, Rule::AboveZero);
        self.value_row(ui, WALL_THICKNESS, field, move |wall| Pipe {
            wall: Some(wall),
            ..pipe.clone()
        });
    }

    fn bend_row(&mut self, ui: &mut Ui) {
        let pipe = self.pipe.clone();
        let field = ("bend", &self.pipe.bend, Rule::ZeroOrMore);
        self.value_row(ui, BEND_RADIUS, field, move |bend| Pipe {
            bend,
            ..pipe.clone()
        });
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let rows = OperationRows {
            feature: self.id(),
            operation: self.pipe.operation,
            ids: ("pipe-operation", "pipe-body"),
        };
        let chosen = feature_fields::body_operation_rows(ui, self.model, &rows, |operation| {
            self.change(pipe_tools::with_operation(self.pipe, operation))
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
    pipe: &Pipe,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        pipe,
        actions,
    };
    widgets::properties(ui, ("pipe-properties", feature.id()), |ui| {
        panel.path_row(ui);
        panel.shape_row(ui);
        panel.size_row(ui);
        panel.wall_rows(ui);
        panel.bend_row(ui);
        panel.operation_rows(ui);
    });
}
