use caditor_document::{
    AxisReference, AxisTurn, Feature, FeatureId, Move, MoveAxis, Transaction, TurnCentre,
    capitalized, describe_axis,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    datum_tools,
    feature_fields::{self, Choice, Picker, Quantity, Rule, Segment},
    model::{Action, Model},
    move_tools, pattern_tools,
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "Turns the body about the axes through the origin, then shifts it";
pub const CENTRED_DESCRIPTION: &str =
    "Turns the body about the axes through the centre of its box, then shifts it";
pub const AXIS_DESCRIPTION: &str = "Turns the body about the chosen axis, then shifts it";
pub const COPY_DESCRIPTION: &str = "Makes a new body from a copy of the body, turned about the axes through the origin, then \
     shifted; the original stays where it is";
pub const CENTRED_COPY_DESCRIPTION: &str = "Makes a new body from a copy of the body, turned about the axes through the centre of its \
     box, then shifted; the original stays where it is";
pub const AXIS_COPY_DESCRIPTION: &str = "Makes a new body from a copy of the body, turned about the chosen axis, then shifted; the \
     original stays where it is";
pub const TURN_ABOUT: &str = "Turn about";
pub const ABOUT_CENTRE: &str = "Centre";
pub const ABOUT_ORIGIN: &str = "Origin";
pub const ABOUT_AXIS: &str = "Axis";
pub const AXIS: &str = "Axis";
pub const ANGLE: &str = "Angle";
pub const MAKE_A_COPY: &str = "Make a copy";
pub const DIRECTIONS: &str = "Directions";
pub const WORLD: &str = "World";
pub const FRAME_DESCRIPTION: &str = "Turns about and shifts along the X, Y and Z axes of the \
                                     coordinate system, and about its origin when turning about \
                                     the origin";
const PICK_HOVER: &str = "Turn about the selected axis, straight edge, round face or sketch line";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Centre {
    Body,
    Origin,
    Axis,
}

impl Centre {
    const ALL: [Self; 3] = [Self::Body, Self::Origin, Self::Axis];

    fn of(about: &TurnCentre) -> Self {
        match about {
            TurnCentre::Body => Self::Body,
            TurnCentre::Origin => Self::Origin,
            TurnCentre::Axis(_) => Self::Axis,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Body => ABOUT_CENTRE,
            Self::Origin => ABOUT_ORIGIN,
            Self::Axis => ABOUT_AXIS,
        }
    }

    fn hover(self) -> &'static str {
        match self {
            Self::Body => {
                "Turn about axes through the centre of the body's box, so it turns in place"
            }
            Self::Origin => "Turn about the X, Y and Z axes through the origin",
            Self::Axis => {
                "Turn by one angle about an axis, straight edge, round face or sketch line"
            }
        }
    }

    fn description(self, copy: bool) -> &'static str {
        match (copy, self) {
            (false, Self::Origin) => DESCRIPTION,
            (false, Self::Body) => CENTRED_DESCRIPTION,
            (false, Self::Axis) => AXIS_DESCRIPTION,
            (true, Self::Origin) => COPY_DESCRIPTION,
            (true, Self::Body) => CENTRED_COPY_DESCRIPTION,
            (true, Self::Axis) => AXIS_COPY_DESCRIPTION,
        }
    }
}

