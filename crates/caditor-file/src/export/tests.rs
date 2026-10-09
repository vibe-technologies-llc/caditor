use std::{
    collections::BTreeMap,
    f64::consts::PI,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use caditor_document::{CancelToken, ModelProperties};
use caditor_geometry::{Plane, Point2, Point3, Vector2, Vector3};
use caditor_kernel::{FaceId, LinearExtent, Profile, ProfileCurve, Selection, Solid, extrude};
use caditor_sketch::Sketch;
use tempfile::TempDir;

use super::{
    stl::{FACET_LENGTH, HEADER_LENGTH},
    three_mf::{CONTENT_TYPES_PATH, Coordinate, MODEL_PATH, RELATIONSHIPS_PATH, THUMBNAIL_PATH},
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
            look: None,
            group: None,
            threads: &[],
            faces: &[],
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
        assert_eq!(read_u32(bytes, local + 18) as usize, compressed);
        assert_eq!(read_u32(bytes, local + 22) as usize, size);
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
    let mut bytes = Vec::new();
    stl::write_binary(&mut bytes, &meshes, None, &CancelToken::never()).unwrap();
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

fn far_block() -> Solid {
    let corners = [
        Point2::new(1_000_000.0, 2_000_000.0),
        Point2::new(1_000_040.0, 2_000_000.0),
        Point2::new(1_000_040.0, 2_000_020.0),
        Point2::new(1_000_000.0, 2_000_020.0),
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

fn text_stl_solids(text: &str) -> Vec<(String, Vec<[Point3; 3]>)> {
    let mut solids: Vec<(String, Vec<[Point3; 3]>)> = Vec::new();
    let mut corners = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("solid") => solids.push((words.collect::<Vec<_>>().join(" "), Vec::new())),
            Some("vertex") => {
                let values: Vec<f64> = words.map(|word| word.parse().unwrap()).collect();
                corners.push(Point3::new(values[0], values[1], values[2]));
            }
            Some("endloop") => {
                let facet: [Point3; 3] = std::mem::take(&mut corners).try_into().unwrap();
                solids.last_mut().unwrap().1.push(facet);
            }
            Some("endsolid") => {
                assert_eq!(
                    words.collect::<Vec<_>>().join(" "),
                    solids.last().unwrap().0
                );
            }
            _ => {}
        }
    }
    solids
}

#[test]
fn a_text_stl_keeps_each_body_as_a_named_solid_with_exact_coordinates() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("parts.stl");
    let far = far_block();
    let pin = pin();
    let bodies = [
        ExportBody {
            name: "Far block",
            solid: &far,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        },
        ExportBody {
            name: "Kühler\tpin",
            solid: &pin,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        },
    ];
    let options = MeshOptions {
        resolution: MeshResolution::Coarse,
        stl: StlEncoding::Text,
        ..MeshOptions::default()
    };

    let exported = export_bodies(
        &path,
        ExportFormat::Stl,
        &options,
        &bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let solids = text_stl_solids(&text);

    assert_eq!(exported.bodies, 2);
    assert_eq!(exported.moved, None);
    assert!(text.is_ascii());
    assert_eq!(
        solids
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["Far_block", "K_hler_pin"]
    );
    for (_, facets) in &solids {
        assert_closed(welded(facets));
    }
    assert_eq!(
        solids.iter().map(|(_, facets)| facets.len()).sum::<usize>(),
        exported.triangles.unwrap()
    );
    assert!(
        solids[0]
            .1
            .iter()
            .flatten()
            .any(|corner| *corner == Point3::new(1_000_040.0, 2_000_020.0, 10.0))
    );
    assert!((signed_volume(&solids[0].1) - 40.0 * 20.0 * 10.0).abs() < 1e-6);
}

#[test]
fn a_binary_stl_far_from_the_origin_is_moved_near_it_and_says_by_how_much() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("far.stl");
    let far = far_block();
    let near = block();
    let bodies = |solid| {
        [ExportBody {
            name: "Block",
            solid,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        }]
    };
    let export = |path: &Path, solid| {
        export_bodies(
            path,
            ExportFormat::Stl,
            &MeshOptions::default(),
            &bodies(solid),
            &ModelProperties::default(),
            &CancelToken::never(),
        )
        .unwrap()
    };

    let exported = export(&path, &far);
    let bytes = std::fs::read(&path).unwrap();
    let facets = stl_facets(&bytes);
    let header = String::from_utf8_lossy(&bytes[..HEADER_LENGTH]).into_owned();
    let unmoved = export(&dir.path().join("near.stl"), &near);

    assert_eq!(
        exported.moved,
        Some(Vector3::new(-1_000_020.0, -2_000_010.0, -5.0))
    );
    assert!(header.contains("moved by -1000020 -2000010 -5"), "{header}");
    assert!(
        facets
            .iter()
            .flatten()
            .any(|corner| *corner == Point3::new(20.0, 10.0, 5.0))
    );
    assert!(
        facets
            .iter()
            .flatten()
            .all(|corner| corner.abs().max_element() <= 20.0)
    );
    assert!((signed_volume(&facets) - 40.0 * 20.0 * 10.0).abs() < 1e-6);
    assert_eq!(unmoved.moved, None);
}

#[test]
fn a_3mf_thumbnail_is_a_png_the_package_relationships_point_to() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("part.3mf");
    let block = block();
    let bodies = [ExportBody {
        name: "Block",
        solid: &block,
        look: None,
        group: None,
        threads: &[],
        faces: &[],
    }];
    let pixels: Vec<u8> = (0..4 * 3)
        .flat_map(|index| [index * 20, 40, 200, 255])
        .collect();
    let options = MeshOptions {
        thumbnail: Some(RgbaImage {
            width: 4,
            height: 3,
            pixels: &pixels,
        }),
        ..MeshOptions::default()
    };

    export_bodies(
        &path,
        ExportFormat::ThreeMf,
        &options,
        &bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let entries = unzip(&std::fs::read(&path).unwrap());
    let plain = unzip(
        &three_mf::encode(
            &[mesh_of(&block, MeshResolution::Coarse)],
            &ModelProperties::default(),
        )
        .unwrap(),
    );
    let relationships = String::from_utf8(entries[RELATIONSHIPS_PATH].clone()).unwrap();
    let types = String::from_utf8(entries[CONTENT_TYPES_PATH].clone()).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(entries[THUMBNAIL_PATH].clone()))
        .read_info()
        .unwrap();
    let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut decoded).unwrap();

    assert_eq!(
        entries.keys().collect::<Vec<_>>(),
        [
            MODEL_PATH,
            THUMBNAIL_PATH,
            CONTENT_TYPES_PATH,
            RELATIONSHIPS_PATH
        ]
    );
    assert!(relationships.contains(
        r#"<Relationship Target="/Metadata/thumbnail.png" Id="rel1" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail"/>"#
    ));
    assert!(types.contains(r#"<Default Extension="png" ContentType="image/png"/>"#));
    assert_eq!((frame.width, frame.height), (4, 3));
    assert_eq!(&decoded[..frame.buffer_size()], pixels.as_slice());
    assert!(!plain.contains_key(THUMBNAIL_PATH));
    assert!(
        !String::from_utf8(plain[RELATIONSHIPS_PATH].clone())
            .unwrap()
            .contains("thumbnail")
    );
    assert!(
        !String::from_utf8(plain[CONTENT_TYPES_PATH].clone())
            .unwrap()
            .contains("png")
    );
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
fn a_3mf_gives_coloured_bodies_a_base_material_and_leaves_the_rest_plain() {
    let block = block();
    let pin = pin();
    let mut meshes = [
        mesh_of(&block, MeshResolution::Coarse),
        mesh_of(&pin, MeshResolution::Coarse),
        mesh_of(&pin, MeshResolution::Coarse),
    ];
    meshes[0].look = Some(Look {
        colour: Rgb::new(200, 64, 52),
        opacity: None,
        material: Some("Steel & co"),
    });
    meshes[2].name = "Tinted";
    meshes[2].look = Some(Look {
        colour: Rgb::new(76, 160, 90),
        opacity: None,
        material: None,
    });

    let entries = unzip(&three_mf::encode(&meshes, &ModelProperties::default()).unwrap());

    let model = String::from_utf8(entries[MODEL_PATH].clone()).unwrap();
    assert!(model.contains(
        r##"<basematerials id="4"><base name="Steel &amp; co" displaycolor="#C84034"/><base name="Tinted" displaycolor="#4CA05A"/></basematerials>"##
    ));
    let headers: Vec<&str> = model
        .split("<object ")
        .skip(1)
        .map(|object| &object[..object.find('>').unwrap()])
        .collect();
    assert!(headers[0].contains(r#"pid="4" pindex="0""#));
    assert!(!headers[1].contains("pid="));
    assert!(headers[2].contains(r#"pid="4" pindex="1""#));
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
    let entries = unzip(&three_mf::encode(&meshes, &ModelProperties::default()).unwrap());
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

    let bytes = obj::encode(&meshes, None, &ModelProperties::default()).unwrap();
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

    let text = String::from_utf8(
        obj::encode(&[first, second], None, &ModelProperties::default()).unwrap(),
    )
    .unwrap();
    let names: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("o "))
        .collect();

    assert_eq!(names, ["Top plate", "body"]);
}

fn export_obj(path: &std::path::Path, bodies: &[ExportBody<'_>]) {
    export_bodies(
        path,
        ExportFormat::Obj,
        &MeshOptions {
            resolution: MeshResolution::Coarse,
            ..MeshOptions::default()
        },
        bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
}

#[test]
fn an_obj_of_coloured_bodies_points_into_a_material_library_beside_it() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let pin = pin();
    let path = dir.path().join("part.obj");
    let bodies = [
        ExportBody {
            name: "Base plate",
            solid: &block,
            look: Some(Look {
                colour: Rgb::new(255, 0, 51),
                opacity: None,
                material: None,
            }),
            group: None,
            threads: &[],
            faces: &[],
        },
        ExportBody {
            name: "Pin",
            solid: &pin,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        },
        ExportBody {
            name: "Cap",
            solid: &block,
            look: Some(Look {
                colour: Rgb::new(0, 0, 255),
                opacity: None,
                material: Some("Cast iron"),
            }),
            group: None,
            threads: &[],
            faces: &[],
        },
    ];

    export_obj(&path, &bodies);
    let obj = std::fs::read_to_string(&path).unwrap();
    let library = std::fs::read_to_string(dir.path().join("part.mtl")).unwrap();
    let used: Vec<&str> = obj
        .lines()
        .filter(|line| line.starts_with("o ") || line.starts_with("usemtl "))
        .collect();

    assert!(obj.lines().nth(1) == Some("mtllib part.mtl"));
    assert_eq!(
        used,
        [
            "o Base plate",
            "usemtl 1_Base_plate",
            "o Pin",
            "o Cap",
            "usemtl 3_Cast_iron"
        ]
    );
    assert!(library.starts_with("# caditor "));
    assert!(library.contains("newmtl 1_Base_plate\nKd 1.0000 0.0000 0.2000\n"));
    assert!(library.contains("newmtl 3_Cast_iron\nKd 0.0000 0.0000 1.0000\n"));
}

#[test]
fn an_obj_never_replaces_a_material_library_it_did_not_write() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("part.obj");
    let foreign = dir.path().join("part.mtl");
    std::fs::write(&foreign, "newmtl hand_made\nKd 1 1 1\n").unwrap();
    let coloured = [ExportBody {
        name: "Block",
        solid: &block,
        look: Some(Look {
            colour: Rgb::new(10, 20, 30),
            opacity: None,
            material: None,
        }),
        group: None,
        threads: &[],
        faces: &[],
    }];

    export_obj(&path, &coloured);
    let obj = std::fs::read_to_string(&path).unwrap();

    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        "newmtl hand_made\nKd 1 1 1\n"
    );
    assert!(!obj.contains("mtllib") && !obj.contains("usemtl"));

    std::fs::remove_file(&foreign).unwrap();
    export_obj(&path, &coloured);
    export_obj(&path, &coloured);

    assert!(
        std::fs::read_to_string(&foreign)
            .unwrap()
            .contains("newmtl 1_Block")
    );
}

#[test]
fn an_obj_of_plain_bodies_writes_no_material_library() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("part.obj");

    export_obj(
        &path,
        &[ExportBody {
            name: "Block",
            solid: &block,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        }],
    );

    assert!(!dir.path().join("part.mtl").exists());
    assert!(!std::fs::read_to_string(&path).unwrap().contains("mtllib"));
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
fn a_glb_gives_a_coloured_body_a_material_in_linear_colour() {
    let block = block();
    let pin = pin();
    let mut meshes = [
        mesh_of(&block, MeshResolution::Coarse),
        mesh_of(&pin, MeshResolution::Coarse),
    ];
    meshes[1].look = Some(Look {
        colour: Rgb::new(255, 0, 128),
        opacity: None,
        material: Some("Brass"),
    });

    let glb = glb(&gltf::encode(&meshes, &ModelProperties::default()).unwrap());

    assert!(
        glb.json["meshes"][0]["primitives"][0]
            .get("material")
            .is_none()
    );
    assert_eq!(glb.json["meshes"][1]["primitives"][0]["material"], 0);
    let material = &glb.json["materials"][0];
    assert_eq!(material["name"], "Brass");
    let factor = &material["pbrMetallicRoughness"]["baseColorFactor"];
    assert_eq!(factor[0], 1.0);
    assert_eq!(factor[1], 0.0);
    assert!((factor[2].as_f64().unwrap() - 0.2158605).abs() < 1e-6);
    assert_eq!(factor[3], 1.0);
}

#[test]
fn a_glb_node_and_an_obj_object_name_the_threads_of_their_body() {
    let block = block();
    let threads = [ExportThread {
        designation: "M6-6H".to_owned(),
        start: Point3::new(5.0, 5.0, 10.0),
        direction: -Vector3::Z,
        length: 8.0,
    }];
    let mut meshes = [
        mesh_of(&block, MeshResolution::Coarse),
        mesh_of(&block, MeshResolution::Coarse),
    ];
    meshes[0].threads = &threads;

    let glb = glb(&gltf::encode(&meshes, &ModelProperties::default()).unwrap());
    let obj = String::from_utf8(obj::encode(&meshes, None, &ModelProperties::default()).unwrap())
        .unwrap();

    assert_eq!(
        glb.json["nodes"][0]["extras"],
        serde_json::json!({ "threads": ["M6-6H"] })
    );
    assert!(glb.json["nodes"][1].get("extras").is_none());
    assert_eq!(obj.matches("# Thread: M6-6H").count(), 1);
    let thread_line = obj.lines().position(|line| line == "# Thread: M6-6H");
    assert_eq!(
        thread_line,
        obj.lines()
            .position(|line| line.starts_with("o "))
            .map(|line| line + 1)
    );
}

#[test]
fn a_glb_of_plain_bodies_has_no_materials() {
    let block = block();
    let meshes = [mesh_of(&block, MeshResolution::Coarse)];

    let glb = glb(&gltf::encode(&meshes, &ModelProperties::default()).unwrap());

    assert!(glb.json.get("materials").is_none());
}

#[test]
fn a_glb_is_a_valid_binary_gltf_with_one_named_node_per_body_in_metres_with_y_up() {
    let block = block();
    let pin = pin();
    let meshes = [
        mesh_of(&block, MeshResolution::Standard),
        mesh_of(&pin, MeshResolution::Standard),
    ];

    let glb = glb(&gltf::encode(&meshes, &ModelProperties::default()).unwrap());

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

    let glb = glb(&gltf::encode(&meshes, &ModelProperties::default()).unwrap());
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
        look: None,
        group: None,
        threads: &[],
        faces: &[],
    }];
    for format in ExportFormat::ALL {
        let path = dir.path().join(format!("part.{}", format.extension()));
        assert!(format.matches(&path));
        let exported = export_bodies(
            &path,
            format,
            &MeshOptions {
                resolution: MeshResolution::Standard,
                ..MeshOptions::default()
            },
            &bodies,
            &ModelProperties::default(),
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
fn a_step_export_styles_a_coloured_body_with_its_colour() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("part.step");
    let bodies = [ExportBody {
        name: "Extrude 1",
        solid: &block,
        look: Some(Look {
            colour: Rgb::new(255, 0, 0),
            opacity: None,
            material: Some("Steel"),
        }),
        group: None,
        threads: &[],
        faces: &[],
    }];

    export_bodies(
        &path,
        ExportFormat::Step,
        &MeshOptions {
            resolution: MeshResolution::Standard,
            ..MeshOptions::default()
        },
        &bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let step = std::fs::read_to_string(&path).unwrap();

    assert!(step.contains("=COLOUR_RGB('',1.0,0.0,0.0);"));
    assert_eq!(step.matches("=STYLED_ITEM('color',").count(), 1);
}

#[test]
fn a_step_export_writes_the_transparency_of_a_see_through_body_and_reads_it_back() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("panel.step");
    let body = |name, opacity| ExportBody {
        name,
        solid: &block,
        look: Some(Look {
            colour: Rgb::new(200, 220, 255),
            opacity,
            material: None,
        }),
        group: None,
        threads: &[],
        faces: &[],
    };
    let bodies = [body("Acrylic", Some(25)), body("Frame", None)];

    export_bodies(
        &path,
        ExportFormat::Step,
        &MeshOptions {
            resolution: MeshResolution::Standard,
            ..MeshOptions::default()
        },
        &bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let step = std::fs::read_to_string(&path).unwrap();
    let read = caditor_step::read_step(&step).unwrap();
    let opacity = |name: &str| {
        read.solids
            .iter()
            .find(|solid| solid.name == name)
            .unwrap()
            .opacity
    };

    assert_eq!(step.matches("=SURFACE_STYLE_TRANSPARENT(0.75);").count(), 1);
    assert_eq!(opacity("Acrylic"), Some(25));
    assert_eq!(opacity("Frame"), None);
}

#[test]
fn a_step_export_writes_the_colours_and_opacities_of_faces_and_reads_them_back() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("faces.step");
    let faces = [
        super::ExportFace {
            face: 0,
            colour: Rgb::new(0, 255, 0),
            opacity: None,
        },
        super::ExportFace {
            face: 2,
            colour: Rgb::new(200, 220, 255),
            opacity: Some(50),
        },
    ];
    let bodies = [ExportBody {
        name: "Block",
        solid: &block,
        look: Some(Look {
            colour: Rgb::new(200, 220, 255),
            opacity: None,
            material: None,
        }),
        group: None,
        threads: &[],
        faces: &faces,
    }];

    export_bodies(
        &path,
        ExportFormat::Step,
        &MeshOptions::default(),
        &bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let read = caditor_step::read_step(&std::fs::read_to_string(&path).unwrap()).unwrap();

    let [solid] = read.solids.as_slice() else {
        panic!("expected one solid");
    };
    assert_eq!((solid.colour, solid.opacity), (Some([200, 220, 255]), None));
    assert_eq!(
        solid.faces,
        [
            caditor_step::FaceLook {
                face: 0,
                colour: Some([0, 255, 0]),
                opacity: None,
            },
            caditor_step::FaceLook {
                face: 2,
                colour: None,
                opacity: Some(50),
            },
        ]
    );
}

#[test]
fn a_step_export_puts_bodies_in_a_folder_on_a_layer_named_after_it() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("parts.step");
    let body = |name, group| ExportBody {
        name,
        solid: &block,
        look: None,
        group,
        threads: &[],
        faces: &[],
    };
    let bodies = [
        body("Bolt", Some("Hardware")),
        body("Plate", None),
        body("Nut", Some("Hardware")),
    ];

    export_bodies(
        &path,
        ExportFormat::Step,
        &MeshOptions::default(),
        &bodies,
        &ModelProperties::default(),
        &CancelToken::never(),
    )
    .unwrap();
    let step = std::fs::read_to_string(&path).unwrap();

    assert_eq!(step.matches("=PRESENTATION_LAYER_ASSIGNMENT(").count(), 1);
    assert!(step.contains("=PRESENTATION_LAYER_ASSIGNMENT('Hardware','',(#"));
}

#[test]
fn a_body_that_cannot_be_meshed_is_left_out_and_named_while_the_others_are_kept() {
    let block = block();
    let bodies = [
        ExportBody {
            name: "Good",
            solid: &block,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        },
        ExportBody {
            name: "Bad",
            solid: &block,
            look: None,
            group: None,
            threads: &[],
            faces: &[],
        },
    ];
    let mesher = |body: &ExportBody<'_>| match body.name {
        "Bad" => Err(ExportError::Meshing("Bad".to_owned())),
        _ => Ok(MeshBody {
            name: "Good",
            look: None,
            threads: &[],
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
        look: None,
        group: None,
        threads: &[],
        faces: &[],
    }];
    let cancelled = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&cancelled);
    let cancel = CancelToken::new(move || flag.load(Ordering::SeqCst));
    assert_eq!(
        export_bodies(
            &path,
            ExportFormat::Stl,
            &MeshOptions {
                resolution: MeshResolution::Fine,
                ..MeshOptions::default()
            },
            &bodies,
            &ModelProperties::default(),
            &cancel
        ),
        Err(ExportError::Cancelled)
    );
    assert_eq!(
        export_bodies(
            &path,
            ExportFormat::Stl,
            &MeshOptions {
                resolution: MeshResolution::Fine,
                ..MeshOptions::default()
            },
            &[],
            &ModelProperties::default(),
            &CancelToken::never()
        ),
        Err(ExportError::Empty)
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    let missing = dir.path().join("gone").join("part.stl");
    let error = export_bodies(
        &missing,
        ExportFormat::Stl,
        &MeshOptions {
            resolution: MeshResolution::Coarse,
            ..MeshOptions::default()
        },
        &bodies,
        &ModelProperties::default(),
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
        look: None,
        group: None,
        threads: &[],
        faces: &[],
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
            export_bodies(
                &path,
                format,
                &MeshOptions {
                    resolution: MeshResolution::Fine,
                    ..MeshOptions::default()
                },
                &bodies,
                &ModelProperties::default(),
                &cancel
            ),
            Err(ExportError::Cancelled),
            "{format:?}"
        );
        assert!(!path.exists());
    }
}

fn drawn_sketch() -> Sketch {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(40.0, 0.0));
    sketch.add_circle(Point2::new(20.0, 10.0), 4.0);
    sketch.add_arc(
        Point2::new(0.0, 0.0),
        Point2::new(10.0, 0.0),
        Point2::new(0.0, 10.0),
    );
    sketch.add_spline(&[
        Point2::new(0.0, 20.0),
        Point2::new(10.0, 30.0),
        Point2::new(20.0, 20.0),
        Point2::new(30.0, 30.0),
    ]);
    sketch.add_point(Point2::new(-5.0, -5.0));
    let guide = sketch.add_line(Point2::new(0.0, -10.0), Point2::new(40.0, -10.0));
    sketch.set_construction(guide, true).unwrap();
    sketch
}

#[test]
fn a_sketch_exports_to_a_dxf_that_reads_back_as_the_same_curves() {
    let sketch = drawn_sketch();

    let (figure, exported) = Figure::of_sketch(&sketch, Construction::LeftOut);
    let text = dxf::encode(&figure);
    let drawing = crate::parse_dxf(text.as_bytes()).unwrap();

    assert_eq!(
        exported,
        SketchExported {
            curves: 4,
            points: 1,
            construction: 0,
            construction_left_out: 1,
            sketches: 1,
            ..SketchExported::default()
        }
    );
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
    assert_eq!(drawing.curves.len(), 5);
    let near = |a: Point2, b: Point2| a.distance(b) < 1e-9;
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Line { start, end }
            if near(*start, Point2::new(0.0, 0.0)) && near(*end, Point2::new(40.0, 0.0))
    )));
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Circle { center, radius }
            if near(*center, Point2::new(20.0, 10.0)) && (radius - 4.0).abs() < 1e-9
    )));
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Arc { center, start, end }
            if near(*center, Point2::ZERO)
                && near(*start, Point2::new(10.0, 0.0))
                && near(*end, Point2::new(0.0, 10.0))
    )));
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Spline { control_points }
            if control_points.len() == 4 && near(control_points[1], Point2::new(10.0, 30.0))
    )));
    assert!(drawing
        .curves
        .iter()
        .any(|curve| matches!(curve, crate::DrawingCurve::Point(point) if near(*point, Point2::new(-5.0, -5.0)))));
}

