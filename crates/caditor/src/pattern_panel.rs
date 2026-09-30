use caditor_document::{
    AxisReference, CircularPattern, Feature, FeatureId, LinearDirection, MAX_PATTERN_INSTANCES,
    Pattern, PatternKind, Transaction, capitalized, describe_axis,
};
use caditor_expression::{Dimension, Expression};
use egui::{Button, ComboBox, Id, Ui};

use crate::{
    field::{self, Expected},
    icons,
    model::{Action, Model, Notice},
    pattern_tools::{self, FULL_TURN, Reference, Shape},
    selection::Selection,
    widgets::{self, FIELD_WIDTH},
};

const USE_SELECTED: &str = "Use selected";
const WHOLE_TOLERANCE: f64 = 1e-9;
const CIRCULAR_HINT: &str = "A total angle of 360° spaces the copies evenly; a smaller angle \
                             runs from the body to the last copy.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Count,
    Spacing,
    Angle,
}

impl Rule {
    fn dimension(self) -> Dimension {
        match self {
            Self::Count => Dimension::NONE,
            Self::Spacing => Dimension::LENGTH,
            Self::Angle => Dimension::ANGLE,
        }
    }

    fn check(self, value: f64) -> Result<(), String> {
        match self {
            Self::Count if (value - value.round()).abs() > WHOLE_TOLERANCE || value < 1.0 => {
                Err("Enter a whole number of at least 1".to_owned())
            }
            Self::Count if value > f64::from(MAX_PATTERN_INSTANCES) => {
                Err(format!("Enter a count of at most {MAX_PATTERN_INSTANCES}"))
            }
            Self::Spacing if value <= 0.0 => {
                Err("Enter a spacing above zero; tick Reversed to go the other way".to_owned())
            }
            Self::Angle if value <= 0.0 || value > FULL_TURN => {
                Err("Enter an angle above 0° and up to 360°".to_owned())
            }
            Self::Count | Self::Spacing | Self::Angle => Ok(()),
        }
    }
}

struct DirectionCaptions {
    salt: &'static str,
    count: &'static str,
    spacing: &'static str,
    direction: &'static str,
}

const FIRST: DirectionCaptions = DirectionCaptions {
    salt: "",
    count: "Count",
    spacing: "Spacing",
    direction: "Direction",
};

const SECOND: DirectionCaptions = DirectionCaptions {
    salt: "second-",
    count: "Second count",
    spacing: "Second spacing",
    direction: "Second direction",
};

struct Panel<'a> {
    model: &'a Model,
    selection: &'a Selection,
    feature: FeatureId,
    pattern: &'a Pattern,
    actions: &'a mut Vec<Action>,
}

