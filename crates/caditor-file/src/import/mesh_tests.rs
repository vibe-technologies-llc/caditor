use std::{fmt::Write, sync::Arc};

use caditor_document::CancelToken;
use caditor_kernel::{SamplingTolerance, Solid, interruptible};

use crate::{
    export::zip::{ZipEntry, archive},
    import::{
        ImportError, MeshFormat, MeshUnit, ModelImport, UnitBasis, mesh::MAX_MODEL_ELEMENTS,
        parse_mesh, parse_mesh_in, read_mesh_file,
    },
};

const CORNERS: [[f64; 3]; 8] = [
    [0.0, 0.0, 0.0],
    [10.0, 0.0, 0.0],
    [10.0, 10.0, 0.0],
    [0.0, 10.0, 0.0],
    [0.0, 0.0, 10.0],
    [10.0, 0.0, 10.0],
    [10.0, 10.0, 10.0],
    [0.0, 10.0, 10.0],
];

const TRIANGLES: [[usize; 3]; 12] = [
    [0, 3, 2],
    [0, 2, 1],
    [4, 5, 6],
    [4, 6, 7],
    [0, 1, 5],
    [0, 5, 4],
    [1, 2, 6],
    [1, 6, 5],
    [2, 3, 7],
    [2, 7, 6],
    [3, 0, 4],
    [3, 4, 7],
];

fn volume(solid: &Solid) -> f64 {
    solid
        .tessellate(&SamplingTolerance::new(1e-3, 0.05).unwrap())
        .unwrap()
        .mass_properties()
        .volume
}

fn only_volume(imported: &ModelImport) -> f64 {
    let [body] = imported.bodies.as_slice() else {
        panic!("expected one body, found {}", imported.bodies.len());
    };
    assert_eq!(body.import.solid.faces().count(), 6);
    volume(&body.import.solid)
}

pub(crate) fn binary_stl() -> Vec<u8> {
    let mut bytes = vec![0u8; 80];
    bytes.extend(12u32.to_le_bytes());
    for triangle in TRIANGLES {
        bytes.extend([0u8; 12]);
        for corner in triangle {
            for value in CORNERS[corner] {
                bytes.extend((value as f32).to_le_bytes());
            }
        }
        bytes.extend([0u8; 2]);
    }
    bytes
}

#[test]
fn a_cancelled_mesh_read_stops_with_the_import_stopped() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cube.stl");
    std::fs::write(&path, binary_stl()).unwrap();

    assert_eq!(
        read_mesh_file(&path, &CancelToken::new(|| true)).map(|_| ()),
        Err(ImportError::Cancelled)
    );
    assert_eq!(
        read_mesh_file(&path, &CancelToken::never())
            .unwrap()
            .bodies
            .len(),
        1
    );
}

#[test]
fn a_binary_stl_cube_imports_as_one_body_of_six_faces_read_in_millimetres() {
    let imported = parse_mesh(&binary_stl(), MeshFormat::Stl, "cube.stl", "cube").unwrap();

    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
    assert_eq!(imported.bodies[0].name, "cube");
    assert_eq!(imported.bodies[0].import.source, "cube.stl");
    assert!(
        imported
            .notes
            .iter()
            .any(|note| note.contains("read as millimetres"))
    );
}

pub(crate) fn text_stl(skipped: usize) -> String {
    scaled_text_stl(skipped, 1.0)
}

