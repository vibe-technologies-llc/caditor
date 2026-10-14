use caditor_document::{Feature, FeatureId, Loft, Transaction, describe_point, loft_skin_name};
use caditor_expression::{Dimension, Expression};
use caditor_kernel::LoftSkin;
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, OperationRows, Picker, Quantity, Rule, Segment, Shown},
    icons, loft_tools,
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const PROFILES: &str = "Profiles";
pub const ENDS_AT: &str = "Ends at";
pub const SIDES: &str = "Sides";
pub const LAST_PROFILE: &str = "Its last profile";
const THROUGH_SELECTED: &str = "Loft through the selected regions or sketches instead, in the \
                                order they were selected";
const END_AT_SELECTED: &str = "End at the selected point";
const END_AT_LAST: &str = "End at the last profile instead of a point";
const EARLIER: &str = "Move this profile earlier";
const LATER: &str = "Move this profile later";
const LEAVE_OUT: &str = "Leave this profile out";
const SMOOTH_HOVER: &str = "One smooth skin passing through every profile";
const RULED_HOVER: &str = "Straight-lined sides from each profile to the next";
const MISSING_PROFILE: &str = "Missing sketch";
pub const CLOSING: &str = "Closing";
pub const OPEN: &str = "Open";
pub const CLOSED: &str = "Closed";
const OPEN_HOVER: &str = "Stop at the last profile or end point";
const CLOSED_HOVER: &str =
    "Run on from the last profile back into the first, making a ring with no ends";
pub const START: &str = "Start";
pub const END: &str = "End";
pub const FREE: &str = "Free";
pub const SQUARE: &str = "Square";
const FREE_HOVER: &str = "Leave the profile however the profiles around it lead";
const SQUARE_HOVER: &str = "Leave the profile square to its plane, as a straight wall would";
pub const START_WEIGHT: &str = "Start weight";
pub const END_WEIGHT: &str = "End weight";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Start,
    End,
}

impl Side {
    fn caption(self) -> &'static str {
        match self {
            Self::Start => START,
            Self::End => END,
        }
    }

    fn weight_caption(self) -> &'static str {
        match self {
            Self::Start => START_WEIGHT,
            Self::End => END_WEIGHT,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }

    fn weight(self, loft: &Loft) -> Option<&Expression> {
        match self {
            Self::Start => loft.square_start.as_ref(),
            Self::End => loft.square_end.as_ref(),
        }
    }
}