impl Panel<'_> {
    fn change(&self, kind: PatternKind) -> Result<Transaction, String> {
        pattern_tools::change(
            self.model,
            self.feature,
            Pattern {
                body: self.pattern.body,
                kind,
            },
        )
    }

    fn apply(&mut self, change: Result<Transaction, String>) {
        match change {
            Ok(transaction) => self.actions.push(Action::Apply(transaction)),
            Err(reason) => self.actions.push(Action::Inform(Notice::error(format!(
                "The pattern was not changed: {reason}"
            )))),
        }
    }

    fn shape_row(&mut self, ui: &mut Ui) {
        widgets::caption(ui, "Shape");
        let current = Shape::of(&self.pattern.kind);
        let mut chosen = None;
        let combo = ComboBox::from_id_salt(("pattern-shape", self.feature))
            .selected_text(current.title())
            .show_ui(ui, |ui| {
                for shape in Shape::ALL {
                    let selected = shape == current;
                    let change = (!selected).then(|| {
                        let reshaped = pattern_tools::reshaped(self.model, self.pattern, shape);
                        self.change(reshaped.kind)
                    });
                    let enabled = !matches!(change, Some(Err(_)));
                    let response =
                        ui.add_enabled(enabled, Button::selectable(selected, shape.title()));
                    let response = match &change {
                        Some(Err(reason)) => response.on_disabled_hover_text(reason),
                        Some(Ok(_)) | None => response.on_hover_text(shape.description()),
                    };
                    if response.clicked() {
                        chosen = change;
                    }
                }
            });
        widgets::tie_to_caption(ui, &combo.response);
        ui.end_row();
        if let Some(change) = chosen {
            self.apply(change);
        }
    }

    fn use_button(&mut self, ui: &mut Ui, hover: &str, reference: Reference) {
        let change = pattern_tools::selected_change(
            self.model,
            self.selection,
            self.feature,
            self.pattern,
            reference,
        );
        let button = widgets::small_button(ui, icons::USE_SELECTED, USE_SELECTED);
        let response = ui.add_enabled(change.is_ok(), button);
        match change {
            Ok(transaction) => {
                if response.on_hover_text(hover).clicked() {
                    self.actions.push(Action::Apply(transaction));
                }
            }
            Err(reason) => {
                response.on_disabled_hover_text(reason);
            }
        }
    }

    fn reference_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        axis: &AxisReference,
        hover: &str,
        reference: Reference,
    ) {
        widgets::caption(ui, caption);
        ui.horizontal_wrapped(|ui| {
            ui.label(capitalized(&describe_axis(self.model.document(), axis)));
            self.use_button(ui, hover, reference);
        });
        ui.end_row();
    }

    fn expression(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        expression: &Expression,
        rule: Rule,
        rebuild: impl Fn(Expression) -> PatternKind,
    ) {
        let model = self.model;
        let document = model.document();
        let parameters = model.parameters();
        let mut error = None;
        let mut committed = None;
        ui.horizontal(|ui| {
            let field = field::commit_field(
                ui,
                Id::new(("pattern-field", salt, self.feature)),
                &document.expression_text(expression),
                FIELD_WIDTH,
                false,
                |text| {
                    let parsed = field::parse_expression(
                        document,
                        parameters,
                        text,
                        Expected {
                            dimension: Some(rule.dimension()),
                            non_negative: false,
                        },
                        model.length_unit(),
                    )?;
                    let value = parameters
                        .evaluate_expression(&parsed)
                        .map_err(|error| field::sentence(&error.to_string()))?
                        .value;
                    rule.check(value)?;
                    self.change(rebuild(parsed))
                },
            );
            committed = field.committed;
            if field.error.is_none()
                && let Some(preview) =
                    field::value_preview(parameters, expression, model.length_unit())
            {
                ui.label(widgets::muted(preview, ui));
            }
            error = field.error;
        });
        if let Some(transaction) = committed {
            self.actions.push(Action::Apply(transaction));
        }
        ui.end_row();
        if let Some(error) = error {
            widgets::error_row(ui, &error);
        }
    }

    fn reversed_row(
        &mut self,
        ui: &mut Ui,
        caption: &str,
        reversed: bool,
        rebuild: impl Fn(bool) -> PatternKind,
    ) {
        widgets::caption(ui, caption);
        let mut flipped = reversed;
        if ui.checkbox(&mut flipped, "Reversed").changed() {
            let change = self.change(rebuild(flipped));
            self.apply(change);
        }
        ui.end_row();
    }

    fn direction_rows(
        &mut self,
        ui: &mut Ui,
        direction: &LinearDirection,
        captions: &DirectionCaptions,
        rebuild: &dyn Fn(LinearDirection) -> PatternKind,
    ) {
        widgets::caption(ui, captions.count);
        let salt = format!("{}count", captions.salt);
        self.expression(ui, &salt, &direction.count, Rule::Count, |count| {
            rebuild(LinearDirection {
                count,
                ..direction.clone()
            })
        });
        widgets::caption(ui, captions.spacing);
        let salt = format!("{}spacing", captions.salt);
        self.expression(ui, &salt, &direction.spacing, Rule::Spacing, |spacing| {
            rebuild(LinearDirection {
                spacing,
                ..direction.clone()
            })
        });
        self.reversed_row(ui, captions.direction, direction.reversed, |reversed| {
            rebuild(LinearDirection {
                reversed,
                ..direction.clone()
            })
        });
    }

    fn second_row(
        &mut self,
        ui: &mut Ui,
        first: &LinearDirection,
        second: Option<&LinearDirection>,
    ) {
        widgets::caption(ui, "Also along");
        ui.horizontal_wrapped(|ui| {
            match second {
                Some(second) => {
                    ui.label(capitalized(&describe_axis(
                        self.model.document(),
                        &second.axis,
                    )));
                }
                None => {
                    ui.label(widgets::muted("Nothing", ui));
                }
            }
            self.use_button(
                ui,
                "Also repeat the rows along the selected edge, round face or axis",
                Reference::Second,
            );
            if second.is_some()
                && widgets::icon_button(ui, icons::REMOVE, "Stop repeating in a second direction")
                    .clicked()
            {
                let change = self.change(PatternKind::Linear {
                    first: first.clone(),
                    second: None,
                });
                self.apply(change);
            }
        });
        ui.end_row();
    }

    fn linear_rows(
        &mut self,
        ui: &mut Ui,
        first: &LinearDirection,
        second: Option<&LinearDirection>,
    ) {
        self.reference_row(
            ui,
            "Along",
            &first.axis,
            "Repeat along the selected edge, round face or axis",
            Reference::First,
        );
        let kept = second.cloned();
        self.direction_rows(ui, first, &FIRST, &|first| PatternKind::Linear {
            first,
            second: kept.clone(),
        });
        self.second_row(ui, first, second);
        if let Some(second) = second {
            let first = first.clone();
            self.direction_rows(ui, second, &SECOND, &|second| PatternKind::Linear {
                first: first.clone(),
                second: Some(second),
            });
        }
    }

    fn circular_rows(&mut self, ui: &mut Ui, circular: &CircularPattern) {
        self.reference_row(
            ui,
            "Around",
            &circular.axis,
            "Turn about the selected axis, straight edge or round face",
            Reference::First,
        );
        widgets::caption(ui, "Count");
        self.expression(ui, "count", &circular.count, Rule::Count, |count| {
            PatternKind::Circular(CircularPattern {
                count,
                ..circular.clone()
            })
        });
        widgets::caption(ui, "Total angle");
        self.expression(ui, "angle", &circular.angle, Rule::Angle, |angle| {
            PatternKind::Circular(CircularPattern {
                angle,
                ..circular.clone()
            })
        });
        self.reversed_row(ui, "Direction", circular.reversed, |reversed| {
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
    selection: &Selection,
    actions: &mut Vec<Action>,
    feature: &Feature,
    pattern: &Pattern,
) {
    let id = feature.id();
    let mut panel = Panel {
        model,
        selection,
        feature: id,
        pattern,
        actions,
    };
    widgets::properties(ui, ("pattern-properties", id), |ui| {
        panel.shape_row(ui);
        match &pattern.kind {
            PatternKind::Linear { first, second } => panel.linear_rows(ui, first, second.as_ref()),
            PatternKind::Circular(circular) => panel.circular_rows(ui, circular),
        }
        widgets::caption(ui, "Body");
        ui.label(
            model
                .document()
                .feature(pattern.body)
                .map_or("a missing body", |body| body.name.as_str()),
        );
        ui.end_row();
    });
    if !pattern.kind.is_linear() {
        ui.label(widgets::muted(CIRCULAR_HINT, ui));
    }
}
