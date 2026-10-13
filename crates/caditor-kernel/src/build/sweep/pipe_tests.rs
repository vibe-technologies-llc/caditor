use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

use caditor_geometry::{Point3, Vector3};

use super::*;
use crate::{
    curve::{Curve, Line},
    interval::Interval,
    measure::{extent, mass_properties_tessellating},
    naming::FaceName,
    tolerance::SamplingTolerance,
    topology::Solid,
};

const FEATURE: u64 = 9;

fn line(key: u128, from: Point3, to: Point3) -> PathSegment {
    PathSegment::new(
        Curve::Line(Line::through(from, to).unwrap()),
        Interval::new(0.0, from.distance(to)).unwrap(),
        key,
    )
}

fn round(size: f64, wall: Option<f64>) -> PipeSection {
    PipeSection {
        shape: PipeShape::Round,
        size,
        wall,
    }
}

fn volume(solid: &Solid) -> f64 {
    mass_properties_tessellating(solid, &SamplingTolerance::new(1e-3, 0.1).unwrap())
        .unwrap()
        .properties
        .volume
}

fn assert_close(name: &str, found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "{name}: {found} against {expected}"
    );
}

fn names(solid: &Solid) -> BTreeSet<FaceName> {
    solid.faces().map(|(_, face)| face.name()).collect()
}

fn elbow() -> [PathSegment; 2] {
    [
        line(10, Point3::ZERO, Point3::new(20.0, 0.0, 0.0)),
        line(
            11,
            Point3::new(20.0, 0.0, 0.0),
            Point3::new(20.0, 30.0, 0.0),
        ),
    ]
}

#[test]
fn a_straight_round_pipe_is_a_cylinder_and_a_hollow_one_a_tube() {
    let path = [line(10, Point3::ZERO, Point3::new(0.0, 0.0, 40.0))];

    let solid = pipe(&path, &round(10.0, None), 0.0, Vector3::Z, FEATURE).unwrap();
    let hollow = pipe(&path, &round(10.0, Some(1.0)), 0.0, Vector3::Z, FEATURE).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert_eq!(hollow.validate(), Ok(()));
    assert_close("solid", volume(&solid), PI * 25.0 * 40.0, 1e-4);
    assert_close("hollow", volume(&hollow), PI * (25.0 - 16.0) * 40.0, 1e-4);
}

#[test]
fn square_and_triangular_sections_lie_flat_against_the_up_direction() {
    let path = [line(10, Point3::ZERO, Point3::new(50.0, 0.0, 0.0))];
    let square = PipeSection {
        shape: PipeShape::Square,
        size: 8.0,
        wall: Some(2.0),
    };
    let triangle = PipeSection {
        shape: PipeShape::Triangle,
        size: 6.0,
        wall: None,
    };

    let boxed = pipe(&path, &square, 0.0, Vector3::Z, FEATURE).unwrap();
    let wedge = pipe(&path, &triangle, 0.0, Vector3::Z, FEATURE).unwrap();
    let bounds = extent(&boxed).unwrap().unwrap();
    let wedge_bounds = extent(&wedge).unwrap().unwrap();

    assert_close("square", volume(&boxed), (64.0 - 16.0) * 50.0, 1e-6);
    assert_close("square height", bounds.max().z - bounds.min().z, 8.0, 1e-9);
    assert_close("square width", bounds.max().y - bounds.min().y, 8.0, 1e-9);
    assert_close(
        "triangle",
        volume(&wedge),
        3f64.sqrt() / 4.0 * 36.0 * 50.0,
        1e-6,
    );
    assert_close(
        "triangle base",
        wedge_bounds.max().y - wedge_bounds.min().y,
        6.0,
        1e-9,
    );
    assert_close(
        "triangle height",
        wedge_bounds.max().z - wedge_bounds.min().z,
        3f64.sqrt() * 3.0,
        1e-9,
    );
}

