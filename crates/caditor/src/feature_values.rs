use caditor_document::{
    Blend, BlendKind, ChamferForm, Edit, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId,
    FeatureKind, FeatureResult, HoleDepth, LinearDirection, LinearSpacing, Move, MoveAxis,
    ParameterValues, PatternKind, PrimitiveShape, Revolve, RevolveExtent, SizeRule, SolidFeature,
    ThreadLength, Transaction, origin_feature,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Aabb, Point3};
use caditor_kernel::{Face, Solid};

use crate::{
    feature_fields::Rule,
    field,
    model::Model,
    move_manipulator::Reach,
    solid_panel,
    value_shapes::{self, Drawn},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueSlot {
    Extrude(Reach),
    RevolveAngle,
    RevolveTotal,
    RevolveForward,
    RevolveBackward,
    BlendSize,
    ShellThickness,
    HoleDiameter,
    HoleDepth,
    Count,
    Spacing,
    SecondCount,
    SecondSpacing,
    PatternAngle,
    ChamferSecondDistance,
    ChamferAngle,
    OffsetDistance,
    Primitive(PrimitiveSize),
    MoveOffset(MoveAxis),
    ThreadDepth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrimitiveSize {
    Length,
    Width,
    Height,
    Diameter,
    TubeDiameter,
    BottomDiameter,
    TopDiameter,
    TopLength,
    Sides,
}

impl PrimitiveSize {
    fn named(what: &str) -> Option<Self> {
        Some(match what {
            "length" => Self::Length,
            "width" => Self::Width,
            "height" => Self::Height,
            "diameter" => Self::Diameter,
            "tube diameter" => Self::TubeDiameter,
            "bottom diameter" => Self::BottomDiameter,
            "top diameter" => Self::TopDiameter,
            "top length" => Self::TopLength,
            "number of sides" => Self::Sides,
            _ => return None,
        })
    }

    pub fn caption(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Width => "Width",
            Self::Height => "Height",
            Self::Diameter => "Diameter",
            Self::TubeDiameter => "Tube diameter",
            Self::BottomDiameter => "Bottom diameter",
            Self::TopDiameter => "Top diameter",
            Self::TopLength => "Top length",
            Self::Sides => "Number of sides",
        }
    }
}

fn move_caption(axis: MoveAxis) -> &'static str {
    match axis {
        MoveAxis::X => "Move along X",
        MoveAxis::Y => "Move along Y",
        MoveAxis::Z => "Move along Z",
    }
}

