use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Plane, Point3, Vector3};

use super::*;
use crate::{
    bspline::BSpline,
    curve::{Circle, Curve, Line},
    interval::Interval,
    measure::mass_properties_tessellating,
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

fn whole_circle(radius: f64) -> PathSegment {
    let frame = Plane::from_frame(Point3::ZERO, Vector3::Z, Vector3::X).unwrap();
    PathSegment::new(
        Curve::Circle(Circle::new(frame, radius).unwrap()),
        Interval::new(0.0, TAU).unwrap(),
        10,
    )
}

fn volume(solid: &Solid) -> f64 {
    mass_properties_tessellating(solid, &SamplingTolerance::new(1e-3, 0.05).unwrap())
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

fn shaped(
    plane: &Plane,
    curves: &[ProfileCurve],
    path: &[PathSegment],
    (orientation, twist, scale): (SweepOrientation, f64, f64),
) -> Result<Solid, PathSweepError> {
    sweep_shaped(
        plane,
        &regions(curves),
        path,
        (orientation, SweepShape { twist, scale }),
        FEATURE,
    )
}

fn twisted(curves: &[ProfileCurve], path: &[PathSegment], twist: f64) -> Solid {
    shaped(
        &upright(),
        curves,
        path,
        (SweepOrientation::Square, twist, 1.0),
    )
    .unwrap()
}

fn corner_heights(solid: &Solid, at: f64) -> Vec<Point3> {
    solid
        .vertices()
        .map(|(_, vertex)| vertex.point())
        .filter(|point| (point.y - at).abs() < 1e-9)
        .collect()
}

#[test]
fn a_twisted_square_prism_keeps_its_area_times_its_length() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let path = [line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0))];

    let quarter = twisted(&profile, &path, FRAC_PI_2);
    let whole = twisted(&profile, &path, TAU);

    assert_eq!(quarter.validate(), Ok(()));
    assert_eq!(whole.validate(), Ok(()));
    assert_close("quarter turn", volume(&quarter), 40.0, 1e-4);
    assert_close("whole turn", volume(&whole), 40.0, 1e-4);
    assert_eq!(quarter.faces().count(), 6);
}

#[test]
fn a_positive_twist_turns_the_end_counter_clockwise_about_the_way_the_path_runs() {
    let profile = rectangle(1, (0.0, -0.5), (2.0, 0.5));
    let path = [line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0))];

    let solid = twisted(&profile, &path, FRAC_PI_2);
    let end = corner_heights(&solid, 10.0);

    assert_eq!(end.len(), 4);
    assert!(
        end.iter()
            .all(|point| point.x.abs() <= 0.5 + 1e-9 && point.z <= 1e-9),
        "{end:?}"
    );
}

#[test]
fn a_tapered_sweep_along_a_line_is_a_frustum() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let disc = [circle(1, (0.0, 0.0), 1.0)];
    let path = [line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0))];
    let frustum = |bottom: f64, top: f64| 10.0 / 3.0 * (bottom + top + (bottom * top).sqrt());

    let square = shaped(
        &upright(),
        &profile,
        &path,
        (SweepOrientation::Square, 0.0, 0.5),
    )
    .unwrap();
    let parallel = shaped(
        &upright(),
        &profile,
        &path,
        (SweepOrientation::Parallel, 0.0, 0.5),
    )
    .unwrap();
    let cone = shaped(
        &upright(),
        &disc,
        &path,
        (SweepOrientation::Square, PI, 2.0),
    )
    .unwrap();

    assert_eq!(square.validate(), Ok(()));
    assert_eq!(cone.validate(), Ok(()));
    assert_close("square frustum", volume(&square), frustum(4.0, 1.0), 1e-4);
    assert_close(
        "parallel frustum",
        volume(&parallel),
        frustum(4.0, 1.0),
        1e-4,
    );
    assert_close("cone frustum", volume(&cone), frustum(PI, 4.0 * PI), 1e-4);
    let end = corner_heights(&square, 10.0);
    assert!(
        end.iter()
            .all(|point| (point.x.abs() - 0.5).abs() < 1e-9 && (point.z.abs() - 0.5).abs() < 1e-9),
        "{end:?}"
    );
}

#[test]
fn a_twisted_block_along_an_arc_keeps_pappus_volume() {
    let radius = 5.0;
    let block = rectangle(1, (radius - 1.0, -0.5), (radius + 1.0, 0.5));
    let frame = Plane::from_frame(Point3::ZERO, Vector3::Z, Vector3::X).unwrap();
    let path = [PathSegment::new(
        Curve::Circle(Circle::new(frame, radius).unwrap()),
        Interval::new(0.0, FRAC_PI_2).unwrap(),
        10,
    )];

    let solid = twisted(&block, &path, PI);

    assert_eq!(solid.validate(), Ok(()));
    assert_close(
        "twisted bend",
        volume(&solid),
        2.0 * radius * FRAC_PI_2,
        1e-4,
    );
}

