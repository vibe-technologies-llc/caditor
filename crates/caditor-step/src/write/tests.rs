use std::time::{Duration, SystemTime};

use caditor_geometry::{Plane, Point2, Point3, Vector3};
use caditor_kernel::ProfileCurve;

use crate::{
    fixtures, read_step,
    write::{
        Data, StepBody, StepDetails, WriteError, real, text, timestamp, write_step,
        write_step_detailed,
    },
};

fn written(name: &str, solid: &caditor_kernel::Solid) -> String {
    write_step(
        &[StepBody {
            name,
            solid,
            colour: None,
        }],
        "part",
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000),
    )
    .unwrap()
}

fn count(step: &str, entity: &str) -> usize {
    step.matches(&format!("={entity}(")).count()
}

#[test]
fn reals_always_carry_a_decimal_point_and_round_trip() {
    assert_eq!(real(1.0), "1.0");
    assert_eq!(real(-0.5), "-0.5");
    assert_eq!(real(1e-7), "1.E-7");
    assert_eq!(real(1.5e300), "1.5E300");
    for value in [0.1, 1.0 / 3.0, -123_456.789, 6.02e23, 5e-324] {
        assert_eq!(real(value).replace('E', "e").parse::<f64>().unwrap(), value);
    }
}

#[test]
fn strings_escape_quotes_backslashes_and_other_scripts() {
    assert_eq!(text("it's"), "'it''s'");
    assert_eq!(text("a\\b"), "'a\\\\b'");
    assert_eq!(text("Kühler"), "'K\\X2\\00FC\\X0\\hler'");
    assert_eq!(text("🙂"), "'\\X4\\0001F642\\X0\\'");
    let written = text("Kühler 🙂");
    assert_eq!(
        crate::part21::decode_text(written.trim_matches('\'')),
        "Kühler 🙂"
    );
}

#[test]
fn timestamps_are_iso_dates_in_utc() {
    let moment = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
    assert_eq!(timestamp(moment), "2026-09-21T14:13:20");
    assert_eq!(timestamp(SystemTime::UNIX_EPOCH), "1970-01-01T00:00:00");
}

