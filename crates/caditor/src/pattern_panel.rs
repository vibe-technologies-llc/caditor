use caditor_document::{
    AxisReference, CircularPattern, CopyOrientation, CurvePattern, CurveSpacing, Feature,
    FeatureId, Instance, LinearDirection, LinearSpacing, ORIGINAL_INSTANCE, Pattern, PatternKind,
    PointReference, PointsPattern, Transaction, capitalized, describe_axis, describe_point,
    instance_name,
};
use caditor_expression::{BinaryOperator, Dimension, Expression};
use egui::{Id, Ui};

use crate::{
    feature_fields::{self, Choice, Picker, Quantity, REVERSE_DIRECTION, Rule, Segment},
    icons,
    model::{Action, Model},
    pattern_tools::{self, Reference, Shape},
    reference_picking::Slot,
    selection::Selection,
    sketch_pattern_tools, widgets,
};

pub const CIRCULAR_HINT: &str = "A total angle of 360° spaces the copies evenly; a smaller angle \
                                 runs from the body to the last copy.";

pub const CURVE_HINT: &str = "The copies are carried along the curve from where it starts: each \
                              moves as the start of the curve would to reach its place.";
pub const POINTS_HINT: &str = "Each copy is moved from the base point to one lone point of the \
                               sketch; a point on the base point is the original itself.";

const COUNT: (Dimension, Rule) = (Dimension::NONE, Rule::Count);

pub const CURVE: &str = "Curve";
pub const POINTS: &str = "Points";
pub const BASE_POINT: &str = "Base point";
pub const SPACED: &str = "Spaced";
pub const SPREAD_EVENLY: &str = "Evenly";
pub const BY_DISTANCE: &str = "By distance";
pub const ORIENTATION: &str = "Copies";
pub const KEPT: &str = "Kept as they are";
pub const FOLLOWING: &str = "Turned with the curve";
pub const BRING_BACK: &str = "Bring the copies back";
pub const USE_ORIGIN: &str = "Use the origin";
const POINTS_LEFT_OUT_HINT: &str = "Click a copy in the view to leave it out of the pattern.";

pub const INSTANCES: &str = "Instances";
pub const REPEATS: &str = "Repeats";
pub const WHOLE_BODY: &str = "The whole body";
pub const REPEAT_CHOSEN: &str = "Repeat the chosen features";
const REPEAT_CHOSEN_HINT: &str = "Choose extrusions, revolves, holes or primitives of this body above the \
                                  pattern in the tree (Ctrl+click), then repeat them instead of \
                                  the whole body";
pub const NO_SECOND_DIRECTION: &str = "None";
pub const MEASURED_EACH: &str = "Each";
pub const MEASURED_OVERALL: &str = "Overall";
const INSTANCES_HINT: &str = "Click a copy to leave it out of the pattern, and again to bring it \
                              back.";

pub struct DirectionCaptions {
    salt: &'static str,
    pub count: &'static str,
    measure: &'static str,
    pub spacing: &'static str,
    pub total: &'static str,
    reverse: &'static str,
}

pub const FIRST: DirectionCaptions = DirectionCaptions {
    salt: "",
    count: "Count",
    measure: "Measured",
    spacing: "Spacing",
    total: "Total length",
    reverse: REVERSE_DIRECTION,
};

pub const SECOND: DirectionCaptions = DirectionCaptions {
    salt: "second-",
    count: "Second count",
    measure: "Second measured",
    spacing: "Second spacing",
    total: "Second total length",
    reverse: "Reverse second direction",
};

pub const TOTAL_ANGLE: &str = "Total angle";

pub fn direction_captions(index: usize) -> &'static DirectionCaptions {
    match index {
        0 => &FIRST,
        _ => &SECOND,
    }
}

fn measured_label(measured: LinearSpacing) -> &'static str {
    match measured {
        LinearSpacing::BetweenCopies => MEASURED_EACH,
        LinearSpacing::Total => MEASURED_OVERALL,
    }
}

fn measured_hover(measured: LinearSpacing) -> &'static str {
    match measured {
        LinearSpacing::BetweenCopies => "The length is the distance from one copy to the next",
        LinearSpacing::Total => {
            "The length runs from the body to the last copy, and the copies share it evenly"
        }
    }
}

