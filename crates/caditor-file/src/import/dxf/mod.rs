mod code_page;
mod flatten;
mod geometry;
mod hatch;
mod mline;
mod outline;
mod pairs;

use std::{
    collections::{BTreeMap, BTreeSet},
    f64::consts::TAU,
    rc::Rc,
};

use caditor_geometry::{Point2, Point3, Vector3};

use crate::import::{
    Drawing, ImportError, MAX_DRAWING_CURVES, MAX_DRAWING_POINTS, MAX_EXPANDED_OBJECTS,
    dxf::{
        flatten::flatten,
        geometry::{Affine, FitPoints, Nurbs, Shape, conic_arc},
        hatch::boundaries,
        mline::mline,
        outline::{face_outline, filled_outline},
        pairs::{Pair, read_pairs},
    },
};

const MAX_BLOCK_DEPTH: usize = 16;
const DEFAULT_LAYER: &str = "0";
const BULGE_EPSILON: f64 = 1e-12;
const CLOSED: i64 = 1;
const POLYLINE_3D: i64 = 8;
const POLYLINE_MESH: i64 = 16 | 64;
const SPLINE_FRAME_VERTEX: i64 = 16;
const FROZEN_LAYER: i64 = 1;
const EXTERNAL_BLOCK: i64 = 4 | 32;
const PAPER_SPACE: i64 = 1;
const INVISIBLE: i64 = 1;
const SHOWN: bool = false;

pub fn parse_dxf(bytes: &[u8]) -> Result<Drawing, ImportError> {
    if !looks_like_dxf(bytes) {
        return Err(ImportError::NotDxf);
    }
    let records = records(read_pairs(bytes)?)?;
    let file = DxfFile::read(&records)?;
    let mut interpreter = Interpreter::new(&file);
    let entities = prepare(&file, &items(&file.entities));
    interpreter.add(
        &entities,
        &Affine::IDENTITY,
        file.hides(DEFAULT_LAYER),
        &mut Vec::new(),
    )?;
    let mut notes = Vec::new();
    let scale = unit_note(file.units, &mut notes);
    let shapes: Vec<Shape> = interpreter
        .shapes
        .iter()
        .map(|shape| shape.transformed(&Affine::scale(Vector3::splat(scale))))
        .collect();
    let drawing = flatten(&shapes, notes);
    interpreter.report(drawing)
}

fn looks_like_dxf(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return true;
    }
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let first_line = bytes
        .split(|byte| *byte == b'\n')
        .map(|line| String::from_utf8_lossy(line).trim().to_owned())
        .find(|line| !line.is_empty());
    matches!(first_line.as_deref(), Some("0" | "999"))
}

#[derive(Debug, Clone, PartialEq)]
struct Record {
    kind: String,
    pairs: Vec<Pair>,
}

trait Fields {
    fn pairs(&self) -> &[Pair];

    fn field(&self, code: i32) -> Option<&Pair> {
        self.pairs().iter().find(|pair| pair.code == code)
    }

    fn real(&self, code: i32) -> Option<f64> {
        self.field(code)?.real()
    }

    fn real_or(&self, code: i32, default: f64) -> f64 {
        self.real(code).unwrap_or(default)
    }

    fn flags(&self, code: i32) -> i64 {
        self.field(code).and_then(Pair::integer).unwrap_or_default()
    }

    fn text(&self, code: i32) -> Option<&str> {
        Some(self.field(code)?.text().trim())
    }

    fn point(&self, code: i32) -> Option<Point3> {
        Some(Point3::new(
            self.real(code)?,
            self.real(code + 10)?,
            self.real_or(code + 20, 0.0),
        ))
    }

    fn normal(&self) -> Vector3 {
        self.point(210).unwrap_or(Vector3::Z)
    }

    fn reals(&self, code: i32) -> Vec<f64> {
        self.pairs()
            .iter()
            .filter(|pair| pair.code == code)
            .filter_map(Pair::real)
            .collect()
    }

    fn points(&self, code: i32) -> Vec<Point3> {
        let mut points: Vec<Point3> = Vec::new();
        for pair in self.pairs() {
            let Some(value) = pair.real() else {
                continue;
            };
            if pair.code == code {
                points.push(Point3::new(value, 0.0, 0.0));
            } else if let Some(last) = points.last_mut() {
                if pair.code == code + 10 {
                    last.y = value;
                } else if pair.code == code + 20 {
                    last.z = value;
                }
            }
        }
        points
    }

