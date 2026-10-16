use caditor_expression::Dimension;
use caditor_geometry::{Plane, Point3, Ray, Vector3};
use caditor_kernel::{
    EdgeReference, FaceId, FaceReference, LINEAR_RESOLUTION, MeshQuality, ReferenceError, Surface,
};

use crate::{
    datum::{
        AxisReference, CurveStation, DatumFrame, FaceTangent, PlaneReference, PointBy, Resolver,
        capitalized, describe_axis, describe_plane, face_axis, feature_name,
    },
    datum_cause::{DatumCause, Pair, Run, Unplaced},
    describe::describe_origin,
    document::FeatureId,
    edge_run::EdgeRun,
    recompute::Failure,
    split_edges::{SplitEdges, Unrun},
    tolerance,
};

const PLANES_MEET_TOLERANCE: f64 = 1e-6;

fn resolved_run<'a>(
    resolver: &'a Resolver<'_>,
    body: FeatureId,
    reference: &EdgeReference,
) -> Result<EdgeRun<'a>, Failure> {
    let document = resolver.inputs.document;
    let solid = resolver.body(body)?;
    let name = feature_name(document, body);
    SplitEdges::new(document, solid)
        .run(reference)
        .map_err(|unrun| {
            let run = match unrun {
                Unrun::Missing => Run::Missing,
                Unrun::Tied => Run::Tied,
                Unrun::Gapped => Run::Gapped,
                Unrun::PartlyTraced => Run::PartlyTraced,
            };
            resolver.error(DatumCause::RunMissing { run, name })
        })
}

fn station_point(
    resolver: &Resolver<'_>,
    station: &CurveStation,
) -> Result<(Point3, Vector3), Failure> {
    let name = feature_name(resolver.inputs.document, station.body);
    let run = resolved_run(resolver, station.body, &station.edge)?;
    let distance = resolver.value(&station.distance, "distance", Dimension::LENGTH)?;
    let length = run.length();

    if distance.abs() > length + LINEAR_RESOLUTION {
        return Err(resolver.error(DatumCause::PastEnd {
            name,
            length,
            distance,
        }));
    }

    let along = if distance >= 0.0 {
        distance
    } else {
        length + distance
    };
    let (point, tangent) = run.at(along.max(0.0));

    if !point.is_finite() || tangent.length() <= f64::EPSILON {
        return Err(resolver.error(DatumCause::NoDirection { name }));
    }
    Ok((point, tangent))
}

pub(crate) fn square_to_curve(
    resolver: &Resolver<'_>,
    station: &CurveStation,
) -> Result<Plane, Failure> {
    let (point, tangent) = station_point(resolver, station)?;
    Plane::new(point, tangent)
        .ok_or_else(|| resolver.error(DatumCause::Unplaced(Unplaced::PlaneSquareToEdge)))
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
            return Err(resolver.error(DatumCause::FaceGone {
                described,
                body: feature_name(document, tangent.body),
            }));
        }
    };
    let Some(face) = pieces.first().copied() else {
        return Err(resolver.error(DatumCause::FaceMissing { described }));
    };
    let surface = solid.face(face).map(|face| face.surface());
    let (Some(surface @ (Surface::Cylinder(_) | Surface::Cone(_))), Some(axis)) =
        (surface, face_axis(solid, face))
    else {
        return Err(resolver.error(DatumCause::NotCurved { described }));
    };
    let toward = resolver.point(&tangent.toward)?;
    let offset = toward - axis.origin();
    let radial = offset - axis.direction() * offset.dot(axis.direction());
    if radial.length() <= LINEAR_RESOLUTION {
        return Err(resolver.error(DatumCause::OnAxis { described }));
    }
    let foot = surface.project(toward, None);
    let touching = surface.point(foot.x, foot.y);
    let mut normal = surface.normal(foot.x, foot.y).ok_or_else(|| {
        resolver.error(DatumCause::NoSurfaceDirection {
            described: described.clone(),
        })
    })?;
    if normal.dot(radial) < 0.0 {
        normal = -normal;
    }
    Plane::with_x_axis(touching, normal, axis.direction()).ok_or_else(|| {
        resolver.error(DatumCause::PlaneAgainst {
            described,
            picked: true,
        })
    })
}

fn edge_middle(
    resolver: &Resolver<'_>,
    body: FeatureId,
    reference: &EdgeReference,
) -> Result<Point3, Failure> {
    let run = resolved_run(resolver, body, reference)?;
    Ok(run.at(0.5 * run.length()).0)
}

