use std::time::SystemTime;

use crate::{
    fixtures,
    read::{Held, ReadError, StepSolid, read_step},
    write::{StepBody, write_step},
};

fn round_trip(name: &str, solid: &caditor_kernel::Solid) -> caditor_kernel::Solid {
    let text = write_step(
        &[StepBody {
            name,
            solid,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        name,
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
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
        Err(ReadError::NoSolids(Held::default()))
    );
}

#[test]
fn a_file_cut_short_inside_its_data_reads_the_solids_before_the_cut() {
    let solid = fixtures::plate_with_hole();
    let written = write_step(
        &[StepBody {
            name: "plate",
            solid: &solid,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let (data, _) = written.split_once("ENDSEC;\nEND-ISO-10303-21;").unwrap();
    let interrupted = format!("{data}#999999=CARTESIAN_POINT('',(1.,2.");

    let model = read_step(&interrupted).unwrap();

    assert_eq!(model.solids.len(), 1);
    assert!(
        model.notes.iter().any(|note| note.contains("cut short")),
        "{:?}",
        model.notes
    );
    let halved = written.get(..written.len() / 2).unwrap();
    assert!(matches!(
        read_step(halved),
        Err(ReadError::NoSolids(_) | ReadError::NotRebuilt { .. })
    ));
}

#[test]
fn a_file_with_a_damaged_header_and_no_closing_line_still_reads_its_solids() {
    let solid = fixtures::plate_with_hole();
    let written = write_step(
        &[StepBody {
            name: "plate",
            solid: &solid,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
            },
            StepBody {
                name: "Right",
                solid: &solid,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
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

#[test]
fn coloured_bodies_read_back_as_the_same_solids() {
    let solid = fixtures::plate_with_hole();
    let step = write_step(
        &[
            StepBody {
                name: "Red",
                solid: &solid,
                colour: Some([200, 30, 30]),
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
            },
            StepBody {
                name: "Plain",
                solid: &solid,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
            },
        ],
        "Bracket",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();

    let names: Vec<String> = sample(&step)
        .solids
        .into_iter()
        .map(|solid| solid.name)
        .collect();

    assert_eq!(names, ["Red", "Plain"]);
}

pub(super) fn faceted_cube(top: &str, missing_face: bool) -> String {
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
fn a_torus_whose_tube_passes_its_axis_is_written_as_the_apple_and_read_back() {
    let rimmed = fixtures::spindle_rimmed();
    let text = write_step(
        &[StepBody {
            name: "Rimmed",
            solid: &rimmed,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "Rimmed",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();

    let read = round_trip("rimmed", &rimmed);

    let degenerate = text.matches("DEGENERATE_TOROIDAL_SURFACE(").count();

    assert!(degenerate >= 1, "{text}");
    assert_eq!(text.matches("TOROIDAL_SURFACE(").count(), degenerate);
    assert_eq!(text.matches(",2.0,3.0,.T.)").count(), degenerate, "{text}");
    assert_same_shape("rimmed", &rimmed, &read);
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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

fn entity_using(text: &str, kind: &str, reference: &str) -> String {
    text.lines()
        .find(|line| line.contains(&format!("={kind}(")) && line.contains(&format!("{reference},")))
        .and_then(|line| line.split_once('='))
        .map(|(id, _)| id.to_owned())
        .unwrap()
}

fn offset_entity(text: &str) -> String {
    text.lines()
        .find_map(|line| line.split_once("=OFFSET_SURFACE"))
        .map(|(id, _)| id.to_owned())
        .unwrap()
}

#[test]
fn a_face_on_an_offset_that_leaves_no_surface_is_lost_and_its_hole_closed_with_a_note() {
    let plate = fixtures::plate_with_hole();
    let collapsed = offset_of(&plate, "CYLINDRICAL_SURFACE", -6.0, ("5.0", "5.0"));
    let offset = offset_entity(&collapsed);
    let face = entity_using(&collapsed, "ADVANCED_FACE", &offset);

    let model = sample(&collapsed);

    assert_eq!(model.solids.len(), 1);
    assert_eq!(
        model.notes,
        vec![format!(
            "“Offset” was imported without its face {face}, because its entity {offset} is \
             offset by more than the radius of its cylinder, which leaves no surface; the hole \
             it left was closed with flat facets, so its curved faces are approximated by flat \
             ones."
        )]
    );
    assert_volume(&model.solids[0].solid, 40.0 * 30.0 * 10.0);
}

#[test]
fn a_hollow_with_a_face_that_cannot_be_read_is_left_out_and_the_rest_kept_exact() {
    let ring = fixtures::hollow_ring();
    let collapsed = offset_of(&ring, "TOROIDAL_SURFACE", -6.0, ("20.0,4.0", "20.0,4.0"));
    let offset = offset_entity(&collapsed);
    let face = entity_using(&collapsed, "ADVANCED_FACE", &offset);

    let model = sample(&collapsed);

    assert_eq!(model.solids.len(), 1);
    assert_eq!(
        model.notes,
        vec![format!(
            "“Offset” was imported without 1 hollow of it, holding its face {face} that could \
             not be read, because its entity {offset} is offset by more than the radius of its \
             torus's tube, which leaves no surface."
        )]
    );
    assert_eq!(model.solids[0].solid.shells().count(), 1);
    assert_volume(
        &model.solids[0].solid,
        std::f64::consts::PI * (30.0f64.powi(2) - 10.0f64.powi(2)) * 20.0,
    );
}

fn half_round_prism() -> caditor_kernel::Solid {
    use caditor_geometry::{Plane, Point2};
    use caditor_kernel::ProfileCurve;

    fixtures::swept(
        Plane::XY,
        &[
            ProfileCurve::line(1, Point2::new(-5.0, 0.0), Point2::new(5.0, 0.0)),
            ProfileCurve::arc(
                2,
                Point2::ZERO,
                Point2::new(5.0, 0.0),
                Point2::new(-5.0, 0.0),
            ),
        ],
        10.0,
    )
}

#[test]
fn a_face_on_an_offset_of_an_extruded_spline_arc_is_fitted_within_the_resolution() {
    let prism = half_round_prism();
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let arc = format!(
        "#9101=CARTESIAN_POINT('',(3.,0.,0.));\n#9102=CARTESIAN_POINT('',(3.,3.,0.));\n\
         #9103=CARTESIAN_POINT('',(0.,3.,0.));\n#9104=CARTESIAN_POINT('',(-3.,3.,0.));\n\
         #9105=CARTESIAN_POINT('',(-3.,0.,0.));\n\
         #9110=(BOUNDED_CURVE() B_SPLINE_CURVE(2,(#9101,#9102,#9103,#9104,#9105),\
         .CIRCULAR_ARC.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS((3,2,3),(0.,0.5,1.),.UNSPECIFIED.) \
         CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((1.,{half:?},1.,\
         {half:?},1.)) REPRESENTATION_ITEM(''));\n\
         #9111=DIRECTION('',(0.,0.,1.));\n#9112=VECTOR('',#9111,1.);\n\
         #9100=SURFACE_OF_LINEAR_EXTRUSION('',#9110,#9112);\nENDSEC;\nEND-ISO-10303-21;"
    );
    let text = written("Half round", &prism);
    let cylinder = text
        .lines()
        .find(|line| line.contains("=CYLINDRICAL_SURFACE("))
        .unwrap();
    let id = cylinder.split_once('=').unwrap().0;
    let offset = text
        .replace(cylinder, &format!("{id}=OFFSET_SURFACE('',#9100,2.,.F.);"))
        .replace("ENDSEC;\nEND-ISO-10303-21;", &arc);

    let model = sample(&offset);

    assert!(model.notes.is_empty(), "{:?}", model.notes);
    assert!(matches!(
        model.solids[0]
            .solid
            .faces()
            .find_map(|(_, face)| match face.surface() {
                caditor_kernel::Surface::Extrusion(extrusion) => Some(extrusion.profile().clone()),
                _ => None,
            }),
        Some(caditor_kernel::Curve::BSpline(_))
    ));
    let read = &model.solids[0].solid;
    let surfaces: Vec<&caditor_kernel::Surface> =
        prism.faces().map(|(_, face)| face.surface()).collect();
    let mesh = read
        .tessellate(&caditor_kernel::SamplingTolerance::new(0.05, 0.3).unwrap())
        .unwrap();
    for point in mesh.positions() {
        let gap = surfaces
            .iter()
            .map(|surface| surface.distance(*point))
            .fold(f64::INFINITY, f64::min);
        assert!(
            gap <= caditor_kernel::LINEAR_RESOLUTION,
            "{point} is {gap} mm off"
        );
    }
    assert_eq!(read.faces().count(), prism.faces().count());
    assert_eq!(read.edges().count(), prism.edges().count());
    assert_volume(read, std::f64::consts::PI * 25.0 / 2.0 * 10.0);
}

fn replica_operator(id: u64, origin: [f64; 3], mirrored: bool) -> String {
    let second = if mirrored {
        format!("#{}", id + 2)
    } else {
        "$".to_owned()
    };
    format!(
        "#{}=CARTESIAN_POINT('',({:?},{:?},{:?}));\n#{}=DIRECTION('',(0.,-1.,0.));\n\
         #{id}=CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',$,{second},#{},1.,$);\n",
        id + 1,
        origin[0],
        origin[1],
        origin[2],
        id + 2,
        id + 1,
    )
}

fn line_of<'t>(text: &'t str, kind: &str) -> (&'t str, &'t str) {
    let line = text
        .lines()
        .find(|line| line.contains(&format!("={kind}(")))
        .unwrap();
    (line, line.split_once('=').unwrap().0)
}

fn coordinates(text: &str, id: &str) -> Vec<f64> {
    arguments(text, id)[1..]
        .iter()
        .map(|value| value.parse().unwrap())
        .collect()
}

fn top_of(text: &str) -> (String, String) {
    let top = text
        .lines()
        .filter_map(|line| line.split_once("=PLANE('',"))
        .find(|(_, placement)| {
            let at = arguments(text, placement.trim_end_matches(");"));
            coordinates(text, at[1])[2] == 10.0 && coordinates(text, at[2])[2].abs() == 1.0
        })
        .map(|(id, _)| id.to_owned())
        .unwrap();
    let face = entity_using(text, "ADVANCED_FACE", &top);
    (top, face)
}

#[test]
fn replicas_of_points_curves_and_surfaces_are_their_parents_transformed() {
    let plate = fixtures::plate_with_hole();
    let text = written("Plate", &plate);
    let (cylinder, cylinder_id) = line_of(&text, "CYLINDRICAL_SURFACE");
    let (line, line_id) = line_of(&text, "LINE");
    let (vertex, vertex_id) = line_of(&text, "VERTEX_POINT");
    let corner = coordinates(&text, arguments(&text, vertex_id)[1]);
    let axis = arguments(&text, cylinder_id)[1];
    let [_, centre, along, reference] = arguments(&text, axis)[..] else {
        panic!("{axis}");
    };
    let centre = coordinates(&text, centre);
    let (top, top_face) = top_of(&text);
    let face_line = text
        .lines()
        .find(|line| line.starts_with(&format!("{top_face}=")))
        .unwrap();
    let flipped_face = match face_line.strip_suffix(&format!(",{top},.T.);")) {
        Some(start) => format!("{start},#9500,.F.);"),
        None => face_line.replace(&format!(",{top},.F.);"), ",#9500,.T.);"),
    };
    let mut extra = format!(
        "#9200=CYLINDRICAL_SURFACE('',#9202,5.0);\n\
         #9201=CARTESIAN_POINT('',({:?},{:?},{:?}));\n\
         #9202=AXIS2_PLACEMENT_3D('',#9201,{along},{reference});\n\
         {}\n",
        centre[0] - 3.0,
        centre[1],
        centre[2],
        line.replacen(line_id, "#9300", 1),
    );
    extra.push_str(&replica_operator(9210, [3.0, 0.0, 0.0], false));
    extra.push_str(&replica_operator(9310, [0.0, 0.0, 0.0], false));
    extra.push_str(&replica_operator(9410, [1.0, -2.0, 0.5], false));
    extra.push_str(&replica_operator(9510, [0.0, 0.0, 0.0], true));
    extra.push_str(&format!(
        "#9400=CARTESIAN_POINT('',({:?},{:?},{:?}));\n\
         #9401=POINT_REPLICA('',#9400,#9410);\n\
         #9500=SURFACE_REPLICA('',{top},#9510);\n\
         ENDSEC;\nEND-ISO-10303-21;",
        corner[0] - 1.0,
        corner[1] + 2.0,
        corner[2] - 0.5,
    ));
    let replicated = text
        .replace(
            cylinder,
            &format!("{cylinder_id}=SURFACE_REPLICA('',#9200,#9210);"),
        )
        .replace(line, &format!("{line_id}=CURVE_REPLICA('',#9300,#9310);"))
        .replace(vertex, &format!("{vertex_id}=VERTEX_POINT('',#9401);"))
        .replace(face_line, &flipped_face)
        .replace("ENDSEC;\nEND-ISO-10303-21;", &extra);

    let model = sample(&replicated);

    assert!(replicated.contains(",#9500,."), "{flipped_face}");
    assert!(model.notes.is_empty(), "{:?}", model.notes);
    assert_same_shape("replicas", &plate, &model.solids[0].solid);
}

pub(super) fn with_precision(
    text: &str,
    representation: &str,
    brep: u64,
    precision: &str,
) -> String {
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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
fn faces_that_meet_only_as_closely_as_the_file_declares_import_as_facets() {
    let bent = faceted_cube("FACETED_BREP('cube',#40)", false)
        .replace("(10.0,10.0,10.0)", "(10.0,10.0,10.0005)");
    let declared = with_precision(&bent, "FACETED_BREP_SHAPE_REPRESENTATION", 41, "1.E-3");

    let plain = read_step(&bent).unwrap_err().to_string();
    let model = read_step(&declared).unwrap();
    let tolerance = caditor_kernel::SamplingTolerance::new(0.01, 0.3).unwrap();
    let volume = fixtures::mesh_volume(&model.solids[0].solid.tessellate(&tolerance).unwrap());

    assert!(bent.contains("(10.0,10.0,10.0005)"));
    assert!(plain.contains("meet only within"), "{plain}");
    assert!(!plain.contains("precision"), "{plain}");
    assert_eq!(model.solids.len(), 1);
    assert!(
        model
            .notes
            .iter()
            .any(|note| note.contains("“cube” was imported as flat facets")),
        "{:?}",
        model.notes
    );
    assert!((volume - 1000.0).abs() < 0.01, "{volume}");
}

#[test]
fn a_curved_face_off_its_neighbour_is_continued_to_meet_it_with_a_note_beyond_the_precision() {
    let original = include_str!("samples/loft.step");
    let loose = original
        .replace(
            "#54 = CARTESIAN_POINT('',(10.,1.121997376282,0.));",
            "#54 = CARTESIAN_POINT('',(10.,1.121997376282,3.E-03));",
        )
        .replace("LENGTH_MEASURE(1.E-07)", "LENGTH_MEASURE(1.E-02)");
    let strict = loose.replace("LENGTH_MEASURE(1.E-02)", "LENGTH_MEASURE(1.E-07)");

    let within = sample(&loose);
    let beyond = sample(&strict);

    assert_ne!(loose, original);
    assert!(within.notes.is_empty(), "{:?}", within.notes);
    assert!(
        beyond
            .notes
            .iter()
            .any(|note| note.contains("was moved onto its faces")),
        "{:?}",
        beyond.notes
    );
    for model in [&within, &beyond] {
        assert_eq!(model.solids.len(), 1);
        assert_volume(&model.solids[0].solid, 4494.9012);
    }
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
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
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

#[test]
fn a_colour_written_on_a_body_reads_back_and_uncoloured_bodies_have_none() {
    let plate = fixtures::plate_with_hole();
    let text = write_step(
        &[
            StepBody {
                name: "Red",
                solid: &plate,
                colour: Some([255, 0, 51]),
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
            },
            StepBody {
                name: "Plain",
                solid: &plate,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
            },
        ],
        "model",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();

    let model = read_step(&text).unwrap();

    let colour = |name: &str| {
        model
            .solids
            .iter()
            .find(|solid| solid.name == name)
            .unwrap()
            .colour
    };
    assert_eq!(colour("Red"), Some([255, 0, 51]));
    assert_eq!(colour("Plain"), None);
}

#[test]
fn a_body_whose_faces_all_share_one_colour_takes_it_and_mixed_faces_the_most_shared() {
    let plate = fixtures::plate_with_hole();
    let text = write_step(
        &[StepBody {
            name: "Plate",
            solid: &plate,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "Plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let faces: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_once("=ADVANCED_FACE("))
        .map(|(id, _)| id)
        .collect();
    let styled = |colours: &[&str]| {
        let mut extra = String::from("#900001=DRAUGHTING_PRE_DEFINED_COLOUR('blue');\n");
        extra.push_str("#900002=FILL_AREA_STYLE_COLOUR('',#900001);\n");
        extra.push_str("#900003=FILL_AREA_STYLE('',(#900002));\n");
        extra.push_str("#900004=SURFACE_STYLE_FILL_AREA(#900003);\n");
        extra.push_str("#900005=SURFACE_SIDE_STYLE('',(#900004));\n");
        extra.push_str("#900006=SURFACE_STYLE_USAGE(.BOTH.,#900005);\n");
        extra.push_str("#900007=PRESENTATION_STYLE_ASSIGNMENT((#900006));\n");
        extra.push_str("#900011=COLOUR_RGB('',0.,1.,0.);\n");
        extra.push_str("#900012=FILL_AREA_STYLE_COLOUR('',#900011);\n");
        extra.push_str("#900013=FILL_AREA_STYLE('',(#900012));\n");
        extra.push_str("#900014=SURFACE_STYLE_FILL_AREA(#900013);\n");
        extra.push_str("#900015=SURFACE_SIDE_STYLE('',(#900014));\n");
        extra.push_str("#900016=SURFACE_STYLE_USAGE(.BOTH.,#900015);\n");
        extra.push_str("#900017=PRESENTATION_STYLE_ASSIGNMENT((#900016));\n");
        for (index, face) in faces.iter().enumerate() {
            let assignment = colours
                .get(index % colours.len())
                .copied()
                .unwrap_or("#900007");
            extra.push_str(&format!(
                "#{}=STYLED_ITEM('',({assignment}),{face});\n",
                910_000 + index
            ));
        }
        let mut styled = text.clone();
        let end = styled.rfind("ENDSEC;").unwrap();
        styled.insert_str(end, &extra);
        read_step(&styled).unwrap().solids.remove(0)
    };

    let one = styled(&["#900007"]);
    let mixed = styled(&["#900007", "#900007", "#900017"]);
    let mostly_green = styled(&["#900017", "#900017", "#900007"]);

    assert_eq!((one.colour, one.faces.len()), (Some([0, 0, 255]), 0));
    assert_eq!(mixed.colour, Some([0, 0, 255]));
    let green: Vec<usize> = (0..faces.len()).filter(|face| face % 3 == 2).collect();
    let found: Vec<usize> = mixed.faces.iter().map(|look| look.face).collect();
    assert_eq!(found, green);
    assert!(
        mixed
            .faces
            .iter()
            .all(|look| (look.colour, look.opacity) == (Some([0, 255, 0]), None))
    );
    assert_eq!(mostly_green.colour, Some([0, 255, 0]));
}

fn styled_faces(
    text: &str,
    solid_style: Option<&str>,
    top_style: &str,
    rest: Option<&str>,
) -> StepSolid {
    let (_, top) = top_of(text);
    let faces: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_once("=ADVANCED_FACE("))
        .map(|(id, _)| id)
        .collect();
    let mut extra = String::from(
        "#900001=DRAUGHTING_PRE_DEFINED_COLOUR('blue');\n\
         #900002=FILL_AREA_STYLE_COLOUR('',#900001);\n\
         #900003=FILL_AREA_STYLE('',(#900002));\n\
         #900004=SURFACE_STYLE_FILL_AREA(#900003);\n\
         #900005=SURFACE_SIDE_STYLE('',(#900004));\n\
         #900006=SURFACE_STYLE_USAGE(.BOTH.,#900005);\n\
         #900007=PRESENTATION_STYLE_ASSIGNMENT((#900006));\n\
         #900011=COLOUR_RGB('',1.,0.,0.);\n\
         #900012=SURFACE_STYLE_TRANSPARENT(0.5);\n\
         #900013=SURFACE_STYLE_RENDERING_WITH_PROPERTIES(.NORMAL_SHADING.,#900011,(#900012));\n\
         #900015=SURFACE_SIDE_STYLE('',(#900013));\n\
         #900016=SURFACE_STYLE_USAGE(.BOTH.,#900015);\n\
         #900017=PRESENTATION_STYLE_ASSIGNMENT((#900016));\n\
         #900021=COLOUR_RGB('',0.,1.,0.);\n\
         #900022=FILL_AREA_STYLE_COLOUR('',#900021);\n\
         #900023=FILL_AREA_STYLE('',(#900022));\n\
         #900024=SURFACE_STYLE_FILL_AREA(#900023);\n\
         #900025=SURFACE_SIDE_STYLE('',(#900024));\n\
         #900026=SURFACE_STYLE_USAGE(.BOTH.,#900025);\n\
         #900027=PRESENTATION_STYLE_ASSIGNMENT((#900026));\n",
    );
    if let Some(style) = solid_style {
        let solid = text
            .lines()
            .find_map(|line| line.split_once("=MANIFOLD_SOLID_BREP("))
            .map(|(id, _)| id)
            .unwrap();
        extra.push_str(&format!("#900030=STYLED_ITEM('',({style}),{solid});\n"));
    }
    for (index, face) in faces.iter().enumerate() {
        let style = if *face == top { Some(top_style) } else { rest };
        if let Some(style) = style {
            extra.push_str(&format!(
                "#{}=STYLED_ITEM('',({style}),{face});\n",
                910_000 + index
            ));
        }
    }
    let mut styled = text.to_owned();
    let end = styled.rfind("ENDSEC;").unwrap();
    styled.insert_str(end, &extra);
    read_step(&styled).unwrap().solids.remove(0)
}

fn is_top(solid: &caditor_kernel::Solid, face: usize) -> bool {
    let (_, face) = solid.faces().nth(face).unwrap();
    matches!(face.surface(), caditor_kernel::Surface::Plane(plane)
        if (plane.frame().origin().z - 10.0).abs() < 1e-9)
}

#[test]
fn faces_of_their_own_colour_or_see_through_carry_their_look_onto_the_imported_faces() {
    let plate = fixtures::plate_with_hole();
    let text = written("Plate", &plate);
    let faces = plate.faces().count();

    let two_colours = styled_faces(&text, None, "#900017", Some("#900007"));
    let overridden = styled_faces(&text, Some("#900007"), "#900027", None);
    let shared = styled_faces(&text, None, "#900007", Some("#900007"));

    assert!(faces > 2);
    assert_eq!(
        (two_colours.colour, two_colours.opacity),
        (Some([0, 0, 255]), None)
    );
    let [top] = two_colours.faces.as_slice() else {
        panic!(
            "expected only the top face to differ: {:?}",
            two_colours.faces
        );
    };
    assert!(is_top(&two_colours.solid, top.face));
    assert_eq!((top.colour, top.opacity), (Some([255, 0, 0]), Some(50)));
    assert_eq!(overridden.colour, Some([0, 0, 255]));
    assert_eq!(overridden.faces.len(), 1);
    assert!(is_top(&overridden.solid, overridden.faces[0].face));
    assert_eq!(overridden.faces[0].colour, Some([0, 255, 0]));
    assert_eq!(shared.colour, Some([0, 0, 255]));
    assert!(shared.faces.is_empty());
}

#[test]
fn an_opacity_written_on_a_body_reads_back_and_opaque_bodies_have_none() {
    let plate = fixtures::plate_with_hole();
    let body = |name, opacity| StepBody {
        name,
        solid: &plate,
        colour: Some([20, 40, 60]),
        opacity,
        layer: None,
        threads: &[],
        faces: &[],
    };
    let text = write_step(
        &[
            body("Clear", Some(25)),
            body("Smoked", Some(70)),
            body("Solid", None),
            body("Full", Some(100)),
        ],
        "model",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();

    let model = read_step(&text).unwrap();

    let look = |name: &str| {
        let solid = model
            .solids
            .iter()
            .find(|solid| solid.name == name)
            .unwrap();
        (solid.colour, solid.opacity)
    };
    assert_eq!(look("Clear"), (Some([20, 40, 60]), Some(25)));
    assert_eq!(look("Smoked"), (Some([20, 40, 60]), Some(70)));
    assert_eq!(look("Solid"), (Some([20, 40, 60]), None));
    assert_eq!(look("Full"), (Some([20, 40, 60]), None));
    assert_eq!(text.matches("SURFACE_STYLE_TRANSPARENT(0.75)").count(), 1);
}

fn plate_styled_by(side_style_elements: &str, entities: &str) -> StepSolid {
    plate_styled_model(side_style_elements, entities)
        .solids
        .remove(0)
}

fn plate_styled_model(side_style_elements: &str, entities: &str) -> crate::read::StepModel {
    let plate = fixtures::plate_with_hole();
    let text = write_step(
        &[StepBody {
            name: "Panel",
            solid: &plate,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "Panel",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let solid = text
        .lines()
        .find_map(|line| line.split_once("=MANIFOLD_SOLID_BREP("))
        .map(|(id, _)| id.to_owned())
        .unwrap();
    let extra = format!(
        "{entities}#900010=SURFACE_SIDE_STYLE('',({side_style_elements}));\n\
         #900011=SURFACE_STYLE_USAGE(.BOTH.,#900010);\n\
         #900012=PRESENTATION_STYLE_ASSIGNMENT((#900011));\n\
         #900013=STYLED_ITEM('',(#900012),{solid});\n"
    );
    let mut styled = text.clone();
    let end = styled.rfind("ENDSEC;").unwrap();
    styled.insert_str(end, &extra);
    read_step(&styled).unwrap()
}

const FILL: &str = "#900001=COLOUR_RGB('',1.,0.,0.);\n\
    #900002=FILL_AREA_STYLE_COLOUR('',#900001);\n\
    #900003=FILL_AREA_STYLE('',(#900002));\n\
    #900004=SURFACE_STYLE_FILL_AREA(#900003);\n";

#[test]
fn a_transparent_surface_style_gives_the_body_its_opacity_beside_its_colour() {
    let entities = format!("{FILL}#900005=SURFACE_STYLE_TRANSPARENT(0.6);\n");

    let solid = plate_styled_by("#900004,#900005", &entities);

    assert_eq!(solid.colour, Some([255, 0, 0]));
    assert_eq!(solid.opacity, Some(40));
}

#[test]
fn the_transparency_of_a_rendering_with_properties_gives_the_body_its_opacity() {
    let entities = format!(
        "{FILL}#900005=SURFACE_STYLE_TRANSPARENT(0.75);\n\
         #900006=SURFACE_STYLE_RENDERING_WITH_PROPERTIES(.NORMAL_SHADING.,#900001,(#900005));\n"
    );

    let solid = plate_styled_by("#900006", &entities);

    assert_eq!(solid.colour, Some([255, 0, 0]));
    assert_eq!(solid.opacity, Some(25));
}

#[test]
fn a_transparency_outside_zero_to_one_is_named_as_not_understood_and_none_means_opaque() {
    let transparent = |transparency: &str| {
        let entities = format!("{FILL}#900005=SURFACE_STYLE_TRANSPARENT({transparency});\n");
        let mut model = plate_styled_model("#900004,#900005", &entities);
        (model.solids.remove(0).opacity, model.notes)
    };

    assert_eq!(transparent("0."), (None, Vec::new()));
    assert_eq!(transparent("1."), (Some(0), Vec::new()));
    for outside in ["-3.", "7."] {
        let (opacity, notes) = transparent(outside);
        assert_eq!(opacity, None);
        assert_eq!(
            notes,
            [
                "Some styling in the file could not be understood, so what it styles keeps the \
              look it has without it: #900005, a transparency that is not between 0 and 1."
            ]
        );
    }
    assert_eq!(plate_styled_by("#900004", FILL).opacity, None);
}

#[test]
fn a_layer_written_on_bodies_reads_back_on_each_and_unlayered_bodies_have_none() {
    let plate = fixtures::plate_with_hole();
    let body = |name, layer| StepBody {
        name,
        solid: &plate,
        colour: Some([10, 20, 30]),
        opacity: None,
        layer,
        threads: &[],
        faces: &[],
    };
    let text = write_step(
        &[
            body("Left", Some("Brackets")),
            body("Right", Some("Brackets")),
            body("Loose", None),
        ],
        "model",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();

    let model = read_step(&text).unwrap();

    let layer = |name: &str| {
        model
            .solids
            .iter()
            .find(|solid| solid.name == name)
            .unwrap()
            .layer
            .clone()
    };
    assert_eq!(text.matches("PRESENTATION_LAYER_ASSIGNMENT").count(), 1);
    assert_eq!(layer("Left").as_deref(), Some("Brackets"));
    assert_eq!(layer("Right").as_deref(), Some("Brackets"));
    assert_eq!(layer("Loose"), None);
}

#[test]
fn a_layer_holding_a_styled_item_puts_the_styled_body_on_it() {
    let plate = fixtures::plate_with_hole();
    let text = write_step(
        &[StepBody {
            name: "Plate",
            solid: &plate,
            colour: Some([1, 2, 3]),
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "Plate",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let styled = text
        .lines()
        .find_map(|line| line.split_once("=STYLED_ITEM("))
        .map(|(id, _)| id.to_owned())
        .unwrap();
    let mut layered = text.clone();
    let end = layered.rfind("ENDSEC;").unwrap();
    layered.insert_str(
        end,
        &format!("#900001=PRESENTATION_LAYER_ASSIGNMENT('Parts','',({styled}));\n"),
    );

    let solid = read_step(&layered).unwrap().solids.remove(0);

    assert_eq!(solid.layer.as_deref(), Some("Parts"));
    assert_eq!(solid.colour, Some([1, 2, 3]));
}

fn written(name: &str, solid: &caditor_kernel::Solid) -> String {
    write_step(
        &[StepBody {
            name,
            solid,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        name,
        SystemTime::UNIX_EPOCH,
    )
    .unwrap()
}

fn arguments<'t>(text: &'t str, id: &str) -> Vec<&'t str> {
    let line = text
        .lines()
        .find(|line| line.starts_with(&format!("{id}=")))
        .unwrap();
    let inside = &line[line.find('(').unwrap() + 1..line.rfind(')').unwrap()];
    inside
        .split([',', '(', ')'])
        .filter(|argument| !argument.is_empty())
        .collect()
}

#[test]
fn an_edge_a_loop_runs_out_along_and_straight_back_is_left_out() {
    let solid = fixtures::plate_with_hole();
    let text = written("plate", &solid);

    let edge_loop = text
        .lines()
        .find(|line| line.contains("=EDGE_LOOP('',("))
        .unwrap();
    let loop_id = &edge_loop[..edge_loop.find('=').unwrap()];
    let first_use = arguments(&text, loop_id)[1];
    let used = arguments(&text, first_use);
    let edge = arguments(&text, used[3]);
    let start = if used[4] == ".T." { edge[1] } else { edge[2] };
    let start_point = arguments(&text, arguments(&text, start)[1]);
    let corner: Vec<f64> = start_point[1..]
        .iter()
        .map(|value| value.parse().unwrap())
        .collect();
    let fin = format!(
        "#9001=CARTESIAN_POINT('',({:?},{:?},{:?}));\n\
         #9002=VERTEX_POINT('',#9001);\n\
         #9003=CARTESIAN_POINT('',({:?},{:?},{:?}));\n\
         #9004=DIRECTION('',(0.,0.6,0.8));#9005=VECTOR('',#9004,1.);\n\
         #9006=LINE('',#9003,#9005);\n\
         #9007=EDGE_CURVE('',{start},#9002,#9006,.T.);\n\
         #9008=ORIENTED_EDGE('',*,*,#9007,.T.);#9009=ORIENTED_EDGE('',*,*,#9007,.F.);\n\
         ENDSEC;\nEND-ISO-10303-21;",
        corner[0] + 0.002,
        corner[1] + 3.0,
        corner[2] + 4.0,
        corner[0] - 0.002,
        corner[1],
        corner[2],
    );
    let with_fin = text
        .replace(
            &format!("{loop_id}=EDGE_LOOP('',({first_use},"),
            &format!("{loop_id}=EDGE_LOOP('',(#9008,#9009,{first_use},"),
        )
        .replace("ENDSEC;\nEND-ISO-10303-21;", &fin);
    let model = sample(&with_fin);

    assert_ne!(with_fin, text);
    assert!(model.notes.is_empty(), "{:?}", model.notes);
    assert_eq!(model.solids.len(), 1);
    assert_eq!(model.solids[0].solid.edges().count(), solid.edges().count());
    assert_volume(&model.solids[0].solid, fixtures::volume(&solid));
}

#[derive(Default)]
struct Entities {
    lines: Vec<String>,
}

impl Entities {
    fn add(&mut self, entity: String) -> usize {
        self.lines.push(entity);
        self.lines.len()
    }

    fn point(&mut self, point: caditor_geometry::Point3) -> usize {
        self.add(format!(
            "CARTESIAN_POINT('',({:?},{:?},{:?}))",
            point.x, point.y, point.z
        ))
    }

    fn direction(&mut self, direction: caditor_geometry::Vector3) -> usize {
        self.add(format!(
            "DIRECTION('',({:?},{:?},{:?}))",
            direction.x, direction.y, direction.z
        ))
    }

    fn text(&self) -> String {
        let data: Vec<String> = self
            .lines
            .iter()
            .enumerate()
            .map(|(index, entity)| format!("#{}={entity};", index + 1))
            .collect();
        format!(
            "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n{}\nENDSEC;\nEND-ISO-10303-21;\n",
            data.join("\n")
        )
    }
}

fn box_with_top(top: impl Fn(&mut Entities) -> usize) -> String {
    const LOOPS: [[usize; 4]; 6] = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let corner = |index: usize| {
        let pick = |bit: usize| if index & bit == 0 { 0.0 } else { 10.0 };
        caditor_geometry::Point3::new(pick(1), pick(2), pick(4))
    };
    let mut entities = Entities::default();
    let vertices: Vec<usize> = (0..8)
        .map(|index| {
            let at = entities.point(corner(index));
            entities.add(format!("VERTEX_POINT('',#{at})"))
        })
        .collect();
    let mut edges = std::collections::BTreeMap::new();
    let mut faces = Vec::new();
    for (face, corners) in LOOPS.iter().enumerate() {
        let mut uses = Vec::new();
        for (index, from) in corners.iter().enumerate() {
            let to = corners[(index + 1) % 4];
            let key = (*from.min(&to), *from.max(&to));
            let edge = *edges.entry(key).or_insert_with(|| {
                let (start, end) = (corner(key.0), corner(key.1));
                let origin = entities.point(start);
                let along = entities.direction((end - start).normalize());
                let vector = entities.add(format!("VECTOR('',#{along},1.)"));
                let line = entities.add(format!("LINE('',#{origin},#{vector})"));
                entities.add(format!(
                    "EDGE_CURVE('',#{},#{},#{line},.T.)",
                    vertices[key.0], vertices[key.1]
                ))
            });
            let forward = if *from < to { ".T." } else { ".F." };
            uses.push(format!(
                "#{}",
                entities.add(format!("ORIENTED_EDGE('',*,*,#{edge},{forward})"))
            ));
        }
        let edge_loop = entities.add(format!("EDGE_LOOP('',({}))", uses.join(",")));
        let bound = entities.add(format!("FACE_OUTER_BOUND('',#{edge_loop},.T.)"));
        let points: Vec<_> = corners.iter().map(|index| corner(*index)).collect();
        let surface = if face == 1 {
            top(&mut entities)
        } else {
            let origin = entities.point(points[0]);
            let axis = entities.direction(
                (points[1] - points[0])
                    .cross(points[2] - points[1])
                    .normalize(),
            );
            let reference = entities.direction((points[1] - points[0]).normalize());
            let placement = entities.add(format!(
                "AXIS2_PLACEMENT_3D('',#{origin},#{axis},#{reference})"
            ));
            entities.add(format!("PLANE('',#{placement})"))
        };
        faces.push(format!(
            "#{}",
            entities.add(format!("ADVANCED_FACE('',(#{bound}),#{surface},.T.)"))
        ));
    }
    let shell = entities.add(format!("CLOSED_SHELL('',({}))", faces.join(",")));
    entities.add(format!("MANIFOLD_SOLID_BREP('box',#{shell})"));
    entities.text()
}

#[test]
fn a_spline_face_ending_just_short_of_its_neighbours_is_continued_to_meet_them() {
    let short_by = 1e-4;
    let text = box_with_top(|entities| {
        let net: Vec<String> = [0.0, 10.0 - short_by]
            .into_iter()
            .map(|x| {
                let row: Vec<String> = [0.0, 10.0]
                    .into_iter()
                    .map(|y| {
                        format!(
                            "#{}",
                            entities.point(caditor_geometry::Point3::new(x, y, 10.0))
                        )
                    })
                    .collect();
                format!("({})", row.join(","))
            })
            .collect();
        entities.add(format!(
            "B_SPLINE_SURFACE_WITH_KNOTS('',1,1,({}),.UNSPECIFIED.,.F.,.F.,.F.,(2,2),(2,2),\
             (0.,1.),(0.,1.),.UNSPECIFIED.)",
            net.join(",")
        ))
    });

    let model = sample(&text);

    assert_eq!(model.solids.len(), 1);
    assert_volume(&model.solids[0].solid, 1000.0);
}

#[test]
fn every_file_of_a_corpus_is_read_and_reported() {
    let Some(directory) = std::env::var_os("STEP_CORPUS") else {
        return;
    };
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    let only = std::env::var("STEP_CORPUS_ONLY").unwrap_or_default();
    for path in paths {
        if !path.to_string_lossy().contains(&only) {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let start = std::time::Instant::now();
        let result = read_step(&text);
        let elapsed = start.elapsed().as_secs_f64();
        match result {
            Ok(model) => {
                let mut volumes: Vec<f64> = model
                    .solids
                    .iter()
                    .map(|solid| fixtures::volume(&solid.solid))
                    .collect();
                volumes.sort_by(|a, b| b.total_cmp(a));
                println!(
                    "OK {} solids={} {:.2}s total={:.4} {:?}",
                    path.display(),
                    model.solids.len(),
                    elapsed,
                    volumes.iter().sum::<f64>(),
                    volumes
                        .iter()
                        .take(12)
                        .map(|volume| (volume * 1e4).round() / 1e4)
                        .collect::<Vec<_>>()
                );
                for note in &model.notes {
                    println!("    note: {note}");
                }
            }
            Err(error) => println!("ERR {} {:.2}s: {error}", path.display(), elapsed),
        }
    }
}