    fn layer(&self) -> &str {
        self.text(8)
            .filter(|layer| !layer.is_empty())
            .unwrap_or(DEFAULT_LAYER)
    }
}

impl Fields for Record {
    fn pairs(&self) -> &[Pair] {
        &self.pairs
    }
}

impl Fields for [Pair] {
    fn pairs(&self) -> &[Pair] {
        self
    }
}

fn records(pairs: Vec<Pair>) -> Result<Vec<Record>, ImportError> {
    let mut records: Vec<Record> = Vec::new();
    for pair in pairs {
        if pair.code == 0 {
            records.push(Record {
                kind: pair.text().trim().to_ascii_uppercase(),
                pairs: Vec::new(),
            });
        } else if let Some(current) = records.last_mut() {
            current.pairs.push(pair);
        } else if pair.code != 999 {
            return Err(ImportError::NotDxf);
        }
    }
    Ok(records)
}

struct Layer {
    hidden: bool,
}

struct Block {
    base: Point3,
    external: bool,
    records: Vec<Record>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HeaderUnits {
    insertion: Option<i64>,
    imperial: bool,
}

struct DxfFile {
    units: HeaderUnits,
    layers: BTreeMap<String, Layer>,
    blocks: BTreeMap<String, Block>,
    entities: Vec<Record>,
    line_styles: LineStyles,
}

#[derive(Default)]
struct LineStyles {
    by_handle: BTreeMap<String, i64>,
    by_name: BTreeMap<String, i64>,
}

impl LineStyles {
    fn add(&mut self, style: &Record) {
        let flags = style.flags(70);
        if let Some(handle) = style.text(5) {
            self.by_handle.insert(handle.to_ascii_uppercase(), flags);
        }
        if let Some(name) = style.text(2) {
            self.by_name.insert(name.to_ascii_uppercase(), flags);
        }
    }

    fn of(&self, line: &Record) -> i64 {
        let by_handle = line
            .text(340)
            .and_then(|handle| self.by_handle.get(&handle.to_ascii_uppercase()));
        let by_name = || {
            line.text(2)
                .and_then(|name| self.by_name.get(&name.to_ascii_uppercase()))
        };
        by_handle.or_else(by_name).copied().unwrap_or_default()
    }
}

impl DxfFile {
    fn hides(&self, layer: &str) -> bool {
        self.layers
            .get(&layer.to_ascii_uppercase())
            .is_some_and(|layer| layer.hidden)
    }

    fn read(records: &[Record]) -> Result<Self, ImportError> {
        let mut file = Self {
            units: HeaderUnits::default(),
            layers: BTreeMap::new(),
            blocks: BTreeMap::new(),
            entities: Vec::new(),
            line_styles: LineStyles::default(),
        };
        let mut section = None;
        let mut sections = 0;
        let mut block: Option<(String, Block)> = None;
        for record in records {
            match record.kind.as_str() {
                "SECTION" => {
                    sections += 1;
                    let name = record.text(2).unwrap_or_default().to_ascii_uppercase();
                    if name == "HEADER" {
                        file.units = header_units(record);
                    }
                    section = Some(name);
                    continue;
                }
                "ENDSEC" => {
                    section = None;
                    continue;
                }
                "EOF" => break,
                _ => {}
            }
            match section.as_deref() {
                Some("TABLES") if record.kind == "LAYER" => {
                    let name = record.text(2).unwrap_or_default().to_ascii_uppercase();
                    let off = record.field(62).and_then(Pair::integer).unwrap_or(7) < 0;
                    let frozen = record.flags(70) & FROZEN_LAYER != 0;
                    file.layers.insert(
                        name,
                        Layer {
                            hidden: off || frozen,
                        },
                    );
                }
                Some("BLOCKS") => match record.kind.as_str() {
                    "BLOCK" => {
                        block = Some((
                            record.text(2).unwrap_or_default().to_ascii_uppercase(),
                            Block {
                                base: record.point(10).unwrap_or(Point3::ZERO),
                                external: record.flags(70) & EXTERNAL_BLOCK != 0,
                                records: Vec::new(),
                            },
                        ));
                    }
                    "ENDBLK" => {
                        if let Some((name, finished)) = block.take() {
                            file.blocks.insert(name, finished);
                        }
                    }
                    _ => {
                        if let Some((_, open)) = &mut block {
                            open.records.push(record.clone());
                        }
                    }
                },
                Some("ENTITIES") => file.entities.push(record.clone()),
                Some("OBJECTS") if record.kind == "MLINESTYLE" => file.line_styles.add(record),
                _ => {}
            }
        }
        if sections == 0 {
            return Err(ImportError::NotDxf);
        }
        Ok(file)
    }
}

fn header_units(header: &Record) -> HeaderUnits {
    let mut units = HeaderUnits::default();
    let mut variable = "";
    for pair in &header.pairs {
        if pair.code == 9 {
            variable = pair.text().trim();
        } else if pair.code == 70 {
            if variable.eq_ignore_ascii_case("$INSUNITS") {
                units.insertion = pair.integer();
            } else if variable.eq_ignore_ascii_case("$MEASUREMENT") {
                units.imperial = pair.integer() == Some(IMPERIAL_MEASUREMENT);
            }
        }
    }
    units
}

struct Item<'a> {
    record: &'a Record,
    vertices: Vec<&'a Record>,
}

