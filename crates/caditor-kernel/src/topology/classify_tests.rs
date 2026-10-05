use std::f64::consts::{FRAC_PI_2, PI};

use caditor_geometry::{Plane, Point2, Point3, Vector3};

use super::*;
use crate::{
    build::{AngularExtent, Axis2, LinearExtent, extrude, revolve},
    fixtures,
    fixtures::{
        cone, cuboid, cylinder, extruded_spline, frustum, holed_block, hollow_cuboid, sphere,
        spline_profile, torus,
    },
    profile::{Profile, Selection},
    test_support::{Random, arc, cancelled_after, circle, line, rectangle},
};

const MARGIN: f64 = 1e-3;

type Oracle = Box<dyn Fn(Point3) -> Option<bool>>;

fn clear(value: f64) -> Option<()> {
    (value.abs() > MARGIN).then_some(())
}

fn inside_box(point: Point3, min: Point3, max: Point3) -> Option<bool> {
    let gaps = [
        point.x - min.x,
        max.x - point.x,
        point.y - min.y,
        max.y - point.y,
        point.z - min.z,
        max.z - point.z,
    ];
    let inside = gaps.iter().all(|gap| *gap > 0.0);
    let near = gaps.iter().any(|gap| gap.abs() <= MARGIN);
    if near && (inside || gaps.iter().filter(|gap| **gap < -MARGIN).count() == 0) {
        None
    } else {
        Some(inside)
    }
}

fn spline_polygon() -> Vec<Point2> {
    let profile = spline_profile();
    let mut polygon: Vec<Point2> = (0..=2000)
        .map(|index| {
            let point = profile.point(index as f64 / 2000.0);
            Point2::new(point.x, point.y)
        })
        .collect();
    polygon.push(Point2::new(20.0, 0.0));
    polygon
}

fn polygon_contains(polygon: &[Point2], point: Point2) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
        {
            inside = !inside;
        }
    }
    inside
}

fn polygon_distance(polygon: &[Point2], point: Point2) -> f64 {
    (0..polygon.len())
        .map(|index| {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            let along = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
            point.distance(a + (b - a) * along)
        })
        .fold(f64::INFINITY, f64::min)
}

fn cases() -> Vec<(&'static str, Solid, Oracle)> {
    let radial = |point: Point3| point.truncate().length();
    let polygon = spline_polygon();
    vec![
        (
            "cuboid",
            cuboid(Vector3::new(4.0, 3.0, 2.0)),
            Box::new(|point| inside_box(point, Point3::ZERO, Point3::new(4.0, 3.0, 2.0))),
        ),
        (
            "hollow cuboid",
            hollow_cuboid(10.0, 4.0),
            Box::new(|point| {
                let outer = inside_box(point, Point3::ZERO, Point3::splat(10.0))?;
                let inner = inside_box(point, Point3::splat(3.0), Point3::splat(7.0))?;
                Some(outer && !inner)
            }),
        ),
        (
            "cylinder",
            cylinder(3.0, 5.0),
            Box::new(move |point| {
                clear(radial(point) - 3.0)?;
                clear(point.z)?;
                clear(point.z - 5.0)?;
                Some(radial(point) < 3.0 && point.z > 0.0 && point.z < 5.0)
            }),
        ),
        (
            "holed block",
            holed_block(10.0, 4.0, 2.5),
            Box::new(|point| {
                let block = inside_box(point, Point3::ZERO, Point3::new(10.0, 10.0, 4.0))?;
                let hole = (point.truncate() - caditor_geometry::Vector2::splat(5.0)).length();
                clear(hole - 2.5)?;
                Some(block && hole > 2.5)
            }),
        ),
        (
            "sphere",
            sphere(4.0),
            Box::new(|point| {
                clear(point.length() - 4.0)?;
                Some(point.length() < 4.0)
            }),
        ),
        (
            "torus",
            torus(6.0, 2.0),
            Box::new(move |point| {
                let tube = (radial(point) - 6.0).hypot(point.z);
                clear(tube - 2.0)?;
                Some(tube < 2.0)
            }),
        ),
        (
            "frustum",
            frustum(4.0, 2.0, 5.0),
            Box::new(move |point| {
                let wall = 4.0 - 2.0 * point.z / 5.0;
                clear(radial(point) - wall)?;
                clear(point.z)?;
                clear(point.z - 5.0)?;
                Some(radial(point) < wall && point.z > 0.0 && point.z < 5.0)
            }),
        ),
        (
            "cone",
            cone(3.0, 4.0),
            Box::new(move |point| {
                let wall = 3.0 * (1.0 - point.z / 4.0);
                clear(radial(point) - wall)?;
                clear(point.z)?;
                Some(radial(point) < wall && point.z > 0.0 && point.z < 4.0)
            }),
        ),
        (
            "extruded spline",
            extruded_spline(5.0),
            Box::new(move |point| {
                let flat = point.truncate();
                if polygon_distance(&polygon, flat) <= MARGIN {
                    return None;
                }
                clear(point.z)?;
                clear(point.z - 5.0)?;
                Some(polygon_contains(&polygon, flat) && point.z > 0.0 && point.z < 5.0)
            }),
        ),
    ]
}