pub fn respaced(direction: &LinearDirection, measured: LinearSpacing) -> Expression {
    let Expression::Number(count) = direction.count else {
        return direction.spacing.clone();
    };
    let steps = count - 1.0;
    if steps < 1.0 || steps.fract() != 0.0 {
        return direction.spacing.clone();
    }
    match (measured, &direction.spacing) {
        (LinearSpacing::Total, Expression::Measure(value, unit)) => {
            Expression::measure(value * steps, *unit)
        }
        (LinearSpacing::BetweenCopies, Expression::Measure(value, unit))
            if (value / steps).fract() == 0.0 =>
        {
            Expression::measure(value / steps, *unit)
        }
        (LinearSpacing::Total, spacing) => Expression::binary(
            BinaryOperator::Multiply,
            spacing.clone(),
            Expression::number(steps),
        ),
        (LinearSpacing::BetweenCopies, spacing) => Expression::binary(
            BinaryOperator::Divide,
            spacing.clone(),
            Expression::number(steps),
        ),
    }
}

fn instance_hover(instance: Instance, kept: bool) -> String {
    let name = capitalized(&instance_name(instance));
    match (instance == ORIGINAL_INSTANCE, kept) {
        (true, _) => format!("{name} body is always kept"),
        (false, true) => format!("{name} is made; click to leave it out"),
        (false, false) => format!("{name} is left out; click to make it again"),
    }
}

fn shape_label(shape: Shape) -> &'static str {
    match shape {
        Shape::Linear => "Linear",
        Shape::Circular => "Circular",
        Shape::Curve => CURVE,
        Shape::Points => POINTS,
    }
}

fn spacing_hover(measured: CurveSpacing) -> &'static str {
    match measured {
        CurveSpacing::Spread => "The copies share the whole length of the curve evenly",
        CurveSpacing::Distance => "The copies stand the spacing apart, measured along the curve",
    }
}

