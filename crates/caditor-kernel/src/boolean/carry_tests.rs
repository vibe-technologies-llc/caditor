use std::time::{Duration, Instant};

use caditor_geometry::{Plane, RigidTransform, Vector3};

use super::*;
use crate::{
    build::{LinearExtent, extrude},
    naming::{EdgeName, FaceName, FaceOrigin},
    profile::{Profile, ProfileCurve, Selection},
    tally,
    test_support::{circle, rectangle},
    topology::PcurveSample,
};

const PITCH: f64 = 10.0;

fn prism(curves: &[ProfileCurve], bottom: f64, height: f64, feature: u64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let solid = extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        feature,
    )
    .unwrap();
    solid
        .transformed(&RigidTransform::translation(Vector3::new(0.0, 0.0, bottom)).unwrap())
        .unwrap()
}

fn block(min: (f64, f64, f64), max: (f64, f64, f64), feature: u64) -> Solid {
    prism(
        &rectangle(1, (min.0, min.1), (max.0, max.1)),
        min.2,
        max.2 - min.2,
        feature,
    )
}

fn plate(columns: usize, rows: usize) -> Solid {
    block(
        (0.0, 0.0, 0.0),
        (PITCH * columns as f64, PITCH * rows as f64, 4.0),
        1,
    )
}

fn drill_at(center: (f64, f64), radius: f64, feature: u64) -> Solid {
    prism(&[circle(1, center, radius)], -1.0, 6.0, feature)
}

fn drill(index: usize, columns: usize) -> Solid {
    let (column, row) = (index % columns, index / columns);

    let center = (PITCH * (column as f64 + 0.5), PITCH * (row as f64 + 0.5));
    drill_at(center, PITCH / 4.0, 100 + index as u64)
}

fn block_at(index: usize) -> Solid {
    let x = 2.0 * PITCH * index as f64;
    block(
        (x, 0.0, 0.0),
        (x + PITCH, PITCH, PITCH),
        1000 + index as u64,
    )
}

fn full(first: &Solid, second: &Solid, operation: BooleanOperation) -> Result<Solid, BooleanError> {
    run(&Input::with_carrying(first, second, false), operation).map(|planned| planned.solid)
}

#[derive(Debug, PartialEq)]
struct Signature {
    faces: Vec<(FaceName, Option<FaceOrigin>, usize)>,
    edges: Vec<(EdgeName, Vec<FaceName>)>,
    counts: [usize; 4],
}

fn signature(solid: &Solid) -> Signature {
    let mut faces: Vec<_> = solid
        .faces()
        .map(|(_, face)| (face.name(), face.origin(), face.loops().len()))
        .collect();

    faces.sort();

    let mut edges: Vec<_> = solid
        .edges()
        .map(|(_, edge)| {
            let mut around: Vec<FaceName> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| solid.coedge_face(*coedge))
                .filter_map(|face| solid.face(face))
                .map(|face| face.name())
                .collect();

            around.sort();
            (edge.name(), around)
        })
        .collect();

    edges.sort();
    Signature {
        faces,
        edges,
        counts: [
            solid.vertices().count(),
            solid.edges().count(),
            solid.faces().count(),
            solid.shells().count(),
        ],
    }
}

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&solid.default_tolerance())
        .unwrap()
        .mass_properties()
        .volume
}

fn assert_same(name: &str, first: &Solid, second: &Solid, operation: BooleanOperation) -> Solid {
    let carried = boolean(first, second, operation);
    let rebuilt = full(first, second, operation);
    let (carried, rebuilt) = match (carried, rebuilt) {
        (Ok(carried), Ok(rebuilt)) => (carried, rebuilt),
        (carried, rebuilt) => {
            assert_eq!(
                carried.as_ref().err(),
                rebuilt.as_ref().err(),
                "{name} {operation:?}"
            );

            return Solid::default();
        }
    };

    assert_eq!(carried.validate(), Ok(()), "{name} {operation:?}");
    assert_eq!(rebuilt.validate(), Ok(()), "{name} {operation:?}");
    assert_eq!(
        signature(&carried),
        signature(&rebuilt),
        "{name} {operation:?}"
    );

    let (carried_volume, rebuilt_volume) = (volume(&carried), volume(&rebuilt));

    assert!(
        (carried_volume - rebuilt_volume).abs() <= 1e-6 * rebuilt_volume.abs().max(1.0),
        "{name} {operation:?}: {carried_volume} against {rebuilt_volume}"
    );
    carried
}

