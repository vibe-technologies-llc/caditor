use std::{
    f64::consts::{FRAC_PI_2, PI, TAU},
    time::{Duration, Instant},
};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    curve::{Circle, Line},
    fixtures::{self, Fixture, Tweak},
    interval::Interval,
    surface::{Cylinder, PlaneSurface, Sphere},
    test_support::assert_cancelled_anywhere,
};

const PRISM_SIDES: usize = 4000;
const PRISM_CROSSING_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn every_fixture_validates() {
    for (name, solid) in fixtures::every_solid() {
        assert_eq!(solid.validate(), Ok(()), "{name}");
    }
}

#[test]
fn fixtures_have_the_expected_topology() {
    let counts = |solid: &Solid| {
        (
            solid.vertices().count(),
            solid.edges().count(),
            solid.faces().count(),
            solid.loops().count(),
            solid.shells().count(),
        )
    };
    let solids: Vec<_> = fixtures::every_solid()
        .into_iter()
        .map(|(name, solid)| (name, counts(&solid)))
        .collect();
    let expected = [
        ("cuboid", (8, 12, 6, 6, 1)),
        ("hollow cuboid", (16, 24, 12, 12, 2)),
        ("cylinder", (2, 3, 3, 3, 1)),
        ("holed block", (10, 15, 7, 9, 1)),
        ("sphere", (2, 1, 1, 1, 1)),
        ("torus", (1, 2, 1, 1, 1)),
        ("frustum", (2, 3, 3, 3, 1)),
        ("cone", (2, 2, 2, 2, 1)),
        ("extruded spline", (4, 6, 4, 4, 1)),
    ];
    assert_eq!(solids, expected);
}

fn seam_offsets(solid: &Solid) -> Vec<f64> {
    solid
        .edges()
        .filter(|(_, edge)| {
            let faces: Vec<_> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| solid.coedge_face(*coedge))
                .collect();
            faces.len() == 2 && faces[0] == faces[1]
        })
        .map(|(_, edge)| {
            let [first, second] = edge.coedges() else {
                panic!("a seam has two coedges")
            };
            let first = solid.coedge(*first).unwrap().pcurve();
            let second = solid.coedge(*second).unwrap().pcurve();
            let apart = second.uv_at(0.5 * (first.start_parameter() + first.end_parameter()))
                - first.uv_at(0.5 * (first.start_parameter() + first.end_parameter()));
            apart.x.abs().max(apart.y.abs())
        })
        .collect()
}

#[test]
fn seam_coedges_sit_one_period_apart() {
    for (name, solid) in [
        ("cylinder", fixtures::cylinder(3.0, 5.0)),
        ("sphere", fixtures::sphere(4.0)),
        ("torus", fixtures::torus(6.0, 2.0)),
        ("cone", fixtures::cone(3.0, 4.0)),
    ] {
        let offsets = seam_offsets(&solid);
        assert!(!offsets.is_empty(), "{name}");
        for offset in offsets {
            assert!((offset - TAU).abs() < 1e-9, "{name}: {offset}");
        }
    }
}

#[test]
fn a_sphere_loop_closes_along_its_poles() {
    let solid = fixtures::sphere(4.0);
    let (_, face) = solid.faces().next().unwrap();
    let face_loop = solid.face_loop(face.outer_loop().unwrap()).unwrap();
    let pcurves: Vec<&Pcurve> = face_loop
        .coedges()
        .iter()
        .map(|id| solid.coedge(*id).unwrap().pcurve())
        .collect();
    assert_eq!(pcurves.len(), 2);
    for (current, next) in [(0, 1), (1, 0)] {
        let end = pcurves[current].end();
        let start = pcurves[next].start();
        assert!((end.y - start.y).abs() < 1e-12);
        assert!((end.y.abs() - PI / 2.0).abs() < 1e-12);
        assert!(((end.x - start.x).abs() - TAU).abs() < 1e-12);
    }
}

#[test]
fn a_missing_face_leaves_edges_used_once() {
    let solid = fixtures::tweaked_cuboid(Tweak {
        missing: Some(3),
        ..Tweak::default()
    });
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::EdgeUseCount { uses: 1, .. })
    ));
}

