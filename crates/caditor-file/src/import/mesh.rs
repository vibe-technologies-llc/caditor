use std::path::Path;

use caditor_document::{CancelToken, Import};
use caditor_geometry::Point3;
use caditor_kernel::{
    FacetedError, Interrupted, MAX_FILLED_HOLE_EDGES, MeshRepairs, TriangleMesh, check_interrupt,
    faceted_solids, interruptible,
};

use crate::{
    import::{
        ImportError, MAX_EXPANDED_OBJECTS, ensure_going,
        model::{ImportedBody, ModelImport, canonical},
        xml::{self, Node, XmlError},
        zip_read,
    },
    read::{MAX_FILE_SIZE, read_file},
    reason::ReadFailure,
    untrusted::UntrustedMap,
};

pub const MESH_IMPORT_EXTENSIONS: [&str; 3] = ["stl", "obj", "3mf"];

const STL_HEADER: usize = 80;
const STL_TRIANGLE: usize = 50;
const THREE_MF_MODEL: &str = "3D/3dmodel.model";
const THREE_MF_RELATIONSHIPS: &str = "_rels/.rels";
const THREE_MF_MODEL_TYPE: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
const MAX_COMPONENT_DEPTH: usize = 16;
pub(super) const MAX_PLACED_OBJECTS: usize = MAX_EXPANDED_OBJECTS;
pub(super) const MAX_PLACED_VERTICES: usize = MAX_MODEL_ELEMENTS;
pub(super) const MAX_PLACED_TRIANGLES: usize = MAX_MODEL_ELEMENTS;
pub(super) const MAX_MODEL_ELEMENTS: usize = 4_000_000;
const NO_UNITS_NOTE: &str = "The file gives no units, so its sizes were read as millimetres; if the \
                             part comes out the wrong size, scale it with Scale body.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshFormat {
    Stl,
    Obj,
    ThreeMf,
}

impl MeshFormat {
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "stl" => Some(Self::Stl),
            "obj" => Some(Self::Obj),
            "3mf" => Some(Self::ThreeMf),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::Obj => "OBJ",
            Self::ThreeMf => "3MF",
        }
    }
}

pub fn read_mesh_file(path: &Path, cancel: &CancelToken) -> Result<ModelImport, ImportError> {
    let format = MeshFormat::of(path).ok_or(ImportError::NotMesh)?;
    let bytes = read_file(path).map_err(|error| ImportError::Reading(ReadFailure::of(&error)))?;
    ensure_going(cancel)?;
    let source = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().trim().to_owned());
    let imported = interruptible(cancel.interrupt(), || {
        parse_mesh(&bytes, format, &source, &stem)
    });
    ensure_going(cancel)?;
    imported
}

pub fn parse_mesh(
    bytes: &[u8],
    format: MeshFormat,
    source: &str,
    name: &str,
) -> Result<ModelImport, ImportError> {
    let damaged = ImportError::DamagedMesh(format.name());
    let (mesh, mut notes) = match format {
        MeshFormat::Stl => (
            read_stl(bytes).ok_or(damaged)?,
            vec![NO_UNITS_NOTE.to_owned()],
        ),
        MeshFormat::Obj => (
            read_obj(bytes).ok_or(damaged)?,
            vec![NO_UNITS_NOTE.to_owned()],
        ),
        MeshFormat::ThreeMf => (read_three_mf(bytes)?, Vec::new()),
    };
    let built = faceted_solids(&mesh).map_err(|error| match error {
        FacetedError::NoTriangles => ImportError::DamagedMesh(format.name()),
        FacetedError::NotClosed { open_edges } => ImportError::MeshNotClosed { open_edges },
        FacetedError::TooDetailed { faces } => ImportError::MeshTooDetailed { faces },
        FacetedError::Build(error) => {
            log::warn!("a mesh could not be built into a solid: {error}");
            ImportError::MeshNotSolid
        }
        FacetedError::Cancelled(_) => ImportError::Cancelled,
    })?;
    notes.extend(repair_notes(&built.repairs));
    let name = if name.is_empty() { "Mesh" } else { name };
    let mut bodies = Vec::with_capacity(built.solids.len());
    for solid in &built.solids {
        let Some(lumps) = canonical(name, solid) else {
            notes.push(
                "A part of it was read but could not be stored in the model, so it was \
                        left out."
                    .to_owned(),
            );
            continue;
        };
        bodies.extend(lumps.into_iter().map(|(stored, step)| ImportedBody {
            name: name.to_owned(),
            import: Import::new(source, stored, step),
            colour: None,
            opacity: None,
            finish: None,
            group: None,
            faces: Vec::new(),
        }));
    }
    if bodies.is_empty() {
        return Err(ImportError::NothingStorable);
    }
    Ok(ModelImport { bodies, notes })
}