fn scaled_text_stl(skipped: usize, factor: f64) -> String {
    let mut text = "solid cube\n".to_owned();
    for triangle in TRIANGLES.iter().skip(skipped) {
        text.push_str("facet normal 0 0 0\nouter loop\n");
        for corner in triangle {
            let [x, y, z] = CORNERS[*corner].map(|value| value * factor);
            writeln!(text, "vertex {x} {y} {z}").unwrap();
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid cube\n");
    text
}

#[test]
fn a_text_stl_with_a_missing_triangle_is_closed_and_says_so() {
    let text = text_stl(1);

    let imported = parse_mesh(text.as_bytes(), MeshFormat::Stl, "cube.stl", "cube").unwrap();

    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
    assert!(
        imported
            .notes
            .contains(&"1 small hole in the surface was closed.".to_owned())
    );
}

pub(crate) fn cube_obj() -> String {
    let mut text = String::new();
    for [x, y, z] in CORNERS {
        writeln!(text, "v {x} {y} {z}").unwrap();
    }
    for [a, b, c, d] in [
        [1, 4, 3, 2],
        [5, 6, 7, 8],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 4, 8, 7],
        [4, 1, 5, 8],
    ] {
        writeln!(text, "f {a}/1 {b}/1 {} {}", c - 9, d).unwrap();
    }
    text
}

#[test]
fn an_obj_of_quads_with_relative_indices_imports() {
    let text = cube_obj();

    let imported = parse_mesh(text.as_bytes(), MeshFormat::Obj, "cube.obj", "cube").unwrap();

    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
}

pub(crate) fn placed_cube_3mf() -> Vec<u8> {
    let mut model = String::from(
        "<?xml version=\"1.0\"?><model unit=\"centimeter\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\"><resources>\
         <object id=\"1\" type=\"model\"><mesh><vertices>",
    );
    for [x, y, z] in CORNERS {
        write!(
            model,
            "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
            x / 10.0,
            y / 10.0,
            z / 10.0
        )
        .unwrap();
    }
    model.push_str("</vertices><triangles>");
    for [a, b, c] in TRIANGLES {
        write!(model, "<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>").unwrap();
    }
    model.push_str(
        "</triangles></mesh></object><object id=\"2\" type=\"model\"><components>\
         <component objectid=\"1\" transform=\"2 0 0 0 1 0 0 0 1 0 0 0\"/></components></object>\
         </resources><build><item objectid=\"2\" transform=\"1 0 0 0 1 0 0 0 1 5 0 0\"/></build>\
         </model>",
    );
    model_package(&model)
}

#[test]
fn a_3mf_places_its_components_and_converts_its_unit() {
    let package = placed_cube_3mf();

    let imported = parse_mesh(&package, MeshFormat::ThreeMf, "cube.3mf", "cube").unwrap();

    assert!((only_volume(&imported) - 2000.0).abs() < 1e-6);
    let bounds = imported.bodies[0].import.solid.bounding_box().unwrap();
    assert!((bounds.min().x - 50.0).abs() < 1e-9 && (bounds.max().x - 70.0).abs() < 1e-9);
    assert!(imported.notes.is_empty());
}

#[test]
fn damaged_and_open_meshes_are_refused_in_words() {
    assert_eq!(
        parse_mesh(b"solid nothing\nendsolid\n", MeshFormat::Stl, "a.stl", "a"),
        Err(ImportError::DamagedMesh(MeshFormat::Stl))
    );
    assert_eq!(
        parse_mesh(b"not a zip", MeshFormat::ThreeMf, "a.3mf", "a"),
        Err(ImportError::DamagedMesh(MeshFormat::ThreeMf))
    );
    let sheet = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    assert!(matches!(
        parse_mesh(sheet.as_bytes(), MeshFormat::Obj, "a.obj", "a"),
        Err(ImportError::MeshNotClosed { .. })
    ));
}

fn model_package(model: &str) -> Vec<u8> {
    archive(&[ZipEntry {
        name: "3D/3dmodel.model",
        contents: model.as_bytes(),
    }])
    .unwrap()
}

#[test]
fn a_3mf_nested_a_hundred_thousand_deep_is_refused_instead_of_overflowing_the_stack() {
    let depth = 100_000;
    let mut model = String::from("<model unit=\"millimeter\"><resources>");
    model.push_str(&"<a>".repeat(depth));
    model.push_str(&"</a>".repeat(depth));
    model.push_str("</resources></model>");

    assert_eq!(
        parse_mesh(
            &model_package(&model),
            MeshFormat::ThreeMf,
            "deep.3mf",
            "deep"
        ),
        Err(ImportError::NestedTooDeep)
    );
}

#[test]
fn a_3mf_with_relationships_nested_a_hundred_thousand_deep_is_refused_instead_of_overflowing_the_stack()
 {
    let depth = 100_000;
    let mut relationships = String::from("<Relationships>");
    relationships.push_str(&"<a>".repeat(depth));
    relationships.push_str(&"</a>".repeat(depth));
    relationships.push_str("</Relationships>");
    let package = archive(&[
        ZipEntry {
            name: "_rels/.rels",
            contents: relationships.as_bytes(),
        },
        ZipEntry {
            name: "3D/3dmodel.model",
            contents: b"<model/>",
        },
    ])
    .unwrap();

    assert_eq!(
        parse_mesh(&package, MeshFormat::ThreeMf, "deep.3mf", "deep"),
        Err(ImportError::DamagedMesh(MeshFormat::ThreeMf))
    );
}

fn component_bomb() -> Vec<u8> {
    let levels = 16;
    let mut model = String::from(
        "<model unit=\"millimeter\"><resources><object id=\"1\"><mesh><vertices>\
         <vertex x=\"0\" y=\"0\" z=\"0\"/><vertex x=\"1\" y=\"0\" z=\"0\"/>\
         <vertex x=\"0\" y=\"1\" z=\"0\"/></vertices><triangles>\
         <triangle v1=\"0\" v2=\"1\" v3=\"2\"/></triangles></mesh></object>",
    );
    for level in 2..=levels + 1 {
        write!(model, "<object id=\"{level}\"><components>").unwrap();
        for _ in 0..4 {
            write!(model, "<component objectid=\"{}\"/>", level - 1).unwrap();
        }
        model.push_str("</components></object>");
    }
    write!(
        model,
        "</resources><build><item objectid=\"{}\"/></build></model>",
        levels + 1
    )
    .unwrap();
    model_package(&model)
}

#[test]
fn a_3mf_whose_components_place_billions_of_objects_is_refused_promptly() {
    assert_eq!(
        parse_mesh(&component_bomb(), MeshFormat::ThreeMf, "bomb.3mf", "bomb"),
        Err(ImportError::ModelPlacesTooMuch)
    );
}

#[test]
fn a_3mf_placing_a_triangle_less_object_a_million_times_is_refused_before_its_vertices_are() {
    let mut model =
        String::from("<model unit=\"millimeter\"><resources><object id=\"1\"><mesh><vertices>");
    for vertex in 0..1000 {
        write!(model, "<vertex x=\"{vertex}\" y=\"0\" z=\"0\"/>").unwrap();
    }
    model.push_str("</vertices><triangles/></mesh></object>");
    for level in 2..=3 {
        write!(model, "<object id=\"{level}\"><components>").unwrap();
        for _ in 0..1000 {
            write!(model, "<component objectid=\"{}\"/>", level - 1).unwrap();
        }
        model.push_str("</components></object>");
    }
    model.push_str("</resources><build><item objectid=\"3\"/></build></model>");

    assert_eq!(
        parse_mesh(
            &model_package(&model),
            MeshFormat::ThreeMf,
            "points.3mf",
            "points"
        ),
        Err(ImportError::ModelPlacesTooMuch)
    );
}

#[test]
fn a_cancelled_token_stops_a_3mf_components_expansion() {
    let stopped = interruptible(Arc::new(|| true), || {
        parse_mesh(&component_bomb(), MeshFormat::ThreeMf, "bomb.3mf", "bomb")
    });

    assert_eq!(stopped, Err(ImportError::Cancelled));
}

const PRODUCTION: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";

fn cube_object(id: u32, side: f64) -> String {
    let mut object = format!("<object id=\"{id}\" type=\"model\"><mesh><vertices>");
    for [x, y, z] in CORNERS {
        let [x, y, z] = [x, y, z].map(|value| value * side / 10.0);
        write!(object, "<vertex x=\"{x}\" y=\"{y}\" z=\"{z}\"/>").unwrap();
    }
    object.push_str("</vertices><triangles>");
    for [a, b, c] in TRIANGLES {
        write!(object, "<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>").unwrap();
    }
    object.push_str("</triangles></mesh></object>");
    object
}

fn production_model(resources: &str, build: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><model unit=\"millimeter\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" \
         xmlns:prod=\"{PRODUCTION}\" requiredextensions=\"prod\"><resources>{resources}\
         </resources><build>{build}</build></model>"
    )
}

