use std::{path::Path, rc::Rc};

use caditor_document::{CancelToken, Import};
use caditor_geometry::Point3;
use caditor_kernel::{
    FacetedError, Interrupted, MAX_FILLED_HOLE_EDGES, MeshRepairs, TriangleMesh, check_interrupt,
    faceted_solids, interruptible,
};

use crate::{
    import::{
        ImportError, MAX_EXPANDED_OBJECTS, ensure_going,
        mesh_unit::{MeshUnit, UnitReading},
        model::{ImportedBody, ModelImport, canonical},
        xml::{self, Node, XmlError},
        zip_read::{self, EntryError},
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
const PRODUCTION_NAMESPACE: &str =
    "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const MAX_COMPONENT_DEPTH: usize = 16;
pub(super) const MAX_PLACED_OBJECTS: usize = MAX_EXPANDED_OBJECTS;
pub(super) const MAX_PLACED_VERTICES: usize = MAX_MODEL_ELEMENTS;
pub(super) const MAX_PLACED_TRIANGLES: usize = MAX_MODEL_ELEMENTS;
pub(super) const MAX_MODEL_ELEMENTS: usize = 4_000_000;
const LINES_BETWEEN_INTERRUPT_POLLS: usize = 4096;

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
    read_mesh_file_in(path, None, cancel)
}

pub fn read_mesh_file_in(
    path: &Path,
    chosen: Option<MeshUnit>,
    cancel: &CancelToken,
) -> Result<ModelImport, ImportError> {
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
        parse_mesh_in(&bytes, format, &source, &stem, chosen)
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
    parse_mesh_in(bytes, format, source, name, None)
}

pub fn parse_mesh_in(
    bytes: &[u8],
    format: MeshFormat,
    source: &str,
    name: &str,
    chosen: Option<MeshUnit>,
) -> Result<ModelImport, ImportError> {
    let mesh = match format {
        MeshFormat::Stl => read_stl(bytes)?,
        MeshFormat::Obj => read_obj(bytes)?,
        MeshFormat::ThreeMf => read_three_mf(bytes)?,
    };
    let unit = (format != MeshFormat::ThreeMf).then(|| UnitReading::of(&mesh, chosen));
    let (mesh, mut notes) = match &unit {
        Some(reading) => (reading.scale(mesh), vec![reading.note()]),
        None => (mesh, Vec::new()),
    };
    let built = faceted_solids(&mesh).map_err(|error| match error {
        FacetedError::NoTriangles => ImportError::DamagedMesh(format),
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
    Ok(ModelImport {
        bodies,
        notes,
        unit,
    })
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

fn read_stl(bytes: &[u8]) -> Result<TriangleMesh, ImportError> {
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
        return count
            .and_then(|count| read_binary_stl(bytes, count))
            .ok_or_else(|| damaged(MeshFormat::Stl));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| damaged(MeshFormat::Stl))?;
    if text.trim_start().starts_with("solid") {
        read_text_stl(text)
    } else {
        count
            .and_then(|count| read_binary_stl(bytes, count))
            .ok_or_else(|| damaged(MeshFormat::Stl))
    }
}

fn damaged(format: MeshFormat) -> ImportError {
    ImportError::DamagedMesh(format)
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

fn poll_between_lines(line: usize) -> Result<(), ImportError> {
    if line.is_multiple_of(LINES_BETWEEN_INTERRUPT_POLLS) {
        check_interrupt().map_err(|Interrupted| ImportError::Cancelled)?;
    }
    Ok(())
}

fn within_budget(vertices: usize, triangles: usize) -> Result<(), ImportError> {
    if vertices > MAX_PLACED_VERTICES || triangles > MAX_PLACED_TRIANGLES {
        return Err(ImportError::ModelTooManyElements);
    }
    Ok(())
}

fn read_text_stl(text: &str) -> Result<TriangleMesh, ImportError> {
    let mut mesh = TriangleMesh::default();
    let mut corners = Vec::with_capacity(3);
    for (line_number, line) in text.lines().enumerate() {
        poll_between_lines(line_number)?;

        let mut words = line.split_whitespace();
        match words.next() {
            Some("vertex") => {
                let values: Vec<f64> = words
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| damaged(MeshFormat::Stl))?;
                let [x, y, z] = values.as_slice() else {
                    return Err(damaged(MeshFormat::Stl));
                };
                corners.push(Point3::new(*x, *y, *z));
                within_budget(mesh.positions.len() + corners.len(), mesh.triangles.len())?;
            }
            Some("endloop") => {
                let start = mesh.positions.len();
                mesh.positions.append(&mut corners);
                let added = mesh.positions.len() - start;
                for fan in 1..added.saturating_sub(1) {
                    mesh.triangles.push([start, start + fan, start + fan + 1]);
                }
                within_budget(mesh.positions.len(), mesh.triangles.len())?;
            }
            _ => {}
        }
    }
    if mesh.triangles.is_empty() {
        return Err(damaged(MeshFormat::Stl));
    }
    Ok(mesh)
}

fn read_obj(bytes: &[u8]) -> Result<TriangleMesh, ImportError> {
    let text = String::from_utf8_lossy(bytes);
    let mut mesh = TriangleMesh::default();
    for (line_number, line) in text.lines().enumerate() {
        poll_between_lines(line_number)?;

        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let values: Vec<f64> = words
                    .take(3)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| damaged(MeshFormat::Obj))?;
                let [x, y, z] = values.as_slice() else {
                    return Err(damaged(MeshFormat::Obj));
                };
                mesh.positions.push(Point3::new(*x, *y, *z));
                within_budget(mesh.positions.len(), mesh.triangles.len())?;
            }
            Some("f") => {
                let count = mesh.positions.len();
                let fanned = words.clone().count().saturating_sub(2);
                within_budget(count, mesh.triangles.len().saturating_add(fanned))?;

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
                    .collect::<Option<_>>()
                    .ok_or_else(|| damaged(MeshFormat::Obj))?;
                let (first, rest) = corners
                    .split_first()
                    .ok_or_else(|| damaged(MeshFormat::Obj))?;
                for pair in rest.windows(2) {
                    if let [b, c] = pair {
                        mesh.triangles.push([*first, *b, *c]);
                    }
                }
            }
            _ => {}
        }
    }
    if mesh.triangles.is_empty() {
        return Err(damaged(MeshFormat::Obj));
    }
    Ok(mesh)
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

    fn in_units(self, scale: f64) -> Self {
        let mut values = self.0;
        for value in values.iter_mut().skip(9) {
            *value *= scale;
        }
        Self(values)
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
    let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf);
    let archive = zip_read::Archive::new(bytes).ok_or_else(damaged)?;
    let mut package = Package {
        archive,
        parts: Vec::new(),
        loaded: UntrustedMap::new(),
        bytes_left: usize::try_from(MAX_FILE_SIZE).unwrap_or(usize::MAX),
        elements_left: MAX_MODEL_ELEMENTS,
    };

    let model_path = match package.model_part()? {
        Some(target) => part_path("", &target)?,
        None => THREE_MF_MODEL.to_owned(),
    };
    let root = package.load(&model_path).map_err(|error| match error {
        ImportError::MissingModelPart { .. } => ImportError::NoModelInPackage,
        other => other,
    })?;
    let build = package
        .parts
        .get(root)
        .and_then(|part| part.build.clone())
        .ok_or_else(damaged)?;

    let mut placement = Placement {
        package,
        mesh: TriangleMesh::default(),
        placed_objects: 0,
        expanding: Vec::new(),
    };
    for item in build.iter() {
        let item = item.as_ref().map_err(Clone::clone)?;
        placement.add_reference(root, item, Transform::IDENTITY)?;
    }
    Ok(placement.mesh)
}