#[test]
fn a_plate_with_a_hole_is_one_closed_shell_of_advanced_faces() {
    let solid = fixtures::plate_with_hole();
    let step = written("Plate", &solid);
    assert!(step.starts_with("ISO-10303-21;\nHEADER;\n"));
    assert!(step.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    assert!(step.contains("FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));"));
    assert!(step.contains("'2026-09-21T14:13:20'"));
    assert!(step.contains("SI_UNIT(.MILLI.,.METRE.)"));
    assert_eq!(count(&step, "MANIFOLD_SOLID_BREP"), 1);
    assert!(step.contains("=MANIFOLD_SOLID_BREP('Plate',"));
    assert_eq!(count(&step, "ADVANCED_FACE"), solid.faces().count());
    assert_eq!(count(&step, "EDGE_CURVE"), solid.edges().count());
    assert_eq!(count(&step, "VERTEX_POINT"), solid.vertices().count());
    assert_eq!(count(&step, "CYLINDRICAL_SURFACE"), 1);
    assert_eq!(count(&step, "FACE_OUTER_BOUND"), solid.faces().count());
    assert_eq!(count(&step, "FACE_BOUND"), 2);
    assert!(!step.contains("NaN") && !step.contains("inf"));
}

#[test]
fn a_hollow_ring_keeps_its_void_as_an_inverted_shell() {
    let step = written("Ring", &fixtures::hollow_ring());
    assert_eq!(count(&step, "BREP_WITH_VOIDS"), 1);
    assert_eq!(count(&step, "ORIENTED_CLOSED_SHELL"), 1);
    assert_eq!(count(&step, "CLOSED_SHELL"), 2);
    assert!(count(&step, "TOROIDAL_SURFACE") >= 1);
}

#[test]
fn every_kind_of_edge_and_face_is_written() {
    let cylinders = written("Cross", &fixtures::crossed_cylinders());
    assert!(count(&cylinders, "B_SPLINE_CURVE_WITH_KNOTS") >= 2);
    let prism = written("Prism", &fixtures::spline_prism());
    assert_eq!(count(&prism, "SURFACE_OF_LINEAR_EXTRUSION"), 1);
    let turned = written("Turned", &fixtures::turned());
    assert!(count(&turned, "CONICAL_SURFACE") == 1);
    assert!(count(&turned, "TOROIDAL_SURFACE") == 1);
    let filleted = written("Block", &fixtures::filleted_block());
    assert!(count(&filleted, "CYLINDRICAL_SURFACE") >= 4);
    assert!(write_step(&[], "none", SystemTime::UNIX_EPOCH) == Err(WriteError::Empty));
}

#[test]
fn without_details_the_product_is_named_after_its_body_and_the_header_after_the_model() {
    let step = written("Plate", &fixtures::plate_with_hole());

    assert!(step.contains("FILE_DESCRIPTION(('part'),'2;1');"));
    assert!(step.contains("FILE_NAME('part','"));
    assert!(step.contains("',(''),(''),'caditor "));
    assert!(step.contains("=PRODUCT('Plate','Plate','',("));
    assert!(step.contains("=PRODUCT_DEFINITION_FORMATION('','',#"));
}

#[test]
fn model_details_fill_the_header_product_and_revision() {
    let plate = fixtures::plate_with_hole();
    let details = StepDetails {
        title: "Wall bracket",
        part_number: "BR-100",
        revision: "C",
        description: "Holds a shelf",
        author: "Drafter",
        organisation: "Workshop",
    };

    let step = write_step_detailed(
        &[StepBody {
            name: "Plate",
            solid: &plate,
            colour: None,
        }],
        "bracket",
        &details,
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
    .text;
    let read = read_step(&step).unwrap();

    assert!(step.contains("FILE_DESCRIPTION(('Holds a shelf'),'2;1');"));
    assert!(step.contains("FILE_NAME('bracket','1970-01-01T00:00:00',('Drafter'),('Workshop'),"));
    assert!(step.contains("=PRODUCT('BR-100','Wall bracket','Holds a shelf',("));
    assert!(step.contains("=PRODUCT_DEFINITION_FORMATION('C','',#"));
    assert_eq!(read.solids.len(), 1);
}

#[test]
fn several_bodies_share_one_representation() {
    let plate = fixtures::plate_with_hole();
    let turned = fixtures::turned();
    let step = write_step(
        &[
            StepBody {
                name: "Plate",
                solid: &plate,
                colour: None,
            },
            StepBody {
                name: "Turned",
                solid: &turned,
                colour: None,
            },
        ],
        "model",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    assert_eq!(count(&step, "MANIFOLD_SOLID_BREP"), 2);
    assert_eq!(count(&step, "ADVANCED_BREP_SHAPE_REPRESENTATION"), 1);
}

#[test]
fn coloured_bodies_are_styled_and_bodies_of_one_colour_share_their_style() {
    let plate = fixtures::plate_with_hole();
    let turned = fixtures::turned();
    let other = fixtures::plate_with_hole();
    let step = write_step(
        &[
            StepBody {
                name: "Plate",
                solid: &plate,
                colour: Some([255, 0, 51]),
            },
            StepBody {
                name: "Turned",
                solid: &turned,
                colour: None,
            },
            StepBody {
                name: "Other",
                solid: &other,
                colour: Some([255, 0, 51]),
            },
        ],
        "model",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();

    assert_eq!(count(&step, "COLOUR_RGB"), 1);
    assert_eq!(count(&step, "PRESENTATION_STYLE_ASSIGNMENT"), 1);
    assert_eq!(count(&step, "STYLED_ITEM"), 2);
    assert_eq!(
        count(
            &step,
            "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION"
        ),
        1
    );
    assert!(step.contains("=COLOUR_RGB('',1.0,0.0,0.2);"));

    let styled: Vec<&str> = step
        .lines()
        .filter_map(|line| line.split_once("=STYLED_ITEM('color',("))
        .filter_map(|(_, rest)| rest.split_once("),"))
        .map(|(_, solid)| solid.trim_end_matches(");"))
        .collect();
    let breps: Vec<(&str, &str)> = step
        .lines()
        .filter_map(|line| line.split_once("=MANIFOLD_SOLID_BREP('"))
        .filter_map(|(id, rest)| Some((id, rest.split_once('\'')?.0)))
        .collect();
    let named = |name: &str| breps.iter().find(|brep| brep.1 == name).unwrap().0;
    assert_eq!(styled, [named("Plate"), named("Other")]);
}

#[test]
fn uncoloured_bodies_write_no_presentation() {
    let step = written("Plate", &fixtures::plate_with_hole());

    assert_eq!(count(&step, "STYLED_ITEM"), 0);
    assert_eq!(
        count(
            &step,
            "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION"
        ),
        0
    );
}

#[test]
fn points_directions_and_placements_are_written_once() {
    for (name, solid) in fixtures::all() {
        let step = written(name, &solid);
        for entity in ["CARTESIAN_POINT", "DIRECTION", "AXIS2_PLACEMENT_3D"] {
            let mut bodies: Vec<&str> = step
                .lines()
                .filter_map(|line| line.split_once(&format!("={entity}(")))
                .map(|(_, body)| body)
                .collect();
            let total = bodies.len();
            bodies.sort_unstable();
            bodies.dedup();
            assert_eq!(bodies.len(), total, "{name}: repeated {entity}");
        }
    }
}

#[test]
fn a_many_sided_prism_writes_each_corner_once() {
    let sides = 500;
    let corners: Vec<Point2> = (0..sides)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / sides as f64;
            Point2::new(50.0 * angle.cos(), 50.0 * angle.sin())
        })
        .collect();
    let curves: Vec<ProfileCurve> = (0..sides)
        .map(|index| {
            let start = corners[index];
            let end = corners[(index + 1) % sides];
            ProfileCurve::line(index as u64 + 1, start, end)
        })
        .collect();
    let solid = fixtures::swept(Plane::XY, &curves, 10.0);
    let step = written("Prism", &solid);
    assert!(count(&step, "CARTESIAN_POINT") <= 2 * sides + 8);
}

#[test]
fn rolling_back_forgets_what_a_failed_body_wrote_and_what_it_shared() {
    let mut data = Data::default();
    let kept = data.point(Point3::new(1.0, 2.0, 3.0));
    let checkpoint = data.checkpoint();

    let dropped = data.point(Point3::new(4.0, 5.0, 6.0));
    data.direction(Vector3::Z);
    data.placement(Point3::ZERO, Vector3::Z, Vector3::X);
    data.real(f64::NAN);
    data.roll_back(checkpoint);

    assert_eq!(data.entities.len(), 1);
    assert!(!data.take_unwritable());
    assert_eq!(data.point(Point3::new(1.0, 2.0, 3.0)), kept);
    assert_eq!(data.point(Point3::new(4.0, 5.0, 6.0)), dropped);
    assert_eq!(data.entities.len(), 2);
}

#[test]
fn samples_for_an_outside_checker() {
    let Some(directory) = std::env::var_os("STEP_SAMPLES") else {
        return;
    };
    for (name, solid) in fixtures::all() {
        let step = written(name, &solid);
        let path =
            std::path::Path::new(&directory).join(format!("{}.step", name.replace(' ', "_")));
        std::fs::write(&path, step).unwrap();
        println!("{} {}", path.display(), fixtures::volume(&solid));
    }
}
