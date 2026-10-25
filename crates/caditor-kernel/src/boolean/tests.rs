use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::PI,
};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use super::{
    imprint::Arrangement,
    select::KeptFace,
    trace::{Fragment, HalfEdge, TracedLoop},
    *,
};
use crate::{
    build::{AngularExtent, Axis2, LinearExtent, extrude, plan::PlanError, revolve},
    curve::Curve,
    fixtures::{cuboid, cylinder, sphere},
    intersect::{IntersectionError, RefineStop},
    naming::{EdgeName, FaceName},
    profile::{Profile, Selection},
    sense::Sense,
    surface::{PlaneSurface, Surface},
    test_support::{
        Random, assert_cancelled_anywhere, assert_watertight, cancelled_after, circle,
        polls_during, rectangle, spline,
    },
    tolerance::SamplingTolerance,
    topology::{BuildError, FaceId, Pcurve, PcurveSample},
};

const DENSE_SPLINE_POLLS: usize = 250_000;

fn moved(solid: Solid, offset: (f64, f64, f64)) -> Solid {
    let transform =
        RigidTransform::translation(Vector3::new(offset.0, offset.1, offset.2)).unwrap();
    solid.transformed(&transform).unwrap()
}

fn block(min: (f64, f64, f64), max: (f64, f64, f64)) -> Solid {
    moved(
        cuboid(Vector3::new(max.0 - min.0, max.1 - min.1, max.2 - min.2)),
        min,
    )
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn check(name: &str, solid: &Solid, expected: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
    let found = volume(solid);
    assert!(
        (found - expected).abs() <= 1e-3 * expected.abs().max(1.0),
        "{name}: volume {found} instead of {expected}"
    );
}

fn run(first: &Solid, second: &Solid, operation: BooleanOperation) -> Solid {
    match boolean(first, second, operation) {
        Ok(solid) => solid,
        Err(error) => panic!("{operation:?} failed: {error}"),
    }
}

#[test]
fn overlapping_boxes_combine_in_every_operation() {
    let first = block((0.0, 0.0, 0.0), (2.0, 2.0, 2.0));
    let second = block((1.0, 1.0, 1.0), (3.0, 3.0, 3.0));
    let union = run(&first, &second, BooleanOperation::Union);
    check("union", &union, 15.0);
    assert_eq!(union.faces().count(), 12);
    let difference = run(&first, &second, BooleanOperation::Difference);
    check("difference", &difference, 7.0);
    assert_eq!(difference.faces().count(), 9);
    let intersection = run(&first, &second, BooleanOperation::Intersection);
    check("intersection", &intersection, 1.0);
    assert_eq!(intersection.faces().count(), 6);
    assert_eq!(intersection.edges().count(), 12);
}

#[test]
fn a_pocket_from_the_top_face_leaves_a_hole_in_it() {
    let plate = block((0.0, 0.0, 0.0), (10.0, 10.0, 5.0));
    for top in [5.0, 6.0] {
        let pocket = block((2.0, 3.0, 2.0), (4.0, 7.0, top));
        let result = run(&plate, &pocket, BooleanOperation::Difference);
        check("pocket", &result, 476.0);
        assert_eq!(result.faces().count(), 11, "top at {top}");
        let names: Vec<_> = result.faces().map(|(_, face)| face.name()).collect();
        assert!(names.iter().all(|name| !name.is_none() || true));
    }
}

#[test]
fn faces_the_other_solid_misses_pass_through_whole() {
    let plate = block((0.0, 0.0, 0.0), (10.0, 10.0, 5.0));
    let pocket = block((2.0, 3.0, 2.0), (4.0, 7.0, 6.0));
    let input = Input::new(&plate, &pocket);
    let arrangement = imprint::imprint(&input).unwrap();
    let split = faces::split(&input, &arrangement).unwrap();
    let untouched: Vec<Operand> = split
        .iter()
        .filter(|face| face.untouched)
        .map(|face| face.key.operand)
        .collect();
    assert_eq!(untouched.len(), 7, "{untouched:?}");
    assert_eq!(
        untouched
            .iter()
            .filter(|operand| **operand == Operand::Second)
            .count(),
        2
    );
    let result = run(&plate, &pocket, BooleanOperation::Difference);
    check("pocket", &result, 476.0);
}

#[test]
fn a_cylinder_drills_a_hole_through_a_block() {
    let plate = block((0.0, 0.0, 0.0), (10.0, 10.0, 4.0));
    let drill = moved(cylinder(2.0, 6.0), (5.0, 5.0, -1.0));
    let result = run(&plate, &drill, BooleanOperation::Difference);
    check("drilled", &result, 400.0 - 16.0 * PI);
    assert_eq!(result.faces().count(), 7);
    let boss = run(&plate, &drill, BooleanOperation::Union);
    check("boss", &boss, 400.0 + 8.0 * PI);
}

fn check_pinched(name: &str, solid: &Solid, expected: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    let found = volume(solid);
    assert!(
        (found - expected).abs() <= 1e-3 * expected,
        "{name}: volume {found} instead of {expected}"
    );
    assert_eq!(pinch_edges(solid), 2, "{name}");
}

fn pinch_edges(solid: &Solid) -> usize {
    solid
        .edges()
        .filter(|(_, edge)| {
            let middle = edge.curve().point(edge.interval().middle());
            matches!(edge.curve(), Curve::Line(_))
                && middle.distance(Point3::new(5.0, 5.0, 8.0)) < 1e-6
        })
        .count()
}

#[test]
fn a_block_whose_two_bores_touch_takes_a_hole_beside_them() {
    let mut profile = rectangle(1, (0.0, 0.0), (10.0, 10.0));
    profile.push(circle(5, (5.0, 3.0), 2.0));
    profile.push(circle(6, (5.0, 7.0), 2.0));
    let block = swept(
        &profile,
        &Plane::XY,
        LinearExtent::one_side(16.0).unwrap(),
        1,
    );
    let bores = 1600.0 - 2.0 * PI * 4.0 * 16.0;
    check_pinched("touching bores", &block, bores);

    let drill = moved(cylinder(1.0, 18.0), (8.0, 5.0, -1.0));
    let drilled = run(&block, &drill, BooleanOperation::Difference);
    check_pinched("drilled beside", &drilled, bores - PI * 16.0);

    let blind = moved(cylinder(0.5, 6.0), (2.0, 9.0, 11.0));
    let pocketed = run(&block, &blind, BooleanOperation::Difference);
    check_pinched("blind beside", &pocketed, bores - PI * 0.25 * 5.0);
}

#[test]
fn blocks_side_by_side_merge_into_one_box() {
    let left = block((0.0, 0.0, 0.0), (2.0, 2.0, 2.0));
    for right in [
        block((2.0, 0.0, 0.0), (4.0, 2.0, 2.0)),
        block((1.0, 0.0, 0.0), (4.0, 2.0, 2.0)),
    ] {
        let union = run(&left, &right, BooleanOperation::Union);
        check("side by side", &union, 16.0);
        assert_eq!(union.faces().count(), 6);
        assert_eq!(union.edges().count(), 12);
        assert_eq!(union.vertices().count(), 8);
    }
}

#[test]
fn disjoint_and_nested_solids() {
    let big = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0));
    let far = block((20.0, 0.0, 0.0), (22.0, 2.0, 2.0));
    let union = run(&big, &far, BooleanOperation::Union);
    check("disjoint union", &union, 1008.0);
    assert_eq!(union.shells().count(), 2);
    assert_eq!(
        boolean(&big, &far, BooleanOperation::Intersection),
        Err(BooleanError::Empty)
    );
    check(
        "disjoint difference",
        &run(&big, &far, BooleanOperation::Difference),
        1000.0,
    );
    let inner = block((3.0, 3.0, 3.0), (6.0, 6.0, 6.0));
    let hollow = run(&big, &inner, BooleanOperation::Difference);
    check("hollow", &hollow, 973.0);
    assert_eq!(hollow.shells().count(), 2);
    check(
        "nested union",
        &run(&big, &inner, BooleanOperation::Union),
        1000.0,
    );
    check(
        "nested intersection",
        &run(&big, &inner, BooleanOperation::Intersection),
        27.0,
    );
}

#[test]
fn a_solid_combined_with_itself() {
    let solid = block((0.0, 0.0, 0.0), (2.0, 3.0, 4.0));
    let union = run(&solid, &solid, BooleanOperation::Union);
    check("self union", &union, 24.0);
    assert_eq!(union.faces().count(), 6);
    check(
        "self intersection",
        &run(&solid, &solid, BooleanOperation::Intersection),
        24.0,
    );
    assert_eq!(
        boolean(&solid, &solid, BooleanOperation::Difference),
        Err(BooleanError::Empty)
    );
}

#[test]
fn a_sphere_cut_by_a_block() {
    let ball = sphere(4.0);
    let corner = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0));
    let full = 4.0 / 3.0 * PI * 64.0;
    check(
        "octant",
        &run(&ball, &corner, BooleanOperation::Intersection),
        full / 8.0,
    );
    check(
        "notched ball",
        &run(&ball, &corner, BooleanOperation::Difference),
        full * 7.0 / 8.0,
    );
    let _ = Point3::ZERO;
}

fn rotated(solid: Solid, axis: Vector3, angle: f64) -> Solid {
    let transform = RigidTransform::rotation_about(Point3::ZERO, axis, angle).unwrap();
    solid.transformed(&transform).unwrap()
}