#[test]
fn a_flipped_face_winds_its_loop_the_wrong_way() {
    let solid = fixtures::tweaked_cuboid(Tweak {
        flipped: Some(2),
        ..Tweak::default()
    });
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::LoopOrientation { .. })
    ));
}

#[test]
fn an_edge_off_its_face_surface_is_found() {
    let solid = fixtures::tweaked_cuboid(Tweak {
        shifted: Some((4, 0.01)),
        ..Tweak::default()
    });
    match solid.validate() {
        Err(ValidationError::EdgeOffSurface { distance, .. }) => {
            assert!((distance - 0.01).abs() < 1e-9)
        }
        other => panic!("expected an edge off its surface, found {other:?}"),
    }
}

#[test]
fn a_loop_whose_coedges_do_not_chain_is_found() {
    let solid = fixtures::tweaked_cuboid(Tweak {
        scrambled: Some(1),
        ..Tweak::default()
    });
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::LoopBreak { .. })
    ));
}

#[test]
fn an_open_loop_is_found() {
    let mut fixture = Fixture::new();
    let corners = fixtures::cuboid_vertices(&mut fixture, Point3::ZERO, Point3::ONE);
    for (index, face) in fixtures::CUBOID_FACES.iter().enumerate() {
        let cycle: Vec<VertexId> = face.iter().map(|corner| corners[*corner]).collect();
        let mut coedges = fixture.polygon_loop(&cycle);
        if index == 0 {
            coedges.pop();
        }
        let plane = fixture.polygon_plane(&cycle);
        fixture.face(PlaneSurface::new(plane).unwrap(), Sense::Same, &[coedges]);
    }
    let solid = fixture.build_unchecked();
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::EdgeUseCount { uses: 1, .. } | ValidationError::LoopBreak { .. })
    ));
}

#[test]
fn an_inside_out_solid_is_a_void_without_an_outer_shell() {
    let solid = fixtures::tweaked_cuboid(Tweak {
        inverted: true,
        ..Tweak::default()
    });
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::VoidOutside(_))
    ));
}

#[test]
fn a_void_outside_its_outer_shell_is_found() {
    let mut fixture = Fixture::new();
    fixtures::add_cuboid(&mut fixture, Point3::ZERO, Point3::ONE, Tweak::default());
    fixture.next_shell();
    fixtures::add_cuboid(
        &mut fixture,
        Point3::splat(3.0),
        Point3::splat(4.0),
        Tweak {
            inverted: true,
            ..Tweak::default()
        },
    );
    let solid = fixture.build_unchecked();
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::VoidOutside(_))
    ));
}

fn cuboids(boxes: &[(f64, f64, bool)]) -> Solid {
    let mut fixture = Fixture::new();
    for (index, (low, high, inverted)) in boxes.iter().enumerate() {
        if index > 0 {
            fixture.next_shell();
        }
        let tweak = Tweak {
            inverted: *inverted,
            ..Tweak::default()
        };
        fixtures::add_cuboid(
            &mut fixture,
            Point3::splat(*low),
            Point3::splat(*high),
            tweak,
        );
    }
    fixture.build_unchecked()
}

fn shifted_cuboids(offsets: &[Vector3]) -> Solid {
    let mut fixture = Fixture::new();
    for (index, offset) in offsets.iter().enumerate() {
        if index > 0 {
            fixture.next_shell();
        }
        let min = Point3::ZERO + *offset;
        fixtures::add_cuboid(&mut fixture, min, min + Vector3::ONE, Tweak::default());
    }
    fixture.build_unchecked()
}

fn add_sphere(fixture: &mut Fixture, radius: f64, sense: Sense) {
    let south = fixture.vertex(Point3::new(0.0, 0.0, -radius));
    let north = fixture.vertex(Point3::new(0.0, 0.0, radius));
    let meridian = Plane::from_frame(Point3::ZERO, Vector3::NEG_Y, Vector3::X).unwrap();
    let seam = fixture.edge(
        Circle::new(meridian, radius).unwrap(),
        Interval::new(-FRAC_PI_2, FRAC_PI_2).unwrap(),
        south,
        north,
    );
    let senses = [Sense::Same, Sense::Reversed].map(|turn| (seam, turn.combined(sense)));
    fixture.face(
        Sphere::new(Plane::XY, radius).unwrap(),
        sense,
        &[senses.to_vec()],
    );
}