fn orientation_hover(orientation: CopyOrientation) -> &'static str {
    match orientation {
        CopyOrientation::Kept => "Every copy keeps the original's orientation",
        CopyOrientation::Following => "Every copy turns as the curve turns from where it starts",
    }
}

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    chosen: &'a [FeatureId],
    feature: &'a Feature,
    pattern: &'a Pattern,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn id(&self) -> FeatureId {
        self.feature.id()
    }

    fn change(&self, kind: PatternKind) -> Result<Transaction, String> {
        pattern_tools::change(
            self.model,
            self.id(),
            Pattern {
                kind,
                ..self.pattern.clone()
            },
        )
    }

    fn measure_row(
        &mut self,
        ui: &mut Ui,
        direction: &LinearDirection,
        captions: &DirectionCaptions,
        rebuild: &dyn Fn(LinearDirection) -> PatternKind,
    ) {
        let segments = [LinearSpacing::BetweenCopies, LinearSpacing::Total]
            .into_iter()
            .map(|measured| Segment {
                label: measured_label(measured),
                hover: measured_hover(measured),
                change: (measured != direction.measured).then(|| {
                    self.change(rebuild(LinearDirection {
                        spacing: respaced(direction, measured),
                        measured,
                        ..direction.clone()
                    }))
                }),
            })
            .collect();
        let chosen =
            feature_fields::segmented_row(ui, captions.measure, &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn instances_row(&mut self, ui: &mut Ui) {
        let Some([columns, rows]) = self.pattern.instances(self.model.parameters()) else {
            return;
        };
        if columns * rows < 2 {
            return;
        }
        widgets::caption(ui, INSTANCES);
        let mut toggled = None;
        ui.vertical(|ui| {
            for row in 0..rows {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = ui.spacing().item_spacing.y;
                    for column in 0..columns {
                        let instance = [column, row];
                        let kept = !self.pattern.is_skipped(instance);
                        let name = capitalized(&instance_name(instance));
                        let hover = instance_hover(instance, kept);
                        let original = instance == ORIGINAL_INSTANCE;
                        let response = ui
                            .add_enabled_ui(!original, |ui| {
                                widgets::instance_toggle(ui, kept, &name, &hover)
                            })
                            .inner;
                        if response.clicked() {
                            toggled = Some(instance);
                        }
                    }
                });
            }
            let left_out = self.pattern.skipped_within([columns, rows]);
            let summary = match left_out {
                0 => INSTANCES_HINT.to_owned(),
                1 => "1 copy is left out.".to_owned(),
                count => format!("{count} copies are left out."),
            };
            ui.label(widgets::muted(summary, ui));
        });
        ui.end_row();
        if let Some(pattern) = toggled.and_then(|instance| self.pattern.toggled(instance)) {
            let change = pattern_tools::change(self.model, self.id(), pattern);
            self.apply(change);
        }
    }

    fn repeats_rows(&mut self, ui: &mut Ui) {
        let document = self.model.document();
        let repeated = self.pattern.repeated.clone();
        widgets::caption(ui, REPEATS);
        if repeated.is_empty() {
            ui.label(WHOLE_BODY);
            ui.end_row();
        }
        for (index, feature) in repeated.iter().enumerate() {
            if index > 0 {
                ui.label("");
            }
            let name = feature_fields::feature_name(document, *feature);
            let hover = format!("Stop repeating {}", name.unwrap_or("the missing feature"));
            let mut dropped = false;
            ui.horizontal(|ui| {
                match name {
                    Some(name) => {
                        ui.label(name);
                    }
                    None => feature_fields::missing(ui, "Missing feature"),
                }
                dropped = widgets::icon_button(ui, icons::REMOVE, &hover).clicked();
            });
            ui.end_row();
            if dropped {
                let kept = repeated
                    .iter()
                    .copied()
                    .filter(|kept| kept != feature)
                    .collect();
                let change = pattern_tools::repeating(self.model, self.id(), self.pattern, kept);
                self.apply(change);
            }
        }
        let offered = pattern_tools::repeatable(document, self.chosen)
            .filter(|(body, chosen)| *body == self.pattern.body && *chosen != repeated);
        ui.label("");
        let button = widgets::small_button(ui, icons::ADD, REPEAT_CHOSEN);
        let hover = match &offered {
            Some((body, chosen)) => format!(
                "Repeat {} instead of {}",
                pattern_tools::subject(document, *body, chosen),
                pattern_tools::subject(document, self.pattern.body, &repeated)
            ),
            None => REPEAT_CHOSEN_HINT.to_owned(),
        };
        let clicked = ui
            .add_enabled(offered.is_some(), button)
            .on_hover_text(&hover)
            .on_disabled_hover_text(&hover)
            .clicked();
        ui.end_row();
        if clicked && let Some((_, chosen)) = offered {
            let change = pattern_tools::repeating(self.model, self.id(), self.pattern, chosen);
            self.apply(change);
        }
    }

    fn apply(&mut self, change: Result<Transaction, String>) {
        self.actions
            .push(feature_fields::applied(&self.feature.name, change));
    }

    fn shape_row(&mut self, ui: &mut Ui) {
        let current = Shape::of(&self.pattern.kind);
        let segments = Shape::ALL
            .into_iter()
            .map(|shape| Segment {
                label: shape_label(shape),
                hover: shape.description(),
                change: (shape != current).then(|| {
                    pattern_tools::reshaped(self.model, self.id(), self.pattern, shape)
                        .map_err(str::to_owned)
                        .and_then(|reshaped| self.change(reshaped.kind))
                }),
            })
            .collect();
        let chosen = feature_fields::segmented_row(ui, "Shape", &self.feature.name, segments);
        self.actions.extend(chosen);
    }

    fn picker(
        &self,
        ctx: &egui::Context,
        slot: Slot,
        reference: Reference,
        hover: &'static str,
    ) -> Picker<'static> {
        Picker {
            feature: self.id(),
            slot,
            selected: feature_fields::offered_change(
                ctx,
                self.model,
                self.selection,
                (self.id(), slot),
                || {
                    pattern_tools::selected_change(
                        self.model,
                        self.selection,
                        self.id(),
                        self.pattern,
                        reference,
                    )
                },
            ),
            hover,
        }
    }

    fn axis_choices(&self, reference: Reference, current: Option<&AxisReference>) -> Vec<Choice> {
        let document = self.model.document();
        let none = match (&self.pattern.kind, reference) {
            (PatternKind::Linear { first, .. }, Reference::Second) => Some(Choice {
                label: NO_SECOND_DIRECTION.to_owned(),
                selected: current.is_none(),
                change: self
                    .change(PatternKind::Linear {
                        first: first.clone(),
                        second: None,
                    })
                    .map(Action::Apply),
            }),
            _ => None,
        };
        let axes = pattern_tools::listed_axes(self.model, self.id())
            .into_iter()
            .map(|axis| Choice {
                label: capitalized(&describe_axis(document, &axis)),
                selected: current == Some(&axis),
                change: pattern_tools::with_axis(self.model, self.pattern, reference, axis)
                    .map_err(str::to_owned)
                    .and_then(|pattern| pattern_tools::change(self.model, self.id(), pattern))
                    .map(Action::Apply),
            });
        none.into_iter().chain(axes).collect()
    }

    fn reference_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        axis: Option<&AxisReference>,
        (reference, slot): (Reference, Slot),
        hover: &'static str,
    ) {
        widgets::caption(ui, caption);
        let shown = axis.map_or_else(
            || NO_SECOND_DIRECTION.to_owned(),
            |axis| capitalized(&describe_axis(self.model.document(), axis)),
        );
        let id = Id::new(("pattern-axis", caption, self.id()));
        ui.vertical(|ui| {
            let chosen =
                feature_fields::combo(ui, id, shown, || self.axis_choices(reference, axis));
            self.actions.extend(chosen);
            let picker = self.picker(ui.ctx(), slot, reference, hover);
            feature_fields::reference_picker(ui, self.model, picker, self.actions);
        });
        ui.end_row();
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        salt: &str,
        expression: &Expression,
        (dimension, rule): (Dimension, Rule),
        rebuild: impl Fn(Expression) -> PatternKind,
    ) {
        let quantity = Quantity {
            feature: self.id(),
            id: Id::new(("pattern-field", salt, self.id())),
            expression,
            dimension,
            rule,
        };
        let drafting =
            feature_fields::expression_row_drafting(ui, self.model, caption, quantity, |parsed| {
                self.change(rebuild(parsed))
            });
        self.actions.extend(drafting.into_actions(self.id()));
    }

    fn reverse_row(
        &mut self,
        ui: &mut Ui,
        label: &str,
        reversed: bool,
        rebuild: impl Fn(bool) -> PatternKind,
    ) {
        if let Some(flipped) = feature_fields::reverse_row(ui, label, reversed) {
            let change = self.change(rebuild(flipped));
            self.apply(change);
        }
    }

    fn direction_rows(
        &mut self,
        ui: &mut Ui,
        direction: &LinearDirection,
        captions: &DirectionCaptions,
        rebuild: &dyn Fn(LinearDirection) -> PatternKind,
    ) {
        let salt = format!("{}count", captions.salt);
        self.expression(
            ui,
            captions.count,
            &salt,
            &direction.count,
            COUNT,
            |count| {
                rebuild(LinearDirection {
                    count,
                    ..direction.clone()
                })
            },
        );
        self.measure_row(ui, direction, captions, rebuild);
        let salt = format!("{}spacing", captions.salt);
        let caption = match direction.measured {
            LinearSpacing::BetweenCopies => captions.spacing,
            LinearSpacing::Total => captions.total,
        };
        self.expression(
            ui,
            caption,
            &salt,
            &direction.spacing,
            (Dimension::LENGTH, Rule::AboveZeroOrReverse),
            |spacing| {
                rebuild(LinearDirection {
                    spacing,
                    ..direction.clone()
                })
            },
        );
        self.reverse_row(ui, captions.reverse, direction.reversed, |reversed| {
            rebuild(LinearDirection {
                reversed,
                ..direction.clone()
            })
        });
    }

    fn linear_rows(
        &mut self,
        ui: &mut Ui,
        first: &LinearDirection,
        second: Option<&LinearDirection>,
    ) {
        self.reference_row(
            ui,
            "Direction",
            Some(&first.axis),
            (Reference::First, Slot::PatternDirection),
            "Repeat along the selected edge, round face or axis",
        );
        let kept = second.cloned();
        self.direction_rows(ui, first, &FIRST, &|first| PatternKind::Linear {
            first,
            second: kept.clone(),
        });
        self.reference_row(
            ui,
            "Second direction",
            second.map(|second| &second.axis),
            (Reference::Second, Slot::PatternSecond),
            "Also repeat the rows along the selected edge, round face or axis",
        );
        if let Some(second) = second {
            let first = first.clone();
            self.direction_rows(ui, second, &SECOND, &|second| PatternKind::Linear {
                first: first.clone(),
                second: Some(second),
            });
        }
    }

    fn sketch_row(&mut self, ui: &mut Ui, caption: &str, current: FeatureId, hover: &'static str) {
        let shape = Shape::of(&self.pattern.kind);
        let document = self.model.document();
        widgets::caption(ui, caption);
        let before = document.feature_index(self.id()).unwrap_or(0);
        let id = Id::new(("pattern-sketch", self.id()));
        ui.vertical(|ui| {
            let shown = feature_fields::combo_text(
                ui,
                feature_fields::feature_name(document, current),
                "Missing sketch",
            );
            let chosen = feature_fields::combo(ui, id, shown, || {
                sketch_pattern_tools::listed(self.model, shape, before)
                    .into_iter()
                    .map(|sketch| Choice {
                        label: feature_fields::feature_name(document, sketch)
                            .unwrap_or_default()
                            .to_owned(),
                        selected: sketch == current,
                        change: sketch_pattern_tools::with_sketch(self.pattern, sketch)
                            .map_err(str::to_owned)
                            .and_then(|pattern| {
                                pattern_tools::change(self.model, self.id(), pattern)
                            })
                            .map(Action::Apply),
                    })
                    .collect()
            });
            self.actions.extend(chosen);
            let (model, selection, feature, pattern) =
                (self.model, self.selection, self.id(), self.pattern);
            let picker = Picker {
                feature,
                slot: Slot::PatternPath,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    selection,
                    (feature, Slot::PatternPath),
                    || sketch_pattern_tools::sketch_change(model, selection, feature, pattern),
                ),
                hover,
            };
            feature_fields::reference_picker(ui, model, picker, self.actions);
        });
        ui.end_row();
    }

    fn curve_rows(&mut self, ui: &mut Ui, curve: &CurvePattern) {
        self.sketch_row(
            ui,
            CURVE,
            curve.sketch,
            "Follow the curves of the selected sketch",
        );
        self.expression(ui, "Count", "count", &curve.count, COUNT, |count| {
            PatternKind::Curve(CurvePattern {
                count,
                ..curve.clone()
            })
        });
        let segments = [
            (CurveSpacing::Spread, SPREAD_EVENLY),
            (CurveSpacing::Distance, BY_DISTANCE),
        ]
        .into_iter()
        .map(|(measured, label)| Segment {
            label,
            hover: spacing_hover(measured),
            change: (measured != curve.measured).then(|| {
                self.change(PatternKind::Curve(CurvePattern {
                    measured,
                    ..curve.clone()
                }))
            }),
        })
        .collect();
        let chosen = feature_fields::segmented_row(ui, SPACED, &self.feature.name, segments);
        self.actions.extend(chosen);
        if curve.measured == CurveSpacing::Distance {
            self.expression(
                ui,
                "Spacing",
                "spacing",
                &curve.spacing,
                (Dimension::LENGTH, Rule::AboveZeroOrReverse),
                |spacing| {
                    PatternKind::Curve(CurvePattern {
                        spacing,
                        ..curve.clone()
                    })
                },
            );
        }
        let segments = [
            (CopyOrientation::Kept, KEPT),
            (CopyOrientation::Following, FOLLOWING),
        ]
        .into_iter()
        .map(|(orientation, label)| Segment {
            label,
            hover: orientation_hover(orientation),
            change: (orientation != curve.orientation).then(|| {
                self.change(PatternKind::Curve(CurvePattern {
                    orientation,
                    ..curve.clone()
                }))
            }),
        })
        .collect();
        let chosen = feature_fields::segmented_row(ui, ORIENTATION, &self.feature.name, segments);
        self.actions.extend(chosen);
        self.reverse_row(ui, REVERSE_DIRECTION, curve.reversed, |reversed| {
            PatternKind::Curve(CurvePattern {
                reversed,
                ..curve.clone()
            })
        });
    }

    fn points_rows(&mut self, ui: &mut Ui, points: &PointsPattern) {
        self.sketch_row(
            ui,
            POINTS,
            points.sketch,
            "Place the copies at the lone points of the selected sketch",
        );
        widgets::caption(ui, BASE_POINT);
        let (model, selection, feature, pattern) =
            (self.model, self.selection, self.id(), self.pattern);
        ui.vertical(|ui| {
            ui.label(capitalized(&describe_point(model.document(), &points.base)));
            let picker = Picker {
                feature,
                slot: Slot::PatternBase,
                selected: feature_fields::offered_change(
                    ui.ctx(),
                    model,
                    selection,
                    (feature, Slot::PatternBase),
                    || sketch_pattern_tools::base_change(model, selection, feature, pattern),
                ),
                hover: "Move the copies from the selected corner, round edge, sphere, sketch \
                        point or datum point",
            };
            feature_fields::reference_picker(ui, model, picker, self.actions);
            if points.base != PointReference::Origin {
                let button = widgets::small_button(ui, icons::ORIGIN, USE_ORIGIN);
                if ui
                    .add(button)
                    .on_hover_text("Move the copies from the origin")
                    .clicked()
                {
                    self.actions.push(feature_fields::applied(
                        &self.feature.name,
                        sketch_pattern_tools::with_base(
                            model,
                            feature,
                            pattern,
                            PointReference::Origin,
                        ),
                    ));
                }
            }
        });
        ui.end_row();
        self.left_out_row(ui);
    }

    fn left_out_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, INSTANCES);
        let left_out = self.pattern.skipped.len();
        let mut brought_back = false;
        ui.vertical(|ui| {
            let summary = match left_out {
                0 => POINTS_LEFT_OUT_HINT.to_owned(),
                1 => "1 copy is left out.".to_owned(),
                count => format!("{count} copies are left out."),
            };
            ui.label(widgets::muted(summary, ui));
            if left_out > 0 {
                let button = widgets::small_button(ui, icons::ADD, BRING_BACK);
                brought_back = ui
                    .add(button)
                    .on_hover_text("Make every copy left out again")
                    .clicked();
            }
        });
        ui.end_row();
        if brought_back {
            let change = pattern_tools::change(
                self.model,
                self.id(),
                Pattern {
                    skipped: Default::default(),
                    ..self.pattern.clone()
                },
            );
            self.apply(change);
        }
    }

    fn circular_rows(&mut self, ui: &mut Ui, circular: &CircularPattern) {
        self.reference_row(
            ui,
            "Axis",
            Some(&circular.axis),
            (Reference::First, Slot::PatternDirection),
            "Turn about the selected axis, straight edge or round face",
        );
        self.expression(ui, "Count", "count", &circular.count, COUNT, |count| {
            PatternKind::Circular(CircularPattern {
                count,
                ..circular.clone()
            })
        });
        self.expression(
            ui,
            TOTAL_ANGLE,
            "angle",
            &circular.angle,
            (Dimension::ANGLE, Rule::Turn),
            |angle| {
                PatternKind::Circular(CircularPattern {
                    angle,
                    ..circular.clone()
                })
            },
        );
        self.reverse_row(ui, REVERSE_DIRECTION, circular.reversed, |reversed| {
            PatternKind::Circular(CircularPattern {
                reversed,
                ..circular.clone()
            })
        });
    }
}

