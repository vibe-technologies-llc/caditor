use std::f64::consts::TAU;

use caditor_geometry::{Maths, Point2, Point3, Vector3};

use super::{Own, Scene, Track, trace::on_contour};
use crate::{
    interrupt::{self, Interrupted},
    intersect::line_window,
    sense::Sense,
    surface::Surface,
    tally,
    topology::{CoedgeId, EdgeId, FaceId, RayView, Solid},
};

const ALONG_FACE: f64 = 1e-9;
const FLAT_CURVATURE: f64 = 1e-9;
const SINGULAR_METRIC: f64 = 1e-18;
const EDGE_ALONG_VIEW: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Visible,
    Hidden,
    Undecided,
}

fn bend_along(surface: &Surface, sense: Sense, uv: Point2, direction: Vector3) -> Option<f64> {
    let at = surface.evaluate(uv.x, uv.y);
    let normal = surface.normal(uv.x, uv.y)? * sense.sign();
    let (first, shared, second) = (
        at.du.length_squared(),
        at.du.dot(at.dv),
        at.dv.length_squared(),
    );
    let metric = first * second - shared * shared;
    if metric <= SINGULAR_METRIC {
        return None;
    }
    let (over_u, over_v) = (direction.dot(at.du), direction.dot(at.dv));
    let a = (over_u * second - over_v * shared) / metric;
    let b = (over_v * first - over_u * shared) / metric;
    Some((at.duu * (a * a) + at.duv * (2.0 * a * b) + at.dvv * (b * b)).dot(normal))
}

struct Side<'a> {
    surface: &'a Surface,
    sense: Sense,
    uv: Point2,
    normal: Vector3,
    inward: Vector3,
}

fn side_of<'a>(
    solid: &'a Solid,
    coedge: CoedgeId,
    parameter: f64,
    point: Point3,
) -> Option<Side<'a>> {
    let data = solid.coedge(coedge)?;
    let edge = solid.edge(data.edge())?;
    let face = solid.face(solid.coedge_face(coedge)?)?;
    let surface = face.surface();
    let uv = surface.project(point, Some(data.pcurve().uv_at(parameter)));
    let normal = surface.normal(uv.x, uv.y)? * face.sense().sign();
    let tangent = edge.curve().evaluate(parameter).first * data.sense().sign();
    Some(Side {
        surface,
        sense: face.sense(),
        uv,
        normal,
        inward: normal.cross(tangent).try_normalize()?,
    })
}

fn runs_into_material(side: &Side<'_>, toward: Vector3) -> bool {
    bend_along(side.surface, side.sense, side.uv, toward).is_none_or(|bend| bend >= -FLAT_CURVATURE)
}

fn edge_hides_itself(
    solid: &Solid,
    edge: EdgeId,
    parameter: f64,
    point: Point3,
    toward: Vector3,
) -> Option<bool> {
    let data = solid.edge(edge)?;
    let [first, second] = data.coedges() else {
        return None;
    };
    let one = side_of(solid, *first, parameter, point)?;
    let two = side_of(solid, *second, parameter, point)?;
    let tangent = data.curve().evaluate(parameter).first.try_normalize()?;
    let square = toward - tangent * toward.dot(tangent);
    if square.length() <= EDGE_ALONG_VIEW {
        return None;
    }
    let turned = |direction: Vector3| {
        (-direction
            .dot(one.normal)
            .atan2_libm(direction.dot(one.inward)))
        .rem_euclid(TAU)
    };
    let seen = turned(square);
    let material = turned(two.inward);
    if seen <= ALONG_FACE || TAU - seen <= ALONG_FACE {
        return Some(runs_into_material(&one, toward));
    }
    if (seen - material).abs() <= ALONG_FACE {
        return Some(runs_into_material(&two, toward));
    }
    Some(seen < material)
}

fn contour_hides_itself(
    solid: &Solid,
    face: FaceId,
    start: Point3,
    sampled: bool,
    toward: Vector3,
) -> Option<(Point3, bool)> {
    let definition = solid.face(face)?;
    let surface = definition.surface();
    let foot = surface.project(start, None);
    let uv = if sampled {
        on_contour(surface, foot, toward).unwrap_or(foot)
    } else {
        foot
    };
    let hidden = bend_along(surface, definition.sense(), uv, toward)
        .is_some_and(|bend| bend > FLAT_CURVATURE);
    Some((surface.point_at(uv), hidden))
}

pub(super) fn covered_along(
    scene: &Scene<'_>,
    origin: Point3,
    direction: Vector3,
    own_solid: Option<usize>,
    own: &[FaceId],
) -> RayView {
    let mut doubtful = false;
    let reaching = scene.tree.matching(|bounds| {
        line_window(origin, direction, bounds).is_some_and(|window| window.end() >= 0.0)
    });
    for index in reaching {
        let Some(view) = scene.views.get(index) else {
            continue;
        };
        let faces = if own_solid == Some(index) { own } else { &[] };
        match view.classifier.ray_view(origin, direction, faces) {
            RayView::Covered => return RayView::Covered,
            RayView::Clear => {}
            RayView::Undecided => doubtful = true,
        }
    }
    if doubtful {
        RayView::Undecided
    } else {
        RayView::Clear
    }
}

pub(super) fn verdict(
    scene: &Scene<'_>,
    track: &Track,
    parameter: f64,
    toward: Vector3,
) -> Result<Verdict, Interrupted> {
    interrupt::check()?;
    tally::add(1);
    let mut point = track.curve.point(parameter);
    let Some(own_solid) = scene.views.get(track.solid) else {
        return Ok(Verdict::Undecided);
    };
    let hidden_locally = match track.own {
        Own::Edge(edge) => {
            edge_hides_itself(own_solid.solid, edge, parameter, point, toward).unwrap_or(false)
        }
        Own::Contour(face) => {
            match contour_hides_itself(own_solid.solid, face, point, track.sampled, toward) {
                Some((on_surface, hidden)) => {
                    point = on_surface;
                    hidden
                }
                None => false,
            }
        }
    };
    if hidden_locally {
        return Ok(Verdict::Hidden);
    }
    let seen = covered_along(scene, point, toward, Some(track.solid), &track.faces);
    interrupt::check()?;
    Ok(match seen {
        RayView::Covered => Verdict::Hidden,
        RayView::Clear => Verdict::Visible,
        RayView::Undecided => Verdict::Undecided,
    })
}