#[test]
fn a_planar_closed_path_closes_after_a_whole_turn_of_twist_only() {
    let radius = 5.0;
    let block = rectangle(1, (radius - 1.0, -0.5), (radius + 1.0, 0.5));
    let path = [whole_circle(radius)];

    let ring = twisted(&block, &path, TAU);
    let quarter = shaped(
        &upright(),
        &block,
        &path,
        (SweepOrientation::Square, FRAC_PI_2, 1.0),
    );
    let scaled = shaped(
        &upright(),
        &block,
        &path,
        (SweepOrientation::Square, 0.0, 0.5),
    );

    assert_eq!(ring.validate(), Ok(()));
    assert_close("twisted ring", volume(&ring), 2.0 * TAU * radius, 1e-4);
    match quarter {
        Err(PathSweepError::ClosedPathTwists {
            turned: Some(turned),
        }) => assert!((turned - FRAC_PI_2).abs() < 1e-6, "{turned}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(scaled.err(), Some(PathSweepError::ClosedPathScales));
}

fn saddle(lift: f64, sink: f64) -> Vec<PathSegment> {
    let reach = 4.0 / 3.0 * 5.0;
    let half = |sign: f64, [first, second]: [f64; 2]| {
        BSpline::new(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![
                Point3::new(5.0 * sign, 0.0, 0.0),
                Point3::new(5.0 * sign, reach * sign, first),
                Point3::new(-5.0 * sign, reach * sign, second),
                Point3::new(-5.0 * sign, 0.0, 0.0),
            ],
        )
        .unwrap()
    };
    vec![
        PathSegment::new(Curve::BSpline(half(1.0, [lift, sink])), Interval::UNIT, 10),
        PathSegment::new(
            Curve::BSpline(half(-1.0, [-sink, -lift])),
            Interval::UNIT,
            11,
        ),
    ]
}

#[test]
fn a_closed_path_out_of_one_plane_closes_once_twist_brings_the_profile_back() {
    let path = saddle(4.0, -2.0);
    let start = path.first().unwrap();
    let tangent = start.curve().evaluate(0.0).first.normalize();
    let plane = Plane::with_x_axis(Point3::new(5.0, 0.0, 0.0), tangent, Vector3::X).unwrap();
    let profile = rectangle(1, (-0.5, -0.5), (0.5, 0.5));
    let length: f64 = path
        .iter()
        .map(|segment| segment.curve().length(segment.range()))
        .sum();

    let refused = shaped(
        &plane,
        &profile,
        &path,
        (SweepOrientation::Square, 0.0, 1.0),
    );
    let Err(PathSweepError::ClosedPathTwists {
        turned: Some(turned),
    }) = refused
    else {
        panic!("{refused:?}");
    };
    let needed = (-turned * 1e5).round() / 1e5;
    let closed = shaped(
        &plane,
        &profile,
        &path,
        (SweepOrientation::Square, needed, 1.0),
    )
    .unwrap();
    let wound = shaped(
        &plane,
        &profile,
        &path,
        (SweepOrientation::Square, needed + TAU, 1.0),
    )
    .unwrap();

    assert!(turned.abs() > 1e-3, "{turned}");
    assert_eq!(closed.validate(), Ok(()));
    assert_eq!(wound.validate(), Ok(()));
    assert_close("saddle ring", volume(&closed), length, 1e-3);
    assert_close("wound saddle ring", volume(&wound), length, 1e-3);
}

#[test]
fn a_shaped_sweep_cannot_mitre_a_corner_and_refuses_unusable_shapes() {
    let profile = rectangle(1, (-1.0, -1.0), (1.0, 1.0));
    let path = [
        line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0)),
        line(
            11,
            Point3::new(0.0, 10.0, 0.0),
            Point3::new(10.0, 10.0, 0.0),
        ),
    ];
    let straight = [line(10, Point3::ZERO, Point3::new(0.0, 10.0, 0.0))];

    let cornered = shaped(
        &upright(),
        &profile,
        &path,
        (SweepOrientation::Square, 1.0, 1.0),
    );
    let unusable = [
        (f64::NAN, 1.0),
        (0.0, 0.0),
        (0.0, -1.0),
        (f64::INFINITY, 1.0),
    ]
    .map(|(twist, scale)| {
        shaped(
            &upright(),
            &profile,
            &straight,
            (SweepOrientation::Square, twist, scale),
        )
        .err()
    });

    assert_eq!(
        cornered.err(),
        Some(PathSweepError::ShapedCorner { segments: [0, 1] })
    );
    assert!(
        unusable
            .iter()
            .all(|error| *error == Some(PathSweepError::UnusableShape))
    );
}