fn assert_same_in_every_operation(name: &str, first: &Solid, second: &Solid) {
    for operation in [
        BooleanOperation::Union,
        BooleanOperation::Difference,
        BooleanOperation::Intersection,
    ] {
        assert_same(name, first, second, operation);
    }
}

#[test]
fn holes_drilled_one_by_one_match_the_full_pipeline() {
    let columns = 4;
    let mut body = plate(columns, columns);

    for index in 0..columns * columns {
        let tool = drill(index, columns);

        body = assert_same("hole", &body, &tool, BooleanOperation::Difference);
    }

    assert_eq!(body.faces().count(), 6 + columns * columns);
}

#[test]
fn holes_breaking_into_earlier_holes_match_the_full_pipeline() {
    let mut body = plate(3, 1);

    for (index, center) in [(5.0, 5.0), (8.0, 5.0), (25.0, 5.0), (11.0, 5.0)]
        .into_iter()
        .enumerate()
    {
        let tool = drill_at(center, 2.0, 200 + index as u64);

        body = assert_same(
            "overlapping hole",
            &body,
            &tool,
            BooleanOperation::Difference,
        );
    }

    let boss = drill_at((25.0, 5.0), 3.5, 300);

    assert_same_in_every_operation("boss around a hole", &body, &boss);
}

#[test]
fn blocks_touching_or_almost_touching_match_the_full_pipeline() {
    let first = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0), 1);

    for gap in [0.0, 0.5 * TOLERANCE, 5.0 * TOLERANCE, 20.0 * TOLERANCE, 1.0] {
        let beside = block((10.0 + gap, 2.0, 2.0), (14.0, 8.0, 8.0), 2);
        let on_edge = block((10.0 + gap, 10.0 + gap, 0.0), (14.0, 14.0, 10.0), 3);
        let flush = block((10.0 + gap, 0.0, 0.0), (14.0, 10.0, 10.0), 4);

        assert_same_in_every_operation("beside", &first, &beside);
        assert_same_in_every_operation("on an edge", &first, &on_edge);
        assert_same_in_every_operation("flush", &first, &flush);
    }
}

#[test]
fn tools_sharing_a_face_match_the_full_pipeline() {
    let body = plate(4, 2);
    let pocket = block((5.0, 5.0, 2.0), (15.0, 15.0, 4.0), 2);
    let boss = block((5.0, 5.0, 4.0), (15.0, 15.0, 7.0), 3);
    let flush_through = block((30.0, -1.0, 0.0), (35.0, 21.0, 4.0), 4);

    assert_same_in_every_operation("pocket", &body, &pocket);
    assert_same_in_every_operation("boss", &body, &boss);
    assert_same_in_every_operation("flush through", &body, &flush_through);

    let drilled = assert_same("hole", &body, &drill(0, 4), BooleanOperation::Difference);
    let plug = drill(0, 4);

    assert_same_in_every_operation("plug", &drilled, &plug);
}

#[test]
fn separated_solids_skip_the_pipeline_with_the_same_result() {
    let first = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0), 1);
    let second = block((30.0, 0.0, 0.0), (40.0, 10.0, 10.0), 2);
    let ball = crate::fixtures::sphere(3.0);
    let far_ball = ball
        .transformed(&RigidTransform::translation(Vector3::new(50.0, 5.0, 5.0)).unwrap())
        .unwrap();

    assert!(apart::combine_apart(&first, &second, BooleanOperation::Union).is_some());
    assert!(apart::combine_apart(&first, &far_ball, BooleanOperation::Difference).is_some());
    assert!(apart::combine_apart(&first, &first, BooleanOperation::Union).is_none());

    assert_same_in_every_operation("separated", &first, &second);
    assert_same_in_every_operation("separated reversed", &second, &first);
    assert_same_in_every_operation("separated ball", &first, &far_ball);

    let joined = assert_same("joined", &first, &second, BooleanOperation::Union);
    let third = block((60.0, 0.0, 0.0), (70.0, 10.0, 10.0), 3);
    let bridge = block((5.0, 2.0, 2.0), (35.0, 8.0, 8.0), 4);

    assert_same_in_every_operation("third", &joined, &third);
    assert_same_in_every_operation("bridge", &joined, &bridge);
}