#[test]
fn random_points_classify_like_analytic_tests_on_the_fixtures() {
    let mut random = Random::new(17);
    for (name, solid, oracle) in cases() {
        let classifier = solid.classifier();
        let bounds = solid.bounding_box().unwrap().expanded(1.0);
        let mut checked = 0;
        for _ in 0..200 {
            let point = Point3::new(
                random.between(bounds.min().x, bounds.max().x),
                random.between(bounds.min().y, bounds.max().y),
                random.between(bounds.min().z, bounds.max().z),
            );
            let Some(expected) = oracle(point) else {
                continue;
            };
            let class = classifier.classify(point);
            let expected = if expected {
                PointClass::Inside
            } else {
                PointClass::Outside
            };
            assert_eq!(class, expected, "{name}: {point:?}");
            checked += 1;
        }
        assert!(checked > 100, "{name}");
    }
}

#[test]
fn points_on_faces_edges_and_vertices_are_on_the_boundary() {
    for (name, solid, _) in cases() {
        let classifier = solid.classifier();
        for (_, vertex) in solid.vertices() {
            assert!(
                matches!(
                    classifier.classify(vertex.point()),
                    PointClass::OnBoundary(_)
                ),
                "{name}: vertex {:?}",
                vertex.point()
            );
        }
        for (_, edge) in solid.edges() {
            let middle = edge.curve().point(edge.interval().middle());
            assert!(
                matches!(classifier.classify(middle), PointClass::OnBoundary(_)),
                "{name}: edge point {middle:?}"
            );
        }
        for (id, face) in solid.faces() {
            let polygon: Vec<Point2> = face
                .loops()
                .iter()
                .flat_map(|loop_id| solid.face_loop(*loop_id).unwrap().coedges().to_vec())
                .flat_map(|coedge| {
                    solid
                        .coedge(coedge)
                        .unwrap()
                        .pcurve()
                        .samples()
                        .iter()
                        .map(|sample| sample.uv)
                        .collect::<Vec<_>>()
                })
                .collect();
            let center =
                polygon.iter().fold(Point2::ZERO, |sum, uv| sum + *uv) / polygon.len() as f64;
            if classifier.point_in_face(id, center) != Some(FaceContainment::Inside) {
                continue;
            }
            let point = face.surface().point_at(center);
            assert_eq!(
                classifier.classify(point),
                PointClass::OnBoundary(id),
                "{name}: face point {point:?}"
            );
        }
    }
}

#[test]
fn rays_through_edges_and_vertices_retry_another_direction() {
    let solid = cuboid(Vector3::new(4.0, 3.0, 2.0));
    let classifier = solid.classifier();
    let first = Vector3::new(0.573_462, 0.612_378, 0.544_223).normalize();
    for (target, inside) in [
        (Point3::new(4.0, 3.0, 2.0), true),
        (Point3::new(4.0, 1.5, 2.0), true),
        (Point3::new(0.0, 0.0, 0.0), false),
        (Point3::new(2.0, 3.0, 0.0), false),
    ] {
        let direction = if inside { first } else { -first };
        let point = target - direction * 0.5;
        let expected = inside_box(point, Point3::ZERO, Point3::new(4.0, 3.0, 2.0)).unwrap();
        let class = classifier.classify(point);
        assert_eq!(
            class,
            if expected {
                PointClass::Inside
            } else {
                PointClass::Outside
            },
            "{point:?}"
        );
    }
    for point in [
        Point3::new(2.0, 1.5, -3.0),
        Point3::new(-3.0, 1.5, 1.0),
        Point3::new(4.0, -2.0, 2.0),
        Point3::new(2.0, 1.5, 1.0),
    ] {
        let expected = inside_box(point, Point3::ZERO, Point3::new(4.0, 3.0, 2.0)).unwrap();
        assert_eq!(
            classifier.classify(point) == PointClass::Inside,
            expected,
            "{point:?}"
        );
    }
}

