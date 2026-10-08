use std::collections::BTreeSet;

use caditor_document::{
    AxisReference, CircularPattern, Document, Edit, FeatureId, FeatureKind, Instance,
    LinearDirection, LinearSpacing, Pattern, PatternKind, PrincipalAxis, Transaction,
    displayed_axis, instance_name,
};
use caditor_expression::Expression;

use crate::{
    bodies, datum_tools,
    editing::{self, EditingCommand},
    field,
    model::{Action, Model, Notice},
    selection::{Pickable, Selection},
    solid_tools,
    units::LengthUnit,
};

pub const LINEAR_TITLE: &str = "Linear pattern";
pub const CIRCULAR_TITLE: &str = "Circular pattern";
pub const LINEAR_DESCRIPTION: &str = "Repeat the body in a row along a direction";
pub const CIRCULAR_DESCRIPTION: &str = "Repeat the body around an axis";
pub const DEFAULT_LINEAR_COUNT: f64 = 3.0;
pub const DEFAULT_SECOND_COUNT: f64 = 2.0;
pub const DEFAULT_CIRCULAR_COUNT: f64 = 6.0;
pub const FULL_TURN: f64 = 360.0;
const FALLBACK_SPACING: f64 = 10.0;
const SPACING_ROOM: f64 = 1.2;
const NO_BODY: &str = "Make a body first, then pattern it";
const SEVERAL_BODIES: &str = "Select faces or edges of one body only";
const NO_AXIS: &str = "Select an axis, straight edge or round face made before this pattern";
const NOT_LINEAR: &str = "Only a linear pattern has a second direction";
const GONE: &str = "The feature no longer exists";
const ORIGINAL_STAYS: &str = "The original is never left out of its pattern";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Linear,
    Circular,
}

impl Shape {
    pub const ALL: [Self; 2] = [Self::Linear, Self::Circular];

    pub fn title(self) -> &'static str {
        match self {
            Self::Linear => LINEAR_TITLE,
            Self::Circular => CIRCULAR_TITLE,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Linear => LINEAR_DESCRIPTION,
            Self::Circular => CIRCULAR_DESCRIPTION,
        }
    }

    pub fn of(kind: &PatternKind) -> Self {
        match kind {
            PatternKind::Linear { .. } => Self::Linear,
            PatternKind::Circular(_) => Self::Circular,
        }
    }

    fn default_axis(self) -> AxisReference {
        match self {
            Self::Linear => AxisReference::Principal(PrincipalAxis::X),
            Self::Circular => AxisReference::Principal(PrincipalAxis::Z),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternSource {
    pub body: FeatureId,
    pub axis: Option<AxisReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    First,
    Second,
}

fn last_body(document: &Document) -> Option<FeatureId> {
    document.bodies_standing().last().copied()
}

pub fn source(
    model: &Model,
    selection: &Selection,
    axis: Option<&AxisReference>,
) -> Result<PatternSource, &'static str> {
    let document = model.document();
    let mut chosen: BTreeSet<FeatureId> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::Face { body, .. } | Pickable::Edge { body, .. } => Some(body),
            _ => None,
        })
        .collect();
    if chosen.len() > 1 {
        return Err(SEVERAL_BODIES);
    }
    let body = chosen
        .pop_first()
        .or_else(|| last_body(document))
        .ok_or(NO_BODY)?;
    Ok(PatternSource {
        body,
        axis: axis.cloned(),
    })
}

fn default_spacing(model: &Model, body: FeatureId, axis: &AxisReference) -> f64 {
    let evaluation = model.evaluation();
    let extent = displayed_axis(evaluation, body, axis).and_then(|ray| {
        let bounds = bodies::shown(evaluation, body)?.bounding_box()?;
        let along = bounds.corners().map(|corner| ray.direction().dot(corner));
        let low = along.iter().copied().fold(f64::INFINITY, f64::min);
        let high = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some(high - low)
    });
    match extent {
        Some(extent) if extent.is_finite() && extent > 0.0 => (extent * SPACING_ROOM).ceil(),
        _ => FALLBACK_SPACING,
    }
}

fn direction(
    model: &Model,
    body: FeatureId,
    axis: AxisReference,
    count: f64,
    unit: LengthUnit,
) -> LinearDirection {
    let spacing = default_spacing(model, body, &axis);
    LinearDirection {
        axis,
        count: Expression::Number(count),
        spacing: unit.default_length(spacing),
        measured: LinearSpacing::BetweenCopies,
        reversed: false,
    }
}

fn kind_for(model: &Model, shape: Shape, body: FeatureId, axis: AxisReference) -> PatternKind {
    match shape {
        Shape::Linear => PatternKind::Linear {
            first: direction(model, body, axis, DEFAULT_LINEAR_COUNT, model.length_unit()),
            second: None,
        },
        Shape::Circular => PatternKind::Circular(CircularPattern {
            axis,
            count: Expression::Number(DEFAULT_CIRCULAR_COUNT),
            angle: solid_tools::degrees(FULL_TURN),
            reversed: false,
        }),
    }
}

pub fn create(
    model: &Model,
    shape: Shape,
    source: &PatternSource,
) -> Result<(Transaction, FeatureId), String> {
    let document = model.document();
    let axis = source.axis.clone().unwrap_or_else(|| shape.default_axis());
    let name = editing::next_feature_name(document, shape.title());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::from(Pattern::new(
            source.body,
            kind_for(model, shape, source.body, axis),
        )),
    );
    field::checked(document, transaction.finish()).map(|transaction| (transaction, feature))
}

