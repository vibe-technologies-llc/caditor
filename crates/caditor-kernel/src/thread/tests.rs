use std::{
    f64::consts::{PI, TAU},
    time::Instant,
};

use caditor_geometry::{Aabb2, Plane, Point2, Point3, RigidTransform, Vector3};

use super::*;
use crate::{
    boolean::{BooleanOperation, boolean},
    curve::Curve,
    fixtures,
    intersect::{SurfacePatch, intersect_curve_surface, intersect_surfaces},
    naming::FaceName,
    surface::{Cylinder, PlaneSurface, Surface},
    tolerance::SamplingTolerance,
    topology::PointClass,
};

const FEATURE: u64 = 9;

fn fundamental_height(pitch: f64) -> f64 {
    3.0_f64.sqrt() / 2.0 * pitch
}

fn external(major: f64, pitch: f64) -> ThreadForm {
    ThreadForm {
        pitch,
        flank_angle: 30.0_f64.to_radians(),
        root_radius: major / 2.0 - 5.0 / 8.0 * fundamental_height(pitch),
        root_width: pitch / 4.0,
        crest_radius: major / 2.0,
    }
}

fn internal(major: f64, pitch: f64) -> ThreadForm {
    ThreadForm {
        pitch,
        flank_angle: 30.0_f64.to_radians(),
        root_radius: major / 2.0,
        root_width: pitch / 8.0,
        crest_radius: major / 2.0 - 5.0 / 8.0 * fundamental_height(pitch),
    }
}

fn down_from(top: Point3) -> Plane {
    Plane::from_frame(top, -Vector3::Z, Vector3::X).unwrap()
}

fn shaft_spec(form: ThreadForm, radius: f64, top: f64, length: f64) -> ThreadSpec {
    ThreadSpec {
        frame: down_from(Point3::new(0.0, 0.0, top)),
        face: ThreadedFace::Shaft,
        face_radius: radius,
        form,
        hand: Handedness::Right,
        length,
        start: ThreadEnd::RunsOut,
        end: ThreadEnd::RunsOut,
    }
}

fn bore_spec(form: ThreadForm, radius: f64, height: f64) -> ThreadSpec {
    ThreadSpec {
        frame: down_from(Point3::new(10.0, 10.0, height)),
        face: ThreadedFace::Bore,
        face_radius: radius,
        form,
        hand: Handedness::Right,
        length: height,
        start: ThreadEnd::RunsOut,
        end: ThreadEnd::RunsOut,
    }
}

fn groove_per_length(form: &ThreadForm, from: f64, to: f64) -> f64 {
    let steps = 20_000;
    let width = (to - from) / steps as f64;
    let moment: f64 = (0..steps)
        .map(|step| {
            let radius = from + width * (step as f64 + 0.5);
            2.0 * form.half_width_at(radius) * radius * width
        })
        .sum();
    TAU * moment / form.pitch
}

fn volume(solid: &Solid) -> f64 {
    let tolerance = SamplingTolerance::new(1e-4, 0.02).unwrap();
    solid
        .tessellate(&tolerance)
        .unwrap()
        .mass_properties()
        .volume
}

fn assert_volume(name: &str, solid: &Solid, expected: f64) {
    let measured = volume(solid);
    assert!(
        (measured - expected).abs() < 2e-4 * expected,
        "{name}: {measured} mm³, expected {expected} mm³"
    );
}

fn threaded_timed(name: &str, solid: &Solid, spec: &ThreadSpec) -> Solid {
    let started = Instant::now();
    let threaded = thread(solid, spec, FEATURE).unwrap();
    eprintln!("{name} threaded in {:?}", started.elapsed());
    threaded
}

fn block_with_bore(diameter: f64, height: f64) -> Solid {
    let block = fixtures::cuboid(Vector3::new(20.0, 20.0, height));
    let drill = fixtures::cylinder(diameter / 2.0, height + 2.0)
        .transformed(&RigidTransform::translation(Vector3::new(10.0, 10.0, -1.0)).unwrap())
        .unwrap();
    boolean(&block, &drill, BooleanOperation::Difference).unwrap()
}

#[test]
fn the_groove_tool_has_a_face_per_turn_and_side() {
    let spec = shaft_spec(external(4.0, 0.7), 2.0, 20.0, 20.0);

    let tools = thread_tools(&spec, FEATURE).unwrap();

    assert_eq!(tools.len(), 1);
    let turns = (20.0 + 2.0 * RUN_OUT * 0.7) / 0.7;
    let first_root = FaceName::thread(FEATURE, GROOVE_PARTS.root);
    let named = tools[0]
        .faces()
        .filter(|(_, face)| face.name() == first_root)
        .count();
    assert_eq!(named, 1);
    assert!(tools[0].faces().count() >= 4 * turns.ceil() as usize + 2);
}

