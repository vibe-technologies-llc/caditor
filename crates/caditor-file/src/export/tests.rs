use std::{
    collections::BTreeMap,
    f64::consts::PI,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use caditor_document::CancelToken;
use caditor_geometry::{Plane, Point2, Point3};
use caditor_kernel::{LinearExtent, Profile, ProfileCurve, Selection, Solid, extrude};
use tempfile::TempDir;

use super::{
    stl::{FACET_LENGTH, HEADER_LENGTH},
    three_mf::{CONTENT_TYPES_PATH, Coordinate, MODEL_PATH, RELATIONSHIPS_PATH},
    zip::{CENTRAL_HEADER_SIGNATURE, DEFLATED, END_SIGNATURE, LOCAL_HEADER_SIGNATURE, STORED},
    *,
};

fn extruded(curves: &[ProfileCurve], height: f64) -> Solid {
    let regions = Profile::new(curves)
        .unwrap()
        .select(&Selection::EvenDepth)
        .unwrap();
    extrude(
        &Plane::XY,
        &regions,
        LinearExtent::one_side(height).unwrap(),
        1,
    )
    .unwrap()
}

fn block() -> Solid {
    let corners = [
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 20.0),
        Point2::new(0.0, 20.0),
    ];
    let lines: Vec<ProfileCurve> = (0..4)
        .map(|index| {
            ProfileCurve::line(
                index,
                corners[index as usize],
                corners[(index as usize + 1) % 4],
            )
        })
        .collect();
    extruded(&lines, 10.0)
}

fn pin() -> Solid {
    extruded(
        &[ProfileCurve::circle(1, Point2::new(100.0, 0.0), 5.0)],
        20.0,
    )
}

fn pin_volume() -> f64 {
    PI * 5.0 * 5.0 * 20.0
}

fn mesh_of(solid: &Solid, resolution: MeshResolution) -> MeshBody<'_> {
    MeshBody::tessellate(
        &ExportBody {
            name: "body",
            solid,
        },
        &resolution.tolerance([solid]),
        &CancelToken::never(),
    )
    .unwrap()
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn read_f32s(bytes: &[u8], at: usize) -> [f64; 3] {
    [0, 1, 2].map(|index| {
        f64::from(f32::from_le_bytes(
            bytes[at + 4 * index..at + 4 * index + 4]
                .try_into()
                .unwrap(),
        ))
    })
}

fn stl_facets(bytes: &[u8]) -> Vec<[Point3; 3]> {
    let count = read_u32(bytes, HEADER_LENGTH) as usize;
    assert_eq!(bytes.len(), HEADER_LENGTH + 4 + count * FACET_LENGTH);
    (0..count)
        .map(|index| {
            let start = HEADER_LENGTH + 4 + index * FACET_LENGTH;
            [1, 2, 3].map(|corner| Point3::from_array(read_f32s(bytes, start + 12 * corner)))
        })
        .collect()
}

fn signed_volume(facets: &[[Point3; 3]]) -> f64 {
    facets
        .iter()
        .map(|[a, b, c]| a.dot(b.cross(*c)) / 6.0)
        .sum()
}

fn assert_closed(triangles: impl IntoIterator<Item = [u32; 3]>) {
    let mut directed = BTreeMap::new();
    for [a, b, c] in triangles {
        for edge in [(a, b), (b, c), (c, a)] {
            *directed.entry(edge).or_insert(0) += 1;
        }
    }
    for ((from, to), uses) in &directed {
        assert_eq!(*uses, 1, "edge {from}–{to} is used {uses} times one way");
        assert_eq!(
            directed.get(&(*to, *from)),
            Some(&1),
            "edge {from}–{to} has no partner"
        );
    }
}