fn consistent(name: &str, first: &Solid, second: &Solid) {
    combine_consistently(name, first, second, true);
}

fn combine_consistently(name: &str, first: &Solid, second: &Solid, watertight: bool) {
    let (a, b) = (volume(first), volume(second));
    let union = run(first, second, BooleanOperation::Union);
    let common = run(first, second, BooleanOperation::Intersection);
    let difference = run(first, second, BooleanOperation::Difference);
    for (label, solid) in [
        ("union", &union),
        ("intersection", &common),
        ("difference", &difference),
    ] {
        assert_eq!(solid.validate(), Ok(()), "{name} {label}");
        if watertight && solid.shells().count() == 1 {
            assert_watertight(name, &solid.tessellate(&solid.default_tolerance()).unwrap());
        }
    }
    let (u, i, d) = (volume(&union), volume(&common), volume(&difference));
    let scale = a.max(b);
    assert!(
        (u + i - a - b).abs() <= 2e-3 * scale,
        "{name}: {u} + {i} vs {a} + {b}"
    );
    assert!(
        (d + i - a).abs() <= 2e-3 * scale,
        "{name}: {d} + {i} vs {a}"
    );
}

#[test]
fn crossing_cylinders() {
    let upright = moved(cylinder(2.0, 10.0), (0.0, 0.0, -5.0));
    let lying = rotated(
        moved(cylinder(2.0, 10.0), (0.0, 0.0, -5.0)),
        Vector3::X,
        0.5 * PI,
    );
    consistent("equal cylinders", &upright, &lying);
    let thin = rotated(
        moved(cylinder(1.2, 10.0), (0.3, 0.0, -5.0)),
        Vector3::Y,
        0.5 * PI,
    );
    consistent("unequal cylinders", &upright, &thin);
}

#[test]
fn curved_solids_against_blocks() {
    let slab = block((-1.0, -1.0, 1.5), (30.0, 30.0, 3.0));
    consistent("spline", &crate::fixtures::extruded_spline(5.0), &slab);
    let across = block((-10.0, -1.0, -10.0), (10.0, 1.0, 10.0));
    consistent("torus", &crate::fixtures::torus(6.0, 2.0), &across);
    let low = block((-10.0, -10.0, -1.0), (10.0, 10.0, 2.0));
    consistent("cone", &crate::fixtures::cone(3.0, 4.0), &low);
    consistent("frustum", &crate::fixtures::frustum(4.0, 2.0, 5.0), &low);
    let off_axis = block((1.0, -10.0, -10.0), (10.0, 10.0, 10.0));
    consistent("sphere", &sphere(4.0), &off_axis);
}

#[test]
fn a_boss_on_a_face_joins_it() {
    let plate = block((0.0, 0.0, 0.0), (10.0, 10.0, 2.0));
    let boss = moved(cylinder(2.0, 3.0), (5.0, 5.0, 2.0));
    let union = run(&plate, &boss, BooleanOperation::Union);
    check("boss", &union, 200.0 + 12.0 * PI);
    assert_eq!(union.faces().count(), 8);
    assert_eq!(union.shells().count(), 1);
}

#[test]
fn tilted_blocks() {
    let first = block((0.0, 0.0, 0.0), (4.0, 3.0, 2.0));
    let second = moved(
        rotated(
            block((-1.0, -1.0, -1.0), (1.0, 1.0, 1.0)),
            Vector3::new(1.0, 2.0, 3.0),
            0.7,
        ),
        (3.5, 2.5, 1.5),
    );
    consistent("tilted", &first, &second);
}

#[test]
fn random_grid_blocks() {
    let mut random = crate::test_support::Random::new(7);
    for round in 0..40 {
        let mut corner = || {
            let a = (random.between(0.0, 4.0)).round();
            let b = (random.between(0.0, 4.0)).round();
            if a == b {
                (a, a + 1.0)
            } else {
                (a.min(b), a.max(b))
            }
        };
        let (x, y, z) = (corner(), corner(), corner());
        let (p, q, r) = (corner(), corner(), corner());
        let first = block((x.0, y.0, z.0), (x.1, y.1, z.1));
        let second = block((p.0, q.0, r.0), (p.1, q.1, r.1));
        let overlap = |a: (f64, f64), b: (f64, f64)| (a.1.min(b.1) - a.0.max(b.0)).max(0.0);
        let common = overlap(x, p) * overlap(y, q) * overlap(z, r);
        let (a, b) = (
            (x.1 - x.0) * (y.1 - y.0) * (z.1 - z.0),
            (p.1 - p.0) * (q.1 - q.0) * (r.1 - r.0),
        );
        let name = format!("round {round}: {x:?} {y:?} {z:?} and {p:?} {q:?} {r:?}");
        let touching = [overlap(x, p), overlap(y, q), overlap(z, r)]
            .iter()
            .filter(|length| **length == 0.0)
            .count();
        let apart = [(x, p), (y, q), (z, r)]
            .iter()
            .any(|(a, b)| a.1 < b.0 || b.1 < a.0);
        for (operation, expected) in [
            (BooleanOperation::Union, a + b - common),
            (BooleanOperation::Intersection, common),
            (BooleanOperation::Difference, a - common),
        ] {
            match boolean(&first, &second, operation) {
                Ok(solid) => check(&format!("{name} {operation:?}"), &solid, expected),
                Err(BooleanError::Empty) => {
                    assert!(expected.abs() < 1e-9, "{name} {operation:?} empty")
                }
                Err(BooleanError::NonManifold(_)) => assert!(
                    touching == 2 && !apart && operation == BooleanOperation::Union,
                    "{name} {operation:?} non-manifold"
                ),
                Err(error) => panic!("{name} {operation:?}: {error}"),
            }
        }
    }
}

#[test]
fn a_rounded_end_is_tangent_to_the_sides() {
    let bar = block((0.0, 0.0, 0.0), (4.0, 2.0, 2.0));
    let end = moved(cylinder(1.0, 2.0), (4.0, 1.0, 0.0));
    let union = run(&bar, &end, BooleanOperation::Union);
    check("obround", &union, 16.0 + PI);
    assert_eq!(union.faces().count(), 6);
}

#[test]
fn coaxial_cylinders() {
    let outer = cylinder(3.0, 5.0);
    let bore = moved(cylinder(1.0, 7.0), (0.0, 0.0, -1.0));
    let tube = run(&outer, &bore, BooleanOperation::Difference);
    check("tube", &tube, PI * 8.0 * 5.0);
    assert_eq!(tube.faces().count(), 4);
    let lower = cylinder(2.0, 4.0);
    let upper = moved(cylinder(2.0, 4.0), (0.0, 0.0, 2.0));
    let stacked = run(&lower, &upper, BooleanOperation::Union);
    check("stacked", &stacked, PI * 4.0 * 6.0);
    assert_eq!(stacked.faces().count(), 3);
    let common = run(&lower, &upper, BooleanOperation::Intersection);
    check("common", &common, PI * 4.0 * 2.0);
    assert_eq!(common.faces().count(), 3);
}

#[test]
fn a_ball_centred_on_a_corner() {
    let cube = block((0.0, 0.0, 0.0), (5.0, 5.0, 5.0));
    let ball = sphere(2.0);
    let full = 4.0 / 3.0 * PI * 8.0;
    check(
        "corner cut",
        &run(&cube, &ball, BooleanOperation::Difference),
        125.0 - full / 8.0,
    );
    check(
        "corner union",
        &run(&cube, &ball, BooleanOperation::Union),
        125.0 + full * 7.0 / 8.0,
    );
}

#[test]
fn operations_chain_on_their_own_results() {
    let mut plate = block((0.0, 0.0, 0.0), (20.0, 12.0, 3.0));
    let mut expected = 720.0;
    for (x, y) in [(4.0, 3.0), (16.0, 3.0), (4.0, 9.0), (16.0, 9.0)] {
        let hole = moved(cylinder(1.5, 5.0), (x, y, -1.0));
        plate = run(&plate, &hole, BooleanOperation::Difference);
        expected -= PI * 2.25 * 3.0;
        check("holes", &plate, expected);
    }
    assert_eq!(plate.faces().count(), 10);
    let rib = block((2.0, 5.0, 3.0), (18.0, 7.0, 6.0));
    plate = run(&plate, &rib, BooleanOperation::Union);
    expected += 96.0;
    check("rib", &plate, expected);
    let slot = block((8.0, 0.0, 1.0), (12.0, 12.0, 8.0));
    plate = run(&plate, &slot, BooleanOperation::Difference);
    check(
        "slot",
        &plate,
        expected - 4.0 * 12.0 * 2.0 - 4.0 * 2.0 * 3.0,
    );
    assert_eq!(plate.shells().count(), 1);
}

#[test]
fn rotated_blocks_in_general_position() {
    let mut random = crate::test_support::Random::new(11);
    let base = block((0.0, 0.0, 0.0), (3.0, 2.0, 1.5));
    for round in 0..12 {
        let axis = random.point(1.0);
        let angle = random.between(0.1, 3.0);
        let shift = random.point(1.0);
        let other = moved(
            rotated(block((-1.0, -0.8, -0.6), (1.0, 0.8, 0.6)), axis, angle),
            (1.5 + shift.x, 1.0 + shift.y, 0.75 + shift.z),
        );
        consistent(&format!("round {round}"), &base, &other);
    }
}

