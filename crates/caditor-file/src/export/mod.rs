mod annotation;
#[cfg(test)]
mod drawing_tests;
mod dxf;
mod figure;
mod gltf;
mod image;
mod obj;
mod outline;
mod sheet;
mod stl;
mod svg;
#[cfg(test)]
mod tests;
mod three_mf;
pub(crate) mod zip;

use std::{
    fs::File,
    io::{self, BufWriter, Write},
    panic::{self, AssertUnwindSafe},
    path::Path,
    time::SystemTime,
};

use caditor_document::{CancelToken, ModelProperties, ModelProperty, Rgb};
use caditor_geometry::{Aabb, Point3, Vector3};
use caditor_kernel::{FaceId, Mesh, SamplingTolerance, Solid, TessellationError, interruptible};
use caditor_sketch::Sketch;
use caditor_step::{StepBody, StepDetails, StepWritten, WriteError, write_step_detailed};

use self::{
    figure::{Figure, Layer, Shape},
    sheet::Part,
};
pub use self::{
    image::{ImageExportError, PNG_EXTENSION, PixelRows, PngExportError, RgbaImage, export_png},
    sheet::{Annotations, DrawingSheet, Nesting, SheetLayout},
    stl::StlEncoding,
};
use crate::{
    reason::WriteFailure,
    save::{replace_atomically, write_atomically},
};

const APPLICATION: &str = concat!("caditor ", env!("CARGO_PKG_VERSION"));
const SMALLEST_EXTENT: f64 = 1.0;
const STREAM_BUFFER: usize = 1 << 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExportFormat {
    #[default]
    Stl,
    ThreeMf,
    Obj,
    Gltf,
    Step,
}

