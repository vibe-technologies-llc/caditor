use caditor_document::{
    Datum, DisplacedFeature, Document, Evaluation, FeatureId, FeatureResult, ModelScale,
    ScaleSummary, ScaledValues, Transaction, principal_words,
};
use caditor_expression::format_number;
use caditor_geometry::Point3;
use egui::{ComboBox, Id, Key, TextEdit, Ui};

use crate::{
    appearance::SPACE_S,
    model::Model,
    widgets::{self, DialogWidth, Tone},
};

pub const TITLE: &str = "Scale model";
pub const FACTOR: &str = "Factor";
pub const CENTRE: &str = "Centre";
pub const VALUES: &str = "Scale";
pub const ORIGIN: &str = "Origin";
pub const PLAIN_VALUES: &str = "Typed values";
pub const WITH_PARAMETERS: &str = "Values and parameters";
pub const SCALE_LABEL: &str = "Scale model";
pub const CANCEL_LABEL: &str = "Cancel";
const EXPLANATION: &str = "Resize every sketch, body and datum by a factor, such as 25.4 for a \
                           part drawn in inches, as one change that Undo takes back. Names and \
                           references stay, and angles, counts and plain numbers never change.";
const PLAIN_HOVER: &str = "Scale the lengths typed into sketches and features, named values \
                           included; a value using your parameters is left to follow them";
const PARAMETERS_HOVER: &str =
    "Also scale every length parameter, so the values using them grow with the model";
const DIALOG_ID: &str = "scale-model";
pub const IN_SKETCH: &str = "Finish the sketch first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Centre {
    Origin,
    Datum(FeatureId),
}

#[derive(Debug, Clone, PartialEq)]
struct Displacement {
    revision: u64,
    centre: Point3,
    moved: Vec<DisplacedFeature>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaleDraft {
    pub factor: String,
    pub centre: Centre,
    pub values: ScaledValues,
    problem: Option<String>,
    focus_pending: bool,
    displaced: Option<Displacement>,
}

impl Default for ScaleDraft {
    fn default() -> Self {
        Self {
            factor: String::new(),
            centre: Centre::Origin,
            values: ScaledValues::Plain,
            problem: None,
            focus_pending: true,
            displaced: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Close,
    Scale(Transaction, String),
}

pub fn factor_field_id() -> Id {
    Id::new((DIALOG_ID, "factor"))
}

fn centres(document: &Document) -> Vec<FeatureId> {
    document
        .features()
        .filter(|feature| {
            feature
                .kind
                .datum()
                .is_some_and(|datum| datum.is_point() || datum.is_frame())
        })
        .map(|feature| feature.id())
        .collect()
}

fn centre_point(evaluation: &Evaluation, feature: FeatureId) -> Option<Point3> {
    let datum = evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::datum)?;
    datum
        .point()
        .or_else(|| datum.frame().map(|frame| frame.origin()))
}

fn centre_name(document: &Document, centre: Centre) -> String {
    match centre {
        Centre::Origin => ORIGIN.to_owned(),
        Centre::Datum(feature) => {
            let name = document
                .feature(feature)
                .map_or("A deleted datum", |feature| feature.name.as_str());
            match document
                .feature(feature)
                .and_then(|feature| feature.kind.datum())
            {
                Some(Datum::Frame(_)) => format!("Origin of {name}"),
                _ => name.to_owned(),
            }
        }
    }
}

pub fn scaling(model: &Model, draft: &ScaleDraft) -> Result<(Transaction, String), String> {
    let document = model.document();
    let text = draft.factor.trim();
    if text.is_empty() {
        return Err("Enter a factor, such as 2 to double the size or 25.4 for inches.".to_owned());
    }
    let expression = document
        .parse(text)
        .map_err(|error| format!("The factor cannot be read: {error}."))?;
    let factor = model
        .parameters()
        .evaluate_expression(&expression)
        .map_err(|error| format!("The factor cannot be evaluated: {error}."))?;
    if !factor.dimension.is_plain() {
        return Err(format!(
            "The factor gives {}, but a plain number is needed, such as 2 or 25.4.",
            factor.dimension
        ));
    }
    let centre = match draft.centre {
        Centre::Origin => Point3::ZERO,
        Centre::Datum(feature) => centre_point(model.evaluation(), feature).ok_or_else(|| {
            format!(
                "{} has no place to scale about until it computes.",
                centre_name(document, draft.centre)
            )
        })?,
    };
    let scaled = document
        .scaled(&ModelScale {
            factor: factor.value,
            centre,
            values: draft.values,
        })
        .map_err(|error| format!("{error} {}", error.remedy()))?;
    let summary = summary(factor.value, &scaled.summary);
    Ok((scaled.transaction, summary))
}

fn listed(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [only] => only.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

pub fn moving_words(moved: &[DisplacedFeature]) -> Option<String> {
    if moved.is_empty() {
        return None;
    }
    let users: Vec<String> = moved
        .iter()
        .map(|feature| {
            let geometry: Vec<String> = feature
                .geometry
                .iter()
                .copied()
                .map(principal_words)
                .collect();
            format!("{} ({})", feature.name, listed(&geometry))
        })
        .collect();
    Some(format!(
        "The principal planes, axes and origin stay where they are, so scaling about this centre \
         moves {} onto new datums standing where that geometry lands.",
        listed(&users)
    ))
}

fn displaced<'a>(model: &Model, draft: &'a mut ScaleDraft) -> &'a [DisplacedFeature] {
    let centre = match draft.centre {
        Centre::Origin => Some(Point3::ZERO),
        Centre::Datum(feature) => centre_point(model.evaluation(), feature),
    };
    let Some(centre) = centre else {
        return &[];
    };
    let revision = model.revision();
    let current = draft
        .displaced
        .as_ref()
        .is_some_and(|known| known.revision == revision && known.centre == centre);
    if !current {
        draft.displaced = Some(Displacement {
            revision,
            centre,
            moved: model.document().displaced_by_scale(centre),
        });
    }
    draft
        .displaced
        .as_ref()
        .map_or(&[], |known| known.moved.as_slice())
}

pub fn summary(factor: f64, summary: &ScaleSummary) -> String {
    let counted = |count: usize, one: &str, many: &str| match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    };
    let mut parts = vec![format!(
        "Scaled the model by {}: {}, {} and {} changed.",
        format_number(factor),
        counted(summary.sketches, "sketch", "sketches"),
        counted(summary.features, "feature", "features"),
        counted(summary.parameters, "parameter", "parameters"),
    )];
    let said = |names: &[String], one: &str, many: &str| match names {
        [] => None,
        [name] => Some(format!("{name} {one}")),
        names => Some(format!("{} {many}", names.join(", "))),
    };
    parts.extend(said(
        &summary.kept,
        "uses parameters, so its values were left to follow them.",
        "use parameters, so their values were left to follow them.",
    ));
    parts.extend(said(
        &summary.cleared_standards,
        "no longer matches a standard size, so the size was cleared.",
        "no longer match a standard size, so the sizes were cleared.",
    ));
    if !summary.moved.is_empty() {
        let moved: Vec<String> = summary
            .moved
            .iter()
            .map(|feature| feature.name.clone())
            .collect();
        parts.push(format!(
            "{} now {} {}, standing where the principal geometry landed.",
            listed(&moved),
            if moved.len() == 1 { "uses" } else { "use" },
            listed(&summary.datums)
        ));
    }
    parts.extend(said(
        &summary.threads,
        "keeps its thread size; check that it still fits.",
        "keep their thread sizes; check that they still fit.",
    ));
    parts.join(" ")
}