fn resolved_faces(
    resolver: &Resolver<'_>,
    body: FeatureId,
    reference: &FaceReference,
) -> Result<Vec<FaceId>, Failure> {
    let document = resolver.inputs.document;
    let solid = resolver.body(body)?;
    let described = describe_origin(document, reference.origin());
    let pieces = match reference.resolve(solid) {
        Ok(found) => vec![found],
        Err(ReferenceError::Ambiguous(pieces)) => pieces,
        Err(ReferenceError::Missing) => Vec::new(),
    };
    if pieces.is_empty() {
        return Err(resolver.error(DatumCause::FaceGone {
            described: capitalized(&described),
            body: feature_name(document, body),
        }));
    }
    Ok(pieces)
}

fn face_centre(
    resolver: &Resolver<'_>,
    body: FeatureId,
    reference: &FaceReference,
) -> Result<Point3, Failure> {
    let solid = resolver.body(body)?;
    let faces = resolved_faces(resolver, body, reference)?;
    let described = describe_origin(resolver.inputs.document, reference.origin());
    let unfound = || {
        resolver.error(DatumCause::CentreNotFound {
            described: described.clone(),
        })
    };
    let mesh = solid
        .tessellate(&solid.tolerance_for(&MeshQuality::SMOOTH))
        .map_err(|_| unfound())?;
    let mut area = 0.0;
    let mut moment = Vector3::ZERO;
    for group in mesh
        .faces()
        .iter()
        .filter(|group| faces.contains(&group.face))
    {
        for triangle in mesh
            .triangles()
            .get(group.triangles.clone())
            .unwrap_or_default()
        {
            let corners = mesh
                .triangle_positions(*triangle)
                .map(|corners| corners.map(|corner| mesh.position(corner)));
            let Some([Some(a), Some(b), Some(c)]) = corners else {
                continue;
            };
            let piece = 0.5 * (b - a).cross(c - a).length();
            area += piece;
            moment +=
                ((a - Point3::ZERO) + (b - Point3::ZERO) + (c - Point3::ZERO)) * (piece / 3.0);
        }
    }
    let centre = Point3::ZERO + moment / area;
    if area > f64::MIN_POSITIVE && centre.is_finite() {
        Ok(centre)
    } else {
        Err(unfound())
    }
}

struct Foot {
    point: Point3,
    normal: Vector3,
    described: String,
}

fn face_foot(resolver: &Resolver<'_>, tangent: &FaceTangent) -> Result<Foot, Failure> {
    let solid = resolver.body(tangent.body)?;
    let faces = resolved_faces(resolver, tangent.body, &tangent.face)?;
    let described = describe_origin(resolver.inputs.document, tangent.face.origin());
    let Some(face) = faces.first().and_then(|face| solid.face(*face)) else {
        return Err(resolver.error(DatumCause::FaceMissing {
            described: capitalized(&described),
        }));
    };
    let surface = face.surface();
    if matches!(surface, Surface::Plane(_)) {
        return Err(resolver.error(DatumCause::FlatFace {
            described: capitalized(&described),
        }));
    }
    let toward = resolver.point(&tangent.toward)?;
    let even = |what: &'static str| {
        resolver.error(DatumCause::EvenSides {
            what,
            described: described.clone(),
        })
    };
    let axis_gap = |origin: Point3, direction: Vector3| {
        let offset = toward - origin;
        (offset - direction * offset.dot(direction)).length()
    };
    match surface {
        Surface::Sphere(sphere) if toward.distance(sphere.center()) <= LINEAR_RESOLUTION => {
            return Err(even("at the centre"));
        }
        Surface::Cylinder(_) | Surface::Cone(_) | Surface::Torus(_) => {
            if let Some(axis) = faces.first().and_then(|face| face_axis(solid, *face))
                && axis_gap(axis.origin(), axis.direction()) <= LINEAR_RESOLUTION
            {
                return Err(even("on the axis"));
            }
        }
        _ => {}
    }
    let foot = surface.project(toward, None);
    let point = surface.point(foot.x, foot.y);
    let normal = surface
        .normal(foot.x, foot.y)
        .filter(|normal| normal.is_finite() && normal.length() > f64::EPSILON)
        .ok_or_else(|| {
            resolver.error(DatumCause::NoNormalNearest {
                described: capitalized(&described),
            })
        })?
        .normalize();
    let away = toward - point;
    let outward = normal * face.sense().sign();
    let normal = if away.length() <= LINEAR_RESOLUTION || away.dot(outward) >= 0.0 {
        outward
    } else {
        -outward
    };
    if !point.is_finite() {
        return Err(resolver.error(DatumCause::NearestNotFound { described }));
    }
    Ok(Foot {
        point,
        normal,
        described,
    })
}