fn welded(facets: &[[Point3; 3]]) -> Vec<[u32; 3]> {
    let mut indices = BTreeMap::new();
    facets
        .iter()
        .map(|facet| {
            facet.map(|corner| {
                let key = corner.to_array().map(f64::to_bits);
                let next = indices.len() as u32;
                *indices.entry(key).or_insert(next)
            })
        })
        .collect()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let end = bytes.len() - 22;
    assert_eq!(read_u32(bytes, end), END_SIGNATURE);
    let count = read_u16(bytes, end + 10) as usize;
    assert_eq!(read_u16(bytes, end + 8) as usize, count);
    let mut central = read_u32(bytes, end + 16) as usize;
    assert_eq!(central + read_u32(bytes, end + 12) as usize, end);
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(read_u32(bytes, central), CENTRAL_HEADER_SIGNATURE);
        let method = read_u16(bytes, central + 10);
        let crc = read_u32(bytes, central + 16);
        let compressed = read_u32(bytes, central + 20) as usize;
        let size = read_u32(bytes, central + 24) as usize;
        let name_length = read_u16(bytes, central + 28) as usize;
        let local = read_u32(bytes, central + 42) as usize;
        let name =
            String::from_utf8(bytes[central + 46..central + 46 + name_length].to_vec()).unwrap();
        assert_eq!(read_u32(bytes, local), LOCAL_HEADER_SIGNATURE);
        assert_eq!(read_u16(bytes, local + 8), method);
        assert_eq!(read_u32(bytes, local + 14), crc);
        let data_start = local + 30 + name_length + read_u16(bytes, local + 28) as usize;
        let data = &bytes[data_start..data_start + compressed];
        let contents = match method {
            DEFLATED => miniz_oxide::inflate::decompress_to_vec(data).unwrap(),
            STORED => data.to_vec(),
            other => panic!("unexpected compression method {other}"),
        };
        assert_eq!(contents.len(), size);
        assert_eq!(crc32fast::hash(&contents), crc);
        entries.insert(name, contents);
        central += 46 + name_length;
    }
    entries
}

fn attribute(element: &str, name: &str) -> String {
    let start = element.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
    let length = element[start..].find('"').unwrap();
    element[start..start + length].to_owned()
}

fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    xml.split(&format!("<{tag} "))
        .skip(1)
        .map(|rest| &rest[..rest.find('>').unwrap()])
        .map(|element| element.trim_end_matches('/'))
        .collect()
}

struct ModelObject {
    name: String,
    positions: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
}

fn objects(xml: &str) -> Vec<ModelObject> {
    xml.split("<object ")
        .skip(1)
        .map(|object| {
            let header = format!(" {}", &object[..object.find('>').unwrap()]);
            let positions = elements(object, "vertex")
                .into_iter()
                .map(|vertex| {
                    let vertex = format!(" {vertex}");
                    Point3::new(
                        attribute(&vertex, "x").parse().unwrap(),
                        attribute(&vertex, "y").parse().unwrap(),
                        attribute(&vertex, "z").parse().unwrap(),
                    )
                })
                .collect();
            let triangles = elements(object, "triangle")
                .into_iter()
                .map(|triangle| {
                    let triangle = format!(" {triangle}");
                    ["v1", "v2", "v3"].map(|corner| attribute(&triangle, corner).parse().unwrap())
                })
                .collect();
            ModelObject {
                name: attribute(&header, "name"),
                positions,
                triangles,
            }
        })
        .collect()
}

#[test]
fn an_stl_holds_every_body_as_a_closed_outward_surface_in_millimetres() {
    let block = block();
    let pin = pin();
    let meshes = [
        mesh_of(&block, MeshResolution::Standard),
        mesh_of(&pin, MeshResolution::Standard),
    ];
    let bytes = stl::encode(&meshes).unwrap();
    assert!(!bytes.starts_with(b"solid"));
    assert!(String::from_utf8_lossy(&bytes[..HEADER_LENGTH]).contains("millimetres"));

    let facets = stl_facets(&bytes);
    assert_eq!(
        facets.len(),
        meshes[0].triangles.len() + meshes[1].triangles.len()
    );
    assert_closed(welded(&facets));
    let volume = signed_volume(&facets);
    let expected = 40.0 * 20.0 * 10.0 + pin_volume();
    assert!(
        (volume - expected).abs() < 0.01 * expected,
        "volume {volume}"
    );

    let first = HEADER_LENGTH + 4;
    let normal = Point3::from_array(read_f32s(&bytes, first));
    let [a, b, c] = facets[0];
    assert!((normal - (b - a).cross(c - a).normalize()).length() < 1e-6);
}