fn repair_notes(repairs: &MeshRepairs) -> Vec<String> {
    let mut notes = Vec::new();
    let mut say = |count: usize, one: &str, many: &str| {
        if count == 1 {
            notes.push(one.to_owned());
        } else if count > 1 {
            notes.push(many.replace("{}", &count.to_string()));
        }
    };
    say(
        repairs.degenerate,
        "1 triangle with no area was left out.",
        "{} triangles with no area were left out.",
    );
    say(
        repairs.duplicate,
        "1 repeated triangle was left out.",
        "{} repeated triangles were left out.",
    );
    say(
        repairs.flipped,
        "1 triangle facing the wrong way was turned around.",
        "{} triangles facing the wrong way were turned around.",
    );
    say(
        repairs.inverted_shells,
        "1 part was inside out and was turned the right way.",
        "{} parts were inside out and were turned the right way.",
    );
    say(
        repairs.filled_holes,
        "1 small hole in the surface was closed.",
        "{} small holes in the surface were closed.",
    );
    say(
        repairs.open_shells,
        &format!(
            "1 part has a gap of more than {MAX_FILLED_HOLE_EDGES} edges or edges shared by more \
             than two triangles, so it encloses no solid and was left out."
        ),
        &format!(
            "{{}} parts have gaps of more than {MAX_FILLED_HOLE_EDGES} edges or edges shared by \
             more than two triangles, so they enclose no solid and were left out."
        ),
    );
    say(
        repairs.tangled_shells,
        "1 part folds through itself or encloses nothing, so it was left out.",
        "{} parts fold through themselves or enclose nothing, so they were left out.",
    );
    say(
        repairs.sealed_voids,
        "1 sealed hollow inside a part was left out, so the part is solid there.",
        "{} sealed hollows inside parts were left out, so the parts are solid there.",
    );
    say(
        repairs.kept_triangles,
        "1 part keeps its triangles as faces, since its flat areas could not be joined exactly.",
        "{} parts keep their triangles as faces, since their flat areas could not be joined \
         exactly.",
    );
    notes
}

fn read_stl(bytes: &[u8]) -> Option<TriangleMesh> {
    let count = bytes
        .get(STL_HEADER..STL_HEADER + 4)
        .and_then(|count| <[u8; 4]>::try_from(count).ok())
        .map(u32::from_le_bytes)
        .and_then(|count| usize::try_from(count).ok());
    let binary = count.is_some_and(|count| {
        count
            .checked_mul(STL_TRIANGLE)
            .and_then(|size| size.checked_add(STL_HEADER + 4))
            == Some(bytes.len())
    });
    if binary {
        return read_binary_stl(bytes, count?);
    }
    let text = std::str::from_utf8(bytes).ok()?;
    if text.trim_start().starts_with("solid") {
        read_text_stl(text)
    } else {
        read_binary_stl(bytes, count?)
    }
}

fn read_binary_stl(bytes: &[u8], count: usize) -> Option<TriangleMesh> {
    let body = bytes.get(STL_HEADER + 4..)?;
    let mut mesh = TriangleMesh::default();
    for record in body.as_chunks::<STL_TRIANGLE>().0.iter().take(count) {
        let float = |at: usize| -> Option<f64> {
            let raw = <[u8; 4]>::try_from(record.get(at..at + 4)?).ok()?;
            Some(f64::from(f32::from_le_bytes(raw)))
        };
        let start = mesh.positions.len();
        for corner in 0..3 {
            let at = 12 + corner * 12;
            mesh.positions
                .push(Point3::new(float(at)?, float(at + 4)?, float(at + 8)?));
        }
        mesh.triangles.push([start, start + 1, start + 2]);
    }
    (!mesh.triangles.is_empty()).then_some(mesh)
}