pub fn dialog(ctx: &egui::Context, model: &Model, draft: &mut ScaleDraft) -> Option<Outcome> {
    let response = widgets::dialog(ctx, DIALOG_ID, TITLE, DialogWidth::Medium, |ui| {
        ui.label(widgets::muted(EXPLANATION, ui));
        ui.add_space(SPACE_S);
        let entered = fields(ui, model, draft);
        if let Some(moving) = moving_words(displaced(model, draft)) {
            ui.add_space(SPACE_S);
            widgets::callout(ui, Tone::Info, |ui| ui.label(moving));
        }
        if let Some(problem) = &draft.problem {
            ui.add_space(SPACE_S);
            widgets::callout(ui, Tone::Error, |ui| ui.label(problem));
        }
        let chosen = widgets::footer(ui, |ui| {
            if ui.add(widgets::primary_button(ui, SCALE_LABEL)).clicked() {
                return Some(true);
            }
            ui.add(widgets::button(CANCEL_LABEL))
                .clicked()
                .then_some(false)
        });
        if entered { Some(true) } else { chosen }
    });
    let closed = response.should_close().then_some(false);
    match response.inner.or(closed)? {
        false => Some(Outcome::Close),
        true => match scaling(model, draft) {
            Ok((transaction, summary)) => Some(Outcome::Scale(transaction, summary)),
            Err(problem) => {
                draft.problem = Some(problem);
                None
            }
        },
    }
}

fn fields(ui: &mut Ui, model: &Model, draft: &mut ScaleDraft) -> bool {
    let document = model.document();
    let focus = std::mem::take(&mut draft.focus_pending);
    widgets::properties(ui, DIALOG_ID, |ui| {
        let entered = widgets::property(ui, FACTOR, |ui| {
            let field = widgets::text_field(ui, |ui| {
                ui.add(
                    TextEdit::singleline(&mut draft.factor)
                        .id(factor_field_id())
                        .hint_text("2")
                        .desired_width(f32::INFINITY),
                )
            });
            widgets::tie_to_caption(ui, &field);
            if focus {
                field.request_focus();
            }
            if field.changed() {
                draft.problem = None;
            }
            field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
        });
        widgets::property(ui, CENTRE, |ui| {
            let combo = ComboBox::from_id_salt((DIALOG_ID, "centre"))
                .selected_text(centre_name(document, draft.centre))
                .show_ui(ui, |ui| {
                    let choices = std::iter::once(Centre::Origin)
                        .chain(centres(document).into_iter().map(Centre::Datum));
                    for centre in choices {
                        let chosen = draft.centre == centre;
                        let label = centre_name(document, centre);
                        if widgets::menu_option(ui, chosen, &label).clicked() {
                            draft.centre = centre;
                            draft.problem = None;
                        }
                    }
                });
            widgets::tie_to_caption(ui, &combo.response);
        });
        widgets::property(ui, VALUES, |ui| {
            let choices = [
                (PLAIN_VALUES, PLAIN_HOVER),
                (WITH_PARAMETERS, PARAMETERS_HOVER),
            ];
            let current = match draft.values {
                ScaledValues::Plain => 0,
                ScaledValues::AndParameters => 1,
            };
            if let Some(chosen) = widgets::segmented(ui, &choices, current) {
                draft.values = if chosen == 0 {
                    ScaledValues::Plain
                } else {
                    ScaledValues::AndParameters
                };
                draft.problem = None;
            }
        });
        entered
    })
}