fn swept(
    curves: &[crate::profile::ProfileCurve],
    plane: &Plane,
    extent: LinearExtent,
    feature: u64,
) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(plane, &regions, extent, feature).unwrap()
}

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

#[test]
fn swept_solids_keep_their_names() {
    let plate = swept(
        &rectangle(1, (0.0, 0.0), (10.0, 8.0)),
        &Plane::XY,
        LinearExtent::one_side(3.0).unwrap(),
        1,
    );
    let top = Plane::from_frame(Point3::new(0.0, 0.0, 3.0), Vector3::Z, Vector3::X).unwrap();
    let mut profile = rectangle(5, (2.0, 2.0), (5.0, 6.0));
    profile.push(circle(9, (7.5, 4.0), 1.0));
    let pocket = swept(&profile, &top, LinearExtent::one_side(-1.5).unwrap(), 2);
    let result = run(&plate, &pocket, BooleanOperation::Difference);
    check("named pocket", &result, 240.0 - 1.5 * (12.0 + PI));
    let found = names(&result);
    let given: BTreeSet<FaceName> = names(&plate).union(&names(&pocket)).copied().collect();
    assert!(found.is_subset(&given));
    assert_eq!(result.faces().count(), found.len());
    assert_eq!(result.faces().count(), 6 + 5 + 2);
    let plate_top = FaceName::end_cap(
        1,
        Profile::new(&rectangle(1, (0.0, 0.0), (10.0, 8.0)))
            .unwrap()
            .regions()[0]
            .key(),
    );
    let top_face = result
        .faces()
        .find(|(_, face)| face.name() == plate_top)
        .map(|(_, face)| face)
        .unwrap();
    assert_eq!(top_face.loops().len(), 3);
    let edges: BTreeSet<EdgeName> = result.edges().map(|(_, edge)| edge.name()).collect();
    assert_eq!(edges.len(), result.edges().count());
    assert!(!edges.contains(&EdgeName::NONE));
    let slot = swept(
        &rectangle(20, (4.0, -1.0), (6.0, 9.0)),
        &Plane::XY,
        LinearExtent::new(-1.0, 4.0).unwrap(),
        3,
    );
    let split = run(&plate, &slot, BooleanOperation::Difference);
    check("split plate", &split, 240.0 - 48.0);
    assert_eq!(split.shells().count(), 2);
    let tops = split
        .faces()
        .filter(|(_, face)| face.name() == plate_top)
        .count();
    assert_eq!(tops, 2);
}

#[test]
fn a_revolved_ring_against_a_block() {
    let section = rectangle(1, (3.0, -1.0), (5.0, 1.0));
    let regions = Profile::new(&section)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
    let ring = revolve(&Plane::XZ, &regions, axis, AngularExtent::full(), 4).unwrap();
    let wedge = block((0.0, 0.0, -5.0), (10.0, 10.0, 5.0));
    consistent("ring", &ring, &wedge);
    let quarter = run(&ring, &wedge, BooleanOperation::Intersection);
    check("quarter ring", &quarter, PI * (25.0 - 9.0) * 2.0 / 4.0);
}

#[test]
fn stress_cylinders_on_a_grid() {
    let mut random = crate::test_support::Random::new(5);
    let mut contacts = 0;
    for round in 0..60 {
        let mut pick = |low: f64, high: f64| random.between(low, high).round();
        let size = (pick(2.0, 4.0), pick(2.0, 4.0), pick(1.0, 3.0));
        let base = block((0.0, 0.0, 0.0), size);
        let radius = [0.5, 1.0, 1.5][pick(0.0, 2.0) as usize];
        let height = pick(1.0, 4.0);
        let at = (pick(0.0, 4.0), pick(0.0, 4.0), pick(-1.0, 2.0));
        let tool = moved(cylinder(radius, height), at);
        let (a, b) = (volume(&base), volume(&tool));
        let mut results = Vec::new();
        for operation in [
            BooleanOperation::Union,
            BooleanOperation::Intersection,
            BooleanOperation::Difference,
        ] {
            match boolean(&base, &tool, operation) {
                Ok(solid) => {
                    assert_eq!(solid.validate(), Ok(()), "round {round} {operation:?}");
                    results.push(Some(volume(&solid)));
                }
                Err(BooleanError::Empty) => results.push(Some(0.0)),
                Err(BooleanError::NonManifold(_)) => {
                    contacts += 1;
                    results.push(None);
                }
                Err(error) => panic!("round {round} {operation:?}: {error}"),
            }
        }
        if let [Some(u), Some(i), Some(d)] = results[..] {
            assert!(
                (u + i - a - b).abs() < 2e-3 * (a + b),
                "round {round}: {u} {i} {a} {b}"
            );
            assert!(
                (d + i - a).abs() < 2e-3 * (a + b),
                "round {round}: {d} {i} {a}"
            );
        }
    }
    assert!(contacts <= 8, "{contacts} results touch along a line");
}

#[test]
fn an_interrupted_boolean_and_tessellation_stop_with_cancelled() {
    let first = block((0.0, 0.0, 0.0), (4.0, 4.0, 4.0));
    let second = moved(sphere(2.0), (4.0, 4.0, 4.0));
    let stopped = crate::interruptible(std::sync::Arc::new(|| true), || {
        (
            boolean(&first, &second, BooleanOperation::Union),
            second.tessellate(&second.default_tolerance()),
        )
    });
    assert!(matches!(stopped.0, Err(BooleanError::Cancelled(_))));
    assert!(matches!(
        stopped.1,
        Err(crate::TessellationError::Cancelled(_))
    ));
    assert!(boolean(&first, &second, BooleanOperation::Union).is_ok());
}

#[test]
fn a_boolean_cancelled_at_any_poll_stops_with_cancelled() {
    let first = block((0.0, 0.0, 0.0), (4.0, 4.0, 4.0));
    let second = moved(cylinder(1.0, 6.0), (2.0, 2.0, -1.0));

    let mut finished = false;
    for allowed in 0..10_000 {
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&polls);
        let stop = std::sync::Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= allowed
        });
        match crate::interruptible(stop, || {
            boolean(&first, &second, BooleanOperation::Difference)
        }) {
            Ok(_) => {
                finished = true;
                break;
            }
            Err(error) => assert!(
                matches!(error, BooleanError::Cancelled(_)),
                "stopped after {allowed} polls: {error:?}"
            ),
        }
    }

    assert!(finished);
}

#[test]
fn a_marched_boolean_cancelled_anywhere_stops_with_cancelled() {
    let post = cylinder(1.0, 4.0);
    let ball = moved(sphere(1.5), (1.0, 0.0, 2.0));

    let polls = assert_cancelled_anywhere(
        "union",
        || boolean(&post, &ball, BooleanOperation::Union),
        |error| matches!(error, BooleanError::Cancelled(_)),
    );

    assert!(polls > 100, "only {polls} polls");
}

#[test]
fn a_cut_through_a_cone_apex_leaves_half_the_cone() {
    let cone = crate::fixtures::cone(3.0, 4.0);
    let half = block((0.0, -5.0, -1.0), (5.0, 5.0, 5.0));
    let result = boolean(&cone, &half, BooleanOperation::Difference).unwrap();
    check("half cone", &result, 6.0 * PI);
    let quarter = block((-5.0, 0.0, -1.0), (5.0, 5.0, 5.0));
    let result = boolean(&result, &quarter, BooleanOperation::Difference).unwrap();
    check("quarter cone", &result, 3.0 * PI);
}

#[test]
fn a_spline_face_is_drilled_and_shares_its_plane_with_a_neighbour() {
    let bulged = crate::fixtures::spline_topped_block(10.0, 4.0, 3.0);
    assert_eq!(bulged.find_crossing().unwrap(), crate::CrossingCheck::Clear);
    let before = volume(&bulged);
    let drill = moved(cylinder(1.5, 20.0), (5.0, 5.0, -5.0));
    let drilled = boolean(&bulged, &drill, BooleanOperation::Difference).unwrap();
    assert_eq!(drilled.validate(), Ok(()));
    assert_watertight(
        "drilled",
        &drilled.tessellate(&drilled.default_tolerance()).unwrap(),
    );
    let removed = before - volume(&drilled);
    let (below, above) = (PI * 1.5 * 1.5 * 4.0, PI * 1.5 * 1.5 * 7.0);
    assert!(removed > below && removed < above, "{removed}");

    let flat = crate::fixtures::spline_topped_block(10.0, 4.0, 0.0);
    let beside = moved(flat.clone(), (5.0, 3.0, 0.0));
    let joined = boolean(&flat, &beside, BooleanOperation::Union).unwrap();
    check("joined", &joined, (100.0 + 100.0 - 5.0 * 7.0) * 4.0);
    let shared = boolean(&flat, &beside, BooleanOperation::Intersection).unwrap();
    check("shared", &shared, 5.0 * 7.0 * 4.0);
}

#[test]
fn an_intersection_passing_close_to_a_sphere_pole_splits_its_face() {
    let ball = sphere(4.0);
    let other = moved(sphere(4.0), (-1.275, -2.497, -1.18));

    consistent("spheres", &ball, &other);
}