pub fn show(
    ui: &mut Ui,
    model: &Model,
    (selection, chosen): (&Selection, &[FeatureId]),
    actions: &mut Vec<Action>,
    feature: &Feature,
    pattern: &Pattern,
) {
    let id = feature.id();
    let mut panel = Panel {
        model,
        selection,
        chosen,
        feature,
        pattern,
        actions,
    };
    widgets::properties(ui, ("pattern-properties", id), |ui| {
        panel.shape_row(ui);
        match &pattern.kind {
            PatternKind::Linear { first, second } => panel.linear_rows(ui, first, second.as_ref()),
            PatternKind::Circular(circular) => panel.circular_rows(ui, circular),
            PatternKind::Curve(curve) => panel.curve_rows(ui, curve),
            PatternKind::Points(points) => panel.points_rows(ui, points),
        }
        panel.instances_row(ui);
        panel.repeats_rows(ui);
        feature_fields::feature_row(ui, model.document(), "Body", pattern.body);
    });
    let hint = match pattern.kind {
        PatternKind::Linear { .. } => None,
        PatternKind::Circular(_) => Some(CIRCULAR_HINT),
        PatternKind::Curve(_) => Some(CURVE_HINT),
        PatternKind::Points(_) => Some(POINTS_HINT),
    };
    if let Some(hint) = hint {
        feature_fields::info_callout(ui, hint);
    }
}
