use std::{collections::BTreeMap, fmt::Write};

use crate::{
    import::{
        MAX_EXPANDED_OBJECTS, MeshFormat, TextOutlines,
        mesh::{MAX_PLACED_OBJECTS, MAX_PLACED_TRIANGLES, MAX_PLACED_VERTICES, read_three_mf},
        mesh_tests::{component_bomb, cube_object, model_package, nested_cube_objects},
        parse_mesh, parse_svg,
        zip_read::Archive,
    },
    tally,
    tests::seeds::{MESH_FORMATS, WRITE_SEEDS, is_refused, seed_mesh, seeds_in, write_seeds},
};

const INTER: &[u8] = include_bytes!("../../../caditor/assets/fonts/InterVariable.ttf");
const INTER_ITALIC: &[u8] =
    include_bytes!("../../../caditor/assets/fonts/InterVariable-Italic.ttf");
const CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const THREE_MF_PARTS: [&str; 2] = ["3D/3dmodel.model", "_rels/.rels"];
const COMPONENT_LEVELS: u32 = 16;
const OBJECT_ROWS: u32 = 10;
const OBJECT_COLUMNS: u32 = 12;
const NESTED_OBJECTS: usize = 400;
const GRID_CELLS: usize = 24;
const SVG_OPEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200mm" height="200mm" viewBox="0 0 200 200">"#;
const DRAWN_GROUP_LEVELS: usize = 99;
const HIDDEN_GROUP_LEVELS: usize = 120;
const USE_FAN_OUT: usize = 4;
const USE_LEVELS: usize = 7;
const REFUSED_USE_LEVELS: usize = 10;
const LONG_PATH_SEGMENTS: usize = 2000;
const TEXTS_ON_PATH: usize = 30;
const LABEL_USES: usize = 20;
const MANY_PATHS: usize = 400;
const MESH_WORK_PER_BYTE: usize = 1;
const SVG_WORK_PER_BYTE: usize = 2;

fn model(resources: &str, build: &str) -> Vec<u8> {
    model_package(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><model unit=\"millimeter\" xmlns=\"{CORE}\">\
         <resources>{resources}</resources><build>{build}</build></model>"
    ))
}

fn components_deep() -> Vec<u8> {
    let mut resources = cube_object(1, 10.0);

    for level in 2..=COMPONENT_LEVELS {
        write!(
            resources,
            "<object id=\"{level}\" type=\"model\"><components>\
             <component objectid=\"{}\" transform=\"0 1 0 -1 0 0 0 0 1 12 0 3\"/>\
             </components></object>",
            level - 1
        )
        .unwrap();
    }
    model(
        &resources,
        &format!("<item objectid=\"{COMPONENT_LEVELS}\" transform=\"1 0 0 0 1 0 0 0 1 5 5 0\"/>"),
    )
}

fn objects_many() -> Vec<u8> {
    let mut resources = String::new();
    let mut build = String::new();

    for row in 0..OBJECT_ROWS {
        for column in 0..OBJECT_COLUMNS {
            let id = row * OBJECT_COLUMNS + column + 1;
            let side = f64::from(1 + id % 10);

            resources.push_str(&cube_object(id, side));
            write!(
                build,
                "<item objectid=\"{id}\" transform=\"1 0 0 0 1 0 0 0 1 {} {} 0\"/>",
                20 * column,
                20 * row
            )
            .unwrap();
        }
    }
    model(&resources, &build)
}

fn triangles_many() -> Vec<u8> {
    let cells = GRID_CELLS;
    let mut index = BTreeMap::new();
    let mut vertices = String::new();
    let mut triangles = String::new();
    let mut corner = |x: usize, y: usize, z: usize, vertices: &mut String| -> usize {
        let count = index.len();

        *index.entry((x, y, z)).or_insert_with(|| {
            write!(vertices, "<vertex x=\"{x}\" y=\"{y}\" z=\"{z}\"/>").unwrap();
            count
        })
    };

    for axis in 0..3 {
        for high in [false, true] {
            let level = if high { cells } else { 0 };
            let at = |u: usize, v: usize| -> (usize, usize, usize) {
                match axis {
                    0 => (level, u, v),
                    1 => (v, level, u),
                    _ => (u, v, level),
                }
            };

            for u in 0..cells {
                for v in 0..cells {
                    let mut quad = [at(u, v), at(u + 1, v), at(u + 1, v + 1), at(u, v + 1)];

                    if !high {
                        quad.reverse();
                    }

                    let [a, b, c, d] = quad.map(|(x, y, z)| corner(x, y, z, &mut vertices));

                    write!(
                        triangles,
                        "<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>\
                         <triangle v1=\"{a}\" v2=\"{c}\" v3=\"{d}\"/>"
                    )
                    .unwrap();
                }
            }
        }
    }
    model(
        &format!(
            "<object id=\"1\" type=\"model\"><mesh><vertices>{vertices}</vertices>\
             <triangles>{triangles}</triangles></mesh></object>"
        ),
        "<item objectid=\"1\"/>",
    )
}