#[test]
fn helices_near_a_cylinder_meet_it_without_endless_subdivision() {
    let spec = shaft_spec(external(4.0, 0.7), 2.0, 20.0, 20.0);
    let tool = thread_tools(&spec, FEATURE).unwrap().remove(0);
    let shaft = Surface::Cylinder(Cylinder::new(Plane::XY, 2.0).unwrap());

    for (_, edge) in tool.edges() {
        let found = intersect_curve_surface(edge.curve(), edge.interval(), &shaft, None);

        assert!(found.is_ok(), "{found:?}");
    }
}

#[test]
fn branches_ending_on_a_flank_edge_never_step_back() {
    let spec = ThreadSpec {
        start: ThreadEnd::Stops,
        ..shaft_spec(external(4.0, 0.7), 2.0, 20.0, 20.0)
    };
    let tool = thread_tools(&spec, FEATURE).unwrap().remove(0);
    let plane = Surface::Plane(PlaneSurface::new(Plane::XY).unwrap());
    let around = Aabb2::from_point(Point2::new(-3.0, -3.0)).including(Point2::new(3.0, 3.0));
    let plane_patch = SurfacePatch::new(&plane, around).unwrap();

    let mut branches = 0;
    for (_, face) in tool.faces() {
        let Surface::BSpline(flank) = face.surface() else {
            continue;
        };
        let domain = Aabb2::from_point(Point2::new(flank.u_domain().start(), 0.0))
            .including(Point2::new(flank.u_domain().end(), 1.0));
        let flank_patch = SurfacePatch::new(face.surface(), domain).unwrap();
        let found = intersect_surfaces(&flank_patch, &plane_patch).unwrap();
        for branch in found.branches() {
            let Curve::Intersection(curve) = &branch.curve else {
                continue;
            };
            branches += 1;
            for pair in curve.nodes().windows(2) {
                let chord = pair[1].point - pair[0].point;
                assert!(pair[0].derivative.dot(chord) > 0.0, "{:?}", pair[0].point);
                assert!(pair[1].derivative.dot(chord) > 0.0, "{:?}", pair[1].point);
            }
        }
    }
    assert!(branches >= 2);
}

#[test]
fn an_m4_shaft_is_threaded_its_whole_length() {
    let form = external(4.0, 0.7);
    let shaft = fixtures::cylinder(2.0, 20.0);

    let threaded = threaded_timed("M4x20", &shaft, &shaft_spec(form, 2.0, 20.0, 20.0));

    let expected = PI * 4.0 * 20.0 - groove_per_length(&form, form.root_radius, 2.0) * 20.0;
    assert_volume("M4x20", &threaded, expected);
}

#[test]
fn a_thread_stopping_short_leaves_the_rest_of_the_shaft_round() {
    let form = external(4.0, 0.7);
    let shaft = fixtures::cylinder(2.0, 20.0);
    let spec = ThreadSpec {
        end: ThreadEnd::Stops,
        ..shaft_spec(form, 2.0, 20.0, 8.0)
    };

    let threaded = threaded_timed("M4 stopping", &shaft, &spec);

    let cut = PI * 4.0 * 20.0 - volume(&threaded);
    let per_length = groove_per_length(&form, form.root_radius, 2.0);
    assert!(cut > per_length * 7.0 && cut < per_length * 8.0, "{cut}");
    assert_eq!(
        threaded.classify_point(Point3::new(0.0, form.root_radius + 0.1, 5.0)),
        PointClass::Inside
    );
}

#[test]
fn hands_wind_opposite_ways() {
    let form = external(6.0, 1.0);
    let shaft = fixtures::cylinder(3.0, 10.0);
    let right = shaft_spec(form, 3.0, 10.0, 10.0);
    let left = ThreadSpec {
        hand: Handedness::Left,
        ..right
    };

    let right_handed = threaded_timed("M6 right", &shaft, &right);
    let left_handed = threaded_timed("M6 left", &shaft, &left);

    let quarter_turn = right.frame.origin() + right.frame.normal() * (2.75 * form.pitch);
    let toward = |side: Vector3| quarter_turn + side * (form.root_radius + 0.1);
    let right_groove = toward(right.frame.y_axis());
    let left_groove = toward(-right.frame.y_axis());
    assert_eq!(
        right_handed.classify_point(right_groove),
        PointClass::Outside
    );
    assert_eq!(left_handed.classify_point(left_groove), PointClass::Outside);
    assert_eq!(right_handed.classify_point(left_groove), PointClass::Inside);
    assert_eq!(left_handed.classify_point(right_groove), PointClass::Inside);
    assert!((volume(&right_handed) - volume(&left_handed)).abs() < 1e-3);
}