impl ValueSlot {
    pub fn words(self) -> &'static str {
        match self {
            Self::Extrude(reach) => reach.field_caption(),
            Self::RevolveAngle => "Angle",
            Self::RevolveTotal | Self::PatternAngle => "Total angle",
            Self::RevolveForward => "Forward angle",
            Self::RevolveBackward => "Backward angle",
            Self::BlendSize => "Size",
            Self::ShellThickness => "Thickness",
            Self::HoleDiameter => "Diameter",
            Self::HoleDepth => "Depth",
            Self::Count => COUNT,
            Self::Spacing => "Spacing",
            Self::SecondCount => SECOND_COUNT,
            Self::SecondSpacing => "Second spacing",
            Self::ChamferSecondDistance => "Second distance",
            Self::ChamferAngle => "Chamfer angle",
            Self::OffsetDistance => "Offset distance",
            Self::Primitive(size) => size.caption(),
            Self::MoveOffset(axis) => move_caption(axis),
            Self::ThreadDepth => "Thread depth",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureValue {
    pub feature: FeatureId,
    pub slot: ValueSlot,
    pub caption: &'static str,
    pub expression: Expression,
    pub dimension: Dimension,
    pub rule: Rule,
    pub drawn: Drawn,
    pub anchor: Point3,
}

struct Found {
    slot: ValueSlot,
    caption: &'static str,
    expression: Expression,
    dimension: Dimension,
    rule: Rule,
}

const COUNT: &str = "Count";
const SECOND_COUNT: &str = "Second count";

fn found(
    slot: ValueSlot,
    caption: &'static str,
    expression: &Expression,
    (dimension, rule): (Dimension, Rule),
) -> Found {
    Found {
        slot,
        caption,
        expression: expression.clone(),
        dimension,
        rule,
    }
}

fn degrees(parameters: &ParameterValues, angle: &Expression) -> f64 {
    parameters
        .evaluate_expression(angle)
        .map_or(0.0, |value| value.value)
}

fn extrude_values(extrude: &Extrude) -> Vec<Found> {
    let length = |rule| (Dimension::LENGTH, rule);
    let end = |reach: Reach, end: &ExtrudeEnd, rule| match end {
        ExtrudeEnd::Distance(distance) => Some(found(
            ValueSlot::Extrude(reach),
            reach.field_caption(),
            distance,
            length(rule),
        )),
        ExtrudeEnd::ThroughAll
        | ExtrudeEnd::UpToNext { .. }
        | ExtrudeEnd::UpToFace { .. }
        | ExtrudeEnd::UpToSurface { .. } => None,
    };
    match &extrude.extent {
        ExtrudeExtent::OneSide { end: only, .. } => {
            end(Reach::Only, only, Rule::AboveZeroOrReverse)
                .into_iter()
                .collect()
        }
        ExtrudeExtent::Symmetric { distance } => vec![found(
            ValueSlot::Extrude(Reach::Symmetric),
            solid_panel::TOTAL_DISTANCE,
            distance,
            length(Rule::AboveZero),
        )],
        ExtrudeExtent::TwoSides { forward, backward } => [
            end(Reach::Forward, forward, Rule::AboveZero),
            end(Reach::Backward, backward, Rule::AboveZero),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

fn revolve_values(parameters: &ParameterValues, revolve: &Revolve) -> Vec<Found> {
    let angle = |rule| (Dimension::ANGLE, rule);
    match &revolve.extent {
        RevolveExtent::Full | RevolveExtent::UpTo { .. } => Vec::new(),
        RevolveExtent::OneSide { angle: turned, .. } => vec![found(
            ValueSlot::RevolveAngle,
            "Angle",
            turned,
            angle(Rule::Turn),
        )],
        RevolveExtent::Symmetric { angle: turned } => vec![found(
            ValueSlot::RevolveTotal,
            "Total angle",
            turned,
            angle(Rule::Turn),
        )],
        RevolveExtent::TwoSides { forward, backward } => vec![
            found(
                ValueSlot::RevolveForward,
                "Forward",
                forward,
                angle(Rule::TurnBeside(degrees(parameters, backward))),
            ),
            found(
                ValueSlot::RevolveBackward,
                "Backward",
                backward,
                angle(Rule::TurnBeside(degrees(parameters, forward))),
            ),
        ],
    }
}

fn direction_values(
    direction: &LinearDirection,
    [count, spacing]: [ValueSlot; 2],
    [count_caption, spacing_caption]: [&'static str; 2],
) -> [Found; 2] {
    [
        found(
            count,
            count_caption,
            &direction.count,
            (Dimension::NONE, Rule::Count),
        ),
        found(
            spacing,
            spacing_caption,
            &direction.spacing,
            (Dimension::LENGTH, Rule::AboveZeroOrReverse),
        ),
    ]
}

fn spacing_caption(direction: &LinearDirection, first: bool) -> &'static str {
    match (direction.measured, first) {
        (LinearSpacing::BetweenCopies, true) => "Spacing",
        (LinearSpacing::Total, true) => "Total length",
        (LinearSpacing::BetweenCopies, false) => "Second spacing",
        (LinearSpacing::Total, false) => "Second total length",
    }
}

fn values_of(parameters: &ParameterValues, kind: &FeatureKind) -> Vec<Found> {
    let length = |rule| (Dimension::LENGTH, rule);
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude_values(extrude),
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve_values(parameters, revolve),
        FeatureKind::Blend(blend) => blend_values(blend),
        FeatureKind::Shell(shell) => vec![found(
            ValueSlot::ShellThickness,
            "Thickness",
            &shell.thickness,
            length(Rule::AboveZero),
        )],
        FeatureKind::Hole(hole) => {
            let diameter = found(
                ValueSlot::HoleDiameter,
                "Diameter",
                &hole.diameter,
                length(Rule::AboveZero),
            );
            let depth = match &hole.depth {
                HoleDepth::Blind(depth) => Some(found(
                    ValueSlot::HoleDepth,
                    "Depth",
                    depth,
                    length(Rule::AboveZero),
                )),
                _ => None,
            };
            [Some(diameter), depth].into_iter().flatten().collect()
        }
        FeatureKind::Pattern(pattern) => match &pattern.kind {
            PatternKind::Linear { first, second } => {
                let mut values: Vec<Found> = direction_values(
                    first,
                    [ValueSlot::Count, ValueSlot::Spacing],
                    [COUNT, spacing_caption(first, true)],
                )
                .into_iter()
                .collect();
                if let Some(second) = second {
                    values.extend(direction_values(
                        second,
                        [ValueSlot::SecondCount, ValueSlot::SecondSpacing],
                        [SECOND_COUNT, spacing_caption(second, false)],
                    ));
                }
                values
            }
            PatternKind::Circular(circular) => vec![
                found(
                    ValueSlot::Count,
                    COUNT,
                    &circular.count,
                    (Dimension::NONE, Rule::Count),
                ),
                found(
                    ValueSlot::PatternAngle,
                    "Total angle",
                    &circular.angle,
                    (Dimension::ANGLE, Rule::Turn),
                ),
            ],
            PatternKind::Curve(_) | PatternKind::Points(_) => Vec::new(),
        },
        FeatureKind::OffsetFace(offset) => vec![found(
            ValueSlot::OffsetDistance,
            "Distance",
            &offset.distance,
            length(Rule::Any),
        )],
        FeatureKind::Primitive(primitive) => primitive_values(&primitive.shape),
        FeatureKind::Move(movement) => move_values(parameters, movement),
        FeatureKind::Thread(thread) => match &thread.length {
            ThreadLength::Depth(depth) => vec![found(
                ValueSlot::ThreadDepth,
                "Depth",
                depth,
                length(Rule::AboveZero),
            )],
            ThreadLength::Full => Vec::new(),
        },
        FeatureKind::Sketch(_)
        | FeatureKind::SplitFace(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Mate(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Split(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Datum(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Measurement(_) => Vec::new(),
    }
}

fn blend_values(blend: &Blend) -> Vec<Found> {
    let caption = match (blend.kind, blend.chamfer_form()) {
        (BlendKind::Fillet, _) => "Radius",
        (BlendKind::Chamfer, ChamferForm::TwoDistances { .. }) => "First distance",
        (BlendKind::Chamfer, ChamferForm::Equal | ChamferForm::DistanceAngle { .. }) => "Distance",
    };
    let size = found(
        ValueSlot::BlendSize,
        caption,
        &blend.size,
        (Dimension::LENGTH, Rule::AboveZero),
    );
    let second = match blend.chamfer_form() {
        ChamferForm::Equal => None,
        ChamferForm::TwoDistances { second } => Some(found(
            ValueSlot::ChamferSecondDistance,
            "Second distance",
            second,
            (Dimension::LENGTH, Rule::AboveZero),
        )),
        ChamferForm::DistanceAngle { angle } => Some(found(
            ValueSlot::ChamferAngle,
            "Angle",
            angle,
            (Dimension::ANGLE, Rule::ChamferAngle),
        )),
    };
    std::iter::once(size).chain(second).collect()
}

fn primitive_values(shape: &PrimitiveShape) -> Vec<Found> {
    shape
        .sizes()
        .into_iter()
        .zip(shape.rules())
        .filter_map(|((what, expression), rule)| {
            let size = PrimitiveSize::named(what)?;
            let checked = match rule {
                SizeRule::AboveZero => (Dimension::LENGTH, Rule::AboveZero),
                SizeRule::ZeroOrMore => (Dimension::LENGTH, Rule::ZeroOrMore),
                SizeRule::Sides => (Dimension::NONE, Rule::Sides),
            };
            Some(found(
                ValueSlot::Primitive(size),
                size.caption(),
                expression,
                checked,
            ))
        })
        .collect()
}

fn move_values(parameters: &ParameterValues, movement: &Move) -> Vec<Found> {
    MoveAxis::ALL
        .into_iter()
        .filter(|axis| {
            parameters
                .evaluate_expression(axis.of(&movement.offset))
                .is_ok_and(|value| value.value != 0.0)
        })
        .map(|axis| {
            found(
                ValueSlot::MoveOffset(axis),
                move_caption(axis),
                axis.of(&movement.offset),
                (Dimension::LENGTH, Rule::Any),
            )
        })
        .collect()
}

fn face_box(solid: &Solid, face: &Face) -> Option<Aabb> {
    face.loops()
        .iter()
        .filter_map(|id| solid.face_loop(*id))
        .flat_map(|found| found.coedges().iter())
        .filter_map(|coedge| solid.edge(solid.coedge(*coedge)?.edge()))
        .map(|edge| edge.curve().bounding_box(edge.interval()))
        .reduce(Aabb::union)
}

fn anchor_of(evaluation: &Evaluation, feature: FeatureId) -> Option<Point3> {
    let body = evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::solid)?;
    made_faces(&body.solid, feature)
        .map(|(_, bounds)| bounds)
        .reduce(Aabb::union)
        .or_else(|| body.bounding_box())
        .map(|bounds| bounds.center())
}

pub fn body_of(evaluation: &Evaluation, feature: FeatureId) -> Option<&Solid> {
    evaluation
        .feature(feature)?
        .result
        .as_deref()
        .and_then(FeatureResult::solid)
        .map(|body| &body.solid)
}

pub fn made_faces(solid: &Solid, feature: FeatureId) -> impl Iterator<Item = (&Face, Aabb)> {
    solid
        .faces()
        .filter(move |(_, face)| face.origin().map(origin_feature) == Some(feature))
        .filter_map(|(_, face)| Some((face, face_box(solid, face)?)))
}

pub fn sketch_of(kind: &FeatureKind) -> Option<FeatureId> {
    match kind {
        FeatureKind::Solid(solid) => Some(solid.sketch()),
        FeatureKind::Hole(hole) => Some(hole.sketch),
        _ => None,
    }
}

pub fn of(model: &Model, feature: FeatureId) -> Vec<FeatureValue> {
    let Some(owner) = model.document().feature(feature) else {
        return Vec::new();
    };
    let values = values_of(model.parameters(), &owner.kind);
    if values.is_empty() {
        return Vec::new();
    }
    let shapes = value_shapes::of(model, feature);
    let drawn_of = |slot: ValueSlot| {
        shapes
            .iter()
            .find(|(shaped, _)| *shaped == slot)
            .map_or(Drawn::Stacked, |(_, drawn)| drawn.clone())
    };
    let Some(anchor) = anchor_of(model.evaluation(), feature)
        .or_else(|| shapes.iter().find_map(|(_, drawn)| drawn.middle()))
    else {
        return Vec::new();
    };
    values
        .into_iter()
        .map(|value| {
            let drawn = drawn_of(value.slot);
            FeatureValue {
                feature,
                slot: value.slot,
                caption: value.caption,
                expression: value.expression,
                dimension: value.dimension,
                rule: value.rule,
                anchor: drawn.middle().unwrap_or(anchor),
                drawn,
            }
        })
        .collect()
}

fn set_end(end: &mut ExtrudeEnd, value: Expression) -> bool {
    match end {
        ExtrudeEnd::Distance(distance) => {
            *distance = value;
            true
        }
        ExtrudeEnd::ThroughAll
        | ExtrudeEnd::UpToNext { .. }
        | ExtrudeEnd::UpToFace { .. }
        | ExtrudeEnd::UpToSurface { .. } => false,
    }
}

fn set_extrude(extent: &mut ExtrudeExtent, reach: Reach, value: Expression) -> bool {
    match (extent, reach) {
        (ExtrudeExtent::OneSide { end, .. }, Reach::Only)
        | (ExtrudeExtent::TwoSides { forward: end, .. }, Reach::Forward)
        | (ExtrudeExtent::TwoSides { backward: end, .. }, Reach::Backward) => set_end(end, value),
        (ExtrudeExtent::Symmetric { distance }, Reach::Symmetric) => {
            *distance = value;
            true
        }
        _ => false,
    }
}

fn set_revolve(extent: &mut RevolveExtent, slot: ValueSlot, value: Expression) -> bool {
    let target = match (extent, slot) {
        (RevolveExtent::OneSide { angle, .. }, ValueSlot::RevolveAngle)
        | (RevolveExtent::Symmetric { angle }, ValueSlot::RevolveTotal)
        | (RevolveExtent::TwoSides { forward: angle, .. }, ValueSlot::RevolveForward)
        | (
            RevolveExtent::TwoSides {
                backward: angle, ..
            },
            ValueSlot::RevolveBackward,
        ) => angle,
        _ => return false,
    };
    *target = value;
    true
}

fn set_pattern(kind: &mut PatternKind, slot: ValueSlot, value: Expression) -> bool {
    let target = match (kind, slot) {
        (PatternKind::Linear { first, .. }, ValueSlot::Count) => &mut first.count,
        (PatternKind::Linear { first, .. }, ValueSlot::Spacing) => &mut first.spacing,
        (
            PatternKind::Linear {
                second: Some(second),
                ..
            },
            ValueSlot::SecondCount,
        ) => &mut second.count,
        (
            PatternKind::Linear {
                second: Some(second),
                ..
            },
            ValueSlot::SecondSpacing,
        ) => &mut second.spacing,
        (PatternKind::Circular(circular), ValueSlot::Count) => &mut circular.count,
        (PatternKind::Circular(circular), ValueSlot::PatternAngle) => &mut circular.angle,
        _ => return false,
    };
    *target = value;
    true
}

fn set_chamfer_second(blend: &mut Blend, slot: ValueSlot, value: Expression) -> bool {
    let fits = matches!(
        (&blend.form, slot),
        (
            ChamferForm::TwoDistances { .. },
            ValueSlot::ChamferSecondDistance
        ) | (ChamferForm::DistanceAngle { .. }, ValueSlot::ChamferAngle)
    );
    match blend.form.expression_mut().filter(|_| fits) {
        Some(target) => {
            *target = value;
            true
        }
        None => false,
    }
}

fn set_primitive(shape: &mut PrimitiveShape, size: PrimitiveSize, value: Expression) -> bool {
    let index = shape
        .sizes()
        .into_iter()
        .position(|(what, _)| PrimitiveSize::named(what) == Some(size));
    match index.and_then(|index| shape.sizes_mut().into_iter().nth(index)) {
        Some(target) => {
            *target = value;
            true
        }
        None => false,
    }
}

fn rebuilt(kind: &FeatureKind, slot: ValueSlot, value: Expression) -> Option<FeatureKind> {
    let mut changed = kind.clone();
    let set = match (&mut changed, slot) {
        (FeatureKind::Solid(SolidFeature::Extrude(extrude)), ValueSlot::Extrude(reach)) => {
            set_extrude(&mut extrude.extent, reach, value)
        }
        (FeatureKind::Solid(SolidFeature::Revolve(revolve)), _) => {
            set_revolve(&mut revolve.extent, slot, value)
        }
        (FeatureKind::Blend(blend), ValueSlot::BlendSize) => {
            blend.size = value;
            true
        }
        (FeatureKind::Shell(shell), ValueSlot::ShellThickness) => {
            shell.thickness = value;
            true
        }
        (FeatureKind::Hole(hole), ValueSlot::HoleDiameter) => {
            hole.diameter = value;
            hole.standard = None;
            true
        }
        (FeatureKind::Hole(hole), ValueSlot::HoleDepth) => match &mut hole.depth {
            HoleDepth::Blind(depth) => {
                *depth = value;
                true
            }
            _ => false,
        },
        (FeatureKind::Pattern(pattern), _) => set_pattern(&mut pattern.kind, slot, value),
        (FeatureKind::Blend(blend), ValueSlot::ChamferSecondDistance | ValueSlot::ChamferAngle) => {
            set_chamfer_second(blend, slot, value)
        }
        (FeatureKind::OffsetFace(offset), ValueSlot::OffsetDistance) => {
            offset.distance = value;
            true
        }
        (FeatureKind::Primitive(primitive), ValueSlot::Primitive(size)) => {
            set_primitive(&mut primitive.shape, size, value)
        }
        (FeatureKind::Move(movement), ValueSlot::MoveOffset(axis)) => {
            *axis.of_mut(&mut movement.offset) = value;
            true
        }
        (FeatureKind::Thread(thread), ValueSlot::ThreadDepth) => match &mut thread.length {
            ThreadLength::Depth(depth) => {
                *depth = value;
                true
            }
            ThreadLength::Full => false,
        },
        _ => false,
    };
    set.then_some(changed)
}

pub const GONE: &str = "The value is no longer there";

pub fn change(
    model: &Model,
    feature: FeatureId,
    slot: ValueSlot,
    value: Expression,
) -> Result<Transaction, String> {
    let document = model.document();
    let owner = document.feature(feature).ok_or_else(|| GONE.to_owned())?;
    let kind = rebuilt(&owner.kind, slot, value).ok_or_else(|| GONE.to_owned())?;
    field::checked(
        document,
        Transaction::single(
            format!("Edit {}", owner.name),
            Edit::SetFeatureKind { id: feature, kind },
        ),
    )
}

#[cfg(test)]
mod tests {
    use caditor_document::{Blend, Shell};
    use caditor_expression::Unit;

    use super::*;

    fn millimetres(value: f64) -> Expression {
        Expression::measure(value, Unit::Millimetre)
    }

    #[test]
    fn a_fillet_offers_its_radius_and_a_shell_its_thickness_each_set_back_into_its_kind() {
        let body = FeatureId::from_raw(1);
        let fillet = FeatureKind::Blend(Blend {
            kind: BlendKind::Fillet,
            body,
            edges: Vec::new(),
            size: millimetres(2.0),
            form: ChamferForm::Equal,
            flipped: false,
        });
        let shell = FeatureKind::Shell(Shell {
            body,
            open: Vec::new(),
            thickness: millimetres(1.0),
        });
        let parameters = ParameterValues::default();

        let radius = values_of(&parameters, &fillet);
        let thickness = values_of(&parameters, &shell);
        let rounder = rebuilt(&fillet, ValueSlot::BlendSize, millimetres(5.0)).unwrap();
        let thicker = rebuilt(&shell, ValueSlot::ShellThickness, millimetres(3.0)).unwrap();

        assert_eq!(radius.len(), 1);
        assert_eq!(radius[0].caption, "Radius");
        assert_eq!(thickness[0].caption, "Thickness");
        assert_eq!(
            rounder.blend().map(|blend| &blend.size),
            Some(&millimetres(5.0))
        );
        assert!(
            matches!(thicker, FeatureKind::Shell(shell) if shell.thickness == millimetres(3.0))
        );
        assert_eq!(
            rebuilt(&shell, ValueSlot::BlendSize, millimetres(3.0)),
            None
        );
    }
}