fn read_text_stl(text: &str) -> Option<TriangleMesh> {
    let mut mesh = TriangleMesh::default();
    let mut corners = Vec::with_capacity(3);
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("vertex") => {
                let values: Vec<f64> = words.map(str::parse).collect::<Result<_, _>>().ok()?;
                let [x, y, z] = values.as_slice() else {
                    return None;
                };
                corners.push(Point3::new(*x, *y, *z));
            }
            Some("endloop") => {
                let start = mesh.positions.len();
                mesh.positions.append(&mut corners);
                let added = mesh.positions.len() - start;
                for fan in 1..added.saturating_sub(1) {
                    mesh.triangles.push([start, start + fan, start + fan + 1]);
                }
            }
            _ => {}
        }
    }
    (!mesh.triangles.is_empty()).then_some(mesh)
}

fn read_obj(bytes: &[u8]) -> Option<TriangleMesh> {
    let text = String::from_utf8_lossy(bytes);
    let mut mesh = TriangleMesh::default();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let values: Vec<f64> = words
                    .take(3)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .ok()?;
                let [x, y, z] = values.as_slice() else {
                    return None;
                };
                mesh.positions.push(Point3::new(*x, *y, *z));
            }
            Some("f") => {
                let count = mesh.positions.len();
                let corners: Vec<usize> = words
                    .map(|word| {
                        let index: i64 = word.split('/').next()?.parse().ok()?;
                        let resolved = if index < 0 {
                            i64::try_from(count).ok()? + index
                        } else {
                            index - 1
                        };
                        usize::try_from(resolved)
                            .ok()
                            .filter(|index| *index < count)
                    })
                    .collect::<Option<_>>()?;
                let (first, rest) = corners.split_first()?;
                for pair in rest.windows(2) {
                    if let [b, c] = pair {
                        mesh.triangles.push([*first, *b, *c]);
                    }
                }
            }
            _ => {}
        }
    }
    (!mesh.triangles.is_empty()).then_some(mesh)
}

#[derive(Clone, Copy)]
struct Transform([f64; 12]);

impl Transform {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);

    fn parse(text: Option<&str>) -> Option<Self> {
        let Some(text) = text else {
            return Some(Self::IDENTITY);
        };
        let values: Vec<f64> = text
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        <[f64; 12]>::try_from(values).ok().map(Self)
    }

    fn apply(self, point: Point3) -> Point3 {
        let [m00, m01, m02, m10, m11, m12, m20, m21, m22, m30, m31, m32] = self.0;
        Point3::new(
            point.x * m00 + point.y * m10 + point.z * m20 + m30,
            point.x * m01 + point.y * m11 + point.z * m21 + m31,
            point.x * m02 + point.y * m12 + point.z * m22 + m32,
        )
    }

    fn then(self, outer: Self) -> Self {
        let [m00, m01, m02, m10, m11, m12, m20, m21, m22, m30, m31, m32] = self.0;
        let row = |x: f64, y: f64, z: f64| {
            let moved = outer.apply(Point3::new(x, y, z)) - outer.apply(Point3::ZERO);
            [moved.x, moved.y, moved.z]
        };
        let [a, b, c] = row(m00, m01, m02);
        let [d, e, f] = row(m10, m11, m12);
        let [g, h, i] = row(m20, m21, m22);
        let origin = outer.apply(Point3::new(m30, m31, m32));
        Self([a, b, c, d, e, f, g, h, i, origin.x, origin.y, origin.z])
    }
}

fn unit_scale(unit: Option<&str>) -> Option<f64> {
    match unit.unwrap_or("millimeter") {
        "micron" => Some(0.001),
        "millimeter" => Some(1.0),
        "centimeter" => Some(10.0),
        "inch" => Some(25.4),
        "foot" => Some(304.8),
        "meter" => Some(1000.0),
        _ => None,
    }
}