fn items(records: &[Record]) -> Vec<Item<'_>> {
    let mut items = Vec::new();
    let mut records = records.iter().peekable();
    while let Some(record) = records.next() {
        let mut vertices = Vec::new();
        let owns_followers = match record.kind.as_str() {
            "POLYLINE" => true,
            "INSERT" => record.flags(66) != 0,
            "VERTEX" | "SEQEND" | "ATTRIB" => continue,
            _ => false,
        };
        if owns_followers {
            while let Some(follower) =
                records.next_if(|next| matches!(next.kind.as_str(), "VERTEX" | "ATTRIB"))
            {
                if follower.kind == "VERTEX" {
                    vertices.push(follower);
                }
            }
            records.next_if(|next| next.kind == "SEQEND");
        }
        items.push(Item { record, vertices });
    }
    items
}

struct Prepared {
    placement: Placement,
    decoded: Decoded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Skipped,
    OnHiddenLayer,
    OnShownLayer,
    OnInheritedLayer,
}

impl Placement {
    fn of(file: &DxfFile, record: &Record) -> Self {
        if record.flags(67) & PAPER_SPACE != 0 || record.flags(60) & INVISIBLE != 0 {
            return Self::Skipped;
        }
        match record.layer() {
            DEFAULT_LAYER => Self::OnInheritedLayer,
            own if file.hides(own) => Self::OnHiddenLayer,
            _ => Self::OnShownLayer,
        }
    }

    fn visibility(self, inherited_hidden: bool) -> Visibility {
        match self {
            Self::Skipped => Visibility::Skipped,
            Self::OnHiddenLayer => Visibility::Hidden,
            Self::OnShownLayer => Visibility::Shown,
            Self::OnInheritedLayer if inherited_hidden => Visibility::Hidden,
            Self::OnInheritedLayer => Visibility::Shown,
        }
    }
}

fn prepare(file: &DxfFile, items: &[Item<'_>]) -> Vec<Prepared> {
    let handles: BTreeMap<String, usize> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| Some((item.record.text(5)?.to_ascii_uppercase(), index)))
        .collect();
    items
        .iter()
        .map(|item| Prepared {
            placement: Placement::of(file, item.record),
            decoded: match item.record.kind.as_str() {
                "INSERT" => Decoded::Insert(Insertion::read(item.record)),
                "HATCH" => hatch(item.record, &handles),
                _ => shapes(file, item),
            },
        })
        .collect()
}

fn hatch(record: &Record, handles: &BTreeMap<String, usize>) -> Decoded {
    let Some(boundaries) = boundaries(record) else {
        return Decoded::Unreadable;
    };
    Decoded::Hatch(
        boundaries
            .into_iter()
            .map(|boundary| HatchBoundary {
                shapes: boundary.shapes,
                traced_by: boundary
                    .sources
                    .iter()
                    .map(|source| handles.get(source).copied())
                    .collect::<Option<_>>()
                    .unwrap_or_default(),
            })
            .collect(),
    )
}

struct HatchBoundary {
    shapes: Vec<Shape>,
    traced_by: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Skipped,
    Hidden,
    Shown,
}

struct Insertion {
    block: String,
    written_name: String,
    normal: Vector3,
    at: Point3,
    scale: Vector3,
    rotation: f64,
    columns: i64,
    rows: i64,
    spacing: Vector3,
}