#[test]
fn a_bend_radius_rounds_a_sharp_corner_between_lines() {
    let area = PI * 4.0;
    let bend = 6.0;
    let straight = 20.0 - bend + 30.0 - bend;

    let rounded = pipe(&elbow(), &round(4.0, None), bend, Vector3::Z, FEATURE).unwrap();
    let mitred = pipe(&elbow(), &round(4.0, None), 0.0, Vector3::Z, FEATURE).unwrap();

    assert_eq!(rounded.validate(), Ok(()));
    assert_close(
        "rounded",
        volume(&rounded),
        area * (straight + bend * FRAC_PI_2),
        1e-3,
    );
    assert_close("mitred", volume(&mitred), area * 50.0, 1e-3);
    assert_eq!(names(&rounded).len(), rounded.faces().count());
    assert!(names(&mitred).is_subset(&names(&rounded)));
}

#[test]
fn a_closed_path_of_lines_rounds_every_corner() {
    let corners = [
        Point3::ZERO,
        Point3::new(40.0, 0.0, 0.0),
        Point3::new(40.0, 30.0, 0.0),
        Point3::new(0.0, 30.0, 0.0),
    ];
    let path: Vec<PathSegment> = (0..4)
        .map(|index| line(10 + index as u128, corners[index], corners[(index + 1) % 4]))
        .collect();
    let bend = 5.0;
    let centreline = 140.0 - 8.0 * bend + 2.0 * PI * bend;

    let ring = pipe(&path, &round(4.0, Some(1.0)), bend, Vector3::Z, FEATURE).unwrap();

    assert_eq!(ring.validate(), Ok(()));
    assert_close("ring", volume(&ring), PI * (4.0 - 1.0) * centreline, 1e-3);
}

#[test]
fn a_bend_using_a_whole_line_leaves_no_line_between_the_arcs() {
    let path = [
        line(10, Point3::ZERO, Point3::new(20.0, 0.0, 0.0)),
        line(
            11,
            Point3::new(20.0, 0.0, 0.0),
            Point3::new(20.0, 10.0, 0.0),
        ),
        line(
            12,
            Point3::new(20.0, 10.0, 0.0),
            Point3::new(40.0, 10.0, 0.0),
        ),
    ];

    let solid = pipe(&path, &round(2.0, None), 5.0, Vector3::Z, FEATURE).unwrap();

    assert_close(
        "s-bend",
        volume(&solid),
        PI * (15.0 + 15.0 + 5.0 * PI),
        1e-3,
    );
}

#[test]
fn sections_and_bends_that_cannot_be_made_are_refused_naming_the_curves() {
    let thick = pipe(&elbow(), &round(4.0, Some(2.0)), 0.0, Vector3::Z, FEATURE);
    let long = pipe(&elbow(), &round(4.0, None), 25.0, Vector3::Z, FEATURE);
    let tight = pipe(&elbow(), &round(4.0, None), 1.5, Vector3::Z, FEATURE);
    let negative = pipe(&elbow(), &round(4.0, None), -1.0, Vector3::Z, FEATURE);

    assert_eq!(thick.unwrap_err(), PipeError::WallTooThick);
    assert_eq!(
        long.unwrap_err(),
        PipeError::BendTooLong { segments: [0, 1] }
    );
    assert_eq!(
        tight.unwrap_err(),
        PipeError::BendTooTight { segments: [0, 1] }
    );
    assert_eq!(negative.unwrap_err(), PipeError::InvalidBend);
}

#[test]
fn path_errors_name_the_segments_as_given() {
    let path = [
        line(10, Point3::ZERO, Point3::new(20.0, 0.0, 0.0)),
        line(
            11,
            Point3::new(25.0, 0.0, 0.0),
            Point3::new(25.0, 30.0, 0.0),
        ),
    ];

    let apart = pipe(&path, &round(4.0, None), 5.0, Vector3::Z, FEATURE);

    assert!(matches!(
        apart.unwrap_err(),
        PipeError::Path(PathSweepError::Apart { .. })
    ));
}