#[test]
fn points_in_a_threaded_body_classify_as_its_mesh_does() {
    let form = external(6.0, 1.0);
    let shaft = fixtures::cylinder(3.0, 10.0);
    let spec = ThreadSpec {
        hand: Handedness::Left,
        ..shaft_spec(form, 3.0, 10.0, 10.0)
    };
    let threaded = thread(&shaft, &spec, FEATURE).unwrap();
    let mesh = threaded
        .tessellate(&SamplingTolerance::new(1e-4, 0.02).unwrap())
        .unwrap();

    for step in 0..40 {
        let along = 0.5 + 0.2 * step as f64;
        for side in [
            spec.frame.y_axis(),
            -spec.frame.y_axis(),
            spec.frame.x_axis(),
        ] {
            let point =
                spec.frame.origin() + spec.frame.normal() * along + side * (form.root_radius + 0.1);
            let expected = match mesh.contains(point, |_| true) {
                Some(true) => PointClass::Inside,
                Some(false) => PointClass::Outside,
                None => continue,
            };
            assert_eq!(threaded.classify_point(point), expected, "{point}");
        }
    }
}

#[test]
fn an_m8_bore_is_tapped_through() {
    let form = internal(8.0, 1.25);
    let block = block_with_bore(6.8, 12.0);

    let tapped = threaded_timed("M8 tapped", &block, &bore_spec(form, 3.4, 12.0));

    let expected = 20.0 * 20.0 * 12.0
        - PI * 3.4 * 3.4 * 12.0
        - groove_per_length(&form, 3.4, form.root_radius) * 12.0;
    assert_volume("M8 tapped", &tapped, expected);
}

#[test]
fn a_bore_narrower_than_the_minor_diameter_is_bored_out_first() {
    let form = internal(8.0, 1.25);
    let block = block_with_bore(6.0, 12.0);
    let spec = ThreadSpec {
        hand: Handedness::Left,
        ..bore_spec(form, 3.0, 12.0)
    };

    let tapped = threaded_timed("M8 bored out", &block, &spec);

    let minor = form.crest_radius;
    let expected = 20.0 * 20.0 * 12.0
        - PI * minor * minor * 12.0
        - groove_per_length(&form, minor, form.root_radius) * 12.0;
    assert_volume("M8 bored out", &tapped, expected);
}

#[test]
fn a_shaft_wider_than_the_major_diameter_is_turned_down_first() {
    let form = external(4.0, 0.7);
    let shaft = fixtures::cylinder(2.3, 12.0);
    let spec = ThreadSpec {
        end: ThreadEnd::Stops,
        ..shaft_spec(form, 2.3, 12.0, 6.0)
    };

    let threaded = threaded_timed("M4 turned down", &shaft, &spec);

    assert_eq!(
        threaded.classify_point(Point3::new(2.15, 0.0, 9.0)),
        PointClass::Outside
    );
    assert_eq!(
        threaded.classify_point(Point3::new(2.15, 0.0, 3.0)),
        PointClass::Inside
    );
}

#[test]
fn unusable_forms_are_refused() {
    let form = external(4.0, 0.7);
    let shaft = fixtures::cylinder(2.0, 20.0);
    let spec = shaft_spec(form, 2.0, 20.0, 20.0);
    let refused = |spec: ThreadSpec| thread(&shaft, &spec, FEATURE).unwrap_err();

    assert_eq!(
        refused(ThreadSpec {
            form: ThreadForm { pitch: 0.0, ..form },
            ..spec
        }),
        ThreadError::NoPitch
    );
    assert_eq!(
        refused(ThreadSpec {
            form: ThreadForm {
                root_width: 0.6,
                ..form
            },
            ..spec
        }),
        ThreadError::GrooveOverlaps
    );
    assert_eq!(
        refused(ThreadSpec {
            face_radius: 1.5,
            ..spec
        }),
        ThreadError::RootOutsideFace
    );
    assert!(matches!(
        refused(ThreadSpec {
            length: 0.2,
            start: ThreadEnd::Stops,
            end: ThreadEnd::Stops,
            ..spec
        }),
        ThreadError::TooShort { .. }
    ));
    assert!(matches!(
        refused(ThreadSpec {
            length: 500.0,
            form: external(4.0, 0.2),
            ..spec
        }),
        ThreadError::TooManyTurns { .. }
    ));
}
