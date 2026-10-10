use std::{collections::BTreeMap, rc::Rc};

use caditor_geometry::Point2;
use ttf_parser::{
    Face, GlyphId, OutlineBuilder, Tag,
    gpos::{PairAdjustment, PositioningSubtable},
};

use crate::import::svg::{
    overlap::{Segment, merged, traced},
    path::Outline,
};

const WEIGHT_AXIS: Tag = Tag::from_bytes(b"wght");
const KERNING: Tag = Tag::from_bytes(b"kern");
const LATIN: Tag = Tag::from_bytes(b"latn");
const DEFAULT_SCRIPT: Tag = Tag::from_bytes(b"DFLT");
const MISSING: GlyphId = GlyphId(0);

pub(super) struct Typeface<'f> {
    upright: Cut<'f>,
    italic: Option<Cut<'f>>,
}

struct Cut<'f> {
    face: Face<'f>,
    units_per_em: f64,
    weights: Option<(f64, f64)>,
    kerning: Vec<u16>,
    weighted: BTreeMap<u16, Face<'f>>,
    outlines: BTreeMap<(u16, GlyphId), Rc<Outline>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Instance {
    weight: u16,
    italic: bool,
}

impl<'f> Typeface<'f> {
    pub fn parse(upright: &'f [u8], italic: Option<&'f [u8]>) -> Option<Self> {
        Some(Self {
            upright: Cut::parse(upright)?,
            italic: italic.and_then(Cut::parse),
        })
    }

    pub fn has_italic(&self) -> bool {
        self.italic.is_some()
    }

    pub fn units_per_em(&self) -> f64 {
        self.upright.units_per_em
    }

    pub fn instance(&self, wanted_weight: f64, wanted_italic: bool) -> Instance {
        let italic = wanted_italic && self.italic.is_some();
        Instance {
            weight: self.cut(italic).weight(wanted_weight),
            italic,
        }
    }

    pub fn glyph(&self, character: char, instance: Instance) -> Option<GlyphId> {
        self.cut(instance.italic).glyph(character)
    }

    pub fn advance(&mut self, instance: Instance, glyph: Option<GlyphId>) -> f64 {
        let glyph = glyph.unwrap_or(MISSING);
        self.cut_mut(instance.italic)
            .weighted(instance.weight)
            .glyph_hor_advance(glyph)
            .map_or(0.0, f64::from)
    }

    pub fn kerning(&self, instance: Instance, first: GlyphId, second: GlyphId) -> f64 {
        self.cut(instance.italic).kerning(first, second)
    }

    pub fn outline(&mut self, instance: Instance, glyph: GlyphId) -> Rc<Outline> {
        self.cut_mut(instance.italic)
            .outline(instance.weight, glyph)
    }

    pub fn curve_count(&mut self, instance: Instance, glyph: GlyphId) -> usize {
        self.cut_mut(instance.italic)
            .curve_count(instance.weight, glyph)
    }

    fn cut(&self, italic: bool) -> &Cut<'f> {
        match (&self.italic, italic) {
            (Some(cut), true) => cut,
            _ => &self.upright,
        }
    }

    fn cut_mut(&mut self, italic: bool) -> &mut Cut<'f> {
        match (&mut self.italic, italic) {
            (Some(cut), true) => cut,
            _ => &mut self.upright,
        }
    }
}

impl<'f> Cut<'f> {
    fn parse(data: &'f [u8]) -> Option<Self> {
        let face = Face::parse(data, 0).ok()?;
        let units_per_em = f64::from(face.units_per_em());
        let weights = face
            .variation_axes()
            .into_iter()
            .find(|axis| axis.tag == WEIGHT_AXIS)
            .map(|axis| (f64::from(axis.min_value), f64::from(axis.max_value)));
        let kerning = kerning_lookups(&face);
        Some(Self {
            face,
            units_per_em,
            weights,
            kerning,
            weighted: BTreeMap::new(),
            outlines: BTreeMap::new(),
        })
    }

    fn weight(&self, wanted: f64) -> u16 {
        match self.weights {
            Some((lightest, heaviest)) => {
                wanted.clamp(lightest, heaviest).round().clamp(0.0, 1000.0) as u16
            }
            None => 0,
        }
    }

    fn glyph(&self, character: char) -> Option<GlyphId> {
        self.face
            .glyph_index(character)
            .filter(|glyph| *glyph != MISSING)
    }