impl Insertion {
    fn read(record: &Record) -> Self {
        let written_name = record.text(2).unwrap_or_default().to_owned();
        Self {
            block: written_name.to_ascii_uppercase(),
            written_name,
            normal: record.normal(),
            at: record.point(10).unwrap_or(Point3::ZERO),
            scale: Vector3::new(
                record.real_or(41, 1.0),
                record.real_or(42, 1.0),
                record.real_or(43, 1.0),
            ),
            rotation: record.real_or(50, 0.0).to_radians(),
            columns: record.flags(70).max(1),
            rows: record.flags(71).max(1),
            spacing: Vector3::new(record.real_or(44, 0.0), record.real_or(45, 0.0), 0.0),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Tally {
    left_out: BTreeMap<(&'static str, &'static str), usize>,
    hidden: usize,
    unreadable: usize,
    too_deep: usize,
    hatches: usize,
}

impl Tally {
    fn add_repeated(&mut self, since: &Self, repeats: usize) {
        for (category, count) in &self.left_out.clone() {
            let before = since.left_out.get(category).copied().unwrap_or(0);
            let extra = count.saturating_sub(before).saturating_mul(repeats);
            if let Some(slot) = self.left_out.get_mut(category) {
                *slot = slot.saturating_add(extra);
            }
        }
        let repeat = |now: usize, before: usize| {
            now.saturating_add(now.saturating_sub(before).saturating_mul(repeats))
        };
        self.hidden = repeat(self.hidden, since.hidden);
        self.unreadable = repeat(self.unreadable, since.unreadable);
        self.too_deep = repeat(self.too_deep, since.too_deep);
        self.hatches = repeat(self.hatches, since.hatches);
    }
}

struct Interpreter<'a> {
    file: &'a DxfFile,
    shapes: Vec<Shape>,
    points: usize,
    tally: Tally,
    visited: usize,
    external: BTreeSet<String>,
    missing: BTreeSet<String>,
    contents: BTreeMap<String, Rc<Vec<Prepared>>>,
    drawing: BTreeMap<String, Option<bool>>,
}

impl<'a> Interpreter<'a> {
    fn new(file: &'a DxfFile) -> Self {
        Self {
            file,
            shapes: Vec::new(),
            points: 0,
            tally: Tally::default(),
            visited: 0,
            external: BTreeSet::new(),
            missing: BTreeSet::new(),
            contents: BTreeMap::new(),
            drawing: BTreeMap::new(),
        }
    }

    fn contents(&mut self, name: &str) -> Option<Rc<Vec<Prepared>>> {
        if let Some(known) = self.contents.get(name) {
            return Some(Rc::clone(known));
        }
        let file = self.file;
        let block = file.blocks.get(name)?;
        let prepared = Rc::new(prepare(file, &items(&block.records)));
        self.contents.insert(name.to_owned(), Rc::clone(&prepared));
        Some(prepared)
    }

    fn draws(&mut self, name: &str, depth: usize) -> bool {
        if let Some(known) = self.drawing.get(name) {
            return known.unwrap_or(true);
        }
        let external = self
            .file
            .blocks
            .get(name)
            .is_none_or(|block| block.external);
        if external {
            return false;
        }
        if depth > MAX_BLOCK_DEPTH {
            return true;
        }
        let Some(contents) = self.contents(name) else {
            return false;
        };
        self.drawing.insert(name.to_owned(), None);
        let draws = contents.iter().any(|item| match &item.decoded {
            Decoded::Shapes(shapes) => !shapes.is_empty(),
            Decoded::Hatch(boundaries) => boundaries
                .iter()
                .any(|boundary| !boundary.shapes.is_empty()),
            Decoded::Insert(insertion) => self.draws(&insertion.block, depth + 1),
            Decoded::Unreadable | Decoded::LeftOut(_) | Decoded::Ignored => false,
        });
        self.drawing.insert(name.to_owned(), Some(draws));
        draws
    }

    fn add(
        &mut self,
        items: &[Prepared],
        transform: &Affine,
        inherited_hidden: bool,
        blocks: &mut Vec<String>,
    ) -> Result<(), ImportError> {
        for item in items {
            self.visit()?;
            match item.placement.visibility(inherited_hidden) {
                Visibility::Skipped => continue,
                Visibility::Hidden => {
                    self.tally.hidden += 1;
                    continue;
                }
                Visibility::Shown => {}
            }
            match &item.decoded {
                Decoded::Insert(insertion) => self.insert(insertion, transform, blocks)?,
                Decoded::Shapes(shapes) => self.push(shapes, transform)?,
                Decoded::Hatch(boundaries) => {
                    let mut drawn = false;
                    for boundary in boundaries {
                        if !Self::traced(items, &boundary.traced_by, inherited_hidden) {
                            drawn |= !boundary.shapes.is_empty();
                            self.push(&boundary.shapes, transform)?;
                        }
                    }
                    if drawn {
                        self.tally.hatches += 1;
                    }
                }
                Decoded::Unreadable => self.tally.unreadable += 1,
                Decoded::LeftOut(category) => {
                    *self.tally.left_out.entry(*category).or_default() += 1;
                }
                Decoded::Ignored => {}
            }
        }
        Ok(())
    }

    fn traced(items: &[Prepared], sources: &[usize], inherited_hidden: bool) -> bool {
        !sources.is_empty()
            && sources.iter().all(|source| {
                items.get(*source).is_some_and(|source| {
                    matches!(&source.decoded, Decoded::Shapes(shapes) if !shapes.is_empty())
                        && matches!(
                            source.placement.visibility(inherited_hidden),
                            Visibility::Shown
                        )
                })
            })
    }

    fn push(&mut self, shapes: &[Shape], transform: &Affine) -> Result<(), ImportError> {
        for shape in shapes {
            self.points = self.points.saturating_add(shape.size());
            if self.points > MAX_DRAWING_POINTS {
                return Err(ImportError::TooDetailed);
            }
            self.shapes.push(shape.transformed(transform));
        }
        if self.shapes.len() > MAX_DRAWING_CURVES {
            return Err(ImportError::TooLarge);
        }
        Ok(())
    }

    fn visit(&mut self) -> Result<(), ImportError> {
        self.visited += 1;
        if self.visited > MAX_EXPANDED_OBJECTS {
            return Err(ImportError::TooManyObjects);
        }
        Ok(())
    }

    fn insert(
        &mut self,
        insertion: &Insertion,
        transform: &Affine,
        blocks: &mut Vec<String>,
    ) -> Result<(), ImportError> {
        let file = self.file;
        let name = &insertion.block;
        let (Some(block), Some(contents)) = (file.blocks.get(name), self.contents(name)) else {
            self.missing.insert(insertion.written_name.clone());
            return Ok(());
        };
        if block.external {
            self.external.insert(insertion.written_name.clone());
            return Ok(());
        }
        if blocks.len() >= MAX_BLOCK_DEPTH || blocks.contains(name) {
            self.tally.too_deep += 1;
            return Ok(());
        }
        let Some(system) = Affine::object_system(insertion.normal) else {
            self.tally.unreadable += 1;
            return Ok(());
        };
        let draws = self.draws(name, blocks.len());
        let placement = Affine::rotation_z(insertion.rotation)
            .then(&Affine::translation(insertion.at))
            .then(&system)
            .then(transform);
        let local = Affine::translation(-block.base).then(&Affine::scale(insertion.scale));
        blocks.push(name.clone());
        if !draws {
            let before = self.tally.clone();
            self.visit()?;
            self.add(&contents, &local.then(&placement), SHOWN, blocks)?;
            let cells = usize::try_from(insertion.columns.saturating_mul(insertion.rows))
                .unwrap_or(usize::MAX);
            self.tally.add_repeated(&before, cells.saturating_sub(1));
            blocks.pop();
            return Ok(());
        }
        for row in 0..insertion.rows {
            for column in 0..insertion.columns {
                let offset = Vector3::new(column as f64, row as f64, 0.0) * insertion.spacing;
                self.visit()?;
                let cell = local.then(&Affine::translation(offset)).then(&placement);
                if !cell.is_finite() {
                    self.tally.unreadable += 1;
                    continue;
                }
                self.add(&contents, &cell, SHOWN, blocks)?;
            }
        }
        blocks.pop();
        Ok(())
    }

    fn report(self, mut drawing: Drawing) -> Result<Drawing, ImportError> {
        let earlier = drawing.notes.len();
        let tally = self.tally;
        if !tally.left_out.is_empty() {
            let kinds: Vec<String> = tally
                .left_out
                .iter()
                .map(|((singular, plural), count)| counted(*count, singular, plural))
                .collect();
            drawing.notes.push(format!(
                "{} {} left out, because sketches hold only points, lines, arcs, circles and \
                 splines.",
                capitalized(&list(&kinds)),
                were(
                    tally
                        .left_out
                        .values()
                        .fold(0, |sum, count| sum.saturating_add(*count))
                )
            ));
        }
        if tally.hatches > 0 {
            drawing.notes.push(if tally.hatches == 1 {
                "The boundary of 1 hatch was imported without its fill.".to_owned()
            } else {
                format!(
                    "The boundaries of {} hatches were imported without their fill.",
                    tally.hatches
                )
            });
        }
        if tally.hidden > 0 {
            drawing.notes.push(format!(
                "{} on hidden or frozen layers {} left out.",
                capitalized(&counted(tally.hidden, "object", "objects")),
                were(tally.hidden)
            ));
        }
        if !self.external.is_empty() {
            let names: Vec<String> = self.external.into_iter().collect();
            drawing.notes.push(format!(
                "Blocks that refer to other drawings were left out: {}.",
                list(&names)
            ));
        }
        if !self.missing.is_empty() {
            let names: Vec<String> = self.missing.into_iter().collect();
            drawing.notes.push(format!(
                "Blocks that the drawing uses but does not contain were left out: {}.",
                list(&names)
            ));
        }
        if tally.too_deep > 0 {
            drawing.notes.push(format!(
                "{} nested too deeply or inside themselves {} left out.",
                capitalized(&counted(tally.too_deep, "block", "blocks")),
                were(tally.too_deep)
            ));
        }
        if tally.unreadable > 0 {
            drawing.notes.push(format!(
                "{} could not be read and {} left out.",
                capitalized(&counted(
                    tally.unreadable,
                    "damaged object",
                    "damaged objects"
                )),
                were(tally.unreadable)
            ));
        }
        if drawing.curves.is_empty() {
            return Err(ImportError::Empty {
                left_out: drawing.notes.split_off(earlier),
            });
        }
        Ok(drawing)
    }
}

enum Decoded {
    Shapes(Vec<Shape>),
    Hatch(Vec<HatchBoundary>),
    Unreadable,
    LeftOut((&'static str, &'static str)),
    Ignored,
    Insert(Insertion),
}

fn shapes(file: &DxfFile, item: &Item<'_>) -> Decoded {
    let record = item.record;
    let decoded = match record.kind.as_str() {
        "LINE" => line(record),
        "POINT" => record.point(10).map(|point| vec![Shape::Point(point)]),
        "CIRCLE" => circle(record, None),
        "ARC" => circle(
            record,
            Some((
                record.real_or(50, 0.0).to_radians(),
                record.real_or(51, 360.0).to_radians(),
            )),
        ),
        "ELLIPSE" => ellipse(record),
        "LWPOLYLINE" => light_polyline(record),
        "POLYLINE" if record.flags(70) & POLYLINE_MESH != 0 => {
            return Decoded::LeftOut(("3D object", "3D objects"));
        }
        "POLYLINE" => polyline(record, &item.vertices),
        "SPLINE" => spline(record),
        "SOLID" | "TRACE" => filled_outline(record),
        "3DFACE" => face_outline(record),
        "MLINE" => mline(record, file.line_styles.of(record)),
        other => {
            return match left_out(other) {
                Some(category) => Decoded::LeftOut(category),
                None => Decoded::Ignored,
            };
        }
    };
    match decoded {
        Some(shapes) => Decoded::Shapes(shapes),
        None => Decoded::Unreadable,
    }
}

fn left_out(kind: &str) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        "TEXT" | "MTEXT" | "RTEXT" => ("text", "texts"),
        "DIMENSION" | "ARC_DIMENSION" | "LARGE_RADIAL_DIMENSION" => ("dimension", "dimensions"),
        "MPOLYGON" => ("hatch", "hatches"),
        "WIPEOUT" => ("filled area", "filled areas"),
        "LEADER" | "MLEADER" | "MULTILEADER" | "TOLERANCE" => ("leader", "leaders"),
        "XLINE" | "RAY" => ("construction line", "construction lines"),
        "3DSOLID" | "BODY" | "REGION" | "SURFACE" | "PLANESURFACE" | "EXTRUDEDSURFACE"
        | "LOFTEDSURFACE" | "REVOLVEDSURFACE" | "SWEPTSURFACE" | "NURBSURFACE" | "MESH"
        | "POLYFACE" => ("3D object", "3D objects"),
        "IMAGE" | "OLEFRAME" | "OLE2FRAME" | "PDFUNDERLAY" | "DWFUNDERLAY" | "DGNUNDERLAY" => {
            ("image", "images")
        }
        "VIEWPORT" | "ATTDEF" | "SEQEND" | "VERTEX" | "ATTRIB" | "ENDBLK" | "BLOCK" => {
            return None;
        }
        _ => ("other object", "other objects"),
    })
}