#[test]
fn a_sketch_ellipse_exports_to_a_dxf_ellipse_that_reads_back_exactly() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_ellipse(Point2::new(5.0, 5.0), Point2::new(5.0, 15.0), 4.0);
    let start = Point2::new(30.0 + 2.0 * 0.6, 3.0 * 0.8);
    let end = Point2::new(30.0, -3.0);
    sketch.add_elliptical_arc(
        Point2::new(30.0, 0.0),
        Point2::new(32.0, 0.0),
        3.0,
        start,
        end,
    );

    let (figure, exported) = Figure::of_sketch(&sketch, Construction::LeftOut);
    let drawing = crate::parse_dxf(dxf::encode(&figure).as_bytes()).unwrap();

    assert_eq!(exported.curves, 2);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
    let near = |a: Point2, b: Point2| a.distance(b) < 1e-9;
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Ellipse { center, major, minor_radius, ends: None }
            if near(*center, Point2::new(5.0, 5.0))
                && (major.length() - 10.0).abs() < 1e-9
                && major.x.abs() < 1e-9
                && (minor_radius - 4.0).abs() < 1e-9
    )));
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Ellipse { center, minor_radius, ends: Some((from, to)), .. }
            if near(*center, Point2::new(30.0, 0.0))
                && (minor_radius - 2.0).abs() < 1e-9
                && near(*from, start)
                && near(*to, end)
    )));
}

