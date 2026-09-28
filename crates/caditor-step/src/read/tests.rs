use std::time::SystemTime;

use crate::{
    fixtures,
    read::{ReadError, read_step},
    write::{StepBody, write_step},
};

fn round_trip(name: &str, solid: &caditor_kernel::Solid) -> caditor_kernel::Solid {
    let text = write_step(&[StepBody { name, solid }], name, SystemTime::UNIX_EPOCH).unwrap();
    let mut model = read_step(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert!(model.notes.is_empty(), "{name}: {:?}", model.notes);
    assert_eq!(model.solids.len(), 1, "{name}");
    let read = model.solids.remove(0);
    assert_eq!(read.name, name);
    read.solid
}

#[test]
fn every_fixture_survives_a_round_trip() {
    for (name, solid) in fixtures::all() {
        let read = round_trip(name, &solid);
        assert_eq!(read.faces().count(), solid.faces().count(), "{name}");
        assert_eq!(read.edges().count(), solid.edges().count(), "{name}");
        assert_eq!(read.vertices().count(), solid.vertices().count(), "{name}");
        assert_eq!(read.shells().count(), solid.shells().count(), "{name}");
        let (before, after) = (fixtures::volume(&solid), fixtures::volume(&read));
        assert!(
            (before - after).abs() < 1e-6 * before,
            "{name}: {before} vs {after}"
        );
    }
}

#[test]
fn files_without_solids_are_refused_in_words() {
    assert_eq!(read_step("solid cube"), Err(ReadError::NotStep));
    let empty = "ISO-10303-21;HEADER;ENDSEC;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;END-ISO-10303-21;";
    assert_eq!(read_step(empty), Err(ReadError::NoSolids));
    assert_eq!(
        read_step("ISO-10303-21;\nDATA;\n#1=X(;"),
        Err(ReadError::Damaged(3))
    );
}

fn sample(text: &str) -> crate::read::StepModel {
    read_step(text).unwrap_or_else(|error| panic!("{error}"))
}

fn assert_volume(solid: &caditor_kernel::Solid, expected: f64) {
    let volume = fixtures::volume(solid);
    assert!(
        (volume - expected).abs() < 1e-3 * expected,
        "{volume} vs {expected}"
    );
}

#[test]
fn files_from_opencascade_import_with_their_volumes() {
    let tee = sample(include_str!("samples/tee.step"));
    assert_eq!(tee.solids.len(), 1);
    assert_volume(&tee.solids[0].solid, 9992.2416);
    let loft = sample(include_str!("samples/loft.step"));
    assert_volume(&loft.solids[0].solid, 4494.9012);
    let sphere = sample(include_str!("samples/sphere_nurbs.step"));
    assert_volume(
        &sphere.solids[0].solid,
        4.0 / 3.0 * std::f64::consts::PI * 125.0,
    );
    let cavity = sample(include_str!("samples/void.step"));
    assert_eq!(cavity.solids[0].solid.shells().count(), 2);
    assert_volume(
        &cavity.solids[0].solid,
        8000.0 - 4.0 / 3.0 * std::f64::consts::PI * 125.0,
    );
    let torus = sample(include_str!("samples/torus_part.step"));
    assert_volume(&torus.solids[0].solid, 1332.3966);
    let pair = sample(include_str!("samples/two_solids.step"));
    assert_eq!(pair.solids.len(), 2);
    assert_volume(&pair.solids[0].solid, 1000.0);
    assert_volume(&pair.solids[1].solid, std::f64::consts::PI * 9.0 * 10.0);
    for model in [&tee, &loft, &sphere, &cavity, &torus, &pair] {
        assert!(model.notes.is_empty(), "{:?}", model.notes);
    }
}

#[test]
fn assembly_parts_are_placed_and_named_after_their_products() {
    let model = sample(include_str!("samples/assembly.step"));
    let names: Vec<&str> = model
        .solids
        .iter()
        .map(|solid| solid.name.as_str())
        .collect();
    assert_eq!(names, ["Block", "Pin"]);
    let bounds = |index: usize| model.solids[index].solid.bounding_box().unwrap();
    let near = |a: caditor_geometry::Point3, b: [f64; 3]| {
        a.distance(caditor_geometry::Point3::from_array(b)) < 1e-9
    };
    assert!(near(bounds(0).min(), [80.0, 0.0, 0.0]) && near(bounds(0).max(), [100.0, 10.0, 5.0]));
    assert!(near(bounds(1).min(), [-2.0, 20.0, -2.0]) && near(bounds(1).max(), [2.0, 50.0, 2.0]));
}

#[test]
fn lengths_follow_the_unit_of_the_file() {
    let solid = fixtures::plate_with_hole();
    let text = write_step(
        &[StepBody {
            name: "Plate",
            solid: &solid,
        }],
        "Plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
    .replace("SI_UNIT(.MILLI.,.METRE.)", "SI_UNIT(.CENTI.,.METRE.)");
    let model = sample(&text);
    let expected = fixtures::volume(&solid) * 1000.0;
    assert_volume(&model.solids[0].solid, expected);
    assert_eq!(
        model.notes,
        ["The file measures lengths in centimetres, so they were converted to millimetres."]
    );
    let inches = write_step(
        &[StepBody {
            name: "Plate",
            solid: &solid,
        }],
        "Plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
    .replace(
        "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))",
        "(CONVERSION_BASED_UNIT('INCH',#9001) LENGTH_UNIT() NAMED_UNIT(#9002))",
    )
    .replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        "#9001=(LENGTH_MEASURE_WITH_UNIT() MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#9003));\n\
         #9002=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);\n\
         #9003=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));\n\
         ENDSEC;\nEND-ISO-10303-21;",
    );
    assert!(inches.contains("#9001="));
    let model = sample(&inches);
    assert_volume(
        &model.solids[0].solid,
        fixtures::volume(&solid) * 25.4_f64.powi(3),
    );
    assert_eq!(
        model.notes,
        ["The file measures lengths in inches, so they were converted to millimetres."]
    );
    let unnamed = write_step(
        &[StepBody {
            name: "Plate",
            solid: &solid,
        }],
        "Plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
    .replace(
        "GLOBAL_UNIT_ASSIGNED_CONTEXT(",
        "GLOBAL_UNIT_ASSIGNED_CONTEX(",
    );
    let model = sample(&unnamed);
    assert_eq!(
        model.notes,
        ["The file does not say which unit it uses, so its numbers were read as millimetres."]
    );
}

#[test]
fn bezier_and_uniform_curves_and_surfaces_are_read_with_the_standard_knots() {
    use caditor_geometry::Point3;
    use caditor_kernel::{Curve, Surface};

    use crate::{
        part21::parse,
        read::{geometry::Geometry, graph::Graph, units::Units},
    };

    let points: Vec<(f64, f64, f64)> = (0..7)
        .map(|index| {
            let x = index as f64;
            (x, if index % 3 == 0 { 0.0 } else { 2.0 }, 0.0)
        })
        .collect();
    let mut data: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(index, (x, y, z))| {
            format!("#{}=CARTESIAN_POINT('',({x:?},{y:?},{z:?}));", index + 1)
        })
        .collect();
    let list = (1..=7)
        .map(|index| format!("#{index}"))
        .collect::<Vec<_>>()
        .join(",");
    data.push(format!(
        "#20=BEZIER_CURVE('',3,({list}),.UNSPECIFIED.,.F.,.F.);"
    ));
    data.push(format!(
        "#21=QUASI_UNIFORM_CURVE('',3,({list}),.UNSPECIFIED.,.F.,.F.);"
    ));
    data.push(format!(
        "#22=UNIFORM_CURVE('',3,({list}),.UNSPECIFIED.,.F.,.F.);"
    ));
    data.push(format!(
        "#23=(BEZIER_CURVE() BOUNDED_CURVE() B_SPLINE_CURVE(3,({list}),.UNSPECIFIED.,.F.,.F.) \
         CURVE() GEOMETRIC_REPRESENTATION_ITEM() REPRESENTATION_ITEM(''));"
    ));
    let grid = "((#1,#2,#3,#4),(#2,#3,#4,#5),(#3,#4,#5,#6),(#4,#5,#6,#7))";
    data.push(format!(
        "#30=BEZIER_SURFACE('',3,3,{grid},.UNSPECIFIED.,.F.,.F.,.F.);"
    ));
    let text = format!(
        "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n{}\nENDSEC;\nEND-ISO-10303-21;\n",
        data.join("\n")
    );
    let exchange = parse(&text).unwrap();
    let geometry = Geometry {
        graph: Graph::new(&exchange),
        units: Units::default(),
    };
    let point = |index: usize| {
        let (x, y, z) = points[index];
        Point3::new(x, y, z)
    };
    for id in [20, 23] {
        let Curve::BSpline(bezier) = geometry.curve(id).unwrap() else {
            panic!("a Bézier curve is a B-spline");
        };
        let domain = bezier.domain();
        let joint = bezier.point(domain.start() + domain.length() / 2.0);
        assert!(joint.distance(point(3)) < 1e-12, "#{id}: {joint}");
    }
    for id in [21, 22] {
        assert!(matches!(geometry.curve(id), Ok(Curve::BSpline(_))), "#{id}");
    }
    assert!(matches!(geometry.surface(30), Ok(Surface::BSpline(_))));
}
