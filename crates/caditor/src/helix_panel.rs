use caditor_document::{
    ClosedEnds, CoilSizing, CoilTurning, Feature, FeatureId, Helix, HelixKind, MAX_PITCH_ROWS,
    Transaction, capitalized, describe_axis, describe_point,
};
use caditor_expression::{Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    coil_panel::{self, AXIS, DIAMETER, SIZED_BY, TAPER, TURNING},
    feature_fields::{self, AxisRow, Picker, Quantity, Rule, Segment, Shown},
    helix_tools::{self, HelixError},
    icons,
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    widgets,
};

pub const DESCRIPTION: &str = "A helix curve winding around an axis from the axis's start along \
                               it, for a sweep or pipe to follow";
pub const SPIRAL_DESCRIPTION: &str = "A flat spiral curve winding out from its inner diameter in \
                                      the plane square to an axis, like a clock spring, for a \
                                      sweep or pipe to follow";
pub const SHAPE: &str = "Shape";
pub const INNER_DIAMETER: &str = "Inner diameter";
pub const OUTER_DIAMETER: &str = "Outer diameter";
pub const START_ANGLE: &str = "Start angle";
pub const STARTS_AT: &str = "Starts at";
pub const BY_ANGLE: &str = "The start angle below";
pub const ENDS: &str = "Ends";
pub const END_TURNS: &str = "End turns";
pub const END_PITCH: &str = "End pitch";
pub const END_CHANGE: &str = "End change";
pub const PITCH_LAW: &str = "Pitch";
pub const FINAL_PITCH: &str = "Final pitch";
pub const ADD_PITCH_ROW: &str = "Add a row";
pub const REMOVE_PITCH_ROW: &str = "Remove the last row";
const EVEN_PITCH: &str = "Even";
const PROGRESSIVE_PITCH: &str = "Progressive";
const TABLE_PITCH: &str = "Table";
const TABLE_PITCH_HOVER: &str = "Follow a table of pitches by turn: the pitch changes evenly from \
                                 one row to the next and stays at the last row's";
const EVEN_PITCH_HOVER: &str = "Wind at the same pitch from end to end";
const PROGRESSIVE_PITCH_HOVER: &str = "Change the pitch evenly from the pitch at the start to the \
                                       final pitch at the end, as a progressive spring's";
const OPEN_ENDS: &str = "Open";
const CLOSED_ENDS: &str = "Closed";
const OPEN_ENDS_HOVER: &str = "Wind at the same pitch from end to end";
const CLOSED_ENDS_HOVER: &str = "Close the ends as a compression spring's are: the end turns wind at \
                                 the end pitch, changing evenly to the pitch over the end change";
const START_POINT_HOVER: &str =
    "Start the helix at the angle of the selected corner, round edge, sketch point or datum point";
const START_POINT_REMOVE_HOVER: &str = "Start at the typed angle again";
const PICK_HOVER: &str = "Wind around the selected axis, straight edge, round face or sketch line";

fn sizing_hover(kind: HelixKind, sizing: CoilSizing) -> &'static str {
    match (kind, sizing) {
        (_, CoilSizing::TurnsAndPitch) => "Type how many turns and how far apart they are",
        (HelixKind::Helix, CoilSizing::HeightAndTurns) => {
            "Type how high the helix rises and how many turns it makes"
        }
        (HelixKind::Helix, CoilSizing::HeightAndPitch) => {
            "Type how high the helix rises and how far apart its turns are"
        }
        (HelixKind::FlatSpiral, CoilSizing::HeightAndTurns) => {
            "Type how wide the spiral reaches and how many turns it makes"
        }
        (HelixKind::FlatSpiral, CoilSizing::HeightAndPitch) => {
            "Type how wide the spiral reaches and how far apart its turns are"
        }
    }
}

fn sizing_name(kind: HelixKind, sizing: CoilSizing) -> &'static str {
    match (kind, sizing) {
        (HelixKind::FlatSpiral, CoilSizing::HeightAndTurns) => "Outer diameter and turns",
        (HelixKind::FlatSpiral, CoilSizing::HeightAndPitch) => "Outer diameter and pitch",
        (_, sizing) => sizing.name(),
    }
}