fn read_three_mf(bytes: &[u8]) -> Result<TriangleMesh, ImportError> {
    let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf.name());
    let limit = usize::try_from(MAX_FILE_SIZE).unwrap_or(usize::MAX);
    let archive = zip_read::Archive::new(bytes).ok_or_else(damaged)?;
    let model_path = archive
        .read(THREE_MF_RELATIONSHIPS, limit)
        .and_then(|relationships| model_part(&relationships))
        .unwrap_or_else(|| THREE_MF_MODEL.to_owned());
    let model = archive
        .read(&model_path, limit)
        .ok_or(ImportError::NoModelInPackage)?;
    let text = std::str::from_utf8(&model).map_err(|_| damaged())?;
    let tree = xml::parse(text, MAX_MODEL_ELEMENTS).map_err(|error| match error {
        XmlError::TooManyElements => ImportError::ModelTooManyElements,
        XmlError::Damaged { .. } => damaged(),
    })?;
    if tree.damaged_at.is_some() {
        return Err(damaged());
    }
    let root = tree.root().ok_or_else(damaged)?;
    let scale = unit_scale(root.attribute("unit")).ok_or_else(damaged)?;
    let mut objects = UntrustedMap::new();
    for object in root.descendants().filter(|node| node.name() == "object") {
        if let Some(id) = object.attribute("id") {
            objects.entry(id).or_insert(object);
        }
    }
    let mut placement = Placement {
        objects,
        mesh: TriangleMesh::default(),
        placed_objects: 0,
    };
    let build = root
        .children()
        .find(|node| node.name() == "build")
        .ok_or_else(damaged)?;
    for item in build.children().filter(|node| node.name() == "item") {
        let id = item.attribute("objectid").ok_or_else(damaged)?;
        let transform = Transform::parse(item.attribute("transform")).ok_or_else(damaged)?;
        placement.add_object(id, transform, 0)?;
    }
    let mut mesh = placement.mesh;
    for position in &mut mesh.positions {
        *position *= scale;
    }
    Ok(mesh)
}

struct Placement<'a, 't> {
    objects: UntrustedMap<&'a str, Node<'a, 't>>,
    mesh: TriangleMesh,
    placed_objects: usize,
}

impl<'a, 't: 'a> Placement<'a, 't> {
    fn add_object(&mut self, id: &str, placed: Transform, depth: usize) -> Result<(), ImportError> {
        let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf.name());
        check_interrupt().map_err(|Interrupted| ImportError::Cancelled)?;
        self.placed_objects += 1;
        if self.placed_objects > MAX_PLACED_OBJECTS {
            return Err(ImportError::ModelPlacesTooMuch);
        }
        if depth > MAX_COMPONENT_DEPTH {
            return Err(damaged());
        }
        let node = *self.objects.get(id).ok_or_else(damaged)?;
        if let Some(shape) = node.children().find(|child| child.name() == "mesh") {
            self.add_mesh(shape, placed)?;
        }
        for component in node
            .descendants()
            .filter(|child| child.name() == "component")
        {
            let inner = Transform::parse(component.attribute("transform")).ok_or_else(damaged)?;
            let objectid = component.attribute("objectid").ok_or_else(damaged)?;
            self.add_object(objectid, inner.then(placed), depth + 1)?;
        }
        Ok(())
    }

    fn add_mesh(&mut self, shape: Node<'a, 't>, placed: Transform) -> Result<(), ImportError> {
        let vertices = || shape.descendants().filter(|child| child.name() == "vertex");
        let triangles = || {
            shape
                .descendants()
                .filter(|child| child.name() == "triangle")
        };
        let mesh = &mut self.mesh;
        let start = mesh.positions.len();
        let count = vertices().count();

        let placed_vertices = start.saturating_add(count);
        let placed_triangles = mesh.triangles.len().saturating_add(triangles().count());
        if placed_vertices > MAX_PLACED_VERTICES || placed_triangles > MAX_PLACED_TRIANGLES {
            return Err(ImportError::ModelPlacesTooMuch);
        }

        let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf.name());
        let coordinate = |vertex: Node<'_, '_>, name: &str| -> Result<f64, ImportError> {
            vertex
                .attribute(name)
                .and_then(|value| value.parse().ok())
                .ok_or_else(damaged)
        };
        for vertex in vertices() {
            let point = Point3::new(
                coordinate(vertex, "x")?,
                coordinate(vertex, "y")?,
                coordinate(vertex, "z")?,
            );
            mesh.positions.push(placed.apply(point));
        }
        for triangle in triangles() {
            let corner = |name: &str| -> Result<usize, ImportError> {
                triangle
                    .attribute(name)
                    .and_then(|index| index.parse::<usize>().ok())
                    .filter(|&index| index < count)
                    .map(|index| start + index)
                    .ok_or_else(damaged)
            };
            mesh.triangles
                .push([corner("v1")?, corner("v2")?, corner("v3")?]);
        }
        Ok(())
    }
}

fn model_part(relationships: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(relationships).ok()?;
    let tree = xml::parse(text, MAX_MODEL_ELEMENTS).ok()?;
    let target = tree
        .nodes()
        .find(|node| {
            node.name() == "Relationship" && node.attribute("Type") == Some(THREE_MF_MODEL_TYPE)
        })?
        .attribute("Target")?;
    Some(target.trim_start_matches('/').to_owned())
}