fn parts_package(parts: &[(&str, &str)]) -> Vec<u8> {
    let relationships = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships \
                         xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
                         <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel-1\" \
                         Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\
                         </Relationships>";
    let mut entries = vec![ZipEntry {
        name: "_rels/.rels",
        contents: relationships.as_bytes(),
    }];
    entries.extend(parts.iter().map(|&(name, contents)| ZipEntry {
        name,
        contents: contents.as_bytes(),
    }));
    archive(&entries).unwrap()
}

fn read_package(package: &[u8]) -> Result<ModelImport, ImportError> {
    parse_mesh(package, MeshFormat::ThreeMf, "project.3mf", "project")
}

#[test]
fn a_bambu_style_project_places_the_meshes_its_objects_take_from_other_parts() {
    let root = production_model(
        "<object id=\"2\" prod:UUID=\"00000001-61cb-4c03-9d28-80fed5dfa1dc\" type=\"model\">\
         <components><component prod:path=\"/3D/Objects/object_1.model\" objectid=\"1\" \
         prod:UUID=\"00010000-b206-40ff-9872-83e8017abed1\" \
         transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/></components></object>\
         <object id=\"4\" type=\"model\"><components>\
         <component prod:path=\"/3d/OBJECTS/Object_1.model\" objectid=\"1\"/></components>\
         </object>",
        "<item objectid=\"2\" transform=\"1 0 0 0 1 0 0 0 1 100 0 0\" printable=\"1\"/>\
         <item objectid=\"4\" transform=\"1 0 0 0 1 0 0 0 1 -100 0 0\" printable=\"1\"/>",
    );
    let object = production_model(&cube_object(1, 10.0), "");
    let package = parts_package(&[
        ("3D/3dmodel.model", &root),
        ("3D/Objects/object_1.model", &object),
    ]);

    let imported = read_package(&package).unwrap();

    assert_eq!(imported.bodies.len(), 2);
    let mut lowest: Vec<f64> = imported
        .bodies
        .iter()
        .map(|body| body.import.solid.bounding_box().unwrap().min().x)
        .collect();
    lowest.sort_by(f64::total_cmp);
    assert!((lowest[0] + 100.0).abs() < 1e-9 && (lowest[1] - 100.0).abs() < 1e-9);
    for body in &imported.bodies {
        assert!((volume(&body.import.solid) - 1000.0).abs() < 1e-6);
    }
}