#[test]
fn a_point_whose_every_ray_meets_an_edge_or_vertex_is_undecided() {
    let solid = cuboid(Vector3::new(4.0, 4.0, 4.0));

    let classifier = solid.classifier();
    let center = Point3::new(2.0, 2.0, 2.0);
    let corners: Vec<Vector3> = [-1.0, 1.0]
        .into_iter()
        .flat_map(|x| {
            [-1.0, 1.0]
                .into_iter()
                .map(move |y| Vector3::new(x, y, 0.0))
        })
        .flat_map(|xy| {
            [-1.0, 1.0]
                .into_iter()
                .map(move |z| (xy + Vector3::Z * z).normalize())
        })
        .collect();
    let edges = [
        Vector3::new(1.0, 1.0, 0.0).normalize(),
        Vector3::new(0.0, -1.0, 1.0).normalize(),
    ];

    assert_eq!(
        classifier.classify_along(center, &corners),
        PointClass::Undecided
    );
    assert_eq!(
        classifier.classify_along(center, &edges),
        PointClass::Undecided
    );
    assert_eq!(
        classifier.classify_boundary_point(center, Vector3::Z),
        BoundaryClass::Inside
    );
    assert_eq!(classifier.classify(center), PointClass::Inside);

    let mixed = [corners[0], Vector3::new(0.3, 0.2, 0.9).normalize()];
    assert_eq!(
        classifier.classify_along(center, &mixed),
        PointClass::Inside
    );
}

#[test]
fn a_doubtful_crossing_beyond_the_nearest_clean_one_does_not_matter() {
    let solid = hollow_cuboid(6.0, 2.0);

    let classifier = solid.classifier();
    let in_void = Point3::splat(3.0);
    let in_wall = Point3::new(1.0, 3.0, 3.0);
    let void_corner = Point3::splat(4.0);

    assert_eq!(
        classifier.classify_along(in_void, &[(void_corner - in_void).normalize()]),
        PointClass::Undecided
    );
    assert_eq!(
        classifier.classify_along(in_wall, &[(void_corner - in_wall).normalize()]),
        PointClass::Inside
    );
    assert_eq!(classifier.classify(in_void), PointClass::Outside);
}

#[test]
fn point_in_face_handles_holes_seams_and_poles() {
    let block = holed_block(10.0, 4.0, 2.5);
    let top = block
        .faces()
        .find(|(_, face)| {
            matches!(face.surface(), crate::surface::Surface::Plane(plane) if plane.frame().origin().z > 3.0)
        })
        .map(|(id, _)| id)
        .unwrap();
    let surface = block.face(top).unwrap().surface().clone();
    let uv_of = |x: f64, y: f64| surface.project(Point3::new(x, y, 4.0), None);
    assert_eq!(
        block.point_in_face(top, uv_of(1.0, 1.0)),
        Some(FaceContainment::Inside)
    );
    assert_eq!(
        block.point_in_face(top, uv_of(5.0, 5.0)),
        Some(FaceContainment::Outside)
    );
    assert_eq!(
        block.point_in_face(top, uv_of(7.5, 5.0)),
        Some(FaceContainment::OnBoundary)
    );
    assert_eq!(
        block.point_in_face(top, uv_of(7.5 + 1e-5, 5.0)),
        Some(FaceContainment::Inside)
    );
    assert_eq!(
        block.point_in_face(top, uv_of(7.5 - 1e-5, 5.0)),
        Some(FaceContainment::Outside)
    );
    assert_eq!(
        block.point_in_face(top, uv_of(10.0, 10.0)),
        Some(FaceContainment::OnBoundary)
    );
    assert_eq!(
        block.point_in_face(top, uv_of(10.0 + 1e-5, 5.0)),
        Some(FaceContainment::Outside)
    );
    let tube = cylinder(3.0, 5.0);
    let (side, _) = tube
        .faces()
        .find(|(_, face)| matches!(face.surface(), crate::surface::Surface::Cylinder(_)))
        .unwrap();
    for u in [0.0, 1e-9, 2.0 * PI - 1e-9, -0.5, 7.0] {
        assert_eq!(
            tube.point_in_face(side, Point2::new(u, 2.5)),
            Some(FaceContainment::Inside),
            "u {u}"
        );
    }
    assert_eq!(
        tube.point_in_face(side, Point2::new(1.0, 5.0)),
        Some(FaceContainment::OnBoundary)
    );
    assert_eq!(
        tube.point_in_face(side, Point2::new(1.0, 6.0)),
        Some(FaceContainment::Outside)
    );
    let ball = sphere(4.0);
    let (id, _) = ball.faces().next().unwrap();
    for uv in [
        Point2::new(0.3, FRAC_PI_2),
        Point2::new(2.0, -FRAC_PI_2),
        Point2::new(0.0, 0.3),
        Point2::new(3.0, 0.0),
    ] {
        assert_eq!(
            ball.point_in_face(id, uv),
            Some(FaceContainment::Inside),
            "{uv:?}"
        );
    }
}

