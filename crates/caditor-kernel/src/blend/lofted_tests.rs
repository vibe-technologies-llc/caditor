use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector3};

use super::{
    tests::{block_at, check, edge_through, fillet, run, spandrel, swept, volume},
    *,
};
use crate::{
    build::{AngularExtent, Axis2, LinearExtent, extrude, extrude_tapered, revolve},
    naming::FaceOrigin,
    profile::{Profile, ProfileCurve, Selection},
    test_support::{arc, line},
    topology::Face,
};

const TANGENT: f64 = 1e-3;

fn chamfer(distance: f64) -> BlendShape {
    BlendShape::Chamfer { distance }
}

fn cylinder() -> Solid {
    swept(
        Plane::XY,
        &[ProfileCurve::circle(1, Point2::new(0.0, 0.0), 40.0)],
        60.0,
    )
}

fn pocket() -> Solid {
    block_at((-10.0, 30.0, 20.0), (20.0, 20.0, 20.0), 2)
}

fn pocketed(body: &Solid, pocket: &Solid) -> Solid {
    boolean(body, pocket, BooleanOperation::Difference).unwrap()
}

fn rim(solid: &Solid) -> Vec<EdgeId> {
    solid
        .edges()
        .filter(|(id, _)| {
            let features: Vec<u64> = edge_faces(solid, *id)
                .into_iter()
                .filter_map(|(face, _)| solid.face(face)?.origin()?.features().first().copied())
                .collect();
            features.contains(&1) && features.iter().any(|feature| *feature != 1)
        })
        .map(|(id, _)| id)
        .collect()
}

fn revolved(curves: &[ProfileCurve]) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    revolve(
        &Plane::XZ,
        &regions,
        Axis2::new(Point2::ZERO, caditor_geometry::Vector2::Y).unwrap(),
        AngularExtent::full(),
        1,
    )
    .unwrap()
}

fn drilled(solid: &Solid, at: Point3, into: Vector3, radius: f64, depth: f64) -> Solid {
    let plane = Plane::from_frame(at - into, into, into.any_orthonormal_vector()).unwrap();
    let regions = Profile::new(&[ProfileCurve::circle(1, Point2::ZERO, radius)])
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let drill = extrude(
        &plane,
        &regions,
        LinearExtent::one_side(depth + 1.0).unwrap(),
        3,
    )
    .unwrap();
    boolean(solid, &drill, BooleanOperation::Difference).unwrap()
}

struct Feet {
    rounds: Vec<(FaceName, [Surface; 2])>,
}

fn feet_of(solid: &Solid, edges: &[EdgeId]) -> Feet {
    let topology = Topology::new(solid);
    let chain = propagate(solid, &topology, edges).unwrap();
    Feet {
        rounds: chain
            .into_iter()
            .filter_map(|edge| {
                let name = FaceName::blend(50, solid.edge(edge)?.name());
                let faces = edge_faces(solid, edge);
                let [(first, _), (second, _)] = faces.as_slice() else {
                    return None;
                };
                Some((
                    name,
                    [
                        solid.face(*first)?.surface().clone(),
                        solid.face(*second)?.surface().clone(),
                    ],
                ))
            })
            .collect(),
    }
}

impl Feet {
    fn meets(&self, rounding: &Face, other: &Face) -> bool {
        self.rounds.iter().any(|(name, surfaces)| {
            *name == rounding.name()
                && surfaces
                    .iter()
                    .any(|surface| surface.same_surface(other.surface()).is_some())
        })
    }
}