impl ExportFormat {
    pub const ALL: [Self; 5] = [Self::Stl, Self::ThreeMf, Self::Obj, Self::Gltf, Self::Step];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::ThreeMf => "3mf",
            Self::Obj => "obj",
            Self::Gltf => "glb",
            Self::Step => STEP_EXTENSION,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::ThreeMf => "3MF",
            Self::Obj => "OBJ",
            Self::Gltf => "glTF",
            Self::Step => "STEP",
        }
    }

    pub fn is_mesh(self) -> bool {
        match self {
            Self::Stl | Self::ThreeMf | Self::Obj | Self::Gltf => true,
            Self::Step => false,
        }
    }

    pub fn matches(self, path: &Path) -> bool {
        let accepted: &[&str] = match self {
            Self::Step => &STEP_EXTENSIONS,
            Self::Stl | Self::ThreeMf | Self::Obj | Self::Gltf => &[self.extension()],
        };
        path.extension().is_some_and(|extension| {
            accepted
                .iter()
                .any(|accepted| extension.eq_ignore_ascii_case(accepted))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SketchFormat {
    #[default]
    Dxf,
    Svg,
}

impl SketchFormat {
    pub const ALL: [Self; 2] = [Self::Dxf, Self::Svg];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Dxf => "dxf",
            Self::Svg => "svg",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dxf => "DXF",
            Self::Svg => "SVG",
        }
    }

    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?;
        Self::ALL
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SketchExported {
    pub sketches: usize,
    pub curves: usize,
    pub points: usize,
    pub construction: usize,
    pub construction_left_out: usize,
    pub dimensions: usize,
    pub too_wide: usize,
}

impl SketchExported {
    fn add(&mut self, other: Self) {
        self.sketches += other.sketches;
        self.curves += other.curves;
        self.points += other.points;
        self.construction += other.construction;
        self.construction_left_out += other.construction_left_out;
        self.dimensions += other.dimensions;
    }

    fn drawn(&self) -> usize {
        self.curves + self.points + self.construction
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Construction {
    #[default]
    LeftOut,
    OnLayer,
}

#[derive(Debug, Clone, Copy)]
pub struct NamedSketch<'a> {
    pub name: &'a str,
    pub sketch: &'a Sketch,
}

pub fn export_sketch(
    path: &Path,
    sketch: &Sketch,
    format: SketchFormat,
    construction: Construction,
    cancel: &CancelToken,
) -> Result<SketchExported, ExportError> {
    let sheet = DrawingSheet {
        construction,
        ..DrawingSheet::default()
    };
    export_sketches(
        path,
        &[NamedSketch { name: "", sketch }],
        format,
        &sheet,
        cancel,
    )
}

pub fn export_sketches(
    path: &Path,
    sketches: &[NamedSketch<'_>],
    format: SketchFormat,
    sheet: &DrawingSheet,
    cancel: &CancelToken,
) -> Result<SketchExported, ExportError> {
    let mut exported = SketchExported::default();
    let mut drawn = Vec::with_capacity(sketches.len());
    for named in sketches {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let (figure, counted) = Figure::of_sketch(named.sketch, sheet.construction);
        exported.add(counted);
        drawn.push((named, figure));
    }
    if exported.drawn() == 0 {
        return Err(ExportError::NoCurves);
    }
    let text_height = sheet::text_height(drawn.iter().map(|(_, figure)| figure));
    let mut parts = Vec::with_capacity(drawn.len());
    for (named, mut figure) in drawn {
        if sheet.annotations == Annotations::Included
            && let Some(bounds) = figure.bounds()
        {
            let dimensions = annotation::dimensions(
                named.sketch,
                sheet.construction,
                text_height,
                bounds.center(),
            );
            exported.dimensions += dimensions.len();
            for dimension in dimensions {
                figure.push(Layer::Dimensions, Shape::Dimension(Box::new(dimension)));
            }
        }
        parts.push(Part {
            figure,
            label: named.name.to_owned(),
        });
    }
    let arranged = sheet::arranged(parts, sheet, text_height, cancel)?;
    exported.too_wide = arranged.too_wide;
    write_figure(path, &arranged.figure, format, cancel)?;
    Ok(exported)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FaceExported {
    pub faces: usize,
    pub loops: usize,
    pub curves: usize,
    pub approximated: usize,
    pub too_wide: usize,
}

impl FaceExported {
    fn add(&mut self, other: Self) {
        self.faces += other.faces;
        self.loops += other.loops;
        self.curves += other.curves;
        self.approximated += other.approximated;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NamedFace<'a> {
    pub name: &'a str,
    pub solid: &'a Solid,
    pub face: FaceId,
}

pub fn export_face(
    path: &Path,
    solid: &Solid,
    face: FaceId,
    format: SketchFormat,
    cancel: &CancelToken,
) -> Result<FaceExported, ExportError> {
    export_faces(
        path,
        &[NamedFace {
            name: "",
            solid,
            face,
        }],
        format,
        &DrawingSheet::default(),
        cancel,
    )
}

pub fn export_faces(
    path: &Path,
    faces: &[NamedFace<'_>],
    format: SketchFormat,
    sheet: &DrawingSheet,
    cancel: &CancelToken,
) -> Result<FaceExported, ExportError> {
    let mut exported = FaceExported::default();
    let mut parts = Vec::with_capacity(faces.len());
    for named in faces {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let outlined = panic::catch_unwind(AssertUnwindSafe(|| {
            outline::face_figure(named.solid, named.face)
        }));
        let (figure, counted) = outlined.unwrap_or_else(|_| {
            log::error!("outlining a face for export panicked");
            Err(ExportError::Encoding)
        })?;
        exported.add(counted);
        parts.push(Part {
            figure,
            label: named.name.to_owned(),
        });
    }
    let text_height = sheet::text_height(parts.iter().map(|part| &part.figure));
    let arranged = sheet::arranged(parts, sheet, text_height, cancel)?;
    exported.too_wide = arranged.too_wide;
    write_figure(path, &arranged.figure, format, cancel)?;
    Ok(exported)
}

fn write_figure(
    path: &Path,
    figure: &Figure,
    format: SketchFormat,
    cancel: &CancelToken,
) -> Result<(), ExportError> {
    let text = match format {
        SketchFormat::Dxf => dxf::encode(figure),
        SketchFormat::Svg => svg::encode(figure)?,
    };
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    write_atomically(path, text.as_bytes())
        .map_err(|error| ExportError::Writing(WriteFailure::of(&error)))
}

pub const STEP_EXTENSION: &str = "step";
pub const STEP_EXTENSIONS: [&str; 2] = ["step", "stp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MeshResolution {
    Coarse,
    #[default]
    Standard,
    Fine,
}

impl MeshResolution {
    pub const ALL: [Self; 3] = [Self::Coarse, Self::Standard, Self::Fine];

    pub fn name(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Standard => "Standard",
            Self::Fine => "Fine",
        }
    }

    fn chord_fraction(self) -> f64 {
        match self {
            Self::Coarse => 1e-3,
            Self::Standard => 2.5e-4,
            Self::Fine => 5e-5,
        }
    }

    fn angle_degrees(self) -> f64 {
        match self {
            Self::Coarse => 20.0,
            Self::Standard => 10.0,
            Self::Fine => 5.0,
        }
    }

    pub fn tolerance<'a>(self, solids: impl IntoIterator<Item = &'a Solid>) -> SamplingTolerance {
        self.tolerance_within(solids.into_iter().filter_map(Solid::bounding_box))
    }

    pub fn tolerance_within(self, bounds: impl IntoIterator<Item = Aabb>) -> SamplingTolerance {
        let extent = extent(bounds);
        SamplingTolerance::new(
            extent * self.chord_fraction(),
            self.angle_degrees().to_radians(),
        )
        .unwrap_or_else(|| SamplingTolerance::for_extent(extent))
    }
}

fn extent(bounds: impl IntoIterator<Item = Aabb>) -> f64 {
    bounds
        .into_iter()
        .map(|bounds| bounds.diagonal())
        .filter(|diagonal| diagonal.is_finite())
        .fold(SMALLEST_EXTENT, f64::max)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshOptions<'a> {
    pub resolution: MeshResolution,
    pub stl: StlEncoding,
    pub thumbnail: Option<RgbaImage<'a>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExportBody<'a> {
    pub name: &'a str,
    pub solid: &'a Solid,
    pub look: Option<Look<'a>>,
    pub group: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look<'a> {
    pub colour: Rgb,
    pub opacity: Option<u8>,
    pub material: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    pub bodies: usize,
    pub triangles: Option<usize>,
    pub left_out: Vec<ExportError>,
    pub moved: Option<Vector3>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    #[error("there are no bodies to export")]
    Empty,
    #[error("the export was cancelled")]
    Cancelled,
    #[error("the sketch has no curves or points to export")]
    NoCurves,
    #[error("the face is curved; only flat faces export as drawings")]
    FaceNotFlat,
    #[error("the face is no longer part of the body")]
    FaceMissing,
    #[error(
        "the body of “{0}” could not be turned into triangles at this resolution; try another \
         resolution"
    )]
    Meshing(String),
    #[error(
        "the body of “{0}” would need more points than caditor meshes at this resolution; try a \
         coarser resolution"
    )]
    TooFine(String),
    #[error("the model has more triangles than the file format can hold; try a coarser resolution")]
    TooLarge,
    #[error("the model could not be converted to the file format")]
    Encoding,
    #[error("{0}")]
    Step(WriteError),
    #[error("{0}")]
    Writing(WriteFailure),
    #[error("the background worker could not start")]
    WorkerUnavailable,
}

pub const EXPORTED_PROPERTIES: [ModelProperty; 6] = [
    ModelProperty::Title,
    ModelProperty::PartNumber,
    ModelProperty::Revision,
    ModelProperty::Author,
    ModelProperty::Organisation,
    ModelProperty::Description,
];

fn exported_properties(
    properties: &ModelProperties,
) -> impl Iterator<Item = (ModelProperty, &str)> {
    EXPORTED_PROPERTIES
        .into_iter()
        .map(|property| (property, properties.get(property)))
        .filter(|(_, value)| !value.is_empty())
}

pub fn export_bodies(
    path: &Path,
    format: ExportFormat,
    options: &MeshOptions<'_>,
    bodies: &[ExportBody<'_>],
    properties: &ModelProperties,
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    if bodies.is_empty() {
        return Err(ExportError::Empty);
    }
    if !format.is_mesh() {
        return export_step(path, bodies, properties, cancel);
    }
    let tolerance = options
        .resolution
        .tolerance(bodies.iter().map(|body| body.solid));
    let (meshes, left_out) = tessellate_all(bodies, cancel, |body| {
        MeshBody::tessellate(body, &tolerance, cancel)
    })?;
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    let exported = |moved| Exported {
        bodies: meshes.len(),
        triangles: Some(meshes.iter().map(|mesh| mesh.triangles.len()).sum()),
        left_out: left_out.clone(),
        moved,
    };
    if format == ExportFormat::Stl {
        let moved = match options.stl {
            StlEncoding::Binary => stl::offset(&meshes),
            StlEncoding::Text => None,
        };
        write_streamed(path, |out| match options.stl {
            StlEncoding::Binary => stl::write_binary(out, &meshes, moved, cancel),
            StlEncoding::Text => stl::write_text(out, &meshes, cancel),
        })?;
        return Ok(exported(moved));
    }
    if format == ExportFormat::ThreeMf {
        let thumbnail = options
            .thumbnail
            .and_then(|image| match image::encode_png(image) {
                Ok(png) => Some(png),
                Err(error) => {
                    log::warn!("the 3MF thumbnail was left out: {error}");
                    None
                }
            });
        write_streamed(path, |out| {
            three_mf::write(out, &meshes, properties, thumbnail.as_deref(), cancel)
        })?;
        return Ok(exported(None));
    }
    let library = (format == ExportFormat::Obj)
        .then(|| obj::Library::beside(path, &meshes))
        .flatten();
    let contents = encode(format, &meshes, library.as_ref(), properties)?;
    let materials = match library {
        Some(library) => Some((library.path, obj::encode_library(&meshes)?)),
        None => None,
    };
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    if let Some((library, materials)) = materials {
        write_atomically(&library, &materials)
            .map_err(|error| ExportError::Writing(WriteFailure::of(&error)))?;
    }
    write_atomically(path, &contents)
        .map_err(|error| ExportError::Writing(WriteFailure::of(&error)))?;
    Ok(exported(None))
}

fn writing(error: io::Error) -> ExportError {
    ExportError::Writing(WriteFailure::of(&error))
}

fn write_streamed(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<&mut File>) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    let mut failure = None;
    let written = replace_atomically(path, |file| {
        let mut out = BufWriter::with_capacity(STREAM_BUFFER, file);
        match write(&mut out) {
            Ok(()) => out.flush(),
            Err(error) => {
                failure = Some(error);
                Err(io::Error::from(io::ErrorKind::Interrupted))
            }
        }
    });
    match (failure, written) {
        (Some(error), _) => Err(error),
        (None, Err(error)) => Err(writing(error)),
        (None, Ok(())) => Ok(()),
    }
}

fn tessellate_all<'a>(
    bodies: &[ExportBody<'a>],
    cancel: &CancelToken,
    tessellate: impl Fn(&ExportBody<'a>) -> Result<MeshBody<'a>, ExportError>,
) -> Result<(Vec<MeshBody<'a>>, Vec<ExportError>), ExportError> {
    let mut meshes = Vec::with_capacity(bodies.len());
    let mut left_out = Vec::new();
    for body in bodies {
        if cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        match tessellate(body) {
            Ok(mesh) => meshes.push(mesh),
            Err(ExportError::Cancelled) => return Err(ExportError::Cancelled),
            Err(error) => left_out.push(error),
        }
    }
    match left_out.first() {
        Some(error) if meshes.is_empty() => Err(error.clone()),
        _ => Ok((meshes, left_out)),
    }
}

fn export_step(
    path: &Path,
    bodies: &[ExportBody<'_>],
    properties: &ModelProperties,
    cancel: &CancelToken,
) -> Result<Exported, ExportError> {
    let step_bodies: Vec<StepBody<'_>> = bodies
        .iter()
        .map(|body| StepBody {
            name: body.name,
            solid: body.solid,
            colour: body
                .look
                .map(|look| [look.colour.red, look.colour.green, look.colour.blue]),
            opacity: body.look.and_then(|look| look.opacity),
            layer: body.group,
        })
        .collect();
    let model_name = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let details = StepDetails {
        title: &properties.title,
        part_number: &properties.part_number,
        revision: &properties.revision,
        description: &properties.description,
        author: &properties.author,
        organisation: &properties.organisation,
    };
    let written = panic::catch_unwind(AssertUnwindSafe(|| {
        interruptible(cancel.interrupt(), || {
            write_step_detailed(&step_bodies, &model_name, &details, SystemTime::now())
        })
    }));
    let StepWritten { text, left_out } = match written {
        Ok(Err(_)) if cancel.is_cancelled() => return Err(ExportError::Cancelled),
        Ok(Ok(written)) => written,
        Ok(Err(WriteError::Empty)) => return Err(ExportError::Empty),
        Ok(Err(error)) => return Err(ExportError::Step(error)),
        Err(_) => {
            log::error!("writing STEP panicked");
            return Err(ExportError::Encoding);
        }
    };
    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    write_atomically(path, text.as_bytes())
        .map_err(|error| ExportError::Writing(WriteFailure::of(&error)))?;
    Ok(Exported {
        bodies: bodies.len() - left_out.len(),
        triangles: None,
        left_out: left_out
            .into_iter()
            .map(|(_, error)| ExportError::Step(error))
            .collect(),
        moved: None,
    })
}

fn encode(
    format: ExportFormat,
    bodies: &[MeshBody<'_>],
    library: Option<&obj::Library>,
    properties: &ModelProperties,
) -> Result<Vec<u8>, ExportError> {
    match format {
        ExportFormat::Stl | ExportFormat::ThreeMf | ExportFormat::Step => {
            Err(ExportError::Encoding)
        }
        ExportFormat::Obj => obj::encode(bodies, library, properties),
        ExportFormat::Gltf => gltf::encode(bodies, properties),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MeshBody<'a> {
    name: &'a str,
    look: Option<Look<'a>>,
    positions: Vec<Point3>,
    triangles: Vec<[u32; 3]>,
}

impl<'a> MeshBody<'a> {
    fn tessellate(
        body: &ExportBody<'a>,
        tolerance: &SamplingTolerance,
        cancel: &CancelToken,
    ) -> Result<Self, ExportError> {
        let meshing = ExportError::Meshing(body.name.to_owned());
        let tessellated = panic::catch_unwind(AssertUnwindSafe(|| {
            interruptible(cancel.interrupt(), || body.solid.tessellate(tolerance))
        }));
        match tessellated {
            Ok(Ok(mesh)) => Self::compact(body.name, &mesh)
                .map(|compacted| Self {
                    look: body.look,
                    ..compacted
                })
                .ok_or(meshing),
            Ok(Err(TessellationError::Cancelled(_))) => Err(ExportError::Cancelled),
            Ok(Err(TessellationError::TooLarge)) => Err(ExportError::TooFine(body.name.to_owned())),
            Ok(Err(error)) => {
                log::warn!("exporting {} failed: {error}", body.name);
                Err(meshing)
            }
            Err(_) => {
                log::error!("meshing {} for export panicked", body.name);
                Err(meshing)
            }
        }
    }

    fn compact(name: &'a str, mesh: &Mesh) -> Option<Self> {
        let mut remap = vec![None; mesh.positions().len()];
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for triangle in mesh.position_triangles() {
            let [a, b, c] = triangle;
            if a == b || b == c || c == a {
                continue;
            }
            let mut compacted = [0; 3];
            for (slot, index) in compacted.iter_mut().zip(triangle) {
                let entry = remap.get_mut(index as usize)?;
                *slot = match *entry {
                    Some(existing) => existing,
                    None => {
                        let next = u32::try_from(positions.len()).ok()?;
                        positions.push(mesh.position(index)?);
                        *entry = Some(next);
                        next
                    }
                };
            }
            triangles.push(compacted);
        }
        Some(Self {
            name,
            look: None,
            positions,
            triangles,
        })
    }

    fn corners(&self) -> impl Iterator<Item = [Point3; 3]> + '_ {
        self.triangles.iter().filter_map(|triangle| {
            let [a, b, c] = triangle.map(|index| self.positions.get(index as usize).copied());
            Some([a?, b?, c?])
        })
    }
}
