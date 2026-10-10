use caditor_document::{
    Document, Edit, ExtrudeExtent, Feature, FeatureKind, PatternKind, PrimitiveAnchor,
    RevolveExtent, SolidFeature, Transaction,
};

use crate::field;

pub const NOTHING_TO_REVERSE: &str = "Only an extrusion, revolve, hole, primitive, pattern, mate \
                                      or split has a direction to reverse";

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
            }
            Ok(FeatureKind::Pattern(pattern))
        }
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
        | FeatureKind::Thread(_) => Err(NOTHING_TO_REVERSE.to_owned()),
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
