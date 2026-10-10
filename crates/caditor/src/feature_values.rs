use caditor_document::{
    BlendKind, ChamferForm, Edit, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId,
    FeatureKind, FeatureResult, HoleDepth, LinearDirection, ParameterValues, PatternKind, Revolve,
    RevolveExtent, SolidFeature, Transaction, origin_feature,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Aabb, Point3};
use caditor_kernel::{Face, Solid};

use crate::{
    feature_fields::Rule, field, model::Model, move_manipulator::Reach, reach_handles, solid_panel,
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
    pub line: Option<[Point3; 2]>,
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
        (caditor_document::LinearSpacing::BetweenCopies, true) => "Spacing",
        (caditor_document::LinearSpacing::Total, true) => "Total length",
        (caditor_document::LinearSpacing::BetweenCopies, false) => "Second spacing",
        (caditor_document::LinearSpacing::Total, false) => "Second total length",
    }
}

fn values_of(parameters: &ParameterValues, kind: &FeatureKind) -> Vec<Found> {
    let length = |rule| (Dimension::LENGTH, rule);
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => extrude_values(extrude),
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => revolve_values(parameters, revolve),
        FeatureKind::Blend(blend) => {
            let caption = match (blend.kind, blend.chamfer_form()) {
                (BlendKind::Fillet, _) => "Radius",
                (BlendKind::Chamfer, ChamferForm::TwoDistances { .. }) => "First distance",
                (BlendKind::Chamfer, ChamferForm::Equal | ChamferForm::DistanceAngle { .. }) => {
                    "Distance"
                }
            };
            vec![found(
                ValueSlot::BlendSize,
                caption,
                &blend.size,
                length(Rule::AboveZero),
            )]
        }
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
        _ => Vec::new(),
    }
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
    let solid = &body.solid;
    solid
        .faces()
        .filter(|(_, face)| face.origin().map(origin_feature) == Some(feature))
        .filter_map(|(_, face)| face_box(solid, face))
        .reduce(Aabb::union)
        .or_else(|| body.bounding_box())
        .map(|bounds| bounds.center())
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
    let lines = reach_handles::reach_lines(model, feature);
    let Some(anchor) = anchor_of(model.evaluation(), feature)
        .or_else(|| lines.first().map(|(_, [from, to])| from.lerp(*to, 0.5)))
    else {
        return Vec::new();
    };
    values
        .into_iter()
        .map(|value| {
            let line = lines
                .iter()
                .find(|(reach, _)| ValueSlot::Extrude(*reach) == value.slot)
                .map(|(_, line)| *line);
            FeatureValue {
                feature,
                slot: value.slot,
                caption: value.caption,
                expression: value.expression,
                dimension: value.dimension,
                rule: value.rule,
                anchor: line.map_or(anchor, |[from, to]| from.lerp(to, 0.5)),
                line,
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