fn line(record: &Record) -> Option<Vec<Shape>> {
    Some(vec![Shape::Line(record.point(10)?, record.point(11)?)])
}

fn circle(record: &Record, angles: Option<(f64, f64)>) -> Option<Vec<Shape>> {
    let system = Affine::object_system(record.normal())?;
    let center = system.point(record.point(10)?);
    let radius = record.real(40).filter(|radius| *radius > 0.0)?;
    let (start, end) = angles.unwrap_or((0.0, TAU));
    Some(vec![conic_arc(
        center,
        system.vector(Vector3::X) * radius,
        system.vector(Vector3::Y) * radius,
        start,
        end,
    )])
}

fn ellipse(record: &Record) -> Option<Vec<Shape>> {
    let center = record.point(10)?;
    let major = record.point(11)?;
    let ratio = record.real(40).filter(|ratio| *ratio > 0.0)?;
    let normal = record.normal().try_normalize()?;
    let minor = normal.cross(major) * ratio;
    let start = record.real_or(41, 0.0);
    let end = record.real_or(42, TAU);
    (major.length() > 0.0).then(|| vec![conic_arc(center, major, minor, start, end)])
}

fn light_polyline(record: &Record) -> Option<Vec<Shape>> {
    let system = Affine::object_system(record.normal())?;
    let elevation = record.real_or(38, 0.0);
    let mut vertices: Vec<(Point2, f64)> = Vec::new();
    for pair in &record.pairs {
        match pair.code {
            10 => vertices.push((Point2::new(pair.real()?, 0.0), 0.0)),
            20 => vertices.last_mut()?.0.y = pair.real()?,
            42 => vertices.last_mut()?.1 = pair.real()?,
            _ => {}
        }
    }
    let closed = record.flags(70) & CLOSED != 0;
    Some(
        polyline_segments(&vertices, closed, elevation)
            .iter()
            .map(|shape| shape.transformed(&system))
            .collect(),
    )
}