fn ball_in_spherical_void(void: f64, ball: f64) -> Solid {
    let mut fixture = Fixture::new();
    fixtures::add_cuboid(
        &mut fixture,
        Point3::splat(-5.0),
        Point3::splat(5.0),
        Tweak::default(),
    );
    fixture.next_shell();
    add_sphere(&mut fixture, void, Sense::Reversed);
    fixture.next_shell();
    add_sphere(&mut fixture, ball, Sense::Same);
    fixture.build_unchecked()
}

#[test]
fn a_lump_inside_another_is_found() {
    let solid = cuboids(&[(0.0, 4.0, false), (1.0, 2.0, false)]);
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::LumpsOverlap { .. })
    ));
}

#[test]
fn overlapping_lumps_are_found() {
    for offset in [
        Vector3::splat(0.5),
        Vector3::new(0.5, 0.0, 0.0),
        Vector3::new(0.999, 0.999, 0.0),
    ] {
        let solid = shifted_cuboids(&[Vector3::ZERO, offset]);
        assert!(
            matches!(solid.validate(), Err(ValidationError::LumpsOverlap { .. })),
            "{offset}"
        );
    }
}

#[test]
fn coincident_lumps_are_found() {
    let solid = shifted_cuboids(&[Vector3::ZERO, Vector3::ZERO]);
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::LumpsCoincide { .. })
    ));
    assert!(matches!(
        ball_in_spherical_void(3.0, 3.0).validate(),
        Err(ValidationError::LumpsCoincide { .. })
    ));
}

#[test]
fn lumps_apart_or_touching_validate() {
    for offset in [
        Vector3::splat(2.0),
        Vector3::X,
        Vector3::new(1.0, 0.5, 0.0),
        Vector3::new(1.0, 1.0, 0.0),
        Vector3::ONE,
    ] {
        let solid = shifted_cuboids(&[Vector3::ZERO, offset]);
        assert_eq!(solid.validate(), Ok(()), "{offset}");
    }
}

#[test]
fn a_lump_inside_a_void_validates_and_one_in_the_wall_does_not() {
    let floating = cuboids(&[(0.0, 6.0, false), (1.0, 5.0, true), (2.0, 4.0, false)]);
    assert_eq!(floating.validate(), Ok(()));
    let in_the_wall = cuboids(&[(0.0, 6.0, false), (2.0, 4.0, true), (0.5, 1.5, false)]);
    assert!(matches!(
        in_the_wall.validate(),
        Err(ValidationError::LumpsOverlap { .. })
    ));
}

#[test]
fn a_ball_in_a_spherical_void_validates_unless_it_reaches_the_wall() {
    for ball in [1.0, 2.9, 2.999] {
        assert_eq!(
            ball_in_spherical_void(3.0, ball).validate(),
            Ok(()),
            "{ball}"
        );
    }
    assert!(matches!(
        ball_in_spherical_void(3.0, 3.5).validate(),
        Err(ValidationError::LumpsOverlap { .. })
    ));
}

#[test]
fn overlapping_or_nested_voids_are_found() {
    for voids in [
        [(1.0, 3.0, true), (2.0, 4.0, true)],
        [(1.0, 5.0, true), (2.0, 4.0, true)],
    ] {
        let mut boxes = vec![(0.0, 6.0, false)];
        boxes.extend(voids);
        assert!(matches!(
            cuboids(&boxes).validate(),
            Err(ValidationError::LumpsOverlap { .. })
        ));
    }
}

