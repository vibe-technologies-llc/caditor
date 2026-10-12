use std::{
    collections::BTreeSet,
    f64::consts::{FRAC_PI_2, PI, TAU},
};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    bspline::BSpline,
    curve::{Circle, Curve, Line},
    interval::Interval,
    measure::mass_properties_tessellating,
    naming::FaceName,
    profile::{Profile, ProfileCurve, Region, Selection},
    test_support::{circle, rectangle},
    tolerance::SamplingTolerance,
    topology::Solid,
};

const FEATURE: u64 = 9;

fn regions(curves: &[ProfileCurve]) -> Vec<Region> {
    Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap()
}

fn upright() -> Plane {
    Plane::with_x_axis(Point3::ZERO, Vector3::Y, Vector3::X).unwrap()
}

fn line(key: u128, from: Point3, to: Point3) -> PathSegment {
    PathSegment::new(
        Curve::Line(Line::through(from, to).unwrap()),
        Interval::new(0.0, from.distance(to)).unwrap(),
        key,
    )
}

fn arc(key: u128, centre: Point3, normal: Vector3, start: Point3, angle: f64) -> PathSegment {
    let radial = start - centre;
    let frame = Plane::from_frame(centre, normal, radial.normalize()).unwrap();
    PathSegment::new(
        Curve::Circle(Circle::new(frame, radial.length()).unwrap()),
        Interval::new(0.0, angle).unwrap(),
        key,
    )
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

fn assert_named_apart(name: &str, solid: &Solid) {
    let names: BTreeSet<FaceName> = solid.faces().map(|(_, face)| face.name()).collect();
    assert_eq!(
        names.len(),
        solid.faces().count(),
        "{name}: repeated face names"
    );
}

fn swept(
    plane: &Plane,
    curves: &[ProfileCurve],
    path: &[PathSegment],
    orientation: SweepOrientation,
) -> Result<Solid, PathSweepError> {
    sweep(plane, &regions(curves), path, orientation, FEATURE)
}

#[test]
fn a_straight_path_sweeps_the_profile_like_an_extrusion() {
    let profile = rectangle(1, (-1.0, -1.5), (1.0, 1.5));
    let path = [line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0))];

    let square = swept(&upright(), &profile, &path, SweepOrientation::Square).unwrap();
    let parallel = swept(&upright(), &profile, &path, SweepOrientation::Parallel).unwrap();

    assert_eq!(square.validate(), Ok(()));
    assert_close("square", volume(&square), 60.0, 1e-6);
    assert_close("parallel", volume(&parallel), 60.0, 1e-6);
    assert_eq!(square.faces().count(), 6);
    assert_named_apart("square", &square);
}

#[test]
fn a_slanted_straight_path_keeps_the_profile_at_its_angle() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let path = [line(10, Point3::ZERO, Point3::new(3.0, 4.0, 0.0))];

    let solid = swept(&upright(), &profile, &path, SweepOrientation::Square).unwrap();

    assert_close("slanted", volume(&solid), 4.0 * 4.0, 1e-6);
}

#[test]
fn a_path_starting_at_its_far_end_is_walked_from_the_end_nearest_the_profile() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let forward = [line(10, Point3::ZERO, Point3::new(0.0, -5.0, 0.0))];
    let backward = [line(10, Point3::new(0.0, -5.0, 0.0), Point3::ZERO)];

    let first = swept(&upright(), &profile, &forward, SweepOrientation::Square).unwrap();
    let second = swept(&upright(), &profile, &backward, SweepOrientation::Square).unwrap();

    assert_close("forward", volume(&first), 20.0, 1e-6);
    assert_close("backward", volume(&second), 20.0, 1e-6);
    let names = |solid: &Solid| -> BTreeSet<FaceName> {
        solid.faces().map(|(_, face)| face.name()).collect()
    };
    assert_eq!(names(&first), names(&second));
}

#[test]
fn an_arc_path_turns_the_profile_about_its_centre_with_pappus_volume() {
    let radius = 5.0;
    let disc = [circle(1, (radius, 0.0), 1.0)];
    let block = rectangle(1, (radius - 1.0, -0.5), (radius + 1.0, 0.5));
    let path = [arc(
        10,
        Point3::ZERO,
        Vector3::Z,
        Point3::new(radius, 0.0, 0.0),
        FRAC_PI_2,
    )];

    let tube = swept(&upright(), &disc, &path, SweepOrientation::Square).unwrap();
    let bar = swept(&upright(), &block, &path, SweepOrientation::Square).unwrap();

    assert_eq!(tube.validate(), Ok(()));
    assert_close("tube", volume(&tube), PI * radius * FRAC_PI_2, 1e-5);
    assert_close("bar", volume(&bar), 2.0 * radius * FRAC_PI_2, 1e-5);
    assert!(
        tube.faces()
            .any(|(_, face)| matches!(face.surface(), Surface::Torus(_)))
    );
    assert_named_apart("bar", &bar);
}