    fn kerning(&self, first: GlyphId, second: GlyphId) -> f64 {
        let Some(positioning) = self.face.tables().gpos else {
            return 0.0;
        };
        let mut total = 0.0;
        for index in &self.kerning {
            let Some(lookup) = positioning.lookups.get(*index) else {
                continue;
            };
            let adjusted = lookup
                .subtables
                .into_iter::<PositioningSubtable<'_>>()
                .find_map(|subtable| match subtable {
                    PositioningSubtable::Pair(pair) => pair_adjustment(pair, first, second),
                    _ => None,
                });
            total += adjusted.map_or(0.0, f64::from);
        }
        total
    }

    fn outline(&mut self, weight: u16, glyph: GlyphId) -> Rc<Outline> {
        if let Some(outline) = self.outlines.get(&(weight, glyph)) {
            return Rc::clone(outline);
        }
        let segments = self.segments(weight, glyph);
        let outline = Rc::new(merged(&segments).unwrap_or_else(|| traced(&segments)));
        self.outlines.insert((weight, glyph), Rc::clone(&outline));
        outline
    }

    fn curve_count(&mut self, weight: u16, glyph: GlyphId) -> usize {
        match self.outlines.get(&(weight, glyph)) {
            Some(outline) => outline.shapes.len(),
            None => self.segments(weight, glyph).len(),
        }
    }

    fn segments(&mut self, weight: u16, glyph: GlyphId) -> Vec<Segment> {
        let mut pen = Pen::default();
        self.weighted(weight).outline_glyph(glyph, &mut pen);
        pen.segments
    }

    fn weighted(&mut self, weight: u16) -> &Face<'f> {
        let base = &self.face;
        self.weighted.entry(weight).or_insert_with(|| {
            let mut face = base.clone();
            if weight > 0 {
                face.set_variation(WEIGHT_AXIS, f32::from(weight));
            }
            face
        })
    }
}

fn kerning_lookups(face: &Face<'_>) -> Vec<u16> {
    let Some(positioning) = face.tables().gpos else {
        return Vec::new();
    };
    let language = positioning
        .scripts
        .find(LATIN)
        .or_else(|| positioning.scripts.find(DEFAULT_SCRIPT))
        .and_then(|script| script.default_language);
    let Some(language) = language else {
        return Vec::new();
    };
    let mut lookups: Vec<u16> = language
        .feature_indices
        .into_iter()
        .filter_map(|index| positioning.features.get(index))
        .filter(|feature| feature.tag == KERNING)
        .flat_map(|feature| feature.lookup_indices)
        .collect();
    lookups.sort_unstable();
    lookups.dedup();
    lookups
}

fn pair_adjustment(pair: PairAdjustment<'_>, first: GlyphId, second: GlyphId) -> Option<i16> {
    match pair {
        PairAdjustment::Format1 { coverage, sets } => {
            let index = coverage.get(first)?;
            let (record, _) = sets.get(index)?.get(second)?;
            Some(record.x_advance)
        }
        PairAdjustment::Format2 {
            coverage,
            classes,
            matrix,
        } => {
            if !coverage.contains(first) {
                return None;
            }
            let (record, _) = matrix.get((classes.0.get(first), classes.1.get(second)))?;
            Some(record.x_advance)
        }
    }
}

#[derive(Default)]
struct Pen {
    segments: Vec<Segment>,
    start: Point2,
    at: Point2,
}

impl Pen {
    fn reach(&mut self, segment: Segment, to: Point2) {
        let drawn = match &segment {
            Segment::Line(from, to) => from != to,
            Segment::Bezier(points) => points.iter().any(|point| *point != self.at),
        };
        if drawn {
            self.segments.push(segment);
        }
        self.at = to;
    }
}

fn point(x: f32, y: f32) -> Point2 {
    Point2::new(f64::from(x), f64::from(y))
}

impl OutlineBuilder for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = point(x, y);
        self.at = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = point(x, y);
        self.reach(Segment::Line(self.at, to), to);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let to = point(x, y);
        self.reach(Segment::Bezier(vec![self.at, point(x1, y1), to]), to);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let to = point(x, y);
        let points = vec![self.at, point(x1, y1), point(x2, y2), to];
        self.reach(Segment::Bezier(points), to);
    }

    fn close(&mut self) {
        let start = self.start;
        self.reach(Segment::Line(self.at, start), start);
    }
}