pub(crate) fn tangent_plane_at(
    resolver: &Resolver<'_>,
    tangent: &FaceTangent,
) -> Result<Plane, Failure> {
    let foot = face_foot(resolver, tangent)?;
    Plane::new(foot.point, foot.normal).ok_or_else(|| {
        resolver.error(DatumCause::PlaneAgainst {
            described: foot.described,
            picked: false,
        })
    })
}

pub(crate) fn square_to_face(
    resolver: &Resolver<'_>,
    tangent: &FaceTangent,
) -> Result<Ray, Failure> {
    let foot = face_foot(resolver, tangent)?;
    Ray::new(foot.point, foot.normal).ok_or_else(|| {
        resolver.error(DatumCause::AxisAgainst {
            described: foot.described,
        })
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
        return Err(resolver.error(DatumCause::SameLine {
            pair: Pair {
                first: first_name,
                second: second_name,
            },
        }));
    }
    let apart = other.origin() - one.origin();
    let normal = if tolerance::parallel(one.direction(), other.direction()) {
        one.direction().cross(apart)
    } else {
        let across = one.direction().cross(other.direction());
        if apart.dot(across.normalize_or_zero()).abs() > LINEAR_RESOLUTION {
            return Err(resolver.error(DatumCause::LinesNotOnePlane {
                pair: Pair {
                    first: first_name,
                    second: second_name,
                },
            }));
        }
        across
    };
    Plane::with_x_axis(one.origin(), normal, one.direction())
        .ok_or_else(|| resolver.error(DatumCause::Unplaced(Unplaced::PlaneThroughLines)))
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
        resolver.error(DatumCause::LinesParallel {
            pair: Pair {
                first: first_name.clone(),
                second: second_name.clone(),
            },
        })
    };
    if tolerance::parallel(one.direction(), other.direction()) {
        return Err(parallel());
    }
    let across = one.direction().cross(other.direction()).normalize_or_zero();
    if (other.origin() - one.origin()).dot(across).abs() > LINEAR_RESOLUTION {
        return Err(resolver.error(DatumCause::LinesApart {
            pair: Pair {
                first: first_name,
                second: second_name,
            },
        }));
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
        return Err(resolver.error(DatumCause::LineAlongPlane {
            axis: axis_name,
            plane: plane_name,
            within: tolerance::along_plane(line, &flat),
        }));
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
        return Err(resolver.error(DatumCause::PlanesNoCorner {
            first: capitalized(&describe_plane(document, &references[0])),
            second: describe_plane(document, &references[1]),
            third: describe_plane(document, &references[2]),
        }));
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
        PointBy::EdgeMiddle { body, edge } => edge_middle(resolver, *body, edge)?,
        PointBy::FaceCentre { body, face } => face_centre(resolver, *body, face)?,
    };
    if point.is_finite() {
        Ok(point)
    } else {
        Err(resolver.error(DatumCause::Unplaced(Unplaced::Point)))
    }
}

pub(crate) fn coordinate_system(
    resolver: &Resolver<'_>,
    frame: &DatumFrame,
) -> Result<Plane, Failure> {
    let document = resolver.inputs.document;
    let origin = resolver.point(&frame.origin)?;
    let axis = resolver.axis(&frame.x_axis)?;
    let plane = resolver.plane(&frame.plane)?;
    let normal = if frame.reverse_z {
        -plane.normal()
    } else {
        plane.normal()
    };
    let along = if frame.reverse_x {
        -axis.direction()
    } else {
        axis.direction()
    };
    if tolerance::parallel(along, normal) {
        return Err(resolver.error(DatumCause::XAxisAcross {
            axis: capitalized(&describe_axis(document, &frame.x_axis)),
            plane: describe_plane(document, &frame.plane),
        }));
    }
    Plane::with_x_axis(origin, normal, along)
        .ok_or_else(|| resolver.error(DatumCause::Unplaced(Unplaced::CoordinateSystem)))
}