fn polyline(record: &Record, vertices: &[&Record]) -> Option<Vec<Shape>> {
    let flags = record.flags(70);
    let closed = flags & CLOSED != 0;
    let used = vertices
        .iter()
        .filter(|vertex| vertex.flags(70) & SPLINE_FRAME_VERTEX == 0);
    if flags & POLYLINE_3D != 0 {
        let points: Vec<Point3> = used.map(|vertex| vertex.point(10)).collect::<Option<_>>()?;
        let mut shapes: Vec<Shape> = points
            .windows(2)
            .filter_map(|pair| match pair {
                [start, end] => Some(Shape::Line(*start, *end)),
                _ => None,
            })
            .collect();
        if closed
            && points.len() > 2
            && let (Some(first), Some(last)) = (points.first(), points.last())
        {
            shapes.push(Shape::Line(*last, *first));
        }
        return Some(shapes);
    }
    let system = Affine::object_system(record.normal())?;
    let elevation = record.point(10).map_or(0.0, |point| point.z);
    let flat: Vec<(Point2, f64)> = used
        .map(|vertex| {
            let point = vertex.point(10)?;
            Some((Point2::new(point.x, point.y), vertex.real_or(42, 0.0)))
        })
        .collect::<Option<_>>()?;
    Some(
        polyline_segments(&flat, closed, elevation)
            .iter()
            .map(|shape| shape.transformed(&system))
            .collect(),
    )
}