fn kind_hover(kind: HelixKind) -> &'static str {
    match kind {
        HelixKind::Helix => "Wind along the axis, rising a pitch each turn",
        HelixKind::FlatSpiral => {
            "Wind outward in the plane square to the axis, growing a pitch each turn, like a \
             clock spring"
        }
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    helix: &'a Helix,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, helix: Helix) -> Result<Transaction, HelixError> {
        helix_tools::change(self.model, self.id(), helix)
    }

    fn apply(&mut self, helix: Helix) {
        let change = self.change(helix);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn axis_row(&mut self, ui: &mut Ui) {
        let (model, id, helix) = (self.model, self.id(), self.helix);
        let row = AxisRow {
            caption: AXIS,
            shown: Some(capitalized(&describe_axis(model.document(), &helix.axis))),
            picker: Picker {
                feature: id,
                slot: Slot::HelixAxis,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    self.selection,
                    (id, Slot::HelixAxis),
                    || helix_tools::axis_change(model, self.selection, id, helix),
                ),
                hover: PICK_HOVER,
            },
        };
        let listed = || {
            feature_fields::model_axis_choices(model, id, Some(&helix.axis), &|axis| {
                self.change(helix_tools::with_axis(helix, axis))
            })
        };
        let mut actions = Vec::new();
        feature_fields::axis_row(ui, model, row, Some(&listed), &mut actions);
        self.actions.extend(actions);
        if let Some(reversed) =
            feature_fields::reverse_row(ui, feature_fields::REVERSE_DIRECTION, helix.reversed)
        {
            self.apply(Helix {
                reversed,
                ..helix.clone()
            });
        }
    }

    fn value_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        (key, expression, dimension, rule): (&'static str, &Expression, Dimension, Rule),
        changed: impl Fn(Expression) -> Helix,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::unique(("helix-field", self.id(), key)),
            expression,
            dimension,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |value| {
                helix_tools::change(self.model, self.id(), changed(value))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn kind_row(&mut self, ui: &mut Ui) {
        let current = self.helix.kind;
        let segments = HelixKind::ALL
            .into_iter()
            .map(|kind| Segment {
                label: kind.name(),
                hover: kind_hover(kind),
                change: (kind != current)
                    .then(|| self.change(helix_tools::with_kind(self.model, self.helix, kind))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SHAPE, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn sizing_rows(&mut self, ui: &mut Ui) {
        let kind = self.helix.kind;
        let current = self.helix.length.sizing();
        let segments = CoilSizing::ALL
            .into_iter()
            .map(|sizing| Segment {
                label: sizing_name(kind, sizing),
                hover: sizing_hover(kind, sizing),
                change: (sizing != current)
                    .then(|| self.change(helix_tools::sized_by(self.model, self.helix, sizing))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, SIZED_BY, &self.feature.name, segments);
        self.actions.extend(chosen);
        for (index, (caption, key, expression)) in coil_panel::length_fields(&self.helix.length)
            .into_iter()
            .enumerate()
        {
            let (dimension, rule) = if key == "turns" {
                (Dimension::NONE, Rule::CoilTurns)
            } else {
                (Dimension::LENGTH, Rule::AboveZero)
            };
            let caption = if self.helix.is_flat() && key == "height" {
                OUTER_DIAMETER
            } else {
                caption
            };
            let helix = self.helix.clone();
            self.value_row(
                ui,
                caption,
                (key, expression, dimension, rule),
                move |value| Helix {
                    length: coil_panel::with_length_value(&helix.length, index, value),
                    ..helix.clone()
                },
            );
        }
    }

    fn diameter_row(&mut self, ui: &mut Ui) {
        let helix = self.helix.clone();
        let field = (
            "diameter",
            &self.helix.diameter,
            Dimension::LENGTH,
            Rule::AboveZero,
        );
        let caption = if self.helix.is_flat() {
            INNER_DIAMETER
        } else {
            DIAMETER
        };
        self.value_row(ui, caption, field, move |diameter| Helix {
            diameter,
            ..helix.clone()
        });
    }

    fn taper_row(&mut self, ui: &mut Ui) {
        let helix = self.helix.clone();
        let field = (
            "taper",
            &self.helix.taper,
            Dimension::ANGLE,
            Rule::CoilTaper,
        );
        self.value_row(ui, TAPER, field, move |taper| Helix {
            taper,
            ..helix.clone()
        });
    }

    fn pitch_law_rows(&mut self, ui: &mut Ui) {
        let tabled = !self.helix.pitch_table.is_empty();
        let progressive = self.helix.final_pitch.is_some() && !tabled;
        let model = self.model;
        let helix = self.helix;
        let laws = [
            (!tabled && !progressive, EVEN_PITCH, EVEN_PITCH_HOVER),
            (progressive, PROGRESSIVE_PITCH, PROGRESSIVE_PITCH_HOVER),
            (tabled, TABLE_PITCH, TABLE_PITCH_HOVER),
        ];
        let segments = laws
            .into_iter()
            .enumerate()
            .map(|(law, (current, label, hover))| Segment {
                label,
                hover,
                change: (!current).then(|| {
                    self.change(match law {
                        0 => helix_tools::with_progression(model, helix, false),
                        1 => helix_tools::with_progression(model, helix, true),
                        _ => helix_tools::with_pitch_table(model, helix, true),
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, PITCH_LAW, &self.feature.name, segments);
        self.actions.extend(chosen);
        if tabled {
            self.pitch_table_rows(ui);
            return;
        }
        let Some(last) = &self.helix.final_pitch else {
            return;
        };
        let helix = self.helix.clone();
        let field = ("final-pitch", last, Dimension::LENGTH, Rule::AboveZero);
        self.value_row(ui, FINAL_PITCH, field, move |pitch| Helix {
            final_pitch: Some(pitch),
            ..helix.clone()
        });
    }

    fn pitch_table_rows(&mut self, ui: &mut Ui) {
        let helix = self.helix.clone();
        for (index, row) in helix.pitch_table.iter().enumerate() {
            let fields = [
                (true, format!("Row {} at turn", index + 1), &row.turns),
                (false, format!("Row {} pitch", index + 1), &row.pitch),
            ];
            for (turns, caption, shown) in fields {
                let (dimension, rule) = if turns {
                    (Dimension::NONE, Rule::AboveZero)
                } else {
                    (Dimension::LENGTH, Rule::AboveZero)
                };
                let key = if turns { "row-turns" } else { "row-pitch" };
                let quantity = Quantity {
                    feature: self.id(),
                    id: Id::unique(("helix-table", index, key, self.id())),
                    expression: shown,
                    dimension,
                    rule,
                };
                let drafting = feature_fields::expression_row_drafting(
                    ui,
                    self.model,
                    &caption,
                    quantity,
                    |value| {
                        helix_tools::change(
                            self.model,
                            self.id(),
                            helix_tools::with_pitch_row(&helix, index, turns, value),
                        )
                    },
                );
                self.actions.extend(drafting.into_actions(self.id()));
            }
        }
        ui.label("");
        let mut change = None;
        ui.horizontal_wrapped(|ui| {
            let add = widgets::small_button(ui, icons::ADD, ADD_PITCH_ROW);
            if ui
                .add_enabled(helix_tools::can_add_pitch_row(&helix), add)
                .on_disabled_hover_text(format!("A helix has at most {MAX_PITCH_ROWS} rows"))
                .clicked()
            {
                change = Some(self.change(helix_tools::with_pitch_row_added(self.model, &helix)));
            }
            let remove = widgets::small_button(ui, icons::SUBTRACT, REMOVE_PITCH_ROW);
            let fewer = (helix.pitch_table.len() > 1)
                .then(|| helix_tools::with_last_pitch_row_removed(&helix));
            if ui
                .add_enabled(fewer.is_some(), remove)
                .on_disabled_hover_text("A table keeps at least one row")
                .clicked()
                && let Some(fewer) = fewer
            {
                change = Some(self.change(fewer));
            }
        });
        ui.end_row();
        if let Some(change) = change {
            self.actions
                .push(feature_fields::applied(&self.feature.name, change));
        }
    }

    fn ends_rows(&mut self, ui: &mut Ui) {
        let closed = self.helix.ends.is_some();
        let segments = [
            (false, OPEN_ENDS, OPEN_ENDS_HOVER),
            (true, CLOSED_ENDS, CLOSED_ENDS_HOVER),
        ]
        .into_iter()
        .map(|(closing, label, hover)| Segment {
            label,
            hover,
            change: (closing != closed)
                .then(|| self.change(helix_tools::with_closed_ends(self.helix, closing))),
        })
        .collect();
        let chosen = feature_fields::segmented_row(ui, ENDS, &self.feature.name, segments);
        self.actions.extend(chosen);
        let current = self.helix;
        let Some(ends) = &current.ends else {
            return;
        };
        let (helix, kept) = (current.clone(), ends.clone());
        let field = ("end-turns", &ends.turns, Dimension::NONE, Rule::ZeroOrMore);
        self.value_row(ui, END_TURNS, field, move |turns| Helix {
            ends: Some(Box::new(ClosedEnds {
                turns,
                ..(*kept).clone()
            })),
            ..helix.clone()
        });
        let (helix, kept) = (current.clone(), ends.clone());
        let field = ("end-pitch", &ends.pitch, Dimension::LENGTH, Rule::AboveZero);
        self.value_row(ui, END_PITCH, field, move |pitch| Helix {
            ends: Some(Box::new(ClosedEnds {
                pitch,
                ..(*kept).clone()
            })),
            ..helix.clone()
        });
        let (helix, kept) = (current.clone(), ends.clone());
        let field = ("end-change", &ends.change, Dimension::NONE, Rule::AboveZero);
        self.value_row(ui, END_CHANGE, field, move |change| Helix {
            ends: Some(Box::new(ClosedEnds {
                change,
                ..(*kept).clone()
            })),
            ..helix.clone()
        });
    }

    fn start_point_row(&mut self, ui: &mut Ui) {
        let (model, id, helix) = (self.model, self.id(), self.helix);
        let shown = match &helix.start_point {
            Some(point) => Shown::Named(capitalized(&describe_point(model.document(), point))),
            None => Shown::Named(BY_ANGLE.to_owned()),
        };
        let picker = Picker {
            feature: id,
            slot: Slot::HelixStart,
            selected: feature_fields::offered_change(
                ui.ctx(),
                model,
                self.selection,
                (id, Slot::HelixStart),
                || helix_tools::start_point_change(model, self.selection, id, helix),
            ),
            hover: START_POINT_HOVER,
        };
        let mut actions = Vec::new();
        let removed = feature_fields::reference_row(
            ui,
            model,
            STARTS_AT,
            shown,
            picker,
            helix
                .start_point
                .is_some()
                .then_some(START_POINT_REMOVE_HOVER),
            &mut actions,
        );
        self.actions.extend(actions);
        if removed {
            self.apply(helix_tools::with_start_point(helix, None));
        }
    }

    fn start_row(&mut self, ui: &mut Ui) {
        self.start_point_row(ui);
        if self.helix.start_point.is_some() {
            return;
        }
        let helix = self.helix.clone();
        let field = ("start", &self.helix.start, Dimension::ANGLE, Rule::Any);
        self.value_row(ui, START_ANGLE, field, move |start| Helix {
            start,
            ..helix.clone()
        });
    }

    fn turning_row(&mut self, ui: &mut Ui) {
        let current = self.helix.turning;
        let segments = CoilTurning::ALL
            .into_iter()
            .map(|turning| Segment {
                label: turning.name(),
                hover: coil_panel::turning_hover(turning),
                change: (turning != current)
                    .then(|| self.change(helix_tools::with_turning(self.helix, turning))),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, TURNING, &self.feature.name, segments);
        self.actions.extend(chosen);
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    helix: &Helix,
) {
    let mut panel = Panel {
        model,
        selection,
        feature,
        helix,
        actions,
    };
    widgets::properties(ui, ("helix-properties", feature.id()), |ui| {
        let flat = helix.is_flat();
        feature_fields::description_row(
            ui,
            if flat {
                SPIRAL_DESCRIPTION
            } else {
                DESCRIPTION
            },
        );
        panel.kind_row(ui);
        panel.axis_row(ui);
        panel.sizing_rows(ui);
        panel.diameter_row(ui);
        if !flat {
            panel.taper_row(ui);
            panel.pitch_law_rows(ui);
            panel.ends_rows(ui);
        }
        panel.start_row(ui);
        panel.turning_row(ui);
    });
}
