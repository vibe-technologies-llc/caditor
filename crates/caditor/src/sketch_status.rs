use caditor_document::{Evaluation, FeatureId, FeatureState, SketchResult};
use caditor_sketch::{ConstraintId, EntityId, SketchSolution};
use egui::Ui;

use crate::{
    commands::Command,
    feature_tree::count,
    icons,
    panels::Focus,
    widgets::{self, PillRole, Tone},
};

pub const BEYOND_HELP: &str = "Points held on a line or arc that lie past its drawn ends, marked in \
                               the view: a point on a line or arc is held to its whole line or \
                               circle. Extend the curve, or move the point, if that is not meant.";
pub const OPEN_ENDS_HELP: &str = "Curve ends joined to nothing, marked in the view. Join them to \
                                  close the outline before extruding or revolving it.";
pub const REDUNDANT_HELP: &str = "Constraints that repeat what others already say. They do no \
                                  harm, but are best deleted.";
pub const SHOW_PROBLEM: &str = "Show the problem";
pub const SELECT_FREE: &str = "Select what is still free";
pub const SELECT_OPEN_ENDS: &str = "Select the open ends and bring them into view";
pub const SELECT_REDUNDANT: &str = "Select the redundant constraints, ready to delete";
pub const NOT_SOLVED: &str = "Open ends and redundant constraints are known once the sketch has solved; fix its problems or wait for it";
pub const NO_OPEN_ENDS: &str = "The sketch has no open ends: every curve end is joined";
pub const NOTHING_REDUNDANT: &str = "No constraint in the sketch repeats what others say";
const WIDEST_FREEDOM: usize = 88;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchStatus {
    NotSolved,
    FullyConstrained,
    Free(usize),
    Conflicting,
    Failed,
    Suppressed,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SketchSummary {
    pub status: SketchStatus,
    pub redundant: usize,
    pub open_ends: usize,
    pub beyond: usize,
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
                open_ends: 0,
                beyond: 0,
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
                    open_ends: up_to_date_result(evaluation, feature)
                        .map_or(0, |result| result.open_ends.len()),
                    beyond: up_to_date_result(evaluation, feature)
                        .map_or(0, |result| result.beyond.len()),
                    problem: None,
                }
            }
            Some(FeatureState::Outdated) | None => Self {
                status: SketchStatus::NotSolved,
                redundant: 0,
                open_ends: 0,
                beyond: 0,
                problem: None,
            },
            Some(FeatureState::Suppressed) => Self {
                status: SketchStatus::Suppressed,
                redundant: 0,
                open_ends: 0,
                beyond: 0,
                problem: None,
            },
            Some(FeatureState::RolledBack) => Self {
                status: SketchStatus::RolledBack,
                redundant: 0,
                open_ends: 0,
                beyond: 0,
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
            SketchStatus::Suppressed => "Suppressed".to_owned(),
            SketchStatus::RolledBack => "Below the rollback bar".to_owned(),
        }
    }

    pub fn open_ends_text(&self) -> Option<String> {
        (self.open_ends > 0).then(|| count(self.open_ends, "open end", "open ends"))
    }

    pub fn beyond_text(&self) -> Option<String> {
        (self.beyond > 0).then(|| {
            count(
                self.beyond,
                "point beyond its curve",
                "points beyond their curves",
            )
        })
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
    up_to_date_result(evaluation, feature).map(|result| &result.solution)
}

pub fn up_to_date_result(evaluation: &Evaluation, feature: FeatureId) -> Option<&SketchResult> {
    let status = evaluation.feature(feature)?;
    if status.state != FeatureState::UpToDate {
        return None;
    }
    status.result.as_deref()?.sketch()
}

pub fn open_ends(
    evaluation: &Evaluation,
    feature: FeatureId,
) -> Result<Vec<EntityId>, &'static str> {
    let result = up_to_date_result(evaluation, feature).ok_or(NOT_SOLVED)?;
    if result.open_ends.is_empty() {
        Err(NO_OPEN_ENDS)
    } else {
        Ok(result.open_ends.clone())
    }
}