#[test]
fn boundary_points_report_coincidence_and_sense() {
    let solid = cuboid(Vector3::new(4.0, 3.0, 2.0));
    let classifier = solid.classifier();
    let top = Point3::new(2.0, 1.5, 2.0);
    assert!(matches!(
        classifier.classify_boundary_point(top, Vector3::Z),
        BoundaryClass::Coincident {
            sense: Sense::Same,
            ..
        }
    ));
    assert!(matches!(
        classifier.classify_boundary_point(top, Vector3::NEG_Z),
        BoundaryClass::Coincident {
            sense: Sense::Reversed,
            ..
        }
    ));
    assert!(matches!(
        classifier.classify_boundary_point(top, Vector3::new(1.0, 0.0, 1.0)),
        BoundaryClass::Touching(_)
    ));
    assert_eq!(
        classifier.classify_boundary_point(Point3::new(2.0, 1.5, 1.0), Vector3::Z),
        BoundaryClass::Inside
    );
    assert_eq!(
        classifier.classify_boundary_point(Point3::new(9.0, 1.5, 1.0), Vector3::Z),
        BoundaryClass::Outside
    );
}

#[test]
fn built_solids_classify_like_their_shapes() {
    let regions = |curves: &[crate::profile::ProfileCurve]| {
        Profile::new(curves)
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap()
    };
    let mut plate_curves = rectangle(1, (0.0, 0.0), (10.0, 8.0));
    plate_curves.push(circle(5, (4.0, 4.0), 2.0));
    let plate = extrude(
        &Plane::XY,
        &regions(&plate_curves),
        LinearExtent::one_side(3.0).unwrap(),
        7,
    )
    .unwrap();
    let y_axis = Axis2::new(Point2::ZERO, caditor_geometry::Vector2::Y).unwrap();
    let ring = revolve(
        &Plane::XY,
        &regions(&[circle(1, (5.0, 0.0), 1.0)]),
        y_axis,
        AngularExtent::full(),
        7,
    )
    .unwrap();
    let ball = revolve(
        &Plane::XY,
        &regions(&[
            arc(1, (0.0, 0.0), (0.0, -2.0), (0.0, 2.0)),
            line(2, (0.0, 2.0), (0.0, -2.0)),
        ]),
        y_axis,
        AngularExtent::full(),
        7,
    )
    .unwrap();
    let wedge = revolve(
        &Plane::XY,
        &regions(&rectangle(1, (2.0, 0.0), (3.0, 4.0))),
        y_axis,
        AngularExtent::one_side(FRAC_PI_2).unwrap(),
        7,
    )
    .unwrap();
    let mut random = Random::new(29);
    let shapes: Vec<(&str, Solid, Oracle)> = vec![
        (
            "plate",
            plate,
            Box::new(|point: Point3| {
                let slab = inside_box(point, Point3::ZERO, Point3::new(10.0, 8.0, 3.0))?;
                let hole = (point.truncate() - caditor_geometry::Vector2::splat(4.0)).length();
                clear(hole - 2.0)?;
                Some(slab && hole > 2.0)
            }),
        ),
        (
            "revolved torus",
            ring,
            Box::new(|point: Point3| {
                let radial = point.x.hypot(point.z);
                let tube = (radial - 5.0).hypot(point.y);
                clear(tube - 1.0)?;
                Some(tube < 1.0)
            }),
        ),
        (
            "revolved sphere",
            ball,
            Box::new(|point: Point3| {
                clear(point.length() - 2.0)?;
                Some(point.length() < 2.0)
            }),
        ),
        (
            "quarter tube",
            wedge,
            Box::new(|point: Point3| {
                let radial = point.x.hypot(point.z);
                let angle = (-point.z).atan2(point.x);
                clear(radial - 2.0)?;
                clear(radial - 3.0)?;
                clear(point.y)?;
                clear(point.y - 4.0)?;
                clear(radial * angle.sin())?;
                clear(radial * (angle - FRAC_PI_2).sin())?;
                Some(
                    radial > 2.0
                        && radial < 3.0
                        && point.y > 0.0
                        && point.y < 4.0
                        && angle > 0.0
                        && angle < FRAC_PI_2,
                )
            }),
        ),
    ];
    for (name, solid, oracle) in shapes {
        let classifier = solid.classifier();
        let bounds = solid.bounding_box().unwrap().expanded(1.0);
        let mut checked = 0;
        for _ in 0..150 {
            let point = Point3::new(
                random.between(bounds.min().x, bounds.max().x),
                random.between(bounds.min().y, bounds.max().y),
                random.between(bounds.min().z, bounds.max().z),
            );
            let Some(expected) = oracle(point) else {
                continue;
            };
            let class = classifier.classify(point);
            assert_eq!(
                class == PointClass::Inside,
                expected,
                "{name}: {point:?} is {class:?}"
            );
            assert!(
                !matches!(class, PointClass::OnBoundary(_)),
                "{name}: {point:?}"
            );
            checked += 1;
        }
        assert!(checked > 60, "{name}: {checked}");
    }
}