pub fn create_actions(model: &Model, shape: Shape, source: &PatternSource) -> Vec<Action> {
    match create(model, shape, source) {
        Ok((transaction, feature)) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::OpenSolid(feature)),
        ],
        Err(reason) => vec![Action::Inform(Notice::info(format!(
            "{}: {reason}.",
            shape.title()
        )))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, pattern: Pattern) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::from(pattern),
        },
    ))
}

pub struct ClickedCopy {
    pub instance: Instance,
    pub pattern: Pattern,
}

pub fn clicked_copy(model: &Model, open: FeatureId, pickable: Pickable) -> Option<ClickedCopy> {
    let Pickable::Face { body, face } = pickable else {
        return None;
    };
    let FeatureKind::Pattern(pattern) = &model.document().feature(open)?.kind else {
        return None;
    };
    if pattern.body != body {
        return None;
    }
    let result = model.evaluation().body_result(body)?.solid()?;
    let found = bodies::find_face(result, face)?;
    let copy = result.solid.face(found)?.origin()?.copy()?;
    (copy.pattern == open.raw()).then(|| ClickedCopy {
        instance: copy.index,
        pattern: pattern.as_ref().clone(),
    })
}

pub fn leave_out_words(document: &Document, open: FeatureId, copy: &ClickedCopy) -> String {
    let name = document
        .feature(open)
        .map_or("the pattern", |feature| feature.name.as_str());
    format!(
        "Click to leave {} out of {name}; the Instances grid brings it back",
        instance_name(copy.instance)
    )
}

pub fn leave_out(model: &Model, open: FeatureId, copy: ClickedCopy) -> Result<Transaction, String> {
    let left_out = copy.pattern.toggled(copy.instance).ok_or(ORIGINAL_STAYS)?;
    change(model, open, left_out)
}

pub fn change(model: &Model, feature: FeatureId, pattern: Pattern) -> Result<Transaction, String> {
    let document = model.document();
    let transaction = edit(document, feature, pattern).ok_or_else(|| GONE.to_owned())?;
    field::checked(document, transaction)
}

pub fn reshaped(model: &Model, pattern: &Pattern, shape: Shape) -> Pattern {
    let kind = match (&pattern.kind, shape) {
        (PatternKind::Linear { .. }, Shape::Linear)
        | (PatternKind::Circular(_), Shape::Circular) => return pattern.clone(),
        (PatternKind::Linear { first, .. }, Shape::Circular) => {
            PatternKind::Circular(CircularPattern {
                axis: first.axis.clone(),
                count: first.count.clone(),
                angle: solid_tools::degrees(FULL_TURN),
                reversed: first.reversed,
            })
        }
        (PatternKind::Circular(circular), Shape::Linear) => {
            let mut first = direction(
                model,
                pattern.body,
                circular.axis.clone(),
                DEFAULT_LINEAR_COUNT,
                model.length_unit(),
            );
            first.count = circular.count.clone();
            first.reversed = circular.reversed;
            PatternKind::Linear {
                first,
                second: None,
            }
        }
    };
    Pattern::new(pattern.body, kind)
}

fn selected_axis(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
) -> Option<AxisReference> {
    let index = model.document().feature_index(feature)?;
    selection
        .iter()
        .find_map(|pickable| datum_tools::axis_reference(model, pickable, index))
}

fn with_selected(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pattern: &Pattern,
    reference: Reference,
) -> Result<Pattern, &'static str> {
    let axis = selected_axis(model, selection, feature).ok_or(NO_AXIS)?;
    with_axis(model, pattern, reference, axis)
}

pub fn with_axis(
    model: &Model,
    pattern: &Pattern,
    reference: Reference,
    axis: AxisReference,
) -> Result<Pattern, &'static str> {
    let already = "It already runs along the selection";
    let kind = match (&pattern.kind, reference) {
        (PatternKind::Linear { first, second }, Reference::First) => {
            if first.axis == axis {
                return Err(already);
            }
            PatternKind::Linear {
                first: LinearDirection {
                    axis,
                    ..first.clone()
                },
                second: second.clone(),
            }
        }
        (PatternKind::Linear { first, second }, Reference::Second) => {
            let second = match second {
                Some(existing) if existing.axis == axis => return Err(already),
                Some(existing) => LinearDirection {
                    axis,
                    ..existing.clone()
                },
                None => direction(
                    model,
                    pattern.body,
                    axis,
                    DEFAULT_SECOND_COUNT,
                    model.length_unit(),
                ),
            };
            PatternKind::Linear {
                first: first.clone(),
                second: Some(second),
            }
        }
        (PatternKind::Circular(circular), Reference::First) => {
            if circular.axis == axis {
                return Err("It already turns about the selection");
            }
            PatternKind::Circular(CircularPattern {
                axis,
                ..circular.clone()
            })
        }
        (PatternKind::Circular(_), Reference::Second) => return Err(NOT_LINEAR),
    };
    Ok(Pattern {
        kind,
        ..pattern.clone()
    })
}

pub fn listed_axes(model: &Model, feature: FeatureId) -> Vec<AxisReference> {
    let document = model.document();
    let before = document.feature_index(feature).unwrap_or(usize::MAX);
    let datums = document
        .features()
        .take(before)
        .filter(|candidate| matches!(candidate.kind.datum(), Some(datum) if datum.is_axis()))
        .map(|datum| AxisReference::Datum(datum.id()));
    PrincipalAxis::ALL
        .into_iter()
        .map(AxisReference::Principal)
        .chain(datums)
        .collect()
}

pub fn selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pattern: &Pattern,
    reference: Reference,
) -> Result<Transaction, String> {
    with_selected(model, selection, feature, pattern, reference)
        .map_err(str::to_owned)
        .and_then(|changed| change(model, feature, changed))
}