fn svg(body: &str) -> Vec<u8> {
    format!("{SVG_OPEN}{body}</svg>").into_bytes()
}

fn groups_deep() -> Vec<u8> {
    let mut body = String::new();

    for level in 0..DRAWN_GROUP_LEVELS {
        write!(
            body,
            "<g transform=\"rotate(3.6 100 100) translate(0.4 0.2)\">\
             <path d=\"M{} 20 h2 v2 h-2 z\"/>",
            level % 50
        )
        .unwrap();
    }
    body.push_str(&"</g>".repeat(DRAWN_GROUP_LEVELS));
    body.push_str(&"<g transform=\"scale(1.001)\">".repeat(HIDDEN_GROUP_LEVELS));
    body.push_str("<circle cx=\"10\" cy=\"10\" r=\"1\"/>");
    body.push_str(&"</g>".repeat(HIDDEN_GROUP_LEVELS));
    svg(&body)
}

fn fanned_uses(leaf: &str, levels: usize) -> Vec<u8> {
    let mut body = format!("<defs><path id=\"u0\" d=\"{leaf}\"/>");

    for level in 1..=levels {
        write!(body, "<g id=\"u{level}\">").unwrap();
        for copy in 0..USE_FAN_OUT {
            write!(
                body,
                "<use xlink:href=\"#u{}\" transform=\"translate({} {}) scale(0.5)\"/>",
                level - 1,
                copy % 2 * 100,
                copy / 2 * 100
            )
            .unwrap();
        }
        body.push_str("</g>");
    }
    write!(body, "</defs><use href=\"#u{levels}\"/>").unwrap();
    svg(&body)
}

fn long_path_data(segments: usize) -> String {
    let mut data = String::from("M 10 50");

    for segment in 1..=segments {
        write!(data, " L {} {}", 10 + segment / 20, 50 + segment % 2).unwrap();
    }
    data
}

fn text_paths() -> Vec<u8> {
    let mut body = format!(
        "<defs><path id=\"long\" d=\"{}\"/>\
         <g id=\"label\"><text font-size=\"2\"><textPath href=\"#long\">Seed</textPath></text></g>\
         </defs>",
        long_path_data(LONG_PATH_SEGMENTS)
    );

    for text in 0..TEXTS_ON_PATH {
        write!(
            body,
            "<text font-size=\"1.5\"><textPath href=\"#long\" startOffset=\"{}%\">On {text}\
             </textPath></text>",
            text * 3
        )
        .unwrap();
    }
    for copy in 0..LABEL_USES {
        write!(
            body,
            "<use href=\"#label\" transform=\"translate(0 {})\"/>",
            copy * 5
        )
        .unwrap();
    }
    body.push_str(
        "<text x=\"5\" y=\"190\" font-size=\"4\" transform=\"rotate(5)\">Plain \
         <tspan font-weight=\"700\" font-style=\"italic\">bold</tspan> text</text>",
    );
    svg(&body)
}

fn paths_many() -> Vec<u8> {
    let mut body = String::new();

    for path in 0..MANY_PATHS {
        let x = path % 20 * 10;
        let y = path / 20 * 10;

        write!(
            body,
            "<path transform=\"matrix(1 0 0.1 1 {x} {y}) rotate({path}) skewX(5)\" \
             d=\"M0 0 c1 2 3 2 4 0 s2-2 3 0 q1 2 2 0 t2 0 a2 1 15 0 1 2 2 l-1 3 h-4 v-2 z\"/>"
        )
        .unwrap();
    }
    svg(&body)
}