#[test]
fn a_cancelled_ray_stops_at_its_first_poll_instead_of_trying_every_direction() {
    let mut polled = 0;
    for (name, solid) in fixtures::every_solid() {
        let bounds = solid.bounding_box().unwrap();
        let classifier = solid.classifier();
        let probes = [
            bounds.center(),
            bounds.min().lerp(bounds.max(), 0.3),
            bounds.min() - Vector3::ONE,
        ];
        for point in probes {
            let (class, polls) = cancelled_after(0, || classifier.classify(point));
            let (crossing, ray_polls) = cancelled_after(0, || {
                classifier.first_crossing(point, Vector3::new(0.3, 0.4, 0.86), 0.0)
            });

            assert!(polls <= 1, "{name}: {point:?} polled {polls} times");
            assert!(ray_polls <= 1, "{name}: {point:?} polled {ray_polls} times");
            if polls == 1 {
                assert_eq!(class, PointClass::Undecided, "{name}: {point:?}");
            }
            if ray_polls == 1 {
                assert_eq!(crossing, RayCrossing::Undecided, "{name}: {point:?}");
            }

            polled += polls + ray_polls;
        }
    }
    assert!(polled > 4, "only {polled} casts polled");
}

#[test]
fn a_point_in_a_face_is_found_however_many_periods_its_uv_is_away() {
    let ball = sphere(4.0);
    let (face, definition) = ball.faces().next().unwrap();
    let point = Point3::new(1.8352887868890053, 3.231926532772807, -1.478636516853581);
    let uv = definition.surface().project(point, None);

    for turns in -4..=4 {
        let shifted = Point2::new(uv.x + f64::from(turns) * 2.0 * PI, uv.y);
        assert_eq!(
            ball.point_in_face(face, shifted),
            Some(FaceContainment::Inside),
            "{turns} turns"
        );
    }
}

#[test]
fn a_plane_tangent_to_a_cylinder_touches_it_without_coinciding() {
    let post = cylinder(2.0, 5.0);
    let classifier = post.classifier();
    let on_the_tangent_line = Point3::new(2.0, 0.0, 2.5);
    let wall = definition_of_plane_surface();

    assert!(matches!(
        classifier.classify_boundary_point(on_the_tangent_line, Vector3::X),
        BoundaryClass::Coincident { .. }
    ));
    assert!(matches!(
        classifier.classify_fragment_point(on_the_tangent_line, Vector3::X, &wall),
        BoundaryClass::Touching(_)
    ));
}

fn definition_of_plane_surface() -> Surface {
    Surface::Plane(
        crate::surface::PlaneSurface::new(
            Plane::new(Point3::new(2.0, 0.0, 0.0), Vector3::X).unwrap(),
        )
        .unwrap(),
    )
}
