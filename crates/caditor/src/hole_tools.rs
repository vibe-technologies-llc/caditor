use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Hole, HoleDepth, HoleStyle, Transaction, hole_centres,
};
use caditor_expression::Expression;

use crate::{
    editing::{self, EditingCommand, SketchEditing},
    model::{Action, Model},
    selection::Selection,
    solid_tools,
    units::LengthUnit,
    visibility,
};

pub const TITLE: &str = "Hole";
pub const DESCRIPTION: &str =
    "Drill a hole at every point of the sketch: plain, counterbored or countersunk";
pub const DEFAULT_DIAMETER: f64 = 6.0;
pub const DEFAULT_DEPTH: f64 = 10.0;
pub const DEFAULT_COUNTERBORE_DIAMETER: f64 = 10.0;
pub const DEFAULT_COUNTERBORE_DEPTH: f64 = 3.0;
pub const DEFAULT_COUNTERSINK_DIAMETER: f64 = 10.0;
pub const DEFAULT_COUNTERSINK_ANGLE: f64 = 90.0;
const NO_SKETCH: &str = "Draw a sketch on a face of a body and place points where the holes go";
const NO_POINTS: &str = "Place points in the sketch where the holes go";
const NO_BODY: &str = "Make a body to drill into first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoleSource {
    pub sketch: FeatureId,
    pub body: FeatureId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Counterbore,
    Countersink,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Plain, Self::Counterbore, Self::Countersink];

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Counterbore => "Counterbore",
            Self::Countersink => "Countersink",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Plain => "A straight hole",
            Self::Counterbore => "A wider flat-bottomed step at the mouth, for a bolt head",
            Self::Countersink => "A cone at the mouth, for a flat-head screw",
        }
    }

    pub fn of(style: &HoleStyle) -> Self {
        match style {
            HoleStyle::Plain => Self::Plain,
            HoleStyle::Counterbore { .. } => Self::Counterbore,
            HoleStyle::Countersink { .. } => Self::Countersink,
        }
    }

    pub fn default_style(self, unit: LengthUnit) -> HoleStyle {
        match self {
            Self::Plain => HoleStyle::Plain,
            Self::Counterbore => HoleStyle::Counterbore {
                diameter: unit.default_length(DEFAULT_COUNTERBORE_DIAMETER),
                depth: unit.default_length(DEFAULT_COUNTERBORE_DEPTH),
            },
            Self::Countersink => HoleStyle::Countersink {
                diameter: unit.default_length(DEFAULT_COUNTERSINK_DIAMETER),
                angle: solid_tools::degrees(DEFAULT_COUNTERSINK_ANGLE),
            },
        }
    }
}

pub fn default_depth(unit: LengthUnit) -> Expression {
    unit.default_length(DEFAULT_DEPTH)
}

pub fn source(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
) -> Result<HoleSource, &'static str> {
    let document = model.document();
    let sketch = solid_tools::sweep_source(document, selection, editing)
        .ok_or(NO_SKETCH)?
        .sketch;
    let definition = editing::edited_sketch(document, sketch).ok_or(NO_SKETCH)?;
    if hole_centres(definition).is_empty() {
        return Err(NO_POINTS);
    }
    let attached = document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(|attachment| attachment.body());
    let body = attached
        .or_else(|| document.bodies_standing().last().copied())
        .ok_or(NO_BODY)?;
    Ok(HoleSource { sketch, body })
}

pub fn create(
    document: &Document,
    source: HoleSource,
    unit: LengthUnit,
) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::Hole(Hole {
            sketch: source.sketch,
            body: source.body,
            diameter: unit.default_length(DEFAULT_DIAMETER),
            depth: HoleDepth::Blind(default_depth(unit)),
            style: HoleStyle::Plain,
            reversed: false,
        }),
    );
    if visibility::is_shown(document, source.sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: source.sketch,
            hidden: true,
        });
    }
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, source: HoleSource) -> Vec<Action> {
    let (transaction, feature) = create(model.document(), source, model.length_unit());
    vec![
        Action::Apply(transaction),
        Action::Editing(EditingCommand::OpenSolid(feature)),
    ]
}

pub fn edit(document: &Document, feature: FeatureId, hole: Hole) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Hole(hole),
        },
    ))
}