#[test]
#[ignore = "writes the 3MF and SVG reader fuzz seeds when CADITOR_WRITE_FUZZ_SEEDS is set"]
fn write_reader_fuzz_seeds() {
    if std::env::var_os(WRITE_SEEDS).is_none() {
        return;
    }

    write_seeds([
        (
            "mesh/components-deep.3mf",
            seed_mesh(MeshFormat::ThreeMf, &components_deep()),
        ),
        (
            "mesh/refused-components-fanned.3mf",
            seed_mesh(MeshFormat::ThreeMf, &component_bomb()),
        ),
        (
            "mesh/objects-many.3mf",
            seed_mesh(MeshFormat::ThreeMf, &objects_many()),
        ),
        (
            "mesh/objects-nested.3mf",
            seed_mesh(
                MeshFormat::ThreeMf,
                &model_package(&nested_cube_objects(NESTED_OBJECTS)),
            ),
        ),
        (
            "mesh/triangles-many.3mf",
            seed_mesh(MeshFormat::ThreeMf, &triangles_many()),
        ),
        ("svg/groups-deep.svg", groups_deep()),
        (
            "svg/uses-fanned.svg",
            fanned_uses("M0 0 L3 0 L0 3 Z", USE_LEVELS),
        ),
        (
            "svg/refused-uses-fanned.svg",
            fanned_uses(&long_path_data(LONG_PATH_SEGMENTS), REFUSED_USE_LEVELS),
        ),
        ("svg/text-paths.svg", text_paths()),
        ("svg/paths-many.svg", paths_many()),
    ]);
}

fn unpacked_len(format: MeshFormat, file: &[u8]) -> usize {
    if format != MeshFormat::ThreeMf {
        return file.len();
    }

    let archive = Archive::new(file).unwrap();

    THREE_MF_PARTS
        .iter()
        .filter_map(|part| archive.read(part, usize::MAX).ok())
        .map(|bytes| bytes.len())
        .sum()
}

fn placed_elements(format: MeshFormat, file: &[u8]) -> usize {
    match format {
        MeshFormat::ThreeMf => {
            read_three_mf(file).map_or(0, |mesh| mesh.positions.len() + mesh.triangles.len())
        }
        MeshFormat::Stl | MeshFormat::Obj => 0,
    }
}

#[test]
fn mesh_seeds_are_read_within_their_work_bound() {
    let placement_caps = MAX_PLACED_OBJECTS + MAX_PLACED_VERTICES + MAX_PLACED_TRIANGLES;

    for (path, bytes) in seeds_in("mesh") {
        let (&tag, file) = bytes.split_first().unwrap();
        let format = MESH_FORMATS[usize::from(tag)];

        let (read, work) = tally::measure(|| parse_mesh(file, format, "seed", "seed"));

        let read_through = unpacked_len(format, file) + placed_elements(format, file);
        let bound = match &read {
            Ok(import) => {
                assert!(!is_refused(&path), "{} was read", path.display());
                assert!(!import.bodies.is_empty(), "{}", path.display());
                MESH_WORK_PER_BYTE * read_through
            }
            Err(error) => {
                assert!(is_refused(&path), "{}: {error}", path.display());
                assert!(!error.to_string().is_empty(), "{}", path.display());
                MESH_WORK_PER_BYTE * read_through + placement_caps
            }
        };

        assert!(work <= bound, "{}: {work} > {bound}", path.display());
    }
}

#[test]
fn svg_seeds_are_read_within_their_work_bound() {
    let outlines = TextOutlines::InFont {
        upright: INTER,
        italic: Some(INTER_ITALIC),
    };

    for (path, bytes) in seeds_in("svg") {
        let (read, work) = tally::measure(|| parse_svg(&bytes, outlines));

        let bound = match &read {
            Ok(drawing) => {
                assert!(!is_refused(&path), "{} was read", path.display());
                assert!(!drawing.curves.is_empty(), "{}", path.display());
                SVG_WORK_PER_BYTE * (bytes.len() + drawing.curves.len())
            }
            Err(error) => {
                assert!(is_refused(&path), "{}: {error}", path.display());
                assert!(!error.to_string().is_empty(), "{}", path.display());
                SVG_WORK_PER_BYTE * bytes.len() + MAX_EXPANDED_OBJECTS
            }
        };

        assert!(work <= bound, "{}: {work} > {bound}", path.display());
    }
}
