use caditor_document::{
    BodyOperation, Document, Edit, Extrude, ExtrudeExtent, Feature, FeatureKind, ParameterValues,
    PatternKind, PrimitiveAnchor, RevolveExtent, SolidFeature, Transaction,
};
use caditor_expression::Expression;

use crate::{coil_tools, field, model::Notice, solid_tools};

pub const NOTHING_TO_REVERSE: &str = "Only an extrusion, revolve, hole, primitive, pattern, mate \
                                      or split has a direction to reverse";
const MATE_HAS_NO_SIDE: &str = "sets an angle or puts a point in place, so it has no side to \
                                flip; edit its angle or what it mates onto instead";
const POINTS_HAVE_NO_DIRECTION: &str =
    "places its copies at the points of a sketch, so it has no direction to reverse";

fn refusal(feature: &Feature, reason: &str) -> String {
    format!("{} {reason}", feature.name)
}

pub fn reversed(feature: &Feature) -> Result<FeatureKind, String> {
    match &feature.kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
            let mut extrude = extrude.clone();
            match &mut extrude.extent {
                ExtrudeExtent::OneSide { reversed, .. } => *reversed = !*reversed,
                ExtrudeExtent::Symmetric { .. } | ExtrudeExtent::TwoSides { .. } => {
                    return Err(refusal(
                        feature,
                        "runs both ways from its sketch; set its Extent to One side to reverse it",
                    ));
                }
            }
            Ok(FeatureKind::Solid(SolidFeature::Extrude(extrude)))
        }
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => {
            let mut revolve = revolve.clone();
            match &mut revolve.extent {
                RevolveExtent::OneSide { reversed, .. } | RevolveExtent::UpTo { reversed, .. } => {
                    *reversed = !*reversed;
                }
                RevolveExtent::Full => {
                    return Err(refusal(
                        feature,
                        "turns a full turn, so it has no direction to reverse",
                    ));
                }
                RevolveExtent::Symmetric { .. } | RevolveExtent::TwoSides { .. } => {
                    return Err(refusal(
                        feature,
                        "turns both ways from its sketch; set its Extent to One side or Up to \
                         face to reverse it",
                    ));
                }
            }
            Ok(FeatureKind::Solid(SolidFeature::Revolve(revolve)))
        }
        FeatureKind::Solid(SolidFeature::Sweep(_)) => Err(refusal(
            feature,
            "follows its path from the profile, so it has no direction to reverse",
        )),
        FeatureKind::Coil(coil) => Ok(FeatureKind::Coil(coil_tools::reversed(coil))),
        FeatureKind::Pipe(_) => Err(refusal(
            feature,
            "follows its path, so it has no direction to reverse",
        )),
        FeatureKind::Loft(_) => Err(refusal(
            feature,
            "runs through its profiles in their order, so it has no direction to reverse",
        )),
        FeatureKind::Hole(hole) => {
            let mut hole = hole.clone();
            hole.reversed = !hole.reversed;
            Ok(FeatureKind::Hole(hole))
        }
        FeatureKind::Primitive(primitive) => {
            if primitive.anchor == PrimitiveAnchor::Centre {
                return Err(refusal(
                    feature,
                    "is centred on its place, so it grows both ways; set Starts at to Corner or \
                     Base centre to reverse it",
                ));
            }
            let mut primitive = primitive.clone();
            primitive.reversed = !primitive.reversed;
            Ok(FeatureKind::Primitive(primitive))
        }
        FeatureKind::Pattern(pattern) => {
            let mut pattern = pattern.clone();
            match &mut pattern.kind {
                PatternKind::Linear { first, .. } => first.reversed = !first.reversed,
                PatternKind::Circular(circular) => circular.reversed = !circular.reversed,
                PatternKind::Curve(curve) => curve.reversed = !curve.reversed,
                PatternKind::Points(_) => {
                    return Err(refusal(feature, POINTS_HAVE_NO_DIRECTION));
                }
            }
            Ok(FeatureKind::Pattern(pattern))
        }
        FeatureKind::Mate(mate) if !mate.flips() => Err(refusal(feature, MATE_HAS_NO_SIDE)),
        FeatureKind::Mate(mate) => {
            let mut mate = mate.clone();
            mate.flipped = !mate.flipped;
            Ok(FeatureKind::Mate(mate))
        }
        FeatureKind::Split(split) => {
            let mut split = split.clone();
            split.flipped = !split.flipped;
            Ok(FeatureKind::Split(split))
        }
        FeatureKind::Sketch(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::SplitFace(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Move(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Datum(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Thread(_)
        | FeatureKind::Measurement(_)
        | FeatureKind::ReferenceImage(_) => Err(NOTHING_TO_REVERSE.to_owned()),
    }
}

pub fn reverse_change(document: &Document, feature: &Feature) -> Result<Transaction, String> {
    let kind = reversed(feature)?;
    field::checked(
        document,
        Transaction::single(
            format!("Reverse {}", feature.name),
            Edit::SetFeatureKind {
                id: feature.id(),
                kind,
            },
        ),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flip {
    Reversed,
    ToRemove,
    ToAdd,
}

impl Flip {
    pub fn between(before: &Extrude, after: &Extrude) -> Option<Self> {
        let direction = |extrude: &Extrude| match extrude.extent {
            ExtrudeExtent::OneSide { reversed, .. } => Some(reversed),
            ExtrudeExtent::Symmetric { .. } | ExtrudeExtent::TwoSides { .. } => None,
        };
        let (Some(was), Some(is)) = (direction(before), direction(after)) else {
            return None;
        };
        if was == is {
            return None;
        }
        Some(match (before.operation, after.operation) {
            (BodyOperation::Add(_), BodyOperation::Remove(_)) => Self::ToRemove,
            (BodyOperation::Remove(_), BodyOperation::Add(_)) => Self::ToAdd,
            _ => Self::Reversed,
        })
    }

    pub fn words(self) -> &'static str {
        match self {
            Self::Reversed => "reversed",
            Self::ToRemove => "reversed and switched to Remove",
            Self::ToAdd => "reversed and switched to Add",
        }
    }

    fn meaning(self) -> &'static str {
        match self {
            Self::Reversed => "goes the other way",
            Self::ToRemove => "cuts the other way",
            Self::ToAdd => "adds the other way",
        }
    }
}

pub fn negated(expression: Expression) -> Expression {
    match expression {
        Expression::Negate(inner) if !matches!(*inner, Expression::Negate(_)) => *inner,
        Expression::Number(value) if value < 0.0 => Expression::number(-value),
        Expression::Measure(value, unit) if value < 0.0 => Expression::measure(-value, unit),
        other => Expression::Negate(Box::new(other)),
    }
}

pub fn unsigned(parameters: &ParameterValues, distance: Expression) -> (Expression, bool) {
    let negative = parameters
        .evaluate_expression(&distance)
        .is_ok_and(|value| value.value < 0.0);
    if negative {
        (negated(distance), true)
    } else {
        (distance, false)
    }
}

pub fn flipped_extrude(extrude: &Extrude) -> Extrude {
    let mut flipped = extrude.clone();
    let ExtrudeExtent::OneSide { reversed, .. } = &mut flipped.extent else {
        return flipped;
    };
    *reversed = !*reversed;
    let operation = match extrude.operation {
        BodyOperation::Add(body) => BodyOperation::Remove(body),
        BodyOperation::Remove(body) => BodyOperation::Add(body),
        BodyOperation::NewBody | BodyOperation::Intersect(_) => return flipped,
    };
    match solid_tools::with_operation(&SolidFeature::Extrude(flipped), operation) {
        SolidFeature::Extrude(switched) => switched,
        SolidFeature::Revolve(_) | SolidFeature::Sweep(_) => extrude.clone(),
    }
}

pub fn flipping_solid(solid: SolidFeature, negative: bool) -> SolidFeature {
    match solid {
        SolidFeature::Extrude(extrude) if negative => {
            SolidFeature::Extrude(flipped_extrude(&extrude))
        }
        other => other,
    }
}

pub fn flipping(kind: FeatureKind, negative: bool) -> FeatureKind {
    match kind {
        FeatureKind::Solid(solid) => FeatureKind::Solid(flipping_solid(solid, negative)),
        other => other,
    }
}

pub fn announcement(document: &Document, transaction: &Transaction) -> Option<Notice> {
    transaction.edits().iter().find_map(|edit| {
        let Edit::SetFeatureKind {
            id,
            kind: FeatureKind::Solid(SolidFeature::Extrude(after)),
        } = edit
        else {
            return None;
        };
        let owner = document.feature(*id)?;
        let Some(SolidFeature::Extrude(before)) = owner.kind.solid() else {
            return None;
        };
        let flip = Flip::between(before, after)?;
        Some(Notice::info(format!(
            "{}: {}; a negative distance {}",
            owner.name,
            flip.words(),
            flip.meaning()
        )))
    })
}

#[cfg(test)]
mod tests {
    use caditor_document::{FeatureId, RegionChoice};
    use caditor_expression::Unit;

    use super::*;

    fn extrude(extent: ExtrudeExtent, operation: BodyOperation) -> Extrude {
        Extrude {
            sketch: FeatureId::from_raw(1),
            regions: RegionChoice::All,
            extent,
            operation,
            start: None,
            other_bodies: Vec::new(),
            taper: None,
            wall: None,
            direction: None,
        }
    }

    fn one_side(reversed: bool) -> ExtrudeExtent {
        ExtrudeExtent::one_side(Expression::measure(5.0, Unit::Millimetre), reversed)
    }

    fn reversed_of(extrude: &Extrude) -> Option<bool> {
        match extrude.extent {
            ExtrudeExtent::OneSide { reversed, .. } => Some(reversed),
            ExtrudeExtent::Symmetric { .. } | ExtrudeExtent::TwoSides { .. } => None,
        }
    }

    #[test]
    fn negating_drops_a_leading_minus_and_keeps_everything_else_referencing_its_parameters() {
        let depth = Expression::Parameter(caditor_expression::ParameterId::from_raw(1));
        let difference = Expression::binary(
            caditor_expression::BinaryOperator::Subtract,
            depth.clone(),
            Expression::measure(30.0, Unit::Millimetre),
        );
        let doubled_minus =
            Expression::Negate(Box::new(Expression::Negate(Box::new(depth.clone()))));

        assert_eq!(
            negated(Expression::measure(-5.0, Unit::Millimetre)),
            Expression::measure(5.0, Unit::Millimetre)
        );
        assert_eq!(negated(Expression::Negate(Box::new(depth.clone()))), depth);
        assert_eq!(
            negated(difference.clone()),
            Expression::Negate(Box::new(difference))
        );
        assert_eq!(
            negated(doubled_minus.clone()),
            Expression::Negate(Box::new(doubled_minus))
        );
        assert_eq!(negated(Expression::Number(-2.0)), Expression::Number(2.0));
    }

    #[test]
    fn a_distance_is_unsigned_only_when_it_evaluates_negative() {
        let parameters = ParameterValues::default();
        let minus = Expression::measure(-5.0, Unit::Millimetre);
        let plus = Expression::measure(5.0, Unit::Millimetre);

        assert_eq!(unsigned(&parameters, minus), (plus.clone(), true));
        assert_eq!(unsigned(&parameters, plus.clone()), (plus, false));
    }

    #[test]
    fn flipping_adds_becomes_removes_and_back_and_reverses_the_direction() {
        let body = FeatureId::from_raw(2);
        let adding = extrude(one_side(false), BodyOperation::Add(body));
        let removing = extrude(one_side(true), BodyOperation::Remove(body));

        let cut = flipped_extrude(&adding);
        let grown = flipped_extrude(&removing);

        assert_eq!(cut.operation, BodyOperation::Remove(body));
        assert_eq!(reversed_of(&cut), Some(true));
        assert_eq!(grown.operation, BodyOperation::Add(body));
        assert_eq!(reversed_of(&grown), Some(false));
        assert_eq!(Flip::between(&adding, &cut), Some(Flip::ToRemove));
        assert_eq!(Flip::between(&removing, &grown), Some(Flip::ToAdd));
    }

    #[test]
    fn flipping_a_new_body_or_an_intersection_only_reverses_it() {
        let body = FeatureId::from_raw(2);
        let new_body = extrude(one_side(false), BodyOperation::NewBody);
        let intersecting = extrude(one_side(false), BodyOperation::Intersect(body));

        let reversed_new = flipped_extrude(&new_body);
        let reversed_intersection = flipped_extrude(&intersecting);

        assert_eq!(reversed_new.operation, BodyOperation::NewBody);
        assert_eq!(reversed_of(&reversed_new), Some(true));
        assert_eq!(
            reversed_intersection.operation,
            BodyOperation::Intersect(body)
        );
        assert_eq!(reversed_of(&reversed_intersection), Some(true));
        assert_eq!(
            Flip::between(&new_body, &reversed_new),
            Some(Flip::Reversed)
        );
    }

    #[test]
    fn a_symmetric_or_two_sided_extrusion_is_never_flipped() {
        let body = FeatureId::from_raw(2);
        let five = Expression::measure(5.0, Unit::Millimetre);
        let symmetric = extrude(
            ExtrudeExtent::Symmetric {
                distance: five.clone(),
            },
            BodyOperation::Add(body),
        );
        let two_sided = extrude(
            ExtrudeExtent::two_sides(five.clone(), five),
            BodyOperation::Add(body),
        );

        assert_eq!(flipped_extrude(&symmetric), symmetric);
        assert_eq!(flipped_extrude(&two_sided), two_sided);
        assert_eq!(Flip::between(&symmetric, &two_sided), None);
    }

    #[test]
    fn an_edit_that_leaves_the_direction_alone_is_not_a_flip() {
        let body = FeatureId::from_raw(2);
        let before = extrude(one_side(false), BodyOperation::Add(body));
        let after = extrude(one_side(false), BodyOperation::Remove(body));

        assert_eq!(Flip::between(&before, &after), None);
    }
}