#[test]
fn construction_kept_goes_on_its_own_dashed_layer_and_reads_back_as_construction() {
    let sketch = drawn_sketch();

    let (figure, exported) = Figure::of_sketch(&sketch, Construction::OnLayer);
    let text = dxf::encode(&figure);
    let drawing = crate::parse_dxf(text.as_bytes()).unwrap();
    let image = svg::encode(&figure).unwrap();
    let only_construction = {
        let mut guides = Sketch::new(Plane::XY);
        let guide = guides.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
        guides.set_construction(guide, true).unwrap();
        Figure::of_sketch(&guides, Construction::OnLayer).1
    };

    assert_eq!(
        exported,
        SketchExported {
            curves: 4,
            points: 1,
            construction: 1,
            construction_left_out: 0,
            sketches: 1,
            ..SketchExported::default()
        }
    );
    assert!(text.contains("  8\nConstruction\n  6\nDASHED\n"));
    assert_eq!(drawing.curves.len(), 6);
    assert_eq!(drawing.construction.len(), 1);
    let guide = drawing.construction.iter().next().copied().unwrap();
    assert!(matches!(
        drawing.curves[guide],
        crate::DrawingCurve::Line { start, .. } if start.distance(Point2::new(0.0, -10.0)) < 1e-9
    ));
    assert!(image.contains(r##"<g id="Construction" stroke="#808080" stroke-dasharray="1 0.5">"##));
    assert_eq!(only_construction.construction, 1);
}

#[test]
fn a_dxf_names_its_unit_so_importing_it_needs_no_conversion() {
    let text = dxf::encode(&Figure::of_sketch(&drawn_sketch(), Construction::LeftOut).0);

    assert!(text.contains("$INSUNITS\n 70\n4\n"));
    assert!(text.ends_with("  0\nEOF\n"));
}

#[test]
fn a_sketch_of_only_construction_has_nothing_to_export() {
    let mut sketch = Sketch::new(Plane::XY);
    let guide = sketch.add_line(Point2::ZERO, Point2::new(10.0, 0.0));
    sketch.set_construction(guide, true).unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("guide.dxf");

    let result = export_sketch(
        &path,
        &sketch,
        SketchFormat::Dxf,
        Construction::LeftOut,
        &CancelToken::never(),
    );

    assert_eq!(result, Err(ExportError::NoCurves));
    assert!(!path.exists());
}

#[test]
fn exporting_a_sketch_writes_the_file_and_a_cancelled_one_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("sketch.dxf");
    let cancelled = dir.path().join("cancelled.dxf");

    let exported = export_sketch(
        &path,
        &drawn_sketch(),
        SketchFormat::Dxf,
        Construction::LeftOut,
        &CancelToken::never(),
    )
    .unwrap();
    let refused = export_sketch(
        &cancelled,
        &drawn_sketch(),
        SketchFormat::Dxf,
        Construction::LeftOut,
        &CancelToken::new(|| true),
    );

    assert_eq!(exported.curves, 4);
    assert!(std::fs::read_to_string(&path).unwrap().contains("SPLINE"));
    assert_eq!(refused, Err(ExportError::Cancelled));
    assert!(!cancelled.exists());
}

#[test]
fn a_sketch_exports_to_an_svg_in_millimetres_with_y_pointing_down() {
    let sketch = drawn_sketch();

    let (figure, exported) = Figure::of_sketch(&sketch, Construction::LeftOut);
    let text = svg::encode(&figure).unwrap();

    assert_eq!(
        exported,
        SketchExported {
            curves: 4,
            points: 1,
            construction: 0,
            construction_left_out: 1,
            sketches: 1,
            ..SketchExported::default()
        }
    );
    assert!(text.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" width="#));
    assert!(text.contains(r#"<line x1="0" y1="0" x2="40" y2="0"/>"#));
    assert!(text.contains(r#"<circle cx="20" cy="-10" r="4"/>"#));
    assert!(text.contains(r#"<path d="M 10 0 A 10 10 0 0 0 0 -10"/>"#));
    assert!(text.contains("<polyline points=\"0,-20 "));
    assert!(text.trim_end().ends_with("</svg>"));
    assert!(!text.contains("-10 L"));
}

#[test]
fn an_svg_frames_the_drawing_with_a_margin_in_its_viewbox() {
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::new(0.0, 0.0), Point2::new(40.0, 20.0));

    let text = svg::encode(&Figure::of_sketch(&sketch, Construction::LeftOut).0).unwrap();

    assert!(
        text.contains(r#"width="42mm" height="22mm" viewBox="-1 -21 42 22""#),
        "{text}"
    );
}

#[test]
fn a_sketch_format_is_found_from_the_extension_in_any_case() {
    assert_eq!(
        SketchFormat::of(Path::new("a.dxf")),
        Some(SketchFormat::Dxf)
    );
    assert_eq!(
        SketchFormat::of(Path::new("a.SVG")),
        Some(SketchFormat::Svg)
    );
    assert_eq!(SketchFormat::of(Path::new("a.png")), None);
    assert_eq!(SketchFormat::of(Path::new("dxf")), None);
}

fn rounded_plate() -> Solid {
    let spline_back = ProfileCurve::spline(
        4,
        3,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        vec![
            Point2::new(0.0, 20.0),
            Point2::new(-10.0, 15.0),
            Point2::new(-10.0, 5.0),
            Point2::new(0.0, 0.0),
        ],
    );
    extruded(
        &[
            ProfileCurve::line(1, Point2::new(0.0, 0.0), Point2::new(40.0, 0.0)),
            ProfileCurve::arc(
                2,
                Point2::new(40.0, 10.0),
                Point2::new(40.0, 0.0),
                Point2::new(40.0, 20.0),
            ),
            ProfileCurve::line(3, Point2::new(40.0, 20.0), Point2::new(0.0, 20.0)),
            spline_back,
            ProfileCurve::circle(5, Point2::new(20.0, 10.0), 4.0),
        ],
        10.0,
    )
}

fn face_facing(solid: &Solid, normal: Vector3) -> FaceId {
    solid
        .faces()
        .map(|(id, _)| id)
        .find(|id| {
            caditor_document::face_plane(solid, *id)
                .is_some_and(|plane| plane.normal().distance(normal) < 1e-9)
        })
        .unwrap()
}

fn layer_of(drawing: &crate::Drawing, index: usize) -> &str {
    &drawing.layers[drawing.curve_layers[index]]
}

fn exported_drawing(solid: &Solid, face: FaceId) -> (crate::Drawing, FaceExported) {
    let (figure, exported) = outline::face_figure(solid, face).unwrap();
    let drawing = crate::parse_dxf(dxf::encode(&figure).as_bytes()).unwrap();
    (drawing, exported)
}

#[test]
fn a_flat_face_exports_its_outline_and_holes_exactly_on_layers_of_their_own() {
    let solid = rounded_plate();
    let near = |a: Point2, b: Point2| a.distance(b) < 1e-9;

    let (drawing, exported) = exported_drawing(&solid, face_facing(&solid, Vector3::Z));

    assert_eq!(
        exported,
        FaceExported {
            faces: 1,
            loops: 2,
            curves: 5,
            approximated: 0,
            too_wide: 0,
        }
    );
    assert!(
        drawing.notes.iter().all(|note| !note.contains("unit")),
        "{:?}",
        drawing.notes
    );
    assert_eq!(drawing.curves.len(), 5);
    for (index, curve) in drawing.curves.iter().enumerate() {
        let expected = match curve {
            crate::DrawingCurve::Circle { .. } => "Holes",
            _ => "Outline",
        };
        assert_eq!(layer_of(&drawing, index), expected);
    }
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Circle { center, radius }
            if near(*center, Point2::new(20.0, 10.0)) && (radius - 4.0).abs() < 1e-9
    )));
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Arc { center, start, end }
            if near(*center, Point2::new(40.0, 10.0))
                && near(*start, Point2::new(40.0, 0.0))
                && near(*end, Point2::new(40.0, 20.0))
    )));
    assert!(drawing.curves.iter().any(|curve| matches!(
        curve,
        crate::DrawingCurve::Line { start, end }
            if [*start, *end].iter().any(|point| near(*point, Point2::new(40.0, 0.0)))
                && [*start, *end].iter().any(|point| near(*point, Point2::ZERO))
    )));
    let spline = drawing
        .curves
        .iter()
        .find_map(|curve| match curve {
            crate::DrawingCurve::Spline { control_points } => Some(control_points),
            _ => None,
        })
        .unwrap();
    assert_eq!(spline.len(), 4);
    assert!(
        spline
            .iter()
            .any(|point| near(*point, Point2::new(-10.0, 15.0)))
    );
}

#[test]
fn a_flat_face_cut_through_a_torus_writes_each_traced_edge_as_one_spline() {
    let ring = caditor_kernel::revolve(
        &Plane::XZ,
        &Profile::new(&[ProfileCurve::circle(1, Point2::new(20.0, 0.0), 5.0)])
            .unwrap()
            .select(&Selection::EvenDepth)
            .unwrap(),
        caditor_kernel::Axis2::new(Point2::ZERO, Point2::new(0.0, 1.0)).unwrap(),
        caditor_kernel::AngularExtent::new(0.0, 2.0 * PI).unwrap(),
        1,
    )
    .unwrap();
    let slab = extruded(
        &[
            ProfileCurve::line(11, Point2::new(-40.0, -40.0), Point2::new(40.0, -40.0)),
            ProfileCurve::line(12, Point2::new(40.0, -40.0), Point2::new(40.0, 40.0)),
            ProfileCurve::line(13, Point2::new(40.0, 40.0), Point2::new(-40.0, 40.0)),
            ProfileCurve::line(14, Point2::new(-40.0, 40.0), Point2::new(-40.0, -40.0)),
        ],
        40.0,
    );
    let tilt = caditor_geometry::RigidTransform::rotation_about(
        Point3::new(0.0, 0.0, 1.0),
        Vector3::Y,
        0.15,
    )
    .unwrap();
    let cutter = slab.transformed(&tilt).unwrap();
    let cut = caditor_kernel::boolean(&ring, &cutter, caditor_kernel::BooleanOperation::Difference)
        .unwrap();
    let normal = tilt.apply_vector(Vector3::Z);
    let face = face_facing(&cut, normal);

    let (figure, exported) = outline::face_figure(&cut, face).unwrap();

    assert!(
        exported.approximated > 0,
        "{exported:?} {:?}",
        figure.shapes
    );
    assert!(
        figure
            .shapes
            .iter()
            .all(|(_, shape)| !matches!(shape, figure::Shape::Polyline(_))),
        "{:?}",
        figure.shapes
    );
    let drawing = crate::parse_dxf(dxf::encode(&figure).as_bytes()).unwrap();
    assert_eq!(drawing.curves.len(), exported.curves);
}

#[test]
fn several_faces_go_into_one_drawing_side_by_side() {
    let plate = rounded_plate();
    let other = block();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nest.dxf");
    let faces = [
        NamedFace {
            name: "Plate",
            solid: &plate,
            face: face_facing(&plate, Vector3::Z),
        },
        NamedFace {
            name: "Block",
            solid: &other,
            face: face_facing(&other, Vector3::Z),
        },
    ];

    let exported = export_faces(
        &path,
        &faces,
        SketchFormat::Dxf,
        &DrawingSheet::default(),
        &CancelToken::never(),
    )
    .unwrap();

    let single = outline::face_figure(&plate, faces[0].face).unwrap().1;
    let second = outline::face_figure(&other, faces[1].face).unwrap().1;
    assert_eq!(exported.faces, 2);
    assert_eq!(exported.curves, single.curves + second.curves);
    assert_eq!(exported.loops, single.loops + second.loops);
    let drawing = crate::parse_dxf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(drawing.curves.len(), exported.curves);
    let (first_part, second_part) = drawing.curves.split_at(single.curves);
    let reach = |curves: &[crate::DrawingCurve]| {
        curves
            .iter()
            .flat_map(|curve| match curve {
                crate::DrawingCurve::Point(point) => vec![*point],
                crate::DrawingCurve::Line { start, end } => vec![*start, *end],
                crate::DrawingCurve::Circle { center, radius } => {
                    vec![
                        *center - Vector2::splat(*radius),
                        *center + Vector2::splat(*radius),
                    ]
                }
                crate::DrawingCurve::Arc { center, start, .. } => {
                    let radius = center.distance(*start);
                    vec![
                        *center - Vector2::splat(radius),
                        *center + Vector2::splat(radius),
                    ]
                }
                crate::DrawingCurve::Spline { control_points } => control_points.clone(),
                crate::DrawingCurve::Ellipse {
                    center,
                    major,
                    minor_radius,
                    ..
                } => {
                    let reach = major.length().max(*minor_radius);
                    vec![
                        *center - Vector2::splat(reach),
                        *center + Vector2::splat(reach),
                    ]
                }
            })
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                (low.min(point.x), high.max(point.x))
            })
    };
    let (_, first_right) = reach(first_part);
    let (second_left, _) = reach(second_part);
    assert!(
        second_left >= first_right + 10.0 - 1e-6,
        "{first_right} {second_left}"
    );
}