#[test]
fn a_dangling_edge_inside_a_flat_face_is_found() {
    let mut fixture = Fixture::new();
    let corners = fixtures::cuboid_vertices(&mut fixture, Point3::ZERO, Point3::splat(2.0));
    let middle = fixture.vertex(Point3::new(1.0, 1.0, 2.0));
    for (index, face) in fixtures::CUBOID_FACES.iter().enumerate() {
        let cycle: Vec<VertexId> = face.iter().map(|corner| corners[*corner]).collect();
        let plane = fixture.polygon_plane(&cycle);
        let mut coedges = Vec::new();
        if index == 1 {
            coedges.push(fixture.line(cycle[0], middle));
            coedges.push(fixture.line(middle, cycle[0]));
        }
        coedges.extend(fixture.polygon_loop(&cycle));
        fixture.face(PlaneSurface::new(plane).unwrap(), Sense::Same, &[coedges]);
    }
    let solid = fixture.build_unchecked();
    assert!(matches!(
        solid.validate(),
        Err(ValidationError::DanglingEdge { .. })
    ));
}

#[test]
fn builder_rejects_bad_edges() {
    let mut builder = SolidBuilder::new();
    assert_eq!(
        builder.vertex(Point3::new(f64::NAN, 0.0, 0.0)),
        Err(BuildError::NonFinitePoint)
    );
    let a = builder.vertex(Point3::ZERO).unwrap();
    let b = builder.vertex(Point3::X).unwrap();
    let c = builder.vertex(Point3::new(1e-9, 0.0, 0.0)).unwrap();
    let line = Line::through(Point3::ZERO, Point3::X).unwrap();
    assert!(matches!(
        builder.edge(line.into(), Interval::new(0.0, 0.5).unwrap(), a, b),
        Err(BuildError::VertexOffCurve { .. })
    ));
    assert!(matches!(
        builder.edge(line.into(), Interval::new(0.0, 1e-9).unwrap(), a, c),
        Err(BuildError::ZeroLengthEdge)
    ));
    let circle = Circle::new(Plane::XY, 1.0).unwrap();
    assert_eq!(
        builder.edge(circle.into(), Interval::new(0.0, 7.0).unwrap(), b, b),
        Err(BuildError::IntervalOutsideDomain)
    );
    let missing = VertexId::from_index(99).unwrap();
    assert_eq!(
        builder.line_edge(a, missing),
        Err(BuildError::UnknownVertex(missing))
    );
    let shell = builder.shell().unwrap();
    let face = builder
        .face(
            shell,
            PlaneSurface::new(Plane::XY).unwrap().into(),
            Sense::Same,
        )
        .unwrap();
    assert_eq!(builder.add_loop(face, &[]), Err(BuildError::EmptyLoop));
    assert!(matches!(builder.build(), Err(BuildError::Invalid(_))));
    assert!(matches!(
        SolidBuilder::new().build(),
        Err(BuildError::Invalid(ValidationError::NoShells))
    ));
}

#[test]
fn names_are_placeholders_until_set() {
    let mut fixture = Fixture::new();
    fixtures::add_cuboid(&mut fixture, Point3::ZERO, Point3::ONE, Tweak::default());
    let face = FaceId::from_index(0).unwrap();
    let edge = EdgeId::from_index(0).unwrap();
    fixture
        .builder
        .set_face_name(face, FaceName::from_digest(7))
        .unwrap();
    fixture
        .builder
        .set_face_origin(face, FaceOrigin::StartCap { feature: 3 })
        .unwrap();
    fixture
        .builder
        .set_edge_name(edge, EdgeName::from_digest(9))
        .unwrap();
    let solid = fixture.build();
    assert_eq!(solid.face(face).unwrap().name(), FaceName::from_digest(7));
    assert_eq!(
        solid.face(face).unwrap().origin(),
        Some(FaceOrigin::StartCap { feature: 3 })
    );
    assert_eq!(solid.edge(edge).unwrap().name(), EdgeName::from_digest(9));
    assert!(
        solid
            .face(FaceId::from_index(1).unwrap())
            .unwrap()
            .name()
            .is_none()
    );
    assert!(EdgeName::default().is_none());
}