struct Package<'z> {
    archive: zip_read::Archive<'z>,
    parts: Vec<Part>,
    loaded: UntrustedMap<String, usize>,
    bytes_left: usize,
    elements_left: usize,
}

struct Part {
    name: String,
    objects: UntrustedMap<String, Object>,
    build: Option<References>,
}

struct Object {
    shape: Option<Result<Shape, ImportError>>,
    components: References,
}

type References = Rc<[Result<Reference, ImportError>]>;

struct Shape {
    positions: Vec<Point3>,
    triangles: Vec<[usize; 3]>,
}

struct Reference {
    part: Option<String>,
    object: String,
    transform: Transform,
}

impl Package<'_> {
    fn model_part(&mut self) -> Result<Option<String>, ImportError> {
        let relationships = match self.archive.read(THREE_MF_RELATIONSHIPS, self.bytes_left) {
            Ok(relationships) => relationships,
            Err(EntryError::TooLarge) => return Err(ImportError::UnpacksTooLarge),
            Err(EntryError::Missing | EntryError::Damaged) => return Ok(None),
        };
        self.bytes_left = self.bytes_left.saturating_sub(relationships.len());
        Ok(model_part(&relationships))
    }

    fn load(&mut self, path: &str) -> Result<usize, ImportError> {
        let key = path.to_ascii_lowercase();
        if let Some(&index) = self.loaded.get(key.as_str()) {
            return Ok(index);
        }
        let part = self.read_part(path)?;
        let index = self.parts.len();
        self.parts.push(part);
        self.loaded.insert(key, index);
        Ok(index)
    }

    fn read_part(&mut self, path: &str) -> Result<Part, ImportError> {
        let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf);
        let name = format!("/{path}");
        let bytes = self
            .archive
            .read(path, self.bytes_left)
            .map_err(|error| match error {
                EntryError::Missing => ImportError::MissingModelPart { part: name.clone() },
                EntryError::Damaged => damaged(),
                EntryError::TooLarge => ImportError::UnpacksTooLarge,
            })?;
        self.bytes_left = self.bytes_left.saturating_sub(bytes.len());

        let text = std::str::from_utf8(&bytes).map_err(|_| damaged())?;
        let tree = xml::parse(text, self.elements_left).map_err(|error| match error {
            XmlError::TooManyElements => ImportError::ModelTooManyElements,
            XmlError::TooDeep => ImportError::NestedTooDeep,
            XmlError::TooManyAttributes => ImportError::TooManyAttributes,
            XmlError::Damaged { .. } => damaged(),
            XmlError::Cancelled => ImportError::Cancelled,
        })?;
        self.elements_left = self.elements_left.saturating_sub(tree.element_count());
        if tree.damaged_at.is_some() {
            return Err(damaged());
        }

        let root = tree.root().ok_or_else(damaged)?;
        let scale = unit_scale(root.attribute("unit")).ok_or_else(damaged)?;
        let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
        let read = |node: Node<'_, '_>| read_reference(node, scale, directory);

        let mut objects = UntrustedMap::new();
        for object in root.descendants().filter(|node| node.name() == "object") {
            let Some(id) = object.attribute("id") else {
                continue;
            };
            if objects.contains_key(id) {
                continue;
            }
            let shape = object
                .children()
                .find(|child| child.name() == "mesh")
                .map(|shape| read_shape(shape, scale));
            let components = object
                .descendants()
                .filter(|child| child.name() == "component")
                .map(read)
                .collect();
            objects.insert(id.to_owned(), Object { shape, components });
        }
        let build = root
            .children()
            .find(|node| node.name() == "build")
            .map(|build| {
                build
                    .children()
                    .filter(|node| node.name() == "item")
                    .map(read)
                    .collect()
            });
        Ok(Part {
            name,
            objects,
            build,
        })
    }
}

