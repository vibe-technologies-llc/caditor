use caditor_document::{
    FeatureId, FeatureKind, Hole, HoleDepth, OffsetFace, SolidFeature, face_plane, hole_centres,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Point3, Vector3};

use crate::{
    feature_values::{self, ValueSlot},
    hole_tools,
    model::Model,
    reach_handles, scene,
    turn_handles::{self, TurnEnd},
};

#[derive(Debug, Clone, PartialEq)]
pub enum Drawn {
    Stacked,
    Path(Vec<Point3>),
    Leader(Point3),
}

impl Drawn {
    pub fn middle(&self) -> Option<Point3> {
        match self {
            Self::Stacked => None,
            Self::Path(points) => halfway(points),
            Self::Leader(at) => Some(*at),
        }
    }
}

fn halfway(points: &[Point3]) -> Option<Point3> {
    let legs = || {
        points
            .windows(2)
            .filter_map(|pair| Some((*pair.first()?, *pair.get(1)?)))
    };
    let total: f64 = legs().map(|(from, to)| from.distance(to)).sum();
    let mut left = total / 2.0;
    for (from, to) in legs() {
        let length = from.distance(to);
        if length >= left && length > 0.0 {
            return Some(from.lerp(to, left / length));
        }
        left -= length;
    }
    points.first().copied()
}

fn length(model: &Model, expression: &Expression) -> Option<f64> {
    let parameters = model.parameters();
    expression
        .evaluate_as(Dimension::LENGTH, &|id| parameters.value(id))
        .ok()
        .filter(|value| value.is_finite())
}

fn turn_slot(end: TurnEnd) -> ValueSlot {
    match end {
        TurnEnd::Only => ValueSlot::RevolveAngle,
        TurnEnd::Symmetric => ValueSlot::RevolveTotal,
        TurnEnd::Forward => ValueSlot::RevolveForward,
        TurnEnd::Backward => ValueSlot::RevolveBackward,
    }
}

struct HoleTop {
    top: Point3,
    down: Vector3,
    across: Vector3,
}

fn hole_top(model: &Model, hole: &Hole) -> Option<HoleTop> {
    let document = model.document();
    let displayed = model.displayed_sketch(document.feature(hole.sketch)?)?;
    let (_, centre) = hole_centres(&displayed).into_iter().next()?;
    let plane = scene::sketch_plane(document, model.evaluation(), hole.sketch)?;
    let down = if hole.reversed {
        plane.normal()
    } else {
        -plane.normal()
    };
    Some(HoleTop {
        top: plane.to_world(centre),
        down,
        across: plane.x_axis(),
    })
}

fn hole_shapes(model: &Model, hole: &Hole) -> Vec<(ValueSlot, Drawn)> {
    let Some(HoleTop { top, down, across }) = hole_top(model, hole) else {
        return Vec::new();
    };
    let rim = length(model, &hole.diameter).map(|diameter| {
        let half = across * diameter / 2.0;
        (
            ValueSlot::HoleDiameter,
            Drawn::Path(vec![top - half, top + half]),
        )
    });
    let depth = match &hole.depth {
        HoleDepth::Blind(depth) => length(model, depth).map(|depth| {
            (
                ValueSlot::HoleDepth,
                Drawn::Path(vec![top, top + down * depth]),
            )
        }),
        _ => None,
    };
    rim.into_iter().chain(depth).collect()
}

fn offset_shape(model: &Model, feature: FeatureId, offset: &OffsetFace) -> Option<Drawn> {
    let before = model.evaluation().body_before(feature)?;
    let solid = &before.solid()?.solid;
    let face = offset.faces.first()?.resolve(solid).ok()?;
    let plane = face_plane(solid, face)?;
    let middle = plane.to_world(hole_tools::middle_of(solid, face, &plane)?);
    let distance = length(model, &offset.distance)?;
    Some(Drawn::Path(vec![
        middle,
        middle + plane.normal() * distance,
    ]))
}

fn face_point(model: &Model, feature: FeatureId) -> Option<Point3> {
    let solid = feature_values::body_of(model.evaluation(), feature)?;
    let (face, bounds) = feature_values::made_faces(solid, feature)
        .max_by(|(_, a), (_, b)| a.diagonal().total_cmp(&b.diagonal()))?;
    let surface = face.surface();
    let at = surface.point_at(surface.project(bounds.center(), None));
    at.is_finite().then_some(at)
}

fn leaders(model: &Model, feature: FeatureId, slots: &[ValueSlot]) -> Vec<(ValueSlot, Drawn)> {
    face_point(model, feature)
        .map(|at| {
            slots
                .iter()
                .map(|slot| (*slot, Drawn::Leader(at)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn of(model: &Model, feature: FeatureId) -> Vec<(ValueSlot, Drawn)> {
    let Some(owner) = model.document().feature(feature) else {
        return Vec::new();
    };
    match &owner.kind {
        FeatureKind::Solid(SolidFeature::Extrude(_)) => reach_handles::reach_lines(model, feature)
            .into_iter()
            .map(|(reach, [from, to])| (ValueSlot::Extrude(reach), Drawn::Path(vec![from, to])))
            .collect(),
        FeatureKind::Solid(SolidFeature::Revolve(_)) => turn_handles::turn_arcs(model, feature)
            .into_iter()
            .map(|(end, points)| (turn_slot(end), Drawn::Path(points)))
            .collect(),
        FeatureKind::Hole(hole) => hole_shapes(model, hole),
        FeatureKind::Blend(_) => leaders(
            model,
            feature,
            &[
                ValueSlot::BlendSize,
                ValueSlot::ChamferSecondDistance,
                ValueSlot::ChamferAngle,
            ],
        ),
        FeatureKind::Shell(_) => leaders(model, feature, &[ValueSlot::ShellThickness]),
        FeatureKind::OffsetFace(offset) => offset_shape(model, feature, offset)
            .map(|drawn| vec![(ValueSlot::OffsetDistance, drawn)])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_of_a_path_is_halfway_along_its_length() {
        let path = vec![
            Point3::ZERO,
            Point3::new(2.0, 0.0, 0.0),
            Point3::new(2.0, 6.0, 0.0),
        ];

        let middle = Drawn::Path(path).middle().unwrap();

        assert!((middle - Point3::new(2.0, 2.0, 0.0)).length() < 1e-12);
        assert_eq!(Drawn::Stacked.middle(), None);
        assert_eq!(Drawn::Leader(Point3::X).middle(), Some(Point3::X));
    }
}