#[test]
fn an_object_id_the_root_shares_with_another_part_places_that_parts_object() {
    let root = production_model(
        &format!(
            "{}<object id=\"2\" type=\"model\"><components>\
             <component prod:path=\"/3D/Objects/object_5.model\" objectid=\"1\"/>\
             </components></object>",
            cube_object(1, 1.0)
        ),
        "<item objectid=\"2\"/>",
    );
    let object = production_model(
        &format!(
            "<object id=\"1\" type=\"model\"><components><component objectid=\"5\" \
             transform=\"1 0 0 0 1 0 0 0 1 0 0 20\"/></components></object>{}",
            cube_object(5, 10.0)
        ),
        "",
    );
    let package = parts_package(&[
        ("3D/3dmodel.model", &root),
        ("3D/Objects/object_5.model", &object),
    ]);

    let imported = read_package(&package).unwrap();

    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
    let bounds = imported.bodies[0].import.solid.bounding_box().unwrap();
    assert!((bounds.min().z - 20.0).abs() < 1e-9 && (bounds.max().z - 30.0).abs() < 1e-9);
}

#[test]
fn a_part_in_its_own_unit_is_placed_in_millimetres() {
    let root = production_model(
        "<object id=\"2\" type=\"model\"><components>\
         <component prod:path=\"Objects/object_1.model\" objectid=\"1\" \
         transform=\"1 0 0 0 1 0 0 0 1 5 0 0\"/></components></object>",
        "<item objectid=\"2\"/>",
    );
    let object = production_model(&cube_object(1, 10.0), "")
        .replace("unit=\"millimeter\"", "unit=\"centimeter\"");
    let package = parts_package(&[
        ("3D/3dmodel.model", &root),
        ("3D/Objects/object_1.model", &object),
    ]);

    let imported = read_package(&package).unwrap();

    assert!((only_volume(&imported) - 1_000_000.0).abs() < 1e-3);
    let bounds = imported.bodies[0].import.solid.bounding_box().unwrap();
    assert!((bounds.min().x - 5.0).abs() < 1e-9 && (bounds.max().x - 105.0).abs() < 1e-9);
}