fn polyline_segments(vertices: &[(Point2, f64)], closed: bool, elevation: f64) -> Vec<Shape> {
    let count = vertices.len();
    let bulged = vertices
        .iter()
        .any(|(_, bulge)| bulge.abs() >= BULGE_EPSILON);
    let segments = if closed && (count > 2 || (count == 2 && bulged)) {
        count
    } else {
        count.saturating_sub(1)
    };
    (0..segments)
        .filter_map(|index| {
            let (start, bulge) = *vertices.get(index)?;
            let (end, _) = *vertices.get((index + 1) % count)?;
            bulge_segment(start, end, bulge, elevation)
        })
        .collect()
}

fn bulge_segment(start: Point2, end: Point2, bulge: f64, elevation: f64) -> Option<Shape> {
    let lift = |point: Point2| Point3::new(point.x, point.y, elevation);
    let chord = end - start;
    let length = chord.length();
    if length == 0.0 {
        return None;
    }
    if bulge.abs() < BULGE_EPSILON || !bulge.is_finite() {
        return Some(Shape::Line(lift(start), lift(end)));
    }
    let sweep = 4.0 * bulge.atan();
    let left = chord.perp() / length;
    let center =
        start + chord / 2.0 + left * (length / 2.0 * (1.0 - bulge * bulge) / (2.0 * bulge));
    let radius = start - center;
    let turning = if sweep > 0.0 {
        radius.perp()
    } else {
        -radius.perp()
    };
    Some(Shape::Conic {
        center: lift(center),
        major: Vector3::new(radius.x, radius.y, 0.0),
        minor: Vector3::new(turning.x, turning.y, 0.0),
        start: 0.0,
        sweep: sweep.abs(),
    })
}