#[test]
fn transformed_solids_stay_valid() {
    let transform = RigidTransform::rotation_about(
        Point3::new(1.0, 2.0, 3.0),
        Vector3::new(1.0, -1.0, 0.5),
        0.7,
    )
    .unwrap()
    .then(&RigidTransform::translation(Vector3::new(50.0, -20.0, 5.0)).unwrap());
    for (name, solid) in fixtures::every_solid() {
        let moved = solid.transformed(&transform).unwrap();
        assert_eq!(moved.validate(), Ok(()), "{name}");
    }
}

#[test]
fn pcurves_check_their_samples_and_interpolate_by_edge_parameter() {
    let sample = |parameter: f64, u: f64, v: f64| PcurveSample {
        parameter,
        uv: caditor_geometry::Point2::new(u, v),
    };
    assert_eq!(
        Pcurve::new(vec![sample(0.0, 0.0, 0.0)], 0.0),
        Err(PcurveError::TooFewSamples)
    );
    assert_eq!(
        Pcurve::new(vec![sample(0.0, 0.0, 0.0), sample(0.0, 1.0, 0.0)], 0.0),
        Err(PcurveError::NotMonotone)
    );
    assert_eq!(
        Pcurve::new(vec![sample(0.0, 0.0, 0.0), sample(1.0, f64::NAN, 0.0)], 0.0),
        Err(PcurveError::NonFinite)
    );
    let backwards = Pcurve::new(
        vec![
            sample(2.0, 0.0, 0.0),
            sample(1.0, 1.0, 2.0),
            sample(0.0, 3.0, 2.0),
        ],
        1e-4,
    )
    .unwrap();
    assert_eq!(
        backwards.uv_at(1.5),
        caditor_geometry::Point2::new(0.5, 1.0)
    );
    assert_eq!(
        backwards.uv_at(0.5),
        caditor_geometry::Point2::new(2.0, 2.0)
    );
    assert_eq!(backwards.uv_at(-4.0), backwards.end());
    assert_eq!(backwards.uv_at(9.0), backwards.start());
    let moved = backwards.shifted(caditor_geometry::Vector2::new(TAU, 0.0));
    assert_eq!(moved.start().x, TAU);
    assert_eq!(moved.tolerance(), 1e-4);
}

#[test]
fn pcurves_on_curved_faces_stay_close_to_their_edges() {
    for (name, solid) in fixtures::every_solid() {
        for (_, coedge) in solid.coedges() {
            let pcurve = coedge.pcurve();
            assert!(pcurve.samples().len() >= 2, "{name}");
            assert!(
                pcurve.samples().len() < 5000,
                "{name}: {} samples",
                pcurve.samples().len()
            );
        }
    }
}

#[test]
fn fitting_pcurves_and_validating_cancelled_anywhere_stop_with_cancelled() {
    for (name, solid) in [
        ("extruded spline", fixtures::extruded_spline(3.0)),
        ("torus", fixtures::torus(6.0, 2.0)),
    ] {
        let refit = || {
            solid
                .coedges()
                .map(|(id, coedge)| {
                    let edge = solid.edge(coedge.edge()).unwrap();
                    let face = solid.face(solid.coedge_face(id).unwrap()).unwrap();
                    fit_pcurve(
                        face.surface(),
                        edge.curve(),
                        edge.interval(),
                        coedge.sense(),
                        Some(coedge.pcurve().start()),
                    )
                })
                .collect::<Result<Vec<Pcurve>, PcurveError>>()
        };

        let fits = assert_cancelled_anywhere(name, refit, |error| {
            matches!(error, PcurveError::Cancelled(_))
        });
        let checks = assert_cancelled_anywhere(
            name,
            || solid.validate(),
            |error| {
                matches!(
                    error,
                    ValidationError::Cancelled(_)
                        | ValidationError::Tessellation(TessellationError::Cancelled(_))
                )
            },
        );

        assert!(fits > solid.coedges().count(), "{name}: {fits} polls");
        assert!(checks > solid.coedges().count(), "{name}: {checks} polls");
    }
}