fn read_reference(
    node: Node<'_, '_>,
    scale: f64,
    directory: &str,
) -> Result<Reference, ImportError> {
    let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf);
    let transform = Transform::parse(node.attribute("transform"))
        .ok_or_else(damaged)?
        .in_units(scale);
    let object = node.attribute("objectid").ok_or_else(damaged)?.to_owned();
    let part = node
        .attribute_in(PRODUCTION_NAMESPACE, "path")
        .map(|path| part_path(directory, path))
        .transpose()?;
    Ok(Reference {
        part,
        object,
        transform,
    })
}

fn read_shape(shape: Node<'_, '_>, scale: f64) -> Result<Shape, ImportError> {
    let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf);
    let coordinate = |vertex: Node<'_, '_>, name: &str| -> Result<f64, ImportError> {
        vertex
            .attribute(name)
            .and_then(|value| value.parse::<f64>().ok())
            .map(|value| value * scale)
            .ok_or_else(damaged)
    };
    let positions = shape
        .descendants()
        .filter(|child| child.name() == "vertex")
        .map(|vertex| {
            Ok(Point3::new(
                coordinate(vertex, "x")?,
                coordinate(vertex, "y")?,
                coordinate(vertex, "z")?,
            ))
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    let count = positions.len();
    let triangles = shape
        .descendants()
        .filter(|child| child.name() == "triangle")
        .map(|triangle| {
            let corner = |name: &str| -> Result<usize, ImportError> {
                triangle
                    .attribute(name)
                    .and_then(|index| index.parse::<usize>().ok())
                    .filter(|&index| index < count)
                    .ok_or_else(damaged)
            };
            Ok([corner("v1")?, corner("v2")?, corner("v3")?])
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(Shape {
        positions,
        triangles,
    })
}

fn part_path(directory: &str, reference: &str) -> Result<String, ImportError> {
    let outside = || ImportError::ModelPartOutsidePackage {
        part: reference.to_owned(),
    };
    let base = if reference.starts_with('/') {
        ""
    } else {
        directory
    };
    let mut segments = Vec::new();
    for segment in base.split('/').chain(reference.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop().ok_or_else(outside)?;
            }
            name => segments.push(name),
        }
    }
    Ok(segments.join("/"))
}

struct Placement<'z> {
    package: Package<'z>,
    mesh: TriangleMesh,
    placed_objects: usize,
    expanding: Vec<(usize, String)>,
}

