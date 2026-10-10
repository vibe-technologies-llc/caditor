use caditor_document::{
    CopyOrientation, CurvePattern, CurveSpacing, Document, FeatureId, FeatureKind, FeatureResult,
    Pattern, PatternKind, PointReference, PointsPattern, Transaction, is_curve_path,
};
use caditor_expression::Expression;
use caditor_sketch::Sketch;

use crate::{
    body_selection, datum_tools,
    model::Model,
    move_tools,
    pattern_tools::{self, PatternSource, Shape},
    selection::{Pickable, Selection},
    units::LengthUnit,
};

pub const DEFAULT_CURVE_COUNT: f64 = 4.0;
const DEFAULT_CURVE_SPACING: f64 = 10.0;
pub const NO_PATH: &str = "Draw the curve to follow in a sketch above the pattern, as one chain of lines, arcs or splines";
pub const NO_POINTS: &str =
    "Draw lone points with the Point tool in a sketch above the pattern to place the copies at";
const NO_BODY: &str = "Make a body first, then pattern it";
const NO_BODY_SELECTED: &str =
    "Select a face, edge or vertex of the body to pattern, or the body in the tree";
const SEVERAL_SKETCHES: &str =
    "Geometry of several sketches is selected; select the curves or points of one";
const NOT_A_PATH: &str =
    "The curves of the selected sketch do not join end to end into one chain to follow";
const POINTLESS: &str = "The selected sketch has no lone points to place copies at";
const MADE_LATER: &str = "The selected sketch is made after the pattern";
const NO_SKETCH_SELECTED: &str = "Select a curve or point of a sketch made before the pattern";
const NO_POINT_SELECTED: &str =
    "Select one corner, round edge, sphere, sketch point or datum point for the base point";
const GONE: &str = "The feature no longer exists";
const NOT_FROM_A_SKETCH: &str = "Only a curve or point pattern follows a sketch";

fn solved(model: &Model, sketch: FeatureId) -> Option<&Sketch> {
    model
        .evaluation()
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
}

pub fn qualifies(model: &Model, shape: Shape, sketch: FeatureId) -> bool {
    solved(model, sketch).is_some_and(|solved| match shape {
        Shape::Curve => is_curve_path(solved),
        Shape::Points => !solved.free_points().is_empty(),
        Shape::Linear | Shape::Circular => false,
    })
}

fn used_after(document: &Document, sketch: FeatureId, index: usize) -> bool {
    document
        .features()
        .skip(index + 1)
        .any(|feature| feature.kind.features().contains(&sketch))
}

pub fn listed(model: &Model, shape: Shape, before: usize) -> Vec<FeatureId> {
    model
        .document()
        .features()
        .take(before)
        .filter(|feature| matches!(feature.kind, FeatureKind::Sketch(_)))
        .map(|feature| feature.id())
        .filter(|sketch| qualifies(model, shape, *sketch))
        .collect()
}

pub fn guess(model: &Model, shape: Shape, before: usize) -> Option<FeatureId> {
    let document = model.document();
    let sketches: Vec<(usize, FeatureId)> = document
        .features()
        .take(before)
        .enumerate()
        .filter(|(_, feature)| matches!(feature.kind, FeatureKind::Sketch(_)))
        .map(|(index, feature)| (index, feature.id()))
        .collect();
    sketches.into_iter().rev().find_map(|(index, sketch)| {
        (!used_after(document, sketch, index) && qualifies(model, shape, sketch)).then_some(sketch)
    })
}

fn wanting(shape: Shape) -> &'static str {
    match shape {
        Shape::Points => NO_POINTS,
        Shape::Curve | Shape::Linear | Shape::Circular => NO_PATH,
    }
}

pub fn selected_sketch(
    model: &Model,
    selection: &Selection,
    shape: Shape,
    before: usize,
) -> Result<Option<FeatureId>, &'static str> {
    let mut sketches: Vec<FeatureId> = selection
        .iter()
        .filter_map(|pickable| match pickable {
            Pickable::SketchEntity { feature, .. } | Pickable::SketchRegion { feature, .. } => {
                Some(feature)
            }
            _ => None,
        })
        .collect();
    sketches.sort_unstable();
    sketches.dedup();
    let [sketch] = sketches.as_slice() else {
        return if sketches.is_empty() {
            Ok(None)
        } else {
            Err(SEVERAL_SKETCHES)
        };
    };
    if model
        .document()
        .feature_index(*sketch)
        .is_none_or(|index| index >= before)
    {
        return Err(MADE_LATER);
    }
    if !qualifies(model, shape, *sketch) {
        return Err(match shape {
            Shape::Points => POINTLESS,
            Shape::Curve | Shape::Linear | Shape::Circular => NOT_A_PATH,
        });
    }
    Ok(Some(*sketch))
}