fn skin_hover(skin: LoftSkin) -> &'static str {
    match skin {
        LoftSkin::Smooth => SMOOTH_HOVER,
        LoftSkin::Ruled => RULED_HOVER,
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    loft: &'a Loft,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, loft: Loft) -> Result<Transaction, String> {
        loft_tools::change(self.model, self.id(), loft)
    }

    fn apply(&mut self, loft: Loft) {
        let change = self.change(loft);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn profile_line(&mut self, ui: &mut Ui, index: usize) {
        let document = self.model.document();
        let Some(section) = self.loft.profiles.get(index) else {
            return;
        };
        let words = loft_tools::profile_words(document, section);
        let count = self.loft.profiles.len();
        let mut wanted = None;
        ui.horizontal_top(|ui| {
            let text = match &words {
                Some(words) => format!("{}. {words}", index + 1),
                None => format!("{}. {MISSING_PROFILE}", index + 1),
            };
            widgets::label_before_icon_buttons(ui, &text, 3);
            let earlier = ui
                .add_enabled_ui(index > 0, |ui| {
                    widgets::icon_button(ui, icons::MOVE_UP, EARLIER)
                })
                .inner;
            if earlier.clicked() {
                wanted = loft_tools::moved(self.loft, index, true);
            }
            let later = ui
                .add_enabled_ui(index + 1 < count, |ui| {
                    widgets::icon_button(ui, icons::MOVE_DOWN, LATER)
                })
                .inner;
            if later.clicked() {
                wanted = loft_tools::moved(self.loft, index, false);
            }
            if widgets::icon_button(ui, icons::REMOVE, LEAVE_OUT).clicked() {
                wanted = Some(loft_tools::without_profile(self.loft, index));
            }
        });
        if let Some(loft) = wanted {
            self.apply(loft);
        }
    }

    fn profiles_row(&mut self, ui: &mut Ui) {
        let id = self.id();
        widgets::caption(ui, PROFILES);
        ui.vertical(|ui| {
            if self.loft.profiles.is_empty() {
                feature_fields::none_chosen(ui);
            }
            for index in 0..self.loft.profiles.len() {
                self.profile_line(ui, index);
            }
            let picker = Picker {
                feature: id,
                slot: Slot::LoftProfiles,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    self.model,
                    self.selection,
                    (id, Slot::LoftProfiles),
                    || loft_tools::selected_change(self.model, self.selection, id, self.loft),
                ),
                hover: THROUGH_SELECTED,
            };
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
    }

    fn end_row(&mut self, ui: &mut Ui) {
        let id = self.id();
        let shown = match &self.loft.end {
            Some(end) => Shown::Named(describe_point(self.model.document(), end)),
            None => Shown::Named(LAST_PROFILE.to_owned()),
        };
        let picker = Picker {
            feature: id,
            slot: Slot::LoftEnd,
            selected: feature_fields::offered_change(
                ui.ctx(),
                self.model,
                self.selection,
                (id, Slot::LoftEnd),
                || loft_tools::end_change(self.model, self.selection, id, self.loft),
            ),
            hover: END_AT_SELECTED,
        };
        let removed = feature_fields::reference_row(
            ui,
            self.model,
            ENDS_AT,
            shown,
            picker,
            self.loft.end.is_some().then_some(END_AT_LAST),
            self.actions,
        );
        if removed {
            self.apply(loft_tools::without_end(self.loft));
        }
    }

    fn skin_row(&mut self, ui: &mut Ui) {
        let current = self.loft.skin;
        let segments = [LoftSkin::Smooth, LoftSkin::Ruled]
            .into_iter()
            .map(|skin| Segment {
                label: loft_skin_name(skin),
                hover: skin_hover(skin),
                change: (skin != current)
                    .then(|| self.change(loft_tools::with_skin(self.loft, skin))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SIDES, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn closing_row(&mut self, ui: &mut Ui) {
        let closed = self.loft.closed;
        let segments = [(false, OPEN, OPEN_HOVER), (true, CLOSED, CLOSED_HOVER)]
            .into_iter()
            .map(|(wanted, label, hover)| Segment {
                label,
                hover,
                change: (wanted != closed)
                    .then(|| self.change(loft_tools::with_closing(self.loft, wanted))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, CLOSING, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn end_condition_rows(&mut self, ui: &mut Ui, side: Side) {
        let weight = side.weight(self.loft).cloned();
        let segments = [(false, FREE, FREE_HOVER), (true, SQUARE, SQUARE_HOVER)]
            .into_iter()
            .map(|(square, label, hover)| Segment {
                label,
                hover,
                change: (square != weight.is_some()).then(|| {
                    let wanted = square.then(|| Expression::number(1.0));
                    self.change(with_weight(self.loft, side, wanted))
                }),
            })
            .collect();
        let chosen =
            feature_fields::segmented_row(ui, side.caption(), &self.feature.name, segments);
        self.actions.extend(chosen);
        let Some(expression) = weight else {
            return;
        };
        let id = self.id();
        let quantity = Quantity {
            feature: id,
            id: Id::new(("loft-field", id, side.key())),
            expression: &expression,
            dimension: Dimension::NONE,
            rule: Rule::AboveZero,
        };
        let loft = self.loft.clone();
        let model = self.model;
        let drafting = feature_fields::expression_row_drafting(
            ui,
            model,
            side.weight_caption(),
            quantity,
            |value| loft_tools::change(model, id, with_weight(&loft, side, Some(value))),
        );
        self.actions.extend(drafting.into_actions(id));
    }

    fn operation_rows(&mut self, ui: &mut Ui) {
        let rows = OperationRows {
            feature: self.id(),
            operation: self.loft.operation,
            ids: ("loft-operation", "loft-body"),
        };
        let chosen = feature_fields::body_operation_rows(ui, self.model, &rows, |operation| {
            self.change(loft_tools::with_operation(self.loft, operation))
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
    loft: &Loft,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        loft,
        actions,
    };
    widgets::properties(ui, ("loft-properties", feature.id()), |ui| {
        panel.profiles_row(ui);
        panel.closing_row(ui);
        if !loft.closed {
            panel.end_row(ui);
        }
        panel.skin_row(ui);
        if !loft.closed && loft.skin == LoftSkin::Smooth {
            panel.end_condition_rows(ui, Side::Start);
            if loft.end.is_none() {
                panel.end_condition_rows(ui, Side::End);
            }
        }
        panel.operation_rows(ui);
    });
}

fn with_weight(loft: &Loft, side: Side, weight: Option<Expression>) -> Loft {
    match side {
        Side::Start => Loft {
            square_start: weight,
            ..loft.clone()
        },
        Side::End => Loft {
            square_end: weight,
            ..loft.clone()
        },
    }
}