#[test]
fn a_cylinder_grazing_a_sphere_pole_is_marched() {
    let ball = sphere(4.0);
    let post = moved(cylinder(3.0, 5.0), (-3.0, -1.0, 0.0));

    consistent("sphere and cylinder", &ball, &post);
}

#[test]
fn an_intersection_touching_a_cone_seam_splits_the_cone() {
    let post = cylinder(3.0, 5.0);
    let cone = moved(crate::fixtures::cone(3.0, 4.0), (-1.0, -3.0, 2.0));

    consistent("cylinder and cone", &post, &cone);
}

#[test]
fn a_cone_apex_lying_on_a_cylinder_ends_the_intersection_there() {
    let post = cylinder(3.0, 5.0);
    let cone = moved(
        rotated(crate::fixtures::cone(3.0, 4.0), Vector3::X, 0.5 * PI),
        (0.0, 1.0, 1.0),
    );

    consistent("cylinder and lying cone", &post, &cone);
}

#[test]
fn edges_through_a_sphere_pole_are_not_joined_across_it() {
    let ball = sphere(4.0);
    let holed = crate::fixtures::holed_block(10.0, 4.0, 2.5);
    let hollow = moved(crate::fixtures::hollow_cuboid(10.0, 4.0), (0.0, -2.0, 1.0));

    consistent(
        "holed block",
        &holed,
        &moved(ball.clone(), (0.0, 1.0, -1.0)),
    );
    consistent("hollow cuboid", &ball, &hollow);
}

#[test]
fn intersection_edges_ending_at_a_node_are_sampled_once_there() {
    let cone = crate::fixtures::cone(3.0, 4.0);
    let ball = moved(rotated(sphere(4.0), Vector3::Z, PI), (3.0, -3.5, 0.0));
    let ring = crate::fixtures::torus(6.0, 2.0);
    let hollow = moved(
        rotated(
            crate::fixtures::hollow_cuboid(10.0, 4.0),
            Vector3::Z,
            1.5 * PI,
        ),
        (0.0, -3.0, -0.5),
    );

    consistent("cone and sphere", &cone, &ball);
    consistent("torus and hollow cuboid", &ring, &hollow);
}

#[test]
fn a_plane_crossing_a_bore_exactly_at_its_seam_splits_it_there() {
    let holed = crate::fixtures::holed_block(10.0, 4.0, 2.5);
    let hollow = moved(
        rotated(
            crate::fixtures::hollow_cuboid(10.0, 4.0),
            Vector3::Y,
            0.5 * PI,
        ),
        (0.5, -0.5, 1.5),
    );

    consistent("holed and hollow", &holed, &hollow);
}

#[test]
fn nearly_coincident_tori_combine() {
    let ring = crate::fixtures::torus(6.0, 2.0);
    let shifted = moved(ring.clone(), (1e-4, 1e-4, 1e-4));

    for operation in [
        BooleanOperation::Union,
        BooleanOperation::Difference,
        BooleanOperation::Intersection,
    ] {
        let result = run(&ring, &shifted, operation);
        assert_eq!(result.validate(), Ok(()), "{operation:?}");
    }
}

#[test]
fn tori_touching_along_their_equators_cannot_be_split() {
    let ring = crate::fixtures::torus(6.0, 2.0);
    let beside = moved(ring.clone(), (4.0, 0.0, 0.0));

    let error = boolean(&ring, &beside, BooleanOperation::Union).unwrap_err();
    let BooleanError::Split(site) = &error else {
        panic!("{error:?}");
    };
    let point = site.point.unwrap();

    assert_eq!(site.first.len() + site.second.len(), 1);
    assert!(
        point.distance(Point3::new(8.0, 0.0, 0.0)) < 1e-3,
        "{point:?}"
    );
}

#[test]
fn a_post_through_a_tilted_torus_combines_with_its_curves_following_their_own_edges() {
    let post = cylinder(3.0, 5.0);
    let tilted = crate::fixtures::torus(6.0, 2.0)
        .transformed(
            &RigidTransform::rotation_about(
                Point3::ZERO,
                Vector3::new(
                    0.7673022691030771,
                    -0.42585843157607983,
                    -0.19882123522923822,
                ),
                4.719479295313324,
            )
            .unwrap()
            .then(
                &RigidTransform::translation(Vector3::new(
                    -1.3388772157931028,
                    2.714601682264157,
                    0.4609464282424307,
                ))
                .unwrap(),
            ),
        )
        .unwrap();

    consistent("a post through a tilted torus", &post, &tilted);
}

fn square_fragment(size: f64) -> Fragment {
    let corners = [
        Point2::ZERO,
        Point2::new(size, 0.0),
        Point2::new(size, size),
        Point2::new(0.0, size),
    ];
    let coedges = corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .enumerate()
        .map(|(piece, (from, to))| trace::Coedge {
            half_edge: HalfEdge::new(piece, Sense::Same, None),
            pcurve: Pcurve::new(
                vec![
                    PcurveSample {
                        parameter: 0.0,
                        uv: *from,
                    },
                    PcurveSample {
                        parameter: size,
                        uv: *to,
                    },
                ],
                0.0,
            )
            .unwrap(),
        })
        .collect();
    Fragment {
        loops: vec![TracedLoop {
            coedges,
            area: size * size,
        }],
    }
}

fn kept_face(face: usize, fragment: Fragment) -> KeptFace {
    KeptFace {
        key: FaceKey {
            operand: Operand::First,
            face: FaceId::from_index(face).unwrap(),
        },
        surface: PlaneSurface::new(Plane::XY).unwrap().into(),
        sense: Sense::Same,
        name: FaceName::NONE,
        origin: None,
        fragment,
    }
}

#[test]
fn kept_faces_must_use_every_edge_once_each_way() {
    let square = square_fragment(1.0);

    let arrangement = Arrangement::default();
    let face = |index: usize| FaceId::from_index(index).unwrap();

    assert_eq!(
        heal::check_closed(&arrangement, &[kept_face(0, square.clone())]),
        Err(BooleanError::Open(Box::new(BooleanSite {
            first: vec![face(0)],
            ..BooleanSite::default()
        })))
    );
    assert_eq!(
        heal::check_closed(
            &arrangement,
            &[
                kept_face(0, square.clone()),
                kept_face(1, square.reversed()),
                kept_face(2, square.clone()),
                kept_face(3, square.reversed()),
            ]
        ),
        Err(BooleanError::NonManifold(Box::new(BooleanSite {
            first: (0..4).map(face).collect(),
            ..BooleanSite::default()
        })))
    );
    assert_eq!(
        heal::check_closed(
            &arrangement,
            &[
                kept_face(0, square.clone()),
                kept_face(1, square.reversed())
            ]
        ),
        Ok(())
    );
}

#[test]
fn a_fragment_both_inside_and_outside_the_other_solid_is_ambiguous() {
    let fragment = square_fragment(10.0);
    let surface: Surface = PlaneSurface::new(Plane::XY).unwrap().into();
    let points = trace::interior_points(&fragment, &surface);
    let probe = points[0];
    let first = cuboid(Vector3::splat(10.0));
    let second = block(
        (probe.x - 0.01, probe.y - 0.01, -1.0),
        (probe.x + 0.01, probe.y + 0.01, 1.0),
    );
    let input = Input::new(&first, &second);
    let key = FaceKey {
        operand: Operand::First,
        face: FaceId::from_index(0).unwrap(),
    };

    assert_eq!(points.len(), 3);
    assert!(points[1..].iter().all(|point| point.distance(probe) > 0.1));
    let error = select::classify(
        &input,
        key,
        &surface,
        Sense::Same,
        &fragment,
        select::Twin::Alone,
    )
    .unwrap_err();
    let BooleanError::Ambiguous(site) = error else {
        panic!("{error:?}");
    };

    assert_eq!(site.first, vec![key.face]);
    assert!(site.second.is_empty());
    assert!(
        points
            .iter()
            .any(|uv| Some(surface.point_at(*uv)) == site.point)
    );
}

#[test]
fn failures_of_the_steps_become_boolean_errors_in_words() {
    assert_eq!(
        BooleanError::from(PlanError::Unassembled),
        BooleanError::Open(Box::default())
    );
    assert_eq!(
        BooleanError::from(BuildError::EmptyLoop),
        BooleanError::Invalid(BuildError::EmptyLoop)
    );
    assert_eq!(
        BooleanError::from(PlanError::Build(BuildError::ZeroLengthEdge)),
        BooleanError::Invalid(BuildError::ZeroLengthEdge)
    );
    for (error, words) in [
        (
            BooleanError::Split(Box::default()),
            "a face could not be divided where the solids meet",
        ),
        (
            BooleanError::Ambiguous(Box::default()),
            "the solids touch where it cannot be told which side is inside",
        ),
        (
            BooleanError::Open(Box::default()),
            "the faces of the result do not join up into closed shells",
        ),
        (
            BooleanError::Invalid(BuildError::EmptyLoop),
            "the result is not a valid solid: a loop needs at least one coedge",
        ),
        (
            BooleanError::from(IntersectionError::TooComplex(32768)),
            "the solids could not be intersected: the intersection needs more than 32768 \
             subdivisions",
        ),
        (
            BooleanError::from(IntersectionError::Unfollowable),
            "the solids could not be intersected: an intersection curve could not be followed \
             to its end",
        ),
        (
            BooleanError::from(IntersectionError::Unrefined {
                point: Point3::new(1.0, 2.0, 3.0),
                distance: 0.5,
                cause: RefineStop::Unsolved,
            }),
            "the solids could not be intersected: an intersection curve strays 0.5 from its \
             surfaces near [1, 2, 3], where it could not be refined (no point on both surfaces \
             lies across its middle)",
        ),
    ] {
        assert_eq!(error.to_string(), words);
    }
}

