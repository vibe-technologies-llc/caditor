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
    let mut kind = feature.kind.clone();
    match &mut kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => match &mut extrude.extent {
            ExtrudeExtent::OneSide { reversed, .. } => *reversed = !*reversed,
            ExtrudeExtent::Symmetric { .. } | ExtrudeExtent::TwoSides { .. } => {
                return Err(refusal(
                    feature,
                    "runs both ways from its sketch; set its Extent to One side to reverse it",
                ));
            }
        },
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => match &mut revolve.extent {
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
                    "turns both ways from its sketch; set its Extent to One side or Up to face \
                     to reverse it",
                ));
            }
        },
        FeatureKind::Hole(hole) => hole.reversed = !hole.reversed,
        FeatureKind::Primitive(primitive) => {
            if primitive.anchor == PrimitiveAnchor::Centre {
                return Err(refusal(
                    feature,
                    "is centred on its place, so it grows both ways; set Starts at to Corner or \
                     Base centre to reverse it",
                ));
            }
            primitive.reversed = !primitive.reversed;
        }
        FeatureKind::Pattern(pattern) => match &mut pattern.kind {
            PatternKind::Linear { first, .. } => first.reversed = !first.reversed,
            PatternKind::Circular(circular) => circular.reversed = !circular.reversed,
        },
        FeatureKind::Mate(mate) => mate.flipped = !mate.flipped,
        FeatureKind::Split(split) => split.flipped = !split.flipped,
        FeatureKind::Sketch(_)
        | FeatureKind::Blend(_)
        | FeatureKind::Shell(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Move(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Scale(_)
        | FeatureKind::Datum(_)
        | FeatureKind::Import(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Thread(_) => return Err(NOTHING_TO_REVERSE.to_owned()),
    }
    Ok(kind)
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
