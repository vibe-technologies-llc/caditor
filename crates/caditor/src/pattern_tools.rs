use std::borrow::Cow;

use caditor_document::{
    AxisReference, CircularPattern, Document, Edit, FeatureId, FeatureKind, Instance,
    LinearDirection, LinearSpacing, Pattern, PatternKind, PrincipalAxis, Transaction,
    displayed_axis, repeatable_on,
};
use caditor_expression::Expression;
use caditor_geometry::Aabb;

use crate::{
    bodies,
    commands::{Command, Reason},
    datum_tools,
    editing::{self, EditingCommand},
    feature_fields::ChoiceRefusal,
    field::{self, CheckError},
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    move_tools,
    reference_picking::{Picking, Slot},
    selection::{Pickable, Selection},
    sketch_pattern_tools, solid_tools,
};

pub const LINEAR_TITLE: &str = "Linear pattern";
pub const CIRCULAR_TITLE: &str = "Circular pattern";
pub const LINEAR_DESCRIPTION: &str = "Repeat the body in a row along a direction";
pub const CIRCULAR_DESCRIPTION: &str = "Repeat the body around an axis";
pub const CURVE_TITLE: &str = "Curve pattern";
pub const POINTS_TITLE: &str = "Point pattern";
pub const CURVE_DESCRIPTION: &str = "Repeat the body along a curve of a sketch";
pub const POINTS_DESCRIPTION: &str = "Repeat the body at the points of a sketch";
pub const DEFAULT_LINEAR_COUNT: f64 = 3.0;
pub const DEFAULT_SECOND_COUNT: f64 = 2.0;
pub const DEFAULT_CIRCULAR_COUNT: f64 = 6.0;
pub const FULL_TURN: f64 = 360.0;
const FALLBACK_SPACING: f64 = 10.0;
const SPACING_ROOM: f64 = 1.2;
const FEATURE_SPACING_ROOM: f64 = 2.0;
const NO_BODY_SELECTED: &str =
    "Select a face, edge or vertex of the body to pattern, or the body in the tree";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatternToolError {
    #[error("Make a body first, then pattern it")]
    NoBody,
    #[error(
        "Several axes, straight edges or round faces are selected; select only the one to follow"
    )]
    SeveralAxes,
    #[error("Select an axis, straight edge or round face made before this pattern")]
    NoAxis,
    #[error("Only a linear pattern has a second direction")]
    NotLinear,
    #[error("Only a linear or circular pattern follows an axis")]
    NoAxisToFollow,
    #[error("The feature no longer exists")]
    FeatureGone,
    #[error("The original is never left out of its pattern")]
    OriginalStays,
    #[error("It already runs along the selection")]
    AlreadyAlong,
    #[error("It already turns about the selection")]
    AlreadyTurns,
    #[error("{}", sketch_pattern_tools::NO_PATH)]
    NoPath,
    #[error(transparent)]
    ChosenBody(#[from] move_tools::BodyChoiceError),
    #[error("{words}")]
    SketchPattern { words: &'static str },
    #[error(transparent)]
    Choice(#[from] ChoiceRefusal),
    #[error(transparent)]
    Check(#[from] CheckError),
}

impl Reason for PatternToolError {
    fn reason(&self) -> Cow<'static, str> {
        Cow::Owned(self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Linear,
    Circular,
    Curve,
    Points,
}

impl Shape {
    pub const ALL: [Self; 4] = [Self::Linear, Self::Circular, Self::Curve, Self::Points];
    pub const ON_RIBBON: [Self; 2] = [Self::Linear, Self::Circular];

    pub fn title(self) -> &'static str {
        match self {
            Self::Linear => LINEAR_TITLE,
            Self::Circular => CIRCULAR_TITLE,
            Self::Curve => CURVE_TITLE,
            Self::Points => POINTS_TITLE,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Linear => LINEAR_DESCRIPTION,
            Self::Circular => CIRCULAR_DESCRIPTION,
            Self::Curve => CURVE_DESCRIPTION,
            Self::Points => POINTS_DESCRIPTION,
        }
    }

    pub fn of(kind: &PatternKind) -> Self {
        match kind {
            PatternKind::Linear { .. } => Self::Linear,
            PatternKind::Circular(_) => Self::Circular,
            PatternKind::Curve(_) => Self::Curve,
            PatternKind::Points(_) => Self::Points,
        }
    }

    pub fn command(self) -> Command {
        match self {
            Self::Linear => Command::LinearPattern,
            Self::Circular => Command::CircularPattern,
            Self::Curve => Command::CurvePattern,
            Self::Points => Command::PointPattern,
        }
    }

    pub fn follows_sketch(self) -> bool {
        matches!(self, Self::Curve | Self::Points)
    }
}

fn linear_default_axis() -> AxisReference {
    AxisReference::Principal(PrincipalAxis::X)
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternSource {
    pub body: FeatureId,
    pub axis: Option<AxisReference>,
    pub repeated: Vec<FeatureId>,
    pub sketch: Option<FeatureId>,
}

impl PatternSource {
    pub fn subject(&self, document: &Document) -> String {
        subject(document, self.body, &self.repeated)
    }
}

pub fn subject(document: &Document, body: FeatureId, repeated: &[FeatureId]) -> String {
    let name = |feature: FeatureId| {
        document.feature(feature).map_or_else(
            || "a deleted feature".to_owned(),
            |found| found.name.clone(),
        )
    };
    match repeated {
        [] => format!("the body of {}", name(body)),
        [only] => name(*only),
        [rest @ .., last] => format!(
            "{} and {}",
            rest.iter()
                .map(|feature| name(*feature))
                .collect::<Vec<_>>()
                .join(", "),
            name(*last)
        ),
    }
}

pub fn repeatable(document: &Document, rows: &[FeatureId]) -> Option<(FeatureId, Vec<FeatureId>)> {
    let mut body = None;
    let mut repeated: Vec<(usize, FeatureId)> = Vec::new();
    for row in rows {
        let feature = document.feature(*row)?;
        let on = repeatable_on(&feature.kind)?;
        if body.is_some_and(|known| known != on) {
            return None;
        }
        body = Some(on);
        if !repeated.iter().any(|(_, known)| known == row) {
            repeated.push((document.feature_index(*row)?, *row));
        }
    }
    let nothing_of_their_own = repeated.iter().all(|(_, row)| {
        document
            .feature(*row)
            .is_some_and(|feature| feature.kind.blend().is_some() || feature.kind.shell().is_some())
    });
    if nothing_of_their_own {
        return None;
    }
    repeated.sort_unstable();
    Some((body?, repeated.into_iter().map(|(_, row)| row).collect()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference {
    First,
    Second,
}

fn last_body(model: &Model) -> Option<FeatureId> {
    model
        .document()
        .bodies_standing_in(model.evaluation())
        .last()
        .copied()
}

pub fn source(
    model: &Model,
    selection: &Selection,
    tree: &[FeatureId],
    rows: &[FeatureId],
) -> Result<PatternSource, PatternToolError> {
    let document = model.document();
    let (body, repeated) = match repeatable(document, rows) {
        Some(repeating) => repeating,
        None if selection.is_empty() && tree.is_empty() => (
            last_body(model).ok_or(PatternToolError::NoBody)?,
            Vec::new(),
        ),
        None => (
            move_tools::chosen_body(model, selection, tree, NO_BODY_SELECTED)?,
            Vec::new(),
        ),
    };
    let axis = chosen_axis(model, selection, body, document.bar_index())?;
    Ok(PatternSource {
        body,
        axis,
        repeated,
        sketch: None,
    })
}

fn chosen_axis(
    model: &Model,
    selection: &Selection,
    body: FeatureId,
    index: usize,
) -> Result<Option<AxisReference>, PatternToolError> {
    let mut candidates: Vec<(Pickable, AxisReference)> = Vec::new();
    for pickable in selection.in_pick_order() {
        if let Some(axis) = datum_tools::axis_reference(model, pickable, index)
            && !candidates.iter().any(|(_, known)| *known == axis)
        {
            candidates.push((pickable, axis));
        }
    }
    let outside: Vec<&AxisReference> = candidates
        .iter()
        .filter(|(pickable, _)| pickable.body() != Some(body))
        .map(|(_, axis)| axis)
        .collect();
    match (candidates.as_slice(), outside.as_slice()) {
        ([], _) => Ok(None),
        ([(_, axis)], _) => Ok(Some(axis.clone())),
        (_, [axis]) => Ok(Some((*axis).clone())),
        _ => Err(PatternToolError::SeveralAxes),
    }
}

pub fn repeated_bounds(model: &Model, repeated: &[FeatureId]) -> Option<Aabb> {
    let evaluation = model.evaluation();
    repeated
        .iter()
        .flat_map(|feature| {
            evaluation
                .cuts(*feature)
                .iter()
                .chain(evaluation.joins(*feature))
        })
        .filter_map(|tool| tool.solid()?.bounding_box())
        .reduce(|all, bounds| all.union(bounds))
}

fn default_spacing(
    model: &Model,
    body: FeatureId,
    repeated: &[FeatureId],
    axis: &AxisReference,
) -> f64 {
    let evaluation = model.evaluation();
    let (bounds, room) = if repeated.is_empty() {
        (
            bodies::shown(evaluation, body).and_then(|shown| shown.bounding_box()),
            SPACING_ROOM,
        )
    } else {
        (repeated_bounds(model, repeated), FEATURE_SPACING_ROOM)
    };
    let extent = displayed_axis(evaluation, body, axis).and_then(|ray| {
        let bounds = bounds?;
        let along = bounds.corners().map(|corner| ray.direction().dot(corner));
        let low = along.iter().copied().fold(f64::INFINITY, f64::min);
        let high = along.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some(high - low)
    });
    match extent {
        Some(extent) if extent.is_finite() && extent > 0.0 => (extent * room).ceil(),
        _ => FALLBACK_SPACING,
    }
}

fn direction(
    model: &Model,
    (body, repeated): (FeatureId, &[FeatureId]),
    axis: AxisReference,
    count: Expression,
    starts: &Starts,
) -> LinearDirection {
    let spacing = starts
        .remembered(Remembered::LinearSpacing)
        .unwrap_or_else(|| {
            starts
                .unit()
                .default_length(default_spacing(model, body, repeated, &axis))
        });
    LinearDirection {
        axis,
        count,
        spacing,
        measured: LinearSpacing::BetweenCopies,
        reversed: false,
    }
}

fn kind_for(
    model: &Model,
    shape: Shape,
    source: (FeatureId, &[FeatureId]),
    axis: Option<AxisReference>,
) -> PatternKind {
    match shape {
        Shape::Linear => PatternKind::Linear {
            first: direction(
                model,
                source,
                axis.unwrap_or_else(linear_default_axis),
                Starts::of(model).count(Remembered::LinearCount, DEFAULT_LINEAR_COUNT),
                &Starts::of(model),
            ),
            second: None,
        },
        Shape::Circular | Shape::Curve | Shape::Points => PatternKind::Circular(CircularPattern {
            axis,
            count: Starts::of(model).count(Remembered::CircularCount, DEFAULT_CIRCULAR_COUNT),
            angle: solid_tools::degrees(FULL_TURN),
            reversed: false,
        }),
    }
}

pub fn create(
    model: &Model,
    shape: Shape,
    source: &PatternSource,
) -> Result<(Transaction, FeatureId), PatternToolError> {
    let document = model.document();
    let kind = match (shape.follows_sketch(), source.sketch) {
        (true, Some(sketch)) => sketch_pattern_tools::kind(shape, sketch, model.length_unit()),
        (true, None) => return Err(PatternToolError::NoPath),
        (false, _) => kind_for(
            model,
            shape,
            (source.body, &source.repeated),
            source.axis.clone(),
        ),
    };
    let name = editing::next_feature_name(document, shape.title());
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(
        name,
        FeatureKind::from(Pattern::new(source.body, kind).repeating(source.repeated.clone())),
    );
    field::checked(document, transaction.finish()).map(|transaction| (transaction, feature))
}

pub fn create_actions(model: &Model, shape: Shape, source: &PatternSource) -> Vec<Action> {
    match create(model, shape, source) {
        Ok((transaction, feature)) => {
            let mut actions = vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ];
            if shape == Shape::Circular && source.axis.is_none() {
                actions.push(Action::Editing(EditingCommand::Pick(Picking::new(
                    feature,
                    Slot::PatternDirection,
                ))));
            }
            actions
        }
        Err(reason) => vec![Action::Inform(Notice::warning(format!(
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
    let brought_back = match copy.pattern.kind {
        PatternKind::Points(_) => "Bring the copies back",
        PatternKind::Linear { .. } | PatternKind::Circular(_) | PatternKind::Curve(_) => {
            "the Instances grid"
        }
    };
    format!(
        "Click to leave {} out of {name}; {brought_back} brings it back",
        copy.pattern.instance_words(copy.instance)
    )
}

pub fn leave_out(
    model: &Model,
    open: FeatureId,
    copy: ClickedCopy,
) -> Result<Transaction, PatternToolError> {
    let left_out = copy
        .pattern
        .toggled(copy.instance)
        .ok_or(PatternToolError::OriginalStays)?;
    change(model, open, left_out)
}

pub fn change(
    model: &Model,
    feature: FeatureId,
    pattern: Pattern,
) -> Result<Transaction, PatternToolError> {
    let document = model.document();
    let transaction = edit(document, feature, pattern).ok_or(PatternToolError::FeatureGone)?;
    field::checked(document, transaction)
}

pub fn reshaped(
    model: &Model,
    feature: FeatureId,
    pattern: &Pattern,
    shape: Shape,
) -> Result<Pattern, PatternToolError> {
    let kind = match (&pattern.kind, shape) {
        (PatternKind::Linear { .. }, Shape::Linear)
        | (PatternKind::Circular(_), Shape::Circular)
        | (PatternKind::Curve(_), Shape::Curve)
        | (PatternKind::Points(_), Shape::Points) => return Ok(pattern.clone()),
        (_, Shape::Curve | Shape::Points) => {
            let before = model
                .document()
                .feature_index(feature)
                .ok_or(PatternToolError::FeatureGone)?;
            sketch_pattern_tools::reshaped_kind(model, pattern, shape, before)
                .map_err(|words| PatternToolError::SketchPattern { words })?
        }
        (PatternKind::Curve(curve), Shape::Linear | Shape::Circular) => {
            let mut kind = kind_for(model, shape, (pattern.body, &pattern.repeated), None);
            match &mut kind {
                PatternKind::Linear { first, .. } => first.count = curve.count.clone(),
                PatternKind::Circular(circular) => circular.count = curve.count.clone(),
                PatternKind::Curve(_) | PatternKind::Points(_) => {}
            }
            kind
        }
        (PatternKind::Points(_), Shape::Linear | Shape::Circular) => {
            kind_for(model, shape, (pattern.body, &pattern.repeated), None)
        }
        (PatternKind::Linear { first, .. }, Shape::Circular) => {
            PatternKind::Circular(CircularPattern {
                axis: Some(first.axis.clone()),
                count: first.count.clone(),
                angle: solid_tools::degrees(FULL_TURN),
                reversed: first.reversed,
            })
        }
        (PatternKind::Circular(circular), Shape::Linear) => {
            let mut first = direction(
                model,
                (pattern.body, &pattern.repeated),
                circular.axis.clone().unwrap_or_else(linear_default_axis),
                Expression::Number(DEFAULT_LINEAR_COUNT),
                &Starts::of(model),
            );
            first.count = circular.count.clone();
            first.reversed = circular.reversed;
            PatternKind::Linear {
                first,
                second: None,
            }
        }
    };
    Ok(Pattern::new(pattern.body, kind).repeating(pattern.repeated.clone()))
}

fn with_selected(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pattern: &Pattern,
    reference: Reference,
) -> Result<Pattern, PatternToolError> {
    let index = model
        .document()
        .feature_index(feature)
        .ok_or(PatternToolError::FeatureGone)?;
    let axis =
        chosen_axis(model, selection, pattern.body, index)?.ok_or(PatternToolError::NoAxis)?;
    with_axis(model, pattern, reference, axis)
}

pub fn with_axis(
    model: &Model,
    pattern: &Pattern,
    reference: Reference,
    axis: AxisReference,
) -> Result<Pattern, PatternToolError> {
    let kind = match (&pattern.kind, reference) {
        (PatternKind::Linear { first, second }, Reference::First) => {
            if first.axis == axis {
                return Err(PatternToolError::AlreadyAlong);
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
                Some(existing) if existing.axis == axis => {
                    return Err(PatternToolError::AlreadyAlong);
                }
                Some(existing) => LinearDirection {
                    axis,
                    ..existing.clone()
                },
                None => direction(
                    model,
                    (pattern.body, &pattern.repeated),
                    axis,
                    Expression::Number(DEFAULT_SECOND_COUNT),
                    &Starts::of(model),
                ),
            };
            PatternKind::Linear {
                first: first.clone(),
                second: Some(second),
            }
        }
        (PatternKind::Circular(circular), Reference::First) => {
            if circular.axis.as_ref() == Some(&axis) {
                return Err(PatternToolError::AlreadyTurns);
            }
            PatternKind::Circular(CircularPattern {
                axis: Some(axis),
                ..circular.clone()
            })
        }
        (PatternKind::Circular(_), Reference::Second) => return Err(PatternToolError::NotLinear),
        (PatternKind::Curve(_) | PatternKind::Points(_), _) => {
            return Err(PatternToolError::NoAxisToFollow);
        }
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
    let frames = datum_tools::frames_before(document, before)
        .into_iter()
        .flat_map(|frame| PrincipalAxis::ALL.map(|axis| AxisReference::Frame { frame, axis }));
    PrincipalAxis::ALL
        .into_iter()
        .map(AxisReference::Principal)
        .chain(datums)
        .chain(frames)
        .collect()
}

pub fn selected_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pattern: &Pattern,
    reference: Reference,
) -> Result<Transaction, PatternToolError> {
    with_selected(model, selection, feature, pattern, reference)
        .and_then(|changed| change(model, feature, changed))
}

pub fn repeating(
    model: &Model,
    feature: FeatureId,
    pattern: &Pattern,
    repeated: Vec<FeatureId>,
) -> Result<Transaction, PatternToolError> {
    change(
        model,
        feature,
        Pattern {
            repeated,
            ..pattern.clone()
        },
    )
}