#[test]
fn faces_are_drawn_as_seen_from_outside_with_z_or_y_up() {
    let solid = rounded_plate();
    let near = |a: Point2, b: Point2| a.distance(b) < 1e-9;
    let arc_center = |drawing: &crate::Drawing| {
        drawing.curves.iter().find_map(|curve| match curve {
            crate::DrawingCurve::Arc { center, .. } => Some(*center),
            _ => None,
        })
    };

    let (bottom, _) = exported_drawing(&solid, face_facing(&solid, Vector3::NEG_Z));
    let (front, front_exported) = exported_drawing(&solid, face_facing(&solid, Vector3::NEG_Y));

    assert!(near(arc_center(&bottom).unwrap(), Point2::new(-40.0, 10.0)));
    assert_eq!(front_exported.loops, 1);
    let corners: Vec<Point2> = front
        .curves
        .iter()
        .filter_map(|curve| match curve {
            crate::DrawingCurve::Line { start, end } => Some([*start, *end]),
            _ => None,
        })
        .flatten()
        .collect();
    for corner in [
        Point2::new(0.0, 0.0),
        Point2::new(40.0, 0.0),
        Point2::new(40.0, 10.0),
        Point2::new(0.0, 10.0),
    ] {
        assert!(
            corners.iter().any(|point| near(*point, corner)),
            "{corners:?}"
        );
    }
}

