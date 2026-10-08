use caditor_expression::Dimension;
use caditor_geometry::{Plane, Point3, Ray, Vector3};
use caditor_kernel::{Curve, EdgeId, FaceId, Interval, LINEAR_RESOLUTION, ReferenceError, Surface};

use crate::{
    datum::{
        AxisReference, CurveStation, FaceTangent, PlaneReference, PointBy, Resolver, capitalized,
        describe_axis, describe_plane, face_axis, feature_name,
    },
    describe::describe_origin,
    recompute::Failure,
    tolerance,
};

const LENGTH_BISECTIONS: usize = 50;
const PLANES_MEET_TOLERANCE: f64 = 1e-6;

fn parameter_at_length(curve: &Curve, interval: Interval, along: f64) -> f64 {
    match curve {
        Curve::Line(_) => interval.start() + along,
        Curve::Circle(circle) => interval.start() + along / circle.radius(),
        _ => {
            let (mut low, mut high) = (interval.start(), interval.end());
            for _ in 0..LENGTH_BISECTIONS {
                let middle = 0.5 * (low + high);
                let reached = Interval::new(interval.start(), middle)
                    .map_or(0.0, |range| curve.length(range));
                if reached < along {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            0.5 * (low + high)
        }
    }
}

fn station_point(
    resolver: &Resolver<'_>,
    station: &CurveStation,
) -> Result<(Point3, Vector3), Failure> {
    let document = resolver.inputs.document;
    let solid = resolver.body(station.body)?;
    let name = feature_name(document, station.body);
    let edge: EdgeId = match station.edge.resolve(solid) {
        Ok(found) => found,
        Err(ReferenceError::Ambiguous(_)) => {
            return Err(resolver.own_error(
                format!("The edge of {name} it follows is now several edges."),
                "Choose the edge again.",
            ));
        }
        Err(ReferenceError::Missing) => {
            return Err(resolver.own_error(
                format!("The edge it follows is no longer part of {name}."),
                "Choose another edge for it.",
            ));
        }
    };
    let definition = solid.edge(edge).ok_or_else(|| {
        resolver.own_error(
            format!("The edge it follows is no longer part of {name}."),
            "Choose another edge for it.",
        )
    })?;
    let distance = resolver.value(&station.distance, "distance", Dimension::LENGTH)?;
    let interval = definition.interval();
    let curve = definition.curve();
    let length = curve.length(interval);
    if distance.abs() > length + LINEAR_RESOLUTION {
        return Err(resolver.own_error(
            format!(
                "The edge of {name} it follows is only {length:.3} mm long, so a distance of \
                 {distance:.3} mm along it lies past its end."
            ),
            "Shorten the distance, or choose a longer edge.",
        ));
    }
    let along = if distance >= 0.0 {
        distance
    } else {
        length + distance
    };
    let parameter = interval.clamp(parameter_at_length(curve, interval, along.max(0.0)));
    let derivatives = curve.evaluate(parameter);
    if !derivatives.point.is_finite() || derivatives.first.length() <= f64::EPSILON {
        return Err(resolver.own_error(
            format!("The edge of {name} it follows has no direction at that distance."),
            "Change the distance.",
        ));
    }
    Ok((derivatives.point, derivatives.first))
}

pub(crate) fn square_to_curve(
    resolver: &Resolver<'_>,
    station: &CurveStation,
) -> Result<Plane, Failure> {
    let (point, tangent) = station_point(resolver, station)?;
    Plane::new(point, tangent).ok_or_else(|| {
        resolver.own_error(
            "The plane could not be placed square to the edge.".to_owned(),
            "Change the distance or choose another edge.",
        )
    })
}

pub(crate) fn tangent_plane(
    resolver: &Resolver<'_>,
    tangent: &FaceTangent,
) -> Result<Plane, Failure> {
    let document = resolver.inputs.document;
    let solid = resolver.body(tangent.body)?;
    let described = describe_origin(document, tangent.face.origin());
    let pieces: Vec<FaceId> = match tangent.face.resolve(solid) {
        Ok(found) => vec![found],
        Err(ReferenceError::Ambiguous(pieces)) => pieces,
        Err(ReferenceError::Missing) => {
            return Err(resolver.own_error(
                format!(
                    "{described} is no longer part of {}.",
                    feature_name(document, tangent.body)
                ),
                "Choose another face for it.",
            ));
        }
    };
    let Some(face) = pieces.first().copied() else {
        return Err(resolver.own_error(
            format!("{described} is no longer part of the body."),
            "Choose another face for it.",
        ));
    };
    let surface = solid.face(face).map(|face| face.surface());
    let (Some(surface @ (Surface::Cylinder(_) | Surface::Cone(_))), Some(axis)) =
        (surface, face_axis(solid, face))
    else {
        return Err(resolver.own_error(
            format!("{described} is no longer a cylindrical or conical face."),
            "Choose a cylindrical or conical face.",
        ));
    };
    let toward = resolver.point(&tangent.toward)?;
    let offset = toward - axis.origin();
    let radial = offset - axis.direction() * offset.dot(axis.direction());
    if radial.length() <= LINEAR_RESOLUTION {
        return Err(resolver.own_error(
            format!(
                "The point that picks the side of {described} lies on its axis, so every side \
                 is equally near."
            ),
            "Choose a point off the axis of the face.",
        ));
    }
    let foot = surface.project(toward, None);
    let touching = surface.point(foot.x, foot.y);
    let mut normal = surface.normal(foot.x, foot.y).ok_or_else(|| {
        resolver.own_error(
            format!("{described} has no surface direction at the chosen side."),
            "Choose a point farther from the tip of the cone.",
        )
    })?;
    if normal.dot(radial) < 0.0 {
        normal = -normal;
    }
    Plane::with_x_axis(touching, normal, axis.direction()).ok_or_else(|| {
        resolver.own_error(
            format!("The plane could not be placed against {described}."),
            "Choose another point to pick the side.",
        )
    })
}

pub(crate) fn plane_through_lines(
    resolver: &Resolver<'_>,
    first: &AxisReference,
    second: &AxisReference,
) -> Result<Plane, Failure> {
    let document = resolver.inputs.document;
    let (one, other) = (resolver.axis(first)?, resolver.axis(second)?);
    let first_name = capitalized(&describe_axis(document, first));
    let second_name = describe_axis(document, second);
    if tolerance::same_line(one, other) {
        return Err(resolver.own_error(
            format!(
                "{first_name} and {second_name} are the same line, so no single plane is fixed \
                 by them."
            ),
            "Choose two lines that are not the same.",
        ));
    }
    let apart = other.origin() - one.origin();
    let normal = if tolerance::parallel(one.direction(), other.direction()) {
        one.direction().cross(apart)
    } else {
        let across = one.direction().cross(other.direction());
        if apart.dot(across.normalize_or_zero()).abs() > LINEAR_RESOLUTION {
            return Err(resolver.own_error(
                format!(
                    "{first_name} and {second_name} do not lie in one plane, since they neither \
                     cross nor run parallel."
                ),
                "Choose two lines that cross or run parallel.",
            ));
        }
        across
    };
    Plane::with_x_axis(one.origin(), normal, one.direction()).ok_or_else(|| {
        resolver.own_error(
            "The plane could not be placed through the two lines.".to_owned(),
            "Choose two other lines.",
        )
    })
}

fn lines_cross(
    resolver: &Resolver<'_>,
    first: &AxisReference,
    second: &AxisReference,
) -> Result<Point3, Failure> {
    let document = resolver.inputs.document;
    let (one, other): (Ray, Ray) = (resolver.axis(first)?, resolver.axis(second)?);
    let first_name = capitalized(&describe_axis(document, first));
    let second_name = describe_axis(document, second);
    let parallel = || {
        resolver.own_error(
            format!("{first_name} and {second_name} run parallel, so they never cross."),
            "Choose two lines that cross.",
        )
    };
    if tolerance::parallel(one.direction(), other.direction()) {
        return Err(parallel());
    }
    let across = one.direction().cross(other.direction()).normalize_or_zero();
    if (other.origin() - one.origin()).dot(across).abs() > LINEAR_RESOLUTION {
        return Err(resolver.own_error(
            format!("{first_name} and {second_name} pass each other without meeting."),
            "Choose two lines that lie in one plane, or move one of them.",
        ));
    }
    one.closest_along_line(other.origin(), other.direction())
        .map(|along| other.at(along))
        .ok_or_else(parallel)
}

fn line_meets_plane(
    resolver: &Resolver<'_>,
    axis: &AxisReference,
    plane: &PlaneReference,
) -> Result<Point3, Failure> {
    let document = resolver.inputs.document;
    let line = resolver.axis(axis)?;
    let flat = resolver.plane(plane)?;
    let axis_name = capitalized(&describe_axis(document, axis));
    let plane_name = describe_plane(document, plane);
    if tolerance::perpendicular(line.direction(), flat.normal()) {
        let reason = if tolerance::along_plane(line, &flat) {
            format!("{axis_name} lies in {plane_name}, so it crosses it everywhere.")
        } else {
            format!("{axis_name} runs parallel to {plane_name}, so it never crosses it.")
        };
        return Err(resolver.own_error(reason, "Choose a line that cuts across the plane."));
    }
    let distance = -flat.signed_distance(line.origin()) / line.direction().dot(flat.normal());
    Ok(line.at(distance))
}

fn planes_meet(
    resolver: &Resolver<'_>,
    references: &[PlaneReference; 3],
) -> Result<Point3, Failure> {
    let document = resolver.inputs.document;
    let [one, two, three] = [
        resolver.plane(&references[0])?,
        resolver.plane(&references[1])?,
        resolver.plane(&references[2])?,
    ];
    let corner = two.normal().cross(three.normal());
    let determinant = one.normal().dot(corner);
    if determinant.abs() <= PLANES_MEET_TOLERANCE {
        return Err(resolver.own_error(
            format!(
                "{}, {} and {} do not meet in a single point, since two of them are parallel or \
                 all three share a line.",
                capitalized(&describe_plane(document, &references[0])),
                describe_plane(document, &references[1]),
                describe_plane(document, &references[2])
            ),
            "Choose three planes or flat faces that cross each other in a corner.",
        ));
    }
    let distance = |plane: &Plane| plane.normal().dot(plane.origin());
    Ok((corner * distance(&one)
        + three.normal().cross(one.normal()) * distance(&two)
        + one.normal().cross(two.normal()) * distance(&three))
        / determinant)
}

pub(crate) fn point_by(resolver: &Resolver<'_>, by: &PointBy) -> Result<Point3, Failure> {
    let point = match by {
        PointBy::LinesCross(first, second) => lines_cross(resolver, first, second)?,
        PointBy::AxisAndPlane(axis, plane) => line_meets_plane(resolver, axis, plane)?,
        PointBy::ThreePlanes(planes) => planes_meet(resolver, planes)?,
        PointBy::Along(station) => station_point(resolver, station)?.0,
    };
    if point.is_finite() {
        Ok(point)
    } else {
        Err(resolver.own_error(
            "The point could not be placed.".to_owned(),
            "Choose other references.",
        ))
    }
}
