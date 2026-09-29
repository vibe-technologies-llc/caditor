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
        read::{
            geometry::{Geometry, MAX_WORK, Work},
            graph::Graph,
            units::Units,
        },
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
    let geometry = Geometry::new(Graph::new(&exchange), Units::default(), Work::new(MAX_WORK));
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

#[test]
fn a_single_body_takes_its_product_name_and_several_keep_their_own() {
    let solid = fixtures::plate_with_hole();
    let single = write_step(
        &[StepBody {
            name: "Body1",
            solid: &solid,
        }],
        "Bracket",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    assert_eq!(sample(&single).solids[0].name, "Body1");
    let from_elsewhere = single.replace(
        "MANIFOLD_SOLID_BREP('Body1'",
        "MANIFOLD_SOLID_BREP('Solid 7'",
    );
    assert_eq!(sample(&from_elsewhere).solids[0].name, "Body1");
    let product_named = single.replace("PRODUCT('Body1','Body1'", "PRODUCT('Bracket','Bracket'");
    assert!(product_named.contains("PRODUCT('Bracket'"));
    assert_eq!(sample(&product_named).solids[0].name, "Bracket");
    let pair = write_step(
        &[
            StepBody {
                name: "Left",
                solid: &solid,
            },
            StepBody {
                name: "Right",
                solid: &solid,
            },
        ],
        "Bracket",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let names: Vec<String> = sample(&pair)
        .solids
        .into_iter()
        .map(|solid| solid.name)
        .collect();
    assert_eq!(names, ["Left", "Right"]);
}

fn faceted_cube(top: &str, missing_face: bool) -> String {
    const FACES: [[usize; 4]; 6] = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let mut data = String::new();
    for index in 0..8 {
        let pick = |bit: usize| if index & bit == 0 { 0.0 } else { 10.0 };
        data.push_str(&format!(
            "#{}=CARTESIAN_POINT('',({:?},{:?},{:?}));\n",
            index + 1,
            pick(1),
            pick(2),
            pick(4)
        ));
    }
    let mut faces = Vec::new();
    for (face, corners) in FACES.iter().enumerate() {
        if missing_face && face == 5 {
            continue;
        }
        let mut points: Vec<String> = corners
            .iter()
            .map(|corner| format!("#{}", corner + 1))
            .collect();
        let orientation = if face == 3 {
            points.reverse();
            ".F."
        } else {
            ".T."
        };
        data.push_str(&format!(
            "#{}=POLY_LOOP('',({}));\n#{}=FACE_OUTER_BOUND('',#{},{orientation});\n#{}=FACE('',(#{}));\n",
            10 + face,
            points.join(","),
            20 + face,
            10 + face,
            30 + face,
            20 + face
        ));
        faces.push(format!("#{}", 30 + face));
    }
    let shell = if missing_face {
        "OPEN_SHELL"
    } else {
        "CLOSED_SHELL"
    };
    data.push_str(&format!(
        "#40={shell}('',({}));\n#41={top};\n",
        faces.join(",")
    ));
    format!("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n{data}ENDSEC;\nEND-ISO-10303-21;\n")
}

#[test]
fn a_solid_is_imported_past_an_entry_that_cannot_be_read_with_a_note() {
    let text = faceted_cube("FACETED_BREP('cube',#40)", false).replacen(
        "DATA;\n",
        "DATA;\n#900=@@;\n#901=THING(1,2 3);\n",
        1,
    );

    let model = sample(&text);

    assert_eq!(model.solids.len(), 1);
    assert_volume(&model.solids[0].solid, 1000.0);
    assert!(
        model
            .notes
            .iter()
            .any(|note| note.starts_with("2 entries of the file") && note.contains("line 5")),
        "{:?}",
        model.notes
    );
}

#[test]
fn faceted_solids_and_closed_surface_models_are_imported() {
    for top in [
        "FACETED_BREP('cube',#40)",
        "SHELL_BASED_SURFACE_MODEL('cube',(#40))",
    ] {
        let model = sample(&faceted_cube(top, false));
        assert_eq!(model.solids.len(), 1, "{top}");
        assert_eq!(model.solids[0].solid.faces().count(), 6, "{top}");
        assert_volume(&model.solids[0].solid, 1000.0);
    }
    let open = read_step(&faceted_cube(
        "SHELL_BASED_SURFACE_MODEL('open',(#40))",
        true,
    ));
    assert_eq!(
        open.map(|model| model.solids.len()),
        Err(ReadError::NoSolids)
    );
}

#[test]
fn a_composite_curve_follows_its_trimmed_segments() {
    use caditor_geometry::Point3;

    use crate::{
        part21::parse,
        read::{
            geometry::{Geometry, MAX_WORK, Work},
            graph::Graph,
            units::Units,
        },
    };

    let text = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
        #1=CARTESIAN_POINT('',(0.,0.,0.));#2=CARTESIAN_POINT('',(10.,0.,0.));\n\
        #3=CARTESIAN_POINT('',(10.,5.,0.));#4=DIRECTION('',(1.,0.,0.));\n\
        #5=DIRECTION('',(0.,1.,0.));#6=VECTOR('',#4,2.);#7=VECTOR('',#5,1.);\n\
        #8=LINE('',#1,#6);#9=LINE('',#2,#7);\n\
        #10=TRIMMED_CURVE('',#8,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(5.)),.T.,.PARAMETER.);\n\
        #11=TRIMMED_CURVE('',#9,(#3),(#2),.F.,.CARTESIAN.);\n\
        #12=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#10);\n\
        #13=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.F.,#11);\n\
        #14=COMPOSITE_CURVE('',(#12,#13),.F.);\nENDSEC;\nEND-ISO-10303-21;\n";
    let exchange = parse(text).unwrap();
    let geometry = Geometry::new(Graph::new(&exchange), Units::default(), Work::new(MAX_WORK));
    let curve = geometry.curve(14).unwrap();
    let range = curve.domain().bounded().unwrap();
    let ends = [curve.point(range.start()), curve.point(range.end())];
    assert!(ends[0].distance(Point3::ZERO) < 1e-9, "{}", ends[0]);
    assert!(
        ends[1].distance(Point3::new(10.0, 5.0, 0.0)) < 1e-9,
        "{}",
        ends[1]
    );
    let corner = curve.point(range.start() + 10.0);
    assert!(
        corner.distance(Point3::new(10.0, 0.0, 0.0)) < 1e-9,
        "{corner}"
    );
}

#[test]
fn composite_curves_nested_many_times_are_built_once_each_within_the_budget() {
    use crate::{
        part21::parse,
        read::{
            geometry::{Geometry, MAX_WORK, Work},
            graph::Graph,
            units::Units,
        },
    };

    const LEVELS: u64 = 6;
    const SEGMENTS: usize = 20;
    let mut data = vec![
        "#1=CARTESIAN_POINT('',(0.,0.,0.));#2=DIRECTION('',(1.,0.,0.));".to_owned(),
        "#3=VECTOR('',#2,1.);#4=LINE('',#1,#3);".to_owned(),
        "#5=TRIMMED_CURVE('',#4,(PARAMETER_VALUE(0.)),(PARAMETER_VALUE(5.)),.T.,.PARAMETER.);"
            .to_owned(),
    ];
    for level in 1..=LEVELS {
        let next = if level == LEVELS { 5 } else { 100 + level + 1 };
        let segments = vec![format!("#{}", 200 + level); SEGMENTS].join(",");
        data.push(format!(
            "#{}=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#{next});",
            200 + level
        ));
        data.push(format!(
            "#{}=COMPOSITE_CURVE('',({segments}),.F.);",
            100 + level
        ));
    }
    let text = format!(
        "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n{}\nENDSEC;\nEND-ISO-10303-21;\n",
        data.join("\n")
    );
    let exchange = parse(&text).unwrap();
    let geometry =
        |budget: usize| Geometry::new(Graph::new(&exchange), Units::default(), Work::new(budget));

    let curve = geometry(MAX_WORK).curve(101).unwrap();
    let starved = geometry(10).curve(101).unwrap_err();

    assert!(curve.domain().bounded().is_some());
    assert!(starved.reason.contains("too intricate"), "{starved}");
}

fn spindle(major: f64, minor: f64, outer: bool) -> String {
    let pole = -(minor * minor - major * major).sqrt();
    let select = if outer { ".T." } else { ".F." };
    format!(
        "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
        #1=CARTESIAN_POINT('',(0.,0.,0.));#2=DIRECTION('',(0.,0.,1.));\n\
        #3=DIRECTION('',(1.,0.,0.));#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);\n\
        #5=DEGENERATE_TOROIDAL_SURFACE('',#4,{major:?},{minor:?},{select});\n\
        #6=CARTESIAN_POINT('',(0.,0.,{pole:?}));#7=VERTEX_POINT('',#6);\n\
        #8=VERTEX_LOOP('',#7);#9=FACE_BOUND('',#8,.T.);\n\
        #10=ADVANCED_FACE('',(#9),#5,.T.);#11=CLOSED_SHELL('',(#10));\n\
        #12=MANIFOLD_SOLID_BREP('spindle',#11);\nENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn spindle_tori_are_read_as_the_apple_or_the_lemon() {
    use std::f64::consts::PI;

    let (major, minor) = (2.0_f64, 5.0_f64);
    let beyond = minor * minor - major * major;
    let segment = minor * minor * (major / minor).acos() - major * beyond.sqrt();
    let reach = 2.0 * beyond.powf(1.5) / (3.0 * segment);
    let apple = sample(&spindle(major, minor, true));
    assert_eq!(apple.solids.len(), 1);
    assert_volume(
        &apple.solids[0].solid,
        2.0 * PI * (PI * minor * minor * major - segment * (major - reach)),
    );
    let lemon = sample(&spindle(major, minor, false));
    assert_volume(&lemon.solids[0].solid, 2.0 * PI * segment * (reach - major));
    for (name, model) in [("apple", &apple), ("lemon", &lemon)] {
        let solid = &model.solids[0].solid;
        let read = round_trip(name, solid);
        assert_eq!(read.faces().count(), solid.faces().count(), "{name}");
        assert_eq!(read.edges().count(), solid.edges().count(), "{name}");
    }
    let horn = read_step(&spindle(3.0, 3.0, true));
    assert!(
        matches!(&horn, Err(error) if error.to_string().contains("just touches its axis")),
        "{horn:?}"
    );
}
