use caditor_document::{FeatureId, FixTarget, TreeRow};
use caditor_expression::ParameterId;
use caditor_geometry::Point3;
use caditor_sketch::ConstraintId;
use egui::Id;

use crate::{
    appearance::{SPACE_L, SPACE_S},
    bodies_tree,
    commands::CommandFrame,
    editing::SketchEditing,
    feature_tree, icons,
    layout::{self, MIN_SIDE_WIDTH, PanelLayout},
    model::{Action, Model},
    parameter_table::{self, NoteDraft, ParameterUses},
    reference_rows::RowCache,
    selection::{Pickable, Selection},
    sketch_toolbar::ConstraintOffers,
    widgets::{self, SectionAction, SectionHeader},
};

pub const FEATURES_TITLE: &str = "Features";
pub const PARAMETERS_TITLE: &str = "Parameters";
pub const FEATURES_EXPLANATION: &str =
    "The steps that build the model, computed from top to bottom. Drag one to reorder it.";
pub const PARAMETERS_EXPLANATION: &str = "Named values that any size or dimension can use by name, such as width * 2. Right-click \
     one to move it, note what it is for or delete it.";
const FEATURES_SECTION: &str = "features";
const PARAMETERS_SECTION: &str = "parameters";
const FOCUS_ATTEMPT_FRAMES: u8 = 30;
pub const REVEAL_FRAMES: u8 = 20;
const CAPPED_SLACK: f32 = 1.0;

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
    TreeFilter,
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
            Self::TreeFilter => Id::new("feature-tree-filter"),
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
            FixTarget::Feature(id) | FixTarget::Unsuppress(id) => Self::Feature(id),
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
    pub renaming_group: Option<Renaming>,
    pub opened_for_editing: Option<FeatureId>,
    pub finished_editing: Option<FeatureId>,
    pub selected: Option<FeatureId>,
    also_selected: Vec<FeatureId>,
    pub dragging: Option<TreeRow>,
    pub deleting: Option<Vec<FeatureId>>,
    pub noting: Option<NoteDraft>,
    pub painting: Option<Painting>,
    pub parameter: Option<ParameterId>,
    pub hovered_in_tree: Option<Pickable>,
    pub chosen_in_tree: Option<Pickable>,
    pub selected_in_tree: Option<Vec<Pickable>>,
    pub shown_place: Option<Point3>,
    pub reference_rows: RowCache,
    pub constraint_offers: ConstraintOffers,
    pub parameter_uses: ParameterUses,
    pub tree_filter: String,
    pub plain_row_height: Option<f32>,
    pub plain_constraint_height: Option<f32>,
    revealing: Option<PendingReveal>,
    layout: PanelLayout,
    layout_restored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingReveal {
    feature: FeatureId,
    frames_left: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Painting {
    pub body: FeatureId,
    pub focus_pending: bool,
    pub naming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renaming {
    pub feature: FeatureId,
    pub focus_pending: bool,
}

impl PanelState {
    pub fn with_layout(layout: PanelLayout) -> Self {
        Self {
            layout,
            ..Self::default()
        }
    }

    pub fn layout(&self) -> PanelLayout {
        self.layout
    }

    pub fn forget_document(&mut self) {
        *self = Self {
            layout: self.layout,
            layout_restored: self.layout_restored,
            ..Self::default()
        };
    }

    fn restore_layout(&mut self, ctx: &egui::Context) {
        if std::mem::replace(&mut self.layout_restored, true) {
            return;
        }
        widgets::set_section_open(ctx, FEATURES_SECTION, self.layout.features_open);
        widgets::set_section_open(ctx, PARAMETERS_SECTION, self.layout.parameters_open);
    }

    fn observe_layout(&mut self, ctx: &egui::Context, width: f32, capped: bool) {
        self.layout = PanelLayout {
            side_width: if capped {
                self.layout.side_width
            } else {
                layout::side_width(width)
            },
            features_open: widgets::is_section_open(ctx, FEATURES_SECTION),
            parameters_open: widgets::is_section_open(ctx, PARAMETERS_SECTION),
        };
    }

    pub fn chosen(&self) -> Vec<FeatureId> {
        let Some(primary) = self.selected else {
            return Vec::new();
        };
        std::iter::once(primary)
            .chain(
                self.also_selected
                    .iter()
                    .copied()
                    .filter(|id| *id != primary),
            )
            .collect()
    }

    pub fn choose_only(&mut self, feature: FeatureId) {
        self.selected = Some(feature);
        self.also_selected.clear();
    }

    pub fn choose_all(&mut self, features: &[FeatureId]) {
        self.selected = features.first().copied();
        self.also_selected = features.iter().skip(1).copied().collect();
    }

    pub fn toggle_chosen(&mut self, feature: FeatureId) {
        let chosen = self.chosen();
        if !chosen.contains(&feature) {
            if self.selected.is_none() {
                self.also_selected.clear();
            }
            self.selected.get_or_insert(feature);
            self.also_selected.push(feature);
            return;
        }
        let remaining: Vec<FeatureId> = chosen.into_iter().filter(|id| *id != feature).collect();
        self.selected = remaining.first().copied();
        self.also_selected = remaining;
    }

    pub fn choose_range(&mut self, features: Vec<FeatureId>) {
        self.also_selected = features;
    }

    pub fn request_focus(&mut self, target: Focus) {
        self.focus = Some(PendingFocus {
            target,
            frames_left: FOCUS_ATTEMPT_FRAMES,
        });
    }

    pub fn reveal(&mut self, feature: FeatureId) {
        self.revealing = Some(PendingReveal {
            feature,
            frames_left: REVEAL_FRAMES,
        });
    }

    pub fn revealing(&self, feature: FeatureId) -> bool {
        self.revealing
            .is_some_and(|pending| pending.feature == feature)
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

    fn wants_features(&self) -> bool {
        self.renaming.is_some()
            || self.revealing.is_some()
            || self.painting.is_some_and(|painting| painting.focus_pending)
            || matches!(
                self.focus.map(|pending| pending.target),
                Some(
                    Focus::Feature(_)
                        | Focus::Dimension { .. }
                        | Focus::Constraint { .. }
                        | Focus::TreeFilter
                )
            )
    }

    fn wants_parameters(&self) -> bool {
        matches!(
            self.focus.map(|pending| pending.target),
            Some(Focus::ParameterName(_) | Focus::ParameterValue(_))
        )
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
        self.reference_rows.begin_frame();
        self.revealing = self.revealing.and_then(|pending| {
            pending
                .frames_left
                .checked_sub(1)
                .map(|frames_left| PendingReveal {
                    frames_left,
                    ..pending
                })
        });
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
    room: f32,
    actions: &mut Vec<Action>,
) {
    state.begin_frame();
    let pointer_held =
        ui.input(|input| input.pointer.primary_down() || input.pointer.primary_released());
    if !pointer_held {
        state.dragging = None;
    }
    state.restore_layout(ui.ctx());
    let widths = layout::panel_widths(room, MIN_SIDE_WIDTH);
    let panel = egui::Panel::left("model")
        .resizable(true)
        .default_size(state.layout.side_width)
        .size_range(widths)
        .show(ui, |ui| {
            if state.wants_features() {
                widgets::reveal_section(ui.ctx(), FEATURES_SECTION);
            }
            if state.wants_parameters() {
                widgets::reveal_section(ui.ctx(), PARAMETERS_SECTION);
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.add_space(SPACE_S);
                    let features = model.document().features().len();
                    let header = SectionHeader {
                        title: FEATURES_TITLE,
                        count: Some(features),
                        action: None,
                        explanation: Some(FEATURES_EXPLANATION),
                    };
                    widgets::panel_section(ui, FEATURES_SECTION, header, |ui| {
                        feature_tree::show(ui, model, selection, editing, state, actions);
                    });
                    ui.add_space(SPACE_L);
                    let parameters = model.document().parameters().len();
                    let header = SectionHeader {
                        title: PARAMETERS_TITLE,
                        count: Some(parameters),
                        action: Some(SectionAction {
                            glyph: icons::ADD,
                            hover: parameter_table::ADD_LABEL,
                        }),
                        explanation: Some(PARAMETERS_EXPLANATION),
                    };
                    let adding = widgets::panel_section(ui, PARAMETERS_SECTION, header, |ui| {
                        parameter_table::show(ui, model, state, actions);
                    });
                    if adding {
                        parameter_table::add(model, state, actions);
                    }
                });
        });
    let width = panel.response.rect.width();
    state.observe_layout(ui.ctx(), width, width >= widths.max - CAPPED_SLACK);
}

pub fn commands(
    context: &feature_tree::CommandContext<'_>,
    state: &mut PanelState,
    commands: &mut CommandFrame<'_>,
    actions: &mut Vec<Action>,
) {
    feature_tree::commands(context, state, commands, actions);
    bodies_tree::commands(context, state, commands, actions);
    parameter_table::commands(context.model, state, commands, actions);
}
