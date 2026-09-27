use std::f64::consts::{PI, TAU};

use caditor_geometry::{Plane, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    curve::{Circle, Line},
    fixtures::{self, Fixture, Tweak},
    surface::PlaneSurface,
};

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
