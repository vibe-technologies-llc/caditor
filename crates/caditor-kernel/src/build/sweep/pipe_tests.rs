use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI},
};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    curve::{Circle, Curve, Line},
    interval::Interval,
    measure::{extent, mass_properties_tessellating},
    naming::FaceName,
    surface::Surface,
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

fn arc(key: u128, centre: Point3, x_axis: Vector3, radius: f64, sweep: f64) -> PathSegment {
    let frame = Plane::from_frame(centre, -Vector3::Z, x_axis).unwrap();
    PathSegment::new(
        Curve::Circle(Circle::new(frame, radius).unwrap()),
        Interval::new(0.0, sweep).unwrap(),
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

fn tori(solid: &Solid) -> usize {
    solid
        .faces()
        .filter(|(_, face)| matches!(face.surface(), Surface::Torus(_)))
        .count()
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

#[test]
fn the_stand_is_the_start_section_and_the_first_corner_with_or_without_a_bend() {
    let path = elbow();

    for bend in [0.0, 5.0] {
        let stand = pipe_stand(&path, bend, Vector3::Z).unwrap();
        let corner = stand.corner.expect("the elbow has a corner");

        assert_eq!(stand.section.origin(), Point3::ZERO);
        assert!(stand.section.normal().distance(Vector3::X) < 1e-9);
        assert_eq!(corner.point, Point3::new(20.0, 0.0, 0.0));
        assert!(corner.incoming.distance(Vector3::X) < 1e-9);
        assert!(corner.outgoing.distance(Vector3::Y) < 1e-9);
    }
}

#[test]
fn a_corner_between_two_lines_has_its_bend_middles_along_the_bisector() {
    let stand = pipe_stand(&elbow(), 5.0, Vector3::Z).unwrap();

    assert_eq!(
        stand.corner.map(|corner| corner.middles),
        Some(BendMiddles::AlongBisector)
    );
}

#[test]
fn a_sharp_corner_touching_an_arc_has_bend_middles_sampled_from_the_corner_outward() {
    let (bend, arc_radius): (f64, f64) = (4.0, 10.0);
    let reach = ((arc_radius + bend).powi(2) - bend * bend).sqrt();
    let bend_centre = Point3::new(30.0 - reach, bend, 0.0);

    let stand = pipe_stand(&line_into_arc(), bend, Vector3::Z).unwrap();
    let corner = stand.corner.expect("the line meets the arc at a corner");
    let BendMiddles::Sampled(middles) = &corner.middles else {
        panic!("a corner touching an arc is sampled: {:?}", corner.middles);
    };
    let at_bend = middles
        .iter()
        .find(|middle| middle.radius == bend)
        .expect("the bend radius itself is sampled");

    assert_eq!(corner.point, Point3::new(20.0, 0.0, 0.0));
    assert_eq!(
        middles.first(),
        Some(&BendMiddle {
            radius: 0.0,
            point: corner.point
        })
    );
    assert!(middles.len() > 8, "{} middles", middles.len());
    assert!(
        middles
            .windows(2)
            .all(|pair| matches!(pair, [before, after] if after.radius > before.radius))
    );
    assert!((at_bend.point.distance(bend_centre) - bend).abs() < 1e-6);
    assert!(at_bend.point.y > 0.0 && at_bend.point.y < bend);
    assert!(at_bend.point.distance(Point3::new(30.0, 0.0, 0.0)) > arc_radius);
}

#[test]
fn a_straight_path_has_no_corner_in_its_stand() {
    let path = [line(10, Point3::ZERO, Point3::new(0.0, 0.0, 40.0))];

    let stand = pipe_stand(&path, 5.0, Vector3::Z).unwrap();

    assert_eq!(stand.corner, None);
}

fn line_into_arc() -> [PathSegment; 2] {
    [
        line(10, Point3::ZERO, Point3::new(20.0, 0.0, 0.0)),
        arc(
            11,
            Point3::new(30.0, 0.0, 0.0),
            -Vector3::X,
            10.0,
            FRAC_PI_2,
        ),
    ]
}

#[test]
fn a_bend_radius_rounds_a_sharp_corner_between_a_line_and_an_arc() {
    let (bend, arc_radius): (f64, f64) = (4.0, 10.0);
    let reach = ((arc_radius + bend).powi(2) - bend * bend).sqrt();
    let touch = bend.atan2(reach);
    let centreline = (30.0 - reach) + bend * (FRAC_PI_2 - touch) + arc_radius * (FRAC_PI_2 - touch);

    let rounded = pipe(
        &line_into_arc(),
        &round(2.0, None),
        bend,
        Vector3::Z,
        FEATURE,
    )
    .unwrap();
    let sharp = pipe(
        &line_into_arc(),
        &round(2.0, None),
        0.0,
        Vector3::Z,
        FEATURE,
    );

    assert_eq!(rounded.validate(), Ok(()));
    assert_close("rounded", volume(&rounded), PI * centreline, 1e-3);
    assert_eq!(tori(&rounded), 2);
    assert_eq!(names(&rounded).len(), rounded.faces().count());
    assert_eq!(
        sharp.unwrap_err(),
        PipeError::Path(PathSweepError::Corner { segments: [0, 1] })
    );
}

#[test]
fn a_bend_radius_rounds_the_point_where_two_arcs_meet() {
    let (bend, arc_radius): (f64, f64) = (3.0, 10.0);
    let sweep = 0.6f64.acos();
    let path = [
        arc(
            10,
            Point3::new(6.0, -8.0, 0.0),
            -Vector3::X,
            arc_radius,
            sweep,
        ),
        arc(
            11,
            Point3::new(-6.0, -8.0, 0.0),
            Vector3::new(0.6, 0.8, 0.0),
            arc_radius,
            sweep,
        ),
    ];
    let rise = ((arc_radius - bend).powi(2) - 36.0).sqrt();
    let centreline = 2.0 * arc_radius * rise.atan2(6.0) + 2.0 * bend * 6f64.atan2(rise);

    let rounded = pipe(&path, &round(2.0, Some(0.5)), bend, Vector3::Z, FEATURE).unwrap();

    assert_eq!(rounded.validate(), Ok(()));
    assert_close(
        "rounded",
        volume(&rounded),
        PI * (1.0 - 0.25) * centreline,
        1e-3,
    );
    assert_eq!(tori(&rounded), 6);
}

#[test]
fn a_corner_out_of_one_plane_is_rounded_by_a_smooth_blend() {
    let path = [
        line(10, Point3::new(-20.0, 0.0, 0.0), Point3::ZERO),
        PathSegment::new(
            Curve::Circle(
                Circle::new(
                    Plane::from_frame(Point3::new(0.0, 0.0, 10.0), Vector3::X, -Vector3::Z)
                        .unwrap(),
                    10.0,
                )
                .unwrap(),
            ),
            Interval::new(0.0, FRAC_PI_2).unwrap(),
            11,
        ),
    ];
    let (bend, area) = (4.0, PI);
    let cut = bend * (FRAC_PI_2 / 2.0).tan();
    let whole = 20.0 + 10.0 * FRAC_PI_2;

    let rounded = pipe(&path, &round(2.0, None), bend, Vector3::Z, FEATURE).unwrap();
    let found = volume(&rounded);

    assert_eq!(rounded.validate(), Ok(()));
    assert_eq!(tori(&rounded), 1);
    assert!(found < area * whole, "{found}");
    assert!(found > area * (whole - 2.0 * cut), "{found}");
}

#[test]
fn a_bend_too_large_for_the_curves_at_a_corner_is_refused_naming_them() {
    let too_long = pipe(
        &line_into_arc(),
        &round(2.0, None),
        60.0,
        Vector3::Z,
        FEATURE,
    );

    assert_eq!(
        too_long.unwrap_err(),
        PipeError::BendTooLong { segments: [0, 1] }
    );
}