#[test]
fn a_sphere_is_bounded_where_no_edge_reaches() {
    let ball = crate::fixtures::sphere(5.0);
    let bounds = ball.bounding_box().unwrap();
    for axis in 0..3 {
        assert!((bounds.min()[axis] + 5.0).abs() < 1e-6, "{bounds:?}");
        assert!((bounds.max()[axis] - 5.0).abs() < 1e-6, "{bounds:?}");
    }
}

#[test]
fn overlapping_lumps_cross_and_valid_solids_do_not() {
    let mut fixture = crate::fixtures::Fixture::new();
    crate::fixtures::add_cuboid(
        &mut fixture,
        Point3::ZERO,
        Point3::splat(4.0),
        crate::fixtures::Tweak::default(),
    );
    fixture.next_shell();
    crate::fixtures::add_cuboid(
        &mut fixture,
        Point3::splat(2.0),
        Point3::splat(6.0),
        crate::fixtures::Tweak::default(),
    );
    let overlapping = fixture.build_unchecked();
    let crossing = overlapping
        .find_crossing()
        .unwrap()
        .crossing()
        .expect("the lumps cross");
    assert_ne!(crossing.faces[0], crossing.faces[1]);
    assert!(overlapping.classify_point(crossing.point) != crate::PointClass::Outside);

    for (name, solid) in crate::fixtures::every_solid() {
        assert_eq!(
            solid.find_crossing().unwrap(),
            CrossingCheck::Clear,
            "{name}"
        );
    }
}

#[test]
fn a_face_folded_onto_its_neighbour_crosses_it() {
    let mut fixture = Fixture::new();
    let corners = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(4.0, 0.0, 0.0),
        Point3::new(4.0, 4.0, 0.0),
        Point3::new(0.0, 4.0, 0.0),
        Point3::new(0.0, 2.0, 0.0),
        Point3::new(4.0, 2.0, 0.0),
    ]
    .map(|point| fixture.vertex(point));
    let base = fixture.polygon(&[corners[0], corners[1], corners[2], corners[3]], &[]);
    let flap = fixture.polygon(&[corners[1], corners[0], corners[4], corners[5]], &[]);
    let folded = fixture.build_unchecked();
    let crossing = folded
        .find_crossing()
        .unwrap()
        .crossing()
        .expect("the flap lies on the base");
    let mut faces = crossing.faces;
    faces.sort();
    assert_eq!(faces, [base, flap]);
    assert!(crossing.point.z.abs() < 1e-9 && crossing.point.y > 1e-3);
}

#[test]
fn a_curved_flap_passing_through_its_neighbour_crosses_it() {
    let mut fixture = Fixture::new();
    let radius = 2.0_f64.sqrt();
    let rim =
        |x: f64, angle: f64| Point3::new(x, 1.0 + radius * angle.cos(), 1.0 + radius * angle.sin());
    let start = -PI / 3.0;
    let end = 1.25 * PI;
    let shared =
        [Point3::new(0.0, 0.0, 0.0), Point3::new(4.0, 0.0, 0.0)].map(|point| fixture.vertex(point));
    let outline = [
        Point3::new(6.0, -1.0, 0.0),
        Point3::new(6.0, 5.0, 0.0),
        Point3::new(-2.0, 5.0, 0.0),
        Point3::new(-2.0, -1.0, 0.0),
    ]
    .map(|point| fixture.vertex(point));
    let base = fixture.polygon(
        &[
            shared[0], shared[1], outline[0], outline[1], outline[2], outline[3],
        ],
        &[],
    );
    let far = [rim(0.0, start), rim(4.0, start)].map(|point| fixture.vertex(point));
    let arcs = [0.0, 4.0].map(|x| {
        Circle::new(
            Plane::from_frame(Point3::new(x, 1.0, 1.0), Vector3::X, Vector3::Y).unwrap(),
            radius,
        )
        .unwrap()
    });
    let [first_arc, second_arc] = arcs;
    let range = Interval::new(start, end).unwrap();
    let near_arc = fixture.edge(first_arc, range, far[0], shared[0]);
    let far_arc = fixture.edge(second_arc, range, far[1], shared[1]);
    let back = fixture.line(shared[1], shared[0]);
    let across = fixture.line(far[0], far[1]);
    let flap = fixture.face(
        Cylinder::new(
            Plane::from_frame(Point3::new(0.0, 1.0, 1.0), Vector3::X, Vector3::Y).unwrap(),
            radius,
        )
        .unwrap(),
        Sense::Same,
        &[vec![
            back,
            (near_arc, Sense::Reversed),
            across,
            (far_arc, Sense::Same),
        ]],
    );
    let solid = fixture.build_unchecked();
    let crossing = solid
        .find_crossing()
        .unwrap()
        .crossing()
        .expect("the flap passes through the base");
    let mut faces = crossing.faces;
    faces.sort();
    assert_eq!(faces, [base, flap]);
    assert!(
        crossing.point.z.abs() < 1e-6 && (crossing.point.y - 2.0).abs() < 1e-6,
        "{crossing:?}"
    );
}

