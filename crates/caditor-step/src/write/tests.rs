use std::time::{Duration, SystemTime};

use crate::{
    fixtures,
    write::{StepBody, WriteError, real, text, timestamp, write_step},
};

fn written(name: &str, solid: &caditor_kernel::Solid) -> String {
    write_step(
        &[StepBody { name, solid }],
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
    assert_eq!(text("🙂"), "'\\X2\\D83DDE42\\X0\\'");
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
fn several_bodies_share_one_representation() {
    let plate = fixtures::plate_with_hole();
    let turned = fixtures::turned();
    let step = write_step(
        &[
            StepBody {
                name: "Plate",
                solid: &plate,
            },
            StepBody {
                name: "Turned",
                solid: &turned,
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