fn assert_tangent_to(name: &str, solid: &Solid, feet: &Feet) {
    let mut checked = 0;
    for (id, edge) in solid.edges() {
        let faces = edge_faces(solid, id);
        let [(first, _), (second, _)] = faces.as_slice() else {
            continue;
        };
        let (Some(first_face), Some(second_face)) = (solid.face(*first), solid.face(*second))
        else {
            continue;
        };
        let rounded = |face: &Face| matches!(face.origin(), Some(FaceOrigin::Fillet { .. }));
        let foot = (rounded(first_face) && feet.meets(first_face, second_face))
            || (rounded(second_face) && feet.meets(second_face, first_face));
        if !foot {
            continue;
        }
        let middle = edge.curve().point(edge.interval().middle());
        let normals = [
            face_normal(solid, *first, middle).unwrap(),
            face_normal(solid, *second, middle).unwrap(),
        ];
        assert!(
            normals[0].angle_between(normals[1]) <= TANGENT,
            "{name}: the fillet meets its face at {} rad at {middle}",
            normals[0].angle_between(normals[1])
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "{name}: no edge of the fillet meets the faces it rounds"
    );
}

fn valid(name: &str, solid: &Solid) {
    check(name, solid, volume(solid));
}

fn lofted(solid: &Solid, edges: &[EdgeId], shape: BlendShape) -> Solid {
    let topology = Topology::new(solid);
    let chosen = propagate(solid, &topology, edges).unwrap();
    let plan = Plan::of(solid, &topology, &chosen, Swept::Lofted).unwrap();
    match apply_plan(solid, &topology, &plan, shape, 50) {
        Ok(result) => result,
        Err(error) => panic!("the lofted blend failed: {error}"),
    }
}

#[test]
fn a_lofted_blend_matches_a_swept_one_where_both_apply() {
    let body = pocketed(&cylinder(), &pocket());
    let before = volume(&body);
    let side = edge_through(&body, (-10.0, 1500.0_f64.sqrt(), 30.0));
    let end = edge_through(&body, (0.0, 40.0, 20.0));

    for (edge, shape) in [
        (side, fillet(2.0)),
        (end, fillet(2.0)),
        (side, chamfer(2.0)),
        (end, chamfer(2.0)),
    ] {
        let swept_away = before - volume(&run(&body, &[edge], shape));
        let lofted = lofted(&body, &[edge], shape);
        let lofted_away = before - volume(&lofted);

        valid("lofted", &lofted);
        assert!(
            (lofted_away - swept_away).abs() <= 2e-3 * swept_away,
            "{shape:?}: the lofted blend removes {lofted_away} where the swept one removes {swept_away}"
        );
    }
}

fn rounded_pocket() -> Solid {
    let body = pocketed(&cylinder(), &pocket());
    let corners: Vec<EdgeId> = [-10.0, 10.0]
        .into_iter()
        .flat_map(|x| {
            [
                edge_through(&body, (x, 34.0, 20.0)),
                edge_through(&body, (x, 34.0, 40.0)),
            ]
        })
        .collect();
    run(&body, &corners, fillet(3.0))
}

#[test]
fn the_rim_of_a_pocket_with_rounded_corners_is_rounded_and_chamfered_all_round() {
    let sharp = pocketed(&cylinder(), &pocket());
    let sharp_rim = rim(&sharp);
    let sharp_away = volume(&sharp) - volume(&run(&sharp, &sharp_rim, fillet(2.0)));
    let body = rounded_pocket();
    let edges = rim(&body);
    let before = volume(&body);
    let feet = feet_of(&body, &edges);

    let rounded = run(&body, &edges[..1], fillet(2.0));
    let rounded_away = before - volume(&rounded);
    let chamfered = run(&body, &edges[..1], chamfer(2.0));
    let chamfered_away = before - volume(&chamfered);

    assert_eq!(edges.len(), 8);
    valid("rounded", &rounded);
    valid("chamfered", &chamfered);
    assert_tangent_to("rounded", &rounded, &feet);
    assert!(
        rounded_away < sharp_away && rounded_away > 0.95 * sharp_away,
        "rounding the rim removes {rounded_away}, the sharp pocket's {sharp_away}"
    );
    assert!(
        chamfered_away > 1.4 * rounded_away,
        "{chamfered_away} {rounded_away}"
    );
}

#[test]
fn the_elliptical_rim_of_a_slanted_pocket_is_rounded_and_chamfered() {
    let turn =
        RigidTransform::rotation_about(Point3::new(0.0, 0.0, 30.0), Vector3::Y, 0.35).unwrap();
    let body = pocketed(&cylinder(), &pocket().transformed(&turn).unwrap());
    let edges = rim(&body);
    let side = edges[..1].to_vec();
    let before = volume(&body);
    let feet = feet_of(&body, &side);

    let rounded = run(&body, &side, fillet(2.0));
    let chamfered = run(&body, &side, chamfer(2.0));
    let rounded_away = before - volume(&rounded);
    let chamfered_away = before - volume(&chamfered);

    assert_eq!(edges.len(), 4);
    assert!(edges.iter().all(|edge| matches!(
        body.edge(*edge).map(|edge| edge.curve()),
        Some(Curve::Ellipse(_))
    )));
    valid("rounded", &rounded);
    valid("chamfered", &chamfered);
    assert_tangent_to("rounded", &rounded, &feet);
    assert!(rounded_away > 0.0 && chamfered_away > 1.2 * rounded_away);
}

fn ringed() -> Solid {
    let ring = revolved(&[ProfileCurve::circle(1, Point2::new(40.0, 0.0), 15.0)]);
    pocketed(&ring, &block_at((-6.0, 45.0, -6.0), (12.0, 20.0, 12.0), 2))
}

fn rounded_alone(name: &str, body: &Solid, edge: EdgeId) {
    let feet = feet_of(body, &[edge]);
    let rounded = run(body, &[edge], fillet(2.0));

    valid(name, &rounded);
    assert_tangent_to(name, &rounded, &feet);
    assert!(volume(&rounded) < volume(body));
}

fn circular(body: &Solid, edge: EdgeId) -> bool {
    matches!(
        body.edge(edge).map(|edge| edge.curve()),
        Some(Curve::Circle(_))
    )
}

#[test]
fn a_pocket_rim_around_a_ring_s_tube_is_rounded() {
    let body = ringed();
    let edge = rim(&body)
        .into_iter()
        .find(|edge| circular(&body, *edge))
        .unwrap();

    rounded_alone("around the tube", &body, edge);
}

#[test]
fn a_pocket_rim_across_a_ring_s_tube_is_rounded() {
    let body = ringed();
    let edge = rim(&body)
        .into_iter()
        .find(|edge| !circular(&body, *edge))
        .unwrap();

    rounded_alone("across the tube", &body, edge);
}

#[test]
fn a_pocket_rim_in_a_ball_is_rounded() {
    let ball = revolved(&[
        line(1, (0.0, -40.0), (0.0, 40.0)),
        arc(2, (0.0, 0.0), (0.0, 40.0), (0.0, -40.0)),
    ]);
    let body = pocketed(
        &ball,
        &block_at((-10.0, 30.0, -10.0), (20.0, 20.0, 20.0), 2),
    );
    let edge = rim(&body)[0];

    rounded_alone("ball", &body, edge);
}

#[test]
fn the_rims_of_a_level_hole_through_tapered_sides_are_rounded_and_chamfered() {
    let curves = [
        line(1, (3.0, 0.0), (17.0, 0.0)),
        arc(2, (17.0, 3.0), (17.0, 0.0), (20.0, 3.0)),
        line(3, (20.0, 3.0), (20.0, 13.0)),
        arc(4, (17.0, 13.0), (20.0, 13.0), (17.0, 16.0)),
        line(5, (17.0, 16.0), (3.0, 16.0)),
        arc(6, (3.0, 13.0), (3.0, 16.0), (0.0, 13.0)),
        line(7, (0.0, 13.0), (0.0, 3.0)),
        arc(8, (3.0, 3.0), (0.0, 3.0), (3.0, 0.0)),
    ];
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let tapered = extrude_tapered(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(20.0).unwrap(),
        7.0_f64.to_radians(),
        1,
    )
    .unwrap();
    let body = drilled(
        &tapered,
        Point3::new(10.0, 2.0, 12.0),
        Vector3::Y,
        2.0,
        30.0,
    );
    let edges: Vec<EdgeId> = rim(&body)
        .into_iter()
        .filter(|edge| {
            matches!(
                body.edge(*edge).map(|edge| edge.curve()),
                Some(Curve::Ellipse(_))
            )
        })
        .collect();
    let before = volume(&body);
    let feet = feet_of(&body, &edges);
    let length: f64 = edges
        .iter()
        .filter_map(|edge| body.edge(*edge))
        .map(|edge| edge.curve().length(edge.interval()))
        .sum();

    let rounded = run(&body, &edges, fillet(0.2));
    let chamfered = run(&body, &edges, chamfer(0.2));
    let rounded_away = before - volume(&rounded);

    assert_eq!(edges.len(), 2);
    valid("rounded", &rounded);
    valid("chamfered", &chamfered);
    assert_tangent_to("rounded", &rounded, &feet);
    assert!(
        (rounded_away - spandrel(0.2) * length).abs() < 0.2 * spandrel(0.2) * length,
        "removed {rounded_away} along {length}"
    );
}

#[test]
fn a_rim_along_a_spline_face_is_rounded_and_chamfered() {
    let bulged = crate::fixtures::spline_topped_block(10.0, 4.0, 3.0);
    let edge = edge_through(&bulged, (5.0, 0.0, 4.0));
    let feet = feet_of(&bulged, &[edge]);

    let rounded = run(&bulged, &[edge], fillet(0.5));
    let chamfered = run(&bulged, &[edge], chamfer(0.5));

    valid("rounded", &rounded);
    valid("chamfered", &chamfered);
    assert_tangent_to("rounded", &rounded, &feet);
    assert!(volume(&chamfered) < volume(&rounded) && volume(&rounded) < volume(&bulged));
}