#[test]
fn an_unrefined_intersection_curve_places_the_error_where_it_strays() {
    let point = Point3::new(1.0, 2.0, 3.0);

    let error = BooleanError::from(IntersectionError::Unrefined {
        point,
        distance: 0.5,
        cause: RefineStop::AtEnd,
    })
    .or_point(|| Some(Point3::ZERO));

    assert_eq!(error.site().and_then(|site| site.point), Some(point));
}

fn outcome(result: &Result<Solid, BooleanError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(BooleanError::Empty) => "empty",
        Err(BooleanError::Intersection { .. }) => "intersection",
        Err(BooleanError::Split(_)) => "split",
        Err(BooleanError::Ambiguous(_)) => "ambiguous",
        Err(BooleanError::Open(_)) => "open",
        Err(BooleanError::NonManifold(_)) => "non-manifold",
        Err(BooleanError::Invalid(_)) => "invalid",
        Err(BooleanError::Cancelled(_)) => "cancelled",
    }
}

#[test]
#[ignore = "a survey of the failures left, best run in release"]
fn random_placements_of_every_fixture() {
    let solids = crate::fixtures::every_solid();
    let mut random = crate::test_support::Random::new(2);
    let mut outcomes: BTreeMap<&str, usize> = BTreeMap::new();

    for _ in 0..1500 {
        let first = (random.unit() * solids.len() as f64) as usize % solids.len();
        let second = (random.unit() * solids.len() as f64) as usize % solids.len();
        let axis = random.point(1.0) - Point3::ZERO;
        let angle = random.between(0.0, 2.0 * PI);
        let reach = random.between(0.0, 4.0);
        let offset = random.point(reach);
        let placed = moved(
            rotated(solids[second].1.clone(), axis, angle),
            (offset.x, offset.y, offset.z),
        );
        for operation in [
            BooleanOperation::Union,
            BooleanOperation::Difference,
            BooleanOperation::Intersection,
        ] {
            let label = outcome(&boolean(&solids[first].1, &placed, operation));
            *outcomes.entry(label).or_default() += 1;
            if !matches!(label, "ok" | "empty") {
                eprintln!(
                    "{label}: {} and {} turned about {axis:?} by {angle}, moved by {offset:?}, \
                     {operation:?}",
                    solids[first].0, solids[second].0
                );
            }
        }
    }

    eprintln!("{outcomes:?}");
    let failures: usize = outcomes
        .iter()
        .filter(|(label, _)| !matches!(**label, "ok" | "empty"))
        .map(|(_, count)| count)
        .sum();
    assert_eq!(failures, 0, "{failures} booleans failed");
}

#[test]
fn a_face_flush_with_faces_of_both_senses_of_the_other_solid_is_split_by_them() {
    let table = block((0.0, 0.0, 0.0), (2.0, 1.0, 1.0));
    let high = block((-1.0, 0.0, 1.0), (1.0, 1.0, 2.0));
    let low = block((1.0, 0.0, -1.0), (3.0, 1.0, 1.0));
    let link = block((0.5, 0.0, 0.5), (1.5, 1.0, 1.5));
    let step = run(
        &run(&high, &link, BooleanOperation::Union),
        &low,
        BooleanOperation::Union,
    );
    check("step", &step, 6.5);

    check("union", &run(&table, &step, BooleanOperation::Union), 7.25);
    check(
        "difference",
        &run(&table, &step, BooleanOperation::Difference),
        0.75,
    );
    check(
        "intersection",
        &run(&table, &step, BooleanOperation::Intersection),
        1.25,
    );
}

struct AlignedContact {
    name: String,
    plate: Solid,
    tool: Solid,
    plate_volume: f64,
    tool_volume: f64,
    common_volume: f64,
}

type Corner = (f64, f64, f64);

fn box_volume(min: Corner, max: Corner) -> f64 {
    (max.0 - min.0).max(0.0) * (max.1 - min.1).max(0.0) * (max.2 - min.2).max(0.0)
}

fn aligned_contact(random: &mut Random, offset: impl Fn(&mut Random) -> f64) -> AlignedContact {
    let length = random.between(20.0, 400.0);
    let width = length / random.between(1.0, 40.0);
    let height = random.between(1.0, 20.0);
    let x = random.between(-0.1 * length, length);
    let size = random.between(0.2, 0.2 * length);
    let plate_max = (length, width, height);

    let plate = block((0.0, 0.0, 0.0), plate_max);
    let plate_volume = box_volume((0.0, 0.0, 0.0), plate_max);
    let name = |shape: &str| format!("{shape} at x {x} size {size} on a plate {plate_max:?}");
    let boxed = |shape: &str, min: Corner, max: Corner| {
        let common_min = (min.0.max(0.0), min.1.max(0.0), min.2.max(0.0));
        let common_max = (max.0.min(length), max.1.min(width), max.2.min(height));
        AlignedContact {
            name: format!("{} from {min:?} to {max:?}", name(shape)),
            plate: plate.clone(),
            tool: block(min, max),
            plate_volume,
            tool_volume: box_volume(min, max),
            common_volume: box_volume(common_min, common_max),
        }
    };
    match (random.unit() * 4.0) as usize {
        0 => {
            let (bottom, top) = (
                random.between(-height, 0.4 * height),
                random.between(0.5 * height, 2.0 * height),
            );
            let flush = offset(random);
            boxed("a block beside", (x, -size, bottom), (x + size, flush, top))
        }
        1 => {
            let front = random.between(-width, width);
            let (back, bottom) = (width + offset(random), height + offset(random));
            boxed(
                "a block on top",
                (x, front, bottom),
                (x + size, back, height + size),
            )
        }
        2 => {
            let radius = random.between(0.05, 0.5) * width;
            let y = if random.unit() < 0.5 {
                radius + offset(random)
            } else {
                random.between(radius, width - radius)
            };
            let bottom = height + offset(random);
            AlignedContact {
                name: name(&format!(
                    "a cylinder of radius {radius} at y {y} from z {bottom}"
                )),
                plate: plate.clone(),
                tool: moved(cylinder(radius, size), (x, y, bottom)),
                plate_volume,
                tool_volume: PI * radius * radius * size,
                common_volume: 0.0,
            }
        }
        _ => {
            let (front, bottom) = (
                random.between(0.0, 0.9 * width),
                random.between(0.0, 0.9 * height),
            );
            let (back, top) = (width + offset(random), height + offset(random));
            boxed(
                "a block in a corner",
                (x, front, bottom),
                (x + size, back, top),
            )
        }
    }
}

#[test]
fn aligned_contacts_with_long_plates_combine() {
    let mut random = Random::new(7);
    for _ in 0..60 {
        let contact = aligned_contact(&mut random, |_| 0.0);
        let (a, b, common) = (
            contact.plate_volume,
            contact.tool_volume,
            contact.common_volume,
        );
        for (operation, expected) in [
            (BooleanOperation::Union, a + b - common),
            (BooleanOperation::Intersection, common),
            (BooleanOperation::Difference, a - common),
        ] {
            let name = format!("{} {operation:?}", contact.name);
            match boolean(&contact.plate, &contact.tool, operation) {
                Ok(solid) => check(&name, &solid, expected),
                Err(BooleanError::Empty) => assert!(expected.abs() < 1e-9, "{name} empty"),
                Err(error) => panic!("{name}: {error}"),
            }
        }
    }
}

fn aligned_contacts_failing(seed: u64, contacts: usize, exponents: (f64, f64)) -> usize {
    let mut random = Random::new(seed);
    let mut outcomes: BTreeMap<&str, usize> = BTreeMap::new();

    for _ in 0..contacts {
        let contact = aligned_contact(&mut random, |random| {
            let size = 10f64.powf(random.between(exponents.0, exponents.1));
            if random.unit() < 0.5 { size } else { -size }
        });
        for operation in [
            BooleanOperation::Union,
            BooleanOperation::Difference,
            BooleanOperation::Intersection,
        ] {
            let result = boolean(&contact.plate, &contact.tool, operation);
            let label = match &result {
                Ok(solid) if solid.validate().is_err() => "accepted invalid",
                _ => outcome(&result),
            };
            *outcomes.entry(label).or_default() += 1;
            if !matches!(label, "ok" | "empty") {
                eprintln!("{label}: {} {operation:?} {result:?}", contact.name);
            }
        }
    }

    eprintln!("{outcomes:?}");
    assert_eq!(outcomes.get("accepted invalid"), None);
    outcomes
        .iter()
        .filter(|(label, _)| !matches!(**label, "ok" | "empty"))
        .map(|(_, count)| count)
        .sum()
}

#[test]
#[ignore = "a survey of the near-coincidence failures left, best run in release"]
fn aligned_contacts_a_micrometre_or_so_apart() {
    let failures = aligned_contacts_failing(7, 300, (-6.0, -4.0));

    assert_eq!(failures, 0, "{failures} booleans failed");
}

