use std::time::SystemTime;

use crate::{
    fixtures,
    read::{Held, ReadError, read_step},
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

fn assert_same_shape(name: &str, original: &caditor_kernel::Solid, read: &caditor_kernel::Solid) {
    let coarse = caditor_kernel::SamplingTolerance::new(0.05, 0.3).unwrap();
    let mesh = read.tessellate(&coarse).unwrap();
    let before = fixtures::mesh_volume(&original.tessellate(&coarse).unwrap());
    let after = fixtures::mesh_volume(&mesh);
    let surfaces: Vec<&caditor_kernel::Surface> =
        original.faces().map(|(_, face)| face.surface()).collect();

    assert_eq!(read.faces().count(), original.faces().count(), "{name}");
    assert_eq!(read.edges().count(), original.edges().count(), "{name}");
    assert_eq!(
        read.vertices().count(),
        original.vertices().count(),
        "{name}"
    );
    assert_eq!(read.shells().count(), original.shells().count(), "{name}");
    assert!(
        (before - after).abs() < 1e-5 * before,
        "{name}: {before} vs {after}"
    );
    for (index, face) in mesh.faces().iter().enumerate() {
        let surface = surfaces[index];
        let corners: std::collections::BTreeSet<u32> = mesh.triangles()[face.triangles.clone()]
            .iter()
            .flat_map(|triangle| mesh.triangle_positions(*triangle).unwrap())
            .collect();
        let mut hint = None;
        for corner in corners {
            let point = mesh.position(corner).unwrap();
            let gap_from = |foot| surface.point_at(foot).distance(point);
            let hinted = hint.map(|hint| surface.project(point, Some(hint)));
            let foot = match hinted {
                Some(foot) if gap_from(foot) <= caditor_kernel::LINEAR_RESOLUTION => foot,
                _ => surface.project(point, None),
            };
            hint = Some(foot);
            let gap = gap_from(foot);
            assert!(
                gap <= caditor_kernel::LINEAR_RESOLUTION,
                "{name}: face {index} is {gap} mm off the original at {point}"
            );
        }
    }
}

#[test]
fn every_fixture_survives_a_round_trip() {
    for (name, solid) in fixtures::all() {
        let read = round_trip(name, &solid);
        assert_same_shape(name, &solid, &read);
    }
}

#[test]
fn files_without_solids_are_refused_in_words() {
    assert_eq!(read_step("solid cube"), Err(ReadError::NotStep));
    let empty = "ISO-10303-21;HEADER;ENDSEC;DATA;#1=CARTESIAN_POINT('',(0.,0.,0.));ENDSEC;END-ISO-10303-21;";
    assert_eq!(read_step(empty), Err(ReadError::NoSolids(Held::default())));
    assert_eq!(
        read_step("ISO-10303-21;\nDATA;\n#1=X(;"),
        Err(ReadError::Damaged(3))
    );
}

#[test]
fn a_file_with_a_damaged_header_and_no_closing_line_still_reads_its_solids() {
    let solid = fixtures::plate_with_hole();
    let written = write_step(
        &[StepBody {
            name: "plate",
            solid: &solid,
        }],
        "plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let (header, data) = written.split_once("DATA;").unwrap();
    let damaged = format!(
        "{}\nFILE_NAME('broken'\nENDSEC;\nDATA;{}",
        header.split_once("HEADER;").unwrap().0.to_owned() + "HEADER;",
        data.trim_end().trim_end_matches("END-ISO-10303-21;")
    );

    let model = read_step(&damaged).unwrap();

    assert_eq!(model.solids.len(), 1);
    assert_eq!(model.notes.len(), 2, "{:?}", model.notes);
    assert!(model.notes.iter().any(|note| note.contains("header")));
    assert!(model.notes.iter().any(|note| note.contains("closing line")));
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
fn assembly_parts_placed_by_scaling_or_mirroring_operators_are_scaled_or_mirrored() {
    let original = sample(include_str!("samples/assembly.step"));
    let placed_by = |operator: &str| {
        let text = include_str!("samples/assembly.step")
            .replace(
                "#48 = ITEM_DEFINED_TRANSFORMATION('','',#11,#15);",
                operator,
            )
            .replace(
                "ENDSEC;\nEND-ISO-10303-21;",
                "#9100=DIRECTION('',(1.,0.,0.));\nENDSEC;\nEND-ISO-10303-21;",
            );
        assert!(text.contains("#9100="));
        sample(&text)
    };
    let near = |a: caditor_geometry::Point3, b: [f64; 3]| {
        a.distance(caditor_geometry::Point3::from_array(b)) < 1e-9
    };
    let block_volume = fixtures::volume(&original.solids[0].solid);

    let doubled = placed_by("#48 = CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',#18,$,#16,2.,#17);");
    let mirrored =
        placed_by("#48 = CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',#18,#9100,#16,1.,#17);");

    for model in [&doubled, &mirrored] {
        assert!(model.notes.is_empty(), "{:?}", model.notes);
        assert_eq!(model.solids.len(), 2);
        for solid in &model.solids {
            assert_eq!(solid.solid.validate(), Ok(()), "{}", solid.name);
        }
    }
    let block = |model: &crate::read::StepModel| model.solids[0].solid.bounding_box().unwrap();
    assert!(
        near(block(&doubled).min(), [60.0, 0.0, 0.0]),
        "{:?}",
        block(&doubled)
    );
    assert!(
        near(block(&doubled).max(), [100.0, 20.0, 10.0]),
        "{:?}",
        block(&doubled)
    );
    assert_volume(&doubled.solids[0].solid, 8.0 * block_volume);
    assert!(
        near(block(&mirrored).min(), [100.0, 0.0, 0.0]),
        "{:?}",
        block(&mirrored)
    );
    assert!(
        near(block(&mirrored).max(), [120.0, 10.0, 5.0]),
        "{:?}",
        block(&mirrored)
    );
    assert_volume(&mirrored.solids[0].solid, block_volume);
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
        Err(ReadError::NoSolids(Held {
            surface_bodies: 1,
            ..Held::default()
        }))
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
}

#[test]
fn horn_tori_are_read_as_the_whole_tube_turned_about_the_point_it_touches() {
    use std::f64::consts::PI;

    let radius = 3.0;

    let horn = sample(&spindle(radius, radius, true));
    let solid = &horn.solids[0].solid;
    let read = round_trip("horn", solid);
    let inside = read_step(&spindle(radius, radius, false));

    assert_volume(solid, 2.0 * PI * PI * radius.powi(3));
    assert_eq!(solid.faces().count(), 1);
    assert_eq!(solid.vertices().count(), 1);
    assert_same_shape("horn", solid, &read);
    assert!(
        matches!(&inside, Err(error) if error.to_string().contains("holds no volume")),
        "{inside:?}"
    );
}

fn offset_of(
    solid: &caditor_kernel::Solid,
    surface: &str,
    distance: f64,
    radii: (&str, &str),
) -> String {
    let text = write_step(
        &[StepBody {
            name: "Offset",
            solid,
        }],
        "Offset",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
    .replacen(
        &format!("{surface}('',"),
        &format!("OFFSET_SURFACE('',#9100,{distance:?},.F.);\n#9100={surface}('',"),
        1,
    );
    text.lines()
        .map(|line| match line.strip_prefix("#9100=") {
            Some(basis) => {
                assert!(basis.ends_with(&format!(",{});", radii.0)), "{basis}");
                line.replace(&format!(",{});", radii.0), &format!(",{});", radii.1))
            }
            None => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn faces_on_offsets_of_elementary_surfaces_are_imported_in_place() {
    let turned = fixtures::turned();
    let plate = fixtures::plate_with_hole();

    let torus = sample(&offset_of(
        &turned,
        "TOROIDAL_SURFACE",
        2.0,
        ("12.0,5.0", "12.0,3.0"),
    ));
    let cylinder = sample(&offset_of(
        &plate,
        "CYLINDRICAL_SURFACE",
        -2.0,
        ("5.0", "7.0"),
    ));

    assert_same_shape("torus", &turned, &torus.solids[0].solid);
    assert_same_shape("cylinder", &plate, &cylinder.solids[0].solid);
}

#[test]
fn a_face_on_an_offset_that_leaves_no_surface_is_refused_naming_the_offset() {
    let plate = fixtures::plate_with_hole();
    let collapsed = offset_of(&plate, "CYLINDRICAL_SURFACE", -6.0, ("5.0", "5.0"));
    let offset = collapsed
        .lines()
        .find_map(|line| line.split_once("=OFFSET_SURFACE"))
        .map(|(id, _)| id.to_owned())
        .unwrap();

    let refusal = read_step(&collapsed).unwrap_err().to_string();

    assert!(
        refusal.contains(&format!(
            "{offset} is offset by more than the radius of its cylinder, which leaves no surface"
        )),
        "{refusal}"
    );
}

fn with_precision(text: &str, representation: &str, brep: u64, precision: &str) -> String {
    text.replace(
        "ENDSEC;\nEND-ISO-10303-21;",
        &format!(
            "#9020={representation}('',(#{brep}),#9021);\n\
             #9021=(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#9022)) \
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#9023)) REPRESENTATION_CONTEXT('',''));\n\
             #9022=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE({precision}),#9023,\
             'distance_accuracy_value','');\n\
             #9023=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));\n\
             ENDSEC;\nEND-ISO-10303-21;"
        ),
    )
}

#[test]
fn a_tube_that_touches_its_axis_within_the_precision_of_the_file_makes_a_horn_torus() {
    let nearly = spindle(3.0, 3.0 + 1e-7, true);
    let coarse = with_precision(&nearly, "ADVANCED_BREP_SHAPE_REPRESENTATION", 12, "1.E-3");

    let spindle = sample(&nearly);
    let horn = sample(&coarse);

    assert_eq!(spindle.solids[0].solid.vertices().count(), 2);
    assert_eq!(horn.solids[0].solid.vertices().count(), 1);
}

#[test]
fn only_repairs_beyond_the_precision_of_the_file_are_reported() {
    let solid = fixtures::plate_with_hole();
    let text = write_step(
        &[StepBody {
            name: "Plate",
            solid: &solid,
        }],
        "Plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let corner = text
        .lines()
        .find_map(|line| line.strip_suffix("=CARTESIAN_POINT('',(0.0,0.0,0.0));"))
        .unwrap();
    let vertex = format!("VERTEX_POINT('',{corner})");
    let lifted = text
        .replacen(&vertex, "VERTEX_POINT('',#90000)", 1)
        .replacen(
            "ENDSEC;\nEND-ISO",
            "#90000=CARTESIAN_POINT('',(0.0,0.0,0.0003));\nENDSEC;\nEND-ISO",
            1,
        );
    let coarse = lifted.replacen("LENGTH_MEASURE(1.E-6)", "LENGTH_MEASURE(1.E-3)", 1);

    let fine = sample(&lifted);
    let coarse = sample(&coarse);

    assert!(text.contains(&vertex));
    assert_eq!(
        fine.notes,
        ["1 edge or corner that did not quite meet its faces was moved onto its faces."]
    );
    assert!(coarse.notes.is_empty(), "{:?}", coarse.notes);
    assert_volume(&coarse.solids[0].solid, fixtures::volume(&solid));
}

#[test]
fn faces_that_meet_only_as_closely_as_the_file_declares_are_refused_with_its_precision() {
    let bent = faceted_cube("FACETED_BREP('cube',#40)", false)
        .replace("(10.0,10.0,10.0)", "(10.0,10.0,10.0005)");
    let declared = with_precision(&bent, "FACETED_BREP_SHAPE_REPRESENTATION", 41, "1.E-3");

    let refusal = |text: &str| read_step(text).unwrap_err().to_string();
    let (plain, precise) = (refusal(&bent), refusal(&declared));

    assert!(bent.contains("(10.0,10.0,10.0005)"));
    assert!(plain.contains("meet only within"), "{plain}");
    assert!(!plain.contains("precision"), "{plain}");
    assert!(
        precise.contains("which the file's precision of 0.001 mm allows"),
        "{precise}"
    );
}

fn bulged_vase() -> caditor_kernel::Solid {
    use caditor_geometry::{Plane, Point2, Vector2};
    use caditor_kernel::{AngularExtent, Axis2, Profile, ProfileCurve, Selection, revolve};

    let curves = vec![
        ProfileCurve::spline(
            1,
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![
                Point2::new(2.0, 0.0),
                Point2::new(6.0, 4.0),
                Point2::new(6.0, 8.0),
                Point2::new(2.0, 12.0),
            ],
        ),
        ProfileCurve::line(2, Point2::new(2.0, 12.0), Point2::new(0.0, 12.0)),
        ProfileCurve::line(3, Point2::new(0.0, 12.0), Point2::new(0.0, 0.0)),
        ProfileCurve::line(4, Point2::new(0.0, 0.0), Point2::new(2.0, 0.0)),
    ];
    let regions = Profile::new(&curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    let axis = Axis2::new(Point2::ZERO, Vector2::Y).unwrap();
    revolve(&Plane::XZ, &regions, axis, AngularExtent::full(), 1).unwrap()
}

#[test]
fn a_surface_of_revolution_whose_profile_is_not_in_a_meridian_plane_is_refused() {
    let vase = bulged_vase();
    let text = write_step(
        &[StepBody {
            name: "Vase",
            solid: &vase,
        }],
        "Vase",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    assert!(text.contains("SURFACE_OF_REVOLUTION"));
    assert_same_shape("vase", &vase, &round_trip("Vase", &vase));

    let placement = text
        .lines()
        .find(|line| line.contains("AXIS1_PLACEMENT"))
        .unwrap();
    let direction_id = placement
        .trim_end_matches(");")
        .rsplit('#')
        .next()
        .unwrap()
        .to_owned();
    let direction_line = text
        .lines()
        .find(|line| line.starts_with(&format!("#{direction_id}=DIRECTION")))
        .unwrap();
    let skewed = text.replace(
        direction_line,
        &format!("#{direction_id}=DIRECTION('',(0.0,0.6,0.8));"),
    );

    let refusal = read_step(&skewed).unwrap_err().to_string();

    assert!(refusal.contains("plane through its axis"), "{refusal}");
}

#[test]
fn a_file_without_solids_says_its_schema_and_what_it_holds() {
    let building = "ISO-10303-21;HEADER;FILE_SCHEMA(('IFC4'));ENDSEC;DATA;\
        #1=TESSELLATED_SHELL('a',(),.F.);#2=TESSELLATED_SHELL('b',(),.F.);\
        #3=GEOMETRIC_CURVE_SET('c',());ENDSEC;END-ISO-10303-21;";

    let refused = read_step(building).unwrap_err();

    assert_eq!(
        refused,
        ReadError::NoSolids(Held {
            schema: Some("IFC4".to_owned()),
            surface_bodies: 0,
            tessellated_shapes: 2,
            wireframes: 1,
        })
    );
    assert_eq!(
        refused.to_string(),
        "it holds no solid bodies (its schema is IFC4 and it holds 2 tessellated shapes, 1 \
         wireframe); caditor imports closed solids only"
    );
}

#[test]
fn a_file_whose_only_solid_cannot_be_rebuilt_names_it_the_entity_and_the_reason() {
    let bent = faceted_cube("FACETED_BREP('cube',#40)", false)
        .replace("(10.0,10.0,10.0)", "(10.0,10.0,10.0005)");

    let error = read_step(&bent).unwrap_err();

    let ReadError::NotRebuilt {
        name,
        entity,
        reason,
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(name, "cube");
    assert!(*entity > 0);
    assert!(reason.contains("meet only within"), "{reason}");
    assert_eq!(
        error.to_string(),
        format!("“cube” could not be rebuilt, because its entity #{entity} {reason}")
    );
}

#[test]
fn every_misplacement_says_what_was_left_out() {
    use crate::read::Misplacement;

    let notes: Vec<String> = [
        Misplacement::InsideItself,
        Misplacement::Unreadable,
        Misplacement::TooDeep,
        Misplacement::SomeCopiesUnreadable,
        Misplacement::CopyUnplaceable,
    ]
    .into_iter()
    .map(|misplacement| misplacement.note("Arm"))
    .collect();

    assert!(
        notes
            .iter()
            .all(|note| note.contains("“Arm”") && note.ends_with('.'))
    );
    assert_eq!(
        notes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        5
    );
    let error = ReadError::NotPlaced {
        name: "Arm".to_owned(),
        misplacement: Misplacement::InsideItself,
    };
    assert_eq!(
        error.to_string(),
        "“Arm” was left out, because the assembly places it inside itself"
    );
}