pub fn source(
    model: &Model,
    selection: &Selection,
    (tree, rows): (&[FeatureId], &[FeatureId]),
    shape: Shape,
) -> Result<PatternSource, &'static str> {
    let document = model.document();
    let before = document.bar_index();
    let (body, repeated) = match pattern_tools::repeatable(document, rows) {
        Some(repeating) => repeating,
        None if tree.is_empty() && body_selection::bodies_in(selection).is_empty() => (
            document.bodies_standing().last().copied().ok_or(NO_BODY)?,
            Vec::new(),
        ),
        None => (
            move_tools::chosen_body(model, selection, tree, NO_BODY_SELECTED)?,
            Vec::new(),
        ),
    };
    let sketch = match selected_sketch(model, selection, shape, before)? {
        Some(sketch) => sketch,
        None => guess(model, shape, before).ok_or(wanting(shape))?,
    };
    Ok(PatternSource {
        body,
        axis: None,
        repeated,
        sketch: Some(sketch),
    })
}

pub fn kind(shape: Shape, sketch: FeatureId, unit: LengthUnit) -> PatternKind {
    match shape {
        Shape::Points => PatternKind::Points(PointsPattern {
            sketch,
            base: PointReference::Origin,
        }),
        Shape::Curve | Shape::Linear | Shape::Circular => PatternKind::Curve(CurvePattern {
            sketch,
            count: Expression::Number(DEFAULT_CURVE_COUNT),
            spacing: unit.default_length(DEFAULT_CURVE_SPACING),
            measured: CurveSpacing::Spread,
            orientation: CopyOrientation::Kept,
            reversed: false,
        }),
    }
}

pub fn reshaped_kind(
    model: &Model,
    pattern: &Pattern,
    shape: Shape,
    before: usize,
) -> Result<PatternKind, &'static str> {
    let kept = pattern
        .kind
        .sketch()
        .filter(|sketch| qualifies(model, shape, *sketch));
    let sketch = kept
        .or_else(|| guess(model, shape, before))
        .ok_or(wanting(shape))?;
    let mut kind = kind(shape, sketch, model.length_unit());
    if let (PatternKind::Curve(curve), Some(count)) = (&mut kind, count_of(&pattern.kind)) {
        curve.count = count;
    }
    Ok(kind)
}

fn count_of(kind: &PatternKind) -> Option<Expression> {
    match kind {
        PatternKind::Linear { first, .. } => Some(first.count.clone()),
        PatternKind::Circular(circular) => Some(circular.count.clone()),
        PatternKind::Curve(curve) => Some(curve.count.clone()),
        PatternKind::Points(_) => None,
    }
}

pub fn with_sketch(pattern: &Pattern, sketch: FeatureId) -> Result<Pattern, &'static str> {
    let kind = match &pattern.kind {
        PatternKind::Curve(curve) if curve.sketch == sketch => return Err(ALREADY),
        PatternKind::Points(points) if points.sketch == sketch => return Err(ALREADY),
        PatternKind::Curve(curve) => PatternKind::Curve(CurvePattern {
            sketch,
            ..curve.clone()
        }),
        PatternKind::Points(points) => PatternKind::Points(PointsPattern {
            sketch,
            ..points.clone()
        }),
        PatternKind::Linear { .. } | PatternKind::Circular(_) => return Err(NOT_FROM_A_SKETCH),
    };
    Ok(Pattern {
        kind,
        skipped: Default::default(),
        ..pattern.clone()
    })
}

const ALREADY: &str = "It already follows that sketch";

pub fn sketch_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pattern: &Pattern,
) -> Result<Transaction, String> {
    let before = model.document().feature_index(feature).ok_or(GONE)?;
    let shape = Shape::of(&pattern.kind);
    let sketch = selected_sketch(model, selection, shape, before)?.ok_or(NO_SKETCH_SELECTED)?;
    let changed = with_sketch(pattern, sketch)?;
    pattern_tools::change(model, feature, changed)
}

pub fn base_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    pattern: &Pattern,
) -> Result<Transaction, String> {
    let index = model.document().feature_index(feature).ok_or(GONE)?;
    let mut picked = selection.iter();
    let base = match (picked.next(), picked.next()) {
        (Some(only), None) => datum_tools::point_reference(model, only, index),
        _ => None,
    }
    .ok_or(NO_POINT_SELECTED)?;
    with_base(model, feature, pattern, base)
}

pub fn with_base(
    model: &Model,
    feature: FeatureId,
    pattern: &Pattern,
    base: PointReference,
) -> Result<Transaction, String> {
    let PatternKind::Points(points) = &pattern.kind else {
        return Err("Only a point pattern has a base point".to_owned());
    };
    if points.base == base {
        return Err("It is already placed from that point".to_owned());
    }
    let kind = PatternKind::Points(PointsPattern {
        base,
        ..points.clone()
    });
    pattern_tools::change(
        model,
        feature,
        Pattern {
            kind,
            ..pattern.clone()
        },
    )
}