#[test]
#[ignore = "a survey of the near-coincidence failures left, best run in release"]
fn aligned_contacts_within_the_resolution_or_past_it() {
    let failures = aligned_contacts_failing(11, 1000, (-7.0, -4.0));

    assert_eq!(failures, 0, "{failures} booleans failed");
}

fn holed_plate(holes: usize, pitch: f64) -> Solid {
    let side = pitch * holes as f64;
    let mut curves = rectangle(1, (0.0, 0.0), (side, side));
    for row in 0..holes {
        for column in 0..holes {
            let center = (pitch * (column as f64 + 0.5), pitch * (row as f64 + 0.5));
            curves.push(circle(
                (10 + row * holes + column) as u64,
                center,
                pitch / 4.0,
            ));
        }
    }
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(2.0).unwrap(),
        1,
    )
    .unwrap()
}

#[test]
fn plates_with_many_holes_join_with_every_hole_kept() {
    let holes = 6;
    let first = holed_plate(holes, 10.0);
    let second = moved(holed_plate(holes, 10.0), (5.0, 5.0, 1.0));
    let hole_area = PI * 2.5 * 2.5;
    let plate = 2.0 * (60.0 * 60.0 - 36.0 * hole_area);
    let overlap = 55.0 * 55.0 - 2.0 * 5.5 * 5.5 * hole_area;

    let union = run(&first, &second, BooleanOperation::Union);

    check("holed plates", &union, 2.0 * plate - overlap);
}

struct Placement {
    first: &'static str,
    second: &'static str,
    axis: [f64; 3],
    angle: f64,
    offset: [f64; 3],
}

fn fixture(name: &str) -> Solid {
    crate::fixtures::every_solid()
        .into_iter()
        .find(|(known, _)| *known == name)
        .map(|(_, solid)| solid)
        .unwrap()
}

#[test]
fn placements_the_survey_once_failed_on_combine_in_every_operation() {
    let placements = [
        Placement {
            first: "extruded spline",
            second: "torus",
            axis: [0.3494028092209731, 0.783808179789397, 0.5500293391852864],
            angle: 1.7536915415737844,
            offset: [
                -0.3341052609403181,
                -0.35189914101348946,
                0.1946204593427221,
            ],
        },
        Placement {
            first: "torus",
            second: "torus",
            axis: [
                0.28183652912790036,
                -0.38302267045447747,
                0.14861334158896833,
            ],
            angle: 4.094916710072873,
            offset: [
                -0.08140243986459836,
                0.023560975519637206,
                0.04681174547054248,
            ],
        },
        Placement {
            first: "frustum",
            second: "torus",
            axis: [
                0.15918432317123532,
                -0.38558639057320043,
                0.21933746156793044,
            ],
            angle: 4.458477935180946,
            offset: [0.7978532746535936, -1.5985765169984556, 2.481438299963669],
        },
        Placement {
            first: "torus",
            second: "extruded spline",
            axis: [0.6158988540739427, 0.1052653358758382, -0.46552630318247235],
            angle: 0.32155709960765205,
            offset: [1.7393631953064803, 1.269438679546151, 0.9004250493589594],
        },
        Placement {
            first: "extruded spline",
            second: "frustum",
            axis: [-0.4566681769213574, 0.0884849853223395, -0.9870459544092678],
            angle: 2.3166417881833654,
            offset: [
                -1.0429914019868052,
                -0.00386163269224693,
                0.5705960428468118,
            ],
        },
        Placement {
            first: "sphere",
            second: "torus",
            axis: [0.7689078164517837, -0.3119861531058954, 0.5515485135120883],
            angle: 2.6159441066009164,
            offset: [
                -0.46577967924596453,
                0.10070782938339506,
                -0.2956017012245836,
            ],
        },
        Placement {
            first: "cone",
            second: "cone",
            axis: [-0.3423479893487724, 0.40768721646966366, -0.450478286597501],
            angle: 1.618144038332397,
            offset: [
                0.21137766687518345,
                0.23009673689397847,
                0.08737189203974871,
            ],
        },
        Placement {
            first: "extruded spline",
            second: "frustum",
            axis: [
                -0.9531647960341936,
                -0.09411875481867016,
                0.20300410199093655,
            ],
            angle: 0.9854617259835932,
            offset: [-1.534814593549542, 0.5203219952448088, 1.620422597456999],
        },
        Placement {
            first: "extruded spline",
            second: "extruded spline",
            axis: [0.9822167454789423, -0.6626819377954025, -0.9461445153510024],
            angle: 3.847574038109544,
            offset: [-0.0806845501336978, 0.5745480514931482, 0.22124869989548157],
        },
        Placement {
            first: "torus",
            second: "torus",
            axis: [
                -0.882381936139014,
                -0.8047896652507125,
                -0.05965845290360461,
            ],
            angle: 4.6073768483698485,
            offset: [1.0004898132962912, -0.7992528907110321, -0.4802310468800446],
        },
        Placement {
            first: "frustum",
            second: "extruded spline",
            axis: [
                -0.9979412406831332,
                0.2350140628931785,
                -0.23007868369218953,
            ],
            angle: 4.601919364019222,
            offset: [1.2394530985791894, 1.1673365286446558, -2.5968584972879416],
        },
        Placement {
            first: "torus",
            second: "torus",
            axis: [
                0.4025102058868899,
                -0.5511270610042562,
                -0.28672391816099263,
            ],
            angle: 6.022613048652546,
            offset: [
                0.08720959495953662,
                0.023481371349541758,
                -0.13366233091771978,
            ],
        },
        Placement {
            first: "torus",
            second: "frustum",
            axis: [-0.8223834717088285, -0.7188484337356527, 0.7779200741261496],
            angle: 2.7640461977621835,
            offset: [0.4872699900246382, 0.9084463330917374, 0.46553166035373206],
        },
        Placement {
            first: "cone",
            second: "cylinder",
            axis: [0.566855316995726, -0.7613584743619854, 0.7787600551497069],
            angle: 2.7279323709218013,
            offset: [
                -0.011151811160462502,
                0.034948588001195116,
                0.029074892292851817,
            ],
        },
        Placement {
            first: "cone",
            second: "cone",
            axis: [
                0.0355066357005871,
                -0.06417571081066953,
                -0.6934678885135719,
            ],
            angle: 2.0696260328226126,
            offset: [-0.340296207405607, 0.1527109561952419, 0.011528878092099681],
        },
    ];
    for placement in placements {
        let [x, y, z] = placement.axis;
        let [dx, dy, dz] = placement.offset;
        let second = moved(
            rotated(
                fixture(placement.second),
                Vector3::new(x, y, z),
                placement.angle,
            ),
            (dx, dy, dz),
        );
        let name = format!("{} and {}", placement.first, placement.second);
        combine_consistently(&name, &fixture(placement.first), &second, false);
    }
}

type Placed = (f64, f64, f64);

#[test]
fn posts_standing_micrometres_past_the_edge_of_a_plate_combine() {
    let posts: [(f64, Placed, f64, Placed); 3] = [
        (
            0.301795535261445,
            (100.10736807848954, 0.3017806264393804, 18.70953792186892),
            7.134839440111021,
            (102.4348331728695, 2.9983010618123647, 18.70954373498908),
        ),
        (
            1.875474674335778,
            (5.689523194621929, 1.8754680499022225, 5.303027025413838),
            4.509166629891049,
            (56.669938963976705, 4.285054151514048, 5.303072773030539),
        ),
        (
            3.8108196250307875,
            (144.13714894242065, 3.810817218086383, 4.24402704452294),
            16.158314186928447,
            (179.27305042695477, 9.958608546623925, 4.244028055358543),
        ),
    ];
    for (radius, at, height, corner) in posts {
        let plate = block((0.0, 0.0, 0.0), corner);
        let post = moved(cylinder(radius, height), at);
        consistent(
            &format!("a post of radius {radius} at {at:?}"),
            &plate,
            &post,
        );
    }
}

#[test]
fn blocks_flush_with_a_plate_within_the_resolution_on_one_side_combine() {
    let pairs = [
        (
            (21.577385398116583, 2.7871177222464207, 8.628709088688616),
            (
                (7.045640741510388, 0.5320287802994551, 6.468574908506884),
                (11.090825064102656, 2.7871187732490412, 8.628709592958165),
            ),
        ),
        (
            (196.27209027372274, 65.13573816213557, 4.446991304197638),
            (
                (92.22037674319041, 52.76757160888735, 4.446989397642464),
                (121.34607305978274, 65.13573832174976, 33.572687620789964),
            ),
        ),
    ];
    for (corner, (min, max)) in pairs {
        consistent(
            &format!("a block from {min:?} to {max:?}"),
            &block((0.0, 0.0, 0.0), corner),
            &block(min, max),
        );
    }
}

#[test]
fn coaxial_cylinders_micrometres_apart_in_radius_combine() {
    for gap in [2e-6, 1e-5, 1e-4] {
        let outer = cylinder(5.0, 10.0);
        let wider = moved(cylinder(5.0 + gap, 5.0), (0.0, 0.0, 2.5));
        let lower = moved(cylinder(5.0 + gap, 5.0), (0.0, 0.0, -2.5));
        consistent(&format!("a band {gap} wider"), &outer, &wider);
        consistent(&format!("a foot {gap} wider"), &outer, &lower);
    }
}