#[test]
fn a_face_exports_to_an_svg_grouping_its_outline_and_holes() {
    let solid = rounded_plate();
    let (figure, _) = outline::face_figure(&solid, face_facing(&solid, Vector3::Z)).unwrap();

    let text = svg::encode(&figure).unwrap();

    let outline = text.find(r#"<g id="Outline">"#).unwrap();
    let holes = text.find(r#"<g id="Holes">"#).unwrap();
    let circle = text.find(r#"<circle cx="20" cy="-10" r="4"/>"#).unwrap();
    assert!(outline < holes && holes < circle);
    assert!(text.contains(r#"<path d="M 40 0 A 10 10 0 "#), "{text}");
    assert!(text.contains(r#" 0 40 -20"/>"#), "{text}");
    assert_eq!(text.matches("<polyline ").count(), 1);
}

#[test]
fn a_curved_or_missing_face_writes_nothing() {
    let solid = rounded_plate();
    let curved = solid
        .faces()
        .map(|(id, _)| id)
        .find(|id| caditor_document::face_plane(&solid, *id).is_none())
        .unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("curved.dxf");

    let result = export_face(
        &path,
        &solid,
        curved,
        SketchFormat::Dxf,
        &CancelToken::never(),
    );

    assert_eq!(result, Err(ExportError::FaceNotFlat));
    assert!(!path.exists());
    let top = face_facing(&solid, Vector3::Z);
    let cancelled = export_face(
        &path,
        &solid,
        top,
        SketchFormat::Svg,
        &CancelToken::new(|| true),
    );
    assert_eq!(cancelled, Err(ExportError::Cancelled));
    assert!(!path.exists());
    let written = export_face(&path, &solid, top, SketchFormat::Dxf, &CancelToken::never());
    assert_eq!(written.map(|exported| exported.curves), Ok(5));
    assert!(std::fs::read_to_string(&path).unwrap().contains("Holes"));
}

#[test]
fn an_elliptical_edge_keeps_its_arc_whichever_way_its_frame_faces() {
    let drawing = Plane::XY;
    let tilted = Vector3::new(1.0, 2.0, 0.0);
    let range = caditor_kernel::Interval::new(0.4, 2.5).unwrap();
    let near = |a: Point2, b: Point2| a.distance(b) < 1e-9;

    for normal in [Vector3::Z, Vector3::NEG_Z] {
        for (along_x, along_y) in [(8.0, 3.0), (3.0, 8.0)] {
            let frame = Plane::with_x_axis(Point3::new(5.0, -2.0, 0.0), normal, tilted).unwrap();
            let ellipse = caditor_kernel::Ellipse::new(frame, along_x, along_y).unwrap();
            let curve = caditor_kernel::Curve::Ellipse(ellipse);
            let on_drawing = |parameter: f64| drawing.to_local(curve.point(parameter));

            let arc =
                outline::ellipse_arc(&drawing, &frame, along_x, along_y, range, false).unwrap();

            assert!(arc.ratio <= 1.0);
            assert!((arc.major.length() - 8.0).abs() < 1e-12);
            let ends = [arc.point_at(arc.start), arc.point_at(arc.end)];
            for parameter in [range.start(), range.end()] {
                assert!(ends.iter().any(|end| near(*end, on_drawing(parameter))));
            }
            let middle = arc.point_at((arc.start + arc.end) / 2.0);
            assert!(near(middle, on_drawing(range.middle())));
        }
    }
}

fn bracket_properties() -> ModelProperties {
    ModelProperties {
        title: "Wall <bracket>".to_owned(),
        part_number: "BR-100".to_owned(),
        revision: "C".to_owned(),
        author: "Drafter".to_owned(),
        organisation: "Workshop".to_owned(),
        description: "Holds a shelf".to_owned(),
        notes: "Never exported".to_owned(),
    }
}

#[test]
fn a_3mf_carries_the_model_properties_it_has_names_for() {
    let block = block();
    let meshes = [mesh_of(&block, MeshResolution::Coarse)];

    let entries = unzip(&three_mf::encode(&meshes, &bracket_properties()).unwrap());
    let plain = unzip(&three_mf::encode(&meshes, &ModelProperties::default()).unwrap());

    let model = String::from_utf8(entries[MODEL_PATH].clone()).unwrap();
    let plain = String::from_utf8(plain[MODEL_PATH].clone()).unwrap();
    assert!(model.contains(r#"<metadata name="Title">Wall &lt;bracket&gt;</metadata>"#));
    assert!(model.contains(r#"<metadata name="Designer">Drafter</metadata>"#));
    assert!(model.contains(r#"<metadata name="Description">Holds a shelf</metadata>"#));
    assert!(model.contains(r#"<item objectid="1" partnumber="BR-100"/>"#));
    assert!(!model.contains("Never exported"));
    assert!(!plain.contains(r#"name="Title""#));
    assert!(plain.contains(r#"<item objectid="1"/>"#));
}

#[test]
fn a_3mf_of_several_bodies_leaves_the_part_number_off_its_items() {
    let block = block();
    let pin = pin();
    let meshes = [
        mesh_of(&block, MeshResolution::Coarse),
        mesh_of(&pin, MeshResolution::Coarse),
    ];

    let entries = unzip(&three_mf::encode(&meshes, &bracket_properties()).unwrap());

    let model = String::from_utf8(entries[MODEL_PATH].clone()).unwrap();
    assert!(!model.contains("partnumber"));
}

#[test]
fn a_glb_keeps_the_model_properties_in_its_asset_extras() {
    let block = block();
    let meshes = [mesh_of(&block, MeshResolution::Coarse)];

    let glb_with = glb(&gltf::encode(&meshes, &bracket_properties()).unwrap());
    let glb_without = glb(&gltf::encode(&meshes, &ModelProperties::default()).unwrap());

    let extras = &glb_with.json["asset"]["extras"];
    assert_eq!(extras["title"], "Wall <bracket>");
    assert_eq!(extras["partNumber"], "BR-100");
    assert_eq!(extras["revision"], "C");
    assert_eq!(extras["author"], "Drafter");
    assert_eq!(extras["organisation"], "Workshop");
    assert_eq!(extras["description"], "Holds a shelf");
    assert!(extras.get("notes").is_none());
    assert!(glb_without.json["asset"].get("extras").is_none());
}

#[test]
fn an_obj_names_the_model_properties_in_its_header() {
    let block = block();
    let meshes = [mesh_of(&block, MeshResolution::Coarse)];

    let text =
        String::from_utf8(obj::encode(&meshes, None, &bracket_properties()).unwrap()).unwrap();
    let header: Vec<&str> = text
        .lines()
        .take_while(|line| line.starts_with('#'))
        .collect();

    assert_eq!(
        header[1..],
        [
            "# Title: Wall <bracket>",
            "# Part number: BR-100",
            "# Revision: C",
            "# Author: Drafter",
            "# Organisation: Workshop",
            "# Description: Holds a shelf",
        ]
    );
}

#[test]
fn a_step_export_names_its_product_and_header_from_the_model_properties() {
    let dir = TempDir::new().unwrap();
    let block = block();
    let path = dir.path().join("bracket.step");
    let bodies = [ExportBody {
        name: "Extrude 1",
        solid: &block,
        look: None,
        group: None,
        threads: &[],
        faces: &[],
    }];

    export_bodies(
        &path,
        ExportFormat::Step,
        &MeshOptions {
            resolution: MeshResolution::Standard,
            ..MeshOptions::default()
        },
        &bodies,
        &bracket_properties(),
        &CancelToken::never(),
    )
    .unwrap();
    let step = std::fs::read_to_string(&path).unwrap();

    assert!(step.contains("FILE_DESCRIPTION(('Holds a shelf'),'2;1');"));
    assert!(step.contains(",('Drafter'),('Workshop'),"));
    assert!(step.contains("=PRODUCT('BR-100','Wall <bracket>','Holds a shelf',("));
    assert!(step.contains("=PRODUCT_DEFINITION_FORMATION('C','',"));
    assert!(!step.contains("Never exported"));
}

#[test]
fn sketches_and_faces_nest_into_one_drawing() {
    let plate = rounded_plate();
    let mut sketch = Sketch::new(Plane::XY);
    sketch.add_line(Point2::ZERO, Point2::new(30.0, 0.0));
    sketch.add_circle(Point2::new(15.0, 10.0), 5.0);
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("mixed.dxf");
    let face = face_facing(&plate, Vector3::Z);
    let sheet = DrawingSheet {
        layout: SheetLayout::Nested(Nesting::new(200.0, 5.0, true).unwrap()),
        annotations: Annotations::Included,
        ..DrawingSheet::default()
    };

    let exported = export_drawing(
        &path,
        &[NamedSketch {
            name: "Bracket",
            sketch: &sketch,
        }],
        &[NamedFace {
            name: "Plate",
            solid: &plate,
            face,
        }],
        SketchFormat::Dxf,
        &sheet,
        &CancelToken::never(),
    )
    .unwrap();

    let outlined = outline::face_figure(&plate, face).unwrap().1;
    assert_eq!(exported.sketches.sketches, 1);
    assert_eq!(exported.sketches.curves, 2);
    assert_eq!(exported.faces.faces, 1);
    assert_eq!(exported.faces.curves, outlined.curves);
    assert_eq!(exported.too_wide, 0);
    let drawing = crate::parse_dxf(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(drawing.curves.len(), 2 + outlined.curves);
    assert!(drawing.layers.iter().any(|layer| layer == "0"));
    assert!(drawing.layers.iter().any(|layer| layer == "Outline"));
    assert!(
        drawing.notes.iter().any(|note| note.contains("2 texts")),
        "{:?}",
        drawing.notes
    );
    assert_eq!(
        export_drawing(
            &path,
            &[],
            &[],
            SketchFormat::Dxf,
            &sheet,
            &CancelToken::never()
        ),
        Err(ExportError::NoCurves)
    );
}
