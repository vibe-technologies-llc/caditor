use std::fmt::Write;

use caditor_document::CancelToken;
use caditor_kernel::{SamplingTolerance, Solid};

use crate::{
    export::zip::{ZipEntry, archive},
    import::{ImportError, MeshFormat, ModelImport, parse_mesh, read_mesh_file},
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

fn binary_stl() -> Vec<u8> {
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

#[test]
fn a_text_stl_with_a_missing_triangle_is_closed_and_says_so() {
    let mut text = "solid cube\n".to_owned();
    for triangle in TRIANGLES.iter().skip(1) {
        text.push_str("facet normal 0 0 0\nouter loop\n");
        for corner in triangle {
            let [x, y, z] = CORNERS[*corner];
            writeln!(text, "vertex {x} {y} {z}").unwrap();
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid cube\n");

    let imported = parse_mesh(text.as_bytes(), MeshFormat::Stl, "cube.stl", "cube").unwrap();

    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
    assert!(
        imported
            .notes
            .contains(&"1 small hole in the surface was closed.".to_owned())
    );
}

#[test]
fn an_obj_of_quads_with_relative_indices_imports() {
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

    let imported = parse_mesh(text.as_bytes(), MeshFormat::Obj, "cube.obj", "cube").unwrap();

    assert!((only_volume(&imported) - 1000.0).abs() < 1e-6);
}

#[test]
fn a_3mf_places_its_components_and_converts_its_unit() {
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
    let package = archive(&[ZipEntry {
        name: "3D/3dmodel.model",
        contents: model.as_bytes(),
    }])
    .unwrap();

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
        Err(ImportError::DamagedMesh("STL"))
    );
    assert_eq!(
        parse_mesh(b"not a zip", MeshFormat::ThreeMf, "a.3mf", "a"),
        Err(ImportError::DamagedMesh("3MF"))
    );
    let sheet = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";

    assert!(matches!(
        parse_mesh(sheet.as_bytes(), MeshFormat::Obj, "a.obj", "a"),
        Err(ImportError::MeshNotClosed { .. })
    ));
}