#[test]
fn a_whole_circle_path_closes_into_a_ring() {
    let radius = 5.0;
    let disc = [circle(1, (radius, 0.0), 1.0)];
    let block = rectangle(1, (radius - 1.0, -0.5), (radius + 1.0, 0.5));
    let path = [arc(
        10,
        Point3::ZERO,
        Vector3::Z,
        Point3::new(radius, 0.0, 0.0),
        TAU,
    )];

    let ring = swept(&upright(), &disc, &path, SweepOrientation::Square).unwrap();
    let washer = swept(&upright(), &block, &path, SweepOrientation::Square).unwrap();

    assert_close("ring", volume(&ring), PI * TAU * radius, 1e-5);
    assert_close("washer", volume(&washer), 2.0 * TAU * radius, 1e-5);
}

fn rounded_square(side: f64, corner: f64) -> Vec<PathSegment> {
    let half = 0.5 * side;
    let inner = half - corner;
    let point = |x: f64, y: f64| Point3::new(x, y, 0.0);
    vec![
        line(20, point(half, -inner), point(half, inner)),
        arc(
            30,
            point(inner, inner),
            Vector3::Z,
            point(half, inner),
            FRAC_PI_2,
        ),
        line(21, point(inner, half), point(-inner, half)),
        arc(
            31,
            point(-inner, inner),
            Vector3::Z,
            point(-inner, half),
            FRAC_PI_2,
        ),
        line(22, point(-half, inner), point(-half, -inner)),
        arc(
            32,
            point(-inner, -inner),
            Vector3::Z,
            point(-half, -inner),
            FRAC_PI_2,
        ),
        line(23, point(-inner, -half), point(inner, -half)),
        arc(
            33,
            point(inner, -inner),
            Vector3::Z,
            point(inner, -half),
            FRAC_PI_2,
        ),
    ]
}

#[test]
fn a_closed_path_of_lines_and_tangent_arcs_sweeps_a_closed_tube() {
    let side = 20.0;
    let corner = 4.0;
    let path = rounded_square(side, corner);
    let disc = [circle(1, (0.5 * side, 0.0), 1.0)];
    let length = 4.0 * (side - 2.0 * corner) + TAU * corner;

    let tube = swept(&upright(), &disc, &path, SweepOrientation::Square).unwrap();

    assert_eq!(tube.validate(), Ok(()));
    assert_close("closed tube", volume(&tube), PI * length, 1e-5);
    assert_named_apart("closed tube", &tube);
}

#[test]
fn a_spline_path_sweeps_a_fitted_tube_of_the_profile_times_its_length() {
    let spline = BSpline::new(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        vec![
            Point3::ZERO,
            Point3::new(0.0, 6.0, 0.0),
            Point3::new(6.0, 10.0, 2.0),
            Point3::new(12.0, 10.0, 4.0),
        ],
    )
    .unwrap();
    let length = Curve::BSpline(spline.clone()).length(Interval::UNIT);
    let path = [PathSegment::new(Curve::BSpline(spline), Interval::UNIT, 10)];
    let disc = [circle(1, (0.0, 0.0), 0.5)];

    let tube = swept(&upright(), &disc, &path, SweepOrientation::Square).unwrap();

    assert_eq!(tube.validate(), Ok(()));
    assert_close("spline tube", volume(&tube), PI * 0.25 * length, 1e-4);
}

#[test]
fn a_parallel_sweep_along_an_arc_keeps_the_profile_upright() {
    let disc = [circle(1, (0.0, 0.0), 1.0)];
    let path = [arc(
        10,
        Point3::new(0.0, 0.0, 10.0),
        Vector3::X,
        Point3::ZERO,
        1.0,
    )];
    let rise = Point3::new(0.0, 0.0, 10.0).distance(Point3::ZERO) * 1.0_f64.sin();

    let solid = swept(&upright(), &disc, &path, SweepOrientation::Parallel).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert_close("parallel arc", volume(&solid), PI * rise, 1e-5);
}