#[test]
fn a_project_taking_objects_from_a_missing_or_outside_part_is_refused_naming_it() {
    let refers_to = |path: &str| {
        let root = production_model(
            &format!(
                "<object id=\"2\" type=\"model\"><components>\
                 <component prod:path=\"{path}\" objectid=\"1\"/></components></object>"
            ),
            "<item objectid=\"2\"/>",
        );
        read_package(&parts_package(&[("3D/3dmodel.model", &root)])).map(|_| ())
    };

    let missing = refers_to("/3D/Objects/object_1.model");
    let outside = refers_to("/3D/../../secret.model");

    assert_eq!(
        missing,
        Err(ImportError::MissingModelPart {
            part: "/3D/Objects/object_1.model".to_owned()
        })
    );
    assert_eq!(
        missing.unwrap_err().to_string(),
        "its 3D model takes objects from the part “/3D/Objects/object_1.model”, which the 3MF \
         package does not hold"
    );
    assert_eq!(
        outside,
        Err(ImportError::ModelPartOutsidePackage {
            part: "/3D/../../secret.model".to_owned()
        })
    );
}

#[test]
fn parts_taking_objects_from_each_other_in_a_loop_are_refused_naming_the_object() {
    let root = production_model(
        "<object id=\"2\" type=\"model\"><components>\
         <component prod:path=\"/3D/Objects/a.model\" objectid=\"1\"/></components></object>",
        "<item objectid=\"2\"/>",
    );
    let a = production_model(
        &format!(
            "{}<object id=\"1\" type=\"model\"><components>\
             <component prod:path=\"/3D/Objects/b.model\" objectid=\"1\"/></components></object>",
            cube_object(3, 10.0)
        ),
        "",
    );
    let b = production_model(
        "<object id=\"1\" type=\"model\"><components>\
         <component prod:path=\"/3D/Objects/a.model\" objectid=\"1\"/></components></object>",
        "",
    );
    let package = parts_package(&[
        ("3D/3dmodel.model", &root),
        ("3D/Objects/a.model", &a),
        ("3D/Objects/b.model", &b),
    ]);

    let refused = read_package(&package).map(|_| ());

    assert_eq!(
        refused,
        Err(ImportError::ComponentCycle {
            object: "1".to_owned(),
            part: "/3D/Objects/a.model".to_owned()
        })
    );
}

#[test]
fn the_parts_of_a_project_share_one_element_limit() {
    let half = MAX_MODEL_ELEMENTS / 2 + 1;
    let filler = "<a/>".repeat(half);
    let root = production_model(
        &format!(
            "{filler}<object id=\"2\" type=\"model\"><components>\
             <component prod:path=\"/3D/Objects/object_1.model\" objectid=\"1\"/>\
             </components></object>"
        ),
        "<item objectid=\"2\"/>",
    );
    let object = production_model(&format!("{filler}{}", cube_object(1, 10.0)), "");
    let package = parts_package(&[
        ("3D/3dmodel.model", &root),
        ("3D/Objects/object_1.model", &object),
    ]);

    assert_eq!(
        read_package(&package).map(|_| ()),
        Err(ImportError::ModelTooManyElements)
    );
}

#[test]
fn a_model_nested_past_the_xml_limit_is_refused() {
    let nested = format!("{}{}", "<a>".repeat(2_000), "</a>".repeat(2_000));
    let root = production_model(
        &format!("{nested}{}", cube_object(1, 10.0)),
        "<item objectid=\"1\"/>",
    );
    let package = parts_package(&[("3D/3dmodel.model", &root)]);

    assert_eq!(
        read_package(&package).map(|_| ()),
        Err(ImportError::NestedTooDeep)
    );
}

#[test]
fn a_model_element_with_thousands_of_attributes_is_refused() {
    let attributes: String = (0..3_000).map(|n| format!(" a{n}=\"1\"")).collect();
    let root = production_model(&format!("<a{attributes}/>"), "");
    let package = parts_package(&[("3D/3dmodel.model", &root)]);

    assert_eq!(
        read_package(&package).map(|_| ()),
        Err(ImportError::TooManyAttributes)
    );
}

fn scaled_obj(factor: f64) -> String {
    let mut text = String::new();
    for corner in CORNERS {
        let [x, y, z] = corner.map(|value| value * factor);
        writeln!(text, "v {x} {y} {z}").unwrap();
    }
    for [a, b, c] in TRIANGLES {
        writeln!(text, "f {} {} {}", a + 1, b + 1, c + 1).unwrap();
    }
    text
}

fn read_stl_in(factor: f64, chosen: Option<MeshUnit>) -> ModelImport {
    let text = scaled_text_stl(0, factor);
    parse_mesh_in(text.as_bytes(), MeshFormat::Stl, "cube.stl", "cube", chosen).unwrap()
}

