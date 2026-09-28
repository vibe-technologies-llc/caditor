use caditor_document::{FeatureId, FixTarget};
use caditor_expression::ParameterId;
use caditor_sketch::ConstraintId;
use egui::Id;

use crate::{
    editing::SketchEditing,
    feature_tree,
    model::{Action, Model},
    parameter_table,
    selection::Selection,
};

const SIDE_PANEL_WIDTH: f32 = 300.0;
const FOCUS_ATTEMPT_FRAMES: u8 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    ParameterName(ParameterId),
    ParameterValue(ParameterId),
    Dimension {
        feature: FeatureId,
        constraint: ConstraintId,
    },
    Feature(FeatureId),
    Constraint {
        feature: FeatureId,
        constraint: ConstraintId,
    },
}

impl Focus {
    pub fn field_id(self) -> Id {
        match self {
            Self::ParameterName(id) => Id::new(("parameter-name", id)),
            Self::ParameterValue(id) => Id::new(("parameter-expression", id)),
            Self::Dimension {
                feature,
                constraint,
            } => Id::new(("dimension", feature, constraint)),
            Self::Feature(id) => Id::new(("feature", id)),
            Self::Constraint {
                feature,
                constraint,
            } => Id::new(("constraint", feature, constraint)),
        }
    }
}

impl From<FixTarget> for Focus {
    fn from(target: FixTarget) -> Self {
        match target {
            FixTarget::Parameter(id) => Self::ParameterValue(id),
            FixTarget::Dimension {
                feature,
                constraint,
            } => Self::Dimension {
                feature,
                constraint,
            },
            FixTarget::Feature(id) => Self::Feature(id),
            FixTarget::Constraint {
                feature,
                constraint,
            } => Self::Constraint {
                feature,
                constraint,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingFocus {
    target: Focus,
    frames_left: u8,
}

#[derive(Debug, Clone, Default)]
pub struct PanelState {
    focus: Option<PendingFocus>,
    pub renaming: Option<Renaming>,
    pub opened_for_editing: Option<FeatureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renaming {
    pub feature: FeatureId,
    pub focus_pending: bool,
}

impl PanelState {
    pub fn request_focus(&mut self, target: Focus) {
        self.focus = Some(PendingFocus {
            target,
            frames_left: FOCUS_ATTEMPT_FRAMES,
        });
    }

    pub fn wants_focus(&self, focus: Focus) -> bool {
        self.focus.is_some_and(|pending| pending.target == focus)
    }

    pub fn take_focus(&mut self, focus: Focus) -> bool {
        let matches = self.wants_focus(focus);
        if matches {
            self.focus = None;
        }
        matches
    }

    pub fn focus_reached(&mut self, focus: Focus, has_focus: bool) {
        if has_focus && self.wants_focus(focus) {
            self.focus = None;
        }
    }

    pub fn take_dimension_focus(&mut self, feature: FeatureId) -> Option<ConstraintId> {
        let Some(Focus::Dimension {
            feature: target,
            constraint,
        }) = self.focus.map(|pending| pending.target)
        else {
            return None;
        };
        (target == feature).then(|| {
            self.focus = None;
            constraint
        })
    }

    pub fn focus_inside(&self, feature: FeatureId) -> bool {
        matches!(
            self.focus.map(|pending| pending.target),
            Some(
                Focus::Dimension { feature: target, .. } | Focus::Constraint { feature: target, .. }
            ) if target == feature
        )
    }

    fn begin_frame(&mut self) {
        self.focus = self.focus.and_then(|pending| {
            pending
                .frames_left
                .checked_sub(1)
                .map(|frames_left| PendingFocus {
                    target: pending.target,
                    frames_left,
                })
        });
    }
}

pub fn show(
    ui: &mut egui::Ui,
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
    state: &mut PanelState,
    actions: &mut Vec<Action>,
) {
    state.begin_frame();
    egui::Panel::left("model")
        .resizable(true)
        .default_size(SIDE_PANEL_WIDTH)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                feature_tree::show(ui, model, editing, state, actions);

                ui.separator();
                parameter_table::show(ui, model, state, actions);

                ui.separator();
                ui.heading("Selection");
                if selection.is_empty() {
                    ui.weak("Nothing selected. Click geometry in the view to select it.");
                }
                for pickable in selection.iter() {
                    ui.label(pickable.describe(model.document(), model.evaluation()));
                }
            });
        });
}
