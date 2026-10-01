use std::f64::consts::{PI, TAU};

use caditor_geometry::{Point2, Point3};

use super::{
    EdgeGeometry, Sweep,
    section::{Blend, SectionCurve},
};
use crate::{
    tolerance::LINEAR_RESOLUTION,
    topology::{EdgeId, Solid},
};

const FOOT_SAMPLES: usize = 8;
const CLEARANCE: f64 = 10.0 * LINEAR_RESOLUTION;

struct Placed<'a> {
    geometry: &'a EdgeGeometry,
    blend: &'a Blend,
}

impl Placed<'_> {
    fn section_point(&self, point: Point3) -> Point2 {
        let frame = &self.geometry.frame;
        match self.geometry.sweep {
            Sweep::Along { .. } => frame.to_local(point),
            Sweep::Around { .. } => {
                let offset = point - frame.origin();
                Point2::new(
                    offset.dot(frame.x_axis()).hypot(offset.dot(frame.normal())),
                    offset.dot(frame.y_axis()),
                )
            }
        }
    }

    fn within_extent(&self, point: Point3) -> bool {
        let frame = &self.geometry.frame;
        match self.geometry.sweep {
            Sweep::Along { length } => {
                let along = frame.signed_distance(point);
                along >= -CLEARANCE && along <= length + CLEARANCE
            }
            Sweep::Around { angle, closed } => {
                let offset = point - frame.origin();
                let turned = (-offset.dot(frame.normal()))
                    .atan2(offset.dot(frame.x_axis()))
                    .rem_euclid(TAU);
                closed || turned <= angle + CLEARANCE || turned >= TAU - CLEARANCE
            }
        }
    }

    fn covers(&self, side: usize, point: Point3) -> bool {
        let (Some(section_side), Some(foot)) = (
            self.geometry.section.sides.get(side),
            self.blend.feet.get(side),
        ) else {
            return false;
        };
        if !self.within_extent(point) {
            return false;
        }
        let corner = self.geometry.section.corner;
        let at = self.section_point(point);
        let (travelled, total) = match section_side.curve {
            SectionCurve::Line => {
                let reach = *foot - corner;
                let length = reach.length();
                if length <= CLEARANCE {
                    return false;
                }
                ((at - corner).dot(reach) / length, length)
            }
            SectionCurve::Circle { center, radius } => {
                let angle_to = |target: Point2| (corner - center).angle_to(target - center);
                let total = angle_to(*foot);
                let travelled = angle_to(at);
                if total * travelled <= 0.0 || travelled.abs() > PI {
                    return false;
                }
                (travelled * radius, total.abs() * radius)
            }
        };
        let travelled = travelled.abs();
        travelled > CLEARANCE && travelled < total - CLEARANCE
    }
}

fn share_a_vertex(solid: &Solid, first: EdgeId, second: EdgeId) -> bool {
    let (Some(first), Some(second)) = (solid.edge(first), solid.edge(second)) else {
        return false;
    };
    [first.start(), first.end()]
        .iter()
        .any(|vertex| *vertex == second.start() || *vertex == second.end())
}

pub(super) fn crossing(solid: &Solid, planned: &[(EdgeGeometry, Blend)]) -> Option<EdgeId> {
    for (index, (geometry, blend)) in planned.iter().enumerate() {
        for (other, (other_geometry, other_blend)) in planned.iter().enumerate() {
            if other == index || share_a_vertex(solid, geometry.edge, other_geometry.edge) {
                continue;
            }
            let neighbour = Placed {
                geometry: other_geometry,
                blend: other_blend,
            };
            for (side, face) in geometry.faces.iter().enumerate() {
                let Some(other_side) = other_geometry
                    .faces
                    .iter()
                    .position(|candidate| candidate == face)
                else {
                    continue;
                };
                let Some(foot) = blend.feet.get(side) else {
                    continue;
                };
                let crosses = (1..FOOT_SAMPLES)
                    .filter_map(|step| geometry.place(*foot, step as f64 / FOOT_SAMPLES as f64))
                    .any(|point| neighbour.covers(other_side, point));
                if crosses {
                    return Some(geometry.edge);
                }
            }
        }
    }
    None
}