#[test]
fn separated_solids_that_the_pipeline_would_rename_go_through_it() {
    let first = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0), 1);
    let copy = first
        .transformed(&RigidTransform::translation(Vector3::new(30.0, 0.0, 0.0)).unwrap())
        .unwrap();
    let steps = prism(
        &[
            crate::test_support::line(1, (40.0, 0.0), (45.0, 0.0)),
            crate::test_support::line(2, (45.0, 0.0), (50.0, 0.0)),
            crate::test_support::line(3, (50.0, 0.0), (50.0, 10.0)),
            crate::test_support::line(4, (50.0, 10.0), (40.0, 10.0)),
            crate::test_support::line(5, (40.0, 10.0), (40.0, 0.0)),
        ],
        0.0,
        5.0,
        7,
    );

    assert!(apart::combine_apart(&first, &copy, BooleanOperation::Union).is_none());
    assert!(apart::combine_apart(&first, &steps, BooleanOperation::Union).is_none());

    assert_same_in_every_operation("copy", &first, &copy);
    assert_same_in_every_operation("split side", &first, &steps);
}

fn row_of_blocks(count: usize) -> Solid {
    let mut joined = block_at(0);

    for index in 1..count {
        joined = boolean(&joined, &block_at(index), BooleanOperation::Union).unwrap();
    }
    joined
}

#[test]
fn lumps_the_tool_does_not_reach_match_the_full_pipeline() {
    let row = row_of_blocks(4);
    let last = 2.0 * PITCH * 3.0;
    let overlapping = block(
        (last + 0.5 * PITCH, 2.0, 2.0),
        (last + 1.5 * PITCH, 8.0, 8.0),
        5000,
    );
    let pocket = block((last + 2.0, 2.0, 2.0), (last + 8.0, 8.0, 8.0), 5001);
    let bridge = block(
        (2.0 * PITCH + 5.0, 2.0, 2.0),
        (4.0 * PITCH + 5.0, 8.0, 8.0),
        5002,
    );
    let swallowing = block(
        (2.0 * PITCH - 1.0, -1.0, -1.0),
        (3.0 * PITCH + 1.0, PITCH + 1.0, PITCH + 1.0),
        5003,
    );
    let flush = block((last, 0.0, PITCH), (last + PITCH, PITCH, 2.0 * PITCH), 5004);

    assert_eq!(row.shells().count(), 4);

    for (name, tool) in [
        ("overlapping one lump", &overlapping),
        ("inside one lump", &pocket),
        ("bridging two lumps", &bridge),
        ("swallowing one lump", &swallowing),
        ("flush on one lump", &flush),
    ] {
        assert_same_in_every_operation(name, &row, tool);
    }

    let hollowed = assert_same("void", &row, &pocket, BooleanOperation::Difference);

    assert_eq!(hollowed.shells().count(), 5);

    let into_void = block((last + 5.0, 4.0, 4.0), (last + 12.0, 6.0, 6.0), 5005);
    let first_block = block((-1.0, 4.0, 4.0), (5.0, 6.0, 6.0), 5006);

    assert_same_in_every_operation("into a void", &hollowed, &into_void);
    assert_same_in_every_operation("beside a hollowed lump", &hollowed, &first_block);
    assert_same_in_every_operation("a hollowed lump", &hollowed, &pocket);
}

#[test]
fn lumps_with_the_same_names_match_the_full_pipeline() {
    let first = block((0.0, 0.0, 0.0), (10.0, 10.0, 10.0), 1);
    let copy = first
        .transformed(&RigidTransform::translation(Vector3::new(30.0, 0.0, 0.0)).unwrap())
        .unwrap();
    let third = block((60.0, 0.0, 0.0), (70.0, 10.0, 10.0), 3);

    let copies = assert_same("copies", &first, &copy, BooleanOperation::Union);
    let row = assert_same("third", &copies, &third, BooleanOperation::Union);
    let drill = drill_at((35.0, 5.0), 2.0, 9);
    let across = block((5.0, 2.0, 2.0), (35.0, 8.0, 8.0), 10);

    assert_eq!(row.shells().count(), 3);

    assert_same_in_every_operation("one of the copies", &row, &drill);
    assert_same_in_every_operation("the third", &row, &drill_at((65.0, 5.0), 2.0, 11));
    assert_same_in_every_operation("across the copies", &row, &across);
}

fn pcurve_buffers(solid: &Solid) -> Vec<*const PcurveSample> {
    solid
        .coedges()
        .map(|(_, coedge)| coedge.pcurve().samples().as_ptr())
        .collect()
}