#[test]
fn a_3mf_coordinate_keeps_a_nanometre_and_drops_the_noise_beyond_it() {
    let printed = |value: f64| Coordinate(value).to_string();

    assert_eq!(printed(10.0), "10");
    assert_eq!(printed(-2.5), "-2.5");
    assert_eq!(printed(1.0 / 3.0), "0.333333");
    assert_eq!(printed(7.071067811865475), "7.071068");
    assert_eq!(printed(-1e-9), "0");
    assert_eq!(printed(0.0), "0");
    assert_eq!(printed(123456.1234567), "123456.123457");
}

#[test]
fn a_3mf_is_a_valid_package_with_one_named_object_per_body() {
    let block = block();
    let pin = pin();
    let mut meshes = [
        mesh_of(&block, MeshResolution::Coarse),
        mesh_of(&pin, MeshResolution::Coarse),
    ];
    meshes[0].name = "Plate & <\"Pin\">";
    meshes[1].name = "Pin 'rod'\u{FFFE}\u{FFFF}\u{7}é";
    let entries = unzip(&three_mf::encode(&meshes).unwrap());
    assert_eq!(
        entries.keys().map(String::as_str).collect::<Vec<_>>(),
        [MODEL_PATH, CONTENT_TYPES_PATH, RELATIONSHIPS_PATH]
    );
    let content_types = String::from_utf8(entries[CONTENT_TYPES_PATH].clone()).unwrap();
    assert!(content_types.contains("application/vnd.ms-package.3dmanufacturing-3dmodel+xml"));
    let relationships = String::from_utf8(entries[RELATIONSHIPS_PATH].clone()).unwrap();
    assert!(relationships.contains("Target=\"/3D/3dmodel.model\""));

    let model = String::from_utf8(entries[MODEL_PATH].clone()).unwrap();
    assert!(model.contains("unit=\"millimeter\""));
    assert!(model.contains("<item objectid=\"1\"/><item objectid=\"2\"/>"));
    let objects = objects(&model);
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].name, "Plate &amp; &lt;&quot;Pin&quot;&gt;");
    assert_eq!(objects[1].name, "Pin &apos;rod&apos;   é");
    for (object, mesh) in objects.iter().zip(&meshes) {
        assert_eq!(object.positions.len(), mesh.positions.len());
        for (read, written) in object.positions.iter().zip(&mesh.positions) {
            assert!(read.distance(*written) < 1e-5, "{read} {written}");
        }
        assert_eq!(object.triangles, mesh.triangles);
        assert_closed(object.triangles.iter().copied());
        let used: std::collections::BTreeSet<u32> =
            object.triangles.iter().flatten().copied().collect();
        assert_eq!(used.len(), object.positions.len());
    }
    let pin_facets: Vec<[Point3; 3]> = meshes[1].corners().collect();
    let volume = signed_volume(&pin_facets);
    assert!((volume - pin_volume()).abs() < 0.05 * pin_volume());
}