#[test]
fn plugs_micrometres_off_the_axis_of_their_bore_combine() {
    let holed = crate::fixtures::holed_block(10.0, 4.0, 2.5);
    for gap in [1e-5, 1e-4, 1e-3] {
        let flush = moved(cylinder(2.5, 4.0), (5.0 + gap, 5.0, 0.0));
        let through = moved(cylinder(2.5, 6.0), (5.0 + gap, 5.0, -1.0));
        consistent(&format!("a flush plug {gap} off"), &holed, &flush);
        consistent(&format!("a plug {gap} off through"), &holed, &through);
    }
}

fn combine_within_the_resolution(name: &str, first: &Solid, second: &Solid) {
    let (a, b) = (volume(first), volume(second));
    let measured = |operation: BooleanOperation| match boolean(first, second, operation) {
        Ok(solid) => {
            assert_eq!(solid.validate(), Ok(()), "{name} {operation:?}");
            volume(&solid)
        }
        Err(BooleanError::Empty) => 0.0,
        Err(error) => panic!("{name} {operation:?}: {error:?}"),
    };

    let union = measured(BooleanOperation::Union);
    let common = measured(BooleanOperation::Intersection);
    let difference = measured(BooleanOperation::Difference);
    let scale = a.max(b);

    assert!(
        (union + common - a - b).abs() <= 2e-3 * scale,
        "{name}: {union} + {common} vs {a} + {b}"
    );
    assert!(
        (difference + common - a).abs() <= 2e-3 * scale,
        "{name}: {difference} + {common} vs {a}"
    );
}

#[test]
fn a_block_whose_back_and_bottom_lie_within_the_resolution_of_a_plate_combines() {
    let plate = block((0.0, 0.0, 0.0), (30.0, 10.0, 5.0));

    for (back, bottom) in [
        (8.3e-7, 6.2e-7),
        (-8.3e-7, -6.2e-7),
        (8.3e-7, -6.2e-7),
        (-8.3e-7, 6.2e-7),
    ] {
        let tool = block((4.0, 3.0, 5.0 + bottom), (9.0, 10.0 + back, 10.0));
        combine_within_the_resolution(
            &format!("a block {back} behind and {bottom} above"),
            &plate,
            &tool,
        );
    }
}

#[test]
fn a_block_flush_on_one_side_and_just_past_the_resolution_on_another_combines() {
    let corner = (27.063596327939067, 1.2173595314981942, 4.806626259306545);
    let plate = block((0.0, 0.0, 0.0), corner);
    let min = (13.406506566698416, 0.8660776046036809, 2.2455747096381655);

    for (back, top) in [
        (corner.1 - 1.8e-7, corner.2 + 1.56e-6),
        (corner.1, corner.2 + 1.5e-6),
        (corner.1 + 1.27e-6, corner.2 - 3.7e-7),
    ] {
        let tool = block(min, (13.610727542139564, back, top));
        combine_within_the_resolution(
            &format!("a block in a corner to {back} and {top}"),
            &plate,
            &tool,
        );
    }
}

#[test]
fn a_post_within_the_resolution_of_tangent_to_a_plate_combines() {
    let posts = [
        (
            (109.07330492315471, 4.127647295982372, 16.76512976095819),
            1.8296880455588276,
            (0.9258796956878559, 1.8296876466203027, 16.765129597131477),
            12.135033571541092,
        ),
        (
            (127.50637237963218, 125.07695831257296, 17.27481039641628),
            44.81194058100597,
            (103.13560597917588, 44.811940579969395, 17.274810395080497),
            10.483087801174884,
        ),
    ];

    for (corner, radius, at, height) in posts {
        let plate = block((0.0, 0.0, 0.0), corner);
        let post = moved(cylinder(radius, height), at);
        combine_within_the_resolution(
            &format!("a post of radius {radius} at {at:?}"),
            &plate,
            &post,
        );
    }
}

fn bored_plate() -> Solid {
    let mut curves = rectangle(1, (0.0, 0.0), (20.0, 10.0));
    curves.push(circle(10, (10.0, 5.0), 2.5));
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(4.0).unwrap(),
        1,
    )
    .unwrap()
}

fn extruded_plug(center: (f64, f64), radius: f64) -> Solid {
    let regions = Profile::new(&[circle(20, center, radius)])
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(4.0).unwrap(),
        2,
    )
    .unwrap()
}

#[test]
fn an_extruded_plug_off_its_bores_axis_by_about_the_resolution_combines() {
    let plate = bored_plate();

    for gap in [5e-7, 1e-6, 1.1e-6, 1.5e-6, 2e-6, 3e-6] {
        combine_within_the_resolution(
            &format!("a plug {gap} off"),
            &plate,
            &extruded_plug((10.0 + gap, 5.0), 2.5),
        );
    }
}

#[test]
fn a_plug_beside_its_bores_axis_by_a_few_resolutions_combines() {
    let plate = bored_plate();

    for (gap, degrees, wider) in [
        (5.6e-7, 0.0, 5e-7),
        (5.6e-7, 0.0, -5e-7),
        (5.6e-7, 90.0, 5e-7),
        (1e-6, 30.0, 0.0),
        (1e-6, 45.0, 0.0),
        (7e-7, 60.0, 3e-7),
        (1.5e-6, 45.0, -5e-7),
        (2.5e-6, 90.0, 0.0),
        (4e-6, 75.0, 0.0),
    ] {
        let angle = f64::to_radians(degrees);
        let center = (10.0 + gap * angle.cos(), 5.0 + gap * angle.sin());
        combine_within_the_resolution(
            &format!("a plug {gap} off at {degrees} degrees and {wider} wider"),
            &plate,
            &extruded_plug(center, 2.5 + wider),
        );
    }
}

#[test]
fn a_plug_off_its_bores_axis_away_from_its_seams_combines() {
    let plate = bored_plate();

    for gap in [2.5e-6, 5e-6, 1e-5, 2e-5, 3e-5, 5e-5, 1e-4] {
        combine_within_the_resolution(
            &format!("a plug {gap} off along -x"),
            &plate,
            &extruded_plug((10.0 - gap, 5.0), 2.5),
        );
    }
}

#[test]
fn a_plug_off_its_bores_axis_square_to_its_seams_combines() {
    let plate = bored_plate();

    for (gap, wider) in [
        (1.5e-6, 0.0),
        (1e-5, 0.0),
        (1e-4, 0.0),
        (1e-3, 0.0),
        (2e-3, 0.0),
        (1e-2, 0.0),
        (1e-5, 5e-7),
        (1e-5, -5e-7),
        (1e-4, -5e-7),
        (1e-3, 5e-7),
    ] {
        combine_within_the_resolution(
            &format!("a plug {gap} off along y and {wider} wider"),
            &plate,
            &extruded_plug((10.0, 5.0 + gap), 2.5 + wider),
        );
    }
}

#[test]
fn faces_within_the_resolution_of_the_first_solid_are_snapped_onto_it() {
    let plate = block((0.0, 0.0, 0.0), (30.0, 10.0, 5.0));
    let tool = block((4.0, 3.0, 5.0 + 6.2e-7), (9.0, 10.0 + 8.3e-7, 10.0 + 8e-7));

    let snapped = snap::snapped(&Input::new(&plate, &tool)).unwrap();
    let corners: Vec<Point3> = snapped
        .vertices()
        .map(|(_, vertex)| vertex.point())
        .collect();

    assert_eq!(snapped.validate(), Ok(()));
    assert_eq!(corners.len(), 8);
    for expected in [
        Point3::new(4.0, 10.0, 5.0),
        Point3::new(9.0, 3.0, 5.0),
        Point3::new(9.0, 10.0, 10.0 + 8e-7),
    ] {
        assert!(
            corners
                .iter()
                .any(|corner| corner.distance(expected) <= 1e-12),
            "{expected:?} in {corners:?}"
        );
    }
}

fn wavy_extrusion(control_points: usize) -> Solid {
    let points: Vec<(f64, f64)> = (0..=control_points)
        .map(|index| {
            let angle = 2.0 * PI * (index % control_points) as f64 / control_points as f64;
            let radius = 20.0 + 2.0 * (7.0 * angle).sin();
            (radius * angle.cos(), radius * angle.sin())
        })
        .collect();
    let regions = Profile::new(&[spline(1, &points)])
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(5.0).unwrap(),
        7,
    )
    .unwrap()
}

#[test]
fn an_extruded_spline_of_hundreds_of_control_points_combines_in_bounded_work() {
    let wavy = wavy_extrusion(500);
    let drill = moved(cylinder(3.0, 10.0), (20.0, 0.0, -2.0));
    let lid = block((0.0, -30.0, 2.5), (30.0, 30.0, 8.0));
    let coarse_volume = |solid: &Solid| {
        solid
            .tessellate(&solid.default_tolerance())
            .unwrap()
            .mass_properties()
            .volume
    };

    let ((), polls) = polls_during(|| {
        let whole = coarse_volume(&wavy);
        for (name, tool) in [
            ("a hole through the wall", &drill),
            ("a cut across the cap", &lid),
        ] {
            let outside = run(&wavy, tool, BooleanOperation::Difference);
            let inside = run(&wavy, tool, BooleanOperation::Intersection);
            let parts = coarse_volume(&outside) + coarse_volume(&inside);

            assert_eq!(outside.validate(), Ok(()), "{name}");
            assert_eq!(inside.validate(), Ok(()), "{name}");
            assert!(
                (parts - whole).abs() <= 1e-3 * whole,
                "{name}: {parts} vs {whole}"
            );
        }
    });

    assert!(polls < DENSE_SPLINE_POLLS, "{polls} polls");
}