fn spline(record: &Record) -> Option<Vec<Shape>> {
    let control_points = record.points(10);
    let weights = record.reals(41);
    let weights = (!weights.is_empty()).then_some(weights);
    let controlled = usize::try_from(record.flags(71))
        .ok()
        .filter(|_| !control_points.is_empty())
        .and_then(|degree| Nurbs::new(degree, record.reals(40), control_points, weights));
    if let Some(nurbs) = controlled {
        return Some(vec![Shape::Spline(nurbs)]);
    }
    let fit_points = record.points(11);
    (fit_points.len() >= 2).then(|| {
        vec![Shape::Interpolated(FitPoints::new(
            fit_points,
            record.point(12),
            record.point(13),
        ))]
    })
}

fn unit_note(units: HeaderUnits, notes: &mut Vec<String>) -> f64 {
    let named = units.insertion.filter(|code| *code != UNITLESS);
    if named.is_none() && units.imperial {
        notes.push(
            "The drawing does not name its unit but uses imperial measurement, so its numbers \
             were read as inches and converted to millimetres."
                .to_owned(),
        );
        return INCH;
    }
    match named.map(|code| (code, unit(code))) {
        Some((_, Some((scale, name)))) => {
            if scale != 1.0 {
                notes.push(format!(
                    "The drawing is in {name}, so its lengths were converted to millimetres."
                ));
            }
            scale
        }
        Some((code, None)) => {
            notes.push(format!(
                "The drawing names a unit caditor does not know (code {code}), so its numbers \
                 were read as millimetres."
            ));
            1.0
        }
        None => {
            notes.push(
                "The drawing does not say which unit it uses, so its numbers were read as \
                 millimetres."
                    .to_owned(),
            );
            1.0
        }
    }
}

const UNITLESS: i64 = 0;
const INCH: f64 = 25.4;
const US_SURVEY_FOOT: f64 = 1_200_000.0 / 3_937.0;
const IMPERIAL_MEASUREMENT: i64 = 0;

fn unit(code: i64) -> Option<(f64, &'static str)> {
    Some(match code {
        1 => (25.4, "inches"),
        2 => (304.8, "feet"),
        3 => (1_609_344.0, "miles"),
        4 => (1.0, "millimetres"),
        5 => (10.0, "centimetres"),
        6 => (1_000.0, "metres"),
        7 => (1_000_000.0, "kilometres"),
        8 => (2.54e-5, "microinches"),
        9 => (0.0254, "mils"),
        10 => (914.4, "yards"),
        11 => (1e-7, "ångströms"),
        12 => (1e-6, "nanometres"),
        13 => (1e-3, "micrometres"),
        14 => (100.0, "decimetres"),
        15 => (10_000.0, "decametres"),
        16 => (100_000.0, "hectometres"),
        17 => (1e12, "gigametres"),
        18 => (1.495_978_707e14, "astronomical units"),
        19 => (9.460_730_472_580_8e18, "light years"),
        20 => (3.085_677_581_491_367e19, "parsecs"),
        21 => (US_SURVEY_FOOT, "US survey feet"),
        22 => (US_SURVEY_FOOT / 12.0, "US survey inches"),
        23 => (US_SURVEY_FOOT * 3.0, "US survey yards"),
        24 => (US_SURVEY_FOOT * 5_280.0, "US survey miles"),
        _ => return None,
    })
}

pub(super) fn counted(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

pub(super) fn were(count: usize) -> &'static str {
    if count == 1 { "was" } else { "were" }
}

pub(super) fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}
