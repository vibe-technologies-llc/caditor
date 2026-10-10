use std::f64::consts::FRAC_PI_2;

use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point3, Rotation3, Vector3};
use caditor_render::Viewpoint;

use crate::{
    datum_tools, measure,
    model::Model,
    scene,
    selection::{Pickable, Selection},
    sketch_placement::{self, FaceChoice},
};

pub const NOTHING_TO_LOOK_AT: &str =
    "Select one face, plane, sketch or straight edge to look straight at it";
pub const SEVERAL_TO_LOOK_AT: &str =
    "Several things are selected; select only the face, plane, sketch or edge to look at";
pub const NO_DIRECTION: &str = "The selected item has no direction to look along; select a flat or round face, a plane, a sketch or a straight edge";
const FACING_SLACK: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LookTarget {
    Normal(Vector3),
    Plane(Plane),
    Axis(Vector3),
}

impl LookTarget {
    pub fn toward_eye(self, current: &Viewpoint) -> Vector3 {
        match self {
            Self::Normal(normal) => normal,
            Self::Plane(plane) => plane.normal(),
            Self::Axis(direction) if direction.dot(current.forward()) > 0.0 => -direction,
            Self::Axis(direction) => direction,
        }
    }

    pub fn viewpoint(self, current: &Viewpoint, target: Point3) -> Viewpoint {
        let toward_eye = self.toward_eye(current);
        if faces(current, toward_eye) {
            return quarter_turned(current, toward_eye, target);
        }
        let fresh = match self {
            Self::Plane(plane) => Some(Viewpoint::facing(&plane, target, current.distance)),
            Self::Normal(_) | Self::Axis(_) => {
                Viewpoint::looking_from(toward_eye, target, current.distance)
            }
        };
        fresh.unwrap_or(Viewpoint { target, ..*current })
    }
}

pub fn faces(viewpoint: &Viewpoint, toward_eye: Vector3) -> bool {
    let toward_eye = toward_eye.normalize_or_zero();
    (-viewpoint.forward()).dot(toward_eye) >= 1.0 - FACING_SLACK
}

pub fn quarter_turned(current: &Viewpoint, toward_eye: Vector3, target: Point3) -> Viewpoint {
    let turn = Rotation3::from_axis_angle(toward_eye.normalize_or_zero(), FRAC_PI_2);
    Viewpoint {
        target,
        orientation: (turn * current.orientation).normalize(),
        distance: current.distance,
    }
}

enum Candidate {
    Sketch(FeatureId),
    Other(Pickable),
}

fn candidate(pickable: Pickable) -> Candidate {
    match pickable {
        Pickable::SketchEntity { feature, .. }
        | Pickable::SketchRegion { feature, .. }
        | Pickable::SketchConstraint { feature, .. } => Candidate::Sketch(feature),
        other => Candidate::Other(other),
    }
}

pub fn look_target(model: &Model, selection: &Selection) -> Result<LookTarget, &'static str> {
    let mut sketch = None;
    let mut other = None;
    for pickable in selection.iter() {
        match candidate(pickable) {
            Candidate::Sketch(feature) => match sketch {
                Some(known) if known != feature => return Err(SEVERAL_TO_LOOK_AT),
                _ => sketch = Some(feature),
            },
            Candidate::Other(pickable) => match other {
                Some(_) => return Err(SEVERAL_TO_LOOK_AT),
                None => other = Some(pickable),
            },
        }
    }
    match (sketch, other) {
        (None, None) => Err(NOTHING_TO_LOOK_AT),
        (Some(_), Some(_)) => Err(SEVERAL_TO_LOOK_AT),
        (Some(sketch), None) => scene::sketch_plane(model.document(), model.evaluation(), sketch)
            .map(LookTarget::Plane)
            .ok_or(NO_DIRECTION),
        (None, Some(pickable)) => target_of(model, pickable),
    }
}

fn target_of(model: &Model, pickable: Pickable) -> Result<LookTarget, &'static str> {
    if let Some(face) = FaceChoice::of(pickable)
        && let Some(normal) = sketch_placement::outward_direction(model, face)
    {
        return Ok(LookTarget::Normal(normal));
    }
    let direction = measure::direction_of(model, pickable)
        .filter(|direction| direction.length_squared() > 0.0)
        .ok_or(NO_DIRECTION)?;
    match pickable {
        Pickable::Plane(_) | Pickable::FramePlane { .. } => Ok(LookTarget::Normal(direction)),
        Pickable::Datum(datum) if datum_tools::is_plane(model.document(), datum) => {
            Ok(LookTarget::Normal(direction))
        }
        Pickable::Face { .. }
        | Pickable::Edge { .. }
        | Pickable::Axis(_)
        | Pickable::FrameAxis { .. }
        | Pickable::Datum(_) => Ok(LookTarget::Axis(direction)),
        _ => Err(NO_DIRECTION),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top_view() -> Viewpoint {
        Viewpoint::looking_from(Vector3::Z, Point3::ZERO, 100.0).unwrap()
    }

    #[test]
    fn looking_again_at_what_the_view_faces_turns_it_a_quarter_turn_about_the_normal() {
        let current = top_view();

        let turned = LookTarget::Normal(Vector3::Z).viewpoint(&current, Point3::ZERO);
        let again = LookTarget::Normal(Vector3::Z).viewpoint(&turned, Point3::ZERO);

        assert!(faces(&turned, Vector3::Z));
        assert!(turned.up().dot(current.up()).abs() < 1e-9);
        assert!((turned.up() + current.right()).length() < 1e-9);
        assert!((again.up() + current.up()).length() < 1e-9);
    }

    #[test]
    fn an_axis_is_looked_along_from_the_side_the_view_is_on() {
        let current = top_view();
        let side =
            Viewpoint::looking_from(Vector3::new(1.0, 0.0, 0.2), Point3::ZERO, 100.0).unwrap();

        let along = LookTarget::Axis(-Vector3::Z).viewpoint(&side, Point3::ZERO);
        let turned = LookTarget::Axis(-Vector3::Z).viewpoint(&current, Point3::ZERO);

        assert!(faces(&along, Vector3::Z));
        assert!(faces(&turned, Vector3::Z));
        assert!((turned.up() + current.right()).length() < 1e-9);
    }

    #[test]
    fn a_plane_is_faced_with_its_own_axes_until_the_view_faces_it() {
        let plane = Plane::from_frame(Point3::ZERO, Vector3::X, Vector3::Y).unwrap();
        let current = top_view();

        let faced = LookTarget::Plane(plane).viewpoint(&current, Point3::ZERO);

        assert!(faces(&faced, Vector3::X));
        assert!((faced.right() - plane.x_axis()).length() < 1e-9);
        assert!((faced.up() - plane.y_axis()).length() < 1e-9);
    }
}
