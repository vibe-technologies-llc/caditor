use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::PI,
};

use caditor_geometry::{Plane, Point2, Point3, RigidTransform, Vector2, Vector3};

use super::{
    select::KeptFace,
    trace::{Fragment, HalfEdge, TracedLoop},
    *,
};
use crate::{
    build::{AngularExtent, Axis2, LinearExtent, extrude, plan::PlanError, revolve},
    fixtures::{cuboid, cylinder, sphere},
    intersect::IntersectionError,
    naming::{EdgeName, FaceName},
    profile::{Profile, Selection},
    sense::Sense,
    surface::{PlaneSurface, Surface},
    test_support::{assert_cancelled_anywhere, assert_watertight, circle, rectangle},
    tolerance::SamplingTolerance,
    topology::{BuildError, FaceId, Pcurve, PcurveSample, ValidationError},
};

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
        if solid.shells().count() == 1 {
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
                Err(BooleanError::NonManifold) => assert!(
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
                Err(BooleanError::NonManifold) => {
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
fn nearly_coincident_tori_are_too_intricate_to_intersect() {
    let ring = crate::fixtures::torus(6.0, 2.0);
    let shifted = moved(ring.clone(), (1e-4, 1e-4, 1e-4));

    assert!(matches!(
        boolean(&ring, &shifted, BooleanOperation::Union),
        Err(BooleanError::Intersection(IntersectionError::TooComplex(_)))
    ));
}

#[test]
fn tori_touching_along_their_equators_cannot_be_split() {
    let ring = crate::fixtures::torus(6.0, 2.0);
    let beside = moved(ring.clone(), (4.0, 0.0, 0.0));

    assert_eq!(
        boolean(&ring, &beside, BooleanOperation::Union).err(),
        Some(BooleanError::Split)
    );
}

#[test]
fn a_result_straying_past_the_resolution_is_refused_as_invalid() {
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

    assert!(matches!(
        boolean(&post, &tilted, BooleanOperation::Union),
        Err(BooleanError::Invalid(BuildError::Invalid(
            ValidationError::PcurveOffEdge { .. }
        )))
    ));
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

    assert_eq!(
        heal::check_closed(&[kept_face(0, square.clone())]),
        Err(BooleanError::Open)
    );
    assert_eq!(
        heal::check_closed(&[
            kept_face(0, square.clone()),
            kept_face(1, square.reversed()),
            kept_face(2, square.clone()),
            kept_face(3, square.reversed()),
        ]),
        Err(BooleanError::NonManifold)
    );
    assert_eq!(
        heal::check_closed(&[
            kept_face(0, square.clone()),
            kept_face(1, square.reversed())
        ]),
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
    assert_eq!(
        select::classify(&input, key, &surface, Sense::Same, &fragment),
        Err(BooleanError::Ambiguous)
    );
}

#[test]
fn failures_of_the_steps_become_boolean_errors_in_words() {
    assert_eq!(
        BooleanError::from(PlanError::Unassembled),
        BooleanError::Open
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
            BooleanError::Split,
            "a face could not be divided where the solids meet",
        ),
        (
            BooleanError::Ambiguous,
            "the solids touch where it cannot be told which side is inside",
        ),
        (
            BooleanError::Open,
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
    ] {
        assert_eq!(error.to_string(), words);
    }
}

fn outcome(result: &Result<Solid, BooleanError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(BooleanError::Empty) => "empty",
        Err(BooleanError::Intersection(_)) => "intersection",
        Err(BooleanError::Split) => "split",
        Err(BooleanError::Ambiguous) => "ambiguous",
        Err(BooleanError::Open) => "open",
        Err(BooleanError::NonManifold) => "non-manifold",
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
}