#[test]
fn a_sharp_corner_between_lines_is_mitred() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let corner = Point3::new(0.0, 10.0, 0.0);
    let path = [
        line(10, Point3::ZERO, corner),
        line(11, corner, Point3::new(8.0, 10.0, 0.0)),
    ];

    let solid = swept(&upright(), &profile, &path, SweepOrientation::Square).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert_close("mitred", volume(&solid), 4.0 * 18.0, 1e-6);
    assert_eq!(solid.faces().count(), 10);
}

#[test]
fn a_mitre_too_sharp_for_the_profile_is_refused() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let corner = Point3::new(0.0, 10.0, 0.0);
    let path = [
        line(10, Point3::ZERO, corner),
        line(11, corner, Point3::new(0.5, 9.0, 0.0)),
    ];

    let refused = swept(&upright(), &profile, &path, SweepOrientation::Square);

    assert!(
        matches!(refused, Err(PathSweepError::CornerTooSharp { .. })),
        "{refused:?}"
    );
}

#[test]
fn a_corner_onto_an_arc_is_refused_in_square_sweeps() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let corner = Point3::new(0.0, 10.0, 0.0);
    let path = [
        line(10, Point3::ZERO, corner),
        arc(11, Point3::new(0.0, 15.0, 0.0), Vector3::Z, corner, 1.0),
    ];

    let refused = swept(&upright(), &profile, &path, SweepOrientation::Square);

    assert_eq!(
        refused.err(),
        Some(PathSweepError::Corner { segments: [0, 1] })
    );
}

#[test]
fn a_profile_reaching_past_the_bend_centre_is_refused() {
    let disc = [circle(1, (2.0, 0.0), 3.0)];
    let path = [arc(
        10,
        Point3::ZERO,
        Vector3::Z,
        Point3::new(2.0, 0.0, 0.0),
        FRAC_PI_2,
    )];

    let refused = swept(&upright(), &disc, &path, SweepOrientation::Square);

    assert_eq!(
        refused.err(),
        Some(PathSweepError::ProfileTooLarge { segment: 0 })
    );
}

#[test]
fn a_path_along_the_profile_plane_is_refused() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let path = [line(10, Point3::ZERO, Point3::new(5.0, 0.0, 0.0))];

    let refused = swept(&upright(), &profile, &path, SweepOrientation::Square);

    assert_eq!(refused.err(), Some(PathSweepError::StartsAlongProfile));
}

#[test]
fn a_parallel_sweep_along_a_closed_path_is_refused() {
    let disc = [circle(1, (5.0, 0.0), 1.0)];
    let path = [arc(
        10,
        Point3::ZERO,
        Vector3::Z,
        Point3::new(5.0, 0.0, 0.0),
        TAU,
    )];

    let refused = swept(&upright(), &disc, &path, SweepOrientation::Parallel);

    assert!(
        matches!(refused, Err(PathSweepError::TurnsAlongProfile { .. })),
        "{refused:?}"
    );
}

#[test]
fn branching_and_separate_paths_are_refused() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let fork = Point3::new(0.0, 5.0, 0.0);
    let branching = [
        line(10, Point3::ZERO, fork),
        line(11, fork, Point3::new(0.0, 10.0, 0.0)),
        line(12, fork, Point3::new(3.0, 8.0, 0.0)),
    ];
    let apart = [
        line(10, Point3::ZERO, fork),
        line(11, Point3::new(0.0, 7.0, 0.0), Point3::new(0.0, 10.0, 0.0)),
    ];

    let branched = swept(&upright(), &profile, &branching, SweepOrientation::Square);
    let separate = swept(&upright(), &profile, &apart, SweepOrientation::Square);

    assert_eq!(
        branched.err(),
        Some(PathSweepError::Branches {
            segments: vec![0, 1, 2]
        })
    );
    assert_eq!(
        separate.err(),
        Some(PathSweepError::Apart { segments: vec![1] })
    );
}

#[test]
fn a_tube_whose_path_comes_back_through_itself_is_refused() {
    let disc = [circle(1, (0.0, 0.0), 1.0)];
    let path = [
        line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0)),
        arc(
            11,
            Point3::new(3.0, 10.0, 0.0),
            -Vector3::Z,
            Point3::new(0.0, 10.0, 0.0),
            1.5 * PI,
        ),
        line(12, Point3::new(3.0, 7.0, 0.0), Point3::new(-5.0, 7.0, 0.0)),
    ];

    let refused = swept(&upright(), &disc, &path, SweepOrientation::Square);

    assert_eq!(refused.err(), Some(PathSweepError::CrossesItself));
}