fn obj_objects(text: &str) -> Vec<(String, Vec<Point3>, Vec<[u32; 3]>)> {
    let mut objects: Vec<(String, Vec<Point3>, Vec<[u32; 3]>)> = Vec::new();
    let mut before = 0_u32;
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let (keyword, rest) = line.split_once(' ').unwrap();
        match keyword {
            "o" => {
                before = objects.iter().map(|object| object.1.len() as u32).sum();
                objects.push((rest.to_owned(), Vec::new(), Vec::new()));
            }
            "v" => {
                let [x, y, z] = rest
                    .split(' ')
                    .map(|number| number.parse().unwrap())
                    .collect::<Vec<f64>>()[..]
                else {
                    panic!("a vertex needs three numbers");
                };
                objects.last_mut().unwrap().1.push(Point3::new(x, y, z));
            }
            "f" => {
                let [a, b, c] = rest
                    .split(' ')
                    .map(|index| index.parse::<u32>().unwrap() - 1 - before)
                    .collect::<Vec<u32>>()[..]
                else {
                    panic!("a face needs three corners");
                };
                objects.last_mut().unwrap().2.push([a, b, c]);
            }
            other => panic!("unexpected keyword {other}"),
        }
    }
    objects
}

#[test]
fn an_obj_holds_each_body_as_a_named_closed_object_in_millimetres() {
    let block = block();
    let pin = pin();
    let meshes = [
        mesh_of(&block, MeshResolution::Standard),
        mesh_of(&pin, MeshResolution::Standard),
    ];

    let bytes = obj::encode(&meshes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let objects = obj_objects(&text);

    assert!(text.starts_with("# caditor "));
    assert!(text.lines().next().unwrap().contains("millimetres, Z up"));
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0].0, "body");
    let mut volume = 0.0;
    for ((_, positions, triangles), mesh) in objects.iter().zip(&meshes) {
        assert_eq!(positions.len(), mesh.positions.len());
        assert_eq!(triangles, &mesh.triangles);
        assert_closed(triangles.iter().copied());
        volume += signed_volume(
            &triangles
                .iter()
                .map(|triangle| triangle.map(|index| positions[index as usize]))
                .collect::<Vec<_>>(),
        );
    }
    let expected = 40.0 * 20.0 * 10.0 + pin_volume();
    assert!(
        (volume - expected).abs() < 0.01 * expected,
        "volume {volume}"
    );
}

#[test]
fn an_obj_names_a_body_without_line_breaks_or_emptiness() {
    let block = block();
    let mut first = mesh_of(&block, MeshResolution::Coarse);
    first.name = "Top\nplate\t";
    let mut second = mesh_of(&block, MeshResolution::Coarse);
    second.name = "  ";

    let text = String::from_utf8(obj::encode(&[first, second]).unwrap()).unwrap();
    let names: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("o "))
        .collect();

    assert_eq!(names, ["Top plate", "body"]);
}

struct Glb {
    json: serde_json::Value,
    binary: Vec<u8>,
}

fn glb(bytes: &[u8]) -> Glb {
    assert_eq!(read_u32(bytes, 0), 0x4654_6c67);
    assert_eq!(read_u32(bytes, 4), 2);
    assert_eq!(read_u32(bytes, 8) as usize, bytes.len());
    let json_length = read_u32(bytes, 12) as usize;
    assert_eq!(read_u32(bytes, 16), 0x4e4f_534a);
    assert_eq!(json_length % 4, 0);
    let json = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    let binary_start = 20 + json_length;
    let binary_length = read_u32(bytes, binary_start) as usize;
    assert_eq!(read_u32(bytes, binary_start + 4), 0x004e_4942);
    assert_eq!(binary_length % 4, 0);
    assert_eq!(binary_start + 8 + binary_length, bytes.len());
    Glb {
        json,
        binary: bytes[binary_start + 8..].to_vec(),
    }
}

fn glb_positions(glb: &Glb, accessor: usize) -> Vec<Point3> {
    let accessor = &glb.json["accessors"][accessor];
    assert_eq!(accessor["componentType"], 5126);
    assert_eq!(accessor["type"], "VEC3");
    let view = &glb.json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let start = view["byteOffset"].as_u64().unwrap() as usize;
    let count = accessor["count"].as_u64().unwrap() as usize;
    assert_eq!(view["byteLength"].as_u64().unwrap() as usize, count * 12);
    (0..count)
        .map(|index| {
            let [x, y, z] = read_f32s(&glb.binary, start + 12 * index);
            Point3::new(x * 1000.0, -z * 1000.0, y * 1000.0)
        })
        .collect()
}