#[test]
fn a_hole_crossing_its_outer_loop_crosses_its_own_face() {
    let mut fixture = Fixture::new();
    let outer = [
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(4.0, 0.0, 0.0),
        Point3::new(4.0, 4.0, 0.0),
        Point3::new(0.0, 4.0, 0.0),
    ]
    .map(|point| fixture.vertex(point));
    let hole = [
        Point3::new(3.0, 1.0, 0.0),
        Point3::new(3.0, 2.0, 0.0),
        Point3::new(5.0, 2.0, 0.0),
        Point3::new(5.0, 1.0, 0.0),
    ]
    .map(|point| fixture.vertex(point));
    let hole_loop = fixture.polygon_loop(&hole);
    let plate = fixture.polygon(&outer, &[hole_loop]);
    let solid = fixture.build_unchecked();
    let crossing = solid
        .find_crossing()
        .unwrap()
        .crossing()
        .expect("the hole crosses the outer loop");
    assert_eq!(crossing.faces, [plate, plate]);
    assert!((crossing.point.x - 4.0).abs() < 1e-9, "{crossing:?}");
}

#[test]
fn blended_solids_do_not_cross() {
    let fillet = crate::blend::BlendShape::Fillet { radius: 0.5 };
    let chamfer = crate::blend::BlendShape::Chamfer { distance: 0.5 };
    for (name, solid, shape) in [
        (
            "rounded box",
            fixtures::cuboid(Vector3::new(4.0, 3.0, 2.0)),
            fillet,
        ),
        (
            "bevelled box",
            fixtures::cuboid(Vector3::new(4.0, 3.0, 2.0)),
            chamfer,
        ),
        ("rounded drum", fixtures::cylinder(3.0, 4.0), fillet),
        (
            "rounded holed block",
            fixtures::holed_block(6.0, 3.0, 1.0),
            fillet,
        ),
    ] {
        let edges: Vec<EdgeId> = solid.edges().map(|(id, _)| id).collect();
        let blended = crate::blend::blend(&solid, &edges, shape, 7).unwrap();
        assert_eq!(
            blended.find_crossing().unwrap(),
            CrossingCheck::Clear,
            "{name}"
        );
    }
}

#[test]
fn imported_solids_name_faces_by_position_and_edges_uniquely() {
    let drum = fixtures::cylinder(3.0, 10.0);
    let right = fixtures::cuboid(Vector3::new(20.0, 20.0, 20.0))
        .transformed(&RigidTransform::translation(Vector3::new(0.0, -10.0, -5.0)).unwrap())
        .unwrap();
    let half_drum = crate::boolean::boolean(
        &drum,
        &right,
        crate::boolean::BooleanOperation::Intersection,
    )
    .unwrap();
    for (name, solid) in fixtures::every_solid()
        .into_iter()
        .chain([("half drum", half_drum)])
    {
        let imported = solid.clone().imported(9);
        for (index, (_, face)) in imported.faces().enumerate() {
            let index = u32::try_from(index).unwrap();
            assert_eq!(face.name(), FaceName::imported(9, index), "{name}");
            assert_eq!(
                face.origin(),
                Some(FaceOrigin::Imported {
                    feature: 9,
                    face: index
                }),
                "{name}"
            );
        }
        let names: Vec<EdgeName> = imported.edges().map(|(_, edge)| edge.name()).collect();
        let distinct: BTreeSet<EdgeName> = names.iter().copied().collect();
        assert_eq!(distinct.len(), names.len(), "{name}");

        let moved = solid
            .transformed(&RigidTransform::translation(Vector3::new(3.0, -2.0, 7.0)).unwrap())
            .unwrap()
            .imported(9);
        let again: Vec<EdgeName> = moved.edges().map(|(_, edge)| edge.name()).collect();
        assert_eq!(again, names, "{name}");
    }
}