fn is_zero(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Number(value) | Expression::Measure(value, _) if *value == 0.0
    )
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    movement: &'a Move,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, movement: Move) -> Result<Transaction, String> {
        move_tools::checked_change(self.model, self.id(), movement)
    }

    fn centre_row(&mut self, ui: &mut Ui) {
        let current = Centre::of(&self.movement.about);
        let segments = Centre::ALL
            .into_iter()
            .map(|centre| Segment {
                label: centre.label(),
                hover: centre.hover(),
                change: (centre != current).then(|| {
                    let about = match centre {
                        Centre::Body => Move {
                            about: TurnCentre::Body,
                            ..self.movement.clone()
                        },
                        Centre::Origin => Move {
                            about: TurnCentre::Origin,
                            ..self.movement.clone()
                        },
                        Centre::Axis => move_tools::about_axis(
                            self.movement,
                            move_tools::first_axis(self.model, self.selection, self.id()),
                        ),
                    };
                    self.change(about)
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, TURN_ABOUT, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn frame_row(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let before = document.feature_index(self.id()).unwrap_or(usize::MAX);
        let frames = datum_tools::frames_before(document, before);
        if frames.is_empty() && self.movement.frame.is_none() {
            return;
        }
        let name = |frame: FeatureId| {
            document.feature(frame).map_or_else(
                || "A deleted coordinate system".to_owned(),
                |feature| feature.name.clone(),
            )
        };
        let current = self.movement.frame.map_or_else(|| WORLD.to_owned(), name);
        widgets::caption(ui, DIRECTIONS);
        let chosen = feature_fields::combo(ui, Id::new(("move-frame", self.id())), current, || {
            std::iter::once(None)
                .chain(frames.into_iter().map(Some))
                .map(|frame| Choice {
                    label: frame.map_or_else(|| WORLD.to_owned(), name),
                    selected: frame == self.movement.frame,
                    change: self
                        .change(Move {
                            frame,
                            ..self.movement.clone()
                        })
                        .map(Action::Apply),
                })
                .collect()
        });
        ui.end_row();
        self.actions.extend(chosen);
        if self.movement.frame.is_some() {
            feature_fields::description_row(ui, FRAME_DESCRIPTION);
        }
    }

    fn axis_rows(&mut self, ui: &mut Ui, turn: &AxisTurn) {
        let document = self.model.document();
        let current = capitalized(&describe_axis(document, &turn.axis));
        widgets::caption(ui, AXIS);
        let chosen = feature_fields::combo(ui, Id::new(("move-axis", self.id())), current, || {
            let listed = pattern_tools::listed_axes(self.model, self.id());
            let shown_too = (!listed.contains(&turn.axis)).then(|| turn.axis.clone());
            listed
                .into_iter()
                .chain(shown_too)
                .map(|axis: AxisReference| Choice {
                    label: capitalized(&describe_axis(document, &axis)),
                    selected: axis == turn.axis,
                    change: self
                        .change(move_tools::about_axis(self.movement, axis))
                        .map(Action::Apply),
                })
                .collect()
        });
        ui.end_row();
        self.actions.extend(chosen);
        ui.label("");
        let picker = Picker {
            feature: self.id(),
            slot: Slot::MoveAxis,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (self.id(), Slot::MoveAxis),
                || move_tools::axis_change(self.model, self.selection, self.id(), self.movement),
            ),
            hover: PICK_HOVER,
        };
        ui.vertical(|ui| {
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
        self.angle_row(ui, turn);
    }

    fn angle_row(&mut self, ui: &mut Ui, turn: &AxisTurn) {
        let id = self.id();
        let movement = self.movement;
        let quantity = Quantity {
            feature: id,
            id: Id::new(("move-field", "angle", 0, id)),
            expression: &turn.angle,
            dimension: Dimension::ANGLE,
            rule: Rule::Any,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, ANGLE, quantity, |angle| {
                self.change(Move {
                    about: TurnCentre::Axis(Box::new(AxisTurn {
                        axis: turn.axis.clone(),
                        angle,
                    })),
                    ..movement.clone()
                })
            });
        self.actions.extend(drafting.into_actions(id));
    }

    fn distance_row(&mut self, ui: &mut Ui, axis: MoveAxis) {
        let id = self.id();
        let movement = self.movement;
        let quantity = Quantity {
            feature: id,
            id: Id::new(("move-field", "offset", axis.index(), id)),
            expression: axis.of(&movement.offset),
            dimension: Dimension::LENGTH,
            rule: Rule::Any,
        };
        let drafting = feature_fields::expression_row_drafting(
            ui,
            self.model,
            &format!("Move along {}", axis.name()),
            quantity,
            |value| {
                let mut changed = movement.clone();
                *axis.of_mut(&mut changed.offset) = value;
                self.change(changed)
            },
        );
        self.actions.extend(drafting.into_actions(id));
    }

    fn turn_row(&mut self, ui: &mut Ui, axis: MoveAxis) {
        let id = self.id();
        let movement = self.movement;
        let quantity = Quantity {
            feature: id,
            id: Id::new(("move-field", "turn", axis.index(), id)),
            expression: axis.of(&movement.turn),
            dimension: Dimension::ANGLE,
            rule: Rule::Any,
        };
        let drafting = feature_fields::expression_row_drafting(
            ui,
            self.model,
            &format!("Turn about {}", axis.name()),
            quantity,
            |value| {
                let mut changed = movement.clone();
                *axis.of_mut(&mut changed.turn) = value;
                self.change(changed)
            },
        );
        self.actions.extend(drafting.into_actions(id));
    }

    fn copy_row(&mut self, ui: &mut Ui) {
        if let Some(copy) = feature_fields::reverse_row(ui, MAKE_A_COPY, self.movement.copy) {
            let change = self.change(Move {
                copy,
                ..self.movement.clone()
            });
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    movement: &Move,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        movement,
        actions,
    };
    widgets::properties(ui, ("move-properties", feature.id()), |ui| {
        let centre = Centre::of(&movement.about);
        feature_fields::description_row(ui, centre.description(movement.copy));
        panel.centre_row(ui);
        panel.frame_row(ui);
        if let Some(turn) = movement.about.axis_turn() {
            panel.axis_rows(ui, turn);
        }
        for axis in MoveAxis::ALL {
            if centre != Centre::Axis || !is_zero(axis.of(&movement.turn)) {
                panel.turn_row(ui, axis);
            }
        }
        for axis in MoveAxis::ALL {
            panel.distance_row(ui, axis);
        }
        feature_fields::feature_row(ui, model.document(), "Body", movement.body);
        panel.copy_row(ui);
    });
}
