use caditor_document::{Evaluation, FeatureId, FeatureState};
use caditor_sketch::SketchSolution;
use egui::{Label, RichText, Sense, Ui};

use crate::{appearance, feature_tree::count, panels::Focus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchStatus {
    NotSolved,
    FullyConstrained,
    Free(usize),
    Conflicting,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SketchSummary {
    pub status: SketchStatus,
    pub redundant: usize,
    pub problem: Option<Focus>,
}

impl SketchSummary {
    pub fn of(evaluation: &Evaluation, feature: FeatureId) -> Self {
        let status = evaluation.feature(feature);
        match status.map(|status| &status.state) {
            Some(FeatureState::Failed(error)) => Self {
                status: if error.constraints.is_empty() {
                    SketchStatus::Failed
                } else {
                    SketchStatus::Conflicting
                },
                redundant: 0,
                problem: Some(error.fix.map_or(Focus::Feature(feature), Focus::from)),
            },
            Some(FeatureState::UpToDate) => {
                let solution = up_to_date_solution(evaluation, feature);
                Self {
                    status: solution.map_or(SketchStatus::NotSolved, |solution| {
                        match solution.degrees_of_freedom() {
                            0 => SketchStatus::FullyConstrained,
                            free => SketchStatus::Free(free),
                        }
                    }),
                    redundant: solution.map_or(0, |solution| solution.redundancies().len()),
                    problem: None,
                }
            }
            Some(FeatureState::Outdated) | None => Self {
                status: SketchStatus::NotSolved,
                redundant: 0,
                problem: None,
            },
        }
    }

    pub fn status_text(&self) -> String {
        match self.status {
            SketchStatus::NotSolved => "Not solved yet".to_owned(),
            SketchStatus::FullyConstrained => "Fully constrained".to_owned(),
            SketchStatus::Free(free) => format!(
                "{} left",
                count(free, "degree of freedom", "degrees of freedom")
            ),
            SketchStatus::Conflicting => "Conflicting constraints".to_owned(),
            SketchStatus::Failed => "The sketch has an error".to_owned(),
        }
    }

    pub fn redundant_text(&self) -> Option<String> {
        (self.redundant > 0).then(|| {
            count(
                self.redundant,
                "redundant constraint",
                "redundant constraints",
            )
        })
    }
}

pub fn up_to_date_solution(evaluation: &Evaluation, feature: FeatureId) -> Option<&SketchSolution> {
    let status = evaluation.feature(feature)?;
    if status.state != FeatureState::UpToDate {
        return None;
    }
    status
        .result
        .as_deref()?
        .sketch()
        .map(|result| &result.solution)
}

pub fn show(ui: &mut Ui, summary: &SketchSummary) -> Option<Focus> {
    let (error, warning) = (ui.visuals().error_fg_color, ui.visuals().warn_fg_color);
    let text = RichText::new(summary.status_text());
    let text = match summary.status {
        SketchStatus::NotSolved => text.weak(),
        SketchStatus::FullyConstrained => text.color(appearance::success_color(ui.visuals())),
        SketchStatus::Free(_) => text,
        SketchStatus::Conflicting | SketchStatus::Failed => text.color(error),
    };
    let mut focus = None;
    match summary.problem {
        Some(problem) => {
            let response = ui
                .add(Label::new(text).selectable(false).sense(Sense::click()))
                .on_hover_text("Show the problem");
            if response.clicked() {
                focus = Some(problem);
            }
        }
        None => {
            ui.label(text);
        }
    }
    if let Some(redundant) = summary.redundant_text() {
        ui.colored_label(warning, redundant);
    }
    focus
}