#[test]
fn the_approximate_size_counts_topology_pcurves_and_spline_data() {
    let block = fixtures::cuboid(Vector3::new(1.0, 1.0, 1.0));
    let spline = fixtures::spline_topped_block(10.0, 5.0, 2.0);
    let pcurve_samples: usize = spline
        .coedges()
        .map(|(_, coedge)| coedge.pcurve().heap_size())
        .sum();
    let arenas = size_of::<Vertex>() * 8
        + size_of::<Edge>() * 12
        + size_of::<Coedge>() * 24
        + size_of::<Loop>() * 6
        + size_of::<Face>() * 6
        + size_of::<Shell>();

    assert!(block.approximate_size() > arenas);
    assert!(block.approximate_size() < 64 * 1024);
    assert!(spline.approximate_size() > block.approximate_size() + pcurve_samples);
}

fn polygonal_prism(sides: usize) -> Solid {
    let mut fixture = Fixture::new();

    let ring = |fixture: &mut Fixture, z: f64| -> Vec<VertexId> {
        (0..sides)
            .map(|index| {
                let angle = TAU * index as f64 / sides as f64;
                fixture.vertex(Point3::new(100.0 * angle.cos(), 100.0 * angle.sin(), z))
            })
            .collect()
    };
    let bottom = ring(&mut fixture, 0.0);
    let top = ring(&mut fixture, 10.0);

    let reversed: Vec<VertexId> = bottom.iter().rev().copied().collect();
    fixture.polygon(&reversed, &[]);
    fixture.polygon(&top, &[]);
    for index in 0..sides {
        let next = (index + 1) % sides;
        fixture.polygon(&[bottom[index], bottom[next], top[next], top[index]], &[]);
    }
    fixture.build_unchecked()
}

#[test]
fn a_prism_of_many_sides_is_checked_for_crossings_in_bounded_time() {
    let prism = polygonal_prism(PRISM_SIDES);

    let clock = Instant::now();
    let check = prism.find_crossing().unwrap();
    let elapsed = clock.elapsed();

    assert_eq!(check, CrossingCheck::Clear);
    assert!(elapsed < PRISM_CROSSING_TIME_LIMIT, "{elapsed:?}");
}

#[test]
fn a_validation_error_names_the_faces_it_involves() {
    let block = fixtures::cuboid(Vector3::new(2.0, 3.0, 4.0));
    let (edge, _) = block.edges().next().unwrap();
    let (shell, _) = block.shells().next().unwrap();
    let (face, _) = block.faces().next().unwrap();
    let (face_loop, _) = block.loops().next().unwrap();
    let (coedge, _) = block.coedges().next().unwrap();
    let all: Vec<FaceId> = block.faces().map(|(id, _)| id).collect();

    assert_eq!(ValidationError::NoShells.faces(&block), Vec::new());
    assert_eq!(ValidationError::EmptyVolume(shell).faces(&block), all);
    assert_eq!(
        ValidationError::FaceWithoutLoops(face).faces(&block),
        vec![face]
    );
    assert_eq!(ValidationError::EdgeSenses(edge).faces(&block).len(), 2);
    assert_eq!(
        ValidationError::EmptyLoop(face_loop).faces(&block),
        vec![block.face_loop(face_loop).unwrap().face()]
    );
    assert_eq!(
        ValidationError::PcurveGap(coedge).faces(&block),
        vec![block.coedge_face(coedge).unwrap()]
    );
}