impl Placement<'_> {
    fn add_reference(
        &mut self,
        part: usize,
        reference: &Reference,
        placed: Transform,
    ) -> Result<(), ImportError> {
        let target = match &reference.part {
            Some(path) => self.package.load(path)?,
            None => part,
        };
        self.add_object(target, &reference.object, reference.transform.then(placed))
    }

    fn add_object(&mut self, part: usize, id: &str, placed: Transform) -> Result<(), ImportError> {
        let damaged = || ImportError::DamagedMesh(MeshFormat::ThreeMf);
        check_interrupt().map_err(|Interrupted| ImportError::Cancelled)?;
        self.placed_objects += 1;
        if self.placed_objects > MAX_PLACED_OBJECTS {
            return Err(ImportError::ModelPlacesTooMuch);
        }
        let home = self.package.parts.get(part).ok_or_else(damaged)?;
        if self
            .expanding
            .iter()
            .any(|(expanding, object)| *expanding == part && object == id)
        {
            return Err(ImportError::ComponentCycle {
                object: id.to_owned(),
                part: home.name.clone(),
            });
        }
        if self.expanding.len() > MAX_COMPONENT_DEPTH {
            return Err(damaged());
        }

        let object = home.objects.get(id).ok_or_else(damaged)?;
        if let Some(shape) = &object.shape {
            let shape = shape.as_ref().map_err(Clone::clone)?;
            add_mesh(&mut self.mesh, shape, placed)?;
        }
        let components = Rc::clone(&object.components);

        self.expanding.push((part, id.to_owned()));
        for component in components.iter() {
            let component = component.as_ref().map_err(Clone::clone)?;
            self.add_reference(part, component, placed)?;
        }
        self.expanding.pop();
        Ok(())
    }
}

fn add_mesh(mesh: &mut TriangleMesh, shape: &Shape, placed: Transform) -> Result<(), ImportError> {
    let start = mesh.positions.len();
    let placed_vertices = start.saturating_add(shape.positions.len());
    let placed_triangles = mesh.triangles.len().saturating_add(shape.triangles.len());
    if placed_vertices > MAX_PLACED_VERTICES || placed_triangles > MAX_PLACED_TRIANGLES {
        return Err(ImportError::ModelPlacesTooMuch);
    }

    mesh.positions
        .extend(shape.positions.iter().map(|&point| placed.apply(point)));
    mesh.triangles.extend(
        shape
            .triangles
            .iter()
            .map(|triangle| triangle.map(|corner| start + corner)),
    );
    Ok(())
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
    Some(target.to_owned())
}