fn bent_spline() -> BSpline<Point3> {
    BSpline::new(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        vec![
            Point3::ZERO,
            Point3::new(0.0, 6.0, 0.0),
            Point3::new(6.0, 10.0, 2.0),
            Point3::new(12.0, 10.0, 4.0),
        ],
    )
    .unwrap()
}

#[test]
fn a_parallel_sweep_along_a_spline_is_an_exact_translation_of_the_profile() {
    let rising = BSpline::new(
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        vec![
            Point3::ZERO,
            Point3::new(0.0, 4.0, 0.0),
            Point3::new(4.0, 6.0, 3.0),
            Point3::new(8.0, 9.0, 3.0),
        ],
    )
    .unwrap();
    let path = [PathSegment::new(Curve::BSpline(rising), Interval::UNIT, 10)];
    let disc = [circle(1, (0.0, 0.0), 0.5)];

    let solid = swept(&upright(), &disc, &path, SweepOrientation::Parallel).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert_close("parallel spline", volume(&solid), PI * 0.25 * 9.0, 1e-6);
}

#[test]
fn a_profile_slanted_to_an_arc_path_is_turned_on_fitted_faces() {
    let tilted = Plane::with_x_axis(Point3::ZERO, Vector3::new(0.0, 1.0, 0.3), Vector3::X).unwrap();
    let disc = [circle(1, (5.0, 0.0), 1.0)];
    let path = [arc(
        10,
        Point3::ZERO,
        Vector3::Z,
        Point3::new(5.0, 0.0, 0.0),
        FRAC_PI_2,
    )];

    let solid = swept(&tilted, &disc, &path, SweepOrientation::Square).unwrap();

    assert_eq!(solid.validate(), Ok(()));
    assert!(
        solid
            .faces()
            .any(|(_, face)| matches!(face.surface(), Surface::BSpline(_)))
    );
    let lean = 0.3_f64.atan().cos();
    assert_close(
        "slanted ring",
        volume(&solid),
        PI * 5.0 * FRAC_PI_2 * lean,
        1e-4,
    );
}

#[test]
fn a_holed_profile_sweeps_a_hollow_bend() {
    let ring = [circle(1, (5.0, 0.0), 1.0), circle(2, (5.0, 0.0), 0.5)];
    let path = [arc(
        10,
        Point3::ZERO,
        Vector3::Z,
        Point3::new(5.0, 0.0, 0.0),
        FRAC_PI_2,
    )];

    let pipe = swept(&upright(), &ring, &path, SweepOrientation::Square).unwrap();

    assert_eq!(pipe.validate(), Ok(()));
    assert_close("pipe", volume(&pipe), PI * 0.75 * 5.0 * FRAC_PI_2, 1e-5);
}

#[test]
fn a_closed_spline_path_closes_its_fitted_tube() {
    let reach = 4.0 / 3.0 * 5.0;
    let half = |sign: f64| {
        BSpline::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![
                Point3::new(5.0 * sign, 0.0, 0.0),
                Point3::new(5.0 * sign, reach * sign, 0.0),
                Point3::new(-5.0 * sign, reach * sign, 0.0),
                Point3::new(-5.0 * sign, 0.0, 0.0),
            ],
        )
        .unwrap()
    };
    let length = Curve::BSpline(half(1.0)).length(Interval::UNIT) * 2.0;
    let path = [
        PathSegment::new(Curve::BSpline(half(1.0)), Interval::UNIT, 10),
        PathSegment::new(Curve::BSpline(half(-1.0)), Interval::UNIT, 11),
    ];
    let disc = [circle(1, (5.0, 0.0), 0.5)];

    let ring = swept(&upright(), &disc, &path, SweepOrientation::Square).unwrap();

    assert_eq!(ring.validate(), Ok(()));
    assert_close("spline ring", volume(&ring), PI * 0.25 * length, 1e-4);
    assert_named_apart("spline ring", &ring);
}

#[test]
fn a_spline_sweep_can_be_cancelled_anywhere() {
    let path = [PathSegment::new(
        Curve::BSpline(bent_spline()),
        Interval::UNIT,
        10,
    )];
    let disc = [circle(1, (0.0, 0.0), 0.5)];
    let regions = regions(&disc);

    crate::test_support::assert_cancelled_anywhere(
        "spline sweep",
        || {
            sweep(
                &upright(),
                &regions,
                &path,
                SweepOrientation::Square,
                FEATURE,
            )
        },
        |error| matches!(error, PathSweepError::Cancelled(_)),
    );
}