pub fn redundant_constraints(
    evaluation: &Evaluation,
    feature: FeatureId,
) -> Result<Vec<ConstraintId>, &'static str> {
    let solution = up_to_date_solution(evaluation, feature).ok_or(NOT_SOLVED)?;
    let redundant: Vec<ConstraintId> = solution
        .redundancies()
        .iter()
        .map(|redundancy| redundancy.constraint)
        .collect();
    if redundant.is_empty() {
        Err(NOTHING_REDUNDANT)
    } else {
        Ok(redundant)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRequest {
    ShowProblem(Focus),
    Run(Command),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pills {
    Full,
    Counts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Act {
    request: StatusRequest,
    does: &'static str,
}

impl SketchSummary {
    fn tone(&self) -> Tone {
        match self.status {
            SketchStatus::NotSolved | SketchStatus::Suppressed | SketchStatus::RolledBack => {
                Tone::Neutral
            }
            SketchStatus::FullyConstrained => Tone::Success,
            SketchStatus::Free(_) => Tone::Info,
            SketchStatus::Conflicting | SketchStatus::Failed => Tone::Error,
        }
    }

    fn status_act(&self, selects: bool) -> Option<Act> {
        match (self.problem, self.status) {
            (Some(problem), _) => Some(Act {
                request: StatusRequest::ShowProblem(problem),
                does: SHOW_PROBLEM,
            }),
            (None, SketchStatus::Free(_)) if selects => Some(Act {
                request: StatusRequest::Run(Command::SelectFree),
                does: SELECT_FREE,
            }),
            _ => None,
        }
    }

    fn counts(&self, selects: bool) -> Vec<Count> {
        let act = |command: Command, does: &'static str| {
            selects.then_some(Act {
                request: StatusRequest::Run(command),
                does,
            })
        };
        [
            self.redundant_text().map(|words| Count {
                tone: Tone::Warning,
                glyph: icons::REDUNDANT,
                amount: self.redundant,
                words,
                help: REDUNDANT_HELP,
                act: act(Command::SelectRedundant, SELECT_REDUNDANT),
            }),
            self.open_ends_text().map(|words| Count {
                tone: Tone::Warning,
                glyph: icons::OPEN_ENDS,
                amount: self.open_ends,
                words,
                help: OPEN_ENDS_HELP,
                act: act(Command::SelectOpenEnds, SELECT_OPEN_ENDS),
            }),
            self.beyond_text().map(|words| Count {
                tone: Tone::Info,
                glyph: icons::BEYOND,
                amount: self.beyond,
                words,
                help: BEYOND_HELP,
                act: None,
            }),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Count {
    tone: Tone,
    glyph: &'static str,
    amount: usize,
    words: String,
    help: &'static str,
    act: Option<Act>,
}

impl Count {
    fn show(&self, ui: &mut Ui, pills: Pills) -> Option<StatusRequest> {
        let name = match self.act {
            Some(act) => format!("{}: {}", self.words, act.does.to_lowercase()),
            None => self.words.clone(),
        };
        let role = match (self.act, pills) {
            (Some(_), _) => PillRole::Button(&name),
            (None, Pills::Counts) => PillRole::Named(&name),
            (None, Pills::Full) => PillRole::Shown,
        };
        let response = match pills {
            Pills::Full => {
                widgets::glyph_pill(ui, self.tone, self.tone.icon(), self.words.clone(), role)
            }
            Pills::Counts => widgets::count_pill(ui, self.tone, self.glyph, self.amount, role),
        };
        let hover = match (pills, self.act) {
            (Pills::Full, Some(act)) => format!("{}\n{}.", self.help, act.does),
            (Pills::Full, None) => self.help.to_owned(),
            (Pills::Counts, Some(act)) => format!("{}. {}\n{}.", self.words, self.help, act.does),
            (Pills::Counts, None) => format!("{}. {}", self.words, self.help),
        };
        let response = response.on_hover_text(hover);
        self.act
            .filter(|_| response.clicked())
            .map(|act| act.request)
    }
}

pub fn show(ui: &mut Ui, summary: &SketchSummary, selects: bool) -> Option<StatusRequest> {
    let status = status_pill(ui, summary, selects);
    summary
        .counts(selects)
        .iter()
        .filter_map(|count| count.show(ui, Pills::Full))
        .fold(status, |chosen, request| chosen.or(Some(request)))
}

pub fn status_pill(ui: &mut Ui, summary: &SketchSummary, selects: bool) -> Option<StatusRequest> {
    let tone = summary.tone();
    let text = summary.status_text();
    let Some(act) = summary.status_act(selects) else {
        widgets::status_pill(ui, tone, text);
        return None;
    };
    let name = format!("{text}: {}", act.does.to_lowercase());
    widgets::glyph_pill(ui, tone, tone.icon(), text, PillRole::Button(&name))
        .on_hover_text(act.does)
        .clicked()
        .then_some(act.request)
}

pub fn counts_from_the_right(
    ui: &mut Ui,
    summary: &SketchSummary,
    selects: bool,
) -> Option<StatusRequest> {
    summary
        .counts(selects)
        .iter()
        .rev()
        .filter_map(|count| count.show(ui, Pills::Counts))
        .fold(None, |chosen, request| chosen.or(Some(request)))
}

pub fn widest_status(ui: &Ui) -> f32 {
    [
        SketchStatus::Free(WIDEST_FREEDOM),
        SketchStatus::Conflicting,
        SketchStatus::Failed,
        SketchStatus::RolledBack,
    ]
    .into_iter()
    .map(|status| {
        let summary = SketchSummary {
            status,
            redundant: 0,
            open_ends: 0,
            beyond: 0,
            problem: None,
        };
        widgets::status_pill_width(ui, summary.tone().icon(), &summary.status_text())
    })
    .fold(0.0, f32::max)
}
