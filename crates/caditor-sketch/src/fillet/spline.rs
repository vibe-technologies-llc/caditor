use caditor_geometry::{Point2, Vector2};

use crate::{curve::BSpline, intersect::closest_spline_parameter};

pub(super) fn parameter_after(corner_at_start: bool, turned: f64) -> f64 {
    if corner_at_start {
        turned
    } else {
        1.0 - turned
    }
}

pub(super) fn turned_to(corner_at_start: bool, parameter: f64) -> f64 {
    if corner_at_start {
        parameter
    } else {
        1.0 - parameter
    }
}

pub(super) fn point_after(spline: &BSpline, corner_at_start: bool, turned: f64) -> Point2 {
    spline.point_at(parameter_after(corner_at_start, turned).clamp(0.0, 1.0))
}

pub(super) fn tangent_at(spline: &BSpline, parameter: f64) -> Option<Vector2> {
    let [tangent, _] = spline.derivatives(parameter.clamp(0.0, 1.0));
    tangent.try_normalize()
}

pub(super) fn leaving(spline: &BSpline, corner_at_start: bool) -> Option<Vector2> {
    if corner_at_start {
        tangent_at(spline, 0.0)
    } else {
        tangent_at(spline, 1.0).map(|tangent| -tangent)
    }
}

pub(super) fn parameter_of(spline: &BSpline, point: Point2) -> f64 {
    closest_spline_parameter(spline, point).clamp(0.0, 1.0)
}

pub(super) fn touch(spline: &BSpline, center: Point2) -> Point2 {
    spline.point_at(parameter_of(spline, center))
}