#[test]
fn the_unit_guess_follows_the_longest_side_and_stays_silent_for_a_plausible_millimetre_size() {
    assert_eq!(MeshUnit::guess(0.05), Some(MeshUnit::Metres));
    assert_eq!(MeshUnit::guess(0.0001), Some(MeshUnit::Metres));
    assert_eq!(MeshUnit::guess(0.999), Some(MeshUnit::Metres));
    assert_eq!(MeshUnit::guess(50_000.0), Some(MeshUnit::Micrometres));
    assert_eq!(MeshUnit::guess(10_000.5), Some(MeshUnit::Micrometres));

    assert_eq!(MeshUnit::guess(1.0), None);
    assert_eq!(MeshUnit::guess(10.0), None);
    assert_eq!(MeshUnit::guess(10_000.0), None);
    assert_eq!(MeshUnit::guess(0.00001), None);
    assert_eq!(MeshUnit::guess(0.0), None);
    assert_eq!(MeshUnit::guess(f64::NAN), None);
    assert_eq!(MeshUnit::guess(f64::INFINITY), None);
}

#[test]
fn a_part_a_hundredth_across_is_read_as_metres_and_the_report_says_so() {
    let imported = read_stl_in(0.001, None);

    let reading = imported.unit.unwrap();
    assert_eq!(reading.unit, MeshUnit::Metres);
    assert_eq!(reading.basis, UnitBasis::Guessed);
    assert!(reading.is_guess());
    assert!((reading.size.max_element() - 0.01).abs() < 1e-12);
    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
    assert!(imported.notes.iter().any(|note| {
        note.contains("read as metres")
            && note.contains("would be 0.01 mm")
            && note.contains("10 mm long")
    }));
}

#[test]
fn a_part_of_plausible_size_is_assumed_to_be_in_millimetres_without_a_guess() {
    let imported = read_stl_in(1.0, None);

    let reading = imported.unit.unwrap();
    assert_eq!(reading.unit, MeshUnit::Millimetres);
    assert_eq!(reading.basis, UnitBasis::Assumed);
    assert!(!reading.is_guess());
    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
    assert!(
        imported
            .notes
            .iter()
            .any(|note| note.contains("read as millimetres"))
    );
}

#[test]
fn a_huge_number_of_units_is_guessed_to_be_micrometres() {
    let imported = read_stl_in(2_000.0, None);

    let reading = imported.unit.unwrap();
    assert_eq!(reading.unit, MeshUnit::Micrometres);
    assert!((only_volume(&imported) - 8_000.0).abs() < 1e-3);
}

#[test]
fn a_chosen_unit_overrides_the_guess_and_the_report_says_it_was_chosen() {
    let imported = read_stl_in(0.05, Some(MeshUnit::Centimetres));

    let reading = imported.unit.unwrap();
    assert_eq!(reading.unit, MeshUnit::Centimetres);
    assert_eq!(reading.basis, UnitBasis::Chosen);
    assert!(!reading.is_guess());
    assert!((only_volume(&imported) - 125.0).abs() < 1e-6);
    assert!(
        imported
            .notes
            .iter()
            .any(|note| note.contains("read as centimetres, as you chose"))
    );
}

#[test]
fn every_offered_unit_scales_the_part_by_its_size_in_millimetres() {
    for unit in MeshUnit::ALL {
        let imported = read_stl_in(1.0, Some(unit));

        let cubed = unit.millimetres().powi(3);
        assert!((only_volume(&imported) - 1000.0 * cubed).abs() < 1e-6 * cubed.max(1.0));
    }
}

#[test]
fn an_obj_is_guessed_like_an_stl() {
    let text = scaled_obj(0.001);

    let imported = parse_mesh(text.as_bytes(), MeshFormat::Obj, "cube.obj", "cube").unwrap();

    assert_eq!(imported.unit.unwrap().unit, MeshUnit::Metres);
    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
}

#[test]
fn a_3mf_declares_its_units_so_it_is_neither_guessed_nor_offered_a_scale() {
    let package = parts_package(&[(
        "3D/3dmodel.model",
        &production_model(&cube_object(1, 0.5), "<item objectid=\"1\"/>"),
    )]);

    let imported = read_package(&package).unwrap();

    assert_eq!(imported.unit, None);
    assert!(imported.notes.iter().all(|note| !note.contains("no units")));
}