#[test]
fn carried_coedges_share_their_pcurves_with_the_operand() {
    let body = plate(2, 2);

    let once = boolean(&body, &drill(0, 2), BooleanOperation::Difference).unwrap();
    let twice = boolean(&once, &drill(3, 2), BooleanOperation::Difference).unwrap();

    let (plate, drilled, redrilled) = (
        pcurve_buffers(&body),
        pcurve_buffers(&once),
        pcurve_buffers(&twice),
    );

    assert_eq!(twice.validate(), Ok(()));
    assert!(plate.iter().all(|buffer| drilled.contains(buffer)));
    assert!(drilled.iter().all(|buffer| redrilled.contains(buffer)));
    assert_eq!(redrilled.len(), drilled.len() + 6);
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

#[test]
#[ignore = "a timing survey, best run in release"]
fn holes_drilled_one_by_one_and_blocks_joined_one_by_one_take_bounded_time() {
    let columns = 12;
    let mut body = plate(columns, columns);
    let clock = Instant::now();

    for index in 0..columns * columns {
        let tool = drill(index, columns);
        let started = Instant::now();

        body = boolean(&body, &tool, BooleanOperation::Difference).unwrap();
        if matches!(index + 1, 1 | 12 | 36 | 72 | 144) {
            eprintln!(
                "hole {}: {:.2} ms",
                index + 1,
                milliseconds(started.elapsed())
            );
        }
    }
    eprintln!(
        "{} holes: {:.0} ms",
        columns * columns,
        milliseconds(clock.elapsed())
    );

    assert_eq!(body.validate(), Ok(()));

    let count = 300;
    let mut joined = block_at(0);
    let clock = Instant::now();

    for index in 1..count {
        let started = Instant::now();

        joined = boolean(&joined, &block_at(index), BooleanOperation::Union).unwrap();
        if matches!(index + 1, 2 | 100 | 300) {
            eprintln!(
                "union {}: {:.2} ms",
                index + 1,
                milliseconds(started.elapsed())
            );
        }
    }
    eprintln!("{count} unions: {:.0} ms", milliseconds(clock.elapsed()));

    assert_eq!(joined.shells().count(), count);

    let last = 2.0 * PITCH * (count - 1) as f64;
    let overlapping = block(
        (last + 0.5 * PITCH, 2.0, 2.0),
        (last + 1.5 * PITCH, 8.0, 8.0),
        5000,
    );
    let started = Instant::now();
    let grown = boolean(&joined, &overlapping, BooleanOperation::Union).unwrap();

    eprintln!(
        "a union touching one of {count} blocks: {:.2} ms",
        milliseconds(started.elapsed())
    );

    assert_eq!(grown.shells().count(), count);
}

fn union_with_the_last_of(count: usize) -> usize {
    let row = row_of_blocks(count);
    let last = 2.0 * PITCH * (count - 1) as f64;
    let overlapping = block(
        (last + 0.5 * PITCH, 2.0, 2.0),
        (last + 1.5 * PITCH, 8.0, 8.0),
        5000,
    );

    let (grown, units) = tally::measure(|| boolean(&row, &overlapping, BooleanOperation::Union));

    assert_eq!(grown.map(|grown| grown.shells().count()), Ok(count));
    units
}

#[test]
fn work_follows_the_contact_rather_than_the_body() {
    let columns = 8;
    let mut body = plate(columns, columns);
    let mut work = Vec::new();
    let mut meshed = Vec::new();

    for index in 0..columns * columns {
        let tool = drill(index, columns);

        let ((drilled, units), faces) = tally::measure_meshed_faces(|| {
            tally::measure(|| boolean(&body, &tool, BooleanOperation::Difference))
        });

        body = drilled.unwrap();
        work.push(units);
        meshed.push(faces);
    }

    let (first, last) = (work.first().copied(), work.last().copied());
    let (_, rebuilt_meshed) = tally::measure_meshed_faces(|| {
        full(&body, &drill(0, columns), BooleanOperation::Difference)
    });

    assert_eq!(body.faces().count(), 6 + columns * columns);
    assert!(
        meshed.iter().all(|faces| *faces == 3),
        "each hole meshes its wall and the top and bottom it opens, not the body: {meshed:?}"
    );
    assert!(
        rebuilt_meshed > 6 + columns * columns,
        "the full pipeline meshes the whole body, {rebuilt_meshed} faces"
    );
    assert!(
        first
            .zip(last)
            .is_some_and(|(first, last)| 10 * last <= 11 * first),
        "the first hole took {first:?} units of work, the last {last:?}: meshing the body, \
         checking every loop and untangling every hole again adds a fifth over 64 holes"
    );

    let (few, many) = (union_with_the_last_of(4), union_with_the_last_of(32));

    assert!(
        many <= 2 * few,
        "a union with one of 32 blocks took {many} units of work, with one of 4 {few}: taking \
         every block through the pipeline grows two and a half times"
    );
}