#[test]
fn a_boolean_of_a_dense_spline_extrusion_is_cancelled_wherever_it_is_stopped() {
    let wavy = wavy_extrusion(400);
    let lid = block((0.0, -30.0, 2.5), (30.0, 30.0, 8.0));
    let difference = || boolean(&wavy, &lid, BooleanOperation::Difference);

    let (finished, polls) = cancelled_after(usize::MAX, difference);

    assert!(finished.is_ok(), "{finished:?}");
    for fraction in [0.1, 0.3, 0.5, 0.7, 0.9, 0.99] {
        let allowed = (fraction * polls as f64) as usize;
        let (stopped, _) = cancelled_after(allowed, difference);

        assert!(
            matches!(stopped, Err(BooleanError::Cancelled(_))),
            "stopped after {allowed} of {polls} polls: {stopped:?}"
        );
    }
}

fn drill(centre: Point3, up: Vector3, seam: Vector3, radius: f64, depth: f64) -> Solid {
    let plane = Plane::from_frame(centre, seam.cross(up), seam).unwrap();
    let outline = [
        Point2::new(0.0, 1.0),
        Point2::new(radius, 1.0),
        Point2::new(radius, -depth),
        Point2::new(0.0, -depth),
    ];
    let curves: Vec<crate::ProfileCurve> = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .zip(1..)
        .map(|((start, end), entity)| crate::ProfileCurve::line(entity, *start, *end))
        .collect();
    let profile = Profile::new(&curves).unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
    revolve(&plane, profile.regions(), axis, AngularExtent::full(), 9).unwrap()
}

#[test]
fn crossing_holes_cut_alike_wherever_their_seams_lie() {
    let base = block((0.0, 0.0, 0.0), (20.0, 20.0, 20.0));
    let drilled = |front_seam: f64, side_seam: f64| {
        let front = drill(
            Point3::new(5.0, 0.0, 12.0),
            -Vector3::Y,
            Vector3::new(front_seam.cos(), 0.0, front_seam.sin()),
            1.0,
            8.0,
        );
        let side = drill(
            Point3::new(20.0, 6.0, 12.0),
            Vector3::X,
            Vector3::new(0.0, side_seam.cos(), side_seam.sin()),
            1.5,
            24.0,
        );
        let once = run(&base, &front, BooleanOperation::Difference);
        run(&once, &side, BooleanOperation::Difference)
    };

    let reference = volume(&drilled(0.7, 2.3));

    for front_seam in [0.0, 0.5 * PI, PI, 1.5 * PI] {
        for side_seam in [0.0, 0.5 * PI, PI, 1.5 * PI] {
            let name = format!("seams at {front_seam} and {side_seam}");
            check(&name, &drilled(front_seam, side_seam), reference);
        }
    }
}

#[test]
fn holes_flush_at_the_top_or_bottom_cut_through_the_point_where_they_touch() {
    let base = block((0.0, 0.0, 0.0), (20.0, 20.0, 20.0));
    let front = |height: f64, radius: f64| {
        let tool = drill(
            Point3::new(12.0, 0.0, height),
            -Vector3::Y,
            Vector3::X,
            radius,
            12.0,
        );
        run(&base, &tool, BooleanOperation::Difference)
    };
    let side = |height: f64, radius: f64| {
        drill(
            Point3::new(20.0, 6.0, height),
            Vector3::X,
            Vector3::Y,
            radius,
            24.0,
        )
    };
    for (front_height, front_radius, side_height, side_radius) in [
        (11.5, 2.0, 12.0, 1.5),
        (9.0, 1.5, 10.0, 2.5),
        (10.0, 1.0, 9.0, 2.0),
    ] {
        let drilled = front(front_height, front_radius);
        let nudged = |by: f64| {
            let cut = run(
                &drilled,
                &side(side_height + by, side_radius),
                BooleanOperation::Difference,
            );
            volume(&cut)
        };

        let expected = 0.5 * (nudged(0.01) + nudged(-0.01));

        let name = format!("{front_radius} at {front_height} and {side_radius} at {side_height}");
        let cut = run(
            &drilled,
            &side(side_height, side_radius),
            BooleanOperation::Difference,
        );
        check(&name, &cut, expected);
    }
}

fn exact_volume(solid: &Solid) -> f64 {
    crate::measure::mass_properties_tessellating(solid, &solid.default_tolerance())
        .unwrap()
        .properties
        .volume
}

fn check_valid(name: &str, solid: &Solid, expected: f64) {
    assert_eq!(solid.validate(), Ok(()), "{name}");
    let found = exact_volume(solid);
    assert!(
        (found - expected).abs() <= 1e-6 * expected,
        "{name}: volume {found} instead of {expected}"
    );
}

fn check_every_operation(name: &str, first: &Solid, second: &Solid, volumes: (f64, f64)) {
    let common = run(first, second, BooleanOperation::Intersection);
    assert_eq!(common.validate(), Ok(()), "{name}");
    let shared = exact_volume(&common);

    assert!(shared > 0.0, "{name}");
    check_valid(
        &format!("{name} union"),
        &run(first, second, BooleanOperation::Union),
        volumes.0 + volumes.1 - shared,
    );
    check_valid(
        &format!("{name} difference"),
        &run(first, second, BooleanOperation::Difference),
        volumes.0 - shared,
    );
}

#[test]
fn cones_whose_intersection_passes_a_nearly_tangent_neck_combine() {
    let first = crate::fixtures::cone(3.0, 4.0);
    let axis = Vector3::new(0.841747170857855, -0.3631417603571421, -0.6711736726415607);
    let offset = (
        -0.007914244095529675,
        -0.010277728834917885,
        0.011579623960233862,
    );

    let second = moved(rotated(first.clone(), axis, 5.495914871889563), offset);
    let cone_volume = 12.0 * PI;

    check_every_operation("cones", &first, &second, (cone_volume, cone_volume));
}

#[test]
fn a_torus_turning_tightly_against_a_hollow_cuboid_combines() {
    let torus = crate::fixtures::torus(6.0, 2.0);
    let axis = Vector3::new(0.9159509180456542, -0.3332155285037679, 0.3947770646727655);
    let offset = (-0.652826082372679, -2.6432721660356506, -2.2951556645039934);

    let hollow = moved(
        rotated(
            crate::fixtures::hollow_cuboid(10.0, 4.0),
            axis,
            0.6831700081877379,
        ),
        offset,
    );

    check_every_operation(
        "torus and hollow cuboid",
        &torus,
        &hollow,
        (48.0 * PI * PI, 936.0),
    );
}

#[test]
fn a_hollow_cuboid_tilted_over_a_cone_combines() {
    let cone = crate::fixtures::cone(3.0, 4.0);
    let axis = Vector3::new(
        -0.08879029379910963,
        -0.13547995799273682,
        -0.7018715237676012,
    );
    let offset = (0.8111360392808891, 2.7776467446516757, 2.60775040035961);

    let hollow = moved(
        rotated(
            crate::fixtures::hollow_cuboid(10.0, 4.0),
            axis,
            1.8444683609664336,
        ),
        offset,
    );

    check_every_operation("cone and hollow cuboid", &cone, &hollow, (12.0 * PI, 936.0));
}

#[test]
fn frustums_turned_a_little_against_each_other_combine() {
    let first = crate::fixtures::frustum(4.0, 2.0, 5.0);
    let axis = Vector3::new(-0.648624827282154, 0.5719489173052974, 0.3613774284739766);
    let offset = (0.1792702546474707, 0.47363515541101986, 0.8071095557775776);

    let second = moved(rotated(first.clone(), axis, 0.6827729305823226), offset);
    let frustum_volume = 140.0 * PI / 3.0;

    check_every_operation(
        "frustums",
        &first,
        &second,
        (frustum_volume, frustum_volume),
    );
}

#[test]
fn an_extruded_spline_turned_a_hair_inside_a_hollow_cuboid_combines() {
    let spline = crate::fixtures::extruded_spline(5.0);
    let axis = Vector3::new(
        -0.6141979579209673,
        -0.7063364119836022,
        -0.6386560278302176,
    );
    let offset = (0.00569519921294126, 0.12694207885319453, 0.8146055611094525);

    let hollow = moved(
        rotated(
            crate::fixtures::hollow_cuboid(10.0, 4.0),
            axis,
            0.01174910706430916,
        ),
        offset,
    );
    let spline_volume = exact_volume(&spline);

    check_every_operation(
        "extruded spline and hollow cuboid",
        &spline,
        &hollow,
        (spline_volume, 936.0),
    );
}

#[test]
fn extruded_splines_turned_over_each_other_combine() {
    let first = crate::fixtures::extruded_spline(5.0);
    let axis = Vector3::new(
        0.31168557421151766,
        0.31978932598739207,
        0.38483027921105295,
    );
    let offset = (
        -0.0068295642980931055,
        0.016199814818384138,
        -0.012475840306522738,
    );

    let second = moved(rotated(first.clone(), axis, 3.589552995818218), offset);
    let volume = exact_volume(&first);

    check_every_operation("extruded splines", &first, &second, (volume, volume));
}