fn glb_triangles(glb: &Glb, accessor: usize) -> Vec<[u32; 3]> {
    let accessor = &glb.json["accessors"][accessor];
    assert_eq!(accessor["componentType"], 5125);
    assert_eq!(accessor["type"], "SCALAR");
    let view = &glb.json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let start = view["byteOffset"].as_u64().unwrap() as usize;
    let count = accessor["count"].as_u64().unwrap() as usize;
    (0..count / 3)
        .map(|triangle| {
            [0, 1, 2].map(|corner| read_u32(&glb.binary, start + 4 * (3 * triangle + corner)))
        })
        .collect()
}

#[test]
fn a_glb_is_a_valid_binary_gltf_with_one_named_node_per_body_in_metres_with_y_up() {
    let block = block();
    let pin = pin();
    let meshes = [
        mesh_of(&block, MeshResolution::Standard),
        mesh_of(&pin, MeshResolution::Standard),
    ];

    let glb = glb(&gltf::encode(&meshes).unwrap());

    assert_eq!(glb.json["asset"]["version"], "2.0");
    assert_eq!(glb.json["scenes"][0]["nodes"], serde_json::json!([0, 1]));
    assert_eq!(
        glb.json["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
        glb.binary.len()
    );
    assert_eq!(glb.json["nodes"][0]["name"], "body");
    let mut volume = 0.0;
    for (index, mesh) in meshes.iter().enumerate() {
        let primitive = &glb.json["meshes"][index]["primitives"][0];
        assert_eq!(primitive["mode"], 4);
        let positions = glb_positions(
            &glb,
            primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
        );
        let triangles = glb_triangles(&glb, primitive["indices"].as_u64().unwrap() as usize);
        assert_eq!(triangles, mesh.triangles);
        for (found, expected) in positions.iter().zip(&mesh.positions) {
            assert!(
                (*found - *expected).length() < 1e-3,
                "{found} vs {expected}"
            );
        }
        assert_closed(triangles.iter().copied());
        volume += signed_volume(
            &triangles
                .iter()
                .map(|triangle| triangle.map(|corner| positions[corner as usize]))
                .collect::<Vec<_>>(),
        );
    }
    let expected = 40.0 * 20.0 * 10.0 + pin_volume();
    assert!(
        (volume - expected).abs() < 0.01 * expected,
        "volume {volume}"
    );
}

#[test]
fn a_glb_gives_each_position_accessor_the_bounds_gltf_requires() {
    let block = block();
    let meshes = [mesh_of(&block, MeshResolution::Coarse)];

    let glb = glb(&gltf::encode(&meshes).unwrap());
    let accessor = &glb.json["accessors"][0];

    let bound = |name: &str| -> Vec<f64> {
        accessor[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect()
    };
    let near = |found: &[f64], expected: [f64; 3]| {
        found
            .iter()
            .zip(expected)
            .all(|(found, expected)| (found - expected).abs() < 1e-6)
    };
    assert!(near(&bound("min"), [0.0, 0.0, -0.02]));
    assert!(near(&bound("max"), [0.04, 0.01, 0.0]));
}

#[test]
fn a_finer_resolution_follows_curved_faces_more_closely() {
    let pin = pin();
    let errors: Vec<f64> = MeshResolution::ALL
        .iter()
        .map(|resolution| {
            let facets: Vec<[Point3; 3]> = mesh_of(&pin, *resolution).corners().collect();
            (signed_volume(&facets) - pin_volume()).abs()
        })
        .collect();
    assert!(errors[0] > errors[1] && errors[1] > errors[2], "{errors:?}");
    assert!(errors[2] < 1e-3 * pin_volume());
}

#[test]
fn exporting_writes_the_file_and_reports_what_it_holds() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let bodies = [ExportBody {
        name: "Extrude 1",
        solid: &block,
    }];
    for format in ExportFormat::ALL {
        let path = dir.path().join(format!("part.{}", format.extension()));
        assert!(format.matches(&path));
        let exported = export_bodies(
            &path,
            format,
            MeshResolution::Standard,
            &bodies,
            &CancelToken::never(),
        )
        .unwrap();
        assert_eq!(exported.bodies, 1);
        let expected = format.is_mesh().then_some(12);
        assert_eq!(exported.triangles, expected);
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }
    let mut names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["part.3mf", "part.glb", "part.obj", "part.step", "part.stl"]
    );
    let step = std::fs::read_to_string(dir.path().join("part.step")).unwrap();
    assert!(step.contains("=MANIFOLD_SOLID_BREP('Extrude 1',"));
    assert!(step.contains("FILE_NAME('part',"));
}

