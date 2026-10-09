use caditor_document::{
    Bore, Feature, FeatureId, Resolution, Thread, ThreadFamily, ThreadHand, ThreadLength,
    ThreadSide, Transaction,
};
use caditor_expression::Dimension;
use egui::{Id, Ui};

use crate::{
    bodies,
    feature_fields::{self, Choice, Picker, Quantity, Rule, Segment},
    field,
    model::{Action, Model},
    reference_picking::Slot,
    selection::Selection,
    thread_tools, widgets,
};

pub const DESCRIPTION: &str =
    "Draws a thread on a round face and names it in the exports; the solid is unchanged";
pub const START_FROM_OTHER_END: &str = "Start from the other end";
const DEFAULT_DEPTH: f64 = 10.0;
const NO_SHAPE_YET: &str = "A face of a body that has no shape yet";
const GONE: &str = "A face that is no longer there";
const FACE_HOVER: &str = "Thread the selected round face instead";

fn side_words(side: ThreadSide) -> &'static str {
    match side {
        ThreadSide::Internal => "Internal, in a bore",
        ThreadSide::External => "External, on a shaft or boss",
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: &'a Feature,
    thread: &'a Thread,
    bore: Option<Bore>,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, thread: Thread) -> Result<Transaction, String> {
        let document = self.model.document();
        let transaction = thread_tools::edit(document, self.id(), thread)
            .ok_or_else(|| "The feature no longer exists".to_owned())?;
        field::checked(document, transaction)
    }

    fn apply(&mut self, thread: Thread) {
        let change = self.change(thread);
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn side(&self) -> Option<ThreadSide> {
        self.bore.map(|bore| bore.side)
    }

    fn face_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Face");
        let evaluation = self.model.evaluation();
        let seen = evaluation
            .body_result_seen_by(self.id(), self.thread.body)
            .or_else(|| bodies::shown(evaluation, self.thread.body));
        let text = match seen {
            None => NO_SHAPE_YET.to_owned(),
            Some(body) => match self.thread.resolution(&body.solid) {
                Resolution::One(face) => {
                    bodies::describe_face_id(self.model.document(), body, face)
                }
                Resolution::Pieces(pieces) => match pieces.first() {
                    Some(face) => format!(
                        "{}, split into {} pieces",
                        bodies::describe_face_id(self.model.document(), body, *face),
                        pieces.len()
                    ),
                    None => GONE.to_owned(),
                },
                Resolution::Tied(_) | Resolution::Missing => GONE.to_owned(),
            },
        };
        let (model, selection, id, thread) = (self.model, self.selection, self.id(), self.thread);
        let picker = Picker {
            feature: id,
            slot: Slot::ThreadFace,
            selected: feature_fields::offered_change(
                ui.ctx(),
                model,
                selection,
                (id, Slot::ThreadFace),
                || thread_tools::face_change(model, selection, id, thread),
            ),
            hover: FACE_HOVER,
        };
        let mut picked = Vec::new();
        ui.vertical(|ui| {
            if text == GONE {
                feature_fields::missing(ui, GONE);
            } else {
                ui.label(text);
            }
            feature_fields::reference_picker(ui, model, picker, &mut picked);
        });
        self.actions.extend(picked);
        ui.end_row();
        if let Some(side) = self.side() {
            widgets::caption(ui, "Side");
            ui.label(widgets::muted(side_words(side), ui));
            ui.end_row();
        }
    }

    fn designation_row(&self, ui: &mut Ui) {
        widgets::caption(ui, "Designation");
        ui.label(widgets::strong(self.thread.designation().text()));
        ui.end_row();
    }

    fn standard_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Standard");
        let current = self.thread.size.family();
        let side = self.side().unwrap_or(ThreadSide::Internal);
        let diameter = self
            .bore
            .map_or(self.thread.size.fitting_diameter(side), |bore| {
                bore.diameter
            });
        let name = self.feature.name.clone();
        let chosen = feature_fields::combo(
            ui,
            Id::new(("thread-standard", self.id())),
            current.label(),
            || {
                ThreadFamily::ALL
                    .into_iter()
                    .map(|family| Choice {
                        label: family.label().to_owned(),
                        selected: family == current,
                        change: self
                            .change(Thread {
                                size: family.nearest(diameter, side),
                                class: family.default_class(side),
                                ..self.thread.clone()
                            })
                            .map(|transaction| feature_fields::applied(&name, Ok(transaction))),
                    })
                    .collect()
            },
        );
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn size_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Size");
        let current = self.thread.size;
        let name = self.feature.name.clone();
        let chosen = feature_fields::combo(
            ui,
            Id::new(("thread-size", self.id())),
            current.label(),
            || {
                current
                    .family()
                    .sizes()
                    .into_iter()
                    .map(|size| Choice {
                        label: size.label(),
                        selected: size == current,
                        change: self
                            .change(Thread {
                                size,
                                ..self.thread.clone()
                            })
                            .map(|transaction| feature_fields::applied(&name, Ok(transaction))),
                    })
                    .collect()
            },
        );
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn class_row(&mut self, ui: &mut Ui) {
        let family = self.thread.size.family();
        let offered: Vec<_> = match self.side() {
            Some(side) => family.classes(side).to_vec(),
            None => [ThreadSide::Internal, ThreadSide::External]
                .into_iter()
                .flat_map(|side| family.classes(side).iter().copied())
                .collect(),
        };
        if offered.len() < 2 && offered.contains(&self.thread.class) {
            return;
        }
        widgets::caption(ui, "Class");
        let current = self.thread.class;
        let name = self.feature.name.clone();
        let chosen = feature_fields::combo(
            ui,
            Id::new(("thread-class", self.id())),
            current.label(),
            || {
                offered
                    .iter()
                    .map(|class| Choice {
                        label: class.label().to_owned(),
                        selected: *class == current,
                        change: self
                            .change(Thread {
                                class: *class,
                                ..self.thread.clone()
                            })
                            .map(|transaction| feature_fields::applied(&name, Ok(transaction))),
                    })
                    .collect()
            },
        );
        self.actions.extend(chosen);
        ui.end_row();
    }

    fn hand_row(&mut self, ui: &mut Ui) {
        let segments = ThreadHand::ALL
            .into_iter()
            .map(|hand| Segment {
                label: hand.label(),
                hover: match hand {
                    ThreadHand::Right => "Tightens turning clockwise, as most threads do",
                    ThreadHand::Left => "Tightens turning anticlockwise",
                },
                change: (hand != self.thread.hand).then(|| {
                    self.change(Thread {
                        hand,
                        ..self.thread.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Hand", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn length_rows(&mut self, ui: &mut Ui) {
        let depth = match &self.thread.length {
            ThreadLength::Full => None,
            ThreadLength::Depth(depth) => Some(depth.clone()),
        };
        let unit = self.model.length_unit();
        let options = [
            (
                "Full face",
                "Thread the whole length of the face",
                ThreadLength::Full,
                depth.is_none(),
            ),
            (
                "Depth",
                "Thread a length from one end of the face",
                ThreadLength::Depth(unit.default_length(DEFAULT_DEPTH)),
                depth.is_some(),
            ),
        ];
        let segments = options
            .into_iter()
            .map(|(label, hover, length, current)| Segment {
                label,
                hover,
                change: (!current).then(|| {
                    self.change(Thread {
                        length,
                        ..self.thread.clone()
                    })
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Length", &self.feature.name, segments);
        self.actions.extend(chosen);
        let Some(depth) = depth else {
            return;
        };
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("thread-depth", self.id())),
            expression: &depth,
            dimension: Dimension::LENGTH,
            rule: Rule::AboveZero,
        };
        let thread = self.thread;
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, "Depth", quantity, |value| {
                self.change(Thread {
                    length: ThreadLength::Depth(value),
                    ..thread.clone()
                })
            });
        self.actions.extend(drafting.into_actions(self.id()));
        if let Some(reversed) =
            feature_fields::reverse_row(ui, START_FROM_OTHER_END, self.thread.reversed)
        {
            self.apply(Thread {
                reversed,
                ..self.thread.clone()
            });
        }
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    thread: &Thread,
) {
    let id = feature.id();
    let bore = thread_tools::threaded_bore(model.evaluation(), id, thread);
    let mut panel = Panel {
        model,
        selection,
        feature,
        thread,
        bore,
        actions,
    };
    widgets::properties(ui, ("thread-properties", id), |ui| {
        feature_fields::description_row(ui, DESCRIPTION);
        panel.face_row(ui);
        panel.designation_row(ui);
        panel.standard_row(ui);
        panel.size_row(ui);
        panel.class_row(ui);
        panel.hand_row(ui);
        panel.length_rows(ui);
        feature_fields::feature_row(ui, model.document(), "Body", thread.body);
    });
}