#[test]
fn a_body_that_cannot_be_meshed_is_left_out_and_named_while_the_others_are_kept() {
    let block = block();
    let bodies = [
        ExportBody {
            name: "Good",
            solid: &block,
        },
        ExportBody {
            name: "Bad",
            solid: &block,
        },
    ];
    let mesher = |body: &ExportBody<'_>| match body.name {
        "Bad" => Err(ExportError::Meshing("Bad".to_owned())),
        _ => Ok(MeshBody {
            name: "Good",
            positions: Vec::new(),
            triangles: Vec::new(),
        }),
    };

    let (meshes, left_out) = tessellate_all(&bodies, &CancelToken::never(), mesher).unwrap();
    assert_eq!(meshes.len(), 1);
    assert_eq!(left_out, [ExportError::Meshing("Bad".to_owned())]);

    let only_bad = tessellate_all(&bodies[1..], &CancelToken::never(), mesher);
    assert_eq!(only_bad, Err(ExportError::Meshing("Bad".to_owned())));

    let cancelled = tessellate_all(&bodies, &CancelToken::never(), |_| {
        Err(ExportError::Cancelled)
    });
    assert_eq!(cancelled, Err(ExportError::Cancelled));
}

#[test]
fn a_cancelled_or_empty_export_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("part.stl");
    let block = block();
    let bodies = [ExportBody {
        name: "Extrude 1",
        solid: &block,
    }];
    let cancelled = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&cancelled);
    let cancel = CancelToken::new(move || flag.load(Ordering::SeqCst));
    assert_eq!(
        export_bodies(
            &path,
            ExportFormat::Stl,
            MeshResolution::Fine,
            &bodies,
            &cancel
        ),
        Err(ExportError::Cancelled)
    );
    assert_eq!(
        export_bodies(
            &path,
            ExportFormat::Stl,
            MeshResolution::Fine,
            &[],
            &CancelToken::never()
        ),
        Err(ExportError::Empty)
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    let missing = dir.path().join("gone").join("part.stl");
    let error = export_bodies(
        &missing,
        ExportFormat::Stl,
        MeshResolution::Coarse,
        &bodies,
        &CancelToken::never(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "its folder no longer exists");
}

#[test]
fn cancelling_stops_the_meshing_of_a_body_already_started() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let bodies = [ExportBody {
        name: "Extrude 1",
        solid: &block,
    }];
    for (format, name, allowed) in [
        (ExportFormat::Stl, "part.stl", 2),
        (ExportFormat::Step, "part.step", 0),
    ] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let cancel = CancelToken::new(move || counted.fetch_add(1, Ordering::SeqCst) >= allowed);
        let path = dir.path().join(name);
        assert_eq!(
            export_bodies(&path, format, MeshResolution::Fine, &bodies, &cancel),
            Err(ExportError::Cancelled),
            "{format:?}"
        );
        assert!(!path.exists());
    }
}
